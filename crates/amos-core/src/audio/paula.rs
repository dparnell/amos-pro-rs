//! Paula (the Amiga sound chip) at register level.
//!
//! Each of the 4 channels reads signed 8-bit samples from "chip memory"
//! by DMA, one byte every `period` clock ticks, and scales them by its
//! volume (0..64). LC/LEN are double buffered: they are latched when DMA
//! starts and again each time a block ends, so writing new values while a
//! block plays queues the next block (instrument loops, sample streaming).
//! Each latch raises the channel's audio interrupt (INTREQ bits 7-10),
//! serviced by the Music extension's sample handler when enabled in INTENA.
//!
//! Output resampling integrates the zero-order-hold signal Paula produces
//! over each host sample (box filter): cheap, no ringing, and it removes
//! most aliasing of the original stepped waveform.

use std::sync::Arc;

use super::tables::LNOISE;

/// PAL Paula clock (`MusClock`).
pub const PAL_CLOCK: u32 = 3_546_895;

/// A chip memory buffer a channel can read.
#[derive(Clone, Debug, Default)]
pub enum Src {
    /// Nothing (reads zeros).
    #[default]
    None,
    /// Bank data, waves, samples: shared, immutable.
    Buf(Arc<[u8]>),
    /// The noise buffer (wave 0), rewritten every VBL while used.
    Noise,
}

/// An address in chip memory: a buffer and an offset in it.
#[derive(Clone, Debug, Default)]
pub struct Ptr {
    pub src: Src,
    pub off: u32,
}

impl Ptr {
    pub fn new(buf: &Arc<[u8]>, off: u32) -> Self {
        Ptr {
            src: Src::Buf(buf.clone()),
            off,
        }
    }

    pub fn noise(off: u32) -> Self {
        Ptr {
            src: Src::Noise,
            off,
        }
    }

