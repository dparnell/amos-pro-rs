//! Packed pictures ("Pac.Pic." banks): the Compact extension's packer
//! (`+Compact.s` `Pack`, `GetSize`) and the unpacker (`UnPack_Bitmap`,
//! `UnPack_Screen`, `+Lib.s:25471`). The byte format is reproduced exactly
//! so banks made by AMOS can be unpacked and vice versa.
//!
//! Format: optional screen header (`PsCode` $12031990, 90 bytes), then the
//! bitmap header (`Pkcode` $06071963, 24 bytes), then three streams:
//! - data bytes (only the bytes that differ from the previous one),
//! - "pointer" bits, packed like the data: one bit per byte of the
//!   intermediate bit table, set when that table byte changed,
//! - the changed intermediate table bytes (one bit per picture byte, set
//!   when the picture byte differs from the previous one).
//!
//! Picture bytes are visited plane by plane, then by rows of `tcar` lines,
//! then byte column by byte column, top to bottom inside the `tcar` lines.

pub const SC_CODE: u32 = 0x1203_1990;
pub const BM_CODE: u32 = 0x0607_1963;
/// Size of the screen header (`PsLong`).
pub const PS_LONG: usize = 90;
/// Size of the bitmap header (`PkLong`).
pub const PK_LONG: usize = 24;

/// Screen parameters saved by `Spack` (`PsTx`...).
#[derive(Clone, Debug, PartialEq)]
pub struct PackedScreen {
    pub tx: u16,
    pub ty: u16,
    pub awx: i16,
    pub awy: i16,
    pub awtx: i16,
    pub awty: i16,
    pub avx: i16,
    pub avy: i16,
    pub con0: u16,
    pub nb_col: u16,
    pub nplan: u16,
    pub palette: [u16; 32],
}

impl PackedScreen {
    pub fn write(&self, out: &mut [u8]) {
        let mut w = |o: usize, v: u16| out[o..o + 2].copy_from_slice(&v.to_be_bytes());
        w(0, (SC_CODE >> 16) as u16);
        w(2, SC_CODE as u16);
        w(4, self.tx);
        w(6, self.ty);
        w(8, self.awx as u16);
        w(10, self.awy as u16);
        w(12, self.awtx as u16);
        w(14, self.awty as u16);
        w(16, self.avx as u16);
        w(18, self.avy as u16);
        w(20, self.con0);
        w(22, self.nb_col);
        w(24, self.nplan);
        for (i, &c) in self.palette.iter().enumerate() {
            w(26 + i * 2, c);
        }
    }

    pub fn read(d: &[u8]) -> Option<PackedScreen> {
        if d.len() < PS_LONG || rd32(d, 0) != SC_CODE {
            return None;
        }
        let w = |o: usize| rd16(d, o);
        let mut palette = [0u16; 32];
        for (i, p) in palette.iter_mut().enumerate() {
            *p = w(26 + i * 2);
        }
        Some(PackedScreen {
            tx: w(4),
            ty: w(6),
            awx: w(8) as i16,
            awy: w(10) as i16,
            awtx: w(12) as i16,
            awty: w(14) as i16,
            avx: w(16) as i16,
            avy: w(18) as i16,
            con0: w(20),
            nb_col: w(22),
            nplan: w(24),
            palette,
        })
    }
}

fn rd16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([
        d.get(o).copied().unwrap_or(0),
        d.get(o + 1).copied().unwrap_or(0),
    ])
}

fn rd32(d: &[u8], o: usize) -> u32 {
    ((rd16(d, o) as u32) << 16) | rd16(d, o + 2) as u32
}

/// Source of the packer: a chunky bitmap read as Amiga bitplanes.
pub struct PlanarView<'a> {
    pub buf: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub planes: usize,
}

impl PlanarView<'_> {
    /// Bytes per line (`EcTLigne`).
    fn tligne(&self) -> usize {
        self.width / 8
    }

    /// Byte of plane `p` at byte offset `off` of the plane.
    fn byte(&self, p: usize, off: usize) -> u8 {
        let tl = self.tligne();
        let (y, xb) = (off / tl, off % tl);
        if y >= self.height {
            return 0;
        }
        let o = y * self.width + xb * 8;
        let mut v = 0;
        for i in 0..8 {
            if self.buf[o + i] & (1 << p) != 0 {
                v |= 0x80 >> i;
            }
        }
        v
    }
}

/// Sizes tried by `GetSize` for the square height.
const TSIZE: [usize; 14] = [1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 24, 32, 48, 64];

