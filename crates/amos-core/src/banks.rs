//! AMOS memory banks and their file formats (`AmBk`, `AmSp`, `AmIc`, `AmBs`).
//!
//! See `Bnk.SaveA0` / `Bnk.Load` in `+Lib.s`.

use crate::error::{AmosError, Result};

/// One image of a sprite or icon bank, stored planar as on the Amiga.
#[derive(Clone, Debug, Default)]
pub struct Image {
    /// Width in 16 pixel words.
    pub width_words: u16,
    pub height: u16,
    pub planes: u16,
    pub hot_x: i16,
    pub hot_y: i16,
    /// Big-endian planar data: `planes` consecutive bitplanes of
    /// `height` rows of `width_words` words.
    pub planar: Vec<u8>,
}

impl Image {
    pub fn width(&self) -> u32 {
        self.width_words as u32 * 16
    }

    pub fn is_empty(&self) -> bool {
        self.width_words == 0 || self.height == 0 || self.planes == 0
    }

    /// Colour index of a pixel.
    pub fn pixel(&self, x: u32, y: u32) -> u8 {
        let row_bytes = self.width_words as usize * 2;
        let plane_bytes = row_bytes * self.height as usize;
        let byte = y as usize * row_bytes + (x / 8) as usize;
        let bit = 7 - (x % 8);
        let mut c = 0u8;
        for p in 0..self.planes as usize {
            if let Some(b) = self.planar.get(p * plane_bytes + byte)
                && (b >> bit) & 1 != 0
            {
                c |= 1 << p;
            }
        }
        c
    }

    /// Converts to one byte per pixel.
    pub fn to_chunky(&self) -> Vec<u8> {
        let w = self.width();
        let mut out = Vec::with_capacity((w * self.height as u32) as usize);
        for y in 0..self.height as u32 {
            for x in 0..w {
                out.push(self.pixel(x, y));
            }
        }
        out
    }
}

/// Contents of a bank.
#[derive(Clone, Debug)]
pub enum BankData {
    /// Raw data bank ("Music", "Samples", "Pac.Pic.", "Resource", "Work"...).
    Raw(Vec<u8>),
    /// Sprite (bank 1) or icon (bank 2) images with their palette.
    Images { images: Vec<Image>, palette: [u16; 32] },
}

#[derive(Clone, Debug)]
pub struct Bank {
    pub number: u16,
    /// Eight character bank name as stored in the file ("Sprites ", "Music   ").
    pub name: String,
    pub chip: bool,
    /// Data bank (saved with the program) rather than a work bank.
    pub data_bank: bool,
    pub data: BankData,
}

impl Bank {
    pub fn is_icons(&self) -> bool {
        self.name.starts_with("Icons")
    }

    pub fn raw(&self) -> Option<&[u8]> {
        match &self.data {
            BankData::Raw(d) => Some(d),
            _ => None,
        }
    }

