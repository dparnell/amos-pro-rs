//! Blocks (`Get Block`, `Get Cblock`...), Screen Copy, Zoom and Appear.
//!
//! Bitmaps are chunky (see `screen.rs`); the planar operations of the
//! original (blitter minterms, plane masks, byte aligned compressed blocks)
//! are done on the bits of each pixel.


/// Error numbers returned by the block routines (`EcWiErr` codes: 44 +
/// screen library error).
pub const BLOCK_NOT_DEFINED: u16 = 46;
pub const BLOCK_NOT_FOUND: u16 = 65;
pub const ILLEGAL_BLOCK_PARAMETERS: u16 = 66;

/// A normal block (`MakeBloc` `+W.s:12346`): an image grabbed from a
/// screen, whole 16 pixel words wide.
#[derive(Clone, Debug)]
pub struct Block {
    pub number: i32,
    /// Position it was grabbed from (default Put Block position).
    pub x: i32,
    pub y: i32,
    /// Width in pixels (multiple of 16) and height.
    pub width: i32,
    pub height: i32,
    pub planes: u8,
    /// Chunky pixels, `width * height`.
    pub data: Vec<u8>,
    /// Mask (pixel != 0), when grabbed with the mask flag.
    pub mask: Option<Vec<bool>>,
}

impl Block {
    /// `Hrev Block`/`Vrev Block` (`RevBloc` -> `Retourne`): flips the image
    /// data (the whole word width).
    pub fn flip(&mut self, horizontal: bool) {
        let (w, h) = (self.width as usize, self.height as usize);
        let flip = |v: &mut Vec<u8>| {
            for y in 0..h {
                if horizontal {
                    v[y * w..(y + 1) * w].reverse();
                }
            }
            if !horizontal {
                for y in 0..h / 2 {
                    for x in 0..w {
                        v.swap(y * w + x, (h - 1 - y) * w + x);
                    }
                }
            }
        };
        flip(&mut self.data);
        if let Some(m) = &mut self.mask {
            let mut mv: Vec<u8> = m.iter().map(|&b| b as u8).collect();
            flip(&mut mv);
            *m = mv.into_iter().map(|b| b != 0).collect();
        }
    }
}

/// A compressed block (`CBloc` `+W.s:12019`), stored in the original byte
/// format: X (pixels), Y, width in bytes, height, planes, then RLE data.
#[derive(Clone, Debug)]
pub struct CBlock {
    pub number: i32,
    pub x: i32,
    pub y: i32,
    pub tx: i32,
    pub ty: i32,
    pub planes: u8,
    pub data: Vec<u8>,
}

/// A font entry of `Get Fonts` (`AvailFonts` TextAttr).
#[derive(Clone, Debug)]
pub struct FontInfo {
    pub name: String,
    pub size: u16,
    /// 1 = ROM/memory, 2 = disc.
    pub kind: u16,
}

/// Fonts listed by `Get Fonts` (`AvailFonts`): the ROM fonts, then the
/// fonts of a standard Workbench 2 `FONTS:` drawer. Programs often expect
/// several fonts; every font is drawn with the built-in topaz 8 glyphs.
pub fn font_list(rom: bool, disc: bool) -> Vec<FontInfo> {
    const DISC: &[(&str, &[u16])] = &[
        ("courier.font", &[11, 13, 15, 18, 24]),
        ("diamond.font", &[12, 20]),
        ("emerald.font", &[17, 20]),
        ("garnet.font", &[9, 16]),
        ("helvetica.font", &[9, 11, 13, 15, 18, 24]),
        ("opal.font", &[9, 12]),
        ("ruby.font", &[8, 12, 15]),
        ("sapphire.font", &[14, 19]),
        ("times.font", &[11, 13, 15, 18, 24]),
        ("topaz.font", &[11]),
        ("xen.font", &[8, 9, 11]),
    ];
    let mut v = Vec::new();
    if rom {
        for size in [8, 9] {
            v.push(FontInfo {
                name: "topaz.font".into(),
                size,
                kind: 1,
            });
        }
    }
    if disc {
        for (name, sizes) in DISC {
            for &size in *sizes {
                v.push(FontInfo {
                    name: (*name).into(),
                    size,
                    kind: 2,
                });
            }
        }
    }
    v
}

