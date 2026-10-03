//! The number <-> text helpers compiled into modules
//! (`amos_compiler::numfmt`) must give exactly the results of `amos_core`
//! (`ffp::format_ffp`, `ffp::format_double`, `ffp::ascii_to_ffp`,
//! `tokenise::parse_number` as used by `Val`, `expr::format_radix`).

#![cfg(not(target_arch = "wasm32"))]

use amos_core::compiled::layout;
use amos_core::ffp::{Ffp, Fix};
use amos_core::tokenise::{Number, parse_number};
use wasmtime::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn ffp(&mut self) -> u32 {
        let r = self.next();
        if r.is_multiple_of(40) {
            return [0, 0x80, 0x8000_0041, 0x8000_00C1, 0xFFFF_FF7F, 0xFFFF_FFFF, 0x8000_0001][(r >> 8) as usize % 7];
        }
        let mant = 0x80_0000 | ((r >> 16) as u32 & 0x7F_FFFF);
        let exp = match (r >> 40) % 4 {
            0 => (r >> 48) as u32 & 0x7F,
            _ => 40 + (r >> 48) as u32 % 50,
        };
        (mant << 8) | (((r >> 60) as u32 & 1) << 7) | exp
    }
}

const MEM: u32 = 64 << 16;

/// Iterations: full counts in release builds, fewer in debug builds.
fn count(n: usize) -> usize {
    if cfg!(debug_assertions) { n / 10 } else { n }
}
const TEXT: u32 = 2048;
const HEAP: u32 = 1 << 16;

struct H {
    store: Store<()>,
    mem: Memory,
    inst: Instance,
}

impl H {
    fn new() -> H {
        let wasm = amos_compiler::numfmt::test_module();
        wasmparser::Validator::new().validate_all(&wasm).unwrap();
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let mem = Memory::new(&mut store, MemoryType::new(64, None)).unwrap();
        let base = Global::new(&mut store, GlobalType::new(ValType::I32, Mutability::Const), Val::I32(0)).unwrap();
        let chunk = Func::wrap(&mut store, |_: i32| -> i32 { panic!("the test heap is large enough") });
        let inst = Instance::new(&mut store, &module, &[mem.into(), base.into(), chunk.into()]).unwrap();
        H { store, mem, inst }
    }

    /// Fresh string heap for the next call.
    fn reset(&mut self) {
        let d = self.mem.data_mut(&mut self.store);
        d[layout::STR_PTR as usize..][..4].copy_from_slice(&HEAP.to_le_bytes());
        d[layout::STR_END as usize..][..4].copy_from_slice(&MEM.to_le_bytes());
        d[HEAP as usize..].fill(0);
    }

    fn string(&self, a: i32) -> Vec<u8> {
        if a == 0 {
            return Vec::new();
        }
        let d = self.mem.data(&self.store);
        let a = a as usize;
        let n = u32::from_le_bytes(d[a..a + 4].try_into().unwrap()) as usize;
        d[a + 4..a + 4 + n].to_vec()
    }

    /// Writes a string block at TEXT and returns its address.
    fn put(&mut self, t: &[u8]) -> i32 {
        let d = self.mem.data_mut(&mut self.store);
        d[TEXT as usize..][..4].copy_from_slice(&(t.len() as u32).to_le_bytes());
        d[TEXT as usize + 4..][..t.len()].copy_from_slice(t);
        TEXT as i32
    }

    fn tag(&self) -> i32 {
        let d = self.mem.data(&self.store);
        i32::from_le_bytes(d[layout::TAG as usize..][..4].try_into().unwrap())
    }
}

fn fixes() -> Vec<Fix> {
    let mut v = vec![Fix::Free, Fix::ExponentFree];
    for n in 0..16 {
        v.push(Fix::Decimals(n));
    }
    for n in 1..16 {
        v.push(Fix::Exponent(n));
    }
    v
}

