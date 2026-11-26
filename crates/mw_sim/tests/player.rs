//! Player controller: one headless test per behavior, driven by scripted inputs.
//! Expected tick counts are derived from `Rules`, so retuning values keeps tests meaningful.

use mw_core::input::{buttons, PlayerId, PlayerInput};
use mw_level::{Layer, LayerKind, Level, Size, Spawn};
use mw_sim::{
    seconds_to_ticks, Facing, PlayerState, Rules, SimHarness, SimId, TickInputs, TICK_DT,
};

const TS: f32 = 32.0;

/// 30 x 12 room. Left part is a ledge (top at y = 4 tiles, edge at x = 11 tiles), the rest
/// is a low floor (top at y = 1). Spawns: 0 on the ledge, 1 on the low floor, 2 high in the air.
fn ledge_level() -> Level {
    let mut rows = vec!["#".repeat(30)];
    for _ in 0..7 {
        rows.push(format!("#{}#", ".".repeat(28)));
    }
    for _ in 0..3 {
        rows.push(format!("{}{}#", "#".repeat(11), ".".repeat(18)));
    }
    rows.push("#".repeat(30));
    let mut level = Level::new("ledge", "Ledge", Size { w: 30, h: 12 });
    level.legend.insert('#', "mossy_ground".into());
    level.layers.push(Layer {
        name: "ground".into(),
        kind: LayerKind::Solid,
        tiles: rows,
    });
    level.spawns = vec![
        Spawn { x: 5.5, y: 4.0 },
        Spawn { x: 20.5, y: 1.0 },
        Spawn { x: 20.5, y: 8.0 },
    ];
    level
}

fn sim(level: &Level, player: u8) -> (SimHarness, SimId) {
    let mut sim = SimHarness::new(level).expect("level loads");
    let id = sim.spawn_player(PlayerId(player));
    (sim, id)
}

fn input(axis_x: i8, jump: bool) -> TickInputs {
    let mut inputs = TickInputs::default();
    inputs.0.insert(
        PlayerId(0),
        PlayerInput {
            axis_x,
            axis_y: 0,
            buttons: if jump { buttons::JUMP } else { 0 },
        },
    );
    // Same input for every player slot used by these tests.
    let p0 = inputs.0[&PlayerId(0)];
    inputs.0.insert(PlayerId(1), p0);
    inputs.0.insert(PlayerId(2), p0);
    inputs
}

const IDLE: (i8, bool) = (0, false);
const RIGHT: (i8, bool) = (127, false);
const JUMP: (i8, bool) = (0, true);

fn ticks((x, j): (i8, bool), n: usize) -> Vec<TickInputs> {
    vec![input(x, j); n]
}

fn settle(sim: &mut SimHarness, id: SimId) {
    sim.step(30, &input(0, false));
    assert!(sim.body(id).contacts.grounded, "settled on the ground");
}

#[test]
fn running_accelerates_to_run_speed_then_brakes() {
    let rules = Rules::default();
    let (mut sim, id) = sim(&ledge_level(), 1);
    settle(&mut sim, id);

    let accel_ticks = (rules.run_speed / (rules.ground_accel * TICK_DT)).ceil() as usize;
    let trace = sim.run_script(id, &ticks(RIGHT, accel_ticks));
    assert!(trace[accel_ticks - 2].velocity.x < rules.run_speed);
    assert_eq!(trace[accel_ticks - 1].velocity.x, rules.run_speed);
    assert!(trace[0].velocity.x > 0.0 && trace[0].velocity.x <= rules.ground_accel * TICK_DT);

    let brake_ticks = (rules.run_speed / (rules.ground_decel * TICK_DT)).ceil() as usize;
    let trace = sim.run_script(id, &ticks(IDLE, brake_ticks));
    assert!(trace[brake_ticks - 2].velocity.x > 0.0);
    assert_eq!(trace[brake_ticks - 1].velocity.x, 0.0);
    assert_eq!(sim.player_state(id), (PlayerState::Idle, Facing::Right));
}

#[test]
fn air_control_is_weaker_than_ground_control() {
    let rules = Rules::default();
    let (mut sim, id) = sim(&ledge_level(), 2);
    let s = sim.run_script(id, &ticks(RIGHT, 1));
    assert!(!s[0].contacts.grounded, "spawned in the air");
    assert_eq!(s[0].velocity.x, rules.air_accel * TICK_DT);
}

