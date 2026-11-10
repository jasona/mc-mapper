//! Tile pixels and their scheduler. Each tile is one `rune::Pixels`, rendered on a small pool of
//! worker threads and published with `Pixels::set`, which uploads and wakes the window without
//! re-running the view.
//!
//! Each view run asks for the visible tiles with a priority (distance from the view centre, in
//! tiles). Workers take the nearest queued tile first, and a queued tile the next view run no
//! longer asks for is dropped before anyone renders it, so a fast pan or zoom never leaves a
//! backlog of tiles nobody will see. While a newly requested tile waits for a worker, it is
//! immediately seeded from any cached ancestor or child tile in memory so zooming never flashes
//! blank squares. Finished tiles stay in a bounded LRU memory cache alongside their compact biome
//! IDs and optional hillshade so toggling Relief repaints in memory.
//!
//! Tiles are surface biomes when a world is loaded, else the M0 checkerboard.

use crate::diskcache::TileCache;
use mcmap_tiles::TileKey;
use mcmap_tiles::{biomes, checker, TILE};
use mcmap_worldgen::terrain::Terrain;
use mcmap_worldgen::Surface;
use rune::Pixels;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Most live GPU tiles kept: 512 tiles of 256 KiB each is 128 MiB of RGBA at most.
const CAPACITY: usize = 512;
/// Compact biome/shade tiles kept in RAM (~192 KiB each with shade) for instant Relief toggles and
/// ancestor/child zoom stand-ins.
const MEM_CAPACITY: usize = 1024;

#[derive(Default)]
struct MemTile {
    ids: Option<Arc<[u16]>>,
    shade: Option<Arc<[u8]>>,
    used: u64,
}

/// A seed's surface biomes and their colours.
pub struct World {
    pub seed: i64,
    pub zoom_seed: i64,
    pub surface: Surface,
    pub colors: Vec<[u8; 3]>,
    pub water: Vec<u8>,
    /// Where finished tiles' biome ids are kept on disk, if anywhere.
    pub cache: Option<TileCache>,
    /// The Overworld's terrain, for the hillshade.
    pub terrain: Option<Terrain>,
    mem: Mutex<HashMap<TileKey, MemTile>>,
    clock: AtomicU64,
}

impl World {
    pub fn new(seed: i64, surface: Surface) -> World {
        let colors = biomes::palette(&surface);
        let water = biomes::water_flags(&surface);
        World {
            seed,
            zoom_seed: mcmap_data::seeds::zoom_seed(seed),
            surface,
            colors,
            water,
            cache: None,
            terrain: None,
            mem: Mutex::new(HashMap::new()),
            clock: AtomicU64::new(1),
        }
    }

    pub fn with_terrain(mut self, terrain: Option<Terrain>) -> World {
        self.terrain = terrain;
        self
    }

    pub fn with_cache(mut self, cache: Option<TileCache>) -> World {
        self.cache = cache;
        self
    }

