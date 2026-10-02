# AMOS Professional: Sound subsystem and Editor research

Source root: `AMOS-Professional-365/`. Line numbers below are 1-based and refer to the files as they are in the repo.
Note: the files are Latin-1 encoded. The interactive `grep` shell function in this environment fails on `+`-prefixed files; use `/usr/bin/grep` with `LC_ALL=C`.

---------------------------------------------------------------------
# PART A: SOUND
---------------------------------------------------------------------

## A.0 Where sound lives

* **All** audio is in the Music extension `+Music.s` (extension slot 1, assembled to `APSystem/AMOSPro_Music.Lib`). `+W.s` (AMOS.library) contains no audio code. It only runs the VBL server that calls extension VBL hooks. `+Lib.s` holds only the generic glue.
  * `+W.s:10345` `VblIn` is the VBL interrupt server, installed via `AddIntServer(INTB_VERTB)` at `+W.s:10328`. At `+W.s:10426-10433` it walks `VblRout(a5)` (8 longs, `+Equ.s:1151`) and calls each non-zero entry. `a6 = $DFF000` at that point.
  * `+Lib.s:2072` `WaitRout`: busy-waits until `T_VblCount` reaches `+d3` VBLs. `Play`'s delay uses it.
  * `+Lib.s:8553` `Bnk.Change` calls every extension's "bank check" hook (`ExtAdr+12`) after any bank load/erase. `+ILib.s:387` `DefRunExtensions` calls every extension's DEFAULT/RUN hook (`ExtAdr+4`).
  * `+Lib.s:3610` holds the bank-name constant `"Music   "`.
* The extension registers itself in `Mus_Cold` (`+Music.s:790-848`) with these hooks:
  * `MusDef` (reset on Default/Run) at `:857`
  * `MusEnd` (quit) at `:907`
  * `BkCheck` (bank changed) at `:961`
  * `MusInt` (VBL routine), which goes in `VblRout[0]`, so it is the **first** VBL routine.
* **Clock** (`:824-832`): `MusClock = 3546895` (PAL) or `3579545` (NTSC). `TempoBase = 100` (PAL) or `120` (NTSC).
* **Data zone `MB`** (`:2085-2114`):
  * `MuVu` (4 VU bytes)
  * `MuBase`, `WaveBase` (linked list of waves)
  * `Waves[4]` (per-voice instrument selector)
  * `EnvOn`, `EnvBase[4]`, `SamBank`, `BSeed`, `Noise`, `PNoise`, `TempoBase`
  * music bank pointers, `MuNumber`, `MuVolume`, `MuDMAsk`, `MuReStart`, `MuChip0-3`
  * `MuBuffer` (3 nested music states)
* The token table (keywords and syntax) is at `+Music.s:380-517`.

Error messages for the extension (base 178) are at `+Music.s:4757-4767`:
* 0 Wave not defined
* 1 Sample not defined
* 2 Sample bank not found
* 3 "256 characters for a wave"
* 4 Waves 0/1 reserved
* 5 Music bank not found
* 6 Music not defined
* 7 Can't open narrator
* 8 Not a tracker module

## A.1 Paula usage (what to emulate)

Hardware registers touched:
* `AUDxLC` (+0), `AUDxLEN` (+4, words), `AUDxPER` (+6), `AUDxVOL` (+8, 0..64), `AUDxDAT` (+$A) at `$DFF0A0/B0/C0/D0`
* `DMACON` `$DFF096` (bits 0-3, `$8000` = set)
* `INTENA`/`INTREQ` audio bits 7-10 (`%0000011110000000`)
* `$BFE001` bit 1 (power LED / audio low-pass filter)

Emulator model (standard Paula):
* 4 voices, 8-bit **signed** samples, linear volume 0..64.
* Output rate per voice is `clock / period` samples/s, so for PAL `3546895/period` Hz.
* AMOS clamps the period to **>= 124** everywhere it computes one (`:2892`, `:3293`).
* Stereo: voices 0 and 3 are left, 1 and 2 are right.
* Paula double-buffers LC/LEN. When DMA starts it latches LC/LEN and raises the audio IRQ. When the block finishes it re-latches whatever is in LC/LEN now (loops) and raises the IRQ again.
* AMOS relies on this for:
  * **instrument loops**: the start block is written, then on the next VBL the repeat block is written (`MuEvery` `:1550-1589`; tracker `:1650-1662`);
  * **sample streaming**: the IRQ handler queues 4 KB chunks.

A software mixer can represent each voice as `{data, pos, len, loop_start, loop_len, period, volume}` and step at `clock/period` with fractional accumulation.

`DmaWait` (`:2986`) and `mt_music` (`:1748-1754`) busy-wait about 5 raster lines between stopping and starting DMA. This is irrelevant for emulation; just restart the voice.

**Led On / Led Off** (`:3891-3900`):
* `Led On` = `bclr #1,$BFE001`, so the filter is enabled (LED bright).
* `Led Off` = `bset`, so the filter is disabled.
* The AMOS music commands 06/07 (`:1340-1345`) and tracker effect `E0x` (`mt_filter` `:2036`) do the same.
* `E` effect quirk: bit 1 of `$BFE001` = `param & 1`, so `E00` turns the filter on and `E01` turns it off. *Any* Exy is treated as filter (there are no other extended effects).
* Emulate the filter as the A500 "LED" filter: 2-pole Butterworth low-pass at about 3.3 kHz. A fixed about 4.4-5 kHz 1-pole RC filter is always on for A500 authenticity, but that is optional.

### Voice ownership / priority (important behavioural detail)

Music, sound effects and samples share the 4 voices.
* `MuDMAsk` is the set of voices music may use.
* Every effect (`Play`, `Bell`, `Boom`, `Shoot`, `Sam Play`, `Say`) calls `StopDma` and `VOnOf` (`:2973`, `:3741`) to remove its voices from `MuDMAsk`. Music keeps running silently on those voices.
* When the effect ends it sets the voice bit in `MuReStart`. This happens on:
  * envelope end (`MuIntE` `:3630-3636`);
  * a non-looping sample reaching its end with a fixed volume (`Sami_handler` `:1046-1048`).
