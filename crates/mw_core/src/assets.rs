//! Animation manifest (`assets/manifest.json`): which PNG frame sequences exist and how to
//! draw them (fps, pivot, display scale, suggested hitbox). Shared by the renderer and the sim.
//!
//! Format documented in `docs/ASSETS.md`. All pixel coordinates are in **source frame space**:
//! origin at the frame's top-left corner, y pointing **down**.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Manifest file name, relative to the assets root.
pub const MANIFEST_FILE: &str = "manifest.json";

/// Manifest format version understood by this build.
pub const MANIFEST_VERSION: u32 = 1;

/// The whole manifest: sprites keyed by id (e.g. `blue_wizard`, `slime_green`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationManifest {
    pub version: u32,
    pub sprites: BTreeMap<String, SpriteDef>,
}

/// A drawable thing with one or more animations sharing the same frame layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpriteDef {
    /// Size of every frame of every animation, in pixels.
    pub frame_size: PixelSize,
    /// Point of the frame placed at the entity position: the feet for characters, the base
    /// for plants (top for hanging ones). Shared by all animations so switching never jitters.
    pub pivot: PixelPoint,
    /// Source pixels per level tile. Render scale = `tile_size / pixels_per_tile`.
    pub pixels_per_tile: f32,
    /// Suggested collision box. The simulation's `Rules` own the actual gameplay value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hitbox: Option<PixelRect>,
    /// Animations keyed by name (e.g. `idle`, `walk`).
    pub animations: BTreeMap<String, AnimationDef>,
}

