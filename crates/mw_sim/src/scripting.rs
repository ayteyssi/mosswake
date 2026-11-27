//! Level scripts inside the simulation: hooks at the right [`SimSet`]s, the world shown to
//! scripts, and the [`ScriptCommand`]s they return applied to the ECS.
//!
//! Scripts run only where a [`ScriptRuntime`] exists (server, solo play); predicted client
//! simulations have none, so they never run scripts. Script positions are in **tiles**.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use mw_level::LevelBundle;
use mw_script::{EntityData, HookOutcome, Limits, ScriptCommand, ScriptError, ScriptHost};

use crate::combat::{DamageEvent, DamageQueue, Health, Hurtbox};
use crate::{
    seconds_to_ticks, Aabb, Body, KnownMechanics, LoadedLevel, Player, Position, Rules, SimId,
    SimIds, SimSet, Velocity, TICK_DT, TICK_HZ,
};

/// Entity kind of script trigger zones (`props`: `name`, `w`, `h` in tiles).
pub const TRIGGER: &str = "trigger";

/// Tags set by scripts on a body.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptTags(pub BTreeSet<String>);

/// Custom variables set by scripts on a body.
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct ScriptVars(pub BTreeMap<String, serde_json::Value>);

/// Kind of a non-player entity (`"slime_green"`, a script-defined kind…).
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct EntityKind(pub String);

/// A zone calling `on_trigger(name, entity)` when a body enters it. Anchored like other
/// entities: `Position` is the bottom-center.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct Trigger {
    pub name: String,
    /// World units.
    pub size: Vec2,
    /// Bodies inside at the end of the last tick.
    pub inside: BTreeSet<SimId>,
}

/// `on_player_spawn` was called for this player.
#[derive(Component)]
struct SpawnAnnounced;

/// The level's scripts and the state the hooks need between ticks.
#[derive(Resource)]
pub struct ScriptRuntime {
    host: ScriptHost,
    started: bool,
    /// Pairs of bodies in contact at the end of the last tick (for `on_collision`).
    touching: BTreeSet<(SimId, SimId)>,
}

impl ScriptRuntime {
    pub fn host(&self) -> &ScriptHost {
        &self.host
    }
}

/// Script output and errors, for the server log and the debug console (#22).
#[derive(Resource, Debug, Default)]
pub struct ScriptLog {
    pub lines: Vec<String>,
    pub errors: Vec<ScriptError>,
    /// Commands the simulation refused (bad rule value, unknown id…).
    pub rejected: Vec<String>,
}

/// Entries kept per list of [`ScriptLog`] when nobody drains it (oldest dropped first).
pub const SCRIPT_LOG_CAPACITY: usize = 256;

impl ScriptLog {
    /// Removes and returns everything logged so far (the server prints it every tick).
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    fn cap(&mut self) {
        fn trim<T>(list: &mut Vec<T>) {
            let excess = list.len().saturating_sub(SCRIPT_LOG_CAPACITY);
            list.drain(..excess);
        }
        trim(&mut self.lines);
        trim(&mut self.errors);
        trim(&mut self.rejected);
    }
}

/// HUD messages requested by scripts, with the ticks they still stay on screen (#11 draws).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct HudMessages(pub Vec<(String, u32)>);

pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<ScriptLog>()
        .init_resource::<HudMessages>()
        .add_systems(FixedUpdate, scripts_pre.in_set(SimSet::ScriptPre))
        .add_systems(FixedUpdate, scripts_post.in_set(SimSet::ScriptPost))
        .add_systems(FixedUpdate, age_hud.in_set(SimSet::Cleanup));
}

