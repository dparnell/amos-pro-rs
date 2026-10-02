//! The Music extension's state (`MB`, `+Music.s:2085`) and its routines:
//! waves, envelopes, Play/Bell/Boom/Shoot, sample playback with the audio
//! interrupt handler, voice sharing with the music, and the VBL routine
//! (`MusInt`). The AMOS music player is in `music.rs`, the tracker in
//! `tracker.rs`.
//!
//! Everything here runs inside the [`super::Mixer`] lock: the VBL routine
//! is called by the mixer every 1/50 s of produced audio, the interpreter
//! calls the instruction entry points.

use std::sync::Arc;

use super::music::{MuState, MusicBank};
use super::paula::{PAL_CLOCK, Paula, Ptr};
use super::tables::*;
use super::tracker::Tracker;

/// Error numbers of the extension (`+Music.s:4757`).
pub const ERR_WAVE_NOT_DEFINED: u16 = 178;
pub const ERR_SAMPLE_NOT_DEFINED: u16 = 179;
pub const ERR_SAMPLE_BANK_NOT_FOUND: u16 = 180;
pub const ERR_WAVE_TOO_SHORT: u16 = 181;
pub const ERR_WAVES_RESERVED: u16 = 182;
pub const ERR_MUSIC_BANK_NOT_FOUND: u16 = 183;
pub const ERR_MUSIC_NOT_DEFINED: u16 = 184;
pub const ERR_NOT_TRACKER: u16 = 186;
pub const ERR_IFONC: u16 = 23;

pub type SResult<T> = Result<T, u16>;

/// Sample streaming chunk size (`Sami_lplay`).
pub const SAMI_LPLAY: u32 = 4096;

/// A wave: 256 byte cycle plus its decimated copies, and its envelope.
#[derive(Clone, Debug)]
pub struct Wave {
    pub nb: i16,
    /// `WaveEnv`: 16 (duration, level) pairs, 0 duration ends.
    pub env: [i16; 34],
    /// `WaveDeb` (unused for wave 0, whose data is the noise buffer).
    pub data: Arc<[u8]>,
}

/// An envelope in progress (`EnvBase`, `+Music.s:125`).
#[derive(Clone, Debug)]
pub struct Env {
    pub nb: u16,
    /// `Volume` of the voice (0..63).
    pub dvol: u16,
    /// 16.16 fixed point volume.
    pub vol: i32,
    pub delta: i32,
    /// Copy of the (duration, level) table and index of the next phase.
    pub table: [i16; 34],
    pub ad: usize,
}

impl Default for Env {
    fn default() -> Self {
        Env {
            nb: 0,
            dvol: 0,
            vol: 0,
            delta: 0,
            table: [0; 34],
            ad: 0,
        }
    }
}

/// Per voice state of the sample interrupt handler (`Sami_int`).
#[derive(Clone, Debug, Default)]
pub struct Sami {
    pub adr: Ptr,
    pub long: u32,
    pub pos: u32,
    pub rpos: i32,
    pub radr: Option<Ptr>,
    pub rlong: u32,
    /// Volume written at each chunk, -1 when an envelope drives it.
    pub dvol: i16,
}

/// A sample ready to play: data, length in bytes, frequency in Hz.
#[derive(Clone, Debug)]
pub struct SampleData {
    pub ptr: Ptr,
    pub len: u32,
    pub freq: u32,
}

/// The whole sound state behind the mixer.
#[derive(Clone, Debug)]
pub struct Engine {
    pub hw: Paula,
    pub clock: u32,
    pub tempo_base: u16,
    pub mu_vu: [u8; 4],
    /// Wave list in creation order (`WaveBase`): wave 0 (noise) first.
    pub waves: Vec<Wave>,
    /// `Waves`: per voice, >0 wave, 0 noise, <0 -sample number.
    pub wave_sel: [i16; 4],
    pub env_on: u16,
    pub env: [Env; 4],
    pub sam_bank: u16,
    pub seed: u16,
    pub noise: u16,
    pub pnoise: u16,
    /// Fake beam position used by the random generator (`VHPOSR`).
    pub beam: u16,
    // AMOS music.
    pub music: Option<MusicBank>,
    pub mu_number: u16,
    pub mu_volume: u16,
    pub mu_dmask: u16,
    pub mu_restart: u16,
    /// `MuChip0-3`: false when the voice is redirected to a dummy area
    /// because an effect uses it.
    pub mu_chip: [bool; 4],
    pub mu_buffer: [MuState; 3],
    /// `MuBase`: index of the running music in `mu_buffer`.
    pub mu_base: Option<usize>,
    // Samples.
    pub sami: [Sami; 4],
    pub sami_bits: u16,
    pub sam_loops: u16,
    // Tracker.
    pub tracker: Tracker,
    /// Number of 50 Hz ticks done (tests, sync).
    pub ticks: u64,
}

