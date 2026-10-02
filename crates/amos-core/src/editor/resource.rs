//! The editor resource bank `AMOSPro_Editor_Resource.Abk` (bank 16
//! "Resource"): 116 packed images (logo, buttons, window parts, sliders),
//! the screen palette and two Interface programs for the dialogs.
//!
//! ```text
//! +0 word 3 ; +2 long ofs_graphics ; +6 long ofs_texts ; +10 long ofs_programs
//! graphics: word nimg ; long ofs[nimg] ; word ncolours ; word mode ; palette
//! ```
//! Images are `Pac.Pic` packed bitmaps (`UnPack_Bitmap`, +Lib.s:25530).

use crate::gfx::Screen;
use crate::gfx::pack::{PackedBitmap, unpack_bitmap};

/// The resource bank shipped with AMOS Professional (fallback when the
/// file cannot be read).
pub static DEFAULT_RESOURCE: &[u8] =
    include_bytes!("../../../../AMOS-Professional-365/AMOS/APSystem/AMOSPro_Editor_Resource.Abk");

/// Image numbers (`+Edit.s:80-92`).
pub const ED_PICS: usize = 1;
pub const ED_BT_PICS: usize = ED_PICS + 4;
pub const ED_BOUTONS_PICS: usize = ED_BT_PICS + 2 * 3;
pub const ED_MEMORY_PICS: usize = ED_BOUTONS_PICS + 2 * 12;
pub const ES_PICS: usize = ED_MEMORY_PICS + 3;
pub const ES_BOUTONS_PICS: usize = ES_PICS + 3;

#[derive(Clone, Debug, Default)]
pub struct Resource {
    /// Packed images, 1-based in the original (`images[n - 1]`).
    pub images: Vec<Vec<u8>>,
    pub palette: Vec<u16>,
    /// Interface programs (dialog sources).
    pub programs: Vec<Vec<u8>>,
    /// The bank data (without the `AmBk` header), for the dialogs.
    pub raw: Vec<u8>,
}

fn rd16(d: &[u8], p: usize) -> usize {
    d.get(p..p + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as usize)
}

fn rd32(d: &[u8], p: usize) -> usize {
    d.get(p..p + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

impl Resource {
    /// Reads a resource bank from an `.Abk` file (or raw bank data).
    pub fn parse(data: &[u8]) -> Option<Resource> {
        let b = if data.starts_with(b"AmBk") { data.get(20..)? } else { data };
        let g = rd32(b, 2);
        let n = rd16(b, g);
        if g == 0 || n == 0 || n > 1000 {
            return None;
        }
        let mut offs: Vec<usize> = (0..n).map(|i| rd32(b, g + 2 + 4 * i)).collect();
        let pal = g + 2 + 4 * n;
        let ncol = rd16(b, pal).min(32);
        let palette = (0..ncol).map(|i| rd16(b, pal + 4 + 2 * i) as u16).collect();
        let progs = rd32(b, 10);
        let mut images = Vec::with_capacity(n);
        let mut sorted = offs.clone();
        sorted.sort_unstable();
        for o in offs.iter_mut() {
            if *o == 0 {
                images.push(Vec::new());
                continue;
            }
            // An image ends where the next one starts.
            let end = sorted.iter().copied().find(|&e| e > *o).map_or(b.len(), |e| g + e).min(b.len());
            images.push(b.get(g + *o..end).unwrap_or(&[]).to_vec());
        }
        let mut programs = Vec::new();
        if progs != 0 {
            let np = rd16(b, progs);
            let starts: Vec<usize> = (0..np).map(|i| progs + rd32(b, progs + 2 + 4 * i)).collect();
            // Each program: word length, then the Interface source.
            for &s in &starts {
                let n = rd16(b, s);
                programs.push(b.get(s + 2..s + 2 + n).unwrap_or(&[]).to_vec());
            }
        }
        Some(Resource { images, palette, programs, raw: b.to_vec() })
    }

    pub fn defaults() -> Resource {
        Resource::parse(DEFAULT_RESOURCE).expect("built-in editor resource")
    }

    /// Size in pixels of image `n` (1-based).
    pub fn image_size(&self, n: usize) -> Option<(usize, usize)> {
        let d = self.images.get(n.checked_sub(1)?)?;
        let (h, _) = PackedBitmap::read(d)?;
        Some((h.tx as usize * 8, h.ty as usize * h.tcar as usize))
    }

    /// `Ed_Unpack` (+Edit.s:13853): unpacks image `n` at (x, y) of the
    /// screen's logic bitmap (x rounded down to a byte). Images that do not
    /// fit are clipped by unpacking into a temporary buffer.
    pub fn draw(&self, screen: &mut Screen, n: usize, x: i32, y: i32) {
        let Some(d) = n.checked_sub(1).and_then(|i| self.images.get(i)) else { return };
        let Some((h, _)) = PackedBitmap::read(d) else { return };
        let (w, hh) = (h.tx as usize * 8, h.ty as usize * h.tcar as usize);
        let planes = h.nplan as usize;
        let mut buf = vec![0u8; w * hh];
        if unpack_bitmap(d, &mut buf, w, hh, planes, 0, 0).is_none() {
            return;
        }
        let (sw, sh) = (screen.width as i32, screen.height as i32);
        let mask = screen.colour_mask();
        let x = x & !7;
        let bm = screen.logic_mut();
        for row in 0..hh as i32 {
            let ty = y + row;
            if ty < 0 || ty >= sh {
                continue;
            }
            for col in 0..w as i32 {
                let tx = x + col;
                if tx < 0 || tx >= sw {
                    continue;
                }
                bm[(ty * sw + tx) as usize] = buf[(row as usize) * w + col as usize] & mask;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_editor_resource() {
        let r = Resource::defaults();
        assert_eq!(r.images.len(), 116);
        assert_eq!(r.palette.len(), 8);
        assert_eq!(r.image_size(ED_PICS), Some((160, 16)));
        assert_eq!(r.image_size(ED_BOUTONS_PICS), Some((32, 16)));
        assert_eq!(r.programs.len(), 2);
        assert!(r.programs[0].starts_with(b"LA"));
        let mut s = Screen::new(9, 640, 256, 8, 0x8000);
        r.draw(&mut s, ED_PICS, 32, 0);
        assert!(s.logic_ref()[..640 * 16].iter().any(|&p| p != 0));
    }
}
