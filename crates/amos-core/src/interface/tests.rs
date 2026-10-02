//! Tests of the Interface engine and of the dialog instructions.

use super::engine::prepass;
use super::*;
use crate::Machine;
use crate::interp::RunState;

#[test]
fn prepass_labels_and_user_instructions() {
    let p = b"LA 1;SV 0,1;JP 2;LA 2;EX;UI AB,2;[PR P1,P2,'x',1;]\0";
    let (labels, users) = prepass(p).unwrap();
    assert_eq!(labels.len(), 2);
    assert_eq!(labels[0].0, 1);
    assert_eq!(&p[labels[0].1..labels[0].1 + 2], b"SV");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].0, *b"AB");
    assert_eq!(users[0].1, 2);
    assert_eq!(p[users[0].2], b'P');
}

#[test]
fn prepass_errors() {
    // Label defined twice.
    assert_eq!(prepass(b"LA 1;LA 1;EX;\0").unwrap_err().0, e::LABEL_DEFINED);
    // Unknown function.
    assert_eq!(prepass(b"SV 0,DX;EX;\0").unwrap_err().0, e::SYNTAX);
    // Not enough parameters.
    assert_eq!(prepass(b"PR 1,2;EX;\0").unwrap_err().0, e::NPARAM);
    // Lower case letters are invisible.
    assert!(prepass(b"SIze 10,20; BAse 0,0; EXit;\0").is_ok());
    // The default resource programs are valid.
    for p in &Resource::default_resource().programs {
        let mut v = p.to_vec();
        v.push(0);
        prepass(&v).unwrap();
    }
}

/// Runs a program for at most `frames` frames.
fn run(src: &str, frames: usize) -> Machine {
    let prg = crate::tokenise::tokenise_program(src.as_bytes()).expect("tokenise");
    let mut m = Machine::new();
    m.run_program(&prg).expect("verify");
    step(&mut m, frames);
    m
}

fn step(m: &mut Machine, frames: usize) {
    for _ in 0..frames {
        m.vbl();
        // The dispatcher's test point may not call the dialogs yet.
        let _ = m.hw.dialogs_test_point(&mut m.interp);
        if matches!(m.state, RunState::Stopped(_)) {
            break;
        }
    }
}

fn stopped(m: &Machine) -> String {
    match &m.state {
        RunState::Stopped(_) => m.hw.log.last().cloned().unwrap_or_default(),
        s => format!("{s:?}"),
    }
}

/// Puts the mouse on screen coordinates of screen `n`.
fn mouse_at(m: &mut Machine, n: usize, x: i32, y: i32) {
    let s = m.hw.screens.get(n).unwrap();
    let (hx, hy) = (s.x_hard(x), s.y_hard(y));
    m.hw.input.mouse_x = hx;
    m.hw.input.mouse_y = hy;
}

/// Value of an integer variable.
fn vint(m: &mut Machine, name: &str) -> i32 {
    match var(m, name) {
        crate::interp::value::Value::Int(n) => n,
        v => panic!("{name} = {v:?}"),
    }
}

/// Value of a string variable.
fn vstr(m: &mut Machine, name: &str) -> Vec<u8> {
    match var(m, name) {
        crate::interp::value::Value::Str(s) => s.to_vec(),
        v => panic!("{name} = {v:?}"),
    }
}

/// Value of a global variable ("A", "B$").
fn var(m: &mut Machine, name: &str) -> crate::interp::value::Value {
    use crate::interp::value::{Value, Var};
    let (base, ty) = match name.strip_suffix('$') {
        Some(b) => (b, 2),
        None => (name, 0),
    };
    let prg = m.interp.prg.clone().unwrap();
    let i = prg
        .globals
        .iter()
        .position(|g| g.ty == ty && !g.array && g.name.eq_ignore_ascii_case(base.as_bytes()))
        .unwrap_or_else(|| panic!("no variable {name}"));
    match &m.interp.globals[i] {
        Var::Scalar(v) => v.clone(),
        _ => Value::zero(ty),
    }
}

