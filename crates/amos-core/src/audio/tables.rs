//! Constant tables of the Music extension (`+Music.s:2118-2156`, `:2279`).

/// Vibrato table `Sinus` (`+Music.s:2120`), also `mt_sin` of the tracker.
pub const SINUS: [u8; 32] = [
    0x00, 0x18, 0x31, 0x4a, 0x61, 0x78, 0x8d, 0xa1, 0xb4, 0xc5, 0xd4, 0xe0, 0xeb, 0xf4, 0xfa, 0xfd,
    0xff, 0xfd, 0xfa, 0xf4, 0xeb, 0xe0, 0xd4, 0xc5, 0xb4, 0xa1, 0x8d, 0x78, 0x61, 0x4a, 0x31, 0x18,
];

/// `Periods` (`+Music.s:2124`) followed by the words that come after it in
/// memory (`EnvDef`, `EnvShoot`): the AMOS music arpeggio can index past the
/// end of the table, and reads these.
#[rustfmt::skip]
pub const PERIODS_EXT: [u16; 54] = [
    0x0358, 0x0328, 0x02fa, 0x02d0, 0x02a6, 0x0280, 0x025c, 0x023a, 0x021a, 0x01fc, 0x01e0, 0x01c5,
    0x01ac, 0x0194, 0x017d, 0x0168, 0x0153, 0x0140, 0x012e, 0x011d, 0x010d, 0x00fe, 0x00f0, 0x00e2,
    0x00d6, 0x00ca, 0x00be, 0x00b4, 0x00aa, 0x00a0, 0x0097, 0x008f, 0x0087, 0x007f, 0x0078, 0x0071,
    0x0000, 0x0000,
    // EnvDef
    1, 64, 4, 55, 5, 50, 25, 0, 0, 0,
    // EnvShoot
    1, 64, 10, 0, 0, 0,
];

/// `mt_periods` of the tracker (`+Music.s:2279`), 36 notes and a 0.
pub const MT_PERIODS: [u16; 37] = [
    0x0358, 0x0328, 0x02fa, 0x02d0, 0x02a6, 0x0280, 0x025c, 0x023a, 0x021a, 0x01fc, 0x01e0, 0x01c5,
    0x01ac, 0x0194, 0x017d, 0x0168, 0x0153, 0x0140, 0x012e, 0x011d, 0x010d, 0x00fe, 0x00f0, 0x00e2,
    0x00d6, 0x00ca, 0x00be, 0x00b4, 0x00aa, 0x00a0, 0x0097, 0x008f, 0x0087, 0x007f, 0x0078, 0x0071,
    0x0000,
];

/// Default envelope copied into every new wave (`EnvDef`).
pub const ENV_DEF: [i16; 10] = [1, 64, 4, 55, 5, 50, 25, 0, 0, 0];
pub const ENV_SHOOT: [i16; 6] = [1, 64, 10, 0, 0, 0];
pub const ENV_BOOM: [i16; 8] = [1, 64, 10, 50, 50, 0, 0, 0];
pub const ENV_BELL: [i16; 8] = [1, 64, 4, 40, 25, 0, 0, 0];

/// `TFreq`: (byte offset in the wave, length in words) of the sub-wave used
/// for each octave.
pub const TFREQ: [(u16, u16); 9] = [
    (0, 128),
    (0, 128),
    (256, 64),
    (384, 32),
    (448, 16),
    (480, 8),
    (496, 4),
    (504, 2),
    (504, 2),
];

/// `TNotes`: frequency in Hz of each note (index = note + 2).
pub const TNOTES: [u16; 100] = [
    0, 0, 0, 33, 35, 37, 39, 41, 44, 46, 49, 52, 55, 58, 62, 65, 69, 73, 78, 82, 87, 92, 98, 104,
    110, 117, 123, 131, 139, 147, 156, 165, 175, 185, 196, 208, 220, 233, 247, 262, 277, 294, 311,
    330, 349, 370, 392, 415, 440, 466, 494, 523, 554, 587, 622, 659, 698, 740, 784, 830, 880, 932,
    988, 1046, 1109, 1175, 1245, 1319, 1397, 1480, 1568, 1661, 1760, 1865, 1986, 2093, 2217, 2349,
    2489, 2637, 2794, 2960, 3136, 3322, 3520, 3729, 3952, 4186, 4435, 4699, 4978, 5274, 5588, 5920,
    6272, 6645, 7040, 7459, 7902, 8372,
];

/// Length of a wave: 256 bytes plus 6 decimated copies (`LWave`).
pub const LWAVE: usize = 256 + 128 + 64 + 32 + 16 + 8 + 4 + 2;
/// Length of the noise buffer (`LNoise`).
pub const LNOISE: usize = LWAVE;
