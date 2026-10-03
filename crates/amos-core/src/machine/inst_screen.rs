//! Screens: Screen Open/Close/Display/Offset/To Front..., palettes, colour
//! effects (`+Lib.s:8660-9520`, `+ILib.s:5360-5450`).

use super::Hardware;
use crate::gfx::Screen;
use crate::gfx::effects::rainbow_table;
use crate::gfx::screen::{self, E_NOT_OPENED, lib_error, logic_value, physic_value};
use crate::interp::value::{ENT_NUL, Value};
use crate::interp::{Exc, Interp, R, err};
use crate::tokens::{Keyword, TK_COMMA, TK_PAR1, TK_PAR2, TK_TO, TokenKind, tk};

const FONCALL: u16 = 23;
const OUT_OF_MEMORY: u16 = 24;
const SCREEN_NOT_OPENED: u16 = 47;

/// Flash of colour 3 done by Screen Open (message 46 of the interpreter
/// configuration, +Interpreter_Config.s:158).
pub const OPEN_FLASH: &[u8] =
    b"(000,2)(440,2)(880,2)(bb0,2)(dd0,2)(ee0,2)(ff2,2)(ff8,2)(ffc,2)(fff,2)(aaf,2)(88c,2)(66a,2)(226,2)(004,2)(001,2)";

fn ec(r: Result<(), u16>) -> R<()> {
    r.map_err(|e| Exc::Error(lib_error(e)))
}

/// `CheckScreenNumber`: 0-7, else "valid screen numbers range 0 to 7".
/// Coordinate conversion of `Hardware::screen_coord`.
#[derive(Clone, Copy)]
enum Coord {
    XHard,
    YHard,
    XScreen,
    YScreen,
}

fn check_screen(n: i32) -> R<usize> {
    if n as u32 >= 8 {
        err(lib_error(screen::E_SCREEN_NUMBER))
    } else {
        Ok(n as usize)
    }
}

impl Hardware {
    // Keywords as typed functions (the token path and compiled code call
    // these with the parameters read; `None` is an omitted parameter).

    /// Screen `n`, or the current screen for `None` (no parameter).
    fn screen_or_current(&self, n: Option<i32>) -> R<usize> {
        match n {
            None => self.cur_screen(),
            Some(n) => check_screen(n),
        }
    }

    /// `Screen n`.
    pub(crate) fn screen(&mut self, n: i32) -> R<()> {
        let n = check_screen(n)?;
        ec(self.screens.activate(n))
    }

    /// `Cls` (`c` None: the windows), `Cls c` (`rect` None: the whole
    /// screen), `Cls c,x1,y1 To x2,y2`. (`rect` is ignored without `c`.)
    pub(crate) fn cls(&mut self, c: Option<i32>, rect: Option<(i32, i32, i32, i32)>) -> R<()> {
        let Some(c) = c else {
            self.cur_screen()?;
            self.cur_mut()?.cls_windows();
            return Ok(());
        };
        let s = self.cur_mut()?;
        let (x1, y1, x2, y2) = rect.unwrap_or((0, 0, 10000, 10000));
        // Word coordinates as in EcCls.
        let w = |v: i32| v as i16 as i32;
        for b in s.autoback_targets() {
            s.cls_rect(b, c as u8, w(x1), w(y1), w(x2), w(y2));
        }
        Ok(())
    }

    /// `Screen To Front` / `Screen To Front n`.
    pub(crate) fn screen_to_front(&mut self, n: Option<i32>) -> R<()> {
        let n = self.screen_or_current(n)?;
        ec(self.screens.to_front(n))
    }

    /// `Colour(n)`.
    pub(crate) fn colour_fn(&mut self, n: i32) -> R<i32> {
        Ok(self.cur_mut()?.palette[(n & 31) as usize] as i32)
    }

    /// `X Hard` / `Y Hard` / `X Screen` / `Y Screen` of `v`, on `screen`
    /// (`None`: the current screen, the one-parameter forms).
    fn screen_coord(&mut self, axis: Coord, screen: Option<i32>, v: i32) -> R<i32> {
        let d3 = match screen {
            Some(s) => s.wrapping_add(1),
            None => {
                self.cur_screen()?;
                0
            }
        };
        let Some(n) = self.screen_param(d3)? else {
            return Ok(ENT_NUL);
        };
        let s = self
            .screens
            .get(n)
            .ok_or(Exc::Error(lib_error(E_NOT_OPENED)))?;
        // Word arithmetic as in the original.
        let v = v as i16 as i32;
        let r = match axis {
            Coord::XHard => s.x_hard(v),
            Coord::YHard => s.y_hard(v),
            Coord::XScreen => s.x_screen(v),
            Coord::YScreen => s.y_screen(v),
        };
        Ok(r as i16 as i32)
    }

