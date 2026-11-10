//! Hillshade from the terrain's offset spline: the height where the Overworld's base density
//! crosses zero, about `128 * offset + 128`. It is smooth and close to the game's terrain at
//! map scale, but leaves out the 3-D noise, caves and the surface rules: a relief shading, not a
//! heightmap. Water is flat at sea level.

use crate::pyramid::{blocks_per_pixel, TileKey, TILE};
use mcmap_worldgen::terrain::Terrain;

/// Pixels between height samples; shading is interpolated between them.
const STEP: usize = 2;
const SEA: f32 = 63.0;
/// Vertical exaggeration, so gentle hills still read.
const EXAGGERATION: f32 = 2.5;

/// Estimated height at a block position.
pub fn height(t: &Terrain, x: i32, z: i32) -> f32 {
    (128.0 * t.offset(x, z) + 128.0).max(SEA)
}

/// One byte per pixel: 128 is flat; brighter faces the light (north-west, 45 degrees up).
pub fn shade(t: &Terrain, key: TileKey) -> Vec<u8> {
    let n = TILE as usize;
    let bpp = blocks_per_pixel(key.level);
    let [ox, oz] = key.origin();
    let m = n / STEP + 3; // grid points -1 ..= n/STEP + 1
    let at = |g: usize, o: f64| (o + ((g as f64 - 1.0) * STEP as f64 + 0.5) * bpp).floor() as i32;
    let mut h = vec![0f32; m * m];
    for gj in 0..m {
        let z = at(gj, oz);
        for gi in 0..m {
            h[gj * m + gi] = height(t, at(gi, ox), z);
        }
    }
    // Shade at grid points 1 ..= n/STEP + 1 (pixel positions 0, STEP, ...).
    let k = m - 2;
    let d = (2.0 * STEP as f64 * bpp) as f32;
    let (lx, ly, lz) = (-1.0f32, 1.4142135f32, -1.0f32);
    let ll = (lx * lx + ly * ly + lz * lz).sqrt();
    let flat = ly / ll;
    let mut s = vec![0f32; k * k];
    for j in 0..k {
        for i in 0..k {
            let (gi, gj) = (i + 1, j + 1);
            let gx = EXAGGERATION * (h[gj * m + gi + 1] - h[gj * m + gi - 1]) / d;
            let gz = EXAGGERATION * (h[(gj + 1) * m + gi] - h[(gj - 1) * m + gi]) / d;
            // Surface normal (-gx, 1, -gz) against the light direction.
            let nl = (gx * gx + 1.0 + gz * gz).sqrt();
            let dot = (-gx * lx + ly - gz * lz) / (nl * ll);
            s[j * k + i] = dot / flat;
        }
    }
    let mut out = vec![128u8; n * n];
    for j in 0..n {
        let (fj, tj) = ((j / STEP).min(k - 2), (j % STEP) as f32 / STEP as f32);
        for i in 0..n {
            let (fi, ti) = ((i / STEP).min(k - 2), (i % STEP) as f32 / STEP as f32);
            let a = s[fj * k + fi] * (1.0 - ti) + s[fj * k + fi + 1] * ti;
            let b = s[(fj + 1) * k + fi] * (1.0 - ti) + s[(fj + 1) * k + fi + 1] * ti;
            let v = a * (1.0 - tj) + b * tj;
            out[j * n + i] = (128.0 * v).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Darken or lighten RGBA pixels by a shade layer: 128 leaves a pixel as it is.
pub fn apply(rgba: &mut [u8], shade: &[u8]) {
    for (p, &s) in rgba.chunks_exact_mut(4).zip(shade) {
        // Soften: a fully dark face keeps 60% of its colour, a lit one gains up to 25%.
        let f = (1.0 + (s as f32 / 128.0 - 1.0) * 0.9).clamp(0.6, 1.25);
        for c in &mut p[..3] {
            *c = (*c as f32 * f).min(255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn flat_ground_is_neutral() {
        let mut px = vec![100u8, 150, 200, 255];
        super::apply(&mut px, &[128]);
        assert_eq!(px, [100, 150, 200, 255]);
    }
}
