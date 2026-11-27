//! Deterministic melee and damage. Hits are resolved in (attacker, target) SimId order.
use std::collections::BTreeSet;

use bevy::prelude::*;
use mw_core::input::buttons;
use mw_level::LayerKind;
use serde::{Deserialize, Serialize};

use crate::{
    seconds_to_ticks, Aabb, Body, Contacts, Facing, LoadedLevel, Player, PlayerControl, Position,
    RegisterMechanic, Rules, SimId, SimSet, Velocity,
};

pub const MELEE: &str = "melee";

#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub current: u32,
    pub max: u32,
    pub invincible_ticks: u32,
    pub stun_ticks: u32,
    pub knockback: [f32; 2],
}
impl Health {
    pub fn new(max: u32) -> Self {
        Self {
            current: max,
            max,
            invincible_ticks: 0,
            stun_ticks: 0,
            knockback: [0.0; 2],
        }
    }
}

/// Damageable area, relative to the body's feet, independent of solid-body collision.
#[derive(Component, Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Hurtbox {
    pub size: [f32; 2],
    pub offset: [f32; 2],
}
impl Hurtbox {
    pub fn from_size(size: Vec2) -> Self {
        Self {
            size: size.to_array(),
            offset: [-size.x / 2.0, 0.0],
        }
    }
}

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeleeState {
    pub cooldown_ticks: u32,
}

/// Vertical directions follow gravity: Up is against it, Down is along it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttackDirection {
    Left,
    Right,
    Up,
    Down,
}

/// One temporary swing attached to its attacker; each target is damaged at most once.
#[derive(Component, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hitbox {
    pub direction: AttackDirection,
    pub size: [f32; 2],
    pub offset: [f32; 2],
    pub damage: u32,
    pub remaining_ticks: u32,
    pub hit_targets: BTreeSet<SimId>,
    pub pogoed: bool,
}

/// An entity with a Hurtbox that can be pogoed without taking damage (e.g. spikes).
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct PogoSurface;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageEvent {
    pub target: SimId,
    pub source: Option<SimId>,
    pub amount: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeathEvent {
    pub target: SimId,
    pub source: Option<SimId>,
}

/// Accepted impacts and deaths of this tick, available in ScriptPost; cleared next collision pass.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct CombatEvents {
    pub damage: Vec<DamageEvent>,
    pub deaths: Vec<DeathEvent>,
}

/// Damage requested by other mechanics/scripts before Collisions. Drained in stable order.
#[derive(Resource, Debug, Default)]
pub struct DamageQueue(pub Vec<DamageEvent>);

pub(crate) fn plugin(app: &mut App) {
    app.register_mechanic(MELEE)
        .init_resource::<CombatEvents>()
        .init_resource::<DamageQueue>()
        .add_systems(
            FixedUpdate,
            enforce_knockback
                .after(SimSet::Mechanics)
                .before(SimSet::Physics),
        )
        .add_systems(
            FixedUpdate,
            combat_step
                .in_set(SimSet::Collisions)
                .after(crate::physics::update_contacts)
                .before(crate::player::update_state),
        );
}

/// Runs after all mobility mechanics, so they cannot cancel hit-stun on the next tick.
fn enforce_knockback(
    mut bodies: Query<(
        &Health,
        &mut Velocity,
        Option<&mut crate::mechanics::MobilityState>,
    )>,
) {
    for (health, mut velocity, mobility) in &mut bodies {
        if health.current == 0 || health.stun_ticks > 0 {
            if let Some(mut mobility) = mobility {
                mobility.dash_ticks = 0;
                mobility.wall_lock_ticks = 0;
            }
        }
        if health.current == 0 {
            velocity.0 = Vec2::ZERO;
        } else if health.stun_ticks > 0 {
            velocity.0 = Vec2::from_array(health.knockback);
        }
    }
}

fn overlap(a: Aabb, b: Aabb) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}
fn area(feet: Vec2, offset: [f32; 2], size: [f32; 2]) -> Aabb {
    let min = feet + Vec2::from_array(offset);
    Aabb {
        min,
        max: min + Vec2::from_array(size),
    }
}