    /// `X Screen(x)` / `X Screen(screen,x)`.
    pub(crate) fn x_screen(&mut self, screen: Option<i32>, v: i32) -> R<i32> {
        self.screen_coord(Coord::XScreen, screen, v)
    }

    /// `Y Screen(y)` / `Y Screen(screen,y)`.
    pub(crate) fn y_screen(&mut self, screen: Option<i32>, v: i32) -> R<i32> {
        self.screen_coord(Coord::YScreen, screen, v)
    }

    /// `X Hard(x)` / `X Hard(screen,x)`.
    pub(crate) fn x_hard(&mut self, screen: Option<i32>, v: i32) -> R<i32> {
        self.screen_coord(Coord::XHard, screen, v)
    }

    /// `Y Hard(y)` / `Y Hard(screen,y)`.
    pub(crate) fn y_hard(&mut self, screen: Option<i32>, v: i32) -> R<i32> {
        self.screen_coord(Coord::YHard, screen, v)
    }

    /// The current screen number (`ScOn`), error 47 if none.
    fn cur_screen(&self) -> R<usize> {
        self.screens.current.ok_or(Exc::Error(SCREEN_NOT_OPENED))
    }

    fn cur_mut(&mut self) -> R<&mut Screen> {
        self.screens
            .current_mut()
            .ok_or(Exc::Error(SCREEN_NOT_OPENED))
    }

    /// Reads a list of values separated by commas (`Plt`): negative values
    /// (and omitted ones) leave the entry unchanged.
    fn palette_list(&mut self, it: &mut Interp, out: &mut [i32; 32]) -> R<()> {
        let mut i = 0;
        loop {
            let v = it.eval(self)?;
            let v = match v {
                Value::Int(n) => n,
                Value::Float(f) => crate::interp::value::float_to_int(f),
                Value::Str(_) => return err(crate::errors::TYPE_MISMATCH),
            };
            if i < 32 && v >= 0 {
                out[i] = v & 0xFFF;
            }
            i += 1;
            if it.peek() == TK_COMMA {
                it.pc += 2;
            } else {
                return Ok(());
            }
        }
    }

    /// `PalRout`: palette of `src` masked by `mask` (bit n = colour n).
    fn masked_palette(src: &[u16; 32], mask: i32) -> [i32; 32] {
        let mut p = [-1; 32];
        for (i, c) in src.iter().enumerate() {
            if mask & (1 << i) != 0 {
                p[i] = *c as i32;
            }
        }
        p
    }

    fn set_palette(&mut self, p: &[i32; 32]) -> R<()> {
        let s = self.cur_mut()?;
        for (i, &c) in p.iter().enumerate() {
            if c >= 0 {
                s.palette[i] = (c & 0xFFF) as u16;
            }
        }
        Ok(())
    }

