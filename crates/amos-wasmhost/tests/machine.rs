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
