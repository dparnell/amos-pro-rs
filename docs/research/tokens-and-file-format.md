# AMOS Professional: program file format, token encoding and token tables

Source tree: `/Users/danielparnell/Documents/Develoment/amos-rs/AMOS-Professional-365`
(all `file:line` references below are relative to it). Every claim here was checked
against hex dumps of the 201 `.AMOS` files in the tree and against the working lister
`scratchpad/detok.py`. The lister's output is **byte-identical** to the 6 reference
`c/*.Asc` listings.

Note: several sources contain Latin-1 bytes. Run `grep`/`awk` with `LC_ALL=C` (and
`grep -a`), or matches are silently lost.

---------------------------------------------------------------------------------
## 1. The `.AMOS` file format

Loader: `Prg_Load` at +Verif.s:4789. Saver: `Prg_Save` at +Verif.s:4964. The header
strings are at +Verif.s:4323-4324 (copies are in +CLib.s:5909 and +APComp.s:11715).

```
offset size  content
0      16    header (ASCII)
16     4     L = length in bytes of the tokenised source (big-endian)
20     L     tokenised lines, stored one after another
20+L   ...   banks: "AmBs" + .w count + banks, or nothing (see 1.3)
```

### 1.1 Header (16 bytes)
* `"AMOS Basic v134 "` (H_1.3): AMOS 1.3 program. Prg_Load compares only the first
  **10** chars, "AMOS Basic" (+Verif.s:4812-4818).
* `"AMOS Pro101v",0,0,0,0` (H_Pro): AMOS Pro program. Prg_Load compares only the
  first **8** chars, "AMOS Pro" (+Verif.s:4821-4826).
* Byte 11 is overwritten when the program is saved: `'V'` means the program was tested
  (verified) and `'v'` means it was not (+Verif.s:4976-4981, `Prg_StModif`).
* Byte 15 of a Pro header holds `Prg_MathFlags` (+Verif.s:4828 and 4973). Double
  precision sets `%10000011` (+Edit.s:4605, `TkKDPre`). For 1.3 headers, byte 15 is
  part of the text.
* Headers seen in the sample files: `AMOS Basic v1.34`, `AMOS Basic V1.34`,
  `AMOS Basic V134 `, `AMOS Basic V1.3 `, `AMOS Pro101v\0\0\0\0`,
  `AMOS Pro101V\0\0\0\x03`, and `AMOS Pro   v1.00`. Accept any header that starts with
  "AMOS Basic" or "AMOS Pro".
* There is no conversion step between 1.3 and Pro programs. Both use the same token
  values, including the extension token offsets.

### 1.2 Tokenised lines
Each line is laid out as:
```
+0  .b  line length in WORDS (header + tokens + terminating 0 word)
+1  .b  indent: number of leading spaces + 1 (1 = no indent, capped at 127)
+2  ... tokens (16-bit big-endian words, each optionally followed by data; everything is word-aligned)
+n  .w  0  (end-of-line token)
```
* The tokeniser writes the length at +Edit.s:14686-14699 and computes the indent at
  +Edit.s:14235-14246.
* The detokeniser prints `indent-1` spaces (+Edit.s:14757-14765).
* The next line starts at `line + 2*len`. A length byte of 0 ends the program in
  memory. On disk the program simply ends after `L` bytes: Prg_Save writes up to, but
  not including, the first zero line (+Verif.s:4985-4990).
* A line is at most 510 bytes (+Edit.s:14693).

#### Procedure line (token `$0376`)
Layout (L = start of the line):
```
L+0  .b len   L+1 .b indent
L+2  .w $0376 (Procedure)
L+4  .l size  distance from L+8 to the start of the "End Proc" line
              (End Proc line = L+8+size; L+14+size = line after End Proc). 0 in never-verified programs.
L+8  .w seed  (used by the encryption)
L+10 .b flags bit7 = folded/closed in editor, bit6 = locked (cannot be opened),
              bit5 = encrypted ("coded"), bit4 = machine-code / compiled procedure
L+11 .b seed2 (encryption)
L+12 .w $0006 + variable record = procedure name, then optional "[" params "]"
```
References:
* +Verif.s:1520-1600 (`V1_Procedure`): the verifier reads the name at `10(a6)` with
  a6 = L+2.
