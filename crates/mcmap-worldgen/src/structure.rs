//! Structures: placement (exact ports of the game's random spread, frequency reducers, exclusion
//! zones and concentric rings) and the per-structure generation point and biome check.
//!
//! Placement and selection are bit-exact. The generation point needs the terrain height for
//! most structures, which the map estimates from the preliminary surface level (see
//! [`World::surface_height`]); a structure that depends on it is reported as `likely`.

use crate::random::Legacy;
use crate::world::{World, SEA_LEVEL};
use crate::BiomeId;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub type Files = BTreeMap<String, String>;

fn strip(s: &str) -> &str {
    s.strip_prefix("minecraft:").unwrap_or(s)
}

fn json(files: &Files, kind: &str, id: &str) -> Result<Value, String> {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    let p = format!("data/{ns}/{kind}/{path}.json");
    let t = files.get(&p).ok_or_else(|| format!("the pack has no {p}"))?;
    serde_json::from_str(t).map_err(|e| format!("{p}: {e}"))
}

/// Biome ids named by a biome list: an id, a `#tag`, or a list of either.
pub fn biome_names(files: &Files, v: &Value, out: &mut BTreeSet<String>) -> Result<(), String> {
    match v {
        Value::String(s) if s.starts_with('#') => {
            let tag = json(files, "tags/worldgen/biome", &s[1..])?;
            for e in tag["values"].as_array().ok_or(format!("tag {s} has no values"))? {
                let e = if let Some(id) = e.get("id") { id } else { e };
                biome_names(files, e, out)?;
            }
            Ok(())
        }
        Value::String(s) => {
            out.insert(if s.contains(':') { s.clone() } else { format!("minecraft:{s}") });
            Ok(())
        }
        Value::Array(a) => a.iter().try_for_each(|e| biome_names(files, e, out)),
        _ => Err(format!("not a biome list: {v}")),
    }
}

/// A set of biomes as flags over the world's biome ids.
#[derive(Clone, Debug, Default)]
pub struct BiomeSet(Vec<bool>);

impl BiomeSet {
    pub fn new(files: &Files, v: &Value, names: &[String]) -> Result<BiomeSet, String> {
        let mut set = BTreeSet::new();
        biome_names(files, v, &mut set)?;
        Ok(BiomeSet(names.iter().map(|n| set.contains(n)).collect()))
    }

