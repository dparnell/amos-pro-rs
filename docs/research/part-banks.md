# AMOS Professional: banks, number formatting, menus, dialogs/Interface

Source root: `AMOS-Professional-365/`. All `file:line` references are to that tree. Files are ISO-8859-1. Line numbers come from the copy studied for this document.
Everything here was read from the 68000 source. Each byte layout was also checked against the real files `bin/+AMOSPro_Mouse.abk`, `AMOS/APSystem/*.Abk` and a set of `*.AMOS` programs, using hexdumps and a small Python parser.

---------------------------------------------------------------------------

## 1. BANK SYSTEM

### 1.1 Flag bits (`+Equ.s:1837-1840`)

| bit | name | meaning |
|---|---|---|
| 0 | `Bnk_BitData` | "Data" bank (saved with program, survives `Erase Temp`). Clear = "Work" (temporary) |
| 1 | `Bnk_BitChip` | allocated in chip RAM |
| 2 | `Bnk_BitBob`  | sprite/bob bank ("Sprites ") |
| 3 | `Bnk_BitIcon` | icon bank ("Icons   ") |

### 1.2 Bank list and the in-memory structure

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

### 1.3 Instructions

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

### 1.4 File formats

Magic longs (`NHunk`, `+Lib.s:3791`): index 1 = "AmBk", 2 = "AmSp", 3 = "AmBs", 4 = "AmIc".
All values are big-endian.

#### "AmBk" – normal bank (`Bnk.SaveA0` `+Lib.s:3894-3930`, `Bnk.Load` `+Lib.s:4025-4073`)

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

#### "AmSp" / "AmIc" – sprites / icons (`SB_Bob`/`SB_Icon` `+Lib.s:3932-3990`, `LB_Sprites`/`LB_Icons` `+Lib.s:4093-4180`)

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

#### "AmBs" – multi-bank (`Bnk.SaveAll` `+Lib.s:3833`, `LB_Multiples` `+Lib.s:4183`)

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

#### Sample bank data (bank name "Samples ", `GetSam` `+Music.s:3181-3206`)

```
data+0  w count
data+2  count * l offset (from data+0) of each sample; 0 = undefined
sample: 8 b name, w frequency (Hz), l length, data bytes (8-bit signed)
```

Checked against the Editor_Samples bank: 9 samples, first one at offset 0x26, named "A       ".

#### Packed picture (Pac.Pic.) headers (`+Equ.s:896-928`)

* Screen header: `PsCode.l=$12031990`, `PsTx`, `PsTy`, `PsAWx`, `PsAWy`, `PsAWTx`, `PsAWTy`, `PsAVx`, `PsAVy`, `PsCon0`, `PsNbCol`, `PsNPlan` (all words), then `PsPal` (32 words).
* Bitmap header: `Pkcode.l=$06071963`, `Pkdx.w`, `Pkdy.w`, `Pktx.w` (bytes), `Pkty.w` (char rows), `Pktcar.w` (lines per row), `Pknplan.w`, `PkDatas2.l`, `PkPoint2.l` (24 bytes).
* The packer itself is in `+Compact.s` and `UnPack_Bitmap`.

### 1.5 .AMOS program file (`Prg_Load` `+Verif.s:4789`, `Prg_Save` `+Verif.s:4962`)

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

## 2. FLOATING POINT AND NUMBER⇄TEXT

### 2.1 Formats and libraries

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

### 2.2 Fix state

* `FixFlg.w` / `ExpFlg.w` (`+Equ.s:1388-1389`). At run start: `FixFlg=-1`, `ExpFlg=0` (`+Verif.s:3964-3965`).
* `Fix n` (`InFix`, `+Lib.s:1929-1942`):
  * n ≥ 0: `ExpFlg=0`.
  * n < 0: `ExpFlg=1` and n=−n.
  * Then, if n ≥ 16 (unsigned compare): `FixFlg=-1`, else `FixFlg=n`.
