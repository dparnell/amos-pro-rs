//! Drawing primitives (Plot, Draw, Box, Bar, Circle, Polygon, Paint, Text).
//!
//! AMOS draws through the screen's graphics.library RastPort
//! (`+Lib.s:9520-10110`), so the pixel rules are the Kickstart ones:
//! lines include both end points, `Bar` (RectFill) is inclusive, the clip
//! rectangle has exclusive right/bottom edges, and writing modes work on
//! bitplanes (`JAM1`=0, `JAM2`=1, `COMPLEMENT`=2, `INVERSVID`=4).
//!
//! The primitives work on a [`Canvas`] (a chunky bitmap) so they can be
//! applied twice when Autoback is active on a double buffered screen.

use super::gfont;

/// Gr Writing bits (graphics.library draw modes).
pub const JAM2: u8 = 1;
pub const COMPLEMENT: u8 = 2;
pub const INVERSVID: u8 = 4;

/// A fill pattern (`Set Pattern`, `SPat` `+W.s:4698`): `height` (a power
/// of 2) rows of 16 pixels per plane, bit 15 = leftmost pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub height: usize,
    /// Number of planes: 1 = normal pattern drawn with Ink/paper, more =
    /// multicolour pattern (AreaPtSz negative).
    pub planes: usize,
    /// `planes` blocks of `height` rows.
    pub rows: Vec<u16>,
}

impl Pattern {
    /// Pattern bit (single plane patterns) at a pixel, rows indexed by `py`.
    fn bit(&self, px: i32, py: i32) -> bool {
        let row = self.rows[(py as usize) & (self.height - 1)];
        row & (0x8000 >> (px & 15)) != 0
    }

    /// Colour of a multicolour pattern at a pixel.
    fn colour(&self, px: i32, py: i32) -> u8 {
        let mut c = 0;
        for p in 0..self.planes.min(8) {
            let row = self.rows[p * self.height + ((py as usize) & (self.height - 1))];
            if row & (0x8000 >> (px & 15)) != 0 {
                c |= 1 << p;
            }
        }
        c
    }

    /// Builds a pattern from a planar image (`SPat2`): only the first word
    /// column is used and the height is rounded down to a power of 2 (at
    /// most 128). `None` when the image is too small or too tall.
    pub fn from_image(
        width_words: usize,
        height: usize,
        planes: usize,
        planar: &[u8],
    ) -> Option<Pattern> {
        // SPat3: find the power of 2 <= height (error if >= 256).
        let mut d0 = 1usize;
        let mut d3 = 0;
        loop {
            if height == d0 {
                break;
            }
            if d0 > height {
                d3 -= 1;
                if d3 <= 0 {
                    return None;
                }
                d0 >>= 1;
                break;
            }
            d0 <<= 1;
            d3 += 1;
            if d3 >= 8 {
                return None;
            }
        }
        if d0 == 0 || width_words == 0 || planes == 0 {
            return None;
        }
        let row_bytes = width_words * 2;
        let plane_bytes = row_bytes * height;
        let mut rows = Vec::with_capacity(d0 * planes);
        for p in 0..planes {
            for r in 0..d0 {
                let o = p * plane_bytes + r * row_bytes;
                let w = match (planar.get(o), planar.get(o + 1)) {
                    (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]),
                    _ => 0,
                };
                rows.push(w);
            }
        }
        Some(Pattern {
            height: d0,
            planes,
            rows,
        })
    }
}

/// The built-in fill patterns: images 4 and up of the default mouse bank
/// (`+AMOSPro_Mouse.abk`, `SoMouse` `+W.s:4796`). `Set Pattern n` (n > 0)
/// uses image 3+n.
static MOUSE_BANK: &[u8] =
    include_bytes!("../../../../AMOS-Professional-365/bin/+AMOSPro_Mouse.abk");

