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

/// The points of `DrawEllipse` (with repeats), in the reference order.
fn ellipse_points(cx: i32, cy: i32, rx: i32, ry: i32, mut out: impl FnMut(i32, i32)) {
    let (a, b) = (rx.abs() as i64, ry.abs() as i64);
    if a == 0 || b == 0 {
        // Degenerate: a line.
        for x in -a..=a {
            for y in -b..=b {
                out(cx + x as i32, cy + y as i32);
            }
        }
        return;
    }
    let mut add4 = |x: i64, y: i64| {
        let (x, y) = (x as i32, y as i32);
        out(cx + x, cy + y);
        out(cx - x, cy + y);
        out(cx + x, cy - y);
        out(cx - x, cy - y);
    };
    // Midpoint ellipse algorithm, region 1 then region 2.
    let (a2, b2) = (a * a, b * b);
    let (mut x, mut y) = (0i64, b);
    let mut d1 = 4 * b2 - 4 * a2 * b + a2;
    while b2 * x * 2 < a2 * y * 2 {
        add4(x, y);
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
        add4(x, y);
        if d2 > 0 {
            d2 += 4 * a2 * (-2 * y + 3);
        } else {
            d2 += 4 * b2 * (2 * x + 2) + 4 * a2 * (-2 * y + 3);
            x += 1;
        }
        y -= 1;
    }
}

/// The `n` Bresenham points of a line (index `i0` first, `steps` = (dx,
/// -|dy|, x step, row step)) given to `f`, except the first / last ones
/// when skipped.
#[inline]
fn line_pixels(
    buf: &mut [u8],
    i0: isize,
    n: u32,
    steps: (i32, i32, isize, isize),
    skip_first: bool,
    skip_last: bool,
    mut f: impl FnMut(&mut u8),
) {
    let (dx, dy, ix, iy) = steps;
    let mut err = dx + dy;
    let mut i = i0;
    let mut step = |i: &mut isize| {
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            *i += ix;
        }
        if e2 <= dx {
            err += dx;
            *i += iy;
        }
    };
    // Points k in first..end are drawn (k = 0 is i0).
    let first = skip_first as u32;
    let end = if skip_last { n - 1 } else { n };
    if first >= end {
        return;
    }
    if first == 1 {
        step(&mut i);
    }
    for _ in first..end - 1 {
        f(&mut buf[i as usize]);
        step(&mut i);
    }
    f(&mut buf[i as usize]);
}

/// What an area fill does to every pixel of a span, decided once.
#[derive(Clone, Copy)]
enum SpanFill {
    Set(u8),
    Xor(u8),
    Keep,
    /// Pixel by pixel from the fill pattern.
    Pattern(Writer),
}

/// What `GrState::put` needs, computed once per primitive: the clip
/// rectangle (intersected with the bitmap) and the draw mode.
#[derive(Clone, Copy)]
struct Writer {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    w: i32,
    mask: u8,
    complement: bool,
    jam2: bool,
    paper: u8,
}

impl Writer {
    #[inline]
    fn inside(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && y >= self.y0 && x < self.x1 && y < self.y1
    }

    /// `put` at an index known to be inside the clip rectangle.
    #[inline]
    fn write(&self, buf: &mut [u8], i: usize, on: bool, ink: u8) {
        if self.complement {
            if on {
                buf[i] ^= self.mask;
            }
        } else if on {
            buf[i] = ink & self.mask;
        } else if self.jam2 {
            buf[i] = self.paper & self.mask;
        }
    }

    /// What `write` does with bit `on` and colour `ink`.
    #[inline]
    fn op(&self, on: bool, ink: u8) -> SpanFill {
        if self.complement {
            if on {
                SpanFill::Xor(self.mask)
            } else {
                SpanFill::Keep
            }
        } else if on {
            SpanFill::Set(ink & self.mask)
        } else if self.jam2 {
            SpanFill::Set(self.paper & self.mask)
        } else {
            SpanFill::Keep
        }
    }

    #[inline]
    fn put(&self, buf: &mut [u8], x: i32, y: i32, on: bool, ink: u8) {
        if self.inside(x, y) {
            self.write(buf, (y * self.w + x) as usize, on, ink);
        }
    }
}

