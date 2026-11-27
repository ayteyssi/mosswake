//! One-way surfaces use the same swept physics and contact probes as solid terrain.

use bevy::math::Vec2;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::{Layer, LayerKind, Level, Size, Spawn};
use mw_sim::{Body, Rules, SimHarness, SimId, TickInputs, Velocity};

const TS: f32 = 32.0;
const SIZE: f32 = 16.0;

fn level() -> Level {
    let mut level = Level::new("platforms", "Platforms", Size { w: 12, h: 12 });
    level.legend.insert('#', "mossy_ground".into());
    level.legend.insert('=', "mossy_ground".into());
    let mut solid = vec!["............".into(); 12];
    solid[11] = "############".into();
    let mut platform = vec!["............".into(); 12];
    platform[12 - 1 - 4] = "....====....".into();
    level.layers = vec![
        Layer {
            name: "solid".into(),
            kind: LayerKind::Solid,
            tiles: solid,
        },
        Layer {
            name: "one_way".into(),
            kind: LayerKind::Platform,
            tiles: platform,
        },
    ];
    level.spawns = vec![Spawn { x: 5.5, y: 1.0 }];
    level
}

fn body(sim: &mut SimHarness, feet: Vec2, velocity: Vec2, gravity_scale: f32) -> SimId {
    sim.spawn_body(
        feet,
        Body {
            size: Vec2::splat(SIZE),
            gravity_scale,
        },
        velocity,
    )
}

fn set_velocity(sim: &mut SimHarness, id: SimId, velocity: Vec2) {
    let world = sim.world_mut();
    let mut query = world.query::<(&SimId, &mut Velocity)>();
    for (sim_id, mut v) in query.iter_mut(world) {
        if *sim_id == id {
            v.0 = velocity;
        }
    }
}

#[test]
fn passes_from_below_then_lands_on_the_top() {
    let mut sim = SimHarness::new(&level()).unwrap();
    let id = body(
        &mut sim,
        Vec2::new(5.5 * TS, 2.0 * TS),
        Vec2::new(0.0, 12000.0),
        0.0,
    );
    sim.step(1, &TickInputs::default());
    let up = sim.body(id);
    assert_eq!(up.position.y, 264.0, "passed right through the platform");
    assert!(!up.contacts.on_ceiling && !up.contacts.grounded);
    set_velocity(&mut sim, id, Vec2::new(0.0, -12000.0));
    sim.step(1, &TickInputs::default());
    let landed = sim.body(id);
    assert_eq!(landed.position.y, 5.0 * TS);
    assert_eq!(landed.velocity.y, 0.0);
    assert!(landed.contacts.grounded);
    assert!(!landed.contacts.on_ceiling);
}

#[test]
fn inverted_gravity_passes_from_above_and_lands_underneath() {
    let mut level = level();
    level
        .rules
        .insert("gravity".into(), serde_json::json!(1800));
    let mut sim = SimHarness::new(&level).unwrap();
    let id = body(
        &mut sim,
        Vec2::new(5.5 * TS, 8.0 * TS),
        Vec2::new(0.0, -12000.0),
        0.0,
    );
    sim.step(1, &TickInputs::default());
    let down = sim.body(id);
    assert!((down.position.y - 56.0).abs() < 0.001);
    assert!(!down.contacts.grounded && !down.contacts.on_ceiling);
    set_velocity(&mut sim, id, Vec2::new(0.0, 12000.0));
    sim.step(1, &TickInputs::default());
    let landed = sim.body(id);
    assert_eq!(landed.position.y + SIZE, 4.0 * TS);
    assert_eq!(landed.velocity.y, 0.0);
    assert!(landed.contacts.grounded);
    assert!(!landed.contacts.on_ceiling);
}

#[test]
fn extreme_speed_stops_at_the_first_platform_of_multiple_layers() {
    for inverted in [false, true] {
        let mut level = level();
        let mut second = level.layers[1].clone();
        second.name = "upper_platform".into();
        second.tiles[7] = "............".into();
        second.tiles[5] = "....====....".into(); // y = 6
        level.layers.push(second);
        if inverted {
            level
                .rules
                .insert("gravity".into(), serde_json::json!(1800));
        }
        let mut sim = SimHarness::new(&level).unwrap();
        let (start, speed, expected) = if inverted {
            (2.0 * TS, 1_000_000.0, 4.0 * TS - SIZE)
        } else {
            (9.0 * TS, -1_000_000.0, 7.0 * TS)
        };
        let id = body(
            &mut sim,
            Vec2::new(5.5 * TS, start),
            Vec2::new(0.0, speed),
            0.0,
        );
        sim.step(1, &TickInputs::default());
        assert_eq!(sim.body(id).position.y, expected);
        assert!(sim.body(id).contacts.grounded);
    }
}

