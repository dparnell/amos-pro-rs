//! Bobs: software sprites drawn into the screen bitmaps with background
//! save and restore (`BobSet`, `BobAct`, `BobEff`, `BobAff`, `BobCalc` in
//! `+W.s` 803-2250).
//!
//! The blitter operations are emulated per pixel on the chunky bitmaps:
//! for every written plane `p`, `D = LF(A = mask, B = image plane p,
//! C = screen plane p)`, over the word aligned area covered by the blit.

use std::collections::BTreeMap;

use super::images::{self, FLIP_MASK, MaskState};
use crate::banks::Image;

/// Maximum number of bobs (`T_BbMax`, `+Interpreter_Config.s`).
pub const BOB_MAX: u16 = 68;

/// Default minterm (cookie cut: `D = A.B + !A.C`).
pub const LF_COOKIE: u8 = 0xCA;

/// Clipping rectangle (`BbLimG/H/D/B`); right and bottom are exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Blit parameters computed by `BobCalc`.
#[derive(Clone, Debug, Default)]
pub struct Calc {
    /// Top left of the image (hot spot subtracted).
    pub x: i32,
    pub y: i32,
    /// Image number (1-based, without flip flags) and flip flags.
    pub image: u16,
    pub flags: u16,
    /// Covered area: word aligned horizontally, clipped.
    pub rx: i32,
    pub ry: i32,
    pub rw: i32,
    pub rh: i32,
    /// Number of planes drawn (min(image, screen)).
    pub nplanes: u8,
    /// Minterm and use of the mask.
    pub lf: u8,
    pub masked: bool,
}

/// One background save buffer (`Decor` slot).
#[derive(Clone, Debug, Default)]
pub struct SaveSlot {
    /// `BbDASize != 0`: the slot describes an area (kept after a restore).
    pub valid: bool,
    pub rx: i32,
    pub ry: i32,
    pub rw: i32,
    pub rh: i32,
    /// Bits of the planes saved / restored.
    pub planes: u8,
    /// Saved pixels (None when the background is erased with a colour).
    pub data: Option<Vec<u8>>,
}

/// One bob (`BbLong` structure).
#[derive(Clone, Debug, Default)]
pub struct Bob {
    pub number: u16,
    /// Change flags: bit 0 image, 1 X, 2 Y; negative: `Bob Off` pending.
    pub act: i8,
    pub x: i16,
    pub y: i16,
    /// Image with flip flags.
    pub image: u16,
    /// Screen the bob belongs to.
    pub screen: usize,
    /// `Set Bob` planes (bit mask of written planes).
    pub planes: u16,
    /// `Set Bob` minterm: 0 = default, else bit 15 + minterm.
    pub minterm: u16,
    pub lim: Limits,
    /// Number of background buffers: 0, 1, or 2 (double buffered screen).
    pub decor: i16,
    /// `Set Bob` background mode: 0 save, >0 fill with colour-1, <0 none.
    pub eff: i16,
    /// Current and previous decor slot (`BbDCur1`, `BbDCur2`).
    pub dcur1: usize,
    pub dcur2: usize,
    pub slots: [SaveSlot; 2],
    /// Saves still to do (`BbDCpt`) and `Put Bob` skip counter (`BbECpt`).
    pub dcpt: i16,
    pub ecpt: i16,
    pub calc: Option<Calc>,
}

impl Bob {
    /// `ResBOB`: a new bob on `screen` (size `w` x `h`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        number: u16,
        screen: usize,
        w: u32,
        h: u32,
        double: bool,
        back: i32,
        planes: i32,
        minterm: i32,
    ) -> Bob {
        let mut decor = 1;
        let mut dcur2 = 0;
        if double {
            decor = 2;
            dcur2 = 1;
        }
        if back < 0 {
            decor = 0;
        }
        let m = (minterm & 0xFF) as u16;
        Bob {
            number,
            screen,
            planes: planes as u16,
            minterm: if m != 0 { m | 0x8000 } else { 0 },
            lim: Limits {
                left: 0,
                top: 0,
                right: w as i32,
                bottom: h as i32,
            },
            decor,
            eff: back as i16,
            dcur2,
            ..Default::default()
        }
    }
}

/// All bobs, sorted by number (`T_BbDeb` list).
#[derive(Debug, Default)]
pub struct Bobs {
    pub list: BTreeMap<u16, Bob>,
    /// `Priority On`: screen whose bobs are sorted by Y (`T_Priorite`).
    pub priority: Option<usize>,
    /// `Priority Reverse On`.
    pub reverse: bool,
    /// Draw order computed by the last `BobAct`.
    pub order: Vec<u16>,
}

