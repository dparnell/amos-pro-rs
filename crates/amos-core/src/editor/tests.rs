//! Editor tests driven through the machine with simulated key presses.

use super::*;
use crate::input::InputEvent;
use crate::input::raw;

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
fn display_rect_is_the_editor_screen() {
    let mut m = Machine::new();
    let ed = Editor::new(&mut m);
    let r = ed.display_rect(&m);
    assert_eq!((r.x, r.y, r.w, r.h), (64, 48, 640, 512));
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
    // End of program: the Direct / Editor line (`Ed_Ligne`): the editor
    // screen in front as a 56 line strip at Es_Y1, the program's screen
    // still shown.
    assert!(matches!(ed.mode, Mode::Stopped(_)), "{:?}", ed.mode);
    let s = m.hw.screens.get(EC_EDIT).expect("editor strip");
    assert_eq!((s.display_y, s.display_h), (ed.cfg.esc_y1 as i32, 56));
    assert_eq!(m.hw.screens.priority[0], EC_EDIT);
    assert!(m.hw.screens.get(0).is_some_and(|s| !s.hidden));
    assert!(ed.in_dialog());
    key(&mut m, raw::RETURN, Some('\r'));
    frames(&mut ed, &mut m, 2);
    assert_eq!(ed.mode, Mode::Edit);
    let s = m.hw.screens.get(EC_EDIT).unwrap();
    assert_eq!((s.display_y, s.display_h), (ed.cfg.wy as i32, 256));
    assert!(m.hw.screens.get(0).is_some_and(|s| s.hidden));
    // End: no message, the cursor stays where it was.
    assert_eq!(ed.current_alert(), None);
}

/// The texts of the stop line: message, line number, the line around the
/// error with `>>>`, as the dialog's variables give them.
#[test]
fn stop_line_texts() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "for i=1 to 3\nnext i\n  if i>2 then a=1 : print 1/0\n");
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 5);
    let i = m.hw.dialogs.channel_index(dialogs::ED_CHANNEL).expect("Ed_Ligne dialog");
    let v = &m.hw.dialogs.channels[i].vars;
    let st = |n: usize| match &v[n] {
        crate::interface::DVal::Str(s) => latin1_to_string(s),
        other => format!("{other:?}"),
    };
    assert_eq!(st(0), "Division by zero");
    assert!(matches!(v[1], crate::interface::DVal::Int(3)));
    assert_eq!(st(2), "2 Then A=1 : ");
    assert_eq!(st(3), "Print 1/0");
    // The dialog builds "message at line n." in variable 8.
    assert_eq!(st(8), "Division by zero at line 3.");
}

/// Ctrl-C (Program interrupted), then Esc: Direct mode.
#[test]
fn break_then_direct() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "do\nloop\n");
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.mode, Mode::Running);
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    key(&mut m, 0x33, Some('\u{3}'));
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    frames(&mut ed, &mut m, 3);
    assert!(matches!(&ed.mode, Mode::Stopped(i) if i.reason == StopReasonOrError::Stop(StopReason::Break)));
    let i = m.hw.dialogs.channel_index(dialogs::ED_CHANNEL).unwrap();
    assert!(
        matches!(&m.hw.dialogs.channels[i].vars[0], crate::interface::DVal::Str(s) if &s[..] == b"Program interrupted")
    );
    key(&mut m, raw::ESC, Some('\u{1b}'));
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.mode, Mode::Direct);
    assert!(m.hw.screens.get(EC_EDIT).is_none());
    assert!(m.hw.screens.get(EC_FONC).is_some());
}

