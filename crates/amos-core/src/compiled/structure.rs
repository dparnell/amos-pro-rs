//! Structure of a verified program as seen by compiled code.
//!
//! The compiler (crate `amos-compiler`) and the runtime of compiled
//! programs must agree exactly on where instructions start (they are the
//! resume points of the compiled state machine), on the boundaries of the
//! parameters of keywords run through the keyword bridge, and on the
//! variables a statement executed by the interpreter uses. Both sides call
//! the functions of this module, so they cannot disagree.
//!
//! Instructions are found the way the interpreter runs them: after an
//! instruction completes in sequence, `pc` is at its end and `fetch` skips
//! `:` and line ends (`Interp::fetch`).

use crate::interp::verify::{Compiled, GLOBAL, VarDecl, token_size};
use crate::program::{read_u16, var_flags};
use crate::tokens::{Keyword, inline_data_size, lookup, lookup_ext, tk, *};

/// One instruction of the program (a resume point).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instr {
    /// Position of its first token.
    pub pos: usize,
    /// Position after it when it completes in sequence.
    pub end: usize,
    /// 0 for the main program, `i + 1` for procedure `i` (its body and its
    /// End Proc line).
    pub scope: usize,
}

#[inline]
pub fn rd(code: &[u8], p: usize) -> u16 {
    if p + 1 < code.len() { read_u16(code, p) } else { TK_EOL }
}

/// True if `t` ends a statement (`Interp::is_end`).
pub fn is_end(t: u16) -> bool {
    t == TK_EOL || t == TK_DP || t == TK_ELSE
}

/// Position of the next instruction at or after `p`, skipping `:` and line
/// ends like `Interp::fetch`. `None` at the end of the program.
pub fn normalize(code: &[u8], mut p: usize) -> Option<usize> {
    loop {
        if p + 1 >= code.len() {
            return None;
        }
        let t = read_u16(code, p);
        if t == TK_DP {
            p += 2;
            continue;
        }
        if t == TK_EOL {
            let next = p + 2;
            if next + 1 >= code.len() || code[next] == 0 {
                return None;
            }
            p = next + 2;
            continue;
        }
        return Some(p);
    }
}

/// Position of the end of line token of the line containing `p`.
pub fn to_eol(code: &[u8], mut p: usize) -> usize {
    while p + 1 < code.len() && read_u16(code, p) != TK_EOL {
        p += token_size(code, p);
    }
    p
}

/// Scope of an instruction position (procedure bodies include their End
/// Proc line; the Procedure token itself belongs to the enclosing flow).
pub fn scope_of(c: &Compiled, pos: usize) -> usize {
    for (i, p) in c.procs.iter().enumerate() {
        if pos > p.pos && pos <= p.end_pos {
            return i + 1;
        }
    }
    0
}

/// Skips an expression roughly (`Verifier::skip_expr`): used for If
/// conditions, which cannot contain the tokens it stops at.
pub fn skip_expr_rough(code: &[u8], mut p: usize) -> usize {
    let mut depth = 0i32;
    loop {
        let t = rd(code, p);
        match t {
            TK_EOL | TK_DP => return p,
            TK_COMMA | TK_BRA2 | TK_TO if depth == 0 => return p,
            TK_PAR1 | TK_BRA1 => depth += 1,
            TK_PAR2 => {
                if depth == 0 {
                    return p;
                }
                depth -= 1;
            }
            _ => {}
        }
        if t == TK_ELSE || t == tk::THEN {
            return p;
        }
        p += token_size(code, p);
    }
}

