//! IFF ILBM pictures: `Load Iff` (`IffFormPlay` `+Lib.s:6991`) and
//! `Save Iff` (`IffSaveScreen` `+Lib.s:7552`).
//!
//! Chunks handled: BMHD, CAMG, CMAP, CCRT, BODY (raw or ByteRun1) and the
//! AMOS specific AMSC chunk (screen display/offset settings).

/// Bad IFF format / compression not recognised / Can't fit picture.
pub const BAD_IFF_FORMAT: u16 = 30;
pub const IFF_COMPRESSION: u16 = 31;
pub const CANT_FIT_PICTURE: u16 = 32;

/// BMHD chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bmhd {
    pub w: u16,
    pub h: u16,
    pub planes: u8,
    pub masking: u8,
    pub compression: u8,
    pub page_w: u16,
    pub page_h: u16,
}

/// AMSC chunk: Screen Display x,y,w,h, Screen Offset x,y, flags.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Amsc {
    pub awx: i16,
    pub awy: i16,
    pub awtx: i16,
    pub awty: i16,
    pub avx: i16,
    pub avy: i16,
    pub flags: u16,
}

/// CCRT chunk (colour cycling).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ccrt {
    pub direction: i16,
    pub start: i8,
    pub end: i8,
    pub micros: u32,
}

/// A parsed ILBM form.
#[derive(Clone, Debug, Default)]
pub struct Ilbm<'a> {
    pub bmhd: Option<Bmhd>,
    pub camg: Option<u32>,
    pub cmap: Option<&'a [u8]>,
    pub ccrt: Option<Ccrt>,
    pub amsc: Option<Amsc>,
    pub body: Option<&'a [u8]>,
}

fn be16(d: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([
        d.get(o).copied().unwrap_or(0),
        d.get(o + 1).copied().unwrap_or(0),
    ])
}

fn be32(d: &[u8], o: usize) -> u32 {
    ((be16(d, o) as u32) << 16) | be16(d, o + 2) as u32
}

/// Parses the first ILBM form of a file (forms "ANIM" are entered).
pub fn parse(d: &[u8]) -> Result<Ilbm<'_>, u16> {
    if d.len() < 12 || &d[0..4] != b"FORM" {
        return Err(BAD_IFF_FORMAT);
    }
    let mut pos = 0;
    // Enter ANIM forms to their first ILBM.
    loop {
        if d.len() < pos + 12 || &d[pos..pos + 4] != b"FORM" {
            return Err(BAD_IFF_FORMAT);
        }
        match &d[pos + 8..pos + 12] {
            b"ANIM" => pos += 12,
            b"ILBM" => break,
            _ => return Err(BAD_IFF_FORMAT),
        }
    }
    let end = (pos + 8 + be32(d, pos + 4) as usize).min(d.len());
    let mut p = pos + 12;
    let mut out = Ilbm::default();
    while p + 8 <= end {
        let id = &d[p..p + 4];
        let len = be32(d, p + 4) as usize;
        let data = &d[p + 8..(p + 8 + len).min(d.len())];
        match id {
            b"BMHD" => {
                out.bmhd = Some(Bmhd {
                    w: be16(data, 0),
                    h: be16(data, 2),
                    planes: data.get(8).copied().unwrap_or(0),
                    masking: data.get(9).copied().unwrap_or(0),
                    compression: data.get(10).copied().unwrap_or(0),
                    page_w: be16(data, 16),
                    page_h: be16(data, 18),
                })
            }
            b"CAMG" => out.camg = Some(be32(data, 0)),
            b"CMAP" => out.cmap = Some(data),
            b"CCRT" => {
                out.ccrt = Some(Ccrt {
                    direction: be16(data, 0) as i16,
                    start: data.get(2).copied().unwrap_or(0) as i8,
                    end: data.get(3).copied().unwrap_or(0) as i8,
                    micros: be32(data, 8),
                })
            }
            b"AMSC" => {
                out.amsc = Some(Amsc {
                    awx: be16(data, 0) as i16,
                    awy: be16(data, 2) as i16,
                    awtx: be16(data, 4) as i16,
                    awty: be16(data, 6) as i16,
                    avx: be16(data, 8) as i16,
                    avy: be16(data, 10) as i16,
                    flags: be16(data, 12),
                })
            }
            b"BODY" => {
                out.body = Some(data);
                // The BODY is played when met: later chunks are ignored.
                break;
            }
            _ => {}
        }
        p += 8 + ((len + 1) & !1);
    }
    Ok(out)
}

