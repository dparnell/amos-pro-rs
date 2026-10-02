//! Back end: emits the wasm module.
//!
//! All the program is in the exported function `run(budget) -> status`, a
//! dispatch loop over the instructions (the resume points):
//!
//! ```text
//! st = host.enter()
//! loop $dispatch
//!   st < 0: return running (-2) or stopped (-3);  point = st; budget...
//!   block $status
//!     block $raise
//!       block $end  block $p[n-1] ... block $p0
//!         br_table point
//!       end $p0  <instruction 0>
//!       end $p1  <instruction 1>   (instructions fall through in sequence)
//!       ...
//!       end $end  st = host.end_program(); br $status
//!     end $raise  st = host.raise(excpos, excode)
//!   end $status
//!   br $dispatch
//! end
//! ```
//!
//! Jumps set `st` to a point number (or a status) and branch to `$status`.
//! Variables live in linear memory (see `amos_core::compiled::layout`), so
//! `run` can return at any instruction and be called again.

use std::borrow::Cow;

use amos_core::compiled::layout::{self, Layout};
use amos_core::compiled::structure::{self, Instr};
use amos_core::errors;
use amos_core::interp::verify::{Compiled, GLOBAL};
use amos_core::tokens::tk;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, ImportSection, Instruction as W, MemArg, MemoryType, Module, TypeSection, ValType,
};

use crate::CompileError;
use crate::ir::*;
use crate::lower::is_comparison;

macro_rules! vt {
    (i) => {
        ValType::I32
    };
    (f) => {
        ValType::F64
    };
}

macro_rules! imports {
    ($($id:ident = $m:literal $n:literal ($($p:ident)*) $(-> $r:ident)?;)*) => {
        /// Imported functions, in import order (= function index).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(u32)]
        pub enum Imp { $($id),* }

        /// (module, name, parameters, results) of each import.
        pub const IMPORTS: &[(&str, &str, &[ValType], &[ValType])] = &[
            $(($m, $n, &[$(vt!($p)),*], &[$(vt!($r))?])),*
        ];
    };
}

imports! {
    Enter = "host" "enter" () -> i;
    Suspend = "host" "suspend" (i);
    EndProgram = "host" "end_program" (i) -> i;
    Raise = "host" "raise" (i i) -> i;
    TestPoint = "host" "test_point" (i) -> i;
    Interp = "host" "interp" (i) -> i;
    Keyword = "host" "keyword" (i) -> i;
    FnI = "host" "fn_i" (i i) -> i;
    FnF = "host" "fn_f" (i i) -> f;
    FnN = "host" "fn_n" (i i) -> f;
    FnS = "host" "fn_s" (i i) -> i;
    PushI = "host" "push_i" (i);
    PushF = "host" "push_f" (f);
    PushS = "host" "push_s" (i);
    PushN = "host" "push_n" (f i);
    PrintBegin = "host" "print_begin" ();
    PrintI = "host" "print_i" (i);
    PrintF = "host" "print_f" (f);
    PrintN = "host" "print_n" (f i);
    PrintS = "host" "print_s" (i);
    PrintTab = "host" "print_tab" ();
    PrintEnd = "host" "print_end" (i i) -> i;
    Aref = "host" "aref" (i i) -> i;
    AgetI = "host" "aget_i" (i i) -> i;
    AgetF = "host" "aget_f" (i i) -> f;
    AgetS = "host" "aget_s" (i i) -> i;
    AsetI = "host" "aset_i" (i i i);
    AsetF = "host" "aset_f" (i i f);
    AsetS = "host" "aset_s" (i i i);
    ForPush = "host" "for_push" (i i i i i i i) -> i;
    Next = "host" "next" (i) -> i;
    LoopPush = "host" "loop_push" (i i i i) -> i;
    WhilePush = "host" "while_push" (i i i) -> i;
    Until = "host" "until" (i i) -> i;
    Wend = "host" "wend" (i) -> i;
    LoopEnd = "host" "loop_end" (i) -> i;
    Exit = "host" "exit" (i i i) -> i;
    GotoPos = "host" "goto_pos" (i i) -> i;
    GotoLabel = "host" "goto_label" (i i) -> i;
    GosubLabel = "host" "gosub_label" (i i i) -> i;
    Return = "host" "return" (i) -> i;
    Call = "host" "call" (i i i i) -> i;
    ProcCheck = "host" "proc_check" (i i) -> i;
    SetParamI = "host" "set_param_i" (i);
    SetParamF = "host" "set_param_f" (f);
    SetParamS = "host" "set_param_s" (i);
    ProcReturn = "host" "proc_return" (i) -> i;
    DynOp = "host" "dyn_op" (i f i f i) -> f;
    Wait = "host" "wait" (i i) -> i;
    NextDone = "host" "next_done" (i) -> i;
    StrF = "host" "str_f" (f) -> i;
    ParamI = "host" "param_i" () -> i;
    ParamF = "host" "param_f" () -> f;
    ParamS = "host" "param_s" () -> i;
    StrLen = "rt" "str_len" (i) -> i;
    StrAsc = "rt" "str_asc" (i) -> i;
    Chr = "rt" "chr" (i) -> i;
    LeftRight = "rt" "left_right" (i i i) -> i;
    Mid = "rt" "mid" (i i i i) -> i;
    StrI = "rt" "str_i" (i) -> i;
    Instr = "rt" "instr" (i i i i) -> i;
    ChangeCase = "rt" "change_case" (i i) -> i;
    IntF = "rt" "int_f" (f) -> f;
    StrConst = "rt" "str_const" (i) -> i;
    StrConcat = "rt" "str_concat" (i i) -> i;
    StrMinus = "rt" "str_minus" (i i) -> i;
    StrCmp = "rt" "str_cmp" (i i) -> i;
    I2f = "rt" "i2f" (i) -> f;
    Fadd = "rt" "fadd" (f f) -> f;
    Fsub = "rt" "fsub" (f f) -> f;
    Fmul = "rt" "fmul" (f f) -> f;
    Fdiv = "rt" "fdiv" (f f) -> f;
    Fcmp = "rt" "fcmp" (f f) -> i;
    Pow = "rt" "pow" (f f) -> f;
}

/// Sentinels returned by the integer helpers.
const OVF: i64 = 1 << 32;
const DIV0: i64 = 1 << 33;

/// Branch targets of the structured control flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lbl {
    Dispatch,
    Status,
    Raise,
    Point(u32),
    Anon(u32),
}

// Fixed locals of `run` (local 0 is the `budget` parameter).
const L_BUDGET: u32 = 0;
const L_POINT: u32 = 1;
const L_ST: u32 = 2;
const L_EXCPOS: u32 = 3;
const L_EXCODE: u32 = 4;
const L_BASE: u32 = 5;
const N_FIXED: u32 = 6;

