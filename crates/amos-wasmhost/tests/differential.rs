//! Differential tests: every program runs interpreted and compiled, and the
//! outputs and the way it ends must be identical.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use amos_core::errors;
use amos_core::interp::StopReasonOrError;
use common::*;

// ----------------------------------------------------------------------
// The cases of `interp/tests.rs`
// ----------------------------------------------------------------------

#[test]
fn print_numbers_and_strings() {
    assert_eq!(run("Print 1\nPrint -5\nPrint \"a\";\"b\""), " 1\n-5\nab\n");
    assert_eq!(run("A=3 : B=4 : Print A*B"), " 12\n");
    assert_eq!(run("Print 1;2"), " 1 2\n");
    assert_eq!(run("Print \"x\",\"y\""), "x\ty\n");
}

#[test]
fn amos_operator_precedence() {
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
    assert_eq!(run_err("Print 1/0"), StopReasonOrError::Error(errors::DIVISION_BY_ZERO));
    assert_eq!(run_err("Read A"), StopReasonOrError::Error(errors::OUT_OF_DATA));
    assert_eq!(run_err("Return"), StopReasonOrError::Error(errors::RETURN_WITHOUT_GOSUB));
    let p = "On Error Goto H\nPrint 1/0\nPrint \"after\"\nEnd\nH: Print \"err\";Errn\nResume Next";
    assert_eq!(run(p), "err 20\nafter\n");
    assert_eq!(run("Trap Print 1/0\nPrint Errtrap"), " 20\n");
    assert_eq!(run_err("A=\"x\""), StopReasonOrError::Test(amos_core::interp::verify::terr::TYPE_MISMATCH));
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
    let out = same_with("Input \"Name\";N$ : Print \"Hi \";N$", &["Bob"]);
    assert_eq!(out.text, "Name\nHi Bob\n");
}
