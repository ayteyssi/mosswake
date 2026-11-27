//! Tile collision grid and swept AABB movement against it.
//!
//! World space: y up, tile `(i, j)` covers `[i*ts, (i+1)*ts) x [j*ts, (j+1)*ts)` where
//! `ts` is the level's `tile_size`. Outside the grid, columns left and right are solid
//! (invisible walls) and rows below or above are empty (bodies can fall out of the level).

use bevy::math::Vec2;
use bevy::prelude::Resource;
use mw_level::Level;

/// Solid cells and one-way platforms, built from the level layers.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct CollisionGrid {
    width: i32,
    height: i32,
    tile_size: f32,
    /// Row-major, `y = 0` first (bottom row).
    solid: Vec<bool>,
    platform: Vec<bool>,
}

/// An axis-aligned box: `min` inclusive, `max` exclusive, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Vec2,
    pub max: Vec2,
}

impl Aabb {
    /// Box of size `size` whose bottom-center is `feet`.
    pub fn from_feet(feet: Vec2, size: Vec2) -> Self {
        let half_w = size.x / 2.0;
        Self {
            min: Vec2::new(feet.x - half_w, feet.y),
            max: Vec2::new(feet.x + half_w, feet.y + size.y),
        }
    }

    /// Bottom-center point.
    pub fn feet(&self) -> Vec2 {
        Vec2::new((self.min.x + self.max.x) / 2.0, self.min.y)
    }

    fn translated(&self, delta: Vec2) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }
}

/// Result of [`CollisionGrid::sweep`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SweepResult {
    pub aabb: Aabb,
    /// Whether the x / y movement was stopped by a solid cell or supporting platform.
    pub blocked_x: bool,
    pub blocked_y: bool,
}

impl CollisionGrid {
    /// Builds the grid from every solid and platform layer of `level`.
    pub fn from_level(level: &Level) -> Self {
        let (width, height) = (level.size.w as i32, level.size.h as i32);
        let mut solid = Vec::with_capacity((width * height).max(0) as usize);
        let mut platform = Vec::with_capacity(solid.capacity());
        for y in 0..level.size.h {
            for x in 0..level.size.w {
                solid.push(level.is_solid(x, y));
                platform.push(level.is_platform(x, y));
            }
        }
        Self {
            width,
            height,
            tile_size: level.tile_size as f32,
            solid,
            platform,
        }
    }

    pub fn tile_size(&self) -> f32 {
        self.tile_size
    }

    /// Whether cell `(x, y)` blocks movement (see module docs for outside the grid).
    pub fn is_solid(&self, x: i32, y: i32) -> bool {
        if x < 0 || x >= self.width {
            return true;
        }
        if y < 0 || y >= self.height {
            return false;
        }
        self.solid[(y * self.width + x) as usize]
    }

    /// One-way cell, never present outside the grid.
    pub fn is_platform(&self, x: i32, y: i32) -> bool {
        x >= 0
            && x < self.width
            && y >= 0
            && y < self.height
            && self.platform[(y * self.width + x) as usize]
    }

    /// Cell indices covered by the half-open interval `[min, max)` on one axis.
    fn cells(&self, min: f32, max: f32) -> std::ops::RangeInclusive<i32> {
        let first = (min / self.tile_size).floor() as i32;
        let last = (max / self.tile_size).ceil() as i32 - 1;
        first..=last
    }

    /// Whether `aabb` overlaps any solid cell.
    pub fn overlaps(&self, aabb: &Aabb) -> bool {
        let rows = self.cells(aabb.min.y, aabb.max.y);
        self.cells(aabb.min.x, aabb.max.x)
            .any(|x| rows.clone().any(|y| self.is_solid(x, y)))
    }

    /// Moves `aabb` by `delta`, x first then y, stopping flush against the first solid
    /// cell on each axis, with downward gravity for platforms. Every cell crossed is checked,
    /// so no speed can tunnel through.
    /// `aabb` must not already overlap a solid cell.
    pub fn sweep(&self, aabb: Aabb, delta: Vec2) -> SweepResult {
        self.sweep_with_gravity(aabb, delta, -1.0)
    }

