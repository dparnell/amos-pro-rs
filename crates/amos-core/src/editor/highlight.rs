//! Syntax highlighting (not in the original editor, which shows every
//! line in one pen; it can be switched off with `EdConfig::highlight`).
//!
//! Stored lines are classified while they are listed: [`detok_classes`]
//! follows `crate::detok::detok_line` exactly and returns, with the same
//! text, the class of every character. The line being edited is still
//! plain text and goes through a small lexer ([`lex_classes`]) using the
//! keyword tables of the tokeniser.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::program::{read_u16, read_u32, read_var};
use crate::tokens::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Variables, labels, operators, punctuation.
    Normal,
    /// Instructions and functions (including extension keywords).
    Keyword,
    /// `"..."` and `'...'` strings.
    String,
    /// `Rem` and `'` comments.
    Comment,
    /// Decimal, `$hex`, `%binary` and floating point constants.
    Number,
}

struct Out {
    text: Vec<u8>,
    class: Vec<Class>,
}

impl Out {
    fn push(&mut self, c: u8, k: Class) {
        self.text.push(c);
        self.class.push(k);
    }
    fn extend(&mut self, s: impl IntoIterator<Item = u8>, k: Class) {
        for c in s {
            self.push(c, k);
        }
    }
    fn pop(&mut self) {
        self.text.pop();
        self.class.pop();
    }
    fn last(&self) -> Option<u8> {
        self.text.last().copied()
    }
}

/// Upper-cases like `detok::capitalise`.
fn capitalise(name: &str, out: &mut Out, k: Class) {
    let b = name.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        let up = i == 0 || (i >= 2 && b[i - 1] == b' ');
        out.push(if up { c.to_ascii_uppercase() } else { c }, k);
    }
}

/// Lists a tokenised line like [`crate::detok::detok_line`], with the
/// class of each character.
pub fn detok_classes(line: &[u8]) -> (Vec<u8>, Vec<Class>) {
    let mut out = Out { text: Vec::with_capacity(80), class: Vec::with_capacity(80) };
    if line.len() < 2 {
        return (out.text, out.class);
    }
    let indent = line[1] as usize;
    out.extend(std::iter::repeat_n(b' ', indent.saturating_sub(1)), Class::Normal);
    let start_len = out.text.len();
    let mut p = 2;
    let mut after_var = false;

    while p + 2 <= line.len() {
        let tk = read_u16(line, p);
        p += 2;
        if tk == TK_EOL {
            break;
        }
        if tk <= TK_LGO {
            let Some(v) = read_var(line, p) else { break };
            if after_var && out.last() != Some(b' ') {
                out.push(b' ', Class::Normal);
            }
            out.extend(v.name.iter().map(|c| c.to_ascii_uppercase()), Class::Normal);
            p += v.size;
            after_var = true;
            if tk == TK_LAB {
                if !v.name.first().is_some_and(|c| c.is_ascii_digit()) {
                    out.push(b':', Class::Normal);
                }
            } else {
                match v.flags & 3 {
                    1 => out.push(b'#', Class::Normal),
                    2 => out.push(b'$', Class::Normal),
                    _ => {}
                }
            }
            continue;
        }
        if tk < TK_EXT || tk == TK_DFL {
            if after_var && out.last() != Some(b' ') {
                out.push(b' ', Class::Normal);
            }
            after_var = false;
            let n = Class::Number;
            match tk {
                TK_CH1 | TK_CH2 => {
                    let q = if tk == TK_CH1 { b'"' } else { b'\'' };
                    let len = read_u16(line, p) as usize;
                    let s = line.get(p + 2..p + 2 + len).unwrap_or(&[]);
                    out.push(q, Class::String);
                    out.extend(s.iter().copied(), Class::String);
                    out.push(q, Class::String);
                    p += 2 + len + (len & 1);
                }
                TK_ENT => {
                    out.extend((read_u32(line, p) as i32).to_string().bytes(), n);
                    p += 4;
                }
                TK_HEX => {
                    out.extend(format!("${:X}", read_u32(line, p)).bytes(), n);
                    p += 4;
                }
                TK_BIN => {
                    out.extend(format!("%{:b}", read_u32(line, p)).bytes(), n);
                    p += 4;
                }
                TK_FL => {
                    out.extend(crate::ffp::format_ffp_listing(crate::ffp::Ffp(read_u32(line, p))).bytes(), n);
                    p += 4;
                }
                TK_DFL => {
                    let mut b = [0u8; 8];
                    b.copy_from_slice(&line[p..p + 8]);
                    out.extend(crate::ffp::format_double_listing(f64::from_be_bytes(b)).bytes(), n);
                    p += 8;
                }
                _ => {
                    out.extend(format!("{{?CST {tk:04X}}}").bytes(), Class::Normal);
                    p += 4;
                }
            }
            continue;
        }
        after_var = false;
        if tk == TK_PAR1 {
            if out.last() == Some(b' ') {
                out.pop();
            }
            out.push(b'(', Class::Normal);
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
                    out.extend(format!("{{?TK {tk:04X}}}").bytes(), Class::Normal);
                    continue;
                }
            }
        };
        let d3 = params.as_bytes().first().copied();
        let func_like = matches!(d3, Some(b'O' | b'V' | b'0'..=b'8'));
        if !func_like && out.text.len() > start_len && out.last() != Some(b' ') {
            out.push(b' ', Class::Normal);
        }
        // Operators (`+`, `=`, `Xor`...) stay in the normal pen.
        let is_word = name.trim_start().as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic());
        let operator = tk >= 0xFF00 || !is_word;
        let rem = tk == TK_REM1 || tk == TK_REM2;
        let k = if rem {
            Class::Comment
        } else if operator {
            Class::Normal
        } else {
            Class::Keyword
        };
        capitalise(name, &mut out, k);
        if rem {
            let len = read_u16(line, p) as usize;
            let txt = line.get(p + 2..p + 2 + len).unwrap_or(&[]);
            let end = txt.iter().position(|&b| b == 0).unwrap_or(txt.len());
            out.extend(txt[..end].iter().copied(), Class::Comment);
            p += 2 + len + (len & 1);
            continue;
        }
        if d3 == Some(b'I') {
            out.push(b' ', Class::Normal);
        }
        p += inline_data_size(tk);
    }
    (out.text, out.class)
}

