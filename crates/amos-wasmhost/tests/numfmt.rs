//! The number <-> text helpers compiled into modules
//! (`amos_compiler::numfmt`) must give exactly the results of `amos_core`
//! (`ffp::format_ffp`, `ffp::format_double`, `ffp::ascii_to_ffp`,
//! `tokenise::parse_number` as used by `Val`, `expr::format_radix`).

#![cfg(not(target_arch = "wasm32"))]

use amos_core::compiled::layout;
use amos_core::ffp::{Ffp, Fix};
use amos_core::tokenise::{Number, parse_float_text, parse_number};
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
        let mem2 = mem;
        let vd = Func::wrap(&mut store, move |c: Caller<'_, ()>, a: i32| -> f64 {
            let d = mem2.data(&c);
            let a = a as usize;
            let n = u32::from_le_bytes(d[a..a + 4].try_into().unwrap()) as usize;
            parse_float_text(&String::from_utf8_lossy(&d[a + 4..a + 4 + n]))
        });
        let inst = Instance::new(&mut store, &module, &[mem.into(), base.into(), chunk.into(), vd.into()]).unwrap();
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
