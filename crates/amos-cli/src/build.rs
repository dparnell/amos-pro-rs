//! `amos-cli build`: turns an AMOS program into a standalone application.
//!
//! ```text
//! amos-cli build PROGRAM [options]
//!     --out DIR            output directory (default: ./build)
//!     --name NAME          application name (default: program name)
//!     --web / --native     outputs to produce (default: both)
//!     --include DIR        bundle the files of DIR (default: the program's
//!                          folder); --no-files bundles the program only
//!     --runtime FILE       native runtime executable (default: the `amos`
//!                          app next to this tool)
//!     --web-runtime DIR    folder with amos_app.js and amos_app_bg.wasm
//!                          (default: web/pkg of the source tree)
//!     --interpreted        do not compile the program to WebAssembly (the
//!                          application interprets it)
//! ```

use std::path::{Path, PathBuf};

use amos_core::bundle::Bundle;

pub fn build(args: &[String]) -> Result<(), String> {
    let program = PathBuf::from(args.first().ok_or("build needs a program")?);
    let mut out = PathBuf::from("build");
    let mut name = None;
    let (mut web, mut native) = (false, false);
    let mut include: Option<PathBuf> = program.parent().map(Path::to_path_buf).filter(|p| !p.as_os_str().is_empty());
    let mut runtime = None;
    let mut web_runtime = None;
    let mut compile = true;
    let mut i = 1;
    while i < args.len() {
        let v = || args.get(i + 1).cloned().ok_or(format!("{} needs a value", args[i]));
        match args[i].as_str() {
            "--out" => {
                out = PathBuf::from(v()?);
                i += 1;
            }
            "--name" => {
                name = Some(v()?);
                i += 1;
            }
            "--web" => web = true,
            "--native" => native = true,
            "--include" => {
                include = Some(PathBuf::from(v()?));
                i += 1;
            }
            "--no-files" => include = None,
            "--interpreted" => compile = false,
            "--runtime" => {
                runtime = Some(PathBuf::from(v()?));
                i += 1;
            }
            "--web-runtime" => {
                web_runtime = Some(PathBuf::from(v()?));
                i += 1;
            }
            o => return Err(format!("unknown option {o}")),
        }
        i += 1;
    }
    if !web && !native {
        web = true;
        native = true;
    }
    let mut bundle = Bundle::from_directory(&program, include.as_deref()).map_err(|e| e.to_string())?;
    // The program must load and pass the test before it is shipped.
    let prg = super::load(&program)?;
    amos_core::interp::verify::Verifier::verify(&prg.source, prg.math_flags)
        .map_err(|e| format!("test error: {}", amos_core::errors::test_message(e.code)))?;
    let name = name.unwrap_or_else(|| {
        program.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "app".into())
    });
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    println!("Bundled {} files ({} KB)", bundle.files.len(), bundle.data_size().div_ceil(1024));
    if compile {
        match amos_build::compile_bundle(&mut bundle) {
            Ok(msg) => println!("Program {msg}"),
            Err(e) => println!("warning: the program could not be compiled ({e}); it will be interpreted"),
        }
    }

    if native {
        let rt = runtime.map_or_else(default_runtime, Ok)?;
        let exe = std::fs::read(&rt).map_err(|e| format!("{}: {e}", rt.display()))?;
        let dest = amos_build::build_native(&bundle, &name, &out, &exe)?;
        println!("Native application: {}", dest.display());
    }
    if web {
        let src = web_runtime.or_else(amos_build::find_web_runtime).unwrap_or_else(|| PathBuf::from("web/pkg"));
        let dir = amos_build::build_web(&bundle, &name, &out, &src)?;
        println!("Web application: {}/ (serve it with any static web server)", dir.display());
    }
    Ok(())
}

/// The `amos` app built next to this tool.
fn default_runtime() -> Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let rt = me.with_file_name(format!("amos{}", std::env::consts::EXE_SUFFIX));
    if rt.exists() {
        Ok(rt)
    } else {
        Err(format!("runtime not found at {} (build it with cargo build -p amos-app, or use --runtime)", rt.display()))
    }
}
