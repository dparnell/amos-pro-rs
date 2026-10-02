//! Console version of the AMOS interpreter: Print goes to stdout, Input and
//! Wait Key read stdin. Everything else (files, banks, maths...) is the
//! normal machine running headless, so graphics instructions are accepted
//! but invisible.
//!
//! ```text
//! amos-console prog.AMOS|prog.txt   run a program
//! amos-console -e 'Print 1+2'       run code given on the command line
//! amos-console -                    read the program text from stdin
//! ```

use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use amos_core::Program;
use amos_core::interp::stmt::InputState;
use amos_core::interp::value::{Value, astr, empty_str};
use amos_core::interp::{Exc, Host, Interp, R, RunState, StopReason, StopReasonOrError};
use amos_core::machine::Hardware;
use amos_core::tokens::{Keyword, tk};

/// The headless machine with text input/output on stdio.
struct Console {
    hw: Hardware,
    /// Partially printed line state: last byte was CR (CR LF = one new line).
    pending_cr: bool,
    /// Bytes left of an ESC sequence being skipped.
    esc_skip: u8,
    /// ANSI escape codes allowed (stdout is a terminal).
    ansi: bool,
    stdin_eof: bool,
    /// Input just read a line: the user's Return already ended the line on
    /// the terminal, so the interpreter's echoed new line is dropped.
    after_input: bool,
}

impl Console {
    fn out(&mut self, text: &[u8]) {
        let mut s = String::new();
        for &c in text {
            if self.esc_skip > 0 {
                self.esc_skip -= 1;
                continue;
            }
            match c {
                27 => self.esc_skip = 2,
                13 => {
                    s.push('\n');
                    self.pending_cr = true;
                    continue;
                }
                10 => {
                    if !self.pending_cr {
                        s.push('\n');
                    }
                }
                9 => s.push('\t'),
                12 | 25 if self.ansi => s.push_str("\x1b[2J\x1b[H"),
                0..=31 => {}
                // ISO-8859-1 to Unicode.
                c => s.push(c as char),
            }
            self.pending_cr = false;
        }
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    }

    /// One line of stdin (without the line ending), None at end of input.
    fn read_stdin_line(&mut self) -> Option<Vec<u8>> {
        if self.stdin_eof {
            return None;
        }
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => {
                self.stdin_eof = true;
                None
            }
            Ok(_) => {
                let l = line.trim_end_matches(['\n', '\r']);
                Some(l.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect())
            }
        }
    }
}

impl Host for Console {
    fn instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        if kw.slot == 0 {
            match kw.token {
                tk::WAIT_KEY => {
                    // Return on the terminal stands for a key press.
                    if self.read_stdin_line().is_none() {
                        return Err(Exc::Stop(StopReason::End));
                    }
                    return Ok(());
                }
                tk::CLS | tk::CLS_2 | tk::CLS_3 | tk::CLW => {
                    it.inst_args(self, kw)?;
                    if self.ansi {
                        self.out(&[12]);
                    }
                    return Ok(());
                }
                tk::HOME => {
                    if self.ansi {
                        self.out_raw("\x1b[H");
                    }
                    return Ok(());
                }
                tk::LOCATE => {
                    let a = it.inst_args(self, kw)?;
                    if self.ansi {
                        let (x, y) = (a.opt(0).unwrap_or(0), a.opt(1).unwrap_or(0));
                        self.out_raw(&format!("\x1b[{};{}H", y + 1, x + 1));
                    }
                    return Ok(());
                }
                tk::CENTRE => {
                    let s = it.inst_args(self, kw)?.str(0);
                    let pad = 40usize.saturating_sub(s.len() / 2);
                    let mut t = vec![b' '; pad];
                    t.extend_from_slice(&s);
                    self.out(&t);
                    return Ok(());
                }
                _ => {}
            }
        }
        self.hw.instruction(it, kw)
    }

    fn function(&mut self, it: &mut Interp, kw: Keyword) -> R<Value> {
        if kw.slot == 0 && kw.token == tk::INKEY_S {
            // No raw keyboard on a console: nothing waiting.
            return Ok(Value::Str(empty_str()));
        }
        if kw.slot == 0 && kw.token == tk::INPUT_S {
            let n = it.func_args(self, kw)?.int(0).max(0) as usize;
            let mut buf = vec![0u8; n];
            let got = std::io::stdin().lock().read(&mut buf).unwrap_or(0);
            buf.truncate(got);
            return Ok(Value::Str(astr(&buf)));
        }
        self.hw.function(it, kw)
    }

    fn reserved_assign(&mut self, it: &mut Interp, kw: Keyword) -> R<()> {
        self.hw.reserved_assign(it, kw)
    }

    fn test_point(&mut self, it: &mut Interp) -> R<()> {
        self.hw.test_point(it)
    }

    fn take_break(&mut self) -> bool {
        false
    }

    fn print(&mut self, _it: &mut Interp, text: &[u8]) -> R<()> {
        if std::mem::take(&mut self.after_input) && text == b"\r\n" {
            return Ok(());
        }
        self.out(text);
        Ok(())
    }

    fn read_line(&mut self, _it: &mut Interp, _state: &mut InputState) -> R<Option<Vec<u8>>> {
        match self.read_stdin_line() {
            Some(l) => {
                self.after_input = std::io::stdin().is_terminal();
                Ok(Some(l))
            }
            // End of input: stop the program instead of waiting forever.
            None => Err(Exc::Stop(StopReason::End)),
        }
    }
}

