//! Miscellaneous system instructions.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn system_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        let _ = (it, kw);
        Ok(false)
    }

    pub(crate) fn system_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        let _ = (it, kw);
        Ok(None)
    }
}
