# AMOS Pro: Bobs, Hardware/Computed Sprites, Collisions, Blocks (+W.s)

All line refs are `+W.s:N` unless noted. Note: the shell `grep` wrapper skips these ISO-8859 files; use `command grep -a`.

## 1. Entry points (SyCall numbers -> +W.s routines)

The jump table is at +W.s:9881-9982 (`SyIn`). The numbers are defined in +Equ.s:263-364.

| # | Name | Routine | Purpose |
|---|---|---|---|
| 24 | SetHs | HsSet 11446 | position one HW sprite (internal) |
| 25 | USetHs | HsUSet 11635 | remove one HW sprite |
| 28 | AffHs | HsAff 11663 | build sprite DMA buffers (columns) |
| 29 | SetSpBank | HsBank 11340 | set T_SprBank |
| 30 | NXYAHs | HsNxya 11400 | `Sprite n,x,y,i` |
| 31/32 | XOffHs/OffHs | 11366/11377 | Sprite Off n / Sprite Off |
| 33 | ActHs | HsAct 11423 | apply pending sprite changes |
| 34 | SBufHs | HsSBuf 11183 | `Set Sprite Buffer n` |
| 35/36 | StActHs/ReActHs | 11407/11414 | |
| 39 | PriHs | HsPri 11299 | `Sprite Priority n` (BPLCON2) |
| 49 | SetBob | BobSet 863 | `Bob n,x,y,i` and `Set Bob` |
| 50/51 | OffBob/OffBobS | BobOff 995 / BobSOff 1014 | |
| 52 | ActBob | BobAct 1191 | recompute bobs and the priority list |
| 53 | AffBob | BobAff 1964 | save backgrounds, then draw bobs |
| 54 | EffBob | BobEff 1873 | restore backgrounds (erase) |
| 57 | LimBob | BobLim 1029 | `Limit Bob` |
| 59 | SprGet | GetBob 669 | `Get Sprite/Get Bob/Get Block` grab |
| 60 | MaskMk | Masque 1787 | `Make Mask` |
| 61 | SpotHot | SpotH 642 | `Hot Spot` |
| 62/63/64 | ColBob/ColGet/ColSpr | BbColl 299 / GetCol 543 / SpColl 410 | software collisions |
| 68/69 | XYBob/XYSp | BobXY 803 / HsXY 11392 | X Bob/Y Bob/I Bob, X Sprite… |
| 70 | PutBob | BobPut 1183 | `Put Bob` |
| 71 | Patch | TPatch 819 | `Paste Bob`/`Paste Icon` |
| 75/76 | SetHCol/GetHCol | HColSet 78 / HColGet 90 | `Set Hardcol`/`=Hardcol` |
| 81 | SPrio | TPrio 1087 | `Priority On/Off`, `Priority Reverse On/Off` |

The EcCall block functions (+Equ.s:576-579, 596-599, 624-625) are wired at +W.s:2522-2571. They are: CBlGet/CBlPut/CBlDel/CBlRaz -> MakeCBloc/DrawCBloc/FreeCBloc/RazCBloc, BlGet/BlDel/BlRaz/BlPut -> MakeBloc/DelBloc/RazBloc/DrawBloc, BlRev -> RevBloc, and DoRev -> RevTrap.

The interpreter's update sequence is in +Lib.s:11435-11446. `Update` is EffBob, ActBob, AffBob, `EcCall SwapScS` (swap every double-buffered screen), then ActHs and AffHs. `Bob Update` (+Lib.s:11459) runs the same four bob steps. `Bob Clear` = EffBob. `Bob Draw` = ActBob + AffBob. Automatic updates are gated by `ActuMask` bits 5 (bobs) and 6 (sprites), plus `T_Actualise` bits BitBobs=13 and BitSprites=14 (+Equ.s:832-833).

## 2. Sprite/bob bank format

### In memory (`T_SprBank`)
Used at e.g. +W.s:155-166 and 1198-1215.
- Word 0: number of images N.
- Then N entries of 8 bytes: `long imagePtr, long maskPtr`. Image i (1-based) is at `bank + 2 + (i-1)*8`; the code uses `lea -8+2(a4,d0.w)` with d0 = i*8.
  - `imagePtr == 0`: the image is empty and is skipped.
  - `maskPtr == 0`: the mask is not computed yet. It is computed lazily on the first draw (BobCalc 1364-1367 calls Masque).
  - `maskPtr < 0`: no mask. `No Mask` stores `$C0000000` (+Lib.s:12529). Collision routines treat `<=0` as "no collision possible" (`ble ColRF`, 164).
  - `maskPtr > 0`: points to `long totalSize`, then one plane of mask data (same size as one image plane). Masque at 1787-1825.

