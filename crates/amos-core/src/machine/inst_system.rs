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
                let a = it.inst_args(self, kw)?;
                if matches!(kw.token, tk::COMP_COMPILE | tk::COMP_COMPILE_2) {
                    self.compile_command(&a.str(0));
                    // Param = 0: compilation finished.
                    it.param_e = 0;
                }
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
            // IOPorts extension: serial, parallel and printer devices do not
            // exist on modern hosts; opening one fails like a missing device.
            6 => {
                it.inst_args(self, kw)?;
                return crate::interp::err(142);
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
            // Amiga devices and libraries are not available.
            DEV_OPEN | LIB_OPEN => {
                it.inst_args(self, kw)?;
                return crate::interp::err(142);
            }
            DEV_CLOSE | DEV_CLOSE_2 | LIB_CLOSE | LIB_CLOSE_2 => {
                it.inst_args(self, kw)?;
            }
            DEV_DO | DEV_SEND | DEV_ABORT => {
                it.inst_args(self, kw)?;
                return crate::interp::err(141);
            }
            // ARexx: there is no ARexx on modern systems.
            AREXX_OPEN => {
                it.inst_args(self, kw)?;
                return crate::interp::err(194);
            }
            AREXX_CLOSE | AREXX_WAIT | AREXX_ANSWER | AREXX_ANSWER_2 => {
                it.inst_args(self, kw)?;
                return crate::interp::err(196);
            }
            LPRINT => {
                // No printer: the text is discarded.
                it.print_text(self)?;
            }
            EXEC => {
                it.inst_args(self, kw)?;
                return Err(Exc::Message("Exec: running AmigaDOS commands is not supported".into()));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Compile "FROM ""src"" TO ""dest"" ..."`: instead of 68000 code, a
    /// standalone application (native + web) is built next to `dest`.
    fn compile_command(&mut self, cmd: &[u8]) {
        let cmd = crate::detok::latin1_to_string(cmd);
        let arg = |key: &str| -> Option<String> {
            let up = cmd.to_uppercase();
            let i = up.find(&format!("{key} "))? + key.len() + 1;
            let rest = cmd[i..].trim_start();
            if let Some(r) = rest.strip_prefix('"') {
                r.split('"').next().map(str::to_string)
            } else {
                rest.split_whitespace().next().map(str::to_string)
            }
        };
        let (Some(src), Some(dest)) = (arg("FROM"), arg("TO")) else {
            // Step / Conf / Cont / Stop sub-commands of the shell.
            return;
        };
        let split = dest.rfind(['/', ':']).map_or(0, |i| i + 1);
        let name = dest[split..].trim_end_matches(".AMOS").trim_end_matches(".amos").to_string();
        let out = if split == 0 { self.files.current_dir.clone() } else { dest[..split].trim_end_matches('/').to_string() };
        self.build_requests.push(super::BuildRequest {
            program: src,
            with_files: true,
            name: if name.is_empty() { "App".into() } else { name },
            out,
            native: true,
            web: true,
        });
    }

    pub(crate) fn system_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot == 5 {
            it.func_args(self, kw)?;
            return Ok(Some(match kw.token {
                // The last build error, if any.
                tk::COMP_COMP_ERR_S => match self.build_results.last() {
                    Some(Err(e)) => Value::Str(astr(e.as_bytes())),
                    _ => Value::Str(empty_str()),
                },
                // The builder is always available.
                tk::COMP_COMP_HERE => Value::Int(-1),
                _ => Value::Int(0),
            }));
        }
        if kw.slot == 6 {
            it.func_args(self, kw)?;
            return crate::interp::err(141);
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
            AREXX_EXIST => {
                it.func_args(self, kw)?;
                Value::Int(0)
            }
            AREXX | AREXX_S => {
                it.func_args(self, kw)?;
                return crate::interp::err(196);
            }
            DEV_CHECK | DEV_BASE | LIB_BASE | LIB_CALL => {
                it.func_args(self, kw)?;
                return crate::interp::err(141);
            }
            // Devices: the mounted volumes.
            DEV_FIRST_S => {
                it.func_args(self, kw)?;
                self.dev_listing = self.files.volume_names();
                self.dev_listing.reverse();
                Value::Str(self.dev_listing.pop().map(|v| astr(format!("{v}:").as_bytes())).unwrap_or_else(empty_str))
            }
            DEV_NEXT_S => {
                it.func_args(self, kw)?;
                Value::Str(self.dev_listing.pop().map(|v| astr(format!("{v}:").as_bytes())).unwrap_or_else(empty_str))
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

#[cfg(test)]
mod tests {
    #[test]
    fn compile_queues_a_build_request() {
        let src = "Compile 'FROM \"Work:game.AMOS\" TO \"Work:out/game_C.AMOS\" TYPE=3'\nPrint Param";
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let mut m = crate::Machine::new();
        m.run_program(&prg).unwrap();
        m.vbl();
        let r = &m.hw.build_requests[0];
        assert_eq!((r.program.as_str(), r.out.as_str(), r.name.as_str()), ("Work:game.AMOS", "Work:out", "game_C"));
    }
}
