use mcmap_data::pack::{pack_path, RawPack};
use mcmap_data::{Dimension, Preset};
use mcmap_worldgen::structure::{Placement, Structures};
use mcmap_worldgen::terrain::Terrain;
use mcmap_worldgen::world::{SpawnFinder, World};
use mcmap_worldgen::Surface;
use std::path::PathBuf;

const SEED: i64 = -7556262131344217808;

fn load_pack() -> Option<RawPack> {
    let p = std::env::var_os("MCMAP_PACK").map(PathBuf::from).unwrap_or_else(|| pack_path("26.3"));
    RawPack::read(&p).ok()
}

struct Hasher(u64);
impl Hasher {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn i32(&mut self, v: i32) {
        self.u64(v as u32 as u64);
    }
    fn u16(&mut self, v: u16) {
        self.u64(v as u64);
    }
    fn f32(&mut self, v: f32) {
        self.u64(v.to_bits() as u64);
    }
}

fn compute_fingerprints(pack: &RawPack) -> [u64; 8] {
    // 0: Overworld Normal + LargeBiomes + Amplified surface biomes
    let mut h0 = Hasher::new();
    for preset in [Preset::Normal, Preset::LargeBiomes, Preset::Amplified] {
        let s = Surface::for_world(pack, SEED, Dimension::Overworld, preset).unwrap();
        let mut hint = None;
        for qz in (-256..256).step_by(7) {
            for qx in (-256..256).step_by(5) {
                h0.u16(s.biome_with_hint(qx, qz, &mut hint).0);
            }
        }
    }

    // 1: Nether surface biomes
    let mut h1 = Hasher::new();
    {
        let s = Surface::for_dimension(pack, SEED, Dimension::Nether).unwrap();
        let mut hint = None;
        for qz in (-200..200).step_by(5) {
            for qx in (-200..200).step_by(5) {
                h1.u16(s.biome_with_hint(qx, qz, &mut hint).0);
            }
        }
    }

    // 2: End surface biomes + erosion density across negative/positive 64-lattice boundaries
    let mut h2 = Hasher::new();
    {
        let s = Surface::for_dimension(pack, SEED, Dimension::End).unwrap();
        let mut hint = None;
        for qz in (-600..600).step_by(9) {
            for qx in (-600..600).step_by(9) {
                h2.u16(s.biome_with_hint(qx, qz, &mut hint).0);
            }
        }
        // Fine step around outer islands crossing block boundaries (-1500..-900 and 900..1500)
        for z in (-1400..-950).step_by(3).chain((950..1400).step_by(3)) {
            for x in (-1400..-950).step_by(5).chain((950..1400).step_by(5)) {
                h2.f32(s.climate.erosion().eval(x, 64, z));
            }
        }
    }

    // 3: Terrain depth, offset, factor, surface (bit-exact f32::to_bits + i32)
    let mut h3 = Hasher::new();
    {
        let t = Terrain::new(&pack.files, SEED, Preset::Normal).unwrap();
        for z in (-512..512).step_by(29) {
            for x in (-512..512).step_by(31) {
                h3.f32(t.offset(x, z));
                h3.f32(t.factor(x, z));
                h3.i32(t.surface(x, z));
                for y in [-48, 0, 64, 160] {
                    h3.f32(t.depth(x, y, z));
                }
            }
        }
    }

    // 4: 3D World::biome across heights
    let mut h4 = Hasher::new();
    let world = World::new(pack, SEED).unwrap();
    {
        let mut hint = None;
        for qy in [-12, 0, 16, 40] {
            for qz in (-128..128).step_by(7) {
                for qx in (-128..128).step_by(7) {
                    h4.u16(world.biome(qx, qy, qz, &mut hint).0);
                }
            }
        }
    }

    // 5: SpawnFinder across 3 seeds
    let mut h5 = Hasher::new();
    for s in [SEED, 12345i64, -1i64] {
        let (sx, sz) = SpawnFinder::overworld(&pack.files, s).unwrap().find();
        h5.i32(sx);
        h5.i32(sz);
    }

    // 6: Stronghold rings (all 128 strongholds)
    let mut h6 = Hasher::new();
    let structures = Structures::new(&pack.files, world.surface.names()).unwrap();
    for set in &structures.sets {
        if matches!(set.placement, Placement::Rings { .. }) {
            for (cx, cz) in structures.rings(&world, set) {
                h6.i32(cx);
                h6.i32(cz);
            }
        }
    }

    // 7: Random-spread structures in region (0, -1)
    let mut h7 = Hasher::new();
    for set in &structures.sets {
        for f in structures.in_area(&world, set, 0, -64, 63, -1) {
            h7.i32(f.chunk.0);
            h7.i32(f.chunk.1);
            h7.i32(f.pos.0);
            h7.i32(f.pos.1);
            h7.i32(f.pos.2);
            h7.i32(f.marker.0);
            h7.i32(f.marker.1);
            h7.u64(f.likely as u64);
        }
    }

    [h0.0, h1.0, h2.0, h3.0, h4.0, h5.0, h6.0, h7.0]
}

#[test]
fn phase1_bit_exact_worldgen() {
    let Some(pack) = load_pack() else { return };
    let got = compute_fingerprints(&pack);
    let expected: [u64; 8] = [
        0xd3113a560b0dee69,
        0x4c850021a11976c5,
        0x518454f0bc57e0f0,
        0x01756977652a1afa,
        0x593a65620bed944b,
        0x31441cf16cb62df4,
        0x40d5e98683b80f20,
        0x39165ea4ddcc4a22,
    ];
    assert_eq!(got, expected);
}
