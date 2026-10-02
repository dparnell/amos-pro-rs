//! User copper lists: Copper Off/On, Cop Reset, Cop Wait, Cop Move,
//! Cop Movel, Cop Swap, =Cop Logic (`+Lib.s:9431-9510`, `+W.s:6790-6907`).
//!
//! The lists live in the emulated chip memory (see `gfx::copper`), so a
//! program can also read and write them with Peek/Poke at `Cop Logic`.
//! While the copper is on, `=Cop Logic` returns a list equivalent to the
//! one AMOS builds for the screens (`EcCopper`), written at that address.

use super::Hardware;
use crate::gfx::copper::{self, ERR_NOT_DISABLED, ERR_PARAM, ERR_TOO_LONG, reg, wait_words};
use crate::interp::value::Value;
use crate::interp::{Interp, R, err};
use crate::tokens::{Keyword, tk};

/// Chip memory address of the bitplane `plane` of bitmap `bitmap` of
/// screen `screen` in the lists made by `=Cop Logic` (decoded by the frame
/// builder): 1 MB per bitmap, 128 KB per plane.
pub fn plane_address(screen: usize, bitmap: usize, plane: usize) -> u32 {
    0x0100_0000 + ((screen * 2 + bitmap) as u32) * 0x10_0000 + plane as u32 * 0x2_0000
}