/// Screen to open for a picture (`IffScreen`): (width, height, planes,
/// colours (4096 = HAM), mode bits $8000 hires / $4 laced).
pub fn screen_params(il: &Ilbm) -> Option<(u32, u32, u8, u32, u32)> {
    let b = il.bmhd?;
    let w = ((b.w as u32) + 15) & !15;
    let h = b.h as u32;
    let mut planes = b.planes;
    let mut colours = 1u32 << planes.clamp(1, 6);
    let mut mode = 0u32;
    if b.page_w >= 640 && planes <= 4 {
        mode |= 0x8000;
    }
    if b.page_h >= 400 {
        mode |= 0x4;
    }
    if let Some(camg) = il.camg {
        mode = 0;
        if camg & 0x800 != 0 {
            planes = 6;
            colours = 4096;
        }
        mode |= camg & 0x8004;
    }
    let _ = planes;
    Some((w, h, planes, colours, mode))
}

/// Palette after the CMAP (`IffPal`): starts from `base` (the default
/// palette), then each CMAP triple, 4 bits per component.
pub fn palette(il: &Ilbm, base: &[u16; 32]) -> [u16; 32] {
    let mut pal = *base;
    if let Some(c) = il.cmap {
        for (i, rgb) in c.as_chunks::<3>().0.iter().take(32).enumerate() {
            pal[i] = ((rgb[0] as u16 & 0xF0) << 4) | (rgb[1] as u16 & 0xF0) | (rgb[2] as u16 >> 4);
        }
    }
    pal
}

/// Decodes the BODY into a chunky bitmap of `width` x `height` with
/// `planes` planes (the picture must fit: checked by the caller). Lines
/// beyond `height` are ignored; a mask plane (masking = 1) is skipped.
pub fn decode_body(il: &Ilbm, buf: &mut [u8], width: usize, height: usize) -> Result<(), u16> {
    let b = il.bmhd.ok_or(BAD_IFF_FORMAT)?;
    let body = il.body.unwrap_or(&[]);
    let row_bytes = (b.w as usize).div_ceil(16) * 2;
    let planes = b.planes as usize + (b.masking == 1) as usize;
    let lines = (b.h as usize).min(height);
    let mut row = vec![0u8; row_bytes];
    let mut src = 0usize;
    for y in 0..lines {
        for p in 0..planes {
            match b.compression {
                0 => {
                    for (i, r) in row.iter_mut().enumerate() {
                        *r = body.get(src + i).copied().unwrap_or(0);
                    }
                    src += row_bytes;
                }
                1 => {
                    // ByteRun1.
                    let mut n = 0;
                    while n < row_bytes {
                        let Some(&c) = body.get(src) else { break };
                        src += 1;
                        let c = c as i8;
                        if c >= 0 {
                            for _ in 0..=c as usize {
                                let v = body.get(src).copied().unwrap_or(0);
                                src += 1;
                                if n < row_bytes {
                                    row[n] = v;
                                }
                                n += 1;
                            }
                        } else if c != -128 {
                            let v = body.get(src).copied().unwrap_or(0);
                            src += 1;
                            for _ in 0..=(-(c as i32)) as usize {
                                if n < row_bytes {
                                    row[n] = v;
                                }
                                n += 1;
                            }
                        }
                    }
                }
                _ => return Err(IFF_COMPRESSION),
            }
            if p >= b.planes as usize || p >= 8 {
                continue;
            }
            let o = y * width;
            for x in 0..(b.w as usize).min(width) {
                let px = &mut buf[o + x];
                if row[x >> 3] & (0x80 >> (x & 7)) != 0 {
                    *px |= 1 << p;
                } else {
                    *px &= !(1 << p);
                }
            }
        }
    }
    Ok(())
}

