//! Hardware sprites, bobs and the mouse pointer.
//!
//! State of the sprite / bob / AMAL subsystem. The instructions and the
//! per frame work are in `machine/inst_sprites.rs`; bobs are in
//! [`super::bobs`], AMAL in [`super::amal`], image helpers in
//! [`super::images`].

use super::amal::AmalState;
use super::bobs::Bobs;
use super::images::MaskState;
use crate::banks::Image;

/// Number of sprites: 0-7 direct, 8-63 computed (`HsNb`).
pub const SPRITE_MAX: usize = 64;

/// Default height of the sprite buffers (`Set Sprite Buffer`, config).
pub const DEFAULT_SPRITE_LINES: u16 = 128;

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

/// Requested state of a sprite (`T_HsTAct`, also the AMAL act block).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HsAct {
    /// Bit 3 (or AMAL bits 0-2): changed; 0x80: to remove.
    pub flag: u8,
    pub x: i16,
    pub y: i16,
    pub image: i16,
}

/// Displayed state of a sprite (`T_HsTable`), updated by `ActHs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HsShown {
    pub active: bool,
    pub x: i16,
    pub y: i16,
    /// Image number in the sprite bank (1-based, no flags).
    pub image: u16,
}

/// State of hardware sprites, bobs and AMAL channels.
#[derive(Debug)]
pub struct SpriteState {
    /// Sprites to display this frame, front first, refreshed by
    /// `Hardware::sprites_prepare_frame`.
    pub display: Vec<DisplaySprite>,
    pub act: [HsAct; SPRITE_MAX],
    pub shown: [HsShown; SPRITE_MAX],
    /// Lines per sprite column (`T_HsPMax`, `Set Sprite Buffer`).
    pub buffer_lines: u16,
    /// `Sprite Priority` of each screen (PF1P of `EcCon2`): sprite pairs
    /// below this number are in front of the screen.
    pub priority: [u8; 12],
    /// Mouse: shown when the counter is >= 0 (`T_MouShow`), pointer
    /// number (`T_MouSpr`, 0-based) and image.
    pub mouse_show: i16,
    pub mouse_spr: u16,
    pub mouse_image: Image,
    pub bobs: Bobs,
    /// Mask state of the images of bank 1 (index 0) and bank 2 (index 1).
    pub masks: [Vec<MaskState>; 2],
    /// Collision table (`T_TColl`, 256 bits).
    pub col: [u8; 32],
    /// `Set Hardcol` value (CLXCON).
    pub hardcol: u16,
    /// `Bob Update On/Off`, `Sprite Update On/Off` (`ActuMask` bits).
    pub update_bobs: bool,
    pub update_sprites: bool,
    /// `Update Every n` (`VBLDelai`) and VBL of the last update.
    pub update_every: u16,
    pub last_update: u64,
    /// Changes waiting for the next update (`T_Actualise` bits).
    pub dirty_bobs: bool,
    pub dirty_sprites: bool,
    pub amal: AmalState,
    /// Act blocks of the 4 rainbows driven by AMAL (X = base colour,
    /// Y = first line, A = height) and their change flags.
    pub rainbow_act: [[i16; 3]; 4],
    pub rainbow_changed: [bool; 4],
    /// Incremented when sprite pixels may have changed (bank edits).
    pub generation: u64,
}

impl Default for SpriteState {
    fn default() -> Self {
        SpriteState {
            display: Vec::new(),
            act: [HsAct::default(); SPRITE_MAX],
            shown: [HsShown::default(); SPRITE_MAX],
            buffer_lines: DEFAULT_SPRITE_LINES,
            priority: [0; 12],
            mouse_show: 0,
            mouse_spr: 0,
            mouse_image: super::images::mouse_bank()
                .0
                .first()
                .cloned()
                .unwrap_or_default(),
            bobs: Bobs::default(),
            masks: [Vec::new(), Vec::new()],
            col: [0; 32],
            hardcol: 0,
            update_bobs: true,
            update_sprites: true,
            update_every: 0,
            last_update: 0,
            dirty_bobs: false,
            dirty_sprites: false,
            amal: AmalState::default(),
            rainbow_act: [[0; 3]; 4],
            rainbow_changed: [false; 4],
            generation: 0,
        }
    }
}

impl SpriteState {
    /// Mask state of image `n` (1-based) of bank 1 (`icons` false) or 2.
    pub fn mask(&self, icons: bool, n: u16) -> MaskState {
        self.masks[icons as usize]
            .get(n as usize)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_mask(&mut self, icons: bool, n: u16, m: MaskState) {
        let v = &mut self.masks[icons as usize];
        if v.len() <= n as usize {
            v.resize(n as usize + 1, MaskState::Lazy);
        }
        v[n as usize] = m;
    }

    /// Image `n` (1-based) inserted / deleted: shifts the mask states.
    pub fn masks_insert(&mut self, icons: bool, n: u16) {
        let v = &mut self.masks[icons as usize];
        if (n as usize) < v.len() {
            v.insert(n as usize, MaskState::Lazy);
        }
    }

    pub fn masks_delete(&mut self, icons: bool, first: u16, last: u16) {
        let v = &mut self.masks[icons as usize];
        let a = (first as usize).min(v.len());
        let b = (last as usize + 1).min(v.len());
        v.drain(a..b);
    }

    pub fn clear_col(&mut self) {
        self.col = [0; 32];
    }

    pub fn set_col(&mut self, n: u16) {
        let n = n & 0xFF;
        self.col[(n >> 3) as usize] |= 1 << (n & 7);
    }

    /// `GetCol` (`=Col(n)`): -1 if bit n is set; for n < 0, the first set
    /// bit from -n (0 if none).
    pub fn get_col(&self, n: i32) -> i32 {
        if n >= 0 {
            let n = n & 0xFF;
            return if self.col[(n >> 3) as usize] & (1 << (n & 7)) != 0 {
                -1
            } else {
                0
            };
        }
        let start = n.unsigned_abs();
        if start >= 255 {
            return 0;
        }
        (start..256)
            .find(|&i| self.col[(i >> 3) as usize] & (1 << (i & 7)) != 0)
            .map_or(0, |i| i as i32)
    }
}

/// Converts the rows of a sprite image (chunky colour indexes 0-31,
/// 0 = transparent) to RGBA with one palette per row.
pub fn to_rgba(
    pixels: &[u8],
    w: u32,
    h: u32,
    palette_of_row: impl Fn(u32) -> [u16; 32],
) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let pal = palette_of_row(y);
        for x in 0..w {
            let i = (y * w + x) as usize;
            let c = pixels[i];
            if c != 0 {
                let rgba = crate::display::rgb12_to_rgba(pal[c as usize & 31]);
                out[i * 4..i * 4 + 4].copy_from_slice(&rgba);
            }
        }
    }
    out
}

/// Cheap content hash used as the texture version of a sprite.
pub fn hash_bytes(data: &[u8], seed: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_table() {
        let mut s = SpriteState::default();
        s.set_col(5);
        s.set_col(40);
        assert_eq!(s.get_col(5), -1);
        assert_eq!(s.get_col(6), 0);
        assert_eq!(s.get_col(-6), 40);
        assert_eq!(s.get_col(-41), 0);
    }
}
