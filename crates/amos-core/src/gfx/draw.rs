//! Drawing primitives (Plot, Draw, Box, Bar, Circle, Paint...).

/// Graphic state of a screen (`EcInkA`, `EcMode`, `EcLine`, clip...).
#[derive(Clone, Debug)]
pub struct GrState {
    /// Ink (APen), paper (BPen) and outline pen.
    pub ink: u8,
    pub paper: u8,
    pub outline: u8,
    /// Gr Writing mode: JAM1=0, JAM2=1, COMPLEMENT=2, INVERSVID=4.
    pub writing: u8,
    /// Set Line pattern.
    pub line_pattern: u16,
    /// Graphic cursor.
    pub x: i32,
    pub y: i32,
    /// Clip rectangle (x1/y1 exclusive).
    pub clip: (i32, i32, i32, i32),
}

impl Default for GrState {
    fn default() -> Self {
        GrState { ink: 2, paper: 1, outline: 2, writing: 1, line_pattern: 0xFFFF, x: 0, y: 0, clip: (0, 0, i32::MAX, i32::MAX) }
    }
}
