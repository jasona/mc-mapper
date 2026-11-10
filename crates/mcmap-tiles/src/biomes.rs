//! Biome tiles: one biome lookup per distinct 4x4-block cell under the tile, coloured from the
//! palette. The palette is the reference capture's where M0 measured it, and the familiar
//! seed-map colours elsewhere (cubiomes' table, as remembered; check against the art when a
//! biome looks off).

use crate::pyramid::{blocks_per_pixel, TileKey, TILE};
use mcmap_worldgen::Surface;

/// Colour of a biome id such as `minecraft:plains`.
pub fn color(name: &str) -> [u8; 3] {
    let key = name.strip_prefix("minecraft:").unwrap_or(name);
    let hex = match key {
        // Measured from the reference capture (M0).
        "forest" => 0x056621,
        "ocean" => 0x000070,
        "cold_ocean" => 0x202070,
        "plains" => 0x8db360,
        "stony_shore" => 0xa2a284,
        "grove" => 0x92b2a0,
        "river" => 0x0000ff,
        "snowy_slopes" => 0xdaf1f1,
        "frozen_peaks" => 0xcdd7ff,
        "meadow" => 0x8ca470,
        "taiga" => 0x0b6659,
        "dark_forest" => 0x40511a,
        "deep_ocean" => 0x000030,
        "old_growth_spruce_taiga" => 0x818e79,
        // Not in the reference view yet.
        "desert" => 0xfa9418,
        "windswept_hills" => 0x606060,
        "swamp" => 0x07f9b2,
        "frozen_ocean" => 0x7070d6,
        "frozen_river" => 0xa0a0ff,
        "snowy_plains" => 0xffffff,
        "mushroom_fields" => 0xff00ff,
        "beach" => 0xfade55,
        "jungle" => 0x537b09,
        "sparse_jungle" => 0x628b17,
        "snowy_beach" => 0xfaf0c0,
        "birch_forest" => 0x307444,
        "snowy_taiga" => 0x31554a,
        "old_growth_pine_taiga" => 0x596651,
        "windswept_forest" => 0x5b7352,
        "savanna" => 0xbdb25f,
        "savanna_plateau" => 0xa79d64,
        "badlands" => 0xd94515,
        "wooded_badlands" => 0xb09765,
        "warm_ocean" => 0x0000ac,
        "lukewarm_ocean" => 0x000090,
        "deep_lukewarm_ocean" => 0x000040,
        "deep_cold_ocean" => 0x202038,
        "deep_frozen_ocean" => 0x404090,
        "sunflower_plains" => 0xb5db88,
        "windswept_gravelly_hills" => 0x888888,
        "flower_forest" => 0x2d8e49,
        "ice_spikes" => 0xb4dcdc,
        "old_growth_birch_forest" => 0x589c6c,
        "windswept_savanna" => 0xe5da87,
        "eroded_badlands" => 0xff6d3d,
        "bamboo_jungle" => 0x768e14,
        "jagged_peaks" => 0xdcdcc8,
        "stony_peaks" => 0x7b8f74,
        "mangrove_swamp" => 0x2ccc8e,
        "cherry_grove" => 0xff91c8,
        "pale_garden" => 0x696d63,
        "dappled_forest" => 0xc86a28,
        "dripstone_caves" => 0x4e3012,
        "lush_caves" => 0x283c00,
        "deep_dark" => 0x031f29,
        "sulfur_caves" => 0xaba64f,
        // Nether and End.
        "nether_wastes" => 0x572526,
        "soul_sand_valley" => 0x4d3a2e,
        "crimson_forest" => 0x981a11,
        "warped_forest" => 0x49907b,
        "basalt_deltas" => 0x645f63,
        "the_end" => 0xd2cc9c,
        "small_end_islands" => 0x454166,
        "end_midlands" => 0xc2bc82,
        "end_highlands" => 0xe6e1a2,
        "end_barrens" => 0x7c7978,
        _ => return fallback(key),
    };
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

/// Deep cosmic void colour used for the End's 1,024-block central exclusion ring and outer abyss.
pub const END_VOID_COLOR: [u8; 3] = [0x0d, 0x0d, 0x16];

/// A stable mid-grey-ish colour for a biome this table doesn't know (a later version's).
fn fallback(key: &str) -> [u8; 3] {
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    [96 + (h & 63) as u8, 96 + ((h >> 8) & 63) as u8, 96 + ((h >> 16) & 63) as u8]
}

/// Biome colours indexed like `surface.names()` (plus the synthetic void entry at index `5` in the End).
pub fn palette(surface: &Surface) -> Vec<[u8; 3]> {
    let mut cols: Vec<[u8; 3]> = surface.names().iter().map(|n| color(n)).collect();
    if surface.is_end() {
        cols.push(END_VOID_COLOR);
    }
    cols
}

/// Per-biome water classification (`0` = land, `1` = ocean, `2` = river) parallel to [`palette`].
pub fn water_flags(surface: &Surface) -> Vec<u8> {
    surface
        .names()
        .iter()
        .map(|name| {
            let key = name.strip_prefix("minecraft:").unwrap_or(name);
            if key.ends_with("ocean") {
                1
            } else if key == "river" || key == "frozen_river" {
                2
            } else {
                0
            }
        })
        .collect()
}

/// The quart (4-block cell) each pixel column or row samples: the one under the pixel centre.
fn quarts(origin: f64, bpp: f64) -> Vec<i32> {
    (0..TILE).map(|i| ((origin + (i as f64 + 0.5) * bpp) / 4.0).floor() as i32).collect()
}

/// The block coordinate each pixel column or row samples: the block under the pixel centre.
fn blocks(origin: f64, bpp: f64) -> Vec<i32> {
    (0..TILE).map(|i| (origin + (i as f64 + 0.5) * bpp).floor() as i32).collect()
}

/// RGBA, `TILE` x `TILE`, opaque.
pub fn render(surface: &Surface, colors: &[[u8; 3]], key: TileKey) -> Vec<u8> {
    paint(&ids(surface, key), colors)
}

/// The tile's biome ids, one per pixel, row by row: what the disk cache keeps at standard zoom.
pub fn ids(surface: &Surface, key: TileKey) -> Vec<u16> {
    let n = TILE as usize;
    let bpp = blocks_per_pixel(key.level);
    let [ox, oz] = key.origin();
    if surface.is_end() {
        let close_zoom = key.level <= -4;
        let (bx, bz) = (blocks(ox, bpp), blocks(oz, bpp));
        let mut out = vec![0u16; n * n];
        let mut memo = mcmap_worldgen::density::Memo::new(64);
        let mut row = vec![0u16; n];
        for j in 0..n {
            let same_z = j > 0
                && if close_zoom {
                    bz[j] == bz[j - 1]
                } else {
                    let in_ring = ((bz[j] >> 4) as i64).abs() <= 64 || ((bz[j - 1] >> 4) as i64).abs() <= 64;
                    if in_ring { bz[j] == bz[j - 1] } else { (bz[j] >> 4) == (bz[j - 1] >> 4) }
                };
            if !same_z {
                let z = bz[j];
                let z_ring = ((z >> 4) as i64).abs() <= 64;
                for i in 0..n {
                    let same_x = i > 0
                        && if close_zoom || z_ring {
                            bx[i] == bx[i - 1]
                        } else {
                            (bx[i] >> 4) == (bx[i - 1] >> 4)
                        };
                    row[i] = if same_x { row[i - 1] } else { surface.end_tile_id(bx[i], z, close_zoom, &mut memo) };
                }
            }
            out[j * n..(j + 1) * n].copy_from_slice(&row);
        }
        return out;
    }
    let (qx, qz) = (quarts(ox, bpp), quarts(oz, bpp));
    let mut out = vec![0u16; n * n];
    let mut hint = None;
    let mut memo = mcmap_worldgen::density::Memo::new(64);
    let mut row: Vec<u16> = vec![0; n];
    for j in 0..n {
        let same_z = j > 0 && qz[j] == qz[j - 1];
        if !same_z {
            for i in 0..n {
                let same_x = i > 0 && qx[i] == qx[i - 1];
                row[i] = if same_x { row[i - 1] } else { surface.biome_with_hint_memo(qx[i], qz[j], &mut hint, &mut memo).0 };
            }
        }
        out[j * n..(j + 1) * n].copy_from_slice(&row);
    }
    out
}

/// Biome ids for `key`, switching to Minecraft's block-exact Voronoi biome zoom (`BiomeManager.getBiome`)
/// when zoomed closely (`key.level <= -4`, i.e. `<= 0.25` blocks per tile pixel) so shorelines and
/// biome borders match in-game block boundaries instead of 4x4 stair-steps.
pub fn ids_for_seed(surface: &Surface, zoom_seed: i64, key: TileKey) -> Vec<u16> {
    if surface.is_end() || key.level > -4 {
        return ids(surface, key);
    }
    let n = TILE as usize;
    let bpp = blocks_per_pixel(key.level);
    let [ox, oz] = key.origin();
    let (bx, bz) = (blocks(ox, bpp), blocks(oz, bpp));
    let qx0 = (bx[0].wrapping_sub(2)) >> 2;
    let qx1 = ((bx[n - 1].wrapping_sub(2)) >> 2) + 1;
    let qz0 = (bz[0].wrapping_sub(2)) >> 2;
    let qz1 = ((bz[n - 1].wrapping_sub(2)) >> 2) + 1;
    let gw = (qx1 - qx0 + 1) as usize;
    let gh = (qz1 - qz0 + 1) as usize;
    let mut grid = vec![0u16; gw * gh];
    let mut hint = None;
    let mut memo = mcmap_worldgen::density::Memo::new(64);
    for gz in 0..gh {
        let qz = qz0 + gz as i32;
        for gx in 0..gw {
            let qx = qx0 + gx as i32;
            grid[gz * gw + gx] = surface.biome_with_hint_memo(qx, qz, &mut hint, &mut memo).0;
        }
    }
    let mut out = vec![0u16; n * n];
    let mut row = vec![0u16; n];
    for j in 0..n {
        if j == 0 || bz[j] != bz[j - 1] {
            let z = bz[j];
            for i in 0..n {
                if i > 0 && bx[i] == bx[i - 1] {
                    row[i] = row[i - 1];
                } else {
                    let (qx, _qy, qz) = mcmap_worldgen::fiddled_quart(zoom_seed, bx[i], 64, z);
                    row[i] = grid[(qz - qz0) as usize * gw + (qx - qx0) as usize];
                }
            }
        }
        out[j * n..(j + 1) * n].copy_from_slice(&row);
    }
    out
}

/// RGBA pixels of biome ids.
pub fn paint(ids: &[u16], colors: &[[u8; 3]]) -> Vec<u8> {
    let rgba: Vec<[u8; 4]> = colors.iter().map(|c| [c[0], c[1], c[2], 255]).collect();
    let mut out = vec![0u8; ids.len() * 4];
    for (px, &id) in out.chunks_exact_mut(4).zip(ids) {
        let c = rgba.get(id as usize).copied().unwrap_or([255, 0, 255, 255]);
        px.copy_from_slice(&c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_colours() {
        assert_eq!(color("minecraft:plains"), [0x8d, 0xb3, 0x60]);
        assert_eq!(color("minecraft:dappled_forest"), [0xc8, 0x6a, 0x28]);
        assert_eq!(color("minecraft:sulfur_caves"), [0xab, 0xa6, 0x4f]);
        assert_eq!(color("minecraft:some_future_biome"), color("minecraft:some_future_biome"));
    }

    #[test]
    fn quarts_follow_the_pixel_centre() {
        // Level 0: one quart per pixel. Level -2: four pixels per quart.
        assert_eq!(&quarts(0.0, 4.0)[..3], &[0, 1, 2]);
        assert_eq!(&quarts(-1024.0, 4.0)[..2], &[-256, -255]);
        assert_eq!(&quarts(0.0, 1.0)[..6], &[0, 0, 0, 0, 1, 1]);
        assert_eq!(&quarts(0.0, 16.0)[..2], &[2, 6]);
    }
}
