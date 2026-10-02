//! AMAL, the AMOS Animation Language (`TokAMAL` / `Animeur` in `+W.s`
//! 7040-9167).
//!
//! A program is compiled into a stream of 16-bit words that mirrors the
//! threaded code of the original: tokens are the original handler offsets
//! (`AmJumps`), offsets are relative to the word that holds them (in words
//! here, bytes in the original). The runtime executes the same way, so the
//! quirks are kept: upper case only lexing (lower case letters and spaces
//! are decoration), strictly left to right 16-bit expressions, jump budgets
//! of 10 (main program) and 20 (autotest), one frame per For/Next loop,
//! new channels frozen until `Amal On`...
//!
//! The objects moved by the channels (sprites, bobs, screens, rainbows) are
//! reached through the [`AmalHost`] trait.

use std::collections::HashMap;
use std::rc::Rc;

// Tokens: offsets in the original `AmJumps` table (+W.s:8370-8424).
pub const T_END: u16 = 0x00;
pub const T_STANIM: u16 = 0x04;
pub const T_STMVX: u16 = 0x08;
pub const T_STMVY: u16 = 0x0C;
pub const T_WAIT: u16 = 0x10;
pub const T_PAUSE: u16 = 0x14;
pub const T_MOVE: u16 = 0x18;
pub const T_JUMP: u16 = 0x1C;
pub const T_LET: u16 = 0x20;
pub const T_IF: u16 = 0x24;
pub const T_FOR: u16 = 0x28;
pub const T_NEXT: u16 = 0x2C;
pub const T_SETA: u16 = 0x30;
pub const T_SETX: u16 = 0x34;
pub const T_SETY: u16 = 0x38;
pub const T_SETR: u16 = 0x3C;
pub const T_GETA: u16 = 0x40;
pub const T_GETX: u16 = 0x44;
pub const T_GETY: u16 = 0x48;
pub const T_GETR: u16 = 0x4C;
pub const T_ON: u16 = 0x50;
pub const T_XS: u16 = 0x54;
pub const T_YS: u16 = 0x58;
pub const T_COL: u16 = 0x5C;
pub const T_AUTON: u16 = 0x60;
pub const T_AUTOFF: u16 = 0x64;
pub const T_AEXIT: u16 = 0x68;
pub const T_DIRECT: u16 = 0x6C;
pub const T_CONST: u16 = 0x70;
pub const T_XM: u16 = 0x74;
pub const T_YM: u16 = 0x78;
pub const T_EXPEND: u16 = 0x7C;
pub const T_J0: u16 = 0x80;
pub const T_J1: u16 = 0x84;
pub const T_K1: u16 = 0x88;
pub const T_K2: u16 = 0x8C;
pub const T_EQ: u16 = 0x90;
pub const T_NE: u16 = 0x94;
pub const T_LT: u16 = 0x98;
pub const T_GT: u16 = 0x9C;
pub const T_ADD: u16 = 0xA0;
pub const T_SUB: u16 = 0xA4;
pub const T_DIV: u16 = 0xA8;
pub const T_MUL: u16 = 0xAC;
pub const T_OR: u16 = 0xB0;
pub const T_AND: u16 = 0xB4;
pub const T_SC: u16 = 0xB8;
pub const T_BC: u16 = 0xBC;
pub const T_PLAY: u16 = 0xC0;
pub const T_XH: u16 = 0xC4;
pub const T_YH: u16 = 0xC8;
pub const T_ANIM: u16 = 0xCC;
pub const T_Z: u16 = 0xD0;
pub const T_V: u16 = 0xD4;
pub const T_XOR: u16 = 0xD8;

/// Number of local registers R0-R9 (`NbInterne`).
pub const NB_LOCAL: usize = 10;

/// Kind of channel program (`AmNb & 3`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChanKind {
    Amal = 0,
    /// STOS compatible `Anim n,a$`.
    Anim = 1,
    MoveX = 2,
    MoveY = 3,
}

/// Object driven by a channel (`Channel n To ...`, `AnCanaux`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Sprite(u16),
    Bob(u16),
    ScreenDisplay(u16),
    ScreenSize(u16),
    ScreenOffset(u16),
    /// `Amal n,a$ To address`: words at an address (no Amiga memory here,
    /// the values are kept in [`AmalState::addr_blocks`]).
    Address(u32),
    Rainbow(u16),
}

/// Field of an "act block": `+2` X, `+4` Y, `+6` image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    X,
    Y,
    A,
}

/// Screen coordinate conversions available in AMAL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conv {
    XHard,
    YHard,
    XScreen,
    YScreen,
}

/// What the AMAL runtime needs from the rest of the machine.
pub trait AmalHost {
    /// Reads a field of the target object.
    fn amal_get(&mut self, t: Target, f: Field) -> i16;
    /// Writes a field, marking it as changed (flag bit + `T_Actualise`).
    fn amal_set(&mut self, t: Target, f: Field, v: i16);
    /// Mouse position in hardware coordinates (`T_XMouse`, `T_YMouse`).
    fn amal_mouse(&self) -> (i16, i16);
    /// `K1` (left) / `K2` (right) button state.
    fn amal_mouse_key(&self, right: bool) -> bool;
    /// `J0` / `J1`: joystick bits.
    fn amal_joy(&self, port: i16) -> i16;
    /// `C(n)`: entry of the last collision table.
    fn amal_col(&self, n: i16) -> i16;
    /// `BC(n,s,e)` / `SC(n,s,e)`.
    fn amal_collide(&mut self, bob: bool, n: i16, s: i16, e: i16) -> i16;
    /// `XH`, `YH`, `XS`, `YS`; `None` if the screen is not opened.
    fn amal_conv(&self, c: Conv, screen: i16, v: i16) -> Option<i16>;
    /// Beam position (`VHPOSR`), used by `Z()`.
    fn amal_vhpos(&self) -> u16;
    /// `V(n)`: music VU meter of voice n (read and clear).
    fn amal_vu(&mut self, voice: i16) -> i16;
}

/// Compilation error: AMOS error code (1..10, the BASIC error is 106+code)
/// and offset in the string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub code: u16,
    pub offset: usize,
}

const E_SYNTAX: u16 = 1;
const E_NEXT: u16 = 2;
const E_LABEL: u16 = 3;
const E_JUMP_AUTO: u16 = 4;
const E_AUTO_OPEN: u16 = 5;
const E_AUTO_ONLY: u16 = 6;
const E_LABEL_DEF: u16 = 8;
const E_IN_AUTO: u16 = 9;
const E_NO_BANK: u16 = 10;

/// Register operand of `Let`, `For`...
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reg {
    A,
    X,
    Y,
    /// Encoded as in the original: `n*2` for RA-RZ, `-(n+1)*2` for R0-R9.
    R(u16),
}

struct Compiler<'a> {
    s: &'a [u8],
    p: usize,
    out: Vec<u16>,
    labels: [Option<(usize, bool)>; 27],
    fixups: Vec<(usize, usize, bool)>,
    fors: Vec<(usize, u16)>,
    /// Position of the offset word of the open autotest.
    auto: Option<usize>,
    has_bank: bool,
}

type CResult<T> = Result<T, u16>;

