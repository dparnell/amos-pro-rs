//! The Interface interpreter (`Dia_OpenChannel` `+Lib.s:19860`,
//! `Dia_RunProgram` `+Lib.s:20506`, `Dia_Loop` `+Lib.s:21229`,
//! `Dia_Evalue` `+Lib.s:22719`).
//!
//! A program is a sequence of `XX params;` where XX is two upper case
//! letters. `Dia_Loop` is recursive in the original (IF blocks, user
//! instructions, zone routines); here IF blocks and user instructions are
//! [`Frame`]s of the channel so that a `RUn` inside them can wait across
//! frames. Routines called by the zone code run to completion (a `RUn`
//! there does not wait).

use std::rc::Rc;

use super::lexer::{self, Class, f};
use super::*;
use crate::gfx::draw::{Canvas, GrState};
use crate::gfx::{gfont, pack};
use crate::interp::Interp;
use crate::machine::Hardware;

/// Result of the Interface routines: Err = Interface error code.
pub type DR<T> = Result<T, u8>;

/// Instruction names (`Dia_Instr` `+Lib.s:21048`): the first match wins,
/// so the second "IL" (Inactive List) is unreachable.
pub const NAMES: [&[u8; 2]; 51] = [
    b"EX", b"UN", b"LI", b"BO", b"SI", b"BA", b"PU", b"SV", b"IL", b"PR", b"PO", b"IN", b"SF",
    b"SW", b"SL", b"SP", b"BU", b"JP", b"RU", b"BR", b"ED", b"JS", b"RT", b"BC", b"KY", b"SA",
    b"DI", b"BL", b"LA", b"BQ", b"HS", b"VS", b"AL", b"IL", b"ZC", b"UI", b"GB", b"GS", b"GL",
    b"IF", b"SZ", b"XY", b"NW", b"VT", b"VL", b"HT", b"CA", b"SM", b"GE", b"GP", b"SS",
];

/// Number of parameters (`Dia_NParam`): >= 7 means the instruction reads
/// its own parameters.
pub const NPARAM: [u8; 51] = [
    0, 3, 4, 5, 2, 2, 1, 2, 0, 4, 5, 3, 2, 1, 1, 2, 8, 1, 2, 1, 8, 1, 0, 2, 2, 1, 8, 1, 1, 0, 9, 9,
    10, 10, 2, 0, 4, 4, 4, 1, 1, 4, 0, 4, 4, 10, 1, 0, 4, 3, 8,
];

const I_LA: usize = 28;
const I_UI: usize = 35;

fn find_instr(name: [u8; 2]) -> Option<usize> {
    NAMES.iter().position(|n| **n == name)
}

/// Result of a run of the interpreter loop.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Done,
    /// `RUn` reached: the program waits.
    Suspend,
}

/// Labels and user instructions found by the pre-pass.
pub type Labels = (Vec<(i32, usize)>, Vec<([u8; 2], u16, usize)>);

/// The pre-pass of `Dia_OpenChannel`: checks the syntax, finds the `LA`
/// labels and the `UI` user instruction definitions. Error: (code,
/// position).
pub fn prepass(p: &[u8]) -> Result<Labels, (u8, usize)> {
    let mut pc = 0usize;
    let mut labels: Vec<(i32, usize)> = Vec::new();
    let mut users: Vec<([u8; 2], u16, usize)> = Vec::new();
    let synt = |pc: usize| Err((e::SYNTAX, pc));
    loop {
        let (c1, cl) = lexer::chr(p, &mut pc);
        if cl == Class::End {
            return Ok((labels, users));
        }
        if c1 == b'[' || c1 == b']' {
            continue;
        }
        let (c2, cl2) = lexer::chr(p, &mut pc);
        if cl2 == Class::End {
            return synt(pc);
        }
        match find_instr([c1, c2]) {
            None => {
                // Call of a user instruction: skips the parameters.
                let (_, cl) = lexer::chr(p, &mut pc);
                match cl {
                    Class::End => return synt(pc),
                    Class::Sep => {}
                    _ => {
                        pc -= 1;
                        loop {
                            let t = skip_expr(p, &mut pc).map_err(|pc| (e::SYNTAX, pc))?;
                            if t != b',' {
                                break;
                            }
                        }
                    }
                }
            }
            Some(I_LA) => {
                let (c, cl) = lexer::chr(p, &mut pc);
                if cl != Class::Num {
                    return synt(pc);
                }
                let n = lexer::number(p, &mut pc, c);
                if !(0..65536).contains(&n) {
                    return synt(pc);
                }
                if labels.iter().any(|l| l.0 == n) {
                    return Err((e::LABEL_DEFINED, pc));
                }
                let (_, cl) = lexer::chr(p, &mut pc);
                if cl != Class::Sep {
                    return synt(pc);
                }
                labels.push((n, pc));
            }
            Some(I_UI) => {
                let (a, cl) = lexer::chr(p, &mut pc);
                if !matches!(cl, Class::Letter | Class::Op) {
                    return synt(pc);
                }
                let (b, cl) = lexer::chr(p, &mut pc);
                if !matches!(cl, Class::Letter | Class::Op | Class::Num) {
                    return synt(pc);
                }
                let name = [a, b];
                if find_instr(name).is_some() {
                    return synt(pc);
                }
                if users.iter().any(|u| u.0 == name) {
                    return Err((e::LABEL_DEFINED, pc));
                }
                if lexer::chr(p, &mut pc).0 != b',' {
                    return synt(pc);
                }
                let (c, cl) = lexer::chr(p, &mut pc);
                if cl != Class::Num {
                    return synt(pc);
                }
                let np = lexer::number(p, &mut pc, c);
                if !(0..=9).contains(&np) {
                    return synt(pc);
                }
                if matches!(
                    lexer::chr(p, &mut pc).1,
                    Class::Letter | Class::Op | Class::Num
                ) {
                    return synt(pc);
                }
                if lexer::chr(p, &mut pc).0 != b'[' {
                    return synt(pc);
                }
                users.push((name, np as u16, pc));
            }
            Some(i) => {
                let np = NPARAM[i];
                if np == 0 {
                    match lexer::chr(p, &mut pc).1 {
                        Class::Sep => {}
                        _ => return synt(pc),
                    }
                } else {
                    for k in 0..np {
                        let t = skip_expr(p, &mut pc).map_err(|pc| (e::SYNTAX, pc))?;
                        if k + 1 < np && t != b',' {
                            return Err((e::NPARAM, pc));
                        }
                    }
                }
            }
        }
    }
}

/// Skips an expression (pre-pass `.Evalue`); returns its terminator.
fn skip_expr(p: &[u8], pc: &mut usize) -> Result<u8, usize> {
    loop {
        let (c, cl) = lexer::chr(p, pc);
        match cl {
            Class::Num => {
                lexer::number(p, pc, c);
            }
            Class::Op => {
                lexer::function(c, 0, true).ok_or(*pc)?;
                if c == b'"' || c == b'\'' {
                    loop {
                        let b = p.get(*pc).copied().unwrap_or(0);
                        *pc += 1;
                        if b < 32 {
                            return Err(*pc);
                        }
                        if b == c {
                            break;
                        }
                    }
                }
            }
            Class::End => return Err(*pc),
            Class::Sep => return Ok(c),
            Class::Letter => {
                let (c2, _) = lexer::chr(p, pc);
                lexer::function(c, c2, false).ok_or(*pc)?;
            }
        }
    }
}

/// Decimal conversion (`Dia_DecToAsc`).
pub fn dec(n: i32) -> Vec<u8> {
    n.to_string().into_bytes()
}

/// Interface strings have a byte length.
fn mkstr(mut v: Vec<u8>) -> DVal {
    v.truncate(v.len() % 256);
    DVal::Str(Rc::from(v))
}

/// Integer value of a parameter.
pub fn int(v: &DVal) -> DR<i32> {
    match v {
        DVal::Int(n) => Ok(*n),
        _ => Err(e::TYPE),
    }
}

/// String value of a parameter.
pub fn string(v: &DVal) -> DR<Rc<[u8]>> {
    match v {
        DVal::Str(s) => Ok(s.clone()),
        _ => Err(e::TYPE),
    }
}

/// True for non zero values (strings and arrays are addresses).
pub fn truth(v: &DVal) -> bool {
    !matches!(v, DVal::Int(0))
}

/// Channel opening parameters.
pub struct OpenParams {
    pub number: i64,
    pub prog: Rc<[u8]>,
    pub nvar: usize,
    pub buffer: usize,
    pub res: Resource,
}

/// Size of the buffer records (`+Equ.s:1018-1107`).
pub mod size {
    pub const BUTTON: usize = 36;
    pub const EDIT: usize = 60;
    pub const DIGIT: usize = 80;
    pub const LIST: usize = 44;
    pub const TEXT: usize = 132;
    pub const SLIDER: usize = 96;
    pub const KEY: usize = 10;
    pub const BLOCK: usize = 6;
}

impl Hardware {
    /// Identity of an open screen (detects a screen closed and opened
    /// again, "screen modified").
    pub(crate) fn dia_screen_id(&self, n: usize) -> Option<usize> {
        self.screens
            .screens
            .get(n)?
            .as_deref()
            .map(|s| s as *const crate::gfx::Screen as usize)
    }

