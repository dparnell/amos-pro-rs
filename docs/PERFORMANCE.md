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

## Summary of rounds 1-2

Baseline 58d103d (before this work) against 1f534a9 (rounds 1-2 and the
round 3 ellipse change), same harness (`perf.rs` copied into a worktree of
the baseline), 200 frames, `--no-hash`, printed text copied to the log (as
the baseline always did).

| | baseline | now | ratio |
|---|---|---|---|
| CPU instructions, all examples + stress + editor (load independent) | 3 037.9 G | 1 310.3 G | 2.3x fewer |
| all 195 examples, ms of `Machine::vbl`, best of 3 | 106 979 | 41 828 | 2.6x |
| stress programs, ms, best of 3 | 12 664 | 5 050 | 2.5x |

| example (ms of vbl, best of 3) | baseline | now | speed-up |
|---|---|---|---|
| Help_55 (Print / Locate busy loop) | 13 319 | 2 169 | 6.1x |
| AMAL_3 (sprite collisions every frame) | 9 873 | 399 | 25x |
| Help_61 (Print busy loop) | 5 504 | 1 986 | 2.8x |
| _Splines (maths + Draw) | 4 261 | 1 174 | 3.6x |
| Help_21 (zones, Mouse Zone / Mouse Key loop) | 4 192 | 1 698 | 2.5x |
| Help_15 | 3 181 | 1 847 | 1.7x |
| Help_57 (Peek / Poke) | 2 927 | 1 519 | 1.9x |
| Help_43 | 2 777 | 651 | 4.3x |
| Help_13 | 2 770 | 1 350 | 2.1x |
| Font8x8_Editor (Mouse Key tests) | 2 402 | 991 | 2.4x |
| Help_36 | 2 259 | 1 234 | 1.8x |
| Disc_Manager | 2 118 | 866 | 2.4x |

| stress program (us of vbl per frame) | baseline | now | speed-up |
|---|---|---|---|
| text_print | 1 496 | 39 | 38x |
| text_scroll | 2 577 | 43 | 60x |
| draw_prims | 2 320 | 104 | 22x |
| draw_paint | 202 | 72 | 2.8x |
| bobs | 325 | 22 | 15x |
| sprites | 3.2 | 1.1 | 2.9x |
| screen_copy | 15.1 | 11.1 | 1.4x |
| interp_maths | 14 928 | 5 225 | 2.9x |
| interp_arrays | 12 952 | 5 374 | 2.4x |
| interp_procs | 9 966 | 3 976 | 2.5x |
| interp_strings | 18 532 | 10 378 | 1.8x |
| amal, music, rainbow, editor | < 0.1 ms, unchanged | | |

Compiled code (wasmhost `suite`, 100 frames): all 194 examples 21.7 s
interpreted, 5.1 s compiled (4.2x).

Top remaining costs (`sample` of all examples, interpreted): the
interpreter core is ~70% (`int_prec` 20%, `Interp::run` 14%, `eval_prec`
12%, `exec_flow` 7%, `eval_int` 3%, `binop` 2.4%, `args_into` 2.1%,
variable access `var_slot` / `var_slot_ref` / `var_ref` 4%), function
dispatch `Host::function` + `function_value` 10% (busy-wait loops on Mouse
Key / Mouse Zone), text printing 3%, string allocation (`Rc<[u8]>`
results, `Var` drops) 2%. Compiled (`suite`): `Interp::run` and the
evaluator for the code still interpreted 30%, line drawing (`line_pen`,
Bresenham, already one store per pixel) 6.5%, text 4%, ellipses 3.5%,
`Host::function` 3.4%. Further interpreter gains are limited by
`Interp::run`'s code generation: changing anything it inlines moves every
statement by +-20 instructions (see the note at the end of round 2).

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

14. **preset_args without allocation**: the preset vector is kept in
    `Interp` and reused; its values are moved out instead of cloned.
