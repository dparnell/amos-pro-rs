//! AMOS Professional for modern platforms: window, input, rendering and
//! sound glue around the platform independent `amos-core` runtime.

mod audio;
mod keymap;
mod renderer;

use std::sync::Arc;

use amos_core::Machine;
use amos_core::editor::Editor;
use amos_core::display::{DISPLAY_HEIGHT, DISPLAY_WIDTH};
use amos_core::input::{InputEvent, MouseButton};
use amos_core::machine::VBL_HZ;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use audio::AudioOutput;
use renderer::Renderer;

/// Events sent to the event loop from asynchronous tasks.
enum UserEvent {
    RendererReady(Result<Renderer, String>),
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    machine: Machine,
    /// The AMOS Professional editor; it runs the programs.
    editor: Option<Editor>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    audio: Option<AudioOutput>,
    audio_failed: bool,
    last_time: Option<Instant>,
    vbl_accumulator: f64,
}

impl App {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let (machine, editor) = new_machine();
        Self {
            proxy,
            machine,
            editor,
            window: None,
            renderer: None,
            audio: None,
            audio_failed: false,
            last_time: None,
            vbl_accumulator: 0.0,
        }
    }

    /// Browsers refuse to start audio before a user gesture, so the stream is
    /// opened lazily on the first input event (on all platforms, for symmetry).
    fn ensure_audio(&mut self) {
        if self.audio.is_none() && !self.audio_failed {
            match AudioOutput::start(self.machine.mixer()) {
                Ok(a) => self.audio = Some(a),
                Err(e) => {
                    log::warn!("Audio disabled: {e}");
                    self.audio_failed = true;
                }
            }
        }
    }

    /// Run as many 50 Hz vertical blanks as real time requires.
    fn advance(&mut self) {
        let now = Instant::now();
        let last = self.last_time.replace(now).unwrap_or(now);
        let elapsed = (now - last).as_secs_f64().min(0.25);
        self.vbl_accumulator += elapsed * VBL_HZ;
        while self.vbl_accumulator >= 1.0 {
            self.vbl_accumulator -= 1.0;
            match &mut self.editor {
                Some(ed) => ed.vbl(&mut self.machine),
                None => self.machine.vbl(),
            }
            for line in self.machine.hw.log.drain(..) {
                log::info!("{line}");
            }
        }
    }

    fn window_to_display(&self, x: f64, y: f64) -> (f32, f32) {
        match &self.renderer {
            Some(r) => {
                let [rx, ry, rw, rh] = r.display_rect();
                (
                    (x as f32 - rx) * DISPLAY_WIDTH as f32 / rw,
                    (y as f32 - ry) * DISPLAY_HEIGHT as f32 / rh,
                )
            }
            None => (x as f32, y as f32),
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[allow(unused_mut)]
        let mut attributes = Window::default_attributes()
            .with_title("AMOS Professional")
            .with_inner_size(winit::dpi::LogicalSize::new(DISPLAY_WIDTH as f64 * 1.5, DISPLAY_HEIGHT as f64 * 1.5));
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes = attributes.with_canvas(web::find_canvas()).with_append(true).with_focusable(true);
        }
        let window = Arc::new(event_loop.create_window(attributes).expect("failed to create window"));
        // AMOS draws its own mouse pointer (hardware sprite 0).
        window.set_cursor_visible(false);
        self.window = Some(window.clone());

        let proxy = self.proxy.clone();
        let init = async move {
            let result = Renderer::new(window).await;
            let _ = proxy.send_event(UserEvent::RendererReady(result));
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(init);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(init);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::RendererReady(Ok(renderer)) => {
                self.renderer = Some(renderer);
                if let Some(w) = &self.window {
                    let size = w.inner_size();
                    self.renderer.as_mut().unwrap().resize(size.width, size.height);
                    w.request_redraw();
                }
            }
            UserEvent::RendererReady(Err(e)) => {
                log::error!("Could not initialise graphics: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                self.advance();
                if self.editor.as_ref().is_some_and(|e| e.quit_requested) {
                    event_loop.exit();
                    return;
                }
                if let Some(r) = &mut self.renderer {
                    r.render(&self.machine.frame());
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.ensure_audio();
                let pressed = event.state == ElementState::Pressed;
                let ch = if pressed { event.text.as_ref().and_then(|t| t.chars().next()) } else { None };
                match event.physical_key {
                    PhysicalKey::Code(code) if keymap::amiga_scancode(code).is_some() => {
                        let scancode = keymap::amiga_scancode(code).unwrap();
                        self.machine.input(InputEvent::Key { scancode, pressed, ch });
                    }
                    _ => {
                        if let Some(c) = ch {
                            self.machine.input(InputEvent::Char(c));
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = self.window_to_display(position.x, position.y);
                self.machine.input(InputEvent::MouseMove { x, y });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.ensure_audio();
                let button = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    _ => return,
                };
                self.machine.input(InputEvent::MouseButton { button, pressed: state == ElementState::Pressed });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 20.0) as f32,
                };
                self.machine.input(InputEvent::MouseWheel { delta });
            }
            _ => {}
        }
    }
}

/// Creates the machine and the editor (`+B.s:47-79`): the editor starts
/// with the program given on the command line (native) or by the page
/// (web), which is run at once; without a program the editor shows an
/// empty one.
fn new_machine() -> (Machine, Option<Editor>) {
    #[allow(unused_mut)]
    let mut m = Machine::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let arg = std::env::args().nth(1).map(std::path::PathBuf::from);
        // The program's folder (or the current one) is the current AMOS
        // directory; the AMOS distribution is searched from there, then from
        // the executable's folder.
        let dir = arg
            .as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| ".".into());
        m.hw.files.set_native_root(&dir);
        if !m.hw.files.volume_names().iter().any(|v| v.eq_ignore_ascii_case("AMOSPro_System"))
            && let Some(root) = find_distribution_from_exe()
        {
            m.hw.files.mount_amos_distribution(&root);
        }
        let mut ed = Editor::new(&mut m);
        if let Some(path) = arg {
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match ed.load(&mut m, &format!("Work:{name}")) {
                Ok(()) => ed.run(&mut m),
                Err(e) => {
                    log::error!("{}: {e}", path.display());
                    ed.alert(e);
                }
            }
        }
        (m, Some(ed))
    }
    #[cfg(target_arch = "wasm32")]
    {
        for (path, data) in web::take_files() {
            m.hw.files.add_distribution_file(&path, &data);
        }
        let _ = m.hw.files.set_current_dir("AMOSPro:");
        let mut ed = Editor::new(&mut m);
        if let Some(path) = web::run_path() {
            match ed.load(&mut m, &path) {
                Ok(()) => ed.run(&mut m),
                Err(e) => {
                    log::error!("cannot load {path}: {e}");
                    ed.alert(e);
                }
            }
        }
        (m, Some(ed))
    }
}