/// Position after the instruction starting at `p` when it completes in
/// sequence.
pub fn instr_end(code: &[u8], p: usize) -> usize {
    let t = rd(code, p);
    match t {
        TK_LAB | TK_REM1 | TK_REM2 => p + token_size(code, p),
        TK_PROCEDURE | TK_DATA | tk::DEF_FN => to_eol(code, p),
        TK_ELSE => {
            let q = p + 4;
            if rd(code, q) == TK_LGO { q + token_size(code, q) } else { q }
        }
        TK_IF => {
            let mut q = skip_expr_rough(code, p + 4);
            if rd(code, q) == tk::THEN {
                q += 2;
                if rd(code, q) == TK_LGO {
                    q += token_size(code, q);
                }
            }
            q
        }
        TK_ELSE_IF => {
            let q = skip_expr_rough(code, p + 4);
            if rd(code, q) == tk::THEN { q + 2 } else { q }
        }
        // Trap is directly followed by the trapped statement.
        tk::TRAP => p + 2,
        _ => {
            let mut q = p + token_size(code, p);
            let mut depth = 0i32;
            loop {
                let t = rd(code, q);
                if t == TK_EOL || (depth <= 0 && is_end(t)) {
                    return q;
                }
                match t {
                    TK_PAR1 | TK_BRA1 => depth += 1,
                    TK_PAR2 | TK_BRA2 => depth -= 1,
                    _ => {}
                }
                q += token_size(code, q);
            }
        }
    }
}

/// All instructions of the program in position order.
pub fn instructions(c: &Compiled) -> Vec<Instr> {
    let code = &c.code;
    let mut out = Vec::new();
    let mut line = 0;
    while line + 1 < code.len() && code[line] != 0 {
        let first = line + 2;
        let next_line = line + code[line] as usize * 2;
        if rd(code, first) == TK_PROCEDURE
            && let Some(pr) = c.procs.iter().find(|pr| pr.pos == first && pr.machine_code)
        {
            // Machine code procedure: only the Procedure token runs (it
            // skips the procedure); the body is binary data.
            out.push(Instr { pos: first, end: first + 2, scope: 0 });
            line = pr.end_line + code[pr.end_line] as usize * 2;
            continue;
        }
        let scope = scope_of(c, first);
        let mut p = first;
        loop {
            let t = rd(code, p);
            if t == TK_EOL || p >= next_line {
                break;
            }
            if t == TK_DP {
                p += 2;
                continue;
            }
            let end = instr_end(code, p);
            out.push(Instr { pos: p, end, scope });
            if end <= p {
                break;
            }
            p = end;
        }
        if next_line <= line {
            break;
        }
        line = next_line;
    }
    out
}

/// Position of the final end of line token (`normalize` of it gives `None`).
pub fn end_position(code: &[u8]) -> usize {
    let mut line = 0;
    let mut last = 0;
    while line + 1 < code.len() && code[line] != 0 {
        last = line;
        line += code[line] as usize * 2;
    }
    if line == 0 { 0 } else { to_eol(code, last + 2) }
}

// ----------------------------------------------------------------------
// Expressions and parameters
// ----------------------------------------------------------------------

/// Keyword of a function / instruction token at `p` and the position after
/// the token (with its inline data).
pub fn keyword_at(code: &[u8], p: usize) -> (Keyword, usize) {
    let t = rd(code, p);
    if t == TK_EXT {
        (Keyword { slot: code[p + 2], token: rd(code, p + 4) }, p + 6)
    } else {
        (Keyword { slot: 0, token: t }, p + 2 + inline_data_size(t))
    }
}

pub fn keyword_def(kw: Keyword) -> Option<&'static crate::tokens::TokenDef> {
    if kw.slot == 0 { lookup(kw.token) } else { lookup_ext(kw.slot as usize, kw.token) }
}

/// Core functions whose single parameter is read with `( expr )`
/// (`str_arg`, `int_arg`, `float_arg` in `interp/expr.rs`).
pub fn is_single_arg_core(t: u16) -> bool {
    use tk::*;
    matches!(
        t,
        LEN | ASC
            | CHR_S
            | VAL
            | UPPER_S
            | LOWER_S
            | FLIP_S
            | SPACE_S
            | SQR
            | LOG
            | LN
            | EXP
            | SIN
            | COS
            | TAN
            | ASIN
            | ACOS
            | ATAN
            | HSIN
            | HCOS
            | HTAN
            | RND
            | ERR_S
    )
}

