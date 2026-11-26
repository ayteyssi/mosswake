//! [`Snapshot`]: the authoritative world state the server sends to clients.
//!
//! It holds everything a client needs to draw the world and to re-simulate its own player
//! from a server state (prediction and reconciliation, #14). It is serializable with serde;
//! the network layer chooses the encoding. **Its encoded form is part of the network
//! protocol**: changing these types bumps `mw_net::PROTOCOL_VERSION`.

use bevy::prelude::*;
use mw_core::input::PlayerId;
use serde::{Deserialize, Serialize};

use crate::{
    Contacts, Facing, Player, PlayerControl, PlayerState, Position, SimId, Tick, Velocity,
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BodySnapshot {
    pub id: SimId,
    pub position: [f32; 2],
    pub velocity: [f32; 2],
    pub contacts: Contacts,
    /// Present when the body is a player.
    pub player: Option<PlayerSnapshot>,
}

/// Player-specific state of a body.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub player: PlayerId,
    pub state: PlayerState,
    pub facing: Facing,
    /// Inputs and jump timers, needed to re-simulate the player exactly.
    pub control: PlayerControl,
}

impl Snapshot {
    /// Captures the current state of `world`.
    pub fn capture(world: &mut World) -> Self {
        let tick = world.get_resource::<Tick>().copied().unwrap_or_default().0;
        let mut query = world.query::<(
            &SimId,
            &Position,
            &Velocity,
            &Contacts,
            Option<(&Player, &PlayerState, &Facing, &PlayerControl)>,
        )>();
        let mut bodies: Vec<BodySnapshot> = query
            .iter(world)
            .map(|(id, position, velocity, contacts, player)| BodySnapshot {
                id: *id,
                position: position.0.to_array(),
                velocity: velocity.0.to_array(),
                contacts: *contacts,
                player: player.map(|(player, state, facing, control)| PlayerSnapshot {
                    player: player.0,
                    state: *state,
                    facing: *facing,
                    control: *control,
                }),
            })
            .collect();
        bodies.sort_by_key(|b| b.id);
        Self { tick, bodies }
    }

    /// The body of `player`, if it is in the snapshot.
    pub fn player(&self, player: PlayerId) -> Option<&BodySnapshot> {
        self.bodies
            .iter()
            .find(|b| b.player.is_some_and(|p| p.player == player))
    }
}