/// Drawing state shared by all screens: blocks, scroll zones, font list.
#[derive(Debug, Default)]
pub struct DrawGlobals {
    /// Normal blocks, newest first (`T_AdBlocs`).
    pub blocks: Vec<Block>,
    /// Compressed blocks, newest first (`T_AdCBlocs`).
    pub cblocks: Vec<CBlock>,
    /// Def Scroll zones 1-10 (`DScrolls`): x1,y1,x2,y2,dx,dy.
    pub scrolls: [Option<[i32; 6]>; 10],
    /// Result of the last Get Fonts (`T_FontInfos`).
    pub fonts: Option<Vec<FontInfo>>,
    /// `Mask Iff`: chunks not loaded (inverted `IffMask`, so the default
    /// 0 loads everything).
    pub iff_mask_off: u32,
}

/// Grabs a block (`GetBob`): `w` is rounded up to whole words, the extra
/// columns are cleared.
#[allow(clippy::too_many_arguments)]
pub fn grab_block(
    buf: &[u8],
    bw: i32,
    n: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    planes: u8,
    with_mask: bool,
) -> Block {
    let width = (w + 15) & !15;
    let mut data = vec![0u8; (width * h) as usize];
    for yy in 0..h {
        for xx in 0..w {
            data[(yy * width + xx) as usize] = buf[((y + yy) * bw + x + xx) as usize];
        }
    }
    let mask = with_mask.then(|| data.iter().map(|&p| p != 0).collect());
    Block {
        number: n,
        x,
        y,
        width,
        height: h,
        planes,
        data,
        mask,
    }
}

/// Applies a blitter minterm (BltBitMap style, bits 4-7 used: index =
/// 4 + 2*B + C, or bits 0-7 with an A channel) to whole bytes.
#[inline]
fn minterm8(m: u8, a: u8, b: u8, c: u8) -> u8 {
    let mut r = 0u8;
    for i in 0..8 {
        if m & (1 << i) != 0 {
            let ai = if i & 4 != 0 { a } else { !a };
            let bi = if i & 2 != 0 { b } else { !b };
            let ci = if i & 1 != 0 { c } else { !c };
            r |= ai & bi & ci;
        }
    }
    r
}

/// `Put Block` (`DrawBloc` `+W.s:12475`): draws through the bob routine,
/// clipped horizontally to the word aligned clip rectangle.
#[allow(clippy::too_many_arguments)]
pub fn put_block(
    b: &Block,
    buf: &mut [u8],
    bw: i32,
    bh: i32,
    screen_planes: u8,
    clip: (i32, i32, i32, i32),
    x: i32,
    y: i32,
    plane_mask: u16,
    minterm: Option<u8>,
) {
    let lim_g = (clip.0 & !15).max(0);
    let lim_d = ((clip.2 + 15) & !15).min(bw);
    let lim_h = clip.1.max(0);
    let lim_b = clip.3.min(bh);
    let nplanes = b.planes.min(screen_planes);
    let pmask = (((1u32 << nplanes) - 1) as u16 & plane_mask) as u8;
    // Default minterm: D = A ? B : C ($CA); without mask A is all ones.
    let m = minterm.unwrap_or(0xCA);
    for yy in 0..b.height {
        let sy = y + yy;
        if sy < lim_h || sy >= lim_b {
            continue;
        }
        for xx in 0..b.width {
            let sx = x + xx;
            if sx < lim_g || sx >= lim_d {
                continue;
            }
            let i = (yy * b.width + xx) as usize;
            let a = match &b.mask {
                Some(mk) => {
                    if mk[i] {
                        0xFF
                    } else {
                        0
                    }
                }
                None => 0xFF,
            };
            let o = (sy * bw + sx) as usize;
            let c = buf[o];
            let r = minterm8(m, a, b.data[i], c);
            buf[o] = (c & !pmask) | (r & pmask);
        }
    }
}

