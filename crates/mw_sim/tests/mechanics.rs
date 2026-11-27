//! Optional movement abilities exercised through the real headless simulation.

use bevy::math::Vec2;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::{Layer, LayerKind, Level, Size, Spawn};
use mw_sim::{
    mechanics::MobilityState, seconds_to_ticks, Rules, SimHarness, SimId, TickInputs, TICK_DT,
};

fn room(mechanics: &[&str]) -> Level {
    let mut level = Level::new("abilities", "Abilities", Size { w: 80, h: 40 });
    level.legend.insert('#', "mossy_ground".into());
    let mut tiles = vec!["#".repeat(80)];
    tiles.extend((0..38).map(|_| format!("#{}#", ".".repeat(78))));
    tiles.push("#".repeat(80));
    level.layers.push(Layer {
        name: "walls".into(),
        kind: LayerKind::Solid,
        tiles,
    });
    level.spawns = vec![
        Spawn { x: 20.0, y: 1.0 },
        Spawn { x: 78.625, y: 20.0 },
        Spawn { x: 1.375, y: 20.0 },
        Spawn { x: 20.0, y: 20.0 },
    ];
    level
        .rules
        .insert("mechanics".into(), serde_json::json!(mechanics));
    level
}

fn spawn(level: &Level, player: u8) -> (SimHarness, SimId) {
    let mut sim = SimHarness::new(level).unwrap();
    let id = sim.spawn_player(PlayerId(player));
    sim.step(1, &input(0, 0)); // establish contacts
    (sim, id)
}

fn input(x: i8, held: u8) -> TickInputs {
    TickInputs(
        (0..4)
            .map(|p| {
                (
                    PlayerId(p),
                    PlayerInput {
                        axis_x: x,
                        buttons: held,
                        ..Default::default()
                    },
                )
            })
            .collect(),
    )
}

fn mobility(sim: &mut SimHarness) -> MobilityState {
    sim.snapshot().bodies[0].player.unwrap().mobility
}

#[test]
fn dash_has_exact_duration_and_distance_without_gravity() {
    for gravity in [-1800.0, 1800.0] {
        let mut level = room(&["movement", "jump", "dash"]);
        level
            .rules
            .insert("gravity".into(), serde_json::json!(gravity));
        let (mut sim, id) = spawn(&level, 3);
        let rules = sim.world().resource::<Rules>().clone();
        let start = sim.body(id).position;
        let duration = seconds_to_ticks(rules.dash_duration);
        for _ in 0..duration {
            sim.step(1, &input(127, buttons::DASH));
            assert_eq!(sim.body(id).velocity, Vec2::new(rules.dash_speed, 0.0));
        }
        let end = sim.body(id).position;
        assert_eq!(end.y, start.y, "gravity is suspended for the whole dash");
        assert!((end.x - start.x - rules.dash_speed * TICK_DT * duration as f32).abs() < 0.01);
        sim.step(1, &input(127, 0));
        assert_eq!(mobility(&mut sim).dash_ticks, 0);
        assert_eq!(
            sim.body(id).velocity.x,
            rules.run_speed,
            "no long high-speed coast"
        );
        assert!(sim.body(id).velocity.y * gravity > 0.0, "gravity resumes");
    }
}

#[test]
fn dash_uses_facing_when_idle_and_needs_a_new_press() {
    let (mut sim, id) = spawn(&room(&["movement", "dash"]), 0);
    let rules = sim.world().resource::<Rules>().clone();
    sim.step(1, &input(-127, 0));
    sim.step(1, &input(0, buttons::DASH));
    assert_eq!(sim.body(id).velocity.x, -rules.dash_speed);
    sim.step(
        seconds_to_ticks(rules.dash_cooldown) + 3,
        &input(0, buttons::DASH),
    );
    assert_eq!(
        mobility(&mut sim).dash_ticks,
        0,
        "holding does not retrigger"
    );
    sim.step(1, &input(0, 0));
    sim.step(1, &input(0, buttons::DASH));
    assert!(
        mobility(&mut sim).dash_ticks > 0,
        "a fresh grounded press works"
    );
}

#[test]
fn dash_is_once_in_air_and_landing_recharges_it() {
    let (mut sim, id) = spawn(&room(&["dash"]), 3);
    let cooldown = seconds_to_ticks(Rules::default().dash_cooldown);
    sim.step(1, &input(127, buttons::DASH));
    sim.step(cooldown + 1, &input(0, 0));
    assert!(!sim.body(id).contacts.grounded);
    sim.step(1, &input(127, buttons::DASH));
    assert_eq!(
        mobility(&mut sim).dash_ticks,
        0,
        "cooldown alone does not recharge in air"
    );
    sim.step(100, &input(0, 0));
    assert!(sim.body(id).contacts.grounded);
    sim.step(1, &input(127, buttons::DASH));
    assert!(
        mobility(&mut sim).dash_ticks > 0,
        "landing restored the charge"
    );
}

