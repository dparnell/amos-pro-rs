//! The file selector (`=Fsel$`, `Dsk.FileSelector` `+Lib.s:17733`).
//!
//! As in the original, the requester is Interface program 2 of the default
//! resource, run on screen 10 (`EcFsel`) in channel `Fs_ChannelN`; the
//! machine code around it reads the directory and reacts to the zones:
//!
//! | zone | | zone | |
//! |---|---|---|---|
//! | 1 | OK (Return) | 10 / 11 | list up / down |
//! | 2 | Cancel (Esc) | 12 | list slider |
//! | 3 | Parent | 13 | the list |
//! | 4 | Devices | 14 | path (Return: read) |
//! | 5 | Assigns | 15 | name (Return: OK) |
//! | 6 | Get Dir | 16 / 17 / 18 | store, delete, store slider |
//! | 7 | Sort | 19 | Help key: find the name |
//! | 8 | Sizes | | right button: devices / assigns / files |
//!
//! The directory is read at once (the original reads one name per frame in
//! a task) and the store list of directories is not implemented (its
//! buttons are drawn but do nothing).

use std::rc::Rc;

use super::engine::{OpenParams, RunResult};
use super::*;
use crate::interp::{Interp, R};
use crate::machine::Hardware;

/// Variables of the selector program (`FsV_*` `+Equ.s:2159`).
const V_TITLE0: usize = 0;
const V_TITLE1: usize = 1;
const V_SORT: usize = 7;
const V_SIZE: usize = 8;
const V_PLIST: usize = 10;
const V_ARRAY: usize = 11;
const V_TX: usize = 12;
const V_TY: usize = 13;
const V_FILE: usize = 14;
const V_PATH: usize = 15;
const V_STORE: usize = 16;
const V_POSFIRST: usize = 25;
const V_AFFFLAG: usize = 26;
const V_MAX: usize = 27;

const Z_SLIDER: i32 = 12;
const Z_LIST: i32 = 13;
const Z_PATH: i32 = 14;
const Z_FILE: i32 = 15;
const Z_STORE_SLIDER: i32 = 18;

/// `PI_FsDSx`, `PI_FsDSy`, `PI_FsDWx`, `PI_FsDWy`, `PI_FsDVApp`
/// (`+Interpreter_Config.s:73`).
const FS_SX: i32 = 448;
const FS_SY: i32 = 158;
const FS_WX: i32 = 129 + 48;
const FS_WY: i32 = 50 + 20;
const FS_SPEED: i32 = 8;
/// `FillFSize`: maximum length of a list entry.
const FILL_SIZE: usize = 64;

/// A name of the list: '*' + name for a directory, ' ' + name for a file
/// or a device, with the file size.
#[derive(Clone, Debug)]
pub struct Entry {
    pub text: Vec<u8>,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// The screen opens from its centre (`AppCentre`).
    Appear(i32),
    Run,
    /// A message box is shown (directory error).
    Message(i64),
    /// Closing, with the result.
    Disappear(i32, Vec<u8>),
}

/// File selector state.
#[derive(Debug)]
pub struct Fsel {
    pub list: Vec<Entry>,
    pub phase: Phase,
    /// 0 files, 1 devices, 2 assigns (`Fs_DevFlag`).
    pub dev_flag: u8,
    /// Last clicked file (double click = OK).
    pub click: i32,
    pub name1: Vec<u8>,
    pub name2: Vec<u8>,
    pub old_screen: Option<usize>,
    pub limits: (i32, i32, i32, i32),
    pub sorted: bool,
    /// Sizes shown, and width of the list (`FsV_Size`, `FsV_Tx`).
    pub sizes: bool,
    pub tx: usize,
    /// Waiting for the right button to be released.
    pub right_wait: bool,
    pub screen_height: i32,
}

impl Fsel {
    /// Text of list element `i` (`Fs_GetName`): with the size at the right
    /// when sizes are shown.
    pub fn display(&self, i: usize) -> Vec<u8> {
        let Some(e) = self.list.get(i) else {
            return Vec::new();
        };
        let mut s = e.text.clone();
        let Some(size) = e.size.filter(|_| self.sizes && s.first() != Some(&b'*')) else {
            return s;
        };
        let col = self.tx.saturating_sub(8);
        s.resize(s.len().max(col), b' ');
        s.truncate(col);
        s.push(b' ');
        s.extend_from_slice(size.to_string().as_bytes());
        let end = col + 8;
        if s.len() < end {
            s.resize(end, b' ');
        }
        s
    }
}

