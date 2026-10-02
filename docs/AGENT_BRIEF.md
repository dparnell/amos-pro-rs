# Working on the AMOS Professional Rust port

This file is the shared brief for everyone (human or agent) implementing a
subsystem. Read it fully before starting.

## Goal

Port AMOS Professional (Commodore Amiga, 68000 assembler, in
`AMOS-Professional-365/`) to cross-platform Rust: macOS, Windows, Linux and
the web (wasm32). Rendering uses wgpu, sound cpal, windows winit (platform
code lives in `crates/amos-app`, which you normally do not touch). Replicate
the original behaviour as faithfully as practical: same results, same
quirks, same error numbers.

## Sources and research

* Original sources: `AMOS-Professional-365/*.s`. They are ISO-8859-1: the
  normal `grep` silently skips them, use `LC_ALL=C /usr/bin/grep -a -n ...`
  and `LC_ALL=C sed -n 'a,bp' file | iconv -f latin1 -t utf-8`.
* Research notes with file:line references: `docs/research/*.md`
  (`graphics-library.md`, `interpreter.md`, `sound-and-editor.md`,
  `tokens-and-file-format.md`, `part-*.md`). Always check the original code
  when the notes are not precise enough.
* Example programs: `AMOS-Professional-365/AMOS/**/*.AMOS` (Examples,
  Tutorial, Productivity...). List one with
  `cargo run -p amos-cli -- list FILE` or `python3 tools/amos_tokens.py FILE`.

## Architecture (crate `crates/amos-core`)

* `interp/`: the interpreter (`Interp`). It runs the verified token stream
  directly. Expressions, control flow, variables, procedures, strings and
  maths are implemented there.
* `machine/`: `Hardware` (everything except the interpreter) implements the
  interpreter's `Host` trait; `Machine` = `Interp` + `Hardware`, driven by
  the platform at 50 Hz (`Machine::vbl`).
* Keyword dispatch: `machine/dispatch.rs` calls, in turn,
  `screen_instruction`, `text_instruction`, `draw_instruction`,
  `sprites_instruction`, `sound_instruction`, `input_instruction`,
  `banks_instruction`, `files_instruction`, `system_instruction` (and the
  `*_function` equivalents). Each lives in `machine/inst_<name>.rs` and
  returns `Ok(false)` / `Ok(None)` for keywords it does not handle.
  Reserved variables used as instructions (`X Mouse=10`, `Timer=0`) are sent
  to the `*_instruction` handlers too: check `kw.def().unwrap().kind()`.
* Keywords: `crate::tokens::Keyword { slot, token }`. Slot 0 is the main
  library, 1 Music, 2 Compact, 3 Request, 4 3D, 5 Compiler, 6 IOPorts.
  Token constants are in `crate::tokens::tk` (generated file
  `src/tokens/names.rs`): e.g. `tk::SCREEN_OPEN`, overload variants get
  `_2`, `_3` suffixes (`tk::BOX`, ...), extension keywords are prefixed
  (`tk::MUSIC_SAM_PLAY`, `tk::COMPACT_UNPACK`). Look the names up in
  `names.rs` (each has a doc comment with the keyword and its signature).
  When `kw.token` is one of several overload variants, the verifier has
  already rewritten it to the variant matching the parameters.
* Reading parameters: in an instruction handler, the interpreter's pc is
  just after the keyword. Call `let a = it.inst_args(self, kw)?;` which
  parses and converts the parameters according to the signature, then use
  `a.int(i)`, `a.opt(i)` (None if omitted), `a.float(i)`, `a.str(i)`.
  Functions: `it.func_args(self, kw)?` (reads the parentheses). Special
  syntax: read tokens yourself with `it.peek()`, `it.eval(self)`,
  `it.eval_int(self)`, `it.expect(TK_COMMA)` etc.
* Errors: `return crate::interp::err(n)` with the AMOS error number
  (`crate::errors` has constants; the full list is in
  `src/errors/messages.rs`). Unsupported Amiga-only features:
  `Err(Exc::Message("...".into()))`.
* Waiting instructions (Wait Key, Sam Wait...): return
  `Err(Exc::Block)`; the instruction is executed again at the next frame.
  `it.wait_vbls(n)` waits n frames; `it.wait_state(|| WaitKind::Other(id, value))`
  keeps per-instruction state between retries; call `it.clear_wait()` when done.
* Hardware coordinates: X in lowres pixels, Y in raster lines, as used by
  `Screen Display`, `X Mouse`, sprites. The display shown by the renderer
  is described in `src/display.rs` (display units, `HW_X0`, `HW_Y0`).
* Screens: `gfx/screen.rs` (`Screen`, `Screens`). Bitmaps are chunky (one
  byte per pixel = colour index; bit p = bitplane p). Always draw into the
  logic bitmap and bump `version` (use `logic_mut()`), the renderer uses it
  to detect changes.

## Rules

* Only edit the files you own (listed in your task). If you need something
  from another subsystem, use the existing public API, or write down what you
  need in your final report. Never revert other people's changes.
* Several people build the same workspace at the same time. Use your own
  target directory to avoid lock waits: `CARGO_TARGET_DIR=target/agent-<name> cargo test -p amos-core ...`.
  Keep the crate compiling at every step (make small edits). If the build
  fails because of a file you do not own, wait a minute and retry (someone
  is in the middle of a change); do not "fix" their file.
* Never commit, never run `cargo fmt` on the whole workspace (format only
  your files with `rustfmt <file>`), never add a dependency without checking
  it is the latest release (`cargo search NAME`), and avoid dependencies in
  `amos-core` unless really useful.
* Code must work on wasm32 too: no threads, no `std::time::Instant`, no
  direct file access in `amos-core` (go through `crate::files`).
* Write unit tests for your code. To run BASIC code end to end, see
  `crates/amos-core/src/interp/tests.rs` (text-only host) or use
  `Machine`: `amos_core::tokenise::tokenise_program(src)`, `m.run_program(&prg)`,
  `m.vbl()` in a loop, `amos_core::display::render_rgba(&m.frame())`.
* Visual check: `cargo run -p amos-cli -- run FILE.AMOS|FILE.txt --frames 100 --png out.png`
  then look at the PNG (you can read images). Text programs are tokenised on
  the fly, which makes small test programs easy to write.
* Match the style of the surrounding code; comment the why, cite the
  original routine (`+W.s:1234`) for non-obvious behaviour.
