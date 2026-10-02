//! "Build Application..." (an addition of this port, in place of the
//! original's compiler accessory): bundles the current program into a
//! standalone native and / or web application.
//!
//! The program must be saved (the bundle reads it from disc). The output
//! folder is chosen with the file selector (default: `Apps/<name>` next to
//! the program, created if needed), then a small Interface dialog in the style of the
//! editor's own asks for the targets and whether the files of the
//! program's folder are included. The request goes to
//! `Hardware::build_requests`; the platform builds it and answers in
//! `Hardware::build_results`, shown in the status line.

use std::rc::Rc;

use super::dialogs::Then;
use super::*;
use crate::machine::BuildRequest;

/// Editor function number of Build Application (beyond the original
/// functions).
pub const BUILD_FUNCTION: u16 = 1102;

/// The targets dialog. Variables: 0 title, 1/3/5 the native, web and
/// files options (0/1), 2/4/6 their texts. The checkboxes and buttons are
/// those of the editor's Quit Options dialog (label 49 of the editor
/// program: images 15/16 of the puzzle, `KY $D0` for F1...).
const TARGETS_DIALOG: &str = "SI\t352,104;BA\tSWSX- 2/,SHSY- 2/ 16-;SA\t9;BO\t0,0,1,SX,SY;LI\t0,20,10,SX;\
PO\t0VACX,12,0VA,0,7;\
BU\t4,16,32,24,12,1VA,0,1;[UN0,0,BP15+; PR32,2,2VA,7;][SV1,BP;]KY\t$D0,0;\
BU\t5,16,48,24,12,3VA,0,1;[UN0,0,BP15+; PR32,2,4VA,7;][SV3,BP;]KY\t$D1,0;\
BU\t6,16,64,24,12,5VA,0,1;[UN0,0,BP15+; PR32,2,6VA,7;][SV5,BP;]KY\t$D2,0;\
BU\t1,SX80-,SY24-,64,16,0,0,1;[UN 0,0,BP 13+; PO 29ME CX,5,29ME,0,4;][BQ;]KY\t13,0;\
BU\t2,SX144-,SY24-,64,16,0,0,1;[UN 0,0,BP 13+; PO 19ME CX,5,19ME,0,4;][BQ;]KY\t$C5,0;\
RU\t0,3;EX;";

/// Directory part of an AMOS path ("Work:dir/" for "Work:dir/prog.AMOS").
fn directory_of(path: &str) -> &str {
    match path.rfind(['/', ':']) {
        Some(i) => &path[..=i],
        None => "",
    }
}

impl Editor {
    /// Build Application: first makes sure the program is saved.
    pub(super) fn build_application(&mut self, m: &mut Machine) {
        if let Err(e) = self.doc_mut().commit() {
            return self.edit_error(e);
        }
        if self.doc().name.is_empty() {
            if self.doc().is_empty() {
                return self.alert_message(206);
            }
            // Save As, then come back here.
            return self.file_selector(m, 74, Then::SaveAs(Some(BUILD_FUNCTION)));
        }
        // "Not saved. Save?" (No builds the version on disc).
        if self.doc().modified && self.saved_check(m, BUILD_FUNCTION) {
            return;
        }
        // The output folder: `Apps` next to the program, created if needed.
        let base = directory_of(&self.doc().name).to_string();
        let apps = format!("{base}Apps");
        if !m.hw.files.is_dir(&apps) {
            let _ = m.hw.files.make_dir(&apps);
        }
        let path = if m.hw.files.is_dir(&apps) { format!("{apps}/") } else { base };
        // The file selector returns a name (an empty one cancels): the
        // application goes in a folder of that name, by default the
        // program's name.
        let t1 = bytes("Build Application");
        let t2 = bytes("Name of the folder to build into");
        let stem = bytes(&self.app_name());
        self.file_selector_with(m, [&bytes(&path), &stem, &t1, &t2], Then::BuildFolder);
    }

    /// Name of the application: the program's file name without extension.
    fn app_name(&self) -> String {
        let program = &self.doc().name;
        let file = program.rsplit(['/', ':']).next().unwrap_or(program);
        let name = match file.rfind('.') {
            Some(i) if i > 0 => &file[..i],
            _ => file,
        };
        if name.is_empty() { "App".to_string() } else { name.to_string() }
    }

    /// The folder was chosen: the targets dialog.
    pub(super) fn build_folder_done(&mut self, m: &mut Machine, chosen: String) {
        // A typed name is a folder to create; a file selects its folder.
        let mut out = chosen.trim_end_matches('/').to_string();
        if m.hw.files.exists(&out) && !m.hw.files.is_dir(&out) {
            out = directory_of(&out).trim_end_matches('/').to_string();
        }
        if !out.is_empty() && !out.ends_with(':') && !m.hw.files.is_dir(&out) && m.hw.files.make_dir(&out).is_err() {
            return self.alert_message(184);
        }
        let [n, w, f] = self.build_options;
        let vars = [
            (0, DVal::str(b"Build Application")),
            (1, DVal::Int(n as i32)),
            (2, DVal::str(b"[F1] Native application")),
            (3, DVal::Int(w as i32)),
            (4, DVal::str(b"[F2] Web application")),
            (5, DVal::Int(f as i32)),
            (6, DVal::str(b"[F3] Include the folder's files")),
        ];
        let prog: Rc<[u8]> = Rc::from(TARGETS_DIALOG.as_bytes());
        self.dialog_program(m, prog, None, &vars, Then::BuildTargets(out));
    }

    /// The targets were chosen: queues the build.
    pub(super) fn build_targets_done(&mut self, m: &mut Machine, out: String, ret: i32, vars: &[DVal]) {
        if ret != 1 {
            return;
        }
        let flag = |i: usize| matches!(vars.get(i), Some(DVal::Int(v)) if *v != 0);
        let (native, web, with_files) = (flag(1), flag(3), flag(5));
        self.build_options = [native, web, with_files];
        if !native && !web {
            return self.alert_message(206);
        }
        let program = self.doc().name.clone();
        let name = self.app_name();
        self.build_waiting = m.hw.build_results.len();
        m.hw.build_requests.push(BuildRequest { program, with_files, name: name.clone(), out, native, web });
        self.building = true;
        self.alert(format!("Building {name}..."));
    }

    /// Shows the result of the build when the platform gives it.
    pub(super) fn build_poll(&mut self, m: &mut Machine) {
        if !self.building || m.hw.build_results.len() <= self.build_waiting {
            return;
        }
        self.building = false;
        match m.hw.build_results.last().cloned() {
            Some(Ok(msg)) => self.alert(msg),
            Some(Err(e)) => self.alert(format!("Build failed: {e}")),
            None => {}
        }
    }
}
