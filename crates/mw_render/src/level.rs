//! Visuals of the loaded level: tiles (auto-tiled from the atlas), placed entities, decorative
//! plants and parallax silhouettes. Rebuilt whenever [`LoadedLevel`] changes.

use std::collections::BTreeMap;

use bevy::prelude::*;
use bevy::sprite::Anchor;
use mw_core::tileset::TilesetAtlas;
use mw_sim::{tiles_to_world, LoadedLevel};

use crate::layout::{cell_hash, CellVisual};
use crate::layout::{cell_visual, piece_placement, piece_texture_rect, plant_spots, sheet_pieces};
use crate::sprites::SpriteLibrary;
use crate::textures::{scale_for, TextureScales};

/// Draw order (z) of each family of visuals.
pub mod z {
    pub const PARALLAX_FAR: f32 = -30.0;
    pub const PARALLAX_NEAR: f32 = -20.0;
    /// Layer `i` is drawn at `LAYERS + 0.1 * i`.
    pub const LAYERS: f32 = 0.0;
    pub const PLANTS: f32 = 1.0;
    pub const ENTITIES: f32 = 5.0;
    pub const PLAYERS: f32 = 10.0;
}

/// Extra size (world units) given to terrain cells so neighbours overlap and hide seams.
pub const TILE_OVERLAP: f32 = 1.0;

/// Animated decorative plants scattered on free ground (client-side only, no gameplay).
pub const PLANTS: &[&str] = &["plant_1", "plant_2", "plant_4", "plant_6", "blue_flower_1"];

/// Percentage of eligible ground cells that grow a plant.
pub const PLANT_DENSITY: u32 = 22;

/// The tileset atlas and its sheet textures.
#[derive(Resource)]
pub struct Tileset {
    pub atlas: TilesetAtlas,
    /// Asset path of the atlas directory, relative to the assets root.
    pub dir: String,
    sheets: BTreeMap<String, Handle<Image>>,
}

impl Tileset {
    pub fn new(atlas: TilesetAtlas, dir: impl Into<String>) -> Self {
        Self {
            atlas,
            dir: dir.into(),
            sheets: BTreeMap::new(),
        }
    }

    fn texture_scale(&self) -> f32 {
        scale_for(self.atlas.pixels_per_tile as f32)
    }

    fn sheet(
        &mut self,
        server: &AssetServer,
        scales: &mut TextureScales,
        sheet: &str,
    ) -> Option<Handle<Image>> {
        let def = self.atlas.sheets.get(sheet)?;
        let path = format!("{}/{}", self.dir, def.file);
        let factor = self.texture_scale();
        Some(
            self.sheets
                .entry(sheet.to_owned())
                .or_insert_with(|| scales.load(server, path, factor))
                .clone(),
        )
    }
}

/// Marks everything spawned for the current level, so a reload can clear it.
#[derive(Component)]
pub struct LevelVisual;

/// A silhouette scrolling at `factor` of the camera speed (0 = world-fixed, 1 = screen-fixed).
#[derive(Component, Debug, Clone, Copy)]
pub struct Parallax {
    pub base: Vec2,
    pub factor: f32,
}