### Image header (10 bytes), then planar data
- `+0` width in **words** (pixel width = w*16).
- `+2` height in lines.
- `+4` number of planes.
- `+6` hot spot X. The low 14 bits are signed (decoded with `lsl #2; asr #2`, e.g. 172-174). **Bit 15 = image currently flipped in X, bit 14 = currently flipped in Y** (state flags, see §4).
- `+8` hot spot Y.
- `+10` data: plane 0 (w*2*h bytes, rows of w words), then plane 1, and so on.

Data size = w*2*h*planes. The allocation is +10 (GetBob 690-704).

### On-disk bank (.abk) / default mouse bank
`"AmSp"`, `word count`, then per image `w,h,planes,hotx,hoty` + data, with **no mask pointers**. After the images comes a 32-word palette ("Pointe la palette", 9281-9288). The default mouse bank is `bin/+AMOSPro_Mouse.abk`, IncBin'd at 16717-16722 and checked at 9267-9289. It has 39 images: images 1-3 are the mouse pointers (1 word wide, 2 planes, e.g. 16x11). Images 4+ are the fill patterns (`Set Pattern` 4697-4730 uses `SoMouse` into this bank).

### Mask generation (Masque 1787)
The mask is one plane, the bitwise **OR of all image planes**, so colour 0 is transparent. Size = w*2*h, preceded by a long holding the allocation size (size+4). The header comment says "1 mot blanc a droite" but no extra word is actually added.

### Grab (GetBob 669-760)
This is `Get Sprite`/`Get Bob`/`Get Block`. It allocates a new image with planes = **EcNPlan of the source screen** and hot spot 0,0. It frees any old mask (`4(desc)` is cleared, so the mask is recomputed lazily). The right edge is masked with MCls. Grabbing at a non-word X uses a shift blit.

## 3. Hot spot (`SpotH` 642-667; wrappers in +Lib.s:12536-12566)
- `Hot Spot n,x,y` (mode 0): sets hotx=x and hoty=y directly.
- `Hot Spot n,$xy` (mode = (p & $77)+1). The X nibble = (p>>4)&3 and the Y nibble = p&3: 0 = left/top (0), 1 = centre (size/2), 2 = right/bottom (full size: w*16 or h).
- The flip flags in bits 15/14 of `+6` are preserved (`and #$C000` / `or`).

## 4. Flipping (`Hrev`/`Vrev`/`Rev`: image number bits $8000 = X flip, $4000 = Y flip)
- The bob image number `BbI` keeps the flags. `BobAct` copies `BbI & $C000` to `BbRetour` (1217-1221), and the index is `BbI & $3FFF`.
- `Retourne` (1652-1674) flips the **bank image in place**, but only when the requested flags differ from the image's current flags in `+6`. It XORs the flags, calls RBobX/RBobY, and stores the new flags. The flip happens at draw time (`BobAff` 2028-2031). If two bobs show the same image with different flips, the data is flipped back and forth on every draw. An emulator can simply render the flipped image.
- X flip (RBobX 1677): reverses bytes per row using a 256-byte bit-reverse table `TRetour` (built by RbInit 1631). It also flips the mask, and sets hotx' = w*16 - hotx.
- Y flip (RBobY 1708): swaps rows and sets hoty' = h - hoty.
- Hot spot under flip in BobCalc (1369-1386): it computes the effective hot spot for the target orientation, so a flipped bob mirrors around its hot spot.
- **HW sprites ignore the flip bits.** `HsAct` masks with `$3FFF` (11450) and `HsSet` never calls Retourne ("Pas de retournement!", 11475). It still uses whatever orientation the shared image is currently in.
- `Bnk.UnRev` (+Lib.s:8405) un-flips a whole bank via `EcCall DoRev` (for saving).

## 5. Bob data structure (+Equ.s:387-443, `BbLong`)