impl Hardware {
    pub(crate) fn copper_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        match kw.token {
            tk::COPPER_OFF => {
                it.inst_args(self, kw)?;
                if self.copper.on {
                    // The registers are left as AMOS's own list leaves
                    // them; the new list is empty (TCopOn, +W.s:6791).
                    self.copper_init_regs();
                    self.copper.on = false;
                    self.copper.pos = self.copper.logic;
                    self.copper_swap()?;
                }
            }
            tk::COPPER_ON => {
                it.inst_args(self, kw)?;
                self.copper.on = true;
            }
            tk::COP_SWAP => {
                it.inst_args(self, kw)?;
                self.copper_swap()?;
            }
            tk::COP_RESET => {
                it.inst_args(self, kw)?;
                self.copper_off_check()?;
                self.copper.pos = self.copper.logic;
                self.copper.cop255 = false;
            }
            tk::COP_WAIT | tk::COP_WAIT_2 => {
                let a = it.inst_args(self, kw)?;
                let (mx, my) = if kw.token == tk::COP_WAIT_2 {
                    (a.int(2), a.int(3))
                } else {
                    (-1, -1)
                };
                self.cop_wait(a.int(0), a.int(1), mx, my)?;
            }
            tk::COP_MOVE => {
                let a = it.inst_args(self, kw)?;
                self.cop_move(a.int(0), a.int(1) as u16)?;
            }
            tk::COP_MOVEL => {
                let a = it.inst_args(self, kw)?;
                let (r, v) = (a.int(0), a.int(1) as u32);
                self.cop_move(r, (v >> 16) as u16)?;
                self.cop_move(r + 2, v as u16)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn copper_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 || kw.token != tk::COP_LOGIC {
            return Ok(None);
        }
        let _ = it;
        if self.copper.on {
            self.copper_build_amos_list();
        }
        Ok(Some(Value::Int(self.copper.logic as i32)))
    }

    fn copper_off_check(&self) -> R<()> {
        if self.copper.on {
            err(ERR_NOT_DISABLED)
        } else {
            Ok(())
        }
    }

    /// Writes one instruction at `T_CopPos` (`CopFin`: the length is
    /// checked after the write).
    fn cop_put(&mut self, w1: u16, w2: u16) -> R<()> {
        let pos = self.copper.pos;
        let mut b = [0u8; 4];
        b[..2].copy_from_slice(&w1.to_be_bytes());
        b[2..].copy_from_slice(&w2.to_be_bytes());
        self.banks.poke_bytes(pos, &b);
        self.copper.pos = pos + 4;
        if self.copper.pos - self.copper.logic >= copper::LIST_LENGTH {
            return err(ERR_TOO_LONG);
        }
        Ok(())
    }

    /// `Cop Wait x,y[,maskx,masky]` (`TCopWt`): lines from 256 need the
    /// wait for the end of line 255 first, inserted once.
    fn cop_wait(&mut self, x: i32, y: i32, mx: i32, my: i32) -> R<()> {
        self.copper_off_check()?;
        if !(0..313).contains(&x) || !(0..313).contains(&y) {
            return err(ERR_PARAM);
        }
        if y >= 256 && !self.copper.cop255 {
            self.cop_put(0xFFE1, 0xFFFE)?;
            self.copper.cop255 = true;
        }
        let (w1, w2) = wait_words(x, y, mx, my);
        self.cop_put(w1, w2)
    }

    /// `Cop Move addr,value` (`TCopMv`).
    fn cop_move(&mut self, r: i32, v: u16) -> R<()> {
        self.copper_off_check()?;
        if !(0..512).contains(&r) {
            return err(ERR_PARAM);
        }
        self.cop_put((r as u16) & 0x1FE, v)
    }

    /// `Cop Swap` (`TCopSw`): ends the logic list, shows it and starts a
    /// new one.
    fn copper_swap(&mut self) -> R<()> {
        self.copper_off_check()?;
        let pos = self.copper.pos;
        self.banks.poke_bytes(pos, &[0xFF, 0xFF, 0xFF, 0xFE]);
        std::mem::swap(&mut self.copper.logic, &mut self.copper.physic);
        self.copper.pos = self.copper.logic;
        self.copper.cop255 = false;
        Ok(())
    }

    /// Register values at the end of AMOS's own copper list: display set
    /// up for the front screen, bitplane DMA off and colour 0 = Colour Back
    /// (`EcCopBa`).
    fn copper_init_regs(&mut self) {
        let regs = self.screen_copper_regs(self.screens.priority.first().copied());
        let c = &mut self.copper;
        for (r, v) in regs {
            c.regs[(r / 2) as usize] = v;
        }
        c.regs[(reg::DMACON / 2) as usize] &= !0x0100;
        c.regs[(reg::COLOR00 / 2) as usize] = self.screens.colour_back;
    }

    /// Display registers of screen `n` for the slice starting at line
    /// `l0`... (`EcCopHo`, +W.s:6272): DIW, DDF, modulos, BPLCON0-2.
    fn screen_copper_regs(&self, n: Option<usize>) -> Vec<(u16, u16)> {
        let Some(s) = n.and_then(|n| self.screens.get(n)) else {
            return Vec::new();
        };
        let (wx, wy) = (s.display_x, s.display_y);
        let ww = s.display_w as i32;
        let wh = s.display_h as i32;
        let fine = s.offset_x & 15;
        let lowres_ddf = |x: i32| if s.hires { (x - 8) / 2 } else { (x - 16) / 2 };
        let mut ddf0 = lowres_ddf(wx) & !7;
        let words = (if s.hires { ww * 2 } else { ww } + 15) / 16;
        let mut ddf1 = if s.hires {
            ddf0 + (words - 2) * 4
        } else {
            ddf0 + (words - 1) * 8
        };
        if fine != 0 {
            ddf0 -= 8;
            ddf1 = ddf1.max(ddf0);
        }
        let fetched = if s.hires {
            ((ddf1 - ddf0) / 4 + 2) * 2
        } else {
            ((ddf1 - ddf0) / 8 + 1) * 2
        };
        let bpr = (s.width / 8) as i32;
        let lace = if s.lace { bpr } else { 0 };
        let modulo = (bpr - fetched + lace) as u16;
        let delay = if fine != 0 { (16 - fine) as u16 } else { 0 };
        let mut con0 = ((s.planes as u16 & 7) << 12) | 0x0200;
        if s.hires {
            con0 |= 0x8000;
        }
        if s.ham {
            con0 |= 0x0800;
        }
        if s.lace {
            con0 |= 0x0004;
        }
        let vstop = wy + wh;
        vec![
            (
                reg::DIWSTRT,
                (((wy & 0xFF) << 8) | ((wx + 1) & 0xFF)) as u16,
            ),
            (
                reg::DIWSTOP,
                (((vstop & 0xFF) << 8) | ((wx + 1 + ww) & 0xFF)) as u16,
            ),
            (reg::DDFSTRT, ddf0 as u16),
            (reg::DDFSTOP, ddf1 as u16),
            (reg::BPL1MOD, modulo),
            (reg::BPL2MOD, modulo),
            (reg::BPLCON0, con0),
            (reg::BPLCON1, delay | (delay << 4)),
            (reg::BPLCON2, if s.dual_priority { 0x64 } else { 0x24 }),
        ]
    }

    /// Writes into the logic list the copper list AMOS makes for the
    /// screens (`EcCopper`, +W.s:5700; simplified: no rainbows, no dual
    /// playfield): a short wait and the 16 sprite pointers (`CpInit`), then
    /// for each slice the screen set up on its first line, bitplane DMA on
    /// from the next, and colour back after the last line.
    fn copper_build_amos_list(&mut self) {
        let mut l: Vec<(u16, u16)> = vec![(0x1003, 0xFFFE)];
        for i in 0..16 {
            l.push((0x120 + 2 * i, 0));
        }
        let mut wrapped = false;
        let mut wait = |l: &mut Vec<(u16, u16)>, line: i32| {
            if line >= 256 && !wrapped {
                l.push((0xFFDF, 0xFFFE));
                wrapped = true;
            }
            l.push((((line as u16 & 0xFF) << 8) | 0x01, 0xFFFE));
        };
        let runs = self.line_runs();
        for (k, &(n, l0, l1)) in runs.iter().enumerate() {
            let Some(s) = self.screens.get(n) else {
                continue;
            };
            let (shown, bm) = self.shown_bitmap(n);
            let pal = s.palette;
            let bpr = (s.width / 8) as i32;
            let first = l0 + 1;
            let row = if s.lace {
                2 * (first - s.display_y) + s.offset_y
            } else {
                first - s.display_y + s.offset_y
            };
            let start = (row * bpr + (s.offset_x >> 4) * 2).max(0) as u32;
            let planes = s.planes as usize;
            wait(&mut l, l0);
            l.push((reg::DMACON, 0x0100));
            for (i, &c) in pal.iter().take(16).enumerate() {
                l.push((reg::COLOR00 + 2 * i as u16, c));
            }
            for p in 0..planes.min(6) {
                let a = plane_address(shown, bm, p) + start;
                l.push((reg::BPL1PT + 4 * p as u16, (a >> 16) as u16));
                l.push((reg::BPL1PT + 4 * p as u16 + 2, a as u16));
            }
            l.extend(self.screen_copper_regs(Some(n)));
            wait(&mut l, first);
            l.push((reg::DMACON, 0x8100));
            for (i, &c) in pal.iter().enumerate().skip(16) {
                l.push((reg::COLOR00 + 2 * i as u16, c));
            }
            if runs.get(k + 1).is_none_or(|r| r.1 != l1) {
                wait(&mut l, l1);
                l.push((reg::DMACON, 0x0100));
                l.push((reg::COLOR00, self.screens.colour_back));
            }
        }
        l.push((0xFFFF, 0xFFFE));
        let mut bytes = Vec::with_capacity(l.len() * 4);
        for (a, b) in l.iter().take((copper::LIST_LENGTH / 4) as usize) {
            bytes.extend_from_slice(&a.to_be_bytes());
            bytes.extend_from_slice(&b.to_be_bytes());
        }
        let at = self.copper.logic;
        self.banks.poke_bytes(at, &bytes);
    }
}

#[cfg(test)]
mod tests {
    use crate::display::{DISPLAY_WIDTH, render_rgba};
    use crate::interp::{RunState, StopReasonOrError};
    use crate::machine::Machine;

