//! The 8x8 text font (`T_JeuDefo`, built by `Wi_MakeFonte`, +W.s:9565).
//!
//! The original renders characters 32-127 and 160-255 with the Topaz 8 ROM
//! font, then copies `bin/+WFont.bin` over characters 0-31 and 128-159
//! (box drawing and gadget glyphs). Topaz is not part of the sources, so
//! `assets/font8x8.txt` holds a re-drawn substitute; Latin-1 accented letters
//! it does not define are composed here from the base letter and an accent.
//!
//! Each glyph is 8 bytes, top row first, bit 7 = leftmost pixel.

use std::sync::OnceLock;

pub type Glyph = [u8; 8];

static WFONT: &[u8; 512] = include_bytes!("../../../../AMOS-Professional-365/bin/+WFont.bin");
static FONT_TXT: &str = include_str!("../../assets/font8x8.txt");

/// The 256 character font.
pub fn font() -> &'static [Glyph; 256] {
    static FONT: OnceLock<[Glyph; 256]> = OnceLock::new();
    FONT.get_or_init(build_font)
}

/// Glyph of character `c`.
pub fn glyph(c: u8) -> &'static Glyph {
    &font()[c as usize]
}

/// Parses the text font file: a line with the hex code, then 8 rows of
/// `.`/`#`. Lines starting with `# ` are comments.
fn parse_txt(txt: &str) -> Vec<(u8, Glyph)> {
    let mut out = Vec::new();
    let mut lines = txt
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("# "));
    while let Some(code) = lines.next() {
        let Ok(c) = u8::from_str_radix(code, 16) else {
            continue;
        };
        let mut g = [0u8; 8];
        for row in g.iter_mut() {
            let line = lines.next().unwrap_or("");
            for (i, ch) in line.bytes().take(8).enumerate() {
                if ch == b'#' {
                    *row |= 0x80 >> i;
                }
            }
        }
        out.push((c, g));
    }
    out
}

fn build_font() -> [Glyph; 256] {
    let mut f = [[0u8; 8]; 256];
    let mut defined = [false; 256];
    for (c, g) in parse_txt(FONT_TXT) {
        f[c as usize] = g;
        defined[c as usize] = true;
    }
    for c in 0..32 {
        f[c].copy_from_slice(&WFONT[c * 8..c * 8 + 8]);
        f[128 + c].copy_from_slice(&WFONT[256 + c * 8..256 + c * 8 + 8]);
        defined[c] = true;
        defined[128 + c] = true;
    }
    for c in 160..256usize {
        if !defined[c]
            && let Some(g) = compose(&f, c as u8)
        {
            f[c] = g;
        }
    }
    f
}

#[derive(Clone, Copy)]
enum Accent {
    Grave,
    Acute,
    Circumflex,
    Tilde,
    Diaeresis,
    Ring,
    Cedilla,
}

/// Two accent rows, placed above the letter.
fn accent_rows(a: Accent) -> [u8; 2] {
    match a {
        Accent::Grave => [0b0011_0000, 0b0001_1000],
        Accent::Acute => [0b0000_1100, 0b0001_1000],
        Accent::Circumflex => [0b0011_1100, 0b0110_0110],
        Accent::Tilde => [0b0111_0110, 0b1101_1100],
        Accent::Diaeresis => [0b0110_0110, 0],
        Accent::Ring => [0b0001_1000, 0b0010_0100],
        Accent::Cedilla => [0, 0],
    }
}

/// Base letter and accent of a Latin-1 accented letter.
fn decompose(c: u8) -> Option<(u8, Accent)> {
    use Accent::*;
    const ACC6: [Accent; 6] = [Grave, Acute, Circumflex, Tilde, Diaeresis, Ring];
    const ACC4: [Accent; 4] = [Grave, Acute, Circumflex, Diaeresis];
    const ACC5: [Accent; 5] = [Grave, Acute, Circumflex, Tilde, Diaeresis];
    Some(match c {
        0xC0..=0xC5 => (b'A', ACC6[(c - 0xC0) as usize]),
        0xC7 => (b'C', Cedilla),
        0xC8..=0xCB => (b'E', ACC4[(c - 0xC8) as usize]),
        0xCC..=0xCF => (b'I', ACC4[(c - 0xCC) as usize]),
        0xD1 => (b'N', Tilde),
        0xD2..=0xD6 => (b'O', ACC5[(c - 0xD2) as usize]),
        0xD9..=0xDC => (b'U', ACC4[(c - 0xD9) as usize]),
        0xDD => (b'Y', Acute),
        0xE0..=0xE5 => (b'a', ACC6[(c - 0xE0) as usize]),
        0xE7 => (b'c', Cedilla),
        0xE8..=0xEB => (b'e', ACC4[(c - 0xE8) as usize]),
        0xEC..=0xEF => (b'i', ACC4[(c - 0xEC) as usize]),
        0xF1 => (b'n', Tilde),
        0xF2..=0xF6 => (b'o', ACC5[(c - 0xF2) as usize]),
        0xF9..=0xFC => (b'u', ACC4[(c - 0xF9) as usize]),
        0xFD => (b'y', Acute),
        0xFF => (b'y', Diaeresis),
        _ => return None,
    })
}

/// Builds an accented letter. Lower case letters have two free rows above
/// them; capitals (rows 0-6) are squeezed into rows 2-6 by dropping
/// repeated rows, as Topaz does.
fn compose(f: &[Glyph; 256], c: u8) -> Option<Glyph> {
    let (base, accent) = decompose(c)?;
    let mut g = f[base as usize];
    if let Accent::Cedilla = accent {
        g[7] = 0b0001_1000;
        if base.is_ascii_lowercase() {
            g[7] = 0b0000_1100;
        }
        return Some(g);
    }
    let acc = accent_rows(accent);
    if base.is_ascii_uppercase() {
        let mut rows: Vec<u8> = g[..7].to_vec();
        while rows.len() > 5 {
            // Remove the last row equal to the one above it, else row 1.
            let dup = (1..rows.len()).rev().find(|&i| rows[i] == rows[i - 1]);
            rows.remove(dup.unwrap_or(1));
        }
        let mut out = [0u8; 8];
        out[0] = acc[0];
        out[1] = acc[1];
        out[2..7].copy_from_slice(&rows);
        out[7] = g[7];
        Some(out)
    } else {
        if base == b'i' {
            // Dotless i.
            g[0] = 0;
            g[1] = 0;
        }
        g[0] = acc[0];
        g[1] = acc[1];
        Some(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_has_all_glyphs() {
        let f = font();
        assert_eq!(f[0], [0; 8]);
        assert_eq!(f[b' ' as usize], [0; 8]);
        assert_ne!(f[b'A' as usize], [0; 8]);
        // Box drawing from +WFont.bin.
        assert_eq!(f[136][3], 0x0F);
        // Every printable Latin-1 character except NBSP has pixels.
        for c in (33..127).chain(161..256) {
            assert_ne!(f[c], [0; 8], "char {c}");
        }
        // Accented letters keep their base letter shape.
        assert_eq!(f[0xE4][2..], f[b'a' as usize][2..]);
    }
}
