//! The AMOS Professional "Interface" dialog system (`+Lib.s:19837-25400`):
//! resource banks, the Interface language (a small interpreted UI
//! language with RPN expressions), dialog channels and their active zones
//! (buttons, edit fields, sliders, lists, hypertext), the `Hslider` /
//! `Vslider` drawings, the text reader (`Read Text`) and the file selector
//! (`Fsel$`).
//!
//! The original runs the Interface programs with the 68000 stack: a
//! `RUn` instruction waits in a loop for the user. Here the blocking BASIC
//! functions return `Exc::Block` and are entered again at the next frame,
//! so the interpreter keeps its state (program position, nested blocks,
//! user instruction calls) in the [`Channel`].
//!
//! Decisions about the original's quirks:
//! * the lexer skips lower case letters and `|`: kept (the `|` operator is
//!   unreachable);
//! * "IL" always resolves to the debug instruction #8 (a 68000 `illegal`):
//!   it is reported as a syntax error here, and Inactive List stays
//!   unreachable;
//! * `*` and `/` are 16 bit (`muls`, `divs`): kept;
//! * `GP` and `GE` set XA/YA/XB/YB from the wrong registers, `GP` ignores
//!   the base, `VT` adds the height to XB, `SP`'s outline flag sets an
//!   unused RastPort bit: all kept;
//! * values are untyped longs in the original (strings are addresses); here
//!   they are typed and using a string as a number (or the reverse) is an
//!   Interface "type mismatch" error instead of reading random memory;
//! * while a button / slider / list is held down from an automatic test
//!   (dialog opened without `RUn`), the original stops the BASIC program;
//!   here the program goes on and the zone result is only given when the
//!   mouse is released;
//! * a `RUn` inside a zone routine (draw / change) does not wait;
//! * an `Every` or `On Break` event while a dialog waits restarts the
//!   waiting instruction (the original does not handle events in `RUn`);
//! * `JP` to an undefined label goes to the start of the program as in the
//!   original; an endless loop is stopped with a syntax error.
//!
//! The automatic tests of dialogs opened without `RUn` are done by
//! [`crate::machine::Hardware::dialogs_test_point`], to be called at each
//! test point of the interpreter.

pub mod engine;
pub mod fsel;
pub mod lexer;
pub mod readtext;
pub mod resource;
pub mod slider;
#[cfg(test)]
mod tests;
pub mod zones;

use std::collections::HashMap;
use std::rc::Rc;

use crate::input::KeyPress;
pub use resource::Resource;

/// Interface error codes (`+Equ.s:1123`); the AMOS error is 119 + code.
pub mod e {
    pub const SYNTAX: u8 = 1;
    pub const OUT_OF_MEMORY: u8 = 2;
    pub const LABEL_DEFINED: u8 = 3;
    pub const LABEL_NOT_DEFINED: u8 = 4;
    pub const CHANNEL_DEFINED: u8 = 5;
    pub const CHANNEL_NOT_DEFINED: u8 = 6;
    pub const SCREEN: u8 = 7;
    pub const VAR_NOT_DEFINED: u8 = 8;
    pub const FCALL: u8 = 9;
    pub const TYPE: u8 = 10;
    pub const BUFFER: u8 = 11;
    pub const NPARAM: u8 = 12;
}

/// `IDia_Errors`: base of the Interface errors.
pub const ERROR_BASE: u16 = 119;

/// Channel used by the file selector (`Fs_ChannelN`).
pub const FSEL_CHANNEL: i64 = 0xAABBCCDD;
/// First channel number of temporary channels (Dialog Box, Read Text).
pub const QUICK_CHANNEL: i64 = 65536;

/// A value of the Interface language. Strings and arrays are addresses in
/// the original.
#[derive(Clone, Debug)]
pub enum DVal {
    Int(i32),
    Str(Rc<[u8]>),
    Arr(ArrRef),
}

impl Default for DVal {
    fn default() -> Self {
        DVal::Int(0)
    }
}

impl DVal {
    pub fn str(s: &[u8]) -> DVal {
        DVal::Str(Rc::from(s))
    }
}

