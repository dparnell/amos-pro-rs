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

type InstructionHandler = fn(&mut Hardware, &mut Interp, Keyword) -> R<bool>;
type FunctionHandler = fn(&mut Hardware, &mut Interp, Keyword) -> R<Option<Value>>;

/// The instruction handlers from `FIRST_DIRECT` on, in chain order. (Only
/// these: taking the address of the first ones would stop them from being
/// inlined into the chain, which then gets slower.)
const LATE_INSTRUCTION_HANDLERS: [InstructionHandler; 6] = [
    Hardware::banks_instruction,
    Hardware::files_instruction,
    Hardware::menus_instruction,
    Hardware::dialogs_instruction,
    Hardware::copper_instruction,
    Hardware::system_instruction,
];

/// The function handlers from `FIRST_DIRECT` on, in chain order.
const LATE_FUNCTION_HANDLERS: [FunctionHandler; 6] = [
    Hardware::banks_function,
    Hardware::files_function,
    Hardware::dialogs_function,
    Hardware::copper_function,
    Hardware::menus_function,
    Hardware::system_function,
];

/// Number of subsystem handlers (instructions and functions).
#[cfg(test)]
const HANDLERS: u8 = 12;

/// Handlers from this one on (1-based, chain order) are remembered per
/// keyword and called directly; the ones before are tried in turn.
const FIRST_DIRECT: u8 = 7;

/// Keyword -> number (1-based, 0 = not met yet) of the subsystem handler
/// that accepted it, for instructions and functions: learnt from the chain
/// the first time a keyword is met.
#[derive(Default)]
pub struct DispatchCache {
    inst: [Vec<u8>; crate::tokens::EXTENSION_SLOTS],
    func: [Vec<u8>; crate::tokens::EXTENSION_SLOTS],
}

impl DispatchCache {
    #[inline]
    fn get(&self, kw: Keyword, func: bool) -> u8 {
        let t = if func { &self.func } else { &self.inst };
        t.get(kw.slot as usize)
            .and_then(|v| v.get(kw.token as usize))
            .copied()
            .unwrap_or(0)
    }

    fn set(&mut self, kw: Keyword, func: bool, h: u8) {
        let t = if func { &mut self.func } else { &mut self.inst };
        if let Some(v) = t.get_mut(kw.slot as usize) {
            let i = kw.token as usize;
            if v.len() <= i {
                v.resize(i + 1, 0);
            }
            v[i] = h;
        }
    }
}

#[cfg(test)]
impl Hardware {
    /// Instruction handler number `h` (1-based), in the order of the chain.
    fn instruction_by(&mut self, h: u8, it: &mut Interp, kw: Keyword) -> R<bool> {
        match h {
            1 => self.screen_instruction(it, kw),
            2 => self.text_instruction(it, kw),
            3 => self.draw_instruction(it, kw),
            4 => self.sprites_instruction(it, kw),
            5 => self.sound_instruction(it, kw),
            6 => self.input_instruction(it, kw),
            7..=HANDLERS => LATE_INSTRUCTION_HANDLERS[(h - FIRST_DIRECT) as usize](self, it, kw),
            _ => Ok(false),
        }
    }

    /// Function handler number `h` (1-based), in the order of the chain.
    fn function_by(&mut self, h: u8, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        match h {
            1 => self.screen_function(it, kw),
            2 => self.text_function(it, kw),
            3 => self.draw_function(it, kw),
            4 => self.sprites_function(it, kw),
            5 => self.sound_function(it, kw),
            6 => self.input_function(it, kw),
            7..=HANDLERS => LATE_FUNCTION_HANDLERS[(h - FIRST_DIRECT) as usize](self, it, kw),
            _ => Ok(None),
        }
    }
}

