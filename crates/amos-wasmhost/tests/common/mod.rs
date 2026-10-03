//! Test harness: runs a program with the interpreter and compiled, on a
//! text-only host (like `interp/tests.rs`) or on a full `Machine`.

#![allow(dead_code)]

use std::rc::Rc;

use amos_core::Machine;
use amos_core::compiled::Env;
use amos_core::interp::stmt::InputState;
use amos_core::interp::value::Value;
use amos_core::interp::{Exc, Host, Interp, R, RunState, StopReason, StopReasonOrError};
use amos_core::tokenise::tokenise_program;
use amos_core::tokens::Keyword;
use amos_wasmhost::CompiledProgram;

/// Host that records printed text and implements nothing else.
#[derive(Default)]
pub struct TextHost {
    pub out: Vec<u8>,
    pub input: Vec<Vec<u8>>,
    /// Number of times `read_line` answers "not yet" before each line.
    pub input_delay: u32,
    delay: u32,
}

impl Host for TextHost {
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        let _ = it.inst_args(self, kw)?;
        Ok(())
    }
    fn function(&mut self, _it: &mut Interp, kw: Keyword) -> R<Value> {
        Err(Exc::Message(format!("function {:?} not available", kw.def().map(|d| d.name))))
    }
    fn reserved_assign(&mut self, _it: &mut Interp, _kw: Keyword) -> R<()> {
        Ok(())
    }
    fn test_point(&mut self, _it: &mut Interp) -> R<()> {
        Ok(())
    }
    fn take_break(&mut self) -> bool {
        false
    }
    fn print(&mut self, _it: &mut Interp, text: &[u8]) -> R<()> {
        self.out.extend_from_slice(text);
        Ok(())
    }
    fn read_line(&mut self, _it: &mut Interp, _state: &mut InputState) -> R<Option<Vec<u8>>> {
        if self.delay < self.input_delay {
            self.delay += 1;
            return Ok(None);
        }
        self.delay = 0;
        if self.input.is_empty() {
            return Ok(Some(Vec::new()));
        }
        Ok(Some(self.input.remove(0)))
    }
}

pub struct TextMachine {
    pub interp: Interp,
    pub host: TextHost,
}

impl Env for TextMachine {
    fn parts(&mut self) -> (&mut Interp, &mut dyn Host) {
        (&mut self.interp, &mut self.host)
    }
}

/// Outcome of a run: printed text (CR LF as LF), how it ended, frames used.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub text: String,
    pub end: StopReasonOrError,
    pub frames: usize,
}

pub fn compile_src(src: &str) -> (amos_core::Program, amos_compiler::Output) {
    let prg = tokenise_program(src.as_bytes()).expect("tokenise");
    let out = amos_compiler::compile_full(&prg).unwrap_or_else(|e| panic!("compile error: {e}"));
    validate(&out.wasm);
    (prg, out)
}

pub fn validate(wasm: &[u8]) {
    let mut v = wasmparser::Validator::new();
    if let Err(e) = v.validate_all(wasm) {
        panic!("invalid wasm: {e}");
    }
}