/// An array given to the Interface (`Array(a$(0))` in BASIC, or the file
/// selector's "magic" array).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrRef {
    Basic { slot: u16, frame: usize },
    Fsel,
}

/// An entry of the interpreter stack (`a3`): values (user instruction
/// parameters) and `JS` return addresses (marked "MoiM").
#[derive(Clone, Debug)]
pub enum SVal {
    Val(DVal),
    Ret(usize),
}

/// What to do when a nested `Dia_Loop` returns.
#[derive(Clone, Debug)]
pub enum FrameKind {
    /// `Dialog Run`: the program ends.
    Root,
    /// `IF cond [block]`: skip the block.
    If,
    /// User instruction call: back to the caller.
    User {
        caller_pc: usize,
        stack: Vec<SVal>,
        pusers: Option<usize>,
        npusers: u16,
    },
    /// Routine called by the zone code (draw / change routines).
    Sync { caller_pc: usize },
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub kind: FrameKind,
    /// Position (a6) and stack (a3) when the loop was entered: restored
    /// when it returns (`Dia_Quit`).
    pub entry_pc: usize,
    pub stack: Vec<SVal>,
}

/// `RUn` in progress.
#[derive(Clone, Debug)]
pub struct RunWait {
    /// Waiting for the mouse button to be released (`RUn` flag bit 1).
    pub release: bool,
}

/// Mouse held on a zone during the tests (`Dia_NoMKey` and the slider
/// loops of `Sl_Clic`): continued at each test until the button is
/// released.
#[derive(Clone, Debug)]
pub enum Modal {
    /// Button: position to set on release (result of the change routine).
    Button {
        zone: usize,
        pos: i32,
    },
    /// List or hypertext zone.
    Hold {
        zone: usize,
        quit_check: bool,
    },
    /// Slider arrows (page up/down) and knob drag.
    SliderDown {
        zone: usize,
    },
    SliderUp {
        zone: usize,
    },
    SliderDrag {
        zone: usize,
        grab: i32,
    },
}

/// The line editor of edit zones (`LEd_*` `+Lib.s:19404`).
#[derive(Clone, Debug, Default)]
pub struct LineEd {
    pub buf: Vec<u8>,
    pub max: usize,
    pub start: i32,
    pub large: i32,
    pub cur: i32,
    pub x: i32,
    pub y: i32,
    pub screen: usize,
    /// LEd_Flags high byte (behaviour flags).
    pub flags: u8,
    /// LEd_Flags low byte: bit 0 erase the right, bit 1 clear the line,
    /// bit 2 cursor shown.
    pub state: u8,
    /// Accepted characters 32..127 (`LEd_Mask`), when filtering.
    pub mask: [u32; 3],
}

pub mod led {
    pub const KEYS: u8 = 1 << 0;
    pub const ONCE: u8 = 1 << 1;
    pub const CURSOR: u8 = 1 << 2;
    pub const FILTER: u8 = 1 << 3;
    pub const MOUSE: u8 = 1 << 4;
    pub const MOUSE_CURSOR: u8 = 1 << 7;
}

#[derive(Clone, Debug)]
pub struct ListZone {
    pub tx: i16,
    pub ty: i16,
    pub pos: i16,
    pub max_act: i16,
    pub array: Option<ArrRef>,
    /// Number of digits of the initial size (`Dia_LiLArray`).
    pub larray: i16,
    pub act: i16,
    /// Flags: bit 0 show numbers, bit 1 numbers from 1, bit 2 select on
    /// click only.
    pub lflags: u8,
}

/// An active part of a hypertext line (8 bytes in the original).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HtZone {
    /// Start / end of the displayed text in the line.
    pub start: u8,
    pub end: u8,
    /// First and last+1 columns on the screen (1, 0 = inactive).
    pub c0: u8,
    pub c1: u8,
    /// Paper and pen, as ASCII digits ('0' + n).
    pub paper: u8,
    pub pen: u8,
    /// Start of the keyword in the line.
    pub kw: u8,
}

