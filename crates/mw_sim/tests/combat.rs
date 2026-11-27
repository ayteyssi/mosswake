//! Melee and damage behavior, driven only by logical ticks and public components.
use bevy::prelude::*;
use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::{Layer, LayerKind, Level, Size, Spawn};
use mw_sim::{combat::*, Body, PlayerControl, Rules, SimHarness, SimId, TickInputs, Velocity};

fn arena(enabled: bool) -> Level {
    let mut level = Level::new("combat", "Combat", Size { w: 30, h: 20 });
    level.spawns = vec![Spawn { x: 5.0, y: 5.0 }];
    level.rules.insert("gravity".into(), serde_json::json!(0.0));
    level.rules.insert(
        "mechanics".into(),
        if enabled {
            serde_json::json!(["melee"])
        } else {
            serde_json::json!([])
        },
    );
    level
}
fn entity(sim: &mut SimHarness, id: SimId) -> Entity {
    sim.world_mut()
        .query::<(Entity, &SimId)>()
        .iter(sim.world())
        .find(|(_, found)| **found == id)
        .unwrap()
        .0
}
fn target(sim: &mut SimHarness, feet: Vec2, hp: u32) -> SimId {
    let size = Vec2::splat(16.0);
    let id = sim.spawn_body(
        feet,
        Body {
            size,
            gravity_scale: 0.0,
        },
        Vec2::ZERO,
    );
    let e = entity(sim, id);
    sim.world_mut()
        .entity_mut(e)
        .insert((Health::new(hp), Hurtbox::from_size(size)));
    id
}
fn health(sim: &mut SimHarness, id: SimId) -> Health {
    let e = entity(sim, id);
    *sim.world().get::<Health>(e).unwrap()
}
fn attack(x: i8, y: i8) -> TickInputs {
    TickInputs(std::collections::BTreeMap::from([(
        PlayerId(0),
        PlayerInput {
            axis_x: x,
            axis_y: y,
            buttons: buttons::ATTACK,
        },
    )]))
}
fn quiet_impacts(sim: &mut SimHarness) {
    let mut rules = sim.world_mut().resource_mut::<Rules>();
    rules.knockback_x = 0.0;
    rules.knockback_y = 0.0;
    rules.damage_invincibility = 0.0;
}
fn damage(sim: &mut SimHarness, event: DamageEvent) {
    sim.world_mut().resource_mut::<DamageQueue>().0.push(event);
}

#[test]
fn four_directions_hit_only_the_intended_target() {
    for (x, y, feet, direction) in [
        (127, 0, Vec2::new(188.0, 180.0), AttackDirection::Right),
        (-127, 0, Vec2::new(132.0, 180.0), AttackDirection::Left),
        (0, 127, Vec2::new(160.0, 225.0), AttackDirection::Up),
        (0, -127, Vec2::new(160.0, 136.0), AttackDirection::Down),
    ] {
        let mut sim = SimHarness::new(&arena(true)).unwrap();
        let player = sim.spawn_player(PlayerId(0));
        let victim = target(&mut sim, feet, 3);
        let distant = target(&mut sim, Vec2::new(500.0, 180.0), 3);
        sim.step(1, &attack(x, y));
        assert_eq!(health(&mut sim, victim).current, 2, "{direction:?}");
        assert_eq!(health(&mut sim, distant).current, 3);
        let e = entity(&mut sim, player);
        assert_eq!(sim.world().get::<Hitbox>(e).unwrap().direction, direction);
        assert_eq!(
            sim.world().resource::<CombatEvents>().damage,
            vec![DamageEvent {
                target: victim,
                source: Some(player),
                amount: 1
            }]
        );
    }
}

