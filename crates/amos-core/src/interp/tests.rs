//! Interpreter tests with a text-only host.

use super::stmt::InputState;
use super::value::Value;
use super::*;
use crate::tokenise::tokenise_program;

/// Host that records printed text and implements nothing else.
#[derive(Default)]
struct TextHost {
    out: Vec<u8>,
    input: Vec<Vec<u8>>,
}

impl Host for TextHost {
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        // Accept and ignore unknown instructions with simple parameters.
        let _ = it.inst_args(self, kw)?;
        Ok(())
    }
    fn function(&mut self, _it: &mut Interp, kw: Keyword) -> R<Value> {
        Err(Exc::Message(format!("function {:?} not available", kw.def().map(|d| d.name))))
    }
    fn reserved_assign(&mut self, _it: &mut Interp, _kw: Keyword) -> R<()> {
        Ok(())
    }
    fn test_point(&mut self, _it: &mut Interp) -> R<()> {
        Ok(())
    }
    fn take_break(&mut self) -> bool {
        false
    }
    fn print(&mut self, _it: &mut Interp, text: &[u8]) -> R<()> {
        self.out.extend_from_slice(text);
        Ok(())
    }
    fn read_line(&mut self, _it: &mut Interp, _state: &mut InputState) -> R<Option<Vec<u8>>> {
        if self.input.is_empty() {
            return Ok(Some(Vec::new()));
        }
        Ok(Some(self.input.remove(0)))
    }
}

/// Runs a program given as text and returns its output (CR LF become LF).
fn run_with(src: &str, input: &[&str]) -> Result<String, StopReasonOrError> {
    let prg = tokenise_program(src.as_bytes()).unwrap();
    let mut it = Interp::new();
    it.load(&prg).map_err(|e| StopReasonOrError::Test(e.code))?;
    let mut host = TextHost { input: input.iter().map(|s| s.as_bytes().to_vec()).collect(), ..Default::default() };
    for _ in 0..10_000 {
        it.vbl();
        match it.run(&mut host, 100_000) {
            RunState::Running => continue,
            RunState::Stopped(info) => {
                let text = String::from_utf8_lossy(&host.out).replace("\r\n", "\n");
                return match info.reason {
                    StopReasonOrError::Stop(StopReason::End) => Ok(text),
                    other => Err(other),
                };
            }
            RunState::Idle => break,
        }
    }
    panic!("program did not end")
}

fn run(src: &str) -> String {
    match run_with(src, &[]) {
        Ok(s) => s,
        Err(e) => panic!("program failed: {e:?}"),
    }
}

fn run_err(src: &str) -> StopReasonOrError {
    run_with(src, &[]).expect_err("expected an error")
}

#[test]
fn print_numbers_and_strings() {
    assert_eq!(run("Print 1\nPrint -5\nPrint \"a\";\"b\""), " 1\n-5\nab\n");
    assert_eq!(run("A=3 : B=4 : Print A*B"), " 12\n");
    assert_eq!(run("Print 1;2"), " 1 2\n");
    assert_eq!(run("Print \"x\",\"y\""), "x\ty\n");
}

#[test]
fn amos_operator_precedence() {
    // Every operator is its own level: * is below /.
    assert_eq!(run("Print 10*3/4"), " 0\n");
    assert_eq!(run("Print 10/2*3"), " 15\n");
    assert_eq!(run("Print 10-2+3"), " 11\n");
    assert_eq!(run("Print 2+3*4"), " 14\n");
    assert_eq!(run("Print -2^2"), " 4\n");
    assert_eq!(run("Print -7 mod 3"), " 0\n");
    assert_eq!(run("Print 7 mod 0"), " 7\n");
    assert_eq!(run("Print 3=3"), "-1\n");
    assert_eq!(run("Print Not 0"), "-1\n");
}

#[test]
fn floats() {
    assert_eq!(run("Print 1.5+1"), " 2.5\n");
    assert_eq!(run("A#=1/3.0 : Print A#"), " 0.3333333\n");
    assert_eq!(run("Print Int(2.7)"), " 2\n");
    assert_eq!(run("Print Sgn(-2.5)"), "-1\n");
}