/// Returns built-in pattern `n` (1..), or None if the mouse bank has no
/// such image (`SPatE`: Illegal function call).
pub fn builtin_pattern(n: usize) -> Option<Pattern> {
    let d = MOUSE_BANK;
    if n == 0 || d.len() < 6 || &d[0..4] != b"AmSp" {
        return None;
    }
    let count = u16::from_be_bytes([d[4], d[5]]) as usize;
    let index = 3 + n;
    if index >= count {
        return None;
    }
    let mut pos = 6;
    for i in 0..=index {
        let rd = |o: usize| -> Option<usize> {
            Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]) as usize)
        };
        let (w, h, p) = (rd(pos)?, rd(pos + 2)?, rd(pos + 4)?);
        let len = w * h * p * 2;
        if i == index {
            return Pattern::from_image(w, h, p, d.get(pos + 10..pos + 10 + len)?);
        }
        pos += 10 + len;
    }
    None
}

/// Graphic state of a screen: the RastPort fields AMOS uses (`EcInkA`,
/// `EcMode`, `EcLine`, clip, pattern, graphic cursor, text style, font).
#[derive(Clone, Debug)]
pub struct GrState {
    /// Ink (APen), paper (BPen) and outline pen (AOlPen).
    pub ink: u8,
    pub paper: u8,
    pub outline: u8,
    /// Gr Writing mode: JAM1=0, JAM2=1, COMPLEMENT=2, INVERSVID=4.
    pub writing: u8,
    /// Set Line pattern (bit 15 first).
    pub line_pattern: u16,
    /// Current bit of the line pattern (graphics.library `linpatcnt`): the
    /// pattern continues from one line to the next.
    pub line_count: u8,
    /// Graphic cursor (cp_x, cp_y).
    pub x: i32,
    pub y: i32,
    /// Clip rectangle (x1/y1 exclusive).
    pub clip: (i32, i32, i32, i32),
    /// Set Pattern: number and pattern (None = solid).
    pub pattern_number: i32,
    pub pattern: Option<Pattern>,
    /// Set Paint 1: outline Bar and Polygon with the outline pen.
    pub paint_outline: bool,
    /// Set Text style (AlgoStyle): bit 0 underline, 1 bold, 2 italic.
    pub text_style: u8,
    /// Set Font number (0 = default topaz 8).
    pub font: i32,
}

impl Default for GrState {
    fn default() -> Self {
        GrState {
            ink: 2,
            paper: 1,
            outline: 2,
            writing: JAM2,
            line_pattern: 0xFFFF,
            line_count: 15,
            x: 0,
            y: 0,
            clip: (0, 0, i32::MAX, i32::MAX),
            pattern_number: 0,
            pattern: None,
            paint_outline: false,
            text_style: 0,
            font: 0,
        }
    }
}

/// A chunky bitmap being drawn into.
pub struct Canvas<'a> {
    pub buf: &'a mut [u8],
    pub w: i32,
    pub h: i32,
    /// Mask of the valid colour bits (number of planes).
    pub mask: u8,
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u8], w: u32, h: u32, planes: u8) -> Self {
        let mask = ((1u32 << planes.min(8)) - 1) as u8;
        Canvas {
            buf,
            w: w as i32,
            h: h as i32,
            mask,
        }
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> Option<u8> {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            None
        } else {
            Some(self.buf[(y * self.w + x) as usize])
        }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: u8) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            self.buf[(y * self.w + x) as usize] = c & self.mask;
        }
    }
}

/// Effective clip rectangle: the user clip intersected with the bitmap.
fn clip_rect(gr: &GrState, c: &Canvas) -> (i32, i32, i32, i32) {
    (
        gr.clip.0.max(0),
        gr.clip.1.max(0),
        gr.clip.2.min(c.w),
        gr.clip.3.min(c.h),
    )
}

impl GrState {
    #[inline]
    fn in_clip(&self, c: &Canvas, x: i32, y: i32) -> bool {
        let (x0, y0, x1, y1) = clip_rect(self, c);
        x >= x0 && y >= y0 && x < x1 && y < y1
    }

    /// Writes one pixel through the draw mode. `on` is the pattern bit
    /// (already inverted by INVERSVID): set bits use the ink, clear bits the
    /// paper in JAM2 and are left alone in JAM1; COMPLEMENT inverts every
    /// plane where the bit is set.
    #[inline]
    fn put(&self, c: &mut Canvas, x: i32, y: i32, on: bool, ink: u8) {
        if !self.in_clip(c, x, y) {
            return;
        }
        let i = (y * c.w + x) as usize;
        if self.writing & COMPLEMENT != 0 {
            if on {
                c.buf[i] ^= c.mask;
            }
        } else if on {
            c.buf[i] = ink & c.mask;
        } else if self.writing & JAM2 != 0 {
            c.buf[i] = self.paper & c.mask;
        }
    }