/// Byte of plane `p` at byte column `xb` of line `y` (8 pixels).
fn plane_byte(buf: &[u8], bw: i32, xb: i32, y: i32, p: u8) -> u8 {
    let mut v = 0;
    let o = (y * bw + xb * 8) as usize;
    for i in 0..8 {
        if buf.get(o + i).is_some_and(|&px| px & (1 << p) != 0) {
            v |= 0x80 >> i;
        }
    }
    v
}

fn set_plane_byte(buf: &mut [u8], bw: i32, xb: i32, y: i32, p: u8, v: u8) {
    let o = (y * bw + xb * 8) as usize;
    for i in 0..8 {
        if let Some(px) = buf.get_mut(o + i) {
            if v & (0x80 >> i) != 0 {
                *px |= 1 << p;
            } else {
                *px &= !(1 << p);
            }
        }
    }
}

/// `Get Cblock` (`CBloc`): byte RLE per line and plane. Returns None when
/// the zone is outside the screen (`CBlE3`). Reproduces the original's
/// encoding exactly, including its handling of runs longer than 64 bytes.
#[allow(clippy::too_many_arguments)]
pub fn make_cblock(
    buf: &[u8],
    bw: i32,
    bh: i32,
    planes: u8,
    n: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Option<CBlock> {
    let tline = bw / 8;
    let xb = ((x as u16) >> 3) as i32;
    let tx = ((w as u16) >> 3) as i32;
    if xb + tx > tline || (y + h) * tline > tline * bh || h <= 0 || tx <= 0 || y < 0 {
        return None;
    }
    let mut out = Vec::new();
    for yy in 0..h {
        for p in 0..planes {
            let row: Vec<u8> = (0..tx)
                .map(|i| plane_byte(buf, bw, xb + i, y + yy, p))
                .collect();
            let mut pos = 0usize;
            let mut d5 = tx;
            // CBl3..CBl8.
            while d5 > 0 {
                let mut d7 = 0;
                let d6 = row[pos];
                pos += 1;
                loop {
                    d5 -= 1;
                    if d5 == 0 || row[pos] != d6 {
                        break;
                    }
                    pos += 1;
                    d7 += 1;
                    if d7 >= 64 {
                        d7 -= 1;
                        break;
                    }
                }
                if d7 != 0 {
                    out.push(0xC0 | d7 as u8);
                    out.push(d6);
                } else {
                    if d6 >= 0xC0 {
                        out.push(0xC0);
                    }
                    out.push(d6);
                }
            }
        }
    }
    Some(CBlock {
        number: n,
        x,
        y,
        tx,
        ty: h,
        planes,
        data: out,
    })
}

/// `Put Cblock` (`PBloc`): x/y < 0 use the grab position. Returns false if
/// the block does not fit.
pub fn put_cblock(
    b: &CBlock,
    buf: &mut [u8],
    bw: i32,
    bh: i32,
    planes: u8,
    x: i32,
    y: i32,
) -> bool {
    let x = if x < 0 { b.x } else { x };
    let y = if y < 0 { b.y } else { y };
    let tline = bw / 8;
    let xb = ((x as u16) >> 3) as i32;
    if xb + b.tx > tline || (y + b.ty) * tline > tline * bh || y < 0 {
        return false;
    }
    let np = b.planes.min(planes);
    let mut src = b.data.iter().copied();
    for yy in 0..b.ty {
        for p in 0..np {
            let mut d5 = b.tx - 1;
            let mut col = 0;
            loop {
                let Some(d6) = src.next() else { return true };
                if d6 < 0xC0 {
                    set_plane_byte(buf, bw, xb + col, y + yy, p, d6);
                    col += 1;
                    if d5 == 0 {
                        break;
                    }
                    d5 -= 1;
                } else {
                    let cnt = (d6 & 0x3F) as i32;
                    d5 -= cnt;
                    let v = src.next().unwrap_or(0);
                    for _ in 0..=cnt {
                        if col < b.tx {
                            set_plane_byte(buf, bw, xb + col, y + yy, p, v);
                        }
                        col += 1;
                    }
                    d5 -= 1;
                    if d5 < 0 {
                        break;
                    }
                }
            }
        }
    }
    true
}

/// Geometry of a Screen Copy after the clipping of `Sco0` (`+Lib.s:10298`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CopyRect {
    pub sx: i32,
    pub sy: i32,
    pub dx: i32,
    pub dy: i32,
    pub w: i32,
    pub h: i32,
}