    #[inline]
    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed)
    }

    fn trim_mem(mem: &mut HashMap<TileKey, MemTile>) {
        if mem.len() <= MEM_CAPACITY + 256 {
            return;
        }
        let mut by_age: Vec<(u64, TileKey)> = mem.iter().map(|(k, v)| (v.used, *k)).collect();
        let excess = mem.len().saturating_sub(MEM_CAPACITY);
        by_age.select_nth_unstable_by_key(excess, |x| x.0);
        for &(_, k) in &by_age[..excess] {
            mem.remove(&k);
        }
    }

    fn ids_for(&self, key: TileKey, n: usize) -> Arc<[u16]> {
        let now = self.tick();
        if let Some(mt) = self.mem.lock().unwrap().get_mut(&key)
            && let Some(ids) = mt.ids.clone()
        {
            mt.used = now;
            return ids;
        }
        // Close-zoom Voronoi tiles (`level <= -4` in Overworld/Nether) cover at most 64x64 blocks
        // (~50 us to compute in memory), so skip disk I/O for them.
        let use_disk = self.surface.is_end() || key.level > -4;
        let vec = match use_disk.then(|| self.cache.as_ref()).flatten().and_then(|c| c.get(key, n)) {
            Some(ids) => ids,
            None => {
                let ids = biomes::ids_for_seed(&self.surface, self.zoom_seed, key);
                if use_disk && let Some(c) = &self.cache {
                    c.put(key, &ids);
                }
                ids
            }
        };
        let arc: Arc<[u16]> = Arc::from(vec);
        let mut mem = self.mem.lock().unwrap();
        Self::trim_mem(&mut mem);
        let entry = mem.entry(key).or_default();
        entry.ids = Some(arc.clone());
        entry.used = now;
        arc
    }

    fn shade_for(&self, terrain: &Terrain, key: TileKey, n: usize) -> Arc<[u8]> {
        let now = self.tick();
        if let Some(mt) = self.mem.lock().unwrap().get_mut(&key)
            && let Some(s) = mt.shade.clone()
        {
            mt.used = now;
            return s;
        }
        let use_disk = key.level > -4;
        let vec = match use_disk.then(|| self.cache.as_ref()).flatten().and_then(|c| c.get_shade(key, n)) {
            Some(s) => s,
            None => {
                let s = mcmap_tiles::relief::shade(terrain, key);
                if use_disk && let Some(c) = &self.cache {
                    c.put_shade(key, &s);
                }
                s
            }
        };
        let arc: Arc<[u8]> = Arc::from(vec);
        let mut mem = self.mem.lock().unwrap();
        Self::trim_mem(&mut mem);
        let entry = mem.entry(key).or_default();
        entry.shade = Some(arc.clone());
        entry.used = now;
        arc
    }

    pub(crate) fn paint_rgba(&self, ids: &[u16], shade: Option<&[u8]>) -> Vec<u8> {
        let mut px = biomes::paint(ids, &self.colors);
        if let Some(s) = shade {
            mcmap_tiles::relief::apply_cartographic(&mut px, s, ids, &self.water);
        }
        px
    }

    /// Synthesise an immediate stand-in RGBA tile from any cached ancestor (`parent` up to 5
    /// levels) and/or direct child tiles in memory so zooming in or out never flashes blank dark
    /// squares while background workers compute crisp tiles.
    pub(crate) fn standin_rgba(&self, key: TileKey, relief: bool) -> Option<Vec<u8>> {
        let side = TILE as usize;
        let want_shade = relief && self.terrain.is_some();
        let now = self.tick();

        type LayerPair = (Arc<[u16]>, Option<Arc<[u8]>>);
        let mut ancestor: Option<(u32, LayerPair)> = None;
        let mut children: [Option<LayerPair>; 4] = [None, None, None, None];
        let mut any_child = false;

        {
            let mut mem = self.mem.lock().unwrap();
            let mut anc = key;
            for shift in 1..=5u32 {
                anc = anc.parent();
                if let Some(mt) = mem.get_mut(&anc)
                    && let Some(ids) = mt.ids.clone()
                {
                    mt.used = now;
                    let sh = if want_shade { mt.shade.clone() } else { None };
                    ancestor = Some((shift, (ids, sh)));
                    break;
                }
            }
            for dz in 0..2i64 {
                for dx in 0..2i64 {
                    let ck = TileKey { level: key.level - 1, x: key.x * 2 + dx, z: key.z * 2 + dz };
                    if let Some(mt) = mem.get_mut(&ck)
                        && let Some(ids) = mt.ids.clone()
                    {
                        mt.used = now;
                        let sh = if want_shade { mt.shade.clone() } else { None };
                        children[(dz * 2 + dx) as usize] = Some((ids, sh));
                        any_child = true;
                    }
                }
            }
        }

        if ancestor.is_none() && !any_child {
            return None;
        }

        let mut ids_buf = vec![0u16; side * side];
        let mut shade_buf = if want_shade { Some(vec![128u8; side * side]) } else { None };
        let has_ancestor = ancestor.is_some();

        if let Some((shift, (anc_ids, anc_shade))) = ancestor {
            let sub = side >> shift;
            let mask = (1i64 << shift) - 1;
            let ox = (key.x.rem_euclid(1i64 << shift) & mask) as usize * sub;
            let oz = (key.z.rem_euclid(1i64 << shift) & mask) as usize * sub;
            for j in 0..side {
                let sj = oz + (j >> shift);
                let src_row = sj * side;
                let dst_row = j * side;
                for i in 0..side {
                    ids_buf[dst_row + i] = anc_ids[src_row + ox + (i >> shift)];
                }
                if let (Some(sb), Some(ash)) = (shade_buf.as_mut(), anc_shade.as_ref()) {
                    for i in 0..side {
                        sb[dst_row + i] = ash[src_row + ox + (i >> shift)];
                    }
                }
            }
        }

        let half = side / 2;
        let mut covered = [has_ancestor; 4];
        for qz in 0..2usize {
            for qx in 0..2usize {
                let q = qz * 2 + qx;
                let Some((c_ids, c_shade)) = &children[q] else { continue };
                covered[q] = true;
                let ox = qx * half;
                let oz = qz * half;
                for j in 0..half {
                    let dst_row = (oz + j) * side + ox;
                    let src_row0 = (j * 2) * side;
                    let src_row1 = src_row0 + side;
                    for i in 0..half {
                        ids_buf[dst_row + i] = c_ids[src_row0 + i * 2];
                    }
                    if let (Some(sb), Some(csh)) = (shade_buf.as_mut(), c_shade.as_ref()) {
                        for i in 0..half {
                            let s00 = csh[src_row0 + i * 2] as u16;
                            let s01 = csh[src_row0 + i * 2 + 1] as u16;
                            let s10 = csh[src_row1 + i * 2] as u16;
                            let s11 = csh[src_row1 + i * 2 + 1] as u16;
                            sb[dst_row + i] = ((s00 + s01 + s10 + s11 + 2) >> 2) as u8;
                        }
                    }
                }
            }
        }

        let mut rgba = self.paint_rgba(&ids_buf, shade_buf.as_deref());
        if !has_ancestor {
            for qz in 0..2usize {
                for qx in 0..2usize {
                    if !covered[qz * 2 + qx] {
                        let ox = qx * half;
                        let oz = qz * half;
                        for j in 0..half {
                            let row = (oz + j) * side + ox;
                            for i in 0..half {
                                let p = (row + i) * 4;
                                rgba[p..p + 4].copy_from_slice(&[20, 20, 26, 255]);
                            }
                        }
                    }
                }
            }
        }
        Some(rgba)
    }

    #[cfg(test)]
    pub(crate) fn warm_tile(&self, key: TileKey) {
        let _ = self.ids_for(key, (TILE * TILE) as usize);
    }
}