/// One PNG frame sequence: `<path>/0000.png`, `<path>/0001.png`, …
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationDef {
    /// Directory of the frames, relative to the assets root (`/` separators).
    pub path: String,
    /// Number of frames.
    pub frames: u32,
    /// Playback speed, in frames per second.
    pub fps: f32,
    /// Whether playback wraps around; otherwise it holds the last frame.
    pub looping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelSize {
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelPoint {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Failure to read or parse the manifest file.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid manifest JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// A problem found by [`AnimationManifest::validate`].
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ManifestIssue {
    #[error("manifest version {found} is not supported (expected {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("sprite `{sprite}`: frame_size must be at least 1x1")]
    EmptyFrameSize { sprite: String },
    #[error("sprite `{sprite}`: pixels_per_tile must be a positive number")]
    InvalidPixelsPerTile { sprite: String },
    #[error("sprite `{sprite}`: pivot is outside the frame")]
    PivotOutsideFrame { sprite: String },
    #[error("sprite `{sprite}`: hitbox is empty or outside the frame")]
    InvalidHitbox { sprite: String },
    #[error("sprite `{sprite}` has no animation")]
    NoAnimation { sprite: String },
    #[error("animation `{sprite}/{animation}` has no frame")]
    NoFrame { sprite: String, animation: String },
    #[error("animation `{sprite}/{animation}`: fps must be a positive number")]
    InvalidFps { sprite: String, animation: String },
    #[error("animation `{sprite}/{animation}`: path `{path}` must be relative, without `..`")]
    InvalidPath {
        sprite: String,
        animation: String,
        path: String,
    },
    #[error("frame `{path}` is missing")]
    MissingFrame { path: String },
    #[error("frame `{path}` is not a readable PNG: {reason}")]
    UnreadableFrame { path: String, reason: String },
    #[error("frame `{path}` is {found_w}x{found_h}, expected {expected_w}x{expected_h}")]
    WrongFrameSize {
        path: String,
        expected_w: u32,
        expected_h: u32,
        found_w: u32,
        found_h: u32,
    },
    #[error("frame `{path}` exists beyond the declared count of `{sprite}/{animation}`")]
    ExtraFrame {
        sprite: String,
        animation: String,
        path: String,
    },
}

impl AnimationDef {
    /// Path of frame `index`, relative to the assets root (e.g. `enemies/slime_green/0007.png`).
    pub fn frame_path(&self, index: u32) -> String {
        format!("{}/{index:04}.png", self.path)
    }

    /// Duration of the whole sequence, in seconds.
    pub fn duration_secs(&self) -> f32 {
        self.frames as f32 / self.fps
    }
}

impl AnimationManifest {
    /// Parses manifest JSON. Does not validate (see [`AnimationManifest::validate`]).
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        Ok(serde_json::from_str(json)?)
    }

    /// Reads `<assets_root>/manifest.json`.
    pub fn load(assets_root: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let path = assets_root.as_ref().join(MANIFEST_FILE);
        let json = std::fs::read_to_string(&path).map_err(|source| ManifestError::Io {
            path: path.clone(),
            source,
        })?;
        Self::from_json(&json)
    }

    /// Checks values and that every declared frame exists under `assets_root` with the
    /// declared size (and no undeclared frame follows). Returns every issue found.
    pub fn validate(&self, assets_root: impl AsRef<Path>) -> Vec<ManifestIssue> {
        let root = assets_root.as_ref();
        let mut issues = Vec::new();
        if self.version != MANIFEST_VERSION {
            issues.push(ManifestIssue::UnsupportedVersion {
                found: self.version,
                expected: MANIFEST_VERSION,
            });
        }
        for (name, sprite) in &self.sprites {
            sprite.validate(name, root, &mut issues);
        }
        issues
    }
}

impl SpriteDef {
    fn validate(&self, name: &str, root: &Path, issues: &mut Vec<ManifestIssue>) {
        let sprite = || name.to_string();
        let PixelSize { w, h } = self.frame_size;
        if w == 0 || h == 0 {
            issues.push(ManifestIssue::EmptyFrameSize { sprite: sprite() });
        }
        if !(self.pixels_per_tile.is_finite() && self.pixels_per_tile > 0.0) {
            issues.push(ManifestIssue::InvalidPixelsPerTile { sprite: sprite() });
        }
        if self.pivot.x > w || self.pivot.y > h {
            issues.push(ManifestIssue::PivotOutsideFrame { sprite: sprite() });
        }
        if let Some(r) = self.hitbox {
            let fits = r.x.checked_add(r.w).is_some_and(|right| right <= w)
                && r.y.checked_add(r.h).is_some_and(|bottom| bottom <= h);
            if r.w == 0 || r.h == 0 || !fits {
                issues.push(ManifestIssue::InvalidHitbox { sprite: sprite() });
            }
        }
        if self.animations.is_empty() {
            issues.push(ManifestIssue::NoAnimation { sprite: sprite() });
        }
        for (anim_name, anim) in &self.animations {
            let animation = || anim_name.to_string();
            if anim.frames == 0 {
                issues.push(ManifestIssue::NoFrame {
                    sprite: sprite(),
                    animation: animation(),
                });
            }
            if !(anim.fps.is_finite() && anim.fps > 0.0) {
                issues.push(ManifestIssue::InvalidFps {
                    sprite: sprite(),
                    animation: animation(),
                });
            }
            if !is_safe_relative_path(&anim.path) {
                issues.push(ManifestIssue::InvalidPath {
                    sprite: sprite(),
                    animation: animation(),
                    path: anim.path.clone(),
                });
                continue; // never touch the disk outside the assets root
            }
            for index in 0..anim.frames {
                let rel = anim.frame_path(index);
                if let Some(issue) = check_frame(root, &rel, self.frame_size) {
                    issues.push(issue);
                }
            }
            let next = anim.frame_path(anim.frames);
            if root.join(&next).exists() {
                issues.push(ManifestIssue::ExtraFrame {
                    sprite: sprite(),
                    animation: animation(),
                    path: next,
                });
            }
        }
    }
}

fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn check_frame(root: &Path, rel: &str, expected: PixelSize) -> Option<ManifestIssue> {
    let path = root.join(rel);
    let (found_w, found_h) = match png_size(&path) {
        Ok(size) => size,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Some(ManifestIssue::MissingFrame { path: rel.into() })
        }
        Err(e) => {
            return Some(ManifestIssue::UnreadableFrame {
                path: rel.into(),
                reason: e.to_string(),
            })
        }
    };
    (found_w != expected.w || found_h != expected.h).then(|| ManifestIssue::WrongFrameSize {
        path: rel.into(),
        expected_w: expected.w,
        expected_h: expected.h,
        found_w,
        found_h,
    })
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// Reads a PNG's dimensions from its header (the IHDR chunk always comes first),
/// without decoding the image.
pub fn png_size(path: &Path) -> std::io::Result<(u32, u32)> {
    let mut header = [0u8; 24];
    File::open(path)?.read_exact(&mut header)?;
    if header[..8] != PNG_SIGNATURE || &header[12..16] != b"IHDR" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a PNG file",
        ));
    }
    let be =
        |i: usize| u32::from_be_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    Ok((be(16), be(20)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes just enough of a PNG (signature + IHDR start) for [`png_size`].
    fn fake_png(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(w.to_be_bytes());
        bytes.extend(h.to_be_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    fn sprite(path: &str, frames: u32) -> SpriteDef {
        SpriteDef {
            frame_size: PixelSize { w: 8, h: 4 },
            pivot: PixelPoint { x: 4, y: 4 },
            pixels_per_tile: 4.0,
            hitbox: Some(PixelRect {
                x: 2,
                y: 0,
                w: 4,
                h: 4,
            }),
            animations: BTreeMap::from([(
                "idle".to_string(),
                AnimationDef {
                    path: path.into(),
                    frames,
                    fps: 10.0,
                    looping: true,
                },
            )]),
        }
    }

    fn manifest(sprite_def: SpriteDef) -> AnimationManifest {
        AnimationManifest {
            version: MANIFEST_VERSION,
            sprites: BTreeMap::from([("blob".to_string(), sprite_def)]),
        }
    }

    #[test]
    fn frame_path_and_duration() {
        let anim = &sprite("enemies/blob", 30).animations["idle"];
        assert_eq!(anim.frame_path(7), "enemies/blob/0007.png");
        assert_eq!(anim.duration_secs(), 3.0);
    }

    #[test]
    fn valid_manifest_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            fake_png(&dir.path().join(format!("enemies/blob/{i:04}.png")), 8, 4);
        }
        let m = manifest(sprite("enemies/blob", 3));
        assert_eq!(m.validate(dir.path()), vec![]);
        assert_eq!(
            png_size(&dir.path().join("enemies/blob/0000.png")).unwrap(),
            (8, 4)
        );
    }

    #[test]
    fn frame_issues() {
        let dir = tempfile::tempdir().unwrap();
        let p = |i: u32| dir.path().join(format!("blob/{i:04}.png"));
        fake_png(&p(0), 8, 4);
        fake_png(&p(1), 9, 4);
        std::fs::write(p(2), b"definitely not a png, but long enough").unwrap();
        // 0003 missing, 0004 exists beyond the declared count of 4.
        fake_png(&p(4), 8, 4);
        let issues = manifest(sprite("blob", 4)).validate(dir.path());
        assert_eq!(issues.len(), 4, "{issues:#?}");
        assert!(matches!(
            &issues[0],
            ManifestIssue::WrongFrameSize { found_w: 9, .. }
        ));
        assert!(
            matches!(&issues[1], ManifestIssue::UnreadableFrame { path, .. } if path == "blob/0002.png")
        );
        assert_eq!(
            issues[2],
            ManifestIssue::MissingFrame {
                path: "blob/0003.png".into()
            }
        );
        assert!(
            matches!(&issues[3], ManifestIssue::ExtraFrame { path, .. } if path == "blob/0004.png")
        );
    }

    #[test]
    fn value_issues() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = sprite("../outside", 0);
        s.pixels_per_tile = 0.0;
        s.pivot.x = 9;
        s.hitbox = Some(PixelRect {
            x: 6,
            y: 0,
            w: 4,
            h: 4,
        });
        s.animations.get_mut("idle").unwrap().fps = f32::NAN;
        let mut m = manifest(s);
        m.version = 2;
        let codes: Vec<String> = m
            .validate(dir.path())
            .iter()
            .map(|i| {
                serde_json::to_value(i).unwrap()["code"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            codes,
            [
                "unsupported_version",
                "invalid_pixels_per_tile",
                "pivot_outside_frame",
                "invalid_hitbox",
                "no_frame",
                "invalid_fps",
                "invalid_path",
            ]
        );
    }

    #[test]
    fn empty_sprite_issues() {
        let mut s = sprite("blob", 1);
        s.frame_size = PixelSize { w: 0, h: 0 };
        s.pivot = PixelPoint { x: 0, y: 0 };
        s.hitbox = None;
        s.animations.clear();
        assert_eq!(
            manifest(s).validate("."),
            vec![
                ManifestIssue::EmptyFrameSize {
                    sprite: "blob".into()
                },
                ManifestIssue::NoAnimation {
                    sprite: "blob".into()
                },
            ]
        );
    }

    #[test]
    fn unsafe_paths_are_rejected() {
        for bad in ["", "/abs", "a/../b", "a//b", "a\\b", "./a"] {
            assert!(!is_safe_relative_path(bad), "{bad:?}");
        }
        assert!(is_safe_relative_path("characters/blue_wizard/idle"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let err = AnimationManifest::from_json(r#"{ "version": 1, "sprites": {}, "extra": 1 }"#)
            .unwrap_err();
        assert!(err.to_string().contains("unknown field `extra`"), "{err}");
    }
}
