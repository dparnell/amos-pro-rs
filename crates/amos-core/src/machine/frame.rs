//! Builds the display [`Frame`] from the screens and sprites.
//!
//! This follows the copper list built by `EcCopper` (+W.s:5700-6050): each
//! raster line belongs to the frontmost visible screen whose range
//! `[WY-1, WY+WTy)` covers it. A screen shows its colour 0 on the first
//! line of each slice it owns (the copper sets the screen up there, bitmap
//! DMA off), then its bitmap; the whole width of its lines outside the
//! display window shows its colour 0. Lines without a screen show
//! `Colour Back`. Rainbows replace one colour register per line.
//!
//! With `Copper Off` the frame is made by running the user copper list
//! instead (`gfx::copper`).

use std::collections::HashMap;

use super::Hardware;
use crate::display::{
    DISPLAY_WIDTH, Frame, Layer, LayerFormat, Rect, hw_x_to_display, hw_y_to_display, rgb12_to_rgba,
};
use crate::gfx::Screen;
use crate::gfx::copper::{self, Chip, PlaneSrc};
use crate::gfx::screen::{END_LINE, FIRST_LINE, Screens};

/// Where the pixels of a layer come from.
#[derive(Clone, Copy, Debug)]
enum Source {
    /// Bitmap `bitmap` of screen `screen`.
    Screen { screen: usize, bitmap: usize },
    /// Decoded HAM pixels of a screen (in `FrameCache::ham`).
    Ham(usize),
    /// Sprite `n` of `SpriteState::display`.
    Sprite(usize),
    /// The picture made by the user copper list.
    Copper,
    /// A one pixel dummy (background colour layers).
    Dummy,
}

#[derive(Clone, Debug)]
struct Spec {
    id: u32,
    version: u64,
    format: LayerFormat,
    source: Source,
    width: u32,
    height: u32,
    palette: usize,
    palette_rows: u32,
    band: Rect,
    window: Rect,
    src_x: i32,
    src_y: i32,
    scale_x: u32,
    scale_y: u32,
    transparent: Option<u8>,
}

/// Decoded HAM screen.
#[derive(Default)]
struct HamCache {
    key: (u64, usize, [u16; 32]),
    rgba: Vec<u8>,
    /// Incremented at each decoding (palette changes alter the pixels).
    serial: u64,
}

/// Buffers kept between frames (converted palettes, HAM pixels...).
#[derive(Default)]
pub struct FrameCache {
    specs: Vec<Spec>,
    palettes: Vec<Vec<[u8; 4]>>,
    ham: HashMap<usize, HamCache>,
    /// Display made by the user copper list (Copper Off).
    copper_rgba: Vec<u8>,
    copper_serial: u64,
}

/// Chip memory seen by the copper: screen bitmaps through the addresses
/// of `Logbase` / `Phybase` and of the lists made by `=Cop Logic`, the
/// emulated memory elsewhere.
struct ChipView<'a> {
    screens: &'a Screens,
    banks: &'a crate::banks::BankSet,
}