#[derive(Clone, Debug, Default)]
pub struct HtRow {
    pub line: Option<usize>,
    pub zones: Vec<HtZone>,
}

#[derive(Clone, Debug)]
pub struct TextZone {
    pub tx: i16,
    pub ty: i16,
    pub pos: i16,
    pub text: Rc<[u8]>,
    /// Start offset and length (a byte) of each line.
    pub lines: Vec<(usize, u8)>,
    pub rows: Vec<HtRow>,
    pub disp_max: u16,
    /// Highlighted active word (row, index) and its row.
    pub act: Option<(usize, usize)>,
    pub pen: u8,
    pub paper: u8,
    pub pp: Vec<u8>,
    /// Keyword of the last clicked text word (`Dia_TxBuffer`).
    pub buffer: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum ZoneKind {
    Button {
        rdraw: usize,
        min: i16,
        max: i16,
    },
    /// Edit (`ED`) or digit (`DI`, with its value) zone.
    Edit {
        led: LineEd,
        digit: Option<i32>,
    },
    List(ListZone),
    Text(Box<TextZone>),
    Slider(slider::Slider),
}

/// An active zone (`Dia_Zo*` header, `+Equ.s:1018`).
#[derive(Clone, Debug)]
pub struct Zone {
    pub number: i16,
    pub x: i16,
    pub y: i16,
    pub sx: i16,
    pub sy: i16,
    /// Offset of the change routine (0 = none).
    pub rchange: usize,
    pub pos: i32,
    pub var: DVal,
    /// bit 5 no wait, bit 6 in Button Change, bit 7 quit.
    pub flags: u8,
    pub kind: ZoneKind,
}

/// Records of the channel buffer, in creation order.
#[derive(Clone, Debug)]
pub enum Record {
    Zone(Zone),
    /// `KY` shortcut, attached to the last button / edit zone.
    Key {
        code: u8,
        shift: u8,
        zone: Option<usize>,
    },
    /// `SA`: background saved in a block.
    Block(i32),
}

/// `Dia_Edited`: edit zone receiving the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edited {
    None,
    /// -1: the last zone was left with Tab / Return.
    Off,
    Zone(usize),
}

/// A dialog channel (`Dia_*` structure, `+Equ.s:966`).
#[derive(Clone, Debug)]
pub struct Channel {
    pub number: i64,
    /// Variables 0..nvar.
    pub vars: Vec<DVal>,
    /// Hidden variables -1 (button position), -2 (button return) and -3
    /// (zone being drawn / changed).
    pub v_bp: DVal,
    pub v_br: DVal,
    pub v_zone: Option<usize>,
    pub prog: Rc<[u8]>,
    pub labels: Vec<(i32, usize)>,
    pub users: Vec<([u8; 2], u16, usize)>,
    pub res: Resource,
    pub screen: usize,
    pub screen_id: usize,
    pub screen_old: Option<usize>,
    pub wind_old: i32,
    pub wind_on: i32,
    /// Buffer accounting (bytes): size, labels, persistent end, temp end.
    pub buf_size: usize,
    pub label_bytes: usize,
    pub pbuf: usize,
    pub abuf: usize,
    pub records: Vec<Record>,
    pub edited: Edited,
    pub timer: u32,
    pub timer_pos: u64,
    pub last_zone: Option<usize>,
    pub next_zone: DVal,
    pub release: Option<usize>,
    pub base_x: i32,
    pub base_y: i32,
    pub sx: i32,
    pub sy: i32,
    pub xa: i16,
    pub ya: i16,
    pub xb: i16,
    pub yb: i16,
    pub puzzle_sx: i32,
    pub puzzle_sy: i32,
    pub puzzle_i: i32,
    pub last_key: Option<KeyPress>,
    pub error: u8,
    pub error_pos: usize,
    pub ret: i16,
    pub exit: i16,
    pub writing: u8,
    /// bit 0 drawn (to erase), bit 1 a RUn happened, bit 2 frozen.
    pub rflags: u8,
    /// RUn flags.
    pub flags: u8,
    pub sl_default: [u8; 16],
    // Execution state.
    pub pc: usize,
    pub frames: Vec<Frame>,
    pub stack: Vec<SVal>,
    pub pusers: Option<usize>,
    pub npusers: u16,
    pub users_depth: u16,
    pub run: Option<RunWait>,
    pub modal: Option<Modal>,
}