/// Screen description for `Save Iff`.
pub struct SaveScreen<'a> {
    pub buf: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub planes: usize,
    pub hires: bool,
    pub lace: bool,
    pub ham: bool,
    /// Number of colours of Screen Open (64 for EHB / HAM).
    pub nb_col: u32,
    pub palette: [u16; 32],
    /// Display width (lowres pixels) and height (lines).
    pub wtx: u16,
    pub wty: u16,
    pub amsc: Amsc,
}

fn plane_row(s: &SaveScreen, y: usize, p: usize) -> Vec<u8> {
    let tl = s.width / 8;
    let mut row = vec![0u8; tl];
    for (xb, r) in row.iter_mut().enumerate() {
        for i in 0..8 {
            if s.buf[y * s.width + xb * 8 + i] & (1 << p) != 0 {
                *r |= 0x80 >> i;
            }
        }
    }
    row
}

/// ByteRun1 compression of one plane line, as `SaveBODY` does it.
/// `next` holds the bytes that follow in memory (the next lines of the
/// plane), read when the original looks ahead past the end of the line.
fn compress_row(row: &[u8], next: &[u8], out: &mut Vec<u8>) {
    let at = |i: usize| {
        row.get(i)
            .or_else(|| next.get(i - row.len()))
            .copied()
            .unwrap_or(0)
    };
    let mut a0 = 0usize;
    let mut d2 = row.len() as i32;
    'sbc3: loop {
        let mut d1 = 0i32;
        let d0 = at(a0);
        a0 += 1;
        d2 -= 1;
        if d2 == 0 {
            out.push(0);
            out.push(d0);
            return;
        }
        loop {
            // SBc4
            if at(a0) != d0 {
                break;
            }
            d1 += 1;
            a0 += 1;
            if d1 >= 127 {
                break;
            }
            d2 -= 1;
            if d2 == 0 {
                break;
            }
        }
        if d1 != 0 {
            out.push((-d1) as u8);
            out.push(d0);
            if d2 != 0 {
                continue 'sbc3;
            }
            return;
        }
        // SBc6: literal run.
        let count_pos = out.len();
        d1 = 0;
        out.push(0);
        out.push(d0);
        loop {
            let c = at(a0);
            if at(a0 + 1) == c && at(a0 + 2) == c {
                break;
            }
            out.push(c);
            a0 += 1;
            d1 += 1;
            d2 -= 1;
            if d2 == 0 || d1 >= 127 {
                break;
            }
        }
        out[count_pos] = d1 as u8;
        if d2 == 0 {
            return;
        }
    }
}

