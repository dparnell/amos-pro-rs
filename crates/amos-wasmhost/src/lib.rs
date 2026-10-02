//! Hosts for AMOS programs compiled to WebAssembly.
//!
//! [`CompiledProgram`] instantiates a module produced by `amos-compiler`
//! with its imports bound to [`amos_core::compiled::Runtime`], and runs it
//! with the same shape as the interpreter: [`CompiledProgram::run`] takes a
//! time budget and returns a `RunState`; [`CompiledProgram::vbl`] is the
//! equivalent of `Machine::vbl` for a `Machine` running a compiled program.
//!
//! * Native: wasmtime (`native.rs`), instantiated synchronously.
//! * Web (wasm32): the browser's WebAssembly API through wasm-bindgen
//!   (`web.rs`), instantiated asynchronously (browsers do not compile large
//!   modules synchronously on the main thread). The module shares the
//!   runtime's own memory: its variables live in a block allocated by the
//!   runtime, whose address is the module's `env.base`.

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::*;

pub use amos_core::compiled::describe_stop;
