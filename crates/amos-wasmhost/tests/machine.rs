//! Compiled programs on a full `Machine`: displays and state must match the
//! interpreter frame by frame.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use amos_core::Machine;
use amos_core::display::render_rgba;
use amos_core::tokenise::tokenise_program;
use amos_wasmhost::CompiledProgram;

fn compare(src: &str, frames: usize) {
    let prg = tokenise_program(src.as_bytes()).expect("tokenise");
    let wasm = amos_compiler::compile(&prg).expect("compile");
    common::validate(&wasm);
    let mut a = Machine::new();
    a.run_program(&prg).expect("test");
    let mut b = Machine::new();
    let mut cp = CompiledProgram::start(&mut b, &prg, &wasm).expect("start");
    for f in 0..frames {
        a.vbl();
        cp.vbl(&mut b);
        assert_eq!(a.state, b.state, "state at frame {f}");
        assert_eq!(a.hw.log, b.hw.log, "log at frame {f}");
        assert!(render_rgba(&a.frame()) == render_rgba(&b.frame()), "display differs at frame {f}");
    }
}

#[test]
fn graphics_with_waits() {
    compare(
        "Screen Open 1,320,200,16,Lowres\nCls 0\nFor I=0 To 40\nInk I mod 16\nBar I*5,I*3 To I*5+20,I*3+20\nCircle 160,100,I*2\nWait Vbl\nNext\nPrint \"done\"",
        60,
    );
}

#[test]
fn text_and_functions() {
    compare(
        "For I=1 To 20\nLocate 0,I : Print \"Line\";I;\" \";Screen Width;Rnd(0)\nX=X Curs : Y=Y Curs\nWait 2\nNext\nPrint X;Y",
        60,
    );
}

#[test]
fn busy_program_time_slicing() {
    // No waits: the time budget makes both yield at the same instructions.
    compare("Do\nInc A\nIf A mod 1000=0 Then Plot A/1000 mod 300,50\nLoop", 20);
}

#[test]
fn every_and_procedures() {
    compare(
        "Every 5 Proc TICK\nFor I=1 To 50 : Wait Vbl : Next\nProcedure TICK\nShared N\nInc N : Locate 0,0 : Print N\nEvery On\nEnd Proc",
        60,
    );
}

#[test]
fn native_procedures_on_a_machine() {
    // Recursion with Every Proc firing inside, waits in procedures, and
    // variables kept in the interpreter (Varptr) as locals and parameters.
    compare(
        "Every 3 Proc TICK\nFor K=1 To 12 : FIB[K] : Print K;Param; : P[2] : Q[2] : Next\nEvery Off\nProcedure FIB[N]\nIf N<2 Then Pop Proc[N]\nFIB[N-1] : A=Param\nFIB[N-2]\nEnd Proc[A+Param]\nProcedure P[N]\nA=N : V=Varptr(A)\nPrint A;V>0;\nIf N>0 Then P[N-1]\nEnd Proc\nProcedure Q[N]\nV=Varptr(N)\nPrint N;\nIf N>0 Then Q[N-1]\nWait Vbl\nEnd Proc\nProcedure TICK\nShared T\nInc T : Locate 0,0 : Print T;\nEvery On\nEnd Proc[T]",
        120,
    );
}

#[test]
fn modules_of_other_programs_are_refused() {
    // The application falls back to the interpreter when this fails.
    let prg = tokenise_program(b"Print 1").unwrap();
    let other = amos_compiler::compile(&tokenise_program(b"Print 2").unwrap()).unwrap();
    let mut m = Machine::new();
    assert!(CompiledProgram::start(&mut m, &prg, &other).is_err());
    assert!(CompiledProgram::start(&mut m, &prg, b"not wasm").is_err());
    // A bundle carries the module and still runs interpreted without it.
    let mut b = amos_core::bundle::Bundle {
        files: vec![("p.txt".into(), b"Print 1".to_vec())],
        main: "p.txt".into(),
        module: None,
    };
    let plain = b.clone();
    b.module = Some(amos_compiler::compile(&prg).unwrap());
    let back = amos_core::bundle::Bundle::from_bytes(&b.to_bytes()).unwrap();
    assert!(CompiledProgram::start(&mut m, &back.program().unwrap(), back.module.as_ref().unwrap()).is_ok());
    assert_eq!(amos_core::bundle::Bundle::from_bytes(&plain.to_bytes()).unwrap().module, None);
}

