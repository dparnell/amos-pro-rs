//! Disc access: AMOS (AmigaDOS style) paths, a virtual file system and the
//! open file channels.
//!
//! AmigaDOS paths look like `Volume:dir/file`, `/file` (parent directory)
//! or `file` (current directory) and are case insensitive. Volumes are
//! mounted on host directories (native builds) or in-memory trees (always
//! available, used on the web and for `Ram:`).

use std::collections::BTreeMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

/// Where a volume's files live.
#[derive(Debug, Clone)]
pub enum Backend {
    /// A host directory.
    #[cfg(not(target_arch = "wasm32"))]
    Native(PathBuf),
    /// In-memory files, keyed by lower case path relative to the volume
    /// ("dir/file"). Directories are implied by their files, plus explicit
    /// empty directories.
    Memory(MemoryTree),
}

#[derive(Debug, Clone, Default)]
pub struct MemoryTree {
    /// lower case relative path -> (display name, data)
    pub files: BTreeMap<String, (String, Vec<u8>)>,
    /// lower case relative paths of directories -> display name
    pub dirs: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Volume {
    /// Name without the colon, as displayed ("Ram Disk", "AMOSPro_System").
    pub name: String,
    /// Other names accepted for this volume ("Ram", "Df0").
    pub aliases: Vec<String>,
    pub backend: Backend,
}

/// A directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    DirNotFound,
    BadName,
    AlreadyExists,
    NotEmpty,
    WriteProtected,
    Io(String),
}

impl FsError {
    /// AMOS error number.
    pub fn amos_error(&self) -> u16 {
        match self {
            FsError::NotFound => 81,
            FsError::DirNotFound => 80,
            FsError::BadName => 82,
            FsError::AlreadyExists => 79,
            FsError::NotEmpty => 85,
            FsError::WriteProtected => 84,
            FsError::Io(_) => 94,
        }
    }
}

pub type FsResult<T> = Result<T, FsError>;

/// An open channel (Open In / Out / Random / Append).
#[derive(Debug, Clone)]
pub struct Channel {
    pub path: String,
    pub mode: ChannelMode,
    pub data: Vec<u8>,
    pub pos: usize,
    pub dirty: bool,
    /// Random access: Field definitions (length, variable slot info is kept
    /// by the caller).
    pub record_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    In,
    Out,
    Append,
    Random,
}

#[derive(Debug, Default)]
pub struct FileSystem {
    pub volumes: Vec<Volume>,
    /// Current directory as a full AmigaDOS path ("Volume:dir").
    pub current_dir: String,
    /// Open channels 1..10.
    pub channels: BTreeMap<u32, Channel>,
    /// State of Dir First$ / Dir Next$.
    pub dir_listing: Vec<String>,
    pub dir_listing_pos: usize,
    /// Directory filter (`Dir$`/`Set Dir`).
    pub dir_filter: String,
    /// Host directory used as the current AMOS directory (native builds).
    #[cfg(not(target_arch = "wasm32"))]
    pub native_root: Option<PathBuf>,
}

/// Splits "Vol:dir/file" into ("Vol", "dir/file"); no volume gives None.
fn split_volume(path: &str) -> (Option<&str>, &str) {
    match path.find(':') {
        Some(i) => (Some(&path[..i]), &path[i + 1..]),
        None => (None, path),
    }
}

/// Normalises an AmigaDOS relative path: removes empty parts and applies
/// leading '/' (parent). Returns the list of components.
fn normalise(base: &[String], rel: &str) -> Vec<String> {
    let mut parts: Vec<String> = base.to_vec();
    let mut rest = rel;
    // Each leading '/' goes up one level.
    while let Some(r) = rest.strip_prefix('/') {
        parts.pop();
        rest = r;
    }
    let comps: Vec<&str> = rest.split('/').collect();
    for (i, c) in comps.iter().enumerate() {
        if c.is_empty() {
            // "a//b": the second '/' means parent.
            if i > 0 && i < comps.len() - 1 {
                parts.pop();
            }
            continue;
        }
        parts.push(c.to_string());
    }
    parts
}

impl FileSystem {
    pub fn new() -> Self {
        let mut fs = FileSystem::default();
        fs.mount_memory("Ram Disk", &["Ram"]);
        fs.current_dir = "Ram Disk:".to_string();
        fs
    }