impl Chip for ChipView<'_> {
    fn word(&self, addr: u32) -> u16 {
        u16::from_be_bytes([self.banks.peek(addr), self.banks.peek(addr.wrapping_add(1))])
    }

    fn decode(&self, addr: u32) -> (PlaneSrc, i64) {
        match addr {
            0x0100_0000..0x0200_0000 => {
                let a = addr - 0x0100_0000;
                let sb = (a >> 20) as usize;
                let within = a & 0xF_FFFF;
                let src = PlaneSrc::Bitmap {
                    screen: sb / 2,
                    bitmap: sb % 2,
                    plane: (within >> 17) as u8,
                };
                (src, (within & 0x1_FFFF) as i64)
            }
            // Logbase(n) / Phybase(n) (inst_screen.rs).
            0x0020_0000..0x0030_0000 => {
                let a = addr - 0x0020_0000;
                let sb = (a >> 16) as usize;
                let src = PlaneSrc::Bitmap {
                    screen: sb / 2,
                    bitmap: sb % 2,
                    plane: ((a & 0xFFFF) / 0x2000) as u8,
                };
                (src, (a & 0x1FFF) as i64)
            }
            _ => (PlaneSrc::Memory, addr as i64),
        }
    }

    fn plane_bytes(&self, src: PlaneSrc, offset: i64, out: &mut [u8]) {
        out.fill(0);
        match src {
            PlaneSrc::Memory => {
                for (k, b) in out.iter_mut().enumerate() {
                    *b = self.banks.peek((offset + k as i64) as u32);
                }
            }
            PlaneSrc::Bitmap {
                screen,
                bitmap,
                plane,
            } => {
                let Some(s) = self.screens.get(screen) else {
                    return;
                };
                let Some(bm) = s.bitmaps.get(bitmap) else {
                    return;
                };
                if plane >= s.planes {
                    return;
                }
                let (w, h) = (s.width as i64, s.height as i64);
                let bpr = (w / 8).max(1);
                for (k, b) in out.iter_mut().enumerate() {
                    let o = offset + k as i64;
                    let (y, x) = (o.div_euclid(bpr), o.rem_euclid(bpr) * 8);
                    if o < 0 || y >= h {
                        continue;
                    }
                    let row = &bm[(y * w + x) as usize..(y * w + x + 8) as usize];
                    for (bit, &px) in row.iter().enumerate() {
                        if (px >> plane) & 1 != 0 {
                            *b |= 0x80 >> bit;
                        }
                    }
                }
            }
        }
    }
}

/// Palette transformation (dual playfield 2 colours).
type PaletteMap = dyn Fn(&[u16; 32]) -> [u16; 32];

static DUMMY_PIXEL: [u8; 4] = [0; 4];
static DUMMY_PALETTE: [[u8; 4]; 1] = [[0; 4]];

/// 256 entry RGBA palette of a screen: EHB screens get colours 32-63 at
/// half brightness.
fn rgba_palette(pal: &[u16; 32], ehb: bool, out: &mut Vec<[u8; 4]>) {
    let start = out.len();
    out.resize(start + 256, [0, 0, 0, 255]);
    for (i, &c) in pal.iter().enumerate() {
        out[start + i] = rgb12_to_rgba(c);
        if ehb {
            out[start + 32 + i] = rgb12_to_rgba((c >> 1) & 0x777);
        }
    }
}

/// Decodes a HAM6 bitmap (planes 4-5 select: 0 palette, 1 blue, 2 red,
/// 3 green; each line starts from colour 0).
fn decode_ham(s: &Screen, bitmap: &[u8], out: &mut Vec<u8>) {
    let (w, h) = (s.width as usize, s.height as usize);
    out.resize(w * h * 4, 0);
    for y in 0..h {
        let mut c = s.palette[0];
        for x in 0..w {
            let p = bitmap[y * w + x];
            let v = (p & 15) as u16;
            c = match p >> 4 & 3 {
                0 => s.palette[v as usize],
                1 => (c & 0xFF0) | v,
                2 => (c & 0x0FF) | (v << 8),
                _ => (c & 0xF0F) | (v << 4),
            };
            out[(y * w + x) * 4..][..4].copy_from_slice(&rgb12_to_rgba(c));
        }
    }
}

/// Shrinks the display window so that it does not go past the end of the
/// bitmap (the Amiga would show the memory that follows; colour 0 is shown
/// instead).
fn clip_window(
    mut w: Rect,
    width: u32,
    height: u32,
    src_x: i32,
    src_y: i32,
    sx: u32,
    sy: u32,
) -> Rect {
    let aw = ((width as i32 - src_x).max(0) as u32) * sx;
    let ah = ((height as i32 - src_y).max(0) as u32) * sy;
    w.w = w.w.min(aw);
    w.h = w.h.min(ah);
    w
}

