//! Interface dialogs, resource banks, sliders and the file selector
//! (`+Lib.s:14292-14990`, `+ILib.s:6670`).
//!
//! The Interface engine itself is in [`crate::interface`].

use std::rc::Rc;

use super::Hardware;
use crate::errors;
use crate::interface::engine::{OpenParams, RunResult};
use crate::interface::{ArrRef, Blocking, DVal, ERROR_BASE, Resource, resource, slider};
use crate::interp::value::{ENT_NUL, Value};
use crate::interp::{Exc, Interp, R, WaitKind, err};
use crate::tokens::{Keyword, TK_COMMA, TK_PAR1, TK_PAR2, TokenKind, tk};

const FONCALL: u16 = errors::ILLEGAL_FUNCTION_CALL;
/// Wait identifier of the blocking dialog functions.
const DIALOG_WAIT: u32 = 0x4449_414C;

/// AMOS error of an Interface error code (`Dia_GoError`).
pub(crate) fn dia_err<T>(code: u8) -> R<T> {
    err(ERROR_BASE + code as u16)
}

/// Converts a BASIC value for the Interface.
fn to_dval(v: Value) -> DVal {
    match v {
        Value::Int(n) => DVal::Int(n),
        Value::Float(f) => DVal::Int(crate::interp::value::float_to_int(f)),
        Value::Str(s) => DVal::Str(Rc::from(&s[..])),
    }
}

fn opt(v: i32) -> Option<i32> {
    if v == ENT_NUL { None } else { Some(v) }
}

impl Hardware {
    /// Dialog state at the start of a program (`Dia_WarmInit` and the
    /// ClearVar routine closing the channels).
    pub fn dialogs_reset(&mut self) {
        let prefs = self.dialogs.fs_prefs;
        self.dialogs = Default::default();
        self.dialogs.fs_prefs = prefs;
        self.dialogs.key_serial = self.input.key_serial;
    }

    /// Resets the dialogs when a new program runs (fallback for when
    /// `reset` does not call [`Hardware::dialogs_reset`]).
    /// Resets the dialogs when another program runs (`Dialog(n)` checks
    /// this before reading its parameter; `dialog_fn` checks it too).
    pub(crate) fn dialogs_check_program(&mut self, it: &Interp) {
        let id = it.prg.as_ref().map_or(0, |p| Rc::as_ptr(p) as usize);
        if id != self.dialogs.program {
            self.dialogs_reset();
            self.dialogs.program = id;
        }
    }

    /// Automatic tests of the dialogs (`GoTest_Dialog`, called by
    /// `Test_Force` `+ILib.s:916` at each test point).
    // To be called by `Host::test_point` (dispatch.rs), before the menus.
    pub(crate) fn dialogs_test_point(&mut self, it: &mut Interp) -> R<()> {
        self.dialogs_check_program(it);
        if self.dialogs.channels.is_empty() || self.dia_reentry(it).is_some() {
            return Ok(());
        }
        match self.dia_auto_test(it, 127) {
            Ok(()) => Ok(()),
            Err(c) => dia_err(c),
        }
    }

    /// The blocking operation of the current instruction, when it is
    /// executed again.
    fn dia_reentry(&self, it: &Interp) -> Option<Blocking> {
        let (pos, b) = self.dialogs.blocking.as_ref()?;
        let w = it.wait.as_ref()?;
        if *pos == it.inst_pos
            && w.pos == it.inst_pos
            && matches!(w.kind, WaitKind::Other(DIALOG_WAIT, _))
        {
            Some(b.clone())
        } else {
            None
        }
    }

    /// Makes the current instruction wait for the next frame.
    pub(crate) fn dia_block<T>(&mut self, it: &mut Interp, b: Blocking) -> R<T> {
        self.dialogs.blocking = Some((it.inst_pos, b));
        it.wait_state(|| WaitKind::Other(DIALOG_WAIT, 0));
        Err(Exc::Block)
    }

    pub(crate) fn dia_unblock(&mut self, it: &mut Interp) {
        self.dialogs.blocking = None;
        it.clear_wait();
    }

