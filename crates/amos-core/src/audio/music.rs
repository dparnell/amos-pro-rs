//! The AMOS music bank player (`Music`, `+Music.s:1111-1640`).
//!
//! Bank layout (bank 3, "Music   "), offsets from the bank data start:
//! `+0` instruments, `+4` songs, `+8` patterns. A pattern is a stream of
//! big-endian command words per voice. One "step" is done when the tempo
//! counter overflows `TempoBase`; the other VBLs run the effects.

use std::sync::Arc;

use super::engine::{ERR_IFONC, ERR_MUSIC_BANK_NOT_FOUND, ERR_MUSIC_NOT_DEFINED, Engine, SResult};
use super::paula::{Channel, Ptr};
use super::tables::{PERIODS_EXT, SINUS};

/// Address of `FoEnd`, a fake pattern holding only "end of pattern".
const FOEND: u32 = u32::MAX - 1;

/// A music bank snapshot.
#[derive(Clone, Debug)]
pub struct MusicBank {
    pub data: Arc<[u8]>,
    /// `BankInst`, `BankSong`, `BankPat` (offsets in `data`).
    pub inst: u32,
    pub song: u32,
    pub pat: u32,
}

impl MusicBank {
    pub fn new(data: Arc<[u8]>) -> Self {
        let mut b = MusicBank {
            data,
            inst: 0,
            song: 0,
            pat: 0,
        };
        b.inst = b.l(0);
        b.song = b.l(4);
        b.pat = b.l(8);
        b
    }

    pub fn w(&self, off: u32) -> u16 {
        if off == FOEND {
            return 0x8000;
        }
        let o = off as usize;
        match self.data.get(o..o + 2) {
            Some(b) => u16::from_be_bytes([b[0], b[1]]),
            None => 0,
        }
    }

    pub fn l(&self, off: u32) -> u32 {
        (self.w(off) as u32) << 16 | self.w(off.wrapping_add(2)) as u32
    }

    /// Number of songs.
    pub fn songs(&self) -> u16 {
        self.w(self.song)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    None,
    Slide,
    Arp,
    PTone,
    Vib,
    VSl,
}

/// Per voice music state (`VoiAdr`...`VoiVib`, `+Music.s:156`).
#[derive(Clone, Debug, Default)]
pub struct MuVoice {
    pub adr: u32,
    pub deb: u32,
    /// Offset of the instrument entry, 0 = none.
    pub inst: u32,
    pub dpat: u32,
    pub pat: u32,
    pub cpt: u16,
    pub rep: u16,
    pub note: u16,
    pub dvol: u16,
    pub vol: u16,
    pub effect: Effect,
    pub value: u16,
    pub ptoto: u16,
    pub ptone: u8,
    pub vib: u8,
}

/// One music in progress (`MuBuffer` entry).
#[derive(Clone, Debug, Default)]
pub struct MuState {
    pub voices: [MuVoice; 4],
    pub cpt: u16,
    pub tempo: u16,
    pub start: u16,
    pub stop: u16,
}

impl Engine {
    /// `MuChipN`: the voice's registers, or None when the music is
    /// redirected to the dummy area.
    fn chip(&mut self, v: usize) -> Option<&mut Channel> {
        if self.mu_chip[v] {
            Some(&mut self.hw.ch[v])
        } else {
            None
        }
    }

    /// `BkCheck` for the music bank: `data` is bank 3 when it is a music
    /// bank.
    pub fn music_bank_check(&mut self, data: Option<Arc<[u8]>>) {
        match data {
            None => {
                if self.music.is_some() {
                    self.mu_init();
                    self.music = None;
                }
            }
            Some(d) => {
                if self.music.as_ref().is_some_and(|m| m.data[..] == d[..]) {
                    return;
                }
                self.music = Some(MusicBank::new(d));
                self.mu_init();
            }
        }
    }

