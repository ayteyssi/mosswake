//! Pure layout decisions (no Bevy world, no GPU): which atlas piece each cell shows, where
//! decorative plants grow, how sprites anchor, where the camera may go. Unit-tested.

use bevy::math::{Rect, Vec2};
use mw_core::assets::{PixelPoint, PixelSize};
use mw_core::tileset::{Anchor as PieceAnchor, Neighborhood, PieceDef, PieceKind, TilesetAtlas};
use mw_level::{Layer, Level, EMPTY_TILE};

/// Deterministic hash of a cell, for visual variety that is stable between runs and clients.
pub fn cell_hash(x: i32, y: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1)
        ^ (y as u32).wrapping_mul(0x85EB_CA77)
        ^ salt.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^ (h >> 15)
}

/// Sprite anchor (Bevy convention: `(-0.5, -0.5)` bottom-left, `(0.5, 0.5)` top-right)
/// placing pixel `pivot` (origin top-left, y down) of a `size` frame at the entity position.
pub fn anchor_from_pivot(pivot: PixelPoint, size: PixelSize) -> Vec2 {
    Vec2::new(
        pivot.x as f32 / size.w as f32 - 0.5,
        0.5 - pivot.y as f32 / size.h as f32,
    )
}

/// How a level cell is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum CellVisual<'a> {
    /// A piece of the atlas (terrain cell or per-tile piece).
    Piece { id: &'a str, piece: &'a PieceDef },
    /// The legend id is unknown to the atlas: draw a visible placeholder.
    Missing { id: &'a str },
}

/// Terrain families of the atlas (ids a legend can use for auto-tiling).
pub fn terrains(atlas: &TilesetAtlas) -> Vec<&str> {
    let mut ids: Vec<&str> = atlas
        .pieces
        .values()
        .filter_map(|p| p.autotile.as_ref().map(|a| a.terrain.as_str()))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Visual of cell `(x, y)` (y up) of `layer`, or `None` if the cell is empty.
///
/// Auto-tiled terrains look at the 8 neighbors of the same legend id; cells outside the level
/// count as filled so that borders do not show moss facing the void.
pub fn cell_visual<'a>(
    atlas: &'a TilesetAtlas,
    level: &'a Level,
    layer: &Layer,
    x: u32,
    y: u32,
) -> Option<CellVisual<'a>> {
    let ch = layer.tile_at(x, y)?;
    let id = level.legend.get(&ch)?.as_str();
    if terrains(atlas).contains(&id) {
        let same = |dx: i32, dy: i32| {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx >= level.size.w as i32 || ny >= level.size.h as i32 {
                return true;
            }
            layer
                .tile_at(nx as u32, ny as u32)
                .and_then(|c| level.legend.get(&c))
                .is_some_and(|other| other == id)
        };
        // Row 0 is above the cell (y + 1), as in tile rows.
        let grid: Neighborhood = [-1, 0, 1].map(|dy| [-1, 0, 1].map(|dx| same(dx, -dy)));
        let candidates: Vec<_> = atlas.autotile_candidates(id, &grid).collect();
        return Some(match candidates.as_slice() {
            [] => CellVisual::Missing { id },
            many => {
                let (id, piece) = many[cell_hash(x as i32, y as i32, 1) as usize % many.len()];
                CellVisual::Piece { id, piece }
            }
        });
    }
    Some(match atlas.pieces.get_key_value(id) {
        Some((id, piece)) => CellVisual::Piece { id, piece },
        None => CellVisual::Missing { id },
    })
}

/// Where a piece is drawn when it occupies the cell `(x, y)`: the sprite anchor and its
/// position (world units), and its drawn size. The footprint starts at the cell.
pub fn piece_placement(
    piece: &PieceDef,
    pixels_per_tile: u32,
    tile_size: f32,
    x: u32,
    y: u32,
) -> (Vec2, Vec2, Vec2) {
    let scale = tile_size / pixels_per_tile as f32;
    let size = Vec2::new(piece.rect.w as f32, piece.rect.h as f32) * scale;
    let foot = Vec2::new(piece.size.w as f32, piece.size.h as f32) * tile_size;
    let origin = Vec2::new(x as f32, y as f32) * tile_size;
    let center_x = origin.x + foot.x / 2.0;
    let (anchor, position) = match piece.anchor {
        PieceAnchor::Center => (Vec2::ZERO, Vec2::new(center_x, origin.y + foot.y / 2.0)),
        PieceAnchor::Bottom => (Vec2::new(0.0, -0.5), Vec2::new(center_x, origin.y)),
        PieceAnchor::Top => (Vec2::new(0.0, 0.5), Vec2::new(center_x, origin.y + foot.y)),
    };
    (anchor, position, size)
}

