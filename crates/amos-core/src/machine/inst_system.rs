//! Miscellaneous system instructions.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn system_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        // Compiler extension (slot 5): its instructions only steer the
        // compiler (Comp Test On/Off, Comp Err...) and do nothing when
        // interpreted.
        if kw.slot == 5 {
            it.inst_args(self, kw)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn system_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot == 5 {
            it.func_args(self, kw)?;
            return Ok(Some(Value::Int(0)));
        }
        Ok(None)
    }
}
