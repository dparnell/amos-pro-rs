//! Stand-in for the Amiga `translator.library` + `narrator.device`
//! (`Say`, `Set Talk`, `Talk Misc`, `+Music.s:2419-2650`).
//!
//! The original translates English to phonemes and sings them with a
//! formant synthesizer. This is a much smaller approximation in the same
//! spirit: crude letter-to-sound rules, then a cascade of three formant
//! resonators driven by a glottal pulse (vowels, liquids, nasals) or noise
//! (fricatives, plosive bursts). It gives the typical robotic Amiga voice,
//! not the exact one.

/// Narrator parameters (`NarInit`, `+Music.s:2490`).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Words per minute (40..400).
    pub rate: u16,
    /// Base pitch in Hz (65..320).
    pub pitch: u16,
    /// 0 natural, 1 robotic (monotone).
    pub mode: u16,
    /// 0 male, 1 female.
    pub sex: u16,
    pub volume: u16,
    /// Output sample frequency (5000..25000).
    pub freq: u16,
    /// Asynchronous speech in progress (`Say a$,1`).
    pub speaking: bool,
    /// Last `Mouth Read` result (width, height).
    pub mouth: (i8, i8),
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rate: 150,
            pitch: 110,
            mode: 0,
            sex: 0,
            volume: 63,
            freq: 22200,
            speaking: false,
            mouth: (0, 0),
        }
    }
}

/// Kind of sound of a phoneme.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Vowel,
    /// Liquids, glides and nasals: voiced, quieter.
    Sonorant,
    /// Noise only (S, SH, F, TH, H).
    Fricative,
    /// Voiced fricative (Z, V, DH, ZH).
    VoicedFric,
    /// Closure then burst (P, T, K).
    Stop,
    /// Voiced closure then burst (B, D, G).
    VoicedStop,
    Pause,
}

#[derive(Clone, Copy, Debug)]
struct Phoneme {
    kind: Kind,
    /// Formants (Hz). For noise sounds f3 is the noise centre.
    f: [f32; 3],
    /// Duration in ms at 150 words per minute.
    ms: f32,
    /// Second target for diphthongs.
    to: Option<[f32; 3]>,
}

const fn ph(kind: Kind, f1: f32, f2: f32, f3: f32, ms: f32) -> Phoneme {
    Phoneme {
        kind,
        f: [f1, f2, f3],
        ms,
        to: None,
    }
}

const fn diph(a: [f32; 3], b: [f32; 3], ms: f32) -> Phoneme {
    Phoneme {
        kind: Kind::Vowel,
        f: a,
        ms,
        to: Some(b),
    }
}

use Kind::*;

const IY: [f32; 3] = [270.0, 2290.0, 3010.0];
const IH: [f32; 3] = [390.0, 1990.0, 2550.0];
const EH: [f32; 3] = [530.0, 1840.0, 2480.0];
const AE: [f32; 3] = [660.0, 1720.0, 2410.0];
const AA: [f32; 3] = [730.0, 1090.0, 2440.0];
const AO: [f32; 3] = [570.0, 840.0, 2410.0];
const UH: [f32; 3] = [440.0, 1020.0, 2240.0];
const UW: [f32; 3] = [300.0, 870.0, 2240.0];
const AH: [f32; 3] = [640.0, 1190.0, 2390.0];

