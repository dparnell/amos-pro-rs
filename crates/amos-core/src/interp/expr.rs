//! Expression evaluation (`New_Evalue`, `+ILib.s`).
//!
//! Every operator is its own precedence level, ordered by its (unsigned)
//! token value, and operators of equal level associate to the left. This
//! reproduces AMOS quirks such as `10*3/4 = 0` (evaluated as `10*(3/4)`).

use super::value::{
    AStr, ArrayData, ENT_NUL, STRING_MAX, Value, Var, astr, empty_str, float_to_int,
};
use std::rc::Rc;

use super::params::ArgVec;
use super::{Exc, Host, Interp, R, err};
use crate::errors;
use crate::ffp::Ffp;
use crate::number::ffp_to_f64;
use crate::program::read_u32;
use crate::tokens::{tk, *};

impl Interp {
    /// Evaluates an expression at pc.
    pub fn eval(&mut self, hw: &mut dyn Host) -> R<Value> {
        // Integer expressions (constants, integer variables, integer
        // operators) are evaluated without values first; anything else, or
        // an error, and the expression is evaluated again the general way
        // (the attempt has no side effects).
        let mut p = self.pc;
        if let Some(v) = self.int_prec(&mut p, 0x7FFF) {
            self.pc = p;
            return Ok(Value::Int(v));
        }
        self.eval_prec(hw, 0x7FFF)
    }

    /// `eval_prec` for an integer only expression at `*p` (moved past it),
    /// `None` when the expression is not one (or fails).
    fn int_prec(&self, p: &mut usize, min: u16) -> Option<i32> {
        let mut lhs = self.int_operand(p)?;
        loop {
            let op = self.rd(*p);
            if op & 0x8000 == 0 || op <= min {
                return Some(lhs);
            }
            *p += 2;
            let rhs = self.int_prec(p, op)?;
            lhs = int_binop(op, lhs, rhs)?;
        }
    }

    /// `operand` for an integer: constant, integer scalar variable,
    /// parenthesised integer expression or Not, with unary minus.
    fn int_operand(&self, p: &mut usize) -> Option<i32> {
        let mut negate = false;
        while self.rd(*p) & 0x8000 != 0 {
            *p += 2;
            negate = true;
        }
        let q = *p;
        let v = match self.rd(q) {
            TK_ENT | TK_HEX | TK_BIN => {
                *p = q + 6;
                read_u32(&self.code, q + 2) as i32
            }
            TK_VAR => {
                let flags = self.code[q + 5];
                if flags & 3 != 0 {
                    return None;
                }
                *p = q + 6 + self.code[q + 4] as usize;
                if flags & crate::program::var_flags::ARRAY != 0 {
                    self.int_element(self.rd(q + 2), p)?
                } else {
                    match self.var_slot_ref(self.rd(q + 2)) {
                        Var::Scalar(Value::Int(i)) => *i,
                        Var::Unset => 0,
                        _ => return None,
                    }
                }
            }
            TK_PAR1 => {
                *p = q + 2;
                let v = self.int_prec(p, 0x7FFF)?;
                if self.rd(*p) != TK_PAR2 {
                    return None;
                }
                *p += 2;
                v
            }
            TK_NOT => {
                *p = q + 2;
                !self.int_prec(p, 0x7FFF)?
            }
            _ => return None,
        };
        Some(if negate { v.wrapping_neg() } else { v })
    }

