//! AMOS token tables and special token values.
//!
//! A keyword token is the byte offset of its entry in the library token table
//! (`C_Tk` in `+Lib.s`), operators are negative offsets from the end of the
//! editor's operator table, and extension keywords are `TK_EXT` followed by the
//! extension slot and the offset in that extension's table. The tables in
//! [`table`] are generated from the original sources by
//! `tools/gen_tokens_rs.py`, so token values match real `.AMOS` files exactly.

mod table;
#[path = "names.rs"]
pub mod tk;

use std::collections::HashMap;
use std::sync::OnceLock;

pub use table::{EXTENSIONS, MAIN, OPERATORS};

/// One entry of a token table.
#[derive(Debug)]
pub struct TokenDef {
    /// Token value (byte offset in the table, or negative offset for operators).
    pub token: u16,
    /// Keyword in lower case, without the overload marker `!`.
    pub name: &'static str,
    /// Kind and parameter signature, e.g. `"I0,0t0,0"` (see the module docs of
    /// [`Signature`]).
    pub params: &'static str,
    /// -1 last definition, -2 an overload follows, -3 overload of the other kind.
    pub term: i8,
    /// Name of the 68000 routine implementing the instruction form (for reference).
    pub inst: &'static str,
    /// Name of the 68000 routine implementing the function form (for reference).
    pub func: &'static str,
    /// Verifier classes from the `//$AA$BB$CC$DD` comments of `+Lib.s`
    /// (instruction class, operand class, offset of the type char, compiler class).
    pub verif: [u8; 4],
}

impl TokenDef {
    pub fn kind(&self) -> TokenKind {
        match self.params.as_bytes().first() {
            Some(b'I') => TokenKind::Instruction,
            // '4': integer or float, depending on the parameter (Abs, Int...).
            Some(b'0' | b'4') => TokenKind::Function(ValueType::Int),
            Some(b'1') => TokenKind::Function(ValueType::Float),
            Some(b'2') => TokenKind::Function(ValueType::Str),
            Some(b'V') => TokenKind::ReservedVariable,
            Some(b'O') => TokenKind::Operator,
            Some(b'C') => TokenKind::Constant,
            _ => TokenKind::Structure,
        }
    }

    /// Parameter type letters, without the kind prefix (and without the type
    /// digit of reserved variables).
    pub fn param_types(&self) -> &'static str {
        let p = self.params;
        match p.as_bytes().first() {
            Some(b'V') => p.get(2..).unwrap_or(""),
            Some(_) => &p[1..],
            None => "",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Int,
    Float,
    Str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Instruction,
    Function(ValueType),
    ReservedVariable,
    Operator,
    Constant,
    /// Structural keyword (For, To, Then, Procedure...) with no signature.
    Structure,
}

pub const TK_EOL: u16 = 0x0000;
pub const TK_VAR: u16 = 0x0006;
pub const TK_LAB: u16 = 0x000C;
pub const TK_PRO: u16 = 0x0012;
pub const TK_LGO: u16 = 0x0018;
pub const TK_BIN: u16 = 0x001E;
pub const TK_CH1: u16 = 0x0026;
pub const TK_CH2: u16 = 0x002E;
pub const TK_HEX: u16 = 0x0036;
pub const TK_ENT: u16 = 0x003E;
pub const TK_FL: u16 = 0x0046;
pub const TK_EXT: u16 = 0x004E;
pub const TK_DP: u16 = 0x0054;
pub const TK_COMMA: u16 = 0x005C;
pub const TK_SEMI: u16 = 0x0064;
pub const TK_HASH: u16 = 0x006C;
pub const TK_PAR1: u16 = 0x0074;
pub const TK_PAR2: u16 = 0x007C;
pub const TK_BRA1: u16 = 0x0084;
pub const TK_BRA2: u16 = 0x008C;
pub const TK_TO: u16 = 0x0094;
pub const TK_NOT: u16 = 0x009C;
pub const TK_FOR: u16 = 0x023C;
pub const TK_REPEAT: u16 = 0x0250;
pub const TK_WHILE: u16 = 0x0268;
pub const TK_DO: u16 = 0x027E;
pub const TK_EXIT_IF: u16 = 0x0290;
pub const TK_EXIT: u16 = 0x029E;
pub const TK_IF: u16 = 0x02BE;
pub const TK_ELSE: u16 = 0x02D0;
pub const TK_ON: u16 = 0x0316;
pub const TK_PROCEDURE: u16 = 0x0376;
pub const TK_END_PROC: u16 = 0x0390;
pub const TK_DATA: u16 = 0x0404;
pub const TK_REM1: u16 = 0x064A;
pub const TK_REM2: u16 = 0x0652;
pub const TK_ELSE_IF: u16 = 0x25A4;
pub const TK_EQU: u16 = 0x2A40;
pub const TK_STRUC_STR: u16 = 0x2A64;
pub const TK_DFL: u16 = 0x2B6A;
/// Machine code procedure marker (`@_apml_@`).
pub const TK_ML: u16 = 0x258C;

/// Number of extension slots (0 = main library, 1..=6 = standard extensions).
pub const EXTENSION_SLOTS: usize = 7;

/// Returns the definition of a main-library token or an operator.
#[inline]
pub fn lookup(token: u16) -> Option<&'static TokenDef> {
    if token & 0x8000 != 0 {
        return OPERATORS.binary_search_by_key(&token, |d| d.token).ok().map(|i| &OPERATORS[i]);
    }
    lookup_ext(0, token)
}

