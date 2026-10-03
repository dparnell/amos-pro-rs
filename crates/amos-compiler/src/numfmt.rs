//! Number <-> text inside the module: `Val`, `Str$` of floats, `Hex$`,
//! `Bin$`, `Repeat$`.
//!
//! Instruction by instruction ports of `amos_core::ffp` (`ascii_to_ffp`,
//! `ffp2a`, `F2a`, `Clean`, `ExFix1`, `ExVir1`, `FloatToAsc`, `dtoa`,
//! `format_double`), `tokenise::parse_number` (`ValRout`) and
//! `interp::expr::format_radix`, so the results are bit-identical. The
//! one exception: the decimal to double conversion of `Val` in double
//! precision (`parse_float_text`, a correctly rounded conversion) is done
//! by the runtime (`host.val_double`); everything around it is here.
//!
//! Intermediate texts use two scratch buffers of the memory header
//! (`layout::SCR_A`, `SCR_B`); results are new strings (`strings.rs`).
//! `tests/numfmt.rs` of `amos-wasmhost` compares every helper with
//! `amos_core` on millions of inputs.

use amos_core::compiled::layout;
use wasm_encoder::{BlockType, Function, InstructionSink, MemArg, ValType};

use crate::ffp::H;
use crate::strings::S;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum N {
    Ffp2a,
    Clean,
    PushExp,
    ExFix,
    ExVir,
    F2a,
    FloatToAsc,
    Dtoa,
    FmtDouble,
    Renorm,
    A2ffp,
    Val,
    Radix,
    Repeat,
    StrF,
    Tab,
    Pow10,
    HalfDown,
    Norm,
    Shape,
    SubInt,
    MulTen,
}

const I: ValType = ValType::I32;
const F: ValType = ValType::F64;
const L: ValType = ValType::I64;

pub const HELPERS: &[(N, &str, &[ValType], &[ValType])] = &[
    (N::Ffp2a, "ffp2a", &[I, I, I], &[I]),
    (N::Clean, "clean", &[I, I, I], &[I]),
    (N::PushExp, "push_exp", &[I, I, I, I], &[I]),
    (N::ExFix, "ex_fix", &[I, I, I, I], &[I]),
    (N::ExVir, "ex_vir", &[I, I, I, I, I], &[I]),
    (N::F2a, "f2a", &[I, I, I, I], &[I]),
    (N::FloatToAsc, "float_to_asc", &[I, I, I], &[I]),
    (N::Dtoa, "dtoa", &[F, I, I, I], &[I]),
    (N::FmtDouble, "format_double", &[F, I, I], &[I]),
    (N::Renorm, "renormalise", &[I], &[I]),
    (N::A2ffp, "a2ffp", &[I, I], &[I]),
    (N::Val, "val", &[I, I], &[F]),
    (N::Radix, "radix", &[I, I, I], &[I]),
    (N::Repeat, "repeat", &[I, I], &[I]),
    (N::StrF, "str_f", &[F, I], &[I]),
    (N::Tab, "tab", &[I], &[I]),
    (N::Pow10, "pow10", &[I, I], &[I]),
    (N::HalfDown, "half_down", &[I], &[I]),
    (N::Norm, "norm", &[I], &[]),
    (N::Shape, "shape", &[I, I], &[I]),
    (N::SubInt, "sub_int", &[I, I], &[I]),
    (N::MulTen, "mul_ten", &[I], &[I]),
];

/// Function indices of the helper families and imports used here.
#[derive(Clone, Copy)]
pub struct Idx {
    pub ffp: u32,
    pub str: u32,
    pub num: u32,
    /// `host.val_double`.
    pub val_double: u32,
}

impl Idx {
    fn h(&self, h: H) -> u32 {
        self.ffp + h as u32
    }
    fn s(&self, s: S) -> u32 {
        self.str + s as u32
    }
    fn n(&self, n: N) -> u32 {
        self.num + n as u32
    }
}

const ONE: i32 = 0x8000_0041u32 as i32;
const TWO: i32 = 0x8000_0042u32 as i32;
const HALF: i32 = 0x8000_0040u32 as i32;
const TEN: i32 = 0xA000_0044u32 as i32;
const TWO24: i32 = 0x8000_0059u32 as i32;

/// `DDebut` (`ffp.rs` `DTAB`).
const DTAB: [u64; 18] = [
    0x4024_0000_0000_0000,
    0x3FF0_0000_0000_0000,
    0x3FE0_0000_0000_0000,
    0x3FA9_9999_9999_999A,
    0x3F74_7AE1_47AE_147B,
    0x3F40_624D_D2F1_A9FC,
    0x3F0A_36E2_EB1C_432D,
    0x3ED4_F8B5_88E3_68F1,
    0x3EA0_C6F7_A0B5_ED8E,
    0x3E6A_D7F2_9ABC_AF49,
    0x3E35_798E_E230_8C3A,
    0x3E01_2E0B_E826_D695,
    0x3DCB_7CDF_D9D7_BDBB,
    0x3D95_FD7F_E179_6496,
    0x3D61_9799_812D_EA12,
    0x3D2C_25C2_6849_7682,
    0x3CF6_849B_86A1_2B9C,
    0x3CC2_03AF_9EE7_5616,
];

fn b8() -> MemArg {
    MemArg { offset: 0, align: 0, memory_index: 0 }
}

fn e() -> BlockType {
    BlockType::Empty
}

fn ri() -> BlockType {
    BlockType::Result(I)
}

/// Pushes the byte at `buf + idx`, or 0 when `idx >= len` (`at(i)`).
fn at(s: &mut InstructionSink, buf: u32, len: u32, idx: u32) {
    s.local_get(idx).local_get(len).i32_lt_u().if_(ri());
    s.local_get(buf).local_get(idx).i32_add().i32_load8_u(b8());
    s.else_().i32_const(0).end();
}

/// Pushes the byte at `buf + idx` (no bound).
fn byte(s: &mut InstructionSink, buf: u32, idx: u32) {
    s.local_get(buf).local_get(idx).i32_add().i32_load8_u(b8());
}

/// `buf[o] = <value pushed by v>; o += 1`.
fn out(s: &mut InstructionSink, buf: u32, o: u32, v: impl FnOnce(&mut InstructionSink)) {
    s.local_get(buf).local_get(o).i32_add();
    v(s);
    s.i32_store8(b8());
    s.local_get(o).i32_const(1).i32_add().local_set(o);
}

fn outc(s: &mut InstructionSink, buf: u32, o: u32, c: u8) {
    out(s, buf, o, |s| {
        s.i32_const(c as i32);
    });
}

/// Pushes base + offset (address of a header buffer).
fn hdr_addr(s: &mut InstructionSink, off: u32) {
    s.global_get(0).i32_const(off as i32).i32_add();
}

/// Pushes `(a cmp b)` of FFP values (-1, 0, 1) with `b` a constant.
fn fcmp_c(s: &mut InstructionSink, ix: &Idx, a: u32, b: i32) {
    s.local_get(a).i32_const(b).call(ix.h(H::Cmp));
}

/// `memory.copy(dst, src, n)` with the three values on the stack.
fn copy(s: &mut InstructionSink) {
    s.memory_copy(0, 0);
}

/// The body of helper `n`.
pub fn body(n: N, ix: &Idx, double_const: bool) -> Function {
    let _ = double_const;
    let locals: Vec<(u32, ValType)> = match n {
        N::Ffp2a => vec![(12, I)],
        N::Clean => vec![(4, I)],
        N::PushExp => vec![],
        N::ExFix => vec![(11, I)],
        N::ExVir => vec![(10, I)],
        N::F2a => vec![(15, I)],
        N::FloatToAsc => vec![(8, I)],
        N::Dtoa => vec![(16, I), (1, L), (1, F)],
        N::FmtDouble => vec![(6, I)],
        N::Renorm => vec![(3, I)],
        N::A2ffp => vec![(19, I)],
        N::Val => vec![(16, I), (2, L), (1, F)],
        N::Radix => vec![(8, I)],
        N::Repeat => vec![(3, I)],
        N::StrF => vec![(2, I)],
        N::Tab => vec![],
        N::Pow10 => vec![(1, I)],
        N::HalfDown => vec![],
        N::Norm => vec![(5, I)],
        N::Shape => vec![(5, I)],
        N::SubInt => vec![(4, I)],
        N::MulTen => vec![(3, I)],
    };
    let mut f = Function::new(locals);
    let s = &mut f.instructions();
    match n {
        N::Ffp2a => ffp2a(s, ix),
        N::Clean => clean(s, ix),
        N::PushExp => push_exp(s),
        N::ExFix => ex_fix(s, ix),
        N::ExVir => ex_vir(s, ix),
        N::F2a => f2a(s, ix),
        N::FloatToAsc => float_to_asc(s, ix),
        N::Dtoa => dtoa(s),
        N::FmtDouble => format_double(s, ix),
        N::Renorm => renorm(s, ix),
        N::A2ffp => a2ffp(s, ix),
        N::Val => val(s, ix),
        N::Radix => radix(s, ix),
        N::Repeat => repeat(s, ix),
        N::StrF => str_f(s, ix),
        N::Tab => tab(s),
        N::Pow10 => pow10(s, ix),
        N::HalfDown => half_down(s, ix),
        N::Norm => norm(s, ix),
        N::Shape => shape(s, ix),
        N::SubInt => sub_int(s, ix),
        N::MulTen => {
            emit_mul_ten(s, ix, 0, 1, 2, 3);
        }
    }
    s.end();
    f
}

