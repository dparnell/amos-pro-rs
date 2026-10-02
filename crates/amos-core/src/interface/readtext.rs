//! The text reader (`Read Text`, `IRText` `+Lib.s:14723`): Interface
//! program 1 of the default resource on screen 10, a hypertext zone with
//! its slider and buttons. A text starting with `#HYPn` has active words
//! (`{[keyword]text}`, n per line at most): clicking a word whose keyword
//! is not a number closes the reader and returns the keyword in `Param$`.

use super::engine::{OpenParams, RunResult};
use super::*;
use crate::interp::{Interp, R, err};
use crate::machine::Hardware;

/// `PI_RtSx`, `PI_RtSy`, `PI_RtWy`, `PI_RtSpeed` (`+Interpreter_Config.s:66`).
const RT_SX: i32 = 640;
const RT_SY: i32 = 256;
const RT_WY: i32 = 50;
const RT_SPEED: i32 = 8;
/// Zone of the text.
const Z_TEXT: i32 = 5;

#[derive(Clone, Debug, PartialEq)]
pub enum RtPhase {
    Appear(i32),
    Run,
    Disappear(i32),
}

/// Text reader state.
#[derive(Debug)]
pub struct ReadText {
    pub channel: i64,
    pub phase: RtPhase,
    pub old_screen: Option<usize>,
    pub height: i32,
}

impl Hardware {
    /// `Read Text file$`.
    pub(crate) fn read_text_file(&mut self, it: &mut Interp, name: &[u8]) -> R<()> {
        if self.rt_reentry(it) {
            return self.rt_step(it);
        }
        if name.is_empty() || name.len() >= 108 {
            return err(crate::errors::ILLEGAL_FUNCTION_CALL);
        }
        let text = self.files_read_all(name)?;
        let title = Resource::default_resource().messages.get(20);
        self.rt_start(it, &title, text)
    }

    /// `Read Text title$,address,length`.
    pub(crate) fn read_text_memory(
        &mut self,
        it: &mut Interp,
        title: &[u8],
        addr: i32,
        len: i32,
    ) -> R<()> {
        if self.rt_reentry(it) {
            return self.rt_step(it);
        }
        let mut text = self.banks.peek_bytes(addr as u32, len.max(0) as usize);
        if let Some(z) = text.iter().position(|&c| c == 0) {
            text.truncate(z);
        }
        self.rt_start(it, title, text)
    }

    fn rt_reentry(&self, it: &Interp) -> bool {
        matches!(
            (&self.dialogs.blocking, &it.wait),
            (Some((pos, Blocking::ReadText)), Some(w)) if *pos == it.inst_pos && w.pos == it.inst_pos
        )
    }

    fn rt_start(&mut self, it: &mut Interp, title: &[u8], mut text: Vec<u8>) -> R<()> {
        it.param_s = crate::interp::value::empty_str();
        // Hypertext: "#HYPn" with 1 <= n <= 9 active words per line.
        let mut hyper = 0;
        let mut buffer = (text.len() / 16) * 4 + 2048;
        if text.starts_with(b"#HYP") && text.len() > 4 && (b'1'..=b'9').contains(&text[4]) {
            hyper = (text[4] - b'0') as i32;
            buffer += 2048;
            text.drain(..8.min(text.len()));
        }
        let res = Resource::default_resource();
        let old_screen = self.screens.current;
        let n = crate::gfx::screen::MAX_SCREENS - 2;
        self.dia_rsc_open(&res.graphics, n, RT_SX, RT_SY, 0, None)?;
        let q = self.dialogs.free_quick_channel();
        let prog = res.programs[0].clone();
        self.dialogs.read_text = Some(Box::new(ReadText {
            channel: q,
            phase: RtPhase::Appear(1),
            old_screen,
            height: RT_SY,
        }));
        if let Some(s) = self.screens.get_mut(n) {
            s.pending_display = [None, Some(RT_WY + RT_SY / 2), None, Some(2)];
            s.pending_offset[1] = Some(RT_SY / 2 - 1);
        }
        let ok = self
            .dia_open_channel(OpenParams {
                number: q,
                prog,
                nvar: 8,
                buffer,
                res,
            })
            .is_ok();
        if ok {
            if let Some(i) = self.dialogs.channel_index(q) {
                let v = &mut self.dialogs.channels[i].vars;
                v[2] = DVal::Int(hyper);
                v[0] = DVal::str(&text);
                v[1] = DVal::str(title);
            }
            if !matches!(
                self.dia_run_program(it, q, None, Some(0), Some(0)),
                Ok(RunResult::Value(_))
            ) {
                self.rt_close(it);
                return err(crate::errors::OUT_OF_MEMORY);
            }
        } else {
            self.rt_close(it);
            return err(crate::errors::OUT_OF_MEMORY);
        }
        self.dia_block(it, Blocking::ReadText)
    }

