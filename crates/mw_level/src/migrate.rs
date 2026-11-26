//! Schema versioning.
//!
//! A level file is first parsed as raw JSON, then upgraded one version at a time by the
//! functions in [`MIGRATIONS`], and only then deserialized into [`Level`]. To introduce
//! schema v(N+1): bump [`CURRENT_SCHEMA_VERSION`], append a `migrate_vN_to_vN+1` function
//! (which must also set `schema_version`), update `docs/LEVEL_FORMAT.md` and re-save the
//! levels in `levels/`.

use serde_json::Value;

use crate::{Level, LevelError, CURRENT_SCHEMA_VERSION};

/// A migration upgrades a raw level from version `N` to `N + 1`.
type Migration = fn(&mut Value) -> Result<(), String>;

/// `MIGRATIONS[i]` upgrades from version `i + 1` to `i + 2`. Empty while only v1 exists.
const MIGRATIONS: &[Migration] = &[];

// Every version below the current one needs a migration.
const _: () = assert!(MIGRATIONS.len() + 1 == CURRENT_SCHEMA_VERSION as usize);

/// Parses level JSON of any supported version into an up-to-date [`Level`].
pub fn level_from_json(json: &str) -> Result<Level, LevelError> {
    let mut raw: Value = serde_json::from_str(json)?;
    migrate(&mut raw)?;
    Ok(serde_json::from_value(raw)?)
}

/// Upgrades a raw level in place to [`CURRENT_SCHEMA_VERSION`].
pub fn migrate(raw: &mut Value) -> Result<(), LevelError> {
    let found = raw
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or(LevelError::MissingSchemaVersion)?;
    let unsupported = LevelError::UnsupportedSchemaVersion {
        found,
        supported: CURRENT_SCHEMA_VERSION,
    };
    let from = match u32::try_from(found) {
        Ok(v) if (1..=CURRENT_SCHEMA_VERSION).contains(&v) => v,
        _ => return Err(unsupported),
    };
    for (version, step) in (from..).zip(&MIGRATIONS[from as usize - 1..]) {
        step(raw).map_err(|reason| LevelError::Migration {
            from: version,
            reason,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_version_is_untouched() {
        let mut raw = json!({ "schema_version": CURRENT_SCHEMA_VERSION, "id": "x" });
        let before = raw.clone();
        migrate(&mut raw).unwrap();
        assert_eq!(raw, before);
    }

    #[test]
    fn missing_version_is_rejected() {
        let err = migrate(&mut json!({ "id": "x" })).unwrap_err();
        assert!(matches!(err, LevelError::MissingSchemaVersion));
        let err = migrate(&mut json!({ "schema_version": "1" })).unwrap_err();
        assert!(matches!(err, LevelError::MissingSchemaVersion));
    }

    #[test]
    fn out_of_range_versions_are_rejected() {
        for v in [0, u64::from(CURRENT_SCHEMA_VERSION) + 1, u64::MAX] {
            let err = migrate(&mut json!({ "schema_version": v })).unwrap_err();
            assert!(
                matches!(err, LevelError::UnsupportedSchemaVersion { found, .. } if found == v),
                "version {v}: {err}"
            );
        }
    }
}
