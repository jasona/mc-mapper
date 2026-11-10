//! Sprite atlases. The marker atlas ships without a crop table, so sprites are found by
//! segmenting the image on alpha: each 4-connected group of visible pixels is a sprite, and
//! groups whose boxes come within `gap` pixels are merged (an outline drawn apart from its fill
//! stays one sprite). The marker atlas packs sprites one transparent pixel apart, so it is cut
//! with 4-connectivity and gap 0.
//!
//! Sprites are pixel art and Rune samples bilinearly, so they are enlarged by a whole factor with
//! nearest-neighbour before upload, the same rule the tiles follow.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// At most `gap` empty pixels between the boxes on both axes.
    fn touches(&self, o: &Rect, gap: u32) -> bool {
        !(self.x + self.w + gap < o.x || o.x + o.w + gap < self.x || self.y + self.h + gap < o.y || o.y + o.h + gap < self.y)
    }

    fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect { x, y, w: (self.x + self.w).max(o.x + o.w) - x, h: (self.y + self.h).max(o.y + o.h) - y }
    }
}

/// Alpha above which a marker-atlas pixel counts as part of a sprite (the atlas has a few
/// near-transparent stray pixels).
pub const MARKER_ALPHA: u8 = 8;

/// An RGBA image, straight alpha.
#[derive(Clone, Debug)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

impl Rgba {
    pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
        let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
        Ok(Rgba { w: img.width(), h: img.height(), px: img.into_raw() })
    }

    fn alpha(&self, x: u32, y: u32) -> u8 {
        self.px[((y * self.w + x) * 4 + 3) as usize]
    }

    pub fn crop(&self, r: Rect) -> Rgba {
        let mut px = Vec::with_capacity((r.w * r.h * 4) as usize);
        for y in r.y..r.y + r.h {
            let s = ((y * self.w + r.x) * 4) as usize;
            px.extend_from_slice(&self.px[s..s + (r.w * 4) as usize]);
        }
        Rgba { w: r.w, h: r.h, px }
    }

    /// Nearest-neighbour resample into a `w` x `h` box, keeping the aspect, centred on transparency.
    pub fn fit(&self, w: u32, h: u32) -> Rgba {
        let k = (w as f64 / self.w as f64).min(h as f64 / self.h as f64);
        let (sw, sh) = (((self.w as f64 * k).round() as u32).max(1), ((self.h as f64 * k).round() as u32).max(1));
        let (ox, oy) = ((w - sw.min(w)) / 2, (h - sh.min(h)) / 2);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..sh.min(h) {
            for x in 0..sw.min(w) {
                let sx = ((x as f64 + 0.5) / k) as u32;
                let sy = ((y as f64 + 0.5) / k) as u32;
                let s = ((sy.min(self.h - 1) * self.w + sx.min(self.w - 1)) * 4) as usize;
                let d = (((y + oy) * w + x + ox) * 4) as usize;
                px[d..d + 4].copy_from_slice(&self.px[s..s + 4]);
            }
        }
        Rgba { w, h, px }
    }

    /// Enlarge by a whole factor, nearest-neighbour.
    pub fn upscale(&self, k: u32) -> Rgba {
        let k = k.max(1);
        let (w, h) = (self.w * k, self.h * k);
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let s = (((y / k) * self.w + x / k) * 4) as usize;
                let d = ((y * w + x) * 4) as usize;
                px[d..d + 4].copy_from_slice(&self.px[s..s + 4]);
            }
        }
        Rgba { w, h, px }
    }
}