* +Verif.s:5041-5100: `Tk_FindN` and `Tk_SizeL`. A closed procedure is
  `14+size` bytes long.
* +Edit.s:8651-8836: the folded and locked bits.

In the 788 procedures found in the sample files, `End Proc` starts at exactly
`L+8+size`.

**Machine-code procedures** (flags `$D0`): the proc line is followed by a fake line,
then raw code, then a 3-word `End Proc` line.
* The fake line is `$0301`, `$258C` (`@_apml_@`, _TkML), then a `.w` parameter
  offset.
* The raw code is 68000 code loaded from a hunk file.
* The `End Proc` line is `$0301 $0390 $0000`.

These procedures cannot be listed: skip to `L+8+size`. The layout is built at
+Edit.s:8705-8760 and skipped by the verifier at +Verif.s:1574-1582.

**Encrypted procedures** (bit6 and bit5 both set): decode them with `ProCode`
(+Verif.s:5167; same routine at +CLib.s:5330, called from +Verif.s:1529-1534).
With a6 = L+2:
```
d5 = rol.l(size,8); d5.b = byte[L+11]; d4.w = 1; d3.w = word[L+8]
for each line from the line after Procedure, while (line+4) != L+18+size:
    for each word w from line+4 to line_end:      (header + first token NOT coded)
        w ^= d5.w; d5.w += d4.w; d4.w += d3.w; d5 = ror.l(d5,1)
flags ^= $20
```
The key stream continues across lines, and the loop also covers the `End Proc` line's
terminator word. There are no encrypted samples in the tree, so this is implemented
in detok.py (`procode`) but untested.

### 1.3 Banks after the program
The bank routines are in +Lib.s.
* Hunk ids are listed in `NHunk` (+Lib.s:3789-3794): 1 = "AmBk", 2 = "AmSp",
  3 = "AmBs", 4 = "AmIc".
* Saving: `Bnk.SaveAll` +Lib.s:3833, `Bnk.SaveA0` +Lib.s:3894.
* Loading: `Bnk.Load` +Lib.s:4025.

```
"AmBs"  .w number_of_banks   then number_of_banks bank records:

"AmBk" .w bank_number  .w 0=chip / 1=fast
       .l length  (bit31 = DATA bank (else work), bits 28-30 masked off on load;
                   length INCLUDES the 8 byte name)
       8 bytes name ("Music   ", "Samples ", "Pac.Pic.", "Resource", "Asm", "Datas", ...)
       length-8 bytes of data
"AmSp" (sprite bank = bank 1) / "AmIc" (icon bank = bank 2)
       .w count
       count x { .w width_in_words .w height .w depth(planes) .w hot_x .w hot_y
                 width*height*depth*2 bytes planar data }   (an empty slot = 10 zero bytes)
                 (bits 14-15 of hot_x are flags, masked with $3FFF on load)
       32 x .w palette (64 bytes)
```
If a program has no banks, the file still ends with `"AmBs" 00 00`
(`Bnk.SaveVide`, +Lib.s:3873; `Prog_Finish`, +APComp.s:11727).

The same records, without the `AmBs` wrapper, make up a single-bank `.Abk` file.
`Load` accepts both forms (+Lib.s:4040-4056).

Verified with `parse_banks()`: in all 201 sample files, the bank records consume the
file exactly to its end. Hex example (`AMOS/Editor_Config.AMOS`):
```
00000000: 414d 4f53 2050 726f 3130 3176 0000 0000  AMOS Pro101v....
00000010: 0000 3426 2701 0652 0046 207e 7e7e ...   L=$3426, line len=$27 words, indent 1, ' token
00003432: 0000 0301 0390 0000 416d 4273 0001 416d  End Proc line | AmBs, 1 bank | AmBk
00003442: 426b 0010 0001 8000 0ee0 5265 736f 7572  bank 16, fast, DATA, len $EE0 | "Resource"
```
Sprite bank example (`Productivity1/Quatro.AMOS`): `AmBs 0005 AmSp 0014 | 0001 0010 0004 0000 0000 ...`
(20 sprites; the first is 1 word wide, 16 high and 4 planes deep).

---------------------------------------------------------------------------------
## 2. Token encoding

The equates for the special tokens are at +Equ.s:1994-2124 (`_TkXxx`). A token value
is the **byte offset of its entry from the start of the token table** `C_Tk`
(+Lib.s:48). The interpreter finds the entry with `lea 4(table,token),a0`
(+Edit.s:14824).