impl Host for Hardware {
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        // The subsystems in priority order. The first ones are inlined here
        // (the compiler merges their keyword tests). The later ones are big
        // functions called one after the other: the one that accepted a
        // keyword is remembered and called directly the next time. Each
        // handler accepts or refuses a keyword by its value alone (tested
        // below), so the ones before it would refuse it again.
        if self.screen_instruction(it, kw)?
            || self.text_instruction(it, kw)?
            || self.draw_instruction(it, kw)?
            || self.sprites_instruction(it, kw)?
            || self.sound_instruction(it, kw)?
            || self.input_instruction(it, kw)?
        {
            return Ok(());
        }
        let known = self.dispatch.get(kw, false);
        if known >= FIRST_DIRECT
            && LATE_INSTRUCTION_HANDLERS[(known - FIRST_DIRECT) as usize](self, it, kw)?
        {
            return Ok(());
        }
        for (h, f) in (FIRST_DIRECT..).zip(LATE_INSTRUCTION_HANDLERS) {
            if f(self, it, kw)? {
                self.dispatch.set(kw, false, h);
                return Ok(());
            }
        }
        not_implemented(kw)
    }

    fn function(&mut self, it: &mut Interp, kw: Keyword) -> R<Value> {
        // As `instruction`.
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
        let known = self.dispatch.get(kw, true);
        if known >= FIRST_DIRECT
            && let Some(v) = LATE_FUNCTION_HANDLERS[(known - FIRST_DIRECT) as usize](self, it, kw)?
        {
            return Ok(v);
        }
        for (h, f) in (FIRST_DIRECT..).zip(LATE_FUNCTION_HANDLERS) {
            if let Some(v) = f(self, it, kw)? {
                self.dispatch.set(kw, true, h);
                return Ok(v);
            }
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

    /// Number of the first subsystem of the chain that accepts `kw` (does
    /// not refuse it: accepting includes failing or panicking on the probe
    /// parameters), on a fresh machine prepared by `setup`.
    fn probe(kw: Keyword, func: bool, setup: &str, dir: &std::path::Path) -> Option<u8> {
        let prg = crate::tokenise::tokenise_program(setup.as_bytes()).unwrap();
        let mut m = Machine::new();
        m.hw.files.set_native_root(dir);
        m.run_program(&prg).unwrap();
        m.vbl();
        let sig = kw.def().map_or("", |d| d.param_types()).as_bytes();
        let values: Vec<Option<Value>> = sig
            .iter()
            .step_by(2)
            .map(|t| match t {
                b'2' => Some(Value::str(b"")),
                b'1' | b'5' => Some(Value::Float(1.0)),
                _ => Some(Value::Int(1)),
            })
            .collect();
        for h in 1..=HANDLERS {
            if !values.is_empty() {
                m.interp.preset_args(&values);
            }
            let (hw, it) = (&mut m.hw, &mut m.interp);
            let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if func {
                    matches!(hw.function_by(h, it, kw), Ok(None))
                } else {
                    matches!(hw.instruction_by(h, it, kw), Ok(false))
                }
            }))
            .unwrap_or(false);
            if !refused {
                return Some(h);
            }
        }
        None
    }

    /// The dispatch cache relies on every subsystem accepting or refusing a
    /// keyword by its value alone: for every keyword of every extension, as
    /// an instruction and as a function, the chain picks the same subsystem
    /// in two very different machine states, and the dispatcher (cache
    /// learnt through `Host::instruction` / `Host::function`) uses it.
    #[test]
    fn dispatch_picks_the_handler_of_the_chain_for_every_keyword() {
        let dir = std::env::temp_dir().join(format!("amos-dispatch-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let other = "Screen Open 2,640,200,4,Hires\nScreen Close 0\nDouble Buffer\nReserve As Work 10,64\n\
Reserve Zone 3\nDegree\nGet Bob 1,0,0 To 16,16\nWind Open 1,0,0,20,10\n";
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let mut checked = 0;
        for (slot, table) in crate::tokens::EXTENSIONS.iter().enumerate() {
            for d in table.iter() {
                let kw = Keyword {
                    slot: slot as u8,
                    token: d.token,
                };
                for func in [false, true] {
                    let a = probe(kw, func, "", &dir);
                    let b = probe(kw, func, other, &dir);
                    assert_eq!(a, b, "{} {:?} (function: {func})", d.name, kw);
                    let Some(h) = a else { continue };
                    // The dispatcher on a machine where the keyword is new
                    // learns this handler when the call succeeds.
                    let prg = crate::tokenise::tokenise_program(b"").unwrap();
                    let mut m = Machine::new();
                    m.hw.files.set_native_root(&dir);
                    m.run_program(&prg).unwrap();
                    let (hw, it) = (&mut m.hw, &mut m.interp);
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if func {
                            let _ = Host::function(hw, it, kw);
                        } else {
                            let _ = Host::instruction(hw, it, kw);
                        }
                    }));
                    let learnt = m.hw.dispatch.get(kw, func);
                    assert!(
                        learnt == 0 || learnt == h,
                        "{} {:?}: {learnt} vs {h}",
                        d.name,
                        kw
                    );
                    checked += 1;
                }
            }
        }
        std::panic::set_hook(prev_hook);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(checked > 700, "{checked}");
    }
}