    pub fn length(&self) -> usize {
        match &self.data {
            BankData::Raw(d) => d.len(),
            BankData::Images { images, .. } => images.len(),
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(AmosError::BadFormat)?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Parses the banks following a program (`"AmBs"` + count + records), or the
/// content of an `.Abk` file (a single bank record, or an `AmBs` set).
pub fn parse_banks(data: &[u8]) -> Result<Vec<Bank>> {
    let mut r = Reader { data, pos: 0 };
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let count = if data.starts_with(b"AmBs") {
        r.pos = 4;
        r.u16()? as usize
    } else {
        1
    };
    let mut banks = Vec::with_capacity(count);
    for _ in 0..count {
        banks.push(parse_bank(&mut r)?);
    }
    Ok(banks)
}

fn parse_bank(r: &mut Reader) -> Result<Bank> {
    let tag = r.bytes(4)?;
    match tag {
        b"AmBk" => {
            let number = r.u16()?;
            let fast = r.u16()?;
            let len = r.u32()?;
            let size = (len & 0x0FFF_FFFF) as usize;
            let name_bytes = r.bytes(8)?;
            let name: String = name_bytes.iter().map(|&b| b as char).collect();
            let body = r.bytes(size.saturating_sub(8))?.to_vec();
            Ok(Bank {
                number,
                name,
                chip: fast == 0,
                data_bank: len & 0x8000_0000 != 0,
                data: BankData::Raw(body),
            })
        }
        b"AmSp" | b"AmIc" => {
            let icons = tag == b"AmIc";
            let count = r.u16()? as usize;
            let mut images = Vec::with_capacity(count);
            for _ in 0..count {
                let width_words = r.u16()?;
                let height = r.u16()?;
                let planes = r.u16()?;
                let hot_x = (r.u16()? & 0x3FFF) as i16;
                let hot_y = r.u16()? as i16;
                let n = width_words as usize * height as usize * planes as usize * 2;
                let planar = r.bytes(n)?.to_vec();
                images.push(Image { width_words, height, planes, hot_x, hot_y, planar });
            }
            let mut palette = [0u16; 32];
            for c in palette.iter_mut() {
                *c = r.u16()?;
            }
            Ok(Bank {
                number: if icons { 2 } else { 1 },
                name: if icons { "Icons   ".into() } else { "Sprites ".into() },
                chip: true,
                data_bank: true,
                data: BankData::Images { images, palette },
            })
        }
        _ => Err(AmosError::BadFormat),
    }
}

/// Serialises banks as an `AmBs` set (as appended to a saved program).
pub fn save_banks(banks: &[Bank]) -> Vec<u8> {
    let mut out = b"AmBs".to_vec();
    out.extend_from_slice(&(banks.len() as u16).to_be_bytes());
    for b in banks {
        save_bank(b, &mut out);
    }
    out
}

/// Serialises one bank record (the format of a single bank `.Abk` file).
pub fn save_bank(bank: &Bank, out: &mut Vec<u8>) {
    match &bank.data {
        BankData::Raw(data) => {
            out.extend_from_slice(b"AmBk");
            out.extend_from_slice(&bank.number.to_be_bytes());
            out.extend_from_slice(&(if bank.chip { 0u16 } else { 1 }).to_be_bytes());
            let mut len = (data.len() + 8) as u32;
            if bank.data_bank {
                len |= 0x8000_0000;
            }
            out.extend_from_slice(&len.to_be_bytes());
            let mut name = [b' '; 8];
            for (d, s) in name.iter_mut().zip(bank.name.bytes()) {
                *d = s;
            }
            out.extend_from_slice(&name);
            out.extend_from_slice(data);
        }
        BankData::Images { images, palette } => {
            out.extend_from_slice(if bank.is_icons() { b"AmIc" } else { b"AmSp" });
            out.extend_from_slice(&(images.len() as u16).to_be_bytes());
            for i in images {
                for v in [i.width_words, i.height, i.planes, i.hot_x as u16, i.hot_y as u16] {
                    out.extend_from_slice(&v.to_be_bytes());
                }
                out.extend_from_slice(&i.planar);
            }
            for c in palette {
                out.extend_from_slice(&c.to_be_bytes());
            }
        }
    }
}

/// Base of the virtual addresses given to banks (`Start(n)`).
pub const BANK_ADDRESS_BASE: u32 = 0x0010_0000;
const PAGE: u32 = 4096;

/// The banks of the running program, and the virtual memory used by
/// `Start`, `Peek`, `Poke`, `Copy`...
///
/// Each bank's data is given a stable virtual address range. Accesses
/// outside every bank go to a sparse "free memory" area so programs that
/// poke at addresses they computed themselves keep working.
#[derive(Debug, Default)]
pub struct BankSet {
    pub banks: std::collections::BTreeMap<u16, Bank>,
    addresses: std::collections::BTreeMap<u16, u32>,
    next_address: u32,
    free_memory: std::collections::HashMap<u32, Box<[u8; PAGE as usize]>>,
    /// Incremented whenever a bank is added, removed or renumbered.
    pub generation: u64,
}

impl BankSet {
    /// Installs the banks saved with a program.
    pub fn load_program_banks(&mut self, banks: &[Bank]) {
        self.erase_all();
        for b in banks {
            self.insert(b.clone());
        }
    }

    pub fn get(&self, n: u16) -> Option<&Bank> {
        self.banks.get(&n)
    }

    pub fn get_mut(&mut self, n: u16) -> Option<&mut Bank> {
        self.banks.get_mut(&n)
    }

    /// Adds (or replaces) a bank and gives it an address.
    pub fn insert(&mut self, bank: Bank) {
        let n = bank.number;
        self.erase(n);
        let len = match &bank.data {
            BankData::Raw(d) => d.len() as u32,
            BankData::Images { images, .. } => 26 + images.len() as u32 * 8 + 64,
        };
        if self.next_address == 0 {
            self.next_address = BANK_ADDRESS_BASE;
        }
        let addr = self.next_address;
        // Keep a gap after each bank so out of range accesses do not land in
        // the next one.
        self.next_address = (addr + len + PAGE * 16).div_ceil(PAGE) * PAGE;
        self.addresses.insert(n, addr);
        self.banks.insert(n, bank);
        self.generation += 1;
    }

    /// `Reserve As ...`: a cleared raw bank.
    pub fn reserve(&mut self, n: u16, len: usize, name: &str, data_bank: bool, chip: bool) {
        self.insert(Bank { number: n, name: format!("{name:<8}"), chip, data_bank, data: BankData::Raw(vec![0; len]) });
    }

    pub fn erase(&mut self, n: u16) -> bool {
        self.addresses.remove(&n);
        let removed = self.banks.remove(&n).is_some();
        if removed {
            self.generation += 1;
        }
        removed
    }

    pub fn erase_all(&mut self) {
        self.banks.clear();
        self.addresses.clear();
        self.generation += 1;
    }

    /// `Erase Temp`: erases the work banks.
    pub fn erase_temp(&mut self) {
        let temp: Vec<u16> = self.banks.values().filter(|b| !b.data_bank).map(|b| b.number).collect();
        for n in temp {
            self.erase(n);
        }
    }

    /// `Bank Swap a,b`.
    pub fn swap(&mut self, a: u16, b: u16) {
        let ba = self.banks.remove(&a);
        let bb = self.banks.remove(&b);
        let aa = self.addresses.remove(&a);
        let ab = self.addresses.remove(&b);
        if let Some(mut x) = ba {
            x.number = b;
            self.banks.insert(b, x);
            self.addresses.insert(b, aa.unwrap_or(0));
        }
        if let Some(mut y) = bb {
            y.number = a;
            self.banks.insert(a, y);
            self.addresses.insert(a, ab.unwrap_or(0));
        }
        self.generation += 1;
    }

    /// Address of the data of bank `n` (`Start`).
    pub fn start(&self, n: u16) -> Option<u32> {
        self.addresses.get(&n).copied()
    }

    /// `Length(n)`: data length, or number of images for sprite/icon banks.
    pub fn length(&self, n: u16) -> Option<usize> {
        self.banks.get(&n).map(|b| b.length())
    }

    /// Finds the bank holding `addr`: (bank number, offset).
    fn locate(&self, addr: u32) -> Option<(u16, usize)> {
        for (&n, &base) in &self.addresses {
            if let Some(BankData::Raw(d)) = self.banks.get(&n).map(|b| &b.data)
                && addr >= base
                && ((addr - base) as usize) < d.len()
            {
                return Some((n, (addr - base) as usize));
            }
        }
        None
    }

    /// Converts "bank number or address" parameters (`Bnk.OrAdr`): values
    /// below 1024 are bank numbers.
    pub fn bank_or_address(&self, v: i32) -> Option<u32> {
        if (0..1024).contains(&v) { self.start(v as u16) } else { Some(v as u32) }
    }

    pub fn peek(&self, addr: u32) -> u8 {
        if let Some((n, off)) = self.locate(addr)
            && let Some(BankData::Raw(d)) = self.banks.get(&n).map(|b| &b.data)
        {
            return d[off];
        }
        self.free_memory.get(&(addr / PAGE)).map_or(0, |p| p[(addr % PAGE) as usize])
    }

    pub fn poke(&mut self, addr: u32, v: u8) {
        if let Some((n, off)) = self.locate(addr)
            && let Some(BankData::Raw(d)) = self.banks.get_mut(&n).map(|b| &mut b.data)
        {
            d[off] = v;
            return;
        }
        let page = self.free_memory.entry(addr / PAGE).or_insert_with(|| Box::new([0; PAGE as usize]));
        page[(addr % PAGE) as usize] = v;
    }

    pub fn peek_bytes(&self, addr: u32, len: usize) -> Vec<u8> {
        (0..len as u32).map(|i| self.peek(addr.wrapping_add(i))).collect()
    }

    pub fn poke_bytes(&mut self, addr: u32, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            self.poke(addr.wrapping_add(i as u32), b);
        }
    }

    /// Data of a raw bank.
    pub fn raw(&self, n: u16) -> Option<&[u8]> {
        self.banks.get(&n).and_then(|b| b.raw())
    }

    pub fn raw_mut(&mut self, n: u16) -> Option<&mut Vec<u8>> {
        match self.banks.get_mut(&n).map(|b| &mut b.data) {
            Some(BankData::Raw(d)) => Some(d),
            _ => None,
        }
    }

    /// Text of `Listbank`.
    pub fn listing(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (n, b) in &self.banks {
            let pad = if *n < 10 { " " } else { "" };
            let line = format!(
                "{pad}{n} - {:<8} S: ${:08X} L: {}\r\n",
                b.name.chars().take(8).collect::<String>(),
                self.start(*n).unwrap_or(0),
                b.length()
            );
            out.extend(line.bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_peek_poke() {
        let mut b = BankSet::default();
        b.reserve(10, 100, "Work", false, false);
        let a = b.start(10).unwrap();
        b.poke(a + 5, 42);
        assert_eq!(b.raw(10).unwrap()[5], 42);
        assert_eq!(b.peek(a + 5), 42);
        // Outside banks: free memory.
        b.poke(0x50, 7);
        assert_eq!(b.peek(0x50), 7);
        b.swap(10, 11);
        assert_eq!(b.peek(b.start(11).unwrap() + 5), 42);
        assert!(b.get(10).is_none());
    }
}
