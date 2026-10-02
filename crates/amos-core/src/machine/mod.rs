//! The complete AMOS machine: interpreter + display + sound + input.
//!
//! [`Machine`] is what the platform layer drives: it calls [`Machine::vbl`]
//! 50 times per second, renders [`Machine::frame`] and forwards input.
//!
//! [`Hardware`] implements the interpreter's [`Host`] interface. Each
//! subsystem adds its instructions in its own file through methods named
//! `<subsystem>_instruction` / `<subsystem>_function`, called in turn by the
//! dispatcher below; they return `Ok(false)` / `Ok(None)` for keywords they
//! do not handle.

mod dispatch;
pub mod frame;
pub mod inst_banks;
pub mod inst_copper;
pub mod inst_dialogs;
pub mod inst_draw;
pub mod inst_files;
pub mod inst_input;
pub mod inst_menus;
pub mod inst_screen;
pub mod inst_sound;
pub mod inst_sprites;
pub mod inst_system;
pub mod inst_text;

use std::sync::{Arc, Mutex};

use crate::audio::{Mixer, SoundState};
use crate::banks::BankSet;
use crate::display::Frame;
use crate::files::FileSystem;
use crate::gfx::Screens;
use crate::gfx::sprites::SpriteState;
use crate::input::{InputEvent, InputState};
use crate::interp::{Interp, RunState};
use crate::program::Program;

/// Vertical blank frequency of a PAL Amiga.
pub const VBL_HZ: f64 = 50.0;

/// Everything except the interpreter.
pub struct Hardware {
    pub screens: Screens,
    pub sprites: SpriteState,
    pub sound: SoundState,
    pub mixer: Arc<Mutex<Mixer>>,
    pub input: InputState,
    pub banks: BankSet,
    pub files: FileSystem,
    pub frame: frame::FrameCache,
    /// Number of vertical blanks since start (`T_VBLCount`).
    pub vbl_count: u64,
    /// `Timer` (`T_VBLTimer`), writable by the program.
    pub timer: i32,
    /// Messages for the console / editor (errors, debug).
    pub log: Vec<String>,
    /// Address and data registers for Call / Execall (`Areg`, `Dreg`).
    pub areg: [i32; 8],
    pub dreg: [i32; 8],
    /// Field definitions of random access channels.
    pub fields: std::collections::HashMap<u32, inst_files::Fields>,
    /// End of line characters for Input # (`Set Input`).
    pub input_separators: (i32, i32),
    /// Program to start after the current one stopped (`Run "file"`).
    pub pending_run: Option<Vec<u8>>,
    pub menus: crate::menus::Menus,
    /// Variables given an address with Varptr / Array.
    pub var_maps: Vec<inst_banks::VarMap>,
    /// `Command Line$`: text passed by the program that ran this one.
    pub command_line: Vec<u8>,
    /// Request On/Off/Wb setting (1, 0, 2).
    pub system_requests: u8,
    /// Blocks, scroll zones, font list (shared by all screens).
    pub draw: crate::gfx::blocks::DrawGlobals,
    /// Last key checked against menu shortcuts.
    pub menu_key_serial: u64,
    /// The menu bar while it is open.
    pub menu_session: Option<Box<inst_menus::MenuSession>>,
    /// User copper list state (Copper Off mode).
    pub copper: crate::gfx::copper::Copper,
    /// Interface dialogs, resource bank, file selector.
    pub dialogs: crate::interface::DialogState,
}

impl Hardware {
    pub fn new() -> Self {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let mut hw = Hardware {
            screens: Screens::new(),
            sprites: SpriteState::default(),
            sound: SoundState::new(mixer.clone()),
            mixer,
            input: InputState::default(),
            banks: BankSet::default(),
            files: FileSystem::new(),
            frame: frame::FrameCache::default(),
            vbl_count: 0,
            timer: 0,
            log: Vec::new(),
            areg: [0; 8],
            dreg: [0; 8],
            fields: Default::default(),
            input_separators: (10, -1),
            pending_run: None,
            menus: Default::default(),
            draw: Default::default(),
            command_line: Vec::new(),
            var_maps: Vec::new(),
            system_requests: 1,
            menu_key_serial: 0,
            menu_session: None,
            copper: Default::default(),
            dialogs: Default::default(),
        };
        hw.reset();
        hw
    }

    /// `Default`: state at the start of a program (default screen open).
    pub fn reset(&mut self) {
        self.screen_reset();
        self.draw_reset();
        self.sprites_reset();
        self.sound_reset();
        // Menus and user copper lists belong to the program that defined them.
        self.menus = Default::default();
        self.menu_session = None;
        self.copper = Default::default();
        self.dialogs_reset();
        self.var_maps.clear();
    }

    /// Work done at each vertical blank (the VBL interrupt).
    pub fn vbl(&mut self) {
        self.vbl_count += 1;
        self.timer = self.timer.wrapping_add(1);
        self.input_vbl();
        self.screen_vbl();
        self.sprites_vbl();
        self.sound_vbl();
    }
}

impl Default for Hardware {
    fn default() -> Self {
        Self::new()
    }
}

/// The AMOS computer.
pub struct Machine {
    pub interp: Interp,
    pub hw: Hardware,
    /// Maximum instructions executed per vertical blank (time slicing).
    pub instructions_per_frame: usize,
    pub state: RunState,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Machine { interp: Interp::new(), hw: Hardware::new(), instructions_per_frame: 200_000, state: RunState::Idle }
    }

    pub fn mixer(&self) -> Arc<Mutex<Mixer>> {
        self.hw.mixer.clone()
    }

    /// Loads and starts a program. Its banks replace the current ones.
    pub fn run_program(&mut self, program: &Program) -> Result<(), crate::interp::verify::TestError> {
        self.interp.load(program)?;
        self.hw.reset();
        self.hw.banks.load_program_banks(&program.banks);
        self.hw.on_banks_changed();
        self.state = RunState::Running;
        Ok(())
    }

    /// Runs a direct mode line (tokenised) in the context of the last
    /// program run, keeping its screens and banks (`Esc_R`, +Edit.s).
    pub fn run_direct(&mut self, line: &[u8]) -> Result<(), crate::interp::verify::TestError> {
        self.interp.run_direct(line)?;
        self.state = RunState::Running;
        Ok(())
    }

    pub fn input(&mut self, event: InputEvent) {
        self.hw.input_event(event);
    }

    /// Advances the machine by one vertical blank (1/50 s): interrupt work,
    /// then the program runs until it waits or its time slice is used.
    pub fn vbl(&mut self) {
        self.hw.vbl();
        self.interp.vbl();
        if self.interp.running {
            self.state = self.interp.run(&mut self.hw, self.instructions_per_frame);
            if let RunState::Stopped(info) = &self.state {
                if let Some(path) = self.hw.pending_run.take() {
                    // Run "file": chain to another program.
                    match self.hw.files_read_all(&path).ok().and_then(|d| Program::load(&d).ok()) {
                        Some(prg) => {
                            if let Err(e) = self.run_program(&prg) {
                                self.hw.log.push(crate::errors::test_message(e.code).to_string());
                            }
                            return;
                        }
                        None => self.hw.log.push(crate::errors::message(81).to_string()),
                    }
                    return;
                }
                let msg = crate::machine::dispatch::describe_stop(&self.interp, info);
                self.hw.log.push(msg);
            }
        }
    }

    pub fn frame(&mut self) -> Frame<'_> {
        self.hw.build_frame()
    }
}
