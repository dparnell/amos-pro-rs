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
        "procedures (fib 24)",
        "FIB[24]\nPrint Param\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]",
    ),
    (
        "arrays (sieve x20)",
        "Dim F(40000)\nFor R=1 To 20\nC=0\nFor I=2 To 20000 : F(I)=0 : Next\nFor I=2 To 20000\nIf F(I)=0\nInc C\nFor J=I+I To 20000 Step I : F(J)=1 : Next J\nEnd If\nNext I\nNext R\nPrint C",
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