struct Gen<'a> {
    prg: &'a Compiled,
    instrs: &'a [Instr],
    double: bool,
    code: Vec<W<'static>>,
    labels: Vec<Lbl>,
    anon: u32,
    locals: Vec<ValType>,
    free: Vec<(u32, ValType)>,
    fn_imul: u32,
    fn_idiv: u32,
    /// Position of the instruction being compiled.
    pos: usize,
}

fn mem32(offset: u32) -> MemArg {
    MemArg { offset: offset as u64, align: 2, memory_index: 0 }
}

fn mem64(offset: u32) -> MemArg {
    MemArg { offset: offset as u64, align: 3, memory_index: 0 }
}

impl<'a> Gen<'a> {
    fn w(&mut self, i: W<'static>) {
        self.code.push(i);
    }

    fn i32c(&mut self, v: i32) {
        self.w(W::I32Const(v));
    }

    fn call(&mut self, f: Imp) {
        self.w(W::Call(f as u32));
    }

    fn get(&mut self, l: u32) {
        self.w(W::LocalGet(l));
    }

    fn set(&mut self, l: u32) {
        self.w(W::LocalSet(l));
    }

    fn tmp(&mut self, t: ValType) -> u32 {
        if let Some(i) = self.free.iter().position(|&(_, ft)| ft == t) {
            return self.free.swap_remove(i).0;
        }
        self.locals.push(t);
        // Local 0 is the parameter.
        self.locals.len() as u32
    }

    fn release(&mut self, l: u32, t: ValType) {
        self.free.push((l, t));
    }

    fn depth(&self, l: Lbl) -> u32 {
        let i = self.labels.iter().rposition(|&x| x == l).expect("branch target");
        (self.labels.len() - 1 - i) as u32
    }

    fn br(&mut self, l: Lbl) {
        let d = self.depth(l);
        self.w(W::Br(d));
    }

    fn br_if(&mut self, l: Lbl) {
        let d = self.depth(l);
        self.w(W::BrIf(d));
    }

    fn new_anon(&mut self) -> Lbl {
        self.anon += 1;
        Lbl::Anon(self.anon)
    }

    fn block(&mut self) -> Lbl {
        let l = self.new_anon();
        self.w(W::Block(BlockType::Empty));
        self.labels.push(l);
        l
    }

    fn if_(&mut self, bt: BlockType) {
        let l = self.new_anon();
        self.w(W::If(bt));
        self.labels.push(l);
    }

    fn else_(&mut self) {
        self.w(W::Else);
    }

    fn end(&mut self) {
        self.w(W::End);
        self.labels.pop();
    }

    /// Loads a header word of the memory.
    fn hdr(&mut self, off: u32) {
        self.get(L_BASE);
        self.w(W::I32Load(mem32(off)));
    }

    /// A status in `st`: continue unless it is `ST_CONTINUE`.
    fn status_check(&mut self) {
        self.w(W::LocalTee(L_ST));
        self.i32c(-1);
        self.w(W::I32Ne);
        self.br_if(Lbl::Status);
    }

    fn status_jump(&mut self) {
        self.set(L_ST);
        self.br(Lbl::Status);
    }

    fn jump_point(&mut self, point: u32) {
        self.i32c(point as i32);
        self.status_jump();
    }

    /// Branches to `$raise` if a runtime function left a pending error.
    fn err_check(&mut self) {
        self.hdr(layout::ERR);
        self.br_if(Lbl::Raise);
    }

    fn raise(&mut self, code: u16) {
        self.i32c(code as i32);
        self.set(L_EXCODE);
        self.br(Lbl::Raise);
    }

    /// Test point: calls the runtime when the attention word is set.
    fn test_point(&mut self) {
        self.hdr(layout::ATT);
        self.if_(BlockType::Empty);
        self.i32c(self.pos as i32);
        self.call(Imp::TestPoint);
        self.status_check();
        self.end();
    }

    fn point_of(&self, pos: usize) -> Result<u32, CompileError> {
        match structure::normalize(&self.prg.code, pos) {
            None => Ok(self.instrs.len() as u32),
            Some(p) => self
                .instrs
                .binary_search_by_key(&p, |i| i.pos)
                .map(|i| i as u32)
                .map_err(|_| CompileError::Internal(format!("no instruction at position {p}"))),
        }
    }

    // ------------------------------------------------------------------
    // Variables
    // ------------------------------------------------------------------

    /// Pushes the base address of `slot` and returns the offset to use.
    fn var_addr(&mut self, slot: u16) -> u32 {
        if slot & GLOBAL != 0 {
            self.get(L_BASE);
            layout::GLOBALS + (slot & !GLOBAL) as u32 * 8
        } else {
            // FP holds an absolute address.
            self.hdr(layout::FP);
            slot as u32 * 8
        }
    }

    fn load_var(&mut self, slot: u16, ty: Ty) {
        let off = self.var_addr(slot);
        match ty {
            Ty::Float => self.w(W::F64Load(mem64(off))),
            _ => self.w(W::I32Load(mem32(off))),
        }
    }

    fn store(&mut self, off: u32, ty: u8) {
        match ty {
            1 => self.w(W::F64Store(mem64(off))),
            _ => self.w(W::I32Store(mem32(off))),
        }
    }

    /// Evaluates the indices of an element of array `slot`; returns the
    /// local holding its flat index.
    fn aref(&mut self, slot: u16, idx: &[Expr]) -> u32 {
        // Indices are evaluated in order (an index can itself use an array:
        // keep them in locals until all are known); the first 8 go to the
        // IDX area, the interpreter evaluates and ignores the others.
        let mut temps = Vec::new();
        for (i, e) in idx.iter().enumerate() {
            self.expr(e);
            self.as_int(e.ty);
            if i < 8 {
                let t = self.tmp(ValType::I32);
                self.set(t);
                temps.push(t);
            } else {
                self.w(W::Drop);
            }
        }
        for (i, t) in temps.into_iter().enumerate() {
            self.get(L_BASE);
            self.get(t);
            self.w(W::I32Store(mem32(layout::IDX + i as u32 * 4)));
            self.release(t, ValType::I32);
        }
        self.i32c(slot as i32);
        self.i32c(idx.len() as i32);
        self.call(Imp::Aref);
        let t = self.tmp(ValType::I32);
        self.set(t);
        self.err_check();
        t
    }

    // ------------------------------------------------------------------
    // Conversions
    // ------------------------------------------------------------------

    fn as_int(&mut self, ty: Ty) {
        match ty {
            Ty::Int | Ty::Str => {}
            Ty::Float => self.w(W::I32TruncSatF64S),
            Ty::Dyn => {
                self.w(W::Drop);
                self.w(W::I32TruncSatF64S);
            }
        }
    }

