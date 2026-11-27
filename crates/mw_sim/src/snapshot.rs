//! [`Snapshot`]: the authoritative world state the server sends to clients.
//!
//! It holds everything a client needs to draw the world and to re-simulate its own player
//! from a server state (prediction and reconciliation, #14). It is serializable with serde;
//! the network layer chooses the encoding. **Its encoded form is part of the network
//! protocol**: changing these types bumps `mw_net::PROTOCOL_VERSION`.

use std::collections::BTreeMap;

use bevy::prelude::*;
use mw_core::input::PlayerId;
use serde::{Deserialize, Serialize};

use crate::mechanics::MobilityState;

use crate::{
    combat::CombatSnapshot, Body, Contacts, Facing, Player, PlayerControl, PlayerState, Position,
    Rules, SimId, SimIds, Tick, Velocity,
};

/// The state of the simulation after [`Snapshot::tick`] ticks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Number of ticks simulated when the snapshot was taken (see [`Tick`]).
    pub tick: u64,
    /// Every simulated body, ordered by [`SimId`].
    pub bodies: Vec<BodySnapshot>,
}

/// One body. Positions are feet, world units, y up (see [`Position`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BodySnapshot {
    pub id: SimId,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub contacts: Contacts,
    /// Present when the body is a player.
    pub player: Option<PlayerSnapshot>,
    pub combat: Option<CombatSnapshot>,
}

/// Player-specific state of a body.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub player: PlayerId,
    pub state: PlayerState,
    pub facing: Facing,
    /// Inputs and jump timers, needed to re-simulate the player exactly.
    pub control: PlayerControl,
    /// Dash, air-jump and wall-jump state needed for exact replay.
    pub mobility: MobilityState,
}

impl Snapshot {
    /// Captures the current state of `world`.
    pub fn capture(world: &mut World) -> Self {
        let tick = world.get_resource::<Tick>().copied().unwrap_or_default().0;
        let mut query = world.query::<(
            Entity,
            &SimId,
            &Position,
            &Velocity,
            &Contacts,
            Option<(
                &Player,
                &PlayerState,
                &Facing,
                &PlayerControl,
                &MobilityState,
            )>,
        )>();
        let mut bodies: Vec<BodySnapshot> = query
            .iter(world)
            .map(
                |(entity, id, position, velocity, contacts, player)| BodySnapshot {
                    combat: CombatSnapshot::capture(world, entity),
                    id: *id,
                    position: position.0.to_array(),
                    velocity: velocity.0.to_array(),
                    contacts: *contacts,
                    player: player.map(|(player, state, facing, control, mobility)| {
                        PlayerSnapshot {
                            player: player.0,
                            state: *state,
                            facing: *facing,
                            control: *control,
                            mobility: *mobility,
                        }
                    }),
                },
            )
            .collect();
        bodies.sort_by_key(|b| b.id);
        Self { tick, bodies }
    }

    /// Overwrites the simulation in `world` with this snapshot: bodies missing from it are
    /// despawned, players missing from `world` are spawned, every recorded component is set,
    /// and [`Tick`] becomes [`Snapshot::tick`]. Stepping the restored world with the same
    /// inputs as the server then gives bit-identical states: the basis of client prediction
    /// and reconciliation.
    ///
    /// Non-player bodies unknown to `world` are skipped (their shape is not in the snapshot);
    /// none exist yet.
    pub fn restore(&self, world: &mut World) {
        // What the snapshot says each id is: a given player, or a non-player body.
        let wanted: BTreeMap<SimId, Option<PlayerId>> = self
            .bodies
            .iter()
            .map(|b| (b.id, b.player.map(|p| p.player)))
            .collect();
        let mut query = world.query::<(Entity, &SimId, Option<&Player>)>();
        let mut existing = BTreeMap::new();
        let mut stale = Vec::new();
        for (entity, id, player) in query.iter(world) {
            // Keep an entity only if the snapshot has the same kind of body under its id.
            if wanted.get(id) == Some(&player.map(|p| p.0)) {
                existing.insert(*id, entity);
            } else {
                stale.push(entity);
            }
        }
        for entity in stale {
            world.despawn(entity);
        }
        for body in &self.bodies {
            let entity = match (existing.get(&body.id), body.player) {
                (Some(entity), _) => *entity,
                (None, Some(player)) => {
                    let rules = world.resource::<Rules>();
                    let size = Vec2::new(rules.player_width, rules.player_height);
                    world
                        .spawn((
                            body.id,
                            Player(player.player),
                            Body {
                                size,
                                gravity_scale: 1.0,
                            },
                        ))
                        .id()
                }
                (None, None) => continue,
            };
            let mut entity = world.entity_mut(entity);
            entity.insert((
                Position(Vec2::from_array(body.position)),
                Velocity(Vec2::from_array(body.velocity)),
                body.contacts,
            ));
            if let Some(p) = body.player {
                entity.insert((p.state, p.facing, p.control, p.mobility));
            }
            CombatSnapshot::restore(body.combat.as_ref(), &mut entity);
        }
        if let Some(last) = self.bodies.iter().map(|b| b.id).max() {
            world.resource_mut::<SimIds>().reserve_through(last);
        }
        world.insert_resource(Tick(self.tick));
    }

    /// The body of `player`, if it is in the snapshot.
    pub fn player(&self, player: PlayerId) -> Option<&BodySnapshot> {
        self.bodies
            .iter()
            .find(|b| b.player.is_some_and(|p| p.player == player))
    }
}