#[test]
fn disabled_melee_does_nothing_and_pvp_is_not_enabled() {
    for enabled in [false, true] {
        let mut sim = SimHarness::new(&arena(enabled)).unwrap();
        let player = sim.spawn_player(PlayerId(0));
        let victim = target(&mut sim, Vec2::new(188.0, 180.0), 3);
        let other = sim.spawn_player(PlayerId(1));
        sim.teleport(other, Vec2::new(188.0, 160.0));
        sim.step(1, &attack(127, 0));
        assert_eq!(
            health(&mut sim, victim).current,
            if enabled { 2 } else { 3 }
        );
        assert_eq!(
            health(&mut sim, other).current,
            Rules::default().player_health
        );
        if !enabled {
            let e = entity(&mut sim, player);
            assert!(sim.world().get::<Hitbox>(e).is_none());
        }
    }
}

#[test]
fn swings_are_temporary_hit_once_and_require_a_new_press_after_cooldown() {
    let mut sim = SimHarness::new(&arena(true)).unwrap();
    let player = sim.spawn_player(PlayerId(0));
    let victim = target(&mut sim, Vec2::new(188.0, 180.0), 9);
    quiet_impacts(&mut sim);
    let active = mw_sim::seconds_to_ticks(Rules::default().melee_duration);
    sim.step(active, &attack(127, 0));
    assert_eq!(
        health(&mut sim, victim).current,
        8,
        "one hit despite zero invincibility"
    );
    let e = entity(&mut sim, player);
    assert!(
        sim.world().get::<Hitbox>(e).is_none(),
        "expires after active ticks"
    );
    sim.step(1, &TickInputs::default());
    sim.step(1, &attack(127, 0));
    assert_eq!(health(&mut sim, victim).current, 8, "still in cooldown");
    sim.step(30, &attack(127, 0));
    assert_eq!(
        health(&mut sim, victim).current,
        8,
        "holding does not repeat"
    );
    sim.step(1, &TickInputs::default());
    sim.step(1, &attack(127, 0));
    assert_eq!(health(&mut sim, victim).current, 7);
}

#[test]
fn invincibility_knockback_and_death_events_use_tick_timers() {
    let mut level = arena(true);
    level.rules.insert(
        "mechanics".into(),
        serde_json::json!(["movement", "jump", "melee"]),
    );
    level
        .rules
        .insert("damage_invincibility".into(), serde_json::json!(0.05));
    let mut sim = SimHarness::new(&level).unwrap();
    let player = sim.spawn_player(PlayerId(0));
    let source = target(&mut sim, Vec2::new(100.0, 160.0), 3);
    let hit = DamageEvent {
        target: player,
        source: Some(source),
        amount: 1,
    };
    damage(&mut sim, hit);
    sim.step(1, &TickInputs::default());
    let rules = sim.world().resource::<Rules>().clone();
    assert_eq!(health(&mut sim, player).current, rules.player_health - 1);
    assert_eq!(
        sim.body(player).velocity,
        Vec2::new(rules.knockback_x, rules.knockback_y)
    );
    for _ in 0..2 {
        damage(&mut sim, hit);
        sim.step(1, &attack(-127, 0));
        assert_eq!(health(&mut sim, player).current, rules.player_health - 1);
        assert!(sim.world().resource::<CombatEvents>().damage.is_empty());
        assert_eq!(
            sim.body(player).velocity.x,
            rules.knockback_x,
            "movement cannot cancel stun"
        );
    }
    damage(&mut sim, hit);
    sim.step(1, &TickInputs::default());
    assert_eq!(health(&mut sim, player).current, rules.player_health - 2);
    sim.step(3, &TickInputs::default());
    damage(
        &mut sim,
        DamageEvent {
            amount: u32::MAX,
            ..hit
        },
    );
    sim.step(1, &TickInputs::default());
    assert_eq!(health(&mut sim, player).current, 0);
    assert_eq!(
        sim.world().resource::<CombatEvents>().deaths,
        vec![DeathEvent {
            target: player,
            source: Some(source)
        }]
    );
    damage(&mut sim, hit);
    sim.step(1, &attack(127, 0));
    assert!(
        sim.world().resource::<CombatEvents>().deaths.is_empty(),
        "death emitted once"
    );
    let e = entity(&mut sim, player);
    assert!(
        sim.world().get::<Hitbox>(e).is_none(),
        "dead players cannot attack"
    );
}

