//! Client-side rendering: sprites, animations, tilemap, camera, parallax.
//!
//! Reads the simulation (`LoadedLevel`, `Position`, `PlayerState`, `Facing`) and never writes
//! gameplay state: it only adds its own visual components. See `docs/ARCHITECTURE.md`.
//!
//! - [`ClientRenderPlugin`]: everything visual for a loaded level.
//! - [`LocalInputPlugin`]: keyboard / gamepad → `PlayerInput` of the local player.

use std::path::Path;

use bevy::prelude::*;
use mw_core::assets::{AnimationManifest, ManifestError};
use mw_core::input::PlayerId;
use mw_core::tileset::{AtlasError, TilesetAtlas};
use mw_sim::{LoadedLevel, Rules};

mod actors;
mod camera;
mod input;
pub mod layout;
mod level;
mod sprites;
mod textures;

pub use actors::{player_animation, PrevPosition, PLAYER_SPRITE};
pub use camera::{CameraTarget, VIEW_HEIGHT_TILES};
pub use input::{quantize_axis, LocalInput, LocalInputPlugin, KEY_BINDINGS, PAD_BINDINGS};
pub use level::{LevelVisual, Parallax, Tileset};
pub use sprites::{Animator, SpriteLibrary};
pub use textures::{TextureScales, TARGET_PIXELS_PER_TILE};

/// Tileset used by every level for now (asset path relative to the assets root).
pub const TILESET_DIR: &str = "tilesets/mossy";

/// Background color behind the parallax layers.
pub const CLEAR_COLOR: Color = Color::srgb(0.13, 0.19, 0.21);

/// Why the renderer could not start.
#[derive(Debug, thiserror::Error)]
pub enum RenderSetupError {
    #[error("animation manifest: {0}")]
    Manifest(#[from] ManifestError),
    #[error("tileset atlas: {0}")]
    Atlas(#[from] AtlasError),
}

/// Run condition: a level is loaded and it changed (Bevy's `resource_changed` panics when
/// the resource is missing, which is the case while `join` downloads the level).
fn level_changed(level: Option<Res<LoadedLevel>>) -> bool {
    level.is_some_and(|level| level.is_changed())
}

/// Draws the loaded level and its players, follows `follow` with the camera.
pub struct ClientRenderPlugin {
    manifest: AnimationManifest,
    atlas: TilesetAtlas,
    follow: PlayerId,
}

impl ClientRenderPlugin {
    /// Reads the manifest and the atlas from `assets_root` (the directory Bevy loads from).
    pub fn new(assets_root: &Path, follow: PlayerId) -> Result<Self, RenderSetupError> {
        Ok(Self {
            manifest: AnimationManifest::load(assets_root)?,
            atlas: TilesetAtlas::load(assets_root.join(TILESET_DIR))?,
            follow,
        })
    }
}

impl Plugin for ClientRenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(CLEAR_COLOR))
            .insert_resource(SpriteLibrary::new(self.manifest.clone()))
            .insert_resource(Tileset::new(self.atlas.clone(), TILESET_DIR))
            .insert_resource(CameraTarget(self.follow))
            .init_resource::<TextureScales>()
            .add_systems(FixedPreUpdate, actors::remember_positions)
            .add_systems(
                Update,
                (
                    // On arrival of the first level: at startup in solo play, after the
                    // download when joining a game.
                    camera::spawn_camera.run_if(resource_added::<LoadedLevel>),
                    // `resource_changed` panics on a missing resource: in `join` the
                    // level only arrives after the download.
                    level::rebuild_level_visuals.run_if(level_changed),
                    actors::attach_player_visuals.run_if(resource_exists::<LoadedLevel>),
                    actors::animate_players.run_if(resource_exists::<Rules>),
                    sprites::animate,
                    actors::interpolate,
                    camera::follow.run_if(resource_exists::<LoadedLevel>),
                    level::scroll_parallax,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, textures::downscale_loaded);
    }
}
