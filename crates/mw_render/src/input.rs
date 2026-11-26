//! Keyboard and gamepad → [`PlayerInput`] for the local player.
//!
//! Sampled every frame, handed to the simulation once per tick. A button tapped between two
//! ticks (possible above 60 fps) is latched so the tick still sees it.

use bevy::prelude::*;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_sim::TickInputs;

/// Stick values below this are ignored.
pub const DEADZONE: f32 = 0.25;

/// Feeds the keyboard and gamepads to `player`.
pub struct LocalInputPlugin {
    pub player: PlayerId,
}

impl Plugin for LocalInputPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(LocalInput {
            player: self.player,
            current: PlayerInput::default(),
            latched: 0,
        })
        .add_systems(PreUpdate, sample.after(bevy::input::InputSystems))
        .add_systems(FixedPreUpdate, hand_to_simulation);
    }
}

/// Latest sampled input of the local player.
#[derive(Resource, Debug)]
pub struct LocalInput {
    pub player: PlayerId,
    pub current: PlayerInput,
    /// Buttons pressed since the last tick.
    latched: u8,
}

/// Keyboard bindings: (button, keys).
pub const KEY_BINDINGS: &[(u8, &[KeyCode])] = &[
    (
        buttons::JUMP,
        &[KeyCode::Space, KeyCode::KeyK, KeyCode::KeyZ],
    ),
    (
        buttons::DASH,
        &[KeyCode::ShiftLeft, KeyCode::KeyL, KeyCode::KeyX],
    ),
    (buttons::ATTACK, &[KeyCode::KeyJ, KeyCode::KeyC]),
    (buttons::INTERACT, &[KeyCode::KeyE, KeyCode::ArrowUp]),
];

/// Gamepad bindings: (button, pad buttons).
pub const PAD_BINDINGS: &[(u8, &[GamepadButton])] = &[
    (buttons::JUMP, &[GamepadButton::South]),
    (
        buttons::DASH,
        &[GamepadButton::RightTrigger, GamepadButton::East],
    ),
    (buttons::ATTACK, &[GamepadButton::West]),
    (buttons::INTERACT, &[GamepadButton::North]),
];

/// Quantizes an axis in `-1.0..=1.0` (with deadzone) to the `i8` wire format.
pub fn quantize_axis(value: f32) -> i8 {
    if value.abs() < DEADZONE {
        0
    } else {
        (value.clamp(-1.0, 1.0) * 127.0).round() as i8
    }
}

fn sample(keys: Res<ButtonInput<KeyCode>>, pads: Query<&Gamepad>, mut local: ResMut<LocalInput>) {
    let key_axis = |neg: &[KeyCode], pos: &[KeyCode]| {
        let down = |codes: &[KeyCode]| codes.iter().any(|k| keys.pressed(*k));
        f32::from(i8::from(down(pos)) - i8::from(down(neg)))
    };
    let mut x = key_axis(
        &[KeyCode::KeyA, KeyCode::ArrowLeft],
        &[KeyCode::KeyD, KeyCode::ArrowRight],
    );
    let mut y = key_axis(&[KeyCode::KeyS, KeyCode::ArrowDown], &[KeyCode::KeyW]);
    let (mut held, mut pressed) = (0u8, 0u8);
    for (bit, codes) in KEY_BINDINGS {
        if codes.iter().any(|k| keys.pressed(*k)) {
            held |= bit;
        }
        if codes.iter().any(|k| keys.just_pressed(*k)) {
            pressed |= bit;
        }
    }
    for pad in &pads {
        let stick = |axis| pad.get(axis).unwrap_or(0.0);
        let dpad = |neg, pos| f32::from(i8::from(pad.pressed(pos)) - i8::from(pad.pressed(neg)));
        let px = stick(GamepadAxis::LeftStickX)
            + dpad(GamepadButton::DPadLeft, GamepadButton::DPadRight);
        let py =
            stick(GamepadAxis::LeftStickY) + dpad(GamepadButton::DPadDown, GamepadButton::DPadUp);
        if px.abs() > x.abs() {
            x = px;
        }
        if py.abs() > y.abs() {
            y = py;
        }
        for (bit, pad_buttons) in PAD_BINDINGS {
            if pad_buttons.iter().any(|b| pad.pressed(*b)) {
                held |= bit;
            }
            if pad_buttons.iter().any(|b| pad.just_pressed(*b)) {
                pressed |= bit;
            }
        }
    }
    local.latched |= pressed;
    local.current = PlayerInput {
        axis_x: quantize_axis(x),
        axis_y: quantize_axis(y),
        buttons: held | local.latched,
    };
}

fn hand_to_simulation(mut local: ResMut<LocalInput>, mut inputs: ResMut<TickInputs>) {
    inputs.0.insert(local.player, local.current);
    // The tick has seen the taps; from now on only held buttons count.
    local.current.buttons &= !local.latched;
    local.latched = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_quantization() {
        assert_eq!(quantize_axis(0.0), 0);
        assert_eq!(quantize_axis(0.2), 0, "deadzone");
        assert_eq!(quantize_axis(1.0), 127);
        assert_eq!(quantize_axis(-1.0), -127);
        assert_eq!(quantize_axis(2.0), 127, "keyboard + pad saturate");
        assert_eq!(quantize_axis(0.5), 64);
    }

    #[test]
    fn a_tap_between_ticks_reaches_the_next_tick_once() {
        let mut app = App::new();
        app.add_plugins(LocalInputPlugin {
            player: PlayerId(0),
        })
        .init_resource::<TickInputs>()
        .init_resource::<ButtonInput<KeyCode>>();
        // Frame 1: Space pressed and released within the frame (no tick ran).
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.world_mut().run_schedule(PreUpdate);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::Space);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut().run_schedule(PreUpdate);
        // Next tick sees the jump…
        app.world_mut().run_schedule(FixedPreUpdate);
        let seen = app.world().resource::<TickInputs>().0[&PlayerId(0)];
        assert!(seen.pressed(buttons::JUMP));
        // …and the following one does not.
        app.world_mut().run_schedule(PreUpdate);
        app.world_mut().run_schedule(FixedPreUpdate);
        let seen = app.world().resource::<TickInputs>().0[&PlayerId(0)];
        assert!(!seen.pressed(buttons::JUMP));
    }
}
