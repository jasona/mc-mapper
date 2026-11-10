//! Noise, in the 26.x single-precision form: `GradientNoise`/`PerlinNoise` return `f32` from
//! `f64` coordinates, `NoiseStack` sums `amplitude * octave` in `f32`, and `NormalNoise` builds
//! the octave list from `base_octave`, `base_amplitude`, `octave_count` and
//! `amplitude_modifiers`. Every operation is in the same order as the game's bytecode, because
//! a different association of float adds changes the last bit and, near a biome edge, the biome.

use crate::random::Xoroshiro;
use serde::Deserialize;

const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
    [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
    [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
    [1, 1, 0], [0, -1, 1], [-1, 1, 0], [0, -1, -1],
];

const ROUND_OFF: f64 = 33_554_432.0;
const HALF_ROUND_OFF: f64 = 16_777_216.0;

/// `GradientNoise.wrap`: keeps coordinates within ±2^24 so the fraction keeps its precision.
#[inline]
fn wrap(v: f64) -> f64 {
    if v >= -HALF_ROUND_OFF && v < HALF_ROUND_OFF { v } else { v - (v / ROUND_OFF + 0.5).floor() * ROUND_OFF }
}

#[inline]
fn grad_dot(h: i32, x: f32, y: f32, z: f32) -> f32 {
    let g = GRADIENT[(h & 15) as usize];
    g[0] as f32 * x + g[1] as f32 * y + g[2] as f32 * z
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(d: f32, a: f32, b: f32) -> f32 {
    a + d * (b - a)
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn lerp3(dx: f32, dy: f32, dz: f32, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32, g: f32, h: f32) -> f32 {
    lerp(dz, lerp(dy, lerp(dx, a, b), lerp(dx, c, d)), lerp(dy, lerp(dx, e, f), lerp(dx, g, h)))
}

/// What noise construction draws from: `RandomSource` in either algorithm.
pub trait NoiseRand {
    fn next_double(&mut self) -> f64;
    fn next_int_bounded(&mut self, bound: i32) -> i32;
}

impl NoiseRand for Xoroshiro {
    fn next_double(&mut self) -> f64 {
        Xoroshiro::next_double(self)
    }
    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        Xoroshiro::next_int_bounded(self, bound)
    }
}

impl NoiseRand for crate::random::Legacy {
    fn next_double(&mut self) -> f64 {
        crate::random::Legacy::next_double(self)
    }
    fn next_int_bounded(&mut self, bound: i32) -> i32 {
        crate::random::Legacy::next_int_bounded(self, bound)
    }
}

/// One octave: the game's `PerlinNoise` (improved Perlin noise with a random offset).
#[derive(Clone, Debug)]
pub struct Perlin {
    perms: [u8; 256],
    ox: f64,
    oy: f64,
    oz: f64,
}

impl Perlin {
    pub fn new(r: &mut impl NoiseRand) -> Perlin {
        let ox = r.next_double() * 256.0;
        let oy = r.next_double() * 256.0;
        let oz = r.next_double() * 256.0;
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = r.next_int_bounded(256 - i as i32) as usize;
            perms.swap(i, i + j);
        }
        Perlin { perms, ox, oy, oz }
    }

    #[inline]
    fn p(&self, i: i32) -> i32 {
        self.perms[(i & 255) as usize] as i32
    }

    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        let x = wrap(x) + self.ox;
        let y = wrap(y) + self.oy;
        let z = wrap(z) + self.oz;
        let (ix, iy, iz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
        let fx = (x - ix as f64) as f32;
        let fy = (y - iy as f64) as f32;
        let fz = (z - iz as f64) as f32;
        self.sample_and_lerp(ix, iy, iz, fx, fy, fz, fy)
    }

    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp(&self, x: i32, y: i32, z: i32, fx: f32, fy: f32, fz: f32, sy: f32) -> f32 {
        let i = self.p(x);
        let j = self.p(x.wrapping_add(1));
        let k = self.p(i.wrapping_add(y));
        let l = self.p(i.wrapping_add(y).wrapping_add(1));
        let m = self.p(j.wrapping_add(y));
        let n = self.p(j.wrapping_add(y).wrapping_add(1));
        let d0 = grad_dot(self.p(k.wrapping_add(z)), fx, fy, fz);
        let d1 = grad_dot(self.p(m.wrapping_add(z)), fx - 1.0, fy, fz);
        let d2 = grad_dot(self.p(l.wrapping_add(z)), fx, fy - 1.0, fz);
        let d3 = grad_dot(self.p(n.wrapping_add(z)), fx - 1.0, fy - 1.0, fz);
        let d4 = grad_dot(self.p(k.wrapping_add(z).wrapping_add(1)), fx, fy, fz - 1.0);
        let d5 = grad_dot(self.p(m.wrapping_add(z).wrapping_add(1)), fx - 1.0, fy, fz - 1.0);
        let d6 = grad_dot(self.p(l.wrapping_add(z).wrapping_add(1)), fx, fy - 1.0, fz - 1.0);
        let d7 = grad_dot(self.p(n.wrapping_add(z).wrapping_add(1)), fx - 1.0, fy - 1.0, fz - 1.0);
        lerp3(smoothstep(fx), smoothstep(sy), smoothstep(fz), d0, d1, d2, d3, d4, d5, d6, d7)
    }
}

/// A noise definition from `worldgen/noise/*.json` (26.x format).
#[derive(Clone, Debug, Deserialize)]
pub struct NoiseParams {
    pub base_octave: i32,
    pub base_amplitude: f64,
    pub octave_count: i32,
    #[serde(default)]
    pub amplitude_modifiers: Vec<f64>,
    /// "enabled" (the default), "disabled" or "legacy". Only the default is supported.
    #[serde(default)]
    pub normalize: Option<serde_json::Value>,
}

struct OctaveInfo {
    index: i32,
    frequency: f64,
    amplitude: f64,
}

fn build_octaves(p: &NoiseParams, normalize: bool) -> Vec<OctaveInfo> {
    let n = p.octave_count;
    let mut freq = 2f64.powi(p.base_octave);
    let mut amp = p.base_amplitude;
    if normalize {
        amp *= 0.5f64.powi(-(n - 1)) / (0.5f64.powi(-n) - 1.0);
    }
    let mut out = Vec::new();
    for i in 0..n {
        let m = if p.amplitude_modifiers.is_empty() { 1.0 } else { p.amplitude_modifiers[i as usize] };
        if m != 0.0 {
            out.push(OctaveInfo { index: p.base_octave + i, frequency: freq, amplitude: amp * m });
        }
        freq *= 2.0;
        amp *= 0.5;
    }
    out
}

fn normalization_factor(sum: f64, octaves: &[OctaveInfo]) -> f64 {
    let mut s = 0.0;
    for o in octaves {
        let d = 0.270_224_783_124_521_1 * o.amplitude.abs();
        s += d * d;
    }
    let dev = s.sqrt();
    if dev == 0.0 {
        return 0.0;
    }
    let denom = dev * 2f64.sqrt();
    (sum * 0.333_333_333_333_333_3) / denom
}

/// `DoubleStream.sum()`: Kahan-compensated, as the JDK (18+) does it.
fn java_double_sum(values: impl Iterator<Item = f64>) -> f64 {
    let (mut sum, mut comp, mut simple) = (0.0f64, 0.0f64, 0.0f64);
    for v in values {
        let t = v - comp;
        let velvel = sum + t;
        comp = (velvel - sum) - t;
        sum = velvel;
        simple += v;
    }
    let r = sum - comp;
    if r.is_nan() && simple.is_infinite() { simple } else { r }
}

#[derive(Clone, Debug)]
struct Layer {
    noise: Perlin,
    frequency: f64,
    amplitude: f32,
}

/// An instantiated `NormalNoise`: two Perlin stacks interleaved as one layer list.
#[derive(Clone, Debug)]
pub struct NormalNoise {
    layers: Vec<Layer>,
}

impl NormalNoise {
    pub fn create(p: &NoiseParams, r: &mut Xoroshiro) -> Result<NormalNoise, String> {
        let normalize = match p.normalize.as_ref().and_then(|v| v.as_str()) {
            None | Some("enabled") | Some("minecraft:enabled") => true,
            Some(other) => return Err(format!("noise normalization {other:?} is not supported yet")),
        };
        let octaves = build_octaves(p, normalize);
        let sum = java_double_sum(octaves.iter().map(|o| o.amplitude.abs()));
        let factor = normalization_factor(sum, &octaves);
        let a = r.fork_positional();
        let b = r.fork_positional();
        let mut layers = Vec::with_capacity(octaves.len() * 2);
        for o in &octaves {
            let seed = format!("octave_{}", o.index);
            let pa = Perlin::new(&mut a.from_hash_of(&seed));
            let pb = Perlin::new(&mut b.from_hash_of(&seed));
            let amp = (factor * o.amplitude) as f32;
            layers.push(Layer { noise: pa, frequency: o.frequency, amplitude: amp });
            layers.push(Layer { noise: pb, frequency: o.frequency * 1.018_126_888_217_522_7, amplitude: amp });
        }
        Ok(NormalNoise { layers })
    }

    /// `NormalNoise.createForLegacyNetherBiome`: two `LegacyFbmInitializer` stacks from one
    /// legacy random (seed + 0 for temperature, + 1 for vegetation), octaves created from the
    /// highest index down with 262 draws skipped for each octave outside the amplitude list.
    pub fn legacy_nether(p: &NoiseParams, r: &mut crate::random::Legacy) -> Result<NormalNoise, String> {
        let normalize = match p.normalize.as_ref().and_then(|v| v.as_str()) {
            None | Some("enabled") | Some("minecraft:enabled") => true,
            Some(other) => return Err(format!("noise normalization {other:?} is not supported yet")),
        };
        let octaves = build_octaves(p, normalize);
        let sum = java_double_sum(octaves.iter().map(|o| o.amplitude.abs()));
        let factor = normalization_factor(sum, &octaves);
        let mods: Vec<f64> = if p.amplitude_modifiers.is_empty() { vec![1.0; p.octave_count as usize] } else { p.amplitude_modifiers.clone() };
        let stack = |r: &mut crate::random::Legacy| -> Result<Vec<Layer>, String> {
            let n = mods.len() as i32;
            let top = -p.base_octave;
            let mut noises: Vec<Option<Perlin>> = vec![None; n as usize];
            let first = Perlin::new(r);
            if top >= 0 && top < n && mods[top as usize] != 0.0 {
                noises[top as usize] = Some(first);
            }
            let mut j = top - 1;
            while j >= 0 {
                if j < n && mods[j as usize] != 0.0 {
                    noises[j as usize] = Some(Perlin::new(r));
                } else {
                    for _ in 0..262 {
                        r.next_int();
                    }
                }
                j -= 1;
            }
            if top < n - 1 {
                return Err("positive octaves are not supported by the legacy initializer".into());
            }
            let mut freq = 2f64.powf(-(top as f64));
            let mut amp = 2f64.powf((n - 1) as f64) / (2f64.powf(n as f64) - 1.0);
            let mut out = Vec::new();
            for (i, noise) in noises.into_iter().enumerate() {
                if let Some(noise) = noise {
                    out.push(Layer { noise, frequency: freq, amplitude: (amp * mods[i]) as f32 });
                }
                freq *= 2.0;
                amp /= 2.0;
            }
            Ok(out)
        };
        let a = stack(r)?;
        let b = stack(r)?;
        let f5 = (factor * p.base_amplitude) as f32;
        let mut layers = Vec::with_capacity(a.len() * 2);
        for l in a {
            layers.push(Layer { noise: l.noise, frequency: l.frequency * 1.0, amplitude: l.amplitude * f5 });
        }
        for l in b {
            layers.push(Layer { noise: l.noise, frequency: l.frequency * 1.018_126_888_217_522_7, amplitude: l.amplitude * f5 });
        }
        Ok(NormalNoise { layers })
    }

    #[inline]
    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut s = 0f32;
        for l in &self.layers {
            let f = l.frequency;
            s += l.amplitude * l.noise.get(x * f, y * f, z * f);
        }
        s
    }
}

