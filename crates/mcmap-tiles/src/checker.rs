//! A stand-in tile: a checkerboard in block space with a tile border, tinted by level. M0 uses it
//! to prove pan, zoom and tile streaming before world generation exists.

use crate::pyramid::{blocks_per_pixel, TileKey, TILE};

/// RGBA, straight alpha, `TILE` x `TILE`.
pub fn render(key: TileKey) -> Vec<u8> {
    let n = TILE as usize;
    let bpp = blocks_per_pixel(key.level);
    let [ox, oz] = key.origin();
    // Squares are 64 blocks (four chunks), or coarser when a pixel covers more than 8 blocks.
    let square = 64.0 * (bpp / 8.0).max(1.0);
    let hue = level_hue(key.level);
    let mut out = vec![0u8; n * n * 4];
    for j in 0..n {
        let bz = oz + (j as f64 + 0.5) * bpp;
        for i in 0..n {
            let bx = ox + (i as f64 + 0.5) * bpp;
            let odd = ((bx / square).floor() as i64 + (bz / square).floor() as i64).rem_euclid(2) == 1;
            let border = i == 0 || j == 0;
            let origin_axis = (bx.abs() < bpp * 0.5) || (bz.abs() < bpp * 0.5);
            let c = if origin_axis {
                [230, 60, 60]
            } else if border {
                [20, 20, 24]
            } else if odd {
                hue
            } else {
                [hue[0] / 2 + 100, hue[1] / 2 + 100, hue[2] / 2 + 100]
            };
            let p = (j * n + i) * 4;
            out[p..p + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    out
}

fn level_hue(level: i32) -> [u8; 3] {
    const HUES: [[u8; 3]; 6] = [[70, 120, 90], [80, 100, 140], [130, 100, 70], [110, 80, 130], [70, 120, 130], [120, 120, 70]];
    HUES[level.rem_euclid(6) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_are_full_size_and_opaque() {
        let px = render(TileKey { level: -2, x: 3, z: -7 });
        assert_eq!(px.len(), (TILE * TILE * 4) as usize);
        assert!(px.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn neighbouring_levels_agree_on_squares() {
        // The same block lands on the same checker parity at levels -1 and 0.
        let a = TileKey { level: -1, x: 0, z: 0 };
        let b = TileKey { level: 0, x: 0, z: 0 };
        let (pa, pb) = (render(a), render(b));
        let at = |px: &[u8], key: TileKey, block: [f64; 2]| {
            let bpp = blocks_per_pixel(key.level);
            let (i, j) = ((block[0] / bpp) as usize, (block[1] / bpp) as usize);
            let p = (j * TILE as usize + i) * 4;
            [px[p], px[p + 1], px[p + 2]]
        };
        let lum = |c: [u8; 3]| c.iter().map(|&v| v as u32).sum::<u32>();
        for block in [[100.0, 100.0], [100.0, 20.0], [300.0, 400.0]] {
            assert_eq!(lum(at(&pa, a, block)) > 400, lum(at(&pb, b, block)) > 400, "{block:?}");
        }
    }
}
