//! `amos-cli compile PROG [-o OUT.wasm] [--verbose]`: compiles a program to
//! a WebAssembly module (see `docs/COMPILER.md`), and `run --compiled`.

use std::path::PathBuf;
use std::time::Instant;

use amos_core::{Machine, Program};
use amos_wasmhost::CompiledProgram;

pub fn compile(args: &[String]) -> Result<(), String> {
    let path = PathBuf::from(&args[0]);
    let mut out = path.with_extension("wasm");
    let mut verbose = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                out = PathBuf::from(args.get(i + 1).ok_or("-o needs a file")?);
                i += 1;
            }
            "--verbose" | "-v" => verbose = true,
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    let prg = super::load(&path)?;
    let t = Instant::now();
    let o = amos_compiler::compile_full(&prg).map_err(|e| e.to_string())?;
    let ms = t.elapsed().as_secs_f64() * 1e3;
    std::fs::write(&out, &o.wasm).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "{}: {} instructions, {} run by the interpreter, {} bytes in {ms:.1} ms",
        out.display(),
        o.instructions,
        o.interpreted.len(),
        o.wasm.len()
    );
    if verbose {
        for (pos, why) in &o.interpreted {
            println!("  interpreted at {pos}: {why}");
        }
    }
    Ok(())
}

/// Compiles `prg` and starts it on `m` (for `run --compiled`).
pub fn start(m: &mut Machine, prg: &Program) -> Result<CompiledProgram, String> {
    let t = Instant::now();
    let o = amos_compiler::compile_full(prg).map_err(|e| e.to_string())?;
    let p = CompiledProgram::start(m, prg, &o.wasm)?;
    println!(
        "-- compiled: {} instructions ({} interpreted), {} bytes, {:.1} ms",
        o.instructions,
        o.interpreted.len(),
        o.wasm.len(),
        t.elapsed().as_secs_f64() * 1e3
    );
    Ok(p)
}
