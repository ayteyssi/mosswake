//! A configurable number of additional jumps, recharged only on landing.

use bevy::prelude::*;
use mw_core::input::buttons;

use super::{MechanicSet, MobilityState};
use crate::{mechanic_active, Contacts, Player, PlayerControl, RegisterMechanic, Rules, Velocity};

pub const ID: &str = "double_jump";
pub struct DoubleJumpPlugin;

impl Plugin for DoubleJumpPlugin {
    fn build(&self, app: &mut App) {
        app.register_mechanic(ID).add_systems(
            FixedUpdate,
            air_jump
                .in_set(MechanicSet::AirJump)
                .run_if(mechanic_active(ID)),
        );
    }
}

fn air_jump(
    rules: Res<Rules>,
    mut players: Query<
        (
            &Contacts,
            &mut PlayerControl,
            &mut MobilityState,
            &mut Velocity,
        ),
        With<Player>,
    >,
) {
    for (contacts, mut control, mut state, mut velocity) in &mut players {
        if !contacts.grounded
            && !state.jumped_this_tick
            && !state.is_dashing(&rules)
            && state.air_jumps_used < rules.air_jumps
            && control.just_pressed(buttons::JUMP)
        {
            velocity.0.y = -rules.gravity_dir() * rules.jump_speed();
            state.air_jumps_used += 1;
            state.jumped_this_tick = true;
            control.buffer_ticks = 0;
            control.coyote_ticks = 0;
            control.jumping = true;
        }
    }
}
