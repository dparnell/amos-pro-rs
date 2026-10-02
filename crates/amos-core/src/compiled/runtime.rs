//! Implementation of the imports of a compiled module.
//!
//! Every function takes the machine (`env`) and/or the module's linear
//! memory (`mem`, starting at the module's base address) as needed, so the
//! same code serves the wasmtime host and the web host.
//!
//! Functions returning a *status* use: [`ST_CONTINUE`] (go on with the next
//! instruction), [`ST_YIELD`] (return from `run`, the program waits;
//! `Interp::pc` holds the resume position), [`ST_STOP`] (the program
//! stopped, see [`Runtime::take_stop`]) or a point number `>= 0` (continue
//! at that instruction; the number of instructions means the end of the
//! program).
//!
//! Value functions (strings, maths, array access, function calls) cannot
//! return a status: on error they record a pending exception and set the
//! `ERR` word of the memory; the module checks it and calls
//! [`Runtime::raise`].

use std::collections::HashMap;
use std::rc::Rc;

use super::layout::{self, Layout};
use super::structure::{self, Call, Instr};
use crate::errors;
use crate::ffp::Ffp;
use crate::interp::value::{AStr, Value, Var, astr, float_to_int};
use crate::interp::verify::{Compiled, GLOBAL};
use crate::interp::{
    Ctl, DataPtr, Exc, Host, Interp, OnError, ProcFrame, R, StopInfo, StopReason, StopReasonOrError, VarLoc,
};
use crate::tokens::*;

mod arrays;
mod strings;
pub use arrays::HeapKind;

pub const ST_CONTINUE: i32 = -1;
pub const ST_YIELD: i32 = -2;
pub const ST_STOP: i32 = -3;
/// `Runtime::dim` needs more memory (`Runtime::grow_bytes`): the host grows
/// the module memory and calls it again.
pub const ST_GROW: i32 = -4;

/// The machine a compiled program runs on: the interpreter state (control
/// stack, events, arrays...) and the rest of the machine.
pub trait Env {
    fn parts(&mut self) -> (&mut Interp, &mut dyn Host);
}

impl Env for crate::machine::Machine {
    fn parts(&mut self) -> (&mut Interp, &mut dyn Host) {
        (&mut self.interp, &mut self.hw)
    }
}

// ----------------------------------------------------------------------
// Memory access
// ----------------------------------------------------------------------

#[inline]
pub fn ld_i32(mem: &[u8], a: u32) -> i32 {
    let a = a as usize;
    mem.get(a..a + 4).map_or(0, |b| i32::from_le_bytes(b.try_into().unwrap()))
}

#[inline]
pub fn st_i32(mem: &mut [u8], a: u32, v: i32) {
    let a = a as usize;
    if let Some(b) = mem.get_mut(a..a + 4) {
        b.copy_from_slice(&v.to_le_bytes());
    }
}

#[inline]
pub fn ld_f64(mem: &[u8], a: u32) -> f64 {
    let a = a as usize;
    mem.get(a..a + 8).map_or(0.0, |b| f64::from_le_bytes(b.try_into().unwrap()))
}

#[inline]
pub fn st_f64(mem: &mut [u8], a: u32, v: f64) {
    let a = a as usize;
    if let Some(b) = mem.get_mut(a..a + 8) {
        b.copy_from_slice(&v.to_le_bytes());
    }
}

/// Runtime state of one compiled program.
pub struct Runtime {
    prg: Rc<Compiled>,
    instrs: Vec<Instr>,
    /// Point of each even code position (`structure::normalize` then the
    /// instruction index; the number of instructions for the end, -1 for a
    /// position inside an instruction).
    point_at: Vec<i32>,
    pub layout: Layout,
    /// Address of the module's memory region (added to `FP`).
    base: u32,
    double: bool,
    end_pos: usize,
    /// Strings in linear memory.
    strings: strings::Strings,
    gc_wanted: bool,
    /// Error to report when the module traps (allocation failure).
    trap_error: Option<u16>,
    /// Parameters of the next call (procedure or keyword).
    stack: Vec<Value>,
    print_buf: Vec<u8>,
    pending: Option<Exc>,
    /// Control stack indices of the procedure frames copied to memory.
    frames: Vec<usize>,
    /// Scalars kept in the interpreter (`structure::resident_vars`).
    resident: Vec<(usize, u16)>,
    vars: HashMap<usize, Rc<[(u16, u8)]>>,
    calls: HashMap<usize, Option<Rc<Call>>>,
    bridge_buf: Vec<u8>,
    stopped: Option<StopInfo>,
    /// Procedure frames of returned calls, reused (no allocation per call:
    /// the boxes go back into `Ctl::Proc` as they are).
    #[allow(clippy::vec_box)]
    frame_pool: Vec<Box<ProcFrame>>,
    /// Arrays in linear memory.
    heap: arrays::Heap,
}

impl Runtime {
    /// Runtime for the verified program `prg` (the one the interpreter was
    /// started with); `base` is the address of the module's memory region,
    /// `heap` where arrays are allocated.
    pub fn new(prg: Rc<Compiled>, base: u32, heap: HeapKind) -> Runtime {
        let instrs = structure::instructions(&prg);
        let layout = Layout::new(&prg);
        let end_pos = structure::end_position(&prg.code);
        let resident = structure::resident_vars(&prg, &instrs);
        let point_at = point_table(&prg.code, &instrs);
        let consts = structure::string_constants(&prg);
        Runtime {
            point_at,
            resident,
            double: prg.double,
            prg,
            instrs,
            layout,
            base,
            end_pos,
            strings: strings::Strings::new(consts),
            gc_wanted: false,
            trap_error: None,
            stack: Vec::new(),
            print_buf: Vec::new(),
            pending: None,
            frames: Vec::new(),
            vars: HashMap::new(),
            calls: HashMap::new(),
            bridge_buf: Vec::new(),
            stopped: None,
            frame_pool: Vec::new(),
            heap: arrays::Heap::new(heap, layout.size),
        }
    }

    /// Checks that a module was compiled for this runtime and program.
    pub fn check_module(&self, abi: i32, hash: u32) -> Result<(), String> {
        if abi != super::ABI_VERSION {
            return Err(format!("compiled module interface {abi}, runtime {}", super::ABI_VERSION));
        }
        if hash != structure::code_hash(&self.prg.code) {
            return Err("compiled module does not match the program".into());
        }
        Ok(())
    }

    pub fn program(&self) -> &Rc<Compiled> {
        &self.prg
    }

    pub fn instructions(&self) -> &[Instr] {
        &self.instrs
    }

    /// Why the program stopped (after a `run` returned stopped).
    pub fn take_stop(&mut self) -> Option<StopInfo> {
        self.stopped.take()
    }

    /// Error to report after the module trapped (allocation failure).
    pub fn take_trap_error(&mut self) -> Option<u16> {
        self.trap_error.take()
    }

    // ------------------------------------------------------------------
    // Variables in memory
    // ------------------------------------------------------------------

    /// Address of scalar `slot` with `depth` procedure frames active (the
    /// resolution of `Interp::var_slot`).
    fn scalar_addr(&self, slot: u16, depth: usize) -> u32 {
        if slot & GLOBAL != 0 {
            self.layout.globals + (slot & !GLOBAL) as u32 * 8
        } else {
            self.layout.frame_base(depth) + slot as u32 * 8
        }
    }

    fn mem_value(&self, mem: &mut [u8], a: u32, ty: u8) -> Value {
        match ty {
            1 => Value::Float(ld_f64(mem, a)),
            2 => Value::Str(self.str_of(mem, ld_i32(mem, a))),
            _ => Value::Int(ld_i32(mem, a)),
        }
    }

