//! Builds standalone applications from AMOS programs (the replacement for
//! the 68000 compiler): the program and its files are bundled with the
//! AMOS runtime as a native application and/or a web application. The
//! program is compiled to WebAssembly (`amos-compiler`) and the module is
//! stored in the bundle; the runtime interprets the program when the
//! bundle has no module (or one it cannot use).

use std::path::{Path, PathBuf};

use amos_core::bundle::{Bundle, WEB_INDEX_HTML};

/// Compiles the bundle's main program and stores the module in the bundle.
/// Returns a short description (instructions, how many are left to the
/// interpreter, size). On error the bundle is left without a module, so
/// the application interprets the program.
pub fn compile_bundle(bundle: &mut Bundle) -> Result<String, String> {
    bundle.module = None;
    let prg = bundle.program().map_err(|e| format!("cannot load {}: {e}", bundle.main))?;
    let o = amos_compiler::compile_full(&prg).map_err(|e| e.to_string())?;
    let msg = format!(
        "compiled {} instructions ({} run by the interpreter), {} KB of WebAssembly",
        o.instructions,
        o.interpreted.len(),
        o.wasm.len().div_ceil(1024)
    );
    bundle.module = Some(o.wasm);
    Ok(msg)
}

/// Native application: on macOS an `.app` bundle (the runtime keeps its
/// code signature, the program goes in `Contents/Resources`), elsewhere a
/// single executable with the bundle appended. `runtime` is the AMOS app
/// executable image (any bundle already appended to it is removed).
pub fn build_native(bundle: &Bundle, name: &str, out: &Path, runtime: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let exe = Bundle::strip_executable(runtime);
    if cfg!(target_os = "macos") {
        return mac_app(out, name, exe, bundle);
    }
    let dest = out.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&dest, bundle.append_to_executable(exe)).map_err(|e| e.to_string())?;
    make_executable(&dest)?;
    Ok(dest)
}

/// Web application folder: `index.html`, the runtime (`amos_app.js`,
/// `amos_app_bg.wasm` copied from `web_runtime`) and `app.amospak`.
pub fn build_web(bundle: &Bundle, name: &str, out: &Path, web_runtime: &Path) -> Result<PathBuf, String> {
    let dir = out.join(format!("{name}-web"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for f in ["amos_app.js", "amos_app_bg.wasm"] {
        std::fs::copy(web_runtime.join(f), dir.join(f))
            .map_err(|e| format!("{}: {e} (build it with scripts/build-web.sh)", web_runtime.join(f).display()))?;
    }
    std::fs::write(dir.join("app.amospak"), bundle.to_bytes()).map_err(|e| e.to_string())?;
    let title = name.replace('&', "&amp;").replace('<', "&lt;");
    std::fs::write(dir.join("index.html"), WEB_INDEX_HTML.replace("{TITLE}", &title)).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Finds the web runtime (`web/pkg` of the source tree, or `pkg` / `web`
/// next to the AMOS executable).
pub fn find_web_runtime() -> Option<PathBuf> {
    let mut cands = vec![PathBuf::from("web/pkg")];
    if let Ok(exe) = std::env::current_exe() {
        let mut d = exe.parent();
        while let Some(p) = d {
            cands.push(p.join("web").join("pkg"));
            cands.push(p.join("pkg"));
            d = p.parent();
        }
    }
    cands.into_iter().find(|c| c.join("amos_app_bg.wasm").is_file())
}

fn mac_app(out: &Path, name: &str, exe: &[u8], bundle: &Bundle) -> Result<PathBuf, String> {
    let app = out.join(format!("{name}.app"));
    let _ = std::fs::remove_dir_all(&app);
    let macos = app.join("Contents/MacOS");
    let res = app.join("Contents/Resources");
    std::fs::create_dir_all(&macos).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&res).map_err(|e| e.to_string())?;
    let bin = macos.join(name);
    std::fs::write(&bin, exe).map_err(|e| e.to_string())?;
    std::fs::write(res.join("app.amospak"), bundle.to_bytes()).map_err(|e| e.to_string())?;
    let id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{name}</string>
  <key>CFBundleDisplayName</key><string>{name}</string>
  <key>CFBundleExecutable</key><string>{name}</string>
  <key>CFBundleIdentifier</key><string>org.amos-rs.app.{id}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1.0</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    );
    std::fs::write(app.join("Contents/Info.plist"), plist).map_err(|e| e.to_string())?;
    make_executable(&bin)?;
    // Ad-hoc signature for the whole bundle (macOS refuses unsigned code).
    let _ = std::process::Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app).output();
    Ok(app)
}

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(p).map_err(|e| e.to_string())?.permissions();
    perm.set_mode(0o755);
    std::fs::set_permissions(p, perm).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundles_are_compiled() {
        let mut b = Bundle { files: vec![("p.txt".into(), b"For I=1 To 3 : Print I : Next".to_vec())], main: "p.txt".into(), module: None };
        let msg = compile_bundle(&mut b).unwrap();
        assert!(msg.contains("0 run by the interpreter"), "{msg}");
        assert!(b.module.as_ref().is_some_and(|m| m.starts_with(b"\0asm")));
        let mut bad = Bundle { files: vec![("p.txt".into(), b"A=\"x\"".to_vec())], main: "p.txt".into(), module: None };
        assert!(compile_bundle(&mut bad).is_err());
        assert!(bad.module.is_none());
    }
}