    /// `Music n`.
    pub fn music(&mut self, n: i32) -> SResult<()> {
        if n == 0 {
            return Err(ERR_IFONC);
        }
        let Some(bank) = self.music.clone() else {
            return Err(ERR_MUSIC_BANK_NOT_FOUND);
        };
        if n as u16 > bank.songs() || n < 0 {
            return Err(ERR_MUSIC_NOT_DEFINED);
        }
        let song = bank
            .song
            .wrapping_add(bank.l(bank.song + 2 + (n as u32 - 1) * 4));
        if self.mu_number >= 3 {
            return Ok(());
        }
        self.mu_base = None;
        let b = self.mu_number as usize;
        self.mu_number += 1;
        let m = &mut self.mu_buffer[b];
        *m = MuState {
            cpt: self.tempo_base,
            tempo: 17,
            ..Default::default()
        };
        for (v, voice) in m.voices.iter_mut().enumerate() {
            voice.cpt = 1;
            voice.adr = FOEND;
            let list = song.wrapping_add(bank.w(song + v as u32 * 2) as i16 as i32 as u32);
            voice.pat = list;
            voice.dpat = list;
            voice.effect = Effect::None;
        }
        // "No more samples".
        self.hw.intena(0x0780);
        self.mu_base = Some(b);
        Ok(())
    }

    /// `Music Off`.
    pub fn music_off(&mut self) {
        self.mu_base = None;
        self.mu_number = 0;
        self.m_off();
    }

    /// `Music Stop`: the current music ends at the next step.
    pub fn music_stop(&mut self) {
        if let Some(b) = self.mu_base {
            for v in self.mu_buffer[b].voices.iter_mut() {
                v.cpt = 0;
            }
        }
    }

    /// `Tempo n`.
    pub fn tempo(&mut self, t: i32) -> SResult<()> {
        if !(0..=100).contains(&t) {
            return Err(ERR_IFONC);
        }
        if let Some(b) = self.mu_base {
            self.mu_buffer[b].tempo = t as u16;
        }
        Ok(())
    }

    /// Music part of `MusInt`.
    pub(super) fn music_vbl(&mut self) {
        let Some(base) = self.mu_base else { return };
        self.mu_every(base);
        let m = &mut self.mu_buffer[base];
        m.cpt = m.cpt.wrapping_add(m.tempo);
        if m.cpt < self.tempo_base {
            self.do_effects(base);
            return;
        }
        m.cpt -= self.tempo_base;
        let mut d5 = 0;
        let mut d7 = 0u16;
        for v in 0..4 {
            let voice = &mut self.mu_buffer[base].voices[v];
            let c = voice.cpt & 0xFF;
            if c != 0 {
                d5 += 1;
                voice.cpt = (voice.cpt & 0xFF00) | (c - 1);
                if c == 1 {
                    self.mu_step(base, v, &mut d7);
                }
            }
        }
        // The new notes' voices are stopped; MuEvery restarts them at the
        // next VBL so Paula latches the new sample.
        self.hw.dmacon(d7 & self.mu_dmask);
        if d5 == 0 {
            self.mu_fin();
        }
    }

    /// `MuFin`: the music ended, resume the previous one.
    fn mu_fin(&mut self) {
        self.mu_number = self.mu_number.saturating_sub(1);
        if self.mu_number == 0 {
            self.mu_base = None;
            self.m_off();
        } else {
            let b = self.mu_number as usize - 1;
            self.mu_base = Some(b);
            self.mu_restart = self.mu_dmask;
            self.mu_buffer[b].start = 0;
            self.mu_buffer[b].stop = 0;
        }
    }

    /// `MuEvery`: second part of the samples, voice starts and restarts.
    fn mu_every(&mut self, base: usize) {
        let Some(bank) = self.music.clone() else {
            return;
        };
        let stop = self.mu_buffer[base].stop;
        if stop != 0 {
            for v in 0..4 {
                let inst = self.mu_buffer[base].voices[v].inst;
                if stop & (1 << v) != 0
                    && inst != 0
                    && let Some(ch) = self.chip(v)
                {
                    ch.lc = Ptr::new(&bank.data, bank.inst.wrapping_add(bank.l(inst + 4)));
                    ch.len = bank.w(inst + 10);
                }
            }
        }
        let m = &mut self.mu_buffer[base];
        let start = m.start;
        m.stop = start;
        m.start = 0;
        self.hw.dmacon(((stop | start) & self.mu_dmask) | 0x8000);
        let restart = self.mu_restart;
        if restart != 0 {
            let mut d3 = 0;
            for v in 0..4 {
                if restart & (1 << v) != 0 {
                    self.mu_chip[v] = true;
                    self.hw.ch[v].len = 2;
                    if self.mu_buffer[base].voices[v].inst != 0 {
                        d3 |= 1 << v;
                    }
                }
            }
            self.mu_restart = 0;
            self.mu_dmask |= restart;
            self.mu_buffer[base].stop |= d3;
        }
    }

