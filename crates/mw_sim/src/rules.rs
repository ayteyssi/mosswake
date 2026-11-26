//! Gameplay parameters ([`Rules`]) and the registry of activable mechanics.
//!
//! Every gameplay number lives in [`Rules`]: code defaults, overridden by the level's `rules`
//! map, then changeable at runtime by scripts. Adding a parameter is just adding a field with
//! a default: it becomes overridable from `level.json` with no extra code.

use std::collections::BTreeSet;

use bevy::prelude::*;
use mw_level::RulesOverrides;
use serde::{Deserialize, Serialize};

/// All gameplay parameters. Distances in world units (1 tile = `tile_size`), times in seconds.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    /// Vertical acceleration, world units/s². Negative pulls down; positive inverts gravity.
    pub gravity: f32,
    /// Maximum speed along the gravity direction, world units/s (> 0).
    pub max_fall_speed: f32,
    /// Ids of the active mechanics (see [`KnownMechanics`]), e.g. `["jump", "dash"]`.
    pub mechanics: Vec<String>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            gravity: -1800.0,
            max_fall_speed: 900.0,
            mechanics: Vec::new(),
        }
    }
}

/// Why rule overrides or values were rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RulesError {
    #[error("unknown rule `{0}`")]
    UnknownRule(String),
    #[error("invalid rule value: {0}")]
    InvalidValue(String),
    #[error("unknown mechanic `{0}`")]
    UnknownMechanic(String),
}

impl Rules {
    /// Returns these rules with `overrides` applied, or every problem found.
    ///
    /// Works generically: the rules are turned into a JSON object, overridden key by key,
    /// and parsed back, so types are checked by serde and new fields need no code here.
    pub fn with_overrides(&self, overrides: &RulesOverrides) -> Result<Self, Vec<RulesError>> {
        let serde_json::Value::Object(mut fields) = serde_json::to_value(self)
            .map_err(|e| vec![RulesError::InvalidValue(e.to_string())])?
        else {
            unreachable!("Rules serializes to a JSON object");
        };
        let unknown: Vec<RulesError> = overrides
            .keys()
            .filter(|key| !fields.contains_key(*key))
            .map(|key| RulesError::UnknownRule(key.clone()))
            .collect();
        if !unknown.is_empty() {
            return Err(unknown);
        }
        for (key, value) in overrides {
            fields.insert(key.clone(), value.clone());
        }
        serde_json::from_value(serde_json::Value::Object(fields))
            .map_err(|e| vec![RulesError::InvalidValue(e.to_string())])
    }

    /// Checks values that serde cannot: ranges and mechanic ids.
    pub fn validate(&self, known: &KnownMechanics) -> Vec<RulesError> {
        let mut errors = Vec::new();
        if !self.gravity.is_finite() {
            errors.push(RulesError::InvalidValue("gravity must be finite".into()));
        }
        if !(self.max_fall_speed.is_finite() && self.max_fall_speed > 0.0) {
            errors.push(RulesError::InvalidValue(
                "max_fall_speed must be a positive number".into(),
            ));
        }
        for id in &self.mechanics {
            if !known.contains(id) {
                errors.push(RulesError::UnknownMechanic(id.clone()));
            }
        }
        errors
    }

    /// Whether mechanic `id` is active.
    pub fn is_active(&self, id: &str) -> bool {
        self.mechanics.iter().any(|m| m == id)
    }

    /// Unit vector of gravity on the y axis: `-1.0` (down, also when gravity is 0) or `1.0`.
    pub fn gravity_dir(&self) -> f32 {
        if self.gravity > 0.0 {
            1.0
        } else {
            -1.0
        }
    }
}

/// Ids of every mechanic the simulation implements. Filled by mechanic plugins through
/// [`RegisterMechanic::register_mechanic`]; levels may only activate registered ids.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownMechanics(BTreeSet<&'static str>);

impl KnownMechanics {
    pub fn contains(&self, id: &str) -> bool {
        self.0.contains(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().copied()
    }
}

/// Registration of mechanics on an [`App`].
pub trait RegisterMechanic {
    /// Declares mechanic `id`. Its systems should run under [`mechanic_active`]`(id)`.
    fn register_mechanic(&mut self, id: &'static str) -> &mut Self;
}

impl RegisterMechanic for App {
    fn register_mechanic(&mut self, id: &'static str) -> &mut Self {
        self.init_resource::<KnownMechanics>();
        self.world_mut()
            .resource_mut::<KnownMechanics>()
            .0
            .insert(id);
        self
    }
}

/// Run condition: the system runs only while mechanic `id` is active in [`Rules`].
pub fn mechanic_active(id: &'static str) -> impl Fn(Res<Rules>) -> bool + Clone {
    move |rules: Res<Rules>| rules.is_active(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn overrides(value: serde_json::Value) -> RulesOverrides {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn overrides_replace_only_given_fields() {
        let rules = Rules::default()
            .with_overrides(&overrides(
                json!({ "gravity": -900, "mechanics": ["jump"] }),
            ))
            .unwrap();
        assert_eq!(rules.gravity, -900.0);
        assert_eq!(rules.mechanics, ["jump"]);
        assert_eq!(rules.max_fall_speed, Rules::default().max_fall_speed);
    }

    #[test]
    fn unknown_rules_are_all_reported() {
        let errors = Rules::default()
            .with_overrides(&overrides(
                json!({ "gravity": 1, "jump_heigth": 3, "zz": 0 }),
            ))
            .unwrap_err();
        assert_eq!(
            errors,
            [
                RulesError::UnknownRule("jump_heigth".into()),
                RulesError::UnknownRule("zz".into()),
            ]
        );
    }

    #[test]
    fn wrong_types_are_rejected() {
        let errors = Rules::default()
            .with_overrides(&overrides(json!({ "gravity": "strong" })))
            .unwrap_err();
        assert!(matches!(&errors[..], [RulesError::InvalidValue(m)] if m.contains("invalid type")));
    }

    #[test]
    fn validate_checks_ranges_and_mechanics() {
        let mut known = KnownMechanics::default();
        known.0.insert("jump");
        let rules = Rules {
            gravity: f32::NAN,
            max_fall_speed: 0.0,
            mechanics: vec!["jump".into(), "fly".into()],
        };
        let errors = rules.validate(&known);
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert_eq!(errors[2], RulesError::UnknownMechanic("fly".into()));
        assert!(Rules::default().validate(&known).is_empty());
    }

    #[test]
    fn register_mechanic_and_run_condition() {
        let mut app = App::new();
        app.register_mechanic("dash").register_mechanic("jump");
        let known = app.world().resource::<KnownMechanics>();
        assert_eq!(known.iter().collect::<Vec<_>>(), ["dash", "jump"]);

        #[derive(Resource, Default)]
        struct Ran(u32);
        app.init_resource::<Ran>()
            .insert_resource(Rules::default())
            .add_systems(
                Update,
                (|mut ran: ResMut<Ran>| ran.0 += 1).run_if(mechanic_active("dash")),
            );
        app.update();
        assert_eq!(app.world().resource::<Ran>().0, 0, "inactive");
        app.world_mut().resource_mut::<Rules>().mechanics = vec!["dash".into()];
        app.update();
        assert_eq!(app.world().resource::<Ran>().0, 1, "active");
    }

    #[test]
    fn gravity_direction() {
        assert_eq!(Rules::default().gravity_dir(), -1.0);
        let inverted = Rules {
            gravity: 500.0,
            ..Rules::default()
        };
        assert_eq!(inverted.gravity_dir(), 1.0);
    }
}