    /// `Dia_GetPuzzle`: the resource in use (bank set by `Resource Bank`,
    /// else the default resource) and its number of programs.
    pub(crate) fn dia_get_puzzle(&self) -> R<Resource> {
        let def = Resource::default_resource();
        if self.dialogs.bank_puzzle == 0 {
            let mut r = def;
            r.programs.clear();
            return Ok(r);
        }
        let n = self.dialogs.bank_puzzle;
        let b = if (0..65536).contains(&n) {
            self.banks.get(n as u16)
        } else {
            None
        };
        match b {
            Some(b) if b.name.starts_with("Reso") => match b.raw() {
                Some(d) => Ok(Resource::from_bank(d, &def)),
                None => err(errors::BANK_NOT_RESERVED),
            },
            _ => err(errors::BANK_NOT_RESERVED),
        }
    }

    /// Program source of `Dialog Open` / `Dialog Box`: a program number of
    /// the resource (< 1024), a string, or the address of a string.
    fn dia_program(&self, v: &Value, res: &Resource) -> R<Rc<[u8]>> {
        match v {
            Value::Str(s) => Ok(Rc::from(&s[..])),
            _ => {
                let n = match v {
                    Value::Int(n) => *n,
                    Value::Float(f) => crate::interp::value::float_to_int(*f),
                    _ => 0,
                };
                if (n as u32) < 1024 {
                    if n < 1 || n as usize > res.programs.len() {
                        return err(FONCALL);
                    }
                    Ok(res.programs[n as usize - 1].clone())
                } else {
                    // A string in memory: word length then the text.
                    let a = n as u32;
                    let len = (self.banks.peek(a) as usize) << 8
                        | self.banks.peek(a.wrapping_add(1)) as usize;
                    Ok(Rc::from(self.banks.peek_bytes(a.wrapping_add(2), len)))
                }
            }
        }
    }

    /// Opens a channel and reports the errors as the original.
    fn dia_open(&mut self, p: OpenParams) -> R<()> {
        self.dialogs.error = 0;
        match self.dia_open_channel(p) {
            Ok(()) => Ok(()),
            Err((c, pos)) => {
                self.dialogs.error = pos as i32;
                dia_err(c)
            }
        }
    }

    /// `Dia_CloseChannel`.
    pub(crate) fn dia_close_channel(&mut self, it: &mut Interp, n: i64) -> R<()> {
        if self.dia_eff_channel(it, n).is_err() {
            return dia_err(crate::interface::e::CHANNEL_NOT_DEFINED);
        }
        if let Some((_, ch)) = self.dialogs.take(n)
            && let Some(Blocking::Run(b) | Blocking::Box(b)) =
                self.dialogs.blocking.as_ref().map(|x| &x.1)
            && *b == ch.number
        {
            self.dialogs.blocking = None;
        }
        Ok(())
    }

    /// Result of a run: value, wait, or error (with `=Edialog`).
    fn dia_run_result(
        &mut self,
        it: &mut Interp,
        r: Result<RunResult, (u8, usize)>,
        b: Blocking,
    ) -> R<i32> {
        match r {
            Ok(RunResult::Value(v)) => {
                self.dia_unblock(it);
                Ok(v)
            }
            Ok(RunResult::Waiting) => self.dia_block(it, b),
            Err((c, pos)) => {
                self.dia_unblock(it);
                self.dialogs.error = pos as i32;
                dia_err(c)
            }
        }
    }

    /// `Dia_RunQuick` (`Dialog Box`): temporary channel with 16 variables,
    /// run, closed.
    fn dia_dialog_box(&mut self, it: &mut Interp, a: &crate::interp::params::Args) -> R<i32> {
        if let Some(Blocking::Box(n)) = self.dia_reentry(it) {
            let r = self.dia_run_resume(it, n);
            if !matches!(r, Ok(RunResult::Waiting)) {
                let _ = self.dia_close_channel(it, n);
            }
            return self.dia_run_result(it, r, Blocking::Box(n));
        }
        let res = self.dia_get_puzzle()?;
        let prog = self.dia_program(&a.value(0), &res)?;
        let n = self.dialogs.free_quick_channel();
        let v = a.opt(1).unwrap_or(ENT_NUL);
        let vs = if a.len() > 2 {
            a.str(2)
        } else {
            crate::interp::value::empty_str()
        };
        let (x, y) = (a.opt(3), a.opt(4));
        self.dialogs.error = 0;
        if let Err((c, pos)) = self.dia_open_channel(OpenParams {
            number: n,
            prog,
            nvar: 16,
            buffer: 1024,
            res,
        }) {
            self.dialogs.error = pos as i32;
            return dia_err(c);
        }
        if let Some(i) = self.dialogs.channel_index(n) {
            let ch = &mut self.dialogs.channels[i];
            ch.vars[0] = DVal::Int(v);
            ch.vars[1] = DVal::Str(Rc::from(&vs[..]));
            ch.vars[2] = DVal::Int(0);
        }
        let r = self.dia_run_program(it, n, None, x, y);
        if !matches!(r, Ok(RunResult::Waiting)) {
            let _ = self.dia_close_channel(it, n);
        }
        self.dia_run_result(it, r, Blocking::Box(n))
    }

