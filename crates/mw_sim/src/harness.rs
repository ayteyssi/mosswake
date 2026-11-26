//! [`SimHarness`]: runs the simulation tick by tick without a window or a clock, for tests
//! (here and in other crates) and for tools such as the MCP `simulate_level`.

use bevy::prelude::*;
use mw_level::Level;

use crate::{
    load_level, Body, Contacts, Position, SimId, SimIds, SimLoadError, SimPlugin, Tick, TickInputs,
    Velocity,
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
