//! Tileset atlas (`assets/tilesets/<name>/atlas.json`): how the painted sheets are cut into
//! named pieces usable in a level grid, and how terrain is auto-tiled from its neighbors.
//!
//! Format documented in `docs/ASSETS.md`. Pixel rects are in sheet space (origin top-left,
//! y down); sizes in tiles are footprints on the level grid.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::assets::{png_size, PixelRect, PixelSize};

/// Atlas file name inside a tileset directory.
pub const ATLAS_FILE: &str = "atlas.json";

/// Atlas format version understood by this build.
pub const ATLAS_VERSION: u32 = 1;

/// The whole atlas of one tileset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TilesetAtlas {
    pub version: u32,
    /// Sheet pixels per level tile, for every piece. Render scale = `tile_size / pixels_per_tile`.
    pub pixels_per_tile: u32,
    /// Source images keyed by id.
    pub sheets: BTreeMap<String, SheetDef>,
    /// Pieces keyed by id. Level legends refer to these ids or to an autotiled terrain id.
    pub pieces: BTreeMap<String, PieceDef>,
}

/// One source image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SheetDef {
    /// PNG file, relative to the atlas directory.
    pub file: String,
    /// Expected image size, checked against the file.
    pub size: PixelSize,
}

/// A rectangle of a sheet with its gameplay role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PieceDef {
    pub sheet: String,
    /// Source rectangle in the sheet. Drawn size in tiles = `rect / pixels_per_tile`.
    pub rect: PixelRect,
    pub kind: PieceKind,
    /// Footprint on the level grid, in whole tiles. The drawing may overhang it.
    pub size: TileSize,
    /// Whether the footprint is solid.
    pub collision: bool,
    /// How the drawn rect is aligned on its footprint.
    pub anchor: Anchor,
    /// Auto-tiling rule; present exactly for [`PieceKind::Terrain`] pieces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autotile: Option<Autotile>,
}

/// Gameplay role of a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PieceKind {
    /// One grid cell of an auto-tiled terrain.
    Terrain,
    /// Free sprite you can stand on.
    Platform,
    /// Visual only, drawn in the level plane.
    Decor,
    /// Hurts on contact.
    Hazard,
    /// Background silhouette for parallax layers.
    Parallax,
}

/// Alignment of a piece's drawn rect on its footprint (always centered horizontally).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// Rect centered on the footprint (terrain, floating platforms).
    Center,
    /// Rect bottom on the footprint bottom (things standing on the ground).
    Bottom,
    /// Rect top on the footprint top (things hanging from a ceiling).
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileSize {
    pub w: u32,
    pub h: u32,
}

/// Which grid configuration a terrain piece is drawn for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Autotile {
    /// Terrain family (e.g. `mossy_ground`), the id a level legend uses.
    pub terrain: String,
    /// 3x3 neighborhood this piece is drawn for, centered on the cell itself.
    pub neighbors: NeighborPattern,
}

/// One cell of a [`NeighborPattern`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// `#`: must be the same terrain.
    Solid,
    /// `.`: must not be the same terrain.
    Empty,
    /// `*`: either.
    Any,
}

/// A 3x3 neighborhood written as `"NW N NE / W C E / SW S SE"` without spaces,
/// e.g. `"*.*/.##/*##"` (top-left corner of a block). `#` solid, `.` empty, `*` any.
/// Rows go top to bottom, like the level's tile rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NeighborPattern([[Cell; 3]; 3]);

/// A neighborhood as found in a level: `grid[row][col]`, row 0 = above, `[1][1]` = the cell.
pub type Neighborhood = [[bool; 3]; 3];

impl NeighborPattern {
    /// Whether `grid` satisfies this pattern.
    pub fn matches(&self, grid: &Neighborhood) -> bool {
        self.0.iter().zip(grid).all(|(pattern_row, grid_row)| {
            pattern_row
                .iter()
                .zip(grid_row)
                .all(|(cell, &solid)| match cell {
                    Cell::Solid => solid,
                    Cell::Empty => !solid,
                    Cell::Any => true,
                })
        })
    }
}