* The next VBL's `MuEvery` (`:1599-1640`) gives the voice back to music. It sets `AUDxLEN=2` and, if that music voice has an instrument, re-arms its repeat part.
* `Voice mask` (`InVoice` `:3728`) sets `MuDMAsk` directly. `Boom`/`Shoot`/`Say` silence all music (`VOnOf(0)`).

## A.2 VBL routine order (`MusInt`, `+Music.s:1066`) at 50 Hz PAL / 60 Hz NTSC

1. **Envelopes** (`:1066-1091`). For each voice with `EnvOn` bit set:
   * `EnvVol += EnvDelta` (16.16 fixed point);
   * `AUDxVOL = EnvVol>>16`;
   * `--EnvNb`; at 0 call `MuIntE` to load the next phase.
   * Voices whose envelope ended have DMA and IRQ switched off (`d5`, `:1089-1091`).
2. **Noise refresh** (`:1093-1108`). If any voice is playing noise, 8 more words of the noise buffer are re-randomised per VBL. The cursor `PNoise` moves downward and wraps at `LNoise-2`.
   * RNG (also used at init, `:886-891`): `seed = ((seed + VHPOSR) * $3171) >> 8` (16-bit `mulu`, keep low word). The written word is the new seed.
   * Port: any 16-bit PRNG is fine.
3. **AMOS music** (`Music` `:1111`) if `MuBase != 0`, **else Tracker** (`Tracker` `:1647`). They are mutually exclusive, and the tracker only runs if no AMOS music is playing.

## A.3 Built-in effects: Boom / Shoot / Bell (no sample data, all synthesized)

These are generated from the **noise wave (wave 0)** or the **square wave (wave 1)** plus fixed envelope tables (`:2131-2134`). Envelope format: pairs of `(duration_in_VBLs, level 0..64)`, ending with a 0 duration.

```
EnvDef   1,64, 4,55, 5,50, 25,0, 0          default for every wave
EnvShoot 1,64, 10,0, 0
EnvBoom  1,64, 10,50, 50,0, 0
EnvBell  1,64, 4,40, 25,0, 0
```

* **Boom** (`InBoom` `:2676`):
  * note 36, noise, `EnvBoom`, via `Shout` (`:2696`).
  * `Shout` silences all music, then plays on **voice 3 note 36, voice 2 note 37, voice 1 note 38, voice 0 note 39** (`:2702-2709`, the "stereo effect").
  * No wait.
* **Shoot** (`:2687`): same as Boom with base note 60 and `EnvShoot` (notes 60-63 on voices 3..0).
* **Bell [n]** (`:2655-2670`):
  * note 70 (or n), forced wave 1 (square), `EnvBell`, all 4 voices, same note.
  * `GoBel` (`:2796`) checks note <= 96.

Square wave 1 is built in `MusDef` (`:871-878`): 256 bytes, the first 128 are `+127` and the last 128 are `-127`. Wave 0 (noise) is 255 random words.

Effective volume of an envelope phase is `EnvDVol * level / 64`, where `EnvDVol` = the voice's `Volume` setting (default 56).

## A.4 Wave synthesis for `Play`/`Bell`

Each wave (`NeWave` `:3462`) is a 256-byte signed cycle plus 6 successively 2:1-decimated copies:
* sizes 128, 64, 32, 16, 8, 4;
* each decimated byte is `(a+b)>>1` (`NewRout` `:3528`);
* total `LWave = 510` bytes (`:145`).

The struct is `WaveNext.l, WaveNb.w, WaveEnv[16 words], WaveDeb[510]` (`:144-152`).

`VPlay` (`:2839`). Note is 1..96, 0 = silence (`VSil` `:2914`):
```
idx    = note + 2                      ; after addq #3 / subq #1
octave = idx / 12                      ; TFreq row (:2140)
TFreq rows (byte offset into WaveDeb, length in WORDS):
  0:(0,128) 1:(0,128) 2:(256,64) 3:(384,32) 4:(448,16) 5:(480,8) 6:(496,4) 7:(504,2) 8:(504,2)
freq   = TNotes[idx]                   ; Hz table (:2149), TNotes[3]=33 .. TNotes[48]=440 .. 8372
period = MusClock / (freq * len_bytes) ; clamp >=124
```
So middle C (262 Hz) is note 37, and A440 is note 46. The selected sub-wave loops forever. Volume follows the wave's envelope (`WaveEnv`, copied from `EnvDef` at creation, editable with `Set Envel`). With no ADSR the voice would never stop, but every envelope ends with `dur=0`, which stops the voice (`MuIntE` `:3612-3636`).

**Envelope engine** (`MuIntE`). A phase is a word pair `(dur, level)`:
* `dur == 0`: end. Voice DMA off, noise off, music restart bit set.
* `dur < 0`: loop back to the first phase (`EnvDeb`). Not settable from Basic.
* otherwise `target = EnvDVol*level>>6`, `delta = (((target - cur_int) << 8) / dur) << 8` (signed `divs`, 16.16), and `EnvNb = dur`.

Each VBL adds `delta`. The starting volume is 0 (`clr.w EnvVol`).

**Noise via Play** (`VPl4` `:2935`):
* plays the 510-byte noise buffer as a *sample* at `freq = 2000 * TNotes[idx] / 440` Hz;
* `period = clock/freq`;
* the envelope is wave 0's envelope with bit0 set, which means "sample loops".
* With Boom/Shoot's fixed envelope the bit is clear, so the noise buffer plays once (about 0.45 s for note 36) while the envelope runs.

**Sample via Play** (`VPl2` `:2921`), after `Sample n To voice`:
* `freq = sample_freq * TNotes[idx] / 440`, so note 46 plays at the recorded rate;
* no envelope; volume = the voice's `Volume`.