/// Every keyword compiled code calls directly (`machine::plain_args`, the
/// parameters given with `Interp::preset_args`): all parameters, the first
/// one omitted, a float for an integer, a negative value (errors), angles
/// after Degree, and functions without parameters as parameters, on full
/// machines, against the interpreter (state, log and display).
#[test]
fn plain_keywords_match_the_interpreter() {
    use amos_core::tokens::{Keyword, MAIN, TokenKind, ValueType};
    let mut programs = Vec::new();
    for def in MAIN {
        let kw = Keyword { slot: 0, token: def.token };
        if !amos_core::machine::plain_args(kw) {
            continue;
        }
        let sig = def.param_types().as_bytes();
        let types: Vec<u8> = sig.iter().step_by(2).copied().collect();
        let seps: Vec<u8> = sig.iter().skip(1).step_by(2).copied().collect();
        let value = |i: usize, t: u8| match t {
            b'2' => "\"a\"".to_string(),
            b'1' => format!("{}.5", i + 1),
            b'5' => "0.5".to_string(),
            _ => format!("{}", i + 1),
        };
        let join = |vals: &[String]| {
            let mut s = String::new();
            for (i, v) in vals.iter().enumerate() {
                s.push_str(v);
                match seps.get(i) {
                    Some(b't') => s.push_str(" To "),
                    Some(_) => s.push(','),
                    None => {}
                }
            }
            s
        };
        let mut variants: Vec<Vec<String>> = vec![types.iter().enumerate().map(|(i, &t)| value(i, t)).collect()];
        if types.len() >= 2 {
            let mut v = variants[0].clone();
            v[0] = String::new();
            variants.push(v);
        }
        // (Parameters of the wrong type are refused when the program is
        // verified, the same way for both: run time conversions and errors
        // instead.)
        if let Some(&t) = types.first()
            && t != b'2'
        {
            let mut v = variants[0].clone();
            v[0] = "2.7".into();
            variants.push(v);
            let mut v = variants[0].clone();
            v[0] = "-1".into();
            variants.push(v);
        }
        // Functions without parameters as parameters (evaluated by the
        // runtime in direct calls, `layout::BRIDGE_CALL`), alone, with
        // constants, and with a parameter the module evaluates.
        if !types.is_empty() {
            let fused = |i: usize, t: u8| match t {
                b'2' => "Inkey$".to_string(),
                _ => ["X Mouse", "Y Mouse", "Mouse Key"][i % 3].to_string(),
            };
            variants.push(types.iter().enumerate().map(|(i, &t)| fused(i, t)).collect());
            let mut v = variants[0].clone();
            v[0] = fused(0, types[0]);
            variants.push(v);
            if types.len() >= 2 && types[1] != b'2' {
                let mut v = variants[0].clone();
                v[0] = fused(0, types[0]);
                v[1] = "Rnd(3)".into();
                variants.push(v);
            }
        }
        let name = def.name;
        for (k, vals) in variants.iter().enumerate() {
            let args = join(vals);
            let call = match def.kind() {
                TokenKind::Instruction => format!("{name} {args}"),
                kind => {
                    let target = match kind {
                        TokenKind::Function(ValueType::Str) => "V$",
                        TokenKind::ReservedVariable if def.params.as_bytes().get(1) == Some(&b'2') => "V$",
                        _ => "V",
                    };
                    if types.is_empty() { format!("{target}={name}") } else { format!("{target}={name}({args})") }
                }
            };
            programs.push((name, format!("Curs Off\n{call}\nPrint V;V$\n{call}")));
            if k == 0 && types.contains(&b'5') {
                programs.push((name, format!("Degree\n{call}\nPrint V;V$")));
            }
        }
    }
    assert!(programs.len() > 100, "{}", programs.len());
    let mut compared = 0;
    let mut failures = Vec::new();
    // Machine panics (bugs of the machine, the same both ways) are listed.
    let mut panics = Vec::new();
    let run = |src: &str, compiled: bool| -> Option<Vec<String>> {
        let prg = tokenise_program(src.as_bytes()).ok()?;
        let wasm = amos_compiler::compile(&prg).ok()?;
        let mut m = Machine::new();
        let mut cp = None;
        if compiled {
            cp = Some(CompiledProgram::start(&mut m, &prg, &wasm).ok()?);
        } else {
            m.run_program(&prg).ok()?;
        }
        // Some input for the functions read in the parameters.
        m.input(amos_core::input::InputEvent::MouseMove { x: 200.0, y: 120.0 });
        for c in "kz".chars() {
            m.input(amos_core::input::InputEvent::Char(c));
        }
        let mut frames = Vec::new();
        for _ in 0..3 {
            match &mut cp {
                Some(cp) => cp.vbl(&mut m),
                None => m.vbl(),
            }
            let h = render_rgba(&m.frame()).iter().fold(0u64, |h, &b| h.wrapping_mul(31).wrapping_add(b as u64));
            frames.push(format!("{:?} {:?} {h}", m.state, m.hw.log));
        }
        Some(frames)
    };
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut covered = std::collections::BTreeSet::new();
    for (name, src) in &programs {
        let a = std::panic::catch_unwind(|| run(src, false));
        let b = std::panic::catch_unwind(|| run(src, true));
        match (a, b) {
            (Ok(Some(a)), Ok(Some(b))) => {
                compared += 1;
                covered.insert(*name);
                if a != b {
                    failures.push(format!("{src}\n  {a:?}\n  {b:?}"));
                }
            }
            (Ok(None), Ok(None)) => {}
            (Err(_), Err(_)) => panics.push(src.clone()),
            (a, b) => {
                failures.push(format!("{src}\n  interpreted ok/panic: {:?}, compiled: {:?}", a.is_ok(), b.is_ok()))
            }
        }
    }
    std::panic::set_hook(hook);
    let skipped: Vec<&String> = programs
        .iter()
        .map(|(_, src)| src)
        .filter(|src| {
            let Ok(prg) = tokenise_program(src.as_bytes()) else { return true };
            amos_compiler::compile(&prg).is_err() || Machine::new().run_program(&prg).is_err()
        })
        .collect();
    eprintln!(
        "{} programs, {compared} compared ({} keywords), {} not runnable: {:#?}",
        programs.len(),
        covered.len(),
        skipped.len(),
        &skipped[..skipped.len().min(15)]
    );
    if !panics.is_empty() {
        eprintln!("machine panics (both ways): {panics:#?}");
    }
    assert!(compared > 150 && covered.len() > 40, "only {compared} programs ({} keywords) compared", covered.len());
    assert!(failures.is_empty(), "{} of {compared} differ:\n{}", failures.len(), failures.join("\n"));
}

