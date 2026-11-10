//! Fit the End city height stand-in: the islands density at the four corners of the 5 x 5 box.
use mcmap_data::pack::RawPack;
use mcmap_worldgen::random::Legacy;
use mcmap_worldgen::Surface;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let pack = RawPack::read(std::path::Path::new(&a[1])).unwrap();
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&a[2]).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let seed = lines[0]["seed"].as_i64().unwrap();
    let s = Surface::for_dimension(&pack, seed, mcmap_data::Dimension::End).unwrap();
    let mut pts = Vec::new();
    for l in &lines[1..] {
        if l["set"] != "minecraft:end_cities" { continue; }
        let (cx, cz) = (l["cx"].as_i64().unwrap() as i32, l["cz"].as_i64().unwrap() as i32);
        let yes = l["result"] != "none";
        let mut r = Legacy::large_feature(seed, cx, cz);
        let rot = r.next_int_bounded(4);
        let (x, z) = ((cx << 4) + 7, (cz << 4) + 7);
        let (dx, dz) = match rot { 1 => (-5, 5), 2 => (-5, -5), 3 => (5, -5), _ => (5, 5) };
        let m = [(x, z), (x, z + dz), (x + dx, z), (x + dx, z + dz)].iter().map(|&(a, b)| s.climate.erosion().eval(a, 64, b)).fold(f32::MAX, f32::min);
        pts.push((m, yes));
    }
    for t in [-0.3f32, -0.2, -0.1, -0.05, 0.0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4] {
        let ok = pts.iter().filter(|(m, y)| (*m > t) == *y).count();
        let fneg = pts.iter().filter(|(m, y)| *y && *m <= t).count();
        println!("t {t}: {ok}/{} agree, {fneg} false negatives", pts.len());
    }
}