/// Highest point reached (relative to the start) while running `script`.
fn apex(sim: &mut SimHarness, id: SimId, script: &[TickInputs]) -> f32 {
    let start = sim.body(id).position.y;
    sim.run_script(id, script)
        .iter()
        .map(|s| s.position.y - start)
        .fold(0.0, f32::max)
}

#[test]
fn full_jump_reaches_jump_height() {
    let rules = Rules::default();
    let (mut sim, id) = sim(&ledge_level(), 1);
    settle(&mut sim, id);
    let rise = apex(&mut sim, id, &ticks(JUMP, 60));
    assert!(
        (rise - rules.jump_height).abs() < 1.0,
        "rise {rise} vs jump_height {}",
        rules.jump_height
    );
}

#[test]
fn releasing_early_cuts_the_jump() {
    let rules = Rules::default();
    let (mut sim, id) = sim(&ledge_level(), 1);
    settle(&mut sim, id);
    let mut script = ticks(JUMP, 1);
    script.extend(ticks(IDLE, 60));
    let tap = apex(&mut sim, id, &script);
    assert!(tap > 0.0 && tap < 0.5 * rules.jump_height, "tap rise {tap}");

    // Holding longer gives a higher jump: height is variable.
    let (mut sim, id) = self::sim(&ledge_level(), 1);
    settle(&mut sim, id);
    let mut script = ticks(JUMP, 10);
    script.extend(ticks(IDLE, 60));
    let medium = apex(&mut sim, id, &script);
    assert!(
        tap < medium && medium < rules.jump_height,
        "medium rise {medium}"
    );
}

/// Walks right off the ledge, then presses jump `late` ticks after leaving the ground.
/// Returns whether a jump happened.
fn jump_after_leaving_ledge(late: u32) -> bool {
    let (mut sim, id) = sim(&ledge_level(), 0);
    settle(&mut sim, id);
    for _ in 0..200 {
        sim.step(1, &input(127, false));
        if !sim.body(id).contacts.grounded {
            break;
        }
    }
    assert!(!sim.body(id).contacts.grounded, "walked off the ledge");
    sim.step(late - 1, &input(127, false));
    sim.step(1, &input(127, true));
    sim.body(id).velocity.y > 0.0
}

#[test]
fn coyote_time_allows_late_jumps_only_within_the_window() {
    let window = seconds_to_ticks(Rules::default().coyote_time);
    assert!(jump_after_leaving_ledge(1));
    assert!(
        jump_after_leaving_ledge(window - 1),
        "last tick of coyote time"
    );
    assert!(!jump_after_leaving_ledge(window), "coyote time is over");
}

/// Drops from spawn 2, presses (and holds) jump `early` ticks before the tick where the
/// jump could trigger on landing. Returns whether the player bounced off the ground.
fn jump_before_landing(early: u32) -> bool {
    // Dry run to find the landing tick.
    let (mut sim, id) = sim(&ledge_level(), 2);
    let landing = (1..200)
        .find(|_| {
            sim.step(1, &input(0, false));
            sim.body(id).contacts.grounded
        })
        .expect("lands");
    // The jump can trigger the tick after landing (contacts are from the previous tick).
    let trigger = landing + 1;
    let (mut sim, id) = self::sim(&ledge_level(), 2);
    sim.step(trigger - early, &input(0, false));
    sim.step(early + 5, &input(0, true));
    sim.body(id).position.y > TS + 1.0
}

#[test]
fn jump_buffer_triggers_on_landing_only_within_the_window() {
    let window = seconds_to_ticks(Rules::default().jump_buffer);
    assert!(jump_before_landing(1));
    assert!(
        jump_before_landing(window - 1),
        "pressed at the start of the buffer"
    );
    assert!(!jump_before_landing(window + 1), "pressed too early");
}

#[test]
fn no_jump_in_mid_air() {
    let (mut sim, id) = sim(&ledge_level(), 2);
    sim.step(20, &input(0, false)); // past any coyote window, still falling
    let before = sim.body(id).velocity.y;
    sim.step(1, &input(0, true));
    let after = sim.body(id).velocity.y;
    assert!(after < before, "kept falling: {before} -> {after}");
}

