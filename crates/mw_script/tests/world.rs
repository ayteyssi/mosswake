//! The world API without a simulation: views in, commands out.

use std::collections::{BTreeMap, BTreeSet};

use mw_script::{EntityData, Limits, ScriptCommand, ScriptHost};
use serde_json::json;

fn host(source: &str) -> ScriptHost {
    ScriptHost::new(
        &[("scripts/main.rhai".into(), source.into())],
        Limits::default(),
    )
    .unwrap_or_else(|e| panic!("{e:#?}"))
}

fn body(id: i64, player: Option<i64>, x: f64) -> EntityData {
    EntityData {
        id,
        kind: if player.is_some() {
            "player".into()
        } else {
            "slime_green".into()
        },
        player,
        x,
        y: 2.0,
        vx: 0.0,
        vy: 0.0,
        hp: Some((5, 5)),
        tags: BTreeSet::new(),
        vars: BTreeMap::new(),
    }
}

#[test]
fn rules_edits_become_set_rule_commands() {
    let mut h = host(
        "fn on_level_start() { rules.gravity = -rules.gravity; rules.jump_height += 32.0; enable_mechanic(\"dash\"); }",
    );
    h.set_rules(
        &json!({ "gravity": -1800.0, "jump_height": 128.0, "mechanics": ["movement", "jump"] }),
    );
    let commands = h.on_level_start().commands;
    assert_eq!(
        commands,
        [
            ScriptCommand::SetRule {
                key: "gravity".into(),
                value: json!(1800.0)
            },
            ScriptCommand::SetRule {
                key: "jump_height".into(),
                value: json!(160.0)
            },
            ScriptCommand::SetRule {
                key: "mechanics".into(),
                value: json!(["movement", "jump", "dash"])
            },
        ]
    );
    // Diffs are against the new baseline: gravity and jump_height change again, while
    // enabling an already active mechanic changes nothing.
    assert_eq!(h.call_hook("on_level_start", ()).commands.len(), 2);
}

#[test]
fn reading_rules_and_toggling_mechanics() {
    let mut h = host(
        "fn on_tick(dt) { if mechanic_active(\"dash\") { disable_mechanic(\"dash\"); state.g = rules.gravity; } }",
    );
    h.set_rules(&json!({ "gravity": -900.0, "mechanics": ["dash"] }));
    let commands = h.on_tick(1.0 / 60.0).commands;
    assert_eq!(
        commands,
        [ScriptCommand::SetRule {
            key: "mechanics".into(),
            value: json!([])
        }]
    );
    assert_eq!(h.state()["g"].as_float().unwrap(), -900.0);
}

#[test]
fn entities_read_the_view_and_write_commands() {
    let mut h = host(
        r#"fn on_tick(dt) {
            for p in players() {
                if p.x < 5.0 { p.damage(2, "lava"); }
                p.vx = 3.0;
                p.add_tag("seen");
                p.set("score", 7);
                state.seen = p.has_tag("seen") && p.get("score") == 7;
            }
            let slime = entity(9);
            slime.teleport(1, 2.5);
            slime.heal(1);
            state.count = entities().len();
            state.missing = entity(42) == ();
            spawn_entity("slime_orange", 4, 5, #{ patrol: 2 }); // `spawn` is a Rhai reserved word
        }"#,
    );
    h.set_view(vec![body(1, Some(0), 3.0), body(9, None, 10.0)]);
    let commands = h.on_tick(1.0 / 60.0).commands;
    assert_eq!(
        commands,
        [
            ScriptCommand::Damage {
                id: 1,
                amount: 2,
                cause: "lava".into()
            },
            ScriptCommand::SetVelocity {
                id: 1,
                vx: 3.0,
                vy: 0.0
            },
            ScriptCommand::SetTag {
                id: 1,
                tag: "seen".into(),
                on: true
            },
            ScriptCommand::SetVar {
                id: 1,
                key: "score".into(),
                value: json!(7)
            },
            ScriptCommand::SetPosition {
                id: 9,
                x: 1.0,
                y: 2.5
            },
            ScriptCommand::SetVelocity {
                id: 9,
                vx: 0.0,
                vy: 0.0
            },
            ScriptCommand::Heal { id: 9, amount: 1 },
            ScriptCommand::Spawn {
                kind: "slime_orange".into(),
                x: 4.0,
                y: 5.0,
                props: json!({ "patrol": 2 })
            },
        ]
    );
    let state = h.state();
    assert!(
        state["seen"].as_bool().unwrap(),
        "reads see the script's own writes"
    );
    assert_eq!(state["count"].as_int().unwrap(), 2);
    assert!(state["missing"].as_bool().unwrap());
}

