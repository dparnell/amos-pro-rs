//! Low level drawing on the editor screens: 8x8 text with pen and paper,
//! rectangles and bitmap copies (the editor's text windows use the AMOS
//! window colour codes of `+Editor_Config.s:457-481`, reproduced here as
//! pen / paper pairs).

use crate::gfx::Screen;
use crate::gfx::font::glyph;

/// Colours of the default editor palette (`Ed_Palette`).
pub mod col {
    /// Text area paper and pen (system string 20: `B2 P3`).
    pub const TEXT_PAPER: u8 = 2;
    pub const TEXT_PEN: u8 = 3;
    /// Status line (strings 9-12: `B6 P7`).
    pub const STATUS_PAPER: u8 = 6;
    pub const STATUS_PEN: u8 = 7;
    /// Alerts (string 8: `B4 P3`).
    pub const ALERT_PAPER: u8 = 4;
    pub const ALERT_PEN: u8 = 3;
    /// Blocks (string 17: `B3 P2`).
    pub const BLOCK_PAPER: u8 = 3;
    pub const BLOCK_PEN: u8 = 2;
}

/// Fills a rectangle (x2, y2 exclusive), clipped to the screen.
pub fn fill(s: &mut Screen, x1: i32, y1: i32, x2: i32, y2: i32, c: u8) {
    let (w, h) = (s.width as i32, s.height as i32);
    let (x1, x2) = (x1.clamp(0, w), x2.clamp(0, w));
    let (y1, y2) = (y1.clamp(0, h), y2.clamp(0, h));
    if x1 >= x2 || y1 >= y2 {
        return;
    }
    let c = c & s.colour_mask();
    let bm = s.logic_mut();
    for y in y1..y2 {
        let o = (y * w) as usize;
        bm[o + x1 as usize..o + x2 as usize].fill(c);
    }
}

/// Draws text at pixel position (x, y): one 8x8 cell per character.
/// Returns the x after the text.
pub fn text(s: &mut Screen, x: i32, y: i32, t: &[u8], pen: u8, paper: u8) -> i32 {
    let (w, h) = (s.width as i32, s.height as i32);
    let mask = s.colour_mask();
    let (pen, paper) = (pen & mask, paper & mask);
    let bm = s.logic_mut();
    let mut cx = x;
    for &c in t {
        let g = glyph(c);
        for (row, bits) in g.iter().enumerate() {
            let py = y + row as i32;
            if py < 0 || py >= h {
                continue;
            }
            for bit in 0..8 {
                let px = cx + bit;
                if px < 0 || px >= w {
                    continue;
                }
                bm[(py * w + px) as usize] = if bits & (0x80 >> bit) != 0 { pen } else { paper };
            }
        }
        cx += 8;
    }
    cx
}

/// Text padded with spaces (or cut) to `n` characters.
pub fn text_n(s: &mut Screen, x: i32, y: i32, t: &[u8], n: usize, pen: u8, paper: u8) {
    let mut v: Vec<u8> = t.iter().copied().take(n).collect();
    v.resize(n, b' ');
    text(s, x, y, &v, pen, paper);
}

/// Copies a rectangle within the screen (`ScCopy`), the areas may overlap.
pub fn copy(s: &mut Screen, sx: i32, sy: i32, w: i32, h: i32, dx: i32, dy: i32) {
    let (sw, sh) = (s.width as i32, s.height as i32);
    let mut tmp = Vec::with_capacity((w.max(0) * h.max(0)) as usize);
    for y in 0..h {
        for x in 0..w {
            tmp.push(s.pixel(sx + x, sy + y).unwrap_or(0));
        }
    }
    let bm = s.logic_mut();
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (dx + x, dy + y);
            if px >= 0 && py >= 0 && px < sw && py < sh {
                bm[(py * sw + px) as usize] = tmp[(y * w + x) as usize];
            }
        }
    }
}

/// Outline rectangle (x2, y2 inclusive).
pub fn frame(s: &mut Screen, x1: i32, y1: i32, x2: i32, y2: i32, c: u8) {
    fill(s, x1, y1, x2 + 1, y1 + 1, c);
    fill(s, x1, y2, x2 + 1, y2 + 1, c);
    fill(s, x1, y1, x1 + 1, y2 + 1, c);
    fill(s, x2, y1, x2 + 1, y2 + 1, c);
}

/// Swaps two colours in a rectangle (used for highlighting).
pub fn invert(s: &mut Screen, x1: i32, y1: i32, x2: i32, y2: i32, a: u8, b: u8) {
    let (w, h) = (s.width as i32, s.height as i32);
    let bm = s.logic_mut();
    for y in y1.max(0)..y2.min(h) {
        for x in x1.max(0)..x2.min(w) {
            let p = &mut bm[(y * w + x) as usize];
            if *p == a {
                *p = b;
            } else if *p == b {
                *p = a;
            }
        }
    }
}

/// Decimal number left aligned in `n` characters (`Et_Chiffre`).
pub fn number(v: i64, n: usize) -> Vec<u8> {
    let mut s = v.to_string().into_bytes();
    s.resize(n, b' ');
    s.truncate(n);
    s
}
