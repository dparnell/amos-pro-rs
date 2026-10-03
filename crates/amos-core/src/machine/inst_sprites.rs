//! Hardware sprites, bobs, AMAL, collisions (`Sprite.s` part of `+Lib.s`
//! 11400-12900, library routines in `+W.s`).

use std::rc::Rc;

use super::Hardware;
use crate::banks::{BankData, Image};
use crate::errors;
use crate::gfx::amal::{AmalHost, ChanKind, Conv, Field, Target};
use crate::gfx::bobs::{self, BOB_MAX, Bob, Limits, Src};
use crate::gfx::images::{self, FLIP_MASK, MaskState};
use crate::gfx::sprites::{DisplaySprite, HsShown, SPRITE_MAX, hash_bytes, to_rgba};
use crate::interp::value::Value;
use crate::interp::{Interp, R, err};
use crate::tokens::{Keyword, TokenKind, tk};

const E_FONCALL: u16 = errors::ILLEGAL_FUNCTION_CALL;
const E_SCREEN: u16 = errors::SCREEN_NOT_OPENED;
const E_BANK: u16 = errors::BANK_NOT_RESERVED;
const E_BOB: u16 = errors::BOB_NOT_DEFINED;
/// `AdBErr` (`EcEBase+30-1`): image number beyond the bank.
const E_IMAGE: u16 = 74;
/// AMAL compilation errors are `106 + code` (`SpEBase+2`).
const E_AMAL_BASE: u16 = 106;

/// One 16 pixel wide column of a displayed hardware sprite.
#[derive(Clone, Debug)]
pub struct Slice {
    /// Display order key and texture id.
    pub id: u32,
    /// Hardware channel (0-7): front channels are in front.
    pub channel: u8,
    /// Top left in hardware coordinates.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Colour index 16-31, or 0 for transparent.
    pub pixels: Vec<u8>,
}

/// Images of the sprite bank (1) or icon bank (2).
fn image_bank_of(banks: &crate::banks::BankSet, icons: bool) -> Option<&Vec<Image>> {
    let b = banks.banks.get(&if icons { 2 } else { 1 })?;
    if b.is_icons() != icons {
        return None;
    }
    match &b.data {
        BankData::Images { images, .. } => Some(images),
        _ => None,
    }
}

/// Builds the pixels of one 16 pixel column of a sprite image: 4 colour
/// sprites use the colours of their channel pair, 16 colour (attached)
/// sprites colours 16-31.
fn sprite_column(img: &Image, word: u32, channel: u8, attached: bool) -> Vec<u8> {
    let h = img.height as u32;
    let mut out = vec![0u8; (16 * h) as usize];
    let base = 16 + 4 * (channel as u32 / 2);
    for y in 0..h {
        for x in 0..16 {
            let c = img.pixel(word * 16 + x, y) as u32;
            let v = if attached {
                if c & 15 != 0 { 16 + (c & 15) } else { 0 }
            } else if c & 3 != 0 {
                base + (c & 3)
            } else {
                0
            };
            out[(y * 16 + x) as usize] = v as u8;
        }
    }
    out
}

impl Hardware {
    // ------------------------------------------------------------------
    // Banks
    // ------------------------------------------------------------------

    /// Images of the sprite bank (1) or icon bank (2).
    pub(crate) fn image_bank(&self, icons: bool) -> Option<&Vec<Image>> {
        image_bank_of(&self.banks, icons)
    }

    fn image_bank_mut(&mut self, icons: bool) -> Option<&mut Vec<Image>> {
        let b = self.banks.banks.get_mut(&if icons { 2 } else { 1 })?;
        if b.is_icons() != icons {
            return None;
        }
        match &mut b.data {
            BankData::Images { images, .. } => Some(images),
            _ => None,
        }
    }

    fn bank_palette(&self, icons: bool) -> Option<[u16; 32]> {
        let b = self.banks.banks.get(&if icons { 2 } else { 1 })?;
        match &b.data {
            BankData::Images { palette, .. } if b.is_icons() == icons => Some(*palette),
            _ => None,
        }
    }

    /// A non empty image of the sprite bank (1-based, flags ignored).
    pub(crate) fn sprite_image(&self, n: u16) -> Option<&Image> {
        let n = bobs::index(n) as usize;
        let img = self.image_bank(false)?.get(n.checked_sub(1)?)?;
        if img.is_empty() { None } else { Some(img) }
    }

    /// `AdBob` / `AdIcon`: checks an image number, returns it without flags.
    fn ad_image(&self, icons: bool, n: i32) -> R<u16> {
        let n = n & 0x3FFF;
        if n == 0 {
            return err(E_FONCALL);
        }
        let count = self.image_bank(icons).map_or(0, |b| b.len());
        if count == 0 {
            return err(E_BANK);
        }
        if n as usize > count {
            return err(E_IMAGE);
        }
        Ok(n as u16)
    }

    // ------------------------------------------------------------------
    // Reset and per frame work
    // ------------------------------------------------------------------

    pub(crate) fn sprites_reset(&mut self) {
        self.sprites = crate::gfx::sprites::SpriteState::default();
    }

    /// AMAL and sprite updates done by the VBL interrupt.
    pub(crate) fn sprites_vbl(&mut self) {
        if !self.sprites.amal.sync_off {
            self.amal_tick();
        }
    }

    fn amal_tick(&mut self) {
        let mut amal = std::mem::take(&mut self.sprites.amal);
        amal.tick(self);
        self.sprites.amal = amal;
    }

    /// Bob redraw and screen swap done at interpreter test points
    /// (`Test_Force`, +ILib.s:970-1000).
    pub(crate) fn sprites_test_point(&mut self, it: &mut Interp) -> R<()> {
        let _ = it;
        let s = &self.sprites;
        if !s.dirty_bobs && !s.dirty_sprites {
            return Ok(());
        }
        if self.vbl_count.wrapping_sub(s.last_update) < s.update_every as u64 {
            return Ok(());
        }
        self.sprites.last_update = self.vbl_count;
        if std::mem::take(&mut self.sprites.dirty_bobs) && self.sprites.update_bobs {
            self.bob_update();
        }
        if std::mem::take(&mut self.sprites.dirty_sprites) && self.sprites.update_sprites {
            self.sprites_act();
        }
        Ok(())
    }

    /// `ActHs`: applies the requested sprite positions.
    pub(crate) fn sprites_act(&mut self) {
        let count = self.image_bank(false).map_or(0, |b| b.len());
        if count == 0 {
            return;
        }
        for n in 0..SPRITE_MAX {
            let a = self.sprites.act[n];
            if a.flag == 0 {
                continue;
            }
            if a.flag & 0x80 != 0 {
                self.sprites.act[n].flag = 0;
                self.sprites.act[n].image = 0;
                self.sprites.shown[n] = HsShown::default();
                continue;
            }
            self.sprites.act[n].flag = 0;
            let i = (a.image as u16) & 0x3FFF;
            let Some(img) = self.sprite_image(i) else {
                continue;
            };
            // Sprite 0 is the mouse while it is shown; too tall sprites are
            // ignored (HsSet).
            if n == 0 && self.sprites.mouse_show >= 0
                || img.height as u32 + 1 >= self.sprites.buffer_lines as u32
            {
                continue;
            }
            self.sprites.shown[n] = HsShown {
                active: true,
                x: a.x,
                y: a.y,
                image: i,
            };
        }
    }