/// Signature used by the interpreter to read the parameters of the function
/// `kw` (core functions with a fixed signature, others from the table).
pub fn function_signature(kw: Keyword) -> &'static str {
    use tk::*;
    if kw.slot == 0 {
        match kw.token {
            STR_S | ABS | INT | SGN => return "4",
            STRING_S | REPEAT_S | LEFT_S | RIGHT_S => return "2,0",
            MIN | MAX | BTST => return "3,3",
            PARAM | PARAM_F | PARAM_S | TRUE | FALSE | ERRN | ERRTRAP | PI_F => return "",
            t if is_single_arg_core(t) => return "3",
            _ => {}
        }
    }
    keyword_def(kw).map_or("", |d| d.param_types())
}

/// One parameter slot of a call: present (an expression from `start` to
/// `end`) or omitted, followed by an optional separator token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub present: bool,
    pub start: usize,
    pub end: usize,
    /// Separator after the slot (`TK_COMMA` / `TK_TO`), 0 for none.
    pub sep: u16,
}

/// Parameters read by `Interp::args` with signature `sig` from `q`.
/// Returns the slots and the position after them, or `None` if the tokens
/// do not follow the signature.
pub fn arg_slots(code: &[u8], mut q: usize, sig: &str) -> Option<(Vec<Slot>, usize)> {
    let sig = sig.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < sig.len() {
        let t = rd(code, q);
        let mut slot = Slot { present: false, start: q, end: q, sep: 0 };
        if !(t == TK_COMMA || t == TK_TO || t == TK_PAR2 || is_end(t)) {
            let e = skip_expr(code, q)?;
            slot = Slot { present: true, start: q, end: e, sep: 0 };
            q = e;
        }
        if let Some(&sep) = sig.get(i + 1) {
            let want = if sep == b't' { TK_TO } else { TK_COMMA };
            if rd(code, q) != want {
                return None;
            }
            slot.sep = want;
            q += 2;
        }
        out.push(slot);
        i += 2;
    }
    Some((out, q))
}

/// Shape of a function call at `p`: keyword, position after the token,
/// whether the parameters are in parentheses, the slots and the end.
#[derive(Clone, Debug)]
pub struct Call {
    pub kw: Keyword,
    pub tok_end: usize,
    pub paren: bool,
    pub slots: Vec<Slot>,
    pub end: usize,
}

/// Function call at `p` as the interpreter reads it (`operand_value` then
/// `fn_args`). `None` for functions with a special syntax (`Fn`, `Match`,
/// `Varptr`...) that the bridge cannot rebuild.
pub fn function_call(code: &[u8], p: usize) -> Option<Call> {
    let (kw, q) = keyword_at(code, p);
    if kw.slot == 0 && matches!(kw.token, tk::FN | tk::MATCH | tk::VARPTR | tk::ARRAY) {
        return None;
    }
    if kw.slot == 0 && matches!(kw.token, TK_LAB | TK_PRO | TK_LGO | TK_REM1 | TK_REM2 | TK_VAR) {
        return None;
    }
    let sig = function_signature(kw);
    if sig.is_empty() {
        return Some(Call { kw, tok_end: q, paren: false, slots: Vec::new(), end: q });
    }
    if rd(code, q) != TK_PAR1 {
        return None;
    }
    let (slots, r) = arg_slots(code, q + 2, sig)?;
    if rd(code, r) != TK_PAR2 {
        return None;
    }
    Some(Call { kw, tok_end: q, paren: true, slots, end: r + 2 })
}

/// Instruction at `p` with standard parameters (verified by `st_generic`):
/// the parameters follow the signature of the (already chosen) overload
/// and end with the statement.
pub fn instruction_call(code: &[u8], p: usize) -> Option<Call> {
    let (kw, q) = keyword_at(code, p);
    let def = keyword_def(kw)?;
    let (slots, r) = arg_slots(code, q, def.param_types())?;
    if !is_end(rd(code, r)) {
        return None;
    }
    Some(Call { kw, tok_end: q, paren: false, slots, end: r })
}