## A.5 Sample playback: Sam Play / Sam Raw / Sam Bank / Sam Loop / Sam Swap / Sam Stop / Sload

**Sample bank format** (`GetSam` `:3181`; verified on `Examples/Samples/Instruments.abk`):
* `AmBk` bank, name `"Samples "`, default bank number **5** (`Sam Bank n`, 1..16, `:3008`). The data begins after the 20-byte AmBk header.
```
+0   word  nsamples
+2   long  offset[n]   (relative to bank data start; sample k at offset[k-1])
sample: +0 8 bytes name, +8 word frequency_Hz, +10 long length_bytes, +14 signed 8-bit data
```

Commands:
* `Sam Play [voices,] n [,freq]` (`:3102-3129`). `freq > 500` is required; the default is the sample's own frequency. `Sam Raw voices,addr,len,freq` (`:3131`) needs `len > 256` and `freq > 500`.
* `GoSam` (`:3143`) removes the voices from music and calls `SPlay`/`SPl0` (`:3248-3327`) per voice:
  * `period = MusClock / freq` (32-bit `Div32` `:3329`), clamped >= 124;
  * volume = the voice's `Volume` (`EnvDVol`), held in `Sami_dvol`.
* Streaming happens in the audio IRQ (`Sami_handler` `:1012-1063`, per-voice struct `:2159-2229`). It feeds `Sami_lplay = 4096`-byte chunks.
* At the end of data:
  * if `Sam Loop On` (`Sami_rpos=0`), restart from offset 0 (the whole sample loops);
  * else if a swap buffer is queued (`Sam Swap`), switch to it;
  * else let the last chunk finish, then on the next IRQ silence the voice (DMA off, `AUDxDAT=0`) and give the voice back to music.
* Emulator equivalent: play `[0,len)` once, or loop `[0,len)` if loop is on. Then free the voice and set music restart.
* `Sam Loop On/Off [voices]` (`SL0` `:3047`) sets the per-voice `SamLoops` bits.
* `Sam Swap voices To addr,len` (`:4054`) queues the next buffer, giving double-buffered streaming. `=Sam Swapped(v)` (`:4029`) returns:
  * 1 if the voice IRQ is off (stopped);
  * 0 if the swap is still pending or the new buffer is only at its first chunk;
  * -1 (true) once the swap has happened.
* `Sam Stop [voices]` (`:4077`): DMA and IRQ off.
* `Sload file,addr,len` (`:3213`): raw `Read()` from an open AMOS file channel into memory, for streaming.
* `Sample n To voices` (`:3076`) sets `Waves[v] = -n`. `Noise To voices` (`:3067`) sets `Waves[v] = 0`. `Wave n To voices` (`:3347`) sets `Waves[v] = n`, and n must exist.

## A.6 Play / Volume / Voice / Wave / Envel commands

