//! Runtime support for AMOS programs compiled to WebAssembly.
//!
//! The compiler (crate `amos-compiler`, see `docs/COMPILER.md`) turns the
//! verified token stream into a wasm module whose `run(budget)` function is
//! a resumable state machine over the program's instructions. Everything
//! the module cannot do on its own goes through its imports (`rt.*` value
//! helpers and `host.*` machine and control operations), implemented by
//! [`Runtime`] here so that native hosts (wasmtime) and the web host share
//! one implementation, next to the interpreter.
//!
//! Division of work:
//!
//! * The module keeps scalar variables in its linear memory (see
//!   [`layout`]) and evaluates expressions, assignments, `If` and `Print`
//!   natively.
//! * The control stack, procedure frames, events, error handling, arrays
//!   and Def Fn definitions stay in the interpreter's [`Interp`] state, so
//!   loops, Gosub, procedures, Every, On Error, menus... behave exactly as
//!   in the interpreter: the runtime calls the same `Interp` methods.
//!   Positions in the code are the resume points: `Interp::pc` holds the
//!   position where a suspended program continues. Gosubs and procedure
//!   calls the module can do on its own (the common case) are kept as
//!   *pending* entries in memory, with the procedure's locals in its memory
//!   frame, and pushed on the interpreter's control stack
//!   ([`Runtime::flush`]) before anything else can see the stack: the hosts
//!   flush before every import. A Return / End Proc of a still pending entry
//!   just drops it.
//! * Keywords of the subsystems run through the *keyword bridge*: the
//!   module evaluates the parameters, the runtime builds a small token
//!   stream with the values as constants and calls the existing handler.
//! * Any instruction the compiler does not translate is run by the
//!   interpreter itself ([`Runtime::interp`]) after copying the scalar
//!   variables it uses into the interpreter, and back afterwards.
//!
//! [`Interp`]: crate::interp::Interp

pub mod layout;
pub mod runtime;
pub mod structure;

pub use runtime::{Env, HeapKind, Runtime, ST_CONTINUE, ST_GROW, ST_STOP, ST_YIELD};

/// Version of the module / runtime interface. A module records the version
/// it was compiled for (exported global `amos_abi`).
pub const ABI_VERSION: i32 = 13;

/// Status returned by the module's `run` export.
pub const RUN_RUNNING: i32 = 1;
pub const RUN_STOPPED: i32 = 2;

use crate::Machine;
use crate::interp::{RunState, StopInfo, StopReasonOrError};

/// Text describing why a program stopped (as `Machine::vbl` logs it).
pub fn describe_stop(info: &StopInfo) -> String {
    match &info.reason {
        StopReasonOrError::Stop(r) => format!("{r:?}"),
        StopReasonOrError::Error(n) => crate::errors::message(*n).to_string(),
        StopReasonOrError::Message(m) => m.clone(),
        StopReasonOrError::Test(n) => crate::errors::test_message(*n).to_string(),
    }
}

/// One vertical blank of a machine running a compiled program: what
/// `Machine::vbl` does, with `run` (the compiled program's `run`) in place
/// of `Interp::run`. Returns true when the program chained to another one
/// with `Run "file"`: that program was started on the interpreter, and the
/// caller must use `Machine::vbl` from now on.
pub fn machine_vbl(m: &mut Machine, run: impl FnOnce(&mut Machine, usize) -> RunState) -> bool {
    m.hw.vbl();
    m.interp.vbl();
    if !m.interp.running {
        return false;
    }
    let budget = m.instructions_per_frame;
    m.state = run(m, budget);
    let RunState::Stopped(info) = &m.state else { return false };
    if let Some(path) = m.hw.pending_run.take() {
        match m.hw.files_read_all(&path).ok().and_then(|d| crate::Program::load(&d).ok()) {
            Some(prg) => {
                if let Err(e) = m.run_program(&prg) {
                    m.hw.log.push(crate::errors::test_message(e.code).to_string());
                }
                return true;
            }
            None => m.hw.log.push(crate::errors::message(81).to_string()),
        }
        return false;
    }
    let msg = describe_stop(info);
    m.hw.log.push(msg);
    false
}
