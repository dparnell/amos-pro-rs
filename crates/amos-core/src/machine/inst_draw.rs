//! Drawing primitives, blocks, IFF pictures, packing.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn draw_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        let _ = (it, kw);
        Ok(false)
    }

    pub(crate) fn draw_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        let _ = (it, kw);
        Ok(None)
    }
}