impl Hardware {
    /// Screen whose bitmap a screen shows (itself, or the original of a
    /// clone) and the index of the displayed bitmap.
    pub(crate) fn shown_bitmap(&self, n: usize) -> (usize, usize) {
        let s = self.screens.get(n).unwrap();
        if let Some(o) = s.clone_of
            && let Some(orig) = self.screens.get(o)
            && orig.width == s.width
            && orig.height == s.height
        {
            return (o, orig.physic);
        }
        (n, s.physic)
    }

    /// Owner of each raster line (index = line - FIRST_LINE).
    fn line_owners(&self) -> Vec<Option<usize>> {
        let mut owners = vec![None; (END_LINE - FIRST_LINE) as usize];
        for (i, o) in owners.iter_mut().enumerate() {
            let l = FIRST_LINE + i as i32;
            *o = self.screens.priority.iter().copied().find(|&n| {
                self.screens.get(n).is_some_and(|s| {
                    !s.hidden && l >= s.display_y - 1 && l < s.display_y + s.display_h as i32
                })
            });
        }
        owners
    }

    /// Slices of lines owned by the same screen: (screen, first line, end).
    pub(crate) fn line_runs(&self) -> Vec<(usize, i32, i32)> {
        let owners = self.line_owners();
        let mut runs: Vec<(usize, i32, i32)> = Vec::new();
        let mut i = 0;
        while i < owners.len() {
            let Some(n) = owners[i] else {
                i += 1;
                continue;
            };
            let mut j = i;
            while j < owners.len() && owners[j] == Some(n) {
                j += 1;
            }
            let (l0, l1) = (FIRST_LINE + i as i32, FIRST_LINE + j as i32);
            // A slice cannot start on the last lines (+W.s:5940).
            if l0 < END_LINE - 1 {
                runs.push((n, l0, l1));
            }
            i = j;
        }
        runs
    }

    /// Copper Off: the whole display comes from the user copper list (no
    /// screens, no sprites).
    fn copper_frame(&mut self) {
        let chip = ChipView {
            screens: &self.screens,
            banks: &self.banks,
        };
        self.copper.render(&chip, &mut self.frame.copper_rgba);
        self.frame.copper_serial += 1;
        let r = Rect {
            x: 0,
            y: 0,
            w: DISPLAY_WIDTH,
            h: (copper::SHOWN_LINES * 2) as u32,
        };
        self.frame.specs.push(Spec {
            id: 0x7FFE_0000,
            version: self.frame.copper_serial,
            format: LayerFormat::Rgba,
            source: Source::Copper,
            width: DISPLAY_WIDTH,
            height: copper::SHOWN_LINES as u32,
            palette: usize::MAX,
            palette_rows: 1,
            band: r,
            window: r,
            src_x: 0,
            src_y: 0,
            scale_x: 1,
            scale_y: 2,
            transparent: None,
        });
    }

    /// Adds a palette (rows of 256 colours for lines `l0..l1`, or a single
    /// row) and returns its index and row count.
    fn add_palette(
        &mut self,
        pal: &[u16; 32],
        ehb: bool,
        l0: i32,
        l1: i32,
        map: Option<&PaletteMap>,
    ) -> (usize, u32) {
        let mut out = Vec::new();
        let rows = if self.screens.rainbow_in(l0..l1) {
            (l1 - l0).max(1)
        } else {
            1
        };
        for r in 0..rows {
            let mut p = if rows > 1 {
                self.screens.line_palette(pal, l0 + r)
            } else {
                *pal
            };
            if let Some(f) = map {
                p = f(&p);
            }
            rgba_palette(&p, ehb, &mut out);
        }
        self.frame.palettes.push(out);
        (self.frame.palettes.len() - 1, rows as u32)
    }