| Command | Routine | Semantics |
|---|---|---|
| `Play [voices,]note,delay` | `InPlay2/3` `:2776-2795` | Voices default to `%1111`. Note 0..96 (0 = silence). Uses `Waves[v]`: >0 wave, 0 noise, <0 sample. After starting, `WaitRout(delay)` waits that many VBLs (`:2830-2833`). |
| `Play Off [voices]` | `:2951-2970` | `EnvOff`: `AUDxLEN=2`, `VOL=0`, music restart. |
| `Volume [voices,]v` | `:2713-2749` | v 0..63. Sets `EnvDVol` per voice (and `Sami_dvol` if the sample isn't envelope-driven). `Volume v` with one argument **also sets the music volume** (`MVol`). |
| `Mvolume v` | `:3694-3725` | Music volume 0..63. Recomputes `VoiVol = VoiDVol*MuVolume>>6` for all nested musics. |
| `Voice mask` | `:3728-3787` | Which voices music may use. |
| `Set Wave n,a$` | `:3361` | a$ must be >= 256 chars (the first 256 bytes are the signed cycle). n >= 1. Builds the decimated copies and the default envelope. |
| `Del Wave n` | `:3379` | n >= 2 (0 and 1 are reserved). |
| `Set Envel w,phase To dur,level` | `:3400-3428` | phase 0..6, level 0..63, dur > 0 for phase 0. Writes the pair at `WaveEnv[phase]` and a terminating 0 after it. |
| `Wave n To voices` | `:3347` | |
| `Noise To voices` | `:3067` | |
| `=Vumeter(v)` | `:3867` | Returns and clears the VU byte. Set to the note volume on each music/tracker note start (`:1220`, `:1810`). |
| `Led On/Off` | `:3891` | Filter (see A.1). |

`MusDef` defaults (`:857-900`): waves reset, wave 0 = noise, wave 1 = square, `Volume 56` on all voices, `Mvolume 56`, `Sam Bank 5`, sam loop off, music stopped, tracker stopped, narrator reset.

## A.7 AMOS music bank and player (`Music`, `Tempo`, `Mvolume`, `Music Stop/Off`)

**Bank**: `AmBk`, name `"Music   "`, bank **3** (`BkCheck` `:961-1003`). Verified on `Examples/Music/Music.abk`. All offsets are relative to the start of each section.
```
data+0  long ofs_instruments  (from bank data start)  -> BankInst
data+4  long ofs_songs                                -> BankSong
data+8  long ofs_patterns                             -> BankPat
Instruments: word count; then 32-byte entries starting at BankInst+2:
   +0 long sample_start   (offset from BankInst)
   +4 long repeat_start   (offset from BankInst)
   +8 word length         (words)
  +10 word repeat_length  (words)
  +12 word volume (0..64, clamped to 63)
  +14 word 0, +16 16-byte name
Songs: word count; long offset[count] (from BankSong; song n at offset[n-1])
   song+0: 4 words = offsets (from song start) of the 4 per-voice pattern lists
   song+8: word tempo(unused by player), word 0, 16-byte name   (not read by replay)
   pattern list: words = pattern numbers; terminator $FFFF(-1)=stop voice,
                 other negative (e.g. $FFFE) = loop to list start
Patterns: word count; then per pattern 4 words (one per voice) = offset from BankPat
   to that voice's command stream (0 = empty). Entry for (pat,voice) at BankPat+2+(pat*4+voice)*2
```

**Pattern command stream** (16-bit big-endian words, `MuStep` `:1197`):
* `0x0000-0x3FFF`: **note**. `period = w & 0x0FFF` (a raw Paula period). Triggers the current instrument (start+length, then repeat on the next VBL) and **continues** reading.
* `0x4000-0x7FFF`: **old-format note** (V1 compatible, `:1233-1249`). The low byte is the wait count. The *next* word is the period (0 = no note). This ends the voice's step.
* `0x8000-0xFFFF`: **command** `cmd = (w>>8)&0x7F`, `param = w & 0xFF` (jump table `:1252-1283`):

| cmd | meaning |
|---|---|
| 00 | end of pattern: fetch the next pattern number from the list |
| 01/02 | old slides (ignored) |
| 03 | set volume (0..63, scaled by `Mvolume`) |
| 04 | stop effect |
| 05 | repeat: 0 marks the loop start, n repeats n times |
| 06/07 | LED on/off |
| 08 | set tempo |
| 09 | set instrument (32-byte entry n) |
| 10 | arpeggio (xy) |
| 11 | portamento (speed); the next note becomes the target |
| 12 | vibrato |
| 13 | volume slide (`x0` = +x, `0y` = -y per tick) |
| 14 | slide up (period -= p) |
| 15 | slide down |
| 16 | delay: wait `param` steps, which ends the step |
| 17 | jump to pattern-list position |
| 18-31 | no-ops |

**Timing** (`:1117-1125`):
* Each VBL does `MuCpt += MuTempo`.
* If `MuCpt >= TempoBase` (100 PAL / 120 NTSC), subtract it and do **one step**. Otherwise run the **effects** (`DoEffects` `:1416`).
* So steps/second = `vbl_rate*tempo/TempoBase` = **tempo/2** on both PAL and NTSC.
* The initial tempo is 17 (`:3827`). `Tempo t` accepts 1..100.
* Effects run only on non-step VBLs. Each voice decrements its wait counter on each step and parses commands when it reaches 0.
* The volume register is written every effect tick (`AUDxVOL = VoiVol`).
* When all 4 voices have stopped (counter 0), the music ends and the previous nested music resumes (`MuFin` `:1178`). Up to **3** nested `Music n` calls are stacked (`InMusic` `:3789`). `Music Stop` stops the current one, `Music Off` stops all.

Effect formulas:
* **Slide**: clamp the period to `$71..$358`.
* **Portamento**: move towards the target by `param`.
* **Vibrato**:
  * the table `Sinus` is 32 bytes (`:2120`);
  * `delta = sin[(pos>>2)&31] * (param&15) >> 6`;
  * add if `pos>=0`, subtract if `pos<0`;
  * `pos += (param>>2)&$3C`.
* **Arpeggio**: cycles high-nibble, low-nibble, base note, with semitone offsets looked up in `Periods` (`:2124`).
* **Volume slide**: clamp 0..63.

Instrument loop emulation: on a note, play `[start, start+len*2)` bytes, then loop `[repeat, repeat+replen*2)` forever.

## A.8 Tracker (Track Load / Track Play) – NoiseTracker-style 31-instrument MOD

* `Track Load "file",bank` (`:4094-4180`) loads the whole file into a chip bank named `"Tracker "` (default bank 6). `TrackCheck` (`:4185`) stops playback if the bank disappears.
* `Track Play [bank[,pattern]]` (`:4240-4334`). The pattern argument is not supported. Init:
  * finds the number of patterns as the max of the 128 order bytes at `$3B8`, plus 1;
  * samples follow the patterns at `$43C + npat*1024`;
  * **zeros the first 4 bytes of every sample** (`clr.l`) and **clears finetune**, so finetune is ignored;
  * sets speed 6.
* `Track Stop`, `Track Loop On/Off` (`:4203-4235`). With loop off the song stops at the end (`Track_Stop`, `:1739-1744`). With loop on it restarts at the restart byte `$3B7`.
* Replay (`mt_music` `:1683`) is the classic NoiseTracker 2.x replayer ("mt_hejaSverige", `:1806`):
  * called **once per VBL** (50 Hz): no CIA timing, no BPM;
  * `Fxx` clamps the speed to 1..31 (`:2066-2077`);
  * song length is at `$3B6`, the order table at `$3B8`, rows of 16 bytes (4 channels x 4 bytes), 64 rows per pattern.
* Loop rule (`:1786-1808`): sample header n at `$14+30*(n-1)`, words `len, finevol, repstart, replen`, all in words.
  * If `repstart != 0`: `loop_ptr = start + repstart*2` and `play_len = repstart+replen`.
  * Else: `loop_ptr = start` and `play_len = len`. The loop length is `replen`.
* Supported effects:
  * `0` arpeggio (tick%3)
  * `1`/`2` porta up/down (clamps `$71`/`$358`)
  * `3` tone porta
  * `4` vibrato (`>>7`, NoiseTracker depth)
  * `5`, `6`
  * `A` volume slide
  * `B` position jump. Quirk: it also writes `$FFF` to `COLOR00` (`:2049`).
  * `C` set volume (<= 64)
  * `D` pattern break (the parameter is **ignored**; always row 0)
  * `E` filter only (see A.1)
  * `F` speed
* Not supported: 7, 8, 9, extended `E` commands.
* The period table is `mt_periods` (`:2279`, 36 notes, no finetune).
* `MED Load/Play/Stop/Cont/Midi On` (`:4430-4740`) just wrap the external `medplayer.library`. A port would need an OctaMED replayer or can drop this.

## A.9 Narrator: Say / Set Talk / Talk Stop / Mouth *

Uses the Amiga `translator.library` (English to phonemes) and `narrator.device` (`OpNar` `:2419`, `Say` `:2521`). It can only be approximated with a modern TTS or formant synthesizer.
* `Say a$[,mode]`:
  * if a$ starts with `~` it is already phonemes, and `"Q#U"` is appended;
  * otherwise it is translated (1024-byte buffer);
  * mode 0 is synchronous (`DoIO`); mode 1 is asynchronous (`SendIO`) with mouth-shape generation.
  * All 4 voices are taken from music, and the sample IRQ is removed during speech.
* `NarInit` defaults (`:2490-2506`): rate 150 wpm, pitch 110 Hz, mode 0 natural, sex 0 male, channel masks `Amaps` = `3,5,10,12` (a left/right voice pair), volume 63, sample frequency 22200.
* `Set Talk sex,mode,pitch,rate` (`:2595`): rate 40..400, pitch 65..320, mode 0/1 (natural/robotic monotone), sex 0/1. Any argument can be omitted.
* `Mouth Read`, `=Mouth Width`, `=Mouth Height` (`:2632-2650`, `:4344`) read narrator mouth data in async mode. `Talk Stop` (`:2751`), `Talk Misc` (`:4371`).

## A.10 Editor sound effects

The editor plays samples by **first letter of the sample name** from `APSystem/AMOSPro_Editor_Samples.Abk`, loaded as editor bank 5 when "Sounds" is on (`Ed_SamPlay` `+Edit.s:4800-4834`). It calls the Music extension's private entry `MB-4` = `Rbra L_GoSam` (`+Music.s:2084`).

Names in the bank: A, B, C, D, E, F, G, H, I.
* "A" is the startup hello (`+Edit.s:178`).
* "C" plays on each keypress (`:1630`).
* "H" plays on menus (`:1652`).

## A.11 Mixer recipe (48 kHz)

Per voice state: `{src:&[i8], pos_fp, end, loop:Option<(start,len)>, period, vol(0..64), enabled}`.
* Step: `pos += (3546895/period)/48000` per output sample. Use the PAL clock; the 50 Hz VBL tick drives everything else.
* Every 1/50 s, call in this order:
  1. envelope tick;
  2. noise refresh;
  3. AMOS-music tick (`MuEvery` restarts/loop pointers, then the step-or-effects decision) or tracker tick;
  4. emulate the "next VBL" loop-pointer latch by switching to the loop region when the first block ends.
* Mix L = v0+v3, R = v1+v2 (optionally with partial crossfeed), scaled by `vol/64`. Then apply the optional LED filter.

---------------------------------------------------------------------
# PART B: EDITOR
---------------------------------------------------------------------

## B.0 Packaging and entry points

* The editor is `+Edit.s` (15382 lines), assembled to a separately loaded segment `APSystem/AMOSPro_Editor`. In a debug build it is `Include`d from `+B.s:1371`; `+B.s:1370` includes `+Monitor.s` the same way.
* Entry jump table `EDebut` (`+Edit.s:40-54`): `Ed_Cold`, `Ed_Title`, `Ed_End`, `Ed_Loop`, `Ed_ErrRun`, `Ed_CloseEditor`, `Ed_KillEditor`, `Ed_ZapFonction`, `Ed_ZapIn`, `Ed_RunDirect`, **`Tokenise`**, **`Detok`**, `Mon_Detok`, `TInst`. Other code reaches these as `JJsr L_Ed_xxx`.
* Boot (`+B.s:47-79`):
  * if a program was given (e.g. autoexec), it is run with `L_Prg_RunIt`;
  * otherwise the editor loads: `Edit_Load`, `L_Prg_New`, `L_Ed_Cold`, `L_Ed_Title`, `JJmp L_Ed_Loop`.
* `Ed_Cold` (`:133`) needs >= 50 KB chip and >= 110 KB total. `Ed_LoadAllConfig` (`:216`) loads:
  * the config (`AMOSPro_Editor_Config`, built from `+Editor_Config.s`);
  * the resource bank (system string 45 = `AMOSPro_Editor_Resource.Abk`) as **editor bank 6**;
  * macros (46);
  * samples (48).
* `Ed_SetBanks` swaps to the editor's *private* bank list, which is separate from the user program's banks.

## B.1 Screen layout and use of AMOS.library (`+W.s`)

The editor is built entirely from ordinary AMOS screens, windows, zones and the AMOS menu system:
* **Screen 9 `EcEdit`** (`+Equ.s:763`): the main editor.
  * Opened in `Ed_OpenIt` (`+Edit.s:298-419`) via `L_Dia_RScOpen` with the resource bank's screen definition: palette, mode, 8 colours, hires.
  * Size is `Ed_Sx`x`Ed_Sy` (default **640x256**). Position `Ed_Wx=129`, `Ed_Wy=50`. Optional interlace.
  * `Ed_Ty = (Sy-16)/8` text rows.
  * Shown with `L_AppCentre` (scrolls into view at speed `Ed_VScrol`, `Ed_Appear` `:9647`).
* **Screen 8 `EcFonc`**: a 640x16 title/button strip used by Direct mode (the Escape screen).
* **Screen 10 `EcFsel`** is the file selector and **screen 11 `EcReq`** is the requester. Both come from the Lib.
* **Top bar** (`Ed_DrawTop` `:727`): y=0, 16 px high. It holds:
  * the AMOS logo (image `Ed_Pics`, 160 px);
  * **12 32x16 image buttons** (`Ed_BoutonsPics`, 2 images per button for up/down). Button 1 "Direct" is at x=0 and button 2 "WB" is at the right edge. Button 10 is Insert/Overwrite (on/off);
  * memory sliders (`Ed_MemoryDraw`).
  * Images are unpacked from the resource bank with `Ed_Unpack`.
* **Program windows** (`Edt_*` structures, `Edt_OpWindow` `:11245`, `Ed_DrawWindows` `:11595`). Several programs/views are stacked vertically on EcEdit, up to `Ed_WMax=(Ty-6)/3`. Each is an AMOS text window (`WiCall`) with:
  * a status line (`Edt_EtatSy=11` px), template strings at `+Editor_Config.s:449-455`: `Window- L- C- Free- Edit-` with Insert/Overwrite flags;
  * text rows (8 px each);
  * a bottom bar (5 px);
  * horizontal and vertical sliders;
  * splitting, linked scrolling and hidden windows.
* Text styling uses AMOS window escape codes (`ESC B/P/J/D/V/C`), strings 8-20 at `+Editor_Config.s:457-481`.
* The palette is 8 colours (`Ed_Palette` in config `+Editor_Config.s:50`), patched into the resource screen definition by `EdC_SetPalette` (`+Edit.s:4747`).

## B.2 Resource bank and the "Interface" dialog language

* `AMOSPro_Editor_Resource.Abk`: `AmBk`, bank 16, name `"Resource"`. Data layout (decoded):
  ```
  +0 word 3 ; +2 long ofs_graphics ; +6 long ofs_texts ; +10 long ofs_programs
  graphics: word nimg(=116) ; long ofs[nimg] ; word ncolours(8) ; word mode($8000 hires|4 lace) ; palette...
            images are AMOS packed bitmaps (Pac.Pic format, unpacked by L_UnPack_Bitmap, +Lib.s:25530ff)
  texts:    sequence of (byte 0, byte len, chars) ... terminated 0,$FF
  programs: word nprog(=2) ; long ofs[nprog] ; each program = ASCII Interface source text
  ```
  The editor dialog program #1 starts `LA62;SV0,187ME;JP1;LA2;...`.
* **Interface** is a compact interpreted UI language, executed by the Lib (`+Lib.s`):
  * `Dia_OpenChannel` `:19860`, `Dia_RunProgram` `:20506`, `Dia_Loop` `:21229`, `Dia_Evalue` `:22719`;
  * the instruction table `Dia_Instr` is at `+Lib.s:21048-21152`; two-letter opcodes with fixed parameter counts;
  * function tokens are listed at `+Lib.s:21157-21225` (binary table `bin/+Dialog_Funcs.bin`).
  * Instructions include:
    * `EX` exit, `UN x,y,img` unpack image, `LI`/`BO` line/box made from puzzle images, `SI` size, `BA` base
    * `PU` puzzle set, `SV n,v` set variable, `PR`/`PO` print / outline print, `IN` ink, `SF`/`SW`/`SL`/`SP`
    * `BU z,x,y,sx,sy,pos,min,max [draw][action]` button, `BR`/`BQ`/`BC` button return/quit/change
    * `JP`/`JS`/`RT` jump/gosub/return, `LA n` label, `RU` run (wait), `ED`/`DI` text/digit edit fields
    * `KY` key shortcut, `HS`/`VS` sliders, `AL`/`IL` active/inactive lists, `HT` hypertext, `IF`
    * `GB`/`GS`/`GL`/`GE`/`GP` graphics, `SA` save background, `SM` screen move, `CA` call, `XY`, `NW`
  * Expressions are **RPN** (`SX2/16-`). Functions include `BX BY SX SY ++ -- ** // ME`(message n), `VA`(variable), `TW TH CX SW SH MI MA BP ## XA YA XB YB P1..P9 == << >> && ||`.
  * Blocks in `[...]` are button draw and change routines.
  * The same engine powers user-program `Dialog`/`Resource` commands (`+Lib.s:14292-14990`).
* The editor opens Interface channel 1 with program #1 (`Ed_InitDialogues` `+Edit.s:3055`) and calls dialogs by label number via `Ed_Dialogue` and `L_Dia_RunProgram` (`:3108-3145`). The puzzle images base is `Ed_DiaImages=66`.
* Dialog strings come from the config message table (`ME n`).

## B.3 Menus

* The editor uses the standard **AMOS menu system** (`L_MnGere`, `Ed_MnGere` `+Edit.s:1640`). The right mouse button shows the menu strip.
* Menu text comes from `bin/+Editor_Menus.Asc`, included in the config (`+Editor_Config.s:520-524`).
* The tree definition comes from `bin/+Editor_Menus.Bin` (`:1131-1133`). It is built into the AMOS menu tree by `EdM_Init` (`+Edit.s:12580`, "Creation du menu" `:12548`).
* The top-level menus are **Project, Editor, Search, Config, User, Help** with sub-menus:
  * Procedures, Windows, Macros, System/User Marks, Cursor Move, Insert/Delete, Block, Set Editor;
  * "Programs" (the AMOS branch, a list of hidden programs, `EdM_BranchAMOS` `:12758`).
* Choosing a menu item gives 4 menu-level bytes, looked up in `EdM_Table` to get a **function number**, which is passed to `Ed_FCall`.
* Key shortcuts can be shown in the menus (`EdM_Keys`, `:12994`). A "User" menu runs accessory programs (`Ed_AutoLoad` table `+Editor_Config.s:65-252`, program paths `:1051-1102`).

## B.4 Keyboard shortcuts / function dispatch

* `Ed_Key` (`+Edit.s:1559`): plays a macro if one is active, then reads `SyCall Inkey` (a long: ASCII | scancode<<16 | shifts<<24). It records macros if needed, then calls `Ed_Ky2Fonc` (`:1690`).
* `Ed_Ky2Fonc` matches against the `Ed_KFonc` table from the config (`+Editor_Config.s:253-441`). Entries are `(key, shifts, 0)`:
  * key `$80|scancode` or an ASCII letter (case-insensitive);
  * shifts `Shf/Ctr/Alt/Ami`.
* A match calls `Ed_FCall` (`:2568`), which jumps through **`JFonc`** (`:3150`, about 184 functions, flags in `FlagFonc` `:3348`, `HiddenCommands=184`). Otherwise `Ed_PKey` (`:1794`) inserts or overwrites the character in the current line buffer (max 250 chars).
* Defaults (scancodes: `$4C` up, `$4D` down, `$4F` left, `$4E` right, `$50-$59` F1-F10, `$5F` Help, `$45` Esc, `$41` BS, `$46` Del, `$42` Tab):

| Key | Function |
|---|---|
| arrows | cursor |
| Shift+arrows | page top/bottom, word left/right |
| Ctrl+arrows | page up/down, line start/end |
| Ctrl+Shift+Up/Down | text top/bottom |
| Alt+Up/Down | previous/next label |
| Return | 19 |
| BS / Del | backspace / delete |
| Shift+BS / Shift+Del | delete word |
| Ctrl+BS | delete to start of line |
| Ctrl+Del | delete to end of line |
| Ctrl+Q | clear line |
| Ctrl+Y | delete line |
| F10 | insert line |
| Tab / Shift+Tab / Ctrl+Tab | add tab / remove tab / set tab |
| Help | user menu / help |
| Esc | Direct mode |
| **F1 Run, F2 Test, F3 Indent, F4 Monitor**, F5 Help | |
| F6/F7 | previous/next window |
| F8 | insert mode |
| F9 | open/close procedure |
| Amiga+L/S/Shift+S | load / save / save as |
| Amiga+Shift+L | open+load |
| Amiga+Q / Amiga+Shift+Q | new / close |
| Amiga+H | hide |
| Amiga+F/N/B | search / next / previous |
| Amiga+Shift+F/N/B | replace / next / previous |
| Amiga+G | goto line |
| Amiga+I | info |
| Amiga+M / Amiga+Shift+M | merge / merge ASCII |
| Amiga+P | print |
| Amiga+Shift+I | check 1.3 |
| Amiga+C | link cursor |
| Amiga+Shift+V | split |
| Amiga+K | show keys |
| Amiga+U / Amiga+Shift+U | add/delete user menu |
| Ctrl+B / Ctrl+F | block on/off / forget block |
| Ctrl+C / Ctrl+P | cut / paste |
| Ctrl+S | store block |
| Ctrl+A | all text as block |
| Ctrl+U / Ctrl+Shift+U | undo / redo |
| Ctrl+K | recall alert |
| Ctrl+M | new macro |
| Ctrl+Shift+0-9 | set mark |
| Ctrl+0-9 | goto mark |
| Ctrl+Shift+A / Ctrl+Shift+S | save block ASCII / save block |
| Ctrl+Shift+P | print block |
| Amiga+Shift+T | text buffer |

* Keyboard macros (`EdMa_*`) record and replay 3-byte key events.
* Multi-level undo/redo (`:1897`) stores, per line, both the old and the new ASCII text.

## B.5 Line model, tokenizing and detokenizing

* **Program storage**: the program stays tokenized in memory (`Prg_*` struct, `a6`).
* **Edit buffer**: the editor keeps a per-window buffer of detokenized ASCII lines, 256 bytes per screen row:
  * word length + text;
  * byte 255 = "not editable" flag (e.g. folded procedures, `L_Tk_EditL`).
  * `Ed_BufUntok` (`+Edit.s:10846`) detokenizes the whole visible page.
* Typing edits only the ASCII buffer and sets `Edt_LEdited`. When the cursor leaves the line (or any command runs), **`Ed_TokCur` / `Ed_TokStok2`** (`+Edit.s:10730-10842`):
  1. NUL-terminates the ASCII line;
  2. calls **`Tokenise`** (`+Edit.s:14227`) into `Ed_BufT`;
  3. inserts or replaces the line in the program with `Ed_Stocke` (`:10952`);
  4. **re-detokenizes** the stored line back into the buffer with `Detok` (normalized case, spacing and indentation);
  5. redraws it and records undo.
* **Tokenizer** (`Tokenise`, `+Edit.s:14227-14735`; tables built by `Tok_Init` `:14091`). A copy exists in `+CLib.s:564` for the compiler.
  * Output line format: `byte len_in_words, byte indent(=leading spaces+1, max 127), tokens..., word 0`.
  * Leading digits become a line-number label (`_TkVar` / `_TkLab`). A leading `'` becomes `_TkRem2`.
  * Strings become `_TkCh1`/`_TkCh2` + word length + bytes, padded to even.
  * Variables, labels and procedure names become `_TkVar` (or `_TkLab` if followed by `:` at the line start) + long 0 (filled by the verifier) + byte name length + byte type flag (0 int, 1 `#` float, 2 `$` string) + lowercase name, padded to even.
  * Numbers are parsed by `L_ValRout` into int/float/double/hex/bin constant tokens.
  * Keywords: case-insensitive **longest-match** search over the main token table and each extension's table. A per-first-letter fast index lives in `AdTTokens[26]`. Operators are tried first (`Dtk_Operateurs` `:15183`, negative tokens).
    * A main-library token value = **byte offset of the keyword entry within the main token table** (`AdTokens`).
    * Extension keywords become `_TkExt`, byte ext#, byte 0, word offset-in-extension-table (`TkKtE` `:14608`).
  * `?` maps to Print. Special handling exists for Rem, Else/Then followed by a line number (`_TkLGo`), Data, procedures and structure tokens.
* **Detokenizer** (`Detok` `+Edit.s:14744`; `Mon_Detok` `:14741` for the Monitor) reverses this. It also returns the X position of a given token address, used to place the cursor on errors. Keyword case comes from config `DtkMaj1/2`.
* `TInst` (`:15104`) is a token-instruction helper exported for the Lib.

## B.6 Run / Test / Direct mode: handing control to the interpreter

* **Run (F1)** `Ed_Run` (`+Edit.s:8166`):
  1. `Ed_TokCur`; free the block, undos and menus (`EdM_Program`);
  2. clear the command line;
  3. call `JJsr L_Prg_RunIt` with `a1 = Ed_ErrRun` (return/error vector) and `a2 = Ed_TestMessage` (progress hooks).
* `Prg_RunIt` (`+Verif.s:4336`):
  1. pushes the current program context;
  2. switches to the program's banks and calls `Bnk.Change` (extensions such as music see the program's banks);
  3. `ClearVar`;
  4. **`PTest`**, the test-time verifier (`+Verif.s:73`): checks structure, resolves labels and procedures, allocates variables. It reports "test-time" errors;
  5. `DefRun1`/`DefRun2` (Default: reset screens, call extension DEFAULT hooks, including the music `MusDef`);
  6. `JJmp L_New_ChrGet` (`+ILib.s:430`), the interpreter main loop. **It never returns.**
