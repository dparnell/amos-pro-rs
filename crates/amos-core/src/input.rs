//! Keyboard, mouse and joystick (`IoHandler`, `Cla_Event`, `MousInt` in
//! `+W.s`).

use std::collections::VecDeque;

use crate::display::{HW_X0, HW_Y0};

/// Mouse buttons as AMOS numbers them (Mouse Key bit 0, 1, 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// A key changed state. `scancode` is an Amiga raw key code; `ch` is
    /// the character it produced (host keyboard layout), if any.
    Key { scancode: u8, pressed: bool, ch: Option<char> },
    /// A character typed without a known key (input methods, paste, tests).
    Char(char),
    /// Mouse position in display units (see [`crate::display`]).
    MouseMove { x: f32, y: f32 },
    MouseButton { button: MouseButton, pressed: bool },
    /// Wheel movement in lines, positive = up.
    MouseWheel { delta: f32 },
}

/// One entry of the keyboard buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyPress {
    /// Qualifiers: bit 0 LShift, 1 RShift, 2 Caps Lock, 3 Ctrl, 4 LAlt,
    /// 5 RAlt, 6 LAmiga, 7 RAmiga.
    pub shift: u8,
    /// Amiga raw key code (0 for keys put with Put Key).
    pub raw: u8,
    /// ISO-8859-1 character, 0 for keys without one (function keys...).
    pub ascii: u8,
}

/// Raw key codes.
pub mod raw {
    pub const SPACE: u8 = 0x40;
    pub const BACKSPACE: u8 = 0x41;
    pub const TAB: u8 = 0x42;
    pub const ENTER: u8 = 0x43;
    pub const RETURN: u8 = 0x44;
    pub const ESC: u8 = 0x45;
    pub const DEL: u8 = 0x46;
    pub const UP: u8 = 0x4C;
    pub const DOWN: u8 = 0x4D;
    pub const RIGHT: u8 = 0x4E;
    pub const LEFT: u8 = 0x4F;
    pub const F1: u8 = 0x50;
    pub const HELP: u8 = 0x5F;
    pub const LSHIFT: u8 = 0x60;
    pub const CTRL: u8 = 0x63;
    pub const LALT: u8 = 0x64;
    pub const RALT: u8 = 0x65;
    pub const LAMIGA: u8 = 0x66;
    pub const RAMIGA: u8 = 0x67;
}

/// `Cla_Special`: characters of raw keys $40-$5F. $FF = convert with the
/// keymap, $FE = function key.
const CLA_SPECIAL: [u8; 32] = [
    0xff, 0x08, 0x09, 0x0d, 0x0d, 0x1b, 0x00, 0x00, // $40-$47
    0x00, 0x00, 0xff, 0x00, 0x1e, 0x1f, 0x1c, 0x1d, // $48-$4F
    0xfe, 0xfe, 0xfe, 0xfe, 0xfe, 0xfe, 0xfe, 0xfe, // $50-$57
    0xfe, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, // $58-$5F
];

/// Size of the keyboard buffer (32 slots, one always free).
const KEY_BUFFER: usize = 31;

/// Mouse movement accumulated in half pixels like the original.
#[derive(Debug)]
pub struct InputState {
    break_pressed: bool,
    /// Mouse position in hardware coordinates (lowres pixels / raster
    /// lines, as returned by `X Mouse` / `Y Mouse`).
    pub mouse_x: i32,
    pub mouse_y: i32,
    /// Mouse buttons: bit 0 left, bit 1 right, bit 2 middle (`Mouse Key`).
    pub mouse_buttons: u8,
    /// Buttons newly pressed since the last `Mouse Click`.
    click_latch: u8,
    /// Limits of the mouse (hardware coordinates, inclusive).
    pub mouse_limits: (i32, i32, i32, i32),
    /// Mouse wheel movement not yet read (lines).
    pub wheel: f32,
    /// State of every raw key (`T_ClTable`): bit `raw & 7` of byte `raw >> 3`.
    pub key_matrix: [u8; 16],
    pub buffer: VecDeque<KeyPress>,
    pub last_key: KeyPress,
    /// Function key strings (`Key$(1..20)`).
    pub function_keys: Vec<Vec<u8>>,
    /// Scan code and shifts of the last key read by Inkey$.
    pub scancode: u8,
    pub scanshift: u8,
    /// Keys that emulate joystick port 1: cursor keys, fire on Ctrl / Alt.
    pub joystick_keys: bool,
    /// Incremented for every key press stored (menu shortcut detection).
    pub key_serial: u64,
}

