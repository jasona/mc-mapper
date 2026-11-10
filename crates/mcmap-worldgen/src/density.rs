//! The density-function language, read from the pack's JSON into a compact deduplicated arena DAG
//! and evaluated at integer block positions like the game's `sampleValue`, with exact per-query
//! `(x, z)` and `(x, y, z)` memoization across shared subgraphs. Covers what the climate, depth
//! and preliminary surface level need: references, caches, constants, noises and shifts, binary
//! and unary operators, `clamp`, `lerp`, `gradient`, `spline`, `slice`, `distance_to_point`,
//! `end_outer_islands`, and `find_top_surface`. Blending is off (`blend_alpha = 1`,
//! `blend_offset = 0`). Arithmetic is single precision, matching 26.3 bit-for-bit.

use crate::noise::{EndIslands, NoiseParams, NormalNoise, Simplex};
use crate::random::Positional;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Files = BTreeMap<String, String>;

/// Opaque node handle inside a [`Builder`]'s arena before [`Builder::finish`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(u32);

use std::mem::MaybeUninit;

/// Stack-allocated per-coordinate memo table shared across density evaluations at the same `(x, z)`.
///
/// 2-D nodes (`!uses_y`) stay valid across any `y` change (such as `find_top_surface` stepping
/// down a column), while 3-D nodes (`uses_y`) invalidate whenever `y` changes.
pub struct Memo {
    y: i32,
    valid_xz: u64,
    valid_xyz: u64,
    vals_xz: [MaybeUninit<f32>; 64],
    vals_xyz: [MaybeUninit<f32>; 64],
}

impl Memo {
    #[inline(always)]
    pub fn new(y: i32) -> Memo {
        Memo {
            y,
            valid_xz: 0,
            valid_xyz: 0,
            // SAFETY: an array of `MaybeUninit<f32>` requires no initialization; a slot is read
            // only when its corresponding bit in `valid_xz` / `valid_xyz` has been set after `write`.
            vals_xz: [const { MaybeUninit::uninit() }; 64],
            vals_xyz: [const { MaybeUninit::uninit() }; 64],
        }
    }

    #[inline(always)]
    pub fn reset(&mut self, y: i32) {
        self.y = y;
        self.valid_xz = 0;
        self.valid_xyz = 0;
    }

    #[inline(always)]
    pub fn set_y(&mut self, y: i32) {
        if self.y != y {
            self.y = y;
            self.valid_xyz = 0;
        }
    }
}

#[derive(Clone, Debug)]
enum NodeKind {
    Const(f32),
    /// `noise`: `noise(x * xz + shift_x, y * y_scale + shift_y, z * xz + shift_z)`.
    Noise { noise: Arc<NormalNoise>, xz: f64, y: f64, shift: Option<[u32; 3]> },
    /// `shift_a`: `offset(x / 4, 0, z / 4) * 4`.
    ShiftA(Arc<NormalNoise>),
    /// `shift_b`: `offset(z / 4, x / 4, 0) * 4`.
    ShiftB(Arc<NormalNoise>),
    /// `shift`: `offset(x / 4, y / 4, z / 4) * 4`.
    Shift(Arc<NormalNoise>),
    Bin(Op2, u32, u32),
    Un(Op1, u32),
    Clamp(u32, f32, f32),
    /// `lerp(alpha, first, second)`; alpha 0 and 1 return one side without the other.
    Lerp(u32, u32, u32),
    /// `gradient` along an axis (0 x, 1 y, 2 z), clamped tiling: `from_value + (clamp(c) - from) * factor`.
    Gradient { axis: u8, from: i32, min: i32, max: i32, from_value: f32, factor: f32 },
    Spline(u32),
    /// `find_top_surface`: the highest y on a `cell` grid below `upper` where `density` > 0.
    TopSurface { density: u32, upper: u32, lower: i32, cell: i32 },
    /// `slice`: the input with one axis (0 x, 1 y, 2 z) held at a coordinate.
    Slice { axis: u8, coord: i32, input: u32 },
    /// `distance_to_point` with a metric: 0 euclidean, 1 euclidean squared, 2 manhattan, 3 chebyshev.
    Distance { metric: u8, point: [i32; 3] },
    /// `end_outer_islands`: `(getHeightValue(x / 8, z / 8) - 8) / 128`.
    EndIslands(Arc<EndIslands>),
}