    fn inverse(&self) -> bool {
        self.writing & INVERSVID != 0
    }

    /// `WritePixel` at (x,y) in the ink (`Plot`).
    pub fn plot(&self, c: &mut Canvas, x: i32, y: i32) {
        self.put(c, x, y, true, self.ink);
    }

    /// `ReadPixel` (`Point`): -1 outside the bitmap.
    pub fn point(&self, c: &Canvas, x: i32, y: i32) -> i32 {
        c.get(x, y).map_or(-1, |v| v as i32)
    }

    /// Next bit of the line pattern.
    fn line_bit(&mut self) -> bool {
        let b = self.line_pattern & (1 << self.line_count) != 0;
        self.line_count = if self.line_count == 0 {
            15
        } else {
            self.line_count - 1
        };
        b ^ self.inverse()
    }

    /// Bresenham line from (x0,y0) to (x1,y1), both ends included unless
    /// `skip_first` / `skip_last`. Uses the line pattern and the ink `pen`.
    #[allow(clippy::too_many_arguments)]
    fn line_pen(
        &mut self,
        c: &mut Canvas,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        skip_first: bool,
        skip_last: bool,
        pen: u8,
    ) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        let mut first = true;
        loop {
            let last = x == x1 && y == y1;
            if !(first && skip_first) && !(last && skip_last) {
                let on = self.line_bit();
                self.put(c, x, y, on, pen);
            }
            if last {
                break;
            }
            first = false;
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// `Draw` from the graphic cursor to (x,y); the cursor moves to (x,y).
    pub fn draw_to(&mut self, c: &mut Canvas, x: i32, y: i32) {
        let (x0, y0) = (self.x, self.y);
        self.line_pen(c, x0, y0, x, y, false, false, self.ink);
        self.x = x;
        self.y = y;
    }

    /// `PolyDraw`: lines from the cursor through `points`. Shared vertices
    /// are drawn once, so COMPLEMENT mode gives clean joints; with `close`
    /// the last line stops one pixel short of the start point (Box).
    pub fn poly_draw(&mut self, c: &mut Canvas, points: &[(i32, i32)], close: bool, pen: u8) {
        let n = points.len();
        for (i, &(x, y)) in points.iter().enumerate() {
            let (x0, y0) = (self.x, self.y);
            let skip_last = close && i + 1 == n;
            self.line_pen(c, x0, y0, x, y, i > 0, skip_last, pen);
            self.x = x;
            self.y = y;
        }
    }

    /// `Box x1,y1 To x2,y2` (`InBox` `+Lib.s:9673`): PolyDraw from (x1,y1)
    /// through (x1,y2),(x2,y2),(x2,y1) back to the start.
    pub fn draw_box(&mut self, c: &mut Canvas, x1: i32, y1: i32, x2: i32, y2: i32) {
        self.x = x1;
        self.y = y1;
        let pts = [(x1, y2), (x2, y2), (x2, y1), (x1, y1)];
        self.poly_draw(c, &pts, true, self.ink);
        self.x = x1;
        self.y = y1;
    }

    /// Area fill pixel: pattern bit (or multicolour pattern) with the
    /// draw mode. `py` is the pattern row index.
    #[inline]
    fn fill_pixel(&self, c: &mut Canvas, x: i32, y: i32, py: i32) {
        match &self.pattern {
            None => self.put(c, x, y, !self.inverse(), self.ink),
            Some(p) if p.planes == 1 => {
                let on = p.bit(x, py) ^ self.inverse();
                self.put(c, x, y, on, self.ink);
            }
            Some(p) => {
                // Multicolour pattern: the pattern planes are the colour.
                let col = p.colour(x, py);
                if self.writing & COMPLEMENT != 0 {
                    if self.in_clip(c, x, y) {
                        let i = (y * c.w + x) as usize;
                        c.buf[i] ^= col & c.mask;
                    }
                } else if (col != 0 || self.writing & JAM2 != 0) && self.in_clip(c, x, y) {
                    c.set(x, y, col);
                }
            }
        }
    }

    /// `RectFill` (`Bar`): inclusive rectangle with the area pattern; the
    /// outline pen draws the border when Set Paint is on.
    pub fn bar(&mut self, c: &mut Canvas, x1: i32, y1: i32, x2: i32, y2: i32) {
        let (cx0, cy0, cx1, cy1) = clip_rect(self, c);
        for y in y1.max(cy0)..=y2.min(cy1 - 1) {
            for x in x1.max(cx0)..=x2.min(cx1 - 1) {
                self.fill_pixel(c, x, y, y);
            }
        }
        if self.paint_outline {
            let saved = (self.x, self.y, self.line_pattern, self.line_count);
            self.line_pattern = 0xFFFF;
            self.x = x1;
            self.y = y1;
            let pts = [(x1, y2), (x2, y2), (x2, y1), (x1, y1)];
            let pen = self.outline;
            self.poly_draw(c, &pts, true, pen);
            (self.x, self.y, self.line_pattern, self.line_count) = saved;
        }
        self.x = x1;
        self.y = y1;
    }

    /// `DrawEllipse` centred on (cx,cy): outline only, each pixel drawn once.
    pub fn ellipse(&mut self, c: &mut Canvas, cx: i32, cy: i32, rx: i32, ry: i32) {
        let (a, b) = (rx.abs() as i64, ry.abs() as i64);
        let mut pts: Vec<(i32, i32)> = Vec::new();
        let add4 = |x: i64, y: i64, pts: &mut Vec<(i32, i32)>| {
            let (x, y) = (x as i32, y as i32);
            pts.push((cx + x, cy + y));
            pts.push((cx - x, cy + y));
            pts.push((cx + x, cy - y));
            pts.push((cx - x, cy - y));
        };
        if a == 0 || b == 0 {
            // Degenerate: a line.
            for x in -a..=a {
                for y in -b..=b {
                    pts.push((cx + x as i32, cy + y as i32));
                }
            }
        } else {
            // Midpoint ellipse algorithm, region 1 then region 2.
            let (a2, b2) = (a * a, b * b);
            let (mut x, mut y) = (0i64, b);
            let mut d1 = 4 * b2 - 4 * a2 * b + a2;
            while b2 * x * 2 < a2 * y * 2 {
                add4(x, y, &mut pts);
                if d1 < 0 {
                    d1 += 4 * b2 * (2 * x + 3);
                } else {
                    d1 += 4 * b2 * (2 * x + 3) + 4 * a2 * (-2 * y + 2);
                    y -= 1;
                }
                x += 1;
            }
            let mut d2 = b2 * (2 * x + 1) * (2 * x + 1) + 4 * a2 * (y - 1) * (y - 1) - 4 * a2 * b2;
            while y >= 0 {
                add4(x, y, &mut pts);
                if d2 > 0 {
                    d2 += 4 * a2 * (-2 * y + 3);
                } else {
                    d2 += 4 * b2 * (2 * x + 2) + 4 * a2 * (-2 * y + 3);
                    x += 1;
                }
                y -= 1;
            }
        }
        pts.sort_unstable();
        pts.dedup();
        for (x, y) in pts {
            self.put(c, x, y, true, self.ink);
        }
    }

    /// Area fill of a polygon (`AreaMove`/`AreaDraw`/`AreaEnd`): even-odd
    /// rule, boundary pixels included (blitter inclusive fill), then the
    /// outline in the outline pen when Set Paint is on.
    pub fn polygon(&mut self, c: &mut Canvas, points: &[(i32, i32)]) {
        if points.is_empty() {
            return;
        }
        let (cx0, cy0, cx1, cy1) = clip_rect(self, c);
        let n = points.len();
        let ymin = points.iter().map(|p| p.1).min().unwrap().max(cy0);
        let ymax = points.iter().map(|p| p.1).max().unwrap().min(cy1 - 1);
        let mut xs: Vec<i32> = Vec::new();
        for y in ymin..=ymax {
            xs.clear();
            for i in 0..n {
                let (xa, ya) = points[i];
                let (xb, yb) = points[(i + 1) % n];
                if ya == yb {
                    continue;
                }
                let (xt, yt, xbm, ybm) = if ya < yb {
                    (xa, ya, xb, yb)
                } else {
                    (xb, yb, xa, ya)
                };
                if y < yt || y >= ybm {
                    continue;
                }
                // Rounded intersection.
                let num = (y - yt) as i64 * (xbm - xt) as i64;
                let den = (ybm - yt) as i64;
                let x = xt as i64 + (2 * num + den.signum() * den).div_euclid(2 * den);
                xs.push(x as i32);
            }
            xs.sort_unstable();
            for pair in xs.chunks(2) {
                if pair.len() == 2 {
                    for x in pair[0].max(cx0)..=pair[1].min(cx1 - 1) {
                        self.fill_pixel(c, x, y, y);
                    }
                }
            }
        }
        if self.paint_outline && n > 1 {
            let saved = (self.x, self.y, self.line_pattern, self.line_count);
            self.line_pattern = 0xFFFF;
            self.x = points[0].0;
            self.y = points[0].1;
            let mut pts: Vec<(i32, i32)> = points[1..].to_vec();
            pts.push(points[0]);
            let pen = self.outline;
            self.poly_draw(c, &pts, true, pen);
            (self.x, self.y, self.line_pattern, self.line_count) = saved;
        }
    }

    /// `Paint` (`TPaint` `+W.s:4309`): fills the 4-connected region of the
    /// seed colour inside the clip rectangle with the pattern (ink where the
    /// pattern bit is set, paper elsewhere). The draw mode is not used.
    pub fn paint(&self, c: &mut Canvas, x: i32, y: i32, seed: u8) {
        let (cx0, cy0, cx1, cy1) = clip_rect(self, c);
        if x < cx0 || y < cy0 || x >= cx1 || y >= cy1 {
            return;
        }
        let cw = (cx1 - cx0) as usize;
        let chh = (cy1 - cy0) as usize;
        let mut done = vec![false; cw * chh];
        let idx = |x: i32, y: i32| (y - cy0) as usize * cw + (x - cx0) as usize;
        let matches = |c: &Canvas, x: i32, y: i32| c.buf[(y * c.w + x) as usize] == seed;
        let mut stack = vec![(x, y)];
        let mut filled: Vec<(i32, i32, i32)> = Vec::new();
        while let Some((sx, sy)) = stack.pop() {
            if done[idx(sx, sy)] || !matches(c, sx, sy) {
                continue;
            }
            let mut l = sx;
            while l > cx0 && !done[idx(l - 1, sy)] && matches(c, l - 1, sy) {
                l -= 1;
            }
            let mut r = sx;
            while r + 1 < cx1 && !done[idx(r + 1, sy)] && matches(c, r + 1, sy) {
                r += 1;
            }
            for xx in l..=r {
                done[idx(xx, sy)] = true;
            }
            filled.push((l, r, sy));
            for ny in [sy - 1, sy + 1] {
                if ny < cy0 || ny >= cy1 {
                    continue;
                }
                let mut xx = l;
                while xx <= r {
                    if !done[idx(xx, ny)] && matches(c, xx, ny) {
                        stack.push((xx, ny));
                        while xx <= r && !done[idx(xx, ny)] && matches(c, xx, ny) {
                            xx += 1;
                        }
                    } else {
                        xx += 1;
                    }
                }
            }
        }
        // Pattern rows count from the top of the clip rectangle (the blit
        // starts there); columns from the word aligned left edge.
        for (l, r, y) in filled {
            let py = y - cy0;
            for x in l..=r {
                let col = match &self.pattern {
                    None => self.ink,
                    Some(p) if p.planes == 1 => {
                        if p.bit(x, py) {
                            self.ink
                        } else {
                            self.paper
                        }
                    }
                    Some(p) => p.colour(x, py),
                };
                c.set(x, y, col);
            }
        }
    }

    /// Graphic `Text` at (x, baseline y) in the current font and style.
    /// Returns the advance in pixels.
    pub fn text(&mut self, c: &mut Canvas, x: i32, y: i32, s: &[u8]) -> i32 {
        let base = gfont::BASELINE;
        let top = y - base;
        let bold = self.text_style & 2 != 0;
        let italic = self.text_style & 4 != 0;
        let under = self.text_style & 1 != 0;
        let inv = self.inverse();
        let mut cx = x;
        for &ch in s {
            let g = gfont::glyph(ch);
            let cell_w = gfont::WIDTH + bold as i32;
            for row in 0..gfont::HEIGHT {
                // 9 bit row, bit 8 = leftmost pixel; bold smears one pixel
                // to the right.
                let mut bits = (g[row as usize] as u32) << 1;
                if bold {
                    bits |= bits >> 1;
                }
                if under && row == base + 1 {
                    bits = 0x1FF;
                }
                // Italic: rows above the baseline lean right (graphics.library
                // shifts by (baseline - row) / 2).
                let shift = if italic { (base - row) >> 1 } else { 0 };
                for px in 0..cell_w {
                    let on = bits & (0x100 >> px) != 0;
                    self.put(c, cx + px + shift, top + row, on ^ inv, self.ink);
                }
            }
            cx += gfont::WIDTH;
        }
        cx - x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(buf: &mut [u8]) -> Canvas<'_> {
        Canvas::new(buf, 32, 32, 4)
    }

    fn count(buf: &[u8], c: u8) -> usize {
        buf.iter().filter(|&&p| p == c).count()
    }

    #[test]
    fn line_includes_both_ends() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            ink: 3,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.x = 1;
        g.y = 1;
        g.draw_to(&mut c, 10, 1);
        assert_eq!(count(&b, 3), 10);
        assert_eq!((g.x, g.y), (10, 1));
    }

