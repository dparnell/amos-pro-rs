# AMAL (AMOS Animation Language) — implementation notes from +W.s

All line numbers refer to `AMOS-Professional-365/+W.s` unless prefixed with another file.
(Note: the shell `grep` wrapper skips these ISO-8859 files; use `command grep -a`.)

## 1. Entry points (AMOS.library system vector, `SyCall n`)

System jump table at +W.s:9881ff (`SyCall` = `jsr n*4(T_SyVect)`; numbers from +Equ.s:263-364):

| # | Equate | +W.s line | Routine | Purpose |
|---|--------|-----------|---------|---------|
| 40 | AMALTok | 9922 | TokAMAL (7157) | tokenise only |
| 41 | AMALCre | 9923 | CreAMAL (7946) | tokenise + create/replace channel |
| 42 | AMALMvO | 9924 | MvOAMAL (8267) | On/Off/Freeze channel(s) |
| 43 | AMALDAll | 9925 | DAllAMAL (8338) | delete all channels |
| 44 | AMAL | 9926 | Animeur (8428) | run one tick of all channels |
| 45 | AMALReg | 9927 | RegAMAL (7882) | address of a register |
| 46 | AMALClr | 9928 | ClrAMAL (7855) | delete all + zero global regs |
| 47 | AMALFrz | 9929 | FrzAMAL (8348) | freeze all |
| 48 | AMALUFrz | 9930 | UFrzAMAL (8359) | unfreeze all |
| 65 | SetSync | 9947 | SyncO (7867) | Synchro On/Off (T_SyncOff) |
| 66 | Synchro | 9948 | Sync (7870) | manual tick (only if Synchro Off) |
| 67 | PlaySet | 9949 | SetPlay (7913) | Amplay: set R0/R1 of a channel range |
| 77 | MovOn | 9959 | TMovon (8225) | =Movon(n) |
| 79 | ChanA | 9961 | TChanA (8253) | =Chanan(n) |
| 80 | ChanM | 9962 | TChanM (8257) | =Chanmv(n) |

Init: `AMALInit` (7848) clears list, seed `T_AmSeed=$1234`; `ClrAMAL` zeroes the 26 global registers `T_AmRegs` (+WEqu.s:253).
Globals in +WEqu.s:249-256: `T_AmDeb` (list head), `T_AmFreeze`, `T_AmChaine` (list pointer used by the interrupt; cleared = nothing runs), `T_AmBank`, `T_AmRegs` (26 words RA..RZ), `T_SyncOff`, `T_AMALSp`, `T_AmSeed`.

### BASIC side (+Lib.s / +ILib.s)
- Token table +Lib.s:1210-1272: `Move On/Off/Freeze [n]`, `Anim On/Off/Freeze [n]`, `Anim n,a$ [To ...]`, `=Movon(n)`, `=Chanan(n)`, `=Chanmv(n)`, `Channel n To ...`, `Amreg(r)`/`Amreg(ch,r)` (instr+func), `Amal On/Off/Freeze [n]`, `=Amalerr`, `Amal n,a$|bankprog` / `Amal n,a$ To address`, `Amplay speed,dir [,start To end]`, `Synchro On/Off`, `Synchro`.
- On/Off/Freeze (+Lib.s:11631-11751): type mask d2: `%0001`=AMAL, `%0010`=Anim, `%1100`=Move X/Y; d3: 1=On, -1=Off (delete), 0=Freeze. No parameter → d1=-1 (all channels).
- `Amal`/`Anim`/`Move X`/`Move Y` (+Lib.s:11756-11888, `L_MvA3`): channel limit **16 when Synchro On (interrupt), 64 when Synchro Off** (+Lib.s:11815-11821). AMAL bank = bank #4 named "Amal" (+Lib.s:11823-11832). If the string argument is a number < 1024 it is a program index into the bank (+Lib.s:11834-11850). Object target from `AnCanaux` table (2 bytes/channel: type, number), unless `To address` form (type 5).
- `Channel n To Sprite|Bob|Screen Display|Screen Size|Screen Offset|Rainbow m` (+ILib.s:5567-5600): type 0 sprite (m<64), 1 bob (m<64), 2 screen display (m<8), 3 screen size, 4 screen offset, 6 rainbow (m<4). Default after Run: channel n → Sprite n (+ILib.s:211-218).
- Errors (+Lib.s:11877-11888): CreAMAL returns -1 → out of memory; negative -3 → "screen not opened", -24 → "bob not defined" (via L_EcWiErr); positive k → BASIC error `SpEBase+2+k` = 106+k; offset of error in string stored to `PAmalE`, returned by `=Amalerr` (+Lib.s:11587-11592).
- `Synchro On` clears `InterOff`, `Synchro Off` sets it to -1 (+Lib.s:11609-11626).
- `Amreg` (+Lib.s:11931-11983): global regs 0..25; local regs channel<64, reg<10.
- `Amplay` (+Lib.s:11988-12007): range default 0..63.