/// Phoneme codes (narrator style) and their parameters.
fn phoneme(code: &str) -> Option<Phoneme> {
    Some(match code {
        "IY" => ph(Vowel, IY[0], IY[1], IY[2], 130.0),
        "IH" => ph(Vowel, IH[0], IH[1], IH[2], 90.0),
        "EH" => ph(Vowel, EH[0], EH[1], EH[2], 100.0),
        "AE" => ph(Vowel, AE[0], AE[1], AE[2], 130.0),
        "AA" => ph(Vowel, AA[0], AA[1], AA[2], 130.0),
        "AO" => ph(Vowel, AO[0], AO[1], AO[2], 130.0),
        "UH" => ph(Vowel, UH[0], UH[1], UH[2], 90.0),
        "UW" => ph(Vowel, UW[0], UW[1], UW[2], 130.0),
        "AH" => ph(Vowel, AH[0], AH[1], AH[2], 90.0),
        "AX" => ph(Vowel, 500.0, 1400.0, 2300.0, 60.0),
        "ER" => ph(Vowel, 490.0, 1350.0, 1690.0, 130.0),
        "EY" => diph(EH, IY, 170.0),
        "AY" => diph(AA, IY, 190.0),
        "OY" => diph(AO, IY, 200.0),
        "AW" => diph(AA, UW, 190.0),
        "OW" => diph(AO, UW, 170.0),
        "L" => ph(Sonorant, 360.0, 1300.0, 2700.0, 70.0),
        "R" => ph(Sonorant, 330.0, 1060.0, 1380.0, 70.0),
        "W" => ph(Sonorant, 290.0, 610.0, 2150.0, 60.0),
        "Y" => ph(Sonorant, 260.0, 2070.0, 3020.0, 60.0),
        "M" => ph(Sonorant, 250.0, 1100.0, 2200.0, 70.0),
        "N" => ph(Sonorant, 250.0, 1700.0, 2600.0, 70.0),
        "NX" => ph(Sonorant, 250.0, 2300.0, 2750.0, 70.0),
        "S" => ph(Fricative, 0.0, 0.0, 5500.0, 100.0),
        "SH" => ph(Fricative, 0.0, 0.0, 2600.0, 100.0),
        "F" => ph(Fricative, 0.0, 0.0, 7000.0, 90.0),
        "TH" => ph(Fricative, 0.0, 0.0, 6500.0, 90.0),
        "/H" => ph(Fricative, 0.0, 0.0, 1500.0, 60.0),
        "Z" => ph(VoicedFric, 250.0, 1700.0, 5500.0, 80.0),
        "ZH" => ph(VoicedFric, 250.0, 1700.0, 2600.0, 80.0),
        "V" => ph(VoicedFric, 250.0, 1100.0, 7000.0, 70.0),
        "DH" => ph(VoicedFric, 250.0, 1500.0, 6500.0, 60.0),
        "P" => ph(Stop, 0.0, 0.0, 1000.0, 80.0),
        "T" => ph(Stop, 0.0, 0.0, 4000.0, 80.0),
        "K" => ph(Stop, 0.0, 0.0, 2000.0, 80.0),
        "B" => ph(VoicedStop, 200.0, 900.0, 1000.0, 60.0),
        "D" => ph(VoicedStop, 200.0, 1700.0, 4000.0, 60.0),
        "G" => ph(VoicedStop, 200.0, 2000.0, 2000.0, 60.0),
        " " => ph(Pause, 0.0, 0.0, 0.0, 50.0),
        "." => ph(Pause, 0.0, 0.0, 0.0, 250.0),
        _ => return None,
    })
}

