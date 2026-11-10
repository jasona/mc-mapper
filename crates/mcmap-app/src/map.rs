//! The map: an `f64` camera over a tile pyramid, drawn with Rune's compositor-side camera, plus
//! the toolbar (seed, version, dimension, follow), the feature picker, the hover readout and the
//! player marker.
//!
//! The view lays tiles out once per (level, origin, tile range) and binds the world element to a
//! `Live` offset and scale. Pan, zoom, hover and position updates move only live values and say
//! `tx.live_only()`, so the view re-runs only when the set of visible tiles changes.
//!
//! Layout space: the world element is exactly the canvas, and the block `origin` sits at its
//! centre. One tile pixel is `1 / scale` points before the live scale, so a tile shows at
//! `k = zoom * scale * blocks_per_pixel` device pixels per tile pixel, always in (0.5, 1].

use crate::chrome::Texture;
use crate::features::Features;
use crate::markers::{self, Engine, MarkerStore, REGION};
use crate::sprites::Sprites;
use crate::tiles::{TileStore, World};
use mcmap_data::link::{parse_link, seed_from_text};
use mcmap_data::pack::RawPack;
use mcmap_data::{Dimension, Preset};
use crate::live;
use mcmap_data::seeds::{zoom_seed, SeedStore};
use mcmap_live::{Fix, SourceKind};
use mcmap_tiles::pyramid::{blocks_per_pixel, tile_blocks, Camera, TileKey, TILE};
use mcmap_worldgen::structure::Found;
use mcmap_worldgen::BiomeSampler;
use rune::tok::TextStyle;
use rune::{col, el, row, text, text_input, Color, Cx, Drag, El, Live, LiveText, MagnifyEvent, Phase, PointerEvent, TextEdit, Tx, WheelEvent};
use std::cell::{Cell, RefCell};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

/// The reference capture's view (seed -7556262131344217808, Java 26.3): 1.25 px per block.
pub const HOME: Camera = Camera { center: [2839.0, -1418.0], zoom: 1.25 };

/// Height of the toolbar above the canvas, in points.
pub const BAR: f32 = 40.0;

const MARKER: f32 = 14.0;
/// The heading dot: its size and its distance from the marker's centre.
const HEADING: f32 = 6.0;
const HEADING_R: f64 = 11.0;
/// A fix older than this many seconds is stale: the marker dims.
pub const STALE_SECS: f64 = 10.0;

/// Structure markers show from this zoom (px per block) in; further out they would be thousands.
pub const STRUCTURE_ZOOM: f64 = 0.15;
/// Slime chunks show from this zoom in.
pub const SLIME_ZOOM: f64 = 0.75;

/// A clicked marker: what the info panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    pub name: String,
    pub x: i32,
    pub y: Option<i32>,
    pub z: i32,
    pub likely: bool,
}

impl Selected {
    pub fn of(f: &Found) -> Selected {
        // The marker is where the icon is; y is the generation point's estimate.
        Selected { name: markers::display_name(&f.structure), x: f.marker.0, y: Some(f.pos.1), z: f.marker.1, likely: f.likely }
    }

    pub fn tp(&self) -> String {
        match self.y {
            Some(y) => format!("/tp @s {} {} {}", self.x, y, self.z),
            None => format!("/tp @s {} ~ {}", self.x, self.z),
        }
    }
}

/// What one view run laid out. A camera move that keeps it can skip the view.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub level: i32,
    pub origin: [f64; 2],
    pub tiles: Vec<TileKey>,
    pub size: [f64; 2],
    pub scale: f64,
}

pub struct Map {
    pub camera: Camera,
    pan: Live<[f32; 2]>,
    k: Live<f32>,
    inv_k: Live<f32>,
    readout: LiveText,
    hover: LiveText,
    player_at: Live<[f32; 2]>,
    /// What the last view run laid out; handlers compare against it.
    shown: RefCell<Option<Layout>>,
    pub tiles: TileStore,
    pub sprites: Option<Sprites>,
    pub features: Option<Features>,
    pan_at_press: Cell<[f64; 2]>,
    /// The version pack worlds are built from.
    pub pack: Option<Arc<RawPack>>,
    pub seed_field: TextEdit,
    /// One line under the toolbar's control, for errors and hints ("not a seed", "M4").
    pub status: String,
    /// Last position from a position source, and whether the camera follows it.
    pub player: Option<Fix>,
    pub follow: bool,
    pub fixes: Option<Receiver<Fix>>,
    /// Notes and session changes from the live sources (`live::start`).
    pub live: Option<live::Live>,
    /// The running game and its launcher world, from the session scan.
    pub session: Option<live::Session>,
    /// Seeds the map has learned, by zoom seed and quick-play id.
    pub seeds: SeedStore,
    /// The zoom seed of a world the map doesn't know yet: the next matching seed typed is kept.
    pub pending_zoom: Option<i64>,
    heading_at: Live<[f32; 2]>,
    age: LiveText,
    /// The marker is dimmed (its fix is older than `STALE_SECS`).
    pub stale: bool,
    /// The pointer is over the canvas: show the hover readout.
    pub hovering: bool,
    /// Draw the whole marker atlas as a grid (M0 check, `MCMAP_SPRITE_DEMO`).
    pub sprite_demo: bool,
    pub markers: MarkerStore,
    pub selected: Option<Selected>,
    pub dimension: Dimension,
    /// World preset of the Overworld.
    pub preset: Preset,
    /// Toolbar and panel textures.
    pub chrome: Option<crate::chrome::Chrome>,
    /// What the tiles and markers were built for, so a repeated load is free.
    loaded: Option<(i64, Dimension, Preset)>,
}

