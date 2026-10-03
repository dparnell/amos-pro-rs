//! Performance and output-stability harness for the runtime (interpreter,
//! graphics, frame building, audio, editor).
//!
//! ```text
//! cargo run --release -p amos-core --example perf -- [OPTIONS] [WORKLOADS...]
//!   WORKLOADS   `examples` (every AMOS-Professional-365/AMOS program, run on a
//!               temporary copy), `stress` (examples/perf/*.txt), `editor`,
//!               or .AMOS / .txt files. Default: examples stress editor.
//!   --frames N  frames per program (default 200)
//!   --filter S  only programs whose path contains S
//!   --out FILE  write the checksums (display, audio, log) of every program
//!   --check FILE compare the checksums with a file written by --out
//!   --no-hash   do not render/hash the display and audio (pure timing)
//!   --no-print-log  do not copy printed text to the log (as the app does)
//!   --top N     list the N slowest programs (default 15)
//!   --repeat N  run everything N times, keep the fastest time of each
//!               program (less noise on a busy machine)
//! ```
//!
//! For each program it reports the time spent in `Machine::vbl` (interrupt
//! work + interpreter), building the frame (`Machine::frame`), mixing the
//! audio, and (for the checksums) the reference compositor `render_rgba`.
//! Display checksums hash `render_rgba` of every frame; audio checksums
//! hash the bits of 441 stereo samples at 22050 Hz rendered every frame.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use amos_core::Machine;
use amos_core::audio::AudioSource;
use amos_core::display::render_rgba;
use amos_core::editor::Editor;
use amos_core::input::InputEvent;

struct Opts {
    frames: usize,
    filter: Option<String>,
    hash: bool,
    top: usize,
    print_log: bool,
}

#[derive(Default, Clone, Copy)]
struct Times {
    vbl: Duration,
    frame: Duration,
    audio: Duration,
    render: Duration,
    frames: usize,
}

impl Times {
    fn add(&mut self, o: &Times) {
        self.vbl += o.vbl;
        self.frame += o.frame;
        self.audio += o.audio;
        self.render += o.render;
        self.frames += o.frames;
    }
    fn total(&self) -> Duration {
        self.vbl + self.frame + self.audio
    }
}

struct Hasher(u64);

