//! Tokeniser: converts a line of AMOS text into tokens, following the editor
//! routine `Tokenise` (`+Edit.s`) and the number parser `ValRout` (`+ILib.s`).
//!
//! Keyword matching rules of the original are kept, including its quirks:
//! operators are matched first (first match wins), keywords starting with a
//! letter use the longest match over all tables, spaces inside keywords are
//! optional in the input, and a keyword is recognised even when it is the
//! beginning of a longer word.

use std::sync::OnceLock;

use crate::tokens::*;

/// Maximum tokenised line length in bytes.
pub const MAX_LINE: usize = 510;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokeniseError {
    /// The tokenised line is longer than 510 bytes.
    LineTooLong,
}

/// Result of tokenising one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokenised {
    /// The complete tokenised line (length byte, indent byte, tokens, 0 word).
    pub line: Vec<u8>,
    /// The line contained `Set Double Precision`.
    pub double_precision: bool,
}

/// Parsed numeric constant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Number {
    Int(i32),
    Hex(u32),
    Bin(u32),
    /// Value, and the FFP bits `a2ffp` gives for the original text.
    Float(f64, crate::ffp::Ffp),
}

impl Number {
    pub fn value_f64(self) -> f64 {
        match self {
            Number::Int(i) => i as f64,
            Number::Hex(h) | Number::Bin(h) => h as i32 as f64,
            Number::Float(f, _) => f,
        }
    }
}

fn lower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

/// `ValRout`: parses a number at the start of `s` (spaces are ignored inside
/// numbers, as in the original). Returns the number and the bytes consumed.
/// With `signed`, a leading `-` or `+` is accepted.
pub fn parse_number(s: &[u8], signed: bool) -> Option<(Number, usize)> {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut i = 0;
    let mut neg = false;
    // Where `ValRout` starts the text of a float (`a2`): the sign.
    let mut from = 0;
    if signed {
        while at(i) == b' ' {
            i += 1;
        }
        from = i;
        match at(i) {
            b'-' => {
                neg = true;
                i += 1;
            }
            b'+' => i += 1,
            _ => {}
        }
    }
    while at(i) == b' ' {
        i += 1;
    }
    match at(i) {
        b'$' => {
            i += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            loop {
                let c = at(i);
                let d = match c {
                    b'0'..=b'9' => c - b'0',
                    b'a'..=b'f' => c - b'a' + 10,
                    b'A'..=b'F' => c - b'A' + 10,
                    _ => break,
                };
                n += 1;
                if n == 9 {
                    return None;
                }
                v = (v << 4) | d as u32;
                i += 1;
            }
            if n == 0 {
                return None;
            }
            let v = if neg { (v as i32).wrapping_neg() as u32 } else { v };
            Some((Number::Hex(v), i))
        }
        b'%' => {
            i += 1;
            let mut v: u32 = 0;
            let mut n = 0;
            loop {
                while at(i) == b' ' {
                    i += 1;
                }
                let c = at(i);
                if c != b'0' && c != b'1' {
                    break;
                }
                n += 1;
                if n == 33 {
                    return None;
                }
                v = (v << 1) | (c - b'0') as u32;
                i += 1;
            }
            if n == 0 {
                return None;
            }
            let v = if neg { (v as i32).wrapping_neg() as u32 } else { v };
            Some((Number::Bin(v), i))
        }
        b'.' | b'0'..=b'9' => {
            // Find the end of the number and decide int or float.
            let mut j = i;
            let mut is_float = false;
            let mut dot = false;
            loop {
                let c = at(j);
                if c == b' ' || c.is_ascii_digit() {
                    j += 1;
                    continue;
                }
                if c == b'.' {
                    if dot {
                        break;
                    }
                    dot = true;
                    is_float = true;
                    j += 1;
                    continue;
                }
                if c == b'e' || c == b'E' {
                    let mut k = j + 1;
                    while at(k) == b' ' {
                        k += 1;
                    }
                    let mut exp_float = false;
                    if at(k) == b'+' || at(k) == b'-' {
                        exp_float = true;
                        k += 1;
                        while at(k) == b' ' {
                            k += 1;
                        }
                    }
                    if at(k).is_ascii_digit() {
                        exp_float = true;
                    }
                    if exp_float {
                        is_float = true;
                        while at(k).is_ascii_digit() || at(k) == b' ' {
                            k += 1;
                        }
                        j = k;
                    }
                }
                break;
            }
            if is_float {
                // `BuFloat`: from the sign, without spaces, at most 33
                // characters, converted by `AscToDouble` / `AscToFloat`.
                let text = crate::softdouble::bufloat(&s[from..j]);
                let v = crate::softdouble::asc_to_f64(&text);
                let ffp = crate::ffp::ascii_to_ffp(&text);
                // Trailing spaces belong to the number for the original too.
                Some((Number::Float(v, ffp), j))
            } else {
                let mut v: u32 = 0;
                let mut digits = 0;
                let mut k = i;
                loop {
                    let c = at(k);
                    if c == b' ' {
                        k += 1;
                        continue;
                    }
                    if !c.is_ascii_digit() {
                        break;
                    }
                    let nv = (v as u64) * 10 + (c - b'0') as u64;
                    if nv > i32::MAX as u64 {
                        return None;
                    }
                    v = nv as u32;
                    digits += 1;
                    k += 1;
                }
                if digits == 0 {
                    return None;
                }
                let v = if neg { -(v as i32) } else { v as i32 };
                Some((Number::Int(v), k))
            }
        }
        _ => None,
    }
}

