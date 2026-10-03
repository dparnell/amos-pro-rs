//! Control flow instructions (`+ILib.s`): loops, tests, jumps, procedures,
//! Data/Read, error handling and events.

use super::value::Value;
use super::verify::token_size;
use super::{Ctl, EveryTarget, Exc, Host, Interp, OnError, R, StopReason, err};
use crate::errors;
use crate::program::read_u16;
use crate::tokens::{tk, *};

impl Interp {
    /// First token of the statement following position `p` (`Rpt0`).
    pub fn statement_start(&self, mut p: usize) -> usize {
        loop {
            match self.rd(p) {
                TK_DP => p += 2,
                TK_EOL => {
                    let next = p + 2;
                    if next + 1 >= self.code.len() || self.code[next] == 0 {
                        return p;
                    }
                    p = next + 2;
                }
                _ => return p,
            }
        }
    }

    fn field_target(&self, field: usize) -> usize {
        field + read_u16(&self.code, field) as usize
    }

    /// Jump target of a label operand: `TK_LGO` (resolved by the verifier)
    /// or an expression giving a label name or line number.
    pub fn label_target(&mut self, hw: &mut dyn Host) -> R<usize> {
        let p = self.pc;
        if self.rd(p) == TK_LGO {
            let idx = self.rd(p + 2) as usize;
            self.pc = self.skip_token(p);
            let scope = self.scope;
            return self.compiled().scopes[scope].labels.get(idx).map(|l| l.target).ok_or(Exc::Error(errors::LABEL_NOT_DEFINED));
        }
        let v = self.eval(hw)?;
        let name: Vec<u8> = match v {
            Value::Int(i) => i.to_string().into_bytes(),
            Value::Float(f) => super::value::float_to_int(f).to_string().into_bytes(),
            Value::Str(s) => {
                if s.len() >= 32 {
                    return err(errors::LABEL_NOT_DEFINED);
                }
                s.iter().map(|c| c.to_ascii_lowercase()).collect()
            }
        };
        let scope = self.scope;
        let s = &self.compiled().scopes[scope];
        s.by_name.get(&name).map(|&i| s.labels[i].target).ok_or(Exc::Error(errors::LABEL_NOT_DEFINED))
    }

    /// Procedure number of a `TK_PRO` (or resolved variable) operand.
    pub fn proc_operand(&mut self) -> R<usize> {
        let p = self.pc;
        match self.rd(p) {
            TK_PRO | TK_VAR => {
                self.pc = self.skip_token(p);
                Ok(self.rd(p + 2) as usize)
            }
            _ => err(errors::SYNTAX_ERROR),
        }
    }

