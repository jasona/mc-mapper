//! Noise, in the 26.x single-precision form: `GradientNoise`/`PerlinNoise` return `f32` from
//! `f64` coordinates, `NoiseStack` sums `amplitude * octave` in `f32`, and `NormalNoise` builds
//! the octave list from `base_octave`, `base_amplitude`, `octave_count` and
//! `amplitude_modifiers`. Every operation is in the same order as the game's bytecode, because
//! a different association of float adds changes the last bit and, near a biome edge, the biome.

use crate::random::Xoroshiro;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0], [-1, 1, 0], [1, -1, 0], [-1, -1, 0],
    [1, 0, 1], [-1, 0, 1], [1, 0, -1], [-1, 0, -1],
    [0, 1, 1], [0, -1, 1], [0, 1, -1], [0, -1, -1],
    [1, 1, 0], [0, -1, 1], [-1, 1, 0], [0, -1, -1],
];

const GRADIENT_F32: [[f32; 4]; 16] = [
    [1.0, 1.0, 0.0, 0.0], [-1.0, 1.0, 0.0, 0.0], [1.0, -1.0, 0.0, 0.0], [-1.0, -1.0, 0.0, 0.0],
    [1.0, 0.0, 1.0, 0.0], [-1.0, 0.0, 1.0, 0.0], [1.0, 0.0, -1.0, 0.0], [-1.0, 0.0, -1.0, 0.0],
    [0.0, 1.0, 1.0, 0.0], [0.0, -1.0, 1.0, 0.0], [0.0, 1.0, -1.0, 0.0], [0.0, -1.0, -1.0, 0.0],
    [1.0, 1.0, 0.0, 0.0], [0.0, -1.0, 1.0, 0.0], [-1.0, 1.0, 0.0, 0.0], [0.0, -1.0, -1.0, 0.0],
];

const ROUND_OFF: f64 = 33_554_432.0;
const HALF_ROUND_OFF: f64 = 16_777_216.0;

/// `GradientNoise.wrap`: keeps coordinates within ±2^24 so the fraction keeps its precision.
#[inline(always)]
fn wrap(v: f64) -> f64 {
    if v >= -HALF_ROUND_OFF && v < HALF_ROUND_OFF { v } else { v - (v / ROUND_OFF + 0.5).floor() * ROUND_OFF }
}

#[inline(always)]
fn grad_dot(h: u8, x: f32, y: f32, z: f32) -> f32 {
    let g = GRADIENT_F32[(h & 15) as usize];
    g[0] * x + g[1] * y + g[2] * z
}

#[inline(always)]
fn smoothstep(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline(always)]
fn lerp(d: f32, a: f32, b: f32) -> f32 {
    a + d * (b - a)
}

#[inline(always)]
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
    perms: [u8; 512],
    ox: f64,
    oy: f64,
    oz: f64,
    iy0: i32,
    fy0: f32,
    sy0: f32,
}

impl Perlin {
    pub fn new(r: &mut impl NoiseRand) -> Perlin {
        let ox = r.next_double() * 256.0;
        let oy = r.next_double() * 256.0;
        let oz = r.next_double() * 256.0;
        let mut p256 = [0u8; 256];
        for (i, p) in p256.iter_mut().enumerate() {
            *p = i as u8;
        }
        for i in 0..256 {
            let j = r.next_int_bounded(256 - i as i32) as usize;
            p256.swap(i, i + j);
        }
        let mut perms = [0u8; 512];
        perms[..256].copy_from_slice(&p256);
        perms[256..].copy_from_slice(&p256);
        let y0 = wrap(0.0) + oy;
        let iy0 = y0.floor() as i32;
        let fy0 = (y0 - iy0 as f64) as f32;
        let sy0 = smoothstep(fy0);
        Perlin { perms, ox, oy, oz, iy0, fy0, sy0 }
    }

    #[inline(always)]
    pub fn get(&self, x: f64, y: f64, z: f64) -> f32 {
        let x = wrap(x) + self.ox;
        let z = wrap(z) + self.oz;
        let (ix, iz) = (x.floor() as i32, z.floor() as i32);
        let fx = (x - ix as f64) as f32;
        let fz = (z - iz as f64) as f32;
        if y == 0.0 {
            return self.sample_and_lerp_sy(ix, self.iy0, iz, fx, self.fy0, fz, self.sy0);
        }
        let y = wrap(y) + self.oy;
        let iy = y.floor() as i32;
        let fy = (y - iy as f64) as f32;
        self.sample_and_lerp_sy(ix, iy, iz, fx, fy, fz, smoothstep(fy))
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn sample_and_lerp_sy(&self, x: i32, y: i32, z: i32, fx: f32, fy: f32, fz: f32, sy: f32) -> f32 {
        let x0 = (x & 255) as usize;
        let y0 = (y & 255) as usize;
        let z0 = (z & 255) as usize;
        let i = self.perms[x0] as usize + y0;
        let j = self.perms[x0 + 1] as usize + y0;
        let k = self.perms[i] as usize + z0;
        let l = self.perms[i + 1] as usize + z0;
        let m = self.perms[j] as usize + z0;
        let n = self.perms[j + 1] as usize + z0;
        let fx1 = fx - 1.0;
        let fy1 = fy - 1.0;
        let fz1 = fz - 1.0;
        let d0 = grad_dot(self.perms[k], fx, fy, fz);
        let d1 = grad_dot(self.perms[m], fx1, fy, fz);
        let d2 = grad_dot(self.perms[l], fx, fy1, fz);
        let d3 = grad_dot(self.perms[n], fx1, fy1, fz);
        let d4 = grad_dot(self.perms[k + 1], fx, fy, fz1);
        let d5 = grad_dot(self.perms[m + 1], fx1, fy, fz1);
        let d6 = grad_dot(self.perms[l + 1], fx, fy1, fz1);
        let d7 = grad_dot(self.perms[n + 1], fx1, fy1, fz1);
        lerp3(smoothstep(fx), sy, smoothstep(fz), d0, d1, d2, d3, d4, d5, d6, d7)
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
    layers: Box<[Layer]>,
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
        Ok(NormalNoise { layers: layers.into_boxed_slice() })
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
        Ok(NormalNoise { layers: layers.into_boxed_slice() })
    }

    #[inline(always)]
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

type EndTlsSlot = (u64, i32, i32, Option<Arc<[u64; 64]>>);

thread_local! {
    static END_TLS: RefCell<(usize, [EndTlsSlot; 4])> = const {
        RefCell::new((0, [(0, 0, 0, None), (0, 0, 0, None), (0, 0, 0, None), (0, 0, 0, None)]))
    };
}

/// Cached End island height sampler: memoizes `a*a + b*b > 4096 && simplex(a, b) < -0.9` in
/// 64x64 lattice blocks (`[u64; 64]`), turning the 625-point Simplex scan into bitmask scans.
pub struct EndIslands {
    simplex: Simplex,
    id: u64,
    blocks: RwLock<HashMap<(i32, i32), Arc<[u64; 64]>>>,
}

impl std::fmt::Debug for EndIslands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndIslands").field("id", &self.id).finish()
    }
}

