//! The window's textures: the page texture behind the toolbar and the content texture behind
//! the panels. Rune draws an image stretched to its element, with no repeat mode, so each
//! texture is tiled into an image of the element's exact size (from its last layout), at
//! device resolution with nearest-neighbour texels.

use mcmap_tiles::atlas::Rgba;
use rune::Pixels;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texture {
    Page,
    Content,
}

pub struct Chrome {
    page: Rgba,
    content: Rgba,
    made: RefCell<HashMap<(Texture, u32, u32), Pixels>>,
}

impl Chrome {
    pub fn load(art: &Path) -> Result<Chrome, String> {
        let read = |f: &str| {
            let p = art.join("ui").join(f);
            Rgba::decode(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?)
        };
        Ok(Chrome { page: read("page-texture.png")?, content: read("content-texture.png")?, made: RefCell::new(HashMap::new()) })
    }

    /// `t` tiled over `size` points at `scale` device pixels per point. Page texels are 2
    /// points, content texels 1.
    pub fn pixels(&self, t: Texture, size: [f32; 2], scale: f32) -> Pixels {
        let (w, h) = ((size[0] * scale).round().max(1.0) as u32, (size[1] * scale).round().max(1.0) as u32);
        let mut made = self.made.borrow_mut();
        if made.len() > 16 {
            made.clear();
        }
        made.entry((t, w, h))
            .or_insert_with(|| {
                let (src, texel) = match t {
                    Texture::Page => (&self.page, 2.0 * scale),
                    Texture::Content => (&self.content, scale),
                };
                let img = tile(src, w, h, texel.max(1.0));
                let px = Pixels::new();
                px.set(img.w, img.h, img.px);
                px
            })
            .clone()
    }
}

/// Repeat `src` over w x h, each texel `texel` pixels wide.
pub fn tile(src: &Rgba, w: u32, h: u32, texel: f32) -> Rgba {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let sy = ((y as f32 / texel) as u32) % src.h;
        for x in 0..w {
            let sx = ((x as f32 / texel) as u32) % src.w;
            let i = ((sy * src.w + sx) * 4) as usize;
            px.extend_from_slice(&src.px[i..i + 4]);
        }
    }
    Rgba { w, h, px }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tiling_repeats_and_scales_texels() {
        let src = mcmap_tiles::atlas::Rgba { w: 2, h: 1, px: vec![1, 1, 1, 255, 2, 2, 2, 255] };
        let t = super::tile(&src, 6, 1, 2.0);
        let firsts: Vec<u8> = t.px.chunks(4).map(|c| c[0]).collect();
        assert_eq!(firsts, [1, 1, 2, 2, 1, 1]);
    }
}
