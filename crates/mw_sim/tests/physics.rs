//! Headless simulation tests: N ticks with scripted setups, checked against expectations.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use mw_level::{Layer, LayerKind, Level, LevelBundle, Size, Spawn};
use mw_sim::{
    tiles_to_world, Body, Rules, RulesError, SimHarness, SimLoadError, SimPlugin, SimSet, Tick,
    TickInputs, Velocity, TICK_DT,
};

const TS: f32 = 32.0;

/// Level from rows written top row first (`#` = solid), with one spawn in its first empty cell.
fn level(rows: &[&str]) -> Level {
    let mut level = Level::new(
        "test",
        "Test",
        Size {
            w: rows[0].len() as u32,
            h: rows.len() as u32,
        },
    );
    level.legend.insert('#', "mossy_ground".into());
    level.layers.push(Layer {
        name: "ground".into(),
        kind: LayerKind::Solid,
        tiles: rows.iter().map(|r| r.to_string()).collect(),
    });
    let (x, y) = (0..level.size.h)
        .flat_map(|y| (0..level.size.w).map(move |x| (x, y)))
        .find(|&(x, y)| !level.is_solid(x, y))
        .expect("test level has an empty cell");
    level.spawns.push(Spawn {
        x: x as f32 + 0.5,
        y: y as f32,
    });
    level
}

/// A player-sized body: 0.75 x 1.75 tiles.
fn player() -> Body {
    Body {
        size: Vec2::new(24.0, 56.0),
        gravity_scale: 1.0,
    }
}

const ROOM: &[&str] = &[
    "##########",
    "#........#",
    "#........#",
    "#........#",
    "#........#",
    "#........#",
    "##########",
];

fn idle() -> TickInputs {
    TickInputs::default()
}

#[test]
fn falls_and_lands_on_the_floor() {
    let mut sim = SimHarness::new(&level(ROOM)).unwrap();
    let id = sim.spawn_body(Vec2::new(3.5 * TS, 3.0 * TS), player(), Vec2::ZERO);
    sim.step(1, &idle());
    let s = sim.body(id);
    assert!(
        s.velocity.y < 0.0 && !s.contacts.grounded,
        "falling after one tick"
    );

    sim.step(120, &idle());
    let s = sim.body(id);
    assert_eq!(
        s.position,
        Vec2::new(3.5 * TS, TS),
        "feet exactly on the floor top"
    );
    assert_eq!(s.velocity, Vec2::ZERO);
    assert!(s.contacts.grounded && !s.contacts.on_ceiling);
    assert_eq!(sim.tick(), 121);
}

#[test]
fn stops_flush_against_a_wall() {
    let mut sim = SimHarness::new(&level(ROOM)).unwrap();
    let id = sim.spawn_body(Vec2::new(5.0 * TS, TS), player(), Vec2::new(600.0, 0.0));
    sim.step(60, &idle());
    let s = sim.body(id);
    assert_eq!(
        s.position.x,
        9.0 * TS - 12.0,
        "right side touches the wall at x = 9 tiles"
    );
    assert_eq!(s.velocity.x, 0.0);
    assert!(s.contacts.on_wall_right && !s.contacts.on_wall_left);
    assert!(s.contacts.grounded, "still standing on the floor");
}

#[test]
fn no_tunneling_at_extreme_speed() {
    // 1-tile-thick floor and wall, approached at ~300 tiles per tick.
    let mut sim = SimHarness::new(&level(&[
        "..........",
        "..........",
        "......#...",
        "..........",
        "##########",
        "..........",
    ]))
    .unwrap();
    let down = sim.spawn_body(
        Vec2::new(2.5 * TS, 5.0 * TS),
        Body {
            gravity_scale: 0.0,
            ..player()
        },
        Vec2::new(0.0, -600_000.0),
    );
    let right = sim.spawn_body(
        Vec2::new(2.5 * TS, 3.0 * TS + 4.0),
        Body {
            size: Vec2::new(24.0, 16.0),
            gravity_scale: 0.0,
        },
        Vec2::new(600_000.0, 0.0),
    );
    sim.step(1, &idle());
    assert_eq!(
        sim.body(down).position.y,
        2.0 * TS,
        "stopped on the thin floor"
    );
    assert_eq!(
        sim.body(right).position.x,
        6.0 * TS - 12.0,
        "stopped by the thin wall"
    );
}

#[test]
fn ceiling_contact_when_jumping_into_it() {
    let mut sim = SimHarness::new(&level(ROOM)).unwrap();
    let id = sim.spawn_body(Vec2::new(4.5 * TS, TS), player(), Vec2::new(0.0, 2000.0));
    let hit = (0..20).find_map(|_| {
        sim.step(1, &idle());
        let s = sim.body(id);
        s.contacts.on_ceiling.then_some(s)
    });
    let s = hit.expect("reaches the ceiling within 20 ticks");
    assert_eq!(s.position.y + 56.0, 6.0 * TS, "head flush with the ceiling");
    assert_eq!(s.velocity.y, 0.0, "upward speed cancelled by the hit");
}

#[test]
fn max_fall_speed_is_enforced() {
    let mut sim = SimHarness::new(&level(&["...", "...", "..."])).unwrap();
    let id = sim.spawn_body(Vec2::new(1.5 * TS, 2.0 * TS), player(), Vec2::ZERO);
    sim.step(240, &idle());
    let max = Rules::default().max_fall_speed;
    assert_eq!(sim.body(id).velocity.y, -max, "terminal velocity");
    assert!(sim.body(id).position.y < 0.0, "fell out of the open level");
}

