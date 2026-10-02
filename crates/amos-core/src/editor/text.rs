//! The program being edited: tokenised lines plus the editing state of
//! its window (cursor, current line buffer, block, undo).
//!
//! As in the original the program stays tokenised; only the line under the
//! cursor is edited as text. When the cursor leaves an edited line it is
//! tokenised and stored back, then listed again so its case, spacing and
//! indentation are normalised (`Ed_TokCur` / `Ed_TokStok2`,
//! `+Edit.s:10730`).
//!
//! The cursor line `y` ranges over `0..=lines.len()`: the line after the
//! last one is an empty line that becomes real when something is typed in
//! it. Procedures can be folded: only their `Procedure` line is shown and
//! it cannot be edited.

use crate::banks::Bank;
use crate::detok::detok_line;
use crate::program::{Program, proc_flags, read_u16};
use crate::tokenise::{TokeniseError, tokenise_line};
use crate::tokens::*;

/// Maximum length of an edited line (`Ed_PKey`, +Edit.s:1794).
pub const MAX_LINE_CHARS: usize = 250;

/// Problems reported in the status line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditError {
    /// "Line too long." (199)
    LineTooLong,
    /// "This line can't be modified" (183)
    NotEditable,
    /// "No more Undo." (4)
    NoUndo,
    /// "No mode Redo." (5)
    NoRedo,
    /// "What block?" (6)
    NoBlock,
    /// "Not a procedure." (203)
    NotProcedure,
    /// "Not found." (205)
    NotFound,
    /// "Top of text." (200) / "Bottom of text." (201)
    TopOfText,
    BottomOfText,
    /// "Mark not defined." (65)
    NoMark,
    /// A message was already shown.
    Reported,
}

impl EditError {
    /// Number of the editor message describing the error.
    pub fn message(self) -> usize {
        match self {
            EditError::LineTooLong => 199,
            EditError::NotEditable => 183,
            EditError::NoUndo => 4,
            EditError::NoRedo => 5,
            EditError::NoBlock => 6,
            EditError::NotProcedure => 203,
            EditError::NotFound => 205,
            EditError::TopOfText => 200,
            EditError::BottomOfText => 201,
            EditError::NoMark => 65,
            EditError::Reported => 0,
        }
    }
}

pub type EResult<T = ()> = Result<T, EditError>;

/// One change of the line list: `removed` lines at `at` were replaced by
/// `inserted`.
#[derive(Clone, Debug)]
struct Splice {
    at: usize,
    removed: Vec<Vec<u8>>,
    inserted: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
struct UndoGroup {
    splices: Vec<Splice>,
    /// Cursor before the change (y, x).
    cursor: (usize, usize),
}

/// Maximum number of undo steps kept (the original limits by memory,
/// `Ed_LUndo` / `Ed_NUndo`).
const MAX_UNDO: usize = 1000;

#[derive(Clone, Debug, Default)]
pub struct Doc {
    /// Tokenised lines (length byte, indent, tokens, 0 word).
    pub lines: Vec<Vec<u8>>,
    /// AMOS path of the program, empty for a new program.
    pub name: String,
    pub banks: Vec<Bank>,
    pub math_flags: u8,
    /// Changed since loaded or saved (`Prg_Change`).
    pub modified: bool,
    /// Changed since last tested (`Prg_StModif`).
    pub untested: bool,
    /// Cursor: line index (0..=lines.len()) and column.
    pub y: usize,
    pub x: usize,
    /// First visible row, first visible column.
    pub top: usize,
    pub left: usize,
    /// Text of the current line while it is being edited.
    edit: Option<Vec<u8>>,
    /// Block: line where the block was started (it extends to the cursor
    /// line). `block_fixed` keeps a block after Block Off.
    pub block_start: Option<usize>,
    pub block_end: Option<usize>,
    pub block_on: bool,
    /// Marks 0-9 (line indices).
    pub marks: [Option<usize>; 10],
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    group: Option<UndoGroup>,
}

fn is_proc_line(line: &[u8]) -> bool {
    line.len() >= 12 && read_u16(line, 2) == TK_PROCEDURE
}

fn is_end_proc_line(line: &[u8]) -> bool {
    line.len() >= 4 && read_u16(line, 2) == TK_END_PROC
}

/// True for a `Procedure` line that is folded.
pub fn is_folded(line: &[u8]) -> bool {
    is_proc_line(line) && line[10] & proc_flags::FOLDED != 0
}

fn tokenise(text: &[u8]) -> EResult<(Vec<u8>, bool)> {
    match tokenise_line(text) {
        Ok(Some(t)) => Ok((t.line, t.double_precision)),
        Ok(None) => Ok((vec![2, 0, 0, 0], false)),
        Err(TokeniseError::LineTooLong) => Err(EditError::LineTooLong),
    }
}

impl Doc {
    pub fn new() -> Doc {
        Doc::default()
    }

    /// A document holding a loaded program.
    pub fn from_program(prg: &Program, name: &str) -> Doc {
        let lines = prg.lines().map(|(_, l)| l.to_vec()).collect();
        Doc {
            lines,
            name: name.to_string(),
            banks: prg.banks.clone(),
            math_flags: prg.math_flags,
            untested: true,
            ..Doc::default()
        }
    }

