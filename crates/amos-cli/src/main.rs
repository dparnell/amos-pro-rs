//! Command line tool for working on the AMOS port without a window.
//!
//! ```text
//! amos-cli list prog.AMOS                 list a program as text
//! amos-cli run prog.AMOS|prog.txt [opts]  run headless
//!     --frames N      number of 1/50 s frames to run (default 150)
//!     --png FILE      save the final display as a PNG
//!     --keys TEXT     type TEXT (\n = Return) during the run
//!     --dir DIR       directory used as the current AMOS directory
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use amos_core::display::{DISPLAY_HEIGHT, DISPLAY_WIDTH, render_rgba};
use amos_core::input::InputEvent;
use amos_core::interp::RunState;
use amos_core::{Machine, Program};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("list") if args.len() >= 2 => list(Path::new(&args[1])),
        Some("run") if args.len() >= 2 => run(&args[1..]),
        _ => {
            eprintln!("usage: amos-cli list FILE | run FILE [--frames N] [--png FILE] [--keys TEXT]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load(path: &Path) -> Result<Program, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.starts_with(b"AMOS") {
        Program::load(&data).map_err(|e| e.to_string())
    } else {
        amos_core::tokenise::tokenise_program(&data).map_err(|(line, e)| format!("line {line}: {e:?}"))
    }
}

fn list(path: &Path) -> Result<(), String> {
    let prg = load(path)?;
    let text = amos_core::detok::list_program(&prg);
    print!("{}", amos_core::detok::latin1_to_string(&text));
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let path = PathBuf::from(&args[0]);
    let mut frames = 150;
    let mut png_path = None;
    let mut keys = String::new();
    let mut dir = None;
    let mut i = 1;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--frames" => frames = v.parse().map_err(|_| "bad --frames")?,
            "--png" => png_path = Some(PathBuf::from(v)),
            "--keys" => keys = v.replace("\\n", "\n"),
            "--dir" => dir = Some(PathBuf::from(v)),
            other => return Err(format!("unknown option {other}")),
        }
        i += 2;
    }
    let prg = load(&path)?;
    let mut m = Machine::new();
    let base = dir.or_else(|| path.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
    m.hw.files.set_native_root(&base);
    if let Err(e) = m.run_program(&prg) {
        return Err(format!("test error: {} at {}", amos_core::errors::test_message(e.code), e.pos));
    }
    let mut keys = keys.chars();
    for frame in 0..frames {
        // Type one character every 5 frames.
        if frame % 5 == 4
            && let Some(c) = keys.next()
        {
            m.input(InputEvent::Char(c));
        }
        m.vbl();
        for line in m.hw.log.drain(..) {
            println!("{line}");
        }
        if let RunState::Stopped(_) = m.state {
            break;
        }
    }
    println!("-- state after run: {:?}", m.state);
    if let Some(p) = png_path {
        let rgba = render_rgba(&m.frame());
        save_png(&p, &rgba)?;
        println!("-- saved {}", p.display());
    }
    Ok(())
}

fn save_png(path: &Path, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), DISPLAY_WIDTH, DISPLAY_HEIGHT);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    w.write_image_data(rgba).map_err(|e| e.to_string())?;
    Ok(())
}