/// Builds the IFF file of `Save Iff` (`comp` 0 = raw, else ByteRun1).
pub fn save(s: &SaveScreen, comp: u8) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"FORM\0\0\0\0ILBM");
    // BMHD
    f.extend_from_slice(b"BMHD");
    f.extend_from_slice(&20u32.to_be_bytes());
    f.extend_from_slice(&(s.width as u16).to_be_bytes());
    f.extend_from_slice(&(s.height as u16).to_be_bytes());
    f.extend_from_slice(&[0, 0, 0, 0, s.planes as u8, 0, comp, 0, 0, 0]);
    let (mut xa, mut ya, mut pw, mut ph) = (20u8, 22u8, s.wtx, s.wty);
    if s.hires {
        xa /= 2;
        pw *= 2;
    }
    if s.lace {
        ya /= 2;
        ph *= 2;
    }
    f.extend_from_slice(&[xa, ya]);
    f.extend_from_slice(&pw.to_be_bytes());
    f.extend_from_slice(&ph.to_be_bytes());
    // CAMG
    f.extend_from_slice(b"CAMG");
    f.extend_from_slice(&4u32.to_be_bytes());
    let mut camg = 0u32;
    if s.hires {
        camg |= 0x8000;
    }
    if s.ham {
        camg |= 0x800;
    }
    if s.lace {
        camg |= 4;
    }
    if s.nb_col == 64 && !s.ham {
        camg |= 0x80;
    }
    f.extend_from_slice(&camg.to_be_bytes());
    // AMSC
    f.extend_from_slice(b"AMSC");
    f.extend_from_slice(&14u32.to_be_bytes());
    for v in [
        s.amsc.awx,
        s.amsc.awy,
        s.amsc.awtx,
        s.amsc.awty,
        s.amsc.avx,
        s.amsc.avy,
    ] {
        f.extend_from_slice(&v.to_be_bytes());
    }
    f.extend_from_slice(&(s.amsc.flags & 0x8000).to_be_bytes());
    // CMAP
    f.extend_from_slice(b"CMAP");
    f.extend_from_slice(&96u32.to_be_bytes());
    for c in s.palette {
        f.push(((c >> 8) & 15) as u8 * 16);
        f.push(((c >> 4) & 15) as u8 * 16);
        f.push((c & 15) as u8 * 16);
    }
    // BODY
    f.extend_from_slice(b"BODY");
    let len_pos = f.len();
    f.extend_from_slice(&[0; 4]);
    let start = f.len();
    let planes: Vec<Vec<u8>> = (0..s.planes)
        .map(|p| (0..s.height).flat_map(|y| plane_row(s, y, p)).collect())
        .collect();
    let tl = s.width / 8;
    for y in 0..s.height {
        for plane in planes.iter() {
            let row = &plane[y * tl..(y + 1) * tl];
            if comp == 0 {
                f.extend_from_slice(row);
            } else {
                compress_row(row, &plane[(y + 1) * tl..], &mut f);
            }
        }
    }
    let mut blen = f.len() - start;
    if comp != 0 && blen & 1 != 0 {
        f.push(0);
        blen += 1;
    }
    f[len_pos..len_pos + 4].copy_from_slice(&(blen as u32).to_be_bytes());
    let total = (f.len() - 8) as u32;
    f[4..8].copy_from_slice(&total.to_be_bytes());
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip() {
        let (w, h) = (64usize, 20usize);
        let buf: Vec<u8> = (0..w * h)
            .map(|i| {
                if (i / 7) % 3 == 0 {
                    0
                } else {
                    ((i / 9) % 16) as u8
                }
            })
            .collect();
        for comp in [0u8, 1] {
            let s = SaveScreen {
                buf: &buf,
                width: w,
                height: h,
                planes: 4,
                hires: false,
                lace: false,
                ham: false,
                nb_col: 16,
                palette: [0x123; 32],
                wtx: 64,
                wty: 20,
                amsc: Amsc {
                    awx: 128,
                    awy: 50,
                    awtx: 64,
                    awty: 20,
                    ..Default::default()
                },
            };
            let f = save(&s, comp);
            let il = parse(&f).unwrap();
            assert_eq!(il.amsc.unwrap().awx, 128);
            assert_eq!(screen_params(&il), Some((64, 20, 4, 16, 0)));
            let mut out = vec![0u8; w * h];
            decode_body(&il, &mut out, w, h).unwrap();
            assert_eq!(out, buf, "comp {comp}");
            assert_eq!(palette(&il, &[0; 32])[5], 0x123);
        }
    }

    #[test]
    fn loads_example_logo() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../AMOS-Professional-365/AMOS/Examples/Iff/Logo.iff"
        );
        let Ok(d) = std::fs::read(path) else { return };
        let il = parse(&d).unwrap();
        let (w, h, planes, _, _) = screen_params(&il).unwrap();
        let mut out = vec![0u8; (w * h) as usize];
        decode_body(&il, &mut out, w as usize, h as usize).unwrap();
        assert!(planes >= 1);
        assert!(out.iter().any(|&p| p != 0));
    }
}
