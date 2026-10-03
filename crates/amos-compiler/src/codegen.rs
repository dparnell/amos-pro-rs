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
use crate::ffp::{self as ffp_helpers, H};
use crate::ir::*;
use crate::lower::is_comparison;
use crate::numfmt::{self as num_helpers, N};
use crate::strings::{self as string_helpers, S};

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
    Keyword = "host" "keyword" (i i) -> i;
    FnI = "host" "fn_i" (i i i) -> i;
    FnF = "host" "fn_f" (i i i) -> f;
    FnN = "host" "fn_n" (i i i) -> f;
    FnS = "host" "fn_s" (i i i) -> i;
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
    CallProc = "host" "call_proc" (i i i i) -> i;
    GotoValue = "host" "goto_value" (i i i) -> i;
    Restore = "host" "restore" (i i) -> i;
    ReadData = "host" "read_data" (i i);
    ProcCheck = "host" "proc_check" (i i) -> i;
    ProcEnd = "host" "proc_end" (i) -> i;
    DynOp = "host" "dyn_op" (i f i f i) -> f;
    Wait = "host" "wait" (i i) -> i;
    StrChunk = "host" "str_chunk" (i) -> i;
    StrConst = "rt" "str_const" (i i) -> i;
    FnDef = "host" "fn_def" (i i) -> i;
    WhileEnd = "host" "while_end" (i i) -> i;
    Dim = "host" "dim" (i i i i i) -> i;
    MatchResident = "host" "match_resident" (i i) -> i;
    SortArray = "rt" "sort_array" (i);
    MatchArray = "rt" "match_array" (i) -> i;
    NextDone = "host" "next_done" (i) -> i;
    ParamS = "host" "param_s" () -> i;
    IntF = "rt" "int_f" (f) -> f;
    Pow = "rt" "pow" (f f) -> f;
}

/// Sentinels returned by the integer helpers.
const OVF: i64 = 1 << 32;
const DIV0: i64 = 1 << 33;

