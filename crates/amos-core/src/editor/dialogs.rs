//! The editor dialogs: the Interface program #1 of the editor resource
//! bank run on the editor screen (`Ed_InitDialogues`, `Ed_Dialogue`,
//! +Edit.s:3055-3145), and the file selector (`Ed_File_Selector`, the
//! same `Fsel$` as in programs).
//!
//! The original waits in `Dia_RunProgram` until the dialog is closed; here
//! the dialog is started, then resumed at each frame by [`Editor::vbl`]
//! and the editor function continues in [`Editor::dialog_done`] /
//! [`Editor::fsel_done`] with the result.

use std::rc::Rc;

use super::*;
use crate::interface::engine::{OpenParams, RunResult};
use crate::interface::resource::Messages;
use crate::interface::{DVal, Resource as DiaResource};

/// The editor's dialog channel (channel 1 of the editor in the original;
/// a number no program can use here, as the editor shares the machine's
/// dialog state).
pub const ED_CHANNEL: i64 = 0x0ED1_0001;

/// First image of the dialog puzzle in the editor bank (`Ed_DiaImages`).
const ED_DIA_IMAGES: i32 = 66;

/// Labels of the editor dialog program (`EdD_*`, +Edit.s:15331).
pub mod label {
    pub const TITLE: i32 = 0;
    pub const QUIT: i32 = 2;
    pub const SEARCH: i32 = 4;
    pub const REPLACE: i32 = 6;
    pub const W_BLOCK: i32 = 8;
    pub const W_TEXT: i32 = 9;
    pub const CHANGES: i32 = 10;
    pub const SAVED: i32 = 11;
    pub const GOTO_LINE: i32 = 35;
    pub const SET_BUF: i32 = 36;
    pub const SET_TAB: i32 = 38;
    pub const INFOS: i32 = 54;
    pub const ABOUT_EXT: i32 = 55;
    pub const LIGNE: i32 = 59;
}

/// What the editor does when a dialog or the file selector closes.
#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    Nothing,
    Load,
    LoadNew,
    /// Save under the chosen name, then call the function (`Ed_Saved`).
    SaveAs(Option<u16>),
    Merge,
    MergeAscii,
    SaveBlock,
    SaveBlockAscii,
    Search,
    Replace,
    ReplaceAll,
    GotoLine,
    SetTab,
    SetBuffer,
    /// "Program not saved. Save?": then call the function.
    Saved(u16),
    Quit,
    AboutExt(usize),
    /// The line shown when a program stops (`Ed_Ligne`).
    Ligne(crate::interp::StopInfo),
}

/// A dialog in progress.
#[derive(Clone, Debug)]
pub enum Modal {
    Dialog(Then),
    Fsel(Then),
}

/// Values read from the dialog when it closes.
#[derive(Debug, Default)]
pub struct DialogResult {
    /// Exit value (number of the button).
    pub ret: i32,
    /// Values of the edit zones 3 and 4.
    pub zone3: Option<DVal>,
    pub zone4: Option<DVal>,
    pub vars: Vec<DVal>,
}

fn dval_int(v: &Option<DVal>) -> Option<i32> {
    match v {
        Some(DVal::Int(n)) => Some(*n),
        _ => None,
    }
}

fn dval_str(v: &Option<DVal>) -> Vec<u8> {
    match v {
        Some(DVal::Str(s)) => s.to_vec(),
        _ => Vec::new(),
    }
}

/// Extensions of this AMOS (`AdTokens` slots), for About Extensions.
const EXTENSIONS: [&str; 6] = [
    "AMOSPro Music extension",
    "AMOSPro Compactor extension",
    "AMOSPro Requester extension",
    "AMOSPro 3D extension",
    "AMOSPro Compiler extension",
    "AMOSPro IO Ports extension",
];

/// The editor screen has 16 colours: 8 of the editor, 8 for the syntax
/// colours (an addition of this port). The dialog images of the editor
/// bank have 3 bitplanes, and `UnPack_Bitmap` only unpacks images with
/// the screen's number of planes: while a dialog is open the screen is
/// seen as an 8 colour screen, as in the original (it is drawn without
/// the syntax colours before).
fn dialog_planes(m: &mut Machine, dialog: bool) {
    if let Some(s) = m.hw.screens.get_mut(EC_EDIT) {
        let (planes, colours) = if dialog { (3, 8) } else { (4, 16) };
        s.planes = planes;
        s.colours = colours;
        s.version += 1;
    }
}