/// Crude English letter-to-sound rules (stands in for translator.library).
fn translate_word(w: &[u8], out: &mut Vec<&'static str>) {
    let n = w.len();
    let at = |i: usize| -> u8 { if i < n { w[i] } else { 0 } };
    let vowel = |c: u8| matches!(c, b'A' | b'E' | b'I' | b'O' | b'U' | b'Y');
    // "Magic e": vowel + consonant + final E makes the vowel long.
    let magic = |i: usize| n >= 3 && i + 2 == n - 1 && at(n - 1) == b'E' && !vowel(at(i + 1));
    let mut i = 0;
    while i < n {
        let c = w[i];
        let two = [c, at(i + 1)];
        let (codes, len): (&[&'static str], usize) = match &two {
            b"TH" => (&["TH"], 2),
            b"SH" => (&["SH"], 2),
            b"CH" => (&["T", "SH"], 2),
            b"PH" => (&["F"], 2),
            b"WH" => (&["W"], 2),
            b"CK" => (&["K"], 2),
            b"NG" => (&["NX"], 2),
            b"QU" => (&["K", "W"], 2),
            b"GH" => (&[], 2),
            b"EE" | b"EA" | b"IE" => (&["IY"], 2),
            b"OO" => (&["UW"], 2),
            b"OU" => (&["AW"], 2),
            b"OW" => (&["OW"], 2),
            b"AI" | b"AY" | b"EY" => (&["EY"], 2),
            b"OI" | b"OY" => (&["OY"], 2),
            b"AU" | b"AW" => (&["AO"], 2),
            b"ER" | b"IR" | b"UR" => (&["ER"], 2),
            b"AR" => (&["AA", "R"], 2),
            b"OR" => (&["AO", "R"], 2),
            _ => {
                let codes: &[&'static str] = match c {
                    b'A' if magic(i) => &["EY"],
                    b'A' => &["AE"],
                    b'E' if i == n - 1 && n > 2 => &[],
                    b'E' if n <= 2 && i == n - 1 => &["IY"],
                    b'E' => &["EH"],
                    b'I' if magic(i) => &["AY"],
                    b'I' => &["IH"],
                    b'O' if magic(i) || i == n - 1 => &["OW"],
                    b'O' => &["AA"],
                    b'U' if magic(i) => &["UW"],
                    b'U' => &["AH"],
                    b'Y' if i == 0 => &["Y"],
                    b'Y' if n <= 3 => &["AY"],
                    b'Y' => &["IY"],
                    b'B' => &["B"],
                    b'C' if matches!(at(i + 1), b'E' | b'I' | b'Y') => &["S"],
                    b'C' => &["K"],
                    b'D' => &["D"],
                    b'F' => &["F"],
                    b'G' => &["G"],
                    b'H' => &["/H"],
                    b'J' => &["D", "ZH"],
                    b'K' => &["K"],
                    b'L' => &["L"],
                    b'M' => &["M"],
                    b'N' => &["N"],
                    b'P' => &["P"],
                    b'Q' => &["K"],
                    b'R' => &["R"],
                    b'S' if i > 0 && i == n - 1 && vowel(at(i - 1)) => &["Z"],
                    b'S' => &["S"],
                    b'T' => &["T"],
                    b'V' => &["V"],
                    b'W' => &["W"],
                    b'X' => &["K", "S"],
                    b'Z' => &["Z"],
                    _ => &[],
                };
                (codes, 1)
            }
        };
        // Doubled consonants sound once.
        if len == 1 && i > 0 && w[i - 1] == c && !vowel(c) {
            i += 1;
            continue;
        }
        out.extend_from_slice(codes);
        i += len;
    }
}

const DIGITS: [&str; 10] = [
    "ZERO", "WUN", "TOO", "THREE", "FOR", "FIVE", "SIKS", "SEVEN", "AYT", "NINE",
];

/// Text to phoneme codes. Text starting with `~` is already phonemes
/// (narrator codes separated by spaces or packed; stress digits ignored).
fn translate(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if let Some(ph) = text.strip_prefix('~') {
        let b: Vec<u8> = ph.bytes().map(|c| c.to_ascii_uppercase()).collect();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            if c.is_ascii_digit() {
                i += 1;
                continue;
            }
            if matches!(c, b'.' | b',' | b'?' | b'!' | b'-') {
                out.push(".");
                i += 1;
                continue;
            }
            if c == b' ' {
                out.push(" ");
                i += 1;
                continue;
            }
            let two = std::str::from_utf8(&b[i..(i + 2).min(b.len())]).unwrap_or("");
            if two.len() == 2
                && let Some(code) = PHONEME_CODES.iter().find(|&&p| p == two)
            {
                out.push(code);
                i += 2;
                continue;
            }
            let one = std::str::from_utf8(&b[i..i + 1]).unwrap_or("");
            if one == "H" {
                out.push("/H");
            } else if let Some(code) = PHONEME_CODES.iter().find(|&&p| p == one) {
                out.push(code);
            }
            i += 1;
        }
        return out;
    }
    let mut word = Vec::new();
    let flush = |word: &mut Vec<u8>, out: &mut Vec<&'static str>| {
        if !word.is_empty() {
            translate_word(word, out);
            out.push(" ");
            word.clear();
        }
    };
    for c in text.bytes() {
        let c = c.to_ascii_uppercase();
        if c.is_ascii_uppercase() || c == b'\'' {
            if c != b'\'' {
                word.push(c);
            }
        } else if c.is_ascii_digit() {
            flush(&mut word, &mut out);
            word.extend_from_slice(DIGITS[(c - b'0') as usize].as_bytes());
            flush(&mut word, &mut out);
        } else {
            flush(&mut word, &mut out);
            if matches!(c, b'.' | b',' | b'?' | b'!' | b';' | b':') {
                out.push(".");
            }
        }
    }
    flush(&mut word, &mut out);
    out
}