/// Returns the definition of a token in extension `slot` (0 = main library).
#[inline]
pub fn lookup_ext(slot: usize, offset: u16) -> Option<&'static TokenDef> {
    let table = EXTENSIONS.get(slot)?;
    // Direct index (the interpreter looks keywords up at every call).
    let i = *token_index()[slot].get(offset as usize)?;
    (i != u16::MAX).then(|| &table[i as usize])
}

/// For each extension slot: token value -> index in its table (`u16::MAX`
/// for none). Same answers as a binary search of the sorted tables.
#[inline]
fn token_index() -> &'static [Vec<u16>; EXTENSION_SLOTS] {
    static INDEX: OnceLock<[Vec<u16>; EXTENSION_SLOTS]> = OnceLock::new();
    INDEX.get_or_init(|| {
        std::array::from_fn(|slot| {
            let table = EXTENSIONS[slot];
            let max = table.iter().map(|d| d.token as usize).max().unwrap_or(0);
            let mut v = vec![u16::MAX; max + 1];
            for (t, e) in v.iter_mut().enumerate() {
                if let Ok(i) = table.binary_search_by_key(&(t as u16), |d| d.token) {
                    *e = i as u16;
                }
            }
            v
        })
    })
}

/// Parameter type letters of a keyword (`TokenDef::param_types`, "" when
/// unknown), from a table indexed by token: read at every instruction and
/// function call.
#[inline]
pub fn param_types_of(kw: Keyword) -> &'static str {
    static SIGS: OnceLock<[Vec<&'static str>; EXTENSION_SLOTS]> = OnceLock::new();
    if kw.slot == 0 && kw.token & 0x8000 != 0 {
        return kw.def().map_or("", |d| d.param_types());
    }
    let sigs = SIGS.get_or_init(|| {
        std::array::from_fn(|slot| {
            token_index()[slot]
                .iter()
                .map(|&i| if i == u16::MAX { "" } else { EXTENSIONS[slot][i as usize].param_types() })
                .collect()
        })
    });
    sigs.get(kw.slot as usize).and_then(|v| v.get(kw.token as usize)).copied().unwrap_or("")
}

/// All variants of the overloaded keyword `token` (in table order); a
/// single element slice for keywords without overloads.
pub fn overload_group(slot: usize, token: u16) -> &'static [TokenDef] {
    let table = EXTENSIONS[slot];
    let Ok(i) = table.binary_search_by_key(&token, |d| d.token) else { return &[] };
    let name = table[i].name;
    let mut a = i;
    while a > 0 && table[a - 1].name == name && table[a - 1].term != -1 {
        a -= 1;
    }
    let mut b = i;
    while table[b].term != -1 && b + 1 < table.len() && table[b + 1].name == name {
        b += 1;
    }
    &table[a..=b]
}

/// Number of bytes of inline data following a token word.
pub fn inline_data_size(token: u16) -> usize {
    match token {
        TK_FOR | TK_REPEAT | TK_WHILE | TK_DO | TK_IF | TK_ELSE | TK_ELSE_IF | TK_DATA => 2,
        TK_EXIT | TK_EXIT_IF | TK_ON | TK_EXT => 4,
        TK_PROCEDURE => 8,
        TK_EQU..=TK_STRUC_STR => 6,
        _ => 0,
    }
}

/// A keyword reference: extension slot (0 = main library) and token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Keyword {
    pub slot: u8,
    pub token: u16,
}

impl Keyword {
    #[inline]
    pub fn def(&self) -> Option<&'static TokenDef> {
        if self.slot == 0 { lookup(self.token) } else { lookup_ext(self.slot as usize, self.token) }
    }
}

/// All keyword spellings, grouped by name, for the tokeniser.
pub struct KeywordIndex {
    /// name -> every keyword with that name (overloads), in table order.
    pub by_name: HashMap<&'static str, Vec<Keyword>>,
    /// Keyword names sorted by decreasing length for longest-match scanning.
    pub names_longest_first: Vec<&'static str>,
}

pub fn keyword_index() -> &'static KeywordIndex {
    static INDEX: OnceLock<KeywordIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut by_name: HashMap<&'static str, Vec<Keyword>> = HashMap::new();
        for d in OPERATORS {
            by_name.entry(d.name).or_default().push(Keyword { slot: 0, token: d.token });
        }
        for (slot, table) in EXTENSIONS.iter().enumerate() {
            for d in table.iter() {
                // Skip the special entries (constants, variables) and dummies.
                if d.name.is_empty() || d.name.starts_with('@') {
                    continue;
                }
                by_name.entry(d.name).or_default().push(Keyword { slot: slot as u8, token: d.token });
            }
        }
        let mut names: Vec<&'static str> = by_name.keys().copied().collect();
        names.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        KeywordIndex { by_name, names_longest_first: names }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_types_table_matches_the_definitions() {
        for slot in 0..EXTENSION_SLOTS as u8 + 2 {
            for token in 0..=u16::MAX {
                let kw = Keyword { slot, token };
                assert_eq!(param_types_of(kw), kw.def().map_or("", |d| d.param_types()), "{kw:?}");
            }
        }
    }

    #[test]
    fn known_tokens() {
        assert_eq!(lookup(TK_FOR).unwrap().name, "for ");
        assert_eq!(lookup(TK_PROCEDURE).unwrap().name, "procedure ");
        assert_eq!(lookup(0xFFA2).unwrap().name, "=");
        assert_eq!(lookup(0x0476).unwrap().name, "print");
        assert_eq!(lookup(0x09EA).unwrap().kind(), TokenKind::Instruction);
        assert_eq!(lookup(0x09EA).unwrap().param_types(), "0,0,0,0,0");
    }
}
