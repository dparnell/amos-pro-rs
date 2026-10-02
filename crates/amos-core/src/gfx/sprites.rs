//! Hardware sprites, bobs and the mouse pointer.

/// A hardware sprite (or the mouse pointer) ready to be displayed, as RGBA
/// pixels (alpha 0 = transparent).
#[derive(Debug, Clone, Default)]
pub struct DisplaySprite {
    /// Stable id (for the renderer texture cache) and pixel version.
    pub id: u32,
    pub version: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Top left position in display units (see `crate::display`).
    pub x: i32,
    pub y: i32,
    /// Display units per sprite pixel (2 = lowres pixel).
    pub scale_x: u32,
    pub scale_y: u32,
}

/// State of hardware sprites, bobs and AMAL channels.
#[derive(Debug, Default)]
pub struct SpriteState {
    /// Sprites to display this frame, front first, refreshed by
    /// `Hardware::sprites_prepare_frame`.
    pub display: Vec<DisplaySprite>,
}
