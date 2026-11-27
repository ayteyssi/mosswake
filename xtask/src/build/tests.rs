use super::*;
use mw_core::assets::{AnimationDef, MANIFEST_VERSION};

fn fixture(root: &Path, transparent: bool) -> AnimationManifest {
    let mut animations = BTreeMap::new();
    for (name, frames) in [("left", 2), ("right", 1)] {
        let animation = AnimationDef {
            path: format!("wizard/{name}"),
            frames,
            fps: 12.0,
            looping: name == "left",
        };
        fs::create_dir_all(root.join(&animation.path)).unwrap();
        for index in 0..frames {
            let mut image = RgbaImage::new(12, 12);
            if !transparent {
                image.put_pixel(3, 5, Rgba([255, 20, 0, 128]));
                if name == "left" && index == 1 {
                    image.put_pixel(1, 2, Rgba([255, 0, 0, 1]));
                }
                if name == "right" {
                    image.put_pixel(11, 0, Rgba([255, 0, 0, 255]));
                    image.put_pixel(4, 11, Rgba([255, 0, 0, 255]));
                }
            }
            image.save(root.join(animation.frame_path(index))).unwrap();
        }
        animations.insert(name.into(), animation);
    }
    let manifest = AnimationManifest {
        version: MANIFEST_VERSION,
        sprites: BTreeMap::from([(
            // Names are data, not paths in the generated tree.
            "wizard/../custom".into(),
            SpriteDef {
                frame_size: PixelSize { w: 12, h: 12 },
                pivot: PixelPoint { x: 7, y: 9 },
                pixels_per_tile: 8.0,
                hitbox: (!transparent).then_some(PixelRect {
                    x: 2,
                    y: 4,
                    w: 7,
                    h: 5,
                }),
                animations,
            },
        )]),
    };
    write_json(&root.join(MANIFEST_FILE), &manifest).unwrap();
    manifest
}

fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn generated_manifest_preserves_world_pivot_and_encloses_the_original_hitbox() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let output = dir.path().join("output");
    let original = fixture(&source, false);
    let report = build_assets(&source, &output, 4).unwrap();
    let generated = AnimationManifest::load(&output).unwrap();
    assert!(generated.validate(&output).is_empty());
    let old = &original.sprites["wizard/../custom"];
    let new = &generated.sprites["wizard/../custom"];
    let crop = report.sprites["wizard/../custom"];
    assert_eq!((crop.x, crop.y, crop.w, crop.h), (-1, -3, 16, 16));
    assert_eq!(new.frame_size, PixelSize { w: 4, h: 4 });
    assert_eq!(new.pixels_per_tile, old.pixels_per_tile / 4.0);
    assert_eq!(i64::from(new.pivot.x) * 4 + crop.x, i64::from(old.pivot.x));
    assert_eq!(i64::from(new.pivot.y) * 4 + crop.y, i64::from(old.pivot.y));
    let (h, g) = (old.hitbox.unwrap(), new.hitbox.unwrap());
    let old_edges = [h.x, h.y, h.x + h.w, h.y + h.h].map(i64::from);
    let generated_edges = [g.x, g.y, g.x + g.w, g.y + g.h];
    for (i, edge) in generated_edges.into_iter().enumerate() {
        let offset = if i % 2 == 0 { crop.x } else { crop.y };
        let expanded = i64::from(edge) * 4 + offset;
        let difference = if i < 2 {
            old_edges[i] - expanded
        } else {
            expanded - old_edges[i]
        };
        assert!(
            (0..4).contains(&difference),
            "hitbox edge {i}: {difference}"
        );
    }
    for (name, animation) in &old.animations {
        let actual = &new.animations[name];
        assert_eq!(
            (actual.frames, actual.fps, actual.looping),
            (animation.frames, animation.fps, animation.looping)
        );
    }
    assert_eq!(report.frames, 3);
    assert_eq!(report.source_rgba_bytes, 1728);
    assert_eq!(report.output_rgba_bytes, 192);
}

#[test]
fn builds_are_byte_identical_and_source_files_never_change() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fixture(&source, false);
    let before = tree(&source);
    let output = dir.path().join("output");
    build_assets(&source, &output, 4).unwrap();
    let first = tree(&output);
    build_assets(&source, &output, 4).unwrap();
    assert_eq!(tree(&output), first);
    let second = dir.path().join("other/output");
    build_assets(&source, &second, 4).unwrap();
    assert_eq!(tree(&second), first);
    assert_eq!(tree(&source), before);
}

