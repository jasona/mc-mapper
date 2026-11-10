use crate::map::{view, Map, HOME};
use crate::sprites::Sprites;
use mcmap_tiles::atlas::Rgba;
use mcmap_tiles::pyramid::TileKey;
use rune::{Env, MagnifyEvent, Runtime, WheelEvent};

const SIZE: (f32, f32) = (800.0, 528.0);
use crate::map::BAR;

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
    let map = Map::new(Some(Sprites::from_atlas(&atlas())), None);
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

fn fix_at(x: f64, z: f64) -> mcmap_live::Fix {
    mcmap_live::Fix {
        world: Default::default(),
        dimension: Some(mcmap_data::Dimension::Overworld),
        dimension_key: "minecraft:overworld".into(),
        pos: [x, 64.0, z],
        yaw: 0.0,
        pitch: 0.0,
        taken_at: std::time::SystemTime::now(),
        source: mcmap_live::SourceKind::Replay,
    }
}

/// Run frames the way the app does: poll, then draw.
fn poll_frames(rt: &mut Runtime<Map>) {
    for _ in 0..10 {
        rt.update_if(|s, tx| s.take_fixes(tx));
        settle(rt);
    }
}

#[test]
fn the_player_marker_tracks_fixes_and_follow_moves_the_camera() {
    let mut rt = runtime(1.0);
    let (tx, rx) = std::sync::mpsc::channel();
    rt.state.fixes = Some(rx);
    tx.send(fix_at(2850.0, -1400.0)).unwrap();
    poll_frames(&mut rt);
    let m = rect(&rt, "player");
    assert!(near([m[0] + m[2] / 2.0, m[1] + m[3] / 2.0], expected(&rt, [2850.0, -1400.0])), "marker at {m:?}");

    tx.send(fix_at(2852.0, -1401.0)).unwrap();
    poll_frames(&mut rt);
    let m = rect(&rt, "player");
    assert!(near([m[0] + m[2] / 2.0, m[1] + m[3] / 2.0], expected(&rt, [2852.0, -1401.0])), "marker at {m:?}");

    // Following: the camera centres on the player, even after a long move.
    rt.state.follow = true;
    tx.send(fix_at(9000.0, 4000.0)).unwrap();
    poll_frames(&mut rt);
    assert_eq!(rt.state.camera.center, [9000.0, 4000.0]);
    let m = rect(&rt, "player");
    assert!(near([m[0] + m[2] / 2.0, m[1] + m[3] / 2.0], [SIZE.0 / 2.0, BAR + (SIZE.1 - BAR) / 2.0]), "marker at {m:?}");

    // Dragging the map stops following.
    rt.wheel_event(WheelEvent::new(-30.0, 0.0).at(400.0, 250.0).precise(true));
    settle(&mut rt);
    assert!(!rt.state.follow);
}

#[test]
fn queued_tiles_nobody_looks_at_are_dropped() {
    let store = crate::tiles::TileStore::new(None);
    // Ask for far more tiles than the workers can finish, then for only a few.
    store.begin_frame();
    for x in 0..400 {
        store.request(TileKey { level: -3, x, z: 0 }, x as f64);
    }
    store.end_frame();
    store.begin_frame();
    for x in 0..3 {
        store.request(TileKey { level: -3, x, z: 0 }, x as f64);
    }
    store.end_frame();
    assert!(store.queued() <= 3, "{} tiles still queued", store.queued());
}

#[test]
fn hover_reads_the_block_under_the_pointer() {
    let rt = runtime(1.0);
    let size = [SIZE.0 as f64, (SIZE.1 - BAR) as f64];
    let s = rt.state.hover_at([size[0] / 2.0, size[1] / 2.0]);
    assert_eq!(s, format!("{}, {}", HOME.center[0].floor() as i64, HOME.center[1].floor() as i64));
}

/// Every Overworld structure in the pack has a picker label (needs `MCMAP_PACK`).
#[test]
#[ignore]
fn every_overworld_structure_has_a_layer() {
    let pack = mcmap_data::pack::RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").unwrap())).unwrap();
    let e = crate::markers::Engine::new(&pack, 1, mcmap_data::Dimension::Overworld).unwrap();
    let mut missing = Vec::new();
    for set in &e.structures.sets {
        for (s, _) in &set.entries {
            // Nether and End structures have no Overworld biomes.
            let other = matches!(s.kind, mcmap_worldgen::structure::Kind::Other(_)) || s.biomes.is_empty();
            if !other && crate::markers::kind_of(&s.id).is_none() {
                missing.push(s.id.clone());
            }
        }
    }
    assert!(missing.is_empty(), "no layer for {missing:?}");
}

