# Runtime performance log

Running log of the runtime / interpreter / renderer optimisation work.

## Tools

* `cargo run --release -p amos-core --example perf -- [examples] [stress] [editor] [FILES...]`
  runs every example program (on a temporary copy of its directory), the
  stress programs of `crates/amos-core/examples/perf/*.txt` and an editor
  session (scroll + typing in the largest example), and reports the time
  spent per frame in `Machine::vbl`, frame building and audio mixing.
  `--out FILE` writes a checksum per program (FNV of `render_rgba` of every
  frame, of the audio samples rendered every frame, and of the log + final
  state); `--check FILE` compares with them. `--repeat N` keeps the fastest
  of N runs (the machine is shared, timings are noisy). `--no-hash` skips
  the reference compositor (pure timing); `--no-print-log` does not copy
  printed text to the log, as the app does.
* Load-independent measurements: `/usr/bin/time -l` reports the retired
  instruction count; the difference between a 60 and a 20 frame run of a
  micro program gives the cost per frame (200 000 interpreted instructions).
* Profiles: `sample PID 10 -f out.sample` on a release build with
  `CARGO_PROFILE_RELEASE_DEBUG=true`.

Every step below was checked with: `cargo test --release --workspace`
(including the compiled-vs-interpreted differential tests), the checksums of
all 195 examples + 14 stress programs + the editor over 200 frames
(display, audio, log: 0 differences against the baseline taken before any
change), `cargo run --release -p amos-wasmhost --example compare 100`
(194 identical, 0 different, 1 skipped, as before) and a wasm32 build of
amos-core and amos-app. Edge cases the examples do not cover (text at the
bitmap edges, writing modes 0-20, shade / underline / inverse, windows,
scrolling, cursor shapes, autoback text; bobs with flips, hot spots,
clipping, minterms and No Mask; sprite / bob collisions every frame) were
written as small programs and compared over 400 frames with the same
checksums produced by a build of the baseline commit.

## Results

Baseline = HEAD 58d103d (before this work). Best of 3 runs of 200 frames,
`--no-hash`, same machine, same session; ms of `Machine::vbl` (interrupts +
interpreter + everything the program does) unless noted.

| Workload | before | after | speed-up |
|---|---|---|---|
| all 195 example programs, total | 112 445 ms | 56 277 ms | 2.0x |
| Help_55 (Print / Locate busy loop) | 18 334 ms | 3 824 ms | 4.8x |
| AMAL_3 (sprite collisions every frame) | 10 116 ms | ~630 ms | 16x |
| Help_61 (Print busy loop) | 5 798 ms | 2 716 ms | 2.1x |
| Help_21 (zones) | 4 436 ms | 2 617 ms | 1.7x |
| _Splines (maths + Draw) | 4 040 ms | 2 169 ms | 1.9x |
| Help_57 (Peek / Poke) | 2 844 ms | 1 840 ms | 1.5x |
| stress programs, total | 12 454 ms | 7 463 ms | 1.7x |

Stress programs (us of vbl per frame):

| program | before | after | speed-up |
|---|---|---|---|
| text_print (Print, Pen, Paper, At) | 1 601 | 59 | 27x |
| text_scroll (Print with scrolling) | 2 835 | 59 | 48x |
| draw_prims (Plot/Draw/Bar/Circle/Box/Ellipse) | 2 278 | 330 | 6.9x |
| bobs (31 bobs, double buffer, autoback) | 326 | 88 | 3.7x |
| sprites | 3.5 | 1.9 | 1.8x |
| screen_copy (Screen Copy, Scroll) | 15.1 | 11.8 | 1.3x |
| interp_maths | 14 680 | 8 180 | 1.8x |
| interp_arrays | 12 473 | 7 977 | 1.6x |
| interp_procs | 9 746 | 6 943 | 1.4x |
| interp_strings | 18 110 | 13 463 | 1.3x |
| draw_paint, amal, rainbow, music, editor | unchanged (already < 0.3 ms) | | |