/// Compiles `bundle`'s scripts, spawns its trigger zones and starts running hooks.
/// Call after [`crate::load_level`].
pub fn load_scripts(world: &mut World, bundle: &LevelBundle) -> Result<(), Vec<ScriptError>> {
    let mut host = ScriptHost::from_bundle(bundle, Limits::default())?;
    host.set_tick_rate(TICK_HZ);
    let ts = bundle.level.tile_size as f32;
    for entity in bundle.level.entities.iter().filter(|e| e.kind == TRIGGER) {
        let props = serde_json::Value::Object(entity.props.clone().into_iter().collect());
        spawn_entity(world, TRIGGER, Vec2::new(entity.x, entity.y) * ts, &props);
    }
    if bundle.level.scripts.is_empty() {
        return Ok(()); // nothing to run: no per-tick cost for unscripted levels
    }
    world.insert_resource(ScriptRuntime {
        host,
        started: false,
        touching: BTreeSet::new(),
    });
    Ok(())
}

fn tile_size(world: &World) -> f32 {
    world
        .get_resource::<LoadedLevel>()
        .map_or(32.0, |l| l.0.tile_size as f32)
}

/// What scripts see: every body, by id, in tiles.
fn view(world: &mut World) -> Vec<EntityData> {
    let ts = tile_size(world);
    let mut query = world.query::<(
        &SimId,
        &Position,
        Option<&Velocity>,
        Option<&Player>,
        Option<&Health>,
        Option<&EntityKind>,
        Option<&Trigger>,
        Option<&ScriptTags>,
        Option<&ScriptVars>,
    )>();
    let mut bodies: Vec<EntityData> = query
        .iter(world)
        .map(
            |(id, pos, vel, player, health, kind, trigger, tags, vars)| {
                let kind = match (player, kind, trigger) {
                    (Some(_), _, _) => "player".to_string(),
                    (_, Some(k), _) => k.0.clone(),
                    (_, _, Some(_)) => TRIGGER.to_string(),
                    _ => "body".to_string(),
                };
                let v = vel.map_or(Vec2::ZERO, |v| v.0) / ts;
                EntityData {
                    id: i64::from(id.0),
                    kind,
                    player: player.map(|p| i64::from(p.0 .0)),
                    x: f64::from(pos.0.x / ts),
                    y: f64::from(pos.0.y / ts),
                    vx: f64::from(v.x),
                    vy: f64::from(v.y),
                    hp: health.map(|h| (i64::from(h.current), i64::from(h.max))),
                    tags: tags.map(|t| t.0.clone()).unwrap_or_default(),
                    vars: vars.map(|v| v.0.clone()).unwrap_or_default(),
                }
            },
        )
        .collect();
    bodies.sort_by_key(|b| b.id);
    bodies
}

/// Gives scripts the current world and rules.
fn prepare(world: &mut World, runtime: &mut ScriptRuntime) {
    runtime.host.set_view(view(world));
    let rules = serde_json::to_value(world.resource::<Rules>()).unwrap_or_default();
    runtime.host.set_rules(&rules);
}

/// A script value for the body `id` (or `()` if it is gone).
fn entity_arg(world: &mut World, runtime: &ScriptRuntime, id: SimId) -> rhai::Dynamic {
    view(world)
        .into_iter()
        .find(|b| b.id == i64::from(id.0))
        .map_or(rhai::Dynamic::UNIT, |b| runtime.host.entity(b))
}

/// Runs `f` with the runtime taken out of the world (if scripts are loaded).
fn with_runtime<R>(
    world: &mut World,
    f: impl FnOnce(&mut World, &mut ScriptRuntime) -> R,
) -> Option<R> {
    if !world.contains_resource::<ScriptRuntime>() {
        return None;
    }
    Some(world.resource_scope(|world, mut runtime: Mut<ScriptRuntime>| f(world, &mut runtime)))
}

/// Calls a hook with fresh state and applies what it returns.
fn hook(
    world: &mut World,
    runtime: &mut ScriptRuntime,
    call: impl FnOnce(&mut World, &mut ScriptRuntime) -> HookOutcome,
) {
    prepare(world, runtime);
    let outcome = call(world, runtime);
    apply(world, outcome);
}