#[test]
fn strings() {
    assert_eq!(run("A$=\"Hello\" : Print Left$(A$,2);Right$(A$,2);Mid$(A$,2,3)"), "Heloell\n");
    assert_eq!(run("Print Len(\"abc\");Asc(\"A\");Chr$(66)"), " 3 65B\n");
    assert_eq!(run("Print Instr(\"hello\",\"l\")"), " 3\n");
    assert_eq!(run("Print \"aabb\"-\"ab\""), "\n");
    assert_eq!(run("Print Upper$(\"abc\")+Lower$(\"DEF\")"), "ABCdef\n");
    assert_eq!(run("Print Str$(12);Val(\"34\")"), " 12 34\n");
    assert_eq!(run("Print Hex$(255);Hex$(255,4);Bin$(5)"), "$FF$00FF%101\n");
    assert_eq!(run("A$=\"Hello\" : Mid$(A$,2,3)=\"EYY\" : Print A$"), "HEYYo\n");
    assert_eq!(run("Print String$(\"ab\",3);Space$(2);\"|\""), "aaa  |\n");
}

#[test]
fn loops() {
    assert_eq!(run("For I=1 To 3 : Print I; : Next I"), " 1 2 3");
    assert_eq!(run("For I=10 To 1 : Print I; : Next"), " 10");
    assert_eq!(run("For I=6 To 0 Step -2\nPrint I;\nNext I"), " 6 4 2 0");
    assert_eq!(run("I=0\nRepeat\nInc I\nUntil I=5\nPrint I"), " 5\n");
    assert_eq!(run("I=0\nWhile I<3\nInc I\nWend\nPrint I"), " 3\n");
    assert_eq!(run("I=0\nDo\nInc I\nIf I=4 Then Exit\nLoop\nPrint I"), " 4\n");
    assert_eq!(run("For I=1 To 10\nExit If I=3\nNext\nPrint I"), " 3\n");
    assert_eq!(run("For I=1 To 2\nFor J=1 To 2\nPrint I*10+J;\nNext J\nNext I"), " 11 12 21 22");
}

#[test]
fn if_structures() {
    let p = "For I=1 To 4\nIf I=1\nPrint \"a\";\nElse If I=2\nPrint \"b\";\nElse If I=3\nPrint \"c\";\nElse\nPrint \"d\";\nEnd If\nNext";
    assert_eq!(run(p), "abcd");
    assert_eq!(run("A=1 : If A=1 Then Print \"y\" Else Print \"n\""), "y\n");
    assert_eq!(run("A=2 : If A=1 Then Print \"y\" Else Print \"n\""), "n\n");
    assert_eq!(run("A=2 : If A=1 Then Print \"y\"\nPrint \"z\""), "z\n");
    assert_eq!(run("A=1\nIf A=1\nPrint \"1\"\nEnd If\nPrint \"2\""), "1\n2\n");
}

#[test]
fn gosub_goto() {
    assert_eq!(run("Gosub L : Print \"b\" : End\nL: Print \"a\" : Return"), "a\nb\n");
    assert_eq!(run("Goto 10\nPrint \"x\"\n10 Print \"y\""), "y\n");
    assert_eq!(run("A=2 : On A Goto L1,L2\nL1: Print 1 : End\nL2: Print 2"), " 2\n");
}

#[test]
fn procedures() {
    let p = "TEST[3,\"x\"]\nPrint Param\nProcedure TEST[A,B$]\n  Print A;B$\nEnd Proc[A*2]";
    assert_eq!(run(p), " 3x\n 6\n");
    let rec = "FACT[5]\nPrint Param\nProcedure FACT[N]\n If N<=1 Then Pop Proc[1]\n FACT[N-1]\n R=Param*N\nEnd Proc[R]";
    assert_eq!(run(rec), " 120\n");
    // Locals are separate from globals; Shared gives access.
    let sh = "A=1 : B=2\nT\nPrint A;B\nProcedure T\n Shared A\n A=10 : B=20\nEnd Proc";
    assert_eq!(run(sh), " 10 2\n");
    let gl = "Global G\nG=5\nT\nPrint G\nProcedure T\n G=G+1\nEnd Proc";
    assert_eq!(run(gl), " 6\n");
}

