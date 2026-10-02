//! Single precision (Motorola FFP) arithmetic inside the module.
//!
//! Instruction by instruction ports of the routines of `amos_core::ffp`
//! (themselves ports of the AMOS 68000 code, `L28422` add, `L2858A`
//! multiply, `L28518` divide...), so that single precision operations need
//! no call to the runtime and give bit-identical results. Floats are
//! carried as `f64` holding exact FFP values: the `F*` wrappers convert to
//! FFP bits (`Ffp::from_f64`), operate, and convert back (`Ffp::to_f64`).
//!
//! The tests of `amos-wasmhost` (`tests/ffp.rs`) compare every function
//! with `amos_core::ffp` on millions of inputs.

use wasm_encoder::{BlockType, Function, InstructionSink, ValType};

/// Helper functions, in module order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum H {
    F2b,
    B2f,
    Add,
    Sub,
    AddCore,
    AddCarry,
    AddSame,
    AddDiff,
    NormSub,
    Mul,
    Div,
    Cmp,
    FromLong,
    FAdd,
    FSub,
    FMul,
    FDiv,
    FCmp,
    I2f,
    ToLong,
}

const I: ValType = ValType::I32;
const F: ValType = ValType::F64;
const L: ValType = ValType::I64;

pub const HELPERS: &[(H, &str, &[ValType], &[ValType])] = &[
    (H::F2b, "f2b", &[F], &[I]),
    (H::B2f, "b2f", &[I], &[F]),
    (H::Add, "add", &[I, I], &[I]),
    (H::Sub, "sub", &[I, I], &[I]),
    (H::AddCore, "add_core", &[I, I, I], &[I]),
    (H::AddCarry, "add_carry", &[I, I], &[I]),
    (H::AddSame, "add_same", &[I, I, I, I], &[I]),
    (H::AddDiff, "add_diff", &[I, I, I, I], &[I]),
    (H::NormSub, "normalise_sub", &[I, I, I], &[I]),
    (H::Mul, "mul", &[I, I], &[I]),
    (H::Div, "div", &[I, I], &[I]),
    (H::Cmp, "cmp", &[I, I], &[I]),
    (H::FromLong, "from_long", &[I], &[I]),
    (H::FAdd, "fadd", &[F, F], &[F]),
    (H::FSub, "fsub", &[F, F], &[F]),
    (H::FMul, "fmul", &[F, F], &[F]),
    (H::FDiv, "fdiv", &[F, F], &[F]),
    (H::FCmp, "fcmp", &[F, F], &[I]),
    (H::I2f, "i2f", &[I], &[F]),
    (H::ToLong, "to_long", &[I], &[I]),
];

/// Pushes `(r & !0xFF) | b` (`setb`) for locals r, b.
fn setb(s: &mut InstructionSink, r: u32, b: u32) {
    s.local_get(r).i32_const(-256).i32_and().local_get(b).i32_or();
}

/// Pushes the low byte of local r.
fn lb(s: &mut InstructionSink, r: u32) {
    s.local_get(r).i32_const(0xFF).i32_and();
}

/// Pushes `(a > b) - (a < b)` for signed locals.
fn cmp(s: &mut InstructionSink, a: u32, b: u32) {
    s.local_get(a).local_get(b).i32_gt_s().local_get(a).local_get(b).i32_lt_s().i32_sub();
}

fn empty() -> BlockType {
    BlockType::Empty
}

