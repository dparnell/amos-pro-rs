//! Writes a compiled bundle (`.amospak`) for every synthetic workload of
//! the `suite` example (amos-wasmhost), for timing the web runtime under
//! node (`scripts/web-bench.mjs`):
//! `cargo run -p amos-build --example bundle_synthetic -- OUT_DIR`.

#[allow(dead_code)]
mod synthetic {
    include!("../../amos-wasmhost/examples/shared/synthetic.rs");
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir_all(&out).unwrap();
    for (i, (name, src)) in synthetic::SYNTHETIC.iter().enumerate() {
        let mut b = amos_core::bundle::Bundle {
            files: vec![("main.asc".to_string(), src.as_bytes().to_vec())],
            main: "main.asc".to_string(),
            module: None,
        };
        amos_build::compile_bundle(&mut b).expect(name);
        let stem: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        std::fs::write(out.join(format!("{i:02}_{stem}.amospak")), b.to_bytes()).unwrap();
    }
}