    /// `int_operand` for an element of the integer array in `slot`, `*p`
    /// after the array token: `(i, ...)` with integer only indices, read
    /// as `var_ref` / `read_loc` do (`None` for anything else, including
    /// the errors, left to the general path).
    #[inline(never)]
    fn int_element(&self, slot: u16, p: &mut usize) -> Option<i32> {
        if self.rd(*p) != TK_PAR1 {
            return None;
        }
        *p += 2;
        let mut idx = [0i32; 8];
        let mut n = 0;
        loop {
            let v = self.int_prec(p, 0x7FFF)?;
            *idx.get_mut(n)? = v;
            n += 1;
            match self.rd(*p) {
                TK_COMMA => *p += 2,
                TK_PAR2 => {
                    *p += 2;
                    break;
                }
                _ => return None,
            }
        }
        match self.var_slot_ref(slot) {
            Var::Array(a) => match &a.data {
                ArrayData::Int(v) => v.get(a.index(&idx[..n])?).copied(),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn eval_int(&mut self, hw: &mut dyn Host) -> R<i32> {
        let v = self.eval(hw)?;
        self.to_int(v)
    }

    pub fn eval_float(&mut self, hw: &mut dyn Host) -> R<f64> {
        let v = self.eval(hw)?;
        self.to_float(v)
    }

    pub fn eval_str(&mut self, hw: &mut dyn Host) -> R<AStr> {
        match self.eval(hw)? {
            Value::Str(s) => Ok(s),
            _ => err(errors::TYPE_MISMATCH),
        }
    }

    /// Condition of If / While / Until: floats are truncated first.
    pub fn eval_cond(&mut self, hw: &mut dyn Host) -> R<bool> {
        Ok(self.eval_int(hw)? != 0)
    }

    pub fn to_int(&self, v: Value) -> R<i32> {
        match v {
            Value::Int(i) => Ok(i),
            Value::Float(f) => Ok(float_to_int(f)),
            Value::Str(_) => err(errors::TYPE_MISMATCH),
        }
    }

    pub fn to_float(&self, v: Value) -> R<f64> {
        match v {
            Value::Int(i) => Ok(self.int_to_float(i)),
            Value::Float(f) => Ok(f),
            Value::Str(_) => err(errors::TYPE_MISMATCH),
        }
    }

    fn eval_prec(&mut self, hw: &mut dyn Host, min: u16) -> R<Value> {
        let mut lhs = self.operand(hw)?;
        loop {
            let op = self.peek();
            if op & 0x8000 == 0 || op <= min {
                return Ok(lhs);
            }
            self.pc += 2;
            // After an integer (e.g. a function value: `Mouse Key and
            // M>0`), the right operand is tried as an integer only
            // expression first, as `eval` does.
            let mut p = self.pc;
            let rhs = match lhs {
                Value::Int(_) => match self.int_prec(&mut p, op) {
                    Some(v) => {
                        self.pc = p;
                        Value::Int(v)
                    }
                    None => self.eval_prec(hw, op)?,
                },
                _ => self.eval_prec(hw, op)?,
            };
            lhs = self.binop(op, lhs, rhs)?;
        }
    }

    fn operand(&mut self, hw: &mut dyn Host) -> R<Value> {
        let mut negate = false;
        while self.peek() & 0x8000 != 0 {
            // Any operator token in operand position is a unary minus; the
            // original negates once whatever the number of signs.
            self.pc += 2;
            negate = true;
        }
        let v = self.operand_value(hw)?;
        if !negate {
            return Ok(v);
        }
        Ok(match v {
            Value::Int(i) => Value::Int(i.wrapping_neg()),
            Value::Float(f) => Value::Float(-f),
            Value::Str(_) => return err(errors::TYPE_MISMATCH),
        })
    }

    fn operand_value(&mut self, hw: &mut dyn Host) -> R<Value> {
        let p = self.pc;
        let t = self.rd(p);
        match t {
            TK_VAR => {
                let flags = self.code[p + 5];
                if flags & crate::program::var_flags::ARRAY == 0 {
                    // Scalar: what `var_ref` + `read_loc` do, without the
                    // location record.
                    let slot = self.rd(p + 2);
                    self.pc = p + 6 + self.code[p + 4] as usize;
                    return Ok(match self.var_slot(slot) {
                        Var::Scalar(v) => v.clone(),
                        _ => Value::zero(flags & 3),
                    });
                }
                let (loc, ty) = self.var_ref(hw)?;
                Ok(self.read_loc(&loc, ty))
            }
            TK_ENT | TK_HEX | TK_BIN => {
                self.pc += 6;
                Ok(Value::Int(read_u32(&self.code, p + 2) as i32))
            }
            TK_FL => {
                self.pc += 6;
                Ok(Value::Float(ffp_to_f64(read_u32(&self.code, p + 2))))
            }
            TK_DFL => {
                self.pc += 10;
                let mut b = [0u8; 8];
                b.copy_from_slice(&self.code[p + 2..p + 10]);
                Ok(Value::Float(self.round_float(f64::from_be_bytes(b))))
            }
            TK_CH1 | TK_CH2 => {
                let n = self.rd(p + 2) as usize;
                self.pc = p + 4 + n + (n & 1);
                if !Rc::ptr_eq(&self.code, &self.prog_code) {
                    return Ok(Value::Str(astr(&self.code[p + 4..p + 4 + n])));
                }
                // A constant of the program: made once, then shared.
                if self.str_consts.is_empty() {
                    self.str_consts.resize(self.code.len() / 2 + 1, None);
                }
                let slot = &mut self.str_consts[p / 2];
                let s = slot.get_or_insert_with(|| astr(&self.code[p + 4..p + 4 + n]));
                Ok(Value::Str(s.clone()))
            }
            TK_PAR1 => {
                self.pc += 2;
                let v = self.eval(hw)?;
                self.expect(TK_PAR2)?;
                Ok(v)
            }
            TK_NOT => {
                self.pc += 2;
                let v = self.eval_int(hw)?;
                Ok(Value::Int(!v))
            }
            // Punctuation as an operand: omitted parameter.
            TK_COMMA | TK_PAR2 | TK_TO | TK_EOL | TK_DP => Ok(Value::Int(ENT_NUL)),
            TK_EXT => {
                let kw = Keyword { slot: self.code[p + 2], token: self.rd(p + 4) };
                self.pc += 6;
                hw.function(self, kw)
            }
            _ => {
                self.pc += 2 + inline_data_size(t);
                self.function_value(hw, Keyword { slot: 0, token: t })
            }
        }
    }

    /// The value of the function `kw` whose token was just read (pc after
    /// it and its inline data), as an operand: the interpreter's own
    /// functions (`core_function`, main library only), else `hw.function`
    /// (for compiled code; `TK_EXT` keywords always go to `hw.function`).
    pub fn function_value(&mut self, hw: &mut dyn Host, kw: Keyword) -> R<Value> {
        if kw.slot == 0 {
            // `core_function` accepts or refuses a token by its value alone.
            let (word, bit) = (kw.token as usize / 64, 1u64 << (kw.token % 64));
            if self.not_core.get(word).is_none_or(|w| w & bit == 0) {
                if let Some(v) = self.core_function(hw, kw.token)? {
                    return Ok(v);
                }
                if self.not_core.len() <= word {
                    self.not_core.resize(word + 1, 0);
                }
                self.not_core[word] |= bit;
            }
        }
        hw.function(self, kw)
    }

    // ------------------------------------------------------------------
    // Operators
    // ------------------------------------------------------------------

    fn fop(&self, a: f64, b: f64, f: impl Fn(Ffp, Ffp) -> Ffp, d: impl Fn(f64, f64) -> f64) -> f64 {
        if self.double { d(a, b) } else { f(Ffp::from_f64(a), Ffp::from_f64(b)).to_f64() }
    }

    fn compat(&self, a: Value, b: Value) -> R<(Value, Value)> {
        Ok(match (a, b) {
            (Value::Int(x), Value::Float(y)) => (Value::Float(self.int_to_float(x)), Value::Float(y)),
            (Value::Float(x), Value::Int(y)) => (Value::Float(x), Value::Float(self.int_to_float(y))),
            (a @ Value::Str(_), b @ Value::Str(_)) => (a, b),
            (Value::Str(_), _) | (_, Value::Str(_)) => return err(errors::TYPE_MISMATCH),
            (a, b) => (a, b),
        })
    }

    pub fn binop(&mut self, op: u16, a: Value, b: Value) -> R<Value> {
        use tk::*;
        if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
            // Fast path: the integer arms below, without the conversions.
            let (x, y) = (*x, *y);
            match op {
                OP_AND => return Ok(Value::Int(x & y)),
                OP_OR => return Ok(Value::Int(x | y)),
                OP_XOR => return Ok(Value::Int(x ^ y)),
                OP_MOD => return Ok(int_mod(x, y)),
                OP_EQ | OP_NE | OP_NE2 | OP_LT | OP_GT | OP_LE | OP_LE2 | OP_GE | OP_GE2 => {
                    return Ok(compare_result(op, x.cmp(&y)));
                }
                OP_PLUS => return int_add(x, y),
                OP_MINUS => return int_sub(x, y),
                OP_MUL => return int_mul(x, y),
                OP_DIV => return int_div(x, y),
                _ => {}
            }
        }
        match op {
            OP_AND | OP_OR | OP_XOR => {
                let (x, y) = (self.to_int(a)?, self.to_int(b)?);
                Ok(Value::Int(match op {
                    OP_AND => x & y,
                    OP_OR => x | y,
                    _ => x ^ y,
                }))
            }
            OP_MOD => {
                let (x, y) = (self.to_int(a)?, self.to_int(b)?);
                Ok(int_mod(x, y))
            }
            OP_POW => {
                let (x, y) = (self.to_float(a)?, self.to_float(b)?);
                Ok(Value::Float(self.round_float(x.powf(y))))
            }
            OP_EQ | OP_NE | OP_NE2 | OP_LT | OP_GT | OP_LE | OP_LE2 | OP_GE | OP_GE2 => {
                let (a, b) = self.compat(a, b)?;
                let ord = match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => x.cmp(y),
                    (Value::Float(x), Value::Float(y)) => {
                        if self.double {
                            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                        } else {
                            Ffp::from_f64(*x).cmp(Ffp::from_f64(*y))
                        }
                    }
                    (Value::Str(x), Value::Str(y)) => x[..].cmp(&y[..]),
                    _ => unreachable!(),
                };
                Ok(compare_result(op, ord))
            }
            OP_PLUS => match self.compat(a, b)? {
                (Value::Int(x), Value::Int(y)) => int_add(x, y),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(self.fop(x, y, Ffp::add, |a, b| a + b))),
                (Value::Str(x), Value::Str(y)) => {
                    if x.is_empty() {
                        return Ok(Value::Str(y));
                    }
                    if y.is_empty() {
                        return Ok(Value::Str(x));
                    }
                    if x.len() + y.len() >= STRING_MAX {
                        return err(errors::STRING_TOO_LONG);
                    }
                    // (Built in place: one allocation.)
                    Ok(Value::Str(x.iter().chain(y.iter()).copied().collect()))
                }
                _ => unreachable!(),
            },
            OP_MINUS => match self.compat(a, b)? {
                (Value::Int(x), Value::Int(y)) => int_sub(x, y),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(self.fop(x, y, Ffp::sub, |a, b| a - b))),
                (Value::Str(x), Value::Str(y)) => Ok(Value::Str(string_minus(&x, &y))),
                _ => unreachable!(),
            },
            OP_MUL => match self.compat(a, b)? {
                (Value::Int(x), Value::Int(y)) => int_mul(x, y),
                (Value::Float(x), Value::Float(y)) => Ok(Value::Float(self.fop(x, y, Ffp::mul, |a, b| a * b))),
                _ => err(errors::TYPE_MISMATCH),
            },
            OP_DIV => match self.compat(a, b)? {
                (Value::Int(x), Value::Int(y)) => int_div(x, y),
                (Value::Float(x), Value::Float(y)) => {
                    if y == 0.0 {
                        return err(errors::DIVISION_BY_ZERO);
                    }
                    Ok(Value::Float(self.fop(x, y, Ffp::div, |a, b| a / b)))
                }
                _ => err(errors::TYPE_MISMATCH),
            },
            _ => err(errors::SYNTAX_ERROR),
        }
    }

    // ------------------------------------------------------------------
    // Core functions
    // ------------------------------------------------------------------

    /// Reads `(a, b, ...)` function parameters according to `sig`.
    pub fn fn_args(&mut self, hw: &mut dyn Host, sig: &str) -> R<ArgVec> {
        if sig.is_empty() {
            return Ok(ArgVec::default());
        }
        self.expect(TK_PAR1)?;
        let v = self.args(hw, sig)?;
        self.expect(TK_PAR2)?;
        Ok(v)
    }

    // Parameters of the common string functions, read one by one: the same
    // values, conversions and errors as `fn_args` (`args_into`) without the
    // general list. (No new aggregate of values: their drop code changes
    // what is inlined into `run`.)

    /// One parameter of signature type `ty`: omitted (`ENT_NUL`), or
    /// evaluated and converted.
    #[inline]
    fn fn_param(&mut self, hw: &mut dyn Host, ty: u8) -> R<Value> {
        let t = self.peek();
        if t == TK_COMMA || t == TK_TO || t == TK_PAR2 || Self::is_end(t) {
            return Ok(Value::Int(ENT_NUL));
        }
        let v = self.eval(hw)?;
        self.convert_param(ty, v)
    }

    /// The comma between two parameters.
    #[inline]
    fn fn_sep(&mut self) -> R<()> {
        if self.peek() != TK_COMMA {
            return err(errors::SYNTAX_ERROR);
        }
        self.pc += 2;
        Ok(())
    }

    /// The parameter of a function of signature `4` (any type).
    #[inline]
    fn fn_arg_any(&mut self, hw: &mut dyn Host) -> R<Value> {
        self.expect(TK_PAR1)?;
        let v = self.fn_param(hw, b'4')?;
        self.expect(TK_PAR2)?;
        Ok(v)
    }

    /// `(string, integer)` parameters (signature `2,0`).
    #[inline]
    fn fn_str_int(&mut self, hw: &mut dyn Host) -> R<(AStr, i32)> {
        self.expect(TK_PAR1)?;
        let s = str_of(&self.fn_param(hw, b'2')?);
        self.fn_sep()?;
        let n = int_of(&self.fn_param(hw, b'0')?);
        self.expect(TK_PAR2)?;
        Ok((s, n))
    }

    /// Functions implemented by the interpreter itself (strings, maths,
    /// procedures, errors). Returns `None` for others. pc is after the token.
    fn core_function(&mut self, hw: &mut dyn Host, t: u16) -> R<Option<Value>> {
        use tk::*;
        let v = match t {
            FN => return self.call_fn(hw).map(Some),
            MIN | MAX => {
                self.expect(TK_PAR1)?;
                let a = self.eval(hw)?;
                self.expect(TK_COMMA)?;
                let b = self.eval(hw)?;
                self.expect(TK_PAR2)?;
                let (a, b) = self.compat(a, b)?;
                let gt = match (&a, &b) {
                    (Value::Int(x), Value::Int(y)) => x > y,
                    (Value::Float(x), Value::Float(y)) => x > y,
                    (Value::Str(x), Value::Str(y)) => x[..] > y[..],
                    _ => false,
                };
                if (t == MAX) == gt { a } else { b }
            }
            MATCH => {
                self.expect(TK_PAR1)?;
                let (loc, ty) = self.array_ref(hw)?;
                self.expect(TK_COMMA)?;
                let v = self.eval(hw)?;
                self.expect(TK_PAR2)?;
                let v = self.convert_for(ty, v)?;
                let arr = self.array_mut(&loc)?;
                Value::Int(array_match(arr, &v))
            }
            PARAM => Value::Int(self.param_e),
            PARAM_F => Value::Float(self.param_f),
            PARAM_S => Value::Str(self.param_s.clone()),
            TRUE => Value::Int(-1),
            FALSE => Value::Int(0),
            ERRN => Value::Int(self.error_on.saturating_sub(1) as i32),
            ERRTRAP => Value::Int(self.trap_err as i32),
            PI_F => Value::Float(self.round_float(std::f64::consts::PI)),
            _ => {
                if let Some(v) = self.string_maths_function(hw, t)? {
                    v
                } else {
                    return Ok(None);
                }
            }
        };
        Ok(Some(v))
    }

    /// `Fn name(args)`: assigns the arguments to the Def Fn parameters and
    /// evaluates its expression.
    fn call_fn(&mut self, hw: &mut dyn Host) -> R<Value> {
        let p = self.pc;
        let slot = self.rd(p + 2);
        self.pc = self.skip_token(p);
        let mut args = Vec::new();
        if self.peek() == TK_PAR1 {
            self.pc += 2;
            loop {
                args.push(self.eval(hw)?);
                match self.next_token() {
                    TK_COMMA => continue,
                    TK_PAR2 => break,
                    _ => return err(errors::SYNTAX_ERROR),
                }
            }
        }
        let def = match self.var_slot(slot) {
            Var::Fn(pos) => *pos,
            _ => return err(15),
        };
        let ret = self.pc;
        // Parameter list of the Def Fn: (a,b,...)=expression
        self.pc = def;
        let mut i = 0;
        if self.peek() == TK_PAR1 {
            self.pc += 2;
            loop {
                let (loc, ty) = self.var_ref(hw)?;
                let Some(v) = args.get(i).cloned() else { return err(16) };
                self.write_loc(&loc, ty, v)?;
                i += 1;
                match self.next_token() {
                    TK_COMMA => continue,
                    TK_PAR2 => break,
                    _ => return err(errors::SYNTAX_ERROR),
                }
            }
        }
        if i != args.len() {
            return err(16);
        }
        self.expect(tk::OP_EQ)?;
        let v = self.eval(hw);
        self.pc = ret;
        v
    }

    /// String and maths functions.
    fn string_maths_function(&mut self, hw: &mut dyn Host, t: u16) -> R<Option<Value>> {
        use tk::*;
        // Signature of the overloaded functions (only looked up by them).
        let sig = || crate::tokens::lookup(t).map_or("", |d| d.param_types());
        let v = match t {
            LEN => Value::Int(self.str_arg(hw)?.len() as i32),
            ASC => Value::Int(self.str_arg(hw)?.first().copied().unwrap_or(0) as i32),
            CHR_S => {
                let n = self.int_arg(hw)?;
                if !(0..=255).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Str(astr(&[n as u8]))
            }
            VAL => {
                let s = self.str_arg(hw)?;
                self.val(&s)
            }
            STR_S => match self.fn_arg_any(hw)? {
                Value::Int(i) => Value::Str(astr(crate::ffp::int_text(i, &mut [0; 11]))),
                Value::Float(f) => Value::Str(astr(self.format_float(f).as_bytes())),
                Value::Str(_) => return err(errors::TYPE_MISMATCH),
            },
            UPPER_S | LOWER_S => {
                let s = self.str_arg(hw)?;
                let upper = t == UPPER_S;
                Value::Str(
                    s.iter()
                        .map(|&c| {
                            if upper {
                                c.to_ascii_uppercase()
                            } else {
                                c.to_ascii_lowercase()
                            }
                        })
                        .collect(),
                )
            }
            FLIP_S => {
                let s = self.str_arg(hw)?;
                Value::Str(s.iter().rev().copied().collect())
            }
            SPACE_S => {
                let n = self.int_arg(hw)?;
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Str(filled(b' ', n))
            }
            STRING_S => {
                let (s, n) = self.fn_str_int(hw)?;
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                match s.first() {
                    Some(&c) => Value::Str(filled(c, n)),
                    None => Value::Str(empty_str()),
                }
            }
            REPEAT_S => {
                let (s, n) = self.fn_str_int(hw)?;
                if !(0..207).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Str(repeat_code(&s, n))
            }
            LEFT_S | RIGHT_S => {
                let (s, n) = self.fn_str_int(hw)?;
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let n = (n as usize).min(s.len());
                Value::Str(if t == LEFT_S { astr(&s[..n]) } else { astr(&s[s.len() - n..]) })
            }
            MID_S | MID_S_2 => {
                // `2,0,0` / `2,0`
                self.expect(TK_PAR1)?;
                let s = str_of(&self.fn_param(hw, b'2')?);
                self.fn_sep()?;
                let p = int_of(&self.fn_param(hw, b'0')?);
                let n = if t == MID_S {
                    self.fn_sep()?;
                    Some(int_of(&self.fn_param(hw, b'0')?))
                } else {
                    None
                };
                self.expect(TK_PAR2)?;
                if p < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let start = (p.max(1) - 1) as usize;
                if start >= s.len() {
                    Value::Str(empty_str())
                } else if let Some(n) = n {
                    if n == 0 {
                        Value::Str(empty_str())
                    } else if n < 0 {
                        return err(errors::ILLEGAL_FUNCTION_CALL);
                    } else {
                        let end = (start + n as usize).min(s.len());
                        Value::Str(astr(&s[start..end]))
                    }
                } else {
                    Value::Str(astr(&s[start..]))
                }
            }
            INSTR | INSTR_2 => {
                // `2,2` / `2,2,0`
                self.expect(TK_PAR1)?;
                let h = str_of(&self.fn_param(hw, b'2')?);
                self.fn_sep()?;
                let n = str_of(&self.fn_param(hw, b'2')?);
                let start = if t == INSTR_2 {
                    self.fn_sep()?;
                    int_of(&self.fn_param(hw, b'0')?)
                } else {
                    1
                };
                self.expect(TK_PAR2)?;
                if start < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Int(instr(&h, &n, start.max(1) as usize))
            }
            HEX_S | HEX_S_2 | BIN_S | BIN_S_2 => {
                let a = self.fn_args(hw, sig())?;
                let n = int_of(&a[0]) as u32;
                let digits = if a.len() == 2 { int_of(&a[1]) } else { -1 };
                let hex = t == HEX_S || t == HEX_S_2;
                let (b, len) = radix_text(n, hex, digits);
                Value::Str(astr(&b[..len]))
            }
            ABS => match self.fn_arg_any(hw)? {
                Value::Int(i) => Value::Int(i.wrapping_abs()),
                Value::Float(f) => Value::Float(f.abs()),
                v => v,
            },
            INT => match self.fn_arg_any(hw)? {
                Value::Float(f) => Value::Float(self.round_float(f.floor())),
                v => v,
            },
            SGN => {
                let v = self.fn_arg_any(hw)?;
                Value::Int(match v {
                    Value::Int(i) => i.signum(),
                    Value::Float(f) => {
                        if f > 0.0 {
                            1
                        } else if f < 0.0 {
                            -1
                        } else {
                            0
                        }
                    }
                    _ => 0,
                })
            }
            SQR => {
                let x = self.float_arg(hw)?;
                if x < 0.0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Float(self.round_float(x.sqrt()))
            }
            LOG | LN => {
                let x = self.float_arg(hw)?;
                if x < 0.0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Float(self.round_float(if t == LOG { x.log10() } else { x.ln() }))
            }
            EXP => {
                let x = self.float_arg(hw)?;
                Value::Float(self.round_float(x.exp()))
            }
            SIN | COS | TAN => {
                let mut x = self.float_arg(hw)?;
                if self.degrees {
                    x = x.to_radians();
                }
                Value::Float(self.round_float(match t {
                    SIN => x.sin(),
                    COS => x.cos(),
                    _ => x.tan(),
                }))
            }
            ASIN | ACOS | ATAN => {
                let x = self.float_arg(hw)?;
                let mut r = match t {
                    ASIN => x.asin(),
                    ACOS => x.acos(),
                    _ => x.atan(),
                };
                if self.degrees {
                    r = r.to_degrees();
                }
                Value::Float(self.round_float(r))
            }
            HSIN | HCOS | HTAN => {
                let x = self.float_arg(hw)?;
                Value::Float(self.round_float(match t {
                    HSIN => x.sinh(),
                    HCOS => x.cosh(),
                    _ => x.tanh(),
                }))
            }
            RND => {
                let n = self.int_arg(hw)?;
                Value::Int(self.rnd(n))
            }
            ERR_S => {
                let n = self.int_arg(hw)?;
                Value::Str(astr(crate::errors::message(n.clamp(0, 0xFFFF) as u16).as_bytes()))
            }
            BTST => {
                self.expect(TK_PAR1)?;
                let bit = self.eval_int(hw)?;
                self.expect(TK_COMMA)?;
                let v = self.eval_int(hw)?;
                self.expect(TK_PAR2)?;
                Value::Int(if v & (1 << (bit & 31)) != 0 { -1 } else { 0 })
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    fn str_arg(&mut self, hw: &mut dyn Host) -> R<AStr> {
        self.expect(TK_PAR1)?;
        let s = self.eval_str(hw)?;
        self.expect(TK_PAR2)?;
        Ok(s)
    }

    fn int_arg(&mut self, hw: &mut dyn Host) -> R<i32> {
        self.expect(TK_PAR1)?;
        let v = self.eval_int(hw)?;
        self.expect(TK_PAR2)?;
        Ok(v)
    }

    fn float_arg(&mut self, hw: &mut dyn Host) -> R<f64> {
        self.expect(TK_PAR1)?;
        let v = self.eval_float(hw)?;
        self.expect(TK_PAR2)?;
        Ok(v)
    }

    /// `Val` and Input number conversion (`ValRout` with sign).
    pub fn val(&self, s: &[u8]) -> Value {
        match crate::tokenise::parse_number(s, true) {
            Some((crate::tokenise::Number::Int(i), _)) => Value::Int(i),
            Some((crate::tokenise::Number::Hex(h) | crate::tokenise::Number::Bin(h), _)) => Value::Int(h as i32),
            Some((crate::tokenise::Number::Float(f, bits), _)) => {
                Value::Float(if self.double { f } else { bits.to_f64() })
            }
            None => Value::Int(0),
        }
    }

    /// Float to text as Print and Str$ do.
    pub fn format_float(&self, f: f64) -> String {
        if self.double {
            crate::ffp::format_double(f, self.fix)
        } else {
            crate::ffp::format_ffp(Ffp::from_f64(f), self.fix)
        }
    }

    /// `Rnd(n)`: 0..n inclusive (`+Lib.s` Rnd / RRnd).
    pub fn rnd(&mut self, n: i32) -> i32 {
        if n == 0 {
            return self.old_rnd;
        }
        let m = n.unsigned_abs();
        let mut mask: u32 = 1;
        while mask < m && mask < 0xFF_FFFF {
            mask = (mask << 1) | 1;
        }
        let mask = mask.min(0xFF_FFFF);
        let limit = m.min(0xFF_FFFF);
        loop {
            self.seed = mulu32(self.seed, 0xBB40_E62D).wrapping_add(1);
            let mut r = self.seed >> 8;
            if n > 0 {
                // The original mixes in the video beam position.
                let beam = (self.vbl_count as u32).wrapping_mul(227) & 0xFFFF;
                r = (r & 0xFFFF_0000) | ((r as u16).wrapping_add(beam as u16) as u32);
            }
            r &= mask;
            if r <= limit {
                self.old_rnd = r as i32;
                return r as i32;
            }
        }
    }
}

/// `binop` on two integers when its result is an integer (`None` for
/// other operators and on errors).
#[inline]
fn int_binop(op: u16, x: i32, y: i32) -> Option<i32> {
    use tk::*;
    let v = match op {
        OP_AND => Value::Int(x & y),
        OP_OR => Value::Int(x | y),
        OP_XOR => Value::Int(x ^ y),
        OP_MOD => int_mod(x, y),
        OP_EQ | OP_NE | OP_NE2 | OP_LT | OP_GT | OP_LE | OP_LE2 | OP_GE | OP_GE2 => compare_result(op, x.cmp(&y)),
        OP_PLUS => int_add(x, y).ok()?,
        OP_MINUS => int_sub(x, y).ok()?,
        OP_MUL => int_mul(x, y).ok()?,
        OP_DIV => int_div(x, y).ok()?,
        _ => return None,
    };
    match v {
        Value::Int(i) => Some(i),
        _ => None,
    }
}

/// Result of a comparison operator (-1 true, 0 false).
#[inline]
fn compare_result(op: u16, ord: std::cmp::Ordering) -> Value {
    use std::cmp::Ordering::*;
    use tk::*;
    let r = match op {
        OP_EQ => ord == Equal,
        OP_NE | OP_NE2 => ord != Equal,
        OP_LT => ord == Less,
        OP_GT => ord == Greater,
        OP_LE | OP_LE2 => ord != Greater,
        _ => ord != Less,
    };
    Value::Int(if r { -1 } else { 0 })
}

#[inline]
fn int_mod(x: i32, y: i32) -> Value {
    if y == 0 {
        return Value::Int(x);
    }
    Value::Int(((x as u32) % y.unsigned_abs()) as i32)
}

#[inline]
fn int_add(x: i32, y: i32) -> R<Value> {
    x.checked_add(y).map(Value::Int).ok_or(Exc::Error(errors::OVERFLOW))
}

#[inline]
fn int_sub(x: i32, y: i32) -> R<Value> {
    x.checked_sub(y).map(Value::Int).ok_or(Exc::Error(errors::OVERFLOW))
}

#[inline]
fn int_mul(x: i32, y: i32) -> R<Value> {
    let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
    let neg = (x < 0) != (y < 0);
    if ax < 65536 && ay < 65536 {
        let m = ax.wrapping_mul(ay);
        Ok(Value::Int(if neg { (m as i32).wrapping_neg() } else { m as i32 }))
    } else {
        let m = ax as u64 * ay as u64;
        if m >= 1 << 31 {
            return err(errors::OVERFLOW);
        }
        Ok(Value::Int(if neg { -(m as i32) } else { m as i32 }))
    }
}

#[inline]
fn int_div(x: i32, y: i32) -> R<Value> {
    if y == 0 {
        return err(errors::DIVISION_BY_ZERO);
    }
    let q = x.unsigned_abs() / y.unsigned_abs();
    let neg = (x < 0) != (y < 0);
    Ok(Value::Int(if neg { (q as i32).wrapping_neg() } else { q as i32 }))
}

/// The original's 32 bit multiply drops the high*high term.
fn mulu32(a: u32, b: u32) -> u32 {
    let (al, ah) = (a & 0xFFFF, a >> 16);
    let (bl, bh) = (b & 0xFFFF, b >> 16);
    let rot = |x: u32| x.rotate_left(16);
    (al * bl).wrapping_add(rot(ah * bl)).wrapping_add(rot(al * bh))
}

pub fn str_of(v: &Value) -> AStr {
    match v {
        Value::Str(s) => s.clone(),
        _ => empty_str(),
    }
}

pub fn int_of(v: &Value) -> i32 {
    match v {
        Value::Int(i) => *i,
        Value::Float(f) => float_to_int(*f),
        _ => 0,
    }
}

pub fn float_of(v: &Value) -> f64 {
    match v {
        Value::Int(i) => *i as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    }
}

/// `a$-b$`: removes every occurrence of b$, searching again from the start
/// after each removal.
fn string_minus(a: &[u8], b: &[u8]) -> AStr {
    if b.is_empty() {
        return astr(a);
    }
    let mut s = a.to_vec();
    while let Some(i) = s.windows(b.len()).position(|w| w == b) {
        s.drain(i..i + b.len());
    }
    s.into()
}

/// `Instr`: 1 based position of `needle` at or after `start`, else 0.
pub fn instr(h: &[u8], n: &[u8], start: usize) -> i32 {
    if n.is_empty() || h.is_empty() || start > h.len() {
        return 0;
    }
    h[start - 1..].windows(n.len()).position(|w| w == n).map_or(0, |i| (i + start) as i32)
}

/// `Space$(n)` / `String$(a$, n)` (n >= 0): `c` repeated, the count
/// truncated to 16 bits; one allocation of the exact size.
pub fn filled(c: u8, n: i32) -> AStr {
    std::iter::repeat_n(c, (n as u32 & 0xFFFF) as usize).collect()
}

/// `Repeat$(a$, n)` (0 <= n < 207): the Print control code `ESC R0`, the
/// text, `ESC R` and `'0' + n`, built at the exact size.
pub fn repeat_code(s: &[u8], n: i32) -> AStr {
    if s.len() <= 64 {
        // (Short: built in place.)
        return [27, b'R', b'0'].into_iter().chain(s.iter().copied()).chain([27, b'R', 48 + n as u8]).collect();
    }
    let mut v = Vec::with_capacity(s.len() + 6);
    v.extend_from_slice(&[27, b'R', b'0']);
    v.extend_from_slice(s);
    v.extend_from_slice(&[27, b'R', 48 + n as u8]);
    v.into()
}

/// `Hex$` / `Bin$` with optional digit count.
pub fn format_radix(n: u32, hex: bool, digits: i32) -> String {
    let (b, len) = radix_text(n, hex, digits);
    b[..len].iter().map(|&c| c as char).collect()
}

/// The text of `Hex$` / `Bin$` (`$` or `%`, then the digits) in a buffer,
/// and its length: `digits` digits if 0 <= digits <= 8 (hex) / 32 (binary),
/// else as many as the value needs ("0" for 0).
pub fn radix_text(n: u32, hex: bool, digits: i32) -> ([u8; 33], usize) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let (bits, prefix) = if hex { (4, b'$') } else { (1, b'%') };
    let max = 32 / bits;
    let count = if (0..=max as i32).contains(&digits) {
        digits as u32
    } else if n == 0 {
        1
    } else {
        (32 - n.leading_zeros()).div_ceil(bits)
    };
    let mut b = [0u8; 33];
    b[0] = prefix;
    let mask = (1 << bits) - 1;
    for i in 0..count {
        b[1 + i as usize] = DIGITS[((n >> ((count - 1 - i) * bits)) & mask) as usize];
    }
    (b, count as usize + 1)
}

/// `Match(a(0),v)` binary then linear search (see the research notes).
fn array_match(arr: &super::value::Array, v: &Value) -> i32 {
    use std::cmp::Ordering;
    let n = arr.len();
    let cmp = |i: usize| -> Ordering {
        match (arr.get(i), v) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Value::Str(a), Value::Str(b)) => a[..].cmp(&b[..]),
            _ => Ordering::Equal,
        }
    };
    let mut lo = 0usize;
    let mut step = n >> 1;
    loop {
        let i = lo + step;
        if i < n {
            match cmp(i) {
                Ordering::Equal => return i as i32,
                Ordering::Less => lo += step,
                Ordering::Greater => {}
            }
        }
        if step == 0 {
            break;
        }
        step >>= 1;
    }
    while lo < n {
        match cmp(lo) {
            Ordering::Equal => return lo as i32,
            Ordering::Greater => break,
            Ordering::Less => lo += 1,
        }
    }
    -(lo as i32 + 1)
}