    /// Computes the layers of the frame into `self.frame`.
    fn prepare_frame(&mut self) {
        // Changes made since the last VBL show in this frame (the program
        // may have ended before the next VBL applies them).
        if self.screens.auto_view {
            self.screens.apply_pending();
        }
        self.frame.specs.clear();
        self.frame.palettes.clear();
        if !self.copper.on {
            self.copper_frame();
            return;
        }
        let screens = &self.screens;
        self.frame
            .ham
            .retain(|n, _| screens.get(*n).is_some_and(|s| s.ham));

        // Rainbows of colour 0 also colour the lines without a screen.
        if self
            .screens
            .effects
            .rainbows
            .iter()
            .any(|r| r.colour == 0 && !r.buf.is_empty() && r.height >= 0)
        {
            let mut back = [0u16; 32];
            back[0] = self.screens.colour_back;
            let (l0, l1) = (FIRST_LINE, END_LINE);
            let (palette, palette_rows) = self.add_palette(&back, false, l0, l1, None);
            let band = Rect {
                x: 0,
                y: hw_y_to_display(l0),
                w: DISPLAY_WIDTH,
                h: ((l1 - l0) * 2) as u32,
            };
            self.frame.specs.push(Spec {
                id: 0x7FFF_0000,
                version: 0,
                format: LayerFormat::Indexed,
                source: Source::Dummy,
                width: 1,
                height: 1,
                palette,
                palette_rows,
                band,
                window: Rect::default(),
                src_x: 0,
                src_y: 0,
                scale_x: 1,
                scale_y: 1,
                transparent: None,
            });
        }

        let runs = self.line_runs();
        let mut segment: HashMap<usize, u32> = HashMap::new();
        for (n, l0, l1) in runs {
            let seg = segment.entry(n).or_insert(0);
            let seg_id = *seg;
            *seg += 1;
            self.screen_layers(n, seg_id, l0, l1);
        }

        // Hardware sprites and mouse pointer, in front.
        for (k, sp) in self.sprites.display.iter().enumerate().rev() {
            let r = Rect {
                x: sp.x,
                y: sp.y,
                w: sp.width * sp.scale_x.max(1),
                h: sp.height * sp.scale_y.max(1),
            };
            self.frame.specs.push(Spec {
                id: 0x8000_0000 | sp.id,
                version: sp.version,
                format: LayerFormat::Rgba,
                source: Source::Sprite(k),
                width: sp.width,
                height: sp.height,
                palette: usize::MAX,
                palette_rows: 1,
                band: r,
                window: r,
                src_x: 0,
                src_y: 0,
                scale_x: sp.scale_x.max(1),
                scale_y: sp.scale_y.max(1),
                transparent: Some(0),
            });
        }
    }

