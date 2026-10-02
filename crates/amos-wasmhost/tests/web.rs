//! The web backend, run by `wasm-bindgen-test-runner` (node):
//! `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
//! cargo test -p amos-wasmhost --target wasm32-unknown-unknown --test web`.
//! Programs are compiled, instantiated with `WebAssembly.instantiate` and
//! must behave as with the interpreter (also running in wasm).

#![cfg(target_arch = "wasm32")]

use amos_core::compiled::Env;
use amos_core::interp::stmt::InputState;
use amos_core::interp::value::Value;
use amos_core::interp::{Exc, Host, Interp, R, RunState, StopReasonOrError};
use amos_core::tokens::Keyword;
use amos_wasmhost::CompiledProgram;
use wasm_bindgen_test::*;

#[derive(Default)]
struct TextHost {
    out: Vec<u8>,
}

impl Host for TextHost {
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        let _ = it.inst_args(self, kw)?;
        Ok(())
    }
    fn function(&mut self, _: &mut Interp, _: Keyword) -> R<Value> {
        Err(Exc::Message("no functions".into()))
    }
    fn reserved_assign(&mut self, _: &mut Interp, _: Keyword) -> R<()> {
        Ok(())
    }
    fn test_point(&mut self, _: &mut Interp) -> R<()> {
        Ok(())
    }
    fn take_break(&mut self) -> bool {
        false
    }
    fn print(&mut self, _: &mut Interp, text: &[u8]) -> R<()> {
        self.out.extend_from_slice(text);
        Ok(())
    }
    fn read_line(&mut self, _: &mut Interp, _: &mut InputState) -> R<Option<Vec<u8>>> {
        Ok(Some(b"7".to_vec()))
    }
}

struct M {
    it: Interp,
    host: TextHost,
}

impl Env for M {
    fn parts(&mut self) -> (&mut Interp, &mut dyn Host) {
        (&mut self.it, &mut self.host)
    }
}

fn machine(src: &str) -> M {
    let prg = amos_core::tokenise::tokenise_program(src.as_bytes()).unwrap();
    let mut it = Interp::new();
    it.load(&prg).unwrap();
    M { it, host: TextHost::default() }
}

/// (output, end, frames)
fn interpreted(src: &str, budget: usize) -> (String, StopReasonOrError, usize) {
    let mut m = machine(src);
    for f in 0..1_000_000 {
        m.it.vbl();
        if let RunState::Stopped(i) = m.it.run(&mut m.host, budget) {
            return (String::from_utf8_lossy(&m.host.out).into_owned(), i.reason, f);
        }
    }
    panic!("did not end: {src}")
}

async fn compiled(src: &str, budget: usize) -> (String, StopReasonOrError, usize) {
    let mut m = machine(src);
    let prg = amos_core::tokenise::tokenise_program(src.as_bytes()).unwrap();
    let wasm = amos_compiler::compile(&prg).unwrap();
    let mut cp = CompiledProgram::new_async(&wasm, m.it.prg.clone().unwrap()).await.unwrap();
    for f in 0..1_000_000 {
        m.it.vbl();
        if let RunState::Stopped(i) = cp.run(&mut m, budget) {
            return (String::from_utf8_lossy(&m.host.out).into_owned(), i.reason, f);
        }
    }
    panic!("did not end: {src}")
}