    pub fn mount_memory(&mut self, name: &str, aliases: &[&str]) {
        self.unmount(name);
        self.volumes.push(Volume {
            name: name.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            backend: Backend::Memory(MemoryTree::default()),
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn mount_native(&mut self, name: &str, aliases: &[&str], dir: &Path) {
        self.unmount(name);
        self.volumes.push(Volume {
            name: name.to_string(),
            aliases: aliases.iter().map(|s| s.to_string()).collect(),
            backend: Backend::Native(dir.to_path_buf()),
        });
    }

    pub fn unmount(&mut self, name: &str) {
        self.volumes.retain(|v| !v.name.eq_ignore_ascii_case(name));
    }

    /// Uses a host directory as the current directory (volume "Work").
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_native_root(&mut self, dir: &Path) {
        if self.volumes.is_empty() {
            *self = FileSystem::new();
        }
        self.native_root = Some(dir.to_path_buf());
        self.mount_native("Work", &["Dh0", "Df0", "Sys"], dir);
        self.current_dir = "Work:".to_string();
        // AMOS distribution volumes, when the AMOS directory is found.
        let mut d = Some(dir);
        while let Some(p) = d {
            let amos = p.join("AMOS-Professional-365").join("AMOS");
            let alt = p.join("AMOS");
            let found = if amos.join("APSystem").is_dir() {
                Some(amos)
            } else if alt.join("APSystem").is_dir() {
                Some(alt)
            } else if p.join("APSystem").is_dir() {
                Some(p.to_path_buf())
            } else {
                None
            };
            if let Some(root) = found {
                self.mount_amos_distribution(&root);
                break;
            }
            d = p.parent();
        }
    }

    /// Mounts the standard AMOS Professional volumes on the folders of an
    /// AMOS distribution directory.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn mount_amos_distribution(&mut self, root: &Path) {
        let vols = [
            ("AMOSPro_System", "APSystem"),
            ("AMOSPro_Accessories", "Accessories"),
            ("AMOSPro_Tutorial", "Tutorial"),
            ("AMOSPro_Examples", "Examples"),
            ("AMOSPro_Productivity1", "Productivity1"),
            ("AMOSPro_Productivity2", "Productivity2"),
            ("AMOSPro_Compiler", "Compiler"),
        ];
        for (vol, dir) in vols {
            let p = root.join(dir);
            if p.is_dir() {
                self.mount_native(vol, &[], &p);
            }
        }
        self.mount_native("AMOSPro", &[], root);
    }

    fn volume_index(&self, name: &str) -> Option<usize> {
        self.volumes.iter().position(|v| {
            v.name.eq_ignore_ascii_case(name) || v.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
        })
    }

    /// Resolves an AMOS path to (volume index, components).
    pub fn resolve(&self, path: &str) -> FsResult<(usize, Vec<String>)> {
        let (vol, rel) = split_volume(path);
        match vol {
            Some(v) => {
                let i = self.volume_index(v).ok_or(FsError::DirNotFound)?;
                Ok((i, normalise(&[], rel)))
            }
            None => {
                let (cv, crel) = split_volume(&self.current_dir);
                let i = self.volume_index(cv.unwrap_or("")).ok_or(FsError::DirNotFound)?;
                let base = normalise(&[], crel);
                Ok((i, normalise(&base, rel)))
            }
        }
    }

    /// Full AmigaDOS path of a resolved location.
    fn display_path(&self, vol: usize, comps: &[String]) -> String {
        format!("{}:{}", self.volumes[vol].name, comps.join("/"))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn native_path(dir: &Path, comps: &[String]) -> FsResult<PathBuf> {
        // Case insensitive lookup of each component.
        let mut p = dir.to_path_buf();
        for c in comps {
            let exact = p.join(c);
            if exact.exists() {
                p = exact;
                continue;
            }
            let found = std::fs::read_dir(&p)
                .ok()
                .and_then(|rd| {
                    rd.filter_map(|e| e.ok())
                        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(c))
                        .map(|e| e.path())
                });
            p = found.unwrap_or(exact);
        }
        Ok(p)
    }

    pub fn read(&self, path: &str) -> FsResult<Vec<u8>> {
        let (v, comps) = self.resolve(path)?;
        if comps.is_empty() {
            return Err(FsError::BadName);
        }
        match &self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => {
                let p = Self::native_path(dir, &comps)?;
                if p.is_dir() {
                    return Err(FsError::NotFound);
                }
                std::fs::read(&p).map_err(|_| FsError::NotFound)
            }
            Backend::Memory(t) => {
                t.files.get(&comps.join("/").to_lowercase()).map(|(_, d)| d.clone()).ok_or(FsError::NotFound)
            }
        }
    }

    pub fn write(&mut self, path: &str, data: &[u8]) -> FsResult<()> {
        let (v, comps) = self.resolve(path)?;
        if comps.is_empty() {
            return Err(FsError::BadName);
        }
        match &mut self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => {
                let p = Self::native_path(dir, &comps)?;
                std::fs::write(&p, data).map_err(|e| FsError::Io(e.to_string()))
            }
            Backend::Memory(t) => {
                let key = comps.join("/").to_lowercase();
                let name = comps.last().unwrap().clone();
                t.files.insert(key, (name, data.to_vec()));
                Ok(())
            }
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        let Ok((v, comps)) = self.resolve(path) else { return false };
        match &self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => Self::native_path(dir, &comps).is_ok_and(|p| p.exists()),
            Backend::Memory(t) => {
                let key = comps.join("/").to_lowercase();
                key.is_empty() || t.files.contains_key(&key) || t.dirs.contains_key(&key)
            }
        }
    }

