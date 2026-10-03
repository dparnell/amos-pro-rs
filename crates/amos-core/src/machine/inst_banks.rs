//! Memory banks and memory access (`+Lib.s` Bnk.*, Peek/Poke...).

use super::Hardware;
use crate::banks::{self, Bank, BankData};
use crate::errors;
use crate::interp::value::Value;
use crate::interp::{Exc, Interp, R, VarLoc, err};

/// A variable given an address by Varptr / Array.
#[derive(Clone, Debug)]
pub struct VarMap {
    /// Address returned to the program.
    pub addr: u32,
    /// Start of the memory image (the length word for strings).
    pub base: u32,
    pub cap: u32,
    pub loc: VarLoc,
    pub ty: u8,
    pub array: bool,
}

/// Memory image of a variable: big-endian value, or length word + chars
/// for strings, or the elements of an array.
fn var_bytes(it: &mut Interp, loc: &VarLoc, ty: u8, array: bool) -> Vec<u8> {
    use crate::interp::value::ArrayData;
    if array {
        return match it.array_mut(loc) {
            Ok(a) => match &a.data {
                ArrayData::Int(v) => v.iter().flat_map(|x| x.to_be_bytes()).collect(),
                ArrayData::Float(v) => v.iter().flat_map(|x| crate::ffp::Ffp::from_f64(*x).0.to_be_bytes()).collect(),
                ArrayData::Str(v) => v.iter().flat_map(|_| 0u32.to_be_bytes()).collect(),
            },
            Err(_) => Vec::new(),
        };
    }
    match it.read_loc(loc, ty) {
        Value::Int(i) => i.to_be_bytes().to_vec(),
        Value::Float(f) if it.double => f.to_be_bytes().to_vec(),
        Value::Float(f) => crate::ffp::Ffp::from_f64(f).0.to_be_bytes().to_vec(),
        Value::Str(s) => {
            let mut v = (s.len() as u16).to_be_bytes().to_vec();
            v.extend_from_slice(&s);
            v
        }
    }
}
use crate::tokens::{Keyword, TK_COMMA, TK_EOL, TK_VAR, tk};