#[test]
fn pogo_bounces_on_enemies_hazard_tiles_and_entities_in_both_gravity_directions() {
    for inverted in [false, true] {
        for surface in ["enemy", "tile", "entity"] {
            let mut level = arena(true);
            level.rules.insert(
                "gravity".into(),
                serde_json::json!(if inverted { 1800.0 } else { -1800.0 }),
            );
            if surface == "tile" {
                level.legend.insert('^', "thorns_tile".into());
                let mut rows = vec![".".repeat(30); 20];
                let y = if inverted { 7 } else { 4 };
                rows[19 - y].replace_range(5..6, "^");
                level.layers.push(Layer {
                    name: "hazards".into(),
                    kind: LayerKind::Hazard,
                    tiles: rows,
                });
            }
            let mut sim = SimHarness::new(&level).unwrap();
            let player = sim.spawn_player(PlayerId(0));
            if surface != "tile" {
                let id = target(
                    &mut sim,
                    Vec2::new(160.0, if inverted { 225.0 } else { 136.0 }),
                    5,
                );
                if surface == "entity" {
                    let e = entity(&mut sim, id);
                    sim.world_mut()
                        .entity_mut(e)
                        .remove::<Health>()
                        .insert(PogoSurface);
                }
            }
            sim.step(1, &attack(0, -127));
            let rules = sim.world().resource::<Rules>();
            let expected = -rules.gravity_dir() * rules.pogo_speed;
            assert_eq!(
                sim.body(player).velocity.y,
                expected,
                "{surface}, inverted={inverted}"
            );
            let e = entity(&mut sim, player);
            assert!(!sim.world().get::<PlayerControl>(e).unwrap().jumping);
            assert!(sim.world().get::<Hitbox>(e).unwrap().pogoed);
        }
    }
}

#[test]
fn one_swing_cannot_pogo_repeatedly_or_remain_active_when_disabled() {
    let mut sim = SimHarness::new(&arena(true)).unwrap();
    let player = sim.spawn_player(PlayerId(0));
    target(&mut sim, Vec2::new(160.0, 136.0), 5);
    quiet_impacts(&mut sim);
    sim.step(1, &attack(0, -127));
    let e = entity(&mut sim, player);
    sim.world_mut().get_mut::<Velocity>(e).unwrap().0.y = 123.0;
    sim.step(1, &attack(0, -127));
    assert_eq!(
        sim.body(player).velocity.y,
        123.0,
        "pogo only once per swing"
    );
    sim.world_mut().resource_mut::<Rules>().mechanics.clear();
    sim.step(1, &attack(0, -127));
    assert!(sim.world().get::<Hitbox>(e).is_none());
}