15. **Omitted coordinates** (`Scin(,y)`, `Hzone`, `Scroll`, `Get Cblock`,
    `Sprite Base` / `Icon Base`): word / 32 bit arithmetic wraps as on the
    68000 (EntNul's low word is 0 in `GetSIn` / `ZoEc`) instead of
    overflowing (debug builds panicked; release results are unchanged). A
    test calls every keyword with each parameter omitted in turn, as an
    instruction and as a function, and checks nothing panics (memory range
    keywords Copy / Fill / Hunt / Bsave / Ssave are skipped: an omitted end
    address is a 2 GB range).

16. **Argument handling**: the parameter types of a keyword come from a
    table indexed by token (`tokens::param_types_of`, tested against
    `TokenDef::param_types` for every slot and token); `inst_args` /
    `func_args` fill the `Args` they return in place (`args_into`,
    `preset_into`) instead of copying the list through two `Result`s; the
    preset is one vector kept in `Interp` plus a flag; parameters already of
    the signature's type skip the conversion; the inline slots of `ArgVec`
    past its length are not dropped (they hold `Int(0)`). A pooled
    (thread local) vector was tried and was slower (TLS access on macOS).
    Interpreted: `Ink 2` loop 90.6 -> 83.2, busy wait (Scin, X/Y Mouse,
    Mouse Key) 131.4 -> 121.4, Joy / Key State 112.5 -> 102.9 M
    instructions per frame; suite: busy wait 532 -> 441 ms, inkey$ / joy /
    timer 507 -> 380 ms. Compiled (`spin`): Ink 109.5 -> 102.9, Plot 167.8
    -> 158.6, Locate 164.2 -> 155.0, busy wait 121.6 -> 115.3, Joy / Key
    State 122.4 -> 112.5 M instructions per frame.

17. **Drawing primitives** (`gfx/draw.rs`, `machine/inst_draw.rs`). The
    previous implementations are kept as `gfx/draw_reference.rs` (test
    only); `primitives_match_the_reference` runs 20 000 random cases (Plot,
    Draw, line runs, Box, Bar, Ellipse, Polygon, Paint, Text) over random
    canvas sizes and planes, all writing modes, line patterns and counters,
    1 and multi plane fill patterns, Set Paint and clip windows, and
    compares the pixels and the whole graphic state afterwards.
    * the clip rectangle and draw mode are decided once per primitive
      (`Writer`) instead of per pixel;
    * lines: when both ends are inside the clip rectangle no pixel is
      clipped (the line stays in their bounding box); with a full / empty
      line style every pixel gets the same operation and the pattern
      position is advanced once (Bresenham visits max(|dx|,|dy|)+1
      points); other styles keep a local pattern position;
    * Bar / Polygon fills: whole spans with the operation decided once
      (memset for solid fills, row of pattern bits for patterns);
    * Ellipse / Circle: points drawn as generated (writing the ink is
      idempotent); COMPLEMENT marks done pixels in a bitset instead of
      sorting all points;
    * Paint: solid fills colour the region in place (filled pixels stop
      matching the seed: no visited array), patterned fills scan rows;
    * graphic Text: unclipped cells written directly;
    * `draw_op`: no `Vec` of targets and no clone of the graphic state
      (with its pattern) unless Autoback draws twice.

    Millions of CPU instructions per frame (200 000 interpreted
    instructions; compiled with `spin`, interpreted with `perf`):

    | loop | compiled before | after | interpreted before | after |
    |---|---|---|---|---|
    | Plot 10,20 | 157.3 | 125.8 | 145.0 | 114.1 |
    | Draw 0,0 To 300,150 | 2149 | 646 | 2150 | 648 |
    | same, Set Line $F0F0 | 2179 | 1159 | 2180 | 1160 |
    | same, Gr Writing 2 | 2179 | 707 | 2180 | 709 |
    | Box 10,10 To 200,150 | 4715 | 1249 | 4716 | 1250 |
    | Bar 10,10 To 200,150 | 1010 | 922 | 1011 | 924 |
    | same, Set Pattern 2 | 194 200 | 7 608 | 194 210 | 7 610 |
    | same, Set Paint 1 | 5230 | 2020 | 5231 | 2020 |
    | Circle 160,100,60 | 6836 | 1387 | 6830 | 1381 |
    | Ellipse, Gr Writing 2 | 9583 | 3061 | 9573 | 3062 |
    | Polygon (triangle) | 88 408 | 4 845 | 88 373 | 4 812 |
    | Cls + Circle + Paint | 21 664 | 8 839 | 21 594 | 8 795 |
    | Text 10,50,"Hello world" | 1648 | 1361 | 1634 | 1348 |
    | Polyline | 3628 | 1109 | 3595 | 1076 |
    | Draw with Double Buffer + Autoback | 4117 | 1143 | 4118 | 1145 |
    | suite plot/draw | 938 | 374 | 948 | 385 |
    | suite bar/box/circle | 2237 | 680 | 2269 | 715 |

    `suite -- 100 20 -` (ms): plot/draw 3113 -> 2259 compiled, 3209 ->
    2382 interpreted; bar/box/circle 8850 -> 3108 compiled, 9044 -> 3257
    interpreted.
18. **Preset parameters written in place** (`interp/params.rs`,
    separate change): `inst_args` / `func_args` fill the returned `Args`
    from the preset as they do from the tokens (`take_preset_into`), no
    `Args` moved through a `Result`. Same instruction count, fewer cycles
    (store forwarding of the 176 byte copy): compiled Plot loop 4.28 ->
    3.97 G cycles over 200 frames, Ink 3.52 -> 3.26.

### Round 2 (profile of all examples at 9b4eab4: interpreter 84%)

Instruction counts are M instructions per frame of a micro program (the
statement in a `Do : Loop`), before -> after.

19. **Procedure calls** (`interp/mod.rs`): procedure frames and argument
    vectors pooled. `r_proc` 233.8 -> 123.3.
20. **String constants shared** (`interp`): a string constant is one `Rc`
    per program, cloned instead of copied. `A$="abc"` 79.9 -> 44.5.
21. **Integer evaluator** (`interp/expr.rs`, `int_prec`): integer-only
    expressions (constants, integer variables, operators) evaluated
    without `Value`s first; anything else, or an error, falls back to the
    general evaluator (no side effects). `exec_flow` split. m3_expr 110.8
    -> 77.9, interp_maths 152.9 -> 123.0.
22. **Else cache** (`interp/flow.rs`): the end of an Else block per
    position, cached per program. If/Else 50.2 -> 43.6.
23. **Put Block / Zoom / Appear** (`gfx/blocks.rs`, `machine/inst_draw.rs`):
    no clone of the block or of the source bitmap per call; Put Block row
    by row with byte masks; Zoom row-major fast path. Put Block 64x64
    28242 -> 810, Zoom 160x128 -> 320x256 168202 -> 102883. Reference
    versions kept as tests.
24. **Text in writing modes** (`gfx/window.rs`): glyphs with Writing /
    Shade / Under flags 8 pixels at a time. Writing 2 27735 -> 3553
    (Print of 33 characters). Pixel by pixel version kept for clipped
    cells and as test reference.
25. **Print and scrolling** (`gfx/window.rs`, `interp/stmt.rs`): scrolled
    rows copied without reading the destination when no plane is kept;
    Print's text buffer kept in `Interp`, integers formatted in place
    (`ffp::push_int`). Print with scroll 4313 -> 2912, `Print A;` 285.5 ->
    218.3.
26. **String results in one allocation** (`interp/expr.rs`, `stmt.rs`):
    `A$+B$`, Upper$/Lower$/Flip$, Mid$ statement collected straight into
    the `Rc<[u8]>`; Str$ of an integer formatted on the stack; Pen/Paper &
    co no longer copy their control string; cursor save in place.
    `C$=A$+B$` 159.6 -> 123.2, `Str$(I)` 197.3 -> 138.3, `Pen 4 : Paper
    3` 178.5 -> 135.0. (No interning of short strings: the interface
    compares strings by address, as the original does.)
27. **String/maths function parameters** (`interp/expr.rs`): Mid$,
    Left$, Right$, Instr, String$, Repeat$, Str$, Abs, Int, Sgn read their
    parameters one by one instead of through the general list. Mid$ 184.3
    -> 158.7, Abs 96.5 -> 72.9.
28. **Integer array elements** (`interp/expr.rs`, `int_element`): read by
    the integer evaluator. `A=T(5)` 75.0 -> 55.2, `T(I+1)=T(I)+1` 140.2 ->
    113.5 (scalar integer statements +0.6..1.5%).
29. **Integer presets** (`interp/params.rs`): `preset_ints` values written
    into the `Args` slots in place. Compiled `Ink 2,3,4` loop 105.8 ->
    97.1, `Bar` 142.8 -> 131.3.
30. **Input functions first** (`machine/dispatch.rs`): Mouse Key is called
    798 M times over all examples (busy-wait loops); the function chain
    now starts with the input handler (no keyword has two handlers: test
    `every_keyword_has_at_most_one_handler`). `Repeat : Until Mouse Key`
    91.8 -> 81.6. All examples: 1418.5 G -> 1363.5 G instructions (items
    27-30).

Codegen trap met several times: `Interp::run` inlines much of the
interpreter, and unrelated changes (a grown inlined helper, a new
`[Value; N]` type whose drop glue changes inlining, a first-token check
before `int_prec`) cost +20 instructions per statement on every program.
Check `m0_loop` (`Do : Loop`, 26.0) after any interpreter change; keep
new code out of line (`#[inline(never)]` helpers) when it moves.

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
* `plain_args` also covers Screen To Front/Back, Hide/Show, Screen (both),
  Zone / Hzone, Choice, Dialog, Scancode, Key Shift, Limit Mouse,
  Peek/Deek/Leek, Poke/Doke/Loke, Centre and the collision functions
  (Bob Col, Sprite Col...).
* `Interp::preset_buf()` (the preset vector, filled in place) and
  `Interp::preset_ints(&[i32], given_mask)` (integer presets, no `Value`).
* `Interp::ctl_generation()`: changes with every structural change of the
  control stack or the frame stack (`push_frame_index` /
  `pop_frame_index` / `clear_frame_stack` / `bump_ctl_generation` for
  changes made on the fields); debug builds check it after every
  interpreted instruction.
* Typed keyword functions on `Hardware` (`pub(crate)`, one implementation
  per keyword, the token arms call them): locate, pen, paper, zone_fn,
  ink, plot, draw_to, draw, box_, bar, circle, point, limit_mouse /
  limit_mouse_screen / limit_mouse_area, screen_to_front, colour_fn,
  x_screen / y_screen / x_hard / y_hard, choice_fn, dialog_fn,
  poke / doke / loke, peek / deek / leek, mouse_click, scancode
  (signatures in their doc comments; tests in `typed_keywords`).
