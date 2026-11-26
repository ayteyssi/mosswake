//! Content validation shared by the game, the editor and the MCP server.
//!
//! Validation never stops at the first problem: it returns every [`ValidationError`] found,
//! in a deterministic order, so a tool (or an agent) can fix them all in one pass.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{Level, LevelBundle, EMPTY_TILE, KNOWN_ENTITY_KINDS};

/// A problem in a level's content. Serializes as `{"code": "<variant>", ...fields}`.
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ValidationError {
    #[error("id `{id}` must be non-empty and only contain [a-z0-9_]")]
    InvalidId { id: String },
    #[error("size {w}x{h} must be at least 1x1 tile")]
    InvalidSize { w: u32, h: u32 },
    #[error("tile_size must be greater than 0")]
    InvalidTileSize,
    #[error("legend character {ch:?} must be printable ASCII and not '{EMPTY_TILE}'")]
    InvalidLegendChar { ch: char },
    #[error("legend character {ch:?} maps to an empty tile id")]
    EmptyTileId { ch: char },
    #[error("legend character {ch:?} maps to unknown tile id `{id}`")]
    UnknownTileId { ch: char, id: String },
    #[error("layer name must not be empty (layer #{index})")]
    EmptyLayerName { index: usize },
    #[error("layer name `{name}` is used more than once")]
    DuplicateLayerName { name: String },
    #[error("layer `{layer}` has {found} rows, expected {expected} (size.h)")]
    WrongRowCount {
        layer: String,
        expected: u32,
        found: usize,
    },
    #[error("layer `{layer}` row {row} has {found} tiles, expected {expected} (size.w)")]
    WrongRowWidth {
        layer: String,
        row: usize,
        expected: u32,
        found: usize,
    },
    #[error(
        "layer `{layer}` uses {ch:?} (first at row {row}, col {col}) which is not in the legend"
    )]
    UnknownTile {
        layer: String,
        ch: char,
        row: usize,
        col: usize,
    },
    #[error("the level has no player spawn")]
    NoSpawn,
    #[error("spawn #{index} at ({x}, {y}) is outside the level")]
    SpawnOutOfBounds { index: usize, x: f32, y: f32 },
    #[error("spawn #{index} at ({x}, {y}) is inside a solid tile")]
    SpawnInSolid { index: usize, x: f32, y: f32 },
    #[error("entity #{index} (`{kind}`) at ({x}, {y}) is outside the level")]
    EntityOutOfBounds {
        index: usize,
        kind: String,
        x: f32,
        y: f32,
    },
    #[error("entity #{index} has unknown kind `{kind}`")]
    UnknownEntityKind { index: usize, kind: String },
    #[error("script path `{path}` must be a relative `scripts/*.rhai` path")]
    InvalidScriptPath { path: String },
    #[error("script `{path}` is listed twice")]
    DuplicateScript { path: String },
    #[error("script `{path}` is listed but its source is missing")]
    MissingScript { path: String },
}