const PROGRAMS: &[&str] = &[
    "N=0\nGosub R\nEnd\nR: Inc N : Print N; : Gosub R : Return",
    "For I=1 To 3 : Gosub L : Print I; : Next\nOn Error Goto H\nGosub S\nPrint T\nEnd\nL: For J=1 To 5 : If J=2 Then Return\nNext J : Return\nS: T=1/0 : Return\nH: T=7 : Resume Next",
    "For F=-17 To 17 : Fix F : Print Str$(3.14159);Str$(-0.00012345);Str$(1E+20) : Next : Fix 16\nFor I=1 To 200 : X#=I*1.37-150 : Print Str$(X#*X#/97);Val(Str$(X#)); : Next\nPrint Val(\"$FF\");Val(\"%101\");Val(\"1 2.5e 2\");Val(\"x\");Hex$(-1,4);Bin$(5);Repeat$(\"a\",2)",
    "Set Double Precision\nFor F=-17 To 17 : Fix F : Print Str$(3.14159);Str$(-0.00012345);Str$(1E+20) : Next : Fix 16\nFor I=1 To 200 : X#=I*1.37-150 : Print Str$(X#*X#/97);Val(Str$(X#)); : Next\nPrint Val(\"12345.678e-2\");Val(\"1e40\");Val(\"-0.0\")",
    "For I=1 To 300000\nA$=Str$(I)+\"-\"+Upper$(\"abc\")+Space$(I mod 50)\nNext\nPrint A$",
    "Dim S$(300)\nFor I=0 To 300 : S$(I)=\"k\"+Str$(I) : Next\nFor J=1 To 60\nFor I=0 To 300 : T$=S$(I)+String$(\"x\",200) : Next\nNext\nSort S$(0)\nPrint S$(0);S$(300);Match(S$(0),\"k 150\")",
    "R[\"\",10]\nPrint Len(Param$);Left$(Param$,30)\nProcedure R[S$,N]\nT$=S$+Chr$(65+N)\nIf N=0 Then Pop Proc[T$]\nFor I=1 To 300 : G$=T$+Str$(I) : Next\nR[T$,N-1]\nEnd Proc[Param$+T$]",
    "A$=\"Hello\" : Mid$(A$,2,3)=\"XYZW\" : Left$(A$,1)=\"j\" : Right$(A$,2)=\"!?\" : Print A$;\"aXbXc\"-\"X\";Flip$(A$);Instr(A$,\"Z\");String$(A$,3);Lower$(A$)",
    "For I=1 To 200\nA$=Space$(60000)+Str$(I)\nB$=Right$(A$,4)\nNext\nPrint B$;Len(A$)",
    "For I=1 To 3 : Print I; : Next I",
    "A$=\"Hello\" : Print Left$(A$,2);Right$(A$,2);Mid$(A$,2,3);Len(A$);Str$(1.5)",
    "FIB[12]\nPrint Param\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]",
    "Dim A(3,3)\nFor I=0 To 3 : For J=0 To 3 : A(I,J)=I*J : Next : Next\nPrint A(3,3);A(2,1)",
    "X#=1 : For I=1 To 30 : X#=X#*1.1 : Next : Print X#",
    "Set Double Precision\nX#=1 : For I=1 To 30 : X#=X#*1.1 : Next : Print X#",
    "On Error Goto H\nA=1/0\nPrint \"after\";E\nEnd\nH: E=Errn : Resume Next",
    "C=0\nEvery 2 Gosub E\nFor I=1 To 10 : Wait Vbl : Next\nEvery Off\nPrint C>2\nEnd\nE: Inc C : Every On : Return",
    "Read A,B$ : Print A;B$\nData 5,\"z\"\nInput N : Print N*2",
    "Gosub L : Print \"b\" : End\nL: Print \"a\" : Return",
    "A=Val(\"2.5\")*2 : Print A;Val(\"3\")+1",
    "N=0\nFor I=1 To 20000\nA$=Str$(I)+\"-\"\nIf Len(A$)>2 Then Inc N\nNext\nPrint N",
    "For K=1 To 200 : P[K] : Next : Print Param\nProcedure P[N]\nDim BIG(2000),S$(100)\nBIG(2000)=N : S$(100)=Str$(N)\nEnd Proc[BIG(2000)+Len(S$(100))]",
    "Dim S$(4),A(9)\nS$(0)=\"d\" : S$(1)=\"b\" : S$(2)=\"a\"\nSort S$(0) : Print S$(2);S$(3);S$(4);Match(S$(0),\"b\")\nFor I=0 To 9 : A(I)=9-I : Next : Sort A(0) : Inc A(3) : Swap A(0),A(9) : Print A(0);A(3);Match(A(0),5)",
    "Dim A(60000),B#(60000)\nA(60000)=1 : B#(60000)=2.5 : Print A(60000)+B#(60000)",
];

#[wasm_bindgen_test]
async fn web_host_matches_interpreter() {
    for p in PROGRAMS {
        // (With 3 instructions per frame an Every handler never ends.)
        let budgets: &[usize] = if p.contains("Every") { &[100_000, 50] } else { &[100_000, 3] };
        for &budget in budgets {
            let a = interpreted(p, budget);
            let b = compiled(p, budget).await;
            assert_eq!(a, b, "budget {budget}: {p}");
        }
    }
}

#[wasm_bindgen_test]
async fn bad_modules_are_refused() {
    let m = machine("Print 1");
    let other = amos_core::tokenise::tokenise_program(b"Print 2").unwrap();
    let wasm = amos_compiler::compile(&other).unwrap();
    // Compiled from another program: the hash does not match.
    assert!(CompiledProgram::new_async(&wasm, m.it.prg.clone().unwrap()).await.is_err());
    assert!(CompiledProgram::new_async(b"not wasm", m.it.prg.clone().unwrap()).await.is_err());
}
