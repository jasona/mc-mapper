//! Decoration feature candidates against the game: dumps written by `cargo xtask oracle
//! featcands <seed> <dim> <cx0> <cz0> <cx1> <cz1> $MCMAP_FEATS/<name>.ndjson <ids>`. Geodes and
//! fossils must match exactly; the rest is reported.
//!
//! ```sh
//! MCMAP_PACK=... MCMAP_FEATS=$DIR cargo test -p mcmap-worldgen --release --test decoration -- --ignored --nocapture
//! ```

use mcmap_data::pack::RawPack;
use mcmap_worldgen::decoration::Decorations;
use mcmap_worldgen::world::World;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[test]
#[ignore]
fn decoration_candidates_match_the_game() {
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").expect("MCMAP_PACK"))).unwrap();
    let dir = std::env::var("MCMAP_FEATS").expect("MCMAP_FEATS");
    let mut bad = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "ndjson") {
            continue;
        }
        let lines: Vec<Value> = std::fs::read_to_string(&p).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let meta = &lines[0];
        let seed = meta["seed"].as_i64().unwrap();
        let dim = match meta["dimension"].as_str().unwrap() {
            "nether" => mcmap_data::Dimension::Nether,
            "end" => mcmap_data::Dimension::End,
            _ => mcmap_data::Dimension::Overworld,
        };
        let g = |k: &str| meta[k].as_i64().unwrap() as i32;
        let world = World::for_dimension(&pack, seed, dim).unwrap();
        let ids: Vec<String> = meta["features"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_owned()).collect();
        // The oracle doesn't place features either, so counted features (dungeons) compare
        // position for position here, though in the game only their first position is right.
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let deco = Decorations::new(&pack.files, dim, world.surface.names(), &ids).unwrap();
        for f in &deco.features {
            let want = meta["features"].as_array().unwrap().iter().find(|x| x["id"] == f.id.as_str()).unwrap();
            assert_eq!((f.step as i64, f.index as i64), (want["step"].as_i64().unwrap(), want["index"].as_i64().unwrap()), "{} order", f.id);
        }
        let zoom = mcmap_data::seeds::zoom_seed(seed);
        let mut want: BTreeMap<&str, BTreeSet<(i32, i32, i32)>> = BTreeMap::new();
        for l in &lines[1..] {
            let f = l["f"].as_str().unwrap();
            if ids.contains(&f) {
                let c = |k: &str| l[k].as_i64().unwrap() as i32;
                want.entry(f).or_default().insert((c("x"), c("y"), c("z")));
            }
        }
        let mut got: BTreeMap<String, Vec<(i32, i32, i32, bool)>> = BTreeMap::new();
        for cz in g("cz0")..=g("cz1") {
            for cx in g("cx0")..=g("cx1") {
                for c in deco.chunk(&world, zoom, cx, cz) {
                    got.entry(c.feature.clone()).or_default().push((c.pos.0, c.pos.1, c.pos.2, c.estimated_height));
                }
            }
        }
        println!("{}: seed {seed}", p.file_name().unwrap().to_string_lossy());
        for id in &ids {
            let w = want.get(id).cloned().unwrap_or_default();
            let gv = got.get(*id).cloned().unwrap_or_default();
            let est = gv.iter().any(|c| c.3);
            let wy: std::collections::HashMap<(i32, i32), i32> = w.iter().map(|v| ((v.0, v.2), v.1)).collect();
            let n_exact = gv.iter().filter(|c| w.contains(&(c.0, c.1, c.2))).count();
            let n_xz = gv.iter().filter(|c| wy.contains_key(&(c.0, c.2))).count();
            let mut dy: Vec<i32> = gv.iter().filter_map(|c| wy.get(&(c.0, c.2)).map(|y| (c.1 - y).abs())).collect();
            dy.sort();
            let wxz: BTreeSet<(i32, i32)> = w.iter().map(|v| (v.0, v.2)).collect();
            let gxz: BTreeSet<(i32, i32)> = gv.iter().map(|v| (v.0, v.2)).collect();
            let missing = wxz.difference(&gxz).count();
            let extra = gxz.difference(&wxz).count();
            println!(
                "  {id:<32} game {:>5}  ours {:>5}  exact {:>5}  xz {:>5}  missing {missing:>3}  extra {extra:>3}{}",
                w.len(),
                gv.len(),
                n_exact,
                n_xz,
                if est { format!("  |dy| p50 {} p90 {}", dy.get(dy.len() / 2).unwrap_or(&0), dy.get(dy.len() * 9 / 10).unwrap_or(&0)) } else { String::new() }
            );
            // Geodes and fossils must match exactly. Surface features take their height (and so
            // the biome they check) from the terrain estimate, and springs read cave biomes at
            // depth where a few differ: those are reported only.
            if ["minecraft:amethyst_geode", "minecraft:fossil_upper", "minecraft:fossil_lower"].contains(id) {
                bad += missing + extra + (gv.len() - n_exact);
            }
        }
    }
    assert_eq!(bad, 0, "underground candidates must match the game exactly");
}
