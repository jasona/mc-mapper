//! Tile pixels: one `rune::Pixels` per tile, rendered on the rayon pool and published with
//! `Pixels::set`, which uploads and wakes the window without re-running the view. Tiles are
//! surface biomes when a world is loaded, else the M0 checkerboard.

use mcmap_tiles::{biomes, checker};
use mcmap_tiles::TileKey;
use mcmap_worldgen::Surface;
use rune::Pixels;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

/// A seed's surface biomes and their colours.
pub struct World {
    pub seed: i64,
    pub surface: Surface,
    pub colors: Vec<[u8; 3]>,
}

impl World {
    pub fn new(seed: i64, surface: Surface) -> World {
        let colors = biomes::palette(&surface);
        World { seed, surface, colors }
    }
}

fn render(world: Option<&World>, key: TileKey) -> Vec<u8> {
    match world {
        Some(w) => biomes::render(&w.surface, &w.colors, key),
        None => checker::render(key),
    }
}

/// Most tiles kept: 512 tiles of 256 KiB each is 128 MiB of RGBA at most.
const CAPACITY: usize = 512;

pub struct TileStore {
    tiles: RefCell<HashMap<TileKey, (Pixels, u64)>>,
    frame: Cell<u64>,
    /// Render on the calling thread (headless snapshots and tests), not the pool.
    pub sync: Cell<bool>,
    pub world: Option<Arc<World>>,
}

impl TileStore {
    pub fn new(world: Option<Arc<World>>) -> TileStore {
        TileStore { tiles: RefCell::new(HashMap::new()), frame: Cell::new(0), sync: Cell::new(false), world }
    }

    pub fn begin_frame(&self) {
        self.frame.set(self.frame.get() + 1);
    }

    /// The tile's pixels; schedules the render the first time it's asked for.
    pub fn request(&self, key: TileKey) -> Pixels {
        let now = self.frame.get();
        let mut tiles = self.tiles.borrow_mut();
        if let Some((px, used)) = tiles.get_mut(&key) {
            *used = now;
            return px.clone();
        }
        let px = Pixels::new();
        let out = px.clone();
        if self.sync.get() {
            out.set(mcmap_tiles::TILE, mcmap_tiles::TILE, render(self.world.as_deref(), key));
        } else {
            let world = self.world.clone();
            rayon::spawn(move || out.set(mcmap_tiles::TILE, mcmap_tiles::TILE, render(world.as_deref(), key)));
        }
        tiles.insert(key, (px.clone(), now));
        if tiles.len() > CAPACITY {
            let mut by_age: Vec<(u64, TileKey)> = tiles.iter().map(|(k, (_, u))| (*u, *k)).collect();
            by_age.sort();
            for (used, k) in by_age.into_iter().take(tiles.len() - CAPACITY) {
                if used < now {
                    tiles.remove(&k);
                }
            }
        }
        px
    }
}
