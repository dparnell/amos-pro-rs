//! Runs example programs interpreted and compiled on a full Machine and
//! compares every frame, the logs and the final state.
//!
//! `cargo run --release -p amos-wasmhost --example compare -- [FRAMES] [FILES...]`
//! (all the examples of `AMOS-Professional-365/AMOS` without files).

use amos_core::Machine;
use amos_core::display::render_rgba;
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

/// Copies the program's directory to a fresh temporary directory, so
/// programs that save files never write into the original distribution.
fn temp_copy(dir: &std::path::Path, tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("amos-compare-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    copy_dir(dir, &base);
    base
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

fn hash(v: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in v {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(100);
    let mut files: Vec<std::path::PathBuf> = args.iter().skip(1).map(Into::into).collect();
    if files.is_empty() {
        walk(std::path::Path::new("AMOS-Professional-365/AMOS"), &mut files);
        files.sort();
    }
    let (mut same, mut differ, mut skipped) = (0, 0, 0);
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let Ok(prg) = amos_core::Program::load(&data) else { continue };
        let wasm = match amos_compiler::compile(&prg) {
            Ok(w) => w,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let dir = f.parent().unwrap();
        let (dir_a, dir_b) = (temp_copy(dir, "a"), temp_copy(dir, "b"));
        let mut a = Machine::new();
        a.hw.files.set_native_root(&dir_a);
        if a.run_program(&prg).is_err() {
            skipped += 1;
            continue;
        }
        let mut b = Machine::new();
        b.hw.files.set_native_root(&dir_b);
        let mut cp = match CompiledProgram::start(&mut b, &prg, &wasm) {
            Ok(c) => c,
            Err(e) => {
                println!("{}: start failed: {e}", f.display());
                differ += 1;
                continue;
            }
        };
        let mut diff = None;
        for fr in 0..frames {
            input(&mut a, fr);
            input(&mut b, fr);
            a.vbl();
            cp.vbl(&mut b);
            let (la, lb) = (std::mem::take(&mut a.hw.log), std::mem::take(&mut b.hw.log));
            if la != lb {
                diff = Some(format!("frame {fr}: log {la:?} vs {lb:?}"));
                break;
            }
            if a.state != b.state {
                diff = Some(format!("frame {fr}: state {:?} vs {:?}", a.state, b.state));
                break;
            }
            if fr % 5 == 4 || fr + 1 == frames {
                let (ha, hb) = (hash(&render_rgba(&a.frame())), hash(&render_rgba(&b.frame())));
                if ha != hb {
                    diff = Some(format!("frame {fr}: display differs"));
                    break;
                }
            }
            if !a.interp.running && !b.interp.running {
                break;
            }
        }
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
        match diff {
            None => same += 1,
            Some(d) => {
                differ += 1;
                println!("{}: {d}", f.display());
            }
        }
    }
    println!("{same} identical, {differ} different, {skipped} skipped");
}
