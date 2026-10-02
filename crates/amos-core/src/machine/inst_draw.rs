//! Drawing primitives, graphic text, blocks, Screen Copy/Zoom/Appear,
//! IFF pictures and packed pictures (Compact extension).

use super::Hardware;
use crate::banks::{Bank, BankData};
use crate::errors;
use crate::gfx::blocks;
use crate::gfx::draw::{Canvas, GrState, Pattern, builtin_pattern};
use crate::gfx::screen::{Screen, lib_error};
use crate::gfx::{gfont, iff, pack};
use crate::interp::value::Value;
use crate::interp::{Exc, Host, Interp, R, WaitKind, err};
use crate::tokens::*;

/// Wait id of `Appear` (instruction specific state).
const APPEAR_WAIT: u32 = 0x4150_5045;

/// Coordinates are 16 bit words in the RastPort.
fn w16(v: i32) -> i32 {
    v as i16 as i32
}

fn opt16(v: Option<i32>) -> Option<i32> {
    v.map(w16)
}

/// "Not a packed bitmap" (Compact extension error 0).
fn no_pac<T>() -> R<T> {
    Err(Exc::Message("Not a packed bitmap".into()))
}

impl Hardware {
    /// Drawing state at the start of a program (`+ILib.s:202-229`): blocks,
    /// compressed blocks, font list and scroll zones are erased.
    // To be called by `Hardware::reset`.
    #[allow(dead_code)]
    /// Runs `f` on the drawing state shared by all screens.
    fn with_draw<T>(&mut self, f: impl FnOnce(&mut blocks::DrawGlobals) -> T) -> T {
        f(&mut self.draw)
    }

    pub(crate) fn draw_reset(&mut self) {
        self.with_draw(|g| *g = blocks::DrawGlobals::default());
    }

    /// The current screen, or "Screen not opened".
    fn draw_screen(&mut self) -> R<&mut Screen> {
        match self.screens.current_mut() {
            Some(s) => Ok(s),
            None => err(errors::SCREEN_NOT_OPENED),
        }
    }

    fn gr(&mut self) -> R<&mut GrState> {
        Ok(&mut self.draw_screen()?.gr)
    }

    /// Runs a drawing operation through the RastPort: into the logic
    /// bitmap, and again into the physic one when Autoback is on for a
    /// double buffered screen (`L_GfxFunc` `+Lib.s:11249`, simplified).
    fn draw_op(&mut self, f: impl FnMut(&mut GrState, &mut Canvas)) -> R<()> {
        self.draw_op_with(true, f)
    }

    fn draw_op_with(
        &mut self,
        autoback: bool,
        mut f: impl FnMut(&mut GrState, &mut Canvas),
    ) -> R<()> {
        let s = self.draw_screen()?;
        let targets = if autoback {
            s.autoback_targets()
        } else {
            vec![s.logic]
        };
        let saved = s.gr.clone();
        let (w, h, planes) = (s.width, s.height, s.planes);
        for (k, &bi) in targets.iter().enumerate() {
            let mut other;
            let g: &mut GrState = if k == 0 {
                &mut s.gr
            } else {
                other = saved.clone();
                &mut other
            };
            let mut c = Canvas::new(&mut s.bitmaps[bi], w, h, planes);
            f(g, &mut c);
        }
        s.version += 1;
        Ok(())
    }

    /// Moves the graphic cursor, omitted coordinates are kept (`GrXY`).
    fn gr_xy(&mut self, x: Option<i32>, y: Option<i32>) -> R<(i32, i32)> {
        let g = self.gr()?;
        if let Some(y) = opt16(y) {
            g.y = y;
        }
        if let Some(x) = opt16(x) {
            g.x = x;
        }
        Ok((g.x, g.y))
    }

    /// Screen and bitmap of a Screen Copy / Zoom / Appear parameter
    /// (`GetEc` `+Lib.s:11289`): a screen number, or a `Physic`/`Logic`
    /// value (bit 31 set, bit 30 = physic, negative low word = current).
    fn get_ec(&self, v: i32) -> R<(usize, usize)> {
        let (n, physic) = if v >= 0 {
            if v >= 8 {
                return err(errors::ILLEGAL_FUNCTION_CALL);
            }
            (v as usize, false)
        } else {
            let lw = v as i16;
            let n = if lw < 0 {
                match self.screens.current {
                    Some(c) => c,
                    None => return err(errors::SCREEN_NOT_OPENED),
                }
            } else {
                if lw >= 8 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                lw as usize
            };
            (n, v & (1 << 30) != 0)
        };
        match self.screens.get(n) {
            Some(s) => Ok((n, if physic { s.physic } else { s.logic })),
            None => err(errors::SCREEN_NOT_OPENED),
        }
    }

    /// Data of a "bank or address" parameter (`Bnk.OrAdr`).
    fn bank_data(&self, v: i32) -> R<Vec<u8>> {
        if (0..1024).contains(&v) {
            return match self.banks.raw(v as u16) {
                Some(d) => Ok(d.to_vec()),
                None => err(errors::BANK_NOT_RESERVED),
            };
        }
        let addr = v as u32;
        for (&n, b) in &self.banks.banks {
            if let (Some(start), Some(d)) = (self.banks.start(n), b.raw())
                && addr >= start
                && ((addr - start) as usize) < d.len()
            {
                return Ok(d[(addr - start) as usize..].to_vec());
            }
        }
        err(errors::ILLEGAL_FUNCTION_CALL)
    }

