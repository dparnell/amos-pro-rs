//! Software emulation of the Amiga audio output.

/// Something that produces interleaved audio samples for the host.
pub trait AudioSource: Send {
    /// Fill `out` with interleaved samples in -1.0..=1.0 for `channels`
    /// channels at `sample_rate` Hz.
    fn render(&mut self, out: &mut [f32], channels: usize, sample_rate: u32);
}

/// Placeholder mixer: produces silence until Paula emulation is wired in.
#[derive(Default)]
pub struct Mixer {}

impl AudioSource for Mixer {
    fn render(&mut self, out: &mut [f32], _channels: usize, _sample_rate: u32) {
        out.fill(0.0);
    }
}

/// Sound state owned by the machine (music, samples, effects).
pub struct SoundState {
    pub mixer: std::sync::Arc<std::sync::Mutex<Mixer>>,
}

impl SoundState {
    pub fn new(mixer: std::sync::Arc<std::sync::Mutex<Mixer>>) -> Self {
        SoundState { mixer }
    }
}