/// Source rect of a piece in a texture downscaled by `texture_scale`, shrunk by half a texel
/// on each side so linear filtering does not bleed neighbouring cells of the sheet.
pub fn piece_texture_rect(piece: &PieceDef, texture_scale: f32) -> Rect {
    let r = piece.rect;
    let min = Vec2::new(r.x as f32, r.y as f32) * texture_scale + 0.5;
    let max = Vec2::new((r.x + r.w) as f32, (r.y + r.h) as f32) * texture_scale - 0.5;
    Rect::from_corners(min, max.max(min))
}

/// Decorative plant spots: top surfaces of solid cells with two free cells above. Returns
/// `(x, y)` of the free cell the plant stands in and a hash to pick kind and phase.
pub fn plant_spots(level: &Level, density_percent: u32) -> Vec<(u32, u32, u32)> {
    let mut spots = Vec::new();
    for y in 0..level.size.h.saturating_sub(2) {
        for x in 0..level.size.w {
            let free = |yy: u32| !level.is_solid(x, yy);
            let hazard_free = level
                .layers
                .iter()
                .filter(|l| l.kind == mw_level::LayerKind::Hazard)
                .all(|l| l.tile_at(x, y + 1).is_none());
            if level.is_solid(x, y) && free(y + 1) && free(y + 2) && hazard_free {
                let h = cell_hash(x as i32, y as i32, 7);
                if h % 100 < density_percent {
                    spots.push((x, y + 1, h));
                }
            }
        }
    }
    spots
}

/// Camera center clamped so the view stays inside `[0, level]` on each axis; an axis where
/// the view is larger than the level is centered.
pub fn clamp_camera(target: Vec2, view: Vec2, level: Vec2) -> Vec2 {
    let axis = |t: f32, v: f32, l: f32| {
        if v >= l {
            l / 2.0
        } else {
            t.clamp(v / 2.0, l - v / 2.0)
        }
    };
    Vec2::new(
        axis(target.x, view.x, level.x),
        axis(target.y, view.y, level.y),
    )
}

/// Pieces of one sheet usable as parallax silhouettes, sorted by id.
pub fn sheet_pieces<'a>(atlas: &'a TilesetAtlas, sheet: &str) -> Vec<(&'a str, &'a PieceDef)> {
    atlas
        .pieces
        .iter()
        .filter(|(_, p)| p.sheet == sheet && p.kind != PieceKind::Terrain)
        .map(|(id, p)| (id.as_str(), p))
        .collect()
}