/// Keyword names (lower case, as in the token tables) by first letter.
fn keywords() -> &'static HashMap<u8, Vec<&'static [u8]>> {
    static K: OnceLock<HashMap<u8, Vec<&'static [u8]>>> = OnceLock::new();
    K.get_or_init(|| {
        let mut m: HashMap<u8, Vec<&'static [u8]>> = HashMap::new();
        for name in &keyword_index().names_longest_first {
            let b = name.trim_start().as_bytes();
            if let Some(&c) = b.first()
                && c.is_ascii_alphabetic()
            {
                m.entry(c).or_default().push(b);
            }
        }
        m
    })
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0xC0
}

/// Length of the keyword at the start of `s` (spaces of the keyword are
/// optional in the text, as for the tokeniser), if any.
fn match_keyword(s: &[u8]) -> Option<usize> {
    let c0 = s.first()?.to_ascii_lowercase();
    let mut best = None;
    for name in keywords().get(&c0)? {
        let name = name.strip_suffix(b" ").unwrap_or(name);
        let (mut i, mut k) = (0, 0);
        let ok = loop {
            if k == name.len() {
                break true;
            }
            let kc = name[k];
            let ic = s.get(i).map(|c| c.to_ascii_lowercase());
            if kc == b' ' {
                if ic == Some(b' ') {
                    i += 1;
                }
                k += 1;
                continue;
            }
            if ic == Some(kc) {
                i += 1;
                k += 1;
            } else {
                break false;
            }
        };
        // A whole word: the next character does not continue an
        // identifier (unless the keyword ends with a symbol: `Left$`).
        let ends_word = !name.last().is_some_and(|&c| is_ident(c)) || !s.get(i).is_some_and(|&c| is_ident(c));
        if ok && ends_word && best.is_none_or(|b| i > b) {
            best = Some(i);
        }
    }
    best
}

