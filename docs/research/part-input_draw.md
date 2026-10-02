# Part: Input (mouse/keyboard/joystick) and Drawing primitives

All refs relative to `AMOS-Professional-365/`. NB: the shell `grep` wrapper skips these ISO-8859 files; use `command grep -a`.

---------------------------------------------------------------------
## A. INPUT

### A.1 System ("Sy") jump table
`+W.s:9881-9982` `SyIn:` is a table of `bra` instructions (4 bytes each). Interpreter calls via
`SyCall n` = `move.l T_SyVect(a5),a0 ; jsr n*4(a0)` (`+Equ.s:366-385`; variants SyCalA loads a1, SyCalD loads d1, SyCal2 both). Numbers in `+Equ.s:264-364`.

| # | Equ name | +W.s target | purpose |
|---|---|---|---|
|0 Inkey|ClInky|pop key from buffer|
|1 ClearKey|ClVide|empty key buffer|
|2 Shifts|ClSh|Key Shift|
|3 Instant|ClInst|Key State(n)|
|4 KeyMap|ClKeyM|no-op (`rts`, 12965)|
|5 Joy|ClJoy|joystick|
|6 PutKey|ClPutK|stuff string into buffer|
|7/8 Hide/Show|MHide/MShow|mouse visibility counter|
|9 ChangeM|MChange|Change Mouse n|
|10 XyMou|MXy|X/Y Mouse (hardware)|
|11 XyHard|CXyHard|screen->hardware coords|
|12 XyScr|CXyScr|hardware->screen coords|
|13 MouseKey|MBout|Mouse Key|
|14 SetM|MSetAb|X Mouse=/Y Mouse=|
|15 ScIn|GetSIn|Mouse Screen / screen under point|
|16 XyWin|CXyWi|screen -> text-window char coords|
|17 LimitM|MLimA|Limit Mouse x1,y1 To x2,y2|
|18-22|SyZoHd/SyResZ/SyRazZ/SySetZ/SyMouZ|zones (Hzone, Reserve Zone, Reset Zone, Set Zone, Mouse Zone)|
|23 WaitVbl|WVbl|graphics WaitTOF|
|24-36,39|HsSet...HsPri|hardware sprites|
|26/27|ClFFk/ClGFFk|Key$(n)= / =Key$(n)|
|40-48|TokAMAL...UFrzAMAL|AMAL|
|49-54,57,59-62,68,70,71|Bob* / GetBob / Masque / SpotH / BbColl|bobs|
|63/64|GetCol/SpColl|collisions|
|65/66 SetSync/Synchro|SyncO/Sync|Synchro on/off/step|
|72 MouRel|MRout|Mouse Click (newly pressed)|
|73 LimitMEc|MLimEc|Limit Mouse to screen|
|75/76|HColSet/HColGet|Set Hardcol/=Hardcol|
|78 KeySpeed|TKSpeed|Key Speed|
|79-81|TChanA/TChanM/TPrio|Chanan/Chanmv/Priority|
|83/84|Add_VBL/Rem_VBL|install/remove VBL server|
|85 KeyWaiting|ClKWait|key pending?|
|86 MouScrFront|WMouScrFront|mouse coords in front screen|
|87-93,96-99|memory/routine helpers|
|94 Send_FakeEvent|WSend_FakeEvent| |
|95 Test_Cyclique|WTest_Cyclique|Amiga-A flip handling outside IRQ|
|100|WRequest_OnOff| |