    /// Like [`Self::sweep`], with the current gravity direction (-1 down, +1 up).
    /// Platforms block only motion along gravity, from outside their supporting face.
    /// They never block horizontal movement or movement against gravity.
    pub fn sweep_with_gravity(&self, aabb: Aabb, delta: Vec2, gravity_dir: f32) -> SweepResult {
        let (aabb, blocked_x) = self.sweep_x(aabb, delta.x);
        let (aabb, blocked_y) = self.sweep_y(aabb, delta.y, gravity_dir);
        SweepResult {
            aabb,
            blocked_x,
            blocked_y,
        }
    }

    fn sweep_x(&self, aabb: Aabb, dx: f32) -> (Aabb, bool) {
        let ts = self.tile_size;
        let rows = self.cells(aabb.min.y, aabb.max.y);
        let column_blocks = |x: i32| rows.clone().any(|y| self.is_solid(x, y));
        if dx > 0.0 {
            // Columns whose left edge lies in [max.x, max.x + dx).
            let first = (aabb.max.x / ts).ceil() as i32;
            let last = ((aabb.max.x + dx) / ts).ceil() as i32 - 1;
            if let Some(x) = (first..=last).find(|&x| column_blocks(x)) {
                let stop = x as f32 * ts - aabb.max.x;
                return (aabb.translated(Vec2::new(stop, 0.0)), true);
            }
        } else if dx < 0.0 {
            // Columns whose right edge lies in (min.x + dx, min.x].
            let first = (aabb.min.x / ts).floor() as i32 - 1;
            let last = ((aabb.min.x + dx) / ts).floor() as i32;
            if let Some(x) = (last..=first).rev().find(|&x| column_blocks(x)) {
                let stop = (x + 1) as f32 * ts - aabb.min.x;
                return (aabb.translated(Vec2::new(stop, 0.0)), true);
            }
        }
        (aabb.translated(Vec2::new(dx, 0.0)), false)
    }