#[test]
fn arrays_and_data() {
    assert_eq!(run("Dim A(3)\nFor I=0 To 3 : A(I)=I*I : Next\nPrint A(3)"), " 9\n");
    assert_eq!(run("Dim B$(2,2)\nB$(1,2)=\"x\"\nPrint B$(1,2)"), "x\n");
    assert_eq!(run("Read A,B$ : Print A;B$\nData 5,\"z\""), " 5z\n");
    assert_eq!(run("For I=1 To 3 : Read A : Print A; : Next\nData 1,2\nData 3"), " 1 2 3");
    assert_eq!(run("Read A : Restore : Read B : Print A;B\nData 7"), " 7 7\n");
    assert_eq!(run("Dim C(3)\nC(0)=3 : C(1)=1 : C(2)=2 : C(3)=0\nSort C(0)\nPrint C(0);C(3)"), " 0 3\n");
}

#[test]
fn errors_and_trapping() {
    assert_eq!(run_err("Print 1/0"), StopReasonOrError::Error(crate::errors::DIVISION_BY_ZERO));
    assert_eq!(run_err("Read A"), StopReasonOrError::Error(crate::errors::OUT_OF_DATA));
    assert_eq!(run_err("Return"), StopReasonOrError::Error(crate::errors::RETURN_WITHOUT_GOSUB));
    let p = "On Error Goto H\nPrint 1/0\nPrint \"after\"\nEnd\nH: Print \"err\";Errn\nResume Next";
    assert_eq!(run(p), "err 20\nafter\n");
    assert_eq!(run("Trap Print 1/0\nPrint Errtrap"), " 20\n");
    assert_eq!(run_err("A=\"x\""), StopReasonOrError::Test(crate::interp::verify::terr::TYPE_MISMATCH));
}

#[test]
fn def_fn_and_misc() {
    assert_eq!(run("Def Fn SQ(X)=X*X\nPrint Fn SQ(4)"), " 16\n");
    assert_eq!(run("A=1 : B=2 : Swap A,B : Print A;B"), " 2 1\n");
    assert_eq!(run("A=5 : Add A,3 : Print A : Add A,10,0 To 10 : Print A"), " 8\n 0\n");
    assert_eq!(run("Print Max(3,7);Min(3,7)"), " 7 3\n");
}

#[test]
fn input() {
    let out = run_with("Input \"Name\";N$ : Print \"Hi \";N$", &["Bob"]).unwrap();
    assert_eq!(out, "Name\nHi Bob\n");
}

#[test]
fn shared_string_constants_are_never_changed() {
    // String constants are shared between evaluations: changing a
    // variable that holds one must not change the constant.
    let src = "For I=1 To 3\nA$=\"abc\" : Mid$(A$,1,1)=\"X\" : Left$(A$,1)=\"Y\" : Right$(A$,1)=\"Z\"\nPrint A$;\"abc\"\nNext I";
    assert_eq!(run(src), "YbZabc\nYbZabc\nYbZabc\n");
}

#[test]
fn integer_expressions_fast_and_general_agree() {
    // Integer only expressions take the fast evaluator; the same values
    // through a float variable take the general one.
    let src = "A=7 : B=-3 : C#=1.0\n\
Print A+B*2-(A/2) mod 3;A*B/2;Not A=7;-A+-B;(A and 3) or (B xor 5);A>B;A<=B;10*3/4\n\
Print A+B*2-(A/2) mod 3+C#-1.0\n\
Print $7FFFFFFF-1;%101+$10;U;-(-A)\n";
    // (Expected output from the evaluator before the fast path.)
    assert_eq!(run(src), " 1-7 0-4-5-1 0 0\n 1\n 2147483646 21 0 7\n");
    // Errors: the same as the general evaluator.
    assert_eq!(run_err("A=$7FFFFFFF : B=A+1"), StopReasonOrError::Error(crate::errors::OVERFLOW));
    assert_eq!(run_err("A=0 : B=5/A"), StopReasonOrError::Error(crate::errors::DIVISION_BY_ZERO));
    assert_eq!(run_err("A=0 : B=5/(A*1)+1"), StopReasonOrError::Error(crate::errors::DIVISION_BY_ZERO));
}
