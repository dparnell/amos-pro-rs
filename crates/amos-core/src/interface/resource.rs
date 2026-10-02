//! Resource banks ("Resource", normally bank 16): packed images, messages
//! and Interface programs (`Dia_GetPuzzle` `+Lib.s:14914`,
//! `Dia_GetDefault` `+Lib.s:14953`).
//!
//! Bank data (after the 8 character name):
//!
//! ```text
//! 0   w  number of valid sections (1 = graphics, 2 = +messages, 3 = +programs)
//! 2   l  offset of the graphics section
//! 6   l  offset of the messages section
//! 10  l  offset of the programs section
//! ```
//!
//! A section with a zero offset (or beyond the count) is taken from the
//! default resource (`AMOSPro_Default_Resource.Abk`, loaded at start-up as
//! `Sys_Resource`), except the programs: a user bank without programs has
//! none.

use std::rc::Rc;

/// The default resource (`Sys_Resource`, `+B.s:2085`).
static DEFAULT_RESOURCE: &[u8] =
    include_bytes!("../../../../AMOS-Professional-365/AMOS/APSystem/AMOSPro_Default_Resource.Abk");

/// System messages of the interpreter configuration (`Sys_Messages`,
/// `+Interpreter_Config.s:107-160`), read by `=Resource$(-1..-1000)`.
pub const SYSTEM_MESSAGES: &[&str] = &[
    "APSystem/",
    "",
    "",
    "Def_Icon",
    "AutoExec.AMOS",
    "AMOSPro_Editor",
    "AMOSPro_Editor_Config",
    "AMOSPro_Default_Resource.Abk",
    "AMOSPro_Productivity1:Equates/AMOSPro_System_Equates",
    "AMOSPro_Monitor",
    "AMOSPro_Monitor_Resource.Abk",
    "AMOSPro_Accessories:AMOSPro_Help/AMOSPro_Help",
    "AMOSPro_Accessories:AMOSPro_Help/LatestNews",
    "AMOSPro.Lib",
    "",
    "AMOSPro_Music.Lib",
    "AMOSPro_Compact.Lib",
    "AMOSPro_Request.Lib",
    "AMOSPro_3d.Lib",
    "AMOSPro_Compiler.Lib",
    "AMOSPro_IOPorts.Lib",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "Par:",
    "Aux:",
    "",
    "(000,2)(440,2)(880,2)(bb0,2)(dd0,2)(ee0,2)(ff2,2)(ff8,2)(ffc,2)(fff,2)(aaf,2)(88c,2)(66a,2)(226,2)(004,2)(001,2)",
    "",
];

fn rd16(d: &[u8], o: usize) -> u16 {
    match (d.get(o), d.get(o + 1)) {
        (Some(&a), Some(&b)) => u16::from_be_bytes([a, b]),
        _ => 0,
    }
}

fn rd32(d: &[u8], o: usize) -> u32 {
    (rd16(d, o) as u32) << 16 | rd16(d, o + 2) as u32
}

/// Graphics section: packed images plus the screen definition used by
/// `Resource Screen Open`.
#[derive(Debug)]
pub struct Graphics {
    /// Section bytes (offsets are relative to its start).
    pub data: Vec<u8>,
    /// Offset of each packed bitmap (images are numbered from 1).
    pub offsets: Vec<usize>,
    /// Number of colours (4096 = HAM).
    pub colours: u16,
    /// Screen mode ($8000 hires, $4 laced).
    pub mode: u16,
    pub palette: [u16; 32],
}

impl Graphics {
    fn parse(d: &[u8]) -> Graphics {
        let n = rd16(d, 0) as usize;
        let offsets = (0..n).map(|i| rd32(d, 2 + i * 4) as usize).collect();
        let p = 2 + n * 4;
        let mut palette = [0u16; 32];
        for (i, c) in palette.iter_mut().enumerate() {
            *c = rd16(d, p + 4 + i * 2);
        }
        Graphics {
            data: d.to_vec(),
            offsets,
            colours: rd16(d, p),
            mode: rd16(d, p + 2),
            palette,
        }
    }

    pub fn count(&self) -> usize {
        self.offsets.len()
    }

    /// Packed bitmap `n` (1-based).
    pub fn image(&self, n: usize) -> Option<&[u8]> {
        let o = *self.offsets.get(n.checked_sub(1)?)?;
        self.data.get(o..)
    }
}

/// Messages section: a 0 byte, then `[len][chars][0]` per message, ending
/// with a length of $FF.
#[derive(Debug)]
pub struct Messages {
    pub data: Vec<u8>,
}

impl Messages {
    /// `GetMessage` (`+B.s:553`): message `n` (1-based), empty when out of
    /// range.
    pub fn get(&self, n: i32) -> Vec<u8> {
        let d = &self.data;
        let mut a = 1usize;
        let mut n = n;
        loop {
            n -= 1;
            if n <= 0 {
                break;
            }
            let l = d.get(a).copied().unwrap_or(0xFF);
            if l == 0xFF {
                return Vec::new();
            }
            a += l as usize + 2;
        }
        let l = d.get(a).copied().unwrap_or(0) as usize;
        d.get(a + 1..a + 1 + l)
            .map_or_else(Vec::new, |s| s.to_vec())
    }

