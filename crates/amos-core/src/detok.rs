//! Detokeniser: converts tokenised lines back to text exactly like the
//! AMOS Pro editor (`Detok`, `+Edit.s`), including its spacing rules.

use crate::program::{Program, read_u16, read_u32, read_var};
use crate::tokens::*;

/// Upper-cases the first character and every character following a space
/// (from the third character on, so `" xor "` stays lower case).
fn capitalise(name: &str, out: &mut Vec<u8>) {
    let b = name.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        let up = i == 0 || (i >= 2 && b[i - 1] == b' ');
        out.push(if up { c.to_ascii_uppercase() } else { c });
    }
}

/// Lists one tokenised line (starting with its length byte). The result is
/// Latin-1 encoded bytes, as in the original.
pub fn detok_line(line: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(80);
    if line.len() < 2 {
        return out;
    }
    let indent = line[1] as usize;
    out.extend(std::iter::repeat_n(b' ', indent.saturating_sub(1)));
    let start_len = out.len();
    let mut p = 2;
    let mut after_var = false;
    let last = |out: &Vec<u8>| out.last().copied();

    while p + 2 <= line.len() {
        let tk = read_u16(line, p);
        p += 2;
        if tk == TK_EOL {
            break;
        }
        if tk <= TK_LGO {
            let Some(v) = read_var(line, p) else { break };
            if after_var && last(&out) != Some(b' ') {
                out.push(b' ');
            }
            out.extend(v.name.iter().map(|c| c.to_ascii_uppercase()));
            p += v.size;
            after_var = true;
            if tk == TK_LAB {
                if !v.name.first().is_some_and(|c| c.is_ascii_digit()) {
                    out.push(b':');
                }
            } else {
                match v.flags & 3 {
                    1 => out.push(b'#'),
                    2 => out.push(b'$'),
                    _ => {}
                }
            }
            continue;
        }
        if tk < TK_EXT || tk == TK_DFL {
            if after_var && last(&out) != Some(b' ') {
                out.push(b' ');
            }
            after_var = false;
            match tk {
                TK_CH1 | TK_CH2 => {
                    let q = if tk == TK_CH1 { b'"' } else { b'\'' };
                    let n = read_u16(line, p) as usize;
                    let s = line.get(p + 2..p + 2 + n).unwrap_or(&[]);
                    out.push(q);
                    out.extend_from_slice(s);
                    out.push(q);
                    p += 2 + n + (n & 1);
                }
                TK_ENT => {
                    out.extend((read_u32(line, p) as i32).to_string().bytes());
                    p += 4;
                }
                TK_HEX => {
                    out.extend(format!("${:X}", read_u32(line, p)).bytes());
                    p += 4;
                }
                TK_BIN => {
                    out.extend(format!("%{:b}", read_u32(line, p)).bytes());
                    p += 4;
                }
                TK_FL => {
                    out.extend(crate::ffp::format_ffp_listing(crate::ffp::Ffp(read_u32(line, p))).bytes());
                    p += 4;
                }
                TK_DFL => {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&line[p..p + 8]);
                    out.extend(crate::ffp::format_double_listing(f64::from_be_bytes(b)).bytes());
                    p += 8;
                }
                _ => {
                    out.extend(format!("{{?CST {tk:04X}}}").bytes());
                    p += 4;
                }
            }
            continue;
        }
        after_var = false;
        if tk == TK_PAR1 {
            if last(&out) == Some(b' ') {
                out.pop();
            }
            out.push(b'(');
            continue;
        }
        let ext_name;
        let (name, params) = if tk == TK_EXT {
            let slot = line[p] as usize;
            let off = read_u16(line, p + 2);
            match lookup_ext(slot, off) {
                Some(d) => (d.name, d.params),
                None => {
                    ext_name = format!("Extension {}:{:04X} ", (b'A' + slot as u8) as char, off);
                    (ext_name.as_str(), "I")
                }
            }
        } else {
            match lookup(tk) {
                Some(d) => (d.name, d.params),
                None => {
                    out.extend(format!("{{?TK {tk:04X}}}").bytes());
                    continue;
                }
            }
        };
        let d3 = params.as_bytes().first().copied();
        let func_like = matches!(d3, Some(b'O' | b'V' | b'0'..=b'8'));
        if !func_like && out.len() > start_len && last(&out) != Some(b' ') {
            out.push(b' ');
        }
        capitalise(name, &mut out);
        if tk == TK_REM1 || tk == TK_REM2 {
            let n = read_u16(line, p) as usize;
            let txt = line.get(p + 2..p + 2 + n).unwrap_or(&[]);
            let end = txt.iter().position(|&b| b == 0).unwrap_or(txt.len());
            out.extend_from_slice(&txt[..end]);
            p += 2 + n + (n & 1);
            continue;
        }
        if d3 == Some(b'I') {
            out.push(b' ');
        }
        p += inline_data_size(tk);
    }
    out
}

/// Lists a whole program as Latin-1 text with `\n` line endings.
pub fn list_program(prg: &Program) -> Vec<u8> {
    let mut out = Vec::new();
    for (_, line) in prg.lines() {
        out.extend(detok_line(line));
        out.push(b'\n');
    }
    out
}

/// Converts Latin-1 bytes to a Rust string.
pub fn latin1_to_string(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn src_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../AMOS-Professional-365")
    }

    /// The reference `.Asc` listings shipped with the sources must be
    /// reproduced byte for byte.
    #[test]
    fn matches_reference_listings() {
        let dir = src_dir().join("c");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("AMOS") {
                continue;
            }
            let asc = path.with_extension("Asc");
            let Ok(expected) = std::fs::read(&asc) else { continue };
            let prg = Program::load(&std::fs::read(&path).unwrap()).unwrap();
            let listing = list_program(&prg);
            let norm = |b: &[u8]| -> Vec<Vec<u8>> {
                b.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l).to_vec()).collect::<Vec<_>>()
            };
            let (got, want) = (norm(&listing), norm(&expected));
            for (i, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                assert_eq!(latin1_to_string(g), latin1_to_string(w), "{} line {}", path.display(), i + 1);
            }
            checked += 1;
        }
        assert!(checked >= 6, "only {checked} reference listings found");
    }
}