impl<'a> Compiler<'a> {
    fn byte(&self, p: usize) -> u8 {
        self.s.get(p).copied().unwrap_or(0)
    }

    /// `AniChr`: next character, skipping lower case letters, spaces and
    /// anything outside 33..'Z' except `|` and `!`; ESC skips 2 more bytes.
    fn chr(&mut self) -> u8 {
        loop {
            let c = self.byte(self.p);
            self.p += 1;
            if c == 0 || (33..=b'Z').contains(&c) || c == b'|' || c == b'!' {
                return c;
            }
            if c == 27 {
                self.p += 2;
            }
        }
    }

    fn back(&mut self) {
        self.p = self.p.saturating_sub(1);
    }

    /// `StChr`: character reader of the STOS compatible strings.
    fn st_chr(&mut self) -> u8 {
        loop {
            let c = self.byte(self.p);
            self.p += 1;
            if c == b' ' {
                continue;
            }
            return c.to_ascii_uppercase();
        }
    }

    fn expect(&mut self, c: u8) -> CResult<()> {
        if self.chr() == c {
            Ok(())
        } else {
            Err(E_SYNTAX)
        }
    }

    fn st_expect(&mut self, c: u8) -> CResult<()> {
        if self.st_chr() == c {
            Ok(())
        } else {
            Err(E_SYNTAX)
        }
    }

    fn emit(&mut self, w: u16) {
        self.out.push(w);
    }

    /// `AniLong`: decimal or `$` hexadecimal number with optional `-`.
    fn long(&mut self) -> CResult<i32> {
        let mut c = self.chr();
        let neg = c == b'-';
        if neg {
            c = self.chr();
        }
        let mut v: i32;
        if c == b'$' {
            c = self.chr();
            v = hex_digit(c).ok_or(E_SYNTAX)? as i32;
            loop {
                let c = self.chr();
                match hex_digit(c) {
                    Some(d) => v = v.wrapping_shl(4).wrapping_add(d as i32),
                    None => break,
                }
            }
        } else {
            if !c.is_ascii_digit() {
                return Err(E_SYNTAX);
            }
            v = (c - b'0') as i32;
            loop {
                let c = self.chr();
                if c.is_ascii_digit() {
                    v = v.wrapping_mul(10).wrapping_add((c - b'0') as i32);
                } else {
                    break;
                }
            }
        }
        self.back();
        Ok(if neg { v.wrapping_neg() } else { v })
    }

    /// `AniReg`.
    fn reg(&mut self) -> Option<Reg> {
        let c = self.chr();
        self.reg_from(c)
    }

    /// `AniR`: register whose first letter is `c`.
    fn reg_from(&mut self, c: u8) -> Option<Reg> {
        match c {
            b'A' => Some(Reg::A),
            b'X' => Some(Reg::X),
            b'Y' => Some(Reg::Y),
            b'R' => {
                let d = self.chr();
                if d.is_ascii_digit() {
                    let n = (d - b'0') as i16;
                    Some(Reg::R((-(n + 1) * 2) as u16))
                } else if (b'A'..=b'A' + 26).contains(&d) {
                    Some(Reg::R(((d - b'A') as u16) * 2))
                } else {
                    self.back();
                    None
                }
            }
            _ => {
                self.back();
                None
            }
        }
    }

    fn label_index(c: u8) -> CResult<usize> {
        if (b'A'..=b'A' + 26).contains(&c) {
            Ok((c - b'A') as usize)
        } else {
            Err(E_SYNTAX)
        }
    }

    /// `(` exp `)` of `C()`, `Z()`, `V()`.
    fn paren1(&mut self) -> CResult<()> {
        self.expect(b'(')?;
        self.exp()?;
        self.expect(b')')
    }

    /// `(` exp `,` exp `)`.
    fn paren2(&mut self) -> CResult<()> {
        self.expect(b'(')?;
        self.exp()?;
        self.expect(b',')?;
        self.exp()?;
        self.expect(b')')
    }

    /// `AniOpe`: one operand.
    fn operand(&mut self) -> CResult<()> {
        let c = self.chr();
        match c {
            b'K' => {
                let t = match self.chr() {
                    b'1' => T_K1,
                    b'2' => T_K2,
                    _ => return Err(E_SYNTAX),
                };
                self.emit(t);
            }
            b'J' => {
                let t = match self.chr() {
                    b'0' => T_J0,
                    b'1' => T_J1,
                    _ => return Err(E_SYNTAX),
                };
                self.emit(t);
            }
            b'O' => self.emit(T_ON),
            b'S' | b'B' => {
                self.emit(if c == b'S' { T_SC } else { T_BC });
                self.expect(b'C')?;
                self.expect(b'(')?;
                self.exp()?;
                self.expect(b',')?;
                self.exp()?;
                self.expect(b',')?;
                self.exp()?;
                self.expect(b')')?;
            }
            b'C' => {
                self.emit(T_COL);
                self.paren1()?;
            }
            b'Z' => {
                self.emit(T_Z);
                self.paren1()?;
            }
            b'V' => {
                self.emit(T_V);
                self.paren1()?;
            }
            b'-' | b'$' | b'0'..=b'9' => {
                self.back();
                let v = self.long()?;
                self.emit(T_CONST);
                self.emit(v as u16);
            }
            _ => match self.reg_from(c).ok_or(E_SYNTAX)? {
                Reg::A => self.emit(T_GETA),
                Reg::R(w) => {
                    self.emit(T_GETR);
                    self.emit(w);
                }
                r => {
                    let x = r == Reg::X;
                    let n = self.chr();
                    self.back();
                    match n {
                        b'S' | b'H' => {
                            self.p += 1;
                            self.emit(match (n, x) {
                                (b'S', true) => T_XS,
                                (b'S', false) => T_YS,
                                (_, true) => T_XH,
                                _ => T_YH,
                            });
                            self.paren2()?;
                        }
                        b'M' => {
                            self.p += 1;
                            self.emit(if x { T_XM } else { T_YM });
                        }
                        _ => self.emit(if x { T_GETX } else { T_GETY }),
                    }
                }
            },
        }
        Ok(())
    }

    /// `AniExp`: operand (operator operand)*, evaluated left to right.
    fn exp(&mut self) -> CResult<()> {
        self.operand()?;
        loop {
            let c = self.chr();
            let op = match c {
                b'=' => T_EQ,
                b'<' => {
                    if self.byte(self.p) == b'>' {
                        self.p += 1;
                        T_NE
                    } else {
                        T_LT
                    }
                }
                b'>' => T_GT,
                b'+' => T_ADD,
                b'-' => T_SUB,
                b'/' => T_DIV,
                b'*' => T_MUL,
                b'|' => T_OR,
                b'&' => T_AND,
                b'!' => T_XOR,
                _ => {
                    self.emit(T_EXPEND);
                    self.back();
                    return Ok(());
                }
            };
            self.operand()?;
            self.emit(op);
        }
    }

    fn jump_ref(&mut self, auto_flag: bool) -> CResult<()> {
        let c = self.chr();
        let l = Self::label_index(c)?;
        self.fixups.push((self.out.len(), l, auto_flag));
        self.emit(0);
        Ok(())
    }

    fn in_auto(&self) -> bool {
        self.auto.is_some()
    }

