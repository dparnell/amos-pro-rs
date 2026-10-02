//! Text windows and text output.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn text_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        let _ = (it, kw);
        Ok(false)
    }

    pub(crate) fn text_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        let _ = (it, kw);
        Ok(None)
    }

    /// Prints text in the current window of the current screen.
    pub(crate) fn text_print(&mut self, it: &mut Interp, text: &[u8]) -> R<()> {
        let _ = it;
        self.log.push(crate::detok::latin1_to_string(text));
        Ok(())
    }
}