#[test]
fn timers_fire_on_their_tick_repeat_and_cancel() {
    let mut h = host(
        r#"state.log = [];
        fn on_level_start() {
            after(0.05, || state.log.push("once"));            // 3 ticks
            state.every_id = every(2.0 / 60.0, || state.log.push("tick"));
        }
        fn on_tick(dt) { if state.log.len() >= 4 { cancel(state.every_id); } }"#,
    );
    h.on_level_start();
    assert_eq!(h.timer_count(), 2);
    let mut seen = Vec::new();
    for tick in 1..=10 {
        h.on_tick(1.0 / 60.0);
        let log = h.state()["log"].clone().into_array().unwrap();
        seen.push((tick, log.len()));
    }
    let log: Vec<String> = h.state()["log"]
        .clone()
        .into_array()
        .unwrap()
        .into_iter()
        .map(|v| v.into_string().unwrap())
        .collect();
    // every: ticks 2, 4, 6 …; after: tick 3; cancelled once 4 entries exist (tick 6 run kept).
    assert_eq!(log, ["tick", "once", "tick", "tick"], "{seen:?}");
    assert_eq!(h.timer_count(), 0);
}

#[test]
fn filter_hook_chains_return_values() {
    let mut h = ScriptHost::new(
        &[
            (
                "scripts/a.rhai".into(),
                "fn on_damage(t, s, amount) { amount * 2 }".into(),
            ),
            (
                "scripts/b.rhai".into(),
                "fn on_damage(t, s, amount) { if amount > 5 { amount - 1 } }".into(),
            ),
            (
                "scripts/c.rhai".into(),
                "fn on_damage(t, s, amount) { state.target = t.id; }".into(),
            ),
        ],
        Limits::default(),
    )
    .unwrap();
    let target = h.entity(body(3, Some(0), 1.0));
    let (amount, outcome) = h.filter_hook("on_damage", vec![target, rhai::Dynamic::UNIT], 4);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert_eq!(
        amount, 7,
        "4 * 2 = 8, then 8 - 1; c returns () and keeps it"
    );
    assert_eq!(h.state()["target"].as_int().unwrap(), 3);
}

#[test]
fn a_failing_call_rolls_back_its_rule_edits_and_timers() {
    let mut h = host("fn on_tick(dt) { rules.gravity = 1.0; after(1.0, || 1); throw \"boom\"; }");
    h.set_rules(&json!({ "gravity": -1800.0 }));
    let outcome = h.on_tick(1.0 / 60.0);
    assert_eq!(outcome.errors.len(), 1);
    assert!(outcome.commands.is_empty());
    assert_eq!(h.timer_count(), 0);
    // The rules object is back to the baseline for the next hook.
    let mut probe = host("fn on_tick(dt) { state.g = rules.gravity; }");
    probe.set_rules(&json!({ "gravity": -1800.0 }));
    probe.on_tick(1.0 / 60.0);
    assert_eq!(probe.state()["g"].as_float().unwrap(), -1800.0);
}

#[test]
fn a_repeating_timer_can_cancel_itself() {
    let mut h = host(
        r#"state.n = 0;
        state.id = every(1.0 / 60.0, || { state.n += 1; if state.n == 3 { cancel(state.id); } });"#,
    );
    for _ in 0..10 {
        h.on_tick(1.0 / 60.0);
    }
    assert_eq!(h.state()["n"].as_int().unwrap(), 3);
    assert_eq!(h.timer_count(), 0);
}