/// Whether a legend character is drawn at all (`.` is empty).
pub fn is_drawn(ch: char) -> bool {
    ch != EMPTY_TILE
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_level::{LayerKind, Size, Spawn};
    use std::path::PathBuf;

    fn atlas() -> TilesetAtlas {
        TilesetAtlas::load(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/tilesets/mossy"),
        )
        .expect("atlas loads")
    }

    fn level(rows: &[&str]) -> Level {
        let mut level = Level::new(
            "t",
            "T",
            Size {
                w: rows[0].len() as u32,
                h: rows.len() as u32,
            },
        );
        level.legend.insert('#', "mossy_ground".into());
        level.legend.insert('p', "platform_2x1_a".into());
        level.legend.insert('?', "no_such_piece".into());
        level.layers.push(Layer {
            name: "ground".into(),
            kind: LayerKind::Solid,
            tiles: rows.iter().map(|r| r.to_string()).collect(),
        });
        level.spawns.push(Spawn { x: 0.5, y: 0.0 });
        level
    }

    fn piece_id(atlas: &TilesetAtlas, level: &Level, x: u32, y: u32) -> String {
        match cell_visual(atlas, level, &level.layers[0], x, y) {
            Some(CellVisual::Piece { id, .. }) => id.to_string(),
            other => panic!("({x},{y}): {other:?}"),
        }
    }

    #[test]
    fn terrain_cells_pick_the_matching_autotile_piece() {
        let atlas = atlas();
        // A 3-wide, 1-tall strip in open air.
        let level = level(&["....", ".###", "....", "...."]);
        let neighbors = |x| {
            let id = piece_id(&atlas, &level, x, 2);
            atlas.pieces[&id]
                .autotile
                .as_ref()
                .unwrap()
                .neighbors
                .to_string()
        };
        assert_eq!(neighbors(1), "*.*/.##/*.*", "left cap");
        assert_eq!(neighbors(2), "*.*/###/*.*", "middle");
        // Rightmost cell touches the level border, which counts as filled.
        assert_eq!(neighbors(3), "*.*/###/*.*");
    }

    #[test]
    fn non_terrain_ids_and_unknown_ids() {
        let atlas = atlas();
        let level = level(&["p?", ".."]);
        assert_eq!(piece_id(&atlas, &level, 0, 1), "platform_2x1_a");
        assert_eq!(
            cell_visual(&atlas, &level, &level.layers[0], 1, 1),
            Some(CellVisual::Missing {
                id: "no_such_piece"
            })
        );
        assert_eq!(cell_visual(&atlas, &level, &level.layers[0], 0, 0), None);
    }

    #[test]
    fn every_cell_of_the_tutorial_ground_resolves() {
        let atlas = atlas();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../levels/tutorial");
        let level = mw_level::LevelBundle::load(dir).unwrap().level;
        let ground = level.layers.iter().find(|l| l.name == "ground").unwrap();
        for y in 0..level.size.h {
            for x in 0..level.size.w {
                if ground.tile_at(x, y) == Some('#') {
                    assert!(
                        matches!(
                            cell_visual(&atlas, &level, ground, x, y),
                            Some(CellVisual::Piece { .. })
                        ),
                        "({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn anchors_from_pivots() {
        let size = PixelSize { w: 512, h: 512 };
        assert_eq!(
            anchor_from_pivot(PixelPoint { x: 256, y: 512 }, size),
            Vec2::new(0.0, -0.5),
            "bottom-center"
        );
        assert_eq!(
            anchor_from_pivot(PixelPoint { x: 0, y: 0 }, size),
            Vec2::new(-0.5, 0.5),
            "top-left"
        );
    }

    #[test]
    fn piece_placement_follows_anchor_and_footprint() {
        let atlas = atlas();
        let rock = &atlas.pieces["rock_a"];
        let (anchor, pos, size) = piece_placement(rock, 512, 32.0, 3, 2);
        assert_eq!(anchor, Vec2::new(0.0, -0.5));
        assert_eq!(pos.y, 64.0, "standing on the cell bottom");
        assert_eq!(pos.x, 3.0 * 32.0 + rock.size.w as f32 * 16.0);
        assert_eq!(
            size,
            Vec2::new(rock.rect.w as f32, rock.rect.h as f32) / 16.0
        );
    }

    #[test]
    fn texture_rect_is_scaled_and_inset() {
        let atlas = atlas();
        let cell = &atlas.pieces["mossy_ground_0_0"];
        let r = piece_texture_rect(cell, 0.125);
        assert_eq!(r.min, Vec2::splat(0.5));
        assert_eq!(r.max, Vec2::splat(63.5));
    }

    #[test]
    fn plants_grow_on_free_top_surfaces_only() {
        let mut l = level(&["....", "....", "....", "####", "#..#"]);
        l.layers.push(Layer {
            name: "spikes".into(),
            kind: LayerKind::Hazard,
            tiles: vec![
                "....".into(),
                "....".into(),
                "...^".into(),
                "....".into(),
                "....".into(),
            ],
        });
        l.legend.insert('^', "thorns_small_long".into());
        let spots = plant_spots(&l, 100);
        let cells: Vec<_> = spots.iter().map(|&(x, y, _)| (x, y)).collect();
        // Row y = 1 (the `####` row) has free space above; x = 3 is under a hazard.
        assert_eq!(cells, [(0, 2), (1, 2), (2, 2)]);
        assert_eq!(plant_spots(&l, 0), []);
        assert_eq!(spots, plant_spots(&l, 100), "deterministic");
    }

    #[test]
    fn camera_stays_inside_the_level() {
        let view = Vec2::new(400.0, 200.0);
        let level = Vec2::new(1000.0, 600.0);
        assert_eq!(
            clamp_camera(Vec2::new(10.0, 10.0), view, level),
            Vec2::new(200.0, 100.0)
        );
        assert_eq!(
            clamp_camera(Vec2::new(990.0, 300.0), view, level),
            Vec2::new(800.0, 300.0)
        );
        assert_eq!(
            clamp_camera(Vec2::new(10.0, 10.0), Vec2::new(2000.0, 200.0), level),
            Vec2::new(500.0, 100.0),
            "view wider than the level: centered"
        );
    }

    #[test]
    fn cell_hash_is_stable_and_spread() {
        assert_eq!(cell_hash(3, 4, 1), cell_hash(3, 4, 1));
        assert_ne!(cell_hash(3, 4, 1), cell_hash(4, 3, 1));
        let buckets = (0..100)
            .map(|i| cell_hash(i, 0, 7) % 10)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(buckets.len() >= 8, "poorly spread: {buckets:?}");
    }
}
