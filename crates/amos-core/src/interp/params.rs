//! Reading instruction and function parameters.

use super::value::{AStr, ENT_NUL, Value};
use super::{Host, Interp, R, err};
use crate::errors;
use crate::tokens::*;

/// Parameters of an instruction or function, already converted to the
/// types of the signature. Omitted parameters hold [`ENT_NUL`].
#[derive(Debug, Default)]
pub struct Args(pub Vec<Value>);

impl Args {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn int(&self, i: usize) -> i32 {
        match self.0.get(i) {
            Some(Value::Int(v)) => *v,
            Some(Value::Float(f)) => super::value::float_to_int(*f),
            _ => ENT_NUL,
        }
    }

    /// Integer parameter, or `None` when omitted.
    pub fn opt(&self, i: usize) -> Option<i32> {
        let v = self.int(i);
        if v == ENT_NUL { None } else { Some(v) }
    }

    pub fn float(&self, i: usize) -> f64 {
        match self.0.get(i) {
            Some(Value::Int(v)) => *v as f64,
            Some(Value::Float(f)) => *f,
            _ => 0.0,
        }
    }

    pub fn str(&self, i: usize) -> AStr {
        match self.0.get(i) {
            Some(Value::Str(s)) => s.clone(),
            _ => super::value::empty_str(),
        }
    }

    pub fn value(&self, i: usize) -> Value {
        self.0.get(i).cloned().unwrap_or(Value::Int(ENT_NUL))
    }

    pub fn is_str(&self, i: usize) -> bool {
        matches!(self.0.get(i), Some(Value::Str(_)))
    }
}

impl Interp {
    /// Reads parameters following the signature `sig` (type chars separated
    /// by `,` or `t` for `To`), as the patched `Parameters` routines do.
    pub fn args(&mut self, hw: &mut dyn Host, sig: &str) -> R<Vec<Value>> {
        let sig = sig.as_bytes();
        let mut out = Vec::with_capacity(sig.len().div_ceil(2));
        let mut i = 0;
        while i < sig.len() {
            let ty = sig[i];
            let t = self.peek();
            let v = if t == TK_COMMA || t == TK_TO || t == TK_PAR2 || Self::is_end(t) {
                Value::Int(ENT_NUL)
            } else {
                let v = self.eval(hw)?;
                self.convert_param(ty, v)?
            };
            out.push(v);
            if let Some(&sep) = sig.get(i + 1) {
                let want = if sep == b't' { TK_TO } else { TK_COMMA };
                if self.peek() == want {
                    self.pc += 2;
                } else {
                    return err(errors::SYNTAX_ERROR);
                }
            }
            i += 2;
        }
        Ok(out)
    }

    /// Reads the parameters of the instruction `kw` (pc after the token).
    pub fn inst_args(&mut self, hw: &mut dyn Host, kw: Keyword) -> R<Args> {
        let sig = kw.def().map_or("", |d| d.param_types());
        Ok(Args(self.args(hw, sig)?))
    }

    /// Reads the parameters of the function `kw` (pc after the token).
    pub fn func_args(&mut self, hw: &mut dyn Host, kw: Keyword) -> R<Args> {
        let sig = kw.def().map_or("", |d| d.param_types());
        Ok(Args(self.fn_args(hw, sig)?))
    }

    pub fn convert_param(&self, ty: u8, v: Value) -> R<Value> {
        Ok(match ty {
            b'0' => Value::Int(self.to_int(v)?),
            b'1' => Value::Float(self.to_float(v)?),
            b'5' => {
                let mut f = self.to_float(v)?;
                if self.degrees {
                    f = f.to_radians();
                }
                Value::Float(f)
            }
            b'2' => match v {
                s @ Value::Str(_) => s,
                _ => return err(errors::TYPE_MISMATCH),
            },
            _ => v,
        })
    }
}
