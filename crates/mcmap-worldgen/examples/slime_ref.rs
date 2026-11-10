//! Check slime chunks against the reference feature capture: every chunk whose centre falls in
//! the view must be green there exactly when it is a slime chunk.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let img = image::open(&a[1]).unwrap().to_rgba8();
    let (w, h) = (img.width() as f64, img.height() as f64);
    let (seed, cx, cz, ppb) = (-7556262131344217808i64, 2839.0, -1418.0, 1.25);
    let (mut ok, mut bad) = (0, 0);
    let x0 = ((cx - w / 2.0 / ppb) as i32 >> 4) - 1;
    let x1 = ((cx + w / 2.0 / ppb) as i32 >> 4) + 1;
    let z0 = ((cz - h / 2.0 / ppb) as i32 >> 4) - 1;
    let z1 = ((cz + h / 2.0 / ppb) as i32 >> 4) + 1;
    for z in z0..=z1 {
        for x in x0..=x1 {
            // Sample a few pixels inside the chunk, away from the edges and icons.
            let bx = (x * 16 + 8) as f64;
            let bz = (z * 16 + 8) as f64;
            let px = (bx - cx) * ppb + w / 2.0;
            let pz = (bz - cz) * ppb + h / 2.0;
            if px < 2.0 || pz < 2.0 || px >= w - 2.0 || pz >= h - 2.0 {
                continue;
            }
            let p = img.get_pixel(px as u32, pz as u32);
            if p[3] > 8 && !(p[1] > 120 && p[0] < 90 && p[2] < 90) {
                continue; // an icon covers it
            }
            let green = p[3] > 8;
            let slime = mcmap_worldgen::random::is_slime_chunk(seed, x, z);
            if green == slime { ok += 1 } else { bad += 1; println!("chunk {x},{z}: capture {green}, computed {slime}"); }
        }
    }
    println!("slime chunks: {ok} agree, {bad} disagree");
}