/// `ffp2a(x, prec)` into `dst`; returns the length.
/// x=0 prec=1 dst=2; ndig=3 e=4 o=5 r=6 i=7 d=8
fn ffp2a(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(1).i32_const(0).i32_le_s().if_(ri()).i32_const(1).else_();
    s.local_get(1).i32_const(22).i32_gt_s().if_(ri()).i32_const(23).else_().local_get(1).i32_const(1).i32_add().end();
    s.end().local_set(3);
    fcmp_c(s, ix, 0, 0);
    s.i32_const(0).i32_lt_s().if_(e());
    outc(s, 2, 5, b'-');
    // ffp_neg: a zero low byte is left alone.
    s.local_get(0).i32_const(0x80).i32_xor().local_get(0).local_get(0).i32_const(0xFF).i32_and().select().local_set(0);
    s.end();
    // Scaled to [1, 10) (cached: Str$ converts the same value twice).
    s.local_get(0).call(ix.n(N::Norm));
    hdr_addr(s, layout::NORM_XN);
    s.i32_load(MemArg { offset: 0, align: 2, memory_index: 0 }).local_set(0);
    hdr_addr(s, layout::NORM_E);
    s.i32_load(MemArg { offset: 0, align: 2, memory_index: 0 }).local_set(4);
    s.local_get(3).local_get(4).i32_add().i32_extend16_s().local_set(3);
    round_half(s, ix, 0, 3);
    s.local_set(0);
    fcmp_c(s, ix, 0, TEN);
    s.i32_const(0).i32_ge_s().if_(e());
    s.i32_const(ONE).local_set(0);
    s.local_get(4).i32_const(1).i32_add().local_set(4);
    s.end();
    s.local_get(4).i32_const(0).i32_lt_s().if_(e());
    {
        outc(s, 2, 5, b'0');
        outc(s, 2, 5, b'.');
        s.local_get(3).i32_const(0).i32_lt_s().if_(e());
        s.local_get(4).local_get(3).i32_sub().local_set(4);
        s.end();
        s.i32_const(-1).local_set(7);
        s.block(e()).loop_(e());
        s.local_get(7).local_get(4).i32_le_s().br_if(1);
        outc(s, 2, 5, b'0');
        s.local_get(7).i32_const(1).i32_sub().local_set(7);
        s.br(0).end().end();
    }
    s.end();
    s.i32_const(0).local_set(7);
    s.block(e()).loop_(e());
    {
        s.local_get(7).local_get(3).i32_ge_s().br_if(1);
        // The digit d=8 and the next x: (x - d) * 10, done with integers for
        // a positive normalised x below 16 (`ffp::sub_int_part`,
        // `ffp::mul_ten`), else by `sub_int`. eb=9 sh=10 m=11 frac=12 z=13
        // c=14
        s.block(e());
        {
            s.block(e());
            s.local_get(0).i32_const(0).i32_ge_s().br_if(0);
            s.local_get(0).i32_const(0xFF).i32_and().local_tee(9).i32_const(0x41).i32_sub().i32_const(3).i32_le_u();
            s.if_(e());
            {
                s.i32_const(0x58).local_get(9).i32_sub().local_set(10);
                s.local_get(0).i32_const(8).i32_shr_u().local_tee(11).local_get(10).i32_shr_u().local_set(8);
                s.local_get(11).i32_const(1).local_get(10).i32_shl().i32_const(1).i32_sub().i32_and().local_tee(12);
                s.i32_eqz().if_(e()).i32_const(0).local_set(0).br(3).end();
                s.local_get(12).i32_clz().i32_const(8).i32_sub().local_set(13);
                s.local_get(12).local_get(13).i32_shl().i32_const(8).i32_shl();
                s.local_get(9).local_get(13).i32_sub().i32_or().local_set(0);
                emit_mul_ten(s, ix, 0, 11, 9, 14);
                s.local_set(0).br(2);
            }
            s.end();
            s.local_get(9).i32_const(2).i32_sub().i32_const(0x3E).i32_le_u().if_(e());
            {
                s.i32_const(0).local_set(8);
                emit_mul_ten(s, ix, 0, 11, 9, 14);
                s.local_set(0).br(2);
            }
            s.end();
            s.end();
            s.local_get(0).call(ix.h(H::ToLong)).i32_extend16_s().local_set(8);
            s.local_get(0).local_get(8).call(ix.n(N::SubInt)).local_set(0);
        }
        s.end();
        out(s, 2, 5, |s| {
            s.local_get(8).i32_const(48).i32_add();
        });
        s.local_get(7).local_get(4).i32_eq().if_(e());
        outc(s, 2, 5, b'.');
        s.end();
        s.local_get(7).i32_const(1).i32_add().local_set(7);
        s.br(0);
    }
    s.end().end();
    s.local_get(5);
}

/// `Clean(x, dec)` into `dst`. x=0 dec=1 dst=2; len=3 dot=4 end=5 k=6
fn clean(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(0).local_get(1).local_get(2).call(ix.n(N::Ffp2a)).local_set(3);
    s.i32_const(-1).local_set(4);
    s.i32_const(0).local_set(6);
    s.block(e()).loop_(e());
    s.local_get(6).local_get(3).i32_ge_u().br_if(1);
    byte(s, 2, 6);
    s.i32_const(b'.' as i32).i32_eq().if_(e()).local_get(6).local_set(4).br(2).end();
    s.local_get(6).i32_const(1).i32_add().local_set(6);
    s.br(0).end().end();
    s.local_get(4).i32_const(0).i32_lt_s().if_(e()).local_get(3).return_().end();
    s.local_get(4).local_set(5);
    s.local_get(4).i32_const(1).i32_add().local_set(6);
    s.block(e()).loop_(e());
    s.local_get(6).local_get(3).i32_ge_u().br_if(1);
    byte(s, 2, 6);
    s.i32_const(b'0' as i32).i32_ne().if_(e()).local_get(6).i32_const(1).i32_add().local_set(5).end();
    s.local_get(6).i32_const(1).i32_add().local_set(6);
    s.br(0).end().end();
    s.local_get(5);
}

/// `push_exp(out, sign, d2)`: dst=0 o=1 sign=2 d2=3; returns o.
fn push_exp(s: &mut InstructionSink) {
    outc(s, 0, 1, b'E');
    out(s, 0, 1, |s| {
        s.local_get(2);
    });
    outc(s, 0, 1, b'0');
    s.block(e()).loop_(e());
    s.local_get(3).i32_const(0xFF).i32_and().i32_const(10).i32_lt_u().br_if(1);
    // *out.last_mut() += 1
    s.local_get(0).local_get(1).i32_add().i32_const(1).i32_sub();
    s.local_get(0).local_get(1).i32_add().i32_const(1).i32_sub().i32_load8_u(b8()).i32_const(1).i32_add();
    s.i32_store8(b8());
    s.local_get(3).i32_const(10).i32_sub().local_set(3);
    s.br(0).end().end();
    out(s, 0, 1, |s| {
        s.local_get(3).i32_const(0xFF).i32_and().i32_const(48).i32_add();
    });
    s.local_get(1);
}

/// `ExFix1`. x=0 n=1 fix=2 dst=3; d2=4 n2=5 slen=6 o=7 p=8 dot=9 d1=10
/// c=11 end=12 k=13 a=14
fn ex_fix(s: &mut InstructionSink, ix: &Idx) {
    hdr_addr(s, layout::SCR_A);
    s.local_set(14);
    s.local_get(1).i32_const(2).i32_sub().local_set(4);
    s.local_get(1).i32_const(7).local_get(1).i32_const(7).i32_lt_s().select().local_set(5);
    s.local_get(0).i32_const(9).local_get(5).i32_sub().local_get(14).call(ix.n(N::Ffp2a)).local_set(6);
    s.i32_const(0).local_set(7);
    s.i32_const(0).local_set(8);
    at(s, 14, 6, 8);
    s.i32_const(b'-' as i32).i32_eq().if_(e());
    outc(s, 3, 7, b'-');
    s.i32_const(1).local_set(8);
    s.end();
    at(s, 14, 6, 8);
    s.local_set(11);
    out(s, 3, 7, |s| {
        s.local_get(11);
    });
    s.local_get(8).i32_const(1).i32_add().local_set(8);
    outc(s, 3, 7, b'.');
    s.local_get(7).i32_const(1).i32_sub().local_set(9);
    s.local_get(2).i32_const(5).local_get(2).i32_const(5).i32_lt_u().select().local_set(10);
    s.block(e()).loop_(e());
    {
        at(s, 14, 6, 8);
        s.local_set(11);
        s.local_get(8).i32_const(1).i32_add().local_set(8);
        s.local_get(11).i32_eqz().br_if(1);
        s.local_get(11).i32_const(b'.' as i32).i32_eq().br_if(0);
        out(s, 3, 7, |s| {
            s.local_get(11);
        });
        s.local_get(10).i32_const(1).i32_sub().i32_extend16_s().local_tee(10).i32_eqz().br_if(1);
        s.br(0);
    }
    s.end().end();
    s.local_get(2).i32_const(0).i32_lt_s().if_(e());
    {
        s.local_get(9).local_set(12);
        s.local_get(9).i32_const(1).i32_add().local_set(13);
        s.block(e()).loop_(e());
        s.local_get(13).local_get(7).i32_ge_u().br_if(1);
        byte(s, 3, 13);
        s.i32_const(b'0' as i32).i32_ne().if_(e()).local_get(13).i32_const(1).i32_add().local_set(12).end();
        s.local_get(13).i32_const(1).i32_add().local_set(13);
        s.br(0).end().end();
        s.local_get(12).local_set(7);
    }
    s.end();
    s.local_get(3).local_get(7).i32_const(b'+' as i32).local_get(4).call(ix.n(N::PushExp));
}

