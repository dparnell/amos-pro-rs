//! Front end: lowers the verified token stream to the IR.
//!
//! Parsing mirrors the interpreter exactly (`interp/expr.rs`, `flow.rs`,
//! `stmt.rs`): every operator is its own precedence level, any operator in
//! operand position is a single negation, `Not` applies to the whole
//! following expression... Anything this front end does not translate
//! becomes [`Stmt::Interp`]: the instruction is run by the interpreter at
//! run time, so nothing is ever translated approximately.

use amos_core::compiled::structure::{self, Instr, rd};
use amos_core::interp::verify::{Compiled, token_size};
use amos_core::number::ffp_to_f64;
use amos_core::program::{read_u32, var_flags};
use amos_core::tokens::{Keyword, TokenKind, ValueType, tk, *};

use crate::ir::*;

/// Why an instruction is not translated (it is then interpreted).
pub type Unsupported = &'static str;
type Res<T> = Result<T, Unsupported>;

pub struct Lower<'a> {
    pub prg: &'a Compiled,
    code: &'a [u8],
    /// Variables kept in the interpreter (`structure::resident_vars`).
    resident: Vec<(usize, u16)>,
}

/// Instructions whose handlers take a variable parameter by reference when
/// it is a lone variable (`Hardware::bit_op`): they cannot get a value
/// through the keyword bridge.
fn takes_var_by_reference(kw: Keyword) -> bool {
    use tk::*;
    kw.slot == 0 && matches!(kw.token, BSET | BCLR | BCHG | ROR_B | ROR_W | ROR_L | ROL_B | ROL_W | ROL_L)
}

/// Instructions of the interpreter core (`exec_flow`, `exec_core`): never
/// sent to the keyword bridge.
fn is_core_instruction(t: u16) -> bool {
    use tk::*;
    matches!(
        t,
        TK_FOR
            | TK_REPEAT
            | TK_DO
            | TK_WHILE
            | NEXT
            | UNTIL
            | WEND
            | LOOP
            | TK_EXIT
            | TK_EXIT_IF
            | TK_IF
            | TK_ELSE
            | TK_ELSE_IF
            | END_IF
            | THEN
            | GOTO
            | GOSUB
            | RETURN
            | POP
            | TK_ON
            | TK_PRO
            | PROC
            | TK_PROCEDURE
            | TK_END_PROC
            | POP_PROC
            | TK_DATA
            | READ
            | RESTORE
            | ON_ERROR
            | RESUME
            | RESUME_NEXT
            | RESUME_LABEL
            | TRAP
            | ERROR
            | EVERY
            | EVERY_ON
            | EVERY_OFF
            | BREAK_ON
            | BREAK_OFF
            | ON_BREAK_PROC
            | END
            | STOP
            | EDIT
            | DIRECT
            | SYSTEM
            | PRINT
            | DIM
            | INC
            | DEC
            | ADD
            | ADD_2
            | SWAP
            | SORT
            | SHARED
            | GLOBAL
            | DEF_FN
            | SET_BUFFER
            | SET_STACK
            | SET_DOUBLE_PRECISION
            | SET_ACCESSORY
            | AMOS_LOCK
            | AMOS_UNLOCK
            | CLOSE_EDITOR
            | CLOSE_WORKBENCH
            | RANDOMIZE
            | DEGREE
            | RADIAN
            | FIX
            | WAIT
            | WAIT_VBL
            | MID_S
            | MID_S_2
            | LEFT_S
            | RIGHT_S
            | INPUT
            | LINE_INPUT
    )
}

/// Functions of the interpreter core (`core_function`,
/// `string_maths_function`).
fn is_core_function(t: u16) -> bool {
    use tk::*;
    structure::is_single_arg_core(t)
        || matches!(
            t,
            FN | MIN
                | MAX
                | MATCH
                | PARAM
                | PARAM_F
                | PARAM_S
                | TRUE
                | FALSE
                | ERRN
                | ERRTRAP
                | PI_F
                | STR_S
                | STRING_S
                | REPEAT_S
                | LEFT_S
                | RIGHT_S
                | MID_S
                | MID_S_2
                | INSTR
                | INSTR_2
                | HEX_S
                | HEX_S_2
                | BIN_S
                | BIN_S_2
                | ABS
                | INT
                | SGN
                | BTST
        )
}