fn scripts_pre(world: &mut World) {
    with_runtime(world, |world, runtime| {
        if !runtime.started {
            runtime.started = true;
            hook(world, runtime, |_, rt| rt.host.on_level_start());
        }
        let mut query =
            world.query_filtered::<(Entity, &SimId), (With<Player>, Without<SpawnAnnounced>)>();
        let mut new_players: Vec<(SimId, Entity)> =
            query.iter(world).map(|(e, id)| (*id, e)).collect();
        new_players.sort();
        for (id, entity) in new_players {
            world.entity_mut(entity).insert(SpawnAnnounced);
            hook(world, runtime, |world, rt| {
                let arg = entity_arg(world, rt, id);
                rt.host.call_hook("on_player_spawn", (arg,))
            });
        }
        hook(world, runtime, |_, rt| rt.host.on_tick(f64::from(TICK_DT)));
    });
}

fn scripts_post(world: &mut World) {
    with_runtime(world, |world, runtime| {
        // Deaths of this tick, in their stable order.
        let deaths = world
            .resource::<crate::combat::CombatEvents>()
            .deaths
            .clone();
        for death in deaths {
            hook(world, runtime, |world, rt| {
                let target = entity_arg(world, rt, death.target);
                let source = death
                    .source
                    .map_or(rhai::Dynamic::UNIT, |s| entity_arg(world, rt, s));
                // `on_death(target)` or `on_death(target, source)`: both are hooks.
                let mut outcome = rt.host.call_hook("on_death", (target.clone(),));
                let both = rt.host.call_hook("on_death", (target, source));
                outcome.commands.extend(both.commands);
                outcome.errors.extend(both.errors);
                outcome
            });
        }
        // Contacts that began this tick (pairs are only tracked if a script listens: the set
        // of hooks never changes after loading).
        if runtime.host.has_hook("on_collision", 2) {
            collisions(world, runtime);
        }
        if runtime.host.has_hook("on_trigger", 2) {
            triggers(world, runtime);
        }
    });
}

fn collisions(world: &mut World, runtime: &mut ScriptRuntime) {
    let boxes = body_boxes(world);
    let mut touching = BTreeSet::new();
    for (i, (a, box_a)) in boxes.iter().enumerate() {
        for (b, box_b) in &boxes[i + 1..] {
            if overlap(*box_a, *box_b) {
                touching.insert((*a, *b));
            }
        }
    }
    let started: Vec<(SimId, SimId)> = touching.difference(&runtime.touching).copied().collect();
    runtime.touching = touching;
    for (a, b) in started {
        hook(world, runtime, |world, rt| {
            let (a, b) = (entity_arg(world, rt, a), entity_arg(world, rt, b));
            rt.host.call_hook("on_collision", (a, b))
        });
    }
}

/// Bodies entering trigger zones, with positions as left by the hooks before.
fn triggers(world: &mut World, runtime: &mut ScriptRuntime) {
    let boxes = body_boxes(world);
    let mut query = world.query::<(Entity, &SimId, &Position, &Trigger)>();
    let mut triggers: Vec<(SimId, Entity, Vec2, Trigger)> = query
        .iter(world)
        .map(|(e, id, p, t)| (*id, e, p.0, t.clone()))
        .collect();
    triggers.sort_by_key(|(id, ..)| *id);
    for (_, entity, feet, trigger) in triggers {
        let zone = Aabb::from_feet(feet, trigger.size);
        let inside: BTreeSet<SimId> = boxes
            .iter()
            .filter(|(_, b)| overlap(zone, *b))
            .map(|(id, _)| *id)
            .collect();
        let entered: Vec<SimId> = inside.difference(&trigger.inside).copied().collect();
        if let Some(mut t) = world.get_mut::<Trigger>(entity) {
            t.inside = inside;
        }
        for id in entered {
            let name = trigger.name.clone();
            hook(world, runtime, |world, rt| {
                let e = entity_arg(world, rt, id);
                rt.host.call_hook("on_trigger", (name, e))
            });
        }
    }
}

/// Solid bodies (not trigger zones), by id.
fn body_boxes(world: &mut World) -> Vec<(SimId, Aabb)> {
    let mut query = world.query::<(&SimId, &Position, &Body)>();
    let mut boxes: Vec<(SimId, Aabb)> = query
        .iter(world)
        .map(|(id, p, b)| (*id, Aabb::from_feet(p.0, b.size)))
        .collect();
    boxes.sort_by_key(|(id, _)| *id);
    boxes
}