#[test]
fn single_precision_formatting() {
    let mut h = H::new();
    let f = h.inst.get_typed_func::<(i32, i32, i32), i32>(&mut h.store, "float_to_asc").unwrap();
    let sf = h.inst.get_typed_func::<(f64, i32), i32>(&mut h.store, "str_f").unwrap();
    let mut r = Rng(0x0123_4567_89AB_CDEF);
    let fixes = fixes();
    for i in 0..count(400_000) {
        let x = r.ffp();
        let fix = fixes[i % fixes.len()];
        h.reset();
        let a = f.call(&mut h.store, (x as i32, fix.fix_flg() as i32, fix.exp_flg() as i32)).unwrap();
        let want = amos_core::ffp::format_ffp(Ffp(x), fix);
        assert_eq!(String::from_utf8_lossy(&h.string(a)), want, "x={x:08x} fix={fix:?}");
        if i % 8 == 0 {
            // Through Str$ (f64 holding the value, Fix from the header).
            h.reset();
            let d = h.mem.data_mut(&mut h.store);
            d[layout::FIX_FLG as usize..][..4].copy_from_slice(&(fix.fix_flg() as i32).to_le_bytes());
            d[layout::EXP_FLG as usize..][..4].copy_from_slice(&(fix.exp_flg() as i32).to_le_bytes());
            // As `Interp::format_float`: the f64 back to FFP, then formatted.
            let v = Ffp(x).to_f64();
            let a = sf.call(&mut h.store, (v, 0)).unwrap();
            let want = amos_core::ffp::format_ffp(Ffp::from_f64(v), fix);
            assert_eq!(String::from_utf8_lossy(&h.string(a)), want, "str_f x={x:08x} fix={fix:?}");
        }
    }
}

#[test]
fn double_precision_formatting() {
    let mut h = H::new();
    let f = h.inst.get_typed_func::<(f64, i32, i32), i32>(&mut h.store, "format_double").unwrap();
    let mut r = Rng(0xFEDC_BA98_7654_3210);
    let fixes = fixes();
    for i in 0..count(400_000) {
        let x = match r.below(6) {
            0 => f64::from_bits(r.next()),
            1 => (r.below(2_000_000) as f64 - 1_000_000.0) / (1u64 << r.below(20)) as f64,
            2 => r.below(1000) as f64,
            3 => [0.0, -0.0, 1.0, 0.1, 1e15, 1e16, 1e-5, 9.999999999999999, 0.5, f64::INFINITY, f64::NAN, -1e300]
                [r.below(12) as usize],
            4 => (r.next() as i64 as f64) * 10f64.powi(r.below(60) as i32 - 30),
            _ => f64::from_bits(r.next() & 0x800F_FFFF_FFFF_FFFF | (r.below(80) + 980) << 52),
        };
        let fix = fixes[i % fixes.len()];
        h.reset();
        let a = f.call(&mut h.store, (x, fix.fix_flg() as i32, fix.exp_flg() as i32)).unwrap();
        let want = amos_core::ffp::format_double(x, fix);
        assert_eq!(String::from_utf8_lossy(&h.string(a)), want, "x={x:e} ({:016x}) fix={fix:?}", x.to_bits());
    }
}

fn random_text(r: &mut Rng) -> Vec<u8> {
    let set = b" 0123456789.eE+-$%abcdefABCDEFx\t";
    match r.below(4) {
        0 => (0..r.below(20)).map(|_| set[r.below(set.len() as u64) as usize]).collect(),
        1 => {
            // A number, with spaces and an exponent sometimes.
            let mut t = String::new();
            if r.below(3) == 0 {
                t.push_str(["-", "+", " - ", "  "][r.below(4) as usize]);
            }
            for _ in 0..r.below(12) + 1 {
                t.push((b'0' + r.below(10) as u8) as char);
                if r.below(10) == 0 {
                    t.push(' ');
                }
            }
            if r.below(2) == 0 {
                t.push('.');
                for _ in 0..r.below(10) {
                    t.push((b'0' + r.below(10) as u8) as char);
                }
            }
            if r.below(3) == 0 {
                t.push_str(["e", "E", "e-", "e+", "E -", "e "][r.below(6) as usize]);
                for _ in 0..r.below(6) {
                    t.push((b'0' + r.below(10) as u8) as char);
                }
            }
            if r.below(4) == 0 {
                t.push_str(["x", " ", "1.2", "e5"][r.below(4) as usize]);
            }
            t.into_bytes()
        }
        2 => format!("{}", (r.next() as i64) >> r.below(63)).into_bytes(),
        _ => match r.below(5) {
            0 => format!("${:x}", r.next() >> r.below(64)).into_bytes(),
            1 => format!("%{:b}", r.next() >> r.below(64)).into_bytes(),
            2 => format!("{:e}", f64::from_bits(r.next() & 0x7FEF_FFFF_FFFF_FFFF)).into_bytes(),
            3 => format!("{}", r.below(10_000_000) as f64 / 1000.0).into_bytes(),
            _ => format!("% 1 0 1 1 {}", r.below(2)).into_bytes(),
        },
    }
}