#[derive(Clone, Debug)]
struct Node {
    kind: NodeKind,
    uses_y: bool,
    slot: Option<u8>,
}

#[derive(Clone, Copy, Debug)]
struct SplinePoint {
    loc: f32,
    val: u32,
    deriv: f32,
}

#[derive(Clone, Copy, Debug)]
enum SplineNode {
    Const(f32),
    Multi { coord: u32, start: u32, len: u32, uses_y: bool },
}

impl SplineNode {
    fn uses_y(&self) -> bool {
        match self {
            SplineNode::Const(_) => false,
            SplineNode::Multi { uses_y, .. } => *uses_y,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Arena {
    nodes: Vec<Node>,
    splines: Vec<SplineNode>,
    spline_pts: Vec<SplinePoint>,
    num_xz: u8,
    num_xyz: u8,
}

/// A compiled density function backed by a shared [`Arena`]. Cloning is `O(1)`.
#[derive(Clone, Debug)]
pub struct Df {
    arena: Arc<Arena>,
    root: u32,
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

/// `Mth.lerp(float, float, float)`.
#[inline(always)]
pub fn lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

/// `Mth.clamp(float, float, float)`.
#[inline(always)]
fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else { v.min(hi) }
}

impl Arena {
    #[inline]
    fn eval_node(&self, id: u32, x: i32, y: i32, z: i32, memo: &mut Memo) -> f32 {
        let node = &self.nodes[id as usize];
        if let Some(s) = node.slot {
            let idx = s as usize;
            let bit = 1u64 << s;
            if !node.uses_y {
                if memo.valid_xz & bit != 0 {
                    // SAFETY: `bit` in `valid_xz` is only set immediately after `vals_xz[idx].write(v)`
                    // and `valid_xz` is never set without writing.
                    return unsafe { memo.vals_xz[idx].assume_init_read() };
                }
                let v = self.eval_kind(&node.kind, x, y, z, memo);
                memo.vals_xz[idx].write(v);
                memo.valid_xz |= bit;
                return v;
            } else {
                memo.set_y(y);
                if memo.valid_xyz & bit != 0 {
                    // SAFETY: `bit` in `valid_xyz` is only set after `vals_xyz[idx].write(v)` at `memo.y == y`.
                    return unsafe { memo.vals_xyz[idx].assume_init_read() };
                }
                let v = self.eval_kind(&node.kind, x, y, z, memo);
                memo.set_y(y);
                memo.vals_xyz[idx].write(v);
                memo.valid_xyz |= bit;
                return v;
            }
        }
        self.eval_kind(&node.kind, x, y, z, memo)
    }

    fn eval_spline(&self, id: u32, x: i32, y: i32, z: i32, memo: &mut Memo) -> f32 {
        match self.splines[id as usize] {
            SplineNode::Const(v) => v,
            SplineNode::Multi { coord, start, len, .. } => {
                let pts = &self.spline_pts[start as usize..(start + len) as usize];
                let c = self.eval_node(coord, x, y, z, memo);
                // findIntervalStart: first index whose location is above c, minus one.
                let idx = pts.partition_point(|p| !(c < p.loc)) as i32 - 1;
                let last = pts.len() as i32 - 1;
                if idx < 0 {
                    let p = &pts[0];
                    let v = self.eval_spline(p.val, x, y, z, memo);
                    return if p.deriv == 0.0 { v } else { v + p.deriv * (c - p.loc) };
                }
                if idx == last {
                    let p = &pts[last as usize];
                    let v = self.eval_spline(p.val, x, y, z, memo);
                    return if p.deriv == 0.0 { v } else { v + p.deriv * (c - p.loc) };
                }
                let i = idx as usize;
                let (p0, p1) = (&pts[i], &pts[i + 1]);
                let (l0, l1) = (p0.loc, p1.loc);
                let t = (c - l0) / (l1 - l0);
                let v0 = self.eval_spline(p0.val, x, y, z, memo);
                let v1 = self.eval_spline(p1.val, x, y, z, memo);
                let a = p0.deriv * (l1 - l0) - (v1 - v0);
                let b = -p1.deriv * (l1 - l0) + (v1 - v0);
                lerp(t, v0, v1) + t * (1.0 - t) * lerp(t, a, b)
            }
        }
    }

    fn eval_kind(&self, kind: &NodeKind, x: i32, y: i32, z: i32, memo: &mut Memo) -> f32 {
        match kind {
            NodeKind::Const(c) => *c,
            NodeKind::Noise { noise, xz, y: ys, shift } => match shift {
                None => noise.get(x as f64 * xz, y as f64 * ys, z as f64 * xz),
                Some(s) => {
                    let nx = x as f64 * xz + self.eval_node(s[0], x, y, z, memo) as f64;
                    let ny = y as f64 * ys + self.eval_node(s[1], x, y, z, memo) as f64;
                    let nz = z as f64 * xz + self.eval_node(s[2], x, y, z, memo) as f64;
                    noise.get(nx, ny, nz)
                }
            },
            NodeKind::ShiftA(n) => n.get(x as f64 * 0.25, 0.0, z as f64 * 0.25) * 4.0,
            NodeKind::ShiftB(n) => n.get(z as f64 * 0.25, x as f64 * 0.25, 0.0) * 4.0,
            NodeKind::Shift(n) => n.get(x as f64 * 0.25, y as f64 * 0.25, z as f64 * 0.25) * 4.0,
            NodeKind::Bin(op, l, r) => {
                let a = self.eval_node(*l, x, y, z, memo);
                match op {
                    Op2::Add => a + self.eval_node(*r, x, y, z, memo),
                    Op2::Sub => a - self.eval_node(*r, x, y, z, memo),
                    // The game skips the right side when the left is 0.
                    Op2::Mul => if a == 0.0 { 0.0 } else { a * self.eval_node(*r, x, y, z, memo) },
                    Op2::Div => if a == 0.0 { 0.0 } else { a / self.eval_node(*r, x, y, z, memo) },
                    Op2::Min => a.min(self.eval_node(*r, x, y, z, memo)),
                    Op2::Max => a.max(self.eval_node(*r, x, y, z, memo)),
                }
            }
            NodeKind::Un(op, i) => {
                let v = self.eval_node(*i, x, y, z, memo);
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
            NodeKind::Clamp(i, lo, hi) => clampf(self.eval_node(*i, x, y, z, memo), *lo, *hi),
            NodeKind::Lerp(a, f, s) => {
                let t = self.eval_node(*a, x, y, z, memo);
                if t == 0.0 {
                    self.eval_node(*f, x, y, z, memo)
                } else if t == 1.0 {
                    self.eval_node(*s, x, y, z, memo)
                } else {
                    lerp(t, self.eval_node(*f, x, y, z, memo), self.eval_node(*s, x, y, z, memo))
                }
            }
            NodeKind::Gradient { axis, from, min, max, from_value, factor } => {
                let c = match axis { 0 => x, 1 => y, _ => z };
                from_value + (c.clamp(*min, *max) - from) as f32 * factor
            }
            NodeKind::Spline(s) => self.eval_spline(*s, x, y, z, memo),
            NodeKind::Slice { axis, coord, input } => match axis {
                0 => {
                    let mut sub = Memo::new(y);
                    self.eval_node(*input, *coord, y, z, &mut sub)
                }
                1 => {
                    memo.set_y(*coord);
                    let v = self.eval_node(*input, x, *coord, z, memo);
                    memo.set_y(y);
                    v
                }
                _ => {
                    let mut sub = Memo::new(y);
                    self.eval_node(*input, x, y, *coord, &mut sub)
                }
            },
            NodeKind::Distance { metric, point } => {
                let (a, b, c) = ((point[0] - x) as f32, (point[1] - y) as f32, (point[2] - z) as f32);
                match metric {
                    0 => (a * a + b * b + c * c).sqrt(),
                    1 => a * a + b * b + c * c,
                    2 => a.abs() + b.abs() + c.abs(),
                    _ => a.abs().max(b.abs()).max(c.abs()),
                }
            }
            NodeKind::EndIslands(n) => (n.height(x / 8, z / 8) - 8.0) / 128.0,
            NodeKind::TopSurface { density, upper, lower, cell } => {
                let u = self.eval_node(*upper, x, y, z, memo);
                let top = (u / *cell as f32).floor() as i32 * cell;
                if top <= *lower {
                    return *lower as f32;
                }
                let mut yy = top;
                while yy >= *lower {
                    memo.set_y(yy);
                    if self.eval_node(*density, x, yy, z, memo) > 0.0 {
                        memo.set_y(y);
                        return yy as f32;
                    }
                    yy -= cell;
                }
                memo.set_y(y);
                *lower as f32
            }
        }
    }
}

impl Df {
    pub fn from_parts(arena: Arc<Arena>, id: NodeId) -> Df {
        Df { arena, root: id.0 }
    }

    #[inline]
    pub fn eval(&self, x: i32, y: i32, z: i32) -> f32 {
        let mut memo = Memo::new(y);
        self.arena.eval_node(self.root, x, y, z, &mut memo)
    }

    /// Evaluate using a caller-supplied [`Memo`] bound to `(x, z)` inside the same [`Arena`].
    #[inline]
    pub fn eval_memo(&self, x: i32, y: i32, z: i32, memo: &mut Memo) -> f32 {
        self.arena.eval_node(self.root, x, y, z, memo)
    }

    pub fn shares_arena(&self, other: &Df) -> bool {
        Arc::ptr_eq(&self.arena, &other.arena)
    }
}

/// Builds density functions for one seed, sharing noise instances and named subgraphs in one [`Arena`].
pub struct Builder<'a> {
    files: &'a Files,
    random: Positional,
    noises: BTreeMap<String, Arc<NormalNoise>>,
    named: BTreeMap<String, NodeId>,
    shifts: BTreeMap<(u8, usize), NodeId>,
    end_islands: Option<NodeId>,
    arena: Arena,
    /// The world seed, for the End islands' and Nether's legacy random.
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

fn canonical_id(id: &str) -> String {
    if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") }
}

impl<'a> Builder<'a> {
    pub fn new(files: &'a Files, random: Positional) -> Builder<'a> {
        Builder {
            files,
            random,
            noises: BTreeMap::new(),
            named: BTreeMap::new(),
            shifts: BTreeMap::new(),
            end_islands: None,
            arena: Arena::default(),
            seed: 0,
        }
    }

    pub fn finish(self) -> Arc<Arena> {
        Arc::new(self.arena)
    }

    pub fn json(&self, kind: &str, id: &str) -> Result<Value, String> {
        let path = file_of(kind, id);
        let text = self.files.get(&path).ok_or_else(|| format!("the pack has no {path}"))?;
        serde_json::from_str(text).map_err(|e| format!("{path}: {e}"))
    }

    /// `Noises.instantiate`: the noise seeded from the hash of its id.
    pub fn noise(&mut self, id: &str) -> Result<Arc<NormalNoise>, String> {
        let id = canonical_id(id);
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

    fn push_node(&mut self, kind: NodeKind, uses_y: bool) -> NodeId {
        let id = NodeId(self.arena.nodes.len() as u32);
        self.arena.nodes.push(Node { kind, uses_y, slot: None });
        id
    }

    fn ensure_slot(&mut self, id: NodeId) {
        let node = &self.arena.nodes[id.0 as usize];
        if node.slot.is_some() || matches!(node.kind, NodeKind::Const(_) | NodeKind::Gradient { .. }) {
            return;
        }
        let slot = if !node.uses_y {
            if self.arena.num_xz < 64 {
                let s = self.arena.num_xz;
                self.arena.num_xz += 1;
                Some(s)
            } else {
                None
            }
        } else if self.arena.num_xyz < 64 {
            let s = self.arena.num_xyz;
            self.arena.num_xyz += 1;
            Some(s)
        } else {
            None
        };
        self.arena.nodes[id.0 as usize].slot = slot;
    }

    fn uses_y(&self, id: NodeId) -> bool {
        self.arena.nodes[id.0 as usize].uses_y
    }

    fn spline(&mut self, v: &Value) -> Result<u32, String> {
        match v {
            Value::Number(n) => {
                let id = self.arena.splines.len() as u32;
                self.arena.splines.push(SplineNode::Const(n.as_f64().unwrap_or(0.0) as f32));
                Ok(id)
            }
            Value::Object(o) => {
                let coord = self.build_id(o.get("coordinate").ok_or("spline without a coordinate")?)?;
                self.ensure_slot(coord);
                let points = o.get("points").and_then(Value::as_array).ok_or("spline without points")?;
                let mut tmp = Vec::with_capacity(points.len());
                let mut uses_y = self.uses_y(coord);
                for p in points {
                    let loc = p.get("location").and_then(Value::as_f64).ok_or("spline point without a location")? as f32;
                    let deriv = p.get("derivative").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    let val = self.spline(p.get("value").ok_or("spline point without a value")?)?;
                    uses_y |= self.arena.splines[val as usize].uses_y();
                    tmp.push(SplinePoint { loc, val, deriv });
                }
                let start = self.arena.spline_pts.len() as u32;
                let len = tmp.len() as u32;
                self.arena.spline_pts.extend(tmp);
                let id = self.arena.splines.len() as u32;
                self.arena.splines.push(SplineNode::Multi { coord: coord.0, start, len, uses_y });
                Ok(id)
            }
            _ => Err(format!("not a spline: {v}")),
        }
    }

    pub fn build(&mut self, v: &Value) -> Result<Df, String> {
        let id = self.build_id(v)?;
        Ok(Df::from_parts(Arc::new(self.arena.clone()), id))
    }

    pub fn build_id(&mut self, v: &Value) -> Result<NodeId, String> {
        match v {
            Value::Number(n) => Ok(self.push_node(NodeKind::Const(n.as_f64().unwrap_or(0.0) as f32), false)),
            Value::String(id) => {
                let canon = canonical_id(id);
                if let Some(&existing) = self.named.get(&canon) {
                    return Ok(existing);
                }
                let val = self.json("density_function", &canon)?;
                let nid = self.build_id(&val).map_err(|e| format!("{id}: {e}"))?;
                self.ensure_slot(nid);
                self.named.insert(canon, nid);
                Ok(nid)
            }
            Value::Object(o) => {
                let ty = o.get("type").and_then(Value::as_str).ok_or("density function without a type")?;
                let noise_of = |b: &mut Self| -> Result<Arc<NormalNoise>, String> {
                    let id = o.get("noise").and_then(Value::as_str).ok_or(format!("{ty} without a noise"))?;
                    b.noise(id)
                };
                match strip(ty) {
                    "cache" | "flat_cache" | "cache_2d" | "cache_once" | "cache_all_in_cell" | "interpolated" => {
                        let nid = self.build_id(o.get("argument").or(o.get("input")).ok_or(format!("{ty} without an input"))?)?;
                        self.ensure_slot(nid);
                        Ok(nid)
                    }
                    "constant" => self.build_id(o.get("argument").or(o.get("value")).ok_or("constant without a value")?),
                    "shift_a" => {
                        let n = noise_of(self)?;
                        let key = (0u8, Arc::as_ptr(&n) as usize);
                        if let Some(&existing) = self.shifts.get(&key) {
                            return Ok(existing);
                        }
                        let nid = self.push_node(NodeKind::ShiftA(n), false);
                        self.ensure_slot(nid);
                        self.shifts.insert(key, nid);
                        Ok(nid)
                    }
                    "shift_b" => {
                        let n = noise_of(self)?;
                        let key = (1u8, Arc::as_ptr(&n) as usize);
                        if let Some(&existing) = self.shifts.get(&key) {
                            return Ok(existing);
                        }
                        let nid = self.push_node(NodeKind::ShiftB(n), false);
                        self.ensure_slot(nid);
                        self.shifts.insert(key, nid);
                        Ok(nid)
                    }
                    "shift" => {
                        let n = noise_of(self)?;
                        let key = (2u8, Arc::as_ptr(&n) as usize);
                        if let Some(&existing) = self.shifts.get(&key) {
                            return Ok(existing);
                        }
                        let nid = self.push_node(NodeKind::Shift(n), true);
                        self.ensure_slot(nid);
                        self.shifts.insert(key, nid);
                        Ok(nid)
                    }
                    "noise" | "shifted_noise" => {
                        let noise = noise_of(self)?;
                        let num = |k: &str| o.get(k).and_then(Value::as_f64);
                        let xz = num("xz_scale").unwrap_or(1.0);
                        let y = num("y_scale").unwrap_or(1.0);
                        let keys = ["shift_x", "shift_y", "shift_z"];
                        let mut uses_y = y != 0.0;
                        let shift = if keys.iter().any(|k| o.contains_key(*k)) {
                            let mut s = [0u32; 3];
                            for (idx, k) in keys.iter().enumerate() {
                                let cid = match o.get(*k) {
                                    Some(v) => self.build_id(v)?,
                                    None => self.push_node(NodeKind::Const(0.0), false),
                                };
                                uses_y |= self.uses_y(cid);
                                s[idx] = cid.0;
                            }
                            Some(s)
                        } else {
                            None
                        };
                        Ok(self.push_node(NodeKind::Noise { noise, xz, y, shift }, uses_y))
                    }
                    "blend_alpha" => Ok(self.push_node(NodeKind::Const(1.0), false)),
                    "blend_offset" => Ok(self.push_node(NodeKind::Const(0.0), false)),
                    "blend_density" => self.build_id(o.get("argument").or(o.get("input")).ok_or("blend_density without an input")?),
                    t @ ("add" | "sub" | "mul" | "div" | "min" | "max") => {
                        let op = match t { "add" => Op2::Add, "sub" => Op2::Sub, "mul" => Op2::Mul, "div" => Op2::Div, "min" => Op2::Min, _ => Op2::Max };
                        let l = self.build_id(o.get("left").or(o.get("argument1")).ok_or(format!("{t} without a left side"))?)?;
                        let r = self.build_id(o.get("right").or(o.get("argument2")).ok_or(format!("{t} without a right side"))?)?;
                        let uses_y = self.uses_y(l) || self.uses_y(r);
                        Ok(self.push_node(NodeKind::Bin(op, l.0, r.0), uses_y))
                    }
                    t @ ("abs" | "square" | "cube" | "sqrt" | "half_negative" | "quarter_negative" | "reciprocal" | "negate" | "squeeze" | "log" | "sign") => {
                        let op = match t {
                            "abs" => Op1::Abs, "square" => Op1::Square, "cube" => Op1::Cube, "sqrt" => Op1::Sqrt,
                            "half_negative" => Op1::HalfNegative, "quarter_negative" => Op1::QuarterNegative,
                            "reciprocal" => Op1::Reciprocal, "negate" => Op1::Negate, "squeeze" => Op1::Squeeze,
                            "log" => Op1::Log, _ => Op1::Sign,
                        };
                        let i = self.build_id(o.get("input").or(o.get("argument")).ok_or(format!("{t} without an input"))?)?;
                        let uses_y = self.uses_y(i);
                        Ok(self.push_node(NodeKind::Un(op, i.0), uses_y))
                    }
                    "clamp" => {
                        let i = self.build_id(o.get("input").ok_or("clamp without an input")?)?;
                        let f = |k: &str| o.get(k).and_then(Value::as_f64).map(|v| v as f32).ok_or(format!("clamp without {k}"));
                        let uses_y = self.uses_y(i);
                        Ok(self.push_node(NodeKind::Clamp(i.0, f("min")?, f("max")?), uses_y))
                    }
                    "lerp" => {
                        let a = self.build_id(o.get("alpha").ok_or("lerp without alpha")?)?;
                        let f = self.build_id(o.get("first").ok_or("lerp without first")?)?;
                        let s = self.build_id(o.get("second").ok_or("lerp without second")?)?;
                        if let NodeKind::Const(t) = self.arena.nodes[a.0 as usize].kind {
                            if t == 0.0 {
                                return Ok(f);
                            }
                            if t == 1.0 {
                                return Ok(s);
                            }
                        }
                        let uses_y = self.uses_y(a) || self.uses_y(f) || self.uses_y(s);
                        Ok(self.push_node(NodeKind::Lerp(a.0, f.0, s.0), uses_y))
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
                        Ok(self.push_node(
                            NodeKind::Gradient { axis, from, min: from.min(to), max: from.max(to), from_value: fv, factor: (tv - fv) / (to - from) as f32 },
                            axis == 1,
                        ))
                    }
                    "spline" => {
                        let sid = self.spline(o.get("spline").ok_or("spline without a spline")?)?;
                        let uses_y = self.arena.splines[sid as usize].uses_y();
                        Ok(self.push_node(NodeKind::Spline(sid), uses_y))
                    }
                    "find_top_surface" => {
                        let density = self.build_id(o.get("density").ok_or("find_top_surface without density")?)?;
                        let upper = self.build_id(o.get("upper_bound").ok_or("find_top_surface without upper_bound")?)?;
                        let lower = o.get("lower_bound").and_then(Value::as_i64).ok_or("find_top_surface without lower_bound")? as i32;
                        let cell = o.get("cell_height").and_then(Value::as_i64).ok_or("find_top_surface without cell_height")? as i32;
                        let uses_y = self.uses_y(upper);
                        Ok(self.push_node(NodeKind::TopSurface { density: density.0, upper: upper.0, lower, cell }, uses_y))
                    }
                    "slice" => {
                        let axis = match o.get("axis").and_then(Value::as_str).unwrap_or("y") { "x" => 0, "y" => 1, _ => 2 };
                        let coord = o.get("coordinate").and_then(Value::as_i64).unwrap_or(0) as i32;
                        let input = self.build_id(o.get("input").or(o.get("argument")).ok_or("slice without an input")?)?;
                        let uses_y = axis != 1 && self.uses_y(input);
                        Ok(self.push_node(NodeKind::Slice { axis, coord, input: input.0 }, uses_y))
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
                        Ok(self.push_node(NodeKind::Distance { metric, point: [g(0), g(1), g(2)] }, true))
                    }
                    "end_outer_islands" | "end_islands" => {
                        if let Some(existing) = self.end_islands {
                            return Ok(existing);
                        }
                        let mut r = crate::random::Legacy::new(self.seed);
                        for _ in 0..17292 {
                            r.next_int();
                        }
                        let nid = self.push_node(NodeKind::EndIslands(Arc::new(EndIslands::new(Simplex::new(&mut r, true)))), false);
                        self.ensure_slot(nid);
                        self.end_islands = Some(nid);
                        Ok(nid)
                    }
                    other => Err(format!("density function type {other:?} is not supported yet")),
                }
            }
            _ => Err(format!("not a density function: {v}")),
        }
    }
}
