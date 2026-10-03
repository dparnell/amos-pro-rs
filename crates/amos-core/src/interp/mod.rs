//! The AMOS interpreter.
//!
//! The program runs directly from its (verified) token stream, like the
//! original: `pc` is a byte offset into [`verify::Compiled::code`].
//! Execution is cooperative: [`Interp::run`] executes instructions until the
//! program has to wait (Wait Vbl, Wait Key, Input...) or a time budget is
//! used, so the host can keep rendering at 50 Hz, also in a browser.
//!
//! Instruction implementations are spread over several files, each adding
//! `impl Interp` blocks: control flow (`flow.rs`), core statements and
//! functions (`stmt.rs`, `expr.rs`), and the subsystems.

pub mod direct;
pub mod expr;
pub mod flow;
pub mod params;
pub mod stmt;
pub mod value;
pub mod verify;

use std::rc::Rc;

use crate::errors;
use crate::program::Program;
use crate::tokens::*;
use value::{Array, Value, Var};
use verify::{Compiled, GLOBAL, TestError, Verifier};

/// Why execution stopped (non-local exits are carried in `Err`).
#[derive(Clone, Debug, PartialEq)]
pub enum Exc {
    /// Runtime error number (see `errors`).
    Error(u16),
    /// Error with an extension message (`Error` with text).
    Message(String),
    /// The instruction must wait: it will be executed again later.
    Block,
    /// An event (Every, On Break, menu) moved pc to its handler: the
    /// interrupted instruction is abandoned and executed again on return.
    Jump,
    /// `End`, `Edit`, `Direct`, `System` or end of program.
    Stop(StopReason),
    /// Test-time error while verifying (Run, Include, direct mode line).
    Test(TestError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    End,
    Edit,
    Direct,
    System,
    /// Stop instruction / Ctrl-C ("Program interrupted").
    Break,
}

pub type R<T> = Result<T, Exc>;

pub fn err<T>(n: u16) -> R<T> {
    Err(Exc::Error(n))
}

/// Result of a call to [`Interp::run`].
#[derive(Clone, Debug, PartialEq)]
pub enum RunState {
    /// No program running.
    Idle,
    /// Still running (time budget used, or waiting).
    Running,
    /// Program ended or stopped with an error.
    Stopped(StopInfo),
}

#[derive(Clone, Debug, PartialEq)]
pub struct StopInfo {
    pub reason: StopReasonOrError,
    /// Position in the code of the instruction (for the editor).
    pub pos: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StopReasonOrError {
    Stop(StopReason),
    Error(u16),
    Message(String),
    Test(u16),
}

/// Entry of the control stack (loops, Gosub and procedure frames).
#[derive(Clone, Debug)]
pub enum Ctl {
    For { var: VarLoc, step: i32, limit: i32, body: usize, exit: usize },
    Repeat { body: usize, exit: usize },
    While { start: usize, body: usize, exit: usize },
    Do { body: usize, exit: usize },
    Gosub { ret: usize },
    Proc(Box<ProcFrame>),
}

impl Ctl {
    /// Bytes used by the frame in the original (for "Out of stack space").
    fn size(&self) -> usize {
        match self {
            Ctl::For { .. } => 24,
            Ctl::Repeat { .. } | Ctl::While { .. } | Ctl::Do { .. } => 10,
            Ctl::Gosub { .. } => 12,
            Ctl::Proc(_) => 42,
        }
    }
}

/// Saved state of a procedure call.
#[derive(Clone, Debug, Default)]
pub struct ProcFrame {
    pub proc_index: usize,
    /// Where to continue after End Proc (`None`: return to the caller of
    /// an event handler, e.g. a menu).
    pub ret: usize,
    pub locals: Vec<Var>,
    pub data: DataPtr,
    pub on_error: OnError,
    pub error_on: u16,
    pub error_pos: usize,
    pub scope: usize,
}

/// Position of the next Data item.
#[derive(Clone, Copy, Debug, Default)]
pub struct DataPtr {
    /// Start of the scope (Restore without label).
    pub base: usize,
    /// Next line to search for Data.
    pub line: usize,
    /// Next item within the current Data statement (0 = none).
    pub item: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnError {
    #[default]
    None,
    Goto(usize),
    Proc(usize),
}

/// A resolved variable (scalar or array element).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VarLoc {
    pub slot: u16,
    /// Flat index for array elements.
    pub index: Option<usize>,
    /// Procedure depth the local belongs to (index in `frames`).
    pub frame: usize,
}

/// Every / On Break / menu jump handling (`ActuMask`).
#[derive(Clone, Debug, Default)]
pub struct Events {
    pub break_on: bool,
    pub on_break_proc: Option<usize>,
    pub every_on: bool,
    pub every_count: i32,
    pub every_reload: i32,
    pub every_target: Option<EveryTarget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EveryTarget {
    Gosub(usize),
    Proc(usize),
}

/// State of a waiting instruction.
#[derive(Clone, Debug, PartialEq)]
pub struct Wait {
    /// Position of the instruction.
    pub pos: usize,
    pub kind: WaitKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WaitKind {
    /// Until the VBL counter reaches this value.
    Vbl(u64),
    Key,
    Click,
    /// Line input in progress.
    Input(Box<stmt::InputState>),
    /// Anything else, identified by a number (instruction specific).
    Other(u32, u64),
}

/// The complete interpreter state.
pub struct Interp {
    pub prg: Option<Rc<Compiled>>,
    /// Code currently executed (program + direct mode line).
    pub code: Rc<Vec<u8>>,
    pub pc: usize,
    /// Start of the instruction being executed.
    pub inst_pos: usize,
    pub globals: Vec<Var>,
    pub ctl: Vec<Ctl>,
    ctl_bytes: usize,
    pub stack_limit: usize,
    /// Procedure frames: index into `ctl` of each active procedure frame.
    pub frame_stack: Vec<usize>,
    pub scope: usize,
    pub data: DataPtr,
    pub events: Events,
    pub wait: Option<Wait>,
    // Errors
    pub on_error: OnError,
    pub error_on: u16,
    pub error_pos: usize,
    pub trap_pos: Option<usize>,
    pub trap_err: u16,
    /// Procedure depth of the On Error Proc handler being executed.
    pub error_proc_depth: Option<usize>,
    /// Label recorded by `Resume Label label`.
    pub resume_label: Option<usize>,
    // Procedure results
    pub param_e: i32,
    pub param_f: f64,
    pub param_s: value::AStr,
    // Maths state
    pub degrees: bool,
    pub fix: crate::ffp::Fix,
    pub double: bool,
    pub seed: u32,
    pub old_rnd: i32,
    /// Set by the 50 Hz interrupt; the next test point handles events.
    pub vbl_pending: bool,
    pub vbl_count: u64,
    pub running: bool,
    pub direct_mode: bool,
    /// Length of the program part of `code` (direct lines are appended).
    prog_len: usize,
    /// Main library tokens that `core_function` does not handle (bit set,
    /// learnt as they are met): their calls go straight to the host.
    not_core: Vec<u64>,
    /// Main library instruction tokens handled by the host (learnt as they
    /// are met): 1 instruction, 2 reserved variable assignment, 0 unknown.
    host_inst: Vec<u8>,
    /// Parameters of the next `inst_args` / `func_args` call, already
    /// evaluated by compiled code (`preset_args`).
    /// (The vector is kept between presets: no allocation per call.)
    preset: Vec<Option<Value>>,
    /// `preset` holds parameters for the next call.
    preset_set: bool,
    /// Frames of returned procedures, reused by the next calls (boxed: the
    /// boxes themselves are reused, `Ctl::Proc` holds one).
    #[allow(clippy::vec_box)]
    frame_pool: Vec<Box<ProcFrame>>,
    /// Vector reused for the arguments of procedure calls.
    proc_args: Vec<Value>,
    /// The program's code as `start` set it, and the string constants
    /// met in it (index = position / 2): evaluating a constant again
    /// shares its string instead of allocating a copy. Only used while
    /// `code` is this code (not for direct mode lines or code swapped in).
    prog_code: Rc<Vec<u8>>,
    str_consts: Vec<Option<value::AStr>>,
}

impl Default for Interp {
    fn default() -> Self {
        Self::new()
    }
}

impl Interp {
    pub fn new() -> Self {
        Interp {
            prg: None,
            code: Rc::new(vec![0, 0, 0, 0]),
            pc: 0,
            inst_pos: 0,
            globals: Vec::new(),
            ctl: Vec::new(),
            ctl_bytes: 0,
            stack_limit: 52 * 42,
            frame_stack: Vec::new(),
            scope: 0,
            data: DataPtr::default(),
            events: Events { break_on: true, ..Default::default() },
            wait: None,
            on_error: OnError::None,
            error_on: 0,
            error_pos: 0,
            trap_pos: None,
            trap_err: 0,
            error_proc_depth: None,
            resume_label: None,
            param_e: 0,
            param_f: 0.0,
            param_s: value::empty_str(),
            degrees: false,
            fix: crate::ffp::Fix::Free,
            double: false,
            seed: 0,
            old_rnd: 0,
            vbl_pending: false,
            vbl_count: 0,
            running: false,
            direct_mode: false,
            prog_len: 0,
            not_core: Vec::new(),
            host_inst: Vec::new(),
            preset: Vec::new(),
            preset_set: false,
            frame_pool: Vec::new(),
            proc_args: Vec::new(),
            prog_code: Rc::new(Vec::new()),
            str_consts: Vec::new(),
        }
    }

