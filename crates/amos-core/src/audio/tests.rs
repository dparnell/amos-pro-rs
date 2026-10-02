//! Mixer and player tests. Set `AMOS_SOUND_WAV=dir` to also write what
//! the tests render as WAV files (for listening).

use super::*;
use crate::banks::{BankData, parse_banks};

const RATE: u32 = 22050;

fn example(path: &str) -> Option<Vec<u8>> {
    let p = format!(
        "{}/../../AMOS-Professional-365/AMOS/Examples/{path}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(p).ok()
}

fn bank_data(file: &[u8]) -> Arc<[u8]> {
    let b = parse_banks(file).expect("bank");
    match &b[0].data {
        BankData::Raw(d) => d.clone().into(),
        _ => panic!("not raw"),
    }
}

/// Renders `secs` seconds, returns interleaved stereo.
fn render(m: &mut Mixer, secs: f32) -> Vec<f32> {
    let mut out = vec![0.0; (RATE as f32 * secs) as usize * 2];
    // In blocks, as an audio callback would.
    for chunk in out.chunks_mut(1024) {
        m.render(chunk, 2, RATE);
    }
    out
}

/// RMS of each 1/50 s block.
fn energy(s: &[f32]) -> Vec<f32> {
    s.chunks((RATE / 50) as usize * 2)
        .map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt())
        .collect()
}

