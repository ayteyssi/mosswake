//! Slide while pushing into a wall; a fresh jump press kicks away from it.

use bevy::prelude::*;
use mw_core::input::buttons;

use super::{MechanicSet, MobilityState};
use crate::{
    mechanic_active, seconds_to_ticks, Contacts, Facing, Player, PlayerControl, RegisterMechanic,
    Rules, SimSet, Velocity,
};

pub const ID: &str = "wall_jump";
pub struct WallJumpPlugin;

impl Plugin for WallJumpPlugin {
    fn build(&self, app: &mut App) {
        app.register_mechanic(ID)
            .add_systems(
                FixedUpdate,
                wall_jump
                    .in_set(MechanicSet::WallJump)
                    .run_if(mechanic_active(ID)),
            )
            .add_systems(
                FixedUpdate,
                wall_slide
                    .in_set(SimSet::Physics)
                    .after(crate::physics::apply_gravity)
                    .before(crate::physics::move_and_collide)
                    .run_if(mechanic_active(ID)),
            );
    }
}

fn wall_jump(
    rules: Res<Rules>,
    mut players: Query<
        (
            &Contacts,
            &mut PlayerControl,
            &mut MobilityState,
            &mut Velocity,
            &mut Facing,
        ),
        With<Player>,
    >,
) {
    for (contacts, mut control, mut state, mut velocity, mut facing) in &mut players {
        if !contacts.grounded
            && !state.jumped_this_tick
            && !state.is_dashing(&rules)
            && control.just_pressed(buttons::JUMP)
            && (contacts.on_wall_left || contacts.on_wall_right)
        {
            // If squeezed between two walls, prefer the one the player pushes into.
            let away =
                if contacts.on_wall_left && (!contacts.on_wall_right || control.input.axis_x < 0) {
                    1.0
                } else {
                    -1.0
                };
            velocity.0 = Vec2::new(
                away * rules.wall_jump_speed_x,
                -rules.gravity_dir() * rules.wall_jump_speed_y,
            );
            *facing = if away > 0.0 {
                Facing::Right
            } else {
                Facing::Left
            };
            state.wall_lock_ticks = seconds_to_ticks(rules.wall_jump_lock);
            state.jumped_this_tick = true;
            control.buffer_ticks = 0;
            control.coyote_ticks = 0;
            control.jumping = true;
        }
    }
}

fn wall_slide(
    rules: Res<Rules>,
    mut players: Query<(&Contacts, &PlayerControl, &MobilityState, &mut Velocity), With<Player>>,
) {
    for (contacts, control, state, mut velocity) in &mut players {
        let pushing = control.input.axis_x < 0 && contacts.on_wall_left
            || control.input.axis_x > 0 && contacts.on_wall_right;
        if !contacts.grounded
            && !state.is_dashing(&rules)
            && pushing
            && velocity.0.y * rules.gravity_dir() > rules.wall_slide_speed
        {
            velocity.0.y = rules.wall_slide_speed * rules.gravity_dir();
        }
    }
}
