//! Direct mode (`Ed_Escape`, `Esc_Loop`, +Edit.s:8877-9330).
//!
//! It uses screen 8 (`EcFonc`) as a strip at the bottom of the display,
//! over the screens of the program: the program's output stays visible.
//! The direct mode strip has the button bar of the resource bank (button
//! 1 back to the editor, the logo, the output button, the ten function
//! keys, memory sliders) and a few text lines; lines are typed after the
//! `AMOS Pro>` prompt (system string 14) and recalled with Up / Down
//! (`Esc_KMem`). F1-F10 (and the buttons) insert the commands of system
//! strings 24-43, a final backquote meaning Return.

use super::config::EdConfig;
use super::draw::{self, col};
use super::resource::*;
use super::{EC_EDIT, EC_FONC};
use crate::Machine;
use crate::gfx::Screen;
use crate::input::raw;

/// Number of text rows of the direct mode strip.
const ROWS: usize = 5;
const BAR_SY: i32 = 16;
/// Maximum length of a direct mode line (`L_LEd_Init` with 160).
const MAX_LINE: usize = 160;

pub enum Action {
    None,
    /// Esc: back to the editor.
    Editor,
    /// Return: run this line.
    Run(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct DirectMode {
    pub history: Vec<Vec<u8>>,
    hist_pos: usize,
    pub line: Vec<u8>,
    cursor: usize,
    /// Text area lines (oldest first).
    pub output: Vec<Vec<u8>>,
    prompt: Vec<u8>,
    max_history: usize,
    mouse_prev: u8,
}

/// Printable part of a string with AMOS window escape codes.
fn printable(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            27 => i += 3,
            c if c < 32 => i += 1,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

impl DirectMode {
    pub fn new(cfg: &EdConfig) -> DirectMode {
        DirectMode { prompt: printable(cfg.sys(14)), max_history: cfg.esc_kmem_max as usize, ..Default::default() }
    }

    fn open_screen(&self, m: &mut Machine, cfg: &EdConfig, height: u32) {
        let mut s = Screen::new(EC_FONC, cfg.sx as u32, height, 8, 0x8000);
        s.palette[..8].copy_from_slice(&cfg.palette);
        let y = cfg.wy as i32 + cfg.sy as i32 - height as i32;
        s.pending_display = [Some(cfg.wx as i32), Some(y), None, None];
        s.apply_pending();
        let cur = m.hw.screens.current;
        m.hw.screens.insert(s);
        m.hw.screens.current = cur.filter(|&c| c != EC_EDIT && c != EC_FONC).or(Some(0));
        if m.hw.screens.get(0).is_none() && m.hw.screens.current == Some(0) {
            m.hw.screens.current = None;
        }
        m.hw.sprites.mouse_show = m.hw.sprites.mouse_show.max(0);
    }

    /// `Esc_Appear`: opens the direct mode strip.
    pub fn open(&mut self, m: &mut Machine, cfg: &EdConfig, res: &Resource) {
        self.close(m);
        self.open_screen(m, cfg, (BAR_SY + ROWS as i32 * 8 + 8) as u32);
        m.hw.input.clear_keys();
        self.mouse_prev = m.hw.input.mouse_buttons;
        self.redraw(m, cfg, res);
    }

    /// Removes the strip.
    pub fn close(&mut self, m: &mut Machine) {
        if m.hw.screens.get(EC_FONC).is_some() {
            let cur = m.hw.screens.current;
            m.hw.screens.remove(EC_FONC);
            if cur != Some(EC_FONC) {
                m.hw.screens.current = cur;
            }
        }
    }

    fn print(&mut self, t: Vec<u8>) {
        self.output.push(t);
        let n = self.output.len();
        if n > ROWS {
            self.output.drain(..n - ROWS);
        }
    }

    /// Shows an error message in the text area.
    pub fn print_error(&mut self, m: &mut Machine, cfg: &EdConfig, res: &Resource, msg: &str) {
        self.print(super::bytes(msg));
        self.redraw(m, cfg, res);
    }

    /// Draws the strip: button bar, output lines and the input line.
    pub fn redraw(&mut self, m: &mut Machine, cfg: &EdConfig, res: &Resource) {
        let Some(s) = m.hw.screens.get_mut(EC_FONC) else { return };
        let sx = s.width as i32;
        let h = s.height as i32;
        draw::fill(s, 0, 0, sx, h, 0);
        // Button bar (`Es_Pics`, `Es_BoutonsPics`).
        res.draw(s, ES_BOUTONS_PICS, 0, 0);
        res.draw(s, ES_PICS, 32, 0);
        for b in 3..=13usize {
            res.draw(s, ES_BOUTONS_PICS + (b - 1) * 2, 160 + (b as i32 - 3) * 32, 0);
        }
        res.draw(s, ED_MEMORY_PICS, 512, 0);
        res.draw(s, ED_MEMORY_PICS + 1, 544, 0);
        res.draw(s, ED_MEMORY_PICS + 2, 576, 0);
        res.draw(s, ES_BOUTONS_PICS + 2, sx - 32, 0);
        // Text area.
        let tx = (sx / 8) as usize;
        draw::fill(s, 0, BAR_SY, sx, h, col::TEXT_PAPER);
        for (i, l) in self.output.iter().enumerate() {
            draw::text_n(s, 0, BAR_SY + i as i32 * 8, l, tx, col::TEXT_PEN, col::TEXT_PAPER);
        }
        let y = BAR_SY + self.output.len().min(ROWS) as i32 * 8;
        let px = draw::text(s, 0, y, &self.prompt, col::TEXT_PAPER, col::TEXT_PEN);
        let room = tx - self.prompt.len() - 1;
        let skip = self.cursor.saturating_sub(room);
        let shown: Vec<u8> = self.line.iter().copied().skip(skip).take(room).collect();
        draw::text(s, px, y, &shown, col::TEXT_PEN, col::TEXT_PAPER);
        let cx = px + (self.cursor - skip) as i32 * 8;
        draw::invert(s, cx, y, cx + 8, y + 8, col::TEXT_PAPER, col::TEXT_PEN);
        let _ = cfg;
    }

    /// `LEd_Loop` in direct mode: line editing and the direct mode keys.
    pub fn input(&mut self, m: &mut Machine, cfg: &EdConfig, res: &Resource) -> Action {
        let mut changed = false;
        let mut action = Action::None;
        while let Some(k) = m.hw.input.inkey() {
            changed = true;
            match k.raw {
                raw::ESC => return Action::Editor,
                raw::RETURN | raw::ENTER => {
                    action = self.enter();
                    break;
                }
                raw::UP | raw::DOWN => {
                    if self.history.is_empty() {
                        continue;
                    }
                    if k.raw == raw::UP {
                        self.hist_pos = self.hist_pos.saturating_sub(1);
                    } else {
                        self.hist_pos = (self.hist_pos + 1).min(self.history.len());
                    }
                    self.line = self.history.get(self.hist_pos).cloned().unwrap_or_default();
                    self.cursor = self.line.len();
                }
                raw::LEFT => self.cursor = self.cursor.saturating_sub(1),
                raw::RIGHT => self.cursor = (self.cursor + 1).min(self.line.len()),
                raw::BACKSPACE => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.line.remove(self.cursor);
                    }
                }
                raw::DEL => {
                    if self.cursor < self.line.len() {
                        self.line.remove(self.cursor);
                    }
                }
                0x50..=0x59 => {
                    let n = (k.raw - 0x50) as usize + if k.shift & 3 != 0 { 10 } else { 0 };
                    if let Some(a) = self.function_key(cfg, n) {
                        action = a;
                        break;
                    }
                }
                _ => {
                    if k.ascii == 13 {
                        action = self.enter();
                        break;
                    }
                    if k.ascii == 27 {
                        return Action::Editor;
                    }
                    if k.ascii >= 32 && self.line.len() < MAX_LINE {
                        self.line.insert(self.cursor, k.ascii);
                        self.cursor += 1;
                    }
                }
            }
        }
        // Buttons.
        let buttons = m.hw.input.mouse_buttons;
        let pressed = buttons & !self.mouse_prev;
        self.mouse_prev = buttons;
        let _ = m.hw.input.take_clicks();
        if pressed & 3 != 0
            && let Some(s) = m.hw.screens.get(EC_FONC)
        {
            let (x, y) = (s.x_screen(m.hw.input.mouse_x), s.y_screen(m.hw.input.mouse_y));
            if (0..BAR_SY).contains(&y) {
                if x < 32 {
                    return Action::Editor;
                }
                if (192..512).contains(&x) {
                    let n = ((x - 192) / 32) as usize + if pressed & 2 != 0 { 10 } else { 0 };
                    if let Some(a) = self.function_key(cfg, n) {
                        action = a;
                    }
                    changed = true;
                }
            }
        }
        if changed {
            self.redraw(m, cfg, res);
        }
        action
    }

    /// Inserts the command of function key `n` (0-19); a final backquote
    /// executes it.
    fn function_key(&mut self, cfg: &EdConfig, n: usize) -> Option<Action> {
        let s = cfg.sys(24 + n).to_vec();
        let (s, run) = match s.strip_suffix(b"`") {
            Some(t) => (t.to_vec(), true),
            None => (s, false),
        };
        for c in s {
            if self.line.len() < MAX_LINE {
                self.line.insert(self.cursor, c);
                self.cursor += 1;
            }
        }
        if run { Some(self.enter()) } else { None }
    }

    /// Return: the line goes to the history and the text area.
    fn enter(&mut self) -> Action {
        let line = std::mem::take(&mut self.line);
        self.cursor = 0;
        let mut shown = self.prompt.clone();
        shown.extend_from_slice(&line);
        self.print(shown);
        if !line.iter().all(|&c| c == b' ') && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > self.max_history {
                self.history.remove(0);
            }
        }
        self.hist_pos = self.history.len();
        Action::Run(line)
    }
}
