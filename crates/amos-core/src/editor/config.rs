//! The editor configuration file `AMOSPro_Editor_Config` (built from
//! `+Editor_Config.s`).
//!
//! Layout: a long giving the size of the configuration block, the block
//! (screen size, palette, flags, the 184 autoload entries and the key
//! table `Ed_KFonc`), then eight sections each preceded by their length:
//! system strings, menu strings, editor messages, test-time messages,
//! run-time messages, autoload programs, user menu and menu definitions.
//! String sections are lists of `(0, length, text)` records ended by
//! `(0, $FF)`.

use crate::input::KeyPress;

/// The configuration shipped with AMOS Professional, used when the file
/// cannot be read from the `AMOSPro_System:` volume.
pub static DEFAULT_CONFIG: &[u8] =
    include_bytes!("../../../../AMOS-Professional-365/AMOS/APSystem/AMOSPro_Editor_Config");

/// Shift groups of the key table (`+Equ.s:775`).
pub const SHF: u8 = 0b0000_0011;
pub const CTR: u8 = 0b0000_1000;
pub const ALT: u8 = 0b0011_0000;
pub const AMI: u8 = 0b1100_0000;

#[derive(Clone, Debug)]
pub struct EdConfig {
    pub sx: u16,
    pub sy: u16,
    /// Hardware position of the editor screen.
    pub wx: u16,
    pub wy: u16,
    /// Speed of the editor appearing (`Ed_VScrol`).
    pub vscroll: u16,
    pub interlace: bool,
    pub colour_back: u16,
    /// Keyword case when listing (`DtkMaj1/2`).
    pub dtk_maj1: u8,
    pub dtk_maj2: u8,
    /// Direct mode history size (`Esc_KMemMax`).
    pub esc_kmem_max: u16,
    pub palette: [u16; 8],
    /// Direct mode window position (`Es_Y1`, `Es_Y2`).
    pub esc_y1: u16,
    pub esc_y2: u16,
    pub search_mode: u16,
    pub tabs: u16,
    /// Quit options (`Ed_QuitFlags`): bit 0 confirm quit.
    pub quit_flags: u8,
    pub insert: bool,
    pub sounds: bool,
    /// Key table: for each function (index = number - 1), the list of
    /// (key, shifts) combinations. Key `$80|n` is raw key n, otherwise an
    /// upper case ASCII letter; key 1 means none.
    pub keys: Vec<Vec<(u8, u8)>>,
    pub sys: Vec<Vec<u8>>,
    pub menus: Vec<Vec<u8>>,
    pub messages: Vec<Vec<u8>>,
    pub test_messages: Vec<Vec<u8>>,
    /// Run-time messages, starting at error 0.
    pub run_messages: Vec<Vec<u8>>,
    /// Programs called by editor functions (`Ed_AutoLoad`): for each
    /// function (index = number - 1), flags (bit 0: run as an accessory,
    /// keeping the current program), the program (message number in
    /// `autoload`, 0 = none) and its command line (message number, 0 = the
    /// current line from the cursor).
    pub autoload_table: Vec<[u8; 3]>,
    pub autoload: Vec<Vec<u8>>,
    pub user_menu: Vec<Vec<u8>>,
    /// Menu tree definitions: 8 byte records (see `menu.rs`).
    pub menu_defs: Vec<[u8; 8]>,
    /// Syntax highlighting of the program text (an addition of this port,
    /// not stored in the file; false gives the original look).
    pub highlight: bool,
}