    pub(crate) fn draw_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot == 2 {
            return self.compact_instruction(it, kw);
        }
        if kw.slot != 0 {
            return Ok(false);
        }
        match kw.token {
            tk::INK | tk::INK_2 | tk::INK_3 => {
                let a = it.inst_args(self, kw)?;
                let g = self.gr()?;
                if let Some(c) = a.opt(2) {
                    g.outline = c as u8;
                }
                if let Some(b) = a.opt(1) {
                    g.paper = b as u8;
                }
                if let Some(i) = a.opt(0) {
                    g.ink = i as u8;
                }
            }
            tk::GR_WRITING => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                let n = a.int(0);
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.gr()?.writing = n as u8;
            }
            tk::SET_LINE => {
                let a = it.inst_args(self, kw)?;
                self.gr()?.line_pattern = a.int(0) as u16;
            }
            tk::SET_PAINT => {
                let a = it.inst_args(self, kw)?;
                self.gr()?.paint_outline = a.int(0) != 0;
            }
            tk::SET_PATTERN => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                let n = a.int(0) as i16 as i32;
                let p = self.find_pattern(n)?;
                let g = self.gr()?;
                g.pattern_number = n;
                g.pattern = p;
            }
            tk::SET_TEXT => {
                let a = it.inst_args(self, kw)?;
                self.gr()?.text_style = a.int(0) as u8;
            }
            tk::CLIP => {
                it.inst_args(self, kw)?;
                let g = self.gr()?;
                g.clip = (0, 0, i32::MAX, i32::MAX);
            }
            tk::CLIP_2 => {
                let a = it.inst_args(self, kw)?;
                let s = self.draw_screen()?;
                let (tx, ty) = (s.width as i32, s.height as i32);
                let old = s.gr.clip;
                let x0 = a.opt(0).unwrap_or(old.0.min(tx));
                let y0 = a.opt(1).unwrap_or(old.1.min(ty));
                let x1 = a.opt(2).unwrap_or(old.2.min(tx));
                let y1 = a.opt(3).unwrap_or(old.3.min(ty));
                // TSClip +W.s:4187.
                if x0 < 0 || y0 < 0 || x1 > tx || y1 > ty || x1 <= x0 || y1 <= y0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                s.gr.clip = (x0, y0, x1, y1);
            }
            tk::SET_TEMPRAS | tk::SET_TEMPRAS_2 | tk::SET_TEMPRAS_3 => {
                // The temporary raster is not needed: only the checks remain.
                let a = it.inst_args(self, kw)?;
                if !a.is_empty() {
                    let size = a.int(a.len() - 1);
                    if !(256..65536).contains(&size) {
                        return err(errors::ILLEGAL_FUNCTION_CALL);
                    }
                }
            }
            tk::PLOT | tk::PLOT_2 => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                if let Some(c) = a.opt(2) {
                    if c < 0 {
                        return err(errors::ILLEGAL_FUNCTION_CALL);
                    }
                    self.gr()?.ink = c as u8;
                }
                let (x, y) = self.gr_xy(a.opt(0), a.opt(1))?;
                self.draw_op(|g, c| g.plot(c, x, y))?;
            }
            tk::GR_LOCATE => {
                let a = it.inst_args(self, kw)?;
                self.gr_xy(a.opt(0), a.opt(1))?;
            }
            tk::DRAW_TO => {
                let a = it.inst_args(self, kw)?;
                let g = self.gr()?;
                let x = opt16(a.opt(0)).unwrap_or(g.x);
                let y = opt16(a.opt(1)).unwrap_or(g.y);
                self.draw_op(|g, c| g.draw_to(c, x, y))?;
            }
            tk::DRAW => {
                let a = it.inst_args(self, kw)?;
                self.gr_xy(a.opt(0), a.opt(1))?;
                let g = self.gr()?;
                let x = opt16(a.opt(2)).unwrap_or(g.x);
                let y = opt16(a.opt(3)).unwrap_or(g.y);
                self.draw_op(|g, c| g.draw_to(c, x, y))?;
            }
            tk::BOX => {
                let a = it.inst_args(self, kw)?;
                let (x1, y1, x2, y2) = (w16(a.int(0)), w16(a.int(1)), w16(a.int(2)), w16(a.int(3)));
                self.draw_op(|g, c| g.draw_box(c, x1, y1, x2, y2))?;
            }
            tk::BAR => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                let (x1, y1, x2, y2) = (w16(a.int(0)), w16(a.int(1)), w16(a.int(2)), w16(a.int(3)));
                if x2 <= x1 || y2 <= y1 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.draw_op(|g, c| g.bar(c, x1, y1, x2, y2))?;
            }
            tk::CIRCLE => {
                let a = it.inst_args(self, kw)?;
                let r = a.int(2);
                if r == 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let hires = self.draw_screen()?.hires;
                let rx = if hires { w16(r << 1) } else { w16(r) };
                let (x, y) = self.gr_xy(a.opt(0), a.opt(1))?;
                self.draw_op(|g, c| g.ellipse(c, x, y, rx, w16(r)))?;
            }
            tk::ELLIPSE => {
                let a = it.inst_args(self, kw)?;
                let (rx, ry) = (a.int(2), a.int(3));
                if rx == 0 || ry == 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.draw_screen()?;
                let (x, y) = self.gr_xy(a.opt(0), a.opt(1))?;
                self.draw_op(|g, c| g.ellipse(c, x, y, w16(rx), w16(ry)))?;
            }
            tk::POLYLINE => {
                self.draw_screen()?;
                let pts = self.read_points(it, false)?;
                if pts.len() < 2 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let (x0, y0) = pts[0];
                self.draw_op(|g, c| {
                    g.x = x0;
                    g.y = y0;
                    let pen = g.ink;
                    g.poly_draw(c, &pts, false, pen);
                })?;
            }
            tk::POLYGON => {
                self.draw_screen()?;
                let pts = self.read_points(it, true)?;
                // Polygon calls the area functions directly: no autoback.
                self.draw_op_with(false, |g, c| g.polygon(c, &pts))?;
            }
            tk::PAINT | tk::PAINT_2 => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                let (x, y) = self.gr_xy(a.opt(0), a.opt(1))?;
                let seed = self.draw_screen()?.pixel(x, y);
                if let Some(seed) = seed {
                    self.draw_op(|g, c| g.paint(c, x, y, seed))?;
                }
            }
            tk::TEXT => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                let s = a.str(2);
                let (x, y) = self.gr_xy(a.opt(0), a.opt(1))?;
                if !s.is_empty() {
                    self.draw_op(|g, c| {
                        let adv = g.text(c, x, y, &s);
                        g.x = x + adv;
                    })?;
                }
            }
            tk::GET_FONTS | tk::GET_DISC_FONTS | tk::GET_ROM_FONTS => {
                it.inst_args(self, kw)?;
                let (rom, disc) = match kw.token {
                    tk::GET_ROM_FONTS => (true, false),
                    tk::GET_DISC_FONTS => (false, true),
                    _ => (true, true),
                };
                self.with_draw(|g| g.fonts = Some(blocks::font_list(rom, disc)));
            }
            tk::SET_FONT => {
                let a = it.inst_args(self, kw)?;
                self.draw_screen()?;
                let n = a.int(0);
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                if n > 0 {
                    // TSFont / TGFont +W.s:4878.
                    match self.with_draw(|g| g.fonts.as_ref().map(|f| f.len())) {
                        None => return err(37),
                        Some(len) if n as usize > len => return err(44),
                        _ => {}
                    }
                }
                self.gr()?.font = n;
            }
            tk::GET_BLOCK | tk::GET_BLOCK_2 => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if n == 0 || !(0..65536).contains(&n) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                let (x, y, w, h) = (w16(a.int(1)), w16(a.int(2)), w16(a.int(3)), w16(a.int(4)));
                let mask = a.opt(5).unwrap_or(0) != 0;
                let s = self.draw_screen()?;
                let (tx, ty) = (s.width as i32, s.height as i32);
                if x < 0 || y < 0 || w <= 0 || h <= 0 || y + h > ty || x + w > tx {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                let b = blocks::grab_block(s.logic_ref(), tx, n, x, y, w, h, s.planes, mask);
                self.with_draw(|g| {
                    if let Some(old) = g.blocks.iter_mut().find(|o| o.number == n) {
                        *old = b;
                    } else {
                        g.blocks.insert(0, b);
                    }
                });
            }
            tk::PUT_BLOCK | tk::PUT_BLOCK_2 | tk::PUT_BLOCK_3 | tk::PUT_BLOCK_4 => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if n == 0 || !(0..65536).contains(&n) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                let Some(b) = self.with_draw(|g| g.blocks.iter().find(|o| o.number == n).cloned())
                else {
                    return err(blocks::BLOCK_NOT_DEFINED);
                };
                let x = opt16(a.opt(1)).unwrap_or(b.x);
                let y = opt16(a.opt(2)).unwrap_or(b.y);
                let planes = a.opt(3).map_or(0xFFFF, |p| p as u16);
                let minterm = a.opt(4).map(|m| m as u8);
                let s = self.draw_screen()?;
                let (w, h, sp) = (s.width as i32, s.height as i32, s.planes);
                let clip = s.gr.clip;
                blocks::put_block(&b, s.logic_mut(), w, h, sp, clip, x, y, planes, minterm);
            }
            tk::DEL_BLOCK => {
                it.inst_args(self, kw)?;
                self.with_draw(|g| g.blocks.clear());
            }
            tk::DEL_BLOCK_2 => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if n == 0 || !(0..65536).contains(&n) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                if !self.with_draw(|g| {
                    let l = g.blocks.len();
                    g.blocks.retain(|b| b.number != n);
                    l != g.blocks.len()
                }) {
                    return err(blocks::BLOCK_NOT_FOUND);
                }
            }
            tk::HREV_BLOCK | tk::VREV_BLOCK => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                let h = kw.token == tk::HREV_BLOCK;
                if !self.with_draw(|g| match g.blocks.iter_mut().find(|b| b.number == n) {
                    Some(b) => {
                        b.flip(h);
                        true
                    }
                    None => false,
                }) {
                    return err(blocks::BLOCK_NOT_DEFINED);
                }
            }
            tk::GET_CBLOCK => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if n == 0 || !(0..65536).contains(&n) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                let s = self.draw_screen()?;
                let Some(b) = blocks::make_cblock(
                    s.logic_ref(),
                    s.width as i32,
                    s.height as i32,
                    s.planes,
                    n,
                    a.int(1),
                    a.int(2),
                    a.int(3),
                    a.int(4),
                ) else {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                };
                self.with_draw(|g| {
                    g.cblocks.retain(|o| o.number != n);
                    g.cblocks.insert(0, b);
                });
            }
            tk::PUT_CBLOCK | tk::PUT_CBLOCK_2 => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if n == 0 || !(0..65536).contains(&n) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                let Some(b) = self.with_draw(|g| g.cblocks.iter().find(|o| o.number == n).cloned())
                else {
                    return err(blocks::BLOCK_NOT_FOUND);
                };
                let x = a.opt(1).map_or(-1, w16);
                let y = a.opt(2).map_or(-1, w16);
                let s = self.draw_screen()?;
                let (w, h, p) = (s.width as i32, s.height as i32, s.planes);
                if !blocks::put_cblock(&b, s.logic_mut(), w, h, p, x, y) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
            }
            tk::DEL_CBLOCK => {
                it.inst_args(self, kw)?;
                self.with_draw(|g| g.cblocks.clear());
            }
            tk::DEL_CBLOCK_2 => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if n == 0 || !(0..65536).contains(&n) {
                    return err(blocks::ILLEGAL_BLOCK_PARAMETERS);
                }
                if !self.with_draw(|g| {
                    let l = g.cblocks.len();
                    g.cblocks.retain(|b| b.number != n);
                    l != g.cblocks.len()
                }) {
                    return err(blocks::BLOCK_NOT_FOUND);
                }
            }
            tk::SCREEN_COPY | tk::SCREEN_COPY_2 => {
                let a = it.inst_args(self, kw)?;
                let m = a.opt(2).unwrap_or(0xCC);
                self.screen_copy(a.int(0), 0, 0, 10000, 10000, a.int(1), 0, 0, m)?;
            }
            tk::SCREEN_COPY_3 | tk::SCREEN_COPY_4 => {
                let a = it.inst_args(self, kw)?;
                let m = a.opt(8).unwrap_or(0xCC);
                self.screen_copy(
                    a.int(0),
                    a.int(1),
                    a.int(2),
                    a.int(3),
                    a.int(4),
                    a.int(5),
                    a.int(6),
                    a.int(7),
                    m,
                )?;
            }
            tk::DEF_SCROLL => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if !(1..=10).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let z = [a.int(1), a.int(2), a.int(3), a.int(4), a.int(5), a.int(6)].map(w16);
                self.with_draw(|g| g.scrolls[n as usize - 1] = Some(z));
            }
            tk::SCROLL => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0) - 1;
                if !(0..10).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let Some([x1, y1, x2, y2, dx, dy]) = self.with_draw(|g| g.scrolls[n as usize]) else {
                    return err(72);
                };
                let s = self.draw_screen()?;
                let (w, h, planes) = (s.width as i32, s.height as i32, s.planes);
                if let Some(r) = blocks::clip_copy(w, h, w, h, x1, y1, x2, y2, x1 + dx, y1 + dy) {
                    for bi in s.autoback_targets() {
                        let src = blocks::extract(&s.bitmaps[bi], w, &r);
                        blocks::blit(&src, s.bitmap_mut(bi), w, &r, 0xCC, planes);
                    }
                }
            }
            tk::ZOOM => {
                let a = it.inst_args(self, kw)?;
                self.zoom(&a)?;
            }
            tk::APPEAR | tk::APPEAR_2 => {
                let a = it.inst_args(self, kw)?;
                return self
                    .appear(it, a.int(0), a.int(1), a.int(2), a.opt(3).unwrap_or(0))
                    .map(|_| true);
            }
            tk::MASK_IFF => {
                let a = it.inst_args(self, kw)?;
                self.with_draw(|g| g.iff_mask_off = !(a.int(0) as u32));
            }
            tk::LOAD_IFF | tk::LOAD_IFF_2 => {
                let a = it.inst_args(self, kw)?;
                let data = self.files_read_all(&a.str(0))?;
                self.load_iff_bytes(it, &data, a.opt(1))?;
            }
            tk::SAVE_IFF | tk::SAVE_IFF_2 => {
                let a = it.inst_args(self, kw)?;
                let comp = a.opt(1).unwrap_or(1);
                let data = self.save_iff_bytes(comp)?;
                self.files_write_all(&a.str(0), &data)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn draw_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        let v = match kw.token {
            tk::POINT => {
                let a = it.func_args(self, kw)?;
                self.draw_screen()?;
                let (x, y) = self.gr_xy(a.opt(0), a.opt(1))?;
                let s = self.draw_screen()?;
                s.pixel(x, y).map_or(-1, |p| p as i32)
            }
            tk::XGR => (self.gr()?.x as u16) as i32,
            tk::YGR => (self.gr()?.y as u16) as i32,
            tk::TEXT_LENGTH => {
                let a = it.func_args(self, kw)?;
                self.draw_screen()?;
                gfont::text_length(&a.str(0))
            }
            tk::TEXT_BASE => {
                self.draw_screen()?;
                gfont::BASELINE
            }
            tk::TEXT_STYLES => self.gr()?.text_style as i32,
            // FnPicture +Lib.s:4343 returns a constant.
            tk::PICTURE => 0b111_1111,
            tk::FONT_S => {
                let a = it.func_args(self, kw)?;
                let n = a.int(0);
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let Some(fonts) = self.with_draw(|g| g.fonts.clone()) else {
                    return err(37);
                };
                if n == 0 || n as usize > fonts.len() {
                    return Ok(Some(Value::str(b"")));
                }
                // FnFont +Lib.s:9757: 38 characters, size at 30, kind at 34.
                let f = &fonts[n as usize - 1];
                let mut s = vec![b' '; 38];
                for (i, c) in f.name.bytes().take(30).enumerate() {
                    s[i] = c;
                }
                for (i, c) in f.size.to_string().bytes().take(4).enumerate() {
                    s[30 + i] = c;
                }
                s[34..38].copy_from_slice(if f.kind == 1 { b"Rom " } else { b"Disc" });
                return Ok(Some(Value::str(&s)));
            }
            _ => return Ok(None),
        };
        Ok(Some(Value::Int(v)))
    }

    /// `Set Pattern n`: built-in pattern (n > 0), sprite bank image -n
    /// (n < 0, silently ignored if missing) or solid (0).
    fn find_pattern(&self, n: i32) -> R<Option<Pattern>> {
        if n == 0 {
            return Ok(None);
        }
        if n > 0 {
            return match builtin_pattern(n as usize) {
                Some(p) => Ok(Some(p)),
                None => err(errors::ILLEGAL_FUNCTION_CALL),
            };
        }
        let img = match self.banks.get(1).map(|b| &b.data) {
            Some(BankData::Images { images, .. }) => images.get((-n) as usize - 1),
            _ => None,
        };
        let Some(img) = img.filter(|i| !i.is_empty()) else {
            return Ok(None);
        };
        match Pattern::from_image(
            img.width_words as usize,
            img.height as usize,
            img.planes as usize,
            &img.planar,
        ) {
            Some(p) => Ok(Some(p)),
            None => err(errors::ILLEGAL_FUNCTION_CALL),
        }
    }

    /// Reads the point list of `Polyline` / `Polygon`: `x,y To x,y...` or
    /// `To x,y...` starting from the graphic cursor.
    fn read_points(&mut self, it: &mut Interp, polygon: bool) -> R<Vec<(i32, i32)>> {
        let mut pts = Vec::new();
        if it.peek() == TK_TO {
            let g = self.gr()?;
            pts.push((g.x, g.y));
        } else {
            let x = it.eval_int(self)?;
            it.expect(TK_COMMA)?;
            let y = it.eval_int(self)?;
            pts.push((w16(x), w16(y)));
        }
        while it.peek() == TK_TO {
            it.pc += 2;
            let x = it.eval_int(self)?;
            it.expect(TK_COMMA)?;
            let y = it.eval_int(self)?;
            pts.push((w16(x), w16(y)));
        }
        let _ = polygon;
        Ok(pts)
    }

    /// `Screen Copy` (`Sco0` `+Lib.s:10298`): BltBitMap with a minterm on
    /// min(planes) planes; the destination region may overlap the source.
    #[allow(clippy::too_many_arguments)]
    fn screen_copy(
        &mut self,
        s1: i32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        s2: i32,
        dx: i32,
        dy: i32,
        minterm: i32,
    ) -> R<()> {
        let (sn, sb) = self.get_ec(s1)?;
        let (dn, db) = self.get_ec(s2)?;
        let src = self.screens.get(sn).unwrap();
        let (sw, sh, sp) = (src.width as i32, src.height as i32, src.planes);
        let dst = self.screens.get(dn).unwrap();
        let (dw, dh, dp) = (dst.width as i32, dst.height as i32, dst.planes);
        let Some(r) = blocks::clip_copy(sw, sh, dw, dh, x1, y1, x2, y2, dx, dy) else {
            return Ok(());
        };
        let data = blocks::extract(&src.bitmaps[sb], sw, &r);
        let dst = self.screens.get_mut(dn).unwrap();
        blocks::blit(&data, dst.bitmap_mut(db), dw, &r, minterm as u8, sp.min(dp));
        Ok(())
    }

    /// `Zoom s1,x1,y1,x2,y2 To s2,x3,y3,x4,y4` (`InZoom` `+Lib.s:10531`).
    fn zoom(&mut self, a: &crate::interp::params::Args) -> R<()> {
        let (sn, sb) = self.get_ec(a.int(0))?;
        let (dn, db) = self.get_ec(a.int(5))?;
        let (x1, y1, x2, y2) = (a.int(1), a.int(2), a.int(3), a.int(4));
        let (x3, y3, x4, y4) = (a.int(6), a.int(7), a.int(8), a.int(9));
        let src = self.screens.get(sn).unwrap();
        let (sw, sh, sp) = (src.width as i32, src.height as i32, src.planes);
        let dst = self.screens.get(dn).unwrap();
        let (dw, dh, dp) = (dst.width as i32, dst.height as i32, dst.planes);
        // The original compares unsigned: negative starts are errors.
        let bad = |a1: i32, a2: i32, lim_s: i32, b1: i32, b2: i32, lim_d: i32| {
            b2 < 0
                || b2 > lim_d
                || (b1 as u32) >= (b2 as u32)
                || a2 < 0
                || a2 > lim_s
                || (a1 as u32) >= (a2 as u32)
        };
        if bad(x1, x2, sw, x3, x4, dw) || bad(y1, y2, sh, y3, y4, dh) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        let tx = blocks::zoom_table(x1, x2, x3, x4);
        let ty = blocks::zoom_table(y1, y2, y3, y4);
        let data = src.bitmaps[sb].clone();
        let dst = self.screens.get_mut(dn).unwrap();
        blocks::zoom(
            &data,
            sw,
            dst.bitmap_mut(db),
            dw,
            x1,
            y1,
            x3,
            y3,
            &tx,
            &ty,
            sp.min(dp),
        );
        Ok(())
    }

    /// `Appear s1 To s2,pixel[,n]`: copies the pixels one at a time in a
    /// pseudo random order. The original runs for several seconds while the
    /// display keeps updating: here the instruction copies a 68000-speed
    /// estimate of pixels per frame and waits for the next one.
    fn appear(&mut self, it: &mut Interp, s1: i32, s2: i32, pixel: i32, n: i32) -> R<()> {
        if n < 0 || pixel == 0 {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        let (dn, db) = self.get_ec(s2)?;
        let (sn, sb) = self.get_ec(s1)?;
        let src = self.screens.get(sn).unwrap();
        let (sw, sh, sp) = (src.width as i32, src.height as i32, src.planes);
        let dst = self.screens.get(dn).unwrap();
        let (dw, dh, dp) = (dst.width as i32, dst.height as i32, dst.planes);
        let planes = sp.min(dp);
        let total = (sw as u64 / 8) * sh as u64 * 8;
        let count = if n == 0 { total } else { n as u64 };
        let mut step = pixel as u32 as u64;
        if count > 0 {
            step %= count;
        }
        let kind = it.wait_state(|| WaitKind::Other(APPEAR_WAIT, count << 24));
        let WaitKind::Other(_, st) = kind else {
            unreachable!()
        };
        let mut pos = *st & 0xFF_FFFF;
        let remaining = *st >> 24;
        // About 300 cycles per pixel plus 40 per plane at 7.09 MHz.
        let budget = (141_000 / (300 + 40 * planes as u64)).max(1);
        let todo = remaining.min(budget);
        let data = src.bitmaps[sb].clone();
        let dst = self.screens.get_mut(dn).unwrap();
        blocks::appear_step(
            &data,
            sw,
            dst.bitmap_mut(db),
            dw,
            dh,
            planes,
            &mut pos,
            step,
            total,
            todo,
        );
        let left = remaining - todo;
        if left == 0 {
            it.clear_wait();
            return Ok(());
        }
        if let WaitKind::Other(_, st) = it.wait_state(|| WaitKind::Other(APPEAR_WAIT, 0)) {
            *st = (left << 24) | pos;
        }
        Err(Exc::Block)
    }

    /// `Load Iff` from file data: into the current screen, or into screen
    /// `screen` opened with the picture's size and mode.
    pub(crate) fn load_iff_bytes(
        &mut self,
        it: &mut Interp,
        data: &[u8],
        screen: Option<i32>,
    ) -> R<()> {
        let mut il = iff::parse(data).map_err(Exc::Error)?;
        // Mask Iff: chunk bits BMHD 0, CAMG 1, CMAP 2, CCRT 3, BODY 4, AMSC 5.
        let off = self.with_draw(|g| g.iff_mask_off);
        if off & 1 != 0 {
            il.bmhd = None;
        }
        if off & 2 != 0 {
            il.camg = None;
        }
        if off & 4 != 0 {
            il.cmap = None;
        }
        if off & 8 != 0 {
            il.ccrt = None;
        }
        if off & 16 != 0 {
            il.body = None;
        }
        if off & 32 != 0 {
            il.amsc = None;
        }
        if il.body.is_none() {
            return Ok(());
        }
        if let Some(n) = screen {
            let Some((w, h, _planes, colours, mode)) = iff::screen_params(&il) else {
                return err(iff::BAD_IFF_FORMAT);
            };
            if !(0..8).contains(&n) {
                return err(errors::VALID_SCREEN_NUMBERS);
            }
            self.screens
                .open(n as usize, w as i32, h as i32, colours, mode)
                .map_err(|e| Exc::Error(lib_error(e)))?;
            // IffCentre: the AMSC chunk restores display and offset.
            if let Some(m) = il.amsc {
                let s = self.draw_screen()?;
                s.pending_display = [
                    Some(m.awx as i32),
                    Some(m.awy as i32),
                    Some(m.awtx as i32),
                    Some(m.awty as i32),
                ];
                s.pending_offset = [Some(m.avx as i32), Some(m.avy as i32)];
                s.hidden = m.flags & 0x8000 != 0;
            }
        }
        let default_pal = self.screens.default_palette;
        self.draw_screen()?.palette = iff::palette(&il, &default_pal);
        // IffShift: a CCRT chunk starts a colour shift.
        if let Some(cc) = il.ccrt
            && cc.direction != 0
            && cc.start >= 0
            && cc.end >= 0
            && cc.start < cc.end
        {
            let speed = (cc.micros / 20).min(0xFFFF) as i32;
            if speed != 0 {
                self.screens
                    .start_shift(
                        speed,
                        cc.start as i32,
                        cc.end as i32,
                        cc.direction > 0,
                        true,
                    )
                    .map_err(|e| Exc::Error(lib_error(e)))?;
            }
        }
        let s = self.draw_screen()?;
        let Some(b) = il.bmhd else {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        };
        if b.w as u32 > s.width || b.planes > s.planes {
            return err(iff::CANT_FIT_PICTURE);
        }
        let (w, h) = (s.width as usize, s.height as usize);
        iff::decode_body(&il, s.logic_mut(), w, h).map_err(Exc::Error)?;
        // The text cursor is switched off (`ChCuOff`).
        let _ = self.print(it, b"\x1bC0");
        Ok(())
    }

    /// `Save Iff` of the current screen (`comp` 0 = raw, 1-2 = ByteRun1).
    pub(crate) fn save_iff_bytes(&mut self, comp: i32) -> R<Vec<u8>> {
        let s = self.draw_screen()?;
        if !(0..3).contains(&comp) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        let ss = iff::SaveScreen {
            buf: s.logic_ref(),
            width: s.width as usize,
            height: s.height as usize,
            planes: s.planes as usize,
            hires: s.hires,
            lace: s.lace,
            ham: s.ham,
            nb_col: if s.ham { 64 } else { s.colours },
            palette: s.palette,
            wtx: s.display_w as u16,
            wty: s.display_h as u16,
            amsc: iff::Amsc {
                awx: s.display_x as i16,
                awy: s.display_y as i16,
                awtx: s.display_w as i16,
                awty: (s.display_h * if s.lace { 2 } else { 1 }) as i16,
                avx: s.offset_x as i16,
                avy: s.offset_y as i16,
                flags: if s.hidden { 0x8000 } else { 0 },
            },
        };
        Ok(iff::save(&ss, comp as u8))
    }

    /// Compact extension (slot 2): Pack, Spack, Unpack.
    fn compact_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        match kw.token {
            tk::COMPACT_PACK | tk::COMPACT_PACK_2 | tk::COMPACT_SPACK | tk::COMPACT_SPACK_2 => {
                let a = it.inst_args(self, kw)?;
                let screen_header = matches!(kw.token, tk::COMPACT_SPACK | tk::COMPACT_SPACK_2);
                let (x1, y1) = (a.opt(2).unwrap_or(0), a.opt(3).unwrap_or(0));
                let (x2, y2) = (a.opt(4).unwrap_or(10000), a.opt(5).unwrap_or(10000));
                self.pack_screen(a.int(0), a.int(1), x1, y1, x2, y2, screen_header)?;
            }
            tk::COMPACT_UNPACK | tk::COMPACT_UNPACK_3 => {
                let a = it.inst_args(self, kw)?;
                if self.screens.current.is_none() {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let data = self.bank_data(a.int(0))?;
                let x = a.opt(1).map_or(-1, |v| v.max(-1));
                let y = a.opt(2).map_or(-1, |v| v.max(-1));
                let s = self.draw_screen()?;
                let (w, h, p) = (s.width as usize, s.height as usize, s.planes as usize);
                for bi in s.autoback_targets() {
                    if pack::unpack_bitmap(&data, s.bitmap_mut(bi), w, h, p, x, y).is_none() {
                        return no_pac();
                    }
                }
            }
            tk::COMPACT_UNPACK_2 => {
                let a = it.inst_args(self, kw)?;
                let data = self.bank_data(a.int(0))?;
                self.unpack_screen(it, &data, a.int(1))?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Unpack bank To screen` (`UnPack_Screen`): opens the screen saved by
    /// Spack and unpacks the picture into it.
    pub(crate) fn unpack_screen(&mut self, it: &mut Interp, data: &[u8], n: i32) -> R<()> {
        let Some(h) = pack::PackedScreen::read(data) else {
            return no_pac();
        };
        if !(0..8).contains(&n) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        let colours = if h.con0 & 0x800 != 0 {
            4096
        } else {
            h.nb_col as u32
        };
        let mode = (h.con0 & 0x8004) as u32;
        self.screens
            .open(n as usize, h.tx as i32, h.ty as i32, colours, mode)
            .map_err(|_| Exc::Error(errors::OUT_OF_MEMORY))?;
        let _ = self.print(it, b"\x1bC0");
        let s = self.draw_screen()?;
        s.palette = h.palette;
        s.pending_display = [
            Some(h.awx as i32),
            Some(h.awy as i32),
            Some(h.awtx as i32),
            Some(h.awty as i32),
        ];
        s.pending_offset = [Some(h.avx as i32), Some(h.avy as i32)];
        let (w, hh, p) = (s.width as usize, s.height as usize, s.planes as usize);
        if pack::unpack_bitmap(data, s.logic_mut(), w, hh, p, 0, 0).is_none() {
            return no_pac();
        }
        Ok(())
    }

    /// `Pack`/`Spack screen To bank[,x1,y1,x2,y2]` (`PacPar`, `Pack`).
    #[allow(clippy::too_many_arguments)]
    fn pack_screen(
        &mut self,
        screen: i32,
        bank: i32,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        header: bool,
    ) -> R<()> {
        let (sn, bi) = self.get_ec(screen)?;
        let s = self.screens.get(sn).unwrap();
        let tl = (s.width / 8) as i32;
        let x1b = ((x1 as u16) >> 3) as i32;
        let x2b = (((x2 as u16) >> 3) as i32).min(tl);
        let y2 = y2.min(s.height as i32);
        let (tx, ty) = (x2b - x1b, y2 - y1);
        if tx <= 0 || ty <= 0 || y1 < 0 {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        if !(1..65536).contains(&bank) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        let view = pack::PlanarView {
            buf: &s.bitmaps[bi],
            width: s.width as usize,
            height: s.height as usize,
            planes: s.planes as usize,
        };
        let bm = pack::pack_bitmap(&view, x1b as usize, y1 as usize, tx as usize, ty as usize);
        let mut out = Vec::with_capacity(bm.len() + pack::PS_LONG);
        if header {
            let mut hdr = vec![0u8; pack::PS_LONG];
            let con0 = (if s.hires { 0x8000 } else { 0 })
                | ((s.planes as u16) << 12)
                | (if s.ham { 0x800 } else { 0 })
                | (if s.lace { 4 } else { 0 });
            pack::PackedScreen {
                tx: s.width as u16,
                ty: s.height as u16,
                awx: s.display_x as i16,
                awy: s.display_y as i16,
                awtx: s.display_w as i16,
                awty: (s.display_h * if s.lace { 2 } else { 1 }) as i16,
                avx: s.offset_x as i16,
                avy: s.offset_y as i16,
                con0,
                nb_col: if s.ham { 64 } else { s.colours as u16 },
                nplan: s.planes as u16,
                palette: s.palette,
            }
            .write(&mut hdr);
            out.extend_from_slice(&hdr);
        }
        out.extend_from_slice(&bm);
        self.banks.insert(Bank {
            number: bank as u16,
            name: "Pac.Pic.".into(),
            chip: false,
            data_bank: true,
            data: BankData::Raw(out),
        });
        self.on_banks_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Machine;
    use crate::interp::RunState;

    /// Runs a program until it stops; returns the machine and the log.
    fn run(src: &str) -> Machine {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).expect("tokenise");
        let mut m = Machine::new();
        m.run_program(&prg).expect("verify");
        for _ in 0..2000 {
            m.vbl();
            if matches!(m.state, RunState::Stopped(_)) {
                break;
            }
        }
        m
    }

    fn px(m: &Machine, n: usize, x: i32, y: i32) -> u8 {
        m.hw.screens.get(n).unwrap().pixel(x, y).unwrap()
    }

    /// Background colour (the paper of the default window).
    fn bg(m: &Machine) -> u8 {
        px(m, 0, 300, 200)
    }

    fn stopped_ok(m: &Machine) {
        assert_eq!(
            m.hw.log.last().map(String::as_str),
            Some("End"),
            "log: {:?}",
            m.hw.log
        );
    }

    #[test]
    fn plot_draw_box_bar() {
        let m = run(
            "Ink 3 : Plot 10,10\nDraw 0,20 To 9,20\nInk 4 : Box 30,30 To 40,40\nInk 5 : Bar 50,50 To 52,52\nPlot 100,100,6\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 0, 10, 10), 3);
        assert_eq!(px(&m, 0, 0, 20), 3);
        assert_eq!(px(&m, 0, 9, 20), 3);
        assert_eq!(px(&m, 0, 10, 20), bg(&m));
        assert_eq!(px(&m, 0, 30, 35), 4);
        assert_eq!(px(&m, 0, 35, 35), bg(&m));
        assert_eq!(px(&m, 0, 52, 52), 5);
        assert_eq!(px(&m, 0, 53, 52), bg(&m));
        assert_eq!(px(&m, 0, 100, 100), 6);
    }

    #[test]
    fn point_xgr_clip_paint() {
        let m = run(
            "Ink 2 : Box 0,0 To 20,20 : Ink 7 : Paint 5,5\nA=Point(5,5) : X=Xgr : Y=Ygr\nClip 100,100 To 110,110 : Ink 1 : Bar 90,90 To 120,120\nIf A<>7 Then Error 23\nIf X<>5 or Y<>5 Then Error 24\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 0, 10, 10), 7);
        assert_eq!(px(&m, 0, 99, 100), bg(&m));
        assert_eq!(px(&m, 0, 100, 100), 1);
        assert_eq!(px(&m, 0, 109, 109), 1);
        assert_eq!(px(&m, 0, 110, 110), bg(&m));
    }

    #[test]
    fn polygon_circle_text() {
        let m = run(
            "Ink 3 : Polygon 0,0 To 40,0 To 0,40\nInk 4 : Circle 100,100,10\nInk 5,0 : Text 200,100,\"Hi\"\nL=Text Length(\"Hello\") : If L<>40 Then Error 23\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 0, 5, 5), 3);
        assert_eq!(px(&m, 0, 110, 100), 4);
        let s = m.hw.screens.get(0).unwrap();
        let n = (94..102)
            .flat_map(|y| (200..216).map(move |x| (x, y)))
            .filter(|&(x, y)| s.pixel(x, y) == Some(5))
            .count();
        assert!(n > 10);
    }

    #[test]
    fn blocks_and_cblocks() {
        let m = run(
            "Ink 3 : Bar 0,0 To 15,7\nGet Block 1,0,0,16,8\nPut Block 1,32,0\nGet Cblock 2,0,0,16,8\nPut Cblock 2,64,0\nHrev Block 1 : Del Block 1 : Del Cblock\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 0, 40, 4), 3);
        assert_eq!(px(&m, 0, 70, 4), 3);
        assert_eq!(px(&m, 0, 90, 4), bg(&m));
    }

    #[test]
    fn block_errors() {
        let m = run("Put Block 5\n");
        assert!(
            m.hw.log.iter().any(|l| l.contains("Block not defined")),
            "{:?}",
            m.hw.log
        );
    }

    #[test]
    fn screen_copy_scroll_zoom() {
        let m = run(
            "Ink 2 : Bar 0,0 To 9,9\nScreen Copy 0,0,0,10,10 To 0,20,0\nDef Scroll 1,0,0 To 10,10,0,30 : Scroll 1\nZoom 0,0,0,10,10 To 0,100,100,120,120\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 0, 25, 5), 2);
        assert_eq!(px(&m, 0, 5, 35), 2);
        assert_eq!(px(&m, 0, 119, 119), 2);
        assert_eq!(px(&m, 0, 121, 121), bg(&m));
    }

    #[test]
    fn pack_unpack() {
        let m = run(
            "Ink 5 : Bar 10,10 To 50,50\nSpack 0 To 10\nPack 0 To 11,0,0,64,64\nInk 0 : Bar 0,0 To 100,100\nUnpack 11\nUnpack 10 To 1\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 0, 20, 20), 5);
        assert_eq!(px(&m, 1, 20, 20), 5);
        assert_eq!(m.hw.banks.get(10).unwrap().name, "Pac.Pic.");
    }

    #[test]
    fn iff_roundtrip() {
        let mut m = Machine::new();
        {
            let s = m.hw.screens.current_mut().unwrap();
            let (w, h) = (s.width, s.height);
            let mut gr = s.gr.clone();
            gr.ink = 9;
            let mut c = crate::gfx::draw::Canvas::new(&mut s.bitmaps[0], w, h, 4);
            gr.bar(&mut c, 5, 5, 30, 30);
        }
        let data = m.hw.save_iff_bytes(1).unwrap();
        let prg = crate::tokenise::tokenise_program(b"Rem\n").unwrap();
        m.run_program(&prg).unwrap();
        let mut it = std::mem::take(&mut m.interp);
        m.hw.load_iff_bytes(&mut it, &data, Some(2)).unwrap();
        assert_eq!(m.hw.screens.get(2).unwrap().pixel(10, 10), Some(9));
    }

    #[test]
    fn appear_finishes() {
        let m = run(
            "Ink 3 : Bar 0,0 To 319,255\nSpack 0 To 10 : Unpack 10 To 1\nInk 0 : Bar 0,0 To 319,255\nT=Timer : Appear 0 To 1,17 : T=Timer-T\nIf T<50 Then Error 23\n",
        );
        stopped_ok(&m);
        assert_eq!(px(&m, 1, 100, 100), 3);
        assert_eq!(px(&m, 1, 0, 0), 3);
        assert_eq!(px(&m, 1, 319, 255), 3);
    }

    /// Debug helper: `AMOS_DRAW_PRG=file.txt|file.AMOS AMOS_DRAW_OUT=out.ppm
    /// cargo test dump_screen -- --ignored` runs a program and writes the
    /// current screen (palette applied) as a PPM picture.
    #[test]
    #[ignore]
    fn dump_screen() {
        let Ok(path) = std::env::var("AMOS_DRAW_PRG") else {
            return;
        };
        let out = std::env::var("AMOS_DRAW_OUT").unwrap_or("screen.ppm".into());
        let frames: usize = std::env::var("AMOS_DRAW_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let data = std::fs::read(&path).unwrap();
        let mut prg = if path.to_ascii_uppercase().ends_with(".AMOS") {
            crate::program::Program::load(&data).unwrap()
        } else {
            crate::tokenise::tokenise_program(&data).unwrap()
        };
        // AMOS_DRAW_BANKS=prog.AMOS: use the banks of another program.
        if let Ok(b) = std::env::var("AMOS_DRAW_BANKS") {
            prg.banks = crate::program::Program::load(&std::fs::read(b).unwrap())
                .unwrap()
                .banks;
        }
        let mut m = Machine::new();
        if let Some(dir) = std::path::Path::new(&path).parent() {
            m.hw.files.set_native_root(dir);
        }
        m.run_program(&prg).unwrap();
        for _ in 0..frames {
            m.vbl();
            if matches!(m.state, RunState::Stopped(_)) {
                break;
            }
        }
        eprintln!("log: {:?}", m.hw.log);
        let s = m.hw.screens.current().unwrap();
        let mut ppm = format!("P6 {} {} 255\n", s.width, s.height).into_bytes();
        for &p in s.logic_ref() {
            let c = s.palette[(p & 31) as usize];
            let c = if p >= 32 && !s.ham {
                (c >> 1) & 0x777
            } else {
                c
            };
            for sh in [8, 4, 0] {
                ppm.push((((c >> sh) & 15) * 17) as u8);
            }
        }
        std::fs::write(out, ppm).unwrap();
    }
}
