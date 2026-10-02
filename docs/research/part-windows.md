# AMOS Pro text window system (+W.s / +Equ.s)

All line numbers are for `AMOS-Professional-365/`. Files are ISO-8859, so use `command grep -a`.

## 1. Entry vector (WiCall)

`WiInit` (+W.s:13188) installs `WiIn` (+W.s:13193) as `T_WiVect`. The interpreter calls it with `WiCall n` (+Equ.s:740), which expands to `move.l T_WiVect(a5),a0 / jsr n*4(a0)`. Every entry is a `bra`. Indices are defined at +Equ.s:719-738.

| # | Equ name | Routine (+W.s) | Purpose |
|---|---|---|---|
| 0 | ChrOut | WOutC 15311 | Print one char (d1). Wraps it in cursor off/on. |
| 1 | Print | WPrint 15421 | Print a 0-terminated string (a1). |
| 2 | Centre | WCentre 15497 | Centre a string on the current line. |
| 3 | WindOp | WOpen 13602 | Wind Open. |
| 4 | Locate | WLocate 15264 | Locate x,y. EntNul (omitted) keeps the current value. |
| 5 | QWindow | WQWind 13776 | `Window n`: activate a window (bring it to front). |
| 6 | WinDel | WDel 14037 | Wind Close. |
| 7 | SBord | WSBor 13966 | Border n,paper,pen. |
| 8 | STitle | WSTit 13990 | Title Top/Bottom. |
| 9 | GAdr | WAdr 14017 | Returns the current window number. |
| 10 | MoveWi | WiMove 13825 | Wind Move. |
| 11 | ClsWi | WiCls 14087 | Close every window except 0, then CLW. |
| 12 | SizeWi | WiSize 13895 | Wind Size. |
| 13 | SCurWi | WiSCur 14025 | Set Curs (8-byte shape). |
| 14 | XYCuWi | WiXYCu 15213 | Returns the cursor column (d1) and line (d2). |
| 15 | XGrWi | WiXGr 15226 | X Graphic: `(x+DxI)*8`, or -1 if out of range. |
| 16 | YGrWi | WiYGr 15235 | Y Graphic: `y*8+DyI`. |
| 17 | Print2 | WPrint2 15386 | Print d1 chars from a1. |
| 18 | Print3 | WPrint3 15330 | Print with a horizontal skip and a clip window (used by the editor). |
| 19 | SXSYCuWi | WiXYWi 15201 | Returns WiSys, Tx and Ty. |

Error codes returned in d0 (+W.s:15779-15797):

- 1: out of memory
- 10: window not open
- 11: window already open
- 12: window too small
- 13: window too big
- 14: font parameter is not 0
- 16: illegal parameter (PErr7)
- 18: window 0 can't be modified
- 19: window has no border

## 2. Window structure (+Equ.s:656-714)

Offsets were computed by hand from the equ chain. Total size `WiLong` = 326.