impl Hardware {
    pub(crate) fn banks_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        match kw.token {
            RESERVE_AS_WORK | RESERVE_AS_CHIP_WORK | RESERVE_AS_DATA | RESERVE_AS_CHIP_DATA => {
                let a = it.inst_args(self, kw)?;
                let (n, len) = (a.int(0), a.int(1));
                if !(1..65536).contains(&n) || len <= 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                if len > 16 * 1024 * 1024 {
                    return err(errors::OUT_OF_MEMORY);
                }
                let data = matches!(kw.token, RESERVE_AS_DATA | RESERVE_AS_CHIP_DATA);
                let chip = matches!(kw.token, RESERVE_AS_CHIP_WORK | RESERVE_AS_CHIP_DATA);
                self.banks.reserve(n as u16, len as usize, if data { "Data" } else { "Work" }, data, chip);
                self.on_banks_changed();
            }
            ERASE => {
                let n = it.inst_args(self, kw)?.int(0);
                if (1..65536).contains(&n) && self.banks.erase(n as u16) {
                    self.on_banks_changed();
                }
            }
            ERASE_TEMP => {
                self.banks.erase_temp();
                self.on_banks_changed();
            }
            ERASE_ALL => {
                self.banks.erase_all();
                self.on_banks_changed();
            }
            BANK_SWAP => {
                let a = it.inst_args(self, kw)?;
                let (x, y) = (a.int(0), a.int(1));
                if !(1..65536).contains(&x) || !(1..65536).contains(&y) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.banks.swap(x as u16, y as u16);
                self.on_banks_changed();
            }
            BANK_SHRINK => {
                let a = it.inst_args(self, kw)?;
                let (n, len) = (a.int(0), a.int(1));
                let Some(bank) = self.banks.get_mut(n.clamp(0, 65535) as u16) else {
                    return err(errors::BANK_NOT_RESERVED);
                };
                match &mut bank.data {
                    BankData::Raw(d) if len >= 0 && len as usize <= d.len() => d.truncate(len as usize),
                    _ => return err(errors::ILLEGAL_FUNCTION_CALL),
                }
            }
            LIST_BANK => {
                let text = self.banks.listing();
                crate::interp::Host::print(self, it, &text)?;
            }
            POKE | DOKE | LOKE => {
                let a = it.inst_args(self, kw)?;
                let (addr, v) = (a.int(0), a.int(1));
                match kw.token {
                    POKE => self.poke(it, addr, v)?,
                    DOKE => self.doke(it, addr, v)?,
                    _ => self.loke(it, addr, v)?,
                }
            }
            POKE_S => {
                let a = it.inst_args(self, kw)?;
                let (addr, data) = (a.int(0) as u32, a.str(1));
                self.refresh_var_maps(it, addr, data.len());
                self.mem_write(addr, &data);
                self.write_back_var_maps(it, addr, data.len())?;
            }
            COPY => {
                let a = it.inst_args(self, kw)?;
                let (s, e, d) = (a.int(0) as u32, a.int(1) as u32, a.int(2) as u32);
                if e > s {
                    self.refresh_var_maps(it, s, (e - s) as usize);
                    let data = self.banks.peek_bytes(s, (e - s) as usize);
                    self.mem_write(d, &data);
                    self.write_back_var_maps(it, d, (e - s) as usize)?;
                }
            }
            FILL => {
                // Fill start To end,value (long pattern)
                let a = it.inst_args(self, kw)?;
                let (s, e, v) = (a.int(0) as u32, a.int(1) as u32, a.int(2) as u32);
                let pat = v.to_be_bytes();
                let mut addr = s;
                while addr < e {
                    self.banks.poke(addr, pat[((addr - s) & 3) as usize]);
                    addr += 1;
                }
            }
            BSET | BCLR | BCHG | ROR_B | ROR_W | ROR_L | ROL_B | ROL_W | ROL_L => self.bit_op(it, kw)?,
            LOAD | LOAD_2 => {
                let a = it.inst_args(self, kw)?;
                let data = self.files_read_all(&a.str(0))?;
                self.load_banks(&data, a.opt(1))?;
            }
            SAVE | SAVE_2 => {
                let a = it.inst_args(self, kw)?;
                let data = match a.opt(1) {
                    Some(n) => {
                        let bank = self.banks.get(n.clamp(0, 65535) as u16).ok_or(Exc::Error(errors::BANK_NOT_RESERVED))?;
                        let mut out = Vec::new();
                        banks::save_bank(bank, &mut out);
                        out
                    }
                    None => {
                        let all: Vec<Bank> = self.banks.banks.values().cloned().collect();
                        banks::save_banks(&all)
                    }
                };
                self.files_write_all(&a.str(0), &data)?;
            }
            BLOAD => {
                let a = it.inst_args(self, kw)?;
                let data = self.files_read_all(&a.str(0))?;
                let addr = self.banks.bank_or_address(a.int(1)).ok_or(Exc::Error(errors::BANK_NOT_RESERVED))?;
                self.mem_write(addr, &data);
            }
            BSAVE => {
                let a = it.inst_args(self, kw)?;
                let (s, e) = (a.int(1) as u32, a.int(2) as u32);
                let data = if e > s { self.banks.peek_bytes(s, (e - s) as usize) } else { Vec::new() };
                self.files_write_all(&a.str(0), &data)?;
            }
            AREG | DREG => {
                // Areg(n)=v / Dreg(n)=v: registers for Call/Execall (stored only).
                let idx = self.reg_index(it)?;
                it.expect(OP_EQ)?;
                let v = it.eval_int(self)?;
                let regs = if kw.token == AREG { &mut self.areg } else { &mut self.dreg };
                regs[idx] = v;
            }
            CALL | EXECALL | GFXCALL | DOSCALL | INTCALL => {
                return Err(Exc::Message("Amiga machine code and library calls are not supported".into()));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn banks_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            START | BSTART => {
                let n = it.func_args(self, kw)?.int(0);
                let addr = self.banks.start(n.clamp(0, 65535) as u16).ok_or(Exc::Error(errors::BANK_NOT_RESERVED))?;
                Value::Int(addr as i32)
            }
            LENGTH | BLENGTH => {
                let n = it.func_args(self, kw)?.int(0);
                Value::Int(self.banks.length(n.clamp(0, 65535) as u16).unwrap_or(0) as i32)
            }
            PEEK | DEEK | LEEK => {
                let addr = it.func_args(self, kw)?.int(0);
                Value::Int(match kw.token {
                    PEEK => self.peek(it, addr),
                    DEEK => self.deek(it, addr),
                    _ => self.leek(it, addr),
                })
            }
            PEEK_S | PEEK_S_2 => {
                // Peek$(address,length[,stop$]): bytes up to the length or
                // the first character of stop$.
                let a = it.func_args(self, kw)?;
                let (addr, len) = (a.int(0) as u32, a.int(1));
                if len < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                self.refresh_var_maps(it, addr, len as usize);
                let mut data = self.banks.peek_bytes(addr, (len as usize).min(crate::interp::value::STRING_MAX));
                if a.len() > 2
                    && let Some(&stop) = a.str(2).first()
                    && let Some(i) = data.iter().position(|&c| c == stop)
                {
                    data.truncate(i);
                }
                Value::Str(data.into())
            }
            HUNT => {
                let a = it.func_args(self, kw)?;
                let (s, e, needle) = (a.int(0) as u32, a.int(1) as u32, a.str(2));
                let mut found = 0;
                if !needle.is_empty() && e > s {
                    let hay = self.banks.peek_bytes(s, (e - s) as usize);
                    if let Some(i) = hay.windows(needle.len()).position(|w| w == &needle[..]) {
                        found = s as i32 + i as i32;
                    }
                }
                Value::Int(found)
            }
            VARPTR => {
                it.expect(crate::tokens::TK_PAR1)?;
                let (loc, ty) = it.var_ref(self)?;
                it.expect(crate::tokens::TK_PAR2)?;
                Value::Int(self.map_variable(it, loc, ty, false) as i32)
            }
            ARRAY => {
                it.expect(crate::tokens::TK_PAR1)?;
                let (loc, ty) = it.array_ref(self)?;
                it.expect(crate::tokens::TK_PAR2)?;
                Value::Int(self.map_variable(it, loc, ty, true) as i32)
            }
            FREE => Value::Int(32_000),
            CHIP_FREE => Value::Int(1_500_000),
            FAST_FREE => Value::Int(6_000_000),
            AREG | DREG => {
                let idx = self.reg_index(it)?;
                Value::Int(if kw.token == AREG { self.areg[idx] } else { self.dreg[idx] })
            }
            EXECALL | GFXCALL | DOSCALL | INTCALL => {
                return Err(Exc::Message("Amiga library calls are not supported".into()));
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    fn reg_index(&mut self, it: &mut Interp) -> R<usize> {
        it.expect(crate::tokens::TK_PAR1)?;
        let n = it.eval_int(self)?;
        it.expect(crate::tokens::TK_PAR2)?;
        if !(0..8).contains(&n) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        Ok(n as usize)
    }

    /// `Varptr(v)` / `Array(a(0))`: gives the variable an address in the
    /// emulated memory. The memory is refreshed from the variable before
    /// each read and written back after each write, so Peek/Poke/Leek on
    /// variable addresses work like on the Amiga. Strings point at their
    /// characters, with the length word just before as in AMOS.
    fn map_variable(&mut self, it: &mut Interp, loc: VarLoc, ty: u8, array: bool) -> u32 {
        if let Some(m) = self.var_maps.iter().find(|m| m.loc == loc && m.array == array) {
            let a = m.addr;
            self.refresh_var_maps(it, a, 1);
            return a;
        }
        let bytes = var_bytes(it, &loc, ty, array);
        let cap = (bytes.len() + 64).next_multiple_of(4) as u32;
        // Variable images live in their own area of free memory.
        let base = 0x0080_0000 + self.var_maps.iter().map(|m| m.cap + 16).sum::<u32>();
        let addr = if ty == 2 && !array { base + 2 } else { base };
        self.var_maps.push(VarMap { addr, base, cap, loc, ty, array });
        self.banks.poke_bytes(base, &bytes);
        addr
    }

    // Keywords as typed functions (the token path and compiled code call
    // these with the parameters read). Memory mapped on Varptr'd variables
    // is refreshed from / written back to them around the access.

    /// Writes `data` at `addr` (`Poke`, `Doke`, `Loke`).
    fn mem_store(&mut self, it: &mut Interp, addr: i32, data: &[u8]) -> R<()> {
        let addr = addr as u32;
        self.refresh_var_maps(it, addr, 4);
        self.mem_write(addr, data);
        self.write_back_var_maps(it, addr, 4)
    }

    /// `Poke addr,v`.
    pub(crate) fn poke(&mut self, it: &mut Interp, addr: i32, v: i32) -> R<()> {
        self.mem_store(it, addr, &[v as u8])
    }

    /// `Doke addr,v`.
    pub(crate) fn doke(&mut self, it: &mut Interp, addr: i32, v: i32) -> R<()> {
        self.mem_store(it, addr, &(v as u16).to_be_bytes())
    }

    /// `Loke addr,v`.
    pub(crate) fn loke(&mut self, it: &mut Interp, addr: i32, v: i32) -> R<()> {
        self.mem_store(it, addr, &(v as u32).to_be_bytes())
    }

    /// `Peek(addr)`.
    pub(crate) fn peek(&mut self, it: &mut Interp, addr: i32) -> i32 {
        let addr = addr as u32;
        self.refresh_var_maps(it, addr, 4);
        self.mem_read(addr, 1)[0] as i32
    }

    /// `Deek(addr)`.
    pub(crate) fn deek(&mut self, it: &mut Interp, addr: i32) -> i32 {
        let addr = addr as u32;
        self.refresh_var_maps(it, addr, 4);
        u16::from_be_bytes(self.mem_read(addr, 2).try_into().unwrap()) as i32
    }

    /// `Leek(addr)`.
    pub(crate) fn leek(&mut self, it: &mut Interp, addr: i32) -> i32 {
        let addr = addr as u32;
        self.refresh_var_maps(it, addr, 4);
        u32::from_be_bytes(self.mem_read(addr, 4).try_into().unwrap()) as i32
    }

    /// Copies variables mapped over [addr, addr+len) into memory.
    fn refresh_var_maps(&mut self, it: &mut Interp, addr: u32, len: usize) {
        let end = addr.wrapping_add(len as u32);
        let maps: Vec<VarMap> =
            self.var_maps.iter().filter(|m| m.base < end && addr < m.base + m.cap).cloned().collect();
        for m in maps {
            let mut bytes = var_bytes(it, &m.loc, m.ty, m.array);
            bytes.truncate(m.cap as usize);
            self.banks.poke_bytes(m.base, &bytes);
        }
    }

    /// Writes memory changes over [addr, addr+len) back to mapped variables.
    fn write_back_var_maps(&mut self, it: &mut Interp, addr: u32, len: usize) -> R<()> {
        let end = addr.wrapping_add(len as u32);
        let maps: Vec<VarMap> =
            self.var_maps.iter().filter(|m| m.base < end && addr < m.base + m.cap).cloned().collect();
        for m in maps {
            let old = var_bytes(it, &m.loc, m.ty, m.array);
            let new = self.banks.peek_bytes(m.base, old.len().min(m.cap as usize));
            if new == old[..new.len()] {
                continue;
            }
            if m.array {
                if let Ok(arr) = it.array_mut(&m.loc) {
                    let n = arr.len();
                    for i in 0..n {
                        let w = u32::from_be_bytes(new[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
                        match &mut arr.data {
                            crate::interp::value::ArrayData::Int(v) => v[i] = w as i32,
                            crate::interp::value::ArrayData::Float(v) => v[i] = crate::ffp::Ffp(w).to_f64(),
                            crate::interp::value::ArrayData::Str(_) => {}
                        }
                    }
                }
                continue;
            }
            let v = match m.ty {
                0 => Value::Int(u32::from_be_bytes(new[..4].try_into().unwrap()) as i32),
                1 if it.double => Value::Float(f64::from_be_bytes(new[..8].try_into().unwrap())),
                1 => Value::Float(crate::ffp::Ffp(u32::from_be_bytes(new[..4].try_into().unwrap())).to_f64()),
                // Strings keep their length; only the characters change.
                _ => Value::Str(new[2..].to_vec().into()),
            };
            it.write_loc(&m.loc, m.ty, v)?;
        }
        Ok(())
    }

    /// Reads memory, including the emulated hardware registers.
    pub(crate) fn mem_read(&mut self, addr: u32, len: usize) -> Vec<u8> {
        (0..len as u32).map(|i| self.mem_read_byte(addr.wrapping_add(i))).collect()
    }

    fn mem_read_byte(&mut self, addr: u32) -> u8 {
        match addr {
            // CIA-A PRA: bit 6 = left mouse button / fire 0, bit 7 = fire 1
            // (active low), bit 1 = LED.
            0xBFE001 => {
                let mut v = 0xFC;
                if self.input.mouse_buttons & 1 != 0 {
                    v &= !0x40;
                }
                if self.input.joy_state(1) & 16 != 0 {
                    v &= !0x80;
                }
                v
            }
            // VHPOSR: beam position.
            0xDFF006 => ((self.vbl_count * 7) & 0xFF) as u8,
            0xDFF007 => ((self.vbl_count * 13) & 0xFF) as u8,
            _ => self.banks.peek(addr),
        }
    }

    /// Writes memory; colour registers ($DFF180-$DFF1BE) change the palette
    /// of the current screen.
    pub(crate) fn mem_write(&mut self, addr: u32, data: &[u8]) {
        if (0xDFF180..0xDFF1C0).contains(&addr) && data.len() >= 2 && addr & 1 == 0 {
            for (i, w) in data.chunks(2).enumerate() {
                if w.len() == 2 {
                    let reg = ((addr - 0xDFF180) / 2) as usize + i;
                    if reg < 32
                        && let Some(s) = self.screens.current_mut()
                    {
                        s.palette[reg] = u16::from_be_bytes([w[0], w[1]]) & 0xFFF;
                    }
                }
            }
            return;
        }
        self.banks.poke_bytes(addr, data);
    }

    /// Loads banks from file data (`Load`): a bank file, a set of banks, or
    /// the banks of an AMOS program.
    pub(crate) fn load_banks(&mut self, data: &[u8], number: Option<i32>) -> R<()> {
        let banks = if data.starts_with(b"AMOS") {
            crate::program::Program::load(data).map_err(|_| Exc::Error(95))?.banks
        } else {
            banks::parse_banks(data).map_err(|_| Exc::Error(95))?
        };
        for mut b in banks {
            if let Some(n) = number
                && matches!(b.data, BankData::Raw(_))
            {
                b.number = n.clamp(1, 65535) as u16;
            }
            if let BankData::Images { images: new, palette } = &b.data
                && let Some(BankData::Images { images, .. }) = self.banks.get_mut(b.number).map(|x| &mut x.data)
                && number.is_some_and(|n| n != 0)
            {
                // Load "x.abk",1 appends to the sprite bank.
                images.extend(new.iter().cloned());
                let _ = palette;
                continue;
            }
            self.banks.insert(b);
        }
        self.on_banks_changed();
        Ok(())
    }

    /// Bset / Bclr / Bchg / Ror / Rol on a variable or a memory address.
    fn bit_op(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        use tk::*;
        let n = it.eval_int(self)?;
        it.expect(TK_COMMA)?;
        let is_var = it.peek() == TK_VAR && {
            let after = it.skip_token(it.pc);
            matches!(it.rd(after), TK_EOL | crate::tokens::TK_DP | crate::tokens::TK_ELSE)
        };
        let apply = |v: u32| -> u32 {
            match kw.token {
                BSET => v | (1 << (n & 31)),
                BCLR => v & !(1 << (n & 31)),
                BCHG => v ^ (1 << (n & 31)),
                ROR_B => (v & !0xFF) | ((v as u8).rotate_right(n as u32 & 7) as u32),
                ROL_B => (v & !0xFF) | ((v as u8).rotate_left(n as u32 & 7) as u32),
                ROR_W => (v & !0xFFFF) | ((v as u16).rotate_right(n as u32 & 15) as u32),
                ROL_W => (v & !0xFFFF) | ((v as u16).rotate_left(n as u32 & 15) as u32),
                ROR_L => v.rotate_right(n as u32 & 31),
                _ => v.rotate_left(n as u32 & 31),
            }
        };
        if is_var {
            let (loc, ty) = it.var_ref(self)?;
            let v = match it.read_loc(&loc, ty) {
                Value::Int(i) => i as u32,
                _ => return err(errors::TYPE_MISMATCH),
            };
            it.write_loc(&loc, ty, Value::Int(apply(v) as i32))?;
        } else {
            let addr = it.eval_int(self)? as u32;
            let v = u32::from_be_bytes(self.mem_read(addr, 4).try_into().unwrap());
            self.mem_write(addr, &apply(v).to_be_bytes());
        }
        Ok(())
    }

    /// Called when banks were loaded or erased (sprite bank, samples...).
    pub(crate) fn on_banks_changed(&mut self) {
        self.sound_bank_check();
        // Sprite/icon masks are made lazily from the bank images.
        self.sprites.masks = Default::default();
    }
}

#[cfg(test)]
mod tests {
    use crate::Machine;

    fn run(src: &str) -> String {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let mut m = Machine::new();
        m.run_program(&prg).unwrap();
        for _ in 0..5 {
            m.vbl();
        }
        m.hw.log.concat().replace("\r\n", "\n").trim_end_matches("End").to_string()
    }

    #[test]
    fn varptr_maps_variables_to_memory() {
        assert_eq!(run("A$=\"ABCD\" : Print Leek(Varptr(A$)) : Print Deek(Varptr(A$)-2)"), " 1094861636\n 4\n");
        assert_eq!(run("A$=\"ABCD\" : Poke Varptr(A$),90 : Print A$"), "ZBCD\n");
        assert_eq!(run("X=5 : Loke Varptr(X),7 : Print X"), " 7\n");
        assert_eq!(run("Dim T(3) : T(2)=9 : Print Leek(Array(T(0))+8)"), " 9\n");
        assert_eq!(run("Reserve As Work 10,16 : Poke Start(10)+3,42 : Print Peek(Start(10)+3)"), " 42\n");
    }
}
