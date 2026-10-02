//! More differential tests: compiled and interpreted runs must agree.

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