/// A variable or element whose indices were evaluated.
enum Place {
    Scalar(u16),
    /// Element of a linear array: local holding its address.
    Linear(u32),
    /// Element of an array kept in the interpreter: slot, local holding
    /// the flat index.
    Resident(u16, u32),
}

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
    layout: Layout,
    /// Arrays kept in the interpreter (`structure::var_key`s, sorted).
    resident_arrays: &'a [(usize, u16)],
    /// Scalars kept in the interpreter (`structure::resident_vars`).
    resident_vars: Vec<(usize, u16)>,
    /// Scope of the instruction being compiled.
    scope: usize,
    /// Point number of the instruction being compiled.
    k: u32,
    instrs: &'a [Instr],
    double: bool,
    code: Vec<W<'static>>,
    labels: Vec<Lbl>,
    anon: u32,
    locals: Vec<ValType>,
    free: Vec<(u32, ValType)>,
    fn_imul: u32,
    fn_idiv: u32,
    /// Function index of the first single precision helper.
    fn_ffp: u32,
    /// Function index of the first string helper.
    fn_str: u32,
    /// Function index of the first number <-> text helper.
    fn_num: u32,
    /// The program uses the number <-> text helpers (their code is large:
    /// left out of programs that do not).
    uses_num: bool,
    /// Positions of the string constants (index = slot in the table).
    consts: Vec<usize>,
    /// Position of the instruction being compiled.
    pos: usize,
    /// First free keyword bridge slot (`layout::BRIDGE_SLOT`): the
    /// parameters of the calls being evaluated are in the slots before.
    bridge_top: u32,
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

    /// Calls a number <-> text helper of the module.
    fn nhelper(&mut self, h: N) {
        self.uses_num = true;
        self.w(W::Call(self.fn_num + h as u32));
    }

    /// Calls a string helper of the module.
    fn shelper(&mut self, h: S) {
        self.w(W::Call(self.fn_str + h as u32));
    }

    /// Raises `code` if the value on the stack is -1 (sentinel of the string
    /// helpers); leaves the value.
    fn check_sentinel(&mut self, code: u16) {
        let t = self.tmp(ValType::I32);
        self.w(W::LocalTee(t));
        self.i32c(-1);
        self.w(W::I32Eq);
        self.if_(BlockType::Empty);
        self.raise(code);
        self.end();
        self.get(t);
        self.release(t, ValType::I32);
    }

    /// Raises `code` if the condition on the stack is true.
    fn raise_if(&mut self, code: u16) {
        self.if_(BlockType::Empty);
        self.raise(code);
        self.end();
    }

    /// Calls a single precision helper of the module.
    fn helper(&mut self, h: H) {
        self.w(W::Call(self.fn_ffp + h as u32));
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

    /// Continues at `point`: a direct branch when it is a later instruction
    /// (its block encloses this code), else through the dispatcher.
    fn jump_point(&mut self, point: u32) {
        if point > self.k {
            self.br(Lbl::Point(point));
        } else {
            self.i32c(point as i32);
            self.status_jump();
        }
    }

    /// A status on the stack, usually `expected` (a point): a direct branch
    /// for it when it is a later instruction, the dispatcher otherwise.
    fn status_jump_expect(&mut self, expected: Option<u32>) {
        match expected {
            Some(pt) if pt > self.k => {
                self.w(W::LocalTee(L_ST));
                self.i32c(pt as i32);
                self.w(W::I32Eq);
                self.br_if(Lbl::Point(pt));
                self.br(Lbl::Status);
            }
            _ => self.status_jump(),
        }
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

    /// True if array `slot` is in linear memory (not kept in the
    /// interpreter).
    fn is_linear(&self, slot: u16) -> bool {
        self.resident_arrays.binary_search(&structure::var_key(self.scope, slot)).is_err()
    }

    /// Evaluates the indices of an element of the linear array `slot` and
    /// returns a local holding its absolute address (`Interp::var_ref`:
    /// indices first, then error 27 for a non dimensioned array, 23 for a
    /// bad index).
    fn elem_addr(&mut self, slot: u16, ty: u8, idx: &[Expr]) -> u32 {
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
        let d = self.tmp(ValType::I32);
        self.load_var(slot, Ty::Int);
        self.w(W::LocalTee(d));
        self.w(W::I32Eqz);
        self.if_(BlockType::Empty);
        self.raise(errors::NON_DIMENSIONED_ARRAY);
        self.end();
        self.get(d);
        self.w(W::I32Load(mem32(layout::ARR_NDIMS)));
        self.i32c(temps.len() as i32);
        self.w(W::I32Ne);
        self.if_(BlockType::Empty);
        self.raise(errors::ILLEGAL_FUNCTION_CALL);
        self.end();
        let (flat, m) = (self.tmp(ValType::I32), self.tmp(ValType::I32));
        self.i32c(0);
        self.set(flat);
        for (j, &t) in temps.iter().enumerate() {
            self.get(d);
            self.w(W::I32Load(mem32(layout::ARR_DIMS + j as u32 * 4)));
            self.w(W::LocalTee(m));
            self.get(t);
            self.w(W::I32LtU);
            self.if_(BlockType::Empty);
            self.raise(errors::ILLEGAL_FUNCTION_CALL);
            self.end();
            self.get(flat);
            self.get(m);
            self.i32c(1);
            self.w(W::I32Add);
            self.w(W::I32Mul);
            self.get(t);
            self.w(W::I32Add);
            self.set(flat);
            self.release(t, ValType::I32);
        }
        self.get(d);
        self.get(flat);
        self.i32c(layout::elem_size(ty) as i32);
        self.w(W::I32Mul);
        self.w(W::I32Add);
        self.i32c(layout::ARR_DATA as i32);
        self.w(W::I32Add);
        self.set(d);
        self.release(flat, ValType::I32);
        self.release(m, ValType::I32);
        d
    }

    /// Locates a variable or element (evaluating its indices).
    fn place(&mut self, lv: &LValue) -> Place {
        match lv {
            LValue::Scalar { slot, .. } => Place::Scalar(*slot),
            LValue::Elem { slot, ty, idx } => {
                if self.is_linear(*slot) {
                    Place::Linear(self.elem_addr(*slot, *ty, idx))
                } else {
                    Place::Resident(*slot, self.aref(*slot, idx))
                }
            }
        }
    }

    fn load_place(&mut self, p: &Place, ty: u8) {
        match *p {
            Place::Scalar(slot) => self.load_var(slot, Ty::of_var(ty)),
            Place::Linear(a) => {
                self.get(a);
                self.w(if ty == 1 { W::F64Load(mem64(0)) } else { W::I32Load(mem32(0)) });
            }
            Place::Resident(slot, f) => {
                self.i32c(slot as i32);
                self.get(f);
                self.call(match ty {
                    0 => Imp::AgetI,
                    1 => Imp::AgetF,
                    _ => Imp::AgetS,
                });
            }
        }
    }

    /// Pushes what a store to `p` needs before the value; returns the
    /// memory offset for `store_place`.
    fn store_prefix(&mut self, p: &Place) -> u32 {
        match *p {
            Place::Scalar(slot) => self.var_addr(slot),
            Place::Linear(a) => {
                self.get(a);
                0
            }
            Place::Resident(slot, f) => {
                self.i32c(slot as i32);
                self.get(f);
                0
            }
        }
    }

    /// Stores the value (of the variable's type) on the stack.
    fn store_place(&mut self, p: &Place, off: u32, ty: u8) {
        match p {
            Place::Scalar(_) | Place::Linear(_) => self.store(off, ty),
            Place::Resident(..) => {
                self.call(match ty {
                    0 => Imp::AsetI,
                    1 => Imp::AsetF,
                    _ => Imp::AsetS,
                });
                self.err_check();
            }
        }
    }

    fn release_place(&mut self, p: Place) {
        if let Place::Linear(l) | Place::Resident(_, l) = p {
            self.release(l, ValType::I32);
        }
    }

    /// Loads the descriptor of linear array `slot`, error 27 if it is not
    /// dimensioned; returns the local holding it.
    fn array_desc(&mut self, slot: u16) -> u32 {
        let d = self.tmp(ValType::I32);
        self.load_var(slot, Ty::Int);
        self.w(W::LocalTee(d));
        self.w(W::I32Eqz);
        self.if_(BlockType::Empty);
        self.raise(errors::NON_DIMENSIONED_ARRAY);
        self.end();
        d
    }

    /// Evaluates expressions for their effects only (`Interp::array_ref`).
    fn exprs_dropped(&mut self, es: &[Expr]) {
        for e in es {
            self.expr(e);
            self.w(W::Drop);
            if e.ty == Ty::Dyn {
                self.w(W::Drop);
            }
        }
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
            self.helper(H::I2f);
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
    /// Evaluates the parameters of a keyword bridge call into the bridge
    /// slots (`layout::BRIDGE_SLOT`) and returns the first slot; -1 if they
    /// do not fit (then pushed on the runtime's stack).
    fn bridge_args(&mut self, args: &[Expr]) -> i32 {
        let base = self.bridge_top;
        if args.is_empty() {
            return base as i32;
        }
        if base + args.len() as u32 > layout::BRIDGE_SLOTS {
            for a in args {
                self.expr(a);
                self.push_value(a.ty);
            }
            return -1;
        }
        // Calls inside the parameters use the following slots.
        self.bridge_top = base + args.len() as u32;
        for (k, a) in args.iter().enumerate() {
            let slot = self.layout.bridge + (base + k as u32) * layout::BRIDGE_SLOT;
            match a.ty {
                Ty::Int | Ty::Str => {
                    let t = self.tmp(ValType::I32);
                    self.expr(a);
                    self.set(t);
                    self.get(L_BASE);
                    self.get(t);
                    self.w(W::I32Store(mem32(slot + 8)));
                    self.release(t, ValType::I32);
                    self.get(L_BASE);
                    self.i32c(if a.ty == Ty::Int { layout::BRIDGE_INT } else { layout::BRIDGE_STR });
                    self.w(W::I32Store(mem32(slot)));
                }
                Ty::Float => {
                    let t = self.tmp(ValType::F64);
                    self.expr(a);
                    self.set(t);
                    self.get(L_BASE);
                    self.get(t);
                    self.w(W::F64Store(mem64(slot + 8)));
                    self.release(t, ValType::F64);
                    self.get(L_BASE);
                    self.i32c(layout::BRIDGE_FLOAT);
                    self.w(W::I32Store(mem32(slot)));
                }
                Ty::Dyn => {
                    // f64 payload and runtime type (0 integer, 1 float).
                    let (x, tg) = (self.tmp(ValType::F64), self.tmp(ValType::I32));
                    self.expr(a);
                    self.set(tg);
                    self.set(x);
                    self.get(L_BASE);
                    self.get(x);
                    self.w(W::F64Store(mem64(slot + 8)));
                    self.get(L_BASE);
                    self.i32c(layout::BRIDGE_FLOAT);
                    self.i32c(layout::BRIDGE_DYN_INT);
                    self.get(tg);
                    self.w(W::Select);
                    self.w(W::I32Store(mem32(slot)));
                    self.release(x, ValType::F64);
                    self.release(tg, ValType::I32);
                }
            }
        }
        self.bridge_top = base;
        base as i32
    }

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
                // Cached in the constant table once created.
                let idx = self.consts.binary_search(p).expect("string constant") as u32;
                let t = self.tmp(ValType::I32);
                self.get(L_BASE);
                self.w(W::I32Load(mem32(self.layout.consts + idx * 4)));
                self.w(W::LocalTee(t));
                self.w(W::I32Eqz);
                self.if_(BlockType::Result(ValType::I32));
                self.i32c(*p as i32);
                self.i32c(idx as i32);
                self.call(Imp::StrConst);
                self.else_();
                self.get(t);
                self.end();
                self.release(t, ValType::I32);
            }
            ExprKind::Var(slot) => self.load_var(*slot, e.ty),
            ExprKind::Elem(slot, idx) => {
                let ty = match e.ty {
                    Ty::Float => 1,
                    Ty::Str => 2,
                    _ => 0,
                };
                let lv = LValue::Elem { slot: *slot, ty, idx: idx.clone() };
                let p = self.place(&lv);
                self.load_place(&p, ty);
                self.release_place(p);
            }
            ExprKind::FnCall { slot, args, defs } => {
                // Arguments first, then the current definition
                // (`Interp::call_fn`), whose parameters get the arguments
                // before its expression is evaluated.
                let mut temps = Vec::new();
                for a in args {
                    self.expr(a);
                    let ts = match a.ty {
                        Ty::Float => vec![(self.tmp(ValType::F64), ValType::F64)],
                        Ty::Dyn => vec![(self.tmp(ValType::F64), ValType::F64), (self.tmp(ValType::I32), ValType::I32)],
                        _ => vec![(self.tmp(ValType::I32), ValType::I32)],
                    };
                    for &(l, _) in ts.iter().rev() {
                        self.set(l);
                    }
                    temps.push(ts);
                }
                let dp = self.tmp(ValType::I32);
                self.i32c(self.pos as i32);
                self.i32c(*slot as i32);
                self.call(Imp::FnDef);
                self.set(dp);
                self.err_check();
                let rs: Vec<(u32, ValType)> = match e.ty {
                    Ty::Float => vec![(self.tmp(ValType::F64), ValType::F64)],
                    Ty::Dyn => vec![(self.tmp(ValType::F64), ValType::F64), (self.tmp(ValType::I32), ValType::I32)],
                    _ => vec![(self.tmp(ValType::I32), ValType::I32)],
                };
                let done = self.block();
                for (after, params, body) in defs {
                    self.get(dp);
                    self.i32c(*after as i32);
                    self.w(W::I32Eq);
                    self.if_(BlockType::Empty);
                    for ((p, ts), a) in params.iter().zip(&temps).zip(args) {
                        let pl = self.place(p);
                        let off = self.store_prefix(&pl);
                        for &(l, _) in ts {
                            self.get(l);
                        }
                        self.convert_for(a.ty, p.ty());
                        self.store_place(&pl, off, p.ty());
                        self.release_place(pl);
                    }
                    self.expr(body);
                    if e.ty == Ty::Dyn {
                        self.as_dyn(body.ty);
                    }
                    for &(l, _) in rs.iter().rev() {
                        self.set(l);
                    }
                    self.br(done);
                    self.end();
                }
                self.raise(15);
                self.end();
                for &(l, _) in &rs {
                    self.get(l);
                }
                for (l, t) in rs.into_iter().chain(temps.into_iter().flatten()) {
                    self.release(l, t);
                }
                self.release(dp, ValType::I32);
            }
            ExprKind::Match { slot, ty, idx, value } => {
                self.exprs_dropped(idx);
                self.get(L_BASE);
                self.expr(value);
                self.convert_for(value.ty, *ty);
                self.store(layout::RET, *ty);
                if self.is_linear(*slot) {
                    let d = self.array_desc(*slot);
                    self.get(d);
                    self.release(d, ValType::I32);
                    self.call(Imp::MatchArray);
                } else {
                    self.i32c(self.pos as i32);
                    self.i32c(*slot as i32);
                    self.call(Imp::MatchResident);
                    self.err_check();
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
                let base = self.bridge_args(args);
                self.i32c(self.pos as i32);
                self.i32c(*fpos as i32);
                self.i32c(base);
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
                self.shelper(if nf == Nf::Len { S::Len } else { S::Asc });
            }
            Nf::Chr => {
                self.int_arg(&a[0]);
                let t = self.tmp(ValType::I32);
                self.w(W::LocalTee(t));
                self.i32c(255);
                self.w(W::I32GtU);
                self.raise_if(errors::ILLEGAL_FUNCTION_CALL);
                self.get(t);
                self.release(t, ValType::I32);
                self.shelper(S::Chr);
            }
            Nf::Left | Nf::Right => {
                self.expr(&a[0]);
                self.int_arg(&a[1]);
                let t = self.tmp(ValType::I32);
                self.w(W::LocalTee(t));
                self.i32c(0);
                self.w(W::I32LtS);
                self.raise_if(errors::ILLEGAL_FUNCTION_CALL);
                self.get(t);
                self.release(t, ValType::I32);
                self.shelper(if nf == Nf::Left { S::Left } else { S::Right });
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
                self.shelper(S::Mid);
                self.check_sentinel(errors::ILLEGAL_FUNCTION_CALL);
            }
            Nf::Str => {
                self.expr(&a[0]);
                match a[0].ty {
                    Ty::Int => self.shelper(S::StrI),
                    Ty::Float => {
                        self.i32c(self.double as i32);
                        self.nhelper(N::StrF);
                    }
                    _ => {
                        // Integer or float as known at run time.
                        let (x, t) = (self.tmp(ValType::F64), self.tmp(ValType::I32));
                        self.set(t);
                        self.set(x);
                        self.get(t);
                        self.if_(BlockType::Result(ValType::I32));
                        self.get(x);
                        self.i32c(self.double as i32);
                        self.nhelper(N::StrF);
                        self.else_();
                        self.get(x);
                        self.w(W::I32TruncSatF64S);
                        self.shelper(S::StrI);
                        self.end();
                        self.release(x, ValType::F64);
                        self.release(t, ValType::I32);
                    }
                }
            }
            Nf::Val => {
                self.expr(&a[0]);
                self.i32c(self.double as i32);
                self.nhelper(N::Val);
                self.hdr(layout::TAG);
            }
            Nf::Hex | Nf::Bin => {
                self.int_arg(&a[0]);
                self.i32c((nf == Nf::Hex) as i32);
                match a.get(1) {
                    Some(d) => self.int_arg(d),
                    None => self.i32c(-1),
                }
                self.nhelper(N::Radix);
            }
            Nf::Repeat => {
                self.expr(&a[0]);
                self.int_arg(&a[1]);
                let t = self.tmp(ValType::I32);
                self.w(W::LocalTee(t));
                self.i32c(207);
                self.w(W::I32GeU);
                self.raise_if(errors::ILLEGAL_FUNCTION_CALL);
                self.get(t);
                self.release(t, ValType::I32);
                self.nhelper(N::Repeat);
            }
            Nf::Instr2 | Nf::Instr3 => {
                self.expr(&a[0]);
                self.expr(&a[1]);
                if nf == Nf::Instr3 {
                    self.int_arg(&a[2]);
                    let t = self.tmp(ValType::I32);
                    self.w(W::LocalTee(t));
                    self.i32c(0);
                    self.w(W::I32LtS);
                    self.raise_if(errors::ILLEGAL_FUNCTION_CALL);
                    // start.max(1)
                    self.get(t);
                    self.i32c(1);
                    self.get(t);
                    self.i32c(1);
                    self.w(W::I32GtS);
                    self.w(W::Select);
                    self.release(t, ValType::I32);
                } else {
                    self.i32c(1);
                }
                self.shelper(S::Instr);
            }
            Nf::Upper | Nf::Lower => {
                self.expr(&a[0]);
                self.i32c((nf == Nf::Lower) as i32);
                self.shelper(S::Case);
            }
            Nf::Flip => {
                self.expr(&a[0]);
                self.shelper(S::Flip);
            }
            Nf::StringS | Nf::Space => {
                // String$(a$,n): first character n times; Space$(n).
                let (st, nt) = (self.tmp(ValType::I32), self.tmp(ValType::I32));
                if nf == Nf::StringS {
                    self.expr(&a[0]);
                    self.set(st);
                    self.int_arg(&a[1]);
                } else {
                    self.int_arg(&a[0]);
                }
                self.w(W::LocalTee(nt));
                self.i32c(0);
                self.w(W::I32LtS);
                self.raise_if(errors::ILLEGAL_FUNCTION_CALL);
                if nf == Nf::StringS {
                    // An empty a$ gives an empty string.
                    self.get(st);
                    self.if_(BlockType::Result(ValType::I32));
                    self.get(st);
                    self.w(W::I32Load8U(MemArg { offset: 4, align: 0, memory_index: 0 }));
                    self.get(nt);
                    self.i32c(0xFFFF);
                    self.w(W::I32And);
                    self.shelper(S::Fill);
                    self.else_();
                    self.i32c(0);
                    self.end();
                } else {
                    self.i32c(32);
                    self.get(nt);
                    self.i32c(0xFFFF);
                    self.w(W::I32And);
                    self.shelper(S::Fill);
                }
                self.release(st, ValType::I32);
                self.release(nt, ValType::I32);
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
                        self.shelper(S::Cmp);
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
            // Mirrored in the header by the runtime.
            Nf::Param => self.hdr(layout::PARAM_E),
            Nf::ParamF => {
                self.get(L_BASE);
                self.w(W::F64Load(mem64(layout::PARAM_F)));
            }
            Nf::ParamS => {
                // Set by an End Proc of the module since the last import, or
                // the interpreter's.
                self.hdr(layout::PARAM_SET);
                self.i32c(4);
                self.w(W::I32And);
                self.if_(BlockType::Result(ValType::I32));
                self.hdr(layout::PARAM_S);
                self.else_();
                self.call(Imp::ParamS);
                self.end();
            }
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
                    self.shelper(S::Cmp);
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
                        self.helper(H::FCmp);
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
                    self.shelper(S::Concat);
                    self.check_sentinel(errors::STRING_TOO_LONG);
                } else {
                    self.shelper(S::Minus);
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
                    self.helper(match op {
                        OP_PLUS => H::FAdd,
                        OP_MINUS => H::FSub,
                        OP_MUL => H::FMul,
                        _ => H::FDiv,
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
        let p = self.place(lv);
        let off = self.store_prefix(&p);
        self.expr(e);
        self.convert_for(e.ty, lv.ty());
        self.store_place(&p, off, lv.ty());
        self.release_place(p);
    }

    /// Restores the control stack mirror words saved in the pending entry
    /// whose address is in local `e`.
    fn restore_mirror(&mut self, e: u32) {
        for (k, &w) in layout::MIRROR_WORDS.iter().enumerate() {
            self.get(L_BASE);
            self.get(e);
            self.w(W::I32Load(mem32(layout::PE_MIRROR + k as u32 * 4)));
            self.w(W::I32Store(mem32(w)));
        }
    }

    /// The mirror of a Gosub / procedure frame on top of the control stack.
    fn routine_mirror(&mut self) {
        for (w, v) in [
            (layout::TOP_KIND, layout::TOP_OTHER),
            (layout::FOR_ADDR, 0),
            (layout::LOOP_LO, 0),
            (layout::LOOP_HI, i32::MAX),
        ] {
            self.get(L_BASE);
            self.i32c(v);
            self.w(W::I32Store(mem32(w)));
        }
    }

    /// Pushes the condition "the control stack surely has room for one more
    /// entry": `(CTL_LEN + PEND_COUNT + 1) * 42 <= STACK_LIMIT`.
    fn room_check(&mut self) {
        self.hdr(layout::CTL_LEN);
        self.hdr(layout::PEND_COUNT);
        self.w(W::I32Add);
        self.i32c(1);
        self.w(W::I32Add);
        self.i32c(layout::CTL_MAX_ENTRY);
        self.w(W::I32Mul);
        self.hdr(layout::STACK_LIMIT);
        self.w(W::I32LeS);
    }

    /// Call of procedure `index` with `nargs` parameters (converted, in the
    /// `args` area) returning to `ret`, done by the module when the stack
    /// surely has room: a pending entry (`Runtime::flush` pushes the frame
    /// on the interpreter's stack before the next import), the locals in
    /// the next memory frame, then the body. Emits nothing (the caller does
    /// the call through the runtime) for procedures whose parameters are
    /// kept in the interpreter.
    fn native_call(&mut self, index: usize, nargs: usize, ret: usize) -> Result<(), CompileError> {
        let p = &self.prg.procs[index];
        let params: Vec<(u16, u8)> = p.params.iter().copied().zip(p.param_types.iter().copied()).take(nargs).collect();
        let n_locals = p.locals.len() as u32;
        let resident = params.iter().any(|&(slot, _)| {
            let scope = if slot & GLOBAL != 0 { 0 } else { index + 1 };
            self.resident_vars.binary_search(&structure::var_key(scope, slot)).is_ok()
        });
        if p.machine_code || resident {
            return Ok(());
        }
        let (body, rpt) = (self.point_of(p.body)?, self.point_of(ret)?);
        self.room_check();
        self.if_(BlockType::Empty);
        let e = self.tmp(ValType::I32);
        self.get(L_BASE);
        self.hdr(layout::PEND_COUNT);
        self.i32c(layout::PEND_ENTRY as i32);
        self.w(W::I32Mul);
        self.w(W::I32Add);
        self.i32c(self.layout.pending as i32);
        self.w(W::I32Add);
        self.set(e);
        for (off, v) in [(layout::PE_RET, ret as i32), (layout::PE_POINT, rpt as i32), (layout::PE_KIND, index as i32)]
        {
            self.get(e);
            self.i32c(v);
            self.w(W::I32Store(mem32(off)));
        }
        let saved = layout::MIRROR_WORDS.iter().enumerate().map(|(k, &w)| (layout::PE_MIRROR + k as u32 * 4, w));
        let saved: Vec<(u32, u32)> = saved
            .chain([
                (layout::PE_FP, layout::FP),
                (layout::PE_SCOPE, layout::SCOPE),
                (layout::PE_PREV, layout::PEND_PROC),
            ])
            .collect();
        for (off, w) in saved {
            self.get(e);
            self.hdr(w);
            self.w(W::I32Store(mem32(off)));
        }
        self.release(e, ValType::I32);
        // PEND_PROC = PEND_COUNT = PEND_COUNT + 1
        let n = self.tmp(ValType::I32);
        self.get(L_BASE);
        self.hdr(layout::PEND_COUNT);
        self.i32c(1);
        self.w(W::I32Add);
        self.w(W::LocalTee(n));
        self.w(W::I32Store(mem32(layout::PEND_COUNT)));
        self.get(L_BASE);
        self.get(n);
        self.w(W::I32Store(mem32(layout::PEND_PROC)));
        self.release(n, ValType::I32);
        // Global parameters.
        for (k, &(slot, _)) in params.iter().enumerate() {
            if slot & GLOBAL != 0 {
                self.get(L_BASE);
                self.get(L_BASE);
                self.w(W::I64Load(mem64(self.layout.args + k as u32 * 8)));
                self.w(W::I64Store(mem64(layout::GLOBALS + (slot & !GLOBAL) as u32 * 8)));
            }
        }
        // The new frame: base + locals + DEPTH * frame_size.
        let fp = self.tmp(ValType::I32);
        self.get(L_BASE);
        self.hdr(layout::DEPTH);
        self.i32c(self.layout.frame_size as i32);
        self.w(W::I32Mul);
        self.w(W::I32Add);
        self.i32c(self.layout.locals as i32);
        self.w(W::I32Add);
        self.set(fp);
        let is_param = |i: u32| params.iter().any(|&(slot, _)| slot & GLOBAL == 0 && slot as u32 == i);
        if n_locals > 16 {
            self.get(fp);
            self.i32c(0);
            self.i32c(n_locals as i32 * 8);
            self.w(W::MemoryFill(0));
        } else {
            for i in (0..n_locals).filter(|&i| !is_param(i)) {
                self.get(fp);
                self.w(W::I64Const(0));
                self.w(W::I64Store(mem64(i * 8)));
            }
        }
        for (k, &(slot, _)) in params.iter().enumerate() {
            if slot & GLOBAL == 0 {
                self.get(fp);
                self.get(L_BASE);
                self.w(W::I64Load(mem64(self.layout.args + k as u32 * 8)));
                self.w(W::I64Store(mem64(slot as u32 * 8)));
            }
        }
        self.get(L_BASE);
        self.get(fp);
        self.w(W::I32Store(mem32(layout::FP)));
        self.release(fp, ValType::I32);
        self.get(L_BASE);
        self.hdr(layout::DEPTH);
        self.i32c(1);
        self.w(W::I32Add);
        self.w(W::I32Store(mem32(layout::DEPTH)));
        self.get(L_BASE);
        self.i32c(index as i32 + 1);
        self.w(W::I32Store(mem32(layout::SCOPE)));
        self.routine_mirror();
        self.jump_point(body);
        self.end();
        Ok(())
    }

    /// End Proc / Pop Proc of a procedure call still pending (done by the
    /// module, see `native_call`), its value in `RET` / `RET_TAG` (`tag`: its
    /// type, -2 known at run time only): sets Param (`PARAM_SET`), drops the
    /// entry and the Gosubs above it, restores the caller's mirror words and
    /// continues at the return point. Not for the On Error procedure being
    /// run with an error (`ERR_PROC`, which the runtime handles).
    fn native_return(&mut self, tag: i32) {
        self.hdr(layout::PEND_PROC);
        self.i32c(0);
        self.w(W::I32Ne);
        self.hdr(layout::DEPTH);
        self.hdr(layout::ERR_PROC);
        self.w(W::I32Ne);
        self.w(W::I32And);
        self.if_(BlockType::Empty);
        let set_param = |g: &mut Self, t: i32| {
            let (word, bit) = match t {
                0 => (layout::PARAM_E, 1),
                1 => (layout::PARAM_F, 2),
                _ => (layout::PARAM_S, 4),
            };
            g.get(L_BASE);
            g.get(L_BASE);
            if t == 1 {
                g.w(W::F64Load(mem64(layout::RET)));
                g.w(W::F64Store(mem64(word)));
            } else {
                g.w(W::I32Load(mem32(layout::RET)));
                g.w(W::I32Store(mem32(word)));
            }
            g.get(L_BASE);
            g.hdr(layout::PARAM_SET);
            g.i32c(bit);
            g.w(W::I32Or);
            g.w(W::I32Store(mem32(layout::PARAM_SET)));
        };
        match tag {
            -1 => {}
            -2 => {
                self.hdr(layout::RET_TAG);
                self.if_(BlockType::Empty);
                set_param(self, 1);
                self.else_();
                set_param(self, 0);
                self.end();
            }
            t => set_param(self, t),
        }
        let e = self.tmp(ValType::I32);
        let k = self.tmp(ValType::I32);
        self.hdr(layout::PEND_PROC);
        self.i32c(1);
        self.w(W::I32Sub);
        self.set(k);
        self.get(L_BASE);
        self.get(k);
        self.i32c(layout::PEND_ENTRY as i32);
        self.w(W::I32Mul);
        self.w(W::I32Add);
        self.i32c(self.layout.pending as i32);
        self.w(W::I32Add);
        self.set(e);
        self.get(L_BASE);
        self.get(k);
        self.w(W::I32Store(mem32(layout::PEND_COUNT)));
        self.release(k, ValType::I32);
        for (w, off) in
            [(layout::PEND_PROC, layout::PE_PREV), (layout::FP, layout::PE_FP), (layout::SCOPE, layout::PE_SCOPE)]
        {
            self.get(L_BASE);
            self.get(e);
            self.w(W::I32Load(mem32(off)));
            self.w(W::I32Store(mem32(w)));
        }
        self.restore_mirror(e);
        self.get(L_BASE);
        self.hdr(layout::DEPTH);
        self.i32c(1);
        self.w(W::I32Sub);
        self.w(W::I32Store(mem32(layout::DEPTH)));
        self.get(e);
        self.w(W::I32Load(mem32(layout::PE_POINT)));
        self.release(e, ValType::I32);
        self.status_jump();
        self.end();
    }

    /// Gosub label `idx` returning to `ret`. Fast path (no host call) when
    /// nothing is pending at the test point, the label is in the current
    /// scope and the control stack surely has room: the Gosub becomes a
    /// pending entry in memory, pushed on the interpreter's stack before the
    /// next import (`Runtime::flush`). Always leaves.
    fn gosub(&mut self, idx: u16, ret: usize) -> Result<(), CompileError> {
        let scope = self.scope;
        let target = self.prg.scopes.get(scope).and_then(|s| s.labels.get(idx as usize)).map(|l| l.target);
        let mut expected = None;
        if let Some(target) = target {
            let (pt, rpt) = (self.point_of(target)?, self.point_of(ret)?);
            self.hdr(layout::ATT);
            self.w(W::I32Eqz);
            self.hdr(layout::SCOPE);
            self.i32c(scope as i32);
            self.w(W::I32Eq);
            self.w(W::I32And);
            self.room_check();
            self.w(W::I32And);
            self.if_(BlockType::Empty);
            {
                let e = self.tmp(ValType::I32);
                self.get(L_BASE);
                self.hdr(layout::PEND_COUNT);
                self.i32c(layout::PEND_ENTRY as i32);
                self.w(W::I32Mul);
                self.w(W::I32Add);
                self.i32c(self.layout.pending as i32);
                self.w(W::I32Add);
                self.set(e);
                self.get(e);
                self.i32c(ret as i32);
                self.w(W::I32Store(mem32(layout::PE_RET)));
                self.get(e);
                self.i32c(rpt as i32);
                self.w(W::I32Store(mem32(layout::PE_POINT)));
                self.get(e);
                self.i32c(-1);
                self.w(W::I32Store(mem32(layout::PE_KIND)));
                for (k, &w) in layout::MIRROR_WORDS.iter().enumerate() {
                    self.get(e);
                    self.hdr(w);
                    self.w(W::I32Store(mem32(layout::PE_MIRROR + k as u32 * 4)));
                }
                self.release(e, ValType::I32);
                self.get(L_BASE);
                self.hdr(layout::PEND_COUNT);
                self.i32c(1);
                self.w(W::I32Add);
                self.w(W::I32Store(mem32(layout::PEND_COUNT)));
                self.routine_mirror();
                self.jump_point(pt);
            }
            self.end();
            expected = Some(pt);
        }
        self.i32c(self.pos as i32);
        self.i32c(idx as i32);
        self.i32c(ret as i32);
        self.call(Imp::GosubLabel);
        self.status_jump_expect(expected);
        Ok(())
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
        let mut expected = None;
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
            expected = Some(pt);
        }
        self.i32c(self.pos as i32);
        self.i32c(idx as i32);
        self.call(Imp::GotoLabel);
        self.status_jump_expect(expected);
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
                        self.status_jump_expect(Some(pt));
                    }
                }
                self.end();
            }
            Stmt::For { lv, start, limit, step, body, exit } => {
                // The variable is located once (`Interp::var_ref`), then
                // assigned the start value.
                let ty = lv.ty();
                let p = self.place(lv);
                let off = self.store_prefix(&p);
                self.expr(start);
                self.convert_for(start.ty, ty);
                self.store_place(&p, off, ty);
                // Flat index of an element (-1 for a scalar).
                let flat = self.tmp(ValType::I32);
                match p {
                    Place::Scalar(_) => self.i32c(-1),
                    Place::Resident(_, f) => self.get(f),
                    Place::Linear(a) => {
                        self.get(a);
                        self.load_var(lv.slot(), Ty::Int);
                        self.w(W::I32Sub);
                        self.i32c(layout::ARR_DATA as i32);
                        self.w(W::I32Sub);
                        self.i32c(layout::elem_size(ty) as i32);
                        self.w(W::I32DivU);
                    }
                }
                self.set(flat);
                self.release_place(p);
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
                self.get(flat);
                self.get(lim);
                self.get(stp);
                self.i32c(*body as i32);
                self.i32c(*exit as i32);
                self.call(Imp::ForPush);
                self.status_check();
                self.release(flat, ValType::I32);
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
                let c = self.tmp(ValType::I32);
                self.set(c);
                // Did this program's Wend leave our entry on the stack?
                let reuse = self.tmp(ValType::I32);
                self.hdr(layout::LAZY_WHILE);
                self.hdr(layout::TOP_KIND);
                self.i32c(layout::TOP_WHILE);
                self.w(W::I32Eq);
                self.w(W::I32And);
                self.hdr(layout::TOP_START);
                self.i32c(pos);
                self.w(W::I32Eq);
                self.w(W::I32And);
                self.set(reuse);
                self.get(c);
                self.w(W::I32Eqz);
                self.if_(BlockType::Empty);
                {
                    self.get(reuse);
                    self.if_(BlockType::Empty);
                    self.i32c(pos);
                    self.i32c(*exit as i32);
                    self.call(Imp::WhileEnd);
                    self.status_jump_expect(Some(exit_point));
                    self.end();
                    self.jump_point(exit_point);
                }
                self.end();
                self.get(reuse);
                self.if_(BlockType::Empty);
                {
                    // Same entry as the interpreter pushes again: keep it.
                    self.get(L_BASE);
                    self.i32c(0);
                    self.w(W::I32Store(mem32(layout::LAZY_WHILE)));
                }
                self.else_();
                {
                    self.i32c(pos);
                    self.i32c(*body as i32);
                    self.i32c(*exit as i32);
                    self.call(Imp::WhilePush);
                    self.status_check();
                }
                self.end();
                self.release(c, ValType::I32);
                self.release(reuse, ValType::I32);
            }
            Stmt::Until(cond) => {
                self.test_point();
                self.expr(cond);
                self.as_int(cond.ty);
                let c = self.tmp(ValType::I32);
                self.set(c);
                // Fast path: top is a Repeat and the condition is false.
                self.hdr(layout::TOP_KIND);
                self.i32c(layout::TOP_REPEAT);
                self.w(W::I32Eq);
                self.get(c);
                self.w(W::I32Eqz);
                self.w(W::I32And);
                self.if_(BlockType::Empty);
                self.hdr(layout::FOR_BODY);
                self.status_jump();
                self.end();
                self.i32c(pos);
                self.get(c);
                self.release(c, ValType::I32);
                self.call(Imp::Until);
                self.status_check();
            }
            Stmt::Wend => {
                self.hdr(layout::ATT);
                self.w(W::I32Eqz);
                self.hdr(layout::TOP_KIND);
                self.i32c(layout::TOP_WHILE);
                self.w(W::I32Eq);
                self.w(W::I32And);
                self.if_(BlockType::Empty);
                self.get(L_BASE);
                self.i32c(1);
                self.w(W::I32Store(mem32(layout::LAZY_WHILE)));
                self.hdr(layout::TOP_START_POINT);
                self.status_jump();
                self.end();
                self.i32c(pos);
                self.call(Imp::Wend);
                self.status_jump();
            }
            Stmt::Loop => {
                self.hdr(layout::ATT);
                self.w(W::I32Eqz);
                self.hdr(layout::TOP_KIND);
                self.i32c(layout::TOP_DO);
                self.w(W::I32Eq);
                self.w(W::I32And);
                self.if_(BlockType::Empty);
                self.hdr(layout::FOR_BODY);
                self.status_jump();
                self.end();
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
                let pt = self.point_of(*target)?;
                self.status_jump_expect(Some(pt));
                if cond.is_some() {
                    self.end();
                }
            }
            Stmt::Goto(idx) => self.goto_label(k, *idx)?,
            Stmt::Gosub { label, ret } => self.gosub(*label, *ret)?,
            Stmt::On { n, kind, targets, after } => {
                // `Interp::exec_on`: out of range continues after the list;
                // otherwise a test point, then the jump.
                let after_pt = self.point_of(*after)?;
                self.expr(n);
                self.as_int(n.ty);
                let nt = self.tmp(ValType::I32);
                self.set(nt);
                for (i, &t) in targets.iter().enumerate() {
                    self.get(nt);
                    self.i32c(i as i32 + 1);
                    self.w(W::I32Eq);
                    self.if_(BlockType::Empty);
                    match *kind {
                        tk::GOTO => self.goto_label(k, t)?,
                        tk::GOSUB => self.gosub(t, *after)?,
                        _ => {
                            self.test_point();
                            self.i32c(pos);
                            self.i32c(t as i32);
                            self.i32c(*after as i32);
                            self.i32c(0);
                            self.call(Imp::CallProc);
                            self.status_jump();
                        }
                    }
                    self.end();
                }
                self.release(nt, ValType::I32);
                if after_pt != next {
                    self.jump_point(after_pt);
                }
            }
            Stmt::GotoExpr { e, gosub, ret } => {
                self.test_point();
                self.expr(e);
                self.push_value(e.ty);
                self.i32c(pos);
                self.i32c(*gosub as i32);
                self.i32c(*ret as i32);
                self.call(Imp::GotoValue);
                self.status_jump();
            }
            Stmt::Restore(label) => {
                self.i32c(pos);
                self.i32c(label.map_or(-1, |l| l as i32));
                self.call(Imp::Restore);
                self.status_check();
            }
            Stmt::Read(lvs) => {
                for lv in lvs {
                    let ty = lv.ty();
                    let p = self.place(lv);
                    let off = self.store_prefix(&p);
                    self.i32c(pos);
                    self.i32c(ty as i32);
                    self.call(Imp::ReadData);
                    self.err_check();
                    self.get(L_BASE);
                    self.w(if ty == 1 { W::F64Load(mem64(layout::RET)) } else { W::I32Load(mem32(layout::RET)) });
                    self.store_place(&p, off, ty);
                    self.release_place(p);
                }
            }
            Stmt::Return => {
                // Fast path: nothing at the test point, and the Gosub was
                // done by the module (pending, above any pending procedure
                // call): pop it, restore the mirror.
                self.hdr(layout::ATT);
                self.w(W::I32Eqz);
                self.hdr(layout::PEND_COUNT);
                self.hdr(layout::PEND_PROC);
                self.w(W::I32GtS);
                self.w(W::I32And);
                self.if_(BlockType::Empty);
                {
                    let e = self.tmp(ValType::I32);
                    self.get(L_BASE);
                    self.hdr(layout::PEND_COUNT);
                    self.i32c(1);
                    self.w(W::I32Sub);
                    self.w(W::LocalTee(e));
                    self.w(W::I32Store(mem32(layout::PEND_COUNT)));
                    self.get(L_BASE);
                    self.get(e);
                    self.i32c(layout::PEND_ENTRY as i32);
                    self.w(W::I32Mul);
                    self.w(W::I32Add);
                    self.i32c(self.layout.pending as i32);
                    self.w(W::I32Add);
                    self.set(e);
                    self.restore_mirror(e);
                    self.get(e);
                    self.w(W::I32Load(mem32(layout::PE_POINT)));
                    self.release(e, ValType::I32);
                    self.status_jump();
                }
                self.end();
                self.i32c(pos);
                self.call(Imp::Return);
                self.status_jump();
            }
            Stmt::Call { proc, args, ret } => {
                self.test_point();
                // Parameters converted to their types, in the `args` area.
                let types = self.prg.procs[*proc].param_types.clone();
                for (k, (a, &ty)) in args.iter().zip(&types).enumerate() {
                    self.get(L_BASE);
                    self.expr(a);
                    self.convert_for(a.ty, ty);
                    self.store(self.layout.args + k as u32 * 8, ty);
                }
                self.native_call(*proc, args.len(), *ret)?;
                self.i32c(pos);
                self.i32c(*proc as i32);
                self.i32c(*ret as i32);
                self.i32c(args.len() as i32);
                self.call(Imp::CallProc);
                let body = self.point_of(self.prg.procs[*proc].body)?;
                self.status_jump_expect(Some(body));
            }
            Stmt::EndProc { pop, value } => {
                self.test_point();
                // No procedure running (only possible after odd jumps).
                self.hdr(layout::DEPTH);
                self.w(W::I32Eqz);
                self.if_(BlockType::Empty);
                self.i32c(pos);
                self.i32c(*pop as i32);
                self.call(Imp::ProcCheck);
                self.status_jump();
                self.end();
                let tag = match value {
                    None => -1,
                    Some(e) => {
                        self.get(L_BASE);
                        self.expr(e);
                        match e.ty {
                            Ty::Int => {
                                self.w(W::I32Store(mem32(layout::RET)));
                                0
                            }
                            Ty::Str => {
                                self.w(W::I32Store(mem32(layout::RET)));
                                2
                            }
                            Ty::Float => {
                                self.w(W::F64Store(mem64(layout::RET)));
                                1
                            }
                            Ty::Dyn => {
                                // Integer or float as known at run time.
                                let (t, x, a) =
                                    (self.tmp(ValType::I32), self.tmp(ValType::F64), self.tmp(ValType::I32));
                                self.set(t);
                                self.set(x);
                                self.set(a);
                                self.get(t);
                                self.if_(BlockType::Empty);
                                self.get(a);
                                self.get(x);
                                self.w(W::F64Store(mem64(layout::RET)));
                                self.else_();
                                self.get(a);
                                self.get(x);
                                self.w(W::I32TruncSatF64S);
                                self.w(W::I32Store(mem32(layout::RET)));
                                self.end();
                                self.get(L_BASE);
                                self.get(t);
                                self.w(W::I32Store(mem32(layout::RET_TAG)));
                                for (l, vt) in [(t, ValType::I32), (x, ValType::F64), (a, ValType::I32)] {
                                    self.release(l, vt);
                                }
                                -2
                            }
                        }
                    }
                };
                if tag != -2 {
                    self.get(L_BASE);
                    self.i32c(tag);
                    self.w(W::I32Store(mem32(layout::RET_TAG)));
                }
                self.native_return(tag);
                self.i32c(pos);
                self.call(Imp::ProcEnd);
                self.status_jump();
            }
            Stmt::IncDec { lv, inc } => {
                let ty = lv.ty();
                let p = self.place(lv);
                let off = self.store_prefix(&p);
                self.load_place(&p, ty);
                if ty == 1 {
                    // `f + 1.0` without rounding, as the interpreter.
                    self.w(W::F64Const(1f64.into()));
                    self.w(if *inc { W::F64Add } else { W::F64Sub });
                } else {
                    self.i32c(1);
                    self.w(if *inc { W::I32Add } else { W::I32Sub });
                }
                self.store_place(&p, off, ty);
                self.release_place(p);
            }
            Stmt::Add { lv, n, range } => {
                let ty = lv.ty();
                let p = self.place(lv);
                let off = self.store_prefix(&p);
                self.expr(n);
                self.as_int(n.ty);
                let nt = self.tmp(ValType::I32);
                self.set(nt);
                self.load_place(&p, ty);
                if ty == 1 {
                    self.w(W::I32TruncSatF64S);
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
                if ty == 1 {
                    self.int_to_float();
                }
                self.store_place(&p, off, ty);
                self.release_place(p);
            }
            Stmt::Swap(a, b) => {
                let ty = a.ty();
                let vt = if ty == 1 { ValType::F64 } else { ValType::I32 };
                let pa = self.place(a);
                let pb = self.place(b);
                let (va, vb) = (self.tmp(vt), self.tmp(vt));
                self.load_place(&pa, ty);
                self.set(va);
                self.load_place(&pb, ty);
                self.set(vb);
                let off = self.store_prefix(&pa);
                self.get(vb);
                self.store_place(&pa, off, ty);
                let off = self.store_prefix(&pb);
                self.get(va);
                self.store_place(&pb, off, ty);
                self.release(va, vt);
                self.release(vb, vt);
                self.release_place(pa);
                self.release_place(pb);
            }
            Stmt::Dim(arrays) => {
                for (slot, ty, dims) in arrays {
                    // Each maximum index is checked as soon as it is known,
                    // the element count after each comma (`Interp::dim`).
                    let count = self.tmp(ValType::I32);
                    self.i32c(1);
                    self.set(count);
                    let mut temps = Vec::new();
                    for (j, e) in dims.iter().enumerate() {
                        self.expr(e);
                        self.as_int(e.ty);
                        let t = self.tmp(ValType::I32);
                        self.w(W::LocalTee(t));
                        self.i32c(0xFFFF);
                        self.w(W::I32GeU);
                        self.if_(BlockType::Empty);
                        self.raise(errors::ILLEGAL_FUNCTION_CALL);
                        self.end();
                        if j + 1 < dims.len() {
                            self.get(count);
                            self.get(t);
                            self.i32c(1);
                            self.w(W::I32Add);
                            self.w(W::I32Mul);
                            self.w(W::LocalTee(count));
                            self.i32c(0x10000);
                            self.w(W::I32GeU);
                            self.if_(BlockType::Empty);
                            self.raise(errors::ILLEGAL_FUNCTION_CALL);
                            self.end();
                        }
                        temps.push(t);
                    }
                    for (j, t) in temps.into_iter().enumerate() {
                        self.get(L_BASE);
                        self.get(t);
                        self.w(W::I32Store(mem32(layout::IDX + j as u32 * 4)));
                        self.release(t, ValType::I32);
                    }
                    self.release(count, ValType::I32);
                    self.i32c(pos);
                    self.i32c(*slot as i32);
                    self.i32c(*ty as i32);
                    self.i32c(dims.len() as i32);
                    self.i32c(!self.is_linear(*slot) as i32);
                    self.call(Imp::Dim);
                    self.status_check();
                }
            }
            Stmt::Sort { slot, idx } => {
                if self.is_linear(*slot) {
                    self.exprs_dropped(idx);
                    let d = self.array_desc(*slot);
                    self.get(d);
                    self.release(d, ValType::I32);
                    self.call(Imp::SortArray);
                } else {
                    self.i32c(pos);
                    self.call(Imp::Interp);
                    self.status_jump();
                }
            }
            Stmt::MidAssign { kind, lv, nums, e } => {
                // `Interp::mid_assign`: numbers, value, then the current
                // string; negative numbers are an error.
                let p = self.place(lv);
                let off = self.store_prefix(&p);
                let mut nts = Vec::new();
                for n in nums {
                    self.int_arg(n);
                    let t = self.tmp(ValType::I32);
                    self.set(t);
                    nts.push(t);
                }
                self.expr(e);
                let et = self.tmp(ValType::I32);
                self.set(et);
                self.load_place(&p, 2);
                let cur = self.tmp(ValType::I32);
                self.set(cur);
                for &t in &nts {
                    self.get(t);
                    self.i32c(0);
                    self.w(W::I32LtS);
                    self.raise_if(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.get(cur);
                let max1_minus1 = |g: &mut Self, t: u32| {
                    g.get(t);
                    g.i32c(1);
                    g.get(t);
                    g.i32c(1);
                    g.w(W::I32GtS);
                    g.w(W::Select);
                    g.i32c(1);
                    g.w(W::I32Sub);
                };
                match *kind {
                    tk::MID_S => {
                        max1_minus1(self, nts[0]);
                        self.get(nts[1]);
                    }
                    tk::MID_S_2 => {
                        max1_minus1(self, nts[0]);
                        self.i32c(-1);
                    }
                    tk::LEFT_S => {
                        self.i32c(0);
                        self.get(nts[0]);
                    }
                    _ => {
                        // Right$: n >= len ? (0, len) : (len - n, n)
                        let l = self.tmp(ValType::I32);
                        self.get(cur);
                        self.shelper(S::Len);
                        self.set(l);
                        // start: n >= len ? 0 : len - n
                        self.i32c(0);
                        self.get(l);
                        self.get(nts[0]);
                        self.w(W::I32Sub);
                        self.get(nts[0]);
                        self.get(l);
                        self.w(W::I32GeU);
                        self.w(W::Select);
                        // count: n >= len ? len : n
                        self.get(l);
                        self.get(nts[0]);
                        self.get(nts[0]);
                        self.get(l);
                        self.w(W::I32GeU);
                        self.w(W::Select);
                        self.release(l, ValType::I32);
                    }
                }
                self.get(et);
                self.shelper(S::MidSet);
                self.store_place(&p, off, 2);
                self.release_place(p);
                for t in nts {
                    self.release(t, ValType::I32);
                }
                self.release(et, ValType::I32);
                self.release(cur, ValType::I32);
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
                let base = self.bridge_args(args);
                self.i32c(pos);
                self.i32c(base);
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
            self.scope = self.instrs[k].scope;
            self.k = k as u32;
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
pub fn module(
    prg: &Compiled,
    instrs: &[Instr],
    stmts: &[Stmt],
    resident_arrays: &[(usize, u16)],
) -> Result<Vec<u8>, CompileError> {
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
    for (_, _, p, r) in ffp_helpers::HELPERS {
        let t = type_of(&mut types, p, r);
        funcs.function(t);
    }
    for (_, p, r) in string_helpers::HELPERS {
        let t = type_of(&mut types, p, r);
        funcs.function(t);
    }
    for (_, _, p, r) in num_helpers::HELPERS {
        let t = type_of(&mut types, p, r);
        funcs.function(t);
    }

    let mut g = Gen {
        prg,
        layout: lay,
        resident_arrays,
        resident_vars: structure::resident_vars(prg, instrs),
        scope: 0,
        k: 0,
        instrs,
        double: prg.double,
        code: Vec::new(),
        labels: Vec::new(),
        anon: 0,
        locals: vec![ValType::I32; (N_FIXED - 1) as usize],
        free: Vec::new(),
        fn_imul: n_imports + 1,
        fn_idiv: n_imports + 2,
        fn_ffp: n_imports + 3,
        fn_str: n_imports + 3 + ffp_helpers::HELPERS.len() as u32,
        fn_num: n_imports + 3 + (ffp_helpers::HELPERS.len() + string_helpers::HELPERS.len()) as u32,
        uses_num: false,
        consts: structure::string_constants(prg),
        pos: 0,
        bridge_top: 0,
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
    for (h, ..) in ffp_helpers::HELPERS {
        code.function(&ffp_helpers::body(*h, n_imports + 3));
    }
    let first_str = n_imports + 3 + ffp_helpers::HELPERS.len() as u32;
    for (h, ..) in string_helpers::HELPERS {
        code.function(&string_helpers::body(*h, first_str, Imp::StrChunk as u32));
    }
    let ix =
        num_helpers::Idx { ffp: n_imports + 3, str: first_str, num: first_str + string_helpers::HELPERS.len() as u32 };
    for (h, ..) in num_helpers::HELPERS {
        if g.uses_num {
            code.function(&num_helpers::body(*h, &ix, prg.double));
        } else {
            let mut f = Function::new([]);
            f.instruction(&W::Unreachable);
            f.instruction(&W::End);
            code.function(&f);
        }
    }

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
