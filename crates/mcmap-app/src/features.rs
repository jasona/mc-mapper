//! The feature picker's icons, cut from `ui/feature-atlas.webp` with the rectangles in the art
//! manifest. Which features are on is kept here; 

use mcmap_tiles::atlas::{Rect, Rgba};
use rune::Pixels;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "featurePickerSprites")]
    sprites: Sheet,
}

#[derive(Deserialize)]
struct Sheet {
    file: String,
    rectangles: Vec<ManifestRect>,
}

#[derive(Deserialize)]
struct ManifestRect {
    label: String,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

pub struct Feature {
    pub label: String,
    image: Rgba,
    /// The dimension whose picker shows it; `None` for every dimension (Biomes).
    pub dimension: Option<mcmap_data::Dimension>,
}

pub struct Features {
    pub list: Vec<Feature>,
    pub on: BTreeSet<String>,
    uploaded: RefCell<HashMap<(usize, u32), Pixels>>,
}

impl Features {
    pub fn load(art: &Path) -> Result<Features, String> {
        let mpath = art.join("manifest.json");
        let text = std::fs::read_to_string(&mpath).map_err(|e| format!("{}: {e}", mpath.display()))?;
        let m: Manifest = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", mpath.display()))?;
        let ipath = art.join(&m.sprites.file);
        let atlas = Rgba::decode(&std::fs::read(&ipath).map_err(|e| format!("{}: {e}", ipath.display()))?)?;
        let list = m
            .sprites
            .rectangles
            .into_iter()
            .filter(|r| r.x + r.width <= atlas.w && r.y + r.height <= atlas.h)
            .map(|r| Feature {
                image: atlas.crop(Rect { x: r.x, y: r.y, w: r.width, h: r.height }),
                dimension: (r.label != "Biomes").then_some(mcmap_data::Dimension::Overworld),
                label: r.label,
            })
            .collect();
        Ok(Features { list, on: ["Biomes".to_owned()].into(), uploaded: RefCell::new(HashMap::new()) })
    }

    /// Add a picker entry for another dimension, with its icon scaled to the picker's size.
    pub fn push(&mut self, label: &str, image: &Rgba, dimension: mcmap_data::Dimension) {
        let size = self.list.first().map(|f| f.image.w).unwrap_or(22);
        let image = image.fit(size, size);
        self.list.push(Feature { label: label.to_owned(), image, dimension: Some(dimension) });
    }

    /// On, and in the picker of `dimension`.
    pub fn is_on_in(&self, label: &str, dimension: mcmap_data::Dimension) -> bool {
        self.is_on(label) && self.list.iter().any(|f| f.label == label && f.dimension.is_none_or(|d| d == dimension))
    }

    pub fn is_on(&self, label: &str) -> bool {
        self.on.contains(label)
    }

    pub fn toggle(&mut self, label: &str) {
        if !self.on.remove(label) {
            self.on.insert(label.to_owned());
        }
    }

    /// Icon `i` enlarged `factor` times, nearest-neighbour.
    pub fn pixels(&self, i: usize, factor: u32) -> Pixels {
        self.uploaded
            .borrow_mut()
            .entry((i, factor))
            .or_insert_with(|| {
                let img = self.list[i].image.upscale(factor);
                let px = Pixels::new();
                px.set(img.w, img.h, img.px);
                px
            })
            .clone()
    }

    pub fn size(&self, i: usize) -> [f32; 2] {
        [self.list[i].image.w as f32, self.list[i].image.h as f32]
    }
}