impl Default for InputState {
    fn default() -> Self {
        InputState {
            break_pressed: false,
            mouse_x: 200,
            mouse_y: 100,
            mouse_buttons: 0,
            click_latch: 0,
            mouse_limits: (0, 0, 458, 312),
            wheel: 0.0,
            key_matrix: [0; 16],
            buffer: VecDeque::new(),
            last_key: KeyPress::default(),
            function_keys: vec![Vec::new(); 20],
            scancode: 0,
            scanshift: 0,
            joystick_keys: true,
            key_serial: 0,
        }
    }
}

impl InputState {
    pub fn event(&mut self, event: InputEvent) {
        match event {
            InputEvent::Key { scancode, pressed, ch } => self.key(scancode, pressed, ch),
            InputEvent::Char(c) => {
                if let Some(a) = latin1(c) {
                    self.store(KeyPress { shift: 0, raw: 0, ascii: a });
                }
            }
            InputEvent::MouseMove { x, y } => {
                self.mouse_x = HW_X0 + (x / 2.0).floor() as i32;
                self.mouse_y = HW_Y0 + (y / 2.0).floor() as i32;
                self.clamp_mouse();
            }
            InputEvent::MouseButton { button, pressed } => {
                let bit = match button {
                    MouseButton::Left => 1,
                    MouseButton::Right => 2,
                    MouseButton::Middle => 4,
                };
                if pressed {
                    self.mouse_buttons |= bit;
                    self.click_latch |= bit;
                } else {
                    self.mouse_buttons &= !bit;
                }
            }
            InputEvent::MouseWheel { delta } => self.wheel += delta,
        }
    }

    fn clamp_mouse(&mut self) {
        let (x1, y1, x2, y2) = self.mouse_limits;
        self.mouse_x = self.mouse_x.clamp(x1, x2.max(x1));
        self.mouse_y = self.mouse_y.clamp(y1, y2.max(y1));
    }

    /// `Limit Mouse x1,y1 To x2,y2` (hardware coordinates), or no limits.
    pub fn limit_mouse(&mut self, limits: Option<(i32, i32, i32, i32)>) {
        let (mut x1, mut y1, mut x2, mut y2) = limits.unwrap_or((0, 0, 458, 312));
        x2 = x2.min(458);
        y2 = y2.min(312);
        x1 = x1.max(0);
        y1 = y1.max(0);
        if x2 < x1 {
            std::mem::swap(&mut x1, &mut x2);
        }
        if y2 < y1 {
            std::mem::swap(&mut y1, &mut y2);
        }
        self.mouse_limits = (x1, y1, x2, y2);
        self.clamp_mouse();
    }

    /// `X Mouse=`, `Y Mouse=`.
    pub fn set_mouse(&mut self, x: Option<i32>, y: Option<i32>) {
        if let Some(x) = x {
            self.mouse_x = x;
        }
        if let Some(y) = y {
            self.mouse_y = y;
        }
        self.clamp_mouse();
    }

    /// `Mouse Click`: buttons pressed since the last call.
    pub fn take_clicks(&mut self) -> u8 {
        std::mem::take(&mut self.click_latch)
    }

    pub fn key_down(&self, raw: u8) -> bool {
        let raw = raw & 0x7F;
        self.key_matrix[(raw >> 3) as usize] & (1 << (raw & 7)) != 0
    }

    /// `Key Shift`: byte 12 of the key matrix = qualifier keys $60-$67.
    pub fn shifts(&self) -> u8 {
        self.key_matrix[12]
    }

