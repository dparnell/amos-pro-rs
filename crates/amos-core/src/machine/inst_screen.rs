//! Screens: Screen Open/Close/Display/Offset/To Front..., palettes, colour effects.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn screen_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        let _ = (it, kw);
        Ok(false)
    }

    pub(crate) fn screen_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        let _ = (it, kw);
        Ok(None)
    }

    /// Screens at the start of a program: closes everything and opens the
    /// default screen 0 (320x256, 16 colours, lowres).
    pub(crate) fn screen_reset(&mut self) {
        self.screens = crate::gfx::Screens::new();
        let screen = crate::gfx::Screen::new(0, 320, 256, 16, 0);
        self.screens.insert(screen);
    }

    /// Colour effects (flash, shift, fade) run by the VBL interrupt.
    pub(crate) fn screen_vbl(&mut self) {}

    /// Applies pending Screen Display / Offset changes (copper rebuild).
    pub(crate) fn screen_test_point(&mut self, it: &mut Interp) -> R<()> {
        let _ = it;
        Ok(())
    }
}