    pub(crate) fn screen_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        let reserved = kw
            .def()
            .is_some_and(|d| d.kind() == TokenKind::ReservedVariable);
        match kw.token {
            SCREEN_OPEN => {
                let a = it.inst_args(self, kw)?;
                let mut mode = a.int(4) as u32 & 0x8004;
                let colours = a.int(3);
                let planes = if colours == 4096 {
                    if mode & screen::MODE_HIRES != 0 {
                        return err(FONCALL);
                    }
                    mode |= 0x0800;
                    6
                } else {
                    match colours {
                        2 => 1,
                        4 => 2,
                        8 => 3,
                        16 => 4,
                        32 => 5,
                        64 => 6,
                        _ => return err(lib_error(screen::E_ILLEGAL_COLOURS)),
                    }
                };
                let n = check_screen(a.int(0))?;
                if mode & screen::MODE_HIRES != 0 && planes > 4 {
                    return err(FONCALL);
                }
                ec(self
                    .screens
                    .open(n, a.int(1), a.int(2), colours as u32, mode & 0x8004))?;
                if planes > 1 {
                    let _ = self.screens.effects.flash(n, 3, OPEN_FLASH);
                }
            }
            SCREEN_CLOSE => {
                let n = check_screen(it.inst_args(self, kw)?.int(0))?;
                if self.screens.get(n).is_none() {
                    return err(SCREEN_NOT_OPENED);
                }
                self.screens.remove(n);
            }
            SCREEN => {
                let n = it.inst_args(self, kw)?.int(0);
                self.screen(n)?;
            }
            SCREEN_DISPLAY => {
                let a = it.inst_args(self, kw)?;
                let n = check_screen(a.int(0))?;
                let s = self
                    .screens
                    .get_mut(n)
                    .ok_or(Exc::Error(SCREEN_NOT_OPENED))?;
                for i in 0..4 {
                    if let Some(v) = a.opt(i + 1) {
                        s.pending_display[i] = Some(v);
                    }
                }
            }
            SCREEN_OFFSET => {
                let a = it.inst_args(self, kw)?;
                self.cur_screen()?;
                let n = check_screen(a.int(0))?;
                let s = self
                    .screens
                    .get_mut(n)
                    .ok_or(Exc::Error(SCREEN_NOT_OPENED))?;
                if let Some(x) = a.opt(1) {
                    s.pending_offset[0] = Some(x);
                }
                if let Some(y) = a.opt(2) {
                    s.pending_offset[1] = Some(y);
                }
            }
            SCREEN_TO_FRONT | SCREEN_TO_FRONT_2 => {
                let a = it.inst_args(self, kw)?;
                self.screen_to_front(if a.is_empty() { None } else { Some(a.int(0)) })?;
            }
            SCREEN_TO_BACK | SCREEN_TO_BACK_2 | SCREEN_HIDE | SCREEN_HIDE_2 | SCREEN_SHOW
            | SCREEN_SHOW_2 => {
                let a = it.inst_args(self, kw)?;
                let n = self.screen_or_current(if a.is_empty() { None } else { Some(a.int(0)) })?;
                ec(match kw.token {
                    SCREEN_TO_BACK | SCREEN_TO_BACK_2 => self.screens.to_back(n),
                    SCREEN_HIDE | SCREEN_HIDE_2 => self.screens.set_hidden(n, true),
                    _ => self.screens.set_hidden(n, false),
                })?;
            }
            SCREEN_CLONE => {
                let a = it.inst_args(self, kw)?;
                self.cur_screen()?;
                let n = check_screen(a.int(0))?;
                ec(self.screens.clone_current(n))?;
            }
            DOUBLE_BUFFER => {
                self.cur_screen()?;
                ec(self.screens.double_buffer())?;
            }
            SCREEN_SWAP => {
                self.cur_screen()?;
                self.screens.swap_all();
            }
            SCREEN_SWAP_2 => {
                let n = check_screen(it.inst_args(self, kw)?.int(0))?;
                ec(self.screens.swap(n))?;
            }
            DUAL_PLAYFIELD | DUAL_PRIORITY => {
                let a = it.inst_args(self, kw)?;
                let s1 = check_screen(a.int(0))?;
                let s2 = check_screen(a.int(1))?;
                ec(if kw.token == DUAL_PLAYFIELD {
                    self.screens.set_dual(s1, s2)
                } else {
                    self.screens.dual_priority(s1, s2)
                })?;
            }
            AUTOBACK => {
                let n = it.inst_args(self, kw)?.int(0);
                if n as u32 >= 3 {
                    return err(FONCALL);
                }
                self.cur_mut()?.autoback = n as u8;
            }
            VIEW => {
                let auto = std::mem::replace(&mut self.screens.auto_view, true);
                self.screen_update();
                self.screens.auto_view = auto;
            }
            AUTO_VIEW_ON => self.screens.auto_view = true,
            AUTO_VIEW_OFF => self.screens.auto_view = false,
            DEFAULT => {
                self.screen_reset();
                self.sprites_reset();
            }
            CLS => self.cls(None, None)?,
            CLS_2 | CLS_3 => {
                let a = it.inst_args(self, kw)?;
                let rect = if kw.token == CLS_2 {
                    None
                } else {
                    Some((a.int(1), a.int(2), a.int(3), a.int(4)))
                };
                self.cls(Some(a.int(0)), rect)?;
            }
            // Palettes
            COLOUR => {
                let a = it.inst_args(self, kw)?;
                let s = self.cur_mut()?;
                s.palette[(a.int(0) & 31) as usize] = (a.int(1) & 0xFFF) as u16;
            }
            COLOUR_BACK => {
                let c = it.inst_args(self, kw)?.int(0);
                self.screens.colour_back = (c & 0xFFF) as u16;
            }
            PALETTE => {
                self.cur_screen()?;
                let mut p = [-1; 32];
                self.palette_list(it, &mut p)?;
                self.set_palette(&p)?;
            }
            DEFAULT_PALETTE => {
                let mut p = [-1; 32];
                self.palette_list(it, &mut p)?;
                for (i, &c) in p.iter().enumerate() {
                    if c >= 0 {
                        self.screens.default_palette[i] = c as u16;
                    }
                }
            }
            GET_PALETTE | GET_PALETTE_2 => {
                let a = it.inst_args(self, kw)?;
                let mask = if a.len() > 1 { a.int(1) } else { -1 };
                let (n, _) = self.screens.resolve_bitmap(a.int(0)).map_err(Exc::Error)?;
                let src = self
                    .screens
                    .get(n)
                    .ok_or(Exc::Error(SCREEN_NOT_OPENED))?
                    .palette;
                self.cur_screen()?;
                let p = Self::masked_palette(&src, mask);
                self.set_palette(&p)?;
            }
            FLASH_OFF => {
                let n = self.cur_screen()?;
                self.screens.effects.flash_off(n);
            }
            FLASH => {
                let a = it.inst_args(self, kw)?;
                let n = self.cur_screen()?;
                let colour = a.int(0) as u16 as usize;
                ec(self.screens.effects.flash(n, colour, &a.str(1)))?;
            }
            SHIFT_OFF => {
                let n = self.cur_screen()?;
                if self
                    .screens
                    .effects
                    .shift
                    .as_ref()
                    .is_some_and(|s| s.screen == n)
                {
                    self.screens.effects.shift = None;
                }
            }
            SHIFT_UP | SHIFT_DOWN => {
                let a = it.inst_args(self, kw)?;
                self.cur_screen()?;
                let c1 = if a.int(1) < 0 { 1 } else { a.int(1) };
                let rotate = a.int(3) as u8 != 0;
                ec(self.screens.start_shift(
                    a.int(0),
                    c1,
                    a.int(2),
                    kw.token == SHIFT_DOWN,
                    rotate,
                ))?;
            }
            FADE => self.fade(it)?,
            // Rainbows
            SET_RAINBOW | SET_RAINBOW_2 => {
                let a = it.inst_args(self, kw)?;
                let start = if kw.token == SET_RAINBOW_2 {
                    a.int(6)
                } else {
                    0
                };
                let (n, colour, lines) = (a.int(0), a.int(1), a.int(2));
                if !(16..32700).contains(&lines) || colour < 0 || n as u32 >= 4 {
                    return err(FONCALL);
                }
                let r = &mut self.screens.effects.rainbows[n as usize];
                *r = Default::default();
                let colour = (colour & 31) as usize;
                if colour >= 16 {
                    return err(FONCALL);
                }
                let Some(buf) = rainbow_table(
                    lines as usize,
                    start as u16,
                    &a.str(3),
                    &a.str(4),
                    &a.str(5),
                ) else {
                    return err(FONCALL);
                };
                *r = crate::gfx::effects::Rainbow {
                    colour,
                    buf,
                    base: 0,
                    y: 0,
                    height: -1,
                };
            }
            RAINBOW => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                let r = self
                    .screens
                    .effects
                    .rainbows
                    .get_mut(n as u32 as usize)
                    .filter(|r| !r.buf.is_empty())
                    .ok_or(Exc::Error(OUT_OF_MEMORY))?;
                if let Some(b) = a.opt(1)
                    && (0..r.buf.len() as i32).contains(&b)
                {
                    r.base = b;
                }
                if let Some(y) = a.opt(2) {
                    r.y = y;
                }
                if let Some(h) = a.opt(3) {
                    r.height = h;
                }
                // AMAL reads the same values (X, Y, A of a rainbow channel).
                let act = [r.base as i16, r.y as i16, r.height as i16];
                self.sprites.rainbow_act[n as usize] = act;
            }
            RAINBOW_DEL => self.screens.effects.rainbows = Default::default(),
            RAINBOW_DEL_2 => {
                let n = it.inst_args(self, kw)?.int(0);
                if n < 0 {
                    self.screens.effects.rainbows = Default::default();
                } else {
                    let r = self
                        .screens
                        .effects
                        .rainbows
                        .get_mut(n as usize)
                        .ok_or(Exc::Error(OUT_OF_MEMORY))?;
                    *r = Default::default();
                }
            }
            RAIN if reserved => {
                it.expect(TK_PAR1)?;
                let n = it.eval_int(self)?;
                it.expect(TK_COMMA)?;
                let y = it.eval_int(self)?;
                it.expect(TK_PAR2)?;
                it.expect(OP_EQ)?;
                let v = it.eval_int(self)?;
                *self.rain_var(n, y)? = (v & 0xFFF) as u16;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Rain(n,y)` entry (`TRVar`): error "out of memory" (sic) if invalid.
    fn rain_var(&mut self, n: i32, y: i32) -> R<&mut u16> {
        let r = self
            .screens
            .effects
            .rainbows
            .get_mut(n as u32 as usize)
            .ok_or(Exc::Error(OUT_OF_MEMORY))?;
        if y < 0 {
            return err(OUT_OF_MEMORY);
        }
        r.buf.get_mut(y as usize).ok_or(Exc::Error(OUT_OF_MEMORY))
    }

    /// `Fade n`, `Fade n,c1,c2...`, `Fade n To s[,mask]` (`InFade`,
    /// +ILib.s:5411).
    fn fade(&mut self, it: &mut Interp) -> R<()> {
        let speed = it.eval_int(self)?;
        let target: [i32; 32] = match it.peek() {
            TK_TO => {
                it.pc += 2;
                let s = it.eval_int(self)?;
                let src = if s < 0 {
                    match self.banks.get(1).map(|b| &b.data) {
                        Some(crate::banks::BankData::Images { palette, .. }) => *palette,
                        _ => return err(36),
                    }
                } else {
                    let (n, _) = self.screens.resolve_bitmap(s).map_err(Exc::Error)?;
                    self.screens
                        .get(n)
                        .ok_or(Exc::Error(SCREEN_NOT_OPENED))?
                        .palette
                };
                let mut mask = -1;
                if it.peek() == TK_COMMA {
                    it.pc += 2;
                    mask = it.eval_int(self)?;
                }
                Self::masked_palette(&src, mask)
            }
            TK_COMMA => {
                it.pc += 2;
                let mut p = [-1; 32];
                self.palette_list(it, &mut p)?;
                p
            }
            _ => [0; 32],
        };
        self.cur_screen()?;
        if speed <= 0 {
            return err(FONCALL);
        }
        self.screens.start_fade(speed as u16, &target);
        Ok(())
    }

    pub(crate) fn screen_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            SCREEN_2 => Value::Int(self.screens.current.map_or(-1, |n| n as i32)),
            HIRES => Value::Int(0x8000),
            LOWRES => Value::Int(0),
            LACED => Value::Int(4),
            NTSC => Value::Int(0),
            DISPLAY_HEIGHT => Value::Int(311),
            SCREEN_MODE => Value::Int(self.cur_mut()?.mode() as i32),
            COLOUR_2 => {
                let n = it.func_args(self, kw)?.int(0);
                Value::Int(self.colour_fn(n)?)
            }
            SCREEN_WIDTH | SCREEN_HEIGHT | SCREEN_WIDTH_2 | SCREEN_HEIGHT_2 => {
                let a = it.func_args(self, kw)?;
                let s = if a.is_empty() {
                    self.cur_mut()?
                } else {
                    let n = check_screen(a.int(0))?;
                    self.screens
                        .get_mut(n)
                        .ok_or(Exc::Error(SCREEN_NOT_OPENED))?
                };
                Value::Int(if matches!(kw.token, SCREEN_WIDTH | SCREEN_WIDTH_2) {
                    s.width
                } else {
                    s.height
                } as i32)
            }
            SCREEN_COLOUR => Value::Int(self.cur_mut()?.screen_colour() as i32),
            SCREEN_BASE => Value::Int(0x0010_0000 + self.cur_screen()? as i32 * 0x1000),
            LOGBASE | PHYBASE => {
                let p = it.func_args(self, kw)?.int(0);
                let s = self.cur_mut()?;
                if p as u32 >= s.planes as u32 {
                    return err(FONCALL);
                }
                let b = if kw.token == LOGBASE {
                    s.logic
                } else {
                    s.physic
                };
                Value::Int(0x0020_0000 + ((s.number * 2 + b) as i32) * 0x10000 + p * 0x2000)
            }
            PHYSIC | LOGIC => Value::Int(if kw.token == PHYSIC {
                physic_value(None)
            } else {
                logic_value(None)
            }),
            PHYSIC_2 | LOGIC_2 => {
                let n = it.func_args(self, kw)?.int(0);
                Value::Int(if kw.token == PHYSIC_2 {
                    physic_value(Some(n))
                } else {
                    logic_value(Some(n))
                })
            }
            X_HARD | Y_HARD | X_SCREEN | Y_SCREEN | X_HARD_2 | Y_HARD_2 | X_SCREEN_2
            | Y_SCREEN_2 => {
                let a = it.func_args(self, kw)?;
                let (screen, v) = if a.len() > 1 {
                    (Some(a.int(0)), a.int(1))
                } else {
                    (None, a.int(0))
                };
                Value::Int(match kw.token {
                    X_HARD | X_HARD_2 => self.x_hard(screen, v)?,
                    Y_HARD | Y_HARD_2 => self.y_hard(screen, v)?,
                    X_SCREEN | X_SCREEN_2 => self.x_screen(screen, v)?,
                    _ => self.y_screen(screen, v)?,
                })
            }
            SCIN | SCIN_2 => {
                let a = it.func_args(self, kw)?;
                let (d3, x, y) = if kw.token == SCIN_2 {
                    (a.int(0).wrapping_add(1), a.int(1), a.int(2))
                } else {
                    (0, a.int(0), a.int(1))
                };
                self.cur_screen()?;
                let first = if d3 > 0 { self.screen_param(d3)? } else { None };
                Value::Int(
                    self.screens
                        .screen_at(x, y, first, 8)
                        .map_or(ENT_NUL, |n| n as i32),
                )
            }
            MOUSE_SCREEN => {
                self.cur_screen()?;
                let (x, y) = (self.input.mouse_x, self.input.mouse_y);
                Value::Int(
                    self.screens
                        .screen_at(x, y, None, 8)
                        .map_or(ENT_NUL, |n| n as i32),
                )
            }
            RAIN => {
                let a = it.func_args(self, kw)?;
                Value::Int(*self.rain_var(a.int(0), a.int(1))? as i32)
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// Screens at the start of a program (and `Default`, `DefRun1`):
    /// closes everything, restores the default palette and opens the
    /// default screen 0 (320x256, 16 colours, lowres) with colour 3
    /// flashing.
    pub(crate) fn screen_reset(&mut self) {
        self.screens = crate::gfx::Screens::new();
        let _ = self.screens.open(0, 320, 256, 16, 0);
        let _ = self.screens.effects.flash(0, 3, OPEN_FLASH);
    }

    /// Colour effects (flash, shift, fade) run by the VBL interrupt; pending
    /// display changes are applied once per frame if Auto View is on.
    pub(crate) fn screen_vbl(&mut self) {
        self.screens.effects_vbl();
        self.screen_update();
    }

    /// Screen part of the display update (`EcCopper`): rainbows moved by
    /// AMAL, then pending Screen Display / Offset (also written by AMAL).
    fn screen_update(&mut self) {
        for n in 0..4 {
            if std::mem::take(&mut self.sprites.rainbow_changed[n]) {
                let [x, y, a] = self.sprites.rainbow_act[n];
                let r = &mut self.screens.effects.rainbows[n];
                if (0..r.buf.len() as i32).contains(&(x as i32)) {
                    r.base = x as i32;
                }
                r.y = y as i32;
                r.height = a as i32;
            }
        }
        if self.screens.auto_view {
            self.screens.apply_pending();
        }
    }

    /// Applies pending Screen Display / Offset changes (copper rebuild).
    pub(crate) fn screen_test_point(&mut self, it: &mut Interp) -> R<()> {
        let _ = it;
        self.screen_update();
        Ok(())
    }
}
