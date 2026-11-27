//! Level scripts driving the simulation headless: every exposed function and hook.

use std::collections::BTreeMap;

use bevy::prelude::*;
use mw_core::input::PlayerId;
use mw_level::{EntityDef, Level, LevelBundle, Size, Spawn};
use mw_sim::combat::{DamageEvent, DamageQueue, Health};
use mw_sim::scripting::{
    EntityKind, HudMessages, ScriptLog, ScriptRuntime, ScriptTags, ScriptVars,
};
use mw_sim::{Body, Rules, SimHarness, SimId, SimLoadError, TickInputs};
use rhai::Map;

const TS: f32 = 32.0;

fn level() -> Level {
    let mut level = Level::new("scripted", "Scripted", Size { w: 40, h: 20 });
    level.spawns = vec![Spawn { x: 5.0, y: 5.0 }];
    level.rules.insert("gravity".into(), serde_json::json!(0.0));
    level.scripts = vec!["scripts/main.rhai".into()];
    level
}

fn bundle(level: Level, source: &str) -> LevelBundle {
    LevelBundle {
        level,
        scripts: BTreeMap::from([("scripts/main.rhai".to_string(), source.to_string())]),
    }
}

fn sim(source: &str) -> SimHarness {
    sim_with(level(), source)
}

fn sim_with(level: Level, source: &str) -> SimHarness {
    SimHarness::from_bundle(&bundle(level, source), |_| {}).unwrap_or_else(|e| panic!("{e}"))
}

fn step(sim: &mut SimHarness, ticks: u32) {
    sim.step(ticks, &TickInputs::default());
}

fn state(sim: &SimHarness) -> Map {
    sim.world().resource::<ScriptRuntime>().host().state()
}

fn log(sim: &SimHarness) -> &ScriptLog {
    sim.world().resource::<ScriptLog>()
}

fn entity(sim: &mut SimHarness, id: SimId) -> Option<Entity> {
    sim.world_mut()
        .query::<(Entity, &SimId)>()
        .iter(sim.world())
        .find(|(_, s)| **s == id)
        .map(|(e, _)| e)
}

fn get<C: Component + Clone>(sim: &mut SimHarness, id: SimId) -> Option<C> {
    let e = entity(sim, id)?;
    sim.world().get::<C>(e).cloned()
}

fn int(map: &Map, key: &str) -> i64 {
    map.get(key)
        .unwrap_or_else(|| panic!("state.{key} missing in {map:?}"))
        .as_int()
        .unwrap()
}