/// Find sprites: alpha above `threshold`, 4-connected, boxes within `gap` pixels merged.
/// Sorted top to bottom in rows, then left to right.
pub fn segment(img: &Rgba, threshold: u8, gap: u32) -> Vec<Rect> {
    let (w, h) = (img.w as usize, img.h as usize);
    let mut seen = vec![false; w * h];
    let mut boxes: Vec<Rect> = Vec::new();
    let mut stack = Vec::new();
    for y0 in 0..h {
        for x0 in 0..w {
            if seen[y0 * w + x0] || img.alpha(x0 as u32, y0 as u32) <= threshold {
                continue;
            }
            let (mut x1, mut y1, mut x2, mut y2) = (x0, y0, x0, y0);
            seen[y0 * w + x0] = true;
            stack.push((x0, y0));
            while let Some((x, y)) = stack.pop() {
                (x1, y1, x2, y2) = (x1.min(x), y1.min(y), x2.max(x), y2.max(y));
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let (nx, ny) = (nx as usize, ny as usize);
                        if !seen[ny * w + nx] && img.alpha(nx as u32, ny as u32) > threshold {
                            seen[ny * w + nx] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            boxes.push(Rect { x: x1 as u32, y: y1 as u32, w: (x2 - x1 + 1) as u32, h: (y2 - y1 + 1) as u32 });
        }
    }
    // Merge boxes that touch, until nothing changes.
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < boxes.len() {
            let mut j = i + 1;
            while j < boxes.len() {
                if boxes[i].touches(&boxes[j], gap) {
                    let b = boxes.swap_remove(j);
                    boxes[i] = boxes[i].union(&b);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    // Rows: a sprite joins the row of the first sprite whose vertical span its centre falls in.
    boxes.sort_by_key(|r| (r.y, r.x));
    let mut rows: Vec<Vec<Rect>> = Vec::new();
    for r in boxes {
        let cy = r.y + r.h / 2;
        match rows.iter_mut().find(|row| row.first().is_some_and(|f| cy >= f.y && cy < f.y + f.h)) {
            Some(row) => row.push(r),
            None => rows.push(vec![r]),
        }
    }
    rows.into_iter().flat_map(|mut row| {
        row.sort_by_key(|r| r.x);
        row
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: u32, h: u32, dots: &[(u32, u32)]) -> Rgba {
        let mut px = vec![0u8; (w * h * 4) as usize];
        for &(x, y) in dots {
            px[((y * w + x) * 4 + 3) as usize] = 255;
        }
        Rgba { w, h, px }
    }

    #[test]
    fn segments_merge_touching_pieces_and_sort_in_rows() {
        // Two blobs in the top row (one made of two pieces a pixel apart), one below.
        // Two blobs in the top row (one made of a diagonal pair and a piece a pixel apart), one below.
        let img = canvas(20, 20, &[(1, 1), (2, 2), (4, 2), (10, 1), (11, 1), (3, 12), (3, 13)]);
        let r = segment(&img, 0, 1);
        assert_eq!(r, vec![
            Rect { x: 1, y: 1, w: 4, h: 2 },
            Rect { x: 10, y: 1, w: 2, h: 1 },
            Rect { x: 3, y: 12, w: 1, h: 2 },
        ]);
    }

    #[test]
    fn sprites_one_pixel_apart_stay_apart_at_gap_zero() {
        let img = canvas(10, 3, &[(1, 1), (2, 1), (4, 1), (5, 1)]);
        assert_eq!(segment(&img, 0, 0).len(), 2);
        assert_eq!(segment(&img, 0, 1).len(), 1);
    }

    /// `MCMAP_ART=art/seed-map cargo test -p mcmap-tiles -- --ignored` on a machine with the art.
    #[test]
    #[ignore]
    fn the_marker_atlas_cuts_into_sprites() {
        let dir = std::env::var("MCMAP_ART").expect("MCMAP_ART");
        let img = Rgba::decode(&std::fs::read(format!("{dir}/map/marker-atlas.webp")).unwrap()).unwrap();
        let r = segment(&img, MARKER_ALPHA, 0);
        let sizes: std::collections::BTreeMap<(u32, u32), usize> = r.iter().fold(Default::default(), |mut m, r| { *m.entry((r.w, r.h)).or_default() += 1; m });
        println!("{} sprites, sizes {sizes:?}", r.len());
        assert!(r.len() >= 60 && r.iter().all(|r| r.w <= 40 && r.h <= 40), "{r:?}");
    }

    #[test]
    fn upscale_is_nearest_neighbour() {
        let img = Rgba { w: 2, h: 1, px: vec![255, 0, 0, 255, 0, 0, 255, 128] };
        let up = img.upscale(3);
        assert_eq!((up.w, up.h), (6, 3));
        assert_eq!(&up.px[0..4], &[255, 0, 0, 255]);
        assert_eq!(&up.px[(2 * 6 + 5) * 4..(2 * 6 + 5) * 4 + 4], &[0, 0, 255, 128]);
    }
}