impl Console {
    fn out_raw(&mut self, s: &str) {
        let mut o = std::io::stdout().lock();
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    }
}

fn load(arg: &str, args: &[String]) -> Result<(Program, PathBuf), String> {
    let (text, dir) = match arg {
        "-e" => (args.get(2).ok_or("-e needs code")?.replace("\\n", "\n").into_bytes(), PathBuf::from(".")),
        "-" => {
            let mut v = Vec::new();
            std::io::stdin().read_to_end(&mut v).map_err(|e| e.to_string())?;
            (v, PathBuf::from("."))
        }
        path => {
            let p = PathBuf::from(path);
            let data = std::fs::read(&p).map_err(|e| format!("{path}: {e}"))?;
            let dir = p.parent().map(|d| d.to_path_buf()).filter(|d| !d.as_os_str().is_empty());
            (data, dir.unwrap_or_else(|| PathBuf::from(".")))
        }
    };
    let prg = if text.starts_with(b"AMOS") {
        Program::load(&text).map_err(|e| e.to_string())?
    } else {
        // Text programs: UTF-8 is converted to ISO-8859-1.
        let latin: Vec<u8> = String::from_utf8_lossy(&text)
            .chars()
            .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
            .collect();
        amos_core::tokenise::tokenise_program(&latin).map_err(|(l, e)| format!("line {l}: {e:?}"))?
    };
    Ok((prg, dir))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(arg) = args.get(1) else {
        eprintln!("usage: amos-console FILE | -e CODE | -");
        return ExitCode::from(2);
    };
    let (prg, dir) = match load(arg, &args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut console = Console {
        hw: Hardware::new(),
        pending_cr: false,
        esc_skip: 0,
        ansi: std::io::stdout().is_terminal(),
        stdin_eof: false,
        after_input: false,
    };
    console.hw.files.set_native_root(&dir);
    console.hw.banks.load_program_banks(&prg.banks);
    let mut it = Interp::new();
    if let Err(e) = it.load(&prg) {
        eprintln!("Test error: {}", amos_core::errors::test_message(e.code));
        return ExitCode::FAILURE;
    }
    // 50 Hz loop: waits (Wait, Wait Vbl...) take real time.
    let frame = std::time::Duration::from_millis(20);
    loop {
        let start = std::time::Instant::now();
        console.hw.vbl();
        it.vbl();
        match it.run(&mut console, 1_000_000) {
            RunState::Running => {
                if it.wait.is_some() {
                    if let Some(rest) = frame.checked_sub(start.elapsed()) {
                        std::thread::sleep(rest);
                    }
                }
            }
            RunState::Stopped(info) => {
                return match info.reason {
                    StopReasonOrError::Stop(_) => ExitCode::SUCCESS,
                    StopReasonOrError::Error(n) => {
                        eprintln!("\n{}", amos_core::errors::message(n));
                        ExitCode::FAILURE
                    }
                    StopReasonOrError::Message(m) => {
                        eprintln!("\n{m}");
                        ExitCode::FAILURE
                    }
                    StopReasonOrError::Test(n) => {
                        eprintln!("\n{}", amos_core::errors::test_message(n));
                        ExitCode::FAILURE
                    }
                };
            }
            RunState::Idle => return ExitCode::SUCCESS,
        }
    }
}
