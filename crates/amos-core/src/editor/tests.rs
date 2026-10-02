//! Editor tests driven through the machine with simulated key presses.

use super::*;
use crate::input::InputEvent;

fn key(m: &mut Machine, scancode: u8, ch: Option<char>) {
    m.input(InputEvent::Key { scancode, pressed: true, ch });
    m.input(InputEvent::Key { scancode, pressed: false, ch: None });
}

fn with_shift(m: &mut Machine, shift: u8, scancode: u8, ch: Option<char>) {
    m.input(InputEvent::Key { scancode: shift, pressed: true, ch: None });
    key(m, scancode, ch);
    m.input(InputEvent::Key { scancode: shift, pressed: false, ch: None });
}

fn type_text(ed: &mut Editor, m: &mut Machine, s: &str) {
    for c in s.chars() {
        if c == '\n' {
            key(m, raw::RETURN, Some('\r'));
        } else {
            m.input(InputEvent::Char(c));
        }
        // The keyboard buffer holds 31 keys.
        ed.vbl(m);
    }
    ed.vbl(m);
}

fn frames(ed: &mut Editor, m: &mut Machine, n: usize) {
    for _ in 0..n {
        ed.vbl(m);
    }
}

fn listing(ed: &Editor) -> String {
    latin1_to_string(&crate::detok::list_program(&ed.doc().to_program()))
}

#[test]
fn starts_with_editor_screen() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    frames(&mut ed, &mut m, 2);
    let s = m.hw.screens.get(EC_EDIT).expect("editor screen");
    assert_eq!((s.width, s.height), (640, 256));
    assert_eq!(m.hw.screens.priority[0], EC_EDIT);
    // The status line shows the template.
    assert_eq!(ed.mode, Mode::Edit);
}

#[test]
fn type_run_and_return() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\nprint a+1\n");
    assert_eq!(listing(&ed), "A=1\nPrint A+1\n");
    key(&mut m, raw::F1, None);
    ed.vbl(&mut m);
    assert_eq!(ed.mode, Mode::Running);
    assert!(m.hw.screens.get(EC_EDIT).is_none());
    frames(&mut ed, &mut m, 5);
    // End of program: the Direct / Editor line.
    assert!(matches!(ed.mode, Mode::Stopped(_)), "{:?}", ed.mode);
    assert!(m.hw.screens.get(EC_FONC).is_some());
    key(&mut m, raw::RETURN, Some('\r'));
    frames(&mut ed, &mut m, 2);
    assert_eq!(ed.mode, Mode::Edit);
    assert!(m.hw.screens.get(EC_EDIT).is_some());
    assert!(m.hw.screens.get(EC_FONC).is_none());
}

#[test]
fn runtime_error_goes_to_line() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\nb=0\nprint a/b\n");
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 5);
    assert!(matches!(ed.mode, Mode::Stopped(_)));
    key(&mut m, raw::RETURN, Some('\r'));
    frames(&mut ed, &mut m, 2);
    assert_eq!(ed.mode, Mode::Edit);
    assert_eq!(ed.doc().y, 2);
    assert_eq!(ed.current_alert(), Some("Division by zero."));
}

#[test]
fn test_time_error() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1\nnext i\n");
    key(&mut m, 0x51, None);
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.doc().y, 1);
    assert_eq!(ed.current_alert(), Some("NEXT without FOR."));
    // F1 shows the same error without running.
    key(&mut m, raw::UP, None);
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.mode, Mode::Edit);
    assert_eq!(ed.doc().y, 1);
}

#[test]
fn direct_mode() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=41\n");
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 5);
    // Esc on the stop line: direct mode with the program's variables.
    key(&mut m, raw::ESC, Some('\u{1b}'));
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.mode, Mode::Direct);
    type_text(&mut ed, &mut m, "b=a+1\n");
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.mode, Mode::Direct);
    type_text(&mut ed, &mut m, "if b<>42 then error 23\n");
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.mode, Mode::Direct);
    assert!(ed.direct.output.iter().all(|l| !l.starts_with(b"Illegal")), "{:?}", ed.direct.output);
    type_text(&mut ed, &mut m, "next\n");
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.direct.output.last().map(|l| latin1_to_string(l)), Some("NEXT without FOR".into()));
    // Esc goes back to the editor.
    key(&mut m, raw::ESC, Some('\u{1b}'));
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.mode, Mode::Edit);
    assert!(m.hw.screens.get(EC_EDIT).is_some());
}

#[test]
fn blocks_and_undo_with_keys() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\nb=2\nc=3\n");
    // Ctrl+Shift+Up: top of text, Ctrl+B, Down, Ctrl+B, Ctrl+C (cut).
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    with_shift(&mut m, raw::LSHIFT, raw::UP, None);
    key(&mut m, 0x35, Some('\u{2}'));
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    key(&mut m, raw::DOWN, None);
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    key(&mut m, 0x35, Some('\u{2}'));
    key(&mut m, 0x33, Some('\u{3}'));
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    frames(&mut ed, &mut m, 2);
    assert_eq!(listing(&ed), "C=3\n");
    // Ctrl+P pastes above the cursor line.
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    key(&mut m, 0x19, Some('\u{10}'));
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    frames(&mut ed, &mut m, 1);
    assert_eq!(listing(&ed), "A=1\nB=2\nC=3\n");
    // Ctrl+U undoes the paste.
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    key(&mut m, 0x16, Some('\u{15}'));
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    frames(&mut ed, &mut m, 1);
    assert_eq!(listing(&ed), "C=3\n");
}

