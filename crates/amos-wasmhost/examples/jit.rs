//! JIT setup of the compiled examples: wasm size and wasmtime compile time.
//! `cargo run --release -p amos-wasmhost --example jit`
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
fn main() {
    let mut files = Vec::new();
    walk(std::path::Path::new("AMOS-Professional-365/AMOS"), &mut files);
    files.sort();
    let engine = amos_wasmhost::engine();
    let mut rows = Vec::new();
    for f in files {
        let Ok(prg) = amos_core::Program::load(&std::fs::read(&f).unwrap()) else { continue };
        let Ok(wasm) = amos_compiler::compile(&prg) else { continue };
        // (Minimum of 3: less sensitive to the load of the machine.)
        let d = (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                let _ = wasmtime::Module::new(engine, &wasm).unwrap();
                t.elapsed()
            })
            .min()
            .unwrap();
        rows.push((d, wasm.len(), f.display().to_string()));
    }
    let total: std::time::Duration = rows.iter().map(|r| r.0).sum();
    let bytes: usize = rows.iter().map(|r| r.1).sum();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    println!(
        "{} modules, {} KB, JIT {:.1} ms total, {:.2} ms average",
        rows.len(),
        bytes / 1024,
        total.as_secs_f64() * 1e3,
        total.as_secs_f64() * 1e3 / rows.len() as f64
    );
    for (d, b, n) in rows.iter().take(8) {
        println!("{:>8.1} ms {:>7} B  {n}", d.as_secs_f64() * 1e3, b);
    }
}
