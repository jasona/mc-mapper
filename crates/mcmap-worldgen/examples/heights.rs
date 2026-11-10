//! Compare the preliminary surface level with the game's heightmaps from an oracle `structs` dump.
use mcmap_data::pack::RawPack;
use mcmap_worldgen::terrain::Terrain;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").unwrap())).unwrap();
    let seed: i64 = a[1].parse().unwrap();
    let t = Terrain::overworld(&pack.files, seed).unwrap();
    let mut hist = std::collections::BTreeMap::new();
    let mut hist2 = std::collections::BTreeMap::new();
    for f in &a[2..] {
        for l in std::fs::read_to_string(f).unwrap().lines() {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            if v.get("height").is_none() { continue; }
            let (x, z) = (v["x"].as_i64().unwrap() as i32, v["z"].as_i64().unwrap() as i32);
            let (ws, of) = (v["ws"].as_i64().unwrap() as i32, v["of"].as_i64().unwrap() as i32);
            let p = t.surface(x, z);
            *hist.entry(of - p).or_insert(0) += 1;
            *hist2.entry(ws - p.max(63)).or_insert(0) += 1;
        }
    }
    println!("of - prelim: {hist:?}");
    println!("ws - max(prelim,63): {hist2:?}");
}
