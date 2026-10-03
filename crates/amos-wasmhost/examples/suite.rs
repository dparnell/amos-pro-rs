//! Realistic workloads on a full Machine: every example program (from a
//! temporary copy of its directory) and synthetic graphics / keyword
//! loops, interpreted and compiled, timing the frames (the programs yield at
//! the same instructions both ways, so the times compare the speed of the
//! same work).
//!
//! `cargo run --release -p amos-wasmhost --example suite -- [FRAMES] [TOP] [FILES... | -]` (`-`: synthetic
//! workloads only)

use std::time::{Duration, Instant};

use amos_core::Machine;
use amos_core::input::InputEvent;
use amos_wasmhost::CompiledProgram;

fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("amos")) {
            out.push(p)
        }
    }
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    let _ = std::fs::create_dir_all(to);
    let Ok(rd) = std::fs::read_dir(from) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            let _ = std::fs::copy(&p, &t);
        }
    }
}

/// Input played identically to both machines.
fn input(m: &mut Machine, frame: usize) {
    if frame % 40 == 20 {
        m.input(InputEvent::Char(' '));
    }
    if frame % 70 == 35 {
        m.input(InputEvent::Char('\r'));
    }
}

include!("shared/synthetic.rs");

struct Row {
    name: String,
    interp: Duration,
    compiled: Duration,
}

fn time_prg(prg: &amos_core::Program, dir: Option<&std::path::Path>, frames: usize) -> Option<(Duration, Duration)> {
    let wasm = amos_compiler::compile(prg).ok()?;
    let mut times = [Duration::ZERO; 2];
    for (k, t) in times.iter_mut().enumerate() {
        let tmp = dir.map(|d| {
            let base = std::env::temp_dir().join(format!("amos-suite-{}-{k}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            copy_dir(d, &base);
            base
        });
        let mut m = Machine::new();
        if let Some(t) = &tmp {
            m.hw.files.set_native_root(t);
        }
        let mut cp = None;
        if k == 0 {
            m.run_program(prg).ok()?;
        } else {
            cp = Some(CompiledProgram::start(&mut m, prg, &wasm).ok()?);
        }
        for fr in 0..frames {
            input(&mut m, fr);
            let s = Instant::now();
            match &mut cp {
                None => m.vbl(),
                Some(cp) => cp.vbl(&mut m),
            }
            *t += s.elapsed();
            if !m.interp.running {
                break;
            }
        }
        if let Some(t) = tmp {
            let _ = std::fs::remove_dir_all(t);
        }
    }
    Some((times[0], times[1]))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(200);
    let top: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(15);
    let mut files: Vec<std::path::PathBuf> = args.iter().skip(2).filter(|a| *a != "-").map(Into::into).collect();
    // "-": the synthetic workloads only.
    if files.is_empty() && !args.iter().any(|a| a == "-") {
        walk(std::path::Path::new("AMOS-Professional-365/AMOS"), &mut files);
        files.sort();
    }
    let mut rows = Vec::new();
    for (name, src) in SYNTHETIC {
        let prg = amos_core::tokenise::tokenise_program(src.as_bytes()).expect("tokenise");
        if let Some((i, c)) = time_prg(&prg, None, frames) {
            rows.push(Row { name: format!("* {name}"), interp: i, compiled: c });
        }
    }
    let synthetic = rows.len();
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let Ok(prg) = amos_core::Program::load(&data) else { continue };
        if let Some((i, c)) = time_prg(&prg, f.parent(), frames) {
            let name = f.strip_prefix("AMOS-Professional-365/AMOS").unwrap_or(f).display().to_string();
            rows.push(Row { name, interp: i, compiled: c });
        }
    }
    let (syn, ex) = rows.split_at_mut(synthetic);
    let total = |r: &[Row]| r.iter().fold((Duration::ZERO, Duration::ZERO), |a, r| (a.0 + r.interp, a.1 + r.compiled));
    ex.sort_by_key(|a| std::cmp::Reverse(a.interp));
    println!("{frames} frames; times spent in the frames (ms)");
    println!("{:<52} {:>10} {:>10} {:>8}", "program", "interp", "compiled", "speedup");
    let line = |r: &Row| {
        println!(
            "{:<52} {:>10.1} {:>10.1} {:>7.1}x",
            r.name.chars().take(52).collect::<String>(),
            r.interp.as_secs_f64() * 1e3,
            r.compiled.as_secs_f64() * 1e3,
            r.interp.as_secs_f64() / r.compiled.as_secs_f64().max(1e-9)
        )
    };
    syn.iter().for_each(line);
    ex.iter().take(top).for_each(line);
    let (ti, tc) = total(ex);
    println!(
        "all {} examples: interpreted {:.1} ms, compiled {:.1} ms ({:.2}x)",
        ex.len(),
        ti.as_secs_f64() * 1e3,
        tc.as_secs_f64() * 1e3,
        ti.as_secs_f64() / tc.as_secs_f64().max(1e-9)
    );
}
