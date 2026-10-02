# AMOS to WebAssembly compiler: design

The original AMOS Pro compiler turned tokenised programs into 68000 code.
This port compiles them to **WebAssembly** instead. The compiled module is
run by the same AMOS runtime that runs interpreted programs (screens,
sound, input, files...), in the browser and in native applications.

Goals, in order:

1. **Same behaviour as the interpreter** (which follows the original):
   results, error numbers, quirks (operator precedence, FFP floats,
   `Not`, `mod`...), event timing (Every, Break, menus at test points),
   waits that let the display run.
2. **Speed**: expressions and control flow run as native wasm code.
3. **Self-contained applications**: the module is stored in the program
   bundle (`crate::bundle`) next to (or instead of) the tokenised program.

## Pipeline

```
.AMOS ──Verifier──► verified tokens (Compiled) ──lower──► IR ──codegen──► .wasm
```

* **Front end**: reuses `interp::verify` (the test pass) so the compiler
  accepts exactly what the interpreter accepts, with the same test-time
  errors. Lowering walks the verified token stream with the same parsing
  rules as the interpreter (`interp/expr.rs`, `flow.rs`, `stmt.rs`) and
  produces an IR: scopes (main + procedures), statements with resume ids,
  typed expression trees.
* **Back end**: `wasm-encoder` emits one module.

## Execution model: resumable state machine

AMOS programs are cooperative: `Wait Vbl`, `Input`, `Wait Key`, menus...
return control to the host for a frame and continue later; Every / On
Error / menus can jump into handlers at test points. The compiled code must
therefore be resumable at statement boundaries, exactly like the
interpreter (`Exc::Block` re-executes the instruction).

* The module exports `run(budget: i32) -> i32` (status: running / waiting
  / ended / error). All code is in this one function as a dispatch loop
  over **resume points** (`br_table`): every statement that can yield,
  jump, call or fail has a point id; straight-line statements between them
  are emitted inline.
* The **current point**, the AMOS **control stack** (For / Repeat / While
  / Do loops, Gosub returns, procedure frames) and all variables live in
  linear memory, never in wasm locals across points, so `run` can return
  at any point and be re-entered.
* Loops, If, Goto, Gosub, procedure calls and returns set the next point
  and branch to the dispatcher (later: direct branches inside a scope when
  no resume point is crossed).
* Test points (`Next`, `Loop`, `Wend`, `Until`, jumps, calls, waits...) call
  `host.test_point()`, which may request a jump to an Every / menu handler,
  a break, or a freeze (menu open) — same places as the interpreter.

## Values

| AMOS | wasm |
|---|---|
| integer | `i32` (with the interpreter's overflow rules: `+`/`-`/`*` errors) |
| float, single precision | `f64` holding an exact FFP value; arithmetic through the runtime's bit-exact FFP routines |
| float, double precision | `f64`, native wasm arithmetic |
| string | `i32` handle owned by the runtime string table |
| array | descriptor in linear memory |

## Runtime interface

The module imports:

* `rt.*`: support functions — FFP arithmetic, string operations (concat,
  compare, Mid$, Str$, Val...), number formatting, array allocation, Data
  reading — implemented in Rust next to the interpreter so both share one
  implementation.
* `host.*`: the machine — `print`, `test_point`, `keyword(slot, token,
  argc)` to run any instruction / function of the subsystems with argument
  values, input, errors.

`host.keyword` reuses the existing subsystem handlers (which read their
parameters from the token stream) by giving them a small synthetic token
stream built from the argument values, so every keyword the interpreter
supports works in compiled code from day one. Keywords with special syntax
or variable parameters (Input, Read, Swap, Inc, Dim, Varptr, Channel, Menu$,
Mid$= ...) are compiled specially.

## Hosting

* **Native**: wasmtime (cranelift JIT) inside the AMOS app; the bundle's
  module is instantiated with the `rt` / `host` imports bound to the
  machine.
* **Web**: the runtime instantiates the module through the browser's
  WebAssembly API, imports bound with wasm-bindgen.

## Testing

* Differential tests: every interpreter test program (`interp/tests.rs`
  and the example sweep) is also compiled and run; outputs, final
  variables and screens must match the interpreter.
* Benchmarks: interpreter vs compiled on loops, maths, string code.

## Milestones

1. Compiler crate + native host (wasmtime): integers, floats, strings,
   variables, arrays, all control flow, procedures with recursion, Print,
   generic keywords, waits; differential tests pass.
2. Errors (On Error / Resume / Trap), Every, Data/Read, Input, Def Fn, all
   special-syntax keywords; example sweep matches the interpreter.
3. Web host; bundles carry the module; editor "Build Application" and
   `Compile` produce compiled apps.
4. Optimisation: direct branches inside scopes, typed locals for hot loops,
   inlining of FFP/string helpers into the module.

## Status

* Milestones 1 and 3 are done; parts of milestone 2 work through the
  interpreter fallback (instructions the compiler does not translate yet are
  executed by the interpreter one at a time with variables synced, so every
  keyword works). `amos-cli compile PROG -v` lists them.
* Bundles carry the module in an optional `WASM` section; `amos-cli build`,
  `Compile` and the editor's Build Application compile by default
  (`--interpreted` skips it) and fall back to interpreting on failure.
* Hosts: `amos-wasmhost` (wasmtime natively, `WebAssembly.instantiate` on
  the web) share the import list in `amos-wasmhost/src/imports.rs`
  (interface version 8, `amos_core::compiled::ABI_VERSION`).
* Gosub / Return and procedure calls / End Proc / Pop Proc run in the
  module: a call the stack surely has room for becomes a *pending* entry in
  memory (locals and parameters in the procedure's memory frame, Param /
  Param# / Param$ in header words), and `Runtime::flush` turns pending
  entries into the interpreter's `Ctl::Gosub` / `Ctl::Proc` frames before
  every import, so errors (13 at the same depth, 8), On Error / Every /
  menu procedures, Resume, Trap, Data pointers, local arrays, string roots
  and yields all see the exact interpreter stack.
* Checks: differential tests (`cargo test -p amos-wasmhost`), the native
  frame-by-frame comparison of all examples (`cargo run -p amos-wasmhost
  --example compare`), and the same comparison in the real web runtime under
  node (`scripts/check-web-compiled.sh`).
