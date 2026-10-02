# AMOS Professional interpreter: research notes for the Rust port

Source tree: `/Users/danielparnell/Documents/Develoment/amos-rs/AMOS-Professional-365` (68000 asm, French comments, ISO-8859-1. Convert with `iconv -f latin1 -t utf-8` before grepping, because UTF-8 grep silently skips lines). References are `file:line` in the original files. Line numbers are the same in the UTF-8 copies.

Important layout fact: **`+ILib.s` is `Include`d into `+Lib.s` (+Lib.s:1710)**. Together they make `AMOSPro.Lib`, which holds the token table, the interpreter core (ChrGet loop, evaluator, control flow, errors) and every instruction. `+Verif.s` is included in `+B.s` (the AMOSPro executable). `+W.s` is amos.library, which handles graphics, screens, sprites and interrupts.

Contents
- Part A: Startup, memory layout, program structure, editor/interpreter switching, library and token-table format (covers task 1)
- Part B: The verification/test pass (covers task 2)
- Part C: The run-time execution model (covers task 3)
- Part D: Error messages and numbering, error dispatch (covers task 4)
- Part E: Banks and .Abk/.AMOS formats, float formats and number formatting, menus, Dialog/Interface (covers tasks 5, 6 and 7)

Cross-checked facts and caveats:
- Operator precedence was re-checked against the code (`Tk_Operateurs` +ILib.s:3048 and `New_Evalue` +ILib.s:3101). Each operator token is its own precedence level, ranked by unsigned token value: xor < or < and < <> < >< < <= < =< < >= < => < = < < < > < + < - < mod < * < / < ^. A new operator is pushed only if its value is strictly higher than the stacked one, otherwise the stacked one is applied first. So `10*3/4` evaluates as `10*(3/4)` = 0. This is unusual, so confirm it on an emulator before relying on it.
- The bit-exact float formatting and FFP arithmetic in Part E and Part C come from reading the code only, not from running it. Validate them against real AMOS (for example in an emulator).
- Behaviour that depends on Amiga ROM math libraries (FFP rounding, how float-to-int conversion saturates) has to come from the AROS sources or from hardware tests.

---

# Part A: Startup, memory, program management

## 1. Startup sequence (+B.s), memory layout, editor/interpreter switching

All paths relative to `AMOS-Professional-365/`. Comments in the sources are French. Note: files are ISO-8859-1; `grep` under a UTF-8 locale silently misses lines — convert with `iconv -f latin1 -t utf-8` first.

### 1.1 Module map

