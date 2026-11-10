//! Structures against the game: dumps written by `cargo xtask oracle structs <seed> <cx0> <cz0>
//! <cx1> <cz1> $MCMAP_STRUCTS/<name>.ndjson` (the game's own placement, selection and
//! `Structure.generate`). Placement and ring positions must match exactly; generation results
//! are reported per structure set, with a floor on agreement.
//!
//! ```sh
//! MCMAP_PACK=... MCMAP_STRUCTS=$DIR cargo test -p mcmap-worldgen --release --test structures -- --ignored --nocapture
//! ```

use mcmap_data::pack::RawPack;
use mcmap_worldgen::structure::{Placement, Structures};
use mcmap_worldgen::world::World;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Tally {
    placements: usize,
    placement_misses: usize,
    agree: usize,
    disagree: usize,
    false_pos: usize,
    false_neg: usize,
    wrong_type: usize,
    xz_exact: usize,
    y_err: Vec<i32>,
}

#[test]
#[ignore]
fn structures_match_the_game() {
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").expect("MCMAP_PACK"))).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("MCMAP_STRUCTS").expect("MCMAP_STRUCTS"));
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut ring_bad = 0;
    let verbose = std::env::var("MCMAP_VERBOSE").is_ok();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "ndjson") {
            continue;
        }
        let lines: Vec<Value> = std::fs::read_to_string(&p).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let meta = &lines[0];
        assert!(meta["meta"].as_bool().unwrap_or(false), "{}: no meta line; dump it again", p.display());
        let seed = meta["seed"].as_i64().unwrap();
        let g = |k: &str| meta[k].as_i64().unwrap() as i32;
        let (cx0, cz0, cx1, cz1) = (g("cx0"), g("cz0"), g("cx1"), g("cz1"));
        let dim = meta["dimension"].as_str().and_then(mcmap_data::Dimension::from_key).unwrap_or(mcmap_data::Dimension::Overworld);
        let world = World::for_dimension(&pack, seed, dim).unwrap();
        if let (Some(sx), Some(sz)) = (meta["spawn_x"].as_i64(), meta["spawn_z"].as_i64()) {
            let got = mcmap_worldgen::world::SpawnFinder::overworld(&pack.files, seed).unwrap().find();
            println!("  spawn: got {got:?} want ({sx}, {sz})");
            ring_bad += usize::from(got != (sx as i32, sz as i32));
        }
        let st = Structures::new(&pack.files, world.surface.names()).unwrap();
        let t = std::time::Instant::now();
        println!("{}: seed {seed}, chunks {cx0}..{cx1} x {cz0}..{cz1}", p.file_name().unwrap().to_string_lossy());
        // The game's answers, by set and chunk.
        let mut want: BTreeMap<(String, i32, i32), (String, Value)> = BTreeMap::new();
        let mut rings: BTreeMap<String, Vec<(i32, i32)>> = BTreeMap::new();
        for l in &lines[1..] {
            if l.get("height").is_some() {
                continue;
            }
            let set = l["set"].as_str().unwrap().to_owned();
            let (cx, cz) = (l["cx"].as_i64().unwrap() as i32, l["cz"].as_i64().unwrap() as i32);
            if l.get("ring").is_some() {
                rings.entry(set).or_default().push((cx, cz));
            } else {
                want.insert((set, cx, cz), (l["result"].as_str().unwrap().to_owned(), l.clone()));
            }
        }
        for set in &st.sets {
            let ta = tallies.entry(set.id.clone()).or_default();
            if let Placement::Rings { .. } = set.placement {
                if dim != mcmap_data::Dimension::Overworld {
                    continue;
                }
                let got = st.rings(&world, set);
                let w = rings.get(&set.id).cloned().unwrap_or_default();
                let bad = got.iter().zip(&w).filter(|(a, b)| a != b).count() + got.len().abs_diff(w.len());
                println!("  {} rings: {}/{} positions match", set.id, w.len() - bad.min(w.len()), w.len());
                if bad > 0 && verbose {
                    for (a, b) in got.iter().zip(&w).filter(|(a, b)| a != b).take(5) {
                        println!("    got {a:?} want {b:?}");
                    }
                }
                ring_bad += bad;
                continue;
            }
            // The oracle lists only the sets that can occur in the dimension.
            if dim != mcmap_data::Dimension::Overworld && !want.keys().any(|k| k.0 == set.id) {
                continue;
            }
            let mut ours = BTreeSet::new();
            for cz in cz0..=cz1 {
                for cx in cx0..=cx1 {
                    if st.is_placement_chunk(set, seed, cx, cz) {
                        ours.insert((cx, cz));
                    }
                }
            }
            let theirs: BTreeSet<(i32, i32)> = want.keys().filter(|k| k.0 == set.id).map(|k| (k.1, k.2)).collect();
            if set.entries.iter().all(|e| matches!(e.0.kind, mcmap_worldgen::structure::Kind::Other(_))) {
                continue; // Structures the engine doesn't model (nether fossils).
            }
            ta.placements += theirs.len();
            ta.placement_misses += ours.symmetric_difference(&theirs).count();
            for &(cx, cz) in &theirs {
                let (res, line) = &want[&(set.id.clone(), cx, cz)];
                let got = st.generate(&world, set, cx, cz);
                let gname = got.as_ref().map(|f| f.structure.clone()).unwrap_or_else(|| "none".into());
                if &gname == res {
                    ta.agree += 1;
                    if let Some(f) = &got {
                        let (wx, wy, wz) = (line["x"].as_i64().unwrap_or(0) as i32, line["y"].as_i64().unwrap_or(0) as i32, line["z"].as_i64().unwrap_or(0) as i32);
                        if f.pos.0 == wx && f.pos.2 == wz {
                            ta.xz_exact += 1;
                        } else if verbose {
                            println!("    {} ({cx},{cz}) {gname}: position {:?} want {wx},{wy},{wz}", set.id, f.pos);
                        }
                        ta.y_err.push(f.pos.1 - wy);
                    }
                } else {
                    ta.disagree += 1;
                    match (res.as_str(), gname.as_str()) {
                        ("none", _) => ta.false_pos += 1,
                        (_, "none") => ta.false_neg += 1,
                        _ => ta.wrong_type += 1,
                    }
                    if verbose {
                        println!("    {} ({cx},{cz}): got {gname} {:?} want {res} at {},{},{}", set.id, got.map(|f| f.pos), line["x"], line["y"], line["z"]);
                    }
                }
            }
        }
        println!("  took {:.1} s", t.elapsed().as_secs_f64());
    }
    println!("\n{:<32} {:>6} {:>6} {:>7} {:>5} {:>5} {:>5} {:>6} {:>9}", "set", "cands", "plcmiss", "agree", "fp", "fn", "type", "xz ok", "|dy| p90");
    let (mut miss, mut agree, mut total) = (0, 0, 0);
    for (k, t) in &tallies {
        if t.placements == 0 {
            continue;
        }
        let n = t.agree + t.disagree;
        let mut dy: Vec<i32> = t.y_err.iter().map(|d| d.abs()).collect();
        dy.sort();
        let p90 = dy.get(dy.len() * 9 / 10).copied().unwrap_or(0);
        let found = t.y_err.len();
        println!(
            "{:<32} {:>6} {:>6} {:>6.1}% {:>5} {:>5} {:>5} {:>6} {:>9}",
            k,
            t.placements,
            t.placement_misses,
            100.0 * t.agree as f64 / n.max(1) as f64,
            t.false_pos,
            t.false_neg,
            t.wrong_type,
            format!("{}/{found}", t.xz_exact),
            p90
        );
        miss += t.placement_misses;
        agree += t.agree;
        total += n;
    }
    println!("overall agreement {:.2}% of {total}", 100.0 * agree as f64 / total.max(1) as f64);
    assert_eq!(miss, 0, "placement must match the game exactly");
    assert_eq!(ring_bad, 0, "stronghold rings must match the game exactly");
}