impl Hasher {
    fn new() -> Self {
        Hasher(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, v: &[u8]) {
        // FNV-1a over 8 byte words (fast enough for 1.7 MB per frame).
        let mut h = self.0;
        let (words, rest) = v.as_chunks::<8>();
        for c in words {
            h ^= u64::from_le_bytes(*c);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for &b in rest {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 = h;
    }
}

struct Result {
    name: String,
    times: Times,
    sums: String,
}

fn walk(d: &Path, out: &mut Vec<PathBuf>) {
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

fn copy_dir(from: &Path, to: &Path) {
    let _ = std::fs::create_dir_all(to);
    let Ok(rd) = std::fs::read_dir(from) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            let _ = std::fs::copy(&p, &t);
        }
    }
}

fn load(path: &Path) -> Option<amos_core::Program> {
    let data = std::fs::read(path).ok()?;
    if data.starts_with(b"AMOS") {
        amos_core::Program::load(&data).ok()
    } else {
        match amos_core::tokenise::tokenise_program(&data) {
            Ok(p) => Some(p),
            Err((line, e)) => {
                eprintln!("{}: line {line}: {e:?}", path.display());
                None
            }
        }
    }
}

/// Input played to every program (same as the compiled/interpreted compare).
fn input(m: &mut Machine, frame: usize) {
    if frame % 40 == 20 {
        m.input(InputEvent::Char(' '));
    }
    if frame % 70 == 35 {
        m.input(InputEvent::Char('\r'));
    }
}

/// Runs a program in `dir` (a temporary copy) for `frames` frames.
fn run_program(name: &str, prg: &amos_core::Program, dir: &Path, o: &Opts) -> Result {
    let mut m = Machine::new();
    m.hw.log_print = o.print_log;
    m.hw.files.set_native_root(dir);
    let mut t = Times::default();
    let (mut hd, mut ha, mut hl) = (Hasher::new(), Hasher::new(), Hasher::new());
    let mut audio = vec![0f32; 441 * 2];
    if let Err(e) = m.run_program(prg) {
        return Result { name: name.to_string(), times: t, sums: format!("test-error {}", e.code) };
    }
    for f in 0..o.frames {
        input(&mut m, f);
        let a = Instant::now();
        m.vbl();
        let b = Instant::now();
        let frame = m.frame();
        let c = Instant::now();
        if o.hash {
            hd.bytes(&render_rgba(&frame));
        } else {
            std::hint::black_box(&frame);
        }
        drop(frame);
        let d = Instant::now();
        m.hw.mixer.lock().unwrap().render(&mut audio, 2, 22050);
        let e = Instant::now();
        if o.hash {
            for s in &audio {
                ha.bytes(&s.to_bits().to_le_bytes());
            }
        }
        for l in m.hw.log.drain(..) {
            hl.bytes(l.as_bytes());
            hl.bytes(b"\n");
        }
        t.vbl += b - a;
        t.frame += c - b;
        t.render += d - c;
        t.audio += e - d;
        t.frames += 1;
    }
    hl.bytes(format!("{:?}", m.state).as_bytes());
    Result { name: name.to_string(), times: t, sums: format!("{:016x} {:016x} {:016x}", hd.0, ha.0, hl.0) }
}

fn examples(o: &Opts, tmp: &Path, out: &mut Vec<Result>) {
    let mut files = Vec::new();
    walk(Path::new("AMOS-Professional-365/AMOS"), &mut files);
    files.sort();
    for f in files {
        let name = f.to_string_lossy().into_owned();
        if o.filter.as_ref().is_some_and(|s| !name.contains(s.as_str())) {
            continue;
        }
        let Some(prg) = load(&f) else { continue };
        // A fresh copy of the program's directory: programs may write files.
        let dir = tmp.join("run");
        let _ = std::fs::remove_dir_all(&dir);
        copy_dir(f.parent().unwrap(), &dir);
        out.push(run_program(&name, &prg, &dir, o));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn files(paths: &[PathBuf], o: &Opts, tmp: &Path, out: &mut Vec<Result>) {
    for f in paths {
        let name = f.to_string_lossy().into_owned();
        if o.filter.as_ref().is_some_and(|s| !name.contains(s.as_str())) {
            continue;
        }
        let Some(prg) = load(f) else { continue };
        let dir = tmp.join("run");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        out.push(run_program(&name, &prg, &dir, o));
    }
}

fn stress_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/perf");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// The editor: loads the largest example program, scrolls through it, types.
fn editor(o: &Opts, tmp: &Path, out: &mut Vec<Result>) {
    if o.filter.as_ref().is_some_and(|s| !"editor".contains(s.as_str())) {
        return;
    }
    let mut files = Vec::new();
    walk(Path::new("AMOS-Professional-365/AMOS"), &mut files);
    let Some(big) = files.iter().max_by_key(|f| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)) else { return };
    let dir = tmp.join("edit");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::copy(big, dir.join("big.AMOS"));
    let mut m = Machine::new();
    m.hw.files.set_native_root(&dir);
    let mut ed = Editor::new(&mut m);
    if let Err(e) = ed.load(&mut m, "Work:big.AMOS") {
        eprintln!("editor load: {e}");
        return;
    }
    let mut t = Times::default();
    let (mut hd, mut hl) = (Hasher::new(), Hasher::new());
    let key = |m: &mut Machine, raw: u8, ch: Option<char>| {
        m.input(InputEvent::Key { scancode: raw, pressed: true, ch });
        m.input(InputEvent::Key { scancode: raw, pressed: false, ch: None });
    };
    for f in 0..o.frames {
        // Scroll down for a while, type a line, scroll back up.
        match (f / 60) % 3 {
            0 => key(&mut m, 0x4D, None),
            1 => {
                let c = b"Print \"typing in the editor\" : A=A+1"[f % 36] as char;
                m.input(InputEvent::Char(c));
                if f % 36 == 35 {
                    key(&mut m, 0x44, Some('\r'));
                }
            }
            _ => key(&mut m, 0x4C, None),
        }
        let a = Instant::now();
        ed.vbl(&mut m);
        let b = Instant::now();
        let frame = m.frame();
        let c = Instant::now();
        if o.hash {
            hd.bytes(&render_rgba(&frame));
        }
        drop(frame);
        let d = Instant::now();
        for l in m.hw.log.drain(..) {
            hl.bytes(l.as_bytes());
        }
        t.vbl += b - a;
        t.frame += c - b;
        t.render += d - c;
        t.frames += 1;
    }
    out.push(Result { name: "editor".into(), times: t, sums: format!("{:016x} 0 {:016x}", hd.0, hl.0) });
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut o = Opts { frames: 200, filter: None, hash: true, top: 15, print_log: true };
    let mut repeat = 1;
    let (mut out_file, mut check_file) = (None, None);
    let mut work: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--frames" => o.frames = v.parse().expect("--frames N"),
            "--filter" => o.filter = Some(v),
            "--out" => out_file = Some(v),
            "--check" => check_file = Some(v),
            "--top" => o.top = v.parse().expect("--top N"),
            "--repeat" => repeat = v.parse().expect("--repeat N"),
            "--no-hash" => {
                o.hash = false;
                i += 1;
                continue;
            }
            "--no-print-log" => {
                o.print_log = false;
                i += 1;
                continue;
            }
            w => {
                work.push(w.to_string());
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    if work.is_empty() {
        work = vec!["examples".into(), "stress".into(), "editor".into()];
    }
    let tmp = std::env::temp_dir().join(format!("amos-perf-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let mut results: Vec<Result> = Vec::new();
    let start = Instant::now();
    for r in 0..repeat {
        let mut run = Vec::new();
        for w in &work {
            match w.as_str() {
                "examples" => examples(&o, &tmp, &mut run),
                "stress" => files(&stress_files(), &o, &tmp, &mut run),
                "editor" => editor(&o, &tmp, &mut run),
                f => files(&[PathBuf::from(f)], &o, &tmp, &mut run),
            }
        }
        if r == 0 {
            results = run;
            continue;
        }
        for (a, b) in results.iter_mut().zip(run) {
            if b.times.total() < a.times.total() {
                a.times = b.times;
            }
            if a.sums != b.sums {
                println!("NOT DETERMINISTIC: {}", a.name);
            }
        }
    }
    let wall = start.elapsed();
    let _ = std::fs::remove_dir_all(&tmp);

    // Totals per group.
    let mut groups: BTreeMap<&str, Times> = BTreeMap::new();
    for r in &results {
        let g = if r.name.starts_with("AMOS-Professional-365") {
            "examples"
        } else if r.name == "editor" {
            "editor"
        } else {
            "stress"
        };
        groups.entry(g).or_default().add(&r.times);
    }
    println!("{:<10} {:>7} {:>10} {:>10} {:>10} {:>10} {:>10}", "group", "frames", "vbl ms", "frame ms", "audio ms", "render ms", "us/frame");
    for (g, t) in &groups {
        println!(
            "{:<10} {:>7} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1}",
            g,
            t.frames,
            ms(t.vbl),
            ms(t.frame),
            ms(t.audio),
            ms(t.render),
            t.total().as_secs_f64() * 1e6 / t.frames.max(1) as f64
        );
    }
    println!("\nstress / editor (us per frame: vbl, frame, audio)");
    for r in results.iter().filter(|r| !r.name.starts_with("AMOS-Professional-365")) {
        let n = r.times.frames.max(1) as f64;
        println!(
            "  {:<28} {:>9.1} {:>8.1} {:>8.1}",
            Path::new(&r.name).file_name().map_or(r.name.clone(), |f| f.to_string_lossy().into_owned()),
            r.times.vbl.as_secs_f64() * 1e6 / n,
            r.times.frame.as_secs_f64() * 1e6 / n,
            r.times.audio.as_secs_f64() * 1e6 / n
        );
    }
    let mut slow: Vec<&Result> = results.iter().filter(|r| r.name.starts_with("AMOS-Professional-365")).collect();
    slow.sort_by_key(|r| std::cmp::Reverse(r.times.total()));
    if !slow.is_empty() {
        println!("\nslowest examples (ms total: vbl, frame, audio)");
        for r in slow.iter().take(o.top) {
            println!(
                "  {:<60} {:>8.1} {:>7.1} {:>7.1}",
                r.name.trim_start_matches("AMOS-Professional-365/AMOS/"),
                ms(r.times.vbl),
                ms(r.times.frame),
                ms(r.times.audio)
            );
        }
    }
    println!("\nwall {:.1} s", wall.as_secs_f64());

    if let Some(f) = out_file {
        let text: String = results.iter().map(|r| format!("{}\t{}\n", r.name, r.sums)).collect();
        std::fs::write(&f, text).expect("write --out");
        println!("checksums written to {f}");
    }
    if let Some(f) = check_file {
        let text = std::fs::read_to_string(&f).expect("read --check");
        let want: BTreeMap<&str, &str> = text.lines().filter_map(|l| l.split_once('\t')).collect();
        let mut bad = 0;
        for r in &results {
            match want.get(r.name.as_str()) {
                Some(w) if *w == r.sums => {}
                Some(w) => {
                    bad += 1;
                    println!("DIFFERS {}: {} (expected {})", r.name, r.sums, w);
                }
                None => println!("no reference for {}", r.name),
            }
        }
        println!("check: {} programs, {} differ", results.len(), bad);
        if bad > 0 {
            std::process::exit(1);
        }
    }
}