/// Parses float text such as `1.5`, `.5`, `1e10`, `2.e-3` (lower case, no spaces).
pub fn parse_float_text(t: &str) -> f64 {
    let mut s = t.to_string();
    if s.starts_with('.') {
        s.insert(0, '0');
    }
    if let Some(p) = s.find(".e") {
        s.insert(p + 1, '0');
    }
    if s.ends_with('.') {
        s.push('0');
    }
    // Incomplete exponents ("1e") are ignored.
    if s.ends_with('e') || s.ends_with("e+") || s.ends_with("e-") {
        s.truncate(s.find('e').unwrap());
    }
    s.parse::<f64>().unwrap_or(0.0)
}

/// Lookup structures for keyword matching.
struct Tables {
    /// Operators in table order: (name bytes, token).
    operators: Vec<(&'static [u8], u16)>,
    /// Per table (main + extensions): all entries with their match name.
    tables: Vec<Vec<Entry>>,
}

struct Entry {
    /// Keyword bytes (without `!`), last byte is the terminating char.
    name: &'static [u8],
    token: u16,
    /// First significant letter (after skipping `!` and a leading space).
    first: u8,
    /// Length of the full table name for longest-match comparison.
    len: usize,
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let operators = OPERATORS.iter().map(|d| (d.name.as_bytes(), d.token)).collect();
        let mut tables = Vec::new();
        for (slot, table) in EXTENSIONS.iter().enumerate() {
            let mut v = Vec::new();
            // Skip the dummy first entry; in the main table skip the special
            // entries that have no name.
            for d in table.iter() {
                if d.name.is_empty() || (slot != 0 && d.token == 0) || d.params.starts_with('C') {
                    continue;
                }
                // Variants with a reused name ($80 entries) repeat the name:
                // only the first entry of an overload group is matched.
                if v.last().is_some_and(|e: &Entry| e.name == d.name.as_bytes()) {
                    continue;
                }
                let name = d.name.as_bytes();
                let first = name.iter().copied().find(|&c| c != b' ').unwrap_or(0);
                v.push(Entry { name, token: d.token, first, len: name.len() });
            }
            tables.push(v);
        }
        Tables { operators, tables }
    })
}

/// Attempts to match keyword `name` against the input starting at `input`,
/// where the first significant keyword character has already been matched
/// against the input's first character. Returns input bytes consumed.
fn match_rest(name: &[u8], input: &[u8], first_char_idx: usize) -> Option<usize> {
    // Position in the name just after its first significant character.
    let mut k = first_char_idx + 1;
    let mut i = 1; // input index (first char already matched)
    if k > name.len() {
        return None;
    }
    // When the first significant char was the last one, the match is complete.
    if k == name.len() {
        return Some(1);
    }
    loop {
        let kc = name[k];
        let last = k + 1 == name.len();
        let ic = lower(input.get(i).copied().unwrap_or(0));
        if last {
            if kc == b' ' {
                return Some(i);
            }
            return if kc == ic && ic != 0 { Some(i + 1) } else { None };
        }
        if kc == b' ' {
            if ic == b' ' {
                i += 1;
            }
            k += 1;
            continue;
        }
        if kc == ic && ic != 0 {
            i += 1;
            k += 1;
            continue;
        }
        return None;
    }
}

/// Index of the first significant character in a keyword name (skipping a
/// leading space, as for `" xor "`).
fn first_index(name: &[u8]) -> usize {
    name.iter().position(|&c| c != b' ').unwrap_or(0)
}

