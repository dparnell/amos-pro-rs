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

/// Keywords whose handler (here or in the interpreter core) reads its
/// parameters only through one `inst_args` / `func_args` call made before
/// anything else, and otherwise never looks at the token stream, `pc` or
/// `code`: compiled code can call them with [`Interp::preset_args`] instead
/// of a token stream. Keywords without parameters qualify when they read no
/// tokens at all (call them without a preset: their handler may not read
/// parameters). Reserved variables (X Mouse, Y Mouse, Timer) qualify when
/// read; their assignment (`Timer=0`) evaluates its value from the code.
/// Each handler was checked by hand; anything unsure is left out (waiting
/// instructions, which run again from the code, are left out).
pub fn plain_args(kw: Keyword) -> bool {
    use crate::tokens::tk::*;
    kw.slot == 0
        && matches!(
            kw.token,
            // Drawing (inst_draw.rs).
            PLOT | PLOT_2
                | DRAW_TO
                | DRAW
                | ELLIPSE
                | CIRCLE
                | BAR
                | BOX
                | PAINT
                | PAINT_2
                | GR_LOCATE
                | TEXT
                | INK
                | INK_2
                | INK_3
                | POINT
                // Text (inst_text.rs).
                | LOCATE
                | PEN
                | PAPER
                | CURS_PEN
                | HOME
                | CURS_OFF
                | CURS_ON
                | MOUSE_ZONE
                // Screens (inst_screen.rs).
                | CLS
                | CLS_2
                | CLS_3
                | COLOUR
                | COLOUR_2
                | SCREEN_DISPLAY
                | SCREEN_OFFSET
                | SCIN
                | SCIN_2
                | X_HARD
                | X_HARD_2
                | Y_HARD
                | Y_HARD_2
                | X_SCREEN
                | X_SCREEN_2
                | Y_SCREEN
                | Y_SCREEN_2
                // Bobs and sprites (inst_sprites.rs).
                | BOB
                | SPRITE
                | PASTE_BOB
                | PASTE_ICON
                | X_BOB
                | Y_BOB
                | I_BOB
                | X_SPRITE
                | Y_SPRITE
                | I_SPRITE
                // Input (inst_input.rs).
                | X_MOUSE
                | Y_MOUSE
                | MOUSE_KEY
                | MOUSE_CLICK
                | JOY
                | JUP
                | JDOWN
                | JLEFT
                | JRIGHT
                | FIRE
                | KEY_STATE
                | TIMER
                | INKEY_S
        )
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
            || self.menus_instruction(it, kw)?
            || self.dialogs_instruction(it, kw)?
            || self.copper_instruction(it, kw)?
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
        if let Some(v) = self.dialogs_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.copper_function(it, kw)? {
            return Ok(v);
        }
        if let Some(v) = self.menus_function(it, kw)? {
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
        self.screen_test_point(it)?;
        self.dialogs_test_point(it)?;
        self.menus_test_point(it)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Machine;
    use crate::display::render_rgba;
    use crate::interp::RunState;
    use crate::tokens::TokenKind;

    const SETUP: &str = "Screen Open 0,320,200,16,Lowres\nCls 0\nInk 3 : Bar 0,0 To 15,15 : Get Bob 1,0,0 To 16,16\n\
Ink 5 : Circle 8,8,6 : Get Bob 2,0,0 To 16,16\nReserve Zone 5 : Set Zone 1,0,0 To 100,100\nBob 3,50,50,1\n";

    fn run(src: &str) -> Machine {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).expect(src);
        let mut m = Machine::new();
        m.run_program(&prg).expect(src);
        for _ in 0..5 {
            m.vbl();
            if !m.interp.running {
                break;
            }
        }
        m
    }

    fn word_case(name: &str) -> String {
        let words: Vec<String> = name
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            })
            .collect();
        words.join(" ")
    }

    /// Calling the handler of every `plain_args` keyword with preset values
    /// (and pc somewhere else) does exactly what the call in the code does.
    #[test]
    fn plain_keywords_run_the_same_with_preset_values() {
        let mut checked = 0;
        for d in crate::tokens::MAIN.iter() {
            let kw = Keyword {
                slot: 0,
                token: d.token,
            };
            if !plain_args(kw) {
                continue;
            }
            let func = !matches!(d.kind(), TokenKind::Instruction);
            let sig = d.param_types().as_bytes();
            let n = sig.len().div_ceil(2);
            let name = word_case(d.name);
            for set in 0..2 {
                let ints = [[1, 20, 30, 1, 2, 3], [2, 10, 100, 90, 1, 1]][set];
                let mut text = String::new();
                let mut values = Vec::new();
                for k in 0..n {
                    if k > 0 {
                        text += if sig[2 * k - 1] == b't' { " To " } else { "," };
                    }
                    match sig[2 * k] {
                        b'2' => {
                            text += "\"Hi\"";
                            values.push(Some(Value::str(b"Hi")));
                        }
                        _ => {
                            text += &ints[k].to_string();
                            values.push(Some(Value::Int(ints[k])));
                        }
                    }
                }
                let call = match (func, n) {
                    (true, 0) => format!("Print {name}\n"),
                    (true, _) => format!("Print {name}({text})\n"),
                    (false, _) => format!("{name} {text}\n"),
                };
                let mut a = run(&format!("{SETUP}{call}"));
                let mut b = run(SETUP);
                // The log of A, without its stop message.
                let mut a_log = a.hw.log.clone();
                a_log.pop();
                let b_log_start = b.hw.log.len();
                let want = match &a.state {
                    RunState::Stopped(info) => format!("{:?}", info.reason),
                    s => panic!("{call}: {s:?}"),
                };
                if n > 0 {
                    b.interp.preset_args(&values);
                }
                let got = if func {
                    match b.hw.function(&mut b.interp, kw) {
                        Ok(v) => {
                            let mut t = b.interp.value_text(&v);
                            t.extend_from_slice(b"\r\n");
                            // What `Print` does with the value.
                            b.hw.print(&mut b.interp, &t).unwrap();
                            "Stop(End)".to_string()
                        }
                        Err(e) => format!("{e:?}"),
                    }
                } else {
                    match b.hw.instruction(&mut b.interp, kw) {
                        Ok(()) => "Stop(End)".to_string(),
                        Err(e) => format!("{e:?}"),
                    }
                };
                assert_eq!(want, got, "{call}");
                // (B's setup ended with its own stop message.)
                assert_eq!(a_log[..], b.hw.log[b_log_start..], "{call}");
                a.vbl();
                b.vbl();
                assert!(
                    render_rgba(&a.frame()) == render_rgba(&b.frame()),
                    "{call}: display differs"
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "{checked}");
    }
}
