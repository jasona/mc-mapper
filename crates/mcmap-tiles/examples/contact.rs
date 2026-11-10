//! Print the marker atlas segments (index, x, y, w, h), for building the crop table.
use mcmap_tiles::atlas::{segment, Rgba, MARKER_ALPHA};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let atlas = Rgba::decode(&std::fs::read(&a[1]).unwrap()).unwrap();
    for (i, r) in segment(&atlas, MARKER_ALPHA, 0).iter().enumerate() {
        println!("{i} {} {} {} {}", r.x, r.y, r.w, r.h);
    }
}