    /// Lays out the displayed sprites on the 8 hardware channels (`HsAff`,
    /// +W.s:11663-11944): direct sprites 0-7 take their channels, computed
    /// sprites 8-63 share the free channels, sorted by Y.
    pub fn sprite_slices(&self) -> Vec<Slice> {
        let mut out = Vec::new();
        let pmax = self.sprites.buffer_lines as i32;
        // Free columns: Some((first free line, lines used)).
        let mut cols: [Option<(i32, i32)>; 8] = [None; 8];
        let mut c = 0usize;
        if self.sprites.mouse_show >= 0 {
            let img = &self.sprites.mouse_image;
            let x = self.input.mouse_x - images::hot_x(img);
            let y = self.input.mouse_y - images::hot_y(img);
            out.push(Slice {
                id: 0x8000,
                channel: 0,
                x,
                y,
                w: 16,
                h: img.height as u32,
                pixels: sprite_column(img, 0, 0, false),
            });
            c = 1;
        }
        while c < 8 {
            let s = self.sprites.shown[c];
            let img = if s.active {
                self.sprite_image(s.image)
            } else {
                None
            };
            let Some(img) = img else {
                cols[c] = Some((0, 0));
                c += 1;
                continue;
            };
            let x = (s.x as i32 - images::hot_x(img)).max(0);
            let y = (s.y as i32 - images::hot_y(img)).max(0);
            let h = img.height as u32;
            if img.planes < 4 {
                for word in 0..img.width_words as u32 {
                    if c >= 8 {
                        break;
                    }
                    out.push(Slice {
                        id: 0x100 + (c as u32) * 16 + word,
                        channel: c as u8,
                        x: x + 16 * word as i32,
                        y,
                        w: 16,
                        h,
                        pixels: sprite_column(img, word, c as u8, false),
                    });
                    c += 1;
                }
            } else {
                if c & 1 == 1 {
                    c += 1;
                }
                for word in 0..img.width_words as u32 {
                    if c + 1 >= 8 {
                        break;
                    }
                    out.push(Slice {
                        id: 0x100 + (c as u32) * 16 + word,
                        channel: c as u8,
                        x: x + 16 * word as i32,
                        y,
                        w: 16,
                        h,
                        pixels: sprite_column(img, word, c as u8, true),
                    });
                    c += 2;
                }
            }
        }
        if cols.iter().all(Option::is_none) {
            return out;
        }
        // Computed sprites, sorted by top line then number.
        let mut list: Vec<(i32, usize)> = Vec::new();
        for n in 8..SPRITE_MAX {
            let s = self.sprites.shown[n];
            if !s.active {
                continue;
            }
            if let Some(img) = self.sprite_image(s.image) {
                list.push(((s.y as i32 - images::hot_y(img)).max(0), n));
            }
        }
        list.sort();
        let next_free = |mut c: usize, cols: &[Option<(i32, i32)>; 8]| {
            for _ in 0..16 {
                c = (c + 1) % 8;
                if cols[c].is_some() {
                    break;
                }
            }
            c
        };
        let mut cur = if cols[0].is_some() {
            0
        } else {
            next_free(0, &cols)
        };
        for (yr, n) in list {
            let s = self.sprites.shown[n];
            let Some(img) = self.sprite_image(s.image) else {
                continue;
            };
            let x = (s.x as i32 - images::hot_x(img)).max(0);
            let h1 = img.height as i32 + 1;
            let words = img.width_words as u32;
            let fits = |col: Option<(i32, i32)>| {
                col.is_some_and(|(yact, pact)| yr >= yact && pact + h1 < pmax)
            };
            if img.planes < 4 {
                let mut tries = 8;
                let mut word = 0u32;
                loop {
                    if fits(cols[cur]) {
                        let (_, pact) = cols[cur].unwrap();
                        cols[cur] = Some((yr + h1, pact + h1));
                        out.push(Slice {
                            id: 0x1000 + (n as u32) * 16 + word,
                            channel: cur as u8,
                            x: x + 16 * word as i32,
                            y: yr,
                            w: 16,
                            h: img.height as u32,
                            pixels: sprite_column(img, word, cur as u8, false),
                        });
                        word += 1;
                        tries = 8;
                        if word == words {
                            cur = next_free(cur, &cols);
                            break;
                        }
                    } else {
                        tries -= 1;
                        if tries == 0 {
                            if cols[cur].is_none() {
                                cur = next_free(cur, &cols);
                            }
                            break;
                        }
                    }
                    // Next column; unavailable columns count as failed tries.
                    loop {
                        cur = (cur + 1) % 8;
                        if cols[cur].is_some() {
                            break;
                        }
                        tries -= 1;
                        if tries == 0 {
                            break;
                        }
                    }
                    if tries == 0 {
                        break;
                    }
                }
            } else {
                let mut tries = 4;
                let mut word = 0u32;
                if cur & 1 == 1 {
                    cur = (cur + 1) % 8;
                }
                loop {
                    let pair_ok = cols[cur].is_some() && cur + 1 < 8 && cols[cur + 1].is_some();
                    if pair_ok && fits(cols[cur]) && fits(cols[cur + 1]) {
                        let (_, p0) = cols[cur].unwrap();
                        let (_, p1) = cols[cur + 1].unwrap();
                        cols[cur] = Some((yr + h1, p0 + h1));
                        cols[cur + 1] = Some((yr + h1, p1 + h1));
                        out.push(Slice {
                            id: 0x1000 + (n as u32) * 16 + word,
                            channel: cur as u8,
                            x: x + 16 * word as i32,
                            y: yr,
                            w: 16,
                            h: img.height as u32,
                            pixels: sprite_column(img, word, cur as u8, true),
                        });
                        word += 1;
                        if word == words {
                            cur = next_free((cur + 1) % 8, &cols);
                            break;
                        }
                        tries = 4;
                        continue;
                    }
                    if pair_ok || cols[cur].is_none() {
                        tries -= 1;
                        if tries == 0 {
                            break;
                        }
                    }
                    cur = (cur + 2) % 8;
                }
            }
        }
        out
    }

    /// Palette used by sprites on a raster line: the colours of the screen
    /// displayed on that line (front screen first). Lines below a screen
    /// keep its colours; lines above every screen use the front screen.
    fn line_palette(&self, line: i32) -> [u16; 32] {
        let mut best: Option<(i32, [u16; 32])> = None;
        for &n in &self.screens.priority {
            let Some(s) = self.screens.get(n) else {
                continue;
            };
            if s.hidden || s.dual_slave {
                continue;
            }
            if line >= s.display_y && line < s.display_y + s.display_h as i32 {
                return s.palette;
            }
            if s.display_y <= line && best.is_none_or(|(y, _)| s.display_y > y) {
                best = Some((s.display_y, s.palette));
            }
        }
        if let Some((_, p)) = best {
            return p;
        }
        self.screens
            .priority
            .first()
            .and_then(|&n| self.screens.get(n))
            .map_or(crate::gfx::screen::DEFAULT_PALETTE, |s| s.palette)
    }

    /// Refreshes `self.sprites.display` (hardware sprites and mouse pointer)
    /// before a frame is built. Colours come from the palette of the screen
    /// displayed on each line (an approximation of the copper colour
    /// changes).
    pub(crate) fn sprites_prepare_frame(&mut self) {
        let mut slices = self.sprite_slices();
        slices.sort_by_key(|s| (s.channel, s.id));
        let mut display = Vec::with_capacity(slices.len());
        for s in slices {
            let rgba = to_rgba(&s.pixels, s.w, s.h, |row| {
                self.line_palette(s.y + row as i32)
            });
            let version = hash_bytes(&rgba, s.w as u64);
            display.push(DisplaySprite {
                id: s.id,
                version,
                width: s.w,
                height: s.h,
                rgba,
                x: crate::display::hw_x_to_display(s.x),
                y: crate::display::hw_y_to_display(s.y),
                scale_x: 2,
                scale_y: 2,
            });
        }
        self.sprites.display = display;
    }

    // ------------------------------------------------------------------
    // Bobs
    // ------------------------------------------------------------------

    /// Bobs of closed screens disappear (`BbEcOff`).
    fn bobs_prune(&mut self) {
        let gone: Vec<u16> = self
            .sprites
            .bobs
            .list
            .values()
            .filter(|b| self.screens.get(b.screen).is_none())
            .map(|b| b.number)
            .collect();
        for n in gone {
            self.sprites.bobs.list.remove(&n);
            self.sprites.amal.remove_target(Target::Bob(n));
        }
    }

    /// `EffBob`: restores the backgrounds saved two updates ago.
    pub(crate) fn bob_eff(&mut self) {
        self.bobs_prune();
        let screens = &mut self.screens;
        for bob in self.sprites.bobs.list.values_mut() {
            if bob.ecpt != 0 {
                bob.ecpt -= 1;
                continue;
            }
            let slot = &bob.slots[bob.dcur2];
            if !slot.valid {
                continue;
            }
            let Some(s) = screens.get_mut(bob.screen) else {
                continue;
            };
            let (w, h, planes) = (s.width, s.height, s.planes);
            let mut t = bobs::Target {
                pixels: s.logic_mut(),
                width: w,
                height: h,
                planes,
            };
            if bob.eff > 0 {
                bobs::restore(&mut t, slot, Some((bob.eff - 1) as u8));
            } else if slot.data.is_some() {
                bobs::restore(&mut t, slot, None);
            }
        }
    }

    /// `BobAct`: flips the save slots, recomputes changed bobs and builds
    /// the draw order.
    pub(crate) fn bob_act(&mut self) {
        self.bobs_prune();
        let Some(bank) = image_bank_of(&self.banks, false) else {
            self.sprites.bobs.order.clear();
            return;
        };
        let count = bank.len();
        let mut order = Vec::new();
        let mut prio = Vec::new();
        let mut deleted = Vec::new();
        let mut made = Vec::new();
        let priority = self.sprites.bobs.priority;
        for bob in self.sprites.bobs.list.values_mut() {
            std::mem::swap(&mut bob.dcur1, &mut bob.dcur2);
            if bob.ecpt == 0 && bob.act != 0 {
                if bob.act < 0 {
                    if bob.decor >= 2 {
                        bob.decor -= 1;
                    } else {
                        deleted.push(bob.number);
                    }
                    continue;
                }
                bob.act = 0;
                let flags = bob.image & FLIP_MASK;
                let idx = bob.image & !FLIP_MASK;
                if idx == 0 || idx as usize > count {
                    continue;
                }
                let img = &bank[idx as usize - 1];
                if img.is_empty() {
                    continue;
                }
                let mut mask = self.sprites.masks[0]
                    .get(idx as usize)
                    .copied()
                    .unwrap_or_default();
                if mask == MaskState::Lazy {
                    mask = MaskState::Made;
                    made.push(idx);
                }
                let Some(s) = self.screens.get(bob.screen) else {
                    continue;
                };
                let (hx, hy) = images::flipped_hot(img, flags);
                bob.calc = bobs::calc(
                    img,
                    idx,
                    flags,
                    bob.x as i32 - hx,
                    bob.y as i32 - hy,
                    bob.lim,
                    (s.width, s.height, s.planes),
                    bob.minterm,
                    mask,
                );
                if bob.calc.is_none() {
                    continue;
                }
            }
            if bob.decor != 0 {
                let Some(c) = &bob.calc else { continue };
                bob.dcpt = bob.decor;
                let pm = (bob.planes as u32 & ((1u32 << c.nplanes) - 1)) as u8;
                let slot = &mut bob.slots[bob.dcur1];
                slot.valid = true;
                slot.rx = c.rx;
                slot.ry = c.ry;
                slot.rw = c.rw;
                slot.rh = c.rh;
                slot.planes = pm;
            }
            if Some(bob.screen) == priority {
                prio.push((bob.y, bob.x, bob.number));
            } else {
                order.push(bob.number);
            }
        }
        for n in made {
            self.sprites.set_mask(false, n, MaskState::Made);
        }
        for n in deleted {
            self.sprites.bobs.list.remove(&n);
            self.sprites.amal.remove_target(Target::Bob(n));
        }
        // Priority On: sorted by Y then X, drawn after the others.
        prio.sort_by_key(|&(y, x, _)| (y, x));
        order.extend(prio.into_iter().map(|(_, _, n)| n));
        if self.sprites.bobs.reverse {
            order.reverse();
        }
        self.sprites.bobs.order = order;
    }