/// Target bitmap of a draw.
pub struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
    pub planes: u8,
}

/// Image data ready for drawing: chunky pixels and mask.
pub struct Src {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}

impl Src {
    pub fn new(img: &Image, flags: u16) -> Src {
        Src {
            w: img.width(),
            h: img.height as u32,
            pixels: images::chunky(img, flags),
        }
    }
}

/// `BobCalc`: computes the covered area of an image drawn with its top
/// left corner at `x`,`y`. None when completely clipped.
#[allow(clippy::too_many_arguments)]
pub fn calc(
    img: &Image,
    image: u16,
    flags: u16,
    x: i32,
    y: i32,
    lim: Limits,
    screen: (u32, u32, u8),
    minterm: u16,
    mask: MaskState,
) -> Option<Calc> {
    if img.is_empty() {
        return None;
    }
    let (sw, sh, splanes) = screen;
    let h = img.height as i32;
    let rx0 = x.div_euclid(16) * 16;
    let words = img.width_words as i32 + if x.rem_euclid(16) != 0 { 1 } else { 0 };
    let rx1 = rx0 + words * 16;
    let left = lim.left.max(0).max(rx0);
    let right = lim.right.min(sw as i32).min(rx1);
    let top = lim.top.max(0).max(y);
    let bottom = lim.bottom.min(sh as i32).min(y + h);
    if left >= right || top >= bottom {
        return None;
    }
    // A clipped area that does not show any image column is still drawn by
    // the blitter (word granularity), as in the original.
    let masked = mask != MaskState::None;
    let lf = if minterm & 0x8000 != 0 {
        minterm as u8
    } else {
        LF_COOKIE
    };
    Some(Calc {
        x,
        y,
        image,
        flags,
        rx: left,
        ry: top,
        rw: right - left,
        rh: bottom - top,
        nplanes: (img.planes as u8).min(splanes),
        lf,
        masked,
    })
}

/// Draws an image according to `c` (`BbAp`, `BbA16`, `BMA16`...).
pub fn draw(t: &mut Target, src: &Src, c: &Calc, plane_mask: u16) {
    let pm = (plane_mask as u32 & ((1u32 << c.nplanes) - 1)) as u8;
    if pm == 0 {
        return;
    }
    // The minterm applied to all planes at once: for each pixel, bit p of
    // the result is bit (A<<2 | S_p<<1 | D_p) of `lf`, A being the mask.
    let terms = |a: u8| -> [u8; 4] {
        std::array::from_fn(|sd| {
            if (c.lf >> ((a << 2) | sd as u8)) & 1 != 0 {
                0xFF
            } else {
                0
            }
        })
    };
    let (t0, t1) = (terms(0), terms(1));
    for py in c.ry..c.ry + c.rh {
        let iy = py - c.y;
        let row_in = iy >= 0 && (iy as u32) < src.h;
        let row = (py as u32 * t.width) as usize;
        for px in c.rx..c.rx + c.rw {
            let ix = px - c.x;
            let inside = row_in && ix >= 0 && (ix as u32) < src.w;
            let s = if inside {
                src.pixels[(iy as u32 * src.w + ix as u32) as usize]
            } else {
                0
            };
            let a = inside && (!c.masked || s != 0);
            let tm = if a { &t1 } else { &t0 };
            let o = row + px as usize;
            let d = t.pixels[o];
            let res = (tm[3] & s & d) | (tm[2] & s & !d) | (tm[1] & !s & d) | (tm[0] & !s & !d);
            t.pixels[o] = (d & !pm) | (res & pm);
        }
    }
}

/// Saves the background covered by `c` (`BobAff` saisie).
pub fn save(t: &Target, slot: &mut SaveSlot) {
    let mut data = Vec::with_capacity((slot.rw * slot.rh).max(0) as usize);
    for y in slot.ry..slot.ry + slot.rh {
        let o = (y as u32 * t.width) as usize;
        data.extend_from_slice(&t.pixels[o + slot.rx as usize..o + (slot.rx + slot.rw) as usize]);
    }
    slot.data = Some(data);
}

