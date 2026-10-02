//! NoiseTracker module replay (`Track Play`, `mt_music`, `+Music.s:1647-2077`).
//!
//! Called once per VBL (no CIA timing). 31 instruments, finetune ignored,
//! effects 0-6, A-F (E = filter only).

use std::sync::Arc;

use super::engine::Engine;
use super::paula::Ptr;
use super::tables::{MT_PERIODS, SINUS};

/// Per channel state (`mt_voice1`..4: 28 bytes each).
#[derive(Clone, Debug, Default)]
pub struct TrackVoice {
    /// The 4 bytes of the current note.
    pub cmd: [u8; 4],
    pub start: u32,
    pub length: u16,
    pub loopstart: u32,
    pub replen: u16,
    pub period: u16,
    /// Finetune (high byte, cleared) and volume (low byte).
    pub finevol: u16,
    pub portdir: u8,
    pub portspeed: u8,
    pub wanted: u16,
    pub vibcmd: u8,
    pub vibpos: u8,
}

impl TrackVoice {
    fn volume(&self) -> u8 {
        self.finevol as u8
    }

    fn set_volume(&mut self, v: u8) {
        self.finevol = (self.finevol & 0xFF00) | v as u16;
    }

    fn param(&self) -> u8 {
        self.cmd[3]
    }

    fn command(&self) -> u8 {
        self.cmd[2] & 0x0F
    }
}

#[derive(Clone, Debug)]
pub struct Tracker {
    pub on: bool,
    /// `Track Loop On`.
    pub looping: bool,
    stop: bool,
    /// Default bank of `Track Load` / `Track Play` (6).
    pub bank: u16,
    pub data: Option<Arc<[u8]>>,
    speed: u8,
    counter: u8,
    pattpos: u16,
    songpos: u8,
    brk: bool,
    dmacon: u16,
    samplestarts: [u32; 31],
    pub voices: [TrackVoice; 4],
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker {
            on: false,
            looping: false,
            stop: false,
            bank: 6,
            data: None,
            speed: 6,
            counter: 0,
            pattpos: 0,
            songpos: 0,
            brk: false,
            dmacon: 0,
            samplestarts: [0; 31],
            voices: Default::default(),
        }
    }
}

