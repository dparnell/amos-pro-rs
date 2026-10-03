//! Text windows (`WOpen`, `COut`... in `+W.s:13188-16670`).
//!
//! The original works on bitplanes, one byte (8 pixels) per character
//! column. Here bitmaps are chunky, so every planar operation is done on the
//! bits of the pixel values: bit `p` of a pixel is plane `p`.
//!
//! Positions follow the original structure (`+Equ.s:656`): X in bytes
//! (character columns of the screen), Y in pixels, sizes in characters.
//! Errors are the library codes of `+W.s` (`EcWiErr` adds 44).

use super::font::glyph;
use super::screen::Screen;

/// `DefCurs` (+W.s:16661): a two line underline.
pub const DEF_CURS: [u8; 8] = [0, 0, 0, 0, 0, 0, 0xFF, 0xFF];

// Library error codes.
pub const E_NOT_OPENED: u16 = 10;
pub const E_ALREADY_OPENED: u16 = 11;
pub const E_TOO_SMALL: u16 = 12;
pub const E_TOO_BIG: u16 = 13;
pub const E_ILLEGAL: u16 = 16;
pub const E_WINDOW0: u16 = 18;
pub const E_NO_BORDER: u16 = 19;

/// Length of the `Repeat$` buffer (`WiRepL`).
const REP_LEN: usize = 80;

/// Border styles (`Brd`, +W.s:16565): TL, TR, top, right, BL, BR, bottom,
/// left.
const BORDERS: [[u8; 8]; 7] = [
    [136, 138, 137, 139, 140, 141, 137, 139],
    [128, 130, 129, 132, 133, 135, 134, 131],
    [157, 2, 1, 3, 6, 4, 5, 7],
    [8, 10, 9, 11, 14, 12, 13, 15],
    [16, 18, 17, 19, 22, 20, 21, 23],
    [24, 26, 25, 158, 30, 28, 29, 31],
    [32; 8],
];

/// `Border$` frames (`TEncadre`, +W.s:16649): TL, top, TR, right, BR,
/// bottom, BL, left. Row 0 is the spaces row before the table.
const ENCADRE: [[u8; 8]; 8] = [
    [32; 8],
    [136, 137, 138, 139, 141, 137, 140, 139],
    [128, 129, 130, 132, 135, 134, 133, 131],
    [157, 1, 2, 3, 4, 5, 6, 7],
    [8, 9, 10, 11, 12, 13, 14, 15],
    [16, 17, 18, 19, 20, 21, 22, 23],
    [24, 25, 26, 158, 28, 29, 30, 31],
    [32; 8],
];

fn border_style(b: i32) -> &'static [u8; 8] {
    match b {
        1..=6 => &BORDERS[(b - 1) as usize],
        16 => &BORDERS[6],
        _ => &BORDERS[0],
    }
}

/// Outer and inner geometry of a window (`WiAdr`, +W.s:13717).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    /// Outer area: X in bytes, Y in pixels, size in characters.
    pub dx_r: i32,
    pub dy_r: i32,
    pub tx_r: i32,
    pub ty_r: i32,
    /// Outer height in pixels, right (bytes) and bottom (pixels) edges.
    pub ty_p: i32,
    pub fx_r: i32,
    pub fy_r: i32,
    /// Inner area.
    pub dx_i: i32,
    pub dy_i: i32,
    pub tx_i: i32,
    pub ty_i: i32,
}

/// A text window (`WiPrev`... +Equ.s:656).
#[derive(Clone, Debug, Default)]
pub struct Window {
    pub number: i32,
    pub geo: Geometry,
    /// Active area (inner, or outer while drawing the border): size in
    /// characters and position (`WiTx`, `WiTy`, `WiAdhg`).
    pub tx: i32,
    pub ty: i32,
    pub ax: i32,
    pub ay: i32,
    /// Reverse column counter (`WiX` = Tx - column) and line.
    pub wx: i32,
    pub y: i32,
    pub paper: i32,
    pub pen: i32,
    pub cur_col: i32,
    pub bor_pap: i32,
    pub bor_pen: i32,
    pub tab: i32,
    /// Border style 0-16 (`WiBord`).
    pub border: i32,
    /// `WiFlags`: $8000 writing not default, 2 shade, 4 underline.
    pub flags: u16,
    /// Writing mode (0 replace, 1 or, 2 xor, 3 and, 4 ignore) and source
    /// (0 normal, 1 paper only, 2 pen only).
    pub wmode: u8,
    pub wsel: u8,
    /// Control codes printed as glyphs (`WiGraph`).
    pub graph: bool,
    /// `WiSys` bits: scroll, cursor, inverse.
    pub scroll: bool,
    pub cursor: bool,
    pub inverse: bool,
    /// Disabled planes, bit p = plane p (ESC J).
    pub planes_off: u8,
    /// Escape state: 0 none, 2 waiting for the letter, 1 for the parameter.
    pub esc: u8,
    pub esc_par: u8,
    /// Memorised X (reverse counter) and Y.
    pub mx: i32,
    pub my: i32,
    /// `Zone$` start.
    pub zo_dx: i32,
    pub zo_dy: i32,
    /// Cursor shape.
    pub shape: [u8; 8],
    pub title_top: Vec<u8>,
    pub title_bottom: Vec<u8>,
    /// Wind Save buffer: the outer area (tx_r*8 x ty_p pixels).
    pub save: Option<(i32, Vec<u8>)>,
}

impl Window {
    /// Cursor column.
    pub fn col(&self) -> i32 {
        self.tx - self.wx
    }
}

/// Pixels saved under the text cursor (`EcCurS`).
#[derive(Clone, Debug)]
pub struct CursorSave {
    pub bitmap: usize,
    pub x: i32,
    pub y: i32,
    pub pixels: [u8; 64],
}

/// Text windows of a screen.
#[derive(Clone, Debug, Default)]
pub struct TextState {
    /// Windows, front (current) first.
    pub windows: Vec<Window>,
    pub cursor_save: Option<CursorSave>,
    /// `Wind Save` (`EcWiDec`).
    pub wind_save: bool,
    /// Bitmap being drawn into (`EcCurrent`).
    pub target: usize,
    /// `Repeat$` capture buffer (`T_WiRep`).
    pub rep: Option<Vec<u8>>,
    /// `Border$` start (`T_WiEncDX`, `T_WiEncDY`).
    pub enc: (i32, i32),
}

type WR = Result<(), u16>;

/// Pen, paper, disabled planes, flags, writing mode and source of a glyph.
type GlyphStyle = (u8, u8, u8, u16, u8, u8);

/// The 8 bits of a glyph row as a word whose byte k is $FF when bit 7-k
/// is set (pixel k of the row), in memory order.
#[inline]
fn expand_bits(bits: u8) -> u64 {
    const TABLE: [u64; 256] = {
        let mut t = [0u64; 256];
        let mut b = 0;
        while b < 256 {
            let mut bytes = [0u8; 8];
            let mut k = 0;
            while k < 8 {
                if b & (0x80 >> k) != 0 {
                    bytes[k] = 0xFF;
                }
                k += 1;
            }
            t[b] = u64::from_ne_bytes(bytes);
            b += 1;
        }
        t
    };
    TABLE[bits as usize]
}

impl Screen {
    fn win(&self) -> &Window {
        &self.text.windows[0]
    }

    fn win_mut(&mut self) -> &mut Window {
        &mut self.text.windows[0]
    }

    pub fn has_window(&self) -> bool {
        !self.text.windows.is_empty()
    }

    // ------------------------------------------------------------------
    // Pixel helpers
    // ------------------------------------------------------------------

    /// Writes `value` at (x, y) of the target bitmap, leaving the planes
    /// in `keep` untouched.
    #[inline]
    fn put(&mut self, x: i32, y: i32, value: u8, keep: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = (y as u32 * self.width + x as u32) as usize;
        let mask = self.colour_mask();
        let t = self.text.target.min(self.bitmaps.len() - 1);
        let p = &mut self.bitmaps[t][i];
        *p = (*p & keep) | (value & !keep & mask);
    }