impl Default for Engine {
    fn default() -> Self {
        let mut e = Engine {
            hw: Paula::default(),
            clock: PAL_CLOCK,
            tempo_base: 100,
            mu_vu: [0; 4],
            waves: Vec::new(),
            wave_sel: [1; 4],
            env_on: 0,
            env: Default::default(),
            sam_bank: 5,
            seed: 0x1234,
            noise: 0,
            pnoise: 0,
            beam: 0,
            music: None,
            mu_number: 0,
            mu_volume: 56,
            mu_dmask: 0xF,
            mu_restart: 0,
            mu_chip: [true; 4],
            mu_buffer: Default::default(),
            mu_base: None,
            sami: Default::default(),
            sami_bits: 0,
            sam_loops: 0,
            tracker: Tracker::default(),
            ticks: 0,
        };
        e.mus_def();
        e
    }
}

/// `Div32` and `divu` helpers.
fn div32(a: u32, b: u32) -> u32 {
    a.checked_div(b).unwrap_or(u32::MAX)
}

impl Engine {
    /// Raster position read by the random generator: any changing value.
    fn vhposr(&mut self) -> u16 {
        self.beam = self.beam.wrapping_mul(25173).wrapping_add(13849);
        self.beam
    }

    /// One step of the noise generator (`+Music.s:886`).
    fn rnd(&mut self) -> u16 {
        let s = self.seed.wrapping_add(self.vhposr());
        self.seed = ((s as u32 * 0x3171) >> 8) as u16;
        self.seed
    }

    /// `MusDef`: state after Default / Run (`+Music.s:857`).
    pub fn mus_def(&mut self) {
        self.tracker_stop();
        self.tracker.looping = false;
        self.hw.dmacon(0x000F);
        self.sami_install();
        self.raz_wave();
        let square: Vec<u8> = (0..256)
            .map(|i| if i < 128 { 127u8 } else { (-127i8) as u8 })
            .collect();
        self.ne_wave(0, &square);
        self.ne_wave(1, &square);
        for i in 0..LNOISE / 2 {
            let w = self.rnd();
            self.hw.noise[i * 2..i * 2 + 2].copy_from_slice(&w.to_be_bytes());
        }
        let _ = self.vol(56, 0xF);
        self.mvol(56);
        self.sam_bank = 5;
        self.sl0(-1, 0xF);
        self.mu_init();
    }

    /// `Sami_install` / `Sami_stop`: no audio interrupt enabled or pending.
    fn sami_install(&mut self) {
        self.hw.intena(0x0780);
        self.hw.intreq(0x0780);
    }

    // ------------------------------------------------------------------
    // Waves

    pub fn wave_index(&self, nb: i16) -> Option<usize> {
        self.waves.iter().position(|w| w.nb == nb)
    }

    /// `RazWave`: stops envelopes and erases every wave.
    fn raz_wave(&mut self) {
        self.env_off(0xF);
        self.waves.clear();
        self.no_wave();
    }

    /// `NoWave`: every voice back to wave 1.
    fn no_wave(&mut self) {
        self.wave_sel = [1; 4];
    }

