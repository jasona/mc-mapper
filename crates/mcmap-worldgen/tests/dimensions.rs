//! Nether and End biomes against the game: grids from `cargo xtask oracle dimgrid <seed>
//! <nether|end> <x0> <z0> <w> <h> <step> <out>`, saved as
//! Large Biomes and Amplified surfaces come from `dimgrid <seed> <large_biomes|amplified> ...`,
//! saved as `large-biomes_...` and `amplified_...`.
//! `$MCMAP_DIM/<dim>_<seed>_<x0>_<z0>_<w>_<h>_<step>.txt` (one biome id per line).
//!
//! ```sh
//! MCMAP_PACK=... MCMAP_DIM=$DIR cargo test -p mcmap-worldgen --release --test dimensions -- --ignored --nocapture
//! ```

use mcmap_data::pack::RawPack;
use mcmap_data::{Dimension, Preset};
use mcmap_worldgen::Surface;

#[test]
#[ignore]
fn nether_and_end_match_the_game() {
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").expect("MCMAP_PACK"))).unwrap();
    let dir = std::env::var("MCMAP_DIM").expect("MCMAP_DIM");
    let mut bad_total = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        // Overworld presets are named `large-biomes` and `amplified` here (no underscore).
        let parts: Vec<&str> = name.split('_').collect();
        let (dim, preset) = match parts[0] {
            "nether" => (Dimension::Nether, Preset::Normal),
            "end" => (Dimension::End, Preset::Normal),
            "large-biomes" => (Dimension::Overworld, Preset::LargeBiomes),
            "amplified" => (Dimension::Overworld, Preset::Amplified),
            o => panic!("{o}: unknown grid kind"),
        };
        let n: Vec<i64> = parts[1..].iter().map(|s| s.parse().unwrap()).collect();
        let (seed, x0, z0, w, h, step) = (n[0], n[1] as i32, n[2] as i32, n[3] as i32, n[4] as i32, n[5] as i32);
        let s = Surface::for_world(&pack, seed, dim, preset).unwrap();
        let want: Vec<String> = std::fs::read_to_string(&p).unwrap().lines().map(str::to_owned).collect();
        let mut bad = 0;
        let mut hint = None;
        for j in 0..h {
            for i in 0..w {
                let (x, z) = (x0 + i * step, z0 + j * step);
                let got = &s.names()[s.biome_with_hint(x >> 2, z >> 2, &mut hint).0 as usize];
                let w = &want[(j * w + i) as usize];
                if got != w {
                    if bad < 5 {
                        println!("  {name}: ({x}, {z}) got {got}, want {w}");
                    }
                    bad += 1;
                }
            }
        }
        println!("{name}: {} cells, {bad} differ", want.len());
        bad_total += bad;
    }
    assert_eq!(bad_total, 0);
}
