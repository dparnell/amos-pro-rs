//! Plain keywords called straight through their typed functions on
//! `Hardware` (`docs/PERFORMANCE.md`), when every given parameter is an
//! integer: what their handler does after reading its `Args`, without the
//! preset (`Interp::preset_ints`, `take_preset_into`, `Args`).

use crate::interp::value::{ENT_NUL, Value};
use crate::interp::{Interp, R};
use crate::machine::Hardware;
use crate::tokens::tk::*;

/// The integer parameters of a plain call: `given` has bit k set when
/// parameter k is given.
#[derive(Clone, Copy)]
pub(super) struct Ints<'a> {
    pub vals: &'a [i32],
    pub given: u32,
}

impl Ints<'_> {
    /// `Args::int(k)`: `ENT_NUL` when omitted.
    fn int(&self, k: usize) -> i32 {
        if self.given & (1 << k) != 0 { self.vals.get(k).copied().unwrap_or(ENT_NUL) } else { ENT_NUL }
    }

    /// `Args::opt(k)`: `None` when omitted (or `ENT_NUL`).
    fn opt(&self, k: usize) -> Option<i32> {
        let v = self.int(k);
        if v == ENT_NUL { None } else { Some(v) }
    }
}

/// Is `token` an instruction `instruction` runs?
pub(super) fn has_instruction(token: u16) -> bool {
    matches!(
        token,
        LOCATE
            | PEN
            | PAPER
            | INK
            | INK_2
            | INK_3
            | PLOT
            | PLOT_2
            | DRAW_TO
            | DRAW
            | BOX
            | BAR
            | CIRCLE
            | LIMIT_MOUSE
            | LIMIT_MOUSE_2
            | LIMIT_MOUSE_3
            | SCREEN_TO_FRONT
            | SCREEN_TO_FRONT_2
            | POKE
            | DOKE
            | LOKE
    )
}

/// Is `token` a function `function` gives?
pub(super) fn has_function(token: u16) -> bool {
    matches!(
        token,
        ZONE | ZONE_2
            | HZONE
            | HZONE_2
            | POINT
            | COLOUR_2
            | X_SCREEN
            | X_SCREEN_2
            | Y_SCREEN
            | Y_SCREEN_2
            | X_HARD
            | X_HARD_2
            | Y_HARD
            | Y_HARD_2
            | CHOICE
            | CHOICE_2
            | PEEK
            | DEEK
            | LEEK
            | MOUSE_CLICK
            | SCANCODE
    )
}

/// The instruction `token` (`has_instruction`) with parameters `a`, as its
/// handler runs it.
pub(super) fn instruction(hw: &mut Hardware, it: &mut Interp, token: u16, a: Ints) -> R<()> {
    match token {
        LOCATE => hw.locate(a.opt(0), a.opt(1)),
        PEN => hw.pen(a.int(0)),
        PAPER => hw.paper(a.int(0)),
        INK | INK_2 | INK_3 => hw.ink(a.opt(0), a.opt(1), a.opt(2)),
        PLOT | PLOT_2 => hw.plot(a.opt(0), a.opt(1), a.opt(2)),
        DRAW_TO => hw.draw_to(a.opt(0), a.opt(1)),
        DRAW => hw.draw(a.opt(0), a.opt(1), a.opt(2), a.opt(3)),
        BOX => hw.box_(a.int(0), a.int(1), a.int(2), a.int(3)),
        BAR => hw.bar(a.int(0), a.int(1), a.int(2), a.int(3)),
        CIRCLE => hw.circle(a.opt(0), a.opt(1), a.int(2)),
        LIMIT_MOUSE => {
            hw.limit_mouse();
            Ok(())
        }
        LIMIT_MOUSE_2 => hw.limit_mouse_screen(a.int(0)),
        LIMIT_MOUSE_3 => {
            hw.limit_mouse_area(a.int(0), a.int(1), a.int(2), a.int(3));
            Ok(())
        }
        SCREEN_TO_FRONT => hw.screen_to_front(None),
        SCREEN_TO_FRONT_2 => hw.screen_to_front(Some(a.int(0))),
        POKE => hw.poke(it, a.int(0), a.int(1)),
        DOKE => hw.doke(it, a.int(0), a.int(1)),
        LOKE => hw.loke(it, a.int(0), a.int(1)),
        _ => unreachable!("not a direct instruction"),
    }
}

/// The function `token` (`has_function`) with parameters `a`, as its
/// handler gives it.
pub(super) fn function(hw: &mut Hardware, it: &mut Interp, token: u16, a: Ints) -> R<Value> {
    Ok(Value::Int(match token {
        ZONE => hw.zone_fn(None, a.int(0), a.int(1), false)?,
        ZONE_2 => hw.zone_fn(Some(a.int(0)), a.int(1), a.int(2), false)?,
        HZONE => hw.zone_fn(None, a.int(0), a.int(1), true)?,
        HZONE_2 => hw.zone_fn(Some(a.int(0)), a.int(1), a.int(2), true)?,
        POINT => hw.point(a.opt(0), a.opt(1))?,
        COLOUR_2 => hw.colour_fn(a.int(0))?,
        X_SCREEN => hw.x_screen(None, a.int(0))?,
        X_SCREEN_2 => hw.x_screen(Some(a.int(0)), a.int(1))?,
        Y_SCREEN => hw.y_screen(None, a.int(0))?,
        Y_SCREEN_2 => hw.y_screen(Some(a.int(0)), a.int(1))?,
        X_HARD => hw.x_hard(None, a.int(0))?,
        X_HARD_2 => hw.x_hard(Some(a.int(0)), a.int(1))?,
        Y_HARD => hw.y_hard(None, a.int(0))?,
        Y_HARD_2 => hw.y_hard(Some(a.int(0)), a.int(1))?,
        CHOICE => hw.choice_fn(None)?,
        CHOICE_2 => hw.choice_fn(Some(a.int(0)))?,
        PEEK => hw.peek(it, a.int(0)),
        DEEK => hw.deek(it, a.int(0)),
        LEEK => hw.leek(it, a.int(0)),
        MOUSE_CLICK => hw.mouse_click(),
        SCANCODE => hw.scancode(),
        _ => unreachable!("not a direct function"),
    }))
}