    fn statement(&mut self, c1: u8) -> CResult<()> {
        match c1 {
            b'J' => {
                self.emit(T_JUMP);
                let a = self.in_auto();
                self.jump_ref(a)?;
            }
            b'L' => {
                self.emit(T_LET);
                let r = self.reg().ok_or(E_SYNTAX)?;
                self.expect(b'=')?;
                self.exp()?;
                match r {
                    Reg::A => self.emit(T_SETA),
                    Reg::X => self.emit(T_SETX),
                    Reg::Y => self.emit(T_SETY),
                    Reg::R(w) => {
                        self.emit(T_SETR);
                        self.emit(w);
                    }
                }
            }
            b'M' => {
                if self.in_auto() {
                    return Err(E_IN_AUTO);
                }
                self.emit(T_MOVE);
                self.exp()?;
                self.expect(b',')?;
                self.exp()?;
                self.expect(b',')?;
                self.exp()?;
            }
            b'F' => {
                if self.in_auto() {
                    return Err(E_IN_AUTO);
                }
                self.emit(T_FOR);
                let Some(Reg::R(w)) = self.reg() else {
                    return Err(E_SYNTAX);
                };
                self.expect(b'=')?;
                self.exp()?;
                self.expect(b'T')?;
                self.exp()?;
                self.fors.push((self.out.len(), w));
                self.emit(w);
                self.emit(0);
            }
            b'N' => {
                if self.in_auto() {
                    return Err(E_IN_AUTO);
                }
                self.emit(T_NEXT);
                let Some(Reg::R(w)) = self.reg() else {
                    return Err(E_SYNTAX);
                };
                match self.fors.last() {
                    Some(&(pos, r)) if r == w => {
                        self.fors.pop();
                        let off = pos as isize - self.out.len() as isize;
                        self.emit(off as u16);
                    }
                    _ => return Err(E_NEXT),
                }
            }
            b'I' => {
                self.emit(T_IF);
                self.exp()?;
                match self.chr() {
                    b'J' => self.statement(b'J')?,
                    b'D' => self.statement(b'D')?,
                    b'X' => self.statement(b'X')?,
                    _ => return Err(E_SYNTAX),
                }
            }
            b'W' => {
                if self.in_auto() {
                    return Err(E_IN_AUTO);
                }
                self.emit(T_WAIT);
            }
            b'P' => {
                // Raw byte test: `PLay`, not `Play` (+W.s:7404).
                if self.byte(self.p) == b'L' {
                    self.p += 1;
                    if !self.has_bank {
                        return Err(E_NO_BANK);
                    }
                    self.emit(T_PLAY);
                    self.exp()?;
                } else {
                    self.emit(T_PAUSE);
                }
            }
            b'E' => self.emit(T_END),
            b'A' => {
                if self.byte(self.p) == b'U' {
                    // AUtotest (
                    self.p += 1;
                    let c = self.chr();
                    if self.in_auto() {
                        return Err(E_AUTO_OPEN);
                    }
                    if c != b'(' {
                        return Err(E_SYNTAX);
                    }
                    if self.chr() == b')' {
                        self.emit(T_AUTOFF);
                    } else {
                        self.back();
                        self.emit(T_AUTON);
                        self.auto = Some(self.out.len());
                        self.emit(0);
                    }
                } else {
                    self.emit(T_ANIM);
                    self.exp()?;
                    let skip = self.out.len();
                    self.emit(0);
                    self.expect(b',')?;
                    self.expect(b'(')?;
                    loop {
                        self.exp()?;
                        self.expect(b',')?;
                        self.exp()?;
                        self.expect(b')')?;
                        if self.chr() != b'(' {
                            self.back();
                            break;
                        }
                    }
                    self.emit(0);
                    self.out[skip] = (self.out.len() - skip) as u16;
                }
            }
            b'D' => {
                if !self.in_auto() {
                    return Err(E_AUTO_ONLY);
                }
                self.emit(T_DIRECT);
                // Direct always targets a label of the main program.
                self.jump_ref(false)?;
            }
            b'X' => {
                if !self.in_auto() {
                    return Err(E_AUTO_ONLY);
                }
                self.emit(T_AEXIT);
                self.emit(0);
            }
            b')' => {
                let Some(a) = self.auto else {
                    return Err(E_AUTO_ONLY);
                };
                self.emit(T_AEXIT);
                self.out[a] = (self.out.len() - a) as u16;
                self.auto = None;
            }
            // Anything else is silently ignored.
            _ => {}
        }
        Ok(())
    }

    fn amal(&mut self) -> CResult<()> {
        loop {
            let c1 = self.chr();
            if c1 == 0 {
                return Ok(());
            }
            let c = self.chr();
            if c == b':' {
                let l = Self::label_index(c1)?;
                if self.labels[l].is_some() {
                    return Err(E_LABEL_DEF);
                }
                self.labels[l] = Some((self.out.len(), self.in_auto()));
                continue;
            }
            self.back();
            self.statement(c1)?;
        }
    }

    /// STOS `Anim` string: `(img,delay)(img,delay)...[L]`.
    fn stos_anim(&mut self) -> CResult<()> {
        self.emit(T_STANIM);
        self.st_expect(b'(')?;
        loop {
            let img = self.long()?;
            self.emit(img as u16);
            self.st_expect(b',')?;
            let d = self.long()? as u16;
            self.emit(d);
            if (d as i16) < 0 {
                return Err(E_SYNTAX);
            }
            self.st_expect(b')')?;
            match self.st_chr() {
                0 => {
                    self.emit(0xFFFF);
                    return Ok(());
                }
                b'L' => {
                    self.emit(0xFFFE);
                    return Ok(());
                }
                b'(' => {}
                _ => return Err(E_SYNTAX),
            }
        }
    }

    /// STOS `Move X/Y` string: `[start](speed,step,count)...[L|E [end]]`.
    fn stos_move(&mut self, kind: ChanKind) -> CResult<()> {
        self.emit(if kind == ChanKind::MoveX {
            T_STMVX
        } else {
            T_STMVY
        });
        let h = self.out.len();
        self.emit(0x8000); // start
        self.emit(0); // loop flag
        self.emit(0x8000); // end value
        let c = self.st_chr();
        if c == 0 {
            return Err(E_SYNTAX);
        }
        if c != b'(' {
            self.back();
            let v = self.long()?;
            self.out[h] = v as u16;
            self.st_expect(b'(')?;
        }
        loop {
            let speed = self.long()? as u16;
            self.emit(speed);
            if speed as i16 <= 0 {
                return Err(E_SYNTAX);
            }
            self.st_expect(b',')?;
            let step = self.long()? as u16;
            self.emit(step);
            self.st_expect(b',')?;
            let count = self.long()? as u16;
            self.emit(count);
            if (count as i16) < 0 {
                return Err(E_SYNTAX);
            }
            self.st_expect(b')')?;
            let c = self.st_chr();
            if c == b'(' {
                continue;
            }
            self.emit(0);
            match c {
                0 => return Ok(()),
                b'L' => self.out[h + 1] = 0xFFFF,
                b'E' => {}
                _ => return Err(E_SYNTAX),
            }
            if self.st_chr() == 0 {
                return Ok(());
            }
            self.back();
            let v = self.long()?;
            self.out[h + 2] = v as u16;
            return Ok(());
        }
    }

