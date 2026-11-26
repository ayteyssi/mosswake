//! Loading and saving `levels/<id>/` directories.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::migrate::level_from_json;
use crate::validate::is_valid_script_path;
use crate::{Level, LevelError};

/// File name of the level description inside a level directory.
pub const LEVEL_FILE: &str = "level.json";

/// A level together with the sources of its scripts, as one unit that can be validated,
/// sent over the network or edited without touching the disk.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelBundle {
    pub level: Level,
    /// Script sources keyed by their path in [`Level::scripts`] (e.g. `scripts/main.rhai`).
    pub scripts: BTreeMap<String, String>,
}

impl Level {
    /// Serializes to the canonical on-disk form: pretty JSON, stable key order, final newline.
    pub fn to_json(&self) -> Result<String, LevelError> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }

    /// Parses level JSON, migrating older schema versions. Does not validate.
    pub fn from_json(json: &str) -> Result<Self, LevelError> {
        level_from_json(json)
    }
}

impl LevelBundle {
    /// Loads `dir/level.json` and every script it lists. Missing scripts are not an error
    /// here; they show up in [`LevelBundle::validate`]. Unsafe script paths are refused.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, LevelError> {
        let dir = dir.as_ref();
        let level_path = dir.join(LEVEL_FILE);
        let json = fs::read_to_string(&level_path).map_err(|e| LevelError::io(&level_path, e))?;
        let level = Level::from_json(&json)?;

        let mut scripts = BTreeMap::new();
        for rel in &level.scripts {
            if !is_valid_script_path(rel) {
                return Err(LevelError::UnsafeScriptPath(rel.clone()));
            }
            let path = dir.join(rel);
            match fs::read_to_string(&path) {
                Ok(source) => {
                    scripts.insert(rel.clone(), source);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(LevelError::io(path, e)),
            }
        }
        Ok(Self { level, scripts })
    }

    /// Writes `dir/level.json` and every script source, creating directories as needed.
    /// Files already in `dir` but no longer referenced are left untouched.
    pub fn save(&self, dir: impl AsRef<Path>) -> Result<(), LevelError> {
        let dir = dir.as_ref();
        // Check every path before writing anything, so a bad bundle leaves no partial save.
        if let Some(bad) = self.scripts.keys().find(|p| !is_valid_script_path(p)) {
            return Err(LevelError::UnsafeScriptPath(bad.clone()));
        }
        let json = self.level.to_json()?;
        fs::create_dir_all(dir).map_err(|e| LevelError::io(dir, e))?;
        let level_path = dir.join(LEVEL_FILE);
        fs::write(&level_path, json).map_err(|e| LevelError::io(&level_path, e))?;
        for (rel, source) in &self.scripts {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| LevelError::io(parent, e))?;
            }
            fs::write(&path, source).map_err(|e| LevelError::io(&path, e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityDef, Layer, LayerKind, Size, Spawn};

    fn sample_bundle() -> LevelBundle {
        let mut level = Level::new("io_test", "I/O test", Size { w: 3, h: 2 });
        level.legend.insert('#', "mossy_ground".into());
        level.layers.push(Layer {
            name: "ground".into(),
            kind: LayerKind::Solid,
            tiles: vec!["...".into(), "###".into()],
        });
        level.spawns.push(Spawn { x: 1.5, y: 1.0 });
        let mut props = crate::Props::new();
        props.insert("patrol".into(), serde_json::json!(2.5));
        level.entities.push(EntityDef {
            kind: "slime_orange".into(),
            x: 2.5,
            y: 1.0,
            props,
        });
        level
            .rules
            .insert("mechanics".into(), serde_json::json!(["jump", "dash"]));
        level
            .rules
            .insert("gravity".into(), serde_json::json!(-1800));
        level.scripts.push("scripts/main.rhai".into());
        let scripts = BTreeMap::from([(
            "scripts/main.rhai".to_string(),
            "fn on_level_start() {}\n".to_string(),
        )]);
        LevelBundle { level, scripts }
    }

    #[test]
    fn save_then_load_is_identical() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = sample_bundle();
        bundle.save(dir.path()).unwrap();
        let loaded = LevelBundle::load(dir.path()).unwrap();
        assert_eq!(loaded, bundle);
        assert_eq!(loaded.validate(), vec![]);
    }

    #[test]
    fn json_is_canonical() {
        let json = sample_bundle().level.to_json().unwrap();
        let reparsed = Level::from_json(&json).unwrap();
        assert_eq!(reparsed.to_json().unwrap(), json, "save is byte-stable");
        assert!(json.ends_with("}\n"));
        // Rows stay one per line so the grid is readable in the file and in diffs.
        assert!(
            json.contains("\n        \"...\",\n        \"###\"\n"),
            "{json}"
        );
        // Rule keys are sorted regardless of insertion order.
        assert!(json.find("\"gravity\"").unwrap() < json.find("\"mechanics\"").unwrap());
    }

    #[test]
    fn optional_fields_default() {
        let level = Level::from_json(
            r#"{ "schema_version": 1, "id": "min", "name": "Min",
                 "size": { "w": 1, "h": 1 }, "layers": [], "spawns": [{ "x": 0.5, "y": 0 }] }"#,
        )
        .unwrap();
        assert_eq!(level.tile_size, crate::DEFAULT_TILE_SIZE);
        assert!(level.legend.is_empty() && level.entities.is_empty());
        assert!(level.rules.is_empty() && level.scripts.is_empty());
        assert_eq!(level.validate(), vec![]);
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = Level::from_json(
            r#"{ "schema_version": 1, "id": "x", "name": "X", "size": { "w": 1, "h": 1 },
                 "layers": [], "spawn": [] }"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field `spawn`"), "{err}");
    }

    #[test]
    fn missing_script_file_is_a_validation_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut bundle = sample_bundle();
        bundle.scripts.clear();
        bundle.save(dir.path()).unwrap();
        let loaded = LevelBundle::load(dir.path()).unwrap();
        assert_eq!(
            loaded.validate(),
            vec![crate::ValidationError::MissingScript {
                path: "scripts/main.rhai".into()
            }]
        );
    }

    #[test]
    fn unsafe_script_paths_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut bundle = sample_bundle();
        bundle
            .scripts
            .insert("../escape.rhai".into(), String::new());
        assert!(matches!(
            bundle.save(dir.path()),
            Err(LevelError::UnsafeScriptPath(p)) if p == "../escape.rhai"
        ));
        assert!(!dir.path().join(LEVEL_FILE).exists(), "nothing written");

        let mut level = sample_bundle().level;
        level.scripts = vec!["../escape.rhai".into()];
        fs::write(dir.path().join(LEVEL_FILE), level.to_json().unwrap()).unwrap();
        assert!(matches!(
            LevelBundle::load(dir.path()),
            Err(LevelError::UnsafeScriptPath(_))
        ));
    }

    #[test]
    fn missing_level_file_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            LevelBundle::load(dir.path()),
            Err(LevelError::Io { .. })
        ));
    }
}
