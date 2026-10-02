//! Hardware sprites, bobs, AMAL, collisions.

use super::Hardware;
use crate::interp::value::Value;
use crate::interp::{Interp, R};
use crate::tokens::Keyword;

impl Hardware {
    pub(crate) fn sprites_instruction(&mut self, it: &mut Interp, kw: Keyword) -> R<bool> {
        let _ = (it, kw);
        Ok(false)
    }

    pub(crate) fn sprites_function(&mut self, it: &mut Interp, kw: Keyword) -> R<Option<Value>> {
        let _ = (it, kw);
        Ok(None)
    }

    pub(crate) fn sprites_reset(&mut self) {}

    /// Refreshes `self.sprites.display` (hardware sprites and mouse pointer)
    /// before a frame is built.
    pub(crate) fn sprites_prepare_frame(&mut self) {}

    /// AMAL and sprite updates done by the VBL interrupt.
    pub(crate) fn sprites_vbl(&mut self) {}

    /// Bob redraw and screen swap done at interpreter test points.
    pub(crate) fn sprites_test_point(&mut self, it: &mut Interp) -> R<()> {
        let _ = it;
        Ok(())
    }
}