fn overlap(a: Aabb, b: Aabb) -> bool {
    a.min.x < b.max.x && b.min.x < a.max.x && a.min.y < b.max.y && b.min.y < a.max.y
}

/// `on_damage(target, source, amount)` may change the amount (0 cancels the hit). Called by
/// the combat system for every damage about to be applied.
pub(crate) fn filter_damage(world: &mut World, event: &DamageEvent) -> u32 {
    let filtered = with_runtime(world, |world, runtime| {
        if !runtime.host.has_hook("on_damage", 3) {
            return None;
        }
        prepare(world, runtime);
        let target = entity_arg(world, runtime, event.target);
        let source = event
            .source
            .map_or(rhai::Dynamic::UNIT, |s| entity_arg(world, runtime, s));
        let (amount, outcome) =
            runtime
                .host
                .filter_hook("on_damage", vec![target, source], i64::from(event.amount));
        apply(world, outcome);
        Some(amount.clamp(0, i64::from(u32::MAX)) as u32)
    });
    filtered.flatten().unwrap_or(event.amount)
}

fn find(world: &mut World, id: i64) -> Option<Entity> {
    let id = SimId(u32::try_from(id).ok()?);
    let mut query = world.query::<(Entity, &SimId)>();
    query.iter(world).find(|(_, s)| **s == id).map(|(e, _)| e)
}

/// Applies a hook's commands in order; refused ones are logged, never fatal.
fn apply(world: &mut World, outcome: HookOutcome) {
    let ts = tile_size(world);
    world
        .resource_mut::<ScriptLog>()
        .errors
        .extend(outcome.errors);
    for command in outcome.commands {
        let refused = match command {
            ScriptCommand::Log { message } => {
                world.resource_mut::<ScriptLog>().lines.push(message);
                None
            }
            ScriptCommand::Hud { text, seconds } => {
                let ticks = seconds_to_ticks(seconds as f32).max(1);
                world.resource_mut::<HudMessages>().0.push((text, ticks));
                None
            }
            ScriptCommand::SetRule { key, value } => set_rule(world, &key, value),
            ScriptCommand::SetPosition { id, x, y } => find(world, id)
                .and_then(|e| world.get_mut::<Position>(e))
                .map(|mut p| p.0 = Vec2::new(x as f32, y as f32) * ts)
                .map_or(Some(format!("no body {id}")), |_| None),
            ScriptCommand::SetVelocity { id, vx, vy } => find(world, id)
                .and_then(|e| world.get_mut::<Velocity>(e))
                .map(|mut v| v.0 = Vec2::new(vx as f32, vy as f32) * ts)
                .map_or(Some(format!("no moving body {id}")), |_| None),
            ScriptCommand::Damage { id, amount, .. } => match u32::try_from(id) {
                Ok(target) => {
                    world.resource_mut::<DamageQueue>().0.push(DamageEvent {
                        target: SimId(target),
                        source: None,
                        amount: amount.clamp(0, i64::from(u32::MAX)) as u32,
                    });
                    None
                }
                Err(_) => Some(format!("no body {id}")),
            },
            ScriptCommand::Heal { id, amount } => find(world, id)
                .and_then(|e| world.get_mut::<Health>(e))
                .map(|mut h| {
                    let healed = i64::from(h.current).saturating_add(amount.max(0));
                    h.current = healed.min(i64::from(h.max)) as u32;
                })
                .map_or(Some(format!("no damageable body {id}")), |_| None),
            ScriptCommand::SetTag { id, tag, on } => match find(world, id) {
                Some(e) => {
                    let mut entity = world.entity_mut(e);
                    if !entity.contains::<ScriptTags>() {
                        entity.insert(ScriptTags::default());
                    }
                    let mut tags = entity.get_mut::<ScriptTags>().expect("inserted above");
                    if on {
                        tags.0.insert(tag);
                    } else {
                        tags.0.remove(&tag);
                    }
                    None
                }
                None => Some(format!("no body {id}")),
            },
            ScriptCommand::SetVar { id, key, value } => match find(world, id) {
                Some(e) => {
                    let mut entity = world.entity_mut(e);
                    if !entity.contains::<ScriptVars>() {
                        entity.insert(ScriptVars::default());
                    }
                    entity
                        .get_mut::<ScriptVars>()
                        .expect("inserted above")
                        .0
                        .insert(key, value);
                    None
                }
                None => Some(format!("no body {id}")),
            },
            ScriptCommand::Spawn { kind, x, y, props } => {
                spawn_entity(world, &kind, Vec2::new(x as f32, y as f32) * ts, &props);
                None
            }
            ScriptCommand::Despawn { id } => match find(world, id) {
                Some(e) => {
                    world.despawn(e);
                    None
                }
                None => Some(format!("no body {id}")),
            },
        };
        if let Some(reason) = refused {
            world.resource_mut::<ScriptLog>().rejected.push(reason);
        }
    }
    world.resource_mut::<ScriptLog>().cap();
}

