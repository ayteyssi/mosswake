//! Level file format (JSON), schema versioning and validation.
//!
//! A level lives in `levels/<id>/` as `level.json` plus optional `scripts/*.rhai`.
//! [`LevelBundle::load`] reads it (migrating old schema versions), [`LevelBundle::validate`]
//! lists every content problem, and [`LevelBundle::save`] writes it back in a canonical,
//! diff-friendly form. The format is documented in `docs/LEVEL_FORMAT.md`.
//!
//! This crate must not depend on Bevy: it is shared by the game, the editor and the MCP server.

mod error;
mod io;
pub mod migrate;
mod model;
mod package;
mod validate;

pub use error::LevelError;
pub use io::{LevelBundle, LEVEL_FILE};
pub use model::{EntityDef, Layer, LayerKind, Level, Props, RulesOverrides, Size, Spawn};
pub use validate::{is_valid_script_path, ValidationError};

/// Schema version written by this build. See [`migrate`].
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Default world units per tile.
pub const DEFAULT_TILE_SIZE: u32 = 32;

/// Tile character meaning "no tile". Reserved: it cannot appear in a level's legend.
pub const EMPTY_TILE: char = '.';

/// Entity kinds a level may place. Extended as the simulation gains entities.
pub const KNOWN_ENTITY_KINDS: &[&str] = &["slime_green", "slime_orange", "trigger"];