    /// Builds the program (`Prg_Save`): Procedure lines get the distance to
    /// their End Proc line as the original stores it.
    pub fn to_program(&self) -> Program {
        let mut source = Vec::new();
        let mut proc_start: Option<usize> = None;
        for l in &self.lines {
            if is_proc_line(l) {
                proc_start = Some(source.len());
            } else if is_end_proc_line(l)
                && let Some(p) = proc_start.take()
            {
                let d = (source.len() - (p + 8)) as u32;
                source[p + 4..p + 8].copy_from_slice(&d.to_be_bytes());
            }
            source.extend_from_slice(l);
        }
        Program { source, banks: self.banks.clone(), math_flags: self.math_flags, ..Program::default() }
    }

    /// Number of lines (without the empty line after the end).
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    // ------------------------------------------------------------------
    // Rows (folding)
    // ------------------------------------------------------------------

    /// Line indices of the visible rows, including the empty line after
    /// the end of the text.
    pub fn rows(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.lines.len() + 1);
        let mut i = 0;
        while i < self.lines.len() {
            out.push(i);
            if is_folded(&self.lines[i]) {
                i += 1;
                while i < self.lines.len() {
                    let end = is_end_proc_line(&self.lines[i]);
                    i += 1;
                    if end {
                        break;
                    }
                }
                continue;
            }
            i += 1;
        }
        out.push(self.lines.len());
        out
    }

    /// Row of line `y` (the row of the folded procedure containing it).
    pub fn row_of(&self, rows: &[usize], y: usize) -> usize {
        match rows.binary_search(&y) {
            Ok(r) => r,
            Err(r) => r.saturating_sub(1),
        }
    }

    pub fn cursor_row(&self) -> usize {
        let rows = self.rows();
        self.row_of(&rows, self.y)
    }

    /// Number of visible rows (with the last empty one).
    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    // ------------------------------------------------------------------
    // Text access
    // ------------------------------------------------------------------

    /// Text of line `i` as shown (the edit buffer for the current line).
    pub fn line_text(&self, i: usize) -> Vec<u8> {
        if i == self.y
            && let Some(e) = &self.edit
        {
            return e.clone();
        }
        self.lines.get(i).map(|l| detok_line(l)).unwrap_or_default()
    }

    /// Text of the current line.
    pub fn current_text(&self) -> Vec<u8> {
        self.line_text(self.y)
    }

    pub fn is_edited(&self) -> bool {
        self.edit.is_some()
    }

    /// Lines that cannot be edited: folded procedures.
    pub fn is_editable(&self, i: usize) -> bool {
        self.lines.get(i).is_none_or(|l| !is_folded(l))
    }

    fn edit_buffer(&mut self) -> EResult<&mut Vec<u8>> {
        if !self.is_editable(self.y) {
            return Err(EditError::NotEditable);
        }
        if self.edit.is_none() {
            self.edit = Some(self.current_text());
        }
        Ok(self.edit.as_mut().unwrap())
    }

    // ------------------------------------------------------------------
    // Undo
    // ------------------------------------------------------------------

    fn begin(&mut self) {
        if self.group.is_none() {
            self.group = Some(UndoGroup { splices: Vec::new(), cursor: (self.y, self.x) });
        }
    }

