//! Sound: the Music extension's instructions (extension slot 1,
//! `+Music.s`). The sound state itself lives in [`crate::audio::Mixer`],
//! shared with the audio thread; this file reads the parameters, resolves
//! banks and memory, and calls the engine under the mixer lock.

use std::sync::Arc;

use super::Hardware;
use crate::audio::engine::{
    ERR_IFONC, ERR_NOT_TRACKER, ERR_SAMPLE_BANK_NOT_FOUND, ERR_SAMPLE_NOT_DEFINED,
};
use crate::audio::{Ptr, SampleData, narrator, tracker};
use crate::banks::{Bank, BankData};
use crate::errors;
use crate::interp::value::{ENT_NUL, Value};
use crate::interp::{Exc, Interp, R, err};
use crate::tokens::{Keyword, tk};

/// Converts an engine error number.
fn serr<T>(r: Result<T, u16>) -> R<T> {
    r.map_err(Exc::Error)
}

fn be16(d: &[u8], o: usize) -> u16 {
    d.get(o..o + 2)
        .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn be32(d: &[u8], o: usize) -> u32 {
    (be16(d, o) as u32) << 16 | be16(d, o + 2) as u32
}

/// `I/O error` (`IDError`, DEBase+15).
const ERR_IO: u16 = 94;
/// `Cannot load med.library`.
const ERR_NO_MED: u16 = 187;

impl Hardware {
    pub(crate) fn sound_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        use tk::*;
        if kw.slot != 1 {
            return Ok(false);
        }
        self.sound_bank_check();
        match kw.token {
            MUSIC_VOICE => {
                let a = it.inst_args(self, kw)?;
                self.sound.lock().engine.v_on_of(a.int(0) as u16 & 0xF);
            }
            MUSIC_MUSIC_OFF => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.music_off();
            }
            MUSIC_MUSIC_STOP => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.music_stop();
            }
            MUSIC_TEMPO => {
                let a = it.inst_args(self, kw)?;
                serr(self.sound.lock().engine.tempo(a.int(0)))?;
            }
            MUSIC_MUSIC => {
                let a = it.inst_args(self, kw)?;
                serr(self.sound.lock().engine.music(a.int(0)))?;
            }
            MUSIC_NOISE_TO => {
                let a = it.inst_args(self, kw)?;
                self.sound.lock().engine.set_voice_wave(0, a.int(0) as u16);
            }
            MUSIC_BOOM => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.boom();
            }
            MUSIC_SHOOT => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.shoot();
            }
            MUSIC_SAM_BANK => {
                let n = it.inst_args(self, kw)?.int(0);
                if !(1..=16).contains(&n) {
                    return err(ERR_IFONC);
                }
                self.sound.lock().engine.sam_bank = n as u16;
            }
            // The token table pairs these oddly (`+Music.s:405-412`): both
            // parameterless "Sam Loop On" forms loop all voices, and
            // "Sam Loop On voices" runs InSamLoopOff1 (loop off).
            MUSIC_SAM_LOOP_ON | MUSIC_SAM_LOOP_ON_2 => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.sl0(0, 0xF);
            }
            MUSIC_SAM_LOOP_OFF => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.sl0(-1, 0xF);
            }
            MUSIC_SAM_LOOP_ON_3 => {
                let a = it.inst_args(self, kw)?;
                self.sound.lock().engine.sl0(-1, a.int(0) as u16);
            }
            MUSIC_SAMPLE => {
                let a = it.inst_args(self, kw)?;
                let n = a.int(0);
                self.sound_get_sam(n)?;
                self.sound
                    .lock()
                    .engine
                    .set_voice_wave((n as i16).wrapping_neg(), a.int(1) as u16);
            }
            MUSIC_SAM_PLAY | MUSIC_SAM_PLAY_2 | MUSIC_SAM_PLAY_3 => {
                let a = it.inst_args(self, kw)?;
                let (voices, n) = match kw.token {
                    MUSIC_SAM_PLAY => (0xF, a.int(0)),
                    _ => (a.int(0), a.int(1)),
                };
                let mut s = self.sound_get_sam(n)?;
                if kw.token == MUSIC_SAM_PLAY_3 {
                    let f = a.int(2);
                    if f <= 500 {
                        return err(ERR_IFONC);
                    }
                    s.freq = f as u32;
                }
                self.sound.lock().engine.go_sam(voices as u16 & 0xF, &s);
            }
            MUSIC_SAM_RAW => {
                let a = it.inst_args(self, kw)?;
                let (voices, addr, len, freq) = (a.int(0), a.int(1), a.int(2), a.int(3));
                if freq <= 500 || len <= 256 {
                    return err(ERR_IFONC);
                }
                let data = self.sound_mem(addr as u32, len as u32);
                let s = SampleData {
                    ptr: Ptr::new(&data, 0),
                    len: len as u32,
                    freq: freq as u32,
                };
                self.sound.lock().engine.go_sam(voices as u16 & 0xF, &s);
            }
            MUSIC_BELL | MUSIC_BELL_2 => {
                let a = it.inst_args(self, kw)?;
                let note = if kw.token == MUSIC_BELL { 70 } else { a.int(0) };
                serr(self.sound.lock().engine.bell(note))?;
            }
            MUSIC_PLAY_OFF | MUSIC_PLAY_OFF_2 => {
                let a = it.inst_args(self, kw)?;
                let voices = if kw.token == MUSIC_PLAY_OFF {
                    0xF
                } else {
                    a.int(0)
                };
                self.sound.lock().engine.env_off(voices as u16);
            }
            MUSIC_PLAY | MUSIC_PLAY_2 => {
                let a = it.inst_args(self, kw)?;
                // Retried while waiting the delay: do not replay the note.
                if it.wait.as_ref().is_some_and(|w| w.pos == it.inst_pos) {
                    it.wait_vbls(0)?;
                    return Ok(true);
                }
                let (voices, note, delay) = match kw.token {
                    MUSIC_PLAY => (0xF, a.int(0), a.int(1)),
                    _ => (a.int(0), a.int(1), a.int(2)),
                };
                if delay < 0 {
                    return err(ERR_IFONC);
                }
                self.sound_play(voices as u16, note)?;
                if delay > 0 {
                    it.wait_vbls(delay as u64)?;
                }
            }
            MUSIC_SET_WAVE => {
                let a = it.inst_args(self, kw)?;
                let s = a.str(1);
                serr(self.sound.lock().engine.set_wave(a.int(0), &s))?;
            }
            MUSIC_DEL_WAVE => {
                let a = it.inst_args(self, kw)?;
                serr(self.sound.lock().engine.del_wave(a.int(0)))?;
            }
            MUSIC_SET_ENVEL => {
                let a = it.inst_args(self, kw)?;
                serr(
                    self.sound
                        .lock()
                        .engine
                        .set_envel(a.int(0), a.int(1), a.int(2), a.int(3)),
                )?;
            }
            MUSIC_MVOLUME => {
                let v = it.inst_args(self, kw)?.int(0);
                if !(0..64).contains(&v) {
                    return err(ERR_IFONC);
                }
                self.sound.lock().engine.mvol(v as u16);
            }
            MUSIC_VOLUME => {
                let v = it.inst_args(self, kw)?.int(0);
                let mut m = self.sound.lock();
                serr(m.engine.vol(v, 0xF))?;
                m.engine.mvol(v as u16);
            }
            MUSIC_VOLUME_2 => {
                let a = it.inst_args(self, kw)?;
                serr(self.sound.lock().engine.vol(a.int(1), a.int(0) as u16))?;
            }
            MUSIC_WAVE => {
                let a = it.inst_args(self, kw)?;
                serr(self.sound.lock().engine.wave_to(a.int(0), a.int(1)))?;
            }
            MUSIC_LED_ON => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.hw.led = true;
            }
            MUSIC_LED_OFF => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.hw.led = false;
            }
            MUSIC_SAY | MUSIC_SAY_2 => {
                let a = it.inst_args(self, kw)?;
                if it.wait.as_ref().is_some_and(|w| w.pos == it.inst_pos) {
                    if it.wait_vbls(0).is_err() {
                        return Err(Exc::Block);
                    }
                    self.sound.lock().engine.v_on_of(0xF);
                    return Ok(true);
                }
                let mode = if kw.token == MUSIC_SAY { 0 } else { a.int(1) };
                let text = String::from_utf8_lossy(&a.str(0)).into_owned();
                let vbls = self.sound_say(&text, mode != 0);
                if mode == 0 {
                    if vbls > 0 {
                        it.wait_vbls(vbls)?;
                    }
                    self.sound.lock().engine.v_on_of(0xF);
                }
            }
            MUSIC_SET_TALK => {
                let a = it.inst_args(self, kw)?;
                let n = &mut self.sound.narrator;
                let (sex, mode, pitch, rate) = (a.int(0), a.int(1), a.int(2), a.int(3));
                if rate != ENT_NUL {
                    if !(40..=400).contains(&(rate as u16)) {
                        return err(ERR_IFONC);
                    }
                    n.rate = rate as u16;
                }
                if pitch != ENT_NUL {
                    if !(65..=320).contains(&(pitch as u16)) {
                        return err(ERR_IFONC);
                    }
                    n.pitch = pitch as u16;
                }
                if mode != ENT_NUL {
                    n.mode = mode as u16 & 1;
                }
                if sex != ENT_NUL {
                    n.sex = sex as u16 & 1;
                }
            }
            MUSIC_TALK_MISC => {
                let a = it.inst_args(self, kw)?;
                let (volume, freq) = (a.int(0), a.int(1));
                let n = &mut self.sound.narrator;
                if freq != ENT_NUL {
                    if !(5000..=25000).contains(&(freq as u16)) {
                        return err(ERR_IFONC);
                    }
                    n.freq = freq as u16;
                }
                if volume != ENT_NUL {
                    if volume < 0 || volume as u16 > 64 {
                        return err(ERR_IFONC);
                    }
                    n.volume = volume as u16;
                }
            }
            MUSIC_TALK_STOP => {
                it.inst_args(self, kw)?;
                if self.sound.narrator.speaking {
                    self.sound.narrator.speaking = false;
                    self.sound.lock().engine.sam_stop(0xF);
                }
            }
            MUSIC_MOUTH_READ => {
                it.inst_args(self, kw)?;
                // No mouth shapes are generated: "no data" (-1, -1).
                self.sound.narrator.mouth = (-1, -1);
            }
            MUSIC_SLOAD => {
                let a = it.inst_args(self, kw)?;
                let (file, addr, len) = (a.int(0), a.int(1), a.int(2));
                if len < 0 {
                    return err(ERR_IFONC);
                }
                let addr = self
                    .banks
                    .bank_or_address(addr)
                    .ok_or(Exc::Error(errors::BANK_NOT_RESERVED))?;
                let ch = self.sound_channel(file)?;
                let start = ch.pos.min(ch.data.len());
                let end = start.saturating_add(len as usize).min(ch.data.len());
                let data = ch.data[start..end].to_vec();
                ch.pos = end;
                self.banks.poke_bytes(addr, &data);
            }
            MUSIC_SSAVE => {
                let a = it.inst_args(self, kw)?;
                let (file, start, end) = (a.int(0), a.int(1), a.int(2));
                let len = end.wrapping_sub(start);
                if len <= 0 {
                    return err(ERR_IFONC);
                }
                let data = self.banks.peek_bytes(start as u32, len as usize);
                let ch = self.sound_channel(file)?;
                let p = ch.pos.min(ch.data.len());
                let e = p + data.len();
                if ch.data.len() < e {
                    ch.data.resize(e, 0);
                }
                ch.data[p..e].copy_from_slice(&data);
                ch.pos = e;
                ch.dirty = true;
            }
            MUSIC_SAM_SWAP => {
                let a = it.inst_args(self, kw)?;
                let (voices, addr, len) = (a.int(0), a.int(1), a.int(2));
                if len < 0 {
                    return err(ERR_IFONC);
                }
                let addr = self
                    .banks
                    .bank_or_address(addr)
                    .ok_or(Exc::Error(errors::BANK_NOT_RESERVED))?;
                let ptr = (addr != 0).then(|| Ptr::new(&self.sound_mem(addr, len as u32), 0));
                self.sound
                    .lock()
                    .engine
                    .sam_swap(voices as u16, ptr, len as u32);
            }
            MUSIC_SAM_STOP | MUSIC_SAM_STOP_2 => {
                let a = it.inst_args(self, kw)?;
                let v = if kw.token == MUSIC_SAM_STOP {
                    0xF
                } else {
                    a.int(0)
                };
                self.sound.lock().engine.sam_stop(v as u16);
            }
            MUSIC_TRACK_STOP => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.tracker_stop();
            }
            MUSIC_TRACK_LOOP_ON => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.tracker.looping = true;
            }
            MUSIC_TRACK_LOOP_OF => {
                it.inst_args(self, kw)?;
                self.sound.lock().engine.tracker.looping = false;
            }
            MUSIC_TRACK_PLAY | MUSIC_TRACK_PLAY_2 | MUSIC_TRACK_PLAY_3 => {
                let a = it.inst_args(self, kw)?;
                // The pattern parameter is not supported by the original.
                let bank = a
                    .opt(0)
                    .unwrap_or(self.sound.lock().engine.tracker.bank as i32);
                self.track_play(bank)?;
            }
            MUSIC_TRACK_LOAD => {
                let a = it.inst_args(self, kw)?;
                let name = String::from_utf8_lossy(&a.str(0)).into_owned();
                let bank = a.int(1);
                if !(0..0x10000).contains(&bank) || name.is_empty() || name.len() > 128 {
                    return err(ERR_IFONC);
                }
                {
                    let mut m = self.sound.lock();
                    if m.engine.tracker.bank == bank as u16 && m.engine.tracker.on {
                        m.engine.tracker_stop();
                    }
                    m.engine.tracker.bank = bank as u16;
                }
                let data = self.files.read(&name).map_err(|_| Exc::Error(ERR_IO))?;
                self.track_load_bytes(bank as u16, data);
            }
            MUSIC_MED_LOAD | MUSIC_MED_PLAY | MUSIC_MED_PLAY_2 | MUSIC_MED_PLAY_3 => {
                it.inst_args(self, kw)?;
                // medplayer.library is not emulated: the error the original
                // gives on an Amiga without it.
                return err(ERR_NO_MED);
            }
            MUSIC_MED_STOP | MUSIC_MED_CONT | MUSIC_MED_MIDI_ON => {
                it.inst_args(self, kw)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn sound_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        use tk::*;
        if kw.slot != 1 {
            return Ok(None);
        }
        self.sound_bank_check();
        Ok(Some(match kw.token {
            MUSIC_MUBASE => {
                it.func_args(self, kw)?;
                // Address of the extension's data zone: not mapped.
                Value::Int(0)
            }
            MUSIC_VUMETER => {
                let v = it.func_args(self, kw)?.int(0);
                Value::Int(serr(self.sound.lock().engine.vumeter(v))?)
            }
            MUSIC_SAM_SWAPPED => {
                let v = it.func_args(self, kw)?.int(0);
                if !(0..=3).contains(&v) {
                    return err(ERR_IFONC);
                }
                Value::Int(self.sound.lock().engine.sam_swapped(v as usize))
            }
            MUSIC_MOUTH_WIDTH => {
                it.func_args(self, kw)?;
                Value::Int(self.sound.narrator.mouth.0 as i32)
            }
            MUSIC_MOUTH_HEIGHT => {
                it.func_args(self, kw)?;
                Value::Int(self.sound.narrator.mouth.1 as i32)
            }
            _ => return Ok(None),
        }))
    }

    /// `MusDef`: Default / Run.
    pub(crate) fn sound_reset(&mut self) {
        self.sound.lock().engine.mus_def();
        self.sound.narrator = narrator::Settings::default();
        self.sound.bank_generation = None;
    }

    pub(crate) fn sound_vbl(&mut self) {
        self.sound_bank_check();
        self.sound.lock().host_vbl();
    }

    /// `BkCheck` / `TrackCheck`: called when the bank list changed (checked
    /// through `BankSet::generation`).
    pub(crate) fn sound_bank_check(&mut self) {
        let g = self.banks.generation;
        if self.sound.bank_generation == Some(g) {
            return;
        }
        self.sound.bank_generation = Some(g);
        let music: Option<Arc<[u8]>> = self
            .banks
            .get(3)
            .filter(|b| b.name.starts_with("Musi"))
            .and_then(|b| b.raw())
            .map(Arc::from);
        let mut m = self.sound.lock();
        m.engine.music_bank_check(music);
        let t = &m.engine.tracker;
        if t.data.is_some() {
            let ok = self.banks.banks.values().any(|b| {
                b.name == "Tracker "
                    && b.raw()
                        .is_some_and(|d| t.data.as_ref().is_some_and(|x| x.len() == d.len()))
            });
            if !ok {
                m.engine.tracker_bank_gone();
            }
        }
    }

    /// `GetSam`: sample `n` of the current sample bank.
    pub(crate) fn sound_get_sam(&mut self, n: i32) -> R<SampleData> {
        let bank = self.sound.lock().engine.sam_bank;
        if bank == 0 {
            return err(ERR_IFONC);
        }
        let Some(d) = self
            .banks
            .get(bank)
            .filter(|b| b.name.starts_with("Samp"))
            .and_then(|b| b.raw())
        else {
            return err(ERR_SAMPLE_BANK_NOT_FOUND);
        };
        if n == 0 {
            return err(ERR_IFONC);
        }
        if n as u16 > be16(d, 0) {
            return err(ERR_SAMPLE_NOT_DEFINED);
        }
        let idx = ((n as u16).wrapping_shl(2) as i16 as i32 - 2) as usize;
        let off = be32(d, idx) as usize;
        if off == 0 {
            return err(ERR_SAMPLE_NOT_DEFINED);
        }
        let freq = be16(d, off + 8) as u32;
        let len = be32(d, off + 10);
        let start = (off + 14).min(d.len());
        let end = start.saturating_add(len as usize).min(d.len());
        let data: Arc<[u8]> = Arc::from(&d[start..end]);
        Ok(SampleData {
            ptr: Ptr::new(&data, 0),
            len,
            freq,
        })
    }

    /// Copy of `len` bytes of memory at `addr` (bank data or free memory),
    /// for Sam Raw / Sam Swap. The sound engine plays from this snapshot.
    pub(crate) fn sound_mem(&self, addr: u32, len: u32) -> Arc<[u8]> {
        for (&n, b) in &self.banks.banks {
            if let (Some(start), BankData::Raw(d)) = (self.banks.start(n), &b.data)
                && addr >= start
                && ((addr - start) as usize) < d.len()
            {
                let o = (addr - start) as usize;
                let end = o.saturating_add(len as usize);
                let mut v = d[o..end.min(d.len())].to_vec();
                if end > d.len() {
                    v.extend(self.banks.peek_bytes(start + d.len() as u32, end - d.len()));
                }
                return v.into();
            }
        }
        self.banks.peek_bytes(addr, len as usize).into()
    }

    /// Play [voices,]note: resolves the samples of voices set by
    /// `Sample n To`, then starts the note.
    fn sound_play(&mut self, voices: u16, note: i32) -> R<()> {
        let sel = self.sound.lock().engine.wave_sel;
        let mut samples: [Option<SampleData>; 4] = Default::default();
        if (0..=96).contains(&note) && note != 0 {
            for v in 0..4 {
                if voices & (1 << v) != 0 && sel[v] < 0 {
                    samples[v] = Some(self.sound_get_sam(-(sel[v] as i32))?);
                }
            }
        }
        serr(
            self.sound
                .lock()
                .engine
                .go_bel(voices, note, -1, None, &samples),
        )
    }

    /// `Track Play bank`.
    fn track_play(&mut self, bank: i32) -> R<()> {
        let n = self.sound_bank_number(bank);
        let Some(n) = n.filter(|&n| self.banks.get(n).is_some_and(|b| b.name == "Tracker ")) else {
            return err(ERR_NOT_TRACKER);
        };
        let Some(d) = self.banks.raw_mut(n) else {
            return err(ERR_NOT_TRACKER);
        };
        let starts = tracker::prepare_module(d);
        let data: Arc<[u8]> = Arc::from(&d[..]);
        self.sound.lock().engine.track_play(data, starts);
        Ok(())
    }

    /// Bank number for a "bank number or address" parameter (`Bnk.OrAdr`).
    fn sound_bank_number(&self, v: i32) -> Option<u16> {
        if (0..1024).contains(&v) {
            return Some(v as u16);
        }
        self.banks
            .banks
            .keys()
            .copied()
            .find(|&n| self.banks.start(n) == Some(v as u32))
    }

    /// `Track Load`: installs a module file as a "Tracker " bank (32 spare
    /// bytes at the end, as the original reserves).
    pub fn track_load_bytes(&mut self, bank: u16, mut data: Vec<u8>) {
        data.resize(data.len() + 32, 0);
        self.banks.insert(Bank {
            number: bank,
            name: "Tracker ".into(),
            chip: true,
            data_bank: true,
            data: BankData::Raw(data),
        });
        self.on_banks_changed();
        self.sound_bank_check();
    }

    /// `Say`: synthesizes and plays the text on voices 0 and 1 (the first
    /// narrator channel map). Returns the duration in VBLs.
    fn sound_say(&mut self, text: &str, asynchronous: bool) -> u64 {
        let s = self.sound.narrator.clone();
        let pcm = narrator::synthesize(text, &s);
        let mut m = self.sound.lock();
        let e = &mut m.engine;
        e.say_start();
        if pcm.is_empty() {
            return 0;
        }
        let len = pcm.len() as u32;
        let data: Arc<[u8]> = pcm.into();
        e.say_play(
            &SampleData {
                ptr: Ptr::new(&data, 0),
                len,
                freq: s.freq as u32,
            },
            s.volume.min(64),
        );
        drop(m);
        self.sound.narrator.speaking = asynchronous;
        (len as u64 * 50).div_ceil(s.freq.max(1) as u64)
    }

    /// Open channel `n` (1..9) for Sload / Ssave.
    fn sound_channel(&mut self, n: i32) -> R<&mut crate::files::Channel> {
        if !(1..10).contains(&n) {
            return err(ERR_IFONC);
        }
        self.files
            .channels
            .get_mut(&(n as u32))
            .ok_or(Exc::Error(ERR_IFONC))
    }
}