fn new_hitbox(
    body: &Body,
    facing: Facing,
    control: &PlayerControl,
    contacts: &Contacts,
    rules: &Rules,
) -> Hitbox {
    let direction = if control.input.axis_y > 0 {
        AttackDirection::Up
    } else if control.input.axis_y < 0 && !contacts.grounded {
        AttackDirection::Down
    } else if control.input.axis_x < 0 || control.input.axis_x == 0 && facing == Facing::Left {
        AttackDirection::Left
    } else {
        AttackDirection::Right
    };
    let r = rules.melee_reach;
    let t = rules.melee_thickness;
    let (size, offset) = match direction {
        AttackDirection::Left => ([r, t], [-body.size.x / 2.0 - r, (body.size.y - t) / 2.0]),
        AttackDirection::Right => ([r, t], [body.size.x / 2.0, (body.size.y - t) / 2.0]),
        AttackDirection::Up | AttackDirection::Down => {
            let up = -rules.gravity_dir();
            let sign = if direction == AttackDirection::Up {
                up
            } else {
                -up
            };
            (
                [t, r],
                [-t / 2.0, if sign > 0.0 { body.size.y } else { -r }],
            )
        }
    };
    Hitbox {
        direction,
        size,
        offset,
        damage: rules.melee_damage,
        remaining_ticks: seconds_to_ticks(rules.melee_duration).max(1),
        hit_targets: BTreeSet::new(),
        pogoed: false,
    }
}

fn combat_step(world: &mut World) {
    let rules = world.resource::<Rules>().clone();
    *world.resource_mut::<CombatEvents>() = CombatEvents::default();
    for mut health in world.query::<&mut Health>().iter_mut(world) {
        health.invincible_ticks = health.invincible_ticks.saturating_sub(1);
        health.stun_ticks = health.stun_ticks.saturating_sub(1);
    }
    for mut state in world.query::<&mut MeleeState>().iter_mut(world) {
        state.cooldown_ticks = state.cooldown_ticks.saturating_sub(1);
    }
    let mut queued = std::mem::take(&mut world.resource_mut::<DamageQueue>().0);
    queued.sort_by_key(|e| (e.target, e.source, e.amount));
    for event in queued {
        apply_damage(world, &rules, event);
    }

    let enabled = rules.is_active(MELEE);
    let mut query = world.query::<(
        Entity,
        &SimId,
        &Body,
        &PlayerControl,
        &Facing,
        &Contacts,
        &Health,
        &MeleeState,
    )>();
    let mut starts: Vec<_> = query
        .iter(world)
        .filter(|(entity, _, _, control, _, _, health, state)| {
            enabled
                && health.current > 0
                && health.stun_ticks == 0
                && state.cooldown_ticks == 0
                && control.just_pressed(buttons::ATTACK)
                && world.get::<Hitbox>(*entity).is_none()
        })
        .map(|(entity, id, body, control, facing, contacts, _, _)| {
            (
                *id,
                entity,
                new_hitbox(body, *facing, control, contacts, &rules),
            )
        })
        .collect();
    starts.sort_by_key(|(id, _, _)| *id);
    for (_, entity, hitbox) in starts {
        world.entity_mut(entity).insert((
            hitbox,
            MeleeState {
                cooldown_ticks: seconds_to_ticks(rules.melee_cooldown).max(1),
            },
        ));
    }
    let mut attacks: Vec<_> = world
        .query::<(Entity, &SimId, &Position, &Hitbox)>()
        .iter(world)
        .map(|(e, id, p, h)| (e, *id, p.0, h.clone()))
        .collect();
    attacks.sort_by_key(|(_, id, _, _)| *id);
    let mut targets: Vec<_> = world
        .query::<(
            Entity,
            &SimId,
            &Position,
            &Hurtbox,
            Option<&Player>,
            Option<&PogoSurface>,
        )>()
        .iter(world)
        .map(|(e, id, p, h, player, pogo)| {
            (
                e,
                *id,
                area(p.0, h.offset, h.size),
                player.is_some(),
                pogo.is_some(),
            )
        })
        .collect();
    targets.sort_by_key(|(_, id, _, _, _)| *id);
    for (entity, source, feet, mut hitbox) in attacks {
        if !enabled || world.get::<Health>(entity).is_some_and(|h| h.current == 0) {
            world.entity_mut(entity).remove::<Hitbox>();
            continue;
        }
        let box_area = area(feet, hitbox.offset, hitbox.size);
        let mut pogo = hitbox.direction == AttackDirection::Down
            && !hitbox.pogoed
            && hazard_overlap(world, box_area);
        for (target_entity, target, target_area, is_player, is_pogo) in &targets {
            // PvP belongs to #17. Players never damage each other in this mechanic.
            if *target == source || *is_player || !overlap(box_area, *target_area) {
                continue;
            }
            let alive = world
                .get::<Health>(*target_entity)
                .is_some_and(|h| h.current > 0);
            if hitbox.direction == AttackDirection::Down && !hitbox.pogoed && (alive || *is_pogo) {
                pogo = true;
            }
            if !hitbox.hit_targets.contains(target)
                && apply_damage(
                    world,
                    &rules,
                    DamageEvent {
                        target: *target,
                        source: Some(source),
                        amount: hitbox.damage,
                    },
                )
            {
                hitbox.hit_targets.insert(*target);
            }
        }
        if pogo {
            cancel_mobility(world, entity);
            if let Some(mut velocity) = world.get_mut::<Velocity>(entity) {
                velocity.0.y = -rules.gravity_dir() * rules.pogo_speed;
            }
            if let Some(mut control) = world.get_mut::<PlayerControl>(entity) {
                control.jumping = false;
                control.coyote_ticks = 0;
                control.buffer_ticks = 0;
            }
            hitbox.pogoed = true;
        }
        hitbox.remaining_ticks = hitbox.remaining_ticks.saturating_sub(1);
        if hitbox.remaining_ticks == 0 {
            world.entity_mut(entity).remove::<Hitbox>();
        } else {
            world.entity_mut(entity).insert(hitbox);
        }
    }
}