/// The AMOS distribution folder near the executable (for an installed
/// build started from elsewhere).
#[cfg(not(target_arch = "wasm32"))]
fn find_distribution_from_exe() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut d = exe.parent();
    while let Some(p) = d {
        for cand in [p.join("AMOS-Professional-365").join("AMOS"), p.join("AMOS")] {
            if cand.join("APSystem").is_dir() {
                return Some(cand);
            }
        }
        d = p.parent();
    }
    None
}

/// Start AMOS Professional (native entry point; on the web see [`web::start`]).
pub fn run() {
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("failed to create event loop");
    let app = App::new(event_loop.create_proxy());

    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        event_loop.run_app(&mut app).expect("event loop failed");
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    /// Use an existing `<canvas id="amos">` if the page provides one.
    pub fn find_canvas() -> Option<web_sys::HtmlCanvasElement> {
        web_sys::window()?
            .document()?
            .get_element_by_id("amos")?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .ok()
    }

    use std::cell::RefCell;

    thread_local! {
        static FILES: RefCell<Vec<(String, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
        static RUN: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// Adds a file of the AMOS distribution (path relative to the AMOS
    /// folder) before `start` is called.
    #[wasm_bindgen]
    pub fn add_file(path: String, data: Vec<u8>) {
        FILES.with(|f| f.borrow_mut().push((path, data)));
    }

    pub(super) fn take_files() -> Vec<(String, Vec<u8>)> {
        FILES.with(|f| std::mem::take(&mut *f.borrow_mut()))
    }

    pub(super) fn run_path() -> Option<String> {
        RUN.with(|r| r.borrow().clone())
    }

    /// Starts AMOS; `run` is an AMOS path of a program to run
    /// (e.g. "AMOSPro_Examples:Examples/H-1/Help_16.AMOS").
    #[wasm_bindgen]
    pub fn start(run: Option<String>) {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Info);
        RUN.with(|r| *r.borrow_mut() = run);
        super::run();
    }
}
