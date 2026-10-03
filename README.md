# amos-rs

A port of **AMOS Professional** (Commodore Amiga, 68000 assembler) to
cross-platform Rust. It runs on macOS, Windows and Linux, and in a web
browser through WebAssembly. Rendering uses [wgpu](https://wgpu.rs), sound
[cpal](https://github.com/RustAudio/cpal) and windows
[winit](https://github.com/rust-windowing/winit).

The original sources (public domain since 2012) are in
`AMOS-Professional-365/`, and the port follows them closely: same token
format and `.AMOS` files, same quirks (operator precedence, float format,
error numbers), same editor, resources and banks.

## Running

```sh
cargo run --release -p amos-app                           # the AMOS Pro editor
cargo run --release -p amos-app -- path/to/program.AMOS   # run a program
```

Text programs (`.txt`/`.Asc`) are tokenised on load. The AMOS distribution
folders are mounted as the usual volumes (`AMOSPro_System:`,
`AMOSPro_Examples:`...), and the folder of the program becomes the current
directory.

In the editor: F1 runs, F2 tests, Esc enters Direct mode, the right mouse
button opens the menus. The Amiga key is Command (macOS) or the Windows key.
Game controllers work as Amiga joysticks: the first one is in the joystick port
(port 1), the second in the mouse port (port 0); d-pad or left stick to move,
any face or shoulder button to fire. Without a controller, port 1 is emulated
with the cursor keys and Ctrl/Alt for fire.

### Building standalone applications

The original compiler produced 68000 code; here "compiling" builds a
self-contained application that runs the program with the AMOS runtime:

```sh
cargo build --release -p amos-app -p amos-cli && ./scripts/build-web.sh
target/release/amos-cli build path/to/Game.AMOS --out build
```

This writes `build/Game.app` (macOS; a single executable on Linux and
Windows) and `build/Game-web/` (copy it to any web server). By default the
files of the program's folder are bundled too, so pictures and data the
program loads are available (`--no-files` bundles the program only,
`--include DIR` another folder). Bundled files appear in the `Bundle:`
volume, which is the current directory. The `Compile` instruction of the
Compiler extension and the editor's Build Application (Project menu) build
applications the same way.

The program is compiled to WebAssembly and run natively by the app (see
`docs/COMPILER.md`); `--interpreted` ships the interpreter only. To compile
or run compiled without building an app:

```sh
target/release/amos-cli compile Game.AMOS -o Game.wasm -v
target/release/amos-cli run --compiled Game.AMOS
```

### Console version

`amos-console` runs programs with `Print`/`Input` on the terminal:

```sh
cargo run -p amos-cli --bin amos-console -- program.AMOS
cargo run -p amos-cli --bin amos-console -- -e 'For I=1 To 3 : Print I : Next'
echo 'Print "Hello"' | cargo run -p amos-cli --bin amos-console -- -
```

### Web

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <same as the wasm-bindgen crate>
./scripts/build-web.sh
python3 -m http.server -d web 8000
```

The page preloads the AMOS distribution files and lets you pick a program.

### Developer tools

```sh
cargo run -p amos-cli -- list program.AMOS               # listing (identical to AMOS)
cargo run -p amos-cli -- run program.AMOS --png out.png  # headless run + screenshot
cargo run -p amos-cli -- edit program.AMOS --png out.png # drive the editor
python3 tools/sweep.py 150 -v                            # run every example program
python3 tools/coverage.py                                # keywords not implemented yet
```

## Layout

* `crates/amos-core`: everything platform independent: tokens and file
  formats, tokeniser/lister, verifier, interpreter, screens, text windows,
  drawing, sprites/bobs/AMAL, Paula sound and music players, menus,
  Interface dialogs, editor, virtual file system.
* `crates/amos-app`: the winit + wgpu + cpal application (native and wasm).
* `crates/amos-cli`: headless runner, editor driver and `amos-console`.
* `docs/research`: notes on the original sources with file:line references.
* `tools`: generators for the token and message tables, sweep and coverage
  scripts.

## Differences from the original

* The Topaz 8 font is a re-drawn look-alike (the real one is in the Kickstart
  ROM).
* Amiga-only features (machine code procedures, `Doscall`/`Execall`, devices,
  ARexx, serial/parallel ports, MED) report errors instead of working.
* The editor has optional syntax highlighting (Config menu, "Syntax Colours").
* The compiler does not produce 68000 code: it builds standalone native and
  web applications instead (see above).