/// Skips one operand exactly as `Interp::operand` reads it.
fn skip_operand(code: &[u8], mut p: usize) -> Option<usize> {
    while rd(code, p) & 0x8000 != 0 {
        p += 2;
    }
    let t = rd(code, p);
    match t {
        TK_VAR => {
            let after = p + token_size(code, p);
            let array = code[p + 5] & var_flags::ARRAY != 0;
            if array && rd(code, after) == TK_PAR1 {
                let mut q = after + 2;
                loop {
                    q = skip_expr(code, q)?;
                    match rd(code, q) {
                        TK_COMMA => q += 2,
                        TK_PAR2 => return Some(q + 2),
                        _ => return None,
                    }
                }
            }
            Some(after)
        }
        TK_ENT | TK_HEX | TK_BIN | TK_FL | TK_DFL | TK_CH1 | TK_CH2 => Some(p + token_size(code, p)),
        TK_PAR1 => {
            let q = skip_expr(code, p + 2)?;
            if rd(code, q) != TK_PAR2 {
                return None;
            }
            Some(q + 2)
        }
        TK_NOT => skip_expr(code, p + 2),
        TK_COMMA | TK_PAR2 | TK_TO | TK_EOL | TK_DP => Some(p),
        _ if t == tk::FN => {
            let q = p + 2;
            if rd(code, q) != TK_VAR {
                return None;
            }
            let mut r = q + token_size(code, q);
            if rd(code, r) == TK_PAR1 {
                r += 2;
                loop {
                    r = skip_expr(code, r)?;
                    match rd(code, r) {
                        TK_COMMA => r += 2,
                        TK_PAR2 => return Some(r + 2),
                        _ => return None,
                    }
                }
            }
            Some(r)
        }
        _ if t == tk::MATCH => {
            let q = p + 2;
            if rd(code, q) != TK_PAR1 {
                return None;
            }
            let r = skip_operand(code, q + 2)?;
            if rd(code, r) != TK_COMMA {
                return None;
            }
            let r = skip_expr(code, r + 2)?;
            if rd(code, r) != TK_PAR2 {
                return None;
            }
            Some(r + 2)
        }
        _ if t == tk::VARPTR || t == tk::ARRAY => {
            let q = p + 2;
            if rd(code, q) != TK_PAR1 {
                return None;
            }
            let r = skip_operand(code, q + 2)?;
            if rd(code, r) != TK_PAR2 {
                return None;
            }
            Some(r + 2)
        }
        _ => function_call(code, p).map(|c| c.end),
    }
}

/// Skips one expression exactly as `Interp::eval` reads it. `None` if the
/// tokens are not a well formed expression.
pub fn skip_expr(code: &[u8], p: usize) -> Option<usize> {
    let mut q = skip_operand(code, p)?;
    while rd(code, q) & 0x8000 != 0 {
        q = skip_operand(code, q + 2)?;
    }
    Some(q)
}

// ----------------------------------------------------------------------
// Variables
// ----------------------------------------------------------------------

/// Declaration of the variable `slot` used in `scope`.
pub fn decl_of(c: &Compiled, scope: usize, slot: u16) -> Option<&VarDecl> {
    if slot & GLOBAL != 0 {
        return c.globals.get((slot & !GLOBAL) as usize);
    }
    if scope == 0 {
        return c.globals.get(slot as usize);
    }
    c.procs.get(scope - 1)?.locals.get(slot as usize)
}

/// True for declarations stored in linear memory by compiled code (scalar
/// variables; arrays and Def Fn definitions stay in the interpreter).
pub fn is_scalar(d: &VarDecl) -> bool {
    !d.array && !d.func
}

/// Key of a variable for [`resident_vars`]: globals do not depend on the
/// scope.
pub fn var_key(scope: usize, slot: u16) -> (usize, u16) {
    if slot & GLOBAL != 0 { (0, slot) } else { (scope, slot) }
}