Operator tokens are **negative**: they are offsets back from `Dtk_OpFin`
(+Edit.s:15183-15221), as shown in the operator table below.

### Token values that carry inline data
The skip sizes come from `TInst` (+Edit.s:15104) and the tokeniser (+Edit.s:14555-14620).

| value | name | data following the token word |
|---|---|---|
| `$0000` | end of line | – |
| `$0006` _TkVar | variable (also proc call / label ref before verification) | `.w` var offset (0 until verified), `.b` N name length (even), `.b` flags, N bytes name (lower-case, 0-padded) |
| `$000C` _TkLab | label definition `NAME:`, or a line number at the start of the line | same as variable |
| `$0012` _TkPro | procedure call (set by verifier, +Verif.s:3276) | same as variable |
| `$0018` _TkLGo | label reference after Goto/Gosub/Then/Else... (+Verif.s:3386, +Edit.s:14633) | same as variable |
| `$001E` _TkBin | `%binary` | `.l` |
| `$0026` _TkCh1 | `"string"` | `.w` len, len bytes, pad to even |
| `$002E` _TkCh2 | `'string'` | same |
| `$0036` _TkHex | `$hex` | `.l` |
| `$003E` _TkEnt | decimal integer | `.l` (signed) |
| `$0046` _TkFl | single-precision float | `.l` in **Motorola FFP** format |
| `$004E` _TkExt | extension instruction | `.b` extension number (1..26), `.b` param count (0 from tokeniser, set by verifier: −1 for AP20 libraries, else the count; +Verif.s:402-435), `.w` offset in that extension's table |
| `$064A` _TkRem1 | `Rem` | `.w` len, text (padded to even with a SPACE, +Edit.s:14674) |
| `$0652` _TkRem2 | `'` comment | same |
| `$2B6A` _TkDFl | double-precision float | 8 bytes IEEE double |
| `$023C` For, `$0250` Repeat, `$0268` While, `$027E` Do, `$02BE` If, `$02D0` Else, `$25A4` Else If, `$0404` Data | | `.w` (0, filled in by the verifier with a jump offset) |
| `$029E` Exit, `$0290` Exit If, `$0316` On | | `.l` |
| `$0376` Procedure | | 8 bytes (see 1.2) |
| `$2A40` Equ, `$2A4A` Lvo, `$2A54` Struc, `$2A64` Struc$ | | 6 bytes |

All other tokens are bare words.

Variable flags byte (from the tokeniser at +Edit.s:14334-14353 and the verifier):
* bits 0-1: type (0 = integer, 1 = float `#`, 2 = string `$`).
* bit 3: Def Fn name (+Verif.s:877).
* bit 6: array (+Verif.s:838).
* bit 7: procedure name or call (+Verif.s:1550, +Verif.s:3277).

Observed values: `00 01 02 08 40 41 42 80 82`. The name length byte is even, because
the name is zero-padded (+Edit.s:14343-14350).

FFP format (confirmed from the sample data, e.g. `0.1` = `$CCCCCD3D`, `15.5` = `$F8000044`):
```
bits 31..8 = mantissa (normalised, top bit set), bit 7 = sign, bits 6..0 = exponent+64
value = mant/2^24 * 2^(exp-64)          ($00000000 = 0.0)
```
The tokeniser calls `AscToFloat`, which is `a2ffp` (+ILib.s:7110, +Lib.s:25911).
Conversion routines `FFP2Ieee` and `Ieee2FFP` are at +Lib.s:25787-25801.

### Operators (negative tokens)
Each entry in `Dtk_Operateurs` (+Edit.s:15183) is a `bra` instruction (4 bytes)
followed by the name and parameter bytes, padded to even. The token is the entry
start minus `Dtk_OpFin`:

```
FF3E " xor "  FF4C " or "  FF58 " and "  FF66 "<>"  FF70 "><"  FF7A "<="
FF84 "=<"     FF8E ">="    FF98 "=>"     FFA2 "="   FFAC "<"   FFB6 ">"
FFC0 "+"      FFCA "-"     FFD4 " mod "  FFE2 "*"   FFEC "/"   FFF6 "^"
```
These values match the equates `_TkEg = FFA2`, `_TkM = FFCA` and `_TkPow = FFF6`.

