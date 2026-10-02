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