* So: `Fix 16` gives proportional (the default). `Fix -1` gives exponential with 1 decimal. `Fix -16` gives exponential with proportional digits (FixFlg=−1, ExpFlg=1).

### 2.3 Integer printing (Print / Str$ of an integer)

`LongToAsc` (`+Lib.s:25671-25716`) is called with d3=−1 (proportional) and d4=1 (signed) from Print (`+ILib.s:5126`) and Str$ (`FnStrE`, `+Lib.s:1754`).
* Negative: "-" followed by the magnitude. `$80000000` prints as -2147483648.
* Otherwise a leading **space**.
* Then decimal digits with leading zeros suppressed. The last digit is always printed.
* Examples: `Print 5` → " 5", `Str$(-12)` → "-12", `Str$(0)` → " 0".

`LongToDec` (`+Lib.s:25659`, used by Listbank and others) calls the same routine with d4=0, so there is no leading space.

### 2.4 Hex$ / Bin$ (`+Lib.s:14239-14283`, `LongToHex` `+Lib.s:25718`, `LongToBin` `+Lib.s:25754`)

* `Hex$(v)`: "$" followed by upper-case hex digits with leading zeros suppressed (last digit always printed). `Hex$(0)` = "$0". `Hex$(-1)` = "$FFFFFFFF".
* `Hex$(v,n)`: "$" followed by the **low** n digits of the 8-digit value, with leading zeros kept. `Hex$(255,4)` = "$00FF". n=0 gives "$". n>8 behaves as proportional (because skip = 8−n < 0).
* `Bin$` works the same way with "%" and 32 digits.

### 2.5 Single precision float → text (Print, Str$)

Entry point is `Float2Ascii` (`+ILib.s:7676`). It sets d0 = FFP value, d4 = FixFlg with bit 31 cleared (bit 31 clear means "add a leading space"), d5 = ExpFlg, then jumps to `FloatToAsc` (`+Lib.s:25923`). `Str$` uses the same path (`FnStrF` `+Lib.s:1766`).

#### Core primitive `ffp2a(x, buf, prec)` (`+Lib.s:26226-26270`)

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

#### `F2a(x, d4=fix, d5=exp)` (`+Lib.s:25976-26222`), result in `DeFloat(a5)`

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

#### `FloatToAsc` post-processing (`+Lib.s:25923-25972`)

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

### 2.6 Double precision float → text (`Float2AsciiD`, `+ILib.s:7684-7708`)

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

### 2.7 Text → number (Val, Input) — `FnVal` `+ILib.s:6653`, `ValRout` `+ILib.s:7018-7262`

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

### 2.8 Print statement formatting (`+ILib.s:5046-5333`)

* ',' emits TAB (9).
* ';' emits nothing.
* At the end of the statement (no trailing separator), emit CR LF.
* Strings are sent in chunks of 120 bytes.
* `Using` (`using1` `+ILib.s:5177`, `using50` `+ILib.s:5305`):
  * '#' is a digit position; '+' and '-' are sign positions; '.' aligns the decimal point; ';' marks the point without printing it; '^' is an exponent position (fake "E+000" if the number has no exponent).
  * For strings, '~' is a character position.

---------------------------------------------------------------------------

## 3. MENUS

### 3.1 Data structures

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

### 3.2 Instructions

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

### 3.3 Menu item string language (`MnObjet`, `+Lib.s:17435-17650`)

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

## 4. DIALOGS / INTERFACE LANGUAGE

### 4.1 Resource bank ("Resource", normally bank 16)

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

### 4.2 Basic instructions (`+Lib.s:14292-14660`)

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

### 4.3 The Interface interpreter (`+Lib.s:21048-24860`)

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

### 4.4 Implementation quirks to keep or decide on

1. "IL" always resolves to #8 (Illegal), so Inactive List is unreachable. The `|` operator is unreachable because the lexer skips '|'.
2. `Dia_FDiv` uses `divs` (16-bit quotient), and `Dia_FMul` uses `muls` (16×16).
3. Lower-case letters are invisible to the lexer, so "BUtton" ≡ "BU". Only upper-case letters form names.