/// Verifier classes of instructions with a special syntax (`st_keyword`):
/// their parameters are not a plain signature list.
fn is_special_class(class: u8) -> bool {
    matches!(
        class,
        0x01..=0x05
            | 0x09..=0x14
            | 0x16..=0x19
            | 0x1B..=0x1F
            | 0x21
            | 0x22
            | 0x25..=0x27
            | 0x2B
            | 0x2C
            | 0x2F..=0x49
            | 0x4A..=0x4E
            | 0x4F
            | 0x52..=0x55
            | 0x57
            | 0xFA..=0xFF
    )
}

fn is_op(op: u16) -> bool {
    use tk::*;
    matches!(
        op,
        OP_XOR
            | OP_OR
            | OP_AND
            | OP_NE
            | OP_NE2
            | OP_LE
            | OP_LE2
            | OP_GE
            | OP_GE2
            | OP_EQ
            | OP_LT
            | OP_GT
            | OP_PLUS
            | OP_MINUS
            | OP_MOD
            | OP_MUL
            | OP_DIV
            | OP_POW
    )
}

pub fn is_comparison(op: u16) -> bool {
    use tk::*;
    matches!(op, OP_NE | OP_NE2 | OP_LE | OP_LE2 | OP_GE | OP_GE2 | OP_EQ | OP_LT | OP_GT)
}

/// Type of `a op b` (`Interp::binop`, `compat`).
fn bin_type(op: u16, a: Ty, b: Ty) -> Res<Ty> {
    use tk::*;
    let num = a.is_num() && b.is_num();
    Ok(match op {
        OP_AND | OP_OR | OP_XOR | OP_MOD => {
            if !num {
                return Err("type mismatch");
            }
            Ty::Int
        }
        OP_POW => {
            if !num {
                return Err("type mismatch");
            }
            Ty::Float
        }
        _ if is_comparison(op) => {
            if a.is_num() != b.is_num() {
                return Err("type mismatch");
            }
            Ty::Int
        }
        _ => {
            if a == Ty::Str && b == Ty::Str {
                if op == OP_PLUS || op == OP_MINUS {
                    Ty::Str
                } else {
                    return Err("type mismatch");
                }
            } else if !num {
                return Err("type mismatch");
            } else {
                compat(a, b)
            }
        }
    })
}

/// Common numeric type of two operands.
fn compat(a: Ty, b: Ty) -> Ty {
    match (a, b) {
        (Ty::Dyn, _) | (_, Ty::Dyn) => Ty::Dyn,
        (Ty::Int, Ty::Int) => Ty::Int,
        (Ty::Str, Ty::Str) => Ty::Str,
        _ => Ty::Float,
    }
}

impl<'a> Lower<'a> {
    pub fn new(prg: &'a Compiled, instrs: &[Instr]) -> Self {
        Lower { prg, code: &prg.code, resident: structure::resident_vars(prg, instrs) }
    }

    /// True if the instruction uses a variable kept in the interpreter.
    fn uses_resident(&self, ins: &Instr) -> bool {
        if self.resident.is_empty() {
            return false;
        }
        let mut p = ins.pos;
        while p < ins.end {
            if self.rd(p) == TK_VAR
                && self.resident.binary_search(&structure::var_key(ins.scope, self.rd(p + 2))).is_ok()
            {
                return true;
            }
            p += token_size(self.code, p);
        }
        false
    }

    fn rd(&self, p: usize) -> u16 {
        rd(self.code, p)
    }

    fn expect(&self, p: usize, t: u16) -> Res<usize> {
        if self.rd(p) == t { Ok(p + 2) } else { Err("syntax") }
    }

    fn field_target(&self, field: usize) -> usize {
        field + self.rd(field) as usize
    }

