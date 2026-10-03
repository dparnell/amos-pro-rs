//! Interpreter vs compiled timings (run with `--release`).
//!
//! `cargo run --release -p amos-wasmhost --example bench`

use std::time::{Duration, Instant};

use amos_core::compiled::Env;
use amos_core::interp::stmt::InputState;
use amos_core::interp::value::Value;
use amos_core::interp::{Exc, Host, Interp, R, RunState};
use amos_core::tokens::Keyword;
use amos_wasmhost::CompiledProgram;

#[derive(Default)]
struct Quiet {
    out: Vec<u8>,
}

impl Host for Quiet {
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
        Ok(Some(Vec::new()))
    }
}

struct M {
    it: Interp,
    host: Quiet,
}

impl Env for M {
    fn parts(&mut self) -> (&mut Interp, &mut dyn Host) {
        (&mut self.it, &mut self.host)
    }
}

const BUDGET: usize = 200_000;

fn machine(prg: &amos_core::Program) -> M {
    let mut it = Interp::new();
    it.load(prg).unwrap();
    M { it, host: Quiet::default() }
}

fn interpreted(prg: &amos_core::Program) -> (Duration, String) {
    let mut m = machine(prg);
    let t = Instant::now();
    loop {
        m.it.vbl();
        match m.it.run(&mut m.host, BUDGET) {
            RunState::Running => {}
            RunState::Stopped(i) => {
                assert!(matches!(i.reason, amos_core::interp::StopReasonOrError::Stop(_)), "{:?}", i);
                break;
            }
            RunState::Idle => break,
        }
    }
    (t.elapsed(), String::from_utf8_lossy(&m.host.out).into_owned())
}

fn compiled(prg: &amos_core::Program, wasm: &[u8]) -> (Duration, Duration, String) {
    let mut m = machine(prg);
    let t0 = Instant::now();
    let mut cp = CompiledProgram::new(wasm, m.it.prg.clone().unwrap()).unwrap();
    let setup = t0.elapsed();
    let t = Instant::now();
    loop {
        m.it.vbl();
        match cp.run(&mut m, BUDGET) {
            RunState::Running => {}
            RunState::Stopped(i) => {
                assert!(matches!(i.reason, amos_core::interp::StopReasonOrError::Stop(_)), "{:?}", i);
                break;
            }
            RunState::Idle => break,
        }
    }
    (setup, t.elapsed(), String::from_utf8_lossy(&m.host.out).into_owned())
}

const PROGRAMS: &[(&str, &str)] = &[
    ("integer loop", "A=0\nFor I=1 To 2000000\nA=(A+I*3) mod 1000\nNext I\nPrint A"),
    (
        "nested loops + If",
        "C=0\nFor I=1 To 1000\nFor J=1 To 1000\nIf (I+J) mod 3=0 Then Inc C\nNext J\nNext I\nPrint C",
    ),
    ("while / repeat", "I=0 : S=0\nWhile I<1000000\nInc I : S=S+(I and 7)\nWend\nRepeat\nDec I\nUntil I=0\nPrint S"),
    ("float single", "X#=0\nFor I=1 To 300000\nX#=X#+0.5*I\nNext\nPrint X#"),
    ("float double", "Set Double Precision\nX#=0\nFor I=1 To 300000\nX#=X#+0.5*I\nNext\nPrint X#"),
    (
        "string building",
        "A$=\"\" : N=0\nFor I=1 To 200000\nA$=A$+Chr$(65+I mod 26)\nIf Len(A$)>50 Then A$=Mid$(A$,10) : Inc N\nNext\nPrint N;A$",
    ),
    (
        "string ops",
        "N=0 : A$=\"The quick brown fox jumps over the lazy dog\"\nFor I=1 To 100000\nB$=Mid$(A$,I mod 40+1,5)\nIf B$>\"m\" Then Inc N\nC$=Left$(A$,10)+Right$(A$,5)-\"o\"\nN=N+Instr(A$,\"o\",I mod 30+1)+Len(C$)+Asc(B$)\nD$=Upper$(B$)+Chr$(65+I mod 26)+Flip$(B$)+String$(\"*\",3)+Space$(2)\nIf D$=B$ Then Inc N\nNext\nPrint N;D$",
    ),
    (
        "val / str$ / hex$",
        "S=0 : F#=0\nFor I=1 To 50000\nA$=Str$(I)+\".25\"\nF#=F#+Val(A$)\nS=(S+Val(Str$(I*3))+Len(Hex$(I))+Len(Bin$(I,16))+Val(Hex$(I))) mod 100000\nB$=Str$(F#/7)\nNext\nPrint S;F#;B$",
    ),
    (
        "val / str$ double",
        "Set Double Precision\nS=0 : F#=0\nFor I=1 To 50000\nA$=Str$(I)+\".25\"\nF#=F#+Val(A$)\nS=(S+Val(Str$(I*3))+Len(Hex$(I,8))) mod 100000\nB$=Str$(F#/7)\nNext\nPrint S;F#;B$",
    ),
    (
        "procedures (fib 24)",
        "FIB[24]\nPrint Param\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]",
    ),
    (
        "arrays (sieve x20)",
        "Dim F(40000)\nFor R=1 To 20\nC=0\nFor I=2 To 20000 : F(I)=0 : Next\nFor I=2 To 20000\nIf F(I)=0\nInc C\nFor J=I+I To 20000 Step I : F(J)=1 : Next J\nEnd If\nNext I\nNext R\nPrint C",
    ),
    ("host calls (Len x1M)", "B$=\"abc\"\nFor I=1 To 1000000 : A=Len(B$) : Next\nPrint A"),
    ("hex$/bin$", "For I=1 To 300000 : A$=Hex$(I) : B$=Bin$(I) : Next : Print A$;B$"),
    ("hex$/bin$ digits", "For I=1 To 300000 : A$=Hex$(-I*977,8) : B$=Bin$(I,32) : Next : Print A$;B$"),
    (
        "hex$/bin$ building",
        "For I=1 To 300000 : N=N+Len(Hex$(I,4)+Bin$(I and 255,8)) : S$=S$+Hex$(I) : If Len(S$)>200 Then S$=\"\"\nNext : Print N;S$",
    ),
    ("gosub", "For I=1 To 300000 : Gosub L : Next : Print A : End\nL: A=A+1 : Return"),
];

fn main() {
    println!("{:<22} {:>12} {:>12} {:>8} {:>10}", "program", "interpreter", "compiled", "speedup", "jit setup");
    for (name, src) in PROGRAMS {
        let prg = amos_core::tokenise::tokenise_program(src.as_bytes()).unwrap();
        let out = amos_compiler::compile_full(&prg).unwrap();
        let (ti, oi) = interpreted(&prg);
        let (setup, tc, oc) = compiled(&prg, &out.wasm);
        assert_eq!(oi, oc, "{name}: outputs differ");
        println!(
            "{:<22} {:>10.1}ms {:>10.1}ms {:>7.1}x {:>8.1}ms{}",
            name,
            ti.as_secs_f64() * 1e3,
            tc.as_secs_f64() * 1e3,
            ti.as_secs_f64() / tc.as_secs_f64(),
            setup.as_secs_f64() * 1e3,
            if out.interpreted.is_empty() {
                String::new()
            } else {
                format!("  ({} interpreted)", out.interpreted.len())
            }
        );
    }
}
