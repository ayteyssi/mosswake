use std::{collections::BTreeMap, fs, path::Path};

use mosswake_mcp::*;
use mw_level::LevelBundle;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult};
use serde_json::{json, Value};

fn setup() -> (tempfile::TempDir, LevelServer) {
    let dir = tempfile::tempdir().unwrap();
    let atlas = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/tilesets/mossy");
    let server = LevelServer::new(LevelStore::new(dir.path(), atlas).unwrap());
    (dir, server)
}

fn ok(result: CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.unwrap()
}

fn error(result: CallToolResult) -> String {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    result.structured_content.unwrap()["error"]
        .as_str()
        .unwrap()
        .into()
}

fn id() -> Parameters<LevelId> {
    Parameters(LevelId { id: "demo".into() })
}

fn create(server: &LevelServer) {
    ok(server.create_level(Parameters(CreateLevel {
        id: "demo".into(),
        name: "Demo".into(),
        width: 8,
        height: 5,
        spawns: Some(vec![Point { x: 1.5, y: 1.0 }]),
    })));
}

fn tile() -> SetTiles {
    SetTiles {
        id: "demo".into(),
        layer: "ground".into(),
        kind: None,
        x: 0,
        y: 0,
        width: 8,
        height: 1,
        tile: "#".into(),
        tile_id: Some("mossy_ground".into()),
    }
}