    #[inline]
    fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 0;
        }
        let t = self.text.target.min(self.bitmaps.len() - 1);
        self.bitmaps[t][(y as u32 * self.width + x as u32) as usize]
    }

    /// Index in the target bitmap of the pixel span (x, y)..(x + n, y) when
    /// it lies entirely inside the bitmap (the fast paths below write such
    /// spans directly; `put` / `get` clip pixel by pixel otherwise).
    #[inline]
    fn span(&self, x: i32, y: i32, n: i32) -> Option<usize> {
        if x < 0 || y < 0 || n < 0 || x + n > self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }

    /// Index of the bitmap drawn into.
    #[inline]
    fn target_index(&self) -> usize {
        self.text.target.min(self.bitmaps.len() - 1)
    }

    /// Fills a rectangle (X/W in characters) with the window paper, honouring
    /// the plane mask (`ClFin`).
    fn fill_paper(&mut self, xb: i32, y: i32, wc: i32, h: i32) {
        self.version += 1;
        let (paper, keep) = (self.win().paper as u8, self.win().planes_off);
        let v = paper & !keep & self.colour_mask();
        let t = self.target_index();
        for yy in y..y + h {
            if let Some(i) = self.span(xb * 8, yy, wc * 8) {
                for p in &mut self.bitmaps[t][i..i + (wc * 8) as usize] {
                    *p = (*p & keep) | v;
                }
                continue;
            }
            for xx in xb * 8..(xb + wc) * 8 {
                self.put(xx, yy, paper, keep);
            }
        }
    }

    /// Copies `h` pixel rows from `src_y` to `dst_y` over `wc` characters at
    /// `xb` (`Scrolle`).
    fn copy_rows(&mut self, xb: i32, wc: i32, src_y: i32, dst_y: i32, h: i32) {
        self.version += 1;
        let keep = self.win().planes_off;
        let mask = !keep & self.colour_mask();
        let t = self.target_index();
        let n = (wc * 8).max(0) as usize;
        for k in 0..h.max(0) {
            let r = if dst_y < src_y { k } else { h - 1 - k };
            if let (Some(si), Some(di)) = (
                self.span(xb * 8, src_y + r, wc * 8),
                self.span(xb * 8, dst_y + r, wc * 8),
            ) {
                // Whole rows inside the bitmap: same result as `get` / `put`.
                let bm = &mut self.bitmaps[t];
                if si == di {
                    for p in &mut bm[di..di + n] {
                        *p = (*p & keep) | (*p & mask);
                    }
                } else {
                    // Different rows: the spans do not overlap.
                    let (src, dst) = if si < di {
                        let (a, b) = bm.split_at_mut(di);
                        (&a[si..si + n], &mut b[..n])
                    } else {
                        let (a, b) = bm.split_at_mut(si);
                        (&b[..n], &mut a[di..di + n])
                    };
                    if keep == 0 {
                        // (The usual case: the destination is not read.)
                        for (d, &s) in dst.iter_mut().zip(src) {
                            *d = s & mask;
                        }
                    } else {
                        for (d, &s) in dst.iter_mut().zip(src) {
                            *d = (*d & keep) | (s & mask);
                        }
                    }
                }
                continue;
            }
            for xx in xb * 8..(xb + wc) * 8 {
                let v = self.get(xx, src_y + r);
                self.put(xx, dst_y + r, v, keep);
            }
        }
    }

    /// Scrolls `h` rows at `y` of the inner width by one character left
    /// (`ScGFin`) or right (`ScDFin`), filling with paper.
    fn hscroll_rows(&mut self, y: i32, h: i32, left: bool) {
        self.version += 1;
        let g = self.win().geo;
        let (paper, keep) = (self.win().paper as u8, self.win().planes_off);
        let (x0, x1) = (g.dx_i * 8, (g.dx_i + g.tx_i) * 8);
        for yy in y..y + h {
            if left {
                for xx in x0..x1 - 8 {
                    let v = self.get(xx + 8, yy);
                    self.put(xx, yy, v, keep);
                }
                for xx in x1 - 8..x1 {
                    self.put(xx, yy, paper, keep);
                }
            } else {
                for xx in (x0 + 8..x1).rev() {
                    let v = self.get(xx - 8, yy);
                    self.put(xx, yy, v, keep);
                }
                for xx in x0..x0 + 8 {
                    self.put(xx, yy, paper, keep);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Cursor
    // ------------------------------------------------------------------

    /// Pixel position of the cursor cell (`AdCurs`).
    fn cursor_xy(&self) -> (i32, i32) {
        let w = self.win();
        ((w.ax + w.col()) * 8, w.ay + w.y * 8)
    }

    /// `AffCur`: draws the cursor (saving the pixels under it).
    fn aff_cur(&mut self) {
        if !self.has_window() || !self.win().cursor {
            return;
        }
        let (x, y) = self.cursor_xy();
        let (shape, col) = (self.win().shape, self.win().cur_col as u8);
        let mut pixels = [0u8; 64];
        if let (Some(_), Some(_)) = (self.span(x, y, 8), self.span(x, y + 7, 8)) {
            // The cell is inside the bitmap: same as `get` / `put` below.
            let v = col & self.colour_mask();
            let (t, w) = (self.target_index(), self.width as usize);
            let bm = &mut self.bitmaps[t];
            let vv = u64::from_ne_bytes([v; 8]);
            for r in 0..8 {
                let i = (y as usize + r) * w + x as usize;
                let row: &mut [u8; 8] = (&mut bm[i..i + 8]).try_into().unwrap();
                pixels[r * 8..r * 8 + 8].copy_from_slice(row);
                let m = expand_bits(shape[r]);
                *row = ((u64::from_ne_bytes(*row) & !m) | (vv & m)).to_ne_bytes();
            }
        } else {
            self.aff_cur_clipped(x, y, shape, col, &mut pixels);
        }
        self.version += 1;
        self.text.cursor_save = Some(CursorSave {
            bitmap: self.text.target,
            x,
            y,
            pixels,
        });
    }

    fn aff_cur_clipped(&mut self, x: i32, y: i32, shape: [u8; 8], col: u8, pixels: &mut [u8; 64]) {
        for r in 0..8 {
            for c in 0..8 {
                pixels[r * 8 + c] = self.get(x + c as i32, y + r as i32);
                if shape[r] & (0x80 >> c) != 0 {
                    self.put(x + c as i32, y + r as i32, col, 0);
                }
            }
        }
    }

    /// `EffCur`: restores the pixels under the cursor.
    fn eff_cur(&mut self) {
        if !self.has_window() || !self.win().cursor {
            return;
        }
        let Some(cs) = self.text.cursor_save.take() else {
            return;
        };
        let t = std::mem::replace(&mut self.text.target, cs.bitmap);
        if let (Some(_), Some(_)) = (self.span(cs.x, cs.y, 8), self.span(cs.x, cs.y + 7, 8)) {
            // Inside the bitmap: same as the `put`s below.
            let mask = self.colour_mask();
            let (ti, w) = (self.target_index(), self.width as usize);
            let bm = &mut self.bitmaps[ti];
            let mm = u64::from_ne_bytes([mask; 8]);
            for r in 0..8 {
                let i = (cs.y as usize + r) * w + cs.x as usize;
                let saved = u64::from_ne_bytes(cs.pixels[r * 8..r * 8 + 8].try_into().unwrap());
                bm[i..i + 8].copy_from_slice(&(saved & mm).to_ne_bytes());
            }
        } else {
            for r in 0..8 {
                for c in 0..8 {
                    self.put(cs.x + c, cs.y + r, cs.pixels[(r * 8 + c) as usize], 0);
                }
            }
        }
        self.text.target = t;
        self.version += 1;
    }

    // ------------------------------------------------------------------
    // Geometry and window list
    // ------------------------------------------------------------------

    /// `WiAdr`: computes the geometry of a window at byte `dx`, pixel `dy`.
    fn wi_adr(&self, dx: i32, dy: i32, tx: i32, ty: i32, border: bool) -> Result<Geometry, u16> {
        let tligne = (self.width / 8) as i32;
        let (dx, dy) = (dx & 0xFFFF, dy & 0xFFFF);
        let tx = tx & 0xFFFE;
        if tx == 0 {
            return Err(E_TOO_SMALL);
        }
        if dx + tx > tligne {
            return Err(E_TOO_BIG);
        }
        let ty = ty & 0xFFFF;
        if ty == 0 {
            return Err(E_TOO_SMALL);
        }
        let ty_p = ty * 8;
        let fy = dy + ty_p;
        if fy > self.height as i32 {
            return Err(E_TOO_BIG);
        }
        let mut g = Geometry {
            dx_r: dx,
            dy_r: dy,
            tx_r: tx,
            ty_r: ty,
            ty_p,
            fx_r: dx + tx,
            fy_r: fy,
            dx_i: dx,
            dy_i: dy,
            tx_i: tx,
            ty_i: ty,
        };
        if border {
            g.dx_i += 1;
            g.dy_i += 8;
            g.tx_i -= 2;
            g.ty_i -= 2;
            if g.tx_i <= 0 || g.ty_i <= 0 {
                return Err(E_TOO_SMALL);
            }
        }
        Ok(g)
    }

    /// `WiInt`: the inner area becomes the active area.
    fn wi_int(&mut self) {
        let w = self.win_mut();
        w.tx = w.geo.tx_i;
        w.ty = w.geo.ty_i;
        w.ax = w.geo.dx_i;
        w.ay = w.geo.dy_i;
    }

    /// `WiExt`: the outer area becomes the active area.
    fn wi_ext(&mut self) {
        let w = self.win_mut();
        w.tx = w.geo.tx_r;
        w.ty = w.geo.ty_r;
        w.ax = w.geo.dx_r;
        w.ay = w.geo.dy_r;
    }

    /// `WiStore`: saves the pixels of the current window (Wind Save).
    fn wi_store(&mut self) {
        if !self.text.wind_save || !self.has_window() {
            return;
        }
        let g = self.win().geo;
        let w = g.tx_r * 8;
        let mut buf = Vec::with_capacity((w * g.ty_p) as usize);
        let t = std::mem::replace(&mut self.text.target, self.logic);
        for y in g.dy_r..g.fy_r {
            for x in g.dx_r * 8..g.fx_r * 8 {
                buf.push(self.get(x, y));
            }
        }
        self.text.target = t;
        self.win_mut().save = Some((g.tx_r, buf));
    }

    /// `WiEff`: restores window `i` from its buffer for the lines
    /// `y0..y1`, except where windows `clip` (in front of it) are.
    fn wi_eff(&mut self, i: usize, y0: i32, y1: i32, clip: &[usize]) {
        let g = self.text.windows[i].geo;
        self.wi_eff_rect(i, g.dx_r, g.fx_r, y0.max(g.dy_r), y1.min(g.fy_r), clip);
    }

    fn wi_eff_rect(&mut self, i: usize, xb0: i32, xb1: i32, y0: i32, y1: i32, clip: &[usize]) {
        if !self.text.wind_save {
            return;
        }
        let Some((stride, buf)) = self.text.windows[i].save.clone() else {
            return;
        };
        let g = self.text.windows[i].geo;
        let rects: Vec<Geometry> = clip.iter().map(|&j| self.text.windows[j].geo).collect();
        let t = std::mem::replace(&mut self.text.target, self.logic);
        for y in y0..y1 {
            for x in xb0 * 8..xb1 * 8 {
                if rects
                    .iter()
                    .any(|r| x >= r.dx_r * 8 && x < r.fx_r * 8 && y >= r.dy_r && y < r.fy_r)
                {
                    continue;
                }
                let bx = x - g.dx_r * 8;
                let by = y - g.dy_r;
                if bx < 0 || by < 0 || bx >= stride * 8 {
                    continue;
                }
                if let Some(&v) = buf.get((by * stride * 8 + bx) as usize) {
                    self.put(x, y, v, 0);
                }
            }
        }
        self.text.target = t;
        self.version += 1;
    }

    fn find_window(&self, n: i32) -> Option<usize> {
        self.text.windows.iter().position(|w| w.number == n)
    }

    /// Recomputes the per window colour state (`AdColor`): nothing to
    /// cache in the chunky version.
    fn ad_color(&mut self) {}

    // ------------------------------------------------------------------
    // Window instructions
    // ------------------------------------------------------------------

    /// `Wind Open` (`WOpen`, +W.s:13602): x, y in pixels, tx, ty in
    /// characters, border 0-16, `clw` clears the window.
    #[allow(clippy::too_many_arguments)]
    pub fn wind_open(
        &mut self,
        n: i32,
        x: i32,
        y: i32,
        tx: i32,
        ty: i32,
        border: i32,
        clw: bool,
    ) -> WR {
        self.text.target = self.logic;
        if self.find_window(n).is_some() {
            return Err(E_ALREADY_OPENED);
        }
        if !(0..=16).contains(&border) {
            return Err(E_ILLEGAL);
        }
        let mut dx = ((x as u16 >> 4) << 1) as i32;
        if border != 0 {
            dx += 1;
        }
        let geo = self.wi_adr(dx, y, tx, ty, border != 0)?;
        let mut w = Window {
            number: n,
            geo,
            border,
            tab: 4,
            scroll: true,
            ..Default::default()
        };
        if let Some(cur) = self.text.windows.first() {
            w.paper = cur.paper;
            w.pen = cur.pen;
            w.cur_col = cur.cur_col;
            w.bor_pap = cur.bor_pap;
            w.bor_pen = cur.bor_pen;
            w.tab = cur.tab;
        } else if self.planes == 1 {
            (w.paper, w.pen, w.cur_col, w.bor_pap, w.bor_pen) = (0, 1, 1, 0, 1);
        } else {
            (w.paper, w.pen, w.cur_col, w.bor_pap, w.bor_pen) = (1, 2, 3, 1, 2);
        }
        if self.has_window() {
            self.eff_cur();
            self.wi_store();
        }
        // The new window is put in front now so the drawing routines can
        // use it (the original links it at the end; nothing in between
        // depends on the list).
        w.shape = DEF_CURS;
        self.text.windows.insert(0, w);
        if border != 0 {
            let _ = self.des_bord();
        }
        self.wi_int();
        if clw {
            self.clw();
        }
        self.home();
        self.win_mut().cursor = true;
        self.aff_cur();
        Ok(())
    }

    /// `Window n` (`WQWind`, +W.s:13776).
    pub fn window_activate(&mut self, n: i32) -> WR {
        self.text.target = self.logic;
        let i = self.find_window(n).ok_or(E_NOT_OPENED)?;
        if i != 0 {
            self.eff_cur();
            self.wi_store();
            let w = self.text.windows.remove(i);
            self.text.windows.insert(0, w);
            self.wi_eff(0, 0, 10000, &[]);
            self.win_mut().save = None;
            self.aff_cur();
        }
        self.win_mut().esc = 0;
        Ok(())
    }

    /// `Wind Close` (`WDel`, +W.s:14037).
    pub fn wind_close(&mut self) -> WR {
        self.text.target = self.logic;
        if !self.has_window() {
            return Err(E_NOT_OPENED);
        }
        if self.win().number == 0 {
            return Err(E_WINDOW0);
        }
        self.eff_cur();
        self.cclw();
        let old = self.text.windows.remove(0);
        let (y0, y1) = (old.geo.dy_r, old.geo.fy_r);
        if self.has_window() {
            for i in 0..self.text.windows.len() {
                let clip: Vec<usize> = (0..i).collect();
                self.wi_eff(i, y0, y1, &clip);
                if i == 0 {
                    self.win_mut().save = None;
                }
            }
            self.aff_cur();
        }
        Ok(())
    }

    /// `Cls` without parameters (`WiCls`, +W.s:14087): closes every window
    /// but 0, which is cleared.
    pub fn cls_windows(&mut self) {
        self.text.target = self.logic;
        if !self.has_window() {
            return;
        }
        self.eff_cur();
        self.text.windows.retain(|w| w.number == 0);
        if !self.has_window() {
            return;
        }
        self.clw();
        self.win_mut().save = None;
        self.aff_cur();
    }

    /// `Wind Move x,y` (`WiMove`, +W.s:13825); None keeps the old value.
    pub fn wind_move(&mut self, x: Option<i32>, y: Option<i32>) -> WR {
        self.text.target = self.logic;
        if self.win().number == 0 {
            return Err(E_WINDOW0);
        }
        self.eff_cur();
        self.wi_store();
        self.repaint_behind();
        let old = self.win().geo;
        let border = self.win().border != 0;
        let dx = match x {
            Some(x) => (((x as u16) >> 4) << 1) as i32 + border as i32,
            None => old.dx_r,
        };
        let dy = y.unwrap_or(old.dy_r);
        let res = self.wi_adr(dx, dy, old.tx_r, old.ty_r, border);
        if let Ok(g) = res {
            self.win_mut().geo = g;
        }
        self.wi_int();
        self.wi_eff(0, 0, 10000, &[]);
        self.win_mut().save = None;
        self.aff_cur();
        res.map(|_| ())
    }

    /// `Wind Size w,h` (`WiSize`, +W.s:13895).
    pub fn wind_size(&mut self, tx: Option<i32>, ty: Option<i32>) -> WR {
        self.text.target = self.logic;
        if self.win().number == 0 {
            return Err(E_WINDOW0);
        }
        self.eff_cur();
        self.wi_store();
        let old = self.win().geo;
        self.repaint_behind();
        let border = self.win().border != 0;
        let res = self.wi_adr(
            old.dx_r,
            old.dy_r,
            tx.unwrap_or(old.tx_r),
            ty.unwrap_or(old.ty_r),
            border,
        );
        if let Ok(g) = res {
            self.win_mut().geo = g;
        }
        self.wi_int();
        if border {
            let _ = self.des_bord();
        }
        self.clw();
        // WiEff2: old contents clipped to the new size.
        let g = self.win().geo;
        let wc = old.tx_r.min(g.tx_r);
        let hp = old.ty_p.min(g.ty_p);
        let (mut x0, mut y0, mut x1, mut y1) = (g.dx_r, g.dy_r, g.dx_r + wc, g.dy_r + hp);
        if border {
            x0 += 1;
            y0 += 8;
            x1 -= 1;
            y1 -= 8;
        }
        if self.win().save.is_some() {
            self.wi_eff_rect(0, x0, x1, y0, y1, &[]);
        }
        self.win_mut().save = None;
        self.aff_cur();
        res.map(|_| ())
    }

    /// Restores the windows behind the current one (Wind Move / Size).
    fn repaint_behind(&mut self) {
        let (y0, y1) = (self.win().geo.dy_r, self.win().geo.fy_r);
        for i in 1..self.text.windows.len() {
            let clip: Vec<usize> = (1..i).collect();
            self.wi_eff(i, y0, y1, &clip);
        }
    }

    /// `Border n,paper,pen` (`WSBor`, +W.s:13966).
    pub fn set_border(&mut self, n: Option<i32>, paper: Option<i32>, pen: Option<i32>) -> WR {
        self.text.target = self.logic;
        let nb = self.num_colours();
        if let Some(n) = n {
            if !(0..16).contains(&n) {
                return Err(E_ILLEGAL);
            }
            if n != 0 {
                self.win_mut().border = n;
            }
        }
        if let Some(p) = paper {
            if p as u16 as u32 >= nb {
                return Err(E_ILLEGAL);
            }
            self.win_mut().bor_pap = p;
        }
        if let Some(p) = pen {
            if p as u16 as u32 >= nb {
                return Err(E_ILLEGAL);
            }
            self.win_mut().bor_pen = p;
        }
        self.re_bord();
        Ok(())
    }

    /// `Title Top` / `Title Bottom` (`WSTit`, +W.s:13990).
    pub fn set_title(&mut self, top: Option<&[u8]>, bottom: Option<&[u8]>) -> WR {
        self.text.target = self.logic;
        if self.win().border == 0 {
            return Err(E_NO_BORDER);
        }
        let cut =
            |s: &[u8]| -> Vec<u8> { s.iter().copied().take_while(|&c| c != 0).take(79).collect() };
        if let Some(t) = top {
            self.win_mut().title_top = cut(t);
        }
        if let Some(t) = bottom {
            self.win_mut().title_bottom = cut(t);
        }
        self.re_bord();
        Ok(())
    }

    /// `Set Curs` (`WiSCur`).
    pub fn set_cursor_shape(&mut self, shape: [u8; 8]) {
        self.text.target = self.logic;
        self.eff_cur();
        self.win_mut().shape = shape;
        self.aff_cur();
    }

    /// `ReBord`: redraws the border keeping the cursor position.
    fn re_bord(&mut self) {
        let (wx, y) = (self.win().wx, self.win().y);
        let _ = self.des_bord();
        let w = self.win_mut();
        w.wx = wx;
        w.y = y;
    }

    /// Draws the border (`DesBord`, +W.s:14131).
    fn des_bord(&mut self) -> WR {
        let b = self.win().border;
        if b == 0 {
            return Err(E_NO_BORDER);
        }
        let st = *border_style(b);
        self.wi_ext();
        self.home();
        let tt = self.win().title_top.clone();
        self.dhoriz(st[0], st[1], st[2], &tt);
        let tx = self.win().tx;
        let _ = self.loca(tx - 1, 1);
        self.dvert(st[3]);
        let ty = self.win().ty;
        let _ = self.loca(0, ty - 1);
        let tb = self.win().title_bottom.clone();
        self.dhoriz(st[4], st[5], st[6], &tb);
        let _ = self.loca(0, 1);
        self.dvert(st[7]);
        self.wi_int();
        Ok(())
    }

    /// Saves the window state and sets up border drawing (`SetBord`).
    fn set_bord(&mut self) -> (bool, bool, bool, u16, i32, i32) {
        let w = self.win_mut();
        let saved = (w.scroll, w.cursor, w.inverse, w.flags, w.paper, w.pen);
        w.scroll = false;
        w.graph = true;
        w.flags &= 1;
        w.paper = w.bor_pap;
        w.pen = w.bor_pen;
        saved
    }

    /// `SetNorm`.
    fn set_norm(&mut self, s: (bool, bool, bool, u16, i32, i32)) {
        let w = self.win_mut();
        (w.scroll, w.cursor, w.inverse, w.flags, w.paper, w.pen) = s;
        w.graph = false;
    }

    /// Horizontal border line with its title (`DHoriz`, +W.s:14179).
    fn dhoriz(&mut self, left: u8, right: u8, mid: u8, title: &[u8]) {
        let saved = self.set_bord();
        let y = self.win().y;
        let _ = self.cout(left);
        if self.win().y != y {
            let _ = self.cleft();
        }
        let d6 = self.win().col();
        let tx = self.win().tx;
        let d7 = (tx - 1).max(0).max(d6);
        let _ = self.loca(d7, y);
        if self.win().y == y {
            let _ = self.cout(right);
        }
        let _ = self.loca(d6, y);
        while self.win().col() < d7 {
            if self.cout(mid).is_err() {
                break;
            }
            if self.win().y != y {
                break;
            }
        }
        let _ = self.loca(d6, y);
        for &c in title {
            if c == 0 || self.win().col() >= d7 || self.win().y != y {
                break;
            }
            let _ = self.cout(c);
        }
        self.set_norm(saved);
    }

    /// Vertical border line (`DVert`, +W.s:14268).
    fn dvert(&mut self, c: u8) {
        let saved = self.set_bord();
        let d4 = self.win().col();
        let ty_i = self.win().geo.ty_i;
        for row in 1..=ty_i {
            let _ = self.loca(d4, row);
            let _ = self.cout(c);
        }
        self.set_norm(saved);
    }

    /// `CClw`: erases the border (style 16 = spaces in paper), then CLW.
    fn cclw(&mut self) {
        if self.win().border != 0 {
            let w = self.win_mut();
            w.title_top.clear();
            w.title_bottom.clear();
            w.border = 16;
            w.bor_pap = w.paper;
            let _ = self.des_bord();
        }
        self.clw();
    }

    // ------------------------------------------------------------------
    // Cursor movement and clearing
    // ------------------------------------------------------------------

    /// `Loca` (+W.s:15278): error 16 if outside the window.
    pub(crate) fn loca(&mut self, x: i32, y: i32) -> WR {
        let w = self.win_mut();
        if y as u16 as i32 >= w.ty {
            return Err(E_ILLEGAL);
        }
        let d0 = w.tx as i16 as i32 - x as i16 as i32;
        if d0 <= 0 || x as u16 as i32 > w.tx {
            return Err(E_ILLEGAL);
        }
        w.wx = d0;
        w.y = y as u16 as i32;
        Ok(())
    }

    fn home(&mut self) {
        let w = self.win_mut();
        w.wx = w.tx;
        w.y = 0;
    }

    fn clw(&mut self) {
        let g = self.win().geo;
        self.fill_paper(g.dx_i, g.dy_i, g.tx_i, g.ty_i * 8);
        self.home();
    }

    fn cl_line(&mut self) {
        let g = self.win().geo;
        let y = g.dy_i + self.win().y * 8;
        self.fill_paper(g.dx_i, y, g.tx_i, 8);
    }

    fn cl_eol(&mut self) {
        let (x, y) = self.cursor_xy();
        let n = self.win().wx;
        self.fill_paper(x / 8, y, n, 8);
    }

    fn raz_cur(&mut self, n: i32) {
        let n = (n as u16 as i32).min(self.win().wx);
        let (x, y) = self.cursor_xy();
        if n > 0 {
            self.fill_paper(x / 8, y, n, 8);
        }
    }

    fn cleft(&mut self) -> WR {
        let w = self.win_mut();
        if w.wx + 1 > w.tx {
            w.wx = 1;
            return self.cup();
        }
        w.wx += 1;
        Ok(())
    }

    fn cright(&mut self) -> WR {
        let w = self.win_mut();
        w.wx -= 1;
        if w.wx == 0 {
            w.wx = w.tx;
            return self.cdown();
        }
        Ok(())
    }

    fn cup(&mut self) -> WR {
        let w = self.win_mut();
        w.y -= 1;
        if w.y < 0 {
            if w.scroll {
                w.y = 0;
                self.sc_bas();
            } else {
                w.y = w.ty - 1;
            }
        }
        Ok(())
    }

    fn cdown(&mut self) -> WR {
        let w = self.win_mut();
        let y = w.y + 1;
        if y < w.ty {
            w.y = y;
        } else if w.scroll {
            self.sc_haut();
        } else {
            w.y = 0;
        }
        Ok(())
    }

    fn tab(&mut self) {
        let w = self.win();
        let col = w.col();
        let t = w.tab;
        if t == 0 {
            return;
        }
        let mut d = t;
        while d <= col {
            d += t;
        }
        if d < w.tx {
            let y = w.y;
            let _ = self.loca(d, y);
        }
    }

    /// `ScHaut`: lines 1..Y move up, line Y cleared.
    fn sc_haut(&mut self) {
        let g = self.win().geo;
        let y = self.win().y;
        self.copy_rows(g.dx_i, g.tx_i, g.dy_i + 8, g.dy_i, y * 8);
        self.cl_line();
    }

    /// `ScBas`: lines Y..bottom-1 move down, line Y cleared.
    fn sc_bas(&mut self) {
        let g = self.win().geo;
        let y = self.win().y;
        let n = g.ty_i - y - 1;
        if n > 0 {
            self.copy_rows(g.dx_i, g.tx_i, g.dy_i + y * 8, g.dy_i + (y + 1) * 8, n * 8);
        }
        self.cl_line();
    }

    /// `ScBasHaut`: lines 0..Y-1 move down, top line cleared.
    fn sc_bas_haut(&mut self) {
        let g = self.win().geo;
        let y = self.win().y;
        self.copy_rows(g.dx_i, g.tx_i, g.dy_i, g.dy_i + 8, y * 8);
        self.fill_paper(g.dx_i, g.dy_i, g.tx_i, 8);
    }

    /// `ScHautBas`: lines Y+1..bottom move up, bottom line cleared.
    fn sc_haut_bas(&mut self) {
        let g = self.win().geo;
        let y = self.win().y;
        let n = g.ty_i - y - 1;
        if n > 0 {
            self.copy_rows(g.dx_i, g.tx_i, g.dy_i + (y + 1) * 8, g.dy_i + y * 8, n * 8);
        }
        self.fill_paper(g.dx_i, g.dy_i + (g.ty_i - 1) * 8, g.tx_i, 8);
    }

    // ------------------------------------------------------------------
    // Character output
    // ------------------------------------------------------------------

    /// Prints the plain characters (>= 32) at the start of `text` that stay
    /// on the current line, as `cout` would, with the per character work
    /// done once. Returns how many were printed (0: use `cout`).
    fn cout_run(&mut self, text: &[u8]) -> usize {
        let w = self.win();
        if w.esc != 0 || w.flags != 0 || w.wx <= 1 {
            return 0;
        }
        let (x0, y) = self.cursor_xy();
        let (pen, paper, keep) = (w.pen as u8, w.paper as u8, w.planes_off);
        // Characters that fit before the last column (which wraps: `cout`).
        let room = (w.wx - 1) as usize;
        let n = text.iter().take(room).take_while(|&&c| c >= 32).count();
        if n == 0 || self.span(x0, y, 8 * n as i32).is_none() || self.span(x0, y + 7, 8).is_none() {
            return 0;
        }
        let mask = !keep & self.colour_mask();
        let rep = |b: u8| u64::from_ne_bytes([b; 8]);
        let (fg, bg, kp) = (rep(pen & mask), rep(paper & mask), rep(keep));
        let (t, bw) = (self.target_index(), self.width as usize);
        let bm = &mut self.bitmaps[t];
        let font = super::font::font();
        #[allow(clippy::needless_range_loop)] // (r indexes the glyph rows)
        for r in 0..8 {
            let i = (y as usize + r) * bw + x0 as usize;
            for (px, &c) in bm[i..i + 8 * n]
                .as_chunks_mut::<8>()
                .0
                .iter_mut()
                .zip(&text[..n])
            {
                let m = expand_bits(font[c as usize][r]);
                *px = ((u64::from_ne_bytes(*px) & kp) | (fg & m) | (bg & !m)).to_ne_bytes();
            }
        }
        // `draw_glyph` + `COutFin` for each character (no wrap: n < wx).
        self.version += n as u64;
        self.win_mut().wx -= n as i32;
        n
    }

    /// `COut` (+W.s:15573): one character or control code.
    pub(crate) fn cout(&mut self, c: u8) -> WR {
        if self.win().esc != 0 {
            return self.esc(c);
        }
        if c < 32 && !self.win().graph {
            return self.control(c);
        }
        self.draw_glyph(c);
        // COutFin: one cell right.
        let w = self.win_mut();
        w.wx -= 1;
        if w.wx == 0 {
            w.wx = w.tx;
            let y = w.y + 1;
            if y < w.ty {
                w.y = y;
            } else if w.scroll {
                self.sc_haut();
            } else {
                w.y = 0;
            }
        }
        Ok(())
    }

    fn draw_glyph(&mut self, c: u8) {
        let (x, y) = self.cursor_xy();
        let w = self.win();
        let (pen, paper, keep, flags) = (w.pen as u8, w.paper as u8, w.planes_off, w.flags);
        let (wmode, wsel) = (w.wmode, w.wsel);
        let g = glyph(c);
        self.version += 1;
        if flags == 0 {
            // Fast path: replace.
            if let (Some(_), Some(_)) = (self.span(x, y, 8), self.span(x, y + 7, 8)) {
                // Eight pixels at a time: byte k of the word is pixel k.
                let mask = !keep & self.colour_mask();
                let rep = |b: u8| u64::from_ne_bytes([b; 8]);
                let (fg, bg, kp) = (rep(pen & mask), rep(paper & mask), rep(keep));
                let (t, w) = (self.target_index(), self.width as usize);
                let bm = &mut self.bitmaps[t];
                for (r, &bits) in g.iter().enumerate() {
                    let i = (y as usize + r) * w + x as usize;
                    let px: &mut [u8; 8] = (&mut bm[i..i + 8]).try_into().unwrap();
                    let m = expand_bits(bits);
                    let old = u64::from_ne_bytes(*px);
                    *px = ((old & kp) | (fg & m) | (bg & !m)).to_ne_bytes();
                }
                return;
            }
            for (r, &bits) in g.iter().enumerate() {
                for i in 0..8 {
                    let v = if bits & (0x80 >> i) != 0 { pen } else { paper };
                    self.put(x + i, y + r as i32, v, keep);
                }
            }
            return;
        }
        // Slow path (YaFlag, +W.s:15680), plane by plane.
        if !self.glyph_planes(x, y, g, (pen, paper, keep, flags, wmode, wsel)) {
            self.glyph_planes_clipped(x, y, g, (pen, paper, keep, flags, wmode, wsel));
        }
    }

    /// `draw_glyph` with writing mode flags, 8 pixels at a time; false (and
    /// nothing done) when the cell is not entirely inside the bitmap.
    fn glyph_planes(&mut self, x: i32, y: i32, g: &[u8; 8], style: GlyphStyle) -> bool {
        let (pen, paper, keep, flags, wmode, wsel) = style;
        let under = flags & 4 != 0;
        if let (Some(_), Some(_)) = (self.span(x, y, 8), self.span(x, y + 7, 8)) {
            // The cell is inside the bitmap: the same plane by plane rules
            // on 8 pixels at a time (byte k of a word is pixel k).
            let rep = |b: u8| u64::from_ne_bytes([b; 8]);
            let mask = rep(self.colour_mask());
            let planes = self.planes;
            let (t, w) = (self.target_index(), self.width as usize);
            let bm = &mut self.bitmaps[t];
            let mut shade: u16 = if flags & 2 != 0 { 0xAAAA } else { 0xFFFF };
            for (r, &row) in g.iter().enumerate() {
                let sh = shade as u8;
                shade = shade.rotate_right(1);
                let i = (y as usize + r) * w + x as usize;
                let px: &mut [u8; 8] = (&mut bm[i..i + 8]).try_into().unwrap();
                let old = u64::from_ne_bytes(*px);
                let mut new = old;
                for p in 0..planes {
                    if keep & (1 << p) != 0 {
                        continue;
                    }
                    let pm = if paper & (1 << p) != 0 { 0xFFu8 } else { 0 };
                    let qm = if pen & (1 << p) != 0 { 0xFFu8 } else { 0 };
                    let src = if under && r == 7 {
                        qm & sh
                    } else {
                        let gb = row & sh;
                        let (a, b) = (!gb & pm, gb & qm);
                        match wsel {
                            1 => a,
                            2 => b,
                            _ => a | b,
                        }
                    };
                    let bit = rep(1 << p);
                    let (s8, d) = (expand_bits(src) & bit, old & bit);
                    let res = match wmode {
                        0 => s8,
                        1 => d | s8,
                        2 => d ^ s8,
                        3 => d & s8,
                        _ => d,
                    };
                    new = (new & !bit) | res;
                }
                *px = (new & mask).to_ne_bytes();
            }
            return true;
        }
        false
    }

    /// `draw_glyph` with writing mode flags, pixel by pixel, plane by plane
    /// (clipped cells; the reference of `glyph_planes`).
    fn glyph_planes_clipped(&mut self, x: i32, y: i32, g: &[u8; 8], style: GlyphStyle) {
        let (pen, paper, keep, flags, wmode, wsel) = style;
        let under = flags & 4 != 0;
        let mut shade: u16 = if flags & 2 != 0 { 0xAAAA } else { 0xFFFF };
        for r in 0..8 {
            let sh = shade as u8;
            shade = shade.rotate_right(1);
            for i in 0..8 {
                let bit = 0x80u8 >> i;
                let old = self.get(x + i, y + r);
                let mut new = old;
                for p in 0..self.planes {
                    if keep & (1 << p) != 0 {
                        continue;
                    }
                    let pm = if paper & (1 << p) != 0 { 0xFFu8 } else { 0 };
                    let qm = if pen & (1 << p) != 0 { 0xFFu8 } else { 0 };
                    let src = if under && r == 7 {
                        qm & sh
                    } else {
                        let gb = g[r as usize] & sh;
                        let a = !gb & pm;
                        let b = gb & qm;
                        match wsel {
                            1 => a,
                            2 => b,
                            _ => a | b,
                        }
                    };
                    let d = if old & (1 << p) != 0 { 0xFFu8 } else { 0 };
                    let res = match wmode {
                        0 => src,
                        1 => d | src,
                        2 => d ^ src,
                        3 => d & src,
                        _ => d,
                    };
                    if res & bit != 0 {
                        new |= 1 << p;
                    } else {
                        new &= !(1 << p);
                    }
                }
                self.put(x + i, y + r, new, 0);
            }
        }
    }

    /// Control codes 0-31 (`CCont`, +W.s:16497).
    fn control(&mut self, c: u8) -> WR {
        match c {
            7 => self.cl_eol(),
            8 | 29 => return self.cleft(),
            9 => self.tab(),
            10 | 31 => return self.cdown(),
            12 | 24 => self.home(),
            13 => {
                let w = self.win_mut();
                w.wx = w.tx;
            }
            16 => {
                let y = self.win().geo.dy_i + self.win().y * 8;
                self.hscroll_rows(y, 8, true);
            }
            17 => {
                let g = self.win().geo;
                self.hscroll_rows(g.dy_i, g.ty_i * 8, true);
            }
            18 => {
                let y = self.win().geo.dy_i + self.win().y * 8;
                self.hscroll_rows(y, 8, false);
            }
            19 => {
                let g = self.win().geo;
                self.hscroll_rows(g.dy_i, g.ty_i * 8, false);
            }
            20 => self.sc_bas(),
            21 => self.sc_bas_haut(),
            22 => self.sc_haut(),
            23 => self.sc_haut_bas(),
            25 => self.clw(),
            26 => self.cl_line(),
            27 => self.win_mut().esc = 2,
            28 => return self.cright(),
            30 => return self.cup(),
            _ => {}
        }
        Ok(())
    }

    /// Escape sequences (`Esc`, +W.s:15759).
    fn esc(&mut self, c: u8) -> WR {
        let w = self.win_mut();
        w.esc -= 1;
        if w.esc != 0 {
            w.esc_par = c;
            return Ok(());
        }
        let letter = w.esc_par;
        if !letter.is_ascii_uppercase() {
            return Ok(());
        }
        let d1 = c as i32 - 48;
        let nb = self.num_colours();
        let w = self.win_mut();
        match letter {
            b'B' => {
                if d1 as u16 as u32 >= nb {
                    return Err(E_ILLEGAL);
                }
                if w.inverse {
                    w.inverse = false;
                    w.pen = w.paper;
                }
                w.paper = d1;
            }
            b'P' => {
                if d1 as u16 as u32 >= nb {
                    return Err(E_ILLEGAL);
                }
                if w.inverse {
                    w.inverse = false;
                    w.paper = w.pen;
                }
                w.pen = d1;
            }
            b'C' => w.cursor = d1 as u16 != 0,
            b'D' => {
                if d1 as u16 as u32 >= nb {
                    return Err(E_ILLEGAL);
                }
                w.cur_col = d1;
            }
            b'E' => return self.encadre(d1),
            b'I' => {
                let on = d1 as u16 != 0;
                if on != w.inverse {
                    w.inverse = on;
                    std::mem::swap(&mut w.pen, &mut w.paper);
                }
            }
            b'J' => {
                let mut off = 0u8;
                for p in 0..self.planes {
                    if d1 & (1 << p) == 0 {
                        off |= 1 << p;
                    }
                }
                self.win_mut().planes_off = off;
            }
            b'K' => w.graph = d1 as u16 != 0,
            b'M' => match d1 {
                0 => w.mx = w.wx,
                1 => {
                    if w.mx != 0 && w.mx <= w.tx {
                        w.wx = w.mx;
                    }
                }
                2 => w.my = w.y,
                3 if w.my < w.ty => w.y = w.my,
                _ => {}
            },
            b'N' => {
                let d = (c.wrapping_sub(128)) as i8 as i32;
                let (col, y) = (w.col(), w.y);
                return self.loca(col + d, y);
            }
            b'O' => {
                let d = (c.wrapping_sub(128)) as i8 as i32;
                let (col, y) = (w.col(), w.y);
                return self.loca(col, y + d);
            }
            b'Q' => self.raz_cur(d1),
            b'R' => return self.repete(d1),
            b'S' => {
                if d1 as u16 != 0 {
                    w.flags |= 2;
                } else {
                    w.flags &= !2;
                }
            }
            b'T' => {
                if d1 as u16 as i32 >= w.tx {
                    return Err(E_ILLEGAL);
                }
                w.tab = d1;
            }
            b'U' => {
                if d1 as u16 != 0 {
                    w.flags |= 4;
                } else {
                    w.flags &= !4;
                }
            }
            b'V' => w.scroll = d1 as u16 != 0,
            b'W' => self.writing(d1),
            b'X' => {
                let y = w.y;
                return self.loca(d1, y);
            }
            b'Y' => {
                let col = w.col();
                return self.loca(col, d1);
            }
            b'Z' => return self.wi_zone(d1),
            _ => {}
        }
        self.ad_color();
        Ok(())
    }

    /// `Writing` (+W.s:13223): `n = mode + 8*source`.
    fn writing(&mut self, n: i32) {
        let w = self.win_mut();
        let mode = (n & 7) as u8;
        if mode < 5 {
            w.flags &= !0x8000;
            if mode != 0 {
                w.flags |= 0x8000;
            }
            w.wmode = mode;
        }
        let sel = ((n >> 3) & 3) as u8;
        if sel < 3 {
            if sel != 0 {
                w.flags |= 0x8000;
            }
            w.wsel = sel;
        }
    }

    /// `Repeat$` (`Repete`, +W.s:14947).
    fn repete(&mut self, d1: i32) -> WR {
        match self.text.rep.as_mut() {
            None => {
                if d1 == 0 {
                    // Capture: every following byte comes back here as
                    // the parameter of ESC R.
                    self.text.rep = Some(Vec::new());
                    self.win_mut().esc = 1;
                }
                Ok(())
            }
            Some(buf) => {
                let c = (d1 + 48) as u8;
                let n = buf.len();
                let count = if n >= 2 && buf[n - 2] == 27 && buf[n - 1] == b'R' {
                    buf.truncate(n - 2);
                    (c as i32 - 49).max(0) + 1
                } else {
                    buf.push(c);
                    if buf.len() < REP_LEN - 1 {
                        self.win_mut().esc = 1;
                        return Ok(());
                    }
                    1
                };
                let buf = self.text.rep.take().unwrap_or_default();
                for _ in 0..count {
                    for &b in &buf {
                        if b == 0 {
                            break;
                        }
                        self.cout(b)?;
                    }
                }
                Ok(())
            }
        }
    }

    /// `Zone$` (`WiZone`, +W.s:15049).
    fn wi_zone(&mut self, d1: i32) -> WR {
        if d1 & 0xFF == 0 {
            let w = self.win_mut();
            w.zo_dx = w.col();
            w.zo_dy = w.y;
            return Ok(());
        }
        self.cleft()?;
        let w = self.win();
        let x1 = (w.zo_dx + w.geo.dx_i) * 8;
        let y1 = w.zo_dy * 8 + w.geo.dy_i;
        let x2 = (w.col() + w.geo.dx_i) * 8 + 7;
        let y2 = (w.y + 1) * 8 + w.geo.dy_i;
        let r = self.set_zone(d1 & 0xFF, x1, y1, x2, y2);
        self.cright()?;
        r
    }

    /// `Border$` (`Encadre`, +W.s:15094).
    fn encadre(&mut self, d1: i32) -> WR {
        if d1 & 0xFF == 0 {
            let w = self.win();
            self.text.enc = (w.col(), w.y);
            return Ok(());
        }
        let (wx, wy) = (self.win().wx, self.win().y);
        self.win_mut().graph = true;
        let st = ENCADRE[(d1 & 7) as usize];
        let (edx, edy) = self.text.enc;
        let d3 = self.win().col() - edx;
        let d4 = self.win().y - edy;
        if d3 >= 0 && d4 >= 0 {
            let d3 = d3 - 1;
            let _ = self.loca(edx, edy);
            let _ = self.cleft();
            let _ = self.cup();
            let _ = self.cout(st[0]);
            for _ in 0..=d3 {
                let _ = self.cout(st[1]);
            }
            let _ = self.cout(st[2]);
            let _ = self.cleft();
            let _ = self.cdown();
            for _ in 0..=d4 {
                let _ = self.cout(st[3]);
                let _ = self.cleft();
                let _ = self.cdown();
            }
            let _ = self.cout(st[4]);
            let _ = self.cleft();
            let _ = self.cleft();
            for _ in 0..=d3 {
                let _ = self.cout(st[5]);
                let _ = self.cleft();
                let _ = self.cleft();
            }
            let _ = self.cout(st[6]);
            let _ = self.cleft();
            let _ = self.cup();
            for _ in 0..=d4 {
                let _ = self.cout(st[7]);
                let _ = self.cleft();
                let _ = self.cup();
            }
        }
        let w = self.win_mut();
        w.graph = false;
        w.wx = wx;
        w.y = wy;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Print
    // ------------------------------------------------------------------

    /// Runs `f` with autoback (`AutoPrt`, +W.s:15450): on a double
    /// buffered screen with autoback on, the output is done in both
    /// bitmaps from the same window state.
    fn auto_prt(&mut self, f: &mut dyn FnMut(&mut Screen) -> WR) -> WR {
        if self.autoback != 0 && self.is_double_buffered() && self.has_window() {
            let mut snap = self.win().clone();
            snap.save = None;
            let cur = self.text.cursor_save.clone();
            self.text.target = self.logic;
            let _ = f(self);
            let save = self.win_mut().save.take();
            *self.win_mut() = snap;
            self.win_mut().save = save;
            self.text.cursor_save = cur.map(|mut c| {
                c.bitmap = self.physic;
                c
            });
            self.text.target = self.physic;
            let r = f(self);
            self.text.target = self.logic;
            r
        } else {
            self.text.target = self.logic;
            f(self)
        }
    }

    /// `Print` (`WPrint`): prints until the end or a 0 byte; stops at the
    /// first error.
    pub fn print_text(&mut self, text: &[u8]) -> WR {
        if !self.has_window() {
            return Ok(());
        }
        self.auto_prt(&mut |s: &mut Screen| {
            s.eff_cur();
            let mut r = Ok(());
            let mut i = 0;
            while i < text.len() {
                // Runs of plain characters on one line at once.
                let n = s.cout_run(&text[i..]);
                if n > 0 {
                    i += n;
                    continue;
                }
                let c = text[i];
                if c == 0 {
                    break;
                }
                r = s.cout(c);
                if r.is_err() {
                    break;
                }
                i += 1;
            }
            s.aff_cur();
            r
        })
    }

    /// `Locate x,y` (`WLocate`): None keeps the current value.
    pub fn locate(&mut self, x: Option<i32>, y: Option<i32>) -> WR {
        self.auto_prt(&mut |s: &mut Screen| {
            s.eff_cur();
            let x = x.unwrap_or(s.win().col());
            let y = y.unwrap_or(s.win().y);
            let r = s.loca(x, y);
            s.aff_cur();
            r
        })
    }

    /// `Centre a$` (`WCentre`).
    pub fn centre(&mut self, text: &[u8]) -> WR {
        let text: Vec<u8> = text.iter().copied().take_while(|&c| c != 0).collect();
        let len = printed_len(&text);
        self.auto_prt(&mut |s: &mut Screen| {
            s.eff_cur();
            let x = ((s.win().tx - len) as u16 >> 1) as i32;
            let y = s.win().y;
            let mut r = s.loca(x, y);
            if r.is_ok() {
                for &c in &text {
                    r = s.cout(c);
                    if r.is_err() {
                        break;
                    }
                }
            }
            s.aff_cur();
            r
        })
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Current window number (`=Windon`).
    pub fn windon(&self) -> i32 {
        self.text.windows.first().map_or(0, |w| w.number)
    }

    /// Cursor column and line (`X Curs`, `Y Curs`).
    pub fn cursor_pos(&self) -> (i32, i32) {
        self.text.windows.first().map_or((0, 0), |w| (w.col(), w.y))
    }

    /// `X Graphic(x)`: -1 if outside the window.
    pub fn x_graphic(&self, x: i32) -> i32 {
        let w = self.win();
        if x as u16 as i32 >= w.tx {
            -1
        } else {
            (x + w.geo.dx_i) * 8
        }
    }

    /// `Y Graphic(y)`: -1 if outside the window.
    pub fn y_graphic(&self, y: i32) -> i32 {
        let w = self.win();
        if y as u16 as i32 >= w.ty {
            -1
        } else {
            y * 8 + w.geo.dy_i
        }
    }

    /// `X Text(x)` / `Y Text(y)` (`CXyWi`): None outside the window.
    pub fn x_text(&self, x: i32) -> Option<i32> {
        let w = self.win();
        let c = ((x as u16) >> 3) as i32 - w.geo.dx_i;
        if c < 0 || c >= w.geo.tx_i {
            None
        } else {
            Some(c)
        }
    }

    pub fn y_text(&self, y: i32) -> Option<i32> {
        let w = self.win();
        let d = y as i16 as i32 - w.geo.dy_i;
        if d < 0 {
            return None;
        }
        let l = d / 8;
        if l >= w.geo.ty_i { None } else { Some(l) }
    }
}

/// Printed length of a string: escape sequences take no room (`Compte`).
pub fn printed_len(s: &[u8]) -> i32 {
    let mut n = 0;
    let mut i = 0;
    while i < s.len() && s[i] != 0 {
        if s[i] == 27 {
            i += 3;
        } else {
            n += 1;
            i += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::super::screen::Screens;
    use super::*;

    fn screen() -> Screens {
        let mut ss = Screens::new();
        ss.open(0, 320, 200, 16, 0).unwrap();
        ss
    }

    fn row(s: &Screen, y: i32, x0: i32, n: i32) -> Vec<u8> {
        (x0..x0 + n).map(|x| s.pixel(x, y).unwrap()).collect()
    }

    /// Glyphs in every writing mode, source, shade / underline flag and
    /// plane mask, 8 pixels at a time, against the pixel by pixel version.
    #[test]
    fn glyph_planes_match_pixel_by_pixel() {
        let mut seed = 3u32;
        let mut rnd = |n: u32| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 8) % n
        };
        for colours in [2, 4, 8, 16, 32, 64] {
            for _ in 0..400 {
                let mut a = Screen::new(0, 64, 16, colours, 0);
                for p in a.bitmaps[0].iter_mut() {
                    *p = rnd(256) as u8;
                }
                let mut b = a.clone();
                let g: [u8; 8] = std::array::from_fn(|_| rnd(256) as u8);
                let style = (
                    rnd(256) as u8,
                    rnd(256) as u8,
                    rnd(64) as u8,
                    0x8000 | rnd(8) as u16,
                    rnd(6) as u8,
                    rnd(4) as u8,
                );
                let (x, y) = (rnd(57) as i32, rnd(9) as i32);
                assert!(a.glyph_planes(x, y, &g, style));
                b.glyph_planes_clipped(x, y, &g, style);
                assert!(a.bitmaps[0] == b.bitmaps[0], "{colours} {style:?}");
            }
        }
    }

    #[test]
    fn window0_and_print() {
        let mut ss = screen();
        let s = ss.current_mut().unwrap();
        assert_eq!(s.win().geo.tx_i, 40);
        assert_eq!(s.win().geo.ty_i, 25);
        // Cleared to paper 1, cursor (colour 3) on rows 6-7 of cell 0.
        assert_eq!(s.pixel(0, 0), Some(1));
        assert_eq!(s.pixel(0, 7), Some(3));
        s.print_text(b"A").unwrap();
        assert_eq!(s.cursor_pos(), (1, 0));
        // 'A' row 0 is ...##...
        assert_eq!(row(s, 0, 0, 8), vec![1, 1, 1, 2, 2, 1, 1, 1]);
        s.print_text(b"\r\nB").unwrap();
        assert_eq!(s.cursor_pos(), (1, 1));
    }

    #[test]
    fn scroll_at_bottom() {
        let mut ss = screen();
        let s = ss.current_mut().unwrap();
        s.locate(Some(0), Some(24)).unwrap();
        s.print_text(b"X\r\n").unwrap();
        assert_eq!(s.cursor_pos(), (0, 24));
        // The X moved to line 23.
        assert!(row(s, 23 * 8 + 1, 0, 8).contains(&2));
        assert_eq!(s.locate(Some(40), Some(0)), Err(E_ILLEGAL));
    }

    #[test]
    fn escapes_and_inverse() {
        let mut ss = screen();
        let s = ss.current_mut().unwrap();
        s.print_text(b"\x1bP5\x1bB4").unwrap();
        assert_eq!((s.win().pen, s.win().paper), (5, 4));
        s.print_text(b"\x1bI1").unwrap();
        assert_eq!((s.win().pen, s.win().paper), (4, 5));
        s.print_text(b"\x1bP7").unwrap();
        assert_eq!((s.win().pen, s.win().paper, s.win().inverse), (7, 4, false));
        assert_eq!(s.print_text(b"\x1bP\x7f"), Err(E_ILLEGAL));
        // At / Locate via escapes.
        s.print_text(b"\x1bX5\x1bY3").unwrap();
        assert_eq!(s.cursor_pos(), (5, 3));
        // Repeat$
        s.print_text(b"\x1bR0ab\x1bR3").unwrap();
        assert_eq!(s.cursor_pos(), (11, 3));
    }

    #[test]
    fn windows_open_close() {
        let mut ss = screen();
        let s = ss.current_mut().unwrap();
        s.wind_open(1, 16, 16, 10, 5, 1, true).unwrap();
        let g = s.win().geo;
        assert_eq!((g.dx_r, g.dx_i, g.dy_i, g.tx_i, g.ty_i), (3, 4, 24, 8, 3));
        assert_eq!(s.wind_open(1, 0, 0, 4, 4, 0, true), Err(E_ALREADY_OPENED));
        assert_eq!(s.wind_open(2, 0, 0, 50, 4, 0, true), Err(E_TOO_BIG));
        assert_eq!(s.wind_open(2, 0, 0, 2, 2, 1, true), Err(E_TOO_SMALL));
        // Border glyph 136 (top left corner) at the outer top left.
        assert_eq!(s.pixel(24 + 7, 16 + 3), Some(2));
        s.wind_close().unwrap();
        assert_eq!(s.windon(), 0);
        assert_eq!(s.wind_close(), Err(E_WINDOW0));
    }

    #[test]
    fn writing_modes() {
        let mut ss = screen();
        let s = ss.current_mut().unwrap();
        s.print_text(b"\x1bC0").unwrap();
        s.print_text(b"\x1bX1Q").unwrap();
        let before = row(s, 1, 8, 8);
        // Writing 2,2 (xor, pen only): ESC W chr$(48+2+16).
        s.print_text(b"\x1bWB\x1bX1M\x1bX1M").unwrap();
        assert_eq!(row(s, 1, 8, 8), before);
        s.print_text(b"\x1bX1M").unwrap();
        assert_ne!(row(s, 1, 8, 8), before);
        // Under on: row 7 in pen, background 0.
        s.print_text(b"\x1bW0\x1bU1\x1bX3 ").unwrap();
        assert_eq!(row(s, 7, 24, 8), vec![2; 8]);
    }
}
