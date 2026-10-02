//! Disc access and files (`+Lib.s` Open/Close/Dir..., `+ILib.s` Print #).

use super::Hardware;
use crate::errors;
use crate::files::{Channel, ChannelMode, FsError};
use crate::interp::value::{Value, astr, empty_str};
use crate::interp::{Exc, Host, Interp, R, StopReason, VarLoc, err};
use crate::tokens::{Keyword, TK_COMMA, TK_HASH, TK_SEMI, TokenKind, tk};

fn fs_err(e: FsError) -> Exc {
    Exc::Error(e.amos_error())
}

/// Field definitions of a random access channel: (length, variable).
pub type Fields = Vec<(usize, VarLoc, u8)>;

impl Hardware {
    pub(crate) fn files_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        if kw.slot != 0 {
            return Ok(false);
        }
        use tk::*;
        let reserved = kw.def().is_some_and(|d| d.kind() == TokenKind::ReservedVariable);
        match kw.token {
            OPEN_IN | OPEN_OUT | APPEND | OPEN_RANDOM => {
                let a = it.inst_args(self, kw)?;
                let n = channel_number(a.int(0))?;
                if self.files.channels.contains_key(&n) {
                    return err(96);
                }
                let path = crate::detok::latin1_to_string(&a.str(1));
                let (mode, data) = match kw.token {
                    OPEN_IN => (ChannelMode::In, self.files.read(&path).map_err(fs_err)?),
                    OPEN_OUT => (ChannelMode::Out, Vec::new()),
                    APPEND => (ChannelMode::Append, self.files.read(&path).unwrap_or_default()),
                    _ => (ChannelMode::Random, self.files.read(&path).unwrap_or_default()),
                };
                let pos = if mode == ChannelMode::Append { data.len() } else { 0 };
                let dirty = mode != ChannelMode::In;
                self.files.channels.insert(n, Channel { path, mode, data, pos, dirty, record_len: 0 });
                if dirty {
                    self.flush_channel(n)?;
                }
            }
            CLOSE => {
                let all: Vec<u32> = self.files.channels.keys().copied().collect();
                for n in all {
                    self.close_channel(n)?;
                }
            }
            CLOSE_2 => {
                let n = it.inst_args(self, kw)?.int(0);
                let n = channel_number(n)?;
                self.close_channel(n)?;
            }
            PRINT_HASH => {
                let n = it.eval_int(self)?;
                if it.peek() == TK_COMMA || it.peek() == TK_SEMI {
                    it.pc += 2;
                }
                let text = it.print_text(self)?;
                self.channel_write(n, &text)?;
            }
            INPUT_HASH | LINE_INPUT_HASH => {
                let n = it.eval_int(self)?;
                it.expect(TK_COMMA)?;
                let line = kw.token == LINE_INPUT_HASH;
                loop {
                    let (loc, ty) = it.var_ref(self)?;
                    let field = self.channel_read_field(n, line)?;
                    let v = if ty == 2 { Value::Str(field.into()) } else { it.val(&field) };
                    it.write_loc(&loc, ty, v)?;
                    if it.peek() == TK_COMMA {
                        it.pc += 2;
                    } else {
                        break;
                    }
                }
            }
            FIELD => {
                // Field #n, len As var$, ...
                if it.peek() == TK_HASH {
                    it.pc += 2;
                }
                let n = channel_number(it.eval_int(self)?)?;
                let mut fields = Vec::new();
                let mut total = 0;
                while it.peek() == TK_COMMA {
                    it.pc += 2;
                    let len = it.eval_int(self)?.max(0) as usize;
                    it.expect(AS)?;
                    let (loc, ty) = it.var_ref(self)?;
                    total += len;
                    fields.push((len, loc, ty));
                }
                let ch = self.files.channels.get_mut(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
                ch.record_len = total;
                self.fields.insert(n, fields);
            }
            PUT | GET => {
                let a = it.inst_args(self, kw)?;
                let n = channel_number(a.int(0))?;
                let rec = a.int(1).max(1) as usize - 1;
                let fields = self.fields.get(&n).cloned().unwrap_or_default();
                let ch = self.files.channels.get_mut(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
                if ch.mode != ChannelMode::Random || ch.record_len == 0 {
                    return err(98);
                }
                let start = rec * ch.record_len;
                if kw.token == PUT {
                    let mut buf = Vec::with_capacity(ch.record_len);
                    for (len, loc, ty) in &fields {
                        let s = match it.read_loc(loc, *ty) {
                            Value::Str(s) => s,
                            _ => empty_str(),
                        };
                        let mut f = s.to_vec();
                        f.resize(*len, b' ');
                        buf.extend(f);
                    }
                    let ch = self.files.channels.get_mut(&n).unwrap();
                    if ch.data.len() < start + buf.len() {
                        ch.data.resize(start + buf.len(), 0);
                    }
                    ch.data[start..start + buf.len()].copy_from_slice(&buf);
                    ch.dirty = true;
                    self.flush_channel(n)?;
                } else {
                    if start >= ch.data.len() {
                        return err(errors::END_OF_FILE);
                    }
                    let data = ch.data.clone();
                    let mut p = start;
                    for (len, loc, ty) in fields {
                        let end = (p + len).min(data.len());
                        it.write_loc(&loc, ty, Value::Str(astr(&data[p.min(end)..end])))?;
                        p += len;
                    }
                }
            }
            POF if reserved => {
                // Pof(n)=position
                it.expect(crate::tokens::TK_PAR1)?;
                let n = channel_number(it.eval_int(self)?)?;
                it.expect(crate::tokens::TK_PAR2)?;
                it.expect(OP_EQ)?;
                let pos = it.eval_int(self)?.max(0) as usize;
                let ch = self.files.channels.get_mut(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
                ch.pos = pos.min(ch.data.len());
            }
            DIR_S if reserved => {
                it.expect(OP_EQ)?;
                let s = it.eval_str(self)?;
                let p = crate::detok::latin1_to_string(&s);
                // A pattern is kept as the directory filter.
                let (dir, pat) = crate::files::split_pattern(&p);
                if let Some(pat) = pat {
                    self.files.dir_filter = pat;
                }
                if !dir.is_empty() {
                    self.files.set_current_dir(&dir).map_err(fs_err)?;
                }
            }
            PARENT => self.files.parent(),
            KILL => {
                let a = it.inst_args(self, kw)?;
                self.files.delete(&crate::detok::latin1_to_string(&a.str(0))).map_err(fs_err)?;
            }
            RENAME => {
                let a = it.inst_args(self, kw)?;
                let from = crate::detok::latin1_to_string(&a.str(0));
                let to = crate::detok::latin1_to_string(&a.str(1));
                self.files.rename(&from, &to).map_err(fs_err)?;
            }
            MKDIR => {
                let a = it.inst_args(self, kw)?;
                self.files.make_dir(&crate::detok::latin1_to_string(&a.str(0))).map_err(fs_err)?;
            }
            DIR | DIR_2 | DIR_W | DIR_W_2 | LDIR | LDIR_2 | LDIR_W | LDIR_W_2 => {
                let a = it.inst_args(self, kw)?;
                let path = if a.is_empty() { self.dir_pattern() } else { crate::detok::latin1_to_string(&a.str(0)) };
                let wide = matches!(kw.token, DIR_W | DIR_W_2 | LDIR_W | LDIR_W_2);
                let text = self.dir_listing(&path, wide)?;
                self.print(it, &text)?;
            }
            SET_DIR | SET_DIR_2 => {
                let a = it.inst_args(self, kw)?;
                if a.len() > 1 {
                    self.files.dir_filter = crate::detok::latin1_to_string(&a.str(1));
                }
            }
            SET_INPUT => {
                let a = it.inst_args(self, kw)?;
                self.input_separators = (a.int(0), a.int(1));
            }
            RUN | RUN_2 => {
                let a = it.inst_args(self, kw)?;
                if a.is_empty() {
                    return Err(Exc::Message("Run without a file name is only available from the editor".into()));
                }
                self.pending_run = Some(a.str(0).to_vec());
                return Err(Exc::Stop(StopReason::End));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(crate) fn files_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        if kw.slot != 0 {
            return Ok(None);
        }
        use tk::*;
        let v = match kw.token {
            EOF | LOF | POF => {
                let n = channel_number(it.func_args(self, kw)?.int(0))?;
                let ch = self.files.channels.get(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
                Value::Int(match kw.token {
                    EOF => {
                        if ch.pos >= ch.data.len() {
                            -1
                        } else {
                            0
                        }
                    }
                    LOF => ch.data.len() as i32,
                    _ => ch.pos as i32,
                })
            }
            INPUT_S_2 => {
                // Input$(f,n): n bytes from a file.
                let a = it.func_args(self, kw)?;
                let n = channel_number(a.int(0))?;
                let count = a.int(1).max(0) as usize;
                let ch = self.files.channels.get_mut(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
                if ch.pos + count > ch.data.len() {
                    return err(errors::END_OF_FILE);
                }
                let s = astr(&ch.data[ch.pos..ch.pos + count]);
                ch.pos += count;
                Value::Str(s)
            }
            EXIST => {
                let p = it.func_args(self, kw)?.str(0);
                Value::Int(if self.files.exists(&crate::detok::latin1_to_string(&p)) { -1 } else { 0 })
            }
            DIR_S => Value::Str(astr(self.files.current_dir.as_bytes())),
            DIR_FIRST_S => {
                let p = crate::detok::latin1_to_string(&it.func_args(self, kw)?.str(0));
                let entries = self.files.list(&p).map_err(fs_err)?;
                self.files.dir_listing = entries.iter().map(|e| dir_entry_line(e, false)).collect();
                self.files.dir_listing_pos = 0;
                Value::Str(self.next_dir_entry())
            }
            DIR_NEXT_S => Value::Str(self.next_dir_entry()),
            DRIVE => {
                let p = crate::detok::latin1_to_string(&it.func_args(self, kw)?.str(0));
                let vol = p.split(':').next().unwrap_or("");
                Value::Int(if self.files.volume_names().iter().any(|v| v.eq_ignore_ascii_case(vol)) { -1 } else { 0 })
            }
            DFREE => Value::Int(512 * 1024),
            FSEL_S | FSEL_S_2 | FSEL_S_3 | FSEL_S_4 => {
                it.func_args(self, kw)?;
                // The file selector needs the requester UI: behave as Cancel.
                Value::Str(empty_str())
            }
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// Reads a whole file (AMOS path, Latin-1 bytes). Errors are AMOS disc
    /// errors (81 File not found...).
    pub(crate) fn files_read_all(&mut self, path: &[u8]) -> R<Vec<u8>> {
        let p = crate::detok::latin1_to_string(path);
        self.files.read(&p).map_err(fs_err)
    }

    /// Writes a whole file.
    pub(crate) fn files_write_all(&mut self, path: &[u8], data: &[u8]) -> R<()> {
        let p = crate::detok::latin1_to_string(path);
        self.files.write(&p, data).map_err(fs_err)
    }


    fn close_channel(&mut self, n: u32) -> R<()> {
        self.flush_channel(n)?;
        self.files.channels.remove(&n);
        self.fields.remove(&n);
        Ok(())
    }

    /// Writes a modified channel back to its file.
    fn flush_channel(&mut self, n: u32) -> R<()> {
        if let Some(ch) = self.files.channels.get(&n)
            && ch.dirty
        {
            let (path, data) = (ch.path.clone(), ch.data.clone());
            self.files.write(&path, &data).map_err(fs_err)?;
            self.files.channels.get_mut(&n).unwrap().dirty = false;
        }
        Ok(())
    }

    fn channel_write(&mut self, n: i32, data: &[u8]) -> R<()> {
        let n = channel_number(n)?;
        let ch = self.files.channels.get_mut(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
        if ch.mode == ChannelMode::In {
            return err(98);
        }
        let end = ch.pos + data.len();
        if ch.data.len() < end {
            ch.data.resize(end, 0);
        }
        ch.data[ch.pos..end].copy_from_slice(data);
        ch.pos = end;
        ch.dirty = true;
        self.flush_channel(n)
    }

    /// Reads one Input # field: up to a comma (Input) or the end of line.
    fn channel_read_field(&mut self, n: i32, line: bool) -> R<Vec<u8>> {
        let n = channel_number(n)?;
        let (eol1, eol2) = self.input_separators;
        let ch = self.files.channels.get_mut(&n).ok_or(Exc::Error(errors::FILE_NOT_OPENED))?;
        if ch.pos >= ch.data.len() {
            return err(errors::END_OF_FILE);
        }
        let mut out = Vec::new();
        while ch.pos < ch.data.len() {
            let c = ch.data[ch.pos];
            ch.pos += 1;
            if (!line && c == b',') || c as i32 == eol1 || c as i32 == eol2 || c == 10 || c == 13 {
                // CR LF counts as one end of line.
                if c == 13 && ch.data.get(ch.pos) == Some(&10) {
                    ch.pos += 1;
                }
                break;
            }
            out.push(c);
            if out.len() >= 1000 {
                return err(99);
            }
        }
        Ok(out)
    }

    fn next_dir_entry(&mut self) -> crate::interp::value::AStr {
        let i = self.files.dir_listing_pos;
        self.files.dir_listing_pos += 1;
        match self.files.dir_listing.get(i) {
            Some(s) => astr(s.as_bytes()),
            None => empty_str(),
        }
    }

    fn dir_pattern(&self) -> String {
        let d = self.files.current_dir.clone();
        if self.files.dir_filter.is_empty() {
            d
        } else if d.ends_with(':') || d.ends_with('/') {
            format!("{d}{}", self.files.dir_filter)
        } else {
            format!("{d}/{}", self.files.dir_filter)
        }
    }

    /// Text printed by `Dir`.
    fn dir_listing(&self, path: &str, wide: bool) -> R<Vec<u8>> {
        let entries = self.files.list(path).map_err(fs_err)?;
        let mut out = Vec::new();
        out.extend(format!("Directory of {path}\r\n").bytes());
        for (i, e) in entries.iter().enumerate() {
            out.extend(dir_entry_line(e, wide).bytes());
            if !wide || i % 2 == 1 {
                out.extend(b"\r\n");
            }
        }
        if wide && entries.len() % 2 == 1 {
            out.extend(b"\r\n");
        }
        Ok(out)
    }
}

/// Directory line: files as "name   size", directories as "*name".
fn dir_entry_line(e: &crate::files::DirEntry, wide: bool) -> String {
    if wide {
        let n = if e.is_dir { format!("*{}", e.name) } else { e.name.clone() };
        format!("{n:<20}")
    } else if e.is_dir {
        format!("*{}", e.name)
    } else {
        format!(" {:<24}{:>8}", e.name, e.size)
    }
}

fn channel_number(n: i32) -> R<u32> {
    if !(1..=10).contains(&n) {
        return err(errors::ILLEGAL_FUNCTION_CALL);
    }
    Ok(n as u32)
}
