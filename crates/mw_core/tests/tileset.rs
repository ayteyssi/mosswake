//! The real `assets/tilesets/mossy/atlas.json` must be valid and auto-tile every configuration.

use std::path::PathBuf;

use mw_core::tileset::{Anchor, Neighborhood, PieceKind, TilesetAtlas};

fn mossy_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/tilesets/mossy")
}

fn atlas() -> TilesetAtlas {
    TilesetAtlas::load(mossy_dir()).expect("atlas loads")
}

/// The 3x3 neighborhood encoded by the 8 low bits of `bits` (center set separately).
fn neighborhood(bits: u8, center: bool) -> Neighborhood {
    let b = |i: u8| bits & (1 << i) != 0;
    [[b(0), b(1), b(2)], [b(3), center, b(4)], [b(5), b(6), b(7)]]
}

#[test]
fn real_atlas_is_valid() {
    let issues = atlas().validate(mossy_dir());
    assert!(issues.is_empty(), "atlas issues: {issues:#?}");
}

#[test]
fn every_sheet_is_used() {
    let atlas = atlas();
    for sheet in atlas.sheets.keys() {
        assert!(
            atlas.pieces.values().any(|p| &p.sheet == sheet),
            "sheet `{sheet}` has no piece"
        );
    }
}

#[test]
fn mossy_ground_autotiles_every_neighborhood_exactly_once() {
    let atlas = atlas();
    for bits in 0..=u8::MAX {
        let grid = neighborhood(bits, true);
        let patterns: Vec<String> = atlas
            .autotile_candidates("mossy_ground", &grid)
            .map(|(_, piece)| {
                piece
                    .autotile
                    .as_ref()
                    .expect("terrain")
                    .neighbors
                    .to_string()
            })
            .collect();
        assert!(!patterns.is_empty(), "no piece for {grid:?}");
        assert!(
            patterns.iter().all(|p| p == &patterns[0]),
            "ambiguous rules for {grid:?}: {patterns:?}"
        );
    }
}

#[test]
fn empty_cells_draw_no_terrain() {
    let atlas = atlas();
    for bits in 0..=u8::MAX {
        let grid = neighborhood(bits, false);
        assert_eq!(atlas.autotile_candidates("mossy_ground", &grid).count(), 0);
    }
}

#[test]
fn piece_kinds_have_consistent_flags() {
    for (id, piece) in &atlas().pieces {
        match piece.kind {
            PieceKind::Terrain | PieceKind::Platform => assert!(piece.collision, "{id}"),
            PieceKind::Decor | PieceKind::Hazard | PieceKind::Parallax => {
                assert!(!piece.collision, "{id}")
            }
        }
    }
}

/// Free pieces are bounding boxes of separate drawings, so they may only touch. Interleaved
/// drawings can make boxes overlap by a few pixels: that is harmless when the shared area is
/// transparent (checked on the pixels when `atlas.json` was generated, see docs/ASSETS.md),
/// but a larger overlap means two drawings were cut into one rect.
/// `thorns_tile` deliberately reuses a crop of `thorns_small_long` (checked separately).
#[test]
fn free_pieces_do_not_overlap() {
    const TOLERANCE_PX: u32 = 8;
    let atlas = atlas();
    let free: Vec<_> = atlas
        .pieces
        .iter()
        .filter(|(_, p)| p.kind != PieceKind::Terrain && p.sheet != "tile_set")
        .collect();
    for (i, (a_id, a)) in free.iter().enumerate() {
        for (b_id, b) in &free[i + 1..] {
            if a_id.as_str() == "thorns_small_long" && b_id.as_str() == "thorns_tile" {
                continue;
            }
            if a.sheet != b.sheet {
                continue;
            }
            let (ra, rb) = (a.rect, b.rect);
            let overlap_w = (ra.x + ra.w)
                .min(rb.x + rb.w)
                .saturating_sub(ra.x.max(rb.x));
            let overlap_h = (ra.y + ra.h)
                .min(rb.y + rb.h)
                .saturating_sub(ra.y.max(rb.y));
            assert!(
                overlap_w.min(overlap_h) <= TOLERANCE_PX,
                "`{a_id}` and `{b_id}` overlap by {overlap_w}x{overlap_h} px"
            );
        }
    }
}

#[test]
fn hazard_tile_fits_one_cell_and_reuses_only_the_small_thorn_drawing() {
    let atlas = atlas();
    let tile = &atlas.pieces["thorns_tile"];
    let source = &atlas.pieces["thorns_small_long"];
    assert_eq!(tile.kind, PieceKind::Hazard);
    assert_eq!((tile.size.w, tile.size.h), (1, 1));
    assert_eq!(tile.anchor, Anchor::Bottom);
    assert!(!tile.collision);
    assert_eq!(tile.sheet, source.sheet);
    assert_eq!(tile.rect.w, atlas.pixels_per_tile);
    assert!(tile.rect.h <= atlas.pixels_per_tile);
    assert!(tile.rect.x >= source.rect.x && tile.rect.y >= source.rect.y);
    assert!(tile.rect.x + tile.rect.w <= source.rect.x + source.rect.w);
    assert!(tile.rect.y + tile.rect.h <= source.rect.y + source.rect.h);
}