#[test]
fn ascii_to_ffp_and_val() {
    let mut h = H::new();
    let a2 = h.inst.get_typed_func::<(i32, i32), i32>(&mut h.store, "a2ffp").unwrap();
    let val = h.inst.get_typed_func::<(i32, i32), f64>(&mut h.store, "val").unwrap();
    let mut r = Rng(0x5555_AAAA_1234_9876);
    for _ in 0..count(400_000) {
        let t = random_text(&mut r);
        // a2ffp on float texts as Val gives them (lower case, no spaces).
        let ft: Vec<u8> = t.iter().filter(|&&c| c != b' ').map(|c| c.to_ascii_lowercase()).collect();
        h.reset();
        let p = h.put(&ft);
        let got = a2.call(&mut h.store, (p + 4, ft.len() as i32)).unwrap() as u32;
        assert_eq!(got, amos_core::ffp::ascii_to_ffp(&ft).0, "a2ffp {:?}", String::from_utf8_lossy(&ft));
        for double in [false, true] {
            h.reset();
            let p = h.put(&t);
            let v = val.call(&mut h.store, (p, double as i32)).unwrap();
            let (tag, want) = match parse_number(&t, true) {
                Some((Number::Int(i), _)) => (0, i as f64),
                Some((Number::Hex(x) | Number::Bin(x), _)) => (0, x as i32 as f64),
                Some((Number::Float(f, bits), _)) => (1, if double { f } else { bits.to_f64() }),
                None => (0, 0.0),
            };
            assert_eq!(
                (h.tag(), v.to_bits()),
                (tag, want.to_bits()),
                "Val {:?} double={double}",
                String::from_utf8_lossy(&t)
            );
        }
    }
}

#[test]
fn radix() {
    let mut h = H::new();
    let f = h.inst.get_typed_func::<(i32, i32, i32), i32>(&mut h.store, "radix").unwrap();
    let mut r = Rng(42);
    for i in 0..count(200_000) {
        let n = (r.next() >> r.below(64)) as u32;
        let hex = i % 2 == 0;
        let digits = match r.below(4) {
            0 => -1,
            1 => r.below(40) as i32 - 3,
            2 => [0, 8, 9, 32, 33, i32::MIN, i32::MAX][r.below(7) as usize],
            _ => r.below(9) as i32,
        };
        h.reset();
        let a = f.call(&mut h.store, (n as i32, hex as i32, digits)).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&h.string(a)),
            amos_core::interp::expr::format_radix(n, hex, digits),
            "n={n} hex={hex} digits={digits}"
        );
    }
}

