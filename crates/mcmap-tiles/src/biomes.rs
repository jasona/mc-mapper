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
        "dripstone_caves" => 0x4e3012,
        "lush_caves" => 0x283c00,
        "deep_dark" => 0x031f29,
        // Nether and End (cubiomes' table).
        "nether_wastes" => 0x572526,
        "soul_sand_valley" => 0x4d3a2e,
        "crimson_forest" => 0x981a11,
        "warped_forest" => 0x49907b,
        "basalt_deltas" => 0x645f63,
        "the_end" => 0x8080ff,
        "small_end_islands" => 0x4b4bab,
        "end_midlands" => 0xc9c959,
        "end_highlands" => 0xb5b536,
        "end_barrens" => 0x7070cc,
        _ => return fallback(key),
    };
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

/// A stable mid-grey-ish colour for a biome this table doesn't know (a later version's).
fn fallback(key: &str) -> [u8; 3] {
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    [96 + (h & 63) as u8, 96 + ((h >> 8) & 63) as u8, 96 + ((h >> 16) & 63) as u8]
}

/// Biome colours indexed like `surface.names()`.
pub fn palette(surface: &Surface) -> Vec<[u8; 3]> {
    surface.names().iter().map(|n| color(n)).collect()
}

/// The quart (4-block cell) each pixel column or row samples: the one under the pixel centre.
fn quarts(origin: f64, bpp: f64) -> Vec<i32> {
    (0..TILE).map(|i| ((origin + (i as f64 + 0.5) * bpp) / 4.0).floor() as i32).collect()
}

/// RGBA, `TILE` x `TILE`, opaque.
pub fn render(surface: &Surface, colors: &[[u8; 3]], key: TileKey) -> Vec<u8> {
    paint(&ids(surface, key), colors)
}

/// The tile's biome ids, one per pixel, row by row: what the disk cache keeps.
pub fn ids(surface: &Surface, key: TileKey) -> Vec<u16> {
    let n = TILE as usize;
    let bpp = blocks_per_pixel(key.level);
    let [ox, oz] = key.origin();
    let (qx, qz) = (quarts(ox, bpp), quarts(oz, bpp));
    let is_end = surface.is_end();
    let mut out = vec![0u16; n * n];
    let mut hint = None;
    let mut memo = mcmap_worldgen::density::Memo::new(64);
    let mut row: Vec<u16> = vec![0; n];
    for j in 0..n {
        let same_z = j > 0 && if is_end { (qz[j] >> 2) == (qz[j - 1] >> 2) } else { qz[j] == qz[j - 1] };
        if !same_z {
            for i in 0..n {
                let same_x = i > 0 && if is_end { (qx[i] >> 2) == (qx[i - 1] >> 2) } else { qx[i] == qx[i - 1] };
                row[i] = if same_x { row[i - 1] } else { surface.biome_with_hint_memo(qx[i], qz[j], &mut hint, &mut memo).0 };
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