* On END, an error, Ctrl-C or EDIT, the interpreter jumps to `Prg_JError` (`Ed_ErrRun` `+Edit.s:8253`) with `d0` = error number. Special values:
  * `10` END
  * `1000` Edit
  * `1001` Direct
  * `1002` return to Workbench
  * negative = test error
* `Ed_ErrRun` reopens or brings forward the editor (`Ed_OpenEditor`, `Ed_Appear`), places the cursor on `VerPos` (the error position, via `Ed_FindA` and `Detok`) and shows an alert. Or it asks Edit/Direct (`Ed_Ligne`).
* **Test (F2)** `Ed_Test` (`:8425`) uses `Prg_TestIt`: verify only, then pull the context back. **Indent (F3)** `:8459`.
* Hidden programs and accessories run the same way with `d0=1` (`Ed_RunHidden` `:8106`). Accessories can drive the editor through the "remote control" API (`Ed_ZapIn` `:2650`, `Ed_ZapFonction` `:2798`; e.g. function 12 = `EdZ_Token`, which tokenizes a line).
* **Direct mode** (Esc): `Ed_Escape` (`:8877`), `Esc_Appear` (`:9330`), `Esc_Loop` (`:8889`).
  * The escape screen is a resizable strip: `EcFonc` with a logo, 13 buttons and memory sliders, plus a text area on `EcEdit` positioned over the program's output screens (`Es_Y1/Es_Y2`, drag to move or resize).
  * Line input uses the Lib's line editor `L_LEd_Init/L_LEd_Loop` (160 chars, `*` cursor, history `Esc_KMem` up to `Esc_KMemMax=20`, recalled with Up/Down).
  * F1-F10 / buttons insert preset commands from config strings 24-43 (`ListBank`, `Default`, `Dir`, `Load Iff`, `Screen Open`...).
  * On Return (`Esc_R` `:9185`):
    1. **Tokenise** the line;
    2. switch to the current program's banks;
    3. `JJsr L_VerDirect` (`+Verif.s:43`): verify the direct line against the *last run program's* variables and procedures;
    4. activate the program's current screen;
    5. `JJmp L_New_ChrGet`.
  * Completion or an error returns via `Prg_JError = Ed_ErrDirect` (`:9294`) to `Esc_Loop`.
  * Esc again returns to the editor.

