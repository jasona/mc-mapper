//! Timing of what the map computes per seed and per region.
use mcmap_data::pack::RawPack;
use mcmap_worldgen::structure::{Placement, Structures};
use mcmap_worldgen::world::{SpawnFinder, World};
use std::time::Instant;
fn main() {
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").unwrap())).unwrap();
    let seed = -7556262131344217808i64;
    let t = Instant::now();
    let w = World::new(&pack, seed).unwrap();
    println!("world {:?}", t.elapsed());
    let t = Instant::now();
    let st = Structures::new(&pack.files, w.surface.names()).unwrap();
    println!("structures {:?}", t.elapsed());
    let t = Instant::now();
    println!("spawn {:?} in {:?}", SpawnFinder::overworld(&pack.files, seed).unwrap().find(), t.elapsed());
    for s in &st.sets {
        if matches!(s.placement, Placement::Rings { .. }) {
            let t = Instant::now();
            let r = st.rings(&w, s);
            println!("{} rings {} in {:?}", s.id, r.len(), t.elapsed());
        }
    }
    let t = Instant::now();
    let mut n = 0;
    for s in &st.sets {
        n += st.in_area(&w, s, 0, 0, 63, 63).len();
    }
    println!("one 64x64-chunk region: {n} structures in {:?}", t.elapsed());
}
