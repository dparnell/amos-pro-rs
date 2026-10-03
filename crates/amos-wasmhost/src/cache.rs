//! On-disk cache of the native code compiled from modules (wasmtime
//! `Module::serialize`), so that running a program again skips the JIT.
//!
//! An entry is named after a hash of the wasm bytes, wasmtime's
//! compatibility hash of the engine (version, configuration and target) and
//! the target; its content is checked (header, key and a checksum of the
//! code) before wasmtime loads it. Anything wrong (missing, unreadable,
//! corrupt, made by another wasmtime or configuration) is ignored: the
//! module is compiled and the entry written again. Writing never fails the
//! caller.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use wasmtime::{Engine, Module};

const MAGIC: &[u8; 8] = b"AMOSJIT\0";
/// Version of the entry format.
const FORMAT: u32 = 1;
const HEADER: usize = 8 + 4 + 8 + 8 + 8;

/// FNV-1a 64 (stable across runs and builds, unlike `DefaultHasher`).
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = Fnv::new();
    h.write(bytes);
    h.finish()
}

/// The key of a module: its bytes, the engine and the target.
fn key(engine: &Engine, wasm: &[u8]) -> u64 {
    let mut h = Fnv::new();
    h.write(&fnv(wasm).to_le_bytes());
    h.write(&(wasm.len() as u64).to_le_bytes());
    engine.precompile_compatibility_hash().hash(&mut h);
    std::env::consts::ARCH.hash(&mut h);
    std::env::consts::OS.hash(&mut h);
    h.finish()
}

fn entry_path(dir: &Path, key: u64, wasm: &[u8]) -> PathBuf {
    dir.join(format!("{key:016x}-{:x}.cwasm", wasm.len()))
}

/// The compiled module of `wasm`: from the cache in `dir` when a valid
/// entry is there, else compiled (and the entry written).
pub fn module(engine: &Engine, wasm: &[u8], dir: &Path) -> Result<Module, String> {
    let key = key(engine, wasm);
    let path = entry_path(dir, key, wasm);
    if let Some(m) = load(engine, &path, key) {
        return Ok(m);
    }
    let module = Module::new(engine, wasm).map_err(|e| e.to_string())?;
    store(&module, dir, &path, key);
    Ok(module)
}

fn load(engine: &Engine, path: &Path, key: u64) -> Option<Module> {
    let data = std::fs::read(path).ok()?;
    if data.len() < HEADER || &data[..8] != MAGIC {
        return None;
    }
    let word = |i: usize, n: usize| -> u64 {
        let mut b = [0u8; 8];
        b[..n].copy_from_slice(&data[i..i + n]);
        u64::from_le_bytes(b)
    };
    let (format, k, len, sum) = (word(8, 4) as u32, word(12, 8), word(20, 8), word(28, 8));
    let code = &data[HEADER..];
    if format != FORMAT || k != key || len != code.len() as u64 || sum != fnv(code) {
        return None;
    }
    // SAFETY: the bytes are what `Module::serialize` produced for this
    // engine and module (key and checksum checked above), in a cache
    // directory the caller owns; wasmtime also checks its own header
    // (version, configuration) and fails instead of loading a mismatch.
    unsafe { Module::deserialize(engine, code) }.ok()
}

fn store(module: &Module, dir: &Path, path: &Path, key: u64) {
    let Ok(code) = module.serialize() else { return };
    let mut data = Vec::with_capacity(HEADER + code.len());
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&FORMAT.to_le_bytes());
    data.extend_from_slice(&key.to_le_bytes());
    data.extend_from_slice(&(code.len() as u64).to_le_bytes());
    data.extend_from_slice(&fnv(&code).to_le_bytes());
    data.extend_from_slice(&code);
    // Written next to the entry, then renamed: readers never see half an
    // entry.
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".{key:016x}-{}.tmp", std::process::id()));
    if std::fs::write(&tmp, &data).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("amos-jit-cache-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn wasm() -> Vec<u8> {
        let prg = amos_core::tokenise::tokenise_program(b"A=1 : For I=1 To 10 : A=A*2 : Next : Print A").unwrap();
        amos_compiler::compile(&prg).unwrap()
    }

    #[test]
    fn entries_are_written_reused_and_rebuilt() {
        let engine = crate::engine();
        let dir = temp_dir("reuse");
        let wasm = wasm();
        let m1 = module(engine, &wasm, &dir).unwrap();
        let path = entry_path(&dir, key(engine, &wasm), &wasm);
        assert!(path.exists());
        // Reused (same exports), and still valid after a reload.
        let m2 = module(engine, &wasm, &dir).unwrap();
        assert_eq!(m1.exports().count(), m2.exports().count());
        assert!(load(engine, &path, key(engine, &wasm)).is_some());
        // Corrupt entries (code, truncated, other key) are ignored and rebuilt.
        let good = std::fs::read(&path).unwrap();
        for bad in [
            {
                let mut d = good.clone();
                let i = d.len() - 10;
                d[i] ^= 0x55;
                d
            },
            good[..good.len() / 2].to_vec(),
            b"garbage".to_vec(),
            {
                let mut d = good.clone();
                d[12] ^= 1;
                d
            },
        ] {
            std::fs::write(&path, &bad).unwrap();
            assert!(load(engine, &path, key(engine, &wasm)).is_none());
            module(engine, &wasm, &dir).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), good);
        }
        // Another module: another entry.
        let prg = amos_core::tokenise::tokenise_program(b"Print 2").unwrap();
        let other = amos_compiler::compile(&prg).unwrap();
        module(engine, &other, &dir).unwrap();
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        // An unwritable directory: compiled anyway.
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        module(engine, &wasm, &file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