    /// Verifies `program` and prepares to run it from the start.
    pub fn load(&mut self, program: &Program) -> Result<(), TestError> {
        let compiled = Verifier::verify(&program.source, program.math_flags)?;
        self.start(Rc::new(compiled));
        Ok(())
    }

    pub fn start(&mut self, compiled: Rc<Compiled>) {
        self.code = Rc::new(compiled.code.clone());
        self.prog_code = self.code.clone();
        self.str_consts.clear();
        self.prog_len = compiled.code.len();
        self.globals = compiled.globals.iter().map(|_| Var::Unset).collect();
        self.ctl.clear();
        self.ctl_bytes = 0;
        self.stack_limit = (compiled.stack_size + 1) * 42 - 64;
        self.frame_stack.clear();
        self.scope = 0;
        self.data = DataPtr { base: 2, line: 0, item: 0 };
        self.events = Events { break_on: true, ..Default::default() };
        self.wait = None;
        self.on_error = OnError::None;
        self.error_on = 0;
        self.trap_pos = None;
        self.trap_err = 0;
        self.error_proc_depth = None;
        self.resume_label = None;
        self.param_e = 0;
        self.param_f = 0.0;
        self.param_s = value::empty_str();
        self.degrees = false;
        self.fix = crate::ffp::Fix::Free;
        self.double = compiled.double;
        self.pc = 2;
        self.running = true;
        self.direct_mode = false;
        self.prg = Some(compiled);
    }