fn render(world: Option<&World>, key: TileKey, relief: bool) -> Vec<u8> {
    let Some(w) = world else { return checker::render(key) };
    let n = (TILE * TILE) as usize;
    let ids = w.ids_for(key, n);
    let shade = if relief && let Some(t) = &w.terrain {
        Some(w.shade_for(t, key, n))
    } else {
        None
    };
    w.paint_rgba(&ids, shade.as_deref())
}

struct Job {
    px: Pixels,
    priority: f64,
    world: Option<Arc<World>>,
    relief: bool,
}

#[derive(Default)]
struct Queue {
    jobs: HashMap<TileKey, Job>,
    shutdown: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

impl Shared {
    fn worker(self: Arc<Self>) {
        loop {
            let (key, job) = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if q.shutdown {
                        return;
                    }
                    let best = q.jobs.iter().min_by(|a, b| a.1.priority.total_cmp(&b.1.priority)).map(|(k, _)| *k);
                    if let Some(k) = best {
                        let job = q.jobs.remove(&k).unwrap();
                        break (k, job);
                    }
                    q = self.ready.wait(q).unwrap();
                }
            };
            let px = render(job.world.as_deref(), key, job.relief);
            job.px.set(TILE, TILE, px);
        }
    }
}

struct Entry {
    px: Pixels,
    used: u64,
}

pub struct TileStore {
    tiles: RefCell<HashMap<TileKey, Entry>>,
    frame: Cell<u64>,
    /// Render on the calling thread (headless snapshots and tests), not the workers.
    pub sync: Cell<bool>,
    world: RefCell<Option<Arc<World>>>,
    /// Shade tiles by the terrain.
    relief: Cell<bool>,
    shared: Arc<Shared>,
    workers: Cell<bool>,
}

impl TileStore {
    pub fn new(world: Option<Arc<World>>) -> TileStore {
        TileStore {
            tiles: RefCell::new(HashMap::new()),
            frame: Cell::new(0),
            sync: Cell::new(false),
            world: RefCell::new(world),
            relief: Cell::new(false),
            shared: Arc::new(Shared { queue: Mutex::new(Queue::default()), ready: Condvar::new() }),
            workers: Cell::new(false),
        }
    }

    pub fn world(&self) -> Option<Arc<World>> {
        self.world.borrow().clone()
    }

    /// Switch worlds: every cached and queued tile belongs to the old one.
    pub fn set_world(&self, world: Option<Arc<World>>) {
        *self.world.borrow_mut() = world;
        self.tiles.borrow_mut().clear();
        self.shared.queue.lock().unwrap().jobs.clear();
    }

    pub fn relief(&self) -> bool {
        self.relief.get()
    }

