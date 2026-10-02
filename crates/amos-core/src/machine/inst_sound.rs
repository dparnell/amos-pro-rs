//! Sound: samples, music, tracker, Boom/Shoot/Bell.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn sound_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        let _ = (it, kw);
        Ok(false)
    }

    pub(crate) fn sound_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        let _ = (it, kw);
        Ok(None)
    }

    pub(crate) fn sound_reset(&mut self) {}

    pub(crate) fn sound_vbl(&mut self) {}
}