    pub fn add(&self, n: u32) -> Self {
        Ptr {
            src: self.src.clone(),
            off: self.off.wrapping_add(n),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.src, Src::None)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Channel {
    // Registers.
    pub lc: Ptr,
    pub len: u16,
    pub per: u16,
    pub vol: u16,
    pub dat: i8,
    // Internal state of the DMA state machine.
    active: bool,
    cur: Ptr,
    words_left: u32,
    byte: u32,
    sample: i8,
    /// Clock ticks until the next sample is loaded.
    left: f64,
}

impl Channel {
    fn volume(&self) -> f32 {
        if self.vol & 0x40 != 0 {
            64.0
        } else {
            (self.vol & 0x3F) as f32
        }
    }

    fn period(&self) -> f64 {
        if self.per == 0 {
            65536.0
        } else {
            self.per as f64
        }
    }
}

/// The 4 audio channels plus DMACON / INTENA / INTREQ audio bits and the
/// power LED low-pass filter.
#[derive(Clone, Debug)]
pub struct Paula {
    pub ch: [Channel; 4],
    /// Audio DMA enable bits (0-3).
    pub dma: u16,
    /// INTENA audio bits (7-10).
    pub intena: u16,
    /// INTREQ audio bits (7-10).
    pub intreq: u16,
    /// LED filter on (`$BFE001` bit 1 clear).
    pub led: bool,
    /// The noise buffer (WaveDeb of wave 0).
    pub noise: [u8; LNOISE],
}

impl Default for Paula {
    fn default() -> Self {
        Paula {
            ch: Default::default(),
            dma: 0,
            intena: 0,
            intreq: 0,
            led: false,
            noise: [0; LNOISE],
        }
    }
}

impl Paula {
    fn read(noise: &[u8; LNOISE], p: &Ptr, add: u32) -> i8 {
        let off = p.off.wrapping_add(add) as usize;
        (match &p.src {
            Src::None => 0,
            Src::Buf(b) => b.get(off).copied().unwrap_or(0),
            Src::Noise => noise.get(off).copied().unwrap_or(0),
        }) as i8
    }

    /// `move.w v,DMACON` restricted to the audio bits.
    pub fn dmacon(&mut self, v: u16) {
        let bits = v & 0x000F;
        if v & 0x8000 != 0 {
            let new = bits & !self.dma;
            self.dma |= bits;
            for c in 0..4 {
                if new & (1 << c) != 0 {
                    self.start(c);
                }
            }
        } else {
            let old = bits & self.dma;
            self.dma &= !bits;
            for c in 0..4 {
                if old & (1 << c) != 0 {
                    let ch = &mut self.ch[c];
                    ch.active = false;
                    ch.dat = ch.sample;
                }
            }
        }
    }

    /// `move.w v,INTENA` restricted to the audio bits.
    pub fn intena(&mut self, v: u16) {
        let bits = v & 0x0780;
        if v & 0x8000 != 0 {
            self.intena |= bits
        } else {
            self.intena &= !bits
        }
    }

    /// `move.w v,INTREQ` (acknowledge / request).
    pub fn intreq(&mut self, v: u16) {
        let bits = v & 0x0780;
        if v & 0x8000 != 0 {
            self.intreq |= bits
        } else {
            self.intreq &= !bits
        }
    }

    /// Next channel whose interrupt is requested and enabled, acknowledged.
    pub fn take_irq(&mut self) -> Option<usize> {
        let pending = self.intreq & self.intena;
        (0..4)
            .find(|&c| pending & (0x80 << c) != 0)
            .inspect(|&c| self.intreq &= !(0x80 << c))
    }

    /// DMA start: latch LC/LEN and request the interrupt.
    fn start(&mut self, c: usize) {
        let noise = &self.noise;
        let ch = &mut self.ch[c];
        ch.active = true;
        Self::latch(ch);
        ch.sample = Self::read(noise, &ch.cur, 0);
        ch.left = ch.period();
        self.intreq |= 0x80 << c;
    }

    fn latch(ch: &mut Channel) {
        ch.cur = ch.lc.clone();
        ch.words_left = if ch.len == 0 { 0x10000 } else { ch.len as u32 };
        ch.byte = 0;
    }

    /// Loads the next sample of a playing channel.
    fn advance(&mut self, c: usize) {
        let noise = &self.noise;
        let ch = &mut self.ch[c];
        ch.byte += 1;
        if ch.byte == 2 {
            ch.byte = 0;
            ch.cur.off = ch.cur.off.wrapping_add(2);
            ch.words_left -= 1;
            if ch.words_left == 0 {
                Self::latch(ch);
                self.intreq |= 0x80 << c;
            }
        }
        ch.sample = Self::read(noise, &ch.cur, ch.byte);
    }

    /// Average output of channel `c` over the next `dt` clock ticks, in
    /// -1.0..1.0 including the volume.
    fn channel_out(&mut self, c: usize, dt: f64) -> f32 {
        if !self.ch[c].active {
            let ch = &self.ch[c];
            return ch.dat as f32 * ch.volume() * (1.0 / (128.0 * 64.0));
        }
        let mut t = dt;
        let mut acc = 0.0f64;
        // Bounded: at most dt/period+1 iterations (dt is about 74 at 48 kHz,
        // periods are >= 113 in practice).
        while self.ch[c].left <= t {
            let ch = &self.ch[c];
            acc += ch.sample as f64 * ch.left;
            t -= ch.left;
            self.advance(c);
            let ch = &mut self.ch[c];
            ch.left = ch.period();
        }
        let ch = &mut self.ch[c];
        acc += ch.sample as f64 * t;
        ch.left -= t;
        (acc / dt) as f32 * ch.volume() * (1.0 / (128.0 * 64.0))
    }

    /// Produces one host sample: (left, right) before filtering, each the
    /// sum of two channels (L = 0+3, R = 1+2).
    pub fn output(&mut self, dt: f64) -> (f32, f32) {
        let c0 = self.channel_out(0, dt);
        let c1 = self.channel_out(1, dt);
        let c2 = self.channel_out(2, dt);
        let c3 = self.channel_out(3, dt);
        (c0 + c3, c1 + c2)
    }

    /// Current sample position of a channel (tests, `Sam Swapped`).
    pub fn is_playing(&self, c: usize) -> bool {
        self.ch[c].active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(len: usize) -> Arc<[u8]> {
        (0..len)
            .map(|i| if i < len / 2 { 100u8 } else { (-100i8) as u8 })
            .collect::<Vec<_>>()
            .into()
    }

    #[test]
    fn period_gives_frequency() {
        // A 2-byte cycle (+100, -100) at period 400: 3546895/400/2 Hz.
        let mut p = Paula::default();
        let buf = square(2);
        p.ch[0].lc = Ptr::new(&buf, 0);
        p.ch[0].len = 1;
        p.ch[0].per = 400;
        p.ch[0].vol = 64;
        p.dmacon(0x8001);
        let rate = 48000.0;
        let dt = PAL_CLOCK as f64 / rate;
        let n = 48000;
        let mut crossings = 0;
        let mut last = 0.0;
        for _ in 0..n {
            let (l, _) = p.output(dt);
            if last <= 0.0 && l > 0.0 {
                crossings += 1;
            }
            last = l;
        }
        let expected = PAL_CLOCK as f64 / 400.0 / 2.0;
        assert!(
            (crossings as f64 - expected).abs() < 3.0,
            "{crossings} vs {expected}"
        );
    }

    #[test]
    fn volume_scales_output() {
        let mut p = Paula::default();
        let buf: Arc<[u8]> = vec![64u8; 64].into();
        p.ch[1].lc = Ptr::new(&buf, 0);
        p.ch[1].len = 32;
        p.ch[1].per = 200;
        p.ch[1].vol = 32;
        p.dmacon(0x8002);
        let (l, r) = p.output(74.0);
        assert_eq!(l, 0.0);
        assert!((r - 0.25).abs() < 1e-6, "{r}");
        p.ch[1].vol = 64;
        let (_, r) = p.output(74.0);
        assert!((r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn relatch_loops_and_requests_interrupts() {
        let mut p = Paula::default();
        let a: Arc<[u8]> = vec![10u8; 4].into();
        let b: Arc<[u8]> = vec![20u8; 4].into();
        p.ch[2].lc = Ptr::new(&a, 0);
        p.ch[2].len = 2;
        p.ch[2].per = 100;
        p.ch[2].vol = 64;
        p.dmacon(0x8004);
        assert_eq!(p.intreq, 0x200);
        p.intreq(0x200);
        // Queue the next block: it plays after the 4 bytes of the first.
        p.ch[2].lc = Ptr::new(&b, 0);
        let mut seen = vec![];
        for _ in 0..12 {
            p.output(100.0);
            seen.push(p.ch[2].sample);
        }
        assert_eq!(&seen[..6], &[10, 10, 10, 20, 20, 20]);
        assert_ne!(p.intreq & 0x200, 0);
        // Disabled interrupts are not taken.
        assert_eq!(p.take_irq(), None);
        p.intena(0x8200);
        assert_eq!(p.take_irq(), Some(2));
        assert_eq!(p.take_irq(), None);
    }
}