/// Packs the zone (dx bytes, dy lines, tx bytes, ty lines) with square
/// height `tcar`; returns the packed bitmap (header included, before the
/// final even rounding of the size).
fn pack_with(
    src: &PlanarView,
    dx: usize,
    dy: usize,
    tx: usize,
    ty: usize,
    tcar: usize,
) -> (Vec<u8>, usize) {
    let rows = ty / tcar;
    let nplan = src.planes;
    let tl = src.tligne();
    // Intermediate bit table: one bit per picture byte (`Pack`).
    let tsize = ((tcar * tx * rows * nplan) >> 3) + 2;
    let mut table = vec![0u8; tsize + 1];
    let mut data: Vec<u8> = vec![0]; // byte at PkDatas1 is the initial 0
    let mut ti = 0usize;
    let mut bit = 7i32;
    let mut last = 0u8;
    let base = dy * tl + dx;
    for p in 0..nplan {
        for r in 0..rows {
            for c in 0..tx {
                for l in 0..tcar {
                    let off = base + (r * tcar + l) * tl + c;
                    let b = src.byte(p, off);
                    if b != last {
                        last = b;
                        data.push(b);
                        table[ti] |= 1 << bit;
                    }
                    bit -= 1;
                    if bit < 0 {
                        bit = 7;
                        ti += 1;
                    }
                }
            }
        }
    }
    // a5 is now one past the last data byte: PkPoint2.
    let point2 = PK_LONG + data.len();
    let ptr_len = (tsize >> 3) + 2;
    let datas2 = point2 + ptr_len;
    let mut ptr = vec![0u8; ptr_len];
    let mut data2: Vec<u8> = vec![0];
    let mut last = 0u8;
    let mut pi = 0usize;
    let mut bit = 7i32;
    for &t in table.iter().take(tsize) {
        if t != last {
            last = t;
            data2.push(t);
            ptr[pi] |= 1 << bit;
        }
        bit -= 1;
        if bit < 0 {
            bit = 7;
            pi += 1;
        }
    }
    let mut out = vec![0u8; PK_LONG];
    out[0..4].copy_from_slice(&BM_CODE.to_be_bytes());
    out[4..6].copy_from_slice(&(dx as u16).to_be_bytes());
    out[6..8].copy_from_slice(&(dy as u16).to_be_bytes());
    out[8..10].copy_from_slice(&(tx as u16).to_be_bytes());
    out[10..12].copy_from_slice(&(rows as u16).to_be_bytes());
    out[12..14].copy_from_slice(&(tcar as u16).to_be_bytes());
    out[14..16].copy_from_slice(&(nplan as u16).to_be_bytes());
    out[16..20].copy_from_slice(&(datas2 as u32).to_be_bytes());
    out[20..24].copy_from_slice(&(point2 as u32).to_be_bytes());
    out.extend_from_slice(&data);
    out.extend_from_slice(&ptr);
    out.extend_from_slice(&data2);
    // Final size as computed by PacSize: (last written byte + 3) & ~1.
    let size = (out.len() - 1 + 3) & !1;
    (out, size)
}

/// `Pack`/`Spack` body: chooses the best square height (`GetSize`) and
/// packs. Zone in bytes (dx, tx) and lines (dy, ty). Returns the bytes,
/// already padded to the bank size computed by the original.
pub fn pack_bitmap(src: &PlanarView, dx: usize, dy: usize, tx: usize, ty: usize) -> Vec<u8> {
    let mut best: Option<(usize, usize)> = None;
    for &t in &TSIZE {
        if !ty.is_multiple_of(t) {
            continue;
        }
        let (_, size) = pack_with(src, dx, dy, tx, ty, t);
        if best.is_none_or(|(s, _)| size < s) {
            best = Some((size, t));
        }
    }
    let (size, tcar) = best.unwrap_or((0, 1));
    let (mut out, _) = pack_with(src, dx, dy, tx, ty, tcar);
    out.resize(size.max(out.len()), 0);
    out
}

/// Header of a packed bitmap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackedBitmap {
    pub dx: u16,
    pub dy: u16,
    pub tx: u16,
    pub ty: u16,
    pub tcar: u16,
    pub nplan: u16,
    pub datas2: u32,
    pub point2: u32,
}

impl PackedBitmap {
    /// Reads the bitmap header, skipping a screen header if present.
    /// Returns the header and the offset of the bitmap header in `d`.
    pub fn read(d: &[u8]) -> Option<(PackedBitmap, usize)> {
        let o = if rd32(d, 0) == SC_CODE { PS_LONG } else { 0 };
        if d.len() < o + PK_LONG || rd32(d, o) != BM_CODE {
            return None;
        }
        Some((
            PackedBitmap {
                dx: rd16(d, o + 4),
                dy: rd16(d, o + 6),
                tx: rd16(d, o + 8),
                ty: rd16(d, o + 10),
                tcar: rd16(d, o + 12),
                nplan: rd16(d, o + 14),
                datas2: rd32(d, o + 16),
                point2: rd32(d, o + 20),
            },
            o,
        ))
    }
}

