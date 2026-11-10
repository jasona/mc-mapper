//! MC Mapper's window.
//!
//! cargo run -p mcmap-app --release                         open the map
//! cargo run -p mcmap-app -- --snap out.png --scene home    render the reference view headless
//!
//! Art is read from `$MCMAP_ART`, else `art/seed-map` under the current directory; it supplies
//! the feature picker's icons and the map markers. `MCMAP_SPRITE_DEMO=1` also draws the marker
//! atlas as a test grid.
//!
//! The starting seed is `$MCMAP_SEED` (default: the reference capture's seed); the seed field
//! changes it. The version pack is `$MCMAP_PACK`, else the data directory's 26.3 pack; without
//! one the map shows the M0 checkerboard.
//!
//! `MCMAP_REPLAY=path.ndjson` plays a recorded path as the player position (see
//! `mcmap_live::replay`), which turns on the player marker and the Follow button.

mod features;
mod map;
mod markers;
mod sprites;
mod tiles;

use map::Map;
use mcmap_data::link::seed_from_text;
use mcmap_data::pack::{pack_path, RawPack};
use mcmap_live::replay::ReplaySource;
use rune::{AppSpec, Tx, Waker};
use std::path::PathBuf;
use std::sync::Arc;
use tiles::World;

/// The reference capture's seed.
const DEFAULT_SEED: i64 = -7556262131344217808;

fn art_dir() -> PathBuf {
    std::env::var_os("MCMAP_ART").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("art/seed-map"))
}

fn pack() -> Option<Arc<RawPack>> {
    let path = std::env::var_os("MCMAP_PACK").map(PathBuf::from).unwrap_or_else(|| pack_path("26.3"));
    match RawPack::read(&path) {
        Ok(p) => Some(Arc::new(p)),
        Err(e) => {
            eprintln!("mcmap: no version pack ({e}); showing the checkerboard. Make it with `cargo run -q -p xtask -- extract --version 26.3`.");
            None
        }
    }
}

fn init() -> Map {
    let sprites = sprites::Sprites::load(&art_dir()).map_err(|e| eprintln!("mcmap: no marker art ({e})")).ok();
    let seed = std::env::var("MCMAP_SEED").ok().and_then(|s| seed_from_text(&s)).unwrap_or(DEFAULT_SEED);
    let pack = pack();
    let world = pack.as_ref().and_then(|p| match mcmap_worldgen::Surface::new(p, seed) {
        Ok(s) => Some(Arc::new(World::new(seed, s))),
        Err(e) => {
            eprintln!("mcmap: {e}");
            None
        }
    });
    let mut map = Map::new(sprites, world);
    map.sprite_demo = std::env::var_os("MCMAP_SPRITE_DEMO").is_some();
    if let Some(p) = &pack {
        match markers::Engine::new(p, seed) {
            Ok(e) => map.markers.set_engine(Some(Arc::new(e))),
            Err(e) => eprintln!("mcmap: structures: {e}"),
        }
    }
    map.pack = pack;
    map.features = features::Features::load(&art_dir()).map_err(|e| eprintln!("mcmap: no feature picker ({e})")).ok();
    map
}

/// Start the position source, if one is configured. Fixes go through a forwarding thread that
/// wakes the window for each batch.
fn setup(s: &mut Map, wake: Waker) {
    s.markers.set_waker(wake.clone());
    let Some(path) = std::env::var_os("MCMAP_REPLAY") else { return };
    let source = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| ReplaySource::parse(&t)) {
        Ok(mut r) => {
            r.looped = true;
            r
        }
        Err(e) => {
            eprintln!("mcmap: replay {}: {e}", PathBuf::from(&path).display());
            return;
        }
    };
    let (rx, stopper) = mcmap_live::spawn(Box::new(source));
    let (tx, out) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _keep = stopper;
        while let Ok(f) = rx.recv() {
            if tx.send(f).is_err() {
                return;
            }
            wake();
        }
    });
    s.fixes = Some(out);
    s.follow = true;
}

fn poll(s: &mut Map, _t: f64, tx: &mut Tx) -> bool {
    let fixes = s.take_fixes(tx);
    let markers = s.take_marker_updates();
    fixes || markers
}

/// Headless scenes render tiles synchronously so a snapshot never shows a tile still in flight.
fn sync(s: &mut Map) {
    s.tiles.sync.set(true);
    s.markers.sync = true;
}

fn main() {
    rune::run(AppSpec::<Map> {
        scenes: &[
            ("home", |s| sync(s)),
            ("zoomed-out", |s| {
                sync(s);
                s.camera.zoom = 0.05;
            }),
            ("zoomed-in", |s| {
                sync(s);
                s.camera.zoom = 6.0;
                s.camera.center = [2839.0 - 160.0, -1418.0 - 64.0];
            }),
            ("far", |s| {
                sync(s);
                s.camera.center = [29_999_000.5, -29_999_000.5];
                s.camera.zoom = 4.0;
            }),
            // The reference view with the layers features.png shows (trial chambers, ruined
            // portals, ancient cities, slime chunks) plus every other structure and the spawn.
            ("structures", |s| {
                sync(s);
                if let Some(p) = s.pack.clone() {
                    let seed = s.tiles.world().map(|w| w.seed).unwrap_or(DEFAULT_SEED);
                    s.markers.set_engine(markers::Engine::new(&p, seed).ok().map(Arc::new));
                }
                if let Some(f) = &mut s.features {
                    f.on.clear();
                    for l in ["Biomes", "Slime Chunk", "Spawn Point", "Stronghold"].into_iter().chain(markers::KINDS.iter().map(|k| k.label)) {
                        f.on.insert(l.to_owned());
                    }
                }
            }),
            // The reference capture's view and size (1133 x 566 below the toolbar) with no
            // overlays, for the M2 gate: compare with art/seed-map/reference/biomes.png.
            ("manifest", |s| {
                sync(s);
                s.features = None;
            }),
        ],
        setup: Some(setup),
        poll: Some(poll),
        ..AppSpec::new("MC Mapper", (1133.0, 566.0 + map::BAR), init, map::view)
    });
}

#[cfg(test)]
mod tests;
