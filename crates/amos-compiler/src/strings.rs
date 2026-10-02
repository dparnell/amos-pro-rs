//! String operations inside the module (strings in linear memory, see
//! `amos_core::compiled::runtime::strings`).
//!
//! A string is the absolute address of `[u32 length][bytes]`, 0 for the
//! empty string (never an allocated block of length 0). The semantics are
//! those of the interpreter (`interp/expr.rs`, `stmt.rs`); argument checks
//! that raise errors are done by the caller (which can branch to `$raise`),
//! except where they depend on the string (`Mid$`: -1 means error 23,
//! concatenation: -1 means error 21).

use amos_core::compiled::layout;
use wasm_encoder::{BlockType, Function, InstructionSink, MemArg, ValType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum S {
    Alloc,
    Len,
    Concat,
    Cmp,
    Asc,
    Chr,
    Left,
    Right,
    Mid,
    Instr,
    Case,
    Flip,
    Fill,
    Minus,
    StrI,
    MidSet,
}

const I: ValType = ValType::I32;
const L: ValType = ValType::I64;

pub const HELPERS: &[(S, &[ValType], &[ValType])] = &[
    (S::Alloc, &[I], &[I]),
    (S::Len, &[I], &[I]),
    (S::Concat, &[I, I], &[I]),
    (S::Cmp, &[I, I], &[I]),
    (S::Asc, &[I], &[I]),
    (S::Chr, &[I], &[I]),
    (S::Left, &[I, I], &[I]),
    (S::Right, &[I, I], &[I]),
    (S::Mid, &[I, I, I, I], &[I]),
    (S::Instr, &[I, I, I], &[I]),
    (S::Case, &[I, I], &[I]),
    (S::Flip, &[I], &[I]),
    (S::Fill, &[I, I], &[I]),
    (S::Minus, &[I, I], &[I]),
    (S::StrI, &[I], &[I]),
    (S::MidSet, &[I, I, I, I], &[I]),
];

fn m32(offset: u32) -> MemArg {
    MemArg { offset: offset as u64, align: 2, memory_index: 0 }
}

fn m8(offset: u32) -> MemArg {
    MemArg { offset: offset as u64, align: 0, memory_index: 0 }
}

fn e() -> BlockType {
    BlockType::Empty
}

/// Pushes the length of the string in local `a` (0 for the empty string).
fn len(s: &mut InstructionSink, a: u32) {
    s.local_get(a).if_(BlockType::Result(I)).local_get(a).i32_load(m32(0)).else_().i32_const(0).end();
}

/// memory.copy(dst, src, n) with the three values pushed by `f`.
fn copy(s: &mut InstructionSink) {
    s.memory_copy(0, 0);
}

/// The body of helper `h`. `first` is the function index of the first
/// helper, `chunk` the index of the `host.str_chunk` import.
pub fn body(h: S, first: u32, chunk: u32) -> Function {
    let call = |h: S| first + h as u32;
    let locals: &[(u32, ValType)] = match h {
        S::Alloc => &[(2, I)],
        S::Chr => &[(1, I)],
        S::Concat => &[(3, I)],
        S::Cmp => &[(5, I)],
        S::Left | S::Right => &[(2, I)],
        S::Mid => &[(4, I)],
        S::Instr => &[(5, I)],
        S::Case => &[(4, I)],
        S::Flip => &[(3, I)],
        S::Fill => &[(1, I)],
        S::Minus => &[(6, I)],
        S::StrI => &[(4, I), (2, L)],
        S::MidSet => &[(4, I)],
        _ => &[],
    };
    let mut f = Function::new(locals.iter().copied());
    let s = &mut f.instructions();
    match h {
        S::Alloc => {
            // len=0; size=1 p=2
            s.local_get(0).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(0).i32_const(7).i32_add().i32_const(-4).i32_and().local_set(1);
            s.global_get(0).i32_load(m32(layout::STR_PTR)).local_set(2);
            s.local_get(2).local_get(1).i32_add().global_get(0).i32_load(m32(layout::STR_END)).i32_gt_u();
            s.if_(e());
            {
                // A new chunk (the runtime may stop the program: no memory).
                s.local_get(1).call(chunk).i32_const(-1).i32_ne().if_(e()).unreachable().end();
                s.global_get(0).i32_load(m32(layout::STR_PTR)).local_set(2);
            }
            s.end();
            s.local_get(2).local_get(0).i32_store(m32(0));
            s.global_get(0).local_get(2).local_get(1).i32_add().i32_store(m32(layout::STR_PTR));
            s.local_get(2);
        }
        S::Len => len(s, 0),
        S::Concat => {
            // a=0 b=1; la=2 lb=3 r=4
            s.local_get(0).i32_eqz().if_(e()).local_get(1).return_().end();
            s.local_get(1).i32_eqz().if_(e()).local_get(0).return_().end();
            s.local_get(0).i32_load(m32(0)).local_set(2);
            s.local_get(1).i32_load(m32(0)).local_set(3);
            s.local_get(2).local_get(3).i32_add().i32_const(0xFFC0).i32_ge_u().if_(e()).i32_const(-1).return_().end();
            s.local_get(2).local_get(3).i32_add().call(call(S::Alloc)).local_set(4);
            s.local_get(4).i32_const(4).i32_add().local_get(0).i32_const(4).i32_add().local_get(2);
            copy(s);
            s.local_get(4)
                .i32_const(4)
                .i32_add()
                .local_get(2)
                .i32_add()
                .local_get(1)
                .i32_const(4)
                .i32_add()
                .local_get(3);
            copy(s);
            s.local_get(4);
        }
        S::Cmp => {
            // a=0 b=1; la=2 lb=3 n=4 i=5 ca=6
            s.local_get(0).local_get(1).i32_eq().if_(e()).i32_const(0).return_().end();
            len(s, 0);
            s.local_set(2);
            len(s, 1);
            s.local_set(3);
            s.local_get(2).local_get(3).local_get(2).local_get(3).i32_lt_u().select().local_set(4);
            s.i32_const(0).local_set(5);
            s.block(e()).loop_(e());
            s.local_get(5).local_get(4).i32_ge_u().br_if(1);
            s.local_get(0).local_get(5).i32_add().i32_load8_u(m8(4)).local_tee(6);
            s.local_get(1).local_get(5).i32_add().i32_load8_u(m8(4)).i32_ne().if_(e());
            s.i32_const(-1).i32_const(1).local_get(6).local_get(1).local_get(5).i32_add().i32_load8_u(m8(4)).i32_lt_u();
            s.select().return_().end();
            s.local_get(5).i32_const(1).i32_add().local_set(5);
            s.br(0);
            s.end().end();
            s.local_get(2).local_get(3).i32_gt_u().local_get(2).local_get(3).i32_lt_u().i32_sub();
        }
        S::Asc => {
            s.local_get(0).if_(BlockType::Result(I)).local_get(0).i32_load8_u(m8(4)).else_().i32_const(0).end();
        }
        S::Chr => {
            // n=0 (0..=255, checked); r=1
            s.i32_const(1).call(call(S::Alloc)).local_tee(1).local_get(0).i32_store8(m8(4)).local_get(1);
        }
        S::Left | S::Right => {
            // a=0 n=1 (>= 0); la=2 r=3
            len(s, 0);
            s.local_set(2);
            s.local_get(1).local_get(2).i32_ge_u().if_(e()).local_get(0).return_().end();
            s.local_get(1).call(call(S::Alloc)).local_set(3);
            s.local_get(3).i32_const(4).i32_add().local_get(0).i32_const(4).i32_add();
            if h == S::Right {
                s.local_get(2).local_get(1).i32_sub().i32_add();
            }
            s.local_get(1);
            copy(s);
            s.local_get(3);
        }
        S::Mid => {
            // a=0 p=1 n=2 has_n=3; start=4 la=5 end=6 r=7
            s.local_get(1).i32_const(0).i32_lt_s().if_(e()).i32_const(-1).return_().end();
            s.local_get(1)
                .i32_const(1)
                .local_get(1)
                .i32_const(1)
                .i32_gt_s()
                .select()
                .i32_const(1)
                .i32_sub()
                .local_set(4);
            len(s, 0);
            s.local_set(5);
            s.local_get(4).local_get(5).i32_ge_u().if_(e()).i32_const(0).return_().end();
            s.local_get(3).if_(e());
            {
                s.local_get(2).i32_eqz().if_(e()).i32_const(0).return_().end();
                s.local_get(2).i32_const(0).i32_lt_s().if_(e()).i32_const(-1).return_().end();
                // end = min(start + n, la) without overflow
                s.local_get(2).local_get(5).local_get(4).i32_sub().i32_ge_u().if_(e());
                s.local_get(5).local_set(6);
                s.else_();
                s.local_get(4).local_get(2).i32_add().local_set(6);
                s.end();
            }
            s.else_();
            s.local_get(5).local_set(6);
            s.end();
            s.local_get(6).local_get(4).i32_sub().call(call(S::Alloc)).local_set(7);
            s.local_get(7).i32_const(4).i32_add().local_get(0).i32_const(4).i32_add().local_get(4).i32_add();
            s.local_get(6).local_get(4).i32_sub();
            copy(s);
            s.local_get(7);
        }
        S::Instr => {
            // h=0 n=1 start=2 (>= 1); lh=3 ln=4 i=5 j=6 last=7
            len(s, 0);
            s.local_set(3);
            len(s, 1);
            s.local_set(4);
            s.local_get(4).i32_eqz().local_get(3).i32_eqz().i32_or().local_get(2).local_get(3).i32_gt_u().i32_or();
            s.if_(e()).i32_const(0).return_().end();
            s.local_get(4).local_get(3).i32_gt_u().if_(e()).i32_const(0).return_().end();
            s.local_get(3).local_get(4).i32_sub().local_set(7);
            s.local_get(2).i32_const(1).i32_sub().local_set(5);
            s.block(e()).loop_(e());
            {
                s.local_get(5).local_get(7).i32_gt_u().br_if(1);
                s.i32_const(0).local_set(6);
                s.block(e()).loop_(e());
                s.local_get(6).local_get(4).i32_ge_u().br_if(1);
                s.local_get(0).local_get(5).i32_add().local_get(6).i32_add().i32_load8_u(m8(4));
                s.local_get(1).local_get(6).i32_add().i32_load8_u(m8(4)).i32_ne().br_if(1);
                s.local_get(6).i32_const(1).i32_add().local_set(6);
                s.br(0);
                s.end().end();
                s.local_get(6).local_get(4).i32_eq().if_(e()).local_get(5).i32_const(1).i32_add().return_().end();
                s.local_get(5).i32_const(1).i32_add().local_set(5);
                s.br(0);
            }
            s.end().end();
            s.i32_const(0);
        }
        S::Case => {
            // a=0 lower=1; la=2 r=3 i=4 c=5
            len(s, 0);
            s.local_tee(2).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(2).call(call(S::Alloc)).local_set(3);
            s.i32_const(0).local_set(4);
            s.block(e()).loop_(e());
            s.local_get(4).local_get(2).i32_ge_u().br_if(1);
            s.local_get(0).local_get(4).i32_add().i32_load8_u(m8(4)).local_set(5);
            // c in the range to change: 'a'..='z' (upper) or 'A'..='Z' (lower)
            s.local_get(5).i32_const(b'A' as i32).i32_const(b'a' as i32).local_get(1).select().i32_sub();
            s.i32_const(26).i32_lt_u().if_(e());
            s.local_get(5).i32_const(32).i32_xor().local_set(5);
            s.end();
            s.local_get(3).local_get(4).i32_add().local_get(5).i32_store8(m8(4));
            s.local_get(4).i32_const(1).i32_add().local_set(4);
            s.br(0);
            s.end().end();
            s.local_get(3);
        }
        S::Flip => {
            // a=0; la=1 r=2 i=3
            len(s, 0);
            s.local_tee(1).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(1).call(call(S::Alloc)).local_set(2);
            s.i32_const(0).local_set(3);
            s.block(e()).loop_(e());
            s.local_get(3).local_get(1).i32_ge_u().br_if(1);
            s.local_get(2).local_get(1).i32_add().local_get(3).i32_sub().i32_const(1).i32_sub();
            s.local_get(0).local_get(3).i32_add().i32_load8_u(m8(4)).i32_store8(m8(4));
            s.local_get(3).i32_const(1).i32_add().local_set(3);
            s.br(0);
            s.end().end();
            s.local_get(2);
        }
        S::Fill => {
            // c=0 n=1 (0..=65535); r=2
            s.local_get(1).call(call(S::Alloc)).local_tee(2).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(2).i32_const(4).i32_add().local_get(0).local_get(1).memory_fill(0);
            s.local_get(2);
        }
        S::Minus => {
            // a=0 b=1; la=2 lb=3 r=4 len=5 i=6 j=7
            len(s, 1);
            s.local_tee(3).i32_eqz().if_(e()).local_get(0).return_().end();
            len(s, 0);
            s.local_tee(2).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(2).call(call(S::Alloc)).local_set(4);
            s.local_get(4).i32_const(4).i32_add().local_get(0).i32_const(4).i32_add().local_get(2);
            copy(s);
            s.local_get(2).local_set(5);
            // Remove the first occurrence, then search again from the start.
            s.block(e()).loop_(e());
            {
                s.local_get(3).local_get(5).i32_gt_u().br_if(1);
                s.i32_const(0).local_set(6);
                s.block(e()).loop_(e());
                {
                    s.local_get(6).local_get(5).local_get(3).i32_sub().i32_gt_u().br_if(3);
                    s.i32_const(0).local_set(7);
                    s.block(e()).loop_(e());
                    s.local_get(7).local_get(3).i32_ge_u().br_if(1);
                    s.local_get(4).local_get(6).i32_add().local_get(7).i32_add().i32_load8_u(m8(4));
                    s.local_get(1).local_get(7).i32_add().i32_load8_u(m8(4)).i32_ne().br_if(1);
                    s.local_get(7).i32_const(1).i32_add().local_set(7);
                    s.br(0);
                    s.end().end();
                    s.local_get(7).local_get(3).i32_eq().br_if(1);
                    s.local_get(6).i32_const(1).i32_add().local_set(6);
                    s.br(0);
                }
                s.end().end();
                // Found at i: close the gap.
                s.local_get(4).i32_const(4).i32_add().local_get(6).i32_add();
                s.local_get(4).i32_const(4).i32_add().local_get(6).i32_add().local_get(3).i32_add();
                s.local_get(5).local_get(6).i32_sub().local_get(3).i32_sub();
                copy(s);
                s.local_get(5).local_get(3).i32_sub().local_set(5);
                s.br(0);
            }
            s.end().end();
            s.local_get(5).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(4).local_get(5).i32_store(m32(0));
            s.local_get(4);
        }
        S::StrI => {
            // Str$ of an integer (`ffp::format_int`): " 12" / "-12".
            // n=0; len=1 r=2 i=3 digits=4; v=5 t=6 (i64)
            s.local_get(0).i64_extend_i32_s().local_tee(5).i64_const(0).i64_lt_s().if_(e());
            s.i64_const(0).local_get(5).i64_sub().local_set(5);
            s.end();
            s.i32_const(1).local_set(4);
            s.local_get(5).local_set(6);
            s.block(e()).loop_(e());
            s.local_get(6).i64_const(10).i64_lt_u().br_if(1);
            s.local_get(6).i64_const(10).i64_div_u().local_set(6);
            s.local_get(4).i32_const(1).i32_add().local_set(4);
            s.br(0);
            s.end().end();
            s.local_get(4).i32_const(1).i32_add().local_tee(1).call(call(S::Alloc)).local_set(2);
            s.local_get(2).i32_const(45).i32_const(32).local_get(0).i32_const(0).i32_lt_s().select().i32_store8(m8(4));
            s.local_get(1).i32_const(1).i32_sub().local_set(3);
            s.block(e()).loop_(e());
            {
                s.local_get(2).local_get(3).i32_add();
                s.local_get(5).i64_const(10).i64_rem_u().i32_wrap_i64().i32_const(48).i32_add().i32_store8(m8(4));
                s.local_get(5).i64_const(10).i64_div_u().local_set(5);
                s.local_get(3).i32_const(1).i32_sub().local_tee(3).i32_eqz().br_if(1);
                s.br(0);
            }
            s.end().end();
            s.local_get(2);
        }
        S::MidSet => {
            // cur=0 start=1 count=2 (unsigned) e=3; la=4 r=5 n=6 le=7
            len(s, 0);
            s.local_tee(4).i32_eqz().if_(e()).i32_const(0).return_().end();
            s.local_get(4).call(call(S::Alloc)).local_set(5);
            s.local_get(5).i32_const(4).i32_add().local_get(0).i32_const(4).i32_add().local_get(4);
            copy(s);
            s.local_get(1).local_get(4).i32_lt_u().if_(e());
            {
                len(s, 3);
                s.local_set(7);
                // n = min(count, la - start, le)
                s.local_get(4).local_get(1).i32_sub().local_set(6);
                s.local_get(2).local_get(6).local_get(2).local_get(6).i32_lt_u().select().local_set(6);
                s.local_get(7).local_get(6).local_get(7).local_get(6).i32_lt_u().select().local_set(6);
                s.local_get(5)
                    .i32_const(4)
                    .i32_add()
                    .local_get(1)
                    .i32_add()
                    .local_get(3)
                    .i32_const(4)
                    .i32_add()
                    .local_get(6);
                copy(s);
            }
            s.end();
            s.local_get(5);
        }
    }
    s.end();
    f
}
