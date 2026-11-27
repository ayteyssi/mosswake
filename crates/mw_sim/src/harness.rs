//! [`SimHarness`]: runs the simulation tick by tick without a window or a clock. Used by
//! tests (here and in other crates), tools such as the MCP `simulate_level`, and the game
//! server loop, which decides itself when a tick is due.

use bevy::prelude::*;
use mw_core::input::PlayerId;
use mw_level::Level;

use crate::{
    load_level, Body, Contacts, Facing, PlayerState, Position, SimId, SimIds, SimLoadError,
    SimPlugin, Snapshot, Tick, TickInputs, Velocity,
};

/// A loaded simulation advanced explicitly with [`SimHarness::step`].
pub struct SimHarness {
    app: App,
}

/// State of one body, as compared by determinism tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyState {
    pub id: SimId,
    pub position: Vec2,
    pub velocity: Vec2,
    pub contacts: Contacts,
}

impl SimHarness {
    /// A simulation of `level` with only the core plugin.
    pub fn new(level: &Level) -> Result<Self, SimLoadError> {
        Self::with_setup(level, |_| {})
    }

    /// Like [`SimHarness::new`], with `setup` adding plugins (mechanics…) before the level
    /// is loaded, so the level may activate their mechanics.
    pub fn with_setup(level: &Level, setup: impl FnOnce(&mut App)) -> Result<Self, SimLoadError> {
        let mut app = App::new();
        app.add_plugins(SimPlugin);
        setup(&mut app);
        app.finish();
        app.cleanup();
        load_level(app.world_mut(), level)?;
        Ok(Self { app })
    }

    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn world(&self) -> &World {
        self.app.world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.app.world_mut()
    }

    /// Spawns a body with its feet at `feet` (world units) and returns its id.
    pub fn spawn_body(&mut self, feet: Vec2, body: Body, velocity: Vec2) -> SimId {
        let world = self.app.world_mut();
        let id = world.resource_mut::<SimIds>().allocate();
        world.spawn((id, body, Position(feet), Velocity(velocity)));
        id
    }

    /// Spawns `player` on its level spawn point (see [`crate::spawn_player`]).
    pub fn spawn_player(&mut self, player: PlayerId) -> SimId {
        crate::spawn_player(self.app.world_mut(), player)
    }

    /// Despawns the bodies of `player` and returns their ids.
    pub fn despawn_player(&mut self, player: PlayerId) -> Vec<SimId> {
        crate::despawn_player(self.app.world_mut(), player)
    }

    /// The current world state, as sent to clients.
    pub fn snapshot(&mut self) -> Snapshot {
        Snapshot::capture(self.app.world_mut())
    }

    /// Replaces the simulation state with `snapshot` (see [`Snapshot::restore`]).
    pub fn restore(&mut self, snapshot: &Snapshot) {
        snapshot.restore(self.app.world_mut());
    }

    /// Runs one tick per element of `script`, each with its inputs, and returns the body
    /// state of `id` after every tick (handy to assert on a whole trajectory).
    pub fn run_script(&mut self, id: SimId, script: &[TickInputs]) -> Vec<BodyState> {
        script
            .iter()
            .map(|inputs| {
                self.step(1, inputs);
                self.body(id)
            })
            .collect()
    }

    /// Animation state and facing of the player body `id`.
    pub fn player_state(&mut self, id: SimId) -> (PlayerState, Facing) {
        let mut query = self
            .app
            .world_mut()
            .query::<(&SimId, &PlayerState, &Facing)>();
        query
            .iter(self.app.world())
            .find(|(sim_id, _, _)| **sim_id == id)
            .map(|(_, state, facing)| (*state, *facing))
            .unwrap_or_else(|| panic!("no player {id:?}"))
    }

    /// Moves body `id` so its feet are at `feet` (world units) and stops it. Tests and
    /// tools only: the simulation itself never teleports.
    pub fn teleport(&mut self, id: SimId, feet: Vec2) {
        let mut query = self
            .app
            .world_mut()
            .query::<(&SimId, &mut Position, &mut Velocity)>();
        for (sim_id, mut position, mut velocity) in query.iter_mut(self.app.world_mut()) {
            if *sim_id == id {
                position.0 = feet;
                velocity.0 = Vec2::ZERO;
            }
        }
    }

    /// Runs `ticks` simulation ticks with the same `inputs` for each.
    pub fn step(&mut self, ticks: u32, inputs: &TickInputs) {
        for _ in 0..ticks {
            self.app.world_mut().insert_resource(inputs.clone());
            self.app.world_mut().run_schedule(FixedUpdate);
        }
    }

    /// Ticks simulated since the level was loaded.
    pub fn tick(&self) -> u64 {
        self.world().resource::<Tick>().0
    }

    /// Every body, ordered by id.
    pub fn bodies(&mut self) -> Vec<BodyState> {
        let mut query = self
            .app
            .world_mut()
            .query::<(&SimId, &Position, &Velocity, &Contacts)>();
        let mut states: Vec<BodyState> = query
            .iter(self.app.world())
            .map(|(id, p, v, c)| BodyState {
                id: *id,
                position: p.0,
                velocity: v.0,
                contacts: *c,
            })
            .collect();
        states.sort_by_key(|s| s.id);
        states
    }

    /// State of body `id`. Panics if it does not exist (test helper).
    pub fn body(&mut self, id: SimId) -> BodyState {
        self.bodies()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no body {id:?}"))
    }
}