    #[test]
    fn line_pattern_jam1_jam2() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            ink: 3,
            paper: 5,
            line_pattern: 0xF0F0,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.draw_to(&mut c, 15, 0);
        assert_eq!(&b[0..16], &[3, 3, 3, 3, 5, 5, 5, 5, 3, 3, 3, 3, 5, 5, 5, 5]);
        let mut b = vec![0; 32 * 32];
        g.writing = 0;
        g.line_count = 15;
        g.x = 0;
        let mut c = canvas(&mut b);
        g.draw_to(&mut c, 15, 0);
        assert_eq!(&b[0..8], &[3, 3, 3, 3, 0, 0, 0, 0]);
    }

    #[test]
    fn box_complement_draws_each_pixel_once() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            writing: COMPLEMENT,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.draw_box(&mut c, 2, 2, 10, 8);
        // Perimeter of a 9x7 box = 2*9 + 2*5 = 28 pixels, all inverted.
        assert_eq!(count(&b, 15), 28);
        assert_eq!(b[2 * 32 + 2], 15);
    }

    #[test]
    fn bar_inclusive_and_clip() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            ink: 4,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.bar(&mut c, 0, 0, 3, 3);
        assert_eq!(count(&b, 4), 16);
        let mut b = vec![0; 32 * 32];
        g.clip = (1, 1, 3, 3);
        let mut c = canvas(&mut b);
        g.bar(&mut c, 0, 0, 9, 9);
        assert_eq!(count(&b, 4), 4);
    }

    #[test]
    fn paint_fills_region() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            ink: 1,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.draw_box(&mut c, 0, 0, 9, 9);
        g.ink = 2;
        g.paint(&mut c, 5, 5, 0);
        assert_eq!(count(&b, 2), 64);
        assert_eq!(count(&b, 1), 36);
    }

    #[test]
    fn circle_is_symmetric() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            ink: 1,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.ellipse(&mut c, 16, 16, 5, 5);
        assert_eq!(b[16 * 32 + 21], 1);
        assert_eq!(b[16 * 32 + 11], 1);
        assert_eq!(b[11 * 32 + 16], 1);
        assert_eq!(b[21 * 32 + 16], 1);
        assert_eq!(b[16 * 32 + 16], 0);
    }

    #[test]
    fn polygon_fills_triangle() {
        let mut b = vec![0; 32 * 32];
        let mut g = GrState {
            ink: 7,
            ..Default::default()
        };
        let mut c = canvas(&mut b);
        g.polygon(&mut c, &[(0, 0), (20, 0), (0, 20)]);
        assert_eq!(b[5 * 32 + 5], 7);
        assert_eq!(b[19 * 32 + 19], 0);
    }

    #[test]
    fn builtin_patterns_decode() {
        let p = builtin_pattern(1).expect("pattern 1");
        assert!(p.height.is_power_of_two());
        assert!(builtin_pattern(200).is_none());
    }
}