#[test]
fn dialog_box_button_click() {
    let src = "Screen Open 0,320,200,8,0\n\
               A$=\"BU 1,16,16,40,10,0,0,1;[GB 0,0,40,10;][BQ;]RU 0,3;EX;\"\n\
               X=Dialog Box(A$)\n\
               Print X\n";
    let mut m = run(src, 5);
    assert_eq!(stopped(&m), "Running", "{:?}", m.hw.log);
    // Drawn in ink 2 (pen of the default window).
    let s = m.hw.screens.get(0).unwrap();
    assert_ne!(s.pixel(20, 20).unwrap(), s.pixel(100, 100).unwrap());
    mouse_at(&mut m, 0, 20, 20);
    m.hw.input.mouse_buttons = 1;
    step(&mut m, 3);
    m.hw.input.mouse_buttons = 0;
    step(&mut m, 5);
    assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
    assert_eq!(vint(&mut m, "X"), 1);
}

#[test]
fn dialog_box_timer_and_variables() {
    // VA0 = v, VA1 = v$, timer exits with 0.
    let src = "Screen Open 0,320,200,8,0\n\
               X=Dialog Box(\"SV 3,0VA 2*;BR 3VA;IF 1VA TL 3=;[SV 4,1;]RU 5,0;EX;\",21,\"abc\")\n";
    let mut m = run(src, 20);
    assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
    assert_eq!(vint(&mut m, "X"), 0);
}

#[test]
fn dialog_open_run_and_values() {
    let src = "Screen Open 0,320,200,8,0\n\
               Dialog Open 1,\"SV 2,'hello';BU 7,0,0,16,16,5,0,9;[][]EX;LA 3;SV 1,42;EX;\",8\n\
               X=Dialog Run(1)\n\
               A=Rdialog(1,7)\n\
               B$=Vdialog$(1,2)\n\
               C=Vdialog(1,1)\n\
               D=Dialog(1)\n\
               Y=Dialog Run(1,3)\n\
               E=Vdialog(1,1)\n\
               Vdialog(1,5)=99\n\
               F=Vdialog(1,5)\n\
               Dialog Close\n";
    let mut m = run(src, 10);
    assert!(
        matches!(m.state, RunState::Stopped(_)),
        "{:?} {:?}",
        m.state,
        m.hw.log
    );
    assert_eq!(vint(&mut m, "X"), 0);
    assert_eq!(vint(&mut m, "A"), 5);
    assert_eq!(vstr(&mut m, "B$"), b"hello");
    assert_eq!(vint(&mut m, "C"), 0);
    assert_eq!(vint(&mut m, "D"), 0);
    assert_eq!(vint(&mut m, "E"), 42);
    assert_eq!(vint(&mut m, "F"), 99);
}

#[test]
fn interface_errors() {
    let m = run(
        "Screen Open 0,320,200,8,0\nDialog Open 1,\"SV 0,DX;EX;\"\n",
        5,
    );
    assert!(
        stopped(&m).contains("Interface error: bad syntax"),
        "{}",
        stopped(&m)
    );
    let mut m = run(
        "Screen Open 0,320,200,8,0\nOn Error Goto E\nDialog Open 1,\"SV 0,1;SV 1,1 0/;EX;\"\nX=Dialog Run(1)\nEnd\nE: P=Edialog : N=Errn : Resume Next\n",
        5,
    );
    assert_eq!(vint(&mut m, "N"), 120);
    assert_eq!(vint(&mut m, "P"), 16);
    let m = run("Dialog Close 3\n", 5);
    assert!(
        stopped(&m).contains("channel not defined"),
        "{}",
        stopped(&m)
    );
}