/// The boundaries of the shortcuts of the conversions (tables, integer
/// digit accumulation, exact digit steps, the cached scaling): every
/// exponent with mantissa edges, values next to powers of ten, texts
/// around 2^24 and the table ends; each value twice in a row (cache hit).
#[test]
fn conversion_shortcut_boundaries() {
    use amos_core::ffp::{POW10_DOWN, POW10_UP};
    let mut h = H::new();
    let f = h.inst.get_typed_func::<(i32, i32, i32), i32>(&mut h.store, "float_to_asc").unwrap();
    let a2 = h.inst.get_typed_func::<(i32, i32), i32>(&mut h.store, "a2ffp").unwrap();
    let fixes = fixes();
    let mut xs = vec![0u32, 0x80, 0x8000_0041, 0x8000_00C1, 0xFFFF_FF7F, 0xFFFF_FFFF, 0x8000_0001, 0x8000_0081];
    for exp in 0..0x80u32 {
        for m in [0x80_0000u32, 0x80_0001, 0x9F_FFFF, 0xA0_0000, 0xC8_0000, 0xCC_CCCD, 0xF9_FFFF, 0xFF_FFFE, 0xFF_FFFF]
        {
            xs.push((m << 8) | exp);
            xs.push((m << 8) | 0x80 | exp);
        }
    }
    for p in POW10_UP.iter().chain(POW10_DOWN.iter()) {
        for d in -20i32..=20 {
            let m = ((p >> 8) as i32 + d) as u32;
            if (0x80_0000..0x100_0000).contains(&m) {
                xs.push((m << 8) | (p & 0xFF));
            }
        }
    }
    for x in xs {
        for &fix in &fixes {
            let want = amos_core::ffp::format_ffp(Ffp(x), fix);
            for _ in 0..2 {
                h.reset();
                let a = f.call(&mut h.store, (x as i32, fix.fix_flg() as i32, fix.exp_flg() as i32)).unwrap();
                assert_eq!(String::from_utf8_lossy(&h.string(a)), want, "x={x:08x} fix={fix:?}");
            }
        }
    }
    let mut texts: Vec<String> = Vec::new();
    for n in (16_777_100u32..16_777_300).chain(1_677_700..1_677_800).chain(167_772_000..167_772_100) {
        texts.extend([n.to_string(), format!("{n}.5"), format!("-{n}e-3"), format!("0.{n}")]);
    }
    for e in -120i32..120 {
        for m in ["1", "9.99", "1.5", "123456789", "0", ".1", "16777217"] {
            texts.push(format!("{m}e{e}"));
        }
    }
    for digits in 1..40 {
        let s: String = (0..digits).map(|i| char::from(b'0' + ((i * 7 + 3) % 10) as u8)).collect();
        texts.extend([s.clone(), format!("9{s}"), format!("{s}.{s}"), format!(".{s}e7")]);
    }
    for t in texts {
        h.reset();
        let p = h.put(t.as_bytes());
        let got = a2.call(&mut h.store, (p + 4, t.len() as i32)).unwrap() as u32;
        assert_eq!(got, amos_core::ffp::ascii_to_ffp(t.as_bytes()).0, "a2ffp {t:?}");
    }
}

/// The integer shortcuts of the helpers against the FFP operations:
/// `mul_ten` (x * 10), `sub_int` ((x - int(x)) * 10) and `norm` (scaling
/// to [1, 10) by repeated * 10 / 10, with the specialised division).
#[test]
fn integer_arithmetic_of_the_helpers() {
    let mut h = H::new();
    let mul_ten = h.inst.get_typed_func::<i32, i32>(&mut h.store, "mul_ten").unwrap();
    let sub_int = h.inst.get_typed_func::<(i32, i32), i32>(&mut h.store, "sub_int").unwrap();
    let norm = h.inst.get_typed_func::<i32, ()>(&mut h.store, "norm").unwrap();
    let ten = Ffp::TEN;
    let step = if cfg!(debug_assertions) { 61 } else { 3 };
    let mut xs: Vec<u32> = Vec::new();
    for eb in 0..0x100u32 {
        let st = if (0x3C..=0x45).contains(&eb) { step } else { step * 64 };
        xs.extend((0x80_0000u32..0x100_0000).step_by(st as usize).map(|m| (m << 8) | eb));
        xs.extend([0x1u32, 0x7F_FFFF, 0xFF_FFFF].map(|m| (m << 8) | eb));
    }
    let mut r = Rng(0xDEAD_BEEF_0BAD_F00D);
    xs.extend((0..count(1_000_000)).map(|_| r.next() as u32));
    for &x in &xs {
        let got = mul_ten.call(&mut h.store, x as i32).unwrap() as u32;
        assert_eq!(got, Ffp(x).mul(ten).0, "mul_ten {x:08x}");
        let d = Ffp(x).to_i32() as i16 as i32;
        let got = sub_int.call(&mut h.store, (x as i32, d)).unwrap() as u32;
        assert_eq!(got, Ffp(x).sub(Ffp::from_i32(d)).mul(ten).0, "sub_int {x:08x}");
        // norm of positive normalised values (as ffp2a calls it).
        if x & 0x8000_0080 == 0x8000_0000 && x & 0x7F != 0 {
            let (mut v, mut e) = (Ffp(x), 0);
            while v.cmp(Ffp::ONE).is_lt() {
                v = v.mul(ten);
                e -= 1;
            }
            while !v.cmp(ten).is_lt() {
                v = v.div(ten);
                e += 1;
            }
            norm.call(&mut h.store, x as i32).unwrap();
            let d = h.mem.data(&h.store);
            let rd = |o: u32| i32::from_le_bytes(d[o as usize..][..4].try_into().unwrap());
            assert_eq!((rd(layout::NORM_XN) as u32, rd(layout::NORM_E)), (v.0, e), "norm {x:08x}");
        }
    }
}