/// `ExVir1`. x=0 z=1 fix=2 zero=3 dst=4; slen=5 d2=6 o=7 p=8 ap=9 first=10
/// c=11 d1=12 keep=13 a=14
fn ex_vir(s: &mut InstructionSink, ix: &Idx) {
    hdr_addr(s, layout::SCR_A);
    s.local_set(14);
    s.local_get(3).if_(e());
    {
        for (k, &c) in b"0.0000000".iter().enumerate() {
            s.local_get(14).i32_const(c as i32).i32_store8(MemArg { offset: k as u64, align: 0, memory_index: 0 });
        }
        s.i32_const(9).local_set(5);
        s.i32_const(0).local_set(6);
    }
    s.else_();
    {
        s.local_get(0).local_get(1).i32_const(6).i32_add().local_get(14).call(ix.n(N::Ffp2a)).local_set(5);
        s.local_get(1).local_set(6);
    }
    s.end();
    s.i32_const(0).local_set(7);
    s.i32_const(0).local_set(8);
    at(s, 14, 5, 8);
    s.i32_const(b'-' as i32).i32_eq().if_(e());
    outc(s, 4, 7, b'-');
    s.i32_const(1).local_set(8);
    s.end();
    s.local_get(8).i32_const(2).i32_add().local_set(9);
    s.block(e()).loop_(e());
    {
        at(s, 14, 5, 8);
        s.local_set(11);
        s.local_get(8).i32_const(1).i32_add().local_set(8);
        s.local_get(11).i32_eqz().if_(e());
        s.local_get(9).local_set(8);
        s.i32_const(b'0' as i32).local_set(10);
        s.br(2);
        s.end();
        s.local_get(11).i32_const(b'.' as i32).i32_eq().local_get(11).i32_const(b'0' as i32).i32_eq().i32_or().br_if(0);
        s.local_get(11).local_set(10);
        s.br(1);
    }
    s.end().end();
    s.local_get(2).i32_const(6).local_get(2).i32_const(6).i32_lt_u().select().local_set(12);
    out(s, 4, 7, |s| {
        s.local_get(10);
    });
    outc(s, 4, 7, b'.');
    s.local_get(7).i32_const(1).i32_sub().local_set(13);
    s.block(e()).loop_(e());
    {
        s.local_get(12).i32_eqz().br_if(1);
        at(s, 14, 5, 8);
        s.local_set(11);
        s.local_get(8).i32_const(1).i32_add().local_set(8);
        s.local_get(11).if_(e());
        out(s, 4, 7, |s| {
            s.local_get(11);
        });
        s.local_get(11).i32_const(b'0' as i32).i32_ne().if_(e()).local_get(7).local_set(13).end();
        s.end();
        s.local_get(12).i32_const(1).i32_sub().local_set(12);
        s.br(0);
    }
    s.end().end();
    s.local_get(2).i32_const(0).i32_lt_s().if_(e()).local_get(13).local_set(7).end();
    s.local_get(3).if_(ri());
    s.local_get(4).local_get(7).i32_const(b'+' as i32).i32_const(0).call(ix.n(N::PushExp));
    s.else_();
    s.local_get(4).local_get(7).i32_const(b'-' as i32).local_get(6).call(ix.n(N::PushExp));
    s.end();
}

/// `F2a`. x=0 fix=1 exp=2 dst=3; f=4 len=5 eb=6 prec=7 slen=8 a1=9 p=10
/// c=11 n=12 d=13 start=14 z=15 zero=16 a=17 st=18
fn f2a(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(2).i32_eqz().local_get(1).i32_const(0).i32_ge_s().i32_and().if_(e());
    {
        // f = fix >= 8 ? 7 : fix
        s.i32_const(7).local_get(1).local_get(1).i32_const(8).i32_ge_s().select().local_set(4);
        s.local_get(0).local_get(4).local_get(3).call(ix.n(N::Ffp2a)).local_set(5);
        s.local_get(4).i32_eqz().local_get(5).i32_const(0).i32_gt_s().i32_and().if_(e());
        s.local_get(3).local_get(5).i32_add().i32_const(1).i32_sub().i32_load8_u(b8()).i32_const(b'.' as i32).i32_eq();
        s.if_(e()).local_get(5).i32_const(1).i32_sub().local_set(5).end();
        s.end();
        s.local_get(5).return_();
    }
    s.end();
    s.local_get(0).i32_const(0x7F).i32_and().local_set(6);
    s.local_get(6).i32_const(0x41).i32_ge_u().if_(ri()).i32_const(7).else_();
    s.local_get(6).i32_const(0x31).i32_ge_u().if_(ri()).i32_const(10).else_().i32_const(22).end();
    s.end().local_set(7);
    // What the text of ffp2a(x, prec) starts with (`ffp::ffp2a_shape`), st=18:
    // n > 0 or -z; read from the text for zero / odd values.
    s.local_get(0).local_get(7).call(ix.n(N::Shape)).local_set(18);
    s.local_get(18).i32_const(i32::MIN).i32_eq().if_(e());
    {
        hdr_addr(s, layout::SCR_A);
        s.local_set(17);
        s.local_get(0).local_get(7).local_get(17).call(ix.n(N::Ffp2a)).local_set(8);
        s.i32_const(0).local_set(10);
        at(s, 17, 8, 10);
        s.i32_const(b'-' as i32).i32_eq().local_set(9);
        at(s, 17, 8, 9);
        s.i32_const(b'0' as i32).i32_ne().if_(e());
        {
            s.local_get(9).local_set(10);
            s.block(e()).loop_(e());
            at(s, 17, 8, 10);
            s.local_set(11);
            s.local_get(10).i32_const(1).i32_add().local_set(10);
            s.local_get(11).i32_eqz().local_get(11).i32_const(b'.' as i32).i32_eq().i32_or().br_if(1);
            s.br(0).end().end();
            s.local_get(10).local_get(9).i32_sub().local_set(18);
        }
        s.else_();
        {
            s.local_get(9).i32_const(2).i32_add().local_tee(14).local_set(10);
            s.block(e()).loop_(e());
            at(s, 17, 8, 10);
            s.local_set(11);
            s.local_get(10).i32_const(1).i32_add().local_set(10);
            s.local_get(11).i32_eqz().local_get(11).i32_const(b'0' as i32).i32_ne().i32_or().br_if(1);
            s.br(0).end().end();
            s.local_get(14).local_get(10).i32_sub().local_set(18);
        }
        s.end();
    }
    s.end();
    s.local_get(18).i32_const(0).i32_gt_s().if_(e());
    {
        s.local_get(18).local_set(12);
        s.local_get(2).i32_const(0).i32_ne().local_get(12).i32_const(8).i32_ge_s().i32_or().if_(e());
        s.local_get(0).local_get(12).local_get(1).local_get(3).call(ix.n(N::ExFix)).return_();
        s.end();
        // d = min(7 - n, 5)
        s.i32_const(7)
            .local_get(12)
            .i32_sub()
            .local_tee(13)
            .i32_const(5)
            .local_get(13)
            .i32_const(5)
            .i32_lt_s()
            .select();
        s.local_set(13);
        s.local_get(0).local_get(13).local_get(3).call(ix.n(N::Clean)).return_();
    }
    s.end();
    s.i32_const(0).local_get(18).i32_sub().local_set(15);
    s.i32_const(0).local_set(16);
    s.local_get(15).i32_const(22).i32_ge_s().if_(e());
    s.i32_const(6).local_set(15);
    s.i32_const(1).local_set(16);
    s.else_();
    s.local_get(15).i32_const(4).i32_ge_s().if_(e());
    s.local_get(0).local_get(15).local_get(1).i32_const(0).local_get(3).call(ix.n(N::ExVir)).return_();
    s.end();
    s.end();
    s.local_get(2).if_(e());
    s.local_get(0).local_get(15).local_get(1).local_get(16).local_get(3).call(ix.n(N::ExVir)).return_();
    s.end();
    s.local_get(0).local_get(15).i32_const(6).i32_add().local_get(3).call(ix.n(N::Clean));
}

/// `FloatToAsc` with the sign space; returns a new string.
/// x=0 fix=1 exp=2; b=3 sl=4 r=5 o=6 dot=7 k=8 end=9 last=10
fn float_to_asc(s: &mut InstructionSink, ix: &Idx) {
    hdr_addr(s, layout::SCR_B);
    s.local_set(3);
    s.local_get(0).local_get(1).local_get(2).local_get(3).call(ix.n(N::F2a)).local_set(4);
    s.local_get(4).i32_const(2).i32_add().call(ix.s(S::Alloc)).i32_const(4).i32_add().local_set(5);
    s.i32_const(0).local_set(6);
    // Copies b[from..to] to the output.
    let copy_range =
        |s: &mut InstructionSink, from: &dyn Fn(&mut InstructionSink), to: &dyn Fn(&mut InstructionSink)| {
            s.local_get(5).local_get(6).i32_add();
            s.local_get(3);
            from(s);
            s.i32_add();
            to(s);
            from(s);
            s.i32_sub();
            copy(s);
            to(s);
            from(s);
            s.i32_sub().local_get(6).i32_add().local_set(6);
        };
    s.local_get(4)
        .i32_eqz()
        .if_(ri())
        .i32_const(1)
        .else_()
        .local_get(3)
        .i32_load8_u(b8())
        .i32_const(b'-' as i32)
        .i32_ne()
        .end();
    s.if_(e());
    outc(s, 5, 6, b' ');
    s.end();
    s.block(e());
    {
        s.local_get(1).i32_const(0).i32_ge_s().if_(e());
        copy_range(
            s,
            &|s| {
                s.i32_const(0);
            },
            &|s| {
                s.local_get(4);
            },
        );
        s.br(1).end();
        // Find the point.
        s.i32_const(-1).local_set(7);
        s.i32_const(0).local_set(8);
        s.block(e()).loop_(e());
        s.local_get(8).local_get(4).i32_ge_u().br_if(1);
        byte(s, 3, 8);
        s.i32_const(b'.' as i32).i32_eq().if_(e()).local_get(8).local_set(7).br(2).end();
        s.local_get(8).i32_const(1).i32_add().local_set(8);
        s.br(0).end().end();
        s.local_get(7).i32_const(0).i32_lt_s().if_(e());
        copy_range(
            s,
            &|s| {
                s.i32_const(0);
            },
            &|s| {
                s.local_get(4);
            },
        );
        s.br(1).end();
        copy_range(
            s,
            &|s| {
                s.i32_const(0);
            },
            &|s| {
                s.local_get(7);
            },
        );
        s.local_get(7).i32_const(1).i32_add().local_tee(9).local_set(10);
        s.block(e()).loop_(e());
        s.local_get(9).local_get(4).i32_ge_u().br_if(1);
        byte(s, 3, 9);
        s.i32_const(b'E' as i32).i32_eq().br_if(1);
        byte(s, 3, 9);
        s.i32_const(b'0' as i32).i32_ne().if_(e()).local_get(9).i32_const(1).i32_add().local_set(10).end();
        s.local_get(9).i32_const(1).i32_add().local_set(9);
        s.br(0).end().end();
        s.local_get(10).local_get(7).i32_const(1).i32_add().i32_ne().if_(e());
        outc(s, 5, 6, b'.');
        copy_range(
            s,
            &|s| {
                s.local_get(7).i32_const(1).i32_add();
            },
            &|s| {
                s.local_get(10);
            },
        );
        s.end();
        s.local_get(9).local_get(4).i32_lt_u().if_(e());
        outc(s, 5, 6, b' ');
        s.end();
        copy_range(
            s,
            &|s| {
                s.local_get(9);
            },
            &|s| {
                s.local_get(4);
            },
        );
    }
    s.end();
    // The string's length, then its address.
    s.local_get(5).i32_const(4).i32_sub().local_get(6).i32_store(MemArg { offset: 0, align: 2, memory_index: 0 });
    s.local_get(5).i32_const(4).i32_sub();
}

