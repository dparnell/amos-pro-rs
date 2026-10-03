//! Text windows, text output and zones (`+Lib.s:13031-14160`, 10895-11060).
//!
//! Most text instructions send a control string to the current window
//! (`GoWn`: `WiCall Print`), so they share the escape sequence code of
//! `gfx/window.rs`.

use super::Hardware;
use crate::gfx::Screen;
use crate::gfx::screen::lib_error;
use crate::interp::value::{ENT_NUL, Value, astr};
use crate::interp::{Exc, Interp, R, err};
use crate::tokens::{Keyword, TokenKind, tk};

/// "Screen not opened".
const SCREEN_NOT_OPENED: u16 = 47;
const FONCALL: u16 = 23;
/// `WFonCall`: illegal text window parameter.
const WFONCALL: u16 = 60;

fn wi(r: Result<(), u16>) -> R<()> {
    r.map_err(|e| Exc::Error(lib_error(e)))
}

fn opt(v: i32) -> Option<i32> {
    if v == ENT_NUL { None } else { Some(v) }
}

fn str_val(v: Vec<u8>) -> Value {
    Value::Str(astr(&v))
}

impl Hardware {
    /// The current screen (`ScOn` test): error 47 if none.
    pub(crate) fn text_screen(&mut self) -> R<&mut Screen> {
        match self.screens.current_mut() {
            Some(s) if s.has_window() => Ok(s),
            _ => err(SCREEN_NOT_OPENED),
        }
    }

    /// Sends a control string to the current window (`GoWn`).
    fn go_wn(&mut self, s: &[u8]) -> R<()> {
        let s = s.to_vec();
        let scr = self.text_screen()?;
        wi(scr.print_text(&s))
    }