#[test]
fn starting_inside_a_platform_does_not_snap_back_to_its_supporting_face() {
    for gravity in [-1800, 1800] {
        let mut level = level();
        level
            .rules
            .insert("gravity".into(), serde_json::json!(gravity));
        let mut sim = SimHarness::new(&level).unwrap();
        let start = 4.5 * TS;
        let dir = if gravity < 0 { -1.0 } else { 1.0 };
        let id = body(
            &mut sim,
            Vec2::new(5.5 * TS, start),
            Vec2::new(0.0, dir * 1800.0),
            0.0,
        );
        sim.step(1, &TickInputs::default());
        assert_eq!(sim.body(id).position.y, start + dir * 30.0);
        assert!(!sim.body(id).contacts.grounded);
    }
}

#[test]
fn platforms_are_neither_side_walls_nor_ceilings() {
    let mut sim = SimHarness::new(&level()).unwrap();
    let id = body(
        &mut sim,
        Vec2::new(3.0 * TS, 4.25 * TS),
        Vec2::new(2400.0, 0.0),
        0.0,
    );
    sim.step(1, &TickInputs::default());
    let state = sim.body(id);
    assert_eq!(state.position.x, 3.0 * TS + 40.0);
    assert_eq!(state.velocity.x, 2400.0);
    assert!(!state.contacts.on_wall_left && !state.contacts.on_wall_right);
    assert!(!state.contacts.on_ceiling && !state.contacts.grounded);
}

#[test]
fn support_is_stable_until_walking_off_the_edge() {
    for gravity in [-1800, 1800] {
        let mut level = level();
        level
            .rules
            .insert("gravity".into(), serde_json::json!(gravity));
        let mut sim = SimHarness::new(&level).unwrap();
        let start = if gravity < 0 {
            5.0 * TS
        } else {
            4.0 * TS - SIZE
        };
        let id = body(&mut sim, Vec2::new(5.5 * TS, start), Vec2::ZERO, 1.0);
        sim.step(300, &TickInputs::default());
        assert_eq!(sim.body(id).position.y, start);
        assert_eq!(sim.body(id).velocity.y, 0.0);
        assert!(sim.body(id).contacts.grounded);
        set_velocity(&mut sim, id, Vec2::new(2400.0, 0.0));
        sim.step(4, &TickInputs::default());
        let off = sim.body(id);
        assert!(!off.contacts.grounded);
        assert!(
            (off.position.y - start) * gravity as f32 > 0.0,
            "started falling in gravity direction"
        );
    }
}

#[test]
fn a_solid_cell_overlapping_a_platform_still_blocks_both_directions() {
    let mut level = level();
    level.layers[0].tiles[7] = "....####....".into();
    let mut sim = SimHarness::new(&level).unwrap();
    let id = body(
        &mut sim,
        Vec2::new(5.5 * TS, 2.0 * TS),
        Vec2::new(0.0, 12000.0),
        0.0,
    );
    sim.step(1, &TickInputs::default());
    assert_eq!(sim.body(id).position.y + SIZE, 4.0 * TS);
    assert!(sim.body(id).contacts.on_ceiling);
    assert!(!sim.body(id).contacts.grounded);
}

#[test]
fn a_real_player_jumps_through_then_uses_the_platform_as_ground() {
    let mut level = level();
    // Two tiles above the floor, so a full default jump can emerge completely above it.
    level.layers[1].tiles[7] = "............".into();
    level.layers[1].tiles[9] = "....====....".into(); // y = 2
    let mut sim = SimHarness::new(&level).unwrap();
    let id = sim.spawn_player(PlayerId(0));
    sim.step(1, &TickInputs::default());
    let jump = TickInputs(
        [(
            PlayerId(0),
            PlayerInput {
                buttons: buttons::JUMP,
                ..Default::default()
            },
        )]
        .into(),
    );
    sim.step(50, &jump);
    let at = sim.body(id);
    assert_eq!(at.position.y, 3.0 * TS);
    assert!(at.contacts.grounded);
    // Keep holding: the platform must not create extra jump edges.
    sim.step(30, &jump);
    assert_eq!(sim.body(id).position, at.position);
    sim.step(1, &TickInputs::default());
    sim.step(1, &jump);
    assert!(sim.body(id).velocity.y > Rules::default().gravity.abs() / 60.0);
}

#[test]
fn horizontal_edge_touch_without_overlap_does_not_land() {
    let mut sim = SimHarness::new(&level()).unwrap();
    let id = body(
        &mut sim,
        Vec2::new(4.0 * TS - SIZE / 2.0, 7.0 * TS),
        Vec2::new(0.0, -6000.0),
        0.0,
    );
    sim.step(1, &TickInputs::default());
    assert!((sim.body(id).position.y - 124.0).abs() < 0.001);
    assert!(!sim.body(id).contacts.grounded);
}

#[test]
fn platform_motion_is_deterministic() {
    let run = || {
        let mut sim = SimHarness::new(&level()).unwrap();
        let id = sim.spawn_player(PlayerId(0));
        let mut trace = Vec::new();
        for tick in 0..300 {
            let input = PlayerInput {
                axis_x: if tick % 60 < 30 { 127 } else { -127 },
                buttons: if tick % 43 < 25 { buttons::JUMP } else { 0 },
                ..Default::default()
            };
            sim.step(1, &TickInputs([(PlayerId(0), input)].into()));
            trace.push(sim.body(id));
        }
        trace
    };
    assert_eq!(run(), run());
}