/// `dtoa(x, ndig, mode)` into `dst`; returns the length.
/// x=0(f64) ndig=1 mode=2 dst=3; o=4 d4=5 d6=6 d7=7 d5=8 a6=9 a3=10 d=11
/// k=12 z=13 point=14 last=15 end=16 to=17 c=18 sg=19; bits=20(i64) t=21(f64)
fn dtoa(s: &mut InstructionSink) {
    let ten = || f64::from_bits(DTAB[0]);
    let one = f64::from_bits(DTAB[1]);
    s.local_get(0).i64_reinterpret_f64().local_set(20);
    s.local_get(20).i64_const(52).i64_shr_u().i64_const(0x7FF).i64_and().i64_const(0x7FF).i64_eq().if_(e());
    {
        s.i32_const(b'-' as i32).i32_const(b'+' as i32).local_get(20).i64_const(0).i64_lt_s().select().local_set(18);
        s.i32_const(0).local_set(12);
        s.block(e()).loop_(e());
        s.local_get(12).local_get(1).i32_ge_s().br_if(1);
        out(s, 3, 4, |s| {
            s.local_get(18);
        });
        s.local_get(12).i32_const(1).i32_add().local_set(12);
        s.br(0).end().end();
        s.local_get(4).return_();
    }
    s.end();
    // sgn(x): 0 for a zero exponent, else -1 / 1.
    let sgn = |s: &mut InstructionSink| {
        s.local_get(0).i64_reinterpret_f64().i64_const(52).i64_shr_u().i64_const(0x7FF).i64_and().i64_eqz();
        s.if_(ri()).i32_const(0).else_();
        s.i32_const(-1).i32_const(1).local_get(0).i64_reinterpret_f64().i64_const(0).i64_lt_s().select();
        s.end();
    };
    sgn(s);
    s.i32_const(0).i32_lt_s().if_(e());
    s.local_get(0).f64_neg().local_set(0);
    outc(s, 3, 4, b'-');
    s.end();
    sgn(s);
    s.i32_const(0).i32_gt_s().if_(e());
    {
        s.block(e()).loop_(e());
        s.local_get(0).f64_const(one.into()).f64_ge().br_if(1);
        s.local_get(0).f64_const(ten().into()).f64_mul().local_set(0);
        s.local_get(5).i32_const(1).i32_sub().local_set(5);
        s.br(0).end().end();
        s.block(e()).loop_(e());
        s.local_get(0).f64_const(ten().into()).f64_lt().br_if(1);
        s.local_get(0).f64_const(ten().into()).f64_div().local_set(0);
        s.local_get(5).i32_const(1).i32_add().local_set(5);
        s.br(0).end().end();
    }
    s.else_();
    s.f64_const(0.0.into()).local_set(0);
    s.end();
    s.local_get(1).local_set(6);
    s.local_get(2).i32_const(3).i32_and().local_set(7);
    s.local_get(7).i32_const(2).i32_eq().if_(e());
    {
        s.local_get(6).i32_eqz().if_(e()).i32_const(1).local_set(6).end();
        s.local_get(5).i32_const(-4).i32_lt_s().local_get(5).local_get(6).i32_ge_s().i32_or();
        s.if_(e()).i32_const(-1).local_set(7).end();
        s.local_get(6).local_set(8);
    }
    s.else_();
    s.local_get(7).i32_const(1).i32_eq().if_(e());
    s.local_get(6).local_get(5).i32_add().i32_const(1).i32_add().local_set(8);
    s.else_();
    s.local_get(6).i32_const(1).i32_add().local_set(8);
    s.end();
    s.end();
    s.local_get(8).i32_const(0).i32_gt_s().if_(e());
    {
        // x += DTAB[min(d5, 16) + 1]
        s.local_get(8)
            .i32_const(16)
            .local_get(8)
            .i32_const(16)
            .i32_lt_s()
            .select()
            .i32_const(1)
            .i32_add()
            .local_set(12);
        s.f64_const(f64::from_bits(DTAB[17]).into()).local_set(21);
        for (k, &v) in DTAB.iter().enumerate().take(17).skip(2) {
            s.local_get(12).i32_const(k as i32).i32_eq().if_(e());
            s.f64_const(f64::from_bits(v).into()).local_set(21);
            s.end();
        }
        s.local_get(0).local_get(21).f64_add().local_set(0);
        s.local_get(0).f64_const(ten().into()).f64_ge().if_(e());
        s.f64_const(one.into()).local_set(0);
        s.local_get(5).i32_const(1).i32_add().local_set(5);
        s.local_get(7).i32_const(0).i32_gt_s().if_(e()).local_get(8).i32_const(1).i32_add().local_set(8).end();
        s.end();
    }
    s.end();
    s.local_get(7).i32_const(0).i32_gt_s().if_(e());
    {
        s.local_get(5).i32_const(0).i32_lt_s().if_(e());
        {
            outc(s, 3, 4, b'0');
            outc(s, 3, 4, b'.');
            s.local_get(8).i32_const(0).i32_gt_s().if_(ri());
            s.i32_const(0).local_get(5).i32_sub().i32_const(1).i32_sub();
            s.else_().local_get(6).end().local_set(13);
            s.block(e()).loop_(e());
            s.local_get(13).i32_const(0).i32_le_s().br_if(1);
            outc(s, 3, 4, b'0');
            s.local_get(13).i32_const(1).i32_sub().local_set(13);
            s.br(0).end().end();
            s.i32_const(0).local_set(9);
        }
        s.else_();
        s.local_get(5).i32_const(1).i32_add().local_set(9);
        s.end();
    }
    s.else_();
    s.i32_const(1).local_set(9);
    s.end();
    s.local_get(8).i32_const(0).i32_gt_s().if_(e());
    {
        s.i32_const(0).local_set(10);
        s.block(e()).loop_(e());
        {
            s.local_get(10).i32_const(16).i32_lt_s().if_(e());
            s.local_get(0).i32_trunc_sat_f64_s().local_set(11);
            out(s, 3, 4, |s| {
                s.local_get(11).i32_const(48).i32_add();
            });
            s.local_get(0).local_get(11).f64_convert_i32_s().f64_sub().f64_const(ten().into()).f64_mul().local_set(0);
            s.else_();
            outc(s, 3, 4, b'0');
            s.end();
            s.local_get(8).i32_const(1).i32_sub().local_tee(8).i32_eqz().br_if(1);
            s.local_get(9).if_(e());
            s.local_get(9).i32_const(1).i32_sub().local_tee(9).i32_eqz().if_(e());
            outc(s, 3, 4, b'.');
            s.end();
            s.end();
            s.local_get(10).i32_const(1).i32_add().local_set(10);
            s.br(0);
        }
        s.end().end();
    }
    s.end();
    s.local_get(9).if_(e());
    outc(s, 3, 4, b'.');
    s.end();
    s.local_get(7).i32_const(0).i32_le_s().if_(e());
    {
        outc(s, 3, 4, b'e');
        s.local_get(5).i32_const(0).i32_lt_s().if_(e());
        s.i32_const(0).local_get(5).i32_sub().local_set(5);
        outc(s, 3, 4, b'-');
        s.else_();
        outc(s, 3, 4, b'+');
        s.end();
        out(s, 3, 4, |s| {
            s.local_get(5).i32_const(100).i32_div_s().i32_const(48).i32_add();
        });
        out(s, 3, 4, |s| {
            s.local_get(5).i32_const(100).i32_rem_s().i32_const(10).i32_div_s().i32_const(48).i32_add();
        });
        out(s, 3, 4, |s| {
            s.local_get(5).i32_const(100).i32_rem_s().i32_const(10).i32_rem_s().i32_const(48).i32_add();
        });
    }
    s.end();
    s.local_get(2).i32_const(0xFF).i32_and().i32_const(2).i32_eq().if_(e());
    {
        // Remove the zeros after the point (and a bare point).
        s.i32_const(-1).local_set(14);
        s.i32_const(-1).local_set(15);
        s.local_get(4).local_set(16);
        s.i32_const(0).local_set(12);
        s.block(e()).loop_(e());
        {
            s.local_get(12).local_get(4).i32_ge_u().br_if(1);
            byte(s, 3, 12);
            s.local_set(18);
            s.local_get(18)
                .i32_const(b'e' as i32)
                .i32_eq()
                .local_get(18)
                .i32_const(b'E' as i32)
                .i32_eq()
                .i32_or()
                .if_(e());
            s.local_get(12).local_set(16).br(2);
            s.end();
            s.local_get(18).i32_const(b'.' as i32).i32_eq().if_(e());
            s.local_get(12).i32_const(1).i32_add().local_set(14);
            s.else_();
            s.local_get(18).i32_const(b'0' as i32).i32_ne().local_get(14).i32_const(0).i32_ge_s().i32_and().if_(e());
            s.local_get(12).i32_const(1).i32_add().local_set(15);
            s.end();
            s.end();
            s.local_get(12).i32_const(1).i32_add().local_set(12);
            s.br(0);
        }
        s.end().end();
        s.local_get(14).i32_const(0).i32_ge_s().if_(e());
        {
            s.local_get(15)
                .local_get(14)
                .i32_const(1)
                .i32_sub()
                .local_get(15)
                .i32_const(0)
                .i32_ge_s()
                .select()
                .local_set(17);
            s.local_get(3)
                .local_get(17)
                .i32_add()
                .local_get(3)
                .local_get(16)
                .i32_add()
                .local_get(4)
                .local_get(16)
                .i32_sub();
            copy(s);
            s.local_get(4).local_get(16).local_get(17).i32_sub().i32_sub().local_set(4);
        }
        s.end();
    }
    s.end();
    s.local_get(4);
}

