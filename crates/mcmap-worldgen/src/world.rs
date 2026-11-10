//! One Overworld seed: surface biomes, the terrain shape functions, and biomes at any height.

use crate::climate::{quantize, ClimateIds, ClimateSampler};
use crate::density::{Arena, Builder, Df, Files, Memo, NodeId};
use crate::random::world_positional;
use crate::terrain::{Terrain, TerrainIds};
use crate::{tree_for_dimension, BiomeId, Surface};
use mcmap_data::pack::RawPack;
use mcmap_data::{Dimension, Preset};
use std::sync::Arc;

#[derive(Clone)]
pub struct World {
    pub seed: i64,
    pub surface: Surface,
    /// The Overworld's terrain shape; `None` in the Nether and the End, whose biomes are 2-D.
    pub terrain: Option<Terrain>,
    pub dimension: Dimension,
}

/// Sea level of the Overworld preset.
pub const SEA_LEVEL: i32 = 63;

/// Measured on 26.3: the game's heightmaps sit about 13 blocks above the preliminary surface
/// level, give or take 7 (the 3-D noise it leaves out). See `examples/heights.rs`.
const SURFACE_BIAS: i32 = 13;

impl World {
    pub fn new(pack: &RawPack, seed: i64) -> Result<World, String> {
        World::for_dimension(pack, seed, Dimension::Overworld)
    }

    pub fn for_dimension(pack: &RawPack, seed: i64, dimension: Dimension) -> Result<World, String> {
        World::for_world(pack, seed, dimension, Preset::Normal)
    }

    pub fn for_world(pack: &RawPack, seed: i64, dimension: Dimension, preset: Preset) -> Result<World, String> {
        let (world, _) = World::for_world_with_spawn(pack, seed, dimension, preset, false)?;
        Ok(world)
    }

    /// Build [`World`] and (when `with_spawn` is true in the Overworld) [`SpawnFinder`] in a
    /// single shared density-function [`Arena`] so noise instances and subgraphs are instantiated once.
    pub fn for_world_with_spawn(
        pack: &RawPack,
        seed: i64,
        dimension: Dimension,
        preset: Preset,
        with_spawn: bool,
    ) -> Result<(World, Option<SpawnFinder>), String> {
        let (tree, names) = tree_for_dimension(pack, dimension)?;
        if dimension == Dimension::Overworld {
            let mut b = Builder::new(&pack.files, world_positional(seed));
            b.seed = seed;
            let c_ids = ClimateIds::prepare(&mut b, preset.settings())?;
            let t_ids = TerrainIds::prepare(&mut b, preset)?;
            let s_ids = if with_spawn { Some(SpawnIds::prepare(&mut b, preset)?) } else { None };
            let arena = b.finish();
            let surface = Surface::from_parts(c_ids.into_sampler(arena.clone()), tree, names, dimension);
            let terrain = Some(t_ids.into_terrain(arena.clone()));
            let spawn = s_ids.map(|s| s.into_finder(arena));
            Ok((World { seed, surface, terrain, dimension }, spawn))
        } else {
            let settings = match dimension {
                Dimension::Overworld => unreachable!(),
                Dimension::Nether => "minecraft:nether",
                Dimension::End => "minecraft:end",
            };
            let climate = ClimateSampler::from_settings(&pack.files, seed, settings)?;
            let surface = Surface::from_parts(climate, tree, names, dimension);
            Ok((World { seed, surface, terrain: None, dimension }, None))
        }
    }

    /// Biome at quart coordinates, at that height (`MultiNoiseBiomeSource.getNoiseBiome`).
    #[inline]
    pub fn biome(&self, qx: i32, qy: i32, qz: i32, hint: &mut Option<u32>) -> BiomeId {
        let Some(terrain) = &self.terrain else { return self.surface.biome_with_hint(qx, qz, hint) };
        let (x, y, z) = (qx << 2, qy << 2, qz << 2);
        let (v, d) = if self.surface.climate.shares_arena_with(terrain.depth_df()) {
            let mut memo = Memo::new(y);
            let v = self.surface.climate.raw_memo(x, y, z, &mut memo);
            let d = terrain.depth_memo(x, y, z, &mut memo);
            (v, d)
        } else {
            (self.surface.climate.raw(x, y, z), terrain.depth(x, y, z))
        };
        let t = [quantize(v[0]), quantize(v[1]), quantize(v[2]), quantize(v[3]), quantize(d), quantize(v[4])];
        BiomeId(self.surface.tree.search(&t, hint))
    }