#[test]
fn scripts_read_and_write_every_rule_and_toggle_mechanics() {
    let mut sim = sim(r#"
        fn on_level_start() {
            state.speed = rules.run_speed;
            rules.run_speed = rules.run_speed * 2;
            set_rule("jump_height", 200);
            enable_mechanic("dash");
            rules.max_fall_speed = "fast"; // refused: not a number
        }"#);
    let before = sim.world().resource::<Rules>().clone();
    step(&mut sim, 1);
    let rules = sim.world().resource::<Rules>();
    assert_eq!(rules.run_speed, before.run_speed * 2.0);
    assert_eq!(rules.jump_height, 200.0);
    assert!(rules.mechanics.iter().any(|m| m == "dash"));
    assert_eq!(
        rules.max_fall_speed, before.max_fall_speed,
        "invalid value kept out"
    );
    assert_eq!(log(&sim).rejected.len(), 1, "{:?}", log(&sim).rejected);
    assert_eq!(
        state(&sim)["speed"].as_float().unwrap(),
        f64::from(before.run_speed)
    );

    // Next tick the script sees its own changes, and can turn a mechanic off again.
    let mut sim2 = sim_with(
        level(),
        "fn on_tick(dt) { if mechanic_active(\"jump\") { disable_mechanic(\"jump\"); } }",
    );
    step(&mut sim2, 2);
    assert!(!sim2
        .world()
        .resource::<Rules>()
        .mechanics
        .iter()
        .any(|m| m == "jump"));
}

#[test]
fn entity_api_moves_damages_heals_tags_and_spawns() {
    let mut sim = sim(r#"
        fn on_player_spawn(p) {
            state.spawned = p.player;
            p.teleport(3, 4);
            p.vx = 2;
            p.damage(2);
            p.add_tag("hero");
            p.set("coins", 2);
            spawn_entity("crate", 10, 5, #{ hp: 3, gravity: 0 });
        }
        fn on_tick(dt) {
            for e in entities_with_tag("hero") {
                state.hp = e.hp;
                state.coins = e.get("coins");
            }
            for e in entities() {
                if e.kind == "crate" && e.hp == 3 { e.heal(5); e.remove_tag("none"); }
            }
        }"#);
    let player = sim.spawn_player(PlayerId(0));
    step(&mut sim, 1);
    let body = sim.body(player);
    // Teleported to (3, 4) tiles, then moved one tick at up to 2 tiles/s (the `movement`
    // mechanic already brakes a player without input in that same tick).
    assert_eq!(body.position.y, 4.0 * TS);
    assert!(
        body.position.x > 3.0 * TS && body.position.x <= 3.0 * TS + 2.0 * TS / 60.0,
        "{}",
        body.position
    );
    assert_eq!(get::<Health>(&mut sim, player).unwrap().current, 3, "5 - 2");
    assert!(get::<ScriptTags>(&mut sim, player)
        .unwrap()
        .0
        .contains("hero"));
    assert_eq!(
        get::<ScriptVars>(&mut sim, player).unwrap().0["coins"],
        serde_json::json!(2)
    );
    assert_eq!(int(&state(&sim), "spawned"), 0);

    step(&mut sim, 1);
    let s = state(&sim);
    assert_eq!((int(&s, "hp"), int(&s, "coins")), (3, 2));
    let crate_id = sim
        .world_mut()
        .query::<(&SimId, &EntityKind)>()
        .iter(sim.world())
        .find(|(_, k)| k.0 == "crate")
        .map(|(id, _)| *id)
        .expect("spawned crate");
    assert_eq!(sim.body(crate_id).position, Vec2::new(10.0 * TS, 5.0 * TS));
    let hp = get::<Health>(&mut sim, crate_id).unwrap();
    assert_eq!(
        (hp.current, hp.max),
        (3, 3),
        "heal never exceeds the maximum"
    );
    assert!(log(&sim).rejected.is_empty(), "{:?}", log(&sim).rejected);
}

#[test]
fn despawn_and_unknown_ids_are_handled() {
    let mut sim = sim(r#"
        fn on_tick(dt) {
            for e in entities() { if e.kind == "rock" { e.despawn(); } }
            let ghost = entity(999);
            state.ghost = ghost == ();
        }
        fn on_level_start() { spawn_entity("rock", 1, 1); }"#);
    step(&mut sim, 1);
    // Spawned by on_level_start, despawned by on_tick of the same tick.
    assert_eq!(sim.bodies().len(), 0);
    assert!(state(&sim)["ghost"].as_bool().unwrap());
}

#[test]
fn timers_run_on_exact_ticks() {
    let mut sim = sim(r#"
        state.at = [];
        fn on_level_start() {
            after(0.5, || state.at.push(state.ticks));
            state.id = every(0.25, || {
                state.at.push(state.ticks);
                if state.at.len() >= 3 { cancel(state.id); }
            });
        }
        fn on_tick(dt) { state.ticks = (state.ticks ?? 0) + 1; }"#);
    step(&mut sim, 120);
    let at: Vec<i64> = state(&sim)["at"]
        .clone()
        .into_array()
        .unwrap()
        .into_iter()
        .map(|v| v.as_int().unwrap())
        .collect();
    // every 0.25 s = 15 ticks; after 0.5 s = 30 ticks, scheduled before it so it runs first.
    assert_eq!(at, [15, 30, 30]);
    assert_eq!(
        sim.world().resource::<ScriptRuntime>().host().timer_count(),
        0
    );
}

#[test]
fn hud_messages_last_their_duration() {
    let mut sim = sim("fn on_level_start() { hud(\"Bienvenue\", 0.1); }");
    step(&mut sim, 1);
    assert_eq!(
        sim.world().resource::<HudMessages>().0,
        [("Bienvenue".to_string(), 5)]
    );
    step(&mut sim, 5);
    assert!(sim.world().resource::<HudMessages>().0.is_empty());
}

#[test]
fn on_player_spawn_runs_once_per_player() {
    let mut sim = sim("fn on_player_spawn(p) { state.n = (state.n ?? 0) + 1; }");
    sim.spawn_player(PlayerId(0));
    step(&mut sim, 3);
    sim.spawn_player(PlayerId(1));
    step(&mut sim, 3);
    assert_eq!(int(&state(&sim), "n"), 2);
}

#[test]
fn on_collision_fires_when_contact_starts() {
    let mut sim = sim(
        "fn on_collision(a, b) { state.n = (state.n ?? 0) + 1; state.pair = a.kind + \"/\" + b.kind; }",
    );
    let player = sim.spawn_player(PlayerId(0));
    step(&mut sim, 1);
    let feet = sim.body(player).position;
    let block = Body {
        size: Vec2::splat(16.0),
        gravity_scale: 0.0,
    };
    let rock = sim.spawn_body(feet + Vec2::new(200.0, 0.0), block, Vec2::ZERO);
    step(&mut sim, 2);
    assert!(!state(&sim).contains_key("n"), "apart: no collision");
    sim.teleport(rock, feet);
    step(&mut sim, 5);
    let s = state(&sim);
    assert_eq!(int(&s, "n"), 1, "one call while the contact lasts");
    assert_eq!(s["pair"].clone().into_string().unwrap(), "player/body");
    sim.teleport(rock, feet + Vec2::new(200.0, 0.0));
    step(&mut sim, 1);
    sim.teleport(rock, feet);
    step(&mut sim, 1);
    assert_eq!(int(&state(&sim), "n"), 2, "a new contact calls again");
}

#[test]
fn trigger_zones_from_the_level_call_on_trigger_on_entry() {
    let mut level = level();
    level.entities.push(EntityDef {
        kind: "trigger".into(),
        x: 5.0,
        y: 5.0,
        props: BTreeMap::from([
            ("name".to_string(), serde_json::json!("door")),
            ("w".to_string(), serde_json::json!(2)),
            ("h".to_string(), serde_json::json!(2)),
        ]),
    });
    assert!(level.validate().is_empty(), "{:?}", level.validate());
    let mut sim = sim_with(
        level,
        "fn on_trigger(name, e) { state.n = (state.n ?? 0) + 1; state.name = name; state.who = e.kind; }",
    );
    let player = sim.spawn_player(PlayerId(0)); // spawns inside the zone
    step(&mut sim, 10);
    let s = state(&sim);
    assert_eq!(int(&s, "n"), 1);
    assert_eq!(s["name"].clone().into_string().unwrap(), "door");
    assert_eq!(s["who"].clone().into_string().unwrap(), "player");
    sim.teleport(player, Vec2::new(20.0, 5.0) * TS);
    step(&mut sim, 1);
    sim.teleport(player, Vec2::new(5.0, 5.0) * TS);
    step(&mut sim, 1);
    assert_eq!(int(&state(&sim), "n"), 2, "leaving then entering again");
}

#[test]
fn on_damage_changes_hits_and_on_death_sees_the_kill() {
    let mut sim = sim(r#"
        fn on_damage(target, source, amount) {
            state.calls = (state.calls ?? 0) + 1;
            if target.is_player { 0 } else { amount * 10 }
        }
        fn on_death(target, source) { state.dead = target.kind; state.by = source == (); }
        fn on_level_start() { spawn_entity("crate", 10, 5, #{ hp: 5, gravity: 0 }); }"#);
    let player = sim.spawn_player(PlayerId(0));
    step(&mut sim, 1);
    let crate_id = SimId(player.0 + 1);
    assert_eq!(get::<EntityKind>(&mut sim, crate_id).unwrap().0, "crate");
    sim.world_mut().resource_mut::<DamageQueue>().0.extend([
        DamageEvent {
            target: player,
            source: None,
            amount: 3,
        },
        DamageEvent {
            target: crate_id,
            source: None,
            amount: 1,
        },
    ]);
    step(&mut sim, 1);
    assert_eq!(
        get::<Health>(&mut sim, player).unwrap().current,
        5,
        "cancelled by the script"
    );
    assert_eq!(
        get::<Health>(&mut sim, crate_id).unwrap().current,
        0,
        "1 * 10 kills"
    );
    let s = state(&sim);
    assert_eq!(s["dead"].clone().into_string().unwrap(), "crate");
    assert!(s["by"].as_bool().unwrap());
    // A hit on a dead (or invincible) body never reaches `on_damage`.
    sim.world_mut()
        .resource_mut::<DamageQueue>()
        .0
        .push(DamageEvent {
            target: crate_id,
            source: None,
            amount: 1,
        });
    step(&mut sim, 1);
    assert_eq!(int(&state(&sim), "calls"), 2);
}

#[test]
fn scripted_runs_are_deterministic() {
    let source = r#"
        state.seed = 7;
        fn on_level_start() { every(0.1, || spawn_entity("orb", state.seed % 30, 10, #{ hp: 2 })); }
        fn on_tick(dt) {
            state.seed = (state.seed * 31 + 11) % 1000;
            for p in players() { p.vx = (state.seed % 7) - 3; }
            for e in entities() { if e.kind == "orb" && e.y < 3 { e.despawn(); } }
        }
        fn on_collision(a, b) { a.damage(1); }"#;
    let run = || {
        let mut sim = sim(source);
        sim.spawn_player(PlayerId(0));
        sim.spawn_player(PlayerId(1));
        step(&mut sim, 240);
        (sim.bodies(), format!("{:?}", state(&sim)))
    };
    let (a, b) = (run(), run());
    assert!(a.0.len() > 2, "orbs were spawned");
    assert_eq!(a, b);
}

#[test]
fn script_errors_are_reported_without_stopping_the_game() {
    let err = SimHarness::from_bundle(&bundle(level(), "fn on_tick(dt) {"), |_| {})
        .err()
        .expect("syntax error");
    assert!(
        matches!(err, SimLoadError::Scripts(ref e) if e.len() == 1),
        "{err}"
    );

    let mut sim =
        sim("fn on_tick(dt) { state.n = (state.n ?? 0) + 1; if state.n == 2 { throw \"boom\"; } }");
    step(&mut sim, 5);
    assert_eq!(log(&sim).errors.len(), 1);
    assert_eq!(int(&state(&sim), "n"), 5, "later ticks still run");
}

#[test]
fn hostile_values_cannot_corrupt_or_stall_the_simulation() {
    let mut sim = sim(r#"
        fn on_player_spawn(p) {
            spawn_entity("wall", 2, 2, #{ w: 1e12, h: -3, gravity: 1e300, hp: 99999999999 });
            p.heal(9223372036854775807);
            p.vx = 1e30;                  // bounded by the level's invisible walls
        }
        fn on_tick(dt) {
            if state.nan == () { state.nan = true; for p in players() { p.x = 0.0 / 0.0; } }
        }"#);
    let player = sim.spawn_player(PlayerId(0));
    step(&mut sim, 3);
    let wall = sim
        .world_mut()
        .query::<(&EntityKind, &Body, &Health)>()
        .iter(sim.world())
        .find(|(k, ..)| k.0 == "wall")
        .map(|(_, b, h)| (b.size, b.gravity_scale, h.max))
        .expect("wall spawned");
    assert_eq!(
        wall.0,
        Vec2::new(40.0 * TS, TS / 16.0),
        "clamped to the level, minimum side"
    );
    assert_eq!(wall.1, 1.0, "infinite gravity scale ignored");
    assert_eq!(wall.2, u32::MAX);
    assert_eq!(
        get::<Health>(&mut sim, player).unwrap().current,
        5,
        "heal saturates"
    );
    let x = sim.body(player).position.x;
    assert!(x.is_finite() && x <= 40.0 * TS, "{x}");
    assert_eq!(log(&sim).errors.len(), 1, "NaN position is a script error");
}
