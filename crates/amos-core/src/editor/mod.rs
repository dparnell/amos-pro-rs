//! The AMOS Professional editor (`+Edit.s`).
//!
//! The editor is the program that starts when AMOS is launched without a
//! program. Like the original it is built from AMOS screens: screen 9
//! (`EcEdit`, 640x256 hires, 8 colours) holds the top bar of image buttons
//! from the editor resource bank and the program windows; screen 8
//! (`EcFonc`) is used for the Direct mode strip and for the "end of
//! program" line shown over the program's screens.
//!
//! [`Editor::vbl`] is called at each vertical blank instead of
//! [`Machine::vbl`]: while a program runs it forwards to the machine, and
//! when the program stops control comes back to the editor.
//!
//! The dialogs (search, goto line, about...) are the Interface programs of
//! the editor resource bank and the file selector is `Fsel$` (see
//! `dialogs.rs`).
//!
//! Integration points not wired yet:
//! * TODO(menus): the editor menu is drawn by `menu.rs` on the editor
//!   screen rather than with the AMOS menu system.

pub mod build;
pub mod config;
pub mod dialogs;
pub mod direct;
pub mod draw;
pub mod highlight;
pub mod menu;
pub mod resource;
pub mod text;

use crate::Machine;
use crate::detok::latin1_to_string;
use crate::gfx::Screen;
use crate::input::KeyPress;
use crate::interface::DVal;
use crate::interp::verify::Verifier;
use crate::interp::{RunState, StopInfo, StopReason, StopReasonOrError};
use crate::program::Program;
use config::EdConfig;
use dialogs::Then;
use draw::col;
use resource::*;
use text::{Doc, EditError};

/// Screen numbers (`+Equ.s:763`).
pub const EC_FONC: usize = 8;
pub const EC_EDIT: usize = 9;

/// Heights of the parts of a program window (`+Edit.s:75-78`).
const TITLE_SY: i32 = 16;
const ETAT_SY: i32 = 11;
const BAS_SY: i32 = 5;

/// Height of the `Ed_Ligne` strip.
const LIGNE_SY: i32 = 56;

/// Size of the text buffer shown as "Free" (`PI_DefSize`).
const TEXT_BUFFER: i64 = 64 * 1024;

/// What the editor is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// Editing programs.
    Edit,
    /// A program runs (editor hidden).
    Running,
    /// The program stopped: line asking Direct mode / Editor (`Ed_Ligne`).
    Stopped(StopInfo),
    /// Direct mode (`Esc_Loop`).
    Direct,
    /// A direct mode line is executing.
    DirectRunning,
}

/// The editor.
pub struct Editor {
    pub cfg: EdConfig,
    res: Resource,
    menu_root: menu::MenuNode,
    menu: menu::MenuUi,
    /// Programs, one per window, top to bottom.
    pub docs: Vec<Doc>,
    /// Text rows of each window.
    heights: Vec<usize>,
    pub current: usize,
    pub insert: bool,
    pub mode: Mode,
    /// The editor screen while it is not displayed.
    screen: Option<Box<Screen>>,
    dirty: bool,
    /// Status line alert (`Ed_Alert`) and the last one (`Ed_RAlert`).
    alert: Option<String>,
    last_alert: String,
    clipboard: Vec<Vec<u8>>,
    search: Vec<u8>,
    replace: Vec<u8>,
    /// Dialog or file selector in progress, and the current screen to
    /// restore after it.
    modal: Option<dialogs::Modal>,
    dialog_screen: Option<Option<usize>>,
    /// Search options (`Ed_SchMode`): bit 0 upper case = lower case, bit 1
    /// backwards, bit 2 in the block, bit 3 all occurrences.
    search_mode: u16,
    /// Text buffer size shown as "Free" (`Set Text Buffer`).
    text_buffer: i64,
    /// The "save changes?" question was answered for the function called.
    skip_saved: bool,
    /// Build Application: native, web, folder files; a build in progress
    /// and the number of results before it.
    build_options: [bool; 3],
    building: bool,
    build_waiting: usize,
    direct: direct::DirectMode,
    blink: u32,
    mouse_prev: u8,
    /// Window being dragged on its slider.
    slider_drag: Option<usize>,
    /// Document that was run.
    running_doc: usize,
    /// The program running is an accessory called by an editor function
    /// (the Help): the programs being edited are left as they are.
    accessory: bool,
    /// Quit was chosen (the platform closes the window).
    pub quit_requested: bool,
    /// Program screens hidden while the editor is shown.
    hidden_screens: Vec<usize>,
}

/// Bytes of a string (Latin-1).
fn bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
}

impl Editor {
    /// `Ed_Cold` + `Ed_Title`: loads the configuration and resources from
    /// `AMOSPro_System:` (or the built-in copies) and opens the editor
    /// screen with an empty program.
    pub fn new(m: &mut Machine) -> Editor {
        let cfg =
            m.hw.files
                .read("AMOSPro_System:AMOSPro_Editor_Config")
                .ok()
                .and_then(|d| EdConfig::parse(&d))
                .unwrap_or_else(EdConfig::defaults);
        let res_name = latin1_to_string(cfg.sys(45));
        let res =
            m.hw.files
                .read(&format!("AMOSPro_System:{res_name}"))
                .ok()
                .and_then(|d| Resource::parse(&d))
                .unwrap_or_else(Resource::defaults);
        let menu_root = menu::build(&cfg);
        let direct = direct::DirectMode::new(&cfg);
        let search_mode = cfg.search_mode;
        let mut ed = Editor {
            insert: cfg.insert,
            cfg,
            res,
            menu_root,
            menu: Default::default(),
            docs: vec![Doc::new()],
            heights: vec![0],
            current: 0,
            mode: Mode::Edit,
            screen: None,
            dirty: true,
            alert: None,
            last_alert: String::new(),
            clipboard: Vec::new(),
            search: Vec::new(),
            replace: Vec::new(),
            modal: None,
            dialog_screen: None,
            search_mode,
            text_buffer: TEXT_BUFFER,
            skip_saved: false,
            build_options: [true; 3],
            building: false,
            build_waiting: 0,
            direct,
            blink: 0,
            mouse_prev: 0,
            slider_drag: None,
            running_doc: 0,
            accessory: false,
            quit_requested: false,
            hidden_screens: Vec::new(),
        };
        ed.layout();
        let s = ed.new_screen();
        ed.screen = Some(Box::new(s));
        ed.show(m);
        ed
    }