    /// Turn the hillshade on or off: repaint any tile whose layers are already in memory, and
    /// re-render only tiles whose hillshade has not been computed yet.
    pub fn set_relief(&self, on: bool) {
        if self.relief.replace(on) != on {
            self.shared.queue.lock().unwrap().jobs.clear();
            let world = self.world();
            let mut tiles = self.tiles.borrow_mut();
            let Some(w) = world.as_deref() else {
                tiles.clear();
                return;
            };
            let mem = w.mem.lock().unwrap();
            tiles.retain(|key, entry| {
                let Some(mt) = mem.get(key) else { return false };
                let Some(ids) = &mt.ids else { return false };
                if on && w.terrain.is_some() {
                    let Some(shade) = &mt.shade else { return false };
                    entry.px.set(TILE, TILE, w.paint_rgba(ids, Some(shade)));
                    true
                } else {
                    entry.px.set(TILE, TILE, w.paint_rgba(ids, None));
                    true
                }
            });
        }
    }

    fn start_workers(&self) {
        if self.workers.replace(true) {
            return;
        }
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).max(1);
        for i in 0..n {
            let s = self.shared.clone();
            std::thread::Builder::new()
                .name(format!("mcmap-tiles-{i}"))
                .spawn(move || s.worker())
                .expect("spawn tile worker");
        }
    }

    pub fn begin_frame(&self) {
        self.frame.set(self.frame.get() + 1);
    }

    /// In synchronous mode (snapshots/tests), render all missing visible tiles in parallel first.
    pub fn prefetch_sync(&self, keys: &[TileKey]) {
        if !self.sync.get() {
            return;
        }
        let world = self.world();
        let relief = self.relief.get();
        let missing: Vec<TileKey> = {
            let tiles = self.tiles.borrow();
            keys.iter().copied().filter(|k| !tiles.contains_key(k)).collect()
        };
        if missing.len() <= 1 {
            return;
        }
        use rayon::prelude::*;
        let rendered: Vec<(TileKey, Vec<u8>)> = missing
            .into_par_iter()
            .map(|k| (k, render(world.as_deref(), k, relief)))
            .collect();
        let now = self.frame.get();
        let mut tiles = self.tiles.borrow_mut();
        for (k, data) in rendered {
            let px = Pixels::new();
            px.set(TILE, TILE, data);
            tiles.insert(k, Entry { px, used: now });
        }
    }

    /// The tile's pixels; queues the render the first time it's asked for, and refreshes its
    /// priority while it waits.
    pub fn request(&self, key: TileKey, priority: f64) -> Pixels {
        let now = self.frame.get();
        let mut tiles = self.tiles.borrow_mut();
        if let Some(e) = tiles.get_mut(&key) {
            e.used = now;
            if let Some(j) = self.shared.queue.lock().unwrap().jobs.get_mut(&key) {
                j.priority = priority;
            }
            return e.px.clone();
        }
        let px = Pixels::new();
        let world = self.world();
        let relief = self.relief.get();
        if self.sync.get() {
            px.set(TILE, TILE, render(world.as_deref(), key, relief));
        } else {
            if let Some(w) = world.as_deref()
                && let Some(standin) = w.standin_rgba(key, relief)
            {
                px.set(TILE, TILE, standin);
            }
            self.start_workers();
            self.shared.queue.lock().unwrap().jobs.insert(key, Job { px: px.clone(), priority, world, relief });
            self.shared.ready.notify_one();
        }
        tiles.insert(key, Entry { px: px.clone(), used: now });
        px
    }

    /// After a view run: drop queued tiles it didn't ask for (they'd be rendered for nobody),
    /// and trim the cache to its bound, oldest first.
    pub fn end_frame(&self) {
        let now = self.frame.get();
        let mut tiles = self.tiles.borrow_mut();
        {
            let mut q = self.shared.queue.lock().unwrap();
            let stale: Vec<TileKey> = q.jobs.keys().filter(|k| tiles.get(k).is_none_or(|e| e.used < now)).copied().collect();
            for k in stale {
                q.jobs.remove(&k);
                tiles.remove(&k);
            }
        }
        if tiles.len() > CAPACITY {
            let mut by_age: Vec<(u64, TileKey)> = tiles.iter().map(|(k, e)| (e.used, *k)).collect();
            by_age.sort();
            let excess = tiles.len() - CAPACITY;
            for (used, k) in by_age.into_iter().take(excess) {
                if used < now {
                    tiles.remove(&k);
                }
            }
        }
    }

    /// Tiles waiting for a worker (for tests).
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.shared.queue.lock().unwrap().jobs.len()
    }
}

impl Drop for TileStore {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap().shutdown = true;
        self.shared.ready.notify_all();
    }
}
