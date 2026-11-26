//! Every level shipped in `levels/` must load, validate and already be in canonical form.

use std::fs;
use std::path::PathBuf;

use mw_level::{LayerKind, LevelBundle, LEVEL_FILE};

fn levels_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels")
}

fn level_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(levels_dir())
        .expect("levels/ exists")
        .map(|entry| entry.expect("readable entry").path())
        .filter(|path| path.join(LEVEL_FILE).is_file())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn repo_levels_are_valid_and_canonical() {
    let dirs = level_dirs();
    assert!(!dirs.is_empty(), "no level found in {:?}", levels_dir());
    for dir in dirs {
        let bundle = LevelBundle::load(&dir).unwrap_or_else(|e| panic!("{dir:?}: {e}"));
        let errors = bundle.validate();
        assert!(errors.is_empty(), "{dir:?} is invalid: {errors:#?}");

        let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        assert_eq!(
            bundle.level.id, dir_name,
            "level id must match its directory"
        );

        let on_disk = fs::read_to_string(dir.join(LEVEL_FILE)).expect("readable");
        assert_eq!(
            bundle.level.to_json().expect("serializable"),
            on_disk,
            "{dir:?}/{LEVEL_FILE} is not canonical: re-save it with LevelBundle::save"
        );
    }
}

#[test]
fn tutorial_has_what_the_sim_needs() {
    let bundle = LevelBundle::load(levels_dir().join("tutorial")).expect("tutorial loads");
    let level = &bundle.level;
    assert!(level.spawns.len() >= 2, "two players can spawn");
    assert!(level.layers.iter().any(|l| l.kind == LayerKind::Solid));
    assert!(level.entities.iter().any(|e| e.kind.starts_with("slime_")));
    // Each spawn stands on solid ground.
    for s in &level.spawns {
        assert!(level.is_solid(s.x.floor() as u32, s.y as u32 - 1), "{s:?}");
    }
}