## B.7 `+Monitor.s` (what it is)

* The AMOS Pro **Monitor**: an interactive source-level debugger. It is a separately loaded segment `APSystem/AMOSPro_Monitor` with resource `AMOSPro_Monitor_Resource.Abk`.
* Entry table `MDebut` (`+Monitor.s:33-37`):
  * `In_Editor`: from the editor, F4 (function 145) on the current program;
  * `In_Program`: the `Monitor` instruction inside a running program;
  * `MonitorChr`: a per-instruction hook installed into ChrGet.
* It uses EcEdit (640x116, the listing/info area) and EcFonc (button panel) plus an Interface ("DBL") channel. It shows the program listing (detokenized with `Mon_Detok`), the current line, an information/evaluation area, and a view of the program's screens.
* Buttons (`Tt_Bra` `:290-306`):
  * Stop
  * single step
  * three run speeds (semi-fast, fast, ultra-fast)
  * scroll
  * change screen
  * Init
  * Quit
  * view output
  * Help, Break and Eval (set breakpoint / evaluate expression)
* It runs the program through `L_Prg_RunIt` (`:1936`) with error trapping to `Tt_Error`.
* For a Rust port it is a debugger UI built on the same primitives (detok, ChrGet hook, Interface). It is optional and can come late.

## B.8 Compiler: `+APComp.s` and `+CLib.s` (port or not?)

