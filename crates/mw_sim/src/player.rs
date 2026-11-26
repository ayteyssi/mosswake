//! Players: spawning, per-tick input, and the `movement` and `jump` mechanics.
//!
//! Everything tunable comes from [`Rules`]. Timers (coyote time, jump buffer) count whole
//! ticks so they behave identically everywhere (no float accumulation).

use bevy::prelude::*;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use serde::{Deserialize, Serialize};

use crate::{
    mechanic_active, tiles_to_world, Body, Contacts, LoadedLevel, Position, RegisterMechanic,
    Rules, SimId, SimIds, SimSet, TickInputs, Velocity, TICK_DT, TICK_HZ,
};

/// Mechanic id: horizontal running with ground and air acceleration.
pub const MOVEMENT: &str = "movement";
/// Mechanic id: variable-height jump with coyote time and jump buffer.
pub const JUMP: &str = "jump";

/// Marks a player-controlled body.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
#[require(PlayerControl, PlayerState, Facing)]
pub struct Player(pub PlayerId);

/// Input of this tick and the previous one (for press / release edges) and jump timers.
/// Part of [`crate::Snapshot`]: a client needs it to re-simulate its player exactly.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlayerControl {
    pub input: PlayerInput,
    pub previous: PlayerInput,
    /// Ticks left during which a jump is allowed although not grounded.
    pub coyote_ticks: u32,
    /// Ticks left during which a buffered jump press is still pending.
    pub buffer_ticks: u32,
    /// Rising from a jump whose button is still held (release cuts the jump).
    pub jumping: bool,
}

impl PlayerControl {
    /// Whether `button` went down this tick.
    pub fn just_pressed(&self, button: u8) -> bool {
        self.input.pressed(button) && !self.previous.pressed(button)
    }
}

/// What the player is doing, for animation. Updated after collisions every tick.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlayerState {
    #[default]
    Idle,
    Run,
    /// Airborne and moving against gravity.
    Jump,
    /// Airborne and moving with gravity (or not moving vertically).
    Fall,
}

/// Which way the player looks; kept when the input is released.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Facing {
    Left,
    #[default]
    Right,
}

/// Speeds below this (units/s) count as standing still for [`PlayerState`].
const STILL: f32 = 1.0;

pub(crate) fn plugin(app: &mut App) {
    app.register_mechanic(MOVEMENT)
        .register_mechanic(JUMP)
        .add_systems(FixedUpdate, read_inputs.in_set(SimSet::Input))
        .add_systems(
            FixedUpdate,
            (
                movement.run_if(mechanic_active(MOVEMENT)),
                jump.run_if(mechanic_active(JUMP)),
            )
                .in_set(SimSet::Mechanics),
        )
        .add_systems(
            FixedUpdate,
            update_state
                .in_set(SimSet::Collisions)
                .after(crate::physics::update_contacts),
        );
}

/// Converts a duration from [`Rules`] to whole ticks (rounded).
pub fn seconds_to_ticks(seconds: f32) -> u32 {
    (f64::from(seconds) * TICK_HZ).round() as u32
}

/// Spawns `player` on the level's spawn point `player % spawns` with the body size from
/// [`Rules`]. Requires a loaded level.
pub fn spawn_player(world: &mut World, player: PlayerId) -> SimId {
    let level = &world.resource::<LoadedLevel>().0;
    let spawn = level.spawns[usize::from(player.0) % level.spawns.len()];
    let feet = tiles_to_world(level, spawn.x, spawn.y);
    let rules = world.resource::<Rules>();
    let body = Body {
        size: Vec2::new(rules.player_width, rules.player_height),
        gravity_scale: 1.0,
    };
    let id = world.resource_mut::<SimIds>().allocate();
    world.spawn((id, Player(player), body, Position(feet)));
    id
}

/// Despawns every body of `player` (on disconnect) and returns their ids, by id.
pub fn despawn_player(world: &mut World, player: PlayerId) -> Vec<SimId> {
    let mut query = world.query::<(Entity, &SimId, &Player)>();
    let mut found: Vec<(SimId, Entity)> = query
        .iter(world)
        .filter(|(_, _, p)| p.0 == player)
        .map(|(entity, id, _)| (*id, entity))
        .collect();
    found.sort_by_key(|(id, _)| *id);
    for (_, entity) in &found {
        world.despawn(*entity);
    }
    found.into_iter().map(|(id, _)| id).collect()
}

