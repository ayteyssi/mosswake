//! Animated sprites from `assets/manifest.json`: frame loading (on demand, downscaled) and
//! playback driven by render time.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use mw_core::assets::AnimationManifest;

use crate::layout::anchor_from_pivot;
use crate::textures::{scale_for, TextureScales};

/// The animation manifest plus the frame handles loaded so far.
#[derive(Resource)]
pub struct SpriteLibrary {
    manifest: AnimationManifest,
    frames: HashMap<(String, String), Vec<Handle<Image>>>,
}

/// Frames and timing of one animation.
#[derive(Debug, Clone)]
pub struct Clip {
    pub frames: Vec<Handle<Image>>,
    pub fps: f32,
    pub looping: bool,
}

impl SpriteLibrary {
    pub fn new(manifest: AnimationManifest) -> Self {
        Self {
            manifest,
            frames: HashMap::new(),
        }
    }

    pub fn has(&self, sprite: &str) -> bool {
        self.manifest.sprites.contains_key(sprite)
    }

    /// Clip `anim` of `sprite`, loading its frames on first use.
    pub fn clip(
        &mut self,
        server: &AssetServer,
        scales: &mut TextureScales,
        sprite: &str,
        anim: &str,
    ) -> Option<Clip> {
        let def = self.manifest.sprites.get(sprite)?;
        let anim_def = def.animations.get(anim)?;
        let factor = scale_for(def.pixels_per_tile);
        let frames = self
            .frames
            .entry((sprite.to_owned(), anim.to_owned()))
            .or_insert_with(|| {
                (0..anim_def.frames)
                    .map(|i| scales.load(server, anim_def.frame_path(i), factor))
                    .collect()
            })
            .clone();
        Some(Clip {
            frames,
            fps: anim_def.fps,
            looping: anim_def.looping,
        })
    }

    /// A sprite playing `anim` of `sprite`, sized for `tile_size` world units per tile and
    /// anchored on the manifest pivot (feet / plant base).
    pub fn animated(
        &mut self,
        server: &AssetServer,
        scales: &mut TextureScales,
        sprite: &str,
        anim: &str,
        tile_size: f32,
        phase: f32,
    ) -> Option<(Sprite, Anchor, Animator)> {
        let def = self.manifest.sprites.get(sprite)?.clone();
        let clip = self.clip(server, scales, sprite, anim)?;
        let world_per_px = tile_size / def.pixels_per_tile;
        let size = Vec2::new(def.frame_size.w as f32, def.frame_size.h as f32) * world_per_px;
        let sprite_component = Sprite {
            image: clip.frames[0].clone(),
            custom_size: Some(size),
            ..default()
        };
        let anchor = Anchor(anchor_from_pivot(def.pivot, def.frame_size));
        let animator = Animator {
            sprite: sprite.to_owned(),
            anim: anim.to_owned(),
            clip,
            elapsed: phase,
        };
        Some((sprite_component, anchor, animator))
    }
}

/// Plays a [`Clip`] on the entity's [`Sprite`].
#[derive(Component, Debug, Clone)]
pub struct Animator {
    pub sprite: String,
    pub anim: String,
    clip: Clip,
    elapsed: f32,
}

impl Animator {
    /// Switches to `anim` (restarting it) unless it is already playing.
    pub fn play(
        &mut self,
        library: &mut SpriteLibrary,
        server: &AssetServer,
        scales: &mut TextureScales,
        anim: &str,
    ) {
        if self.anim == anim {
            return;
        }
        if let Some(clip) = library.clip(server, scales, &self.sprite, anim) {
            self.anim = anim.to_owned();
            self.clip = clip;
            self.elapsed = 0.0;
        }
    }

    /// Frame index at the current time.
    pub fn frame(&self) -> usize {
        frame_at(
            self.elapsed,
            self.clip.fps,
            self.clip.frames.len(),
            self.clip.looping,
        )
    }
}

/// Frame shown `elapsed` seconds into a clip of `count` frames.
pub fn frame_at(elapsed: f32, fps: f32, count: usize, looping: bool) -> usize {
    if count == 0 {
        return 0;
    }
    let index = (elapsed * fps).max(0.0) as usize;
    if looping {
        index % count
    } else {
        index.min(count - 1)
    }
}

pub(crate) fn animate(time: Res<Time>, mut query: Query<(&mut Animator, &mut Sprite)>) {
    for (mut animator, mut sprite) in &mut query {
        animator.elapsed += time.delta_secs();
        let frame = &animator.clip.frames[animator.frame()];
        if sprite.image != *frame {
            sprite.image = frame.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_loop_or_hold() {
        assert_eq!(frame_at(0.0, 10.0, 4, true), 0);
        assert_eq!(frame_at(0.35, 10.0, 4, true), 3);
        assert_eq!(frame_at(0.45, 10.0, 4, true), 0, "wraps");
        assert_eq!(frame_at(9.0, 10.0, 4, false), 3, "holds the last frame");
        assert_eq!(frame_at(1.0, 10.0, 0, true), 0);
    }
}
