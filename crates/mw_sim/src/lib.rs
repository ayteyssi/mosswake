//! Headless, deterministic gameplay simulation (physics, players, enemies, mechanics).
//!
//! [`SimPlugin`] runs in Bevy's `FixedUpdate` at [`TICK_HZ`] with no window
//! (`MinimalPlugins` on the server, the full client app for prediction). Systems never read
//! wall-clock time: they advance by exactly [`TICK_DT`] per tick, so the same level, rules
//! and inputs always produce the same state. [`SimHarness`] steps it directly for tests.
//!
//! See `docs/ARCHITECTURE.md` (Simulation) for the boundaries of this crate.

use std::collections::BTreeMap;

use bevy::prelude::*;
use mw_core::input::{PlayerId, PlayerInput};
use mw_level::{Level, ValidationError};
use serde::{Deserialize, Serialize};

pub mod combat;
mod grid;
mod harness;
pub mod mechanics;
mod physics;
mod player;
mod rules;
mod snapshot;

pub use grid::{Aabb, CollisionGrid, SweepResult};
pub use harness::{BodyState, SimHarness};
pub use physics::{Body, Contacts, Position, Velocity};
pub use player::{
    despawn_player, seconds_to_ticks, spawn_player, Facing, Player, PlayerControl, PlayerState,
    JUMP, MOVEMENT,
};
pub use rules::{mechanic_active, KnownMechanics, RegisterMechanic, Rules, RulesError};
pub use snapshot::{BodySnapshot, PlayerSnapshot, Snapshot};

/// Simulation ticks per second.
pub const TICK_HZ: f64 = 60.0;

/// Duration of one tick, seconds. Every system integrates with this constant.
pub const TICK_DT: f32 = 1.0 / TICK_HZ as f32;

/// Order of the simulation stages inside `FixedUpdate` (see docs/ARCHITECTURE.md).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Read this tick's [`TickInputs`].
    Input,
    /// Script hooks that run before mechanics (`on_tick`, `on_player_input`).
    ScriptPre,
    /// Activable mechanics (movement, jump, dash…) set velocities and intents.
    Mechanics,
    /// Gravity and movement against the tile grid.
    Physics,
    /// Contact flags, then entity-vs-entity overlaps (damage, triggers).
    Collisions,
    /// Script hooks that react to this tick's events.
    ScriptPost,
    /// Despawns, tick counter.
    Cleanup,
}

/// Number of ticks simulated since the level was loaded.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tick(pub u64);

/// Inputs of every player for the tick being simulated. Ordered by player for determinism.
#[derive(Resource, Debug, Clone, PartialEq, Eq, Default)]
pub struct TickInputs(pub BTreeMap<PlayerId, PlayerInput>);

/// Stable identity of a simulated entity, identical on every machine running the same
/// session (unlike Bevy's `Entity`). Allocated in spawn order by [`SimIds`].
#[derive(
    Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SimId(pub u32);

/// Allocator of [`SimId`]s.
#[derive(Resource, Debug, Default)]
pub struct SimIds {
    next: u32,
}

impl SimIds {
    pub fn allocate(&mut self) -> SimId {
        let id = SimId(self.next);
        self.next += 1;
        id
    }
}

/// The level currently simulated.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct LoadedLevel(pub Level);

/// Why a level could not be loaded into the simulation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SimLoadError {
    #[error("invalid level: {0:?}")]
    InvalidLevel(Vec<ValidationError>),
    #[error("invalid level rules: {0:?}")]
    InvalidRules(Vec<RulesError>),
}

/// The simulation: resources, ordered stages and the core systems.
pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
            .init_resource::<Rules>()
            .init_resource::<KnownMechanics>()
            .init_resource::<Tick>()
            .init_resource::<TickInputs>()
            .init_resource::<SimIds>()
            .configure_sets(
                FixedUpdate,
                (
                    SimSet::Input,
                    SimSet::ScriptPre,
                    SimSet::Mechanics,
                    SimSet::Physics,
                    SimSet::Collisions,
                    SimSet::ScriptPost,
                    SimSet::Cleanup,
                )
                    .chain(),
            )
            .add_systems(FixedUpdate, advance_tick.in_set(SimSet::Cleanup))
            .add_plugins((
                physics::plugin,
                player::plugin,
                mechanics::plugin,
                combat::plugin,
            ));
    }
}

fn advance_tick(mut tick: ResMut<Tick>) {
    tick.0 += 1;
}

/// Converts a level position (tiles) to world units.
pub fn tiles_to_world(level: &Level, x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y) * level.tile_size as f32
}

/// Loads `level` into `world`: validates it, builds [`Rules`] from the defaults plus the
/// level's overrides (checked against [`KnownMechanics`]), and inserts the collision grid.
/// Mechanic plugins must be added before calling this.
pub fn load_level(world: &mut World, level: &Level) -> Result<(), SimLoadError> {
    let errors = level.validate();
    if !errors.is_empty() {
        return Err(SimLoadError::InvalidLevel(errors));
    }
    let rules = Rules::default()
        .with_overrides(&level.rules)
        .map_err(SimLoadError::InvalidRules)?;
    let known = world
        .get_resource::<KnownMechanics>()
        .cloned()
        .unwrap_or_default();
    let errors = rules.validate(&known);
    if !errors.is_empty() {
        return Err(SimLoadError::InvalidRules(errors));
    }
    world.insert_resource(rules);
    world.insert_resource(CollisionGrid::from_level(level));
    world.insert_resource(LoadedLevel(level.clone()));
    world.insert_resource(Tick::default());
    Ok(())
}