    // ------------------------------------------------------------------
    // Code access
    // ------------------------------------------------------------------

    #[inline]
    pub fn rd(&self, p: usize) -> u16 {
        // (One bounds check instead of two.)
        u16::from_be_bytes(self.code[p..p + 2].try_into().unwrap())
    }

    #[inline]
    pub fn peek(&self) -> u16 {
        self.rd(self.pc)
    }

    #[inline]
    pub fn next_token(&mut self) -> u16 {
        let t = self.rd(self.pc);
        self.pc += 2;
        t
    }

    pub fn compiled(&self) -> &Compiled {
        self.prg.as_deref().expect("no program")
    }

    /// Skips a token (with its inline data) at `p`.
    pub fn skip_token(&self, p: usize) -> usize {
        p + verify::token_size(&self.code, p)
    }

    /// True if `t` ends a statement.
    pub fn is_end(t: u16) -> bool {
        t == TK_EOL || t == TK_DP || t == TK_ELSE
    }

    /// Expects the token `t` at pc.
    #[inline]
    pub fn expect(&mut self, t: u16) -> R<()> {
        if self.peek() == t {
            self.pc += 2;
            Ok(())
        } else {
            err(errors::SYNTAX_ERROR)
        }
    }

    // ------------------------------------------------------------------
    // Control stack
    // ------------------------------------------------------------------

    pub fn push_ctl(&mut self, c: Ctl) -> R<()> {
        let size = c.size();
        if self.ctl_bytes + size > self.stack_limit {
            return err(errors::OUT_OF_STACK);
        }
        self.ctl_bytes += size;
        self.ctl.push(c);
        Ok(())
    }

    pub fn pop_ctl(&mut self) -> Option<Ctl> {
        let c = self.ctl.pop()?;
        self.ctl_bytes -= c.size();
        Some(c)
    }