fn rw(d: &[u8], off: usize) -> u16 {
    d.get(off..off + 2)
        .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

/// Prepares a module for playing as `Track Play` does: zeros the first 4
/// bytes of every sample and the finetunes (modifies the bank data).
/// Returns the sample start offsets.
pub fn prepare_module(d: &mut [u8]) -> [u32; 31] {
    let mut npat: i8 = 0;
    for i in 0..128 {
        let o = d.get(0x3B8 + i).copied().unwrap_or(0) as i8;
        if o > npat {
            npat = o;
        }
    }
    let mut a2 = 0x43C + (npat as usize + 1) * 1024;
    let mut starts = [0u32; 31];
    for (i, s) in starts.iter_mut().enumerate() {
        for b in d.iter_mut().skip(a2).take(4) {
            *b = 0;
        }
        *s = a2 as u32;
        let hdr = 0x2A + i * 30;
        if let Some(f) = d.get_mut(hdr + 2) {
            *f = 0;
        }
        a2 += rw(d, hdr) as usize * 2;
    }
    starts
}

impl Engine {
    /// `Track Play`: starts a module (already prepared, see
    /// [`prepare_module`]).
    pub fn track_play(&mut self, data: Arc<[u8]>, starts: [u32; 31]) {
        self.sam_stop(0xF);
        self.tracker_stop();
        let t = &mut self.tracker;
        t.data = Some(data);
        t.samplestarts = starts;
        t.speed = 6;
        for c in 0..4 {
            self.hw.ch[c].vol = 0;
        }
        let t = &mut self.tracker;
        t.songpos = 0;
        t.counter = 0;
        t.pattpos = 0;
        t.on = true;
    }

    /// `Track Stop`.
    pub fn tracker_stop(&mut self) {
        let t = &mut self.tracker;
        if !t.on {
            return;
        }
        t.on = false;
        t.dmacon = 0;
        t.stop = false;
        for c in 0..4 {
            self.hw.ch[c].vol = 0;
        }
        self.hw.dmacon(0x000F);
    }

    /// `TrackCheck`: stops the tracker if its bank disappeared.
    pub fn tracker_bank_gone(&mut self) {
        if self.tracker.on {
            self.tracker_stop();
        }
        self.tracker.data = None;
    }

    /// `Tracker` part of `MusInt`.
    pub(super) fn tracker_vbl(&mut self) {
        if !self.tracker.on {
            return;
        }
        let Some(data) = self.tracker.data.clone() else {
            return;
        };
        if self.tracker.dmacon != 0 {
            // Second part of the samples (loops) for all voices.
            for c in 0..4 {
                let v = &self.tracker.voices[c];
                self.hw.ch[c].lc = Ptr::new(&data, v.loopstart);
                self.hw.ch[c].len = v.replen;
            }
        }
        self.mt_music(&data);
        if self.tracker.stop {
            self.tracker.on = false;
            self.tracker.dmacon = 0;
            self.tracker.stop = false;
            for c in 0..4 {
                self.hw.ch[c].vol = 0;
            }
            self.hw.dmacon(0x000F);
        }
    }

    fn mt_music(&mut self, d: &Arc<[u8]>) {
        let t = &mut self.tracker;
        t.counter = t.counter.wrapping_add(1);
        if (t.counter as i8) < (t.speed as i8) {
            for c in 0..4 {
                self.mt_com(c);
            }
            if self.tracker.brk {
                self.mt_next(d);
            }
            return;
        }
        t.counter = 0;
        t.dmacon = 0;
        let order = d.get(0x3B8 + t.songpos as usize).copied().unwrap_or(0) as usize;
        let row = 0x43C + order * 1024 + t.pattpos as usize;
        for c in 0..4 {
            self.mt_playvoice(d, c, row + c * 4);
        }
        if self.tracker.dmacon != 0 {
            self.tracker.dmacon |= 0x8000;
        }
        self.tracker.pattpos += 16;
        if self.tracker.pattpos == 1024 {
            self.mt_next(d);
        }
        if self.tracker.brk {
            self.mt_next(d);
        }
        let dma = self.tracker.dmacon;
        if dma != 0 {
            self.hw.dmacon(dma);
        }
    }

    fn mt_next(&mut self, d: &[u8]) {
        let t = &mut self.tracker;
        t.pattpos = 0;
        t.brk = false;
        t.songpos = t.songpos.wrapping_add(1) & 0x7F;
        if d.get(0x3B6).copied().unwrap_or(0) == t.songpos {
            if !t.looping {
                t.stop = true;
            }
            t.songpos = d.get(0x3B7).copied().unwrap_or(0);
        }
    }

    fn mt_playvoice(&mut self, d: &Arc<[u8]>, c: usize, at: usize) {
        let mut cmd = [0u8; 4];
        if let Some(b) = d.get(at..at + 4) {
            cmd.copy_from_slice(b);
        }
        let ins = ((cmd[2] >> 4) | (cmd[0] & 0xF0)) as usize;
        let v = &mut self.tracker.voices[c];
        v.cmd = cmd;
        if ins != 0 && ins <= 31 {
            v.start = self.tracker.samplestarts[ins - 1];
            let hdr = 0x0C + ins * 30;
            v.length = rw(d, hdr);
            v.finevol = rw(d, hdr + 2);
            let repstart = rw(d, hdr + 4);
            let replen = rw(d, hdr + 6);
            if repstart != 0 {
                v.loopstart = v.start + repstart as u32 * 2;
                v.length = repstart.wrapping_add(replen);
            } else {
                v.loopstart = v.start;
            }
            v.replen = replen;
            self.hw.ch[c].vol = v.finevol;
            self.mu_vu[c] = v.volume();
        }
        // mt_oldinstr
        let v = &mut self.tracker.voices[c];
        let per = u16::from_be_bytes([cmd[0], cmd[1]]) & 0x0FFF;
        if per != 0 {
            let bit = 1u16 << c;
            if v.length == 0 {
                self.hw.dmacon(bit);
            } else if v.command() == 3 || v.command() == 5 {
                // mt_setport
                v.wanted = per;
                v.portdir = 0;
                if per == v.period {
                    v.wanted = 0;
                    return;
                }
                if (per as i16) < (v.period as i16) {
                    v.portdir = 1;
                }
            } else {
                v.period = per;
                v.vibpos = 0;
                self.hw.dmacon(bit);
                let ch = &mut self.hw.ch[c];
                ch.lc = Ptr::new(d, v.start);
                ch.len = v.length;
                ch.per = per;
                self.tracker.dmacon |= bit;
            }
        }
        self.mt_com2(c);
    }

    /// Effects done on the row (`mt_com2`).
    fn mt_com2(&mut self, c: usize) {
        let v = &mut self.tracker.voices[c];
        let p = v.param();
        match v.command() {
            0x0E => self.hw.led = p & 1 == 0,
            0x0D => self.tracker.brk = true,
            0x0B => {
                // The original also sets COLOR00 to $FFF here (ignored).
                self.tracker.brk = true;
                self.tracker.songpos = p.wrapping_sub(1);
            }
            0x0C => {
                if p > 0x40 {
                    v.cmd[3] = 0x40;
                }
                let vol = v.param();
                v.set_volume(vol);
                self.hw.ch[c].vol = vol as u16;
            }
            0x0F => {
                let s = p.clamp(1, 0x1F);
                self.tracker.speed = s;
            }
            _ => {}
        }
    }

    /// Effects done between rows (`mt_com`).
    fn mt_com(&mut self, c: usize) {
        let v = &self.tracker.voices[c];
        if u16::from_be_bytes([v.cmd[2], v.cmd[3]]) & 0x0FFF == 0 {
            self.hw.ch[c].per = v.period;
            return;
        }
        match v.command() {
            0 => self.mt_arp(c),
            6 => {
                self.mt_vib2(c);
                self.mt_volslide(c);
            }
            4 => {
                let v = &mut self.tracker.voices[c];
                if v.param() != 0 {
                    v.vibcmd = v.param();
                }
                self.mt_vib2(c);
            }
            5 => {
                self.mt_port2(c);
                self.mt_volslide(c);
            }
            3 => {
                let v = &mut self.tracker.voices[c];
                if v.param() != 0 {
                    v.portspeed = v.param();
                    v.cmd[3] = 0;
                }
                self.mt_port2(c);
            }
            1 => {
                let v = &mut self.tracker.voices[c];
                v.period = v.period.wrapping_sub(v.param() as u16);
                if (v.period as i16) < 0x71 {
                    v.period = 0x71;
                }
                self.hw.ch[c].per = v.period;
            }
            2 => {
                let v = &mut self.tracker.voices[c];
                v.period = v.period.wrapping_add(v.param() as u16);
                if (v.period as i16) >= 0x358 {
                    v.period = 0x358;
                }
                self.hw.ch[c].per = v.period;
            }
            cmd => {
                self.hw.ch[c].per = v.period;
                if cmd == 0x0A {
                    self.mt_volslide(c);
                }
            }
        }
    }

    fn mt_arp(&mut self, c: usize) {
        let v = &self.tracker.voices[c];
        let n = match self.tracker.counter % 3 {
            0 => {
                self.hw.ch[c].per = v.period;
                return;
            }
            1 => v.param() >> 4,
            _ => v.param() & 0x0F,
        } as usize;
        let i = MT_PERIODS
            .iter()
            .position(|&p| (v.period as i16) >= p as i16)
            .unwrap_or(36);
        self.hw.ch[c].per = MT_PERIODS.get(i + n).copied().unwrap_or(0);
    }

    fn mt_port2(&mut self, c: usize) {
        let v = &mut self.tracker.voices[c];
        if v.wanted == 0 {
            return;
        }
        let s = v.portspeed as u16;
        if v.portdir == 0 {
            v.period = v.period.wrapping_add(s);
            if (v.wanted as i16) <= (v.period as i16) {
                v.period = v.wanted;
                v.wanted = 0;
            }
        } else {
            v.period = v.period.wrapping_sub(s);
            if (v.wanted as i16) >= (v.period as i16) {
                v.period = v.wanted;
                v.wanted = 0;
            }
        }
        self.hw.ch[c].per = v.period;
    }

    fn mt_vib2(&mut self, c: usize) {
        let v = &mut self.tracker.voices[c];
        let s = SINUS[((v.vibpos >> 2) & 0x1F) as usize] as u16;
        let d2 = (s * (v.vibcmd & 0x0F) as u16) >> 7;
        let per = if (v.vibpos as i8) < 0 {
            v.period.wrapping_sub(d2)
        } else {
            v.period.wrapping_add(d2)
        };
        self.hw.ch[c].per = per;
        v.vibpos = v.vibpos.wrapping_add((v.vibcmd >> 2) & 0x3C);
    }

    fn mt_volslide(&mut self, c: usize) {
        let v = &mut self.tracker.voices[c];
        let p = v.param();
        let mut vol = v.volume() as i8;
        if p >> 4 != 0 {
            vol = vol.wrapping_add((p >> 4) as i8);
            if vol.wrapping_sub(0x40) >= 0 {
                vol = 0x40;
            }
        } else {
            vol = vol.wrapping_sub((p & 0x0F) as i8);
            if vol < 0 {
                vol = 0;
            }
        }
        v.set_volume(vol as u8);
        self.hw.ch[c].vol = v.finevol;
    }
}
