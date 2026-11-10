//! M3 gate: the reference capture (`art/seed-map/reference/features.png`, seed
//! -7556262131344217808, centre (2839, -1418), 1.25 px per block) shows two trial chambers, two
//! ruined portals and an ancient city. Every icon must have a computed marker within 2 px, and no
//! computed marker of those types may fall in the view without an icon.
//!
//! ```sh
//! MCMAP_PACK=... cargo test -p mcmap-worldgen --release --test reference_view -- --ignored --nocapture
//! ```

use mcmap_data::pack::RawPack;
use mcmap_worldgen::structure::Structures;
use mcmap_worldgen::world::World;

const SEED: i64 = -7556262131344217808;
const W: f64 = 1133.0;
const H: f64 = 566.0;

fn px(x: i32, z: i32) -> (f64, f64) {
    ((x as f64 - 2839.0) * 1.25 + W / 2.0, (z as f64 + 1418.0) * 1.25 + H / 2.0)
}

/// Icon centres measured on the capture.
const ICONS: &[(&str, f64, f64)] = &[
    ("minecraft:trial_chambers", 87.0, 47.0),
    ("minecraft:trial_chambers", 507.0, 127.0),
    ("minecraft:ruined_portal", 327.0, 88.0),
    ("minecraft:ruined_portal", 227.0, 484.0),
    ("minecraft:ancient_city", 927.0, 365.0),
];

#[test]
#[ignore]
fn reference_view_markers() {
    let pack = RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").expect("MCMAP_PACK"))).unwrap();
    let world = World::new(&pack, SEED).unwrap();
    let st = Structures::new(&pack.files, world.surface.names()).unwrap();
    assert!(st.has_geometry(), "the pack has no template geometry; run cargo xtask extract again");
    let (bx0, bx1) = ((2839.0 - W / 2.5) as i32, (2839.0 + W / 2.5) as i32);
    let (bz0, bz1) = ((-1418.0 - H / 2.5) as i32, (-1418.0 + H / 2.5) as i32);
    let mut got = Vec::new();
    for set in &st.sets {
        // Ruined portals come in one set per variant; the capture shows them all as one layer.
        for f in st.in_area(&world, set, (bx0 >> 4) - 2, (bz0 >> 4) - 2, (bx1 >> 4) + 2, (bz1 >> 4) + 2) {
            let id = if f.structure.starts_with("minecraft:ruined_portal") { "minecraft:ruined_portal".to_string() } else { f.structure.clone() };
            if !ICONS.iter().any(|i| i.0 == id) {
                continue;
            }
            let (x, y) = px(f.marker.0, f.marker.1);
            if (0.0..W).contains(&x) && (0.0..H).contains(&y) {
                println!("{id} at {:?}: px ({x:.1}, {y:.1})", f.marker);
                got.push((id, x, y));
            }
        }
    }
    let mut bad = 0;
    for (id, x, y) in ICONS {
        let hit = got.iter().any(|g| g.0 == *id && (g.1 - x).abs() <= 2.0 && (g.2 - y).abs() <= 2.0);
        if !hit {
            println!("MISSING {id} at ({x}, {y})");
            bad += 1;
        }
    }
    for g in &got {
        if !ICONS.iter().any(|i| i.0 == g.0 && (g.1 - i.1).abs() <= 2.0 && (g.2 - i.2).abs() <= 2.0) {
            println!("EXTRA {} at ({:.1}, {:.1})", g.0, g.1, g.2);
            bad += 1;
        }
    }
    assert_eq!(bad, 0);
}