/// The same input on both machines before frame `f`: mouse moves and
/// buttons, typed characters, keys held (Esc, cursor keys for the joystick
/// emulation), game controllers.
fn feed(m: &mut Machine, f: usize) {
    use amos_core::input::{InputEvent, MouseButton, raw};
    m.input(InputEvent::MouseMove { x: (40 + f * 37 % 600) as f32, y: (30 + f * 23 % 400) as f32 });
    if f % 3 == 1 {
        m.input(InputEvent::Char((b'a' + (f % 26) as u8) as char));
    }
    if f % 7 == 2 {
        m.input(InputEvent::Char('x'));
        m.input(InputEvent::Char('y'));
    }
    for (key, on, off) in [(raw::ESC, 2, 5), (raw::UP, 4, 9), (raw::LEFT, 6, 7), (0x60, 1, 6), (0x63, 3, 4)] {
        if f % 10 == on {
            m.input(InputEvent::Key { scancode: key, pressed: true, ch: None });
        }
        if f % 10 == off {
            m.input(InputEvent::Key { scancode: key, pressed: false, ch: None });
        }
    }
    let button = if f.is_multiple_of(2) { MouseButton::Left } else { MouseButton::Right };
    m.input(InputEvent::MouseButton { button, pressed: f % 8 >= 3 && f % 8 < 6 });
    m.hw.input.set_gamepad(0, (f * 5 % 32) as u8);
    m.hw.input.set_gamepad(1, (f * 3 % 32) as u8);
}

/// `compare` with input fed to both machines before every frame.
fn compare_with_input(src: &str, frames: usize) {
    let prg = tokenise_program(src.as_bytes()).expect("tokenise");
    let wasm = amos_compiler::compile(&prg).expect("compile");
    common::validate(&wasm);
    let mut a = Machine::new();
    a.run_program(&prg).expect("test");
    let mut b = Machine::new();
    let mut cp = CompiledProgram::start(&mut b, &prg, &wasm).expect("start");
    for f in 0..frames {
        feed(&mut a, f);
        feed(&mut b, f);
        a.vbl();
        cp.vbl(&mut b);
        assert_eq!(a.state, b.state, "state at frame {f}:\n{src}");
        assert_eq!(a.hw.log, b.hw.log, "log at frame {f}:\n{src}");
        assert!(render_rgba(&a.frame()) == render_rgba(&b.frame()), "display differs at frame {f}:\n{src}");
    }
}