#[test]
fn nearest_search_finds_a_village_and_selects_it() {
    let Ok(path) = std::env::var("MCMAP_PACK") else { return };
    let pack = std::sync::Arc::new(mcmap_data::pack::RawPack::read(std::path::Path::new(&path)).unwrap());
    let mut map = Map::new(None, None);
    map.markers.sync = true;
    map.markers.set_engine(Some(std::sync::Arc::new(crate::markers::Engine::new(&pack, -7556262131344217808, mcmap_data::Dimension::Overworld).unwrap())));
    map.nearest("Village");
    assert!(map.take_marker_updates());
    let sel = map.selected.clone().expect("a village");
    assert!(sel.name.starts_with("Village"), "{sel:?}");
    assert_eq!(map.camera.center, [sel.x as f64 + 0.5, sel.z as f64 + 0.5]);
    assert!(sel.tp().starts_with("/tp @s "));
}

fn centre(r: [f32; 4]) -> [f32; 2] {
    [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0]
}

#[test]
fn the_heading_dot_points_where_the_player_looks() {
    let mut rt = runtime(1.0);
    let (tx, rx) = std::sync::mpsc::channel();
    rt.state.fixes = Some(rx);
    // Yaw 0 faces +z: down the screen.
    tx.send(fix_at(2850.0, -1400.0)).unwrap();
    poll_frames(&mut rt);
    let (m, h) = (centre(rect(&rt, "player")), centre(rect(&rt, "heading")));
    assert!((h[0] - m[0]).abs() < 0.5 && h[1] > m[1] + 8.0, "player {m:?} heading {h:?}");
    // Yaw 90 faces -x: left.
    let mut f = fix_at(2850.0, -1400.0);
    f.yaw = 90.0;
    tx.send(f).unwrap();
    poll_frames(&mut rt);
    let (m, h) = (centre(rect(&rt, "player")), centre(rect(&rt, "heading")));
    assert!(h[0] < m[0] - 8.0 && (h[1] - m[1]).abs() < 0.5, "player {m:?} heading {h:?}");
}

#[test]
fn an_old_fix_dims_the_marker() {
    let mut rt = runtime(1.0);
    let mut f = fix_at(2850.0, -1400.0);
    f.taken_at = std::time::SystemTime::now() - std::time::Duration::from_secs(30);
    rt.update_if(|s, tx| s.apply_fix(f.clone(), tx));
    settle(&mut rt);
    rt.update_if(|s, tx| s.take_live_events(tx));
    settle(&mut rt);
    assert!(rt.state.stale);
    // A fresh fix wakes it.
    rt.update_if(|s, tx| s.apply_fix(fix_at(2851.0, -1400.0), tx));
    settle(&mut rt);
    assert!(!rt.state.stale);
}

fn agent_fix(zoom: i64) -> mcmap_live::Fix {
    let mut f = fix_at(100.0, 200.0);
    f.source = mcmap_live::SourceKind::Agent;
    f.world = mcmap_live::WorldHint { server: Some("Test Realm".into()), realm: true, zoom_seed: Some(zoom), seed: None };
    f
}