| File | Role |
|---|---|
| `+B.s` | The `AMOSPro` executable: cold start, library loader/relocator, panic exit, misc internal helpers (memory, lists, disk). `Include "+Verif.s"` at +B.s:1365, so the test pass, `Prg_*` program management and program load/save physically live in +B.s's hunk. |
| `+Verif.s` | Test/verification pass (`PTest`, `SsTest`, `VerDirect`), `ClearVar`, `Stack_Reserve`, `ResVarBuf`, program structure routines `Prg_RunIt/TestIt/Push/Pull/New/Load/Save` (+Verif.s:4336–5050), line navigation `Tk_FindL/FindA/SizeL`, locked-procedure codec `ProCode` (+Verif.s:5167), KwiK verify table swap `Ver_Verif/Ver_Run` (+Verif.s:5203). |
| `+Lib.s` | `AMOSPro.Lib` — token table (`C_Tk`, +Lib.s:48) + all instruction/function routines + interpreter core (ChrGet loop, evaluator). Built as a library file loaded at runtime. |
| `+ILib.s` | Library routine numbering (`Lib_Def` = routine in AMOSPro.Lib, `Lib_Int` = internal routine supplied by the AMOSPro exe via the `AMOSJmps` patch table). e.g. `Prg_RunIt` = Lib_Int (+ILib.s:67), `Ed_Cold`..`Ed_RunDirect` = Lib_Def slots filled when the editor is loaded (+ILib.s:39–58). |
| `+W.s` | `amos.library` (graphics/screens/sprites/bobs/text windows/interrupts — the "trap"). Called via `SyCall/EcCall/WiCall` macros. |
| `+Edit.s` | `AMOSPro_Editor` (separately LoadSeg'ed). |
| `+Monitor.s` | `AMOSPro_Monitor` (separately LoadSeg'ed). |
| `+Header.s` | Header of **compiled** programs (APCmp output), not used by the interpreter: decodes hunks for program + libraries, opens libs, then starts the compiled code (+Header.s:20–120). Flags `FHead_PRun`, `FHead_Backed`. Not needed for an interpreter port except as reference. |
| `+Interpreter_Config.s` | Source of `AMOSPro_Interpreter_Config` file (see 1.4). |
| `+Editor_Config.s` | Source of `AMOSPro_Editor_Config` — **contains all error message tables** (see §4). |

### 1.2 Cold start (`Cold_Start`, +B.s:1416)

1. Save SP (`SaveSp`), allocate a scratch frame on the stack (`SP_*` RS struct defined just above Cold_Start, +B.s:1395–1410).
2. Open dos.library, intuition.library (+B.s:1425–1445).
3. Workbench vs CLI (+B.s:1455): WB → `WaitPort/GetMsg` the WBStartup, CD to the program's dir, if an argument icon is passed copy its name to `SP_AutoName`. CLI → parse command line (`FromCLI`, +B.s:1507): `-C "file"` alternate config, `-W` = don't bring AMOS to front (clears `SP_WBench`), first non-option word = program to load, rest = command line given to the program (`SP_CommandLine`).
4. `LoadSeg` amos.library: tries `APSystem/amos.library`, `libs/amos.library`, `libs:amos.library` (`CommandX`, +B.s:1576–1600).
5. Allocate the **data zone** (+B.s:1604): one `AllocMem(Public|Clear)` of `LDataWork + W-datas-length (from amos.library header offset 8) + DataLong`. `a5` points *after* the work buffers and the W.Lib data (so W.Lib data sits at negative offsets from a5; BASIC data `DataLong` at positive offsets defined by `RsReset` struct in +Equ.s:1145+). Work buffers carved downward (+B.s:1621–1650): `BufLabel`(32), `BufBob`, `MnTDraw`, `AAreaBuf`, `AAreaInfo`, IFF buffers (`BufAMSC/CCRT/CMAP/CAMG/BMHD`), `DirFNeg`(108), `Name2`(108), `Name1`(256), `Buffer`(`TBuffer`=1024).
6. Call amos.library init `jsr 12(a4)` with magic `"V2.0"`, must return `"W2.0"` (+B.s:1665–1675).
7. Load **interpreter config** (+B.s:1683–1730): `-C` file, else `AMOSPro_Interpreter_Config`, `s/…`, `s:…`. Format: `"PId1"`, long length, raw block copied to `PI_Start(a5)`; then `"PIt1"`, long length, text block → `Sys_Messages(a5)`.
8. Find APSystem path (message #1 `APSystem/`), Lock + `AskDir2` to build absolute path into `Sys_Pathname` (+B.s:1750).
9. Open `mathffp.library` → `FloatBase` (+B.s:1765). (Double/IEEE libs are opened later on demand, see §6.) graphics.library, optionally icon.library + `Def_Icon`.
10. **Libraries** (+B.s:1802): `Libraries_Load` (main `AMOSPro.Lib` = message 14, then extensions 1..26 = messages 16..41, +B.s:2108), `Library_Patch` (fill internal routine slots from `AMOSJmps`/`+Internal_Jumps.s`), `Libraries_Reloc` (+B.s:2136; `Library_Reloc` then `Library_GetParams`). Sets `Sys_LibStarted`.
11. `LdMouse` — loads `PI_AdMouse` sprite bank (message 2; must start `"AmSp"`, ≥4 images) (`LdMouse`, +B.s:2044). `LdFSel` — loads `AMOSPro_Default_Resource.Abk` (message 8) as bank 16 into `Sys_Banks`, checks name `"Reso"`, stores `Sys_Resource` (`LdFSel`, +B.s:2081).
12. `AutoAssigns` — runs `c:assign` for `AMOSPro_Accessories:` etc if they don't exist (+B.s:1936).
13. Start amos.library interrupts/graphics `jsr (a6)` with `PI_ParaTrap`, mouse bank, default palette `PI_DefEPa`, default Amiga-A key (+B.s:1820–1836).
14. `Libraries_Init` — calls routine #0 of each library with `"APex"` magic + version; extension must return its own number (called +B.s:1837, routine +B.s:2818).
15. Program name: command line / WB arg, else message 5 `AutoExec.AMOS` (+B.s:1850). Command line copied to `Buffer+TBuffer-256-6` as `"CmdL"`, word length, chars (read by `Command Line$`) (+B.s:1866–1880).
16. Measure total Chip/Fast memory; `AskDir/CopyPath`; optional auto-close Workbench (`PI_AutoWB`); `jmp Init_Fin`.

### 1.3 `Init_Fin` (+B.s:42) — choose run-only vs editor

```
Prg_NewStructure(PI_DefSize)        ; program structure + text buffer (default 320 KB)
Prg_New(-2)                          ; no default screen
Prg_Load(-1)  (always adapt buffer)  ; try Name1 (autoexec/cmdline program)
  ok  -> Prg_RunIt(d0=-1, a1=RunErr_RunOnly, a2=0)   ; run directly (run-only)
  fail-> Sys_VerInstall; Edit_Load (LoadSeg AMOSPro_Editor, patch Ed_* slots);
         Prg_New(-1); Ed_Cold; WOption; Ed_Title; jmp Ed_Loop   ; editor main loop
```
So **if `AutoExec.AMOS` (or the command-line program) exists it runs without the editor**; when it ends/errs, `RunErr_RunOnly` (+B.s:967) reloads the editor (`Edit_Load`+`Ed_Cold`, with memory-recovery fallbacks `MemMaximum`/`MemDelBanks`) and jumps to `Ed_ErrRun`. Exit code 1002 ("System") from a program quits to `TheEnd`.

`Program_Load` (+B.s:434): LoadSeg a module (editor = message 6, monitor = message 10), then copy its exported jump list into the main library's routine-address table at `AdTokens(a5) - LB_Size - 4*(first function number)` — i.e. editor/monitor entry points are slots in AMOSPro.Lib's jump table (`L_Ed_Start`, `L_Mon_Start`).

### 1.4 Program structure & running

`Prg_*` structure (+Equ.s:1848–1882): `Prg_Next`, `Prg_NLigne`, text buffer `Prg_StMini` (alloc base) / `Prg_StTTexte` (size) / `Prg_StHaut` (top) / `Prg_StBas` (start of tokenised source — the source sits at the *top* of the buffer, ending with a zero word at `StHaut-2`), `Prg_Banks` (bank list head), `Prg_Dialogs`, flags `Prg_StModif` (needs re-test), `Prg_Change`, `Prg_Edited`, `Prg_Not1.3`, `Prg_Reloaded`, `Prg_MathFlags` (double precision), `Prg_Previous` (stack of running programs for `Prun`), `Prg_RunData` (saved data zone of the caller), `Prg_AdEProc/XEProc` (error position in folded proc), undo, marks, `Prg_NamePrg[128]`.

`Prg_RunIt` (+Verif.s:4336) — `d0` = 0 normal / 1 accessory / -1 PRun; `a1` = error-return address (stored in `Prg_JError`), `a2` = editor callbacks (0/4/8 = before test / after test / during DefRun).
1. `Prg_DejaRunned` — refuse if already in the running chain.
2. `Prg_Push` — if another program is running, save the runtime part of the data zone `DebSave..FinSave` (+Equ.s:1325–1520) plus the `Sys_ClearRoutines` list into `Prg_RunData` of the caller, then `Prg_DataNew` resets per-program state (var buffers, menus, includes, files, devices, libraries).
3. `Prg_SetBanks` (Cur_Banks=&Prg_Banks, Cur_Dialogs, Prg_Source=StBas), `Bnk.Change` (notify extensions + sprite bank to W.Lib).
4. `ClearVar`, **`PTest`** (verification), record `VerNot1.3` and `MathFlags` into the structure.
5. Normal program: `DefFlag=-1`, `DefRun1`, `DefRun2` (reset screens: default screen from PI_DefE*), `ReCop`, activate screen 0; accessory/PRun: `DefRunAcc` (keeps screens). Then `JJmp L_New_ChrGet` — **the interpreter loop never returns here**; termination goes through `Prg_JError` with `d0` = code (see §4).

`Prg_TestIt` (+Verif.s:4406) = same up to PTest then `Prg_Pull` (used by the editor's Test command).

Editor side (+Edit.s): `Ed_Run` (+Edit.s:8166) → `Prg_RunIt(0, a1=Ed_ErrRun, a2=Ed_TestMessage)`; `Ed_RunHidden` (+Edit.s:8106) accessory; `Ed_RunDirect` (+Edit.s:8133) runs from direct mode. Direct mode line: tokenise line into `Ed_BufT`, `Prg_JError=Ed_ErrDirect`, `VerDirect` (+Verif.s:43: reserve 4 KB var buffer if none, `ResDir` direct-mode variable space `PI_TVDirect`, name buffer `PI_VNmMax`, `Stack_Size=10`, `Phase=1`, `DirFlag=1`, `SsTest`, then `Ver_Run`), then `New_ChrGet` (+Edit.s:9228–9242).

Return codes delivered to `Prg_JError` (`Ed_ErrRun`, +Edit.s:8253; `Ed_GetError` +Edit.s:8335):
- `d0 < 0` → **test-time** error `-d0`, message from `Ed_TstMessages`.
- `0 <= d0 < 256` → run-time error, message `Ed_RunMessages[d0+1]` (1-based message list whose entry 1 is error 0).
- `a0 != 0` → `a0` is a literal message string (extensions' custom errors).
- `10` = End of program, `1000` = Edit (return to editor silently), `1001` = Direct (go to direct mode), `1002` = System (quit AMOS).
- Otherwise `Ed_Ligne` dialog offers Edit/Direct; editor positions cursor at `VerPos(a5)` (address of the failing token).

`Prg_Load` (+Verif.s:4789) — .AMOS file: 16-byte header `"AMOS Basic v134 "` (1.3; compared on 10 chars) or `"AMOS Pro101v\0\0\0\0"` (Pro; 8 chars compared); byte 11 = `'V'` tested / `'v'` not tested (set by `Prg_Save` +Verif.s:4964), **byte 15 = math flags** (Pro only, `Prg_MathFlags`, bit = double precision); then long source length, the tokenised source (loaded so it ends at `StHaut-2`), then banks via `Bnk.Load(EntNul)` (an `"AmBs"` block — see §5). `Prg_Save` finds the real end via `Tk_FindN` (stops at first zero-length line).

Line format used by all navigation (+Verif.s:5041–5165): byte 0 = line length in **words**, byte 1 = indent, tokens from offset 2. A `Procedure` line (`_TkProc` at +2) has long at +4 = size of the procedure body to `End Proc`, word/flags at +10 (bit 7 of byte 10 = folded/closed, bit 4 = compiled, bit 5 = locked/encoded, …); a closed procedure is skipped by `12+2+long(4)`. Locked procedures are XOR-encoded (`ProCode`).

### 1.5 Library file format (AMOSPro.Lib & extensions) — `Library_Load` (+B.s:2177)

After a `$20`-byte hunk header: `long` = number of routines ×2, `long` token-table size, `long` code size, `long` title size; optional `"AP20"` marker (new 2.0 format, flag `LBF_20`); then one `word` per routine = routine size in words; then token table + code + title. (In source: +Lib.s:27–32.) Routine address table is built *below* the token table (`-LB_Size-4*(n+1)` from the library base `AdTokens[n]`).

**Token table / token numbers**: a token value is the **byte offset of its entry in the main token table `C_Tk`** (+Lib.s:48); e.g. `_TkVar=$06`, `_TkFor=$23C` (list in +Equ.s:1992–2122). Entry = `dc.w instr_routine, func_routine` + name (lower-case, last char |$80) + parameter spec bytes terminated by `-1` (or `-2` = "alias"/continuation entry), word-aligned. Spec string: first char `I` = instruction, `0/1/2` = function returning int/float/string, `V` = reserved variable, `C` = constant, `O` = operator; following chars are parameter types separated by `,`: `0` int, `1` float, `2` string, `3` any, `4` math (float, calls math fn), `5` angle (float in degrees/radians). `Library_Reloc` (+B.s:2361–2440) converts these into a flag word (+Equ.s:2373: `L_FFloat`, `L_FAngle`, `L_FMath`, `L_VRes`, low 6 bits = param count) and patches routine pointers; `Library_GetParams` (+B.s:2694) inserts a `JSR` to the parameter-collection routine (`L_Parameters` table: integer 0–5 params, float, angle, math, reserved var) in the 4-byte `"GetP"` slot in front of each routine. Optional trailer blocks: `"FSwp"` (single/double float routine swap indices `LB_DFloatSwap/LB_FFloatSwap`), `"ComP"` (compiler data), `"KwiK"` (fast verify table swapped in during test by `Ver_Verif`/`Ver_Run`).

Extensions are numbered 1..26 (`AdTokens[1..26]`); default config: 1 Music, 2 Compact, 3 Request, 4 3D, 5 Compiler, 6 IOPorts (+Interpreter_Config.s:118–123). An extension token in a program is `_TkExt` ($4E) followed by extension number and offset (see Verif part).

### 1.6 Interpreter config (`+Interpreter_Config.s`) — values matter for defaults

`PId1` block (offsets from PI_Start, +Interpreter_Config.s:27–90 / +Equ.s:1525–1590):
0 PI_ParaTrap (actualisation addr), 4 PI_AdMouse, 8 bobs=68, 10 default screen Y position=50, 12 copper list size=12 KB, 16 sprite lines=128, 20 `PI_VNmMax`=16 KB variable-name buffer, 24 `PI_TVDirect`=42*6 direct-mode variable bytes, 26 `PI_DefSize`=320 KB program text buffer, 30/32 dir name size 30 / max 128, 34 `PI_PrtRet`=1 (CR on Print chr 10), 35 icons=0, 36 auto-close WB=0, 37 allow Close Workbench=1, 38/39 allow Close/Kill Editor=1, 40–42 file selector sort/size/store, 48–56 Read Text screen 640×256 at 129,50 speed 8, 58–66 file selector 448×158 at 177,70 speed 8, **68 default screen 320×256, 4 planes, 16 colours, mode 0, back colour 0**, **80 default palette** `$000,$A40,$FFF,$000,$F00,$0F0,$00F,$666,$555,$333,$733,$373,$773,$337,$737,$377` (+16 zeros), 144/146 default screen window pos, 148 default Amiga-A key `$00404161`.

`PIt1` text block — message list format: repeated `0, len, chars…` entries, terminated `0,$FF`. Lookup `Sys_GetMessage`/`GetMessage` (+B.s:551–575): 1-based index. Entries: 1 `APSystem/`, 4 `Def_Icon`, 5 `AutoExec.AMOS`, 6 `AMOSPro_Editor`, 7 `AMOSPro_Editor_Config`, 8 `AMOSPro_Default_Resource.Abk`, 9 system equates file, 10/11 monitor + its resource, 12/13 help, 14 `AMOSPro.Lib`, 16–41 extensions 1–26, 43 `Par:`, 44 `Aux:`, 46 cursor flash string, 47 directory negative filter.

### 1.7 Shutdown (`TheEnd`, +B.s:96)
Calls `Sys_EndRoutines`, closes files/printer, frees equates/includes, `Ed_End`, frees EDT and PRG structures, unloads editor/monitor, frees resources/mouse, reopens WB, stops libraries (routine at ExtAdr+8 for each extension), stops amos.library, closes libraries, prints `Panic` message (CLI: Write to Output; WB: AutoRequest), frees data zone, replies WB message.

---

# Part B: Verification / test pass

## AMOS Professional — the Verification / Test pass (`+Verif.s`)

Source root: `AMOS-Professional-365/`. Line numbers refer to the original files (ISO‑8859‑1).
All structures are big‑endian 68000. `a5` = global data zone (fields defined in `+Equ.s`),
`a6` = program pointer during the test.

---

### 1. Entry points and when they run

| Routine | File:line | Caller / when |
|---|---|---|
| `Prg_RunIt` | +Verif.s:4336 | RUN (editor F1, `Run`, `PRun`, accessories). Pushes program context (`Prg_Push`), `ClearVar`, **`PTest`**, stores results into the program structure, then starts the interpreter (`L_New_ChrGet`). Called from +Edit.s:7963/8125/8147/8178, +ILib.s:1490/1553. |
| `Prg_TestIt` | +Verif.s:4406 | TEST only (editor "Test", before folding a procedure, Indent, Check 1.3). Same as RunIt but `Prg_Pull`s afterwards and returns. Called by `Ed_VaTester` (+Edit.s:8567) only if `Prg_StModif(a6)` ≠ 0 (program modified since last test). |
| `PTest` | +Verif.s:73 | The real whole‑program test (phases + passes). |
| `VerDirect` / `Ver_Direct` | +Verif.s:43 | Direct‑mode line (+Edit.s:9234). Tests the single tokenised line in `Ed_BufT` with `DirFlag=1`, `Phase=1`. |
| `SsTest` | +Verif.s:225 | Sub‑test: one *phase* (main program or one procedure) = pass 1 + pass 2. |
| `Ver_Verif` / `Ver_Run` | +Verif.s:5203/5206 | Swap the token tables between "verif" and "run" layouts (see §2.3). |

Both RunIt/TestIt (+Verif.s:4357‑4363, 4426‑4432) after `PTest` do:
`Prg_StModif=0` (tested), `Prg_Not1.3 = VerNot1.3`, `Prg_MathFlags = MathFlags` (double precision/math usage).
Errors never return: `VerErr` (+Verif.s:712) restores the run tables (`Ver_Run`), maps the error position
back from the include‑expanded source (`Includes_Adr`), closes equate file, frees tables, `Prg_Pull`s, and
jumps to `Prg_JError` with **d0 = –error number** (negative = test‑time message table) and `a0=0`.

#### 1.1 `PTest` sequence (+Verif.s:73‑200)

1. `VerNot1.3=0`; `Prg_Run = Prg_Source`; `Get_Includes` (+Verif.s:4108) builds `Prg_FullSource` if any
   `Include "file"` lines exist → `Prg_Run = Prg_Test = FullSource`.
2. `ResVarBuf(8 KB)` (+4045), `ResVNom(PI_VNmMax)` (+4084); `Phase=0, ErrRet=0, DirFlag=0, VarBufFlg=0,
   Stack_Size=51, Prg_Accessory=0, MathFlags=0, Ver_SPConst=Ver_DPConst=0, VerNInst=0`.
3. **Phase 0** (`.ReVer`): `VarLong=0`, local name table = just under `DVNmBas`; `SsTest`. If `SsTest`
   returns non‑zero (only `Set Buffer` with a new size, +Verif.s:816‑821) the whole phase 0 restarts.
4. Keep main TablA in `Ver_MainTablA`; global name table becomes `DVNmBas..DVNmHaut`; `GloLong = VarLong`.
5. **Phase n>0**: for every TablA entry whose `Vta_Flag == 1<<VF_Proc` (one per `Procedure`), set
   `Prg_Test = Vta_Prog` (address of the `Procedure` token), `Phase++`, fresh local name table, `Locale`
   (hide non‑`Global` globals), `SsTest`, then **poke the local variable area size into the Procedure
   token +6** (`move.w VarLong,6(a0)`, +Verif.s:146‑147).
6. Allocate global variable area just below the label table (§4.5), clear it, `Globale` (mark every global
   visible – for direct mode), free tables, `Ver_Run` (restore run token tables).
7. 1.3 compatibility: any bank number > 16 → `VerNot1.3=1` (+186‑196).

#### 1.2 `SsTest` (+Verif.s:225‑491) — one phase = pass 1 + pass 2

* Clears `ErrRet, Passe, Ver_NBoucles, Ver_PBoucles`; `Ver_Verif` (verif token tables);
  `Reserve_Reloc`; `Reserve_TablA`; `a6 = a3 = Prg_Test`.
* **Pass 1** = the dispatch loop `VerD`/`VerDd`/`VerLoop` (§2.4) walking every line/instruction, checking
  syntax and types, creating variables/labels, writing **relocation records** (§3.3) and **TablA**
  structure records (§6.1).
* End of program (line header word = 0), or `End Proc` in a procedure phase, jumps to `VerX` (+496):
  **Pass 2**: `Passe=1`, emit `Reloc_End`, replay the relocation stream (patch variable offsets, resolve
  labels and procedure calls), then call every TablA entry's `Vta_Jump` (loop/if/goto resolution).
* Delayed error (`ErrRet`, set by `ERetard` +595) is raised at the end; else return d0=0.

#### 1.3 State built by the test (what the interpreter relies on)

* Token stream patched in place: variable offsets, label offsets, loop/If/Exit/On/Data distance words,
  Procedure size/varsize fields, token substitutions (overloaded instruction variants, `_TkAd2/_TkAd4`,
  `_TkPro`, `_TkLGo`), extension parameter counts, resolved equates.
* Label table (`LabBas..LabHaut`) in VarBuf, global variable area (`VarGlo`, size `GloLong`).
* `MathFlags` (bit7 double, bit1 math lib, bit0 floats), `Stack_Size`, `Prg_Accessory`, `VerNot1.3`,
  `Ver_SPConst/Ver_DPConst` (precision‑mismatch warning, +Edit.s:8427‑8437).

---

### 2. Tokenised program format as seen by Verif

#### 2.1 Lines

```
+0  byte  line length in WORDS (header + tokens + terminator); 0 = end of program
+1  byte  indent (leading spaces + 1, max 127)            (+Edit.s:14236‑14245)
+2  word  token ... token
    word  0  (end‑of‑line terminator)
```
A program ends with a line whose header word is 0. Tokens are word offsets into a library's token table
(main library: offset from `C_Tk` in +Lib.s:46; operators: negative words). `VDLigne(a5)` = address of the
current line header (+Verif.s:244).

#### 2.2 Token table entries (+Lib.s:46 ff, `;TOKEN_START`..`;TOKEN_END`)

Run layout of one entry (each entry starts word‑aligned):

```
dc.w  L_Instr, L_Func        ; library routine numbers for instruction / function use (L_Nul=1 = none)
dc.b  "nam","e"+$80          ; name, last char | $80 ($80 alone = no name / variant)
dc.b  T, params..., -1|-2    ; type char + parameter spec, terminated by -1 (end) or -2 (another variant follows)
(even)
```
* A name starting with `!` = first of an overloaded group; following variants have empty name (`$80`) and are
  linked by the `-2` terminator (e.g. `!logic`/variant, +Lib.s:118‑121; `!mid$` +Lib.s:288‑291).
* Type char `T`:
  `I` instruction; `0` integer function; `1` float function; `2` string function; `3` any;
  `4` integer‑or‑float ("indifferent numeric" e.g. `str$`, "24"); `5` math (angle) function
  (Ope_CheckType jump table, +Verif.s:2663‑2678); `V` reserved variable (followed by its own type char,
  e.g. `"V0"`); `C` constant; `O` operator.
* Parameter characters: `0` integer, `1` float, `2` string, `3` any type, `4` int/float; separators `,`
  (comma) and `t` (`To`). E.g. `"I0,0"` = instruction with two integer params; `"22,0"` = string function
  (string, integer); `"I0t0"` = `x To y`.

#### 2.3 The verification ("KwiK") table and table swapping

Each `dc.w` line carries a comment `//$AA$BB$CC$DD`. `c/Make_Toktable.Asc` extracts those 4 bytes for every
token into `+Toktab_Verif.Bin`, included in the library as chunk `"KwiK"` (+Lib.s:1685‑1688); its address is
`LB_Verif(lib)` = –24 (+Equ.s:2245). `Ver_Echange` (+Verif.s:5207‑5249) walks the 27 `AdTokens` slots
(main + 26 extensions) and, for libraries having a KwiK table, **swaps the first 4 bytes of every token
entry** with the KwiK entry. Flag `LBF_Verif` (bit 0 of `LB_Flags`, –12) records the current state.
`Ver_Verif` installs verif bytes, `Ver_Run` restores routine numbers.

In verif layout an entry's first 4 bytes are:

| byte | meaning | used at |
|---|---|---|
| 0 `AA` | **instruction class** (dispatch index for statement use; $FA‑$FF negative = line‑start only) | `VerDd`/`VerLoop` `move.b 0(a0,d0.w),d1` (+255, +276) |
| 1 `BB` | **function/operand class** | `Ope_Loop` `move.b 1(a0,d0.w),d1` (+2589) |
| 2 `CC` | byte offset from entry start to the type char (= 4 + name length) | `Ver_DInst` (+3179‑3187) |
| 3 `DD` | not read by Verif (compiler class; $12 on flow tokens, $0A Procedure, $0B End Proc, …) | — |

`Ver_DInst` returns d0 = type char and a0 = first param char (if the byte is negative—no type—a0 is backed
up). `Ver_OlDInst` (+3191) is the extension/"old" way: skip 4 bytes + name to find the type char.

#### 2.4 Statement dispatch (pass 1)

* `VerD` (+244): store `VDLigne`, read line header; 0 → pass 2 (`VerX`).
* `VerDd` (+250): first token of a line. Negative class (≥$FA) goes to the line‑start table:
  FA/FB `Global`/`Shared` → `VerSha`; FC `Def Fn` → `VerDFn`; FD `Data` → `VerData`;
  FE `End Proc` → `V1_EndProc`; FF `Procedure` → `V1_Procedure`.
* `VerLoop` (+271): subsequent instructions; the same tokens there are errors: FA/FB → 15, FC/FD → 4,
  FE/FF → 16. Token 0 word → next line; negative token (operator) → syntax error 35.
* After most instructions `VerDP` (+483): next must be `0` (EOL), `:` (`_TkDP`) or `Else` (left for the
  loop), else error 35.

Instruction class table (+Verif.s:291‑383):

| cls | handler | cls | handler | cls | handler |
|---|---|---|---|---|---|
|00|Ver_Normal (VerI)|1A|Call|34|While|
|01|syntax error|1B|Menu$(..)=|35|Wend|
|02|Rem|1C|Menu Del|36|Do|
|03|Set Buffer|1D|Set Menu|37|Loop|
|04|Set Double Precision|1E|Menu Key|38|Exit|
|05|Set Stack|1F|Menu misc|39|Exit If|
|06|Variable (assign / proc call)|20|Fade|3A|If|
|07|Label definition|21|Sort|3B|Else|
|08|`_TkPro` proc call|22|Swap|3C|Else If|
|09|Dim|23|Follow|3D|End If|
|0A|Print|24|Set Accessory|3E|Goto|
|0B|Print #|25|Trap|3F|Gosub|
|0C|Input|26|Struc|40|On Error|
|0D|Input #|27|Struc$|41|On Break Proc|
|0E|Dec|28|Extension token|42|On Menu|
|0F|Proc|29|AMOSPro normal (Not1.3)|43|On|
|10|debug (Debug=2) / normal|2A|AMOSPro already‑tested|44|Resume|
|11|Default Palette|2B|reserved variable|45|Resume Label|
|12|Palette|2C|AMOSPro reserved var|46|Pop Proc|
|13|Read|2D|already‑tested normal|47|Every|
|14|Restore|2E|(free → VerD)|48|LPrint|
|15|Channel|2F|end of line|49|Line Input|
|16|Inc|30|For|4A|Line Input #|
|17|Add|31|Next|4B‑4E|Mid$(3)/Mid$(2)/Left$/Right$ as instruction|
|18|Polyline/Polygon|32|Repeat|4F|Add (4 params)|
|19|Field|33|Until|50 dialogs (Pro), 51 Dir, 52 Then→syntax, 53 Return, 54 Pop, 55 machine‑code proc (Pro), 56 Bset/Bchg/Ror…, 57 APCmp → syntax|

"Pro" classes call `SetNot1.3` (+214): sets `VerNot1.3`; if `VerCheck1.3` (editor "Check 1.3") → error 47.
Every instruction except Rem/Set Buffer/Set Stack sets `VarBufFlg` bit 0 ("program has begun").

#### 2.5 Generic instruction / function checking (`VerI`, `VerF`, `VerC`)

`VerI` (+2989, instruction; a6 = after token) and `VerF` (+3061, function; params in `(` `)`) evaluate each
parameter with `Ver_Evalue` and build on the stack a byte list
`type, sep, type, sep, …, $FF` (type = `"0"` numeric or `"2"` string; sep = `","` or `"t"`; max 19 bytes),
then call `VerC` (+3120):

* Spec pointer `22(sp)`; null = procedure (accept).
* Compare spec param vs actual: `3` matches anything; `2` must be string; any other spec digit (`0`,`1`,`4`)
  must be numeric `"0"` → mismatch = **error 40 Type mismatch** immediately.
* Separators must match exactly; if counts or separators differ, skip to the spec terminator: if it is
  **–2**, align to even, compute the next variant's token value (`a1 – VerBase`) and **overwrite the
  instruction token in the program** (`move.w d0,(a0)` where a0 = instruction address `26(sp)`), skip
  the variant's 4 header bytes + name + type char (+ extra char if `V`) and retry. Terminator –1 → error 35.
* `VerI/VerF` return d0 = number of params `(count+1)/2` (used for old extensions).

`VerI_DejaTestee`/`VerF_DejaTeste` (+2972/+3042, class 2D/2A and function class 19): evaluate each parameter
following the spec skeleton without any type check or variant search.

`Ope_InstFonction` (function class 18, +2735): token whose first variant is an instruction (`I`); walks the
`-2` chain to the first non‑`I` variant, rewrites the token to it, then `VerF`.

#### 2.6 Expressions (`Ver_Evalue`, +2528‑2578; operands `Ver_Operande`, +2582‑2967)

* Operator tokens are negative words = negative byte offsets of the operator's entry before `Tst_Jumps` in
  `Tst_Operateurs` (+5254‑5293); entry = `bra handler` + name + `"Oxx"`. Precedence = unsigned token value
  (closer to `Tst_Jumps` = higher): `xor < or < and < comparisons (<> >< <= =< >= => = < >) < + - < mod * /
  < ^`. `_TkEg=$FFA2`, `_TkM=$FFCA`, `_TkPow=$FFF6`. Sentinel `$7FFF` ends at any non‑operator token.
* Operator type rules: `Tst_Chiffre` (both numeric), `Tst_Comp` (equal types → result numeric),
  `Tst_Mixte` (`+`,`-`: equal types, result that type), `Tst_Puis` (`^`: also `MathFlags |= %11`).
* `Parenth(a5)` counts parentheses; a function argument list ends when it reaches –1.
* Operand classes (byte 1): 00 normal, 01 syntax, 02 end (omitted param allowed only after `,`), 03 `,`
  (omitted param → integer), 04 `(`, 05 Val, 06 extension, 07 variable, 08 Varptr, 09 Fn, 0A Not,
  0B X/Y Menu & Menu$, 0C Equ, 0D Match, 0E Array, 0F Min, 10 Lvo, 11 Struc, 12 Struc$, 13 math function
  (`MathFlags|=%11`), 14 integer const (skip 4), 15 float const (skip 4, `Ver_SPConst=1`, MathFlags bit0),
  16 double const (skip 8, `Ver_DPConst=1`, Not1.3), 17 string const (word len + text, even), 18 inst+func,
  19 already tested, 1A reserved variable, 1B‑1F ParamE/F/S/False/True, 20 Max, 21‑24 Mid$/Left$/Right$,
  25 dialog functions, 26 file selector, 27 Btst.
* `Ope_CheckType` (+2663) converts the function's result type to the verifier's two types: `1` (float) sets
  MathFlags bit0, `5` sets bits 0+1, `1`/`4`/`5` become `"0"`. **Verif only distinguishes numeric vs
  string**; int↔float conversion is done at runtime.
* Constant tokens: `_TkBin $1E`, `_TkHex $36`, `_TkEnt $3E` (+ long), `_TkFl $46` (+ 4‑byte float),
  `_TkDFl $2B6A` (+ 8‑byte double), `_TkCh1 $26`/`_TkCh2 $2E` (+ word length + chars, padded even).

---

### 3. Variables

#### 3.1 Variable token (`_TkVar` = $0006)

```
+0 word  $0006  (_TkVar)  — or _TkPro $0012 (proc call), _TkLab $000C, _TkLGo $0018 (same shape)
+2 word  offset           — 0 from tokenizer; patched by Verif (see 3.3)
+4 byte  name length      — even (name padded with 0)
+5 byte  flags
+6       name (lower case), length bytes
```
Flags byte (+Edit.s:14382‑14404 sets low nibble; Verif clears upper nibble then sets bits):

| bits | meaning |
|---|---|
| 0‑3 | type: 0 integer, 1 float (`#` suffix), 2 string (`$` suffix) |
| 3 | Def Fn function name (`bset #3`, +877, +2898) — note it overlaps the low nibble mask, so a Fn name has flag 8+type |
| 6 | array (`bset #6`, +838, +1485, +3219, +3248) |
| 7 | procedure name / procedure call (`or.b #$80`, +1549, +3277) |

`VarA0` (+3659): a0 = address after the name; d2 = `"2"` if (flag&7)∉{0,1} else `"0"`.

#### 3.2 Variable name table (VNm buffer)

Separate buffer of `PI_VNmMax` bytes (`ResVNom` +4084). Tables grow *downwards*; entry list is scanned from
its `Bas` upward until a zero length byte.

```
+0 byte  name length
+1 byte  flags (as token, upper nibble only bit6/bit3)
+2 word  offset in the variable area (VarLong at creation)
+4 byte  number of dimensions (arrays, written by Dim: move.b d0,4(a1) +843)
+5 byte  visibility: 0 = local/hidden, 1 = visible (Shared in current proc / all after Globale), 2 = Global
+6       name (length bytes)
```
* Global (main program) table = `DVNmBas..DVNmHaut`; procedure local table placed directly below it
  (`VNmBas..VNmHaut`, `VNmHaut = DVNmBas`) and rebuilt for each procedure.
* `V1_StoVar` (+3521‑3630): in phase>0 first search the global table for an entry with visibility ≠ 0 and
  equal length+flags+name → store `DVNmHaut – entry` (**positive**) into token +2, result "existing".
  Otherwise search local table; if absent create entry (error 37 if below `VNmMini`):
  `offset = VarLong; VarLong += 6`, **+4 more if type float and `MathFlags` bit7 (double) and not an
  array** (flag byte exactly 1). Token +2 = `entry – VNmHaut` (**negative**). Returns Z=1 existing,
  Z=0 new. Then emits a `Reloc_Var` record and skips the variable.
* Pass 2 `V2_StoVar` (+3634): token word negative → local: replace by `entry.offset` (≥0).
  Positive → global: replace by `–(entry.offset + 1)` (negative).
* Runtime (`InVar`/`FnVar`/`FindVar`, +ILib.s:3734, 3807, 3864): offset ≥0 → `VarLoc + offset + 2`;
  negative → `VarGlo + (–offset) + 1` = `VarGlo + offset_real + 2`. Each slot = `byte length (4 or 8),
  byte type flag, value (4 or 8 bytes)`; `Long_Var` = `$04040404` or `$04080404` in double mode
  (+ILib.s:631/648). Array slot holds a pointer to the array block (built by `InDim`).

#### 3.3 Relocation stream (pass 1 → pass 2)

Buffers of `Reloc_Step`=1024 bytes, linked by first long (`New_Reloc` +3731, limit = buffer+1024‑32).
Each record = position delta from previous record (`a3`) then a code byte:
* delta ≤ 254: one byte `delta/2` (bit7 clear);
* 254 < delta ≤ 762: byte 127 (=skip 254) repeated;
* ≤ 65534: `Reloc_Long $84, hi, lo`; larger: `$84,$FF,$FE` repeated.
* Codes (bit7 set): `$80 End, $82 Var, $84 Long, $86 NewBuffer, $88 Proc1, $8A Proc2 (+ raw type byte
  via Out_Reloc), $8C Proc3, $8E Proc4, $90 Debug, $92 Label` (+Verif.s:27‑36).
Decoder (+506‑534): bit7 clear → `a6 += byte*2`; else dispatch `(code&$7F)*2` into a `bra` table.
`Ver_NoReloc` suppresses records (used by `VerMid`, which evaluates its variable twice).

#### 3.4 Globals, Shared, Global

* `VerSha` (+3826‑3892), line‑start only, must be alone on its line (else 15), illegal in direct mode (7).
* **Phase 0** (main): `VpSha` (+3803) creates the variables in the main table (arrays must be written
  `a()` with empty brackets → else error 14). During phase 0 procedure bodies are skipped but their
  `Shared`/`Global` lines are fed to `VpSha` too (+1594‑1606), so shared names always exist in main.
  Then each name is looked up and marked visibility **2 (Global)**.
* **Phase n**: `Locale` (+3499) resets every global entry with visibility ≠2 to 0. A `Shared` (or
  `Global`) statement inside the procedure looks the name up in the global table (not found → error 38)
  and marks it 1 unless already 2. Subsequent uses resolve to the global (positive offset).
* After all phases `Globale` (+3510) sets every global to 1 (direct mode sees all main variables).
* Unused messages 11‑13 (VerShp/VerAlG/VerPaG) exist but are never raised in this version.

#### 3.5 Arrays, Dim, Def Fn

* `Dim` (+831): variable with bit6 must be **new** (existing → error 39); `VerTablo` (+3675) checks each
  index is numeric and counts dimensions → stored in VNm +4. Multiple arrays separated by `,`.
* Any array use (`VerGV` +1478, `V1_IVariable` +3207, `V1_FVariable` +3243) requires the array to exist
  already (textually earlier Dim, else error 38); instruction/function uses also require the same number
  of indices (error 19).
* `Def Fn name[(params)] = expr` (+873): line‑start, alone on the line (error 4); name flag bit3; params are
  ordinary variables; expression type must equal name type (40). `Fn name(...)` (+2895): the Fn
  variable must already exist (error 2).
* `Sort a()` requires an array; `Swap a,b` same types; `Inc/Dec/Add` need a numeric scalar
  (`VerVEnt` +1460). `Add v,e` → token becomes `_TkAd2`; `Add v,e,b To t` → `_TkAd4` (+1213‑1233).

#### 3.6 Variable area size

* Main: `GloLong` = phase‑0 `VarLong`. Placed under the label table (+154‑172):
  `LabBas – 6` holds a sentinel (`word $FFFF`, `long 0`), globals occupy `GloLong` bytes below; error 8 if
  that crosses `HiChaine`. `VarGlo = VarLoc = TabBas` = its bottom.
* Procedure: `VarLong` of the phase (params first, then locals) poked into Procedure token +6;
  `CallProc` (+ILib.s:2482) allocates that many bytes below `TabBas` per call.
* VarBuf layout (`ResVarBuf` +4045): `[ChVide word][strings ↑ … HiChaine] … [arrays ↓ TabBas][globals]
  [$FFFF + long][labels LabBas…][0 word][LabHaut=end]`. Default 8 KB for programs, 4 KB for direct mode,
  `Set Buffer n` → n·1024.

---

### 4. Labels, procedures, calls

#### 4.1 Label definition (`_TkLab` $000C) and label table

* Tokenizer makes `_TkLab` from a name or number at line start followed by `:` (+Edit.s:14355‑14359).
  `VerLab` (+764) → `V1_StockLabel` (+3413). `Data` may follow a label (`label: Data …`).
* Lookup key (`Get_Label` +3462) = long `[len byte][flags byte][phase word]` + name; labels are
  **per phase** (each procedure has its own label space; phase 0 = main). Duplicate → error 42.
* Entry (grows down from `LabHaut`, scanned from `LabBas` upward, zero‑terminated):
```
+0 byte len   +1 byte flags   +2 word phase
+4 long jump address
+8 name (len bytes)
```
  Jump address = the token after the label; if that is EOL and another line follows, the first token
  of the next line (skips terminator and next line header) (+3434‑3441). Overflow below `LabMini` → error 8.
* Reference token (pass 2 `V2_FindLabel` +3446): token +2 word = `(entry+4) – LabHaut` (negative);
  runtime `move.l LabHaut(a5),a0; move.l 0(a0,d0.w),a6` (+ILib.s:2868‑2873, `Goto2` +2746).

#### 4.2 Goto/Gosub targets (`V1_GoLabel`, +3362)

`_TkLGo` (line number written after Goto/Then/Else) is first turned into `_TkVar`. A `_TkVar` with type
nibble 0, followed by EOL, `,`, `:`, `Then` or `Else` is a **label**: token rewritten to `_TkLGo`
($0018) and a `Reloc_Label` record emitted; pass 2 resolves it (undefined → error 41). Anything else is
evaluated as an expression (computed label by name or number at runtime, `GetLabel` +ILib.s:2860).

* `Goto` (+2167) also creates a TablA entry (`VF_Goto`, `V2_Goto`) → pass 2 `Goto_Loops` (+2462): the
  target's loop depth (NBoucles of the last TablA entry at/before the target) must not exceed the Goto's
  depth, else **error 9** "No jumps allowed into the middle of a loop".
* `Gosub`, `Restore label`, `Resume Label label`, `Every n Gosub` → `V1_GoLabel` only.
* `On e Goto|Gosub|Proc l1,l2…` (+2184): tokenizer reserves 4 bytes after `On` (+Edit.s:14594):
  `+0 word` = byte length of the label list (end – placeholder – 4), `+2 word` = number of targets.
  `On … Goto` adds TablA `V2_OnGoto` (loop check for each label). `On … Proc` uses
  `V1_GoProcedureNoParam`.
* `On Error Goto l` → `V1_Goto`; `On Error Proc p` → continues as `Proc` instruction; `Resume [l]` → Goto
  check; `On Break Proc p` = `V1_Proc`; `On Menu Goto|Gosub|Proc a,b,…` (+1061).

#### 4.3 Procedure header line

Tokenizer reserves 8 bytes after `_TkProc` (+Edit.s:14612). With **T** = address of the `Procedure` token
(line+2):

```
T+0  word  _TkProc ($0376)
T+2  long  distance: (address of End Proc line's 0 terminator) – T – 10   (poked +1614‑1617)
T+6  word  size of the procedure's local variable area (poked by PTest +147);
           for machine‑code procs = value copied into VarLong (+1671)
T+8  byte  flags: bit7 folded/closed, bit6 locked (cannot be opened), bit5 currently encrypted,
           bit4 compiled / machine‑code procedure (+Edit.s:8653‑8658, 8733 "%0101<<8")
T+9  byte  encryption seed (ProCode +5167)
T+10 word  _TkVar  name token: T+12 offset(label), T+14 len, T+15 flags (=$80|type), T+16 name
     then optional  _TkBra1 '['  _TkVar p1 , _TkVar p2 ... _TkBra2 ']'   then 0 (must end line, else 16)
```
* Runtime skip: `InProcedure` (+ILib.s:2464) `a6 = T+6+4+distance` = End Proc terminator.
  `Tk_FindN`/`Prg_CptLines` skip closed procedures with `T+2` (+Verif.s:4908‑4911, 5059‑5064).
* Locked & encrypted (bits 6 and 5) → `ProCode` decrypts in place before testing (+1530‑1534).

**Phase 0** (`V1_Procedure` +1519): TablA entry (`_TkProc`, `VF_Proc`, d1=4 extra); stores the name as a
**phase‑0 label** whose address = T; validates params syntax (clears their flag high nibble); scans lines
until `End Proc` (EOL‑0 header before → error 17), feeding Shared/Global lines to `VpSha`; compiled
(`_TkAPCmp` $2BF4 on the next line) → `Ver_APCmp` (+1625) relocates the compiled code (header in
+CEqu.s:189‑194: `APrg_MathFlags w, APrg_Relocation l, APrg_EndProc l, APrg_OldReloc l`, code at +16;
reloc bytes: 0 end, 1 = skip 508, n = advance n·2 and add delta) and ORs its math flags/accessory.
**Phase n** (`V1_ProcedureIn` +1656): parameters are created as the first local variables (offsets 0,6,…).
`End Proc [ [expr] ]` (+1676) ends the phase (→ pass 2); in phase 0 → error 18; `Pop Proc` in main → 18.

#### 4.4 Procedure calls (`V1_CallProc` +3270, pass 2 +3306‑3355)

A statement that is a variable not followed by `=` or `(` is a procedure call (`V1_IVariable` +3207); also
`Proc name` (class 0F, must become a call else error 20). Pass 1: token → `_TkPro` ($0012), flags |= $80,
records `Reloc_Proc1`, then per bracket argument `Reloc_Proc2` + the argument's type byte, `Reloc_Proc3`
at `]`, or `Reloc_Proc4` if none. Illegal in direct mode (7).
Pass 2: `Proc1` finds the phase‑0 label (error 20 if absent), patches the label offset into the call
token, sets a3 to the definition's parameter list (or 0). `Proc2` checks arity (error 19) and that the
param variable's type (float counted as numeric) equals the argument type (error 40). `Proc3`/`Proc4`
check no parameters remain (19). Runtime `CallProc` reads `LabHaut[offset]` = T.

---

### 5. Loops, tests, structure (TablA)

#### 5.1 TablA record (+2293‑2303), chunks of 1024 bytes (`Init_TablA` +2349)

```
+0  long Vta_Next     +4  long Vta_Prev     +8  word Vta_Token
+10 byte Vta_Flag  (1<<VF_Boucles=1, VF_If=2, VF_Proc=4, VF_Exit=8, VF_Goto=16, VF_Debug=32)
+11 byte Vta_UFlag    +12 long Vta_Prog (a6 at creation = placeholder address)
+16 word Vta_NBoucles (loop depth)  +18 word Vta_PBoucles (loop‑stack bytes)
+20 long Vta_Jump (pass‑2 handler; closers initially point to an ERROR routine)
+24 ...  Vta_Variable (d1 extra bytes)
```
Openers increment `Ver_NBoucles` and add the runtime loop‑stack frame size to `Ver_PBoucles`
(`TForNxt=24`, `TRptUnt=TWhlWnd=TDoLoop=10`, +Equ.s:2368‑2371) **before** creating their record; closers
decrement before. Pass 2 calls each record's `Vta_Jump` in program order; an opener finds its closer
with `Find_TablA` (+2380: next record with `NBoucles == opener‑1`, same flag, given token) and clears the
closer's `Vta_Jump` (so an unmatched closer later raises its error). `Find_End` (+2444) = address after the
closer: past `:`; or, at EOL with a following line, the first token of the next line.
`Doke_Distance` (+2434): `word = target – field – 2` (> $FFFF → error 10).

#### 5.2 Patched fields (placeholders reserved by the tokenizer, +Edit.s:14571‑14615)

| Statement | bytes after token | Verif writes | runtime use |
|---|---|---|---|
| `For v=a To b [Step c]` | 2 | word → after matching `Next` | +ILib.s:2075 exit address |
| `Repeat` | 2 | word → after `Until expr` | +ILib.s:2185 |
| `While e` | 2 | word → after `Wend` | +ILib.s:2232 |
| `Do` | 2 | word → after `Loop` | = Repeat |
| `Next [v]`, `Until e`, `Wend`, `Loop`, `End If` | 0 | — | — |
| `Exit [n]` | 4 | word distance (after closer – field – 4), word loop‑stack bytes to pop | +ILib.s:2297 |
| `Exit If e[,n]` | 4 | same | +ILib.s:2316 |
| `If e` | 2 | word distance to Else‑body / after End If / ElseIf placeholder; **bit0=1 → target is an Else If** | +ILib.s:2335 |
| `Else If e` | 2 | same as If | |
| `Else` | 2 | structured: → after End If; one‑line: → line terminator | +ILib.s:2369 |
| `On` | 4 | word list length, word count | +ILib.s:2788 |
| `Data` | 2 | word = offset from line header to this word | +ILib.s:4589 |
| `Procedure` | 8 | see 4.3 | |
| `Equ`/`Lvo`/`Struc`/`Struc$` | 6 | long value, byte type 0‑7, byte bit7=resolved | |
| `Rem`/`'` | 2 | (tokenizer) byte length of comment text | +ILib.s:1791 |

Details:
* **For/Next** (+1701‑1773): control variable must be numeric scalar/array element; start/limit/step
  same type as variable. `Next v` stores v's (pass‑1) offset; pass 2 compares with the For's variable
  (mismatch → error 33). Unmatched For → 33, Next → 34. Repeat/Until 31/32, While/Wend 29/30, Do/Loop 27/28.
* **Exit n** (+1894‑1929): n = integer constant (default 1). Target = closer record at depth
  `exit.NBoucles – n` (none/negative → error 26); stack delta = `exit.PBoucles – closer.PBoucles`.
* **If** (+1953): record holds (placeholder, line start). With `Then` → one‑line If (UFlag=1,
  `V2_IfThen` +1978): searches same‑line `Else` (counting nested one‑line Ifs) → Else gets `V2_ElseThen`;
  target is after the Else placeholder or the line's terminator. `Then <linenumber>` label is resolved
  and loop‑checked. Structured If (`V2_If` +2029) uses `Find_TablATest` (+2397): next Else/ElseIf/EndIf
  at the same nesting (nested Ifs counted); meeting **any one‑line If (UFlag≠0) during the scan raises
  error 25** "No THEN in a structured test" (faithful behaviour). Found Else → its handler set to
  `V2_Else`, ElseIf → `V2_ElsI` (chain), End If → handler cleared. Errors: If without EndIf 22,
  Else/ElseIf without If 21, EndIf without If 23. (Messages 24 and 44 exist but are not raised.)
* `Then` as an instruction → syntax error; `Trap` must be followed by an instruction (43).

---

### 6. Other statements

* **Rem** (`_TkRem1 $064A`, `_TkRem2 $0652` for `'`): skips `add.w (a6)+,a6` then the terminator;
  illegal in direct mode (+745). Does not set VarBufFlg.
* **Data** (+1117): only at line start (or after a line‑start label); writes the line‑start offset word,
  checks each value is an expression. No chain is built: `Read` scans lines at run time (+ILib.s:4602).
* **Set Buffer n** (+806): constant integer only; must precede all instructions (error 3); if the size
  differs, `VarBufFlg` bit2, re‑allocate VarBuf and **restart phase 0** (second change → error 3).
* **Set Stack n** (+787): constant, before instructions, once (else 50); `Stack_Size = max(n,10)+1`
  (default 51; direct 10). Loop stack = `(Stack_Size+1)*Stack_ProcSize(42)` bytes (`Stack_Reserve` +3983).
* **Set Accessory** (+825): `Prg_Accessory++`, Not1.3.
* **Struc/Struc$/Equ/Lvo** (+1262‑1456, +2935‑2967): `Equ_Verif` looks up `"\n"+["_LVO"]+name+":"` in the
  equates file (path from system message #9, loaded once into `Equ_Base`, flushed via a flush routine),
  parses `[-]value,type` (integer only) and caches it in the token's 6‑byte field (bit7 of byte 5 = done).
  Errors 51/52/53; Struc needs type<7, Struc$ type 6 (54). All are Not1.3.
* **Extension tokens** (`_TkExt $004E`, +402/+2699): `word $004E, byte ext# (AdTokens index 1‑26),
  byte param count, word token offset in that extension's table`. Extension not loaded → error 5.
  Verif pokes the param count (old libraries) or –1 for AMOSPro 2.0 libraries (`LBF_20`).
  As function an `I` token is a syntax error.
* Print/LPrint/Print#/Input/Line Input/Input#, Palette (≤32 values), Fade, Menu$, Set Menu, Menu Key,
  Channel … To Screen Display/Offset/Size/Bob/Sprite/Rainbow/address, Polyline/Polygon, Mid$/Left$/
  Right$ as l‑values, Field … As, Call, Follow, Read, Restore have dedicated handlers (+905‑1258).
* `Include "file"` (`_TkIncl $25B2`) is expanded before testing (`Get_Includes` +4108: header must be
  `"AMOS Pro101v"` (8 chars) or `"AMOS Basic v134 "` (10 chars); errors 45, 46, 35, 36). Chunk table
  `Prg_Includes`: 20 bytes/entry (`+0` addr, `+4` source length before include, `+8` handle,
  `+12` include size, `+16` include line length).

---

### 7. Double precision & float type selection

* `MathFlags(a5)` byte: bit0 floats used, bit1 math library (trig, `^`) needed, **bit7 double precision**.
* `Set Double Precision` (`_TkDPre $2B84`, class 04, +775): Not1.3; must be the first instruction
  (only `Set Buffer`, Rem allowed before; VarBufFlg bit0 → error 50) and only once (bit1 → error 50);
  `MathFlags = %10000011`. The tokenizer also sets MathFlags the same way when it tokenises the
  command (+Edit.s:14619‑14620), so constants are tokenised as `_TkDFl` (8 bytes).
* PTest clears MathFlags at start; compiled procedures OR in theirs. RunIt/TestIt copy it to
  `Prg_MathFlags`; `Prg_Save` writes it into header byte 15 (`H_Pro`, +4972) and `Prg_Load` reads it
  back (+4821‑4822). Header byte 11 = `'V'` tested / `'v'` untested.
* Effect: double‑mode scalar float variables get 10‑byte slots (6+4) in `V1_StoVar`; runtime uses
  `Long_Var=$04080404`, DFloat library and `ValPi`/`Val180` doubles (+ILib.s:626‑660). Single = Motorola
  FFP (`mathffp`).
* Mixed constants: `Ver_SPConst`/`Ver_DPConst`; the editor's Test command warns message 221
  "precision mismatch" when single constants appear in a double program or vice‑versa.

---

### 8. Test‑time errors (numbers passed negated to `Prg_JError`; texts +Editor_Config.s:760‑813)

| # | Message | Raised at (+Verif.s) |
|---|---|---|
|1|Bad structure (delayed, `VerCrs`)|defined 590, unused|
|2|User function not defined|2902|
|3|Variable buffer can't be changed in the middle of a program!|809, 818|
|4|This instruction must be alone on a line|287‑288, 889|
|5|Extension not loaded|412, 2708|
|6|Too many direct mode variables|unused|
|7|Illegal direct mode|746, 754, 765, 789, 807, 1503, 1523, 1678, 3272, 3829|
|8|Variable buffer too small|160, 3424|
|9|No jumps allowed into the middle of a loop!|2474, 2485|
|10|Structure too long|2062, 2438|
|11‑13|Shared‑related|unused|
|14|Use empty brackets when defining a shared array|3822, 3856|
|15|Shared must be alone on a line|285‑286, 3891|
|16|Procedure's limits must be alone on a line|289‑290, 1571|
|17|Procedure not closed|1590|
|18|Procedure not opened|1680, 2270|
|19|Illegal number of parameters|3224, 3253, 3324, 3330, 3333, 3354|
|20|Undefined procedure|1514, 3310|
|21|ELSE without IF|2072, 2087, 2110|
|22|IF without ENDIF|2034|
|23|ENDIF without IF|2143, 2152|
|25|No THEN in a structured test|2430|
|26|Not enough loops to exit|1915, 1918|
|27/28|DO without LOOP / LOOP without DO|1873 / 1884|
|29/30|WHILE without WEND / WEND without WHILE|1836 / 1847|
|31/32|REPEAT without UNTIL / UNTIL without REPEAT|1794 / 1805|
|33/34|FOR without NEXT / NEXT without FOR|1742, 1747 / 1758|
|35|Syntax error|`VerSynt` many; also bad Include (4241)|
|36|Out of memory|2334, 3781, 4055, 4094, 4235|
|37|Variable name's buffer too small|3596|
|38|Array not dimensioned (also Shared name not in main)|1488, 3221, 3250, 3862|
|39|Array already dimensioned|841|
|40|Type mismatch|`VerType` (VerC, assignments, For, operators, …)|
|41|Undefined label|533, 2020|
|42|Label defined twice|3416|
|43|Trap must be immediately followed by an instruction|2263|
|45/46|Cannot load included file / not an AMOS program|4237 / 4239|
|47|Instruction not compatible with AMOS 1.3 (Check 1.3 mode)|219|
|48/49|1.3 bank count / compatible (editor info)|+Edit.s:8448‑8451|
|50|This command must begin your program (but AFTER Set Buffer)|782|
|51‑54|Equate not defined / cannot load / bad format / wrong type|1393, 1445, 1397, 1300|

The error position reported is `VerPos(a5)` (updated before each instruction/expression), mapped back to
the original source by `Includes_Adr` (+4278).

---

### 9. Porting notes

* Verif is a two‑pass, per‑phase compiler front end that **mutates the token stream**; a Rust port should
  model it as `pass1` (walk, check, record fixups + structure records) then `pass2` (apply fixups, resolve
  structures) per main/procedure scope.
* Only two types exist for checking (numeric/string); the int/float distinction only affects storage size
  (double mode) and MathFlags.
* Overloaded instructions are resolved by **rewriting the token** to the matching `-2`‑chained variant;
  a Rust implementation needs the variant chain and param spec strings from +Lib.s.
* Several messages (1, 6, 11‑13, 24, 44) are defined but unreachable; the one‑line‑If inside a structured
  If quirk (error 25) is genuine behaviour of this source.

---

# Part C: Run-time execution model

## AMOS Professional: runtime execution model (interpreter core)

Source root: `AMOS-Professional-365/`. All line numbers refer to the original ISO-8859-1 files (a UTF-8 conversion keeps the same line numbers).
- `+ILib.s` is the interpreter core. `+Lib.s` does `Include "+ILib.s"` at +Lib.s:1710, so ILib is library #0, routines L_0.. .
- `+Lib.s` holds most instructions and functions.
- `+Equ.s` holds the data-zone layout and token constants.
- `+Verif.s` is the test/"verification" pass. It is covered by another agent, but it is cited here where it fixes runtime semantics.

Notation: `L_Xxx` is the library routine number Xxx. "Lib_Par" routines start with a "GetP" stub that gets patched into a parameter-gathering JSR (see §3). `a5` is the data zone (DZ) and `X(a5)` is a DZ variable.

---------------------------------------------------------------------------
### 1. Main dispatch loop ("ChrGet")

#### 1.1 Entry: `New_ChrGet` (+ILib.s:430-516)
- `sp = BasSp`, `a4 = AdTokens(a5)`, which is the base of the main token table.
- `Prg_InsRet = _IRet`. Every non-local jump (errors, Every, menus, GC restart) pushes this address so that `rts` lands back in the loop.
- `Stack_Reserve` allocates the loop/procedure stack. Then `a3 = PLoop = BasA3 = HoLoop` (the top of the stack) and `MinLoop = BaLoop + 64`.
- `Open_MathLibraries` (+ILib.s:627):
  - It sets the FFP constants `ValPi=$C90FD942` and `Val180=$B4000048`, and `Long_Var=$04040404` (bytes per type: int, float, string, -).
  - In double mode (`MathFlags` bit 7) it sets IEEE pi/180 and `Long_Var=$04080404`.
  - It then swaps the float routine table: `Lib_FloatSwap`, which swaps the `L_xxx`/`L_Dxxx` pairs between `Start_FloatSwap`..`End_FloatSwap` (+ILib.s:7266-7722).
- `AData=0`, `DefFlag=1`, `a6 = Prg_Run` (the program start). Also `DProc = PData = a6`.

#### 1.2 The loop (+ILib.s:472-516)
```
_NLine  tst.w (a6)+        ; line header word; 0 => end of program -> InEnd (error 10 "End of program")
_ILoop  move.w (a6)+,d0    ; token; 0 => end of line -> _NLine
_Inst   move.l a6,d7       ; d7 = address just AFTER current instruction token (instr start = d7-2)
        move.w 0(a4,d0.w),d1          ; token -> routine index (word in token table)
_Poke   move.l -LB_Size(a4,d1.w),a0   ; instruction address (function address is at -LB_Size-4)
        jsr (a0)
_IRet   move.w (a6)+,d0 ; bne _Inst ; tst.w (a6)+ ; bne _ILoop ; bra InEnd
```
**Line format.** Each line is:
- a header word: byte 0 is the line length in words, including the header; byte 1 is the indent.
- then the tokens;
- then a terminating `0` word.

`InData` (+ILib.s:4589) shows the length semantics: `line_start + len*2 - 2` is the terminator word. A header of `0` means end of program. Instructions on the same line are separated by the `:` token (`_TkDP=$54`, routine `InNull` = `rts`).

**Token encoding.** A token is a word.
- Positive values are byte offsets into the token table. The entry layout is `dc.w L_Instr, L_Func; dc.b name..., paramspec, -1`.
- Operators are negative offsets from `OP_Jumps` (§2.1).
- Fixed tokens are listed at +Equ.s:1994-2120. Examples: `_TkVar=$06`, `_TkLab=$0C`, `_TkPro=$12`, `_TkLGo=$18`, `_TkEnt=$3E`, `_TkFl=$46`, `_TkDFl=$2B6A`, `_TkCh1=$26`, `_TkCh2=$2E`, `_TkVir=$5C`, `_TkPar1=$74`, `_TkPar2=$7C`, `_TkBra1=$84` (`[`), `_TkTo=$94`, `_TkThen=$2C6`, `_TkElse=$2D0`, `_TkProc=$376`, `_TkEndP=$390`, `_TkData=$404`.
- A token is followed by inline operand words. `TInst` (+ILib.s:2962) gives the size of each inline operand. This matters for Resume Next/Trap skipping:
  - 2 extra bytes: For/Repeat/While/Do/If/Else/ElseIf/Data.
  - 4 extra bytes: Exit/ExitIf/On/const int/const float/ext.
  - 8 extra bytes: Procedure and double constants.
  - variable/label tokens: `4 + namelen`.
  - string/rem tokens: `2 + len`, rounded up to even.

**Register conventions at run time:**

| reg | meaning |
|---|---|
| a6 | program pointer (next token) |
| a5 | data zone (all interpreter state) |
| a4 | main token table base (`AdTokens`) |
| a3 | one downward-growing stack shared by loop/proc/gosub frames, expression evaluator operands, and instruction parameters (`PLoop` = current loop-stack top) |
| d7 | address after current instruction token. Used for error position, Every/menu return (re-execution), and GC restart |
| d6 | preserved by every instruction (debug code checks d6/d7/a3 are unchanged, +ILib.s:487-508) |
| d3 (d4) | value of last evaluated expression / last parameter / function result (d4 = low half of IEEE double) |
| d2 | type of d3: 0=int, 1=float, 2=string |
| sp | 68k stack, reset to `BasSp` on jumps/errors |

#### 1.3 Per-instruction checks (Break, Every, menus, screen refresh)
These are **not** done on every instruction.
- **What raises the flag.** The VBL interrupt (+W.s:10420-10424) does `T_VBLCount++`, `T_VBLTimer++`, `T_EveCpt--` (a word), and `bset #BitVBL(15),T_Actualise`. The keyboard handler sets `BitControl(8)` in `T_Actualise` on Ctrl-C (+W.s:12840). The bit numbers are at +Equ.s:826-833: `BitControl=8`, `BitMenu=9`, `BitJump=10`, `BitEvery=11`, `BitEcrans=12`, `BitBobs=13`, `BitSprites=14`, `BitVBL=15`.
- **Where it is tested.** Only these instructions test the flag: `tst.b T_Actualise(a5); bmi -> Test_Force`. Because the high byte is tested, this effectively means "has a VBL happened since the last test", so the test runs at most once per frame (50 Hz PAL):
  - Next (+ILib.s:2118), Until (2209), Wend (2252), Loop (2287), Exit (2300; Exit If only when true), Gosub (2391), Return (2416), Pop (2438), Proc call (`CallProc` 2485), End Proc (2633), Pop Proc (2698), Goto (2759), On (2791).
  - Resume, Resume Next and Resume Label call `Test_Normal` (1934/1953/1983).
  - Wait (`Wait_Normal`, +Lib.s:2057), Wait 0 (`Wait_Event` loops forever until a jump), Multi Wait, Wait Vbl and Wait Key call `Test_Normal`.
  - Input and some other blocking routines call `Test_PaSaut`, which runs the test with jumps forbidden (`Bit_PaSaut`).
  - For, While (first entry), Do, If and assignments **never** test. A program without loops/jumps/waits never sees Break or Every.
- **`Test_Force`** (+ILib.s:915-1031), with `d4 = ActuMask` (the enable mask) and `d3 = T_Actualise`, does the following in order:
  1. Dialog auto-test.
  2. Menus: if `BitMenu` is enabled and a menu is defined, it handles menu keys and the right mouse button (`MnGere`).
  3. **Ctrl-C** (`BitControl` set in `T_Actualise`):
     - If `BitControl` is set in `ActuMask` (Break On, the default), it raises **error 9 "Program interrupted"**. Error 9 is not trappable (§9).
     - Otherwise (Break Off) it calls `OnBreakGo`. If `On Break Proc` is set, that procedure is called; otherwise Ctrl-C is ignored.
  4. `BitJump`: On Menu branch (`GoMenu`, +ILib.s:1039).
  5. Screen/bob/sprite/extension refresh, at most every `VBLDelai` VBLs (Update Every).
  6. **Every**: if `BitEvery` is set in `ActuMask` and `T_EveCpt <= 0`, then `T_EveCpt = EveCharge`, `BitEvery` is cleared, and `EveJump` runs. **Every disables itself when it fires**, so the program must execute `Every On` to re-arm it.
  7. Clears `BitVBL`, but only if no jump happened.
- **Jump mechanics** (`GoMenu`, `OnBreakGo`, `EveJump`; +ILib.s:1039-1135):
  - `sp = BasSp`, push `Prg_InsRet`, and set `a6 = d7-2`, the start of the interrupted instruction.
  - A Gosub (`Gos2`) or procedure call (`InProE`) is then performed with that return address. When the Every/Break/Menu routine returns, **the interrupted instruction is re-executed from scratch**. For example, a `Wait 50` that was interrupted restarts its full 50-frame wait.
  - Jumps are suppressed when `Bit_PaSaut` is set, or in direct mode (menus).
- **Defaults** (`DefRun1`, +ILib.s:236): `ActuMask = %0111_0001_0000_0000`, which means Break enabled, screens/bobs/sprites refresh on, Every off, menus off.

---------------------------------------------------------------------------
### 2. Expression evaluator (+ILib.s:3091-3716)

#### 2.1 Operators and precedence
`Tk_Operateurs` (+ILib.s:3048-3086): each entry is `bra Op_X` followed by the operator's name string. An operator's token value is the (negative) offset of its entry from `OP_Jumps`. Values from the Devpac layout, confirmed against `_TkEg=$FFA2`, `_TkM=$FFCA` and `_TkPow=$FFF6`:

| token | op | routine |
|---|---|---|
| $FF3E | xor | Op_Xor |
| $FF4C | or | Op_Or |
| $FF58 | and | Op_And |
| $FF66 | <> | Op_Diff |
| $FF70 | >< | Op_Diff |
| $FF7A | <= | Op_InfEg |
| $FF84 | =< | Op_InfEg |
| $FF8E | >= | Op_SupEg |
| $FF98 | => | Op_SupEg |
| $FFA2 | = | Op_Egal |
| $FFAC | < | Op_Inf |
| $FFB6 | > | Op_Sup |
| $FFC0 | + | Op_Plus |
| $FFCA | - | Op_Moins |
| $FFD4 | mod | Op_Modulo |
| $FFE2 | * | Op_Mult |
| $FFEC | / | Op_Div |
| $FFF6 | ^ | Op_Puis |

**Precedence is the unsigned token value. Every token is its own level, and equal levels associate to the left.** The algorithm (`New_Evalue`, +ILib.s:3102):
```
push word $7FFF (marker); clr.w -(sp) (unary-minus counter)
OpeRe: tok=(a6)+; if tok<0 (signed): counter++ ; goto OpeRe       ; any negative token as operand = unary minus
       call FUNCTION entry of tok (-LB_Size-4(a4,idx)) -> d2/d3/d4
       if counter!=0: negate once (NOT parity: "- - 5" = -5)
OP_Ret: tok=(a6)+; if (unsigned)tok > (unsigned)top_of_stack: push d2,d3,d4 (12 bytes) + tok (2 bytes); goto operand
        else a6-=2; pop w; if w>=0 (marker) { if tok==')' consume it; return } else jmp operator(w)  ; operator pops 12 bytes, goes to OP_Ret
```
Consequences that the Rust port must reproduce:
- `a*b/c` evaluates as `a*(b/c)`, because `/` ranks above `*`. So `10*3/4 = 10*0 = 0`. By contrast, `a/b*c` is `(a/b)*c`.
- `a+b-c` evaluates as `a+(b-c)`. `a-b+c` is `(a-b)+c`. `a mod b*c` is `a mod (b*c)`.
- `2^3^2` is `(2^3)^2 = 64` (left associative).
- `a<b=c` is `(a<b)=c`. `a=b<c` is `a=(b<c)`. `<>` ranks below `><`, `<=` below `=<`, and `>=` below `=>`.
- `xor` < `or` < `and`.
- Unary minus applies to the single following operand only: `-2^2 = (-2)^2 = 4`.
- **`Not` is a function** (`FnNot`, +ILib.s:4296) that calls `New_Evalue` on the *whole rest of the expression*. So `Not a=b` means `Not(a=b)`, and `Not(a)+1` means `Not(a+1)`. It is a bitwise `not.l`; a float argument is converted to int first.
- `(` is a function (`L_New_Evalue`). The closing `)` is consumed by whichever evaluation level stops on it (Eva3/Eva4).
- Operands are function entries in the token table: variables (`FnVar`), constants (`FnCEntier` int, `FnCstFl` single, `FnCstDFl` double, `FnCstCh` string), functions, and `(`. Punctuation tokens used as operands call `FnNull`, which returns `EntNul = $80000000` and un-reads the token. **`$80000000` is the "omitted parameter" sentinel** (for example `At(,y)`), so the literal value -2147483648 cannot be told apart from an omitted parameter.

#### 2.2 Types and conversions
- Types (d2): `0` = int (signed 32), `1` = float, `2` = string (d3 points to `[len.w][bytes]`).
- **Float formats:**
  - Single precision is **Motorola FFP** (mathffp.library), not IEEE. FFP layout: mantissa in bits 31-8, sign in bit 7, exponent (excess-64) in bits 6-0.
  - `Set Double Precision` (Verif sets `MathFlags=%10000011`, +Verif.s:776) switches to **IEEE double** (mathieeedoubbas/doubtrans). The value is carried in d3:d4 (high:low) and occupies 8 bytes in variables.
  - Float constants in tokens are FFP (`_TkFl`) or IEEE double (`_TkDFl`). They are converted on the fly (`FnCstFl`/`FnCstDFl`, +ILib.s:4113-4144, via `FFP2Ieee`/`Sp2Dp`, `Dp2Sp`/`Ieee2FFP`).
  - For bit-exact single-precision results, emulate mathffp arithmetic (see AROS mathffp), not IEEE f32.
- **`Compat`** (+ILib.s:3617) makes two operands compatible. If the types differ, the int side is converted to float (`IntToFl1` for the right operand, `IntToFl2` for the stacked left operand). It returns flags: int → N, float → Z, string → otherwise. Mixed string/number is rejected earlier by Verif (Type mismatch).
- `QuEntier` (3632) forces both operands to int with `FlToInt` (used by and/or/xor/mod). `QueFloat` (3611) forces float (used by `^`).
- `IntToFl` = `SPFlt`/`IEEEDPFlt`. `FlToInt` = `SPFix`/`IEEEDPFix`, which truncate toward zero and saturate (behaviour of the ROM libraries) (+ILib.s:7277-7378).
- Parameters, assignments, If/While/Until conditions and labels all convert implicitly: float → int by truncation, int → float as needed.
  - **Conditions use `New_Expentier`** (+ILib.s:3707), which converts a float to int *before* testing for non-zero. So `If 0.5` is false.

#### 2.3 Operator semantics
**Comparisons.** Comparisons return **-1 (true) or 0 (false)** as an int (`Vrai`/`Faux`, +ILib.s:3468-3474).
- Strings are compared by `compch` (3643): unsigned byte lexicographic, case-sensitive; a shorter string that is a prefix of the other compares as smaller.
- Floats are compared with `SPCmp`/`IEEEDPCmp` (`Float_Compare`, +ILib.s:7410).

**`+`** (Op_Plus, 3161):
- int: `add.l`; signed overflow raises **error 29 Overflow**.
- float: SPAdd.
- string: concatenation. If one side is empty, the other side is returned unchanged (shared pointer, no copy). A combined length `>= String_Max ($FFC0 = 65472)` raises **error 21 String too long**.

**`-`** (Op_Moins, 3221):
- int: `sub.l` with overflow raising error 29.
- float: SPSub (left - right).
- **string**: `a$-b$` removes *all* occurrences of b$. It repeatedly searches from the start (`InstrFind` from position 1) and deletes the first match, until no match remains. This cascades: `"aabb"-"ab"` gives `"ab"` and then `""`. If b$ is empty, the result is a copy of a$.

**`*`** (Op_Mult, 3297), integer case:
- If both |a| and |b| < 65536, the result is `mulu` with the sign applied, **with no overflow check**. So `50000*50000` wraps to -1794967296.
- Otherwise a 32×32 multiply checks partial products and raises **error 29** when |a|·|b| ≥ 2^31. That includes the exact result -2^31.
- float: SPMul.

**`/`** (Op_Div, 3364):
- int: division by 0 raises **error 20 Division by zero**. Otherwise the result is truncated toward zero (unsigned divide of magnitudes, then the sign XOR is applied). `$80000000 / -1` gives `$80000000` with no error.
- float: if the divisor tests zero (SPTst), error 20; otherwise SPDiv.

**`mod`** (Op_Modulo, 3428): both operands are forced to int. The result is `(a as u32) % |b|` computed unsigned.
- The **left operand is NOT made absolute**, so `-7 mod 3 = 0xFFFFFFF9 % 3 = 0`. The result is always non-negative.
- **There is no zero check: `a mod 0 = a`** (the remainder loop simply shifts a through).

**`^`** (Op_Puis, 3417): both operands become float, then SPPow/IEEEDPPow (mathtrans), giving `left^right`. The result is always a float.

**and / or / xor** (3576-3605): 32-bit bitwise on ints.

**Unary minus** (Chs0, 3143): int uses `neg.l` (no overflow check). FFP flips bit 7. Double flips bit 31.

---------------------------------------------------------------------------
### 3. Parameter passing to instruction/function routines

`Library_GetParams` (+B.s:2694-2826) patches each `Lib_Par` routine's 4-byte `"GetP"` placeholder (macro +Equ.s:2184) into `jsr <Parameters routine>`. The routine is chosen from the parameter code in the token table:
- bits 0-5: parameter count;
- bit 6: instruction; bit 7: function;
- bits 12-14: kind (0 int, 1 float, 2 angle, 3 math, 4 reserved variable).

The parameter routines (`Parameters` table, +ILib.s:717-889):
- `Ent_N` (int/string). Evaluates each parameter with `New_Evalue` and converts float to int. Every parameter **except the last** is pushed with `move.l d3,-(a3)`. **The last parameter stays in d3, with its type in d2.** Separators (`,`, or `(` for functions; the `F*` entry points do `addq.l #2,a6` first) are skipped one token at a time. The final `)` is consumed by the evaluator.
- `Flt_N`: the same, but int → float. Note that a double value only pushes `d3`, so float parameters are single-word.
- `Par_Angle` (functions Sin/Cos/Tan): evaluate, convert to float, and if in Degree mode multiply by pi/180 (`FFAngle`, +ILib.s:7601).
- `Par_Math` (Sgn/Abs/Int/Str$): evaluate one parameter and dispatch on its type. An int jumps to the int routine past its GetP; a float jumps to the routine defined right after it (`FnSgnF`, `FnAbsF`, `FnIntF`, `FnStrF`).
- `IVar_N` (reserved-variable assignment, e.g. `X Mouse=...`): index parameters are pushed, then the value expression is evaluated into d3.

**Ordering.** On routine entry, `(a3)` holds the second-to-last parameter, `4(a3)` the one before it, and so on. Routines pop with `(a3)+` in reverse order. Example: `FnMid3` pops `d3=n`, then `(a3)+=start`, then `(a3)+=string` (+ILib.s:6537).

**Return.** A function returns its value in d3 (d3:d4 for a double) and its type in d2. The `Ret_Int`/`Ret_Float`/`Ret_String` macros (+CEqu.s:22-33) are `moveq #0/1/2,d2; rts`. Strings are returned as a pointer to newly allocated `[len][chars]` at `HiChaine`.

**Extensions.** "New" (2.0) extensions use the same patched scheme. "Old" extensions push every parameter (as ints) onto a3 and save d6/d7 in `ErrorSave` (`InExtCall`/`FnExtCall`, +ILib.s:533-620).

---------------------------------------------------------------------------
### 4. Variable storage

#### 4.1 The variable buffer (`VarBuf`, size `VarBufL`)
`ResVarBuf` is at +Verif.s:4045 and is run by PTest (+Verif.s:73-200).

The buffer, from low to high addresses:
- **String heap.** It starts at `LoChaine`. The first word is a `0` length, which is the shared empty string `ChVide` (also the initial `ParamC`). The heap grows upward to `HiChaine`.
- free space;
- **arrays and procedure local frames.** These grow *downward*; `TabBas` is the lowest used address.
- **global variables.** They start at `VarGlo`, which is also the initial `VarLoc`/`TabBas`. They are terminated by `$FFFF` + `0.l` at `LabBas`.
- **label table**, from `LabBas` to `LabHaut`.

**Size.**
- The default is **8 KB** (PTest `move.l #8*1024,d1`, +Verif.s:89).
- `Set Buffer n` sets `n*1024` and restarts verification (+Verif.s:806-820). It must precede any variable use.
- Direct mode uses a 4 KB buffer plus a `PI_TVDirect=42*6` bytes direct-variable chunk (`ResDir`).

#### 4.2 Variable slot layout
Each scalar slot is **6 bytes**: `[len.b][flag.b][value.l]`. A double-precision float scalar is **10 bytes** (Verif reserves `+4`, +Verif.s:3586).
- `len` is 4 or 8.
- `flag` low nibble is the type: 0 int, 1 float, 2 string. Bit 6 means array; bit 3 marks a Def Fn variable.
- The value is: an int; a float (FFP 4 bytes, or IEEE 8 bytes); a string pointer (`0` = never assigned, read as `""`; `FnVar` then initialises it to `ChVide` with flag `$0402`); or an array header pointer.

**Addressing in tokens.** `_TkVar` is followed by `w offset`, `b namelen`, `b flags`, then the name.
- offset ≥ 0 is local: `VarLoc + offset + 2` is the value address.
- offset < 0 is global: `VarGlo - offset + 1` (+ILib.s:3734-3770, `FindVar` 3854).

Variables are found by name **and type** (`A`, `A#` and `A$` are distinct, `V1_StoVar`, +Verif.s:3521). The buffer is zeroed when allocated, and global variables are zeroed again at the start of each run.

#### 4.3 Local, Shared and Global
These are resolved at verify time (+Verif.s:3499-3520 `Locale`/`Globale`, `VerSha` 3826). Runtime `InShared` (+ILib.s:4177) only skips tokens.
- Variables used inside a procedure are local to it (each call gets a fresh, zeroed frame).
- `Global a,b()` in the main program marks the names as global level 2, visible in **all** procedures.
- `Shared a,b()` inside a procedure makes the named main-program variables visible in **that** procedure only. The variable must exist in the main program.
- Procedure parameters are local variables, passed by value.

#### 4.4 Arrays (`InDim`, +ILib.s:3897; `GetTablo`, 4010)
**Header**, allocated by lowering `TabBas`:
```
+0 b  ndims
+1 b  shift (2 => 4-byte elements; 3 => 8-byte doubles)
+2    ndims x { w max_index_i ; w mult_i }    (dim 1 first; mult_n = 1, mult_i = prod_{j>i}(max_j+1))
+2+4*ndims  elements (row-major, last index fastest), size 1<<shift
```
**Dim rules.**
- Each dimension value `n` must satisfy unsigned `n < $FFFF`, otherwise **error 23** (so negative values fail).
- The running element count (not counting the last dimension) must stay `< $10000`, otherwise error 23.
- Dim of an already-dimensioned array raises **error 28**.
- If there is not enough space, the GC runs and then **error 11** (Out of variable space) is raised if space is still short.

Elements are initialised to 0, or to `ChVide` for strings. The variable slot gets flag word `$04<<8 | type | $40` and the header pointer.

**Access.** Each index is converted to int and checked `unsigned idx <= max`, otherwise **error 23** (negatives fail). Using an array before Dim raises **error 27**.

Local arrays live below the procedure's locals and are freed when `TabBas` is restored at End Proc. `=Array(a(0))` returns the header address; `VarPtr(v)` returns the value address (string: chars address = ptr+2).

---------------------------------------------------------------------------
### 5. String memory and garbage collection

**String format.** A string is `[len.w][bytes]`, padded to an even address. The maximum length is `$FFC0` (65472).

**Allocation** (`Demande`, +ILib.s:6031). It is a bump allocator. To allocate `d3` bytes it checks `HiChaine + d3 + 4 < TabBas`. On success it returns `a0 = a1 = HiChaine`; the caller writes the string and then sets `HiChaine` to the even end. Strings are immutable, so assignment copies only the pointer. Constant strings point into program memory, except in direct mode where they are copied (`FnCstCh`, 4146).

**When space runs out:**
1. `Menage` (GC) runs.
2. If space is still insufficient, **error 11 "Out of variable space"** is raised (`OOfBuf`, 1166). Error 11 is trappable even though it is below 19 (§9).
3. If the GC succeeded and `d7 > 0`, **the current instruction is restarted from its beginning** (`a6 = d7`, re-dispatch) with `d7 = 0` as a marker. A GC is needed again during that same restarted instruction raises error 11 (`FinMenE`).

   This restart is needed because the GC invalidates every temporary string pointer held in registers or on the a3 stack. Side effects already performed by the instruction (for example partial Print output) are repeated.
4. `DDemande` (`d7 = -1`) retries in place with no restart, for internal callers.

**GC `Menage`** (+ILib.s:6110-6437). The roots are only the variables. The GC walks variable chunks starting at `VarLoc`: a flag word of 0 is skipped; a positive word is a slot whose length is in the high byte; a negative `$FFFF` word is followed by a `.l` link to the next chunk, with 0 meaning end. Each procedure frame starts with `$FFFF` + the saved caller `VarLoc`, so all active frames and globals are visited. String arrays are scanned element by element.
- **FAST_MENAGE** (when a temporary buffer the size of the heap can be allocated):
  1. Each live string inside `[LoChaine, HiChaine)` is copied in variable-walk order into the temporary buffer.
  2. The original's length word is overwritten with a `$FFC0xxxx` forwarding marker, so shared strings are copied once.
  3. Empty strings are redirected to `ChVide`.
  4. The buffer is copied back.
- **SLOW_MENAGE** sorts live pointers with a small table (`TMenage=1536` bytes) in several passes and compacts in address order.

Either way the result is a compacted heap: `HiChaine = end of live data`. Only live strings, meaning those reachable from variables, survive.

**Rust port.** With reference-counted strings no GC is needed. To reproduce "Out of variable space" exactly, track a byte budget:
`VarBufL >= labels + globals + Σ frames(6 + varsize) + Σ arrays(2 + 4*ndims + n*elem) + live string bytes (2+len, even) + 4`.
Raise error 11 when an allocation fails after a virtual GC.

**`Set Buffer`.** At run time `InSetBuffer` (+ILib.s:1799) is a no-op `rts`; the size is applied during verification.

---------------------------------------------------------------------------
### 6. The a3 control stack and its frames

`Set Stack n` (+Verif.s:789-803) stores `max(n,10)+1` into `Stack_Size`; the default is 51. The stack size is `(Stack_Size+1)*42` bytes (`Stack_ProcSize=42`, +Equ.s:1333; `Stack_Reserve` +Verif.s:3983).

Frames grow downward. A push that leaves `a3 < MinLoop (= BaLoop+64)` raises **error 13 "Out of stack space"**. Only loop/gosub/proc pushes check this; evaluator and parameter pushes do not.

| frame | size | layout from a3 upward |
|---|---|---|
| For | 24 (`TForNxt`) | `w 24`, `l NextAddr`, `l bodyStart`, `w type(0)`, `l step`, `l limit`, `l &var` |
| Repeat / Do | 10 | `w 10`, `l Until/LoopAddr`, `l bodyStart` |
| While | 10 | `w 10`, `l afterWend`, `l condStart` |
| Gosub | 12 | `l "Gosb"`, `l returnA6`, `l oldBasA3` |
| Proc | 42 | `l "Proc"`, `l returnA6`, `l DProc`, `l AData`, `l PData`, `w ErrorOn`, `l ErrorChr`, `l OnErrLine`, `l TabBas`, `l VarLoc`, `l BasA3` |

`BasA3` points at the innermost Gosub or Proc frame. With the default 51, about 50 nested procedure calls fit; with Set Stack n, about n.

**`LGoto`** (+ILib.s:2766) runs after *every* jump: Goto, If/Else, Resume, Pop, On Goto. Starting from a3 and staying below `BasA3`, it pops each loop frame unless `bodyStart(6(a3)) <= a6 <= end(2(a3))`. So jumping out of loops discards their frames, and jumping into a loop body keeps them.

---------------------------------------------------------------------------
### 7. Control flow instructions (+ILib.s)

**Loop body start** (`Rpt0`, 2189). Used by For, Repeat and Do. After the token's operand words, if the next word is `:` it is skipped; if it is the end-of-line `0`, the next line's header is also skipped. The saved body start therefore always points at an instruction token.

**For** (`InFor`, 2074):
- Inline: `w offset to Next`.
- Steps:
  1. Find the variable.
  2. Evaluate the start expression and **store it into the variable first**.
  3. Evaluate the limit. This happens after the store, so the limit sees the new value.
  4. Evaluate the optional `Step`; the default is 1.
  5. Push the frame.
- **There is no initial test: the body always runs at least once**, so `For I=10 To 1` runs once.
- The loop variable must be an integer, and the start/limit/step must be integers (Verif raises Type mismatch, +Verif.s:1700-1730). The float path `NextF` (2153) is dead code in the interpreter.

**Next** (2116):
1. Test point (§1.3).
2. `var += step` (`add.l`, **no overflow check**, wraps silently).
3. If `step >= 0`: exit when `var > limit`. If `step < 0`: exit when `var < limit`.
4. Loop: `a6 = bodyStart`. Exit: pop the frame and skip the optional variable after Next.

A step of 0 loops forever while `var <= limit`.

**Repeat/Until** (2184/2207). Until: test point, then evaluate `New_Expentier` (float truncated). If 0, jump to the body; otherwise pop the frame.

**While/Wend** (2231/2250):
- While: inline `w offset` pointing past Wend. It pushes the frame, then evaluates the condition.
- Wend: test point, then jump to `condStart` and re-evaluate the condition. If true, continue into the body; if false, continue after Wend and pop the frame.

**Do/Loop.** Do is Repeat. Loop: test point, then `a6 = bodyStart`.

**Exit n** (2298): inline `w distance`, `w a3 adjustment` (bytes of loop frames to drop, computed by Verif). **Exit If cond[,n]** (2317): evaluate the condition (truncated to int). If true, behave as Exit. If false, skip the optional `,n` (8 bytes).

**If (multi-line or single-line)** (2336):
- Inline: `w offset`. The condition is evaluated with `New_Expentier`. If true, skip the optional `Then` token.
- If false: jump `a6 += offset & ~1`.
  - If bit 0 of the offset is 0, the target is an Else body or the point after End If / end of line, and `LGoto` runs.
  - If bit 0 is 1, the target is an `Else If`, so its condition is evaluated in the same loop.
- **Else** and **Else If** reached sequentially (both use `InElse`, 2370; the token table maps "else if" to L_InElse at +Lib.s:1451):
  - Read their own offset and jump.
  - If the target is another Else If (bit 0 set), or lands just after an `Else` token, keep chaining.
  - The chain therefore always ends at End If, or at end of line for single-line If. End If is `InNull`.

**Goto** (2757): test point, `GetLabel`, `LGoto`.

**Gosub** (2389): test point, `GetLabel`, then push a Gosub frame `{Gosb, a6, BasA3}` and set `BasA3 = a3`.
- **Return** (2414): `a3 = BasA3` (this **discards any loops opened inside the gosub**), check `"Gosb"` (error **1** if absent), and restore a6 and BasA3.
- **Pop** (2436): drop the Gosub frame (error **2** if absent), then `LGoto`.

**Label resolution** (`GetLabel`, 2859):
- `_TkPro`/`_TkLGo` tokens map directly through the `LabHaut` table.
- Any other operand is evaluated as an expression:
  - int (float is truncated) → decimal text with no sign space;
  - string → lowercased A-Z, length must be under 32.
  - The name is then searched in the `LabBas` list.
- Not found raises **error 40 "Label not defined"**.

**On e Goto/Gosub/Proc l1,l2,...** (2789):
- Inline: `w offset to end`, `w label count`.
- Let `n` be the expression value.
- `n <= 0` or `n > count`: continue after the statement. No error.
- Otherwise it skips `n-1` labels; skipping evaluates expression labels.
- Then it does a Goto, a Gosub (returning after the On statement), or a Proc call (no parameters; returning after the statement).

**Def Fn / Fn** (4194/4206):
- Def Fn stores the address of its parameter list in the Fn variable and skips the rest of the line, so it must be alone on the line.
- `Fn f(args)`:
  - raises error **15** if f is undefined;
  - raises error **16** on a parameter count or shape mismatch;
  - assigns each argument to the *ordinary variable* named in the Def Fn (in the current scope, so this is a side effect), with int/float conversion; a string vs number mismatch raises error 34;
  - then evaluates the body expression.

---------------------------------------------------------------------------
### 8. Procedures

**Procedure header.** The procedure "label" is the address of its `_TkProc` token: `+0 w token`, `+2 l distance to End Proc`, `+6 w local var bytes` (patched by Verif), `+8 b,b flags`, `+10` name token (`w,w,b len,b flag,name`), then the optional `[ param vars ]`. Executing a `Procedure` line in normal flow skips the whole body (`InProcedure`, 2465).

**Call** (`CallProc`, 2483; `InProc` = `Proc name`, 2475):
1. Test point.
2. Allocate the local frame below `TabBas`: store old `VarLoc` (`.l`) and `$FFFF` (`.w`), then `varsize` bytes, zeroed. If there is no room, run the GC; if it still fails, take the Demande/restart path, which leads to error 11.
3. **Arguments**: for each `[e1,e2,...]`, evaluate the argument *in the caller's scope* (VarLoc not yet switched). Convert int↔float to the declared type and store into the callee's slot, setting its len/flag. A parameter variable may be global (negative offset → `InPaGlo`).
4. Push the 42-byte Proc frame. Then:
   - `BasA3 = a3`; `VarLoc = TabBas = newFrame`;
   - **`OnErrLine = 0`**, so On Error handlers are per procedure and are not inherited from the caller;
   - `DProc = PData = body`, `AData = 0`, so Data/Read is scoped to the procedure;
   - `ErrorOn = 0`, or the error code if entered via On Error Proc;
   - `a6 = body`.
5. Recursion is unlimited apart from VarBuf space (locals) and the a3 stack (42 bytes per call).

**End Proc [expr]** (2631):
1. Test point.
2. With `[expr]`, `FnEProc` (2675) sets **one** of `ParamE` (int), `ParamF`/`ParamF2` (float) or `ParamC` (string), according to the expression type. The other two keep their old values.
3. If `ErrorOn` is set, raise **error 8**.
4. Scan a3 upward in 2-byte steps until the `"Proc"` magic is found, then restore everything.
5. If the return address is 0, return to the menu handler.

**Reading return values.** `Param` returns `ParamE` (int), `Param#` returns `ParamF` as float, `Param$` returns `ParamC` (+Lib.s:2149-2160; `FnParamF` at +ILib.s:7642). They are reset only at run start (`ParamC = ChVide`).

**Pop Proc [expr]** (2696): same as End Proc via `PopP` (2714). A zero return address raises error 23.

Local strings simply become garbage. Local arrays are freed when `TabBas` is restored.

---------------------------------------------------------------------------
### 9. Errors, On Error, Resume, Trap

**Raising an error.** `RunErr` (+ILib.s:1267): `d0` = error number, threshold `d1 = 19`, `d2 = -1` (`ErrorExt` passes an extension number in d2). The message list is at +Editor_Config.s:820-900. For example: 1 Return without Gosub, 2 Pop without Gosub, 3 Error not resumed, 4 Can't resume to a label, 5 No On Error Proc, 6 Resume label not defined, 7 Resume without error, 8 Error proc must Resume, 9 Program interrupted, 10 End of program, 11 Out of variable space, 13 Out of stack space, 15/16 user fn, 20 Div by zero, 21 String too long, 22 Syntax, 23 Illegal function call, 24 Out of memory, 27 Non dimensioned array, 28 Array already dimensioned, 29 Overflow, 33 Out of data, 34 Type mismatch, 40 Label not defined, 41 No data after this label.

**Trappability** (1293-1301). An error is *not* trappable and goes straight to the editor if any of these hold:
- error number `< 19`, except 11;
- error number `>= 1000` (1000 Edit, 1001 Direct, 1002 System);
- in direct mode;
- `ErrorOn != 0` (an error inside the handler);
- no `On Error` is set and no Trap is active.

So **End (10), Stop (9 "Program interrupted"), Break (9), and Return/Pop/Resume errors cannot be caught.**

**`Trap instr`** (2011): `TrapAdr = address of the trapped instruction's token + 2`. If an error occurs with `d7 == TrapAdr`:
- `TrapErr = err | (ext+1)<<8`, with no +1 on the error number.
- Execution skips to the next instruction, using the same scan as Resume Next.
- `=Errtrap` returns `TrapErr`; it is cleared by each `Trap`.

**On Error Goto L** (1876):
- If `ErrorOn != 0` at that point, raise error 3.
- `On Error` (bare) or `On Error Goto 0` disables the handler.
- On an error:
  - `ErrorOn = (err+1) | (ext+1)<<8`;
  - `ErrorChr = start of the failing instruction`;
  - a3 keeps the current loop stack (`PLoop`); sp is reset; jump to L;
  - Trap state is cleared.
- `=Errn` = `ErrorOn - 1` (0 if none).

**On Error Proc P**: `ErrorChr` bit 31 is set. On an error, P is called via `InProE`:
- the frame's return address is the failing instruction;
- `ErrorOn` is pushed and becomes the callee's `ErrorOn`.

**Resume** (1951) — test point, then error **7** if `ErrorOn == 0`:
- **Goto mode:**
  - `Resume` clears ErrorOn and **re-executes the failing instruction**.
  - `Resume L` clears ErrorOn and jumps to L.
- **Proc mode:**
  - `Resume` performs Pop Proc and clears ErrorOn, then re-executes the failing instruction.
  - `Resume L` raises error 4.

**Resume Next** (1981): continues after the failing instruction. It scans tokens from `ErrorChr` using `TInst` sizes until it consumes one of `:`, end of line (the next line header is also skipped), **`Then` or `Else`**. Resuming after an error in an `If` condition therefore runs the Then part. In proc mode it first performs Pop Proc.

**Resume Label** (1917):
- *With* a label: allowed only in proc mode (otherwise error 5). It records the label: `ErrorChr = label | $80000000`.
- *Without* a label, inside the error procedure: Pop Proc, clear ErrorOn, Goto the recorded label. Error 5 if proc mode is not set; error 6 if no label was recorded.

**`Error n`** (+Lib.s:11396): values ≥ 256 are clamped to 255, then `RunErr` is called. `Error 0..18` (except 11) are therefore fatal.

**End / Stop / Edit / Direct / System.** End raises error 10 (also raised by running off the end of the program, `InEnd` +ILib.s:519). Stop raises 9 (+Lib.s:13013). Edit raises 1000, Direct 1001, System 1002 (+ILib.s:1820-1843).

**Break On / Off** (1846/1853):
- Break On sets `BitControl` in `ActuMask` and clears `OnBreak`.
- Break Off clears the bit.
- `On Break Proc P` (1862) stores P and also clears the bit, which implies Break Off.
- Ctrl-C handling is described in §1.3.

**Every n Gosub|Proc L** (`InEvery`, 2034):
- `n` must satisfy `1 <= n < 32767` (unsigned check), otherwise error 23. The unit is VBLs (50 Hz PAL).
- It sets `EveCharge = T_EveCpt = n`, the type and the label, and sets `BitEvery`.
- **Every Off** clears the bit; **Every On** sets it. Every On does **not** reload the counter, which keeps decrementing (as a word) even while Every is off.
- The handler fires at the next test point once the counter is ≤ 0 (§1.3), then disables itself.

**On Menu Goto/Gosub/Proc** uses the same jump path (`GoMenu`). It is enabled by `On Menu On` (BitMenu/BitJump).

---------------------------------------------------------------------------
### 10. Data / Read / Restore (+ILib.s:4589-4735)

**Data.** The token carries the distance back to its line start. At run time Data skips to the end of its line.

**Data search order** (`NxD1`). Lines are scanned from `PData`:
- A line counts only when its **first** instruction is `Data`, or when it starts with a label immediately followed by `Data`.
- `Procedure` blocks are skipped entirely.
- Reaching `End Proc` or the end of the program raises **error 33 Out of data** (and `PData = 0`).
- `PData` then advances to the line after the found Data line. `AData` points at the next item within the current line.

**Reading items.** Each item is a **full expression evaluated at Read time** in the current scope, so variables in Data are allowed. An empty item (`,,` or end of line) yields 0, or `""` for a string target.
- A number read into a string variable, or the reverse, raises **error 34**.
- int↔float values are converted.

**Restore.**
- With no label, Restore resets to `DProc`, the start of the current procedure or the main program.
- `Restore L` requires L's line to start with `Data`; otherwise **error 41**.

Each procedure therefore has its own Data scope.

---------------------------------------------------------------------------
### 11. Built-in functions: exact behaviour

#### Rnd / Randomize (+Lib.s:1947-2015)
`Seed` (.l) lives *outside* the per-run cleared area, so it persists across runs. It starts at 0 when the data zone is allocated. `OldRnd` holds the last result.

`Rnd(n)` (n converted to int):
- `n == 0` returns `OldRnd`.
- `n < 0` uses `m = -n` with no beam mixing. `n > 0` uses `m = n` and mixes in the video beam position `$dff006` (nondeterministic; a port cannot reproduce it).
- Mask: the smallest `2^k - 1 >= m`, capped at `$FFFFFF` (24 bits). For `m > $FFFFFF` the result is limited to 0..$FFFFFF.
- Loop:
  ```
  r = RRnd()
  if (n>0) r.low16 = (r.low16 + VHPOSR) & 0xFFFF   ; add.w
  r &= mask
  until r <= m
  ```
  The result is `0..m` **inclusive**, and it is stored in `OldRnd`.
- `RRnd`: `Seed = Mulu32(Seed, $BB40E62D) + 1`, and the returned value is `Seed >> 8`.
- **`Mulu32` is not a true 32-bit multiply** (+Lib.s:1990). With `lo`/`hi` meaning 16-bit halves and `rot16(x) = (x<<16)|(x>>16)`:
  `r = lo(a)*lo(b) + rot16(hi(a)*lo(b)) + rot16(lo(a)*hi(b))  (mod 2^32)`.
  The hi(a)*hi(b) term is computed and then discarded.

`Randomize n` sets `Seed = n`.

#### Integer division and Mod
See §2.3: division truncates toward zero; Mod uses unsigned left, |right|, and `x mod 0 = x`.

#### Str$ / Bin$ / Hex$ (+Lib.s:1754, 14239-14280, 25671-25790)
**`Str$(int)`** (`LongToAsc`, proportional mode, sign space): `"-123"` or `" 123"`; `Str$(0) = " 0"`; `-2147483648` prints correctly. Float Str$ is covered by the float-formatting agent.

**`Hex$(n[,d])`** returns `"$"` followed by uppercase hex digits:
- Without d: leading zeros suppressed, at least one digit.
- With `0 <= d <= 8`: exactly the **last d** nibbles (`Hex$(255,4) = "$00FF"`, `Hex$(x,0) = "$"`).
- With `d > 8` or `d < 0`: proportional, as without d.

**`Bin$(n[,d])`** works the same way: `"%"` and 32 bits. `Bin$` without d suppresses leading zeros (`Bin$(0) = "%0"`).

#### Left$ / Mid$ / Right$ (functions: +ILib.s:6453-6585 `RFnMid`)
Positions are 1-based; `p = 0` is treated as 1.
- `Left$(a$,n)`:
  - empty a$ → `""`;
  - `n < 0` → error 23;
  - `n = 0` → `""`;
  - `n > len` → whole string.
- `Right$(a$,n)`:
  - `n < 0` → error 23;
  - `n >= len` → whole string;
  - otherwise the last n characters.
- `Mid$(a$,p)`:
  - `p < 0` → error 23;
  - `p > len` → `""`.
- `Mid$(a$,p,n)` checks in this order:
  1. `p < 0` → error 23;
  2. start offset `>= len` → `""` (**even if n < 0**);
  3. `n = 0` → `""`;
  4. `n < 0` → error 23;
  5. the result is clipped to the end of the string.

**Instruction forms** `Left$(v$,n)=e$`, `Right$(v$,n)=e$` and `Mid$(v$,p[,n])=e$` (`RInMid`/`RInMid2`, 6589-6650):
1. Copy v$'s current string to a new allocation (copy-on-write).
2. Overwrite characters in place. The length **never changes**: at most `min(n, len - start, len(e$))` characters are written.
3. If start is beyond the length, nothing happens.
4. `n < 0` or `p < 0` raises error 23. Right$= with `n >= len` starts at position 1.
Evaluation order: the variable, then the numeric parameters, then e$.

#### Instr (+Lib.s:13798-13862)
`Instr(h$,n$[,start])`:
- `start < 0` → error 23; `start = 0` is treated as 1.
- Returns the 1-based position of the first match at or after `start`, or 0.
- **An empty needle returns 0** (not `start`), and an empty haystack returns 0.
- The match is case-sensitive.

#### Upper$ / Lower$ (14181-14225)
Only ASCII A-Z / a-z are changed; accented ISO-8859-1 letters are not. An empty argument returns `""`.

#### String$ / Space$ (13893-13930)
- `Space$(n)`: n spaces; `n < 0` → error 23.
- `String$(a$,n)`: the **first character** of a$ repeated n times; `n < 0` → error 23; **if a$ is empty the result is `""` regardless of n**.
- Neither checks against String_Max; the length word is truncated to 16 bits.

#### Chr$ / Asc (13930, 14227)
- `Chr$(n)` requires `0 <= n <= 255` (unsigned check), otherwise error 23.
- `Asc(a$)` returns the first byte (0..255), or **0 for `""`**.

#### Len, Flip$
- `Len` returns the 16-bit length.
- `Flip$` reverses the string.

#### Fix / Int / Abs / Sgn (+Lib.s:1738-1945)
These dispatch on the argument type (`Par_Math`).
- **Int**: an int is returned unchanged. A float returns **floor(x) as a float** (SPFloor/IEEEDPFloor), so the result type is float.
- **Abs**: an int uses `neg` (`Abs(-2147483648)` stays negative); a float uses SPAbs and returns a float.
- **Sgn**: returns -1/0/1 as an int, for both int and float arguments.
- **Fix n** sets the float print format: `n < 0` → exponent format with |n| digits, `n >= 16` → -1 (free format); stored in `FixFlg`/`ExpFlg`. Formatting itself is the other agent's area.
- Sqr/Log/Ln raise error 23 on a negative argument; zero is not checked.
- Sin/Cos/Tan take degrees when `Degree` is set. ASin/ACos/ATan results are converted to degrees.

#### Inc / Dec / Add (+ILib.s:4353-4396)
These work on integer variables only (Verif `VerVEnt`). There are **no overflow checks** (wrap mod 2^32).
- `Add v,n` is `v += n`.
- `Add v,n,a To b`: `t = v + n` (wrapping), then
  - if `t < a`: `v = b`;
  - else if `t > b`: `v = a`;
  - else `v = t`.

  This is a wrap-around between the two bounds (signed comparisons).

#### Swap (4271)
Exchanges the 4-byte values, or 8 bytes for doubles. Strings swap pointers. Both variables must have the same type (Verif).

#### Min / Max (4308-4351)
Both operands are made compatible first (int → float).
- `Max(a,b)` returns a if `a > b`, else b.
- `Min(a,b)` returns a if `a <= b`, else b.
- **Bug for strings:** `MinMax` (`MMx2`) computes the string comparison and then restores d3/d4 before `cmp.l d3,d4`. The decision is therefore made on the string pointer versus a garbage d4, so it is effectively unpredictable. A port should implement the intended lexicographic comparison and note the deviation.

#### Sort a(0) (4399)
- Sorts the **whole** array; the index parameters are evaluated and ignored.
- The order is ascending. Strings use the case-sensitive byte comparison; floats use SPCmp.
- The algorithm is Shell sort (gap halving). The result equals any correct ascending sort, since equal keys cannot be told apart.

#### Match(a(0),v) (4449)
Requires a sorted array. Indices run from 0 to N-1 over the whole flattened array. v is converted to the array's type; a string vs number mismatch raises error 34.

The search runs as follows:
1. Start with `lo = 0` and `step = N>>1`.
2. Binary phase. Repeat:
   - probe index `lo + step`;
   - if it is equal, return that index;
   - if the element is less than v, set `lo += step`;
   - if `step == 0`, leave the loop; otherwise `step >>= 1`.

   Note that a probe with `step = 0` happens once.
3. Linear phase. While `lo < N`:
   - if `elem[lo] == v`, return lo;
   - if `elem[lo] > v`, stop;
   - otherwise `lo++`.
4. Not found: return `-(lo+1)`, where lo is the insertion point.

Examples with `[10,20,30,40,50]`:

| v | result |
|---|---|
| 40 | 3 |
| 35 | -4 |
| 5 | -1 |
| 60 | -6 |

#### Errn / Err$ / Errtrap
- `Errn` = `ErrorOn - 1`.
- `Err$(n)` returns message n+1 from the run-time message list.
- `Errtrap` = `TrapErr`.

#### Timer
`=Timer` and `Timer=` read and write `T_VBLTimer` (incremented every VBL).

---

# Part D: Errors

## 4. Error messages, numbering and run-time error dispatch

### 4.1 Where the texts live
The interpreter itself carries **no** error strings. They are in the editor configuration file `AMOSPro_Editor_Config` (source `+Editor_Config.s`), loaded by the editor into pointers `Ed_Systeme, EdM_Messages, Ed_Messages, Ed_TstMessages, Ed_RunMessages` (+Equ.s:1672–1677). Format of each list = `dc.l length` then entries `0, len, text…` (`EdT n,<text>` macro), terminated by `0,$FF`; lookup by `GetMessage` with a 1-based index (+B.s:551). (`+Header.s:283/667` — compiled programs copy the run-time list in as well.)

### 4.2 Test-time (verifier) errors — `Ed_TstMessages` (+Editor_Config.s:757–815)
Raised by the verifier as **negative** codes (`d0 = -n`) to `Prg_JError`.
```
 1 Bad structure                          28 LOOP without DO
 2 User function not defined              29 WHILE without matching WEND
 3 Variable buffer can't be changed in the middle of a program!
 4 This instruction must be alone on a line
 5 Extension not loaded                   30 WEND without WHILE
 6 Too many direct mode variables         31 REPEAT without matching UNTIL
 7 Illegal direct mode                    32 UNTIL without REPEAT
 8 Variable buffer too small              33 FOR without matching NEXT
 9 No jumps allowed into the middle of a loop!
10 Structure too long                     34 NEXT without FOR
11 This instruction must be used within a procedure
12 This variable is already defined as SHARED
13 This array is not defined in the main program
14 Use empty brakets when defining a shared array
15 Shared must be alone on a line         35 Syntax error
16 Procedure's limits must be alone on a line
17 Procedure not closed                   36 Out of memory
18 Procedure not opened                   37 Variable name's buffer too small
19 Illegal number of parameters           38 Array not dimensioned
20 Undefined procedure                    39 Array already dimensioned
21 ELSE without IF                        40 Type mismatch error
22 IF without ENDIF                       41 Undefined label
23 ENDIF without IF                       42 Label defined twice
24 ELSE without ENDIF                     43 Trap must be immediately followed by an instruction
25 No THEN in a structured test           44 No ELSE IF after an ELSE
26 Not enough loops to exit               45 Cannot load included file
27 DO without LOOP                        46 Included file is not an AMOS program
47 Instruction not compatible with AMOS 1.3
48 This program holds too many banks for AMOS 1.3
49 This program is compatible with AMOS 1.3
50 This command must begin your program (but AFTER 'Set Buffer')
51 Equate not defined    52 Cannot load equate file
53 Bad format in equate file    54 Equate not of the right type
```

### 4.3 Run-time errors — `Ed_RunMessages` (+Editor_Config.s:820–1049)
Error number `n` → message index `n+1` (entry for 0 is empty). Empty strings are unused numbers. **0–18 are "fatal"** (cannot be caught by On Error / Trap — see 4.4), 20+ are normal.
```
 1 RETURN without GOSUB          2 POP without GOSUB          3 Error not resumed
 4 Can't resume to a label      5 No ON ERROR PROC before this instruction
 6 Resume label not defined     7 Resume without error       8 Error procedure must RESUME to end
 9 Program interrupted         10 End of program            11 Out of variable space
12 Cannot open math libraries  13 Out of stack space        15 User function not defined
16 Illegal user function call  17 Illegal direct mode
20 Division by zero  21 String too long  22 Syntax error  23 Illegal function call
24 Out of memory  25 Address error  27 Non dimensioned array  28 Array already dimensioned
29 Overflow  30 Bad IFF format  31 IFF compression not recognised  32 Can't fit picture in current screen
33 Out of data  34 Type mismatch  35 Bank already reserved  36 Bank not reserved
37 Fonts not examined  38 Menu not opened  39 Menu item not defined  40 Label not defined
41 No data after this label  44 Font not available
46 Block not defined  47 Screen not opened  48 Illegal screen parameter  49 Illegal number of colours
50 Valid screen numbers range 0 to 7  51 Too many colours in flash  52 Flash declaration error
53 Shift declaration error  54 Text window not opened  55 Text window already opened
56 Text window too small  57 Text window too large  59 Bordered text windows not on edge of screen
60 Illegal text window parameter  62 Text window 0 can't be closed  63 This text window has no border
65 Block not found  66 Illegal block parameters  67 Screens can't be animated  68 Bob not defined
69 Screen already in double buffering  70 Can't set dual playfield  71 Screen not in dual playfield mode
72 Scrolling zone not defined  73 No zones defined  74 Icon not defined  75 Rainbow not defined
76 Copper not disabled  77 Copper list too long  78 Illegal copper parameter
79 File already exists  80 Directory not found  81 File not found  82 Illegal file name
83 Disc is not validated  84 Disc is write protected  85 Directory not empty  86 Device not available
88 Disc full  89 File is protected against deletion  90 File is write protected
91 File is protected against reading  92 Not an AmigaDOS disc  93 No disc in drive  94 I/O error
95 File format not recognised  96 File already opened  97 File not opened  98 File type mismatch
99 Input too long  100 End of file  101 Disc error  102 Instruction not allowed here
105 Sprite error  107 Syntax error in animation string  108 Next without For in animation string
109 Label not defined in animation string  110 Jump To/Within autotest in animation string
111 Autotest already opened  112 Instruction only valid in autotest  113 Animation string too long
114 Label already defined in animation string  115 Illegal instruction during autotest
116 Amal bank not reserved
120 Interface error: bad syntax  121 …out of memory  122 …label defined twice  123 …label not defined
124 …channel already defined  125 …channel not defined  126 …screen modified  127 …variable not defined
128 …illegal function call  129 …type mismatch  130 …buffer to small  131 …illegal number of parameters
   (Interface errors = 119 + EDia_* code, +Equ.s:1123–1134, IDia_Errors equ 120-1)
140 Device already opened  141 Device not opened  142 Device cannot be opened
143 Command not supported by device  144 Device error  145 Serial device already in use
147 Invalid baud rate  148 Out of memory (serial device)  149 Bad parameter  150 Hardware data overrun
155 Timeout error  156 Buffer overflow  157 No data set ready  159 Break detected
160 Selected unit already in use  161 User canceled request  162 Printer cannot output graphics
164 Illegal print dimensions  166 Out of memory (printer device)  167 Out of internal memory (printer device)
168 Library already opened  169 Library not opened  170 Cannot open library
171 Parallel device already used  172 Out of memory (parallel device)  173 Invalid parallel parameter
174 Parallel line error  176 Parallel port reset  177 Parallel initialisation error
178 Wave not defined  179 Sample not defined  180 Sample bank not found  181 256 characters for a wave
182 Wave 0 and 1 are reserved  183 Music bank not found  184 Music not defined  185 Can't open narrator
186 Not a tracker module  187 Cannot load med.library  188 Cannot start med.library  189 Not a med module
193 Arexx port already opened  194 Arexx library not found  195 Cannot open Arexx port
196 Arexx port not opened  197 No Arexx message waiting  198 Arexx message not answered to
199 Arexx Device not interactive  200 Cannot open powerpacker.library (v35)
```
Editor message 221 "Warning: precision mismatch. Please read help file." is shown when a program's double-precision flag differs from the loaded state (Ed_Messages, +Editor_Config.s:750).

Error-raising stubs in AMOSPro.Lib (+ILib.s:1170–1250, +Lib.s:12958+): `RetGsb`=1, `PopGsb`=2, `NoResume`=3, `ResPLab`=4, `NoOnErr`=5, `ResLNo`=6, `NoErr`=7, (8 error proc must resume), `OofStack`=13, `DByZero`=20, `FonCall`=23, `AdrErr`=25, `NonDim`=27, `AlrDim`=28, `OverFlow`=29, `CantFit`=32, `TypeMis`=34, etc. All go to `RunErr` (`Lib_Def Error`).

### 4.4 Run-time error dispatch (`RunErr` / `RunErrExt`, +ILib.s:1263–1400)
Inputs: `d0` = error number; `d1` = fatal threshold (19 for core errors); `d2` = −1 for core errors or extension number (then `a0` = extension message list, NUL-separated).
1. Restore d6/d7 if `ErrorRegs`; call `Sys_ErrorRoutines` (cleanup hooks); if `Patch_Errors` (monitor) set → jump there.
2. Clear `PrintPos`, `InputFlg`, `ContFlg`.
3. **Not trappable** if: `d0 < 19` (except **11 Out of variable space, which is trappable**), `d0 >= 1000`, direct mode, or already handling an error (`ErrorOn != 0`).
4. If `TrapAdr == d7` (current instruction is the one following `Trap`): `TrapErr = (n+1) | ((ext+1)<<8)`, stack reset to `BasSp`, skip the faulting instruction and continue after it.
5. Else if `OnErrLine` set: `ErrorOn = (n+1) | ((ext+1)<<8)` (core: high byte 0), reset expression stack (`a3 = PLoop`) and SP, `ErrorChr = address of failing instruction` (for Resume). If `ErrorChr` ≥ 0 → **On Error Goto**: continue at `OnErrLine`. If negative (flag) → **On Error Proc**: call the procedure with resume address = failing instruction (`InProE`).
6. Otherwise (`rErr1`): `VerPos = d7-2` (position of failing token, adjusted for Include files via `Includes_Adr`), clear includes, `Prg_Pull`, then `jmp Prg_JError` with `d0` (for extension errors `d0=1` and `a0` = message text).

`Errn` returns `ErrorOn-1` (0 if no error) (+Lib.s:1716). `Err$(n)` returns run-time message `n+1` (+Lib.s:1724). `Error n` clamps n to 255 then raises it (+Lib.s:11396). Error 10 "End of program" is how `End` reaches the editor (Ed_ErrRun treats 10 and 1000 as silent return).

---

# Part E: Banks, floats & number formatting, menus, dialogs

## AMOS Professional: banks, number formatting, menus, dialogs/Interface

Source root: `AMOS-Professional-365/`. All `file:line` references are to that tree. Files are ISO-8859-1. Line numbers come from the copy studied for this document.
Everything here was read from the 68000 source. Each byte layout was also checked against the real files `bin/+AMOSPro_Mouse.abk`, `AMOS/APSystem/*.Abk` and a set of `*.AMOS` programs, using hexdumps and a small Python parser.

---------------------------------------------------------------------------

### 1. BANK SYSTEM

#### 1.1 Flag bits (`+Equ.s:1837-1840`)

| bit | name | meaning |
|---|---|---|
| 0 | `Bnk_BitData` | "Data" bank (saved with program, survives `Erase Temp`). Clear = "Work" (temporary) |
| 1 | `Bnk_BitChip` | allocated in chip RAM |
| 2 | `Bnk_BitBob`  | sprite/bob bank ("Sprites ") |
| 3 | `Bnk_BitIcon` | icon bank ("Icons   ") |

#### 1.2 Bank list and the in-memory structure

* Each program has its own list. `Prg_Banks` sits in the program structure (`+Equ.s:1858`). The current list head is `Cur_Banks(a5)` (`+Equ.s:1327`). `Bnk.PrevProgram` and `Bnk.CurProgram` (`+B.s:1262…`) swap `Cur_Banks` between the current and the calling program. `BStart`, `BLength`, `BGrab` and `BSend` use this swap (`+Lib.s:2240-2326`).
* The list is a simple singly-linked list of `NEXT.l, SIZE.l` elements (`+B.s:1180-1260`):
  * `Lst.Cree` / `Lst.New` / `Lst.ChipNew` allocate `size+8` bytes and **insert at the head**. The stored SIZE is the requested size, without the 8 header bytes.
  * `Lst.Del` unlinks and frees. `Lst.Insert` and `Lst.Remove` only relink.
  * Consequence: banks are kept in reverse order of creation or loading.

**Normal bank** (`Bnk.Reserve`, `+Lib.s:8441-8480`): `Reserve` is called with length L. It allocates `L+16` bytes through `Lst.Cree`, so the block is `L+24` bytes in total. Memory is `Public|Clear`, plus `Chip` if the chip bit is set.

```
+0   l  next element
+4   l  size  = L+16           (Length() = size-16)
+8   l  bank number (1..65535)  <- Bank Swap swaps this long
+12  w  flags (Bnk_Bit*)
+14  w  0
+16  8b name, space padded ("Work    ", "Data    ", ...)
+24  .. data  (Start(n) returns this address; L bytes)
```

`Bnk.GetAdr` (`+Lib.s:7888`) walks the list comparing the long at +8. It returns the flags in d0.w and the data address (+24) in a0 and a1.

**Sprite/Icon bank** (`Bnk.Ric2`, `+Lib.s:8139-8233`): allocated with `Lst.New` (fast RAM). The size is `n*8 + 8*2 + 2 + 64`.

```
+0  next, +4 size, +8 number (ALWAYS 1 for sprites, 2 for icons), +12 flags
    (Bob|Data or Icon|Data), +14 0, +16 "Sprites " / "Icons   "
+24 w   count n
+26 n * { l image_ptr, l mask_ptr }    (0 = empty slot; mask_ptr<=0 = no mask)
+26+8n  32 words palette
```

Each image is a separate chip allocation of `10 + w*h*d*2` bytes:

```
+0 w width in WORDS (16-pixel units)
+2 w height (lines)
+4 w depth (planes)
+6 w hot-spot X  (top 2 bits = flip flags in memory; see below)
+8 w hot-spot Y
+10 planar data: plane 0 (h rows of w words), then plane 1, ... (plane-major)
```

Plane-major order was confirmed on `+AMOSPro_Mouse.abk` image 1 (w=1, h=11, d=2): plane 0 holds the arrow outline and plane 1 the fill.
The mask (pointer +4) is a separate block whose first long is its own size (`Bnk.EffBobA0`, `+Lib.s:7984`). It is never saved.

Bank names (`+Lib.s:3600-3640`) are "Sprites ", "Icons   ", "Music   ", "Amal    ", "Menu    ", "Data    ", "Work    ", "Asm     ", "Iff     " and "Loading!" (a temporary name used during load).
Other names come from other modules:
* "Pac.Pic." — `+Compact.s:264`, created with flags Data|Fast.
* "Samples " — checked as `"Samp"` in `+Music.s:3193`.
* "Tracker " — `+Music.s:4179`.
* "Resource" — checked as `"Reso"` in `+Lib.s:14922` and `+B.s:2096`.
* "Datas   " and "Code" — names seen in programs.

`Length()` and `Start()` treat any bank uniformly: they only look at the flags and the `size` field.

#### 1.3 Instructions

| Instruction | Location | Behaviour |
|---|---|---|
| `Reserve As Work n,L` | `+Lib.s:2398` | `Bnk.Reserve(n, flags=0, L, "Work    ")` |
| `Reserve As Chip Work` | `+Lib.s:2404` | flags=Chip, name "Work" |
| `Reserve As Data` | `+Lib.s:2411` | flags=Data, name "Data" |
| `Reserve As Chip Data` | `+Lib.s:2418` | flags=Data+Chip |
| (common `RsBqX`) | `+Lib.s:2425` | L must be > 0 and 1 <= n < 65536, otherwise "Illegal function call". If allocation fails: "Out of memory". Any existing bank n is erased first (`Bnk.Reserve`). New memory is cleared. |
| `Erase n` | `+Lib.s:2180` | `Bnk.Eff`. A missing bank is silently ignored. |
| `Erase Temp` | `+Lib.s:2188`, `Bnk.EffTemp` `+Lib.s:8030` | erases every bank whose Data bit is **clear** |
| `Erase All` | `+Lib.s:2196`, `+Lib.s:8012` | erases every bank |
| `Bank Swap a,b` | `+Lib.s:2204-2236` | swaps the number longs (+8). If one bank does not exist, the other is simply renumbered. Both numbers must be 1..65535. |
| `Bank Shrink n To L` | `+Lib.s:2441`, `Bnk.Schrink` `+Lib.s:8236` | normal banks only (bob/icon → error 23). New L+24 must be <= the old allocation. Implemented as FreeMem + AllocAbs at the same address, then `size = L+16`. Missing bank → error 36. |
| `=Start(n)` | `+Lib.s:2452` | data address. Missing bank → "Bank not reserved". |
| `=Length(n)` | `+Lib.s:2463` | `size-16`. For sprite/icon banks it is the **number of images** (word at +24). Missing bank → 0 (no error). |
| `=BStart/=BLength` | `+Lib.s:2240`,`2254` | the same, on the previous program's bank list |
| `BGrab n` / `BSend n` | `+Lib.s:2275`,`2304` | move a bank between program lists |
| `Ins/Del Sprite/Icon` | `+Lib.s:2330-2397`, `Bnk.InsBob` `+Lib.s:8287`, `Bnk.DelBob` `+Lib.s:8343` | rebuilds the pointer table. Deleting the last image erases the bank. |
| `Listbank` | `+Lib.s:2164`, `Bnk.List` `+Lib.s:8587` | see below |
| `Bnk.OrAdr` | `+Lib.s:8053` | an "address or bank" argument: values < 1024 are bank numbers (replaced by Start), anything else is an address |

`Bnk.Change` (`+Lib.s:8553`) must run after every bank change. It notifies the 26 extensions (the 4th long of each `ExtAdr` entry) and re-installs the sprite bank (`SyCall SetSpBank`).

**Listbank line format** (`Bnk.List`): banks are printed in ascending number order. It repeatedly picks the smallest number greater than the previous one, so the list order does not matter.

```
[" " if n<10] <decimal n> " - " <8-char name> " S: $" <8 hex digits, upper> " L: " <decimal len> CR LF
```

`len` is `size-16`, or the image count for bob and icon banks. Example: ` 1 - Sprites  S: $00C2A0F8 L: 39`.

#### 1.4 File formats

Magic longs (`NHunk`, `+Lib.s:3791`): index 1 = "AmBk", 2 = "AmSp", 3 = "AmBs", 4 = "AmIc".
All values are big-endian.

##### "AmBk" – normal bank (`Bnk.SaveA0` `+Lib.s:3894-3930`, `Bnk.Load` `+Lib.s:4025-4073`)

```
0   4  "AmBk"
4   w  bank number
6   w  0 = CHIP, 1 = FAST      (save: 1 unless chip bit; load: ==0 -> chip)
8   l  bit31 = DATA bank, bits 0-27 = length + 8   (bits 28-30 masked off on load)
12  8  name (space padded)
20  L  data, where L = (long & 0x0FFFFFFF) - 8
```

Checked against `AMOSPro_Default_Resource.Abk`: number=16, fast, `0x800026CE` (Data, 0x26CE = 9934 = filesize 9946 − 12), name "Resource".
`AMOSPro_Editor_Samples.Abk`: number=5, chip, "Samples ".

Load number selection: `Load f$` passes `EntNul` (negative), so the number stored in the file is used, and file number 0 becomes **5**. `Load f$,n` forces n. The bank is reserved under the temporary name "Loading!" and then the 8 name bytes plus data are read over it. The name is therefore whatever the file contains.

##### "AmSp" / "AmIc" – sprites / icons (`SB_Bob`/`SB_Icon` `+Lib.s:3932-3990`, `LB_Sprites`/`LB_Icons` `+Lib.s:4093-4180`)

```
0  4  "AmSp" or "AmIc"
4  w  count n
then n records:
     w width_words, w height, w depth, w hotX, w hotY   (10 bytes)
     width_words*height*depth*2 bytes planar data (plane-major)
     An empty slot is written as 10 zero bytes and no data.
end: 32 words palette ($0RGB, 64 bytes)
```

* No bank number is stored. Sprites always go into bank 1 and icons into bank 2.
* Load with number 0, or no number: overwrite.
* Load with a non-zero n while the bank exists: **append** the new images after the existing ones. The palette is taken from the file.
* On save, `Bnk.UnRev` first un-flips every image and sign-extends hot-spot X from 14 bits (`lsl #2; asr #2`, `+Lib.s:8408`).
* On load, hot-spot X is masked with `$3FFF`.
* `+AMOSPro_Mouse.abk` checks out: 39 images, first image 1×11×2. The 64-byte palette ends exactly at the end of the file.

##### "AmBs" – multi-bank (`Bnk.SaveAll` `+Lib.s:3833`, `LB_Multiples` `+Lib.s:4183`)

```
0 4 "AmBs"
4 w number of banks
then each bank as a complete AmBk / AmSp / AmIc record (with its own magic)
```

On load, **every bank is erased first** (`Bnk.EffAll`). Then each sub-bank is loaded with its default number.
Banks are saved in list order, i.e. newest first.

`Save f$` (`InSave1`, `+Lib.s:3820`) always writes AmBs. `Save f$,n` (`InSave2`, `+Lib.s:3800`) writes the single bank.

`Bload f$,addr|bank` (`+Lib.s:4269`) reads the whole file. `Bsave f$,start To end` (`+Lib.s:4304`) writes `end-start` bytes.
`PLoad f$,n` (`+Lib.s:4209`) skips to the first `HUNK_CODE` ($3E9) of an executable and loads it as an "Asm" Data bank. A negative n means chip RAM.

##### Sample bank data (bank name "Samples ", `GetSam` `+Music.s:3181-3206`)

```
data+0  w count
data+2  count * l offset (from data+0) of each sample; 0 = undefined
sample: 8 b name, w frequency (Hz), l length, data bytes (8-bit signed)
```

Checked against the Editor_Samples bank: 9 samples, first one at offset 0x26, named "A       ".

##### Packed picture (Pac.Pic.) headers (`+Equ.s:896-928`)

* Screen header: `PsCode.l=$12031990`, `PsTx`, `PsTy`, `PsAWx`, `PsAWy`, `PsAWTx`, `PsAWTy`, `PsAVx`, `PsAVy`, `PsCon0`, `PsNbCol`, `PsNPlan` (all words), then `PsPal` (32 words).
* Bitmap header: `Pkcode.l=$06071963`, `Pkdx.w`, `Pkdy.w`, `Pktx.w` (bytes), `Pkty.w` (char rows), `Pktcar.w` (lines per row), `Pknplan.w`, `PkDatas2.l`, `PkPoint2.l` (24 bytes).
* The packer itself is in `+Compact.s` and `UnPack_Bitmap`.

#### 1.5 .AMOS program file (`Prg_Load` `+Verif.s:4789`, `Prg_Save` `+Verif.s:4962`)

```
0   16  header:  "AMOS Pro101V\0\0\0" + mathflags byte   (Pro)
                 or "AMOS Basic V134 "                    (1.3)
        byte 11 = 'V' if the program was verified ("tested"), 'v' if not (save side)
        byte 15 (Pro) = Prg_MathFlags (bit7 = double precision, see §2)
16  l   tokenised source length S (bytes; up to the first zero-length line)
20  S   tokenised source
20+S    banks: ALWAYS an "AmBs" record (count may be 0)
```

Load checks:
* Pro is recognised by its first **8** chars ("AMOS Pro"). 1.3 is recognised by its first **10** chars ("AMOS Basic").
* The real files use the variants "AMOS Basic v1.34", "AMOS Basic V134 " and "AMOS Basic v134 ".
* Not a 1.3 header ⇒ `Prg_Not1.3 = 1`.
* Byte 15 is copied to `Prg_MathFlags` only for Pro headers. For 1.3 it is 0.
* Seen in the wild: $00, $03, and $30 (`Sample_Bank_Maker.AMOS`). Only bit 7 (double) matters on load.
* The source goes at the top of the text buffer, followed by a zero word. Then `Bnk.Load` reads the AmBs record (`+Verif.s:4840-4846`).

Checked against `Object_Editor.AMOS` (3 AmBk) and `SuperBlockout.AMOS` (AmSp 31, AmIc 1, Music, Samples, 3×Pac.Pic.). Both parse exactly to EOF.

`+Header.s:835-837` checks "AmSp"/"AmIc" when a compiled or run-only program's banks are relocated.

---------------------------------------------------------------------------

### 2. FLOATING POINT AND NUMBER⇄TEXT

#### 2.1 Formats and libraries

* **Single precision = Motorola FFP**, a 32-bit long:
  * bits 31..8: 24-bit mantissa, normalised so that bit 31 is 1 (value 0.5–1).
  * bit 7: sign.
  * bits 6..0: exponent, excess-64.
  * value = mant/2^24 × 2^(exp−64). 0 = `$00000000`.
  * Constants: 1.0 = `$80000041`, 10.0 = `$A0000044`, 2.0 = `$80000042`, 0.5 = `$80000040`, π = `$C90FD942`, 180 = `$B4000048` (`+ILib.s:628-631`).
* **Double = IEEE 754 double**, held in d3 (high) and d4 (low).
* The token for a float constant (`FnCstFl`, `+ILib.s:4113`) always stores FFP. In double mode it is converted FFP→IEEE single→double (`FFP2Ieee` `+Lib.s:25787`, `Sp2Dp` `+Lib.s:25846`).
* The double-constant token (`FnCstDFl`, `+ILib.s:4129`) stores an IEEE double. In single mode it is converted Dp2Sp→Ieee2FFP.
* `MathFlags(a5)` (`+Equ.s:1504`), mirrored in `Prg_MathFlags`:
  * bit 0: float used
  * bit 1: transcendental functions used
  * bit 7: **double precision**
  * `Set Double Precision` sets `%10000011` (`+Verif.s:774-783`). It must come before any variable use, otherwise error 50.
  * The Verif pass ORs bits 0/1 whenever float or trig is used (`+Verif.s:2570,2675,2827`).
* Libraries:
  * `mathffp.library` is always opened at startup (`+B.s:1764-1769`, name `+B.s:2910`).
  * `Open_MathLibraries` (`+ILib.s:625-664`) opens `mathtrans.library` if bit 1 is set.
  * If bit 7 is set it opens `mathieeedoubbas.library` and `mathieeedoubtrans.library`. It also switches ValPi/Val180 to double constants and calls `Lib_FloatSwap`, which swaps the single/double jump vectors of every library that has `LB_FFloatSwap`.

#### 2.2 Fix state

* `FixFlg.w` / `ExpFlg.w` (`+Equ.s:1388-1389`). At run start: `FixFlg=-1`, `ExpFlg=0` (`+Verif.s:3964-3965`).
* `Fix n` (`InFix`, `+Lib.s:1929-1942`):
  * n ≥ 0: `ExpFlg=0`.
  * n < 0: `ExpFlg=1` and n=−n.
  * Then, if n ≥ 16 (unsigned compare): `FixFlg=-1`, else `FixFlg=n`.
* So: `Fix 16` gives proportional (the default). `Fix -1` gives exponential with 1 decimal. `Fix -16` gives exponential with proportional digits (FixFlg=−1, ExpFlg=1).

#### 2.3 Integer printing (Print / Str$ of an integer)

`LongToAsc` (`+Lib.s:25671-25716`) is called with d3=−1 (proportional) and d4=1 (signed) from Print (`+ILib.s:5126`) and Str$ (`FnStrE`, `+Lib.s:1754`).
* Negative: "-" followed by the magnitude. `$80000000` prints as -2147483648.
* Otherwise a leading **space**.
* Then decimal digits with leading zeros suppressed. The last digit is always printed.
* Examples: `Print 5` → " 5", `Str$(-12)` → "-12", `Str$(0)` → " 0".

`LongToDec` (`+Lib.s:25659`, used by Listbank and others) calls the same routine with d4=0, so there is no leading space.

#### 2.4 Hex$ / Bin$ (`+Lib.s:14239-14283`, `LongToHex` `+Lib.s:25718`, `LongToBin` `+Lib.s:25754`)

* `Hex$(v)`: "$" followed by upper-case hex digits with leading zeros suppressed (last digit always printed). `Hex$(0)` = "$0". `Hex$(-1)` = "$FFFFFFFF".
* `Hex$(v,n)`: "$" followed by the **low** n digits of the 8-digit value, with leading zeros kept. `Hex$(255,4)` = "$00FF". n=0 gives "$". n>8 behaves as proportional (because skip = 8−n < 0).
* `Bin$` works the same way with "%" and 32 digits.

#### 2.5 Single precision float → text (Print, Str$)

Entry point is `Float2Ascii` (`+ILib.s:7676`). It sets d0 = FFP value, d4 = FixFlg with bit 31 cleared (bit 31 clear means "add a leading space"), d5 = ExpFlg, then jumps to `FloatToAsc` (`+Lib.s:25923`). `Str$` uses the same path (`FnStrF` `+Lib.s:1766`).

##### Core primitive `ffp2a(x, buf, prec)` (`+Lib.s:26226-26270`)

This is a C-style ftoa that does **all arithmetic in FFP**, using its own routines:
* `L28212` add (`L28422`)
* `L28232` compare
* `L28250` div (`L28518`)
* `L28270` long→FFP
* `L28300` FFP→long (truncating)
* `L28388` mul (`L2858A`, rounds with +$80)
* `L283A8` neg
* `L283C4` sub

```
ndig = prec<=0 ? 1 : prec>22 ? 23 : prec+1
e = 0
if x<0 { out '-'; x=-x }
if x>0 { while x<1.0 { x*=10; e-- } }
while x>=10.0 { x/=10; e++ }
ndig += e
r = 1.0; for i=1..ndig-1: r /= 10        // r = 10^-(ndig-1), by repeated FFP division
x += r/2                                  // r / 2.0
if x>=10.0 { x = 1.0; e++ }
if e<0 { out "0."; if ndig<0 { e -= ndig }; for i=-1; i>e; i-- out '0' }
for i=0; i<ndig; i++ { d=int(x); out '0'+d; if i==e out '.'; x=(x-d)*10 }
out NUL     // a0 left pointing at NUL; d0 = buffer start
```

A bit-exact port **must emulate FFP add/sub/mul/div** (24-bit mantissa, the exact rounding of these routines). It cannot use f32 here.

##### `F2a(x, d4=fix, d5=exp)` (`+Lib.s:25976-26222`), result in `DeFloat(a5)`

* **A. Fixed** (d5==0 and fix ≥ 0, i.e. `Fix 0..15`):
  * If fix ≥ 8 then fix = 7 (single precision never shows more than 7 decimals).
  * `ffp2a(x,fix)`.
  * If fix==0 and the last char is '.', remove it.
* **B. Proportional or exponential** (fix<0 or d5≠0). Classify by the FFP exponent byte e = x & $7F:
  * e ≥ $41 (|x| ≥ 1): prec=7.
  * e ≥ $31: prec=10.
  * else (tiny or 0): prec=22.
  * s = `ffp2a(x,prec)` (sign skipped for analysis).
  * **|s| starts with a non-'0'** (≥1 after rounding): n = number of integer digits + 1.
    * If d5≠0 or n≥8 (≥7 integer digits) → **ExFix1**.
    * Else decimals = min(7−n, 5) → **Clean**.
  * **s starts with '0'**: z = count of '0's after "0.", plus 1 (counting stops at the first non-'0' or the end).
    * If z ≥ 22 (true zero): z=6 and the zero flag is set.
    * Else if z ≥ 4 (|x| < 0.001): **ExVir1**.
    * If d5≠0: **ExVir1**.
    * Else decimals = z+6 → **Clean**.
* **Clean(dec)**: `ffp2a(x,dec)`, then strip trailing '0's after the point, and the point itself if nothing remains after it.
* **ExFix1** (|x|≥1, exponent form):
  * exp = n−2 (integer digits − 1).
  * n' = min(n,7). s = `ffp2a(x, 9−n')`.
  * Output: [sign] first digit, '.', then the next k digits of s (skipping the '.'). k = min(fix,5) if fix≥0, else 5. The digits are **copied, not re-rounded**, so this truncates.
  * If fix<0: strip trailing zeros (and the '.').
  * Then "E+" and a 2-digit exponent ("E+0" then the tens increment that '0': "E+06", "E+12").
* **ExVir1** (|x|<1):
  * Zero: source "0.0000000", exp 0, sign "+".
  * Otherwise: exp = z, s = `ffp2a(x, z+6)`.
  * Output: [sign] first non-zero digit (or '0'), '.', then up to k digits, k = min(fix,6) if fix≥0, else 6.
  * If fix<0: strip trailing zeros.
  * Then "E-" and 2 digits (zero gives "E+00").

##### `FloatToAsc` post-processing (`+Lib.s:25923-25972`)

* If d4 bit 31 is clear: emit ' '. If the number begins with '-', the space is overwritten, so negatives get no space.
* If fix ≥ 0: copy F2a's string verbatim.
* If fix < 0 (proportional):
  * Copy the integer part up to '.'. If there is **no '.', copy everything** (this includes forms like "1E-04").
  * Otherwise emit '.' and the decimals up to the last non-zero digit. Emit no point if all decimals are zero.
  * Then, if an 'E' follows, emit **a space before it** (comment: "imprime un espace avant le E").
  * Result: proportional exponent output looks like " 1.23456 E+07", while " 1E+07" (no point) has no space.
  * This is what the code says. Verify against a real machine before relying on it. `Print Using` explicitly skips a space next to the E, see `+ILib.s:5291`.

Worked examples (single, default Fix):

| value | output |
|---|---|
| 3.14159 | " 3.14159" |
| 1.5 | " 1.5" |
| 100.0 | " 100" |
| 0.5 | " 0.5" |
| -2.25 | "-2.25" |
| 0 | " 0" |
| 12345678.0 | " 1.23456 E+07" (7th digit truncated) |
| 0.0001 | " 1E-04" |

With `Fix 2`, 3.14159 gives " 3.14". With `Fix 0` it gives " 3". With `Fix -3` it gives " 3.141E+00" (truncated, no space before E).

#### 2.6 Double precision float → text (`Float2AsciiD`, `+ILib.s:7684-7708`)

* mode = 2, ndig = 15.
* If FixFlg ≥ 0: ndig = FixFlg. If ExpFlg is also set: mode = 0.
* Mode 1 (%f) is **never used**. As a result, `Fix n` in double precision means **n significant digits** (%g style), not n decimals.
* Emit ' ' if the sign bit of the high word is clear (+0 gets a space).
* Call `DoubleToAsc(hi, lo, buf, ndig, mode)` (`Dtoa`, `+Lib.s:27080-27360`). It uses software IEEE routines:
  * `L3DFA0` mul, `L3E12A` div, `L3DE80` add, `L3DE7A` sub, `L3DDC6` cmp, `L3DD4A` dbl→int, `L3DDA6` int→dbl, `L3DE4E` neg, `L3E854` / `L3E87C` integer div/mod.
  * Constants at `DDebut` (`+Lib.s:28335`): [0]=10.0, [1]=1.0, [2..17] = 0.5, 0.05, 0.005, … 5e-16.

```
At entry, bit5 ("#" flag) of the mode byte is forced ON (cleared at exit).
if exponent==0x7FF: write ndig copies of '+' or '-' (sign), NUL, exit
D4=0; if x<0 {x=-x; out '-'}
if x>0 { while x<1 {x*=10; D4--}; while x>=10 {x/=10; D4++} }
mode 2: if ndig==0 ndig=1; D7=2; if D4<-4 || D4>=ndig then D7=-1 (exponent); D5=ndig
mode 1: D5=ndig+D4+1          mode 0: D7=0; D5=ndig+1
if D5>0 { x += round[min(D5,16)]   // round[k] = 5*10^-k (table)
          if x>=10 { x=1; D4++; if D7>0 D5++ } }
if D7>0 { if D4<0 { out "0."; z = (D5>0) ? -D4-1 : ndig; out z '0'; A6=0 }
          else A6 = D4+1 }               // digits before point
else A6=1
loop while D5>0 (A3=0..): digit = (A3<16) ? int(x) : '0' ; if A3<16 x=(x-digit)*10
       out digit; if --D5==0 break; if A6 && --A6==0 out '.'
if A6!=0 out '.'                        // '#' flag always set
if D7<=0: out 'e' (lowercase), sign '+'/'-', then 3 digits: D4/100, (D4%100)/10, D4%10
NUL
exit fix-up (only if mode param == 2): find '.', remember the position after the last
non-'0' digit following the point (or the point itself if none) and move the
"e…"/NUL tail down to it: trailing zeros and a bare point are removed.
```

Examples (double, default): 3.5 → " 3.5"; 123.0 → " 123"; 1e20 → " 1e+020"; 1.5e-7 → " 1.5e-007"; 0.001 → " 0.001".

#### 2.7 Text → number (Val, Input) — `FnVal` `+ILib.s:6653`, `ValRout` `+ILib.s:7018-7262`

1. An empty string gives integer 0.
2. Copy the string into the buffer (max 510 chars). With the sign option on, skip spaces. A '-' sets the negate flag, and '+' is skipped. The sign position is remembered.
3. Skip spaces.
   * '$' → `hexalong`: 0-9, A-F (lowercase is upper-cased). Overflow on the 9th digit makes the whole result 0.
   * '%' → `binlong`: up to 32 bits; the 33rd digit overflows → 0.
   * '.' or a digit → decimal.
   * Anything else → integer 0.
4. Decimal: scan ahead. Spaces are allowed anywhere. A first '.' or an 'e'/'E' that is followed by '+', '-' or a digit makes it a float. A second '.' or any other character ends the number. 'E' is rewritten to 'e'.
5. Integer (`declong`, `+ILib.s:7187`):
   * acc = acc*10 + d. Spaces between digits are skipped.
   * Overflow beyond $7FFFFFFF gives **0** (not an error).
   * Then negate if the sign flag is set.
   * Returns integer type (`_TkEnt`; hex `_TkHex` and bin `_TkBin` are also integers).
6. Float: copy the non-space chars from the sign position to the end of the number (max 33 chars). Then:
   * single → `AscToFloat` → `a2ffp` (`+Lib.s:26774`)
   * double → `AscToDouble` → `L3D9C0` (`+Lib.s:27361`)

**a2ffp** (single):
* Skip spaces and tabs. Sign. Collect every char up to NUL or 'e'/'E' into a buffer, dropping '.' and counting the digits after it as `fd`.
* Optional exponent with its own sign.
* `L280A8`: v = Σ in FFP (v = v*10 + digit) over the leading digit run.
* `L28654`: exponent as a signed 16-bit value.
* p = (expsign ? −e : e) − fd.
* scale = 10^p, built by repeated FFP ×10 or ÷10 starting from 1.0 (`L28040`).
* result = v × scale.
* `L28116` renormalises the result (it divides or multiplies by 2 into [0.5,1), multiplies by 2^24, truncates to an integer and rebuilds the FFP). The sign bit is ORed in.
* Bit-exact emulation needs FFP arithmetic.

**AscToDouble**: integer digits are summed right-to-left as `digit × 10^k` (10^k by repeated multiplication), fraction digits as `digit / 10^k`. Then ÷ or × 10^exp. Overflow returns ±`$43A999998D2C7175`. The result is not correctly rounded.

#### 2.8 Print statement formatting (`+ILib.s:5046-5333`)

* ',' emits TAB (9).
* ';' emits nothing.
* At the end of the statement (no trailing separator), emit CR LF.
* Strings are sent in chunks of 120 bytes.
* `Using` (`using1` `+ILib.s:5177`, `using50` `+ILib.s:5305`):
  * '#' is a digit position; '+' and '-' are sign positions; '.' aligns the decimal point; ';' marks the point without printing it; '^' is an exponent position (fake "E+000" if the number has no exponent).
  * For strings, '~' is a character position.

---------------------------------------------------------------------------

### 3. MENUS

#### 3.1 Data structures

**Menu node** (`+Equ.s:779-830`, `MnLong` = 70 bytes, fast RAM, cleared):

```
0  l MnPrev     4 l MnNext (next item at same level)   8 l MnLat (first child)
12 w MnNb (item number at this level)                    14 w MnFlag
16 w MnX  18 w MnY  20 w MnTx 22 w MnTy 24 w MnMX 26 w MnMY 28 w MnXX 30 w MnYY
32 w MnZone   34 b MnKFlag (0 none, 1 ASCII, -1 scancode+shift)
35 b MnKAsc   36 b MnKSc   37 b MnKSh
38 l MnObF (background object)  42 l MnOb1 (normal)  46 l MnOb2 (selected)
50 l MnOb3 (inactive)  54 l MnAdSave  58 l MnDatas  62 w MnLData
64..69 b MnInkA1,B1,C1,A2,B2,C2
```

MnFlag bits: 0 MnFlat, 1 MnFixed (position set), 2 MnSep (Separate), 3 MnBar (Bar), 4 MnOff (Inactive), 5 MnTotal (Tline), 6 MnTBouge (Movable), 7 MnBouge (Item Movable). The low byte of MnFlag is also used as the Called flag (`Menu Called` sets it to −1, `Menu Once` sets it to 0).

**Globals** (`+Equ.s:1399-1426`):
* `MnNDim` = 8 levels.
* `MnBase` (root), `MnChange`, `MnMouse`, `MnAdEc` (menu screen).
* `MnChoix` (8 words: the selected item number at each level).
* `MnDFlags` (8 default flag bytes per level).
* `MnChoice` (the "a choice happened" word).
* `MnProc`.
* `OMnBase`, `OMnNb`, `OMnType` (On Menu jump table).

#### 3.2 Instructions

| Instruction | Location | Behaviour |
|---|---|---|
| `Menu$(l1,…,ln)=a$[,b$[,c$[,d$]]]` | `+ILib.s:6827-6915` | finds or creates the node via `MnFind`/`MnIns` (`+Lib.s:17161`, `17196`). Each level number must be 1..1023. Takes inks from the current screen (paper/pen). Compiles each string into MnOb1, MnOb2, MnOb3, MnObF with `MnObjet`. A missing argument (EntNul) leaves the object unchanged. An empty string deletes it. Increments `MnChange`. |
| `Menu Del [(…)]` | `+ILib.s:6925` | no args: `MnRaz` (delete all). With args: delete that branch. |
| `Set Menu (…) To x,y` | `+ILib.s:6944` | sets MnX/MnY and MnFixed |
| `Menu Key (…) To c$ \| scan[,shift]` | `+ILib.s:6731` | leaf only (error if MnLat≠0). Scan < 128, shift < 256. Without "To": disable. |
| `On Menu Goto/Gosub/Proc l1,l2,…` | `+ILib.s:6780` | stores the label addresses (4 bytes each) in `OMnBase`, `OMnNb`=count, `OMnType`=token. Clears BitJump. |
| `On Menu On` / `Off` | `+Lib.s:15326-15340` | On sets BitJump in ActuMask if any labels exist. (Off as written also *sets* the bit, which looks like a source bug.) |
| `On Menu Del` | `+Lib.s:15346` | frees the jump table |
| `Menu On`/`Off` | `+Lib.s:15563-15578` | set/clear BitMenu (On only if MnBase≠0) |
| `Menu Mouse On/Off`, `Menu Base x,y`, `Menu X/Y(…)`, `Menu Bar/Line/Tline/Movable/Static/Item Movable/Item Static/Active/Inactive/Separate/Link/Called/Once`, `Menu Calc` | `+Lib.s:15582-15757` | flag setters on `MnDim`. A plain level number addresses MnDFlags[n−1] (the default for new items at that level). A parenthesised list addresses that node's MnFlag. |
| `=Choice` | `+Lib.s:15635` | returns MnChoice (−1 if a choice was made) and clears it |
| `=Choice(n)` | `+Lib.s:15642` | MnChoix[n−1], n=1..8 |
| `Menu To Bank n` | `+Lib.s:15372-15456` | flattens the tree into one Data/Fast bank "Menu    " |
| `Bank To Menu n` | `+Lib.s:15465-15560` | rebuilds the tree. The bank name must be "Menu    ". |

**On Menu dispatch** (`GoMenu`, `+ILib.s:1038-1080`): this runs from the tests loop when BitJump is set. It reads MnChoix[0], which must be 1..OMnNb. It clears BitJump, so On Menu On must be re-issued, and resets the stack to BasSp. It then does a Goto, Gosub or Proc to label[MnChoix[0]−1].
`MenuKeyExplore` (`+Lib.s:17655`) matches keyboard shortcuts against leaf nodes and fills MnChoix the same way.

**Menu To Bank format**: a depth-first copy. Each node is written as its 70 raw bytes, then its 4 objects in the order ObF, Ob1, Ob2, Ob3 (each a raw copy of the object bytes, length = the object's first word). Then come the children (MnLat subtree), then the next sibling.
In the copy, `MnLat`, `MnNext` and the four object pointers are replaced by **offsets from the start of the bank data**, with 0 meaning none. `MnPrev` keeps its stale value and is rebuilt on load.

#### 3.3 Menu item string language (`MnObjet`, `+Lib.s:17435-17650`)

The source string is plain text mixed with commands in parentheses: `"(BA 20,10)Load(LO 2,1)…"`.
* An ESC (27) skips itself plus the next 2 chars.
* Inside the parentheses, commands are separated by ':'.
* Only the first 2 letters of a command name count; further letters are skipped (case-insensitive).
* Numeric arguments are decimal, optionally with '-', and separated by ','.

**Compiled object** (fast RAM): word total size (including itself), then a sequence of word opcodes, then a 0 word at the end:

| op | src | args | meaning (`MnODraw` `+Lib.s:16624-16990`) |
|---|---|---|---|
| 4 | text | w len, bytes (padded even) | print text at the cursor (Text), advance X by TextLength, Y by font height |
| 8 | BA | x,y | RectFill from cursor to (x,y). Cursor = (x+1,y+1) |
| 12 | LI | x,y | Draw line cursor→(x,y) |
| 16 | EL | rx,ry | Ellipse |
| 20 | PA | n | Set pattern |
| 24 | IN | n,c | ink n (1=A, 2=B, 3=C) = c |
| 28 | BO | n | paste bob n |
| 32 | IC | n | paste icon n (cursor advances by width×16, height) |
| 36 | LO | x,y | locate cursor |
| 40 | OU | f | outline on/off |
| 44 | SL | n | set line pattern |
| 48 | SF | n | set font |
| 52 | PR | name | call procedure: w length + name bytes (padded). Registers: D0/D1 = x,y; D2 = draw flag; D3 = active; D4 = first draw; A0 = menu node; A1 = data zone. Returns new x,y. |
| 56 | RE | n | reserve n bytes of zone data (MnDatas) |
| 60 | SS | n | set text style |

The jump table `ObJumps` is at `+Lib.s:16671`. The token table `MnOToken` is at `+Lib.s:17634`. Opcode = 8 + 4×index.

---------------------------------------------------------------------------

### 4. DIALOGS / INTERFACE LANGUAGE

#### 4.1 Resource bank ("Resource", normally bank 16)

`Dia_GetPuzzle` is at `+Lib.s:14914`. The default system resource `Sys_Resource` is loaded at startup from `AMOSPro_Default_Resource.Abk` as bank 16 (`+B.s:2085-2098`).
Bank data, with offsets relative to the start of the data (after the 8-byte name):

```
0   w  N = number of valid sections (1=graphics, 2=+messages, 3=+programs)
2   l  offset graphics section
6   l  offset messages section
10  l  offset programs section
14  .. further longs may follow (editor data; e.g. Default has 3 extra) — ignore,
       always use the offsets.
```

The default resource: N=3, offsets $1A / $16F0 / $18FA.

**Graphics section** (`Resource Unpack` `+Lib.s:14969`, `Dia_Unpack` `+Lib.s:21618`, `Dia_RScOpen` `+Lib.s:20976`):

```
w  count G
G * l offset (from the graphics base) of packed bitmap #i (1-based), BMCode $06071963 header
w  number of colours (4096 => HAM: 6 planes, mode|$0800)
w  screen mode (only bits $8004 used: hires/laced)
32 w palette
(optional: w length + name string, e.g. "AMOSPro_System…"; unused at runtime)
packed bitmaps ...
```

Default resource: G=93, 8 colours.

**Messages section**: a leading byte 0, then for each message `[len.b][chars][0]` (equivalently `[0][len][chars]`), ending with len = $FF. Messages are 1-based.
`GetMessage` is at `+B.s:553`. `=Resource$(n)` (`+ILib.s:6670`):
* n > 0: resource message n.
* n = 0: system path.
* −1..−1000: system messages.
* −1001..: editor system messages.
* −2001..: editor menus.
* −3001..: editor messages.
* −4001..: test messages.
* −5001..: run-time messages.

**Programs section**:

```
w  count P
P * l offset (from the programs base) of program i
program: w length, then plain ASCII Interface source (not tokenised)
```

Example: Default program 1 starts `"SIze   SW,SH;\nINk 0,0,0; GB 0,0,SX,SY;…"`.

`Resource Bank n` stores n in `IDia_BankPuzzle`; 0 means use the default. `Resource Screen Open n,w,h,flash` uses the graphics palette and mode.

#### 4.2 Basic instructions (`+Lib.s:14292-14660`)

| Instruction | Notes |
|---|---|
| `Dialog Open ch,prog$\|n[,nvar[,buffer]]` | defaults nvar=16, buffer=1024 (must be > 256). n < 1024 means program n of the resource; otherwise it is a string, which is copied. `Dia_OpenChannel` (`+Lib.s:19860`) does a pre-pass that validates syntax, collects `LA` labels and `UI` user-instruction definitions, and allocates the channel. |
| `=Dialog Run(ch[,label[,x,y]])` | `Dia_RunProgram` `+Lib.s:20506`. If the program executed `RU`, waits and returns `Dia_Return` (the button result); otherwise returns 0 and the zones stay active. |
| `=Dialog(ch)` | last return value (−1 if not run), cleared after reading (`Dia_GetReturn` `+Lib.s:20949`) |
| `Dialog Close [ch]`, `Dialog Clr ch`, `Dialog Freeze/Unfreeze [ch]`, `Dialog Update ch,zone[,a[,b[,c]]]` | |
| `=Vdialog(ch,n)` / `Vdialog(ch,n)=v`, `Vdialog$` | variable array, index 0..nvar (`Dia_GetVariable` `+Lib.s:20764`) |
| `=Rdialog(ch,zone[,k])`, `Rdialog$` | value of zone (k-th zone with that number). Buttons, lists and sliders return the position; Edit returns a string; Digit returns a number (`Dia_GetValue` `+Lib.s:20814`). |
| `=Zdialog(ch,x,y)` | zone number under (x,y), or −1 |
| `=Edialog` | position of the last error inside the Interface program (`IDia_Error`) |
| `=Dialog Box(prog[,v[,v$[,x,y]]])` | opens a temporary channel (≥ 65536) with 16 variables and a 1024 buffer. VA0 = v, VA1 = v$, VA2 = 0. Runs, closes, returns the RU result (`Dia_RunQuick` `+Lib.s:20407`). |

Error codes (`+Equ.s:1123-1134`) are added to `IDia_Errors` = 119 to give the AMOS error number:
1 Syntax, 2 OutOfMem, 3 Label already defined, 4 Label not defined, 5 Channel already defined, 6 Channel not defined, 7 Screen, 8 Var not defined, 9 Illegal function call, 10 Type mismatch, 11 Buffer too small, 12 Wrong number of parameters.

#### 4.3 The Interface interpreter (`+Lib.s:21048-24860`)

**Lexer `Dia_Chr`** (`+Lib.s:24660-24800`): returns the next significant byte with CCR flags taken from a 128-entry table. Bytes ≥ 128 and table entries with bit 7 set are **skipped**. Skipped characters include:
* space and control chars
* all lower-case letters (so "SIze" ≡ "SI")
* `( ) . { } ~ \x7f`
* and **'|'** — so the documented '|' OR operator can never be reached.

The classes are:

| class | chars | flags |
|---|---|---|
| end | NUL | Z |
| letter | A–Z | C |
| operator | `! " # & ' * + - / < = > ? @ [ \ ] ^ _` | C+V |
| number | 0–9 `$ %` | N+C |
| separator | `, : ; \`` | none |

**Program structure**: a sequence of `XX params;` where XX is two upper-case letters. Parameters are separated by ','. '[' … ']' delimit blocks (BU draw/change routines, IF bodies, UI bodies, labels). A ']' at top level ends the current routine. EX ends the program.

**Main loop `Dia_Loop`** (`+Lib.s:21229`):
1. Read 2 letters and search `Dia_Instr` (the **first match wins**).
2. If NParam is 0: consume one separator.
3. If NParam is 1..6: evaluate that many RPN expressions into d2..d7.
4. If NParam is ≥ 7: the handler parses its own parameters.
5. Unknown names are looked up as user instructions (`UI`). Up to 10 levels of nesting are allowed, and P1..P9 read their parameters.

Instruction table (`Dia_Instr` `+Lib.s:21048-21099`, `Dia_NParam` `+Lib.s:21106-21155`, jumps `+Lib.s:21258-21313`):

| # | code | params | meaning |
|---|---|---|---|
| 0 | EX | 0 | exit |
| 1 | UN | 3 | UNpack x,y,image (resource image + PU offset) |
| 2 | LI | 4 | LIne x,y,image,x2 (3-piece horizontal line from images i,i+1,i+2; x snapped to 8) |
| 3 | BO | 5 | BOx x,y,image,x2,y2 (9-piece box from images i..i+8) |
| 4 | SI | 2 | SIze sx,sy (sx rounded up to a multiple of 16) |
| 5 | BA | 2 | BAse x,y (x snapped to a multiple of 16) |
| 6 | PU | 1 | PUzzle base image index offset |
| 7 | SV | 2 | SetVar n,value |
| 8 | IL | 0 | "Illegal" (executes a 68000 `illegal`, debug) — **shadows #33** |
| 9 | PR | 4 | PRint x,y,text,ink |
| 10 | PO | 5 | Print Outline x,y,text,outlineink,ink |
| 11 | IN | 3 | INk a,b,c |
| 12 | SF | 2 | SetFont font,style |
| 13 | SW | 1 | Set Writing mode |
| 14 | SL | 1 | Set Line pattern |
| 15 | SP | 2 | Set Pattern p,outline |
| 16 | BU | 8 | BUtton z,x,y,sx,sy,pos,min,max;[draw][change] |
| 17 | JP | 1 | JumP label |
| 18 | RU | 2 | RUn timer,flags (wait for input; flags bit0 = clear keys, bit1 = clear clicks) |
| 19 | BR | 1 | Button Return value (stored as result) |
| 20 | ED | 8 | EDit z,x,y,sx,maxlen,'default',paper,pen |
| 21 | JS | 1 | Jump Subroutine label |
| 22 | RT | 0 | ReTurn |
| 23 | BC | 2 | Button Change number,position |
| 24 | KY | 2 | KeY shortcut code,shift for the last zone |
| 25 | SA | 1 | SAve background block |
| 26 | DI | 8 | DIgit z,x,y,sx,value,flag,paper,pen |
| 27 | BL | 1 | block (internal) |
| 28 | LA | 1 | LAbel n (pre-pass) |
| 29 | BQ | 0 | Button Quit |
| 30 | HS | 9 | Horizontal Slider z,x,y,sx,sy,pos,trigger,total,step;[change] |
| 31 | VS | 9 | Vertical Slider |
| 32 | AL | 10 | Active List z,x,y,sx,sy,array,first,flag,paper,pen;[change] |
| 33 | IL | 10 | Inactive List (unreachable, see #8) |
| 34 | ZC | 2 | Zone Change |
| 35 | UI | – | User Instruction definition `UI XX,nparams;[ … ]` (pre-pass only) |
| 36 | GB | 4 | Graphic Box (filled) x1,y1,x2,y2 |
| 37 | GS | 4 | Graphic Square (outline) |
| 38 | GL | 4 | Graphic Line |
| 39 | IF | 1 | IF expr [block] |
| 40 | SZ | 1 | Set Zone variable |
| 41 | XY | 4 | set XA,YA,XB,YB |
| 42 | NW | 0 | NoWait (button) |
| 43 | VT | 4 | Vertical Text |
| 44 | VL | 4 | Vertical Line x,y,image,y2 |
| 45 | HT | 10 | HyperText z,x,y,sx,sy,text,pos,buffer,paper,pen;[change] |
| 46 | CA | 1 | CAll machine code address |
| 47 | SM | 0 | Screen Move (drag) |
| 48 | GE | 4 | Graphic Ellipse |
| 49 | GP | 3 | Graphic Plot |
| 50 | SS | 8 | Set Slider (colours) |

All drawing coordinates are relative to BA (BaseX/BaseY). Drawing commands update XA/YA/XB/YB (the last bounding box).

**Expressions (`Dia_Evalue`, `+Lib.s:22719-23250`)** use **RPN** on a long stack (a3), with d7 as the depth. An expression ends at a separator and must leave exactly 1 value.
* Numbers: decimal, `$hex`, `%bin`.
* Strings: `'…'` or `"…"`, stored as `[0][len][chars]`.
* Operators and functions are found through `+Dialog_Funcs.bin`.

**`bin/+Dialog_Funcs.bin` (158 bytes)**:

```
bytes 0..31   : index by (firstChar - 64) -> offset (from byte 32) of that letter's list; 0 = none
bytes 32..    : lists of words  (hi byte = function# *4 ; lo byte = second char), 0-terminated
offset 0 list = single-char operators (looked up with the operator char itself)
```

Decoded content (function index → code):

| list | entries |
|---|---|
| operators | `+`4 `-`5 `*`6 `/`7 `"`14 `'`15 `#`22 `!`23 `=`40 `\`41 `<`42 `>`43 `&`44 `\|`45 |
| A | AR 29, AS 49 |
| B | BX 0, BY 1, BP 21 |
| C | CX 16 |
| M | ME 9 (ME 10 duplicate), MI 19, MA 20, MZ 24 |
| N | NE 8 |
| P | P1..P9 31..39 |
| S | SX 2, SY 3, SW 17, SH 18 |
| T | TW 11, TH 12, TL 48 |
| V | VA 13 |
| X | XA 25, XB 27 |
| Y | YA 26, YB 28 |
| Z | ZP 30, ZV 46, ZN 47 |

Function semantics (`+Lib.s:22760-23250`):
* BX/BY: base. SX/SY: size. SW/SH: screen width/height.
* `+ - * /`: 32-bit add and subtract; `*` and `/` are 16-bit `muls`/`divs`.
* NE: negate.
* `= \ < >`: comparisons giving −1 or 0. `&`: AND. `|` (unreachable): OR.
* ME: message n (from the resource) → string.
* TW: text width. TH: font height. TL: string length.
* CX: (SX − TW)/2, clamped ≥ 0.
* VA: `n VA` → variable n.
* `#`: number → string. `!`: concatenate. MZ: `addr maxlen MZ` → string from zero-terminated memory.
* MI/MA: min/max.
* BP: button position. ZP: zone position. ZV: zone variable. ZN: zone number.
* XA/YA/XB/YB: last box.
* AR: `array index AR` (AMOS 1-D integer array). AS: array size.
* P1..P9: user-instruction parameters.

Example: `"SX16-"` means SX−16, and `"5VA1-"` means VA[5]−1.

The pre-pass (`Dia_OpenChannel`) builds a label table in the buffer:
* `LA n;`: long n, word offset.
* `UI XX,np;[`: word name, long?/word np, word offset.
* Terminated by −1.
* Zones (buttons, edits, lists, sliders, texts, keys) are allocated as records in the channel buffer. Each record has a word length and a word marker (`Dia_BtMark`, `Dia_EdMark`, `"Ky"`, `"Bl"` …). The layouts are in `+Equ.s:1015-1115`.

Channel structure: `+Equ.s:966-1013`. Variables `Dia_Vars` follow it (nvar+1 longs).

#### 4.4 Implementation quirks to keep or decide on

1. "IL" always resolves to #8 (Illegal), so Inactive List is unreachable. The `|` operator is unreachable because the lexer skips '|'.
2. `Dia_FDiv` uses `divs` (16-bit quotient), and `Dia_FMul` uses `muls` (16×16).
3. Lower-case letters are invisible to the lexer, so "BUtton" ≡ "BU". Only upper-case letters form names.