Error texts (+Editor_Config.s:940-948, numbers = 106+k):
1 → 107 Syntax error in animation string; 2 → 108 Next without For; 3 → 109 Label not defined; 4 → 110 Jump To/Within autotest; 5 → 111 Autotest already opened; 6 → 112 Instruction only valid in autotest; 7 → 113 Animation string too long; 8 → 114 Label already defined; 9 → 115 Illegal instruction during autotest; 10 → 116 Amal bank not reserved. (Codes assigned by the fall-through chain AniE10..AniE1, +W.s:7601-7620.)

## 2. Channel structure ("FORMAT D'UNE SEQUENCE", +W.s:7123-7145)

One heap block per (channel,type); doubly linked list sorted by `AmNb`.

| Off | Name | Size | Meaning |
|-----|------|------|---------|
| 0 | AmPrev | L | prev channel |
| 4 | AmNext | L | next channel |
| 8 | AmLong | W | block length (for free) |
| 10 | AmNb | W | `channel*4 + type` (type 0=AMAL,1=Anim,2=MoveX,3=MoveY) — sort key (7974-7975) |
| 12 | AmPos | L | PC of main program (0 = stopped) |
| 16 | AmAuto | L | PC of autotest body (0 = none) |
| 20 | AmAct | L | pointer to target "act block" (see §4) |
| 24 | AmBit | W | bit15 = frozen/off; low bits = bit number to set in `T_Actualise` |
| 26 | AmCpt | W | Move/Play step counter |
| 28 | AmDeltX | L | Move: X delta 16.16 / Play: X data pointer / STOS-move: count |
| 32 | AmDeltY | L | Move: Y delta 16.16 / Play: Y data pointer / STOS-move: end value |
| 36 | AmVirgX | W | Move: X fraction / Play: X wait counter |
| 38 | AmVirgY | W | Move: Y fraction / Play: Y wait counter |
| 40 | AmFin | L | PC of instruction after current Move/Play |
| 44 | AmAJsr | L | per-frame routine (Anim/STOS anim/STOS move), 0 = none |
| 48 | AmAAd | L | anim current pointer |
| 52 | AmAALoop | L | anim loop start |
| 56 | AmACLoop | W | anim remaining loop count (0 = forever) |
| 58 | AmACpt | W | anim frame delay counter |
| 60 | AmIRegs | 10 W | local registers R0..R9, stored **reversed**: Rn at `AmIRegs+20-2*(n+1)` |
| 80 | AmStart | — | tokenised code follows |

A newly created channel has `AmBit = $8000 + bit` (8099, 8114, 8127, 8146) — i.e. it is created **frozen**: nothing runs until `Amal On` (OnOfFrz clears bit 15, 8307-8315). Replacing an existing (channel,type) frees the old block (7999-8019). During creation `T_AmChaine`/`T_AmFreeze` are cleared so the interrupt skips everything, then restored (7971-7972, 8035).

## 3. Tokeniser (TokAMAL, 7157-7620)

Inputs: D3 type (0 AMAL, 1 STOS Anim, 2 Move X, 3 Move Y), A1 string, A2/D2 output buffer, D1 work buffer (labels: 26 words; jump fixups at +52; FOR stack at +52+104). Returns D0=0 ok, -1 buffer too small (A0 = needed length; CreAMAL then allocates exactly and re-tokenises, 7953-7965; overflow handling `AniTrop` 7224), >0 error code with A0 = offset in string.

### Character reading (AniChr, 7046-7061)
Returns only chars in 33..'Z' (uppercase letters, digits, punctuation) plus `|` and `!`. **Lowercase letters, spaces and everything else are skipped**; ESC (27) skips itself + 2 following bytes. Hence keywords are single uppercase letters and the rest of the word is decoration: `Move`=`M`, `Let`=`L`, `PLay`=`PL`, `AUtotest`=`AU`, `eXit`=`X`, etc. Unrecognised statement letters are silently ignored (7222).

Numbers (AniLong 7064-7120): optional `-`, decimal, or `$`hex; 32-bit parsed, used as 16-bit word.

