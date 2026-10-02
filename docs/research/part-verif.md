# AMOS Professional — the Verification / Test pass (`+Verif.s`)

Source root: `AMOS-Professional-365/`. Line numbers refer to the original files (ISO‑8859‑1).
All structures are big‑endian 68000. `a5` = global data zone (fields defined in `+Equ.s`),
`a6` = program pointer during the test.

---

## 1. Entry points and when they run

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

### 1.1 `PTest` sequence (+Verif.s:73‑200)

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

### 1.2 `SsTest` (+Verif.s:225‑491) — one phase = pass 1 + pass 2

* Clears `ErrRet, Passe, Ver_NBoucles, Ver_PBoucles`; `Ver_Verif` (verif token tables);
  `Reserve_Reloc`; `Reserve_TablA`; `a6 = a3 = Prg_Test`.
* **Pass 1** = the dispatch loop `VerD`/`VerDd`/`VerLoop` (§2.4) walking every line/instruction, checking
  syntax and types, creating variables/labels, writing **relocation records** (§3.3) and **TablA**
  structure records (§6.1).
* End of program (line header word = 0), or `End Proc` in a procedure phase, jumps to `VerX` (+496):
  **Pass 2**: `Passe=1`, emit `Reloc_End`, replay the relocation stream (patch variable offsets, resolve
  labels and procedure calls), then call every TablA entry's `Vta_Jump` (loop/if/goto resolution).
* Delayed error (`ErrRet`, set by `ERetard` +595) is raised at the end; else return d0=0.

### 1.3 State built by the test (what the interpreter relies on)

* Token stream patched in place: variable offsets, label offsets, loop/If/Exit/On/Data distance words,
  Procedure size/varsize fields, token substitutions (overloaded instruction variants, `_TkAd2/_TkAd4`,
  `_TkPro`, `_TkLGo`), extension parameter counts, resolved equates.
* Label table (`LabBas..LabHaut`) in VarBuf, global variable area (`VarGlo`, size `GloLong`).
* `MathFlags` (bit7 double, bit1 math lib, bit0 floats), `Stack_Size`, `Prg_Accessory`, `VerNot1.3`,
  `Ver_SPConst/Ver_DPConst` (precision‑mismatch warning, +Edit.s:8427‑8437).

---

## 2. Tokenised program format as seen by Verif

### 2.1 Lines

```
+0  byte  line length in WORDS (header + tokens + terminator); 0 = end of program
+1  byte  indent (leading spaces + 1, max 127)            (+Edit.s:14236‑14245)
+2  word  token ... token
    word  0  (end‑of‑line terminator)
```
A program ends with a line whose header word is 0. Tokens are word offsets into a library's token table
(main library: offset from `C_Tk` in +Lib.s:46; operators: negative words). `VDLigne(a5)` = address of the
current line header (+Verif.s:244).

### 2.2 Token table entries (+Lib.s:46 ff, `;TOKEN_START`..`;TOKEN_END`)

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

### 2.3 The verification ("KwiK") table and table swapping

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

### 2.4 Statement dispatch (pass 1)

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

### 2.5 Generic instruction / function checking (`VerI`, `VerF`, `VerC`)

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

### 2.6 Expressions (`Ver_Evalue`, +2528‑2578; operands `Ver_Operande`, +2582‑2967)

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

## 3. Variables

### 3.1 Variable token (`_TkVar` = $0006)

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

### 3.2 Variable name table (VNm buffer)

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

### 3.3 Relocation stream (pass 1 → pass 2)

Buffers of `Reloc_Step`=1024 bytes, linked by first long (`New_Reloc` +3731, limit = buffer+1024‑32).
Each record = position delta from previous record (`a3`) then a code byte:
* delta ≤ 254: one byte `delta/2` (bit7 clear);
* 254 < delta ≤ 762: byte 127 (=skip 254) repeated;
* ≤ 65534: `Reloc_Long $84, hi, lo`; larger: `$84,$FF,$FE` repeated.
* Codes (bit7 set): `$80 End, $82 Var, $84 Long, $86 NewBuffer, $88 Proc1, $8A Proc2 (+ raw type byte
  via Out_Reloc), $8C Proc3, $8E Proc4, $90 Debug, $92 Label` (+Verif.s:27‑36).