/// `Dia_SetVFlags`: bit i of `flags` in variable i (0 or 1).
fn set_vflags(flags: u16, n: usize) -> Vec<DVal> {
    (0..n).map(|i| DVal::Int(((flags >> i) & 1) as i32)).collect()
}

/// `Dia_GetVFlags`: variable i non zero sets bit i.
fn get_vflags(vars: &[DVal]) -> u16 {
    vars.iter().enumerate().fold(0, |f, (i, v)| match v {
        DVal::Int(0) => f,
        DVal::Int(_) => f | 1 << i,
        _ => f,
    })
}

impl Editor {
    /// The Interface resource of the editor dialogs: the graphics of the
    /// editor bank and the editor messages of the configuration (`ME n`).
    fn dialog_resource(&self) -> DiaResource {
        let def = DiaResource::default_resource();
        let mut r = DiaResource::from_bank(&self.res.raw, &def);
        let mut data = Vec::new();
        for m in &self.cfg.messages {
            data.push(0);
            data.push(m.len().min(127) as u8);
            data.extend_from_slice(&m[..m.len().min(127)]);
        }
        data.extend_from_slice(&[0, 0xFF]);
        r.messages = Rc::new(Messages { data });
        r
    }

    /// True while a dialog or the file selector is open.
    pub fn in_dialog(&self) -> bool {
        self.modal.is_some()
    }

    /// `Ed_Dialogue`: runs the dialog at `label` with the variables given;
    /// `then` continues when it closes.
    pub fn dialog(&mut self, m: &mut Machine, label: i32, vars: &[(usize, DVal)], then: Then) {
        // The dialog saves the screen behind it: draw the editor first,
        // without the syntax colours (see `dialog_planes`).
        let hl = self.cfg.highlight;
        self.cfg.highlight = false;
        self.redraw(m);
        self.cfg.highlight = hl;
        self.dirty = false;
        dialog_planes(m, true);
        let _ = m.hw.dia_close_channel(&mut m.interp, ED_CHANNEL);
        self.dialog_screen = Some(m.hw.screens.current);
        m.hw.screens.current = Some(EC_EDIT);
        let Some(prog) = self.res.programs.first() else { return self.dialog_failed(m, 0) };
        let res = self.dialog_resource();
        if let Err((c, _)) = m.hw.dia_open_channel(OpenParams {
            number: ED_CHANNEL,
            prog: Rc::from(&prog[..]),
            nvar: 16,
            buffer: 1024,
            res,
        }) {
            return self.dialog_failed(m, c);
        }
        if let Some(i) = m.hw.dialogs.channel_index(ED_CHANNEL) {
            let ch = &mut m.hw.dialogs.channels[i];
            ch.puzzle_i = ED_DIA_IMAGES;
            for (n, v) in vars {
                if let Some(slot) = ch.vars.get_mut(*n) {
                    *slot = v.clone();
                }
            }
        }
        m.hw.input.clear_keys();
        self.modal = Some(Modal::Dialog(then.clone()));
        let r = m.hw.dia_run_program(&mut m.interp, ED_CHANNEL, Some(label), None, None);
        self.dialog_progress(m, then, r);
    }

    fn dialog_failed(&mut self, m: &mut Machine, code: u8) {
        let _ = m.hw.dia_close_channel(&mut m.interp, ED_CHANNEL);
        dialog_planes(m, false);
        if let Some(c) = self.dialog_screen.take() {
            m.hw.screens.current = c;
        }
        self.modal = None;
        self.dirty = true;
        self.alert(crate::errors::message(119 + code as u16).to_string());
    }

    /// Handles the result of a run step: still waiting, or closed.
    fn dialog_progress(&mut self, m: &mut Machine, then: Then, r: Result<RunResult, (u8, usize)>) {
        let ret = match r {
            Ok(RunResult::Waiting) => return,
            Ok(RunResult::Value(v)) => v,
            Err((c, _)) => return self.dialog_failed(m, c),
        };
        let mut res = DialogResult { ret, ..Default::default() };
        res.zone3 = m.hw.dia_get_value(ED_CHANNEL, 3, -1).ok();
        res.zone4 = m.hw.dia_get_value(ED_CHANNEL, 4, 1).ok();
        if let Some(i) = m.hw.dialogs.channel_index(ED_CHANNEL) {
            res.vars = m.hw.dialogs.channels[i].vars.clone();
        }
        let _ = m.hw.dia_close_channel(&mut m.interp, ED_CHANNEL);
        dialog_planes(m, false);
        if let Some(c) = self.dialog_screen.take() {
            m.hw.screens.current = c;
        }
        self.modal = None;
        self.mouse_prev = m.hw.input.mouse_buttons;
        self.dirty = true;
        self.dialog_done(m, then, res);
        self.ensure_visible();
    }