    fn int_to_float(&mut self) {
        if self.double {
            self.w(W::F64ConvertI32S);
        } else {
            self.call(Imp::I2f);
        }
    }

    fn as_float(&mut self, ty: Ty) {
        match ty {
            Ty::Int => self.int_to_float(),
            Ty::Float | Ty::Str => {}
            Ty::Dyn => {
                let tag = self.tmp(ValType::I32);
                let x = self.tmp(ValType::F64);
                self.set(tag);
                self.set(x);
                self.get(tag);
                self.if_(BlockType::Result(ValType::F64));
                self.get(x);
                self.else_();
                self.get(x);
                self.w(W::I32TruncSatF64S);
                self.int_to_float();
                self.end();
                self.release(tag, ValType::I32);
                self.release(x, ValType::F64);
            }
        }
    }

    fn as_dyn(&mut self, ty: Ty) {
        match ty {
            Ty::Int => {
                self.w(W::F64ConvertI32S);
                self.i32c(0);
            }
            Ty::Float => self.i32c(1),
            Ty::Dyn | Ty::Str => {}
        }
    }

    /// Converts a value of type `ty` for storage in a variable of type `vt`.
    fn convert_for(&mut self, ty: Ty, vt: u8) {
        match vt {
            0 => self.as_int(ty),
            1 => self.as_float(ty),
            _ => {}
        }
    }