/// Restores a background (`BobEff`), or fills it with colour `fill`.
pub fn restore(t: &mut Target, slot: &SaveSlot, fill: Option<u8>) {
    let pm = slot.planes;
    let mut i = 0;
    for y in slot.ry..slot.ry + slot.rh {
        let o = (y as u32 * t.width) as usize;
        for x in slot.rx..slot.rx + slot.rw {
            let d = &mut t.pixels[o + x as usize];
            let v = match (fill, &slot.data) {
                (Some(c), _) => c,
                (None, Some(data)) => data[i],
                (None, None) => *d,
            };
            *d = (*d & !pm) | (v & pm);
            i += 1;
        }
    }
}

/// Mask of the image pixel at (x, y) relative to its top left.
pub fn mask_at(src: &Src, x: i32, y: i32) -> bool {
    x >= 0
        && y >= 0
        && (x as u32) < src.w
        && (y as u32) < src.h
        && src.pixels[(y as u32 * src.w + x as u32) as usize] != 0
}

/// Pixel collision of two images placed at (ax, ay) and (bx, by) (top
/// left corners), `ColRout`.
pub fn collide(a: &Src, ax: i32, ay: i32, b: &Src, bx: i32, by: i32) -> bool {
    let x0 = ax.max(bx);
    let x1 = (ax + a.w as i32).min(bx + b.w as i32);
    let y0 = ay.max(by);
    let y1 = (ay + a.h as i32).min(by + b.h as i32);
    if x0 >= x1 || y0 >= y1 {
        return false;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            if mask_at(a, x - ax, y - ay) && mask_at(b, x - bx, y - by) {
                return true;
            }
        }
    }
    false
}

/// Collision view of a bank image shown with flip `flags`: reads the mask
/// straight from the planar data, so a collision test costs nothing when
/// the rectangles do not overlap (same result as [`collide`] on
/// `Src::new(img, flags)`).
pub struct ColImg<'a> {
    img: &'a Image,
    flags: u16,
    pub w: u32,
    pub h: u32,
}

impl<'a> ColImg<'a> {
    pub fn new(img: &'a Image, flags: u16) -> Self {
        ColImg {
            img,
            flags,
            w: img.width(),
            h: img.height as u32,
        }
    }

    /// Pixel (x, y) of the flipped image is not colour 0.
    #[inline]
    fn mask_at(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as u32 >= self.w || y as u32 >= self.h {
            return false;
        }
        let (mut x, mut y) = (x as u32, y as u32);
        if self.flags & images::FLIP_X != 0 {
            x = self.w - 1 - x;
        }
        if self.flags & images::FLIP_Y != 0 {
            y = self.h - 1 - y;
        }
        self.img.pixel(x, y) != 0
    }
}

/// [`collide`] for [`ColImg`]s.
pub fn collide_img(a: &ColImg, ax: i32, ay: i32, b: &ColImg, bx: i32, by: i32) -> bool {
    let x0 = ax.max(bx);
    let x1 = (ax + a.w as i32).min(bx + b.w as i32);
    let y0 = ay.max(by);
    let y1 = (ay + a.h as i32).min(by + b.h as i32);
    if x0 >= x1 || y0 >= y1 {
        return false;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            if a.mask_at(x - ax, y - ay) && b.mask_at(x - bx, y - by) {
                return true;
            }
        }
    }
    false
}

