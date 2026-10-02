//! The test ("verification") pass run before a program starts.
//!
//! Like `PTest` in `+Verif.s` it checks the syntax and types of every
//! statement and patches the token stream so the interpreter can run it
//! quickly:
//!
//! * variable tokens get a slot reference in their offset word: a local slot
//!   index, or `0x8000 | index` for a global;
//! * label references (`TK_LGO`) get the index of the label in the label
//!   table of their scope, procedure calls (`TK_PRO`) the procedure number;
//! * loop and test tokens get the distance to their jump target in their
//!   inline data;
//! * overloaded keywords are rewritten to the variant matching their
//!   parameters.

use std::collections::HashMap;

use crate::program::{proc_flags, read_u16, read_u32, var_flags};
use crate::tokens::{self, tk, Keyword, TokenDef, TokenKind, *};

/// Test-time error (number in the test message table) and its position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestError {
    pub code: u16,
    pub pos: usize,
}

pub mod terr {
    pub const ALONE_ON_LINE: u16 = 4;
    pub const EXTENSION_NOT_LOADED: u16 = 5;
    pub const ILLEGAL_DIRECT: u16 = 7;
    pub const STRUCTURE_TOO_LONG: u16 = 10;
    pub const EMPTY_BRACKETS: u16 = 14;
    pub const SHARED_ALONE: u16 = 15;
    pub const PROC_LIMITS_ALONE: u16 = 16;
    pub const PROC_NOT_CLOSED: u16 = 17;
    pub const PROC_NOT_OPENED: u16 = 18;
    pub const ILLEGAL_NB_PARAMS: u16 = 19;
    pub const UNDEFINED_PROC: u16 = 20;
    pub const ELSE_WITHOUT_IF: u16 = 21;
    pub const IF_WITHOUT_ENDIF: u16 = 22;
    pub const ENDIF_WITHOUT_IF: u16 = 23;
    pub const NO_THEN: u16 = 25;
    pub const NOT_ENOUGH_LOOPS: u16 = 26;
    pub const DO_WITHOUT_LOOP: u16 = 27;
    pub const LOOP_WITHOUT_DO: u16 = 28;
    pub const WHILE_WITHOUT_WEND: u16 = 29;
    pub const WEND_WITHOUT_WHILE: u16 = 30;
    pub const REPEAT_WITHOUT_UNTIL: u16 = 31;
    pub const UNTIL_WITHOUT_REPEAT: u16 = 32;
    pub const FOR_WITHOUT_NEXT: u16 = 33;
    pub const NEXT_WITHOUT_FOR: u16 = 34;
    pub const SYNTAX: u16 = 35;
    pub const ARRAY_NOT_DIMENSIONED: u16 = 38;
    pub const ARRAY_ALREADY_DIMENSIONED: u16 = 39;
    pub const TYPE_MISMATCH: u16 = 40;
    pub const UNDEFINED_LABEL: u16 = 41;
    pub const LABEL_TWICE: u16 = 42;
    pub const TRAP_FOLLOWED: u16 = 43;
    pub const USER_FN_NOT_DEFINED: u16 = 2;
    pub const MUST_BEGIN: u16 = 50;
}

type VResult<T> = Result<T, TestError>;

/// Declaration of a variable slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarDecl {
    pub name: Vec<u8>,
    /// 0 integer, 1 float, 2 string.
    pub ty: u8,
    pub array: bool,
    pub func: bool,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub name: Vec<u8>,
    /// Position of the first token after the label (or of the first token of
    /// the next line when the label is alone on its line).
    pub target: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub labels: Vec<Label>,
    pub by_name: HashMap<Vec<u8>, usize>,
}

#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub name: Vec<u8>,
    /// Position of the `Procedure` token.
    pub pos: usize,
    /// Start of the `End Proc` line, and position of its token.
    pub end_line: usize,
    pub end_pos: usize,
    /// First token of the body (line after the Procedure line).
    pub body: usize,
    /// Slot references of the parameters, and their types.
    pub params: Vec<u16>,
    pub param_types: Vec<u8>,
    pub locals: Vec<VarDecl>,
    pub machine_code: bool,
}

/// A verified program ready to run.
#[derive(Clone, Debug, Default)]
pub struct Compiled {
    /// Patched tokens, terminated by a zero line header.
    pub code: Vec<u8>,
    pub globals: Vec<VarDecl>,
    pub procs: Vec<ProcInfo>,
    /// Scope 0 is the main program, scope `i + 1` procedure `i`.
    pub scopes: Vec<Scope>,
    /// Exit target of `Else` / `Else If` tokens reached in sequence.
    pub else_exit: HashMap<usize, usize>,
    pub double: bool,
    pub stack_size: usize,
    pub buffer_size: usize,
}

pub const GLOBAL: u16 = 0x8000;

/// Size in bytes of the token at `p`, including its inline data.
pub fn token_size(code: &[u8], p: usize) -> usize {
    let t = read_u16(code, p);
    match t {
        TK_VAR | TK_LAB | TK_PRO | TK_LGO => 6 + code[p + 4] as usize,
        TK_CH1 | TK_CH2 | TK_REM1 | TK_REM2 => {
            let n = read_u16(code, p + 2) as usize;
            4 + n + (n & 1)
        }
        TK_ENT | TK_HEX | TK_BIN | TK_FL => 6,
        TK_DFL => 10,
        TK_EXT => 6,
        _ => 2 + inline_data_size(t),
    }
}

/// Type of an expression as seen by the verifier (only numbers and strings
/// are distinguished).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Num,
    Str,
}

fn ty_of(t: u8) -> Ty {
    if t == 2 { Ty::Str } else { Ty::Num }
}

/// Parameter list parsed from the program, compared with signatures.
#[derive(Default, Debug)]
struct Args {
    types: Vec<Ty>,
    /// Separator before each parameter after the first: b',' or b't'.
    seps: Vec<u8>,
}

fn signature_matches(sig: &str, args: &Args) -> Result<bool, ()> {
    let s = sig.as_bytes();
    // Signature: type chars separated by ',' or 't'.
    let mut types = Vec::new();
    let mut seps = Vec::new();
    for (i, &c) in s.iter().enumerate() {
        if i % 2 == 0 {
            types.push(c);
        } else {
            seps.push(c);
        }
    }
    if types.len() != args.types.len() || seps != args.seps {
        return Ok(false);
    }
    for (&want, &got) in types.iter().zip(&args.types) {
        let ok = match want {
            b'3' => true,
            b'2' => got == Ty::Str,
            _ => got == Ty::Num,
        };
        if !ok {
            return Err(());
        }
    }
    Ok(true)
}

/// Open loop structure.
struct LoopRec {
    token: u16,
    /// Position of the opener's inline field.
    field: usize,
    /// For: key of the control variable.
    var: Option<u16>,
    /// Pending `Exit` fields to patch with the exit target: (field, frames).
    exits: Vec<(usize, u16)>,
    pos: usize,
}

/// Open structured If.
struct IfRec {
    /// Field to patch with the next branch target (If / Else If false jump).
    pending_false: Option<usize>,
    /// Else / Else If tokens whose sequential exit is the End If.
    exits: Vec<usize>,
    /// Else / Else If fields for the sequential exit (Else field).
    else_fields: Vec<usize>,
    seen_else: bool,
    pos: usize,
}

struct Phase {
    /// 0 = main program, n = procedure n-1.
    index: usize,
    locals: Vec<VarDecl>,
    local_map: HashMap<(Vec<u8>, u8), u16>,
    loops: Vec<LoopRec>,
    ifs: Vec<IfRec>,
    /// Label references to resolve at the end of the phase: (token pos).
    label_refs: Vec<usize>,
    /// Procedure calls to resolve once all procedures are known: (token pos).
    proc_calls: Vec<usize>,
    dimmed: Vec<(Vec<u8>, u8)>,
}

pub struct Verifier {
    code: Vec<u8>,
    globals: Vec<VarDecl>,
    global_map: HashMap<(Vec<u8>, u8), u16>,
    /// Visibility of each global in the current procedure: 0 hidden,
    /// 1 shared in this procedure, 2 declared `Global`.
    global_vis: Vec<u8>,
    procs: Vec<ProcInfo>,
    proc_by_name: HashMap<Vec<u8>, usize>,
    /// Procedure calls of all phases, resolved once every procedure is known.
    proc_calls: Vec<usize>,
    scopes: Vec<Scope>,
    else_exit: HashMap<usize, usize>,
    double: bool,
    stack_size: usize,
    buffer_size: usize,
    program_started: bool,
    direct: bool,
}

/// Key used to find a variable: name + (type | array<<4 | fn<<5).
fn var_key(name: &[u8], ty: u8, array: bool, func: bool) -> (Vec<u8>, u8) {
    (name.to_vec(), ty | (array as u8) << 4 | (func as u8) << 5)
}

fn is_stmt_end(t: u16) -> bool {
    t == TK_EOL || t == TK_DP || t == TK_ELSE
}

impl Verifier {
    /// Verifies a whole program.
    pub fn verify(source: &[u8], math_flags: u8) -> Result<Compiled, TestError> {
        let mut code = source.to_vec();
        code.extend_from_slice(&[0, 0]);
        let mut v = Verifier {
            code,
            globals: Vec::new(),
            global_map: HashMap::new(),
            global_vis: Vec::new(),
            procs: Vec::new(),
            proc_by_name: HashMap::new(),
            proc_calls: Vec::new(),
            scopes: vec![Scope::default()],
            else_exit: HashMap::new(),
            double: math_flags & 0x80 != 0,
            stack_size: 51,
            buffer_size: 8 * 1024,
            program_started: false,
            direct: false,
        };
        v.find_procedures()?;
        v.collect_labels()?;
        // Phase 0: main program.
        let mut phase = v.new_phase(0);
        v.verify_main(&mut phase)?;
        v.finish_phase(&mut phase)?;
        // Then each procedure.
        for i in 0..v.procs.len() {
            if v.procs[i].machine_code {
                // Only the parameter list can be checked.
                let mut phase = v.new_phase(i + 1);
                v.proc_params(&mut phase, i)?;
                v.procs[i].locals = phase.locals;
                continue;
            }
            for vis in v.global_vis.iter_mut() {
                if *vis != 2 {
                    *vis = 0;
                }
            }
            let mut phase = v.new_phase(i + 1);
            v.verify_procedure(&mut phase, i)?;
            v.finish_phase(&mut phase)?;
            let n = phase.locals.len() as u16;
            let pos = v.procs[i].pos;
            v.code[pos + 6..pos + 8].copy_from_slice(&n.to_be_bytes());
            v.procs[i].locals = phase.locals;
        }
        v.resolve_proc_calls()?;
        Ok(Compiled {
            code: v.code,
            globals: v.globals,
            procs: v.procs,
            scopes: v.scopes,
            else_exit: v.else_exit,
            double: v.double,
            stack_size: v.stack_size,
            buffer_size: v.buffer_size,
        })
    }