    /// Pass 2: final End token and label resolution.
    fn finish(&mut self) -> Result<(), CompileError> {
        self.emit(T_END);
        for &(pos, l, auto) in &self.fixups {
            let Some((target, lauto)) = self.labels[l] else {
                return Err(CompileError {
                    code: E_LABEL,
                    offset: 0,
                });
            };
            if lauto != auto {
                return Err(CompileError {
                    code: E_JUMP_AUTO,
                    offset: 0,
                });
            }
            self.out[pos] = (target as isize - pos as isize) as u16;
        }
        Ok(())
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Compiles an AMAL (or STOS Anim / Move) string.
pub fn compile(src: &[u8], kind: ChanKind, has_bank: bool) -> Result<Vec<u16>, CompileError> {
    let mut c = Compiler {
        s: src,
        p: 0,
        out: Vec::new(),
        labels: [None; 27],
        fixups: Vec::new(),
        fors: Vec::new(),
        auto: None,
        has_bank,
    };
    let r = match kind {
        ChanKind::Amal => c.amal(),
        ChanKind::Anim => c.stos_anim(),
        ChanKind::MoveX | ChanKind::MoveY => c.stos_move(kind),
    };
    if let Err(code) = r {
        return Err(CompileError {
            code,
            offset: c.p.min(src.len()),
        });
    }
    c.finish()?;
    Ok(c.out)
}

/// Per frame routine of a channel (`AmAJsr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimRoutine {
    /// AMAL `Anim` (`AmDoAni`).
    Anim,
    /// STOS `Anim` string (`StAni`).
    StAnim,
    /// STOS `Move X` / `Move Y` (`StMvX` / `StMvY`).
    StMoveX,
    StMoveY,
}

/// One channel program (`AmNb` = channel*4 + kind).
#[derive(Clone, Debug)]
pub struct Channel {
    pub id: u16,
    pub code: Vec<u16>,
    /// Main program position (`AmPos`), autotest start (`AmAuto`).
    pub pos: Option<usize>,
    pub auto: Option<usize>,
    pub target: Target,
    /// Bit 15 of `AmBit`: frozen / not yet `On`.
    pub frozen: bool,
    pub cpt: i16,
    pub delt_x: i32,
    pub delt_y: i32,
    pub virg_x: u16,
    pub virg_y: u16,
    pub fin: usize,
    pub ajsr: Option<AnimRoutine>,
    pub aad: usize,
    pub aaloop: usize,
    pub acloop: i16,
    pub acpt: i16,
    /// R0..R9.
    pub regs: [i16; NB_LOCAL],
}

impl Channel {
    pub fn number(&self) -> u16 {
        self.id >> 2
    }

    pub fn kind(&self) -> u16 {
        self.id & 3
    }
}

/// State of all AMAL channels.
#[derive(Debug)]
pub struct AmalState {
    /// Channels sorted by id.
    pub channels: Vec<Channel>,
    /// Global registers RA-RZ (`T_AmRegs`).
    pub regs: [i16; 26],
    /// `Freeze`: nothing runs (`T_AmChaine` moved to `T_AmFreeze`).
    pub frozen_all: bool,
    /// `Synchro Off`: the VBL does not run the channels.
    pub sync_off: bool,
    pub seed: u16,
    /// Offset of the last compilation error (`=Amalerr`).
    pub error_pos: u16,
    /// `Channel n To ...` table (`AnCanaux`).
    pub targets: [Target; 64],
    /// Movement data used by `PLay` (`T_AmBank`): body of the AMAL bank
    /// found by the last compilation.
    pub bank: Option<Rc<Vec<u8>>>,
    /// Values of `Amal n,a$ To address` targets.
    pub addr_blocks: HashMap<u32, [i16; 3]>,
}

impl Default for AmalState {
    fn default() -> Self {
        AmalState {
            channels: Vec::new(),
            regs: [0; 26],
            frozen_all: false,
            sync_off: false,
            seed: 0x1234,
            error_pos: 0,
            targets: std::array::from_fn(|i| Target::Sprite(i as u16)),
            bank: None,
            addr_blocks: HashMap::new(),
        }
    }
}

impl AmalState {
    /// `ClrAMAL` + `AnCanaux` reset (start of a program).
    pub fn reset(&mut self) {
        *self = AmalState::default();
    }

    /// `CreAMAL`: compiles and installs a channel (replacing the same
    /// channel/kind). The new channel is frozen until `Amal On`.
    pub fn create(
        &mut self,
        channel: u16,
        kind: ChanKind,
        src: &[u8],
        target: Target,
        bank: Option<Rc<Vec<u8>>>,
    ) -> Result<(), CompileError> {
        self.error_pos = 0;
        self.bank = bank;
        let code = match compile(src, kind, self.bank.is_some()) {
            Ok(c) => c,
            Err(e) => {
                self.error_pos = e.offset as u16;
                return Err(e);
            }
        };
        let id = channel * 4 + kind as u16;
        let ch = Channel {
            id,
            code,
            pos: Some(0),
            auto: None,
            target,
            frozen: true,
            cpt: 0,
            delt_x: 0,
            delt_y: 0,
            virg_x: 0,
            virg_y: 0,
            fin: 0,
            ajsr: None,
            aad: 0,
            aaloop: 0,
            acloop: 0,
            acpt: 0,
            regs: [0; NB_LOCAL],
        };
        match self.channels.binary_search_by_key(&id, |c| c.id) {
            Ok(i) => self.channels[i] = ch,
            Err(i) => self.channels.insert(i, ch),
        }
        self.frozen_all = false;
        Ok(())
    }

    /// `MvOAMAL`: On (1), Off (-1, deletes) or Freeze (0) the channels of
    /// the kinds in `mask` (bit = kind), for one channel or all.
    pub fn on_off_freeze(&mut self, channel: Option<u16>, mask: u8, mode: i32) {
        let sel =
            |c: &Channel| channel.is_none_or(|n| c.number() == n) && mask & (1 << c.kind()) != 0;
        if mode < 0 {
            self.channels.retain(|c| !sel(c));
        } else {
            for c in self.channels.iter_mut().filter(|c| sel(c)) {
                c.frozen = mode == 0;
            }
        }
        self.frozen_all = false;
    }

    /// `DAdAMAL`: removes the channels driving `target`.
    pub fn remove_target(&mut self, target: Target) {
        self.channels.retain(|c| c.target != target);
        self.frozen_all = false;
    }

    /// Value of a register: global (channel `None`, reg 0-25) or local
    /// register of the AMAL channel. `None` if the channel does not exist.
    pub fn reg_mut(&mut self, channel: Option<u16>, reg: usize) -> Option<&mut i16> {
        match channel {
            None => self.regs.get_mut(reg),
            Some(n) => {
                let c = self.channels.iter_mut().find(|c| c.id == n * 4)?;
                c.regs.get_mut(reg)
            }
        }
    }

    /// `SetPlay` (`Amplay`): R0 = speed, R1 = direction of channels
    /// `start..=end` (by id: channel*4+kind <= end*4).
    pub fn set_play(&mut self, start: u16, end: u16, speed: Option<i16>, dir: Option<i16>) {
        for c in self.channels.iter_mut() {
            if c.id > end * 4 {
                break;
            }
            if c.id < start * 4 {
                continue;
            }
            if let Some(s) = speed {
                c.regs[0] = s;
            }
            if let Some(d) = dir {
                c.regs[1] = d;
            }
        }
    }