    /// `Ed_File_Selector` (+Edit.s): the file selector with the editor
    /// messages `msg` (pattern), `msg+1` (default name), `msg+2` and
    /// `msg+3` (titles).
    pub fn file_selector(&mut self, m: &mut Machine, msg: usize, then: Then) {
        self.redraw(m);
        self.dirty = false;
        let s = |n: usize| self.cfg.messages.get(n - 1).cloned().unwrap_or_default();
        let (path, def, t1, t2) = (s(msg), s(msg + 1), s(msg + 2), s(msg + 3));
        m.hw.input.clear_keys();
        match m.hw.fsel_start(&mut m.interp, &path, &def, &t1, &t2) {
            Ok(()) => self.modal = Some(Modal::Fsel(then)),
            Err(_) => self.alert_message(184),
        }
    }

    /// One frame of the open dialog (called instead of the editor input).
    pub(super) fn modal_vbl(&mut self, m: &mut Machine) {
        match self.modal.clone() {
            Some(Modal::Dialog(then)) => {
                let r = m.hw.dia_run_resume(&mut m.interp, ED_CHANNEL);
                self.dialog_progress(m, then, r);
            }
            Some(Modal::Fsel(then)) => {
                if let Some(name) = m.hw.fsel_step(&mut m.interp) {
                    self.modal = None;
                    self.mouse_prev = m.hw.input.mouse_buttons;
                    self.dirty = true;
                    self.fsel_done(m, then, latin1_to_string(&name));
                    self.ensure_visible();
                }
            }
            None => {}
        }
    }

    /// Continues after the file selector: `name` is empty if cancelled.
    fn fsel_done(&mut self, m: &mut Machine, then: Then, name: String) {
        if name.is_empty() {
            return;
        }
        let r: Result<(), String> = match then {
            Then::Load => self.load(m, &name),
            Then::LoadNew => {
                self.open_window();
                let r = self.load(m, &name);
                if r.is_err() {
                    self.close_window();
                }
                r
            }
            Then::SaveAs(after) => {
                let r = self.save_as(m, &name);
                if r.is_ok()
                    && let Some(f) = after
                {
                    self.call_saved(m, f);
                }
                r
            }
            Then::Merge | Then::MergeAscii => self.merge(m, &name),
            Then::SaveBlock | Then::SaveBlockAscii => self.save_block(m, &name, then == Then::SaveBlock),
            _ => Ok(()),
        };
        if let Err(e) = r {
            self.alert(e);
        }
    }

    fn merge(&mut self, m: &mut Machine, name: &str) -> Result<(), String> {
        let data = m.hw.files.read(name).map_err(|_| self.cfg.message(184))?;
        let d = &mut self.docs[self.current];
        let r = if data.starts_with(b"AMOS") {
            match Program::load(&data) {
                Ok(p) => d.paste(&p.lines().map(|(_, l)| l.to_vec()).collect::<Vec<_>>()),
                Err(_) => return Err(self.cfg.message(207)),
            }
        } else {
            d.paste_text(&data)
        };
        r.map_err(|e| self.cfg.message(e.message()))
    }

    fn save_block(&mut self, m: &mut Machine, name: &str, tokenised: bool) -> Result<(), String> {
        let lines = self.docs[self.current].block_copy().map_err(|e| self.cfg.message(e.message()))?;
        let data = if tokenised {
            Program { source: lines.concat(), ..Program::default() }.save()
        } else {
            let mut out = Vec::new();
            for l in &lines {
                out.extend(crate::detok::detok_line(l));
                out.push(b'\n');
            }
            out
        };
        m.hw.files.write(name, &data).map_err(|_| self.cfg.message(184))
    }

    /// Calls `f` after the "save changes" question was answered.
    fn call_saved(&mut self, m: &mut Machine, f: u16) {
        self.skip_saved = true;
        self.function(m, f);
        self.skip_saved = false;
    }