    /// Verifies one direct mode line in the context of a compiled program
    /// (all main program variables visible). `line` is a tokenised line.
    pub fn verify_direct(compiled: &Compiled, line: &[u8]) -> Result<(Vec<u8>, Vec<VarDecl>), TestError> {
        let mut code = line.to_vec();
        code.extend_from_slice(&[0, 0]);
        let mut v = Verifier {
            code,
            globals: compiled.globals.clone(),
            global_map: HashMap::new(),
            global_vis: vec![1; compiled.globals.len()],
            procs: compiled.procs.clone(),
            proc_by_name: HashMap::new(),
            proc_calls: Vec::new(),
            scopes: vec![Scope::default()],
            else_exit: HashMap::new(),
            double: compiled.double,
            stack_size: compiled.stack_size,
            buffer_size: compiled.buffer_size,
            program_started: true,
            direct: true,
        };
        for (i, g) in v.globals.iter().enumerate() {
            v.global_map.insert(var_key(&g.name, g.ty, g.array, g.func), i as u16);
        }
        for (i, p) in v.procs.iter().enumerate() {
            v.proc_by_name.insert(p.name.clone(), i);
        }
        let mut phase = v.new_phase(0);
        let mut p = 2;
        loop {
            let t = read_u16(&v.code, p);
            if t == TK_EOL {
                break;
            }
            if t == TK_DP {
                p += 2;
                continue;
            }
            p = v.statement(&mut phase, p, 0)?;
        }
        if let Some(l) = phase.loops.first() {
            return Err(TestError { code: open_loop_error(l.token), pos: l.pos });
        }
        v.resolve_label_refs(&mut phase)?;
        v.proc_calls.append(&mut phase.proc_calls);
        v.resolve_proc_calls()?;
        Ok((v.code, v.globals))
    }

    fn err<T>(&self, code: u16, pos: usize) -> VResult<T> {
        Err(TestError { code, pos })
    }

    fn rd(&self, p: usize) -> u16 {
        read_u16(&self.code, p)
    }

    fn new_phase(&self, index: usize) -> Phase {
        Phase {
            index,
            locals: Vec::new(),
            local_map: HashMap::new(),
            loops: Vec::new(),
            ifs: Vec::new(),
            label_refs: Vec::new(),
            proc_calls: Vec::new(),
            dimmed: Vec::new(),
        }
    }

    // ------------------------------------------------------------------
    // Program structure
    // ------------------------------------------------------------------

    /// Iterates over line starts from `from`, stopping at the end of program.
    fn next_line(&self, line: usize) -> usize {
        line + self.code[line] as usize * 2
    }

    fn find_procedures(&mut self) -> VResult<()> {
        let mut line = 0;
        while self.code[line] != 0 {
            let t = self.rd(line + 2);
            if t == TK_END_PROC {
                return self.err(terr::PROC_NOT_OPENED, line + 2);
            }
            if t == TK_PROCEDURE {
                let pos = line + 2;
                let flags = self.code[pos + 8];
                let machine_code = flags & proc_flags::MACHINE_CODE != 0;
                if self.rd(pos + 10) != TK_VAR {
                    return self.err(terr::SYNTAX, pos);
                }
                let name_len = self.code[pos + 14] as usize;
                let name = trim_name(&self.code[pos + 16..pos + 16 + name_len]);
                let (end_line, end_pos) = if machine_code {
                    let size = read_u32(&self.code, pos + 2) as usize;
                    let el = line + 8 + size;
                    (el, el + 2)
                } else {
                    let mut l = self.next_line(line);
                    loop {
                        if self.code[l] == 0 {
                            return self.err(terr::PROC_NOT_CLOSED, pos);
                        }
                        let t = self.rd(l + 2);
                        if t == TK_PROCEDURE {
                            return self.err(terr::PROC_NOT_CLOSED, pos);
                        }
                        if t == TK_END_PROC {
                            break;
                        }
                        l = self.next_line(l);
                    }
                    (l, l + 2)
                };
                // Store the distance to End Proc as the original does.
                let size = (end_line - line - 8) as u32;
                self.code[pos + 2..pos + 6].copy_from_slice(&size.to_be_bytes());
                let body = self.next_line(line) + 2;
                let index = self.procs.len();
                if self.proc_by_name.insert(name.clone(), index).is_some() {
                    return self.err(terr::LABEL_TWICE, pos);
                }
                self.procs.push(ProcInfo {
                    name,
                    pos,
                    end_line,
                    end_pos,
                    body,
                    params: Vec::new(),
                    param_types: Vec::new(),
                    locals: Vec::new(),
                    machine_code,
                });
                self.scopes.push(Scope::default());
                line = self.next_line(end_line);
                continue;
            }
            line = self.next_line(line);
        }
        Ok(())
    }

    /// Scope index of a line position.
    fn scope_of(&self, pos: usize) -> usize {
        for (i, p) in self.procs.iter().enumerate() {
            if pos > p.pos && pos < p.end_pos {
                return i + 1;
            }
        }
        0
    }

    /// First token position of the statement following position `p`, where
    /// `p` points at a `:` or end-of-line token, or anywhere a statement
    /// starts.
    fn statement_start(&self, mut p: usize) -> usize {
        loop {
            let t = self.rd(p);
            if t == TK_DP {
                p += 2;
                continue;
            }
            if t == TK_EOL {
                // Is there a next line?
                let next = p + 2;
                if self.code[next] != 0 {
                    p = next + 2;
                    continue;
                }
                return p;
            }
            return p;
        }
    }

    fn collect_labels(&mut self) -> VResult<()> {
        let mut line = 0;
        while self.code[line] != 0 {
            let p = line + 2;
            if let Some(pr) = self.procs.iter().find(|pr| pr.pos == p && pr.machine_code) {
                line = self.next_line(pr.end_line);
                continue;
            }
            if self.rd(p) == TK_LAB {
                let len = self.code[p + 4] as usize;
                let name = trim_name(&self.code[p + 6..p + 6 + len]);
                let after = p + 6 + len;
                let target = if self.rd(after) == TK_EOL {
                    let next = self.next_line(line);
                    if self.code[next] != 0 { next + 2 } else { after }
                } else {
                    after
                };
                let scope = self.scope_of(p);
                let s = &mut self.scopes[scope];
                if s.by_name.contains_key(&name) {
                    return self.err(terr::LABEL_TWICE, p);
                }
                s.by_name.insert(name.clone(), s.labels.len());
                s.labels.push(Label { name, target });
            }
            line = self.next_line(line);
        }
        // Procedures are also known as labels of the main program
        // (On ... Proc, Every ... Proc).
        Ok(())
    }

    fn verify_main(&mut self, ph: &mut Phase) -> VResult<()> {
        let mut line = 0;
        while self.code[line] != 0 {
            if let Some(i) = self.procs.iter().position(|pr| pr.pos == line + 2) {
                // Procedure parameters must exist; Shared / Global lines inside
                // procedures create their variables in the main program.
                let (el, mc) = (self.procs[i].end_line, self.procs[i].machine_code);
                if !mc {
                    let mut l = self.next_line(line);
                    while l < el {
                        let t = self.rd(l + 2);
                        if t == tk::SHARED || t == tk::GLOBAL {
                            self.shared_global(ph, l + 2, true)?;
                        }
                        l = self.next_line(l);
                    }
                }
                line = self.next_line(el);
                continue;
            }
            self.verify_line(ph, line)?;
            line = self.next_line(line);
        }
        Ok(())
    }

    fn verify_procedure(&mut self, ph: &mut Phase, index: usize) -> VResult<()> {
        self.proc_params(ph, index)?;
        let pos = self.procs[index].pos;
        let mut line = self.next_line(pos - 2);
        let end_line = self.procs[index].end_line;
        while line < end_line {
            self.verify_line(ph, line)?;
            line = self.next_line(line);
        }
        // End Proc [expression]
        let mut p = self.procs[index].end_pos + 2;
        if self.rd(p) == TK_BRA1 {
            let (np, _) = self.expr(ph, p + 2)?;
            p = np;
            if self.rd(p) != TK_BRA2 {
                return self.err(terr::SYNTAX, p);
            }
            p += 2;
        }
        if self.rd(p) != TK_EOL {
            return self.err(terr::PROC_LIMITS_ALONE, p);
        }
        Ok(())
    }

    fn proc_params(&mut self, ph: &mut Phase, index: usize) -> VResult<()> {
        let pos = self.procs[index].pos;
        // Parameters: the first local variables.
        let mut p = pos + 10;
        p += token_size(&self.code, p);
        let mut params = Vec::new();
        let mut types = Vec::new();
        if self.rd(p) == TK_BRA1 {
            p += 2;
            loop {
                if self.rd(p) != TK_VAR {
                    return self.err(terr::SYNTAX, p);
                }
                let (slot, ty) = self.declare_local(ph, p)?;
                params.push(slot);
                types.push(ty);
                p += token_size(&self.code, p);
                match self.rd(p) {
                    TK_COMMA => p += 2,
                    TK_BRA2 => {
                        p += 2;
                        break;
                    }
                    _ => return self.err(terr::SYNTAX, p),
                }
            }
        }
        if self.rd(p) != TK_EOL {
            return self.err(terr::PROC_LIMITS_ALONE, p);
        }
        self.procs[index].params = params;
        self.procs[index].param_types = types;
        Ok(())
    }