| Field | Size | Meaning |
|---|---|---|
| BbPrev/BbNext | l,l | doubly linked list, **kept sorted by bob number** (BobSet 873-913) |
| BbNb | w | bob number (0..BbMax-1; BbMax is from config, BbInit 762) |
| BbAct | w (byte used) | change flags: bit0 image, bit1 X, bit2 Y changed; **<0 = Bob Off pending**. The AMAL channel links to `BbAct` (DAdAMAL 1143). |
| BbX,BbY,BbI | w | position (screen coords) and image (+flip bits) |
| BbEc | l | screen the bob belongs to (the current screen at creation, ResBOB 966-970) |
| BbAAEc | l | screen offset (in words) for the draw |
| BbAData/BbAMask | l | source pointers for the blit (data plane 0 / mask, with clipping offset applied) |
| BbNPlan | w | planes to draw - 1 = min(image planes, screen planes) - 1 (BobCalc 1388-1393) |
| BbAPlan | w | **bitmask of screen planes to write** (`Set Bob` planes param, default -1 = all) |
| BbASize | w | BLTSIZE for the draw |
| BbAMaskG/BbAMaskD | w | first/last word masks (left/right clipping and shift) |
| BbTPlan | w | image plane size in bytes |
| BbTLigne | w | (unused here) |
| BbAModO/BbAModD | w | source/dest modulos |
| BbACon/BbACon0/BbACon1 | w | minterm word (bit15 = user minterm) / BLTCON0 / BLTCON1 |
| BbADraw | l | pointer to the draw routine (BbA16/BbAp/BbAL, without-mask variants via `bmi`) |
| BbLimG/D/H/B | w | clip rectangle (Limit Bob). Default 0,EcTx,0,EcTy. X is multiple of 16 |
| BbARetour/BbRetour | l,w | descriptor pointer + requested flip flags |
| BbDecor | w | number of background buffers: 1, or **2 if the screen is double buffered** (BitDble), or **0 if back = -1** (ResBOB 975-983) |
| BbEff | w | `Set Bob` back param: 0 = save/restore background; >0 = erase by filling with colour (back-1); -1 = no erase |
| BbDCur1/BbDCur2 | w | offsets (0 or `Decor`) of the current/previous background slot. **Swapped every BobAct** (1205-1207) |
| BbDCpt | w | number of saves still to do this frame |
| BbEMod/BbECpt/BbEAEc/BbESize/BbETPlan | w | erase params. BbECpt = Put Bob skip counter |
| Decor slot (20 bytes, x2 at BbDABuf and BbDABuf+Decor) | | BbDABuf (l) buffer pointer, BbDLBuf (w) length/2, BbDAEc (w) screen offset, BbDAPlan (l, word used) planes, BbDNPlan (l, word used) nplanes-1, BbDMod (w) modulo, BbDASize (w) BLTSIZE |

`Set Bob n,back,planes,minterm` (+Lib.s:12205-12226) calls SetBob with x/y/image = EntNul. **The parameters are applied only when the bob structure is created** (ResBOB). For an existing bob, BobSet only pokes X/Y/I (CreBb5 932-947). So `Set Bob` must come before the first `Bob n,...`. The defaults from `Bob` are back=0, planes=-1, minterm=0 (+Lib.s:12235-12238).

## 6. Bob rendering pipeline

### Order per update
`EffBob` (restore the old backgrounds on the **logic** bitmap, using slot BbDCur2) → `BobAct` (flip the decor slots, recompute changed bobs, build the priority list) → `BobAff` (save the backgrounds under the new positions into slot BbDCur1, then draw in priority order) → `SwapScS` (swap logic/physic for all double-buffered screens, 2650-2703).

### BobCalc (1356-1625)
This computes blit params for a bob or block in screen A0.
- It subtracts the hot spot from X/Y (bit31 of d3 = no hot spot, used by Paste/blocks).
- Planes = min(image, screen). An image with fewer planes than the screen **only touches its own planes**; the other screen planes keep their pixels.
- Clips against BbLim*. Left and right clipping are word-granular, with masks from the MCls2 tables.
- Shifted case (x & 15 != 0): an extra word is added and the shift goes in BLTCON0/1 (1420-1527). The draw routine is `BbAp`, or `BbAL` if too tall for one BLTSIZE.
- Word-aligned case: `BbND` 1534-1625, routine `BbA16`.