/// Clips a Screen Copy like `Sco0`. Coordinates are 16 bit words; the
/// end of the source zone is exclusive. None = nothing to copy.
#[allow(clippy::too_many_arguments)]
pub fn clip_copy(
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    dx: i32,
    dy: i32,
) -> Option<CopyRect> {
    let (mut d0, mut d1, mut d2, mut d3) = (
        x1 as i16 as i32,
        y1 as i16 as i32,
        dx as i16 as i32,
        dy as i16 as i32,
    );
    let (mut d4, mut d5) = (x2 as i16 as i32, y2 as i16 as i32);
    if d0 < 0 {
        d2 -= d0;
        d0 = 0;
    }
    if d1 < 0 {
        d3 -= d1;
        d1 = 0;
    }
    if d2 < 0 {
        d0 -= d2;
        d2 = 0;
    }
    if d3 < 0 {
        d1 -= d3;
        d3 = 0;
    }
    if d0 >= src_w || d1 >= src_h || d2 >= dst_w || d3 >= dst_h {
        return None;
    }
    if d4 < 0 || d5 < 0 {
        return None;
    }
    d4 = d4.min(src_w);
    d5 = d5.min(src_h);
    d4 -= d0;
    d5 -= d1;
    if d4 <= 0 || d5 <= 0 {
        return None;
    }
    let over = d2 + d4 - dst_w;
    if over > 0 {
        d4 -= over;
        if d4 <= 0 {
            return None;
        }
    }
    let over = d3 + d5 - dst_h;
    if over > 0 {
        d5 -= over;
        if d5 <= 0 {
            return None;
        }
    }
    Some(CopyRect {
        sx: d0,
        sy: d1,
        dx: d2,
        dy: d3,
        w: d4,
        h: d5,
    })
}

/// Extracts a rectangle of pixels.
pub fn extract(buf: &[u8], bw: i32, r: &CopyRect) -> Vec<u8> {
    let mut v = Vec::with_capacity((r.w * r.h) as usize);
    for y in 0..r.h {
        let o = ((r.sy + y) * bw + r.sx) as usize;
        v.extend_from_slice(&buf[o..o + r.w as usize]);
    }
    v
}

/// Writes pixels extracted with [`extract`] using a BltBitMap minterm
/// (B = source, C = destination) on the first `planes` planes.
pub fn blit(src: &[u8], buf: &mut [u8], bw: i32, r: &CopyRect, minterm: u8, planes: u8) {
    let pm = ((1u32 << planes.min(8)) - 1) as u8;
    let m = (minterm & 0xF0) | (minterm >> 4); // A is always set: only bits 4-7 count
    let fast = minterm & 0xF0 == 0xC0;
    for y in 0..r.h {
        let o = ((r.dy + y) * bw + r.dx) as usize;
        let s = (y * r.w) as usize;
        for x in 0..r.w as usize {
            let c = buf[o + x];
            let b = src[s + x];
            let v = if fast { b } else { minterm8(m, 0xFF, b, c) };
            buf[o + x] = (c & !pm) | (v & pm);
        }
    }
}

/// Zoom table (`ZooTab` `+Lib.s:10703`): one entry per destination
/// column/line, the number of source pixels to advance first.
pub fn zoom_table(a1: i32, a2: i32, b1: i32, b2: i32) -> Vec<i32> {
    let src = a2 - a1;
    let dst = b2 - b1;
    let mut t = Vec::new();
    if dst < src {
        // Reduce.
        let mut d0 = -1;
        let mut d4 = src - 1;
        for _ in 0..src {
            d0 += 1;
            d4 -= dst;
            if d4 < 0 {
                d4 += src;
                t.push(d0);
                d0 = 0;
            }
        }
    } else {
        // Zoom.
        t.push(0);
        let mut d5 = dst - 1;
        for _ in 0..(dst - 1).max(0) {
            d5 -= src;
            if d5 < 0 {
                d5 += dst;
                t.push(1);
            } else {
                t.push(0);
            }
        }
    }
    t
}

