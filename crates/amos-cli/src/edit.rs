//! `amos-cli edit [FILE] [--keys KEYS] [--frames N] [--png OUT]`: drives
//! the editor headless with simulated keys and saves the display.
//!
//! KEYS is typed text where `{NAME}` is a special key: `{RET}`, `{ESC}`,
//! `{UP}`, `{DOWN}`, `{LEFT}`, `{RIGHT}`, `{BS}`, `{DEL}`, `{TAB}`,
//! `{HELP}`, `{F1}`..`{F10}`, a letter, with optional prefixes `C-` (Ctrl),
//! `S-` (Shift), `A-` (Amiga), `L-` (Alt); `{WAIT n}` waits n frames and
//! `{RMB x,y}` / `{RUP x,y}` press / release the right button at editor
//! screen position x,y (menus), `{LMB x,y}` clicks the left button.

use std::path::{Path, PathBuf};

use amos_core::Machine;
use amos_core::display::render_rgba;
use amos_core::editor::{EC_EDIT, Editor};
use amos_core::input::{InputEvent, MouseButton};

enum Step {
    Char(char),
    Key { shifts: Vec<u8>, raw: u8, ch: Option<char> },
    Wait(usize),
    Mouse { x: i32, y: i32, button: MouseButton, pressed: bool },
}

fn parse_keys(s: &str) -> Result<Vec<Step>, String> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '{' {
            out.push(if c == '\n' { Step::Key { shifts: vec![], raw: 0x44, ch: Some('\r') } } else { Step::Char(c) });
            continue;
        }
        let mut name = String::new();
        for c in it.by_ref() {
            if c == '}' {
                break;
            }
            name.push(c);
        }
        if let Some(n) = name.strip_prefix("WAIT ") {
            out.push(Step::Wait(n.trim().parse().map_err(|_| "bad WAIT")?));
            continue;
        }
        let mouse = |rest: &str, button, pressed| -> Result<Step, String> {
            let (x, y) = rest.trim().split_once(',').ok_or("bad mouse position")?;
            Ok(Step::Mouse {
                x: x.trim().parse().map_err(|_| "bad x")?,
                y: y.trim().parse().map_err(|_| "bad y")?,
                button,
                pressed,
            })
        };
        if let Some(r) = name.strip_prefix("RMB ") {
            out.push(mouse(r, MouseButton::Right, true)?);
            continue;
        }
        if let Some(r) = name.strip_prefix("RUP ") {
            out.push(mouse(r, MouseButton::Right, false)?);
            continue;
        }
        if let Some(r) = name.strip_prefix("LMB ") {
            out.push(mouse(r, MouseButton::Left, true)?);
            out.push(Step::Wait(1));
            out.push(mouse(r, MouseButton::Left, false)?);
            continue;
        }
        let mut shifts = Vec::new();
        let mut rest = name.as_str();
        loop {
            let (code, r) = match rest.get(..2) {
                Some("C-") => (0x63, &rest[2..]),
                Some("S-") => (0x60, &rest[2..]),
                Some("A-") => (0x66, &rest[2..]),
                Some("L-") => (0x64, &rest[2..]),
                _ => break,
            };
            shifts.push(code);
            rest = r;
        }
        let (raw, ch) = match rest {
            "RET" => (0x44, Some('\r')),
            "ESC" => (0x45, Some('\u{1b}')),
            "UP" => (0x4C, None),
            "DOWN" => (0x4D, None),
            "RIGHT" => (0x4E, None),
            "LEFT" => (0x4F, None),
            "BS" => (0x41, Some('\u{8}')),
            "DEL" => (0x46, None),
            "TAB" => (0x42, Some('\t')),
            "HELP" => (0x5F, None),
            f if f.starts_with('F') && f.len() > 1 => {
                let n: u8 = f[1..].parse().map_err(|_| format!("bad key {f}"))?;
                (0x4F + n, None)
            }
            l if l.len() == 1 => {
                let c = l.chars().next().unwrap().to_ascii_lowercase();
                let raw = "qwertyuiop"
                    .find(c)
                    .map(|i| 0x10 + i as u8)
                    .or_else(|| "asdfghjkl".find(c).map(|i| 0x20 + i as u8))
                    .or_else(|| "zxcvbnm".find(c).map(|i| 0x31 + i as u8))
                    .ok_or(format!("bad key {l}"))?;
                // Control letters give control characters on a real keyboard.
                let ch = if shifts.contains(&0x63) { char::from_u32(c as u32 & 0x1F) } else { Some(c) };
                (raw, ch)
            }
            other => return Err(format!("unknown key {{{other}}}")),
        };
        out.push(Step::Key { shifts, raw, ch });
    }
    Ok(out)
}

pub fn edit(args: &[String]) -> Result<(), String> {
    let mut file = None;
    let mut keys = String::new();
    let mut frames = 10;
    let mut png = None;
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--keys" => keys = v.replace("\\n", "\n"),
            "--frames" => frames = v.parse().map_err(|_| "bad --frames")?,
            "--png" => png = Some(PathBuf::from(v)),
            f if !f.starts_with("--") && file.is_none() => {
                file = Some(PathBuf::from(f));
                i += 1;
                continue;
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 2;
    }
    let mut m = Machine::new();
    let base = file.as_ref().and_then(|f| f.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
    let base = if base.as_os_str().is_empty() { PathBuf::from(".") } else { base };
    m.hw.files.set_native_root(&base);
    let mut ed = Editor::new(&mut m);
    if let Some(f) = &file {
        let name = f.file_name().and_then(|n| n.to_str()).ok_or("bad file name")?;
        ed.load(&mut m, &format!("Work:{name}"))?;
    }
    for step in parse_keys(&keys)? {
        match step {
            Step::Char(c) => m.input(InputEvent::Char(c)),
            Step::Key { shifts, raw, ch } => {
                for &s in &shifts {
                    m.input(InputEvent::Key { scancode: s, pressed: true, ch: None });
                }
                m.input(InputEvent::Key { scancode: raw, pressed: true, ch });
                m.input(InputEvent::Key { scancode: raw, pressed: false, ch: None });
                for &s in &shifts {
                    m.input(InputEvent::Key { scancode: s, pressed: false, ch: None });
                }
            }
            Step::Wait(n) => {
                for _ in 0..n {
                    ed.vbl(&mut m);
                }
            }
            Step::Mouse { x, y, button, pressed } => {
                if let Some(s) = m.hw.screens.get(EC_EDIT) {
                    let hx = s.display_x + x / 2;
                    let hy = s.display_y + y;
                    let dx = ((hx - amos_core::display::HW_X0) * 2) as f32 + 0.5;
                    let dy = ((hy - amos_core::display::HW_Y0) * 2) as f32 + 0.5;
                    m.input(InputEvent::MouseMove { x: dx, y: dy });
                }
                ed.vbl(&mut m);
                m.input(InputEvent::MouseButton { button, pressed });
            }
        }
        ed.vbl(&mut m);
    }
    for _ in 0..frames {
        ed.vbl(&mut m);
    }
    println!("-- mode {:?}, line {}, column {}", ed.mode, ed.doc().y + 1, ed.doc().x + 1);
    if let Some(a) = ed.current_alert() {
        println!("-- alert: {a}");
    }
    if let Some(p) = png {
        let rgba = render_rgba(&m.frame());
        super::save_png(&p, &rgba)?;
        println!("-- saved {}", p.display());
    }
    Ok(())
}