    /// Index in `ctl` of the innermost Gosub / Proc frame (loops above it
    /// belong to the current routine).
    pub fn routine_base(&self) -> usize {
        self.ctl.iter().rposition(|c| matches!(c, Ctl::Gosub { .. } | Ctl::Proc(_))).map_or(0, |i| i + 1)
    }

    /// After a jump: drop the loops of the current routine that do not
    /// contain the new position (`LGoto`).
    pub fn after_jump(&mut self) {
        // Usual case, decided by the innermost entry alone: a Gosub / Proc
        // frame (no loop of this routine) or a loop containing pc.
        match self.ctl.last() {
            None | Some(Ctl::Gosub { .. } | Ctl::Proc(_)) => return,
            Some(
                Ctl::For { body, exit, .. }
                | Ctl::Repeat { body, exit }
                | Ctl::Do { body, exit }
                | Ctl::While { body, exit, .. },
            ) if *body <= self.pc && self.pc <= *exit => return,
            _ => {}
        }
        let base = self.routine_base();
        while self.ctl.len() > base {
            let pc = self.pc;
            let keep = match self.ctl.last().unwrap() {
                Ctl::For { body, exit, .. }
                | Ctl::Repeat { body, exit }
                | Ctl::Do { body, exit }
                | Ctl::While { body, exit, .. } => *body <= pc && pc <= *exit,
                _ => true,
            };
            if keep {
                break;
            }
            self.pop_ctl();
        }
    }

    // ------------------------------------------------------------------
    // Variables
    // ------------------------------------------------------------------

    /// Variables of the current procedure (or globals in the main program).
    #[inline]
    fn frame_vars(&mut self, frame: usize) -> &mut Vec<Var> {
        let idx = self.frame_stack[frame];
        match &mut self.ctl[idx] {
            Ctl::Proc(f) => &mut f.locals,
            _ => unreachable!("procedure frame expected"),
        }
    }

    #[inline]
    pub fn var_slot(&mut self, slot: u16) -> &mut Var {
        if slot & GLOBAL != 0 {
            return &mut self.globals[(slot & !GLOBAL) as usize];
        }
        let depth = self.frame_stack.len();
        if depth == 0 {
            // Direct mode variables of a procedure scope are not supported:
            // locals are only valid inside procedures.
            return &mut self.globals[slot as usize];
        }
        &mut self.frame_vars(depth - 1)[slot as usize]
    }

    #[inline]
    pub fn var_loc_slot(&mut self, loc: &VarLoc) -> &mut Var {
        if loc.slot & GLOBAL != 0 || self.frame_stack.is_empty() {
            return self.var_slot(loc.slot);
        }
        let f = loc.frame.min(self.frame_stack.len() - 1);
        &mut self.frame_vars(f)[loc.slot as usize]
    }

    /// Type of the variable token at `p` (0 int, 1 float, 2 string).
    pub fn var_type_at(&self, p: usize) -> u8 {
        self.code[p + 5] & 3
    }

    #[inline]
    pub fn read_loc(&mut self, loc: &VarLoc, ty: u8) -> Value {
        let double = self.double;
        let v = self.var_loc_slot(loc);
        match (v, loc.index) {
            (Var::Scalar(v), None) => v.clone(),
            (Var::Array(a), Some(i)) => a.get(i),
            _ => {
                let _ = double;
                Value::zero(ty)
            }
        }
    }

    #[inline]
    pub fn write_loc(&mut self, loc: &VarLoc, ty: u8, val: Value) -> R<()> {
        let val = self.convert_for(ty, val)?;
        let v = self.var_loc_slot(loc);
        match loc.index {
            None => *v = Var::Scalar(val),
            Some(i) => {
                if let Var::Array(a) = v {
                    a.set(i, val);
                } else {
                    return err(errors::NON_DIMENSIONED_ARRAY);
                }
            }
        }
        Ok(())
    }

    /// Converts a value for storage in a variable of type `ty`.
    #[inline(always)]
    pub fn convert_for(&self, ty: u8, val: Value) -> R<Value> {
        Ok(match (ty, val) {
            (0, Value::Float(f)) => Value::Int(value::float_to_int(f)),
            (1, Value::Int(i)) => Value::Float(self.int_to_float(i)),
            (2, v @ Value::Str(_)) => v,
            (2, _) | (0 | 1, Value::Str(_)) => return err(errors::TYPE_MISMATCH),
            (_, v) => v,
        })
    }