    fn first_of(&self, n: u16) -> Option<&Channel> {
        self.channels.iter().find(|c| c.number() == n)
    }

    /// `=Movon(n)`.
    pub fn movon(&self, n: u16) -> bool {
        match self
            .channels
            .iter()
            .find(|c| c.number() == n && c.kind() >= 2)
        {
            Some(c) => !c.frozen && c.ajsr.is_some(),
            None => false,
        }
    }

    /// `=Chanan(n)`.
    pub fn chanan(&self, n: u16) -> bool {
        self.first_of(n)
            .is_some_and(|c| !c.frozen && c.ajsr.is_some())
    }

    /// `=Chanmv(n)`.
    pub fn chanmv(&self, n: u16) -> bool {
        self.first_of(n)
            .is_some_and(|c| !c.frozen && c.pos.is_some())
    }

    /// `Animeur`: one tick of every channel.
    pub fn tick(&mut self, host: &mut dyn AmalHost) {
        if self.frozen_all {
            return;
        }
        let mut chans = std::mem::take(&mut self.channels);
        for ch in chans.iter_mut() {
            if ch.frozen {
                continue;
            }
            let mut run = Run { ch, st: self, host };
            if let Some(a) = run.ch.auto {
                run.exec(a, 20, false);
            }
            if let Some(p) = run.ch.pos {
                run.exec(p, 10, true);
            }
            if let Some(r) = run.ch.ajsr {
                run.anim(r);
            }
        }
        // Channels created while running cannot happen; keep the order.
        self.channels = chans;
    }
}

struct Run<'a> {
    ch: &'a mut Channel,
    st: &'a mut AmalState,
    host: &'a mut dyn AmalHost,
}

