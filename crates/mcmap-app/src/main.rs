//! MC Mapper's window.
//!
//! cargo run -p mcmap-app --release                         open the map
//! cargo run -p mcmap-app -- --snap out.png --scene home    render the reference view headless
//!
//! Art is read from `$MCMAP_ART`, else `art/seed-map` under the current directory. Without it the
//! map still runs, with no markers.
//!
//! The seed is `$MCMAP_SEED` (default: the reference capture's seed) and the version pack is
//! `$MCMAP_PACK`, else the data directory's 26.3 pack. Without a pack the map shows the M0
//! checkerboard.

mod map;
mod sprites;
mod tiles;

use map::Map;
use mcmap_data::pack::{pack_path, RawPack};
use rune::AppSpec;
use std::path::PathBuf;
use std::sync::Arc;
use tiles::World;

/// The reference capture's seed.
const DEFAULT_SEED: i64 = -7556262131344217808;

fn world() -> Option<Arc<World>> {
    let seed = match std::env::var("MCMAP_SEED") {
        Ok(s) => match s.trim().parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("mcmap: MCMAP_SEED {s:?} is not a number; using {DEFAULT_SEED}");
                DEFAULT_SEED
            }
        },
        Err(_) => DEFAULT_SEED,
    };
    let path = std::env::var_os("MCMAP_PACK").map(PathBuf::from).unwrap_or_else(|| pack_path("26.3"));
    let made = RawPack::read(&path).and_then(|p| mcmap_worldgen::Surface::new(&p, seed));
    match made {
        Ok(s) => Some(Arc::new(World::new(seed, s))),
        Err(e) => {
            eprintln!("mcmap: no biomes ({e}); showing the checkerboard. Make the pack with `cargo run -q -p xtask -- extract --version 26.3`.");
            None
        }
    }
}

fn art_dir() -> PathBuf {
    std::env::var_os("MCMAP_ART").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("art/seed-map"))
}

fn init() -> Map {
    let sprites = match sprites::Sprites::load(&art_dir()) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("mcmap: no marker art ({e}); running without markers");
            None
        }
    };
    Map::new(sprites, world())
}

/// Headless scenes render tiles synchronously so a snapshot never shows a tile still in flight.
fn sync(s: &mut Map) {
    s.tiles.sync.set(true);
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
        ],
        ..AppSpec::new("MC Mapper", (1133.0, 594.0), init, map::view)
    });
}

#[cfg(test)]
mod tests;
