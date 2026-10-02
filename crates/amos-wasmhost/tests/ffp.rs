//! The single precision routines compiled into modules
//! (`amos_compiler::ffp`) must be bit-identical to `amos_core::ffp`.

#![cfg(not(target_arch = "wasm32"))]

use amos_core::ffp::Ffp;
use wasmtime::{Engine, Instance, Module, Store};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A normalised FFP value (or zero), with edge exponents more often.
    fn ffp(&mut self) -> u32 {
        let r = self.next();
        if r.is_multiple_of(50) {
            return 0;
        }
        let mant = match (r >> 8) % 6 {
            0 => 0x80_0000,
            1 => 0xFF_FFFF,
            _ => 0x80_0000 | ((r >> 16) as u32 & 0x7F_FFFF),
        };
        let exp = match (r >> 40) % 8 {
            0 => [0, 1, 2, 63, 64, 65, 126, 127][(r >> 44) as usize % 8],
            1 => 60 + (r >> 48) as u32 % 10,
            _ => (r >> 48) as u32 & 0x7F,
        };
        (mant << 8) | (((r >> 60) as u32 & 1) << 7) | exp
    }
}

#[test]
fn ffp_helpers_are_bit_exact() {
    let wasm = amos_compiler::ffp::test_module();
    wasmparser::Validator::new().validate_all(&wasm).unwrap();
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).unwrap();
    let mut store = Store::new(&engine, ());
    let inst = Instance::new(&mut store, &module, &[]).unwrap();
    let f2 = |s: &mut Store<()>, n: &str| inst.get_typed_func::<(i32, i32), i32>(&mut *s, n).unwrap();
    let (add, sub, mul, div, cmp) = (
        f2(&mut store, "add"),
        f2(&mut store, "sub"),
        f2(&mut store, "mul"),
        f2(&mut store, "div"),
        f2(&mut store, "cmp"),
    );
    let from_long = inst.get_typed_func::<i32, i32>(&mut store, "from_long").unwrap();
    let f2b = inst.get_typed_func::<f64, i32>(&mut store, "f2b").unwrap();
    let b2f = inst.get_typed_func::<i32, f64>(&mut store, "b2f").unwrap();
    let fadd = inst.get_typed_func::<(f64, f64), f64>(&mut store, "fadd").unwrap();
    let fcmp = inst.get_typed_func::<(f64, f64), i32>(&mut store, "fcmp").unwrap();
    let i2f = inst.get_typed_func::<i32, f64>(&mut store, "i2f").unwrap();
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for i in 0..2_000_000u32 {
        let (x, y) = (rng.ffp(), rng.ffp());
        let (fx, fy) = (Ffp(x), Ffp(y));
        let w = |r: i32| r as u32;
        assert_eq!(w(add.call(&mut store, (x as i32, y as i32)).unwrap()), fx.add(fy).0, "add {x:08x} {y:08x}");
        assert_eq!(w(sub.call(&mut store, (x as i32, y as i32)).unwrap()), fx.sub(fy).0, "sub {x:08x} {y:08x}");
        assert_eq!(w(mul.call(&mut store, (x as i32, y as i32)).unwrap()), fx.mul(fy).0, "mul {x:08x} {y:08x}");
        if y & 0xFF != 0 {
            assert_eq!(w(div.call(&mut store, (x as i32, y as i32)).unwrap()), fx.div(fy).0, "div {x:08x} {y:08x}");
        }
        let c = fx.cmp(fy) as i32;
        assert_eq!(cmp.call(&mut store, (x as i32, y as i32)).unwrap(), c, "cmp {x:08x} {y:08x}");
        // Conversions.
        let bits = rng.next();
        let d = f64::from_bits(bits);
        assert_eq!(w(f2b.call(&mut store, d).unwrap()), Ffp::from_f64(d).0, "f2b {bits:016x}");
        let any = rng.next() as u32;
        assert_eq!(b2f.call(&mut store, any as i32).unwrap().to_bits(), Ffp(any).to_f64().to_bits(), "b2f {any:08x}");
        let n = match i % 4 {
            0 => rng.next() as i32,
            1 => (rng.next() % 100_000) as i32 - 50_000,
            2 => [0, 1, -1, i32::MIN, i32::MAX, 0xFF_FFFF, 0x100_0001][(rng.next() % 7) as usize],
            _ => (rng.next() as i32) >> (rng.next() % 31),
        };
        assert_eq!(w(from_long.call(&mut store, n).unwrap()), Ffp::from_i32(n).0, "from_long {n}");
        assert_eq!(i2f.call(&mut store, n).unwrap().to_bits(), Ffp::from_i32(n).to_f64().to_bits());
        // f64 level, on exact values (as the compiled code holds them).
        let (a, b) = (fx.to_f64(), fy.to_f64());
        let r = Ffp::from_f64(a).add(Ffp::from_f64(b)).to_f64();
        assert_eq!(fadd.call(&mut store, (a, b)).unwrap().to_bits(), r.to_bits());
        assert_eq!(fcmp.call(&mut store, (a, b)).unwrap(), Ffp::from_f64(a).cmp(Ffp::from_f64(b)) as i32);
    }
}