    fn sweep_y(&self, aabb: Aabb, dy: f32, gravity_dir: f32) -> (Aabb, bool) {
        let ts = self.tile_size;
        let columns = self.cells(aabb.min.x, aabb.max.x);
        let row_blocks = |y: i32| {
            let support = if gravity_dir < 0.0 && dy < 0.0 {
                aabb.min.y >= (y + 1) as f32 * ts
            } else if gravity_dir > 0.0 && dy > 0.0 {
                aabb.max.y <= y as f32 * ts
            } else {
                false
            };
            columns
                .clone()
                .any(|x| self.is_solid(x, y) || support && self.is_platform(x, y))
        };
        // Rows outside the grid are empty: only scan the part of the path inside it.
        let (lo, hi) = (0, self.height - 1);
        if dy > 0.0 {
            let first = ((aabb.max.y / ts).ceil() as i32).max(lo);
            let last = (((aabb.max.y + dy) / ts).ceil() as i32 - 1).min(hi);
            if let Some(y) = (first..=last).find(|&y| row_blocks(y)) {
                let stop = y as f32 * ts - aabb.max.y;
                return (aabb.translated(Vec2::new(0.0, stop)), true);
            }
        } else if dy < 0.0 {
            let first = ((aabb.min.y / ts).floor() as i32 - 1).min(hi);
            let last = (((aabb.min.y + dy) / ts).floor() as i32).max(lo);
            if let Some(y) = (last..=first).rev().find(|&y| row_blocks(y)) {
                let stop = (y + 1) as f32 * ts - aabb.min.y;
                return (aabb.translated(Vec2::new(0.0, stop)), true);
            }
        }
        (aabb.translated(Vec2::new(0.0, dy)), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mw_level::{Layer, LayerKind, Size};

    /// Grid from rows written top row first, `#` solid, tile size 10.
    fn grid(rows: &[&str]) -> CollisionGrid {
        let mut level = Level::new(
            "t",
            "T",
            Size {
                w: rows[0].len() as u32,
                h: rows.len() as u32,
            },
        );
        level.tile_size = 10;
        level.legend.insert('#', "ground".into());
        level.layers.push(Layer {
            name: "ground".into(),
            kind: LayerKind::Solid,
            tiles: rows.iter().map(|r| r.to_string()).collect(),
        });
        CollisionGrid::from_level(&level)
    }

    fn body(x: f32, y: f32) -> Aabb {
        Aabb::from_feet(Vec2::new(x, y), Vec2::new(6.0, 8.0))
    }

    #[test]
    fn grid_is_y_up_with_side_walls() {
        let g = grid(&["#...", "...."]);
        assert!(g.is_solid(0, 1));
        assert!(!g.is_solid(0, 0));
        assert!(
            g.is_solid(-1, 0) && g.is_solid(4, 0),
            "invisible side walls"
        );
        assert!(
            !g.is_solid(1, -1) && !g.is_solid(1, 2),
            "open below and above"
        );
    }

    #[test]
    fn falls_onto_floor_flush() {
        let g = grid(&["....", "....", "####"]);
        let r = g.sweep(body(15.0, 25.0), Vec2::new(0.0, -20.0));
        assert!(r.blocked_y && !r.blocked_x);
        assert_eq!(
            r.aabb.feet(),
            Vec2::new(15.0, 10.0),
            "stops on top of row 0 (y = 0..10)"
        );
        // Resting exactly on the floor: a further move down is blocked at once.
        let again = g.sweep(r.aabb, Vec2::new(0.0, -1.0));
        assert!(again.blocked_y);
        assert_eq!(again.aabb, r.aabb);
    }

    #[test]
    fn no_tunneling_through_thin_floor_or_wall() {
        let g = grid(&[
            ".........",
            ".........",
            ".........",
            "....#....",
            ".........",
        ]);
        let r = g.sweep(body(45.0, 45.0), Vec2::new(0.0, -10_000.0));
        assert_eq!(r.aabb.feet().y, 20.0, "lands on the 1-tile block");
        let g = grid(&["....#....", "....#....", "....#...."]);
        let r = g.sweep(body(15.0, 0.0), Vec2::new(10_000.0, 0.0));
        assert!(r.blocked_x);
        assert_eq!(r.aabb.max.x, 40.0, "flush against the wall's left side");
        let r = g.sweep(body(75.0, 0.0), Vec2::new(-10_000.0, 0.0));
        assert_eq!(r.aabb.min.x, 50.0, "flush against the wall's right side");
    }

    #[test]
    fn hits_ceiling() {
        let g = grid(&["####", "....", "....", "...."]);
        let r = g.sweep(body(15.0, 0.0), Vec2::new(0.0, 100.0));
        assert!(r.blocked_y);
        assert_eq!(r.aabb.max.y, 30.0);
    }

    #[test]
    fn x_then_y_slides_along_floor() {
        let g = grid(&["....", "....", "####"]);
        let start = body(15.0, 10.0);
        let r = g.sweep(start, Vec2::new(5.0, -3.0));
        assert!(!r.blocked_x && r.blocked_y);
        assert_eq!(r.aabb.feet(), Vec2::new(20.0, 10.0));
    }

    #[test]
    fn touching_edges_do_not_block_perpendicular_motion() {
        // Body flush against a wall on its right can still move vertically.
        let g = grid(&["..#.", "..#.", "..#.", "...."]);
        let flush = Aabb::from_feet(Vec2::new(17.0, 0.0), Vec2::new(6.0, 8.0));
        assert_eq!(flush.max.x, 20.0);
        assert!(!g.overlaps(&flush));
        let r = g.sweep(flush, Vec2::new(0.0, 25.0));
        assert!(!r.blocked_y);
    }

    #[test]
    fn falls_out_below_the_grid() {
        let g = grid(&["....", "...."]);
        let r = g.sweep(body(15.0, 5.0), Vec2::new(0.0, -1000.0));
        assert!(!r.blocked_y);
        assert_eq!(r.aabb.feet().y, -995.0);
    }

    #[test]
    fn side_walls_outside_grid() {
        let g = grid(&["....", "...."]);
        let r = g.sweep(body(15.0, 0.0), Vec2::new(-100.0, 0.0));
        assert!(r.blocked_x);
        assert_eq!(r.aabb.min.x, 0.0);
    }
}
