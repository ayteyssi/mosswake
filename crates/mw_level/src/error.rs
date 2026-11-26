//! Errors raised while loading, migrating or saving a level.
//!
//! Content problems of a well-formed level are not errors but [`crate::ValidationError`]s.

use std::path::PathBuf;

/// Failure to read, parse, migrate or write a level.
#[derive(Debug, thiserror::Error)]
pub enum LevelError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid level JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing or non-integer `schema_version`")]
    MissingSchemaVersion,
    #[error("unsupported schema_version {found} (this build supports 1..={supported})")]
    UnsupportedSchemaVersion { found: u64, supported: u32 },
    #[error("migration from schema_version {from} failed: {reason}")]
    Migration { from: u32, reason: String },
    #[error("refusing to touch script path `{0}` (must be a relative `scripts/*.rhai` path)")]
    UnsafeScriptPath(String),
}

impl LevelError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
