//! The world API exposed to scripts: [`Entity`] values, `players()` / `entities()`,
//! `spawn`, timers and mechanics toggles. Still free of any ECS: the simulation provides a
//! view ([`EntityData`]) before each hook and applies the [`ScriptCommand`]s afterwards.
//!
//! Units are **tiles** (positions) and **tiles per second** (velocities), like `level.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use rhai::{Array, Dynamic, Engine, EvalAltResult, FnPtr, ImmutableString, Map, INT};

use crate::command::{dynamic_to_json, ScriptCommand};

/// A body as scripts see it at the start of a hook.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityData {
    /// Simulation id (stable, as in snapshots).
    pub id: i64,
    /// `"player"` for players, the level entity kind otherwise (e.g. `"slime_green"`).
    pub kind: String,
    /// Player slot for players.
    pub player: Option<i64>,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    /// `(current, max)` health, when the body can take damage.
    pub hp: Option<(i64, i64)>,
    pub tags: BTreeSet<String>,
    pub vars: BTreeMap<String, serde_json::Value>,
}

pub(crate) type Sink = Arc<Mutex<Vec<ScriptCommand>>>;

fn push(sink: &Sink, command: ScriptCommand) {
    sink.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(command);
}

/// The value scripts manipulate. Reads see the state at the start of the hook plus this
/// script's own writes; writes become commands applied by the simulation after the hook.
#[derive(Debug, Clone)]
pub struct Entity {
    data: EntityData,
    sink: Sink,
}

impl Entity {
    pub(crate) fn new(data: EntityData, sink: Sink) -> Self {
        Self { data, sink }
    }

    fn emit(&self, command: ScriptCommand) {
        push(&self.sink, command);
    }
}

/// A pending timer, run by the host on the tick it is due.
#[derive(Debug, Clone)]
pub(crate) struct Timer {
    pub id: INT,
    pub script: usize,
    pub due: u64,
    pub period: Option<u64>,
    pub callback: FnPtr,
}

/// Timer bookkeeping shared between the host and the registered functions.
#[derive(Debug, Default)]
pub(crate) struct TimerBook {
    pub next_id: INT,
    /// Current tick (advanced by `on_tick`).
    pub now: u64,
    /// Script being run (timers belong to the script that created them).
    pub script: usize,
    pub ticks_per_second: f64,
    pub added: Vec<Timer>,
    pub cancelled: BTreeSet<INT>,
}

impl TimerBook {
    fn ticks(&self, seconds: f64) -> u64 {
        ((seconds.max(0.0) * self.ticks_per_second).round() as u64).max(1)
    }
}

/// Everything the registered functions share with the host.
#[derive(Clone)]
pub(crate) struct Shared {
    pub sink: Sink,
    pub view: Arc<Mutex<Vec<EntityData>>>,
    pub timers: Arc<Mutex<TimerBook>>,
    /// The `rules` object (a shared Rhai map, also resolved by `on_var`).
    pub rules: Dynamic,
}

/// Converts JSON (rules, entity variables, props) to a script value.
pub fn json_to_dynamic(value: &serde_json::Value) -> Dynamic {
    use serde_json::Value;
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from_bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => Dynamic::from_int(i),
            None => Dynamic::from_float(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Dynamic::from(s.clone()),
        Value::Array(items) => Dynamic::from_array(items.iter().map(json_to_dynamic).collect()),
        Value::Object(map) => Dynamic::from_map(
            map.iter()
                .map(|(k, v)| (k.as_str().into(), json_to_dynamic(v)))
                .collect(),
        ),
    }
}

fn to_float(value: &Dynamic) -> Result<f64, Box<EvalAltResult>> {
    if let Some(f) = value.clone().try_cast::<rhai::FLOAT>() {
        return Ok(f);
    }
    if let Some(i) = value.clone().try_cast::<INT>() {
        return Ok(i as f64);
    }
    Err(format!("expected a number, got {}", value.type_name()).into())
}