impl TryFrom<String> for NeighborPattern {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let bad = || {
            format!("invalid neighbor pattern `{text}`: expected 3 rows of `#`, `.` or `*` joined by `/`")
        };
        let rows: Vec<&str> = text.split('/').collect();
        if rows.len() != 3 {
            return Err(bad());
        }
        let mut cells = [[Cell::Any; 3]; 3];
        for (row, line) in cells.iter_mut().zip(rows) {
            let chars: Vec<char> = line.chars().collect();
            if chars.len() != 3 {
                return Err(bad());
            }
            for (cell, ch) in row.iter_mut().zip(chars) {
                *cell = match ch {
                    '#' => Cell::Solid,
                    '.' => Cell::Empty,
                    '*' => Cell::Any,
                    _ => return Err(bad()),
                };
            }
        }
        Ok(Self(cells))
    }
}

impl fmt::Display for NeighborPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, row) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            for cell in row {
                f.write_str(match cell {
                    Cell::Solid => "#",
                    Cell::Empty => ".",
                    Cell::Any => "*",
                })?;
            }
        }
        Ok(())
    }
}

impl From<NeighborPattern> for String {
    fn from(pattern: NeighborPattern) -> Self {
        pattern.to_string()
    }
}

/// Failure to read or parse an atlas file.
#[derive(Debug, thiserror::Error)]
pub enum AtlasError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid atlas JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// A problem found by [`TilesetAtlas::validate`].
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum AtlasIssue {
    #[error("atlas version {found} is not supported (expected {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("pixels_per_tile must be greater than 0")]
    InvalidPixelsPerTile,
    #[error("sheet `{sheet}`: file `{file}` must be a plain file name or relative path")]
    InvalidSheetPath { sheet: String, file: String },
    #[error("sheet `{sheet}`: cannot read `{file}`: {reason}")]
    UnreadableSheet {
        sheet: String,
        file: String,
        reason: String,
    },
    #[error("sheet `{sheet}` is {found_w}x{found_h}, declared {declared_w}x{declared_h}")]
    WrongSheetSize {
        sheet: String,
        declared_w: u32,
        declared_h: u32,
        found_w: u32,
        found_h: u32,
    },
    #[error("piece `{piece}` uses unknown sheet `{sheet}`")]
    UnknownSheet { piece: String, sheet: String },
    #[error("piece `{piece}`: rect is empty or outside its sheet")]
    RectOutOfBounds { piece: String },
    #[error("piece `{piece}`: footprint size must be at least 1x1")]
    EmptyFootprint { piece: String },
    #[error("piece `{piece}`: terrain pieces need `autotile`, other kinds must not have it")]
    AutotileMismatch { piece: String },
}

impl TilesetAtlas {
    /// Parses atlas JSON. Does not validate (see [`TilesetAtlas::validate`]).
    pub fn from_json(json: &str) -> Result<Self, AtlasError> {
        Ok(serde_json::from_str(json)?)
    }

    /// Reads `<dir>/atlas.json`.
    pub fn load(dir: impl AsRef<Path>) -> Result<Self, AtlasError> {
        let path = dir.as_ref().join(ATLAS_FILE);
        let json = std::fs::read_to_string(&path).map_err(|source| AtlasError::Io {
            path: path.clone(),
            source,
        })?;
        Self::from_json(&json)
    }

