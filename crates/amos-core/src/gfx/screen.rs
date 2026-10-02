//! AMOS screens (`EcCree`... in `+W.s`).
//!
//! Bitmaps are stored "chunky": one byte per pixel holding the colour index
//! (0..63). Bit `p` of a pixel is the value of bitplane `p` on the Amiga, so
//! planar operations (writing modes, plane masks) are done on bits.

use super::draw::GrState;
use super::window::TextState;

/// Number of screens: 0-7 user screens, 8-11 system screens.
pub const MAX_SCREENS: usize = 12;

/// Default AMOS palette (`+Interpreter_Config.s`); colours 16-31 come from
/// the mouse sprite bank at start-up.
pub const DEFAULT_PALETTE: [u16; 32] = [
    0x000, 0xA40, 0xFFF, 0x000, 0xF00, 0x0F0, 0x00F, 0x666, 0x555, 0x333, 0x733, 0x373, 0x773, 0x337, 0x737, 0x377,
    0x000, 0xD86, 0xA40, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000,
];

/// Screen Open mode bits.
pub const MODE_HIRES: u32 = 0x8000;
pub const MODE_LACED: u32 = 0x0004;

#[derive(Clone, Debug)]
pub struct Screen {
    pub number: usize,
    /// Bitmap size in pixels (`EcTx`, `EcTy`).
    pub width: u32,
    pub height: u32,
    pub planes: u8,
    /// Number of colours requested in Screen Open (4096 = HAM).
    pub colours: u32,
    pub hires: bool,
    pub lace: bool,
    pub ham: bool,
    /// One bitmap, or two when double buffered.
    pub bitmaps: Vec<Vec<u8>>,
    /// Index of the bitmap drawn into (logic) and displayed (physic).
    pub logic: usize,
    pub physic: usize,
    /// Incremented whenever bitmap pixels change (renderer cache key).
    pub version: u64,
    /// Hardware palette, $0RGB.
    pub palette: [u16; 32],
    /// Applied display position: hardware X (lowres, multiple of 16) and
    /// raster line of the first bitmap line (`EcWX`, `EcWY - EcYBase`).
    pub display_x: i32,
    pub display_y: i32,
    /// Applied display size: width in lowres pixels, height in raster lines.
    pub display_w: u32,
    pub display_h: u32,
    /// Pending `Screen Display` values (applied at the next update).
    pub pending_display: [Option<i32>; 4],
    /// Screen Offset (scroll position of the bitmap).
    pub offset_x: i32,
    pub offset_y: i32,
    pub pending_offset: [Option<i32>; 2],
    pub hidden: bool,
    /// Screen Clone: shares the bitmap of another screen.
    pub clone_of: Option<usize>,
    /// Dual playfield partner: `Some(n)` on the master screen.
    pub dual_with: Option<usize>,
    pub dual_slave: bool,
    pub dual_priority: bool,
    /// Autoback mode 0/1/2.
    pub autoback: u8,
    /// Graphic state (ink, writing mode, clip, patterns, graphic cursor).
    pub gr: GrState,
    /// Text windows.
    pub text: TextState,
}

impl Screen {
    /// Creates a screen; `mode` has the Screen Open bits ($8000 hires,
    /// $4 laced). Validation is done by the caller.
    pub fn new(number: usize, width: u32, height: u32, colours: u32, mode: u32) -> Screen {
        let ham = colours == 4096;
        let planes = match colours {
            0..=2 => 1,
            3..=4 => 2,
            5..=8 => 3,
            9..=16 => 4,
            17..=32 => 5,
            _ => 6,
        };
        let width = width & !15;
        Screen {
            number,
            width,
            height,
            planes,
            colours,
            hires: mode & MODE_HIRES != 0,
            lace: mode & MODE_LACED != 0,
            ham,
            bitmaps: vec![vec![0; (width * height) as usize]],
            logic: 0,
            physic: 0,
            version: 1,
            palette: DEFAULT_PALETTE,
            display_x: 128,
            display_y: 50,
            display_w: width,
            display_h: height,
            pending_display: [None; 4],
            offset_x: 0,
            offset_y: 0,
            pending_offset: [None; 2],
            hidden: false,
            clone_of: None,
            dual_with: None,
            dual_slave: false,
            dual_priority: false,
            autoback: 0,
            gr: GrState::default(),
            text: TextState::default(),
        }
    }

    /// Number of colours available (palette entries usable by pens).
    pub fn num_colours(&self) -> u32 {
        if self.ham { 16 } else { 1 << self.planes }
    }

    /// Mask of valid colour index bits.
    pub fn colour_mask(&self) -> u8 {
        ((1u32 << self.planes) - 1) as u8
    }

    pub fn is_double_buffered(&self) -> bool {
        self.bitmaps.len() > 1
    }

    /// Bitmap drawn into.
    pub fn logic_mut(&mut self) -> &mut Vec<u8> {
        self.version += 1;
        let i = self.logic;
        &mut self.bitmaps[i]
    }

    pub fn logic_ref(&self) -> &[u8] {
        &self.bitmaps[self.logic]
    }

    pub fn physic_ref(&self) -> &[u8] {
        &self.bitmaps[self.physic]
    }

    /// Colour index of a pixel of the logic bitmap (`Point`), or None if
    /// outside the bitmap.
    pub fn pixel(&self, x: i32, y: i32) -> Option<u8> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        Some(self.bitmaps[self.logic][(y as u32 * self.width + x as u32) as usize])
    }

    /// Sets a pixel of the logic bitmap (no clipping beyond the bitmap).
    pub fn set_pixel(&mut self, x: i32, y: i32, c: u8) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let w = self.width;
        let mask = self.colour_mask();
        self.logic_mut()[(y as u32 * w + x as u32) as usize] = c & mask;
    }
}

/// All screens plus the display ordering.
#[derive(Debug, Default)]
pub struct Screens {
    pub screens: Vec<Option<Box<Screen>>>,
    /// Screen numbers, front first (`T_EcPri`).
    pub priority: Vec<usize>,
    /// Current screen (target of drawing and text instructions).
    pub current: Option<usize>,
    /// `Colour Back`: colour of lines without a screen.
    pub colour_back: u16,
}

impl Screens {
    pub fn new() -> Self {
        Screens { screens: (0..MAX_SCREENS).map(|_| None).collect(), ..Default::default() }
    }

    pub fn get(&self, n: usize) -> Option<&Screen> {
        self.screens.get(n)?.as_deref()
    }

    pub fn get_mut(&mut self, n: usize) -> Option<&mut Screen> {
        self.screens.get_mut(n)?.as_deref_mut()
    }

    pub fn current(&self) -> Option<&Screen> {
        self.get(self.current?)
    }

    pub fn current_mut(&mut self) -> Option<&mut Screen> {
        let c = self.current?;
        self.get_mut(c)
    }

    /// Installs a screen (replacing any screen with the same number), puts
    /// it in front and makes it current.
    pub fn insert(&mut self, screen: Screen) {
        let n = screen.number;
        self.screens[n] = Some(Box::new(screen));
        self.priority.retain(|&s| s != n);
        self.priority.insert(0, n);
        self.current = Some(n);
    }

    pub fn remove(&mut self, n: usize) {
        if n < self.screens.len() {
            self.screens[n] = None;
        }
        self.priority.retain(|&s| s != n);
        if self.current == Some(n) {
            // AMOS makes the front screen current.
            self.current = self.priority.first().copied();
        }
    }
}