pub(crate) fn rebuild_level_visuals(
    mut commands: Commands,
    level: Res<LoadedLevel>,
    server: Res<AssetServer>,
    mut tileset: ResMut<Tileset>,
    mut library: ResMut<SpriteLibrary>,
    mut scales: ResMut<TextureScales>,
    old: Query<Entity, With<LevelVisual>>,
) {
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let level = &level.0;
    let ts = level.tile_size as f32;
    let tex_scale = tileset.texture_scale();
    let ppt = tileset.atlas.pixels_per_tile;

    // Tiles.
    let mut warned = Vec::new();
    for (index, layer) in level.layers.iter().enumerate() {
        let z = z::LAYERS + 0.1 * index as f32;
        for y in 0..level.size.h {
            for x in 0..level.size.w {
                match cell_visual(&tileset.atlas, level, layer, x, y) {
                    None => {}
                    Some(CellVisual::Piece { piece, .. }) => {
                        let piece = piece.clone();
                        let Some(image) = tileset.sheet(&server, &mut scales, &piece.sheet) else {
                            continue;
                        };
                        // Per-cell pieces fill their cell; terrain pieces are exactly one cell.
                        let (anchor, pos, mut size) = piece_placement(&piece, ppt, ts, x, y);
                        if piece.autotile.is_some() {
                            // Overlap neighbours slightly: abutting quads at sub-pixel camera
                            // positions leave 1 px seams otherwise.
                            size += TILE_OVERLAP;
                        }
                        commands.spawn((
                            LevelVisual,
                            Sprite {
                                image,
                                rect: Some(piece_texture_rect(&piece, tex_scale)),
                                custom_size: Some(size),
                                ..default()
                            },
                            Anchor(anchor),
                            Transform::from_translation(pos.extend(z)),
                        ));
                    }
                    Some(CellVisual::Missing { id }) => {
                        if !warned.contains(&id.to_owned()) {
                            warn!("legend id `{id}` is not in the tileset atlas (drawn as a placeholder, see #32)");
                            warned.push(id.to_owned());
                        }
                        commands.spawn((
                            LevelVisual,
                            Sprite::from_color(Color::srgba(0.8, 0.2, 0.7, 0.6), Vec2::splat(ts)),
                            Transform::from_translation(
                                ((Vec2::new(x as f32, y as f32) + 0.5) * ts).extend(z),
                            ),
                        ));
                    }
                }
            }
        }
    }

    // Placed entities (slimes…): drawn where the level puts them until the simulation
    // spawns and moves them (#8).
    for entity in &level.entities {
        if let Some(bundle) = library.animated(&server, &mut scales, &entity.kind, "idle", ts, 0.0)
        {
            let pos = tiles_to_world(level, entity.x, entity.y);
            commands.spawn((
                LevelVisual,
                bundle,
                Transform::from_translation(pos.extend(z::ENTITIES)),
            ));
        }
    }

    // Decorative plants.
    for (x, y, hash) in plant_spots(level, PLANT_DENSITY) {
        let kind = PLANTS[(hash / 100) as usize % PLANTS.len()];
        let phase = (hash % 997) as f32 / 997.0 * 3.0;
        if let Some((mut sprite, anchor, animator)) =
            library.animated(&server, &mut scales, kind, "idle", ts, phase)
        {
            sprite.flip_x = hash & 1 == 1;
            let pos = Vec2::new(x as f32 + 0.5, y as f32) * ts;
            let z = z::PLANTS + (hash % 100) as f32 / 1000.0;
            commands.spawn((
                LevelVisual,
                sprite,
                anchor,
                animator,
                Transform::from_translation(pos.extend(z)),
            ));
        }
    }

    // Parallax: far silhouettes, then nearer hills, spread over three level widths.
    let level_size = Vec2::new(level.size.w as f32, level.size.h as f32) * ts;
    let center = level_size / 2.0;
    // Silhouettes are black with mossy rims: translucency blends them into the background
    // (aerial perspective), more so for the far layer.
    for (sheet, factor, tint, enlarge, layer_z, salt) in [
        (
            "background_decoration",
            0.7,
            Color::srgba(0.55, 0.75, 0.70, 0.30),
            2.2,
            z::PARALLAX_FAR,
            11,
        ),
        (
            "mossy_hills",
            0.4,
            Color::srgba(0.70, 0.85, 0.75, 0.55),
            1.4,
            z::PARALLAX_NEAR,
            23,
        ),
    ] {
        let pieces: Vec<_> = sheet_pieces(&tileset.atlas, sheet)
            .into_iter()
            .map(|(_, p)| p.clone())
            .collect();
        let Some(image) = tileset.sheet(&server, &mut scales, sheet) else {
            continue;
        };
        if pieces.is_empty() {
            continue;
        }
        let scale = enlarge * ts / ppt as f32;
        let mut x = -level_size.x;
        let mut i = 0;
        while x < 2.0 * level_size.x {
            let hash = cell_hash(i, 0, salt);
            let piece = &pieces[hash as usize % pieces.len()];
            let size = Vec2::new(piece.rect.w as f32, piece.rect.h as f32) * scale;
            // Where it should be when the camera is at the level center.
            let desired = Vec2::new(x + size.x / 2.0, 0.0);
            commands.spawn((
                LevelVisual,
                Sprite {
                    image: image.clone(),
                    rect: Some(piece_texture_rect(piece, tex_scale)),
                    custom_size: Some(size),
                    color: tint,
                    flip_x: hash & 2 == 2,
                    ..default()
                },
                Anchor::BOTTOM_CENTER,
                Parallax {
                    base: desired - center * factor,
                    factor,
                },
                Transform::from_translation(desired.extend(layer_z + (i % 7) as f32 * 0.01)),
            ));
            x += size.x * (0.55 + (hash % 50) as f32 / 100.0);
            i += 1;
        }
    }
}

pub(crate) fn scroll_parallax(
    camera: Query<&Transform, (With<Camera2d>, Without<Parallax>)>,
    mut layers: Query<(&Parallax, &mut Transform)>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let cam = camera.translation.truncate();
    for (parallax, mut transform) in &mut layers {
        let pos = parallax.base + cam * parallax.factor;
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
    }
}