    /// Pushes a value of type `ty` on the runtime parameter stack.
    fn push_value(&mut self, ty: Ty) {
        self.call(match ty {
            Ty::Int => Imp::PushI,
            Ty::Float => Imp::PushF,
            Ty::Str => Imp::PushS,
            Ty::Dyn => Imp::PushN,
        });
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Int(v) => self.i32c(*v),
            ExprKind::Float(x) => self.w(W::F64Const((*x).into())),
            ExprKind::Str(p) => {
                self.i32c(*p as i32);
                self.call(Imp::StrConst);
            }
            ExprKind::Var(slot) => self.load_var(*slot, e.ty),
            ExprKind::Elem(slot, idx) => {
                let t = self.aref(*slot, idx);
                self.i32c(*slot as i32);
                self.get(t);
                self.release(t, ValType::I32);
                match e.ty {
                    Ty::Float => self.call(Imp::AgetF),
                    Ty::Str => self.call(Imp::AgetS),
                    _ => self.call(Imp::AgetI),
                }
            }
            ExprKind::Neg(a) => match a.ty {
                Ty::Int => {
                    self.i32c(0);
                    self.expr(a);
                    self.w(W::I32Sub);
                }
                Ty::Float => {
                    self.expr(a);
                    self.w(W::F64Neg);
                }
                _ => {
                    self.expr(a);
                    let tag = self.tmp(ValType::I32);
                    let x = self.tmp(ValType::F64);
                    self.set(tag);
                    self.set(x);
                    self.get(tag);
                    self.if_(BlockType::Result(ValType::F64));
                    self.get(x);
                    self.w(W::F64Neg);
                    self.else_();
                    self.i32c(0);
                    self.get(x);
                    self.w(W::I32TruncSatF64S);
                    self.w(W::I32Sub);
                    self.w(W::F64ConvertI32S);
                    self.end();
                    self.get(tag);
                    self.release(tag, ValType::I32);
                    self.release(x, ValType::F64);
                }
            },
            ExprKind::Not(a) => {
                self.expr(a);
                self.as_int(a.ty);
                self.i32c(-1);
                self.w(W::I32Xor);
            }
            ExprKind::Bin(op, a, b) => self.binop(*op, a, b, e.ty),
            ExprKind::Native(nf, args) => self.native(*nf, args, e.ty),
            ExprKind::Call(fpos, args) => {
                for a in args {
                    self.expr(a);
                    self.push_value(a.ty);
                }
                self.i32c(self.pos as i32);
                self.i32c(*fpos as i32);
                match e.ty {
                    Ty::Int => self.call(Imp::FnI),
                    Ty::Float => self.call(Imp::FnF),
                    Ty::Str => self.call(Imp::FnS),
                    Ty::Dyn => self.call(Imp::FnN),
                }
                self.err_check();
                if e.ty == Ty::Dyn {
                    self.hdr(layout::TAG);
                }
            }
        }
    }

    fn int_arg(&mut self, e: &Expr) {
        self.expr(e);
        self.as_int(e.ty);
    }

    fn native(&mut self, nf: Nf, a: &[Expr], ty: Ty) {
        match nf {
            Nf::Len | Nf::Asc => {
                self.expr(&a[0]);
                self.call(if nf == Nf::Len { Imp::StrLen } else { Imp::StrAsc });
            }
            Nf::Chr => {
                self.int_arg(&a[0]);
                self.call(Imp::Chr);
                self.err_check();
            }
            Nf::Left | Nf::Right => {
                self.expr(&a[0]);
                self.int_arg(&a[1]);
                self.i32c((nf == Nf::Right) as i32);
                self.call(Imp::LeftRight);
                self.err_check();
            }
            Nf::Mid2 | Nf::Mid3 => {
                self.expr(&a[0]);
                self.int_arg(&a[1]);
                if nf == Nf::Mid3 {
                    self.int_arg(&a[2]);
                    self.i32c(1);
                } else {
                    self.i32c(0);
                    self.i32c(0);
                }
                self.call(Imp::Mid);
                self.err_check();
            }
            Nf::Str => {
                self.expr(&a[0]);
                self.call(if a[0].ty == Ty::Int { Imp::StrI } else { Imp::StrF });
            }
            Nf::Instr2 | Nf::Instr3 => {
                self.expr(&a[0]);
                self.expr(&a[1]);
                if nf == Nf::Instr3 {
                    self.int_arg(&a[2]);
                    self.i32c(1);
                } else {
                    self.i32c(0);
                    self.i32c(0);
                }
                self.call(Imp::Instr);
                self.err_check();
            }
            Nf::Upper | Nf::Lower => {
                self.expr(&a[0]);
                self.i32c((nf == Nf::Lower) as i32);
                self.call(Imp::ChangeCase);
            }
            Nf::Abs => {
                self.expr(&a[0]);
                if a[0].ty == Ty::Int {
                    // wrapping_abs
                    let x = self.tmp(ValType::I32);
                    self.w(W::LocalTee(x));
                    self.get(x);
                    self.i32c(31);
                    self.w(W::I32ShrS);
                    self.w(W::I32Xor);
                    self.get(x);
                    self.i32c(31);
                    self.w(W::I32ShrS);
                    self.w(W::I32Sub);
                    self.release(x, ValType::I32);
                } else {
                    self.w(W::F64Abs);
                }
            }
            Nf::Int => {
                self.expr(&a[0]);
                if a[0].ty == Ty::Float {
                    if self.double {
                        self.w(W::F64Floor);
                    } else {
                        self.call(Imp::IntF);
                    }
                }
            }
            Nf::Sgn => {
                self.expr(&a[0]);
                if a[0].ty == Ty::Int {
                    let x = self.tmp(ValType::I32);
                    self.w(W::LocalTee(x));
                    self.i32c(0);
                    self.w(W::I32GtS);
                    self.get(x);
                    self.i32c(0);
                    self.w(W::I32LtS);
                    self.w(W::I32Sub);
                    self.release(x, ValType::I32);
                } else {
                    let x = self.tmp(ValType::F64);
                    self.w(W::LocalTee(x));
                    self.w(W::F64Const(0f64.into()));
                    self.w(W::F64Gt);
                    self.get(x);
                    self.w(W::F64Const(0f64.into()));
                    self.w(W::F64Lt);
                    self.w(W::I32Sub);
                    self.release(x, ValType::F64);
                }
            }
            Nf::Min | Nf::Max => {
                // gt = a > b; Max: gt ? a : b, Min: gt ? b : a.
                let vt = if ty == Ty::Float { ValType::F64 } else { ValType::I32 };
                let (x, y) = (self.tmp(vt), self.tmp(vt));
                self.expr(&a[0]);
                if ty == Ty::Float {
                    self.as_float(a[0].ty);
                }
                self.set(x);
                self.expr(&a[1]);
                if ty == Ty::Float {
                    self.as_float(a[1].ty);
                }
                self.set(y);
                if nf == Nf::Max {
                    self.get(x);
                    self.get(y);
                } else {
                    self.get(y);
                    self.get(x);
                }
                self.get(x);
                self.get(y);
                match ty {
                    Ty::Float => self.w(W::F64Gt),
                    Ty::Str => {
                        self.call(Imp::StrCmp);
                        self.i32c(0);
                        self.w(W::I32GtS);
                    }
                    _ => self.w(W::I32GtS),
                }
                self.w(W::Select);
                self.release(x, vt);
                self.release(y, vt);
            }
            Nf::True => self.i32c(-1),
            Nf::False => self.i32c(0),
            Nf::Param => self.call(Imp::ParamI),
            Nf::ParamF => self.call(Imp::ParamF),
            Nf::ParamS => self.call(Imp::ParamS),
            Nf::Pi => {
                let pi = if self.double {
                    std::f64::consts::PI
                } else {
                    amos_core::ffp::Ffp::from_f64(std::f64::consts::PI).to_f64()
                };
                self.w(W::F64Const(pi.into()));
            }
        }
    }

    fn cmp_i32(&mut self, op: u16) {
        use tk::*;
        self.w(match op {
            OP_EQ => W::I32Eq,
            OP_NE | OP_NE2 => W::I32Ne,
            OP_LT => W::I32LtS,
            OP_GT => W::I32GtS,
            OP_LE | OP_LE2 => W::I32LeS,
            _ => W::I32GeS,
        });
    }

    /// 0/1 to AMOS 0/-1.
    fn bool_to_amos(&mut self) {
        self.i32c(-1);
        self.w(W::I32Mul);
    }

    fn binop(&mut self, op: u16, a: &Expr, b: &Expr, ty: Ty) {
        use tk::*;
        match op {
            OP_AND | OP_OR | OP_XOR => {
                self.expr(a);
                self.as_int(a.ty);
                self.expr(b);
                self.as_int(b.ty);
                self.w(match op {
                    OP_AND => W::I32And,
                    OP_OR => W::I32Or,
                    _ => W::I32Xor,
                });
                return;
            }
            OP_MOD => {
                self.expr(a);
                self.as_int(a.ty);
                self.expr(b);
                self.as_int(b.ty);
                let (x, y, s) = (self.tmp(ValType::I32), self.tmp(ValType::I32), self.tmp(ValType::I32));
                self.set(y);
                self.set(x);
                self.get(y);
                self.w(W::I32Eqz);
                self.if_(BlockType::Result(ValType::I32));
                self.get(x);
                self.else_();
                self.get(x);
                // |y| as unsigned
                self.get(y);
                self.i32c(31);
                self.w(W::I32ShrS);
                self.set(s);
                self.get(y);
                self.get(s);
                self.w(W::I32Xor);
                self.get(s);
                self.w(W::I32Sub);
                self.w(W::I32RemU);
                self.end();
                for t in [x, y, s] {
                    self.release(t, ValType::I32);
                }
                return;
            }
            OP_POW => {
                self.expr(a);
                self.as_float(a.ty);
                self.expr(b);
                self.as_float(b.ty);
                self.call(Imp::Pow);
                return;
            }
            _ => {}
        }
        let ct = if a.ty == Ty::Dyn || b.ty == Ty::Dyn {
            Ty::Dyn
        } else if a.ty == Ty::Str {
            Ty::Str
        } else if a.ty == Ty::Int && b.ty == Ty::Int {
            Ty::Int
        } else {
            Ty::Float
        };
        if ct == Ty::Dyn {
            self.i32c(op as i32);
            self.expr(a);
            self.as_dyn(a.ty);
            self.expr(b);
            self.as_dyn(b.ty);
            self.call(Imp::DynOp);
            self.err_check();
            self.hdr(layout::TAG);
            if ty == Ty::Int {
                self.as_int(Ty::Dyn);
            }
            return;
        }
        if is_comparison(op) {
            match ct {
                Ty::Str => {
                    self.expr(a);
                    self.expr(b);
                    self.call(Imp::StrCmp);
                    self.i32c(0);
                    self.cmp_i32(op);
                }
                Ty::Int => {
                    self.expr(a);
                    self.expr(b);
                    self.cmp_i32(op);
                }
                _ => {
                    self.expr(a);
                    self.as_float(a.ty);
                    self.expr(b);
                    self.as_float(b.ty);
                    if self.double {
                        self.float_cmp(op);
                    } else {
                        self.call(Imp::Fcmp);
                        self.i32c(0);
                        self.cmp_i32(op);
                    }
                }
            }
            self.bool_to_amos();
            return;
        }
        match ct {
            Ty::Str => {
                self.expr(a);
                self.expr(b);
                if op == OP_PLUS {
                    self.call(Imp::StrConcat);
                    self.err_check();
                } else {
                    self.call(Imp::StrMinus);
                }
            }
            Ty::Int => {
                match op {
                    OP_PLUS | OP_MINUS => {
                        self.expr(a);
                        self.w(W::I64ExtendI32S);
                        self.expr(b);
                        self.w(W::I64ExtendI32S);
                        self.w(if op == OP_PLUS { W::I64Add } else { W::I64Sub });
                    }
                    _ => {
                        self.expr(a);
                        self.expr(b);
                        self.w(W::Call(if op == OP_MUL { self.fn_imul } else { self.fn_idiv }));
                    }
                }
                let r = self.tmp(ValType::I64);
                self.w(W::LocalTee(r));
                if op == OP_DIV {
                    self.w(W::I64Const(DIV0));
                    self.w(W::I64Eq);
                    self.if_(BlockType::Empty);
                    self.raise(errors::DIVISION_BY_ZERO);
                    self.end();
                } else {
                    self.get(r);
                    self.w(W::I32WrapI64);
                    self.w(W::I64ExtendI32S);
                    self.w(W::I64Ne);
                    self.if_(BlockType::Empty);
                    self.raise(errors::OVERFLOW);
                    self.end();
                }
                self.get(r);
                self.w(W::I32WrapI64);
                self.release(r, ValType::I64);
            }
            _ => {
                self.expr(a);
                self.as_float(a.ty);
                self.expr(b);
                self.as_float(b.ty);
                if op == OP_DIV {
                    let y = self.tmp(ValType::F64);
                    self.w(W::LocalTee(y));
                    self.w(W::F64Const(0f64.into()));
                    self.w(W::F64Eq);
                    self.if_(BlockType::Empty);
                    self.raise(errors::DIVISION_BY_ZERO);
                    self.end();
                    self.get(y);
                    self.release(y, ValType::F64);
                }
                if self.double {
                    self.w(match op {
                        OP_PLUS => W::F64Add,
                        OP_MINUS => W::F64Sub,
                        OP_MUL => W::F64Mul,
                        _ => W::F64Div,
                    });
                } else {
                    self.call(match op {
                        OP_PLUS => Imp::Fadd,
                        OP_MINUS => Imp::Fsub,
                        OP_MUL => Imp::Fmul,
                        _ => Imp::Fdiv,
                    });
                }
            }
        }
    }

    /// Double precision comparison with the interpreter's NaN rule
    /// (`partial_cmp(..).unwrap_or(Equal)`).
    fn float_cmp(&mut self, op: u16) {
        use tk::*;
        let (x, y) = (self.tmp(ValType::F64), self.tmp(ValType::F64));
        self.set(y);
        self.set(x);
        let lt = |g: &mut Self| {
            g.get(x);
            g.get(y);
            g.w(W::F64Lt);
        };
        let gt = |g: &mut Self| {
            g.get(x);
            g.get(y);
            g.w(W::F64Gt);
        };
        match op {
            OP_LT => lt(self),
            OP_GT => gt(self),
            OP_LE | OP_LE2 => {
                gt(self);
                self.w(W::I32Eqz);
            }
            OP_GE | OP_GE2 => {
                lt(self);
                self.w(W::I32Eqz);
            }
            OP_EQ => {
                lt(self);
                gt(self);
                self.w(W::I32Or);
                self.w(W::I32Eqz);
            }
            _ => {
                lt(self);
                gt(self);
                self.w(W::I32Or);
            }
        }
        self.release(x, ValType::F64);
        self.release(y, ValType::F64);
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn assign(&mut self, lv: &LValue, e: &Expr) {
        match lv {
            LValue::Scalar { slot, ty } => {
                let off = self.var_addr(*slot);
                self.expr(e);
                self.convert_for(e.ty, *ty);
                self.store(off, *ty);
            }
            LValue::Elem { slot, ty, idx } => {
                let t = self.aref(*slot, idx);
                self.i32c(*slot as i32);
                self.get(t);
                self.release(t, ValType::I32);
                self.expr(e);
                self.convert_for(e.ty, *ty);
                self.call(match ty {
                    0 => Imp::AsetI,
                    1 => Imp::AsetF,
                    _ => Imp::AsetS,
                });
                self.err_check();
            }
        }
    }

    /// Pushes 1 if `target` is in the range of the top loop (no loop to
    /// drop by `after_jump`).
    fn in_loop_range(&mut self, target: usize) {
        self.i32c(target as i32);
        self.hdr(layout::LOOP_LO);
        self.w(W::I32GeS);
        self.i32c(target as i32);
        self.hdr(layout::LOOP_HI);
        self.w(W::I32LeS);
        self.w(W::I32And);
    }

    /// Goto label `idx` with its test point (`Interp::label_target`): a
    /// direct jump when nothing is pending, the scope is the static one and
    /// no loop is dropped; the runtime otherwise. Always leaves.
    fn goto_label(&mut self, k: usize, idx: u16) -> Result<(), CompileError> {
        let scope = self.instrs[k].scope;
        if let Some(l) = self.prg.scopes.get(scope).and_then(|s| s.labels.get(idx as usize)) {
            let target = l.target;
            let pt = self.point_of(target)?;
            self.hdr(layout::ATT);
            self.w(W::I32Eqz);
            self.hdr(layout::SCOPE);
            self.i32c(scope as i32);
            self.w(W::I32Eq);
            self.w(W::I32And);
            self.in_loop_range(target);
            self.w(W::I32And);
            self.if_(BlockType::Empty);
            self.jump_point(pt);
            self.end();
        }
        self.i32c(self.pos as i32);
        self.i32c(idx as i32);
        self.call(Imp::GotoLabel);
        self.status_jump();
        Ok(())
    }

    fn stmt(&mut self, k: usize, s: &Stmt) -> Result<(), CompileError> {
        let pos = self.pos as i32;
        let next = k as u32 + 1;
        match s {
            Stmt::Nop => {}
            Stmt::Jump(target) => {
                let p = self.point_of(*target)?;
                if p != next {
                    self.jump_point(p);
                }
            }
            Stmt::Assign(lv, e) => self.assign(lv, e),
            Stmt::Print { items, newline } => {
                self.call(Imp::PrintBegin);
                for it in items {
                    match it {
                        PrintItem::Tab => self.call(Imp::PrintTab),
                        PrintItem::Value(e) => {
                            self.expr(e);
                            self.call(match e.ty {
                                Ty::Int => Imp::PrintI,
                                Ty::Float => Imp::PrintF,
                                Ty::Str => Imp::PrintS,
                                Ty::Dyn => Imp::PrintN,
                            });
                        }
                    }
                }
                self.i32c(pos);
                self.i32c(*newline as i32);
                self.call(Imp::PrintEnd);
                self.status_check();
            }
            Stmt::If { branches, false_target, false_label } => {
                let done = self.block();
                for (cond, action) in branches {
                    self.expr(cond);
                    self.as_int(cond.ty);
                    self.if_(BlockType::Empty);
                    match action {
                        IfTrue::Label(idx) => self.goto_label(k, *idx)?,
                        IfTrue::At(p) => {
                            let pt = self.point_of(*p)?;
                            if pt == next {
                                self.br(done);
                            } else {
                                self.jump_point(pt);
                            }
                        }
                    }
                    self.end();
                }
                match false_label {
                    Some(idx) => self.goto_label(k, *idx)?,
                    None => {
                        // `after_jump` drops nothing when the target is in
                        // the range of the top loop.
                        let pt = self.point_of(*false_target)?;
                        self.in_loop_range(*false_target);
                        self.if_(BlockType::Empty);
                        if pt == next {
                            self.br(done);
                        } else {
                            self.jump_point(pt);
                        }
                        self.end();
                        self.i32c(pos);
                        self.i32c(*false_target as i32);
                        self.call(Imp::GotoPos);
                        self.status_jump();
                    }
                }
                self.end();
            }
            Stmt::For { lv, start, limit, step, body, exit } => {
                // (For on an array element is left to the interpreter.)
                let LValue::Scalar { .. } = lv else {
                    return Err(CompileError::Internal("For on an array element".into()));
                };
                self.assign(lv, start);
                let flat: Option<u32> = None;
                let lim = self.tmp(ValType::I32);
                self.expr(limit);
                self.as_int(limit.ty);
                self.set(lim);
                let stp = self.tmp(ValType::I32);
                match step {
                    Some(e) => {
                        self.expr(e);
                        self.as_int(e.ty);
                    }
                    None => self.i32c(1),
                }
                self.set(stp);
                self.i32c(pos);
                self.i32c(lv.slot() as i32);
                self.i32c(flat.map_or(-1, |f| f as i32));
                self.get(lim);
                self.get(stp);
                self.i32c(*body as i32);
                self.i32c(*exit as i32);
                self.call(Imp::ForPush);
                self.status_check();
                self.release(lim, ValType::I32);
                self.release(stp, ValType::I32);
            }
            Stmt::Next => {
                // Fast path: nothing to do at the test point and the top of
                // the control stack is a For on an integer variable.
                self.hdr(layout::ATT);
                self.w(W::I32Eqz);
                self.hdr(layout::FOR_ADDR);
                self.i32c(0);
                self.w(W::I32Ne);
                self.w(W::I32And);
                self.if_(BlockType::Result(ValType::I32));
                let (a, v) = (self.tmp(ValType::I32), self.tmp(ValType::I32));
                self.hdr(layout::FOR_ADDR);
                self.w(W::LocalTee(a));
                self.get(a);
                self.w(W::I32Load(mem32(0)));
                self.hdr(layout::FOR_STEP);
                self.w(W::I32Add);
                self.w(W::LocalTee(v));
                self.w(W::I32Store(mem32(0)));
                self.hdr(layout::FOR_STEP);
                self.i32c(0);
                self.w(W::I32GeS);
                self.if_(BlockType::Result(ValType::I32));
                self.get(v);
                self.hdr(layout::FOR_LIMIT);
                self.w(W::I32GtS);
                self.else_();
                self.get(v);
                self.hdr(layout::FOR_LIMIT);
                self.w(W::I32LtS);
                self.end();
                self.w(W::I32Eqz);
                self.if_(BlockType::Empty);
                self.hdr(layout::FOR_BODY);
                self.status_jump();
                self.end();
                self.release(a, ValType::I32);
                self.release(v, ValType::I32);
                self.i32c(pos);
                self.call(Imp::NextDone);
                self.else_();
                self.i32c(pos);
                self.call(Imp::Next);
                self.end();
                self.status_check();
            }
            Stmt::LoopStart { do_loop, body, exit } => {
                self.i32c(pos);
                self.i32c(*do_loop as i32);
                self.i32c(*body as i32);
                self.i32c(*exit as i32);
                self.call(Imp::LoopPush);
                self.status_check();
            }
            Stmt::While { cond, body, exit } => {
                let exit_point = self.point_of(*exit)?;
                self.expr(cond);
                self.as_int(cond.ty);
                self.w(W::I32Eqz);
                self.if_(BlockType::Empty);
                self.jump_point(exit_point);
                self.end();
                self.i32c(pos);
                self.i32c(*body as i32);
                self.i32c(*exit as i32);
                self.call(Imp::WhilePush);
                self.status_check();
            }
            Stmt::Until(cond) => {
                self.test_point();
                self.i32c(pos);
                self.expr(cond);
                self.as_int(cond.ty);
                self.call(Imp::Until);
                self.status_check();
            }
            Stmt::Wend => {
                self.i32c(pos);
                self.call(Imp::Wend);
                self.status_jump();
            }
            Stmt::Loop => {
                self.i32c(pos);
                self.call(Imp::LoopEnd);
                self.status_jump();
            }
            Stmt::Exit { cond, frames, target } => {
                if let Some(c) = cond {
                    self.expr(c);
                    self.as_int(c.ty);
                    self.if_(BlockType::Empty);
                }
                self.i32c(pos);
                self.i32c(*frames as i32);
                self.i32c(*target as i32);
                self.call(Imp::Exit);
                self.status_jump();
                if cond.is_some() {
                    self.end();
                }
            }
            Stmt::Goto(idx) => self.goto_label(k, *idx)?,
            Stmt::Gosub { label, ret } => {
                self.i32c(pos);
                self.i32c(*label as i32);
                self.i32c(*ret as i32);
                self.call(Imp::GosubLabel);
                self.status_jump();
            }
            Stmt::Return => {
                self.i32c(pos);
                self.call(Imp::Return);
                self.status_jump();
            }
            Stmt::Call { proc, args, ret } => {
                self.test_point();
                for a in args {
                    self.expr(a);
                    self.push_value(a.ty);
                }
                self.i32c(pos);
                self.i32c(*proc as i32);
                self.i32c(*ret as i32);
                self.i32c(args.len() as i32);
                self.call(Imp::Call);
                self.status_jump();
            }
            Stmt::EndProc { pop, value } => {
                self.test_point();
                self.i32c(pos);
                self.i32c(*pop as i32);
                self.call(Imp::ProcCheck);
                self.status_check();
                if let Some(e) = value {
                    self.expr(e);
                    match e.ty {
                        Ty::Int => self.call(Imp::SetParamI),
                        Ty::Float => self.call(Imp::SetParamF),
                        Ty::Str => self.call(Imp::SetParamS),
                        Ty::Dyn => {
                            let tag = self.tmp(ValType::I32);
                            let x = self.tmp(ValType::F64);
                            self.set(tag);
                            self.set(x);
                            self.get(tag);
                            self.if_(BlockType::Empty);
                            self.get(x);
                            self.call(Imp::SetParamF);
                            self.else_();
                            self.get(x);
                            self.w(W::I32TruncSatF64S);
                            self.call(Imp::SetParamI);
                            self.end();
                            self.release(tag, ValType::I32);
                            self.release(x, ValType::F64);
                        }
                    }
                }
                self.i32c(pos);
                self.call(Imp::ProcReturn);
                self.status_jump();
            }
            Stmt::IncDec { slot, ty, inc } => {
                let off = self.var_addr(*slot);
                let off2 = self.var_addr(*slot);
                debug_assert_eq!(off, off2);
                if *ty == 1 {
                    self.w(W::F64Load(mem64(off)));
                    self.w(W::F64Const(1f64.into()));
                    self.w(if *inc { W::F64Add } else { W::F64Sub });
                    self.w(W::F64Store(mem64(off)));
                } else {
                    self.w(W::I32Load(mem32(off)));
                    self.i32c(1);
                    self.w(if *inc { W::I32Add } else { W::I32Sub });
                    self.w(W::I32Store(mem32(off)));
                }
            }
            Stmt::Add { slot, ty, n, range } => {
                let off = self.var_addr(*slot);
                self.expr(n);
                self.as_int(n.ty);
                let nt = self.tmp(ValType::I32);
                self.set(nt);
                let off2 = self.var_addr(*slot);
                debug_assert_eq!(off, off2);
                if *ty == 1 {
                    self.w(W::F64Load(mem64(off)));
                    self.w(W::I32TruncSatF64S);
                } else {
                    self.w(W::I32Load(mem32(off)));
                }
                self.get(nt);
                self.w(W::I32Add);
                if let Some((a, b)) = range {
                    let v = nt;
                    self.set(v);
                    let (ta, tb) = (self.tmp(ValType::I32), self.tmp(ValType::I32));
                    self.expr(a);
                    self.as_int(a.ty);
                    self.set(ta);
                    self.expr(b);
                    self.as_int(b.ty);
                    self.set(tb);
                    // v < a ? b : (v > b ? a : v)
                    self.get(tb);
                    self.get(ta);
                    self.get(v);
                    self.get(v);
                    self.get(tb);
                    self.w(W::I32GtS);
                    self.w(W::Select);
                    self.get(v);
                    self.get(ta);
                    self.w(W::I32LtS);
                    self.w(W::Select);
                    self.release(ta, ValType::I32);
                    self.release(tb, ValType::I32);
                }
                self.release(nt, ValType::I32);
                if *ty == 1 {
                    self.int_to_float();
                }
                self.store(off, *ty);
            }
            Stmt::Wait(n) => {
                self.i32c(pos);
                match n {
                    Some(e) => {
                        self.expr(e);
                        self.as_int(e.ty);
                    }
                    None => self.i32c(1),
                }
                self.call(Imp::Wait);
                self.status_check();
            }
            Stmt::Keyword(args) => {
                for a in args {
                    self.expr(a);
                    self.push_value(a.ty);
                }
                self.i32c(pos);
                self.call(Imp::Keyword);
                self.status_check();
            }
            Stmt::Interp => {
                self.i32c(pos);
                self.call(Imp::Interp);
                self.status_jump();
            }
        }
        Ok(())
    }

    /// Time budget: counted per instruction like `Interp::run`, so the
    /// program yields at the same places.
    fn budget(&mut self, point: u32) {
        self.get(L_BUDGET);
        self.i32c(1);
        self.w(W::I32Sub);
        self.w(W::LocalTee(L_BUDGET));
        self.i32c(0);
        self.w(W::I32LtS);
        self.if_(BlockType::Empty);
        self.i32c(point as i32);
        self.call(Imp::Suspend);
        self.i32c(amos_core::compiled::RUN_RUNNING);
        self.w(W::Return);
        self.end();
    }

    /// The `run` function.
    fn run_function(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        let n = self.instrs.len() as u32;
        self.w(W::GlobalGet(0));
        self.set(L_BASE);
        self.i32c(-1);
        self.set(L_EXCPOS);
        self.call(Imp::Enter);
        self.set(L_ST);
        self.w(W::Loop(BlockType::Empty));
        self.labels.push(Lbl::Dispatch);
        // Status handling.
        self.get(L_ST);
        self.i32c(0);
        self.w(W::I32LtS);
        self.if_(BlockType::Empty);
        self.get(L_ST);
        self.i32c(amos_core::compiled::ST_YIELD);
        self.w(W::I32Eq);
        self.if_(BlockType::Empty);
        self.i32c(amos_core::compiled::RUN_RUNNING);
        self.w(W::Return);
        self.end();
        self.i32c(amos_core::compiled::RUN_STOPPED);
        self.w(W::Return);
        self.end();
        self.get(L_ST);
        self.set(L_POINT);
        // Dispatch.
        self.w(W::Block(BlockType::Empty));
        self.labels.push(Lbl::Status);
        self.w(W::Block(BlockType::Empty));
        self.labels.push(Lbl::Raise);
        self.w(W::Block(BlockType::Empty));
        self.labels.push(Lbl::Point(n));
        for k in (0..n).rev() {
            self.w(W::Block(BlockType::Empty));
            self.labels.push(Lbl::Point(k));
        }
        self.get(L_POINT);
        let targets: Vec<u32> = (0..=n).collect();
        self.w(W::BrTable(Cow::Owned(targets), n));
        for (k, s) in stmts.iter().enumerate() {
            self.end(); // $p[k]
            self.pos = self.instrs[k].pos;
            self.budget(k as u32);
            self.i32c(self.pos as i32);
            self.set(L_EXCPOS);
            self.i32c(0);
            self.set(L_EXCODE);
            self.stmt(k, s)?;
        }
        self.end(); // $end
        // `Interp::run` checks the budget before finding the end.
        self.budget(n);
        // The last instruction started (`Interp::inst_pos` at the end).
        self.get(L_EXCPOS);
        self.call(Imp::EndProgram);
        self.status_jump();
        self.end(); // $raise
        self.get(L_EXCPOS);
        self.get(L_EXCODE);
        self.call(Imp::Raise);
        self.set(L_ST);
        self.end(); // $status
        self.br(Lbl::Dispatch);
        self.end(); // $dispatch
        self.w(W::Unreachable);
        self.w(W::End);
        Ok(())
    }
}

