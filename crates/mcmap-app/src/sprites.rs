//! Marker sprites sliced from `art/seed-map/map/marker-atlas.webp`. The art is personal-use only
//! and is never in the repo: it is read at run time from the art directory (see `main.rs`).

use mcmap_tiles::atlas::{segment, Rgba, MARKER_ALPHA};
use rune::Pixels;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

pub struct Marker {
    pub sprite: usize,
    /// Block coordinate (x, z) of the marker's centre.
    pub block: [f64; 2],
    /// Size on screen in points, constant at every zoom.
    pub size: [f32; 2],
}

pub struct Sprites {
    images: Vec<Rgba>,
    /// Uploaded images per (sprite, whole-number enlargement).
    uploaded: RefCell<HashMap<(usize, u32), Pixels>>,
    markers: Vec<Marker>,
}

impl Sprites {
    pub fn load(art: &Path) -> Result<Sprites, String> {
        let path = art.join("map/marker-atlas.webp");
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Sprites::from_atlas(&Rgba::decode(&bytes)?))
    }

    pub fn from_atlas(atlas: &Rgba) -> Sprites {
        let rects = segment(atlas, MARKER_ALPHA, 0);
        let images: Vec<Rgba> = rects.iter().map(|&r| atlas.crop(r)).collect();
        // M0 has no structure engine yet: lay every sprite out in a grid north-west of the
        // reference view's centre, 32 blocks apart, so the camera can be checked against them.
        let markers = rects
            .iter()
            .enumerate()
            .map(|(i, r)| Marker {
                sprite: i,
                block: [2839.0 - 176.0 + 32.0 * (i % 12) as f64, -1418.0 - 80.0 + 32.0 * (i / 12) as f64],
                size: [r.w as f32, r.h as f32],
            })
            .collect();
        Sprites { images, uploaded: RefCell::new(HashMap::new()), markers }
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }

    /// The sprite enlarged `factor` times with nearest-neighbour, so it stays crisp when drawn at
    /// `factor` device pixels per point.
    pub fn pixels(&self, sprite: usize, factor: u32) -> Pixels {
        self.uploaded
            .borrow_mut()
            .entry((sprite, factor))
            .or_insert_with(|| {
                let img = self.images[sprite].upscale(factor);
                let px = Pixels::new();
                px.set(img.w, img.h, img.px);
                px
            })
            .clone()
    }
}