#[test]
fn inverted_gravity_grounds_on_the_ceiling() {
    let mut l = level(ROOM);
    l.rules.insert("gravity".into(), serde_json::json!(1800.0));
    let mut sim = SimHarness::new(&l).unwrap();
    let id = sim.spawn_body(Vec2::new(4.5 * TS, 2.0 * TS), player(), Vec2::ZERO);
    sim.step(120, &idle());
    let s = sim.body(id);
    assert!(s.contacts.grounded && !s.contacts.on_ceiling, "{s:?}");
    assert_eq!(s.position.y + 56.0, 6.0 * TS);
}

/// Several bodies bouncing around for 10 s must end bit-identical across two runs.
#[test]
fn simulation_is_deterministic() {
    let run = || {
        let mut sim = SimHarness::new(&level(ROOM)).unwrap();
        for i in 0..8 {
            let i = i as f32;
            sim.spawn_body(
                Vec2::new((1.5 + i) * TS, (1.0 + (i % 4.0)) * TS + 0.37 * i),
                Body {
                    size: Vec2::new(10.0 + 3.0 * i, 20.0 + 2.0 * i),
                    gravity_scale: 0.5 + 0.25 * i,
                },
                Vec2::new(173.3 * (i - 3.5), 911.7 - 97.1 * i),
            );
        }
        sim.step(600, &idle());
        sim.bodies()
            .iter()
            .map(|s| {
                (
                    s.id,
                    s.position.x.to_bits(),
                    s.position.y.to_bits(),
                    s.velocity.x.to_bits(),
                    s.velocity.y.to_bits(),
                    s.contacts,
                )
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    assert_eq!(a.len(), 8);
    assert_eq!(a, b);
}

#[test]
fn level_rules_override_defaults_and_are_checked() {
    let mut l = level(ROOM);
    l.rules
        .insert("max_fall_speed".into(), serde_json::json!(300));
    let sim = SimHarness::new(&l).unwrap();
    assert_eq!(sim.world().resource::<Rules>().max_fall_speed, 300.0);

    l.rules
        .insert("mechanics".into(), serde_json::json!(["teleport"]));
    assert_eq!(
        SimHarness::new(&l).err(),
        Some(SimLoadError::InvalidRules(vec![
            RulesError::UnknownMechanic("teleport".into())
        ]))
    );

    l.rules.clear();
    l.rules.insert("gravty".into(), serde_json::json!(1));
    assert!(matches!(
        SimHarness::new(&l),
        Err(SimLoadError::InvalidRules(e)) if e == [RulesError::UnknownRule("gravty".into())]
    ));
}

#[test]
fn registered_mechanics_can_be_activated() {
    use mw_sim::RegisterMechanic;
    let mut l = level(ROOM);
    l.rules
        .insert("mechanics".into(), serde_json::json!(["jump"]));
    let sim = SimHarness::with_setup(&l, |app| {
        app.register_mechanic("jump");
    })
    .unwrap();
    assert!(sim.world().resource::<Rules>().is_active("jump"));
}

#[test]
fn invalid_levels_are_refused() {
    let mut l = level(ROOM);
    l.spawns.clear();
    assert!(matches!(
        SimHarness::new(&l),
        Err(SimLoadError::InvalidLevel(_))
    ));
}

/// A velocity set in `Mechanics` is applied by `Physics` in the same tick.
#[test]
fn mechanics_run_before_physics() {
    let mut sim = SimHarness::with_setup(&level(ROOM), |app| {
        app.add_systems(
            FixedUpdate,
            (|mut q: Query<&mut Velocity>| {
                for mut v in &mut q {
                    v.0.x = 60.0;
                }
            })
            .in_set(SimSet::Mechanics),
        );
    })
    .unwrap();
    let id = sim.spawn_body(Vec2::new(3.5 * TS, TS), player(), Vec2::ZERO);
    sim.step(1, &idle());
    assert_eq!(sim.body(id).position.x, 3.5 * TS + 60.0 * TICK_DT);
}

/// Bodies dropped on the tutorial's spawns stay where they are: spawns are on the ground.
#[test]
fn tutorial_spawns_are_standing_on_ground() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels/tutorial");
    let bundle = LevelBundle::load(dir).expect("tutorial loads");
    let mut sim = SimHarness::new(&bundle.level).unwrap();
    let ids: Vec<_> = bundle
        .level
        .spawns
        .iter()
        .map(|s| {
            let feet = tiles_to_world(&bundle.level, s.x, s.y);
            (sim.spawn_body(feet, player(), Vec2::ZERO), feet)
        })
        .collect();
    sim.step(60, &idle());
    for (id, feet) in ids {
        let s = sim.body(id);
        assert_eq!(s.position, feet, "spawn {id:?} moved");
        assert!(s.contacts.grounded);
    }
}

/// The real runtime: `MinimalPlugins` (no window), `FixedUpdate` driven by Bevy's clock.
#[test]
fn runs_with_minimal_plugins_at_60_hz() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, SimPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
    mw_sim::load_level(app.world_mut(), &level(ROOM)).unwrap();
    for _ in 0..31 {
        app.update();
    }
    let ticks = app.world().resource::<Tick>().0;
    assert!(
        (29..=31).contains(&ticks),
        "about one tick per 1/60 s frame, got {ticks}"
    );
}
