//! Software emulation of the Amiga audio output and of the AMOS Music
//! extension (`+Music.s`).
//!
//! * [`paula`]: the sound chip at register level (DMA, double buffered
//!   LC/LEN, audio interrupts).
//! * [`engine`]: the extension's state and routines (Play, Bell, samples,
//!   envelopes, voice sharing), [`music`] the AMOS music player,
//!   [`tracker`] the NoiseTracker replayer, [`narrator`] a small speech
//!   synthesizer standing in for `narrator.device`.
//! * [`Mixer`]: what the platform's audio thread locks. It owns the engine
//!   and runs its 50 Hz interrupt routine every `sample_rate / 50` produced
//!   samples, so music timing follows the audio clock, not the video.
//!   The interpreter changes the engine through the same lock.

pub mod engine;
pub mod music;
pub mod narrator;
pub mod paula;
pub mod tables;
pub mod tracker;

use std::sync::{Arc, Mutex, MutexGuard};

pub use engine::{Engine, SampleData};
pub use paula::{PAL_CLOCK, Ptr, Src};

/// Something that produces interleaved audio samples for the host.
pub trait AudioSource: Send {
    /// Fill `out` with interleaved samples in -1.0..=1.0 for `channels`
    /// channels at `sample_rate` Hz.
    fn render(&mut self, out: &mut [f32], channels: usize, sample_rate: u32);
}

/// Output options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerConfig {
    /// 1.0 = hard Amiga panning (voices 0+3 left, 1+2 right), 0.0 = mono.
    pub stereo_separation: f32,
    /// Emulate the A500 fixed output low-pass filter (about 4.4 kHz).
    pub a500_filter: bool,
    /// Overall gain.
    pub gain: f32,
}

impl Default for MixerConfig {
    fn default() -> Self {
        MixerConfig {
            stereo_separation: 1.0,
            a500_filter: true,
            gain: 1.0,
        }
    }
}

/// RBJ biquad low-pass (the LED filter: 2-pole Butterworth, 3.3 kHz).
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn lowpass(fs: f32, fc: f32, q: f32) -> Self {
        let w0 = 2.0 * std::f32::consts::PI * (fc / fs).min(0.49);
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 - c) / 2.0 / a0,
            b1: (1.0 - c) / a0,
            b2: (1.0 - c) / 2.0 / a0,
            a1: -2.0 * c / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        // Transposed direct form II.
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Output filters of one stereo side.
#[derive(Clone, Copy, Debug, Default)]
struct Filters {
    rc: f32,
    led: Biquad,
    dc_x: f32,
    dc_y: f32,
}

/// Number of host VBLs without any `render` call after which the mixer
/// runs itself from [`Mixer::host_vbl`] (no audio device, or the browser
/// has not started audio yet).
const HEADLESS_AFTER: u32 = 25;
const HEADLESS_RATE: u32 = 22050;

/// The Amiga audio hardware plus the Music extension, rendered for the host.
pub struct Mixer {
    pub engine: Engine,
    pub config: MixerConfig,
    rate: u32,
    /// Samples until the next 50 Hz tick.
    tick_left: f64,
    filters: [Filters; 2],
    rc_a: f32,
    dc_r: f32,
    vbls_since_render: u32,
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            engine: Engine::default(),
            config: MixerConfig::default(),
            rate: 0,
            tick_left: 0.0,
            filters: Default::default(),
            rc_a: 1.0,
            dc_r: 1.0,
            vbls_since_render: 0,
            scratch: Vec::new(),
        }
    }
}

impl Mixer {
    fn set_rate(&mut self, rate: u32) {
        if rate == self.rate {
            return;
        }
        self.rate = rate;
        let fs = rate as f32;
        let two_pi = 2.0 * std::f32::consts::PI;
        // A500: R=360 ohm, C=0.1 uF -> 4.42 kHz, 1 pole.
        self.rc_a = 1.0 - (-two_pi * 4420.0 / fs).exp();
        // About 5 Hz DC blocking (output coupling capacitor).
        self.dc_r = 1.0 - two_pi * 5.0 / fs;
        for f in self.filters.iter_mut() {
            // A500 LED filter: Sallen-Key 3275 Hz, Q 0.660.
            f.led = Biquad::lowpass(fs, 3275.0, 0.660);
        }
    }

