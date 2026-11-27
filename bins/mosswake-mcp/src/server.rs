use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use anyhow::{bail, Context, Result};
use mw_level::{EntityDef, Layer, LayerKind, Level, LevelBundle, Size, Spawn};
use rmcp::{
    handler::server::wrapper::Parameters, model::CallToolResult, schemars, tool, tool_router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::LevelStore;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LevelId {
    /// Directory id under levels/: 1..=64 characters from [a-z0-9_], never a path.
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    /// Horizontal feet position in tiles, origin at the left edge.
    pub x: f32,
    /// Vertical feet position in tiles, Y up, origin at the bottom edge.
    pub y: f32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLevel {
    /// New directory id: [a-z0-9_], 1..=64 characters. Existing levels are never overwritten.
    pub id: String,
    /// Human-readable level name.
    pub name: String,
    /// Grid width in tiles (positive; width * height <= 262144).
    pub width: u32,
    /// Grid height in tiles (positive; width * height <= 262144).
    pub height: u32,
    /// Optional feet positions in tiles, Y up. Default: [{x:0.5,y:0.0}].
    pub spawns: Option<Vec<Point>>,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LayerType {
    Solid,
    Decor,
    Hazard,
}

impl From<LayerType> for LayerKind {
    fn from(kind: LayerType) -> Self {
        match kind {
            LayerType::Solid => Self::Solid,
            LayerType::Decor => Self::Decor,
            LayerType::Hazard => Self::Hazard,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetTiles {
    /// Existing level directory id.
    pub id: String,
    /// Layer name. A missing layer is created only if kind is supplied.
    pub layer: String,
    /// Kind of a newly created layer; cannot change an existing layer's kind.
    pub kind: Option<LayerType>,
    /// Leftmost cell of the rectangle, in tiles.
    pub x: u32,
    /// Bottom cell of the rectangle, Y up.
    pub y: u32,
    /// Rectangle width (>=1); use 1 for a vertical line.
    pub width: u32,
    /// Rectangle height (>=1); use 1 for a horizontal line.
    pub height: u32,
    /// Exactly one printable ASCII non-space character; '.' erases cells.
    pub tile: String,
    /// Optional atlas piece/terrain id to add/update this character in the level legend.
    /// Omit if the character already exists; do not supply when erasing with '.'.
    pub tile_id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceEntity {
    /// Existing level directory id.
    pub id: String,
    /// An entity kind returned by list_palette (currently slime_green or slime_orange).
    pub kind: String,
    /// Horizontal feet position in tiles.
    pub x: f32,
    /// Vertical feet position in tiles, Y up.
    pub y: f32,
    /// Kind-specific properties, for example {"patrol":2.0}. Defaults to {}.
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveEntity {
    /// Existing level directory id.
    pub id: String,
    /// Zero-based entity index from the latest get_level. Later indices shift after removal.
    pub index: usize,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetRules {
    /// Existing level directory id.
    pub id: String,
    /// Rule overrides, e.g. {"gravity":-1800,"mechanics":["movement","jump"]}.
    /// mw_level validates structure; rule keys/types are checked by mw_sim when playing.
    pub rules: BTreeMap<String, Value>,
    /// False (default): merge supplied keys. True: replace all overrides.
    #[serde(default)]
    pub replace: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScriptPath {
    /// Existing level directory id.
    pub id: String,
    /// Relative path scripts/**/*.rhai, e.g. scripts/main.rhai. No '..' or symlinks.
    pub path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteScript {
    /// Existing level directory id.
    pub id: String,
    /// Relative path scripts/**/*.rhai. New files are added to level.scripts automatically.
    pub path: String,
    /// UTF-8 Rhai source (maximum 1 MiB). Stored as text; compilation belongs to mw_script.
    pub source: String,
}

#[derive(Clone)]
pub struct LevelServer {
    store: Arc<Mutex<LevelStore>>,
}

impl LevelServer {
    pub fn new(store: LevelStore) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    fn run(&self, action: impl FnOnce(&LevelStore) -> Result<Value>) -> CallToolResult {
        match self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("level store lock poisoned"))
            .and_then(|store| action(&store))
        {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(json!({"error": format!("{error:#}")})),
        }
    }

    fn edit(
        &self,
        id: &str,
        action: impl FnOnce(&mut LevelBundle) -> Result<Value>,
    ) -> CallToolResult {
        self.run(|store| {
            let mut bundle = store.load(id)?;
            let output = action(&mut bundle)?;
            store.save(&bundle)?;
            Ok(output)
        })
    }
}

#[tool_router(server_handler)]
impl LevelServer {
    #[tool(
        description = "List levels in sorted id order, including names, sizes and validation errors. No writes. Malformed levels are reported without hiding the others."
    )]
    pub fn list_levels(&self) -> CallToolResult {
        self.run(LevelStore::list)
    }

    #[tool(
        description = "List available entity kinds, named atlas pieces (with kind, size and collision) and autotiled terrain ids. Use these ids in set_tiles.tile_id."
    )]
    pub fn list_palette(&self) -> CallToolResult {
        self.run(|store| Ok(store.palette()))
    }

    #[tool(
        description = "Create a valid empty level with a ground (solid) layer and spawn(s). Coordinates are tiles, Y up. Use set_tiles to draw; existing levels are never overwritten."
    )]
    pub fn create_level(&self, Parameters(args): Parameters<CreateLevel>) -> CallToolResult {
        self.run(|store| {
            if args.width == 0
                || args.height == 0
                || u64::from(args.width) * u64::from(args.height) > 262144
            {
                bail!("width and height must be positive, with at most 262144 cells");
            }
            let mut level = Level::new(
                &args.id,
                args.name,
                Size {
                    w: args.width,
                    h: args.height,
                },
            );
            level.layers.push(Layer {
                name: "ground".into(),
                kind: LayerKind::Solid,
                tiles: vec![".".repeat(args.width as usize); args.height as usize],
            });
            level.spawns = args
                .spawns
                .unwrap_or_else(|| vec![Point { x: 0.5, y: 0.0 }])
                .into_iter()
                .map(|p| Spawn { x: p.x, y: p.y })
                .collect();
            store.create(&LevelBundle {
                level,
                scripts: BTreeMap::new(),
            })?;
            Ok(json!({"created": args.id}))
        })
    }

    #[tool(
        description = "Read a level: full JSON, readable character grids (first row is top), indexed entities and validation errors. Coordinates used by editing tools are Y up."
    )]
    pub fn get_level(&self, Parameters(args): Parameters<LevelId>) -> CallToolResult {
        self.run(|store| store.get(&args.id))
    }

    #[tool(
        description = "Fill a rectangle or a line with one tile character; '.' erases. (x,y) is its bottom-left cell, Y up. Bounds and the complete resulting level are validated before any file is changed. A new layer requires kind."
    )]
    pub fn set_tiles(&self, Parameters(args): Parameters<SetTiles>) -> CallToolResult {
        self.edit(&args.id, |bundle| {
            let level = &mut bundle.level;
            if args.width == 0
                || args.height == 0
                || args
                    .x
                    .checked_add(args.width)
                    .is_none_or(|x| x > level.size.w)
                || args
                    .y
                    .checked_add(args.height)
                    .is_none_or(|y| y > level.size.h)
            {
                bail!("tile rectangle must have positive size and fit inside the level");
            }
            if args.tile.len() != 1 || !args.tile.as_bytes()[0].is_ascii_graphic() {
                bail!("tile must be exactly one printable ASCII non-space character");
            }
            let tile = args.tile.as_bytes()[0] as char;
            if let Some(tile_id) = args.tile_id {
                if tile == '.' {
                    bail!("'.' is reserved for empty cells and cannot have a tile_id");
                }
                level.legend.insert(tile, tile_id);
            }
            let index = match level.layers.iter().position(|l| l.name == args.layer) {
                Some(index) => {
                    if args
                        .kind
                        .is_some_and(|kind| LayerKind::from(kind) != level.layers[index].kind)
                    {
                        bail!("kind cannot change an existing layer");
                    }
                    index
                }
                None => {
                    let kind = args
                        .kind
                        .context("layer does not exist; provide kind to create it")?;
                    level.layers.push(Layer {
                        name: args.layer,
                        kind: kind.into(),
                        tiles: vec![".".repeat(level.size.w as usize); level.size.h as usize],
                    });
                    level.layers.len() - 1
                }
            };
            let layer = &mut level.layers[index];
            for y in args.y..args.y + args.height {
                let row = layer
                    .tiles
                    .get_mut((level.size.h - 1 - y) as usize)
                    .context("malformed layer row count; validate_level for details")?;
                // Work in characters even when repairing a malformed non-ASCII input file.
                let mut cells: Vec<_> = row.chars().collect();
                let end = (args.x + args.width) as usize;
                if cells.len() < end {
                    bail!("malformed layer row width; validate_level for details");
                }
                cells[args.x as usize..end].fill(tile);
                *row = cells.into_iter().collect();
            }
            Ok(json!({"updated": args.id}))
        })
    }

    #[tool(
        description = "Append an entity of a kind returned by list_palette, using feet coordinates in tiles (Y up). Returns its zero-based index. Invalid placements leave the files unchanged."
    )]
    pub fn place_entity(&self, Parameters(args): Parameters<PlaceEntity>) -> CallToolResult {
        self.edit(&args.id, |bundle| {
            let index = bundle.level.entities.len();
            bundle.level.entities.push(EntityDef {
                kind: args.kind,
                x: args.x,
                y: args.y,
                props: args.props,
            });
            Ok(json!({"index":index}))
        })
    }

    #[tool(
        description = "Remove the entity at the zero-based index returned by get_level/place_entity. Obtain fresh indices after each removal. Out-of-range indices do not modify files."
    )]
    pub fn remove_entity(&self, Parameters(args): Parameters<RemoveEntity>) -> CallToolResult {
        self.edit(&args.id, |bundle| {
            if args.index >= bundle.level.entities.len() {
                bail!("entity index out of range");
            }
            bundle.level.entities.remove(args.index);
            Ok(json!({"removed":args.index}))
        })
    }

    #[tool(
        description = "Merge rule overrides, or replace them all with replace=true. Structural validation runs before saving; gameplay rule names/types are validated by mw_sim on load, not this editor."
    )]
    pub fn set_rules(&self, Parameters(args): Parameters<SetRules>) -> CallToolResult {
        self.edit(&args.id, |bundle| {
            if args.replace {
                bundle.level.rules = args.rules;
            } else {
                bundle.level.rules.extend(args.rules);
            }
            Ok(json!({"rules":bundle.level.rules}))
        })
    }

    #[tool(
        description = "Create or replace a Rhai script (max 1 MiB) and automatically list it in level.scripts. Path must be scripts/**/*.rhai without traversal/symlinks. Validates the bundle before atomically replacing files; does not compile Rhai."
    )]
    pub fn write_script(&self, Parameters(args): Parameters<WriteScript>) -> CallToolResult {
        self.edit(&args.id, |bundle| {
            if !mw_level::is_valid_script_path(&args.path) {
                bail!("invalid script path: use scripts/**/*.rhai without '..'");
            }
            if args.source.len() > 1024 * 1024 {
                bail!("script exceeds the 1 MiB limit");
            }
            if !bundle.level.scripts.contains(&args.path) {
                bundle.level.scripts.push(args.path.clone());
            }
            bundle.scripts.insert(args.path.clone(), args.source);
            Ok(json!({"written":args.path}))
        })
    }

    #[tool(
        description = "Read the UTF-8 source of a script listed by the level. Path is relative scripts/**/*.rhai; unlisted scripts and traversal are rejected."
    )]
    pub fn read_script(&self, Parameters(args): Parameters<ScriptPath>) -> CallToolResult {
        self.run(|store| {
            if !mw_level::is_valid_script_path(&args.path) {
                bail!("invalid script path");
            }
            let bundle = store.load(&args.id)?;
            if !bundle.level.scripts.contains(&args.path) {
                bail!("script is not listed in level.scripts");
            }
            let source = bundle
                .scripts
                .get(&args.path)
                .context("listed script file is missing")?;
            Ok(json!({"path":args.path,"source":source}))
        })
    }

    #[tool(
        description = "Return all mw_level validation errors (structured code fields), including missing scripts. No files are modified. This checks level structure and that every legend id exists in the tileset atlas (`unknown_tile_id`), not Rhai syntax or gameplay rule keys/types."
    )]
    pub fn validate_level(&self, Parameters(args): Parameters<LevelId>) -> CallToolResult {
        self.run(|store| {
            let errors = store.validate(&store.load(&args.id)?);
            Ok(json!({"valid":errors.is_empty(), "errors":errors}))
        })
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    #[test]
    fn tool_schemas_expose_the_full_editing_surface() {
        let tools = LevelServer::tool_router().list_all();
        let mut names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "create_level",
                "get_level",
                "list_levels",
                "list_palette",
                "place_entity",
                "read_script",
                "remove_entity",
                "set_rules",
                "set_tiles",
                "validate_level",
                "write_script"
            ]
        );
        for tool in tools {
            assert!(!tool.description.as_ref().unwrap().is_empty());
            if let Some(properties) = tool
                .input_schema
                .get("properties")
                .and_then(Value::as_object)
            {
                for (name, field) in properties {
                    assert!(
                        field.get("description").is_some(),
                        "{}.{name} has no description",
                        tool.name
                    );
                }
            }
        }
    }
}
