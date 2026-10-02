//! Linear memory layout of a compiled program.
//!
//! All addresses are relative to the module's memory base (the `env.base`
//! global imported by the module: 0 when the module has its own memory).
//! Values are little endian, as wasm loads and stores them.
//!
//! * A small header of words shared by the module and the runtime.
//! * Global variables: 8 bytes per slot of `Compiled::globals`.
//! * Procedure frames: one block of `frame_size` bytes per procedure
//!   depth; slot `i` of the frame at depth `d` (1 = first call) is at
//!   `locals + (d - 1) * frame_size + i * 8`.
//!
//! A slot holds an `i32` (integer), an `f64` (float, an exact FFP value in
//! single precision) or an `i32` string handle (0 = empty string). Zero
//! bytes are an unset variable, as in the interpreter. Arrays and Def Fn
//! definitions live in the interpreter (`Interp::globals` and frame
//! locals) and have no storage here.

use crate::interp::verify::Compiled;

/// Non-zero when the next test point has work to do: VBL pending (events,
/// menus) or strings to collect. Set by the runtime.
pub const ATT: u32 = 16;
/// Non-zero when a runtime function left a pending error / exception.
pub const ERR: u32 = 20;
/// Type of the last dynamic number returned by the runtime (0 int, 1 float).
pub const TAG: u32 = 24;
/// Absolute address (base included) of the current procedure frame (the
/// globals in the main program).
pub const FP: u32 = 28;
/// Current label scope (`Interp::scope`).
pub const SCOPE: u32 = 32;
/// Mirror of the top of the control stack, refreshed by the runtime after
/// every operation (the control stack only changes in the runtime), so
/// `Next` and jumps run without calling the runtime in the common case.
///
/// Top entry is a For loop on an integer scalar: absolute address of the
/// variable (0 otherwise), step, limit and point of the loop body.
pub const FOR_ADDR: u32 = 36;
pub const FOR_STEP: u32 = 40;
pub const FOR_LIMIT: u32 = 44;
pub const FOR_BODY: u32 = 48;
/// Range `[LOOP_LO, LOOP_HI]` of positions a jump can reach without
/// `Interp::after_jump` dropping a loop: the body..exit range of the top
/// loop of the current routine, or everything.
pub const LOOP_LO: u32 = 52;
pub const LOOP_HI: u32 = 56;
/// Indices of an array element (8 `i32`), written by the module before
/// calling `host.aref`.
pub const IDX: u32 = 64;
/// Start of the global variables.
pub const GLOBALS: u32 = 128;

pub const PAGE: u32 = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub globals: u32,
    pub n_globals: u32,
    pub locals: u32,
    pub frame_size: u32,
    pub max_frames: u32,
    /// Total size in bytes.
    pub size: u32,
    /// Size in 64 KiB pages.
    pub pages: u32,
}

impl Layout {
    pub fn new(c: &Compiled) -> Layout {
        let n_globals = c.globals.len() as u32;
        let locals = (GLOBALS + n_globals.max(1) * 8).next_multiple_of(16);
        let max_locals = c.procs.iter().map(|p| p.locals.len() as u32).max().unwrap_or(0).max(1);
        let frame_size = max_locals * 8;
        // The control stack limit of the interpreter (`Interp::start`)
        // bounds the number of procedure frames (42 bytes each).
        let stack_limit = ((c.stack_size + 1) * 42).saturating_sub(64) as u32;
        let max_frames = stack_limit / 42 + 2;
        let size = locals + frame_size * max_frames;
        let pages = size.div_ceil(PAGE).max(1);
        Layout { globals: GLOBALS, n_globals, locals, frame_size, max_frames, size, pages }
    }

    /// Start of the frame of procedure depth `depth` (globals for 0).
    pub fn frame_base(&self, depth: usize) -> u32 {
        if depth == 0 { self.globals } else { self.locals + (depth as u32 - 1) * self.frame_size }
    }
}
