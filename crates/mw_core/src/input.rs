//! Player identity and per-tick input, shared by the simulation and the network layer.
//!
//! Inputs are quantized (`i8` axes, bit buttons) so they are compact on the wire and
//! bit-identical on every machine, which keeps the simulation deterministic.

use serde::{Deserialize, Serialize};

/// A player slot in a session (host assigns them, `0..`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u8);

/// Bit flags of [`PlayerInput::buttons`]. Mechanics decide what each one does.
pub mod buttons {
    pub const JUMP: u8 = 1 << 0;
    pub const DASH: u8 = 1 << 1;
    pub const ATTACK: u8 = 1 << 2;
    pub const INTERACT: u8 = 1 << 3;
}

/// What one player does during one simulation tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Horizontal axis, `-127` (full left) to `127` (full right).
    pub axis_x: i8,
    /// Vertical axis, `-127` (full down) to `127` (full up).
    pub axis_y: i8,
    /// Held buttons, see [`buttons`].
    pub buttons: u8,
}

impl PlayerInput {
    /// Whether every bit of `button` is held.
    pub fn pressed(&self, button: u8) -> bool {
        self.buttons & button == button
    }

    /// Horizontal axis as `-1.0..=1.0` (`-128` is clamped like `-127`).
    pub fn x(&self) -> f32 {
        f32::from(self.axis_x.max(-127)) / 127.0
    }

    /// Vertical axis as `-1.0..=1.0`.
    pub fn y(&self) -> f32 {
        f32::from(self.axis_y.max(-127)) / 127.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_are_normalized_and_symmetric() {
        let input = PlayerInput {
            axis_x: 127,
            axis_y: i8::MIN,
            buttons: 0,
        };
        assert_eq!(input.x(), 1.0);
        assert_eq!(input.y(), -1.0);
        assert_eq!(PlayerInput::default().x(), 0.0);
    }

    #[test]
    fn buttons() {
        let input = PlayerInput {
            buttons: buttons::JUMP | buttons::ATTACK,
            ..Default::default()
        };
        assert!(input.pressed(buttons::JUMP));
        assert!(input.pressed(buttons::JUMP | buttons::ATTACK));
        assert!(!input.pressed(buttons::DASH));
        assert!(!input.pressed(buttons::JUMP | buttons::DASH));
    }
}