#[test]
fn dash_cooldown_is_enforced_on_the_ground() {
    let (mut sim, _) = spawn(&room(&["dash"]), 0);
    let rules = Rules::default();
    sim.step(1, &input(127, buttons::DASH));
    sim.step(seconds_to_ticks(rules.dash_duration), &input(0, 0));
    assert!(mobility(&mut sim).dash_available);
    sim.step(1, &input(127, buttons::DASH));
    assert_eq!(
        mobility(&mut sim).dash_ticks,
        0,
        "charge available, cooldown still running"
    );
    sim.step(seconds_to_ticks(rules.dash_cooldown), &input(0, 0));
    sim.step(1, &input(127, buttons::DASH));
    assert!(mobility(&mut sim).dash_ticks > 0);
}

#[test]
fn dash_disabled_does_not_change_the_trajectory_and_cannot_tunnel() {
    let run = |button| {
        let (mut sim, id) = spawn(&room(&["movement", "jump"]), 3);
        sim.step(20, &input(127, button));
        sim.body(id)
    };
    assert_eq!(run(0), run(buttons::DASH));

    let mut level = room(&["dash"]);
    level
        .rules
        .insert("dash_speed".into(), serde_json::json!(100000));
    let (mut sim, id) = spawn(&level, 1);
    sim.step(1, &input(127, buttons::DASH));
    let state = sim.body(id);
    assert_eq!(
        state.position.x,
        79.0 * 32.0 - Rules::default().player_width / 2.0
    );
    assert_eq!(state.velocity.x, 0.0, "wall stopped an extreme-speed dash");
}

#[test]
fn air_jump_count_is_configurable_and_restored_on_landing() {
    let mut level = room(&["jump", "double_jump"]);
    level.rules.insert("air_jumps".into(), serde_json::json!(2));
    let (mut sim, id) = spawn(&level, 0);
    sim.step(1, &input(0, buttons::JUMP));
    assert_eq!(
        mobility(&mut sim).air_jumps_used,
        0,
        "ground jump costs no air jump"
    );
    for n in 1..=2 {
        sim.step(8, &input(0, 0));
        sim.step(1, &input(0, buttons::JUMP));
        assert_eq!(mobility(&mut sim).air_jumps_used, n);
        assert!(sim.body(id).velocity.y > 0.0);
    }
    sim.step(8, &input(0, 0));
    let before = sim.body(id).velocity.y;
    sim.step(1, &input(0, buttons::JUMP));
    assert!(sim.body(id).velocity.y < before, "third air jump refused");
    sim.step(150, &input(0, 0));
    assert!(sim.body(id).contacts.grounded);
    assert_eq!(mobility(&mut sim).air_jumps_used, 0);
    sim.step(1, &input(0, buttons::JUMP));
    sim.step(8, &input(0, 0));
    sim.step(1, &input(0, buttons::JUMP));
    assert_eq!(mobility(&mut sim).air_jumps_used, 1, "air jumps recharged");
}

#[test]
fn air_jump_is_optional_bounded_and_requires_an_edge() {
    for (mechanics, count, succeeds) in [
        (vec!["jump"], 1, false),
        (vec!["jump", "double_jump"], 0, false),
        (vec!["jump", "double_jump"], 1, true),
        (vec!["double_jump"], 1, true),
    ] {
        let mut level = room(&mechanics);
        level
            .rules
            .insert("air_jumps".into(), serde_json::json!(count));
        let (mut sim, id) = spawn(&level, 3);
        sim.step(1, &input(0, buttons::JUMP));
        assert_eq!(sim.body(id).velocity.y > 0.0, succeeds);
        sim.step(60, &input(0, buttons::JUMP));
        assert_eq!(
            mobility(&mut sim).air_jumps_used,
            u32::from(succeeds),
            "held input never spends a second charge"
        );
    }
}