    /// `ME` of the Interface (`Dia_FMess` `+Lib.s:23098`): None ("illegal
    /// function call") for n <= 0 or when a length >= 128 is met while
    /// skipping.
    pub fn interface(&self, n: i32) -> Option<Vec<u8>> {
        if n <= 0 {
            return None;
        }
        let d = &self.data;
        let mut a = 0usize;
        for _ in 1..n {
            let l = d.get(a + 1).copied().unwrap_or(0xFF);
            if l >= 0x80 {
                return None;
            }
            a += l as usize + 2;
        }
        let l = d.get(a + 1).copied().unwrap_or(0) as usize;
        Some(
            d.get(a + 2..a + 2 + l)
                .map_or_else(Vec::new, |s| s.to_vec()),
        )
    }
}

/// The parts of a resource bank used by the dialogs.
#[derive(Debug, Clone)]
pub struct Resource {
    pub graphics: Rc<Graphics>,
    pub messages: Rc<Messages>,
    /// Interface programs (source text); empty when the bank has none.
    pub programs: Vec<Rc<[u8]>>,
}

impl Resource {
    /// The default resource (`Dia_GetDefault`): all its sections, but no
    /// programs for `Dialog Open n` (d0 = 0).
    pub fn default_resource() -> Resource {
        thread_local! {
            static DEF: Resource = Resource::parse_default();
        }
        DEF.with(|r| r.clone())
    }

    fn parse_default() -> Resource {
        let banks = crate::banks::parse_banks(DEFAULT_RESOURCE).expect("default resource");
        let d = banks[0].raw().expect("raw bank").to_vec();
        let sect = |i: usize| rd32(&d, 2 + i * 4) as usize;
        Resource {
            graphics: Rc::new(Graphics::parse(&d[sect(0)..])),
            messages: Rc::new(Messages {
                data: d[sect(1)..].to_vec(),
            }),
            programs: parse_programs(&d[sect(2)..]),
        }
    }

    /// Resource from the data of a "Resource" bank; missing sections come
    /// from `def` (`Dia_GetPuzzle`).
    pub fn from_bank(d: &[u8], def: &Resource) -> Resource {
        let n = rd16(d, 0);
        let sect = |i: u16| {
            if n > i {
                let o = rd32(d, 2 + i as usize * 4) as usize;
                (o != 0 && o < d.len()).then_some(o)
            } else {
                None
            }
        };
        Resource {
            graphics: sect(0).map_or_else(
                || def.graphics.clone(),
                |o| Rc::new(Graphics::parse(&d[o..])),
            ),
            messages: sect(1).map_or_else(
                || def.messages.clone(),
                |o| {
                    Rc::new(Messages {
                        data: d[o..].to_vec(),
                    })
                },
            ),
            programs: sect(2).map_or_else(Vec::new, |o| parse_programs(&d[o..])),
        }
    }
}

/// Programs section: word count, long offsets, then each program as a word
/// length followed by the ASCII source.
fn parse_programs(d: &[u8]) -> Vec<Rc<[u8]>> {
    let n = rd16(d, 0) as usize;
    (0..n)
        .map(|i| {
            let o = rd32(d, 2 + i * 4) as usize;
            let len = rd16(d, o) as usize;
            let s = d.get(o + 2..(o + 2 + len).min(d.len())).unwrap_or(&[]);
            Rc::from(s)
        })
        .collect()
}

/// `=Resource$(n)` for n <= 0 (`FnResource` `+ILib.s:6670`): 0 is the
/// system path, -1..-1000 the system messages, then editor system
/// messages, editor menus (not available), editor messages, test and
/// run-time error messages. None: illegal function call.
pub fn system_resource(n: i32) -> Option<Vec<u8>> {
    let latin = |s: &str| s.chars().map(|c| c as u32 as u8).collect::<Vec<u8>>();
    if n == 0 {
        return Some(b"AMOSPro_System:APSystem/".to_vec());
    }
    let d = -(n as i64);
    let (group, k) = ((d - 1) / 1000, ((d - 1) % 1000 + 1) as u16);
    let s = match group {
        0 => SYSTEM_MESSAGES.get(k as usize - 1).copied().unwrap_or(""),
        1 => lookup(crate::errors::messages::SYSTEM, k),
        2 => "",
        3 => crate::errors::editor_message(k),
        4 => lookup(crate::errors::messages::TEST, k),
        5 => lookup(crate::errors::messages::RUNTIME, k),
        _ => return None,
    };
    Some(latin(s))
}

fn lookup(list: &'static [(u16, &'static str)], n: u16) -> &'static str {
    list.iter().find(|(k, _)| *k == n).map_or("", |(_, t)| *t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resource_sections() {
        let r = Resource::default_resource();
        assert_eq!(r.graphics.count(), 93);
        assert_eq!(r.graphics.colours, 8);
        assert_eq!(r.programs.len(), 4);
        assert!(r.programs[0].starts_with(b"SIze"));
        assert_eq!(r.messages.get(1), b"File Selector");
        assert_eq!(r.messages.get(20), b"AMOS Professional Text Reader");
        assert_eq!(r.messages.get(500), b"");
        assert_eq!(r.messages.interface(3).unwrap(), b"OK");
        assert!(r.messages.interface(0).is_none());
    }

    #[test]
    fn system_messages() {
        assert_eq!(
            system_resource(-12).unwrap(),
            b"AMOSPro_Accessories:AMOSPro_Help/AMOSPro_Help"
        );
        assert_eq!(system_resource(-5001 - 19).unwrap(), b"Division by zero");
        assert!(system_resource(-7000).is_none());
    }
}