    /// Layers of the slice `l0..l1` of screen `n` (colour 0 on line `l0`,
    /// bitmap below).
    fn screen_layers(&mut self, n: usize, seg: u32, l0: i32, l1: i32) {
        let s = self.screens.get(n).unwrap();
        let (wx, wy, wtx) = (s.display_x, s.display_y, s.display_w as i32);
        let (hires, lace, ham, ehb) = (s.hires, s.lace, s.ham, s.is_ehb());
        let pal = s.palette;
        let (vx, vy) = (s.offset_x, s.offset_y);
        let dual = s.dual_with;
        let dual_front_pf2 = s.dual_priority;

        // Real display window (+W.s:6340-6365): clamped on the right.
        let limit = if hires { 465 } else { 465 + 16 };
        let mut wtxr = wtx;
        if wx + 1 + wtx >= limit {
            wtxr = (limit - 17 - wx).max(0);
        }
        let band = Rect {
            x: 0,
            y: hw_y_to_display(l0),
            w: DISPLAY_WIDTH,
            h: ((l1 - l0) * 2) as u32,
        };
        let first = l0 + 1;
        let window = Rect {
            x: hw_x_to_display(wx),
            y: hw_y_to_display(first),
            w: (wtxr * 2) as u32,
            h: ((l1 - first).max(0) * 2) as u32,
        };
        let (scale_x, scale_y) = (if hires { 1 } else { 2 }, if lace { 1 } else { 2 });
        let row = |vy: i32| {
            if lace {
                2 * (first - wy) + vy
            } else {
                first - wy + vy
            }
        };
        let id = ((n as u32) << 16) | (seg << 4);

        if let Some(n2) = dual
            && self.screens.get(n2).is_some()
        {
            // Dual playfield: playfield 1 uses colours 0-7, playfield 2
            // colours 8-15 (0 transparent); the master's palette.
            let (vx2, vy2) = {
                let s2 = self.screens.get(n2).unwrap();
                (s2.offset_x, s2.offset_y)
            };
            let pf2_map = |p: &[u16; 32]| -> [u16; 32] {
                let mut q = *p;
                q[1..8].copy_from_slice(&p[9..16]);
                q
            };
            let (p1, r1) = self.add_palette(&pal, false, l0, l1, None);
            let (p2, r2) = self.add_palette(&pal, false, l0, l1, Some(&pf2_map));
            let (b1, bm1) = self.shown_bitmap(n);
            let (b2, bm2) = self.shown_bitmap(n2);
            let pf1 = self.bitmap_spec(
                id,
                b1,
                bm1,
                p1,
                r1,
                band,
                window,
                vx,
                row(vy),
                scale_x,
                scale_y,
            );
            let pf2 = self.bitmap_spec(
                id | 1,
                b2,
                bm2,
                p2,
                r2,
                band,
                window,
                vx2,
                row(vy2),
                scale_x,
                scale_y,
            );
            let (mut back, mut front) = if dual_front_pf2 {
                (pf1, pf2)
            } else {
                (pf2, pf1)
            };
            back.transparent = None;
            front.transparent = Some(0);
            self.frame.specs.push(back);
            self.frame.specs.push(front);
            return;
        }

        let (pi, rows) = self.add_palette(&pal, ehb, l0, l1, None);
        let (b, bm) = self.shown_bitmap(n);
        if ham {
            let s = self.screens.get(b).unwrap();
            let key = (s.version, bm, s.palette);
            let entry = self.frame.ham.entry(n).or_default();
            if entry.key != key || entry.rgba.is_empty() {
                entry.key = key;
                entry.serial += 1;
                let s = self.screens.get(b).unwrap();
                decode_ham(s, &s.bitmaps[bm], &mut entry.rgba);
            }
            let version = entry.serial;
            let s = self.screens.get(b).unwrap();
            self.frame.specs.push(Spec {
                id,
                version,
                format: LayerFormat::Rgba,
                source: Source::Ham(n),
                width: s.width,
                height: s.height,
                palette: pi,
                palette_rows: rows,
                band,
                window: clip_window(window, s.width, s.height, vx, row(vy), scale_x, scale_y),
                src_x: vx,
                src_y: row(vy),
                scale_x,
                scale_y,
                transparent: None,
            });
            return;
        }
        let spec = self.bitmap_spec(
            id,
            b,
            bm,
            pi,
            rows,
            band,
            window,
            vx,
            row(vy),
            scale_x,
            scale_y,
        );
        self.frame.specs.push(spec);
    }

    #[allow(clippy::too_many_arguments)]
    fn bitmap_spec(
        &self,
        id: u32,
        screen: usize,
        bitmap: usize,
        palette: usize,
        palette_rows: u32,
        band: Rect,
        window: Rect,
        src_x: i32,
        src_y: i32,
        scale_x: u32,
        scale_y: u32,
    ) -> Spec {
        let s = self.screens.get(screen).unwrap();
        Spec {
            id,
            version: s.version.wrapping_mul(2) + bitmap as u64,
            format: LayerFormat::Indexed,
            source: Source::Screen { screen, bitmap },
            width: s.width,
            height: s.height,
            palette,
            palette_rows,
            band,
            window: clip_window(window, s.width, s.height, src_x, src_y, scale_x, scale_y),
            src_x,
            src_y,
            scale_x,
            scale_y,
            transparent: None,
        }
    }