    /// `NeWave`: (re)defines wave `nb` from 256 signed bytes.
    pub fn ne_wave(&mut self, nb: i16, cycle: &[u8]) {
        while let Some(i) = self.wave_index(nb) {
            self.env_off(0xF);
            self.waves.remove(i);
        }
        let mut env = [0i16; 34];
        // The default envelope is copied by longs up to the first 0 long.
        env[..10].copy_from_slice(&ENV_DEF);
        let mut data = vec![0u8; LWAVE];
        data[..256].copy_from_slice(&cycle[..256]);
        // Decimated copies: each byte is (a+b)>>1 of two bytes of the
        // previous level (`NewRout`).
        let (mut src, mut dst, mut n) = (0usize, 256usize, 128usize);
        while n >= 4 {
            for i in 0..n {
                let a = data[src + i * 2] as i8 as i16;
                let b = data[src + i * 2 + 1] as i8 as i16;
                data[dst + i] = ((a + b) >> 1) as i8 as u8;
            }
            src = dst;
            dst += n;
            n /= 2;
        }
        self.waves.push(Wave {
            nb,
            env,
            data: data.into(),
        });
    }

    /// `Set Wave n,a$`.
    pub fn set_wave(&mut self, nb: i32, shape: &[u8]) -> SResult<()> {
        if shape.len() < 256 {
            return Err(ERR_WAVE_TOO_SHORT);
        }
        if nb == 0 {
            return Err(ERR_IFONC);
        }
        self.ne_wave(nb as i16, shape);
        Ok(())
    }

    /// `Del Wave n`.
    pub fn del_wave(&mut self, nb: i32) -> SResult<()> {
        if nb < 0 {
            return Err(ERR_IFONC);
        }
        if nb <= 1 {
            return Err(ERR_WAVES_RESERVED);
        }
        self.env_off(0xF);
        let i = self.wave_index(nb as i16).ok_or(ERR_WAVE_NOT_DEFINED)?;
        self.waves.remove(i);
        self.no_wave();
        Ok(())
    }

    /// `Set Envel w,phase To duration,level`.
    pub fn set_envel(&mut self, w: i32, phase: i32, dur: i32, level: i32) -> SResult<()> {
        if !(0..64).contains(&level) || !(0..7).contains(&phase) || w < 0 {
            return Err(ERR_IFONC);
        }
        if phase == 0 && dur as i16 == 0 {
            return Err(ERR_IFONC);
        }
        let i = self.wave_index(w as i16).ok_or(ERR_WAVE_NOT_DEFINED)?;
        let p = phase as usize * 2;
        let env = &mut self.waves[i].env;
        env[p] = dur as i16;
        env[p + 1] = level as i16;
        env[p + 2] = 0;
        Ok(())
    }

    /// `ISmt`: sets the instrument of voices (`Wave n To`, `Noise To`,
    /// `Sample n To`).
    pub fn set_voice_wave(&mut self, sel: i16, voices: u16) {
        for v in 0..4 {
            if voices & (1 << v) != 0 {
                self.wave_sel[v] = sel;
            }
        }
    }