#[test]
fn user_instructions_and_subroutines() {
    // UI with parameters, JS/RT, IF blocks, labels.
    let src = "Screen Open 0,320,200,8,0\n\
               Dialog Open 1,\"JP 1;UI AD,2;[SV P1,P1 VA P2 +;]LA 1;SV 0,10;AD 0,5;JS 2;IF 0VA 18=;[SV 2,1;]EX;LA 2;AD 0,3;RT;\"\n\
               X=Dialog Run(1)\n\
               A=Vdialog(1,0) : B=Vdialog(1,2)\n";
    let mut m = run(src, 5);
    assert!(
        matches!(m.state, RunState::Stopped(_)),
        "{:?} {:?}",
        m.state,
        m.hw.log
    );
    assert_eq!(vint(&mut m, "A"), 18);
    assert_eq!(vint(&mut m, "B"), 1);
}

#[test]
fn edit_zone_typing() {
    let src = "Screen Open 0,320,200,8,0\n\
               Dialog Open 1,\"ED 1,16,16,20,20,'ab',0,2;DI 2,16,40,6,12,1,0,2;EX;\"\n\
               X=Dialog Run(1)\n\
               Do : Wait Vbl : Loop\n";
    let mut m = run(src, 5);
    m.hw.input.put_key(b"cd\r");
    step(&mut m, 3);
    m.hw.input.put_key(b"9");
    step(&mut m, 3);
    let v = m.hw.dia_get_value(1, 1, 1).unwrap();
    assert!(matches!(&v, DVal::Str(s) if &s[..] == b"abcd"), "{v:?}");
    let v = m.hw.dia_get_value(1, 2, 1).unwrap();
    assert!(matches!(v, DVal::Int(129)), "{v:?}");
}

#[test]
fn slider_and_list() {
    let src = "Screen Open 0,320,200,8,0\n\
               Dim A$(9) : For I=0 To 9 : A$(I)=\"Item\"+Str$(I) : Next\n\
               Dialog Open 1,\"HS 1,0,0,100,8,0,10,100,5;[]AL 2,0,16,10,4,0VA,0,0,0,2;[]EX;\"\n\
               Vdialog(1,0)=Array(A$(0))\n\
               X=Dialog Run(1)\n\
               Dialog Update 1,1,50\n\
               A=Rdialog(1,1)\n\
               Do : Wait Vbl : Loop\n";
    let mut m = run(src, 5);
    assert_eq!(stopped(&m), "Running", "{:?}", m.hw.log);
    assert_eq!(vint(&mut m, "A"), 50);
    // Click the second line of the list.
    mouse_at(&mut m, 0, 4, 16 + 8 + 2);
    m.hw.input.mouse_buttons = 1;
    step(&mut m, 2);
    m.hw.input.mouse_buttons = 0;
    step(&mut m, 2);
    let v = m.hw.dia_get_value(1, 2, 1).unwrap();
    assert!(matches!(v, DVal::Int(1)), "{v:?}");
    let ch = &m.hw.dialogs.channels[0];
    assert_eq!(ch.ret, 2);
    // The text of the list is printed in the window.
    let s = m.hw.screens.get(0).unwrap();
    assert!((0..80).any(|x| s.pixel(x, 18).unwrap() == 2));
}

#[test]
fn resource_functions() {
    let mut m = run("A$=Resource$(1) : B$=Resource$(-12) : C$=Resource$(0)\n", 5);
    assert_eq!(vstr(&mut m, "A$"), b"File Selector");
    assert_eq!(
        vstr(&mut m, "B$"),
        b"AMOSPro_Accessories:AMOSPro_Help/AMOSPro_Help"
    );
    let m = run("Resource Bank 5 : A$=Resource$(1)\n", 5);
    assert!(stopped(&m).contains("Bank not reserved"), "{}", stopped(&m));
}

