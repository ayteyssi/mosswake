//! Bodies, gravity, movement against the tile grid and contact flags.

use bevy::math::Vec2;
use bevy::prelude::*;

use crate::grid::{Aabb, CollisionGrid};
use crate::{Rules, SimSet, TICK_DT};

/// Position of a body's **feet** (bottom-center of its box), world units, y up.
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct Position(pub Vec2);

/// World units per second.
#[derive(Component, Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocity(pub Vec2);

/// A box that moves and collides with the level.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
#[require(Position, Velocity, Contacts)]
pub struct Body {
    /// Box size, world units.
    pub size: Vec2,
    /// Multiplier of [`Rules::gravity`] (0 = floats).
    pub gravity_scale: f32,
}

/// Solid cells touching a body, refreshed every tick after movement.
///
/// `grounded` and `on_ceiling` follow the gravity direction: with inverted gravity,
/// `grounded` means touching a cell **above**.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Contacts {
    pub grounded: bool,
    pub on_ceiling: bool,
    pub on_wall_left: bool,
    pub on_wall_right: bool,
}

/// How far contact probes look, world units. Bodies rest exactly on cell edges (sweeps
/// clamp to them), so any small positive distance detects the neighbor.
const PROBE: f32 = 0.01;

pub(crate) fn plugin(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        (apply_gravity, move_and_collide)
            .chain()
            .in_set(SimSet::Physics),
    )
    .add_systems(FixedUpdate, update_contacts.in_set(SimSet::Collisions));
}

fn apply_gravity(rules: Res<Rules>, mut bodies: Query<(&Body, &mut Velocity)>) {
    let dir = rules.gravity_dir();
    // Floating bodies (scale 0) are neither accelerated nor capped: max_fall_speed is a
    // terminal velocity, it only makes sense for bodies gravity acts on.
    for (body, mut velocity) in bodies.iter_mut().filter(|(b, _)| b.gravity_scale != 0.0) {
        velocity.0.y += rules.gravity * body.gravity_scale * TICK_DT;
        // Clamp only the speed along gravity; moving against it (jumps) is unbounded here.
        if velocity.0.y * dir > rules.max_fall_speed {
            velocity.0.y = rules.max_fall_speed * dir;
        }
    }
}

fn move_and_collide(
    grid: Option<Res<CollisionGrid>>,
    mut bodies: Query<(&Body, &mut Position, &mut Velocity)>,
) {
    let Some(grid) = grid else {
        return; // no level loaded
    };
    for (body, mut position, mut velocity) in &mut bodies {
        let aabb = Aabb::from_feet(position.0, body.size);
        let result = grid.sweep(aabb, velocity.0 * TICK_DT);
        position.0 = result.aabb.feet();
        if result.blocked_x {
            velocity.0.x = 0.0;
        }
        if result.blocked_y {
            velocity.0.y = 0.0;
        }
    }
}

pub(crate) fn update_contacts(
    grid: Option<Res<CollisionGrid>>,
    rules: Res<Rules>,
    mut bodies: Query<(&Body, &Position, &mut Contacts)>,
) {
    let Some(grid) = grid else {
        return;
    };
    let dir = rules.gravity_dir();
    for (body, position, mut contacts) in &mut bodies {
        let aabb = Aabb::from_feet(position.0, body.size);
        let touches = |dx: f32, dy: f32| {
            grid.overlaps(&Aabb {
                min: aabb.min + Vec2::new(dx, dy),
                max: aabb.max + Vec2::new(dx, dy),
            })
        };
        let below = touches(0.0, -PROBE);
        let above = touches(0.0, PROBE);
        let new = Contacts {
            grounded: if dir < 0.0 { below } else { above },
            on_ceiling: if dir < 0.0 { above } else { below },
            on_wall_left: touches(-PROBE, 0.0),
            on_wall_right: touches(PROBE, 0.0),
        };
        // Avoid flagging the component as changed when nothing changed.
        contacts.set_if_neq(new);
    }
}