fn save_wav(name: &str, s: &[f32]) {
    let Ok(dir) = std::env::var("AMOS_SOUND_WAV") else {
        return;
    };
    let mut w = Vec::new();
    let data_len = (s.len() * 2) as u32;
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for x in s {
        w.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(format!("{dir}/{name}.wav"), w).unwrap();
}

fn check_sane(s: &[f32]) {
    assert!(s.iter().all(|x| x.is_finite() && x.abs() <= 1.0));
}

/// Upward zero crossings per second of the left channel.
fn frequency(s: &[f32]) -> f32 {
    let left: Vec<f32> = s.iter().step_by(2).copied().collect();
    let n = left
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    n as f32 * RATE as f32 / left.len() as f32
}

#[test]
fn silence_by_default() {
    let mut m = Mixer::default();
    let s = render(&mut m, 0.5);
    assert!(s.iter().all(|&x| x == 0.0));
}

#[test]
fn bell_pitch_and_envelope() {
    let mut m = Mixer::default();
    m.engine.bell(70).unwrap();
    let s = render(&mut m, 1.0);
    check_sane(&s);
    save_wav("bell", &s);
    let e = energy(&s);
    // EnvBell: 1,64 4,40 25,0 -> 30 VBLs of sound.
    assert!(e[1] > 0.05, "{e:?}");
    assert!(e[10] > 0.01);
    assert!(e[33..].iter().all(|&x| x < 0.002), "{e:?}");
    // Note 70: 8 byte square at period 251 -> 1766 Hz.
    let f = frequency(&s[..(RATE as usize / 5) * 2]);
    assert!((f - 1766.0).abs() < 40.0, "{f}");
    assert_eq!(m.engine.env_on, 0);
}

#[test]
fn boom_and_shoot_are_noise_bursts() {
    let mut m = Mixer::default();
    m.engine.boom();
    let s = render(&mut m, 1.5);
    check_sane(&s);
    save_wav("boom", &s);
    let e = energy(&s);
    assert!(e[2] > 0.05, "{e:?}");
    // The noise buffer plays once: 510 bytes at 2000*247/440 Hz = 0.45 s.
    assert!(e[25..].iter().all(|&x| x < 0.01), "{e:?}");
    assert!(frequency(&s[..RATE as usize / 5 * 2]) > 100.0);

    let mut m = Mixer::default();
    m.engine.shoot();
    let s = render(&mut m, 0.5);
    let e = energy(&s);
    assert!(e[1] > 0.05);
    assert!(e[14..].iter().all(|&x| x < 0.01), "{e:?}");
}

#[test]
fn play_wave_with_volume_and_play_off() {
    let mut m = Mixer::default();
    m.engine.vol(30, 0b0001).unwrap();
    m.engine
        .go_bel(0b0001, 46, -1, None, &Default::default())
        .unwrap();
    let s = render(&mut m, 0.3);
    // Voice 0 only: right channel silent.
    assert!(s.iter().skip(1).step_by(2).all(|&x| x.abs() < 1e-6));
    // A440 (note 46): 128 byte wave at period 3546895/(256*440) = 31.
    let f = frequency(&s[(RATE as usize / 10) * 2..]);
    assert!((f - 440.0).abs() < 15.0, "{f}");
    m.engine.env_off(0xF);
    let s = render(&mut m, 0.3);
    assert!(energy(&s)[2..].iter().all(|&x| x < 0.002));
}

#[test]
fn set_wave_and_envelope() {
    let mut e = Engine::default();
    let saw: Vec<u8> = (0..256).map(|i| i as u8).collect();
    assert_eq!(e.set_wave(2, &saw[..100]), Err(engine::ERR_WAVE_TOO_SHORT));
    e.set_wave(2, &saw).unwrap();
    let w = &e.waves[e.wave_index(2).unwrap()];
    // First decimated byte: (0 + 1) >> 1.
    assert_eq!(w.data[256], 0);
    assert_eq!(w.data[257], 2);
    e.set_envel(2, 1, 10, 32).unwrap();
    let w = &e.waves[e.wave_index(2).unwrap()];
    assert_eq!(&w.env[..6], &[1, 64, 10, 32, 0, 50]);
    assert_eq!(e.del_wave(1), Err(engine::ERR_WAVES_RESERVED));
    assert_eq!(e.del_wave(7), Err(engine::ERR_WAVE_NOT_DEFINED));
    e.wave_to(2, 0b11).unwrap();
    assert_eq!(e.wave_sel, [2, 2, 1, 1]);
    e.del_wave(2).unwrap();
    assert_eq!(e.wave_sel, [1; 4]);
}

#[test]
fn envelope_volume_ramp() {
    let mut e = Engine::default();
    e.go_bel(0b0100, 30, -1, None, &Default::default()).unwrap();
    // EnvDef phase 0: (1 VBL, 64) with Volume 56 -> 56 after one VBL.
    e.vbl();
    assert_eq!(e.hw.ch[2].vol, 56);
    // Phase 1: 4 VBLs to 55*56/64 = 48.
    for _ in 0..4 {
        e.vbl();
    }
    assert_eq!(e.hw.ch[2].vol, 48);
}

#[test]
fn sample_loop_and_end() {
    let data: Arc<[u8]> = (0..1000)
        .map(|i| ((i % 20) * 10) as u8)
        .collect::<Vec<_>>()
        .into();
    let s = SampleData {
        ptr: Ptr::new(&data, 0),
        len: 1000,
        freq: 10000,
    };
    let mut m = Mixer::default();
    m.engine.go_sam(0b0010, &s);
    let out = render(&mut m, 0.5);
    let e = energy(&out);
    // 1000 bytes at 10 kHz = 0.1 s.
    assert!(e[1] > 0.01);
    // (The test sample has a DC offset: the output DC blocker settles.)
    assert!(e[8..].iter().all(|&x| x < 0.002), "{e:?}");
    assert_eq!(m.engine.sam_swapped(1), 1);
    // Back to the music.
    assert_ne!(m.engine.mu_restart & 2, 0);

    let mut m = Mixer::default();
    m.engine.sl0(0, 0xF);
    m.engine.go_sam(0b0010, &s);
    let out = render(&mut m, 0.5);
    assert!(energy(&out)[20] > 0.01);
    assert_eq!(m.engine.sam_swapped(1), -1);
}

#[test]
fn long_sample_streams_in_chunks_and_swaps() {
    let a: Arc<[u8]> = vec![50u8; 10000].into();
    let b: Arc<[u8]> = vec![(-50i8) as u8; 6000].into();
    let mut m = Mixer::default();
    m.engine.go_sam(
        1,
        &SampleData {
            ptr: Ptr::new(&a, 0),
            len: 10000,
            freq: 20000,
        },
    );
    // The DMA start interrupt already queued the second chunk.
    assert_eq!(m.engine.sam_swapped(0), -1);
    m.engine.sam_swap(1, Some(Ptr::new(&b, 0)), 6000);
    assert_eq!(m.engine.sam_swapped(0), 0);
    // After 0.5 s the first buffer (0.5 s) has been swapped for the second.
    render(&mut m, 0.55);
    assert!(m.engine.sami[0].radr.is_none());
    assert!(m.engine.hw.ch[0].lc.off < 6000);
    assert_eq!(m.engine.sam_swapped(0), -1);
    render(&mut m, 0.5);
    assert_eq!(m.engine.sam_swapped(0), 1);
}

#[test]
fn music_bank_plays() {
    let Some(file) = example("Music/Music.abk") else {
        return;
    };
    let mut m = Mixer::default();
    m.engine.music_bank_check(Some(bank_data(&file)));
    let songs = m.engine.music.as_ref().unwrap().songs();
    assert!(songs >= 1);
    assert_eq!(
        m.engine.music(songs as i32 + 1),
        Err(engine::ERR_MUSIC_NOT_DEFINED)
    );
    m.engine.music(1).unwrap();
    let mut vu = 0;
    let mut all = Vec::new();
    for _ in 0..20 * 5 {
        all.extend(render(&mut m, 0.2));
        for v in 0..4 {
            vu = vu.max(m.engine.vumeter(v).unwrap());
        }
    }
    check_sane(&all);
    save_wav("music1", &all);
    assert!(vu > 0);
    let per_sec: Vec<f32> = energy(&all)
        .chunks(50)
        .map(|c| c.iter().sum::<f32>() / 50.0)
        .collect();
    let loud = per_sec.iter().filter(|&&x| x > 0.01).count();
    assert!(loud >= 15, "{per_sec:?}");
    m.engine.music_off();
    let s = render(&mut m, 0.5);
    assert!(energy(&s)[5..].iter().all(|&x| x < 0.002));
}

#[test]
fn music_and_effects_share_voices() {
    let Some(file) = example("Music/Music.abk") else {
        return;
    };
    let mut e = Engine::default();
    e.music_bank_check(Some(bank_data(&file)));
    e.music(1).unwrap();
    for _ in 0..20 {
        e.vbl();
    }
    e.go_bel(0b0001, 40, -1, None, &Default::default()).unwrap();
    assert_eq!(e.mu_dmask, 0b1110);
    assert!(!e.mu_chip[0]);
    // The envelope ends (31 VBLs): the voice goes back to the music.
    for _ in 0..40 {
        e.vbl();
    }
    assert_eq!(e.mu_dmask, 0xF);
    assert!(e.mu_chip[0]);
}

#[test]
fn nested_musics_and_stop() {
    let Some(file) = example("Music/Music.abk") else {
        return;
    };
    let mut e = Engine::default();
    e.music_bank_check(Some(bank_data(&file)));
    e.music(1).unwrap();
    e.music(1).unwrap();
    assert_eq!(e.mu_number, 2);
    e.music_stop();
    for _ in 0..10 {
        e.vbl();
    }
    assert_eq!(e.mu_number, 1);
    assert_eq!(e.mu_base, Some(0));
    e.tempo(100).unwrap();
    assert_eq!(e.tempo(101), Err(engine::ERR_IFONC));
    e.music_off();
    assert_eq!(e.mu_base, None);
}

#[test]
fn sample_bank_format() {
    let Some(file) = example("Samples/Instruments.abk") else {
        return;
    };
    let b = parse_banks(&file).unwrap();
    assert!(b[0].name.starts_with("Samples"));
}

#[test]
fn tracker_module_plays() {
    let Some(file) = example("Music/Mod.Tracker") else {
        return;
    };
    let mut d = file.clone();
    let starts = tracker::prepare_module(&mut d);
    let mut m = Mixer::default();
    m.engine.tracker.looping = true;
    m.engine.track_play(d.into(), starts);
    let s = render(&mut m, 10.0);
    check_sane(&s);
    save_wav("tracker", &s);
    let per_sec: Vec<f32> = energy(&s)
        .chunks(50)
        .map(|c| c.iter().sum::<f32>() / 50.0)
        .collect();
    assert!(
        per_sec.iter().filter(|&&x| x > 0.01).count() >= 8,
        "{per_sec:?}"
    );
    m.engine.tracker_stop();
    let s = render(&mut m, 0.3);
    assert!(energy(&s)[5..].iter().all(|&x| x < 0.002));
}

#[test]
fn led_filter_attenuates_highs() {
    let hi = |led: bool| {
        let mut m = Mixer::default();
        m.engine.hw.led = led;
        m.engine
            .go_bel(1, 90, -1, None, &Default::default())
            .unwrap();
        energy(&render(&mut m, 0.2))[3]
    };
    let (off, on) = (hi(false), hi(true));
    assert!(on < off * 0.7, "{on} {off}");
}

#[test]
fn stereo_separation() {
    let mut m = Mixer::default();
    m.config.stereo_separation = 0.0;
    m.engine
        .go_bel(1, 46, -1, None, &Default::default())
        .unwrap();
    let s = render(&mut m, 0.1);
    assert!(s.chunks(2).all(|f| (f[0] - f[1]).abs() < 1e-6));
    assert!(s.iter().any(|&x| x != 0.0));
}

#[test]
fn headless_ticks_from_host_vbl() {
    let mut m = Mixer::default();
    m.set_headless();
    let t = m.engine.ticks;
    for _ in 0..10 {
        m.host_vbl();
    }
    assert_eq!(m.engine.ticks - t, 10);
    // A render call takes over.
    let mut buf = vec![0.0; 2 * 441];
    m.render(&mut buf, 2, 22050);
    let t = m.engine.ticks;
    m.host_vbl();
    assert_eq!(m.engine.ticks, t);
}