    /// `Interp::statement_start`.
    fn statement_start(&self, mut p: usize) -> usize {
        loop {
            match self.rd(p) {
                TK_DP => p += 2,
                TK_EOL => {
                    let next = p + 2;
                    if next + 1 >= self.code.len() || self.code[next] == 0 {
                        return p;
                    }
                    p = next + 2;
                }
                _ => return p,
            }
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    pub fn expr(&self, p: usize) -> Res<(Expr, usize)> {
        self.expr_prec(p, 0x7FFF)
    }

    fn expr_prec(&self, p: usize, min: u16) -> Res<(Expr, usize)> {
        let (mut lhs, mut q) = self.operand(p)?;
        loop {
            let op = self.rd(q);
            if op & 0x8000 == 0 || op <= min {
                return Ok((lhs, q));
            }
            if !is_op(op) {
                return Err("unknown operator");
            }
            let (rhs, nq) = self.expr_prec(q + 2, op)?;
            let ty = bin_type(op, lhs.ty, rhs.ty)?;
            lhs = Expr::new(ExprKind::Bin(op, Box::new(lhs), Box::new(rhs)), ty);
            q = nq;
        }
    }

    fn operand(&self, mut p: usize) -> Res<(Expr, usize)> {
        let mut negate = false;
        while self.rd(p) & 0x8000 != 0 {
            p += 2;
            negate = true;
        }
        let (v, q) = self.operand_value(p)?;
        if !negate {
            return Ok((v, q));
        }
        if v.ty == Ty::Str {
            return Err("type mismatch");
        }
        let ty = v.ty;
        Ok((Expr::new(ExprKind::Neg(Box::new(v)), ty), q))
    }

    /// A variable or array element (`Interp::var_ref`).
    fn var_ref(&self, p: usize) -> Res<(LValue, usize)> {
        if self.rd(p) != TK_VAR {
            return Err("syntax");
        }
        let slot = self.rd(p + 2);
        let flags = self.code[p + 5];
        let ty = flags & 3;
        let array = flags & var_flags::ARRAY != 0;
        if flags & var_flags::DEF_FN != 0 {
            return Err("Def Fn variable");
        }
        let after = p + token_size(self.code, p);
        if array && self.rd(after) == TK_PAR1 {
            let mut q = after + 2;
            let mut idx = Vec::new();
            loop {
                let (e, nq) = self.expr(q)?;
                if !e.ty.is_num() {
                    return Err("type mismatch");
                }
                idx.push(e);
                q = nq;
                match self.rd(q) {
                    TK_COMMA => q += 2,
                    TK_PAR2 => break,
                    _ => return Err("syntax"),
                }
            }
            return Ok((LValue::Elem { slot, ty, idx }, q + 2));
        }
        if array {
            // An array used without indices reads as zero: rare, leave it
            // to the interpreter.
            return Err("array without index");
        }
        Ok((LValue::Scalar { slot, ty }, after))
    }

    fn operand_value(&self, p: usize) -> Res<(Expr, usize)> {
        let t = self.rd(p);
        match t {
            TK_VAR => {
                let (lv, q) = self.var_ref(p)?;
                let e = match lv {
                    LValue::Scalar { slot, ty } => Expr::new(ExprKind::Var(slot), Ty::of_var(ty)),
                    LValue::Elem { slot, ty, idx } => Expr::new(ExprKind::Elem(slot, idx), Ty::of_var(ty)),
                };
                Ok((e, q))
            }
            TK_ENT | TK_HEX | TK_BIN => {
                Ok((Expr::new(ExprKind::Int(read_u32(self.code, p + 2) as i32), Ty::Int), p + 6))
            }
            TK_FL => Ok((Expr::new(ExprKind::Float(ffp_to_f64(read_u32(self.code, p + 2))), Ty::Float), p + 6)),
            TK_DFL => {
                let mut b = [0u8; 8];
                b.copy_from_slice(&self.code[p + 2..p + 10]);
                let x = f64::from_be_bytes(b);
                // `round_float` at run time.
                let x = if self.prg.double { x } else { amos_core::ffp::Ffp::from_f64(x).to_f64() };
                Ok((Expr::new(ExprKind::Float(x), Ty::Float), p + 10))
            }
            TK_CH1 | TK_CH2 => Ok((Expr::new(ExprKind::Str(p), Ty::Str), p + token_size(self.code, p))),
            TK_PAR1 => {
                let (e, q) = self.expr(p + 2)?;
                Ok((e, self.expect(q, TK_PAR2)?))
            }
            TK_NOT => {
                let (e, q) = self.expr(p + 2)?;
                if !e.ty.is_num() {
                    return Err("type mismatch");
                }
                Ok((Expr::new(ExprKind::Not(Box::new(e)), Ty::Int), q))
            }
            // Punctuation as an operand: omitted parameter.
            TK_COMMA | TK_PAR2 | TK_TO | TK_EOL | TK_DP => Ok((Expr::new(ExprKind::Int(i32::MIN), Ty::Int), p)),
            _ => self.function(p),
        }
    }

    /// A function call, evaluated through the keyword bridge.
    fn function(&self, p: usize) -> Res<(Expr, usize)> {
        let t = self.rd(p);
        if t == tk::VARPTR || t == tk::ARRAY {
            return Err("Varptr / Array");
        }
        let call = structure::function_call(self.code, p).ok_or("special function syntax")?;
        let core = call.kw.slot == 0 && is_core_function(call.kw.token);
        let mut args = Vec::new();
        for s in call.slots.iter().filter(|s| s.present) {
            let (e, q) = self.expr(s.start)?;
            if q != s.end {
                return Err("parameter boundaries");
            }
            args.push(e);
        }
        let ty = if core { self.core_type(call.kw.token, &args)? } else { self.keyword_type(call.kw)? };
        if core && let Some(nf) = native_function(call.kw.token, &args) {
            return Ok((Expr::new(ExprKind::Native(nf, args), ty), call.end));
        }
        Ok((Expr::new(ExprKind::Call(p, args), ty), call.end))
    }

    fn is_bare_var(&self, a: usize, b: usize) -> bool {
        self.rd(a) == TK_VAR && a + token_size(self.code, a) == b
    }

    /// Result type of a core function (as `core_function` returns it).
    fn core_type(&self, t: u16, args: &[Expr]) -> Res<Ty> {
        use tk::*;
        let arg = |i: usize| args.get(i).map_or(Ty::Int, |e| e.ty);
        Ok(match t {
            LEN | ASC | INSTR | INSTR_2 | SGN | RND | BTST | PARAM | TRUE | FALSE | ERRN | ERRTRAP => Ty::Int,
            PARAM_F | PI_F | SQR | LOG | LN | EXP | SIN | COS | TAN | ASIN | ACOS | ATAN | HSIN | HCOS | HTAN => {
                Ty::Float
            }
            CHR_S | STR_S | UPPER_S | LOWER_S | FLIP_S | SPACE_S | STRING_S | REPEAT_S | LEFT_S | RIGHT_S | MID_S
            | MID_S_2 | HEX_S | HEX_S_2 | BIN_S | BIN_S_2 | ERR_S | PARAM_S => Ty::Str,
            ABS | INT => arg(0),
            MIN | MAX => {
                let (a, b) = (arg(0), arg(1));
                if a.is_num() != b.is_num() {
                    return Err("type mismatch");
                }
                compat(a, b)
            }
            VAL => Ty::Dyn,
            _ => return Err("function"),
        })
    }

    /// Result type of a function of the subsystems.
    fn keyword_type(&self, kw: Keyword) -> Res<Ty> {
        let def = structure::keyword_def(kw).ok_or("unknown function")?;
        Ok(match def.kind() {
            TokenKind::Function(ValueType::Str) => Ty::Str,
            TokenKind::ReservedVariable if def.params.as_bytes().get(1) == Some(&b'2') => Ty::Str,
            TokenKind::Function(_) | TokenKind::ReservedVariable | TokenKind::Constant => Ty::Dyn,
            _ => return Err("not a function"),
        })
    }

    fn num_expr(&self, p: usize) -> Res<(Expr, usize)> {
        let (e, q) = self.expr(p)?;
        if !e.ty.is_num() {
            return Err("type mismatch");
        }
        Ok((e, q))
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    /// Lowers one instruction; `Err` means it is left to the interpreter.
    pub fn stmt(&self, ins: &Instr) -> Res<Stmt> {
        let p = ins.pos;
        let t = self.rd(p);
        if self.uses_resident(ins) {
            return Err("variable mapped to memory");
        }
        let st = match t {
            TK_VAR => {
                let (lv, q) = self.var_ref(p)?;
                let q = self.expect(q, tk::OP_EQ)?;
                let (e, q) = self.expr(q)?;
                if (lv.ty() == 2) != (e.ty == Ty::Str) {
                    return Err("type mismatch");
                }
                self.check_end(q, ins)?;
                Stmt::Assign(lv, e)
            }
            TK_LAB | TK_REM1 | TK_REM2 => Stmt::Nop,
            TK_PRO => self.proc_call(p, ins)?,
            TK_EXT => self.keyword(p, ins)?,
            TK_FOR => {
                let (lv, q) = self.var_ref(p + 4)?;
                if lv.ty() == 2 {
                    return Err("type mismatch");
                }
                if !matches!(lv, LValue::Scalar { .. }) {
                    return Err("For on an array element");
                }
                let q = self.expect(q, tk::OP_EQ)?;
                let (start, q) = self.num_expr(q)?;
                let q = self.expect(q, TK_TO)?;
                let (limit, mut q) = self.num_expr(q)?;
                let mut step = None;
                if self.rd(q) == tk::STEP {
                    let (e, nq) = self.num_expr(q + 2)?;
                    step = Some(e);
                    q = nq;
                }
                self.check_end(q, ins)?;
                let body = self.statement_start(q);
                let exit = self.field_target(p + 2);
                Stmt::For { lv, start, limit, step, body, exit }
            }
            TK_REPEAT | TK_DO => {
                let exit = self.field_target(p + 2);
                let body = self.statement_start(p + 4);
                Stmt::LoopStart { do_loop: t == TK_DO, body, exit }
            }
            TK_WHILE => {
                let exit = self.field_target(p + 2);
                let (cond, q) = self.num_expr(p + 4)?;
                self.check_end(q, ins)?;
                let body = self.statement_start(q);
                Stmt::While { cond, body, exit }
            }
            tk::NEXT => Stmt::Next,
            tk::UNTIL => {
                let (cond, q) = self.num_expr(p + 2)?;
                self.check_end(q, ins)?;
                Stmt::Until(cond)
            }
            tk::WEND => Stmt::Wend,
            tk::LOOP => Stmt::Loop,
            TK_EXIT | TK_EXIT_IF => {
                let frames = self.rd(p + 4);
                let target = self.field_target(p + 2);
                let cond = if t == TK_EXIT_IF {
                    let (c, _) = self.num_expr(p + 6)?;
                    Some(c)
                } else {
                    None
                };
                Stmt::Exit { cond, frames, target }
            }
            TK_IF => self.if_stmt(p)?,
            TK_ELSE | TK_ELSE_IF => Stmt::Jump(self.prg.else_exit.get(&p).copied().unwrap_or(p + 4)),
            tk::END_IF => Stmt::Nop,
            tk::GOTO => match self.rd(p + 2) {
                TK_LGO => Stmt::Goto(self.rd(p + 4)),
                _ => return Err("computed Goto"),
            },
            tk::GOSUB => match self.rd(p + 2) {
                TK_LGO => {
                    let ret = p + 2 + token_size(self.code, p + 2);
                    Stmt::Gosub { label: self.rd(p + 4), ret }
                }
                _ => return Err("computed Gosub"),
            },
            tk::RETURN => Stmt::Return,
            tk::PROC => self.proc_call(p + 2, ins)?,
            TK_PROCEDURE => {
                let pr = self.prg.procs.iter().find(|pr| pr.pos == p).ok_or("procedure")?;
                Stmt::Jump(structure::to_eol(self.code, pr.end_pos))
            }
            TK_END_PROC | tk::POP_PROC => {
                let q = p + 2;
                let value = if self.rd(q) == TK_BRA1 {
                    let (e, nq) = self.expr(q + 2)?;
                    self.expect(nq, TK_BRA2)?;
                    Some(e)
                } else {
                    None
                };
                Stmt::EndProc { pop: t == tk::POP_PROC, value }
            }
            TK_DATA | tk::SHARED | tk::GLOBAL => Stmt::Nop,
            tk::PRINT => self.print(p + 2, ins)?,
            tk::INC | tk::DEC => match self.var_ref(p + 2)? {
                (LValue::Scalar { slot, ty }, q) if ty != 2 => {
                    self.check_end(q, ins)?;
                    Stmt::IncDec { slot, ty, inc: t == tk::INC }
                }
                _ => return Err("Inc / Dec"),
            },
            tk::ADD | tk::ADD_2 => {
                let (LValue::Scalar { slot, ty }, q) = self.var_ref(p + 2)? else {
                    return Err("Add on an array element");
                };
                if ty == 2 {
                    return Err("type mismatch");
                }
                let q = self.expect(q, TK_COMMA)?;
                let (n, mut q) = self.num_expr(q)?;
                let mut range = None;
                if self.rd(q) == TK_COMMA {
                    let (a, nq) = self.num_expr(q + 2)?;
                    let nq = self.expect(nq, TK_TO)?;
                    let (b, nq) = self.num_expr(nq)?;
                    range = Some((a, b));
                    q = nq;
                }
                self.check_end(q, ins)?;
                Stmt::Add { slot, ty, n, range }
            }
            tk::WAIT => {
                let (n, q) = self.num_expr(p + 2)?;
                self.check_end(q, ins)?;
                Stmt::Wait(Some(n))
            }
            tk::WAIT_VBL => {
                self.check_end(p + 2, ins)?;
                Stmt::Wait(None)
            }
            _ if is_core_instruction(t) => return Err("core instruction"),
            _ => self.keyword(p, ins)?,
        };
        Ok(st)
    }

    fn check_end(&self, q: usize, ins: &Instr) -> Res<()> {
        if q == ins.end { Ok(()) } else { Err("instruction boundaries") }
    }

    fn proc_call(&self, p: usize, ins: &Instr) -> Res<Stmt> {
        let t = self.rd(p);
        if t != TK_PRO && t != TK_VAR {
            return Err("syntax");
        }
        let proc = self.rd(p + 2) as usize;
        let pr = self.prg.procs.get(proc).ok_or("procedure")?;
        if pr.machine_code {
            return Err("machine code procedure");
        }
        let mut q = p + token_size(self.code, p);
        let mut args = Vec::new();
        if self.rd(q) == TK_BRA1 {
            q += 2;
            loop {
                let (e, nq) = self.expr(q)?;
                args.push(e);
                q = nq;
                match self.rd(q) {
                    TK_COMMA => q += 2,
                    TK_BRA2 => {
                        q += 2;
                        break;
                    }
                    _ => return Err("syntax"),
                }
            }
        }
        self.check_end(q, ins)?;
        Ok(Stmt::Call { proc, args, ret: q })
    }

    /// An instruction of the subsystems, through the keyword bridge.
    fn keyword(&self, p: usize, ins: &Instr) -> Res<Stmt> {
        let (kw, _) = structure::keyword_at(self.code, p);
        let def = structure::keyword_def(kw).ok_or("unknown keyword")?;
        if def.kind() == TokenKind::ReservedVariable {
            return Err("reserved variable assignment");
        }
        if kw.slot == 0 && (is_core_instruction(kw.token) || is_special_class(def.verif[0])) {
            return Err("special syntax");
        }
        let call = structure::instruction_call(self.code, p).ok_or("special syntax")?;
        if call.end != ins.end {
            return Err("instruction boundaries");
        }
        let mut args = Vec::new();
        for s in call.slots.iter().filter(|s| s.present) {
            if takes_var_by_reference(kw) && self.is_bare_var(s.start, s.end) {
                return Err("variable parameter");
            }
            let (e, q) = self.expr(s.start)?;
            if q != s.end {
                return Err("parameter boundaries");
            }
            args.push(e);
        }
        Ok(Stmt::Keyword(args))
    }

    fn print(&self, mut q: usize, ins: &Instr) -> Res<Stmt> {
        let mut items = Vec::new();
        let mut newline = true;
        loop {
            let t = self.rd(q);
            if structure::is_end(t) {
                break;
            }
            newline = true;
            match t {
                TK_SEMI => {
                    q += 2;
                    newline = false;
                }
                TK_COMMA => {
                    q += 2;
                    items.push(PrintItem::Tab);
                    newline = false;
                }
                _ if t == tk::USING => return Err("Print Using"),
                _ => {
                    let (e, nq) = self.expr(q)?;
                    if nq == q {
                        return Err("syntax");
                    }
                    items.push(PrintItem::Value(e));
                    q = nq;
                }
            }
        }
        self.check_end(q, ins)?;
        Ok(Stmt::Print { items, newline })
    }

    /// If / Else If chain (`Interp::exec_if`).
    fn if_stmt(&self, p: usize) -> Res<Stmt> {
        let mut field = p + 2;
        let mut q = p + 4;
        let mut branches = Vec::new();
        loop {
            let (cond, mut nq) = self.num_expr(q)?;
            let mut action = IfTrue::At(nq);
            if self.rd(nq) == tk::THEN {
                nq += 2;
                action = IfTrue::At(nq);
                if self.rd(nq) == TK_LGO {
                    action = IfTrue::Label(self.rd(nq + 2));
                }
            }
            branches.push((cond, action));
            let d = self.rd(field) as usize;
            let target = field + (d & !1);
            if d & 1 != 0 {
                field = target - 2;
                q = target;
                if branches.len() > 1000 {
                    return Err("If chain");
                }
                continue;
            }
            let false_label = if self.rd(target) == TK_LGO { Some(self.rd(target + 2)) } else { None };
            return Ok(Stmt::If { branches, false_target: target, false_label });
        }
    }
}

/// Core functions compiled natively for these parameter types.
fn native_function(t: u16, args: &[Expr]) -> Option<Nf> {
    use tk::*;
    let ty = |i: usize| args.get(i).map(|e| e.ty);
    let num = |i: usize| ty(i).is_some_and(|t| t.is_num());
    let s = |i: usize| ty(i) == Some(Ty::Str);
    let known = |i: usize| matches!(ty(i), Some(Ty::Int | Ty::Float));
    Some(match t {
        LEN if s(0) => Nf::Len,
        ASC if s(0) => Nf::Asc,
        CHR_S if num(0) => Nf::Chr,
        LEFT_S if s(0) && num(1) => Nf::Left,
        RIGHT_S if s(0) && num(1) => Nf::Right,
        MID_S if s(0) && num(1) && num(2) => Nf::Mid3,
        MID_S_2 if s(0) && num(1) => Nf::Mid2,
        STR_S if known(0) => Nf::Str,
        INSTR if s(0) && s(1) => Nf::Instr2,
        INSTR_2 if s(0) && s(1) && num(2) => Nf::Instr3,
        UPPER_S if s(0) => Nf::Upper,
        LOWER_S if s(0) => Nf::Lower,
        ABS if known(0) => Nf::Abs,
        INT if known(0) => Nf::Int,
        SGN if known(0) => Nf::Sgn,
        MIN | MAX if args.len() == 2 && args.iter().all(|a| a.ty != Ty::Dyn) => {
            if t == MAX {
                Nf::Max
            } else {
                Nf::Min
            }
        }
        TRUE => Nf::True,
        FALSE => Nf::False,
        PARAM => Nf::Param,
        PARAM_F => Nf::ParamF,
        PARAM_S => Nf::ParamS,
        PI_F => Nf::Pi,
        _ => return None,
    })
}

/// Lowers every instruction; also returns why some are interpreted.
pub fn lower_all(prg: &Compiled, instrs: &[Instr]) -> (Vec<Stmt>, Vec<(usize, Unsupported)>) {
    let l = Lower::new(prg, instrs);
    let mut out = Vec::with_capacity(instrs.len());
    let mut interpreted = Vec::new();
    for ins in instrs {
        match l.stmt(ins) {
            Ok(s) => out.push(s),
            Err(why) => {
                interpreted.push((ins.pos, why));
                out.push(Stmt::Interp);
            }
        }
    }
    (out, interpreted)
}
