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
use crate::interp::value::{AStr, STRING_MAX, Value, Var, astr, empty_str, float_to_int};
use crate::interp::verify::{Compiled, GLOBAL};
use crate::interp::{Ctl, Exc, Host, Interp, OnError, R, StopInfo, StopReason, StopReasonOrError, VarLoc};
use crate::tokens::*;

pub const ST_CONTINUE: i32 = -1;
pub const ST_YIELD: i32 = -2;
pub const ST_STOP: i32 = -3;

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
    // String table: handles are indices, 0 is the empty string.
    strs: Vec<AStr>,
    live: Vec<bool>,
    pinned: Vec<bool>,
    free: Vec<u32>,
    consts: HashMap<usize, i32>,
    allocs: usize,
    gc_limit: usize,
    gc_wanted: bool,
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
}

impl Runtime {
    /// Runtime for the verified program `prg` (the one the interpreter was
    /// started with); `base` is the address of the module's memory region.
    pub fn new(prg: Rc<Compiled>, base: u32) -> Runtime {
        let instrs = structure::instructions(&prg);
        let layout = Layout::new(&prg);
        let end_pos = structure::end_position(&prg.code);
        let resident = structure::resident_vars(&prg, &instrs);
        let point_at = point_table(&prg.code, &instrs);
        Runtime {
            point_at,
            resident,
            double: prg.double,
            prg,
            instrs,
            layout,
            base,
            end_pos,
            strs: vec![empty_str()],
            live: vec![true],
            pinned: vec![true],
            free: Vec::new(),
            consts: HashMap::new(),
            allocs: 0,
            gc_limit: 8192,
            gc_wanted: false,
            stack: Vec::new(),
            print_buf: Vec::new(),
            pending: None,
            frames: Vec::new(),
            vars: HashMap::new(),
            calls: HashMap::new(),
            bridge_buf: Vec::new(),
            stopped: None,
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

    /// Number of live strings (for tests).
    pub fn string_count(&self) -> usize {
        self.live.iter().filter(|&&l| l).count()
    }

    // ------------------------------------------------------------------
    // Strings
    // ------------------------------------------------------------------

    pub fn str_of(&self, h: i32) -> AStr {
        self.strs.get(h as usize).cloned().unwrap_or_else(empty_str)
    }

    fn alloc(&mut self, mem: &mut [u8], s: AStr) -> i32 {
        if s.is_empty() {
            return 0;
        }
        self.allocs += 1;
        if self.allocs >= self.gc_limit && !self.gc_wanted {
            self.gc_wanted = true;
            st_i32(mem, layout::ATT, 1);
        }
        if let Some(h) = self.free.pop() {
            self.strs[h as usize] = s;
            self.live[h as usize] = true;
            h as i32
        } else {
            self.strs.push(s);
            self.live.push(true);
            self.pinned.push(false);
            (self.strs.len() - 1) as i32
        }
    }

    /// Frees the strings no variable refers to. Only called where the
    /// module holds no string in its wasm locals (start of an instruction
    /// or test point).
    fn gc(&mut self, it: &Interp, mem: &mut [u8]) {
        let mut mark = vec![false; self.strs.len()];
        let prg = self.prg.clone();
        let mut mark_slot = |a: u32| {
            let h = ld_i32(mem, a);
            if h > 0 && (h as usize) < mark.len() {
                mark[h as usize] = true;
            }
        };
        for (i, d) in prg.globals.iter().enumerate() {
            if d.ty == 2 && structure::is_scalar(d) {
                mark_slot(self.layout.globals + i as u32 * 8);
            }
        }
        for (k, &idx) in self.frames.iter().enumerate() {
            if let Some(Ctl::Proc(f)) = it.ctl.get(idx) {
                let base = self.layout.frame_base(k + 1);
                for (i, d) in prg.procs[f.proc_index].locals.iter().enumerate() {
                    if d.ty == 2 && structure::is_scalar(d) {
                        mark_slot(base + i as u32 * 8);
                    }
                }
            }
        }
        let mut live = 0;
        for (h, &marked) in mark.iter().enumerate().skip(1) {
            if self.live[h] && !marked && !self.pinned[h] {
                self.live[h] = false;
                self.strs[h] = empty_str();
                self.free.push(h as u32);
            } else if self.live[h] {
                live += 1;
            }
        }
        self.allocs = 0;
        self.gc_limit = (live * 2).max(8192);
        self.gc_wanted = false;
        st_i32(mem, layout::ATT, it.vbl_pending as i32);
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

    fn mem_value(&self, mem: &[u8], a: u32, ty: u8) -> Value {
        match ty {
            1 => Value::Float(ld_f64(mem, a)),
            2 => Value::Str(self.str_of(ld_i32(mem, a))),
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
                if Rc::ptr_eq(&self.str_of(cur), s) {
                    return;
                }
                let h = self.alloc(mem, s.clone());
                st_i32(mem, a, h);
            }
            _ => {}
        }
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
        self.mirror(it, mem);
    }

    /// Refreshes the mirror of the top of the control stack.
    fn mirror(&mut self, it: &Interp, mem: &mut [u8]) {
        let (mut addr, mut step, mut limit, mut body_point) = (0, 0, 0, 0);
        let (mut lo, mut hi) = (0, i32::MAX);
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
                    }
                }
            }
            Some(Ctl::Repeat { body, exit } | Ctl::Do { body, exit } | Ctl::While { body, exit, .. }) => {
                lo = *body as i32;
                hi = *exit as i32;
            }
            _ => {}
        }
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
        self.sync(it, mem);
        if self.gc_wanted {
            self.gc(it, mem);
        }
        let pc = it.pc;
        self.point(it, pc)
    }

    /// The time budget of `run` is used: the program continues at `point`
    /// next time.
    pub fn suspend(&mut self, env: &mut dyn Env, point: i32) {
        let (it, _) = env.parts();
        it.pc = self.instrs.get(point as usize).map_or(self.end_pos, |i| i.pos);
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

    pub fn push_s(&mut self, h: i32) {
        let s = self.str_of(h);
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

    pub fn print_s(&mut self, h: i32) {
        let s = self.str_of(h);
        self.print_buf.extend_from_slice(&s);
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

    /// Flat index of an element of array `slot` (`Interp::var_ref`).
    #[allow(clippy::too_many_arguments)]
    pub fn aref(&mut self, env: &mut dyn Env, mem: &mut [u8], slot: i32, n: i32, idx: [i32; 8]) -> i32 {
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
        let s = self.str_of(h);
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

    /// Calls procedure `index` with `n` parameters from the stack; returns
    /// to `ret`. The test point was done before the parameters.
    pub fn call(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, index: i32, ret: i32, n: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let args = self.stack.split_off(self.stack.len().saturating_sub(n.max(0) as usize));
        let r = it.call_proc(index as usize, ret as usize, args).map(|_| {
            let pc = it.pc;
            self.point(it, pc)
        });
        self.result(it, hw, mem, r, pos)
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

    pub fn set_param_i(&mut self, env: &mut dyn Env, v: i32) {
        env.parts().0.param_e = v;
    }

    pub fn set_param_f(&mut self, env: &mut dyn Env, v: f64) {
        env.parts().0.param_f = v;
    }

    pub fn set_param_s(&mut self, env: &mut dyn Env, h: i32) {
        let s = self.str_of(h);
        env.parts().0.param_s = s;
    }

    /// End of End Proc / Pop Proc: returns to the caller.
    pub fn proc_return(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let in_error_proc = it.error_proc_depth.is_some() && it.error_proc_depth == Some(it.frame_stack.len());
        let r = if it.error_on != 0 && in_error_proc {
            Err(Exc::Error(8))
        } else {
            it.return_proc().map(|_| {
                let pc = it.pc;
                self.point(it, pc)
            })
        };
        self.result(it, hw, mem, r, pos)
    }

    // ------------------------------------------------------------------
    // Values (`rt.*`)
    // ------------------------------------------------------------------

    /// String constant of the `TK_CH1` / `TK_CH2` token at `pos`.
    pub fn str_const(&mut self, mem: &mut [u8], pos: i32) -> i32 {
        if let Some(&h) = self.consts.get(&(pos as usize)) {
            return h;
        }
        let p = pos as usize;
        let code = &self.prg.code;
        let n = structure::rd(code, p + 2) as usize;
        let s = astr(code.get(p + 4..p + 4 + n).unwrap_or(&[]));
        let h = self.alloc(mem, s);
        if h > 0 {
            self.pinned[h as usize] = true;
        }
        self.consts.insert(p, h);
        h
    }

    pub fn str_concat(&mut self, mem: &mut [u8], a: i32, b: i32) -> i32 {
        let (x, y) = (self.str_of(a), self.str_of(b));
        if x.is_empty() {
            return b;
        }
        if y.is_empty() {
            return a;
        }
        if x.len() + y.len() >= STRING_MAX {
            self.set_pending(mem, Exc::Error(errors::STRING_TOO_LONG));
            return 0;
        }
        let mut v = Vec::with_capacity(x.len() + y.len());
        v.extend_from_slice(&x);
        v.extend_from_slice(&y);
        self.alloc(mem, v.into())
    }

    /// `a$-b$`: removes every occurrence of b$ (`string_minus`).
    pub fn str_minus(&mut self, mem: &mut [u8], a: i32, b: i32) -> i32 {
        let (x, y) = (self.str_of(a), self.str_of(b));
        if y.is_empty() {
            return a;
        }
        let mut s = x.to_vec();
        while let Some(i) = s.windows(y.len()).position(|w| w == &y[..]) {
            s.drain(i..i + y.len());
        }
        self.alloc(mem, s.into())
    }

    pub fn str_cmp(&self, a: i32, b: i32) -> i32 {
        if a == b {
            return 0;
        }
        match self.str_of(a)[..].cmp(&self.str_of(b)[..]) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    pub fn str_len(&self, h: i32) -> i32 {
        self.str_of(h).len() as i32
    }

    // String and maths functions of the interpreter core
    // (`Interp::string_maths_function`), for the most used ones.

    pub fn str_asc(&self, h: i32) -> i32 {
        self.str_of(h).first().copied().unwrap_or(0) as i32
    }

    pub fn chr(&mut self, mem: &mut [u8], n: i32) -> i32 {
        if !(0..=255).contains(&n) {
            self.set_pending(mem, Exc::Error(errors::ILLEGAL_FUNCTION_CALL));
            return 0;
        }
        self.alloc(mem, astr(&[n as u8]))
    }

    /// `Left$` (`right` 0) / `Right$` (1).
    pub fn left_right(&mut self, mem: &mut [u8], h: i32, n: i32, right: i32) -> i32 {
        if n < 0 {
            self.set_pending(mem, Exc::Error(errors::ILLEGAL_FUNCTION_CALL));
            return 0;
        }
        let s = self.str_of(h);
        let n = (n as usize).min(s.len());
        if n == s.len() {
            return h;
        }
        let r = if right == 0 { astr(&s[..n]) } else { astr(&s[s.len() - n..]) };
        self.alloc(mem, r)
    }

    /// `Mid$(s,p,n)` (`has_n` 1) / `Mid$(s,p)`.
    pub fn mid(&mut self, mem: &mut [u8], h: i32, p: i32, n: i32, has_n: i32) -> i32 {
        if p < 0 {
            self.set_pending(mem, Exc::Error(errors::ILLEGAL_FUNCTION_CALL));
            return 0;
        }
        let s = self.str_of(h);
        let start = (p.max(1) - 1) as usize;
        if start >= s.len() {
            return 0;
        }
        let r = if has_n != 0 {
            if n == 0 {
                return 0;
            } else if n < 0 {
                self.set_pending(mem, Exc::Error(errors::ILLEGAL_FUNCTION_CALL));
                return 0;
            }
            let end = (start + n as usize).min(s.len());
            astr(&s[start..end])
        } else {
            astr(&s[start..])
        };
        self.alloc(mem, r)
    }

    pub fn str_i(&mut self, mem: &mut [u8], n: i32) -> i32 {
        let s = crate::ffp::format_int(n);
        self.alloc(mem, astr(s.as_bytes()))
    }

    pub fn str_f(&mut self, env: &mut dyn Env, mem: &mut [u8], x: f64) -> i32 {
        let s = env.parts().0.format_float(x);
        self.alloc(mem, astr(s.as_bytes()))
    }

    /// `Instr(h$,n$)` (`has_start` 0) / `Instr(h$,n$,start)`.
    pub fn instr(&mut self, mem: &mut [u8], h: i32, n: i32, start: i32, has_start: i32) -> i32 {
        let start = if has_start != 0 { start } else { 1 };
        if start < 0 {
            self.set_pending(mem, Exc::Error(errors::ILLEGAL_FUNCTION_CALL));
            return 0;
        }
        crate::interp::expr::instr(&self.str_of(h), &self.str_of(n), start.max(1) as usize)
    }

    /// `Upper$` (`lower` 0) / `Lower$` (1).
    pub fn change_case(&mut self, mem: &mut [u8], h: i32, lower: i32) -> i32 {
        let s = self.str_of(h);
        let v: Vec<u8> =
            s.iter().map(|&c| if lower == 0 { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).collect();
        self.alloc(mem, v.into())
    }

    pub fn param_i(&mut self, env: &mut dyn Env) -> i32 {
        env.parts().0.param_e
    }

    pub fn param_f(&mut self, env: &mut dyn Env) -> f64 {
        env.parts().0.param_f
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