| Off | Field | Meaning |
|---|---|---|
| 0 | WiPrev.l | Window in FRONT. 0 means this is the current/front window. |
| 4 | WiNext.l | Window behind. `EcWindow(screen)` points to the current (front) window. |
| 8 | WiFont.l | 2048-byte 8x8 font (always `T_JeuDefo`). |
| 12 | WiAdhg.l | Byte offset (in a plane) of the top-left of the active area. Set to the inner or outer area by WiInt/WiExt. |
| 16 | WiAdhgR.l | Byte offset of the outer (real) area. |
| 20 | WiAdhgI.l | Byte offset of the inner area. |
| 24 | WiAdCur.l | Byte offset of the cursor cell. |
| 28 | WiColor 6×l | Per plane (plane 0 first), a pointer to the fast glyph routine: CZero, CNorm, CInv, CUn or CNul. |
| 52 | WiColFl 6×(w,w) | Per plane: paper mask word, then pen mask word ($0000 or $FFFF). |
| 76 | WiX.w | **Reverse column counter**: `WiX = Tx - column`, so the column is `Tx - WiX`. |
| 78 | WiY.w | Cursor line. |
| 80/82 | WiTx/WiTy | Active width in chars and height in lines. |
| 84 | WiTyCar | Char height. Always 8 (13622). |
| 86 | WiTLigne | Bytes per text line (`EcTLigne*8`). |
| 88/90 | WiTxR/WiTyR | Outer size in chars. |
| 92/94 | WiDxI/WiDyI | Inner X in bytes and Y in pixels. |
| 96/98 | WiTxI/WiTyI | Inner size in chars. |
| 100/102 | WiDxR/WiDyR | Outer X in bytes and Y in pixels. |
| 104/106 | WiFxR/WiFyR | Outer right edge (bytes) and bottom edge (pixels), both exclusive. |
| 108 | WiTyP | Outer height in pixels. |
| 110/114/118 | WiDBuf.l/WiTBuf.l/WiTxBuf.w | Wind Save background buffer. |
| 120 | WiPaper.w | Paper colour. |
| 122 | WiPen.w | Pen colour. |
| 124 | WiBorder.w | Unused. Border style is held in WiBord. |
| 126 | WiFlags.w | Bit 15: writing mode is not default (forces the slow path). Bit 1: shade. Bit 2: underline. |
| 128 | WiGraph.w | Nonzero: codes 0-31 are drawn as glyphs instead of executed. |
| 130 | WiNPlan.w | Number of planes minus 1. |
| 132 | WiNumber.w | Window number. |
| 134 | WiSys.w | See the bit layout below. |
| 136 | WiEsc.w | Escape state: 0 = none, 2 = waiting for the letter, 1 = waiting for the parameter. |
| 138 | WiEscPar.w | The escape letter. |
| 140 | WiTab.w | Tab size. Default 4. |
| 142 | WiBord.w | Border style 0-16. |
| 144/146 | WiBorPap/WiBorPen | Border paper and pen. |
| 148/150 | WiMx/WiMy | Memorised X and Y. |
| 152/154 | WiZoDx/WiZoDy | Zone$ start position. |
| 156 | WiCuDraw 8b | Cursor shape. |
| 164 | WiCuCol.w | Cursor colour. |
| 166 | WiTitH 80b | Top title. `WiSAuto` = 166 is the part of the structure saved by autoback. |
| 246 | WiTitB 80b | Bottom title. |

### WiSys bit layout

High byte (the byte at `WiSys`):

- Bit 0: scroll on
- Bit 1: cursor on
- Bit 2: inverse active

Low byte (`WiSys+1`): mask of **disabled** planes, stored in reversed bit order. Bit `(NPlan-1-p)` set means plane p is skipped. This mask is set by ESC J (`Planes`, 14832) and is honoured by glyph drawing, CLW, all scrolls and RazCur.

### Screen-level fields used

- `EcWindow`: the window list.
- `EcWiDec`: Wind Save enabled. Set to -1 by `InWindsave` (+Lib.s:13072).
- `EcCurS` (8×6 bytes): saved pixels under the cursor.
- `EcCurrent[6]`: the bitmap being drawn to.
- `EcTLigne`: bytes per row.
- `EcNbCol`: limit for pen and paper values.
- `EcAuto`: autoback mode.

## 3. Wind Open geometry (WOpen 13602, WiAdr 13717)

Inputs:

- d1: window number
- d2: X in pixels
- d3: Y in pixels
- d4: TX in chars
- d5: TY in lines
- d6: bit 0 = CLW
- d7: border (0-16, more than 16 gives error 16)
- a1: font (must be 0, otherwise error 14)

Calculation:

- `DxR = (X>>4)<<1` bytes, so X snaps **down to 16 pixels**. With a border, DxR gets **+1 byte** (13628-13634, and the same in WiMove 13852-13857), so the bordered outer box starts 8 px right of the 16-px boundary.
- TX is made even (`and #$FFFE`). It must be > 0 and `DxR+TX <= EcTLigne`, otherwise error 13.
- `TyP = TY*8`. The window must fit inside the plane (13741-13743).
- Inner area equals outer. With a border: `DxI = DxR+1`, `DyI = DyR+8`, `TxI = TX-2`, `TyI = TY-2`. If either inner size is ≤ 0, error 12.
- Y is arbitrary pixel precision. X is byte (8 px) precision.

Defaults:

- If another window exists, the new window inherits Paper, Pen, CuCol, BorPap, BorPen and Tab from the current window (13644-13654).
- Otherwise: Paper=1, Pen=2, CuCol=3, Tab=4, BorPap=1, BorPen=2.
- On a 1-plane screen: Paper=0, Pen=1, CuCol=1, BorPap=0, BorPen=1 (13656-13668).

Scroll is on. Writing is 0. If a current window exists, its cursor is erased and its contents stored (WiStore, only if Wind Save is on).

Order of operations: draw the border (no titles), switch to the inner area, CLW if d6 bit 0, Home, cursor = `DefCurs` and cursor on, then link the window in front.

Window 0 is created by Screen Open (+W.s:3056-3068): full screen, `TX = (EcTx>>4)<<1`, `TY = EcTy>>3`, CLW, no border. EcInkA/B and the other graphic inks come from the window's pen and paper (3073+).

### Window stacking and Wind Save

- `WQWind` (13776) unlinks the window and puts it at the head. Before that, it stores the old front window's pixels (WiStore 13270) and restores the new front window's pixels from its buffer (WiEff 13367, clipped against windows in front by WiClip 13482). It then frees that buffer.
- WiStore copies the outer rectangle: `TxR` bytes × `TyP` rows × planes. All of this happens only when `EcWiDec != 0`. Without Wind Save, windows behind are never repainted.
- `WDel` (14037):
  1. CClw: if bordered, redraw the border as style 16 (spaces) using BorPap = Paper, then CLW.
  2. Free the window.
  3. Restore every remaining window from its buffer.
  4. Window 0 can't be closed (error 18).
- `WiMove` and `WiSize` store, repaint the windows behind, recompute geometry (falling back to the old geometry on error), then repaint and restore contents. WiSize redraws the border and does CLW. It restores the old content clipped to the new size (WiEff2).

## 4. Glyph rendering (COut, 15573)

`d1 &= 255`.

- If an escape is in progress, go to Esc.
- If `d1 < 32` and `WiGraph == 0`, go to the control table.
- Otherwise draw glyph `font + d1*8` (8 rows, 1 bpp, MSB = leftmost pixel) at byte `WiAdCur` in **every** plane 0..WiNPlan of `EcCurrent`, with row stride `EcTLigne`. A character always occupies exactly one byte column × 8 rows.

### Fast path (WiFlags == 0)

Each plane uses the routine chosen by `AdColor` (+W.s:2394), via the table `TAdCol` (+W.s:9777). The index is `(paperbit<<1)|penbit` for that plane:

| Index | Routine | Writes |
|---|---|---|
| 0 | CZero | 0 |
| 1 | CNorm | glyph |
| 2 | CInv | ~glyph |
| 3 | CUn | $FF |
| plane disabled | CNul | nothing |

The net effect is **replace**: pixel = glyph ? Pen : Paper.

### Slow path (YaFlag 15680, any of Writing ≠ 0, Shade, Under)

For each enabled plane, with `P`/`Q` = that plane's paper/pen mask bytes, and for each row r:

```
g = glyph[r] & shade        ; shade = $FF, or $AA/$55 alternating by row when Shade is on
src = WGet(~g & P, g & Q)   ; Writing bits 3-4: 0 → (~g&P)|(g&Q); 1 → ~g&P ("paper only"); 2 → g&Q ("pen only")
dest = WMod(dest, src)      ; Writing bits 0-2: 0 replace, 1 OR, 2 XOR, 3 AND, 4 no-op
```

- `WMod1/2/3` and `WGet1/2` are self-modified by `Writing` (13223). The source tables are `WrtTab` and `GetTab` at 13256-13264.
- **Underline**: rows 0-6 come from the glyph. Row 7 is replaced by `Q & shade`, combined with WMod. The underline is pen-only, so its background is colour 0, not paper (15715-15739).
- Shade is a 16-bit mask $AAAA, rotated right by 1 per row.

### After the glyph

`WiAdCur += 1` and `WiX -= 1`. At the end of the line, the column goes to 0 and the line advances. Past the last line: if scroll is on, ScHaut (scroll the whole window up by one line, clear the last line); otherwise wrap to line 0 (15654-15673).

### Pen, Paper and Inverse