    pub(crate) fn text_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        match kw.token {
            WIND_OPEN | WIND_OPEN_2 | WIND_OPEN_3 => {
                let a = it.inst_args(self, kw)?;
                let border = if a.len() > 5 { a.int(5) } else { 0 };
                let font = if a.len() > 6 { a.int(6) } else { 0 };
                let s = self.text_screen()?;
                let n = a.int(0);
                if n as u32 >= 65536 {
                    return err(WFONCALL);
                }
                if font != 0 {
                    return err(lib_error(14));
                }
                wi(s.wind_open(n, a.int(1), a.int(2), a.int(3), a.int(4), border, true))?;
            }
            WIND_SAVE => self.text_screen()?.text.wind_save = true,
            WIND_MOVE | WIND_SIZE => {
                let a = it.inst_args(self, kw)?;
                self.text_screen()?;
                let (x, y) = (a.int(0), a.int(1));
                if x < 0 || y < 0 {
                    return err(FONCALL);
                }
                let s = self.text_screen()?;
                if kw.token == WIND_MOVE {
                    wi(s.wind_move(Some(x), Some(y)))?;
                } else {
                    wi(s.wind_size(Some(x), Some(y)))?;
                }
            }
            WIND_CLOSE => wi(self.text_screen()?.wind_close())?,
            WINDOW => {
                let n = it.inst_args(self, kw)?.int(0);
                wi(self.text_screen()?.window_activate(n))?;
            }
            WRITING | WRITING_2 => {
                let a = it.inst_args(self, kw)?;
                let mode = a.int(0);
                let sel = if a.len() > 1 { a.int(1) } else { 0 };
                if sel as u32 >= 3 || mode as u32 >= 5 {
                    return err(WFONCALL);
                }
                self.go_wn(&[27, b'W', (48 + mode + sel * 8) as u8])?;
            }
            BORDER => {
                let a = it.inst_args(self, kw)?;
                wi(self
                    .text_screen()?
                    .set_border(opt(a.int(0)), opt(a.int(1)), opt(a.int(2))))?;
            }
            TITLE_TOP | TITLE_BOTTOM => {
                let t = it.inst_args(self, kw)?.str(0);
                let s = self.text_screen()?;
                if kw.token == TITLE_TOP {
                    wi(s.set_title(Some(&t), None))?;
                } else {
                    wi(s.set_title(None, Some(&t)))?;
                }
            }
            SET_CURS => {
                let a = it.inst_args(self, kw)?;
                let mut shape = [0u8; 8];
                for (i, b) in shape.iter_mut().enumerate() {
                    *b = a.int(i) as u8;
                }
                self.text_screen()?.set_cursor_shape(shape);
            }
            LOCATE => {
                let a = it.inst_args(self, kw)?;
                wi(self.text_screen()?.locate(opt(a.int(0)), opt(a.int(1))))?;
            }
            CENTRE => {
                let t = it.inst_args(self, kw)?.str(0);
                wi(self.text_screen()?.centre(&t))?;
            }
            CMOVE => {
                let a = it.inst_args(self, kw)?;
                let dx = opt(a.int(0)).unwrap_or(0) + 128;
                let dy = opt(a.int(1)).unwrap_or(0) + 128;
                if dx as u32 > 255 || dy as u32 > 255 {
                    return err(WFONCALL);
                }
                self.go_wn(&[27, b'N', dx as u8, 27, b'O', dy as u8])?;
            }
            CURS_PEN | PAPER | PEN => {
                let n = it.inst_args(self, kw)?.int(0);
                let l = match kw.token {
                    CURS_PEN => b'D',
                    PAPER => b'B',
                    _ => b'P',
                };
                self.go_wn(&[27, l, (n as u8).wrapping_add(48)])?;
            }
            CLW => self.go_wn(&[25])?,
            HOME => self.go_wn(&[12])?,
            CLEFT => self.go_wn(&[29])?,
            CRIGHT => self.go_wn(&[28])?,
            CUP => self.go_wn(&[30])?,
            CDOWN => self.go_wn(&[31])?,
            CURS_OFF => self.go_wn(b"\x1bC0")?,
            CURS_ON => self.go_wn(b"\x1bC1")?,
            INVERSE_ON => self.go_wn(b"\x1bI1")?,
            INVERSE_OFF => self.go_wn(b"\x1bI0")?,
            UNDER_ON => self.go_wn(b"\x1bU1")?,
            UNDER_OFF => self.go_wn(b"\x1bU0")?,
            SCROLL_ON => self.go_wn(b"\x1bV1")?,
            SCROLL_OFF => self.go_wn(b"\x1bV0")?,
            SHADE_ON => self.go_wn(b"\x1bS1")?,
            SHADE_OFF => self.go_wn(b"\x1bS0")?,
            CLINE => self.go_wn(&[26])?,
            CLINE_2 => {
                let n = it.inst_args(self, kw)?.int(0);
                if n == 0 || n as u32 >= 255 - 48 {
                    return err(WFONCALL);
                }
                self.go_wn(&[27, b'Q', (48 + n) as u8])?;
            }
            MEMORIZE_X => self.go_wn(b"\x1bM0")?,
            MEMORIZE_Y => self.go_wn(b"\x1bM2")?,
            REMEMBER_X => self.go_wn(b"\x1bM1")?,
            REMEMBER_Y => self.go_wn(b"\x1bM3")?,
            HSCROLL | VSCROLL => {
                let n = it.inst_args(self, kw)?.int(0);
                if !(1..=4).contains(&n) {
                    return err(WFONCALL);
                }
                let base = if kw.token == HSCROLL { 15 } else { 19 };
                self.go_wn(&[(base + n) as u8])?;
            }
            SET_TAB => {
                let n = it.inst_args(self, kw)?.int(0);
                if n as u32 > 255 - 48 {
                    return err(WFONCALL);
                }
                self.go_wn(&[27, b'T', (48 + n) as u8])?;
            }
            // Zones
            RESERVE_ZONE | RESERVE_ZONE_2 => {
                let a = it.inst_args(self, kw)?;
                let n = if kw.token == RESERVE_ZONE {
                    0
                } else {
                    a.int(0)
                };
                if n < 0 {
                    return err(FONCALL);
                }
                let s = self.text_screen()?;
                s.zones = vec![[0; 4]; (n as u32 & 0xFFFF) as usize];
            }
            RESET_ZONE | RESET_ZONE_2 => {
                let a = it.inst_args(self, kw)?;
                let n = if kw.token == RESET_ZONE { 0 } else { a.int(0) };
                if n < 0 {
                    return err(FONCALL);
                }
                let s = self.text_screen()?;
                if s.zones.is_empty() {
                    return err(73);
                }
                let n = n as u16 as usize;
                if n == 0 {
                    s.zones.iter_mut().for_each(|z| *z = [0; 4]);
                } else if n > s.zones.len() {
                    return err(73);
                } else {
                    s.zones[n - 1] = [0; 4];
                }
            }
            SET_ZONE => {
                let a = it.inst_args(self, kw)?;
                if a.int(0) <= 0 {
                    return err(FONCALL);
                }
                let s = self.text_screen()?;
                if s.set_zone(a.int(0), a.int(1), a.int(2), a.int(3), a.int(4))
                    .is_err()
                {
                    return err(FONCALL);
                }
            }
            _ => return Ok(false),
        }
        let _ = TokenKind::Instruction;
        Ok(true)
    }

    pub(crate) fn text_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            WINDON => Value::Int(self.text_screen()?.windon()),
            X_CURS => Value::Int(self.text_screen()?.cursor_pos().0),
            Y_CURS => Value::Int(self.text_screen()?.cursor_pos().1),
            X_GRAPHIC | Y_GRAPHIC => {
                let n = it.func_args(self, kw)?.int(0);
                let s = self.text_screen()?;
                if n < 0 {
                    return err(FONCALL);
                }
                Value::Int(if kw.token == X_GRAPHIC {
                    s.x_graphic(n)
                } else {
                    s.y_graphic(n)
                })
            }
            X_TEXT | Y_TEXT => {
                let n = it.func_args(self, kw)?.int(0);
                let s = self.text_screen()?;
                let r = if kw.token == X_TEXT {
                    s.x_text(n)
                } else {
                    s.y_text(n)
                };
                Value::Int(r.unwrap_or(ENT_NUL))
            }
            TAB_S => str_val(vec![9]),
            CLEFT_S => str_val(vec![29]),
            CRIGHT_S => str_val(vec![28]),
            CUP_S => str_val(vec![30]),
            CDOWN_S => str_val(vec![31]),
            PEN_S | PAPER_S => {
                let n = it.func_args(self, kw)?.int(0);
                if n as u32 >= 32 {
                    return err(WFONCALL);
                }
                let l = if kw.token == PEN_S { b'P' } else { b'B' };
                str_val(vec![27, l, 48 + n as u8])
            }
            AT => {
                let a = it.func_args(self, kw)?;
                let mut out = Vec::new();
                let (x, y) = (a.int(0), a.int(1));
                for v in [x, y] {
                    if v != ENT_NUL && v as u32 > 255 - 48 {
                        return err(WFONCALL);
                    }
                }
                if x != ENT_NUL {
                    out.extend_from_slice(&[27, b'X', (48 + x) as u8]);
                }
                if y != ENT_NUL {
                    out.extend_from_slice(&[27, b'Y', (48 + y) as u8]);
                }
                str_val(out)
            }
            CMOVE_S => {
                let a = it.func_args(self, kw)?;
                let mut out = Vec::new();
                let (dx, dy) = (a.int(0), a.int(1));
                for v in [dx, dy] {
                    if v != 0 && v != ENT_NUL && (v >= 128 || v <= -128) {
                        return err(WFONCALL);
                    }
                }
                if dx != 0 && dx != ENT_NUL {
                    out.extend_from_slice(&[27, b'N', (dx + 128) as u8]);
                }
                if dy != 0 && dy != ENT_NUL {
                    out.extend_from_slice(&[27, b'O', (dy + 128) as u8]);
                }
                str_val(out)
            }
            BORDER_S | ZONE_S => {
                let a = it.func_args(self, kw)?;
                let s = a.str(0);
                let n = a.int(1);
                let max = if kw.token == BORDER_S { 16 } else { 255 - 48 };
                if n == 0 || n as u32 >= max {
                    return err(WFONCALL);
                }
                let l = if kw.token == BORDER_S { b'E' } else { b'Z' };
                let mut out = vec![27, l, b'0'];
                out.extend_from_slice(&s);
                out.extend_from_slice(&[27, l, (48 + n) as u8]);
                str_val(out)
            }
            ZONE | ZONE_2 | HZONE | HZONE_2 => {
                let a = it.func_args(self, kw)?;
                let three = matches!(kw.token, ZONE_2 | HZONE_2);
                let (s, x, y) = if three {
                    (a.int(0), a.int(1), a.int(2))
                } else {
                    (-1, a.int(0), a.int(1))
                };
                self.text_screen()?;
                let Some(n) = self.screen_param(s.wrapping_add(1))? else {
                    return Ok(Some(Value::Int(ENT_NUL)));
                };
                let scr = self.screens.get(n).ok_or(Exc::Error(SCREEN_NOT_OPENED))?;
                let z = if n >= 8 {
                    0
                } else if matches!(kw.token, ZONE | ZONE_2) {
                    scr.zone_at(x, y)
                } else {
                    scr.zone_at_hard(x, y)
                };
                Value::Int(z)
            }
            MOUSE_ZONE => {
                let (mx, my) = (self.input.mouse_x, self.input.mouse_y);
                let s = self.text_screen()?;
                Value::Int(if s.number < 8 {
                    s.zone_at_hard(mx, my)
                } else {
                    0
                })
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// Screen parameter of the coordinate functions (`EcToD1`): 0 = the
    /// current screen, n > 0 = screen n-1, negative = none (the function
    /// returns `EntNul`).
    pub(crate) fn screen_param(&self, d3: i32) -> R<Option<usize>> {
        if (d3 as i16) < 0 {
            return Ok(None);
        }
        if d3 as i16 == 0 {
            return self
                .screens
                .current
                .map(Some)
                .ok_or(Exc::Error(SCREEN_NOT_OPENED));
        }
        let n = (d3 as i16 - 1) as usize;
        if self.screens.get(n).is_none() {
            return err(SCREEN_NOT_OPENED);
        }
        Ok(Some(n))
    }

    /// Prints text in the current window of the current screen.
    pub(crate) fn text_print(&mut self, it: &mut Interp, text: &[u8]) -> R<()> {
        let _ = it;
        if self.log_print {
            // (ASCII text is already valid UTF-8: same string, built faster.)
            self.log.push(match text.is_ascii() {
                true => String::from_utf8(text.to_vec()).unwrap_or_default(),
                false => crate::detok::latin1_to_string(text),
            });
        }
        let s = self.text_screen()?;
        wi(s.print_text(text))
    }
}