fn read_inputs(inputs: Res<TickInputs>, mut players: Query<(&Player, &mut PlayerControl)>) {
    for (player, mut control) in &mut players {
        control.previous = control.input;
        control.input = inputs.0.get(&player.0).copied().unwrap_or_default();
    }
}

/// Moves `current` towards `target` by at most `step`.
fn approach(current: f32, target: f32, step: f32) -> f32 {
    if current < target {
        (current + step).min(target)
    } else {
        (current - step).max(target)
    }
}

fn movement(
    rules: Res<Rules>,
    mut players: Query<(&PlayerControl, &Contacts, &mut Velocity, &mut Facing), With<Player>>,
) {
    for (control, contacts, mut velocity, mut facing) in &mut players {
        let axis = control.input.x();
        let target = axis * rules.run_speed;
        // Accelerate when pushing along the current motion, otherwise brake.
        let pushing =
            axis != 0.0 && (velocity.0.x == 0.0 || velocity.0.x.signum() == axis.signum());
        let rate = match (contacts.grounded, pushing) {
            (true, true) => rules.ground_accel,
            (true, false) => rules.ground_decel,
            (false, true) => rules.air_accel,
            (false, false) => rules.air_decel,
        };
        velocity.0.x = approach(velocity.0.x, target, rate * TICK_DT);
        if axis > 0.0 {
            *facing = Facing::Right;
        } else if axis < 0.0 {
            *facing = Facing::Left;
        }
    }
}

fn jump(
    rules: Res<Rules>,
    mut players: Query<(&mut PlayerControl, &Contacts, &mut Velocity), With<Player>>,
) {
    let up = -rules.gravity_dir();
    for (mut control, contacts, mut velocity) in &mut players {
        control.coyote_ticks = if contacts.grounded {
            seconds_to_ticks(rules.coyote_time).max(1)
        } else {
            control.coyote_ticks.saturating_sub(1)
        };
        control.buffer_ticks = if control.just_pressed(buttons::JUMP) {
            seconds_to_ticks(rules.jump_buffer).max(1)
        } else {
            control.buffer_ticks.saturating_sub(1)
        };

        if control.buffer_ticks > 0 && control.coyote_ticks > 0 {
            velocity.0.y = rules.jump_speed() * up;
            control.buffer_ticks = 0;
            control.coyote_ticks = 0;
            control.jumping = true;
        } else if control.jumping {
            let rising = velocity.0.y * up > 0.0;
            if !rising {
                control.jumping = false;
            } else if !control.input.pressed(buttons::JUMP) {
                velocity.0.y *= rules.jump_cut;
                control.jumping = false;
            }
        }
    }
}

fn update_state(
    rules: Res<Rules>,
    mut players: Query<(&PlayerControl, &Contacts, &Velocity, &mut PlayerState), With<Player>>,
) {
    let up = -rules.gravity_dir();
    for (control, contacts, velocity, mut state) in &mut players {
        let new = if contacts.grounded {
            if velocity.0.x.abs() > STILL
                || control.input.axis_x != 0 && !blocked(contacts, control)
            {
                PlayerState::Run
            } else {
                PlayerState::Idle
            }
        } else if velocity.0.y * up > STILL {
            PlayerState::Jump
        } else {
            PlayerState::Fall
        };
        state.set_if_neq(new);
    }
}

/// Pushing into a wall: not running.
fn blocked(contacts: &Contacts, control: &PlayerControl) -> bool {
    (control.input.axis_x > 0 && contacts.on_wall_right)
        || (control.input.axis_x < 0 && contacts.on_wall_left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_never_overshoots() {
        assert_eq!(approach(0.0, 10.0, 3.0), 3.0);
        assert_eq!(approach(9.0, 10.0, 3.0), 10.0);
        assert_eq!(approach(5.0, -10.0, 3.0), 2.0);
        assert_eq!(approach(-9.0, -10.0, 3.0), -10.0);
    }

    #[test]
    fn seconds_round_to_ticks() {
        assert_eq!(seconds_to_ticks(0.1), 6);
        assert_eq!(seconds_to_ticks(0.0), 0);
        assert_eq!(seconds_to_ticks(1.0), 60);
    }

    #[test]
    fn just_pressed_is_an_edge() {
        let held = PlayerInput {
            buttons: buttons::JUMP,
            ..Default::default()
        };
        let mut control = PlayerControl {
            input: held,
            ..Default::default()
        };
        assert!(control.just_pressed(buttons::JUMP));
        control.previous = held;
        assert!(!control.just_pressed(buttons::JUMP));
    }
}