    pub fn is_dir(&self, path: &str) -> bool {
        let Ok((v, comps)) = self.resolve(path) else { return false };
        match &self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => Self::native_path(dir, &comps).is_ok_and(|p| p.is_dir()),
            Backend::Memory(t) => {
                let key = comps.join("/").to_lowercase();
                key.is_empty()
                    || t.dirs.contains_key(&key)
                    || t.files.keys().any(|k| k.starts_with(&format!("{key}/")))
            }
        }
    }

    pub fn delete(&mut self, path: &str) -> FsResult<()> {
        let (v, comps) = self.resolve(path)?;
        match &mut self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => {
                let p = Self::native_path(dir, &comps)?;
                if p.is_dir() {
                    std::fs::remove_dir(&p).map_err(|_| FsError::NotEmpty)
                } else {
                    std::fs::remove_file(&p).map_err(|_| FsError::NotFound)
                }
            }
            Backend::Memory(t) => {
                let key = comps.join("/").to_lowercase();
                if t.files.remove(&key).is_some() {
                    return Ok(());
                }
                if t.files.keys().any(|k| k.starts_with(&format!("{key}/"))) {
                    return Err(FsError::NotEmpty);
                }
                t.dirs.remove(&key).map(|_| ()).ok_or(FsError::NotFound)
            }
        }
    }

    pub fn rename(&mut self, from: &str, to: &str) -> FsResult<()> {
        if self.exists(to) {
            return Err(FsError::AlreadyExists);
        }
        let data = self.read(from)?;
        self.write(to, &data)?;
        self.delete(from)
    }

    pub fn make_dir(&mut self, path: &str) -> FsResult<()> {
        let (v, comps) = self.resolve(path)?;
        match &mut self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => {
                let p = Self::native_path(dir, &comps)?;
                std::fs::create_dir(&p).map_err(|_| FsError::AlreadyExists)
            }
            Backend::Memory(t) => {
                let key = comps.join("/").to_lowercase();
                t.dirs.insert(key, comps.last().cloned().unwrap_or_default());
                Ok(())
            }
        }
    }

    /// Lists a directory (path may end with a pattern such as `*.AMOS`).
    pub fn list(&self, path: &str) -> FsResult<Vec<DirEntry>> {
        let (dir_path, pattern) = split_pattern(path);
        let (v, comps) = self.resolve(&dir_path)?;
        let mut out = Vec::new();
        match &self.volumes[v].backend {
            #[cfg(not(target_arch = "wasm32"))]
            Backend::Native(dir) => {
                let p = Self::native_path(dir, &comps)?;
                let rd = std::fs::read_dir(&p).map_err(|_| FsError::DirNotFound)?;
                for e in rd.filter_map(|e| e.ok()) {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let md = e.metadata().ok();
                    out.push(DirEntry {
                        name,
                        is_dir: md.as_ref().is_some_and(|m| m.is_dir()),
                        size: md.map_or(0, |m| m.len()),
                    });
                }
            }
            Backend::Memory(t) => {
                let prefix = comps.join("/").to_lowercase();
                let prefix = if prefix.is_empty() { prefix } else { format!("{prefix}/") };
                let mut seen = std::collections::BTreeSet::new();
                for (k, (name, data)) in &t.files {
                    if let Some(rest) = k.strip_prefix(&prefix) {
                        match rest.find('/') {
                            None => out.push(DirEntry { name: name.clone(), is_dir: false, size: data.len() as u64 }),
                            Some(i) => {
                                if seen.insert(rest[..i].to_string()) {
                                    out.push(DirEntry { name: rest[..i].to_string(), is_dir: true, size: 0 });
                                }
                            }
                        }
                    }
                }
                for (k, name) in &t.dirs {
                    if let Some(rest) = k.strip_prefix(&prefix)
                        && !rest.contains('/')
                        && !rest.is_empty()
                        && seen.insert(rest.to_string())
                    {
                        out.push(DirEntry { name: name.clone(), is_dir: true, size: 0 });
                    }
                }
            }
        }
        if let Some(pat) = pattern {
            out.retain(|e| e.is_dir || wildcard_match(&pat, &e.name));
        }
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    }

    /// Adds a file of the AMOS distribution (path relative to the `AMOS`
    /// folder, e.g. "Examples/Examples/H-1/Help_1.AMOS") to in-memory
    /// volumes: "AMOSPro:" and the matching "AMOSPro_Examples:"... volume.
    /// Used where there is no host file system (web).
    pub fn add_distribution_file(&mut self, rel: &str, data: &[u8]) {
        let rel = rel.trim_start_matches('/');
        let volumes = [
            ("APSystem", "AMOSPro_System"),
            ("Accessories", "AMOSPro_Accessories"),
            ("Tutorial", "AMOSPro_Tutorial"),
            ("Examples", "AMOSPro_Examples"),
            ("Productivity1", "AMOSPro_Productivity1"),
            ("Productivity2", "AMOSPro_Productivity2"),
            ("Compiler", "AMOSPro_Compiler"),
        ];
        let mut targets = vec![("AMOSPro".to_string(), rel.to_string())];
        if let Some((first, rest)) = rel.split_once('/')
            && let Some((_, vol)) = volumes.iter().find(|(d, _)| d.eq_ignore_ascii_case(first))
        {
            targets.push((vol.to_string(), rest.to_string()));
        }
        for (vol, path) in targets {
            if self.volume_index(&vol).is_none() {
                self.mount_memory(&vol, &[]);
            }
            let _ = self.write(&format!("{vol}:{path}"), data);
        }
    }

    /// Changes the current directory (`Dir$=`).
    pub fn set_current_dir(&mut self, path: &str) -> FsResult<()> {
        let (v, comps) = self.resolve(path)?;
        let full = self.display_path(v, &comps);
        if !self.is_dir(&full) {
            return Err(FsError::DirNotFound);
        }
        self.current_dir = full;
        Ok(())
    }

    /// `Parent`: current directory goes up one level.
    pub fn parent(&mut self) {
        if let Ok((v, mut comps)) = self.resolve("") {
            comps.pop();
            self.current_dir = self.display_path(v, &comps);
        }
    }

    /// Names of the mounted volumes (for `Drive`, file selectors).
    pub fn volume_names(&self) -> Vec<String> {
        self.volumes.iter().map(|v| v.name.clone()).collect()
    }
}