const PHONEME_CODES: [&str; 41] = [
    "IY", "IH", "EH", "AE", "AA", "AO", "UH", "UW", "AH", "AX", "ER", "EY", "AY", "OY", "AW", "OW",
    "L", "R", "W", "Y", "M", "N", "NX", "S", "SH", "F", "TH", "/H", "Z", "ZH", "V", "DH", "P", "T",
    "K", "B", "D", "G", " ", ".", "Q",
];

/// Klatt two-pole resonator.
#[derive(Clone, Copy, Default)]
struct Reson {
    a: f32,
    b: f32,
    c: f32,
    y1: f32,
    y2: f32,
}

impl Reson {
    fn set(&mut self, f: f32, bw: f32, fs: f32) {
        let t = 1.0 / fs;
        let f = f.min(fs * 0.45);
        self.c = -(-2.0 * std::f32::consts::PI * bw * t).exp();
        self.b = 2.0
            * (-std::f32::consts::PI * bw * t).exp()
            * (2.0 * std::f32::consts::PI * f * t).cos();
        self.a = 1.0 - self.b - self.c;
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.a * x + self.b * self.y1 + self.c * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// Synthesizes `text` as signed 8-bit samples at `s.freq` Hz.
pub fn synthesize(text: &str, s: &Settings) -> Vec<u8> {
    let codes = translate(text);
    let phonemes: Vec<Phoneme> = codes.iter().filter_map(|c| phoneme(c)).collect();
    if phonemes.iter().all(|p| p.kind == Pause) {
        return Vec::new();
    }
    let fs = s.freq.clamp(5000, 25000) as f32;
    let speed = 150.0 / s.rate.clamp(40, 400) as f32;
    let fscale = if s.sex == 1 { 1.15 } else { 1.0 };
    let base = s.pitch.clamp(65, 320) as f32;
    let total_ms: f32 = phonemes.iter().map(|p| p.ms * speed).sum();
    let mut out: Vec<f32> = Vec::with_capacity((total_ms / 1000.0 * fs) as usize + 16);
    let mut res = [Reson::default(); 3];
    let mut fric = Reson::default();
    let mut prev = [500.0f32, 1500.0, 2500.0];
    let mut phase = 0.0f32;
    let mut seed = 0x1234_5678u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let mut last_glottal = 0.0f32;
    let mut t_ms = 0.0f32;
    for p in phonemes.iter() {
        let n = (p.ms * speed / 1000.0 * fs) as usize;
        let voiced = matches!(p.kind, Vowel | Sonorant | VoicedFric | VoicedStop);
        let target = if voiced { p.f } else { prev };
        for i in 0..n {
            let x = i as f32 / n.max(1) as f32;
            // Formant transitions: the first 30% glides from the previous
            // phoneme; diphthongs glide over their whole length.
            let mut f = target;
            if let Some(to) = p.to {
                for k in 0..3 {
                    f[k] = p.f[k] + (to[k] - p.f[k]) * x;
                }
            } else if x < 0.3 {
                for k in 0..3 {
                    f[k] = prev[k] + (target[k] - prev[k]) * (x / 0.3);
                }
            }
            if i % 32 == 0 {
                let bws = [70.0, 100.0, 160.0];
                for k in 0..3 {
                    res[k].set(f[k] * fscale, bws[k], fs);
                }
                let center = p.f[2];
                fric.set(center.max(500.0), center * 0.5, fs);
            }
            // Pitch: natural speech declines over the sentence, robotic
            // mode is monotone.
            let pos = (t_ms + i as f32 / fs * 1000.0) / total_ms.max(1.0);
            let f0 = if s.mode == 1 {
                base
            } else {
                base * (1.1 - 0.25 * pos)
            };
            phase += f0 / fs;
            if phase >= 1.0 {
                phase -= 1.0;
            }
            // Rosenberg glottal pulse, differentiated (lip radiation).
            let g = if phase < 0.4 {
                0.5 * (1.0 - (std::f32::consts::PI * phase / 0.4).cos())
            } else if phase < 0.56 {
                (std::f32::consts::PI * (phase - 0.4) / 0.32).cos()
            } else {
                0.0
            };
            let src = g - last_glottal;
            last_glottal = g;
            let (av, an) = match p.kind {
                Vowel => (1.0, 0.0),
                Sonorant => (0.5, 0.0),
                Fricative => (0.0, if p.f[2] > 6000.0 { 0.25 } else { 0.6 }),
                VoicedFric => (0.4, 0.3),
                Stop => (0.0, if x > 0.65 { 0.8 } else { 0.0 }),
                VoicedStop => (
                    if x < 0.6 { 0.15 } else { 0.6 },
                    if (0.6..0.75).contains(&x) { 0.6 } else { 0.0 },
                ),
                Pause => (0.0, 0.0),
            };
            // Short fades at phoneme edges avoid clicks.
            let edge = (x * 20.0).min((1.0 - x) * 20.0).min(1.0);
            let mut v = src * av * 8.0;
            for r in res.iter_mut() {
                v = r.run(v);
            }
            let fr = fric.run(noise() * an);
            out.push((v + fr * 0.6) * edge.max(if voiced { 0.3 } else { 0.0 }));
        }
        if voiced {
            prev = p.to.unwrap_or(p.f);
        }
        t_ms += p.ms * speed;
    }
    // Normalise on the RMS level with a soft limiter (bursts and resonator
    // transients would make peak normalisation too quiet).
    let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len().max(1) as f32)
        .sqrt()
        .max(1e-6);
    let gain = 45.0 / rms;
    out.iter()
        .map(|&x| ((127.0 * (x * gain / 127.0).tanh()).round() as i8) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_words() {
        let p = translate("Hello, world 7");
        assert!(
            p.contains(&"/H") && p.contains(&"L") && p.contains(&"OW"),
            "{p:?}"
        );
        assert!(p.contains(&"."));
        let p = translate("~HEH4LOW");
        assert_eq!(&p[..], &["/H", "EH", "L", "OW"]);
    }

    #[test]
    fn synthesizes_audio() {
        let s = Settings::default();
        let pcm = synthesize("Welcome to AMOS Professional", &s);
        // About 2 seconds at 22200 Hz, not silent.
        assert!(pcm.len() > 22200 && pcm.len() < 22200 * 5, "{}", pcm.len());
        let loud = pcm.iter().filter(|&&b| (b as i8).abs() > 20).count();
        assert!(loud > pcm.len() / 10);
        assert!(synthesize("", &s).is_empty());
        let fast = synthesize(
            "Welcome to AMOS Professional",
            &Settings {
                rate: 300,
                ..s.clone()
            },
        );
        assert!(fast.len() < pcm.len() * 2 / 3);
    }
}