/// The Edit and Direct instructions skip the line.
#[test]
fn edit_and_direct_instructions() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "edit\n");
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.mode, Mode::Edit);
    assert!(!ed.in_dialog());
    ed.function(&mut m, 1080);
    type_text(&mut ed, &mut m, "direct\n");
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 3);
    assert_eq!(ed.mode, Mode::Direct);
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
    frames(&mut ed, &mut m, 3);
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
    // Load through the file selector (Amiga+L): it lists the current
    // directory, the name is typed and Return chooses it.
    ed.function(&mut m, 1080);
    with_shift(&mut m, raw::LAMIGA, 0x28, Some('l'));
    frames(&mut ed, &mut m, 20);
    assert!(ed.in_dialog());
    assert!(m.hw.screens.get(10).is_some(), "file selector screen");
    let names: Vec<String> =
        m.hw.dialogs.fsel.as_ref().unwrap().list.iter().map(|e| latin1_to_string(&e.text)).collect();
    assert!(names.iter().any(|n| n.contains("test.AMOS")), "{names:?}");
    type_text(&mut ed, &mut m, "test.AMOS\n");
    frames(&mut ed, &mut m, 40);
    assert!(!ed.in_dialog());
    assert!(m.hw.screens.get(10).is_none());
    assert_eq!(listing(&ed), "Print \"saved\"\n");
    assert!(ed.doc().name.ends_with("test.AMOS"));
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
fn search_dialog_and_fold_keys() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1\nprocedure TEST\nprint 2\nend proc\nprint 3\n");
    // Top of text (Ctrl+Shift+Up), then Amiga+F, the string, Return.
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: true, ch: None });
    with_shift(&mut m, raw::LSHIFT, raw::UP, None);
    m.input(InputEvent::Key { scancode: raw::CTRL, pressed: false, ch: None });
    with_shift(&mut m, raw::LAMIGA, 0x23, Some('f'));
    frames(&mut ed, &mut m, 1);
    assert!(ed.in_dialog());
    // Return leaves the edit zone, the second one is the Ok shortcut.
    type_text(&mut ed, &mut m, "Print 2\n\n");
    assert!(!ed.in_dialog());
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

/// Opens a dialog with function `f` and checks it is shown on the editor
/// screen (the channel exists and the box was drawn).
fn open_dialog(ed: &mut Editor, m: &mut Machine, f: u16) {
    let before = m.hw.screens.get(EC_EDIT).unwrap().logic_ref().to_vec();
    ed.function(m, f);
    frames(ed, m, 2);
    assert!(ed.in_dialog(), "function {f}");
    assert!(m.hw.dialogs.channel_index(dialogs::ED_CHANNEL).is_some());
    assert_ne!(m.hw.screens.get(EC_EDIT).unwrap().logic_ref(), &before[..], "function {f} drew nothing");
}

#[test]
fn goto_line_and_set_tab_dialogs() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\nb=2\nc=3\nd=4\n");
    // Amiga+G: Goto Line.
    with_shift(&mut m, raw::LAMIGA, 0x24, Some('g'));
    frames(&mut ed, &mut m, 2);
    assert!(ed.in_dialog());
    type_text(&mut ed, &mut m, "3\n\n");
    assert!(!ed.in_dialog());
    assert_eq!(ed.doc().y, 2);
    // Ctrl+Tab: Set Tab (the field holds the current value 3).
    open_dialog(&mut ed, &mut m, 26);
    key(&mut m, raw::BACKSPACE, None);
    type_text(&mut ed, &mut m, "5\n\n");
    assert!(!ed.in_dialog());
    assert_eq!(ed.cfg.tabs, 5);
    // Cancel keeps the value (Escape is not a shortcut: the Cancel button
    // has the $C5 shortcut, i.e. Esc as a raw key).
    open_dialog(&mut ed, &mut m, 76);
    key(&mut m, raw::ESC, Some('\u{1b}'));
    frames(&mut ed, &mut m, 3);
    assert!(!ed.in_dialog());
    assert_eq!(ed.doc().y, 2);
}

