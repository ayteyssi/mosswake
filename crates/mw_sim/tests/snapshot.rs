//! Snapshots and player despawn, as used by the game server.

use bevy::math::Vec2;
use mw_core::input::{PlayerId, PlayerInput};
use mw_level::{Layer, LayerKind, Level, Size, Spawn};
use mw_sim::{Body, PlayerState, SimHarness, TickInputs};

/// 10 x 4 room with a floor, two spawns.
fn room() -> Level {
    let mut rows = vec!["#".repeat(10)];
    for _ in 0..2 {
        rows.push(format!("#{}#", ".".repeat(8)));
    }
    rows.push("#".repeat(10));
    let mut level = Level::new("room", "Room", Size { w: 10, h: 4 });
    level.legend.insert('#', "mossy_ground".into());
    level.layers.push(Layer {
        name: "ground".into(),
        kind: LayerKind::Solid,
        tiles: rows,
    });
    level.spawns = vec![Spawn { x: 2.5, y: 1.0 }, Spawn { x: 7.5, y: 1.0 }];
    level
}

fn run_right(player: u8) -> TickInputs {
    let mut inputs = TickInputs::default();
    inputs.0.insert(
        PlayerId(player),
        PlayerInput {
            axis_x: 127,
            ..Default::default()
        },
    );
    inputs
}

#[test]
fn snapshot_holds_tick_and_every_body_by_id() {
    let mut sim = SimHarness::new(&room()).unwrap();
    // Spawn order differs from player order on purpose.
    let b = sim.spawn_player(PlayerId(1));
    let crate_body = sim.spawn_body(
        Vec2::new(160.0, 32.0),
        Body {
            size: Vec2::splat(16.0),
            gravity_scale: 1.0,
        },
        Vec2::ZERO,
    );
    let a = sim.spawn_player(PlayerId(0));
    sim.step(10, &run_right(0));

    let snap = sim.snapshot();
    assert_eq!(snap.tick, 10);
    let ids: Vec<_> = snap.bodies.iter().map(|body| body.id).collect();
    assert_eq!(ids, [b, crate_body, a], "ordered by SimId");
    assert!(snap.bodies[1].player.is_none(), "not a player");

    let p0 = snap.player(PlayerId(0)).expect("player 0 in snapshot");
    let state = sim.body(a);
    assert_eq!(p0.position, state.position.to_array());
    assert_eq!(p0.velocity, state.velocity.to_array());
    assert_eq!(p0.contacts, state.contacts);
    let player = p0.player.expect("player data");
    assert_eq!(player.state, PlayerState::Run);
    assert_eq!(player.control.input.axis_x, 127, "inputs travel with it");
    assert!(snap.player(PlayerId(5)).is_none());
}

#[test]
fn despawn_player_removes_only_that_player() {
    let mut sim = SimHarness::new(&room()).unwrap();
    let a = sim.spawn_player(PlayerId(0));
    let b = sim.spawn_player(PlayerId(1));
    assert_eq!(sim.despawn_player(PlayerId(0)), [a]);
    assert_eq!(sim.despawn_player(PlayerId(0)), [], "already gone");
    sim.step(1, &TickInputs::default());
    let snap = sim.snapshot();
    assert_eq!(snap.bodies.len(), 1);
    assert_eq!(snap.bodies[0].id, b);
}

#[test]
fn snapshots_are_deterministic() {
    let run = || {
        let mut sim = SimHarness::new(&room()).unwrap();
        sim.spawn_player(PlayerId(0));
        sim.spawn_player(PlayerId(1));
        let mut snaps = Vec::new();
        for _ in 0..30 {
            sim.step(2, &run_right(1));
            snaps.push(sim.snapshot());
        }
        snaps
    };
    assert_eq!(run(), run());
}