    fn end(&mut self) {
        if let Some(g) = self.group.take()
            && !g.splices.is_empty()
        {
            self.undo.push(g);
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    /// Replaces `n` lines at `at` by `inserted`, recording the change.
    fn splice(&mut self, at: usize, n: usize, inserted: Vec<Vec<u8>>) {
        let at = at.min(self.lines.len());
        let n = n.min(self.lines.len() - at);
        let removed: Vec<Vec<u8>> = self.lines.splice(at..at + n, inserted.iter().cloned()).collect();
        let fresh = self.group.is_none();
        self.begin();
        self.group.as_mut().unwrap().splices.push(Splice { at, removed, inserted });
        if fresh {
            self.end();
        }
        self.modified = true;
        self.untested = true;
        self.fix_marks_after(at, n);
    }

    fn fix_marks_after(&mut self, at: usize, removed: usize) {
        let len = self.lines.len();
        for m in self.marks.iter_mut().flatten() {
            if *m > at + removed {
                *m = (*m).min(len);
            }
            *m = (*m).min(len);
        }
        if let Some(b) = &mut self.block_start {
            *b = (*b).min(len);
        }
        if let Some(b) = &mut self.block_end {
            *b = (*b).min(len);
        }
    }

    fn apply_reverse(&mut self, g: &UndoGroup) {
        for s in g.splices.iter().rev() {
            self.lines.splice(s.at..s.at + s.inserted.len(), s.removed.iter().cloned());
        }
    }

    fn apply_forward(&mut self, g: &UndoGroup) {
        for s in g.splices.iter() {
            self.lines.splice(s.at..s.at + s.removed.len(), s.inserted.iter().cloned());
        }
    }

    /// `Ed_Undo` (Ctrl+U): first forgets the changes of the line being
    /// edited, then undoes stored changes one at a time.
    pub fn undo(&mut self) -> EResult {
        if self.edit.take().is_some() {
            return Ok(());
        }
        let g = self.undo.pop().ok_or(EditError::NoUndo)?;
        self.apply_reverse(&g);
        let (y, x) = g.cursor;
        self.redo.push(g);
        self.y = y.min(self.lines.len());
        self.x = x;
        self.modified = true;
        self.untested = true;
        Ok(())
    }

    /// `Ed_Redo` (Ctrl+Shift+U).
    pub fn redo(&mut self) -> EResult {
        self.commit()?;
        let g = self.redo.pop().ok_or(EditError::NoRedo)?;
        self.apply_forward(&g);
        self.y = g.cursor.0.min(self.lines.len());
        self.undo.push(g);
        self.modified = true;
        self.untested = true;
        Ok(())
    }

    pub fn clear_undo(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    // ------------------------------------------------------------------
    // Storing the current line
    // ------------------------------------------------------------------

    /// `Ed_TokCur`: tokenises the edited line and stores it. The line is
    /// then shown listed again (normalised).
    pub fn commit(&mut self) -> EResult {
        let Some(text) = self.edit.clone() else { return Ok(()) };
        let (line, dp) = tokenise(&text)?;
        self.edit = None;
        if dp {
            self.math_flags |= 0x83;
        }
        if self.y >= self.lines.len() {
            self.splice(self.lines.len(), 0, vec![line]);
        } else if self.lines[self.y] != line {
            self.splice(self.y, 1, vec![line]);
        }
        Ok(())
    }

    /// Replaces the text of the current line (used by search / replace and
    /// the remote control `EdZ_NewLine`).
    pub fn set_current_text(&mut self, text: Vec<u8>) -> EResult {
        *self.edit_buffer()? = text;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Typing
    // ------------------------------------------------------------------

    /// `Ed_PKey`: inserts or overwrites a character at the cursor. A cursor
    /// beyond the end of the line first moves to the end.
    pub fn type_char(&mut self, c: u8, insert: bool) -> EResult {
        let x = self.x;
        let buf = self.edit_buffer()?;
        let x = x.min(buf.len());
        if insert || x == buf.len() {
            if buf.len() >= MAX_LINE_CHARS {
                return Err(EditError::LineTooLong);
            }
            buf.insert(x, c);
        } else {
            buf[x] = c;
        }
        self.x = x + 1;
        Ok(())
    }

    /// Inserts a string at the cursor.
    pub fn type_str(&mut self, s: &[u8], insert: bool) -> EResult {
        for &c in s {
            self.type_char(c, insert)?;
        }
        Ok(())
    }

    /// `Ed_Delete`: deletes the character under the cursor.
    pub fn delete(&mut self) -> EResult {
        let x = self.x;
        let buf = self.edit_buffer()?;
        if x < buf.len() {
            buf.remove(x);
        }
        Ok(())
    }

    /// `Ed_Back`: deletes the character left of the cursor, or joins the
    /// line with the previous one at column 0 (`Ed_Join`).
    pub fn backspace(&mut self) -> EResult {
        if self.x == 0 {
            return self.join_previous();
        }
        let len = self.current_text().len();
        self.x -= 1;
        if self.x < len {
            self.delete()?;
        }
        Ok(())
    }

    /// `Ed_Join`.
    fn join_previous(&mut self) -> EResult {
        if self.y == 0 {
            return Ok(());
        }
        let rows = self.rows();
        let r = self.row_of(&rows, self.y);
        if r == 0 {
            return Ok(());
        }
        let prev = rows[r - 1];
        let cur = self.current_text();
        if !cur.is_empty() && (!self.is_editable(prev) || !self.is_editable(self.y)) {
            return Err(EditError::NotEditable);
        }
        self.commit()?;
        let prev_text = self.line_text(prev);
        let mut joined = prev_text.clone();
        joined.extend_from_slice(&cur);
        if joined.len() >= MAX_LINE_CHARS {
            return Err(EditError::LineTooLong);
        }
        let (line, _) = tokenise(&joined)?;
        self.begin();
        if self.y < self.lines.len() {
            self.splice(self.y, 1, vec![]);
        }
        self.splice(prev, 1, vec![line]);
        self.end();
        self.y = prev;
        self.x = prev_text.len();
        Ok(())
    }

    /// `Ed_Return`: at column 0 inserts an empty line above; otherwise
    /// splits the line at the cursor and moves to the start of the new
    /// line.
    pub fn split_line(&mut self) -> EResult {
        if self.x == 0 || !self.is_editable(self.y) {
            self.commit()?;
            if self.x == 0 && self.is_editable(self.y) {
                self.insert_line()?;
            }
            self.move_down();
            self.x = 0;
            return Ok(());
        }
        let text = self.current_text();
        let x = self.x.min(text.len());
        let (left, right) = (text[..x].to_vec(), text[x..].to_vec());
        let (l1, dp1) = tokenise(&left)?;
        let (l2, dp2) = tokenise(&right)?;
        if dp1 || dp2 {
            self.math_flags |= 0x83;
        }
        self.edit = None;
        self.begin();
        if self.y >= self.lines.len() {
            self.splice(self.y, 0, vec![l1]);
            if !right.is_empty() {
                self.splice(self.y + 1, 0, vec![l2]);
            }
        } else {
            self.splice(self.y, 1, vec![l1, l2]);
        }
        self.end();
        self.y += 1;
        self.x = 0;
        Ok(())
    }

    /// `Ed_InsLine` (F10): inserts an empty line at the cursor.
    pub fn insert_line(&mut self) -> EResult {
        self.commit()?;
        if self.y < self.lines.len() {
            self.splice(self.y, 0, vec![vec![2, 0, 0, 0]]);
        }
        Ok(())
    }

    /// `Ed_DelLiCu` (Ctrl+Y): deletes the current line (a folded procedure
    /// is deleted completely).
    pub fn delete_line(&mut self) -> EResult {
        self.edit = None;
        if self.y >= self.lines.len() {
            return Ok(());
        }
        let n = self.folded_span(self.y);
        self.splice(self.y, n, vec![]);
        Ok(())
    }

    /// Number of lines of row starting at line `y` (1, or the whole folded
    /// procedure).
    fn folded_span(&self, y: usize) -> usize {
        if !self.lines.get(y).is_some_and(|l| is_folded(l)) {
            return 1;
        }
        let mut i = y + 1;
        while i < self.lines.len() {
            let end = is_end_proc_line(&self.lines[i]);
            i += 1;
            if end {
                break;
            }
        }
        i - y
    }

    /// `Ed_EffLigne` (Ctrl+Q): clears the text of the line.
    pub fn clear_line(&mut self) -> EResult {
        self.edit_buffer()?.clear();
        self.x = 0;
        Ok(())
    }

    /// `Ed_DelFin` (Ctrl+Del).
    pub fn delete_to_end(&mut self) -> EResult {
        let x = self.x;
        let b = self.edit_buffer()?;
        b.truncate(x.min(b.len()));
        Ok(())
    }

    /// `Ed_DelDebut` (Ctrl+Backspace).
    pub fn delete_to_start(&mut self) -> EResult {
        let x = self.x;
        let b = self.edit_buffer()?;
        let x = x.min(b.len());
        b.drain(..x);
        self.x = 0;
        Ok(())
    }

    /// `Ed_DelMot` (Shift+Del): deletes up to the start of the next word.
    pub fn delete_word_right(&mut self) -> EResult {
        let text = self.current_text();
        let end = word_right(&text, self.x);
        let x = self.x;
        let b = self.edit_buffer()?;
        if x < b.len() {
            b.drain(x..end.min(b.len()));
        }
        Ok(())
    }

    /// `Ed_BackMot` (Shift+Backspace).
    pub fn delete_word_left(&mut self) -> EResult {
        let text = self.current_text();
        let x = self.x.min(text.len());
        let start = word_left(&text, x);
        let b = self.edit_buffer()?;
        b.drain(start..x);
        self.x = start;
        Ok(())
    }

    /// `Ed_Tab` (Tab): moves the whole line right by the tab value.
    pub fn tab(&mut self, tabs: usize) -> EResult {
        let b = self.edit_buffer()?;
        if b.len() + tabs >= MAX_LINE_CHARS {
            return Err(EditError::LineTooLong);
        }
        for _ in 0..tabs {
            b.insert(0, b' ');
        }
        self.x += tabs;
        Ok(())
    }

    /// `Ed_ShTab` (Shift+Tab): removes up to `tabs` leading spaces.
    pub fn untab(&mut self, tabs: usize) -> EResult {
        let b = self.edit_buffer()?;
        let n = b.iter().take(tabs).take_while(|&&c| c == b' ').count();
        b.drain(..n);
        self.x = self.x.saturating_sub(n);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Cursor movement (the line is stored before moving vertically)
    // ------------------------------------------------------------------

    /// Moves to row `r` (clamped), keeping the column.
    pub fn goto_row(&mut self, r: usize) -> EResult {
        self.commit()?;
        let rows = self.rows();
        self.y = rows[r.min(rows.len() - 1)];
        Ok(())
    }

    /// Moves to line `y` (clamped); unfolds nothing: a hidden line moves
    /// the cursor to its procedure.
    pub fn goto_line(&mut self, y: usize) -> EResult {
        self.commit()?;
        let rows = self.rows();
        self.y = rows[self.row_of(&rows, y.min(self.lines.len()))];
        Ok(())
    }

    pub fn move_up(&mut self) -> bool {
        let r = self.cursor_row();
        if r == 0 || self.goto_row(r - 1).is_err() {
            return false;
        }
        true
    }

    pub fn move_down(&mut self) -> bool {
        let r = self.cursor_row();
        if r + 1 >= self.row_count() || self.goto_row(r + 1).is_err() {
            return false;
        }
        true
    }

    pub fn move_left(&mut self) {
        self.x = self.x.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        if self.x < MAX_LINE_CHARS {
            self.x += 1;
        }
    }

    pub fn word_left(&mut self) {
        let t = self.current_text();
        self.x = word_left(&t, self.x.min(t.len()));
    }

    pub fn word_right(&mut self) {
        let t = self.current_text();
        self.x = word_right(&t, self.x);
    }

    pub fn line_start(&mut self) {
        self.x = 0;
    }

    pub fn line_end(&mut self) {
        self.x = self.current_text().len();
    }

    pub fn text_top(&mut self) -> EResult {
        self.goto_row(0)?;
        self.x = 0;
        Ok(())
    }

    pub fn text_bottom(&mut self) -> EResult {
        self.goto_row(usize::MAX)?;
        self.x = 0;
        Ok(())
    }

    /// Previous / next line starting with a label or a Procedure
    /// (`Ed_PLabel`, `Ed_NLabel`).
    pub fn goto_label(&mut self, forward: bool) -> EResult {
        self.commit()?;
        let rows = self.rows();
        let r = self.row_of(&rows, self.y);
        let is_label = |i: usize| {
            self.lines.get(i).is_some_and(|l| l.len() >= 4 && matches!(read_u16(l, 2), TK_LAB | TK_PROCEDURE))
        };
        let found = if forward {
            rows[r + 1..].iter().copied().find(|&i| is_label(i))
        } else {
            rows[..r].iter().rev().copied().find(|&i| is_label(i))
        };
        match found {
            Some(i) => {
                self.y = i;
                self.x = 0;
                Ok(())
            }
            None if forward => Err(EditError::BottomOfText),
            None => Err(EditError::TopOfText),
        }
    }

    // ------------------------------------------------------------------
    // Blocks (whole lines)
    // ------------------------------------------------------------------

    /// `Ed_BlocOn` (Ctrl+B): starts a block at the cursor line, or fixes
    /// its end.
    pub fn block_toggle(&mut self) -> EResult {
        self.commit()?;
        if self.block_on {
            self.block_on = false;
            self.block_end = Some(self.y);
        } else {
            self.block_on = true;
            self.block_start = Some(self.y);
            self.block_end = None;
        }
        Ok(())
    }

    /// `Ed_BlocAll` (Ctrl+A).
    pub fn block_all(&mut self) -> EResult {
        self.commit()?;
        if self.lines.is_empty() {
            return Err(EditError::NoBlock);
        }
        self.block_on = false;
        self.block_start = Some(0);
        self.block_end = Some(self.lines.len() - 1);
        Ok(())
    }

    /// `Ed_BlocForget` (Ctrl+F).
    pub fn block_forget(&mut self) {
        self.block_on = false;
        self.block_start = None;
        self.block_end = None;
    }

    /// Lines of the block (inclusive range, folded procedures included).
    pub fn block_range(&self) -> Option<(usize, usize)> {
        let s = self.block_start?;
        let e = if self.block_on { self.y } else { self.block_end? };
        let (a, b) = if s <= e { (s, e) } else { (e, s) };
        let b = (b + self.folded_span(b)).min(self.lines.len());
        if a >= b {
            return None;
        }
        Some((a, b - 1))
    }

    /// `Ed_BlocStore` (Ctrl+S): copies the block lines.
    pub fn block_copy(&mut self) -> EResult<Vec<Vec<u8>>> {
        self.commit()?;
        let (a, b) = self.block_range().ok_or(EditError::NoBlock)?;
        Ok(self.lines[a..=b].to_vec())
    }

    /// `Ed_BlocCut` (Ctrl+C): removes the block and returns its lines.
    pub fn block_cut(&mut self) -> EResult<Vec<Vec<u8>>> {
        self.commit()?;
        let (a, b) = self.block_range().ok_or(EditError::NoBlock)?;
        let lines = self.lines[a..=b].to_vec();
        self.splice(a, b - a + 1, vec![]);
        self.block_forget();
        self.y = a.min(self.lines.len());
        Ok(lines)
    }

    /// `Ed_BlocPaste` (Ctrl+P): inserts lines above the cursor line.
    pub fn paste(&mut self, lines: &[Vec<u8>]) -> EResult {
        self.commit()?;
        if lines.is_empty() {
            return Err(EditError::NoBlock);
        }
        let at = self.y.min(self.lines.len());
        self.splice(at, 0, lines.to_vec());
        Ok(())
    }

    /// Inserts text lines (an ASCII listing) above the cursor line.
    pub fn paste_text(&mut self, text: &[u8]) -> EResult {
        let mut lines = Vec::new();
        for raw in text.split(|&c| c == b'\n') {
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            let l: Vec<u8> = raw.iter().map(|&c| if c == b'\t' { b' ' } else { c }).collect();
            lines.push(tokenise(&l)?.0);
        }
        if text.ends_with(b"\n") {
            lines.pop();
        }
        self.paste(&lines)
    }

    // ------------------------------------------------------------------
    // Procedures
    // ------------------------------------------------------------------

    /// The Procedure line enclosing line `y` (or `y` itself).
    pub fn procedure_of(&self, y: usize) -> Option<usize> {
        let mut i = y.min(self.lines.len().saturating_sub(1));
        if self.lines.is_empty() {
            return None;
        }
        loop {
            let l = &self.lines[i];
            if is_proc_line(l) {
                return Some(i);
            }
            if is_end_proc_line(l) && i != y {
                return None;
            }
            if i == 0 {
                return None;
            }
            i -= 1;
        }
    }

    fn set_folded(&mut self, i: usize, fold: bool) {
        let mut l = self.lines[i].clone();
        if fold {
            l[10] |= proc_flags::FOLDED;
        } else {
            l[10] &= !proc_flags::FOLDED;
        }
        if l != self.lines[i] {
            self.splice(i, 1, vec![l]);
        }
    }

    /// `Ed_ProcOpen` (F9): folds or unfolds the procedure at the cursor.
    pub fn toggle_fold(&mut self) -> EResult {
        self.commit()?;
        let p = self.procedure_of(self.y).ok_or(EditError::NotProcedure)?;
        let fold = !is_folded(&self.lines[p]);
        self.set_folded(p, fold);
        self.y = p;
        self.x = 0;
        Ok(())
    }

    /// `Ed_ProcsOpen` / `Ed_ProcsClose`: unfolds or folds all procedures.
    pub fn fold_all(&mut self, fold: bool) -> EResult {
        self.commit()?;
        self.begin();
        for i in 0..self.lines.len() {
            if is_proc_line(&self.lines[i]) {
                self.set_folded(i, fold);
            }
        }
        self.end();
        if fold && let Some(p) = self.procedure_of(self.y) {
            self.y = p;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Indentation (F3)
    // ------------------------------------------------------------------

    /// `Indent` (+Edit.s:8472): recomputes the indentation of every line
    /// from the structure tokens.
    pub fn indent(&mut self, tab: usize) -> EResult {
        self.commit()?;
        let tab = tab as i32;
        let mut new_lines = self.lines.clone();
        let mut d6: i32 = 0; // indent of the next line
        let mut i = 0;
        while i < new_lines.len() {
            let line = &new_lines[i];
            let mut d5 = d6; // indent of this line
            let mut d7 = 0; // loops opened on this line
            let mut d4 = false; // Then seen
            let mut p = 2;
            let mut skip_to = None;
            while p + 2 <= line.len() {
                let t = read_u16(line, p);
                if t == TK_EOL {
                    break;
                }
                match t {
                    TK_PROCEDURE => {
                        d5 = 0;
                        d6 = tab;
                        if line[10] & proc_flags::FOLDED != 0 {
                            d6 = 0;
                            skip_to = Some(i + self.folded_span(i));
                            break;
                        }
                    }
                    TK_END_PROC => {
                        d5 = 0;
                        d6 = 0;
                    }
                    TK_FOR | TK_REPEAT | TK_WHILE | TK_DO | TK_IF => {
                        d7 += 1;
                        d6 += tab;
                    }
                    _ if t == tk::THEN => {
                        d4 = true;
                        minus(&mut d7, &mut d5, &mut d6, tab);
                    }
                    TK_ELSE | TK_ELSE_IF => {
                        if !d4 {
                            d5 -= tab;
                        }
                    }
                    _ if t == tk::END_IF || t == tk::NEXT || t == tk::UNTIL || t == tk::WEND || t == tk::LOOP => {
                        minus(&mut d7, &mut d5, &mut d6, tab);
                    }
                    _ => {}
                }
                p += crate::interp::verify::token_size(line, p);
            }
            let v = (d5.clamp(0, 126) + 1) as u8;
            // Empty lines keep their indent 0.
            let empty = line.len() <= 4 && line[1] == 0;
            if !empty {
                new_lines[i][1] = v;
            }
            i = skip_to.unwrap_or(i + 1);
        }
        if new_lines != self.lines {
            self.splice(0, self.lines.len(), new_lines);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Search / replace
    // ------------------------------------------------------------------

    /// Finds `what` from the cursor (excluded) forwards or backwards in the
    /// listed text and moves the cursor to it.
    pub fn search(&mut self, what: &[u8], forward: bool, ignore_case: bool) -> EResult {
        if what.is_empty() {
            return Err(EditError::NotFound);
        }
        self.commit()?;
        let norm = |s: &[u8]| -> Vec<u8> { if ignore_case { s.to_ascii_lowercase() } else { s.to_vec() } };
        let pat = norm(what);
        let rows = self.rows();
        let r0 = self.row_of(&rows, self.y);
        let n = rows.len();
        let find_in = |text: &[u8], from: Option<usize>, to: Option<usize>| -> Option<usize> {
            let t = norm(text);
            let mut hits = (0..t.len().saturating_sub(pat.len() - 1)).filter(|&i| t[i..].starts_with(&pat));
            if forward {
                hits.find(|&i| from.is_none_or(|f| i >= f))
            } else {
                hits.rfind(|&i| to.is_none_or(|e| i < e))
            }
        };
        // Current row first, after (or before) the cursor.
        let cur = self.line_text(rows[r0]);
        let hit = if forward { find_in(&cur, Some(self.x + 1), None) } else { find_in(&cur, None, Some(self.x)) };
        if let Some(x) = hit {
            self.x = x;
            return Ok(());
        }
        for k in 1..n {
            let r = if forward {
                if r0 + k >= n {
                    break;
                }
                r0 + k
            } else {
                if k > r0 {
                    break;
                }
                r0 - k
            };
            let text = self.line_text(rows[r]);
            if let Some(x) = find_in(&text, None, None) {
                self.y = rows[r];
                self.x = x;
                return Ok(());
            }
        }
        Err(EditError::NotFound)
    }

    /// Replaces `what` at the cursor (if it is there) by `with`. Returns
    /// true if a replacement was done.
    pub fn replace_here(&mut self, what: &[u8], with: &[u8], ignore_case: bool) -> EResult<bool> {
        let text = self.current_text();
        let x = self.x;
        let matches = text.len() >= x + what.len()
            && if ignore_case {
                text[x..x + what.len()].eq_ignore_ascii_case(what)
            } else {
                &text[x..x + what.len()] == what
            };
        if !matches || what.is_empty() {
            return Ok(false);
        }
        let mut t = text;
        t.splice(x..x + what.len(), with.iter().copied());
        if t.len() > MAX_LINE_CHARS {
            return Err(EditError::LineTooLong);
        }
        self.set_current_text(t)?;
        self.x = x + with.len();
        self.commit()?;
        Ok(true)
    }

    // ------------------------------------------------------------------
    // Marks
    // ------------------------------------------------------------------

    pub fn set_mark(&mut self, n: usize) {
        self.marks[n % 10] = Some(self.y);
    }

    pub fn goto_mark(&mut self, n: usize) -> EResult {
        let y = self.marks[n % 10].ok_or(EditError::NoMark)?;
        self.goto_line(y)
    }

    // ------------------------------------------------------------------
    // Errors
    // ------------------------------------------------------------------

    /// Line index and column of a byte offset in the (verified) program
    /// code, which has the same layout as the source. Folded procedures
    /// containing the line are unfolded so it can be shown.
    pub fn locate_offset(&mut self, pos: usize) -> Option<(usize, usize)> {
        let mut start = 0;
        for (i, l) in self.lines.iter().enumerate() {
            if pos < start + l.len() {
                let k = pos - start;
                // Column: length of the listing of the tokens before pos.
                let col = if k <= 2 { line_indent(l) } else { detok_line(&l[..k]).len() };
                let col = col.min(detok_line(l).len());
                return Some((i, col));
            }
            start += l.len();
        }
        None
    }

    /// Makes line `y` visible by unfolding its procedure, then puts the
    /// cursor there.
    pub fn reveal(&mut self, y: usize, x: usize) {
        self.edit = None;
        if let Some(p) = self.procedure_of(y)
            && p != y
            && is_folded(&self.lines[p])
        {
            self.set_folded(p, false);
        }
        self.y = y.min(self.lines.len());
        self.x = x;
    }
}

fn minus(d7: &mut i32, d5: &mut i32, d6: &mut i32, tab: i32) {
    *d7 -= 1;
    if *d7 < 0 {
        *d7 = 0;
        *d5 -= tab;
    }
    *d6 -= tab;
}

fn line_indent(l: &[u8]) -> usize {
    l.get(1).map_or(0, |&i| (i as usize).saturating_sub(1))
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'#' || c >= 0xC0
}

/// Start of the word left of `x` (`R_MotGauche`).
pub fn word_left(t: &[u8], x: usize) -> usize {
    let mut i = x.min(t.len());
    while i > 0 && !is_word(t[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word(t[i - 1]) {
        i -= 1;
    }
    i
}

/// Start of the next word right of `x` (`R_MotDroi`).
pub fn word_right(t: &[u8], x: usize) -> usize {
    let mut i = x;
    if i >= t.len() {
        return i;
    }
    while i < t.len() && is_word(t[i]) {
        i += 1;
    }
    while i < t.len() && !is_word(t[i]) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detok::latin1_to_string;

    fn doc(src: &str) -> Doc {
        let prg = crate::tokenise::tokenise_program(src.as_bytes()).unwrap();
        Doc::from_program(&prg, "")
    }

    fn listing(d: &Doc) -> String {
        latin1_to_string(&crate::detok::list_program(&d.to_program()))
    }

    #[test]
    fn typing_and_commit_normalises() {
        let mut d = Doc::new();
        d.type_str(b"print \"hello\"", true).unwrap();
        assert_eq!(d.current_text(), b"print \"hello\"");
        assert!(d.lines.is_empty());
        d.commit().unwrap();
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.current_text(), b"Print \"hello\"");
        // Moving down from the last line goes to the empty line after it.
        assert!(d.move_down());
        assert_eq!(d.y, 1);
        d.type_str(b"for i=1 to 10", true).unwrap();
        assert!(d.move_up());
        assert_eq!(listing(&d), "Print \"hello\"\nFor I=1 To 10\n");
        assert_eq!(d.y, 0);
    }

    #[test]
    fn overwrite_insert_delete_backspace() {
        let mut d = doc("abc=1");
        d.x = 0;
        d.type_char(b'x', false).unwrap();
        assert_eq!(d.current_text(), b"xBC=1");
        d.type_char(b'y', true).unwrap();
        assert_eq!(d.current_text(), b"xyBC=1");
        d.backspace().unwrap();
        assert_eq!(d.current_text(), b"xBC=1");
        d.delete().unwrap();
        assert_eq!(d.current_text(), b"xC=1");
        // Typing beyond the end of the line goes to its end.
        d.x = 20;
        d.type_char(b'2', true).unwrap();
        assert_eq!(d.current_text(), b"xC=12");
        assert_eq!(d.x, 5);
        d.undo().unwrap();
        assert_eq!(d.current_text(), b"ABC=1");
    }

    #[test]
    fn split_and_join() {
        let mut d = doc("a=1 : b=2");
        d.x = 3;
        d.split_line().unwrap();
        assert_eq!(listing(&d), "A=1\n : B=2\n");
        assert_eq!((d.y, d.x), (1, 0));
        d.backspace().unwrap();
        assert_eq!(listing(&d), "A=1 : B=2\n");
        assert_eq!((d.y, d.x), (0, 3));
        // Return at column 0 inserts an empty line above.
        d.x = 0;
        d.split_line().unwrap();
        assert_eq!(listing(&d), "\nA=1 : B=2\n");
        assert_eq!(d.y, 1);
        // Undo the insertion, then the join, then the split.
        d.undo().unwrap();
        assert_eq!(listing(&d), "A=1 : B=2\n");
        d.undo().unwrap();
        assert_eq!(listing(&d), "A=1\n : B=2\n");
        d.redo().unwrap();
        assert_eq!(listing(&d), "A=1 : B=2\n");
    }

    #[test]
    fn line_operations() {
        let mut d = doc("a=1\nb=2\nc=3");
        d.y = 1;
        d.delete_line().unwrap();
        assert_eq!(listing(&d), "A=1\nC=3\n");
        d.insert_line().unwrap();
        assert_eq!(listing(&d), "A=1\n\nC=3\n");
        d.y = 2;
        d.x = 1;
        d.delete_to_end().unwrap();
        d.commit().unwrap();
        assert_eq!(listing(&d), "A=1\n\nC\n");
        d.y = 0;
        d.tab(3).unwrap();
        d.commit().unwrap();
        assert_eq!(listing(&d), "   A=1\n\nC\n");
        d.untab(3).unwrap();
        d.commit().unwrap();
        assert_eq!(listing(&d), "A=1\n\nC\n");
    }

    #[test]
    fn blocks() {
        let mut d = doc("a=1\nb=2\nc=3\nd=4");
        d.y = 1;
        d.block_toggle().unwrap();
        d.move_down();
        d.block_toggle().unwrap();
        assert_eq!(d.block_range(), Some((1, 2)));
        let cut = d.block_cut().unwrap();
        assert_eq!(listing(&d), "A=1\nD=4\n");
        d.y = 0;
        d.paste(&cut).unwrap();
        assert_eq!(listing(&d), "B=2\nC=3\nA=1\nD=4\n");
        d.block_all().unwrap();
        assert_eq!(d.block_copy().unwrap().len(), 4);
    }

    #[test]
    fn folding() {
        let mut d = doc("Print 1\nProcedure HELLO\n  Print 2\nEnd Proc\nPrint 3");
        assert_eq!(d.row_count(), 6);
        d.y = 2;
        d.toggle_fold().unwrap();
        assert_eq!(d.y, 1);
        assert_eq!(d.rows(), vec![0, 1, 4, 5]);
        assert!(!d.is_editable(1));
        assert_eq!(d.type_char(b'x', true), Err(EditError::NotEditable));
        assert!(d.move_down());
        assert_eq!(d.y, 4);
        // The program still runs and saves with the procedure distance.
        let prg = d.to_program();
        let p = prg.lines().nth(1).unwrap().1;
        assert_eq!(
            u32::from_be_bytes([p[4], p[5], p[6], p[7]]) as usize,
            p.len() + prg.lines().nth(2).unwrap().1.len() - 8
        );
        d.fold_all(false).unwrap();
        assert_eq!(d.row_count(), 6);
    }

    #[test]
    fn indent() {
        let mut d =
            doc("For I=1 To 3\nIf I=2\nPrint I\nElse\nPrint 0\nEnd If\nNext I\nProcedure A\nDo\nLoop\nEnd Proc");
        d.indent(3).unwrap();
        assert_eq!(
            listing(&d),
            "For I=1 To 3\n   If I=2\n      Print I\n   Else \n      Print 0\n   End If \nNext I\nProcedure A\n   Do \n   Loop \nEnd Proc\n"
        );
    }

    #[test]
    fn search_and_replace() {
        let mut d = doc("Print \"one\"\nPrint \"two\"\nA=1");
        d.search(b"print", true, true).unwrap();
        assert_eq!((d.y, d.x), (1, 0));
        assert_eq!(d.search(b"zzz", true, true), Err(EditError::NotFound));
        d.search(b"one", false, true).unwrap();
        assert_eq!((d.y, d.x), (0, 7));
        assert!(d.replace_here(b"one", b"1", true).unwrap());
        assert_eq!(listing(&d), "Print \"1\"\nPrint \"two\"\nA=1\n");
    }

    #[test]
    fn error_location() {
        let mut d = doc("A=1\nPrint B+C");
        let first = d.lines[0].len();
        // Offset of the "+" token in the second line.
        let (y, x) = d.locate_offset(first + 2 + 2 + 8).unwrap();
        assert_eq!(y, 1);
        assert_eq!(&d.line_text(1)[..x], b"Print B");
    }
}
