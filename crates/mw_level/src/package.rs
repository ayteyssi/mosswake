//! A level bundle as one byte blob, to send over the network or cache by hash.
//!
//! Format: canonical JSON `{"level": <level.json content>, "scripts": {path: source}}`.
//! Canonical (fixed field order, sorted script map, no extra whitespace) so the same level
//! always gives the same bytes, hence the same hash on every server.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::migrate::migrate;
use crate::validate::is_valid_script_path;
use crate::{Level, LevelBundle, LevelError};

#[derive(Serialize)]
struct PackageOut<'a> {
    level: &'a Level,
    scripts: &'a BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageIn {
    level: Value,
    scripts: BTreeMap<String, String>,
}

impl LevelBundle {
    /// Encodes the bundle as canonical JSON bytes.
    pub fn to_package(&self) -> Result<Vec<u8>, LevelError> {
        Ok(serde_json::to_vec(&PackageOut {
            level: &self.level,
            scripts: &self.scripts,
        })?)
    }

    /// Decodes a package (migrating an older level schema). Script paths are checked like
    /// on disk: a package can never name a file outside `scripts/`. Does not validate the
    /// level content (see [`LevelBundle::validate`]).
    pub fn from_package(bytes: &[u8]) -> Result<Self, LevelError> {
        let PackageIn { mut level, scripts } = serde_json::from_slice(bytes)?;
        if let Some(bad) = scripts.keys().find(|p| !is_valid_script_path(p)) {
            return Err(LevelError::UnsafeScriptPath(bad.clone()));
        }
        migrate(&mut level)?;
        let level: Level = serde_json::from_value(level)?;
        Ok(Self { level, scripts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Size, Spawn};

    fn bundle() -> LevelBundle {
        let mut level = Level::new("pkg", "Package", Size { w: 2, h: 1 });
        level.spawns.push(Spawn { x: 0.5, y: 0.0 });
        level.scripts.push("scripts/main.rhai".into());
        LevelBundle {
            level,
            scripts: BTreeMap::from([("scripts/main.rhai".into(), "fn on_tick(dt) {}".into())]),
        }
    }

    #[test]
    fn package_roundtrip_is_identical_and_canonical() {
        let b = bundle();
        let bytes = b.to_package().unwrap();
        let back = LevelBundle::from_package(&bytes).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.to_package().unwrap(), bytes, "same bundle, same bytes");
    }

    #[test]
    fn unsafe_script_paths_are_refused() {
        let mut b = bundle();
        b.scripts.insert("../../.bashrc".into(), "x".into());
        let bytes = b.to_package().unwrap();
        assert!(matches!(
            LevelBundle::from_package(&bytes),
            Err(LevelError::UnsafeScriptPath(p)) if p == "../../.bashrc"
        ));
    }

    #[test]
    fn garbage_and_unknown_versions_are_errors() {
        assert!(LevelBundle::from_package(b"not json").is_err());
        let future = br#"{"level": {"schema_version": 99}, "scripts": {}}"#;
        assert!(matches!(
            LevelBundle::from_package(future),
            Err(LevelError::UnsupportedSchemaVersion { found: 99, .. })
        ));
    }
}
