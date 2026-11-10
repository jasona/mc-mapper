//! Differential tests against the game's own classes. They need a version pack and grids written
//! by the oracle, so they are ignored by default:
//!
//! ```sh
//! cargo run -q -p xtask -- oracle -- climgrid <seed> <x0> <z0> <w> <h> <step> $DIR/<name>_<seed>.bin (and <name>_<seed>.txt holding "x0 z0 step")
//! MCMAP_PACK=~/.local/share/mcmap/packs/26.3.mcpack MCMAP_DIFF=$DIR \
//!     cargo test -p mcmap-worldgen --release --test oracle -- --ignored --nocapture
//! ```
//!
//! Every file in `$MCMAP_DIFF` named `*_<seed>.bin` is checked: the five raw climate floats bit
//! for bit, and the surface biome of every cell with the search hint replayed in the oracle's order.

use mcmap_data::pack::RawPack;
use mcmap_worldgen::Surface;
use std::path::PathBuf;

struct Grid {
    w: usize,
    h: usize,
    names: Vec<String>,
    vals: Vec<[f32; 5]>,
    biomes: Vec<u16>,
}

fn read_grid(bytes: &[u8]) -> Grid {
    let mut p = 0usize;
    let mut take = |n: usize| {
        let s = &bytes[p..p + n];
        p += n;
        s
    };
    assert_eq!(take(4), b"MCCG");
    let u32_ = |s: &[u8]| u32::from_be_bytes(s.try_into().unwrap()) as usize;
    let w = u32_(take(4));
    let h = u32_(take(4));
    let n = u32_(take(4));
    let mut names = Vec::new();
    for _ in 0..n {
        let len = u16::from_be_bytes(take(2).try_into().unwrap()) as usize;
        names.push(String::from_utf8(take(len).to_vec()).unwrap());
    }
    let mut vals = Vec::with_capacity(w * h);
    let mut biomes = Vec::with_capacity(w * h);
    for _ in 0..w * h {
        let mut v = [0f32; 5];
        for x in &mut v {
            *x = f32::from_be_bytes(take(4).try_into().unwrap());
        }
        vals.push(v);
        biomes.push(u16::from_be_bytes(take(2).try_into().unwrap()));
    }
    Grid { w, h, names, vals, biomes }
}

/// Seed from the grid name `<name>_<seed>.bin`, with x0, z0 and step in a sidecar `.txt`.
fn meta(path: &std::path::Path) -> (i64, i32, i32, i32) {
    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
    let seed: i64 = stem.rsplit_once('_').map(|(_, s)| s).unwrap().parse().unwrap();
    let txt = std::fs::read_to_string(path.with_extension("txt")).expect("sidecar .txt with x0 z0 step");
    let v: Vec<i32> = txt.split_whitespace().map(|s| s.parse().unwrap()).collect();
    (seed, v[0], v[1], v[2])
}

#[test]
#[ignore]
fn matches_the_game() {
    let pack = RawPack::read(&PathBuf::from(std::env::var("MCMAP_PACK").expect("MCMAP_PACK"))).unwrap();
    let dir = PathBuf::from(std::env::var("MCMAP_DIFF").expect("MCMAP_DIFF"));
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "bin")).collect();
    files.sort();
    assert!(!files.is_empty(), "no grids in {}", dir.display());
    let mut total_bad = 0;
    for f in files {
        let (seed, x0, z0, step) = meta(&f);
        let g = read_grid(&std::fs::read(&f).unwrap());
        let s = Surface::new(&pack, seed).unwrap();
        let t0 = std::time::Instant::now();
        let (mut bad_vals, mut bad_biomes, mut first) = (0usize, 0usize, None);
        let mut hint = None;
        for j in 0..g.h {
            for i in 0..g.w {
                let c = j * g.w + i;
                let (x, z) = (x0 + i as i32 * step, z0 + j as i32 * step);
                let (qx, qz) = (x >> 2, z >> 2);
                let (bx, bz) = (qx << 2, qz << 2);
                let c5 = &s.climate;
                let got = c5.raw(bx, 64, bz);
                if got.map(f32::to_bits) != g.vals[c].map(f32::to_bits) {
                    bad_vals += 1;
                    first.get_or_insert(format!("({x}, {z}) values {got:?} want {:?}", g.vals[c]));
                }
                let b = s.biome_with_hint(qx, qz, &mut hint);
                if s.names()[b.0 as usize] != g.names[g.biomes[c] as usize] {
                    bad_biomes += 1;
                    first.get_or_insert(format!("({x}, {z}) biome {} want {}", s.names()[b.0 as usize], g.names[g.biomes[c] as usize]));
                }
            }
        }
        println!(
            "{}: seed {seed}, {} cells in {:.2?}: {bad_vals} climate mismatches, {bad_biomes} biome mismatches{}",
            f.file_name().unwrap().to_string_lossy(),
            g.w * g.h,
            t0.elapsed(),
            first.map(|f| format!(" (first: {f})")).unwrap_or_default()
        );
        total_bad += bad_vals + bad_biomes;
    }
    assert_eq!(total_bad, 0);
}
