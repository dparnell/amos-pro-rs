//! Sliders: drawing (`SliHor`, `SliVer`, `SliPour` `+W.s:5020-5260`) and
//! the slider structure of the Interface (`Sl_*` `+Lib.s:25082`).
//!
//! A slider is three rectangles filled with `RectFill` and outlined
//! (AREAOUTLINE): the part before the knob and the part after it with the
//! "frame" inks, the knob with the "interior" inks. Each set is ink A,
//! ink B (paper), ink C (outline) and a fill pattern.

use super::SliderInks;
use crate::gfx::Screen;
use crate::gfx::draw::{Canvas, Pattern, builtin_pattern};

/// The slider structure (`Sl_*`, `+Equ.s:841`).
#[derive(Clone, Debug, Default)]
pub struct Slider {
    pub sx: i32,
    pub sy: i32,
    pub global: i32,
    pub position: i32,
    pub window: i32,
    pub x: i32,
    pub y: i32,
    pub vertical: bool,
    pub start: i32,
    pub size: i32,
    pub scroll: i32,
    /// Knob limits relative to `start` (computed when drawn).
    pub mouse1: i32,
    pub mouse2: i32,
    pub inactive: SliderInks,
    pub active: SliderInks,
}

/// Default slider inks of a screen (`EcCree` `+W.s:3076`): frame and knob
/// inks A and B = paper, C = pen, patterns 2 and 1.
pub fn default_inks(paper: i32, pen: i32) -> SliderInks {
    [paper, paper, pen, 2, paper, paper, pen, 1]
}

/// `SliPour`: position and size of the knob in a slider of `len` pixels
/// for `pos` / `size` out of `total`. Word arithmetic as the original.
pub fn pour(len: i32, total: i32, pos: i32, size: i32) -> (i32, i32) {
    let len = len as u16 as u32;
    let mut total = total as u16 as u32;
    let mut pos = pos as u16 as u32;
    let mut size = size as u16 as u32;
    if size >= total {
        if total == 0 {
            total = 1;
        }
        size = total;
    }
    // Flag: knob reaches the end (drawn exactly at the end).
    let full = ((size + pos) & 0xFFFF) >= total;
    let d1 = len;
    let mut d0;
    let q = (len << 16) / total;
    if q <= 0xFFFF {
        pos = (q * pos) >> 16;
        d0 = q * size;
        if d0 & 0xFFFF >= 0x8000 {
            d0 += 0x10000;
        }
        d0 = (d0 >> 16) & 0xFFFF;
    } else {
        let q = (len << 8) / total;
        if q <= 0xFFFF {
            pos = (q * pos) >> 8;
            d0 = q * size;
            if d0 & 0xFF >= 0x80 {
                d0 += 0x100;
            }
            d0 >>= 8;
        } else {
            let q = (len / total) & 0xFFFF;
            pos *= q;
            d0 = q * size;
        }
    }
    let mut d0 = d0 as u16 as i32;
    if (d0 as u16) < 4 {
        d0 = 4;
    }
    let d1 = d1 as i32;
    let mut d6 = pos as u16 as i32;
    if (d6 as u16) >= (d1 as u16) {
        d6 = d1 - d0;
    }
    let mut d7 = (d6 + d0) as u16 as i32;
    if (d7 as u16) > (d1 as u16) {
        d6 = d1 - d0;
        d7 = d1;
    }
    let mut size = (d7 - d6) as u16 as i32;
    let mut off = d6 as i16 as i32;
    if full {
        off = d1 - size;
    }
    size = size as i16 as i32;
    (off, size)
}

/// `SPat`: fill pattern n (0 none, > 0 built in, < 0 sprite image).
fn pattern(n: i32, sprites: Option<&crate::banks::Bank>) -> Option<Pattern> {
    let n = n as i16 as i32;
    if n > 0 {
        return builtin_pattern(n as usize);
    }
    if n < 0
        && let Some(crate::banks::BankData::Images { images, .. }) = sprites.map(|b| &b.data)
        && let Some(img) = images.get((-n) as usize - 1)
        && !img.is_empty()
    {
        return Pattern::from_image(
            img.width_words as usize,
            img.height as usize,
            img.planes as usize,
            &img.planar,
        );
    }
    None
}

