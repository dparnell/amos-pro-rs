//! Compiles every example program; prints statistics.
use std::collections::HashMap;
fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("amos")) {
            out.push(p)
        }
    }
}
fn main() {
    let mut files = Vec::new();
    walk(std::path::Path::new("AMOS-Professional-365/AMOS"), &mut files);
    let (mut total, mut interp, mut ok, mut errs) = (0, 0, 0, 0);
    let mut why: HashMap<&str, usize> = HashMap::new();
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let Ok(prg) = amos_core::Program::load(&data) else { continue };
        match amos_compiler::compile_full(&prg) {
            Ok(o) => {
                ok += 1;
                total += o.instructions;
                interp += o.interpreted.len();
                let c = amos_core::interp::verify::Verifier::verify(&prg.source, prg.math_flags).unwrap();
                for (p, w) in &o.interpreted {
                    let (kw, _) = amos_core::compiled::structure::keyword_at(&c.code, *p);
                    let name = amos_core::compiled::structure::keyword_def(kw).map_or("?", |d| d.name);
                    *why.entry(Box::leak(format!("{w}: {name}").into_boxed_str())).or_default() += 1;
                }
                let mut v = wasmparser::Validator::new();
                if let Err(e) = v.validate_all(&o.wasm) {
                    println!("INVALID {}: {e}", f.display());
                }
            }
            Err(amos_compiler::CompileError::Test(_)) => {}
            Err(e) => {
                errs += 1;
                println!("{}: {e}", f.display());
            }
        }
    }
    println!("{ok} compiled, {errs} errors; {total} instructions, {interp} interpreted");
    let mut w: Vec<_> = why.into_iter().collect();
    w.sort_by_key(|x| std::cmp::Reverse(x.1));
    for (k, n) in w {
        println!("  {n:6} {k}");
    }
}
