//! Serde types of the level format (schema v1). See `docs/LEVEL_FORMAT.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CURRENT_SCHEMA_VERSION, DEFAULT_TILE_SIZE, EMPTY_TILE};

/// A level as stored in `levels/<id>/level.json`.
///
/// Field order is the serialization order: keep it stable so saved files diff cleanly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// Format version, see [`crate::migrate`].
    pub schema_version: u32,
    /// Identifier, also the directory name under `levels/` (`[a-z0-9_]+`).
    pub id: String,
    /// Human readable name.
    pub name: String,
    /// Grid size, in tiles.
    pub size: Size,
    /// World units per tile.
    #[serde(default = "default_tile_size")]
    pub tile_size: u32,
    /// Maps each tile character used in layers to a tileset piece id.
    /// [`EMPTY_TILE`] (`.`) is implicit and must not appear here.
    #[serde(default)]
    pub legend: BTreeMap<char, String>,
    /// Tile layers, drawn in order (first = furthest back).
    pub layers: Vec<Layer>,
    /// Player spawn points (at least one).
    pub spawns: Vec<Spawn>,
    /// Entities placed in the level (enemies, props…).
    #[serde(default)]
    pub entities: Vec<EntityDef>,
    /// Overrides of the simulation `Rules` (key → JSON value). Keys are checked by the sim.
    #[serde(default)]
    pub rules: RulesOverrides,
    /// Rhai scripts, relative to the level directory (e.g. `scripts/main.rhai`).
    #[serde(default)]
    pub scripts: Vec<String>,
}

/// Overrides of the simulation `Rules`. A `BTreeMap` keeps keys sorted on save.
pub type RulesOverrides = BTreeMap<String, serde_json::Value>;

/// Free-form entity properties. A `BTreeMap` keeps keys sorted on save.
pub type Props = BTreeMap<String, serde_json::Value>;

fn default_tile_size() -> u32 {
    DEFAULT_TILE_SIZE
}

/// Width and height, in tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

/// What a layer's tiles do in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    /// Collides with players and entities.
    Solid,
    /// Visual only.
    Decor,
    /// Hurts on contact.
    Hazard,
}

/// A grid of tiles.
///
/// `tiles` holds one string per row, **top row first**, one character per tile, so the JSON
/// reads like the level itself. Use [`Layer::tile_at`] for y-up access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub name: String,
    pub kind: LayerKind,
    pub tiles: Vec<String>,
}

impl Layer {
    /// Tile character at grid cell `(x, y)`, with `y = 0` the **bottom** row.
    ///
    /// Returns `None` outside the grid and for [`EMPTY_TILE`].
    pub fn tile_at(&self, x: u32, y: u32) -> Option<char> {
        let row_from_top = self.tiles.len().checked_sub(1)?.checked_sub(y as usize)?;
        let ch = self.tiles[row_from_top].chars().nth(x as usize)?;
        (ch != EMPTY_TILE).then_some(ch)
    }
}

/// A player spawn point, in tile units (feet position, y up).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    pub x: f32,
    pub y: f32,
}

/// An entity placed in the level, in tile units (bottom-center of its hitbox, y up).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityDef {
    /// Entity type, one of [`crate::KNOWN_ENTITY_KINDS`].
    pub kind: String,
    pub x: f32,
    pub y: f32,
    /// Kind-specific parameters.
    #[serde(default)]
    pub props: Props,
}

impl Level {
    /// An empty, valid-shaped level of the given size (no spawn yet, so not valid as is).
    pub fn new(id: impl Into<String>, name: impl Into<String>, size: Size) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: id.into(),
            name: name.into(),
            size,
            tile_size: DEFAULT_TILE_SIZE,
            legend: BTreeMap::new(),
            layers: Vec::new(),
            spawns: Vec::new(),
            entities: Vec::new(),
            rules: RulesOverrides::new(),
            scripts: Vec::new(),
        }
    }

    /// Whether grid cell `(x, y)` (y up) holds a tile in any [`LayerKind::Solid`] layer.
    pub fn is_solid(&self, x: u32, y: u32) -> bool {
        self.layers
            .iter()
            .filter(|l| l.kind == LayerKind::Solid)
            .any(|l| l.tile_at(x, y).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(rows: &[&str]) -> Layer {
        Layer {
            name: "ground".into(),
            kind: LayerKind::Solid,
            tiles: rows.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn tile_at_is_y_up() {
        let l = layer(&["#..", "..=", "##."]);
        assert_eq!(l.tile_at(0, 0), Some('#'));
        assert_eq!(l.tile_at(1, 0), Some('#'));
        assert_eq!(l.tile_at(2, 0), None, "'.' is empty");
        assert_eq!(l.tile_at(2, 1), Some('='));
        assert_eq!(l.tile_at(0, 2), Some('#'));
    }

    #[test]
    fn tile_at_out_of_bounds_is_none() {
        let l = layer(&["#.", "##"]);
        assert_eq!(l.tile_at(2, 0), None);
        assert_eq!(l.tile_at(0, 2), None);
        assert_eq!(layer(&[]).tile_at(0, 0), None);
    }

    #[test]
    fn is_solid_ignores_non_solid_layers() {
        let mut level = Level::new("t", "T", Size { w: 2, h: 1 });
        level.layers.push(Layer {
            name: "deco".into(),
            kind: LayerKind::Decor,
            tiles: vec!["#.".into()],
        });
        assert!(!level.is_solid(0, 0));
        level.layers.push(layer(&[".#"]));
        assert!(level.is_solid(1, 0));
        assert!(!level.is_solid(0, 0));
    }
}