    pub fn contains(&self, b: BiomeId) -> bool {
        self.0.get(b.0 as usize).copied().unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        !self.0.iter().any(|&b| b)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reduction {
    Default,
    /// `legacy_type_1`, pillager outposts.
    Legacy1,
    /// `legacy_type_2`, buried treasure.
    Legacy2,
    /// `legacy_type_3`, mineshafts.
    Legacy3,
}

#[derive(Clone, Debug)]
pub struct Spreading {
    pub salt: i32,
    pub frequency: f32,
    pub reduction: Reduction,
    /// Another set and a chunk count: no placement within that many chunks of its placements.
    pub exclusion: Option<(String, i32)>,
    pub locate_offset: [i32; 3],
}

#[derive(Clone, Debug)]
pub enum Placement {
    Random { common: Spreading, spacing: i32, separation: i32, triangular: bool },
    Rings { common: Spreading, distance: i32, spread: i32, count: i32, preferred: BiomeSet },
}

impl Placement {
    pub fn common(&self) -> &Spreading {
        match self {
            Placement::Random { common, .. } | Placement::Rings { common, .. } => common,
        }
    }

    /// `RandomSpreadStructurePlacement.getPotentialStructureChunk`: the one candidate chunk of
    /// the region that contains chunk (cx, cz).
    pub fn potential_chunk(&self, seed: i64, cx: i32, cz: i32) -> Option<(i32, i32)> {
        let Placement::Random { common, spacing, separation, triangular } = self else { return None };
        let (rx, rz) = (cx.div_euclid(*spacing), cz.div_euclid(*spacing));
        let mut r = Legacy::large_feature_with_salt(seed, rx, rz, common.salt);
        let n = spacing - separation;
        let mut off = || if *triangular { (r.next_int_bounded(n) + r.next_int_bounded(n)) / 2 } else { r.next_int_bounded(n) };
        let ox = off();
        let oz = off();
        Some((rx * spacing + ox, rz * spacing + oz))
    }

    /// The frequency reducer (`applyAdditionalChunkRestrictions`).
    pub fn passes_frequency(&self, seed: i64, cx: i32, cz: i32) -> bool {
        let c = self.common();
        if c.frequency >= 1.0 {
            return true;
        }
        match c.reduction {
            // The game passes the salt where the region x goes; kept as it is.
            Reduction::Default => Legacy::large_feature_with_salt(seed, c.salt, cx, cz).next_float() < c.frequency,
            Reduction::Legacy1 => {
                let (i, j) = (cx >> 4, cz >> 4);
                let mut r = Legacy::new(((i ^ (j << 4)) as i64) ^ seed);
                r.next_int();
                r.next_int_bounded((1.0 / c.frequency) as i32) == 0
            }
            Reduction::Legacy2 => Legacy::large_feature_with_salt(seed, cx, cz, 10_387_320).next_float() < c.frequency,
            Reduction::Legacy3 => Legacy::large_feature(seed, cx, cz).next_double() < c.frequency as f64,
        }
    }

    /// `getLocatePos`: the chunk's minimum block plus the locate offset.
    pub fn locate_pos(&self, cx: i32, cz: i32) -> (i32, i32) {
        let o = self.common().locate_offset;
        ((cx << 4) + o[0], (cz << 4) + o[2])
    }

    fn parse(files: &Files, v: &Value, names: &[String]) -> Result<Placement, String> {
        let int = |k: &str, d: i64| v.get(k).and_then(Value::as_i64).unwrap_or(d) as i32;
        let reduction = match v.get("frequency_reduction_method").and_then(Value::as_str).unwrap_or("default") {
            "default" => Reduction::Default,
            "legacy_type_1" => Reduction::Legacy1,
            "legacy_type_2" => Reduction::Legacy2,
            "legacy_type_3" => Reduction::Legacy3,
            o => return Err(format!("frequency reduction {o:?} is not supported")),
        };
        let exclusion = v.get("exclusion_zone").map(|e| {
            (e["other_set"].as_str().unwrap_or("").to_owned(), e["chunk_count"].as_i64().unwrap_or(0) as i32)
        });
        let lo = v.get("locate_offset").and_then(Value::as_array);
        let locate_offset = match lo {
            Some(a) if a.len() == 3 => [0, 1, 2].map(|i| a[i].as_i64().unwrap_or(0) as i32),
            _ => [0; 3],
        };
        let common = Spreading {
            salt: int("salt", 0),
            frequency: v.get("frequency").and_then(Value::as_f64).unwrap_or(1.0) as f32,
            reduction,
            exclusion,
            locate_offset,
        };
        match strip(v["type"].as_str().unwrap_or("")) {
            "random_spread" => Ok(Placement::Random {
                common,
                spacing: int("spacing", 1),
                separation: int("separation", 0),
                triangular: v.get("spread_type").and_then(Value::as_str) == Some("triangular"),
            }),
            "concentric_rings" => Ok(Placement::Rings {
                common,
                distance: int("distance", 0),
                spread: int("spread", 0),
                count: int("count", 0),
                preferred: BiomeSet::new(files, &v["preferred_biomes"], names)?,
            }),
            o => Err(format!("placement {o:?} is not supported")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heightmap {
    WorldSurface,
    OceanFloor,
}

/// How a structure finds its generation point, per structure type.
#[derive(Clone, Debug)]
pub enum Kind {
    Jigsaw { start_height: HeightSpec, project: Option<Heightmap>, start_pool: String, anchor: Option<String> },
    /// Desert pyramid, jungle temple: footprint, needs land above sea level.
    SinglePiece { width: i32, depth: i32 },
    /// On top of the chunk centre: swamp hut, igloo, shipwreck, ocean ruin, buried treasure.
    OnTop(Heightmap),
    Mineshaft,
    Monument,
    Mansion,
    Stronghold,
    RuinedPortal(Vec<PortalSetup>),
    /// Nether and End structures, which the Overworld engine skips.
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vertical {
    OnLandSurface,
    PartlyBuried,
    OnOceanFloor,
    InMountain,
    Underground,
    InNether,
}

#[derive(Clone, Debug)]
pub struct PortalSetup {
    pub placement: Vertical,
    pub air_pocket_probability: f32,
    pub weight: f32,
}

#[derive(Clone, Debug)]
pub enum HeightSpec {
    Absolute(i32),
    Uniform(i32, i32),
}

#[derive(Clone, Debug)]
pub struct Structure {
    pub id: String,
    pub kind: Kind,
    pub biomes: BiomeSet,
}

#[derive(Clone, Debug)]
pub struct StructureSet {
    pub id: String,
    pub placement: Placement,
    pub entries: Vec<(Structure, i32)>,
}

/// What generating a candidate chunk gave.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub structure: String,
    pub chunk: (i32, i32),
    /// The generation point (block x, y, z); y and, for jigsaw structures, x and z are estimates.
    pub pos: (i32, i32, i32),
    /// True when the answer depends on the terrain height estimate.
    pub likely: bool,
    /// Where the map draws the icon (block x, z). Matches the reference capture: the start
    /// piece centre for trial chambers, the template centre for ruined portals, otherwise the
    /// chunk centre.
    pub marker: (i32, i32),
}

fn height_spec(v: &Value) -> Result<HeightSpec, String> {
    if let Some(a) = v.get("absolute").and_then(Value::as_i64) {
        return Ok(HeightSpec::Absolute(a as i32));
    }
    match strip(v.get("type").and_then(Value::as_str).unwrap_or("")) {
        "uniform" => {
            let lo = v["min_inclusive"]["absolute"].as_i64().ok_or("uniform height without an absolute min")? as i32;
            let hi = v["max_inclusive"]["absolute"].as_i64().ok_or("uniform height without an absolute max")? as i32;
            Ok(HeightSpec::Uniform(lo, hi))
        }
        "constant" => height_spec(&v["value"]),
        _ => Err(format!("height {v} is not supported")),
    }
}

impl Structure {
    fn parse(files: &Files, id: &str, names: &[String]) -> Result<Structure, String> {
        let v = json(files, "worldgen/structure", id)?;
        let biomes = BiomeSet::new(files, &v["biomes"], names)?;
        let hm = |s: Option<&str>| match s {
            Some("WORLD_SURFACE_WG") => Some(Heightmap::WorldSurface),
            Some("OCEAN_FLOOR_WG") => Some(Heightmap::OceanFloor),
            _ => None,
        };
        let kind = match strip(v["type"].as_str().unwrap_or("")) {
            "jigsaw" => Kind::Jigsaw {
                start_height: height_spec(&v["start_height"])?,
                project: hm(v.get("project_start_to_heightmap").and_then(Value::as_str)),
                start_pool: v["start_pool"].as_str().unwrap_or("").to_owned(),
                anchor: v.get("start_jigsaw_name").and_then(Value::as_str).map(str::to_owned),
            },
            "desert_pyramid" => Kind::SinglePiece { width: 21, depth: 21 },
            "jungle_temple" => Kind::SinglePiece { width: 12, depth: 15 },
            "swamp_hut" | "igloo" => Kind::OnTop(Heightmap::WorldSurface),
            "shipwreck" => Kind::OnTop(if v["is_beached"].as_bool().unwrap_or(false) { Heightmap::WorldSurface } else { Heightmap::OceanFloor }),
            "ocean_ruin" | "buried_treasure" => Kind::OnTop(Heightmap::OceanFloor),
            "mineshaft" => Kind::Mineshaft,
            "ocean_monument" => Kind::Monument,
            "woodland_mansion" => Kind::Mansion,
            "stronghold" => Kind::Stronghold,
            "ruined_portal" => {
                let mut setups = Vec::new();
                for su in v["setups"].as_array().ok_or("ruined portal without setups")? {
                    let placement = match su["placement"].as_str().unwrap_or("") {
                        "on_land_surface" => Vertical::OnLandSurface,
                        "partly_buried" => Vertical::PartlyBuried,
                        "on_ocean_floor" => Vertical::OnOceanFloor,
                        "in_mountain" => Vertical::InMountain,
                        "underground" => Vertical::Underground,
                        "in_nether" => Vertical::InNether,
                        o => return Err(format!("ruined portal placement {o:?}")),
                    };
                    let f = |k: &str| su[k].as_f64().unwrap_or(0.0) as f32;
                    setups.push(PortalSetup { placement, air_pocket_probability: f("air_pocket_probability"), weight: f("weight") });
                }
                Kind::RuinedPortal(setups)
            }
            o => Kind::Other(o.to_owned()),
        };
        Ok(Structure { id: id.to_owned(), kind, biomes })
    }
}

/// One element of a start pool: its bounding box at the origin per rotation (none, cw 90, 180,
/// ccw 90) and the positions of its named jigsaws per rotation.
#[derive(Clone, Debug)]
struct PoolElement {
    empty: bool,
    boxes: [[i32; 6]; 4],
    anchors: BTreeMap<String, [Vec<[i32; 3]>; 4]>,
}

/// A start pool in the game's weighted order.
#[derive(Clone, Debug)]
struct Pool {
    order: Vec<usize>,
    elements: Vec<PoolElement>,
}

/// Template geometry from the pack (see `mcmap_data::pack::TEMPLATES`).
#[derive(Clone, Debug, Default)]
struct Geometry {
    pools: BTreeMap<String, Pool>,
    /// Ruined portal templates: boxes per mirror (none, front_back) then rotation.
    portals: BTreeMap<String, [[i32; 6]; 8]>,
}

fn arr6(v: &Value) -> [i32; 6] {
    let mut a = [0; 6];
    for (i, x) in a.iter_mut().enumerate() {
        *x = v[i].as_i64().unwrap_or(0) as i32;
    }
    a
}

impl Geometry {
    fn parse(text: &str) -> Result<Geometry, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("templates: {e}"))?;
        let mut g = Geometry::default();
        for (id, p) in v["pools"].as_object().ok_or("templates: no pools")? {
            let order = p["order"].as_array().ok_or("pool order")?.iter().map(|i| i.as_u64().unwrap_or(0) as usize).collect();
            let mut elements = Vec::new();
            for e in p["elements"].as_array().ok_or("pool elements")? {
                let b = e["boxes"].as_array().ok_or("element boxes")?;
                let boxes = [0, 1, 2, 3].map(|r| arr6(&b[r]));
                let mut anchors = BTreeMap::new();
                if let Some(a) = e["anchors"].as_object() {
                    for (name, per) in a {
                        let per = [0, 1, 2, 3].map(|r| {
                            per[r].as_array().map(|l| l.iter().map(|q| [0, 1, 2].map(|k| q[k].as_i64().unwrap_or(0) as i32)).collect()).unwrap_or_default()
                        });
                        anchors.insert(name.clone(), per);
                    }
                }
                let empty = e["element"].as_str().is_some_and(|s| s.starts_with("Empty"));
                elements.push(PoolElement { empty, boxes, anchors });
            }
            g.pools.insert(id.clone(), Pool { order, elements });
        }
        if let Some(t) = v["templates"].as_object() {
            for (id, tv) in t {
                let b = tv["boxes"].as_array().ok_or("template boxes")?;
                g.portals.insert(id.clone(), [0, 1, 2, 3, 4, 5, 6, 7].map(|i| arr6(&b[i])));
            }
        }
        Ok(g)
    }
}

const PORTALS: [&str; 10] = ["portal_1", "portal_2", "portal_3", "portal_4", "portal_5", "portal_6", "portal_7", "portal_8", "portal_9", "portal_10"];
const GIANT_PORTALS: [&str; 3] = ["giant_portal_1", "giant_portal_2", "giant_portal_3"];

/// All structure sets of a pack, parsed against one world's biome list.
pub struct Structures {
    pub sets: Vec<StructureSet>,
    monument_surroundings: BiomeSet,
    geometry: Option<Geometry>,
}

impl Structures {
    pub fn new(files: &Files, names: &[String]) -> Result<Structures, String> {
        let mut sets = Vec::new();
        for path in files.keys() {
            let Some(rest) = path.strip_prefix("data/minecraft/worldgen/structure_set/") else { continue };
            let id = format!("minecraft:{}", rest.trim_end_matches(".json"));
            let v = json(files, "worldgen/structure_set", &id)?;
            let placement = Placement::parse(files, &v["placement"], names).map_err(|e| format!("{id}: {e}"))?;
            let mut entries = Vec::new();
            for e in v["structures"].as_array().ok_or(format!("{id}: no structures"))? {
                let sid = e["structure"].as_str().ok_or(format!("{id}: structure without an id"))?;
                let s = Structure::parse(files, sid, names).map_err(|e| format!("{sid}: {e}"))?;
                entries.push((s, e.get("weight").and_then(Value::as_i64).unwrap_or(1) as i32));
            }
            sets.push(StructureSet { id, placement, entries });
        }
        let monument_surroundings = BiomeSet::new(files, &Value::String("#minecraft:required_ocean_monument_surrounding".into()), names)?;
        let geometry = match files.get(mcmap_data::pack::TEMPLATES) {
            Some(t) => Some(Geometry::parse(t)?),
            None => None,
        };
        Ok(Structures { sets, monument_surroundings, geometry })
    }

    /// Whether the pack carries template geometry (packs extracted before M3 do not).
    pub fn has_geometry(&self) -> bool {
        self.geometry.is_some()
    }

    pub fn set(&self, id: &str) -> Option<&StructureSet> {
        self.sets.iter().find(|s| s.id == id)
    }

    /// `StructurePlacement.isStructureChunk` for a random-spread set: the region's candidate,
    /// the frequency reducer and the exclusion zone. Ring sets answer through [`rings`].
    pub fn is_placement_chunk(&self, set: &StructureSet, seed: i64, cx: i32, cz: i32) -> bool {
        if set.placement.potential_chunk(seed, cx, cz) != Some((cx, cz)) {
            return false;
        }
        if !set.placement.passes_frequency(seed, cx, cz) {
            return false;
        }
        if let Some((other, n)) = &set.placement.common().exclusion
            && let Some(o) = self.set(other)
        {
            for z in cz - n..=cz + n {
                for x in cx - n..=cx + n {
                    if o.placement.potential_chunk(seed, x, z) == Some((x, z)) && o.placement.passes_frequency(seed, x, z) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Placement chunks of a random-spread set in the chunk rectangle (inclusive), one
    /// candidate per region, so the cost is per region, not per chunk.
    pub fn placements_in(&self, set: &StructureSet, seed: i64, cx0: i32, cz0: i32, cx1: i32, cz1: i32) -> Vec<(i32, i32)> {
        let Placement::Random { spacing, .. } = &set.placement else { return Vec::new() };
        let mut out = Vec::new();
        for rz in cz0.div_euclid(*spacing)..=cz1.div_euclid(*spacing) {
            for rx in cx0.div_euclid(*spacing)..=cx1.div_euclid(*spacing) {
                let Some((x, z)) = set.placement.potential_chunk(seed, rx * spacing, rz * spacing) else { continue };
                if x < cx0 || x > cx1 || z < cz0 || z > cz1 {
                    continue;
                }
                if self.is_placement_chunk(set, seed, x, z) {
                    out.push((x, z));
                }
            }
        }
        out
    }

    /// Every structure of a random-spread set generated in the chunk rectangle (inclusive).
    /// Ring sets (strongholds) go through [`Structures::rings`] once per seed instead.
    pub fn in_area(&self, world: &World, set: &StructureSet, cx0: i32, cz0: i32, cx1: i32, cz1: i32) -> Vec<Found> {
        self.placements_in(set, world.seed, cx0, cz0, cx1, cz1).into_iter().filter_map(|(x, z)| self.generate(world, set, x, z)).collect()
    }

    /// `ChunkGenerator.createStructures` for one set at a placement chunk: the weighted pick,
    /// retried without the failed entry, the way the game does it.
    pub fn generate(&self, world: &World, set: &StructureSet, cx: i32, cz: i32) -> Option<Found> {
        if set.entries.len() == 1 {
            return self.try_structure(world, &set.entries[0].0, cx, cz);
        }
        let mut r = Legacy::large_feature(world.seed, cx, cz);
        let mut left: Vec<&(Structure, i32)> = set.entries.iter().collect();
        let mut total: i32 = left.iter().map(|e| e.1).sum();
        while !left.is_empty() {
            let mut pick = r.next_int_bounded(total);
            let mut i = 0;
            for e in &left {
                pick -= e.1;
                if pick < 0 {
                    break;
                }
                i += 1;
            }
            if let Some(f) = self.try_structure(world, &left[i].0, cx, cz) {
                return Some(f);
            }
            total -= left[i].1;
            left.remove(i);
        }
        None
    }

    /// One structure's `findValidGenerationPoint` at chunk (cx, cz).
    pub fn try_structure(&self, world: &World, s: &Structure, cx: i32, cz: i32) -> Option<Found> {
        let (mx, mz) = ((cx << 4) + 8, (cz << 4) + 8);
        let (x0, z0) = (cx << 4, cz << 4);
        let found = |pos: (i32, i32, i32), likely: bool| {
            let marker = if s.id == "minecraft:trial_chambers" { (pos.0, pos.2) } else { (mx, mz) };
            Found { structure: s.id.clone(), chunk: (cx, cz), pos, likely, marker }
        };
        let ok = |x: i32, y: i32, z: i32| s.biomes.contains(world.biome_at(x, y, z));
        // The structure's own random (`GenerationContext.random`).
        let mut r = Legacy::large_feature(world.seed, cx, cz);
        match &s.kind {
            Kind::Other(_) => None,
            Kind::Stronghold => ok(x0, 0, z0).then(|| found((x0, 0, z0), false)),
            // The game moves the shaft down by an amount that depends on all of its pieces, which
            // this does not generate; the check at y 50 matches the game 97% of the time.
            Kind::Mineshaft => ok(mx, 50, z0).then(|| found((mx, 50, z0), true)),
            Kind::OnTop(hm) => {
                let y = match hm {
                    Heightmap::WorldSurface => world.surface_height(mx, mz),
                    Heightmap::OceanFloor => world.floor_height(mx, mz),
                } - 1;
                ok(mx, y, mz).then(|| found((mx, y, mz), true))
            }
            Kind::SinglePiece { width, depth } => {
                let lowest = [(x0, z0), (x0 + width, z0), (x0, z0 + depth), (x0 + width, z0 + depth)]
                    .iter()
                    .map(|&(x, z)| world.surface_height(x, z) - 1)
                    .min()
                    .unwrap();
                if lowest < SEA_LEVEL {
                    return None;
                }
                let y = world.surface_height(mx, mz) - 1;
                ok(mx, y, mz).then(|| found((mx, y, mz), true))
            }
            Kind::Monument => {
                let (x, z) = ((cx << 4) + 9, (cz << 4) + 9);
                if !self.all_within(world, &self.monument_surroundings, x, SEA_LEVEL, z, 29) {
                    return None;
                }
                let y = world.floor_height(mx, mz) - 1;
                ok(mx, y, mz).then(|| found((mx, y, mz), true))
            }
            Kind::Mansion => {
                let (x, z) = ((cx << 4) + 7, (cz << 4) + 7);
                let rot = r.next_int_bounded(4);
                let (dx, dz) = match rot { 1 => (-5, 5), 2 => (-5, -5), 3 => (5, -5), _ => (5, 5) };
                let y = [(x, z), (x, z + dz), (x + dx, z), (x + dx, z + dz)].iter().map(|&(a, b)| world.surface_height(a, b) - 1).min().unwrap();
                if y < 60 {
                    return None;
                }
                ok(x, y, z).then(|| found((x, y, z), true))
            }
            Kind::RuinedPortal(setups) => {
                let Some(g) = &self.geometry else {
                    let y = world.surface_height(mx, mz) - 1;
                    return ok(x0, y, z0).then(|| found((x0, y, z0), true));
                };
                let setup = if setups.len() > 1 {
                    let total: f32 = setups.iter().map(|s| s.weight).sum();
                    let mut f = r.next_float();
                    setups.iter().find(|s| {
                        f -= s.weight / total;
                        f < 0.0
                    })?
                } else {
                    setups.first()?
                };
                let p = setup.air_pocket_probability;
                if p != 0.0 && p != 1.0 {
                    r.next_float();
                }
                let name = if r.next_float() < 0.05 { GIANT_PORTALS[r.next_int_bounded(3) as usize] } else { PORTALS[r.next_int_bounded(10) as usize] };
                let rot = r.next_int_bounded(4) as usize;
                let mirror = if r.next_float() < 0.5 { 0 } else { 1 };
                let b = g.portals.get(&format!("minecraft:ruined_portal/{name}"))?[mirror * 4 + rot];
                let bb = [b[0] + x0, b[1], b[2] + z0, b[3] + x0, b[4], b[5] + z0];
                let (cxb, czb) = (bb[0] + (bb[3] - bb[0] + 1) / 2, bb[2] + (bb[5] - bb[2] + 1) / 2);
                let floor = setup.placement == Vertical::OnOceanFloor;
                let h = |x: i32, z: i32| if floor { world.floor_height(x, z) } else { world.surface_height(x, z) } - 1;
                let height = h(cxb, czb);
                let span = bb[4] - bb[1] + 1;
                let min_y = -64 + 15;
                let within = |r: &mut Legacy, lo: i32, hi: i32| if lo < hi { lo + r.next_int_bounded(hi - lo + 1) } else { hi };
                let j = match setup.placement {
                    Vertical::InMountain => within(&mut r, 70, height - span),
                    Vertical::Underground => within(&mut r, min_y, height - span),
                    Vertical::PartlyBuried => height - span + 2 + r.next_int_bounded(7),
                    Vertical::InNether => return None,
                    _ => height,
                };
                // findSuitableY: down from j until three of the four corner columns are solid;
                // with heights only, that is the second lowest corner.
                let mut corners = [h(bb[0], bb[2]), h(bb[3], bb[2]), h(bb[0], bb[5]), h(bb[3], bb[5])];
                corners.sort();
                let y = j.min(corners[1]).max(min_y);
                ok(x0, y, z0).then(|| Found { marker: ((bb[0] + bb[3] + 1) / 2, (bb[2] + bb[5] + 1) / 2), ..found((x0, y, z0), true) })
            }
            Kind::Jigsaw { start_height, project, start_pool, anchor } => {
                let base = match start_height {
                    HeightSpec::Absolute(y) => *y,
                    HeightSpec::Uniform(lo, hi) => lo + r.next_int_bounded(hi - lo + 1),
                };
                let (mut x, mut z) = (mx, mz);
                if let Some(pool) = self.geometry.as_ref().and_then(|g| g.pools.get(start_pool)) {
                    let rot = r.next_int_bounded(4) as usize;
                    let e = &pool.elements[pool.order[r.next_int_bounded(pool.order.len() as i32) as usize]];
                    if e.empty {
                        return None;
                    }
                    // The start piece sits at the chunk corner, or with its named jigsaw there.
                    let (mut ox, mut oz) = (x0, z0);
                    if let Some(a) = anchor {
                        let p = e.anchors.get(a).and_then(|per| per[rot].first())?;
                        ox -= p[0];
                        oz -= p[2];
                    }
                    let b = e.boxes[rot];
                    x = (b[0] + ox + b[3] + ox) / 2;
                    z = (b[2] + oz + b[5] + oz) / 2;
                }
                let y = match project {
                    Some(Heightmap::WorldSurface) => world.surface_height(x, z) + base,
                    Some(Heightmap::OceanFloor) => world.floor_height(x, z) + base,
                    None => base,
                };
                ok(x, y, z).then(|| found((x, y, z), project.is_some()))
            }
        }
    }

    /// `BiomeSource.getBiomesWithin`: every biome in the box of quarts around a point is in `set`.
    fn all_within(&self, world: &World, set: &BiomeSet, x: i32, y: i32, z: i32, r: i32) -> bool {
        let mut hint = None;
        for qy in (y - r) >> 2..=(y + r) >> 2 {
            for qz in (z - r) >> 2..=(z + r) >> 2 {
                for qx in (x - r) >> 2..=(x + r) >> 2 {
                    if !set.contains(world.biome(qx, qy, qz, &mut hint)) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// `ChunkGeneratorStructureState.generateRingPositions`, with the biome search of
    /// `findBiomeHorizontal` at y 0 within 112 blocks.
    pub fn rings(&self, world: &World, set: &StructureSet) -> Vec<(i32, i32)> {
        let Placement::Rings { distance, spread, count, preferred, .. } = &set.placement else { return Vec::new() };
        let (distance, count) = (*distance, *count);
        let mut spread = *spread;
        let mut out = Vec::with_capacity(count as usize);
        if count == 0 {
            return out;
        }
        let mut r = Legacy::new(world.seed);
        let mut angle = r.next_double() * std::f64::consts::PI * 2.0;
        let (mut placed, mut ring) = (0, 0);
        for n in 0..count {
            let dist = (4 * distance + distance * ring * 6) as f64 + (r.next_double() - 0.5) * (distance as f64 * 2.5);
            let kx = (angle.cos() * dist).round() as i32;
            let kz = (angle.sin() * dist).round() as i32;
            let mut fork = r.fork();
            out.push(match find_biome_horizontal(world, (kx << 4) + 8, 0, (kz << 4) + 8, 112, preferred, &mut fork) {
                Some((bx, bz)) => (bx >> 4, bz >> 4),
                None => (kx, kz),
            });
            angle += std::f64::consts::PI * 2.0 / spread as f64;
            placed += 1;
            if placed == spread {
                ring += 1;
                placed = 0;
                spread += 2 * spread / (ring + 1);
                spread = spread.min(count - n);
                angle += r.next_double() * std::f64::consts::PI * 2.0;
            }
        }
        out
    }
}

/// `BiomeSource.findBiomeHorizontal` (not closest-first): every matching quart in the square has
/// an equal chance, by reservoir sampling with the given random.
pub fn find_biome_horizontal(world: &World, x: i32, y: i32, z: i32, radius: i32, set: &BiomeSet, r: &mut Legacy) -> Option<(i32, i32)> {
    let (qx, qz, k, qy) = (x >> 2, z >> 2, radius >> 2, y >> 2);
    let mut best = None;
    let mut found = 0;
    let mut hint = None;
    for dz in -k..=k {
        for dx in -k..=k {
            let (bx, bz) = (qx + dx, qz + dz);
            if set.contains(world.biome(bx, qy, bz, &mut hint)) {
                if best.is_none() || r.next_int_bounded(found + 1) == 0 {
                    best = Some((bx << 2, bz << 2));
                }
                found += 1;
            }
        }
    }
    best
}
