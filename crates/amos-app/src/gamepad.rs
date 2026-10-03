//! Game controllers (gilrs) as Amiga joysticks.
//!
//! The first controller is plugged into the joystick port (port 1, the one
//! most AMOS games read with `Joy(1)`), the second into the mouse port
//! (port 0). Directions come from the d-pad or the left stick; any face
//! button or shoulder trigger is fire, like the single button of an Amiga
//! joystick.

use amos_core::input::InputState;
use gilrs::{Axis, Button, GamepadId, Gilrs};

/// Stick deflection that counts as a direction.
const DEADZONE: f32 = 0.5;

pub struct Gamepads {
    gilrs: Option<Gilrs>,
    /// Controllers in the order they were connected.
    order: Vec<GamepadId>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                log::warn!("Game controllers unavailable: {e}");
                None
            }
        };
        let order = gilrs.as_ref().map(|g| g.gamepads().map(|(id, _)| id).collect()).unwrap_or_default();
        Gamepads { gilrs, order }
    }

    /// Reads the controllers and updates the joystick ports.
    pub fn poll(&mut self, input: &mut InputState) {
        let Some(gilrs) = &mut self.gilrs else { return };
        // Events must be drained for gilrs to update its state.
        while let Some(ev) = gilrs.next_event() {
            match ev.event {
                gilrs::EventType::Connected => {
                    if !self.order.contains(&ev.id) {
                        log::info!("Game controller connected: {}", gilrs.gamepad(ev.id).name());
                        self.order.push(ev.id);
                    }
                }
                gilrs::EventType::Disconnected => self.order.retain(|&id| id != ev.id),
                _ => {}
            }
        }
        // First controller: port 1 (joystick), second: port 0 (mouse port).
        for (slot, port) in [(0usize, 1usize), (1, 0)] {
            let bits = self.order.get(slot).and_then(|&id| gilrs.connected_gamepad(id)).map_or(0, |pad| {
                let x = pad.value(Axis::LeftStickX);
                let y = pad.value(Axis::LeftStickY);
                let mut b = 0u8;
                if pad.is_pressed(Button::DPadUp) || y > DEADZONE {
                    b |= 1;
                }
                if pad.is_pressed(Button::DPadDown) || y < -DEADZONE {
                    b |= 2;
                }
                if pad.is_pressed(Button::DPadLeft) || x < -DEADZONE {
                    b |= 4;
                }
                if pad.is_pressed(Button::DPadRight) || x > DEADZONE {
                    b |= 8;
                }
                let fire = [
                    Button::South,
                    Button::East,
                    Button::West,
                    Button::North,
                    Button::LeftTrigger,
                    Button::RightTrigger,
                    Button::LeftTrigger2,
                    Button::RightTrigger2,
                ];
                if fire.iter().any(|&f| pad.is_pressed(f)) {
                    b |= 16;
                }
                b
            });
            input.set_gamepad(port, bits);
        }
    }
}