impl Map {
    pub fn new(sprites: Option<Sprites>, world: Option<Arc<World>>) -> Map {
        let seed = world.as_ref().map(|w| w.seed.to_string()).unwrap_or_default();
        Map {
            camera: HOME,
            pan: Live::new([0.0, 0.0]),
            k: Live::new(1.0),
            inv_k: Live::new(1.0),
            readout: LiveText::new(""),
            hover: LiveText::new(""),
            player_at: Live::new([0.0, 0.0]),
            shown: RefCell::new(None),
            tiles: TileStore::new(world),
            sprites,
            features: None,
            pan_at_press: Cell::new(HOME.center),
            pack: None,
            seed_field: TextEdit::new(&seed),
            status: String::new(),
            player: None,
            follow: false,
            fixes: None,
            live: None,
            session: None,
            seeds: SeedStore::default(),
            pending_zoom: None,
            heading_at: Live::new([0.0, 0.0]),
            age: LiveText::new(""),
            stale: false,
            hovering: false,
            sprite_demo: true,
            markers: MarkerStore::new(),
            selected: None,
            dimension: Dimension::Overworld,
            preset: Preset::Normal,
            loaded: None,
            chrome: None,
        }
    }

    fn layout_for(&self, size: [f64; 2], scale: f64) -> Layout {
        let level = self.camera.level(scale);
        let mut tiles = self.camera.visible_tiles(size, scale, 1);
        let origin = TileKey::containing(level, self.camera.center).origin();
        tiles.sort();
        Layout { level, origin, tiles, size, scale }
    }

    /// Layout point of a block under `l`.
    fn point(l: &Layout, block: [f64; 2]) -> [f64; 2] {
        let d = blocks_per_pixel(l.level) * l.scale;
        [l.size[0] / 2.0 + (block[0] - l.origin[0]) / d, l.size[1] / 2.0 + (block[1] - l.origin[1]) / d]
    }

    /// Set the live camera values for layout `l`.
    fn publish(&self, l: &Layout) {
        let k = self.camera.tile_scale(l.level, l.scale);
        let q = Self::point(l, self.camera.center);
        let c = [l.size[0] / 2.0, l.size[1] / 2.0];
        self.pan.set([(-(q[0] - c[0]) * k) as f32, (-(q[1] - c[1]) * k) as f32]);
        self.k.set(k as f32);
        self.inv_k.set((1.0 / k) as f32);
        let [x, z] = self.camera.center;
        self.readout.set(format!("x {x:.0}  z {z:.0}   {:.3} px/block", self.camera.zoom));
        if let Some(p) = &self.player {
            let s = self.camera.point_of([p.pos[0], p.pos[2]], l.size);
            self.player_at.set([(s[0] - MARKER as f64 / 2.0) as f32, (s[1] - MARKER as f64 / 2.0) as f32]);
            // Yaw 0 faces +z (down the screen), 90 faces -x.
            let a = (p.yaw as f64).to_radians();
            let h = [s[0] - a.sin() * HEADING_R, s[1] + a.cos() * HEADING_R];
            self.heading_at.set([(h[0] - HEADING as f64 / 2.0) as f32, (h[1] - HEADING as f64 / 2.0) as f32]);
        }
    }

    /// After a camera change in a handler: if the visible tiles are the same, move only the live
    /// values and skip the view.
    fn camera_moved(&mut self, tx: &mut Tx) {
        let shown = self.shown.borrow().clone();
        if let Some(old) = shown {
            let new = self.layout_for(old.size, old.scale);
            if new.level == old.level && new.tiles == old.tiles && new.origin == old.origin {
                self.publish(&old);
                tx.live_only();
            }
            // Otherwise the view re-runs, lays out the new tiles and publishes.
        }
    }

    pub fn pan_by(&mut self, dx_points: f64, dy_points: f64, tx: &mut Tx) {
        let z = self.camera.zoom;
        self.follow = false;
        self.camera.center[0] -= dx_points / z;
        self.camera.center[1] -= dy_points / z;
        self.camera_moved(tx);
    }

    pub fn zoom_about(&mut self, point: [f64; 2], factor: f64, tx: &mut Tx) {
        let size = self.shown.borrow().as_ref().map(|l| l.size).unwrap_or([0.0, 0.0]);
        // While following, zoom about the player so it stays put.
        let point = match (&self.player, self.follow) {
            (Some(p), true) => self.camera.point_of([p.pos[0], p.pos[2]], size),
            _ => point,
        };
        self.camera.zoom_about(point, size, factor);
        self.camera_moved(tx);
    }

    fn drag(&mut self, d: &Drag, tx: &mut Tx) {
        if d.phase == Phase::Down {
            self.pan_at_press.set(self.camera.center);
        }
        self.follow = false;
        let start = self.pan_at_press.get();
        let z = self.camera.zoom;
        self.camera.center = if d.cancelled() { start } else { [start[0] - d.dx as f64 / z, start[1] - d.dy as f64 / z] };
        self.camera_moved(tx);
    }

