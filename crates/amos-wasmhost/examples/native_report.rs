//! Prints which instructions of a program are compiled natively.
fn main() {
    let path = std::env::args().nth(1).expect("program");
    let data = std::fs::read(&path).unwrap();
    let prg = if data.starts_with(b"AMOS") {
        amos_core::Program::load(&data).unwrap()
    } else {
        amos_core::tokenise::tokenise_program(&data).unwrap()
    };
    let out = amos_compiler::compile_full(&prg).unwrap();
    println!("{} instructions, {} interpreted, {} bytes", out.instructions, out.interpreted.len(), out.wasm.len());
    for (p, why) in &out.interpreted {
        println!("  {p}: {why}");
    }
}