    /// Reads the variable token at pc (with array indices) and returns its
    /// location and type.
    pub fn var_ref(&mut self, hw: &mut dyn Host) -> R<(VarLoc, u8)> {
        let p = self.pc;
        let t = self.rd(p);
        if t != TK_VAR {
            return err(errors::SYNTAX_ERROR);
        }
        let slot = self.rd(p + 2);
        let (len, flags) = (self.code[p + 4], self.code[p + 5]);
        let ty = flags & 3;
        let array = flags & crate::program::var_flags::ARRAY != 0;
        // `token_size` of a TK_VAR.
        self.pc = p + 6 + len as usize;
        let frame = self.frame_stack.len().saturating_sub(1);
        if array && self.peek() == TK_PAR1 {
            self.pc += 2;
            let mut idx = [0i32; 8];
            let mut n = 0;
            loop {
                let v = self.eval_int(hw)?;
                if n < 8 {
                    idx[n] = v;
                }
                n += 1;
                match self.next_token() {
                    TK_COMMA => continue,
                    TK_PAR2 => break,
                    _ => return err(errors::SYNTAX_ERROR),
                }
            }
            let loc = VarLoc { slot, index: None, frame };
            let index = match self.var_loc_slot(&loc) {
                Var::Array(a) => a.index(&idx[..n.min(8)]).ok_or(Exc::Error(errors::ILLEGAL_FUNCTION_CALL))?,
                _ => return err(errors::NON_DIMENSIONED_ARRAY),
            };
            return Ok((VarLoc { slot, index: Some(index), frame }, ty));
        }
        Ok((VarLoc { slot, index: None, frame }, ty))
    }

    /// Reads the array variable token at pc (`A(...)`) and returns its slot
    /// without indexing (indices are skipped).
    pub fn array_ref(&mut self, hw: &mut dyn Host) -> R<(VarLoc, u8)> {
        let p = self.pc;
        if self.rd(p) != TK_VAR {
            return err(errors::SYNTAX_ERROR);
        }
        let slot = self.rd(p + 2);
        let ty = self.var_type_at(p);
        self.pc = self.skip_token(p);
        if self.peek() == TK_PAR1 {
            self.pc += 2;
            loop {
                self.eval(hw)?;
                match self.next_token() {
                    TK_COMMA => continue,
                    TK_PAR2 => break,
                    _ => return err(errors::SYNTAX_ERROR),
                }
            }
        }
        let frame = self.frame_stack.len().saturating_sub(1);
        Ok((VarLoc { slot, index: None, frame }, ty))
    }

    pub fn array_mut(&mut self, loc: &VarLoc) -> R<&mut Array> {
        match self.var_loc_slot(loc) {
            Var::Array(a) => Ok(a),
            _ => err(errors::NON_DIMENSIONED_ARRAY),
        }
    }

    // ------------------------------------------------------------------
    // Main loop
    // ------------------------------------------------------------------

    /// Positions pc at the next instruction: skips `:` and line ends.
    /// Returns false at the end of the program.
    fn fetch(&mut self) -> bool {
        loop {
            let t = self.peek();
            if t == TK_DP {
                self.pc += 2;
                continue;
            }
            if t == TK_EOL {
                let next = self.pc + 2;
                if next + 1 >= self.code.len() || self.code[next] == 0 {
                    return false;
                }
                self.pc = next + 2;
                continue;
            }
            return true;
        }
    }

    /// Runs the program until it waits, ends, or `max_instructions` have
    /// been executed. `hw` is the rest of the machine.
    pub fn run(&mut self, hw: &mut dyn Host, max_instructions: usize) -> RunState {
        if !self.running {
            return RunState::Idle;
        }
        let mut count = 0;
        loop {
            if count >= max_instructions {
                return RunState::Running;
            }
            count += 1;
            if !self.fetch() {
                return self.stop(Exc::Stop(StopReason::End));
            }
            self.inst_pos = self.pc;
            let result = self.exec_instruction(hw);
            match result {
                Ok(()) | Err(Exc::Jump) => {}
                Err(Exc::Block) => {
                    self.pc = self.inst_pos;
                    // Events (Every, Break) are still handled while waiting.
                    match self.test_point(hw) {
                        Ok(()) | Err(Exc::Block) => return RunState::Running,
                        Err(Exc::Jump) => {
                            // The waiting instruction restarts from scratch
                            // when the handler returns.
                            self.wait = None;
                        }
                        Err(e) => {
                            if let Some(s) = self.handle_error(e) {
                                return s;
                            }
                        }
                    }
                }
                Err(e) => {
                    if let Some(s) = self.handle_error(e) {
                        return s;
                    }
                }
            }
        }
    }