/// Double precision `Val` (`softdouble`): the module's integer arithmetic
/// (`d_add`, `d_mul`, `d_div`, `d_pow10`) and `AscToDouble` (`d_a2d`)
/// against `amos_core::softdouble`.
#[test]
fn double_precision_val() {
    use amos_core::softdouble as sd;
    let mut h = H::new();
    let add = h.inst.get_typed_func::<(i64, i64), i64>(&mut h.store, "d_add").unwrap();
    let mul = h.inst.get_typed_func::<(i64, i64), i64>(&mut h.store, "d_mul").unwrap();
    let div = h.inst.get_typed_func::<(i64, i64), i64>(&mut h.store, "d_div").unwrap();
    let pow = h.inst.get_typed_func::<i32, i64>(&mut h.store, "d_pow10").unwrap();
    let a2d = h.inst.get_typed_func::<(i32, i32), i64>(&mut h.store, "d_a2d").unwrap();
    let val = h.inst.get_typed_func::<(i32, i32), f64>(&mut h.store, "val").unwrap();
    let mut r = Rng(0xA5A5_5A5A_0F0F_F0F0);
    let operand = |r: &mut Rng| -> u64 {
        let v = r.next();
        match r.below(7) {
            0 => v,
            1 => (v & 0x800F_FFFF_FFFF_FFFF) | ((r.below(4) + (0x7FC * r.below(2))) << 52),
            2 => [0, 1 << 63, sd::ONE, sd::TEN, sd::OVERFLOW, sd::overflow_value(0), 0x0010_0000_0000_0000]
                [r.below(7) as usize],
            3 => sd::pow10(r.below(330) as i32),
            4 => (v & 0x800F_0000_0000_0000) | ((r.below(0x7FE) + 1) << 52),
            _ => (v & 0x800F_FFFF_FFFF_FFFF) | ((0x3FF - 70 + r.below(140)) << 52),
        }
    };
    for _ in 0..count(1_000_000) {
        let (a, b) = (operand(&mut r), operand(&mut r));
        let call =
            |f: &TypedFunc<(i64, i64), i64>, h: &mut H| f.call(&mut h.store, (a as i64, b as i64)).unwrap() as u64;
        assert_eq!(call(&add, &mut h), sd::add(a, b), "add {a:016X} {b:016X}");
        assert_eq!(call(&mul, &mut h), sd::mul(a, b), "mul {a:016X} {b:016X}");
        assert_eq!(call(&div, &mut h), sd::div(a, b), "div {a:016X} {b:016X}");
        // Close exponents (the alignment of the addition).
        let c = (r.next() & 0x800F_FFFF_FFFF_FFFF) | (((a >> 52) & 0x7FF).saturating_sub(r.below(70)) << 52);
        assert_eq!(
            add.call(&mut h.store, (a as i64, c as i64)).unwrap() as u64,
            sd::add(a, c),
            "add {a:016X} {c:016X}"
        );
    }
    for n in -5..3000 {
        assert_eq!(pow.call(&mut h.store, n).unwrap() as u64, sd::pow10(n), "10^{n}");
    }
    let mut texts: Vec<Vec<u8>> = [
        "",
        "-",
        "+",
        ".",
        "e5",
        "1e",
        "1e+",
        "1.e5",
        ".5e-3",
        "0",
        "-0",
        "-0.0",
        "1e308",
        "1e309",
        "-1e309",
        "1e-330",
        "1e400",
        "1e99999999999",
        "1e-99999999999",
        "1e2147483647",
        "1e2147483648",
        "1e-2147483648",
        "0.86",
        "9007199254740993",
        "00000000000000000000000000000001",
        " \t 12",
        "1x5",
        "--1",
    ]
    .iter()
    .map(|t| t.as_bytes().to_vec())
    .collect();
    for _ in 0..count(300_000) {
        texts.push(random_text(&mut r));
    }
    for t in &texts {
        h.reset();
        let p = h.put(t);
        let got = a2d.call(&mut h.store, (p + 4, t.len() as i32)).unwrap() as u64;
        assert_eq!(got, sd::asc_to_double(t), "d_a2d {:?}", String::from_utf8_lossy(t));
    }
    // Val, through BuFloat (sign, no spaces, at most 33 characters).
    let long = [
        "1234567890123456789012345678901234567890",
        "-  123456789012345678901234567890.123456",
        "0.000000000000000000000000000000000000001",
        "12345678901234567890123456789012.5",
        "1.2345678901234567890123456789012345e10",
    ];
    for t in long.iter().map(|t| t.as_bytes()).chain(texts.iter().map(|t| &t[..])) {
        for double in [false, true] {
            h.reset();
            let p = h.put(t);
            let v = val.call(&mut h.store, (p, double as i32)).unwrap();
            let want = match parse_number(t, true) {
                Some((Number::Int(i), _)) => i as f64,
                Some((Number::Hex(x) | Number::Bin(x), _)) => x as i32 as f64,
                Some((Number::Float(f, bits), _)) => {
                    if double {
                        f
                    } else {
                        bits.to_f64()
                    }
                }
                None => 0.0,
            };
            assert_eq!(v.to_bits(), want.to_bits(), "Val {:?} double={double}", String::from_utf8_lossy(t));
        }
    }
}