    /// Terrain pieces of `terrain` drawn for the neighborhood `grid`, in id order.
    /// Several results are visual variants; pick one deterministically (e.g. by position).
    ///
    /// The results borrow only the atlas, not the arguments, so they can outlive a
    /// neighborhood computed on the fly.
    pub fn autotile_candidates<'a>(
        &'a self,
        terrain: &str,
        grid: &Neighborhood,
    ) -> impl Iterator<Item = (&'a str, &'a PieceDef)> + use<'a> {
        let (terrain, grid) = (terrain.to_owned(), *grid);
        self.pieces.iter().filter_map(move |(id, piece)| {
            let rule = piece.autotile.as_ref()?;
            (rule.terrain == terrain && rule.neighbors.matches(&grid))
                .then_some((id.as_str(), piece))
        })
    }

    /// Checks values, references and that every sheet exists under `dir` with its declared
    /// size and contains every rect. Returns every issue found.
    pub fn validate(&self, dir: impl AsRef<Path>) -> Vec<AtlasIssue> {
        let dir = dir.as_ref();
        let mut issues = Vec::new();
        if self.version != ATLAS_VERSION {
            issues.push(AtlasIssue::UnsupportedVersion {
                found: self.version,
                expected: ATLAS_VERSION,
            });
        }
        if self.pixels_per_tile == 0 {
            issues.push(AtlasIssue::InvalidPixelsPerTile);
        }
        for (id, sheet) in &self.sheets {
            check_sheet(dir, id, sheet, &mut issues);
        }
        for (id, piece) in &self.pieces {
            let piece_name = || id.clone();
            match self.sheets.get(&piece.sheet) {
                None => issues.push(AtlasIssue::UnknownSheet {
                    piece: piece_name(),
                    sheet: piece.sheet.clone(),
                }),
                Some(sheet) if !rect_fits(piece.rect, sheet.size) => {
                    issues.push(AtlasIssue::RectOutOfBounds {
                        piece: piece_name(),
                    })
                }
                Some(_) => {}
            }
            if piece.size.w == 0 || piece.size.h == 0 {
                issues.push(AtlasIssue::EmptyFootprint {
                    piece: piece_name(),
                });
            }
            if (piece.kind == PieceKind::Terrain) != piece.autotile.is_some() {
                issues.push(AtlasIssue::AutotileMismatch {
                    piece: piece_name(),
                });
            }
        }
        issues
    }
}

fn rect_fits(r: PixelRect, size: PixelSize) -> bool {
    r.w > 0
        && r.h > 0
        && r.x.checked_add(r.w).is_some_and(|right| right <= size.w)
        && r.y.checked_add(r.h).is_some_and(|bottom| bottom <= size.h)
}

