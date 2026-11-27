//! Downscale textures as they finish loading, before they reach the GPU.
//!
//! Source art is far larger than it is drawn (a 512 px terrain cell is drawn ~45 px tall).
//! Shrinking each texture to ~[`TARGET_PIXELS_PER_TILE`] cuts GPU memory by up to 16x and
//! avoids the shimmering of a heavily minified texture without mipmaps.

use std::collections::HashMap;

use bevy::prelude::*;
use image::imageops::FilterType;

/// Texture resolution kept per level tile: about twice the on-screen size at 720p.
pub const TARGET_PIXELS_PER_TILE: f32 = 64.0;

/// Factor to apply to art drawn at `pixels_per_tile` (never upscales).
pub fn scale_for(pixels_per_tile: f32) -> f32 {
    (TARGET_PIXELS_PER_TILE / pixels_per_tile).min(1.0)
}

/// Textures waiting to be downscaled, by asset id.
#[derive(Resource, Default)]
pub struct TextureScales {
    pending: HashMap<AssetId<Image>, f32>,
}

impl TextureScales {
    /// Loads `path` and schedules its downscale by `factor`.
    pub fn load(&mut self, server: &AssetServer, path: String, factor: f32) -> Handle<Image> {
        let handle = server.load(path);
        if factor < 0.999 {
            self.pending.entry(handle.id()).or_insert(factor);
        }
        handle
    }

    /// Number of textures not downscaled yet (loading progress).
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

pub(crate) fn downscale_loaded(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    mut scales: ResMut<TextureScales>,
) {
    for event in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id }) = event else {
            continue;
        };
        let Some(factor) = scales.pending.remove(id) else {
            continue;
        };
        let Some(mut image) = images.get_mut(*id) else {
            continue;
        };
        let (w, h) = (image.width(), image.height());
        let size = |v: u32| ((v as f32 * factor).round() as u32).max(1);
        let sampler = image.sampler.clone();
        let usage = image.asset_usage;
        match image.clone().try_into_dynamic() {
            Ok(dynamic) => {
                let small = dynamic.resize_exact(size(w), size(h), FilterType::Triangle);
                let mut resized = Image::from_dynamic(small, true, usage);
                resized.sampler = sampler;
                *image = resized;
            }
            Err(e) => warn!("cannot downscale texture {id:?}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_targets_64_px_per_tile_and_never_upscales() {
        assert_eq!(scale_for(512.0), 0.125);
        assert_eq!(scale_for(128.0), 0.5);
        assert_eq!(scale_for(32.0), 1.0);
    }
}
