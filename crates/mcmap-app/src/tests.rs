use crate::map::{view, Map, HOME};
use crate::sprites::Sprites;
use mcmap_tiles::atlas::Rgba;
use mcmap_tiles::pyramid::TileKey;
use rune::{Env, MagnifyEvent, Runtime, WheelEvent};

const SIZE: (f32, f32) = (800.0, 528.0);
const BAR: f32 = 28.0;

fn atlas() -> Rgba {
    // Two opaque 4x3 sprites side by side on a transparent 12x5 atlas.
    let (w, h) = (12u32, 5u32);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 1..4 {
        for x in (1..5).chain(7..11) {
            px[((y * w + x) * 4) as usize..][..4].copy_from_slice(&[200, 50, 50, 255]);
        }
    }
    Rgba { w, h, px }
}

fn runtime(scale: f32) -> Runtime<Map> {
    let map = Map::new(Some(Sprites::from_atlas(&atlas())));
    map.tiles.sync.set(true);
    let mut rt = Runtime::new(map, view, SIZE, Env::new(true, scale));
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<Map>) {
    for _ in 0..600 {
        if !rt.frame(1.0 / 60.0) && !rt.has_pending_input() {
            return;
        }
    }
    panic!("never settled");
}

fn rect(rt: &Runtime<Map>, key: &str) -> [f32; 4] {
    rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("no element {key}")).rect
}

/// Where the camera says a block shows, in window points.
fn expected(rt: &Runtime<Map>, block: [f64; 2]) -> [f32; 2] {
    let p = rt.state.camera.point_of(block, [SIZE.0 as f64, (SIZE.1 - BAR) as f64]);
    [p[0] as f32, p[1] as f32 + BAR]
}

fn tile_key(t: TileKey) -> String {
    format!("tile/{}/{}/{}", t.level, t.x, t.z)
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.6 && (a[1] - b[1]).abs() < 0.6
}

#[test]
fn tiles_land_where_the_camera_says_at_1x_and_2x() {
    for scale in [1.0, 2.0] {
        let rt = runtime(scale);
        let t = TileKey::containing(HOME.level(scale as f64), HOME.center);
        let r = rect(&rt, &tile_key(t));
        assert!(near([r[0], r[1]], expected(&rt, t.origin())), "scale {scale}: tile at {r:?}, camera says {:?}", expected(&rt, t.origin()));
        // A tile shows at k in (0.5, 1] device pixels per tile pixel.
        let k = r[2] * scale / 256.0;
        assert!(k > 0.5 && k <= 1.0, "scale {scale}: k {k}");
    }
}

#[test]
fn panning_inside_the_tile_set_skips_the_view() {
    let mut rt = runtime(1.0);
    let t = TileKey::containing(HOME.level(1.0), HOME.center);
    let before = rect(&rt, &tile_key(t));
    let skipped = rt.live_only_events();
    rt.wheel_event(WheelEvent::new(-3.0, -2.0).at(400.0, 250.0).precise(true));
    settle(&mut rt);
    let after = rect(&rt, &tile_key(t));
    assert!(near([after[0], after[1]], [before[0] - 3.0, before[1] - 2.0]), "{before:?} -> {after:?}");
    assert_eq!(rt.live_only_events(), skipped + 1, "a small pan moves only live values");
}

#[test]
fn panning_across_tiles_and_zooming_keep_blocks_in_place() {
    let mut rt = runtime(1.0);
    // A long pan crosses tile boundaries, so the view re-runs with a new origin.
    for _ in 0..20 {
        rt.wheel_event(WheelEvent::new(-60.0, 25.0).at(400.0, 250.0).precise(true));
        settle(&mut rt);
    }
    let t = TileKey::containing(rt.state.camera.level(1.0), rt.state.camera.center);
    let r = rect(&rt, &tile_key(t));
    assert!(near([r[0], r[1]], expected(&rt, t.origin())), "after pan: {r:?} vs {:?}", expected(&rt, t.origin()));

    // Pinch about a point: the block under it stays under it, through a level change.
    let p = [300.0f32, 200.0f32];
    let anchor = rt.state.camera.block_at([p[0] as f64, p[1] as f64], [SIZE.0 as f64, (SIZE.1 - BAR) as f64]);
    let level = rt.state.camera.level(1.0);
    for _ in 0..6 {
        rt.magnify_event(MagnifyEvent::new(-0.2).at(p[0], p[1] + BAR));
        settle(&mut rt);
    }
    assert_ne!(rt.state.camera.level(1.0), level, "zoomed out past a level");
    let back = expected(&rt, anchor);
    assert!(near(back, [p[0], p[1] + BAR]), "anchor drifted to {back:?}");
    let t = TileKey::containing(rt.state.camera.level(1.0), anchor);
    let r = rect(&rt, &tile_key(t));
    assert!(near([r[0], r[1]], expected(&rt, t.origin())), "after zoom: {r:?} vs {:?}", expected(&rt, t.origin()));
}

#[test]
fn markers_keep_their_screen_size_and_follow_the_camera() {
    for scale in [1.0, 2.0] {
        let mut rt = runtime(scale);
        for step in 0..3 {
            let m1 = rect(&rt, "marker/1");
            let b1 = rt.state.sprites.as_ref().unwrap().markers()[1].block;
            assert!(near([m1[0] + m1[2] / 2.0, m1[1] + m1[3] / 2.0], expected(&rt, b1)), "scale {scale} step {step}: marker 1 at {m1:?}, camera says {:?}", expected(&rt, b1));
            let m = rect(&rt, "marker/0");
            assert!((m[2] - 4.0).abs() < 0.05 && (m[3] - 3.0).abs() < 0.05, "scale {scale} step {step}: marker size {m:?}");
            let block = rt.state.sprites.as_ref().unwrap().markers()[0].block;
            let centre = [m[0] + m[2] / 2.0, m[1] + m[3] / 2.0];
            assert!(near(centre, expected(&rt, block)), "scale {scale} step {step}: marker at {centre:?}, camera says {:?}", expected(&rt, block));
            rt.magnify_event(MagnifyEvent::new(-0.15).at(200.0, 200.0));
            settle(&mut rt);
        }
    }
}