/// Draws a slider (`SliHor` / `SliVer`) on a screen: `x, y` top left
/// corner, `tx, ty` size in pixels minus one. Returns false if
/// `pos > total` (nothing drawn). The graphic state is saved and restored
/// (`Ec_Push`, `Ec_Pull`).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Screen,
    inks: &SliderInks,
    sprites: Option<&crate::banks::Bank>,
    vertical: bool,
    x: i32,
    y: i32,
    tx: i32,
    ty: i32,
    total: i32,
    pos: i32,
    size: i32,
) -> bool {
    if (pos as u16) > (total as u16) {
        return false;
    }
    let len = if vertical { ty } else { tx };
    let (off, knob) = pour(len, total, pos, size);
    let (x2, y2) = (x + tx, y + ty);
    let saved = s.gr.clone();
    let frame = (inks[0], inks[1], inks[2], inks[3]);
    let inner = (inks[4], inks[5], inks[6], inks[7]);
    let parts = if vertical {
        let a = y + off;
        let b = a + knob;
        [
            (x, y, x2, a, frame),
            (x, a, x2, b, inner),
            (x, b, x2, y2, frame),
        ]
    } else {
        let a = x + off;
        let b = a + knob;
        [
            (x, y, a, y2, frame),
            (a, y, b, y2, inner),
            (b, y, x2, y2, frame),
        ]
    };
    let (w, h, planes) = (s.width, s.height, s.planes);
    for (x0, y0, x1, y1, (ia, ib, ic, pat)) in parts {
        // SliDess: only when x2 > x1 and y2 > y1 (words, unsigned).
        if (x1 as u16) <= (x0 as u16) || (y1 as u16) <= (y0 as u16) {
            continue;
        }
        s.gr.ink = ia as u8;
        s.gr.paper = ib as u8;
        s.gr.outline = ic as u8;
        s.gr.line_pattern = 0xFFFF;
        s.gr.paint_outline = true;
        s.gr.pattern = pattern(pat, sprites);
        s.gr.pattern_number = pat;
        let li = s.logic;
        let mut c = Canvas::new(&mut s.bitmaps[li], w, h, planes);
        let mut g = s.gr.clone();
        g.bar(&mut c, x0, y0, x1, y1);
    }
    s.gr = saved;
    s.version += 1;
    true
}

impl Slider {
    /// `Sl_Draw`: draws with the inactive or active inks (setting the
    /// screen's slider inks, `SetSli`) and computes the knob limits.
    pub fn draw(
        &mut self,
        s: &mut Screen,
        screen_inks: &mut SliderInks,
        active: bool,
        sprites: Option<&crate::banks::Bank>,
    ) {
        let inks = if active { self.active } else { self.inactive };
        *screen_inks = inks;
        // Writing JAM2 for the slider (the caller restores Dia_Writing).
        s.gr.writing = crate::gfx::draw::JAM2;
        draw(
            s,
            &inks,
            sprites,
            self.vertical,
            self.x,
            self.y,
            self.sx,
            self.sy,
            self.global,
            self.position,
            self.window,
        );
        let len = if self.vertical { self.sy } else { self.sx };
        let (off, knob) = pour(len, self.global, self.position, self.window);
        self.mouse1 = off;
        self.mouse2 = (off + knob) as i16 as i32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_geometry() {
        // Half the range shown from the start.
        assert_eq!(pour(100, 10, 0, 5), (0, 50));
        // Knob at the end.
        assert_eq!(pour(100, 10, 5, 5), (50, 50));
        // Minimum size of 4 pixels.
        assert_eq!(pour(100, 1000, 0, 1), (0, 4));
        // Window larger than the total: full slider.
        assert_eq!(pour(64, 3, 0, 10), (0, 64));
    }

    #[test]
    fn draws_three_parts() {
        let mut s = Screen::new(0, 64, 16, 4, 0);
        let inks = [1, 1, 2, 0, 3, 3, 3, 0];
        assert!(draw(&mut s, &inks, None, false, 0, 0, 63, 7, 10, 0, 5,));
        let w = 64;
        // Knob (inks 3) at the left, frame (1 with outline 2) at the right.
        assert_eq!(s.bitmaps[0][3 * w + 10], 3);
        assert_eq!(s.bitmaps[0][3 * w + 50], 1);
        assert_eq!(s.bitmaps[0][50], 2);
        assert!(!draw(&mut s, &inks, None, false, 0, 0, 63, 7, 10, 11, 5));
    }
}
