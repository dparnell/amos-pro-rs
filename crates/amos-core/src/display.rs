//! Description of what is on the (virtual) Amiga display for one frame.
//!
//! The core never talks to the GPU. Every video frame it produces a list of
//! [`Layer`]s that the platform renderer composites back to front.
//!
//! Coordinates are *display units*: one unit is a hires pixel horizontally
//! and an interlaced line vertically, so a lowres non-interlaced pixel
//! covers 2x2 units. The display shows the PAL overscan area starting at
//! hardware position ([`HW_X0`], [`HW_Y0`]) (lowres colour clocks / raster
//! lines as used by AMOS `Screen Display`, `X Hard`...).

/// Width of the visible display in display units.
pub const DISPLAY_WIDTH: u32 = 736;
/// Height of the visible display in display units.
pub const DISPLAY_HEIGHT: u32 = 576;
/// Hardware X (lowres pixels) of the left edge of the display.
pub const HW_X0: i32 = 96;
/// Hardware raster line of the top edge of the display.
pub const HW_Y0: i32 = 26;

/// Converts a hardware X coordinate (lowres pixels) to display units.
pub fn hw_x_to_display(x: i32) -> i32 {
    (x - HW_X0) * 2
}

/// Converts a hardware raster line to display units.
pub fn hw_y_to_display(y: i32) -> i32 {
    (y - HW_Y0) * 2
}

/// How the pixels of a layer are to be interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerFormat {
    /// One byte per pixel, index into the layer palette.
    Indexed,
    /// Four bytes per pixel, already converted RGBA (HAM screens, sprites).
    /// Pixels with alpha 0 are transparent.
    Rgba,
}

/// Rectangle in display units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// One AMOS screen (or sprite) on the display.
///
/// On the Amiga a screen owns whole scanlines: from its first line, the
/// whole width of the display shows the screen's colour 0 except where its
/// bitmap is shown. `band` is that area (normally the full display width)
/// and `window` the part showing the bitmap.
pub struct Layer<'a> {
    /// Stable identifier, lets the renderer cache GPU textures.
    pub id: u32,
    /// Changes whenever `pixels` changed since the previous frame.
    pub pixels_version: u64,
    pub format: LayerFormat,
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],
    /// Palette: `palette_rows` rows of 256 RGBA colours. With several rows,
    /// row `n` applies to the hardware line `n` of the band (rainbows and
    /// other per line colour changes).
    pub palette: &'a [[u8; 4]],
    pub palette_rows: u32,
    /// Area covered by the layer (colour 0 outside `window`).
    pub band: Rect,
    /// Area showing the bitmap.
    pub window: Rect,
    /// First bitmap pixel shown at the window's top left (Screen Offset).
    pub src_x: i32,
    pub src_y: i32,
    /// Display units per bitmap pixel (2 for lowres / non interlaced, else 1).
    pub scale_x: u32,
    pub scale_y: u32,
    /// Colour index treated as transparent (dual playfield front screen,
    /// sprites), if any. Transparent pixels and the band are not drawn.
    pub transparent: Option<u8>,
}

/// Everything the renderer needs for one video frame.
pub struct Frame<'a> {
    /// Colour shown where no screen is displayed (`Colour Back`).
    pub border: [u8; 4],
    /// Layers in back-to-front order.
    pub layers: Vec<Layer<'a>>,
}

/// Converts an Amiga $0RGB colour to RGBA.
pub fn rgb12_to_rgba(c: u16) -> [u8; 4] {
    let r = ((c >> 8) & 15) as u8;
    let g = ((c >> 4) & 15) as u8;
    let b = (c & 15) as u8;
    [r * 17, g * 17, b * 17, 255]
}

/// Software reference compositor: renders a frame to RGBA (used for tests
/// and screenshots; the GPU renderer must produce the same picture).
pub fn render_rgba(frame: &Frame) -> Vec<u8> {
    let (w, h) = (DISPLAY_WIDTH as usize, DISPLAY_HEIGHT as usize);
    let mut out = vec![0u8; w * h * 4];
    for px in out.chunks_exact_mut(4) {
        px.copy_from_slice(&frame.border);
    }
    for layer in &frame.layers {
        let b = layer.band;
        for y in b.y.max(0)..(b.y + b.h as i32).min(h as i32) {
            let row = ((y - b.y) / 2).max(0) as u32;
            let prow = if layer.palette_rows > 1 { row.min(layer.palette_rows - 1) } else { 0 } as usize;
            for x in b.x.max(0)..(b.x + b.w as i32).min(w as i32) {
                let win = layer.window;
                let inside = x >= win.x && x < win.x + win.w as i32 && y >= win.y && y < win.y + win.h as i32;
                let colour = if inside {
                    let sx = (x - win.x) / layer.scale_x.max(1) as i32 + layer.src_x;
                    let sy = (y - win.y) / layer.scale_y.max(1) as i32 + layer.src_y;
                    if sx < 0 || sy < 0 || sx >= layer.width as i32 || sy >= layer.height as i32 {
                        None
                    } else {
                        let i = sy as usize * layer.width as usize + sx as usize;
                        match layer.format {
                            LayerFormat::Indexed => {
                                let idx = layer.pixels[i];
                                if Some(idx) == layer.transparent {
                                    None
                                } else {
                                    layer.palette.get(prow * 256 + idx as usize).copied()
                                }
                            }
                            LayerFormat::Rgba => {
                                let c = [
                                    layer.pixels[i * 4],
                                    layer.pixels[i * 4 + 1],
                                    layer.pixels[i * 4 + 2],
                                    layer.pixels[i * 4 + 3],
                                ];
                                if c[3] == 0 { None } else { Some(c) }
                            }
                        }
                    }
                } else if layer.transparent.is_some() {
                    None
                } else {
                    layer.palette.get(prow * 256).copied()
                };
                if let Some(c) = colour {
                    let o = (y as usize * w + x as usize) * 4;
                    out[o..o + 4].copy_from_slice(&c);
                }
            }
        }
    }
    out
}
