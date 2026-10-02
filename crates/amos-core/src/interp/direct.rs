//! Direct mode: running a single line typed in the editor's Escape screen
//! in the context of the last program run (`Esc_R`, `+Edit.s:9185`, and
//! `VerDirect`, `+Verif.s:43`).
//!
//! The line is verified against the program's variables and procedures,
//! then appended after the program's code (the original also keeps the
//! direct line in a separate buffer and jumps to it with `New_ChrGet`).
//! Variables created by the line are added to the program's globals so a
//! later direct line sees them too.

use std::collections::HashMap;
use std::rc::Rc;

use super::value::Var;
use super::verify::{Compiled, TestError, Verifier, token_size};
use super::{Interp, OnError};
use crate::program::read_u16;
use crate::tokens::*;

impl Interp {
    /// True if a program has been loaded (direct lines then see its
    /// variables).
    pub fn has_program(&self) -> bool {
        self.prg.is_some()
    }

    /// Verifies the tokenised line `line` (as produced by
    /// [`crate::tokenise::tokenise_line`]) and prepares to run it. The
    /// caller then runs the interpreter as usual (`Machine::vbl`); it stops
    /// with `End` at the end of the line.
    pub fn run_direct(&mut self, line: &[u8]) -> Result<(), TestError> {
        let compiled = match &self.prg {
            Some(c) => c.clone(),
            None => {
                // No program run yet: an empty one gives the context.
                let c = Rc::new(Verifier::verify(&[], 0)?);
                self.start(c.clone());
                self.running = false;
                c
            }
        };
        let (code, globals) = Verifier::verify_direct(&compiled, line)?;

        // Else reached in sequence: one line Else goes to the end of the
        // line, a block Else to the token after the following End If.
        // `verify_direct` does not return these targets.
        let base = self.prog_len;
        let mut else_exit: HashMap<usize, usize> = HashMap::new();
        let mut p = 2;
        while p + 2 <= code.len() {
            let t = read_u16(&code, p);
            if t == TK_EOL {
                break;
            }
            if t == TK_ELSE || t == TK_ELSE_IF {
                let mut q = p + token_size(&code, p);
                let mut target = None;
                while q + 2 <= code.len() {
                    let u = read_u16(&code, q);
                    if u == TK_EOL {
                        break;
                    }
                    if u == tk::END_IF {
                        target = Some(q + 2);
                        break;
                    }
                    q += token_size(&code, q);
                }
                else_exit.insert(base + p, base + target.unwrap_or(q));
            }
            p += token_size(&code, p);
        }

        let mut new_compiled: Option<Compiled> = None;
        if globals.len() > compiled.globals.len() || !else_exit.is_empty() {
            let mut c = (*compiled).clone();
            c.globals = globals;
            c.else_exit.extend(else_exit);
            new_compiled = Some(c);
        }
        if let Some(c) = new_compiled {
            self.globals.resize(c.globals.len(), Var::Unset);
            self.prg = Some(Rc::new(c));
        }

        let mut all = self.code[..self.prog_len].to_vec();
        all.extend_from_slice(&code);
        self.code = Rc::new(all);
        // The direct line runs at the main program level: loops, Gosubs
        // and procedure frames of the stopped program are forgotten.
        while self.pop_ctl().is_some() {}
        self.frame_stack.clear();
        self.scope = 0;
        self.wait = None;
        self.on_error = OnError::None;
        self.error_on = 0;
        self.trap_pos = None;
        self.error_proc_depth = None;
        self.pc = base + 2;
        self.inst_pos = self.pc;
        self.running = true;
        self.direct_mode = true;
        Ok(())
    }

    /// Start of the direct mode line in the code (positions at or after it
    /// belong to the direct line rather than the program).
    pub fn direct_base(&self) -> usize {
        self.prog_len
    }
}

#[cfg(test)]
mod tests {
    use crate::Machine;
    use crate::interp::{RunState, StopReasonOrError};
    use crate::tokenise::{tokenise_line, tokenise_program};

    fn run(m: &mut Machine) -> RunState {
        for _ in 0..50 {
            m.vbl();
            if let RunState::Stopped(_) = &m.state {
                break;
            }
        }
        m.state.clone()
    }

    #[test]
    fn direct_line_sees_program_variables() {
        let mut m = Machine::new();
        let prg = tokenise_program(b"A=42\nB$=\"hi\"").unwrap();
        m.run_program(&prg).unwrap();
        run(&mut m);
        let line = tokenise_line(b"C=A+1 : D$=B$+\"!\"").unwrap().unwrap();
        m.run_direct(&line.line).unwrap();
        let st = run(&mut m);
        assert!(
            matches!(st, RunState::Stopped(ref i) if i.reason == StopReasonOrError::Stop(crate::interp::StopReason::End)),
            "{st:?}"
        );
        // A new direct line sees the variables created by the previous one.
        let line = tokenise_line(b"If C=43 Then E=1 Else E=2").unwrap().unwrap();
        m.run_direct(&line.line).unwrap();
        run(&mut m);
        let line = tokenise_line(b"If E<>1 Then Error 23").unwrap().unwrap();
        m.run_direct(&line.line).unwrap();
        let st = run(&mut m);
        assert!(
            matches!(st, RunState::Stopped(ref i) if i.reason == StopReasonOrError::Stop(crate::interp::StopReason::End)),
            "{st:?}"
        );
    }

    #[test]
    fn direct_without_program() {
        let mut m = Machine::new();
        let line = tokenise_line(b"X=1+2").unwrap().unwrap();
        m.run_direct(&line.line).unwrap();
        let st = run(&mut m);
        assert!(matches!(st, RunState::Stopped(_)));
        let bad = tokenise_line(b"Next").unwrap().unwrap();
        assert!(m.interp.run_direct(&bad.line).is_err());
    }
}
