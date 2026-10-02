//! The interpreter's [`Host`] interface: dispatches keywords to the
//! subsystems.

use super::Hardware;
use crate::interp::stmt::InputState;
use crate::interp::value::Value;
use crate::interp::{Exc, Host, Interp, R, StopInfo, StopReasonOrError};
use crate::tokens::Keyword;

/// Name of a keyword for messages.
pub fn keyword_name(kw: Keyword) -> String {
    kw.def().map_or_else(|| format!("token {:04X}", kw.token), |d| d.name.trim().to_string())
}

pub fn not_implemented<T>(kw: Keyword) -> R<T> {
    Err(Exc::Message(format!("Not implemented: {}", keyword_name(kw))))
}

/// Text describing why a program stopped.
pub fn describe_stop(it: &Interp, info: &StopInfo) -> String {
    let _ = it;
    match &info.reason {
        StopReasonOrError::Stop(r) => format!("{r:?}"),
        StopReasonOrError::Error(n) => crate::errors::message(*n).to_string(),
        StopReasonOrError::Message(m) => m.clone(),
        StopReasonOrError::Test(n) => crate::errors::test_message(*n).to_string(),
    }
}

impl Host for Hardware {
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        if self.screen_instruction(it, kw)?
            || self.text_instruction(it, kw)?
            || self.draw_instruction(it, kw)?
            || self.sprites_instruction(it, kw)?
            || self.sound_instruction(it, kw)?
            || self.input_instruction(it, kw)?
            || self.banks_instruction(it, kw)?
            || self.files_instruction(it, kw)?
            || self.system_instruction(it, kw)?
        {
            return Ok(());
        }
        not_implemented(kw)
    }

    fn function(&mut self, it: &mut Interp, kw: Keyword) -> R<Value> {
        if let Some(v) = self.screen_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.text_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.draw_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.sprites_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.sound_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.input_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.banks_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.files_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.system_function(it, kw)? {
            return Ok(v);
        }
        not_implemented(kw)
    }

    /// Reserved variables used as instructions (`X Mouse=...`) go through
    /// the instruction handlers, which see a `V` keyword.
    fn reserved_assign(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        self.instruction(it, kw)
    }

    fn test_point(&mut self, it: &mut Interp) -> R<()> {
        self.sprites_test_point(it)?;
        self.screen_test_point(it)
    }

    fn take_break(&mut self) -> bool {
        self.input.take_break()
    }

    fn print(&mut self, it: &mut Interp, text: &[u8]) -> R<()> {
        self.text_print(it, text)
    }

    fn read_line(&mut self, it: &mut Interp, state: &mut InputState) -> R<Option<Vec<u8>>> {
        self.input_read_line(it, state)
    }
}
