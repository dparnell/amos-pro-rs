//! AMOS Professional to WebAssembly compiler (see `docs/COMPILER.md`).
//!
//! ```text
//! .AMOS ──Verifier──► verified tokens ──lower──► IR ──codegen──► .wasm
//! ```
//!
//! The module runs against the runtime in `amos_core::compiled` (imports
//! `rt.*` / `host.*`), hosted natively by `amos-wasmhost` (wasmtime).

pub mod codegen;
pub mod ir;
pub mod lower;

use amos_core::Program;
use amos_core::compiled::structure;
use amos_core::interp::verify::{Compiled, TestError, Verifier};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileError {
    /// The program does not pass the test (same errors as the interpreter).
    Test(TestError),
    /// A construct the compiled runtime cannot support, at a position.
    Unsupported { pos: usize, what: String },
    /// Bug in the compiler.
    Internal(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Test(e) => {
                write!(f, "{} (at {})", amos_core::errors::test_message(e.code), e.pos)
            }
            CompileError::Unsupported { pos, what } => write!(f, "{what} is not supported by the compiler (at {pos})"),
            CompileError::Internal(m) => write!(f, "internal compiler error: {m}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Result of a compilation.
#[derive(Clone, Debug)]
pub struct Output {
    pub wasm: Vec<u8>,
    /// Number of instructions.
    pub instructions: usize,
    /// Instructions left to the interpreter at run time: (position, reason).
    pub interpreted: Vec<(usize, &'static str)>,
}

/// Compiles a program to a wasm module.
pub fn compile(program: &Program) -> Result<Vec<u8>, CompileError> {
    compile_full(program).map(|o| o.wasm)
}

pub fn compile_full(program: &Program) -> Result<Output, CompileError> {
    let c = Verifier::verify(&program.source, program.math_flags).map_err(CompileError::Test)?;
    compile_verified(&c)
}

/// Compiles an already verified program.
pub fn compile_verified(c: &Compiled) -> Result<Output, CompileError> {
    let instrs = structure::instructions(c);
    check_supported(c, &instrs)?;
    let (stmts, interpreted) = lower::lower_all(c, &instrs);
    let wasm = codegen::module(c, &instrs, &stmts)?;
    Ok(Output { wasm, instructions: instrs.len(), interpreted })
}

/// Constructs that even the interpreter fallback cannot run in a compiled
/// program (none at the moment: Varptr / Field variables stay in the
/// interpreter, see `structure::resident_vars`).
fn check_supported(_c: &Compiled, _instrs: &[structure::Instr]) -> Result<(), CompileError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_src(src: &str) -> Output {
        let prg = amos_core::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let o = compile_full(&prg).unwrap();
        wasmparser::Validator::new().validate_all(&o.wasm).expect("valid wasm");
        o
    }

    #[test]
    fn core_constructs_are_native() {
        let o = compile_src(
            "A=1 : B#=2.5 : C$=\"x\"+Str$(A)\nDim T(10)\nFor I=0 To 10 : T(I)=I*I : Next\nWhile A<10 : Inc A : Wend\nIf A=10 Then Print C$ Else Print B#\nP[A]\nProcedure P[N]\nEnd Proc[N*2]",
        );
        let interpreted: Vec<_> = o.interpreted.iter().map(|x| x.1).collect();
        // Only Dim is left to the interpreter.
        assert_eq!(interpreted, ["core instruction"]);
    }

    #[test]
    fn test_errors_are_reported() {
        let prg = amos_core::tokenise::tokenise_program(b"A=\"x\"").unwrap();
        assert!(matches!(compile(&prg), Err(CompileError::Test(_))));
    }

    #[test]
    fn varptr_variables_stay_in_the_interpreter() {
        let o = compile_src("A=5\nP=Varptr(A)\nB=A+1\nPrint B");
        assert!(o.interpreted.iter().any(|x| x.1 == "variable mapped to memory"));
    }
}
