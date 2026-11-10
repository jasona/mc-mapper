//! Tile pyramid geometry. Tiles are 256x256 device pixels. At level L one tile pixel covers
//! 4 * 2^L blocks, so level 0 is native biome resolution and negative levels enlarge cells.
//!
//! The view draws the level whose tiles show at a scale in (0.5, 1]: tiles only ever minify, which
//! keeps them crisp under Rune's bilinear sampling.

pub const TILE: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub level: i32,
    pub x: i64,
    pub z: i64,
}

/// Blocks covered by one tile pixel at `level`.
pub fn blocks_per_pixel(level: i32) -> f64 {
    4.0 * 2f64.powi(level)
}

/// Blocks covered by one tile edge at `level`.
pub fn tile_blocks(level: i32) -> f64 {
    blocks_per_pixel(level) * TILE as f64
}

impl TileKey {
    /// Block coordinate of the tile's top-left (north-west) corner.
    pub fn origin(self) -> [f64; 2] {
        let s = tile_blocks(self.level);
        [self.x as f64 * s, self.z as f64 * s]
    }

    pub fn containing(level: i32, block: [f64; 2]) -> TileKey {
        let s = tile_blocks(level);
        TileKey { level, x: (block[0] / s).floor() as i64, z: (block[1] / s).floor() as i64 }
    }

    /// The tile one level up that covers this one.
    pub fn parent(self) -> TileKey {
        TileKey { level: self.level + 1, x: self.x.div_euclid(2), z: self.z.div_euclid(2) }
    }
}

/// The map camera. Coordinates stay `f64` blocks end to end: Minecraft worlds reach 30 million
/// blocks, past `f32` integer precision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Block coordinate at the centre of the view (x, z).
    pub center: [f64; 2],
    /// Points per block.
    pub zoom: f64,
}

pub const MIN_ZOOM: f64 = 1.0 / 512.0;
pub const MAX_ZOOM: f64 = 32.0;

impl Camera {
    pub fn new(center: [f64; 2], zoom: f64) -> Camera {
        Camera { center, zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM) }
    }

    /// The tile level for this zoom at `scale` device pixels per point.
    pub fn level(&self, scale: f64) -> i32 {
        // Tile scale k = zoom * scale * 4 * 2^L must land in (0.5, 1].
        let px_per_block = self.zoom * scale;
        let l = (-(px_per_block * 4.0).log2()).floor() as i32;
        // Guard the boundary against rounding: k must not exceed 1.
        if px_per_block * blocks_per_pixel(l) > 1.0 + 1e-9 { l - 1 } else { l }
    }

    /// How large a level-`level` tile pixel draws, in device pixels: in (0.5, 1] for `self.level`.
    pub fn tile_scale(&self, level: i32, scale: f64) -> f64 {
        self.zoom * scale * blocks_per_pixel(level)
    }

    /// The block under a point of a viewport `size` points large.
    pub fn block_at(&self, point: [f64; 2], size: [f64; 2]) -> [f64; 2] {
        [
            self.center[0] + (point[0] - size[0] / 2.0) / self.zoom,
            self.center[1] + (point[1] - size[1] / 2.0) / self.zoom,
        ]
    }

    /// Where a block shows in a viewport `size` points large.
    pub fn point_of(&self, block: [f64; 2], size: [f64; 2]) -> [f64; 2] {
        [
            size[0] / 2.0 + (block[0] - self.center[0]) * self.zoom,
            size[1] / 2.0 + (block[1] - self.center[1]) * self.zoom,
        ]
    }

    /// Zoom by `factor` keeping the block under `point` fixed.
    pub fn zoom_about(&mut self, point: [f64; 2], size: [f64; 2], factor: f64) {
        let anchor = self.block_at(point, size);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.center = [
            anchor[0] - (point[0] - size[0] / 2.0) / self.zoom,
            anchor[1] - (point[1] - size[1] / 2.0) / self.zoom,
        ];
    }

    /// Visible tiles for a viewport, plus `margin` rings of prefetch, nearest the centre first.
    pub fn visible_tiles(&self, size: [f64; 2], scale: f64, margin: i64) -> Vec<TileKey> {
        let level = self.level(scale);
        let a = TileKey::containing(level, self.block_at([0.0, 0.0], size));
        let b = TileKey::containing(level, self.block_at(size, size));
        let c = TileKey::containing(level, self.center);
        let mut v: Vec<TileKey> = ((a.z - margin)..=(b.z + margin))
            .flat_map(|z| ((a.x - margin)..=(b.x + margin)).map(move |x| TileKey { level, x, z }))
            .collect();
        v.sort_by_key(|t| ((t.x - c.x).abs().max((t.z - c.z).abs()), (t.x - c.x).abs() + (t.z - c.z).abs(), t.z, t.x));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_keep_tiles_minified() {
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let mut zoom = MIN_ZOOM;
            while zoom <= MAX_ZOOM {
                let cam = Camera::new([0.0, 0.0], zoom);
                let k = cam.tile_scale(cam.level(scale), scale);
                assert!(k > 0.5 - 1e-9 && k <= 1.0 + 1e-9, "zoom {zoom} scale {scale}: tile scale {k}");
                zoom *= 1.07;
            }
        }
        // The reference capture: 1.25 device px per block at scale 1 is level -3 (0.5 blocks per px), k 0.625.
        let cam = Camera::new([2839.0, -1418.0], 1.25);
        assert_eq!(cam.level(1.0), -3);
        assert_eq!(cam.level(2.0), -4);
    }

    #[test]
    fn zoom_about_keeps_the_anchor() {
        let size = [800.0, 600.0];
        let mut cam = Camera::new([2839.0, -1418.0], 1.0);
        let p = [100.0, 450.0];
        let before = cam.block_at(p, size);
        cam.zoom_about(p, size, 3.7);
        let after = cam.block_at(p, size);
        assert!((before[0] - after[0]).abs() < 1e-9 && (before[1] - after[1]).abs() < 1e-9);
    }

    #[test]
    fn far_coordinates_keep_sub_block_precision() {
        let size = [800.0, 600.0];
        let cam = Camera::new([29_999_983.25, -29_999_983.75], 8.0);
        let p = cam.point_of([29_999_983.5, -29_999_983.5], size);
        assert!((p[0] - 402.0).abs() < 1e-6 && (p[1] - 302.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn visible_tiles_cover_the_view_centre_first() {
        let cam = Camera::new([0.0, 0.0], 0.25);
        let tiles = cam.visible_tiles([1024.0, 768.0], 1.0, 0);
        assert_eq!(tiles[0], TileKey::containing(cam.level(1.0), [0.0, 0.0]));
        // Every corner of the view is in some tile.
        let size = [1024.0, 768.0];
        for p in [[0.0, 0.0], [1024.0, 0.0], [0.0, 768.0], [1024.0, 768.0]] {
            let t = TileKey::containing(tiles[0].level, cam.block_at(p, size));
            assert!(tiles.contains(&t), "{p:?} -> {t:?}");
        }
        assert_eq!(TileKey { level: 0, x: -1, z: 3 }.parent(), TileKey { level: 1, x: -1, z: 1 });
    }
}