    /// Starts the current instrument on voice `v` (part of `DoNote`).
    fn note_on(&mut self, bank: &MusicBank, base: usize, v: usize, d7: &mut u16) {
        let inst = self.mu_buffer[base].voices[v].inst;
        if inst != 0
            && let Some(ch) = self.chip(v)
        {
            ch.lc = Ptr::new(&bank.data, bank.inst.wrapping_add(bank.l(inst)));
            ch.len = bank.w(inst + 8);
        }
        let bit = 1 << v;
        *d7 |= bit;
        let m = &mut self.mu_buffer[base];
        m.stop &= !bit;
        m.start |= bit;
        self.mu_vu[v] = m.voices[v].vol as u8;
    }

    fn set_period(&mut self, v: usize, per: u16) {
        if let Some(ch) = self.chip(v) {
            ch.per = per;
        }
    }

    /// `MuStep`: reads the commands of voice `v` until a wait.
    fn mu_step(&mut self, base: usize, v: usize, d7: &mut u16) {
        let Some(bank) = self.music.clone() else {
            return;
        };
        let mut a2 = self.mu_buffer[base].voices[v].adr;
        // Protection against corrupt banks (the original would hang).
        for _ in 0..65536 {
            let w = bank.w(a2);
            a2 = a2.wrapping_add(2);
            if w & 0x8000 == 0 {
                if w & 0x4000 != 0 {
                    // Old format note: wait count, then the period.
                    self.mu_buffer[base].voices[v].cpt = w;
                    let p = bank.w(a2);
                    a2 = a2.wrapping_add(2);
                    if p != 0 {
                        let per = p & 0x0FFF;
                        self.mu_buffer[base].voices[v].note = per;
                        self.set_period(v, per);
                        self.note_on(&bank, base, v, d7);
                    }
                    self.mu_buffer[base].voices[v].adr = a2;
                    return;
                }
                let per = w & 0x0FFF;
                self.note_on(&bank, base, v, d7);
                let voice = &mut self.mu_buffer[base].voices[v];
                if voice.ptone != 0 {
                    voice.ptone = 0;
                    voice.ptoto = per;
                    voice.effect = Effect::PTone;
                } else {
                    voice.note = per;
                    self.set_period(v, per);
                }
                continue;
            }
            let cmd = (w >> 8) & 0x7F;
            let p = w & 0xFF;
            let mu_volume = self.mu_volume as u32;
            let voice = &mut self.mu_buffer[base].voices[v];
            match cmd {
                0 | 17 => {
                    if cmd == 17 {
                        voice.pat = voice.dpat.wrapping_add(p as u32 * 2);
                    }
                    // EtEnd: next pattern of the list.
                    voice.cpt = 0;
                    voice.rep = 0;
                    voice.deb = 0;
                    voice.effect = Effect::None;
                    match Self::re_pat(&bank, voice, v) {
                        Some(a) => a2 = a,
                        None => return,
                    }
                }
                3 => {
                    voice.dvol = p.min(63);
                    voice.vol = ((voice.dvol as u32 * mu_volume) >> 6) as u16;
                }
                4 => voice.effect = Effect::None,
                5 => {
                    if p == 0 {
                        voice.deb = a2;
                    } else if voice.rep == 0 {
                        voice.rep = p;
                    } else {
                        voice.rep -= 1;
                        if voice.rep != 0 && voice.deb != 0 {
                            a2 = voice.deb;
                        }
                    }
                }
                6 => self.hw.led = true,
                7 => self.hw.led = false,
                8 => self.mu_buffer[base].tempo = p,
                9 => {
                    let inst = bank.inst.wrapping_add(2 + p as u32 * 32);
                    voice.inst = inst;
                    let vol = bank.w(inst + 12);
                    voice.dvol = if vol < 64 { vol } else { 63 };
                    voice.vol = ((voice.dvol as u32 * mu_volume) >> 6) as u16;
                }
                10 => {
                    voice.value = (voice.value & 0xFF00) | p;
                    voice.effect = Effect::Arp;
                }
                11 => {
                    voice.ptone = 1;
                    voice.value = p;
                    voice.effect = Effect::PTone;
                }
                12 => {
                    voice.value = p;
                    voice.effect = Effect::Vib;
                }
                13 => {
                    let hi = p >> 4;
                    voice.value = if hi != 0 {
                        hi
                    } else {
                        (p & 0x0F).wrapping_neg()
                    };
                    voice.effect = Effect::VSl;
                }
                14 => {
                    voice.value = p.wrapping_neg();
                    voice.effect = Effect::Slide;
                }
                15 => {
                    voice.value = p;
                    voice.effect = Effect::Slide;
                }
                16 => {
                    voice.cpt = w;
                    voice.adr = a2;
                    return;
                }
                _ => {}
            }
        }
        self.mu_buffer[base].voices[v].cpt = 0;
    }