    /// Produces `frames` stereo frames into `out` (interleaved by
    /// `channels`).
    fn render_frames(&mut self, out: &mut [f32], channels: usize, rate: u32) {
        let rate = rate.max(1000);
        self.set_rate(rate);
        let dt = self.engine.clock as f64 / rate as f64;
        let per_tick = rate as f64 / 50.0;
        let sep = self.config.stereo_separation.clamp(0.0, 1.0);
        let (ga, gb) = ((1.0 + sep) * 0.5, (1.0 - sep) * 0.5);
        let gain = self.config.gain * 0.5;
        for frame in out.chunks_mut(channels.max(1)) {
            if self.tick_left <= 0.0 {
                self.engine.vbl();
                self.tick_left += per_tick;
            }
            self.tick_left -= 1.0;
            let (l, r) = self.engine.hw.output(dt);
            if self.engine.hw.intreq & self.engine.hw.intena != 0 {
                self.engine.service_irqs();
            }
            let mut lr = [l * ga + r * gb, r * ga + l * gb];
            let led = self.engine.hw.led;
            for (x, f) in lr.iter_mut().zip(self.filters.iter_mut()) {
                let mut s = *x;
                if self.config.a500_filter {
                    f.rc += self.rc_a * (s - f.rc);
                    s = f.rc;
                }
                if led {
                    s = f.led.run(s);
                } else {
                    // Keep the filter state following the signal so turning
                    // it on does not click.
                    f.led.run(s);
                }
                let y = s - f.dc_x + self.dc_r * f.dc_y;
                f.dc_x = s;
                f.dc_y = y;
                *x = (y * gain).clamp(-1.0, 1.0);
            }
            match frame.len() {
                0 => {}
                1 => frame[0] = (lr[0] + lr[1]) * 0.5,
                n => {
                    frame[0] = lr[0];
                    frame[1] = lr[1];
                    for s in frame[2..n].iter_mut() {
                        *s = 0.0;
                    }
                }
            }
        }
    }

    /// Called by the machine at each video VBL. When no audio device pulls
    /// samples (headless, tests, browser before the first gesture), runs
    /// 1/50 s of emulation so music and samples still progress.
    pub fn host_vbl(&mut self) {
        self.vbls_since_render = self.vbls_since_render.saturating_add(1);
        if self.vbls_since_render > HEADLESS_AFTER {
            let mut buf = std::mem::take(&mut self.scratch);
            buf.resize((HEADLESS_RATE / 50) as usize * 2, 0.0);
            self.render_frames(&mut buf, 2, HEADLESS_RATE);
            self.scratch = buf;
        }
    }

    /// Forces headless mode (tests, tools without audio output).
    pub fn set_headless(&mut self) {
        self.vbls_since_render = HEADLESS_AFTER + 1;
    }
}

impl AudioSource for Mixer {
    fn render(&mut self, out: &mut [f32], channels: usize, sample_rate: u32) {
        self.vbls_since_render = 0;
        self.render_frames(out, channels, sample_rate);
    }
}

/// Sound state owned by the machine: the shared mixer plus what only the
/// interpreter side needs.
pub struct SoundState {
    pub mixer: Arc<Mutex<Mixer>>,
    /// `BankSet::generation` seen by the last bank check.
    pub bank_generation: Option<u64>,
    pub narrator: narrator::Settings,
}

impl SoundState {
    pub fn new(mixer: Arc<Mutex<Mixer>>) -> Self {
        SoundState {
            mixer,
            bank_generation: None,
            narrator: narrator::Settings::default(),
        }
    }

    /// Locks the mixer (a poisoned lock is still usable: the state is
    /// plain data).
    pub fn lock(&self) -> MutexGuard<'_, Mixer> {
        self.mixer.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests;