    /// `BobAff`: saves the backgrounds, then draws the bobs.
    pub(crate) fn bob_aff(&mut self) {
        let screens = &mut self.screens;
        for bob in self.sprites.bobs.list.values_mut() {
            if bob.dcpt == 0 || bob.eff != 0 {
                continue;
            }
            let Some(s) = screens.get_mut(bob.screen) else {
                continue;
            };
            let slot = &mut bob.slots[bob.dcur1];
            if !slot.valid {
                continue;
            }
            bob.dcpt -= 1;
            let (w, h, planes) = (s.width, s.height, s.planes);
            let t = bobs::Target {
                pixels: &mut s.bitmaps[s.logic],
                width: w,
                height: h,
                planes,
            };
            bobs::save(&t, slot);
        }
        let order = self.sprites.bobs.order.clone();
        for n in order {
            let Some(bob) = self.sprites.bobs.list.get(&n) else {
                continue;
            };
            let Some(c) = bob.calc.clone() else { continue };
            let (screen, planes) = (bob.screen, bob.planes);
            let Some(img) = self.sprite_image(c.image) else {
                continue;
            };
            let src = Src::new(img, c.flags);
            let Some(s) = self.screens.get_mut(screen) else {
                continue;
            };
            let (w, h, sp) = (s.width, s.height, s.planes);
            let mut t = bobs::Target {
                pixels: s.logic_mut(),
                width: w,
                height: h,
                planes: sp,
            };
            bobs::draw(&mut t, &src, &c, planes);
        }
    }

    /// `Bob Update` / automatic update: erase, move, draw, screen swap.
    pub fn bob_update(&mut self) {
        self.bob_eff();
        self.bob_act();
        self.bob_aff();
        self.screens.swap_all();
    }

    /// Runs a drawing operation on the current screen with the autoback
    /// handling of the original (`TAbk1`-`TAbk3`, +W.s:3552): mode 1 draws
    /// into the logic then the physic bitmap, mode 2 erases the bobs, draws,
    /// updates and swaps, then draws again into the other bitmap.
    pub fn autoback(&mut self, mut draw: impl FnMut(&mut Hardware)) {
        let Some(cur) = self.screens.current else {
            return draw(self);
        };
        let mode = self.screens.get(cur).map_or(0, |s| s.autoback);
        match mode {
            0 => draw(self),
            1 => {
                draw(self);
                let swap = |hw: &mut Hardware| {
                    if let Some(s) = hw.screens.get_mut(cur) {
                        std::mem::swap(&mut s.logic, &mut s.physic);
                    }
                };
                swap(self);
                draw(self);
                swap(self);
            }
            _ => {
                self.bob_eff();
                draw(self);
                self.bob_act();
                self.bob_aff();
                self.screens.swap_all();
                self.bob_eff();
                draw(self);
                self.bob_act();
                self.bob_aff();
                self.screens.swap_all();
                self.sprites.dirty_bobs = false;
            }
        }
    }

    /// `BobSet`: creates or changes a bob.
    #[allow(clippy::too_many_arguments)]
    fn bob_set(
        &mut self,
        n: i32,
        x: Option<i32>,
        y: Option<i32>,
        i: Option<i32>,
        back: i32,
        planes: i32,
        minterm: i32,
    ) -> R<()> {
        let Some(cur) = self.screens.current else {
            return err(E_SCREEN);
        };
        if n < 0 || n >= BOB_MAX as i32 {
            return err(E_FONCALL);
        }
        let n = n as u16;
        if !self.sprites.bobs.list.contains_key(&n) {
            let s = self
                .screens
                .get(cur)
                .ok_or(crate::interp::Exc::Error(E_SCREEN))?;
            let bob = Bob::new(
                n,
                cur,
                s.width,
                s.height,
                s.is_double_buffered(),
                back,
                planes,
                minterm,
            );
            self.sprites.bobs.list.insert(n, bob);
        }
        let bob = self.sprites.bobs.list.get_mut(&n).unwrap();
        if bob.act >= 0 {
            if let Some(x) = x {
                bob.x = x as i16;
                bob.act |= 2;
            }
            if let Some(y) = y {
                bob.y = y as i16;
                bob.act |= 4;
            }
            if let Some(i) = i {
                bob.image = i as u16;
                bob.act |= 1;
            }
        }
        self.sprites.dirty_bobs = true;
        Ok(())
    }

    /// `Paste Bob` / `Paste Icon` (`TPatch`): draws an image at x,y (no hot
    /// spot) in the clip rectangle of the current screen.
    fn paste(&mut self, icons: bool, x: i32, y: i32, n: i32) -> R<()> {
        if n < 0 {
            return err(E_FONCALL);
        }
        let idx = self.ad_image(icons, n)?;
        let empty = self
            .image_bank(icons)
            .is_none_or(|b| b[idx as usize - 1].is_empty());
        if empty {
            return err(if icons { E_IMAGE } else { E_BOB });
        }
        let Some(cur) = self.screens.current else {
            return err(E_SCREEN);
        };
        let mut mask = self.sprites.mask(icons, idx);
        if mask == MaskState::Lazy {
            // Icons are pasted without mask unless Make Icon Mask was used;
            // bob images get their mask computed (BobCalc).
            mask = if icons {
                MaskState::None
            } else {
                MaskState::Made
            };
            self.sprites.set_mask(icons, idx, mask);
        }
        let flags = (n as u16) & FLIP_MASK;
        let img = self.image_bank(icons).unwrap()[idx as usize - 1].clone();
        let src = Src::new(&img, flags);
        let s = self.screens.get(cur).unwrap();
        let (x0, y0, x1, y1) = s.gr.clip;
        let lim = Limits {
            left: x0 & !15,
            top: y0,
            right: ((x1.min(s.width as i32) + 15) & !15).min(s.width as i32),
            bottom: y1.min(s.height as i32),
        };
        let Some(c) = bobs::calc(
            &img,
            idx,
            flags,
            x,
            y,
            lim,
            (s.width, s.height, s.planes),
            0,
            mask,
        ) else {
            return Ok(());
        };
        self.autoback(|hw| {
            if let Some(s) = hw.screens.get_mut(cur) {
                let (w, h, p) = (s.width, s.height, s.planes);
                let mut t = bobs::Target {
                    pixels: s.logic_mut(),
                    width: w,
                    height: h,
                    planes: p,
                };
                bobs::draw(&mut t, &src, &c, 0xFFFF);
            }
        });
        Ok(())
    }