#[test]
fn coyote_jump_wins_over_air_jump_and_inverted_air_jump_goes_down() {
    let (mut sim, id) = spawn(&room(&["jump", "double_jump"]), 0);
    sim.teleport(id, Vec2::new(640.0, 200.0));
    sim.step(1, &input(0, 0)); // old ground contacts grant coyote time
    sim.step(1, &input(0, buttons::JUMP));
    assert!(sim.body(id).velocity.y > 0.0);
    assert_eq!(mobility(&mut sim).air_jumps_used, 0, "coyote jump was free");

    let mut level = room(&["double_jump"]);
    level
        .rules
        .insert("gravity".into(), serde_json::json!(1800));
    let (mut sim, id) = spawn(&level, 3);
    sim.step(1, &input(0, buttons::JUMP));
    assert!(sim.body(id).velocity.y < 0.0);
}

#[test]
fn wall_jump_kicks_away_from_either_wall_and_locks_horizontal_control() {
    let rules = Rules::default();
    for (player, axis, away) in [(1, 127, -1.0), (2, -127, 1.0)] {
        let (mut sim, id) = spawn(
            &room(&["movement", "jump", "wall_jump", "double_jump"]),
            player,
        );
        sim.step(1, &input(axis, buttons::JUMP));
        assert_eq!(sim.body(id).velocity.x, away * rules.wall_jump_speed_x);
        assert_eq!(
            sim.body(id).velocity.y,
            rules.wall_jump_speed_y + rules.gravity * TICK_DT
        );
        assert_eq!(
            mobility(&mut sim).air_jumps_used,
            0,
            "wall jump wins over air jump"
        );
        sim.step(
            seconds_to_ticks(rules.wall_jump_lock) - 1,
            &input(axis, buttons::JUMP),
        );
        assert_eq!(
            sim.body(id).velocity.x,
            away * rules.wall_jump_speed_x,
            "held input cannot erase the kick"
        );
        sim.step(1, &input(axis, buttons::JUMP));
        assert!(
            sim.body(id).velocity.x.abs() < rules.wall_jump_speed_x,
            "air control resumes"
        );
    }
}

#[test]
fn wall_slide_caps_speed_only_when_enabled_and_pushing_against_the_wall() {
    for (enabled, axis, capped) in [(true, 127, true), (false, 127, false), (true, 0, false)] {
        let mechanics = if enabled { vec!["wall_jump"] } else { vec![] };
        let (mut sim, id) = spawn(&room(&mechanics), 1);
        sim.step(15, &input(axis, 0));
        let falling = -sim.body(id).velocity.y;
        assert_eq!(
            falling <= Rules::default().wall_slide_speed,
            capped,
            "enabled={enabled}, axis={axis}, speed={falling}"
        );
        if capped {
            assert_eq!(falling, Rules::default().wall_slide_speed);
        }
        sim.step(1, &input(axis, buttons::JUMP));
        assert_eq!(
            sim.body(id).velocity.y > 0.0,
            enabled,
            "wall jump disabled means no kick"
        );
    }
}

#[test]
fn wall_slide_and_jump_follow_inverted_gravity() {
    let mut level = room(&["wall_jump"]);
    level
        .rules
        .insert("gravity".into(), serde_json::json!(1800));
    let (mut sim, id) = spawn(&level, 1);
    sim.step(15, &input(127, 0));
    assert_eq!(sim.body(id).velocity.y, Rules::default().wall_slide_speed);
    sim.step(1, &input(127, buttons::JUMP));
    assert!(sim.body(id).velocity.y < 0.0);
    assert!(sim.body(id).velocity.x < 0.0);
}

#[test]
fn combined_inputs_and_multiple_players_are_deterministic() {
    let run = || {
        let mut sim = SimHarness::new(&room(&[
            "movement",
            "jump",
            "dash",
            "double_jump",
            "wall_jump",
        ]))
        .unwrap();
        for p in 0..4 {
            sim.spawn_player(PlayerId(p));
        }
        let mut snapshots = Vec::new();
        for tick in 0..240 {
            let held = if tick % 23 < 4 { buttons::JUMP } else { 0 }
                | if tick % 31 < 3 { buttons::DASH } else { 0 };
            sim.step(1, &input(if tick % 50 < 25 { 127 } else { -127 }, held));
            snapshots.push(sim.snapshot());
        }
        snapshots
    };
    assert_eq!(run(), run(), "including mobility counters in snapshots");

    let (mut sim, id) = spawn(&room(&["jump", "dash", "double_jump", "wall_jump"]), 0);
    let start = sim.body(id).position.y;
    sim.step(1, &input(127, buttons::JUMP | buttons::DASH));
    assert_eq!(
        sim.body(id).velocity,
        Vec2::new(Rules::default().dash_speed, 0.0)
    );
    assert_eq!(
        sim.body(id).position.y,
        start,
        "dash wins simultaneous inputs"
    );
}