### A.2 VBL interrupt (50 Hz PAL)
`VblInit` `+W.s:10301-10314`: inits flash/shift, sets `T_MouDes` = MouBank (first shape), `T_MouTY` = shape height, `T_MouShow=-1` (hidden), then `Add_VBL` (10317) adds an exec INTB_VERTB server, priority 100.
`VblIn` `+W.s:10345-10460` per-frame order:
1. Interlace bitplane pointer switching (10350-10378): for each interlaced screen, on long/short frame (VPOSR bit 15) add `EcTx/8` (one line) to plane pointers poked in copper.
2. Pending screen swaps (`T_SwapList`, 10381-10405): poke new bitplane pointers (`Screen Swap` takes effect at next VBL).
3. Hardware sprite copper pointer change if `T_HsChange` (10408-10412).
4. `T_VBLCount++`, `T_VBLTimer++` (= `Timer`), `T_EveCpt--` (EVERY counter), set `BitVBL` in `T_Actualise` (10414-10418).
5. Call user VBL routine list `VblRout` (10421-10428).
6. `MousInt` (mouse sprite position), `Shifter` (Shift Up/Down colour cycling), `FlInt` (Flash), `FadeI` (Fade) (10430-10440).
7. `Animeur` (AMAL) unless `T_SyncOff` != 0 (i.e. Synchro Off => AMAL run manually by `Synchro`) (10443-10448).
`WVbl` (10454) = `graphics.WaitTOF` d0 times.

### A.3 Mouse
**Position accumulation** – input.device handler `IoHandler` `+W.s:12638`. For `IECLASS_RAWMOUSE` with RELATIVEMOUSE qualifier: `T_MouseX += ie_X; T_MouseY += ie_Y` (12705-12712). Raw mickeys added 1:1 to a *half-pixel* accumulator. Qualifier word stored in `T_MouXOld` (used as button state). `T_MouYOld & 3` selects event passing mode: 0 normal (AMOS eats events), 1 trash, 2 pass all, 3 keys only (12686-12701).
Fake events (`Fake_Code=$789A789A`, 12920) are let through.

**Per-VBL (MousInt, 10473-10531)**: clamp `MouseX` to [MouXMin,MouXMax], `MouseY` to [MouYMin,MouYMax] (limits are stored ×2); then `XMouse = MouseX>>1`, `YMouse = MouseY>>1` — these are the *hardware* coordinates returned by `X Mouse`/`Y Mouse` (MXy 10729). Sprite 0 control words built from (XMouse-HotX, YMouse-HotY) as standard SPRxPOS/SPRxCTL: VSTART low 8 bits/HSTART>>1, VSTOP = VSTART+height, bits 8 of VSTART/VSTOP, HSTART bit0 (10500-10530). So mouse pointer = hardware sprite 0, drawn at hardware position minus hot spot.

**Set mouse** `MSetAb` 10877: X Mouse=x stores x*2 clamped to limits (absolute hardware coordinates); `EntNul` means "unchanged".
**Limit Mouse** `MLimA` 10930: args hardware x1,y1,x2,y2; clamp x2<=458, y2<=312, negative -> 0, swap if reversed, store ×2. `MLimEc` 10912: limits to screen n's visible area by converting (0,0) and (EcTx-1,EcTy-1) via CXyHard. File selector uses 128,30,448,312 (`+Lib.s:17780-17784`). (No explicit default found in +W.s; interpreter sets limits at start.)

**Buttons** `MBout` 10536: from stored input qualifier: bit0 = left (IEQUALIFIERB_LEFTBUTTON), bit1 = right, bit2 = middle. `MRout` 10551 (`Mouse Click`): bits set only on press edge vs `T_OldMk`.

**Show/Hide counter** (10633-10668): `T_MouShow`: 0 = shown, negative = hidden. `Show` increments, `Hide` decrements; `Show On` forces 0, `Hide On` forces -1. Only when value becomes exactly 0 is the sprite shown, exactly -1 hidden (`HiSho`). Ignored when copper off.
**Change Mouse n** `MChange` 10601: n=1..3 (internally 0..2 after -1? caller passes 0-based; `d1<3` => mouse bank) selects shape in built-in `Mouse.abk`; n>=3 (=user image n-3+1... `subq #3`) uses sprite bank image which must be 1 word wide (16 px) & 2 planes (4 colours) else falls back to shape 1. Hot spot from image header (+6/+8).
**Mouse bank** (`+W.s:16717-16719`, `bin/+AMOSPro_Mouse.abk`, 1824 bytes): `"AmSp"`, word count (=$27? first word after magic; must be >=4) then shapes: `w width_words, w height, w nplanes, w hotX, w hotY, data[width*height*nplanes words]` (plane-major). Shape 0: 16x11 arrow 2 planes hot 0,0; shape 1: 16x16 crosshair hot 7,7; shape 2: 16x16 (clock). Shapes from index 4 onward are the **fill patterns** (Set Pattern). After last shape: 32 words palette; colours 16-31 copied into default screen palette (sprite colours) (`+W.s:9276-9293`). A word -1 is written after last shape as terminator.

