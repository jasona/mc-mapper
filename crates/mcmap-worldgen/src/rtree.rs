//! Biome lookup: the game's `Climate.RTree`, built the same way (19 children per node, the same
//! sort keys, stable sorts) and flattened into contiguous 64-byte SIMD-friendly nodes so ties
//! between equally distant boxes resolve to the exact same biome without pointer chasing.
//! The game seeds each search with the previous result on that thread; `search` takes that hint
//! explicitly.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Param {
    pub min: i64,
    pub max: i64,
}

impl Param {
    #[inline]
    fn distance(&self, v: i64) -> i64 {
        let a = v - self.max;
        let b = self.min - v;
        if a > 0 { a } else { b.max(0) }
    }

    fn span(&self, o: &Param) -> Param {
        Param { min: self.min.min(o.min), max: self.max.max(o.max) }
    }
}

const DIMS: usize = 7;
const LANES: usize = 8;
const CHILDREN: usize = 19;
/// Safe bound for 32-bit coordinate differences whose squared 7-D sum fits below `i64::MAX`.
const COMPACT_LIMIT: i64 = 500_000_000;

/// One entry of the parameter list: a box in climate space and its biome.
#[derive(Clone, Debug)]
pub struct Entry {
    pub space: [Param; DIMS],
    pub biome: u16,
}

#[derive(Clone, Debug)]
struct BuildNode {
    space: [Param; DIMS],
    mids: [i64; DIMS],
    cost: i64,
    leaf: Option<u32>,
    children: Vec<u32>,
}

/// Pure 8-lane `i32` bounding box (64 bytes = 1 cache line) for clean 256-bit SIMD distance math.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
struct FlatBounds {
    mins: [i32; LANES],
    maxs: [i32; LANES],
}

#[derive(Clone, Copy, Debug)]
struct FlatMeta {
    /// For a leaf (`len == 0`): entry index. For a subtree (`len > 0`): start index of children.
    index: u32,
    len: u32,
}

struct RTreeInner {
    entries: Vec<Entry>,
    biomes: Box<[u16]>,
    leaf_of: Box<[u32]>,
    bounds: Box<[FlatBounds]>,
    meta: Box<[FlatMeta]>,
    wide_spaces: Box<[[Param; DIMS]]>,
    compact: bool,
}

/// Shareable multi-noise R-tree. Cloning is `O(1)`.
#[derive(Clone)]
pub struct RTree(Arc<RTreeInner>);

#[derive(Deserialize)]
struct RawEntry {
    biome: String,
    temperature: [i64; 2],
    humidity: [i64; 2],
    continentalness: [i64; 2],
    erosion: [i64; 2],
    depth: [i64; 2],
    weirdness: [i64; 2],
    offset: i64,
}

#[derive(Deserialize)]
struct RawList {
    entries: Vec<RawEntry>,
}

/// Parse the oracle's `params` JSON. Biome ids index `names`, in first-seen order.
pub fn parse_parameters(json: &str) -> Result<(Vec<Entry>, Vec<String>), String> {
    let raw: RawList = serde_json::from_str(json).map_err(|e| format!("parameter list: {e}"))?;
    let mut names: Vec<String> = Vec::new();
    let mut by_name: HashMap<String, u16> = HashMap::new();
    let mut out = Vec::with_capacity(raw.entries.len());
    for e in raw.entries {
        let id = match by_name.get(&e.biome) {
            Some(&i) => i,
            None => {
                let i = names.len() as u16;
                by_name.insert(e.biome.clone(), i);
                names.push(e.biome);
                i
            }
        };
        let p = |a: [i64; 2]| Param { min: a[0], max: a[1] };
        out.push(Entry {
            space: [p(e.temperature), p(e.humidity), p(e.continentalness), p(e.erosion), p(e.depth), p(e.weirdness), Param { min: e.offset, max: e.offset }],
            biome: id,
        });
    }
    Ok((out, names))
}