    /// The hover readout for a canvas point: block coordinates and the biome there.
    pub fn hover_at(&self, point: [f64; 2]) -> String {
        let size = self.shown.borrow().as_ref().map(|l| l.size).unwrap_or([0.0, 0.0]);
        let [x, z] = self.camera.block_at(point, size);
        let (bx, bz) = (x.floor() as i64, z.floor() as i64);
        let biome = self.tiles.world().map(|w| {
            let b = w.surface.biome((bx >> 2) as i32, 16, (bz >> 2) as i32);
            w.surface.names()[b.0 as usize].trim_start_matches("minecraft:").replace('_', " ")
        });
        match biome {
            Some(b) => format!("{bx}, {bz}   {b}"),
            None => format!("{bx}, {bz}"),
        }
    }

    /// Apply the seed field: a seed, any text, or a pasted seed-map link.
    pub fn submit_seed(&mut self) {
        let text = self.seed_field.text.clone();
        let (seed, link) = match parse_link(&text) {
            Some(l) => (l.seed, Some(l)),
            None => (seed_from_text(&text), None),
        };
        let Some(seed) = seed else {
            self.status = "Type a seed, or paste a seed-map link".into();
            return;
        };
        if let Some(l) = &link {
            if let (Some(x), Some(z)) = (l.x, l.z) {
                self.camera.center = [x, z];
                self.follow = false;
            }
            if l.version.as_deref().is_some_and(|v| v != "26.3") {
                self.status = format!("The link is for {}; showing 26.3, the only version pack so far", l.version.as_deref().unwrap_or("?"));
            }
        }
        let dimension = link.as_ref().and_then(|l| l.dimension).unwrap_or(self.dimension);
        self.load_world(seed, dimension);
        self.seed_field = TextEdit::new(&seed.to_string());
        self.learn_seed(seed);
    }

    /// A seed was typed: keep it for the world the player is in, when that is safe to say. With
    /// the agent's zoom seed the seed must hash to it; without one (F3+C only) it is paired with
    /// the launcher's last-played world while the game runs.
    fn learn_seed(&mut self, seed: i64) {
        let quick_play = self.session.as_ref().and_then(|s| s.quick_play.as_ref());
        let zoom = self.player.as_ref().and_then(|p| p.world.zoom_seed).or(self.pending_zoom);
        let name = self.player.as_ref().and_then(|p| p.world.server.clone()).or_else(|| quick_play.and_then(|q| q.name.clone()));
        let key = quick_play.map(|q| q.key());
        let keep = match zoom {
            Some(z) if z == zoom_seed(seed) => true,
            Some(_) => {
                self.status = "That seed isn't the one of the world you're in; showing it, not remembering it".into();
                false
            }
            None => self.player.is_some() && key.is_some(),
        };
        if !keep {
            return;
        }
        self.pending_zoom = None;
        if self.seeds.remember(seed, name.clone(), key) {
            match self.seeds.save() {
                Ok(()) => self.status = format!("Remembered the seed for {}", name.as_deref().unwrap_or("this world")),
                Err(e) => self.status = format!("Couldn't save the seed: {e}"),
            }
        }
    }

    /// The seed of the world the player is in, as far as the map can tell.
    fn world_seed(&self, f: &Fix) -> Option<i64> {
        if let Some(s) = f.world.seed {
            return Some(s);
        }
        if let Some(z) = f.world.zoom_seed {
            if let Some(w) = self.tiles.world()
                && zoom_seed(w.seed) == z
            {
                return Some(w.seed);
            }
            return self.seeds.by_zoom(z).map(|w| w.seed);
        }
        let q = self.session.as_ref()?.quick_play.as_ref()?;
        self.seeds.by_quick_play(&q.key()).map(|w| w.seed)
    }

    /// Switch dimension. The view keeps its place: Nether coordinates are an eighth of the
    /// Overworld's.
    pub fn set_dimension(&mut self, d: Dimension) {
        if d == self.dimension {
            return;
        }
        let k = match (self.dimension, d) {
            (Dimension::Overworld, Dimension::Nether) => 0.125,
            (Dimension::Nether, Dimension::Overworld) => 8.0,
            _ => 1.0,
        };
        if d == Dimension::End {
            self.camera.center = [0.0, 0.0];
        } else if self.dimension != Dimension::End {
            self.camera.center = [self.camera.center[0] * k, self.camera.center[1] * k];
        }
        let seed = self.tiles.world().map(|w| w.seed);
        if let Some(seed) = seed {
            self.load_world(seed, d);
        } else {
            self.dimension = d;
        }
    }

    /// Switch world preset (Large Biomes, Amplified). Only the Overworld changes.
    pub fn set_preset(&mut self, p: Preset) {
        if p == self.preset {
            return;
        }
        self.preset = p;
        if let Some(seed) = self.tiles.world().map(|w| w.seed) {
            self.loaded = None;
            self.load_world(seed, self.dimension);
        }
    }

