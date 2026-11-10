//! World generation for the surface map: the game's random sources, noise and climate sampler,
//! evaluated from a version pack, and the biome lookup over the pack's parameter table.
//!
//! The surface biome is evaluated at depth 0: sample the five horizontal climate values at y 64,
//! set depth to 0 and look up the nearest parameter box. On 26.3 that matches the reference
//! capture cell for cell, and this port matches the game's own classes (see `tests/oracle.rs`).

pub mod climate;
pub mod decoration;
pub mod density;
pub mod noise;
pub mod random;
pub mod rtree;
pub mod terrain;
pub mod structure;
pub mod world;

use std::cell::Cell;

/// A biome as an index into [`Surface::names`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BiomeId(pub u16);

/// What the tile renderer needs from world generation: the biome of a 4x4-block cell.
pub trait BiomeSampler: Send + Sync {
    /// Biome of the cell at quart coordinates (block >> 2). `qy` is ignored on the surface layer.
    fn biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId;
}

/// Surface biomes of one seed in one dimension. The Overworld and the Nether are multi-noise
/// lookups; the End is `TheEndBiomeSource` (the central island, then the islands' erosion).
pub struct Surface {
    pub climate: climate::ClimateSampler,
    pub tree: rtree::RTree,
    names: Vec<String>,
    pub dimension: mcmap_data::Dimension,
    /// Tells this surface's search hint apart from another surface's on the same thread.
    id: u64,
}

fn next_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `TheEndBiomeSource`'s biomes, in the order [`Surface::names`] lists them for the End.
const END_BIOMES: [&str; 5] = ["minecraft:the_end", "minecraft:end_highlands", "minecraft:end_midlands", "minecraft:small_end_islands", "minecraft:end_barrens"];

thread_local! {
    /// The game keeps the last lookup result per thread and starts the next search from it. The
    /// hint belongs to one surface: another surface's index may not exist in this tree.
    static LAST: Cell<(u64, Option<u32>)> = const { Cell::new((0, None)) };
}

impl Surface {
    pub fn new(pack: &mcmap_data::pack::RawPack, seed: i64) -> Result<Surface, String> {
        let params = pack.overworld_parameters.as_deref().ok_or("the pack has no Overworld parameter list; extract it again with the oracle")?;
        let (entries, names) = rtree::parse_parameters(params)?;
        Ok(Surface { climate: climate::ClimateSampler::overworld(&pack.files, seed)?, tree: rtree::RTree::new(entries), names, dimension: mcmap_data::Dimension::Overworld, id: next_id() })
    }

    /// Biomes of `dimension`. Nether needs a pack extracted with M4's oracle (its table is built
    /// in code, like the Overworld's).
    pub fn for_dimension(pack: &mcmap_data::pack::RawPack, seed: i64, dimension: mcmap_data::Dimension) -> Result<Surface, String> {
        Surface::for_world(pack, seed, dimension, mcmap_data::Preset::Normal)
    }

    /// Biomes of `dimension` under a world preset (Large Biomes and Amplified change only the
    /// Overworld).
    pub fn for_world(pack: &mcmap_data::pack::RawPack, seed: i64, dimension: mcmap_data::Dimension, preset: mcmap_data::Preset) -> Result<Surface, String> {
        use mcmap_data::Dimension;
        match dimension {
            Dimension::Overworld => {
                let params = pack.overworld_parameters.as_deref().ok_or("the pack has no Overworld parameter list; extract it again with the oracle")?;
                let (entries, names) = rtree::parse_parameters(params)?;
                let climate = climate::ClimateSampler::from_settings(&pack.files, seed, preset.settings())?;
                Ok(Surface { climate, tree: rtree::RTree::new(entries), names, dimension, id: next_id() })
            }
            Dimension::Nether => {
                let params = pack.files.get(mcmap_data::pack::NETHER_PARAMETERS).ok_or("the pack has no Nether table; run `cargo run -q -p xtask -- extract --version 26.3` again")?;
                let (entries, names) = rtree::parse_parameters(params)?;
                Ok(Surface { climate: climate::ClimateSampler::from_settings(&pack.files, seed, "minecraft:nether")?, tree: rtree::RTree::new(entries), names, dimension, id: next_id() })
            }
            Dimension::End => {
                // The tree is never searched in the End; it only keeps the type uniform.
                let fake: Vec<String> = END_BIOMES.iter().map(|b| {
                    format!(r#"{{"biome":"{b}","temperature":[0,0],"humidity":[0,0],"continentalness":[0,0],"erosion":[0,0],"depth":[0,0],"weirdness":[0,0],"offset":0}}"#)
                }).collect();
                let (entries, names) = rtree::parse_parameters(&format!(r#"{{"entries":[{}]}}"#, fake.join(",")))?;
                Ok(Surface { climate: climate::ClimateSampler::from_settings(&pack.files, seed, "minecraft:end")?, tree: rtree::RTree::new(entries), names, dimension, id: next_id() })
            }
        }
    }

    /// `TheEndBiomeSource.getNoiseBiome`.
    fn end_biome(&self, qx: i32, qz: i32) -> BiomeId {
        let (x, z) = (qx << 2, qz << 2);
        let (sx, sz) = ((x >> 4) as i64, (z >> 4) as i64);
        if sx * sx + sz * sz <= 4096 {
            return BiomeId(0);
        }
        let (ex, ez) = (((x >> 4) * 2 + 1) * 8, ((z >> 4) * 2 + 1) * 8);
        let d = self.climate.erosion().eval(ex, 64, ez) as f64;
        BiomeId(if d > 0.25 {
            1
        } else if d >= -0.0625 {
            2
        } else if d < -0.21875 {
            3
        } else {
            4
        })
    }

    /// Biome ids, e.g. `minecraft:plains`, indexed by [`BiomeId`].
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn id_of(&self, name: &str) -> Option<BiomeId> {
        self.names.iter().position(|n| n == name).map(|i| BiomeId(i as u16))
    }

    /// Biome at quart coordinates with an explicit search hint.
    pub fn biome_with_hint(&self, qx: i32, qz: i32, hint: &mut Option<u32>) -> BiomeId {
        if self.dimension == mcmap_data::Dimension::End {
            return self.end_biome(qx, qz);
        }
        BiomeId(self.tree.search(&self.climate.surface(qx, qz), hint))
    }
}

impl BiomeSampler for Surface {
    fn biome(&self, qx: i32, _qy: i32, qz: i32) -> BiomeId {
        LAST.with(|l| {
            let (id, last) = l.get();
            let mut h = if id == self.id { last } else { None };
            let b = self.biome_with_hint(qx, qz, &mut h);
            l.set((self.id, h));
            b
        })
    }
}