fn calc_mids(s: &[Param; DIMS]) -> [i64; DIMS] {
    let mut m = [0i64; DIMS];
    for k in 0..DIMS {
        m[k] = (s[k].min + s[k].max) / 2;
    }
    m
}

fn calc_cost(s: &[Param; DIMS]) -> i64 {
    s.iter().map(|p| (p.max - p.min).abs()).sum()
}

fn push_subtree(children: Vec<u32>, pool: &mut Vec<BuildNode>) -> u32 {
    let mut space = pool[children[0] as usize].space;
    for &c in &children[1..] {
        let s = pool[c as usize].space;
        for k in 0..DIMS {
            space[k] = space[k].span(&s[k]);
        }
    }
    let id = pool.len() as u32;
    pool.push(BuildNode {
        mids: calc_mids(&space),
        cost: calc_cost(&space),
        space,
        leaf: None,
        children,
    });
    id
}

fn sort_nodes(list: &mut [u32], pool: &[BuildNode], dim: usize, abs: bool) {
    list.sort_by_key(|&id| {
        let mids = &pool[id as usize].mids;
        let mut k = [0i64; DIMS];
        for (j, slot) in k.iter_mut().enumerate() {
            let m = mids[(dim + j) % DIMS];
            *slot = if abs { m.abs() } else { m };
        }
        k
    });
}

fn bucketize(list: &[u32], pool: &mut Vec<BuildNode>) -> Vec<u32> {
    let per = (CHILDREN as f64).powf(((list.len() as f64 - 0.01).ln() / (CHILDREN as f64).ln()).floor()) as usize;
    let mut out = Vec::with_capacity(list.len().div_ceil(per));
    for chunk in list.chunks(per) {
        out.push(push_subtree(chunk.to_vec(), pool));
    }
    out
}

fn build(mut list: Vec<u32>, pool: &mut Vec<BuildNode>) -> u32 {
    if list.len() == 1 {
        return list[0];
    }
    if list.len() <= CHILDREN {
        list.sort_by_key(|&id| pool[id as usize].mids.iter().map(|m| m.abs()).sum::<i64>());
        return push_subtree(list, pool);
    }
    let mark = pool.len();
    let mut best = i64::MAX;
    let mut best_dim = 0;
    let mut best_groups: Vec<Vec<u32>> = Vec::new();
    let per = (CHILDREN as f64).powf(((list.len() as f64 - 0.01).ln() / (CHILDREN as f64).ln()).floor()) as usize;
    for d in 0..DIMS {
        sort_nodes(&mut list, pool, d, false);
        pool.truncate(mark);
        let buckets = bucketize(&list, pool);
        let c: i64 = buckets.iter().map(|&b| pool[b as usize].cost).sum();
        if best > c {
            best = c;
            best_dim = d;
            best_groups = list.chunks(per).map(|ch| ch.to_vec()).collect();
        }
    }
    pool.truncate(mark);
    let mut buckets: Vec<u32> = best_groups.into_iter().map(|g| push_subtree(g, pool)).collect();
    sort_nodes(&mut buckets, pool, best_dim, true);
    let bucket_children: Vec<Vec<u32>> = buckets.iter().map(|&b| std::mem::take(&mut pool[b as usize].children)).collect();
    let children: Vec<u32> = buckets
        .into_iter()
        .zip(bucket_children)
        .map(|(b, ch)| if pool[b as usize].leaf.is_some() { b } else { build(ch, pool) })
        .collect();
    push_subtree(children, pool)
}

fn make_flat(node: &BuildNode) -> (FlatBounds, FlatMeta, [Param; DIMS]) {
    let s = node.space;
    let mut mins = [0i32; LANES];
    let mut maxs = [0i32; LANES];
    for k in 0..DIMS {
        mins[k] = s[k].min.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        maxs[k] = s[k].max.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    }
    let index = node.leaf.unwrap_or(0);
    (FlatBounds { mins, maxs }, FlatMeta { index, len: 0 }, s)
}

