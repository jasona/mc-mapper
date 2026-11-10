//! Decoration features (geodes, fossils, lava lakes, desert wells): where the game's placement
//! modifiers put them, chunk by chunk.
//!
//! `ChunkGenerator.applyBiomeDecoration` seeds a Xoroshiro `WorldgenRandom` per chunk with
//! `setDecorationSeed(world seed, chunk min x, chunk min z)`, then for each placed feature
//! `setFeatureSeed(decoration seed, index, step)`, where step and index come from the
//! generator's feature order (the pack's `mcmap/feature_order_*.json`, dumped by the oracle).
//! The modifiers then run in order from the chunk's origin. Biome filters use the game's zoomed
//! biome lookup (`BiomeManager.getBiome`) at the final position.
//!
//! Exact: the random stream, rarity, square spread, height ranges, offsets and biome filters.
//! Estimated: heightmaps (the terrain estimate, so surface features are "likely"). Not
//! modelled: modifiers that read placed blocks (block predicates, environment scans) and
//! whether the feature itself succeeds, so every result is a candidate.

use crate::density::Files;
use crate::random::upgrade_seed;
use crate::world::World;
use crate::BiomeId;
use mcmap_data::Dimension;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The `WorldgenRandom` the decoration pass uses: java.util.Random's API (`BitRandomSource`)
/// over the top bits of a Xoroshiro source.
#[derive(Clone, Debug)]
pub struct Rng {
    lo: u64,
    hi: u64,
}

impl Rng {
    pub fn new(seed: i64) -> Rng {
        let mut r = Rng { lo: 0, hi: 0 };
        r.set_seed(seed);
        r
    }

    pub fn set_seed(&mut self, seed: i64) {
        let (lo, hi) = upgrade_seed(seed);
        let x = crate::random::Xoroshiro::from_parts(lo, hi);
        // `from_parts` handles the all-zero state; copy its words back out.
        let (lo, hi) = x.parts();
        self.lo = lo;
        self.hi = hi;
    }

    /// `WorldgenRandom.next(bits)` over a Xoroshiro source: the top bits of its next long.
    fn next(&mut self, bits: u32) -> i32 {
        let mut x = crate::random::Xoroshiro::from_parts(self.lo, self.hi);
        let v = x.next_long();
        (self.lo, self.hi) = x.parts();
        (v >> (64 - bits)) as i32
    }

    /// `BitRandomSource.nextLong`.
    pub fn next_long(&mut self) -> i64 {
        let hi = self.next(32) as i64;
        let lo = self.next(32) as i64;
        (hi << 32).wrapping_add(lo)
    }

    /// `BitRandomSource.nextInt(bound)`: java.util.Random's algorithm on `next(31)`.
    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0);
        if bound & (bound - 1) == 0 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }

    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 * 5.960_464_5e-8
    }

    /// `WorldgenRandom.setDecorationSeed`.
    pub fn set_decoration_seed(&mut self, world: i64, x: i32, z: i32) -> i64 {
        self.set_seed(world);
        let a = self.next_long() | 1;
        let b = self.next_long() | 1;
        let s = ((x as i64).wrapping_mul(a)).wrapping_add((z as i64).wrapping_mul(b)) ^ world;
        self.set_seed(s);
        s
    }

    /// `WorldgenRandom.setFeatureSeed`.
    pub fn set_feature_seed(&mut self, decoration: i64, index: i32, step: i32) {
        self.set_seed(decoration.wrapping_add(index as i64).wrapping_add(10000 * step as i64));
    }
}

/// `BiomeManager.getBiome`: the biome the game reports at a block, from the noise biomes around
/// it, jittered by the zoom seed.
pub fn zoomed_biome(world: &World, zoom: i64, x: i32, y: i32, z: i32, hint: &mut Option<u32>) -> BiomeId {
    let (p, q, r) = crate::fiddled_quart(zoom, x, y, z);
    world.biome(p, q, r, hint)
}

