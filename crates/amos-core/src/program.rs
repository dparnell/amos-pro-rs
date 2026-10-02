//! Tokenised AMOS programs: `.AMOS` file loading/saving and line access.
//!
//! File layout (`Prg_Load` / `Prg_Save` in `+Verif.s`): a 16 byte header
//! ("AMOS Basic..." or "AMOS Pro..."), the length of the tokenised source, the
//! tokenised lines, then the banks (`"AmBs"` + records).

use crate::banks::{self, Bank};
use crate::error::{AmosError, Result};
use crate::tokens::TK_PROCEDURE;

/// Header written by AMOS Pro when saving an untested program.
pub const HEADER_PRO: &[u8; 16] = b"AMOS Pro101v\0\0\0\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramKind {
    Amos13,
    AmosPro,
}

/// A tokenised program with its banks.
#[derive(Clone, Debug)]
pub struct Program {
    pub kind: ProgramKind,
    pub header: [u8; 16],
    /// Math flags (Set Double Precision etc.) from byte 15 of a Pro header.
    pub math_flags: u8,
    /// Tokenised lines, without the trailing zero length byte.
    pub source: Vec<u8>,
    pub banks: Vec<Bank>,
}

impl Default for Program {
    fn default() -> Self {
        Self { kind: ProgramKind::AmosPro, header: *HEADER_PRO, math_flags: 0, source: Vec::new(), banks: Vec::new() }
    }
}

/// Flags byte of a Procedure line.
pub mod proc_flags {
    pub const FOLDED: u8 = 0x80;
    pub const LOCKED: u8 = 0x40;
    pub const ENCRYPTED: u8 = 0x20;
    pub const MACHINE_CODE: u8 = 0x10;
}

impl Program {
    pub fn load(data: &[u8]) -> Result<Self> {
        if data.len() < 20 {
            return Err(AmosError::BadFormat);
        }
        let kind = if data.starts_with(b"AMOS Basic") {
            ProgramKind::Amos13
        } else if data.starts_with(b"AMOS Pro") {
            ProgramKind::AmosPro
        } else {
            return Err(AmosError::BadFormat);
        };
        let mut header = [0u8; 16];
        header.copy_from_slice(&data[..16]);
        let math_flags = if kind == ProgramKind::AmosPro { header[15] } else { 0 };
        let len = u32::from_be_bytes([data[16], data[17], data[18], data[19]]) as usize;
        let end = 20usize.checked_add(len).filter(|&e| e <= data.len()).ok_or(AmosError::BadFormat)?;
        let mut source = data[20..end].to_vec();
        // Stop at an explicit end-of-program marker if one was saved.
        let mut p = 0;
        while p < source.len() {
            let n = source[p] as usize * 2;
            if n == 0 {
                source.truncate(p);
                break;
            }
            p += n;
        }
        let banks = banks::parse_banks(&data[end..]).unwrap_or_else(|e| {
            log::warn!("Could not read program banks: {e}");
            Vec::new()
        });
        let mut prg = Self { kind, header, math_flags, source, banks };
        prg.decrypt_procedures();
        Ok(prg)
    }

    pub fn save(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.source.len() + 32);
        let mut header = *HEADER_PRO;
        header[15] = self.math_flags;
        out.extend_from_slice(&header);
        out.extend_from_slice(&(self.source.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.source);
        out.extend_from_slice(&banks::save_banks(&self.banks));
        out
    }

