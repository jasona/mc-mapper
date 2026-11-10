//! Tile pixels and their scheduler. Each tile is one `rune::Pixels`, rendered on a small pool of
//! worker threads and published with `Pixels::set`, which uploads and wakes the window without
//! re-running the view.
//!
//! Each view run asks for the visible tiles with a priority (distance from the view centre, in
//! tiles). Workers take the nearest queued tile first, and a queued tile the next view run no
//! longer asks for is dropped before anyone renders it, so a fast pan or zoom never leaves a
//! backlog of tiles nobody will see. Finished tiles stay in a bounded memory cache.
//!
//! Tiles are surface biomes when a world is loaded, else the M0 checkerboard.

use crate::diskcache::TileCache;
use mcmap_tiles::{biomes, checker};
use mcmap_tiles::TileKey;
use mcmap_worldgen::terrain::Terrain;
use mcmap_worldgen::Surface;
use rune::Pixels;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

/// Most tiles kept: 512 tiles of 256 KiB each is 128 MiB of RGBA at most.
const CAPACITY: usize = 512;

/// A seed's surface biomes and their colours.
pub struct World {
    pub seed: i64,
    pub surface: Surface,
    pub colors: Vec<[u8; 3]>,
    /// Where finished tiles' biome ids are kept on disk, if anywhere.
    pub cache: Option<TileCache>,
    /// The Overworld's terrain, for the hillshade.
    pub terrain: Option<Terrain>,
}

impl World {
    pub fn new(seed: i64, surface: Surface) -> World {
        let colors = biomes::palette(&surface);
        World { seed, surface, colors, cache: None, terrain: None }
    }

    pub fn with_terrain(mut self, terrain: Option<Terrain>) -> World {
        self.terrain = terrain;
        self
    }

    pub fn with_cache(mut self, cache: Option<TileCache>) -> World {
        self.cache = cache;
        self
    }
}

fn render(world: Option<&World>, key: TileKey, relief: bool) -> Vec<u8> {
    let Some(w) = world else { return checker::render(key) };
    let n = (mcmap_tiles::TILE * mcmap_tiles::TILE) as usize;
    let ids = match w.cache.as_ref().and_then(|c| c.get(key, n)) {
        Some(ids) => ids,
        None => {
            let ids = biomes::ids(&w.surface, key);
            if let Some(c) = &w.cache {
                c.put(key, &ids);
            }
            ids
        }
    };
    let mut px = biomes::paint(&ids, &w.colors);
    if relief && let Some(t) = &w.terrain {
        let shade = match w.cache.as_ref().and_then(|c| c.get_shade(key, n)) {
            Some(s) => s,
            None => {
                let s = mcmap_tiles::relief::shade(t, key);
                if let Some(c) = &w.cache {
                    c.put_shade(key, &s);
                }
                s
            }
        };
        mcmap_tiles::relief::apply(&mut px, &shade);
    }
    px
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
            job.px.set(mcmap_tiles::TILE, mcmap_tiles::TILE, px);
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

    /// Turn the hillshade on or off: every tile is drawn again.
    pub fn set_relief(&self, on: bool) {
        if self.relief.replace(on) != on {
            self.tiles.borrow_mut().clear();
            self.shared.queue.lock().unwrap().jobs.clear();
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
        if self.sync.get() {
            px.set(mcmap_tiles::TILE, mcmap_tiles::TILE, render(world.as_deref(), key, self.relief.get()));
        } else {
            self.start_workers();
            self.shared.queue.lock().unwrap().jobs.insert(key, Job { px: px.clone(), priority, world, relief: self.relief.get() });
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
