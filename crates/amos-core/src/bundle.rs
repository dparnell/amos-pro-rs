//! Self-contained application bundles ("compiled" AMOS programs).
//!
//! A bundle holds a program (with its banks) and the data files it uses.
//! The runtime finds it appended to its own executable (native apps) or is
//! given it by the web page, mounts the files as the volume `Bundle:` and
//! runs the program without the editor.
//!
//! Layout: `MAGIC`, `u32` file count, then for each file `u16` path length,
//! path (UTF-8, '/' separated), `u32` data length, data; then `u16` length
//! and path of the main program; optionally the program compiled to
//! WebAssembly (`docs/COMPILER.md`): `WASM_TAG`, `u32` length, module.
//! Readers ignore what follows the main program path, so older runtimes
//! still read bundles with a module (and interpret the program). When
//! appended to an executable, the payload is followed by a trailer: `u64`
//! payload offset and `MAGIC`.

pub const MAGIC: &[u8; 8] = b"AMOSPAK1";

/// Tag of the compiled module section.
pub const WASM_TAG: &[u8; 4] = b"WASM";

/// Name of the volume holding the bundled files.
pub const VOLUME: &str = "Bundle";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    /// Files as (path relative to the bundle root, data).
    pub files: Vec<(String, Vec<u8>)>,
    /// Path of the program to run.
    pub main: String,
    /// The main program compiled to WebAssembly (`amos-compiler`), run
    /// instead of interpreting the program when the runtime supports it.
    pub module: Option<Vec<u8>>,
}

impl Bundle {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(self.files.len() as u32).to_be_bytes());
        for (path, data) in &self.files {
            out.extend_from_slice(&(path.len() as u16).to_be_bytes());
            out.extend_from_slice(path.as_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(data);
        }
        out.extend_from_slice(&(self.main.len() as u16).to_be_bytes());
        out.extend_from_slice(self.main.as_bytes());
        if let Some(m) = &self.module {
            out.extend_from_slice(WASM_TAG);
            out.extend_from_slice(&(m.len() as u32).to_be_bytes());
            out.extend_from_slice(m);
        }
        out
    }

    pub fn from_bytes(data: &[u8]) -> Option<Bundle> {
        let mut p = 0;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = data.get(p..p + n)?;
            p += n;
            Some(s)
        };
        if take(8)? != MAGIC {
            return None;
        }
        let count = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
        let mut files = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let n = u16::from_be_bytes(take(2)?.try_into().ok()?) as usize;
            let path = String::from_utf8(take(n)?.to_vec()).ok()?;
            let len = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
            files.push((path, take(len)?.to_vec()));
        }
        let n = u16::from_be_bytes(take(2)?.try_into().ok()?) as usize;
        let main = String::from_utf8(take(n)?.to_vec()).ok()?;
        let mut module = None;
        if take(4).is_some_and(|t| t == WASM_TAG) {
            let len = u32::from_be_bytes(take(4)?.try_into().ok()?) as usize;
            module = Some(take(len)?.to_vec());
        }
        Some(Bundle { files, main, module })
    }

    /// Appends the bundle to an executable image.
    pub fn append_to_executable(&self, exe: &[u8]) -> Vec<u8> {
        let mut out = exe.to_vec();
        let offset = out.len() as u64;
        out.extend(self.to_bytes());
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(MAGIC);
        out
    }

    /// Finds a bundle appended to an executable image.
    pub fn from_executable(exe: &[u8]) -> Option<Bundle> {
        let n = exe.len();
        if n < 16 || &exe[n - 8..] != MAGIC {
            return None;
        }
        let offset = u64::from_be_bytes(exe[n - 16..n - 8].try_into().ok()?) as usize;
        Bundle::from_bytes(exe.get(offset..n - 16)?)
    }

    /// Reads a bundle appended to an executable file without loading the
    /// whole executable.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_executable_file(path: &std::path::Path) -> Option<Bundle> {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(path).ok()?;
        let len = f.seek(SeekFrom::End(0)).ok()?;
        if len < 16 {
            return None;
        }
        let mut trailer = [0u8; 16];
        f.seek(SeekFrom::End(-16)).ok()?;
        f.read_exact(&mut trailer).ok()?;
        if &trailer[8..] != MAGIC {
            return None;
        }
        let offset = u64::from_be_bytes(trailer[..8].try_into().ok()?);
        if offset >= len - 16 {
            return None;
        }
        let mut data = vec![0u8; (len - 16 - offset) as usize];
        f.seek(SeekFrom::Start(offset)).ok()?;
        f.read_exact(&mut data).ok()?;
        Bundle::from_bytes(&data)
    }

    /// The executable without any appended bundle (to build a new one).
    pub fn strip_executable(exe: &[u8]) -> &[u8] {
        let n = exe.len();
        if n >= 16 && &exe[n - 8..] == MAGIC {
            let offset = u64::from_be_bytes(exe[n - 16..n - 8].try_into().unwrap()) as usize;
            if offset < n {
                return &exe[..offset];
            }
        }
        exe
    }

    /// Builds a bundle from a program and the files of a host directory
    /// (recursively; hidden files and Amiga `.info` icons are skipped).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_directory(program: &std::path::Path, dir: Option<&std::path::Path>) -> std::io::Result<Bundle> {
        fn walk(root: &std::path::Path, d: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) -> std::io::Result<()> {
            let mut entries: Vec<_> = std::fs::read_dir(d)?.filter_map(|e| e.ok()).collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name.to_lowercase().ends_with(".info") {
                    continue;
                }
                let p = e.path();
                if p.is_dir() {
                    walk(root, &p, out)?;
                } else {
                    let rel = p.strip_prefix(root).unwrap_or(&p);
                    let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                    out.push((rel, std::fs::read(&p)?));
                }
            }
            Ok(())
        }
        let main_name = program.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let mut files = Vec::new();
        if let Some(d) = dir {
            walk(d, d, &mut files)?;
        }
        // The program itself is always at the root of the bundle.
        files.retain(|(p, _)| *p != main_name);
        files.insert(0, (main_name.clone(), std::fs::read(program)?));
        Ok(Bundle { files, main: main_name, module: None })
    }

    /// Builds a bundle from an AMOS path, optionally with all the files of
    /// the program's directory (through the virtual file system, so it also
    /// works from the editor).
    pub fn from_vfs(fs: &crate::files::FileSystem, program: &str, with_dir: bool) -> crate::files::FsResult<Bundle> {
        let full = fs.full_path(program).ok_or(crate::files::FsError::NotFound)?;
        let split = full.rfind(['/', ':']).map_or(0, |i| i + 1);
        let (dir, main) = (&full[..split], full[split..].to_string());
        let mut files = if with_dir { fs.read_tree(dir)? } else { Vec::new() };
        files.retain(|(p, _)| *p != main);
        files.insert(0, (main.clone(), fs.read(&full)?));
        Ok(Bundle { files, main, module: None })
    }

    /// The main program (not installed).
    pub fn program(&self) -> crate::Result<crate::Program> {
        let data = &self.files.iter().find(|(p, _)| *p == self.main).ok_or(crate::AmosError::BadFormat)?.1;
        if data.starts_with(b"AMOS") {
            crate::Program::load(data)
        } else {
            crate::tokenise::tokenise_program(data).map_err(|_| crate::AmosError::BadFormat)
        }
    }

    /// Total size of the bundled data.
    pub fn data_size(&self) -> usize {
        self.files.iter().map(|(_, d)| d.len()).sum()
    }

    /// Mounts the files as the `Bundle:` volume, makes it the current
    /// directory and loads the main program.
    pub fn install(&self, fs: &mut crate::files::FileSystem) -> crate::Result<crate::Program> {
        fs.mount_memory(VOLUME, &["Work", "Df0", "Dh0", "Sys"]);
        for (path, data) in &self.files {
            // Create the directories, then the file.
            let parts: Vec<&str> = path.split('/').collect();
            for i in 1..parts.len() {
                let _ = fs.make_dir(&format!("{VOLUME}:{}", parts[..i].join("/")));
            }
            fs.write(&format!("{VOLUME}:{path}"), data).map_err(|e| crate::AmosError::Io(format!("{e:?}")))?;
        }
        let _ = fs.set_current_dir(&format!("{VOLUME}:"));
        let main = fs.read(&format!("{VOLUME}:{}", self.main)).map_err(|_| crate::AmosError::Io("missing program".into()))?;
        if main.starts_with(b"AMOS") {
            crate::Program::load(&main)
        } else {
            crate::tokenise::tokenise_program(&main).map_err(|_| crate::AmosError::BadFormat)
        }
    }
}