### Minterms (BobCalc 1394-1411)
Blitter channels: A = mask, B = image data, C = screen, D = screen.
- Default (minterm 0): BLTCON0 = `$0FCA` (A,B,C,D on, LF=$CA → D = A·B + ¬A·C, the cookie cut). With **no mask** it is `$07CA` (A channel off, BLTADAT = $FFFF in BMAp 2214-2240, so the result is an opaque rectangle).
- User minterm m (bit15 set, from `Set Bob`/`Paste Bob`/`Put Block` minterm): `$0F00|m` with a mask, or `$0700|m` without one.
- The fast path BMA16 (2159-2212) handles no-mask + LF=$CA + aligned as a plain A→D copy.
- Emulation in pixel terms, per written plane p (bit set in BbAPlan, p < nplanes): `dst_p = LF(mask, src_p, dst_p)`. With the default LF it writes src_p wherever the mask (OR of all image planes) is set. Note that the mask is the OR over **all image planes**, while only min(image, screen) planes are written.

### Background save/restore
- Save (BobAff 1969-2020) copies screen→buffer for each plane in BbDAPlan. The buffer size is (nplan+1)*BbETPlan, which covers the extra shift word and border lines.
- Restore (BobEff 1887-1950) copies buffer→screen. If BbEff>0 it fills with colour BbEff-1: each plane is filled with 0 or $FFFF by colour bit (BbEfC 1928).
- Double buffering: two decor slots alternate (BbDCur1/2). Each buffer gets its own save, and EffBob restores the slot saved two frames ago, i.e. the same bitmap.
- Bob Off: BbAct = -1. BobAct's BbDel (1276-1288) decrements BbDecor once per update and frees the bob only after 1 or 2 more erases, so both buffers get cleaned.
- Put Bob (BobPut 1183): BbECpt = BbDecor, so the next 1 or 2 EffBob passes skip the restore and the image stays imprinted (BbE5 1924-1927).

### Draw order / priority (BobAct 1290-1352, TPrio 1087-1098)
- The default draw order is ascending bob number (the list is sorted by number). Later draws appear **on top**, so higher numbers are in front.
- `Priority On`: T_Priorite = the current screen. Bobs on that screen go into BbPrio2 and are bubble-sorted by **Y ascending, then X ascending**. They are appended **after** the other screens' bobs, so the bob with the greatest Y is drawn last and appears in front.
- `Priority Off`: T_Priorite = 0.
- `Priority Reverse On` (T_PriRev): the final table is reversed (1336-1347), so draw order is reversed.

### Limit Bob (BobLim 1029-1083)
- `Limit Bob [n,]x1,y1 to x2,y2` applies to bobs on the current screen (n = -1 means all).
- x1 and x2 are rounded down to multiples of 16. It requires x1 < x2 ≤ EcTx and y1 < y2 ≤ EcTy.
- `Limit Bob` with no params resets to the full screen (EntNul → 0/EcTx/EcTy).

### Paste Bob (TPatch 819-861)
- Draws immediately using the current screen's **clip rectangle** (EcClipX0 rounded down to 16, EcClipX1 rounded up). There is no background save.
- Handles autoback: when EcAuto != 0 it draws twice, wrapped in TAbk1/TAbk2/TAbk3.
- `Put Block` (DrawBloc 12475) works the same way.

### Autoback (TAbk1-4, 3552-3628; EcAuto = Autoback 0/1/2)
- **Mode 1** draws the graphic to the logic bitmap and then again to the physic bitmap. TAbk2 retargets EcCurrent/RastPort to physic; TAbk3 restores logic.
- **Mode 2**: WaitVbl + BobEff, draw, then BobAct/BobAff/ScSwapS/WaitVbl/BobEff, draw again, then BobAct/BobAff/ScSwapS/WaitVbl.
- **Mode 0**: draws only to logic.

## 7. Hardware sprites

### Tables (+WEqu.s:174-233)
- `T_HsTAct`: 64 entries of 8 bytes, the "requested state". Fields: `+0` flag byte (bit3 = changed, <0 = remove), `+2` X, `+4` Y, `+6` image. `Sprite n,x,y,i` writes here (HsNxya 11400-11420). HsAct (11423-11444) later applies it via HsSet/HsUSet. AMAL sprite channels also point here.
- `T_HsTable`: 64 entries of `HsLong`=20 bytes (+WEqu.s:224-232). Fields: HsPrev(w), HsNext(w) (Y-sorted linked list of byte offsets for computed sprites; the list head is at offset -4), HsX, HsY, HsYr (top Y after hot spot), HsLien, HsImage(l), HsControl(l = SPRxPOS/SPRxCTL words). For direct sprites, word 0 = active and word 2 = "copy image into N buffers" counter (set to 3).

