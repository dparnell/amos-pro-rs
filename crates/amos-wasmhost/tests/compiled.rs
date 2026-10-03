//! More differential tests: compiled and interpreted runs must agree.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use amos_core::errors;
use amos_core::interp::{StopReason, StopReasonOrError};
use common::*;

#[test]
fn integer_arithmetic_and_overflow() {
    assert_eq!(run("Print 2147483647-1;-2147483647-1"), " 2147483646-2147483648\n");
    assert_eq!(run_err("A=2147483647 : B=A+1"), StopReasonOrError::Error(errors::OVERFLOW));
    assert_eq!(run_err("A=-2147483647 : B=A-2"), StopReasonOrError::Error(errors::OVERFLOW));
    // 16 bit operands wrap, larger ones overflow (the original's multiply).
    assert_eq!(run("Print 50000*50000"), "-1794967296\n");
    assert_eq!(run("Print 65536*32767;-65536*32767"), " 2147418112-2147418112\n");
    assert_eq!(run_err("A=65536*32768"), StopReasonOrError::Error(errors::OVERFLOW));
    assert_eq!(run("Print -7/2;7/-2;-7 mod 2;7 mod -2;7 mod 0"), "-3-3 1 1 7\n");
    assert_eq!(run_err("A=0 : Print 5/A"), StopReasonOrError::Error(errors::DIVISION_BY_ZERO));
    assert_eq!(run("A=-2147483647-1 : Print A/1;A/-1;Abs(A)"), "-2147483648-2147483648-2147483648\n");
    assert_eq!(run("Print 5 and 3;5 or 3;5 xor 3;Not 5"), " 1 7 6-6\n");
    assert_eq!(run("Print 1<2;2<1;1<=1;1>=2;1<>1;3=3"), "-1 0-1 0 0-1\n");
    // Any number of signs negates once (`Interp::operand`).
    assert_eq!(run("Print --5;-(-5);- -5"), "-5 5-5\n");
    assert_eq!(run("Print 2^10;2^0.5"), " 1024 1.41421\n");
}

#[test]
fn single_precision_floats() {
    assert_eq!(run("A#=0.1 : B#=0.2 : Print A#+B#;A#*B#;A#-B#;A#/B#"), " 0.3 0.02-0.1 0.5\n");
    assert_eq!(run("X#=1 : For I=1 To 30 : X#=X#*1.1 : Next : Print X#"), " 17.4494\n");
    assert_eq!(run("A#=16777217 : Print A#;Int(2.5);Int(-2.5)"), " 1.67772 E+07 2-3\n");
    assert_eq!(run("A=7 : B#=A/2 : C#=A/2.0 : Print B#;C#"), " 3 3.5\n");
    assert_eq!(run("A#=3.7 : B=A# : Print B;-A#"), " 3-3.7\n");
    assert_eq!(run("Print 0.1+0.2=0.3;1.5>1;1<1.5"), "-1-1-1\n");
    assert_eq!(run_err("A#=0 : Print 1.0/A#"), StopReasonOrError::Error(errors::DIVISION_BY_ZERO));
    assert_eq!(run("A#=2.5 : Inc A# : Dec A# : Dec A# : Print A#"), " 1.5\n");
    assert_eq!(
        run("Print Sqr(2);Sin(1);Abs(-2.5);Sgn(0.0);Max(1,2.5);Min(1.5,2)"),
        " 1.41421 0.841471 2.5 0 2.5 1.5\n"
    );
    assert_eq!(run("Fix 2 : Print 3.14159;1/3.0"), " 3.14 0.33\n");
    assert_eq!(run("Degree : Print Sin(90);Cos(0)"), " 1 1\n");
}

#[test]
fn double_precision_floats() {
    let p = "Set Double Precision\nA#=0.1 : B#=0.2 : Print A#+B#\nX#=1 : For I=1 To 30 : X#=X#*1.1 : Next : Print X#\nPrint 1/3.0;2^0.5;Int(-2.5)";
    let out = run(p);
    assert!(!out.is_empty());
    assert_eq!(run("Set Double Precision\nA#=16777217 : Print A#;A#=16777217"), " 16777217-1\n");
    assert_eq!(
        run_err("Set Double Precision\nA#=0 : Print 1.0/A#"),
        StopReasonOrError::Error(errors::DIVISION_BY_ZERO)
    );
    assert_eq!(run("Set Double Precision\nPrint 0.5<0.25;0.5>=0.5;Sgn(-0.5)"), " 0-1-1\n");
}

#[test]
fn dynamic_numbers() {
    // Val returns an integer or a float depending on the text.
    assert_eq!(run("Print Val(\"12\")+1;Val(\"1.5\")+1;Val(\"1.5\")*2;-Val(\"3\")"), " 13 2.5 3-3\n");
    assert_eq!(run("A$=\"7\" : B=Val(A$)*3 : C#=Val(A$)/2 : Print B;C#"), " 21 3\n");
    assert_eq!(run("Print Val(\"2\")=2;Val(\"2.5\")>2;Abs(Val(\"-4\"));Max(Val(\"3\"),2)"), "-1-1 4 3\n");
    assert_eq!(run_err("A=Val(\"2147483647\")+1"), StopReasonOrError::Error(errors::OVERFLOW));
    let p = "TEST[1]\nPrint Param;Param#\nProcedure TEST[A]\nEnd Proc[Val(\"2.5\")]";
    assert_eq!(run(p), " 0 2.5\n");
}