### Extension tokens
`$004E`, then `.b ext#`, `.b npar`, `.w off`. The tokeniser builds them at
+Edit.s:14608-14620 (`TkKtE`) and the detokeniser reads them at +Edit.s:14792-14817.
* `ext#` is the slot in `AdTokens(a5)`. Slot 0 is the main library.
* The slot numbers come from `+Interpreter_Config.s:120-129` and the `ExtNb equ n-1`
  line in each extension:

| ext# | Library | Source | Table label |
|---|---|---|---|
| 1 | AMOSPro_Music.Lib | +Music.s | `C_Tk` (+Music.s:380) |
| 2 | AMOSPro_Compact.Lib | +Compact.s | `C_Tk` (+Compact.s:49) |
| 3 | AMOSPro_Request.Lib | +Request.s | `C_Tk` (+Request.s:51) |
| 4 | AMOSPro_3d.Lib | +3d.s | `TokenTable:`/`ExTk` (+3d.s:265) |
| 5 | AMOSPro_Compiler.Lib | +CompExt.s | `C_Tk` (+CompExt.s:75) |
| 6 | AMOSPro_IOPorts.Lib | +IO_Ports.s | `C_Tk` (+IO_Ports.s:79) |

* `off` is the byte offset of the entry from that extension's own `C_Tk`. Every
  extension table starts with a dummy entry (`dc.w 1,0` / `dc.b $80,-1`), so real
  offsets start at 6.
* If the extension is missing, the editor lists `Extension X`, where X = `'A'+ext#`
  (+Edit.s:14809-14817, `ExtNot` at +Edit.s:15223).

---------------------------------------------------------------------------------
## 3. Instruction token tables

### 3.1 Library header
+Lib.s:25-32 (the same layout is used in every extension):
```
dc.l C_Tk-C_Off, C_Lib-C_Tk, C_Title-C_Lib, C_End-C_Title
dc.w 0 / -1          ; "force copy routine 0" flag for the compiler
dc.b "AP20"          ; new (2.0) format marker; absent in old/1.3 libs (+3d.s has none)
C_Off: one dc.w per library routine (routine length/2, generated by MC macro) ; Lib_Size entries
C_Tk:  token table   (TOKEN_START comment +Lib.s:46 ... TOKEN_END +Lib.s:1667)
C_Lib: routines L0, L1, ... (declared with Lib_Def/Lib_Par/Lib_Int/Lib_Empty; +Equ.s:2147-2175)
```
The `.Lib` binaries in `AMOS/APSystem/` are AmigaDOS hunk files: `$3F3`, hunk table,
`$3E9`, size, code. In the code hunk, `C_Off` is at offset 22 when "AP20" is present
and at offset 18 when it is not. detok.py `--libs` reads these files directly and
produces exactly the same table as the source parse.

### 3.2 Entry format
The format is documented by Lionet in +Music.s:300-370.
```
        dc.w  INSTR_ROUTINE, FUNC_ROUTINE        ; L_xxx routine numbers (L_Nul / 1 / -1 = none)
        dc.b  "name chars", lastchar+$80, "param list", TERMINATOR
        (even)                                     ; Devpac aligns the next dc.w
```
* **Name:** lower-case. The last character has bit 7 set. Spaces belong to the
  keyword (`"screen open"`). A trailing space marks a word-keyword such as
  `"for"," "+$80`, so the detokeniser prints `For `.
  * A leading `!` marks an overloaded name (`"!mid","$"+$80`). The `!` is never
    printed.
  * A name consisting of the single byte `$80` reuses the name of the nearest
    preceding `!` entry (a variant with a different parameter list).
  * In the editor, a first byte of `$81-$9F` means "go back n bytes" (+Edit.s:14844-14861).
    Nothing in the shipped tables uses it.