/// `format_double(v, fix)`: x=0 fixflg=1 expflg=2; mode=3 ndig=4 b=5 len=6
/// sp=7 r=8
fn format_double(s: &mut InstructionSink, ix: &Idx) {
    s.i32_const(2).local_set(3);
    s.i32_const(15).local_set(4);
    s.local_get(1).i32_const(0).i32_ge_s().if_(e());
    s.local_get(1).local_set(4);
    s.local_get(2).if_(e()).i32_const(0).local_set(3).end();
    s.end();
    hdr_addr(s, layout::SCR_B);
    s.local_set(5);
    s.local_get(0).i64_reinterpret_f64().i64_const(0).i64_ge_s().local_set(7);
    s.local_get(0).local_get(4).local_get(3).local_get(5).call(ix.n(N::Dtoa)).local_set(6);
    s.local_get(6).local_get(7).i32_add().i32_eqz().if_(e()).i32_const(0).return_().end();
    s.local_get(6).local_get(7).i32_add().call(ix.s(S::Alloc)).local_set(8);
    s.local_get(7).if_(e());
    s.local_get(8).i32_const(b' ' as i32).i32_store8(MemArg { offset: 4, align: 0, memory_index: 0 });
    s.end();
    s.local_get(8).i32_const(4).i32_add().local_get(7).i32_add().local_get(5).local_get(6);
    copy(s);
    s.local_get(8);
}

/// `renormalise(x)`: x=0; neg=1 e=2 n=3
fn renorm(s: &mut InstructionSink, ix: &Idx) {
    // A normalised value comes back unchanged (`ffp::renormalise`).
    s.local_get(0).i32_const(0).i32_lt_s().local_get(0).i32_const(0x7F).i32_and().i32_const(0).i32_ne().i32_and();
    s.if_(e()).local_get(0).return_().end();
    fcmp_c(s, ix, 0, 0);
    s.i32_eqz().if_(e()).i32_const(0).return_().end();
    fcmp_c(s, ix, 0, 0);
    s.i32_const(0).i32_lt_s().if_(e());
    // ffp_neg: a zero low byte is left alone.
    s.local_get(0).i32_const(0x80).i32_xor().local_get(0).local_get(0).i32_const(0xFF).i32_and().select().local_set(0);
    s.i32_const(1).local_set(1);
    s.end();
    s.block(e()).loop_(e());
    fcmp_c(s, ix, 0, ONE);
    s.i32_const(0).i32_lt_s().br_if(1);
    s.local_get(2).i32_const(1).i32_add().local_set(2);
    s.local_get(0).i32_const(TWO).call(ix.h(H::Div)).local_set(0);
    s.br(0).end().end();
    s.block(e()).loop_(e());
    fcmp_c(s, ix, 0, HALF);
    s.i32_const(0).i32_ge_s().br_if(1);
    s.local_get(2).i32_const(1).i32_sub().local_set(2);
    s.local_get(0).i32_const(TWO).call(ix.h(H::Mul)).local_set(0);
    s.br(0).end().end();
    s.local_get(0).i32_const(TWO24).call(ix.h(H::Mul)).call(ix.h(H::ToLong)).i32_const(8).i32_shl();
    s.local_get(2).i32_const(0x40).i32_add().i32_const(0x7F).i32_and().i32_or();
    s.local_get(1).i32_const(7).i32_shl().i32_or();
}

/// `ascii_to_ffp(text)`: text=0 len=1; i=2 neg=3 fr=4 a5=5 dots=6 fd=7
/// eneg=8 a4=9 v=10 p=11 q=12 eneg2=13 e=14 pw=15 scale=16 t=17 c=18 vi=19
/// exact=20
fn a2ffp(s: &mut InstructionSink, ix: &Idx) {
    let fr_at = |s: &mut InstructionSink, idx: u32| {
        s.local_get(4).local_get(idx).i32_add().i32_load8_u(b8());
    };
    s.block(e()).loop_(e());
    at(s, 0, 1, 2);
    s.local_tee(18).i32_const(b' ' as i32).i32_ne().local_get(18).i32_const(9).i32_ne().i32_and().br_if(1);
    s.local_get(2).i32_const(1).i32_add().local_set(2);
    s.br(0).end().end();
    at(s, 0, 1, 2);
    s.local_tee(18).i32_const(b'-' as i32).i32_eq().local_set(3);
    s.local_get(18).i32_const(b'-' as i32).i32_eq().local_get(18).i32_const(b'+' as i32).i32_eq().i32_or();
    s.if_(e()).local_get(2).i32_const(1).i32_add().local_set(2).end();
    // The frame (zeroed): exponent at 0, mantissa from 4.
    s.local_get(1).i32_const(8).i32_add().call(ix.s(S::Alloc)).i32_const(4).i32_add().local_set(4);
    s.i32_const(4).local_set(5);
    s.block(e()).loop_(e());
    {
        at(s, 0, 1, 2);
        s.local_tee(18).i32_eqz().br_if(1);
        s.local_get(18).i32_const(b'e' as i32).i32_eq().local_get(18).i32_const(b'E' as i32).i32_eq().i32_or().br_if(1);
        s.local_get(18).i32_const(b'.' as i32).i32_eq().if_(e());
        s.local_get(6).i32_const(1).i32_add().local_set(6);
        s.else_();
        s.local_get(4).local_get(5).i32_add().local_get(18).i32_store8(b8());
        s.local_get(5).i32_const(1).i32_add().local_set(5);
        s.local_get(6).if_(e()).local_get(7).i32_const(1).i32_add().i32_extend16_s().local_set(7).end();
        s.end();
        s.local_get(2).i32_const(1).i32_add().local_set(2);
        s.br(0);
    }
    s.end().end();
    s.local_get(4).local_get(5).i32_add().i32_const(0).i32_store8(b8());
    at(s, 0, 1, 2);
    s.local_tee(18).i32_const(b'e' as i32).i32_eq().local_get(18).i32_const(b'E' as i32).i32_eq().i32_or().if_(e());
    {
        s.local_get(2).i32_const(1).i32_add().local_set(2);
        at(s, 0, 1, 2);
        s.local_tee(18).i32_const(b'-' as i32).i32_eq().local_set(8);
        s.local_get(18).i32_const(b'-' as i32).i32_eq().local_get(18).i32_const(b'+' as i32).i32_eq().i32_or();
        s.if_(e()).local_get(2).i32_const(1).i32_add().local_set(2).end();
        s.block(e()).loop_(e());
        at(s, 0, 1, 2);
        s.local_tee(18).i32_eqz().br_if(1);
        s.local_get(4).local_get(9).i32_add().local_get(18).i32_store8(b8());
        s.local_get(9).i32_const(1).i32_add().local_set(9);
        s.local_get(2).i32_const(1).i32_add().local_set(2);
        s.br(0).end().end();
    }
    s.end();
    s.local_get(4).local_get(9).i32_add().i32_const(0).i32_store8(b8());
    // Mantissa digits.
    // (Integers while below 2^24, as `ffp::ascii_to_ffp`.) vi=19 exact=20
    s.i32_const(4).local_set(11);
    s.i32_const(1).local_set(20);
    s.block(e()).loop_(e());
    {
        fr_at(s, 11);
        s.i32_const(48).i32_sub().local_tee(18).i32_const(9).i32_gt_u().br_if(1);
        s.local_get(11).i32_const(1).i32_add().local_set(11);
        s.local_get(20).if_(e());
        {
            s.local_get(19).i32_const(10).i32_mul().local_get(18).i32_add().local_tee(17);
            s.i32_const(amos_core::ffp::EXACT_INT as i32).i32_lt_u().if_(e());
            s.local_get(17).local_set(19).br(2);
            s.end();
            s.i32_const(0).local_set(20);
            s.local_get(19).call(ix.h(H::FromLong)).local_set(10);
        }
        s.end();
        s.local_get(10).i32_const(TEN).call(ix.h(H::Mul)).local_set(10);
        s.local_get(18).call(ix.h(H::FromLong)).local_get(10).call(ix.h(H::Add)).local_set(10);
        s.br(0);
    }
    s.end().end();
    s.local_get(20).if_(e()).local_get(19).call(ix.h(H::FromLong)).local_set(10).end();
    // Exponent.
    fr_at(s, 12);
    s.local_tee(18).i32_const(b'+' as i32).i32_eq().if_(e());
    s.i32_const(1).local_set(12);
    s.else_();
    s.local_get(18).i32_const(b'-' as i32).i32_eq().if_(e());
    s.i32_const(1).local_set(12);
    s.i32_const(1).local_set(13);
    s.end();
    s.end();
    s.block(e()).loop_(e());
    {
        fr_at(s, 12);
        s.i32_const(48).i32_sub().local_tee(18).i32_const(9).i32_gt_u().br_if(1);
        s.local_get(14).i32_const(10).i32_mul().local_get(18).i32_add().i32_extend16_s().local_set(14);
        s.local_get(12).i32_const(1).i32_add().local_set(12);
        s.br(0);
    }
    s.end().end();
    s.local_get(13).if_(e()).i32_const(0).local_get(14).i32_sub().i32_extend16_s().local_set(14).end();
    s.local_get(8).if_(ri()).i32_const(0).local_get(14).i32_sub().i32_extend16_s().else_().local_get(14).end();
    s.local_get(7).i32_sub().i32_extend16_s().local_set(15);
    // 10^pw (tables of the repeated multiplications / divisions).
    s.local_get(15).i32_const(0).i32_lt_s().if_(ri());
    s.i32_const(0).local_get(15).i32_sub().i32_const(0).call(ix.n(N::Pow10));
    s.else_();
    s.local_get(15).i32_const(1).call(ix.n(N::Pow10));
    s.end();
    s.local_set(16);
    s.local_get(16).local_get(10).call(ix.h(H::Mul)).call(ix.n(N::Renorm));
    s.local_get(3).i32_const(7).i32_shl().i32_or();
}

