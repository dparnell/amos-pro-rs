//! Writes a compiled bundle (`.amospak`) for every example program, for
//! testing the runtimes (e.g. the web runtime under node):
//! `cargo run -p amos-build --example bundle_examples -- OUT_DIR`.
//! Only reads the distribution.

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
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    let mut files = Vec::new();
    walk(std::path::Path::new("AMOS-Professional-365/AMOS"), &mut files);
    files.sort();
    let (mut n, mut failed) = (0, 0);
    for (i, f) in files.iter().enumerate() {
        // The program and the files of its folder (not sub-folders: keeps
        // bundles small).
        let dir = f.parent().unwrap();
        let mut b = match amos_core::bundle::Bundle::from_directory(f, None) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if p.is_file() && name != b.main && !name.ends_with(".info") {
                    b.files.push((name, std::fs::read(&p).unwrap_or_default()));
                }
            }
        }
        if amos_build::compile_bundle(&mut b).is_err() {
            failed += 1;
            continue;
        }
        let stem = f.file_stem().unwrap().to_string_lossy();
        std::fs::write(out.join(format!("{i:03}_{stem}.amospak")), b.to_bytes()).unwrap();
        n += 1;
    }
    println!("{n} bundles written, {failed} not compiled");
}
