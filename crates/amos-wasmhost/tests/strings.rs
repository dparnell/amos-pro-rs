//! Strings in linear memory: random programs over every string operation
//! (compared with the interpreter, errors included), garbage collection
//! under pressure, strings across yields, procedures and arrays.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use amos_core::interp::{StopReason, StopReasonOrError};
use common::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a>(&mut self, v: &[&'a str]) -> &'a str {
        v[self.below(v.len() as u64) as usize]
    }
}

const LITS: &[&str] =
    &["\"\"", "\"a\"", "\"ab\"", "\"Hello\"", "\"aXbXc\"", "\"zz\"", "\"The Quick\"", "\"x y\"", "\"AAA\""];

fn num(r: &mut Rng, d: u32) -> String {
    if d == 0 || r.below(3) == 0 {
        return match r.below(4) {
            0 => format!("{}", r.below(12) as i64 - 2),
            1 => "N".into(),
            2 => "K".into(),
            _ => format!("{}", r.below(6)),
        };
    }
    match r.below(9) {
        0 => format!("Len({})", st(r, d - 1)),
        1 => format!("Asc({})", st(r, d - 1)),
        2 => format!("Instr({},{})", st(r, d - 1), st(r, d - 1)),
        3 => format!("Instr({},{},{})", st(r, d - 1), st(r, d - 1), num(r, d - 1)),
        4 => format!("({}{}{})", st(r, d - 1), r.pick(&["=", "<>", "<", ">", "<=", ">="]), st(r, d - 1)),
        5 => format!("({}+{})", num(r, d - 1), num(r, d - 1)),
        6 => format!("Max({},{})", num(r, d - 1), num(r, d - 1)),
        7 => format!("Len(Max({},{}))", st(r, d - 1), st(r, d - 1)),
        _ => format!("({} mod 7)", num(r, d - 1)),
    }
}

fn st(r: &mut Rng, d: u32) -> String {
    if d == 0 || r.below(4) == 0 {
        return match r.below(5) {
            0 => "A$".into(),
            1 => "B$".into(),
            2 => "T$(K mod 4)".into(),
            _ => r.pick(LITS).into(),
        };
    }
    let (a, n) = (st(r, d - 1), num(r, d - 1));
    match r.below(17) {
        0 | 1 => format!("({}+{})", a, st(r, d - 1)),
        2 => format!("({}-{})", a, st(r, d - 1)),
        3 => format!("Left$({a},{n})"),
        4 => format!("Right$({a},{n})"),
        5 => format!("Mid$({a},{n})"),
        6 => format!("Mid$({a},{n},{})", num(r, d - 1)),
        7 => format!("Upper$({a})"),
        8 => format!("Lower$({a})"),
        9 => format!("Flip$({a})"),
        10 => format!("String$({a},{n} mod 9)"),
        11 => format!("Space$({n} mod 5)"),
        12 => format!("Chr$(65+({n}) mod 60)"),
        13 => format!("Str$({n})"),
        14 => format!("Max({a},{})", st(r, d - 1)),
        15 => format!("Min({a},{})", st(r, d - 1)),
        _ => format!("Repeat$({a},2)"),
    }
}

fn program(r: &mut Rng, handler: bool) -> String {
    let mut p = String::new();
    if handler {
        p.push_str("On Error Goto H\n");
    }
    p.push_str("Dim T$(3)\nA$=\"start\" : B$=\"bb\" : N=2\n");
    p.push_str("For K=0 To 3\n");
    for _ in 0..6 {
        let line = match r.below(9) {
            0 | 1 => format!("A$={}", st(r, 3)),
            2 => format!("B$={}", st(r, 3)),
            3 => format!("T$(K)={}", st(r, 2)),
            4 => format!("N={}", num(r, 3)),
            5 => format!("Mid$(A$,{},{})={}", num(r, 1), num(r, 1), st(r, 1)),
            6 => format!("{}(B$,{})={}", r.pick(&["Left$", "Right$"]), num(r, 1), st(r, 1)),
            7 => format!("Mid$(A$,{})={}", num(r, 1), st(r, 1)),
            _ => format!("If {}<{} Then A$=A$+B$ Else B$=B$+\"!\"", st(r, 2), st(r, 2)),
        };
        p.push_str(&line);
        p.push('\n');
        p.push_str("Print A$;\"|\";B$;\"|\";N;\"|\";T$(K)\n");
    }
    p.push_str("Next K\nSort T$(0)\nPrint T$(0);T$(3);Match(T$(0),A$)\n");
    if handler {
        p.push_str("End\nH: Print \"err\";Errn : Resume Next\n");
    }
    p
}

#[test]
fn random_string_programs() {
    let mut r = Rng(0xA5A5_1234_DEAD_BEEF);
    let mut ended_ok = 0;
    for i in 0..400 {
        let p = program(&mut r, i % 2 == 1);
        let o = same(&p);
        if o.end == StopReasonOrError::Stop(StopReason::End) {
            ended_ok += 1;
        }
        if i % 10 == 0 {
            same_budget(&p, 3);
        }
    }
    // Most programs should run to the end (errors are caught or rare).
    assert!(ended_ok > 150, "{ended_ok}");
}

#[test]
fn edge_cases() {
    let progs = [
        "Print Mid$(\"abc\",0);Mid$(\"abc\",4);Mid$(\"abc\",2,0);Mid$(\"abc\",3,100);\"|\"",
        "Print Mid$(\"abc\",-1)",
        "Print Mid$(\"abc\",4,-1);\"|\" : Print Mid$(\"abc\",1,-1)",
        "Print Instr(\"abc\",\"\");Instr(\"\",\"a\");Instr(\"abc\",\"c\",3);Instr(\"abc\",\"c\",4);Instr(\"abc\",\"a\",0)",
        "Print Instr(\"abc\",\"a\",-1)",
        "Print Left$(\"abc\",0);\"|\";Right$(\"abc\",0);\"|\";Left$(\"\",5);Right$(\"abc\",5)",
        "Print Chr$(0)+Chr$(255)=Chr$(0)+Chr$(255);Chr$(-1)",
        "Print String$(\"\",5);\"|\";String$(\"xy\",0);\"|\";Space$(0);\"|\";Len(Space$(65536+3))",
        "Print String$(\"a\",-1)",
        "Print \"aaa\"-\"a\";\"|\";\"abab\"-\"ab\";\"|\";\"aabbab\"-\"ab\";\"|\";\"x\"-\"\";\"|\";\"\"-\"x\"",
        "Print Str$(0);Str$(-2147483647-1);Str$(2147483647);Str$(-5)",
        "Print Upper$(\"aZ1é\");Lower$(\"AzZ\");Flip$(\"\");Flip$(\"abc\")",
        "Print \"a\"<\"ab\";\"ab\"<\"a\";\"\"=\"\";\"b\">\"abc\";Chr$(200)>\"a\"",
        "A$=\"x\" : For I=1 To 15 : A$=A$+A$ : Next : Print Len(A$) : B$=A$+A$",
        "A$=Space$(65000) : B$=A$+Space$(471) : Print Len(B$) : C$=B$+\" \"",
        "A$=\"Hello\" : Mid$(A$,0,2)=\"XY\" : Print A$ : Mid$(A$,9,2)=\"Q\" : Print A$ : Mid$(A$,2)=\"abcdefg\" : Print A$",
        "A$=\"Hello\" : Left$(A$,3)=\"ab\" : Print A$ : Right$(A$,2)=\"XYZ\" : Print A$ : Right$(A$,9)=\"12\" : Print A$",
        "A$=\"\" : Mid$(A$,1,1)=\"x\" : Print A$;\"|\" : Mid$(A$,-1,1)=\"y\"",
        "A$=\"abc\" : Left$(A$,-1)=\"x\"",
        "Print Max(\"\",\"a\");Min(\"b\",\"a\");Len(Max(\"zz\",\"z\"))",
    ];
    for p in progs {
        same(p);
    }
}

#[test]
fn garbage_collection_under_pressure() {
    let progs = [
        // Long running loop making garbage: memory must stay bounded.
        "For I=1 To 200000\nA$=Str$(I)+\"-\"+Upper$(\"abc\")+Space$(I mod 50)\nIf Len(A$)>100 Then Print \"?\"\nNext\nPrint A$",
        // Strings kept in arrays and variables while garbage is made.
        "Dim S$(500)\nFor I=0 To 500 : S$(I)=\"k\"+Str$(I) : Next\nFor J=1 To 100\nFor I=0 To 500 : T$=S$(I)+String$(\"x\",100) : Next\nNext\nSort S$(0)\nPrint S$(0);S$(500);Match(S$(0),\"k 250\")",
        // Big strings.
        "For I=1 To 300\nA$=Space$(60000)+Str$(I)\nB$=Right$(A$,5)\nNext\nPrint B$;Len(A$)",
        // Recursion with string locals and Param$.
        "R[\"\",12]\nPrint Len(Param$);Left$(Param$,30)\nProcedure R[S$,N]\nT$=S$+Chr$(65+N)\nIf N=0 Then Pop Proc[T$]\nFor I=1 To 50 : G$=T$+Str$(I) : Next\nR[T$,N-1]\nEnd Proc[Param$+T$]",
        // Strings across waits (run returns between frames).
        "For I=1 To 30\nA$=A$+Chr$(48+I mod 10)\nWait Vbl\nB$=Flip$(A$)\nNext\nPrint A$;B$",
        // Every handler building strings while the main loop does too.
        "Every 1 Gosub E\nFor I=1 To 50000 : M$=Str$(I) : Next\nEvery Off\nPrint Len(Z$)>0;M$\nEnd\nE: Z$=Z$+\".\" : Every On : Return",
        // Fallback instructions and keyword bridge exchanging strings.
        "For I=1 To 3000\nA$=Hex$(I)+Bin$(I mod 8)+Str$(I*1.5)\nInput B$ : Swap A$,B$\nNext\nPrint A$;B$",
    ];
    for p in progs {
        same(p);
    }
    for p in &progs[..2] {
        same_budget(p, 997);
    }
}

#[test]
fn memory_stays_bounded() {
    // 2 million temporary strings: the collector must reclaim them.
    let src = "For I=1 To 2000000\nA$=Str$(I)+\"abcdefghijklmnopqrstuvwxyz\"\nNext\nPrint Len(A$)";
    let prg = amos_core::tokenise::tokenise_program(src.as_bytes()).unwrap();
    let wasm = amos_compiler::compile(&prg).unwrap();
    let mut it = amos_core::interp::Interp::new();
    it.load(&prg).unwrap();
    let mut m = TextMachine { interp: it, host: TextHost::default() };
    let mut cp = amos_wasmhost::CompiledProgram::new(&wasm, m.interp.prg.clone().unwrap()).unwrap();
    loop {
        m.interp.vbl();
        match cp.run(&mut m, 200_000) {
            amos_core::interp::RunState::Running => {}
            s => {
                assert!(matches!(s, amos_core::interp::RunState::Stopped(_)));
                break;
            }
        }
    }
    assert_eq!(String::from_utf8_lossy(&m.host.out), " 34\r\n");
    // Much less than the ~90 MB the strings would take without collection.
    eprintln!("memory {}", cp.memory_size());
    assert!(cp.memory_size() < 16 << 20, "memory {}", cp.memory_size());
}