    /// `Dia_OpenChannel`: creates a channel on the current screen.
    pub(crate) fn dia_open_channel(&mut self, p: OpenParams) -> Result<(), (u8, usize)> {
        if self.dialogs.channel_index(p.number).is_some() {
            return Err((e::CHANNEL_DEFINED, 0));
        }
        let Some(screen) = self.screens.current else {
            return Err((e::SCREEN, 0));
        };
        let mut prog = p.prog.to_vec();
        if prog.is_empty() {
            return Err((e::SYNTAX, 0));
        }
        prog.push(0);
        let (labels, users) = prepass(&prog)?;
        let label_bytes = (labels.len() + users.len()) * 6 + 4;
        let ch = Channel {
            number: p.number,
            vars: vec![DVal::Int(0); p.nvar + 1],
            v_bp: DVal::Int(0),
            v_br: DVal::Int(0),
            v_zone: None,
            prog: Rc::from(prog),
            labels,
            users,
            res: p.res,
            screen,
            screen_id: self.dia_screen_id(screen).unwrap_or(0),
            screen_old: None,
            wind_old: 0,
            wind_on: 0,
            buf_size: p.buffer & !1,
            label_bytes,
            pbuf: 0,
            abuf: 0,
            records: Vec::new(),
            edited: Edited::None,
            timer: 0,
            timer_pos: 0,
            last_zone: None,
            next_zone: DVal::Int(0),
            release: None,
            base_x: 0,
            base_y: 0,
            sx: 0,
            sy: 0,
            xa: 0,
            ya: 0,
            xb: 0,
            yb: 0,
            puzzle_sx: 0,
            puzzle_sy: 0,
            puzzle_i: 0,
            last_key: None,
            error: 0,
            error_pos: 0,
            ret: 0,
            exit: 0,
            writing: 0,
            rflags: 0,
            flags: 0,
            sl_default: [0, 0, 0, 1, 4, 4, 4, 1, 0, 0, 0, 1, 3, 3, 3, 1],
            pc: 0,
            frames: Vec::new(),
            stack: Vec::new(),
            pusers: None,
            npusers: 0,
            users_depth: 0,
            run: None,
            modal: None,
        };
        self.dialogs.channels.insert(0, ch);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Screen activation
    // ------------------------------------------------------------------

    /// `Dia_Active`: makes the dialog screen current.
    pub(crate) fn dia_active(&mut self, ch: &mut Channel) -> DR<()> {
        if self.dia_screen_id(ch.screen) != Some(ch.screen_id) {
            return Err(e::SCREEN);
        }
        ch.screen_old = None;
        if self.screens.current != Some(ch.screen) {
            ch.screen_old = self.screens.current;
            let _ = self.screens.activate(ch.screen);
        }
        let w = self.screens.get(ch.screen).map_or(0, |s| s.windon());
        ch.wind_old = w;
        ch.wind_on = w;
        Ok(())
    }

    /// `Dia_ReActive`: back to the previous screen and window.
    pub(crate) fn dia_reactive(&mut self, ch: &mut Channel) {
        if let Edited::Zone(z) = ch.edited {
            self.dia_ed_active(ch, z);
        } else if ch.wind_old != ch.wind_on {
            ch.wind_on = ch.wind_old;
            if let Some(s) = self.screens.get_mut(ch.screen) {
                let _ = s.window_activate(ch.wind_old);
            }
        }
        if let Some(o) = ch.screen_old.take() {
            let _ = self.screens.activate(o);
        }
    }

    /// `Dia_WActive`: activates window `n` of the dialog screen if needed.
    pub(crate) fn dia_wactive(&mut self, ch: &mut Channel, n: i32) {
        if ch.wind_on != n {
            ch.wind_on = n;
            if let Some(s) = self.screens.get_mut(ch.screen) {
                let _ = s.window_activate(n);
            }
        }
    }

    // ------------------------------------------------------------------
    // Buffer accounting
    // ------------------------------------------------------------------

    /// Bytes used by the interpreter stack (a3, top of the buffer).
    fn stack_bytes(ch: &Channel) -> usize {
        ch.stack
            .iter()
            .map(|s| if matches!(s, SVal::Ret(_)) { 8 } else { 4 })
            .sum()
    }

    /// `Dia_GetBuffer`: reserves `n` bytes in the channel buffer.
    pub(crate) fn dia_alloc(ch: &mut Channel, n: usize) -> DR<()> {
        let n = (n + 1) & !1;
        let top = ch.buf_size.saturating_sub(Self::stack_bytes(ch));
        if ch.label_bytes + ch.abuf + n + 4 >= top {
            return Err(e::BUFFER);
        }
        ch.abuf += n;
        Ok(())
    }

    /// String temporaries (`Dia_StDebut` / `Dia_StFini`): not checked.
    fn dia_alloc_str(ch: &mut Channel, len: usize) {
        ch.abuf += (len + 6 + 1) & !1;
    }

    // ------------------------------------------------------------------
    // Running a program
    // ------------------------------------------------------------------

    /// `Dia_RunProgram` (first part): erases the channel, positions the
    /// program and runs it until it ends or waits in `RUn`. Returns
    /// (error, value) as the original: the value is the result of the
    /// `RUn` or 0.
    pub(crate) fn dia_run_program(
        &mut self,
        it: &mut Interp,
        n: i64,
        label: Option<i32>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<RunResult, (u8, usize)> {
        let _ = self.dia_eff_channel(it, n);
        let Some((i, mut ch)) = self.dialogs.take(n) else {
            return Err((e::CHANNEL_NOT_DEFINED, 0));
        };
        let r = self.dia_run_start(it, &mut ch, label, x, y);
        self.dialogs.put(i, ch);
        r
    }

    fn dia_run_start(
        &mut self,
        it: &mut Interp,
        ch: &mut Channel,
        label: Option<i32>,
        x: Option<i32>,
        y: Option<i32>,
    ) -> Result<RunResult, (u8, usize)> {
        if let Some(x) = x {
            ch.base_x = x;
        }
        if let Some(y) = y {
            ch.base_y = y;
        }
        let mut pc = 0;
        if let Some(l) = label {
            match ch.labels.iter().find(|e| e.0 == l) {
                Some(e) => pc = e.1,
                None => return Err((e::LABEL_NOT_DEFINED, 0)),
            }
        }
        ch.pc = pc;
        ch.stack.clear();
        ch.frames.clear();
        ch.pbuf = 0;
        ch.abuf = 0;
        ch.records.clear();
        ch.last_key = None;
        ch.edited = Edited::None;
        ch.last_zone = None;
        ch.xa = 0;
        ch.ya = 0;
        ch.xb = 0;
        ch.yb = 0;
        ch.ret = 0;
        ch.error = 0;
        ch.users_depth = 0;
        ch.pusers = None;
        ch.npusers = 0;
        ch.release = None;
        ch.run = None;
        ch.modal = None;
        if let Err(c) = self.dia_active(ch) {
            return Err((c, 0));
        }
        ch.writing = 0;
        if let Some(s) = self.screens.get_mut(ch.screen) {
            s.gr.writing = 0;
        }
        ch.rflags = (ch.rflags | 1) & !6;
        self.dia_clear_key();
        ch.frames.push(Frame {
            kind: FrameKind::Root,
            entry_pc: pc,
            stack: Vec::new(),
        });
        let r = self.dia_exec(ch, it, 0, true);
        self.dia_run_end(it, ch, r)
    }

    /// End of `Dia_RunProgram` once the loop is done (or waits).
    fn dia_run_end(
        &mut self,
        it: &mut Interp,
        ch: &mut Channel,
        r: DR<Flow>,
    ) -> Result<RunResult, (u8, usize)> {
        match r {
            Ok(Flow::Suspend) => return Ok(RunResult::Waiting),
            Ok(Flow::Done) => {}
            Err(c) => {
                ch.error = c;
            }
        }
        ch.frames.clear();
        self.dia_clear_key();
        if ch.rflags & 2 != 0 {
            // A RUn happened: erase the dialog, return its result.
            self.dia_eff_chan(it, ch);
            if ch.error != 0 {
                return Err((ch.error, ch.error_pos));
            }
            return Ok(RunResult::Value(ch.ret as i32));
        }
        self.dia_ed_first(ch);
        self.dia_reactive(ch);
        if ch.error != 0 {
            self.dia_eff_chan(it, ch);
            return Err((ch.error, ch.error_pos));
        }
        Ok(RunResult::Value(0))
    }

    /// One frame of a waiting `RUn` (the loop of `Dia_Run` `+Lib.s:22655`):
    /// continues the program when the dialog exits.
    pub(crate) fn dia_run_resume(
        &mut self,
        it: &mut Interp,
        n: i64,
    ) -> Result<RunResult, (u8, usize)> {
        let Some((i, mut ch)) = self.dialogs.take(n) else {
            return Err((e::CHANNEL_NOT_DEFINED, 0));
        };
        let r = self.dia_run_step(it, &mut ch);
        self.dialogs.put(i, ch);
        r
    }

    fn dia_run_step(
        &mut self,
        it: &mut Interp,
        ch: &mut Channel,
    ) -> Result<RunResult, (u8, usize)> {
        if ch.run.is_none() || ch.frames.is_empty() {
            return Ok(RunResult::Value(ch.ret as i32));
        }
        if let Err(c) = self.dia_active(ch) {
            ch.error = c;
            let r = Err(c);
            return self.dia_run_end(it, ch, r);
        }
        let exit = self.dia_run_tick(it, ch);
        let r = match exit {
            Ok(false) => {
                self.dia_reactive_screen(ch);
                return Ok(RunResult::Waiting);
            }
            Ok(true) => {
                ch.run = None;
                self.dia_exec(ch, it, 0, true)
            }
            Err(c) => Err(c),
        };
        if r == Ok(Flow::Suspend) {
            self.dia_reactive_screen(ch);
        }
        self.dia_run_end(it, ch, r)
    }

    /// Only the screen part of `Dia_ReActive` (between two frames of a
    /// RUn the dialog screen stays as the original leaves it current).
    fn dia_reactive_screen(&mut self, ch: &mut Channel) {
        if let Some(o) = ch.screen_old.take() {
            let _ = self.screens.activate(o);
        }
    }

    /// One iteration of the RUn loop: timer, Control-C, tests. True when
    /// the dialog must exit.
    fn dia_run_tick(&mut self, it: &mut Interp, ch: &mut Channel) -> DR<bool> {
        if let Some(w) = &mut ch.run
            && w.release
        {
            if self.input.mouse_buttons & 1 != 0 {
                return Ok(false);
            }
            w.release = false;
            self.dia_ed_first(ch);
            return Ok(false);
        }
        if ch.timer != 0 && self.vbl_count.saturating_sub(ch.timer_pos) >= ch.timer as u64 {
            ch.ret = 0;
            return Ok(true);
        }
        if self.input.take_break() {
            ch.ret = 0;
            return Ok(true);
        }
        if let Edited::Zone(z) = ch.edited {
            self.dia_ed_active(ch, z);
        }
        Ok(self.dia_tests(it, ch)? != 0)
    }

    // ------------------------------------------------------------------
    // The interpreter loop
    // ------------------------------------------------------------------

    /// Records an error at the current position (`Dia_Er`).
    fn fail<T>(ch: &mut Channel, code: u8) -> DR<T> {
        if ch.error == 0 {
            ch.error = code;
            ch.error_pos = ch.pc;
        }
        Err(code)
    }

    fn chr(ch: &mut Channel) -> (u8, Class) {
        let prog = ch.prog.clone();
        lexer::chr(&prog, &mut ch.pc)
    }

    /// Runs the program until the frame at depth `base` returns (`Done`)
    /// or a `RUn` waits (only when `can_suspend`).
    pub(crate) fn dia_exec(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        base: usize,
        can_suspend: bool,
    ) -> DR<Flow> {
        let mut budget: u32 = 2_000_000;
        let r = self.dia_exec_inner(ch, it, base, can_suspend, &mut budget);
        if r.is_err() {
            ch.frames.truncate(base);
        }
        r
    }

    fn dia_exec_inner(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        base: usize,
        can_suspend: bool,
        budget: &mut u32,
    ) -> DR<Flow> {
        loop {
            if ch.frames.len() <= base {
                return Ok(Flow::Done);
            }
            if ch.error != 0 {
                return Err(ch.error);
            }
            *budget -= 1;
            if *budget == 0 {
                // Endless loop in the Interface program.
                return Self::fail(ch, e::SYNTAX);
            }
            let (c1, cl) = Self::chr(ch);
            if cl == Class::End {
                return Self::fail(ch, e::SYNTAX);
            }
            if c1 == b']' {
                self.dia_frame_return(ch)?;
                continue;
            }
            let (c2, _) = Self::chr(ch);
            let Some(i) = find_instr([c1, c2]) else {
                self.dia_user_call(ch, it, [c1, c2])?;
                continue;
            };
            ch.abuf = ch.pbuf;
            let np = NPARAM[i] as usize;
            let mut a: [DVal; 6] = Default::default();
            if np == 0 {
                Self::chr(ch);
            } else if np < 7 {
                for v in a.iter_mut().take(np) {
                    *v = self.dia_evalue(ch, it)?;
                }
            }
            match self.dia_instr(ch, it, i, a, can_suspend && ch.frames.len() > base) {
                Ok(Some(Flow::Suspend)) => return Ok(Flow::Suspend),
                Ok(_) => {}
                Err(c) => return Self::fail(ch, c),
            }
        }
    }

    /// `Dia_Quit` of a nested loop: restores the position and the stack.
    fn dia_frame_return(&mut self, ch: &mut Channel) -> DR<()> {
        let Some(fr) = ch.frames.pop() else {
            return Ok(());
        };
        ch.pc = fr.entry_pc;
        ch.stack = fr.stack;
        match fr.kind {
            FrameKind::Root => {}
            FrameKind::If => self.dia_skip_block(ch)?,
            FrameKind::User {
                caller_pc,
                stack,
                pusers,
                npusers,
            } => {
                ch.users_depth -= 1;
                ch.npusers = npusers;
                ch.pusers = pusers;
                ch.pc = caller_pc;
                ch.stack = stack;
            }
            FrameKind::Sync { caller_pc } => ch.pc = caller_pc,
        }
        Ok(())
    }

    /// Skips a `[...]` block whose '[' has been read (raw bytes).
    fn dia_skip_block(&mut self, ch: &mut Channel) -> DR<()> {
        let mut depth = 1;
        loop {
            let b = ch.prog.get(ch.pc).copied().unwrap_or(0);
            ch.pc += 1;
            match b {
                0 => return Self::fail(ch, e::SYNTAX),
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }

    /// Call of a user instruction (`.Inst` `+Lib.s:21345`).
    fn dia_user_call(&mut self, ch: &mut Channel, it: &mut Interp, name: [u8; 2]) -> DR<()> {
        let Some(&(_, np, off)) = ch.users.iter().find(|u| u.0 == name) else {
            return Self::fail(ch, e::SYNTAX);
        };
        if ch.users_depth >= 10 {
            return Self::fail(ch, e::FCALL);
        }
        let saved_stack = ch.stack.clone();
        if np == 0 {
            Self::chr(ch);
        } else {
            for _ in 0..np {
                let v = self.dia_evalue(ch, it)?;
                ch.stack.push(SVal::Val(v));
            }
        }
        let caller_pc = ch.pc;
        let kind = FrameKind::User {
            caller_pc,
            stack: saved_stack,
            pusers: ch.pusers,
            npusers: ch.npusers,
        };
        ch.pusers = Some(ch.stack.len() - np as usize);
        ch.npusers = np;
        ch.users_depth += 1;
        ch.frames.push(Frame {
            kind,
            entry_pc: off,
            stack: ch.stack.clone(),
        });
        ch.pc = off;
        Ok(())
    }

    /// Runs a routine of a zone (draw or change) to its end.
    pub(crate) fn dia_call_routine(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        off: usize,
    ) -> DR<()> {
        let base = ch.frames.len();
        ch.frames.push(Frame {
            kind: FrameKind::Sync { caller_pc: ch.pc },
            entry_pc: off,
            stack: ch.stack.clone(),
        });
        ch.pc = off;
        self.dia_exec(ch, it, base, false).map(|_| ())
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    /// `Dia_Evalue`: evaluates one RPN expression (its terminator is read).
    pub(crate) fn dia_evalue(&mut self, ch: &mut Channel, it: &mut Interp) -> DR<DVal> {
        let mut st: Vec<DVal> = Vec::new();
        loop {
            let (c, cl) = Self::chr(ch);
            let func = match cl {
                Class::Num => {
                    let prog = ch.prog.clone();
                    st.push(DVal::Int(lexer::number(&prog, &mut ch.pc, c)));
                    continue;
                }
                Class::Op => lexer::function(c, 0, true),
                Class::End | Class::Sep => {
                    if st.len() != 1 {
                        return Self::fail(ch, e::SYNTAX);
                    }
                    return Ok(st.pop().unwrap());
                }
                Class::Letter => {
                    let (c2, _) = Self::chr(ch);
                    lexer::function(c, c2, false)
                }
            };
            let Some(func) = func else {
                return Self::fail(ch, e::SYNTAX);
            };
            if let Err(code) = self.dia_function(ch, it, func, &mut st) {
                return Self::fail(ch, code);
            }
        }
    }

    fn dia_function(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        func: u8,
        st: &mut Vec<DVal>,
    ) -> DR<()> {
        let need =
            |st: &Vec<DVal>, n: usize, code: u8| if st.len() < n { Err(code) } else { Ok(()) };
        match func {
            f::BX => st.push(DVal::Int(ch.base_x)),
            f::BY => st.push(DVal::Int(ch.base_y)),
            f::SX => st.push(DVal::Int(ch.sx)),
            f::SY => st.push(DVal::Int(ch.sy)),
            f::PLUS | f::MINUS | f::MUL | f::DIV | f::AND | f::OR | f::MI | f::MA => {
                need(st, 2, e::SYNTAX)?;
                let b = int(&st.pop().unwrap())?;
                let a = int(st.last().unwrap())?;
                let r = match func {
                    f::PLUS => a.wrapping_add(b),
                    f::MINUS => a.wrapping_sub(b),
                    f::MUL => (a as i16 as i32) * (b as i16 as i32),
                    f::DIV => {
                        // divs: word divisor, quotient sign extended; on
                        // overflow the dividend is left unchanged.
                        if b == 0 || b as i16 == 0 {
                            return Err(e::SYNTAX);
                        }
                        let a = a as i16 as i32;
                        let q = a / (b as i16 as i32);
                        if (-32768..=32767).contains(&q) { q } else { a }
                    }
                    f::AND => a & b,
                    f::OR => a | b,
                    f::MI => a.min(b),
                    _ => a.max(b),
                };
                *st.last_mut().unwrap() = DVal::Int(r);
            }
            f::EQ | f::NE_OP | f::LT | f::GT => {
                need(st, 2, e::SYNTAX)?;
                let b = st.pop().unwrap();
                let a = st.last().unwrap();
                let r = match (a, &b) {
                    (DVal::Int(a), DVal::Int(b)) => match func {
                        f::EQ => a == b,
                        f::NE_OP => a != b,
                        f::LT => a < b,
                        _ => a > b,
                    },
                    _ => {
                        let eq = match (a, &b) {
                            (DVal::Str(x), DVal::Str(y)) => Rc::ptr_eq(x, y),
                            (DVal::Arr(x), DVal::Arr(y)) => x == y,
                            _ => false,
                        };
                        match func {
                            f::EQ => eq,
                            f::NE_OP => !eq,
                            _ => return Err(e::TYPE),
                        }
                    }
                };
                *st.last_mut().unwrap() = DVal::Int(if r { -1 } else { 0 });
            }
            f::NE => {
                need(st, 1, e::SYNTAX)?;
                let a = int(st.last().unwrap())?;
                *st.last_mut().unwrap() = DVal::Int(a.wrapping_neg());
            }
            f::ME | f::ME2 => {
                need(st, 1, e::SYNTAX)?;
                let n = int(st.last().unwrap())?;
                let m = ch.res.messages.interface(n).ok_or(e::FCALL)?;
                Self::dia_alloc_str(ch, m.len());
                *st.last_mut().unwrap() = mkstr(m);
            }
            f::TW | f::CX => {
                need(st, 1, e::SYNTAX)?;
                let s = string(st.last().unwrap())?;
                let w = gfont::text_length(&s);
                let r = if func == f::CX {
                    ((ch.sx - w).max(0) as u32 >> 1) as i32
                } else {
                    w
                };
                *st.last_mut().unwrap() = DVal::Int(r);
            }
            f::TH => st.push(DVal::Int(gfont::HEIGHT)),
            f::TL => {
                need(st, 1, e::SYNTAX)?;
                let s = string(st.last().unwrap())?;
                *st.last_mut().unwrap() = DVal::Int(s.len() as i32);
            }
            f::VA => {
                need(st, 1, e::SYNTAX)?;
                let n = int(st.last().unwrap())?;
                let v = match n {
                    -1 => ch.v_bp.clone(),
                    -2 => ch.v_br.clone(),
                    -3 => DVal::Int(ch.v_zone.map_or(0, |z| z as i32 + 1)),
                    _ => ch.vars.get(n as usize).cloned().unwrap_or_default(),
                };
                *st.last_mut().unwrap() = v;
            }
            f::STR1 | f::STR2 => {
                let q = if func == f::STR1 { b'"' } else { b'\'' };
                let mut s = Vec::new();
                loop {
                    let b = ch.prog.get(ch.pc).copied().unwrap_or(0);
                    ch.pc += 1;
                    if b < 32 {
                        ch.pc -= 1;
                        break;
                    }
                    if b == q {
                        break;
                    }
                    s.push(b);
                }
                Self::dia_alloc_str(ch, s.len());
                st.push(mkstr(s));
            }
            f::SW | f::SH => {
                let s = self.screens.current();
                let v = s.map_or(0, |s| if func == f::SW { s.width } else { s.height });
                st.push(DVal::Int(v as i32));
            }
            f::BP | f::ZP => st.push(ch.v_bp.clone()),
            f::DEC => {
                need(st, 1, e::SYNTAX)?;
                let n = int(st.last().unwrap())?;
                let s = dec(n);
                Self::dia_alloc_str(ch, s.len());
                *st.last_mut().unwrap() = mkstr(s);
            }
            f::CONCAT => {
                need(st, 2, e::SYNTAX)?;
                let b = string(&st.pop().unwrap())?;
                let a = string(st.last().unwrap())?;
                let mut s = a.to_vec();
                s.extend_from_slice(&b);
                Self::dia_alloc_str(ch, s.len());
                *st.last_mut().unwrap() = mkstr(s);
            }
            f::MZ => {
                need(st, 2, e::SYNTAX)?;
                let max = int(&st.pop().unwrap())?;
                let s = match st.last().unwrap() {
                    DVal::Int(a) => {
                        let mut s = Vec::new();
                        let mut a = *a as u32;
                        let mut n = max;
                        loop {
                            let b = self.banks.peek(a);
                            a = a.wrapping_add(1);
                            if b < 32 {
                                break;
                            }
                            s.push(b);
                            n -= 1;
                            if n <= 0 {
                                break;
                            }
                        }
                        s
                    }
                    // A string given by the host (the editor passes the
                    // text where the original gives its address).
                    DVal::Str(t) => t
                        .iter()
                        .take_while(|&&b| b >= 32)
                        .take(max.max(1) as usize)
                        .copied()
                        .collect(),
                    _ => Vec::new(),
                };
                Self::dia_alloc_str(ch, s.len());
                *st.last_mut().unwrap() = mkstr(s);
            }
            f::XA => st.push(DVal::Int(ch.xa as i32)),
            f::YA => st.push(DVal::Int(ch.ya as i32)),
            f::XB => st.push(DVal::Int(ch.xb as i32)),
            f::YB => st.push(DVal::Int(ch.yb as i32)),
            f::AR => {
                need(st, 2, e::NPARAM)?;
                let i = int(&st.pop().unwrap())?;
                let a = match st.last().unwrap() {
                    DVal::Arr(a) => *a,
                    _ => return Err(e::FCALL),
                };
                let v = self.dia_array_get(it, a, i).ok_or(e::FCALL)?;
                *st.last_mut().unwrap() = v;
            }
            f::AS => {
                need(st, 1, e::NPARAM)?;
                let n = match st.last().unwrap() {
                    DVal::Int(0) => 0,
                    DVal::Arr(a) => self.dia_array_size(it, *a).ok_or(e::FCALL)?,
                    _ => return Err(e::FCALL),
                };
                *st.last_mut().unwrap() = DVal::Int(n);
            }
            f::P1..=f::P9 => {
                let n = (func - f::P1 + 1) as u16;
                let base = ch.pusers.ok_or(e::SYNTAX)?;
                if n > ch.npusers {
                    return Err(e::SYNTAX);
                }
                match ch.stack.get(base + n as usize - 1) {
                    Some(SVal::Val(v)) => st.push(v.clone()),
                    _ => return Err(e::SYNTAX),
                }
            }
            f::ZV => {
                let v = match ch.v_zone.and_then(|z| ch.zone(z)) {
                    Some(z) => z.var.clone(),
                    None => ch.next_zone.clone(),
                };
                st.push(v);
            }
            f::ZN => {
                let z = ch.v_zone.and_then(|z| ch.zone(z)).ok_or(e::SYNTAX)?;
                st.push(DVal::Int(z.number as u16 as i32));
            }
            _ => return Err(e::SYNTAX),
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Arrays
    // ------------------------------------------------------------------

    /// Number of elements of an array given to the Interface.
    pub(crate) fn dia_array_size(&mut self, it: &mut Interp, a: ArrRef) -> Option<i32> {
        match a {
            ArrRef::Fsel => Some(
                self.dialogs
                    .fsel
                    .as_ref()
                    .map_or(0, |f| f.list.len() as i32),
            ),
            ArrRef::Basic { slot, frame } => {
                let loc = crate::interp::VarLoc {
                    slot,
                    index: None,
                    frame,
                };
                let arr = it.array_mut(&loc).ok()?;
                if arr.dims.len() != 1 {
                    return None;
                }
                Some(arr.dims[0] as i32 + 1)
            }
        }
    }

    /// Element `i` of an array (strings, integers).
    pub(crate) fn dia_array_get(&mut self, it: &mut Interp, a: ArrRef, i: i32) -> Option<DVal> {
        let n = self.dia_array_size(it, a)?;
        if i as u32 >= n as u32 {
            return None;
        }
        match a {
            ArrRef::Fsel => self
                .dialogs
                .fsel
                .as_ref()
                .map(|f| DVal::str(&f.display(i as usize))),
            ArrRef::Basic { slot, frame } => {
                let loc = crate::interp::VarLoc {
                    slot,
                    index: None,
                    frame,
                };
                let arr = it.array_mut(&loc).ok()?;
                Some(match arr.get(i as usize) {
                    crate::interp::value::Value::Str(s) => DVal::Str(Rc::from(&s[..])),
                    crate::interp::value::Value::Int(n) => DVal::Int(n),
                    crate::interp::value::Value::Float(f) => {
                        DVal::Int(crate::interp::value::float_to_int(f))
                    }
                })
            }
        }
    }

    // ------------------------------------------------------------------
    // Drawing helpers
    // ------------------------------------------------------------------

    /// Runs a drawing operation on the dialog screen's logic bitmap.
    pub(crate) fn dia_draw(&mut self, ch: &Channel, f: impl FnOnce(&mut GrState, &mut Canvas)) {
        if let Some(s) = self.screens.get_mut(ch.screen) {
            let (w, h, planes, li) = (s.width, s.height, s.planes, s.logic);
            let mut c = Canvas::new(&mut s.bitmaps[li], w, h, planes);
            f(&mut s.gr, &mut c);
            s.version += 1;
        }
    }

    fn dia_gr(&mut self, ch: &Channel) -> Option<&mut GrState> {
        self.screens.get_mut(ch.screen).map(|s| &mut s.gr)
    }

    /// `Dia_Text`: prints a string at (x, y) relative to the base.
    fn dia_text(&mut self, ch: &mut Channel, x: i32, y: i32, t: &DVal, ink: i32) -> DR<()> {
        let t = string(t)?;
        if ink >= 0
            && let Some(g) = self.dia_gr(ch)
        {
            g.ink = ink as u8;
        }
        ch.xa = x as i16;
        ch.ya = y as i16;
        ch.xb = x as i16;
        ch.yb = y as i16;
        let (px, py) = (
            x + ch.base_x,
            (y + ch.base_y) as i16 as i32 + gfont::BASELINE,
        );
        if !t.is_empty() {
            self.dia_draw(ch, |g, c| {
                g.x = px;
                g.y = py;
                let w = g.text(c, px, py, &t);
                g.x = px + w;
            });
            ch.yb = ch.yb.wrapping_add(gfont::HEIGHT as i16);
            ch.xb = ch.xb.wrapping_add(gfont::text_length(&t) as i16);
        }
        Ok(())
    }

    /// `Dia_Unpack`: unpacks image `i` (+ PU offset) at (x, y) relative to
    /// the base.
    fn dia_unpack(&mut self, ch: &mut Channel, x: i32, y: i32, i: i32) -> DR<()> {
        let n = i.wrapping_add(ch.puzzle_i);
        if n == 0 || (n as u16 as usize) > ch.res.graphics.count() {
            return Err(e::SYNTAX);
        }
        let g = ch.res.graphics.clone();
        let img = g.image(n as u16 as usize).unwrap_or(&[]);
        ch.xa = x as i16;
        ch.ya = y as i16;
        ch.xb = x as i16;
        ch.yb = y as i16;
        let (px, py) = (x + ch.base_x, y + ch.base_y);
        let mut size = (0, 0);
        if let Some(s) = self.screens.get_mut(ch.screen)
            && px >= 0
            && py >= 0
        {
            let (w, h, planes, li) = (
                s.width as usize,
                s.height as usize,
                s.planes as usize,
                s.logic,
            );
            if let Some(r) = pack::unpack_bitmap(img, &mut s.bitmaps[li], w, h, planes, px, py) {
                size = r;
                s.version += 1;
            }
        }
        ch.puzzle_sx = size.0 as i32;
        ch.puzzle_sy = size.1 as i32;
        ch.xb = ch.xb.wrapping_add(size.0 as i16);
        ch.yb = ch.yb.wrapping_add(size.1 as i16);
        Ok(())
    }

    /// `Dia_Line`: horizontal line made of images i (left), i+1 (middle,
    /// repeated), i+2 (right). X coordinates are multiples of 8.
    fn dia_line(&mut self, ch: &mut Channel, x: i32, y: i32, i: i32, x2: i32) -> DR<()> {
        let (x, x2) = (x & !7, x2 & !7);
        let mut d7 = x2 - x;
        if d7 <= 0 {
            return Err(e::FCALL);
        }
        let mut d2 = x;
        loop {
            self.dia_unpack(ch, d2, y, i + 1)?;
            let w = ch.puzzle_sx;
            if w == 0 {
                break;
            }
            d2 += w;
            d7 -= w;
            if d7 < w {
                break;
            }
        }
        self.dia_unpack(ch, x, y, i)?;
        self.dia_unpack(ch, x2 - ch.puzzle_sx, y, i + 2)?;
        ch.xa = (x & !7) as i16;
        ch.ya = y as i16;
        Ok(())
    }

    /// `Dia_VLine`: vertical line made of images i, i+1, i+2.
    fn dia_vline(&mut self, ch: &mut Channel, x: i32, y: i32, i: i32, y2: i32) -> DR<()> {
        let x = x & !7;
        let mut d7 = y2 - y;
        if d7 <= 0 {
            return Err(e::FCALL);
        }
        let mut d3 = y;
        loop {
            self.dia_unpack(ch, x, d3, i + 1)?;
            let h = ch.puzzle_sy;
            if h == 0 {
                break;
            }
            d3 += h;
            d7 -= h;
            if d7 < h {
                break;
            }
        }
        self.dia_unpack(ch, x, y, i)?;
        self.dia_unpack(ch, x, y2 - ch.puzzle_sy, i + 2)?;
        ch.xa = (x & !7) as i16;
        ch.ya = y as i16;
        Ok(())
    }

    /// `Dia_Box`: box made of 9 images (i..i+8): top line, middle line
    /// copied downwards, bottom line.
    fn dia_box(&mut self, ch: &mut Channel, x: i32, y: i32, i: i32, x2: i32, y2: i32) -> DR<()> {
        let mut d7 = y2 - y;
        if d7 <= 0 {
            return Err(e::FCALL);
        }
        self.dia_line(ch, x, y, i + 3, x2)?;
        let h = ch.puzzle_sy;
        if h != 0 {
            d7 -= h;
            if d7 > 0 {
                let x0 = x & !7;
                let mut d3 = y;
                loop {
                    let (bx, by) = (ch.base_x, ch.base_y);
                    self.dia_sc_copy(
                        ch.screen,
                        x0 + bx,
                        d3 + by,
                        x2 + bx,
                        d3 + h + by,
                        x0 + bx,
                        d3 + h + by,
                    );
                    d3 += h;
                    d7 -= h;
                    if d7 < h {
                        break;
                    }
                }
            }
        }
        self.dia_line(ch, x, y, i, x2)?;
        self.dia_line(ch, x, y2 - ch.puzzle_sy, i + 6, x2)?;
        ch.xa = (x & !7) as i16;
        ch.ya = y as i16;
        Ok(())
    }

    /// `Dia_ScCopy` (`Sco0`): copies (x1,y1)-(x2,y2) to (x3,y3) in the same
    /// screen.
    #[allow(clippy::too_many_arguments)]
    fn dia_sc_copy(&mut self, sn: usize, x1: i32, y1: i32, x2: i32, y2: i32, x3: i32, y3: i32) {
        let Some(s) = self.screens.get_mut(sn) else {
            return;
        };
        let (w, h) = (s.width as i32, s.height as i32);
        let Some(r) = crate::gfx::blocks::clip_copy(w, h, w, h, x1, y1, x2, y2, x3, y3) else {
            return;
        };
        let li = s.logic;
        let data = crate::gfx::blocks::extract(&s.bitmaps[li], w, &r);
        let planes = s.planes;
        crate::gfx::blocks::blit(&data, s.bitmap_mut(li), w, &r, 0xCC, planes);
    }

    /// `Dia_Block2`: saves the dialog background in block `n`.
    fn dia_grab(&mut self, ch: &mut Channel, n: i32) -> DR<()> {
        let x = ch.base_x & !15;
        let w = (ch.sx + (ch.base_x - x) + 15) & !15;
        let (y, h) = (ch.base_y, ch.sy);
        let Some(s) = self.screens.get(ch.screen) else {
            return Err(e::SYNTAX);
        };
        let (tx, ty) = (s.width as i32, s.height as i32);
        if n == 0 || x < 0 || y < 0 || w <= 0 || h <= 0 || x + w > tx || y + h > ty {
            return Err(e::SYNTAX);
        }
        let b = crate::gfx::blocks::grab_block(s.logic_ref(), tx, n, x, y, w, h, s.planes, false);
        if let Some(old) = self.draw.blocks.iter_mut().find(|o| o.number == n) {
            *old = b;
        } else {
            self.draw.blocks.insert(0, b);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Instructions
    // ------------------------------------------------------------------

    fn dia_instr(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        i: usize,
        a: [DVal; 6],
        can_suspend: bool,
    ) -> DR<Option<Flow>> {
        let n = |k: usize| int(&a[k]);
        match i {
            0 => self.dia_frame_return(ch)?,
            1 => self.dia_unpack(ch, n(0)?, n(1)?, n(2)?)?,
            2 => self.dia_line(ch, n(0)?, n(1)?, n(2)?, n(3)?)?,
            3 => self.dia_box(ch, n(0)?, n(1)?, n(2)?, n(3)?, n(4)?)?,
            4 => {
                ch.sx = (n(0)? + 15) & !15;
                ch.sy = n(1)?;
            }
            5 => {
                ch.base_x = n(0)? & !15;
                ch.base_y = n(1)?;
            }
            6 => ch.puzzle_i = n(0)?,
            7 => {
                let k = n(0)?;
                let v = a[1].clone();
                match k {
                    -1 => ch.v_bp = v,
                    -2 => ch.v_br = v,
                    _ => {
                        if let Some(slot) = ch.vars.get_mut(k as usize) {
                            *slot = v;
                        }
                    }
                }
                ch.pbuf = ch.abuf;
            }
            // "IL": a 68000 ILLEGAL instruction in the original.
            8 => return Err(e::SYNTAX),
            9 => self.dia_text(ch, n(0)?, n(1)?, &a[2], n(3)?)?,
            10 => {
                ch.writing = 0;
                if let Some(g) = self.dia_gr(ch) {
                    g.writing = 0;
                }
                let (x, y, o, ink) = (n(0)?, n(1)?, n(3)?, n(4)?);
                self.dia_text(ch, x - 1, y, &a[2], o)?;
                self.dia_text(ch, x + 1, y, &a[2], -1)?;
                self.dia_text(ch, x, y - 1, &a[2], -1)?;
                self.dia_text(ch, x, y + 1, &a[2], -1)?;
                self.dia_text(ch, x, y, &a[2], ink)?;
            }
            11 => {
                let (ia, ib, ic) = (n(0)?, n(1)?, n(2)?);
                if let Some(g) = self.dia_gr(ch) {
                    if ic >= 0 {
                        g.outline = ic as u8;
                    }
                    if ib >= 0 {
                        g.paper = ib as u8;
                    }
                    if ia >= 0 {
                        g.ink = ia as u8;
                    }
                }
            }
            12 => {
                let (font, style) = (n(0)?, n(1)?);
                if let Some(g) = self.dia_gr(ch) {
                    if font >= 0 {
                        g.font = font;
                    }
                    if style >= 0 {
                        g.text_style = style as u8;
                    }
                }
            }
            13 => {
                let m = n(0)?;
                if m >= 0 {
                    ch.writing = m as u8;
                    if let Some(g) = self.dia_gr(ch) {
                        g.writing = m as u8;
                    }
                }
            }
            14 => {
                let p = n(0)?;
                if p >= 0
                    && let Some(g) = self.dia_gr(ch)
                {
                    g.line_pattern = p as u16;
                }
            }
            15 => {
                // The outline flag goes to an unused RastPort bit.
                let p = n(0)? as i16 as i32;
                let _ = n(1)?;
                let pat = if p > 0 {
                    crate::gfx::draw::builtin_pattern(p as usize)
                } else {
                    None
                };
                if let Some(g) = self.dia_gr(ch) {
                    g.pattern_number = p;
                    g.pattern = pat;
                }
            }
            16 => self.dia_button(ch, it)?,
            17 | I_LA => self.dia_goto_label(ch, n(0)?),
            18 => return self.dia_run_instr(ch, n(0)?, n(1)?, can_suspend).map(Some),
            19 => ch.v_br = a[0].clone(),
            20 => self.dia_edit(ch, it, false)?,
            21 => {
                ch.stack.push(SVal::Ret(ch.pc));
                self.dia_goto_label(ch, n(0)?);
            }
            22 => match ch.stack.pop() {
                Some(SVal::Ret(p)) => ch.pc = p,
                _ => return Err(e::SYNTAX),
            },
            23 => self.dia_bchange(ch, it, n(0)?, n(1)?)?,
            24 => {
                Self::dia_alloc(ch, size::KEY)?;
                ch.records.push(Record::Key {
                    code: n(0)? as u8,
                    shift: n(1)? as u8,
                    zone: ch.last_zone,
                });
                ch.pbuf = ch.abuf;
            }
            25 => {
                Self::dia_alloc(ch, size::BLOCK)?;
                let b = n(0)?;
                ch.records.push(Record::Block(b));
                ch.pbuf = ch.abuf;
                self.dia_grab(ch, b)?;
            }
            26 => self.dia_edit(ch, it, true)?,
            27 => self.dia_grab(ch, n(0)?)?,
            29 | 42 => {
                let z = ch.v_zone.ok_or(e::SYNTAX)?;
                if let Some(z) = ch.zone_mut(z) {
                    z.flags |= if i == 29 { 0x80 } else { 0x20 };
                }
            }
            30 | 31 => self.dia_slider(ch, it, i == 31)?,
            32 | 33 => self.dia_list(ch, it)?,
            34 => {
                if ch.v_zone.is_none() {
                    return Err(e::SYNTAX);
                }
                self.dia_zupdate(ch, it, n(0)?, Some(a[1].clone()), None, None)?;
            }
            I_UI => return Err(e::SYNTAX),
            36 => {
                let (x1, y1, x2, y2) = (n(0)?, n(1)?, n(2)?, n(3)?);
                ch.xa = x1 as i16;
                ch.ya = y1 as i16;
                ch.xb = x2 as i16;
                ch.yb = y2 as i16;
                let (x1, y1, x2, y2) = (
                    x1 + ch.base_x,
                    y1 + ch.base_y,
                    x2 + ch.base_x,
                    y2 + ch.base_y,
                );
                if x2 <= x1 || y2 <= y1 {
                    return Err(e::FCALL);
                }
                self.dia_draw(ch, |g, c| g.bar(c, x1, y1, x2, y2));
            }
            37 => {
                let (x1, y1, x2, y2) = (n(0)?, n(1)?, n(2)?, n(3)?);
                ch.xa = x1 as i16;
                ch.ya = y1 as i16;
                ch.xb = x2 as i16;
                ch.yb = y2 as i16;
                let (x1, y1, x2, y2) = (
                    x1 + ch.base_x,
                    y1 + ch.base_y,
                    x2 + ch.base_x,
                    y2 + ch.base_y,
                );
                Self::dia_alloc(ch, 16)?;
                self.dia_draw(ch, |g, c| {
                    g.x = x1;
                    g.y = y1;
                    let ink = g.ink;
                    g.poly_draw(c, &[(x1, y2), (x2, y2), (x2, y1), (x1, y1)], true, ink);
                });
            }
            38 => {
                let (x1, y1, x2, y2) = (n(0)?, n(1)?, n(2)?, n(3)?);
                ch.xa = x1 as i16;
                ch.ya = y1 as i16;
                ch.xb = x2 as i16;
                ch.yb = y2 as i16;
                let (x1, y1, x2, y2) = (
                    x1 + ch.base_x,
                    y1 + ch.base_y,
                    x2 + ch.base_x,
                    y2 + ch.base_y,
                );
                self.dia_draw(ch, |g, c| {
                    g.x = x1;
                    g.y = y1;
                    g.draw_to(c, x2, y2);
                });
            }
            39 => {
                if Self::chr(ch).0 != b'[' {
                    return Err(e::SYNTAX);
                }
                if truth(&a[0]) {
                    ch.frames.push(Frame {
                        kind: FrameKind::If,
                        entry_pc: ch.pc,
                        stack: ch.stack.clone(),
                    });
                } else {
                    self.dia_skip_block(ch)?;
                }
            }
            40 => ch.next_zone = a[0].clone(),
            41 => {
                ch.xa = n(0)? as i16;
                ch.ya = n(1)? as i16;
                ch.xb = n(2)? as i16;
                ch.yb = n(3)? as i16;
            }
            43 => self.dia_vtext(ch, n(0)?, n(1)?, &a[2], n(3)?)?,
            44 => self.dia_vline(ch, n(0)?, n(1)?, n(2)?, n(3)?)?,
            45 => self.dia_hypertext(ch, it)?,
            // CAll: machine code, not available.
            46 => {}
            47 => {
                // SM: the screen follows the mouse while the button is held
                // (done by the button hold, see `zones.rs`).
                let my = self.input.mouse_y;
                let wy = self.screens.get(ch.screen).map_or(0, |s| s.display_y);
                self.dialogs.screen_move = Some((ch.screen, my - wy));
            }
            48 => {
                let (x, y, r1, r2) = (n(0)?, n(1)?, n(2)?, n(3)?);
                if r1 <= 0 || r2 <= 0 {
                    return Err(e::FCALL);
                }
                ch.xa = x as i16;
                ch.ya = x as i16;
                ch.xb = y as i16;
                ch.yb = y as i16;
                let (cx, cy) = (x + ch.base_x, y + ch.base_y);
                self.dia_draw(ch, |g, c| g.ellipse(c, cx, cy, r1, r2));
            }
            49 => {
                let (x, y, ink) = (n(0)?, n(1)?, n(2)?);
                if ink >= 0
                    && let Some(g) = self.dia_gr(ch)
                {
                    g.ink = ink as u8;
                }
                ch.xa = x as i16;
                ch.ya = x as i16;
                ch.xb = y.wrapping_add(1) as i16;
                ch.yb = y.wrapping_add(1) as i16;
                self.dia_draw(ch, |g, c| g.plot(c, x, y));
            }
            50 => {
                for k in 0..8 {
                    let v = self.dia_evalue(ch, it)?;
                    ch.sl_default[k] = int(&v)? as u8;
                }
                let (a, b) = ch.sl_default.split_at_mut(8);
                b.copy_from_slice(a);
            }
            _ => return Err(e::SYNTAX),
        }
        Ok(None)
    }

    /// `Dia_Label`: jumps to label `n` (to the start of the program if it
    /// does not exist, as the original).
    fn dia_goto_label(&mut self, ch: &mut Channel, n: i32) {
        ch.pc = ch.labels.iter().find(|l| l.0 == n).map_or(0, |l| l.1);
    }

    /// `Dia_VText`: vertical text.
    fn dia_vtext(&mut self, ch: &mut Channel, x: i32, y: i32, t: &DVal, ink: i32) -> DR<()> {
        let t = string(t)?;
        if ink >= 0
            && let Some(g) = self.dia_gr(ch)
        {
            g.ink = ink as u8;
        }
        ch.xa = x as i16;
        ch.ya = y as i16;
        ch.xb = x as i16;
        ch.yb = y as i16;
        let px = x + ch.base_x;
        let mut py = y + ch.base_y + gfont::BASELINE;
        for &c in t.iter() {
            self.dia_draw(ch, |g, cv| {
                g.text(cv, px, py, &[c]);
            });
            py += gfont::HEIGHT;
            ch.xb = ch.xb.wrapping_add(gfont::HEIGHT as i16);
        }
        Ok(())
    }

    /// `Dia_Run` (`RU timer,flags`).
    fn dia_run_instr(
        &mut self,
        ch: &mut Channel,
        timer: i32,
        flags: i32,
        can_suspend: bool,
    ) -> DR<Flow> {
        ch.flags = flags as u8;
        ch.timer = timer as u32;
        ch.timer_pos = self.vbl_count;
        ch.rflags |= 2;
        if flags & 1 != 0 {
            self.input.clear_keys();
        }
        if !can_suspend {
            // Inside a zone routine: no wait.
            return Ok(Flow::Done);
        }
        ch.run = Some(RunWait {
            release: flags & 2 != 0,
        });
        if flags & 2 == 0 {
            self.dia_ed_first(ch);
        }
        Ok(Flow::Suspend)
    }

    // ------------------------------------------------------------------
    // Zone creation
    // ------------------------------------------------------------------

    /// `Dia_GetEntete`: zone header (number, x, y, sx, sy).
    fn dia_zone_header(&mut self, ch: &mut Channel, it: &mut Interp, len: usize) -> DR<Zone> {
        Self::dia_alloc(ch, len)?;
        let number = int(&self.dia_evalue(ch, it)?)?;
        let x = int(&self.dia_evalue(ch, it)?)?;
        let y = int(&self.dia_evalue(ch, it)?)?;
        let sx = int(&self.dia_evalue(ch, it)?)?;
        let sy = int(&self.dia_evalue(ch, it)?)?;
        ch.xa = x as i16;
        ch.ya = y as i16;
        ch.xb = (x + sx) as i16;
        ch.yb = (y + sy) as i16;
        Ok(Zone {
            number: number as i16,
            x: (x + ch.base_x) as i16,
            y: (y + ch.base_y) as i16,
            sx: sx as i16,
            sy: sy as i16,
            rchange: 0,
            pos: 0,
            var: ch.next_zone.clone(),
            flags: 0,
            kind: ZoneKind::Button {
                rdraw: 0,
                min: 0,
                max: 0,
            },
        })
    }

    /// `Dia_EdDiRout`: header of edit, digit, list and hypertext zones.
    /// Returns the zone and the width in characters.
    fn dia_zone_text_header(&mut self, ch: &mut Channel, it: &mut Interp) -> DR<(Zone, i32)> {
        let number = int(&self.dia_evalue(ch, it)?)?;
        let x = int(&self.dia_evalue(ch, it)?)?;
        ch.xa = x as i16;
        ch.xb = x as i16;
        let zx = (x + ch.base_x) & 0xFFF0;
        let y = int(&self.dia_evalue(ch, it)?)?;
        ch.ya = y as i16;
        ch.yb = y as i16;
        let zy = y + ch.base_y;
        let w = (int(&self.dia_evalue(ch, it)?)? + 1) as i16 & !1;
        if w == 0 {
            return Err(e::SYNTAX);
        }
        ch.xb = ch.xb.wrapping_add(w);
        ch.yb = ch.yb.wrapping_add(8);
        let z = Zone {
            number: number as i16,
            x: zx as i16,
            y: zy as i16,
            sx: w.wrapping_mul(8),
            sy: 8,
            rchange: 0,
            pos: 0,
            var: ch.next_zone.clone(),
            flags: 0,
            kind: ZoneKind::Button {
                rdraw: 0,
                min: 0,
                max: 0,
            },
        };
        Ok((z, w as i32))
    }

    /// `Dia_GetRout`: reads a `[routine]`; returns its offset (0 if empty).
    fn dia_get_rout(&mut self, ch: &mut Channel) -> DR<usize> {
        if Self::chr(ch).0 != b'[' {
            return Err(e::SYNTAX);
        }
        let start = ch.pc;
        let (c, cl) = Self::chr(ch);
        if cl == Class::End {
            return Err(e::SYNTAX);
        }
        if c == b']' {
            return Ok(0);
        }
        self.dia_skip_block(ch)?;
        Ok(start)
    }

    /// `Dia_Button`: BU z,x,y,sx,sy,pos,min,max;[draw][change].
    fn dia_button(&mut self, ch: &mut Channel, it: &mut Interp) -> DR<()> {
        let mut z = self.dia_zone_header(ch, it, size::BUTTON)?;
        z.pos = int(&self.dia_evalue(ch, it)?)?;
        let min = int(&self.dia_evalue(ch, it)?)? as i16;
        let max = int(&self.dia_evalue(ch, it)?)? as i16;
        ch.pbuf = ch.abuf;
        let rdraw = self.dia_get_rout(ch)?;
        z.rchange = self.dia_get_rout(ch)?;
        z.kind = ZoneKind::Button { rdraw, min, max };
        ch.records.push(Record::Zone(z));
        let idx = ch.records.len() - 1;
        ch.last_zone = Some(idx);
        self.dia_bt_draw(ch, it, idx)
    }

    /// `Dia_Edit` / `Dia_Digit`: ED z,x,y,sx,maxlen,'default',paper,pen and
    /// DI z,x,y,sx,value,flag,paper,pen.
    fn dia_edit(&mut self, ch: &mut Channel, it: &mut Interp, digit: bool) -> DR<()> {
        Self::dia_alloc(ch, if digit { size::DIGIT } else { size::EDIT })?;
        let (mut z, w) = self.dia_zone_text_header(ch, it)?;
        let (text, max, flags, value) = if digit {
            let value = int(&self.dia_evalue(ch, it)?)?;
            let flag = int(&self.dia_evalue(ch, it)?)?;
            let t = if flag & 1 != 0 {
                dec(value)
            } else {
                Vec::new()
            };
            (
                t,
                (w - 1).max(0) as usize,
                led::MOUSE_CURSOR | led::MOUSE | led::ONCE | led::KEYS | led::FILTER,
                Some(value),
            )
        } else {
            let max = int(&self.dia_evalue(ch, it)?)?;
            if max == 0 || max as u32 > 1024 {
                return Err(e::FCALL);
            }
            Self::dia_alloc(ch, max as usize + 6)?;
            // A null address (0) gives an empty line.
            let t = match self.dia_evalue(ch, it)? {
                DVal::Int(0) => Vec::new(),
                v => string(&v)?.to_vec(),
            };
            (
                t,
                max as usize,
                led::MOUSE_CURSOR | led::MOUSE | led::ONCE | led::KEYS,
                None,
            )
        };
        // Dia_Edit2: copies the default text (stops at a control code).
        let buf: Vec<u8> = text
            .iter()
            .copied()
            .take(max)
            .filter(|&c| c >= 32)
            .collect();
        let paper = int(&self.dia_evalue(ch, it)?)?;
        let pen = int(&self.dia_evalue(ch, it)?)?;
        let wn = z.number as i32 + 1000;
        ch.wind_on = wn;
        let (zx, zy) = (z.x as i32, z.y as i32);
        if let Some(s) = self.screens.get_mut(ch.screen) {
            let _ = s.wind_open(wn, zx, zy, w, 1, 0, false);
            let init = win_init(paper, pen);
            let _ = s.print_text(&init);
        }
        let mut led = LineEd {
            buf,
            max,
            flags,
            ..Default::default()
        };
        if digit {
            led.mask = [0x0015FFC0, 0, 0];
        }
        z.kind = ZoneKind::Edit { led, digit: value };
        ch.records.push(Record::Zone(z));
        let idx = ch.records.len() - 1;
        ch.last_zone = Some(idx);
        if !self.dia_led_init(ch, idx) {
            return Err(e::FCALL);
        }
        ch.pbuf = ch.abuf;
        Ok(())
    }

    /// `Dia_Slider`: HS / VS z,x,y,sx,sy,pos,size,total,step;[change].
    fn dia_slider(&mut self, ch: &mut Channel, it: &mut Interp, vertical: bool) -> DR<()> {
        let mut z = self.dia_zone_header(ch, it, size::SLIDER)?;
        let mut sl = slider::Slider {
            x: z.x as i32,
            y: z.y as i32,
            sx: z.sx as i32 - 1,
            sy: z.sy as i32 - 1,
            vertical,
            ..Default::default()
        };
        sl.start = if vertical { sl.y } else { sl.x };
        sl.size = if vertical { sl.sy } else { sl.sx };
        let pos = int(&self.dia_evalue(ch, it)?)?;
        sl.position = pos as u16 as i32;
        z.pos = pos;
        sl.window = int(&self.dia_evalue(ch, it)?)? as u16 as i32;
        sl.global = int(&self.dia_evalue(ch, it)?)? as u16 as i32;
        sl.scroll = int(&self.dia_evalue(ch, it)?)? as u16 as i32;
        for k in 0..8 {
            sl.inactive[k] = ch.sl_default[k] as i32;
            sl.active[k] = ch.sl_default[8 + k] as i32;
        }
        ch.pbuf = ch.abuf;
        z.rchange = self.dia_get_rout(ch)?;
        z.kind = ZoneKind::Slider(sl);
        ch.records.push(Record::Zone(z));
        let idx = ch.records.len() - 1;
        self.dia_sl_draw(ch, idx, false);
        Ok(())
    }

    /// `Dia_List`: AL z,x,y,sx,sy,array,first,flags,paper,pen;[change].
    fn dia_list(&mut self, ch: &mut Channel, it: &mut Interp) -> DR<()> {
        Self::dia_alloc(ch, size::LIST)?;
        let (mut z, w) = self.dia_zone_text_header(ch, it)?;
        let ty = int(&self.dia_evalue(ch, it)?)?;
        z.sy = (ty as i16).wrapping_mul(8);
        let arr = self.dia_evalue(ch, it)?;
        let (array, count) = match arr {
            DVal::Int(0) => (None, 0),
            DVal::Arr(a) => {
                let n = self.dia_array_size(it, a).ok_or(e::FCALL)?;
                if n >= 32768 {
                    return Err(e::FCALL);
                }
                (Some(a), n)
            }
            _ => return Err(e::FCALL),
        };
        let mut pos = int(&self.dia_evalue(ch, it)?)?;
        if pos as u16 >= count as u16 {
            pos = count;
        }
        z.pos = -1;
        let wn = z.number as i32 + 2000;
        ch.wind_on = wn;
        let (zx, zy) = (z.x as i32, z.y as i32);
        if let Some(s) = self.screens.get_mut(ch.screen) {
            s.wind_open(wn, zx, zy, w, ty, 0, false)
                .map_err(|_| e::FCALL)?;
        } else {
            return Err(e::FCALL);
        }
        let lflags = int(&self.dia_evalue(ch, it)?)? as u8;
        let paper = int(&self.dia_evalue(ch, it)?)?;
        let pen = int(&self.dia_evalue(ch, it)?)?;
        if let Some(s) = self.screens.get_mut(ch.screen) {
            let _ = s.print_text(&win_init(paper, pen));
        }
        ch.pbuf = ch.abuf;
        z.rchange = self.dia_get_rout(ch)?;
        z.kind = ZoneKind::List(ListZone {
            tx: w as i16,
            ty: ty as i16,
            pos: pos as i16,
            max_act: count as i16,
            array,
            larray: dec(count).len() as i16,
            act: -1,
            lflags,
        });
        ch.records.push(Record::Zone(z));
        let idx = ch.records.len() - 1;
        self.dia_li_draw(ch, it, idx);
        Ok(())
    }

    /// Text of a hypertext zone: a string, or zero terminated text at an
    /// address (`Start(bank)`).
    fn dia_text_source(&self, v: &DVal) -> DR<Rc<[u8]>> {
        match v {
            DVal::Str(s) => Ok(s.clone()),
            DVal::Int(a) if *a > 1024 => {
                let mut out = Vec::new();
                let mut a = *a as u32;
                loop {
                    let b = self.banks.peek(a);
                    if b == 0 || out.len() >= 1 << 22 {
                        break;
                    }
                    out.push(b);
                    a = a.wrapping_add(1);
                }
                Ok(Rc::from(out))
            }
            _ => Err(e::FCALL),
        }
    }

    /// `Dia_HyperText`: HT z,x,y,sx,sy,text,pos,zones,paper,pen;[change].
    fn dia_hypertext(&mut self, ch: &mut Channel, it: &mut Interp) -> DR<()> {
        Self::dia_alloc(ch, size::TEXT)?;
        let (mut z, w) = self.dia_zone_text_header(ch, it)?;
        let ty = int(&self.dia_evalue(ch, it)?)?;
        z.sy = (ty as i16).wrapping_mul(8);
        let tv = self.dia_evalue(ch, it)?;
        let text = self.dia_text_source(&tv)?;
        let pos = int(&self.dia_evalue(ch, it)?)?;
        let wn = z.number as i32 + 3000;
        ch.wind_on = wn;
        let (zx, zy) = (z.x as i32, z.y as i32);
        match self.screens.get_mut(ch.screen) {
            Some(s) => s
                .wind_open(wn, zx, zy, w, ty, 0, false)
                .map_err(|_| e::FCALL)?,
            None => return Err(e::FCALL),
        }
        let disp_max = int(&self.dia_evalue(ch, it)?)? as u16;
        let paper = b'0'.wrapping_add(int(&self.dia_evalue(ch, it)?)? as u8);
        let pen = b'0'.wrapping_add(int(&self.dia_evalue(ch, it)?)? as u8);
        let pp = vec![27, b'B', paper, 27, b'P', pen];
        if let Some(s) = self.screens.get_mut(ch.screen) {
            let _ = s.print_text(&pp);
            let tab = s.text.windows.first().map_or(4, |w| w.tab) as u8;
            let _ = s.print_text(&[27, b'C', b'0', 25, 27, b'T', b'0' + tab, 27, b'V', b'0']);
        }
        z.rchange = self.dia_get_rout(ch)?;
        // Line table (`Dia_HyperText` `+Lib.s:22200`): LF or CR end a
        // line (a CR just after is skipped), ESC skips two characters,
        // line lengths are bytes. The last line, ended by the zero, gets a
        // length of 0 as in the original.
        let mut lines: Vec<(usize, u8)> = Vec::new();
        let mut a = 0usize;
        loop {
            let start = a;
            if lines.len() * 4 + 16 + ch.label_bytes + ch.abuf + 8
                >= ch.buf_size.saturating_sub(Self::stack_bytes(ch))
            {
                return Err(e::BUFFER);
            }
            let mut end = start;
            let finished = loop {
                let c = text.get(a).copied().unwrap_or(0);
                a += 1;
                if c >= 32 {
                    continue;
                }
                match c {
                    0 => break true,
                    27 => a += 2,
                    10 | 13 => {
                        end = a - 1;
                        if text.get(a) == Some(&13) {
                            a += 1;
                        }
                        break false;
                    }
                    _ => {}
                }
            };
            lines.push((start, (end - start) as u8));
            if finished {
                break;
            }
        }
        ch.next_zone = DVal::Int(lines.len() as i32);
        ch.abuf += lines.len() * 4 + 4;
        let disp_size = disp_max as usize * 8 + 8;
        Self::dia_alloc(ch, ty as usize * disp_size + 4)?;
        ch.pbuf = ch.abuf;
        z.kind = ZoneKind::Text(Box::new(TextZone {
            tx: w as i16,
            ty: ty as i16,
            pos: pos as i16,
            text,
            lines,
            rows: vec![HtRow::default(); ty.max(0) as usize],
            disp_max,
            act: None,
            pen,
            paper,
            pp,
            buffer: Vec::new(),
        }));
        ch.records.push(Record::Zone(z));
        let idx = ch.records.len() - 1;
        self.dia_tx_draw(ch, idx);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Zone routines
    // ------------------------------------------------------------------

    /// Saves the drawing context around a zone routine.
    fn dia_routine_ctx(ch: &Channel) -> (i32, i32, i32, i32, [i16; 4], DVal, DVal, Option<usize>) {
        (
            ch.base_x,
            ch.base_y,
            ch.sx,
            ch.sy,
            [ch.xa, ch.ya, ch.xb, ch.yb],
            ch.v_bp.clone(),
            ch.v_br.clone(),
            ch.v_zone,
        )
    }

    #[allow(clippy::type_complexity)]
    fn dia_routine_restore(
        ch: &mut Channel,
        c: (i32, i32, i32, i32, [i16; 4], DVal, DVal, Option<usize>),
    ) {
        (ch.base_x, ch.base_y, ch.sx, ch.sy) = (c.0, c.1, c.2, c.3);
        [ch.xa, ch.ya, ch.xb, ch.yb] = c.4;
        ch.v_bp = c.5;
        ch.v_br = c.6;
        ch.v_zone = c.7;
    }

    /// `Dia_BtDraw`: runs the draw routine of a button with the base and
    /// size of the button.
    pub(crate) fn dia_bt_draw(&mut self, ch: &mut Channel, it: &mut Interp, idx: usize) -> DR<()> {
        let Some(z) = ch.zone(idx) else { return Ok(()) };
        let ZoneKind::Button { rdraw, .. } = z.kind else {
            return Ok(());
        };
        if rdraw == 0 {
            return Ok(());
        }
        let (zx, zy, zsx, zsy, pos) = (
            z.x as u16 as i32,
            z.y as u16 as i32,
            z.sx as u16 as i32,
            z.sy as u16 as i32,
            z.pos,
        );
        let saved = Self::dia_routine_ctx(ch);
        ch.sx = zsx;
        ch.sy = zsy;
        ch.base_x = zx;
        ch.base_y = zy;
        ch.v_bp = DVal::Int(pos);
        ch.v_zone = Some(idx);
        let r = self.dia_call_routine(ch, it, rdraw);
        Self::dia_routine_restore(ch, saved);
        r
    }

    /// `Dia_ZoChange`: runs the change routine of a zone. Returns the
    /// position to keep (variable -2, set by `BR`).
    pub(crate) fn dia_zo_change(
        &mut self,
        ch: &mut Channel,
        it: &mut Interp,
        idx: usize,
    ) -> DR<DVal> {
        let Some(z) = ch.zone_mut(idx) else {
            return Ok(DVal::Int(0));
        };
        let pos = DVal::Int(z.pos);
        let r = z.rchange;
        if r == 0 {
            return Ok(pos);
        }
        z.flags &= !0xA0;
        let saved = Self::dia_routine_ctx(ch);
        ch.v_bp = pos.clone();
        ch.v_br = pos;
        ch.v_zone = Some(idx);
        let res = self.dia_call_routine(ch, it, r);
        let ret = ch.v_br.clone();
        Self::dia_routine_restore(ch, saved);
        res.map(|_| ret)
    }

    /// `Dia_BChange`: BC n,pos: sets the position of the other buttons n.
    fn dia_bchange(&mut self, ch: &mut Channel, it: &mut Interp, n: i32, pos: i32) -> DR<()> {
        let cur = ch.v_zone.ok_or(e::SYNTAX)?;
        if ch.zone(cur).is_some_and(|z| z.flags & 0x40 != 0) {
            return Ok(());
        }
        for i in 0..ch.records.len() {
            let Some(z) = ch.zone_mut(i) else { continue };
            if !matches!(z.kind, ZoneKind::Button { .. }) || z.number != n as i16 || i == cur {
                continue;
            }
            z.flags |= 0x40;
            if z.pos != pos {
                z.pos = pos;
                self.dia_bt_draw(ch, it, i)?;
                self.dia_zo_change(ch, it, i)?;
            }
            if let Some(z) = ch.zone_mut(i) {
                z.flags &= !0xC0;
            }
        }
        Ok(())
    }
}

/// Result of `Dia_RunProgram`.
#[derive(Debug, PartialEq, Eq)]
pub enum RunResult {
    Value(i32),
    Waiting,
}

/// Window initialisation of edit and list zones (`Dia_WInit`): cursor off,
/// paper, pen, all planes, clear, no scrolling.
pub fn win_init(paper: i32, pen: i32) -> Vec<u8> {
    vec![
        27,
        b'C',
        b'0',
        27,
        b'B',
        b'0'.wrapping_add(paper as u8),
        27,
        b'P',
        b'0'.wrapping_add(pen as u8),
        27,
        b'J',
        48 + 31,
        25,
        27,
        b'V',
        b'0',
    ]
}