* **Param list:** ASCII, terminated by the first byte with bit 7 set (the terminator).
  * First char = kind:
    * `I` instruction
    * `0` function returning an integer
    * `1` function returning a float
    * `2` function returning a string
    * `V` reserved variable (followed by its type digit, e.g. `"V0"`, `"V20"`; usable as
      `X Mouse=...`)
    * `O` operator/punctuation (`,` `;` `#` `(` `)` `[` `]` and the operator table)
    * `C` constant token (`C0` int, `C1` float, `C2` string, `C3` double) used by the
      special entries `$1E`-`$46` and `$2B6A`
    * no characters (the terminator follows the name immediately): structural keyword
      with no parameter checking (`for `, `to `, `then `, `else `, `procedure `,
      `rem`, ...).
  * Then the parameter types, separated by `,` (a comma) or `t` (the keyword `To`):
    * `0` integer
    * `1` float/double
    * `2` string
    * `3` integer OR string (distinguished by address, <512 = int)
    * `4` integer OR float: two routines, the float one at instr routine+1; used by
      `sgn`, `abs`, `int`
    * `5` angle (always float radians, even under `Degree`)
  * Examples:
    * `"I0,0t0,0"`: instruction with 4 integer parameters, `a,b To c,d`
    * `"22,0"`: string function `(string,int)`
    * `"I0t0,0,0,0,0"`: `Pack`
    * `"V00,0"`: `Rain(x,y)=...`
* **Terminator:**
  * `-1` ($FF): last (or only) definition.
  * `-2` ($FE): another definition with the same name follows (overload).
  * `-3` ($FD): like −2, but the next variant is the other kind (instruction and
    function). Only used by `!screen` (+Lib.s:563) and `!colour` (+Lib.s:585).
* The table ends with `dc.w 0` (+Lib.s:1665). The walk algorithm (`TklNext`,
  +Edit.s:14727, and +B.s:2350-2358) is:
  skip 4 bytes, skip the name until a byte with bit 7 set, skip the params until a
  byte with bit 7 set, align to even, and stop if the next word is 0.
* The `//$xx$xx$xx$xx` comments in +Lib.s are read by `c/Make_Toktable.Asc`, which
  builds `+Toktab_Verif.Bin` (4 bytes per token for the fast verifier, the "KwiK" table).
  They have nothing to do with token values.

Special entries at the start of the main table (+Lib.s:48-95). Their offsets are the
special token values:
```
0000 dummy | 0006 var | 000C label | 0012 proc | 0018 label-goto
001E C0 bin | 0026 C2 "str" | 002E C2 'str' | 0036 C0 hex | 003E C0 int | 0046 C1 float
004E ext | 0054 ": " | 005C "," | 0064 ";" | 006C "#" | 0074 "(" | 007C ")" | 0084 "[" | 008C "]"
0094 "to " | 009C "not " | ...
```
There are 130 `_Tk*` equates in +Equ.s, including the 3 operator ones. All of them
match the computed offsets, except the two odd values `_TkBcl2 = $355` and
`_TkAMOSPro = $2561`, which are range markers rather than tokens.

Table sizes:
* main: 778 entries
* Music: 64
* Compact: 8
* Request: 4
* 3D: 83
* Compiler: 20
* IOPorts: 45
* operators: 18

The full dump (offset, name, params, terminator, source line) is in
`scratchpad/token_table.txt`.

### 3.3 Binding tokens to implementation routines
`dc.w INSTR, FUNC` hold library routine numbers `L_name`.
* `c/Library_Digest.Asc` numbers these routines sequentially: every
  `Lib_Def`/`Lib_Par`/`Lib_Ext`/`Lib_Empty` gets `L_name set n`, written to
  `+Lib_Labels.s` and `+Lib_Size.s`.
* The `MC` macro builds the `C_Off` size table.

When a library is loaded (+B.s:2360-2440, AP20 format), the loader rewrites each
entry in place:
* The instruction word becomes `-(n+1)*4`: an offset into the jump table placed in
  front of the library.
* The function word becomes a parameter/flag word:
  * bit 6 = has instruction
  * bit 7 = function
  * low bits = number of parameters
  * `L_FFloat`, `L_FMath`, `L_FAngle` and `L_VRes` flags derived from the parameter
    letters

Old (non-AP20) libraries store routine numbers that are relocated differently
(+B.s:2326-2358). The values 0, 1 and negative mean "no routine". +3d.s uses `1` for
"none".

