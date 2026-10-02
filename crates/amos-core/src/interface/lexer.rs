//! Character reader of the Interface language (`Dia_Chr` `+Lib.s:24660`)
//! and the function table (`bin/+Dialog_Funcs.bin`).
//!
//! `Dia_Chr` returns the next significant byte with a class taken from a
//! 128 entry table. Bytes >= 128 and entries with bit 7 set are skipped:
//! spaces, control characters, `( ) . { } ~ DEL`, every lower case letter
//! (so "BUtton" reads as "BU") and `|` (which makes the documented `|`
//! operator unreachable: kept, as in the original).

/// Class of a significant character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// NUL: end of the program (Z flag).
    End,
    /// A-Z (C flag).
    Letter,
    /// `! " # & ' * + - / < = > ? @ [ \ ] ^ _` (C+V flags).
    Op,
    /// 0-9, `$`, `%` (N+C flags).
    Num,
    /// `, : ; `` ` (no flag).
    Sep,
}

/// `Dia_Chr` table: None = skipped.
fn class_of(c: u8) -> Option<Class> {
    Some(match c {
        0 => Class::End,
        b'!' | b'"' | b'#' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'/' => Class::Op,
        b'<' | b'=' | b'>' | b'?' | b'@' | b'[' | b'\\' | b']' | b'^' | b'_' => Class::Op,
        b'$' | b'%' | b'0'..=b'9' => Class::Num,
        b',' | b':' | b';' | b'`' => Class::Sep,
        b'A'..=b'Z' => Class::Letter,
        _ => return None,
    })
}

/// Reads the next significant character from `prg` at `*pc` (advancing
/// past it). Past the end of the program, NUL is returned.
pub fn chr(prg: &[u8], pc: &mut usize) -> (u8, Class) {
    loop {
        let c = prg.get(*pc).copied().unwrap_or(0);
        *pc += 1;
        if c >= 128 {
            continue;
        }
        if let Some(cl) = class_of(c) {
            return (c, cl);
        }
    }
}

/// `Dia_ChrC`: next raw byte minus '0' (as a signed byte).
fn chr_c(prg: &[u8], pc: &mut usize) -> i32 {
    let c = prg.get(*pc).copied().unwrap_or(0);
    *pc += 1;
    c.wrapping_sub(b'0') as i8 as i32
}

/// `Dia_FChif` (`+Lib.s:24805`): reads a number whose first character `c`
/// has already been read: decimal, `$hex` (upper case digits) or `%binary`.
/// Stops at the first other byte, which is not consumed.
pub fn number(prg: &[u8], pc: &mut usize, c: u8) -> i32 {
    let mut d1: i32 = 0;
    match c {
        b'%' => loop {
            let d = chr_c(prg, pc);
            if !(0..2).contains(&d) {
                break;
            }
            d1 = (d1 << 1) | d;
        },
        b'$' => loop {
            let mut d = chr_c(prg, pc);
            if !(0..10).contains(&d) {
                // "add.b #'0'-'A'": A-F give 0..5.
                d = (d as u8).wrapping_add(b'0'.wrapping_sub(b'A')) as i8 as i32;
                if !(0..6).contains(&d) {
                    break;
                }
                d += 10;
            }
            d1 = (d1 << 4) | d;
        },
        _ => {
            // The first digit is taken as a byte: '-' or '+' give garbage,
            // as in the original (seen when reading a Digit zone).
            d1 = c.wrapping_sub(b'0') as i32;
            loop {
                let d = chr_c(prg, pc);
                if !(0..10).contains(&d) {
                    break;
                }
                d1 = d1.wrapping_mul(10).wrapping_add(d);
            }
        }
    }
    *pc -= 1;
    d1
}

/// The function table of the Interface expressions.
static FUNCS: &[u8] = include_bytes!("../../../../AMOS-Professional-365/bin/+Dialog_Funcs.bin");

