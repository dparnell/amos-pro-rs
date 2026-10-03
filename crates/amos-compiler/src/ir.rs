//! Intermediate representation: one statement per instruction of the
//! program (see `amos_core::compiled::structure::instructions`), with typed
//! expression trees.

/// Static type of an expression. `Dyn` is a number whose type is only known
/// at run time (`Val`, functions of the subsystems): it is carried as an
/// `f64` payload and an `i32` tag (0 integer, 1 float).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Int,
    Float,
    Str,
    Dyn,
}

impl Ty {
    pub fn of_var(t: u8) -> Ty {
        match t {
            1 => Ty::Float,
            2 => Ty::Str,
            _ => Ty::Int,
        }
    }

    pub fn is_num(self) -> bool {
        self != Ty::Str
    }
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Ty,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i32),
    Float(f64),
    /// String constant: position of its `TK_CH1` / `TK_CH2` token.
    Str(usize),
    /// Scalar variable (slot reference of the verified token).
    Var(u16),
    /// Array element.
    Elem(u16, Vec<Expr>),
    Neg(Box<Expr>),
    Not(Box<Expr>),
    Bin(u16, Box<Expr>, Box<Expr>),
    /// Function evaluated by the keyword bridge: token position and the
    /// values of its present parameters.
    Call(usize, Vec<Expr>),
    /// Core function compiled natively (same semantics as
    /// `Interp::core_function`).
    Native(Nf, Vec<Expr>),
    /// `Fn name(args)`: the Fn slot, the arguments, and each `Def Fn` of
    /// the scope that may be current: (position of its parameter list,
    /// parameters, expression).
    FnCall {
        slot: u16,
        args: Vec<Expr>,
        defs: Vec<(usize, Vec<LValue>, Expr)>,
    },
    /// `Match(a(i..), value)`: array slot and type, the (ignored) indices,
    /// the value.
    Match {
        slot: u16,
        ty: u8,
        idx: Vec<Expr>,
        value: Box<Expr>,
    },
}

/// Natively compiled core functions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nf {
    /// `Bset`/`Bclr`/`Bchg`/`Ror.x`/`Rol.x n,v` on an integer variable
    /// (the token): the new value from `n` and the current value.
    BitOp(u16),
    Len,
    Asc,
    Chr,
    Left,
    Right,
    Mid2,
    Mid3,
    Str,
    Instr2,
    Instr3,
    Upper,
    Lower,
    Flip,
    StringS,
    Space,
    Val,
    Hex,
    Bin,
    Repeat,
    Abs,
    Int,
    Sgn,
    Min,
    Max,
    True,
    False,
    Param,
    ParamF,
    ParamS,
    Pi,
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Ty) -> Expr {
        Expr { kind, ty }
    }
}

/// Target of an assignment.
#[derive(Clone, Debug)]
pub enum LValue {
    Scalar { slot: u16, ty: u8 },
    Elem { slot: u16, ty: u8, idx: Vec<Expr> },
}

impl LValue {
    pub fn ty(&self) -> u8 {
        match self {
            LValue::Scalar { ty, .. } | LValue::Elem { ty, .. } => *ty,
        }
    }

    pub fn slot(&self) -> u16 {
        match self {
            LValue::Scalar { slot, .. } | LValue::Elem { slot, .. } => *slot,
        }
    }
}

#[derive(Clone, Debug)]
pub enum PrintItem {
    Value(Expr),
    Tab,
}

/// What a true If / Else If condition does.
#[derive(Clone, Debug)]
pub enum IfTrue {
    /// Continue at this position (after the condition / Then).
    At(usize),
    /// `Then label`: jump to label number n of the scope.
    Label(u16),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Nop,
    /// Jump to a position without a test point (Else reached in sequence,
    /// Procedure skipped...).
    Jump(usize),
    Assign(LValue, Expr),
    Print {
        items: Vec<PrintItem>,
        newline: bool,
    },
    If {
        branches: Vec<(Expr, IfTrue)>,
        false_target: usize,
        false_label: Option<u16>,
    },
    For {
        lv: LValue,
        start: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: usize,
        exit: usize,
    },
    Next,
    /// Repeat (`do_loop` false) or Do.
    LoopStart {
        do_loop: bool,
        body: usize,
        exit: usize,
    },
    While {
        cond: Expr,
        body: usize,
        exit: usize,
    },
    Until(Expr),
    Wend,
    Loop,
    Exit {
        cond: Option<Expr>,
        frames: u16,
        target: usize,
    },
    Goto(u16),
    Gosub {
        label: u16,
        ret: usize,
    },
    Return,
    /// `On n Goto/Gosub/Proc a,b,...`: the keyword token, the label numbers
    /// (or procedure numbers), the position after the list.
    On {
        n: Expr,
        kind: u16,
        targets: Vec<OnTarget>,
        after: usize,
    },
    /// `Goto expr` / `Gosub expr` (label name or line number at run time).
    GotoExpr {
        e: Expr,
        gosub: bool,
        ret: usize,
    },
    /// `Restore` / `Restore label`.
    Restore(Option<u16>),
    /// `Read a,b...` (all the Data items of the scope are constants).
    Read(Vec<LValue>),
    Call {
        proc: usize,
        args: Vec<Expr>,
        ret: usize,
    },
    EndProc {
        pop: bool,
        value: Option<Expr>,
    },
    IncDec {
        lv: LValue,
        inc: bool,
    },
    /// `Add v,n` / `Add v,n,a To b`.
    Add {
        lv: LValue,
        n: Expr,
        range: Option<(Expr, Expr)>,
    },
    Swap(LValue, LValue),
    /// `Dim a(..), b(..)`: slot, type and maximum indices of each array.
    Dim(Vec<(u16, u8, Vec<Expr>)>),
    /// `Sort a(..)`: slot, (ignored) indices.
    Sort {
        slot: u16,
        idx: Vec<Expr>,
    },
    /// `Wait n` / `Wait Vbl` (None).
    Wait(Option<Expr>),
    /// `Mid$(a$,p,n)=e$`, `Mid$(a$,p)=`, `Left$(a$,n)=`, `Right$(a$,n)=`:
    /// the keyword token, the string variable, the numbers, the value.
    MidAssign {
        kind: u16,
        lv: LValue,
        nums: Vec<Expr>,
        e: Expr,
    },
    /// Instruction of a subsystem run through the keyword bridge.
    Keyword(Vec<Expr>),
    /// Instruction run by the interpreter (with its variables copied).
    Interp,
}

/// An entry of `On n Goto/Gosub/Proc`.
#[derive(Clone, Debug)]
pub enum OnTarget {
    /// A label of the scope (Goto / Gosub) or a procedure (Proc).
    Label(u16),
    /// A label computed when the entry is chosen (`Interp::label_target`:
    /// a number or a name).
    Expr(Expr),
}
