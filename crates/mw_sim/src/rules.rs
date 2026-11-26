//! Gameplay parameters ([`Rules`]) and the registry of activable mechanics.
//!
//! Every gameplay number lives in [`Rules`]: code defaults, overridden by the level's `rules`
//! map, then changeable at runtime by scripts. Adding a parameter is just adding a field with
//! a default: it becomes overridable from `level.json` with no extra code.

use std::collections::BTreeSet;

use bevy::prelude::*;
use mw_level::RulesOverrides;
use serde::{Deserialize, Serialize};

/// All gameplay parameters. Distances in world units (1 tile = `tile_size`, 32 by default),
/// speeds in units/s, accelerations in units/s², times in seconds.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rules {
    /// Vertical acceleration. Negative pulls down; positive inverts gravity.
    pub gravity: f32,
    /// Maximum speed along the gravity direction (> 0).
    pub max_fall_speed: f32,
    /// Ids of the active mechanics (see [`KnownMechanics`]).
    pub mechanics: Vec<String>,

    // --- Player body ---
    /// Player hitbox width.
    pub player_width: f32,
    /// Player hitbox height.
    pub player_height: f32,

    // --- `movement` mechanic ---
    /// Top horizontal speed.
    pub run_speed: f32,
    /// Acceleration towards the input direction while grounded.
    pub ground_accel: f32,
    /// Deceleration to a stop (or when turning) while grounded.
    pub ground_decel: f32,
    /// Acceleration towards the input direction in the air.
    pub air_accel: f32,
    /// Deceleration without input in the air (low = momentum is kept).
    pub air_decel: f32,

    // --- `jump` mechanic ---
    /// Height of a full jump (button held until the apex).
    pub jump_height: f32,
    /// Factor applied to the rising speed when the button is released early (0..=1):
    /// lower = shorter taps.
    pub jump_cut: f32,
    /// Grace period after leaving a ledge during which a jump is still allowed.
    pub coyote_time: f32,
    /// A jump pressed this long before landing still triggers on landing.
    pub jump_buffer: f32,

    /// Horizontal dash speed, units/s.
    pub dash_speed: f32,
    /// Dash duration, rounded to ticks (at least one tick).
    pub dash_duration: f32,
    /// Minimum time between dash starts, seconds.
    pub dash_cooldown: f32,
    /// Additional jumps allowed before landing.
    pub air_jumps: u32,
    /// Maximum falling speed while pushing against a wall.
    pub wall_slide_speed: f32,
    /// Wall-jump impulse away from the wall.
    pub wall_jump_speed_x: f32,
    /// Wall-jump impulse against gravity.
    pub wall_jump_speed_y: f32,
    /// Time before horizontal movement can override a wall-jump impulse.
    pub wall_jump_lock: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            gravity: -1800.0,
            max_fall_speed: 900.0,
            mechanics: vec!["movement".into(), "jump".into()],
            player_width: 24.0,
            player_height: 56.0,
            run_speed: 220.0,
            ground_accel: 2400.0,
            ground_decel: 3200.0,
            air_accel: 1500.0,
            air_decel: 600.0,
            jump_height: 128.0,
            jump_cut: 0.4,
            coyote_time: 0.1,
            jump_buffer: 0.1,
            dash_speed: 650.0,
            dash_duration: 0.15,
            dash_cooldown: 0.3,
            air_jumps: 1,
            wall_slide_speed: 100.0,
            wall_jump_speed_x: 280.0,
            wall_jump_speed_y: 600.0,
            wall_jump_lock: 0.1,
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
        let mut check = |ok: bool, what: &str| {
            if !ok {
                errors.push(RulesError::InvalidValue(what.into()));
            }
        };
        let positive = |v: f32| v.is_finite() && v > 0.0;
        let non_negative = |v: f32| v.is_finite() && v >= 0.0;
        check(self.gravity.is_finite(), "gravity must be finite");
        check(positive(self.max_fall_speed), "max_fall_speed must be > 0");
        check(positive(self.player_width), "player_width must be > 0");
        check(positive(self.player_height), "player_height must be > 0");
        check(non_negative(self.run_speed), "run_speed must be >= 0");
        for (value, name) in [
            (self.ground_accel, "ground_accel"),
            (self.ground_decel, "ground_decel"),
            (self.air_accel, "air_accel"),
            (self.air_decel, "air_decel"),
        ] {
            check(positive(value), &format!("{name} must be > 0"));
        }
        check(non_negative(self.jump_height), "jump_height must be >= 0");
        check(
            (0.0..=1.0).contains(&self.jump_cut),
            "jump_cut must be between 0 and 1",
        );
        check(non_negative(self.coyote_time), "coyote_time must be >= 0");
        check(non_negative(self.jump_buffer), "jump_buffer must be >= 0");
        for (value, name) in [
            (self.dash_speed, "dash_speed"),
            (self.dash_duration, "dash_duration"),
            (self.wall_jump_speed_x, "wall_jump_speed_x"),
            (self.wall_jump_speed_y, "wall_jump_speed_y"),
        ] {
            check(positive(value), &format!("{name} must be > 0"));
        }
        for (value, name) in [
            (self.dash_cooldown, "dash_cooldown"),
            (self.wall_slide_speed, "wall_slide_speed"),
            (self.wall_jump_lock, "wall_jump_lock"),
        ] {
            check(non_negative(value), &format!("{name} must be >= 0"));
        }
        for id in &self.mechanics {
            if !known.contains(id) {
                errors.push(RulesError::UnknownMechanic(id.clone()));
            }
        }
        errors
    }

    /// Initial speed of a jump reaching [`Rules::jump_height`] under the current gravity, so
    /// the height holds whatever gravity a script sets (0 when there is no gravity).
    ///
    /// Continuous physics gives `sqrt(2 |g| h)`, but the simulation integrates in fixed steps
    /// (gravity applied before each move), which loses about `v·dt/2` of height; the
    /// half-step term `|g|·dt/2` compensates it.
    pub fn jump_speed(&self) -> f32 {
        let g = self.gravity.abs();
        if g == 0.0 {
            return 0.0;
        }
        (2.0 * g * self.jump_height).sqrt() + g * crate::TICK_DT / 2.0
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
        known.0.insert("movement");
        let rules = Rules {
            gravity: f32::NAN,
            max_fall_speed: 0.0,
            jump_cut: 1.5,
            mechanics: vec!["jump".into(), "fly".into()],
            ..Rules::default()
        };
        let errors = rules.validate(&known);
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert_eq!(errors[3], RulesError::UnknownMechanic("fly".into()));
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
    fn jump_speed_reaches_jump_height_in_fixed_steps() {
        for gravity in [-1800.0, -600.0, 2500.0] {
            let rules = Rules {
                gravity,
                ..Rules::default()
            };
            // Integrate exactly like the simulation: velocity first, then position.
            let (g, dt) = (gravity.abs(), crate::TICK_DT);
            let (mut v, mut y, mut apex) = (rules.jump_speed(), 0.0_f32, 0.0_f32);
            while v > 0.0 {
                v -= g * dt;
                y += v.max(0.0) * dt;
                apex = apex.max(y);
            }
            assert!(
                (apex - rules.jump_height).abs() < 1.0,
                "g={gravity}: apex {apex}"
            );
        }
        let floating = Rules {
            gravity: 0.0,
            ..Rules::default()
        };
        assert_eq!(floating.jump_speed(), 0.0);
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