#[test]
fn save_and_load() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print \"saved\"\n");
    ed.save_as(&mut m, "Ram Disk:test").unwrap();
    assert_eq!(ed.doc().name, "Ram Disk:test.AMOS");
    ed.function(&mut m, 1080);
    assert_eq!(listing(&ed), "");
    ed.load(&mut m, "Ram Disk:test.AMOS").unwrap();
    assert_eq!(listing(&ed), "Print \"saved\"\n");
    // Load through the status line prompt (Amiga+L).
    ed.function(&mut m, 1080);
    with_shift(&mut m, raw::LAMIGA, 0x28, Some('l'));
    frames(&mut ed, &mut m, 1);
    type_text(&mut ed, &mut m, "Ram Disk:test.AMOS\n");
    assert_eq!(listing(&ed), "Print \"saved\"\n");
}

#[test]
fn menu_runs_functions() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1\n");
    frames(&mut ed, &mut m, 1);
    let s = m.hw.screens.get(EC_EDIT).unwrap();
    let (hx, hy) = (s.display_x, s.display_y);
    let to_display = |x: i32, y: i32| {
        // Hires screen: 2 screen pixels per hardware pixel.
        let (hwx, hwy) = (hx + x / 2, hy + y);
        (((hwx - crate::display::HW_X0) * 2) as f32 + 0.5, ((hwy - crate::display::HW_Y0) * 2) as f32 + 0.5)
    };
    // Right button over "Project", then over "Test" (second item).
    let (x, y) = to_display(16, 4);
    m.input(InputEvent::MouseMove { x, y });
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Right, pressed: true });
    frames(&mut ed, &mut m, 1);
    assert!(ed.menu.open);
    let (x, y) = to_display(24, 10 + 9 + 4);
    m.input(InputEvent::MouseMove { x, y });
    frames(&mut ed, &mut m, 1);
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Right, pressed: false });
    frames(&mut ed, &mut m, 1);
    assert!(!ed.menu.open);
    assert_eq!(ed.current_alert(), Some("No errors"));
}

fn display_pos(m: &Machine, x: i32, y: i32) -> (f32, f32) {
    let s = m.hw.screens.get(EC_EDIT).unwrap();
    let (hwx, hwy) = (s.display_x + x / 2, s.display_y + y);
    (((hwx - crate::display::HW_X0) * 2) as f32 + 0.5, ((hwy - crate::display::HW_Y0) * 2) as f32 + 0.5)
}

#[test]
fn click_places_cursor() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\nprint \"hello\"\nb=2\n");
    // Text starts at y = 16 (top bar) + 11 (status line).
    let (x, y) = display_pos(&m, 6 * 8 + 2, 27 + 8 + 3);
    m.input(InputEvent::MouseMove { x, y });
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: true });
    frames(&mut ed, &mut m, 1);
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: false });
    frames(&mut ed, &mut m, 1);
    assert_eq!((ed.doc().y, ed.doc().x), (1, 6));
}

#[test]
fn search_prompt_and_fold_keys() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1\nprocedure TEST\nprint 2\nend proc\nprint 3\n");
    // Top of text (Ctrl+Shift+Up), then Amiga+F, the string, Return.
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    with_shift(&mut m, raw::LSHIFT, raw::UP, None);
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    with_shift(&mut m, raw::LAMIGA, 0x23, Some('f'));
    frames(&mut ed, &mut m, 1);
    type_text(&mut ed, &mut m, "print 2\n");
    assert_eq!(ed.doc().y, 2);
    // F9 folds the procedure: the cursor goes to its first line.
    key(&mut m, 0x58, None);
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.doc().y, 1);
    assert_eq!(ed.doc().rows(), vec![0, 1, 4, 5]);
    // The folded procedure still runs.
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 5);
    assert!(matches!(&ed.mode, Mode::Stopped(i) if i.reason == StopReasonOrError::Stop(StopReason::End)));
}

#[test]
fn several_windows() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\n");
    // Amiga+Shift+W: a new window for a new program.
    m.input(InputEvent::Key { scancode: raw::LAMIGA, pressed: true, ch: None });
    with_shift(&mut m, raw::LSHIFT, 0x11, Some('W'));
    m.input(InputEvent::Key { scancode: raw::LAMIGA, pressed: false, ch: None });
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.docs.len(), 2);
    assert_eq!(ed.current, 1);
    type_text(&mut ed, &mut m, "b=2\n");
    // F6: previous window.
    key(&mut m, 0x55, None);
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.current, 0);
    assert_eq!(listing(&ed), "A=1\n");
    ed.current = 1;
    assert_eq!(listing(&ed), "B=2\n");
}