### Coordinates
- Sprite X/Y are **raw Amiga hardware coordinates** in low-res pixels and scan lines. The hot spot is subtracted and clamped to ≥0, then packed straight into SPRxPOS/SPRxCTL (11473-11499).
  - SPRxPOS = (VSTART low 8 bits) << 8 | HSTART >> 1.
  - SPRxCTL = (VSTOP low 8 bits) << 8 | bit2 VSTART bit 8 | bit1 VSTOP bit 8 | bit0 HSTART bit 0.
  - Attach is bit 7 of SPRxCTL.
- Screen↔hard conversion (CXyHard 10760-10778, CXyScr 10737-10757):
  - hardX = (hires ? x/2 : x) + EcWX
  - hardY = (lace ? y/2 : y) + EcWY - $1000. EcWY carries the base EcYBase=$1000 (+Equ.s:546).
  - The reverse adds EcVX/EcVY (the Screen Offset). CXyHard does **not** subtract the offset.
- The default screen display X is `T_DefWX = 129` (9389). DefWY comes from the config.

### Direct sprites 0-7 (HsSet 11458-11500, HsAff 11673-11806)
- Sprite n uses hardware channel n. An image w words wide uses consecutive channels n, n+1, ..., one 16-px column each, with X advanced by 16 per column (`add.l #$00080000` = +8 in HSTART>>1).
- Images with fewer than 4 planes are **4-colour**: only planes 0 and 1 are copied (HsBlit 11969-12006 interleaves 2 planes per line), giving colours 1-3 + transparent from the pair's palette.
- Images with **4 or more planes** are 16-colour **attached** sprites. They use an even/odd channel pair: planes 0-1 go in the even channel and planes 2-3 in the odd channel with the attach bit set. An odd start number skips a channel (HsAd3 11733-11742).
- The image is copied into all 3 buffers (logic/physic/inter) only when it changes (counter 3, 11703-11706). Otherwise only the control words are rewritten (HsAdP).
- If the image height + 1 ≥ HsPMax the sprite is ignored (11466-11468).
- **Sprite 0 is taken by the mouse while it is shown** (T_MouShow ≥ 0, 11463-11465, 11683-11687).

### Computed sprites 8-63 (multiplexing)
- Kept in a linked list sorted by HsYr ascending; ties go by sprite number (Hss20-23, 11538-11555).
- HsAff assigns each one to a "column" (hardware channel) not used by direct sprites. `T_HsPosition` holds 8 columns × 8 bytes: buffer pointer (0 = channel unavailable), HsYAct (next free line), HsPAct (lines used). It ends with -1.
- For each sprite (11818-11872) the code scans columns cyclically from the current one. A column fits if its YAct ≤ sprite top **and** PAct + height + 1 < HsPMax. Each 16-px slice of a wider image goes into another column with X+16. The code gives up after 8 failed tries, so the sprite slice is simply not shown. The +1 is the blank line needed between reused sprite DMA blocks.
- 16-colour computed sprites need an even/odd column pair with both columns free (HsMAff 11874-11944).
- Buffers: `Set Sprite Buffer n` (HsSBuf 11183-11234, +Lib.s:12261) sets HsNLine = n+2 lines per column, each line = 2 words (4 bytes). There are 8 columns × 3 buffers (physic/logic/inter, rotated in HsAff 11946-11958). T_HsChange tells the VBL to poke the new pointers into the copper sprite pointers (HsPCop 11277-11293). HsPMax = lines-2.
- Emulation hint: for faithful behaviour (sprites vanishing when more than 8 overlap on a line), reproduce the column allocator. Otherwise draw each sprite with its palette.

### Sprite palette
This is standard Amiga behaviour, not set specially by AMOS. Hardware sprite pair k (channels 2k, 2k+1) uses colours 16+4k+1..3; attached 16-colour sprites use colours 16-31 (16 = transparent). There are 32 colour registers.