* **`+APComp.s`** (12k lines) is the AMOS Pro Compiler (`APSystem/APCmp`, driven by `Compiler_Shell.AMOS`/`Tiny_Shell.AMOS`).
  * It loads and tokenises the source (ASCII or tokenized), runs the test pass, then walks the tokens (`ChrGet`-style loop `+APComp.s:690-720`).
  * For each instruction it emits **native MC68000 code**: parameter-evaluation code plus `JSR` (`$4EB9`, `:2114`) calls into library routines. Special instructions (`Inst_Jumps`) get inline code for loops, Dim, Read, Swap, Add, Inc/Dec...
  * It **copies the needed routines from `AMOSPro.Lib`, extension `.Lib`s and `Compiler.Lib`** into the object, relocating `Rbsr`/`Rjsr` (the reason the extension sources have the `Lib_Def`/`Rbsr` macro conventions).
  * Output is an **AmigaDOS hunk executable** (`DebHunk`/`FinHunk`; code, data, chip, mouse/library hunks `:613-1080`), or a compiled AMOS program that runs under the interpreter.
* **`+CLib.s`** is the compiler's internal runtime library (`Compiler.Lib`, `Lib_Cmp` routines):
  * memory and temp buffers, includes handling;
  * a second copy of the **tokenizer** (`:564`) and **detokenizer** (`Detok` `:5436`, `TInst` `:5796`) so the compiler can read ASCII source;
  * runtime glue linked into compiled programs.
* **Verdict:** both are tied to 68k code generation and the Amiga hunk format, so there is nothing to port for a cross-platform Rust implementation. What is reusable conceptually is the tokenizer (already in `+Edit.s`) and the token/library-routine mapping. A Rust "compiler" would instead be a bytecode or AOT design of its own. Optionally support *loading* AMOS-compiled `.AMOS` programs? Those contain 68k code, so no.