    /// `RePat`: reads the next pattern number of the voice's list and
    /// returns the address of its commands, None when the voice ends.
    fn re_pat(bank: &MusicBank, voice: &mut MuVoice, v: usize) -> Option<u32> {
        let mut a0 = voice.pat;
        for _ in 0..4 {
            let d0 = bank.w(a0);
            a0 = a0.wrapping_add(2);
            if d0 & 0x8000 != 0 {
                if d0 == 0xFFFF {
                    return None;
                }
                a0 = voice.dpat;
                continue;
            }
            voice.pat = a0;
            if d0 > bank.w(bank.pat) {
                return None;
            }
            let idx = ((d0 << 2).wrapping_add(v as u16) << 1) as i16 as i32;
            let off = bank.w(bank.pat.wrapping_add(2).wrapping_add(idx as u32));
            if off == 0 {
                return None;
            }
            return Some(bank.pat.wrapping_add(off as u32));
        }
        None
    }

    /// `DoEffects`: effects of the 4 voices and volume registers.
    fn do_effects(&mut self, base: usize) {
        for v in 0..4 {
            self.effect(base, v);
            let vol = self.mu_buffer[base].voices[v].vol;
            if let Some(ch) = self.chip(v) {
                ch.vol = vol;
            }
        }
    }

    fn effect(&mut self, base: usize, v: usize) {
        let voice = &mut self.mu_buffer[base].voices[v];
        let per: Option<u16> = match voice.effect {
            Effect::None => Some(voice.note),
            Effect::Slide => {
                if voice.value == 0 {
                    voice.effect = Effect::None;
                    None
                } else {
                    let mut d0 = voice.value.wrapping_add(voice.note);
                    if d0 < 0x71 {
                        d0 = 0x71;
                        voice.effect = Effect::None;
                    }
                    if d0 > 0x358 {
                        d0 = 0x358;
                        voice.effect = Effect::None;
                    }
                    voice.note = d0;
                    Some(d0)
                }
            }
            Effect::Arp => {
                let lo = voice.value & 0xFF;
                let mut c = (voice.value >> 8) as u8;
                if c >= 3 {
                    c = 2;
                }
                c = c.wrapping_sub(1);
                voice.value = ((c as u16) << 8) | lo;
                if c == 0 {
                    Some(voice.note)
                } else {
                    let nib = if (c as i8) > 0 { lo & 0x0F } else { lo >> 4 } as usize;
                    (0..=36)
                        .find(|&i| voice.note as i16 >= PERIODS_EXT[i] as i16)
                        .map(|i| PERIODS_EXT[i + nib])
                }
            }
            Effect::PTone => {
                let d0 = voice.value;
                let mut d1 = voice.note;
                let to = voice.ptoto;
                if d1 == to {
                    voice.effect = Effect::None;
                } else if d1 < to {
                    d1 = d1.wrapping_add(d0);
                    if d1 >= to {
                        d1 = to;
                        voice.effect = Effect::None;
                    }
                } else {
                    d1 = d1.wrapping_sub(d0);
                    if d1 <= to {
                        d1 = to;
                        voice.effect = Effect::None;
                    }
                }
                voice.note = d1;
                Some(d1)
            }
            Effect::Vib => {
                let s = SINUS[((voice.vib >> 2) & 0x1F) as usize] as u16;
                let d2 = (s * (voice.value & 0x0F)) >> 6;
                let per = if (voice.vib as i8) < 0 {
                    voice.note.wrapping_sub(d2)
                } else {
                    voice.note.wrapping_add(d2)
                };
                voice.vib = voice
                    .vib
                    .wrapping_add((((voice.value & 0xFF) >> 2) & 0x3C) as u8);
                Some(per)
            }
            Effect::VSl => {
                let mut d0 = voice.dvol.wrapping_add(voice.value) as i16;
                if d0 < 0 {
                    d0 = 0;
                }
                if d0 >= 0x40 {
                    d0 = 0x3F;
                }
                voice.dvol = d0 as u16;
                voice.vol = ((d0 as u32 * self.mu_volume as u32) >> 6) as u16;
                None
            }
        };
        if let Some(p) = per {
            self.set_period(v, p);
        }
    }
}