    /// Error handling: On Error / Trap, or stop.
    fn handle_error(&mut self, e: Exc) -> Option<RunState> {
        let n = match &e {
            Exc::Error(n) => *n,
            _ => return Some(self.stop(e)),
        };
        // Trap
        if self.trap_pos == Some(self.inst_pos) {
            self.trap_pos = None;
            self.trap_err = n;
            self.pc = self.skip_statement(self.inst_pos);
            return None;
        }
        let fatal = (errors::is_fatal(n) && n != 11) || self.direct_mode || self.error_on != 0;
        if fatal || self.on_error == OnError::None {
            return Some(self.stop(e));
        }
        self.error_on = n + 1;
        self.error_pos = self.inst_pos;
        self.wait = None;
        match self.on_error {
            OnError::Goto(target) => {
                self.pc = target;
                self.after_jump();
            }
            OnError::Proc(index) => {
                let ret = self.inst_pos;
                if let Err(e) = self.call_proc(index, ret, Vec::new()) {
                    return Some(self.stop(e));
                }
                self.error_proc_depth = Some(self.frame_stack.len());
            }
            OnError::None => {}
        }
        None
    }

    fn stop(&mut self, e: Exc) -> RunState {
        self.running = false;
        self.wait = None;
        let reason = match e {
            Exc::Error(n) => StopReasonOrError::Error(n),
            Exc::Message(m) => StopReasonOrError::Message(m),
            Exc::Stop(r) => StopReasonOrError::Stop(r),
            Exc::Test(t) => StopReasonOrError::Test(t.code),
            Exc::Block | Exc::Jump => StopReasonOrError::Stop(StopReason::End),
        };
        RunState::Stopped(StopInfo { reason, pos: self.inst_pos })
    }

    /// Position after the statement starting at `p` (used by Trap and
    /// Resume Next): stops after `:`, end of line, Then or Else.
    pub fn skip_statement(&self, mut p: usize) -> usize {
        loop {
            let t = self.rd(p);
            match t {
                TK_DP => return p + 2,
                TK_EOL => return p,
                TK_ELSE => return p + 4,
                _ if t == tk::THEN => return p + 2,
                _ => p = self.skip_token(p),
            }
        }
    }

    /// Called by loop and jump instructions: handles the events that the
    /// original checks once per VBL (`Test_Normal`).
    pub fn test_point(&mut self, hw: &mut dyn Host) -> R<()> {
        if !self.vbl_pending {
            return Ok(());
        }
        self.vbl_pending = false;
        hw.test_point(self)?;
        if hw.take_break() {
            if self.events.break_on {
                return Err(Exc::Stop(StopReason::Break));
            }
            if let Some(p) = self.events.on_break_proc {
                let ret = self.inst_pos;
                self.call_proc(p, ret, Vec::new())?;
                return Err(Exc::Jump);
            }
        }
        if self.events.every_on && self.events.every_count <= 0 {
            self.events.every_count = self.events.every_reload;
            self.events.every_on = false;
            let ret = self.inst_pos;
            match self.events.every_target {
                Some(EveryTarget::Gosub(t)) => {
                    self.push_ctl(Ctl::Gosub { ret })?;
                    self.pc = t;
                }
                Some(EveryTarget::Proc(p)) => {
                    self.call_proc(p, ret, Vec::new())?;
                }
                None => return Ok(()),
            }
            self.wait = None;
            // Continue at the handler.
            return Err(Exc::Jump);
        }
        Ok(())
    }

    /// Called at each 50 Hz vertical blank.
    pub fn vbl(&mut self) {
        self.vbl_count += 1;
        self.vbl_pending = true;
        self.events.every_count = self.events.every_count.wrapping_sub(1);
    }

    // ------------------------------------------------------------------
    // Procedures
    // ------------------------------------------------------------------

    pub fn call_proc(&mut self, index: usize, ret: usize, mut args: Vec<Value>) -> R<()> {
        self.enter_proc(index, ret, &mut args)
    }