#[test]
fn a_typed_seed_is_remembered_only_when_it_matches_the_agents_world() {
    use mcmap_data::seeds::{zoom_seed, SeedStore};
    let dir = std::env::temp_dir().join(format!("mcmap-seeds-{}", std::process::id()));
    let mut rt = runtime(1.0);
    rt.state.seeds = SeedStore::load(dir.join("worlds.json"));
    rt.update_if(|s, tx| s.apply_fix(agent_fix(zoom_seed(12345)), tx));
    settle(&mut rt);
    assert_eq!(rt.state.pending_zoom, Some(zoom_seed(12345)));
    assert!(rt.state.status.contains("Test Realm"), "{}", rt.state.status);

    // Another seed: shown, not kept.
    rt.update(|s, _| {
        s.seed_field = rune::TextEdit::new("999");
        s.submit_seed();
    });
    assert!(rt.state.seeds.by_zoom(zoom_seed(999)).is_none());
    // The right one: kept, under the Realm's name, and saved.
    rt.update(|s, _| {
        s.seed_field = rune::TextEdit::new("12345");
        s.submit_seed();
    });
    let w = rt.state.seeds.by_zoom(zoom_seed(12345)).expect("remembered");
    assert_eq!((w.seed, w.name.as_deref()), (12345, Some("Test Realm")));
    assert_eq!(SeedStore::load(dir.join("worlds.json")).worlds.len(), 1);
    assert_eq!(rt.state.pending_zoom, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn f3c_text_becomes_a_fix_but_never_overrides_a_fresh_agent_fix() {
    let mut rt = runtime(1.0);
    let line = "/execute in minecraft:overworld run tp @s 2839.50 71.00 -1418.23 -90.15 12.30";
    rt.update_if(|s, tx| s.clipboard_text(line, tx));
    settle(&mut rt);
    let p = rt.state.player.clone().expect("fix");
    assert_eq!((p.pos, p.source), ([2839.5, 71.0, -1418.23], mcmap_live::SourceKind::Clipboard));

    rt.update_if(|s, tx| s.apply_fix(agent_fix(1), tx));
    settle(&mut rt);
    rt.update_if(|s, tx| s.clipboard_text(line, tx));
    settle(&mut rt);
    assert_eq!(rt.state.player.as_ref().unwrap().source, mcmap_live::SourceKind::Agent);
}

/// An agent fix from a world the store knows switches the map to its seed and dimension
/// (needs `MCMAP_PACK`).
#[test]
#[ignore]
fn a_known_world_loads_its_seed_and_follows_the_dimension() {
    use mcmap_data::seeds::{zoom_seed, SeedStore};
    let pack = std::sync::Arc::new(mcmap_data::pack::RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").unwrap())).unwrap());
    let dir = std::env::temp_dir().join(format!("mcmap-known-{}", std::process::id()));
    let mut rt = runtime(1.0);
    rt.state.pack = Some(pack);
    rt.state.seeds = SeedStore::load(dir.join("worlds.json"));
    rt.state.seeds.remember(12345, Some("Test Realm".into()), None);
    let mut f = agent_fix(zoom_seed(12345));
    f.dimension = Some(mcmap_data::Dimension::Nether);
    rt.update_if(|s, tx| s.apply_fix(f.clone(), tx));
    settle(&mut rt);
    assert_eq!(rt.state.tiles.world().map(|w| w.seed), Some(12345));
    assert_eq!(rt.state.dimension, mcmap_data::Dimension::Nether);
    assert_eq!(rt.state.camera.center, [100.0, 200.0]);
    assert!(rt.state.follow);
    rect(&rt, "player");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The live wiring against the stand-in game: the session scan finds it and its launcher world,
/// attaches the agent with `jcmd`, and agent fixes arrive. Needs `MCMAP_FAKEGAME` (the compiled
/// `java/agent/test/fakegame` classes) and `MCMAP_AGENT_JAR`; run it alone (`--test-threads 1`),
/// since it binds the agent port.
#[test]
#[ignore]
fn live_finds_the_game_attaches_and_receives_fixes() {
    let classes = std::env::var("MCMAP_FAKEGAME").expect("MCMAP_FAKEGAME");
    let dir = std::env::temp_dir().join(format!("mcmap-fake-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("launcher_quick_play.json"),
        r#"{"version":2,"quickPlayData":{"acct":[{"epochLastPlayedTimeMs":5,"id":"realm-1","name":"Test Realm","javaInstance":{"game":{"type":"realms"}}}]}}"#,
    )
    .unwrap();
    let mut game = std::process::Command::new("java")
        .args(["-cp", &classes, "net.minecraft.client.main.Main", "--version", "26.3", "--gameDir", dir.to_str().unwrap()])
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let live = crate::live::start(std::sync::Arc::new(|| {}), tx, None);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let (mut session, mut notes, mut fix) = (None, Vec::new(), None);
    while std::time::Instant::now() < end && (session.is_none() || fix.is_none()) {
        while let Ok(e) = live.events.try_recv() {
            match e {
                crate::live::Event::Session(Some(s)) => session = Some(s),
                crate::live::Event::Note(n) => notes.push(n),
                _ => {}
            }
        }
        if let Ok(f) = rx.try_recv() {
            fix = Some(f);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = game.kill();
    let _ = std::fs::remove_dir_all(&dir);
    println!("notes: {notes:?}");
    let s = session.expect("session");
    assert_eq!(s.quick_play.as_ref().map(|q| q.key()).as_deref(), Some("realms:realm-1"));
    let f: mcmap_live::Fix = fix.unwrap_or_else(|| panic!("no fix; notes {notes:?}"));
    println!("fix: {f:?}");
    assert_eq!(f.source, mcmap_live::SourceKind::Agent);
    assert_eq!(f.world.server.as_deref(), Some("Test Realm"));
}

/// The preset button cycles Default, Large Biomes, Amplified, and Large Biomes changes the
/// biomes (needs `MCMAP_PACK`).
#[test]
#[ignore]
fn the_preset_button_switches_the_world_type() {
    let pack = std::sync::Arc::new(mcmap_data::pack::RawPack::read(std::path::Path::new(&std::env::var("MCMAP_PACK").unwrap())).unwrap());
    let mut rt = runtime(1.0);
    rt.state.pack = Some(pack);
    rt.update(|s, _| {
        s.seed_field = rune::TextEdit::new("12345");
        s.submit_seed();
    });
    settle(&mut rt);
    let at = |rt: &Runtime<Map>| {
        let w = rt.state.tiles.world().unwrap();
        (0..50).map(|i| w.surface.biome_with_hint(i * 64, i * 40, &mut None).0).collect::<Vec<_>>()
    };
    let normal = at(&rt);
    let r = rect(&rt, "preset");
    rt.pointer_down(r[0] + 4.0, r[1] + 4.0);
    rt.pointer_up(r[0] + 4.0, r[1] + 4.0);
    settle(&mut rt);
    assert_eq!(rt.state.preset, mcmap_data::Preset::LargeBiomes);
    assert_ne!(at(&rt), normal);
    rt.pointer_down(r[0] + 4.0, r[1] + 4.0);
    rt.pointer_up(r[0] + 4.0, r[1] + 4.0);
    settle(&mut rt);
    assert_eq!(rt.state.preset, mcmap_data::Preset::Amplified);
    // Amplified changes terrain, not surface biomes.
    assert_eq!(at(&rt), normal);
}

#[test]
fn nearest_search_finds_decoration_features() {
    let Ok(path) = std::env::var("MCMAP_PACK") else { return };
    let pack = std::sync::Arc::new(mcmap_data::pack::RawPack::read(std::path::Path::new(&path)).unwrap());
    let mut map = Map::new(None, None);
    map.markers.sync = true;
    map.markers.set_engine(Some(std::sync::Arc::new(crate::markers::Engine::new(&pack, 99, mcmap_data::Dimension::Overworld).unwrap())));
    for (label, name) in [("Amethyst Geode", "Amethyst Geode"), ("Fossil", "Fossil"), ("Desert Well", "Desert Well"), ("Lava Pool", "Lava Pool")] {
        map.nearest(label);
        assert!(map.take_marker_updates());
        let sel = map.selected.clone().unwrap_or_else(|| panic!("no {label}: {}", map.status));
        assert!(sel.name.starts_with(name) && sel.likely, "{sel:?}");
    }
}

#[test]
fn variant_sprites_and_display_names_distinguish_subvariants() {
    use crate::markers::{display_name_of, kind_of, sprite_for};
    use mcmap_worldgen::structure::Found;

    let mk = |structure: &str, variant: Option<&str>| Found {
        structure: format!("minecraft:{structure}"),
        chunk: (0, 0),
        pos: (8, 64, 8),
        marker: (8, 8),
        likely: false,
        variant: variant.map(str::to_owned),
    };
    let check = |structure: &str, variant: Option<&str>, want_sprite: usize, want_name: &str| {
        let f = mk(structure, variant);
        let k = kind_of(&f.structure).unwrap();
        assert_eq!(sprite_for(k, &f), want_sprite, "{structure} {variant:?}");
        assert_eq!(display_name_of(&f), want_name, "{structure} {variant:?}");
    };

    // Bastion Remnant never uses Raw Copper Ore (67); generic is 7 and the 4 types are 3..=6.
    check("bastion_remnant", None, 7, "Bastion Remnant");
    check("bastion_remnant", Some("bridge"), 3, "Bastion Remnant (bridge)");
    check("bastion_remnant", Some("hoglin_stable"), 4, "Bastion Remnant (hoglin stables)");
    check("bastion_remnant", Some("treasure"), 5, "Bastion Remnant (treasure room)");
    check("bastion_remnant", Some("units"), 6, "Bastion Remnant (housing units)");

    // End City with Elytra ship vs plain tower.
    check("end_city", None, 23, "End City");
    check("end_city", Some("ship"), 22, "End City (with ship · Elytra)");

    // Igloo, Ocean Ruins, Ruined Portals, Zombie Villages.
    check("igloo", None, 27, "Igloo");
    check("igloo", Some("basement"), 26, "Igloo (with basement)");
    check("ocean_ruin_warm", None, 34, "Ocean Ruins (warm)");
    check("ocean_ruin_warm", Some("large"), 33, "Ocean Ruins (warm, large)");
    check("ruined_portal_jungle", None, 41, "Ruined Portal (jungle)");
    check("ruined_portal_jungle", Some("giant"), 40, "Ruined Portal (jungle, giant)");
    check("ruined_portal_nether", Some("giant"), 40, "Ruined Portal Nether (giant)");
    check("village_plains", None, 50, "Village (plains)");
    check("village_desert", None, 45, "Village (desert)");
    check("village_plains", Some("zombie"), 49, "Village (plains, zombie)");
}

#[test]
fn native_nether_and_end_picker_icons_load_from_feature_atlas() {
    let art = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../art/seed-map");
    let mut features = crate::features::Features::load(&art).expect("load feature atlas");
    let count_before = features.list.len();
    for (dim, label) in [
        (mcmap_data::Dimension::Nether, "Nether Fortress"),
        (mcmap_data::Dimension::Nether, "Bastion Remnant"),
        (mcmap_data::Dimension::Nether, "Ruined Portal Nether"),
        (mcmap_data::Dimension::End, "End City"),
    ] {
        assert!(features.list.iter().any(|f| f.label == label && f.dimension == Some(dim)), "missing native {label}");
        // Fallback push is a no-op because the native 22x22 icon is already present.
        features.push(label, &atlas(), dim);
    }
    assert_eq!(features.list.len(), count_before);
}

#[test]
fn zoom_standin_synthesizes_from_parent_and_child_tiles() {
    let Ok(path) = std::env::var("MCMAP_PACK") else { return };
    let pack = mcmap_data::pack::RawPack::read(std::path::Path::new(&path)).unwrap();
    let surface = mcmap_worldgen::Surface::for_dimension(&pack, 12345, mcmap_data::Dimension::Overworld).unwrap();
    let world = crate::tiles::World::new(12345, surface);

    let parent = TileKey { level: 0, x: 0, z: 0 };
    let child_br = TileKey { level: -1, x: 1, z: 1 };
    assert!(world.standin_rgba(child_br, false).is_none());

    // Warming the parent lets zooming in synthesize the bottom-right child immediately.
    world.warm_tile(parent);
    let parent_px = world.standin_rgba(TileKey { level: -1, x: 0, z: 0 }, false).expect("top-left child stand-in");
    let br_px = world.standin_rgba(child_br, false).expect("bottom-right child stand-in");
    assert_eq!(parent_px.len(), 256 * 256 * 4);
    assert_eq!(br_px.len(), 256 * 256 * 4);

    // Warming children of an uncached outer tile lets zooming out synthesize a 2x2 downsample.
    let outer = TileKey { level: 2, x: 10, z: -7 };
    assert!(world.standin_rgba(outer, false).is_none());
    for dz in 0..2 {
        for dx in 0..2 {
            world.warm_tile(TileKey { level: 1, x: outer.x * 2 + dx, z: outer.z * 2 + dz });
        }
    }
    let down = world.standin_rgba(outer, false).expect("downsampled parent stand-in");
    assert_eq!(down.len(), 256 * 256 * 4);
}