/// `Val` (`Interp::val` / `parse_number(s, true)`): the value as an f64
/// payload, its type in `TAG` (0 integer, 1 float).
/// a=0 double=1; len=2 i=3 neg=4 start=5 c=6 v=7 n=8 j=9 isf=10 dot=11 k=12
/// ef=13 digits=14 tb=15 tl=16 d=17; v64=18 nv=19 (i64); r=20 (f64)
fn val(s: &mut InstructionSink, ix: &Idx) {
    // Text: a + 4, length.
    s.local_get(0).call(ix.s(S::Len)).local_set(2);
    s.local_get(0).i32_const(4).i32_add().local_set(0);
    let ret_int = |s: &mut InstructionSink, v: &dyn Fn(&mut InstructionSink)| {
        s.global_get(0).i32_const(0).i32_store(MemArg { offset: layout::TAG as u64, align: 2, memory_index: 0 });
        v(s);
        s.f64_convert_i32_s().return_();
    };
    let skip_spaces = |s: &mut InstructionSink, idx: u32| {
        s.block(e()).loop_(e());
        at(s, 0, 2, idx);
        s.i32_const(b' ' as i32).i32_ne().br_if(1);
        s.local_get(idx).i32_const(1).i32_add().local_set(idx);
        s.br(0).end().end();
    };
    let is_digit = |s: &mut InstructionSink| {
        s.i32_const(48).i32_sub().i32_const(10).i32_lt_u();
    };
    skip_spaces(s, 3);
    at(s, 0, 2, 3);
    s.local_tee(6).i32_const(b'-' as i32).i32_eq().if_(e());
    s.i32_const(1).local_set(4);
    s.local_get(3).i32_const(1).i32_add().local_set(3);
    s.else_();
    s.local_get(6).i32_const(b'+' as i32).i32_eq().if_(e()).local_get(3).i32_const(1).i32_add().local_set(3).end();
    s.end();
    skip_spaces(s, 3);
    s.local_get(3).local_set(5);
    at(s, 0, 2, 3);
    s.local_set(6);
    // $hex / %binary
    for (prefix, bits, max) in [(b'$', 4, 9), (b'%', 1, 33)] {
        s.local_get(6).i32_const(prefix as i32).i32_eq().if_(e());
        {
            s.local_get(3).i32_const(1).i32_add().local_set(3);
            s.block(e()).loop_(e());
            {
                if bits == 1 {
                    skip_spaces(s, 3);
                }
                at(s, 0, 2, 3);
                s.local_set(6);
                if bits == 4 {
                    // digit value or break
                    s.local_get(6).i32_const(48).i32_sub().i32_const(10).i32_lt_u().if_(ri());
                    s.local_get(6).i32_const(48).i32_sub();
                    s.else_();
                    s.local_get(6).i32_const(32).i32_or().i32_const(b'a' as i32).i32_sub().i32_const(6).i32_lt_u();
                    s.if_(ri()).local_get(6).i32_const(32).i32_or().i32_const(b'a' as i32 - 10).i32_sub();
                    s.else_().i32_const(-1).end();
                    s.end();
                    s.local_tee(17).i32_const(0).i32_lt_s().br_if(1);
                } else {
                    s.local_get(6).i32_const(48).i32_sub().local_tee(17).i32_const(1).i32_gt_u().br_if(1);
                }
                s.local_get(8).i32_const(1).i32_add().local_tee(8).i32_const(max).i32_eq().if_(e());
                ret_int(s, &|s| {
                    s.i32_const(0);
                });
                s.end();
                s.local_get(7).i32_const(bits).i32_shl().local_get(17).i32_or().local_set(7);
                s.local_get(3).i32_const(1).i32_add().local_set(3);
                s.br(0);
            }
            s.end().end();
            s.local_get(8).i32_eqz().if_(e());
            ret_int(s, &|s| {
                s.i32_const(0);
            });
            s.end();
            ret_int(s, &|s| {
                s.i32_const(0).local_get(7).i32_sub().local_get(7).local_get(4).select();
            });
        }
        s.end();
    }
    // Decimal: '.' or a digit.
    s.local_get(6)
        .i32_const(b'.' as i32)
        .i32_ne()
        .local_get(6)
        .i32_const(48)
        .i32_sub()
        .i32_const(10)
        .i32_ge_u()
        .i32_and();
    s.if_(e());
    ret_int(s, &|s| {
        s.i32_const(0);
    });
    s.end();
    // Find the end and decide integer or float.
    s.local_get(3).local_set(9);
    s.block(e()).loop_(e());
    {
        at(s, 0, 2, 9);
        s.local_set(6);
        s.local_get(6).i32_const(b' ' as i32).i32_eq().local_get(6);
        is_digit(s);
        s.i32_or().if_(e()).local_get(9).i32_const(1).i32_add().local_set(9).br(1).end();
        s.local_get(6).i32_const(b'.' as i32).i32_eq().if_(e());
        s.local_get(11).br_if(2);
        s.i32_const(1).local_set(11);
        s.i32_const(1).local_set(10);
        s.local_get(9).i32_const(1).i32_add().local_set(9);
        s.br(1);
        s.end();
        s.local_get(6).i32_const(b'e' as i32).i32_eq().local_get(6).i32_const(b'E' as i32).i32_eq().i32_or().if_(e());
        {
            s.local_get(9).i32_const(1).i32_add().local_set(12);
            skip_spaces(s, 12);
            s.i32_const(0).local_set(13);
            at(s, 0, 2, 12);
            s.local_tee(17).i32_const(b'+' as i32).i32_eq().local_get(17).i32_const(b'-' as i32).i32_eq().i32_or();
            s.if_(e());
            s.i32_const(1).local_set(13);
            s.local_get(12).i32_const(1).i32_add().local_set(12);
            skip_spaces(s, 12);
            s.end();
            at(s, 0, 2, 12);
            is_digit(s);
            s.if_(e()).i32_const(1).local_set(13).end();
            s.local_get(13).if_(e());
            {
                s.i32_const(1).local_set(10);
                s.block(e()).loop_(e());
                at(s, 0, 2, 12);
                s.local_tee(17).i32_const(b' ' as i32).i32_eq().local_get(17);
                is_digit(s);
                s.i32_or().i32_eqz().br_if(1);
                s.local_get(12).i32_const(1).i32_add().local_set(12);
                s.br(0).end().end();
                s.local_get(12).local_set(9);
            }
            s.end();
        }
        s.end();
    }
    s.end().end();
    s.local_get(10).if_(e());
    {
        // The text without spaces, in lower case.
        s.i32_const(0).local_set(16);
        s.local_get(5).local_set(12);
        s.block(e()).loop_(e());
        s.local_get(12).local_get(9).i32_ge_u().br_if(1);
        at(s, 0, 2, 12);
        s.i32_const(b' ' as i32).i32_ne().if_(e()).local_get(16).i32_const(1).i32_add().local_set(16).end();
        s.local_get(12).i32_const(1).i32_add().local_set(12);
        s.br(0).end().end();
        s.local_get(16).call(ix.s(S::Alloc)).local_set(15);
        s.i32_const(0).local_set(17);
        s.local_get(5).local_set(12);
        s.block(e()).loop_(e());
        {
            s.local_get(12).local_get(9).i32_ge_u().br_if(1);
            at(s, 0, 2, 12);
            s.local_tee(6).i32_const(b' ' as i32).i32_ne().if_(e());
            s.local_get(15).local_get(17).i32_add();
            s.local_get(6).i32_const(b'E' as i32).i32_eq().if_(ri()).i32_const(b'e' as i32).else_().local_get(6).end();
            s.i32_store8(MemArg { offset: 4, align: 0, memory_index: 0 });
            s.local_get(17).i32_const(1).i32_add().local_set(17);
            s.end();
            s.local_get(12).i32_const(1).i32_add().local_set(12);
            s.br(0);
        }
        s.end().end();
        s.global_get(0).i32_const(1).i32_store(MemArg { offset: layout::TAG as u64, align: 2, memory_index: 0 });
        s.local_get(1).if_(e());
        s.local_get(15).call(ix.val_double).local_set(20);
        s.local_get(20).f64_neg().local_get(20).local_get(4).select().return_();
        s.end();
        s.local_get(15).i32_const(4).i32_add().local_get(16).call(ix.n(N::A2ffp));
        s.local_get(4).i32_const(7).i32_shl().i32_or().call(ix.h(H::B2f)).return_();
    }
    s.end();
    // Integer (overflow: Val is 0).
    s.local_get(3).local_set(12);
    s.block(e()).loop_(e());
    {
        at(s, 0, 2, 12);
        s.local_tee(6)
            .i32_const(b' ' as i32)
            .i32_eq()
            .if_(e())
            .local_get(12)
            .i32_const(1)
            .i32_add()
            .local_set(12)
            .br(1)
            .end();
        s.local_get(6);
        is_digit(s);
        s.i32_eqz().br_if(1);
        s.local_get(18)
            .i64_const(10)
            .i64_mul()
            .local_get(6)
            .i32_const(48)
            .i32_sub()
            .i64_extend_i32_u()
            .i64_add()
            .local_tee(19);
        s.i64_const(i32::MAX as i64).i64_gt_u().if_(e());
        ret_int(s, &|s| {
            s.i32_const(0);
        });
        s.end();
        s.local_get(19).local_set(18);
        s.local_get(14).i32_const(1).i32_add().local_set(14);
        s.local_get(12).i32_const(1).i32_add().local_set(12);
        s.br(0);
    }
    s.end().end();
    s.local_get(14).i32_eqz().if_(e());
    ret_int(s, &|s| {
        s.i32_const(0);
    });
    s.end();
    ret_int(s, &|s| {
        s.i32_const(0).local_get(18).i32_wrap_i64().i32_sub().local_get(18).i32_wrap_i64().local_get(4).select();
    });
    s.unreachable();
}