/// `Zoom`: copies (sx1,sy1) using the tables to (dx,dy) of the destination
/// (planes = min of both screens).
#[allow(clippy::too_many_arguments)]
pub fn zoom(
    src: &[u8],
    sw: i32,
    dst: &mut [u8],
    dw: i32,
    sx1: i32,
    sy1: i32,
    dx: i32,
    dy: i32,
    tx: &[i32],
    ty: &[i32],
    planes: u8,
) {
    let pm = ((1u32 << planes.min(8)) - 1) as u8;
    let mut sx = sx1;
    for (i, &ax) in tx.iter().enumerate() {
        sx += ax;
        let mut sy = sy1;
        for (j, &ay) in ty.iter().enumerate() {
            sy += ay;
            let s = src.get((sy * sw + sx) as usize).copied().unwrap_or(0);
            if let Some(d) = dst.get_mut(((dy + j as i32) * dw + dx + i as i32) as usize) {
                *d = (*d & !pm) | (s & pm);
            }
        }
    }
}

/// One step of `Appear` (`InAppear4` `+Lib.s:10443`): copies `count`
/// pixels, advancing the position `pos` by `step` modulo `total` each time.
/// Pixels are addressed linearly in the source bitmap; the destination
/// uses the same line/column if it fits.
#[allow(clippy::too_many_arguments)]
pub fn appear_step(
    src: &[u8],
    sw: i32,
    dst: &mut [u8],
    dw: i32,
    dh: i32,
    planes: u8,
    pos: &mut u64,
    step: u64,
    total: u64,
    count: u64,
) {
    let pm = ((1u32 << planes.min(8)) - 1) as u8;
    for _ in 0..count {
        *pos += step;
        if *pos >= total {
            *pos -= total;
        }
        let x = (*pos % sw as u64) as i32;
        let y = (*pos / sw as u64) as i32;
        if y >= dh || x >= dw {
            continue;
        }
        let s = src[(y * sw + x) as usize];
        let d = &mut dst[(y * dw + x) as usize];
        *d = (*d & !pm) | (s & pm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cblock_roundtrip() {
        let (w, h) = (64, 8);
        let mut buf: Vec<u8> = (0..w * h).map(|i| ((i / 3) % 7) as u8).collect();
        buf[..32].fill(5);
        let b = make_cblock(&buf, w, h, 3, 1, 0, 0, 64, 8).unwrap();
        let mut out = vec![0u8; (w * h) as usize];
        assert!(put_cblock(&b, &mut out, w, h, 3, -1, -1));
        assert_eq!(out, buf);
    }

    #[test]
    fn block_put_mask() {
        let mut buf = vec![0u8; 32 * 4];
        buf[1] = 3;
        let b = grab_block(&buf, 32, 1, 0, 0, 4, 1, 4, true);
        assert_eq!(b.width, 16);
        let mut dst = vec![9u8; 32 * 4];
        put_block(&b, &mut dst, 32, 4, 4, (0, 0, 32, 4), 0, 0, 0xFFFF, None);
        assert_eq!(&dst[0..3], &[9, 3, 9]);
        let b = grab_block(&buf, 32, 1, 0, 0, 4, 1, 4, false);
        put_block(&b, &mut dst, 32, 4, 4, (0, 0, 32, 4), 0, 0, 0xFFFF, None);
        assert_eq!(&dst[0..3], &[0, 3, 0]);
    }

    #[test]
    fn zoom_tables() {
        // Doubling: 0,1,0,1...
        assert_eq!(zoom_table(0, 4, 0, 8).len(), 8);
        assert_eq!(zoom_table(0, 4, 0, 8).iter().sum::<i32>(), 3);
        // Halving: 4 entries advancing by 2 (first one 1).
        let t = zoom_table(0, 8, 0, 4);
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn copy_clip() {
        let r = clip_copy(320, 200, 320, 200, -10, 0, 100, 50, 0, 0).unwrap();
        assert_eq!(
            r,
            CopyRect {
                sx: 0,
                sy: 0,
                dx: 10,
                dy: 0,
                w: 100,
                h: 50
            }
        );
        assert!(clip_copy(320, 200, 320, 200, 0, 0, 10, 10, 320, 0).is_none());
    }
}