fn rd16(d: &[u8], p: usize) -> u16 {
    d.get(p..p + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
}

fn rd32(d: &[u8], p: usize) -> usize {
    d.get(p..p + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

/// Reads a list of `(0, len, text)` records.
fn strings(d: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p + 1 < d.len() && d[p] == 0 && d[p + 1] != 0xFF {
        let n = d[p + 1] as usize;
        out.push(d.get(p + 2..p + 2 + n).unwrap_or(&[]).to_vec());
        p += 2 + n;
    }
    out
}

impl EdConfig {
    /// Parses a configuration file.
    pub fn parse(d: &[u8]) -> Option<EdConfig> {
        let len = rd32(d, 0);
        let c = d.get(4..4 + len)?;
        if len < 650 {
            return None;
        }
        let mut palette = [0u16; 8];
        for (i, p) in palette.iter_mut().enumerate() {
            *p = rd16(c, 28 + i * 2);
        }
        // Key table at offset 92 + 184 * 3 (after the autoload entries).
        let mut keys = Vec::new();
        let mut p = 92 + 184 * 3;
        let mut cur = Vec::new();
        while p < c.len() {
            let k = c[p];
            if k == 0xFF {
                break;
            }
            if k == 0 {
                keys.push(std::mem::take(&mut cur));
                p += 1;
                continue;
            }
            cur.push((k, c.get(p + 1).copied().unwrap_or(0)));
            p += 2;
        }
        let autoload_table = c.get(92..92 + 184 * 3).map_or_else(Vec::new, |t| t.as_chunks::<3>().0.to_vec());
        let mut sections = Vec::new();
        let mut p = 4 + len;
        for _ in 0..8 {
            let n = rd32(d, p);
            let s = d.get(p + 4..p + 4 + n).unwrap_or(&[]);
            sections.push(s);
            p += 4 + n;
        }
        let menu_defs = sections[7].as_chunks::<8>().0.to_vec();
        Some(EdConfig {
            sx: rd16(c, 0).clamp(320, 1008),
            sy: rd16(c, 2).clamp(64, 1008),
            wx: rd16(c, 4),
            wy: rd16(c, 6),
            vscroll: rd16(c, 8),
            interlace: c[10] != 0,
            colour_back: rd16(c, 12),
            dtk_maj1: c[22],
            dtk_maj2: c[23],
            esc_kmem_max: rd16(c, 26).max(1),
            palette,
            esc_y1: rd16(c, 44),
            esc_y2: rd16(c, 46),
            search_mode: rd16(c, 84),
            tabs: rd16(c, 86).clamp(1, 16),
            quit_flags: c[89],
            insert: c[90] != 0,
            sounds: c[91] != 0,
            keys,
            sys: strings(sections[0]),
            menus: strings(sections[1]),
            messages: strings(sections[2]),
            test_messages: strings(sections[3]),
            run_messages: strings(sections[4]),
            autoload_table,
            autoload: strings(sections[5]),
            user_menu: strings(sections[6]),
            menu_defs,
            highlight: true,
        })
    }

    /// The configuration of the original distribution.
    pub fn defaults() -> EdConfig {
        EdConfig::parse(DEFAULT_CONFIG).expect("built-in editor configuration")
    }

    /// System string `n` (1-based, `Ed_GetSysteme`).
    pub fn sys(&self, n: usize) -> &[u8] {
        n.checked_sub(1).and_then(|i| self.sys.get(i)).map_or(&[], |v| v.as_slice())
    }

    /// Editor message `n` (1-based, `Ed_GetMessage`).
    pub fn message(&self, n: usize) -> String {
        let m = n.checked_sub(1).and_then(|i| self.messages.get(i)).map_or(&[][..], |v| v.as_slice());
        crate::detok::latin1_to_string(m)
    }

    /// Test-time error message `n`.
    pub fn test_message(&self, n: u16) -> String {
        match (n as usize).checked_sub(1).and_then(|i| self.test_messages.get(i)) {
            Some(m) => crate::detok::latin1_to_string(m),
            None => crate::errors::test_message(n).to_string(),
        }
    }

    /// Run-time error message `n`.
    pub fn run_message(&self, n: u16) -> String {
        match self.run_messages.get(n as usize) {
            Some(m) if !m.is_empty() => crate::detok::latin1_to_string(m),
            _ => crate::errors::message(n).to_string(),
        }
    }

    /// `Ed_AutoLoad`: the program run by function `f` and its command line
    /// (None: the current line), if `f` runs an accessory.
    pub fn accessory(&self, f: u16) -> Option<(String, Option<Vec<u8>>)> {
        let &[flags, prg, cmd] = self.autoload_table.get((f as usize).checked_sub(1)?)?;
        if flags & 1 == 0 || prg == 0 {
            return None;
        }
        let name = crate::detok::latin1_to_string(self.autoload.get(prg as usize - 1)?);
        let cmd = (cmd != 0).then(|| self.autoload.get(cmd as usize - 1).cloned().unwrap_or_default());
        Some((name, cmd))
    }

    /// `Ed_Ky2Fonc` (+Edit.s:1690): the editor function assigned to a key,
    /// or None. Each shift group pressed must be required by the entry and
    /// every group the entry requires must be pressed; Caps Lock is ignored.
    pub fn function_for_key(&self, k: &KeyPress) -> Option<u16> {
        let letter = k.ascii.to_ascii_uppercase();
        let pressed = k.shift & !0x04;
        for (i, combos) in self.keys.iter().enumerate() {
            for &(key, shifts) in combos {
                let hit = if key & 0x80 != 0 { k.raw != 0 && key & 0x7F == k.raw } else { key > 1 && key == letter };
                if hit && shift_groups(pressed) == shift_groups(shifts) {
                    return Some(i as u16 + 1);
                }
            }
        }
        None
    }

    /// The first key assigned to a function, as text ("Ctrl+B", "F1").
    pub fn key_name(&self, function: u16) -> Option<String> {
        let &(key, shifts) = self.keys.get(function as usize - 1)?.first()?;
        if key <= 1 {
            return None;
        }
        let mut s = String::new();
        if shifts & AMI != 0 {
            s.push_str("A+");
        }
        if shifts & CTR != 0 {
            s.push_str("C+");
        }
        if shifts & ALT != 0 {
            s.push_str("Al+");
        }
        if shifts & SHF != 0 {
            s.push_str("S+");
        }
        if key & 0x80 != 0 {
            let raw = key & 0x7F;
            let name = match raw {
                0x50..=0x59 => format!("F{}", raw - 0x4F),
                0x41 => "Bs".into(),
                0x46 => "Del".into(),
                0x45 => "Esc".into(),
                0x5F => "Help".into(),
                0x42 => "Tab".into(),
                0x4C => "Up".into(),
                0x4D => "Down".into(),
                0x4E => "Right".into(),
                0x4F => "Left".into(),
                _ => return None,
            };
            s.push_str(&name);
        } else {
            s.push(key as char);
        }
        Some(s)
    }
}

/// Which of the four shift groups (shift, control, alt, amiga) are set.
fn shift_groups(s: u8) -> [bool; 4] {
    [s & SHF != 0, s & CTR != 0, s & ALT != 0, s & AMI != 0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_distribution_config() {
        let c = EdConfig::defaults();
        assert_eq!((c.sx, c.sy, c.wx, c.wy), (640, 256, 129, 50));
        assert_eq!(c.palette[3], 0xEEE);
        assert_eq!(c.tabs, 3);
        assert!(c.insert);
        assert!(c.keys.len() >= 184);
        assert_eq!(c.sys(2), b"Window-     L-      C-    Free-         Edit- ");
        assert_eq!(c.message(205), "Not found.");
        assert_eq!(c.test_message(35), "Syntax error");
        assert_eq!(c.run_message(10), "End of program");
        assert!(c.menu_defs.len() > 190);
        assert_eq!(c.menus[0], b" Project ");
    }

    #[test]
    fn key_functions() {
        let c = EdConfig::defaults();
        let k = |raw: u8, ascii: u8, shift: u8| KeyPress { shift, raw, ascii };
        assert_eq!(c.function_for_key(&k(0x50, 0, 0)), Some(77)); // F1 Run
        assert_eq!(c.function_for_key(&k(0x51, 0, 0)), Some(78)); // F2 Test
        assert_eq!(c.function_for_key(&k(0x45, 27, 0)), Some(28)); // Esc
        assert_eq!(c.function_for_key(&k(0x4C, 0x1E, 0)), Some(1));
        assert_eq!(c.function_for_key(&k(0x4C, 0x1E, 1)), Some(5)); // Shift+Up
        assert_eq!(c.function_for_key(&k(0x4C, 0x1E, 9)), Some(17)); // Ctrl+Shift+Up
        assert_eq!(c.function_for_key(&k(0x35, b'b', 8)), Some(59)); // Ctrl+B
        assert_eq!(c.function_for_key(&k(0x21, b's', 0x40)), Some(35)); // Amiga+S
        assert_eq!(c.function_for_key(&k(0x21, b'S', 0x41)), Some(34)); // Amiga+Shift+S
        assert_eq!(c.function_for_key(&k(0x44, 13, 0)), Some(19)); // Return
        assert_eq!(c.function_for_key(&k(0x20, b'a', 0)), None);
        assert_eq!(c.key_name(77).as_deref(), Some("F1"));
    }

    #[test]
    fn help_accessory() {
        let c = EdConfig::defaults();
        let help = "AMOSPro_Accessories:AMOSPro_Help/AMOSPro_Help.AMOS".to_string();
        assert_eq!(c.accessory(27), Some((help.clone(), None)));
        assert_eq!(c.accessory(152), Some((help.clone(), Some(b"HelpMenu".to_vec()))));
        assert_eq!(c.accessory(167), Some((help, Some(b"HelpInfo".to_vec()))));
        assert_eq!(c.accessory(77), None);
    }
}
