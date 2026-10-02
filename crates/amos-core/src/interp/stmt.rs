//! Instruction dispatch and the core (non hardware) statements.

use super::value::{Array, ArrayData, Value, Var, astr, float_to_int};
use super::{Exc, Host, Interp, R, Wait, WaitKind, err};
use crate::errors;
use crate::tokens::{tk, *};

/// State of a `Input` / `Line Input` in progress.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputState {
    pub buffer: Vec<u8>,
    pub cursor: usize,
    pub started: bool,
}

impl Interp {
    /// Executes the instruction at pc.
    pub(super) fn exec_instruction(&mut self, hw: &mut dyn Host) -> R<()> {
        let p = self.pc;
        let t = self.rd(p);
        match t {
            TK_VAR => return self.assign(hw),
            TK_LAB => {
                self.pc = self.skip_token(p);
                return Ok(());
            }
            TK_REM1 | TK_REM2 => {
                self.pc = self.skip_token(p);
                return Ok(());
            }
            TK_EXT => {
                let kw = Keyword { slot: self.code[p + 2], token: self.rd(p + 4) };
                self.pc = p + 6;
                return hw.instruction(self, kw);
            }
            _ => {}
        }
        if self.exec_flow(hw, t)? {
            return Ok(());
        }
        self.pc = p + 2 + inline_data_size(t);
        if self.exec_core(hw, t)? {
            return Ok(());
        }
        let kw = Keyword { slot: 0, token: t };
        if kw.def().is_some_and(|d| d.kind() == TokenKind::ReservedVariable) {
            return hw.reserved_assign(self, kw);
        }
        hw.instruction(self, kw)
    }

    /// `var = expression`.
    fn assign(&mut self, hw: &mut dyn Host) -> R<()> {
        let (loc, ty) = self.var_ref(hw)?;
        self.expect(tk::OP_EQ)?;
        let v = self.eval(hw)?;
        self.write_loc(&loc, ty, v)
    }

    fn exec_core(&mut self, hw: &mut dyn Host, t: u16) -> R<bool> {
        use tk::*;
        match t {
            PRINT => self.print_items(hw)?,
            DIM => self.dim(hw)?,
            INC | DEC => {
                let (loc, ty) = self.var_ref(hw)?;
                let v = match self.read_loc(&loc, ty) {
                    Value::Int(i) => Value::Int(if t == INC { i.wrapping_add(1) } else { i.wrapping_sub(1) }),
                    Value::Float(f) => Value::Float(if t == INC { f + 1.0 } else { f - 1.0 }),
                    v => v,
                };
                self.write_loc(&loc, ty, v)?;
            }
            ADD | ADD_2 => {
                let (loc, ty) = self.var_ref(hw)?;
                self.expect(TK_COMMA)?;
                let n = self.eval_int(hw)?;
                let cur = match self.read_loc(&loc, ty) {
                    Value::Int(i) => i,
                    Value::Float(f) => float_to_int(f),
                    _ => 0,
                };
                let mut v = cur.wrapping_add(n);
                if self.peek() == TK_COMMA {
                    self.pc += 2;
                    let a = self.eval_int(hw)?;
                    self.expect(TK_TO)?;
                    let b = self.eval_int(hw)?;
                    if v < a {
                        v = b;
                    } else if v > b {
                        v = a;
                    }
                }
                self.write_loc(&loc, ty, Value::Int(v))?;
            }
            SWAP => {
                let (a, ta) = self.var_ref(hw)?;
                self.expect(TK_COMMA)?;
                let (b, tb) = self.var_ref(hw)?;
                let va = self.read_loc(&a, ta);
                let vb = self.read_loc(&b, tb);
                self.write_loc(&a, ta, vb)?;
                self.write_loc(&b, tb, va)?;
            }
            SORT => {
                let (loc, _) = self.array_ref(hw)?;
                let arr = self.array_mut(&loc)?;
                match &mut arr.data {
                    ArrayData::Int(v) => v.sort(),
                    ArrayData::Float(v) => v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)),
                    ArrayData::Str(v) => v.sort(),
                }
            }
            SHARED | GLOBAL => {
                while !Self::is_end(self.peek()) {
                    self.pc = self.skip_token(self.pc);
                }
            }
            DEF_FN => {
                // Store the position of the parameter list in the Fn variable.
                let p = self.pc;
                let slot = self.rd(p + 2);
                let after = self.skip_token(p);
                *self.var_slot(slot) = Var::Fn(after);
                let mut q = after;
                while self.rd(q) != TK_EOL {
                    q = self.skip_token(q);
                }
                self.pc = q;
            }
            SET_BUFFER | SET_STACK => {
                self.pc = self.skip_token(self.pc);
            }
            SET_DOUBLE_PRECISION | SET_ACCESSORY | AMOS_LOCK | AMOS_UNLOCK | CLOSE_EDITOR | CLOSE_WORKBENCH => {}
            RANDOMIZE => {
                let n = self.eval_int(hw)?;
                self.seed = n as u32;
            }
            DEGREE => self.degrees = true,
            RADIAN => self.degrees = false,
            FIX => {
                let n = self.eval_int(hw)?;
                self.fix = crate::ffp::Fix::from_fix_arg(n);
            }
            WAIT => {
                let n = self.eval_int(hw)?;
                self.wait_vbls(n.max(0) as u64)?;
            }
            WAIT_VBL => self.wait_vbls(1)?,
            MID_S | MID_S_2 | LEFT_S | RIGHT_S => self.mid_assign(hw, t)?,
            INPUT | LINE_INPUT => self.input(hw, t == LINE_INPUT)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// Waits `n` vertical blanks (also used by Wait Vbl).
    pub fn wait_vbls(&mut self, n: u64) -> R<()> {
        match &self.wait {
            Some(Wait { pos, kind: WaitKind::Vbl(until) }) if *pos == self.inst_pos => {
                if self.vbl_count >= *until {
                    self.wait = None;
                    Ok(())
                } else {
                    Err(Exc::Block)
                }
            }
            _ => {
                self.wait = Some(Wait { pos: self.inst_pos, kind: WaitKind::Vbl(self.vbl_count + n) });
                Err(Exc::Block)
            }
        }
    }

