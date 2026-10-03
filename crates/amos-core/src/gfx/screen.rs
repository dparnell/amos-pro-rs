//! AMOS screens (`EcCree`... in `+W.s`).
//!
//! Bitmaps are stored "chunky": one byte per pixel holding the colour index
//! (0..63). Bit `p` of a pixel is the value of bitplane `p` on the Amiga, so
//! planar operations (writing modes, plane masks) are done on bits.
//!
//! Error values returned by the methods of this file are the library error
//! codes of `+W.s` (`EcWiErr` adds 44 to make the AMOS error number, see
//! [`lib_error`]).

use super::draw::GrState;
use super::effects::Effects;
use super::window::TextState;

/// Number of screens: 0-7 user screens, 8-11 system screens.
pub const MAX_SCREENS: usize = 12;

/// Default AMOS palette (`+Interpreter_Config.s`); colours 16-31 come from
/// the mouse sprite bank at start-up.
pub const DEFAULT_PALETTE: [u16; 32] = [
    0x000, 0xA40, 0xFFF, 0x000, 0xF00, 0x0F0, 0x00F, 0x666, 0x555, 0x333, 0x733, 0x373, 0x773,
    0x337, 0x737, 0x377, 0x000, 0xD86, 0xA40, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000, 0x000,
    0x000, 0x000, 0x000, 0x000, 0x000, 0x000,
];

/// Screen Open mode bits.
pub const MODE_HIRES: u32 = 0x8000;
pub const MODE_LACED: u32 = 0x0004;

/// Default display position (`T_DefWX`=129, `T_DefWY`=50, +W.s:9389).
pub const DEF_WX: i32 = 129;
pub const DEF_WX2: i32 = 129 - 16;
pub const DEF_WY: i32 = 50;
pub const DEF_WY2: i32 = 50 - 8;

/// First and last+1 raster lines where a screen can be shown (PAL):
/// `EcYStrt-1` and `EcYMax-1` (+W.s:5826, 5944).
pub const FIRST_LINE: i32 = 25;
pub const END_LINE: i32 = 310;

// Library error codes (+W.s, `EcWiErr` adds `EcEBase-1` = 44).
pub const E_OUT_OF_MEMORY: u16 = 1;
pub const E_NOT_OPENED: u16 = 3;
pub const E_ILLEGAL_PARAMETER: u16 = 4;
pub const E_ILLEGAL_COLOURS: u16 = 5;
pub const E_SCREEN_NUMBER: u16 = 6;
pub const E_DOUBLE_BUFFER: u16 = 25;
pub const E_CANT_DUAL: u16 = 26;
pub const E_NOT_DUAL: u16 = 27;

/// Converts a library error code to the AMOS error number (`EcWiErr`,
/// +Lib.s:12917): 1 is "out of memory", others are offset by 44.
pub fn lib_error(code: u16) -> u16 {
    if code == 1 { 24 } else { code + 44 }
}

#[derive(Clone, Debug)]
pub struct Screen {
    pub number: usize,
    /// Bitmap size in pixels (`EcTx`, `EcTy`).
    pub width: u32,
    pub height: u32,
    pub planes: u8,
    /// Number of colours requested in Screen Open (`EcNbCol`; 64 for HAM).
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
    /// Applied display size: width in lowres pixels, height in raster lines
    /// (`EcWTx`, `EcWTy`: half the bitmap lines for interlaced screens).
    pub display_w: u32,
    pub display_h: u32,
    /// Pending `Screen Display` values x, y, w, h (applied at the next
    /// update, `EcAWX`...).
    pub pending_display: [Option<i32>; 4],
    /// Screen Offset (scroll position of the bitmap, `EcVX`, `EcVY`).
    pub offset_x: i32,
    pub offset_y: i32,
    pub pending_offset: [Option<i32>; 2],
    pub hidden: bool,
    /// Screen Clone: shows the bitmap of another screen.
    pub clone_of: Option<usize>,
    /// Dual playfield partner: on the master, the second screen.
    pub dual_with: Option<usize>,
    /// True on the second screen of a dual playfield.
    pub dual_slave: bool,
    /// Dual Priority: playfield 2 (the slave) in front (BPLCON2 PF2PRI).
    pub dual_priority: bool,
    /// Autoback mode 0/1/2.
    pub autoback: u8,
    /// Graphic state (ink, writing mode, clip, patterns, graphic cursor).
    pub gr: GrState,
    /// Text windows.
    pub text: TextState,
    /// Zones (`Reserve Zone`): x1, y1, x2, y2 inclusive; all 0 = free.
    pub zones: Vec<[u16; 4]>,
}