    pub fn build_frame(&mut self) -> Frame<'_> {
        self.sprites_prepare_frame();
        self.prepare_frame();
        let this: &Hardware = self;
        let layers = this
            .frame
            .specs
            .iter()
            .map(|sp| {
                let pixels: &[u8] = match sp.source {
                    Source::Screen { screen, bitmap } => {
                        &this.screens.get(screen).unwrap().bitmaps[bitmap]
                    }
                    Source::Ham(n) => &this.frame.ham[&n].rgba,
                    Source::Sprite(k) => &this.sprites.display[k].rgba,
                    Source::Copper => &this.frame.copper_rgba,
                    Source::Dummy => &DUMMY_PIXEL[..1],
                };
                let palette: &[[u8; 4]] = if sp.palette == usize::MAX {
                    &DUMMY_PALETTE
                } else {
                    &this.frame.palettes[sp.palette]
                };
                Layer {
                    id: sp.id,
                    pixels_version: sp.version,
                    format: sp.format,
                    width: sp.width,
                    height: sp.height,
                    pixels,
                    palette,
                    palette_rows: sp.palette_rows,
                    band: sp.band,
                    window: sp.window,
                    src_x: sp.src_x,
                    src_y: sp.src_y,
                    scale_x: sp.scale_x,
                    scale_y: sp.scale_y,
                    transparent: sp.transparent,
                }
            })
            .collect();
        Frame {
            border: rgb12_to_rgba(this.screens.colour_back),
            layers,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::display::{DISPLAY_WIDTH, render_rgba};
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

    /// RGB of the display at hardware position (lowres x, raster line).
    fn at(img: &[u8], hx: i32, hy: i32) -> [u8; 3] {
        let x = crate::display::hw_x_to_display(hx) as usize;
        let y = crate::display::hw_y_to_display(hy) as usize;
        let o = (y * DISPLAY_WIDTH as usize + x) * 4;
        [img[o], img[o + 1], img[o + 2]]
    }

    #[test]
    fn default_screen_layout() {
        let mut m = run("Curs Off : Print \"A\"", 3);
        let img = render_rgba(&m.frame());
        // Default screen: 320x256 at (128, 42), paper colour 1 = $A40.
        assert_eq!(at(&img, 400, 200), [0xAA, 0x44, 0x00]);
        // Line 41 (setup line) shows colour 0, line 300 too (bitmap ends
        // at 297), line 305 the border.
        assert_eq!(at(&img, 400, 41), [0, 0, 0]);
        // Outside the display window: colour 0 of the screen.
        assert_eq!(at(&img, 100, 100), [0, 0, 0]);
        // 'A' top row is ...##... in pen 2 (white) at x=3.
        assert_eq!(at(&img, 128 + 3, 42), [0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn colour_back_and_priority() {
        let src = "Colour Back $F00\nScreen Open 1,320,50,4,Lowres\nScreen Display 1,,150,,\nColour 0,$00F\nCls 0\n";
        let mut m = run(src, 3);
        let img = render_rgba(&m.frame());
        // Screen 1 in front from line 149 (colour 0 setup line) to 200.
        assert_eq!(at(&img, 400, 160), [0, 0, 0xFF]);
        assert_eq!(at(&img, 400, 149), [0, 0, 0xFF]);
        // Screen 0 again below it, colour back at the bottom.
        assert_eq!(at(&img, 400, 220), [0xAA, 0x44, 0]);
        assert_eq!(at(&img, 400, 305), [0xFF, 0, 0]);
    }

    #[test]
    fn rainbow_lines() {
        let src = "Set Rainbow 0,1,64,\"(1,1,15)\",\"\",\"\"\nRainbow 0,0,100,20\n";
        let mut m = run(src, 3);
        let f = m.frame();
        assert!(f.layers.iter().any(|l| l.palette_rows > 1));
        let img = render_rgba(&f);
        let a = at(&img, 400, 101);
        let b = at(&img, 400, 110);
        assert_ne!(a, b);
        assert_eq!(at(&img, 400, 130), [0xAA, 0x44, 0]);
    }
}