fn upper(c: u8) -> u8 {
    c.to_ascii_uppercase()
}

/// `FfComp2` sort key: case insensitive, '*' (directories) first.
fn sort_key(s: &[u8]) -> Vec<u8> {
    s.iter()
        .map(|&c| if c == b'*' { 1 } else { upper(c) })
        .collect()
}

fn latin(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

fn text(s: &[u8]) -> String {
    s.iter().map(|&c| c as char).collect()
}

impl Hardware {
    /// `=Fsel$(path$,default$,title1$,title2$)`: blocking, entered again at
    /// each frame.
    pub(crate) fn fsel(
        &mut self,
        it: &mut Interp,
        path: &[u8],
        def: &[u8],
        t1: &[u8],
        t2: &[u8],
    ) -> R<Vec<u8>> {
        if !matches!(self.dia_reentry_kind(it), Some(Blocking::Fsel)) {
            self.dialogs.fsel = None;
            self.fs_start(it, path, def, t1, t2)?;
            return self.dia_block(it, Blocking::Fsel);
        }
        match self.fs_step(it) {
            Some(r) => {
                self.dia_unblock(it);
                Ok(r)
            }
            None => self.dia_block(it, Blocking::Fsel),
        }
    }

    /// Starts the file selector outside a BASIC function (the editor's
    /// `Ed_File_Selector`): drive it with [`Hardware::fsel_step`] at each
    /// frame.
    pub fn fsel_start(
        &mut self,
        it: &mut Interp,
        path: &[u8],
        def: &[u8],
        t1: &[u8],
        t2: &[u8],
    ) -> R<()> {
        self.dialogs.fsel = None;
        self.fs_start(it, path, def, t1, t2)
    }

    /// One frame of a file selector started with [`Hardware::fsel_start`]:
    /// the chosen name when it closes (empty when cancelled).
    pub fn fsel_step(&mut self, it: &mut Interp) -> Option<Vec<u8>> {
        if self.dialogs.fsel.is_none() {
            return Some(Vec::new());
        }
        self.fs_step(it)
    }

    fn fs_vars(&mut self) -> Option<&mut Vec<DVal>> {
        let i = self.dialogs.channel_index(FSEL_CHANNEL)?;
        Some(&mut self.dialogs.channels[i].vars)
    }

    fn fs_var_int(&mut self, v: usize) -> i32 {
        match self.fs_vars().and_then(|vars| vars.get(v).cloned()) {
            Some(DVal::Int(n)) => n,
            _ => 0,
        }
    }

    fn fs_set_var(&mut self, v: usize, val: DVal) {
        if let Some(vars) = self.fs_vars() {
            vars[v] = val;
        }
    }

    /// Opens the screen and the channel, draws the selector, reads the
    /// directory.
    fn fs_start(
        &mut self,
        it: &mut Interp,
        path: &[u8],
        def: &[u8],
        t1: &[u8],
        t2: &[u8],
    ) -> R<()> {
        let res = Resource::default_resource();
        let old_screen = self.screens.current;
        let limits = self.input.mouse_limits;
        self.input.limit_mouse(Some((128, 30, 448, 312)));
        let mut sy = FS_SY;
        if self
            .dia_rsc_open(
                &res.graphics,
                crate::gfx::screen::MAX_SCREENS - 2,
                FS_SX,
                FS_SY,
                1,
                None,
            )
            .is_err()
        {
            sy = 128;
            self.dia_rsc_open(
                &res.graphics,
                crate::gfx::screen::MAX_SCREENS - 2,
                320,
                128,
                1,
                None,
            )?;
        }
        let n = crate::gfx::screen::MAX_SCREENS - 2;
        if let Some(s) = self.screens.get_mut(n) {
            let _ = s.print_text(b"\x1bC0");
            s.pending_display = [Some(FS_WX), Some(FS_WY + sy / 2), None, Some(2)];
            s.pending_offset[1] = Some(sy / 2 - 1);
        }
        self.dialogs.fsel = Some(Box::new(Fsel {
            list: Vec::new(),
            phase: Phase::Appear(1),
            dev_flag: 0,
            click: -1,
            name1: Vec::new(),
            name2: Vec::new(),
            old_screen,
            limits,
            sorted: false,
            sizes: false,
            tx: 0,
            right_wait: false,
            screen_height: sy,
        }));
        let _ = self.dia_close_channel(it, FSEL_CHANNEL);
        let prog = res.programs[1].clone();
        let buffer = 256 * 5;
        if self
            .dia_open_channel(OpenParams {
                number: FSEL_CHANNEL,
                prog,
                nvar: V_MAX,
                buffer,
                res,
            })
            .is_err()
        {
            return self.fs_abort(it);
        }
        // Fs_GetInputs.
        let mut name1 = path.to_vec();
        self.fs_path_it(&mut name1);
        let prefs = self.dialogs.fs_prefs;
        if let Some(i) = self.dialogs.channel_index(FSEL_CHANNEL) {
            let ch = &mut self.dialogs.channels[i];
            ch.flags |= 0x10;
            if !t1.is_empty() {
                ch.vars[V_TITLE0] = DVal::str(t1);
            }
            if !t2.is_empty() {
                ch.vars[V_TITLE1] = DVal::str(t2);
            }
            ch.vars[V_FILE] = DVal::str(def);
            ch.vars[V_PATH] = DVal::str(&name1);
            ch.vars[V_SORT] = DVal::Int(prefs[0]);
            ch.vars[V_SIZE] = DVal::Int(prefs[1]);
            ch.vars[V_STORE] = DVal::Int(prefs[2]);
            ch.vars[V_ARRAY] = DVal::Arr(ArrRef::Fsel);
        }
        match self.dia_run_program(it, FSEL_CHANNEL, None, Some(0), Some(0)) {
            Ok(RunResult::Value(_)) => {}
            _ => return self.fs_abort(it),
        }
        self.fs_update_store_slider(it);
        self.fs_first(it);
        Ok(())
    }

    /// Error while opening: closes everything (`Fs_BadAPI` = cancel).
    fn fs_abort(&mut self, it: &mut Interp) -> R<()> {
        self.fs_close(it);
        Ok(())
    }

    /// `Dsk.PathIt`: completes a path without a device with the current
    /// directory.
    fn fs_path_it(&self, name: &mut Vec<u8>) {
        if name.contains(&b':') && name.first() != Some(&b':') {
            return;
        }
        let mut cur = latin(&self.files.current_dir);
        if name.first() == Some(&b':') {
            // ":dir": from the root of the current device.
            if let Some(p) = cur.iter().position(|&c| c == b':') {
                cur.truncate(p);
            }
        } else if !cur.is_empty()
            && !cur.ends_with(b":")
            && !cur.ends_with(b"/")
            && !name.is_empty()
        {
            cur.push(b'/');
        }
        cur.extend_from_slice(name);
        *name = cur;
    }

    /// One frame of the selector. Some(result) when it is closed.
    fn fs_step(&mut self, it: &mut Interp) -> Option<Vec<u8>> {
        let n = crate::gfx::screen::MAX_SCREENS - 2;
        let phase = self.dialogs.fsel.as_ref()?.phase.clone();
        let sy = self.dialogs.fsel.as_ref()?.screen_height;
        match phase {
            Phase::Appear(d6) => {
                let half = sy / 2;
                let d6 = (d6 + FS_SPEED).min(half);
                self.fs_app_centre(n, d6, sy);
                self.dialogs.fsel.as_mut()?.phase = if d6 >= half {
                    Phase::Run
                } else {
                    Phase::Appear(d6)
                };
                None
            }
            Phase::Disappear(d6, r) => {
                let d6 = d6 - FS_SPEED;
                if d6 > 0 {
                    self.fs_app_centre(n, d6, sy);
                    self.dialogs.fsel.as_mut()?.phase = Phase::Disappear(d6, r);
                    return None;
                }
                self.fs_close(it);
                Some(r)
            }
            Phase::Message(q) => {
                let r = self.dia_run_resume(it, q);
                if matches!(r, Ok(RunResult::Waiting)) {
                    return None;
                }
                let _ = self.dia_close_channel(it, q);
                self.dialogs.fsel.as_mut()?.phase = Phase::Run;
                None
            }
            Phase::Run => {
                self.fs_run(it);
                None
            }
        }
    }

    /// `AppCentre`: the screen shown on 2*d6 lines around its centre.
    fn fs_app_centre(&mut self, n: usize, d6: i32, sy: i32) {
        if let Some(s) = self.screens.get_mut(n) {
            let d6 = d6.max(1);
            s.pending_display[1] = Some(FS_WY + sy / 2 - d6);
            s.pending_display[3] = Some(2 * d6);
            s.pending_offset[1] = Some(sy / 2 - d6);
        }
    }

    /// Closes the screen and the channel, restores the mouse.
    fn fs_close(&mut self, it: &mut Interp) {
        if let Some(i) = self.dialogs.channel_index(FSEL_CHANNEL) {
            let v = &self.dialogs.channels[i].vars;
            let get = |k: usize| if let DVal::Int(n) = v[k] { n } else { 0 };
            self.dialogs.fs_prefs = [get(V_SORT), get(V_SIZE), get(V_STORE)];
        }
        let _ = self.dia_close_channel(it, FSEL_CHANNEL);
        let n = crate::gfx::screen::MAX_SCREENS - 2;
        if self.screens.get(n).is_some() {
            self.screens.remove(n);
        }
        if let Some(f) = self.dialogs.fsel.take() {
            if let Some(o) = f.old_screen {
                let _ = self.screens.activate(o);
            }
            self.input.limit_mouse(Some(f.limits));
        }
        self.dia_clear_key();
    }

    /// The selector task: tests, then the action of the zone selected.
    fn fs_run(&mut self, it: &mut Interp) {
        if self.dialogs.fsel.as_ref().is_some_and(|f| f.right_wait) {
            if self.input.mouse_buttons & 2 != 0 {
                return;
            }
            self.dialogs.fsel.as_mut().unwrap().right_wait = false;
        }
        let Some((i, mut ch)) = self.dialogs.take(FSEL_CHANNEL) else {
            self.fs_finish(Vec::new());
            return;
        };
        let mut input = 0;
        if ch.rflags & 1 != 0 && self.dia_active(&mut ch).is_ok() {
            if let Edited::Zone(z) = ch.edited {
                self.dia_ed_active(&mut ch, z);
            }
            let _ = self.dia_tests(it, &mut ch);
            self.dia_reactive(&mut ch);
            input = ch.ret as i32;
            ch.ret = 0;
        }
        self.dialogs.put(i, ch);
        if input == 0 && self.input.mouse_buttons & 2 != 0 {
            input = 20;
        }
        match input {
            1 => self.fs_ok(it),
            2 => self.fs_cancel(),
            3 => self.fs_parent(it),
            4 => self.fs_device(it, 1),
            5 => self.fs_device(it, 2),
            6 | 14 => {
                self.fs_new(it);
                self.fs_first(it);
            }
            7 => {
                if self.fs_var_int(V_SORT) != 0
                    && let Some(f) = &mut self.dialogs.fsel
                {
                    f.list.sort_by_key(|e| sort_key(&e.text));
                    f.sorted = true;
                }
                self.fs_aff(it);
            }
            8 => self.fs_aff(it),
            13 => self.fs_name(it),
            15 => self.fs_return(it),
            16 => {
                let v = self.fs_var_int(V_STORE);
                self.dialogs.fs_prefs[2] = v;
            }
            19 => self.fs_help(it),
            20 => {
                let flag = self.dialogs.fsel.as_ref().map_or(0, |f| f.dev_flag);
                match flag {
                    0 => self.fs_device(it, 1),
                    1 => self.fs_device(it, 2),
                    _ => {
                        self.fs_nom_dir();
                        self.fs_new_dir(it, b"");
                    }
                }
                if let Some(f) = &mut self.dialogs.fsel {
                    f.right_wait = true;
                }
            }
            _ => {}
        }
    }

    fn fs_finish(&mut self, r: Vec<u8>) {
        let sy = self.dialogs.fsel.as_ref().map_or(0, |f| f.screen_height);
        if let Some(f) = &mut self.dialogs.fsel {
            f.phase = Phase::Disappear(sy / 2, r);
        }
    }

    /// Value of a zone of the selector.
    fn fs_value(&mut self, zone: i32) -> Vec<u8> {
        match self.dia_get_value(FSEL_CHANNEL, zone, 1) {
            Ok(DVal::Str(s)) => s.to_vec(),
            _ => Vec::new(),
        }
    }

    fn fs_value_int(&mut self, zone: i32) -> i32 {
        match self.dia_get_value(FSEL_CHANNEL, zone, 1) {
            Ok(DVal::Int(n)) => n,
            _ => -1,
        }
    }

    /// `Dialog Update` of a selector zone.
    fn fs_update(
        &mut self,
        it: &mut Interp,
        zone: i32,
        p1: Option<DVal>,
        p2: Option<i32>,
        p3: Option<i32>,
    ) {
        let Some((i, mut ch)) = self.dialogs.take(FSEL_CHANNEL) else {
            return;
        };
        if ch.rflags & 1 != 0 && self.dia_active(&mut ch).is_ok() {
            ch.error = 0;
            let _ = self.dia_zupdate(&mut ch, it, zone, p1, p2, p3);
            self.dia_reactive(&mut ch);
        }
        self.dialogs.put(i, ch);
    }

    /// `Fs_NomDir`: splits the path field into the directory (`Name1`,
    /// ending with ':' or '/') and the filter (`Name2`).
    fn fs_nom_dir(&mut self) {
        let p = self.fs_value(Z_PATH);
        let mut wild = false;
        let mut cut = 0;
        for (i, &c) in p.iter().enumerate() {
            if c == b'*' || c == b'?' {
                wild = true;
            }
            if (c == b':' || c == b'/') && !wild {
                cut = i + 1;
            }
        }
        let (mut n1, n2) = if wild {
            (p[..cut].to_vec(), p[cut..].to_vec())
        } else {
            (p.clone(), Vec::new())
        };
        if !n1.is_empty() && !n1.ends_with(b":") && !n1.ends_with(b"/") {
            n1.push(b'/');
        }
        self.fs_path_it(&mut n1);
        if let Some(f) = &mut self.dialogs.fsel {
            f.name1 = n1;
            f.name2 = n2;
        }
    }

    /// `Fs_New`: empties the list.
    fn fs_new(&mut self, it: &mut Interp) {
        if let Some(f) = &mut self.dialogs.fsel {
            f.list.clear();
            f.click = -1;
        }
        self.fs_set_var(V_PLIST, DVal::Int(0));
        self.fs_aff(it);
    }

    /// `Fs_First`: reads the directory of the path field.
    fn fs_first(&mut self, it: &mut Interp) {
        let sorted = self.fs_var_int(V_SORT) != 0;
        self.fs_set_var(V_POSFIRST, DVal::Int(-1));
        self.fs_nom_dir();
        let Some(f) = &mut self.dialogs.fsel else {
            return;
        };
        f.dev_flag = 0;
        f.click = -1;
        f.sorted = sorted;
        let dir = text(&f.name1);
        let pat = text(&f.name2);
        match self.files.list(&dir) {
            Ok(entries) => {
                let mut list: Vec<Entry> = entries
                    .iter()
                    .filter(|e| {
                        e.is_dir || pat.is_empty() || crate::files::wildcard_match(&pat, &e.name)
                    })
                    .map(|e| {
                        let mut t = vec![if e.is_dir { b'*' } else { b' ' }];
                        t.extend(latin(&e.name).into_iter().take(FILL_SIZE - 2));
                        Entry {
                            text: t,
                            size: (!e.is_dir).then_some(e.size),
                        }
                    })
                    .collect();
                if sorted {
                    list.sort_by_key(|e| sort_key(&e.text));
                }
                if let Some(f) = &mut self.dialogs.fsel {
                    f.list = list;
                }
                self.fs_aff(it);
            }
            Err(e) => {
                let m = match e {
                    crate::files::FsError::DirNotFound => 25,
                    crate::files::FsError::NotFound => 26,
                    crate::files::FsError::BadName => 27,
                    _ => 32,
                };
                self.fs_message(it, m);
            }
        }
    }

    /// `Fs_Error`: shows a message with program 3 of the default resource.
    fn fs_message(&mut self, it: &mut Interp, m: i32) {
        let res = Resource::default_resource();
        let msg = res.messages.get(m);
        let prog = res.programs[2].clone();
        let q = self.dialogs.free_quick_channel();
        self.fs_new(it);
        if self
            .dia_open_channel(OpenParams {
                number: q,
                prog,
                nvar: 16,
                buffer: 256,
                res,
            })
            .is_err()
        {
            return;
        }
        if let Some(i) = self.dialogs.channel_index(q) {
            self.dialogs.channels[i].vars[1] = DVal::Str(Rc::from(msg));
        }
        match self.dia_run_program(it, q, None, Some(0), Some(0)) {
            Ok(RunResult::Waiting) => {
                if let Some(f) = &mut self.dialogs.fsel {
                    f.phase = Phase::Message(q);
                }
            }
            _ => {
                let _ = self.dia_close_channel(it, q);
            }
        }
    }

    /// `_AffF`: new size of the list, slider and list redrawn.
    fn fs_aff(&mut self, it: &mut Interp) {
        let sizes = self.fs_var_int(V_SIZE) != 0;
        let tx = self.fs_var_int(V_TX).max(0) as usize;
        let count = {
            let Some(f) = &mut self.dialogs.fsel else {
                return;
            };
            f.sizes = sizes;
            f.tx = tx;
            f.list.len() as i32
        };
        if let Some(i) = self.dialogs.channel_index(FSEL_CHANNEL) {
            let ch = &mut self.dialogs.channels[i];
            if let Some(z) = ch.find_zone(Z_LIST, 1)
                && let Some(Zone {
                    kind: ZoneKind::List(l),
                    ..
                }) = ch.zone_mut(z)
            {
                l.max_act = count as i16;
            }
        }
        let plist = self.fs_var_int(V_PLIST);
        self.fs_set_var(V_AFFFLAG, DVal::Int(0));
        self.fs_update(it, Z_SLIDER, Some(DVal::Int(plist)), None, Some(count));
        self.fs_set_var(V_AFFFLAG, DVal::Int(1));
    }

    /// `_SliStore`: the store slider (no stored directory here).
    fn fs_update_store_slider(&mut self, it: &mut Interp) {
        self.fs_update(it, Z_STORE_SLIDER, Some(DVal::Int(0)), Some(1), Some(1));
    }

    /// `Fs_NewDir`: path field = Name1 + `add` + filter, then read.
    fn fs_new_dir(&mut self, it: &mut Interp, add: &[u8]) {
        self.fs_new(it);
        let Some(f) = &self.dialogs.fsel else { return };
        let mut p = f.name1.clone();
        p.extend_from_slice(add);
        if !f.name2.is_empty() {
            if !p.is_empty() && !p.ends_with(b"/") && !p.ends_with(b":") {
                p.push(b'/');
            }
            p.extend_from_slice(&f.name2);
        }
        if p.len() >= 256 {
            self.fs_cancel();
            return;
        }
        self.fs_update(it, Z_PATH, Some(DVal::str(&p)), None, None);
        if let Some(f) = &mut self.dialogs.fsel {
            f.click = -1;
        }
        self.fs_first(it);
    }

    /// `Fs_NewName`: sets the name field.
    fn fs_new_name(&mut self, it: &mut Interp, name: &[u8]) {
        self.fs_update(it, Z_FILE, Some(DVal::str(name)), None, None);
    }

    /// `Fs_Ok`: the result is the directory followed by the name; the
    /// directory becomes the current one.
    fn fs_ok(&mut self, _it: &mut Interp) {
        self.fs_nom_dir();
        let name1 = self
            .dialogs
            .fsel
            .as_ref()
            .map(|f| f.name1.clone())
            .unwrap_or_default();
        let d = text(&name1);
        let _ = self.files.set_current_dir(d.trim_end_matches('/'));
        let file = self.fs_value(Z_FILE);
        if file.is_empty() {
            self.fs_cancel();
            return;
        }
        let mut r = name1;
        r.extend_from_slice(&file);
        self.fs_finish(r);
    }

    fn fs_cancel(&mut self) {
        self.fs_finish(Vec::new());
    }

    /// `Fs_Parent`.
    fn fs_parent(&mut self, it: &mut Interp) {
        self.fs_nom_dir();
        let Some(f) = &mut self.dialogs.fsel else {
            return;
        };
        let n = &mut f.name1;
        if !n.ends_with(b"/") {
            return;
        }
        let mut end = n.len() - 1;
        while end > 0 {
            let c = n[end - 1];
            if c == b'/' || c == b':' {
                break;
            }
            end -= 1;
        }
        n.truncate(end);
        self.fs_new_dir(it, b"");
    }

    /// `Fs_Device` / `Fs_Assign`: list of devices (1) or volumes (2).
    fn fs_device(&mut self, it: &mut Interp, flag: u8) {
        self.fs_new(it);
        let mut names: Vec<String> = if flag == 1 {
            self.files
                .volumes
                .iter()
                .map(|v| {
                    v.aliases
                        .first()
                        .cloned()
                        .unwrap_or_else(|| v.name.clone())
                        .to_uppercase()
                })
                .collect()
        } else {
            self.files.volume_names()
        };
        names.sort_by_key(|n| n.to_uppercase());
        names.dedup();
        if let Some(f) = &mut self.dialogs.fsel {
            f.list = names
                .iter()
                .map(|n| {
                    let mut t = vec![b' '];
                    t.extend(latin(n));
                    t.push(b':');
                    Entry {
                        text: t,
                        size: None,
                    }
                })
                .collect();
            f.dev_flag = flag;
        }
        self.fs_aff(it);
    }

    /// `Fs_Name`: a click in the list.
    fn fs_name(&mut self, it: &mut Interp) {
        let p = self.fs_value_int(Z_LIST);
        let Some(f) = &self.dialogs.fsel else { return };
        let Some(e) = f.list.get(p.max(0) as usize).filter(|_| p >= 0).cloned() else {
            return;
        };
        let name = e.text[1..].to_vec();
        if f.dev_flag != 0 {
            if let Some(f) = &mut self.dialogs.fsel {
                f.click = -1;
                f.dev_flag = 0;
                f.name1.clear();
            }
            self.fs_new_dir(it, &name);
            return;
        }
        if e.text[0] != b' ' {
            self.fs_nom_dir();
            let mut d = name;
            d.push(b'/');
            self.fs_new_dir(it, &d);
            return;
        }
        if f.click == p {
            self.fs_ok(it);
            return;
        }
        if let Some(f) = &mut self.dialogs.fsel {
            f.click = p;
        }
        self.fs_new_name(it, &name);
    }

    /// `Fs_Return`: Return in the name field; a name with a path changes
    /// the directory.
    fn fs_return(&mut self, it: &mut Interp) {
        let v = self.fs_value(Z_FILE);
        let cut = v
            .iter()
            .rposition(|&c| c == b':' || c == b'/')
            .map_or(0, |i| i + 1);
        if cut == 0 {
            self.fs_ok(it);
            return;
        }
        let (path, name) = (v[..cut].to_vec(), v[cut..].to_vec());
        self.fs_new_name(it, &name);
        self.fs_nom_dir();
        if path.contains(&b':')
            && let Some(f) = &mut self.dialogs.fsel
        {
            f.name1.clear();
        }
        self.fs_new_dir(it, &path);
    }

    /// `Fs_Help`: scrolls the list to the first name starting with the
    /// name field.
    fn fs_help(&mut self, it: &mut Interp) {
        let v = self.fs_value(Z_FILE);
        if v.is_empty() {
            return;
        }
        let ty = self.fs_var_int(V_TY);
        let Some(f) = &self.dialogs.fsel else { return };
        let key: Vec<u8> = std::iter::once(b' ')
            .chain(v.iter().copied())
            .map(upper)
            .collect();
        let Some(p) = f.list.iter().position(|e| {
            e.text
                .iter()
                .map(|&c| upper(c))
                .collect::<Vec<_>>()
                .starts_with(&key)
        }) else {
            return;
        };
        let count = f.list.len() as i32;
        let mut p = p as i32 + ty;
        if p > count {
            p = count;
        }
        p = (p - ty).max(0);
        self.fs_set_var(V_PLIST, DVal::Int(p));
        self.fs_set_var(V_POSFIRST, DVal::Int(p));
        self.fs_aff(it);
    }

    /// Kind of the blocking operation of the current instruction.
    fn dia_reentry_kind(&self, it: &Interp) -> Option<Blocking> {
        let (pos, b) = self.dialogs.blocking.as_ref()?;
        let w = it.wait.as_ref()?;
        (*pos == it.inst_pos && w.pos == it.inst_pos).then(|| b.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_with_sizes() {
        let f = Fsel {
            list: vec![
                Entry {
                    text: b" file.txt".to_vec(),
                    size: Some(1234),
                },
                Entry {
                    text: b"*dir".to_vec(),
                    size: None,
                },
            ],
            phase: Phase::Run,
            dev_flag: 0,
            click: -1,
            name1: Vec::new(),
            name2: Vec::new(),
            old_screen: None,
            limits: (0, 0, 0, 0),
            sorted: true,
            sizes: true,
            tx: 20,
            right_wait: false,
            screen_height: 158,
        };
        assert_eq!(f.display(0), b" file.txt    1234   ");
        assert_eq!(f.display(1), b"*dir");
        assert!(sort_key(b"*Zeta") < sort_key(b" alpha"));
    }
}