    /// `Ed_Saved`: if the program was changed, asks to save it before
    /// function `f`. Returns true if the dialog was opened (`f` is called
    /// when it closes).
    pub(super) fn saved_check(&mut self, m: &mut Machine, f: u16) -> bool {
        // `Ed_TokCur` first: the line being edited counts as a change.
        let _ = self.doc_mut().commit();
        if self.skip_saved || !self.doc().modified {
            return false;
        }
        let mut name = if self.doc().name.is_empty() {
            self.cfg.sys(7).to_vec()
        } else {
            bytes(self.doc().name.rsplit([':', '/']).next().unwrap_or(""))
        };
        name.truncate(32);
        self.dialog(m, label::SAVED, &[(0, DVal::str(&name))], Then::Saved(f));
        true
    }

    /// Search dialog variables (`Ed_DiaS`): the string, its width, and the
    /// four mode flags in variables 2-5.
    fn search_vars(&self) -> Vec<(usize, DVal)> {
        let mut v = vec![(0, DVal::str(&self.search)), (1, DVal::Int(32))];
        for (i, f) in set_vflags(self.search_mode & 1, 4).into_iter().enumerate() {
            v.push((2 + i, f));
        }
        v
    }

    /// `Ed_Search` (Amiga+F).
    pub fn search_dialog(&mut self, m: &mut Machine) {
        let v = self.search_vars();
        self.dialog(m, label::SEARCH, &v, Then::Search);
    }

    /// `Ed_Replace` (Amiga+Shift+F).
    pub fn replace_dialog(&mut self, m: &mut Machine) {
        let mut v = self.search_vars();
        v.push((8, DVal::str(&self.replace)));
        self.dialog(m, label::REPLACE, &v, Then::Replace);
    }

    /// `Ed_SR`: search from the cursor with the mode flags (bit 0 upper
    /// case = lower case, bit 1 backwards).
    pub(super) fn do_search(&mut self, backwards: bool) -> Result<(), EditError> {
        let s = self.search.clone();
        let ic = self.search_mode & 1 != 0;
        self.doc_mut().search(&s, !backwards, ic)
    }

    pub(super) fn do_replace(&mut self, backwards: bool) -> Result<(), EditError> {
        let (s, r) = (self.search.clone(), self.replace.clone());
        let ic = self.search_mode & 1 != 0;
        let d = self.doc_mut();
        if !d.replace_here(&s, &r, ic)? {
            d.search(&s, !backwards, ic)?;
            d.replace_here(&s, &r, ic)?;
        }
        Ok(())
    }

    /// Replaces every occurrence in the text or the block; returns the
    /// number of changes.
    fn replace_all(&mut self, in_block: bool) -> Result<usize, EditError> {
        let (s, r) = (self.search.clone(), self.replace.clone());
        let ic = self.search_mode & 1 != 0;
        let d = self.doc_mut();
        d.commit()?;
        let (first, last) =
            if in_block { d.block_range().ok_or(EditError::NoBlock)? } else { (0, d.len().saturating_sub(1)) };
        d.goto_line(first)?;
        d.x = 0;
        let mut n = 0;
        loop {
            if d.y > last || d.y >= d.len() {
                break;
            }
            if d.replace_here(&s, &r, ic)? {
                n += 1;
                continue;
            }
            if d.search(&s, true, ic).is_err() {
                break;
            }
        }
        Ok(n)
    }