#[test]
fn disabling_abilities_at_runtime_removes_their_effects() {
    let (mut sim, id) = spawn(&room(&["dash", "double_jump", "wall_jump"]), 1);
    sim.step(1, &input(-127, buttons::DASH));
    assert!(mobility(&mut sim).dash_ticks > 0);
    sim.world_mut().resource_mut::<Rules>().mechanics.clear();
    sim.step(1, &input(0, 0));
    assert_eq!(mobility(&mut sim).dash_ticks, 0);
    assert!(sim.body(id).velocity.y < 0.0, "gravity no longer suspended");
    sim.step(1, &input(0, buttons::JUMP));
    assert!(sim.body(id).velocity.y < 0.0, "no air or wall jump");
}

#[test]
fn level_overrides_are_applied_and_invalid_values_are_rejected() {
    let mut level = room(&["dash", "wall_jump", "double_jump"]);
    for (key, value) in [
        ("dash_speed", 300.0),
        ("dash_duration", 0.05),
        ("dash_cooldown", 0.1),
        ("wall_slide_speed", 42.0),
        ("wall_jump_speed_x", 190.0),
        ("wall_jump_speed_y", 450.0),
        ("wall_jump_lock", 0.2),
    ] {
        level.rules.insert(key.into(), serde_json::json!(value));
    }
    let (mut sim, id) = spawn(&level, 3);
    sim.step(1, &input(127, buttons::DASH));
    assert_eq!(sim.body(id).velocity.x, 300.0);
    assert_eq!(mobility(&mut sim).dash_ticks, 3);
    assert_eq!(mobility(&mut sim).dash_cooldown_ticks, 6);

    for (key, value) in [
        ("dash_speed", -1.0),
        ("dash_duration", 0.0),
        ("dash_cooldown", -0.1),
        ("wall_slide_speed", -1.0),
        ("wall_jump_speed_x", 0.0),
        ("wall_jump_speed_y", 0.0),
        ("wall_jump_lock", -1.0),
        ("air_jumps", -1.0),
    ] {
        let mut invalid = room(&[]);
        invalid.rules.insert(key.into(), serde_json::json!(value));
        assert!(SimHarness::new(&invalid).is_err(), "invalid {key}");
    }
}

#[test]
fn snapshot_restores_mid_ability_state_for_exact_replay() {
    use mw_sim::{Contacts, Facing, PlayerControl, PlayerState, Position, Tick, Velocity};

    let level = room(&["movement", "jump", "dash", "double_jump", "wall_jump"]);
    for held in [buttons::DASH, buttons::JUMP] {
        let (mut server, _) = spawn(&level, 1);
        server.step(3, &input(-127, held));
        let checkpoint = server.snapshot();
        let body = &checkpoint.bodies[0];
        let player = body.player.unwrap();
        let (mut client, _) = spawn(&level, 1);
        let world = client.world_mut();
        world.resource_mut::<Tick>().0 = checkpoint.tick;
        let mut query = world.query::<(
            &mut Position,
            &mut Velocity,
            &mut Contacts,
            &mut PlayerControl,
            &mut MobilityState,
            &mut Facing,
            &mut PlayerState,
        )>();
        let (
            mut position,
            mut velocity,
            mut contacts,
            mut control,
            mut mobility,
            mut facing,
            mut state,
        ) = query.single_mut(world).unwrap();
        position.0 = Vec2::from_array(body.position);
        velocity.0 = Vec2::from_array(body.velocity);
        *contacts = body.contacts;
        *control = player.control;
        *mobility = player.mobility;
        *facing = player.facing;
        *state = player.state;
        for tick in 0..60 {
            let keys = if tick % 11 == 0 { buttons::JUMP } else { 0 };
            let inputs = input(127, keys);
            server.step(1, &inputs);
            client.step(1, &inputs);
            assert_eq!(
                server.snapshot(),
                client.snapshot(),
                "replay tick {tick}, held {held}"
            );
        }
    }
}

#[test]
fn dash_orients_the_player_even_without_the_movement_mechanic() {
    let (mut sim, id) = spawn(&room(&["dash"]), 3);
    sim.step(1, &input(-127, buttons::DASH));
    assert_eq!(sim.player_state(id).1, mw_sim::Facing::Left);
    sim.step(1, &input(127, buttons::DASH));
    assert!(
        sim.body(id).velocity.x < 0.0,
        "direction stays fixed during the dash"
    );
    assert_eq!(sim.player_state(id).1, mw_sim::Facing::Left);
}