/// Whether `path` is a safe script location: `scripts/<name>.rhai`, relative, with no `..`
/// and only `/` separators. Guards against path traversal when loading or saving.
pub fn is_valid_script_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("scripts/") else {
        return false;
    };
    rest.ends_with(".rhai")
        && !path.contains('\\')
        && rest
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Level {
    /// Checks the level's content. An empty result means the level is valid.
    ///
    /// Script sources are not checked here (see [`LevelBundle::validate`]).
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        self.validate_header(&mut errors);
        self.validate_legend(&mut errors);
        self.validate_layers(&mut errors);
        self.validate_placements(&mut errors);
        self.validate_script_paths(&mut errors);
        errors
    }

    /// Structural validation plus a check of every legend id against the caller's palette.
    /// Include both named pieces and autotiled terrain families in `known_tile_ids`.
    /// This crate does not load or depend on an atlas format.
    pub fn validate_with_tile_ids(
        &self,
        known_tile_ids: &BTreeSet<String>,
    ) -> Vec<ValidationError> {
        let mut errors = self.validate();
        self.validate_tile_ids(known_tile_ids, &mut errors);
        errors
    }

    fn validate_tile_ids(
        &self,
        known_tile_ids: &BTreeSet<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        for (&ch, id) in &self.legend {
            // Empty ids already have a structural diagnostic.
            if !id.trim().is_empty() && !known_tile_ids.contains(id) {
                errors.push(ValidationError::UnknownTileId { ch, id: id.clone() });
            }
        }
    }

    fn validate_header(&self, errors: &mut Vec<ValidationError>) {
        if !is_valid_id(&self.id) {
            errors.push(ValidationError::InvalidId {
                id: self.id.clone(),
            });
        }
        let crate::Size { w, h } = self.size;
        if w == 0 || h == 0 {
            errors.push(ValidationError::InvalidSize { w, h });
        }
        if self.tile_size == 0 {
            errors.push(ValidationError::InvalidTileSize);
        }
    }

    fn validate_legend(&self, errors: &mut Vec<ValidationError>) {
        for (&ch, id) in &self.legend {
            if !ch.is_ascii_graphic() || ch == EMPTY_TILE {
                errors.push(ValidationError::InvalidLegendChar { ch });
            }
            if id.trim().is_empty() {
                errors.push(ValidationError::EmptyTileId { ch });
            }
        }
    }

    fn validate_layers(&self, errors: &mut Vec<ValidationError>) {
        let mut seen_names = BTreeSet::new();
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.name.is_empty() {
                errors.push(ValidationError::EmptyLayerName { index });
            } else if !seen_names.insert(layer.name.as_str()) {
                errors.push(ValidationError::DuplicateLayerName {
                    name: layer.name.clone(),
                });
            }
            if layer.tiles.len() != self.size.h as usize {
                errors.push(ValidationError::WrongRowCount {
                    layer: layer.name.clone(),
                    expected: self.size.h,
                    found: layer.tiles.len(),
                });
            }
            // Report each unknown character once, at its first occurrence.
            let mut unknown: BTreeMap<char, (usize, usize)> = BTreeMap::new();
            for (row, line) in layer.tiles.iter().enumerate() {
                let width = line.chars().count();
                if width != self.size.w as usize {
                    errors.push(ValidationError::WrongRowWidth {
                        layer: layer.name.clone(),
                        row,
                        expected: self.size.w,
                        found: width,
                    });
                }
                for (col, ch) in line.chars().enumerate() {
                    if ch != EMPTY_TILE && !self.legend.contains_key(&ch) {
                        unknown.entry(ch).or_insert((row, col));
                    }
                }
            }
            errors.extend(unknown.into_iter().map(|(ch, (row, col))| {
                ValidationError::UnknownTile {
                    layer: layer.name.clone(),
                    ch,
                    row,
                    col,
                }
            }));
        }
    }

    fn validate_placements(&self, errors: &mut Vec<ValidationError>) {
        if self.spawns.is_empty() {
            errors.push(ValidationError::NoSpawn);
        }
        for (index, s) in self.spawns.iter().enumerate() {
            let (x, y) = (s.x, s.y);
            if !self.contains(x, y) {
                errors.push(ValidationError::SpawnOutOfBounds { index, x, y });
            } else if self.is_solid(x.floor() as u32, y.floor() as u32) {
                errors.push(ValidationError::SpawnInSolid { index, x, y });
            }
        }
        for (index, e) in self.entities.iter().enumerate() {
            if !KNOWN_ENTITY_KINDS.contains(&e.kind.as_str()) {
                errors.push(ValidationError::UnknownEntityKind {
                    index,
                    kind: e.kind.clone(),
                });
            }
            if !self.contains(e.x, e.y) {
                errors.push(ValidationError::EntityOutOfBounds {
                    index,
                    kind: e.kind.clone(),
                    x: e.x,
                    y: e.y,
                });
            }
        }
    }

    fn validate_script_paths(&self, errors: &mut Vec<ValidationError>) {
        let mut seen = BTreeSet::new();
        for path in &self.scripts {
            if !is_valid_script_path(path) {
                errors.push(ValidationError::InvalidScriptPath { path: path.clone() });
            } else if !seen.insert(path.as_str()) {
                errors.push(ValidationError::DuplicateScript { path: path.clone() });
            }
        }
    }

    /// Whether the point `(x, y)` (tile units) lies within the level, edges included.
    fn contains(&self, x: f32, y: f32) -> bool {
        (0.0..=self.size.w as f32).contains(&x) && (0.0..=self.size.h as f32).contains(&y)
    }
}

impl LevelBundle {
    /// Bundle validation plus palette validation, including unused legend entries.
    pub fn validate_with_tile_ids(
        &self,
        known_tile_ids: &BTreeSet<String>,
    ) -> Vec<ValidationError> {
        let mut errors = self.validate();
        self.level.validate_tile_ids(known_tile_ids, &mut errors);
        errors
    }

