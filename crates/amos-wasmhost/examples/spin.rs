//! Scratch: a program run for many frames (for a profiler), compiled, or
//! interpreted with a third argument `i`.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: usize = args[0].parse().unwrap();
    let mut m = amos_core::Machine::new();
    // A program file (its folder as the current directory), or the source.
    let prg = if args[1].ends_with(".AMOS") {
        let path = std::path::Path::new(&args[1]);
        // (A copy: programs may write files.)
        if let Some(dir) = path.parent() {
            let tmp = std::env::temp_dir().join(format!("amos-spin-{}", std::process::id()));
            copy_dir(dir, &tmp);
            m.hw.files.set_native_root(&tmp);
        }
        amos_core::Program::load(&std::fs::read(path).unwrap()).unwrap()
    } else {
        amos_core::tokenise::tokenise_program(args[1].replace("\\n", "\n").as_bytes()).unwrap()
    };
    let mut cp = None;
    if args.get(2).is_some_and(|a| a == "i") {
        m.run_program(&prg).unwrap();
    } else {
        let wasm = amos_compiler::compile(&prg).unwrap();
        cp = Some(amos_wasmhost::CompiledProgram::start(&mut m, &prg, &wasm).unwrap());
    }
    let t = std::time::Instant::now();
    for _ in 0..frames {
        match &mut cp {
            Some(cp) => cp.vbl(&mut m),
            None => m.vbl(),
        }
    }
    println!("{:?} per frame, {} instructions per frame", t.elapsed() / frames as u32, m.instructions_per_frame);
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    let _ = std::fs::create_dir_all(to);
    for e in std::fs::read_dir(from).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            let _ = std::fs::copy(&p, to.join(e.file_name()));
        }
    }
}