/// The polling functions the module reads from its input mirror
/// (`layout::IN_VALID`): values, key buffer side effects of Inkey$, the
/// timer, joystick emulation, Scin, and everything that changes them
/// (statements, other functions, Every handlers, interpreted
/// instructions), tight loops and once per frame.
#[test]
fn polling_functions_match_the_interpreter() {
    let progs = [
        // Once per frame.
        "Curs Off\nDo\nA$=Inkey$ : If A$<>\"\" Then Print A$;Scancode;Scanshift;\nPrint X Mouse;Y Mouse;Mouse Key;Joy(0);Joy(1);Jup(1);Fire(0);Key State(69);Key State(76);Key Shift;Timer\nWait Vbl\nLoop",
        // Tight polling loops (the budget ends the frame).
        "Curs Off\nDo\nInc N : A$=Inkey$ : If A$<>\"\" Then Print N mod 97;A$;\nIf Mouse Key Then Inc M\nIf N mod 5000=0 Then Print M;Timer;Joy(1);Key State(76);Key Shift;\nLoop",
        "Curs Off\nDo : N=Scin(X Mouse,Y Mouse) : K=Mouse Key : Inc C : If C mod 3000=0 Then Print N;K;\nLoop",
        "Curs Off\nDo : A$=Inkey$ : J=Joy(1) : T=Timer : K=Key State(69) : Inc C : If A$<>\"\" or C mod 4000=0 Then Print A$;J;T;K;\nLoop",
        "Curs Off\nDo\nRepeat : Inc W : Until Mouse Key<>0 or W>20000\nPrint W;Mouse Key; : W=0\nWait Vbl\nLoop",
        // The program changes what the functions read.
        "Curs Off\nDo\nX Mouse=X Mouse+3 : Print X Mouse;\nY Mouse=50 : Print Y Mouse;\nPut Key \"ab\" : Print Inkey$;Inkey$;Inkey$=\"\";\nTimer=Timer+5 : Print Timer;\nClear Key : Print Inkey$=\"\";\nWait Vbl\nLoop",
        "Curs Off\nLimit Mouse 150,60 To 160,70\nDo : Print X Mouse;Y Mouse; : A=X Mouse+Y Mouse : Limit Mouse : B=X Mouse : Print A;B; : Limit Mouse 150,60 To 160,70 : Wait Vbl : Loop",
        "Curs Off\nDo : C=Mouse Click : K=Mouse Key : If C Then Print C;K;\nI$=Inkey$ : S=Scancode : If S Then Print S;\nLoop",
        "Curs Off\nDo\nPrint Inkey$+Inkey$;Len(Inkey$+Inkey$+Inkey$);\nWait Vbl\nLoop",
        // Joy / Key State with every kind of parameter, errors.
        "Curs Off\nDo\nFor P=-1 To 3 : Print Joy(P); : Next\nFor K=60 To 80 : Print Key State(K); : Next\nPrint Joy(1.7);Key State(69.2)\nWait Vbl\nLoop",
        "Curs Off\nWait 5\nPrint Key State(128)",
        "Curs Off\nWait 5\nK=-1 : Print Key State(K)",
        // Scin while the screens change (moved, opened, closed).
        "Curs Off\nScreen Open 1,320,100,16,Lowres\nDo\nScreen Display 1,,40+(T mod 4)*30,,\nS=Scin(X Mouse,Y Mouse) : R=Scin(200,80) : Inc T\nFor I=1 To 3 : Print Scin(X Mouse,Y Mouse);Scin(200,80+I*10); : Next\nWait Vbl\nLoop",
        "Curs Off\nScreen Open 1,320,100,16,Lowres : Screen Display 1,,60,,\nDo : S=Scin(200,80) : Inc C : If C=3000 Then Screen Close 1\nIf C mod 2000=0 Then Print S;\nIf C=6000 Then Screen Open 1,320,50,16,Lowres : Screen Display 1,,70,,\nLoop",
        "Screen Close 0\nWait 3\nPrint Scin(10,10)",
        // Every handlers and interpreted instructions in between.
        "Curs Off\nEvery 3 Gosub E\nDo : T=Timer : K=Key State(69) : If T<Q Then Print T;Q;\nQ=T : Loop\nE: Timer=0 : Clear Key : Every On : Return",
        "Curs Off\nDo : A$=Inkey$ : If A$<>\"\" Then Print A$;\nX=X Mouse : Inc N : If N mod 3000=0 Then Put Key \"q\"\nLoop",
        "Curs Off\nDo : If Key Shift=1 or Key Shift>2 Then Print Key Shift;\nInc N : If N mod 4000=0 Then Print Key Shift<>0;\nLoop",
        // Mouse Zone kept while the mirror is valid; zones and screens change.
        "Curs Off\nReserve Zone 4 : Set Zone 1,0,0 To 100,60 : Set Zone 2,100,0 To 200,100\nDo : Z=Mouse Zone : Inc N\nIf N mod 3000=0 Then Print Z;Mouse Zone;\nIf N mod 7000=0 Then Set Zone 3,0,60 To 320,200\nIf N mod 11000=0 Then Reset Zone 1\nIf N mod 13000=0 Then Screen Display 0,,40+(N mod 3)*10,,\nLoop",
    ];
    for p in progs {
        compare_with_input(p, 40);
    }
}