    fn set_mem_value(&mut self, mem: &mut [u8], a: u32, ty: u8, v: &Value) {
        match (ty, v) {
            (0, Value::Int(i)) => st_i32(mem, a, *i),
            (0, Value::Float(f)) => st_i32(mem, a, float_to_int(*f)),
            (1, Value::Float(f)) => st_f64(mem, a, *f),
            (1, Value::Int(i)) => st_f64(mem, a, *i as f64),
            (2, Value::Str(s)) => {
                let cur = ld_i32(mem, a);
                if self.str_bytes(mem, cur) == &s[..] {
                    return;
                }
                let h = self.alloc(mem, s.clone());
                st_i32(mem, a, h);
            }
            _ => {}
        }
    }

    /// Address and type of the element `loc` (with an index) of an array in
    /// linear memory; `None` if the array is kept in the interpreter.
    fn linear_elem(&self, it: &mut Interp, mem: &mut [u8], loc: &VarLoc) -> Option<(u32, u8)> {
        let flat = loc.index? as u32;
        let base_loc = VarLoc { index: None, ..*loc };
        if matches!(it.var_loc_slot(&base_loc), Var::Array(_)) {
            return None;
        }
        let desc = ld_i32(mem, self.loc_addr(it, &base_loc)) as u32;
        let (ty, count) = self.array_block(desc)?;
        (flat < count).then(|| (desc + layout::ARR_DATA + flat * layout::elem_size(ty), ty))
    }

    fn is_resident(&self, scope: usize, slot: u16) -> bool {
        !self.resident.is_empty() && self.resident.binary_search(&structure::var_key(scope, slot)).is_ok()
    }

    /// True if the For loop variable `loc` (scalar) is kept in the
    /// interpreter.
    fn loc_resident(&self, it: &Interp, loc: &VarLoc) -> bool {
        if self.resident.is_empty() {
            return false;
        }
        if loc.slot & GLOBAL != 0 || it.frame_stack.is_empty() {
            return self.is_resident(0, loc.slot);
        }
        let f = loc.frame.min(it.frame_stack.len() - 1);
        match it.ctl.get(it.frame_stack[f]) {
            Some(Ctl::Proc(pf)) => self.is_resident(pf.proc_index + 1, loc.slot),
            _ => false,
        }
    }

    /// Address of the variable of a For loop (scalar), as
    /// `Interp::var_loc_slot` resolves it.
    fn loc_addr(&self, it: &Interp, loc: &VarLoc) -> u32 {
        if loc.slot & GLOBAL != 0 || it.frame_stack.is_empty() {
            return self.scalar_addr(loc.slot, it.frame_stack.len());
        }
        let f = loc.frame.min(it.frame_stack.len() - 1);
        self.layout.frame_base(f + 1) + loc.slot as u32 * 8
    }

    fn loc_type(&self, it: &Interp, loc: &VarLoc) -> u8 {
        let d = if loc.slot & GLOBAL != 0 {
            self.prg.globals.get((loc.slot & !GLOBAL) as usize)
        } else if it.frame_stack.is_empty() {
            self.prg.globals.get(loc.slot as usize)
        } else {
            let f = loc.frame.min(it.frame_stack.len() - 1);
            match it.ctl.get(it.frame_stack[f]) {
                Some(Ctl::Proc(pf)) => self.prg.procs[pf.proc_index].locals.get(loc.slot as usize),
                _ => None,
            }
        };
        d.map_or(0, |d| d.ty)
    }

    /// Brings the memory up to date with the interpreter after an
    /// operation that may have called or returned from procedures: new
    /// frames get their locals (parameters) from the interpreter, and the
    /// header words are refreshed.
    fn sync(&mut self, it: &Interp, mem: &mut [u8]) {
        let fs = &it.frame_stack;
        if self.frames.len() != fs.len() || self.frames.last() != fs.last() {
            self.sync_frames(it, mem);
        }
        st_i32(mem, layout::FP, (self.base + self.layout.frame_base(fs.len())) as i32);
        st_i32(mem, layout::SCOPE, it.scope as i32);
        st_i32(mem, layout::ATT, (it.vbl_pending || self.gc_wanted) as i32);
        st_i32(mem, layout::DEPTH, fs.len() as i32);
        st_i32(mem, layout::PARAM_E, it.param_e);
        st_f64(mem, layout::PARAM_F, it.param_f);
        st_i32(mem, layout::FIX_FLG, it.fix.fix_flg() as i32);
        st_i32(mem, layout::EXP_FLG, it.fix.exp_flg() as i32);
        self.mirror(it, mem);
    }

    /// Refreshes the mirror of the top of the control stack.
    fn mirror(&mut self, it: &Interp, mem: &mut [u8]) {
        let (mut addr, mut step, mut limit, mut body_point) = (0, 0, 0, 0);
        let (mut lo, mut hi) = (0, i32::MAX);
        let (mut kind, mut start_pos, mut start_point) = (layout::TOP_OTHER, 0, 0);
        match it.ctl.last() {
            Some(Ctl::For { var, step: s, limit: l, body, exit }) => {
                lo = *body as i32;
                hi = *exit as i32;
                if var.index.is_none() && !self.loc_resident(it, var) && self.loc_type(it, var) == 0 {
                    let bp = self.point_quiet(*body);
                    if bp >= 0 {
                        addr = (self.base + self.loc_addr(it, var)) as i32;
                        step = *s;
                        limit = *l;
                        body_point = bp;
                        kind = layout::TOP_FOR;
                    }
                }
            }
            Some(Ctl::Repeat { body, exit } | Ctl::Do { body, exit } | Ctl::While { body, exit, .. }) => {
                lo = *body as i32;
                hi = *exit as i32;
                let bp = self.point_quiet(*body);
                if bp >= 0 {
                    body_point = bp;
                    kind = match it.ctl.last() {
                        Some(Ctl::Repeat { .. }) => layout::TOP_REPEAT,
                        Some(Ctl::Do { .. }) => layout::TOP_DO,
                        _ => layout::TOP_WHILE,
                    };
                }
                if let Some(Ctl::While { start, .. }) = it.ctl.last() {
                    let sp = self.point_quiet(*start);
                    if sp < 0 {
                        kind = layout::TOP_OTHER;
                    }
                    start_pos = *start as i32;
                    start_point = sp;
                }
            }
            _ => {}
        }
        st_i32(mem, layout::TOP_KIND, kind);
        st_i32(mem, layout::TOP_START, start_pos);
        st_i32(mem, layout::TOP_START_POINT, start_point);
        st_i32(mem, layout::FOR_ADDR, addr);
        st_i32(mem, layout::FOR_STEP, step);
        st_i32(mem, layout::FOR_LIMIT, limit);
        st_i32(mem, layout::FOR_BODY, body_point);
        st_i32(mem, layout::LOOP_LO, lo);
        st_i32(mem, layout::LOOP_HI, hi);
    }

    fn sync_frames(&mut self, it: &Interp, mem: &mut [u8]) {
        let fs = &it.frame_stack;
        let mut d = 0;
        while d < self.frames.len() && d < fs.len() && self.frames[d] == fs[d] {
            d += 1;
        }
        if d < self.frames.len() {
            self.free_arrays_above(d);
        }
        self.frames.truncate(d);
        let prg = self.prg.clone();
        while self.frames.len() < fs.len() {
            let k = self.frames.len();
            let idx = fs[k];
            let base = self.layout.frame_base(k + 1);
            let size = self.layout.frame_size;
            if let Some(b) = mem.get_mut(base as usize..(base + size) as usize) {
                b.fill(0);
            }
            if let Some(Ctl::Proc(f)) = it.ctl.get(idx) {
                for (i, d) in prg.procs[f.proc_index].locals.iter().enumerate() {
                    if !structure::is_scalar(d) || self.is_resident(f.proc_index + 1, i as u16) {
                        continue;
                    }
                    if let Some(Var::Scalar(v)) = f.locals.get(i) {
                        self.set_mem_value(mem, base + i as u32 * 8, d.ty, v);
                    }
                }
            }
            self.frames.push(idx);
        }
    }