#[cfg(test)]
mod tests {
    use crate::interp::{RunState, StopReasonOrError};
    use crate::machine::Machine;

    fn run(src: &str) -> Machine {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).expect("tokenise");
        let mut m = Machine::new();
        m.run_program(&prg).expect("verify");
        for _ in 0..3 {
            m.vbl();
        }
        m
    }

    fn error(src: &str) -> Option<u16> {
        match run(src).state {
            RunState::Stopped(info) => match info.reason {
                StopReasonOrError::Error(n) => Some(n),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn error_numbers() {
        assert_eq!(error("Screen Open 8,320,200,16,Lowres"), Some(50));
        assert_eq!(error("Screen Open 1,320,200,12,Lowres"), Some(49));
        assert_eq!(error("Screen Open 1,320,200,32,Hires"), Some(23));
        assert_eq!(error("Screen Open 1,8,200,16,Lowres"), Some(48));
        assert_eq!(error("Screen 3"), Some(47));
        assert_eq!(error("Wind Close"), Some(62));
        assert_eq!(error("Locate 40,0"), Some(60));
        assert_eq!(error("Window 5"), Some(54));
        assert_eq!(
            error("Wind Open 1,0,0,10,5 : Wind Open 1,0,0,10,5"),
            Some(55)
        );
        assert_eq!(error("Title Top \"x\""), Some(63));
        assert_eq!(error("Double Buffer : Double Buffer"), Some(69));
        assert_eq!(error("Flash 1,\"(fff,0)\""), Some(52));
        assert_eq!(error("Shift Up 1,5,2,1"), Some(53));
        assert_eq!(error("Reset Zone"), Some(73));
        assert_eq!(error("Pen 16"), Some(60));
        assert_eq!(error("Screen Close 0 : Print \"x\""), Some(47));
        assert_eq!(error("Print 1"), None);
    }

    #[test]
    fn functions() {
        let m = run("Screen Open 2,640,200,4,Hires : A=X Hard(100) : B=Y Hard(2,10) : C=X Screen(A) : D=Screen Width
Reserve Zone 2 : Set Zone 1,10,10 To 20,20 : E=Zone(15,15) : F=Zone(25,15) : G=Scin(200,100)
Locate 3,4 : H=X Curs : I=Y Curs : J=X Graphic(3) : K=Y Text(33) : L$=At(1,2) : M=Len(Pen$(3))
Print A;B;C;D;E;F;G;H;I;J;K;Len(L$);M");
        let out = m.hw.log.join("");
        assert!(out.contains(" 178 60 100 640 1 0 2 3 4 24 4 6 3"), "{out}");
    }

    #[test]
    fn wind_save_restores_background() {
        let mut m = run(
            "Wind Save : Curs Off : Paper 4 : Cls : Wind Open 1,64,64,10,5,1 : Curs Off : Paper 5 : Clw : Wind Close",
        );
        let f = m.frame();
        let _ = f;
        let s = m.hw.screens.current().unwrap();
        // The window area is back to paper 4 of window 0.
        assert_eq!(s.pixel(100, 80), Some(4));
    }
}