/// Peek / Poke & co called directly (`plain_args`) on memory mapped to
/// variables by Varptr (the interpreter's variables: Varptr keeps them in
/// the interpreter, `structure::resident_vars`) and on banks; Zone, Hzone,
/// Mouse Zone, Scancode, Screen as functions.
#[test]
fn memory_and_zone_functions_match_the_interpreter() {
    let progs = [
        "Curs Off\nA=5 : P=Varptr(A) : Print Leek(P) : Loke P,1234 : Print A\nDoke P+2,7 : Print A;Deek(P);Peek(P+3) : Poke P+3,9 : Print A;Leek(P)",
        "Curs Off\nB#=1.5 : P=Varptr(B#) : Print Leek(P) : Loke P,Leek(P)+256 : Print B#",
        "Curs Off\nDim T(3) : T(1)=77 : P=Varptr(T(0)) : Print Leek(P+4) : Loke P+8,99 : Print T(2);T(1)",
        "Curs Off\nS$=\"hello\" : P=Varptr(S$) : Print Peek(P);Peek(P+4) : Poke P,72 : Print S$",
        "Curs Off\nA=0 : P=Varptr(A) : For I=1 To 300 : Loke P,Leek(P)+I : Next : Print A\nFor I=1 To 300 : A=A-1 : Q=Leek(P) : Next : Print A;Q",
        "Curs Off\nTEST[3]\nProcedure TEST[N]\nL=N*10 : P=Varptr(L) : Loke P,Leek(P)*2 : Print L\nEnd Proc",
        "Curs Off\nReserve As Work 10,256 : S=Start(10)\nFor I=0 To 255 : Poke S+I,I : Next\nT=0 : For I=0 To 255 : T=T+Peek(S+I) : Next : Print T\nDoke S,$1234 : Loke S+4,-2 : Print Deek(S);Leek(S+4);Peek(S+1)",
        "Curs Off\nReserve Zone 3 : Set Zone 1,0,0 To 50,50 : Set Zone 2,60,0 To 120,40\nDo : Z=Zone(X Screen(X Mouse),Y Screen(Y Mouse)) : H=Hzone(X Mouse,Y Mouse) : M=Mouse Zone : S=Scancode : C=Screen\nInc N : If N mod 3000=0 Then Print Z;H;M;S;C;Zone(0,10,10);Zone(65,5);Hzone(0,200,100)\nLoop",
    ];
    for p in progs {
        compare_with_input(p, 12);
    }
}

/// Runs of plain instructions in one call on full machines: errors of the
/// handlers themselves (screen not opened, illegal values) inside a run,
/// handled with Resume Next, and drawing runs.
#[test]
fn keyword_batches_match_the_interpreter() {
    let progs = [
        "Curs Off\nOn Error Goto H\nFor I=0 To 3 : Ink I : Screen I : Locate I,I : Print I; : Next\nEnd\nH: Print \"e\";Errn; : Resume Next",
        "Curs Off\nDo : Ink 1 : Plot 1,1 : Draw 0,0 To 3,3 : Ink 2,3 : Locate 0,0 : Locate 1,2 : Inc N : If N mod 2000=0 Then Print N;\nLoop",
        "Curs Off\nOn Error Goto H\nFor I=1 To 5 : Ink 1 : Locate 0,I*12 : Plot I,I : Print I; : Next\nEnd\nH: Print \"h\"; : Resume Next",
    ];
    for p in progs {
        compare_with_input(p, 10);
    }
}