impl Channel {
    pub fn zone(&self, i: usize) -> Option<&Zone> {
        match self.records.get(i) {
            Some(Record::Zone(z)) => Some(z),
            _ => None,
        }
    }

    pub fn zone_mut(&mut self, i: usize) -> Option<&mut Zone> {
        match self.records.get_mut(i) {
            Some(Record::Zone(z)) => Some(z),
            _ => None,
        }
    }

    /// `Dia_GetZoneAd`: the k-th zone (k >= 1) numbered `n`.
    pub fn find_zone(&self, n: i32, k: i32) -> Option<usize> {
        let mut k = k - 1;
        for (i, r) in self.records.iter().enumerate() {
            if let Record::Zone(z) = r
                && z.number as i32 == n as i16 as i32
            {
                if k <= 0 {
                    return Some(i);
                }
                k -= 1;
            }
        }
        None
    }
}

/// Slider inks of a screen (`EcFInkA`... `+W.s:3078`): frame inks A, B, C,
/// pattern, then the knob ("interior") inks and pattern.
pub type SliderInks = [i32; 8];

/// Blocking operation in progress (re-entered at each frame).
#[derive(Clone, Debug)]
pub enum Blocking {
    /// `Dialog Run` of a channel that executed `RUn`.
    Run(i64),
    /// `Dialog Box`: its temporary channel.
    Box(i64),
    Fsel,
    ReadText,
}

/// Dialog state of the machine.
#[derive(Debug)]
pub struct DialogState {
    pub channels: Vec<Channel>,
    /// `Resource Bank n` (0 = default resource).
    pub bank_puzzle: i32,
    /// `=Edialog`.
    pub error: i32,
    /// Slider inks per screen, keyed by the screen identity.
    pub slider_inks: HashMap<usize, SliderInks>,
    /// Blocking BASIC function and the position of its instruction.
    pub blocking: Option<(usize, Blocking)>,
    /// `T_ClLast`: last key pressed, until a dialog clears it.
    pub cl_last: Option<KeyPress>,
    pub key_serial: u64,
    /// Identity of the program the state belongs to.
    pub program: usize,
    /// File selector preferences (`PI_FsSort`, `PI_FsSize`, `PI_FsStore`).
    pub fs_prefs: [i32; 3],
    pub fsel: Option<Box<fsel::Fsel>>,
    pub read_text: Option<Box<readtext::ReadText>>,
    /// `SM`: screen dragged by the mouse (screen, mouse offset).
    pub screen_move: Option<(usize, i32)>,
}

impl Default for DialogState {
    fn default() -> Self {
        DialogState {
            channels: Vec::new(),
            bank_puzzle: 0,
            error: 0,
            slider_inks: HashMap::new(),
            blocking: None,
            cl_last: None,
            key_serial: 0,
            program: 0,
            fs_prefs: [1, 1, 0],
            fsel: None,
            read_text: None,
            screen_move: None,
        }
    }
}

impl DialogState {
    pub fn channel_index(&self, n: i64) -> Option<usize> {
        self.channels.iter().position(|c| c.number == n)
    }

    /// Removes a channel from the list while it runs: give it back with
    /// [`DialogState::put`].
    pub fn take(&mut self, n: i64) -> Option<(usize, Channel)> {
        let i = self.channel_index(n)?;
        Some((i, self.channels.remove(i)))
    }

    pub fn put(&mut self, i: usize, ch: Channel) {
        let i = i.min(self.channels.len());
        self.channels.insert(i, ch);
    }

    /// First free temporary channel number.
    pub fn free_quick_channel(&self) -> i64 {
        let mut n = QUICK_CHANNEL;
        while self.channel_index(n).is_some() {
            n += 1;
        }
        n
    }
}