/// `UnPack_Bitmap`: unpacks into a chunky bitmap. `x` (pixels) / `y` < 0
/// use the packed position. Returns the size unpacked (w, h) or None
/// ("Not a packed bitmap": wrong format, different number of planes, or
/// does not fit).
pub fn unpack_bitmap(
    d: &[u8],
    buf: &mut [u8],
    width: usize,
    height: usize,
    planes: usize,
    x: i32,
    y: i32,
) -> Option<(usize, usize)> {
    let (h, o) = PackedBitmap::read(d)?;
    if h.nplan as usize != planes {
        return None;
    }
    let tl = width / 8;
    let xb = if x < 0 {
        h.dx as usize
    } else {
        x as usize >> 3
    };
    let yy = if y < 0 { h.dy as usize } else { y as usize };
    let (tx, tcar, rows) = (h.tx as usize, h.tcar as usize, h.ty as usize);
    if tx + xb > tl || rows * tcar + yy > height || tcar == 0 {
        return None;
    }
    let g = |i: usize| d.get(o + i).copied().unwrap_or(0);
    let mut a4 = PK_LONG; // data
    let mut a5 = h.datas2 as usize; // table bytes
    let mut a6 = h.point2 as usize; // pointer bits
    let mut d0 = 7i32;
    let mut d1 = 7i32;
    let mut d2 = g(a5);
    a5 += 1;
    let mut d3 = g(a4);
    a4 += 1;
    if g(a6) & (1 << d1) != 0 {
        d2 = g(a5);
        a5 += 1;
    }
    d1 -= 1;
    for p in 0..planes {
        for r in 0..rows {
            for c in 0..tx {
                for l in 0..tcar {
                    if d2 & (1 << d0) != 0 {
                        d3 = g(a4);
                        a4 += 1;
                    }
                    // Write byte d3 of plane p.
                    let py = yy + r * tcar + l;
                    let o = py * width + (xb + c) * 8;
                    for i in 0..8 {
                        let px = &mut buf[o + i];
                        if d3 & (0x80 >> i) != 0 {
                            *px |= 1 << p;
                        } else {
                            *px &= !(1 << p);
                        }
                    }
                    d0 -= 1;
                    if d0 < 0 {
                        d0 = 7;
                        if g(a6) & (1 << d1) != 0 {
                            d2 = g(a5);
                            a5 += 1;
                        }
                        d1 -= 1;
                        if d1 < 0 {
                            d1 = 7;
                            a6 += 1;
                        }
                    }
                }
            }
        }
    }
    Some((tx * 8, rows * tcar))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let (w, h, planes) = (64usize, 40usize, 3usize);
        let buf: Vec<u8> = (0..w * h)
            .map(|i| (((i / 5) ^ (i / 64)) % 8) as u8)
            .collect();
        let v = PlanarView {
            buf: &buf,
            width: w,
            height: h,
            planes,
        };
        let packed = pack_bitmap(&v, 0, 0, w / 8, h);
        assert_eq!(packed.len() % 2, 0);
        let mut out = vec![0u8; w * h];
        assert_eq!(
            unpack_bitmap(&packed, &mut out, w, h, planes, -1, -1),
            Some((w, h))
        );
        assert_eq!(out, buf);
    }

    #[test]
    fn partial_zone() {
        let (w, h, planes) = (64usize, 32usize, 2usize);
        let buf: Vec<u8> = (0..w * h).map(|i| ((i * 7 / 3) % 4) as u8).collect();
        let v = PlanarView {
            buf: &buf,
            width: w,
            height: h,
            planes,
        };
        let packed = pack_bitmap(&v, 2, 4, 3, 12);
        let mut out = vec![0u8; w * h];
        unpack_bitmap(&packed, &mut out, w, h, planes, -1, -1).unwrap();
        for y in 4..16 {
            for x in 16..40 {
                assert_eq!(out[y * w + x], buf[y * w + x]);
            }
        }
        assert_eq!(out[0], 0);
    }

    /// Unpacks the Pac.Pic. banks of an example program and packs them
    /// again: the result must be byte identical to the AMOS packer's.
    #[test]
    fn real_banks_repack_identically() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../AMOS-Professional-365/AMOS/Productivity1/SuperBlockout.AMOS"
        );
        let Ok(file) = std::fs::read(path) else {
            return;
        };
        let prg = crate::program::Program::load(&file).unwrap();
        let mut n = 0;
        for b in prg.banks.iter().filter(|b| b.name.starts_with("Pac.Pic")) {
            let d = b.raw().unwrap();
            let (h, o) = PackedBitmap::read(d).unwrap();
            let scr = PackedScreen::read(d);
            let (w, hh, planes) = match &scr {
                Some(s) => (s.tx as usize, s.ty as usize, s.nplan as usize),
                None => (
                    (h.dx + h.tx) as usize * 8,
                    (h.dy + h.ty * h.tcar) as usize,
                    h.nplan as usize,
                ),
            };
            let mut buf = vec![0u8; w * hh];
            unpack_bitmap(d, &mut buf, w, hh, planes, -1, -1).unwrap();
            let v = PlanarView {
                buf: &buf,
                width: w,
                height: hh,
                planes,
            };
            let re = pack_bitmap(
                &v,
                h.dx as usize,
                h.dy as usize,
                h.tx as usize,
                (h.ty * h.tcar) as usize,
            );
            assert_eq!(&re[..], &d[o..o + re.len()], "bank {}", b.number);
            assert_eq!(re.len(), d.len() - o, "bank {} size", b.number);
            n += 1;
        }
        assert_eq!(n, 3);
    }
}