### Sprite priority (HsPri 11299-11320; `Sprite Priority n`, n=0..4, +Lib.s:12272)
- Writes n into the **current screen's BPLCON2 copy (EcCon2)**: bits 0-2 PF1P, keeping bits 3-6.
- For the second screen of a dual playfield (EcDual < 0) it writes n<<3 into PF2P of the first (dual 1) screen.
- In Amiga terms, a value of p means sprite pairs 0..p-1 appear in front of the playfield: p=0 puts all sprites in front, p=4 puts all sprites behind.

### Mouse pointer
- Images 1-3 come from the mouse bank. `Change Mouse n` with n>3 uses sprite bank image n-3, which must be 1 word wide and 2 planes (MChange 10592-10642).
- The VBL interrupt (MousInt 10473-10530) clamps the internal MouseX/Y, which are kept at **double resolution**. XMouse = MouseX/2.
- It then pokes channel 0's control words in all 3 buffers with the hot spot subtracted, so the pointer bypasses HsAff.
- `Limit Mouse` (MLimA 10930-10958) clamps to hard X 0..458 and Y 0..312. `Limit Mouse [screen]` (MLimEc) converts the screen rectangle via CXyHard.
- Show/Hide uses a counter T_MouShow (10645-10690).

## 8. Collisions

### Hardware (HColSet 78-88 / HColGet 90-136)
- `Set Hardcol` writes CLXCON ($DFF098) = (spr&$F)<<12 | (enable&$3F)<<6 | (compare&$3F).
- `=Hardcol(n)`, for n = sprite 0-7, reads CLXDAT ($DFF00E). Table HColT (134-139) gives the CLXDAT bits for each pair against the other pairs (bits 9-14) and the playfields (bits 1-8). The result sets 2 bits per colliding pair in T_TColl, so Col(m) works afterwards.
- n<0 means playfield1 vs playfield2 (CLXDAT bit 0).
- An emulator must synthesise CLXDAT from pixel overlap.

### Software, bob and sprite (ColRout 144-264)
- Takes both images' boxes after the hot spot (image index &$3FFF, so **flip is ignored for the position**). Does a bounding-box reject.
- Then blits mask A AND mask B (BLTCON0 `$0CC0`, no D) over the overlap, with the shift from the X difference, and tests BZERO (DMACONR bit 13).
- **Both objects need masks.** If either mask pointer is ≤0 there is no collision.
- `Bob Col(n[,from to to])` (BbColl 299-389) tests against active bobs on the **same screen** only. With bit31 of d2 set (`Spritebob Col`) it converts the bob to hard coords and tests sprites (BbToSp/GoToSp).
- `Sprite Col` (SpColl 410-491) works the same way on T_HsTAct positions (hardware coords). `Bobsprite Col` uses SpToBb, converting bobs via CXyS.
- Results go in the 256-bit table `T_TColl`. `=Col(n)` (GetCol 543-578) returns -1 if bit n is set. `=Col(-n)` returns the first set index ≥ n (0 if none).

## 9. Blocks

### Normal blocks (`Get Block`/`Put Block`/`Del Block`/`Hrev Block`/`Vrev Block`; 12324-12560)
- Node layout: BlPrev, BlNext, BlNb, BlX, BlY, BlMask, BlCon, BlAPlan, then an 8-byte bank-style descriptor `BlDesc` (image pointer + mask pointer). List head is T_AdBlocs.
- `Get Block n,x,y,w,h[,mask]` (MakeBloc 12341-12400) grabs with GetBob and sets mask = `$C0000000` (none). If the mask flag is set it builds one with Masque.
- `Put Block n[,x,y[,planes[,minterm]]]` (DrawBloc 12475-12538) draws with BobCalc at no hot spot, using the current screen's clip rectangle.
- RevBloc (12542) flips with bits $8000/$4000.

### Compressed blocks (`Get Cblock`/`Put Cblock`; CBloc 12014-12146, PBloc 12148+)
- Byte-aligned: X and width are in units of 8 px.
- Header: prev(l), next(l), length(l), number(w), X/8(w), Y(w), TX/8(w), TY(w), nplanes(w).
- Then the data, row by row and plane by plane within each row, as **byte RLE**:
  - A byte ≥ $C0 is a run: count = (b & $3F)+1 copies of the following byte.
  - A literal byte ≥ $C0 is escaped as `$C0, value`.
  - Any other byte is a literal.
  - Runs are capped at 64.
