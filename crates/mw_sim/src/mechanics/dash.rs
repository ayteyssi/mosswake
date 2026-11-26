//! Horizontal, gravity-free dash in the input direction (or facing when idle).

use bevy::prelude::*;
use mw_core::input::buttons;

use super::{MechanicSet, MobilityState};
use crate::{
    mechanic_active, seconds_to_ticks, Facing, Player, PlayerControl, RegisterMechanic, Rules,
    Velocity,
};

pub const ID: &str = "dash";
pub struct DashPlugin;

impl Plugin for DashPlugin {
    fn build(&self, app: &mut App) {
        app.register_mechanic(ID).add_systems(
            FixedUpdate,
            dash.in_set(MechanicSet::Dash).run_if(mechanic_active(ID)),
        );
    }
}

fn dash(
    rules: Res<Rules>,
    mut players: Query<
        (
            &mut PlayerControl,
            &mut Facing,
            &mut MobilityState,
            &mut Velocity,
        ),
        With<Player>,
    >,
) {
    for (mut control, mut facing, mut state, mut velocity) in &mut players {
        if state.dash_ticks == 0
            && state.dash_cooldown_ticks == 0
            && state.dash_available
            && control.just_pressed(buttons::DASH)
        {
            state.dash_direction = match control.input.axis_x.cmp(&0) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Greater => 1,
                std::cmp::Ordering::Equal => {
                    if *facing == Facing::Left {
                        -1
                    } else {
                        1
                    }
                }
            };
            *facing = if state.dash_direction < 0 {
                Facing::Left
            } else {
                Facing::Right
            };
            state.dash_ticks = seconds_to_ticks(rules.dash_duration).max(1);
            state.dash_cooldown_ticks = seconds_to_ticks(rules.dash_cooldown);
            state.dash_available = false;
            control.jumping = false;
            control.buffer_ticks = 0;
            control.coyote_ticks = 0;
        }
        if state.dash_ticks > 0 {
            velocity.0 = Vec2::new(f32::from(state.dash_direction) * rules.dash_speed, 0.0);
        }
    }
}