thread_local! {
    /// Frames `read_line` waits before each line (see `with_input_delay`).
    static INPUT_DELAY: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Runs `f` with Input answering only every `delay` frames.
pub fn with_input_delay<T>(delay: u32, f: impl FnOnce() -> T) -> T {
    INPUT_DELAY.with(|d| d.set(delay));
    let r = f();
    INPUT_DELAY.with(|d| d.set(0));
    r
}

fn text_machine(src: &str, input: &[&str]) -> Result<(TextMachine, amos_core::Program), StopReasonOrError> {
    let prg = tokenise_program(src.as_bytes()).expect("tokenise");
    let mut it = Interp::new();
    it.load(&prg).map_err(|e| StopReasonOrError::Test(e.code))?;
    let host = TextHost {
        input: input.iter().map(|s| s.as_bytes().to_vec()).collect(),
        input_delay: INPUT_DELAY.with(|d| d.get()),
        ..Default::default()
    };
    Ok((TextMachine { interp: it, host }, prg))
}

const MAX_FRAMES: usize = 20_000;

fn finish(m: &TextMachine, state: RunState, frames: usize) -> Option<Outcome> {
    match state {
        RunState::Running => None,
        RunState::Stopped(info) => {
            Some(Outcome { text: String::from_utf8_lossy(&m.host.out).replace("\r\n", "\n"), end: info.reason, frames })
        }
        RunState::Idle => Some(Outcome {
            text: String::from_utf8_lossy(&m.host.out).replace("\r\n", "\n"),
            end: StopReasonOrError::Stop(StopReason::End),
            frames,
        }),
    }
}

/// Runs with the interpreter.
pub fn interpret(src: &str, input: &[&str], budget: usize) -> Outcome {
    let (mut m, _) = match text_machine(src, input) {
        Ok(m) => m,
        Err(e) => return Outcome { text: String::new(), end: e, frames: 0 },
    };
    for f in 0..MAX_FRAMES {
        m.interp.vbl();
        let st = m.interp.run(&mut m.host, budget);
        if let Some(o) = finish(&m, st, f) {
            return o;
        }
    }
    panic!("interpreted program did not end")
}

/// Runs compiled; also split in functions of 2 instructions (the
/// code generation for large programs): the same outcome.
pub fn run_compiled(src: &str, input: &[&str], budget: usize) -> Outcome {
    let a = run_compiled_with(src, input, budget, &amos_compiler::Options::default());
    let split = amos_compiler::Options { split_min: 1, split_size: 2 };
    let b = run_compiled_with(src, input, budget, &split);
    assert_eq!(a, b, "split in functions, budget {budget}:\n{src}");
    a
}

fn run_compiled_with(src: &str, input: &[&str], budget: usize, options: &amos_compiler::Options) -> Outcome {
    let (mut m, prg) = match text_machine(src, input) {
        Ok(m) => m,
        Err(e) => return Outcome { text: String::new(), end: e, frames: 0 },
    };
    let out = amos_compiler::compile_with(&prg, options).unwrap_or_else(|e| panic!("compile error: {e}"));
    validate(&out.wasm);
    let mut cp = CompiledProgram::new(&out.wasm, m.interp.prg.clone().unwrap()).expect("instantiate");
    for f in 0..MAX_FRAMES {
        m.interp.vbl();
        let st = cp.run(&mut m, budget);
        if let Some(o) = finish(&m, st, f) {
            return o;
        }
    }
    panic!("compiled program did not end")
}

/// Runs both ways and checks the outcomes are identical; returns it.
pub fn same(src: &str) -> Outcome {
    same_with(src, &[])
}

pub fn same_with(src: &str, input: &[&str]) -> Outcome {
    let a = interpret(src, input, 100_000);
    let b = run_compiled(src, input, 100_000);
    assert_eq!(a.text, b.text, "output differs for:\n{src}");
    assert_eq!(a.end, b.end, "end differs for:\n{src}\noutput: {}", a.text);
    assert_eq!(a.frames, b.frames, "frames differ for:\n{src}");
    a
}

/// Runs both ways (must end normally) and returns the output.
pub fn run(src: &str) -> String {
    let o = same(src);
    assert_eq!(o.end, StopReasonOrError::Stop(StopReason::End), "program failed: {src}\n{}", o.text);
    o.text
}

pub fn run_err(src: &str) -> StopReasonOrError {
    let o = same(src);
    assert_ne!(o.end, StopReasonOrError::Stop(StopReason::End), "expected an error: {src}");
    o.end
}

// ----------------------------------------------------------------------
// Full machine
// ----------------------------------------------------------------------

/// Runs on a `Machine` for `frames` frames, interpreted and compiled;
/// returns both machines.
pub fn machines(src: &str, frames: usize) -> (Machine, Machine) {
    let prg = tokenise_program(src.as_bytes()).expect("tokenise");
    let mut a = Machine::new();
    a.run_program(&prg).expect("test");
    for _ in 0..frames {
        a.vbl();
        if !a.interp.running {
            break;
        }
    }
    let wasm = amos_compiler::compile(&prg).expect("compile");
    validate(&wasm);
    let mut b = Machine::new();
    let mut cp = CompiledProgram::start(&mut b, &prg, &wasm).expect("start");
    for _ in 0..frames {
        cp.vbl(&mut b);
        if !b.interp.running {
            break;
        }
    }
    (a, b)
}

pub fn global_value(m: &Interp, name: &str) -> Option<Value> {
    let prg = m.prg.clone()?;
    let i = prg.globals.iter().position(|g| g.name == name.as_bytes())?;
    match &m.globals[i] {
        amos_core::interp::value::Var::Scalar(v) => Some(v.clone()),
        _ => None,
    }
}

pub fn rc<T>(v: T) -> Rc<T> {
    Rc::new(v)
}

/// Runs both ways with a small time budget: the programs must yield at the
/// same places (same number of frames) and produce the same results.
pub fn same_budget(src: &str, budget: usize) -> Outcome {
    let a = interpret(src, &[], budget);
    let b = run_compiled(src, &[], budget);
    assert_eq!(a, b, "outcome differs with budget {budget} for:\n{src}");
    a
}