**Coordinate conversion** (EcYBase=$1000, `+Equ.s:546`; screen field EcWX/EcWY are hardware display position, EcWY includes +EcYBase; EcVX/EcVY = Screen Offset):
- Hardware->screen `CXyScr` 10737: `y = ((Yh + $1000 - EcWY) * (lace?2:1)) + EcVY`; `x = ((Xh - EcWX) * (hires?2:1)) + EcVX`. hires = EcCon0 bit 15, lace = EcCon0 bit 2.
- Screen->hardware `CXyHard` 10760: `Xh = (hires ? x>>1 : x) + EcWX`; `Yh = (lace ? y>>1 : y) + EcWY - $1000`. NOTE CXyHard ignores EcVX/EcVY offset (asymmetry, as in original).
- Screen->window `CXyWi` 10780: `cy = (y - WiDyI)/WiTyCar` (char height), `cx = x/8 - WiDxI`; out of range -> EntNul.
- `GetSIn` 10827 (`Mouse Screen`, `Screen(x,y)`): walk `T_EcPri` priority list front-to-back, skip hidden (BitHide) and screens with number >= limit, hit if `0<=Xh-EcWX<EcWTx` and `0<=Yh+$1000-EcWY<EcWTy`.

### A.4 Joystick `ClJoy` 10962-10986
Port n (0/1): reads JOYnDAT (`Circuits+10+2n` = $DFF00A/$DFF00C), fire = CIAA PRA ($BFE001) bit 6+n, active low -> result bit 4. Direction index = (bit9,bit8 of JOYDAT)<<2 | (bit1,bit0); `JoyTab` (10985): `0000,0010,1010,1000,0001,0000,0000,1001,0101,0000,0000,0000,0100,0110,0000,0000`. Result bits: 0=up,1=down,2=left,3=right,4=fire (`+Lib.s:13640-13690` Jup/Jdown/Jleft/Jright/Fire test these bits). Joy(1) is the normal joystick port.

### A.5 Keyboard
**Handler** `IoHandler` (12638) -> `IeKey` (12733) -> `Cla_Event` (12765-12862). On RAWKEY:
- Key up (bit 7): clear bit in `T_ClTable` (bit index = raw&$7F; byte raw>>3, bit raw&7). `T_ClTable` is declared 12 bytes but indexed over 16: bytes 12-15 ARE `T_ClShift` (declared right above it in `+WEqu.s:308-309`).
- Key down: set bit in `T_ClTable` (12846-12852) (always, for Key State), then:
  - raw `$60-$67` (shift/ctrl/alt/amiga qualifiers): not stored in buffer.
  - raw `$40-$5F`: lookup `Cla_Special` (12867-12871):
    ```
    $40>$47: $ff,$08,$09,$0d,$0d,$1b,$00,$00   (Space->convert, BS=8, Tab=9, KP Enter=13, Return=13, Esc=27, Del=0, $47=0)
    $48>$4f: $00,$00,$ff,$00,$1e,$1f,$1c,$1d   (KP '-'->convert, Up=$1E, Down=$1F, Right=$1C, Left=$1D)
    $50>$57: $fe x8                           (F1-F8)
    $58>$5f: $fe,$fe,$ff,$ff,$ff,$ff,$ff,$00   (F9,F10, KP ( ) / * + -> convert, Help=0)
    ```
    `$ff` -> RawKeyConvert; `$fe` = function key: if qualifier bit 6 (Left Amiga) -> insert string `TFF1[n]` (Key$ 1-10); if bit 7 (Right Amiga) -> `TFF2[n]` (Key$ 11-20); else stored as ascii 0 with scancode. Values 0..$7F stored directly as ascii.
  - other raw codes: console.device `RawKeyConvert` (default keymap) with CTRL qualifier temporarily removed (so Ctrl+key gives normal ascii; shift byte keeps ctrl bit). First output byte used as ascii; 0 if none.
  - Amiga-A check: `(qual & AmigA_Shifts)==AmigA_Shifts && ascii in {Ascii1,Ascii2}` -> AMOS/Workbench flip (not stored). Default `$00406141`: shifts=$40 (Left Amiga), 'a'/'A' (`+W.s:9257-9264`).
  - Ctrl-C (qual bit 3 + 'C'/'c') -> sets `BitControl` in `T_Actualise` (break), not stored.
  - else `Cla_Stocke` (12879).
