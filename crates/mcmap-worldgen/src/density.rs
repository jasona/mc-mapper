//! The density-function language, read from the pack's JSON and evaluated uncached at integer
//! block positions like the game's `sampleValue`. Covers what the climate, depth and the
//! preliminary surface level need: references, caches (pass through), constants, noises and
//! shifts, the binary and unary operators, `clamp`, `lerp`, `gradient`, `spline` and
//! `find_top_surface`. Blending is off, so `blend_alpha` is 1 and `blend_offset` is 0.
//! Anything else is reported by name. Arithmetic is single precision, as in 26.3.

use crate::noise::{NoiseParams, NormalNoise};
use crate::random::Positional;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Files = BTreeMap<String, String>;

#[derive(Debug)]
pub enum Df {
    Const(f32),
    /// `noise`: `noise(x * xz + shift_x, y * y_scale + shift_y, z * xz + shift_z)`.
    Noise { noise: Arc<NormalNoise>, xz: f64, y: f64, shift: Option<Box<[Df; 3]>> },
    /// `shift_a`: `offset(x / 4, 0, z / 4) * 4`.
    ShiftA(Arc<NormalNoise>),
    /// `shift_b`: `offset(z / 4, x / 4, 0) * 4`.
    ShiftB(Arc<NormalNoise>),
    /// `shift`: `offset(x / 4, y / 4, z / 4) * 4`.
    Shift(Arc<NormalNoise>),
    Bin(Op2, Box<Df>, Box<Df>),
    Un(Op1, Box<Df>),
    Clamp(Box<Df>, f32, f32),
    /// `lerp(alpha, first, second)`; alpha 0 and 1 return one side without the other.
    Lerp(Box<Df>, Box<Df>, Box<Df>),
    /// `gradient` along an axis (0 x, 1 y, 2 z), clamped tiling: `from_value + (clamp(c) - from) * factor`.
    Gradient { axis: u8, from: i32, min: i32, max: i32, from_value: f32, factor: f32 },
    Spline(Box<Spline>),
    /// `find_top_surface`: the highest y on a `cell` grid below `upper` where `density` > 0.
    TopSurface { density: Box<Df>, upper: Box<Df>, lower: i32, cell: i32 },
    /// `slice`: the input with one axis (0 x, 1 y, 2 z) held at a coordinate.
    Slice { axis: u8, coord: i32, input: Box<Df> },
    /// `distance_to_point` with a metric: 0 euclidean, 1 euclidean squared, 2 manhattan, 3 chebyshev.
    Distance { metric: u8, point: [i32; 3] },
    /// `end_outer_islands`: `(getHeightValue(x / 8, z / 8) - 8) / 128`.
    EndIslands(Arc<crate::noise::Simplex>),
}

#[derive(Debug, Clone, Copy)]
pub enum Op2 {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
}

#[derive(Debug, Clone, Copy)]
pub enum Op1 {
    Abs,
    Square,
    Cube,
    Sqrt,
    HalfNegative,
    QuarterNegative,
    Reciprocal,
    Negate,
    Squeeze,
    Log,
    Sign,
}

/// `CubicSpline`: a constant, or points along a coordinate with cubic Hermite segments.
#[derive(Debug)]
pub enum Spline {
    Const(f32),
    Multi { coord: Df, locations: Vec<f32>, values: Vec<Spline>, derivatives: Vec<f32> },
}

/// `Mth.lerp(float, float, float)`.
#[inline]
pub fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

impl Spline {
    pub fn sample(&self, x: i32, y: i32, z: i32) -> f32 {
        match self {
            Spline::Const(v) => *v,
            Spline::Multi { coord, locations, values, derivatives } => {
                let c = coord.eval(x, y, z);
                // findIntervalStart: first index whose location is above c, minus one.
                let start = locations.partition_point(|&l| !(c < l)) as i32 - 1;
                let last = locations.len() as i32 - 1;
                let extend = |i: usize| {
                    let v = values[i].sample(x, y, z);
                    let d = derivatives[i];
                    if d == 0.0 { v } else { v + d * (c - locations[i]) }
                };
                if start < 0 {
                    return extend(0);
                }
                if start == last {
                    return extend(last as usize);
                }
                let i = start as usize;
                let (l0, l1) = (locations[i], locations[i + 1]);
                let t = (c - l0) / (l1 - l0);
                let (v0, v1) = (values[i].sample(x, y, z), values[i + 1].sample(x, y, z));
                let a = derivatives[i] * (l1 - l0) - (v1 - v0);
                let b = -derivatives[i + 1] * (l1 - l0) + (v1 - v0);
                lerp(t, v0, v1) + t * (1.0 - t) * lerp(t, a, b)
            }
        }
    }
}