- **Inverse** (ESC I) swaps the WiPen and WiPaper values and sets WiSys bit 2 (14784).
- **Pen** or **Paper** while inverse is active clears the inverse flag and un-swaps the other colour first (14804-14826).
- `AdColor` recomputes WiColor and WiColFl after every change.

### Cursor (13530-13589)

- If cursor is on (WiSys bit 1): for each plane 0..NPlan, save 8 bytes to `EcCurS`. Then OR the shape where CuCol has that plane's bit set; otherwise AND with NOT shape. In other words, shape pixels become CuCol and other pixels are untouched.
- EffCur restores the saved bytes.
- Every Print or ChrOut is bracketed by EffCur and AffCur.
- `DefCurs` (16661): rows 6 and 7 are $FF and the rest are 0, so the cursor is an underline bar two rows high.

## 5. Control codes 0-31 (CCont table, 16497-16528)

`Rien` means ignored.

| Code | Routine | Effect |
|---|---|---|
| 0-6 | Rien | — |
| 7 | ClEol 14377 | Clear from the cursor to the end of the line, in paper. The cursor does not move. |
| 8 | CLeft 14848 | Cursor left, no erase. At column 0: go to the last column, then CUp. |
| 9 | Tab 14929 | Move to the next multiple of WiTab. No move if the result would be ≥ Tx, or if Tab is 0. |
| 10 | CDown 14893 | Line+1. At the bottom: if scroll is on, ScHaut (scroll up); otherwise wrap to line 0. The column is kept. |
| 11 | Rien | — |
| 12 | Home 15192 | Column 0, line 0. |
| 13 | CReturn 14912 | Column 0 only. Print's newline is `13,10` (+Lib.s:5936). |
| 14-15 | Rien | — |
| 16 | ScGLine 14466 | Scroll the cursor line left by 8 px. The right column is filled with paper. |
| 17 | ScGWi 14475 | Scroll the whole inner window left by 8 px. |
| 18 | ScDLine 14528 | Scroll the cursor line right by 8 px. The left column is filled with paper. |
| 19 | ScDWi 14537 | Scroll the whole window right by 8 px. |
| 20 | ScBas 14661 | Insert a line: lines Y..bottom-1 move down by one, then line Y is cleared. |
| 21 | ScBasHaut 14616 | Lines 0..Y-1 move down to 1..Y, then the top line is cleared. |
| 22 | ScHaut 14646 | Lines 1..Y move up to 0..Y-1, then line Y is cleared. When the cursor is on the last line, this is a full scroll up. |
| 23 | ScHautBas 14584 | Delete a line: lines Y+1..bottom move up by one, then the bottom line is cleared. |
| 24 | Home | Same as 12. |
| 25 | Clw 14409 | Clear the inner window to paper, then Home. |
| 26 | ClLine 14420 | Clear the cursor line. The cursor does not move. |
| 27 | EscM 15754 | Start an escape sequence (WiEsc=2). |
| 28 | CRight 14862 | Column+1. At the end of the line: column 0, then CDown. |
| 29 | CLeft | Same as 8. |
| 30 | CUp 14873 | Line-1. At the top: if scroll is on, ScBas at line 0 (insert a line at the top); otherwise wrap to the bottom line. |
| 31 | CDown | Same as 10. |

The clear and scroll operations fill each plane with that plane's paper mask (`WiColFl`) and skip disabled planes.

## 6. ESC sequences: `Chr$(27) + letter + param` (exactly 3 bytes)

How the sequence is parsed (Esc 15759):

1. The letter is stored in WiEscPar.
2. The next byte `c` calls `CEsc[letter-'A']` (table at 16534) with `d1 = c - 48` (`Chr$(48+n)` encodes n).
3. A letter outside A..Z is ignored.