/// `format_radix(n, hex, digits)`: n=0 hex=1 digits=2; bits=3 maxd=4 r=5
/// o=6 i=7 d=8 cnt=9 mask=10
fn radix(s: &mut InstructionSink, ix: &Idx) {
    s.i32_const(4).i32_const(1).local_get(1).select().local_set(3);
    s.i32_const(8).i32_const(32).local_get(1).select().local_set(4);
    s.i32_const(15).i32_const(1).local_get(1).select().local_set(10);
    s.local_get(2).local_get(4).i32_le_u().if_(e());
    // (0 <= digits <= max as unsigned compare)
    s.local_get(2).local_set(9);
    s.else_();
    // Without leading zeros ("0" for 0).
    s.local_get(0).i32_eqz().if_(e());
    s.i32_const(1).local_set(9);
    s.else_();
    s.i32_const(32)
        .local_get(0)
        .i32_clz()
        .i32_sub()
        .local_get(3)
        .i32_add()
        .i32_const(1)
        .i32_sub()
        .local_get(3)
        .i32_div_u()
        .local_set(9);
    s.end();
    s.end();
    s.local_get(9).i32_const(1).i32_add().call(ix.s(S::Alloc)).i32_const(4).i32_add().local_set(5);
    s.local_get(5).i32_const(b'$' as i32).i32_const(b'%' as i32).local_get(1).select().i32_store8(b8());
    s.i32_const(1).local_set(6);
    s.local_get(9).local_set(7);
    s.block(e()).loop_(e());
    {
        s.local_get(7).i32_eqz().br_if(1);
        s.local_get(7).i32_const(1).i32_sub().local_set(7);
        s.local_get(0).local_get(7).local_get(3).i32_mul().i32_shr_u().local_get(10).i32_and().local_set(8);
        out(s, 5, 6, |s| {
            s.local_get(8)
                .i32_const(48)
                .i32_add()
                .local_get(8)
                .i32_const(55)
                .i32_add()
                .local_get(8)
                .i32_const(10)
                .i32_lt_u()
                .select();
        });
        s.br(0);
    }
    s.end().end();
    s.local_get(5).i32_const(4).i32_sub();
}

/// `Repeat$(a$, n)` (n checked): a=0 n=1; la=2 r=3 o=4
fn repeat(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(0).call(ix.s(S::Len)).local_set(2);
    s.local_get(2).i32_const(6).i32_add().call(ix.s(S::Alloc)).i32_const(4).i32_add().local_set(3);
    for (k, c) in [27u8, b'R', b'0'].into_iter().enumerate() {
        s.local_get(3).i32_const(c as i32).i32_store8(MemArg { offset: k as u64, align: 0, memory_index: 0 });
    }
    s.local_get(3).i32_const(3).i32_add().local_get(0).i32_const(4).i32_add().local_get(2);
    copy(s);
    s.local_get(3).local_get(2).i32_add().local_set(4);
    s.local_get(4).i32_const(27).i32_store8(MemArg { offset: 3, align: 0, memory_index: 0 });
    s.local_get(4).i32_const(b'R' as i32).i32_store8(MemArg { offset: 4, align: 0, memory_index: 0 });
    s.local_get(4).local_get(1).i32_const(48).i32_add().i32_store8(MemArg { offset: 5, align: 0, memory_index: 0 });
    s.local_get(3).i32_const(4).i32_sub();
}

/// `Str$` / formatting of a float (`Interp::format_float`) with the `Fix`
/// of the header: x=0 (f64) double=1; fix=2 exp=3
fn str_f(s: &mut InstructionSink, ix: &Idx) {
    s.global_get(0).i32_load(MemArg { offset: layout::FIX_FLG as u64, align: 2, memory_index: 0 }).local_set(2);
    s.global_get(0).i32_load(MemArg { offset: layout::EXP_FLG as u64, align: 2, memory_index: 0 }).local_set(3);
    s.local_get(1).if_(ri());
    s.local_get(0).local_get(2).local_get(3).call(ix.n(N::FmtDouble));
    s.else_();
    s.local_get(0).call(ix.h(H::F2b)).local_get(2).local_get(3).call(ix.n(N::FloatToAsc));
    s.end();
}

/// The FFP tables of `amos_core::ffp` (`POW10_UP`, `POW10_DOWN`,
/// `HALF_DOWN`), one after the other.
fn table() -> Vec<u32> {
    use amos_core::ffp::{HALF_DOWN, POW10_DOWN, POW10_UP};
    POW10_UP.iter().chain(POW10_DOWN.iter()).chain(HALF_DOWN.iter()).copied().collect()
}

const TLEN: i32 = amos_core::ffp::POW10_LEN as i32;

/// `tab(i)`: entry `i` of `table()` (a `br_table` over constants).
fn tab(s: &mut InstructionSink) {
    let t = table();
    let n = t.len() as u32;
    for _ in 0..=n {
        s.block(e());
    }
    s.local_get(0).br_table(0..n, n);
    for v in t {
        s.end();
        s.i32_const(v as i32).return_();
    }
    s.end();
    s.i32_const(0);
}

/// `pow10(k, up)`: 1.0 multiplied (`up`) or divided `k` times by 10 in FFP
/// (`ffp::pow10`). k=0 up=1; v=2
fn pow10(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(0).i32_const(TLEN).i32_lt_u().if_(e());
    s.local_get(0).local_get(1).if_(ri()).i32_const(0).else_().i32_const(TLEN).end().i32_add();
    s.call(ix.n(N::Tab)).return_();
    s.end();
    s.i32_const(TLEN - 1).local_get(1).if_(ri()).i32_const(0).else_().i32_const(TLEN).end().i32_add();
    s.call(ix.n(N::Tab)).local_set(2);
    s.local_get(0).i32_const(TLEN - 1).i32_sub().local_set(0);
    s.block(e()).loop_(e());
    s.local_get(0).i32_eqz().br_if(1);
    s.local_get(1).if_(e());
    s.local_get(2).i32_const(TEN).call(ix.h(H::Mul)).local_set(2);
    s.else_();
    s.local_get(2).i32_const(TEN).call(ix.h(H::Div)).local_set(2);
    s.end();
    s.local_get(0).i32_const(1).i32_sub().local_set(0);
    s.br(0).end().end();
    s.local_get(2);
}

/// `half_down(k)`: `pow10(k, false) / 2` (the rounding of `ffp2a`). k=0
fn half_down(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(0).i32_const(TLEN).i32_lt_u().if_(ri());
    s.local_get(0).i32_const(2 * TLEN).i32_add().call(ix.n(N::Tab));
    s.else_();
    s.local_get(0).i32_const(0).call(ix.n(N::Pow10)).i32_const(TWO).call(ix.h(H::Div));
    s.end();
}

/// Pushes `ffp_cmp(x, c) < 0` for a constant `c` with a positive exponent
/// byte: the exponent bytes compare as signed bytes, then the words.
fn emit_lt_const(s: &mut InstructionSink, x: u32, c: i32) {
    let cb = c & 0xFF;
    s.local_get(x).i32_extend8_s().i32_const(cb).i32_lt_s();
    s.local_get(x).i32_extend8_s().i32_const(cb).i32_eq().local_get(x).i32_const(c).i32_lt_s().i32_and();
    s.i32_or();
}

/// Pushes `ffp_div(x, TEN)` (`ffp::div_ten`). t1..t3: scratch locals.
fn emit_div_ten(s: &mut InstructionSink, ix: &Idx, x: u32, d4b: u32, m: u32, q1: u32) {
    s.block(ri());
    {
        s.local_get(x).i32_eqz().if_(e()).i32_const(0).br(1).end();
        // d4b = (x << 1 ^ 0x80) as i8 - 8, falling back on overflow.
        s.local_get(x).i32_const(1).i32_shl().i32_const(0x80).i32_xor().i32_extend8_s().i32_const(8).i32_sub();
        s.local_tee(d4b).i32_const(-128).i32_lt_s().if_(e());
        s.local_get(x).i32_const(TEN).call(ix.h(H::Div)).br(1);
        s.end();
        s.local_get(x).i32_const(!0xFF).i32_and().local_set(m);
        s.local_get(m).i32_const(16).i32_shr_u().i32_const(0xA000).i32_sub().i32_const(0x8000).i32_and().i32_eqz();
        s.if_(e());
        {
            s.local_get(d4b).i32_const(125).i32_gt_s().if_(e());
            s.local_get(x).i32_const(TEN).call(ix.h(H::Div)).br(2);
            s.end();
            s.local_get(d4b).i32_const(2).i32_add().local_set(d4b);
            s.local_get(m).i32_const(1).i32_shr_u().local_set(m);
        }
        s.end();
        s.local_get(m).i32_const(0xA000).i32_div_u().local_tee(q1).i32_const(0xFFFF).i32_gt_u().if_(e());
        s.local_get(x).i32_const(TEN).call(ix.h(H::Div)).br(1);
        s.end();
        // eb (into d4b) = sign | ((d4b ^ 0x80) & 0xFF) >> 1
        s.local_get(x).i32_const(0x80).i32_and();
        s.local_get(d4b)
            .i32_const(0x80)
            .i32_xor()
            .i32_const(0xFF)
            .i32_and()
            .i32_const(1)
            .i32_shr_u()
            .i32_or()
            .local_set(d4b);
        // d5 (into m) = q1 << 16 | ((m % $A000) << 16) / $A000
        s.local_get(q1).i32_const(16).i32_shl();
        s.local_get(m)
            .i32_const(0xA000)
            .i32_rem_u()
            .i32_const(16)
            .i32_shl()
            .i32_const(0xA000)
            .i32_div_u()
            .i32_or()
            .local_set(m);
        s.local_get(q1).i32_const(0x8000).i32_and().i32_eqz().if_(e());
        s.local_get(m).local_get(m).i32_add().local_set(m);
        s.local_get(d4b).i32_const(1).i32_sub().i32_const(0xFF).i32_and().local_set(d4b);
        s.end();
        s.local_get(d4b).i32_eqz().if_(e()).i32_const(0).br(1).end();
        s.local_get(m).i32_const(0x80).i32_add().i32_const(!0xFF).i32_and().local_get(d4b).i32_or();
    }
    s.end();
}