    fn run(src: &str, frames: usize) -> Machine {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).expect("tokenise");
        let mut m = Machine::new();
        m.run_program(&prg).expect("verify");
        for _ in 0..frames {
            m.vbl();
        }
        m
    }

    fn at(img: &[u8], hx: i32, hy: i32) -> [u8; 3] {
        let x = crate::display::hw_x_to_display(hx) as usize;
        let y = crate::display::hw_y_to_display(hy) as usize;
        let o = (y * DISPLAY_WIDTH as usize + x) * 4;
        [img[o], img[o + 1], img[o + 2]]
    }

    #[test]
    fn errors_need_copper_off() {
        let m = run("Cop Move $180,0", 2);
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == StopReasonOrError::Error(76))
        );
        let m = run("Copper Off : Cop Wait 0,400", 2);
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == StopReasonOrError::Error(78))
        );
        let m = run("Copper Off : Do : Cop Move $180,0 : Loop", 5);
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == StopReasonOrError::Error(77))
        );
    }

    #[test]
    fn user_colour_bars() {
        let src = "Copper Off\nCop Move $180,$F00\nCop Wait 0,100\nCop Move $180,$0F0\nCop Wait 0,280\nCop Move $180,$00F\nCop Swap\nDo : Wait Vbl : Loop";
        let mut m = run(src, 5);
        let img = render_rgba(&m.frame());
        assert_eq!(at(&img, 200, 50), [0xFF, 0, 0]);
        assert_eq!(at(&img, 200, 150), [0, 0xFF, 0]);
        assert_eq!(at(&img, 200, 290), [0, 0, 0xFF]);
        // Back to normal.
        m.hw.copper.on = true;
        let img = render_rgba(&m.frame());
        assert_eq!(at(&img, 400, 200), [0xAA, 0x44, 0]);
    }

    #[test]
    fn amos_list_gives_the_same_picture() {
        // The list returned by Cop Logic, copied and shown with Copper Off,
        // shows the default screen as AMOS does (Cop Move 0,0 reserves the
        // room, as in Multi_Rainbows.AMOS).
        let src = "Curs Off : Cls 1 : Ink 2 : Bar 10,10 To 50,30 : Screen Offset 0,3,0\nWait Vbl\n\
                   A=Cop Logic : L=0 : Dim C(3000)\nRepeat : C(L)=Leek(A+L*4) : Inc L : Until C(L-1)=$FFFFFFFE\n\
                   Copper Off : B=Cop Logic\nFor I=0 To L-1 : Cop Move 0,0 : Loke B+I*4,C(I) : Next\nCop Swap\nDo : Wait Vbl : Loop";
        let mut m = run(src, 10);
        assert!(matches!(m.state, RunState::Running), "{:?}", m.state);
        assert!(!m.hw.copper.on);
        let img = render_rgba(&m.frame());
        m.hw.copper.on = true;
        let reference = render_rgba(&m.frame());
        for (hx, hy) in [
            (140, 60),
            (138, 52),
            (128 + 47, 42 + 25),
            (300, 200),
            (100, 100),
            (400, 41),
            (400, 302),
        ] {
            assert_eq!(at(&img, hx, hy), at(&reference, hx, hy), "at {hx},{hy}");
        }
    }
}