    // ------------------------------------------------------------------
    // Positions, errors and stops
    // ------------------------------------------------------------------

    /// Point number of the instruction at (or following) `pos`.
    fn point(&mut self, it: &mut Interp, pos: usize) -> i32 {
        let p = self.point_quiet(pos);
        if p >= 0 {
            return p;
        }
        self.stop(it, Exc::Message(format!("Compiled program: no instruction at position {pos}")))
    }

    /// Point of `pos`, -1 if it is not at an instruction.
    fn point_quiet(&self, pos: usize) -> i32 {
        if pos & 1 == 0
            && let Some(&p) = self.point_at.get(pos / 2)
        {
            return p;
        }
        match structure::normalize(&self.prg.code, pos) {
            None => self.instrs.len() as i32,
            Some(p) => self.instrs.binary_search_by_key(&p, |i| i.pos).map_or(-1, |i| i as i32),
        }
    }

    fn stop(&mut self, it: &mut Interp, e: Exc) -> i32 {
        it.running = false;
        it.wait = None;
        let reason = match e {
            Exc::Error(n) => StopReasonOrError::Error(n),
            Exc::Message(m) => StopReasonOrError::Message(m),
            Exc::Stop(r) => StopReasonOrError::Stop(r),
            Exc::Test(t) => StopReasonOrError::Test(t.code),
            Exc::Block | Exc::Jump => StopReasonOrError::Stop(StopReason::End),
        };
        self.stopped = Some(StopInfo { reason, pos: it.inst_pos });
        ST_STOP
    }

    /// `Interp::handle_error`: Trap, On Error Goto / Proc, or stop.
    fn handle_error(&mut self, it: &mut Interp, e: Exc) -> i32 {
        let n = match &e {
            Exc::Error(n) => *n,
            _ => return self.stop(it, e),
        };
        if it.trap_pos == Some(it.inst_pos) {
            it.trap_pos = None;
            it.trap_err = n;
            it.pc = it.skip_statement(it.inst_pos);
            return self.point(it, it.pc);
        }
        let fatal = (errors::is_fatal(n) && n != 11) || it.direct_mode || it.error_on != 0;
        if fatal || it.on_error == OnError::None {
            return self.stop(it, e);
        }
        it.error_on = n + 1;
        it.error_pos = it.inst_pos;
        it.wait = None;
        match it.on_error {
            OnError::Goto(target) => {
                it.pc = target;
                it.after_jump();
            }
            OnError::Proc(index) => {
                let ret = it.inst_pos;
                if let Err(e) = it.call_proc(index, ret, Vec::new()) {
                    return self.stop(it, e);
                }
                it.error_proc_depth = Some(it.frame_stack.len());
            }
            OnError::None => {}
        }
        self.point(it, it.pc)
    }

    /// What `Interp::run` does with an exception raised by the instruction
    /// at `pos`.
    fn exc_status(&mut self, it: &mut Interp, hw: &mut dyn Host, e: Exc, pos: usize) -> i32 {
        self.stack.clear();
        it.inst_pos = pos;
        match e {
            Exc::Jump => self.point(it, it.pc),
            Exc::Block => {
                it.pc = pos;
                // Events are still handled while waiting.
                match it.test_point(hw) {
                    Ok(()) | Err(Exc::Block) => {
                        it.pc = pos;
                        ST_YIELD
                    }
                    Err(Exc::Jump) => {
                        // The waiting instruction restarts when the handler
                        // returns.
                        it.wait = None;
                        self.point(it, it.pc)
                    }
                    Err(e) => self.handle_error(it, e),
                }
            }
            e => self.handle_error(it, e),
        }
    }

    fn result(&mut self, it: &mut Interp, hw: &mut dyn Host, mem: &mut [u8], r: R<i32>, pos: usize) -> i32 {
        let st = match r {
            Ok(st) => st,
            Err(e) => self.exc_status(it, hw, e, pos),
        };
        self.sync(it, mem);
        st
    }

    fn set_pending(&mut self, mem: &mut [u8], e: Exc) {
        self.pending = Some(e);
        st_i32(mem, layout::ERR, 1);
    }

    fn test(&mut self, it: &mut Interp, hw: &mut dyn Host, mem: &mut [u8], pos: usize) -> R<()> {
        it.inst_pos = pos;
        if self.gc_wanted {
            self.gc(it, mem);
        }
        it.test_point(hw)
    }

    // ------------------------------------------------------------------
    // Entry, exit
    // ------------------------------------------------------------------

    /// Start of `run`: returns the point to continue at, or `ST_STOP`.
    pub fn enter(&mut self, env: &mut dyn Env, mem: &mut [u8]) -> i32 {
        let (it, _) = env.parts();
        if !it.running {
            return ST_STOP;
        }
        self.stack.clear();
        self.pending = None;
        st_i32(mem, layout::ERR, 0);
        Self::settle(it, mem);
        self.sync(it, mem);
        if self.gc_wanted {
            self.gc(it, mem);
        }
        let pc = it.pc;
        self.point(it, pc)
    }

    /// The time budget of `run` is used: the program continues at `point`
    /// next time.
    pub fn suspend(&mut self, env: &mut dyn Env, mem: &mut [u8], point: i32) {
        let (it, _) = env.parts();
        Self::settle(it, mem);
        it.pc = self.instrs.get(point as usize).map_or(self.end_pos, |i| i.pos);
    }

    /// Does the pop of a While entry a `Wend` of the module left for its
    /// While instruction (`layout::LAZY_WHILE`), when something else comes
    /// first: the interpreter's state is then exactly as after Wend.
    fn settle(it: &mut Interp, mem: &mut [u8]) {
        if ld_i32(mem, layout::LAZY_WHILE) != 0 {
            st_i32(mem, layout::LAZY_WHILE, 0);
            if matches!(it.ctl.last(), Some(Ctl::While { .. })) {
                it.pop_ctl();
            }
        }
    }

    /// While whose entry was left on the stack by the module's Wend, with a
    /// false condition: pops it and continues at `exit`.
    pub fn while_end(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, exit: i32) -> i32 {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        Self::settle(it, mem);
        it.pc = exit as usize;
        let r = Ok(self.point(it, exit as usize));
        self.result(it, hw, mem, r, pos as usize)
    }

    /// End of the program reached; `last` is the position of the last
    /// instruction executed (-1 if unknown).
    pub fn end_program(&mut self, env: &mut dyn Env, last: i32) -> i32 {
        let (it, _) = env.parts();
        if last >= 0 {
            it.inst_pos = last as usize;
        }
        self.stop(it, Exc::Stop(StopReason::End))
    }

    /// Raises the error `code` (> 0) or the pending exception (`code` 0) for
    /// the instruction at `pos`.
    pub fn raise(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, code: i32) -> i32 {
        let (it, hw) = env.parts();
        Self::settle(it, mem);
        st_i32(mem, layout::ERR, 0);
        let e = if code > 0 {
            Exc::Error(code as u16)
        } else {
            self.pending.take().unwrap_or_else(|| Exc::Message("Compiled program: no pending error".into()))
        };
        let st = self.exc_status(it, hw, e, pos as usize);
        self.sync(it, mem);
        st
    }