    /// Iterates over the lines: `(offset in source, line bytes)`.
    /// Machine code procedure bodies are skipped.
    pub fn lines(&self) -> Lines<'_> {
        Lines { src: &self.source, pos: 0 }
    }

    /// Decodes procedures that were saved locked and encrypted (`ProCode`,
    /// `+Verif.s`), so they can be listed and run.
    fn decrypt_procedures(&mut self) {
        let mut p = 0;
        while p + 12 <= self.source.len() {
            let n = self.source[p] as usize * 2;
            if n == 0 {
                break;
            }
            if read_u16(&self.source, p + 2) == TK_PROCEDURE {
                let flags = self.source[p + 10];
                if flags & proc_flags::MACHINE_CODE != 0 {
                    p = p + 8 + read_u32(&self.source, p + 4) as usize;
                    continue;
                }
                if flags & proc_flags::LOCKED != 0 && flags & proc_flags::ENCRYPTED != 0 {
                    procode(&mut self.source, p);
                }
            }
            p += n;
        }
    }
}

pub struct Lines<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let p = self.pos;
        let n = *self.src.get(p)? as usize * 2;
        if n == 0 || p + n > self.src.len() {
            return None;
        }
        let line = &self.src[p..p + n];
        self.pos += n;
        if n >= 12 && read_u16(line, 2) == TK_PROCEDURE && line[10] & proc_flags::MACHINE_CODE != 0 {
            self.pos = p + 8 + read_u32(line, 4) as usize;
        }
        Some((p, line))
    }
}

/// Encrypted procedure decoder (+Verif.s `ProCode`). `l` is the offset of
/// the Procedure line; all lines up to End Proc are XORed in place.
fn procode(src: &mut [u8], l: usize) {
    let a6 = l + 2;
    let size = read_u32(src, a6 + 2);
    let a2 = a6 + 16 + size as usize;
    let mut d5 = size.rotate_left(8);
    d5 = (d5 & 0xFFFF_FF00) | src[a6 + 9] as u32;
    let mut d4: u16 = 1;
    let d3 = read_u16(src, a6 + 6);
    let mut a1 = l + src[l] as usize * 2;
    loop {
        if a1 >= src.len() {
            break;
        }
        let mut a0 = a1;
        a1 = a0 + src[a0] as usize * 2;
        a0 += 4;
        if a0 == a2 || a1 > src.len() || src[a0 - 4] == 0 {
            break;
        }
        while a0 < a1 {
            let w = read_u16(src, a0) ^ (d5 as u16);
            src[a0..a0 + 2].copy_from_slice(&w.to_be_bytes());
            a0 += 2;
            d5 = (d5 & 0xFFFF_0000) | ((d5 as u16).wrapping_add(d4) as u32);
            d4 = d4.wrapping_add(d3);
            d5 = d5.rotate_right(1);
        }
    }
    src[a6 + 8] ^= proc_flags::ENCRYPTED;
}

pub fn read_u16(b: &[u8], p: usize) -> u16 {
    u16::from_be_bytes([b[p], b[p + 1]])
}

pub fn read_u32(b: &[u8], p: usize) -> u32 {
    u32::from_be_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]])
}

/// A variable-like record following `TK_VAR`, `TK_LAB`, `TK_PRO` or `TK_LGO`.
#[derive(Clone, Debug)]
pub struct VarRecord<'a> {
    /// Offset stored by the verifier (0 in untested programs).
    pub offset: u16,
    pub flags: u8,
    /// Name bytes (lower case), without padding.
    pub name: &'a [u8],
    /// Total size of the record after the token word.
    pub size: usize,
}

pub fn read_var(line: &[u8], p: usize) -> Option<VarRecord<'_>> {
    if p + 4 > line.len() {
        return None;
    }
    let len = line[p + 2] as usize;
    let flags = line[p + 3];
    let raw = line.get(p + 4..p + 4 + len)?;
    let name_len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    Some(VarRecord { offset: read_u16(line, p), flags, name: &raw[..name_len], size: 4 + len })
}

/// Variable flag bits.
pub mod var_flags {
    pub const TYPE_MASK: u8 = 0x03;
    pub const FLOAT: u8 = 0x01;
    pub const STRING: u8 = 0x02;
    pub const DEF_FN: u8 = 0x08;
    pub const ARRAY: u8 = 0x40;
    pub const PROC: u8 = 0x80;
}
