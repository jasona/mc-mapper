//! Structure, spawn and slime-chunk markers.
//!
//! Structures are computed per region of 64 x 64 chunks on two worker threads, all structure sets
//! at once, so turning a layer on or off never waits for work. The view asks for the regions it
//! lays out, nearest first; a queued region the next view run doesn't ask for is dropped, as with
//! tiles. Spawn and stronghold rings are computed once per seed on their own thread (the rings
//! take about 2.5 s: 128 biome searches). Finished work sets `dirty` and wakes the window, and
//! `poll` re-runs the view.
//!
//! Which picker label shows which structures, and with which sprite of the marker atlas, is the
//! table in [`KINDS`].

use mcmap_data::pack::RawPack;
use mcmap_data::Dimension;
use mcmap_worldgen::structure::{Found, Placement, Structures};
use mcmap_worldgen::world::{SpawnFinder, World};
use rune::Waker;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

/// Chunks per region side.
pub const REGION: i32 = 64;

/// Regions kept in memory (about 30 structures each).
const CAPACITY: usize = 8192;

/// A picker label and the structures it shows.
pub struct Kind {
    pub label: &'static str,
    /// Structure ids (without `minecraft:`) this label covers; a trailing `*` matches a prefix.
    pub ids: &'static [&'static str],
    /// Marker atlas sprite, by segment index (see `sprites.rs`).
    pub sprite: usize,
}

/// Sprites were matched to the picker icons by image similarity and checked by eye; trial
/// chambers, ruined portals and the ancient city are confirmed against the reference capture.
pub const KINDS: &[Kind] = &[
    Kind { label: "Village", ids: &["village_*"], sprite: 50 },
    Kind { label: "Ancient City", ids: &["ancient_city"], sprite: 2 },
    Kind { label: "Stronghold", ids: &["stronghold"], sprite: 64 },
    Kind { label: "Woodland Mansion", ids: &["mansion"], sprite: 61 },
    Kind { label: "Ocean Monument", ids: &["monument"], sprite: 66 },
    Kind { label: "Pillager Outpost", ids: &["pillager_outpost"], sprite: 35 },
    Kind { label: "Mineshaft", ids: &["mineshaft*"], sprite: 31 },
    Kind { label: "Ruined Portal Overworld", ids: &["ruined_portal", "ruined_portal_desert", "ruined_portal_jungle", "ruined_portal_swamp", "ruined_portal_mountain", "ruined_portal_ocean"], sprite: 41 },
    Kind { label: "Jungle Temple", ids: &["jungle_pyramid"], sprite: 28 },
    Kind { label: "Desert Temple", ids: &["desert_pyramid"], sprite: 21 },
    Kind { label: "Witch Hut", ids: &["swamp_hut"], sprite: 51 },
    Kind { label: "Buried Treasure", ids: &["buried_treasure"], sprite: 56 },
    Kind { label: "Shipwreck", ids: &["shipwreck*"], sprite: 63 },
    Kind { label: "Igloo", ids: &["igloo"], sprite: 27 },
    Kind { label: "Ocean Ruins", ids: &["ocean_ruin_*"], sprite: 34 },
    Kind { label: "Trail Ruins", ids: &["trail_ruins"], sprite: 44 },
    Kind { label: "Trial Chamber", ids: &["trial_chambers"], sprite: 65 },
    Kind { label: "Abandoned Camp", ids: &["abandoned_camp_*"], sprite: 1 },
    Kind { label: "Nether Fortress", ids: &["fortress"], sprite: 32 },
    Kind { label: "Bastion Remnant", ids: &["bastion_remnant"], sprite: 67 },
    Kind { label: "Ruined Portal Nether", ids: &["ruined_portal_nether"], sprite: 41 },
    Kind { label: "End City", ids: &["end_city"], sprite: 23 },
];

/// Picker entries the art manifest doesn't have (its picker is the Overworld's), drawn with
/// marker sprites: (dimension, label, sprite). The bastion sprite is a guess from the art.
pub const EXTRA_PICKER: &[(Dimension, &str, usize)] = &[
    (Dimension::Nether, "Nether Fortress", 32),
    (Dimension::Nether, "Bastion Remnant", 67),
    (Dimension::Nether, "Ruined Portal Nether", 41),
    (Dimension::End, "End City", 23),
];

pub const SPAWN_SPRITE: usize = 43;