    /// Value of `Vdialog(ch,n)=`: `Array(a(0))` is given to the Interface
    /// as a reference to the array.
    fn dia_assign_value(&mut self, it: &mut Interp) -> R<DVal> {
        if it.peek() == tk::ARRAY {
            let save = it.pc;
            it.pc = it.skip_token(it.pc);
            if it.peek() == TK_PAR1 {
                it.pc += 2;
                let (loc, _) = it.array_ref(self)?;
                it.expect(TK_PAR2)?;
                let t = it.peek();
                if Interp::is_end(t) {
                    return Ok(DVal::Arr(ArrRef::Basic {
                        slot: loc.slot,
                        frame: loc.frame,
                    }));
                }
            }
            it.pc = save;
        }
        Ok(to_dval(it.eval(self)?))
    }

    /// Address of variable n of a channel (`Dia_GetVariable`).
    fn dia_var(&mut self, ch: i64, n: i32) -> R<&mut DVal> {
        let Some(i) = self.dialogs.channel_index(ch) else {
            return dia_err(crate::interface::e::CHANNEL_NOT_DEFINED);
        };
        let c = &mut self.dialogs.channels[i];
        if n as u32 as usize >= c.vars.len() {
            return dia_err(crate::interface::e::VAR_NOT_DEFINED);
        }
        Ok(&mut c.vars[n as usize])
    }

    /// Slider inks of the current screen.
    fn dia_screen_inks(&mut self) -> R<(usize, crate::interface::SliderInks)> {
        let Some(c) = self.screens.current else {
            return err(errors::SCREEN_NOT_OPENED);
        };
        let id = self.dia_screen_id(c).unwrap_or(0);
        let inks = match self.dialogs.slider_inks.get(&id) {
            Some(i) => *i,
            None => {
                let s = self.screens.get(c).unwrap();
                let w = s
                    .text
                    .windows
                    .iter()
                    .find(|w| w.number == 0)
                    .or(s.text.windows.first());
                let (paper, pen) = w.map_or((1, 2), |w| (w.paper, w.pen));
                slider::default_inks(paper, pen)
            }
        };
        Ok((id, inks))
    }