Frame building (`Machine::frame`) and audio mixing are 2-20 us per frame
and were left alone. The reference compositor `render_rgba` (only used by
tools and tests) went from ~0.8 to ~0.5 ms per frame.

Micro programs (millions of CPU instructions per frame = 200 000
interpreted instructions, load independent):

| program | before | after |
|---|---|---|
| `Do : Loop` | 31.4 | 25.1 |
| `For I=1 To .. : Next I` | 84.0 | 58.6 |
| `A=B` | 78.5 | 42.6 |
| `A=A+1` | 108.4 | 61.3 |
| `A=A+1 : B=A*3-(A/7) : C=A and 255` | 192.5 | 107.3 |
| `C#=C#+0.5` | 125.9 | 80.1 |
| `T(5)=T(6)+1` | 168.0 | 124.9 |
| `A$="abc"` | 100.4 | 79.9 |
| `If A>10 Then B=1` | 105.8 | 70.8 |
| `X=Mouse Key` | 83.8 | 51.0 |
| `Ink 2` | 141.6 | 89.7 |
| `Locate 0,0 : Print "<37 chars>"` | 1 259 | 612 |

## Changes

1. **Text output** (`gfx/window.rs`): characters are drawn 8 pixels at a
   time (one u64 per glyph row, a 256 entry bit -> byte mask table) when
   the cell is inside the bitmap; scrolling (`copy_rows`), clearing
   (`fill_paper`), cursor draw/erase (`aff_cur` / `eff_cur`) work on whole
   rows. The pixel-by-pixel code stays for clipped cells. `Print` no longer
   copies its text; the console log of printed text is built without a
   per-character conversion for ASCII text.
2. **Sprite/bob collisions** (`gfx/bobs.rs`, `machine/inst_sprites.rs`):
   the collision tests read the image mask straight from the planar bank
   data (`ColImg`) instead of converting both images to chunky pixels for
   every pair tested; rectangles that do not overlap cost nothing. Tested
   against the old path for all flip combinations.
3. **Image conversion** (`banks.rs`): `Image::to_chunky` converts a plane
   byte (8 pixels) at a time (bob drawing converts its image every frame).
4. **Bob drawing** (`gfx/bobs.rs`): the blitter minterm is applied to all
   planes of a pixel at once with byte masks instead of plane by plane
   (tested against the plane-by-plane reference for all 256 minterms).
5. **Interpreter**:
   * keyword definitions are found with a direct index instead of a binary
     search (`tokens::lookup_ext`), and only when needed by `core_function`;
   * instruction / function parameters are collected in an inline
     small-vector (`ArgVec`, no heap allocation for up to 6 parameters);
   * integer fast path for binary operators (same code as the integer arms
     of the general path, shared through small functions);
   * `Next` / `Until` / `Loop` / `Wend` no longer clone the control stack
     entry, `Next` reads its variable once; `after_jump` returns at once when
     the innermost loop contains the new position;
   * a token is read with one bounds check; `TK_VAR` size computed inline;
   * FFP -> f64 conversion builds the power of two from its bits instead of
     calling `powi` (exact, tested against the old formula).
6. **Renderer / app** (`amos-app`): palettes and layer uniforms are uploaded
   only when they changed; when no VBL ran and no input arrived since the
   last frame (displays faster than 50 Hz, idle editor between VBLs) the
   frame is not rebuilt nor recomposited, only blitted to the window again.

7. **More interpreter work**: scalar variables are read and assigned
   without building a location record (`operand_value`, `assign`); main
   library tokens met once as host functions / host instructions skip the
   interpreter's own `match`es afterwards (`function_value`,
   `exec_instruction`; those matches accept or refuse a token by its value
   alone).
