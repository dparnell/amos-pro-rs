//! Linear memory layout of a compiled program.
//!
//! All addresses are relative to the module's memory base (the `env.base`
//! global imported by the module: 0 when the module has its own memory).
//! Values are little endian, as wasm loads and stores them.
//!
//! * A small header of words shared by the module and the runtime.
//! * Global variables: 8 bytes per slot of `Compiled::globals`.
//! * Parameters of the procedure being called: 8 bytes each, converted
//!   to the parameter types by the module.
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
/// Kind of the top entry: `TOP_*`; `FOR_BODY` holds the point of its body.
pub const TOP_KIND: u32 = 60;
pub const TOP_OTHER: i32 = 0;
pub const TOP_FOR: i32 = 1;
pub const TOP_REPEAT: i32 = 2;
pub const TOP_DO: i32 = 3;
pub const TOP_WHILE: i32 = 4;
/// Top While loop: position and point of its While instruction.
pub const TOP_START: u32 = 116;
pub const TOP_START_POINT: u32 = 120;
/// Set by a `Wend` done by the module: the While entry, which the
/// interpreter pops at Wend and pushes again at While, was left on the
/// control stack for the While instruction (executed next) to reuse. The
/// runtime pops it if anything else happens first (error, suspension).
pub const LAZY_WHILE: u32 = 112;
/// Indices of an array element (8 `i32`), written by the module before
/// calling `host.aref`.
pub const IDX: u32 = 64;
/// Value of `End Proc[value]` (`i32` or `f64`), written by the module.
pub const RET: u32 = 96;
/// Type of `RET`: -1 none, 0 integer, 1 float, 2 string handle.
pub const RET_TAG: u32 = 104;
/// Number of procedure frames (`Interp::frame_stack`).
pub const DEPTH: u32 = 108;
/// `Param` and `Param#` (`Interp::param_e` / `param_f`), refreshed by the
/// runtime after every operation.
pub const PARAM_E: u32 = 124;
pub const PARAM_F: u32 = 128;
/// String allocator: next free byte and end of the current chunk
/// (absolute addresses, see `runtime/strings.rs`).
pub const STR_PTR: u32 = 136;
pub const STR_END: u32 = 140;
/// `Fix` (`Interp::fix`) as the `FixFlg` / `ExpFlg` words of the original,
/// refreshed by the runtime after every operation.
pub const FIX_FLG: u32 = 144;
pub const EXP_FLG: u32 = 148;
/// Gosubs and procedure calls done by the module and not yet pushed on
/// the interpreter's control stack (see `Runtime::flush`): their number,
/// then the number of entries of `Interp::ctl` and `Interp::stack_limit`
/// (mirrors, for the module's room check: an entry takes at most
/// `CTL_MAX_ENTRY` bytes).
pub const PEND_COUNT: u32 = 152;
pub const CTL_LEN: u32 = 156;
pub const STACK_LIMIT: u32 = 160;
pub const CTL_MAX_ENTRY: i32 = 42;
/// Index + 1 of the topmost pending procedure call (0: none). Entries above
/// it are Gosubs of that procedure.
pub const PEND_PROC: u32 = 164;
/// `Param` values set by an `End Proc` of the module and not yet given to
/// the interpreter: bit 0 `PARAM_E`, bit 1 `PARAM_F`, bit 2 `PARAM_S` (a
/// string handle, valid while the bit is set).
pub const PARAM_SET: u32 = 168;
pub const PARAM_S: u32 = 172;
/// Mirror of `Interp::error_proc_depth` (-1 for none).
pub const ERR_PROC: u32 = 176;
/// A pending entry (`PEND_ENTRY` bytes): return position, return point,
/// the control stack mirror words it hid (`MIRROR_WORDS`, restored when it
/// returns), then its kind: -1 for a Gosub, else the procedure index,
/// followed (procedures) by the `FP`, `SCOPE` and `PEND_PROC` of the caller.
pub const PEND_ENTRY: u32 = 64;
pub const PE_RET: u32 = 0;
pub const PE_POINT: u32 = 4;
pub const PE_MIRROR: u32 = 8;
pub const PE_KIND: u32 = 44;
pub const PE_FP: u32 = 48;
pub const PE_SCOPE: u32 = 52;
pub const PE_PREV: u32 = 56;
pub const MIRROR_WORDS: [u32; 9] =
    [TOP_KIND, FOR_ADDR, FOR_STEP, FOR_LIMIT, FOR_BODY, LOOP_LO, LOOP_HI, TOP_START, TOP_START_POINT];