    /// `Dia_RScOpen`: opens a screen with the resource palette and mode.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dia_rsc_open(
        &mut self,
        g: &crate::interface::resource::Graphics,
        n: usize,
        w: i32,
        h: i32,
        flash: i32,
        lace: Option<bool>,
    ) -> R<()> {
        use crate::gfx::screen::lib_error;
        let mut mode = (g.mode & 0x8004) as u32;
        if let Some(l) = lace {
            mode = (mode & !4) | if l { 4 } else { 0 };
        }
        let colours = g.colours as u32;
        if colours != 4096 && !matches!(colours, 2 | 4 | 8 | 16 | 32 | 64) {
            return err(lib_error(crate::gfx::screen::E_ILLEGAL_COLOURS));
        }
        self.screens
            .open(n, w, h, colours, mode)
            .map_err(|e| Exc::Error(lib_error(e)))?;
        let s = self.screens.get_mut(n).unwrap();
        s.palette = g.palette;
        if flash == 0 {
            let _ = s.print_text(b"\x1bC0");
        } else {
            if flash as u32 >= s.num_colours() {
                return err(lib_error(1));
            }
            let def = resource::SYSTEM_MESSAGES[45].as_bytes();
            let _ = self.screens.effects.flash(n, flash as usize, def);
            let s = self.screens.get_mut(n).unwrap();
            let _ = s.print_text(&[27, b'D', b'0'.wrapping_add(flash as u8)]);
        }
        Ok(())
    }

    pub(crate) fn dialogs_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        let reserved = kw
            .def()
            .is_some_and(|d| d.kind() == TokenKind::ReservedVariable);
        match kw.token {
            DIALOG_OPEN | DIALOG_OPEN_2 | DIALOG_OPEN_3 => {
                self.dialogs_check_program(it);
                let a = it.inst_args(self, kw)?;
                let res = self.dia_get_puzzle()?;
                let nvar = a.opt(2).unwrap_or(16);
                let buffer = a.opt(3).unwrap_or(1024);
                if buffer as u32 <= 256 || nvar < 0 {
                    return err(FONCALL);
                }
                let prog = self.dia_program(&a.value(1), &res)?;
                let n = a.int(0);
                if n <= 0 {
                    return err(FONCALL);
                }
                self.dia_open(OpenParams {
                    number: n as i64,
                    prog,
                    nvar: nvar as usize,
                    buffer: buffer as usize,
                    res,
                })?;
            }
            DIALOG_CLOSE => {
                self.dialogs_check_program(it);
                it.inst_args(self, kw)?;
                let all: Vec<i64> = self.dialogs.channels.iter().map(|c| c.number).collect();
                for n in all {
                    self.dia_close_channel(it, n)?;
                }
            }
            DIALOG_CLOSE_2 => {
                self.dialogs_check_program(it);
                let n = it.inst_args(self, kw)?.int(0);
                if n <= 0 {
                    return err(FONCALL);
                }
                self.dia_close_channel(it, n as i64)?;
            }
            DIALOG_CLR => {
                self.dialogs_check_program(it);
                let n = it.inst_args(self, kw)?.int(0);
                if n <= 0 {
                    return err(FONCALL);
                }
                if self.dia_eff_channel(it, n as i64).is_err() {
                    return dia_err(crate::interface::e::CHANNEL_NOT_DEFINED);
                }
            }
            DIALOG_FREEZE | DIALOG_UNFREEZE => {
                self.dialogs_check_program(it);
                it.inst_args(self, kw)?;
                let f = kw.token == DIALOG_FREEZE;
                for c in &mut self.dialogs.channels {
                    c.rflags = if f { c.rflags | 4 } else { c.rflags & !4 };
                }
            }
            DIALOG_FREEZE_2 | DIALOG_UNFREEZE_2 => {
                self.dialogs_check_program(it);
                let n = it.inst_args(self, kw)?.int(0);
                if n <= 0 {
                    return err(FONCALL);
                }
                let f = kw.token == DIALOG_FREEZE_2;
                let Some(i) = self.dialogs.channel_index(n as i64) else {
                    return dia_err(crate::interface::e::CHANNEL_NOT_DEFINED);
                };
                let c = &mut self.dialogs.channels[i];
                c.rflags = if f { c.rflags | 4 } else { c.rflags & !4 };
            }
            DIALOG_UPDATE | DIALOG_UPDATE_2 | DIALOG_UPDATE_3 | DIALOG_UPDATE_4 => {
                self.dialogs_check_program(it);
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                let zone = a.int(1);
                let p1 = if a.len() > 2 {
                    match a.value(2) {
                        Value::Int(ENT_NUL) => None,
                        v => Some(to_dval(v)),
                    }
                } else {
                    None
                };
                let p2 = if a.len() > 3 { opt(a.int(3)) } else { None };
                let p3 = if a.len() > 4 { opt(a.int(4)) } else { None };
                if n <= 0 {
                    return err(FONCALL);
                }
                self.dia_update(it, n as i64, zone, p1, p2, p3)?;
            }
            VDIALOG | VDIALOG_S if reserved => {
                self.dialogs_check_program(it);
                it.expect(TK_PAR1)?;
                let ch = it.eval_int(self)?;
                it.expect(TK_COMMA)?;
                let n = it.eval_int(self)?;
                it.expect(TK_PAR2)?;
                it.expect(tk::OP_EQ)?;
                let v = self.dia_assign_value(it)?;
                *self.dia_var(ch as i64, n)? = v;
            }
            RESOURCE_BANK => {
                self.dialogs_check_program(it);
                let n = it.inst_args(self, kw)?.int(0);
                if n < 0 {
                    return err(FONCALL);
                }
                self.dialogs.bank_puzzle = n;
            }
            RESOURCE_SCREEN_OPEN => {
                self.dialogs_check_program(it);
                let a = it.inst_args(self, kw)?;
                let res = self.dia_get_puzzle()?;
                let n = a.int(0);
                if n as u32 >= 8 {
                    return err(crate::gfx::screen::lib_error(
                        crate::gfx::screen::E_SCREEN_NUMBER,
                    ));
                }
                self.dia_rsc_open(
                    &res.graphics,
                    n as usize,
                    a.int(1),
                    a.int(2),
                    a.int(3),
                    None,
                )?;
            }
            RESOURCE_UNPACK => {
                self.dialogs_check_program(it);
                let a = it.inst_args(self, kw)?;
                let res = self.dia_get_puzzle()?;
                let n = a.int(0);
                if n <= 0 || n as usize > res.graphics.count() {
                    return err(FONCALL);
                }
                let img = res.graphics.image(n as usize).unwrap_or(&[]);
                let (x, y) = (a.int(1), a.int(2));
                let Some(s) = self.screens.current_mut() else {
                    return err(errors::SCREEN_NOT_OPENED);
                };
                let (w, h, planes, li) = (
                    s.width as usize,
                    s.height as usize,
                    s.planes as usize,
                    s.logic,
                );
                if x < 0 || y < 0 {
                    return err(FONCALL);
                }
                if crate::gfx::pack::unpack_bitmap(img, &mut s.bitmaps[li], w, h, planes, x, y)
                    .is_none()
                {
                    return err(FONCALL);
                }
                s.version += 1;
            }
            READ_TEXT => {
                self.dialogs_check_program(it);
                let a = it.inst_args(self, kw)?;
                self.read_text_file(it, &a.str(0))?;
            }
            T__3 => {
                self.dialogs_check_program(it);
                let a = it.inst_args(self, kw)?;
                let (title, addr, len) = (a.str(0), a.int(1), a.int(2));
                self.read_text_memory(it, &title, addr, len)?;
            }
            HSLIDER | VSLIDER => {
                let a = it.inst_args(self, kw)?;
                let v: Vec<i32> = (0..7).map(|i| a.int(i)).collect();
                if v.iter().any(|&x| x < 0) {
                    return err(FONCALL);
                }
                let (tx, ty) = (v[2] - v[0], v[3] - v[1]);
                if tx <= 0 || ty <= 0 {
                    return err(FONCALL);
                }
                let (_, inks) = self.dia_screen_inks()?;
                let sprites = self.banks.get(1).cloned();
                let s = self.screens.current_mut().unwrap();
                let vertical = kw.token == VSLIDER;
                if !slider::draw(
                    s,
                    &inks,
                    sprites.as_ref(),
                    vertical,
                    v[0],
                    v[1],
                    tx,
                    ty,
                    v[4],
                    v[5],
                    v[6],
                ) {
                    return err(FONCALL);
                }
            }
            SET_SLIDER => {
                let a = it.inst_args(self, kw)?;
                let (id, mut inks) = self.dia_screen_inks()?;
                for (i, ink) in inks.iter_mut().enumerate() {
                    if let Some(v) = a.opt(i) {
                        *ink = if i == 3 || i == 7 {
                            v as i16 as i32
                        } else {
                            v as u8 as i32
                        };
                    }
                }
                self.dialogs.slider_inks.insert(id, inks);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Dia_Update` (`Dialog Update`).
    fn dia_update(
        &mut self,
        it: &mut Interp,
        n: i64,
        zone: i32,
        p1: Option<DVal>,
        p2: Option<i32>,
        p3: Option<i32>,
    ) -> R<()> {
        use crate::interface::e;
        let Some((i, mut ch)) = self.dialogs.take(n) else {
            return dia_err(e::CHANNEL_NOT_DEFINED);
        };
        let r = (|| {
            if ch.rflags & 1 == 0 {
                return Ok(());
            }
            if self.dia_active(&mut ch).is_err() {
                return Err(e::CHANNEL_NOT_DEFINED);
            }
            ch.error = 0;
            let r = self.dia_zupdate(&mut ch, it, zone, p1, p2, p3);
            self.dia_reactive(&mut ch);
            r
        })();
        self.dialogs.put(i, ch);
        r.or_else(dia_err)
    }

    /// `Dialog(n)`: the result of channel `n` (-1 while it runs).
    pub(crate) fn dialog_fn(&mut self, it: &Interp, n: i32) -> R<i32> {
        self.dialogs_check_program(it);
        if n <= 0 {
            return err(FONCALL);
        }
        let Some(i) = self.dialogs.channel_index(n as i64) else {
            return dia_err(crate::interface::e::CHANNEL_NOT_DEFINED);
        };
        let c = &mut self.dialogs.channels[i];
        if c.rflags & 1 == 0 {
            Ok(-1)
        } else {
            let r = c.ret as u16 as i32;
            c.ret = 0;
            Ok(r)
        }
    }

    pub(crate) fn dialogs_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use crate::interface::e;
        use tk::*;
        let v = match kw.token {
            DIALOG_RUN | DIALOG_RUN_2 | DIALOG_RUN_3 => {
                self.dialogs_check_program(it);
                let a = it.func_args(self, kw)?;
                if let Some(Blocking::Run(n)) = self.dia_reentry(it) {
                    let r = self.dia_run_resume(it, n);
                    return self
                        .dia_run_result(it, r, Blocking::Run(n))
                        .map(|v| Some(Value::Int(v)));
                }
                let n = a.int(0);
                let label = if a.len() > 1 { a.int(1) } else { -1 };
                if label >= 65536 {
                    return err(FONCALL);
                }
                let (x, y) = if a.len() > 3 {
                    (opt(a.int(2)), opt(a.int(3)))
                } else {
                    (None, None)
                };
                if n <= 0 {
                    return err(FONCALL);
                }
                self.dialogs.error = 0;
                let label = if label < 0 { None } else { Some(label) };
                let r = self.dia_run_program(it, n as i64, label, x, y);
                Value::Int(self.dia_run_result(it, r, Blocking::Run(n as i64))?)
            }
            DIALOG => {
                // (The program is checked before the parameter is read.)
                self.dialogs_check_program(it);
                let n = it.func_args(self, kw)?.int(0);
                Value::Int(self.dialog_fn(it, n)?)
            }
            VDIALOG | VDIALOG_S => {
                self.dialogs_check_program(it);
                let a = it.func_args(self, kw)?;
                let v = self.dia_var(a.int(0) as i64, a.int(1))?.clone();
                match (kw.token == VDIALOG_S, v) {
                    (false, DVal::Int(n)) => Value::Int(n),
                    (false, _) => return err(errors::TYPE_MISMATCH),
                    (true, DVal::Str(s)) => Value::str(&s),
                    (true, _) => Value::str(b""),
                }
            }
            RDIALOG | RDIALOG_2 | RDIALOG_S | RDIALOG_S_2 => {
                self.dialogs_check_program(it);
                let a = it.func_args(self, kw)?;
                let k = if a.len() > 2 { a.int(2) } else { 1 };
                let v = self
                    .dia_get_value(a.int(0) as i64, a.int(1), k)
                    .or_else(dia_err)?;
                match (matches!(kw.token, RDIALOG_S | RDIALOG_S_2), v) {
                    (false, DVal::Int(n)) => Value::Int(n),
                    (false, _) => Value::Int(0),
                    (true, DVal::Str(s)) => Value::str(&s),
                    (true, _) => Value::str(b""),
                }
            }
            ZDIALOG => {
                self.dialogs_check_program(it);
                let a = it.func_args(self, kw)?;
                let Some(i) = self.dialogs.channel_index(a.int(0) as i64) else {
                    return dia_err(e::CHANNEL_NOT_DEFINED);
                };
                let c = &self.dialogs.channels[i];
                let z = Self::dia_zone_at(c, a.int(1), a.int(2));
                Value::Int(
                    z.and_then(|(z, _, _)| c.zone(z))
                        .map_or(-1, |z| z.number as i32),
                )
            }
            EDIALOG => {
                it.func_args(self, kw)?;
                Value::Int(self.dialogs.error)
            }
            DIALOG_BOX | DIALOG_BOX_2 | DIALOG_BOX_3 | DIALOG_BOX_4 => {
                self.dialogs_check_program(it);
                let a = it.func_args(self, kw)?;
                Value::Int(self.dia_dialog_box(it, &a)?)
            }
            RESOURCE_S => {
                self.dialogs_check_program(it);
                let n = it.func_args(self, kw)?.int(0);
                if n > 0 {
                    Value::str(&self.dia_get_puzzle()?.messages.get(n))
                } else {
                    match resource::system_resource(n) {
                        Some(s) => Value::str(&s),
                        None => return err(FONCALL),
                    }
                }
            }
            FSEL_S | FSEL_S_2 | FSEL_S_3 | FSEL_S_4 => {
                self.dialogs_check_program(it);
                let a = it.func_args(self, kw)?;
                let s = |i: usize| {
                    if a.len() > i {
                        a.str(i).to_vec()
                    } else {
                        Vec::new()
                    }
                };
                let r = self.fsel(it, &s(0), &s(1), &s(2), &s(3))?;
                Value::str(&r)
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }
}