8. **Print**: runs of plain characters on one line are drawn together (one
   pass per glyph row over the whole run); the cursor save/restore works on
   8 pixel words. The copy of printed text into `Hardware::log` can be
   switched off (`Hardware::log_print`, default on: tools and tests keep
   their console output). The app turns it off unless
   `RUST_LOG=amos_print=debug`: with the app's default `info` filter every
   Print went to stderr / the browser console (one `console.info` per
   Print: a Print loop such as Help_55 made 130 000 of them per frame).
9. **Screen Copy** (`gfx/blocks.rs`): the blit works on whole rows.
10. **Peek / Poke** outside banks: the sparse memory pages use a cheap
    hasher instead of SipHash (Help_57: 1434 -> 919 ms).
11. **Reference compositor** `display::render_rgba` (tests, `--png`, the
    compare tool, the checksums): row based, 30.6 s -> 19 s over all the
    example frames; tested against the per-pixel version on random frames.
12. **App**: when a VBL takes more than 12 ms (a program that never waits,
    on a slow host), the instruction budget per VBL shrinks (down to
    20 000) and grows back to the default (200 000) when VBLs are fast
    again. On a fast host it never changes; on a slow one the display and
    input stay at 50 Hz instead of slowing down. (Before, a busy loop that
    took 30 ms per VBL made the app run up to 12 VBLs per redraw: a few
    frames per second.)

13. **Keyword dispatch** (`machine/dispatch.rs`): measured first. The six
    first subsystems of the chain (screen, text, draw, sprites, sound,
    input) are inlined into `Host::instruction` / `Host::function` and the
    compiler already merges their keyword tests into one switch, so they
    cost nothing to skip. A table of all twelve handlers made it slower:
    once their address is taken they stop being inlined (Scin / X Mouse /
    Mouse Key loop +3% instructions, +10% cycles). The kept version leaves
    that part as it was and only remembers, per keyword, which of the six
    later subsystems (banks, files, menus, dialogs, copper, system: big
    functions called out of line) accepted it, calling it directly next
    time. `X=Choice` loop: 78.2 -> 61.4 M instructions per frame; the
    keywords of the first six subsystems are unchanged (same code).
    A test probes every keyword of every extension, as an instruction and
    as a function, through the chain on two machines in very different
    states (to check that acceptance depends on the keyword only) and checks
    that the dispatcher uses that handler.

## API notes for the compiler side

* `Interp::function_value(hw, kw)` (interp/expr.rs): value of the function
  `kw` whose token was just read (pc after it and its inline data), exactly
  as `operand_value` computes it (`core_function` for main-library keywords,
  else `hw.function`). `operand_value` itself now uses it for main-library
  tokens.
* `Interp::preset_args(&[Option<Value>])` (interp/params.rs): parameters of
  the next `inst_args` / `func_args` call, already evaluated (`None` =
  omitted). They are converted per signature as `args` does (same
  conversions, degrees for type 5, first type error first; fewer values =
  Syntax error after the last one, more values = Syntax error once the
  signature is converted). Used by the next call only, also when it fails.
* `machine::plain_args(kw)` (machine/dispatch.rs): keywords whose handler
  reads its parameters only through one `inst_args` / `func_args` call and
  never looks at the token stream: Plot, Draw / Draw To, Ellipse, Circle,
  Bar, Box, Paint, Gr Locate, Text, Ink, Point, Locate, Pen, Paper, Curs
  Pen, Home, Curs On/Off, Mouse Zone, Cls, Colour, Screen Display / Offset,
  Scin, X/Y Hard, X/Y Screen, Bob, Sprite, Paste Bob / Icon, X/Y/I Bob,
  X/Y/I Sprite, X/Y Mouse, Mouse Key, Mouse Click, Joy / Jup... / Fire,
  Key State, Timer, Inkey$ (all overloads). Keywords without parameters
  are called without a preset; reserved variables only when read.
  Tests: every plain keyword with parameters gives the same `Args` from a
  preset as from the tokens (all values, each one omitted, two value
  sets), and calling every plain handler with presets (pc elsewhere) gives
  the same result, log and display as the statement in a program.