fn check_sheet(dir: &Path, id: &str, sheet: &SheetDef, issues: &mut Vec<AtlasIssue>) {
    let safe = !sheet.file.is_empty()
        && !sheet.file.starts_with('/')
        && !sheet.file.contains('\\')
        && sheet
            .file
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if !safe {
        issues.push(AtlasIssue::InvalidSheetPath {
            sheet: id.into(),
            file: sheet.file.clone(),
        });
        return;
    }
    match png_size(&dir.join(&sheet.file)) {
        Err(e) => issues.push(AtlasIssue::UnreadableSheet {
            sheet: id.into(),
            file: sheet.file.clone(),
            reason: e.to_string(),
        }),
        Ok((w, h)) if (w, h) != (sheet.size.w, sheet.size.h) => {
            issues.push(AtlasIssue::WrongSheetSize {
                sheet: id.into(),
                declared_w: sheet.size.w,
                declared_h: sheet.size.h,
                found_w: w,
                found_h: h,
            })
        }
        Ok(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(s: &str) -> NeighborPattern {
        NeighborPattern::try_from(s.to_string()).unwrap()
    }

    fn grid(rows: [&str; 3]) -> Neighborhood {
        rows.map(|r| {
            let b = r.as_bytes();
            [b[0] == b'#', b[1] == b'#', b[2] == b'#']
        })
    }

    #[test]
    fn pattern_roundtrips_through_text() {
        let p = pattern("*.*/.##/*##");
        assert_eq!(p.to_string(), "*.*/.##/*##");
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"*.*/.##/*##\"");
        let back: NeighborPattern = serde_json::from_str("\"*.*/.##/*##\"").unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn malformed_patterns_are_rejected() {
        for bad in [
            "",
            "###/###",
            "###/###/##",
            "###/###/####",
            "###/#x#/###",
            "###/###/###/",
        ] {
            assert!(
                NeighborPattern::try_from(bad.to_string()).is_err(),
                "{bad:?}"
            );
        }
        let err = serde_json::from_str::<NeighborPattern>("\"abc\"").unwrap_err();
        assert!(
            err.to_string().contains("invalid neighbor pattern"),
            "{err}"
        );
    }

    #[test]
    fn pattern_matching() {
        let top_left = pattern("*.*/.##/*##");
        assert!(top_left.matches(&grid(["...", ".##", ".##"])));
        assert!(
            top_left.matches(&grid(["#.#", ".##", "###"])),
            "`*` ignores corners"
        );
        assert!(
            !top_left.matches(&grid(["...", ".##", ".#."])),
            "SE must be solid"
        );
        assert!(
            !top_left.matches(&grid([".#.", ".##", ".##"])),
            "N must be empty"
        );
    }

    fn atlas_with(piece: PieceDef) -> TilesetAtlas {
        TilesetAtlas {
            version: ATLAS_VERSION,
            pixels_per_tile: 4,
            sheets: BTreeMap::from([(
                "sheet".to_string(),
                SheetDef {
                    file: "sheet.png".into(),
                    size: PixelSize { w: 8, h: 8 },
                },
            )]),
            pieces: BTreeMap::from([("p".to_string(), piece)]),
        }
    }

    fn terrain_piece(neighbors: &str) -> PieceDef {
        PieceDef {
            sheet: "sheet".into(),
            rect: PixelRect {
                x: 0,
                y: 0,
                w: 4,
                h: 4,
            },
            kind: PieceKind::Terrain,
            size: TileSize { w: 1, h: 1 },
            collision: true,
            anchor: Anchor::Center,
            autotile: Some(Autotile {
                terrain: "ground".into(),
                neighbors: pattern(neighbors),
            }),
        }
    }

    fn write_png_header(path: &Path, w: u32, h: u32) {
        let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
        bytes.extend(13u32.to_be_bytes());
        bytes.extend(b"IHDR");
        bytes.extend(w.to_be_bytes());
        bytes.extend(h.to_be_bytes());
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn valid_atlas_has_no_issue() {
        let dir = tempfile::tempdir().unwrap();
        write_png_header(&dir.path().join("sheet.png"), 8, 8);
        assert_eq!(
            atlas_with(terrain_piece("###/###/###")).validate(dir.path()),
            vec![]
        );
    }

    #[test]
    fn piece_and_sheet_issues() {
        let dir = tempfile::tempdir().unwrap();
        write_png_header(&dir.path().join("sheet.png"), 8, 9);
        let mut piece = terrain_piece("###/###/###");
        piece.rect.x = 6; // 6 + 4 > 8
        piece.size.h = 0;
        piece.kind = PieceKind::Decor; // but still has an autotile rule
        let mut atlas = atlas_with(piece.clone());
        atlas.pixels_per_tile = 0;
        piece.sheet = "nope".into();
        atlas.pieces.insert("q".into(), piece);
        atlas.sheets.insert(
            "evil".into(),
            SheetDef {
                file: "../evil.png".into(),
                size: PixelSize { w: 1, h: 1 },
            },
        );
        atlas.sheets.insert(
            "lost".into(),
            SheetDef {
                file: "lost.png".into(),
                size: PixelSize { w: 1, h: 1 },
            },
        );
        let codes: Vec<String> = atlas
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
                "invalid_pixels_per_tile",
                "invalid_sheet_path",
                "unreadable_sheet",
                "wrong_sheet_size",
                "rect_out_of_bounds",
                "empty_footprint",
                "autotile_mismatch",
                "unknown_sheet",
                "empty_footprint",
                "autotile_mismatch",
            ]
        );
    }

    #[test]
    fn autotile_candidates_filter_by_terrain_and_pattern() {
        let mut atlas = atlas_with(terrain_piece("*.*/.##/*##"));
        atlas
            .pieces
            .insert("full".into(), terrain_piece("###/###/###"));
        let g = grid(["...", ".##", ".##"]);
        let ids: Vec<&str> = atlas
            .autotile_candidates("ground", &g)
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, ["p"]);
        assert_eq!(atlas.autotile_candidates("water", &g).count(), 0);
    }
}