/// Function number of an operator (`c1` = the operator), or of a two
/// letter function (`c1` letter, `c2` second character). None if the
/// table has no such entry (syntax error).
pub fn function(c1: u8, c2: u8, op: bool) -> Option<u8> {
    let (start, key) = if op {
        (32usize, c1)
    } else {
        let i = c1.checked_sub(64)? as usize;
        let o = *FUNCS.get(i)? as usize;
        if o == 0 {
            return None;
        }
        (32 + o, c2)
    };
    let mut p = start;
    loop {
        let (hi, lo) = (*FUNCS.get(p)?, *FUNCS.get(p + 1)?);
        if hi == 0 && lo == 0 {
            return None;
        }
        if lo == key {
            return Some(hi >> 2);
        }
        p += 2;
    }
}

/// Function numbers (`Dia_FTokens` comments, `+Lib.s:21161`).
pub mod f {
    pub const BX: u8 = 0;
    pub const BY: u8 = 1;
    pub const SX: u8 = 2;
    pub const SY: u8 = 3;
    pub const PLUS: u8 = 4;
    pub const MINUS: u8 = 5;
    pub const MUL: u8 = 6;
    pub const DIV: u8 = 7;
    pub const NE: u8 = 8;
    pub const ME: u8 = 9;
    pub const ME2: u8 = 10;
    pub const TW: u8 = 11;
    pub const TH: u8 = 12;
    pub const VA: u8 = 13;
    pub const STR1: u8 = 14;
    pub const STR2: u8 = 15;
    pub const CX: u8 = 16;
    pub const SW: u8 = 17;
    pub const SH: u8 = 18;
    pub const MI: u8 = 19;
    pub const MA: u8 = 20;
    pub const BP: u8 = 21;
    pub const DEC: u8 = 22;
    pub const CONCAT: u8 = 23;
    pub const MZ: u8 = 24;
    pub const XA: u8 = 25;
    pub const YA: u8 = 26;
    pub const XB: u8 = 27;
    pub const YB: u8 = 28;
    pub const AR: u8 = 29;
    pub const ZP: u8 = 30;
    pub const P1: u8 = 31;
    pub const P9: u8 = 39;
    pub const EQ: u8 = 40;
    pub const NE_OP: u8 = 41;
    pub const LT: u8 = 42;
    pub const GT: u8 = 43;
    pub const AND: u8 = 44;
    pub const OR: u8 = 45;
    pub const ZV: u8 = 46;
    pub const ZN: u8 = 47;
    pub const TL: u8 = 48;
    pub const AS: u8 = 49;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_significant_chars() {
        let p = b"SIze  SW,SH;";
        let mut pc = 0;
        let mut s = Vec::new();
        loop {
            let (c, cl) = chr(p, &mut pc);
            if cl == Class::End {
                break;
            }
            s.push(c);
        }
        assert_eq!(s, b"SISW,SH;");
    }

    #[test]
    fn numbers() {
        let mut pc = 1;
        assert_eq!(number(b"1234,", &mut pc, b'1'), 1234);
        assert_eq!(pc, 4);
        let mut pc = 1;
        assert_eq!(number(b"$C5,", &mut pc, b'$'), 0xC5);
        let mut pc = 1;
        assert_eq!(number(b"%1011;", &mut pc, b'%'), 11);
    }

    #[test]
    fn function_table() {
        assert_eq!(function(b'+', 0, true), Some(f::PLUS));
        assert_eq!(function(b'|', 0, true), Some(f::OR));
        assert_eq!(function(b'V', b'A', false), Some(f::VA));
        assert_eq!(function(b'M', b'E', false), Some(f::ME));
        assert_eq!(function(b'P', b'9', false), Some(f::P9));
        assert_eq!(function(b'Z', b'N', false), Some(f::ZN));
        assert_eq!(function(b'D', b'X', false), None);
        assert_eq!(function(b'B', b'Q', false), None);
    }
}