#[test]
fn scale_one_preserves_every_nontransparent_source_pixel_across_animations() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let original = fixture(&source, false);
    let output = dir.path().join("output");
    let report = build_assets(&source, &output, 1).unwrap();
    let generated = AnimationManifest::load(&output).unwrap();
    let crop = report.sprites["wizard/../custom"];
    for (name, animation) in &original.sprites["wizard/../custom"].animations {
        let new = &generated.sprites["wizard/../custom"].animations[name];
        for index in 0..animation.frames {
            let old = image::open(source.join(animation.frame_path(index)))
                .unwrap()
                .into_rgba8();
            let actual = image::open(output.join(new.frame_path(index)))
                .unwrap()
                .into_rgba8();
            for (x, y, pixel) in old.enumerate_pixels().filter(|(_, _, p)| p[3] != 0) {
                assert_eq!(
                    actual.get_pixel(
                        (i64::from(x) - crop.x) as u32,
                        (i64::from(y) - crop.y) as u32
                    ),
                    pixel
                );
            }
        }
    }
}

#[test]
fn transparent_sprites_keep_a_valid_pivot_and_one_generated_pixel() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fixture(&source, true);
    let output = dir.path().join("output");
    build_assets(&source, &output, 4).unwrap();
    let manifest = AnimationManifest::load(&output).unwrap();
    assert!(manifest.validate(&output).is_empty());
    let sprite = &manifest.sprites["wizard/../custom"];
    assert_eq!(sprite.frame_size, PixelSize { w: 1, h: 1 });
    assert_eq!(sprite.pivot, PixelPoint { x: 0, y: 0 });
}

#[test]
fn resampling_translucent_edges_preserves_color() {
    let mut canvas = RgbaImage::new(8, 8);
    canvas.put_pixel(4, 4, Rgba([255, 0, 0, 128]));
    let reduced = downsample(canvas, PixelSize { w: 2, h: 2 });
    assert!(reduced.pixels().any(|p| p[3] > 0));
    for p in reduced.pixels().filter(|p| p[3] > 0) {
        assert_eq!([p[0], p[1], p[2]], [255, 0, 0]);
    }
}

#[test]
fn rebuild_removes_stale_frames_and_decode_failure_preserves_previous_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let mut manifest = fixture(&source, false);
    let output = dir.path().join("output");
    build_assets(&source, &output, 4).unwrap();
    let sprite = manifest.sprites.get_mut("wizard/../custom").unwrap();
    let animation = sprite.animations.get_mut("left").unwrap();
    fs::remove_file(source.join(animation.frame_path(1))).unwrap();
    animation.frames = 1;
    write_json(&source.join(MANIFEST_FILE), &manifest).unwrap();
    build_assets(&source, &output, 2).unwrap();
    let generated = AnimationManifest::load(&output).unwrap();
    assert!(generated.validate(&output).is_empty());
    let new = &generated.sprites["wizard/../custom"].animations["left"];
    assert!(!output.join(new.frame_path(1)).exists());
    let before = tree(&output);
    let frame = source.join(manifest.sprites["wizard/../custom"].animations["left"].frame_path(0));
    let png = fs::read(&frame).unwrap();
    fs::write(frame, &png[..24]).unwrap(); // Header passes validation; decoding must fail.
    assert!(build_assets(&source, &output, 4).is_err());
    assert_eq!(tree(&output), before);
}

#[test]
fn unsafe_destinations_invalid_scales_and_invalid_manifests_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let mut manifest = fixture(&source, false);
    let before = tree(&source);
    for output in [
        source.clone(),
        source.join("new/derived"),
        dir.path().to_path_buf(),
    ] {
        assert!(build_assets(&source, &output, 4).is_err());
    }
    let output = dir.path().join("output");
    for scale in [0, 65] {
        assert!(build_assets(&source, &output, scale).is_err());
    }
    fs::create_dir(&output).unwrap();
    fs::write(output.join("keep.txt"), b"keep").unwrap();
    assert!(build_assets(&source, &output, 4).is_err());
    assert_eq!(fs::read(output.join("keep.txt")).unwrap(), b"keep");
    assert_eq!(tree(&source), before);

    manifest
        .sprites
        .get_mut("wizard/../custom")
        .unwrap()
        .animations
        .get_mut("left")
        .unwrap()
        .path = "../escape".into();
    write_json(&source.join(MANIFEST_FILE), &manifest).unwrap();
    let new = dir.path().join("new");
    assert!(build_assets(&source, &new, 4).is_err());
    assert!(!new.exists());
}

#[cfg(unix)]
#[test]
fn output_symlinks_and_parent_symlinks_into_source_are_refused() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    fixture(&source, false);
    let before = tree(&source);
    let link = dir.path().join("link");
    symlink(&source, &link).unwrap();
    assert!(build_assets(&source, &link, 4).is_err());
    assert!(build_assets(&source, &link.join("generated"), 4).is_err());
    assert_eq!(tree(&source), before);
}
