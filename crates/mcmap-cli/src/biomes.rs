//! `render` and `calibrate`: the biome core from the command line.

use mcmap_data::pack::{pack_path, RawPack};
use mcmap_tiles::atlas::Rgba;
use mcmap_tiles::biomes;
use mcmap_worldgen::Surface;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

pub fn load(version: &str, pack: Option<PathBuf>, seed: i64) -> Result<Surface, String> {
    let path = pack.unwrap_or_else(|| pack_path(version));
    let raw = RawPack::read(&path).map_err(|e| format!("{e}\n(make the pack with `cargo run -q -p xtask -- extract --version {version}`)"))?;
    Surface::new(&raw, seed)
}

/// Render `w` x `h` pixels centred on `center`, `ppb` pixels per block, to a PNG.
pub fn render(s: &Surface, center: [f64; 2], ppb: f64, w: u32, h: u32, out: &Path) -> Result<(), String> {
    let colors = biomes::palette(s);
    let t0 = Instant::now();
    let rows: Vec<Vec<u8>> = (0..h)
        .into_par_iter()
        .map(|j| {
            let mut hint = None;
            let bz = center[1] + (j as f64 + 0.5 - h as f64 / 2.0) / ppb;
            let qz = (bz / 4.0).floor() as i32;
            let mut row = Vec::with_capacity(w as usize * 3);
            let mut last: Option<(i32, [u8; 3])> = None;
            for i in 0..w {
                let bx = center[0] + (i as f64 + 0.5 - w as f64 / 2.0) / ppb;
                let qx = (bx / 4.0).floor() as i32;
                let c = match last {
                    Some((q, c)) if q == qx => c,
                    _ => colors[s.biome_with_hint(qx, qz, &mut hint).0 as usize],
                };
                last = Some((qx, c));
                row.extend_from_slice(&c);
            }
            row
        })
        .collect();
    let img = image::RgbImage::from_raw(w, h, rows.concat()).ok_or("image size")?;
    img.save(out).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} ({w}x{h}, {:.2?})", out.display(), t0.elapsed());
    Ok(())
}

/// Reference capture colours measured in M0.
const REFERENCE: [(u32, &str); 14] = [
    (0x056621, "forest"), (0x000070, "ocean"), (0x202070, "cold_ocean"), (0x8db360, "plains"),
    (0xa2a284, "stony_shore"), (0x92b2a0, "grove"), (0x0000ff, "river"), (0xdaf1f1, "snowy_slopes"),
    (0xcdd7ff, "frozen_peaks"), (0x8ca470, "meadow"), (0x0b6659, "taiga"), (0x40511a, "dark_forest"),
    (0x000030, "deep_ocean"), (0x818e79, "old_growth_spruce_taiga"),
];

/// Compare against a reference capture: every 4x4 cell whose centre falls inside the image
/// (away from the edge) is looked up in the capture by colour.
pub fn calibrate(s: &Surface, reference: &Path, center: [f64; 2], ppb: f64) -> Result<bool, String> {
    let img = Rgba::decode(&std::fs::read(reference).map_err(|e| format!("{}: {e}", reference.display()))?)?;
    let by_color: HashMap<[u8; 3], &str> = REFERENCE.iter().map(|&(c, n)| ([(c >> 16) as u8, (c >> 8) as u8, c as u8], n)).collect();
    let (w, h) = (img.w as f64, img.h as f64);
    let block = |p: f64, size: f64, c: f64| c + (p - size / 2.0) / ppb;
    let (qx0, qx1) = ((block(1.0, w, center[0]) / 4.0).ceil() as i32, (block(w - 1.0, w, center[0]) / 4.0).floor() as i32);
    let (qz0, qz1) = ((block(1.0, h, center[1]) / 4.0).ceil() as i32, (block(h - 1.0, h, center[1]) / 4.0).floor() as i32);
    let (mut total, mut agree, mut unknown) = (0usize, 0usize, 0usize);
    let mut confusion: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut hint = None;
    for qz in qz0..qz1 {
        for qx in qx0..qx1 {
            let (bx, bz) = (qx as f64 * 4.0 + 2.0, qz as f64 * 4.0 + 2.0);
            let (px, py) = ((bx - center[0]) * ppb + w / 2.0, (bz - center[1]) * ppb + h / 2.0);
            if px < 1.0 || py < 1.0 || px >= w - 1.0 || py >= h - 1.0 {
                continue;
            }
            let o = ((py as u32 * img.w + px as u32) * 4) as usize;
            let rgb = [img.px[o], img.px[o + 1], img.px[o + 2]];
            let ours = s.names()[s.biome_with_hint(qx, qz, &mut hint).0 as usize].trim_start_matches("minecraft:").to_owned();
            total += 1;
            match by_color.get(&rgb) {
                None => unknown += 1,
                Some(&want) if want == ours => agree += 1,
                Some(&want) => *confusion.entry((want.to_owned(), ours)).or_default() += 1,
            }
        }
    }
    let known = total - unknown;
    println!("{total} cells, {unknown} with a colour outside the reference palette, {agree}/{known} agree ({:.3}%)", 100.0 * agree as f64 / known.max(1) as f64);
    for ((want, got), n) in &confusion {
        println!("  {n:6}  capture {want}, ours {got}");
    }
    Ok(agree == known && known > 0)
}