/// Village variants have their own heads in the atlas. Which head is which variant is a guess
/// from the art (desert turban, snowy hood and so on), not checked against the reference.
pub fn sprite_for(kind: &Kind, structure: &str) -> usize {
    match structure.trim_start_matches("minecraft:") {
        "village_desert" => 45,
        "village_savanna" => 46,
        "village_snowy" => 47,
        "village_taiga" => 48,
        _ => kind.sprite,
    }
}

impl Kind {
    pub fn covers(&self, structure: &str) -> bool {
        let s = structure.trim_start_matches("minecraft:");
        self.ids.iter().any(|p| match p.strip_suffix('*') {
            Some(prefix) => s.starts_with(prefix),
            None => s == *p,
        })
    }
}

pub fn kind_of(structure: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.covers(structure))
}

pub fn kind(label: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.label == label)
}

/// A readable name: "Village (plains)", "Ruined Portal (jungle)".
pub fn display_name(structure: &str) -> String {
    let s = structure.trim_start_matches("minecraft:");
    let base = kind_of(structure).map(|k| k.label.trim_end_matches(" Overworld")).unwrap_or(s);
    let variant = s
        .strip_prefix("village_")
        .or_else(|| s.strip_prefix("ruined_portal_"))
        .or_else(|| s.strip_prefix("shipwreck_"))
        .or_else(|| s.strip_prefix("ocean_ruin_"))
        .or_else(|| s.strip_prefix("mineshaft_"))
        .or_else(|| s.strip_prefix("abandoned_camp_"));
    match variant {
        Some(v) => format!("{base} ({})", v.replace('_', " ")),
        None => base.to_string(),
    }
}

/// One seed's structure engine.
pub struct Engine {
    pub world: World,
    pub structures: Structures,
    /// The Overworld's spawn search; none in the Nether and the End.
    pub spawn: Option<SpawnFinder>,
}

impl Engine {
    pub fn new(pack: &RawPack, seed: i64, dimension: Dimension) -> Result<Engine, String> {
        Engine::with_preset(pack, seed, dimension, mcmap_data::Preset::Normal)
    }

    pub fn with_preset(pack: &RawPack, seed: i64, dimension: Dimension, preset: mcmap_data::Preset) -> Result<Engine, String> {
        let world = World::for_world(pack, seed, dimension, preset)?;
        let structures = Structures::new(&pack.files, world.surface.names())?;
        let spawn = if dimension == Dimension::Overworld { Some(SpawnFinder::new(&pack.files, seed, preset)?) } else { None };
        Ok(Engine { world, structures, spawn })
    }

    /// Every random-spread structure in the region.
    pub fn region(&self, r: (i32, i32)) -> Vec<Found> {
        let (cx0, cz0) = (r.0 * REGION, r.1 * REGION);
        let mut out = Vec::new();
        for set in &self.structures.sets {
            out.extend(self.structures.in_area(&self.world, set, cx0, cz0, cx0 + REGION - 1, cz0 + REGION - 1));
        }
        out
    }

    /// Generated strongholds of the ring sets.
    pub fn strongholds(&self) -> Vec<Found> {
        let mut out = Vec::new();
        for set in &self.structures.sets {
            if matches!(set.placement, Placement::Rings { .. }) {
                for (cx, cz) in self.structures.rings(&self.world, set) {
                    out.extend(self.structures.generate(&self.world, set, cx, cz));
                }
            }
        }
        out
    }

    /// The nearest structure of `kind` to block (x, z), searching regions outward up to
    /// `max_regions` rings. Ring sets are not searched here (see [`Engine::strongholds`]).
    pub fn nearest(&self, kind: &Kind, x: f64, z: f64, max_regions: i32) -> Option<Found> {
        let sets: Vec<_> = self.structures.sets.iter().filter(|s| s.entries.iter().any(|e| kind.covers(&e.0.id))).collect();
        let span = (REGION * 16) as f64;
        let (rx, rz) = ((x / span).floor() as i32, (z / span).floor() as i32);
        let mut best: Option<(f64, Found)> = None;
        for ring in 0..=max_regions {
            // Anything in this ring is at least (ring - 1) regions away.
            if let Some((d, _)) = &best
                && *d < ((ring - 1).max(0) as f64 * span).powi(2)
            {
                break;
            }
            for dz in -ring..=ring {
                for dx in -ring..=ring {
                    if dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    let (cx0, cz0) = ((rx + dx) * REGION, (rz + dz) * REGION);
                    for set in &sets {
                        for f in self.structures.in_area(&self.world, set, cx0, cz0, cx0 + REGION - 1, cz0 + REGION - 1) {
                            if !kind.covers(&f.structure) {
                                continue;
                            }
                            let d = (f.marker.0 as f64 - x).powi(2) + (f.marker.1 as f64 - z).powi(2);
                            if best.as_ref().is_none_or(|b| d < b.0) {
                                best = Some((d, f));
                            }
                        }
                    }
                }
            }
        }
        best.map(|b| b.1)
    }
}