impl EndIslands {
    pub fn new(simplex: Simplex) -> EndIslands {
        static NEXT_END_ID: AtomicU64 = AtomicU64::new(1);
        EndIslands {
            simplex,
            id: NEXT_END_ID.fetch_add(1, Ordering::Relaxed),
            blocks: RwLock::new(HashMap::new()),
        }
    }

    fn compute_block(&self, bx: i32, bz: i32) -> Arc<[u64; 64]> {
        let a0 = (bx as i64) * 64;
        let b0 = (bz as i64) * 64;
        let mut bits = [0u64; 64];
        for (ra, row) in bits.iter_mut().enumerate() {
            let a = a0 + ra as i64;
            let mut word = 0u64;
            for rb in 0..64 {
                let b = b0 + rb as i64;
                if a * a + b * b > 4096 && self.simplex.get(a as f64, b as f64) < -0.9 {
                    word |= 1u64 << rb;
                }
            }
            *row = word;
        }
        Arc::new(bits)
    }

    #[inline]
    fn block(&self, bx: i32, bz: i32) -> Arc<[u64; 64]> {
        if let Some(hit) = END_TLS.with(|tls| {
            let cache = tls.borrow();
            for slot in &cache.1 {
                if slot.0 == self.id && slot.1 == bx && slot.2 == bz {
                    return slot.3.clone();
                }
            }
            None
        }) {
            return hit;
        }
        let arc = if let Some(b) = self.blocks.read().unwrap().get(&(bx, bz)).cloned() {
            b
        } else {
            let built = self.compute_block(bx, bz);
            let mut map = self.blocks.write().unwrap();
            if map.len() > 8192 {
                map.clear();
            }
            map.entry((bx, bz)).or_insert_with(|| built.clone()).clone()
        };
        END_TLS.with(|tls| {
            let mut cache = tls.borrow_mut();
            let idx = cache.0;
            cache.1[idx] = (self.id, bx, bz, Some(arc.clone()));
            cache.0 = (idx + 1) & 3;
        });
        arc
    }

    pub fn height(&self, x: i32, z: i32) -> f32 {
        let (i, j) = (x / 2, z / 2);
        let (k, l) = (x % 2, z % 2);
        let a_start = (i - 12) as i64;
        let a_end = (i + 12) as i64;
        let b_start = (j - 12) as i64;
        let bx0 = a_start.div_euclid(64) as i32;
        let bx1 = a_end.div_euclid(64) as i32;
        let bz0 = b_start.div_euclid(64) as i32;
        let bit0 = b_start.rem_euclid(64) as u32;
        const MASK25: u64 = (1u64 << 25) - 1;

        let b00 = self.block(bx0, bz0);
        let b01 = if bit0 > 39 { Some(self.block(bx0, bz0 + 1)) } else { None };
        let b10 = if bx1 != bx0 { Some(self.block(bx1, bz0)) } else { None };
        let b11 = if bx1 != bx0 && bit0 > 39 { Some(self.block(bx1, bz0 + 1)) } else { None };

        let mut h = -100.0f32;
        for m in -12..=12 {
            let a = (i + m) as i64;
            let ra = a.rem_euclid(64) as usize;
            let second_x = (a.div_euclid(64) as i32) != bx0;
            let w0 = if second_x { b10.as_ref().unwrap()[ra] } else { b00[ra] };
            let mut mask = if bit0 <= 39 {
                (w0 >> bit0) & MASK25
            } else {
                let w1 = if second_x { b11.as_ref().unwrap()[ra] } else { b01.as_ref().unwrap()[ra] };
                ((w0 >> bit0) | (w1 << (64 - bit0))) & MASK25
            };
            if mask == 0 {
                continue;
            }
            let fa = (a as f32).abs() * 3439.0;
            let g = (k - m * 2) as f32;
            let gg = g * g;
            while mask != 0 {
                let p = mask.trailing_zeros();
                mask &= mask - 1;
                let o = p as i32 - 12;
                let b = (j + o) as i64;
                let f = (fa + (b as f32).abs() * 147.0) % 13.0 + 9.0;
                let q = (l - o * 2) as f32;
                let v = (100.0 - (gg + q * q).sqrt() * f).clamp(-100.0, 80.0);
                h = h.max(v);
            }
        }
        h
    }
}