/// Finds the keyword at the start of `input`. Returns `(slot, token, consumed)`.
fn find_keyword(input: &[u8]) -> Option<(u8, u16, usize)> {
    let t = tables();
    let c0 = lower(input[0]);
    // Operators first, first match wins.
    for &(name, token) in &t.operators {
        let fi = first_index(name);
        if name[fi] == c0
            && let Some(n) = match_rest(name, input, fi)
        {
            return Some((0, token, n));
        }
    }
    let letter = c0.is_ascii_lowercase();
    let mut best: Option<(usize, u8, u16, usize)> = None; // (name len, slot, token, consumed)
    for (slot, table) in t.tables.iter().enumerate() {
        for e in table {
            if e.first != c0 {
                continue;
            }
            let fi = first_index(e.name);
            if let Some(n) = match_rest(e.name, input, fi) {
                if !letter {
                    return Some((slot as u8, e.token, n));
                }
                if best.is_none_or(|b| e.len > b.0) {
                    best = Some((e.len, slot as u8, e.token, n));
                }
            }
        }
    }
    best.map(|(_, s, t, n)| (s, t, n))
}

struct Out {
    buf: Vec<u8>,
}

impl Out {
    fn w(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn l(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn pad(&mut self, b: u8) {
        if self.buf.len() & 1 != 0 {
            self.buf.push(b);
        }
    }
}

fn is_var_char(c: u8) -> bool {
    c == b'_' || c.is_ascii_digit() || c.is_ascii_lowercase() || c > 128
}

/// Tokenises one line of text (Latin-1 bytes, no line terminator).
/// Blank lines are kept (with their indentation) as the editor does.
pub fn tokenise_line(text: &[u8]) -> Result<Option<Tokenised>, TokeniseError> {
    let mut o = Out { buf: vec![0, 0] };
    let mut double_precision = false;
    let at = |i: usize| text.get(i).copied().unwrap_or(0);

    // Indent.
    let mut i = 0;
    while at(i) == b' ' {
        i += 1;
    }
    // A completely empty line has indent 0 (`TokVide`); indented blank lines
    // keep their indentation.
    o.buf[1] = if text.is_empty() { 0 } else { (i + 1).min(127) as u8 };
    if at(i) == 0 {
        return finish(o, false);
    }

    let mut line_start = true;

    // A number at the start of the line is a line-number label.
    if at(i).is_ascii_digit() {
        let s = i;
        while at(i).is_ascii_digit() {
            i += 1;
        }
        write_var(&mut o, TK_LAB, &text[s..i], 0);
        if at(i) == b':' {
            i += 1;
        }
        line_start = false;
    } else if at(i) == b'\'' {
        i += 1;
        o.w(TK_REM2);
        write_rem(&mut o, &text[i..]);
        return finish(o, double_precision);
    }

    loop {
        if o.buf.len() > 512 {
            return Err(TokeniseError::LineTooLong);
        }
        let c = at(i);
        if c == 0 {
            break;
        }
        if c == b' ' {
            i += 1;
            continue;
        }
        // Strings.
        if c == b'"' || c == b'\'' {
            let q = c;
            i += 1;
            let s = i;
            while at(i) != 0 && at(i) != q {
                i += 1;
            }
            let body = &text[s..i];
            if at(i) == q {
                i += 1;
            }
            o.w(if q == b'"' { TK_CH1 } else { TK_CH2 });
            o.w(body.len() as u16);
            o.buf.extend_from_slice(body);
            o.pad(0);
            continue;
        }
        // Numbers.
        if let Some((num, n)) = parse_number(&text[i..], false) {
            i += n;
            match num {
                Number::Int(v) => {
                    o.w(TK_ENT);
                    o.l(v as u32);
                }
                Number::Hex(v) => {
                    o.w(TK_HEX);
                    o.l(v);
                }
                Number::Bin(v) => {
                    o.w(TK_BIN);
                    o.l(v);
                }
                Number::Float(f, ffp) => {
                    if double_precision {
                        o.w(TK_DFL);
                        o.buf.extend_from_slice(&f.to_be_bytes());
                    } else {
                        o.w(TK_FL);
                        o.l(ffp.0);
                    }
                }
            }
            line_start = false;
            continue;
        }
        // `?` = Print / Print #.
        if c == b'?' {
            i += 1;
            let mut j = i;
            while at(j) == b' ' {
                j += 1;
            }
            if at(j) == b'#' {
                o.w(tk::PRINT_HASH);
                i = j + 1;
            } else {
                o.w(tk::PRINT);
            }
            line_start = false;
            continue;
        }
        if c == b'!' {
            i += 1;
            continue;
        }
        // Keywords.
        if let Some((slot, token, n)) = find_keyword(&text[i..]) {
            i += n;
            line_start = false;
            if slot != 0 {
                o.w(TK_EXT);
                o.buf.push(slot);
                o.buf.push(0);
                o.w(token);
                continue;
            }
            o.w(token);
            match token {
                TK_EQU..=TK_STRUC_STR => o.buf.extend_from_slice(&[0; 6]),
                TK_ON | TK_EXIT | TK_EXIT_IF => o.l(0),
                TK_DATA | TK_FOR | TK_REPEAT | TK_WHILE | TK_DO | TK_IF | TK_ELSE_IF | TK_ELSE => o.w(0),
                TK_PROCEDURE => o.buf.extend_from_slice(&[0; 8]),
                TK_REM1 => {
                    write_rem(&mut o, &text[i..]);
                    return finish(o, double_precision);
                }
                _ => {}
            }
            if token == tk::SET_DOUBLE_PRECISION {
                double_precision = true;
            }
            if token == TK_ELSE || token == tk::THEN {
                // A line number after Then/Else is a label reference.
                let mut j = i;
                while at(j) == b' ' {
                    j += 1;
                }
                if at(j).is_ascii_digit() {
                    let s = j;
                    while is_var_char(lower(at(j))) {
                        j += 1;
                    }
                    let name: Vec<u8> = text[s..j].iter().map(|&c| lower(c)).collect();
                    write_var(&mut o, TK_LGO, &name, 0);
                    i = j;
                }
            }
            continue;
        }
        // Variables, labels and procedure names.
        let lc = lower(c);
        if lc == b'_' || lc.is_ascii_lowercase() || c > 128 {
            let s = i;
            i += 1;
            while is_var_char(lower(at(i))) {
                i += 1;
            }
            let name: Vec<u8> = text[s..i].iter().map(|&c| lower(c)).collect();
            if line_start && at(i) == b':' {
                i += 1;
                write_var(&mut o, TK_LAB, &name, 0);
            } else {
                let flags = match at(i) {
                    b'$' => {
                        i += 1;
                        2
                    }
                    b'#' => {
                        i += 1;
                        1
                    }
                    _ => 0,
                };
                write_var(&mut o, TK_VAR, &name, flags);
            }
            line_start = false;
            continue;
        }
        // Anything else is ignored, as in the original.
        i += 1;
    }
    finish(o, double_precision)
}

fn write_var(o: &mut Out, token: u16, name: &[u8], flags: u8) {
    o.w(token);
    o.w(0);
    let len = (name.len() + 1) & !1;
    o.buf.push(len as u8);
    o.buf.push(flags);
    o.buf.extend_from_slice(name);
    o.pad(0);
}

fn write_rem(o: &mut Out, text: &[u8]) {
    let len_pos = o.buf.len();
    o.w(0);
    o.buf.extend_from_slice(text);
    o.pad(b' ');
    let n = o.buf.len() - len_pos - 2;
    o.buf[len_pos..len_pos + 2].copy_from_slice(&(n as u16).to_be_bytes());
}

fn finish(mut o: Out, double_precision: bool) -> Result<Option<Tokenised>, TokeniseError> {
    o.w(0);
    if o.buf.len() >= MAX_LINE {
        return Err(TokeniseError::LineTooLong);
    }
    o.buf[0] = (o.buf.len() / 2) as u8;
    Ok(Some(Tokenised { line: o.buf, double_precision }))
}

/// Tokenises a whole program from text (lines separated by LF or CR LF).
pub fn tokenise_program(text: &[u8]) -> Result<crate::program::Program, (usize, TokeniseError)> {
    let mut prg = crate::program::Program::default();
    for (n, raw) in text.split(|&c| c == b'\n').enumerate() {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        // Tabs are expanded to spaces as the editor does on load.
        let line: Vec<u8> = raw.iter().map(|&c| if c == b'\t' { b' ' } else { c }).collect();
        match tokenise_line(&line) {
            Ok(Some(t)) => {
                if t.double_precision {
                    prg.math_flags |= 0x83;
                }
                prg.source.extend_from_slice(&t.line);
            }
            Ok(None) => {}
            Err(e) => return Err((n + 1, e)),
        }
    }
    Ok(prg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detok::{detok_line, latin1_to_string, list_program};
    use crate::program::Program;

    fn roundtrip(s: &str) -> String {
        let t = tokenise_line(s.as_bytes()).unwrap().unwrap();
        latin1_to_string(&detok_line(&t.line))
    }

    #[test]
    fn simple_lines() {
        assert_eq!(roundtrip("print \"hello\""), "Print \"hello\"");
        assert_eq!(roundtrip("a=10*3/4"), "A=10*3/4");
        assert_eq!(roundtrip("for i=1 to 10 : next i"), "For I=1 To 10 : Next I");
        assert_eq!(roundtrip("screen open 0,320,256,16,lowres"), "Screen Open 0,320,256,16,Lowres");
        assert_eq!(roundtrip("  if a$=\"x\" then goto 10"), "  If A$=\"x\" Then Goto 10");
        assert_eq!(roundtrip("label: x#=1.5"), "LABEL: X#=1.5");
        assert_eq!(roundtrip("x=$FF+%101"), "X=$FF+%101");
        assert_eq!(roundtrip("' comment"), "' comment");
    }

    fn amos_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                amos_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("amos")) {
                out.push(path);
            }
        }
    }