#[derive(Clone, Debug, PartialEq)]
enum Anchor {
    Absolute(i32),
    AboveBottom(i32),
    BelowTop(i32),
}

impl Anchor {
    fn parse(v: &Value) -> Result<Anchor, String> {
        if let Some(n) = v.get("absolute").and_then(Value::as_i64) {
            Ok(Anchor::Absolute(n as i32))
        } else if let Some(n) = v.get("above_bottom").and_then(Value::as_i64) {
            Ok(Anchor::AboveBottom(n as i32))
        } else if let Some(n) = v.get("below_top").and_then(Value::as_i64) {
            Ok(Anchor::BelowTop(n as i32))
        } else {
            Err(format!("vertical anchor {v}"))
        }
    }

    fn resolve(&self, min_y: i32, height: i32) -> i32 {
        match *self {
            Anchor::Absolute(n) => n,
            Anchor::AboveBottom(n) => min_y + n,
            Anchor::BelowTop(n) => min_y + height - 1 - n,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heightmap {
    WorldSurface,
    MotionBlocking,
    OceanFloor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum HeightKind {
    Constant,
    Uniform,
    BiasedToBottom,
    VeryBiasedToBottom,
    Trapezoid,
}

/// `Mth.nextInt(random, a, b)`.
fn between(r: &mut Rng, a: i32, b: i32) -> i32 {
    if a >= b { a } else { r.next_int_bounded(b - a + 1) + a }
}

/// `Mth.randomBetweenInclusive`.
fn inclusive(r: &mut Rng, a: i32, b: i32) -> i32 {
    r.next_int_bounded(b - a + 1) + a
}

impl HeightKind {
    /// The `HeightProvider.sample` of each kind.
    fn sample(self, r: &mut Rng, i: i32, j: i32, n: i32) -> i32 {
        match self {
            HeightKind::Constant => i,
            HeightKind::Uniform => if i > j { i } else { inclusive(r, i, j) },
            HeightKind::BiasedToBottom => {
                if j - i - n < 0 {
                    return i;
                }
                let k = r.next_int_bounded(j - i - n + 1);
                r.next_int_bounded(k + n) + i
            }
            HeightKind::VeryBiasedToBottom => {
                if j - i - n < 0 {
                    return i;
                }
                let k = between(r, i + n, j);
                let l = between(r, i, k - 1);
                between(r, i, l - 1 + n)
            }
            HeightKind::Trapezoid => {
                let k = j - i;
                if n >= k {
                    return inclusive(r, i, j);
                }
                let l = (k - n) / 2;
                let m = k - l;
                i + inclusive(r, 0, m) + inclusive(r, 0, l)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Modifier {
    Count(i32),
    Rarity(i32),
    InSquare,
    /// A height provider: kind, min, max and the inner or plateau size.
    Height(HeightKind, Anchor, Anchor, i32),
    Heightmap(Heightmap),
    Offset(i32, i32, i32),
    Biome,
    /// Reads placed blocks: the position stops here, unchecked.
    Unmodelled(String),
}

#[derive(Clone, Debug)]
pub struct PlacedFeature {
    pub id: String,
    modifiers: Vec<Modifier>,
    pub step: i32,
    pub index: i32,
    /// Biomes (by id in the world's table) whose feature lists hold this feature.
    biomes: Vec<bool>,
}

/// One candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub feature: String,
    pub pos: (i32, i32, i32),
    /// The height came from the terrain estimate.
    pub estimated_height: bool,
    /// A modifier that reads placed blocks was skipped.
    pub unchecked: Option<String>,
}

fn int(v: &Value) -> Result<i32, String> {
    v.as_i64().map(|n| n as i32).ok_or_else(|| format!("{v} is not a constant int (only constants are modelled)"))
}

impl PlacedFeature {
    fn parse(id: &str, v: &Value, step: i32, index: i32, biomes: Vec<bool>) -> Result<PlacedFeature, String> {
        let mut modifiers = Vec::new();
        for m in v["placement"].as_array().ok_or("placed feature without placement")? {
            let t = m["type"].as_str().unwrap_or("").trim_start_matches("minecraft:");
            modifiers.push(match t {
                "count" => Modifier::Count(int(&m["count"])?),
                "rarity_filter" => Modifier::Rarity(int(&m["chance"])?),
                "in_square" => Modifier::InSquare,
                "height_range" => {
                    let h = &m["height"];
                    let ty = h["type"].as_str().unwrap_or("").trim_start_matches("minecraft:");
                    let (kind, extra) = match ty {
                        "uniform" => (HeightKind::Uniform, 0),
                        "biased_to_bottom" => (HeightKind::BiasedToBottom, h.get("inner").map(int).unwrap_or(Ok(1))?),
                        "very_biased_to_bottom" => (HeightKind::VeryBiasedToBottom, h.get("inner").map(int).unwrap_or(Ok(1))?),
                        "trapezoid" => (HeightKind::Trapezoid, h.get("plateau").map(int).unwrap_or(Ok(0))?),
                        "constant" | "" => {
                            let a = Anchor::parse(h.get("value").unwrap_or(h))?;
                            modifiers.push(Modifier::Height(HeightKind::Constant, a.clone(), a, 0));
                            continue;
                        }
                        o => return Err(format!("{id}: height provider {o}")),
                    };
                    Modifier::Height(kind, Anchor::parse(&h["min_inclusive"])?, Anchor::parse(&h["max_inclusive"])?, extra)
                }
                "heightmap" => Modifier::Heightmap(match m["heightmap"].as_str().unwrap_or("") {
                    "WORLD_SURFACE_WG" | "WORLD_SURFACE" => Heightmap::WorldSurface,
                    "MOTION_BLOCKING" | "MOTION_BLOCKING_NO_LEAVES" => Heightmap::MotionBlocking,
                    "OCEAN_FLOOR_WG" | "OCEAN_FLOOR" => Heightmap::OceanFloor,
                    o => return Err(format!("{id}: heightmap {o}")),
                }),
                "offset" => {
                    let c = |k: &str| m.get(k).map(int).unwrap_or(Ok(0));
                    Modifier::Offset(c("x")?, c("y")?, c("z")?)
                }
                "biome" => Modifier::Biome,
                o => Modifier::Unmodelled(o.to_owned()),
            });
        }
        Ok(PlacedFeature { id: id.to_owned(), modifiers, step, index, biomes })
    }
}

/// The placed features of one dimension, ready to place.
pub struct Decorations {
    pub features: Vec<PlacedFeature>,
    dimension: Dimension,
}

impl Decorations {
    /// `wanted` placed feature ids, with `names` the world's biome table.
    pub fn new(files: &Files, dimension: Dimension, names: &[String], wanted: &[&str]) -> Result<Decorations, String> {
        let path = mcmap_data::pack::feature_order(dimension);
        let order: Value = serde_json::from_str(files.get(&path).ok_or("the pack has no feature order; run `cargo run -q -p xtask -- extract --version 26.3` again")?)
            .map_err(|e| format!("{path}: {e}"))?;
        let steps = order["steps"].as_array().ok_or("feature order without steps")?;
        // Which biomes list each feature.
        let mut lists: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for (b, name) in names.iter().enumerate() {
            let Ok(v) = crate::density::json_of(files, "biome", name) else { continue };
            for step in v["features"].as_array().into_iter().flatten() {
                for f in step.as_array().into_iter().flatten() {
                    if let Some(f) = f.as_str() {
                        lists.entry(f.to_owned()).or_default().insert(b);
                    }
                }
            }
        }
        let mut features = Vec::new();
        for &id in wanted {
            let Some((step, index)) = steps.iter().enumerate().find_map(|(s, l)| {
                l.as_array()?.iter().position(|f| f.as_str() == Some(id)).map(|i| (s as i32, i as i32))
            }) else {
                continue; // Not in this dimension.
            };
            let v = crate::density::json_of(files, "placed_feature", id)?;
            let mut biomes = vec![false; names.len()];
            for &b in lists.get(id).into_iter().flatten() {
                biomes[b] = true;
            }
            features.push(PlacedFeature::parse(id, &v, step, index, biomes)?);
        }
        Ok(Decorations { features, dimension })
    }

    fn bounds(&self) -> (i32, i32) {
        match self.dimension {
            Dimension::Overworld => (-64, 384),
            _ => (0, 256),
        }
    }

    /// The candidates of every feature in a chunk.
    pub fn chunk(&self, world: &World, zoom: i64, cx: i32, cz: i32) -> Vec<Candidate> {
        let (min_y, height) = self.bounds();
        let mut out = Vec::new();
        let mut r = Rng::new(0);
        let deco = r.set_decoration_seed(world.seed, cx * 16, cz * 16);
        for f in &self.features {
            r.set_feature_seed(deco, f.index, f.step);
            let mut st = Walk { world, zoom, f, min_y, height, out: &mut out };
            st.walk(&mut r, 0, (cx * 16, min_y, cz * 16), false);
        }
        out
    }
}

struct Walk<'a> {
    world: &'a World,
    zoom: i64,
    f: &'a PlacedFeature,
    min_y: i32,
    height: i32,
    out: &'a mut Vec<Candidate>,
}

impl Walk<'_> {
    fn walk(&mut self, r: &mut Rng, i: usize, p: (i32, i32, i32), est: bool) {
        let Some(m) = self.f.modifiers.get(i) else {
            self.out.push(Candidate { feature: self.f.id.clone(), pos: p, estimated_height: est, unchecked: None });
            return;
        };
        match m {
            Modifier::Count(n) => {
                // The game places the feature between positions, which draws from the same
                // random; only the first position is the game's.
                for _ in 0..*n {
                    self.walk(r, i + 1, p, est);
                }
            }
            Modifier::Rarity(chance) => {
                if r.next_float() < 1.0 / *chance as f32 {
                    self.walk(r, i + 1, p, est);
                }
            }
            Modifier::InSquare => {
                let x = r.next_int_bounded(16) + p.0;
                let z = r.next_int_bounded(16) + p.2;
                self.walk(r, i + 1, (x, p.1, z), est);
            }
            Modifier::Height(kind, lo, hi, n) => {
                let (lo, hi) = (lo.resolve(self.min_y, self.height), hi.resolve(self.min_y, self.height));
                let y = kind.sample(r, lo, hi, *n);
                self.walk(r, i + 1, (p.0, y, p.2), est);
            }
            Modifier::Heightmap(h) => {
                let y = match h {
                    Heightmap::OceanFloor => self.world.floor_height(p.0, p.2),
                    _ => self.world.surface_height(p.0, p.2),
                };
                if y > self.min_y {
                    self.walk(r, i + 1, (p.0, y, p.2), true);
                }
            }
            Modifier::Offset(dx, dy, dz) => self.walk(r, i + 1, (p.0 + dx, p.1 + dy, p.2 + dz), est),
            Modifier::Biome => {
                let b = zoomed_biome(self.world, self.zoom, p.0, p.1, p.2, &mut None);
                if self.f.biomes.get(b.0 as usize).copied().unwrap_or(false) {
                    self.walk(r, i + 1, p, est);
                }
            }
            Modifier::Unmodelled(t) => {
                // Skip it and finish the rest (the biome check), marking the result.
                let mut rest = Walk { world: self.world, zoom: self.zoom, f: self.f, min_y: self.min_y, height: self.height, out: &mut Vec::new() };
                rest.walk(r, i + 1, p, est);
                for mut c in rest.out.drain(..) {
                    c.unchecked = Some(t.clone());
                    self.out.push(c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fiddle_matches_known_values() {
        assert!((crate::zoom_fiddle(0) + 0.45).abs() < 1e-12);
        assert_eq!(crate::zoom_lcg(1, 2), 6364136223846793005i64.wrapping_add(1442695040888963407).wrapping_add(2));
    }
}