impl Screen {
    /// Creates a screen; `mode` has the Screen Open bits ($8000 hires,
    /// $4 laced). Validation is done by the caller. No text window is
    /// opened (see [`Screens::open`]).
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
        let hires = mode & MODE_HIRES != 0;
        let lace = mode & MODE_LACED != 0;
        let mut s = Screen {
            number,
            width,
            height,
            planes,
            colours: if ham { 64 } else { colours },
            hires,
            lace,
            ham,
            bitmaps: vec![vec![0; (width * height) as usize]],
            logic: 0,
            physic: 0,
            version: 1,
            palette: DEFAULT_PALETTE,
            display_x: 0,
            display_y: 0,
            display_w: 0,
            display_h: 0,
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
            zones: Vec::new(),
        };
        // Default display (EcCree, +W.s:3030-3052).
        let wtx = if hires { width / 2 } else { width } as i32;
        let x = if wtx >= 320 + 16 { DEF_WX2 } else { DEF_WX };
        let y = if height >= 256 && (!lace || height >= 512) {
            DEF_WY2
        } else {
            DEF_WY
        };
        s.pending_display = [Some(x), Some(y), Some(wtx), Some(height as i32)];
        s.apply_pending();
        s.gr.clip = (0, 0, width as i32, height as i32);
        s
    }

    /// `EcNbCol`: limit for pen and paper values.
    pub fn num_colours(&self) -> u32 {
        self.colours
    }

    /// Number of colours as returned by `=Screen Colour` (4096 for HAM).
    pub fn screen_colour(&self) -> u32 {
        if self.ham { 4096 } else { self.colours }
    }

    /// Mask of valid colour index bits.
    pub fn colour_mask(&self) -> u8 {
        ((1u32 << self.planes) - 1) as u8
    }

    /// Extra half brite: 6 planes without HAM.
    pub fn is_ehb(&self) -> bool {
        self.planes == 6 && !self.ham
    }

    pub fn is_double_buffered(&self) -> bool {
        self.bitmaps.len() > 1
    }

    /// `Screen Mode`: the hires and lace bits.
    pub fn mode(&self) -> u32 {
        (if self.hires { MODE_HIRES } else { 0 }) | (if self.lace { MODE_LACED } else { 0 })
    }

    /// Bitmap drawn into.
    pub fn logic_mut(&mut self) -> &mut Vec<u8> {
        self.version += 1;
        let i = self.logic;
        &mut self.bitmaps[i]
    }

    /// Any bitmap (0/1), for autoback drawing into both buffers.
    pub fn bitmap_mut(&mut self, index: usize) -> &mut Vec<u8> {
        self.version += 1;
        let i = index.min(self.bitmaps.len() - 1);
        &mut self.bitmaps[i]
    }

    pub fn logic_ref(&self) -> &[u8] {
        &self.bitmaps[self.logic]
    }

    pub fn physic_ref(&self) -> &[u8] {
        &self.bitmaps[self.physic]
    }

    /// Bitmaps to draw into: the logic one, plus the physic one when
    /// autoback is on for a double buffered screen.
    pub fn autoback_targets(&self) -> Vec<usize> {
        if self.autoback != 0 && self.is_double_buffered() {
            vec![self.logic, self.physic]
        } else {
            vec![self.logic]
        }
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

    /// `Cls c,x1,y1 To x2,y2` (`EcCls`, +W.s:3636): coordinates clamped to
    /// the screen, x2/y2 exclusive, into `bitmap`.
    pub fn cls_rect(&mut self, bitmap: usize, colour: u8, x1: i32, y1: i32, x2: i32, y2: i32) {
        let (tx, ty) = (self.width as i32, self.height as i32);
        let x1 = x1.clamp(0, tx);
        let y1 = y1.clamp(0, ty);
        let x2 = x2.clamp(0, tx);
        let y2 = y2.clamp(0, ty);
        if x2 <= x1 || y2 <= y1 {
            return;
        }
        let c = colour & self.colour_mask();
        let w = self.width as usize;
        let bm = self.bitmap_mut(bitmap);
        for y in y1..y2 {
            let row = y as usize * w;
            bm[row + x1 as usize..row + x2 as usize].fill(c);
        }
    }

    /// Applies the pending Screen Display / Screen Offset values (start of
    /// `EcCopper`, +W.s:5713-5772). Returns true if the vertical layout
    /// changed.
    pub fn apply_pending(&mut self) -> bool {
        let mut changed = false;
        let [x, y, w, h] = std::mem::take(&mut self.pending_display);
        if let Some(y) = y {
            // EcWY = y + EcYBase, negative -> 0.
            self.display_y = y.max(-0x1000);
            changed = true;
        }
        if let Some(x) = x {
            self.display_x = x & !15;
        }
        if let Some(h) = h
            && h != 0
        {
            let mut h = (h as u32 & 0xFFFF).min(self.height);
            if self.lace {
                h /= 2;
            }
            self.display_h = h;
            changed = true;
        }
        if let Some(w) = w {
            let w = w as u32 & 0xFFF0;
            if w != 0 {
                let tx = if self.hires {
                    self.width / 2
                } else {
                    self.width
                };
                self.display_w = w.min(tx);
            }
        }
        let [ox, oy] = std::mem::take(&mut self.pending_offset);
        if let Some(oy) = oy {
            self.offset_y = oy;
        }
        if let Some(ox) = ox {
            self.offset_x = ox;
        }
        changed
    }

    /// Screen -> hardware coordinates (`CXyHard`, +W.s:10763): no offset.
    pub fn x_hard(&self, x: i32) -> i32 {
        (if self.hires { x >> 1 } else { x }) + self.display_x
    }

    pub fn y_hard(&self, y: i32) -> i32 {
        (if self.lace { y >> 1 } else { y }) + self.display_y
    }

    /// Hardware -> screen coordinates (`CXyScr`, +W.s:10741): with offset.
    pub fn x_screen(&self, x: i32) -> i32 {
        let d = x - self.display_x;
        (if self.hires { d * 2 } else { d }) + self.offset_x
    }

    pub fn y_screen(&self, y: i32) -> i32 {
        let d = y - self.display_y;
        (if self.lace { d * 2 } else { d }) + self.offset_y
    }

    /// Zone containing the screen coordinates (`GZone`, +W.s:11110), 0 if
    /// none.
    pub fn zone_at(&self, x: i32, y: i32) -> i32 {
        let (x, y) = (x as u16, y as u16);
        for (i, z) in self.zones.iter().enumerate() {
            if z[2] == 0 && z[3] == 0 {
                continue;
            }
            if x >= z[0] && y >= z[1] && x <= z[2] && y <= z[3] {
                return i as i32 + 1;
            }
        }
        0
    }

    /// Zone under the hardware coordinates (`ZoEc`, +W.s:11083).
    pub fn zone_at_hard(&self, hx: i32, hy: i32) -> i32 {
        // Word arithmetic (an omitted coordinate is EntNul, low word 0).
        let dx = hx.wrapping_sub(self.display_x) as u16;
        let dy = hy.wrapping_sub(self.display_y) as u16;
        if dx as u32 >= self.display_w || dy as u32 >= self.display_h {
            return 0;
        }
        let mut x = dx as i32;
        let mut y = dy as i32;
        if self.hires {
            x <<= 1;
        }
        if self.lace {
            y <<= 1;
        }
        self.zone_at(x + self.offset_x, y + self.offset_y)
    }

    /// `Set Zone` (`SySetZ`, +W.s:11044). Error codes: 29 no zones, 1 bad
    /// parameter.
    pub fn set_zone(&mut self, n: i32, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<(), u16> {
        if self.zones.is_empty() {
            return Err(29);
        }
        let n = n as u16 as usize;
        if n == 0 || n > self.zones.len() {
            return Err(1);
        }
        let (x1, y1, x2, y2) = (x1 as u16, y1 as u16, x2 as u16, y2 as u16);
        if x1 >= x2 || y1 >= y2 {
            return Err(1);
        }
        self.zones[n - 1] = [x1, y1, x2, y2];
        Ok(())
    }
}

/// Bitmap reference returned by `=Physic` / `=Logic` (decoded by `GetEc`,
/// +Lib.s:11289): bit 31 set = special value, bit 30 = physic; the low
/// word is the screen number, negative for the current screen.
pub fn physic_value(screen: Option<i32>) -> i32 {
    match screen {
        None => -1,
        Some(n) => n | (1 << 31) | (1 << 30),
    }
}

pub fn logic_value(screen: Option<i32>) -> i32 {
    match screen {
        None => !(1 << 30),
        Some(n) => (n | (1 << 31)) & !(1 << 30),
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
    /// `Default Palette` (`DefPal`): palette of new screens.
    pub default_palette: [u16; 32],
    /// Auto View On (`BitEcrans` of `ActuMask`).
    pub auto_view: bool,
    /// Flash, shift, fade and rainbows.
    pub effects: Effects,
}

impl Screens {
    pub fn new() -> Self {
        Screens {
            screens: (0..MAX_SCREENS).map(|_| None).collect(),
            default_palette: DEFAULT_PALETTE,
            auto_view: true,
            ..Default::default()
        }
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

    /// `Screen Open` (`EcCree`, +W.s:2882): validates, creates the screen
    /// with the default palette and its full screen text window 0.
    pub fn open(
        &mut self,
        n: usize,
        width: i32,
        height: i32,
        colours: u32,
        mode: u32,
    ) -> Result<(), u16> {
        let w = width & !15;
        if w <= 0 || w >= 1024 || height <= 0 || height >= 1024 {
            return Err(E_ILLEGAL_PARAMETER);
        }
        if self.get(n).is_some() {
            self.remove(n);
        }
        let mut s = Screen::new(n, w as u32, height as u32, colours, mode);
        s.palette = self.default_palette;
        // Text window 0: full screen, CLW, no border (+W.s:3056).
        let tx = (s.width >> 4) << 1;
        let ty = s.height >> 3;
        s.wind_open(0, 0, 0, tx as i32, ty as i32, 0, true)
            .map_err(|_| E_OUT_OF_MEMORY)?;
        let (pen, paper) = s.text.windows.first().map_or((2, 1), |w| (w.pen, w.paper));
        s.gr.ink = pen as u8;
        s.gr.paper = paper as u8;
        s.gr.outline = pen as u8;
        s.gr.writing = 1;
        s.gr.line_pattern = 0xFFFF;
        s.gr.x = 0;
        s.gr.y = 0;
        self.insert(s);
        Ok(())
    }

    /// `Screen Close` (`EcDel`, +W.s:3324).
    pub fn remove(&mut self, n: usize) {
        let Some(s) = self.screens.get_mut(n).and_then(Option::take) else {
            return;
        };
        self.priority.retain(|&p| p != n);
        // Dual playfield partner reverts to a single playfield.
        if let Some(d) = s.dual_with
            && let Some(o) = self.get_mut(d)
        {
            o.dual_slave = false;
            o.hidden = false;
        }
        if s.dual_slave {
            for o in self.screens.iter_mut().flatten() {
                if o.dual_with == Some(n) {
                    o.dual_with = None;
                    o.dual_priority = false;
                }
            }
        }
        self.effects.stop_screen(n);
        if self.current == Some(n) {
            // The front non clone screen, < 8 if possible.
            let pick = |max: usize| {
                self.priority
                    .iter()
                    .copied()
                    .find(|&p| self.get(p).is_some_and(|s| s.clone_of.is_none() && p < max))
            };
            self.current = pick(8).or_else(|| pick(MAX_SCREENS));
        }
    }

    /// `Screen n` (`EcMarch`): clones cannot be made current.
    pub fn activate(&mut self, n: usize) -> Result<(), u16> {
        let s = self.get(n).ok_or(E_NOT_OPENED)?;
        if s.clone_of.is_some() {
            return Err(E_ILLEGAL_PARAMETER);
        }
        self.current = Some(n);
        Ok(())
    }

    /// `Screen To Front` (`EcFirst`).
    pub fn to_front(&mut self, n: usize) -> Result<(), u16> {
        self.get(n).ok_or(E_NOT_OPENED)?;
        self.priority.retain(|&s| s != n);
        self.priority.insert(0, n);
        Ok(())
    }

    /// `Screen To Back` (`EcLast`).
    pub fn to_back(&mut self, n: usize) -> Result<(), u16> {
        self.get(n).ok_or(E_NOT_OPENED)?;
        self.priority.retain(|&s| s != n);
        self.priority.push(n);
        Ok(())
    }

    /// `Screen Hide/Show` (`EcHide`): no effect on a dual playfield slave.
    pub fn set_hidden(&mut self, n: usize, hide: bool) -> Result<(), u16> {
        let s = self.get_mut(n).ok_or(E_NOT_OPENED)?;
        if !s.dual_slave {
            s.hidden = hide;
        }
        Ok(())
    }

    /// `Screen Clone n` (`EcCClo`, +W.s:2706): copy of the current screen
    /// showing its bitmap, in front.
    pub fn clone_current(&mut self, n: usize) -> Result<(), u16> {
        if self.get(n).is_some() {
            return Err(2);
        }
        let cur = self.current.ok_or(E_NOT_OPENED)?;
        let src = self.get(cur).ok_or(E_NOT_OPENED)?;
        let mut c = src.clone();
        c.number = n;
        c.zones.clear();
        c.text = TextState::default();
        c.clone_of = Some(src.clone_of.unwrap_or(cur));
        c.dual_with = None;
        c.dual_slave = false;
        self.screens[n] = Some(Box::new(c));
        self.priority.insert(0, n);
        Ok(())
    }

    /// `Double Buffer` (`EcDouble`, +W.s:2744).
    pub fn double_buffer(&mut self) -> Result<(), u16> {
        let s = self.current_mut().ok_or(E_NOT_OPENED)?;
        if s.is_double_buffered() {
            return Err(E_DOUBLE_BUFFER);
        }
        let copy = s.bitmaps[0].clone();
        s.bitmaps.push(copy);
        s.logic = 1;
        s.physic = 0;
        s.autoback = 2;
        s.version += 1;
        Ok(())
    }

    /// `Screen Swap n` (`ScSwap`): exchanges logic and physic.
    pub fn swap(&mut self, n: usize) -> Result<(), u16> {
        let s = self.get_mut(n).ok_or(E_NOT_OPENED)?;
        if s.is_double_buffered() {
            std::mem::swap(&mut s.logic, &mut s.physic);
            s.version += 1;
        }
        Ok(())
    }

    /// `Screen Swap` (`ScSwapS`): every double buffered user screen.
    pub fn swap_all(&mut self) {
        for n in 0..8 {
            let _ = self.swap(n);
        }
    }

    /// `Dual Playfield s1,s2` (`Duale`, +W.s:2790).
    pub fn set_dual(&mut self, s1: usize, s2: usize) -> Result<(), u16> {
        if s1 == s2 {
            return Err(E_CANT_DUAL);
        }
        let a = self.get(s1).ok_or(E_NOT_OPENED)?;
        let b = self.get(s2).ok_or(E_NOT_OPENED)?;
        if a.dual_with.is_some() || a.dual_slave || b.dual_with.is_some() || b.dual_slave {
            return Err(E_CANT_DUAL);
        }
        let max = if a.hires { 2 } else { 3 };
        if a.hires != b.hires || a.lace != b.lace || a.ham || b.ham {
            return Err(E_CANT_DUAL);
        }
        let (n1, n2) = (a.planes, b.planes);
        if n1 > max || n2 > max || !(n1 == n2 || n2 + 1 == n1) {
            return Err(E_CANT_DUAL);
        }
        let a = self.get_mut(s1).unwrap();
        a.dual_with = Some(s2);
        a.dual_priority = false;
        let b = self.get_mut(s2).unwrap();
        b.dual_slave = true;
        b.hidden = true;
        Ok(())
    }

    /// `Dual Priority s1,s2` (`DualP`, +W.s:2846): puts playfield s1 in
    /// front of s2.
    pub fn dual_priority(&mut self, s1: usize, s2: usize) -> Result<(), u16> {
        if s1 == s2 {
            return Err(E_NOT_DUAL);
        }
        let a = self.get(s1).ok_or(E_NOT_OPENED)?;
        let b = self.get(s2).ok_or(E_NOT_OPENED)?;
        let a_dual = a.dual_with.is_some() || a.dual_slave;
        let b_dual = b.dual_with.is_some() || b.dual_slave;
        if !a_dual || !b_dual {
            return Err(E_NOT_DUAL);
        }
        if b.dual_slave {
            // s1 is the master: playfield 1 in front.
            self.get_mut(s1).unwrap().dual_priority = false;
        } else {
            self.get_mut(s2).unwrap().dual_priority = true;
        }
        Ok(())
    }

    /// Applies pending display changes of every screen (`EcCopper`).
    pub fn apply_pending(&mut self) {
        for s in self.screens.iter_mut().flatten() {
            s.apply_pending();
        }
    }

    /// Screen displayed at hardware position (`GetSIn`, +W.s:10831):
    /// searches in priority order starting at screen `first` (or the
    /// front), screens below `max` only.
    pub fn screen_at(&self, hx: i32, hy: i32, first: Option<usize>, max: usize) -> Option<usize> {
        let start = first
            .and_then(|f| self.priority.iter().position(|&p| p == f))
            .unwrap_or(0);
        for &n in &self.priority[start.min(self.priority.len())..] {
            let Some(s) = self.get(n) else { continue };
            if n >= max || s.hidden {
                continue;
            }
            // Word arithmetic as in `GetSIn` (+W.s): an omitted coordinate
            // (EntNul) is word 0.
            let dx = hx.wrapping_sub(s.display_x) as u16;
            let dy = hy.wrapping_sub(s.display_y) as u16;
            if (dx as u32) < s.display_w && (dy as u32) < s.display_h {
                return Some(n);
            }
        }
        None
    }

    /// Decodes a screen / bitmap reference (`GetEc`, +Lib.s:11289): a
    /// screen number, or a `=Logic` / `=Physic` value. Returns the screen
    /// and the bitmap index. Errors are AMOS error numbers.
    pub fn resolve_bitmap(&self, v: i32) -> Result<(usize, usize), u16> {
        let (n, physic) = if v >= 0 {
            if v >= 8 {
                return Err(23);
            }
            (v as usize, false)
        } else if (v as i16) < 0 {
            (self.current.ok_or(47u16)?, v & (1 << 30) != 0)
        } else {
            let n = (v & 0xFFFF) as usize;
            if n >= 8 {
                return Err(23);
            }
            (n, v & (1 << 30) != 0)
        };
        let s = self.get(n).ok_or(47u16)?;
        Ok((n, if physic { s.physic } else { s.logic }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_display_position() {
        let s = Screen::new(0, 320, 256, 16, 0);
        assert_eq!(
            (s.display_x, s.display_y, s.display_w, s.display_h),
            (128, 42, 320, 256)
        );
        let s = Screen::new(0, 320, 200, 16, 0);
        assert_eq!((s.display_x, s.display_y), (128, 50));
        let s = Screen::new(0, 640, 200, 16, MODE_HIRES);
        assert_eq!((s.display_x, s.display_w), (128, 320));
        let s = Screen::new(0, 352, 200, 16, 0);
        assert_eq!(s.display_x, 112);
        let s = Screen::new(0, 320, 400, 16, MODE_LACED);
        assert_eq!((s.display_y, s.display_h), (50, 200));
    }

    #[test]
    fn open_close_priority() {
        let mut ss = Screens::new();
        ss.open(0, 320, 200, 16, 0).unwrap();
        ss.open(1, 320, 100, 4, 0).unwrap();
        assert_eq!(ss.priority, vec![1, 0]);
        assert_eq!(ss.current, Some(1));
        ss.to_back(1).unwrap();
        assert_eq!(ss.priority, vec![0, 1]);
        ss.remove(1);
        assert_eq!(ss.current, Some(0));
        assert_eq!(ss.open(2, 8, 10, 2, 0), Err(E_ILLEGAL_PARAMETER));
    }

    #[test]
    fn coordinates() {
        let s = Screen::new(0, 640, 200, 4, MODE_HIRES);
        assert_eq!(s.x_hard(100), 128 + 50);
        assert_eq!(s.x_screen(178), 100);
        assert_eq!(s.y_hard(10), 60);
    }

    #[test]
    fn bitmap_refs() {
        let mut ss = Screens::new();
        ss.open(1, 320, 200, 16, 0).unwrap();
        ss.double_buffer().unwrap();
        assert_eq!(ss.resolve_bitmap(1), Ok((1, 1)));
        assert_eq!(ss.resolve_bitmap(physic_value(Some(1))), Ok((1, 0)));
        assert_eq!(ss.resolve_bitmap(logic_value(None)), Ok((1, 1)));
        assert_eq!(ss.resolve_bitmap(physic_value(None)), Ok((1, 0)));
        assert_eq!(ss.resolve_bitmap(3), Err(47));
    }
}
