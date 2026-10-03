//! Instructions of the example programs left to the interpreter, by reason
//! and keyword: `cargo run --release -p amos-wasmhost --example fallbacks`.
use std::collections::BTreeMap;
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
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for f in files {
        let Ok(prg) = amos_core::Program::load(&std::fs::read(&f).unwrap()) else { continue };
        let Ok(out) = amos_compiler::compile_full(&prg) else { continue };
        let mut it = amos_core::interp::Interp::new();
        if it.load(&prg).is_err() {
            continue;
        }
        let c = it.prg.clone().unwrap();
        for (pos, why) in &out.interpreted {
            let (kw, _) = amos_core::compiled::structure::keyword_at(&c.code, *pos);
            let name = amos_core::compiled::structure::keyword_def(kw).map_or(format!("{:04X}", kw.token), |d| {
                d.name.trim_end_matches(|c: char| !c.is_ascii_graphic()).to_string()
            });
            *counts.entry((why.to_string(), name)).or_default() += 1;
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.1));
    for ((why, name), n) in v.iter().take(60) {
        println!("{n:5} {why:30} {name}");
    }
}