/// `Multi Wait` & co (nothing to do: skipped by compiled code) in polling
/// loops, and `Colour(n)` kept while the input mirror is valid, with the
/// palette changed by statements, by another screen becoming current and
/// between frames (Fade, Flash).
#[test]
fn multi_wait_and_colour_match_the_interpreter() {
    let progs = [
        "Curs Off\nDo : Multi Wait : Inc N : If Mouse Key Then Print N;\nAmos To Front : Amos Lock : Amos Unlock\nLoop",
        "Curs Off\nDo : C=Colour(1) : D=Colour(2) : Inc N\nIf N mod 3000=0 Then Print C;D;Colour(1);\nIf N mod 7000=0 Then Colour 1,N and $FFF\nIf N mod 11000=0 Then Palette $123,$456\nLoop",
        "Curs Off\nScreen Open 1,320,100,16,Lowres : Colour 1,$F00\nDo : Inc N : C=Colour(1) : If N mod 2000=0 Then Print C;\nIf N mod 5000=0 Then Screen N mod 2\nLoop",
        "Curs Off\nFade 3 To 1\nDo : C=Colour(1) : Inc N : If N mod 1000=0 Then Print C;\nLoop",
        "Curs Off\nFlash 1,\"(F00,2)(0F0,2)\"\nDo : C=Colour(1) : Multi Wait : Inc N : If N mod 1000=0 Then Print C;\nLoop",
        "Curs Off\nWait 3\nPrint Colour(40);Colour(-1)",
    ];
    for p in progs {
        compare_with_input(p, 12);
    }
}

/// The typed keyword functions called directly by compiled code
/// (`runtime/direct.rs`): string parameters borrowed from the module's
/// memory (constants, variables, results, empty, long), omitted
/// parameters, every form, errors; collisions and Dialog.
#[test]
fn direct_typed_keywords_match_the_interpreter() {
    let progs = [
        "Curs Off\nA$=\"var\" : For I=0 To 6 : Locate 0,I : Centre \"hello\"+Str$(I) : Next : Centre A$ : Centre \"\"\nCentre String$(\"x\",200) : Centre Mid$(\"abcdef\",2,3)",
        "Curs Off\nScreen Open 1,320,200,16,Lowres : Cls 0 : Ink 3\nFor I=0 To 9 : Text I*20,I*15+10,\"T\"+Str$(I) : Gr Locate I*10,I : Text ,,\"o\" : Next\nText 10,190,\"\" : Text ,50,\"y\"",
        "Curs Off\nScreen Open 1,320,100,16,Lowres : Screen Open 2,320,100,16,Lowres\nFor I=0 To 3 : Screen I mod 3 : Print Screen; : Next\nOn Error Goto H : Screen 5 : Print \"no\" : End\nH: Print \"e\";Errn : Resume Next",
        "Curs Off\nScreen Open 1,320,200,16,Lowres\nCls : Cls 2 : Cls 3,10,10 To 100,100 : Cls 4,-5,-5 To 400,400\nFor I=0 To 30 : Cls I mod 16,I,I To I+20,I+20 : Next",
        "Screen Open 0,320,200,16,Lowres : Curs Off : Cls 0\nInk 3 : Bar 0,0 To 15,15 : Get Bob 1,0,0 To 16,16 : Cls 0\nBob 2,100,50,1 : Bob 3,160,50,1\nFor I=0 To 40 : Bob 1,I*5,50,1 : Bob 4,,I, : Wait Vbl\nC=Bob Col(1) : D=Bob Col(1,3 To 3) : Print I;C;D;Col(2);Col(3)\nNext",
        "Screen Open 0,320,200,16,Lowres : Curs Off : Cls 0\nInk 3 : Bar 0,0 To 15,15 : Get Sprite 1,0,0 To 16,16 : Cls 0\nFor I=0 To 40 : Sprite 1,128+I*5,50+I*3,1 : Sprite 2,300-I*4,80,1 : Sprite 3,,,\nC=Sprite Col(1) : D=Sprite Col(2,1 To 3) : E=Bobsprite Col(1) : F=Spritebob Col(1,0 To 5)\nIf C or D or E or F Then Print I;C;D;E;F;\nWait Vbl : Next",
        "Curs Off\nOn Error Goto H\nPrint Dialog(1) : Print Dialog(-1)\nEnd\nH: Print \"e\";Errn; : Resume Next",
    ];
    for p in progs {
        compare_with_input(p, 50);
    }
}