/// `a * b` with the interpreter's rules (`Interp::binop`, `OP_MUL`): the
/// result as i64, or `OVF`.
fn imul_function() -> Function {
    // params a=0 b=1; locals ax=2 ay=3 m64=4(i64)
    let mut f = Function::new([(2, ValType::I32), (1, ValType::I64)]);
    let abs = |f: &mut Function, l: u32| {
        f.instruction(&W::LocalGet(l));
        f.instruction(&W::LocalGet(l));
        f.instruction(&W::I32Const(31));
        f.instruction(&W::I32ShrS);
        f.instruction(&W::I32Xor);
        f.instruction(&W::LocalGet(l));
        f.instruction(&W::I32Const(31));
        f.instruction(&W::I32ShrS);
        f.instruction(&W::I32Sub);
    };
    abs(&mut f, 0);
    f.instruction(&W::LocalSet(2));
    abs(&mut f, 1);
    f.instruction(&W::LocalSet(3));
    // neg = (a ^ b) < 0
    let neg = |f: &mut Function| {
        f.instruction(&W::LocalGet(0));
        f.instruction(&W::LocalGet(1));
        f.instruction(&W::I32Xor);
        f.instruction(&W::I32Const(0));
        f.instruction(&W::I32LtS);
    };
    // Small operands: 32 bit product, wrapping.
    f.instruction(&W::LocalGet(2));
    f.instruction(&W::I32Const(65536));
    f.instruction(&W::I32LtU);
    f.instruction(&W::LocalGet(3));
    f.instruction(&W::I32Const(65536));
    f.instruction(&W::I32LtU);
    f.instruction(&W::I32And);
    f.instruction(&W::If(BlockType::Empty));
    f.instruction(&W::I32Const(0));
    f.instruction(&W::LocalGet(2));
    f.instruction(&W::LocalGet(3));
    f.instruction(&W::I32Mul);
    f.instruction(&W::I32Sub);
    f.instruction(&W::LocalGet(2));
    f.instruction(&W::LocalGet(3));
    f.instruction(&W::I32Mul);
    neg(&mut f);
    f.instruction(&W::Select);
    f.instruction(&W::I64ExtendI32S);
    f.instruction(&W::Return);
    f.instruction(&W::End);
    // 64 bit product, overflow at 2^31.
    f.instruction(&W::LocalGet(2));
    f.instruction(&W::I64ExtendI32U);
    f.instruction(&W::LocalGet(3));
    f.instruction(&W::I64ExtendI32U);
    f.instruction(&W::I64Mul);
    f.instruction(&W::LocalTee(4));
    f.instruction(&W::I64Const(1 << 31));
    f.instruction(&W::I64GeU);
    f.instruction(&W::If(BlockType::Empty));
    f.instruction(&W::I64Const(OVF));
    f.instruction(&W::Return);
    f.instruction(&W::End);
    f.instruction(&W::I64Const(0));
    f.instruction(&W::LocalGet(4));
    f.instruction(&W::I64Sub);
    f.instruction(&W::LocalGet(4));
    neg(&mut f);
    f.instruction(&W::Select);
    f.instruction(&W::End);
    f
}