    /// Executes a control flow instruction. Returns false if `t` is not one.
    pub fn exec_flow(&mut self, hw: &mut dyn Host, t: u16) -> R<bool> {
        let p = self.pc; // position of the token
        match t {
            TK_FOR => {
                let field = p + 2;
                self.pc = p + 4;
                let (var, ty) = self.var_ref(hw)?;
                self.expect(tk::OP_EQ)?;
                let start = self.eval(hw)?;
                self.write_loc(&var, ty, start)?;
                self.expect(TK_TO)?;
                let limit = self.eval_int(hw)?;
                let step = if self.peek() == tk::STEP {
                    self.pc += 2;
                    self.eval_int(hw)?
                } else {
                    1
                };
                let body = self.statement_start(self.pc);
                let exit = self.field_target(field);
                self.push_ctl(Ctl::For { var, step, limit, body, exit })?;
            }
            TK_REPEAT | TK_DO => {
                let exit = self.field_target(p + 2);
                let body = self.statement_start(p + 4);
                self.pc = p + 4;
                self.push_ctl(if t == TK_DO { Ctl::Do { body, exit } } else { Ctl::Repeat { body, exit } })?;
            }
            TK_WHILE => {
                let exit = self.field_target(p + 2);
                self.pc = p + 4;
                if self.eval_cond(hw)? {
                    let body = self.statement_start(self.pc);
                    self.push_ctl(Ctl::While { start: p, body, exit })?;
                } else {
                    self.pc = exit;
                }
            }
            tk::NEXT => {
                self.test_point(hw)?;
                self.pc = p + 2;
                let Some(&Ctl::For { var, step, limit, body, .. }) = self.ctl.last() else {
                    return err(errors::SYNTAX_ERROR);
                };
                let (cur, ty) = match self.read_loc(&var, 0) {
                    Value::Int(i) => (i, 0),
                    Value::Float(f) => (super::value::float_to_int(f), 1),
                    Value::Str(_) => (0, 0),
                };
                let v = cur.wrapping_add(step);
                self.write_loc(&var, ty, Value::Int(v))?;
                let done = if step >= 0 { v > limit } else { v < limit };
                if done {
                    self.pop_ctl();
                    if self.peek() == TK_VAR {
                        self.pc = self.skip_token(self.pc);
                    }
                } else {
                    self.pc = body;
                }
            }
            tk::UNTIL => {
                self.test_point(hw)?;
                self.pc = p + 2;
                let c = self.eval_cond(hw)?;
                let Some(&Ctl::Repeat { body, .. }) = self.ctl.last() else {
                    return err(errors::SYNTAX_ERROR);
                };
                if c {
                    self.pop_ctl();
                } else {
                    self.pc = body;
                }
            }
            tk::WEND => {
                self.test_point(hw)?;
                let Some(&Ctl::While { start, .. }) = self.ctl.last() else {
                    return err(errors::SYNTAX_ERROR);
                };
                self.pop_ctl();
                self.pc = start;
            }
            tk::LOOP => {
                self.test_point(hw)?;
                let Some(&Ctl::Do { body, .. }) = self.ctl.last() else {
                    return err(errors::SYNTAX_ERROR);
                };
                self.pc = body;
            }
            TK_EXIT | TK_EXIT_IF => {
                self.pc = p + 6;
                if t == TK_EXIT_IF && !self.eval_cond(hw)? {
                    // Skip the optional ",n".
                    if self.peek() == TK_COMMA {
                        self.pc += 2;
                        self.pc = self.skip_token(self.pc);
                    }
                    return Ok(true);
                }
                self.test_point(hw)?;
                let frames = read_u16(&self.code, p + 4) as usize;
                for _ in 0..frames {
                    self.pop_ctl();
                }
                self.pc = self.field_target(p + 2);
            }
            TK_IF => self.exec_if(hw, p)?,
            TK_ELSE | TK_ELSE_IF => {
                // Reached at the end of the previous branch: go to End If
                // (or the end of a one-line If).
                let target = self.compiled().else_exit.get(&p).copied().unwrap_or(p + 4);
                self.pc = target;
            }
            tk::END_IF => self.pc = p + 2,
            tk::THEN => return err(errors::SYNTAX_ERROR),
            tk::GOTO => {
                self.test_point(hw)?;
                self.pc = p + 2;
                self.pc = self.label_target(hw)?;
                self.after_jump();
            }
            tk::GOSUB => {
                self.test_point(hw)?;
                self.pc = p + 2;
                let target = self.label_target(hw)?;
                let ret = self.pc;
                self.push_ctl(Ctl::Gosub { ret })?;
                self.pc = target;
            }
            tk::RETURN => {
                self.test_point(hw)?;
                loop {
                    match self.ctl.last() {
                        Some(Ctl::Gosub { ret }) => {
                            self.pc = *ret;
                            self.pop_ctl();
                            break;
                        }
                        Some(Ctl::Proc(_)) | None => return err(errors::RETURN_WITHOUT_GOSUB),
                        _ => {
                            self.pop_ctl();
                        }
                    }
                }
            }
            tk::POP => {
                self.test_point(hw)?;
                self.pc = p + 2;
                let base = self.routine_base();
                if base == 0 || !matches!(self.ctl[base - 1], Ctl::Gosub { .. }) {
                    return err(errors::POP_WITHOUT_GOSUB);
                }
                while self.ctl.len() >= base {
                    self.pop_ctl();
                }
                self.after_jump();
            }
            TK_ON => self.exec_on(hw, p)?,
            TK_PRO => {
                self.test_point(hw)?;
                self.call_with_args(hw)?;
            }
            tk::PROC => {
                self.test_point(hw)?;
                self.pc = p + 2;
                self.call_with_args(hw)?;
            }
            TK_PROCEDURE => {
                // Normal flow skips procedure definitions.
                let index = self.compiled().procs.iter().position(|pr| pr.pos == p);
                let Some(i) = index else { return err(errors::SYNTAX_ERROR) };
                let mut q = self.compiled().procs[i].end_pos;
                while self.rd(q) != TK_EOL {
                    q = self.skip_token(q);
                }
                self.pc = q;
            }
            TK_END_PROC | tk::POP_PROC => {
                self.test_point(hw)?;
                self.pc = p + 2;
                if self.frame_stack.is_empty() {
                    if t == TK_END_PROC {
                        return Err(Exc::Stop(StopReason::End));
                    }
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                if self.peek() == TK_BRA1 {
                    self.pc += 2;
                    match self.eval(hw)? {
                        Value::Int(i) => self.param_e = i,
                        Value::Float(f) => self.param_f = f,
                        Value::Str(s) => self.param_s = s,
                    }
                    self.expect(TK_BRA2)?;
                }
                if self.error_on != 0 && self.on_error_is_proc_frame() {
                    return err(8);
                }
                self.return_proc()?;
            }
            TK_DATA => {
                let mut q = p;
                while self.rd(q) != TK_EOL {
                    q = self.skip_token(q);
                }
                self.pc = q;
            }
            tk::READ => {
                self.pc = p + 2;
                loop {
                    let (loc, ty) = self.var_ref(hw)?;
                    let v = self.next_data(hw, ty)?;
                    self.write_loc(&loc, ty, v)?;
                    if self.peek() == TK_COMMA {
                        self.pc += 2;
                    } else {
                        break;
                    }
                }
            }
            tk::RESTORE => {
                self.pc = p + 2;
                if Self::is_end(self.peek()) {
                    self.data.line = 0;
                    self.data.item = 0;
                } else {
                    let target = self.label_target(hw)?;
                    if self.rd(target) != TK_DATA {
                        return err(41);
                    }
                    self.data.item = target + 4;
                    self.data.line = self.line_after(target);
                }
            }
            tk::ON_ERROR => {
                self.pc = p + 2;
                if self.error_on != 0 {
                    return err(errors::ERROR_NOT_RESUMED);
                }
                match self.peek() {
                    tk::GOTO => {
                        self.pc += 2;
                        if self.peek() == TK_ENT {
                            self.pc += 6;
                            self.on_error = OnError::None;
                        } else {
                            self.on_error = OnError::Goto(self.label_target(hw)?);
                        }
                    }
                    tk::PROC => {
                        self.pc += 2;
                        self.on_error = OnError::Proc(self.proc_operand()?);
                    }
                    _ => self.on_error = OnError::None,
                }
            }
            tk::RESUME => {
                self.test_point(hw)?;
                self.pc = p + 2;
                if self.error_on == 0 {
                    return err(errors::RESUME_WITHOUT_ERROR);
                }
                let label = if Self::is_end(self.peek()) { None } else { Some(self.label_target(hw)?) };
                if self.in_error_proc() {
                    if label.is_some() {
                        return err(4);
                    }
                    self.return_proc()?;
                    self.error_on = 0;
                } else {
                    self.error_on = 0;
                    self.pc = label.unwrap_or(self.error_pos);
                    self.after_jump();
                }
            }
            tk::RESUME_NEXT => {
                self.test_point(hw)?;
                if self.error_on == 0 {
                    return err(errors::RESUME_WITHOUT_ERROR);
                }
                if self.in_error_proc() {
                    self.return_proc()?;
                }
                self.error_on = 0;
                self.pc = self.skip_statement(self.error_pos);
            }
            tk::RESUME_LABEL => {
                self.test_point(hw)?;
                self.pc = p + 2;
                if !Self::is_end(self.peek()) {
                    let target = self.label_target(hw)?;
                    self.resume_label = Some(target);
                } else {
                    if !self.in_error_proc() {
                        return err(5);
                    }
                    let Some(target) = self.resume_label else { return err(6) };
                    self.return_proc()?;
                    self.error_on = 0;
                    self.pc = target;
                    self.after_jump();
                }
            }
            tk::TRAP => {
                self.pc = p + 2;
                self.trap_pos = Some(self.pc);
                self.trap_err = 0;
            }
            tk::ERROR => {
                self.pc = p + 2;
                let n = self.eval_int(hw)?;
                return err(n.clamp(0, 255) as u16);
            }
            tk::EVERY => {
                self.pc = p + 2;
                let n = self.eval_int(hw)?;
                if !(1..32767).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let target = match self.next_token() {
                    tk::GOSUB => EveryTarget::Gosub(self.label_target(hw)?),
                    tk::PROC => EveryTarget::Proc(self.proc_operand()?),
                    _ => return err(errors::SYNTAX_ERROR),
                };
                self.events.every_reload = n;
                self.events.every_count = n;
                self.events.every_target = Some(target);
                self.events.every_on = true;
            }
            tk::EVERY_ON => {
                self.pc = p + 2;
                self.events.every_on = true;
            }
            tk::EVERY_OFF => {
                self.pc = p + 2;
                self.events.every_on = false;
            }
            tk::BREAK_ON => {
                self.pc = p + 2;
                self.events.break_on = true;
                self.events.on_break_proc = None;
            }
            tk::BREAK_OFF => {
                self.pc = p + 2;
                self.events.break_on = false;
            }
            tk::ON_BREAK_PROC => {
                self.pc = p + 2;
                self.events.on_break_proc = Some(self.proc_operand()?);
                self.events.break_on = false;
            }
            tk::END => return Err(Exc::Stop(StopReason::End)),
            tk::STOP => return Err(Exc::Stop(StopReason::Break)),
            tk::EDIT => return Err(Exc::Stop(StopReason::Edit)),
            tk::DIRECT => return Err(Exc::Stop(StopReason::Direct)),
            tk::SYSTEM => return Err(Exc::Stop(StopReason::System)),
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn in_error_proc(&self) -> bool {
        self.error_proc_depth.is_some() && self.error_proc_depth == Some(self.frame_stack.len())
    }

    fn on_error_is_proc_frame(&self) -> bool {
        self.in_error_proc()
    }

    fn exec_if(&mut self, hw: &mut dyn Host, p: usize) -> R<()> {
        let mut field = p + 2;
        self.pc = p + 4;
        loop {
            if self.eval_cond(hw)? {
                if self.peek() == tk::THEN {
                    self.pc += 2;
                    if self.peek() == TK_LGO {
                        self.test_point(hw)?;
                        self.pc = self.label_target(hw)?;
                        self.after_jump();
                    }
                }
                return Ok(());
            }
            let d = read_u16(&self.code, field) as usize;
            let target = field + (d & !1);
            if d & 1 != 0 {
                // An Else If: evaluate its condition.
                field = target - 2;
                self.pc = target;
                continue;
            }
            self.pc = target;
            // One-line "Else 100".
            if self.peek() == TK_LGO {
                self.test_point(hw)?;
                self.pc = self.label_target(hw)?;
            }
            self.after_jump();
            return Ok(());
        }
    }

    fn exec_on(&mut self, hw: &mut dyn Host, p: usize) -> R<()> {
        let len = read_u16(&self.code, p + 2) as usize;
        let count = read_u16(&self.code, p + 4) as i32;
        self.pc = p + 6;
        let n = self.eval_int(hw)?;
        let kind = self.next_token();
        let list_start = self.pc;
        let after = list_start + len;
        if n <= 0 || n > count {
            self.pc = after;
            return Ok(());
        }
        self.test_point(hw)?;
        // Skip n-1 entries.
        for _ in 1..n {
            self.skip_expression();
            self.pc += 2; // comma
        }
        match kind {
            tk::GOTO => {
                self.pc = self.label_target(hw)?;
                self.after_jump();
            }
            tk::GOSUB => {
                let target = self.label_target(hw)?;
                self.push_ctl(Ctl::Gosub { ret: after })?;
                self.pc = target;
            }
            tk::PROC => {
                let index = self.proc_operand()?;
                self.call_proc(index, after, Vec::new())?;
            }
            _ => return err(errors::SYNTAX_ERROR),
        }
        Ok(())
    }

    /// Skips one expression without evaluating it.
    pub fn skip_expression(&mut self) {
        let mut depth = 0i32;
        loop {
            let t = self.peek();
            match t {
                TK_EOL | TK_DP => return,
                TK_COMMA | TK_TO | TK_BRA2 | TK_SEMI if depth == 0 => return,
                TK_PAR1 | TK_BRA1 => depth += 1,
                TK_PAR2 => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ if t == TK_ELSE || t == tk::THEN => return,
                _ => {}
            }
            self.pc = self.skip_token(self.pc);
        }
    }

    /// `Name[args]` procedure call at pc.
    fn call_with_args(&mut self, hw: &mut dyn Host) -> R<()> {
        let index = self.proc_operand()?;
        let mut args = Vec::new();
        if self.peek() == TK_BRA1 {
            self.pc += 2;
            loop {
                args.push(self.eval(hw)?);
                match self.next_token() {
                    TK_COMMA => continue,
                    TK_BRA2 => break,
                    _ => return err(errors::SYNTAX_ERROR),
                }
            }
        }
        let ret = self.pc;
        self.call_proc(index, ret, args)
    }

    // ------------------------------------------------------------------
    // Data
    // ------------------------------------------------------------------

    /// Start of the line following the line containing `p`.
    fn line_after(&self, p: usize) -> usize {
        let mut line = 0;
        loop {
            let next = line + self.code[line] as usize * 2;
            if next > p || self.code[line] == 0 {
                return next;
            }
            line = next;
        }
    }

    /// Next Data item, evaluated in the current scope.
    fn next_data(&mut self, hw: &mut dyn Host, ty: u8) -> R<Value> {
        if self.data.item == 0 {
            self.find_data_line()?;
        }
        let saved = self.pc;
        self.pc = self.data.item;
        let t = self.peek();
        let v = if t == TK_COMMA || Self::is_end(t) {
            Value::zero(ty)
        } else {
            match self.eval(hw) {
                Ok(v) => v,
                Err(e) => {
                    self.pc = saved;
                    return Err(e);
                }
            }
        };
        self.data.item = if self.peek() == TK_COMMA { self.pc + 2 } else { 0 };
        self.pc = saved;
        if (ty == 2) != v.is_str() {
            return err(errors::TYPE_MISMATCH);
        }
        Ok(v)
    }

    fn find_data_line(&mut self) -> R<()> {
        let mut line = if self.data.line == 0 { self.data.base.saturating_sub(2) } else { self.data.line };
        if self.scope == 0 && self.data.line == 0 {
            line = 0;
        }
        loop {
            if line + 3 >= self.code.len() || self.code[line] == 0 {
                return err(errors::OUT_OF_DATA);
            }
            let first = self.rd(line + 2);
            let next = line + self.code[line] as usize * 2;
            if first == TK_END_PROC {
                return err(errors::OUT_OF_DATA);
            }
            if first == TK_PROCEDURE {
                // Skip the procedure body.
                let index = self.compiled().procs.iter().position(|pr| pr.pos == line + 2);
                if let Some(i) = index {
                    let end_line = self.compiled().procs[i].end_line;
                    line = end_line + self.code[end_line] as usize * 2;
                    continue;
                }
            }
            let mut q = line + 2;
            if first == TK_LAB {
                q += token_size(&self.code, q);
            }
            if self.rd(q) == TK_DATA {
                self.data.item = q + 4;
                self.data.line = next;
                return Ok(());
            }
            line = next;
        }
    }
}