    /// Biome at a block position (its quart).
    pub fn biome_at(&self, x: i32, y: i32, z: i32) -> BiomeId {
        let mut h = None;
        self.biome(x >> 2, y >> 2, z >> 2, &mut h)
    }

    /// Estimated `getBaseHeight(WORLD_SURFACE_WG)`: the first free block above the terrain or the sea.
    pub fn surface_height(&self, x: i32, z: i32) -> i32 {
        match &self.terrain {
            Some(t) => (t.surface(x, z) + SURFACE_BIAS).max(SEA_LEVEL),
            None => SEA_LEVEL,
        }
    }

    /// Estimated `getBaseHeight(OCEAN_FLOOR_WG)`: the first non-solid block above the terrain.
    pub fn floor_height(&self, x: i32, z: i32) -> i32 {
        match &self.terrain {
            Some(t) => t.surface(x, z) + SURFACE_BIAS,
            None => SEA_LEVEL,
        }
    }
}

struct SpawnIds {
    targets: Vec<Vec<(NodeId, i64, i64)>>,
}

impl SpawnIds {
    fn prepare(b: &mut Builder, preset: Preset) -> Result<SpawnIds, String> {
        let settings = b.json("noise_settings", preset.settings())?;
        let mut targets = Vec::new();
        for t in settings["spawn_target"].as_array().ok_or("noise settings without spawn_target")? {
            let mut params = Vec::new();
            for (id, range) in t.as_object().ok_or("spawn target is not an object")? {
                let f = b.build_id(&serde_json::Value::String(id.clone()))?;
                let lo = range[0].as_f64().ok_or("spawn target range")? as f32;
                let hi = range[1].as_f64().ok_or("spawn target range")? as f32;
                params.push((f, quantize(lo), quantize(hi)));
            }
            targets.push(params);
        }
        Ok(SpawnIds { targets })
    }

    fn into_finder(self, arena: Arc<Arena>) -> SpawnFinder {
        let targets = self
            .targets
            .into_iter()
            .map(|t| t.into_iter().map(|(id, lo, hi)| (Df::from_parts(arena.clone(), id), lo, hi)).collect())
            .collect();
        SpawnFinder { targets }
    }
}

/// `NoiseSpawnFinder`: the world spawn's search point, from the noise settings' spawn targets.
/// The server then picks a grass block in that chunk area, which this does not model.
#[derive(Clone)]
pub struct SpawnFinder {
    /// Per target point: (function, quantized min, quantized max) for each listed parameter.
    targets: Vec<Vec<(Df, i64, i64)>>,
}

impl SpawnFinder {
    pub fn overworld(files: &Files, seed: i64) -> Result<SpawnFinder, String> {
        SpawnFinder::new(files, seed, Preset::Normal)
    }

    pub fn new(files: &Files, seed: i64, preset: Preset) -> Result<SpawnFinder, String> {
        let mut b = Builder::new(files, world_positional(seed));
        b.seed = seed;
        let ids = SpawnIds::prepare(&mut b, preset)?;
        Ok(ids.into_finder(b.finish()))
    }

    fn fitness(&self, x: i32, z: i32) -> i64 {
        let (sx, sz) = ((x >> 2) << 2, (z >> 2) << 2);
        let mut memo = Memo::new(0);
        let mut best = i64::MAX;
        for t in &self.targets {
            let mut sum = 0i64;
            for (f, lo, hi) in t {
                let v = quantize(f.eval_memo(sx, 0, sz, &mut memo));
                let d = if v - hi > 0 { v - hi } else { (lo - v).max(0) };
                sum = sum.wrapping_add(d.wrapping_mul(d));
            }
            best = best.min(sum);
        }
        let dist = (x as i64) * (x as i64) + (z as i64) * (z as i64);
        best.wrapping_mul(2048 * 2048).wrapping_add(dist)
    }

    /// The spawn search point (block x, z).
    pub fn find(&self) -> (i32, i32) {
        let mut best = ((0, 0), self.fitness(0, 0));
        for (max, step) in [(2048.0f32, 512.0f32), (512.0, 32.0)] {
            let (bx, bz) = best.0;
            let (mut angle, mut dist) = (0.0f32, step);
            while dist <= max {
                let x = bx + ((angle as f64).sin() * dist as f64) as i32;
                let z = bz + ((angle as f64).cos() * dist as f64) as i32;
                let f = self.fitness(x, z);
                if f < best.1 {
                    best = ((x, z), f);
                }
                angle += step / dist;
                if angle as f64 > std::f64::consts::TAU {
                    angle = 0.0;
                    dist += step;
                }
            }
        }
        best.0
    }
}