/// `a / b` on integers (`OP_DIV`): the result as i64, or `DIV0`.
fn idiv_function() -> Function {
    let mut f = Function::new([(2, ValType::I32)]);
    f.instruction(&W::LocalGet(1));
    f.instruction(&W::I32Eqz);
    f.instruction(&W::If(BlockType::Empty));
    f.instruction(&W::I64Const(DIV0));
    f.instruction(&W::Return);
    f.instruction(&W::End);
    for (src, dst) in [(0, 2), (1, 3)] {
        f.instruction(&W::LocalGet(src));
        f.instruction(&W::LocalGet(src));
        f.instruction(&W::I32Const(31));
        f.instruction(&W::I32ShrS);
        f.instruction(&W::I32Xor);
        f.instruction(&W::LocalGet(src));
        f.instruction(&W::I32Const(31));
        f.instruction(&W::I32ShrS);
        f.instruction(&W::I32Sub);
        f.instruction(&W::LocalSet(dst));
    }
    // q = |a| / |b| (unsigned); neg ? -q : q
    f.instruction(&W::I32Const(0));
    f.instruction(&W::LocalGet(2));
    f.instruction(&W::LocalGet(3));
    f.instruction(&W::I32DivU);
    f.instruction(&W::I32Sub);
    f.instruction(&W::LocalGet(2));
    f.instruction(&W::LocalGet(3));
    f.instruction(&W::I32DivU);
    f.instruction(&W::LocalGet(0));
    f.instruction(&W::LocalGet(1));
    f.instruction(&W::I32Xor);
    f.instruction(&W::I32Const(0));
    f.instruction(&W::I32LtS);
    f.instruction(&W::Select);
    f.instruction(&W::I64ExtendI32S);
    f.instruction(&W::End);
    f
}

