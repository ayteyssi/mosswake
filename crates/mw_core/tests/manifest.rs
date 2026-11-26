//! The real `assets/manifest.json` must be valid and cover every frame directory in `assets/`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mw_core::assets::AnimationManifest;

fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// Every directory under `root` holding a `0000.png`, as a `/`-separated relative path.
fn frame_dirs(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{dir:?}: {e}"));
    for entry in entries {
        let path = entry.expect("readable entry").path();
        if path.is_dir() {
            if path.join("0000.png").is_file() {
                let rel = path.strip_prefix(root).expect("under root");
                let parts: Vec<_> = rel.iter().map(|p| p.to_string_lossy()).collect();
                out.insert(parts.join("/"));
            }
            frame_dirs(root, &path, out);
        }
    }
}

#[test]
fn real_manifest_is_valid() {
    let root = assets_root();
    let manifest = AnimationManifest::load(&root).expect("manifest loads");
    let issues = manifest.validate(&root);
    assert!(issues.is_empty(), "manifest issues: {issues:#?}");
}

#[test]
fn real_manifest_covers_every_animation() {
    let root = assets_root();
    let manifest = AnimationManifest::load(&root).expect("manifest loads");
    let referenced: BTreeSet<String> = manifest
        .sprites
        .values()
        .flat_map(|s| s.animations.values().map(|a| a.path.clone()))
        .collect();
    let mut on_disk = BTreeSet::new();
    frame_dirs(&root, &root, &mut on_disk);
    assert!(on_disk.len() >= 21, "found only {on_disk:?}");
    let missing: Vec<_> = on_disk.difference(&referenced).collect();
    assert!(
        missing.is_empty(),
        "frame dirs not in manifest: {missing:?}"
    );
}

#[test]
fn entity_sprites_have_hitboxes() {
    let manifest = AnimationManifest::load(assets_root()).expect("manifest loads");
    for id in ["blue_wizard", "slime_green", "slime_orange"] {
        let sprite = &manifest.sprites[id];
        assert!(sprite.hitbox.is_some(), "{id} needs a suggested hitbox");
        assert!(
            sprite.animations.contains_key("idle"),
            "{id} needs an idle animation"
        );
    }
}
