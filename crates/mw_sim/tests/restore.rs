//! Snapshot::restore: a client simulation restored from a server snapshot and fed the same
//! inputs stays bit-identical to the server (the basis of prediction and reconciliation).

use std::path::PathBuf;

use bevy::math::Vec2;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::LevelBundle;
use mw_sim::{Body, SimHarness, TickInputs};

fn level() -> mw_level::Level {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels/tutorial");
    let mut level = LevelBundle::load(dir).unwrap().level;
    level.rules.insert(
        "mechanics".into(),
        serde_json::json!(["movement", "jump", "dash"]),
    );
    level
}

/// Varied, deterministic inputs for two players.
fn inputs(tick: u64) -> TickInputs {
    let mut inputs = TickInputs::default();
    for p in 0..2u8 {
        let phase = tick + u64::from(p) * 17;
        let mut pressed = 0;
        if phase % 40 < 12 {
            pressed |= buttons::JUMP;
        }
        if phase % 97 == 50 {
            pressed |= buttons::DASH;
        }
        let axis_x = if (phase / 60).is_multiple_of(2) { 127 } else { -90 };
        inputs.0.insert(
            PlayerId(p),
            PlayerInput {
                axis_x,
                axis_y: 0,
                buttons: pressed,
            },
        );
    }
    inputs
}

fn run(sim: &mut SimHarness, from: u64, ticks: u64) {
    for t in from..from + ticks {
        sim.step(1, &inputs(t));
    }
}

#[test]
fn restored_client_stays_bit_identical_to_the_server() {
    let level = level();
    let mut server = SimHarness::new(&level).unwrap();
    server.spawn_player(PlayerId(0));
    server.spawn_player(PlayerId(1));
    run(&mut server, 0, 120);
    let snapshot = server.snapshot();

    // A client with a different past: no players, and a stray body.
    let mut client = SimHarness::new(&level).unwrap();
    client.spawn_body(
        Vec2::new(100.0, 100.0),
        Body {
            size: Vec2::splat(10.0),
            gravity_scale: 1.0,
        },
        Vec2::ZERO,
    );
    client.step(7, &TickInputs::default());

    client.restore(&snapshot);
    assert_eq!(
        client.snapshot(),
        snapshot,
        "restore reproduces the snapshot exactly"
    );
    assert_eq!(client.tick(), server.tick());

    run(&mut server, 120, 200);
    run(&mut client, 120, 200);
    assert_eq!(
        client.snapshot(),
        server.snapshot(),
        "same inputs, same future"
    );
}

#[test]
fn restoring_twice_and_rewinding_is_harmless() {
    let level = level();
    let mut server = SimHarness::new(&level).unwrap();
    server.spawn_player(PlayerId(0));
    run(&mut server, 0, 30);
    let early = server.snapshot();
    run(&mut server, 30, 30);
    let late = server.snapshot();

    let mut client = SimHarness::new(&level).unwrap();
    client.restore(&late);
    client.restore(&early); // going back in time (reconciliation does this)
    assert_eq!(client.snapshot(), early);
    run(&mut client, 30, 30);
    assert_eq!(client.snapshot(), late);
}

#[test]
fn players_gone_from_the_snapshot_are_removed() {
    let level = level();
    let mut server = SimHarness::new(&level).unwrap();
    server.spawn_player(PlayerId(0));
    let mut client = SimHarness::new(&level).unwrap();
    client.restore(&server.snapshot());
    assert_eq!(client.snapshot().bodies.len(), 1);
    server.despawn_player(PlayerId(0));
    client.restore(&server.snapshot());
    assert!(client.snapshot().bodies.is_empty());
    // New ids never collide with restored ones.
    let id = client.spawn_player(PlayerId(3));
    assert!(id.0 >= 1, "{id:?}");
}