/// Apply one request, reporting accepted damage only. Death is emitted once; the body remains
/// for future respawn/death hooks. Source ids and event order are stable, never Bevy Entity ids.
fn apply_damage(world: &mut World, rules: &Rules, event: DamageEvent) -> bool {
    if event.amount == 0 {
        return false;
    }
    let mut ids = world.query::<(Entity, &SimId, Option<&Position>)>();
    let target = ids
        .iter(world)
        .find(|(_, id, _)| **id == event.target)
        .map(|(e, _, p)| (e, p.map(|p| p.0)));
    let Some((entity, feet)) = target else {
        return false;
    };
    let source_x = event.source.and_then(|source| {
        ids.iter(world)
            .find(|(_, id, _)| **id == source)
            .and_then(|(_, _, p)| p.map(|p| p.0.x))
    });
    let x = match (feet, source_x) {
        (Some(p), Some(x)) if p.x < x => -rules.knockback_x,
        (Some(p), Some(x)) if p.x > x => rules.knockback_x,
        _ => 0.0,
    };
    let knockback = [x, -rules.gravity_dir() * rules.knockback_y];
    let Some(health) = world.get::<Health>(entity) else {
        return false;
    };
    if health.current == 0 || health.invincible_ticks > 0 {
        return false;
    }
    // Level scripts may change or cancel a hit that would land (`on_damage`). Only where
    // scripts run (server, solo); a 0 amount cancels it entirely (no stun, no knockback).
    let event = DamageEvent {
        amount: crate::scripting::filter_damage(world, &event),
        ..event
    };
    if event.amount == 0 {
        return false;
    }
    let Some(mut health) = world.get_mut::<Health>(entity) else {
        return false; // a script despawned the target
    };
    health.current = health.current.saturating_sub(event.amount);
    health.invincible_ticks = seconds_to_ticks(rules.damage_invincibility);
    health.stun_ticks = seconds_to_ticks(rules.hit_stun);
    health.knockback = knockback;
    let dead = health.current == 0;
    cancel_mobility(world, entity);
    if let Some(mut control) = world.get_mut::<PlayerControl>(entity) {
        control.jumping = false;
        control.buffer_ticks = 0;
        control.coyote_ticks = 0;
    }
    if let Some(mut velocity) = world.get_mut::<Velocity>(entity) {
        velocity.0 = if dead {
            Vec2::ZERO
        } else {
            Vec2::from_array(knockback)
        };
    }
    let mut events = world.resource_mut::<CombatEvents>();
    events.damage.push(event);
    if dead {
        events.deaths.push(DeathEvent {
            target: event.target,
            source: event.source,
        });
    }
    if dead {
        world.entity_mut(entity).remove::<Hitbox>();
    }
    true
}

