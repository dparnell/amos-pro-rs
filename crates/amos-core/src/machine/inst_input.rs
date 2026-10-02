//! Keyboard, mouse and joystick instructions.

use super::Hardware;
use crate::errors;
use crate::input::KeyPress;
use crate::interp::stmt::InputState as LineInput;
use crate::interp::value::{Value, astr, empty_str};
use crate::interp::{Exc, Host, Interp, R, WaitKind, err};
use crate::tokens::{Keyword, TokenKind, tk};

/// Wait identifiers for `WaitKind::Other`.
const WAIT_INPUT_S: u32 = 0x1001;

impl Hardware {
    pub(crate) fn input_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        let reserved = kw.def().is_some_and(|d| d.kind() == TokenKind::ReservedVariable);
        match kw.token {
            CLEAR_KEY => self.input.clear_keys(),
            WAIT_KEY => {
                // Waits for a key press and removes it from the buffer.
                if self.input.inkey().is_none() {
                    return Err(Exc::Block);
                }
            }
            PUT_KEY => {
                let a = it.inst_args(self, kw)?;
                self.input.put_key(&a.str(0));
            }
            KEY_SPEED => {
                it.inst_args(self, kw)?;
            }
            KEY_S if reserved => {
                // Key$(n)=a$
                it.expect(crate::tokens::TK_PAR1)?;
                let n = it.eval_int(self)?;
                it.expect(crate::tokens::TK_PAR2)?;
                it.expect(OP_EQ)?;
                let s = it.eval_str(self)?;
                if !(1..=20).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                // A backquote stands for a new line.
                let mut v = Vec::new();
                for &c in s.iter().take(23) {
                    if c == b'`' {
                        v.extend_from_slice(b"\r\n");
                    } else {
                        v.push(c);
                    }
                }
                self.input.function_keys[n as usize - 1] = v;
            }
            X_MOUSE | Y_MOUSE if reserved => {
                it.expect(OP_EQ)?;
                let v = it.eval_int(self)?;
                if kw.token == X_MOUSE {
                    self.input.set_mouse(Some(v), None);
                } else {
                    self.input.set_mouse(None, Some(v));
                }
            }
            LIMIT_MOUSE => self.input.limit_mouse(None),
            LIMIT_MOUSE_2 => {
                let n = it.inst_args(self, kw)?.int(0);
                let s = self.screens.get(n.max(0) as usize).ok_or(Exc::Error(errors::SCREEN_NOT_OPENED))?;
                let (x, y) = (s.display_x, s.display_y);
                let w = s.display_w as i32;
                let h = if s.lace { s.display_h as i32 / 2 } else { s.display_h as i32 };
                self.input.limit_mouse(Some((x, y, x + w - 1, y + h - 1)));
            }
            LIMIT_MOUSE_3 => {
                let a = it.inst_args(self, kw)?;
                self.input.limit_mouse(Some((a.int(0), a.int(1), a.int(2), a.int(3))));
            }
            TIMER if reserved => {
                it.expect(OP_EQ)?;
                self.timer = it.eval_int(self)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn input_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            INKEY_S => match self.input.inkey() {
                Some(k) => Value::Str(astr(&[k.ascii])),
                None => Value::Str(empty_str()),
            },
            SCANCODE => {
                let v = self.input.scancode;
                self.input.scancode = 0;
                Value::Int(v as i32)
            }
            SCANSHIFT => Value::Int(self.input.scanshift as i32),
            KEY_STATE => {
                let n = it.func_args(self, kw)?.int(0);
                if !(0..128).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Int(if self.input.key_down(n as u8) { -1 } else { 0 })
            }
            KEY_SHIFT => Value::Int(self.input.shifts() as i32),
            KEY_S => {
                let n = it.func_args(self, kw)?.int(0);
                if !(1..=20).contains(&n) {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                Value::Str(self.input.function_keys[n as usize - 1].clone().into())
            }
            SCAN_S | SCAN_S_2 => {
                let a = it.func_args(self, kw)?;
                let shift = a.opt(1).unwrap_or(0);
                Value::Str(astr(&[1, shift as u8, a.int(0) as u8, 0]))
            }
            JOY | JUP | JDOWN | JLEFT | JRIGHT | FIRE => {
                let port = it.func_args(self, kw)?.int(0);
                let j = self.input.joy_state(port);
                let bit = match kw.token {
                    JUP => 1,
                    JDOWN => 2,
                    JLEFT => 4,
                    JRIGHT => 8,
                    FIRE => 16,
                    _ => return Ok(Some(Value::Int(j))),
                };
                Value::Int(if j & bit != 0 { -1 } else { 0 })
            }
            X_MOUSE => Value::Int(self.input.mouse_x),
            Y_MOUSE => Value::Int(self.input.mouse_y),
            MOUSE_KEY => Value::Int(self.input.mouse_buttons as i32),
            MOUSE_CLICK => Value::Int(self.input.take_clicks() as i32),
            TIMER => Value::Int(self.timer),
            INPUT_S => {
                // Input$(n): waits for n characters.
                let n = it.func_args(self, kw)?.int(0);
                if n < 0 {
                    return err(errors::ILLEGAL_FUNCTION_CALL);
                }
                let mut got = match it.wait_state(|| WaitKind::Other(WAIT_INPUT_S, 0)) {
                    WaitKind::Other(_, v) => *v,
                    _ => 0,
                };
                // Characters read so far are kept in the key buffer until n
                // are available, so the instruction can be restarted.
                got = got.max(self.input.buffer.len() as u64);
                if (got as i32) < n {
                    return Err(Exc::Block);
                }
                it.clear_wait();
                let s: Vec<u8> = (0..n).filter_map(|_| self.input.inkey()).map(|k| k.ascii).collect();
                Value::Str(s.into())
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    pub(crate) fn input_event(&mut self, event: crate::input::InputEvent) {
        self.input.event(event);
    }

    pub(crate) fn input_vbl(&mut self) {}

    /// Line input for Input / Line Input: echoes typed characters in the
    /// current window; returns the line when Return is pressed.
    pub(crate) fn input_read_line(&mut self, it: &mut Interp, state: &mut LineInput) -> R<Option<Vec<u8>>> {
        while let Some(KeyPress { ascii, raw, .. }) = self.input.inkey() {
            match ascii {
                13 => return Ok(Some(std::mem::take(&mut state.buffer))),
                8 => {
                    if state.buffer.pop().is_some() {
                        // Cursor left, space, cursor left.
                        self.print(it, &[29, b' ', 29])?;
                    }
                }
                0 => {
                    let _ = raw;
                }
                c if c >= 32 => {
                    if state.buffer.len() < 255 {
                        state.buffer.push(c);
                        self.print(it, &[c])?;
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }
}