/// `SimplexNoise` (2-D), as the End islands use it: 26.x returns `f32`, computes in `f64`.
#[derive(Clone, Debug)]
pub struct Simplex {
    perms: [u8; 256],
    ox: f64,
    oy: f64,
}

impl Simplex {
    /// `new SimplexNoise(random, zeroOffset)`: three offsets drawn either way, scaled by 0 or 256.
    pub fn new(r: &mut crate::random::Legacy, zero_offset: bool) -> Simplex {
        let scale = if zero_offset { 0.0 } else { 256.0 };
        let ox = r.next_double() * scale;
        let oy = r.next_double() * scale;
        let _oz = r.next_double() * scale;
        let mut perms = [0u8; 256];
        for (i, p) in perms.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = r.next_int_bounded(256 - i as i32) as usize;
            perms.swap(i, i + j);
        }
        Simplex { perms, ox, oy }
    }

    #[inline]
    fn p(&self, i: i32) -> i32 {
        self.perms[(i & 255) as usize] as i32
    }

    #[inline]
    fn corner(h: i32, x: f64, y: f64) -> f64 {
        let t = 0.5 - x * x - y * y - 0.0 * 0.0;
        if t < 0.0 {
            0.0
        } else {
            let t = t * t;
            let g = GRADIENT[h as usize];
            t * t * (g[0] as f64 * x + g[1] as f64 * y + g[2] as f64 * 0.0)
        }
    }

    pub fn get(&self, x: f64, y: f64) -> f32 {
        let sqrt3 = 3f64.sqrt();
        let f2 = 0.5 * (sqrt3 - 1.0);
        let g2 = (3.0 - sqrt3) / 6.0;
        let (x, y) = (x + self.ox, y + self.oy);
        let s = (x + y) * f2;
        let i = (x + s).floor() as i32;
        let j = (y + s).floor() as i32;
        let t = (i + j) as f64 * g2;
        let x0 = x - (i as f64 - t);
        let y0 = y - (j as f64 - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - i1 as f64 + g2;
        let y1 = y0 - j1 as f64 + g2;
        let x2 = x0 - 1.0 + 2.0 * g2;
        let y2 = y0 - 1.0 + 2.0 * g2;
        let (ii, jj) = (i & 255, j & 255);
        let h0 = self.p(ii + self.p(jj)) % 12;
        let h1 = self.p(ii + i1 + self.p(jj + j1)) % 12;
        let h2 = self.p(ii + 1 + self.p(jj + 1)) % 12;
        let n = Self::corner(h0, x0, y0) + Self::corner(h1, x1, y1) + Self::corner(h2, x2, y2);
        (70.0 * n) as f32
    }
}

/// `EndIslandFunction.getHeightValue`: the outer islands' height at (x / 8, z / 8).
pub fn end_island_height(n: &Simplex, x: i32, z: i32) -> f32 {
    let (i, j) = (x / 2, z / 2);
    let (k, l) = (x % 2, z % 2);
    let mut h = -100.0f32;
    for m in -12..=12 {
        for o in -12..=12 {
            let (a, b) = ((i + m) as i64, (j + o) as i64);
            if a * a + b * b > 4096 && n.get(a as f64, b as f64) < -0.9 {
                let f = ((a as f32).abs() * 3439.0 + (b as f32).abs() * 147.0) % 13.0 + 9.0;
                let (g, q) = ((k - m * 2) as f32, (l - o * 2) as f32);
                let v = (100.0 - (g * g + q * q).sqrt() * f).clamp(-100.0, 80.0);
                h = h.max(v);
            }
        }
    }
    h
}