    /// One frame of the reader.
    fn rt_step(&mut self, it: &mut Interp) -> R<()> {
        let Some(rt) = self.dialogs.read_text.as_ref() else {
            self.dia_unblock(it);
            return Ok(());
        };
        let (q, phase, h) = (rt.channel, rt.phase.clone(), rt.height);
        let n = crate::gfx::screen::MAX_SCREENS - 2;
        match phase {
            RtPhase::Appear(d6) => {
                let d6 = (d6 + RT_SPEED).min(h / 2);
                self.rt_app_centre(n, d6, h);
                if let Some(rt) = self.dialogs.read_text.as_mut() {
                    rt.phase = if d6 >= h / 2 {
                        RtPhase::Run
                    } else {
                        RtPhase::Appear(d6)
                    };
                }
            }
            RtPhase::Run => {
                // Test_PaSaut: the automatic tests of every dialog.
                self.dia_auto_test(it, 127)
                    .or_else(crate::machine::inst_dialogs::dia_err)?;
                let mut quit = false;
                if let Ok(DVal::Str(s)) = self.dia_get_value(q, Z_TEXT, 1)
                    && !s.is_empty()
                {
                    it.param_s = crate::interp::value::astr(&s);
                    quit = true;
                }
                match self.dialogs.channel_index(q) {
                    Some(i) if self.dialogs.channels[i].rflags & 1 != 0 => {}
                    _ => quit = true,
                }
                if quit {
                    let _ = self.dia_close_channel(it, q);
                    if let Some(rt) = self.dialogs.read_text.as_mut() {
                        rt.phase = RtPhase::Disappear(h / 2);
                    }
                }
            }
            RtPhase::Disappear(d6) => {
                let d6 = d6 - RT_SPEED;
                if d6 > 0 {
                    self.rt_app_centre(n, d6, h);
                    if let Some(rt) = self.dialogs.read_text.as_mut() {
                        rt.phase = RtPhase::Disappear(d6);
                    }
                } else {
                    self.rt_close(it);
                    self.dia_unblock(it);
                    return Ok(());
                }
            }
        }
        self.dia_block(it, Blocking::ReadText)
    }

    fn rt_app_centre(&mut self, n: usize, d6: i32, h: i32) {
        if let Some(s) = self.screens.get_mut(n) {
            let d6 = d6.max(1);
            s.pending_display[1] = Some(RT_WY + h / 2 - d6);
            s.pending_display[3] = Some(2 * d6);
            s.pending_offset[1] = Some(h / 2 - d6);
        }
    }

    fn rt_close(&mut self, it: &mut Interp) {
        let Some(rt) = self.dialogs.read_text.take() else {
            return;
        };
        let _ = self.dia_close_channel(it, rt.channel);
        let n = crate::gfx::screen::MAX_SCREENS - 2;
        if self.screens.get(n).is_some() {
            self.screens.remove(n);
        }
        if let Some(o) = rt.old_screen {
            let _ = self.screens.activate(o);
        }
    }
}
