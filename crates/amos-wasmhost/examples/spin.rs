//! Scratch: a compiled program run for many frames (for a profiler).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: usize = args[0].parse().unwrap();
    let prg = amos_core::tokenise::tokenise_program(args[1].replace("\\n", "\n").as_bytes()).unwrap();
    let wasm = amos_compiler::compile(&prg).unwrap();
    let mut m = amos_core::Machine::new();
    let mut cp = amos_wasmhost::CompiledProgram::start(&mut m, &prg, &wasm).unwrap();
    let t = std::time::Instant::now();
    for _ in 0..frames {
        cp.vbl(&mut m);
    }
    println!("{:?} per frame, {} instructions per frame", t.elapsed() / frames as u32, m.instructions_per_frame);
}
