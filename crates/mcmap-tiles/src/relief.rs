//! Cartographic hillshade & bathymetry from the terrain's offset spline: the height where the
//! Overworld's base density crosses zero, about `128 * offset + 128`.
//!
//! Above sea level, directional NW hillshade is paired with subtle alpine ridge lift and Imhof
//! warm-sun / cool-shadow tinting. Below sea level, ocean basins encode smooth submarine
//! bathymetry plus gentle sea-floor contour, while river surfaces stay calm and coastlines receive
//! crisp shoreline definition.

use crate::pyramid::{blocks_per_pixel, TileKey, TILE};
use mcmap_worldgen::terrain::Terrain;

/// Pixels between height samples; shading is interpolated between them.
const STEP: usize = 2;
const SEA: f32 = 63.0;
/// Vertical exaggeration, so gentle hills still read.
const EXAGGERATION: f32 = 2.6;

/// Estimated height at a block position.
pub fn height(t: &Terrain, x: i32, z: i32) -> f32 {
    (128.0 * t.offset(x, z) + 128.0).max(SEA)
}

/// One byte per pixel: 128 is flat land at sea level; brighter faces the NW sun (or high alpine
/// crests); values below 128 in ocean basins encode submarine depth and sea-floor contour.
pub fn shade(t: &Terrain, key: TileKey) -> Vec<u8> {
    let n = TILE as usize;
    let bpp = blocks_per_pixel(key.level);
    let [ox, oz] = key.origin();
    let m = n / STEP + 3; // grid points -1 ..= n/STEP + 1
    let at = |g: usize, o: f64| (o + ((g as f64 - 1.0) * STEP as f64 + 0.5) * bpp).floor() as i32;
    let xs: Vec<i32> = (0..m).map(|gi| at(gi, ox)).collect();
    let zs: Vec<i32> = (0..m).map(|gj| at(gj, oz)).collect();
    let mut raw = vec![0f32; m * m];
    let mut memo = mcmap_worldgen::density::Memo::new(0);
    for gj in 0..m {
        if gj > 0 && zs[gj] == zs[gj - 1] {
            let (prev, curr) = raw.split_at_mut(gj * m);
            curr[..m].copy_from_slice(&prev[(gj - 1) * m..gj * m]);
            continue;
        }
        let z = zs[gj];
        for gi in 0..m {
            raw[gj * m + gi] = if gi > 0 && xs[gi] == xs[gi - 1] {
                raw[gj * m + gi - 1]
            } else {
                128.0 * t.offset_memo(xs[gi], z, &mut memo) + 128.0
            };
        }
    }
    // Shade at grid points 1 ..= n/STEP + 1 (pixel positions 0, STEP, ...).
    let k = m - 2;
    let d = (2.0 * STEP as f64 * bpp) as f32;
    let (lx, ly, lz) = (-1.0f32, std::f32::consts::SQRT_2, -1.0f32);
    let ll = (lx * lx + ly * ly + lz * lz).sqrt();
    let flat = ly / ll;
    let mut s = vec![0f32; k * k];
    for j in 0..k {
        for i in 0..k {
            let (gi, gj) = (i + 1, j + 1);
            let hc = raw[gj * m + gi];
            if hc >= SEA {
                let gx = EXAGGERATION * (raw[gj * m + gi + 1].max(SEA) - raw[gj * m + gi - 1].max(SEA)) / d;
                let gz = EXAGGERATION * (raw[(gj + 1) * m + gi].max(SEA) - raw[(gj - 1) * m + gi].max(SEA)) / d;
                let nl = (gx * gx + 1.0 + gz * gz).sqrt();
                let dot = (-gx * lx + ly - gz * lz) / (nl * ll);
                let alpine = ((hc - 105.0) / 140.0).clamp(0.0, 1.0) * 0.08;
                s[j * k + i] = dot / flat + alpine;
            } else {
                let depth = (SEA - hc).max(0.0);
                // Continental shelves (< ~16 blocks deep) rise slightly above 1.0; deep trenches
                // descend smoothly toward 0.76, modulated by gentle seafloor slope contour.
                let bathy = 1.09 - (depth / 58.0).clamp(0.0, 1.0) * 0.33;
                let gx = 0.38 * EXAGGERATION * (raw[gj * m + gi + 1] - raw[gj * m + gi - 1]) / d;
                let gz = 0.38 * EXAGGERATION * (raw[(gj + 1) * m + gi] - raw[(gj - 1) * m + gi]) / d;
                let nl = (gx * gx + 1.0 + gz * gz).sqrt();
                let dot = (-gx * lx + ly - gz * lz) / (nl * ll);
                s[j * k + i] = bathy + (dot / flat - 1.0) * 0.32;
            }
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
            out[j * n + i] = (128.0 * v).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Darken or lighten RGBA pixels by a shade layer: 128 leaves a pixel as it is.
pub fn apply(rgba: &mut [u8], shade: &[u8]) {
    let factors: [f32; 256] = std::array::from_fn(|s| {
        // Soften: a fully dark face keeps 60% of its colour, a lit one gains up to 25%.
        (1.0 + (s as f32 / 128.0 - 1.0) * 0.9).clamp(0.6, 1.25)
    });
    for (p, &s) in rgba.chunks_exact_mut(4).zip(shade) {
        let f = factors[s as usize];
        p[0] = (p[0] as f32 * f).min(255.0) as u8;
        p[1] = (p[1] as f32 * f).min(255.0) as u8;
        p[2] = (p[2] as f32 * f).min(255.0) as u8;
    }
}

/// Biome-aware cartographic relief: Imhof warm-sun / cool-shadow hillshade on land, smooth
/// submarine bathymetry with cerulean continental-shelf glow in oceans, calm flat water on rivers,
/// and crisp 1-pixel shoreline accents.
pub fn apply_cartographic(rgba: &mut [u8], shade: &[u8], ids: &[u16], water: &[u8]) {
    let n = TILE as usize;
    if ids.len() != n * n || shade.len() != n * n || rgba.len() != n * n * 4 {
        apply(rgba, shade);
        return;
    }
    let land_lut: [[u16; 3]; 256] = std::array::from_fn(|s| {
        if s == 128 {
            return [256, 256, 256];
        }
        let delta = (s as f32 / 128.0 - 1.0) * 0.92;
        let f = (1.0 + delta).clamp(0.58, 1.28);
        let warm = delta.clamp(-0.35, 0.30);
        [
            (f * (1.0 + warm * 0.055) * 256.0).round() as u16,
            (f * (1.0 + warm * 0.015) * 256.0).round() as u16,
            (f * (1.0 - warm * 0.065) * 256.0).round() as u16,
        ]
    });
    let ocean_lut: [[u16; 3]; 256] = std::array::from_fn(|s| {
        if s == 128 {
            return [256, 256, 256];
        }
        let delta = (s as f32 / 128.0 - 1.0) * 0.95;
        let f = (1.0 + delta).clamp(0.64, 1.18);
        [
            (f * 256.0).round() as u16,
            (f * (1.0 + delta * 0.06).clamp(0.86, 1.10) * 256.0).round() as u16,
            (f * (1.0 - delta * 0.03).clamp(0.92, 1.10) * 256.0).round() as u16,
        ]
    });
    // Additive cerulean lift for shallow continental shelves so pure-blue oceans (`R = 0, G = 0`)
    // still reveal submarine shelves around islands and coastlines.
    let shelf_add: [[u8; 3]; 256] = std::array::from_fn(|s| {
        if s <= 128 {
            return [0, 0, 0];
        }
        let t = ((s - 128) as f32 / 18.0).clamp(0.0, 1.0);
        [(t * 9.0).round() as u8, (t * 22.0).round() as u8, (t * 30.0).round() as u8]
    });

    for j in 0..n {
        let row = j * n;
        for i in 0..n {
            let idx = row + i;
            let w = water.get(ids[idx] as usize).copied().unwrap_or(0);
            let s = shade[idx] as usize;
            let mut add = [0u8; 3];
            let mul = match w {
                0 => {
                    // Land: check if bordering water for a crisp cartographic coastline rim.
                    let touches_water = (i > 0 && water.get(ids[idx - 1] as usize).copied().unwrap_or(0) != 0)
                        || (i + 1 < n && water.get(ids[idx + 1] as usize).copied().unwrap_or(0) != 0)
                        || (j > 0 && water.get(ids[idx - n] as usize).copied().unwrap_or(0) != 0)
                        || (j + 1 < n && water.get(ids[idx + n] as usize).copied().unwrap_or(0) != 0);
                    let mut m = land_lut[s];
                    if touches_water {
                        m = [m[0] * 238 / 256, m[1] * 238 / 256, m[2] * 242 / 256];
                    }
                    m
                }
                1 => {
                    // Ocean: bathymetric shading + shallow continental shelf & shoal lift.
                    let touches_land = (i > 0 && water.get(ids[idx - 1] as usize).copied().unwrap_or(0) == 0)
                        || (i + 1 < n && water.get(ids[idx + 1] as usize).copied().unwrap_or(0) == 0)
                        || (j > 0 && water.get(ids[idx - n] as usize).copied().unwrap_or(0) == 0)
                        || (j + 1 < n && water.get(ids[idx + n] as usize).copied().unwrap_or(0) == 0);
                    add = shelf_add[s];
                    if touches_land {
                        add = [add[0].saturating_add(5), add[1].saturating_add(12), add[2].saturating_add(16)];
                    }
                    ocean_lut[s]
                }
                _ => {
                    // River: keep water ribbon calm and flat inside shaded valleys.
                    let calm = ((s as i32 - 128) / 6 + 128).clamp(0, 255) as usize;
                    ocean_lut[calm]
                }
            };
            let p = &mut rgba[idx * 4..idx * 4 + 4];
            p[0] = (((p[0] as u32 * mul[0] as u32 + 128) >> 8) + add[0] as u32).min(255) as u8;
            p[1] = (((p[1] as u32 * mul[1] as u32 + 128) >> 8) + add[1] as u32).min(255) as u8;
            p[2] = (((p[2] as u32 * mul[2] as u32 + 128) >> 8) + add[2] as u32).min(255) as u8;
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