    /// `Limit Bob [n,]x1,y1 To x2,y2` (`BobLim`).
    fn limit_bob(&mut self, n: i32, coords: Option<[i32; 4]>) -> R<()> {
        if self.sprites.bobs.list.is_empty() {
            return Ok(());
        }
        let Some(cur) = self.screens.current else {
            return err(E_SCREEN);
        };
        let s = self.screens.get(cur).unwrap();
        let (tx, ty) = (s.width as i32, s.height as i32);
        // The original compares the low words with EntNul's (0).
        let [mut x1, mut y1, mut x2, mut y2] = coords.unwrap_or([0, 0, 0, 0]);
        if x1 as u16 == 0 {
            x1 = 0;
        }
        if y1 as u16 == 0 {
            y1 = 0;
        }
        if x2 as u16 == 0 {
            x2 = tx;
        }
        if y2 as u16 == 0 {
            y2 = ty;
        }
        let (x1, y1, x2, y2) = (
            (x1 as u16 & 0xFFF0) as i32,
            y1 as u16 as i32,
            (x2 as u16 & 0xFFF0) as i32,
            y2 as u16 as i32,
        );
        // Note: the original compares y2 with x1 (+W.s:1054).
        if x2 <= x1 || y2 <= x1 || x2 > tx || y2 > ty {
            return err(E_FONCALL);
        }
        let mut changed = false;
        for bob in self.sprites.bobs.list.values_mut() {
            if bob.act < 0 || bob.screen != cur {
                continue;
            }
            if n >= 0 {
                if (bob.number as i32) < n {
                    continue;
                }
                if bob.number as i32 > n {
                    break;
                }
            }
            bob.lim = Limits {
                left: x1,
                top: y1,
                right: x2,
                bottom: y2,
            };
            bob.act |= 1;
            changed = true;
        }
        if changed {
            self.sprites.dirty_bobs = true;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Sprite bank operations
    // ------------------------------------------------------------------

    /// `Get Sprite / Get Bob / Get Icon [s,]n,x1,y1 To x2,y2`.
    fn get_image(&mut self, icons: bool, screen: Option<i32>, a: &[i32]) -> R<()> {
        let [n, x1, y1, x2, y2] = [a[0], a[1], a[2], a[3], a[4]];
        let sn = match screen {
            Some(s) => {
                if s < 0 || self.screens.get(s as usize).is_none() {
                    return err(E_SCREEN);
                }
                s as usize
            }
            None => self
                .screens
                .current
                .ok_or(crate::interp::Exc::Error(E_SCREEN))?,
        };
        if x1 < 0 || y1 < 0 || x2 < 0 || y2 < 0 {
            return err(E_FONCALL);
        }
        let s = self.screens.get(sn).unwrap();
        if x2 > s.width as i32 || y2 > s.height as i32 || x2 <= x1 || y2 <= y1 {
            return err(E_FONCALL);
        }
        if n <= 0 {
            return err(E_FONCALL);
        }
        let img = images::grab(
            s.logic_ref(),
            s.width,
            x1,
            y1,
            (x2 - x1) as u32,
            (y2 - y1) as u32,
            s.planes,
        );
        let num = if icons { 2 } else { 1 };
        if self.image_bank(icons).is_none() {
            if let Some(b) = self.banks.banks.get(&num)
                && !matches!(b.data, BankData::Images { .. })
            {
                return err(errors::BANK_ALREADY_RESERVED);
            }
            self.banks.banks.insert(num, images::new_image_bank(num));
        }
        let bank = self.image_bank_mut(icons).unwrap();
        if bank.len() < n as usize {
            bank.resize(n as usize, Image::default());
        }
        bank[n as usize - 1] = img;
        self.sprites.set_mask(
            icons,
            n as u16,
            if icons {
                MaskState::None
            } else {
                MaskState::Lazy
            },
        );
        self.sprites.generation += 1;
        Ok(())
    }

    /// `Del Sprite n [To m]`. The original requires m <= n and deletes m..n
    /// (+Lib.s IDIc3); both orders are accepted here.
    fn del_images(&mut self, icons: bool, a: i32, b: i32) -> R<()> {
        if self.image_bank(icons).is_none() {
            return err(E_BANK);
        }
        if a <= 0 || b <= 0 {
            return err(E_FONCALL);
        }
        let (first, last) = (a.min(b) as usize, a.max(b) as usize);
        let bank = self.image_bank_mut(icons).unwrap();
        let len = bank.len();
        if first <= len {
            bank.drain(first - 1..last.min(len));
        }
        let empty = bank.is_empty();
        self.sprites.masks_delete(icons, first as u16, last as u16);
        if empty {
            self.banks.banks.remove(&if icons { 2 } else { 1 });
        }
        self.sprites.generation += 1;
        Ok(())
    }

    /// `Ins Sprite n`: inserts an empty image (appended if n > count).
    fn ins_image(&mut self, icons: bool, n: i32) -> R<()> {
        let Some(bank) = self.image_bank_mut(icons) else {
            return err(E_BANK);
        };
        if n <= 0 {
            return err(E_FONCALL);
        }
        let at = (n as usize - 1).min(bank.len());
        bank.insert(at, Image::default());
        self.sprites.masks_insert(icons, at as u16 + 1);
        self.sprites.generation += 1;
        Ok(())
    }

    /// `Make Mask [n]` / `No Mask [n]` (and icon versions).
    fn set_masks(&mut self, icons: bool, n: Option<i32>, state: MaskState) -> R<()> {
        let (first, last) = match n {
            Some(n) => {
                let i = self.ad_image(icons, n)?;
                (i, i)
            }
            None => {
                let i = self.ad_image(icons, 1)?;
                (i, self.image_bank(icons).unwrap().len() as u16)
            }
        };
        for i in first..=last {
            if !self.image_bank(icons).unwrap()[i as usize - 1].is_empty() {
                self.sprites.set_mask(icons, i, state);
            }
        }
        Ok(())
    }

    /// `Get Sprite Palette [mask]` (`GSPal`): copies the bank colours to
    /// the current screen.
    fn get_bank_palette(&mut self, icons: bool, mask: i32) -> R<()> {
        let Some(pal) = self.bank_palette(icons) else {
            return err(E_BANK);
        };
        let Some(s) = self.screens.current_mut() else {
            return err(E_SCREEN);
        };
        for (i, &c) in pal.iter().enumerate() {
            if mask & (1 << i) != 0 {
                s.palette[i] = c & 0xFFF;
            }
        }
        s.version += 1;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Mouse
    // ------------------------------------------------------------------

    /// `MChange`: pointer d (0-based): 0-2 from the mouse bank, else image
    /// d-2 of the sprite bank (1 word wide, 2 planes), else pointer 1.
    fn change_mouse(&mut self, d: i32) {
        let pick = |hw: &Hardware, d: i32| -> Option<Image> {
            if d < 3 {
                return images::mouse_bank().0.get(d as usize).cloned();
            }
            let img = hw.image_bank(false)?.get((d - 3) as usize)?;
            (img.width_words == 1 && img.planes == 2).then(|| img.clone())
        };
        let (d, img) = match pick(self, d) {
            Some(i) => (d, i),
            None => (
                0,
                images::mouse_bank().0.first().cloned().unwrap_or_default(),
            ),
        };
        self.sprites.mouse_spr = d as u16;
        self.sprites.mouse_image = img;
    }

    /// `Hide` / `Show` counter (`MHide`, `MShow`).
    fn mouse_show(&mut self, v: i16) {
        self.sprites.mouse_show = v;
        if v == 0 || v == -1 {
            // HiSho1 puts the pointer in sprite 0, HiHi removes it: either
            // way a direct sprite 0 disappears until it is set again. The
            // pointer itself is drawn from the counter.
            self.sprites.shown[0] = HsShown::default();
        }
    }

    // ------------------------------------------------------------------
    // Collisions
    // ------------------------------------------------------------------

    /// Image of a bob/sprite usable for collisions (needs a mask).
    fn col_src(&self, image: u16, flags: u16) -> Option<(bobs::ColImg<'_>, i32, i32)> {
        let i = image & 0x3FFF;
        let img = self.sprite_image(i)?;
        if self.sprites.mask(false, i) != MaskState::Made {
            return None;
        }
        let (hx, hy) = images::flipped_hot(img, flags);
        Some((bobs::ColImg::new(img, flags), hx, hy))
    }

    /// Screen -> hardware coordinates of a bob (`CXyS`).
    fn bob_hard(&self, screen: usize, x: i32, y: i32) -> (i32, i32) {
        match self.screens.get(screen) {
            Some(s) => (s.x_hard(x), s.y_hard(y)),
            None => (x, y),
        }
    }

    /// `BbColl`: bob `n` against bobs (same screen) or sprites (`to_sprites`)
    /// numbered `start..=end`.
    fn bob_col(&mut self, n: i32, start: i32, end: i32, to_sprites: bool) -> i32 {
        self.sprites.clear_col();
        let Some(bob) = self
            .sprites
            .bobs
            .list
            .get(&(n as u16))
            .filter(|_| (0..BOB_MAX as i32).contains(&n))
        else {
            return 0;
        };
        if bob.act < 0 {
            return 0;
        }
        let (screen, bx, by) = (bob.screen, bob.x as i32, bob.y as i32);
        let Some((src, hx, hy)) = self.col_src(bob.image, bob.image & FLIP_MASK) else {
            return 0;
        };
        let (mut ax, mut ay) = (bx - hx, by - hy);
        let mut hits = Vec::new();
        if to_sprites {
            (ax, ay) = self.bob_hard(screen, ax, ay);
            if !(0..64).contains(&end) || end < start {
                return 0;
            }
            for sn in start.max(0)..=end {
                let a = self.sprites.act[sn as usize];
                if a.flag & 0x80 != 0 || a.image <= 0 {
                    continue;
                }
                if let Some((s2, h2x, h2y)) = self.col_src(a.image as u16, 0)
                    && bobs::collide_img(&src, ax, ay, &s2, a.x as i32 - h2x, a.y as i32 - h2y)
                {
                    hits.push(sn as u16);
                }
            }
        } else {
            for other in self.sprites.bobs.list.values() {
                let on = other.number as i32;
                if on < start {
                    continue;
                }
                if on > end {
                    break;
                }
                if on == n || other.screen != screen || other.act < 0 {
                    continue;
                }
                if let Some((s2, h2x, h2y)) = self.col_src(other.image, other.image & FLIP_MASK)
                    && bobs::collide_img(
                        &src,
                        ax,
                        ay,
                        &s2,
                        other.x as i32 - h2x,
                        other.y as i32 - h2y,
                    )
                {
                    hits.push(other.number);
                }
            }
        }
        for h in &hits {
            self.sprites.set_col(*h);
        }
        if hits.is_empty() { 0 } else { -1 }
    }

    /// `SpColl`: sprite `n` against sprites or bobs (`to_bobs`).
    fn spr_col(&mut self, n: i32, start: i32, end: i32, to_bobs: bool) -> i32 {
        self.sprites.clear_col();
        if !(0..64).contains(&n) {
            return 0;
        }
        let a = self.sprites.act[n as usize];
        let Some((src, hx, hy)) = self.col_src(a.image as u16, 0) else {
            return 0;
        };
        let (ax, ay) = (a.x as i32 - hx, a.y as i32 - hy);
        let mut hits = Vec::new();
        if to_bobs {
            for b in self.sprites.bobs.list.values() {
                let bn = b.number as i32;
                if bn < start {
                    continue;
                }
                if bn > end {
                    break;
                }
                if b.act < 0 {
                    continue;
                }
                let (bx, by) = self.bob_hard(b.screen, b.x as i32, b.y as i32);
                if let Some((s2, h2x, h2y)) = self.col_src(b.image, b.image & FLIP_MASK)
                    && bobs::collide_img(&src, ax, ay, &s2, bx - h2x, by - h2y)
                {
                    hits.push(b.number);
                }
            }
        } else {
            if !(0..64).contains(&end) || end < start {
                return 0;
            }
            for sn in start.max(0)..=end {
                if sn == n {
                    continue;
                }
                let o = self.sprites.act[sn as usize];
                if o.flag & 0x80 != 0 || o.image <= 0 {
                    continue;
                }
                if let Some((s2, h2x, h2y)) = self.col_src(o.image as u16, 0)
                    && bobs::collide_img(&src, ax, ay, &s2, o.x as i32 - h2x, o.y as i32 - h2y)
                {
                    hits.push(sn as u16);
                }
            }
        }
        for h in &hits {
            self.sprites.set_col(*h);
        }
        if hits.is_empty() { 0 } else { -1 }
    }

    /// Screen pixel shown at a hardware position (front screen first).
    fn playfield_at(&self, hx: i32, hy: i32) -> Option<u8> {
        for &n in &self.screens.priority {
            let s = self.screens.get(n)?;
            if s.hidden || hy < s.display_y || hy >= s.display_y + s.display_h as i32 {
                continue;
            }
            let w = s.display_w as i32;
            if hx < s.display_x || hx >= s.display_x + w {
                return None;
            }
            let x = s.x_screen(hx);
            let y = s.y_screen(hy);
            if x < 0 || y < 0 || x >= s.width as i32 || y >= s.height as i32 {
                return None;
            }
            return Some(s.physic_ref()[(y as u32 * s.width + x as u32) as usize]);
        }
        None
    }

    /// Synthesises CLXDAT from the displayed sprites (`=Hardcol`).
    fn clxdat(&self) -> u16 {
        let con = self.sprites.hardcol;
        let slices = self.sprite_slices();
        let mut pairs: [std::collections::HashSet<(i32, i32)>; 4] = Default::default();
        for s in &slices {
            if s.channel & 1 == 1 && con & (1 << (12 + s.channel / 2)) == 0 {
                continue;
            }
            for y in 0..s.h {
                for x in 0..s.w {
                    if s.pixels[(y * s.w + x) as usize] != 0 {
                        pairs[s.channel as usize / 2].insert((s.x + x as i32, s.y + y as i32));
                    }
                }
            }
        }
        let mut dat = 0u16;
        let pair_bits = [
            (0, 1, 9),
            (0, 2, 10),
            (0, 3, 11),
            (1, 2, 12),
            (1, 3, 13),
            (2, 3, 14),
        ];
        for (a, b, bit) in pair_bits {
            if pairs[a].iter().any(|p| pairs[b].contains(p)) {
                dat |= 1 << bit;
            }
        }
        let matches = |v: u8, odd: bool| {
            (0..6)
                .filter(|p| (p % 2 == 0) == odd)
                .all(|p| con & (1 << (6 + p)) == 0 || ((v >> p) & 1) as u16 == (con >> p) & 1)
        };
        for (p, set) in pairs.iter().enumerate() {
            for &(x, y) in set {
                if let Some(v) = self.playfield_at(x, y) {
                    if matches(v, true) {
                        dat |= 1 << (1 + p);
                    }
                    if matches(v, false) {
                        dat |= 1 << (5 + p);
                    }
                }
            }
        }
        dat
    }

    /// `HColGet` (`=Hardcol(n)`).
    fn hardcol(&mut self, n: i32) -> i32 {
        self.sprites.clear_col();
        let dat = self.clxdat();
        if n < 0 {
            return if dat & 1 != 0 { -1 } else { 0 };
        }
        const T: [[i8; 6]; 4] = [
            [-1, 9, 10, 11, 1, 5],
            [9, -1, 12, 13, 2, 6],
            [10, 12, -1, 14, 3, 7],
            [11, 13, 14, -1, 4, 8],
        ];
        let row = T[((n & 6) >> 1) as usize];
        let mut d3 = 0u16;
        let mut r = 0;
        for (k, &bit) in row.iter().enumerate() {
            if bit >= 0 && dat & (1 << bit) != 0 {
                d3 |= 3 << (2 * k);
                if k < 4 {
                    r = -1;
                }
            }
        }
        self.sprites.col[0] = d3 as u8;
        self.sprites.col[1] = (d3 >> 8) as u8;
        r
    }

    // ------------------------------------------------------------------
    // AMAL
    // ------------------------------------------------------------------

    /// The AMAL bank (bank 4 named "Amal").
    fn amal_bank(&self) -> Option<&[u8]> {
        let b = self.banks.banks.get(&4)?;
        if !b.name.starts_with("Amal") {
            return None;
        }
        b.raw()
    }

    /// `Amal n,a$` / `Anim` / `Move X` / `Move Y` (`MvA3`, +Lib.s:11816).
    fn amal_create(
        &mut self,
        it: &mut Interp,
        kw: Keyword,
        kind: ChanKind,
        to_address: bool,
    ) -> R<()> {
        let a = it.inst_args(self, kw)?;
        let ch = a.int(0);
        let limit = if self.sprites.amal.sync_off { 64 } else { 16 };
        if !(0..limit).contains(&ch) {
            return err(E_FONCALL);
        }
        let bank = self.amal_bank().map(|d| Rc::new(d.to_vec()));
        let src: Vec<u8> = match a.value(1) {
            Value::Str(s) => s.to_vec(),
            v => {
                let n = match v {
                    Value::Int(n) => n,
                    Value::Float(f) => f as i32,
                    _ => 0,
                };
                if !(0..1024).contains(&n) {
                    return err(E_FONCALL);
                }
                let Some(d) = &bank else { return err(E_BANK) };
                amal_program(d, n as usize)?
            }
        };
        let target = if to_address {
            Target::Address(a.int(2) as u32 & !1)
        } else {
            self.sprites.amal.targets[ch as usize]
        };
        match target {
            Target::Bob(n) if !self.sprites.bobs.list.contains_key(&n) => return err(E_BOB),
            Target::ScreenDisplay(n) | Target::ScreenSize(n) | Target::ScreenOffset(n)
                if self.screens.get(n as usize).is_none() =>
            {
                return err(E_SCREEN);
            }
            _ => {}
        }
        match self
            .sprites
            .amal
            .create(ch as u16, kind, &src, target, bank)
        {
            Ok(()) => Ok(()),
            Err(e) => err(E_AMAL_BASE + e.code),
        }
    }

    /// `Channel n To Sprite|Bob|Screen Display|Screen Size|Screen Offset|Rainbow m`.
    fn channel(&mut self, it: &mut Interp) -> R<()> {
        let n = it.eval_int(self)?;
        if !(0..64).contains(&n) {
            return err(E_FONCALL);
        }
        // The original skips the To token without checking it.
        it.pc += 2;
        let t = it.next_token();
        let m = it.eval_int(self)?;
        let (target, max): (fn(u16) -> Target, i32) = match t {
            tk::SPRITE => (Target::Sprite, 64),
            tk::BOB => (Target::Bob, 64),
            tk::SCREEN_DISPLAY => (Target::ScreenDisplay, 8),
            tk::SCREEN_SIZE => (Target::ScreenSize, 8),
            tk::SCREEN_OFFSET => (Target::ScreenOffset, 8),
            _ => (Target::Rainbow, 4),
        };
        if !(0..max).contains(&m) {
            return err(E_FONCALL);
        }
        self.sprites.amal.targets[n as usize] = target(m as u16);
        Ok(())
    }

    fn amreg(&mut self, it: &mut Interp, kw: Keyword) -> R<&mut i16> {
        let a = it.func_args(self, kw)?;
        let (ch, r) = if kw.token == tk::AMREG_2 {
            (Some(a.int(0)), a.int(1))
        } else {
            (None, a.int(0))
        };
        match ch {
            None if !(0..26).contains(&r) => return err(E_FONCALL),
            Some(c) if !(0..64).contains(&c) || !(0..10).contains(&r) => return err(E_FONCALL),
            _ => {}
        }
        self.sprites
            .amal
            .reg_mut(ch.map(|c| c as u16), r as usize)
            .ok_or(crate::interp::Exc::Error(E_FONCALL))
    }

    // ------------------------------------------------------------------
    // Instructions
    // ------------------------------------------------------------------

    pub(crate) fn sprites_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        match kw.token {
            // ---- Hardware sprites
            SPRITE => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                if !(0..SPRITE_MAX as i32).contains(&n) {
                    return err(E_FONCALL);
                }
                let act = &mut self.sprites.act[n as usize];
                let x = match a.opt(1) {
                    Some(x) => x as i16,
                    None if act.x == 0 => return err(E_FONCALL),
                    None => act.x,
                };
                let y = match a.opt(2) {
                    Some(y) => y as i16,
                    None if act.y == 0 => return err(E_FONCALL),
                    None => act.y,
                };
                act.flag |= 8;
                act.x = x;
                act.y = y;
                if let Some(i) = a.opt(3) {
                    act.image = i as i16;
                }
                self.sprites.dirty_sprites = true;
            }
            SPRITE_OFF | SPRITE_OFF_2 => {
                let a = it.inst_args(self, kw)?;
                let range = match a.opt(0) {
                    Some(n) if !(0..SPRITE_MAX as i32).contains(&n) => return err(E_FONCALL),
                    Some(n) => n as usize..n as usize + 1,
                    None => 0..SPRITE_MAX,
                };
                for n in range {
                    self.sprite_off(n);
                }
            }
            SPRITE_UPDATE_OFF => self.sprites.update_sprites = false,
            SPRITE_UPDATE_ON => self.sprites.update_sprites = true,
            SPRITE_UPDATE => self.sprites_act(),
            SET_SPRITE_BUFFER => {
                let n = it.inst_args(self, kw)?.int(0);
                if n < 16 {
                    return err(E_FONCALL);
                }
                if n as u16 != self.sprites.buffer_lines {
                    self.sprites.buffer_lines = n.min(0xFFFF) as u16;
                    for s in 0..SPRITE_MAX {
                        self.sprite_off(s);
                    }
                }
            }
            SPRITE_PRIORITY => {
                let n = it.inst_args(self, kw)?.int(0);
                let Some(cur) = self.screens.current else {
                    return err(E_SCREEN);
                };
                if !(0..=4).contains(&n) {
                    return err(E_FONCALL);
                }
                self.sprites.priority[cur] = n as u8;
            }
            HIDE => self.mouse_show(self.sprites.mouse_show.wrapping_sub(1)),
            HIDE_ON => self.mouse_show(-1),
            SHOW => self.mouse_show(self.sprites.mouse_show.wrapping_add(1)),
            SHOW_ON => self.mouse_show(0),
            CHANGE_MOUSE => {
                let n = it.inst_args(self, kw)?.int(0);
                if n < 1 {
                    return err(E_FONCALL);
                }
                self.change_mouse(n - 1);
            }
            // ---- Sprite bank
            GET_SPRITE | GET_BOB | GET_ICON => {
                let a = it.inst_args(self, kw)?;
                let v = [a.int(0), a.int(1), a.int(2), a.int(3), a.int(4)];
                self.get_image(kw.token == GET_ICON, None, &v)?;
            }
            GET_SPRITE_2 | GET_BOB_2 | GET_ICON_2 => {
                let a = it.inst_args(self, kw)?;
                let v = [a.int(1), a.int(2), a.int(3), a.int(4), a.int(5)];
                self.get_image(kw.token == GET_ICON_2, Some(a.int(0)), &v)?;
            }
            DEL_SPRITE | DEL_BOB | DEL_ICON => {
                let n = it.inst_args(self, kw)?.int(0);
                self.del_images(kw.token == DEL_ICON, n, n)?;
            }
            DEL_SPRITE_2 | DEL_BOB_2 | DEL_ICON_2 => {
                let a = it.inst_args(self, kw)?;
                self.del_images(kw.token == DEL_ICON_2, a.int(0), a.int(1))?;
            }
            INS_SPRITE | INS_BOB | INS_ICON => {
                let n = it.inst_args(self, kw)?.int(0);
                self.ins_image(kw.token == INS_ICON, n)?;
            }
            GET_SPRITE_PALETTE | GET_BOB_PALETTE | GET_ICON_PALETTE => {
                self.get_bank_palette(kw.token == GET_ICON_PALETTE, -1)?;
            }
            GET_SPRITE_PALETTE_2 | GET_BOB_PALETTE_2 | GET_ICON_PALETTE_2 => {
                let m = it.inst_args(self, kw)?.int(0);
                self.get_bank_palette(kw.token == GET_ICON_PALETTE_2, m)?;
            }
            MAKE_MASK | MAKE_MASK_2 | MAKE_ICON_MASK | MAKE_ICON_MASK_2 => {
                let n = it.inst_args(self, kw)?.opt(0);
                self.set_masks(
                    matches!(kw.token, MAKE_ICON_MASK | MAKE_ICON_MASK_2),
                    n,
                    MaskState::Made,
                )?;
            }
            NO_MASK | NO_MASK_2 | NO_ICON_MASK | NO_ICON_MASK_2 => {
                let n = it.inst_args(self, kw)?.opt(0);
                self.set_masks(
                    matches!(kw.token, NO_ICON_MASK | NO_ICON_MASK_2),
                    n,
                    MaskState::None,
                )?;
            }
            HOT_SPOT | HOT_SPOT_2 => {
                let a = it.inst_args(self, kw)?;
                let i = self.ad_image(false, a.int(0))?;
                let (mode, x, y) = if kw.token == HOT_SPOT {
                    ((a.int(1) & 0x77) + 1, 0, 0)
                } else {
                    (0, a.int(1), a.int(2))
                };
                let img = &mut self.image_bank_mut(false).unwrap()[i as usize - 1];
                if img.is_empty() {
                    return err(E_FONCALL);
                }
                images::set_hot_spot(img, mode, x, y);
                self.sprites.generation += 1;
            }
            // ---- Bobs
            BOB => {
                let a = it.inst_args(self, kw)?;
                self.bob_set(a.int(0), a.opt(1), a.opt(2), a.opt(3), 0, -1, 0)?;
            }
            SET_BOB => {
                let a = it.inst_args(self, kw)?;
                self.bob_set(
                    a.int(0),
                    None,
                    None,
                    None,
                    a.opt(1).unwrap_or(0),
                    a.opt(2).unwrap_or(-1),
                    a.opt(3).unwrap_or(0),
                )?;
            }
            BOB_OFF => {
                for b in self.sprites.bobs.list.values_mut() {
                    b.act = -1;
                }
                self.sprites.dirty_bobs = true;
            }
            BOB_OFF_2 => {
                let n = it.inst_args(self, kw)?.int(0);
                if let Some(b) = self
                    .sprites
                    .bobs
                    .list
                    .get_mut(&(n as u16))
                    .filter(|_| n >= 0)
                {
                    b.act = -1;
                    self.sprites.dirty_bobs = true;
                }
            }
            BOB_UPDATE_OFF => self.sprites.update_bobs = false,
            BOB_UPDATE_ON => self.sprites.update_bobs = true,
            BOB_UPDATE => self.bob_update(),
            BOB_CLEAR => self.bob_eff(),
            BOB_DRAW => {
                self.bob_act();
                self.bob_aff();
            }
            UPDATE_OFF => {
                self.sprites.update_bobs = false;
                self.sprites.update_sprites = false;
            }
            UPDATE_ON => {
                self.sprites.update_bobs = true;
                self.sprites.update_sprites = true;
            }
            UPDATE => {
                self.bob_update();
                self.sprites_act();
            }
            UPDATE_EVERY => {
                let n = it.inst_args(self, kw)?.int(0);
                if !(0..65536).contains(&n) {
                    return err(E_FONCALL);
                }
                self.sprites.update_every = n as u16;
            }
            LIMIT_BOB => self.limit_bob(-1, None)?,
            LIMIT_BOB_2 => {
                let a = it.inst_args(self, kw)?;
                self.limit_bob(-1, Some([a.int(0), a.int(1), a.int(2), a.int(3)]))?;
            }
            LIMIT_BOB_3 => {
                let a = it.inst_args(self, kw)?;
                self.limit_bob(a.int(0), Some([a.int(1), a.int(2), a.int(3), a.int(4)]))?;
            }
            PRIORITY_ON | PRIORITY_OFF => {
                if self.screens.current.is_none() {
                    return err(E_SCREEN);
                }
                self.sprites.bobs.priority = if kw.token == PRIORITY_ON {
                    self.screens.current
                } else {
                    None
                };
            }
            PRIORITY_REVERSE_ON | PRIORITY_REVERSE_OFF => {
                if self.screens.current.is_none() {
                    return err(E_SCREEN);
                }
                self.sprites.bobs.reverse = kw.token == PRIORITY_REVERSE_ON;
            }
            PUT_BOB => {
                let n = it.inst_args(self, kw)?.int(0);
                if n < 0 {
                    return err(E_FONCALL);
                }
                let Some(b) = self.sprites.bobs.list.get_mut(&(n as u16)) else {
                    return err(E_FONCALL);
                };
                b.ecpt = b.decor;
            }
            PASTE_BOB | PASTE_ICON => {
                let a = it.inst_args(self, kw)?;
                self.paste(kw.token == PASTE_ICON, a.int(0), a.int(1), a.int(2))?;
            }
            // ---- Collisions
            SET_HARDCOL => {
                let a = it.inst_args(self, kw)?;
                self.sprites.hardcol =
                    0xF000 | ((a.int(0) as u16 & 0x3F) << 6) | (a.int(1) as u16 & 0x3F);
            }
            // ---- AMAL
            AMAL => self.amal_create(it, kw, ChanKind::Amal, false)?,
            AMAL_2 => self.amal_create(it, kw, ChanKind::Amal, true)?,
            ANIM => self.amal_create(it, kw, ChanKind::Anim, false)?,
            ANIM_FREEZE_3 => self.amal_create(it, kw, ChanKind::Anim, true)?,
            MOVE_X => self.amal_create(it, kw, ChanKind::MoveX, false)?,
            MOVE_X_2 => self.amal_create(it, kw, ChanKind::MoveX, true)?,
            MOVE_Y => self.amal_create(it, kw, ChanKind::MoveY, false)?,
            MOVE_Y_2 => self.amal_create(it, kw, ChanKind::MoveY, true)?,
            AMAL_ON | AMAL_ON_2 | AMAL_OFF | AMAL_OFF_2 | AMAL_FREEZE | AMAL_FREEZE_2 | ANIM_ON
            | ANIM_ON_2 | ANIM_OFF | ANIM_OFF_2 | ANIM_FREEZE | ANIM_FREEZE_2 | MOVE_ON
            | MOVE_ON_2 | MOVE_OFF | MOVE_OFF_2 | MOVE_FREEZE | MOVE_FREEZE_2 => {
                let a = it.inst_args(self, kw)?;
                let (mask, mode) = match kw.token {
                    AMAL_ON | AMAL_ON_2 => (1, 1),
                    AMAL_OFF | AMAL_OFF_2 => (1, -1),
                    AMAL_FREEZE | AMAL_FREEZE_2 => (1, 0),
                    ANIM_ON | ANIM_ON_2 => (2, 1),
                    ANIM_OFF | ANIM_OFF_2 => (2, -1),
                    ANIM_FREEZE | ANIM_FREEZE_2 => (2, 0),
                    MOVE_ON | MOVE_ON_2 => (12, 1),
                    MOVE_OFF | MOVE_OFF_2 => (12, -1),
                    _ => (12, 0),
                };
                let ch = a.opt(0).map(|n| n as u16);
                self.sprites.amal.on_off_freeze(ch, mask, mode);
            }
            AMREG | AMREG_2
                if kw
                    .def()
                    .is_some_and(|d| d.kind() == TokenKind::ReservedVariable) =>
            {
                // Amreg(...)=value
                let a = it.func_args(self, kw)?;
                it.expect(OP_EQ)?;
                let v = it.eval_int(self)?;
                let (ch, r) = if kw.token == AMREG_2 {
                    (Some(a.int(0)), a.int(1))
                } else {
                    (None, a.int(0))
                };
                match ch {
                    None if !(0..26).contains(&r) => return err(E_FONCALL),
                    Some(c) if !(0..64).contains(&c) || !(0..10).contains(&r) => {
                        return err(E_FONCALL);
                    }
                    _ => {}
                }
                let Some(reg) = self.sprites.amal.reg_mut(ch.map(|c| c as u16), r as usize) else {
                    return err(E_FONCALL);
                };
                *reg = v as i16;
            }
            AMPLAY | AMPLAY_2 => {
                let a = it.inst_args(self, kw)?;
                let (speed, dir) = (a.opt(0), a.opt(1));
                let (start, end) = if kw.token == AMPLAY_2 {
                    (a.int(2), a.int(3))
                } else {
                    (0, 63)
                };
                if !(0..64).contains(&end) || start < 0 || end < start {
                    return err(E_FONCALL);
                }
                self.sprites.amal.set_play(
                    start as u16,
                    end as u16,
                    speed.map(|v| v as i16),
                    dir.map(|v| v as i16),
                );
            }
            SYNCHRO_ON => self.sprites.amal.sync_off = false,
            SYNCHRO_OFF => self.sprites.amal.sync_off = true,
            SYNCHRO => {
                if self.sprites.amal.sync_off {
                    self.amal_tick();
                }
            }
            CHANNEL => self.channel(it)?,
            FREEZE => self.sprites.amal.frozen_all = true,
            UNFREEZE => self.sprites.amal.frozen_all = false,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Sprite Off n` (`HsXOff`).
    fn sprite_off(&mut self, n: usize) {
        let a = &mut self.sprites.act[n];
        a.flag = 0;
        a.image = 0;
        self.sprites.amal.remove_target(Target::Sprite(n as u16));
        self.sprites.shown[n] = HsShown::default();
        self.sprites.dirty_sprites = true;
    }

    pub(crate) fn sprites_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            X_SPRITE | Y_SPRITE | I_SPRITE => {
                let n = it.func_args(self, kw)?.int(0);
                if !(0..SPRITE_MAX as i32).contains(&n) {
                    return err(E_FONCALL);
                }
                let a = self.sprites.act[n as usize];
                (match kw.token {
                    X_SPRITE => a.x,
                    Y_SPRITE => a.y,
                    _ => a.image,
                }) as i32
            }
            X_BOB | Y_BOB | I_BOB => {
                let n = it.func_args(self, kw)?.int(0);
                if n < 0 {
                    return err(E_FONCALL);
                }
                let Some(b) = self.sprites.bobs.list.get(&(n as u16)) else {
                    return err(E_FONCALL);
                };
                match kw.token {
                    X_BOB => b.x as i32,
                    Y_BOB => b.y as i32,
                    _ => b.image as i16 as i32,
                }
            }
            HREV => it.func_args(self, kw)?.int(0) | 0x8000,
            VREV => it.func_args(self, kw)?.int(0) | 0x4000,
            REV => it.func_args(self, kw)?.int(0) | 0xC000,
            SPRITE_BASE | ICON_BASE => {
                // Address of the image / mask: no Amiga memory here, return
                // a non zero value when the image exists.
                let n = it.func_args(self, kw)?.int(0);
                let icons = kw.token == ICON_BASE;
                let i = self.ad_image(icons, n.abs())?;
                let has = !self.image_bank(icons).unwrap()[i as usize - 1].is_empty();
                if !has {
                    0
                } else if n < 0 {
                    match self.sprites.mask(icons, i) {
                        MaskState::Made => 1,
                        MaskState::Lazy => 0,
                        MaskState::None => -0x4000_0000,
                    }
                } else {
                    1
                }
            }
            BOB_COL | BOB_COL_2 | BOBSPRITE_COL | BOBSPRITE_COL_2 | SPRITE_COL | SPRITE_COL_2
            | SPRITEBOB_COL | SPRITEBOB_COL_2 => {
                let a = it.func_args(self, kw)?;
                let n = a.int(0);
                let two = matches!(
                    kw.token,
                    BOB_COL_2 | BOBSPRITE_COL_2 | SPRITE_COL_2 | SPRITEBOB_COL_2
                );
                let (start, end) = if two {
                    (a.int(1), a.int(2))
                } else {
                    match kw.token {
                        BOB_COL | SPRITEBOB_COL => (0, 10000),
                        _ => (0, 63),
                    }
                };
                if n < 0 || start < 0 || end < 0 {
                    return err(E_FONCALL);
                }
                if two && matches!(kw.token, BOBSPRITE_COL_2 | SPRITE_COL_2) && end > 63 {
                    return err(E_FONCALL);
                }
                match kw.token {
                    BOB_COL | BOB_COL_2 => self.bob_col(n, start, end, false),
                    BOBSPRITE_COL | BOBSPRITE_COL_2 => self.bob_col(n, start, end, true),
                    SPRITE_COL | SPRITE_COL_2 => self.spr_col(n, start, end, false),
                    _ => self.spr_col(n, start, end, true),
                }
            }
            COL => {
                let n = it.func_args(self, kw)?.int(0);
                self.sprites.get_col(n)
            }
            HARDCOL => {
                let n = it.func_args(self, kw)?.int(0);
                if n >= 8 {
                    return err(E_FONCALL);
                }
                self.hardcol(n)
            }
            AMALERR => self.sprites.amal.error_pos as i32,
            AMREG | AMREG_2 => *self.amreg(it, kw)? as i32,
            MOVON | CHANAN | CHANMV => {
                let n = it.func_args(self, kw)?.int(0);
                if !(0..64).contains(&n) {
                    return err(E_FONCALL);
                }
                let a = &self.sprites.amal;
                let r = match kw.token {
                    MOVON => a.movon(n as u16),
                    CHANAN => a.chanan(n as u16),
                    _ => a.chanmv(n as u16),
                };
                -(r as i32)
            }
            _ => return Ok(None),
        };
        Ok(Some(Value::Int(v)))
    }
}

/// String of AMAL program `n` of the AMAL bank (+Lib.s:11834-11850).
fn amal_program(d: &[u8], n: usize) -> R<Vec<u8>> {
    let rd16 = |p: usize| -> Option<usize> {
        Some(u16::from_be_bytes([*d.get(p)?, *d.get(p + 1)?]) as usize)
    };
    let l = d
        .get(0..4)
        .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if l == 0 {
        return err(E_FONCALL);
    }
    let count = rd16(l).ok_or(crate::interp::Exc::Error(E_FONCALL))?;
    if n > count {
        return err(E_FONCALL);
    }
    let entry = rd16(l + 2 + 2 * n).unwrap_or(0);
    if entry == 0 {
        return Ok(Vec::new());
    }
    let s = l + 2 + 2 * entry;
    let len = rd16(s).unwrap_or(0);
    Ok(d.get(s + 2..s + 2 + len).unwrap_or(&[]).to_vec())
}

impl AmalHost for Hardware {
    fn amal_get(&mut self, t: Target, f: Field) -> i16 {
        match t {
            Target::Sprite(n) => {
                let a = &self.sprites.act[n as usize % SPRITE_MAX];
                match f {
                    Field::X => a.x,
                    Field::Y => a.y,
                    Field::A => a.image,
                }
            }
            Target::Bob(n) => self.sprites.bobs.list.get(&n).map_or(0, |b| match f {
                Field::X => b.x,
                Field::Y => b.y,
                Field::A => b.image as i16,
            }),
            Target::ScreenDisplay(n) | Target::ScreenSize(n) | Target::ScreenOffset(n) => {
                let Some(s) = self.screens.get(n as usize) else {
                    return 0;
                };
                let v = match (t, f) {
                    (Target::ScreenDisplay(_), Field::X) => {
                        s.pending_display[0].unwrap_or(s.display_x)
                    }
                    (Target::ScreenDisplay(_), Field::Y) => {
                        s.pending_display[1].unwrap_or(s.display_y)
                    }
                    (Target::ScreenSize(_), Field::X) => {
                        s.pending_display[2].unwrap_or(s.display_w as i32)
                    }
                    (Target::ScreenSize(_), Field::Y) => {
                        s.pending_display[3].unwrap_or(s.display_h as i32)
                    }
                    (Target::ScreenOffset(_), Field::X) => {
                        s.pending_offset[0].unwrap_or(s.offset_x)
                    }
                    (Target::ScreenOffset(_), Field::Y) => {
                        s.pending_offset[1].unwrap_or(s.offset_y)
                    }
                    _ => 0,
                };
                v as i16
            }
            Target::Rainbow(n) => self.sprites.rainbow_act[n as usize % 4][f as usize],
            Target::Address(_) => 0,
        }
    }

    fn amal_set(&mut self, t: Target, f: Field, v: i16) {
        let bit = match f {
            Field::A => 1,
            Field::X => 2,
            Field::Y => 4,
        };
        match t {
            Target::Sprite(n) => {
                let a = &mut self.sprites.act[n as usize % SPRITE_MAX];
                a.flag |= bit;
                match f {
                    Field::X => a.x = v,
                    Field::Y => a.y = v,
                    Field::A => a.image = v,
                }
                self.sprites.dirty_sprites = true;
            }
            Target::Bob(n) => {
                if let Some(b) = self.sprites.bobs.list.get_mut(&n) {
                    b.act |= bit as i8;
                    match f {
                        Field::X => b.x = v,
                        Field::Y => b.y = v,
                        Field::A => b.image = v as u16,
                    }
                    self.sprites.dirty_bobs = true;
                }
            }
            Target::ScreenDisplay(n) | Target::ScreenSize(n) | Target::ScreenOffset(n) => {
                let Some(s) = self.screens.get_mut(n as usize) else {
                    return;
                };
                let v = v as i32;
                match (t, f) {
                    (Target::ScreenDisplay(_), Field::X) => s.pending_display[0] = Some(v),
                    (Target::ScreenDisplay(_), Field::Y) => s.pending_display[1] = Some(v),
                    (Target::ScreenSize(_), Field::X) => s.pending_display[2] = Some(v),
                    (Target::ScreenSize(_), Field::Y) => s.pending_display[3] = Some(v),
                    (Target::ScreenOffset(_), Field::X) => s.pending_offset[0] = Some(v),
                    (Target::ScreenOffset(_), Field::Y) => s.pending_offset[1] = Some(v),
                    _ => {}
                }
            }
            Target::Rainbow(n) => {
                self.sprites.rainbow_act[n as usize % 4][f as usize] = v;
                self.sprites.rainbow_changed[n as usize % 4] = true;
            }
            Target::Address(_) => {}
        }
    }

    fn amal_mouse(&self) -> (i16, i16) {
        (self.input.mouse_x as i16, self.input.mouse_y as i16)
    }

    fn amal_mouse_key(&self, right: bool) -> bool {
        self.input.mouse_buttons & if right { 2 } else { 1 } != 0
    }

    fn amal_joy(&self, port: i16) -> i16 {
        self.input.joy_state(port as i32) as i16
    }

    fn amal_col(&self, n: i16) -> i16 {
        self.sprites.get_col(n as i32) as i16
    }

    fn amal_collide(&mut self, bob: bool, n: i16, s: i16, e: i16) -> i16 {
        let (n, s, e) = (n as i32, s as i32, e as i32);
        (if bob {
            self.bob_col(n, s, e, false)
        } else {
            self.spr_col(n, s, e, false)
        }) as i16
    }

    fn amal_conv(&self, c: Conv, screen: i16, v: i16) -> Option<i16> {
        let s = self.screens.get(screen as usize)?;
        let v = v as i32;
        // AmXH / AmYH / AmXS / AmYS (+W.s:8926-8974): no interlace handling.
        let r = match c {
            Conv::XHard => (if s.hires { v >> 1 } else { v }) + s.display_x,
            Conv::YHard => v + s.display_y,
            Conv::XScreen => {
                let d = v - s.display_x;
                (if s.hires { d * 2 } else { d }) + s.offset_x
            }
            Conv::YScreen => v - s.display_y + s.offset_y,
        };
        Some(r as i16)
    }

    fn amal_vhpos(&self) -> u16 {
        (self.vbl_count as u16)
            .wrapping_mul(0x1D7)
            .wrapping_add(self.timer as u16)
    }

    fn amal_vu(&mut self, _voice: i16) -> i16 {
        0
    }
}

#[cfg(test)]
mod tests {
    use crate::Machine;
    use crate::banks::{Bank, BankData, Image};
    use crate::gfx::images::grab;
    use crate::interp::RunState;

    /// A w x h image filled with colour `c` (2 planes), hot spot 0,0.
    fn block(w: u32, h: u32, c: u8) -> Image {
        grab(&vec![c; (w * h) as usize], w, 0, 0, w, h, 2)
    }

    /// Runs a program with a sprite bank for `frames` VBLs.
    fn run(src: &str, images: Vec<Image>, frames: usize) -> Machine {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let mut m = Machine::new();
        m.run_program(&prg).unwrap();
        if !images.is_empty() {
            let mut palette = [0u16; 32];
            palette[17] = 0xF00;
            m.hw.banks.banks.insert(
                1,
                Bank {
                    number: 1,
                    name: "Sprites ".into(),
                    chip: true,
                    data_bank: true,
                    data: BankData::Images { images, palette },
                },
            );
        }
        for _ in 0..frames {
            m.vbl();
        }
        m
    }

    /// Same, the program must not stop with an error.
    fn run_ok(src: &str, images: Vec<Image>, frames: usize) -> Machine {
        let m = run(src, images, frames);
        if let RunState::Stopped(info) = &m.state
            && !matches!(
                info.reason,
                crate::interp::StopReasonOrError::Stop(crate::interp::StopReason::End)
            )
        {
            panic!("program stopped: {:?} {:?}", info, m.hw.log);
        }
        m
    }

    fn pixel(m: &Machine, x: i32, y: i32) -> u8 {
        let s = m.hw.screens.get(0).unwrap();
        s.logic_ref()[(y as u32 * s.width + x as u32) as usize]
    }

    #[test]
    fn bob_draw_move_and_erase() {
        let src = "Cls 0\nBob 1,10,20,1\nWait Vbl\nWait Vbl\nBob 1,50,20,1\nWait Vbl\nWait Vbl\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 3)], 2);
        assert_eq!(pixel(&m, 10, 20), 3);
        assert_eq!(pixel(&m, 25, 23), 3);
        assert_eq!(pixel(&m, 26, 23), 0);
        let m = run_ok(src, vec![block(16, 4, 3)], 8);
        assert_eq!(pixel(&m, 10, 20), 0, "old position restored");
        assert_eq!(pixel(&m, 50, 20), 3);
        assert_eq!(m.hw.sprites.bobs.list[&1].x, 50);
    }

    #[test]
    fn bob_hot_spot_flip_and_off() {
        // Left column colour 1 only.
        let img = {
            let mut px = vec![0u8; 64];
            for y in 0..4 {
                px[y * 16] = 1;
            }
            let mut i = grab(&px, 16, 0, 0, 16, 4, 2);
            i.hot_x = 8;
            i.hot_y = 2;
            i
        };
        let src = "Cls 0\nBob 2,100,50,Hrev(1)\nWait Vbl\nWait Vbl\nDo\nLoop\n";
        let m = run_ok(src, vec![img.clone()], 4);
        // Flipped: hot spot 16-8 = 8, pixel column 15 -> x = 100-8+15.
        assert_eq!(pixel(&m, 107, 48), 1);
        assert_eq!(pixel(&m, 92, 48), 0);
        let src = "Cls 0\nBob 2,100,50,1\nWait Vbl\nBob Off 2\nWait Vbl\nWait Vbl\nDo\nLoop\n";
        let m = run_ok(src, vec![img], 6);
        assert_eq!(pixel(&m, 92, 48), 0);
        assert!(m.hw.sprites.bobs.list.is_empty());
    }

    #[test]
    fn paste_bob_and_get_sprite() {
        let src = "Cls 0\nInk 2 : Bar 0,0 To 7,3\nGet Sprite 2,0,0 To 8,4\nCls 0\nPaste Bob 30,40,2\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 3)], 2);
        let bank = m.hw.image_bank(false).unwrap();
        assert_eq!(bank.len(), 2);
        assert_eq!(bank[1].pixel(7, 3), 2);
        assert_eq!(bank[1].pixel(8, 0), 0);
        assert_eq!(pixel(&m, 37, 43), 2);
        assert_eq!(pixel(&m, 38, 43), 0);
    }

    #[test]
    fn sprites_and_slices() {
        let src = "Hide On\nSprite 8,200,100,1\nSprite 1,150,60,1\nDo\nLoop\n";
        let m = run_ok(src, vec![block(32, 4, 1)], 3);
        let sl = m.hw.sprite_slices();
        // Sprite 1: two columns on channels 1 and 2; sprite 8 uses 2 free
        // channels.
        assert_eq!(sl.iter().filter(|s| s.id >= 0x1000).count(), 2);
        let d: Vec<_> = sl
            .iter()
            .filter(|s| s.id < 0x1000)
            .map(|s| (s.channel, s.x, s.y))
            .collect();
        assert_eq!(d, vec![(1, 150, 60), (2, 166, 60)]);
        // Channel 1 is pair 0: colour 17.
        assert_eq!(sl[0].pixels[0], 17);
    }

    #[test]
    fn sprite_functions_and_errors() {
        let src =
            "Sprite 3,100,80,1\nA=X Sprite(3)+Y Sprite(3)*1000+I Sprite(3)*1000000\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 1)], 2);
        let _ = m;
        let m = run("Sprite 64,1,1,1\n", vec![], 2);
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == crate::interp::StopReasonOrError::Error(23))
        );
    }

    #[test]
    fn amal_moves_sprite() {
        let src =
            "Sprite 0,100,100,1\nAmal 0,\"Move 50,0,50 ; Move 0,-20,20\"\nAmal On\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 1)], 80);
        assert_eq!(m.hw.sprites.act[0].x, 150);
        assert_eq!(m.hw.sprites.act[0].y, 80);
    }

    #[test]
    fn amal_on_bob_and_registers() {
        let src = "Bob 3,10,10,1\nChannel 2 To Bob 3\nAmal 2,\"L RA=5 ; L R0=7 ; L X=X+RA\"\nAmal On 2\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 1)], 4);
        assert_eq!(m.hw.sprites.bobs.list[&3].x, 15);
        assert_eq!(m.hw.sprites.amal.regs[0], 5);
        let src = "Bob 3,10,10,1\nChannel 2 To Bob 3\nAmal 2,\"L R0=7\"\nAmal On 2\nWait Vbl\nWait Vbl\nA=Amreg(2,0)\nAmreg(4)=9\nIf A<>7 Then Error 50\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 1)], 6);
        assert_eq!(m.hw.sprites.amal.regs[4], 9);
        // Bob not defined.
        let m = run("Channel 2 To Bob 3\nAmal 2,\"L X=1\"\n", vec![], 2);
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == crate::interp::StopReasonOrError::Error(68))
        );
        // Compilation error.
        let m = run("Amal 1,\"Next R0\"\n", vec![], 2);
        assert!(
            matches!(&m.state, RunState::Stopped(i) if i.reason == crate::interp::StopReasonOrError::Error(108))
        );
    }

    #[test]
    fn bob_collisions() {
        let src = "Bob 1,10,10,1\nBob 2,20,12,1\nBob 3,100,100,1\nWait Vbl\nWait Vbl\nA=Bob Col(1)\nB=Col(2)\nC=Col(3)\nIf A<>-1 or B<>-1 or C<>0 Then Error 50\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 1)], 5);
        assert_eq!(m.hw.sprites.get_col(2), -1);
        assert_eq!(m.hw.sprites.get_col(3), 0);
    }

    #[test]
    fn mouse_pointer_shown() {
        let m = run_ok("Do\nLoop\n", vec![], 2);
        let sl = m.hw.sprite_slices();
        assert_eq!(sl.len(), 1);
        assert_eq!(sl[0].channel, 0);
        let m = run_ok("Hide\nDo\nLoop\n", vec![], 2);
        assert!(m.hw.sprite_slices().is_empty());
    }

    #[test]
    fn double_buffer_bobs() {
        let src = "Cls 0\nDouble Buffer\nBob 1,10,10,1\nFor I=0 To 5 : Bob 1,10+I*20,10,1 : Wait Vbl : Next I\nDo\nLoop\n";
        let m = run_ok(src, vec![block(16, 4, 3)], 12);
        let s = m.hw.screens.get(0).unwrap();
        for bm in &s.bitmaps {
            // Only the final position remains in both buffers.
            let at = |x: u32, y: u32| bm[(y * s.width + x) as usize];
            assert_eq!(at(10, 10), 0);
            assert_eq!(at(70, 10), 0);
        }
        assert_eq!(s.physic_ref()[(10 * s.width + 110) as usize], 3);
    }
}