    /// Returns the wait state of the current instruction (creating it).
    pub fn wait_state(&mut self, make: impl FnOnce() -> WaitKind) -> &mut WaitKind {
        if !matches!(&self.wait, Some(w) if w.pos == self.inst_pos) {
            self.wait = Some(Wait { pos: self.inst_pos, kind: make() });
        }
        &mut self.wait.as_mut().unwrap().kind
    }

    pub fn clear_wait(&mut self) {
        self.wait = None;
    }

    fn dim(&mut self, hw: &mut dyn Host) -> R<()> {
        loop {
            let p = self.pc;
            let slot = self.rd(p + 2);
            let ty = self.var_type_at(p);
            self.pc = self.skip_token(p);
            self.expect(TK_PAR1)?;
            let mut dims = Vec::new();
            let mut count: u64 = 1;
            loop {
                let n = self.eval_int(hw)?;
                if n < 0 || n >= 0xFFFF {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                dims.push(n as u16);
                match self.next_token() {
                    TK_COMMA => {
                        count *= n as u64 + 1;
                        if count >= 0x10000 {
                            return err(errors::ILLEGAL_FUNCTION_CALL);
                        }
                    }
                    TK_PAR2 => break,
                    _ => return err(errors::SYNTAX_ERROR),
                }
            }
            let var = self.var_slot(slot);
            if matches!(var, Var::Array(_)) {
                return err(errors::ARRAY_ALREADY_DIMENSIONED);
            }
            *var = Var::Array(Box::new(Array::new(ty, dims)));
            if self.peek() == TK_COMMA {
                self.pc += 2;
            } else {
                return Ok(());
            }
        }
    }

    /// Print items: `;` joins, `,` emits a tab, no separator at the end
    /// emits a new line.
    pub fn print_items(&mut self, hw: &mut dyn Host) -> R<()> {
        let text = self.print_text(hw)?;
        hw.print(self, &text)
    }

    /// Builds the text of a Print statement.
    pub fn print_text(&mut self, hw: &mut dyn Host) -> R<Vec<u8>> {
        let mut out = Vec::new();
        let mut newline = true;
        loop {
            let t = self.peek();
            if Self::is_end(t) {
                break;
            }
            newline = true;
            match t {
                TK_SEMI => {
                    self.pc += 2;
                    newline = false;
                }
                TK_COMMA => {
                    self.pc += 2;
                    out.push(9);
                    newline = false;
                }
                tk::USING => {
                    self.pc += 2;
                    let fmt = self.eval_str(hw)?;
                    if self.peek() == TK_SEMI {
                        self.pc += 2;
                    }
                    let v = self.eval(hw)?;
                    out.extend(self.print_using(&fmt, &v));
                }
                _ => {
                    let v = self.eval(hw)?;
                    out.extend(self.value_text(&v));
                }
            }
        }
        if newline {
            out.extend_from_slice(b"\r\n");
        }
        Ok(out)
    }

    /// Text of a value as Print shows it.
    pub fn value_text(&self, v: &Value) -> Vec<u8> {
        match v {
            Value::Int(i) => crate::ffp::format_int(*i).into_bytes(),
            Value::Float(f) => self.format_float(*f).into_bytes(),
            Value::Str(s) => s.to_vec(),
        }
    }

    /// `Print Using` formatting.
    fn print_using(&self, fmt: &[u8], v: &Value) -> Vec<u8> {
        match v {
            Value::Str(s) => {
                let mut out = Vec::new();
                let mut chars = s.iter();
                for &c in fmt {
                    if c == b'~' {
                        out.push(*chars.next().unwrap_or(&b' '));
                    } else {
                        out.push(c);
                    }
                }
                out
            }
            _ => {
                let x = match v {
                    Value::Int(i) => *i as f64,
                    Value::Float(f) => *f,
                    _ => 0.0,
                };
                using_number(fmt, x)
            }
        }
    }

    fn mid_assign(&mut self, hw: &mut dyn Host, t: u16) -> R<()> {
        use tk::*;
        self.expect(TK_PAR1)?;
        let (loc, ty) = self.var_ref(hw)?;
        let mut nums = Vec::new();
        while self.peek() == TK_COMMA {
            self.pc += 2;
            nums.push(self.eval_int(hw)?);
        }
        self.expect(TK_PAR2)?;
        self.expect(OP_EQ)?;
        let e = self.eval_str(hw)?;
        let cur = match self.read_loc(&loc, ty) {
            Value::Str(s) => s,
            _ => return err(errors::TYPE_MISMATCH),
        };
        if nums.iter().any(|&n| n < 0) {
            return err(errors::ILLEGAL_FUNCTION_CALL);
        }
        let len = cur.len();
        let (start, count) = match t {
            MID_S => ((nums[0].max(1) - 1) as usize, nums[1] as usize),
            MID_S_2 => ((nums[0].max(1) - 1) as usize, usize::MAX),
            LEFT_S => (0, nums[0] as usize),
            _ => {
                let n = nums[0] as usize;
                if n >= len { (0, len) } else { (len - n, n) }
            }
        };
        let mut v = cur.to_vec();
        if start < len {
            let n = count.min(len - start).min(e.len());
            v[start..start + n].copy_from_slice(&e[..n]);
        }
        self.write_loc(&loc, ty, Value::Str(v.into()))
    }

    /// `Input ["prompt";] a,b$...` and `Line Input`.
    fn input(&mut self, hw: &mut dyn Host, line: bool) -> R<()> {
        let start = self.pc;
        // Prompt
        let mut prompt: Vec<u8> = b"? ".to_vec();
        if self.peek() != TK_VAR {
            prompt = self.eval_str(hw)?.to_vec();
            self.pc += 2;
        }
        let first_time = !matches!(&self.wait, Some(w) if w.pos == self.inst_pos);
        if first_time {
            hw.print(self, &prompt)?;
        }
        let state = match self.wait_state(|| WaitKind::Input(Box::default())) {
            WaitKind::Input(s) => s.clone(),
            _ => Box::default(),
        };
        let mut state = state;
        let done = hw.read_line(self, &mut state)?;
        if let Some(WaitKind::Input(s)) = self.wait.as_mut().map(|w| &mut w.kind) {
            *s = state.clone();
        }
        let Some(text) = done else { return Err(Exc::Block) };
        self.clear_wait();
        // Assign the values.
        let mut fields: Vec<&[u8]> = if line { vec![&text[..]] } else { text.split(|&c| c == b',').collect() };
        let mut i = 0;
        self.pc = start;
        if self.peek() != TK_VAR {
            self.eval_str(hw)?;
            self.pc += 2;
        }
        loop {
            let (loc, ty) = self.var_ref(hw)?;
            let field = fields.get(i).copied().unwrap_or(b"");
            let v = if ty == 2 { Value::Str(astr(field)) } else { self.val(field) };
            self.write_loc(&loc, ty, v)?;
            i += 1;
            match self.peek() {
                TK_COMMA => self.pc += 2,
                TK_SEMI => {
                    self.pc += 2;
                    break;
                }
                _ => break,
            }
        }
        fields.clear();
        hw.print(self, b"\r\n")
    }
}

/// `Print Using` for numbers: `#` digits, `+`/`-` sign, `.` point,
/// `;` invisible point, `^` exponent.
fn using_number(fmt: &[u8], x: f64) -> Vec<u8> {
    let int_digits = fmt.iter().take_while(|&&c| c != b'.' && c != b';').filter(|&&c| c == b'#').count();
    let point = fmt.iter().position(|&c| c == b'.' || c == b';');
    let frac_digits = point.map_or(0, |p| fmt[p + 1..].iter().take_while(|&&c| c == b'#').count());
    let s = format!("{:.*}", frac_digits, x.abs());
    let (ip, fp) = s.split_once('.').unwrap_or((&s, ""));
    let mut ip_digits: Vec<u8> = ip.bytes().collect();
    while ip_digits.len() < int_digits {
        ip_digits.insert(0, b' ');
    }
    if ip_digits.len() > int_digits {
        ip_digits = ip_digits[ip_digits.len() - int_digits..].to_vec();
    }
    let mut out = Vec::new();
    let mut idig = ip_digits.into_iter();
    let mut fdig = fp.bytes();
    let mut after_point = false;
    for &c in fmt {
        match c {
            b'#' => {
                if after_point {
                    out.push(fdig.next().unwrap_or(b'0'));
                } else {
                    out.push(idig.next().unwrap_or(b' '));
                }
            }
            b'.' => {
                after_point = true;
                out.push(b'.');
            }
            b';' => after_point = true,
            b'+' => out.push(if x < 0.0 { b'-' } else { b'+' }),
            b'-' => out.push(if x < 0.0 { b'-' } else { b' ' }),
            _ => out.push(c),
        }
    }
    out
}
