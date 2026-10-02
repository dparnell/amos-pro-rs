//! Sprite and icon bank images: hot spots, flipping, masks and grabbing
//! (`SpotH`, `Retourne`, `Masque`, `GetBob` in `+W.s`).
//!
//! Images stay planar in the bank ([`crate::banks::Image`]). The original
//! flips bank images in place when a flipped bob is drawn and keeps the
//! current orientation in bits 15/14 of the hot spot X; here the bank always
//! holds the unflipped image and flipped images are produced on the fly,
//! which gives the same picture.

use std::sync::OnceLock;

use crate::banks::{Bank, BankData, Image, parse_banks};

/// Image number flags (`Hrev`, `Vrev`, `Rev`).
pub const FLIP_X: u16 = 0x8000;
pub const FLIP_Y: u16 = 0x4000;
pub const FLIP_MASK: u16 = 0xC000;

/// Mask state of a bank image (the mask pointer of `T_SprBank`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MaskState {
    /// Not computed yet (pointer 0): computed by the first bob draw or
    /// `Make Mask`. Collisions are not possible.
    #[default]
    Lazy,
    /// Computed: colour 0 is transparent.
    Made,
    /// `No Mask` ($C0000000): drawn as an opaque rectangle.
    None,
}

/// Hot spot X: 14 bit signed value (bits 15/14 are flip flags).
pub fn hot_x(img: &Image) -> i32 {
    (((img.hot_x as u16) << 2) as i16 >> 2) as i32
}

pub fn hot_y(img: &Image) -> i32 {
    img.hot_y as i32
}

/// Hot spot of the image shown with flip `flags` (`BobCalc` 1369-1386,
/// `RBobX` / `RBobY`).
pub fn flipped_hot(img: &Image, flags: u16) -> (i32, i32) {
    let mut hx = hot_x(img);
    let mut hy = hot_y(img);
    if flags & FLIP_X != 0 {
        hx = img.width() as i32 - hx;
    }
    if flags & FLIP_Y != 0 {
        hy = img.height as i32 - hy;
    }
    (hx, hy)
}

/// One byte per pixel copy of an image, flipped according to `flags`.
pub fn chunky(img: &Image, flags: u16) -> Vec<u8> {
    let w = img.width() as usize;
    let h = img.height as usize;
    let src = img.to_chunky();
    if flags & FLIP_MASK == 0 {
        return src;
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        let sy = if flags & FLIP_Y != 0 { h - 1 - y } else { y };
        for x in 0..w {
            let sx = if flags & FLIP_X != 0 { w - 1 - x } else { x };
            out[y * w + x] = src[sy * w + sx];
        }
    }
    out
}

/// `Hot Spot n,x,y` (mode 0) or `Hot Spot n,$xy` (mode = (p & $77) + 1):
/// nibbles 0 = left/top, 1 = centre, 2 = right/bottom.
pub fn set_hot_spot(img: &mut Image, mode: i32, x: i32, y: i32) {
    let (mut hx, mut hy) = (x, y);
    if mode != 0 {
        let m = mode - 1;
        let w = img.width() as i32;
        let h = img.height as i32;
        hx = match (m >> 4) & 3 {
            0 => 0,
            1 => w / 2,
            _ => w,
        };
        hy = match m & 3 {
            0 => 0,
            1 => h / 2,
            _ => h,
        };
    }
    img.hot_x = (hx & 0x3FFF) as i16;
    img.hot_y = hy as i16;
}

/// `GetBob`: grabs a rectangle of a chunky bitmap into a new image with
/// `planes` planes and hot spot 0,0 (pixels right of the width are 0).
pub fn grab(bitmap: &[u8], bw: u32, x: i32, y: i32, w: u32, h: u32, planes: u8) -> Image {
    let words = w.div_ceil(16) as u16;
    let row_bytes = words as usize * 2;
    let plane_bytes = row_bytes * h as usize;
    let mut planar = vec![0u8; plane_bytes * planes as usize];
    let bh = bitmap.len() as u32 / bw.max(1);
    for yy in 0..h {
        for xx in 0..w {
            let sx = x + xx as i32;
            let sy = y + yy as i32;
            if sx < 0 || sy < 0 || sx as u32 >= bw || sy as u32 >= bh {
                continue;
            }
            let c = bitmap[(sy as u32 * bw + sx as u32) as usize];
            let byte = yy as usize * row_bytes + (xx / 8) as usize;
            let bit = 7 - (xx % 8);
            for p in 0..planes as usize {
                if c & (1 << p) != 0 {
                    planar[p * plane_bytes + byte] |= 1 << bit;
                }
            }
        }
    }
    Image {
        width_words: words,
        height: h as u16,
        planes: planes as u16,
        hot_x: 0,
        hot_y: 0,
        planar,
    }
}

/// The default mouse bank (`+AMOSPro_Mouse.abk`): images 1-3 are the mouse
/// pointers, its palette gives the default colours 16-31.
pub fn mouse_bank() -> &'static (Vec<Image>, [u16; 32]) {
    static BANK: OnceLock<(Vec<Image>, [u16; 32])> = OnceLock::new();
    BANK.get_or_init(|| {
        let data = include_bytes!("../../../../AMOS-Professional-365/bin/+AMOSPro_Mouse.abk");
        match parse_banks(data)
            .ok()
            .and_then(|b| b.into_iter().next())
            .map(|b| b.data)
        {
            Some(BankData::Images { images, palette }) => (images, palette),
            _ => (Vec::new(), [0; 32]),
        }
    })
}

/// A new empty sprite (1) or icon (2) bank.
pub fn new_image_bank(number: u16) -> Bank {
    Bank {
        number,
        name: if number == 2 {
            "Icons   ".into()
        } else {
            "Sprites ".into()
        },
        chip: true,
        data_bank: true,
        data: BankData::Images {
            images: Vec::new(),
            palette: [0; 32],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_bank_loads() {
        let (imgs, pal) = mouse_bank();
        assert!(imgs.len() >= 3);
        assert_eq!(imgs[0].width_words, 1);
        assert_eq!(imgs[0].planes, 2);
        assert_ne!(pal[17], 0);
    }

    #[test]
    fn grab_and_flip() {
        // 3x2 bitmap: colours 1 2 3 / 0 0 5
        let bm = [1u8, 2, 3, 0, 0, 5];
        let img = grab(&bm, 3, 0, 0, 3, 2, 3);
        assert_eq!(img.width(), 16);
        assert_eq!(img.pixel(1, 0), 2);
        assert_eq!(img.pixel(2, 1), 5);
        assert_eq!(img.pixel(3, 0), 0);
        let f = chunky(&img, FLIP_X);
        assert_eq!(f[15], 1);
        assert_eq!(f[13], 3);
        let f = chunky(&img, FLIP_Y);
        assert_eq!(f[2], 5);
    }

    #[test]
    fn hot_spots() {
        let mut img = grab(&[1u8; 32 * 10], 32, 0, 0, 32, 10, 1);
        set_hot_spot(&mut img, 0x11 + 1, 0, 0);
        assert_eq!((hot_x(&img), hot_y(&img)), (16, 5));
        set_hot_spot(&mut img, 0, -3, 2);
        assert_eq!(hot_x(&img), -3);
        assert_eq!(flipped_hot(&img, FLIP_X | FLIP_Y), (35, 8));
    }
}