#[test]
fn holding_jump_jumps_once() {
    let (mut sim, id) = sim(&ledge_level(), 1);
    settle(&mut sim, id);
    // Include the state before the first tick: the takeoff happens on that tick.
    let mut trace = vec![sim.body(id)];
    trace.extend(sim.run_script(id, &ticks(JUMP, 240)));
    let takeoffs = trace
        .windows(2)
        .filter(|w| w[0].contacts.grounded && !w[1].contacts.grounded)
        .count();
    assert_eq!(takeoffs, 1, "a held button must be released to jump again");
    assert!(trace.last().unwrap().contacts.grounded);
}

#[test]
fn states_and_facing_follow_the_action() {
    let (mut sim, id) = sim(&ledge_level(), 1);
    settle(&mut sim, id);
    assert_eq!(sim.player_state(id), (PlayerState::Idle, Facing::Right));

    sim.step(5, &input(-127, false));
    assert_eq!(sim.player_state(id), (PlayerState::Run, Facing::Left));

    sim.step(3, &input(-127, true));
    assert_eq!(sim.player_state(id).0, PlayerState::Jump);

    sim.step(25, &input(0, true));
    assert_eq!(sim.player_state(id).0, PlayerState::Fall);

    sim.step(60, &input(0, false));
    assert_eq!(
        sim.player_state(id),
        (PlayerState::Idle, Facing::Left),
        "facing is kept after releasing the stick"
    );
}

#[test]
fn pushing_into_a_wall_is_not_running() {
    let (mut sim, id) = sim(&ledge_level(), 1);
    settle(&mut sim, id);
    sim.step(120, &input(127, false)); // reach the right wall
    assert!(sim.body(id).contacts.on_wall_right);
    assert_eq!(sim.player_state(id).0, PlayerState::Idle);
}

#[test]
fn mechanics_can_be_disabled_by_the_level() {
    let mut level = ledge_level();
    level
        .rules
        .insert("mechanics".into(), serde_json::json!(["jump"]));
    let (mut sim, id) = sim(&level, 1);
    settle(&mut sim, id);
    sim.step(30, &input(127, false));
    assert_eq!(sim.body(id).velocity.x, 0.0, "no movement mechanic");

    level
        .rules
        .insert("mechanics".into(), serde_json::json!(["movement"]));
    let (mut sim, id) = self::sim(&level, 1);
    settle(&mut sim, id);
    assert_eq!(
        apex(&mut sim, id, &ticks(JUMP, 30)),
        0.0,
        "no jump mechanic"
    );
}

#[test]
fn level_rules_tune_the_controller() {
    let mut level = ledge_level();
    level
        .rules
        .insert("run_speed".into(), serde_json::json!(100));
    level
        .rules
        .insert("jump_height".into(), serde_json::json!(64));
    let (mut sim, id) = sim(&level, 1);
    settle(&mut sim, id);
    sim.step(30, &input(-127, false));
    assert_eq!(sim.body(id).velocity.x, -100.0);
    let rise = apex(&mut sim, id, &ticks((-127, true), 60));
    assert!((rise - 64.0).abs() < 1.0, "rise {rise}");
}

#[test]
fn inverted_gravity_jumps_downwards() {
    let mut level = ledge_level();
    level
        .rules
        .insert("gravity".into(), serde_json::json!(1800));
    let (mut sim, id) = sim(&level, 2);
    sim.step(60, &input(0, false));
    let s = sim.body(id);
    assert!(s.contacts.grounded, "standing on the ceiling");
    sim.step(5, &input(0, true));
    assert!(
        sim.body(id).position.y < s.position.y,
        "jumped away from the ceiling"
    );
    assert_eq!(sim.player_state(id).0, PlayerState::Jump);
}

#[test]
fn players_are_deterministic_under_scripted_inputs() {
    let run = || {
        let level = ledge_level();
        let mut sim = SimHarness::new(&level).unwrap();
        let ids: Vec<_> = (0..3).map(|p| sim.spawn_player(PlayerId(p))).collect();
        for t in 0..400u32 {
            let mut inputs = TickInputs::default();
            for p in 0..3u8 {
                let phase = t.wrapping_mul(7).wrapping_add(u32::from(p) * 13);
                inputs.0.insert(
                    PlayerId(p),
                    PlayerInput {
                        axis_x: ((phase % 255) as i16 - 127) as i8,
                        axis_y: 0,
                        buttons: if phase % 17 < 6 { buttons::JUMP } else { 0 },
                    },
                );
            }
            sim.step(1, &inputs);
        }
        ids.iter()
            .map(|&id| {
                let s = sim.body(id);
                (
                    s.position.x.to_bits(),
                    s.position.y.to_bits(),
                    s.velocity.x.to_bits(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}
