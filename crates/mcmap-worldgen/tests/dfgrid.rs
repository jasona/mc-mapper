//! Diff density functions against the game: grids written by `oracle dfgrid` into
//! `$MCMAP_DF/<function>_<y>.bin` with a sidecar `<same>.txt` holding "seed x0 z0 step".
//!
//! ```sh
//! cargo xtask oracle dfgrid <seed> minecraft:overworld/depth -3000 -3000 200 200 31 64 $DIR/depth_64.bin
//! MCMAP_PACK=... MCMAP_DF=$DIR cargo test -p mcmap-worldgen --release --test dfgrid -- --ignored --nocapture
//! ```

use mcmap_data::pack::RawPack;
use mcmap_worldgen::terrain::Terrain;

#[test]
#[ignore]
fn density_functions_match_the_game() {
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").expect("MCMAP_PACK"))).unwrap();
    let dir = std::path::PathBuf::from(std::env::var("MCMAP_DF").expect("MCMAP_DF"));
    let mut total_bad = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "bin") {
            continue;
        }
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
        let (name, y) = stem.rsplit_once('_').unwrap();
        let y: i32 = y.parse().unwrap();
        let meta: Vec<i64> = std::fs::read_to_string(p.with_extension("txt")).unwrap().split_whitespace().map(|s| s.parse().unwrap()).collect();
        let (seed, x0, z0, step) = (meta[0], meta[1] as i32, meta[2] as i32, meta[3] as i32);
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[..4], b"MCDF");
        let w = u32::from_be_bytes(b[4..8].try_into().unwrap()) as usize;
        let h = u32::from_be_bytes(b[8..12].try_into().unwrap()) as usize;
        let f = Terrain::function(&pack.files, seed, &format!("minecraft:overworld/{name}")).unwrap();
        let t = std::time::Instant::now();
        let (mut bad, mut worst) = (0, 0f32);
        for j in 0..h {
            for i in 0..w {
                let o = 12 + (j * w + i) * 4;
                let want = f32::from_be_bytes(b[o..o + 4].try_into().unwrap());
                let got = f.eval(x0 + i as i32 * step, y, z0 + j as i32 * step);
                if got.to_bits() != want.to_bits() {
                    if bad < 3 {
                        println!("  {name} y {y} at ({}, {}): got {got} want {want}", x0 + i as i32 * step, z0 + j as i32 * step);
                    }
                    bad += 1;
                    worst = worst.max((got - want).abs());
                }
            }
        }
        println!("{name} y {y}: {}/{} bit-exact, worst diff {worst}, {:.1} us per value", w * h - bad, w * h, t.elapsed().as_secs_f64() * 1e6 / (w * h) as f64);
        total_bad += bad;
    }
    assert_eq!(total_bad, 0);
}