### Statements (dispatcher 7187-7222)

| Syntax | Token(s) | Notes |
|--------|----------|-------|
| `L:` (letter + colon) | — | label A..Z (7289-7300); duplicate → err 8; label inside autotest tagged bit0 |
| `J L` | $1C + offset word | Jump (7302-7317); fixup in pass 2 (7560-7581); crossing autotest boundary → err 4 |
| `L r=exp` | $20, expr, $7C, target token [+reg word] | Let (7360-7373); target r = A, X, Y, R0-R9, RA-RZ |
| `M dx,dy,steps` | $18, expr, expr, expr | Move relative (7388-7400); not allowed in autotest (err 9) |
| `F Rn=exp T exp` | $28, expr, expr, reg word, TO slot word | For (7320-7342), register must be R*; not in autotest |
| `N Rn` | $2C, back offset | Next (7344-7357); must match For's reg (err 2) |
| `I exp J L` / `I exp D L` / `I exp X` | $24, expr, then Jump/Direct/eXit | If (7376-7385) — only these three forms |
| `W` | $10 | Wait (7412-7415); not allowed in autotest |
| `P` | $14 | Pause (7404-7407) |
| `PL n` | $C0, expr | Play movement n from AMAL bank (7417-7422); needs a bank (err 10) |
| `E` | $00 | End (7409) |
| `A n,(img,delay)(img,delay)...` | $CC, expr, skip-offset, pairs of exprs, 0 | Anim (7424-7455); n = loop count, 0 = forever |
| `AU( ... )` | $60, offset, body..., $68 | AutoTest On (7230-7246); `AU()` empty → $64 AutoTest Off (7247); nested → err 5 |
| `)` | $68 (+patch offset) | closes autotest (7255-7267); without open → err 6 |
| `X` | $68, 0 | eXit autotest (7250-7254); only in autotest |
| `D L` | $6C, offset | Direct (7269-7286); only in autotest |

Pass 2 appends a final $00 (End) token (7561).

Tokens are literally the handler offsets from table `AmJumps` (8370-8424) → threaded code. Index (comment value /4): 00 Stop/End, 04 STOS-Anim, 08 STOS-MoveX, 0C STOS-MoveY, 10 Wait, 14 Pause, 18 Move, 1C Jump, 20 Let, 24 If, 28 For, 2C Next, 30 A=, 34 X=, 38 Y=, 3C R=, 40 =A, 44 =X, 48 =Y, 4C =R, 50 =On, 54 XS, 58 YS, 5C C(), 60 AutoTest On, 64 AutoTest Off, 68 eXit/end-autotest, 6C Direct, 70 constant, 74 XM, 78 YM, 7C end-of-expression, 80 J0, 84 J1, 88 K1, 8C K2, 90 `=`, 94 `<>`, 98 `<`, 9C `>`, A0 `+`, A4 `-`, A8 `/`, AC `*`, B0 `|`, B4 `&`, B8 SC(), BC BC(), C0 PLay, C4 XH, C8 YH, CC Anim, D0 Z(), D4 V(), D8 `!` (xor).

### Registers (AniReg 7627-7664)
- `A` image, `X`, `Y` of the attached object (read/write).
- `R0`..`R9`: local per channel (10 = `NbInterne`, 7124). Encoded as negative offset.
- `RA`..`RZ`: 26 global words shared by all channels (`T_AmRegs`), also `Amreg(n)` from BASIC.
- Play uses R0 = speed, R1 = direction (8567, 8632, 8650-8651).