/// `Mth.clamp(float, float, float)`.
#[inline]
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else { v.min(hi) }
}

impl Df {
    /// For a `noise` node: the noise evaluated with its shifts already known. Lets a caller that
    /// shares one shift pair between several noises (the climate router does) compute it once.
    pub fn eval_with_shift(&self, x: i32, y: i32, z: i32, sx: f32, sy: f32, sz: f32) -> Option<f32> {
        match self {
            Df::Noise { noise, xz, y: ys, shift: Some(_) } => {
                Some(noise.get(x as f64 * xz + sx as f64, y as f64 * ys + sy as f64, z as f64 * xz + sz as f64))
            }
            _ => None,
        }
    }

    /// The shift triple of a shifted `noise` node.
    pub fn shifts(&self) -> Option<&[Df; 3]> {
        match self {
            Df::Noise { shift: Some(s), .. } => Some(s),
            _ => None,
        }
    }

    /// Same function (same kind of node over the same noise instance), for shift sharing.
    pub fn same_as(&self, o: &Df) -> bool {
        match (self, o) {
            (Df::Const(a), Df::Const(b)) => a.to_bits() == b.to_bits(),
            (Df::ShiftA(a), Df::ShiftA(b)) | (Df::ShiftB(a), Df::ShiftB(b)) | (Df::Shift(a), Df::Shift(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    pub fn eval(&self, x: i32, y: i32, z: i32) -> f32 {
        match self {
            Df::Const(c) => *c,
            Df::Noise { noise, xz, y: ys, shift } => match shift {
                None => noise.get(x as f64 * xz, y as f64 * ys, z as f64 * xz),
                Some(s) => {
                    let nx = x as f64 * xz + s[0].eval(x, y, z) as f64;
                    let ny = y as f64 * ys + s[1].eval(x, y, z) as f64;
                    let nz = z as f64 * xz + s[2].eval(x, y, z) as f64;
                    noise.get(nx, ny, nz)
                }
            },
            Df::ShiftA(n) => n.get(x as f64 * 0.25, y as f64 * 0.0, z as f64 * 0.25) * 4.0,
            Df::ShiftB(n) => n.get(z as f64 * 0.25, x as f64 * 0.25, 0.0) * 4.0,
            Df::Shift(n) => n.get(x as f64 * 0.25, y as f64 * 0.25, z as f64 * 0.25) * 4.0,
            Df::Bin(op, l, r) => {
                let a = l.eval(x, y, z);
                match op {
                    Op2::Add => a + r.eval(x, y, z),
                    Op2::Sub => a - r.eval(x, y, z),
                    // The game skips the right side when the left is 0.
                    Op2::Mul => if a == 0.0 { 0.0 } else { a * r.eval(x, y, z) },
                    Op2::Div => if a == 0.0 { 0.0 } else { a / r.eval(x, y, z) },
                    Op2::Min => a.min(r.eval(x, y, z)),
                    Op2::Max => a.max(r.eval(x, y, z)),
                }
            }
            Df::Un(op, i) => {
                let v = i.eval(x, y, z);
                match op {
                    Op1::Abs => v.abs(),
                    Op1::Square => v * v,
                    Op1::Cube => v * v * v,
                    Op1::Sqrt => v.sqrt(),
                    Op1::HalfNegative => if v > 0.0 { v } else { v * 0.5 },
                    Op1::QuarterNegative => if v > 0.0 { v } else { v * 0.25 },
                    Op1::Reciprocal => 1.0 / v,
                    Op1::Negate => -v,
                    Op1::Squeeze => {
                        let c = clampf(v, -1.0, 1.0);
                        c / 2.0 - c * c * c / 24.0
                    }
                    Op1::Log => v.ln(),
                    Op1::Sign => if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 },
                }
            }
            Df::Clamp(i, lo, hi) => clampf(i.eval(x, y, z), *lo, *hi),
            Df::Lerp(a, f, s) => {
                let t = a.eval(x, y, z);
                if t == 0.0 {
                    f.eval(x, y, z)
                } else if t == 1.0 {
                    s.eval(x, y, z)
                } else {
                    lerp(t, f.eval(x, y, z), s.eval(x, y, z))
                }
            }
            Df::Gradient { axis, from, min, max, from_value, factor } => {
                let c = match axis { 0 => x, 1 => y, _ => z };
                from_value + (c.clamp(*min, *max) - from) as f32 * factor
            }
            Df::Spline(s) => s.sample(x, y, z),
            Df::Slice { axis, coord, input } => match axis {
                0 => input.eval(*coord, y, z),
                1 => input.eval(x, *coord, z),
                _ => input.eval(x, y, *coord),
            },
            Df::Distance { metric, point } => {
                let (a, b, c) = ((point[0] - x) as f32, (point[1] - y) as f32, (point[2] - z) as f32);
                match metric {
                    0 => (a * a + b * b + c * c).sqrt(),
                    1 => a * a + b * b + c * c,
                    2 => a.abs() + b.abs() + c.abs(),
                    _ => a.abs().max(b.abs()).max(c.abs()),
                }
            }
            Df::EndIslands(n) => (crate::noise::end_island_height(n, x / 8, z / 8) - 8.0) / 128.0,
            Df::TopSurface { density, upper, lower, cell } => {
                let u = upper.eval(x, y, z);
                let top = (u / *cell as f32).floor() as i32 * cell;
                if top <= *lower {
                    return *lower as f32;
                }
                let mut yy = top;
                while yy >= *lower {
                    if density.eval(x, yy, z) > 0.0 {
                        return yy as f32;
                    }
                    yy -= cell;
                }
                *lower as f32
            }
        }
    }
}

/// Builds density functions for one seed, sharing noise instances by id.
pub struct Builder<'a> {
    files: &'a Files,
    random: Positional,
    noises: BTreeMap<String, Arc<NormalNoise>>,
    /// The world seed, for the End islands' legacy random.
    pub seed: i64,
}

/// A worldgen JSON file of the pack by kind and id (`biome`, `minecraft:desert`).
pub fn json_of(files: &Files, kind: &str, id: &str) -> Result<Value, String> {
    let path = file_of(kind, id);
    let text = files.get(&path).ok_or_else(|| format!("the pack has no {path}"))?;
    serde_json::from_str(text).map_err(|e| format!("{path}: {e}"))
}

fn file_of(kind: &str, id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("data/{ns}/worldgen/{kind}/{path}.json")
}

fn strip(t: &str) -> &str {
    t.strip_prefix("minecraft:").unwrap_or(t)
}

impl<'a> Builder<'a> {
    pub fn new(files: &'a Files, random: Positional) -> Builder<'a> {
        Builder { files, random, noises: BTreeMap::new(), seed: 0 }
    }

    pub fn json(&self, kind: &str, id: &str) -> Result<Value, String> {
        let path = file_of(kind, id);
        let text = self.files.get(&path).ok_or_else(|| format!("the pack has no {path}"))?;
        serde_json::from_str(text).map_err(|e| format!("{path}: {e}"))
    }

    /// `Noises.instantiate`: the noise seeded from the hash of its id.
    pub fn noise(&mut self, id: &str) -> Result<Arc<NormalNoise>, String> {
        let id = if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") };
        if let Some(n) = self.noises.get(&id) {
            return Ok(n.clone());
        }
        let params: NoiseParams = serde_json::from_value(self.json("noise", &id)?).map_err(|e| format!("noise {id}: {e}"))?;
        // `RandomState`'s compile context: the Nether biome noises keep their legacy seeding in
        // every dimension.
        let n = Arc::new(match id.as_str() {
            "minecraft:nether/temperature" => NormalNoise::legacy_nether(&params, &mut crate::random::Legacy::new(self.seed))?,
            "minecraft:nether/vegetation" => NormalNoise::legacy_nether(&params, &mut crate::random::Legacy::new(self.seed.wrapping_add(1)))?,
            _ => NormalNoise::create(&params, &mut self.random.from_hash_of(&id))?,
        });
        self.noises.insert(id, n.clone());
        Ok(n)
    }

    fn spline(&mut self, v: &Value) -> Result<Spline, String> {
        match v {
            Value::Number(n) => Ok(Spline::Const(n.as_f64().unwrap_or(0.0) as f32)),
            Value::Object(o) => {
                let coord = self.build(o.get("coordinate").ok_or("spline without a coordinate")?)?;
                let points = o.get("points").and_then(Value::as_array).ok_or("spline without points")?;
                let (mut locations, mut values, mut derivatives) = (Vec::new(), Vec::new(), Vec::new());
                for p in points {
                    locations.push(p.get("location").and_then(Value::as_f64).ok_or("spline point without a location")? as f32);
                    derivatives.push(p.get("derivative").and_then(Value::as_f64).unwrap_or(0.0) as f32);
                    values.push(self.spline(p.get("value").ok_or("spline point without a value")?)?);
                }
                Ok(Spline::Multi { coord, locations, values, derivatives })
            }
            _ => Err(format!("not a spline: {v}")),
        }
    }

    pub fn build(&mut self, v: &Value) -> Result<Df, String> {
        match v {
            Value::Number(n) => Ok(Df::Const(n.as_f64().unwrap_or(0.0) as f32)),
            Value::String(id) => {
                let v = self.json("density_function", id)?;
                self.build(&v).map_err(|e| format!("{id}: {e}"))
            }
            Value::Object(o) => {
                let ty = o.get("type").and_then(Value::as_str).ok_or("density function without a type")?;
                let noise_of = |b: &mut Self| -> Result<Arc<NormalNoise>, String> {
                    let id = o.get("noise").and_then(Value::as_str).ok_or(format!("{ty} without a noise"))?;
                    b.noise(id)
                };
                match strip(ty) {
                    "cache" | "flat_cache" | "cache_2d" | "cache_once" | "cache_all_in_cell" | "interpolated" => {
                        self.build(o.get("argument").or(o.get("input")).ok_or(format!("{ty} without an input"))?)
                    }
                    "constant" => self.build(o.get("argument").or(o.get("value")).ok_or("constant without a value")?),
                    "shift_a" => Ok(Df::ShiftA(noise_of(self)?)),
                    "shift_b" => Ok(Df::ShiftB(noise_of(self)?)),
                    "shift" => Ok(Df::Shift(noise_of(self)?)),
                    "noise" | "shifted_noise" => {
                        let noise = noise_of(self)?;
                        let num = |k: &str| o.get(k).and_then(Value::as_f64);
                        let xz = num("xz_scale").unwrap_or(1.0);
                        let y = num("y_scale").unwrap_or(1.0);
                        let keys = ["shift_x", "shift_y", "shift_z"];
                        let shift = if keys.iter().any(|k| o.contains_key(*k)) {
                            let mut s = Vec::new();
                            for k in keys {
                                s.push(match o.get(k) {
                                    Some(v) => self.build(v)?,
                                    None => Df::Const(0.0),
                                });
                            }
                            let s: [Df; 3] = s.try_into().map_err(|_| "shift")?;
                            Some(Box::new(s))
                        } else {
                            None
                        };
                        Ok(Df::Noise { noise, xz, y, shift })
                    }
                    "blend_alpha" => Ok(Df::Const(1.0)),
                    "blend_offset" => Ok(Df::Const(0.0)),
                    "blend_density" => self.build(o.get("argument").or(o.get("input")).ok_or("blend_density without an input")?),
                    t @ ("add" | "sub" | "mul" | "div" | "min" | "max") => {
                        let op = match t { "add" => Op2::Add, "sub" => Op2::Sub, "mul" => Op2::Mul, "div" => Op2::Div, "min" => Op2::Min, _ => Op2::Max };
                        let l = self.build(o.get("left").or(o.get("argument1")).ok_or(format!("{t} without a left side"))?)?;
                        let r = self.build(o.get("right").or(o.get("argument2")).ok_or(format!("{t} without a right side"))?)?;
                        Ok(Df::Bin(op, Box::new(l), Box::new(r)))
                    }
                    t @ ("abs" | "square" | "cube" | "sqrt" | "half_negative" | "quarter_negative" | "reciprocal" | "negate" | "squeeze" | "log" | "sign") => {
                        let op = match t {
                            "abs" => Op1::Abs, "square" => Op1::Square, "cube" => Op1::Cube, "sqrt" => Op1::Sqrt,
                            "half_negative" => Op1::HalfNegative, "quarter_negative" => Op1::QuarterNegative,
                            "reciprocal" => Op1::Reciprocal, "negate" => Op1::Negate, "squeeze" => Op1::Squeeze,
                            "log" => Op1::Log, _ => Op1::Sign,
                        };
                        let i = self.build(o.get("input").or(o.get("argument")).ok_or(format!("{t} without an input"))?)?;
                        Ok(Df::Un(op, Box::new(i)))
                    }
                    "clamp" => {
                        let i = self.build(o.get("input").ok_or("clamp without an input")?)?;
                        let f = |k: &str| o.get(k).and_then(Value::as_f64).map(|v| v as f32).ok_or(format!("clamp without {k}"));
                        Ok(Df::Clamp(Box::new(i), f("min")?, f("max")?))
                    }
                    "lerp" => {
                        let g = |b: &mut Self, k: &str| -> Result<Box<Df>, String> { Ok(Box::new(b.build(o.get(k).ok_or(format!("lerp without {k}"))?)?)) };
                        Ok(Df::Lerp(g(self, "alpha")?, g(self, "first")?, g(self, "second")?))
                    }
                    "gradient" => {
                        let axis = match o.get("axis").and_then(Value::as_str).unwrap_or("y") { "x" => 0, "y" => 1, _ => 2 };
                        let tiling = o.get("tiling").and_then(Value::as_str).unwrap_or("clamp");
                        if !matches!(tiling, "clamp" | "clamped" | "minecraft:clamp") {
                            return Err(format!("gradient tiling {tiling:?} is not supported yet"));
                        }
                        let int = |k: &str| o.get(k).and_then(Value::as_i64).map(|v| v as i32).ok_or(format!("gradient without {k}"));
                        let fl = |k: &str| o.get(k).and_then(Value::as_f64).map(|v| v as f32).ok_or(format!("gradient without {k}"));
                        let (from, to) = (int("from_coordinate")?, int("to_coordinate")?);
                        let (fv, tv) = (fl("from_value")?, fl("to_value")?);
                        Ok(Df::Gradient { axis, from, min: from.min(to), max: from.max(to), from_value: fv, factor: (tv - fv) / (to - from) as f32 })
                    }
                    "spline" => Ok(Df::Spline(Box::new(self.spline(o.get("spline").ok_or("spline without a spline")?)?))),
                    "find_top_surface" => {
                        let density = Box::new(self.build(o.get("density").ok_or("find_top_surface without density")?)?);
                        let upper = Box::new(self.build(o.get("upper_bound").ok_or("find_top_surface without upper_bound")?)?);
                        let lower = o.get("lower_bound").and_then(Value::as_i64).ok_or("find_top_surface without lower_bound")? as i32;
                        let cell = o.get("cell_height").and_then(Value::as_i64).ok_or("find_top_surface without cell_height")? as i32;
                        Ok(Df::TopSurface { density, upper, lower, cell })
                    }
                    "slice" => {
                        let axis = match o.get("axis").and_then(Value::as_str).unwrap_or("y") { "x" => 0, "y" => 1, _ => 2 };
                        let coord = o.get("coordinate").and_then(Value::as_i64).unwrap_or(0) as i32;
                        Ok(Df::Slice { axis, coord, input: Box::new(self.build(o.get("input").or(o.get("argument")).ok_or("slice without an input")?)?) })
                    }
                    "distance_to_point" => {
                        let metric = match o.get("metric").and_then(Value::as_str).map(strip).unwrap_or("euclidean") {
                            "euclidean" => 0,
                            "euclidean_squared" => 1,
                            "manhattan" => 2,
                            "chebyshev" => 3,
                            m => return Err(format!("distance metric {m:?} is not supported")),
                        };
                        let p = o.get("point").and_then(Value::as_array).ok_or("distance_to_point without a point")?;
                        let g = |i: usize| p.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
                        Ok(Df::Distance { metric, point: [g(0), g(1), g(2)] })
                    }
                    "end_outer_islands" | "end_islands" => {
                        let mut r = crate::random::Legacy::new(self.seed);
                        for _ in 0..17292 {
                            r.next_int();
                        }
                        Ok(Df::EndIslands(Arc::new(crate::noise::Simplex::new(&mut r, true))))
                    }
                    other => Err(format!("density function type {other:?} is not supported yet")),
                }
            }
            _ => Err(format!("not a density function: {v}")),
        }
    }
}
