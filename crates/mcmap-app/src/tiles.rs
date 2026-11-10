//! Tile pixels: one `rune::Pixels` per tile, rendered on the rayon pool and published with
//! `Pixels::set`, which uploads and wakes the window without re-running the view.

use mcmap_tiles::checker;
use mcmap_tiles::TileKey;
use rune::Pixels;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Most tiles kept: 512 tiles of 256 KiB each is 128 MiB of RGBA at most.
const CAPACITY: usize = 512;

pub struct TileStore {
    tiles: RefCell<HashMap<TileKey, (Pixels, u64)>>,
    frame: Cell<u64>,
    /// Render on the calling thread (headless snapshots and tests), not the pool.
    pub sync: Cell<bool>,
}

impl TileStore {
    pub fn new() -> TileStore {
        TileStore { tiles: RefCell::new(HashMap::new()), frame: Cell::new(0), sync: Cell::new(false) }
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
            out.set(mcmap_tiles::TILE, mcmap_tiles::TILE, checker::render(key));
        } else {
            rayon::spawn(move || out.set(mcmap_tiles::TILE, mcmap_tiles::TILE, checker::render(key)));
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