/// Scalar variables that stay in the interpreter instead of the module's
/// memory because the machine accesses them by reference after the
/// instruction that names them: `Varptr(v)` (mapped into the emulated
/// memory, `inst_banks.rs`) and the variables of `Field` (read and written
/// by Get / Put, `inst_files.rs`). Compiled code leaves every instruction
/// using them to the interpreter. Sorted keys ([`var_key`]).
pub fn resident_vars(c: &Compiled, instrs: &[Instr]) -> Vec<(usize, u16)> {
    let code = &c.code;
    let mut out = Vec::new();
    for ins in instrs {
        let field = rd(code, ins.pos) == tk::FIELD;
        let mut p = ins.pos;
        while p < ins.end {
            let t = rd(code, p);
            let target = if t == tk::VARPTR && rd(code, p + 2) == TK_PAR1 {
                Some(p + 4)
            } else if field && t == TK_VAR {
                Some(p)
            } else {
                None
            };
            if let Some(q) = target
                && rd(code, q) == TK_VAR
                && code[q + 5] & (var_flags::ARRAY | var_flags::DEF_FN) == 0
            {
                out.push(var_key(ins.scope, rd(code, q + 2)));
            }
            p += token_size(code, p);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Scalar variables (slot, type) referenced by the tokens in `[a, b)`.
fn scalar_vars_in(c: &Compiled, scope: usize, a: usize, b: usize, out: &mut Vec<(u16, u8)>) {
    let code = &c.code;
    let mut p = a;
    while p < b {
        let t = rd(code, p);
        if t == TK_EOL && p + 2 >= b {
            break;
        }
        if t == TK_VAR {
            let slot = rd(code, p + 2);
            let flags = code[p + 5];
            if flags & (var_flags::ARRAY | var_flags::DEF_FN) == 0
                && let Some(d) = decl_of(c, scope, slot)
                && is_scalar(d)
                && !out.iter().any(|v| v.0 == slot)
            {
                out.push((slot, d.ty));
            }
        }
        p += token_size(code, p);
    }
}

/// Scalar variables that an instruction run by the interpreter may read or
/// write: those of its tokens, plus the variables of the Def Fn statements
/// of its scope when it calls `Fn`, and of the Data statements of its scope
/// when it reads Data (their items are expressions).
pub fn fallback_vars(c: &Compiled, instrs: &[Instr], resident: &[(usize, u16)], idx: usize) -> Vec<(u16, u8)> {
    let ins = &instrs[idx];
    let code = &c.code;
    let mut out = Vec::new();
    scalar_vars_in(c, ins.scope, ins.pos, ins.end, &mut out);
    let mut uses_fn = false;
    let mut uses_read = false;
    let mut p = ins.pos;
    while p < ins.end {
        let t = rd(code, p);
        uses_fn |= t == tk::FN;
        uses_read |= t == tk::READ;
        p += token_size(code, p);
    }
    if uses_fn || uses_read {
        for other in instrs.iter().filter(|o| o.scope == ins.scope) {
            let t = rd(code, other.pos);
            if (uses_fn && t == tk::DEF_FN) || (uses_read && t == TK_DATA) {
                scalar_vars_in(c, ins.scope, other.pos, other.end, &mut out);
            }
        }
    }
    out.retain(|v| resident.binary_search(&var_key(ins.scope, v.0)).is_err());
    out
}

/// A constant Data item (what `Interp::eval` reads for it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lit {
    /// Nothing before the comma / end of line: the zero of the variable.
    Empty,
    Int(i32),
    /// Single precision constant (`TK_FL` bits).
    Ffp(u32),
    /// Double precision constant (`TK_DFL`, rounded at run time).
    Dfl(f64),
    /// String constant: position of its token.
    Str(usize),
}

/// The Data item at `p` if it is a constant (with any number of signs
/// before it: one negation, `Interp::operand`): (negated, value, position
/// after the item). `None` for an expression.
pub fn data_item(code: &[u8], p: usize) -> Option<(bool, Lit, usize)> {
    let t = rd(code, p);
    if t == TK_COMMA || t == TK_EOL {
        return Some((false, Lit::Empty, p));
    }
    let mut q = p;
    let mut neg = false;
    while rd(code, q) & 0x8000 != 0 {
        q += 2;
        neg = true;
    }
    let lit = match rd(code, q) {
        TK_ENT | TK_HEX | TK_BIN => Lit::Int(crate::program::read_u32(code, q + 2) as i32),
        TK_FL => Lit::Ffp(crate::program::read_u32(code, q + 2)),
        TK_DFL => {
            let mut b = [0u8; 8];
            b.copy_from_slice(&code[q + 2..q + 10]);
            Lit::Dfl(f64::from_be_bytes(b))
        }
        TK_CH1 | TK_CH2 => Lit::Str(q),
        _ => return None,
    };
    let end = q + token_size(code, q);
    matches!(rd(code, end), TK_COMMA | TK_EOL).then_some((neg, lit, end))
}

/// True if every item of the Data instruction at `p` is a constant.
pub fn data_is_constant(code: &[u8], p: usize) -> bool {
    let mut q = p + 2 + inline_data_size(TK_DATA);
    loop {
        let Some((_, _, end)) = data_item(code, q) else { return false };
        match rd(code, end) {
            TK_COMMA => q = end + 2,
            _ => return true,
        }
    }
}

/// Positions of the string constants (`TK_CH1` / `TK_CH2` tokens) of the
/// program's instructions, in order: a constant's index in this list is its
/// slot in the constant table of compiled programs.
pub fn string_constants(c: &Compiled) -> Vec<usize> {
    let code = &c.code;
    let mut out = Vec::new();
    for ins in instructions(c) {
        let mut p = ins.pos;
        while p < ins.end {
            let t = rd(code, p);
            if t == TK_CH1 || t == TK_CH2 {
                out.push(p);
            }
            p += token_size(code, p);
        }
    }
    out
}

/// FNV-1a hash of the verified code: a compiled module records the hash of
/// the program it was compiled from.
pub fn code_hash(code: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in code {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::verify::Verifier;

    fn compiled(src: &str) -> Compiled {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        Verifier::verify(&prg.source, prg.math_flags).unwrap()
    }

    #[test]
    fn instructions_follow_the_interpreter() {
        let c = compiled(
            "A=1 : B=2\nIf A=1 Then Print 1 : Print 2 Else Print 3\nL: Print 4\nTrap Print 1/0\nData 1,2 : Print 5",
        );
        let toks: Vec<u16> = instructions(&c).iter().map(|i| rd(&c.code, i.pos)).collect();
        use crate::tokens::tk::*;
        assert_eq!(toks, [TK_VAR, TK_VAR, TK_IF, PRINT, PRINT, TK_ELSE, PRINT, TK_LAB, PRINT, TRAP, PRINT, TK_DATA]);
        // Every instruction ends where the next one starts (after separators).
        let ins = instructions(&c);
        for w in ins.windows(2) {
            assert_eq!(normalize(&c.code, w[0].end), Some(w[1].pos));
        }
        assert_eq!(normalize(&c.code, ins.last().unwrap().end), None);
    }

    #[test]
    fn expression_skipping() {
        let c = compiled("A$=Mid$(\"abc\",1+2*3,Len(\"x\"))+Chr$(65) : B=-(1+2)*Not 3");
        for i in instructions(&c) {
            let q = i.pos + token_size(&c.code, i.pos) + 2;
            assert_eq!(skip_expr(&c.code, q), Some(i.end));
        }
    }

    #[test]
    fn resident_variables() {
        let c = compiled("A=1 : B=2\nP=Varptr(A)\nPrint B");
        let ins = instructions(&c);
        let r = resident_vars(&c, &ins);
        assert_eq!(r.len(), 1);
    }
}