    fn new_screen(&self) -> Screen {
        let c = &self.cfg;
        let mode = 0x8000 | if c.interlace { 4 } else { 0 };
        let h = (c.sy as u32 + 7) & !7;
        // 16 colours: 8 for the editor (`Ed_Palette`), 8 for the syntax
        // highlighting.
        let mut s = Screen::new(EC_EDIT, c.sx as u32, h, 16, mode);
        s.palette[..8].copy_from_slice(&c.palette);
        s.palette[8..8 + HIGHLIGHT_PALETTE.len()].copy_from_slice(&HIGHLIGHT_PALETTE);
        s.pending_display = [Some(c.wx as i32), Some(c.wy as i32), None, None];
        s.apply_pending();
        s
    }

    /// Number of text rows available to the windows (`Ed_Ty` minus the
    /// window frames).
    fn rows_total(&self) -> usize {
        let sy = ((self.cfg.sy as i32 + 7) & !7) - TITLE_SY;
        ((sy - (ETAT_SY + BAS_SY) * self.docs.len() as i32) / 8).max(self.docs.len() as i32) as usize
    }

    /// Shares the rows between the windows (`Edt_WMaxSize`).
    fn layout(&mut self) {
        let total = self.rows_total();
        let n = self.docs.len();
        self.heights = vec![total / n; n];
        let extra = total - total / n * n;
        if let Some(h) = self.heights.get_mut(self.current) {
            *h += extra;
        }
        self.dirty = true;
    }

    /// Displays the editor screen in front (`Ed_Appear`).
    fn show(&mut self, m: &mut Machine) {
        if let Some(s) = self.screen.take() {
            let cur = m.hw.screens.current;
            m.hw.screens.insert(*s);
            m.hw.screens.current = cur.or(Some(EC_EDIT));
        }
        let _ = m.hw.screens.to_front(EC_EDIT);
        // The program's screens are not shown around the editor: they are
        // hidden until the editor goes away.
        for n in 0..EC_FONC {
            if let Some(s) = m.hw.screens.get_mut(n)
                && !s.hidden
            {
                s.hidden = true;
                self.hidden_screens.push(n);
            }
        }
        // The editor always has a mouse pointer.
        m.hw.sprites.mouse_show = m.hw.sprites.mouse_show.max(0);
        self.dirty = true;
    }

    /// Removes the editor screen from the display (`Ed_Hide`).
    fn hide(&mut self, m: &mut Machine) {
        for n in self.hidden_screens.drain(..) {
            if let Some(s) = m.hw.screens.get_mut(n) {
                s.hidden = false;
            }
        }
        if let Some(s) = m.hw.screens.screens.get_mut(EC_EDIT).and_then(Option::take) {
            m.hw.screens.priority.retain(|&p| p != EC_EDIT);
            if m.hw.screens.current == Some(EC_EDIT) {
                m.hw.screens.current = m.hw.screens.priority.iter().copied().find(|&p| p < 8);
            }
            self.screen = Some(s);
        }
    }

    /// Area of the display showing the editor screen.
    pub fn display_rect(&self, m: &Machine) -> crate::display::Rect {
        match &self.screen {
            Some(s) => s.display_rect(),
            None => m.hw.screens.get(EC_EDIT).map_or_else(|| self.new_screen().display_rect(), Screen::display_rect),
        }
    }

    pub fn doc(&self) -> &Doc {
        &self.docs[self.current]
    }

    pub fn doc_mut(&mut self) -> &mut Doc {
        &mut self.docs[self.current]
    }

    /// Shows a message in the status line of the current window.
    pub fn alert(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.last_alert = msg.clone();
        self.alert = Some(msg);
        self.dirty = true;
    }

    fn alert_message(&mut self, n: usize) {
        let m = self.cfg.message(n);
        self.alert(m);
    }

    fn edit_error(&mut self, e: EditError) {
        self.alert_message(e.message());
    }

    /// Current alert text, if any (for tests and tools).
    pub fn current_alert(&self) -> Option<&str> {
        self.alert.as_deref()
    }

    // ------------------------------------------------------------------
    // Programs
    // ------------------------------------------------------------------

    /// Puts a loaded program in the current window.
    pub fn set_program(&mut self, prg: &Program, name: &str) {
        self.docs[self.current] = Doc::from_program(prg, name);
        self.dirty = true;
    }

    /// Loads a program (`.AMOS` or ASCII) from an AMOS path into the
    /// current window.
    pub fn load(&mut self, m: &mut Machine, path: &str) -> Result<(), String> {
        let data = m.hw.files.read(path).map_err(|_| self.cfg.message(184))?;
        let prg = load_program_data(&data).map_err(|e| e.unwrap_or_else(|| self.cfg.message(207)))?;
        self.set_program(&prg, path);
        Ok(())
    }

    /// Saves the current program under its name.
    pub fn save(&mut self, m: &mut Machine) -> Result<(), String> {
        let name = self.doc().name.clone();
        self.save_as(m, &name)
    }