/// A hit or pogo ends the active dash/wall lock without recharging spent abilities.
fn cancel_mobility(world: &mut World, entity: Entity) {
    if let Some(mut state) = world.get_mut::<crate::mechanics::MobilityState>(entity) {
        state.dash_ticks = 0;
        state.wall_lock_ticks = 0;
    }
}

fn hazard_overlap(world: &World, attack: Aabb) -> bool {
    let Some(level) = world.get_resource::<LoadedLevel>().map(|l| &l.0) else {
        return false;
    };
    let ts = level.tile_size as f32;
    let x0 = (attack.min.x / ts).floor().max(0.0) as u32;
    let y0 = (attack.min.y / ts).floor().max(0.0) as u32;
    let x1 = (attack.max.x / ts).ceil().max(0.0).min(level.size.w as f32) as u32;
    let y1 = (attack.max.y / ts).ceil().max(0.0).min(level.size.h as f32) as u32;
    level
        .layers
        .iter()
        .filter(|l| l.kind == LayerKind::Hazard)
        .any(|layer| {
            (y0..y1).any(|y| {
                (x0..x1).any(|x| {
                    layer.tile_at(x, y).is_some()
                        && overlap(
                            attack,
                            Aabb {
                                min: Vec2::new(x as f32 * ts, y as f32 * ts),
                                max: Vec2::new((x + 1) as f32 * ts, (y + 1) as f32 * ts),
                            },
                        )
                })
            })
        })
}

/// Combat state carried by body snapshots, including the targets already struck by a swing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombatSnapshot {
    pub health: Option<Health>,
    pub hurtbox: Option<Hurtbox>,
    pub melee: Option<MeleeState>,
    pub hitbox: Option<Hitbox>,
    pub pogo_surface: bool,
}
impl CombatSnapshot {
    /// Makes `entity`'s combat components match `snapshot` exactly (inserting or removing
    /// each one), for [`crate::Snapshot::restore`].
    pub(crate) fn restore(snapshot: Option<&Self>, entity: &mut EntityWorldMut) {
        let empty = Self {
            health: None,
            hurtbox: None,
            melee: None,
            hitbox: None,
            pogo_surface: false,
        };
        let s = snapshot.unwrap_or(&empty);
        match s.health {
            Some(c) => entity.insert(c),
            None => entity.remove::<Health>(),
        };
        match s.hurtbox {
            Some(c) => entity.insert(c),
            None => entity.remove::<Hurtbox>(),
        };
        match s.melee {
            Some(c) => entity.insert(c),
            None => entity.remove::<MeleeState>(),
        };
        match &s.hitbox {
            Some(c) => entity.insert(c.clone()),
            None => entity.remove::<Hitbox>(),
        };
        if s.pogo_surface {
            entity.insert(PogoSurface);
        } else {
            entity.remove::<PogoSurface>();
        }
    }

    pub(crate) fn capture(world: &World, entity: Entity) -> Option<Self> {
        let value = Self {
            health: world.get::<Health>(entity).copied(),
            hurtbox: world.get::<Hurtbox>(entity).copied(),
            melee: world.get::<MeleeState>(entity).copied(),
            hitbox: world.get::<Hitbox>(entity).cloned(),
            pogo_surface: world.get::<PogoSurface>(entity).is_some(),
        };
        (value.health.is_some()
            || value.hurtbox.is_some()
            || value.melee.is_some()
            || value.hitbox.is_some()
            || value.pogo_surface)
            .then_some(value)
    }
}