/// Page of a standalone web application: loads `app.amospak` and starts
/// it (its compiled module when it has one) with the runtime (`amos_app.js` / `amos_app_bg.wasm`). `{TITLE}` is
/// replaced by the program name.
pub const WEB_INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{TITLE}</title>
  <style>
    html, body { margin: 0; height: 100%; background: #000; overflow: hidden; }
    canvas#amos { display: block; width: 100vw; height: 100vh; outline: none; }
    #status { position: fixed; left: 8px; bottom: 8px; color: #889; font: 12px sans-serif; }
  </style>
</head>
<body>
  <canvas id="amos" tabindex="0"></canvas>
  <div id="status">Loading…</div>
  <script type="module">
    import init, { start_bundle } from "./amos_app.js";
    const status = document.getElementById("status");
    await init();
    const data = new Uint8Array(await (await fetch("app.amospak")).arrayBuffer());
    try {
      // Instantiates the compiled program (if the bundle has one), then starts.
      await start_bundle(data);
      status.remove();
      document.getElementById("amos").focus();
    } catch (e) {
      status.textContent = "Cannot start: " + e;
    }
  </script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_executable_trailer() {
        let b = Bundle {
            files: vec![("game.AMOS".into(), b"AMOS Pro101v\0\0\0\0\0\0\0\0".to_vec()), ("data/x.iff".into(), vec![1, 2, 3])],
            main: "game.AMOS".into(),
            module: None,
        };
        assert_eq!(Bundle::from_bytes(&b.to_bytes()).unwrap(), b);
        let exe = b.append_to_executable(b"\x7fELF fake executable");
        assert_eq!(Bundle::from_executable(&exe).unwrap(), b);
        assert!(Bundle::from_executable(b"plain executable").is_none());
        let mut fs = crate::files::FileSystem::new();
        let prg = b.install(&mut fs).unwrap();
        assert!(prg.source.is_empty());
        assert_eq!(fs.read("data/x.iff").unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn compiled_module_section() {
        let mut b = Bundle { files: vec![("p.txt".into(), b"Print 1".to_vec())], main: "p.txt".into(), module: None };
        let old = b.to_bytes();
        b.module = Some(b"\0asm\x01\0\0\0".to_vec());
        let new = b.to_bytes();
        assert_eq!(Bundle::from_bytes(&new).unwrap(), b);
        // Old bundles have no module; the section only follows the old data.
        assert_eq!(Bundle::from_bytes(&old).unwrap().module, None);
        assert!(new.starts_with(&old));
        let exe = b.append_to_executable(b"exe");
        assert_eq!(Bundle::from_executable(&exe).unwrap(), b);
        assert!(b.program().is_ok());
    }
}