/// A rule change from a script, checked like a `level.json` override.
fn set_rule(world: &mut World, key: &str, value: serde_json::Value) -> Option<String> {
    let overrides = BTreeMap::from([(key.to_string(), value)]);
    let rules = match world.resource::<Rules>().with_overrides(&overrides) {
        Ok(rules) => rules,
        Err(errors) => return Some(format!("rule `{key}`: {errors:?}")),
    };
    let known = world
        .get_resource::<KnownMechanics>()
        .cloned()
        .unwrap_or_default();
    let errors = rules.validate(&known);
    if !errors.is_empty() {
        return Some(format!("rule `{key}`: {errors:?}"));
    }
    world.insert_resource(rules);
    None
}

/// Smallest side of a spawned entity, in tiles.
const MIN_SIZE_TILES: f32 = 1.0 / 16.0;

/// A finite numeric prop, or `default`.
fn prop_f32(props: &serde_json::Value, key: &str, default: f32) -> f32 {
    props
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

/// Spawns a script or level entity at `feet` (world units). `props`: `w`, `h` (tiles,
/// default 1), `gravity` (scale, default 1), `hp` (makes it damageable), `name` (triggers);
/// all of them are also kept as the entity's variables.
pub fn spawn_entity(world: &mut World, kind: &str, feet: Vec2, props: &serde_json::Value) -> SimId {
    let ts = tile_size(world);
    // Bounded by the level: collision cost grows with the cells a body covers, so a
    // script must not be able to create a body millions of tiles wide.
    let (max_w, max_h) = world
        .get_resource::<LoadedLevel>()
        .map_or((64.0, 64.0), |l| {
            (l.0.size.w.max(1) as f32, l.0.size.h.max(1) as f32)
        });
    let size = Vec2::new(
        prop_f32(props, "w", 1.0).clamp(MIN_SIZE_TILES, max_w),
        prop_f32(props, "h", 1.0).clamp(MIN_SIZE_TILES, max_h),
    ) * ts;
    let id = world.resource_mut::<SimIds>().allocate();
    let vars = ScriptVars(
        props
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default(),
    );
    if kind == TRIGGER {
        let name = props
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default();
        world.spawn((
            id,
            Position(feet),
            Trigger {
                name: name.to_string(),
                size,
                inside: BTreeSet::new(),
            },
            vars,
        ));
        return id;
    }
    let mut entity = world.spawn((
        id,
        EntityKind(kind.to_string()),
        Body {
            size,
            gravity_scale: prop_f32(props, "gravity", 1.0),
        },
        Position(feet),
        Velocity::default(),
        crate::Contacts::default(),
        vars,
    ));
    if let Some(hp) = props.get("hp").and_then(serde_json::Value::as_u64) {
        let hp = u32::try_from(hp).unwrap_or(u32::MAX).max(1);
        entity.insert((Health::new(hp), Hurtbox::from_size(size)));
    }
    id
}

fn age_hud(mut hud: ResMut<HudMessages>) {
    for (_, ticks) in hud.0.iter_mut() {
        *ticks = ticks.saturating_sub(1);
    }
    hud.0.retain(|(_, ticks)| *ticks > 0);
}