#[test]
fn strings_more() {
    assert_eq!(run("A$=\"\" : For I=1 To 5 : A$=A$+Chr$(64+I) : Next : Print A$;Len(A$)"), "ABCDE 5\n");
    assert_eq!(
        run("A$=\"abcdef\" : Print Mid$(A$,3);Mid$(A$,0,2);Mid$(A$,10,2);Left$(A$,0);Right$(A$,10)"),
        "cdefababcdef\n"
    );
    assert_eq!(run_err("Print Left$(\"abc\",-1)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(run_err("Print Mid$(\"abc\",1,-1)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(run_err("Print Chr$(256)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(run("Print \"abc\"<\"abd\";\"b\">\"abc\";\"x\"=\"x\";\"\"<\"a\""), "-1-1-1-1\n");
    assert_eq!(run("Print Instr(\"hello\",\"l\",4);Instr(\"hello\",\"z\");Instr(\"\",\"a\")"), " 4 0 0\n");
    assert_eq!(run("Print Str$(-3.5);Str$(7);Upper$(\"aBc\");Lower$(\"AbC\")"), "-3.5 7ABCabc\n");
    assert_eq!(run("Print Max(\"a\",\"b\");Min(\"a\",\"b\");Asc(\"\");Flip$(\"abc\")"), "ba 0cba\n");
    assert_eq!(run("A$=\"x\" : For I=1 To 15 : A$=A$+A$ : Next : Print Len(A$)"), " 32768\n");
    assert_eq!(
        run_err("A$=\"x\" : For I=1 To 17 : A$=A$+A$ : Next"),
        StopReasonOrError::Error(errors::STRING_TOO_LONG)
    );
    assert_eq!(run("Print \"hello world\"-\"o\";\"aaa\"-\"\""), "hell wrldaaa\n");
}

#[test]
fn string_garbage_collection() {
    // Many temporary strings: the string table is collected at test points.
    let p = "N=0\nFor I=1 To 50000\nA$=Str$(I)+\"-\"+Str$(I*2)\nB$=Left$(A$,3)\nIf Len(B$)=3 Then Inc N\nNext\nPrint N;A$;B$";
    assert_eq!(run(p), " 50000 50000- 100000 50\n");
    let p = "Dim T$(10)\nFor I=1 To 20000 : T$(I mod 10)=\"v\"+Str$(I) : Next\nFor I=0 To 9 : Print T$(I); : Next";
    run(p);
}

#[test]
fn control_flow_more() {
    // A For loop always runs at least once.
    assert_eq!(run("For I=5 To 1 : Print I; : Next : Print I"), " 5 6\n");
    assert_eq!(run("For I=1 To 3 : Next : Print I"), " 4\n");
    assert_eq!(run("For I=1 To 10 Step 3 : Print I; : Next"), " 1 4 7 10");
    assert_eq!(run("A#=0 : For A#=1 To 2 : Print A#; : Next"), " 1 2");
    // Exit several loops.
    let p = "For I=1 To 3\nFor J=1 To 3\nIf J=2 Then Exit 2\nPrint I;J;\nNext J\nNext I\nPrint \"x\";I;J";
    assert_eq!(run(p), " 1 1x 1 2\n");
    let p = "I=0\nDo\nInc I\nExit If I>3\nPrint I;\nLoop\nPrint \"done\"";
    assert_eq!(run(p), " 1 2 3done\n");
    let p = "I=0\nRepeat\nJ=0\nWhile J<2\nInc J : Print I;J;\nWend\nInc I\nUntil I=2";
    assert_eq!(run(p), " 0 1 0 2 1 1 1 2");
    // Goto out of loops drops them.
    let p = "For I=1 To 3\nFor J=1 To 3\nIf J=2 Then Goto OUT\nNext J\nNext I\nOUT: Print I;J\nFor K=1 To 2 : Print K; : Next";
    assert_eq!(run(p), " 1 2\n 1 2");
    let p = "A=1\nIf A=1 Then Goto L1 Else Goto L2\nL1: Print \"one\" : End\nL2: Print \"two\"";
    assert_eq!(run(p), "one\n");
    let p = "A=2\nIf A=1 Then 10 Else 20\n10 Print \"one\" : End\n20 Print \"two\"";
    assert_eq!(run(p), "two\n");
    let p = "For I=1 To 5\nIf I=1\nPrint \"a\";\nElse If I<4 Then Print \"b\";\nElse\nPrint \"c\";\nEnd If\nNext";
    assert_eq!(run(p), "abbcc");
    let p = "A=3 : If A>1 Then If A>2 Then Print \"both\" Else Print \"one\"";
    run(p);
    let p = "For I=1 To 4 : If I mod 2=0 Then Print I; : Next";
    run(p);
}

#[test]
fn gosub_goto_on() {
    let p = "For I=1 To 3 : On I Gosub A,B,C : Next : End\nA: Print \"a\"; : Return\nB: Print \"b\"; : Return\nC: Print \"c\"; : Return";
    assert_eq!(run(p), "abc");
    let p = "I=5 : On I Goto A,B\nPrint \"none\" : End\nA: Print \"a\" : End\nB: Print \"b\"";
    assert_eq!(run(p), "none\n");
    let p = "Gosub A : Print \"back\" : End\nA: Gosub B : Print \"a\" : Return\nB: Print \"b\" : Return";
    assert_eq!(run(p), "b\na\nback\n");
    let p = "L$=\"TARGET\" : Goto L$\nPrint \"no\"\nTARGET: Print \"yes\"";
    assert_eq!(run(p), "yes\n");
    let p = "N=20 : Gosub N+0 : End\n20 Print \"line\" : Return";
    assert_eq!(run(p), "line\n");
    let p = "For I=1 To 2 : On I Proc P1,P2 : Next\nProcedure P1\nPrint \"p1\";\nEnd Proc\nProcedure P2\nPrint \"p2\";\nEnd Proc";
    assert_eq!(run(p), "p1p2");
    let p = "Gosub A : End\nA: For I=1 To 3 : If I=2 Then Pop : Goto B\nNext\nB: Print I";
    run(p);
}

#[test]
fn procedures_more() {
    let p = "Print \"start\"\nFIB[15]\nPrint Param\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]";
    assert_eq!(run(p), "start\n 610\n");
    // Locals: each call has its own; strings and floats.
    let p = "S$=\"g\" : X#=1.5\nP[3,\"abc\",2.5]\nPrint S$;X#;Param$\nProcedure P[N,T$,F#]\nS$=T$+\"!\" : X#=F#*N\nIf N>0 Then P[N-1,T$+\"x\",F#] : Print S$;X#;\nEnd Proc[S$]";
    let out = run(p);
    assert!(out.ends_with("g 1.5abc!\n"), "{out}");
    let p = "Global A()\nDim A(5)\nFILL[5]\nFor I=0 To 5 : Print A(I); : Next\nProcedure FILL[N]\nFor I=0 To N : A(I)=I*I : Next\nEnd Proc";
    same(p);
    let p = "LOC\nPrint \"x\"\nProcedure LOC\nDim L(3)\nL(1)=7 : Print L(1)\nEnd Proc";
    assert_eq!(run(p), " 7\nx\n");
    let p = "P[1]\nP[2]\nProcedure P[N]\nIf N=1 Then Pop Proc\nPrint \"n=\";N\nEnd Proc";
    assert_eq!(run(p), "n= 2\n");
    let p = "Proc P[4]\nPrint Param\nProcedure P[N]\nEnd Proc[N*N]";
    assert_eq!(run(p), " 16\n");
    // Deep recursion: out of stack space.
    let p = "R[0]\nProcedure R[N]\nR[N+1]\nEnd Proc";
    assert_eq!(run_err(p), StopReasonOrError::Error(errors::OUT_OF_STACK));
    let p =
        "Global G$\nG$=\"a\"\nT\nPrint G$\nProcedure T\nG$=G$+\"b\"\nU\nEnd Proc\nProcedure U\nG$=G$+\"c\"\nEnd Proc";
    assert_eq!(run(p), "abc\n");
    assert_eq!(run_err("End Proc"), StopReasonOrError::Test(18));
}

#[test]
fn arrays_more() {
    let p = "Dim A(2,3)\nFor I=0 To 2 : For J=0 To 3 : A(I,J)=I*10+J : Next : Next\nPrint A(2,3);A(1,2)";
    assert_eq!(run(p), " 23 12\n");
    assert_eq!(run_err("Dim A(3) : A(4)=1"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(run_err("Dim A(3) : Print A(-1)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    run_err("Dim A(3) : Dim A(3)");
    assert_eq!(
        run_err("Dim A(3)\nGosub D : Gosub D : End\nD: Dim B(2) : Return"),
        StopReasonOrError::Error(errors::ARRAY_ALREADY_DIMENSIONED)
    );
    let p = "Dim F#(3)\nF#(1)=1.5 : F#(2)=F#(1)*2 : Print F#(2);F#(0)";
    assert_eq!(run(p), " 3 0\n");
    let p = "Dim S$(3)\nS$(0)=\"c\" : S$(1)=\"a\" : S$(2)=\"b\"\nSort S$(0)\nFor I=0 To 3 : Print S$(I);\",\"; : Next";
    run(p);
    let p = "Dim A(10)\nFor I=0 To 10 : A(I)=I*2 : Next\nPrint Match(A(0),6);Match(A(0),7)";
    run(p);
    let p = "Dim A(3)\nA(1)=5 : Inc A(1) : Add A(1),10 : Print A(1)";
    assert_eq!(run(p), " 16\n");
    let p = "Dim A(3,3),B(3)\nB(2)=3 : A(1,B(2))=7 : A(B(1),B(B(2)-1))=8\nPrint A(1,3);A(0,3);A(1,B(2))";
    assert_eq!(run(p), " 7 8 7\n");
    let p = "Dim A(3)\nFor A(1)=1 To 3 : Print A(1); : Next";
    assert_eq!(run(p), " 1 2 3");
}

#[test]
fn errors_and_handlers() {
    let p = "On Error Goto H\nA=1/0\nPrint \"after\";E\nEnd\nH: E=Errn : Resume Next";
    assert_eq!(run(p), "after 20\n");
    let p = "On Error Goto H\nN=0\nA=10/N\nPrint A\nEnd\nH: N=2 : Resume";
    assert_eq!(run(p), " 5\n");
    let p =
        "On Error Proc HP\nA=1/0\nPrint \"ok\"\nEnd\nProcedure HP\nPrint \"in handler\";Errn\nResume Next\nEnd Proc";
    assert_eq!(run(p), "in handler 20\nok\n");
    let p = "Trap A=1/0 : Print Errtrap\nTrap B=2/1 : Print Errtrap;B";
    assert_eq!(run(p), " 20\n 0 2\n");
    let p = "P\nProcedure P\nTrap A$=Left$(\"a\",-1)\nPrint Errtrap\nEnd Proc";
    assert_eq!(run(p), " 23\n");
    assert_eq!(run_err("Error 42"), StopReasonOrError::Error(42));
    assert_eq!(run_err("On Error Goto H\nA=1/0\nEnd\nH: A=1/0"), StopReasonOrError::Error(errors::DIVISION_BY_ZERO));
    let p = "On Error Goto H\nFor I=1 To 3\nA=10/(I-2)\nPrint A;\nNext\nEnd\nH: Print \"e\"; : Resume Next";
    // The handler is outside the loop: the jump drops the For, so the
    // Next after Resume Next fails too.
    assert_eq!(run(p), "-10e-10e");
    assert_eq!(run_err("Resume"), StopReasonOrError::Error(errors::RESUME_WITHOUT_ERROR));
}

#[test]
fn def_fn_data_input() {
    assert_eq!(run("Def Fn F(X,Y)=X*10+Y\nA=3 : Print Fn F(A,4);Fn F(1,2)"), " 34 12\n");
    assert_eq!(run("Def Fn S$(A$)=A$+A$\nPrint Fn S$(\"ab\")"), "abab\n");
    let p = "Restore D2 : Read A$ : Print A$\nD1: Data \"one\"\nD2: Data \"two\"";
    assert_eq!(run(p), "two\n");
    let p = "K=3 : Read A,B : Print A;B\nData K*2,K+1";
    assert_eq!(run(p), " 6 4\n");
    let p = "P\nProcedure P\nRead X : Print X\nData 9\nEnd Proc\nData 1";
    assert_eq!(run(p), " 9\n");
    assert_eq!(run_err("Read A$\nData 5"), StopReasonOrError::Error(errors::TYPE_MISMATCH));
    let o = same_with("Input A,B$ : Print A*2;B$\nLine Input C$ : Print C$", &["21,xy", "a,b"]);
    assert_eq!(o.text, "? \n 42xy\n? \na,b\n");
}

#[test]
fn misc_statements() {
    assert_eq!(run("A=1 : B=2 : Swap A,B : Print A;B"), " 2 1\n");
    assert_eq!(run("A$=\"a\" : B$=\"b\" : Swap A$,B$ : Print A$;B$"), "ba\n");
    assert_eq!(run("A=5 : Add A,-10,0 To 10 : Print A : Add A,3 : Print A"), " 10\n 13\n");
    assert_eq!(run("A#=1.5 : Add A#,2 : Print A#"), " 3\n");
    assert_eq!(run("Randomize 42 : A=Rnd(100) : Randomize 42 : B=Rnd(100) : Print A=B"), "-1\n");
    assert_eq!(run("Print Hex$(-1);Bin$(5,8)"), "$FFFFFFFF%00000101\n");
    assert_eq!(run("Print Using \"###.##\";3.14159"), "  3.14\n");
    assert_eq!(run("A=1 : Print A;\nPrint A,A"), " 1 1\t 1\n");
    assert_eq!(run_err("Stop"), StopReasonOrError::Stop(StopReason::Break));
    assert_eq!(run_err("Print \"x\" : Edit"), StopReasonOrError::Stop(StopReason::Edit));
    assert_eq!(run("Print \"a\" : End : Print \"b\""), "a\n");
    assert_eq!(run("Print True;False;Pi#"), "-1 0 3.14159\n");
}

#[test]
fn waits_and_events() {
    // Wait Vbl yields to the host: one frame each.
    let o = same("For I=1 To 5 : Wait Vbl : Next : Print \"done\"");
    assert_eq!(o.text, "done\n");
    assert!(o.frames >= 5, "{o:?}");
    let o = same("Wait 10 : Print \"w\"");
    assert!(o.frames >= 10);
    // Every: a Gosub every 2 frames, while the main program waits.
    let p = "C=0\nEvery 2 Gosub E\nFor I=1 To 10 : Wait Vbl : Next\nEvery Off\nPrint C>2\nEnd\nE: Inc C : Every On : Return";
    assert_eq!(same(p).text, "-1\n");
    let p = "C=0\nEvery 3 Proc EP\nFor I=1 To 12 : Wait Vbl : Next\nPrint C>1\nProcedure EP\nShared C\nInc C\nEvery On\nEnd Proc";
    assert_eq!(same(p).text, "-1\n");
    // Events in a busy loop (test points of Next).
    let p = "C=0\nEvery 1 Gosub E\nFor I=1 To 300000 : Next\nEvery Off\nPrint C>0\nEnd\nE: Inc C : Every On : Return";
    assert_eq!(same(p).text, "-1\n");
}

#[test]
fn same_yield_points() {
    // With a tiny budget the programs must run the same number of frames:
    // the compiled program counts instructions like the interpreter.
    let progs = [
        "For I=1 To 100 : A=A+I : Next : Print A",
        "I=0\nWhile I<50\nInc I\nIf I mod 3=0 Then Print I;\nWend",
        "FIB[8]\nPrint Param\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]",
        "C=0\nEvery 1 Gosub E\nFor I=1 To 200 : A$=A$+\"x\" : Next\nEvery Off\nPrint C;Len(A$)\nEnd\nE: Inc C : Every On : Return",
        "On Error Goto H\nFor I=1 To 30 : A=10/(I mod 3) : Next\nPrint N\nEnd\nH: Inc N : Resume Next",
        "Dim A(20)\nFor I=0 To 20 : A(I)=20-I : Next\nSort A(0)\nFor I=0 To 20 : Print A(I); : Next",
    ];
    for p in progs {
        for budget in [1, 2, 3, 7, 50] {
            same_budget(p, budget);
        }
    }
}

#[test]
fn input_waits() {
    // Input blocks (the instruction is executed again each frame) until a
    // line is typed; events still run meanwhile.
    let src = "C=0\nEvery 1 Gosub E\nInput \"N\";N : Line Input A$\nEvery Off\nPrint N*2;A$;C>3\nEnd\nE: Inc C : Every On : Return";
    let o = with_input_delay(5, || same_with(src, &["21", "xy"]));
    // The Every handler interrupts the waiting Input, which starts again
    // (prompt printed again), as in the interpreter.
    assert_eq!(o.text, "NNN\n? ? ? ? \n 42xy-1\n");
    assert!(o.frames >= 3, "{o:?}");
}

#[test]
fn linear_arrays() {
    // Errors: index range (23), not dimensioned (27), Dim checks.
    same("Dim A(3)\nI=1 : B=A(I,I)");
    assert_eq!(run_err("Dim A(3)\nI=4 : A(I)=1"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(run_err("Dim A(3)\nI=-1 : Print A(I)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(
        run_err("Gosub D : Print A(1) : End\nD: Return : Dim A(3)"),
        StopReasonOrError::Error(errors::NON_DIMENSIONED_ARRAY)
    );
    assert_eq!(run_err("N=-1 : Dim A(N)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    assert_eq!(run_err("N=300 : Dim A(N,N,2)"), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    // The last dimension is not counted (quirk of the original).
    assert_eq!(run("N=2 : Dim A(N,20000) : A(2,20000)=5 : Print A(2,20000)"), " 5\n");
    // Multi dimensional, floats, strings.
    let p = "Dim A(2,3,4),F#(3),S$(3)\nFor I=0 To 2 : For J=0 To 3 : For K=0 To 4 : A(I,J,K)=I*100+J*10+K : Next : Next : Next\nF#(1)=1.5 : F#(2)=F#(1)*3 : S$(1)=\"ab\" : S$(2)=S$(1)+S$(1)\nPrint A(2,3,4);A(1,0,2);F#(2);S$(2);S$(0);\"|\"";
    assert_eq!(run(p), " 234 102 4.5abab|\n");
    // Inc, Add, Swap on elements.
    let p = "Dim A(3),F#(2),S$(2)\nA(1)=5 : Inc A(1) : Dec A(2) : Add A(1),10 : Add A(3),5,0 To 3\nF#(1)=2.5 : Inc F#(1) : Add F#(2),2\nS$(0)=\"x\" : S$(1)=\"y\" : Swap S$(0),S$(1) : Swap A(1),A(2)\nPrint A(1);A(2);A(3);F#(1);F#(2);S$(0);S$(1)";
    assert_eq!(run(p), "-1 16 0 3.5 2yx\n");
    // Sort and Match.
    let p = "Dim A(9),S$(4),F#(3)\nFor I=0 To 9 : A(I)=(I*7) mod 10 : Next\nSort A(0)\nFor I=0 To 9 : Print A(I); : Next : Print\nPrint Match(A(0),7);Match(A(0),11)\nS$(0)=\"d\" : S$(1)=\"b\" : S$(2)=\"a\" : S$(3)=\"c\" : S$(4)=\"\"\nSort S$(0) : Print S$(0);S$(1);S$(2);S$(4);Match(S$(0),\"c\")\nF#(0)=2.5 : F#(1)=-1 : Sort F#(0) : Print F#(0);F#(3);Match(F#(0),2.5)";
    same(p);
    // Local arrays in recursive procedures are separate and freed.
    let p =
        "R[5]\nPrint Param\nProcedure R[N]\nDim L(N+1)\nL(N)=N\nIf N>0 Then R[N-1] : L(N)=L(N)+Param\nEnd Proc[L(N)]";
    assert_eq!(run(p), " 15\n");
    let p = "For K=1 To 300 : P[K] : Next : Print Param\nProcedure P[N]\nDim BIG(2000),S$(100)\nBIG(2000)=N : S$(100)=Str$(N)\nEnd Proc[BIG(2000)+Len(S$(100))]";
    assert_eq!(run(p), " 304\n");
    // Big arrays: the memory grows.
    let p = "Dim A(60000),B#(60000),C(60000)\nA(60000)=1 : B#(60000)=2 : C(60000)=3\nPrint A(60000)+B#(60000)+C(60000)";
    assert_eq!(run(p), " 6\n");
    // Strings in arrays survive garbage collection.
    let p =
        "Dim S$(200)\nFor I=1 To 30000 : S$(I mod 200)=\"v\"+Str$(I) : T$=Str$(I)+\"x\" : Next\nPrint S$(0);S$(199);T$";
    assert_eq!(run(p), "v 30000v 29999 30000x\n");
}

#[test]
fn resident_and_linear_arrays_mix() {
    // An array used by an interpreted instruction (Input, Read with an
    // expression, Varptr...) stays in the interpreter.
    let p = "Dim A(3),B(3)\nFor I=0 To 3 : Read A(I) : B(I)=A(I)*2 : Next\nSort A(0)\nPrint A(0);A(3);B(3);Match(A(0),4)\nData 4,3,2,1";
    assert_eq!(run(p), " 1 4 2 3\n");
    let p = "Dim A(3)\nA(1)=7\nV=Varptr(A(1)) : Print V>0;A(1)\nInc A(1) : Swap A(1),A(2) : Print A(1);A(2)";
    same(p);
    let p = "Dim A(2)\nDef Fn F(X)=A(X)*10\nA(1)=4 : Print Fn F(1)";
    assert_eq!(run(p), " 40\n");
    let o = same_with("Dim N(2)\nInput N(1),N(2)\nPrint N(1)+N(2);Match(N(0),5)", &["2,5"]);
    assert_eq!(o.text, "? \n 7 2\n");
}

#[test]
fn loop_fast_paths_keep_the_control_stack_exact() {
    let progs = [
        // Error in a While condition evaluated after Wend, handled; Resume
        // evaluates it again.
        "On Error Goto H\nI=0\nWhile 10/(3-I)>0\nInc I\nWend\nPrint I\nEnd\nH: Print \"e\";I; : I=4 : Resume",
        "On Error Goto H\nI=0\nWhile 10/(3-I)>0\nInc I\nWend\nPrint I\nEnd\nH: Print \"e\";I; : Resume Next",
        // Error with Trap, and a handler that leaves through Gosub / Return.
        "I=0\nWhile I<5\nInc I\nTrap A=10/(I-3)\nWend\nPrint I;Errtrap",
        "Gosub S : Print \"back\" : End\nS: On Error Goto H\nI=0\nWhile 10/(2-I)>0\nInc I\nWend\nReturn\nH: Print \"h\"; : Return",
        // Nested loops of every kind with Exit and Goto out of them.
        "For A=1 To 3\nI=0\nWhile I<3\nInc I : J=0\nRepeat\nInc J : If J=2 Then Exit 2\nUntil J>5\nWend\nK=0\nDo\nInc K : If K=3 Then Goto OUT\nLoop\nOUT: Print A;I;J;K;\nNext",
        "C=0\nEvery 1 Gosub E\nI=0\nWhile I<20000 : Inc I : Wend\nRepeat : Dec I : Until I=0\nEvery Off\nPrint C>0;I\nEnd\nE: Inc C : Every On : Return",
    ];
    for p in progs {
        same(p);
        // (With a few instructions per frame an Every handler never ends.)
        let budgets: &[usize] = if p.contains("Every") { &[40, 97] } else { &[1, 2, 3, 5, 9] };
        for &budget in budgets {
            same_budget(p, budget);
        }
    }
}

#[test]
fn native_jumps_data_and_loops_on_elements() {
    let progs = [
        // On n Goto / Gosub / Proc, including out of range and procedures
        // with parameters (left unset by On ... Proc).
        "For I=0 To 4\nOn I Goto A,B,C\nPrint \"none\";I;\nNX: Next\nEnd\nA: Print \"a\"; : Goto NX\nB: Print \"b\"; : Goto NX\nC: Print \"c\"; : Goto NX",
        "For I=0 To 3 : On I Gosub A,B : Print I; : Next : End\nA: Print \"a\"; : Return\nB: Print \"b\"; : Return",
        "Global G\nFor I=1 To 3 : On I Proc P1,P2,P3 : Next\nPrint G\nProcedure P1\nG=G+1\nEnd Proc\nProcedure P2[X]\nG=G+X+10\nEnd Proc\nProcedure P3[Global G]\nPrint \"p3\";G\nEnd Proc",
        // Computed Goto / Gosub.
        "For I=1 To 3\nL$=\"L\"+Str$(I)-\" \"\nGosub L$\nNext\nGoto 100\nL1: Print 1; : Return\nL2: Print 2; : Return\nL3: Print 3; : Return\n100 Print \"end\"",
        "N=20 : Goto N*2\n20 Print \"no\"\n40 Print \"forty\"",
        "Goto \"NOWHERE\"",
        // Read / Restore with constants (signs, floats, strings, empty).
        "Dim A(5),N$(2)\nFor I=0 To 5 : Read A(I) : Next\nRead X#,Y,N$(1),Z#\nRestore D2 : Read Q\nPrint A(0);A(5);X#;Y;N$(1);Z#;Q\nData 1,-2,$10,%11,-1.5,2\nD2: Data 7,3.9,\"txt\",,8",
        "Read A$\nData 5",
        "Read A,B\nData 1",
        "P\nRead X : Print X\nProcedure P\nRead A,B : Print A;B\nData 3,4\nEnd Proc\nData 9",
        // For on array elements (in memory and kept in the interpreter).
        "Dim A(3)\nFor A(2)=1 To 5 Step 2 : Print A(2); : Next\nPrint A(2)",
        "Dim F#(3)\nFor F#(1)=1 To 3 : Print F#(1); : Next",
        "Dim A(3)\nV=Varptr(A(0))\nFor A(1)=3 To 1 Step -1 : Print A(1); : Next",
        "Dim A(3)\nI=1 : For A(I)=1 To 3 : Inc I : Next",
    ];
    for p in progs {
        same(p);
        for budget in [1, 3, 7] {
            same_budget(p, budget);
        }
    }
}

#[test]
fn def_fn_compiled() {
    let progs = [
        "Def Fn SQ(X)=X*X\nPrint Fn SQ(4);Fn SQ(1.5)",
        "Def Fn F(X,Y)=X*10+Y\nA=3 : Print Fn F(A,4);Fn F(1,2);X;Y",
        "Def Fn S$(A$)=A$+A$\nPrint Fn S$(\"ab\")",
        "Print Fn G(1)\nDef Fn G(X)=X",
        "For A=1 To 0 Step -1\nIf A\nDef Fn H(X)=X+1\nElse\nDef Fn H(X)=X*2.5\nEnd If\nPrint Fn H(3);\nNext",
        "Def Fn D(X)=10/X\nPrint Fn D(5) : Print Fn D(0)",
        "Dim T(3) : T(2)=7\nDef Fn E(I)=T(I)*2\nPrint Fn E(2)",
        "P[2]\nProcedure P[N]\nDef Fn Q(X)=X+N\nPrint Fn Q(1)\nEnd Proc",
    ];
    for p in progs {
        same(p);
        same_budget(p, 2);
    }
}

#[test]
fn number_text_functions() {
    // Str$ of floats under every Fix, single and double precision.
    for double in [false, true] {
        let mut p = String::new();
        if double {
            p.push_str("Set Double Precision\n");
        }
        p.push_str("Dim V#(11)\nV#(0)=0 : V#(1)=1 : V#(2)=-1.5 : V#(3)=3.14159265 : V#(4)=1/3.0 : V#(5)=123456789.0\n");
        p.push_str("V#(6)=0.000012345 : V#(7)=-98765.4321 : V#(8)=1E+20 : V#(9)=2.5E-30 : V#(10)=7 : V#(11)=0.1\n");
        p.push_str(
            "For F=-17 To 17\nFix F\nFor I=0 To 11 : A$=Str$(V#(I)) : Print A$;\"|\"; : Next\nPrint\nNext F\nFix 16\n",
        );
        p.push_str("For I=1 To 300 : X#=I*1.37-150 : X#=X#*X#*X#/97 : Print Str$(X#);Str$(Val(Str$(X#))); : Next\n");
        assert_eq!(same(&p).end, StopReasonOrError::Stop(StopReason::End), "{p}");
    }
    // Val of all kinds of texts.
    let texts = [
        "12",
        " -12",
        "+7",
        "1 2 3",
        "1.5",
        ".5",
        "5.",
        "1e3",
        "1E-3",
        "1e",
        "1e+",
        "2.5e 2",
        "- 3.5",
        "$FF",
        "$ff",
        "-$10",
        "$123456789",
        "%101",
        "% 1 0 1",
        "%",
        "$",
        "abc",
        "",
        "  ",
        "99999999999",
        "2147483647",
        "-2147483648",
        "1..2",
        "3x",
        "0.000001",
        "123456789012",
        "1e40",
        "1e-50",
        "12345.678e-2",
        "0",
        "-0",
        "-0.0",
        "1e0005",
        "0.5e00001",
    ];
    for double in [false, true] {
        let mut p = String::new();
        if double {
            p.push_str("Set Double Precision\n");
        }
        for t in texts {
            p.push_str(&format!("A$=\"{t}\" : V=Val(A$) : W#=Val(A$) : Print Val(A$);V;W#\n"));
        }
        same(&p);
    }
    // Hex$, Bin$, Repeat$.
    let p = "For I=-3 To 40\nPrint Hex$(I*123457);Hex$(I*123457,I);Bin$(I*77);Bin$(-I,I)\nNext\nPrint Repeat$(\"ab\",3);Len(Repeat$(\"\",0))\nPrint Repeat$(\"x\",207)";
    let o = same(p);
    assert_eq!(o.end, StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL));
    // Values of Val used as numbers (dynamic type).
    same(
        "A=Val(\"3\")+Val(\"2.5\") : B#=Val(\"1e3\")/Val(\"$10\") : Print A;B#;Val(\"7\")*Val(\"7\");Str$(Val(\"4.5\"));Str$(Val(\"4\"))",
    );
}

#[test]
fn gosub_and_return() {
    let progs = [
        // Recursive Gosub down to Out of stack space (error 13, same depth).
        "N=0\nGosub R\nEnd\nR: Inc N : Print N; : Gosub R : Return",
        "N=0\nGosub R\nPrint \"back\";N\nEnd\nR: Inc N : If N<40 Then Gosub R\nReturn",
        // Return drops the loops opened inside the Gosub.
        "For I=1 To 3 : Gosub L : Print I; : Next : End\nL: For J=1 To 5 : If J=2 Then Return\nNext J : Return",
        "For I=1 To 3\nGosub L\nNext I\nPrint \"ok\";I\nEnd\nL: Repeat : K=K+1 : If K mod 2=0 Then Return\nUntil K>100 : Return",
        "I=0\nWhile I<3 : Inc I : Gosub L : Wend : Print I;C\nEnd\nL: Do : Inc C : If C mod 3=0 Then Return\nLoop",
        // Return without Gosub (error 1), also inside a procedure.
        "Return",
        "Gosub L : Print \"x\" : End\nL: P : Return\nProcedure P\nReturn\nEnd Proc",
        "P\nProcedure P\nGosub Q : Print \"q\" : End Proc\nQ: Print \"in\"; : Return\nEnd Proc",
        // Pop (error 2 without Gosub).
        "Gosub L : Print \"not here\" : End\nL: Pop : Print \"popped\" : End",
        "Pop",
        "For I=1 To 3 : Gosub L : Next : End\nL: Pop : Goto M\nM: Print \"m\";I : End",
        // Every Gosub while a Gosub is running, and test points in Return.
        "C=0\nEvery 1 Gosub E\nFor I=1 To 3000 : Gosub S : Next\nEvery Off\nPrint C>0;T\nEnd\nS: T=T+1 : Return\nE: Inc C : Every On : Return",
        // Errors inside a Gosub, handled; Resume / Resume Next.
        "On Error Goto H\nFor I=1 To 4 : Gosub S : Print R; : Next\nEnd\nS: R=10/(I-2) : Return\nH: R=-1 : Resume Next",
        "On Error Goto H\nN=0\nGosub S : Print \"after\";N\nEnd\nS: Inc N : A=1/(N-1) : Return\nH: N=3 : Resume",
        "Trap Gosub S\nPrint Errtrap\nEnd\nS: Print 1/0 : Return",
        // On..Gosub, computed Gosub, nested in procedures.
        "For I=1 To 3 : On I Gosub A,B,C : Next : End\nA: Print \"a\"; : Gosub B : Return\nB: Print \"b\"; : Return\nC: Print \"c\"; : Return",
        "For I=1 To 3 : Gosub \"L\"+Chr$(48+I) : Next : End\nL1: Print 1; : Return\nL2: Print 2; : Return\nL3: Print 3; : Return",
        "P[3]\nProcedure P[N]\nGosub L\nIf N>0 Then P[N-1]\nPrint N;\nEnd Proc\nL: Print \"g\";N; : Return\nEnd Proc",
        "P\nProcedure P\nFor I=1 To 3 : Gosub L : Next\nEnd Proc\nL: Print I; : If I=2 Then Pop Proc\nReturn\nEnd Proc",
        // Gosub out of a loop, Goto back in, and a yield inside.
        "For I=1 To 3\nGosub W\nNext\nPrint \"done\"\nEnd\nW: Wait Vbl : Print I; : Return",
        "Gosub A : Print \"end\" : End\nA: Gosub B : Return\nB: For K=1 To 2 : Gosub C : Next : Return\nC: Print K; : Return",
    ];
    for p in progs {
        same(p);
        // (With a few instructions per frame an Every handler never ends.)
        let budgets: &[usize] = if p.contains("Every") { &[40, 97] } else { &[1, 2, 3, 5, 9] };
        for &budget in budgets {
            same_budget(p, budget);
        }
    }
}

#[test]
fn native_procedures() {
    let progs = [
        // Deep recursion down to Out of stack space (error 13) at the same
        // depth, with Gosubs and loops on the stack too.
        "R[0]\nProcedure R[N]\nPrint N;\nR[N+1]\nEnd Proc",
        "R[0]\nProcedure R[N]\nPrint N;\nGosub L\nPop Proc\nL: R[N+1] : Return\nEnd Proc",
        "R[0]\nProcedure R[N]\nFor I=1 To 2 : Print N; : R[N+1] : Next\nEnd Proc",
        "Global D\nR\nProcedure R\nInc D : Print D;\nRepeat : R : Until D<0\nEnd Proc",
        "R[0]\nPrint \"back\"\nProcedure R[N]\nOn Error Goto H\nR[N+1]\nPop Proc\nH: Print \"h\";N;Errn : Resume Next\nEnd Proc",
        "On Error Goto H\nR[0]\nEnd\nH: Print \"main\";Errn : Resume Next\nProcedure R[N]\nR[N+1]\nEnd Proc",
        // Mutual recursion, Param of each kind.
        "ISEVEN[7] : Print Param\nISEVEN[10] : Print Param\nProcedure ISEVEN[N]\nIf N=0 Then Pop Proc[True]\nISODD[N-1]\nEnd Proc[Param]\nProcedure ISODD[N]\nIf N=0 Then Pop Proc[False]\nISEVEN[N-1]\nEnd Proc[Param]",
        "A[4]\nProcedure A[N]\nS$=\"a\"+Str$(N)\nIf N>0 Then B[N-1]\nPrint S$;\nEnd Proc\nProcedure B[N]\nT$=\"b\"+Str$(N)\nIf N>0 Then A[N-1]\nPrint T$;\nEnd Proc",
        "P[1.5,3]\nPrint Param#;Param\nProcedure P[X#,N]\nY#=X#*2\nIf N>0 Then P[Y#,N-1] : Y#=Y#+Param#\nEnd Proc[Y#]",
        "Set Double Precision\nP[1.1,6]\nPrint Param#\nProcedure P[X#,N]\nY#=X#/3\nIf N>0 Then P[Y#,N-1] : Y#=Y#+Param#\nEnd Proc[Y#]",
        "P[3]\nPrint Param$\nProcedure P[N]\nIf N=0 Then Pop Proc[\"z\"]\nP[N-1]\nEnd Proc[Param$+Str$(N)]",
        "P[2]\nPrint Param;Param#;Param$\nProcedure P[N]\nIf N>0 Then P[N-1]\nIf N=0 Then Pop Proc[7]\nIf N=1 Then Pop Proc[2.5]\nEnd Proc[\"s\"+Str$(N)]",
        "P[1] : Print Param;Param#\nP[2] : Print Param;Param#\nProcedure P[N]\nEnd Proc[Abs(N*1.5)]",
        "P : Print Param\nQ : Print Param\nProcedure P\nEnd Proc[5]\nProcedure Q\nEnd Proc",
        // Pop Proc out of loops and Gosubs.
        "P\nPrint Param\nProcedure P\nFor I=1 To 10 : If I=4 Then Pop Proc[I]\nNext\nEnd Proc",
        "P\nPrint Param\nProcedure P\nGosub L\nPop Proc[1]\nL: Pop Proc[2]\nEnd Proc",
        "P[2]\nProcedure P[N]\nGosub L\nPrint N;\nPop Proc\nL: If N>0 Then P[N-1]\nReturn\nEnd Proc",
        "P[3]\nProcedure P[N]\nWhile N>0 : Q[N] : Dec N : Wend\nEnd Proc\nProcedure Q[M]\nDo : Print M; : Pop Proc : Loop\nEnd Proc",
        // Local arrays (freed on return), string locals, garbage collection.
        "P[3]\nProcedure P[N]\nDim A(N)\nFor I=0 To N : A(I)=N*10+I : Next\nIf N>0 Then P[N-1]\nFor I=0 To N : Print A(I); : Next : Print\nEnd Proc",
        "For K=1 To 60 : P[K] : Next : Print \"ok\"\nProcedure P[N]\nDim A$(N*10)\nA$(N)=Str$(N)\nIf N mod 7=0 Then Q[N]\nEnd Proc\nProcedure Q[N]\nDim B#(N)\nB#(1)=N/2\nPrint B#(1);\nEnd Proc",
        "For K=1 To 20 : P[40] : Next\nPrint \"ok\"\nProcedure P[N]\nS$=String$(\"x\",200)+Str$(N)\nIf N>0 Then P[N-1]\nIf Right$(S$,Len(Str$(N)))<>Str$(N) Then Print \"bad\"\nEnd Proc",
        "For I=1 To 2000 : P[I] : A$=A$+Left$(Param$,2) : Next : Print Len(A$);Right$(A$,8)\nProcedure P[N]\nEnd Proc[Str$(N)+Space$(50)]",
        // Shared / Global variables and global parameters.
        "A=1 : B$=\"x\"\nP[3]\nPrint A;B$\nProcedure P[N]\nShared A,B$\nA=A*2 : B$=B$+\"y\"\nIf N>0 Then P[N-1]\nEnd Proc",
        "Global G,H$\nP[5,\"q\"]\nPrint G;H$\nProcedure P[G,H$]\nIf G>0 Then P[G-1,H$+\"r\"]\nEnd Proc",
        "Global T()\nDim T(10)\nP[10]\nFor I=0 To 10 : Print T(I); : Next\nProcedure P[N]\nT(N)=N*N\nIf N>0 Then P[N-1]\nEnd Proc",
        // Errors and handlers inside procedures.
        "P[3]\nPrint \"end\"\nProcedure P[N]\nOn Error Goto H\nA=10/(N-2)\nPrint A;\nIf N>0 Then P[N-1]\nPop Proc\nH: Print \"err\";Errn;N; : Resume Next\nEnd Proc",
        "On Error Proc H\nA=1/0\nPrint \"x\";Param\nProcedure H\nQ[2] : Print Param;\nResume Next\nEnd Proc\nProcedure Q[N]\nIf N>0 Then Q[N-1]\nEnd Proc[N*3+Param]",
        "On Error Proc H\nA=1/0\nPrint \"x\"\nProcedure H\nQ[1]\nEnd Proc\nProcedure Q[N]\nEnd Proc",
        "P[3]\nProcedure P[N]\nOn Error Proc H\nIf N=0 Then A=1/0\nIf N>0 Then P[N-1]\nPrint N;\nEnd Proc\nProcedure H\nPrint \"handler\";Errn;\nResume Next\nEnd Proc",
        "P\nProcedure P\nOn Error Goto H\nA=1/0\nPrint \"no\"\nPop Proc\nH: Resume L\nL: Print \"label\" : Q[2] : Print Param\nEnd Proc\nProcedure Q[N]\nEnd Proc[N+1]",
        "P[2]\nProcedure P[N]\nTrap A=1/N\nPrint Errtrap;\nIf N>0 Then P[N-1]\nEnd Proc",
        "P[3]\nProcedure P[N]\nIf N=0 Then Error 42\nP[N-1]\nEnd Proc",
        "P[3]\nProcedure P[N]\nIf N=0 Then Return\nP[N-1]\nEnd Proc",
        // Data / Read scoping.
        "Read A : P[2] : Read B : Print A;B\nData 1,2\nProcedure P[N]\nRead X : Print X;\nIf N>0 Then P[N-1]\nRead Y : Print Y;\nData 10,20,30\nEnd Proc",
        "P[2] : Read A : Print A\nData 5\nProcedure P[N]\nIf N>0 Then P[N-1]\nRestore L\nRead X : Print X;\nPop Proc\nL: Data 7\nEnd Proc",
        // Yields, waits and the interpreter inside procedures.
        "P[3]\nProcedure P[N]\nWait Vbl\nPrint N;\nIf N>0 Then P[N-1]\nWait 2\nEnd Proc",
        "For I=1 To 2 : On I Proc A,B : Next\nProcedure A\nPrint \"a\";\nEnd Proc\nProcedure B\nPrint \"b\";\nEnd Proc",
        "FIB[12] : Print Param\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]",
        // Many locals (cleared with memory.fill).
        "P[3]\nProcedure P[N]\nPrint V7;V19$;\nV0=N+0 : V1=N+1 : V2=N+2 : V3=N+3 : V4=N+4\nV5=N+5 : V6=N+6 : V7=N+7 : V8=N+8 : V9=N+9\nV10=N+10 : V11=N+11 : V12=N+12 : V13=N+13 : V14=N+14\nV15=N+15 : V16=N+16 : V17=N+17 : V18=N+18 : V19=N+19 : V19$=Str$(N)\nIf N>0 Then P[N-1]\nPrint V7;V19;V19$;\nEnd Proc",
        // Every Proc during recursion (its End Proc sets Param too).
        "C=0\nEvery 1 Proc E\nFIB[16] : Print Param\nEvery Off\nPrint C>0\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]\nProcedure E\nShared C\nInc C\nEvery On\nEnd Proc",
        "C=0\nEvery 1 Proc E\nFor K=1 To 30 : R[K] : Next\nEvery Off\nPrint C>0;Param\nProcedure R[N]\nIf N>0 Then R[N-1]\nEnd Proc[N]\nProcedure E\nShared C\nInc C : Q[C]\nEvery On\nEnd Proc[C]\nProcedure Q[M]\nEnd Proc[-M]",
    ];
    for p in progs {
        same(p);
        // (With a few instructions per frame an Every handler never ends.)
        let budgets: &[usize] = if p.contains("Every") { &[40, 97] } else { &[1, 2, 3, 5, 9, 17] };
        for &budget in budgets {
            same_budget(p, budget);
        }
    }
}

#[test]
fn double_precision_val_and_input() {
    // `AscToDouble` (not correctly rounded): equal results only if the same
    // bits; the differences show in comparisons and digits.
    let progs = [
        "Set Double Precision\nRead N\nFor I=1 To N : Read A$ : V#=Val(A$) : Print A$;\"|\";V#;V#=Val(A$+\" \");V#*1e15-Int(V#*1e15) : Next\nData 14,\"0.86\",\"0.1\",\"123.456\",\"-0\",\"-0.0\",\"1e308\",\"1e309\",\"-1e400\",\"1e-330\",\"9.4e-3\",\" - 12 . 5 e 2\",\"1.2345678901234567890123456789012345e10\",\"123456789012345678901234567890123456\",\"3.14159265358979\"",
        "Set Double Precision\nA#=Val(\"0.1\")+Val(\"0.2\") : B#=Val(\"0.3\")\nPrint A#=B#;A#-B#;Val(\".86\")=0.86\nC#=0.86 : Print C#=Val(\"0.86\")",
        "Set Double Precision\nF#=0\nFor I=1 To 300 : F#=F#+Val(Str$(I)+\".\"+Str$(I*7)) : Next\nPrint F#",
        "R#=Val(\"1234567890123456789012345678901234567\") : Print R#;Val(\"-0\");Val(\"1.5e3\")",
    ];
    for p in progs {
        same(p);
        same_budget(p, 7);
    }
    // Input of doubles.
    same_with("Set Double Precision\nInput A#,B#\nPrint A#*3;B#;A#=B#", &["0.86", "8.6e-1"]);
}

#[test]
fn string_space_repeat_boundaries() {
    for n in ["0", "1", "255", "65535", "65536", "65537", "70000", "131072"] {
        run(&format!(
            "A$=String$(\"xy\",{n}) : B$=Space$({n}) : C$=String$(\"\",{n})\nPrint Len(A$);Len(B$);Len(C$);Left$(A$,3);\"|\";Right$(B$,2);\"|\""
        ));
    }
    for n in ["0", "1", "9", "206"] {
        run(&format!(
            "A$=Repeat$(\"ab\",{n}) : B$=Repeat$(\"\",{n}) : C$=Repeat$(Space$(100),{n})\nPrint Len(A$);Len(B$);Len(C$);Asc(Right$(A$,1))"
        ));
    }
    for p in [
        "A$=Space$(-1)",
        "A$=String$(\"a\",-5)",
        "A$=Repeat$(\"a\",207)",
        "A$=Repeat$(\"a\",-1)",
        "A$=String$(\"\",-1)",
    ] {
        assert_eq!(run_err(p), StopReasonOrError::Error(errors::ILLEGAL_FUNCTION_CALL), "{p}");
    }
}

#[test]
fn on_with_computed_targets() {
    let progs = [
        // Numeric labels (constants resolved at compile time).
        "For J=0 To 5 : On J Gosub 1,2,3,1 : Next : Print : End\n1 Print \"a\"; : Return\n2 Print \"b\"; : Return\n3 Print \"c\"; : Return",
        "For J=1 To 3 : On J Goto 10,20,30\n10 Print 10; : Goto 40\n20 Print 20; : Goto 40\n30 Print 30;\n40 Next : Print",
        // Names as strings, computed values, a missing label (error 40).
        "For J=1 To 2 : On J Gosub \"L1\",\"l2\" : Next : End\nL1: Print \"one\"; : Return\nL2: Print \"two\"; : Return",
        "A=2 : B$=\"X\" : On 2 Gosub A+8,B$ : Print \"back\" : End\n10 Print \"ten\" : Return\nX: Print \"x\" : Return",
        "On 2 Goto 1,99\n1 Print 1",
        "On 1 Goto \"nowhere\"",
        // In a procedure (its own labels), and Every during it.
        "P[2]\nProcedure P[N]\nOn N Gosub 1,2 : Print \"done\"\nPop Proc\n1 Print \"p1\"; : Return\n2 Print \"p2\"; : Return\nEnd Proc",
        "Every 1 Gosub E\nFor I=1 To 2000 : On I mod 3+1 Gosub 1,2,3 : Next\nEvery Off : Print C;D\nEnd\n1 Inc D : Return\n2 Return\n3 Return\nE: Inc C : Every On : Return",
    ];
    for p in progs {
        same(p);
        let budgets: &[usize] = if p.contains("Every") { &[40, 97] } else { &[1, 2, 3, 7] };
        for &b in budgets {
            same_budget(p, b);
        }
    }
}

#[test]
fn bit_operations_on_variables() {
    // (Bset... are machine instructions: on full machines.)
    let mut src = String::from("V=$12345678 : W=-1 : Z=0 : C=0\n");
    for op in ["Bset", "Bclr", "Bchg", "Ror.b", "Ror.w", "Ror.l", "Rol.b", "Rol.w", "Rol.l"] {
        for n in ["0", "1", "3", "7", "8", "15", "16", "31", "32", "33", "-1", "-9", "2.6"] {
            src.push_str(&format!("{op} {n},V : {op} {n},W : {op} {n},Z : C=C xor V xor W xor Z : Rol.l 5,C\n"));
        }
    }
    src.push_str("Dim A(3) : A(1)=5 : Bset 3,A(1) : D=A(1)\n");
    src.push_str("Print Hex$(V);\" \";Hex$(W);\" \";Hex$(Z);\" \";Hex$(C);\" \";D\nF#=1.5 : Bset 1,F#");
    let (mut a, mut b) = machines(&src, 5);
    assert!(global_value(&a.interp, "c").is_some(), "{:?} {:?}", a.state, a.hw.log);
    let png = |m: &mut amos_core::Machine| amos_core::display::render_rgba(&m.frame());
    assert!(png(&mut a) == png(&mut b), "displays differ");
    assert_eq!(a.state, b.state);
}

#[test]
fn loop_regions() {
    let progs = [
        // Nested loops of every kind, single instruction bodies.
        "For I=1 To 3 : Next : For J=1 To 2 : For K=1 To 2 : Print J;K; : Next : Next : Print I",
        "I=0 : Repeat : J=0 : While J<3 : Inc J : Do : Inc K : Exit If K mod 4=0 : Loop : Wend : Inc I : Until I=3 : Print I;J;K",
        // Backward Gotos crossing loops (merged regions), Goto into a loop.
        "N=0\nL1: For I=1 To 3\nInc N : If N=5 Then Goto L2\nNext I\nGoto L1\nL2: Print N;I",
        "A=0\nFor I=1 To 3\nL: Inc A\nIf A<10 and I=2 Then Goto L\nNext\nPrint A;I",
        "A=0 : Goto M\nRepeat\nM: Inc A\nUntil A>5\nPrint A",
        "I=0\nTOP: Inc I : J=0\nW: Inc J : If J<3 Then Goto W\nIf I<4 Then Goto TOP\nPrint I;J",
        // Loops in procedures, Gosubs in loops, an Every handler in a loop.
        "P[3]\nProcedure P[N]\nFor I=1 To N : For J=1 To I : S=S+J : Next : Next : Print S\nEnd Proc",
        "For I=1 To 3 : Gosub L : Next : Print C : End\nL: For K=1 To 4 : Inc C : Next : Return",
        "Every 1 Gosub E\nFor I=1 To 3000 : A=A+1 : Next : Every Off : Print A;B>0 : End\nE: For K=1 To 3 : Inc B : Next : Every On : Return",
    ];
    for p in progs {
        same(p);
        let budgets: &[usize] = if p.contains("Every") { &[40, 97] } else { &[1, 2, 3, 5, 9] };
        for &b in budgets {
            same_budget(p, b);
        }
    }
}