None of this changes token values. The token is always the entry's byte offset in
the source table, so the Rust port can key everything on (ext#, offset).

### 3.4 Tokeniser notes (for the future Rust tokeniser)
`Tokenise` is at +Edit.s:14227. Keyword matching is longest-match, case-insensitive,
over the operator table and then over the main and extension tables. It checks the
"fast" tables per first letter (`AdTTokens`).
* `?` becomes Print, or Print # when a `#` follows (+Edit.s:14424-14434).
* Digits at the start of a line form a line-number label (`_TkLab` with a numeric
  name).
* `ident:` at the start of a line is a label definition.
* After Then/Else, digits become `_TkLGo`.
* A leading `'` is `_TkRem2`.
* Strings use `"` or `'`.
* Numbers are parsed by `ValRout` (+ILib.s:7018). It returns `_TkEnt`, `_TkHex`,
  `_TkBin`, `_TkFl` (FFP) or `_TkDFl`, depending on `MathFlags` bit 7.
* A leading `-` is **not** part of the constant; it is the `-` operator.
* Names are stored in lower case.

---------------------------------------------------------------------------------
## 4. Detokeniser (listing) rules

These follow `Detok` at +Edit.s:14744; an identical copy is at +CLib.s:5436. Case
modes come from `DtkMaj1 = 2` and `DtkMaj2 = 1` (+Editor_Config.s:43-44).
1. Print `indent-1` spaces.
2. For each token:
   * **Tokens <= `$18` (variables and labels):**
     * If the previous token was a variable and the last character is not a space,
       insert a space.
     * Print the name in UPPER case, stopping at the first 0 byte.
     * `$0C` adds `:` unless the name starts with a digit.
     * Other tokens add `#` or `$` according to flags & 3.
   * **`$1E` <= token < `$4E`, or `$2B6A` (constants):**
     * If the previous token was a variable, insert a space.
     * Format the value:
       * `"..."` or `'...'`
       * integers in signed decimal
       * `$HEX` in upper case with no leading zeros
       * `%bin` with no leading zeros
       * floats with 7 significant digits, trailing zeros removed, and `.0` appended
         if there is no `.` or `E` (DtkC8)
   * **Token `$0074` `(`:** remove a preceding space and print `(`. This is why the
     listings show `Exit If(A$="/") or(A$=":")`.
   * **Other tokens (keywords):** look up the entry: main table, extension table, or
     operators for negative tokens. Let `d3` be the first character after the name.
     * If `d3` is not one of `O`, `V` or `0`-`8`, and we are not at the line start and
       the last character is not a space, print a space first.
     * Print the name without `!`:
       * Upper-case the first character.
       * After that, upper-case any character that follows a space. The test starts
         at character 1, so `" xor "` stays lower case while `"screen open"` becomes
         `Screen Open`.
     * Rem and `'`: append the comment text up to the first 0.
     * If `d3 == 'I'`, append a space.
     * Skip the inline data (table in section 2).
3. Lines end with the trailing spaces produced by the rules above. For example,
   `Do ` and `Next ` keep their trailing space; the `.Asc` files contain them too.

---------------------------------------------------------------------------------
## 5. `scratchpad/detok.py`

```
python3 detok.py prog.AMOS                 # list (ISO-8859-1 output; --utf8 for terminals)
python3 detok.py --banks --raw prog.AMOS   # + header/bank summary, + hex of every line
python3 detok.py --libs AMOS/APSystem ...  # use binary .Lib token tables instead of sources
python3 detok.py --dump-table              # dump all token tables
```
* **Building the tables:** a mini Devpac assembler (`assemble_token_table`) handles
  `dc.w`/`dc.b`, string and char expressions such as `"e"+$80` and `$80+"e"`, and
  word alignment. It reads +Lib.s from `C_Tk`, the extensions from `C_Tk`/`TokenTable`,
  and the operators from +Edit.s `Dtk_Operateurs`. The table walk follows +B.s.
* **Results:**
  * All 201 `.AMOS` files in the tree list without errors or unknown tokens.
  * The output for `c/*.AMOS` is byte-identical to the 6 reference `c/*.Asc` files.
  * The source-built tables and the binary `.Lib` tables give identical listings on
    all files.
  * Extension keywords (Sam Play, Track Load, Unpack, Spack, Request On, Serial Open,
    Comp Size, Td ...) all resolve.
* **Limitations:**
  * The float text format is approximated: 7 significant digits, and `E` notation is
    printed as ` E+nn` (FloatToAsc puts a space before E). No sample contains a value
    that needs E notation.
  * Double-precision constants (`$2B6A`) and encrypted procedures are implemented from
    the source but have no samples to test against.
  * Machine-code procedures are listed as `Procedure X` / `' [machine code ...]` /
    `End Proc`.
