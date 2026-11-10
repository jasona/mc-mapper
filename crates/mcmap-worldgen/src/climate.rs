//! The climate sampler: five router entries evaluated at a block position and quantized the way
//! `Climate.quantizeCoord` does (`(long) (value * 10000f)`).

use crate::density::{Builder, Df, Files};
use crate::random::world_positional;

/// Quantized climate: temperature, humidity, continentalness, erosion, depth, weirdness.
pub type Target = [i64; 6];

#[inline]
pub fn quantize(v: f32) -> i64 {
    (v * 10000.0) as i64
}

pub struct ClimateSampler {
    temperature: Df,
    humidity: Df,
    continentalness: Df,
    erosion: Df,
    weirdness: Df,
    /// True when all five are shifted noises over the same shift pair (vanilla's router), so
    /// the shift is computed once per position instead of five times.
    shared_shift: bool,
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
        let settings = b.json("noise_settings", settings)?;
        let router = &settings["noise_router"];
        let mut get = |k: &str| b.build(&router[k]).map_err(|e| format!("noise_router.{k}: {e}"));
        let mut c = ClimateSampler {
            temperature: get("temperature")?,
            humidity: get("vegetation")?,
            continentalness: get("continents")?,
            erosion: get("erosion")?,
            weirdness: get("ridges")?,
            shared_shift: false,
        };
        c.shared_shift = {
            let all = c.all();
            match all[0].shifts() {
                Some(s0) => all.iter().all(|d| d.shifts().is_some_and(|s| (0..3).all(|k| s[k].same_as(&s0[k])))),
                None => false,
            }
        };
        Ok(c)
    }

    /// The five raw values at a block position: temperature, humidity, continentalness,
    /// erosion, weirdness.
    pub fn raw(&self, x: i32, y: i32, z: i32) -> [f32; 5] {
        if self.shared_shift {
            let s = self.temperature.shifts().unwrap();
            let (sx, sy, sz) = (s[0].eval(x, y, z), s[1].eval(x, y, z), s[2].eval(x, y, z));
            return self.all().map(|d| d.eval_with_shift(x, y, z, sx, sy, sz).unwrap());
        }
        [
            self.temperature.eval(x, y, z),
            self.humidity.eval(x, y, z),
            self.continentalness.eval(x, y, z),
            self.erosion.eval(x, y, z),
            self.weirdness.eval(x, y, z),
        ]
    }

    /// The erosion entry, which the End biome source reads on its own.
    pub fn erosion(&self) -> &Df {
        &self.erosion
    }

    fn all(&self) -> [&Df; 5] {
        [&self.temperature, &self.humidity, &self.continentalness, &self.erosion, &self.weirdness]
    }

    /// Surface climate at quart coordinates: sampled at y 64 with depth 0.
    pub fn surface(&self, qx: i32, qz: i32) -> Target {
        let v = self.raw(qx << 2, 64, qz << 2);
        [quantize(v[0]), quantize(v[1]), quantize(v[2]), quantize(v[3]), 0, quantize(v[4])]
    }
}
