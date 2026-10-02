//! Miscellaneous system instructions: the AMOS/Workbench switch, program
//! information, the Request and Compiler extensions, AmigaOS-only features.

use super::Hardware;
use crate::interp::value::{Value, astr, empty_str};
use crate::interp::{Exc, Interp, R};
use crate::tokens::{Keyword, TokenKind, tk};

impl Hardware {
    pub(crate) fn system_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        match kw.slot {
            // Compiler extension: its instructions only steer the compiler
            // (Comp Test On/Off, Comp Err...) and do nothing when interpreted.
            5 => {
                it.inst_args(self, kw)?;
                return Ok(true);
            }
            // Request extension: Request On/Off/Wb choose whether AmigaDOS
            // requesters appear on the AMOS screen. There are no system
            // requesters here, so they only store the setting.
            3 => {
                self.system_requests = match kw.token {
                    tk::REQUEST_REQUEST_ON => 1,
                    tk::REQUEST_REQUEST_OFF => 0,
                    tk::REQUEST_REQUEST_WB => 2,
                    _ => return Ok(false),
                };
                return Ok(true);
            }
            0 => {}
            _ => return Ok(false),
        }
        use tk::*;
        let reserved = kw.def().is_some_and(|d| d.kind() == TokenKind::ReservedVariable);
        match kw.token {
            // There is no Workbench to switch to: the AMOS display stays.
            AMOS_TO_FRONT | AMOS_TO_BACK | AMOS_LOCK | AMOS_UNLOCK | MULTI_WAIT => {}
            COMMAND_LINE_S if reserved => {
                it.expect(OP_EQ)?;
                self.command_line = it.eval_str(self)?.to_vec();
            }
            EXEC => {
                it.inst_args(self, kw)?;
                return Err(Exc::Message("Exec: running AmigaDOS commands is not supported".into()));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn system_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot == 5 {
            it.func_args(self, kw)?;
            return Ok(Some(Value::Int(0)));
        }
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            // AMOS is always the front "screen" here.
            AMOS_HERE => Value::Int(-1),
            COMMAND_LINE_S => Value::Str(astr(&self.command_line)),
            // Programs run from the editor / runner: no program under this one,
            // no other programs in memory.
            PRG_UNDER | PRG_STATE => Value::Int(0),
            PRG_FIRST_S | PRG_NEXT_S => {
                it.func_args(self, kw)?;
                Value::Str(empty_str())
            }
            DISC_INFO_S => {
                let p = it.func_args(self, kw)?.str(0);
                let path = crate::detok::latin1_to_string(&p);
                let vol = path.split(':').next().unwrap_or("").to_string();
                if !self.files.exists(&format!("{vol}:")) {
                    return crate::interp::err(86);
                }
                Value::Str(astr(format!("{vol}:{}", 512 * 1024).as_bytes()))
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }
}