    pub fn save_as(&mut self, m: &mut Machine, path: &str) -> Result<(), String> {
        let d = &mut self.docs[self.current];
        d.commit().map_err(|e| self.cfg.message(e.message()))?;
        let mut path = path.to_string();
        if !path.to_ascii_lowercase().ends_with(".amos") && !path.contains('.') {
            path.push_str(".AMOS");
        }
        let data = d.to_program().save();
        m.hw.files.write(&path, &data).map_err(|_| self.cfg.message(184))?;
        let d = &mut self.docs[self.current];
        d.name = path;
        d.modified = false;
        self.dirty = true;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Run / Test
    // ------------------------------------------------------------------

    /// `Ed_Run` (F1): stores the current line, hides the editor and starts
    /// the program. Test-time errors are shown in the status line with the
    /// cursor on the error.
    pub fn run(&mut self, m: &mut Machine) {
        if let Err(e) = self.doc_mut().commit() {
            return self.edit_error(e);
        }
        self.doc_mut().block_forget();
        let prg = self.doc().to_program();
        self.hide(m);
        match m.run_program(&prg) {
            Ok(()) => {
                self.running_doc = self.current;
                self.doc_mut().untested = false;
                self.mode = Mode::Running;
            }
            Err(e) => {
                self.show(m);
                self.show_error(e.pos, self.cfg.test_message(e.code));
            }
        }
    }

    /// `Ed_Test` (F2).
    pub fn test(&mut self) {
        if let Err(e) = self.doc_mut().commit() {
            return self.edit_error(e);
        }
        let prg = self.doc().to_program();
        match Verifier::verify(&prg.source, prg.math_flags) {
            Ok(_) => {
                self.doc_mut().untested = false;
                self.alert_message(197);
            }
            Err(e) => self.show_error(e.pos, self.cfg.test_message(e.code)),
        }
    }

    /// Puts the cursor on the error at code offset `pos` and shows the
    /// message (`Ed_ErrEdit`).
    fn show_error(&mut self, pos: usize, msg: String) {
        let d = self.doc_mut();
        if let Some((y, x)) = d.locate_offset(pos) {
            d.reveal(y, x);
        }
        self.ensure_visible();
        self.alert(format!("{msg}."));
    }

    /// The program stopped: back to the editor, or the Direct / Editor
    /// choice line (`Ed_ErrRun`).
    fn program_stopped(&mut self, m: &mut Machine, info: StopInfo) {
        if self.accessory {
            return self.accessory_stopped(m, info);
        }
        // Banks belong to the program (saved with it).
        let banks: Vec<_> = m.hw.banks.banks.values().filter(|b| b.data_bank).cloned().collect();
        if let Some(d) = self.docs.get_mut(self.running_doc) {
            d.banks = banks;
        }
        // Menus of the program are removed (`EdM_Program` / `EdM_Init`).
        m.hw.menus = Default::default();
        m.hw.log.clear();
        match &info.reason {
            StopReasonOrError::Stop(StopReason::Edit) | StopReasonOrError::Stop(StopReason::System) => {
                self.back_to_editor(m, None)
            }
            StopReasonOrError::Stop(StopReason::Direct) => self.enter_direct(m),
            StopReasonOrError::Test(n) => {
                let msg = self.cfg.test_message(*n);
                self.back_to_editor(m, Some((info.pos, msg)));
            }
            // End, errors, Control-C: the Direct mode / Editor line.
            _ => {
                self.mode = Mode::Stopped(info.clone());
                self.ligne(m, info);
            }
        }
    }

    fn stop_message(&self, info: &StopInfo) -> String {
        match &info.reason {
            StopReasonOrError::Stop(StopReason::Break) => self.cfg.run_message(9),
            StopReasonOrError::Stop(_) => self.cfg.run_message(10),
            StopReasonOrError::Error(n) => self.cfg.run_message(*n),
            StopReasonOrError::Message(s) => s.clone(),
            StopReasonOrError::Test(n) => self.cfg.test_message(*n),
        }
    }

    /// `Ed_PrgCommand` for an accessory (`.Hidden`): runs program `name`
    /// with `cmd` as its `Command Line$` (None: the current line from the
    /// word at the cursor), then comes back to the editor when it ends.
    fn run_accessory(&mut self, m: &mut Machine, name: &str, cmd: Option<Vec<u8>>) {
        if let Err(e) = self.doc_mut().commit() {
            return self.edit_error(e);
        }
        let cmd = cmd.unwrap_or_else(|| {
            let d = self.doc();
            let text = d.current_text();
            let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'#';
            let mut x = d.x.min(text.len());
            while x > 0 && word(text[x - 1]) {
                x -= 1;
            }
            text[x..].to_vec()
        });
        let prg = match m.hw.files.read(name).ok().map(|d| load_program_data(&d)) {
            Some(Ok(p)) => p,
            Some(Err(e)) => return self.alert(e.unwrap_or_else(|| self.cfg.message(207))),
            None => return self.alert(format!("{}{name}", self.cfg.message(184))),
        };
        self.hide(m);
        match m.run_program(&prg) {
            Ok(()) => {
                m.hw.command_line = cmd;
                self.accessory = true;
                self.mode = Mode::Running;
            }
            Err(e) => {
                self.show(m);
                self.alert(format!("{name}: {}", self.cfg.test_message(e.code)));
            }
        }
    }

    /// End of an accessory: back to the editor as it was, with the error
    /// if the accessory stopped on one.
    fn accessory_stopped(&mut self, m: &mut Machine, info: StopInfo) {
        self.accessory = false;
        m.hw.menus = Default::default();
        m.hw.log.clear();
        let error = match &info.reason {
            StopReasonOrError::Stop(_) => None,
            _ => Some(self.stop_message(&info)),
        };
        self.mode = Mode::Edit;
        self.show(m);
        m.hw.input.clear_keys();
        if let Some(e) = error {
            self.alert(e);
        }
    }

    /// Shows the editor again, with an error to point at.
    fn back_to_editor(&mut self, m: &mut Machine, error: Option<(usize, String)>) {
        self.direct.close(m);
        self.mode = Mode::Edit;
        self.current = self.running_doc.min(self.docs.len() - 1);
        self.show(m);
        m.hw.input.clear_keys();
        if let Some((pos, msg)) = error {
            self.show_error(pos, msg);
        }
    }

    /// `Ed_Escape`: Direct mode over the program's screens.
    pub fn enter_direct(&mut self, m: &mut Machine) {
        if let Err(e) = self.doc_mut().commit() {
            return self.edit_error(e);
        }
        self.hide(m);
        self.mode = Mode::Direct;
        self.direct.open(m, &self.cfg, &self.res);
    }

    // ------------------------------------------------------------------
    // Main loop
    // ------------------------------------------------------------------

    /// One vertical blank: runs the program, or handles the editor input
    /// and display.
    pub fn vbl(&mut self, m: &mut Machine) {
        m.vbl();
        match self.mode.clone() {
            Mode::Running => {
                if let RunState::Stopped(info) = &m.state {
                    let info = info.clone();
                    m.state = RunState::Idle;
                    self.program_stopped(m, info);
                }
            }
            Mode::Stopped(info) => {
                // The `Ed_Ligne` dialog runs until Esc / Return / a click.
                if self.modal.is_some() {
                    self.modal_vbl(m);
                } else {
                    // The dialog could not be shown: back to the editor.
                    self.stopped_to_editor(m, &info);
                }
            }
            Mode::Direct => {
                let _ = m.hw.input.take_break();
                match self.direct.input(m, &self.cfg, &self.res) {
                    direct::Action::None => {}
                    direct::Action::Editor => self.back_to_editor(m, None),
                    direct::Action::Run(line) => {
                        if self.direct_run(m, &line) {
                            self.mode = Mode::DirectRunning;
                        }
                    }
                }
            }
            Mode::DirectRunning => {
                if let RunState::Stopped(info) = &m.state {
                    let info = info.clone();
                    m.state = RunState::Idle;
                    self.direct_stopped(m, info);
                }
            }
            Mode::Edit => self.edit_vbl(m),
        }
    }

    /// `Ed_Ligne` (+Edit.s:8330): the editor screen is shown as a 56 line
    /// strip at `Es_Y1` over the program's screens, and the dialog at label
    /// 59 of the editor resource shows the message ("... at line n."), the
    /// line around the error with `>>>` at the error, and the buttons
    /// "Direct mode [ESC]" / "Editor [RETURN]". The original's `CA 4VA`
    /// calls `EdReCop` (wait VBL, force the copper list): nothing to do
    /// here, the display is rebuilt at each frame.
    fn ligne(&mut self, m: &mut Machine, info: StopInfo) {
        let msg = self.stop_message(&info);
        self.current = self.running_doc.min(self.docs.len() - 1);
        let d = &mut self.docs[self.current];
        let (y, col) = match d.locate_offset(info.pos) {
            Some(l) => l,
            None => (d.len().saturating_sub(1), d.line_text(d.len().saturating_sub(1)).len()),
        };
        let text = d.line_text(y);
        // Variables 2 and 3: up to 13 characters before the error, up to 60
        // from it.
        let col = col.min(text.len());
        let before = &text[col.saturating_sub(13)..col];
        let after = &text[col..(col + 60).min(text.len())];
        let vars = [
            (0, DVal::str(&bytes(&msg))),
            (1, DVal::Int(y as i32 + 1)),
            (2, DVal::str(before)),
            (3, DVal::str(after)),
            (4, DVal::Int(0)),
        ];
        // The editor screen in front, 56 lines at Es_Y1, program screens
        // still visible around it (`EHide`, `First`, `AView`, `OffSet`).
        if let Some(s) = self.screen.take() {
            let cur = m.hw.screens.current;
            m.hw.screens.insert(*s);
            m.hw.screens.current = cur;
        }
        let _ = m.hw.screens.to_front(EC_EDIT);
        if let Some(s) = m.hw.screens.get_mut(EC_EDIT) {
            s.pending_display = [Some(self.cfg.wx as i32), Some(self.cfg.esc_y1 as i32), None, Some(LIGNE_SY)];
            s.pending_offset = [Some(0), Some(0)];
            s.apply_pending();
        }
        // The mouse on the strip (`Esc_MaxMouse`, `SetM`).
        m.hw.sprites.mouse_show = m.hw.sprites.mouse_show.max(0);
        m.hw.input.limit_mouse(None);
        m.hw.input.set_mouse(Some(self.cfg.wx as i32 + 1), Some(self.cfg.esc_y1 as i32 + 1));
        self.dialog(m, dialogs::label::LIGNE, &vars, Then::Ligne(info));
    }

    /// End of `Ed_Ligne`: button 1 (Esc) Direct mode, else the editor.
    pub(super) fn ligne_done(&mut self, m: &mut Machine, info: StopInfo, ret: i32) {
        self.full_screen(m);
        if ret == 1 {
            self.hide(m);
            self.mode = Mode::Direct;
            self.direct.open(m, &self.cfg, &self.res);
        } else {
            self.stopped_to_editor(m, &info);
        }
    }

    /// The editor screen at its normal position and size.
    fn full_screen(&mut self, m: &mut Machine) {
        let h = (self.cfg.sy as i32 + 7) & !7;
        let s = match m.hw.screens.get_mut(EC_EDIT) {
            Some(s) => s,
            None => match self.screen.as_deref_mut() {
                Some(s) => s,
                None => return,
            },
        };
        s.pending_display = [Some(self.cfg.wx as i32), Some(self.cfg.wy as i32), None, Some(h)];
        s.pending_offset = [Some(0), Some(0)];
        s.apply_pending();
    }

    fn stopped_to_editor(&mut self, m: &mut Machine, info: &StopInfo) {
        let error = match &info.reason {
            StopReasonOrError::Stop(StopReason::End) => None,
            _ => Some((info.pos, self.stop_message(info))),
        };
        self.back_to_editor(m, error);
    }

    /// Tokenises and starts a direct mode line (`Esc_R`).
    fn direct_run(&mut self, m: &mut Machine, line: &[u8]) -> bool {
        let double = m.interp.double;
        let tok = match crate::tokenise::tokenise_line_with(line, double) {
            Ok(Some(t)) => t.line,
            Ok(None) => return false,
            Err(_) => {
                let msg = self.cfg.message(199);
                self.direct.print_error(m, &self.cfg, &self.res, &msg);
                return false;
            }
        };
        // An empty line does nothing.
        if tok.len() <= 4 {
            return false;
        }
        match m.run_direct(&tok) {
            Ok(()) => true,
            Err(e) => {
                let msg = self.cfg.test_message(e.code);
                self.direct.print_error(m, &self.cfg, &self.res, &msg);
                false
            }
        }
    }

    /// End of a direct mode line (`Ed_ErrDirect`).
    fn direct_stopped(&mut self, m: &mut Machine, info: StopInfo) {
        m.hw.log.clear();
        match &info.reason {
            StopReasonOrError::Stop(StopReason::End | StopReason::Direct) => {
                self.mode = Mode::Direct;
                self.direct.redraw(m, &self.cfg, &self.res);
            }
            StopReasonOrError::Stop(StopReason::Edit | StopReason::System) => self.back_to_editor(m, None),
            _ => {
                self.mode = Mode::Direct;
                let msg = self.stop_message(&info);
                self.direct.print_error(m, &self.cfg, &self.res, &msg);
            }
        }
    }

    fn edit_vbl(&mut self, m: &mut Machine) {
        if self.modal.is_some() {
            self.modal_vbl(m);
            if self.modal.is_some() || self.mode != Mode::Edit {
                return;
            }
        }
        self.blink = self.blink.wrapping_add(1);
        if self.blink.is_multiple_of(16) {
            self.dirty = true;
        }
        // Keys.
        while let Some(k) = m.hw.input.inkey() {
            if self.menu.open {
                continue;
            }
            self.key(m, k);
            if self.mode != Mode::Edit || self.modal.is_some() {
                return;
            }
        }
        // Control-C is the Cut shortcut in the editor.
        if m.hw.input.take_break() {
            let k = KeyPress { shift: 0x08, raw: 0x33, ascii: b'c' };
            self.key(m, k);
        }
        self.mouse(m);
        self.build_poll(m);
        if self.mode == Mode::Edit && self.dirty && !self.menu.open && self.modal.is_none() {
            self.dirty = false;
            self.redraw(m);
        }
    }

    // ------------------------------------------------------------------
    // Keyboard
    // ------------------------------------------------------------------

    /// `Ed_Key`: a key press in the editor.
    pub fn key(&mut self, m: &mut Machine, k: KeyPress) {
        self.dirty = true;
        self.alert = None;
        // Amiga+Shift+A: Build Application (not in the original table).
        let f = if k.raw == 0x20 && k.shift & config::AMI != 0 && k.shift & config::SHF != 0 && k.shift & 0x38 == 0 {
            Some(build::BUILD_FUNCTION)
        } else {
            self.cfg.function_for_key(&k)
        };
        if let Some(f) = f {
            self.function(m, f);
        } else if k.ascii >= 32 {
            let ins = self.insert;
            if let Err(e) = self.doc_mut().type_char(k.ascii, ins) {
                self.edit_error(e);
            }
        }
        self.ensure_visible();
    }

    /// `Ed_FCall`: executes editor function `f` (numbers of `JFonc`,
    /// +Edit.s:3150).
    pub fn function(&mut self, m: &mut Machine, f: u16) {
        self.dirty = true;
        let tabs = self.cfg.tabs as usize;
        let rows = self.heights[self.current].max(1);
        let r = match f {
            1 => {
                self.doc_mut().move_up();
                Ok(())
            }
            2 => {
                self.doc_mut().move_down();
                Ok(())
            }
            3 => {
                self.doc_mut().move_left();
                Ok(())
            }
            4 => {
                self.doc_mut().move_right();
                Ok(())
            }
            5 => {
                let top = self.doc().top;
                self.doc_mut().goto_row(top)
            }
            6 => {
                let r = self.doc().top + rows - 1;
                self.doc_mut().goto_row(r)
            }
            7 => {
                self.doc_mut().word_left();
                Ok(())
            }
            8 => {
                self.doc_mut().word_right();
                Ok(())
            }
            9 | 10 => {
                // Page up / down (`Ed_PHaut`, `Ed_PBas`): by the window
                // height minus 2.
                let step = rows.saturating_sub(2).max(1);
                let d = self.doc_mut();
                let r = d.cursor_row();
                let top = d.top;
                let res = if f == 9 { d.goto_row(r.saturating_sub(step)) } else { d.goto_row(r + step) };
                d.top = if f == 9 { top.saturating_sub(step) } else { top + step };
                res
            }
            11 => {
                self.doc_mut().line_start();
                Ok(())
            }
            12 => {
                self.doc_mut().line_end();
                Ok(())
            }
            17 => self.doc_mut().text_top(),
            18 => self.doc_mut().text_bottom(),
            19 => self.doc_mut().split_line(),
            20 => self.doc_mut().backspace(),
            21 => self.doc_mut().delete(),
            22 => self.doc_mut().clear_line(),
            23 => self.doc_mut().delete_line(),
            24 => self.doc_mut().tab(tabs),
            25 => self.doc_mut().untab(tabs),
            26 => {
                let t = self.cfg.tabs as i32;
                self.dialog(m, dialogs::label::SET_TAB, &[(2, DVal::Int(t))], Then::SetTab);
                Ok(())
            }
            // Help (Help key, F5 and the Help menu): the Help accessory.
            27 | 152..=167 | 183 => {
                match self.cfg.accessory(if f == 183 { 27 } else { f }) {
                    Some((name, cmd)) => self.run_accessory(m, &name, cmd),
                    None => self.alert_message(13),
                }
                Ok(())
            }
            28 => {
                self.enter_direct(m);
                Ok(())
            }
            29 => self.doc_mut().insert_line(),
            30 => self.doc_mut().delete_to_end(),
            31 => self.doc_mut().goto_label(false),
            32 => self.doc_mut().goto_label(true),
            33 => {
                if !self.saved_check(m, 33) {
                    self.file_selector(m, 70, Then::Load);
                }
                Ok(())
            }
            34 => {
                self.file_selector(m, 74, Then::SaveAs(None));
                Ok(())
            }
            35 => {
                if self.doc().name.is_empty() {
                    self.file_selector(m, 74, Then::SaveAs(None));
                } else {
                    let name = self.doc().name.clone();
                    self.alert(format!("{}{name}", self.cfg.message(154)));
                    if let Err(e) = self.save(m) {
                        self.alert(e);
                    } else {
                        self.alert = None;
                    }
                }
                Ok(())
            }
            36 => self.doc_mut().delete_word_right(),
            37 => self.doc_mut().delete_word_left(),
            39..=48 => {
                self.doc_mut().set_mark((f - 39) as usize);
                Err(self.message_alert(64))
            }
            49..=58 => self.doc_mut().goto_mark((f - 49) as usize),
            59 => self.doc_mut().block_toggle(),
            60 => {
                self.doc_mut().block_forget();
                Ok(())
            }
            61 => {
                self.file_selector(m, 70, Then::LoadNew);
                Ok(())
            }
            62 => self.doc_mut().block_cut().map(|l| self.clipboard = l),
            63 => {
                let c = self.clipboard.clone();
                self.doc_mut().paste(&c)
            }
            64 => self.doc_mut().delete_to_start(),
            65 => self.doc_mut().undo(),
            66 => {
                self.search_dialog(m);
                Ok(())
            }
            67 | 68 => {
                if self.search.is_empty() {
                    self.search_dialog(m);
                    Ok(())
                } else {
                    self.do_search(f == 68)
                }
            }
            70 => {
                let a = self.last_alert.clone();
                self.alert(a);
                Ok(())
            }
            72 => match self.doc_mut().block_copy() {
                Ok(l) => {
                    self.clipboard = l;
                    Err(self.message_alert(7))
                }
                Err(e) => Err(e),
            },
            75 => {
                self.insert = !self.insert;
                Ok(())
            }
            76 => {
                self.dialog(m, dialogs::label::GOTO_LINE, &[], Then::GotoLine);
                Ok(())
            }
            77 => {
                self.run(m);
                Ok(())
            }
            78 => {
                self.test();
                Ok(())
            }
            79 => {
                self.test();
                let tabs = self.cfg.tabs as usize;
                self.doc_mut().indent(tabs)
            }
            80 => {
                if !self.saved_check(m, 80) {
                    self.new_program();
                }
                Ok(())
            }
            81 => {
                if !self.saved_check(m, 81) {
                    self.close_window();
                }
                Ok(())
            }
            82 => {
                // `Ed_Quit`: confirmation if asked by the configuration.
                if self.cfg.quit_flags & 1 != 0 {
                    self.dialog(m, dialogs::label::QUIT, &[], Then::Quit);
                } else if !self.saved_check(m, 1082) {
                    self.quit_requested = true;
                }
                Ok(())
            }
            83 => {
                if let Err(e) = self.doc_mut().commit() {
                    return self.edit_error(e);
                }
                self.infos(m);
                Ok(())
            }
            84 => {
                self.file_selector(m, 78, Then::Merge);
                Ok(())
            }
            85 => {
                self.file_selector(m, 86, Then::MergeAscii);
                Ok(())
            }
            87 => self.doc_mut().toggle_fold(),
            89 => self.doc_mut().fold_all(false),
            90 => self.doc_mut().fold_all(true),
            91 | 92 => {
                self.switch_window(f == 92);
                Ok(())
            }
            94 => self.doc_mut().redo(),
            97 => {
                self.file_selector(m, 82, Then::SaveBlockAscii);
                Ok(())
            }
            98 => {
                self.file_selector(m, 90, Then::SaveBlock);
                Ok(())
            }
            99 => {
                self.replace_dialog(m);
                Ok(())
            }
            100 | 101 => {
                if self.search.is_empty() {
                    self.replace_dialog(m);
                    Ok(())
                } else {
                    self.do_replace(f == 101)
                }
            }
            103 => {
                self.open_window();
                Ok(())
            }
            145 => Err(self.message_alert(222)),
            149 => {
                self.about_extension(m, 0);
                Ok(())
            }
            150 => {
                self.about(m);
                Ok(())
            }
            114 => {
                let b = self.text_buffer as i32;
                self.dialog(m, dialogs::label::SET_BUF, &[(2, DVal::Int(b))], Then::SetBuffer);
                Ok(())
            }
            181 => self.doc_mut().block_all(),
            1080 => {
                self.new_program();
                Ok(())
            }
            1082 => {
                self.quit_requested = true;
                Ok(())
            }
            build::BUILD_FUNCTION => {
                self.build_application(m);
                Ok(())
            }
            menu::SYNTAX_FUNCTION => {
                self.cfg.highlight = !self.cfg.highlight;
                Ok(())
            }
            _ => Err(self.message_alert(13)),
        };
        if let Err(e) = r
            && e != EditError::Reported
        {
            self.edit_error(e);
        }
        self.ensure_visible();
    }

    /// Shows an editor message and returns a value for `function` that
    /// produces no other message.
    fn message_alert(&mut self, n: usize) -> EditError {
        self.alert_message(n);
        EditError::Reported
    }

    fn new_program(&mut self) {
        self.docs[self.current] = Doc::new();
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // Windows
    // ------------------------------------------------------------------

    /// `Ed_OpenWindow`: a new window for a new program.
    pub(crate) fn open_window(&mut self) {
        if self.docs.len() >= self.rows_total() / 3 {
            return self.alert_message(3);
        }
        let _ = self.doc_mut().commit();
        self.current += 1;
        self.docs.insert(self.current, Doc::new());
        self.layout();
    }

    /// `Ed_CloseWindowQuit`.
    pub(crate) fn close_window(&mut self) {
        if self.docs.len() == 1 {
            self.new_program();
            return;
        }
        self.docs.remove(self.current);
        self.current = self.current.min(self.docs.len() - 1);
        self.layout();
    }

    fn switch_window(&mut self, next: bool) {
        let _ = self.doc_mut().commit();
        let n = self.docs.len();
        self.current = if next { (self.current + 1) % n } else { (self.current + n - 1) % n };
        self.dirty = true;
    }

    /// Scrolls the current window so the cursor is visible (`Ed_Cent`).
    fn ensure_visible(&mut self) {
        let rows = self.heights[self.current].max(1);
        let tx = (self.cfg.sx as usize - 16) / 8;
        let d = &mut self.docs[self.current];
        let n = d.row_count();
        let r = d.cursor_row();
        if r < d.top {
            d.top = r;
        } else if r >= d.top + rows {
            d.top = r + 1 - rows;
        }
        d.top = d.top.min(n.saturating_sub(1));
        if d.x < d.left {
            d.left = d.x.saturating_sub(10);
        } else if d.x >= d.left + tx - 1 {
            d.left = d.x + 10 - tx;
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // Mouse
    // ------------------------------------------------------------------

    /// Mouse position on the editor screen.
    fn mouse_pos(&self, m: &Machine) -> (i32, i32) {
        match m.hw.screens.get(EC_EDIT) {
            Some(s) => (s.x_screen(m.hw.input.mouse_x), s.y_screen(m.hw.input.mouse_y)),
            None => (-1, -1),
        }
    }

    /// Top of window `w` on the screen.
    fn window_y(&self, w: usize) -> i32 {
        TITLE_SY + self.heights[..w].iter().map(|&h| ETAT_SY + BAS_SY + h as i32 * 8).sum::<i32>()
    }

    fn mouse(&mut self, m: &mut Machine) {
        let buttons = m.hw.input.mouse_buttons;
        let pressed = buttons & !self.mouse_prev;
        let released = self.mouse_prev & !buttons;
        self.mouse_prev = buttons;
        let _ = m.hw.input.take_clicks();
        let (mx, my) = self.mouse_pos(m);

        // Menu: right button.
        if self.menu.open {
            let Some(s) = m.hw.screens.get_mut(EC_EDIT) else { return };
            self.menu.track(&self.menu_root, mx, my);
            self.menu.draw(&self.menu_root, s);
            if released & 2 != 0 || buttons & 2 == 0 {
                let f = self.menu.chosen(&self.menu_root);
                self.menu.close(s);
                self.dirty = true;
                if let Some(f) = f {
                    self.alert = None;
                    self.function(m, f);
                }
            }
            return;
        }
        if pressed & 2 != 0 {
            if let Some(s) = m.hw.screens.get_mut(EC_EDIT) {
                self.menu.open(s);
                self.menu.track(&self.menu_root, mx, my);
                self.menu.draw(&self.menu_root, s);
            }
            return;
        }

        // Wheel: scrolls the window under the cursor.
        let wheel = std::mem::take(&mut m.hw.input.wheel);
        if wheel.abs() >= 1.0 {
            let n = (wheel.abs() as usize) * 3;
            let rows = self.heights[self.current];
            let d = self.doc_mut();
            let max = d.row_count().saturating_sub(1);
            d.top = if wheel > 0.0 { d.top.saturating_sub(n) } else { (d.top + n).min(max) };
            let r = d.cursor_row();
            if r < d.top {
                let _ = d.goto_row(d.top);
            } else if r >= d.top + rows {
                let _ = d.goto_row(d.top + rows - 1);
            }
            self.dirty = true;
        }

        // Slider drag.
        if let Some(w) = self.slider_drag {
            if buttons & 1 == 0 {
                self.slider_drag = None;
            } else {
                self.slider_to(w, my);
            }
            return;
        }
        if pressed & 1 == 0 {
            return;
        }
        self.alert = None;
        self.dirty = true;
        let sx = self.cfg.sx as i32;
        if my < TITLE_SY {
            // Top bar buttons (`Ed_DrawTop`); string 13 gives their functions.
            let b = if mx < 32 {
                Some(0)
            } else if mx >= sx - 32 {
                Some(1)
            } else if (192..192 + 320).contains(&mx) {
                Some(2 + ((mx - 192) / 32) as usize)
            } else {
                None
            };
            if let Some(b) = b {
                let f = self.cfg.sys(13).get(b).copied().unwrap_or(0);
                if f == 105 {
                    return;
                }
                if f != 0 {
                    let _ = self.doc_mut().commit();
                    self.function(m, f as u16);
                }
            }
            return;
        }
        for w in 0..self.docs.len() {
            let wy = self.window_y(w);
            let h = self.heights[w] as i32 * 8;
            if my < wy || my >= wy + ETAT_SY + h + BAS_SY {
                continue;
            }
            if w != self.current {
                let _ = self.doc_mut().commit();
                self.current = w;
            }
            let ty = wy + ETAT_SY;
            if my >= ty && my < ty + h {
                if mx >= sx - 16 {
                    self.slider_drag = Some(w);
                    self.slider_to(w, my);
                    return;
                }
                let row = ((my - ty) / 8) as usize;
                let col = (mx / 8).max(0) as usize;
                let d = &mut self.docs[w];
                let r = d.top + row;
                if let Err(e) = d.goto_row(r) {
                    self.edit_error(e);
                }
                let d = &mut self.docs[w];
                d.x = d.left + col;
            } else if my < ty && mx < 24 {
                self.close_window();
            }
            break;
        }
        self.ensure_visible();
    }

    /// Moves the view of window `w` from a click / drag on its slider.
    fn slider_to(&mut self, w: usize, my: i32) {
        let wy = self.window_y(w) + ETAT_SY;
        let h = (self.heights[w] as i32 * 8).max(1);
        let rows = self.heights[w];
        let d = &mut self.docs[w];
        let n = d.row_count();
        let max_top = n.saturating_sub(rows);
        let t = ((my - wy).clamp(0, h) as usize * n) / h as usize;
        d.top = t.saturating_sub(rows / 2).min(max_top);
        let r = d.cursor_row();
        if r < d.top || r >= d.top + rows {
            let _ = d.goto_row(d.top);
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // Display
    // ------------------------------------------------------------------

    /// Redraws the editor screen: top bar (`Ed_DrawTop`) and windows
    /// (`Ed_DrawWindows`).
    pub fn redraw(&mut self, m: &mut Machine) {
        let Some(s) = m.hw.screens.get_mut(EC_EDIT) else { return };
        let sx = s.width as i32;
        draw::fill(s, 0, 0, sx, s.height as i32, 0);
        self.draw_top(s);
        for w in 0..self.docs.len() {
            self.draw_window(s, w);
        }
    }

    fn draw_top(&self, s: &mut Screen) {
        let sx = s.width as i32;
        let r = &self.res;
        r.draw(s, ED_PICS, 32, 0);
        // Buttons: 1 Direct at the left, 2 WB at the right, 3-12 after
        // the logo; button 10 shows the insert mode.
        for b in 1..=12usize {
            let x = match b {
                1 => 0,
                2 => sx - 32,
                _ => 192 + (b as i32 - 3) * 32,
            };
            let pos = if b == 10 && !self.insert { 1 } else { 0 };
            r.draw(s, ED_BOUTONS_PICS + (b - 1) * 2 + pos, x, 0);
        }
        // Memory sliders (`Ed_MemoryDraw`).
        let mx = 192 + 320;
        let width = sx - 32 - mx;
        r.draw(s, ED_MEMORY_PICS, mx, 0);
        let mut x = mx + 32;
        for _ in 0..(width / 32 - 2).max(1) {
            r.draw(s, ED_MEMORY_PICS + 1, x, 0);
            x += 32;
        }
        r.draw(s, ED_MEMORY_PICS + 2, x.min(sx - 64), 0);
        let msx = width - 32 - 7;
        let used: i64 = self.docs.iter().map(|d| d.to_program().source.len() as i64).sum();
        let chip = (used * msx as i64 / (512 * 1024)).clamp(0, msx as i64) as i32;
        let fast = ((used + 200 * 1024) * msx as i64 / (2048 * 1024)).clamp(0, msx as i64) as i32;
        draw::fill(s, mx + 32, 3, mx + 32 + chip, 5, 4);
        draw::fill(s, mx + 32, 10, mx + 32 + fast, 12, 4);
    }

    fn draw_window(&self, s: &mut Screen, w: usize) {
        let sx = s.width as i32;
        let d = &self.docs[w];
        let y0 = self.window_y(w);
        let ty = self.heights[w] as i32;
        let r = &self.res;
        // Frame: top (status line), right strip per row, bottom.
        r.draw(s, ED_PICS + 1, 0, y0);
        draw::copy(s, 160, y0, 320, ETAT_SY, sx - 320, y0);
        let ytext = y0 + ETAT_SY;
        for row in 0..ty {
            r.draw(s, ED_PICS + 2, sx - 16, ytext + row * 8);
        }
        let ybas = ytext + ty * 8;
        r.draw(s, ED_PICS + 3, 0, ybas);
        draw::copy(s, 160, ybas, 320, BAS_SY, sx - 320, ybas);
        // Status line buttons: close, hide, size.
        r.draw(s, ED_BT_PICS, 0, y0);
        r.draw(s, ED_BT_PICS + 2, sx - 48, y0);
        r.draw(s, ED_BT_PICS + 4, sx - 24, y0);
        self.draw_status(s, w, y0);

        // Text.
        let tx = ((sx - 16) / 8) as usize;
        draw::fill(s, 0, ytext, sx - 16, ybas, col::TEXT_PAPER);
        let rows = d.rows();
        let block = d.block_range();
        for row in 0..ty as usize {
            let Some(&li) = rows.get(d.top + row) else { break };
            let folded = d.lines.get(li).is_some_and(|l| text::is_folded(l));
            let y = ytext + row as i32 * 8;
            let in_block = block.is_some_and(|(a, b)| li >= a && li <= b && li < d.lines.len());
            let (pen, paper) =
                if in_block { (col::BLOCK_PEN, col::BLOCK_PAPER) } else { (col::TEXT_PEN, col::TEXT_PAPER) };
            if in_block {
                draw::fill(s, 0, y, sx - 16, y + 8, paper);
            }
            if self.cfg.highlight && !in_block && !folded {
                let (t, classes) = if li == d.y && d.is_edited() {
                    let t = d.current_text();
                    let c = highlight::lex_classes(&t);
                    (t, c)
                } else if let Some(l) = d.lines.get(li) {
                    highlight::detok_classes(l)
                } else {
                    (Vec::new(), Vec::new())
                };
                for (i, (&ch, &class)) in t.iter().zip(classes.iter()).enumerate().skip(d.left).take(tx) {
                    let x = (i - d.left) as i32 * 8;
                    draw::text(s, x, y, &[ch], highlight_pen(class), paper);
                }
            } else {
                let t = d.line_text(li);
                let visible: Vec<u8> = t.into_iter().skip(d.left).take(tx).collect();
                draw::text(s, 0, y, &visible, if folded { 5 } else { pen }, paper);
            }
        }
        // Cursor (current window only), blinking.
        if w == self.current && (self.blink / 16).is_multiple_of(2) {
            let r = d.cursor_row() as i32 - d.top as i32;
            let c = d.x as i32 - d.left as i32;
            if (0..ty).contains(&r) && (0..tx as i32).contains(&c) {
                let (x, y) = (c * 8, ytext + r * 8);
                let ch = d.current_text().get(d.x).copied().unwrap_or(b' ');
                let in_block = block.is_some_and(|(a, b)| d.y >= a && d.y <= b && d.y < d.lines.len());
                let (pen, paper) =
                    if in_block { (col::TEXT_PEN, col::TEXT_PAPER) } else { (col::TEXT_PAPER, col::TEXT_PEN) };
                draw::text(s, x, y, &[ch], pen, paper);
            }
        }
        // Vertical slider (`Edt_SlV`): x = window width + 6, 4 wide.
        let n = rows.len().max(1) as i32;
        let sl_x = sx - 16 + 6;
        let sl_y = ytext + 1;
        let sl_h = (ty * 8 - 2).max(1);
        draw::fill(s, sl_x, sl_y, sl_x + 4, sl_y + sl_h, 0);
        let k1 = sl_y + d.top as i32 * sl_h / n;
        let k2 = (sl_y + (d.top as i32 + ty) * sl_h / n).min(sl_y + sl_h).max(k1 + 2);
        draw::fill(s, sl_x, k1, sl_x + 4, k2, 3);
    }

    /// The status line (`Ed_EtPrint`): template string 2 with the window
    /// number, insert flag, line, column, free space and program name at
    /// the positions given by string 1.
    fn draw_status(&self, s: &mut Screen, w: usize, y0: i32) {
        let sx = s.width as i32;
        let d = &self.docs[w];
        let x0 = 32;
        let width = ((sx - 32 - 64) / 8) as usize;
        let y = y0 + 1;
        let cur = w == self.current;
        if cur && let Some(a) = &self.alert {
            let t = bytes(a);
            draw::fill(s, x0, y, x0 + width as i32 * 8, y + 8, col::ALERT_PAPER);
            let cx = x0 + (width.saturating_sub(t.len()) / 2) as i32 * 8;
            draw::text(s, cx, y, &t[..t.len().min(width)], col::ALERT_PEN, col::ALERT_PAPER);
            return;
        }
        let (pen, paper) = (col::STATUS_PEN, col::STATUS_PAPER);
        draw::text_n(s, x0, y, self.cfg.sys(2), width, pen, paper);
        let pos: Vec<usize> =
            (b'1'..=b'7').map(|c| self.cfg.sys(1).iter().position(|&x| x == c).unwrap_or(0)).collect();
        let at = |s: &mut Screen, i: usize, t: &[u8]| {
            draw::text(s, x0 + pos[i] as i32 * 8, y, t, pen, paper);
        };
        at(s, 0, &draw::number(w as i64 + 1, 2));
        at(s, 1, self.cfg.sys(if self.insert { 6 } else { 5 }));
        at(s, 2, &draw::number(d.cursor_row() as i64 + 1, 5));
        at(s, 3, &draw::number(d.x as i64 + 1, 3));
        let free = TEXT_BUFFER - d.to_program().source.len() as i64;
        at(s, 4, &draw::number(free, 7));
        at(s, 5, self.cfg.sys(3));
        let mut name = if d.name.is_empty() {
            self.cfg.sys(7).to_vec()
        } else {
            bytes(d.name.rsplit([':', '/']).next().unwrap_or(&d.name))
        };
        if d.modified {
            name.push(b'*');
        }
        let n = width.saturating_sub(pos[6] + 2);
        draw::text_n(s, x0 + pos[6] as i32 * 8, y, &name, n, pen, paper);
    }
}

/// Colours 8-11 of the editor screen: keywords, strings, comments and
/// numbers (syntax highlighting, not in the original).
pub const HIGHLIGHT_PALETTE: [u16; 4] = [0xFF6, 0xFB8, 0x6CF, 0x9F9];

/// Pen of a syntax class on the editor screen.
fn highlight_pen(c: highlight::Class) -> u8 {
    match c {
        highlight::Class::Normal => col::TEXT_PEN,
        highlight::Class::Keyword => 8,
        highlight::Class::String => 9,
        highlight::Class::Comment => 10,
        highlight::Class::Number => 11,
    }
}

/// Reads a program file: tokenised `.AMOS`, or an ASCII listing. The error
/// is `None` for "Not an AMOS program".
pub fn load_program_data(data: &[u8]) -> Result<Program, Option<String>> {
    if data.starts_with(b"AMOS") {
        return Program::load(data).map_err(|_| None);
    }
    if data.iter().take(4096).any(|&c| c == 0) {
        return Err(None);
    }
    crate::tokenise::tokenise_program(data).map_err(|(l, _)| Some(format!("Line too long. (line {l})")))
}

#[cfg(test)]
mod tests;