| Seq | Routine | Semantics |
|---|---|---|
| A, F, G, H, L | Rien | — |
| B n | Paper 14804 | Paper = n (n < EcNbCol, otherwise error 16). Basic `Paper n` and `Paper$(n)` send this. |
| C n | Curs 14743 | Cursor off (0) / on (≠0). |
| D n | CurCol 14734 | Cursor colour (`Curs Pen`). |
| E n | Encadre 15094 | `Border$`. **E0** stores the start (column, line). **En** (n≠0) draws a frame around the text from the stored start to the current cursor position, one char outside it, using style row `n&7` of TEncadre (16649). The frame is drawn with glyph mode on, and the cursor is restored afterwards. |
| I n | Inv 14784 | Inverse off/on (swaps pen and paper). |
| J n | Planes 14832 | n is a bitmask of planes to draw. A clear bit disables that plane. |
| K n | ChgCar 14754 | WiGraph = n. Nonzero prints codes 0-31 as glyphs. Note: ESC itself is then drawn rather than parsed, so K is effectively internal. |
| M n | MemoCu 14996 | 0 = memorise X, 1 = restore X, 2 = memorise Y, 3 = restore Y (`Memorize X/Y`, `Remember X/Y`). |
| N c | DecaX 15032 | Relative X. The raw byte c gives an offset of `c-128` (signed). `CMove`/`CMove$` encode `dx+128`. Calls LocaX. |
| O c | DecaY 15040 | Relative Y, `c-128`. |
| P n | Pen 14818 | Pen = n. |
| Q n | RazCur 14337 | Clear n chars from the cursor (capped to the rest of the line) in paper. The cursor does not move (`Cline n`). |
| R n | Repete 14947 | `Repeat$(a$,n)` is sent as `ESC R0 a$ ESC R(48+n)`, with n < 207. R0 starts capture: following bytes go to `T_WiRepBuf` (80 bytes) instead of the screen. When `ESC R n` arrives, the buffer is printed n times. If the buffer overflows, it is flushed once (14947-14991). |
| S n | Shade 14762 | Shade off/on. |
| T n | SetTab 14920 | Tab size (must be < Tx). |
| U n | Under 14773 | Underline off/on. |
| V n | Scroll 14724 | Scrolling off/on. |
| W n | Writing 13223 | `n = mode + 8*sel`. mode: 0 replace, 1 OR, 2 XOR, 3 AND, 4 ignore. sel: 0 normal, 1 paper only, 2 pen only (+Lib.s:13150-13160). |
| X n | LocaX 15250 | Column = n. |
| Y n | LocaY 15256 | Line = n. `At(x,y)` sends `ESC X(48+x) ESC Y(48+y)` (+Lib.s:14017). Out-of-range values give error 16, which aborts the Print. |
| Z n | WiZone 15049 | `Zone$(a$,n)` is sent as `ESC Z0 a$ ESC Z(48+n)`. Z0 stores the start. Zn calls SySetZ for zone n with a screen rectangle: `x1 = (startCol+DxI)*8`, `y1 = startLine*8+DyI`, `x2 = (endCol+DxI)*8+7`, `y2 = (line+1)*8+DyI`. |

Notes:

- `Compte` (15551) counts printable length with each ESC treated as a 3-byte zero-width token. Centre uses it: `column = (Tx-len)/2` (15497).
- Locate (`Loca` 15289): `WiAdCur = y*WiTLigne + x + WiAdhg`. Error 16 if `y ≥ Ty` or `x ≥ Tx`.

## 7. Borders and titles (DesBord 14131, data 16565-16625)

### Style lookup

The border style is looked up as `Brd[WiBord-1]`:

| WiBord | Data block |
|---|---|
| 1 | Bor0 |
| 2 | Bor1 |
| 3 | Bor2 |
| 4 | Bor3 |
| 5 | Bor4 |
| 6 | Bor5 |
| 7-15 | Bor0 |
| 16 | Bor15 (spaces, used for erasing) |

### Data format

Each style is 8 zero-terminated strings, in this order:

1. TopLeft
2. TopRight
3. Top (repeated)
4. Right (repeated vertically)
5. BottomLeft
6. BottomRight
7. Bottom (repeated)
8. Left (repeated vertically)

| Style | Characters |
|---|---|
| Bor0 | 136, 138, 137, 139, 140, 141, 137, 139 (single line) |
| Bor1 | 128, 130, 129, 132, 133, 135, 134, 131 (thick box) |
| Bor2 | 157, 2, 1, 3, 6, 4, 5, 7 |
| Bor3 | 8, 10, 9, 11, 14, 12, 13, 15 |
| Bor4 | 16, 18, 17, 19, 22, 20, 21, 23 (rounded) |
| Bor5 | 24, 26, 25, 158, 30, 28, 29, 31 |

### Drawing