#[inline(always)]
fn dist_compact(b: &FlatBounds, t: &[i32; LANES]) -> i64 {
    let mut d = 0i64;
    for k in 0..LANES {
        let hi = (t[k] - b.maxs[k]).max(0);
        let lo = (b.mins[k] - t[k]).max(0);
        let v = (hi + lo) as i64;
        d += v * v;
    }
    d
}

#[inline]
fn distance(s: &[Param; DIMS], t: &[i64; DIMS]) -> i64 {
    let mut d = 0i64;
    for k in 0..DIMS {
        let v = s[k].distance(t[k]);
        d = d.wrapping_add(v.wrapping_mul(v));
    }
    d
}

impl RTreeInner {
    fn search_compact(&self, node_idx: usize, t: &[i32; LANES], mut res: Option<u32>, mut best: i64) -> (Option<u32>, i64) {
        let m = self.meta[node_idx];
        let start = m.index as usize;
        let end = start + m.len as usize;
        for ci in start..end {
            let d1 = dist_compact(&self.bounds[ci], t);
            if best > d1 {
                let cm = self.meta[ci];
                if cm.len == 0 {
                    best = d1;
                    res = Some(cm.index);
                } else {
                    (res, best) = self.search_compact(ci, t, res, best);
                }
                if best == 0 {
                    break;
                }
            }
        }
        (res, best)
    }

    fn search_wide(&self, node_idx: usize, t: &[i64; DIMS], mut res: Option<u32>, mut best: i64) -> (Option<u32>, i64) {
        let m = self.meta[node_idx];
        let start = m.index as usize;
        let end = start + m.len as usize;
        for ci in start..end {
            let d1 = distance(&self.wide_spaces[ci], t);
            if best > d1 {
                let cm = self.meta[ci];
                if cm.len == 0 {
                    best = d1;
                    res = Some(cm.index);
                } else {
                    (res, best) = self.search_wide(ci, t, res, best);
                }
            }
        }
        (res, best)
    }

    fn search_wide_entries(&self, node_idx: usize, t: &[i64; DIMS], mut res: Option<u32>, mut best: i64) -> (Option<u32>, i64) {
        let m = self.meta[node_idx];
        let start = m.index as usize;
        let end = start + m.len as usize;
        for ci in start..end {
            let b = &self.bounds[ci];
            let mut sp = [Param { min: 0, max: 0 }; DIMS];
            for k in 0..DIMS {
                sp[k] = Param { min: b.mins[k] as i64, max: b.maxs[k] as i64 };
            }
            let d1 = distance(&sp, t);
            if best > d1 {
                let cm = self.meta[ci];
                if cm.len == 0 {
                    best = d1;
                    res = Some(cm.index);
                } else {
                    (res, best) = self.search_wide_entries(ci, t, res, best);
                }
            }
        }
        (res, best)
    }
}

impl RTree {
    pub fn new(entries: Vec<Entry>) -> RTree {
        assert!(!entries.is_empty(), "need at least one value to build the search tree");
        let compact = entries.iter().all(|e| {
            e.space.iter().all(|p| p.min <= p.max && p.min.abs() <= COMPACT_LIMIT && p.max.abs() <= COMPACT_LIMIT)
        });
        let biomes: Box<[u16]> = entries.iter().map(|e| e.biome).collect();

        let mut pool = Vec::with_capacity(entries.len() * 2);
        for (i, e) in entries.iter().enumerate() {
            pool.push(BuildNode {
                space: e.space,
                mids: calc_mids(&e.space),
                cost: calc_cost(&e.space),
                leaf: Some(i as u32),
                children: Vec::new(),
            });
        }
        let leaves: Vec<u32> = (0..entries.len() as u32).collect();
        let root = build(leaves, &mut pool);

        let cap = entries.len() + entries.len() / 8 + 4;
        let mut bounds = Vec::with_capacity(cap);
        let mut meta = Vec::with_capacity(cap);
        let mut spaces = Vec::with_capacity(if compact { 0 } else { cap });
        let mut leaf_of = vec![0u32; entries.len()];
        let (rb, rm, rsp) = make_flat(&pool[root as usize]);
        bounds.push(rb);
        meta.push(rm);
        if !compact {
            spaces.push(rsp);
        }
        flatten_into_compact(root, 0, &pool, &mut bounds, &mut meta, if compact { None } else { Some(&mut spaces) }, &mut leaf_of);

        RTree(Arc::new(RTreeInner {
            entries,
            biomes,
            leaf_of: leaf_of.into_boxed_slice(),
            bounds: bounds.into_boxed_slice(),
            meta: meta.into_boxed_slice(),
            wide_spaces: spaces.into_boxed_slice(),
            compact,
        }))
    }