    /// Test point (`Interp::test_point`) of the instruction at `pos`.
    pub fn test_point(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = self.test(it, hw, mem, pos).map(|_| ST_CONTINUE);
        self.result(it, hw, mem, r, pos)
    }

    /// Runs the instruction at `pos` with the interpreter, its scalar
    /// variables copied from and back to memory.
    pub fn interp(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let vars = match self.vars.get(&pos) {
            Some(v) => v.clone(),
            None => {
                let v: Rc<[(u16, u8)]> = match self.instrs.binary_search_by_key(&pos, |i| i.pos) {
                    Ok(i) => structure::fallback_vars(&self.prg, &self.instrs, &self.resident, i).into(),
                    Err(_) => Rc::from(Vec::new()),
                };
                self.vars.insert(pos, v.clone());
                v
            }
        };
        let depth = it.frame_stack.len();
        let frame = it.frame_stack.last().copied();
        for &(slot, ty) in vars.iter() {
            let v = self.mem_value(mem, self.scalar_addr(slot, depth), ty);
            *it.var_slot(slot) = Var::Scalar(v);
        }
        it.pc = pos;
        let state = it.run(hw, 1);
        // Copy the variables back (locals only if their frame still exists).
        let same_frame = depth > 0 && it.frame_stack.get(depth - 1).copied() == frame;
        for &(slot, ty) in vars.iter() {
            let var = if slot & GLOBAL != 0 || depth == 0 {
                it.globals.get((slot & !GLOBAL) as usize)
            } else if same_frame {
                match it.ctl.get(it.frame_stack[depth - 1]) {
                    Some(Ctl::Proc(f)) => f.locals.get(slot as usize),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(Var::Scalar(v)) = var {
                let v = v.clone();
                self.set_mem_value(mem, self.scalar_addr(slot, depth), ty, &v);
            }
        }
        self.sync(it, mem);
        match state {
            crate::interp::RunState::Stopped(info) => {
                self.stopped = Some(info);
                ST_STOP
            }
            crate::interp::RunState::Idle => ST_STOP,
            crate::interp::RunState::Running => {
                if it.pc == pos {
                    // Waiting (Exc::Block): executed again later.
                    ST_YIELD
                } else {
                    let pc = it.pc;
                    self.point(it, pc)
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Keyword bridge
    // ------------------------------------------------------------------

    fn call_at(&mut self, p: usize, function: bool) -> Option<Rc<Call>> {
        if let Some(c) = self.calls.get(&p) {
            return c.clone();
        }
        let c = if function {
            structure::function_call(&self.prg.code, p)
        } else {
            structure::instruction_call(&self.prg.code, p)
        }
        .map(Rc::new);
        self.calls.insert(p, c.clone());
        c
    }

    /// Builds the token stream of the call at `p` with the parameter values
    /// on the stack as constants. Returns it and the position of its end.
    fn bridge_code(&mut self, p: usize, call: &Call) -> (Vec<u8>, usize) {
        let n = call.slots.iter().filter(|s| s.present).count();
        let values = self.stack.split_off(self.stack.len().saturating_sub(n));
        let mut buf = std::mem::take(&mut self.bridge_buf);
        buf.clear();
        buf.extend_from_slice(&[0, 0]);
        buf.extend_from_slice(&self.prg.code[p..call.tok_end]);
        let w = |buf: &mut Vec<u8>, t: u16| buf.extend_from_slice(&t.to_be_bytes());
        if call.paren {
            w(&mut buf, TK_PAR1);
        }
        let mut vi = values.iter();
        for s in &call.slots {
            if s.present {
                match vi.next() {
                    Some(Value::Int(i)) => {
                        w(&mut buf, TK_ENT);
                        buf.extend_from_slice(&(*i as u32).to_be_bytes());
                    }
                    Some(Value::Float(f)) => {
                        w(&mut buf, TK_DFL);
                        buf.extend_from_slice(&f.to_be_bytes());
                    }
                    Some(Value::Str(s)) => {
                        w(&mut buf, TK_CH1);
                        let n = s.len().min(0xFFFF);
                        w(&mut buf, n as u16);
                        buf.extend_from_slice(&s[..n]);
                        if n & 1 != 0 {
                            buf.push(0);
                        }
                    }
                    None => {}
                }
            }
            if s.sep != 0 {
                w(&mut buf, s.sep);
            }
        }
        if call.paren {
            w(&mut buf, TK_PAR2);
        }
        let end = buf.len();
        buf.extend_from_slice(&[0, 0, 0, 0]);
        (buf, end)
    }

    /// Runs `f` with the interpreter reading `code` from `start`; checks
    /// that the parameters were read up to `end`.
    fn with_code<T>(
        &mut self,
        it: &mut Interp,
        hw: &mut dyn Host,
        code: Vec<u8>,
        start: usize,
        end: usize,
        f: impl FnOnce(&mut Interp, &mut dyn Host) -> R<T>,
    ) -> R<T> {
        let saved_code = std::mem::replace(&mut it.code, Rc::new(code));
        let saved_pc = it.pc;
        it.pc = start;
        let r = f(it, hw);
        let consumed = it.pc == end;
        let syn = std::mem::replace(&mut it.code, saved_code);
        it.pc = saved_pc;
        if let Ok(v) = Rc::try_unwrap(syn) {
            self.bridge_buf = v;
        }
        match r {
            Ok(_) if !consumed => Err(Exc::Message("Compiled program: keyword parameters not read as expected".into())),
            r => r,
        }
    }

    /// Instruction of a subsystem at `pos`, parameters on the stack.
    pub fn keyword(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let r = match self.call_at(pos, false) {
            None => Err(Exc::Message("Compiled program: bad keyword call".into())),
            Some(call) => {
                let (code, end) = self.bridge_code(pos, &call);
                let start = 2 + (call.tok_end - pos);
                let kw = call.kw;
                self.with_code(it, hw, code, start, end, |it, hw| hw.instruction(it, kw)).map(|_| ST_CONTINUE)
            }
        };
        self.result(it, hw, mem, r, pos)
    }

    /// Value of the function call at `fpos` in the instruction at `pos`,
    /// parameters on the stack. `None` (and a pending error) on error.
    fn function(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, fpos: i32) -> Option<Value> {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        let fpos = fpos as usize;
        let r = match self.call_at(fpos, true) {
            None => Err(Exc::Message("Compiled program: bad function call".into())),
            Some(call) => {
                let (code, end) = self.bridge_code(fpos, &call);
                self.with_code(it, hw, code, 2, end, |it, hw| it.eval(hw))
            }
        };
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.set_pending(mem, e);
                None
            }
        }
    }

    pub fn fn_i(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, fpos: i32) -> i32 {
        match self.function(env, mem, pos, fpos) {
            Some(Value::Int(i)) => i,
            Some(Value::Float(f)) => float_to_int(f),
            Some(Value::Str(_)) => {
                self.set_pending(mem, Exc::Error(errors::TYPE_MISMATCH));
                0
            }
            None => 0,
        }
    }

    pub fn fn_f(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, fpos: i32) -> f64 {
        match self.function(env, mem, pos, fpos) {
            Some(Value::Float(f)) => f,
            Some(Value::Int(i)) => {
                let (it, _) = env.parts();
                it.int_to_float(i)
            }
            Some(Value::Str(_)) => {
                self.set_pending(mem, Exc::Error(errors::TYPE_MISMATCH));
                0.0
            }
            None => 0.0,
        }
    }

    /// Numeric result of unknown type: the type goes to the `TAG` word.
    pub fn fn_n(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, fpos: i32) -> f64 {
        match self.function(env, mem, pos, fpos) {
            Some(Value::Int(i)) => {
                st_i32(mem, layout::TAG, 0);
                i as f64
            }
            Some(Value::Float(f)) => {
                st_i32(mem, layout::TAG, 1);
                f
            }
            Some(Value::Str(_)) => {
                self.set_pending(mem, Exc::Error(errors::TYPE_MISMATCH));
                0.0
            }
            None => 0.0,
        }
    }

    pub fn fn_s(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, fpos: i32) -> i32 {
        match self.function(env, mem, pos, fpos) {
            Some(Value::Str(s)) => self.alloc(mem, s),
            Some(_) => {
                self.set_pending(mem, Exc::Error(errors::TYPE_MISMATCH));
                0
            }
            None => 0,
        }
    }

    // ------------------------------------------------------------------
    // Parameter stack
    // ------------------------------------------------------------------

    pub fn push_i(&mut self, v: i32) {
        self.stack.push(Value::Int(v));
    }

    pub fn push_f(&mut self, v: f64) {
        self.stack.push(Value::Float(v));
    }

    pub fn push_s(&mut self, mem: &mut [u8], h: i32) {
        let s = self.str_of(mem, h);
        self.stack.push(Value::Str(s));
    }

    pub fn push_n(&mut self, v: f64, tag: i32) {
        self.stack.push(dyn_value(v, tag));
    }

    // ------------------------------------------------------------------
    // Print
    // ------------------------------------------------------------------

    pub fn print_begin(&mut self) {
        self.print_buf.clear();
    }

    pub fn print_i(&mut self, v: i32) {
        self.print_buf.extend(crate::ffp::format_int(v).bytes());
    }

    pub fn print_f(&mut self, env: &mut dyn Env, v: f64) {
        let (it, _) = env.parts();
        self.print_buf.extend(it.format_float(v).bytes());
    }

    pub fn print_n(&mut self, env: &mut dyn Env, v: f64, tag: i32) {
        if tag == 0 { self.print_i(v as i32) } else { self.print_f(env, v) }
    }

    pub fn print_s(&mut self, mem: &mut [u8], h: i32) {
        let s = self.str_bytes(mem, h);
        self.print_buf.extend_from_slice(s);
    }

    pub fn print_tab(&mut self) {
        self.print_buf.push(9);
    }

    pub fn print_end(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, newline: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        if newline != 0 {
            self.print_buf.extend_from_slice(b"\r\n");
        }
        let text = std::mem::take(&mut self.print_buf);
        let r = hw.print(it, &text).map(|_| ST_CONTINUE);
        self.print_buf = text;
        self.result(it, hw, mem, r, pos)
    }

    // ------------------------------------------------------------------
    // Arrays (kept in the interpreter)
    // ------------------------------------------------------------------

    /// Flat index of an element of array `slot` (`Interp::var_ref`); the
    /// first 8 indices are in the `IDX` area, `n` is their number.
    pub fn aref(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, n: i32) -> i32 {
        let mut idx = [0i32; 8];
        for (k, v) in idx.iter_mut().enumerate() {
            *v = ld_i32(mem, layout::IDX + k as u32 * 4);
        }
        let (it, _) = env.parts();
        let n = (n.max(0) as usize).min(9);
        let r = match it.var_slot(slot as u16) {
            Var::Array(a) => a.index(&idx[..n.min(8)]).ok_or(Exc::Error(errors::ILLEGAL_FUNCTION_CALL)),
            _ => Err(Exc::Error(errors::NON_DIMENSIONED_ARRAY)),
        };
        match r {
            Ok(i) => i as i32,
            Err(e) => {
                self.set_pending(mem, e);
                0
            }
        }
    }

    fn aget(&mut self, env: &mut dyn Env, slot: i32, flat: i32, ty: u8) -> Value {
        let (it, _) = env.parts();
        let loc =
            VarLoc { slot: slot as u16, index: Some(flat as usize), frame: it.frame_stack.len().saturating_sub(1) };
        it.read_loc(&loc, ty)
    }

    pub fn aget_i(&mut self, env: &mut dyn Env, slot: i32, flat: i32) -> i32 {
        match self.aget(env, slot, flat, 0) {
            Value::Int(i) => i,
            Value::Float(f) => float_to_int(f),
            _ => 0,
        }
    }

    pub fn aget_f(&mut self, env: &mut dyn Env, slot: i32, flat: i32) -> f64 {
        match self.aget(env, slot, flat, 1) {
            Value::Float(f) => f,
            Value::Int(i) => i as f64,
            _ => 0.0,
        }
    }

    pub fn aget_s(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, flat: i32) -> i32 {
        match self.aget(env, slot, flat, 2) {
            Value::Str(s) => self.alloc(mem, s),
            _ => 0,
        }
    }

    fn aset(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, flat: i32, ty: u8, v: Value) {
        let (it, _) = env.parts();
        let loc =
            VarLoc { slot: slot as u16, index: Some(flat as usize), frame: it.frame_stack.len().saturating_sub(1) };
        if let Err(e) = it.write_loc(&loc, ty, v) {
            self.set_pending(mem, e);
        }
    }

    pub fn aset_i(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, flat: i32, v: i32) {
        self.aset(env, mem, slot, flat, 0, Value::Int(v));
    }

    pub fn aset_f(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, flat: i32, v: f64) {
        self.aset(env, mem, slot, flat, 1, Value::Float(v));
    }

    pub fn aset_s(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, flat: i32, h: i32) {
        let s = self.str_of(mem, h);
        self.aset(env, mem, slot, flat, 2, Value::Str(s));
    }

    // ------------------------------------------------------------------
    // Loops (`interp/flow.rs`)
    // ------------------------------------------------------------------

    /// End of a For statement: pushes the loop. `flat` is the element index
    /// for an array element variable, -1 for a scalar.
    #[allow(clippy::too_many_arguments)]
    pub fn for_push(
        &mut self,
        env: &mut dyn Env,
        mem: &mut [u8],
        pos: i32,
        slot: i32,
        flat: i32,
        limit: i32,
        step: i32,
        body: i32,
        exit: i32,
    ) -> i32 {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        let var = VarLoc {
            slot: slot as u16,
            index: if flat < 0 { None } else { Some(flat as usize) },
            frame: it.frame_stack.len().saturating_sub(1),
        };
        let r =
            it.push_ctl(Ctl::For { var, step, limit, body: body as usize, exit: exit as usize }).map(|_| ST_CONTINUE);
        self.result(it, hw, mem, r, pos as usize)
    }

    pub fn next(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = self.next_inner(it, hw, mem, pos);
        self.result(it, hw, mem, r, pos)
    }

    fn next_inner(&mut self, it: &mut Interp, hw: &mut dyn Host, mem: &mut [u8], pos: usize) -> R<i32> {
        self.test(it, hw, mem, pos)?;
        let Some(Ctl::For { var, step, limit, body, .. }) = it.ctl.last().cloned() else {
            return Err(Exc::Error(errors::SYNTAX_ERROR));
        };
        let v;
        if var.index.is_none() && !self.loc_resident(it, &var) {
            let a = self.loc_addr(it, &var);
            let ty = self.loc_type(it, &var);
            let cur = match ty {
                0 => ld_i32(mem, a),
                1 => float_to_int(ld_f64(mem, a)),
                _ => 0,
            };
            v = cur.wrapping_add(step);
            match ty {
                0 => st_i32(mem, a, v),
                1 => st_f64(mem, a, it.int_to_float(v)),
                _ => return Err(Exc::Error(errors::TYPE_MISMATCH)),
            }
        } else if let Some((a, ty)) = self.linear_elem(it, mem, &var) {
            // Element of an array in linear memory.
            let es = layout::elem_size(ty);
            let b = self.heap.bytes(mem, self.base, a, es);
            let cur = if ty == 1 {
                float_to_int(f64::from_le_bytes(b[..8].try_into().unwrap()))
            } else {
                i32::from_le_bytes(b[..4].try_into().unwrap())
            };
            v = cur.wrapping_add(step);
            if ty == 1 {
                let f = it.int_to_float(v);
                self.heap.bytes(mem, self.base, a, 8).copy_from_slice(&f.to_le_bytes());
            } else {
                self.heap.bytes(mem, self.base, a, 4).copy_from_slice(&v.to_le_bytes());
            }
        } else {
            let val = it.read_loc(&var, 0);
            let cur = match val {
                Value::Int(i) => i,
                Value::Float(f) => float_to_int(f),
                Value::Str(_) => 0,
            };
            v = cur.wrapping_add(step);
            let ty = if matches!(val, Value::Float(_)) { 1 } else { 0 };
            it.write_loc(&var, ty, Value::Int(v))?;
        }
        let done = if step >= 0 { v > limit } else { v < limit };
        if done {
            it.pop_ctl();
            Ok(ST_CONTINUE)
        } else {
            it.pc = body;
            Ok(self.point(it, body))
        }
    }

    /// End of a For loop whose last iteration was done by the module (fast
    /// path of `Next`): pops the loop.
    pub fn next_done(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        it.pop_ctl();
        self.result(it, hw, mem, Ok(ST_CONTINUE), pos as usize)
    }

    /// `Wait n` / `Wait Vbl` (n = 1): waits, then a test point.
    pub fn wait(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, n: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let r = it.wait_vbls(n.max(0) as u64).and_then(|_| {
            it.vbl_pending = true;
            self.test(it, hw, mem, pos)
        });
        let r = r.map(|_| ST_CONTINUE);
        self.result(it, hw, mem, r, pos)
    }

    /// Repeat (`kind` 0) or Do (1).
    pub fn loop_push(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, kind: i32, body: i32, exit: i32) -> i32 {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        let (body, exit) = (body as usize, exit as usize);
        let c = if kind == 1 { Ctl::Do { body, exit } } else { Ctl::Repeat { body, exit } };
        let r = it.push_ctl(c).map(|_| ST_CONTINUE);
        self.result(it, hw, mem, r, pos as usize)
    }

    /// While with a true condition.
    pub fn while_push(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, body: i32, exit: i32) -> i32 {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        let c = Ctl::While { start: pos as usize, body: body as usize, exit: exit as usize };
        let r = it.push_ctl(c).map(|_| ST_CONTINUE);
        self.result(it, hw, mem, r, pos as usize)
    }

    /// Until with its condition (the test point was done before).
    pub fn until(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, cond: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let r = match it.ctl.last() {
            Some(Ctl::Repeat { body, .. }) => {
                let body = *body;
                if cond != 0 {
                    it.pop_ctl();
                    Ok(ST_CONTINUE)
                } else {
                    it.pc = body;
                    Ok(self.point(it, body))
                }
            }
            _ => Err(Exc::Error(errors::SYNTAX_ERROR)),
        };
        self.result(it, hw, mem, r, pos)
    }

    pub fn wend(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = (|| {
            self.test(it, hw, mem, pos)?;
            let Some(Ctl::While { start, .. }) = it.ctl.last().cloned() else {
                return Err(Exc::Error(errors::SYNTAX_ERROR));
            };
            it.pop_ctl();
            it.pc = start;
            Ok(self.point(it, start))
        })();
        self.result(it, hw, mem, r, pos)
    }

    pub fn loop_end(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = (|| {
            self.test(it, hw, mem, pos)?;
            let Some(Ctl::Do { body, .. }) = it.ctl.last().cloned() else {
                return Err(Exc::Error(errors::SYNTAX_ERROR));
            };
            it.pc = body;
            Ok(self.point(it, body))
        })();
        self.result(it, hw, mem, r, pos)
    }

    /// Exit / Exit If (condition true): pops `frames` loops, jumps.
    pub fn exit(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, frames: i32, target: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = (|| {
            self.test(it, hw, mem, pos)?;
            for _ in 0..frames {
                it.pop_ctl();
            }
            it.pc = target as usize;
            Ok(self.point(it, target as usize))
        })();
        self.result(it, hw, mem, r, pos)
    }

    // ------------------------------------------------------------------
    // Jumps
    // ------------------------------------------------------------------

    /// Jump to `target` dropping the loops it leaves (`Interp::after_jump`),
    /// e.g. a false If.
    pub fn goto_pos(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, target: i32) -> i32 {
        let (it, hw) = env.parts();
        it.inst_pos = pos as usize;
        it.pc = target as usize;
        it.after_jump();
        let r = Ok(self.point(it, target as usize));
        self.result(it, hw, mem, r, pos as usize)
    }

    fn label(&self, it: &Interp, idx: i32) -> R<usize> {
        self.prg
            .scopes
            .get(it.scope)
            .and_then(|s| s.labels.get(idx as usize))
            .map(|l| l.target)
            .ok_or(Exc::Error(errors::LABEL_NOT_DEFINED))
    }

    /// Goto label (also If ... Then label / Else label): test point, jump.
    pub fn goto_label(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, idx: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = (|| {
            self.test(it, hw, mem, pos)?;
            it.pc = self.label(it, idx)?;
            it.after_jump();
            let pc = it.pc;
            Ok(self.point(it, pc))
        })();
        self.result(it, hw, mem, r, pos)
    }

    pub fn gosub_label(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, idx: i32, ret: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = (|| {
            self.test(it, hw, mem, pos)?;
            let target = self.label(it, idx)?;
            it.push_ctl(Ctl::Gosub { ret: ret as usize })?;
            it.pc = target;
            Ok(self.point(it, target))
        })();
        self.result(it, hw, mem, r, pos)
    }

    /// Label of a computed Goto / Gosub (`Interp::label_target` with an
    /// expression): a line number or a label name.
    fn label_by_value(&self, it: &Interp, v: Value) -> R<usize> {
        let name: Vec<u8> = match v {
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Float(f) => float_to_int(f).to_string().into_bytes(),
            Value::Str(s) => {
                if s.len() >= 32 {
                    return Err(Exc::Error(errors::LABEL_NOT_DEFINED));
                }
                s.iter().map(|c| c.to_ascii_lowercase()).collect()
            }
        };
        let s = &self.prg.scopes[it.scope];
        s.by_name.get(&name).map(|&i| s.labels[i].target).ok_or(Exc::Error(errors::LABEL_NOT_DEFINED))
    }

    /// Computed Goto (`kind` 0) or Gosub (1, returning to `ret`): the label
    /// value is on the stack; the test point was done before it.
    pub fn goto_value(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, kind: i32, ret: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let v = self.stack.pop().unwrap_or(Value::Int(0));
        let r = (|| {
            let target = self.label_by_value(it, v)?;
            if kind == 1 {
                it.push_ctl(Ctl::Gosub { ret: ret as usize })?;
                it.pc = target;
            } else {
                it.pc = target;
                it.after_jump();
            }
            Ok(self.point(it, target))
        })();
        self.result(it, hw, mem, r, pos)
    }

    /// Position of the parameter list of the current `Def Fn` of slot
    /// `slot` (error 15 when there is none).
    pub fn fn_def(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, slot: i32) -> i32 {
        let (it, _) = env.parts();
        it.inst_pos = pos as usize;
        match it.var_slot(slot as u16) {
            Var::Fn(p) => *p as i32,
            _ => {
                self.set_pending(mem, Exc::Error(15));
                0
            }
        }
    }

    /// `Restore` (`idx` -1) or `Restore label`.
    pub fn restore(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, idx: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let r = if idx < 0 {
            it.data.line = 0;
            it.data.item = 0;
            Ok(ST_CONTINUE)
        } else {
            self.label(it, idx).and_then(|target| {
                if structure::rd(&self.prg.code, target) != TK_DATA {
                    return Err(Exc::Error(41));
                }
                it.data.item = target + 4;
                it.data.line = self.line_after(target);
                Ok(ST_CONTINUE)
            })
        };
        self.result(it, hw, mem, r, pos)
    }

    /// Start of the line following the line containing `p`
    /// (`Interp::line_after`).
    fn line_after(&self, p: usize) -> usize {
        let code = &self.prg.code;
        let mut line = 0;
        loop {
            let next = line + code[line] as usize * 2;
            if next > p || code[line] == 0 {
                return next;
            }
            line = next;
        }
    }

    /// Finds the next Data statement (`Interp::find_data_line`).
    fn find_data_line(&self, it: &mut Interp) -> R<()> {
        let code = &self.prg.code;
        let mut line = if it.data.line == 0 { it.data.base.saturating_sub(2) } else { it.data.line };
        if it.scope == 0 && it.data.line == 0 {
            line = 0;
        }
        loop {
            if line + 3 >= code.len() || code[line] == 0 {
                return Err(Exc::Error(errors::OUT_OF_DATA));
            }
            let first = structure::rd(code, line + 2);
            let next = line + code[line] as usize * 2;
            if first == TK_END_PROC {
                return Err(Exc::Error(errors::OUT_OF_DATA));
            }
            if first == TK_PROCEDURE
                && let Some(pr) = self.prg.procs.iter().find(|pr| pr.pos == line + 2)
            {
                line = pr.end_line + code[pr.end_line] as usize * 2;
                continue;
            }
            let mut q = line + 2;
            if first == TK_LAB {
                q += crate::interp::verify::token_size(code, q);
            }
            if structure::rd(code, q) == TK_DATA {
                it.data.item = q + 4;
                it.data.line = next;
                return Ok(());
            }
            line = next;
        }
    }

    /// Next Data item for a variable of type `ty` (`Interp::next_data`,
    /// for programs whose Data items are constants), converted for the
    /// variable, in `RET`.
    pub fn read_data(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, ty: i32) {
        let (it, _) = env.parts();
        it.inst_pos = pos as usize;
        let ty = ty as u8;
        let r = (|| {
            if it.data.item == 0 {
                self.find_data_line(it)?;
            }
            let code = &self.prg.code;
            let Some((neg, lit, end)) = structure::data_item(code, it.data.item) else {
                return Err(Exc::Message("Compiled program: Data item is not a constant".into()));
            };
            let mut v = match lit {
                structure::Lit::Empty => Value::zero(ty),
                structure::Lit::Int(i) => Value::Int(i),
                structure::Lit::Ffp(b) => Value::Float(crate::number::ffp_to_f64(b)),
                structure::Lit::Dfl(x) => Value::Float(it.round_float(x)),
                structure::Lit::Str(q) => {
                    let n = structure::rd(code, q + 2) as usize;
                    Value::Str(astr(&code[q + 4..q + 4 + n]))
                }
            };
            if neg {
                v = match v {
                    Value::Int(i) => Value::Int(i.wrapping_neg()),
                    Value::Float(f) => Value::Float(-f),
                    Value::Str(_) => return Err(Exc::Error(errors::TYPE_MISMATCH)),
                };
            }
            it.data.item = if structure::rd(code, end) == TK_COMMA { end + 2 } else { 0 };
            if (ty == 2) != v.is_str() {
                return Err(Exc::Error(errors::TYPE_MISMATCH));
            }
            it.convert_for(ty, v)
        })();
        match r {
            Ok(Value::Int(i)) => st_i32(mem, layout::RET, i),
            Ok(Value::Float(f)) => st_f64(mem, layout::RET, f),
            Ok(Value::Str(s)) => {
                let h = self.alloc(mem, s);
                st_i32(mem, layout::RET, h);
            }
            Err(e) => self.set_pending(mem, e),
        }
    }

    pub fn return_(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        let r = (|| {
            self.test(it, hw, mem, pos)?;
            loop {
                match it.ctl.last() {
                    Some(Ctl::Gosub { ret }) => {
                        it.pc = *ret;
                        it.pop_ctl();
                        break;
                    }
                    Some(Ctl::Proc(_)) | None => return Err(Exc::Error(errors::RETURN_WITHOUT_GOSUB)),
                    _ => {
                        it.pop_ctl();
                    }
                }
            }
            let pc = it.pc;
            Ok(self.point(it, pc))
        })();
        self.result(it, hw, mem, r, pos)
    }

    // ------------------------------------------------------------------
    // Procedures
    // ------------------------------------------------------------------

    /// Calls procedure `index` (`Interp::call_proc`), returning to `ret`:
    /// the module has converted the parameters to their types and written
    /// them in the `args` area. The frame is pushed on the interpreter's
    /// control stack as usual (stack limit, events and errors unchanged),
    /// reusing frames of earlier calls; parameters go straight to memory.
    pub fn call_native(
        &mut self,
        env: &mut dyn Env,
        mem: &mut [u8],
        pos: i32,
        index: i32,
        ret: i32,
        nargs: i32,
    ) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let r = self.call_inner(it, mem, index as usize, ret as usize, nargs.max(0) as usize);
        self.result(it, hw, mem, r, pos)
    }

    fn call_inner(&mut self, it: &mut Interp, mem: &mut [u8], index: usize, ret: usize, nargs: usize) -> R<i32> {
        let prg = self.prg.clone();
        let p = &prg.procs[index];
        if p.machine_code {
            return Err(Exc::Message("Machine code procedures are not supported".into()));
        }
        let arg = |k: usize| self.layout.args + k as u32 * 8;
        // Global parameters are assigned before the frame is pushed (only
        // the parameters given: `On n Proc` gives none).
        for (k, (&slot, &ty)) in p.params.iter().zip(&p.param_types).enumerate().take(nargs) {
            if slot & GLOBAL != 0 {
                if self.is_resident(0, slot) {
                    let v = self.mem_value(mem, arg(k), ty);
                    it.globals[(slot & !GLOBAL) as usize] = Var::Scalar(v);
                } else {
                    let (from, to) = (arg(k) as usize, self.scalar_addr(slot, 0) as usize);
                    mem.copy_within(from..from + 8, to);
                }
            }
        }
        let mut frame = self.frame_pool.pop().unwrap_or_else(|| {
            Box::new(ProcFrame {
                proc_index: 0,
                ret: 0,
                locals: Vec::new(),
                data: DataPtr::default(),
                on_error: OnError::None,
                error_on: 0,
                error_pos: 0,
                scope: 0,
            })
        });
        frame.proc_index = index;
        frame.ret = ret;
        frame.locals.clear();
        frame.locals.resize(p.locals.len(), Var::Unset);
        frame.data = it.data;
        frame.on_error = it.on_error;
        frame.error_on = it.error_on;
        frame.error_pos = it.error_pos;
        frame.scope = it.scope;
        it.push_ctl(Ctl::Proc(frame))?;
        it.frame_stack.push(it.ctl.len() - 1);
        it.scope = index + 1;
        it.data = DataPtr { base: p.body, line: 0, item: 0 };
        it.on_error = OnError::None;
        it.pc = p.body;
        // The frame in memory: cleared, then the local parameters.
        let depth = it.frame_stack.len();
        let base = self.layout.frame_base(depth) as usize;
        let size = p.locals.len() * 8;
        let Some(b) = mem.get_mut(base..base + size) else {
            return Err(Exc::Message("Compiled program: procedure frames exhausted".into()));
        };
        b.fill(0);
        for (k, (&slot, &ty)) in p.params.iter().zip(&p.param_types).enumerate().take(nargs) {
            if slot & GLOBAL != 0 {
                continue;
            }
            if self.is_resident(index + 1, slot) {
                let v = self.mem_value(mem, arg(k), ty);
                if let Some(Ctl::Proc(f)) = it.ctl.last_mut() {
                    f.locals[slot as usize] = Var::Scalar(v);
                }
            } else {
                let from = arg(k) as usize;
                mem.copy_within(from..from + 8, base + slot as usize * 8);
            }
        }
        // Memory is up to date for this frame.
        if self.frames.len() > depth - 1 {
            self.free_arrays_above(depth - 1);
        }
        self.frames.truncate(depth - 1);
        self.frames.push(it.ctl.len() - 1);
        Ok(self.point(it, p.body))
    }

    /// End Proc / Pop Proc in a procedure (the test point was done and the
    /// frame checked): sets Param from `RET` / `RET_TAG`, then returns
    /// (`Interp::return_proc`, reusing the frame).
    pub fn proc_end(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        match ld_i32(mem, layout::RET_TAG) {
            0 => it.param_e = ld_i32(mem, layout::RET),
            1 => it.param_f = ld_f64(mem, layout::RET),
            2 => it.param_s = self.str_of(mem, ld_i32(mem, layout::RET)),
            _ => {}
        }
        let in_error_proc = it.error_proc_depth.is_some() && it.error_proc_depth == Some(it.frame_stack.len());
        let r = if it.error_on != 0 && in_error_proc { Err(Exc::Error(8)) } else { self.return_inner(it) };
        self.result(it, hw, mem, r, pos)
    }

    fn return_inner(&mut self, it: &mut Interp) -> R<i32> {
        while let Some(c) = it.ctl.last() {
            if matches!(c, Ctl::Proc(_)) {
                break;
            }
            it.pop_ctl();
        }
        let Some(Ctl::Proc(mut f)) = it.pop_ctl() else { return Err(Exc::Error(errors::ILLEGAL_FUNCTION_CALL)) };
        if it.error_proc_depth == Some(it.frame_stack.len()) {
            it.error_proc_depth = None;
        }
        it.frame_stack.pop();
        it.data = f.data;
        it.on_error = f.on_error;
        it.error_on = f.error_on;
        it.error_pos = f.error_pos;
        it.scope = f.scope;
        it.pc = f.ret;
        f.locals.clear();
        if self.frame_pool.len() < 64 {
            self.frame_pool.push(f);
        }
        let pc = it.pc;
        Ok(self.point(it, pc))
    }

    /// Start of End Proc / Pop Proc (after the test point).
    pub fn proc_check(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, pop: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let r = if !it.frame_stack.is_empty() {
            Ok(ST_CONTINUE)
        } else if pop == 0 {
            Err(Exc::Stop(StopReason::End))
        } else {
            Err(Exc::Error(errors::ILLEGAL_FUNCTION_CALL))
        };
        self.result(it, hw, mem, r, pos)
    }

    // ------------------------------------------------------------------
    // Values (`rt.*`)
    // ------------------------------------------------------------------

    pub fn str_f(&mut self, env: &mut dyn Env, mem: &mut [u8], x: f64) -> i32 {
        let s = env.parts().0.format_float(x);
        self.alloc(mem, astr(s.as_bytes()))
    }

    /// Double precision value of the float text at `a` (the conversion of
    /// `tokenise::parse_number`, used by `Val`).
    pub fn val_double(&self, mem: &mut [u8], a: i32) -> f64 {
        let t = self.str_bytes(mem, a);
        crate::tokenise::parse_float_text(&String::from_utf8_lossy(t))
    }

    pub fn param_s(&mut self, env: &mut dyn Env, mem: &mut [u8]) -> i32 {
        let s = env.parts().0.param_s.clone();
        self.alloc(mem, s)
    }

    /// `Int` of a single precision float (`round_float(x.floor())`).
    pub fn int_f(&self, x: f64) -> f64 {
        Ffp::from_f64(x.floor()).to_f64()
    }

    /// Integer to single precision float (`SPFlt`).
    pub fn i2f(&self, v: i32) -> f64 {
        if self.double { v as f64 } else { Ffp::from_i32(v).to_f64() }
    }

    pub fn fadd(&self, a: f64, b: f64) -> f64 {
        Ffp::from_f64(a).add(Ffp::from_f64(b)).to_f64()
    }

    pub fn fsub(&self, a: f64, b: f64) -> f64 {
        Ffp::from_f64(a).sub(Ffp::from_f64(b)).to_f64()
    }

    pub fn fmul(&self, a: f64, b: f64) -> f64 {
        Ffp::from_f64(a).mul(Ffp::from_f64(b)).to_f64()
    }

    pub fn fdiv(&self, a: f64, b: f64) -> f64 {
        Ffp::from_f64(a).div(Ffp::from_f64(b)).to_f64()
    }

    /// Single precision comparison: -1, 0 or 1.
    pub fn fcmp(&self, a: f64, b: f64) -> i32 {
        match Ffp::from_f64(a).cmp(Ffp::from_f64(b)) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// `x^y` (`round_float(x.powf(y))`).
    pub fn pow(&self, a: f64, b: f64) -> f64 {
        let r = a.powf(b);
        if self.double { r } else { Ffp::from_f64(r).to_f64() }
    }

    /// Binary operator on numbers of types known at run time only
    /// (`Interp::binop`). The result type goes to `TAG`.
    #[allow(clippy::too_many_arguments)]
    pub fn dyn_op(&mut self, env: &mut dyn Env, mem: &mut [u8], op: i32, a: f64, ta: i32, b: f64, tb: i32) -> f64 {
        let (it, _) = env.parts();
        match it.binop(op as u16, dyn_value(a, ta), dyn_value(b, tb)) {
            Ok(Value::Int(i)) => {
                st_i32(mem, layout::TAG, 0);
                i as f64
            }
            Ok(Value::Float(f)) => {
                st_i32(mem, layout::TAG, 1);
                f
            }
            Ok(Value::Str(_)) => {
                self.set_pending(mem, Exc::Error(errors::TYPE_MISMATCH));
                0.0
            }
            Err(e) => {
                self.set_pending(mem, e);
                0.0
            }
        }
    }
}

/// Point of every even position of `code` (see `Runtime::point_at`):
/// instruction starts, and the `:` / end of line tokens that lead to them.
fn point_table(code: &[u8], instrs: &[Instr]) -> Vec<i32> {
    let n = code.len() / 2 + 1;
    let mut table = vec![-1i32; n];
    let end = instrs.len() as i32;
    for (i, ins) in instrs.iter().enumerate() {
        if ins.pos / 2 < n {
            table[ins.pos / 2] = i as i32;
        }
    }
    let mut line = 0;
    while line + 1 < code.len() && code[line] != 0 {
        let mut p = line + 2;
        loop {
            let t = structure::rd(code, p);
            if t == TK_DP || t == TK_EOL {
                let v = match structure::normalize(code, p) {
                    None => end,
                    Some(q) => instrs.binary_search_by_key(&q, |i| i.pos).map_or(-1, |i| i as i32),
                };
                table[p / 2] = v;
            }
            if t == TK_EOL || p + 1 >= code.len() {
                break;
            }
            p += crate::interp::verify::token_size(code, p);
        }
        line += code[line] as usize * 2;
    }
    table
}

/// Value of a dynamic number (payload, tag 0 int / 1 float).
pub fn dyn_value(v: f64, tag: i32) -> Value {
    if tag == 0 { Value::Int(v as i32) } else { Value::Float(v) }
}