impl GrState {
    #[inline]
    fn writer(&self, c: &Canvas) -> Writer {
        let (x0, y0, x1, y1) = clip_rect(self, c);
        Writer {
            x0,
            y0,
            x1,
            y1,
            w: c.w,
            mask: c.mask,
            complement: self.writing & COMPLEMENT != 0,
            jam2: self.writing & JAM2 != 0,
            paper: self.paper,
        }
    }

    /// Writes one pixel through the draw mode. `on` is the pattern bit
    /// (already inverted by INVERSVID): set bits use the ink, clear bits the
    /// paper in JAM2 and are left alone in JAM1; COMPLEMENT inverts every
    /// plane where the bit is set.
    #[inline]
    fn put(&self, c: &mut Canvas, x: i32, y: i32, on: bool, ink: u8) {
        self.writer(c).put(c.buf, x, y, on, ink);
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
        let wr = self.writer(c);
        // A full (or empty) line pattern gives every pixel the same bit:
        // the pattern position only advances (once per pixel drawn).
        let constant = match self.line_pattern {
            0xFFFF => Some(!self.inverse()),
            0 => Some(self.inverse()),
            _ => None,
        };
        // The line stays in the bounding box of its ends: when both are
        // inside the clip rectangle no pixel needs clipping.
        let inside = wr.inside(x0, y0) && wr.inside(x1, y1);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        if let (Some(on), true) = (constant, inside) {
            // Every pixel gets the same operation and none is clipped.
            let op = wr.op(on, pen);
            // Bresenham visits max(|dx|, |dy|) + 1 points.
            let n = dx.max(-dy) as u32 + 1;
            let skipped = if n == 1 {
                (skip_first || skip_last) as u32
            } else {
                skip_first as u32 + skip_last as u32
            };
            let i0 = (y0 * wr.w + x0) as isize;
            let steps = (dx, dy, sx as isize, (sy * wr.w) as isize);
            match op {
                SpanFill::Set(v) => {
                    line_pixels(c.buf, i0, n, steps, skip_first, skip_last, |p| *p = v)
                }
                SpanFill::Xor(m) => {
                    line_pixels(c.buf, i0, n, steps, skip_first, skip_last, |p| *p ^= m)
                }
                _ => {}
            }
            let drawn = n - skipped;
            self.line_count = ((self.line_count as u32 + 16 - drawn % 16) % 16) as u8;
            return;
        }
        if inside {
            // No pixel is clipped: the operations for a set and a clear
            // pattern bit are decided once, the pattern position is local.
            let inv = self.inverse();
            let ops = [wr.op(inv, pen), wr.op(!inv, pen)];
            let pattern = self.line_pattern;
            let mut count = self.line_count as u32;
            // Bresenham visits max(|dx|, |dy|) + 1 points.
            let n = dx.max(-dy) as u32 + 1;
            let mut err = dx + dy;
            let mut i = (y0 * wr.w + x0) as isize;
            let (ix, iy) = (sx as isize, (sy * wr.w) as isize);
            for k in 0..n {
                let skip = (k == 0 && skip_first) || (k + 1 == n && skip_last);
                if !skip {
                    // `line_bit`.
                    let bit = (pattern >> count) & 1;
                    count = if count == 0 { 15 } else { count - 1 };
                    let px = &mut c.buf[i as usize];
                    match ops[bit as usize] {
                        SpanFill::Set(v) => *px = v,
                        SpanFill::Xor(m) => *px ^= m,
                        _ => {}
                    }
                }
                let e2 = 2 * err;
                if e2 >= dy {
                    err += dy;
                    i += ix;
                }
                if e2 <= dx {
                    err += dx;
                    i += iy;
                }
            }
            self.line_count = count as u8;
            return;
        }
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        let mut first = true;
        let mut drawn = 0u32;
        loop {
            let last = x == x1 && y == y1;
            if !(first && skip_first) && !(last && skip_last) {
                let on = match constant {
                    Some(b) => {
                        drawn += 1;
                        b
                    }
                    None => self.line_bit(),
                };
                if inside {
                    wr.write(c.buf, (y * wr.w + x) as usize, on, pen);
                } else {
                    wr.put(c.buf, x, y, on, pen);
                }
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
        if constant.is_some() {
            // `line_bit` steps the position down by one, from 0 to 15.
            self.line_count = ((self.line_count as u32 + 16 - drawn % 16) % 16) as u8;
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

    /// Area fill of x in `xa..=xb` on row `y` (all inside the clip
    /// rectangle) with the pattern (row `py`, or the multicolour pattern)
    /// and the draw mode: the reference's `fill_pixel` on every pixel.
    fn fill_span(&self, c: &mut Canvas, f: &SpanFill, y: i32, xa: i32, xb: i32, py: i32) {
        if xa > xb {
            return;
        }
        let row = &mut c.buf[(y * c.w + xa) as usize..=(y * c.w + xb) as usize];
        match *f {
            SpanFill::Set(v) => row.fill(v),
            SpanFill::Xor(m) => row.iter_mut().for_each(|p| *p ^= m),
            SpanFill::Keep => {}
            SpanFill::Pattern(wr) => match &self.pattern {
                Some(p) if p.planes == 1 => {
                    let inv = self.inverse();
                    let bits = p.rows[(py as usize) & (p.height - 1)];
                    for (k, px) in row.iter_mut().enumerate() {
                        let x = xa + k as i32;
                        let on = (bits & (0x8000 >> (x & 15)) != 0) ^ inv;
                        wr.write(std::slice::from_mut(px), 0, on, self.ink);
                    }
                }
                Some(p) => {
                    for (k, px) in row.iter_mut().enumerate() {
                        let col = p.colour(xa + k as i32, py);
                        if wr.complement {
                            *px ^= col & wr.mask;
                        } else if col != 0 || wr.jam2 {
                            *px = col & wr.mask;
                        }
                    }
                }
                None => {}
            },
        }
    }

    /// What an area fill does to each pixel of a span.
    fn span_fill(&self, c: &Canvas) -> SpanFill {
        let wr = self.writer(c);
        if self.pattern.is_some() {
            return SpanFill::Pattern(wr);
        }
        // Solid: `put` with the bit !INVERSVID.
        wr.op(!self.inverse(), self.ink)
    }

    /// `RectFill` (`Bar`): inclusive rectangle with the area pattern; the
    /// outline pen draws the border when Set Paint is on.
    pub fn bar(&mut self, c: &mut Canvas, x1: i32, y1: i32, x2: i32, y2: i32) {
        let (cx0, cy0, cx1, cy1) = clip_rect(self, c);
        let (xa, xb) = (x1.max(cx0), x2.min(cx1 - 1));
        let f = self.span_fill(c);
        let rows = y1.max(cy0)..=y2.min(cy1 - 1);
        match f {
            _ if xa > xb => {}
            SpanFill::Set(v) => {
                for y in rows {
                    c.buf[(y * c.w + xa) as usize..=(y * c.w + xb) as usize].fill(v);
                }
            }
            SpanFill::Xor(m) => {
                for y in rows {
                    c.buf[(y * c.w + xa) as usize..=(y * c.w + xb) as usize]
                        .iter_mut()
                        .for_each(|p| *p ^= m);
                }
            }
            SpanFill::Keep => {}
            SpanFill::Pattern(_) => {
                for y in rows {
                    self.fill_span(c, &f, y, xa, xb, y);
                }
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
        let wr = self.writer(c);
        let ink = self.ink;
        if !wr.complement {
            // Writing the ink is idempotent: the points are drawn as they
            // come (the reference sorts them to draw each pixel once).
            ellipse_points(cx, cy, rx, ry, |x, y| wr.put(c.buf, x, y, true, ink));
            return;
        }
        // COMPLEMENT: each pixel once. A bit per pixel of the part of the
        // clip rectangle the ellipse covers marks the pixels done (points
        // outside the clip rectangle are not drawn anyway).
        let (a, b) = (rx.abs(), ry.abs());
        let (bx0, by0) = (
            cx.saturating_sub(a).max(wr.x0),
            cy.saturating_sub(b).max(wr.y0),
        );
        let (bx1, by1) = (
            cx.saturating_add(a + 1).min(wr.x1),
            cy.saturating_add(b + 1).min(wr.y1),
        );
        if bx0 >= bx1 || by0 >= by1 {
            return;
        }
        let bw = (bx1 - bx0) as usize;
        let mut seen = vec![0u64; (bw * (by1 - by0) as usize).div_ceil(64)];
        ellipse_points(cx, cy, rx, ry, |x, y| {
            if x >= bx0 && y >= by0 && x < bx1 && y < by1 {
                let k = (y - by0) as usize * bw + (x - bx0) as usize;
                if seen[k / 64] & (1 << (k % 64)) == 0 {
                    seen[k / 64] |= 1 << (k % 64);
                    wr.write(c.buf, (y * wr.w + x) as usize, true, ink);
                }
            }
        });
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
        let f = self.span_fill(c);
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
                    self.fill_span(c, &f, y, pair[0].max(cx0), pair[1].min(cx1 - 1), y);
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
        let w = c.w as usize;
        if self.pattern.is_none() {
            // Solid fill: the region becomes `col`; when that is not the
            // seed colour, filled pixels no longer match and need no
            // separate marking (same region as below).
            let col = self.ink & c.mask;
            if col == seed {
                // Every pixel of the region already holds the colour.
                return;
            }
            let mut stack = vec![(x, y)];
            while let Some((sx, sy)) = stack.pop() {
                let b = &mut c.buf[sy as usize * w..][..w];
                if b[sx as usize] != seed {
                    continue;
                }
                let mut l = sx;
                while l > cx0 && b[l as usize - 1] == seed {
                    l -= 1;
                }
                let mut r = sx;
                while r + 1 < cx1 && b[r as usize + 1] == seed {
                    r += 1;
                }
                b[l as usize..=r as usize].fill(col);
                for ny in [sy - 1, sy + 1] {
                    if ny < cy0 || ny >= cy1 {
                        continue;
                    }
                    let b = &c.buf[ny as usize * w..][..w];
                    let mut xx = l;
                    while xx <= r {
                        if b[xx as usize] == seed {
                            stack.push((xx, ny));
                            while xx <= r && b[xx as usize] == seed {
                                xx += 1;
                            }
                        } else {
                            xx += 1;
                        }
                    }
                }
            }
            return;
        }
        let cw = (cx1 - cx0) as usize;
        let chh = (cy1 - cy0) as usize;
        let mut done = vec![false; cw * chh];
        // Scanline fill of the 4-connected region (the set of pixels found
        // does not depend on the order: same region as the reference).
        let mut stack = vec![(x, y)];
        let mut filled: Vec<(i32, i32, i32)> = Vec::new();
        while let Some((sx, sy)) = stack.pop() {
            let d = &mut done[(sy - cy0) as usize * cw..][..cw];
            let b = &c.buf[sy as usize * w..][..w];
            let free = |d: &[bool], x: i32| !d[(x - cx0) as usize] && b[x as usize] == seed;
            if !free(d, sx) {
                continue;
            }
            let mut l = sx;
            while l > cx0 && free(d, l - 1) {
                l -= 1;
            }
            let mut r = sx;
            while r + 1 < cx1 && free(d, r + 1) {
                r += 1;
            }
            d[(l - cx0) as usize..=(r - cx0) as usize].fill(true);
            filled.push((l, r, sy));
            for ny in [sy - 1, sy + 1] {
                if ny < cy0 || ny >= cy1 {
                    continue;
                }
                let d = &done[(ny - cy0) as usize * cw..][..cw];
                let b = &c.buf[ny as usize * w..][..w];
                let free = |x: i32| !d[(x - cx0) as usize] && b[x as usize] == seed;
                let mut xx = l;
                while xx <= r {
                    if free(xx) {
                        stack.push((xx, ny));
                        while xx <= r && free(xx) {
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
        let mask = c.mask;
        for (l, r, y) in filled {
            let py = y - cy0;
            let row = &mut c.buf[y as usize * w + l as usize..=y as usize * w + r as usize];
            match &self.pattern {
                None => row.fill(self.ink & mask),
                Some(p) if p.planes == 1 => {
                    for (k, px) in row.iter_mut().enumerate() {
                        let x = l + k as i32;
                        *px = if p.bit(x, py) { self.ink } else { self.paper } & mask;
                    }
                }
                Some(p) => {
                    for (k, px) in row.iter_mut().enumerate() {
                        *px = p.colour(l + k as i32, py) & mask;
                    }
                }
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
        let wr = self.writer(c);
        let mut cx = x;
        for &ch in s {
            let g = gfont::glyph(ch);
            let cell_w = gfont::WIDTH + bold as i32;
            // Columns used by the cell, with the italic shifts (3 right on
            // the top row down to 1 left on the bottom one): when all are
            // inside the clip rectangle no pixel needs clipping.
            let (sl, sr) = if italic {
                ((base - (gfont::HEIGHT - 1)) >> 1, base >> 1)
            } else {
                (0, 0)
            };
            let inside =
                wr.inside(cx + sl, top) && wr.inside(cx + cell_w - 1 + sr, top + gfont::HEIGHT - 1);
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
                if inside {
                    let i = ((top + row) * wr.w + cx + shift) as usize;
                    for px in 0..cell_w {
                        let on = bits & (0x100 >> px) != 0;
                        wr.write(c.buf, i + px as usize, on ^ inv, self.ink);
                    }
                    continue;
                }
                for px in 0..cell_w {
                    let on = bits & (0x100 >> px) != 0;
                    wr.put(c.buf, cx + px + shift, top + row, on ^ inv, self.ink);
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

#[cfg(test)]
mod reference_tests {
    use super::super::draw_reference as r;
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
        fn range(&mut self, a: i32, b: i32) -> i32 {
            a + (self.next() % (b - a + 1) as u32) as i32
        }
    }

    fn state_text(g: &GrState) -> String {
        format!(
            "{} {} {} {} {:04x} {} {} {} {:?} {:?} {} {}",
            g.ink,
            g.paper,
            g.outline,
            g.writing,
            g.line_pattern,
            g.line_count,
            g.x,
            g.y,
            g.clip,
            g.pattern,
            g.paint_outline,
            g.text_style
        )
    }

    fn ref_state_text(g: &r::GrState) -> String {
        format!(
            "{} {} {} {} {:04x} {} {} {} {:?} {:?} {} {}",
            g.ink,
            g.paper,
            g.outline,
            g.writing,
            g.line_pattern,
            g.line_count,
            g.x,
            g.y,
            g.clip,
            g.pattern.as_ref().map(|p| Pattern {
                height: p.height,
                planes: p.planes,
                rows: p.rows.clone()
            }),
            g.paint_outline,
            g.text_style
        )
    }

    #[test]
    fn ellipse_points_stay_in_the_bounding_box() {
        for rx in -3..120 {
            for ry in -3..120 {
                ellipse_points(0, 0, rx, ry, |x, y| {
                    assert!(
                        x.abs() <= rx.abs() && y.abs() <= ry.abs(),
                        "{rx} {ry}: {x} {y}"
                    );
                });
            }
        }
    }

    /// Every primitive on random states, modes, patterns, clip windows and
    /// shapes gives the same pixels and graphic state as the reference.
    #[test]
    fn primitives_match_the_reference() {
        let mut rng = Rng(42);
        for round in 0..20000 {
            let (w, h) = (rng.range(8, 80), rng.range(8, 70));
            let planes = rng.range(1, 6) as u8;
            let pixels: Vec<u8> = (0..w * h).map(|_| (rng.next() % 64) as u8).collect();
            let pattern = match rng.next() % 4 {
                0 | 1 => None,
                k => {
                    let height = 1usize << (rng.next() % 4);
                    let pl = if k == 2 { 1 } else { rng.range(2, 6) as usize };
                    let rows = (0..height * pl).map(|_| rng.next() as u16).collect();
                    Some(Pattern {
                        height,
                        planes: pl,
                        rows,
                    })
                }
            };
            let clip = if rng.next().is_multiple_of(3) {
                (0, 0, i32::MAX, i32::MAX)
            } else {
                let x0 = rng.range(-10, w);
                let y0 = rng.range(-10, h);
                (x0, y0, rng.range(x0, w + 10), rng.range(y0, h + 10))
            };
            let line_pattern = match rng.next() % 4 {
                0 => 0xFFFF,
                1 => 0,
                _ => rng.next() as u16,
            };
            let mut g = GrState {
                ink: (rng.next() % 64) as u8,
                paper: (rng.next() % 64) as u8,
                outline: (rng.next() % 64) as u8,
                writing: (rng.next() % 8) as u8,
                line_pattern,
                line_count: (rng.next() % 16) as u8,
                x: rng.range(-20, w + 20),
                y: rng.range(-20, h + 20),
                clip,
                pattern_number: 0,
                pattern: pattern.clone(),
                paint_outline: rng.next().is_multiple_of(3),
                text_style: (rng.next() % 8) as u8,
                font: 0,
            };
            let mut rg = r::GrState {
                ink: g.ink,
                paper: g.paper,
                outline: g.outline,
                writing: g.writing,
                line_pattern: g.line_pattern,
                line_count: g.line_count,
                x: g.x,
                y: g.y,
                clip: g.clip,
                pattern_number: 0,
                pattern: pattern.map(|p| r::Pattern {
                    height: p.height,
                    planes: p.planes,
                    rows: p.rows,
                }),
                paint_outline: g.paint_outline,
                text_style: g.text_style,
                font: 0,
            };
            let (mut a, mut b) = (pixels.clone(), pixels);
            let mut ca = Canvas::new(&mut a, w as u32, h as u32, planes);
            let mut cb = r::Canvas::new(&mut b, w as u32, h as u32, planes);
            let p = |rng: &mut Rng| (rng.range(-30, w + 30), rng.range(-30, h + 30));
            let op = round % 9;
            match op {
                0 => {
                    let (x, y) = p(&mut rng);
                    g.plot(&mut ca, x, y);
                    rg.plot(&mut cb, x, y);
                }
                1 => {
                    let (x, y) = p(&mut rng);
                    g.draw_to(&mut ca, x, y);
                    rg.draw_to(&mut cb, x, y);
                }
                2 | 3 => {
                    let (x1, y1) = p(&mut rng);
                    let (x2, y2) = (x1 + rng.range(-5, 60), y1 + rng.range(-5, 50));
                    if op == 2 {
                        g.draw_box(&mut ca, x1, y1, x2, y2);
                        rg.draw_box(&mut cb, x1, y1, x2, y2);
                    } else {
                        g.bar(&mut ca, x1, y1, x2, y2);
                        rg.bar(&mut cb, x1, y1, x2, y2);
                    }
                }
                4 => {
                    let (x, y) = p(&mut rng);
                    let (rx, ry) = (rng.range(-3, 45), rng.range(-3, 45));
                    g.ellipse(&mut ca, x, y, rx, ry);
                    rg.ellipse(&mut cb, x, y, rx, ry);
                }
                5 => {
                    let n = rng.range(1, 7);
                    let pts: Vec<(i32, i32)> = (0..n).map(|_| p(&mut rng)).collect();
                    g.polygon(&mut ca, &pts);
                    rg.polygon(&mut cb, &pts);
                }
                6 => {
                    let (x, y) = p(&mut rng);
                    let seed = if rng.next().is_multiple_of(4) {
                        (rng.next() % 64) as u8
                    } else {
                        ca.get(x, y).unwrap_or(0)
                    };
                    g.paint(&mut ca, x, y, seed);
                    rg.paint(&mut cb, x, y, seed);
                }
                7 => {
                    let (x, y) = p(&mut rng);
                    let s: Vec<u8> = (0..rng.range(0, 6))
                        .map(|_| (rng.next() % 256) as u8)
                        .collect();
                    let r1 = g.text(&mut ca, x, y, &s);
                    let r2 = rg.text(&mut cb, x, y, &s);
                    assert_eq!(r1, r2);
                }
                _ => {
                    // A run of lines (the line pattern continues).
                    for _ in 0..rng.range(1, 5) {
                        let (x, y) = p(&mut rng);
                        g.draw_to(&mut ca, x, y);
                        rg.draw_to(&mut cb, x, y);
                    }
                }
            }
            assert!(a == b, "round {round} op {op}: pixels differ");
            assert_eq!(state_text(&g), ref_state_text(&rg), "round {round} op {op}");
        }
    }
}