#[test]
fn replace_all_dialogs() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "a=1\nb=a+a\n");
    open_dialog(&mut ed, &mut m, 99);
    // Search string, Return to the replace field, its text, Return.
    type_text(&mut ed, &mut m, "A\nZZ\n");
    // Click on "All Occurences" (its box is at 16,56 from the bottom left
    // of the dialog: display position measured on a rendering).
    m.input(InputEvent::MouseMove { x: 156.0, y: 299.0 });
    frames(&mut ed, &mut m, 1);
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: true });
    frames(&mut ed, &mut m, 2);
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: false });
    frames(&mut ed, &mut m, 2);
    type_text(&mut ed, &mut m, "\n");
    assert_eq!(ed.search_mode & 8, 8, "all occurrences ticked");
    // "Replace in whole text. Are you sure?" then "n change(s) done.".
    assert!(ed.in_dialog());
    frames(&mut ed, &mut m, 2);
    assert!(matches!(ed.modal, Some(dialogs::Modal::Dialog(Then::ReplaceAll))), "{:?}", ed.modal);
    type_text(&mut ed, &mut m, "\n");
    frames(&mut ed, &mut m, 2);
    assert!(ed.in_dialog());
    assert_eq!(listing(&ed), "ZZ=1\nB=ZZ+ZZ\n");
    // The message box goes away with a click.
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: true });
    frames(&mut ed, &mut m, 2);
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: false });
    frames(&mut ed, &mut m, 2);
    assert!(!ed.in_dialog());
}

#[test]
fn saved_new_and_quit_dialogs() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1");
    // Amiga+Q (New) on a changed program: "not saved. Save?" - No (N).
    with_shift(&mut m, raw::LAMIGA, 0x10, Some('q'));
    frames(&mut ed, &mut m, 2);
    assert!(ed.in_dialog());
    assert_eq!(listing(&ed), "Print 1\n");
    key(&mut m, 0x36, Some('n'));
    frames(&mut ed, &mut m, 3);
    assert!(!ed.in_dialog());
    assert_eq!(listing(&ed), "");
    // Quit: confirmation, Return = Yes.
    open_dialog(&mut ed, &mut m, 82);
    key(&mut m, raw::RETURN, Some('\r'));
    frames(&mut ed, &mut m, 3);
    assert!(ed.quit_requested);
}

#[test]
fn information_dialogs() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1 : print 2\n");
    for f in [83, 150, 149] {
        open_dialog(&mut ed, &mut m, f);
        // Ok / Cancel shortcut (Return, or Esc for About Extensions).
        let (k, c) = if f == 149 { (raw::ESC, '\u{1b}') } else { (raw::RETURN, '\r') };
        if f == 150 {
            // About closes on a click or after its time out.
            m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: true });
            frames(&mut ed, &mut m, 2);
            m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Left, pressed: false });
        } else {
            key(&mut m, k, Some(c));
        }
        frames(&mut ed, &mut m, 3);
        assert!(!ed.in_dialog(), "function {f}");
        assert!(m.hw.dialogs.channel_index(dialogs::ED_CHANNEL).is_none());
    }
    // The text is drawn again with the syntax colours.
    assert_eq!(m.hw.screens.get(EC_EDIT).unwrap().planes, 4);
}