#[allow(dead_code)]
fn _unused(_: Exc) {}

#[cfg(test)]
mod radix_tests {
    use super::*;

    /// The earlier `format_radix`, as the reference.
    fn reference(n: u32, hex: bool, digits: i32) -> String {
        let (prefix, bits) = if hex { ('$', 4) } else { ('%', 1) };
        let max_digits = 32 / bits;
        let mut s = String::new();
        s.push(prefix);
        if (0..=max_digits as i32).contains(&digits) {
            for i in (0..digits as u32).rev() {
                let d = (n >> (i * bits)) & ((1 << bits) - 1);
                s.push(std::char::from_digit(d, 16).unwrap().to_ascii_uppercase());
            }
        } else if hex {
            s.push_str(&format!("{n:X}"));
        } else {
            s.push_str(&format!("{n:b}"));
        }
        s
    }

    #[test]
    fn radix_matches_the_reference() {
        let mut values = vec![0u32, 1, 2, 9, 10, 15, 16, 255, 256, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF, 0xDEAD_BEEF];
        for k in 0..32 {
            values.extend([(1u32 << k).wrapping_sub(1), 1 << k, (1u32 << k) + 1]);
        }
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            values.push((x >> (x % 33)) as u32);
        }
        for &n in &values {
            for hex in [false, true] {
                for digits in (-3..=40).chain([i32::MIN, i32::MAX, 100, -100]) {
                    assert_eq!(format_radix(n, hex, digits), reference(n, hex, digits), "{n:#x} {hex} {digits}");
                    let (b, len) = radix_text(n, hex, digits);
                    assert_eq!(&b[..len], reference(n, hex, digits).as_bytes());
                }
            }
        }
    }
}

#[cfg(test)]
mod fill_tests {
    use super::*;

    #[test]
    fn filled_and_repeat_match_the_reference() {
        // The earlier code, as the reference.
        let fill_ref = |c: u8, n: i32| -> AStr { vec![c; (n as u32 & 0xFFFF) as usize].into() };
        let repeat_ref = |s: &[u8], n: i32| -> AStr {
            let mut v = vec![27, b'R', b'0'];
            v.extend_from_slice(s);
            v.extend_from_slice(&[27, b'R', 48 + n as u8]);
            v.into()
        };
        let counts = (0..300).chain([1000, 4095, 4096, 65534, 65535, 65536, 65537, 65800, 131071, 131072, i32::MAX - 1, i32::MAX]);
        for n in counts {
            for c in [b' ', b'x', 0, 255] {
                assert_eq!(filled(c, n), fill_ref(c, n), "{c} {n}");
            }
        }
        let text: Vec<u8> = (0..3000u32).map(|i| (i * 7 % 256) as u8).collect();
        for len in (0..80).chain([255, 256, 1000, 3000]) {
            for n in 0..207 {
                assert_eq!(repeat_code(&text[..len], n), repeat_ref(&text[..len], n), "{len} {n}");
            }
        }
    }
}
