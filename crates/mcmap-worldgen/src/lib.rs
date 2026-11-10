//! World generation: random sources, noise, splines, the density-function interpreter and biome
//! lookup, evaluated from a version pack. M1 builds it; M0 fixes only the seam the tile renderer
//! calls through.
//!
//! The surface biome is evaluated at depth 0: sample the five horizontal climate values, set
//! depth to 0 and look up the nearest parameter box. On 26.3 that matches the reference capture
//! cell for cell (25,538 cells, checked against the game's own classes in M0).

/// A biome as an index into the version pack's biome list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BiomeId(pub u16);

/// What the tile renderer needs from world generation: the biome of a 4x4-block cell.
pub trait BiomeSampler: Send + Sync {
    /// Biome of the cell at quart coordinates (block >> 2). `qy` is ignored on the surface layer.
    fn biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId;
}
