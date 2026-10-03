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
                | SCANCODE
                | LIMIT_MOUSE
                | LIMIT_MOUSE_2
                | LIMIT_MOUSE_3
                // More screens and zones (inst_screen.rs, inst_text.rs).
                | SCREEN_2
                | SCREEN_TO_FRONT
                | SCREEN_TO_FRONT_2
                | SCREEN_TO_BACK
                | SCREEN_TO_BACK_2
                | SCREEN_HIDE
                | SCREEN_HIDE_2
                | SCREEN_SHOW
                | SCREEN_SHOW_2
                | ZONE
                | ZONE_2
                | HZONE
                | HZONE_2
                // Menus and dialogs (inst_menus.rs, inst_dialogs.rs: Dialog
                // checks which program runs before reading its parameter,
                // from `Interp::prg`, not from the code).
                | CHOICE
                | CHOICE_2
                | DIALOG
                // Memory (inst_banks.rs). Varptr'd variables are mapped
                // through the interpreter's variables (`var_maps`).
                | PEEK
                | DEEK
                | LEEK
                | POKE
                | DOKE
                | LOKE
                // More (inst_screen.rs, inst_input.rs, inst_sprites.rs,
                // inst_text.rs).
                | SCREEN
                | KEY_SHIFT
                | BOB_COL
                | BOB_COL_2
                | BOBSPRITE_COL
                | BOBSPRITE_COL_2
                | SPRITE_COL
                | SPRITE_COL_2
                | SPRITEBOB_COL
                | SPRITEBOB_COL_2
                | CENTRE
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
            1 => self.input_function(it, kw),
            2 => self.screen_function(it, kw),
            3 => self.text_function(it, kw),
            4 => self.draw_function(it, kw),
            5 => self.sprites_function(it, kw),
            6 => self.sound_function(it, kw),
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
        // As `instruction`. The input functions come first: Mouse Key, X
        // Mouse, Inkey$... are by far the most called (busy waiting loops).
        // (No keyword is accepted by two handlers: the order only changes
        // the speed, see `every_keyword_has_at_most_one_handler`.)
        if let Some(v) = self.input_function(it, kw)? {
            return Ok(v);
        }
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

    /// Handlers (1-based, chain order) that accept `kw` (`probe` without
    /// stopping at the first).
    fn acceptors(kw: Keyword, func: bool, dir: &std::path::Path) -> Vec<u8> {
        let prg = crate::tokenise::tokenise_program(b"").unwrap();
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
        let mut out = Vec::new();
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
                out.push(h);
            }
        }
        out
    }

    /// No keyword is accepted by two subsystem handlers, as an instruction
    /// or as a function: the order of the chain only changes its speed.
    #[test]
    fn every_keyword_has_at_most_one_handler() {
        let dir = std::env::temp_dir().join(format!("amos-dispatch-one-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
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
                    let a = acceptors(kw, func, &dir);
                    let name = d.name;
                    assert!(a.len() <= 1, "{name} {kw:?} (function: {func}): {a:?}");
                    checked += a.len();
                }
            }
        }
        std::panic::set_hook(prev_hook);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(checked > 700, "{checked}");
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

#[cfg(test)]
mod omitted_parameters {
    use super::*;
    use crate::Machine;

