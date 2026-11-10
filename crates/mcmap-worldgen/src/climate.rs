//! The climate sampler: five router entries evaluated at a block position and quantized the way
//! `Climate.quantizeCoord` does (`(long) (value * 10000f)`).

use crate::density::{Arena, Builder, Df, Files, Memo, NodeId};
use crate::random::world_positional;
use std::sync::Arc;

/// Quantized climate: temperature, humidity, continentalness, erosion, depth, weirdness.
pub type Target = [i64; 6];

#[inline(always)]
pub fn quantize(v: f32) -> i64 {
    (v * 10000.0) as i64
}

pub struct ClimateIds {
    temperature: NodeId,
    humidity: NodeId,
    continentalness: NodeId,
    erosion: NodeId,
    weirdness: NodeId,
}

impl ClimateIds {
    pub fn prepare(b: &mut Builder, settings: &str) -> Result<ClimateIds, String> {
        let settings = b.json("noise_settings", settings)?;
        let router = &settings["noise_router"];
        let mut get = |k: &str| b.build_id(&router[k]).map_err(|e| format!("noise_router.{k}: {e}"));
        Ok(ClimateIds {
            temperature: get("temperature")?,
            humidity: get("vegetation")?,
            continentalness: get("continents")?,
            erosion: get("erosion")?,
            weirdness: get("ridges")?,
        })
    }

    pub fn into_sampler(self, arena: Arc<Arena>) -> ClimateSampler {
        ClimateSampler {
            temperature: Df::from_parts(arena.clone(), self.temperature),
            humidity: Df::from_parts(arena.clone(), self.humidity),
            continentalness: Df::from_parts(arena.clone(), self.continentalness),
            erosion: Df::from_parts(arena.clone(), self.erosion),
            weirdness: Df::from_parts(arena, self.weirdness),
        }
    }
}

#[derive(Clone)]
pub struct ClimateSampler {
    temperature: Df,
    humidity: Df,
    continentalness: Df,
    erosion: Df,
    weirdness: Df,
}

impl ClimateSampler {
    /// The Overworld router of the pack, for `seed`.
    pub fn overworld(files: &Files, seed: i64) -> Result<ClimateSampler, String> {
        ClimateSampler::from_settings(files, seed, "minecraft:overworld")
    }

    /// The router of any noise settings (`minecraft:nether`, `minecraft:end`, ...).
    pub fn from_settings(files: &Files, seed: i64, settings: &str) -> Result<ClimateSampler, String> {
        let mut b = Builder::new(files, world_positional(seed));
        b.seed = seed;
        let ids = ClimateIds::prepare(&mut b, settings)?;
        Ok(ids.into_sampler(b.finish()))
    }

    /// The five raw values at a block position: temperature, humidity, continentalness,
    /// erosion, weirdness.
    #[inline]
    pub fn raw(&self, x: i32, y: i32, z: i32) -> [f32; 5] {
        let mut memo = Memo::new(y);
        self.raw_memo(x, y, z, &mut memo)
    }

    /// Evaluate the five raw climate values using a caller-supplied [`Memo`] at `(x, z)`.
    #[inline]
    pub fn raw_memo(&self, x: i32, y: i32, z: i32, memo: &mut Memo) -> [f32; 5] {
        [
            self.temperature.eval_memo(x, y, z, memo),
            self.humidity.eval_memo(x, y, z, memo),
            self.continentalness.eval_memo(x, y, z, memo),
            self.erosion.eval_memo(x, y, z, memo),
            self.weirdness.eval_memo(x, y, z, memo),
        ]
    }

    /// The erosion entry, which the End biome source reads on its own.
    pub fn erosion(&self) -> &Df {
        &self.erosion
    }

    pub fn shares_arena_with(&self, df: &Df) -> bool {
        self.temperature.shares_arena(df)
    }

    /// Surface climate at quart coordinates: sampled at y 64 with depth 0.
    #[inline]
    pub fn surface(&self, qx: i32, qz: i32) -> Target {
        let v = self.raw(qx << 2, 64, qz << 2);
        [quantize(v[0]), quantize(v[1]), quantize(v[2]), quantize(v[3]), 0, quantize(v[4])]
    }

    /// Surface climate at quart coordinates reusing a caller-supplied [`Memo`].
    #[inline]
    pub fn surface_memo(&self, qx: i32, qz: i32, memo: &mut Memo) -> Target {
        memo.reset(64);
        let v = self.raw_memo(qx << 2, 64, qz << 2, memo);
        [quantize(v[0]), quantize(v[1]), quantize(v[2]), quantize(v[3]), 0, quantize(v[4])]
    }
}
