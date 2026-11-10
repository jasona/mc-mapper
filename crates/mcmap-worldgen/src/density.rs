//! The part of the density-function language the surface climate needs, read from the pack's
//! JSON: references, caches (evaluation is uncached, so they pass through), constants, `noise`
//! with optional shifts, and the `shift_a`, `shift_b` and `shift` offset samplers. Anything else
//! is reported by name; splines and the 3-D terrain functions come with the depth layer.
//!
//! Each node evaluates at an integer block position, like the game's `sampleValue`.

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
        }
    }
}

/// Builds density functions for one seed, sharing noise instances by id.
pub struct Builder<'a> {
    files: &'a Files,
    random: Positional,
    noises: BTreeMap<String, Arc<NormalNoise>>,
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
        Builder { files, random, noises: BTreeMap::new() }
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
        let n = Arc::new(NormalNoise::create(&params, &mut self.random.from_hash_of(&id))?);
        self.noises.insert(id, n.clone());
        Ok(n)
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
                    other => Err(format!("density function type {other:?} is not supported yet")),
                }
            }
            _ => Err(format!("not a density function: {v}")),
        }
    }
}