    /// [`Level::validate`] plus a check that every listed script has a source.
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = self.level.validate();
        let mut reported = BTreeSet::new();
        for path in &self.level.scripts {
            if is_valid_script_path(path)
                && !self.scripts.contains_key(path)
                && reported.insert(path.as_str())
            {
                errors.push(ValidationError::MissingScript { path: path.clone() });
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityDef, Layer, LayerKind, Size, Spawn};

    /// A small valid level:
    /// ```text
    /// ....
    /// ...=
    /// ##.#
    /// ```
    fn valid_level() -> Level {
        let mut level = Level::new("test_01", "Test", Size { w: 4, h: 3 });
        level.legend.insert('#', "mossy_ground".into());
        level.legend.insert('=', "mossy_platform".into());
        level.layers.push(Layer {
            name: "ground".into(),
            kind: LayerKind::Solid,
            tiles: vec!["....".into(), "...=".into(), "##.#".into()],
        });
        level.spawns.push(Spawn { x: 0.5, y: 1.0 });
        level.entities.push(EntityDef {
            kind: "slime_green".into(),
            x: 3.5,
            y: 2.0,
            props: Default::default(),
        });
        level
    }

    fn errors_of(f: impl FnOnce(&mut Level)) -> Vec<ValidationError> {
        let mut level = valid_level();
        f(&mut level);
        level.validate()
    }

    #[test]
    fn valid_level_has_no_errors() {
        assert_eq!(valid_level().validate(), vec![]);
    }

    #[test]
    fn header_errors() {
        let errors = errors_of(|l| {
            l.id = "Bad-Id".into();
            l.tile_size = 0;
        });
        assert_eq!(
            errors,
            vec![
                ValidationError::InvalidId {
                    id: "Bad-Id".into()
                },
                ValidationError::InvalidTileSize,
            ]
        );
        assert!(
            errors_of(|l| l.id.clear()).contains(&ValidationError::InvalidId { id: String::new() })
        );
        assert!(errors_of(|l| l.size.w = 0).contains(&ValidationError::InvalidSize { w: 0, h: 3 }));
    }

    #[test]
    fn legend_errors() {
        let errors = errors_of(|l| {
            l.legend.insert('.', "nope".into());
            l.legend.insert(' ', "space".into());
            l.legend.insert('x', " ".into());
        });
        assert_eq!(
            errors,
            vec![
                ValidationError::InvalidLegendChar { ch: ' ' },
                ValidationError::InvalidLegendChar { ch: '.' },
                ValidationError::EmptyTileId { ch: 'x' },
            ]
        );
    }

    #[test]
    fn palette_validation_checks_all_legend_entries_in_character_order() {
        let mut level = valid_level();
        let palette = BTreeSet::from(["mossy_ground".into(), "platform_1x1_a".into()]);
        level.legend.insert('=', "platform_1x1_a".into());
        assert!(level.validate_with_tile_ids(&palette).is_empty());
        level.legend.insert('z', "missing_unused".into());
        level.legend.insert('=', "mossy_platform".into());
        level.legend.insert('a', " ".into());
        assert_eq!(
            level.validate_with_tile_ids(&palette),
            vec![
                ValidationError::EmptyTileId { ch: 'a' },
                ValidationError::UnknownTileId {
                    ch: '=',
                    id: "mossy_platform".into()
                },
                ValidationError::UnknownTileId {
                    ch: 'z',
                    id: "missing_unused".into()
                },
            ]
        );
        assert_eq!(
            serde_json::to_value(ValidationError::UnknownTileId {
                ch: '^',
                id: "missing".into()
            })
            .unwrap(),
            serde_json::json!({"code": "unknown_tile_id", "ch": "^", "id": "missing"})
        );
    }

    #[test]
    fn bundle_palette_validation_preserves_missing_script_diagnostics() {
        let mut level = valid_level();
        level.scripts.push("scripts/main.rhai".into());
        let bundle = LevelBundle {
            level,
            scripts: BTreeMap::new(),
        };
        let errors = bundle.validate_with_tile_ids(&BTreeSet::new());
        assert!(matches!(&errors[0], ValidationError::MissingScript { .. }));
        assert_eq!(errors.len(), 3);
        assert!(matches!(
            &errors[1],
            ValidationError::UnknownTileId { ch: '#', .. }
        ));
        assert!(matches!(
            &errors[2],
            ValidationError::UnknownTileId { ch: '=', .. }
        ));
    }

    #[test]
    fn layer_shape_errors() {
        let errors = errors_of(|l| {
            l.layers[0].tiles = vec!["....".into(), "..".into()];
        });
        assert_eq!(
            errors,
            vec![
                ValidationError::WrongRowCount {
                    layer: "ground".into(),
                    expected: 3,
                    found: 2,
                },
                ValidationError::WrongRowWidth {
                    layer: "ground".into(),
                    row: 1,
                    expected: 4,
                    found: 2,
                },
                // The spawn at (0.5, 1) is now in the air: no SpawnInSolid either way.
            ]
        );
    }

    #[test]
    fn layer_name_errors() {
        let errors = errors_of(|l| {
            let mut copy = l.layers[0].clone();
            copy.kind = LayerKind::Decor;
            l.layers.push(copy.clone());
            copy.name.clear();
            l.layers.push(copy);
        });
        assert_eq!(
            errors,
            vec![
                ValidationError::DuplicateLayerName {
                    name: "ground".into()
                },
                ValidationError::EmptyLayerName { index: 2 },
            ]
        );
    }

    #[test]
    fn unknown_tile_reported_once_at_first_occurrence() {
        let errors = errors_of(|l| {
            l.layers[0].tiles = vec!["..?.".into(), "...?".into(), "##.#".into()];
        });
        assert_eq!(
            errors,
            vec![ValidationError::UnknownTile {
                layer: "ground".into(),
                ch: '?',
                row: 0,
                col: 2,
            }]
        );
    }

    #[test]
    fn spawn_errors() {
        assert_eq!(
            errors_of(|l| l.spawns.clear()),
            vec![ValidationError::NoSpawn]
        );
        let errors = errors_of(|l| {
            l.spawns = vec![
                Spawn { x: -0.1, y: 1.0 },
                Spawn { x: 0.5, y: 0.5 },
                Spawn { x: 4.0, y: 3.0 },
                Spawn {
                    x: f32::NAN,
                    y: 1.0,
                },
            ];
        });
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert_eq!(
            errors[0],
            ValidationError::SpawnOutOfBounds {
                index: 0,
                x: -0.1,
                y: 1.0
            }
        );
        assert_eq!(
            errors[1],
            ValidationError::SpawnInSolid {
                index: 1,
                x: 0.5,
                y: 0.5
            }
        );
        // Spawn #2 sits exactly on the top-right corner: allowed.
        assert!(matches!(
            errors[2],
            ValidationError::SpawnOutOfBounds { index: 3, .. }
        ));
    }

    #[test]
    fn entity_errors() {
        let errors = errors_of(|l| {
            l.entities[0].kind = "dragon".into();
            l.entities[0].y = 10.0;
        });
        assert_eq!(
            errors,
            vec![
                ValidationError::UnknownEntityKind {
                    index: 0,
                    kind: "dragon".into()
                },
                ValidationError::EntityOutOfBounds {
                    index: 0,
                    kind: "dragon".into(),
                    x: 3.5,
                    y: 10.0
                },
            ]
        );
    }

    #[test]
    fn script_path_rules() {
        for ok in ["scripts/main.rhai", "scripts/lib/util.rhai"] {
            assert!(is_valid_script_path(ok), "{ok}");
        }
        for bad in [
            "main.rhai",
            "/scripts/main.rhai",
            "scripts/../../etc/passwd.rhai",
            "scripts/./main.rhai",
            "scripts//main.rhai",
            "scripts\\main.rhai",
            "scripts/main.lua",
            "scripts/",
        ] {
            assert!(!is_valid_script_path(bad), "{bad}");
        }
    }

    #[test]
    fn script_list_errors() {
        let errors = errors_of(|l| {
            l.scripts = vec![
                "scripts/main.rhai".into(),
                "../evil.rhai".into(),
                "scripts/main.rhai".into(),
            ];
        });
        assert_eq!(
            errors,
            vec![
                ValidationError::InvalidScriptPath {
                    path: "../evil.rhai".into()
                },
                ValidationError::DuplicateScript {
                    path: "scripts/main.rhai".into()
                },
            ]
        );
    }

    #[test]
    fn bundle_reports_missing_script_sources() {
        let mut level = valid_level();
        level.scripts = vec!["scripts/main.rhai".into(), "scripts/extra.rhai".into()];
        let mut bundle = LevelBundle {
            level,
            scripts: BTreeMap::new(),
        };
        bundle
            .scripts
            .insert("scripts/main.rhai".into(), "fn on_tick(dt) {}".into());
        assert_eq!(
            bundle.validate(),
            vec![ValidationError::MissingScript {
                path: "scripts/extra.rhai".into()
            }]
        );
    }

    #[test]
    fn errors_serialize_with_a_code_tag() {
        let json = serde_json::to_value(ValidationError::UnknownEntityKind {
            index: 2,
            kind: "dragon".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "code": "unknown_entity_kind", "index": 2, "kind": "dragon" })
        );
    }
}