/// `norm(x)`: `ffp2a_norm` of a positive `x` into `NORM_XN` / `NORM_E`,
/// unless `x` is `NORM_KEY` (done last time). x=0; v=1 e=2 t=3..6
fn norm(s: &mut InstructionSink, ix: &Idx) {
    let ld = |s: &mut InstructionSink, off: u32| {
        hdr_addr(s, off);
        s.i32_load(MemArg { offset: 0, align: 2, memory_index: 0 });
    };
    let st = |s: &mut InstructionSink, off: u32, l: u32| {
        hdr_addr(s, off);
        s.local_get(l).i32_store(MemArg { offset: 0, align: 2, memory_index: 0 });
    };
    s.local_get(0);
    ld(s, layout::NORM_KEY);
    s.i32_eq().if_(e()).return_().end();
    s.local_get(0).local_set(1);
    fcmp_c(s, ix, 1, 0);
    s.i32_const(0).i32_gt_s().if_(e());
    {
        s.block(e()).loop_(e());
        emit_lt_const(s, 1, ONE);
        s.i32_eqz().br_if(1);
        s.local_get(1).call(ix.n(N::MulTen)).local_set(1);
        s.local_get(2).i32_const(1).i32_sub().local_set(2);
        s.br(0).end().end();
    }
    s.end();
    s.block(e()).loop_(e());
    emit_lt_const(s, 1, TEN);
    s.br_if(1);
    emit_div_ten(s, ix, 1, 3, 4, 5);
    s.local_set(1);
    s.local_get(2).i32_const(1).i32_add().local_set(2);
    s.br(0).end().end();
    st(s, layout::NORM_KEY, 0);
    st(s, layout::NORM_XN, 1);
    st(s, layout::NORM_E, 2);
}

/// Pushes `ndig` of `ffp2a` for precision `prec` and exponent `e`.
fn ndig(s: &mut InstructionSink, prec: u32, e: u32) {
    s.local_get(prec).i32_const(0).i32_le_s().if_(ri()).i32_const(1).else_();
    s.local_get(prec)
        .i32_const(22)
        .i32_gt_s()
        .if_(ri())
        .i32_const(23)
        .else_()
        .local_get(prec)
        .i32_const(1)
        .i32_add()
        .end();
    s.end();
    s.local_get(e).i32_add().i32_extend16_s();
}

/// Pushes `x + half_down(max(ndig - 1, 0))` (the rounding of `ffp2a`).
fn round_half(s: &mut InstructionSink, ix: &Idx, x: u32, ndig: u32) {
    s.local_get(x);
    s.local_get(ndig).i32_const(1).i32_sub().i32_const(0).local_get(ndig).i32_const(1).i32_gt_s().select();
    s.call(ix.n(N::HalfDown)).call(ix.h(H::Add));
}

/// `shape(x, prec)`: `ffp2a_shape` (`n` > 0, or `-z`), `i32::MIN` when `x`
/// is zero or not normalised (the caller reads the text instead).
/// x=0 prec=1; e=2 nd=3 v=4 t=5 u=6
fn shape(s: &mut InstructionSink, ix: &Idx) {
    s.local_get(0).i32_const(0).i32_ge_s().local_get(0).i32_const(0x7F).i32_and().i32_eqz().i32_or();
    s.if_(e()).i32_const(i32::MIN).return_().end();
    s.local_get(0).i32_const(!0x80).i32_and().call(ix.n(N::Norm));
    hdr_addr(s, layout::NORM_XN);
    s.i32_load(MemArg { offset: 0, align: 2, memory_index: 0 }).local_set(4);
    hdr_addr(s, layout::NORM_E);
    s.i32_load(MemArg { offset: 0, align: 2, memory_index: 0 }).local_set(2);
    ndig(s, 1, 2);
    s.local_set(3);
    round_half(s, ix, 4, 3);
    s.i32_const(TEN).call(ix.h(H::Cmp)).i32_const(0).i32_ge_s().if_(e());
    s.local_get(2).i32_const(1).i32_add().local_set(2);
    s.end();
    s.local_get(2).i32_const(0).i32_ge_s().if_(e()).local_get(2).i32_const(2).i32_add().return_().end();
    s.local_get(3).i32_const(0).i32_lt_s().if_(e());
    s.local_get(2).local_get(3).i32_sub().i32_extend16_s().local_set(2);
    s.end();
    // -(max(-1 - e, 0) + 1)
    s.i32_const(-1).local_get(2).i32_sub().local_tee(5).i32_const(0).local_get(5).i32_const(0).i32_gt_s().select();
    s.i32_const(1).i32_add().local_set(5);
    s.i32_const(0).local_get(5).i32_sub();
}

/// Pushes `ffp_mul(x, TEN)` (`ffp::mul_ten`): integer arithmetic for a
/// positive normalised `x` with exponent byte in [2, 0x7B].
fn emit_mul_ten(s: &mut InstructionSink, ix: &Idx, x: u32, d7: u32, eb: u32, c: u32) {
    s.block(ri());
    s.local_get(x).i32_const(0xFF).i32_and().local_tee(eb).i32_const(2).i32_sub().i32_const(0x79).i32_gt_u();
    s.local_get(x).i32_const(0).i32_ge_s().i32_or().if_(e());
    s.local_get(x).i32_const(TEN).call(ix.h(H::Mul)).br(1);
    s.end();
    s.local_get(x).i32_const(8).i32_shr_u().i32_const(160).i32_mul().local_set(d7);
    s.local_get(eb).i32_const(4).i32_add().local_set(eb);
    s.local_get(d7).i32_const(0).i32_lt_s().if_(e());
    s.local_get(d7).i32_const(0x80).i32_add().i32_const(!0xFF).i32_and().local_get(eb).i32_or().br(1);
    s.end();
    s.local_get(eb).i32_const(1).i32_sub().local_set(eb);
    s.local_get(d7).i32_const(0x40).i32_add().local_tee(d7).i32_const(0).i32_lt_s().local_set(c);
    s.local_get(d7).local_get(d7).i32_add().local_set(d7);
    s.local_get(c).if_(e());
    s.local_get(d7).i32_const(1).i32_shr_u().i32_const(i32::MIN).i32_or().local_set(d7);
    s.local_get(eb).i32_const(1).i32_add().local_set(eb);
    s.end();
    s.local_get(d7).i32_const(!0xFF).i32_and().local_get(eb).i32_or();
    s.end();
}

/// `sub_int(x, d)`: the step of `ffp2a`'s digit loop, `(x - d) * 10` with
/// `d` the integer part of `x` (`ffp::sub_int_part` then `ffp::mul_ten`).
/// x=0 d=1; eb=2 frac=3 sh=4 c=5
fn sub_int(s: &mut InstructionSink, ix: &Idx) {
    s.block(e());
    {
        s.local_get(0).i32_const(0xFF).i32_and().local_tee(2).i32_const(0x41).i32_sub().i32_const(3).i32_gt_u();
        s.local_get(0).i32_const(0).i32_ge_s().i32_or().if_(e());
        s.local_get(0).local_get(1).call(ix.h(H::FromLong)).call(ix.h(H::Sub)).local_set(0).br(1);
        s.end();
        s.local_get(0).i32_const(8).i32_shr_u();
        s.i32_const(1).i32_const(24 + 0x40).local_get(2).i32_sub().i32_shl().i32_const(1).i32_sub().i32_and();
        s.local_tee(3).i32_eqz().if_(e()).i32_const(0).return_().end();
        s.local_get(3).i32_clz().i32_const(8).i32_sub().local_set(4);
        s.local_get(3).local_get(4).i32_shl().i32_const(8).i32_shl();
        s.local_get(2).local_get(4).i32_sub().i32_or().local_set(0);
    }
    s.end();
    emit_mul_ten(s, ix, 0, 3, 2, 5);
}

/// A module exporting the helpers (and `a2ffp` etc.) for tests: imports
/// `env.memory`, `env.base`, `host.str_chunk` (i)->i and `host.val_double`
/// (i)->f.
pub fn test_module() -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, EntityType, ExportKind, ExportSection, FunctionSection, GlobalType, ImportSection, MemoryType,
        Module, TypeSection,
    };
    let mut types = TypeSection::new();
    let mut tys: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    let mut ty = |types: &mut TypeSection, p: &[ValType], r: &[ValType]| -> u32 {
        if let Some(i) = tys.iter().position(|(a, b)| a == p && b == r) {
            return i as u32;
        }
        types.ty().function(p.iter().copied(), r.iter().copied());
        tys.push((p.to_vec(), r.to_vec()));
        (tys.len() - 1) as u32
    };
    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None },
    );
    imports.import("env", "base", GlobalType { val_type: I, mutable: false, shared: false });
    let t = ty(&mut types, &[I], &[I]);
    imports.import("host", "str_chunk", EntityType::Function(t));
    let t = ty(&mut types, &[I], &[F]);
    imports.import("host", "val_double", EntityType::Function(t));
    let ffp = 2;
    let str = ffp + crate::ffp::HELPERS.len() as u32;
    let num = str + crate::strings::HELPERS.len() as u32;
    let ix = Idx { ffp, str, num, val_double: 1 };
    let mut funcs = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut exports = ExportSection::new();
    for (h, name, p, r) in crate::ffp::HELPERS {
        funcs.function(ty(&mut types, p, r));
        code.function(&crate::ffp::body(*h, ffp));
        exports.export(name, ExportKind::Func, ffp + *h as u32);
    }
    for (h, p, r) in crate::strings::HELPERS {
        funcs.function(ty(&mut types, p, r));
        code.function(&crate::strings::body(*h, str, 0));
    }
    for (n, name, p, r) in HELPERS {
        funcs.function(ty(&mut types, p, r));
        code.function(&body(*n, &ix, false));
        exports.export(name, ExportKind::Func, num + *n as u32);
    }
    let mut m = Module::new();
    m.section(&types).section(&imports).section(&funcs).section(&exports).section(&code);
    m.finish()
}