impl Run<'_> {
    fn w(&self, p: usize) -> u16 {
        self.ch.code.get(p).copied().unwrap_or(T_END)
    }

    fn get(&mut self, f: Field) -> i16 {
        match self.ch.target {
            Target::Address(a) => {
                let b = self.st.addr_blocks.entry(a).or_default();
                b[f as usize]
            }
            t => self.host.amal_get(t, f),
        }
    }

    fn set(&mut self, f: Field, v: i16) {
        match self.ch.target {
            Target::Address(a) => {
                let b = self.st.addr_blocks.entry(a).or_default();
                b[f as usize] = v;
            }
            t => self.host.amal_set(t, f, v),
        }
    }

    fn reg(&mut self, w: u16) -> &mut i16 {
        let w = w as i16;
        if w >= 0 {
            &mut self.st.regs[(w / 2) as usize % 26]
        } else {
            &mut self.ch.regs[((-w / 2) - 1) as usize % NB_LOCAL]
        }
    }

    /// `AmEvalue`: evaluates an expression at `*pc`.
    fn eval(&mut self, pc: &mut usize) -> i16 {
        let t = self.w(*pc);
        *pc += 1;
        let mut d3 = self.operand(t, pc);
        loop {
            let t = self.w(*pc);
            *pc += 1;
            if t == T_EXPEND || t == T_END && *pc > self.ch.code.len() {
                return d3;
            }
            let d2 = self.operand(t, pc);
            let op = self.w(*pc);
            *pc += 1;
            d3 = match op {
                T_EQ => -((d3 == d2) as i16),
                T_NE => -((d3 != d2) as i16),
                T_LT => -((d3 < d2) as i16),
                T_GT => -((d3 > d2) as i16),
                T_ADD => d3.wrapping_add(d2),
                T_SUB => d3.wrapping_sub(d2),
                T_DIV => {
                    // divs: division by zero or overflow leaves d3 unchanged.
                    if d2 == 0 {
                        d3
                    } else {
                        let q = d3 as i32 / d2 as i32;
                        if (-32768..=32767).contains(&q) {
                            q as i16
                        } else {
                            d3
                        }
                    }
                }
                T_MUL => (d3 as i32).wrapping_mul(d2 as i32) as i16,
                T_OR => d3 | d2,
                T_AND => d3 & d2,
                T_XOR => d3 ^ d2,
                _ => return d3,
            };
        }
    }

    fn operand(&mut self, t: u16, pc: &mut usize) -> i16 {
        match t {
            T_CONST => {
                let v = self.w(*pc) as i16;
                *pc += 1;
                v
            }
            T_GETA => self.get(Field::A),
            T_GETX => self.get(Field::X),
            T_GETY => self.get(Field::Y),
            T_GETR => {
                let w = self.w(*pc);
                *pc += 1;
                *self.reg(w)
            }
            T_ON => {
                if self.ch.cpt > 0 {
                    -1
                } else {
                    0
                }
            }
            T_XM => self.host.amal_mouse().0,
            T_YM => self.host.amal_mouse().1,
            T_K1 => -(self.host.amal_mouse_key(false) as i16),
            T_K2 => -(self.host.amal_mouse_key(true) as i16),
            T_J0 => self.host.amal_joy(0),
            T_J1 => self.host.amal_joy(1),
            T_XS | T_YS | T_XH | T_YH => {
                let s = self.eval(pc);
                let v = self.eval(pc);
                let c = match t {
                    T_XS => Conv::XScreen,
                    T_YS => Conv::YScreen,
                    T_XH => Conv::XHard,
                    _ => Conv::YHard,
                };
                self.host.amal_conv(c, s & 7, v).unwrap_or(-1)
            }
            T_COL => {
                let n = self.eval(pc);
                self.host.amal_col(n)
            }
            T_SC | T_BC => {
                let n = self.eval(pc);
                let s = self.eval(pc);
                let e = self.eval(pc);
                // Only with Synchro Off (they use the blitter); the original
                // does not even read its parameters otherwise.
                if self.st.sync_off {
                    self.host.amal_collide(t == T_BC, n, s, e)
                } else {
                    0
                }
            }
            T_Z => {
                let n = self.eval(pc);
                let v = (self.st.seed as u32) * 0x3171;
                let lo = (v as u16)
                    .wrapping_add(self.host.amal_vhpos())
                    .wrapping_add(1);
                self.st.seed = lo;
                let d2 = (v & 0xFFFF_0000) | lo as u32;
                ((d2 >> 8) as u16 & n as u16) as i16
            }
            T_V => {
                let n = self.eval(pc);
                self.host.amal_vu(n)
            }
            _ => 0,
        }
    }

    /// Runs code from `pc` until the channel yields for this frame.
    fn exec(&mut self, mut pc: usize, mut budget: i32, main: bool) {
        loop {
            let t = self.w(pc);
            pc += 1;
            match t {
                T_END => {
                    self.ch.pos = None;
                    self.ch.auto = None;
                    return;
                }
                T_STANIM => {
                    self.ch.acpt = 1;
                    self.ch.aad = pc;
                    self.ch.aaloop = pc;
                    self.ch.ajsr = Some(AnimRoutine::StAnim);
                    self.ch.pos = None;
                    return;
                }
                T_STMVX | T_STMVY => {
                    self.ch.acpt = 1;
                    self.ch.aaloop = pc;
                    self.ch.ajsr = Some(if t == T_STMVX {
                        AnimRoutine::StMoveX
                    } else {
                        AnimRoutine::StMoveY
                    });
                    self.ch.pos = None;
                    let f = if t == T_STMVX { Field::X } else { Field::Y };
                    self.st_move_init(pc, f);
                    return;
                }
                T_WAIT => {
                    self.ch.pos = None;
                    return;
                }
                T_PAUSE => {
                    self.ch.pos = Some(pc);
                    return;
                }
                T_MOVE => {
                    self.ch.cpt = self.ch.cpt.wrapping_sub(1);
                    if self.ch.cpt < 0 {
                        self.move_init(&mut pc);
                        return;
                    } else if self.ch.cpt == 0 {
                        pc = self.ch.fin;
                    } else {
                        self.move_step();
                        return;
                    }
                }
                T_JUMP => {
                    let off = self.w(pc) as i16 as isize;
                    pc = (pc as isize + off).max(0) as usize;
                    budget -= 1;
                    if budget == 0 {
                        if main {
                            self.ch.pos = Some(pc);
                        }
                        return;
                    }
                }
                T_LET => {
                    let v = self.eval(&mut pc);
                    let tt = self.w(pc);
                    pc += 1;
                    match tt {
                        T_SETA => self.set(Field::A, v),
                        T_SETX => self.set(Field::X, v),
                        T_SETY => self.set(Field::Y, v),
                        T_SETR => {
                            let w = self.w(pc);
                            pc += 1;
                            *self.reg(w) = v;
                        }
                        _ => {}
                    }
                }
                T_IF => {
                    let v = self.eval(&mut pc);
                    if v == 0 {
                        pc += 2;
                    }
                }
                T_FOR => {
                    let start = self.eval(&mut pc);
                    let end = self.eval(&mut pc);
                    let w = self.w(pc);
                    pc += 1;
                    *self.reg(w) = start;
                    if let Some(slot) = self.ch.code.get_mut(pc) {
                        *slot = end as u16;
                    }
                    pc += 1;
                }
                T_NEXT => {
                    let off = self.w(pc) as i16 as isize;
                    let a1 = (pc as isize + off).max(0) as usize;
                    let w = self.w(a1);
                    let r = self.reg(w);
                    *r = r.wrapping_add(1);
                    let v = *r;
                    let to = self.w(a1 + 1) as i16;
                    if to >= v {
                        // Each loop costs one frame.
                        self.ch.pos = Some(a1 + 2);
                        return;
                    }
                    pc += 1;
                }
                T_AUTON => {
                    let off = self.w(pc) as usize;
                    self.ch.auto = Some(pc + 1);
                    pc += off;
                }
                T_AUTOFF => self.ch.auto = None,
                T_AEXIT => return,
                T_DIRECT => {
                    let off = self.w(pc) as i16 as isize;
                    self.ch.pos = Some((pc as isize + off).max(0) as usize);
                    self.ch.cpt = 0;
                    return;
                }
                T_PLAY => {
                    self.ch.cpt = self.ch.cpt.wrapping_sub(1);
                    if self.ch.cpt < 0 {
                        self.ch.pos = Some(pc - 1);
                        let n = self.eval(&mut pc);
                        self.ch.fin = pc;
                        if !self.play_init(n) {
                            pc = self.ch.fin;
                            continue;
                        }
                    } else if self.ch.cpt != 0 {
                        return;
                    }
                    if !self.play_step() {
                        pc = self.ch.fin;
                        continue;
                    }
                    return;
                }
                T_ANIM => {
                    let loops = self.eval(&mut pc);
                    self.ch.acloop = loops;
                    self.ch.acpt = 1;
                    self.ch.aad = pc + 1;
                    self.ch.aaloop = pc + 1;
                    self.ch.ajsr = Some(AnimRoutine::Anim);
                    pc += self.w(pc) as usize;
                }
                // Not a statement: stop this channel for the frame.
                _ => return,
            }
        }
    }

    /// First entry in a Move: computes the 16.16 slopes and does the
    /// first step (+W.s:8508-8561).
    fn move_init(&mut self, pc: &mut usize) {
        self.ch.pos = Some(*pc - 1);
        let dx = self.eval(pc);
        let dy = self.eval(pc);
        let mut n = self.eval(pc);
        self.ch.fin = *pc;
        if n <= 0 {
            n = 1;
        }
        self.ch.cpt = n;
        let slope = |d: i16| -> i32 {
            let a = (d as i32).unsigned_abs() << 8;
            let q = a / n as u32;
            // divu overflow -> 0; the quotient is then used as a signed word.
            let q = if q > 0xFFFF { 0 } else { q as u16 };
            let q = if d < 0 { q.wrapping_neg() } else { q };
            (q as i16 as i32) << 8
        };
        self.ch.delt_x = slope(dx);
        self.ch.delt_y = slope(dy);
        let x = ((self.get(Field::X) as i32) << 16) | 0x8000;
        let y = ((self.get(Field::Y) as i32) << 16) | 0x8000;
        self.move_apply(x, y);
    }

    fn move_step(&mut self) {
        let x = ((self.get(Field::X) as i32) << 16) | self.ch.virg_x as i32;
        let y = ((self.get(Field::Y) as i32) << 16) | self.ch.virg_y as i32;
        self.move_apply(x, y);
    }

    fn move_apply(&mut self, x: i32, y: i32) {
        let ox = (x >> 16) as i16;
        let oy = (y >> 16) as i16;
        let nx = x.wrapping_add(self.ch.delt_x);
        let ny = y.wrapping_add(self.ch.delt_y);
        self.ch.virg_x = nx as u16;
        self.ch.virg_y = ny as u16;
        if (nx >> 16) as i16 != ox {
            self.set(Field::X, (nx >> 16) as i16);
        }
        if (ny >> 16) as i16 != oy {
            self.set(Field::Y, (ny >> 16) as i16);
        }
    }

    fn bank_byte(&self, i: i32) -> u8 {
        if i < 0 {
            return 0;
        }
        self.st
            .bank
            .as_ref()
            .and_then(|b| b.get(i as usize))
            .copied()
            .unwrap_or(0)
    }

    fn bank_word(&self, i: usize) -> u16 {
        u16::from_be_bytes([self.bank_byte(i as i32), self.bank_byte(i as i32 + 1)])
    }

    /// `AmPli`: finds movement `n` in the AMAL bank.
    fn play_init(&mut self, n: i16) -> bool {
        if self.st.bank.is_none() {
            return false;
        }
        let count = self.bank_word(4);
        let n = n as u16;
        if n > count || n == 0 {
            return false;
        }
        let entry = self.bank_word(4 + 2 * n as usize);
        if entry == 0 {
            return false;
        }
        let h = 4 + 2 * entry as usize;
        self.ch.regs[0] = self.bank_word(h) as i16;
        self.ch.regs[1] = 1;
        let yoff = self.bank_word(h + 2) as i16 as i32;
        self.ch.delt_y = h as i32 + 1 + yoff;
        self.ch.delt_x = h as i32 + 5;
        self.ch.virg_x = 0;
        self.ch.virg_y = 0;
        true
    }

    /// `AmPl0`: one step of a Play; false when it ended.
    fn play_step(&mut self) -> bool {
        let dir = self.ch.regs[1];
        if dir < 0 {
            return false;
        }
        for f in [Field::X, Field::Y] {
            let (ptr, wait) = match f {
                Field::X => (self.ch.delt_x, self.ch.virg_x),
                _ => (self.ch.delt_y, self.ch.virg_y),
            };
            let b = self.bank_byte(ptr);
            if b == 0 {
                return false;
            }
            let mut ptr = ptr;
            let mut wait = wait;
            if b & 0x80 == 0 {
                let d = ((b << 1) as i8 >> 1) as i16;
                let v = self.get(f);
                if dir != 0 {
                    self.set(f, v.wrapping_add(d));
                    ptr += 1;
                } else {
                    self.set(f, v.wrapping_sub(d));
                    ptr -= 1;
                }
            } else {
                // Pause: counter loaded the first time, then decremented.
                wait = wait.wrapping_sub(1);
                let advance = if wait == 0 {
                    true
                } else if (wait as i16) > 0 {
                    false
                } else {
                    wait = (b & 0x7F) as u16;
                    wait == 0
                };
                if advance {
                    ptr += if dir != 0 { 1 } else { -1 };
                }
            }
            match f {
                Field::X => {
                    self.ch.delt_x = ptr;
                    self.ch.virg_x = wait;
                }
                _ => {
                    self.ch.delt_y = ptr;
                    self.ch.virg_y = wait;
                }
            }
        }
        self.ch.cpt = self.ch.regs[0];
        true
    }

    /// Init part of the STOS Move X/Y (`StML`): `h` is the header.
    fn st_move_init(&mut self, h: usize, f: Field) {
        let start = self.w(h);
        if start != 0x8000 {
            self.set(f, start as i16);
        }
        self.ch.delt_y = self.w(h + 2) as i16 as i32;
        let p = h + 3;
        self.ch.aad = p;
        self.ch.delt_x = self.w(p + 2) as i16 as i32;
    }

    fn anim(&mut self, r: AnimRoutine) {
        match r {
            AnimRoutine::Anim => {
                self.ch.acpt = self.ch.acpt.wrapping_sub(1);
                if self.ch.acpt != 0 {
                    return;
                }
                let mut p = self.ch.aad;
                loop {
                    if self.w(p) != 0 {
                        let img = self.eval(&mut p);
                        self.set(Field::A, img);
                        let d = self.eval(&mut p);
                        self.ch.acpt = d;
                        self.ch.aad = p;
                        return;
                    }
                    if self.ch.acloop != 0 {
                        self.ch.acloop -= 1;
                        if self.ch.acloop == 0 {
                            self.ch.ajsr = None;
                            return;
                        }
                    }
                    p = self.ch.aaloop;
                }
            }
            AnimRoutine::StAnim => {
                self.ch.acpt = self.ch.acpt.wrapping_sub(1);
                if self.ch.acpt != 0 {
                    return;
                }
                let mut p = self.ch.aad;
                let mut guard = 0;
                loop {
                    let img = self.w(p) as i16;
                    p += 1;
                    if img < 0 {
                        guard += 1;
                        if img == -1 || guard > 1 {
                            self.ch.ajsr = None;
                            return;
                        }
                        p = self.ch.aaloop;
                        continue;
                    }
                    self.set(Field::A, img);
                    let d = self.w(p) as i16;
                    p += 1;
                    self.ch.acpt = d;
                    if d == 0 {
                        self.ch.ajsr = None;
                        return;
                    }
                    self.ch.aad = p;
                    return;
                }
            }
            AnimRoutine::StMoveX | AnimRoutine::StMoveY => {
                let f = if r == AnimRoutine::StMoveX {
                    Field::X
                } else {
                    Field::Y
                };
                self.ch.acpt = self.ch.acpt.wrapping_sub(1);
                if self.ch.acpt != 0 {
                    return;
                }
                let mut p = self.ch.aad;
                self.ch.acpt = self.w(p) as i16;
                p += 1;
                let v = self.get(f).wrapping_add(self.w(p) as i16);
                p += 1;
                self.set(f, v);
                let ended = if v as i32 == self.ch.delt_y {
                    true
                } else {
                    self.ch.delt_x = self.ch.delt_x.wrapping_sub(1) as i16 as i32;
                    if self.ch.delt_x != 0 {
                        return;
                    }
                    // Next triple.
                    p += 1;
                    if self.w(p) != 0 {
                        self.ch.aad = p;
                        self.ch.delt_x = self.w(p + 2) as i16 as i32;
                        return;
                    }
                    true
                };
                if ended {
                    let h = self.ch.aaloop;
                    if self.w(h + 1) == 0 {
                        self.ch.ajsr = None;
                    } else {
                        self.st_move_init(h, f);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with one object per target and fixed inputs.
    #[derive(Default)]
    struct TestHost {
        obj: HashMap<Target, [i16; 3]>,
        sets: usize,
        mouse: (i16, i16),
    }

    impl AmalHost for TestHost {
        fn amal_get(&mut self, t: Target, f: Field) -> i16 {
            self.obj.entry(t).or_default()[f as usize]
        }
        fn amal_set(&mut self, t: Target, f: Field, v: i16) {
            self.sets += 1;
            self.obj.entry(t).or_default()[f as usize] = v;
        }
        fn amal_mouse(&self) -> (i16, i16) {
            self.mouse
        }
        fn amal_mouse_key(&self, _right: bool) -> bool {
            false
        }
        fn amal_joy(&self, _port: i16) -> i16 {
            0
        }
        fn amal_col(&self, _n: i16) -> i16 {
            0
        }
        fn amal_collide(&mut self, _bob: bool, _n: i16, _s: i16, _e: i16) -> i16 {
            0
        }
        fn amal_conv(&self, c: Conv, _s: i16, v: i16) -> Option<i16> {
            Some(match c {
                Conv::XHard => v + 128,
                Conv::YHard => v + 50,
                Conv::XScreen => v - 128,
                Conv::YScreen => v - 50,
            })
        }
        fn amal_vhpos(&self) -> u16 {
            0
        }
        fn amal_vu(&mut self, _v: i16) -> i16 {
            0
        }
    }

    fn setup(src: &str) -> (AmalState, TestHost) {
        let mut st = AmalState::default();
        st.create(0, ChanKind::Amal, src.as_bytes(), Target::Sprite(0), None)
            .unwrap();
        st.on_off_freeze(None, 0b1111, 1);
        (st, TestHost::default())
    }

    fn xy(h: &mut TestHost) -> (i16, i16) {
        (
            h.amal_get(Target::Sprite(0), Field::X),
            h.amal_get(Target::Sprite(0), Field::Y),
        )
    }

    #[test]
    fn compile_let_and_lexing() {
        // Lower case letters and spaces are decoration.
        let c = compile(b"Let X=XM+10", ChanKind::Amal, false).unwrap();
        assert_eq!(
            c,
            vec![T_LET, T_XM, T_CONST, 10, T_ADD, T_EXPEND, T_SETX, T_END]
        );
        let c = compile(b"L R0=-1", ChanKind::Amal, false).unwrap();
        assert_eq!(
            c,
            vec![T_LET, T_CONST, 0xFFFF, T_EXPEND, T_SETR, 0xFFFE, T_END]
        );
        let c = compile(b"L RA=$10", ChanKind::Amal, false).unwrap();
        assert_eq!(c, vec![T_LET, T_CONST, 16, T_EXPEND, T_SETR, 0, T_END]);
    }

    #[test]
    fn compile_errors() {
        assert_eq!(
            compile(b"Jump Z", ChanKind::Amal, false).unwrap_err().code,
            E_LABEL
        );
        assert_eq!(
            compile(b"N R0", ChanKind::Amal, false).unwrap_err().code,
            E_NEXT
        );
        assert_eq!(
            compile(b"L: L:", ChanKind::Amal, false).unwrap_err().code,
            E_LABEL_DEF
        );
        assert_eq!(
            compile(b"PLay 1", ChanKind::Amal, false).unwrap_err().code,
            E_NO_BANK
        );
        assert_eq!(
            compile(b"Direct A", ChanKind::Amal, false)
                .unwrap_err()
                .code,
            E_AUTO_ONLY
        );
        assert_eq!(
            compile(b"AU(M 1,1,1)", ChanKind::Amal, false)
                .unwrap_err()
                .code,
            E_IN_AUTO
        );
        assert_eq!(
            compile(b"Let X=", ChanKind::Amal, false).unwrap_err().code,
            E_SYNTAX
        );
        assert_eq!(
            compile(b"A: AU(J A)", ChanKind::Amal, false)
                .unwrap_err()
                .code,
            E_JUMP_AUTO
        );
    }

    #[test]
    fn expressions_left_to_right() {
        let (mut st, mut h) = setup("Let R0=2+3*4 ; Let R1=10/0 ; Let R2=3<5 ; Let R3=7<>7");
        st.tick(&mut h);
        let c = &st.channels[0];
        assert_eq!(&c.regs[0..4], &[20, 10, -1, 0]);
    }

    #[test]
    fn move_steps() {
        let (mut st, mut h) = setup("Move 10,-5,5 ; Let X=100");
        let mut pos = Vec::new();
        for _ in 0..6 {
            st.tick(&mut h);
            pos.push(xy(&mut h));
        }
        assert_eq!(pos[0], (2, -1));
        assert_eq!(pos[4], (10, -5));
        // Frame n+1: the next instruction runs.
        assert_eq!(pos[5].0, 100);
    }

    #[test]
    fn for_next_one_loop_per_frame() {
        let (mut st, mut h) = setup("For R0=1 To 3 ; Let X=X+1 ; Next R0 ; Let Y=99");
        st.tick(&mut h);
        assert_eq!(xy(&mut h), (1, 0));
        st.tick(&mut h);
        assert_eq!(xy(&mut h), (2, 0));
        st.tick(&mut h);
        assert_eq!(xy(&mut h), (3, 99));
    }

    #[test]
    fn jump_budget_and_pause() {
        // Infinite loop: 10 jumps per frame in the main program.
        let (mut st, mut h) = setup("A: Let X=X+1 ; Jump A");
        st.tick(&mut h);
        assert_eq!(xy(&mut h).0, 10);
        st.tick(&mut h);
        assert_eq!(xy(&mut h).0, 20);
        let (mut st, mut h) = setup("A: Let X=X+1 ; Pause ; Jump A");
        for _ in 0..5 {
            st.tick(&mut h);
        }
        assert_eq!(xy(&mut h).0, 5);
    }

    #[test]
    fn frozen_until_on() {
        let mut st = AmalState::default();
        let mut h = TestHost::default();
        st.create(3, ChanKind::Amal, b"L X=5", Target::Bob(3), None)
            .unwrap();
        st.tick(&mut h);
        assert_eq!(h.amal_get(Target::Bob(3), Field::X), 0);
        st.on_off_freeze(Some(3), 1, 1);
        st.tick(&mut h);
        assert_eq!(h.amal_get(Target::Bob(3), Field::X), 5);
        assert!(!st.chanmv(3));
    }

    #[test]
    fn anim_and_autotest() {
        let (mut st, mut h) = setup("Anim 0,(1,2)(2,2) ; Wait");
        let mut imgs = Vec::new();
        for _ in 0..5 {
            st.tick(&mut h);
            imgs.push(h.amal_get(Target::Sprite(0), Field::A));
        }
        assert_eq!(imgs, vec![1, 1, 2, 2, 1]);
        // Anim with a loop count.
        let (mut st, mut h) = setup("A 1,(3,1)(4,1)");
        for _ in 0..4 {
            st.tick(&mut h);
        }
        assert_eq!(h.amal_get(Target::Sprite(0), Field::A), 4);
        assert!(!st.chanan(0));
        let (mut st, mut h) = setup("AU( I XM>100 D A ) ; Wait ; A: L Y=1 ; Wait");
        st.tick(&mut h);
        assert_eq!(xy(&mut h).1, 0);
        h.mouse = (150, 0);
        st.tick(&mut h);
        assert_eq!(xy(&mut h).1, 1);
    }

    #[test]
    fn stos_anim_and_move() {
        let mut st = AmalState::default();
        let mut h = TestHost::default();
        st.create(1, ChanKind::Anim, b"(5,1)(6,1)L", Target::Sprite(1), None)
            .unwrap();
        st.create(1, ChanKind::MoveX, b"10(1,2,3)", Target::Sprite(1), None)
            .unwrap();
        st.on_off_freeze(None, 0b1111, 1);
        let mut out = Vec::new();
        for _ in 0..5 {
            st.tick(&mut h);
            out.push((
                h.amal_get(Target::Sprite(1), Field::A),
                h.amal_get(Target::Sprite(1), Field::X),
            ));
        }
        assert_eq!(out, vec![(5, 12), (6, 14), (5, 16), (6, 16), (5, 16)]);
        assert!(st.chanan(1));
        assert!(!st.movon(1));
    }

    #[test]
    fn play_movement() {
        // Bank: offset long, movement count 1, entry, header, X/Y data.
        let mut b = vec![0u8, 0, 0, 0, 0, 1, 0, 2];
        // H = 4 + 2*2 = 8: speed 1, yoff 4, then X stream at H+5.
        b.extend_from_slice(&[0, 1, 0, 4]);
        // H+4: sentinel 0, H+5: X data (2, 3, end); Y stream at H+5 too.
        b.extend_from_slice(&[0, 2, 3, 0]);
        let mut st = AmalState::default();
        let mut h = TestHost::default();
        st.create(
            0,
            ChanKind::Amal,
            b"PLay 1 ; L A=7",
            Target::Sprite(0),
            Some(Rc::new(b)),
        )
        .unwrap();
        st.on_off_freeze(None, 1, 1);
        for _ in 0..3 {
            st.tick(&mut h);
        }
        assert_eq!(xy(&mut h), (5, 5));
        assert_eq!(h.amal_get(Target::Sprite(0), Field::A), 7);
    }

    #[test]
    fn random_and_conversions() {
        let (mut st, mut h) = setup("L R0=Z(255) ; L R1=XH(0,10) ; L R2=YS(0,60)");
        st.tick(&mut h);
        let r = st.channels[0].regs;
        assert!((0..=255).contains(&r[0]));
        assert_eq!(r[1], 138);
        assert_eq!(r[2], 10);
    }
}