    /// Every keyword with one parameter omitted (EntNul) at a time, as an
    /// instruction and as a function: no arithmetic overflow (debug builds
    /// panic where release builds wrap) or other panic.
    #[test]
    fn no_panic_with_omitted_parameters() {
        let dir = std::env::temp_dir().join(format!("amos-omit-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut bad = Vec::new();
        for (slot, table) in crate::tokens::EXTENSIONS.iter().enumerate() {
            for d in table.iter() {
                let kw = Keyword {
                    slot: slot as u8,
                    token: d.token,
                };
                let sig = d.param_types().as_bytes();
                let n = sig.len().div_ceil(2);
                // Memory range keywords: an omitted end address (EntNul)
                // means a 2 GB range, as on the Amiga (slow, not wrong).
                if n == 0 || matches!(d.name, "ssave" | "bsave" | "fill" | "copy" | "hunt") {
                    continue;
                }
                for omit in 0..n {
                    for func in [false, true] {
                        let prg =
                            crate::tokenise::tokenise_program(b"Screen Open 1,320,200,16,Lowres\n")
                                .unwrap();
                        let mut m = Machine::new();
                        m.hw.files.set_native_root(&dir);
                        m.run_program(&prg).unwrap();
                        m.vbl();
                        let values: Vec<Option<Value>> = (0..n)
                            .map(|k| {
                                (k != omit).then(|| match sig[2 * k] {
                                    b'2' => Value::str(b"a"),
                                    b'1' | b'5' => Value::Float(1.0),
                                    _ => Value::Int(1),
                                })
                            })
                            .collect();
                        m.interp.preset_args(&values);
                        let (hw, it) = (&mut m.hw, &mut m.interp);
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if func {
                                let _ = Host::function(hw, it, kw);
                            } else {
                                let _ = Host::instruction(hw, it, kw);
                            }
                        }));
                        if let Err(e) = r {
                            let msg = e
                                .downcast_ref::<String>()
                                .cloned()
                                .or(e.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_default();
                            bad.push(format!(
                                "{} (param {omit} omitted, function {func}): {msg}",
                                d.name
                            ));
                        }
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(bad.is_empty(), "{bad:#?}");
    }

    /// `Scin(,y)`: the omitted X is EntNul, whose low word (0) is the X
    /// coordinate (`GetSIn` works on words): no screen there.
    #[test]
    fn scin_and_hzone_with_an_omitted_coordinate() {
        let src = "Reserve Zone 2 : Set Zone 1,0,0 To 100,100\nV=Scin(,60) : W=Scin(130,) : Z=Hzone(,60) : S=Scin(130,60)\n\
Print V;W;Z;S\n";
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let mut m = Machine::new();
        m.run_program(&prg).unwrap();
        m.vbl();
        let out: String = m.hw.log.concat();
        assert_eq!(out, "-2147483648-2147483648 0 0\r\nEnd");
    }
}

/// The typed keyword functions do what the token path does.
#[cfg(test)]
mod typed_keywords {
    use super::*;
    use crate::Machine;
    use crate::display::render_rgba;
    use crate::interp::RunState;
    use crate::tokens::tk;

    const SETUP: &str = "Screen Open 0,320,200,16,Lowres\nCls 0\nReserve As Work 10,64\n\
Reserve Zone 5 : Set Zone 1,0,0 To 100,100 : Set Zone 2,40,40 To 60,60\n";

    type Typed = Box<dyn FnOnce(&mut Hardware, &mut Interp) -> R<Option<i32>>>;

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

    fn call_with(m: &mut Machine, token: u16, args: &[i32]) -> String {
        let v: Vec<Option<Value>> = args.iter().map(|&a| Some(Value::Int(a))).collect();
        call_values(m, token, v)
    }

    fn call_values(m: &mut Machine, token: u16, v: Vec<Option<Value>>) -> String {
        let kw = Keyword { slot: 0, token };
        if !v.is_empty() {
            m.interp.preset_args(&v);
        }
        let r = if crate::tokens::lookup(token)
            .is_some_and(|d| matches!(d.kind(), crate::tokens::TokenKind::Instruction))
        {
            m.hw.instruction(&mut m.interp, kw).map(|()| String::new())
        } else {
            m.hw.function(&mut m.interp, kw).map(|v| format!("{v:?}"))
        };
        format!("{r:?}")
    }

    /// Shows the state the keywords change: text cursor, pens, graphic ink,
    /// mouse limits and the start of bank 10.
    fn reveal(m: &mut Machine) -> String {
        let mut out = String::new();
        let _ = m.hw.print(&mut m.interp, b"Ab\r\n");
        for n in 1..=2 {
            for t in [
                tk::X_BOB,
                tk::Y_BOB,
                tk::I_BOB,
                tk::X_SPRITE,
                tk::Y_SPRITE,
                tk::I_SPRITE,
            ] {
                out += &call_with(m, t, &[n]);
            }
        }
        out += &call_with(m, tk::DRAW_TO, &[310, 195]);
        out += &call_with(m, tk::SET_PAINT, &[1]);
        out += &call_with(m, tk::BAR, &[200, 100, 230, 130]);
        out += &call_with(m, tk::PLOT, &[240, 100]);
        out += &call_with(m, tk::GR_WRITING, &[1]);
        let text = vec![
            Some(Value::Int(200)),
            Some(Value::Int(150)),
            Some(Value::str(b"Gr")),
        ];
        out += &call_values(m, tk::TEXT, text);
        m.hw.input.set_mouse(Some(100_000), Some(100_000));
        out += &format!(" mouse {} {}", m.hw.input.mouse_x, m.hw.input.mouse_y);
        m.hw.input.set_mouse(Some(-100_000), Some(-100_000));
        out += &format!(" {} {}", m.hw.input.mouse_x, m.hw.input.mouse_y);
        m.interp.preset_args(&[Some(Value::Int(10))]);
        match m.hw.function(
            &mut m.interp,
            Keyword {
                slot: 0,
                token: tk::START,
            },
        ) {
            Ok(Value::Int(a)) => out += &format!(" mem {}", m.hw.leek(&mut m.interp, a)),
            r => panic!("{r:?}"),
        }
        out
    }

    /// `call` (a statement, or `Print <function call>`) after the setup in
    /// a program, against `typed` called directly after the setup.
    fn check(setup: &str, call: &str, typed: Typed) {
        let mut a = run(&format!("{SETUP}{setup}\n{call}\n"));
        let mut b = run(&format!("{SETUP}{setup}\n"));
        let mut a_log = a.hw.log.clone();
        a_log.pop();
        let b_log_start = b.hw.log.len();
        let want = match &a.state {
            RunState::Stopped(info) => format!("{:?}", info.reason),
            s => panic!("{call}: {s:?}"),
        };
        let got = match typed(&mut b.hw, &mut b.interp) {
            Ok(v) => {
                if let Some(v) = v {
                    let mut t = b.interp.value_text(&Value::Int(v));
                    t.extend_from_slice(b"\r\n");
                    b.hw.print(&mut b.interp, &t).unwrap();
                }
                "Stop(End)".to_string()
            }
            Err(e) => format!("{e:?}"),
        };
        assert_eq!(want, got, "{call}");
        // B's log: what its setup printed (without its stop message), then
        // what the typed call printed.
        let mut b_log = b.hw.log[..b_log_start.saturating_sub(1)].to_vec();
        b_log.extend_from_slice(&b.hw.log[b_log_start..]);
        assert_eq!(a_log, b_log, "{call}");
        assert_eq!(reveal(&mut a), reveal(&mut b), "{call}");
        a.vbl();
        b.vbl();
        assert!(
            render_rgba(&a.frame()) == render_rgba(&b.frame()),
            "{call}: display differs"
        );
    }

    fn inst(f: impl FnOnce(&mut Hardware, &mut Interp) -> R<()> + 'static) -> Typed {
        Box::new(move |hw, it| f(hw, it).map(|()| None))
    }

    fn func(f: impl FnOnce(&mut Hardware, &mut Interp) -> R<i32> + 'static) -> Typed {
        Box::new(move |hw, it| f(hw, it).map(Some))
    }

    #[test]
    fn text_keywords() {
        check("", "Locate 3,4", inst(|hw, _| hw.locate(Some(3), Some(4))));
        check("", "Locate ,4", inst(|hw, _| hw.locate(None, Some(4))));
        check("", "Locate 3,", inst(|hw, _| hw.locate(Some(3), None)));
        check(
            "",
            "Locate 50,4",
            inst(|hw, _| hw.locate(Some(50), Some(4))),
        );
        check("", "Pen 5", inst(|hw, _| hw.pen(5)));
        check("", "Pen 300", inst(|hw, _| hw.pen(300)));
        check("", "Paper 2", inst(|hw, _| hw.paper(2)));
        check("Screen Close 0", "Pen 2", inst(|hw, _| hw.pen(2)));
    }

    #[test]
    fn drawing_keywords() {
        check("", "Ink 3", inst(|hw, _| hw.ink(Some(3), None, None)));
        check("", "Ink 3,4", inst(|hw, _| hw.ink(Some(3), Some(4), None)));
        check("", "Ink ,4,5", inst(|hw, _| hw.ink(None, Some(4), Some(5))));
        check(
            "",
            "Plot 10,20",
            inst(|hw, _| hw.plot(Some(10), Some(20), None)),
        );
        check(
            "",
            "Plot 10,20,6",
            inst(|hw, _| hw.plot(Some(10), Some(20), Some(6))),
        );
        check(
            "Plot 5,5",
            "Plot ,20",
            inst(|hw, _| hw.plot(None, Some(20), None)),
        );
        check(
            "",
            "Plot 10,20,-1",
            inst(|hw, _| hw.plot(Some(10), Some(20), Some(-1))),
        );
        check(
            "Plot 5,5",
            "Draw To 60,70",
            inst(|hw, _| hw.draw_to(Some(60), Some(70))),
        );
        check(
            "Plot 5,5",
            "Draw To ,70",
            inst(|hw, _| hw.draw_to(None, Some(70))),
        );
        check(
            "",
            "Draw 1,2 To 30,40",
            inst(|hw, _| hw.draw(Some(1), Some(2), Some(30), Some(40))),
        );
        check(
            "Plot 5,5",
            "Draw ,2 To 30,",
            inst(|hw, _| hw.draw(None, Some(2), Some(30), None)),
        );
        check(
            "",
            "Box 10,10 To 50,40",
            inst(|hw, _| hw.box_(10, 10, 50, 40)),
        );
        check(
            "",
            "Bar 10,10 To 50,40",
            inst(|hw, _| hw.bar(10, 10, 50, 40)),
        );
        check(
            "",
            "Bar 50,10 To 10,40",
            inst(|hw, _| hw.bar(50, 10, 10, 40)),
        );
        check(
            "",
            "Circle 50,50,20",
            inst(|hw, _| hw.circle(Some(50), Some(50), 20)),
        );
        check(
            "Plot 80,80",
            "Circle ,,20",
            inst(|hw, _| hw.circle(None, None, 20)),
        );
        check(
            "",
            "Circle 50,50,0",
            inst(|hw, _| hw.circle(Some(50), Some(50), 0)),
        );
        check(
            "Ink 7 : Plot 5,6",
            "Print Point(5,6)",
            func(|hw, _| hw.point(Some(5), Some(6))),
        );
        check(
            "Plot 5,6",
            "Print Point(400,6)",
            func(|hw, _| hw.point(Some(400), Some(6))),
        );
        check(
            "Screen Close 0",
            "Plot 1,1",
            inst(|hw, _| hw.plot(Some(1), Some(1), None)),
        );
    }

    #[test]
    fn screen_and_mouse_keywords() {
        check(
            "",
            "Limit Mouse",
            inst(|hw, _| {
                hw.limit_mouse();
                Ok(())
            }),
        );
        check("", "Limit Mouse 0", inst(|hw, _| hw.limit_mouse_screen(0)));
        check("", "Limit Mouse 1", inst(|hw, _| hw.limit_mouse_screen(1)));
        check(
            "",
            "Limit Mouse 10,20 To 100,90",
            inst(|hw, _| {
                hw.limit_mouse_area(10, 20, 100, 90);
                Ok(())
            }),
        );
        let two = "Screen Open 1,320,100,4,Lowres : Screen 0";
        check(
            two,
            "Screen To Front",
            inst(|hw, _| hw.screen_to_front(None)),
        );
        check(
            two,
            "Screen To Front 1",
            inst(|hw, _| hw.screen_to_front(Some(1))),
        );
        check(
            two,
            "Screen To Front 3",
            inst(|hw, _| hw.screen_to_front(Some(3))),
        );
        check(
            two,
            "Screen To Front 9",
            inst(|hw, _| hw.screen_to_front(Some(9))),
        );
        check(
            "Colour 3,$123",
            "Print Colour(3)",
            func(|hw, _| hw.colour_fn(3)),
        );
        check(
            "Colour 3,$123",
            "Print Colour(35)",
            func(|hw, _| hw.colour_fn(35)),
        );
        check(
            "",
            "Print X Screen(100)",
            func(|hw, _| hw.x_screen(None, 100)),
        );
        check(
            two,
            "Print X Screen(1,200)",
            func(|hw, _| hw.x_screen(Some(1), 200)),
        );
        check(
            "",
            "Print X Screen(5,200)",
            func(|hw, _| hw.x_screen(Some(5), 200)),
        );
        check(
            "",
            "Print Y Screen(100)",
            func(|hw, _| hw.y_screen(None, 100)),
        );
        check(
            two,
            "Print Y Screen(1,90)",
            func(|hw, _| hw.y_screen(Some(1), 90)),
        );
        check(
            "",
            "Print Zone(50,50)",
            func(|hw, _| hw.zone_fn(None, 50, 50, false)),
        );
        check(
            "",
            "Print Zone(0,45,45)",
            func(|hw, _| hw.zone_fn(Some(0), 45, 45, false)),
        );
        check(
            "",
            "Print Zone(-2,45,45)",
            func(|hw, _| hw.zone_fn(Some(-2), 45, 45, false)),
        );
        check(
            "",
            "Print Zone(4,45,45)",
            func(|hw, _| hw.zone_fn(Some(4), 45, 45, false)),
        );
        check(
            "",
            "Print Hzone(200,100)",
            func(|hw, _| hw.zone_fn(None, 200, 100, true)),
        );
        check(
            "",
            "Print Hzone(0,200,100)",
            func(|hw, _| hw.zone_fn(Some(0), 200, 100, true)),
        );
        check("", "Print Mouse Click", func(|hw, _| Ok(hw.mouse_click())));
        check("", "Print Scancode", func(|hw, _| Ok(hw.scancode())));
    }

    #[test]
    fn more_keywords() {
        let two = "Screen Open 1,320,100,4,Lowres : Screen 0";
        check(two, "Screen 1", inst(|hw, _| hw.screen(1)));
        check(two, "Screen 0", inst(|hw, _| hw.screen(0)));
        check(two, "Screen 3", inst(|hw, _| hw.screen(3)));
        check(two, "Screen 9", inst(|hw, _| hw.screen(9)));
        check("", "Centre \"Hello\"", inst(|hw, _| hw.centre(b"Hello")));
        check("Locate 5,3", "Centre \"\"", inst(|hw, _| hw.centre(b"")));
        let long = "x".repeat(60);
        let call = format!("Centre \"{long}\"");
        check("", &call, inst(move |hw, _| hw.centre(long.as_bytes())));
        check(
            "",
            "Gr Locate 10,20",
            inst(|hw, _| hw.gr_locate(Some(10), Some(20))),
        );
        check(
            "Gr Locate 5,5",
            "Gr Locate ,20",
            inst(|hw, _| hw.gr_locate(None, Some(20))),
        );
        let filled = "Ink 2 : Bar 0,0 To 100,100 : Print \"Text\"";
        check(filled, "Cls", inst(|hw, _| hw.cls(None, None)));
        check(filled, "Cls 3", inst(|hw, _| hw.cls(Some(3), None)));
        check(
            filled,
            "Cls 4,10,10 To 50,50",
            inst(|hw, _| hw.cls(Some(4), Some((10, 10, 50, 50)))),
        );
        check(
            "Screen Close 0",
            "Cls 3",
            inst(|hw, _| hw.cls(Some(3), None)),
        );
        check(
            "",
            "Text 10,50,\"Hi\"",
            inst(|hw, _| hw.text(Some(10), Some(50), b"Hi")),
        );
        check(
            "Gr Locate 30,40",
            "Text ,50,\"Hi\"",
            inst(|hw, _| hw.text(None, Some(50), b"Hi")),
        );
        check(
            "",
            "Text 10,50,\"\"",
            inst(|hw, _| hw.text(Some(10), Some(50), b"")),
        );
        let img = "Ink 3 : Bar 0,0 To 15,15 : Get Sprite 1,0,0 To 16,16";
        let bobs = format!("{img} : Bob 1,50,50,1");
        check(
            &bobs,
            "Bob 1,60,70,1",
            inst(|hw, _| hw.bob(1, Some(60), Some(70), Some(1))),
        );
        check(
            &bobs,
            "Bob 1,,80,",
            inst(|hw, _| hw.bob(1, None, Some(80), None)),
        );
        check(
            &bobs,
            "Bob 2,10,10,1",
            inst(|hw, _| hw.bob(2, Some(10), Some(10), Some(1))),
        );
        check(
            &bobs,
            "Bob -1,10,10,1",
            inst(|hw, _| hw.bob(-1, Some(10), Some(10), Some(1))),
        );
        let sprites = format!("{img} : Sprite 1,200,100,1");
        check(
            &sprites,
            "Sprite 1,210,120,1",
            inst(|hw, _| hw.sprite(1, Some(210), Some(120), Some(1))),
        );
        check(
            &sprites,
            "Sprite 1,,130,",
            inst(|hw, _| hw.sprite(1, None, Some(130), None)),
        );
        check(
            &sprites,
            "Sprite 5,,100,1",
            inst(|hw, _| hw.sprite(5, None, Some(100), Some(1))),
        );
        check(
            &sprites,
            "Sprite 70,10,10,1",
            inst(|hw, _| hw.sprite(70, Some(10), Some(10), Some(1))),
        );
    }

    #[test]
    fn collision_functions() {
        let objs = "Ink 3 : Bar 0,0 To 15,15 : Get Sprite 1,0,0 To 16,16\n\
Sprite 1,200,100,1 : Sprite 2,205,105,1 : Sprite 3,300,200,1\n\
Bob 1,50,50,1 : Bob 2,55,55,1 : Bob 3,150,150,1\n\
Sprite 4,X Hard(152),Y Hard(152),1 : Wait Vbl";
        check(
            objs,
            "Print Sprite Col(1)",
            func(|hw, _| hw.sprite_col_fn(1, None)),
        );
        check(
            objs,
            "Print Sprite Col(1,2 To 3)",
            func(|hw, _| hw.sprite_col_fn(1, Some((2, 3)))),
        );
        check(
            objs,
            "Print Sprite Col(1,0 To 64)",
            func(|hw, _| hw.sprite_col_fn(1, Some((0, 64)))),
        );
        check(
            objs,
            "Print Sprite Col(-1)",
            func(|hw, _| hw.sprite_col_fn(-1, None)),
        );
        check(
            objs,
            "Print Bob Col(1)",
            func(|hw, _| hw.bob_col_fn(1, None)),
        );
        check(
            objs,
            "Print Bob Col(1,0 To 500)",
            func(|hw, _| hw.bob_col_fn(1, Some((0, 500)))),
        );
        check(
            objs,
            "Print Bob Col(2,3 To 1)",
            func(|hw, _| hw.bob_col_fn(2, Some((3, 1)))),
        );
        check(
            objs,
            "Print Bob Col(1,-1 To 5)",
            func(|hw, _| hw.bob_col_fn(1, Some((-1, 5)))),
        );
        check(
            objs,
            "Print Bobsprite Col(1)",
            func(|hw, _| hw.bobsprite_col_fn(1, None)),
        );
        check(
            objs,
            "Print Bobsprite Col(1,0 To 70)",
            func(|hw, _| hw.bobsprite_col_fn(1, Some((0, 70)))),
        );
        check(
            objs,
            "Print Spritebob Col(1)",
            func(|hw, _| hw.spritebob_col_fn(1, None)),
        );
        check(
            objs,
            "Print Spritebob Col(4)",
            func(|hw, _| hw.spritebob_col_fn(4, None)),
        );
        check(
            objs,
            "Print Bobsprite Col(3)",
            func(|hw, _| hw.bobsprite_col_fn(3, None)),
        );
        check(
            objs,
            "Print Bobsprite Col(3,4 To 4)",
            func(|hw, _| hw.bobsprite_col_fn(3, Some((4, 4)))),
        );
        check(
            objs,
            "Print Spritebob Col(1,0 To 100)",
            func(|hw, _| hw.spritebob_col_fn(1, Some((0, 100)))),
        );
        check(
            "",
            "Print Sprite Col(1)",
            func(|hw, _| hw.sprite_col_fn(1, None)),
        );
    }

    #[test]
    fn menu_dialog_and_memory_keywords() {
        check("", "Print Choice", func(|hw, _| hw.choice_fn(None)));
        check("", "Print Choice(1)", func(|hw, _| hw.choice_fn(Some(1))));
        check("", "Print Choice(0)", func(|hw, _| hw.choice_fn(Some(0))));
        check("", "Print Dialog(1)", func(|hw, it| hw.dialog_fn(it, 1)));
        check("", "Print Dialog(0)", func(|hw, it| hw.dialog_fn(it, 0)));
        let start = |hw: &mut Hardware, it: &mut Interp| {
            it.preset_args(&[Some(Value::Int(10))]);
            match hw.function(
                it,
                Keyword {
                    slot: 0,
                    token: tk::START,
                },
            ) {
                Ok(Value::Int(a)) => a,
                r => panic!("{r:?}"),
            }
        };
        check(
            "",
            "Poke Start(10),200",
            inst(move |hw, it| {
                let a = start(hw, it);
                hw.poke(it, a, 200)
            }),
        );
        check(
            "",
            "Doke Start(10),$1234",
            inst(move |hw, it| {
                let a = start(hw, it);
                hw.doke(it, a, 0x1234)
            }),
        );
        check(
            "",
            "Loke Start(10),-5",
            inst(move |hw, it| {
                let a = start(hw, it);
                hw.loke(it, a, -5)
            }),
        );
        let fill = "Loke Start(10),$89ABCDEF";
        check(
            fill,
            "Print Peek(Start(10))",
            func(move |hw, it| {
                let a = start(hw, it);
                Ok(hw.peek(it, a))
            }),
        );
        check(
            fill,
            "Print Deek(Start(10))",
            func(move |hw, it| {
                let a = start(hw, it);
                Ok(hw.deek(it, a))
            }),
        );
        check(
            fill,
            "Print Leek(Start(10))",
            func(move |hw, it| {
                let a = start(hw, it);
                Ok(hw.leek(it, a))
            }),
        );
    }
}