### Operands / functions (AniOpe 7667-7795, runtime 8925-9117)
| AMAL | Meaning | Runtime |
|------|---------|---------|
| number, `-n`, `$hex` | constant ($70 + word) | 9088 |
| `A`,`X`,`Y`,`Rn` | registers | 9056-9075 |
| `XM`,`YM` | mouse X/Y in **hardware** coordinates (`T_XMouse/T_YMouse`) | 9091-9095 |
| `K1`,`K2` | left/right mouse button: -1 pressed, 0 not (CIA-A PRA bit 6 / POTGOR bit 10) | 9107-9117 |
| `J0`,`J1` | joystick port bits: 1 up, 2 down, 4 left, 8 right, 16 fire (ClJoy 10963-10990, JoyTab) | 9097-9105 |
| `O` (`On`) | -1 while a Move is in progress (AmCpt>0) else 0 | 9077-9083 |
| `XH(s,x)` | screen→hardware X: `x/2 (if hires) + EcWX` | 8926-8937 |
| `YH(s,y)` | screen→hardware Y: `y + EcWY - EcYBase` | 8939-8948 |
| `XS(s,x)` | hardware→screen X: `(x-EcWX)*2 (if hires) + EcVX` | 8950-8962 |
| `YS(s,y)` | hardware→screen Y: `y + EcYBase - EcWY + EcVY` | 8964-8974 |
| (screen number masked `&7`; missing screen → -1, 8976-8987) | | |
| `BC(n,s,e)` | bob n collides with bobs s..e → -1/0; **only when Synchro Off**, else 0 | 8989-9002 |
| `SC(n,s,e)` | sprite collision, same restriction | 9004-9017 |
| `C(n)` | object n flagged in last collision table `T_TColl` → -1/0 (GetCol 535) | 9021-9027 |
| `Z(n)` | random: `seed=seed*$3171+VHPOSR+1; ((seed>>8) & n)` (n should be 2^k-1) | 9029-9039 |
| `V(n)` | music VU-meter byte for voice n (0-3) from music extension, read-and-clear | 9041-9054 |

### Expressions (AniExp 7798-7844; AmEvalue 8916-8923; operators 9119-9167)
Stream: `operand, operand2, op, operand3, op, ..., $7C`. **Strictly left-to-right, no precedence, no parentheses**, all 16-bit signed. Operators: `=`, `<>`, `<`, `>` (true = -1, false = 0), `+`, `-`, `*` (muls, low word), `/` (divs; divide by zero leaves left operand unchanged), `|` or, `&` and, `!` xor.

## 4. Target objects ("act blocks", Creation 8045-8148)

Every target exposes 4 words: `+0` flag byte (bit0 image changed, bit1 X changed, bit2 Y changed), `+2` X, `+4` Y, `+6` A. Writing X/Y/A sets the flag and `bset AmBit, T_Actualise` so the main loop redraws (8887-8904).

| Type | Target | Act block | Actualise bit |
|------|--------|-----------|---------------|
| 0 sprite n | `T_HsTAct + n*8` (+WEqu.s:221) | X,Y,image | BitSprites=14 (8122-8129) |
| 1 bob n | `BbAct/BbX/BbY/BbI` in bob struct (+Equ.s:393-396) | X,Y,image | BitBobs=13 (8106-8120) |
| 2 screen display | `EcAW/EcAWX/EcAWY` | hardware position | BitEcrans=12 (8097) |
| 3 screen size | `EcAWT/EcAWTX/EcAWTY` | width/height | 12 (8094) |
| 4 screen offset | `EcAV/EcAVX/EcAVY` | scroll offset | 12 (8091) |
| 5 address | arbitrary memory (`Amal n,a$ To addr`) | words at addr | (8132-8139) |
| 6 rainbow n | `RnAct/RnX/RnY/RnI` | X = base colour index, Y = screen line (min 28, +EcYBase), A = height (-1 hidden) | 12 (8142-8148; used in CopBow +W.s:6052-6100) |

## 5. Run-time (Animeur 8428-8471) and timing

Called from the VBL interrupt (`VblIn` +W.s:10446-10452) **once per 50 Hz frame when Synchro On** (`T_SyncOff==0`); with `Synchro Off` the VBL skips it and BASIC `Synchro` calls `Sync` (7870) → Animeur once. (`BC`/`SC` only work in Synchro Off mode because they use the blitter.)

Per tick, for each channel in list order (sorted by channel*4+type):
1. If `AmBit` bit 15 set (frozen / not yet `On`) → skip (8439-8440).
2. If `AmAuto` ≠ 0: execute autotest from its start with **jump budget 20** (8443-8448).
3. If `AmPos` ≠ 0: execute main program from AmPos with **jump budget 10** (8451-8456).
4. If `AmAJsr` ≠ 0: call per-frame anim routine (Anim `A`, STOS Anim, STOS Move) (8459-8462).
Accumulated actualise bits written to `T_Actualise` (8469).

Execution model: threaded code — each handler tail-jumps into the next, so any number of Let/If/etc. run in the same frame. A frame for a channel ends at:
- `P` Pause: AmPos = next instr, return (8817) → resumes next frame.
- `M` Move: one step per frame (see below).
- `N` Next when looping: AmPos = instruction after For, return (8872) → **each For/Next iteration costs one frame**.
- `J` Jump when the budget hits 0: in main, AmPos = jump target, return (8876-8885); in autotest just return.
- `W` Wait: AmPos=0 → main stops; only the autotest keeps running (8820).
- `E` End: AmPos=0, AmAuto=0 (8813-8815).
- `PL` Play: one step every R0 frames.

