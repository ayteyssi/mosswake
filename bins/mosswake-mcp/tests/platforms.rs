//! The platform layer kind must be available through both MCP input schemas and handlers.
use std::path::Path;

use mosswake_mcp::{CreateLevel, LayerType, LevelServer, LevelStore, SetTiles};
use mw_level::{LayerKind, LevelBundle};
use rmcp::{handler::server::wrapper::Parameters, schemars};
use serde_json::json;

#[test]
fn platform_layer_can_be_created_and_edited_through_mcp() {
    let dir = tempfile::tempdir().unwrap();
    let atlas = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/tilesets/mossy");
    let server = LevelServer::new(LevelStore::new(dir.path(), atlas).unwrap());
    let result = server.create_level(Parameters(CreateLevel {
        id: "platform_test".into(),
        name: "Platforms".into(),
        width: 8,
        height: 8,
        spawns: None,
    }));
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let args = json!({"id":"platform_test","layer":"platforms","kind":"platform",
        "x":2,"y":3,"width":3,"height":1,"tile":"=","tile_id":"mossy_ground"});
    let result = server.set_tiles(Parameters(
        serde_json::from_value::<SetTiles>(args).unwrap(),
    ));
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let loaded = LevelBundle::load(dir.path().join("platform_test")).unwrap();
    assert_eq!(loaded.level.schema_version, 2);
    assert_eq!(loaded.level.layers[1].kind, LayerKind::Platform);
    assert!(loaded.level.is_platform(2, 3));
    assert!(loaded.level.is_platform(4, 3));
    assert!(!loaded.level.is_solid(2, 3));
    // A later write may omit kind and keeps the platform behavior.
    let edit = json!({"id":"platform_test","layer":"platforms","x":3,"y":3,
        "width":1,"height":1,"tile":"."});
    let result = server.set_tiles(Parameters(
        serde_json::from_value::<SetTiles>(edit).unwrap(),
    ));
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let loaded = LevelBundle::load(dir.path().join("platform_test")).unwrap();
    assert!(loaded.level.is_platform(2, 3));
    assert!(!loaded.level.is_platform(3, 3));
    assert!(loaded.level.is_platform(4, 3));
    assert!(loaded.validate().is_empty());
}

#[test]
fn mcp_layer_schema_lists_platform() {
    let schema = serde_json::to_value(schemars::schema_for!(LayerType)).unwrap();
    assert!(
        schema["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("platform")),
        "{schema}"
    );
}