    pub fn entries(&self) -> &[Entry] {
        &self.0.entries
    }

    /// Nearest entry to `target` (the six climate values; offset 0). `hint` is the previous
    /// result, as the game keeps it per thread; it is updated.
    #[inline]
    pub fn search(&self, target: &[i64; 6], hint: &mut Option<u32>) -> u16 {
        let inner = &*self.0;
        if inner.meta[0].len == 0 {
            let leaf = inner.meta[0].index;
            *hint = Some(leaf);
            return inner.biomes[leaf as usize];
        }
        let start = hint.filter(|&h| (h as usize) < inner.biomes.len());
        if inner.compact && target.iter().all(|&v| v.abs() <= COMPACT_LIMIT) {
            let tc = [
                target[0] as i32,
                target[1] as i32,
                target[2] as i32,
                target[3] as i32,
                target[4] as i32,
                target[5] as i32,
                0,
                0,
            ];
            let best0 = match start {
                None => i64::MAX,
                Some(c) => {
                    let d = dist_compact(&inner.bounds[inner.leaf_of[c as usize] as usize], &tc);
                    if d == 0 {
                        *hint = Some(c);
                        return inner.biomes[c as usize];
                    }
                    d
                }
            };
            let (res, _) = inner.search_compact(0, &tc, start, best0);
            let leaf = res.expect("a subtree always yields a leaf");
            *hint = Some(leaf);
            return inner.biomes[leaf as usize];
        }
        let t = [target[0], target[1], target[2], target[3], target[4], target[5], 0];
        let best0 = match start {
            None => i64::MAX,
            Some(c) => distance(&inner.entries[c as usize].space, &t),
        };
        let (res, _) = if inner.wide_spaces.is_empty() {
            inner.search_wide_entries(0, &t, start, best0)
        } else {
            inner.search_wide(0, &t, start, best0)
        };
        let leaf = res.expect("a subtree always yields a leaf");
        *hint = Some(leaf);
        inner.biomes[leaf as usize]
    }

    /// Brute force, for tests: first entry at the smallest distance.
    pub fn nearest_brute_force(&self, target: &[i64; 6]) -> u16 {
        let t = [target[0], target[1], target[2], target[3], target[4], target[5], 0];
        let mut best = (i64::MAX, 0u16);
        for e in &self.0.entries {
            let d = distance(&e.space, &t);
            if d < best.0 {
                best = (d, e.biome);
            }
        }
        best.1
    }
}

fn flatten_into_compact(
    id: u32,
    slot: usize,
    pool: &[BuildNode],
    bounds: &mut Vec<FlatBounds>,
    meta: &mut Vec<FlatMeta>,
    mut spaces: Option<&mut Vec<[Param; DIMS]>>,
    leaf_of: &mut [u32],
) {
    let node = &pool[id as usize];
    if let Some(leaf) = node.leaf {
        leaf_of[leaf as usize] = slot as u32;
        return;
    }
    let start = bounds.len();
    for &c in &node.children {
        let (b, m, sp) = make_flat(&pool[c as usize]);
        bounds.push(b);
        meta.push(m);
        if let Some(sps) = spaces.as_deref_mut() {
            sps.push(sp);
        }
    }
    meta[slot].index = start as u32;
    meta[slot].len = node.children.len() as u32;
    for (offset, &c) in node.children.iter().enumerate() {
        flatten_into_compact(c, start + offset, pool, bounds, meta, spaces.as_deref_mut(), leaf_of);
    }
}