    fn load_world(&mut self, seed: i64, dimension: Dimension) {
        if self.loaded == Some((seed, dimension, self.preset)) && self.tiles.world().is_some() {
            return;
        }
        let Some(pack) = self.pack.clone() else {
            self.status = "No version pack: run `cargo run -q -p xtask -- extract --version 26.3`".into();
            return;
        };
        match mcmap_worldgen::world::World::for_world_with_spawn(
            &pack,
            seed,
            dimension,
            self.preset,
            dimension == Dimension::Overworld,
        ) {
            Ok((wg, spawn)) => {
                self.dimension = dimension;
                self.loaded = Some((seed, dimension, self.preset));
                let cache = crate::diskcache::TileCache::for_world(&pack.version, wg.surface.names(), seed, dimension, self.preset);
                self.tiles.set_world(Some(Arc::new(
                    World::new(seed, wg.surface.clone()).with_cache(cache.clone()).with_terrain(wg.terrain.clone()),
                )));
                self.selected = None;
                match Engine::from_world(&pack, wg, spawn, cache) {
                    Ok(e) => self.markers.set_engine(Some(Arc::new(e))),
                    Err(e) => {
                        self.markers.set_engine(None);
                        self.status = format!("Structures: {e}");
                    }
                }
                if !self.status.starts_with("The link") {
                    self.status.clear();
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Take queued position fixes. True when the view must re-run (the first fix, a world or
    /// dimension change, or a camera move that changes the visible tiles); live values are
    /// updated either way.
    pub fn take_fixes(&mut self, tx: &mut Tx) -> bool {
        let Some(rx) = &self.fixes else { return false };
        let mut last = None;
        while let Ok(f) = rx.try_recv() {
            last = Some(f);
        }
        match last {
            Some(f) => self.apply_fix(f, tx),
            None => false,
        }
    }

    /// Take one fix from any source.
    pub fn apply_fix(&mut self, f: Fix, tx: &mut Tx) -> bool {
        // The agent is better than a copied line: a fresh agent fix wins over the clipboard.
        if f.source == SourceKind::Clipboard && self.player.as_ref().is_some_and(|p| p.source == SourceKind::Agent) && !self.stale {
            return false;
        }
        let first = self.player.is_none();
        // A replay carries no world: it plays over whatever seed is shown.
        let new_world = f.source != SourceKind::Replay && self.player.as_ref().is_none_or(|p| p.world != f.world);
        let mut rerun = first || self.stale;
        self.stale = false;
        if first {
            self.follow = true;
        }
        if new_world {
            // A different world: switch to its seed if known, else ask for it once.
            match self.world_seed(&f) {
                Some(seed) => {
                    let d = f.dimension.unwrap_or(self.dimension);
                    if self.tiles.world().is_none_or(|w| w.seed != seed) {
                        self.load_world(seed, d);
                        self.seed_field = TextEdit::new(&seed.to_string());
                        self.pending_zoom = None;
                        rerun = true;
                    }
                }
                None => {
                    self.pending_zoom = f.world.zoom_seed;
                    let place = f.world.server.clone().unwrap_or_else(|| "this world".into());
                    self.status = format!("Type the seed of {place} once and the map remembers it");
                    rerun = true;
                }
            }
        }
        if self.follow
            && let Some(d) = f.dimension
            && d != self.dimension
        {
            self.set_dimension(d);
            rerun = true;
        }
        self.player = Some(f);
        self.age.set(self.age_text());
        if self.follow {
            let p = self.player.as_ref().unwrap().pos;
            self.camera.center = [p[0], p[2]];
        }
        if rerun {
            return true;
        }
        let shown = self.shown.borrow().clone();
        match shown {
            Some(old) => {
                let new = self.layout_for(old.size, old.scale);
                if new.level == old.level && new.tiles == old.tiles && new.origin == old.origin {
                    self.publish(&old);
                    tx.live_only();
                }
                true
            }
            None => true,
        }
    }

    pub fn refresh_age(&self) {
        self.age.set(self.age_text());
    }

    /// Seconds since the last fix.
    pub fn fix_age(&self) -> Option<f64> {
        let p = self.player.as_ref()?;
        Some(p.taken_at.elapsed().map(|d| d.as_secs_f64()).unwrap_or(0.0))
    }

    fn age_text(&self) -> String {
        let Some(p) = &self.player else { return String::new() };
        let src = match p.source {
            SourceKind::Agent => "agent",
            SourceKind::Clipboard => "F3+C",
            SourceKind::Replay => "replay",
        };
        let age = self.fix_age().unwrap_or(0.0);
        if age < 2.0 { format!("{src}, live") } else if age < 120.0 { format!("{src}, {age:.0} s ago") } else { format!("{src}, {:.0} min ago", age / 60.0) }
    }

    /// Drain the live sources' notes and session changes, and refresh the fix age. True when
    /// the view must re-run.
    pub fn take_live_events(&mut self, tx: &mut Tx) -> bool {
        let mut events = Vec::new();
        if let Some(l) = &self.live {
            while let Ok(e) = l.events.try_recv() {
                events.push(e);
            }
        }
        let mut rerun = false;
        for e in events {
            match e {
                live::Event::Note(n) => {
                    self.status = n;
                    rerun = true;
                }
                live::Event::Session(s) => {
                    rerun |= self.set_session(s);
                }
                live::Event::Tick => {}
            }
        }
        if self.player.is_some() {
            self.age.set(self.age_text());
            let stale = self.fix_age().is_some_and(|a| a > STALE_SECS);
            if stale != self.stale {
                self.stale = stale;
                rerun = true;
            }
            if !rerun {
                tx.live_only();
            }
        }
        rerun
    }

    /// The running game changed. Before any fix arrives, show the launcher world's seed if the
    /// map knows it.
    pub fn set_session(&mut self, s: Option<live::Session>) -> bool {
        self.session = s;
        let Some(sess) = &self.session else {
            self.status = "Game closed".into();
            return true;
        };
        let place = sess.quick_play.as_ref().and_then(|q| q.name.clone());
        self.status = format!("Game found{}", place.as_deref().map(|n| format!(": {n}")).unwrap_or_default());
        if self.player.is_none()
            && let Some(q) = &sess.quick_play
            && let Some(w) = self.seeds.by_quick_play(&q.key())
        {
            let seed = w.seed;
            self.load_world(seed, self.dimension);
            self.seed_field = TextEdit::new(&seed.to_string());
            self.status = format!("Game found: {}, seed {seed}", place.as_deref().unwrap_or("last world"));
        }
        true
    }

    /// Text from the clipboard or a paste: an F3+C line becomes a fix.
    pub fn clipboard_text(&mut self, t: &str, tx: &mut Tx) -> bool {
        match mcmap_live::protocol::parse_f3c(t.trim()) {
            Some(f) if self.player.as_ref().is_none_or(|p| p.source == SourceKind::Clipboard || self.stale) => {
                if self.player.as_ref().is_some_and(|p| p.source == SourceKind::Clipboard && p.pos == f.pos && p.yaw == f.yaw) {
                    return false;
                }
                self.apply_fix(f, tx)
            }
            _ => false,
        }
    }
}

impl Map {
    /// Background marker work finished, or a nearest search answered. True when the view must
    /// re-run.
    pub fn take_marker_updates(&mut self) -> bool {
        let mut changed = self.markers.take_dirty();
        if let Some((label, found)) = self.markers.take_nearest() {
            changed = true;
            match found {
                Some(f) => {
                    let sel = Selected::of(&f);
                    self.follow = false;
                    self.camera.center = [sel.x as f64 + 0.5, sel.z as f64 + 0.5];
                    if self.camera.zoom < 0.5 {
                        self.camera.zoom = 0.5;
                    }
                    if let Some(fs) = &mut self.features
                        && !fs.is_on(&label)
                    {
                        fs.toggle(&label);
                    }
                    self.status.clear();
                    self.selected = Some(sel);
                }
                None => self.status = format!("No {label} within about 24,000 blocks"),
            }
        }
        changed
    }

    /// Start a nearest search for a picker label, from the player if following, else the view centre.
    pub fn nearest(&mut self, label: &str) {
        let from = match (&self.player, self.follow) {
            (Some(p), true) => [p.pos[0], p.pos[2]],
            _ => self.camera.center,
        };
        if label == "Spawn Point" {
            if let Some((x, z)) = self.markers.spawn() {
                self.camera.center = [x as f64 + 0.5, z as f64 + 0.5];
                self.follow = false;
                self.selected = Some(Selected { name: "World spawn (search point)".into(), x, y: None, z, likely: true });
            }
            return;
        }
        match self.markers.find_nearest(label, from[0], from[1]) {
            Ok(()) => self.status = format!("Searching for the nearest {label}…"),
            Err(e) => self.status = e,
        }
    }
}

/// A panel's texture, behind its children, at the size it had last frame.
fn textured(s: &Map, cx: &mut Cx<Map>, key: &str, t: Texture, panel: El) -> El {
    let (Some(c), Some(size)) = (&s.chrome, cx.layout_size(key)) else { return panel };
    let scale = cx.env().scale;
    let radius = if t == Texture::Content { 8.0 } else { 0.0 };
    let alpha = if t == Texture::Content { 0.92 } else { 1.0 };
    panel.child(el().key(format!("{key}/texture")).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)).radius(radius).opacity(alpha).pixels(&c.pixels(t, size, scale)))
}

fn style(size: f32) -> TextStyle {
    TextStyle::new(size, 400.0)
}

fn button(label: &str, active: bool) -> El {
    let (bg, fg) = if active { (rgb(0.30, 0.42, 0.62), rgb(0.97, 0.97, 0.98)) } else { (rgb(0.18, 0.18, 0.22), rgb(0.80, 0.82, 0.86)) };
    row().h(26.0).px(10.0).items_center().radius(6.0).fill(bg).child(text(label).color(fg))
}

pub fn view(s: &Map, cx: &mut Cx<Map>) -> El {
    let scale = cx.env().scale as f64;
    let size = cx.layout_size("canvas").map(|[w, h]| [w as f64, h as f64]).unwrap_or([0.0, 0.0]);
    let l = s.layout_for(size, scale);
    s.publish(&l);

    let show_biomes = s.features.as_ref().is_none_or(|f| f.is_on("Biomes"));
    let tile_pts = TILE as f64 / scale;
    let mut world = el().key("world").size(size[0] as f32, size[1] as f32).live_offset(&s.pan).live_scale(&s.k);
    if size[0] > 0.0 && show_biomes {
        s.tiles.begin_frame();
        s.tiles.prefetch_sync(&l.tiles);
        let centre_tile = tile_blocks(l.level);
        for &t in &l.tiles {
            let p = Map::point(&l, t.origin());
            let o = t.origin();
            let d = [(o[0] + centre_tile / 2.0 - s.camera.center[0]) / centre_tile, (o[1] + centre_tile / 2.0 - s.camera.center[1]) / centre_tile];
            let px = s.tiles.request(t, d[0].hypot(d[1]));
            world = world.child(
                el().key(format!("tile/{}/{}/{}", t.level, t.x, t.z))
                    .pin(Some(p[0] as f32), Some(p[1] as f32), None, None)
                    .size(tile_pts as f32, tile_pts as f32)
                    .pixels(&px),
            );
        }
        s.tiles.end_frame();
    }
    if size[0] > 0.0 && s.sprite_demo
        && let Some(sp) = &s.sprites
    {
        let factor = scale.round().max(1.0) as u32;
        for (i, m) in sp.markers().iter().enumerate() {
            let img = sp.pixels(m.sprite, factor);
            let (w, h) = (m.size[0] as f64, m.size[1] as f64);
            let p = Map::point(&l, m.block);
            world = world.child(
                el().key(format!("marker/{i}"))
                    .pin(Some((p[0] - w / 2.0) as f32), Some((p[1] - h / 2.0) as f32), None, None)
                    .size(w as f32, h as f32)
                    .live_scale(&s.inv_k)
                    .pixels(&img),
            );
        }
    }
    if size[0] > 0.0
        && let Some(f) = &s.features
    {
        world = overlay(s, f, &l, world, scale);
    }
    *s.shown.borrow_mut() = (size[0] > 0.0).then_some(l);

    let mut canvas = el().key("canvas").w_full().grow(1.0).clip_soft(0.0).fill(rgb(0.08, 0.08, 0.1)).child(world);
    if s.player.as_ref().is_some_and(|p| p.dimension.is_none_or(|d| d == s.dimension)) {
        let o = if s.stale { 0.45 } else { 1.0 };
        canvas = canvas
            .child(
                el().key("player").pin(Some(0.0), Some(0.0), None, None).size(MARKER, MARKER).live_offset(&s.player_at)
                    .radius(MARKER / 2.0).fill(rgb(0.95, 0.25, 0.2)).border(2.5, Color::WHITE).opacity(o),
            )
            .child(
                el().key("heading").pin(Some(0.0), Some(0.0), None, None).size(HEADING, HEADING).live_offset(&s.heading_at)
                    .radius(HEADING / 2.0).fill(Color::WHITE).opacity(o),
            );
    }
    if let Some(f) = &s.features {
        let factor = scale.round().max(1.0) as u32;
        const COLUMNS: usize = 3;
        let mut picker = textured(s, cx, "features", Texture::Content, col().key("features").pin(Some(10.0), Some(10.0), None, None).padding(6.0).gap(4.0).radius(8.0).fill(Color { r: 0.1, g: 0.1, b: 0.12, a: 0.85 }));
        let mut line = row().gap(4.0);
        let shown: Vec<(usize, &crate::features::Feature)> = f.list.iter().enumerate().filter(|(_, x)| x.dimension.is_none_or(|d| d == s.dimension)).collect();
        let count = shown.len();
        for (n, (i, feat)) in shown.into_iter().enumerate() {
            let on = f.is_on(&feat.label);
            let [w, h] = f.size(i);
            let label = feat.label.clone();
            line = line.child(
                el().key(format!("feature/{}", feat.label)).size(w + 6.0, h + 6.0).padding(3.0).radius(5.0)
                    .fill(if on { rgb(0.30, 0.42, 0.62) } else { Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } })
                    .opacity(if on { 1.0 } else { 0.45 })
                    .child(el().size(w, h).pixels(&f.pixels(i, factor)))
                    .on_press(move |s: &mut Map| {
                        if let Some(f) = &mut s.features {
                            f.toggle(&label);
                            s.status = picker_hint(&label, f.is_on(&label), s.camera.zoom);
                        }
                    })
                    .on_secondary_press({
                        let label = feat.label.clone();
                        move |s: &mut Map, _p, _tx: &mut Tx| s.nearest(&label)
                    }),
            );
            if n % COLUMNS == COLUMNS - 1 || n + 1 == count {
                picker = picker.child(std::mem::replace(&mut line, row().gap(4.0)));
            }
        }
        canvas = canvas.child(picker);
    }
    if let Some(sel) = &s.selected {
        let y = match sel.y {
            Some(y) if sel.likely => format!("  y ≈ {y}"),
            Some(y) => format!("  y {y}"),
            None => String::new(),
        };
        let tp = sel.tp();
        let mut info = col().key("selected").pin(None, Some(10.0), Some(10.0), None).padding(10.0).gap(6.0).radius(8.0)
            .fill(Color { r: 0.1, g: 0.1, b: 0.12, a: 0.9 })
            .child(text(&sel.name).color(rgb(0.97, 0.97, 0.98)))
            .child(text(format!("x {}  z {}{y}", sel.x, sel.z)).color(rgb(0.80, 0.82, 0.86)));
        if sel.likely {
            info = info.child(text("Likely: depends on the terrain height estimate").color(rgb(0.95, 0.75, 0.45)));
        }
        info = info.child(
            row().gap(6.0)
                .child(button("Copy /tp", false).key("copy-tp").on_press_tx(move |_s: &mut Map, tx: &mut Tx| tx.copy(tp.clone())))
                .child(button("Close", false).key("close-selected").on_press(|s: &mut Map| s.selected = None)),
        );
        canvas = canvas.child(info);
    }
    if s.hovering {
        canvas = canvas.child(
        row().key("hover").pin(Some(10.0), None, None, Some(10.0)).h(24.0).px(8.0).items_center().radius(6.0)
            .fill(Color { r: 0.1, g: 0.1, b: 0.12, a: 0.85 })
            .child(text("-00000000, -00000000   old growth spruce taiga").live_text(&s.hover).color(rgb(0.92, 0.93, 0.95))),
        );
    }
    let canvas = canvas
        .on_wheel(|s: &mut Map, e: WheelEvent, tx| {
            if e.precise {
                // Trackpad: two fingers pan.
                s.pan_by(e.dx as f64, e.dy as f64, tx);
            } else {
                // Mouse wheel: zoom about the pointer, 1.25x per notch-sized step.
                let factor = 1.25f64.powf(e.dy as f64 / 40.0);
                s.zoom_about([e.x as f64, e.y as f64], factor, tx);
            }
            true
        })
        .on_magnify(|s: &mut Map, e: MagnifyEvent, tx| {
            s.zoom_about([e.x as f64, e.y as f64], 1.0 + e.delta as f64, tx);
            true
        })
        .on_drag(|s: &mut Map, d: &mut Drag, tx: &mut Tx| s.drag(d, tx))
        .on_pointer_move_local(|s: &mut Map, e: PointerEvent, tx: &mut Tx| {
            s.hover.set(s.hover_at([e.x as f64, e.y as f64]));
            if s.hovering {
                tx.live_only();
            } else {
                s.hovering = true;
            }
        })
        .on_hover_change(|s: &mut Map, inside: bool, _tx: &mut Tx| s.hovering = inside);

    let seed = text_input(&s.seed_field, style(13.0).lh(20.0)).key("seed").single_line().w(250.0)
        .color(rgb(0.95, 0.95, 0.97)).placeholder("Seed, or paste a seed-map link", rgb(0.55, 0.57, 0.62))
        .on_edit(|s: &mut Map| &mut s.seed_field)
        .on_submit(|s: &mut Map, _tx: &mut Tx| s.submit_seed());
    let seed_box = row().h(28.0).px(8.0).items_center().radius(6.0).fill(rgb(0.06, 0.06, 0.08)).border(1.0, rgb(0.28, 0.29, 0.34)).child(seed);

    let dim_button = |label: &str, d: Dimension| {
        button(label, s.dimension == d).key(format!("dim/{label}")).on_press(move |s: &mut Map| s.set_dimension(d))
    };
    let dims = row().gap(4.0)
        .child(dim_button("Overworld", Dimension::Overworld))
        .child(dim_button("Nether", Dimension::Nether))
        .child(dim_button("End", Dimension::End));
    let version = s.tiles.world().map(|_| "Java 26.3").unwrap_or("no pack");
    let mut bar = textured(s, cx, "bar", Texture::Page, row().key("bar").w_full().h(BAR).px(10.0).gap(10.0).items_center().fill(rgb(0.12, 0.12, 0.15)))
        .child(text("Seed").color(rgb(0.7, 0.72, 0.76)))
        .child(seed_box)
        .child(button(version, false).key("version"))
        .child(button(s.preset.label(), s.preset != Preset::Normal).key("preset").on_press(|s: &mut Map| {
            let i = Preset::ALL.iter().position(|&p| p == s.preset).unwrap_or(0);
            s.set_preset(Preset::ALL[(i + 1) % Preset::ALL.len()]);
        }))
        .child(dims)
        .child(button("Relief", s.tiles.relief()).key("relief").on_press(|s: &mut Map| s.tiles.set_relief(!s.tiles.relief())));
    if s.player.is_some() {
        bar = bar.child(button(if s.follow { "Following" } else { "Follow" }, s.follow).key("follow").on_press_tx(|s: &mut Map, tx: &mut Tx| {
            s.follow = !s.follow;
            if s.follow
                && let Some(p) = &s.player
            {
                s.camera.center = [p.pos[0], p.pos[2]];
                s.camera_moved(tx);
            }
        }));
    }
    if s.player.is_some() {
        bar = bar.child(text("F3+C, 0000 min ago").live_text(&s.age).color(if s.stale { rgb(0.6, 0.6, 0.62) } else { rgb(0.55, 0.85, 0.55) }));
    }
    if !s.status.is_empty() {
        bar = bar.child(text(&s.status).color(rgb(0.95, 0.75, 0.45)));
    }
    bar = bar.child(el().grow(1.0)).child(text("x 00000000  z 00000000   000.000 px/block").live_text(&s.readout).color(rgb(0.65, 0.68, 0.72)));
    let root = col().w_full().h_full().child(bar).child(canvas).on_paste(|s: &mut Map, t: &str, tx: &mut Tx| {
        if !s.clipboard_text(t, tx) && parse_link(t).is_some() {
            // A seed-map link pasted on the map: same as the seed field.
            s.seed_field = TextEdit::new(t.trim());
            s.submit_seed();
        }
    });
    // Without a background clipboard watch, read it when the window comes back into focus:
    // press F3+C in the game, switch to the map, and the position is there.
    if s.live.as_ref().is_some_and(|l| l.watch != mcmap_live::clipboard::Watch::Available) {
        root.on_window_focus(|_s: &mut Map, focused: bool, tx: &mut Tx| {
            if focused {
                tx.read_clipboard(|s: &mut Map, c, tx| {
                    s.clipboard_text(c.text(), tx);
                });
            }
        })
    } else {
        root
    }
}

/// The status line after a picker toggle.
fn picker_hint(label: &str, on: bool, zoom: f64) -> String {
    if !on || label == "Biomes" {
        return String::new();
    }
    if label == "Slime Chunk" {
        return if zoom < SLIME_ZOOM { "Zoom in to see slime chunks".into() } else { String::new() };
    }
    if label != "Spawn Point" && markers::kind(label).is_none() {
        return format!("{label}: decoration features come in M4");
    }
    if zoom < STRUCTURE_ZOOM { "Zoom in to see structures. Right-click an icon to jump to the nearest one".into() } else { "Right-click an icon to jump to the nearest one".into() }
}

/// Structure, spawn and slime-chunk markers over the laid-out tiles.
fn overlay(s: &Map, f: &Features, l: &Layout, mut world: El, scale: f64) -> El {
    if l.tiles.is_empty() {
        return world;
    }
    let span = tile_blocks(l.level);
    let (mut x0, mut z0, mut x1, mut z1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for t in &l.tiles {
        let o = t.origin();
        x0 = x0.min(o[0]);
        z0 = z0.min(o[1]);
        x1 = x1.max(o[0] + span);
        z1 = z1.max(o[1] + span);
    }
    let d = blocks_per_pixel(l.level) * l.scale;
    let zoom = s.camera.zoom;

    if f.is_on_in("Slime Chunk", s.dimension) && zoom >= SLIME_ZOOM {
        let seed = s.tiles.world().map(|w| w.seed);
        if let Some(seed) = seed {
            let (cx0, cz0, cx1, cz1) = ((x0 / 16.0).floor() as i32, (z0 / 16.0).floor() as i32, (x1 / 16.0).ceil() as i32, (z1 / 16.0).ceil() as i32);
            if ((cx1 - cx0) as i64) * ((cz1 - cz0) as i64) <= 60_000 {
                for cz in cz0..cz1 {
                    for cx in cx0..cx1 {
                        if mcmap_worldgen::random::is_slime_chunk(seed, cx, cz) {
                            let p = Map::point(l, [(cx * 16) as f64, (cz * 16) as f64]);
                            world = world.child(
                                el().key(format!("slime/{cx}/{cz}"))
                                    .pin(Some(p[0] as f32), Some(p[1] as f32), None, None)
                                    .size((16.0 / d) as f32, (16.0 / d) as f32)
                                    .fill(Color { r: 0.30, g: 0.90, b: 0.25, a: 0.6 }),
                            );
                        }
                    }
                }
            }
        }
    }

    let Some(sp) = &s.sprites else { return world };
    let factor = scale.round().max(1.0) as u32;
    let icon = |world: El, key: String, sprite: usize, block: [f64; 2], sel: Selected| -> El {
        if sprite >= sp.len() {
            return world;
        }
        let [w, h] = sp.size(sprite);
        let p = Map::point(l, block);
        world.child(
            el().key(key)
                .pin(Some((p[0] - w as f64 / 2.0) as f32), Some((p[1] - h as f64 / 2.0) as f32), None, None)
                .size(w, h)
                .live_scale(&s.inv_k)
                .pixels(&sp.pixels(sprite, factor))
                .on_press(move |s: &mut Map| s.selected = Some(sel.clone())),
        )
    };

    let any = markers::KINDS.iter().any(|k| f.is_on_in(k.label, s.dimension));
    if any && zoom >= STRUCTURE_ZOOM {
        let r = (REGION * 16) as f64;
        let (rx0, rz0, rx1, rz1) = ((x0 / r).floor() as i32, (z0 / r).floor() as i32, ((x1 / r).ceil() as i32) - 1, ((z1 / r).ceil() as i32) - 1);
        let mut asked = Vec::with_capacity(((rx1 - rx0 + 1).max(0) * (rz1 - rz0 + 1).max(0)) as usize);
        for rz in rz0..=rz1 {
            for rx in rx0..=rx1 {
                asked.push((rx, rz));
            }
        }
        let mut found: Vec<Found> = Vec::new();
        let c = s.camera.center;
        let store = &s.markers;
        store.begin_frame();
        store.prefetch_sync(&asked);
        for &(rx, rz) in &asked {
            let mid = [(rx as f64 + 0.5) * r, (rz as f64 + 0.5) * r];
            let pr = (mid[0] - c[0]).hypot(mid[1] - c[1]) / r;
            if let Some(v) = store.request((rx, rz), pr) {
                found.extend(v.iter().cloned());
            }
        }
        store.end_frame(&asked);
        if f.is_on_in("Stronghold", s.dimension)
            && let Some(sh) = s.markers.strongholds()
        {
            found.extend(sh.iter().cloned());
        }
        for st in &found {
            let Some(k) = markers::kind_of(&st.structure) else { continue };
            if !f.is_on_in(k.label, s.dimension) {
                continue;
            }
            let (bx, bz) = (st.marker.0 as f64 + 0.5, st.marker.1 as f64 + 0.5);
            if bx < x0 || bx > x1 || bz < z0 || bz > z1 {
                continue;
            }
            let key = format!("structure/{}/{}/{}", st.structure, st.chunk.0, st.chunk.1);
            world = icon(world, key, markers::sprite_for(k, &st.structure), [bx, bz], Selected::of(st));
        }
    }
    if f.is_on_in("Spawn Point", s.dimension)
        && let Some((x, z)) = s.markers.spawn()
    {
        let sel = Selected { name: "World spawn (search point)".into(), x, y: None, z, likely: true };
        world = icon(world, "spawn".into(), markers::SPAWN_SPRITE, [x as f64 + 0.5, z as f64 + 0.5], sel);
    }
    world
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}