    fn key(&mut self, raw: u8, pressed: bool, ch: Option<char>) {
        let raw = raw & 0x7F;
        let (byte, bit) = ((raw >> 3) as usize, 1u8 << (raw & 7));
        if !pressed {
            self.key_matrix[byte] &= !bit;
            return;
        }
        self.key_matrix[byte] |= bit;
        if (0x60..=0x67).contains(&raw) {
            return;
        }
        let shift = self.shifts();
        let mut ascii = if (0x40..0x60).contains(&raw) { CLA_SPECIAL[(raw - 0x40) as usize] } else { 0xff };
        if ascii == 0xfe {
            // Function keys: Amiga + Fn inserts Key$ strings.
            let n = if raw >= 0x58 { raw - 0x58 + 8 } else { raw - 0x50 } as usize;
            let idx = if shift & 0x40 != 0 {
                Some(n)
            } else if shift & 0x80 != 0 {
                Some(n + 10)
            } else {
                None
            };
            if let Some(i) = idx
                && !self.function_keys[i].is_empty()
            {
                let s = self.function_keys[i].clone();
                self.put_key(&s);
                return;
            }
            ascii = 0;
        } else if ascii == 0xff {
            ascii = match ch.and_then(latin1) {
                // Control + letter: the original converts without Ctrl.
                Some(c) if c < 32 && shift & 0x08 != 0 && c != 13 && c != 9 && c != 8 && c != 27 => c + 0x60,
                Some(c) => c,
                None => 0,
            };
        }
        // Control-C breaks the program.
        if shift & 0x08 != 0 && (ascii == b'c' || ascii == b'C') {
            self.break_pressed = true;
            return;
        }
        self.store(KeyPress { shift, raw, ascii });
    }

    fn store(&mut self, k: KeyPress) {
        self.last_key = k;
        self.key_serial += 1;
        if self.buffer.len() < KEY_BUFFER {
            self.buffer.push_back(k);
        }
    }

    /// `Put Key a$`: `'` quoted text is skipped, byte 1 introduces a raw
    /// (shift, scan, ascii) triple.
    pub fn put_key(&mut self, s: &[u8]) {
        let mut i = 0;
        while i < s.len() {
            match s[i] {
                b'\'' => {
                    i += 1;
                    while i < s.len() && s[i] != b'\'' {
                        i += 1;
                    }
                    i += 1;
                }
                1 if i + 3 < s.len() => {
                    self.store(KeyPress { shift: s[i + 1], raw: s[i + 2], ascii: s[i + 3] });
                    i += 4;
                }
                c => {
                    self.store(KeyPress { shift: 0, raw: 0, ascii: c });
                    i += 1;
                }
            }
        }
    }

    /// Next key of the buffer (Inkey$).
    pub fn inkey(&mut self) -> Option<KeyPress> {
        let k = self.buffer.pop_front()?;
        self.scancode = k.raw;
        self.scanshift = k.shift;
        Some(k)
    }

    pub fn clear_keys(&mut self) {
        self.buffer.clear();
    }

    /// True once after Control-C.
    pub fn take_break(&mut self) -> bool {
        std::mem::take(&mut self.break_pressed)
    }

    /// `Joy(n)`: bit 0 up, 1 down, 2 left, 3 right, 4 fire. Port 1 is
    /// emulated with the cursor keys and Ctrl / Alt (fire); port 0 fire is
    /// the right mouse button as on the Amiga.
    pub fn joy_state(&self, port: i32) -> i32 {
        let mut v = 0;
        if port == 1 && self.joystick_keys {
            if self.key_down(raw::UP) {
                v |= 1;
            }
            if self.key_down(raw::DOWN) {
                v |= 2;
            }
            if self.key_down(raw::LEFT) {
                v |= 4;
            }
            if self.key_down(raw::RIGHT) {
                v |= 8;
            }
            if self.key_down(raw::CTRL) || self.key_down(raw::LALT) || self.key_down(raw::RALT) {
                v |= 16;
            }
        }
        if port == 0 && self.mouse_buttons & 2 != 0 {
            v |= 16;
        }
        v
    }
}

fn latin1(c: char) -> Option<u8> {
    let n = c as u32;
    if n < 256 { Some(n as u8) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_buffer() {
        let mut i = InputState::default();
        i.event(InputEvent::Key { scancode: 0x20, pressed: true, ch: Some('a') });
        i.event(InputEvent::Key { scancode: 0x20, pressed: false, ch: None });
        i.event(InputEvent::Key { scancode: raw::UP, pressed: true, ch: None });
        assert_eq!(i.inkey().unwrap().ascii, b'a');
        let up = i.inkey().unwrap();
        assert_eq!((up.ascii, up.raw), (0x1e, raw::UP));
        assert_eq!(i.joy_state(1), 1);
        // Control-C
        i.event(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
        i.event(InputEvent::Key { scancode: 0x33, pressed: true, ch: Some('\u{3}') });
        assert!(i.take_break());
        assert!(i.inkey().is_none());
    }
}