    /// `Wave n To voices`.
    pub fn wave_to(&mut self, nb: i32, voices: i32) -> SResult<()> {
        if nb < 0 {
            return Err(ERR_IFONC);
        }
        self.wave_index(nb as i16).ok_or(ERR_WAVE_NOT_DEFINED)?;
        self.set_voice_wave(nb as i16, voices as u16);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Envelopes

    /// `EnvOff`: stops the envelopes of `voices`; the voices go back to the
    /// music.
    pub fn env_off(&mut self, voices: u16) {
        let mut on = self.env_on;
        let mut restart = 0;
        for v in 0..4 {
            let bit = 1 << v;
            if voices & bit != 0 && on & bit != 0 {
                on &= !bit;
                restart |= bit;
                self.hw.ch[v].len = 2;
                self.hw.ch[v].vol = 0;
            }
        }
        self.env_on = on;
        // Overwrites, as the original does.
        self.mu_restart = restart;
    }

    /// `MuIntE`: loads the next envelope phase of voice `v`. Returns true
    /// when the envelope ended (the caller stops the voice).
    fn mu_int_e(&mut self, v: usize) -> bool {
        let mut loops = 0;
        loop {
            let e = &mut self.env[v];
            let dur = e.table.get(e.ad).copied().unwrap_or(0);
            if dur == 0 || loops > 1 {
                // MuIntS
                self.noise &= !(1 << v);
                self.mu_restart |= 1 << v;
                return true;
            }
            if dur < 0 {
                e.ad = 0;
                loops += 1;
                continue;
            }
            let level = e.table.get(e.ad + 1).copied().unwrap_or(0);
            e.ad += 2;
            e.nb = dur as u16;
            let target = (((e.dvol as u32 * level as u16 as u32) as u16) >> 6) as i16;
            let diff = target.wrapping_sub((e.vol >> 16) as i16);
            // ext.l + lsl.w #8: the high word keeps the sign extension.
            let hi = if diff < 0 { 0xFFFF_0000u32 } else { 0 };
            let d4 = (hi | (((diff as u16) << 8) as u32)) as i32;
            let q = d4 / dur as i32;
            let q = if (i16::MIN as i32..=i16::MAX as i32).contains(&q) {
                q as i16
            } else {
                d4 as i16
            };
            e.delta = (q as i32) << 8;
            e.vol &= !0xFFFF;
            return false;
        }
    }

    fn start_env(&mut self, v: usize, table: &[i16]) {
        let e = &mut self.env[v];
        e.table = [0; 34];
        let n = table.len().min(34);
        e.table[..n].copy_from_slice(&table[..n]);
        e.ad = 0;
        e.vol = 0;
        self.mu_int_e(v);
    }

    // ------------------------------------------------------------------
    // Volume / voices

    /// `Vol`: sets the `Volume` of voices.
    pub fn vol(&mut self, level: i32, voices: u16) -> SResult<()> {
        if !(0..64).contains(&level) {
            return Err(ERR_IFONC);
        }
        for v in 0..4 {
            if voices & (1 << v) != 0 {
                self.env[v].dvol = level as u16;
                if self.sami[v].dvol >= 0 {
                    self.sami[v].dvol = level as i16;
                }
            }
        }
        Ok(())
    }

    /// `StopDma`: stops the DMA and interrupts of the voices NOT in `keep`.
    fn stop_dma(&mut self, keep: u16) {
        let d0 = (keep ^ 0xF) & 0xF;
        self.hw.dmacon(d0);
        self.hw.intena(d0 << 7);
    }

    /// `VOnOf`: voices the music may use (only acts while music plays).
    pub fn v_on_of(&mut self, mask: u16) {
        let Some(base) = self.mu_base else { return };
        let old = self.mu_dmask;
        self.mu_dmask = mask;
        let mut stop = 0;
        for v in 0..4 {
            let bit = 1 << v;
            if mask & bit == 0 {
                if old & bit != 0 {
                    stop |= bit;
                    self.mu_chip[v] = false;
                    self.mu_buffer[base].start &= !bit;
                    self.mu_buffer[base].stop &= !bit;
                }
            } else if old & bit == 0 {
                self.mu_restart |= bit;
            }
        }
        self.hw.dmacon(stop);
    }

    // ------------------------------------------------------------------
    // Play / Bell / Boom / Shoot

    /// `GoBel`: Play / Bell on `voices`. `forced` is the wave forced by
    /// Bell (1), or -1 to use each voice's instrument. `samples[v]` must be
    /// resolved for voices playing a sample (`Sample n To`).
    pub fn go_bel(
        &mut self,
        voices: u16,
        note: i32,
        forced: i16,
        fixed: Option<&[i16]>,
        samples: &[Option<SampleData>; 4],
    ) -> SResult<()> {
        if !(0..=96).contains(&note) {
            return Err(ERR_IFONC);
        }
        let keep = (voices ^ 0xF) & 0xF;
        self.stop_dma(keep);
        self.v_on_of(keep);
        self.go_shot(voices, note as u16, forced, fixed, samples)
    }

    /// `Shout`: Boom / Shoot, a stereo effect on the 4 voices.
    pub fn shout(&mut self, note: u16, env: &[i16]) {
        self.stop_dma(0);
        self.v_on_of(0);
        let none = Default::default();
        for i in 0..4u16 {
            let _ = self.go_shot(8 >> i, note + i, 0, Some(env), &none);
        }
    }

    pub fn boom(&mut self) {
        self.shout(36, &ENV_BOOM);
    }

    pub fn shoot(&mut self) {
        self.shout(60, &ENV_SHOOT);
    }

    pub fn bell(&mut self, note: i32) -> SResult<()> {
        self.go_bel(0xF, note, 1, Some(&ENV_BELL), &Default::default())
    }

    /// `GoShot`: starts the voices.
    fn go_shot(
        &mut self,
        voices: u16,
        note: u16,
        forced: i16,
        fixed: Option<&[i16]>,
        samples: &[Option<SampleData>; 4],
    ) -> SResult<()> {
        let saved_env = self.env_on;
        let mut d7 = self.env_on;
        self.env_on = 0;
        self.sami_bits = 0x8000;
        let mut d0 = 0u16;
        for v in (0..4).rev() {
            if voices & (1 << v) != 0
                && let Err(e) = self.vplay(
                    v,
                    note,
                    forced,
                    fixed,
                    samples[v].as_ref(),
                    &mut d0,
                    &mut d7,
                )
            {
                self.env_on = saved_env;
                return Err(e);
            }
        }
        self.hw.dmacon(d0 | 0x8000);
        self.hw.intena(self.sami_bits);
        self.env_on = d7;
        self.service_irqs();
        Ok(())
    }

    /// `VPlay`: plays a note on voice `v`.
    #[allow(clippy::too_many_arguments)]
    fn vplay(
        &mut self,
        v: usize,
        note: u16,
        forced: i16,
        fixed: Option<&[i16]>,
        sample: Option<&SampleData>,
        d0: &mut u16,
        d7: &mut u16,
    ) -> SResult<()> {
        let bit = 1u16 << v;
        self.hw.dmacon(bit);
        self.hw.intena(bit << 7);
        self.noise &= !bit;
        if note == 0 {
            // VSil
            self.hw.dmacon(bit);
            *d7 &= !bit;
            return Ok(());
        }
        let d2 = note + 3;
        let sel = if forced >= 0 {
            forced
        } else {
            self.wave_sel[v]
        };
        if forced < 0 && sel < 0 {
            // VPl2: sample at its frequency * note / 440.
            let s = sample.ok_or(ERR_SAMPLE_NOT_DEFINED)?;
            let freq = Self::note_freq(s.freq, d2);
            return self.spl0(v, s.ptr.clone(), s.len, freq, None, d0, d7);
        }
        if sel == 0 {
            // VPl4: the noise buffer as a sample at 2000 Hz * note / 440.
            self.noise |= bit;
            let env = match fixed {
                Some(t) => (t.to_vec(), false),
                None => (
                    self.waves
                        .first()
                        .map_or(ENV_DEF.to_vec(), |w| w.env.to_vec()),
                    true,
                ),
            };
            let freq = Self::note_freq(2000, d2);
            return self.spl0(v, Ptr::noise(0), LNOISE as u32, freq, Some(env), d0, d7);
        }
        // VPl0: a wave.
        let wi = self.wave_index(sel).ok_or(ERR_WAVE_NOT_DEFINED)?;
        let idx = (d2 - 1) as usize;
        let (off, len) = TFREQ[(idx / 12).min(8)];
        let ch = &mut self.hw.ch[v];
        ch.lc = Ptr::new(&self.waves[wi].data, off as u32);
        ch.len = len;
        let d3 = (len as u32 * 2) * TNOTES[idx.min(99)] as u32;
        let per = (self.clock / d3.max(1)).max(124);
        ch.per = per as u16;
        let table: Vec<i16> = match fixed {
            Some(t) => t.to_vec(),
            None => self.waves[wi].env.to_vec(),
        };
        self.start_env(v, &table);
        *d0 |= bit;
        *d7 |= bit;
        Ok(())
    }

    /// `freq * TNotes[note+2] / 440` with the 68000 `divu` overflow rule.
    fn note_freq(freq: u32, d2: u16) -> u32 {
        let prod = (freq & 0xFFFF) * TNOTES[(d2 as usize - 1).min(99)] as u32;
        let q = prod / 440;
        if q > 0xFFFF { prod & 0xFFFF } else { q }
    }

    /// `SPl0`: starts a sample on voice `v`, optionally with an envelope
    /// (table, loop flag).
    #[allow(clippy::too_many_arguments)]
    fn spl0(
        &mut self,
        v: usize,
        ptr: Ptr,
        len: u32,
        freq: u32,
        env: Option<(Vec<i16>, bool)>,
        d0: &mut u16,
        d7: &mut u16,
    ) -> SResult<()> {
        let bit = 1u16 << v;
        self.hw.dmacon(bit);
        self.hw.intena(bit << 7);
        self.hw.ch[v].len = 1;
        let s = &mut self.sami[v];
        s.adr = ptr;
        s.long = len;
        s.pos = 0;
        s.rpos = if self.sam_loops & bit != 0 { 0 } else { -1 };
        s.radr = None;
        let per = div32(self.clock, freq).max(124);
        self.hw.ch[v].per = per as u16;
        *d7 &= !bit;
        self.sami[v].dvol = self.env[v].dvol as i16;
        if let Some((table, looping)) = env {
            self.sami[v].rpos = if looping { 0 } else { -1 };
            *d7 |= bit;
            self.sami[v].dvol = -1;
            self.start_env(v, &table);
        }
        self.sami_bits |= bit << 7;
        self.sami_handler(v);
        *d0 |= bit;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Samples

    /// `Sami_handler`: audio interrupt of voice `v`, feeds the next chunk.
    pub fn sami_handler(&mut self, v: usize) {
        let pos = self.sami[v].pos;
        if pos < self.sami[v].long {
            self.sam_chunk(v, pos);
            return;
        }
        // .whatnow
        if let Some(radr) = self.sami[v].radr.take() {
            let s = &mut self.sami[v];
            s.adr = radr;
            s.long = s.rlong;
            self.sam_chunk(v, 0);
            return;
        }
        if self.sami[v].rpos >= 0 {
            self.sam_chunk(v, self.sami[v].rpos as u32);
            return;
        }
        self.sami[v].pos |= 0x8000_0000;
        if (pos as i32) >= 0 {
            // Let the last chunk play.
            return;
        }
        // Sample finished: silence, and give the voice back to the music.
        let bit = 1u16 << v;
        self.hw.dmacon(bit);
        self.hw.ch[v].dat = 0;
        if self.sami[v].dvol >= 0 {
            self.mu_restart |= bit;
        }
        self.hw.intena(bit << 7);
    }

    fn sam_chunk(&mut self, v: usize, d0: u32) {
        let s = &mut self.sami[v];
        let d1 = d0.saturating_add(SAMI_LPLAY).min(s.long);
        s.pos = d1;
        let ch = &mut self.hw.ch[v];
        ch.lc = s.adr.add(d0);
        ch.len = ((d1 - d0) >> 1) as u16;
        if s.dvol >= 0 {
            ch.vol = s.dvol as u16;
        }
    }

    /// Runs the pending audio interrupts.
    pub fn service_irqs(&mut self) {
        let mut guard = 0;
        while let Some(c) = self.hw.take_irq() {
            self.sami_handler(c);
            guard += 1;
            if guard > 64 {
                break;
            }
        }
    }

    /// `GoSam`: plays a sample on `voices` (Sam Play / Sam Raw, and the
    /// editor sounds).
    pub fn go_sam(&mut self, voices: u16, s: &SampleData) {
        let keep = (voices ^ 0xF) & 0xF;
        self.stop_dma(keep);
        self.v_on_of(keep);
        self.start_samples(voices, s);
    }

    /// Body of `GoSam`: starts `s` on `voices`.
    fn start_samples(&mut self, voices: u16, s: &SampleData) {
        let saved = self.env_on;
        let mut d7 = saved;
        self.env_on = 0;
        self.sami_bits = 0x8000;
        let mut d0 = 0;
        for v in (0..4).rev() {
            if voices & (1 << v) != 0 {
                // SPlay
                self.noise &= !(1 << v);
                let _ = self.spl0(v, s.ptr.clone(), s.len, s.freq, None, &mut d0, &mut d7);
            }
        }
        self.hw.dmacon(d0 | 0x8000);
        self.hw.intena(self.sami_bits);
        self.env_on = d7;
        self.service_irqs();
    }

    /// `Say`: the narrator takes all the voices (`+Music.s:2521`).
    pub fn say_start(&mut self) {
        self.stop_dma(0);
        self.v_on_of(0);
        self.env_on = 0;
        self.noise = 0;
    }

    /// Plays synthesized speech on voices 0 and 1 (narrator channel map 3).
    pub fn say_play(&mut self, s: &SampleData, volume: u16) {
        self.start_samples(0b0011, s);
        for v in 0..2 {
            self.sami[v].dvol = volume as i16;
            self.hw.ch[v].vol = volume;
        }
    }

    /// `SL0`: `Sam Loop On/Off`.
    pub fn sl0(&mut self, rpos: i32, voices: u16) {
        for v in 0..4 {
            let bit = 1 << v;
            if voices & bit != 0 {
                self.sam_loops &= !bit;
                self.sami[v].rpos = rpos;
                if rpos == 0 {
                    self.sam_loops |= bit;
                }
            }
        }
    }

    /// `Sam Swap voices To address,length`.
    pub fn sam_swap(&mut self, voices: u16, ptr: Option<Ptr>, len: u32) {
        for v in 0..4 {
            if voices & (1 << v) != 0 {
                self.sami[v].radr = ptr.clone();
                self.sami[v].rlong = len;
            }
        }
    }

    /// `=Sam Swapped(v)`.
    pub fn sam_swapped(&self, v: usize) -> i32 {
        if self.hw.intena & (0x80 << v) == 0 {
            1
        } else if self.sami[v].radr.is_some() || self.sami[v].pos == SAMI_LPLAY {
            0
        } else {
            -1
        }
    }

    /// `Sam Stop [voices]`.
    pub fn sam_stop(&mut self, voices: u16) {
        let v = voices & 0xF;
        self.hw.dmacon(v);
        self.hw.intena(v << 7);
    }

    // ------------------------------------------------------------------
    // Music support routines

    /// `MOff`: stops the music voices.
    pub fn m_off(&mut self) {
        self.hw.intena(0x0780);
        let mask = self.mu_dmask;
        if mask == 0 {
            return;
        }
        self.hw.dmacon(mask);
        // The original tests bit 3 for channel 0, bit 2 for channel 1...
        for d1 in (0..4).rev() {
            if mask & (1 << d1) != 0 {
                let ch = &mut self.hw.ch[3 - d1];
                ch.len = 2;
                ch.vol = 0;
            }
        }
    }

    /// `MuInit`.
    pub fn mu_init(&mut self) {
        self.mu_base = None;
        self.mu_number = 0;
        self.mu_chip = [true; 4];
        self.mu_dmask = 0xF;
        self.mu_restart = 0;
        self.m_off();
    }

    /// `MVol`: music volume, applied to all nested musics.
    pub fn mvol(&mut self, level: u16) {
        let level = level & 63;
        self.mu_volume = level;
        if self.mu_base.is_some() {
            for m in self.mu_buffer.iter_mut().take(self.mu_number as usize) {
                for voice in m.voices.iter_mut() {
                    voice.vol = ((voice.dvol as u32 * level as u32) >> 6) as u16;
                }
            }
        }
    }

    /// `=Vumeter(v)`: returns and clears the VU byte.
    pub fn vumeter(&mut self, v: i32) -> SResult<i32> {
        if !(0..4).contains(&v) {
            return Err(ERR_IFONC);
        }
        let r = self.mu_vu[v as usize];
        self.mu_vu[v as usize] = 0;
        Ok(r as i32)
    }

    // ------------------------------------------------------------------
    // VBL

    /// `MusInt`: the 50 Hz interrupt routine.
    pub fn vbl(&mut self) {
        self.ticks += 1;
        if self.env_on != 0 {
            let mut d0 = self.env_on;
            let mut d5 = 0u16;
            for v in 0..4 {
                if d0 & (1 << v) == 0 {
                    continue;
                }
                let e = &mut self.env[v];
                e.vol = e.vol.wrapping_add(e.delta);
                self.hw.ch[v].vol = (e.vol >> 16) as u16;
                e.nb = e.nb.wrapping_sub(1);
                if e.nb == 0 && self.mu_int_e(v) {
                    d5 |= 1 << v;
                    d0 &= !(1 << v);
                }
            }
            self.env_on = d0;
            self.hw.dmacon(d5);
            self.hw.intena(d5 << 7);
            // Noise refresh.
            if self.noise != 0 {
                let mut p = self.pnoise as i32;
                for _ in 0..8 {
                    let w = self.rnd();
                    let i = p as usize;
                    self.hw.noise[i..i + 2].copy_from_slice(&w.to_be_bytes());
                    p -= 2;
                    if p < 0 {
                        p = LNOISE as i32 - 2;
                    }
                }
                self.pnoise = p as u16;
            }
        }
        if self.mu_base.is_some() {
            self.music_vbl();
        } else {
            self.tracker_vbl();
        }
        self.service_irqs();
    }
}