- The border is drawn in the outer area (WiExt) through COut with:
  - scroll off
  - WiGraph = -1 (so codes 0-31 render as glyphs)
  - WiFlags masked to bit 0 (replace mode)
  - Pen/Paper = BorPen/BorPap (SetBord 14300 / SetNorm 14320)
- **Top row**: TL, then TR placed at `Tx - len`, then the Top string fills between them. Then WiTitH is printed starting just after TL and clipped before TR.
- **Right column**: rows 1..TyI.
- **Bottom row**: BL, BR, Bottom, then WiTitB.
- **Left column**: rows 1..TyI.
- Titles are up to 79 chars (SsWti 14009). Title on an unbordered window gives error 19.
- `Border n,paper,pen` (WSBor 13966): n < 16. EntNul (omitted) keeps the old value, and n=0 also keeps the current border style. Then ReBord.

### Encadre (Border$) frames

TEncadre (16650) has 8 bytes per style, in the order TL, Top, TR, Right, BR, Bottom, BL, Left. Rows are indexed by `n&7`:

| Row | Bytes |
|---|---|
| 1 | 136, 137, 138, 139, 141, 137, 140, 139 |
| 2 | 128, 129, 130, 132, 135, 134, 133, 131 |
| 3 | 157, 1, 2, 3, 4, 5, 6, 7 |
| 4 | 8, 9, 10, 11, 12, 13, 14, 15 |
| 5 | 16..23 |
| 6 | 24, 25, 26, 158, 28, 29, 30, 31 |
| 7 | spaces |
| 0 (also 8) | the spaces row before the table |

## 8. Autoback printing (AutoPrt 15450)

When `EcAuto ≠ 0`, Print, Print2, Locate and Centre go through AutoPrt.

- **Double-buffered screen** (`BitDble`):
  1. Snapshot the window state (the first WiSAuto=166 bytes of the struct, plus EcCurS) into `WiAuto`.
  2. TAbk1, then run the print.
  3. Restore the snapshot.
  4. TAbk2, then run the print again (so the other buffer gets identical output).
  5. TAbk3.
- **Single-buffered screen**: TAbk1, print, TAbk4.

The TAbk routines are in the screens section, around 3552-3617.

For the port: render the text to both logic and physic buffers with identical state.

## 9. Default font (`T_JeuDefo`)

`bin/+WFont.bin` is 512 bytes, included at `Def_Font` (+W.s:9799). It holds 64 glyphs × 8 bytes, 1 bpp, rows top to bottom, bit 7 = leftmost pixel.

- Bytes 0-255: chars **0-31**, the box-drawing set used by borders 3-6.
- Bytes 256-511: chars **128-159**.
  - 128-135: thick box
  - 136-146: single-line box and junctions (├ ┤ ┼ etc.)
  - 147-156: arrow and slider gadget glyphs
  - 157-158: extra corner and side glyphs
  - 159: blank
- Char 0 is blank.

How `Wi_MakeFonte` (+W.s:9565, called at startup 9436) builds the 2048-byte font (256 × 8):

1. Open a temporary 16×8, 1-plane screen.
2. For chars **32-127 and 160-255**, render each char with graphics.library `Text()` into the RastPort at (0, baseline 6), using the system default font (Topaz 8). Copy the 8 rows of the first byte of each.
3. Copy `+WFont.bin` over chars 0-31 and 128-159.

For the port, chars 32-127 and 160-255 therefore need Topaz-8 ROM glyph data (ISO-8859-1), cropped to 8×8 with the baseline at row 6. Only the AMOS-specific 64 glyphs are in the repo.

## 10. Rust port notes

- The window grid is byte-aligned: the X origin is a multiple of 8 px, and with a border it is 16-aligned + 8. Char cells are always 8×8. Everything is planar per-plane logic, so emulate it on an indexed (palette-index) buffer:
  - Fast path: `pixel = g ? pen : paper`.
  - Slow path: do the per-bit-plane combine described in section 4. Writing OR/XOR/AND act per plane bit, not on colour indices.
- The cursor is drawn into the bitmap, with the saved pixels restored before every print. It is part of the pixels, not an overlay.
- Plane masks (ESC J) affect clears and scrolls too.
- Wind Save off (the default) means overlapping windows are never repaired.