enum Region {
    Queued(f64),
    Running,
    Done(Arc<Vec<Found>>, u64),
}

#[derive(Default)]
struct State {
    generation: u64,
    engine: Option<Arc<Engine>>,
    regions: HashMap<(i32, i32), Region>,
    strongholds: Option<Arc<Vec<Found>>>,
    spawn: Option<(i32, i32)>,
    /// A nearest-search result: (label, found or None for nothing within range).
    nearest: Option<(String, Option<Found>)>,
    searching: bool,
    dirty: bool,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
    wake: Mutex<Option<Waker>>,
}

impl Shared {
    fn wake(&self) {
        if let Some(w) = self.wake.lock().unwrap().as_ref() {
            w();
        }
    }

    fn worker(self: Arc<Self>) {
        loop {
            let (key, engine, generation) = {
                let mut st = self.state.lock().unwrap();
                loop {
                    if st.shutdown {
                        return;
                    }
                    let best = st
                        .regions
                        .iter()
                        .filter_map(|(k, r)| match r {
                            Region::Queued(p) => Some((*p, *k)),
                            _ => None,
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0));
                    if let (Some((_, k)), Some(e)) = (best, st.engine.clone()) {
                        st.regions.insert(k, Region::Running);
                        break (k, e, st.generation);
                    }
                    st = self.ready.wait(st).unwrap();
                }
            };
            let found = Arc::new(engine.region(key));
            let mut st = self.state.lock().unwrap();
            if st.generation == generation {
                st.regions.insert(key, Region::Done(found, 0));
                st.dirty = true;
                drop(st);
                self.wake();
            }
        }
    }
}

pub struct MarkerStore {
    shared: Arc<Shared>,
    workers: Cell<bool>,
    frame: Cell<u64>,
    /// Compute on the calling thread (headless snapshots and tests).
    pub sync: bool,
}

impl Default for MarkerStore {
    fn default() -> Self {
        MarkerStore::new()
    }
}

impl MarkerStore {
    pub fn new() -> MarkerStore {
        MarkerStore {
            shared: Arc::new(Shared { state: Mutex::new(State::default()), ready: Condvar::new(), wake: Mutex::new(None) }),
            workers: Cell::new(false),
            frame: Cell::new(0),
            sync: false,
        }
    }

    pub fn set_waker(&self, w: Waker) {
        *self.shared.wake.lock().unwrap() = Some(w);
    }

    /// Switch seeds. Spawn and strongholds start computing at once.
    pub fn set_engine(&mut self, engine: Option<Arc<Engine>>) {
        let generation = {
            let mut st = self.shared.state.lock().unwrap();
            st.generation += 1;
            st.engine = engine.clone();
            st.regions.clear();
            st.strongholds = None;
            st.spawn = None;
            st.nearest = None;
            st.dirty = true;
            st.generation
        };
        let Some(engine) = engine else { return };
        let shared = self.shared.clone();
        let job = move || {
            let Some(finder) = &engine.spawn else { return };
            let spawn = finder.find();
            if let Ok(mut st) = shared.state.lock()
                && st.generation == generation
            {
                st.spawn = Some(spawn);
                st.dirty = true;
            }
            shared.wake();
            let rings = Arc::new(engine.strongholds());
            if let Ok(mut st) = shared.state.lock()
                && st.generation == generation
            {
                st.strongholds = Some(rings);
                st.dirty = true;
            }
            shared.wake();
        };
        if self.sync {
            job();
        } else {
            std::thread::Builder::new().name("mcmap-rings".into()).spawn(job).expect("spawn rings thread");
        }
    }

    fn start_workers(&self) {
        if self.workers.replace(true) {
            return;
        }
        for i in 0..2 {
            let s = self.shared.clone();
            std::thread::Builder::new().name(format!("mcmap-structures-{i}")).spawn(move || s.worker()).expect("spawn structure worker");
        }
    }