- **Buffer** `T_ClBuffer`, `ClLong=96` bytes = 32 slots × 3 bytes: `[shift(qualifier low byte), rawkey, ascii]`; head `T_ClTete`, tail `T_ClQueue`, +3 per entry wrapping at 96; full when next head == tail (key dropped). Also copies last key to `T_ClLast` (4 bytes just below buffer: byte0 shift, byte1 raw, byte3 ascii) used by editor/ON BREAK.
- Shift byte bits (Amiga IEQUALIFIER low byte, also `+Equ.s:775-778`): 0 LShift,1 RShift,2 CapsLock,3 Ctrl,4 LAlt,5 RAlt,6 LAmiga,7 RAmiga.
- `ClInky` 12945: returns d1 = shift<<24 | raw<<16 | ascii (0 if empty). `Inkey$` (`+Lib.s:13582`) returns 1-char string of ascii (even if ascii 0 -> returns Chr$(0)? no: if whole d1==0 returns ""), stores `SScan`=shift<<8|raw for `Scancode` (`+Lib.s:13602`, returns raw & clears) and `Scanshift` (`+Lib.s:13611`).
- `Key State(n)` (`ClInst` 13011, `+Lib.s:13620`): n<128, returns -1 if bit raw n set in ClTable.
- `Key Shift` (`ClSh` 13004) returns byte `T_ClShift`, which is never written by name: it is byte 12 of the key matrix, i.e. the up/down bits of raw keys $60-$67 = bit0 LShift($60), 1 RShift($61), 2 CapsLock($62), 3 Ctrl($63), 4 LAlt($64), 5 RAlt($65), 6 LAmiga($66), 7 RAmiga($67). Same layout as the qualifier byte. (Caps Lock reflects the raw key event state.)
- `Put Key a$` (`ClPutK` 13026): each char stored as (0,0,ch); `'...'` quoted text skipped as comment; byte 1 = escape followed by 3 bytes shift,scan,ascii.
- `Key$(n)=` (`ClFFk` 13064): `TFF1` and `TFF2` are contiguous 10×24 bytes each (index n 0..19 across both), max 23 chars; backquote `` ` `` -> CR LF (13,10); byte 1 + 3 bytes copied raw.
- `Key Speed delay,speed` (`TKSpeed` 12971): values in 1/50 s converted to timeval and set via input.device IND_SETTHRESH / IND_SETPERIOD (OS autorepeat).
- No embedded raw->ASCII keymap: AMOS relies on console.device default keymap except the `Cla_Special` table above. Port must supply a US Amiga keymap (raw $00-$3F main keys, $40 space etc.).

---------------------------------------------------------------------
## B. DRAWING PRIMITIVES

### B.1 Architecture
Almost all BASIC graphics go through **graphics.library on the current screen's RastPort** (`T_RastPort` = `Ec_RastPort` of current screen, set by `Ec_Active` `+W.s:3770-3780`). So pixel semantics = Amiga graphics.library (Kickstart 2.0) semantics. Offsets used: RastPort+8 AreaPtrn, +12 TmpRas, +16 AreaInfo, +25 FgPen, +26 BgPen, +27 AOlPen, +28 DrawMode, +29 AreaPtSz, +32 Flags, +34 LinePtrn, +36 cp_x, +38 cp_y, +56 AlgoStyle, +62 TxBaseline.
Calls go through `L_GfxFunc` (`+Lib.s:11249-11290`): if screen `EcAuto`(Autoback) = 0 just call; else Autoback protocol: double-buffered screen -> AutoBack1 (wait VBL+erase bobs if mode 2), draw, AutoBack2 (mode1: redirect bitmap to physic; mode2: bob draw+swap), restore cp, draw again, AutoBack3 (back to logic). Single buffered -> draw then AutoBack4 (redraw bobs) (`+W.s:3552-3628`). For a GPU port: draw into both buffers when autoback != 0.

### B.2 Instructions (`+Lib.s`)
| Instruction | Line | Implementation / semantics |
|---|---|---|
| `Plot x,y[,c]` | 9524/9535 | optional SetAPen(c); cp=(x,y) (`GrXY` 11225: EntNul coords keep current); `WritePixel`. Updates graphic cursor. |
| `=Point(x,y)` | 9557 | `ReadPixel`; moves cp to x,y. -1 outside. |
| `Draw To x,y` | 9579 | `RDraw` (= graphics `Draw`) from cp to x,y using LinePtrn, FgPen/BgPen, DrawMode. cp ends at x,y. |
| `Draw x1,y1 To x2,y2` | 9588 | Move cp then `Draw`. Both endpoints inclusive (Bresenham of graphics.library). |
| `Circle x,y,r` | 9603 | `DrawEllipse(x,y,rx,ry)` with rx=ry=r, **rx doubled if screen hires** (EcCon0 bit15). r<=0 -> error. Outline only. |
| `Ellipse x,y,rx,ry` | 9617 | `DrawEllipse` (no hires correction). |
| `Box x1,y1 To x2,y2` | 9673 | cp=(x1,y1); `PolyDraw` 4 points: (x1,y2),(x2,y2),(x2,y1),(x1,y1±1) — last point is y1+1 (or y1-1 if y1+1>=y2) so the start pixel isn't drawn twice (matters for COMPLEMENT). Inclusive corners. |
| `Bar x1,y1 To x2,y2` | 9946 | `RectFill(x1,y1,x2,y2)` inclusive; error if x2<=x1 or y2<=y1. Uses AreaPtrn (Set Pattern), FgPen/BgPen, DrawMode; outline in AOlPen if AREAOUTLINE flag (Set Paint 1). cp=(x1,y1). |
| `Polyline x,y To ...` / `Polyline To` | `+ILib.s:5468` | builds point list (start = cp if first token is To), cp=first point, `PolyDraw`. Needs >=2 points. |
| `Polygon ...` | `+ILib.s:5506` | `InitArea` (AAreaInfo), `AreaMove` first, `AreaDraw` each, TmpRas via `L_GetRas` (size = EcTPlan), `AreaEnd` (filled, pattern, outline if Set Paint). Not autobacked. Even-odd fill per graphics.library area fill. |
| `Paint x,y[,mode]` | 9917/9923 | reserves TmpRas, colour = ReadPixel(x,y), calls `SuPaint` -> `TPaint` (`+W.s:4309`) through autoback wrapper (`L_GfxPnt` 11241). mode is masked to bit0 and passed in d4 but **TPaint never reads it** (d4 overwritten at 4363): Paint always = flood of the 4-connected region of pixels equal to the seed colour. |
| `Ink a[,b[,c]]` | 10056-10086 | a -> SetAPen (FgPen), b -> SetBPen, c -> AOlPen (rp+27, outline colour). EntNul = keep. |
| `Gr Writing n` | 10090 | `SetDrMd(n)`: bit0 JAM2 (0=JAM1: only set bits drawn in FgPen; 1=JAM2: 0 bits drawn in BgPen), bit1 COMPLEMENT (XOR planes), bit2 INVERSVID (swap set/unset for text/patterns). Default after screen open = 1 (JAM2) (`+W.s:3089`). |
| `Set Line $mask` | 10103 | rp->LinePtrn (16 bit, MSB first, $FFFF default `+W.s:3090`). Used by Draw/Box/Polyline. |
| `Set Paint 0/1` | 9901 | rp->Flags bit3 AREAOUTLINE: outline Bar/Polygon in Ink c colour. |
| `Set Pattern n` | 9890 -> `SPat` `+W.s:4697-4780` | n=0: solid (AreaPtrn=0). n>0: shape (3+n) of Mouse.abk (skip 4 shapes then n-1). n<0: image -n of sprite bank. Height rounded down to power of 2 (<= image height, max 2^7), AreaPtSz=log2(h); only first 16-px word column of each line used; if image has >1 plane, AreaPtSz negated (multicolour pattern, nplanes planes). |
| `Set Tempras` | 9968-9982 | TmpRas size management. |
| `Clip [x1,y1 To x2,y2]` | 9705/9713 -> `TSClip` `+W.s:4187` | no args: reset to full screen. With args: x2<=EcTx, y2<=EcTy, x2>x1, y2>y1 else error; EntNul keeps old. Stored in EcClipX0..Y1 (X1/Y1 exclusive); installs layers clip region (x0,y0)-(x1-1,y1-1) (`Ec_SetClip` 4227-4298). |
| `Cls [c[,x1,y1 To x2,y2]]` | 8693-8724 -> `EcCls` `+W.s:3636` | no-arg Cls = clear current text window (WiCall ClsWi). `Cls c` = whole screen colour c. Rect: coords clamped to [0,EcTx]/[0,EcTy], fills [x1,x2) × [y1,y2) (**exclusive** end) with colour c via blitter (MCls/MCls2 edge masks 3740-3773). Ignores clip & drawmode; autoback-aware. |
| `Text x,y,a$` | 9820 | cp=(x,y), graphics `Text` -> y is the **baseline** (`=Text Base` returns rp->TxBaseline, 9838). Uses FgPen/BgPen/DrawMode/AlgoStyle. Default font = system topaz 8 (`T_DefaultFont`, `+W.s:3102`). |
| `Set Text style` / `=Text Style` | 9879/9867 | poke rp->AlgoStyle (bit0 underline, bit1 bold, bit2 italic). |
| `=Text Length(a$)` | 9850 | graphics TextLength. |
| `Gr Locate x,y`, `=Xgr`, `=Ygr` | 9661/9641/9650 | cp = rp+36/+38. |

Screen-open defaults (`+W.s:3071-3109`): InkA = window pen, InkB = window paper, DrawMode=1 (JAM2), LinePtrn=$FFFF, cp=(0,0), clip = full screen, font = default system font. Per-screen graphic state is saved/restored (`T_EcSave`, `Ec_Pull` 3206-3235) when switching current screen.

### B.3 Paint algorithm detail (`TPaint` `+W.s:4309-4630`) – for exact reproduction
1. Seed must be inside clip rect (X0<=x<X1, Y0<=y<Y1) else nothing.
2. Build a 1-bpp mask (TmpRas) over clip rect (word aligned): bit=1 where pixel colour != seed colour (blitter minterms: per plane D = B | (bit? ~A : A), `PMask` 4632).
3. Scanline flood fill (left run, then right, pushing up/down seeds; 4-connected; buffers 1024 bytes + chained 2048-byte blocks), setting mask bits for filled pixels (4366-4525). Stops at mask=1 or clip borders.
4. Re-mask: keep only bits whose pixel equals seed colour (= filled pixels) (4527-4531).
5. For each plane, blit with minterm $CA (D = mask ? pattern : screen) where pattern word = (patbit ? FgPen bit : BgPen bit) (solid pattern `FoPat=$FFFF` if no Set Pattern) (4534-4609). Pattern line index = y (relative to clip-rect word-aligned top) modulo pattern height; multicolour pattern advances per plane. DrawMode is NOT applied (always replace). Outline (Set Paint) not applied.
=> Rust: flood fill 4-connected region of seed colour within clip; fill each pixel with pattern(px,py) ? InkA : InkB (InkB only matters with a pattern).

### B.4 Notes on misc tables
- `MCls`/`MCls2` (`+W.s:3740-3773`) are left/right edge word masks for blitter Cls/fills, not fill patterns.
- `FoPat dc.w -1` (`+W.s:9790`) solid pattern.
- `Def_Font IncBin "bin/+WFont.bin"` (`+W.s:9799`) = AMOS special characters for text windows (512 bytes = 64 chars × 8 bytes) — covered by window/font research.
