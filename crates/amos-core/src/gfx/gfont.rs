//! Font used by graphic `Text`: the default system font, topaz 8.
//!
//! The ROM font is not part of the sources; the glyphs come from the
//! redrawn Topaz-style 8x8 font in `assets/font8x8.txt` (the same one the
//! text windows use). Glyph rows are top to bottom, bit 7 = leftmost pixel,
//! with the baseline on row 6 (`Wi_MakeFonte` renders the ROM font there).

use std::sync::OnceLock;

pub const WIDTH: i32 = 8;
pub const HEIGHT: i32 = 8;
/// `Text Base` (rp->TxBaseline of topaz 8).
pub const BASELINE: i32 = 6;

static FONT_TEXT: &str = include_str!("../../assets/font8x8.txt");

fn font() -> &'static [[u8; 8]; 256] {
    static FONT: OnceLock<Box<[[u8; 8]; 256]>> = OnceLock::new();
    FONT.get_or_init(|| {
        let mut f = Box::new([[0u8; 8]; 256]);
        let mut lines = FONT_TEXT
            .lines()
            .filter(|l| !l.starts_with("# ") && !l.trim().is_empty());
        while let Some(code) = lines.next() {
            let Ok(c) = u8::from_str_radix(code.trim(), 16) else {
                continue;
            };
            for row in 0..8 {
                let Some(l) = lines.next() else { break };
                let mut v = 0u8;
                for (i, ch) in l.bytes().take(8).enumerate() {
                    if ch == b'#' {
                        v |= 0x80 >> i;
                    }
                }
                f[c as usize][row] = v;
            }
        }
        f
    })
}

/// The 8 rows of a character.
pub fn glyph(c: u8) -> [u8; 8] {
    font()[c as usize]
}

/// `Text Length`: width of a string in pixels.
pub fn text_length(s: &[u8]) -> i32 {
    s.len() as i32 * WIDTH
}

#[cfg(test)]
mod tests {
    #[test]
    fn has_letters() {
        assert_ne!(super::glyph(b'A'), [0; 8]);
        assert_eq!(super::glyph(b' '), [0; 8]);
    }
}