    /// Re-tokenising the listing of every program in the source tree must
    /// give back the same tokens.
    #[test]
    fn retokenise_all_programs() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../AMOS-Professional-365");
        let mut files = Vec::new();
        amos_files(&dir, &mut files);
        assert!(files.len() > 100);
        for path in files {
            let prg = Program::load(&std::fs::read(&path).unwrap()).unwrap();
            let listing = list_program(&prg);
            let re = tokenise_program(&listing).unwrap();
            for ((_, a), (_, b)) in prg.lines().zip(re.lines()) {
                // Listings can differ in spacing when the verifier picked another
                // variant of an overloaded keyword, so compare the tokens.
                let la = latin1_to_string(&detok_line(a));
                // Strings holding NUL bytes cannot survive a text round trip.
                if la.contains('\0') {
                    break;
                }
                // Variables named like keywords added in AMOS Pro (ERR$).
                if path.ends_with("_Get_DOS_Error.AMOS") {
                    break;
                }
                // AMOS 1.3 programs hold "Else" + "If" where Pro tokenises "Else If".
                if la.contains("Else If") {
                    continue;
                }
                assert_eq!(strip_patched(a), strip_patched(b), "{}: {la}", path.display());
            }
        }
    }

    /// Clears fields filled in by the verifier so token streams can be compared.
    fn strip_patched(line: &[u8]) -> Vec<u8> {
        let mut v = line.to_vec();
        // Empty lines are stored with indent 0 or 1 depending on how they were made.
        if v.len() == 4 && v[1] <= 1 {
            v[1] = 1;
        }
        let mut p = 2;
        while p + 2 <= v.len() {
            let t = u16::from_be_bytes([v[p], v[p + 1]]);
            p += 2;
            if t == 0 {
                break;
            }
            match t {
                TK_VAR | TK_LAB | TK_PRO | TK_LGO => {
                    v[p] = 0;
                    v[p + 1] = 0;
                    v[p + 3] &= 0x0F & !0x08;
                    let len = v[p + 2] as usize;
                    // Procedure calls are stored as variables before testing.
                    v[p - 2..p].copy_from_slice(&if t == TK_PRO || t == TK_LGO { TK_VAR } else { t }.to_be_bytes());
                    p += 4 + len;
                }
                TK_CH1 | TK_CH2 | TK_REM1 | TK_REM2 => {
                    let n = u16::from_be_bytes([v[p], v[p + 1]]) as usize;
                    p += 2 + n + (n & 1);
                }
                TK_ENT | TK_HEX | TK_BIN | TK_FL => p += 4,
                TK_DFL => p += 8,
                TK_EXT => {
                    v[p + 1] = 0;
                    let off = u16::from_be_bytes([v[p + 2], v[p + 3]]);
                    if let Some(base) = overload_group(v[p] as usize, off).first() {
                        v[p + 2..p + 4].copy_from_slice(&base.token.to_be_bytes());
                    }
                    p += 4;
                }
                _ => {
                    if t & 0x8000 == 0
                        && let Some(base) = overload_group(0, t).first()
                    {
                        v[p - 2..p].copy_from_slice(&base.token.to_be_bytes());
                    }
                    let n = inline_data_size(t);
                    for b in &mut v[p..p + n] {
                        *b = 0;
                    }
                    p += n;
                }
            }
        }
        v
    }
}