/// Image number without flags.
pub fn index(image: u16) -> u16 {
    image & !FLIP_MASK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::images::grab;

    fn img() -> Image {
        // 4x2 image: 1 2 0 3 / 0 1 1 0, two planes
        grab(&[1, 2, 0, 3, 0, 1, 1, 0], 4, 0, 0, 4, 2, 2)
    }

    /// The plane by plane minterm (reference for `draw`).
    fn draw_reference(t: &mut Target, src: &Src, c: &Calc, plane_mask: u16) {
        let pm = (plane_mask as u32 & ((1u32 << c.nplanes) - 1)) as u8;
        if pm == 0 {
            return;
        }
        for py in c.ry..c.ry + c.rh {
            let iy = py - c.y;
            for px in c.rx..c.rx + c.rw {
                let ix = px - c.x;
                let inside = ix >= 0 && iy >= 0 && (ix as u32) < src.w && (iy as u32) < src.h;
                let s = if inside {
                    src.pixels[(iy as u32 * src.w + ix as u32) as usize]
                } else {
                    0
                };
                let a = inside && (!c.masked || s != 0);
                let o = (py as u32 * t.width + px as u32) as usize;
                let d = t.pixels[o];
                let mut out = d;
                for p in 0..8 {
                    if pm & (1 << p) == 0 {
                        continue;
                    }
                    let idx = ((a as u8) << 2) | (((s >> p) & 1) << 1) | ((d >> p) & 1);
                    let bit = (c.lf >> idx) & 1;
                    out = (out & !(1 << p)) | (bit << p);
                }
                t.pixels[o] = out;
            }
        }
    }

    #[test]
    fn draw_matches_plane_by_plane_reference() {
        let mut seed = 99u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 16) as u8
        };
        let bm: Vec<u8> = (0..20 * 9).map(|_| rnd() & 31).collect();
        let im = grab(&bm, 20, 0, 0, 20, 9, 5);
        for lf in 0..=255u8 {
            for (x, y, masked, pmask) in [
                (3, 2, true, 0xFFFF),
                (-5, -3, false, 0x15),
                (17, 30, true, 0x3),
            ] {
                let c = Calc {
                    x,
                    y,
                    image: 1,
                    flags: 0,
                    rx: x.div_euclid(16) * 16,
                    ry: y.max(0),
                    rw: 32,
                    rh: 9,
                    nplanes: 5,
                    lf,
                    masked,
                };
                let c = Calc {
                    rx: c.rx.max(0),
                    ..c
                };
                let px: Vec<u8> = (0..48 * 40).map(|_| rnd()).collect();
                let (mut a, mut b) = (px.clone(), px);
                let src = Src::new(&im, 0);
                draw(
                    &mut Target {
                        pixels: &mut a,
                        width: 48,
                        height: 40,
                        planes: 5,
                    },
                    &src,
                    &c,
                    pmask,
                );
                draw_reference(
                    &mut Target {
                        pixels: &mut b,
                        width: 48,
                        height: 40,
                        planes: 5,
                    },
                    &src,
                    &c,
                    pmask,
                );
                assert_eq!(a, b, "minterm {lf:02x}");
            }
        }
    }

    #[test]
    fn cookie_cut_and_clip() {
        let im = img();
        let mut px = vec![4u8; 32 * 8];
        let mut t = Target {
            pixels: &mut px,
            width: 32,
            height: 8,
            planes: 3,
        };
        let lim = Limits {
            left: 0,
            top: 0,
            right: 32,
            bottom: 8,
        };
        let c = calc(&im, 1, 0, 3, 1, lim, (32, 8, 3), 0, MaskState::Made).unwrap();
        assert_eq!((c.rx, c.rw, c.ry, c.rh), (0, 32, 1, 2));
        let mut slot = SaveSlot {
            valid: true,
            rx: c.rx,
            ry: c.ry,
            rw: c.rw,
            rh: c.rh,
            planes: 3,
            data: None,
        };
        save(&t, &mut slot);
        draw(&mut t, &Src::new(&im, 0), &c, 0xFFFF);
        // Plane 2 untouched (image has 2 planes): colour 4 keeps bit 2.
        assert_eq!(t.pixels[32 + 3], 5);
        assert_eq!(t.pixels[32 + 4], 6);
        assert_eq!(t.pixels[32 + 5], 4);
        assert_eq!(t.pixels[32 + 6], 7);
        restore(&mut t, &slot, None);
        assert!(t.pixels.iter().all(|&p| p == 4));
        // No mask: opaque rectangle.
        let c = calc(&im, 1, 0, 3, 1, lim, (32, 8, 3), 0, MaskState::None).unwrap();
        draw(&mut t, &Src::new(&im, 0), &c, 0xFFFF);
        assert_eq!(t.pixels[32 + 5], 4);
        assert_eq!(t.pixels[32 + 2], 4);
        // Fully clipped.
        assert!(calc(&im, 1, 0, 40, 1, lim, (32, 8, 3), 0, MaskState::Made).is_none());
    }

    #[test]
    fn collisions() {
        let im = img();
        let s = Src::new(&im, 0);
        assert!(collide(&s, 0, 0, &s, 1, 0));
        assert!(!collide(&s, 0, 0, &s, 3, 1));
        assert!(!collide(&s, 0, 0, &s, 20, 0));
        // The planar view gives the same answers, flipped or not.
        for fa in [0, images::FLIP_X, images::FLIP_Y, FLIP_MASK] {
            for fb in [0, images::FLIP_X, images::FLIP_Y, FLIP_MASK] {
                let (sa, sb) = (Src::new(&im, fa), Src::new(&im, fb));
                let (ca, cb) = (ColImg::new(&im, fa), ColImg::new(&im, fb));
                for dx in -20..20 {
                    for dy in -4..4 {
                        assert_eq!(
                            collide(&sa, 0, 0, &sb, dx, dy),
                            collide_img(&ca, 0, 0, &cb, dx, dy)
                        );
                    }
                }
            }
        }
    }
}