/// Used by the module only: the last value its `ffp2a` helper scaled to
/// `[1, 10)` (after the sign), the scaled value and its decimal exponent
/// (`Str$` converts the same value twice; zero memory is the entry of 0).
pub const NORM_KEY: u32 = 180;
pub const NORM_XN: u32 = 184;
pub const NORM_E: u32 = 188;
/// Parameters of a keyword bridge call (instead of `host.push_*`): slot `k`
/// of the call holds an `i32` type at +0 (`BRIDGE_INT`, `BRIDGE_FLOAT`,
/// `BRIDGE_STR` or `BRIDGE_DYN_INT`) and the value at +8 (`i32`, `f64`,
/// string handle, or an integer as an `f64`). Each call site writes from a
/// fixed slot (calls inside its parameters use the following slots).
/// `BRIDGE_CALL` (direct calls only, `host.plain_*` / `host.pfn_*`): the
/// value of the main library function without parameters whose token is at
/// +8 (`machine::plain_args`), evaluated by the runtime when it reads the
/// slot.
pub const BRIDGE_SLOT: u32 = 16;
pub const BRIDGE_SLOTS: u32 = 64;
pub const BRIDGE_INT: i32 = 0;
pub const BRIDGE_FLOAT: i32 = 1;
pub const BRIDGE_STR: i32 = 2;
pub const BRIDGE_DYN_INT: i32 = 3;
pub const BRIDGE_CALL: i32 = 4;
/// Scratch buffers of the number formatting helpers (128 bytes each).
pub const SCR_A: u32 = 192;
pub const SCR_B: u32 = 320;
/// Start of the global variables.
pub const GLOBALS: u32 = 512;

pub const PAGE: u32 = 65536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub globals: u32,
    pub n_globals: u32,
    /// Parameters of a procedure call.
    pub args: u32,
    /// Table of string constants: address of each once created (0 before).
    pub consts: u32,
    pub n_consts: u32,
    /// Pending Gosubs and procedure calls (`PEND_ENTRY` bytes each).
    pub pending: u32,
    /// Parameters of keyword bridge calls (`BRIDGE_SLOTS` of
    /// `BRIDGE_SLOT` bytes: see `BRIDGE_SLOT`).
    pub bridge: u32,
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
        let args = (GLOBALS + n_globals.max(1) * 8).next_multiple_of(16);
        let max_params = c.procs.iter().map(|p| p.params.len() as u32).max().unwrap_or(0).max(1);
        let n_consts = crate::compiled::structure::string_constants(c).len() as u32;
        let consts = (args + max_params * 8).next_multiple_of(16);
        let stack_limit = ((c.stack_size + 1) * 42).saturating_sub(64) as u32;
        let pending = (consts + n_consts * 4).next_multiple_of(16);
        let bridge = (pending + (stack_limit / 12 + 2) * PEND_ENTRY).next_multiple_of(16);
        let locals = (bridge + BRIDGE_SLOTS * BRIDGE_SLOT).next_multiple_of(16);
        let max_locals = c.procs.iter().map(|p| p.locals.len() as u32).max().unwrap_or(0).max(1);
        let frame_size = max_locals * 8;
        // The control stack limit of the interpreter (`Interp::start`)
        // bounds the number of procedure frames (42 bytes each).
        let max_frames = stack_limit / 42 + 2;
        let size = locals + frame_size * max_frames;
        let pages = size.div_ceil(PAGE).max(1);
        Layout {
            globals: GLOBALS,
            n_globals,
            args,
            consts,
            n_consts,
            pending,
            bridge,
            locals,
            frame_size,
            max_frames,
            size,
            pages,
        }
    }

    /// Start of the frame of procedure depth `depth` (globals for 0).
    pub fn frame_base(&self, depth: usize) -> u32 {
        if depth == 0 { self.globals } else { self.locals + (depth as u32 - 1) * self.frame_size }
    }
}

/// Arrays in linear memory: a block with this header, then the elements
/// (`i32` integers, `f64` floats, `i32` string handles). A variable slot
/// holding an array contains the absolute address of its block (0: not
/// dimensioned).
pub const ARR_NDIMS: u32 = 0;
pub const ARR_TYPE: u32 = 4;
/// Number of elements.
pub const ARR_COUNT: u32 = 8;
/// Maximum index of each dimension (`Dim A(10)` gives 10), 8 `u32`.
pub const ARR_DIMS: u32 = 16;
pub const ARR_DATA: u32 = 48;

/// Size of an element of type `ty` (0 int, 1 float, 2 string).
pub fn elem_size(ty: u8) -> u32 {
    if ty == 1 { 8 } else { 4 }
}
