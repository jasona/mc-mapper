//! Terrain shape functions of the Overworld router that the structure checks need: `depth`
//! (for biomes below or above the surface) and the preliminary surface level (an approximate
//! terrain height without the 3-D noise and caves).

use crate::density::{Builder, Df, Files};
use crate::random::world_positional;

pub struct Terrain {
    depth: Df,
    offset: Df,
    factor: Df,
    surface: Df,
}

impl Terrain {
    pub fn overworld(files: &Files, seed: i64) -> Result<Terrain, String> {
        Terrain::new(files, seed, mcmap_data::Preset::Normal)
    }

    pub fn new(files: &Files, seed: i64, preset: mcmap_data::Preset) -> Result<Terrain, String> {
        let mut b = Builder::new(files, world_positional(seed));
        let settings = b.json("noise_settings", preset.settings())?;
        let dir = preset.functions();
        let router = &settings["noise_router"];
        let depth = b.build(&router["depth"]).map_err(|e| format!("noise_router.depth: {e}"))?;
        let surface_ref = settings["aquifers"]["surface_level"].clone();
        let surface_ref = if surface_ref.is_null() { serde_json::json!(format!("{dir}/preliminary_surface_level")) } else { surface_ref };
        let surface = b.build(&surface_ref).map_err(|e| format!("preliminary surface level: {e}"))?;
        let offset = b.build(&serde_json::json!(format!("{dir}/offset")))?;
        let factor = b.build(&serde_json::json!(format!("{dir}/factor")))?;
        Ok(Terrain { depth, offset, factor, surface })
    }

    pub fn depth(&self, x: i32, y: i32, z: i32) -> f32 {
        self.depth.eval(x, y, z)
    }

    pub fn offset(&self, x: i32, z: i32) -> f32 {
        self.offset.eval(x, 0, z)
    }

    pub fn factor(&self, x: i32, z: i32) -> f32 {
        self.factor.eval(x, 0, z)
    }

    /// The preliminary surface level at a block column: on an 8-block grid, a few blocks off
    /// the real surface where the 3-D noise is strong.
    pub fn surface(&self, x: i32, z: i32) -> i32 {
        self.surface.eval(x, 0, z) as i32
    }

    /// Any function the builder knows, by id, for diff tests.
    pub fn function(files: &Files, seed: i64, id: &str) -> Result<Df, String> {
        let mut b = Builder::new(files, world_positional(seed));
        b.build(&serde_json::json!(id))
    }
}