/// Classes of a line of text being edited.
pub fn lex_classes(t: &[u8]) -> Vec<Class> {
    let mut out = vec![Class::Normal; t.len()];
    let mut i = 0;
    // At the start of the line a quote starts a comment (`_TkRem2`);
    // elsewhere it starts a string.
    let mut stmt_start = true;
    while i < t.len() {
        let c = t[i];
        if c == b' ' {
            i += 1;
            continue;
        }
        if c == b'\'' && stmt_start {
            out[i..].fill(Class::Comment);
            break;
        }
        stmt_start = false;
        if c == b'"' || c == b'\'' {
            let e = t[i + 1..].iter().position(|&d| d == c).map_or(t.len(), |p| i + 2 + p);
            out[i..e].fill(Class::String);
            i = e;
            continue;
        }
        let prev_ident = i > 0 && is_ident(t[i - 1]);
        if !prev_ident && (c.is_ascii_digit() || (c == b'.' && t.get(i + 1).is_some_and(u8::is_ascii_digit))) {
            let mut e = i;
            while e < t.len() && (t[e].is_ascii_digit() || t[e] == b'.') {
                e += 1;
            }
            if e < t.len() && (t[e] == b'e' || t[e] == b'E') {
                let mut f = e + 1;
                if f < t.len() && (t[f] == b'-' || t[f] == b'+') {
                    f += 1;
                }
                if f < t.len() && t[f].is_ascii_digit() {
                    e = f;
                    while e < t.len() && t[e].is_ascii_digit() {
                        e += 1;
                    }
                }
            }
            out[i..e].fill(Class::Number);
            i = e;
            continue;
        }
        if (c == b'$' || c == b'%') && !prev_ident {
            let digits = |d: u8| if c == b'$' { d.is_ascii_hexdigit() } else { d == b'0' || d == b'1' };
            let e = i + 1 + t[i + 1..].iter().take_while(|&&d| digits(d)).count();
            if e > i + 1 {
                out[i..e].fill(Class::Number);
                i = e;
                continue;
            }
        }
        if c.is_ascii_alphabetic() && !prev_ident {
            if let Some(n) = match_keyword(&t[i..]) {
                if t[i..i + n].eq_ignore_ascii_case(b"rem") {
                    out[i..].fill(Class::Comment);
                    break;
                }
                out[i..i + n].fill(Class::Keyword);
                i += n;
                continue;
            }
            while i < t.len() && (is_ident(t[i]) || t[i] == b'$' || t[i] == b'#') {
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenise::tokenise_line;

    fn classes_of(src: &str) -> (String, String) {
        let t = tokenise_line(src.as_bytes()).unwrap().unwrap();
        let (text, cls) = detok_classes(&t.line);
        let codes: String = cls
            .iter()
            .map(|c| match c {
                Class::Normal => '.',
                Class::Keyword => 'K',
                Class::String => 'S',
                Class::Comment => 'C',
                Class::Number => 'N',
            })
            .collect();
        (crate::detok::latin1_to_string(&text), codes)
    }

    #[test]
    fn classes_of_a_line() {
        let (t, c) = classes_of("print \"hi\";a+$FF : rem done");
        assert_eq!(t, "Print \"hi\";A+$FF : Rem done ");
        assert_eq!(c, "KKKKK.SSSS...NNN...CCCCCCCCC");
        let (t, c) = classes_of("x#=sin(1.5)*%101 : a$=' note'");
        assert_eq!(t, "X#=Sin(1.5)*%101 : A$=' note'");
        assert_eq!(c, "...KKK.NNN..NNNN......SSSSSSS");
        let (_, c) = classes_of("  ' note");
        assert_eq!(c, "..CCCCCCC");
        // Extension keywords are keywords too.
        let (t, c) = classes_of("track play 1");
        assert_eq!(t, "Track Play 1");
        assert_eq!(c, "KKKKKKKKKK.N");
    }

    #[test]
    fn lexer_for_edited_lines() {
        let code = |s: &str| -> String {
            lex_classes(s.as_bytes())
                .iter()
                .map(|c| match c {
                    Class::Normal => '.',
                    Class::Keyword => 'K',
                    Class::String => 'S',
                    Class::Comment => 'C',
                    Class::Number => 'N',
                })
                .collect()
        };
        assert_eq!(code("print \"hi\";a+$ff : rem x"), "KKKKK.SSSS...NNN...CCCCC");
        assert_eq!(code("  ' comment"), "..CCCCCCCCC");
        assert_eq!(code("a$='x' : b=1"), "...SSS.....N");
        assert_eq!(code("screen open 0,320"), "KKKKKKKKKKK.N.NNN");
        assert_eq!(code("printer=1"), "........N");
    }

    /// The text must be exactly the listing of `detok_line`.
    #[test]
    fn same_text_as_detok() {
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../AMOS-Professional-365/AMOS/Examples");
        let mut n = 0;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("amos")) {
                    let Ok(prg) = crate::Program::load(&std::fs::read(&p).unwrap()) else { continue };
                    for (_, l) in prg.lines() {
                        let (t, c) = detok_classes(l);
                        assert_eq!(t, crate::detok::detok_line(l), "{}", p.display());
                        assert_eq!(t.len(), c.len());
                        n += 1;
                    }
                }
            }
        }
        assert!(n > 1000);
    }
}
