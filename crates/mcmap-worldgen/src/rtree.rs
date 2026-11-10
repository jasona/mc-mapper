//! Biome lookup: the game's `Climate.RTree`, built the same way (19 children per node, the same
//! sort keys, stable sorts) and searched the same way, so ties between equally distant boxes
//! resolve to the same biome. The game seeds each search with the previous result on that
//! thread; `search` takes that hint explicitly.

use serde::Deserialize;

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
const CHILDREN: usize = 19;

/// One entry of the parameter list: a box in climate space and its biome.
#[derive(Clone, Debug)]
pub struct Entry {
    pub space: [Param; DIMS],
    pub biome: u16,
}

#[derive(Clone, Debug)]
enum Node {
    Leaf(u32),
    Sub { space: [Param; DIMS], children: Vec<Node> },
}

pub struct RTree {
    entries: Vec<Entry>,
    root: Node,
}

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
    let mut out = Vec::with_capacity(raw.entries.len());
    for e in raw.entries {
        let id = match names.iter().position(|n| *n == e.biome) {
            Some(i) => i,
            None => {
                names.push(e.biome.clone());
                names.len() - 1
            }
        };
        let p = |a: [i64; 2]| Param { min: a[0], max: a[1] };
        out.push(Entry {
            space: [p(e.temperature), p(e.humidity), p(e.continentalness), p(e.erosion), p(e.depth), p(e.weirdness), Param { min: e.offset, max: e.offset }],
            biome: id as u16,
        });
    }
    Ok((out, names))
}

fn space_of(n: &Node, entries: &[Entry]) -> [Param; DIMS] {
    match n {
        Node::Leaf(i) => entries[*i as usize].space,
        Node::Sub { space, .. } => *space,
    }
}

fn mid(p: &Param) -> i64 {
    (p.min + p.max) / 2
}

fn subtree(children: Vec<Node>, entries: &[Entry]) -> Node {
    let mut space = space_of(&children[0], entries);
    for c in &children[1..] {
        let s = space_of(c, entries);
        for k in 0..DIMS {
            space[k] = space[k].span(&s[k]);
        }
    }
    Node::Sub { space, children }
}

fn cost(s: &[Param; DIMS]) -> i64 {
    s.iter().map(|p| (p.max - p.min).abs()).sum()
}

fn sort_nodes(list: &mut [Node], entries: &[Entry], dim: usize, abs: bool) {
    let key = |n: &Node| -> [i64; DIMS] {
        let s = space_of(n, entries);
        let mut k = [0; DIMS];
        for (j, slot) in k.iter_mut().enumerate() {
            let m = mid(&s[(dim + j) % DIMS]);
            *slot = if abs { m.abs() } else { m };
        }
        k
    };
    list.sort_by_cached_key(key);
}

fn bucketize(list: Vec<Node>, entries: &[Entry]) -> Vec<Node> {
    let per = (CHILDREN as f64).powf(((list.len() as f64 - 0.01).ln() / (CHILDREN as f64).ln()).floor()) as usize;
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for n in list {
        cur.push(n);
        if cur.len() >= per {
            out.push(subtree(std::mem::take(&mut cur), entries));
        }
    }
    if !cur.is_empty() {
        out.push(subtree(cur, entries));
    }
    out
}

fn build(mut list: Vec<Node>, entries: &[Entry]) -> Node {
    if list.len() == 1 {
        return list.pop().unwrap();
    }
    if list.len() <= CHILDREN {
        list.sort_by_cached_key(|n| space_of(n, entries).iter().map(|p| mid(p).abs()).sum::<i64>());
        return subtree(list, entries);
    }
    let mut best = i64::MAX;
    let mut best_dim = 0;
    let mut best_buckets: Option<Vec<Node>> = None;
    for d in 0..DIMS {
        sort_nodes(&mut list, entries, d, false);
        let buckets = bucketize(list.clone(), entries);
        let c: i64 = buckets.iter().map(|b| cost(&space_of(b, entries))).sum();
        if best > c {
            best = c;
            best_dim = d;
            best_buckets = Some(buckets);
        }
    }
    let mut buckets = best_buckets.unwrap();
    sort_nodes(&mut buckets, entries, best_dim, true);
    let children = buckets
        .into_iter()
        .map(|b| match b {
            Node::Sub { children, .. } => build(children, entries),
            leaf => leaf,
        })
        .collect();
    subtree(children, entries)
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

impl RTree {
    pub fn new(entries: Vec<Entry>) -> RTree {
        assert!(!entries.is_empty(), "need at least one value to build the search tree");
        let leaves = (0..entries.len() as u32).map(Node::Leaf).collect();
        let root = build(leaves, &entries);
        RTree { entries, root }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Nearest entry to `target` (the six climate values; offset 0). `hint` is the previous
    /// result, as the game keeps it per thread; it is updated.
    pub fn search(&self, target: &[i64; 6], hint: &mut Option<u32>) -> u16 {
        let t = [target[0], target[1], target[2], target[3], target[4], target[5], 0];
        let start = hint.filter(|&h| (h as usize) < self.entries.len());
        let leaf = self.search_node(&self.root, &t, start);
        *hint = Some(leaf);
        self.entries[leaf as usize].biome
    }

    fn search_node(&self, node: &Node, t: &[i64; DIMS], candidate: Option<u32>) -> u32 {
        let children = match node {
            Node::Leaf(i) => return *i,
            Node::Sub { children, .. } => children,
        };
        let mut best = match candidate {
            None => i64::MAX,
            Some(c) => distance(&self.entries[c as usize].space, t),
        };
        let mut res = candidate;
        for child in children {
            let d1 = distance(&space_of(child, &self.entries), t);
            if best > d1 {
                let leaf = self.search_node(child, t, res);
                let d2 = match child {
                    Node::Leaf(i) if *i == leaf => d1,
                    _ => distance(&self.entries[leaf as usize].space, t),
                };
                if best > d2 {
                    best = d2;
                    res = Some(leaf);
                }
            }
        }
        res.expect("a subtree always yields a leaf")
    }

    /// Brute force, for tests: first entry at the smallest distance.
    pub fn nearest_brute_force(&self, target: &[i64; 6]) -> u16 {
        let t = [target[0], target[1], target[2], target[3], target[4], target[5], 0];
        let mut best = (i64::MAX, 0u16);
        for e in &self.entries {
            let d = distance(&e.space, &t);
            if d < best.0 {
                best = (d, e.biome);
            }
        }
        best.1
    }
}
