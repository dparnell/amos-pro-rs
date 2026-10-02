//! Platform independent AMOS Professional runtime.

pub mod audio;
pub mod banks;
pub mod bundle;
pub mod compiled;
pub mod detok;
pub mod display;
pub mod editor;
pub mod error;
pub mod errors;
pub mod ffp;
pub mod files;
pub mod gfx;
pub mod input;
pub mod interface;
pub mod interp;
pub mod machine;
pub mod menus;
pub mod number;
pub mod program;
pub mod tokenise;
pub mod tokens;

pub use error::{AmosError, Result};
pub use machine::Machine;
pub use program::Program;