/// `Hex$` / `Bin$` on every digit count from -3 to 40 (and extremes) for
/// edge values; nothing is written after the string.
#[test]
fn radix_all_digit_counts() {
    let mut h = H::new();
    let f = h.inst.get_typed_func::<(i32, i32, i32), i32>(&mut h.store, "radix").unwrap();
    let mut values =
        vec![0u32, 1, 9, 10, 15, 16, 255, 256, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0x0123_4567, 0x89AB_CDEF];
    for k in 0..32 {
        values.extend([(1u32 << k).wrapping_sub(1), 1 << k, (1u32 << k) + 1]);
    }
    let mut r = Rng(0x0BAD_C0DE_1234_5678);
    values.extend((0..count(20_000)).map(|_| (r.next() >> r.below(64)) as u32));
    for &n in &values {
        for hex in [false, true] {
            for digits in (-3..=40).chain([i32::MIN, i32::MAX, -100, 100]) {
                h.reset();
                let a = f.call(&mut h.store, (n as i32, hex as i32, digits)).unwrap();
                let want = amos_core::interp::expr::format_radix(n, hex, digits);
                assert_eq!(String::from_utf8_lossy(&h.string(a)), want, "n={n:#x} hex={hex} digits={digits}");
                let d = h.mem.data(&h.store);
                let end = a as usize + 4 + want.len();
                assert!(d[end..end + 16].iter().all(|&b| b == 0), "written after the string: {n:#x} {digits}");
            }
        }
    }
}