/// Registers the `Entity` type and the world functions on `engine`.
pub(crate) fn register(engine: &mut Engine, shared: &Shared) {
    engine.register_type_with_name::<Entity>("Entity");

    // Read-only properties.
    engine
        .register_get("id", |e: &mut Entity| e.data.id)
        .register_get("kind", |e: &mut Entity| {
            ImmutableString::from(e.data.kind.as_str())
        })
        .register_get("player", |e: &mut Entity| -> Dynamic {
            e.data.player.map_or(Dynamic::UNIT, Dynamic::from_int)
        })
        .register_get("is_player", |e: &mut Entity| e.data.player.is_some())
        .register_get("hp", |e: &mut Entity| -> Dynamic {
            e.data
                .hp
                .map_or(Dynamic::UNIT, |(hp, _)| Dynamic::from_int(hp))
        })
        .register_get("max_hp", |e: &mut Entity| -> Dynamic {
            e.data
                .hp
                .map_or(Dynamic::UNIT, |(_, max)| Dynamic::from_int(max))
        })
        .register_get("alive", |e: &mut Entity| {
            e.data.hp.is_none_or(|(hp, _)| hp > 0)
        });

    // Position and velocity: read, and write through commands.
    engine
        .register_get("x", |e: &mut Entity| e.data.x)
        .register_get("y", |e: &mut Entity| e.data.y)
        .register_get("vx", |e: &mut Entity| e.data.vx)
        .register_get("vy", |e: &mut Entity| e.data.vy)
        .register_set(
            "x",
            |e: &mut Entity, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
                e.data.x = to_float(&v)?;
                e.emit(ScriptCommand::SetPosition {
                    id: e.data.id,
                    x: e.data.x,
                    y: e.data.y,
                });
                Ok(())
            },
        )
        .register_set(
            "y",
            |e: &mut Entity, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
                e.data.y = to_float(&v)?;
                e.emit(ScriptCommand::SetPosition {
                    id: e.data.id,
                    x: e.data.x,
                    y: e.data.y,
                });
                Ok(())
            },
        )
        .register_set(
            "vx",
            |e: &mut Entity, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
                e.data.vx = to_float(&v)?;
                e.emit(ScriptCommand::SetVelocity {
                    id: e.data.id,
                    vx: e.data.vx,
                    vy: e.data.vy,
                });
                Ok(())
            },
        )
        .register_set(
            "vy",
            |e: &mut Entity, v: Dynamic| -> Result<(), Box<EvalAltResult>> {
                e.data.vy = to_float(&v)?;
                e.emit(ScriptCommand::SetVelocity {
                    id: e.data.id,
                    vx: e.data.vx,
                    vy: e.data.vy,
                });
                Ok(())
            },
        );

    // Actions.
    engine
        .register_fn(
            "teleport",
            |e: &mut Entity, x: Dynamic, y: Dynamic| -> Result<(), Box<EvalAltResult>> {
                e.data.x = to_float(&x)?;
                e.data.y = to_float(&y)?;
                e.data.vx = 0.0;
                e.data.vy = 0.0;
                e.emit(ScriptCommand::SetPosition {
                    id: e.data.id,
                    x: e.data.x,
                    y: e.data.y,
                });
                e.emit(ScriptCommand::SetVelocity {
                    id: e.data.id,
                    vx: 0.0,
                    vy: 0.0,
                });
                Ok(())
            },
        )
        .register_fn("damage", |e: &mut Entity, amount: INT| {
            e.emit(ScriptCommand::Damage {
                id: e.data.id,
                amount: amount.max(0),
                cause: "script".into(),
            });
        })
        .register_fn(
            "damage",
            |e: &mut Entity, amount: INT, cause: ImmutableString| {
                e.emit(ScriptCommand::Damage {
                    id: e.data.id,
                    amount: amount.max(0),
                    cause: cause.to_string(),
                });
            },
        )
        .register_fn("heal", |e: &mut Entity, amount: INT| {
            if let Some((hp, max)) = e.data.hp.as_mut() {
                *hp = (*hp + amount.max(0)).min(*max);
            }
            e.emit(ScriptCommand::Heal {
                id: e.data.id,
                amount: amount.max(0),
            });
        })
        .register_fn("despawn", |e: &mut Entity| {
            e.emit(ScriptCommand::Despawn { id: e.data.id })
        })
        .register_fn("has_tag", |e: &mut Entity, tag: ImmutableString| {
            e.data.tags.contains(tag.as_str())
        })
        .register_fn("add_tag", |e: &mut Entity, tag: ImmutableString| {
            e.data.tags.insert(tag.to_string());
            e.emit(ScriptCommand::SetTag {
                id: e.data.id,
                tag: tag.to_string(),
                on: true,
            });
        })
        .register_fn("remove_tag", |e: &mut Entity, tag: ImmutableString| {
            e.data.tags.remove(tag.as_str());
            e.emit(ScriptCommand::SetTag {
                id: e.data.id,
                tag: tag.to_string(),
                on: false,
            });
        })
        .register_fn("get", |e: &mut Entity, key: ImmutableString| -> Dynamic {
            e.data
                .vars
                .get(key.as_str())
                .map_or(Dynamic::UNIT, json_to_dynamic)
        })
        .register_fn(
            "set",
            |e: &mut Entity,
             key: ImmutableString,
             value: Dynamic|
             -> Result<(), Box<EvalAltResult>> {
                let json = dynamic_to_json(&value)
                    .ok_or_else(|| format!("set(\"{key}\", …): value cannot be stored"))?;
                e.data.vars.insert(key.to_string(), json.clone());
                e.emit(ScriptCommand::SetVar {
                    id: e.data.id,
                    key: key.to_string(),
                    value: json,
                });
                Ok(())
            },
        )
        .register_fn("to_string", |e: &mut Entity| {
            format!(
                "{}#{} ({:.2}, {:.2})",
                e.data.kind, e.data.id, e.data.x, e.data.y
            )
        })
        .register_fn("==", |a: Entity, b: Entity| a.data.id == b.data.id)
        .register_fn("!=", |a: Entity, b: Entity| a.data.id != b.data.id);

    // World queries (state at the start of the hook).
    let entities_of = |shared: &Shared, keep: fn(&EntityData) -> bool| -> Array {
        shared
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|d| keep(d))
            .map(|d| Dynamic::from(Entity::new(d.clone(), Arc::clone(&shared.sink))))
            .collect()
    };
    let s = shared.clone();
    engine.register_fn("players", move || entities_of(&s, |d| d.player.is_some()));
    let s = shared.clone();
    engine.register_fn("entities", move || entities_of(&s, |_| true));
    let s = shared.clone();
    engine.register_fn("entity", move |id: INT| -> Dynamic {
        s.view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|d| d.id == id)
            .map_or(Dynamic::UNIT, |d| {
                Dynamic::from(Entity::new(d.clone(), Arc::clone(&s.sink)))
            })
    });
    let s = shared.clone();
    engine.register_fn("entities_with_tag", move |tag: ImmutableString| -> Array {
        s.view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|d| d.tags.contains(tag.as_str()))
            .map(|d| Dynamic::from(Entity::new(d.clone(), Arc::clone(&s.sink))))
            .collect()
    });

    // Spawning.
    let s = shared.clone();
    engine.register_fn(
        "spawn_entity",
        move |kind: ImmutableString, x: Dynamic, y: Dynamic| -> Result<(), Box<EvalAltResult>> {
            push(
                &s.sink,
                ScriptCommand::Spawn {
                    kind: kind.to_string(),
                    x: to_float(&x)?,
                    y: to_float(&y)?,
                    props: serde_json::Value::Object(Default::default()),
                },
            );
            Ok(())
        },
    );
    let s = shared.clone();
    engine.register_fn(
        "spawn_entity",
        move |kind: ImmutableString,
              x: Dynamic,
              y: Dynamic,
              props: Map|
              -> Result<(), Box<EvalAltResult>> {
            let props = dynamic_to_json(&Dynamic::from_map(props))
                .ok_or("spawn_entity(…): props cannot be stored")?;
            push(
                &s.sink,
                ScriptCommand::Spawn {
                    kind: kind.to_string(),
                    x: to_float(&x)?,
                    y: to_float(&y)?,
                    props,
                },
            );
            Ok(())
        },
    );

    // Timers.
    let add_timer = |s: &Shared, seconds: f64, callback: FnPtr, repeat: bool| -> INT {
        let mut book = s.timers.lock().unwrap_or_else(PoisonError::into_inner);
        let ticks = book.ticks(seconds);
        let id = book.next_id;
        book.next_id += 1;
        let timer = Timer {
            id,
            script: book.script,
            due: book.now + ticks,
            period: repeat.then_some(ticks),
            callback,
        };
        book.added.push(timer);
        id
    };
    let s = shared.clone();
    engine.register_fn(
        "after",
        move |seconds: Dynamic, callback: FnPtr| -> Result<INT, Box<EvalAltResult>> {
            Ok(add_timer(&s, to_float(&seconds)?, callback, false))
        },
    );
    let s = shared.clone();
    engine.register_fn(
        "every",
        move |seconds: Dynamic, callback: FnPtr| -> Result<INT, Box<EvalAltResult>> {
            Ok(add_timer(&s, to_float(&seconds)?, callback, true))
        },
    );
    let s = shared.clone();
    engine.register_fn("cancel", move |id: INT| {
        s.timers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cancelled
            .insert(id);
    });

    // Mechanics: edit `rules.mechanics` (turned into a rule change after the hook).
    let toggle = |rules: &Dynamic, id: &str, on: bool| {
        // A clone of a shared value points to the same data: writing through it edits `rules`.
        let mut rules = rules.clone();
        let mut map = rules.write_lock::<Map>().expect("rules is a shared map");
        let mut list: Array = map
            .get("mechanics")
            .and_then(|m| m.clone().try_cast::<Array>())
            .unwrap_or_default();
        list.retain(|m| m.clone().into_string().ok().as_deref() != Some(id));
        if on {
            list.push(Dynamic::from(id.to_string()));
        }
        map.insert("mechanics".into(), Dynamic::from_array(list));
    };
    let s = shared.clone();
    engine.register_fn("enable_mechanic", move |id: ImmutableString| {
        toggle(&s.rules, &id, true)
    });
    let s = shared.clone();
    engine.register_fn("disable_mechanic", move |id: ImmutableString| {
        toggle(&s.rules, &id, false)
    });
    let s = shared.clone();
    engine.register_fn("mechanic_active", move |id: ImmutableString| -> bool {
        let map = s.rules.read_lock::<Map>().expect("rules is a shared map");
        map.get("mechanics")
            .and_then(|m| m.clone().try_cast::<Array>())
            .is_some_and(|list| {
                list.iter()
                    .any(|m| m.clone().into_string().ok().as_deref() == Some(id.as_str()))
            })
    });
}