    fn finish_phase(&mut self, ph: &mut Phase) -> VResult<()> {
        if let Some(l) = ph.loops.first() {
            return self.err(open_loop_error(l.token), l.pos);
        }
        if let Some(i) = ph.ifs.first() {
            return self.err(terr::IF_WITHOUT_ENDIF, i.pos);
        }
        self.resolve_label_refs(ph)?;
        self.proc_calls.append(&mut ph.proc_calls);
        Ok(())
    }

    fn resolve_label_refs(&mut self, ph: &mut Phase) -> VResult<()> {
        let scope = ph.index;
        for &p in &ph.label_refs {
            let len = self.code[p + 4] as usize;
            let name = trim_name(&self.code[p + 6..p + 6 + len]);
            let Some(&idx) = self.scopes[scope].by_name.get(&name) else {
                return self.err(terr::UNDEFINED_LABEL, p);
            };
            self.code[p + 2..p + 4].copy_from_slice(&(idx as u16).to_be_bytes());
        }
        Ok(())
    }

    fn resolve_proc_calls(&mut self) -> VResult<()> {
        for p in std::mem::take(&mut self.proc_calls) {
            let len = self.code[p + 4] as usize;
            let name = trim_name(&self.code[p + 6..p + 6 + len]);
            let Some(&idx) = self.proc_by_name.get(&name) else {
                return self.err(terr::UNDEFINED_PROC, p);
            };
            self.code[p..p + 2].copy_from_slice(&TK_PRO.to_be_bytes());
            self.code[p + 2..p + 4].copy_from_slice(&(idx as u16).to_be_bytes());
            self.code[p + 5] |= var_flags::PROC;
            // Check the parameters.
            let mut q = p + token_size(&self.code, p);
            let want = &self.procs[idx].param_types;
            let mut n = 0;
            if self.rd(q) == TK_BRA1 {
                q += 2;
                loop {
                    // Argument types were recorded when the call was parsed;
                    // only the count is checked here.
                    n += 1;
                    q = self.skip_expr(q);
                    match self.rd(q) {
                        TK_COMMA => q += 2,
                        _ => break,
                    }
                }
            }
            if n != want.len() {
                return self.err(terr::ILLEGAL_NB_PARAMS, p);
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Lines and statements
    // ------------------------------------------------------------------

    fn verify_line(&mut self, ph: &mut Phase, line: usize) -> VResult<()> {
        let mut p = line + 2;
        let mut first = true;
        loop {
            let t = self.rd(p);
            if t == TK_EOL {
                break;
            }
            if t == TK_DP {
                p += 2;
                continue;
            }
            // Line start only statements.
            if !first && is_line_start_only(t) {
                let code = match t {
                    tk::SHARED | tk::GLOBAL => terr::SHARED_ALONE,
                    TK_PROCEDURE | TK_END_PROC => terr::PROC_LIMITS_ALONE,
                    _ => terr::ALONE_ON_LINE,
                };
                // Data may follow a label.
                if !(t == TK_DATA && self.rd(line + 2) == TK_LAB) {
                    return self.err(code, p);
                }
            }
            p = self.statement(ph, p, line)?;
            first = false;
        }
        // Single line Ifs end with the line.
        Ok(())
    }

    /// Verifies the statement at `p`; returns the position after it.
    fn statement(&mut self, ph: &mut Phase, p: usize, line: usize) -> VResult<usize> {
        let t = self.rd(p);
        if t & 0x8000 != 0 {
            return self.err(terr::SYNTAX, p);
        }
        if t != TK_REM1 && t != TK_REM2 && t != tk::SET_BUFFER && t != tk::SET_STACK {
            if t == tk::SET_DOUBLE_PRECISION && self.program_started {
                return self.err(terr::MUST_BEGIN, p);
            }
            if t != TK_LAB {
                self.program_started = true;
            }
        }
        let end = match t {
            TK_VAR => self.st_variable(ph, p)?,
            // A label is followed directly by the first statement of the line.
            TK_LAB => return Ok(p + token_size(&self.code, p)),
            TK_PRO => self.st_proc_call(ph, p)?,
            TK_LGO => return self.err(terr::SYNTAX, p),
            TK_REM1 | TK_REM2 => {
                let next = p + token_size(&self.code, p);
                if self.rd(next) != TK_EOL {
                    return self.err(terr::SYNTAX, next);
                }
                next
            }
            TK_EXT => self.st_generic(ph, p)?,
            TK_PROCEDURE => return self.err(terr::SYNTAX, p),
            TK_END_PROC => return self.err(terr::PROC_NOT_OPENED, p),
            _ => {
                let Some(def) = tokens::lookup(t) else { return self.err(terr::SYNTAX, p) };
                self.st_keyword(ph, p, def, line)?
            }
        };
        // If ... Then, Else and Else If ... Then are followed by a statement.
        if matches!(t, TK_ELSE | TK_ELSE_IF) || (t == TK_IF && self.rd(end - 2) == tk::THEN) {
            return Ok(end);
        }
        let after = self.rd(end);
        if !is_stmt_end(after) {
            return self.err(terr::SYNTAX, end);
        }
        Ok(end)
    }

    fn st_keyword(&mut self, ph: &mut Phase, p: usize, def: &'static TokenDef, line: usize) -> VResult<usize> {
        let class = def.verif[0];
        let t = def.token;
        let q = p + 2 + inline_data_size(t);
        Ok(match class {
            0x01 | 0x52 | 0x57 | 0x55 => return self.err(terr::SYNTAX, p),
            0x03 => {
                // Set Buffer n (constant)
                let (np, v) = self.const_int(q)?;
                self.buffer_size = (v.max(1) as usize) * 1024;
                np
            }
            0x04 => {
                self.double = true;
                q
            }
            0x05 => {
                let (np, v) = self.const_int(q)?;
                self.stack_size = v.max(10) as usize + 1;
                np
            }
            0x09 => self.st_dim(ph, q)?,
            0x0A | 0x48 => self.st_print(ph, q)?,
            0x0B => {
                // Print #n, ...
                let (np, ty) = self.expr(ph, q)?;
                self.want(ty, Ty::Num, q)?;
                if self.rd(np) == TK_COMMA || self.rd(np) == TK_SEMI {
                    self.st_print(ph, np + 2)?
                } else {
                    np
                }
            }
            0x0C | 0x49 => self.st_input(ph, q, false)?,
            0x0D | 0x4A => self.st_input(ph, q, true)?,
            0x0E | 0x16 => {
                let (np, ty) = self.lvalue(ph, q)?;
                self.want(ty, Ty::Num, q)?;
                np
            }
            0x17 | 0x4F => self.st_add(ph, p)?,
            0x0F => {
                // Proc name[params]
                match self.rd(q) {
                    TK_VAR | TK_PRO => self.st_proc_call(ph, q)?,
                    _ => return self.err(terr::UNDEFINED_PROC, q),
                }
            }
            0x11 | 0x12 => self.expr_list(ph, q, None)?,
            0x13 => {
                // Read v1, v2...
                let mut r = q;
                loop {
                    let (np, _) = self.lvalue(ph, r)?;
                    r = np;
                    if self.rd(r) == TK_COMMA {
                        r += 2;
                    } else {
                        break r;
                    }
                }
            }
            0x14 => {
                // Restore [label]
                if is_stmt_end(self.rd(q)) { q } else { self.label_operand(ph, q)? }
            }
            0x18 => self.st_polyline(ph, p)?,
            0x19 => self.st_field(ph, q)?,
            0x1B => self.st_menu_assign(ph, q)?,
            0x1C | 0x1D | 0x1E | 0x1F => self.st_loose(ph, q)?,
            0x20 | 0x15 | 0x1A | 0x23 | 0x24 | 0x50 | 0x51 | 0x56 | 0x2A => {
                let saved = self.code.clone();
                match self.st_generic(ph, p) {
                    Ok(np) => np,
                    Err(_) => {
                        self.code = saved;
                        self.st_loose_no_paren(ph, q)?
                    }
                }
            }
            0x21 => {
                // Sort a(0)
                let (np, _) = self.lvalue(ph, q)?;
                np
            }
            0x22 => {
                let (np, t1) = self.lvalue(ph, q)?;
                if self.rd(np) != TK_COMMA {
                    return self.err(terr::SYNTAX, np);
                }
                let (np2, t2) = self.lvalue(ph, np + 2)?;
                if t1 != t2 {
                    return self.err(terr::TYPE_MISMATCH, np + 2);
                }
                np2
            }
            0x25 => {
                // Trap instruction
                let next = self.rd(q);
                if is_stmt_end(next) {
                    return self.err(terr::TRAP_FOLLOWED, q);
                }
                return self.statement(ph, q, line);
            }
            0x26 | 0x27 => {
                // Struc(address,"field")=value
                if self.rd(q) != TK_PAR1 {
                    return self.err(terr::SYNTAX, q);
                }
                let (np, _) = self.arg_list(ph, q + 2, true)?;
                if self.rd(np) != tk::OP_EQ {
                    return self.err(terr::SYNTAX, np);
                }
                let (np, ty) = self.expr(ph, np + 2)?;
                self.want(ty, if class == 0x27 { Ty::Str } else { Ty::Num }, np)?;
                np
            }
            0x2B | 0x2C => self.st_reserved_assign(ph, p, def)?,
            0x2F => p,
            0x30 => self.st_for(ph, p)?,
            0x31 => self.st_next(ph, p)?,
            0x32 | 0x36 => {
                ph.loops.push(LoopRec { token: t, field: p + 2, var: None, exits: Vec::new(), pos: p });
                q
            }
            0x33 => {
                let (np, ty) = self.expr(ph, q)?;
                self.want(ty, Ty::Num, q)?;
                self.close_loop(ph, TK_REPEAT, p, np, terr::UNTIL_WITHOUT_REPEAT)?;
                np
            }
            0x34 => {
                ph.loops.push(LoopRec { token: t, field: p + 2, var: None, exits: Vec::new(), pos: p });
                let (np, ty) = self.expr(ph, q)?;
                self.want(ty, Ty::Num, q)?;
                np
            }
            0x35 => {
                self.close_loop(ph, TK_WHILE, p, q, terr::WEND_WITHOUT_WHILE)?;
                q
            }
            0x37 => {
                self.close_loop(ph, TK_DO, p, q, terr::LOOP_WITHOUT_DO)?;
                q
            }
            0x38 | 0x39 => self.st_exit(ph, p)?,
            0x3A => self.st_if(ph, p, line)?,
            0x3B => self.st_else(ph, p)?,
            0x3C => self.st_else_if(ph, p)?,
            0x3D => self.st_end_if(ph, p)?,
            0x3E | 0x3F => self.label_operand(ph, q)?,
            0x40 => self.st_on_error(ph, q)?,
            0x41 => self.proc_operand(ph, q)?,
            0x42 | 0x43 => self.st_on(ph, p, class == 0x42)?,
            0x44 => {
                // Resume [Next | label]
                match self.rd(q) {
                    t2 if is_stmt_end(t2) => q,
                    _ => self.label_operand(ph, q)?,
                }
            }
            0x45 => {
                if is_stmt_end(self.rd(q)) { q } else { self.label_operand(ph, q)? }
            }
            0x46 => {
                // Pop Proc [ [expr] ]
                if ph.index == 0 && !self.direct {
                    return self.err(terr::PROC_NOT_OPENED, p);
                }
                if self.rd(q) == TK_BRA1 {
                    let (np, _) = self.expr(ph, q + 2)?;
                    if self.rd(np) != TK_BRA2 {
                        return self.err(terr::SYNTAX, np);
                    }
                    np + 2
                } else {
                    q
                }
            }
            0x47 => {
                // Every n Gosub|Proc label
                let (np, ty) = self.expr(ph, q)?;
                self.want(ty, Ty::Num, q)?;
                match self.rd(np) {
                    tk::GOSUB => self.label_operand(ph, np + 2)?,
                    tk::PROC => self.proc_operand(ph, np + 2)?,
                    _ => return self.err(terr::SYNTAX, np),
                }
            }
            0x4B..=0x4E => self.st_mid_assign(ph, p, def)?,
            0x53 | 0x54 => q,
            0xFB | 0xFA => {
                if ph.index == 0 && t == tk::SHARED {
                    // Shared in the main program: just declares.
                }
                self.shared_global(ph, p, false)?
            }
            0xFC => self.st_def_fn(ph, p)?,
            0xFD => self.st_data(ph, p, line)?,
            0xFE => return self.err(terr::PROC_NOT_OPENED, p),
            0xFF => return self.err(terr::SYNTAX, p),
            _ => self.st_generic(ph, p)?,
        })
    }

    fn want(&self, got: Ty, want: Ty, pos: usize) -> VResult<()> {
        if got != want { self.err(terr::TYPE_MISMATCH, pos) } else { Ok(()) }
    }

    fn const_int(&self, p: usize) -> VResult<(usize, i32)> {
        match self.rd(p) {
            TK_ENT | TK_HEX | TK_BIN => Ok((p + 6, read_u32(&self.code, p + 2) as i32)),
            _ => self.err(terr::SYNTAX, p),
        }
    }

    // ------------------------------------------------------------------
    // Variables
    // ------------------------------------------------------------------

    /// Finds or creates the variable whose token is at `p`, patches its slot
    /// reference and returns `(slot, type)`.
    fn resolve_var(&mut self, ph: &mut Phase, p: usize, array: bool, func: bool) -> VResult<(u16, u8)> {
        let len = self.code[p + 4] as usize;
        let flags = self.code[p + 5];
        let ty = flags & var_flags::TYPE_MASK;
        let name = trim_name(&self.code[p + 6..p + 6 + len]);
        let key = var_key(&name, ty, array, func);
        let slot = if ph.index == 0 {
            self.global_slot(&key, &name, ty, array, func)
        } else if let Some(&g) = self.global_map.get(&key)
            && self.global_vis[g as usize] != 0
        {
            GLOBAL | g
        } else if let Some(&l) = ph.local_map.get(&key) {
            l
        } else {
            let l = ph.locals.len() as u16;
            ph.locals.push(VarDecl { name: name.clone(), ty, array, func });
            ph.local_map.insert(key, l);
            l
        };
        let mut f = flags & 0x07;
        if array {
            f |= var_flags::ARRAY;
        }
        if func {
            f |= var_flags::DEF_FN;
        }
        self.code[p + 2..p + 4].copy_from_slice(&slot.to_be_bytes());
        self.code[p + 5] = f;
        Ok((slot, ty))
    }

    fn global_slot(&mut self, key: &(Vec<u8>, u8), name: &[u8], ty: u8, array: bool, func: bool) -> u16 {
        if let Some(&g) = self.global_map.get(key) {
            return GLOBAL | g;
        }
        let g = self.globals.len() as u16;
        self.globals.push(VarDecl { name: name.to_vec(), ty, array, func });
        self.global_vis.push(0);
        self.global_map.insert(key.clone(), g);
        GLOBAL | g
    }

    fn declare_local(&mut self, ph: &mut Phase, p: usize) -> VResult<(u16, u8)> {
        self.resolve_var(ph, p, false, false)
    }

    /// Variable or array element used as a value or target. Returns the
    /// position after it and its type.
    fn var_ref(&mut self, ph: &mut Phase, p: usize) -> VResult<(usize, Ty)> {
        let after = p + token_size(&self.code, p);
        if self.rd(after) == TK_PAR1 {
            let (_, ty) = self.resolve_var(ph, p, true, false)?;
            let key = self.var_key_at(p, true);
            if !self.array_known(ph, &key) {
                return self.err(terr::ARRAY_NOT_DIMENSIONED, p);
            }
            let mut q = after + 2;
            loop {
                let (np, ity) = self.expr(ph, q)?;
                self.want(ity, Ty::Num, q)?;
                q = np;
                match self.rd(q) {
                    TK_COMMA => q += 2,
                    TK_PAR2 => break,
                    _ => return self.err(terr::SYNTAX, q),
                }
            }
            Ok((q + 2, ty_of(ty)))
        } else {
            let (_, ty) = self.resolve_var(ph, p, false, false)?;
            Ok((after, ty_of(ty)))
        }
    }

    fn var_key_at(&self, p: usize, array: bool) -> (Vec<u8>, u8) {
        let len = self.code[p + 4] as usize;
        let ty = self.code[p + 5] & var_flags::TYPE_MASK;
        var_key(&trim_name(&self.code[p + 6..p + 6 + len]), ty, array, false)
    }

    fn array_known(&self, ph: &Phase, key: &(Vec<u8>, u8)) -> bool {
        if ph.dimmed.contains(key) {
            return true;
        }
        // Arrays of the main program used in procedures (Shared / Global),
        // or arrays dimensioned anywhere in the main program.
        if let Some(&g) = self.global_map.get(key) {
            return ph.index == 0 || self.global_vis[g as usize] != 0 || self.direct;
        }
        false
    }

    /// A variable that is assigned (or read into).
    fn lvalue(&mut self, ph: &mut Phase, p: usize) -> VResult<(usize, Ty)> {
        if self.rd(p) != TK_VAR {
            return self.err(terr::SYNTAX, p);
        }
        self.var_ref(ph, p)
    }

    fn st_variable(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let after = p + token_size(&self.code, p);
        let next = self.rd(after);
        if next == TK_PAR1 || next == tokens::tk::OP_EQ {
            let (np, ty) = self.var_ref(ph, p)?;
            if self.rd(np) != tk::OP_EQ {
                return self.err(terr::SYNTAX, np);
            }
            let (end, ety) = self.expr(ph, np + 2)?;
            self.want(ety, ty, np + 2)?;
            return Ok(end);
        }
        // A procedure call without Proc.
        self.st_proc_call(ph, p)
    }

    fn st_proc_call(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        if self.direct && false {
            return self.err(terr::ILLEGAL_DIRECT, p);
        }
        ph.proc_calls.push(p);
        let mut q = p + token_size(&self.code, p);
        if self.rd(q) == TK_BRA1 {
            q += 2;
            loop {
                let (np, _) = self.expr(ph, q)?;
                q = np;
                match self.rd(q) {
                    TK_COMMA => q += 2,
                    TK_BRA2 => {
                        q += 2;
                        break;
                    }
                    _ => return self.err(terr::SYNTAX, q),
                }
            }
        }
        Ok(q)
    }

    fn proc_operand(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        match self.rd(p) {
            TK_VAR | TK_PRO => {
                ph.proc_calls.push(p);
                Ok(p + token_size(&self.code, p))
            }
            _ => self.err(terr::UNDEFINED_PROC, p),
        }
    }

    /// A label operand (Goto, Gosub, Restore...): a label name or line number
    /// becomes `TK_LGO`, anything else is an expression evaluated at run time.
    fn label_operand(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let t = self.rd(p);
        if t == TK_LGO || t == TK_LAB {
            self.code[p..p + 2].copy_from_slice(&TK_LGO.to_be_bytes());
            ph.label_refs.push(p);
            return Ok(p + token_size(&self.code, p));
        }
        if t == TK_VAR && self.code[p + 5] & var_flags::TYPE_MASK == 0 {
            let after = p + token_size(&self.code, p);
            let n = self.rd(after);
            if is_stmt_end(n) || n == TK_COMMA || n == tk::THEN {
                self.code[p..p + 2].copy_from_slice(&TK_LGO.to_be_bytes());
                ph.label_refs.push(p);
                return Ok(after);
            }
        }
        let (np, _) = self.expr(ph, p)?;
        Ok(np)
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    /// Verifies an expression starting at `p`. Returns the position after it.
    fn expr(&mut self, ph: &mut Phase, p: usize) -> VResult<(usize, Ty)> {
        self.expr_prec(ph, p, 0x7FFF)
    }

    fn expr_prec(&mut self, ph: &mut Phase, p: usize, min: u16) -> VResult<(usize, Ty)> {
        let (mut q, mut lhs) = self.operand(ph, p)?;
        loop {
            let op = self.rd(q);
            if op & 0x8000 == 0 || op <= min {
                return Ok((q, lhs));
            }
            let (nq, rhs) = self.expr_prec(ph, q + 2, op)?;
            lhs = self.op_type(op, lhs, rhs, q)?;
            q = nq;
        }
    }

    fn op_type(&self, op: u16, a: Ty, b: Ty, pos: usize) -> VResult<Ty> {
        use tk::*;
        match op {
            OP_PLUS | OP_MINUS => {
                if a != b {
                    return self.err(terr::TYPE_MISMATCH, pos);
                }
                Ok(a)
            }
            OP_EQ | OP_NE | OP_NE2 | OP_LT | OP_GT | OP_LE | OP_LE2 | OP_GE | OP_GE2 => {
                if a != b {
                    return self.err(terr::TYPE_MISMATCH, pos);
                }
                Ok(Ty::Num)
            }
            _ => {
                if a != Ty::Num || b != Ty::Num {
                    return self.err(terr::TYPE_MISMATCH, pos);
                }
                Ok(Ty::Num)
            }
        }
    }

    fn operand(&mut self, ph: &mut Phase, p: usize) -> VResult<(usize, Ty)> {
        let t = self.rd(p);
        if t & 0x8000 != 0 {
            if t == tk::OP_MINUS {
                let (np, ty) = self.operand(ph, p + 2)?;
                self.want(ty, Ty::Num, p)?;
                return Ok((np, Ty::Num));
            }
            return self.err(terr::SYNTAX, p);
        }
        match t {
            TK_VAR => self.var_ref(ph, p),
            TK_ENT | TK_HEX | TK_BIN | TK_FL | TK_DFL => Ok((p + token_size(&self.code, p), Ty::Num)),
            TK_CH1 | TK_CH2 => Ok((p + token_size(&self.code, p), Ty::Str)),
            TK_PAR1 => {
                let (np, ty) = self.expr(ph, p + 2)?;
                if self.rd(np) != TK_PAR2 {
                    return self.err(terr::SYNTAX, np);
                }
                Ok((np + 2, ty))
            }
            TK_NOT => {
                let (np, ty) = self.expr(ph, p + 2)?;
                self.want(ty, Ty::Num, p)?;
                Ok((np, Ty::Num))
            }
            tk::FN => self.fn_call(ph, p),
            TK_EXT => {
                let slot = self.code[p + 2] as usize;
                let off = self.rd(p + 4);
                if slot >= EXTENSION_SLOTS || lookup_ext(slot, off).is_none() {
                    return self.err(terr::EXTENSION_NOT_LOADED, p);
                }
                self.function(ph, p, Keyword { slot: slot as u8, token: off })
            }
            TK_EOL | TK_LAB | TK_PRO | TK_LGO | TK_REM1 | TK_REM2 => self.err(terr::SYNTAX, p),
            _ => self.function(ph, p, Keyword { slot: 0, token: t }),
        }
    }

    fn fn_call(&mut self, ph: &mut Phase, p: usize) -> VResult<(usize, Ty)> {
        let q = p + 2;
        if self.rd(q) != TK_VAR {
            return self.err(terr::SYNTAX, q);
        }
        let key = {
            let len = self.code[q + 4] as usize;
            let ty = self.code[q + 5] & var_flags::TYPE_MASK;
            var_key(&trim_name(&self.code[q + 6..q + 6 + len]), ty, false, true)
        };
        let known = ph.local_map.contains_key(&key) || self.global_map.contains_key(&key);
        if !known {
            return self.err(terr::USER_FN_NOT_DEFINED, q);
        }
        let (_, ty) = self.resolve_var(ph, q, false, true)?;
        let mut r = q + token_size(&self.code, q);
        if self.rd(r) == TK_PAR1 {
            r += 2;
            loop {
                let (np, _) = self.expr(ph, r)?;
                r = np;
                match self.rd(r) {
                    TK_COMMA => r += 2,
                    TK_PAR2 => {
                        r += 2;
                        break;
                    }
                    _ => return self.err(terr::SYNTAX, r),
                }
            }
        }
        Ok((r, ty_of(ty)))
    }

    /// A keyword used as a function (or reserved variable / constant).
    fn function(&mut self, ph: &mut Phase, p: usize, kw: Keyword) -> VResult<(usize, Ty)> {
        let group = overload_group(kw.slot as usize, kw.token);
        if group.is_empty() {
            return self.err(terr::SYNTAX, p);
        }
        if kw.slot == 0
            && let Some(r) = self.special_function(ph, p, group[0].verif[1])?
        {
            return Ok(r);
        }
        let candidates: Vec<&TokenDef> = group
            .iter()
            .filter(|d| matches!(d.kind(), TokenKind::Function(_) | TokenKind::ReservedVariable))
            .collect();
        if candidates.is_empty() {
            return self.err(terr::SYNTAX, p);
        }
        let tok_end = if kw.slot == 0 { p + 2 + inline_data_size(kw.token) } else { p + 6 };
        let mut args = Args::default();
        let mut q = tok_end;
        let has_paren_variant = candidates.iter().any(|d| !d.param_types().is_empty());
        if self.rd(q) == TK_PAR1 && has_paren_variant {
            let (np, a) = self.arg_list(ph, q + 2, true)?;
            args = a;
            q = np;
        }
        let chosen = self.choose_variant(&candidates, &args, p)?;
        self.rewrite_token(p, kw.slot, chosen.token);
        let ty = match chosen.kind() {
            TokenKind::Function(ValueType::Str) => Ty::Str,
            TokenKind::ReservedVariable => {
                if chosen.params.as_bytes().get(1) == Some(&b'2') {
                    Ty::Str
                } else {
                    Ty::Num
                }
            }
            _ => Ty::Num,
        };
        Ok((q, ty))
    }

    /// Functions with a special syntax (operand classes of the verifier table).
    fn special_function(&mut self, ph: &mut Phase, p: usize, class: u8) -> VResult<Option<(usize, Ty)>> {
        let q = p + 2 + inline_data_size(self.rd(p));
        let open = |s: &Self| if s.rd(q) == TK_PAR1 { Ok(q + 2) } else { s.err(terr::SYNTAX, q) };
        let close = |s: &Self, r: usize| if s.rd(r) == TK_PAR2 { Ok(r + 2) } else { s.err(terr::SYNTAX, r) };
        Ok(Some(match class {
            // Varptr(var), Array(a(0))
            0x08 | 0x0E => {
                let r = open(self)?;
                let (np, _) = self.lvalue(ph, r)?;
                (close(self, np)?, Ty::Num)
            }
            // Equ("name"), Lvo("name")
            0x0C | 0x10 => {
                let r = open(self)?;
                let (np, ty) = self.expr(ph, r)?;
                self.want(ty, Ty::Str, r)?;
                (close(self, np)?, Ty::Num)
            }
            // Match(a(0),value)
            0x0D => {
                let r = open(self)?;
                let (np, t1) = self.lvalue(ph, r)?;
                if self.rd(np) != TK_COMMA {
                    return self.err(terr::SYNTAX, np);
                }
                let (np2, t2) = self.expr(ph, np + 2)?;
                self.want(t2, t1, np + 2)?;
                (close(self, np2)?, Ty::Num)
            }
            // Min(a,b), Max(a,b): numbers or strings.
            0x0F | 0x20 => {
                let r = open(self)?;
                let (np, t1) = self.expr(ph, r)?;
                if self.rd(np) != TK_COMMA {
                    return self.err(terr::SYNTAX, np);
                }
                let (np2, t2) = self.expr(ph, np + 2)?;
                self.want(t2, t1, np + 2)?;
                (close(self, np2)?, t1)
            }
            // X Menu(path...), Y Menu(path...)
            0x0B => {
                let r = open(self)?;
                let (np, _) = self.arg_list(ph, r, true)?;
                (np, Ty::Num)
            }
            // Btst(bit, variable)
            0x27 => {
                let r = open(self)?;
                let (np, _) = self.expr(ph, r)?;
                if self.rd(np) != TK_COMMA {
                    return self.err(terr::SYNTAX, np);
                }
                let (np2, _) = if self.rd(np + 2) == TK_VAR && is_simple_var_end(self.rd(np + 2 + token_size(&self.code, np + 2))) {
                    self.lvalue(ph, np + 2)?
                } else {
                    self.expr(ph, np + 2)?
                };
                (close(self, np2)?, Ty::Num)
            }
            _ => return Ok(None),
        }))
    }

    fn rewrite_token(&mut self, p: usize, slot: u8, token: u16) {
        if slot == 0 {
            self.code[p..p + 2].copy_from_slice(&token.to_be_bytes());
        } else {
            self.code[p + 4..p + 6].copy_from_slice(&token.to_be_bytes());
        }
    }

    fn choose_variant(&self, candidates: &[&'static TokenDef], args: &Args, p: usize) -> VResult<&'static TokenDef> {
        let mut mismatch = false;
        for d in candidates {
            match signature_matches(d.param_types(), args) {
                Ok(true) => return Ok(d),
                Ok(false) => {}
                Err(()) => mismatch = true,
            }
        }
        if mismatch {
            return self.err(terr::TYPE_MISMATCH, p);
        }
        self.err(terr::SYNTAX, p)
    }

    /// Parses `e1, e2 To e3 ...`. With `paren`, the list ends with `)` (which
    /// is consumed); otherwise at the end of the statement.
    fn arg_list(&mut self, ph: &mut Phase, mut q: usize, paren: bool) -> VResult<(usize, Args)> {
        let mut args = Args::default();
        let ends = |t: u16| if paren { t == TK_PAR2 } else { is_stmt_end(t) };
        if ends(self.rd(q)) {
            if paren {
                return Ok((q + 2, args));
            }
            return Ok((q, args));
        }
        loop {
            let t = self.rd(q);
            // Omitted parameter: a separator (or the end after a separator).
            if t == TK_COMMA || t == TK_TO || (ends(t) && !args.types.is_empty()) {
                args.types.push(Ty::Num);
            } else {
                let (np, ty) = self.expr(ph, q)?;
                args.types.push(ty);
                q = np;
            }
            let t = self.rd(q);
            if t == TK_COMMA {
                args.seps.push(b',');
                q += 2;
            } else if t == TK_TO {
                args.seps.push(b't');
                q += 2;
            } else if ends(t) {
                if paren {
                    q += 2;
                }
                return Ok((q, args));
            } else {
                return self.err(terr::SYNTAX, q);
            }
        }
    }

    /// Skips an expression without checking it (used after verification).
    fn skip_expr(&self, mut p: usize) -> usize {
        let mut depth = 0i32;
        loop {
            let t = self.rd(p);
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
            p += token_size(&self.code, p);
        }
    }

    // ------------------------------------------------------------------
    // Generic instructions
    // ------------------------------------------------------------------

    fn st_generic(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let (kw, q) = if self.rd(p) == TK_EXT {
            let slot = self.code[p + 2] as usize;
            let off = self.rd(p + 4);
            if slot >= EXTENSION_SLOTS || lookup_ext(slot, off).is_none() {
                return self.err(terr::EXTENSION_NOT_LOADED, p);
            }
            (Keyword { slot: slot as u8, token: off }, p + 6)
        } else {
            let t = self.rd(p);
            (Keyword { slot: 0, token: t }, p + 2 + inline_data_size(t))
        };
        let group = overload_group(kw.slot as usize, kw.token);
        let candidates: Vec<&TokenDef> =
            group.iter().filter(|d| d.kind() == TokenKind::Instruction || d.kind() == TokenKind::Structure).collect();
        if candidates.is_empty() {
            return self.err(terr::SYNTAX, p);
        }
        let (np, args) = self.arg_list(ph, q, false)?;
        let chosen = self.choose_variant(&candidates, &args, p)?;
        self.rewrite_token(p, kw.slot, chosen.token);
        if kw.slot != 0 {
            self.code[p + 3] = args.types.len() as u8;
        }
        Ok(np)
    }

    /// Statement whose parameters are checked loosely: any expressions,
    /// separated by `,`, `;`, `To`, `#`, with optional parentheses.
    fn st_loose(&mut self, ph: &mut Phase, mut q: usize) -> VResult<usize> {
        loop {
            let t = self.rd(q);
            if is_stmt_end(t) {
                return Ok(q);
            }
            match t {
                TK_COMMA | TK_SEMI | TK_TO | TK_HASH | TK_PAR1 | TK_PAR2 => q += 2,
                _ if t & 0x8000 == 0 && t > TK_EXT && tokens::lookup(t).is_some_and(|d| {
                    matches!(d.kind(), TokenKind::Instruction | TokenKind::Structure)
                }) =>
                {
                    // Keywords used as parameters (Channel x To Bob ...).
                    q += 2 + inline_data_size(t);
                }
                _ => {
                    let (np, _) = self.expr(ph, q)?;
                    q = np;
                }
            }
        }
    }

    /// Loose parameters where parentheses belong to expressions.
    fn st_loose_no_paren(&mut self, ph: &mut Phase, mut q: usize) -> VResult<usize> {
        loop {
            let t = self.rd(q);
            if is_stmt_end(t) {
                return Ok(q);
            }
            match t {
                TK_COMMA | TK_SEMI | TK_TO | TK_HASH => q += 2,
                _ if t & 0x8000 == 0 && t > TK_EXT && tokens::lookup(t).is_some_and(|d| {
                    matches!(d.kind(), TokenKind::Instruction | TokenKind::Structure)
                }) =>
                {
                    q += 2 + inline_data_size(t);
                }
                _ => {
                    let (np, _) = self.expr(ph, q)?;
                    q = np;
                }
            }
        }
    }

    fn expr_list(&mut self, ph: &mut Phase, mut q: usize, want: Option<Ty>) -> VResult<usize> {
        if is_stmt_end(self.rd(q)) {
            return Ok(q);
        }
        loop {
            if self.rd(q) == TK_COMMA {
                q += 2;
                continue;
            }
            let (np, ty) = self.expr(ph, q)?;
            if let Some(w) = want {
                self.want(ty, w, q)?;
            }
            q = np;
            match self.rd(q) {
                TK_COMMA => q += 2,
                _ => return Ok(q),
            }
        }
    }

    // ------------------------------------------------------------------
    // Specific statements
    // ------------------------------------------------------------------

    fn st_dim(&mut self, ph: &mut Phase, mut q: usize) -> VResult<usize> {
        loop {
            if self.rd(q) != TK_VAR {
                return self.err(terr::SYNTAX, q);
            }
            let after = q + token_size(&self.code, q);
            if self.rd(after) != TK_PAR1 {
                return self.err(terr::SYNTAX, after);
            }
            let key = self.var_key_at(q, true);
            if ph.dimmed.contains(&key) {
                return self.err(terr::ARRAY_ALREADY_DIMENSIONED, q);
            }
            ph.dimmed.push(key.clone());
            self.resolve_var(ph, q, true, false)?;
            let mut r = after + 2;
            loop {
                let (np, ty) = self.expr(ph, r)?;
                self.want(ty, Ty::Num, r)?;
                r = np;
                match self.rd(r) {
                    TK_COMMA => r += 2,
                    TK_PAR2 => {
                        r += 2;
                        break;
                    }
                    _ => return self.err(terr::SYNTAX, r),
                }
            }
            if self.rd(r) == TK_COMMA {
                q = r + 2;
            } else {
                return Ok(r);
            }
        }
    }

    fn st_print(&mut self, ph: &mut Phase, mut q: usize) -> VResult<usize> {
        loop {
            let t = self.rd(q);
            if is_stmt_end(t) {
                return Ok(q);
            }
            if t == TK_SEMI || t == TK_COMMA {
                q += 2;
                continue;
            }
            if t == tk::USING {
                // Print Using format$;expr...
                let (np, ty) = self.expr(ph, q + 2)?;
                self.want(ty, Ty::Str, q + 2)?;
                q = np;
                continue;
            }
            let (np, _) = self.expr(ph, q)?;
            q = np;
        }
    }

    fn st_input(&mut self, ph: &mut Phase, mut q: usize, channel: bool) -> VResult<usize> {
        if channel {
            let (np, ty) = self.expr(ph, q)?;
            self.want(ty, Ty::Num, q)?;
            q = np;
            if self.rd(q) != TK_COMMA {
                return self.err(terr::SYNTAX, q);
            }
            q += 2;
        } else if self.rd(q) != TK_VAR {
            // Prompt string expression.
            let (np, ty) = self.expr(ph, q)?;
            self.want(ty, Ty::Str, q)?;
            q = np;
            if self.rd(q) != TK_SEMI && self.rd(q) != TK_COMMA {
                return self.err(terr::SYNTAX, q);
            }
            q += 2;
        }
        loop {
            let (np, _) = self.lvalue(ph, q)?;
            q = np;
            match self.rd(q) {
                TK_COMMA => q += 2,
                TK_SEMI => {
                    q += 2;
                    if is_stmt_end(self.rd(q)) {
                        return Ok(q);
                    }
                }
                _ => return Ok(q),
            }
        }
    }

    fn st_add(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let q = p + 2;
        let (np, ty) = self.lvalue(ph, q)?;
        self.want(ty, Ty::Num, q)?;
        if self.rd(np) != TK_COMMA {
            return self.err(terr::SYNTAX, np);
        }
        let (np, ty) = self.expr(ph, np + 2)?;
        self.want(ty, Ty::Num, np)?;
        if self.rd(np) == TK_COMMA {
            // Add v,n,a To b
            let (np2, _) = self.expr(ph, np + 2)?;
            if self.rd(np2) != TK_TO {
                return self.err(terr::SYNTAX, np2);
            }
            let (np3, _) = self.expr(ph, np2 + 2)?;
            self.code[p..p + 2].copy_from_slice(&tk::ADD_2.to_be_bytes());
            return Ok(np3);
        }
        self.code[p..p + 2].copy_from_slice(&tk::ADD.to_be_bytes());
        Ok(np)
    }

    fn st_polyline(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let mut q = p + 2;
        loop {
            let t = self.rd(q);
            if is_stmt_end(t) {
                return Ok(q);
            }
            if t == TK_COMMA || t == TK_TO {
                q += 2;
                continue;
            }
            let (np, ty) = self.expr(ph, q)?;
            self.want(ty, Ty::Num, q)?;
            q = np;
        }
    }

    fn st_field(&mut self, ph: &mut Phase, q: usize) -> VResult<usize> {
        // Field #n, len As v$, ...
        let mut r = q;
        loop {
            let t = self.rd(r);
            if is_stmt_end(t) {
                return Ok(r);
            }
            match t {
                TK_COMMA | TK_HASH => r += 2,
                tk::AS => r += 2,
                TK_VAR => {
                    let (np, _) = self.lvalue(ph, r)?;
                    r = np;
                }
                _ => {
                    let (np, _) = self.expr(ph, r)?;
                    r = np;
                }
            }
        }
    }

    fn st_menu_assign(&mut self, ph: &mut Phase, q: usize) -> VResult<usize> {
        // Menu$(a,b...)=s1$[,s2$...]
        if self.rd(q) != TK_PAR1 {
            return self.err(terr::SYNTAX, q);
        }
        let (np, _) = self.arg_list(ph, q + 2, true)?;
        if self.rd(np) != tk::OP_EQ {
            return self.err(terr::SYNTAX, np);
        }
        let mut r = np + 2;
        loop {
            if self.rd(r) == TK_COMMA {
                r += 2;
                continue;
            }
            if is_stmt_end(self.rd(r)) {
                return Ok(r);
            }
            let (nr, _) = self.expr(ph, r)?;
            r = nr;
        }
    }

    fn st_reserved_assign(&mut self, ph: &mut Phase, p: usize, def: &'static TokenDef) -> VResult<usize> {
        // Reserved variable as instruction: name[(params)]=expression.
        let group = overload_group(0, def.token);
        let candidates: Vec<&TokenDef> =
            group.iter().filter(|d| d.kind() == TokenKind::ReservedVariable).collect();
        let mut q = p + 2;
        let mut args = Args::default();
        if self.rd(q) == TK_PAR1 {
            let (np, a) = self.arg_list(ph, q + 2, true)?;
            args = a;
            q = np;
        }
        let chosen = if candidates.is_empty() { def } else { self.choose_variant(&candidates, &args, p)? };
        self.rewrite_token(p, 0, chosen.token);
        if self.rd(q) != tk::OP_EQ {
            return self.err(terr::SYNTAX, q);
        }
        let (np, ty) = self.expr(ph, q + 2)?;
        let want = if chosen.params.as_bytes().get(1) == Some(&b'2') { Ty::Str } else { Ty::Num };
        self.want(ty, want, q + 2)?;
        Ok(np)
    }

    fn st_mid_assign(&mut self, ph: &mut Phase, p: usize, def: &'static TokenDef) -> VResult<usize> {
        // Mid$(v$,p[,n])=e$, Left$(v$,n)=e$, Right$(v$,n)=e$
        let q = p + 2;
        if self.rd(q) != TK_PAR1 {
            return self.err(terr::SYNTAX, q);
        }
        let (np, ty) = self.lvalue(ph, q + 2)?;
        self.want(ty, Ty::Str, q + 2)?;
        let mut r = np;
        let mut n = 0;
        while self.rd(r) == TK_COMMA {
            let (nr, ty) = self.expr(ph, r + 2)?;
            self.want(ty, Ty::Num, r + 2)?;
            r = nr;
            n += 1;
        }
        if self.rd(r) != TK_PAR2 {
            return self.err(terr::SYNTAX, r);
        }
        r += 2;
        if self.rd(r) != tk::OP_EQ {
            return self.err(terr::SYNTAX, r);
        }
        let (end, ety) = self.expr(ph, r + 2)?;
        self.want(ety, Ty::Str, r + 2)?;
        // Mid$ has two forms: pick the one matching the parameter count.
        let expected = match def.name {
            "mid$" => {
                let token = if n == 1 { tk::MID_S_2 } else { tk::MID_S };
                self.code[p..p + 2].copy_from_slice(&token.to_be_bytes());
                if n == 1 || n == 2 { n } else { 0 }
            }
            _ => 1,
        };
        if n != expected {
            return self.err(terr::SYNTAX, p);
        }
        Ok(end)
    }

    fn st_def_fn(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        // Def Fn name[(params)]=expression
        let q = p + 2;
        if self.rd(q) != TK_VAR {
            return self.err(terr::SYNTAX, q);
        }
        let (_, fty) = self.resolve_var(ph, q, false, true)?;
        let mut r = q + token_size(&self.code, q);
        if self.rd(r) == TK_PAR1 {
            r += 2;
            loop {
                if self.rd(r) != TK_VAR {
                    return self.err(terr::SYNTAX, r);
                }
                self.resolve_var(ph, r, false, false)?;
                r += token_size(&self.code, r);
                match self.rd(r) {
                    TK_COMMA => r += 2,
                    TK_PAR2 => {
                        r += 2;
                        break;
                    }
                    _ => return self.err(terr::SYNTAX, r),
                }
            }
        }
        if self.rd(r) != tk::OP_EQ {
            return self.err(terr::SYNTAX, r);
        }
        let (end, ty) = self.expr(ph, r + 2)?;
        self.want(ty, ty_of(fty), r + 2)?;
        if self.rd(end) != TK_EOL {
            return self.err(terr::ALONE_ON_LINE, end);
        }
        Ok(end)
    }

    fn st_data(&mut self, ph: &mut Phase, p: usize, line: usize) -> VResult<usize> {
        let off = (p + 2 - line) as u16;
        self.code[p + 2..p + 4].copy_from_slice(&off.to_be_bytes());
        let mut q = p + 4;
        loop {
            let t = self.rd(q);
            if is_stmt_end(t) {
                return Ok(q);
            }
            if t == TK_COMMA {
                q += 2;
                continue;
            }
            let (np, _) = self.expr(ph, q)?;
            q = np;
        }
    }

    fn shared_global(&mut self, ph: &mut Phase, p: usize, from_proc_in_main: bool) -> VResult<usize> {
        let is_global = self.rd(p) == tk::GLOBAL;
        let mut q = p + 2;
        loop {
            if self.rd(q) != TK_VAR {
                return self.err(terr::SYNTAX, q);
            }
            let after = q + token_size(&self.code, q);
            let array = self.rd(after) == TK_PAR1;
            let mut next = after;
            if array {
                if self.rd(after + 2) != TK_PAR2 {
                    return self.err(terr::EMPTY_BRACKETS, after);
                }
                next = after + 4;
            }
            let len = self.code[q + 4] as usize;
            let ty = self.code[q + 5] & var_flags::TYPE_MASK;
            let name = trim_name(&self.code[q + 6..q + 6 + len]);
            let key = var_key(&name, ty, array, false);
            if ph.index == 0 {
                let slot = self.global_slot(&key, &name, ty, array, false);
                let g = (slot & !GLOBAL) as usize;
                if is_global && !from_proc_in_main {
                    self.global_vis[g] = 2;
                }
                if is_global && from_proc_in_main {
                    // Global inside a procedure also makes it visible everywhere.
                    self.global_vis[g] = 2;
                }
            } else {
                let Some(&g) = self.global_map.get(&key) else {
                    return self.err(terr::ARRAY_NOT_DIMENSIONED, q);
                };
                if self.global_vis[g as usize] == 0 {
                    self.global_vis[g as usize] = 1;
                }
            }
            if !from_proc_in_main {
                let slot = self.global_map[&key];
                self.code[q + 2..q + 4].copy_from_slice(&(GLOBAL | slot).to_be_bytes());
                if array {
                    self.code[q + 5] |= var_flags::ARRAY;
                }
            }
            match self.rd(next) {
                TK_COMMA => q = next + 2,
                TK_EOL => return Ok(next),
                _ => return self.err(terr::SHARED_ALONE, next),
            }
        }
    }

    fn st_on_error(&mut self, ph: &mut Phase, q: usize) -> VResult<usize> {
        // On Error [Goto label | Proc name]
        match self.rd(q) {
            t if is_stmt_end(t) => Ok(q),
            tk::GOTO => {
                // Goto 0 disables the handler.
                if self.rd(q + 2) == TK_ENT {
                    return Ok(q + 8);
                }
                self.label_operand(ph, q + 2)
            }
            tk::PROC => self.proc_operand(ph, q + 2),
            _ => self.err(terr::SYNTAX, q),
        }
    }

    fn st_on(&mut self, ph: &mut Phase, p: usize, menu: bool) -> VResult<usize> {
        // On expr Goto|Gosub|Proc l1,l2...   /  On Menu Goto|Gosub|Proc ...
        let field = p + 2;
        let mut q = if menu { p + 2 } else { p + 6 };
        if !menu {
            let (np, ty) = self.expr(ph, q)?;
            self.want(ty, Ty::Num, q)?;
            q = np;
        }
        let kind = self.rd(q);
        if kind != tk::GOTO && kind != tk::GOSUB && kind != tk::PROC {
            // On Menu On / Off etc. are separate keywords; anything else is an error.
            return self.err(terr::SYNTAX, q);
        }
        q += 2;
        let list_start = q;
        let mut count = 0u16;
        loop {
            q = if kind == tk::PROC { self.proc_operand(ph, q)? } else { self.label_operand(ph, q)? };
            count += 1;
            if self.rd(q) == TK_COMMA {
                q += 2;
            } else {
                break;
            }
        }
        if !menu {
            let len = (q - list_start) as u16;
            self.code[field..field + 2].copy_from_slice(&len.to_be_bytes());
            self.code[field + 2..field + 4].copy_from_slice(&count.to_be_bytes());
        }
        Ok(q)
    }

    // ------------------------------------------------------------------
    // Loops and tests
    // ------------------------------------------------------------------

    fn patch_distance(&mut self, field: usize, target: usize, flag: u16) -> VResult<()> {
        let d = target.checked_sub(field).filter(|&d| d <= 0xFFFE).ok_or(TestError {
            code: terr::STRUCTURE_TOO_LONG,
            pos: field,
        })?;
        let v = d as u16 | flag;
        self.code[field..field + 2].copy_from_slice(&v.to_be_bytes());
        Ok(())
    }

    fn st_for(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let q = p + 4;
        if self.rd(q) != TK_VAR {
            return self.err(terr::SYNTAX, q);
        }
        let (np, ty) = self.var_ref(ph, q)?;
        self.want(ty, Ty::Num, q)?;
        let key = self.rd(q + 2);
        if self.rd(np) != tk::OP_EQ {
            return self.err(terr::SYNTAX, np);
        }
        let (np, ty) = self.expr(ph, np + 2)?;
        self.want(ty, Ty::Num, np)?;
        if self.rd(np) != TK_TO {
            return self.err(terr::SYNTAX, np);
        }
        let (mut np, ty) = self.expr(ph, np + 2)?;
        self.want(ty, Ty::Num, np)?;
        if self.rd(np) == tk::STEP {
            let (n2, ty) = self.expr(ph, np + 2)?;
            self.want(ty, Ty::Num, np)?;
            np = n2;
        }
        ph.loops.push(LoopRec { token: TK_FOR, field: p + 2, var: Some(key), exits: Vec::new(), pos: p });
        Ok(np)
    }

    fn st_next(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let mut q = p + 2;
        let mut var = None;
        if self.rd(q) == TK_VAR {
            let (np, _) = self.var_ref(ph, q)?;
            var = Some(self.rd(q + 2));
            q = np;
        }
        let Some(top) = ph.loops.last() else { return self.err(terr::NEXT_WITHOUT_FOR, p) };
        if top.token != TK_FOR {
            return self.err(terr::NEXT_WITHOUT_FOR, p);
        }
        if let Some(v) = var
            && Some(v) != top.var
        {
            return self.err(terr::NEXT_WITHOUT_FOR, p);
        }
        self.close_loop(ph, TK_FOR, p, q, terr::NEXT_WITHOUT_FOR)?;
        Ok(q)
    }

    /// Closes the innermost loop, which must be a `token` loop. `end` is the
    /// position after the closing statement.
    fn close_loop(&mut self, ph: &mut Phase, token: u16, p: usize, end: usize, err: u16) -> VResult<()> {
        let Some(top) = ph.loops.pop() else { return self.err(err, p) };
        if top.token != token && !(token == TK_REPEAT && top.token == TK_REPEAT) {
            return self.err(if top.token == TK_FOR { terr::FOR_WITHOUT_NEXT } else { err }, p);
        }
        let target = self.statement_start(end);
        self.patch_distance(top.field, target, 0)?;
        for (field, frames) in top.exits {
            self.patch_distance(field, target, 0)?;
            self.code[field + 2..field + 4].copy_from_slice(&frames.to_be_bytes());
        }
        Ok(())
    }

    fn st_exit(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let t = self.rd(p);
        let mut q = p + 6;
        if t == TK_EXIT_IF {
            let (np, ty) = self.expr(ph, q)?;
            self.want(ty, Ty::Num, q)?;
            q = np;
            if self.rd(q) == TK_COMMA {
                q += 2;
            }
        }
        let mut n = 1i32;
        if matches!(self.rd(q), TK_ENT) {
            let (np, v) = self.const_int(q)?;
            n = v;
            q = np;
        }
        if n < 1 || n as usize > ph.loops.len() {
            return self.err(terr::NOT_ENOUGH_LOOPS, p);
        }
        let idx = ph.loops.len() - n as usize;
        ph.loops[idx].exits.push((p + 2, n as u16));
        Ok(q)
    }

    fn st_if(&mut self, ph: &mut Phase, p: usize, _line: usize) -> VResult<usize> {
        let field = p + 2;
        let (np, ty) = self.expr(ph, p + 4)?;
        self.want(ty, Ty::Num, p + 4)?;
        if self.rd(np) != tk::THEN {
            // Structured If.
            ph.ifs.push(IfRec {
                pending_false: Some(field),
                exits: Vec::new(),
                else_fields: Vec::new(),
                seen_else: false,
                pos: p,
            });
            return Ok(np);
        }
        // One line If: find a matching Else on this line.
        let mut q = np + 2;
        let target = self.single_line_target(q);
        self.patch_distance(field, target, 0)?;
        // Then <label>
        if matches!(self.rd(q), TK_LGO) {
            ph.label_refs.push(q);
            q += token_size(&self.code, q);
            if !is_stmt_end(self.rd(q)) {
                return self.err(terr::SYNTAX, q);
            }
            return Ok(q);
        }
        // Statements after Then are verified by the line loop.
        Ok(q)
    }

    /// Target of a false one-line If: after the matching Else on the same
    /// line, or the end of the line.
    fn single_line_target(&self, mut q: usize) -> usize {
        let mut depth = 0;
        loop {
            let t = self.rd(q);
            if t == TK_EOL {
                return q;
            }
            if t == TK_IF {
                // Nested one-line If (with Then) on the same line.
                depth += 1;
            } else if t == TK_ELSE {
                if depth == 0 {
                    return q + 4;
                }
                depth -= 1;
            }
            q += token_size(&self.code, q);
        }
    }

    fn st_else(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let field = p + 2;
        let q = p + 4;
        // One line If's Else: jump to the end of the line.
        if self.is_one_line_else(p) {
            let mut e = q;
            while self.rd(e) != TK_EOL {
                e += token_size(&self.code, e);
            }
            self.patch_distance(field, e, 0)?;
            self.else_exit.insert(p, e);
            if self.rd(q) == TK_LGO {
                ph.label_refs.push(q);
                return Ok(q + token_size(&self.code, q));
            }
            return Ok(q);
        }
        let Some(rec) = ph.ifs.last_mut() else { return self.err(terr::ELSE_WITHOUT_IF, p) };
        if rec.seen_else {
            return self.err(terr::ELSE_WITHOUT_IF, p);
        }
        rec.seen_else = true;
        let pending = rec.pending_false.take();
        rec.exits.push(p);
        rec.else_fields.push(field);
        if let Some(f) = pending {
            self.patch_distance(f, q, 0)?;
        }
        Ok(q)
    }

    /// An Else belongs to a one-line If when an If ... Then precedes it on
    /// the same line.
    fn is_one_line_else(&self, else_pos: usize) -> bool {
        // Find the start of the line containing else_pos.
        let mut line = 0;
        loop {
            let next = self.next_line(line);
            if next > else_pos || self.code[next] == 0 {
                break;
            }
            line = next;
        }
        let mut q = line + 2;
        let mut depth = 0i32;
        while q < else_pos {
            let t = self.rd(q);
            if t == tk::THEN {
                depth += 1;
            } else if t == TK_ELSE {
                depth -= 1;
            }
            q += token_size(&self.code, q);
        }
        depth > 0
    }

    fn st_else_if(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let field = p + 2;
        let Some(rec) = ph.ifs.last_mut() else { return self.err(terr::ELSE_WITHOUT_IF, p) };
        if rec.seen_else {
            return self.err(terr::ELSE_WITHOUT_IF, p);
        }
        let pending = rec.pending_false.replace(field);
        rec.exits.push(p);
        if let Some(f) = pending {
            // False branch of the previous test: evaluate this condition.
            self.patch_distance(f, p + 4, 1)?;
        }
        let (np, ty) = self.expr(ph, p + 4)?;
        self.want(ty, Ty::Num, p + 4)?;
        if self.rd(np) == tk::THEN {
            return Ok(np + 2);
        }
        Ok(np)
    }

    fn st_end_if(&mut self, ph: &mut Phase, p: usize) -> VResult<usize> {
        let Some(rec) = ph.ifs.pop() else { return self.err(terr::ENDIF_WITHOUT_IF, p) };
        let target = p + 2;
        if let Some(f) = rec.pending_false {
            self.patch_distance(f, target, 0)?;
        }
        for f in rec.else_fields {
            self.patch_distance(f, target, 0)?;
        }
        for e in rec.exits {
            self.else_exit.insert(e, target);
        }
        Ok(p + 2)
    }
}

fn is_simple_var_end(t: u16) -> bool {
    t == TK_PAR2 || t == TK_PAR1
}

fn open_loop_error(token: u16) -> u16 {
    match token {
        TK_FOR => terr::FOR_WITHOUT_NEXT,
        TK_REPEAT => terr::REPEAT_WITHOUT_UNTIL,
        TK_WHILE => terr::WHILE_WITHOUT_WEND,
        _ => terr::DO_WITHOUT_LOOP,
    }
}

fn is_line_start_only(t: u16) -> bool {
    matches!(t, tk::SHARED | tk::GLOBAL | tk::DEF_FN | TK_DATA | TK_PROCEDURE | TK_END_PROC)
}

/// Variable names are stored lower case, zero padded.
pub fn trim_name(raw: &[u8]) -> Vec<u8> {
    let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    raw[..n].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenise::tokenise_program;

    fn verify(src: &str) -> Result<Compiled, TestError> {
        let prg = tokenise_program(src.as_bytes()).unwrap();
        Verifier::verify(&prg.source, prg.math_flags)
    }

    #[test]
    fn simple_program_verifies() {
        let c = verify("A=1\nB$=\"x\"+\"y\"\nPrint A,B$\nFor I=1 To 10\nNext I\n").unwrap();
        assert_eq!(c.globals.len(), 3);
    }

    #[test]
    fn type_mismatch() {
        assert_eq!(verify("A=\"x\"").unwrap_err().code, terr::TYPE_MISMATCH);
    }

    #[test]
    fn loops_must_match() {
        assert_eq!(verify("For I=1 To 2\n").unwrap_err().code, terr::FOR_WITHOUT_NEXT);
        assert_eq!(verify("Next I\n").unwrap_err().code, terr::NEXT_WITHOUT_FOR);
        assert_eq!(verify("Repeat\nLoop\n").unwrap_err().code, terr::LOOP_WITHOUT_DO);
        assert!(verify("Do\nRepeat\nUntil A=1\nLoop\n").is_ok());
    }

    #[test]
    fn procedures() {
        let c = verify("TEST[1,\"a\"]\nProcedure TEST[A,B$]\n  Print A,B$\nEnd Proc\n").unwrap();
        assert_eq!(c.procs.len(), 1);
        assert_eq!(c.procs[0].params.len(), 2);
        assert_eq!(verify("FOO\n").unwrap_err().code, terr::UNDEFINED_PROC);
    }

    #[test]
    fn labels() {
        assert!(verify("Goto L\nL:\n").is_ok());
        assert_eq!(verify("Goto M\nL:\n").unwrap_err().code, terr::UNDEFINED_LABEL);
    }

    #[test]
    fn verifies_all_example_programs() {
        // Every program shipped with AMOS must pass the test.
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../AMOS-Professional-365/AMOS");
        let mut files = Vec::new();
        fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("amos")) {
                    out.push(p);
                }
            }
        }
        walk(&dir, &mut files);
        // Compiled program header: holds a compiled procedure.
        files.retain(|f| !f.ends_with("Header_AMOS.AMOS"));
        let mut failures = Vec::new();
        for f in &files {
            let prg = crate::program::Program::load(&std::fs::read(f).unwrap()).unwrap();
            if let Err(e) = Verifier::verify(&prg.source, prg.math_flags) {
                // Find the line for the report.
                let mut line_text = String::new();
                for (off, line) in prg.lines() {
                    if off <= e.pos && e.pos < off + line.len() {
                        line_text = crate::detok::latin1_to_string(&crate::detok::detok_line(line));
                    }
                }
                failures.push(format!(
                    "{}: {} ({}) at {}: {}",
                    f.display(),
                    crate::errors::test_message(e.code),
                    e.code,
                    e.pos,
                    line_text.trim()
                ));
            }
        }
        if !failures.is_empty() {
            panic!("{} of {} programs failed:\n{}", failures.len(), files.len(), failures.join("\n"));
        }
    }
}
