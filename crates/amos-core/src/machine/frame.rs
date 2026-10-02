//! Builds the display [`Frame`] from the screens and sprites.

use super::Hardware;
use crate::display::Frame;

/// Buffers kept between frames (converted palettes, HAM pixels...).
#[derive(Default)]
pub struct FrameCache {}

impl Hardware {
    pub fn build_frame(&mut self) -> Frame<'_> {
        Frame { border: [0, 0, 0, 255], layers: Vec::new() }
    }
}
