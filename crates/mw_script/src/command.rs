//! What scripts ask the game to do. Scripts never touch the simulation directly: they
//! return [`ScriptCommand`]s that the simulation validates and applies (see the Scripting
//! section of `docs/ARCHITECTURE.md`).

use rhai::Dynamic;
use serde::{Deserialize, Serialize};

/// A request emitted by a script during a hook call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScriptCommand {
    /// Output of `print(...)` / `debug(...)`, for the server log and the debug console.
    Log { message: String },
    /// `hud(text)` / `hud(text, seconds)`: show a message to every player.
    Hud { text: String, seconds: f64 },
    /// `set_rule(key, value)`: change a gameplay rule (checked by the simulation like a
    /// `level.json` override).
    SetRule {
        key: String,
        value: serde_json::Value,
    },
    /// `e.x = …`, `e.y = …`, `e.teleport(x, y)`: move a body (feet, tiles).
    SetPosition { id: i64, x: f64, y: f64 },
    /// `e.vx = …`, `e.vy = …`: set a body's velocity (tiles per second).
    SetVelocity { id: i64, vx: f64, vy: f64 },
    /// `e.damage(amount[, cause])`: queued damage (goes through `on_damage` and invincibility).
    Damage { id: i64, amount: i64, cause: String },
    /// `e.heal(amount)`: restore health, up to the maximum.
    Heal { id: i64, amount: i64 },
    /// `e.add_tag(t)` / `e.remove_tag(t)`.
    SetTag { id: i64, tag: String, on: bool },
    /// `e.set(key, value)`: a custom variable stored on the body.
    SetVar {
        id: i64,
        key: String,
        value: serde_json::Value,
    },
    /// `spawn_entity(kind, x, y[, props])`: create an entity (tiles).
    Spawn {
        kind: String,
        x: f64,
        y: f64,
        props: serde_json::Value,
    },
    /// `e.despawn()`: remove a body.
    Despawn { id: i64 },
}

/// Default display time of a HUD message, seconds.
pub const DEFAULT_HUD_SECONDS: f64 = 3.0;

/// Converts a script value to JSON, the format of rule values. Returns `None` for values
/// with no JSON equivalent (functions, timestamps, custom types, non-finite numbers).
pub fn dynamic_to_json(value: &Dynamic) -> Option<serde_json::Value> {
    use serde_json::Value;
    if value.is_unit() {
        return Some(Value::Null);
    }
    if let Some(b) = value.clone().try_cast::<bool>() {
        return Some(Value::Bool(b));
    }
    if let Some(i) = value.clone().try_cast::<rhai::INT>() {
        return Some(Value::from(i));
    }
    if let Some(f) = value.clone().try_cast::<rhai::FLOAT>() {
        return serde_json::Number::from_f64(f).map(Value::Number);
    }
    if value.is_string() {
        return value.clone().into_string().ok().map(Value::String);
    }
    if let Some(array) = value.clone().try_cast::<rhai::Array>() {
        return array
            .iter()
            .map(dynamic_to_json)
            .collect::<Option<Vec<_>>>()
            .map(Value::Array);
    }
    if let Some(map) = value.clone().try_cast::<rhai::Map>() {
        return map
            .iter()
            .map(|(k, v)| dynamic_to_json(v).map(|v| (k.to_string(), v)))
            .collect::<Option<serde_json::Map<_, _>>>()
            .map(Value::Object);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn script_values_to_json() {
        let engine = rhai::Engine::new_raw();
        let value: Dynamic = engine
            .eval(r#"#{ g: -900, f: 1.5, on: true, name: "lava", list: [1, "a", ()] }"#)
            .unwrap();
        assert_eq!(
            dynamic_to_json(&value),
            Some(
                json!({ "g": -900, "f": 1.5, "on": true, "name": "lava", "list": [1, "a", null] })
            )
        );
        let nan: Dynamic = engine.eval("0.0 / 0.0").unwrap();
        assert_eq!(dynamic_to_json(&nan), None);
    }
}
