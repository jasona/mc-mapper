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

use crate::features::Features;
use crate::markers::{self, Engine, MarkerStore, REGION};
use crate::sprites::Sprites;
use crate::tiles::{TileStore, World};
use mcmap_data::link::{parse_link, seed_from_text};
use mcmap_data::pack::RawPack;
use mcmap_live::Fix;
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
    /// The pointer is over the canvas: show the hover readout.
    pub hovering: bool,
    /// Draw the whole marker atlas as a grid (M0 check, `MCMAP_SPRITE_DEMO`).
    pub sprite_demo: bool,
    pub markers: MarkerStore,
    pub selected: Option<Selected>,
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
            hovering: false,
            sprite_demo: true,
            markers: MarkerStore::new(),
            selected: None,
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
            if l.dimension.is_some_and(|d| d != mcmap_data::Dimension::Overworld) {
                self.status = "Only the Overworld is mapped so far (the Nether and End come in M4)".into();
            }
            if l.version.as_deref().is_some_and(|v| v != "26.3") {
                self.status = format!("The link is for {}; showing 26.3, the only version pack so far", l.version.as_deref().unwrap_or("?"));
            }
        }
        self.set_seed(seed);
        self.seed_field = TextEdit::new(&seed.to_string());
    }

    pub fn set_seed(&mut self, seed: i64) {
        if self.tiles.world().is_some_and(|w| w.seed == seed) {
            return;
        }
        let Some(pack) = self.pack.clone() else {
            self.status = "No version pack: run `cargo run -q -p xtask -- extract --version 26.3`".into();
            return;
        };
        match mcmap_worldgen::Surface::new(&pack, seed) {
            Ok(s) => {
                self.tiles.set_world(Some(Arc::new(World::new(seed, s))));
                self.selected = None;
                match Engine::new(&pack, seed) {
                    Ok(e) => self.markers.set_engine(Some(Arc::new(e))),
                    Err(e) => {
                        self.markers.set_engine(None);
                        self.status = format!("Structures: {e}");
                    }
                }
                if !self.status.starts_with("Only") && !self.status.starts_with("The link") {
                    self.status.clear();
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Take queued position fixes. True when the view must re-run (the first fix, or a camera
    /// move that changes the visible tiles); live values are updated either way.
    pub fn take_fixes(&mut self, tx: &mut Tx) -> bool {
        let Some(rx) = &self.fixes else { return false };
        let mut last = None;
        while let Ok(f) = rx.try_recv() {
            last = Some(f);
        }
        let Some(f) = last else { return false };
        let first = self.player.is_none();
        self.player = Some(f);
        if first {
            return true;
        }
        if self.follow {
            let p = self.player.as_ref().unwrap().pos;
            self.camera.center = [p[0], p[2]];
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
    if s.player.is_some() {
        canvas = canvas.child(
            el().key("player").pin(Some(0.0), Some(0.0), None, None).size(MARKER, MARKER).live_offset(&s.player_at)
                .radius(MARKER / 2.0).fill(rgb(0.95, 0.25, 0.2)).border(2.5, Color::WHITE),
        );
    }
    if let Some(f) = &s.features {
        let factor = scale.round().max(1.0) as u32;
        const COLUMNS: usize = 3;
        let mut picker = col().key("features").pin(Some(10.0), Some(10.0), None, None).padding(6.0).gap(4.0).radius(8.0).fill(Color { r: 0.1, g: 0.1, b: 0.12, a: 0.85 });
        let mut line = row().gap(4.0);
        for (i, feat) in f.list.iter().enumerate() {
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
            if i % COLUMNS == COLUMNS - 1 || i + 1 == f.list.len() {
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

    let dims = row().gap(4.0)
        .child(button("Overworld", true).key("dim/overworld"))
        .child(button("Nether", false).key("dim/nether").opacity(0.5).on_press(|s: &mut Map| s.status = "The Nether comes in M4".into()))
        .child(button("End", false).key("dim/end").opacity(0.5).on_press(|s: &mut Map| s.status = "The End comes in M4".into()));
    let version = s.tiles.world().map(|_| "Java 26.3").unwrap_or("no pack");
    let mut bar = row().key("bar").w_full().h(BAR).px(10.0).gap(10.0).items_center().fill(rgb(0.12, 0.12, 0.15))
        .child(text("Seed").color(rgb(0.7, 0.72, 0.76)))
        .child(seed_box)
        .child(button(version, false).key("version"))
        .child(dims);
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
    if !s.status.is_empty() {
        bar = bar.child(text(&s.status).color(rgb(0.95, 0.75, 0.45)));
    }
    bar = bar.child(el().grow(1.0)).child(text("x 00000000  z 00000000   000.000 px/block").live_text(&s.readout).color(rgb(0.65, 0.68, 0.72)));
    col().w_full().h_full().child(bar).child(canvas)
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

    if f.is_on("Slime Chunk") && zoom >= SLIME_ZOOM {
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

    let any = markers::KINDS.iter().any(|k| f.is_on(k.label));
    if any && zoom >= STRUCTURE_ZOOM {
        let r = (REGION * 16) as f64;
        let (rx0, rz0, rx1, rz1) = ((x0 / r).floor() as i32, (z0 / r).floor() as i32, ((x1 / r).ceil() as i32) - 1, ((z1 / r).ceil() as i32) - 1);
        let mut asked = Vec::new();
        let mut found: Vec<Found> = Vec::new();
        let c = s.camera.center;
        let store = &s.markers;
        store.begin_frame();
        for rz in rz0..=rz1 {
            for rx in rx0..=rx1 {
                let mid = [(rx as f64 + 0.5) * r, (rz as f64 + 0.5) * r];
                let pr = (mid[0] - c[0]).hypot(mid[1] - c[1]) / r;
                asked.push((rx, rz));
                if let Some(v) = store.request((rx, rz), pr) {
                    found.extend(v.iter().cloned());
                }
            }
        }
        store.end_frame(&asked);
        if f.is_on("Stronghold")
            && let Some(sh) = s.markers.strongholds()
        {
            found.extend(sh.iter().cloned());
        }
        for st in &found {
            let Some(k) = markers::kind_of(&st.structure) else { continue };
            if !f.is_on(k.label) {
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
    if f.is_on("Spawn Point")
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
