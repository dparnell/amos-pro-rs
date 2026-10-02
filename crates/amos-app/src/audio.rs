//! cpal output stream feeding the AMOS software mixer.

use std::sync::{Arc, Mutex};

use amos_core::audio::AudioSource;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct AudioOutput {
    _stream: cpal::Stream,
}

impl AudioOutput {
    /// Opens the default output device. Browsers only allow this after a
    /// user gesture, so on the web it is called on the first key or click.
    pub fn start<S: AudioSource + 'static>(source: Arc<Mutex<S>>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let rate = config.sample_rate;
        log::info!("Audio output: {} channels at {} Hz ({:?})", channels, rate, sample_format);

        let on_error = |err| log::error!("Audio stream error: {err}");
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |out: &mut [f32], _| render(&source, out, channels, rate),
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => {
                let mut buf = Vec::new();
                device.build_output_stream(
                    config,
                    move |out: &mut [i16], _| {
                        buf.resize(out.len(), 0.0);
                        render(&source, &mut buf, channels, rate);
                        for (o, s) in out.iter_mut().zip(&buf) {
                            *o = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        }
                    },
                    on_error,
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let mut buf = Vec::new();
                device.build_output_stream(
                    config,
                    move |out: &mut [u16], _| {
                        buf.resize(out.len(), 0.0);
                        render(&source, &mut buf, channels, rate);
                        for (o, s) in out.iter_mut().zip(&buf) {
                            *o = ((s.clamp(-1.0, 1.0) + 1.0) * 32767.5) as u16;
                        }
                    },
                    on_error,
                    None,
                )
            }
            other => return Err(format!("unsupported sample format {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { _stream: stream })
    }
}

fn render<S: AudioSource>(source: &Mutex<S>, out: &mut [f32], channels: usize, rate: u32) {
    match source.lock() {
        Ok(mut s) => s.render(out, channels, rate),
        Err(_) => out.fill(0.0),
    }
}