#[test]
fn snapshots_include_combat_state_and_are_independent_of_ecs_iteration_order() {
    #[derive(Component)]
    struct Noise;
    let run = |reorder: bool| {
        let mut sim = SimHarness::new(&arena(true)).unwrap();
        let player = sim.spawn_player(PlayerId(0));
        let a = target(&mut sim, Vec2::new(188.0, 180.0), 5);
        let b = target(&mut sim, Vec2::new(190.0, 180.0), 5);
        if reorder {
            let e = entity(&mut sim, a);
            sim.world_mut().entity_mut(e).insert(Noise);
            sim.world_mut().spawn(Noise);
        }
        sim.step(1, &attack(127, 0));
        let events = sim.world().resource::<CombatEvents>().clone();
        assert_eq!(
            events.damage.iter().map(|e| e.target).collect::<Vec<_>>(),
            [a, b]
        );
        let snapshot = sim.snapshot();
        let combat = snapshot
            .player(PlayerId(0))
            .unwrap()
            .combat
            .as_ref()
            .unwrap();
        assert_eq!(
            combat.health.unwrap().current,
            Rules::default().player_health
        );
        let hitbox = combat.hitbox.as_ref().unwrap();
        assert_eq!(
            hitbox.hit_targets.iter().copied().collect::<Vec<_>>(),
            [a, b]
        );
        assert!(combat.melee.unwrap().cooldown_ticks > 0);
        assert_eq!(events.damage[0].source, Some(player));
        let mut snapshots = vec![snapshot];
        for i in 0..60 {
            let inputs = if i % 20 == 0 {
                attack(127, 0)
            } else {
                TickInputs::default()
            };
            sim.step(1, &inputs);
            snapshots.push(sim.snapshot());
        }
        (snapshots, events)
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn combat_rules_are_overridable_and_ranges_are_validated() {
    let mut level = arena(true);
    level
        .rules
        .insert("player_health".into(), serde_json::json!(9));
    level
        .rules
        .insert("melee_damage".into(), serde_json::json!(3));
    level
        .rules
        .insert("melee_reach".into(), serde_json::json!(100.0));
    let mut sim = SimHarness::new(&level).unwrap();
    let player = sim.spawn_player(PlayerId(0));
    let victim = target(&mut sim, Vec2::new(250.0, 180.0), 8);
    sim.step(1, &attack(127, 0));
    assert_eq!(health(&mut sim, player).current, 9);
    assert_eq!(health(&mut sim, victim).current, 5);
    for (key, value) in [
        ("player_health", 0.0),
        ("melee_damage", 0.0),
        ("melee_reach", -1.0),
        ("melee_thickness", 0.0),
        ("melee_duration", 0.0),
        ("melee_cooldown", -1.0),
        ("damage_invincibility", -1.0),
        ("hit_stun", -1.0),
        ("knockback_x", -1.0),
        ("knockback_y", -1.0),
        ("pogo_speed", -1.0),
    ] {
        let mut bad = arena(true);
        bad.rules.insert(
            key.into(),
            if key == "player_health" || key == "melee_damage" {
                serde_json::json!(0)
            } else {
                serde_json::json!(value)
            },
        );
        assert!(SimHarness::new(&bad).is_err(), "{key}");
    }
}

#[test]
fn hit_stun_and_pogo_cancel_an_active_dash_without_recharging_it() {
    for pogo in [true, false] {
        let mut level = arena(true);
        level.rules.insert(
            "mechanics".into(),
            serde_json::json!([
                "movement",
                "jump",
                "dash",
                "double_jump",
                "wall_jump",
                "melee"
            ]),
        );
        let mut sim = SimHarness::new(&level).unwrap();
        let player = sim.spawn_player(PlayerId(0));
        let source = target(&mut sim, Vec2::new(180.0, 136.0), 5);
        let mut inputs = attack(127, -127);
        inputs.0.get_mut(&PlayerId(0)).unwrap().buttons |= buttons::DASH;
        if !pogo {
            damage(
                &mut sim,
                DamageEvent {
                    target: player,
                    source: Some(source),
                    amount: 1,
                },
            );
        }
        sim.step(1, &inputs);
        let e = entity(&mut sim, player);
        let mobility = sim
            .world()
            .get::<mw_sim::mechanics::MobilityState>(e)
            .unwrap();
        assert_eq!(mobility.dash_ticks, 0);
        assert!(
            !mobility.dash_available,
            "a hit or pogo does not recharge dash"
        );
        let expected = if pogo {
            Rules::default().pogo_speed
        } else {
            Rules::default().knockback_y
        };
        assert_eq!(sim.body(player).velocity.y, expected);
        sim.step(1, &TickInputs::default());
        assert!(
            sim.body(player).velocity.y > 0.0,
            "dash must not cancel the bounce or knockback"
        );
    }
}