Autotest: `AU(` encountered in the main flow sets AmAuto=body and skips over the body (8793-8797); each later frame the body runs before main. Body/`X` end with token $68 = rts (8803) → main continues from AmPos. `D L` (8805-8810) sets AmPos=L and AmCpt=0 (aborts any Move), then the main program runs from L in the same frame. `AU()` → AmAuto=0 (8799).

If (8835-8842): false skips the 4-byte Jump/Direct/eXit. For (8845-8855): evaluate start, end; store start in reg, end in TO slot. Next (8857-8873): reg++; if `TO >= reg` loop (yield) else fall through.

### Move dx,dy,n (8474-8561)
First entry (AmCpt 0 → -1): evaluate dx, dy, n (n≤0 → 1); AmFin = next instruction; delta = `((|d| << 8) / n) << 8` with sign → 16.16 (divu overflow → 0); position starts at current X/Y with fraction $8000 (rounding), and the first step is applied immediately. Each following frame: AmCpt--; if >0 add delta (write X/Y + flags only if integer part changed); when AmCpt reaches 0 the next instruction executes in that same frame (8505-8507). So n steps over n frames; end point can differ from start+d by rounding (delta truncated to 1/256 pixel precision first).

### Anim (A) (8662-8694)
`A loops,(img,delay)...`: sets AmAJsr=AmDoAni and continues the program. Each frame decrement delay counter; when 0 set A=img (flag bit0), counter=delay, advance; at list end (0 word) loop: loops==0 → forever, else decrement, stop when it reaches 0.

### Play n (8564-8659) — movement data from AMAL bank
Init: AmFin = next instr; bank movement section at `bank+4`: word count, then word table (entry n, 1-based, at `bank+4+2n`) holding offset/2 from `bank+4` to movement header H (n=0 or n>count or offset 0 → skip). Header: `word speed` → R0, R1 := 1, `word yoff`; X stream bytes from H+5 (H+4 is a 0 sentinel), Y stream bytes from H+yoff+1 (H+yoff = sentinel). Each step (every R0 frames, AmCpt reloaded from R0, 8632): if R1 < 0 → stop, continue program; for X then Y stream: byte 0 → end of play (go to next instruction); bit7 clear → 7-bit signed delta added (R1≠0 forward, pointer++) or subtracted (R1=0 backward, pointer--); bit7 set → pause: first time loads counter=byte&$7F, decrements each step, advances when it hits 0. X and Y waits are independent.
`Amplay speed,dir[,s To e]` (SetPlay 7913-7935) sets R0/R1 of channels s..e (EntNul = leave unchanged).

### STOS-compatible Anim / Move X / Move Y strings (7459-7544, runtime 8696-8790)
- `Anim n,"(img,delay)(img,delay)...[L]"`: token $04, word pairs, terminated -1 (stop) or -2 (loop, `L`). Runs via AmAJsr (StAni 8704-8722).
- `Move X|Y n,"[start](speed,step,count)...[L | E end]"`: token $08/$0C; header words: start ($8000 = keep current), loop flag (-1 if `L`), end value ($8000 none); triples (speed>0, step, count≥0) ending with 0. Every `speed` frames coordinate += step; stop/loop when coordinate == end value, or after count steps move to next triple (count 0 = until end condition); at list end loop if flag else stop (8745-8790).

### Channel queries
- `=Movon(n)` (8225-8241): a Move X/Y channel (type 2/3) of n exists, is On and AmAJsr ≠ 0.
- `=Chanan(n)` (8253-8256): AmAJsr ≠ 0 (an anim is running) on first entry of channel n that is On.
- `=Chanmv(n)` (8257-8260): AmPos ≠ 0 (program running).
- `Freeze`/`Unfreeze` all: move list pointer `T_AmChaine` ↔ `T_AmFreeze` (8348-8366).

## 6. Rust-port notes
- Implement AMAL as a compiler to an enum bytecode; replicate: uppercase-only lexing, left-to-right expressions, 16-bit wraparound, jump budgets 10/20, Next yields a frame, new channels start frozen until `Amal On`, channel ordering by `ch*4+type`, run order autotest → main → anim.
- Tick at 50 Hz from the frame loop (Synchro On) or on explicit `Synchro` call.
- Object coupling via a small "act block" (flags + X/Y/A) per target, with dirty flags consumed by the sprite/bob/screen/rainbow updater.