#[test]
fn hslider_draws() {
    let m = run(
        "Screen Open 0,320,200,8,0 : Set Slider 1,1,3,0,4,4,4,0 : Hslider 10,10 To 110,20,100,0,50\n",
        5,
    );
    let s = m.hw.screens.get(0).unwrap();
    assert_eq!(s.pixel(20, 15).unwrap(), 4);
    assert_eq!(s.pixel(90, 15).unwrap(), 1);
    assert_eq!(s.pixel(90, 10).unwrap(), 3);
    let m = run("Hslider 10,10 To 110,20,100,101,50\n", 5);
    assert!(
        stopped(&m).contains("Illegal function call"),
        "{}",
        stopped(&m)
    );
}

#[test]
fn file_selector_typed_name() {
    let mut m = Machine::new();
    m.hw.files.write("Ram:one.txt", b"1").unwrap();
    m.hw.files.write("Ram:dir/two.txt", b"22").unwrap();
    let prg =
        crate::tokenise::tokenise_program(b"F$=Fsel$(\"Ram:\",\"\",\"Pick\")\nG$=Dir$\n").unwrap();
    m.run_program(&prg).unwrap();
    step(&mut m, 30);
    assert_eq!(stopped(&m), "Running", "{:?}", m.hw.log);
    // The list: the directory first.
    let f = m.hw.dialogs.fsel.as_ref().unwrap();
    assert_eq!(f.list.len(), 2);
    assert_eq!(f.list[0].text, b"*dir");
    assert_eq!(f.list[1].text, b" one.txt");
    assert!(m.hw.screens.get(10).is_some());
    m.hw.input.put_key(b"abc\r");
    step(&mut m, 40);
    assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
    assert_eq!(vstr(&mut m, "F$"), b"Ram:abc");
    assert!(m.hw.screens.get(10).is_none());
}

#[test]
fn file_selector_cancel_key() {
    let mut m = Machine::new();
    let prg = crate::tokenise::tokenise_program(b"F$=Fsel$(\"Ram:\")\n").unwrap();
    m.run_program(&prg).unwrap();
    step(&mut m, 30);
    // Leave the edit zones with Tab, then Escape (KY 27 on Cancel).
    m.hw.input.put_key(b"\t");
    step(&mut m, 2);
    m.hw.input.put_key(b"\t");
    step(&mut m, 2);
    m.hw.input.put_key(&[27]);
    step(&mut m, 40);
    assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
    assert_eq!(vstr(&mut m, "F$"), b"");
}

#[test]
fn read_text_hypertext_keyword() {
    let mut m = Machine::new();
    let text = b"#HYP1\r\n\nLine one\n{[Topic]Go to topic}\nLast\n";
    m.hw.files.write("Ram:t.txt", text).unwrap();
    let prg = crate::tokenise::tokenise_program(b"Read Text \"Ram:t.txt\"\nP$=Param$\n").unwrap();
    m.run_program(&prg).unwrap();
    step(&mut m, 30);
    assert_eq!(stopped(&m), "Running", "{:?}", m.hw.log);
    // "#HYPn" and the next 3 bytes are skipped: the active word is on the
    // second line.
    let q = m.hw.dialogs.read_text.as_ref().unwrap().channel;
    let ch = &m.hw.dialogs.channels[m.hw.dialogs.channel_index(q).unwrap()];
    let z = ch.find_zone(5, 1).unwrap();
    let Some(ZoneKind::Text(t)) = ch.zone(z).map(|z| &z.kind) else {
        panic!()
    };
    let row = &t.rows[1];
    assert_eq!(row.zones.len(), 1);
    let hz = row.zones[0];
    let (zx, zy) = (ch.zone(z).unwrap().x as i32, ch.zone(z).unwrap().y as i32);
    mouse_at(&mut m, 10, zx + hz.c0 as i32 * 8 + 4, zy + 8 + 3);
    m.hw.input.mouse_buttons = 1;
    step(&mut m, 2);
    m.hw.input.mouse_buttons = 0;
    step(&mut m, 40);
    assert!(matches!(m.state, RunState::Stopped(_)), "{:?}", m.state);
    assert_eq!(vstr(&mut m, "P$"), b"Topic");
}