#[cfg(test)]
mod tests {
    use crate::Machine;
    use crate::interp::RunState;

    fn example_bank(path: &str) -> Option<Vec<crate::banks::Bank>> {
        let p = format!(
            "{}/../../AMOS-Professional-365/AMOS/Examples/{path}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(p)
            .ok()
            .map(|d| crate::banks::parse_banks(&d).unwrap())
    }

    /// Runs a program (with banks) until it stops; returns the machine and
    /// the number of VBLs it took.
    fn run(src: &str, banks: Vec<crate::banks::Bank>) -> (Machine, usize) {
        let mut prg = crate::tokenise::tokenise_program(src.as_bytes()).expect("tokenise");
        prg.banks = banks;
        let mut m = Machine::new();
        m.hw.sound.lock().set_headless();
        m.run_program(&prg).expect("verify");
        for n in 0..3000 {
            m.vbl();
            if matches!(m.state, RunState::Stopped(_)) {
                return (m, n);
            }
        }
        (m, 3000)
    }

    fn log(m: &Machine) -> String {
        m.hw.log.join("|")
    }

    #[test]
    fn effects_run() {
        let (m, _) = run(
            "Boom : Shoot : Bell : Bell 40 : Volume 40 : Volume 3,20 : Led On\nEnd",
            vec![],
        );
        assert!(log(&m).contains("End"), "{}", log(&m));
        let mx = m.hw.sound.lock();
        assert!(mx.engine.hw.led);
        assert_eq!(mx.engine.env[0].dvol, 20);
        assert_eq!(mx.engine.env[3].dvol, 40);
        assert_eq!(mx.engine.mu_volume, 40);
    }

    #[test]
    fn play_waits_its_delay() {
        let (m, n) = run("Play 46,25 : Play 1,40,0 : Play 0,0\nEnd", vec![]);
        assert!(log(&m).contains("End"), "{}", log(&m));
        assert!((25..30).contains(&n), "{n}");
    }

    #[test]
    fn say_waits_until_spoken() {
        let (m, n) = run(
            "Set Talk 1,1,150,200 : Talk Misc 50,20000 : Say \"Hello world\"\nEnd",
            vec![],
        );
        assert!(log(&m).contains("End"), "{}", log(&m));
        assert!((20..200).contains(&n), "{n}");
        assert_eq!(m.hw.sound.narrator.pitch, 150);
        let (m, n) = run("Say \"Hello world\",1\nEnd", vec![]);
        assert!(log(&m).contains("End"), "{}", log(&m));
        assert!(n < 3, "{n}");
        let (m, _) = run("Set Talk ,,500,", vec![]);
        assert!(log(&m).contains("Illegal function call"), "{}", log(&m));
    }

    #[test]
    fn errors() {
        for (src, msg) in [
            ("Music 1", "Music bank not found"),
            ("Play 97,0", "Illegal function call"),
            ("Volume 64", "Illegal function call"),
            ("Set Wave 2,\"abc\"", "256 characters for a wave"),
            ("Del Wave 1", "Wave 0 and 1 are reserved"),
            ("Wave 5 To 1", "Wave not defined"),
            ("Sam Play 1", "Sample bank not found"),
            ("Track Play", "Not a tracker module"),
            ("Med Play", "Cannot load med.library"),
            ("Print Vumeter(4)", "Illegal function call"),
        ] {
            let (m, _) = run(src, vec![]);
            assert!(log(&m).contains(msg), "{src}: {}", log(&m));
        }
    }

    #[test]
    fn music_from_program_bank() {
        let Some(banks) = example_bank("Music/Music.abk") else {
            return;
        };
        let (m, _) = run(
            "Music 1 : Tempo 20 : Wait 50 : V=Vumeter(0)+Vumeter(1)+Vumeter(2)+Vumeter(3)\nEnd",
            banks,
        );
        assert!(log(&m).contains("End"), "{}", log(&m));
        let mx = m.hw.sound.lock();
        assert_eq!(mx.engine.mu_number, 1);
        // The song sets its own tempo (16) at its first step.
        assert_eq!(mx.engine.mu_buffer[0].tempo, 16);
    }

    #[test]
    fn samples_from_program_bank() {
        let Some(mut banks) = example_bank("Samples/Instruments.abk") else {
            return;
        };
        // The file stores bank number 0 (loaded with Load "...",5).
        banks[0].number = 5;
        let (m, _) = run(
            "Sam Play 1 : Sample 2 To 2 : Play 2,46,0 : Sam Play 8,1,8000\nEnd",
            banks,
        );
        assert!(log(&m).contains("End"), "{}", log(&m));
        let mx = m.hw.sound.lock();
        assert_eq!(mx.engine.wave_sel[1], -2);
        // 3546895 / 8000.
        assert_eq!(mx.engine.hw.ch[3].per, 443);
    }

    #[test]
    fn tracker_bank() {
        let p = format!(
            "{}/../../AMOS-Professional-365/AMOS/Examples/Music/Mod.Tracker",
            env!("CARGO_MANIFEST_DIR")
        );
        let Ok(module) = std::fs::read(p) else { return };
        let mut prg =
            crate::tokenise::tokenise_program(b"Track Loop On : Track Play : Wait 20\nEnd")
                .unwrap();
        prg.banks = vec![];
        let mut m = Machine::new();
        m.hw.sound.lock().set_headless();
        m.run_program(&prg).unwrap();
        m.hw.track_load_bytes(6, module);
        for _ in 0..30 {
            m.vbl();
        }
        assert!(log(&m).contains("End"), "{}", log(&m));
        let mx = m.hw.sound.lock();
        assert!(mx.engine.tracker.on);
        assert_ne!(mx.engine.hw.dma, 0);
    }
}
