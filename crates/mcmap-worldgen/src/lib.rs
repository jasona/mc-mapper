//! World generation for the surface map: the game's random sources, noise and climate sampler,
//! evaluated from a version pack, and the biome lookup over the pack's parameter table.
//!
//! The surface biome is evaluated at depth 0: sample the five horizontal climate values at y 64,
//! set depth to 0 and look up the nearest parameter box. On 26.3 that matches the reference
//! capture cell for cell, and this port matches the game's own classes (see `tests/oracle.rs`).

pub mod climate;
pub mod density;
pub mod noise;
pub mod random;
pub mod rtree;

use std::cell::Cell;

/// A biome as an index into [`Surface::names`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BiomeId(pub u16);

/// What the tile renderer needs from world generation: the biome of a 4x4-block cell.
pub trait BiomeSampler: Send + Sync {
    /// Biome of the cell at quart coordinates (block >> 2). `qy` is ignored on the surface layer.
    fn biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId;
}

/// Surface biomes of one Overworld seed.
pub struct Surface {
    pub climate: climate::ClimateSampler,
    pub tree: rtree::RTree,
    names: Vec<String>,
}

thread_local! {
    /// The game keeps the last lookup result per thread and starts the next search from it.
    static LAST: Cell<Option<u32>> = const { Cell::new(None) };
}

impl Surface {
    pub fn new(pack: &mcmap_data::pack::RawPack, seed: i64) -> Result<Surface, String> {
        let params = pack.overworld_parameters.as_deref().ok_or("the pack has no Overworld parameter list; extract it again with the oracle")?;
        let (entries, names) = rtree::parse_parameters(params)?;
        Ok(Surface { climate: climate::ClimateSampler::overworld(&pack.files, seed)?, tree: rtree::RTree::new(entries), names })
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
        BiomeId(self.tree.search(&self.climate.surface(qx, qz), hint))
    }
}

impl BiomeSampler for Surface {
    fn biome(&self, qx: i32, _qy: i32, qz: i32) -> BiomeId {
        LAST.with(|l| {
            let mut h = l.get();
            let b = self.biome_with_hint(qx, qz, &mut h);
            l.set(h);
            b
        })
    }
}
