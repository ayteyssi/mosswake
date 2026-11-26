//! Optional movement mechanics. Their shared schedule resolves simultaneous inputs explicitly.

pub mod dash;
pub mod double_jump;
pub mod wall_jump;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Contacts, Player, Rules, SimSet, Velocity};

/// Order inside [`SimSet::Mechanics`]. A ground jump wins over a wall jump, which wins
/// over an air jump; dash runs last and wins over all three for this tick's velocity.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MechanicSet {
    Prepare,
    Movement,
    Jump,
    WallJump,
    AirJump,
    Dash,
}

/// Persistent controller state, also sent in snapshots for exact client replay.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobilityState {
    pub dash_ticks: u32,
    pub dash_cooldown_ticks: u32,
    pub dash_available: bool,
    /// Horizontal direction captured on dash start, -1 or 1.
    pub dash_direction: i8,
    pub air_jumps_used: u32,
    pub wall_lock_ticks: u32,
    /// Transient intent arbitration, reset before mechanics each tick.
    pub jumped_this_tick: bool,
}

impl Default for MobilityState {
    fn default() -> Self {
        Self {
            dash_ticks: 0,
            dash_cooldown_ticks: 0,
            dash_available: true,
            dash_direction: 1,
            air_jumps_used: 0,
            wall_lock_ticks: 0,
            jumped_this_tick: false,
        }
    }
}

impl MobilityState {
    pub fn is_dashing(&self, rules: &Rules) -> bool {
        rules.is_active(dash::ID) && self.dash_ticks > 0
    }
}

pub(crate) fn plugin(app: &mut App) {
    app.configure_sets(
        FixedUpdate,
        (
            MechanicSet::Prepare,
            MechanicSet::Movement,
            MechanicSet::Jump,
            MechanicSet::WallJump,
            MechanicSet::AirJump,
            MechanicSet::Dash,
        )
            .chain()
            .in_set(SimSet::Mechanics),
    )
    .add_systems(FixedUpdate, prepare.in_set(MechanicSet::Prepare))
    .add_plugins((
        dash::DashPlugin,
        double_jump::DoubleJumpPlugin,
        wall_jump::WallJumpPlugin,
    ));
}

fn prepare(
    rules: Res<Rules>,
    mut players: Query<(&Contacts, &mut MobilityState, &mut Velocity), With<Player>>,
) {
    for (contacts, mut state, mut velocity) in &mut players {
        let was_dashing = state.is_dashing(&rules);
        state.jumped_this_tick = false;
        state.dash_ticks = if rules.is_active(dash::ID) {
            state.dash_ticks.saturating_sub(1)
        } else {
            0
        };
        if was_dashing && state.dash_ticks == 0 {
            velocity.0.x = if rules.is_active(crate::MOVEMENT) {
                velocity.0.x.clamp(-rules.run_speed, rules.run_speed)
            } else {
                0.0
            };
        }
        state.dash_cooldown_ticks = state.dash_cooldown_ticks.saturating_sub(1);
        state.wall_lock_ticks = if rules.is_active(wall_jump::ID) {
            state.wall_lock_ticks.saturating_sub(1)
        } else {
            0
        };
        if contacts.grounded {
            state.air_jumps_used = 0;
            if state.dash_ticks == 0 {
                state.dash_available = true;
            }
        }
    }
}