/// Build Application from the Project menu: output folder with the file
/// selector, targets dialog, then the request for the platform.
#[test]
fn build_application_menu() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print \"app\"\n");
    ed.save_as(&mut m, "Ram Disk:prog").unwrap();
    // Right button on Project, release on its last item.
    let (x, y) = display_pos(&m, 20, 4);
    m.input(InputEvent::MouseMove { x, y });
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Right, pressed: true });
    frames(&mut ed, &mut m, 1);
    let n = ed.menu_root.children[0].children.len() as i32;
    let (x, y) = display_pos(&m, 40, 11 + (n - 1) * 9 + 4);
    m.input(InputEvent::MouseMove { x, y });
    frames(&mut ed, &mut m, 1);
    m.input(InputEvent::MouseButton { button: crate::input::MouseButton::Right, pressed: false });
    frames(&mut ed, &mut m, 25);
    // The file selector in Apps (created), the program's name as default.
    assert!(m.hw.files.is_dir("Ram Disk:Apps"));
    assert!(matches!(ed.modal, Some(dialogs::Modal::Fsel(Then::BuildFolder))), "{:?}", ed.modal);
    key(&mut m, raw::RETURN, Some('\r'));
    frames(&mut ed, &mut m, 30);
    assert!(matches!(ed.modal, Some(dialogs::Modal::Dialog(Then::BuildTargets(_)))), "{:?}", ed.modal);
    // F1 unticks Native, Return = Ok.
    key(&mut m, raw::F1, None);
    frames(&mut ed, &mut m, 3);
    key(&mut m, raw::RETURN, Some('\r'));
    frames(&mut ed, &mut m, 3);
    assert!(!ed.in_dialog());
    assert_eq!(m.hw.build_requests.len(), 1);
    let r = &m.hw.build_requests[0];
    assert_eq!(r.program, "Ram Disk:prog.AMOS");
    assert_eq!(r.name, "prog");
    assert_eq!(r.out, "Ram Disk:Apps/prog");
    assert!(m.hw.files.is_dir("Ram Disk:Apps/prog"));
    assert!(!r.native && r.web && r.with_files);
    assert_eq!(ed.current_alert(), Some("Building prog..."));
    // The platform's answer goes to the status line.
    m.hw.build_requests.clear();
    m.hw.build_results.push(Err("only available in the desktop version".into()));
    frames(&mut ed, &mut m, 1);
    assert_eq!(ed.current_alert(), Some("Build failed: only available in the desktop version"));
}

/// An unsaved, unnamed program is saved first (Save As file selector).
#[test]
fn build_application_saves_first() {
    let mut m = Machine::new();
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "print 1\n");
    // Amiga+Shift+A.
    m.input(InputEvent::Key { scancode: raw::LAMIGA, pressed: true, ch: None });
    with_shift(&mut m, raw::LSHIFT, 0x20, Some('A'));
    m.input(InputEvent::Key { scancode: raw::LAMIGA, pressed: false, ch: None });
    frames(&mut ed, &mut m, 25);
    assert!(
        matches!(ed.modal, Some(dialogs::Modal::Fsel(Then::SaveAs(Some(build::BUILD_FUNCTION))))),
        "{:?}",
        ed.modal
    );
    type_text(&mut ed, &mut m, "first.AMOS\n");
    frames(&mut ed, &mut m, 30);
    assert!(ed.doc().name.ends_with("first.AMOS"));
    // Then the output folder is asked.
    frames(&mut ed, &mut m, 25);
    assert!(matches!(ed.modal, Some(dialogs::Modal::Fsel(Then::BuildFolder))), "{:?}", ed.modal);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn help_key_runs_help_accessory() {
    let amos = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../AMOS-Professional-365/AMOS");
    let mut m = Machine::new();
    m.hw.files.set_native_root(&amos);
    let mut ed = Editor::new(&mut m);
    type_text(&mut ed, &mut m, "Print");
    ed.doc_mut().x = 2;
    ed.function(&mut m, 27);
    assert_eq!(ed.mode, Mode::Running);
    assert!(m.hw.command_line.starts_with(b"Print"));
    frames(&mut ed, &mut m, 150);
    assert_eq!(ed.mode, Mode::Running, "{:?}", m.hw.log);
    // The page of the instruction is in the work bank shown by the dialog.
    let page = m.hw.banks.peek_bytes(m.hw.banks.bank_or_address(10).unwrap(), 400);
    assert!(latin1_to_string(&page).contains("PRINT"), "{}", latin1_to_string(&page));
    // Esc closes the help: back to the program, unchanged.
    key(&mut m, raw::ESC, Some('\x1b'));
    frames(&mut ed, &mut m, 30);
    assert_eq!(ed.mode, Mode::Edit, "{:?}", m.hw.log);
    assert_eq!(ed.current_alert(), None);
    assert!(listing(&ed).contains("Print"));
}