    /// Continues the editor function when its dialog closes.
    fn dialog_done(&mut self, m: &mut Machine, then: Then, res: DialogResult) {
        let r: Result<(), EditError> = match then {
            Then::Search | Then::Replace => {
                self.search_mode = get_vflags(res.vars.get(2..6).unwrap_or(&[]));
                self.search = dval_str(&res.zone3);
                if then == Then::Replace {
                    self.replace = dval_str(&res.zone4);
                }
                if res.ret != 1 {
                    Ok(())
                } else if then == Then::Search {
                    self.do_search(self.search_mode & 2 != 0)
                } else if self.search_mode & 0b1100 != 0 {
                    // "Replace in whole block / text. Are you sure?"
                    let l = if self.search_mode & 4 != 0 { label::W_BLOCK } else { label::W_TEXT };
                    self.dialog(m, l, &[], Then::ReplaceAll);
                    Ok(())
                } else {
                    self.do_replace(self.search_mode & 2 != 0)
                }
            }
            Then::ReplaceAll => {
                if res.ret == 1 && !self.search.is_empty() {
                    match self.replace_all(self.search_mode & 4 != 0) {
                        Ok(n) => {
                            self.dialog(m, label::CHANGES, &[(0, DVal::Int(n as i32))], Then::Nothing);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    Ok(())
                }
            }
            Then::GotoLine => match dval_int(&res.zone3) {
                Some(n) if res.ret == 1 && n > 0 => {
                    let d = self.doc_mut();
                    let r = d.goto_row(n as usize - 1);
                    d.x = 0;
                    r
                }
                _ => Ok(()),
            },
            Then::SetTab => {
                if let Some(n) = dval_int(&res.zone3).filter(|_| res.ret == 1) {
                    self.cfg.tabs = n.clamp(1, 16) as u16;
                }
                Ok(())
            }
            Then::SetBuffer => {
                if let Some(n) = dval_int(&res.zone3).filter(|_| res.ret == 1) {
                    self.text_buffer = (n as i64).max(1024);
                }
                Ok(())
            }
            Then::Saved(f) => {
                match res.ret {
                    // Save, then go on.
                    1 => {
                        if self.doc().name.is_empty() {
                            self.file_selector(m, 74, Then::SaveAs(Some(f)));
                        } else if let Err(e) = self.save(m) {
                            self.alert(e);
                        } else {
                            self.call_saved(m, f);
                        }
                    }
                    // Do not save.
                    2 => self.call_saved(m, f),
                    _ => {}
                }
                Ok(())
            }
            Then::Quit => {
                if res.ret == 1 && !self.saved_check(m, 1082) {
                    self.quit_requested = true;
                }
                Ok(())
            }
            Then::Ligne(info) => {
                self.ligne_done(m, info, res.ret);
                Ok(())
            }
            Then::AboutExt(n) => {
                let next = match res.ret {
                    1 => Some(n.saturating_sub(1)),
                    2 => Some((n + 1).min(EXTENSIONS.len() - 1)),
                    _ => None,
                };
                if let Some(k) = next {
                    self.about_extension(m, k);
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = r
            && e != EditError::Reported
        {
            self.edit_error(e);
        }
    }

    /// `Ed_AboutExt`: extension `n` (0-based).
    pub(super) fn about_extension(&mut self, m: &mut Machine, n: usize) {
        let vars = [(0, DVal::Int(n as i32 + 1)), (1, DVal::str(EXTENSIONS[n].as_bytes()))];
        self.dialog(m, label::ABOUT_EXT, &vars, Then::AboutExt(n));
    }

    /// `Ed_About`: version, number of extensions, registration.
    pub(super) fn about(&mut self, m: &mut Machine) {
        let vars = [
            (0, DVal::str(b" V2.00")),
            (1, DVal::Int(EXTENSIONS.len() as i32)),
            (2, DVal::str(b"amos-rs")),
            (3, DVal::str(b"-")),
        ];
        self.dialog(m, label::TITLE, &vars, Then::Nothing);
    }

    /// `Ed_Infos`: free memory, text and bank lengths, lines,
    /// instructions.
    pub(super) fn infos(&mut self, m: &mut Machine) {
        let prg = self.doc().to_program();
        let banks: usize = self.doc().banks.iter().map(|b| b.length()).sum();
        let text = prg.source.len() as i32;
        let instructions = prg
            .lines()
            .map(|(_, l)| {
                let mut n = 0;
                let mut p = 2;
                let mut any = false;
                while p + 2 <= l.len() {
                    let t = crate::program::read_u16(l, p);
                    if t == crate::tokens::TK_EOL {
                        break;
                    }
                    if t == crate::tokens::TK_DP {
                        n += any as i32;
                        any = false;
                    } else {
                        any = true;
                    }
                    p += crate::interp::verify::token_size(l, p);
                }
                n + any as i32
            })
            .sum::<i32>();
        let vars = [
            (0, DVal::Int(512 * 1024 - text.min(256 * 1024))),
            (1, DVal::Int(2048 * 1024 - banks as i32)),
            (2, DVal::Int(text)),
            (3, DVal::Int(banks as i32)),
            (4, DVal::Int(self.doc().len() as i32)),
            (5, DVal::Int(instructions)),
        ];
        self.dialog(m, label::INFOS, &vars, Then::Nothing);
    }
}