#[test]
fn complete_level_editing_roundtrip_without_transport() {
    let (dir, server) = setup();
    assert_eq!(ok(server.list_levels())["levels"], json!([]));
    create(&server);
    ok(server.set_tiles(Parameters(tile())));
    let mut rectangle = tile();
    rectangle.layer = "decorations".into();
    rectangle.kind = Some(LayerType::Decor);
    rectangle.x = 3;
    rectangle.y = 2;
    rectangle.width = 2;
    rectangle.height = 2;
    ok(server.set_tiles(Parameters(rectangle)));
    let mut erase = tile();
    erase.layer = "decorations".into();
    erase.x = 3;
    erase.y = 2;
    erase.width = 1;
    erase.tile = ".".into();
    erase.tile_id = None;
    ok(server.set_tiles(Parameters(erase)));
    let level = ok(server.get_level(id()));
    assert_eq!(
        level["grids_top_to_bottom"]["ground"],
        "........\n........\n........\n........\n########"
    );
    assert_eq!(
        level["grids_top_to_bottom"]["decorations"],
        "........\n...##...\n....#...\n........\n........"
    );

    for kind in ["slime_green", "slime_orange"] {
        ok(server.place_entity(Parameters(PlaceEntity {
            id: "demo".into(),
            kind: kind.into(),
            x: 4.5,
            y: 1.0,
            props: BTreeMap::new(),
        })));
    }
    assert_eq!(
        ok(server.get_level(id()))["indexed_entities"][1]["index"],
        1
    );
    ok(server.remove_entity(Parameters(RemoveEntity {
        id: "demo".into(),
        index: 0,
    })));
    assert_eq!(
        ok(server.get_level(id()))["level"]["entities"][0]["kind"],
        "slime_orange"
    );

    for (rules, replace) in [
        (json!({"gravity":-900.0}), false),
        (json!({"run_speed":100.0}), false),
        (json!({"gravity":-1800.0}), true),
    ] {
        ok(server.set_rules(Parameters(SetRules {
            id: "demo".into(),
            rules: serde_json::from_value(rules).unwrap(),
            replace,
        })));
    }
    assert_eq!(
        ok(server.get_level(id()))["level"]["rules"],
        json!({"gravity":-1800.0})
    );
    for source in ["fn on_tick(dt) {}", "fn on_level_start() {}"] {
        ok(server.write_script(Parameters(WriteScript {
            id: "demo".into(),
            path: "scripts/nested/main.rhai".into(),
            source: source.into(),
        })));
        assert_eq!(
            ok(server.read_script(Parameters(ScriptPath {
                id: "demo".into(),
                path: "scripts/nested/main.rhai".into()
            })))["source"],
            source
        );
    }
    assert_eq!(
        ok(server.get_level(id()))["level"]["scripts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ok(server.validate_level(id())),
        json!({"valid":true,"errors":[]})
    );
    assert_eq!(ok(server.list_levels())["levels"][0]["id"], "demo");
    let palette = ok(server.list_palette());
    assert!(palette["terrains"]
        .as_array()
        .unwrap()
        .contains(&json!("mossy_ground")));
    assert!(palette["pieces"]["platform_2x1_a"].is_object());
    assert!(palette["entity_kinds"]
        .as_array()
        .unwrap()
        .contains(&json!("slime_green")));
    let bundle = LevelBundle::load(dir.path().join("demo")).unwrap();
    assert!(bundle.validate().is_empty());
    assert_eq!(
        fs::read_to_string(dir.path().join("demo/level.json")).unwrap(),
        bundle.level.to_json().unwrap()
    );
}

#[test]
fn failed_edits_do_not_change_existing_bytes() {
    let (dir, server) = setup();
    create(&server);
    let path = dir.path().join("demo/level.json");
    let before = fs::read(&path).unwrap();
    let mut spawn_blocker = tile();
    spawn_blocker.y = 1;
    assert!(error(server.set_tiles(Parameters(spawn_blocker))).contains("spawn_in_solid"));
    let mut outside = tile();
    outside.x = u32::MAX;
    assert!(error(server.set_tiles(Parameters(outside))).contains("rectangle"));
    let mut unknown = tile();
    unknown.tile_id = None;
    assert!(error(server.set_tiles(Parameters(unknown))).contains("unknown_tile"));
    let mut bad = tile();
    bad.tile = "é".into();
    assert!(error(server.set_tiles(Parameters(bad))).contains("ASCII"));
    let mut change_kind = tile();
    change_kind.kind = Some(LayerType::Hazard);
    assert!(error(server.set_tiles(Parameters(change_kind))).contains("kind"));
    assert!(error(server.place_entity(Parameters(PlaceEntity {
        id: "demo".into(),
        kind: "unknown".into(),
        x: 0.0,
        y: 0.0,
        props: BTreeMap::new()
    })))
    .contains("unknown_entity_kind"));
    assert!(error(server.remove_entity(Parameters(RemoveEntity {
        id: "demo".into(),
        index: 99
    })))
    .contains("index"));
    assert!(error(server.create_level(Parameters(CreateLevel {
        id: "demo".into(),
        name: "Overwrite".into(),
        width: 2,
        height: 2,
        spawns: None
    })))
    .contains("already exists"));
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn invalid_paths_and_oversized_inputs_are_rejected() {
    let (dir, server) = setup();
    create(&server);
    for bad in ["../outside", "..", "/tmp/demo", "a/b", "a\\b", ""] {
        assert!(
            error(server.get_level(Parameters(LevelId { id: bad.into() })))
                .contains("invalid level id")
        );
    }
    for path in [
        "../escape.rhai",
        "scripts/../escape.rhai",
        "scripts/main.txt",
    ] {
        assert!(error(server.write_script(Parameters(WriteScript {
            id: "demo".into(),
            path: path.into(),
            source: "abc".into()
        })))
        .contains("invalid script path"));
    }
    assert!(error(server.write_script(Parameters(WriteScript {
        id: "demo".into(),
        path: "scripts/huge.rhai".into(),
        source: "x".repeat(1024 * 1024 + 1)
    })))
    .contains("1 MiB"));
    assert!(!dir.path().join("demo/scripts").exists());
    for (width, height) in [(0, 1), (u32::MAX, u32::MAX)] {
        assert!(error(server.create_level(Parameters(CreateLevel {
            id: "oversize".into(),
            name: "Too big".into(),
            width,
            height,
            spawns: None
        })))
        .contains("262144"));
    }
    assert!(!dir.path().join("oversize").exists());
}

#[test]
fn missing_scripts_and_malformed_levels_are_reported() {
    let (dir, server) = setup();
    create(&server);
    ok(server.write_script(Parameters(WriteScript {
        id: "demo".into(),
        path: "scripts/main.rhai".into(),
        source: "fn on_tick(dt) {}".into(),
    })));
    fs::remove_file(dir.path().join("demo/scripts/main.rhai")).unwrap();
    let report = ok(server.validate_level(id()));
    assert_eq!(report["valid"], false);
    assert_eq!(report["errors"][0]["code"], "missing_script");
    // An editor can repair missing sources, rather than requiring the input to be valid.
    ok(server.write_script(Parameters(WriteScript {
        id: "demo".into(),
        path: "scripts/main.rhai".into(),
        source: "".into(),
    })));
    fs::create_dir(dir.path().join("broken")).unwrap();
    fs::write(dir.path().join("broken/level.json"), "{").unwrap();
    let listed = ok(server.list_levels());
    assert!(listed["levels"][0]["error"].is_string());
    assert_eq!(listed["levels"][1]["id"], "demo");
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_redirect_level_or_script_reads_and_writes() {
    use std::os::unix::fs::symlink;
    let (dir, server) = setup();
    create(&server);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel.rhai"), "unchanged").unwrap();
    symlink(outside.path(), dir.path().join("demo/scripts")).unwrap();
    assert!(error(server.write_script(Parameters(WriteScript {
        id: "demo".into(),
        path: "scripts/sentinel.rhai".into(),
        source: "changed".into()
    })))
    .contains("symlinks"));
    assert_eq!(
        fs::read_to_string(outside.path().join("sentinel.rhai")).unwrap(),
        "unchanged"
    );
    symlink(outside.path(), dir.path().join("linked")).unwrap();
    assert!(error(server.get_level(Parameters(LevelId {
        id: "linked".into()
    })))
    .contains("symlinks"));
}
