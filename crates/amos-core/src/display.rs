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
    out.as_chunks_mut::<4>().0.fill(frame.border);
    for layer in &frame.layers {
        render_layer(layer, &mut out, w, h);
    }
    out
}

/// Draws one layer: per pixel, the band outside the window shows colour 0
/// of the line's palette row (unless the layer is transparent), the window
/// the bitmap pixel (transparent index / alpha 0 / outside the bitmap: not
/// drawn), as [`render_rgba_reference`] does, a row at a time.
fn render_layer(layer: &Layer, out: &mut [u8], w: usize, h: usize) {
    let b = layer.band;
    let win = layer.window;
    let (scale_x, scale_y) = (layer.scale_x.max(1) as i32, layer.scale_y.max(1) as i32);
    let x0 = b.x.max(0);
    let x1 = (b.x + b.w as i32).min(w as i32);
    if x0 >= x1 {
        return;
    }
    // Window columns within the band's visible columns.
    let (wx0, wx1) = (win.x.max(x0), (win.x + win.w as i32).min(x1));
    for y in b.y.max(0)..(b.y + b.h as i32).min(h as i32) {
        let row = ((y - b.y) / 2).max(0) as u32;
        let prow = if layer.palette_rows > 1 { row.min(layer.palette_rows - 1) } else { 0 } as usize;
        let line = &mut out[y as usize * w * 4..(y as usize + 1) * w * 4];
        // Band outside the window.
        if layer.transparent.is_none()
            && let Some(&c) = layer.palette.get(prow * 256)
        {
            let in_win_rows = y >= win.y && y < win.y + win.h as i32;
            let (a, z) = if in_win_rows && wx0 < wx1 { (wx0, wx1) } else { (x1, x1) };
            for x in (x0..a).chain(z.max(x0)..x1) {
                line[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&c);
            }
        }
        if !(y >= win.y && y < win.y + win.h as i32) || wx0 >= wx1 {
            continue;
        }
        let sy = (y - win.y) / scale_y + layer.src_y;
        if sy < 0 || sy >= layer.height as i32 {
            continue;
        }
        let src_row = sy as usize * layer.width as usize;
        let pal = &layer.palette[(prow * 256).min(layer.palette.len())..];
        for x in wx0..wx1 {
            let sx = (x - win.x) / scale_x + layer.src_x;
            if sx < 0 || sx >= layer.width as i32 {
                continue;
            }
            let i = src_row + sx as usize;
            let colour = match layer.format {
                LayerFormat::Indexed => {
                    let idx = layer.pixels[i];
                    if Some(idx) == layer.transparent {
                        continue;
                    }
                    match pal.get(idx as usize) {
                        Some(&c) => c,
                        None => continue,
                    }
                }
                LayerFormat::Rgba => {
                    let c: [u8; 4] = layer.pixels[i * 4..i * 4 + 4].try_into().unwrap();
                    if c[3] == 0 {
                        continue;
                    }
                    c
                }
            };
            line[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&colour);
        }
    }
}

/// The straightforward per pixel compositor (reference for the tests of
/// [`render_rgba`]).
#[cfg(test)]
pub fn render_rgba_reference(frame: &Frame) -> Vec<u8> {
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

#[cfg(test)]
mod render_tests {
    use super::*;

    #[test]
    fn render_rgba_matches_the_reference() {
        let mut seed = 7u32;
        let mut rnd = |n: u32| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 8) % n.max(1)
        };
        for _ in 0..300 {
            let n_layers = 1 + rnd(4) as usize;
            let mut pixels = Vec::new();
            let mut palettes = Vec::new();
            let mut specs = Vec::new();
            for _ in 0..n_layers {
                let format = if rnd(3) == 0 { LayerFormat::Rgba } else { LayerFormat::Indexed };
                let (width, height) = (1 + rnd(400), 1 + rnd(300));
                let bpp = if format == LayerFormat::Rgba { 4 } else { 1 };
                let px: Vec<u8> = (0..width * height * bpp).map(|_| rnd(256) as u8).collect();
                let palette_rows = 1 + rnd(3) * rnd(200);
                // Sometimes a short palette (missing colours are not drawn).
                let len = if rnd(5) == 0 { rnd(palette_rows * 256) } else { palette_rows * 256 };
                let pal: Vec<[u8; 4]> = (0..len).map(|i| [i as u8, (i >> 8) as u8, rnd(256) as u8, 255]).collect();
                let rect = |r: &mut dyn FnMut(u32) -> u32| Rect {
                    x: r(900) as i32 - 100,
                    y: r(700) as i32 - 60,
                    w: r(900),
                    h: r(700),
                };
                let band = rect(&mut rnd);
                let window = rect(&mut rnd);
                specs.push((format, width, height, palette_rows, band, window, rnd(40) as i32 - 10, rnd(40) as i32 - 10, 1 + rnd(2), 1 + rnd(2), if rnd(2) == 0 { None } else { Some(rnd(4) as u8) }));
                pixels.push(px);
                palettes.push(pal);
            }
            let layers = specs
                .iter()
                .enumerate()
                .map(|(k, s)| Layer {
                    id: k as u32,
                    pixels_version: 0,
                    format: s.0,
                    width: s.1,
                    height: s.2,
                    pixels: &pixels[k],
                    palette: &palettes[k],
                    palette_rows: s.3,
                    band: s.4,
                    window: s.5,
                    src_x: s.6,
                    src_y: s.7,
                    scale_x: s.8,
                    scale_y: s.9,
                    transparent: s.10,
                })
                .collect();
            let frame = Frame { border: [1, 2, 3, 255], layers };
            assert!(render_rgba(&frame) == render_rgba_reference(&frame));
        }
    }
}