/// Builds the module.
pub fn module(prg: &Compiled, instrs: &[Instr], stmts: &[Stmt]) -> Result<Vec<u8>, CompileError> {
    let lay = Layout::new(prg);
    let mut types = TypeSection::new();
    let mut type_ids: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    let mut type_of = |types: &mut TypeSection, p: &[ValType], r: &[ValType]| -> u32 {
        if let Some(i) = type_ids.iter().position(|(a, b)| a == p && b == r) {
            return i as u32;
        }
        types.ty().function(p.iter().copied(), r.iter().copied());
        type_ids.push((p.to_vec(), r.to_vec()));
        (type_ids.len() - 1) as u32
    };
    let mut imports = ImportSection::new();
    imports.import(
        "env",
        "memory",
        MemoryType { minimum: lay.pages as u64, maximum: None, memory64: false, shared: false, page_size_log2: None },
    );
    imports.import("env", "base", GlobalType { val_type: ValType::I32, mutable: false, shared: false });
    for (m, n, p, r) in IMPORTS {
        let t = type_of(&mut types, p, r);
        imports.import(m, n, EntityType::Function(t));
    }
    let n_imports = IMPORTS.len() as u32;
    let run_type = type_of(&mut types, &[ValType::I32], &[ValType::I32]);
    let helper_type = type_of(&mut types, &[ValType::I32, ValType::I32], &[ValType::I64]);
    let mut funcs = FunctionSection::new();
    funcs.function(run_type);
    funcs.function(helper_type);
    funcs.function(helper_type);

    let mut g = Gen {
        prg,
        instrs,
        double: prg.double,
        code: Vec::new(),
        labels: Vec::new(),
        anon: 0,
        locals: vec![ValType::I32; (N_FIXED - 1) as usize],
        free: Vec::new(),
        fn_imul: n_imports + 1,
        fn_idiv: n_imports + 2,
        pos: 0,
    };
    // Locals 1..N_FIXED are the fixed ones; temporaries follow.
    g.run_function(stmts)?;
    let mut run = Function::new(g.locals.iter().map(|&t| (1, t)));
    for i in &g.code {
        run.instruction(i);
    }
    let mut code = CodeSection::new();
    code.function(&run);
    code.function(&imul_function());
    code.function(&idiv_function());

    let mut globals = GlobalSection::new();
    let gt = GlobalType { val_type: ValType::I32, mutable: false, shared: false };
    globals.global(gt, &ConstExpr::i32_const(amos_core::compiled::ABI_VERSION));
    globals.global(gt, &ConstExpr::i32_const(structure::code_hash(&prg.code) as i32));

    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, n_imports);
    // Global 0 is the imported base.
    exports.export("amos_abi", ExportKind::Global, 1);
    exports.export("amos_hash", ExportKind::Global, 2);

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&funcs);
    module.section(&globals);
    module.section(&exports);
    module.section(&code);
    Ok(module.finish())
}