    pub fn begin_frame(&self) {
        self.frame.set(self.frame.get() + 1);
    }

    /// The region's structures if done; otherwise queues it (or refreshes its priority).
    pub fn request(&self, key: (i32, i32), priority: f64) -> Option<Arc<Vec<Found>>> {
        let frame = self.frame.get();
        let mut st = self.shared.state.lock().unwrap();
        let engine = st.engine.clone()?;
        match st.regions.get_mut(&key) {
            Some(Region::Done(f, used)) => {
                *used = frame;
                return Some(f.clone());
            }
            Some(Region::Queued(p)) => {
                *p = priority;
                return None;
            }
            Some(Region::Running) => return None,
            None => {}
        }
        if self.sync {
            let f = Arc::new(engine.region(key));
            st.regions.insert(key, Region::Done(f.clone(), frame));
            return Some(f);
        }
        st.regions.insert(key, Region::Queued(priority));
        drop(st);
        self.start_workers();
        self.shared.ready.notify_one();
        None
    }

    /// After a view run: drop queued regions it didn't ask for, and trim the cache.
    pub fn end_frame(&self, asked: &[(i32, i32)]) {
        let frame = self.frame.get();
        let mut st = self.shared.state.lock().unwrap();
        st.regions.retain(|k, r| !matches!(r, Region::Queued(_)) || asked.contains(k));
        if st.regions.len() > CAPACITY {
            let mut by_age: Vec<(u64, (i32, i32))> = st.regions.iter().filter_map(|(k, r)| match r {
                Region::Done(_, u) if *u < frame => Some((*u, *k)),
                _ => None,
            }).collect();
            by_age.sort();
            let excess = st.regions.len() - CAPACITY;
            for (_, k) in by_age.into_iter().take(excess) {
                st.regions.remove(&k);
            }
        }
    }

    pub fn strongholds(&self) -> Option<Arc<Vec<Found>>> {
        self.shared.state.lock().unwrap().strongholds.clone()
    }

    pub fn spawn(&self) -> Option<(i32, i32)> {
        self.shared.state.lock().unwrap().spawn
    }

    /// True once after background work finished: the view should re-run.
    pub fn take_dirty(&self) -> bool {
        std::mem::take(&mut self.shared.state.lock().unwrap().dirty)
    }

    /// Start a nearest search for `label` from block (x, z); the result arrives through
    /// [`MarkerStore::take_nearest`]. Strongholds answer from the rings once they are known.
    pub fn find_nearest(&self, label: &str, x: f64, z: f64) -> Result<(), String> {
        let kind = kind(label).ok_or_else(|| format!("{label}: no structures to search yet"))?;
        let mut st = self.shared.state.lock().unwrap();
        let engine = st.engine.clone().ok_or("No world loaded")?;
        if kind.label == "Stronghold" {
            let Some(rings) = st.strongholds.clone() else { return Err("Strongholds are still being computed (a few seconds)".into()) };
            let best = rings.iter().min_by(|a, b| {
                let d = |f: &Found| (f.marker.0 as f64 - x).powi(2) + (f.marker.1 as f64 - z).powi(2);
                d(a).total_cmp(&d(b))
            });
            st.nearest = Some((label.to_string(), best.cloned()));
            st.dirty = true;
            drop(st);
            self.shared.wake();
            return Ok(());
        }
        if std::mem::replace(&mut st.searching, true) {
            return Err("Still searching".into());
        }
        let generation = st.generation;
        drop(st);
        let shared = self.shared.clone();
        let label = label.to_string();
        let job = move || {
            // 24 regions is about 24,500 blocks each way.
            let found = engine.nearest(kind, x, z, 24);
            let mut st = shared.state.lock().unwrap();
            st.searching = false;
            if st.generation == generation {
                st.nearest = Some((label, found));
                st.dirty = true;
            }
            drop(st);
            shared.wake();
        };
        if self.sync {
            job();
        } else {
            std::thread::Builder::new().name("mcmap-nearest".into()).spawn(job).expect("spawn search thread");
        }
        Ok(())
    }

    pub fn take_nearest(&self) -> Option<(String, Option<Found>)> {
        self.shared.state.lock().unwrap().nearest.take()
    }
}

impl Drop for MarkerStore {
    fn drop(&mut self) {
        if let Ok(mut st) = self.shared.state.lock() {
            st.shutdown = true;
        }
        self.shared.ready.notify_all();
    }
}
