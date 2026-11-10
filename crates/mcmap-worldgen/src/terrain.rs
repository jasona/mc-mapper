//! Terrain shape functions of the Overworld router that the structure checks need: `depth`
//! (for biomes below or above the surface) and the preliminary surface level (an approximate
//! terrain height without the 3-D noise and caves).

use crate::density::{Arena, Builder, Df, Files, Memo, NodeId};
use crate::random::world_positional;
use std::sync::Arc;

pub struct TerrainIds {
    depth: NodeId,
    offset: NodeId,
    factor: NodeId,
    surface: NodeId,
}

impl TerrainIds {
    pub fn prepare(b: &mut Builder, preset: mcmap_data::Preset) -> Result<TerrainIds, String> {
        let settings = b.json("noise_settings", preset.settings())?;
        let dir = preset.functions();
        let router = &settings["noise_router"];
        let depth = b.build_id(&router["depth"]).map_err(|e| format!("noise_router.depth: {e}"))?;
        let surface_ref = settings["aquifers"]["surface_level"].clone();
        let surface_ref = if surface_ref.is_null() { serde_json::json!(format!("{dir}/preliminary_surface_level")) } else { surface_ref };
        let surface = b.build_id(&surface_ref).map_err(|e| format!("preliminary surface level: {e}"))?;
        let offset = b.build_id(&serde_json::json!(format!("{dir}/offset")))?;
        let factor = b.build_id(&serde_json::json!(format!("{dir}/factor")))?;
        Ok(TerrainIds { depth, offset, factor, surface })
    }

    pub fn into_terrain(self, arena: Arc<Arena>) -> Terrain {
        Terrain {
            depth: Df::from_parts(arena.clone(), self.depth),
            offset: Df::from_parts(arena.clone(), self.offset),
            factor: Df::from_parts(arena.clone(), self.factor),
            surface: Df::from_parts(arena, self.surface),
        }
    }
}

#[derive(Clone)]
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
        b.seed = seed;
        let ids = TerrainIds::prepare(&mut b, preset)?;
        Ok(ids.into_terrain(b.finish()))
    }

    #[inline]
    pub fn depth(&self, x: i32, y: i32, z: i32) -> f32 {
        self.depth.eval(x, y, z)
    }

    #[inline]
    pub fn depth_memo(&self, x: i32, y: i32, z: i32, memo: &mut Memo) -> f32 {
        self.depth.eval_memo(x, y, z, memo)
    }

    pub fn depth_df(&self) -> &Df {
        &self.depth
    }

    #[inline]
    pub fn offset(&self, x: i32, z: i32) -> f32 {
        self.offset.eval(x, 0, z)
    }

    #[inline]
    pub fn offset_memo(&self, x: i32, z: i32, memo: &mut Memo) -> f32 {
        memo.reset(0);
        self.offset.eval_memo(x, 0, z, memo)
    }

    #[inline]
    pub fn factor(&self, x: i32, z: i32) -> f32 {
        self.factor.eval(x, 0, z)
    }

    /// The preliminary surface level at a block column: on an 8-block grid, a few blocks off
    /// the real surface where the 3-D noise is strong.
    #[inline]
    pub fn surface(&self, x: i32, z: i32) -> i32 {
        self.surface.eval(x, 0, z) as i32
    }

    /// Any function the builder knows, by id, for diff tests.
    pub fn function(files: &Files, seed: i64, id: &str) -> Result<Df, String> {
        let mut b = Builder::new(files, world_positional(seed));
        b.seed = seed;
        let nid = b.build_id(&serde_json::json!(id))?;
        Ok(Df::from_parts(b.finish(), nid))
    }
}
