//! The map: an `f64` camera over a tile pyramid, drawn with Rune's compositor-side camera.
//!
//! The view lays tiles out once per (level, origin, tile range) and binds the world element to a
//! `Live` offset and scale. Pan and zoom handlers move only those live values and say
//! `tx.live_only()`, so the view re-runs only when the set of visible tiles changes.
//!
//! Layout space: the world element is exactly the canvas, and the block `origin` sits at its
//! centre. One tile pixel is `1 / scale` points before the live scale, so a tile shows at
//! `k = zoom * scale * blocks_per_pixel` device pixels per tile pixel, always in (0.5, 1].

use crate::sprites::Sprites;
use crate::tiles::{TileStore, World};
use mcmap_tiles::pyramid::{blocks_per_pixel, Camera, TileKey, TILE};
use rune::{Color, Cx, Drag, El, Live, LiveText, MagnifyEvent, Phase, Tx, WheelEvent, col, el, row, text};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// The reference capture's view (seed -7556262131344217808, Java 26.3): 1.25 px per block.
pub const HOME: Camera = Camera { center: [2839.0, -1418.0], zoom: 1.25 };

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
    /// What the last view run laid out; handlers compare against it.
    shown: RefCell<Option<Layout>>,
    pub tiles: TileStore,
    pub sprites: Option<Sprites>,
    pan_at_press: Cell<[f64; 2]>,
}

impl Map {
    pub fn new(sprites: Option<Sprites>, world: Option<Arc<World>>) -> Map {
        Map {
            camera: HOME,
            pan: Live::new([0.0, 0.0]),
            k: Live::new(1.0),
            inv_k: Live::new(1.0),
            readout: LiveText::new(""),
            shown: RefCell::new(None),
            tiles: TileStore::new(world),
            sprites,
            pan_at_press: Cell::new(HOME.center),
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
        self.readout.set(format!("x {x:.1}  z {z:.1}   {:.3} px/block   level {}", self.camera.zoom * l.scale, l.level));
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
        self.camera.center[0] -= dx_points / z;
        self.camera.center[1] -= dy_points / z;
        self.camera_moved(tx);
    }

    pub fn zoom_about(&mut self, point: [f64; 2], factor: f64, tx: &mut Tx) {
        let size = self.shown.borrow().as_ref().map(|l| l.size).unwrap_or([0.0, 0.0]);
        self.camera.zoom_about(point, size, factor);
        self.camera_moved(tx);
    }

    fn drag(&mut self, d: &Drag, tx: &mut Tx) {
        if d.phase == Phase::Down {
            self.pan_at_press.set(self.camera.center);
        }
        let start = self.pan_at_press.get();
        let z = self.camera.zoom;
        self.camera.center = if d.cancelled() { start } else { [start[0] - d.dx as f64 / z, start[1] - d.dy as f64 / z] };
        self.camera_moved(tx);
    }
}

pub fn view(s: &Map, cx: &mut Cx<Map>) -> El {
    let scale = cx.env().scale as f64;
    let size = cx.layout_size("canvas").map(|[w, h]| [w as f64, h as f64]).unwrap_or([0.0, 0.0]);
    let l = s.layout_for(size, scale);
    s.publish(&l);

    let tile_pts = TILE as f64 / scale;
    let mut world = el().key("world").size(size[0] as f32, size[1] as f32).live_offset(&s.pan).live_scale(&s.k);
    if size[0] > 0.0 {
        s.tiles.begin_frame();
        for &t in &l.tiles {
            let p = Map::point(&l, t.origin());
            let px = s.tiles.request(t);
            world = world.child(
                el().key(format!("tile/{}/{}/{}", t.level, t.x, t.z))
                    .pin(Some(p[0] as f32), Some(p[1] as f32), None, None)
                    .size(tile_pts as f32, tile_pts as f32)
                    .pixels(&px),
            );
        }
        if let Some(sp) = &s.sprites {
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
    }
    *s.shown.borrow_mut() = (size[0] > 0.0).then_some(l);

    let canvas = el().key("canvas").w_full().grow(1.0).clip_soft(0.0).fill(rgb(0.08, 0.08, 0.1)).child(world)
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
        .on_drag(|s: &mut Map, d: &mut Drag, tx: &mut Tx| s.drag(d, tx));

    let bar = row().key("bar").w_full().h(28.0).px(10.0).gap(16.0).items_center().fill(rgb(0.12, 0.12, 0.15))
        .child(text(match &s.tiles.world {
            Some(w) => format!("MC Mapper · 26.3 · seed {}", w.seed),
            None => "MC Mapper · no version pack (checkerboard)".to_owned(),
        }).color(rgb(0.9, 0.9, 0.92)))
        .child(text("x 00000000.0  z 00000000.0   000.000 px/block   level -00").live_text(&s.readout).color(rgb(0.65, 0.68, 0.72)));
    col().w_full().h_full().child(bar).child(canvas)
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}