    /// `call_proc` taking the arguments out of `args` (left empty, its
    /// allocation kept by the caller). The frame comes from the pool of
    /// frames of returned procedures.
    pub(crate) fn enter_proc(&mut self, index: usize, ret: usize, args: &mut Vec<Value>) -> R<()> {
        // (A reference to the program: the procedure table is borrowed while
        // the interpreter changes.)
        let prg = self.prg.clone().expect("no program");
        let p = &prg.procs[index];
        if p.machine_code {
            return Err(Exc::Message("Machine code procedures are not supported".into()));
        }
        let mut frame = self.frame_pool.pop().unwrap_or_default();
        frame.locals.resize(p.locals.len(), Var::Unset);
        for ((slot, ty), v) in p.params.iter().zip(p.param_types.iter()).zip(args.drain(..)) {
            let v = match self.convert_for(*ty, v) {
                Ok(v) => v,
                Err(e) => {
                    frame.locals.clear();
                    self.frame_pool.push(frame);
                    return Err(e);
                }
            };
            if slot & GLOBAL != 0 {
                self.globals[(slot & !GLOBAL) as usize] = Var::Scalar(v);
            } else {
                frame.locals[*slot as usize] = Var::Scalar(v);
            }
        }
        let body = p.body;
        frame.proc_index = index;
        frame.ret = ret;
        frame.data = self.data;
        frame.on_error = self.on_error;
        frame.error_on = self.error_on;
        frame.error_pos = self.error_pos;
        frame.scope = self.scope;
        self.push_ctl(Ctl::Proc(frame))?;
        self.frame_stack.push(self.ctl.len() - 1);
        self.scope = index + 1;
        self.data = DataPtr { base: body, line: 0, item: 0 };
        self.on_error = OnError::None;
        self.pc = body;
        Ok(())
    }

    /// Returns from the current procedure (End Proc / Pop Proc).
    pub fn return_proc(&mut self) -> R<()> {
        // Drop loops and gosubs of the procedure.
        while let Some(c) = self.ctl.last() {
            if matches!(c, Ctl::Proc(_)) {
                break;
            }
            self.pop_ctl();
        }
        let Some(Ctl::Proc(f)) = self.pop_ctl() else { return err(errors::ILLEGAL_FUNCTION_CALL) };
        if self.error_proc_depth == Some(self.frame_stack.len()) {
            self.error_proc_depth = None;
        }
        self.frame_stack.pop();
        self.data = f.data;
        self.on_error = f.on_error;
        self.error_on = f.error_on;
        self.error_pos = f.error_pos;
        self.scope = f.scope;
        self.pc = f.ret;
        // The frame is kept for the next call (its variables are dropped
        // now, as before).
        let mut f = f;
        f.locals.clear();
        if self.frame_pool.len() < 64 {
            self.frame_pool.push(f);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Numbers
    // ------------------------------------------------------------------

    /// Rounds a float to the current precision.
    pub fn round_float(&self, x: f64) -> f64 {
        if self.double { x } else { crate::ffp::Ffp::from_f64(x).to_f64() }
    }

    pub fn int_to_float(&self, i: i32) -> f64 {
        if self.double { i as f64 } else { crate::ffp::Ffp::from_i32(i).to_f64() }
    }
}

/// The rest of the machine as seen by the interpreter: screens, sound,
/// input, files... Implemented by [`crate::machine::Machine`].
pub trait Host {
    /// Executes an instruction token not handled by the interpreter core.
    /// `kw` is the keyword; the interpreter's pc is after the token.
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()>;
    /// Evaluates a function not handled by the interpreter core.
    fn function(&mut self, it: &mut Interp, kw: Keyword) -> R<Value>;
    /// Assignment to a reserved variable (`X Mouse=...`).
    fn reserved_assign(&mut self, it: &mut Interp, kw: Keyword) -> R<()>;
    /// Handles screen updates and menus at a test point.
    fn test_point(&mut self, it: &mut Interp) -> R<()>;
    /// True once after Control-C was pressed.
    fn take_break(&mut self) -> bool;
    /// Text output (Print) to the current window.
    fn print(&mut self, it: &mut Interp, text: &[u8]) -> R<()>;
    /// Line editing for Input: returns the line once Return was pressed.
    fn read_line(&mut self, it: &mut Interp, state: &mut stmt::InputState) -> R<Option<Vec<u8>>>;
}

#[cfg(test)]
mod tests;