/// Splits "dir/*.abk" into ("dir", Some("*.abk")) when the last component
/// holds wildcards (`*`, `?`, `#?`).
pub fn split_pattern(path: &str) -> (String, Option<String>) {
    let last_sep = path.rfind(['/', ':']).map_or(0, |i| i + 1);
    let last = &path[last_sep..];
    if last.contains('*') || last.contains('?') {
        let dir = path[..last_sep].trim_end_matches('/').to_string();
        let dir = if dir.is_empty() && path[..last_sep].ends_with(':') { path[..last_sep].to_string() } else { dir };
        (dir, Some(last.to_string()))
    } else {
        (path.to_string(), None)
    }
}

/// Case insensitive wildcard match (`*` any, `?` one char, AmigaDOS `#?`).
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pat: Vec<char> = pattern.replace("#?", "*").to_lowercase().chars().collect();
    let s: Vec<char> = name.to_lowercase().chars().collect();
    fn m(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') => (0..=s.len()).any(|i| m(&p[1..], &s[i..])),
            Some('?') => !s.is_empty() && m(&p[1..], &s[1..]),
            Some(c) => s.first() == Some(c) && m(&p[1..], &s[1..]),
        }
    }
    m(&pat, &s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_volume() {
        let mut fs = FileSystem::new();
        fs.write("Ram:test.txt", b"hello").unwrap();
        assert_eq!(fs.read("ram:TEST.TXT").unwrap(), b"hello");
        assert_eq!(fs.read("test.txt").unwrap(), b"hello");
        fs.make_dir("Ram:sub").unwrap();
        fs.write("Ram:sub/a.abk", b"x").unwrap();
        fs.set_current_dir("Ram:sub").unwrap();
        assert_eq!(fs.read("a.abk").unwrap(), b"x");
        assert_eq!(fs.read("/test.txt").unwrap(), b"hello");
        let l = fs.list("Ram:*.txt").unwrap();
        assert_eq!(l.len(), 2); // sub (dir) + test.txt
        assert!(wildcard_match("#?.AMOS", "Game.amos"));
    }
}