Decoder (+506‑534): bit7 clear → `a6 += byte*2`; else dispatch `(code&$7F)*2` into a `bra` table.
`Ver_NoReloc` suppresses records (used by `VerMid`, which evaluates its variable twice).

### 3.4 Globals, Shared, Global

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

### 3.5 Arrays, Dim, Def Fn

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

### 3.6 Variable area size

* Main: `GloLong` = phase‑0 `VarLong`. Placed under the label table (+154‑172):
  `LabBas – 6` holds a sentinel (`word $FFFF`, `long 0`), globals occupy `GloLong` bytes below; error 8 if
  that crosses `HiChaine`. `VarGlo = VarLoc = TabBas` = its bottom.
* Procedure: `VarLong` of the phase (params first, then locals) poked into Procedure token +6;
  `CallProc` (+ILib.s:2482) allocates that many bytes below `TabBas` per call.
* VarBuf layout (`ResVarBuf` +4045): `[ChVide word][strings ↑ … HiChaine] … [arrays ↓ TabBas][globals]
  [$FFFF + long][labels LabBas…][0 word][LabHaut=end]`. Default 8 KB for programs, 4 KB for direct mode,
  `Set Buffer n` → n·1024.

---

## 4. Labels, procedures, calls

### 4.1 Label definition (`_TkLab` $000C) and label table

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

### 4.2 Goto/Gosub targets (`V1_GoLabel`, +3362)

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

### 4.3 Procedure header line

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

### 4.4 Procedure calls (`V1_CallProc` +3270, pass 2 +3306‑3355)

A statement that is a variable not followed by `=` or `(` is a procedure call (`V1_IVariable` +3207); also
`Proc name` (class 0F, must become a call else error 20). Pass 1: token → `_TkPro` ($0012), flags |= $80,
records `Reloc_Proc1`, then per bracket argument `Reloc_Proc2` + the argument's type byte, `Reloc_Proc3`
at `]`, or `Reloc_Proc4` if none. Illegal in direct mode (7).
Pass 2: `Proc1` finds the phase‑0 label (error 20 if absent), patches the label offset into the call
token, sets a3 to the definition's parameter list (or 0). `Proc2` checks arity (error 19) and that the
param variable's type (float counted as numeric) equals the argument type (error 40). `Proc3`/`Proc4`
check no parameters remain (19). Runtime `CallProc` reads `LabHaut[offset]` = T.

---

## 5. Loops, tests, structure (TablA)

### 5.1 TablA record (+2293‑2303), chunks of 1024 bytes (`Init_TablA` +2349)

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

### 5.2 Patched fields (placeholders reserved by the tokenizer, +Edit.s:14571‑14615)

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

## 6. Other statements

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

## 7. Double precision & float type selection

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

## 8. Test‑time errors (numbers passed negated to `Prg_JError`; texts +Editor_Config.s:760‑813)

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

## 9. Porting notes

* Verif is a two‑pass, per‑phase compiler front end that **mutates the token stream**; a Rust port should
  model it as `pass1` (walk, check, record fixups + structure records) then `pass2` (apply fixups, resolve
  structures) per main/procedure scope.
* Only two types exist for checking (numeric/string); the int/float distinction only affects storage size
  (double mode) and MathFlags.
* Overloaded instructions are resolved by **rewriting the token** to the matching `-2`‑chained variant;
  a Rust implementation needs the variant chain and param spec strings from +Lib.s.
* Several messages (1, 6, 11‑13, 24, 44) are defined but unreachable; the one‑line‑If inside a structured
  If quirk (error 25) is genuine behaviour of this source.
