//! Memory banks and memory access (`+Lib.s` Bnk.*, Peek/Poke...).

use super::Hardware;
use crate::banks::{self, Bank, BankData};
use crate::errors;
use crate::interp::value::Value;
use crate::interp::{Exc, Interp, R, err};
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
                let (addr, v) = (a.int(0) as u32, a.int(1) as u32);
                match kw.token {
                    POKE => self.mem_write(addr, &[v as u8]),
                    DOKE => self.mem_write(addr, &(v as u16).to_be_bytes()),
                    _ => self.mem_write(addr, &v.to_be_bytes()),
                }
            }
            COPY => {
                let a = it.inst_args(self, kw)?;
                let (s, e, d) = (a.int(0) as u32, a.int(1) as u32, a.int(2) as u32);
                if e > s {
                    let data = self.banks.peek_bytes(s, (e - s) as usize);
                    self.mem_write(d, &data);
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
                let addr = it.func_args(self, kw)?.int(0) as u32;
                let v = match kw.token {
                    PEEK => self.mem_read(addr, 1)[0] as i32,
                    DEEK => u16::from_be_bytes(self.mem_read(addr, 2).try_into().unwrap()) as i32,
                    _ => u32::from_be_bytes(self.mem_read(addr, 4).try_into().unwrap()) as i32,
                };
                Value::Int(v)
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