/// The body of helper `h`; `base` is the function index of the first helper.
pub fn body(h: H, base: u32) -> Function {
    let call = |h: H| base + h as u32;
    let locals: &[(u32, ValType)] = match h {
        H::F2b => &[(1, L), (3, I)],
        H::B2f => &[(1, F)],
        H::Sub => &[(1, I)],
        H::AddCore => &[(1, I)],
        H::AddCarry => &[(1, I)],
        H::AddSame => &[(7, I)],
        H::AddDiff => &[(10, I)],
        H::NormSub => &[(1, I)],
        H::Mul => &[(14, I)],
        H::Div => &[(17, I)],
        H::Cmp => &[(2, I)],
        H::FromLong => &[(3, I)],
        H::ToLong => &[(3, I)],
        _ => &[],
    };
    let mut f = Function::new(locals.iter().copied());
    let s = &mut f.instructions();
    match h {
        H::F2b => {
            // v=0 (f64); bits=1 (i64); bexp=2 e=3 sign=4
            s.local_get(0).local_get(0).f64_ne().if_(empty()).i32_const(0).return_().end();
            s.local_get(0).f64_abs().i64_reinterpret_f64().local_set(1);
            s.local_get(1).i64_const(52).i64_shr_u().i32_wrap_i64().local_set(2);
            s.local_get(2).i32_eqz().if_(empty()).i32_const(0).return_().end();
            s.local_get(2).i32_const(1022 - 64).i32_sub().local_set(3);
            s.local_get(3).i32_const(0).i32_le_s().if_(empty()).i32_const(0).return_().end();
            s.local_get(0).f64_const(0.0.into()).f64_lt().i32_const(7).i32_shl().local_set(4);
            s.local_get(3).i32_const(127).i32_gt_s().if_(empty());
            s.i32_const(-129).local_get(4).i32_or().return_().end();
            s.local_get(1).i64_const(1 << 52).i64_or().i64_const(29).i64_shr_u().i32_wrap_i64();
            s.i32_const(8).i32_shl().local_get(4).i32_or().local_get(3).i32_or();
        }
        H::B2f => {
            // v=0; x=1 (f64). Exact: mantissa * 2^(e - 64 - 24).
            s.local_get(0).i32_const(0xFF).i32_and().i32_eqz().if_(empty()).f64_const(0.0.into()).return_().end();
            s.local_get(0).i32_const(8).i32_shr_u().f64_convert_i32_u();
            s.local_get(0).i32_const(0x7F).i32_and().i32_const(1023 - 88).i32_add();
            s.i64_extend_i32_u().i64_const(52).i64_shl().f64_reinterpret_i64().f64_mul().local_set(1);
            s.local_get(0).i32_const(0x80).i32_and().if_(empty()).local_get(1).f64_neg().return_().end();
            s.local_get(1);
        }
        H::Add => {
            s.local_get(0).local_get(1);
            lb(s, 1);
            s.call(call(H::AddCore));
        }
        H::Sub => {
            // d4b=2
            lb(s, 1);
            s.local_tee(2).i32_eqz().if_(empty()).local_get(0).return_().end();
            s.local_get(0).local_get(1).local_get(2).i32_const(0x80).i32_xor().call(call(H::AddCore));
        }
        H::AddCore => {
            // d7=0 d6=1 d4b=2; d5b=3
            s.local_get(2).i32_const(0x80).i32_and().if_(empty());
            {
                lb(s, 0);
                s.local_set(3);
                s.local_get(3).i32_const(0x80).i32_and().if_(empty());
                s.local_get(0).local_get(1).local_get(2).local_get(3).call(call(H::AddSame)).return_().end();
                s.local_get(3).i32_eqz().if_(empty());
                setb(s, 1, 2);
                s.return_().end();
                s.local_get(0).local_get(1).local_get(2).local_get(3).call(call(H::AddDiff)).return_();
            }
            s.end();
            s.local_get(2).i32_eqz().if_(empty()).local_get(0).return_().end();
            lb(s, 0);
            s.local_set(3);
            s.local_get(3).i32_const(0x80).i32_and().if_(empty());
            s.local_get(0).local_get(1).local_get(2).local_get(3).call(call(H::AddDiff)).return_().end();
            s.local_get(3).i32_eqz().if_(empty());
            setb(s, 1, 2);
            s.return_().end();
            s.local_get(0).local_get(1).local_get(2).local_get(3).call(call(H::AddSame));
        }
        H::AddCarry => {
            // sum=0 d4b=1; d7=2
            s.local_get(0).i32_const(1).i32_shr_u().i32_const(i32::MIN).i32_or().local_set(2);
            s.local_get(1).i32_const(0x7F).i32_eq().local_get(1).i32_const(0xFF).i32_eq().i32_or();
            s.if_(empty()).i32_const(-256).local_get(1).i32_or().return_().end();
            s.local_get(2).i32_const(-256).i32_and();
            s.local_get(1).i32_const(1).i32_add().i32_const(0xFF).i32_and().i32_or();
        }
        H::AddSame => {
            // d7=0 d6=1 d4b=2 d5b=3; d5=4 d5s=5 n=6 x=7 sum=8 d3=9 d4b'=10
            s.local_get(3).local_get(2).i32_sub().i32_const(0xFF).i32_and().local_set(4);
            s.local_get(4).i32_extend8_s().local_set(5);
            s.local_get(5).i32_const(0).i32_lt_s().if_(empty());
            {
                s.local_get(5).i32_const(-24).i32_le_s().if_(empty());
                setb(s, 1, 2);
                s.return_().end();
                s.i32_const(0).local_get(5).i32_sub().local_set(6);
                s.local_get(1).i32_const(-256).i32_and().i32_const(0x80).i32_or().local_set(9);
                s.local_get(0).i32_const(-256).i32_and().local_get(6).i32_shr_u().local_set(7);
                s.local_get(7).local_get(9).i32_add().local_set(8);
                s.local_get(8).local_get(7).i32_lt_u().if_(empty());
                s.local_get(8).local_get(2).call(call(H::AddCarry)).return_().end();
                setb(s, 8, 2);
                s.return_();
            }
            s.end();
            lb(s, 0);
            s.local_set(10);
            s.local_get(4).i32_const(24).i32_ge_u().if_(empty()).local_get(0).return_().end();
            s.local_get(1).i32_const(-256).i32_and().local_get(4).i32_shr_u().local_set(9);
            s.local_get(0).i32_const(-256).i32_and().i32_const(0x80).i32_or().local_set(7);
            s.local_get(7).local_get(9).i32_add().local_set(8);
            s.local_get(8).local_get(7).i32_lt_u().if_(empty());
            s.local_get(8).local_get(10).call(call(H::AddCarry)).return_().end();
            setb(s, 8, 10);
        }
        H::AddDiff => {
            // d7=0 d6=1 d4b=2 d5bx=3; d5=4 xb=5 diff=6 d5s=7 big=8 small=9 n=10
            // d4bb=11 d3=12 r=13
            s.local_get(3).i32_const(0x80).i32_xor().local_get(2).i32_sub().i32_const(0xFF).i32_and().local_set(4);
            s.local_get(4).i32_eqz().if_(empty());
            {
                lb(s, 0);
                s.local_set(5);
                s.local_get(0).i32_const(-256).i32_and();
                lb(s, 1);
                s.i32_or().local_get(1).i32_sub().local_set(6);
                s.local_get(6).i32_eqz().if_(empty()).i32_const(0).return_().end();
                s.local_get(6).i32_const(0).i32_gt_s().if_(empty());
                s.local_get(6).local_get(5).local_get(5).call(call(H::NormSub)).return_().end();
                s.i32_const(0).local_get(6).i32_sub().local_get(2).local_get(2).call(call(H::NormSub)).return_();
            }
            s.end();
            s.local_get(4).i32_extend8_s().local_set(7);
            s.local_get(7).i32_const(0).i32_lt_s().if_(empty());
            {
                s.local_get(7).i32_const(-24).i32_le_s().if_(empty());
                setb(s, 1, 2);
                s.return_().end();
                s.local_get(1).i32_const(-256).i32_and().i32_const(0x80).i32_or().local_set(8);
                s.local_get(0).local_set(9);
                s.i32_const(0).local_get(7).i32_sub().local_set(10);
                s.local_get(2).local_set(11);
            }
            s.else_();
            {
                s.local_get(4).i32_const(24).i32_ge_u().if_(empty()).local_get(0).return_().end();
                s.local_get(0).i32_const(-256).i32_and().i32_const(0x80).i32_or().local_set(8);
                s.local_get(1).local_set(9);
                s.local_get(4).local_set(10);
                lb(s, 0);
                s.local_set(11);
            }
            s.end();
            s.local_get(9).i32_const(-256).i32_and().local_get(10).i32_shr_u().local_set(12);
            s.local_get(8).local_get(12).i32_sub().local_set(13);
            s.local_get(13).i32_const(0).i32_lt_s().if_(empty());
            setb(s, 13, 11);
            s.return_().end();
            s.local_get(13).local_get(11).local_get(11).call(call(H::NormSub));
        }
        H::NormSub => {
            // d7=0 d4b=1 d5b=2; i=3
            s.local_get(0).i32_const(-256).i32_and().local_set(0);
            s.local_get(1).i32_const(1).i32_sub().i32_const(0xFF).i32_and().local_set(1);
            s.local_get(0).i32_const(0x7FFF).i32_le_u().if_(empty());
            s.local_get(0).i32_const(16).i32_rotl().local_set(0);
            s.local_get(1).i32_const(0x10).i32_sub().i32_const(0xFF).i32_and().local_set(1);
            s.end();
            s.i32_const(40).local_set(3);
            s.block(empty()).loop_(empty());
            s.local_get(0).local_get(0).i32_add().local_set(0);
            s.local_get(0).i32_const(0).i32_lt_s().br_if(1);
            s.local_get(0).i32_eqz().br_if(1);
            s.local_get(1).i32_const(1).i32_sub().i32_const(0xFF).i32_and().local_set(1);
            s.local_get(3).i32_const(1).i32_sub().local_tee(3).br_if(0);
            s.end().end();
            s.local_get(1).local_get(2).i32_xor().i32_const(0x80).i32_and().if_(empty()).i32_const(0).return_().end();
            s.local_get(1).i32_eqz().if_(empty()).i32_const(0).return_().end();
            setb(s, 0, 1);
        }
        H::Mul => {
            // x=0 y=1; d5b=2 d4b=3 a=4 b=5 sum=6 eb=7 xh=8 xl=9 yh=10 yl=11
            // d4=12 s=13 d7=14 c=15
            lb(s, 0);
            s.local_tee(2).i32_eqz().if_(empty()).local_get(0).return_().end();
            lb(s, 1);
            s.local_tee(3).i32_eqz().if_(empty()).i32_const(0).return_().end();
            for (src, dst) in [(2, 4), (3, 5)] {
                s.local_get(src).i32_const(1).i32_shl().i32_const(0x80).i32_xor().i32_const(0xFF).i32_and();
                s.i32_extend8_s().local_set(dst);
            }
            s.local_get(4).local_get(5).i32_add().local_set(6);
            s.local_get(6).i32_const(-128).i32_lt_s().local_get(6).i32_const(127).i32_gt_s().i32_or().if_(empty());
            {
                s.local_get(6).i32_extend8_s().i32_const(0).i32_ge_s().if_(empty()).i32_const(0).return_().end();
                s.local_get(0).i32_const(-256).i32_and().local_get(2).local_get(3).i32_xor().i32_or();
                s.i32_const(-129).i32_or().return_();
            }
            s.end();
            s.local_get(2).local_get(3).i32_xor().i32_const(0x80).i32_and();
            s.local_get(6).i32_const(0xFF).i32_and().i32_const(0x80).i32_xor().i32_const(1).i32_shr_u();
            s.i32_or().local_set(7);
            s.local_get(0).i32_const(16).i32_shr_u().local_set(8);
            s.local_get(0).i32_const(0xFF00).i32_and().local_set(9);
            s.local_get(1).i32_const(16).i32_shr_u().local_set(10);
            s.local_get(1).i32_const(0xFF00).i32_and().local_set(11);
            s.local_get(11).local_get(9).i32_mul().i32_const(16).i32_rotl().local_set(12);
            s.local_get(12).local_get(8).local_get(11).i32_mul().i32_add().local_set(12);
            s.local_get(12).local_get(10).local_get(9).i32_mul().i32_add().local_set(13);
            s.local_get(13).local_get(12).i32_lt_u().i32_const(16).i32_shl();
            s.local_get(13).i32_const(16).i32_shr_u().i32_or().local_set(12);
            s.local_get(8).local_get(10).i32_mul().local_get(12).i32_add().local_set(14);
            s.local_get(14).i32_const(0).i32_lt_s().if_(empty());
            {
                s.local_get(14).i32_const(0x80).i32_add().local_set(14);
                s.local_get(7).i32_eqz().if_(empty()).i32_const(0).return_().end();
                setb(s, 14, 7);
                s.return_();
            }
            s.end();
            s.local_get(7).i32_const(0x80).i32_eq().local_get(7).i32_eqz().i32_or();
            s.if_(empty()).i32_const(0).return_().end();
            s.local_get(7).i32_const(1).i32_sub().i32_const(0xFF).i32_and().local_set(7);
            s.local_get(14).i32_const(0x40).i32_add().local_set(14);
            s.local_get(14).i32_const(0).i32_lt_s().local_set(15);
            s.local_get(14).local_get(14).i32_add().local_set(14);
            s.local_get(15).if_(empty());
            s.local_get(14).i32_const(1).i32_shr_u().i32_const(i32::MIN).i32_or().local_set(14);
            s.local_get(7).i32_const(1).i32_add().i32_const(0xFF).i32_and().local_set(7);
            s.end();
            s.local_get(7).i32_eqz().if_(empty()).i32_const(0).return_().end();
            setb(s, 14, 7);
        }
        H::Div => {
            // x=0 y=1; d5b=2 a=3 b=4 diff=5 d4b=6 m=7 xh=8 yh=9 eb=10 q1=11
            // remhi=12 d7=13 d3=14 r=15 d5=16 q=17 xb=18
            lb(s, 1);
            s.local_set(2);
            lb(s, 0);
            s.local_set(18);
            s.local_get(2).i32_eqz().if_(empty());
            s.i32_const(-129).local_get(18).local_get(2).i32_xor().i32_const(0x80).i32_and().i32_or().return_();
            s.end();
            s.local_get(0).i32_eqz().if_(empty()).i32_const(0).return_().end();
            for (src, dst) in [(18, 3), (2, 4)] {
                s.local_get(src).i32_const(1).i32_shl().i32_const(0x80).i32_xor().i32_const(0xFF).i32_and();
                s.i32_extend8_s().local_set(dst);
            }
            s.local_get(3).local_get(4).i32_sub().local_set(5);
            s.local_get(5).i32_const(-128).i32_lt_s().local_get(5).i32_const(127).i32_gt_s().i32_or().if_(empty());
            {
                s.local_get(5).i32_extend8_s().i32_const(0).i32_lt_s().if_(empty());
                s.local_get(0).i32_const(-256).i32_and().local_get(18).local_get(2).i32_xor().i32_or();
                s.i32_const(-129).i32_or().return_().end();
                s.i32_const(0).return_();
            }
            s.end();
            s.local_get(5).i32_const(0xFF).i32_and().local_set(6);
            s.local_get(0).i32_const(-256).i32_and().local_set(7);
            s.local_get(7).i32_const(16).i32_shr_u().local_set(8);
            s.local_get(1).i32_const(16).i32_shr_u().local_set(9);
            s.local_get(8).local_get(9).i32_sub().i32_const(0x8000).i32_and().i32_eqz().if_(empty());
            {
                s.local_get(6).i32_extend8_s().i32_const(125).i32_gt_s().if_(empty());
                s.local_get(2).i32_const(-129).i32_or().return_().end();
                s.local_get(6).i32_const(2).i32_add().i32_const(0xFF).i32_and().local_set(6);
                s.local_get(7).i32_const(1).i32_shr_u().local_set(7);
            }
            s.end();
            s.local_get(18).local_get(2).i32_xor().i32_const(0x80).i32_and();
            s.local_get(6).i32_const(0x80).i32_xor().i32_const(1).i32_shr_u().i32_or().local_set(10);
            // First 16 quotient bits (DIVU.W, overflow keeps the low word).
            s.local_get(7).local_get(9).i32_div_u().local_set(17);
            s.local_get(17).i32_const(0xFFFF).i32_gt_u().if_(empty());
            s.local_get(7).i32_const(0xFFFF).i32_and().local_set(11);
            s.else_();
            s.local_get(17).local_set(11);
            s.end();
            s.local_get(7).local_get(11).local_get(9).i32_mul().i32_sub().local_set(12);
            s.local_get(12).i32_const(16).i32_rotl().local_set(13);
            s.local_get(1).i32_const(0xFF00).i32_and().local_get(11).i32_mul().local_set(14);
            s.local_get(13).local_get(14).i32_sub().local_set(15);
            s.local_get(14).local_get(13).i32_gt_u().if_(empty());
            s.local_get(15).local_get(1).i32_const(-256).i32_and().i32_add().local_set(13);
            s.local_get(11).i32_const(1).i32_sub().i32_const(0xFFFF).i32_and().local_set(11);
            s.else_();
            s.local_get(15).local_set(13);
            s.end();
            s.local_get(13).i32_const(-65536).i32_and().local_set(13);
            s.local_get(13).local_get(9).i32_div_u().local_set(17);
            s.local_get(17).i32_const(0xFFFF).i32_le_u().if_(empty());
            s.local_get(13).local_get(9).i32_rem_u().i32_const(16).i32_shl().local_get(17).i32_or().local_set(13);
            s.end();
            s.local_get(11).i32_const(16).i32_shl().local_get(13).i32_const(0xFFFF).i32_and().i32_or().local_set(16);
            s.local_get(11).i32_const(0x8000).i32_and().i32_eqz().if_(empty());
            s.local_get(16).local_get(16).i32_add().local_set(16);
            s.local_get(10).i32_const(1).i32_sub().i32_const(0xFF).i32_and().local_set(10);
            s.end();
            s.local_get(16).i32_const(0x80).i32_add().local_set(16);
            s.local_get(10).i32_eqz().if_(empty()).i32_const(0).return_().end();
            setb(s, 16, 10);
        }
        H::Cmp => {
            // x=0 y=1; xb=2 yb=3
            s.local_get(0).i32_extend8_s().local_set(2);
            s.local_get(1).i32_extend8_s().local_set(3);
            s.local_get(2).i32_const(0).i32_lt_s().local_get(3).i32_const(0).i32_lt_s().i32_and().if_(empty());
            {
                s.local_get(2).local_get(3).i32_ne().if_(empty());
                cmp(s, 3, 2);
                s.return_().end();
                cmp(s, 1, 0);
                s.return_();
            }
            s.end();
            s.local_get(2).local_get(3).i32_ne().if_(empty());
            cmp(s, 2, 3);
            s.return_().end();
            cmp(s, 0, 1);
        }
        H::FromLong => {
            // v=0; neg=1 e=2 sh=3
            s.local_get(0).i32_const(i32::MIN).i32_eq().if_(empty()).i32_const(0x8000_00E0u32 as i32).return_().end();
            s.local_get(0).i32_const(0).i32_lt_s().local_set(1);
            s.local_get(1).if_(empty()).i32_const(0).local_get(0).i32_sub().local_set(0).end();
            s.local_get(0).i32_eqz().if_(empty()).i32_const(0).return_().end();
            // Normalise so that the top bit is bit 23.
            s.i32_const(31).local_get(0).i32_clz().i32_sub().i32_const(23).i32_sub().local_set(3);
            s.local_get(3).i32_const(0).i32_gt_s().if_(empty());
            s.local_get(0).local_get(3).i32_shr_u().local_set(0);
            s.else_();
            s.local_get(0).i32_const(0).local_get(3).i32_sub().i32_shl().local_set(0);
            s.end();
            s.i32_const(24).local_get(3).i32_add().local_set(2);
            s.local_get(0).i32_const(8).i32_shl();
            s.local_get(2).i32_const(0x40).i32_add().i32_const(0x7F).i32_and().i32_or();
            s.local_get(1).i32_const(7).i32_shl().i32_or();
        }
        H::ToLong => {
            // `ffp_to_long`: truncation toward zero, saturating. x=0; e=1 neg=2 m=3
            s.local_get(0).i32_const(0x7F).i32_and().i32_const(0x40).i32_sub().local_set(1);
            s.local_get(0)
                .i32_eqz()
                .local_get(1)
                .i32_const(0)
                .i32_lt_s()
                .i32_or()
                .if_(empty())
                .i32_const(0)
                .return_()
                .end();
            s.local_get(0).i32_const(0x80).i32_and().local_set(2);
            s.local_get(1).i32_const(31).i32_gt_s().if_(empty());
            s.i32_const(i32::MIN).i32_const(i32::MAX).local_get(2).select().return_();
            s.end();
            s.local_get(0).i32_const(8).i32_shr_s().i32_const(0xFF_FFFF).i32_and().local_set(3);
            s.local_get(1).i32_const(24).i32_sub().local_tee(1).i32_const(0).i32_lt_s().if_(empty());
            s.local_get(3).i32_const(0).local_get(1).i32_sub().i32_shr_s().local_set(3);
            s.else_();
            s.local_get(3).local_get(1).i32_shl().local_set(3);
            s.end();
            s.local_get(2).if_(empty()).i32_const(0).local_get(3).i32_sub().return_().end();
            s.local_get(3);
        }
        H::FAdd | H::FSub | H::FMul | H::FDiv => {
            let op = match h {
                H::FAdd => H::Add,
                H::FSub => H::Sub,
                H::FMul => H::Mul,
                _ => H::Div,
            };
            s.local_get(0).call(call(H::F2b)).local_get(1).call(call(H::F2b)).call(call(op)).call(call(H::B2f));
        }
        H::FCmp => {
            s.local_get(0).call(call(H::F2b)).local_get(1).call(call(H::F2b)).call(call(H::Cmp));
        }
        H::I2f => {
            s.local_get(0).call(call(H::FromLong)).call(call(H::B2f));
        }
    }
    s.end();
    f
}

/// A module exporting the helpers under their names (for tests).
pub fn test_module() -> Vec<u8> {
    use wasm_encoder::{CodeSection, ExportKind, ExportSection, FunctionSection, Module, TypeSection};
    let mut types = TypeSection::new();
    let mut funcs = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    for (i, (h, name, p, r)) in HELPERS.iter().enumerate() {
        types.ty().function(p.iter().copied(), r.iter().copied());
        funcs.function(i as u32);
        exports.export(name, ExportKind::Func, i as u32);
        code.function(&body(*h, 0));
    }
    let mut m = Module::new();
    m.section(&types).section(&funcs).section(&exports).section(&code);
    m.finish()
}
