//! Version packs: the world-generation data of one game version, taken from the user's own client
//! jar by `cargo xtask extract`. Packs are Mojang's data, so they live in the user data directory
//! and are never committed.
//!
//! M0 stores the raw JSON plus the Overworld biome table the jar builds in code (dumped by the
//! oracle harness). M1 compiles this into the interpreter's own form.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const FORMAT: u32 = 1;

/// Pack entry with the start-piece and ruined-portal template geometry the oracle dumps
/// (`templates` command). Older packs lack it; structure positions then fall back to estimates.
pub const TEMPLATES: &str = "mcmap/templates.json";

/// Pack entry with the Nether's multi-noise parameter table (built in code, dumped by the oracle).
pub const NETHER_PARAMETERS: &str = "mcmap/nether_parameters.json";

/// The generator's per-step decoration feature order of a dimension (`oracle featorder`), which
/// sets each feature's random seed.
pub fn feature_order(dimension: crate::Dimension) -> String {
    format!("mcmap/feature_order_{}.json", dimension.key().trim_start_matches("minecraft:"))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RawPack {
    pub format: u32,
    pub version: String,
    /// Jar path (`data/minecraft/worldgen/...`, `data/minecraft/tags/worldgen/...`) to JSON text.
    pub files: BTreeMap<String, String>,
    /// The Overworld multi-noise parameter list, as the oracle's `params` command writes it.
    pub overworld_parameters: Option<String>,
}

/// What a pack holds, for `xtask extract` to print and for tests to check.
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub biomes: usize,
    pub structures: usize,
    pub structure_sets: usize,
    pub noises: usize,
    pub density_functions: usize,
    /// Every density-function `type` used anywhere in the pack, without the namespace.
    pub density_function_types: BTreeSet<String>,
    pub overworld_parameter_entries: Option<usize>,
}

impl RawPack {
    pub fn summary(&self) -> Summary {
        let count = |dir: &str| self.files.keys().filter(|k| k.starts_with(&format!("data/minecraft/worldgen/{dir}/"))).count();
        let mut types = BTreeSet::new();
        for (path, text) in &self.files {
            if (path.contains("/worldgen/density_function/") || path.contains("/worldgen/noise_settings/"))
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
                    collect_types(&v, &mut types);
                }
        }
        let overworld_parameter_entries = self
            .overworld_parameters
            .as_deref()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|v| v["entries"].as_array().map(Vec::len));
        Summary {
            biomes: count("biome"),
            structures: count("structure"),
            structure_sets: count("structure_set"),
            noises: count("noise"),
            density_functions: count("density_function"),
            density_function_types: types,
            overworld_parameter_entries,
        }
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = postcard::to_stdvec(self).map_err(|e| e.to_string())?;
        let packed = zstd::encode_all(&bytes[..], 19).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, packed).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn read(path: &Path) -> Result<RawPack, String> {
        let packed = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = zstd::decode_all(&packed[..]).map_err(|e| e.to_string())?;
        let pack: RawPack = postcard::from_bytes(&bytes).map_err(|e| e.to_string())?;
        if pack.format != FORMAT {
            return Err(format!("{}: pack format {} (this build reads {FORMAT}); run `cargo run -p xtask -- extract` again", path.display(), pack.format));
        }
        Ok(pack)
    }
}

/// Should this jar entry go into a pack?
pub fn wanted(path: &str) -> bool {
    path.ends_with(".json")
        && (path.starts_with("data/minecraft/worldgen/")
            || path.starts_with("data/minecraft/tags/worldgen/")
            || path.starts_with("data/minecraft/dimension_type/"))
}

/// Where packs live: `$MCMAP_DATA_DIR`, else `$XDG_DATA_HOME/mcmap`, else `~/.local/share/mcmap`.
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("MCMAP_DATA_DIR") {
        return d.into();
    }
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return Path::new(&d).join("mcmap");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    Path::new(&home).join(".local/share/mcmap")
}

pub fn pack_path(version: &str) -> PathBuf {
    data_dir().join("packs").join(format!("{version}.mcpack"))
}

fn collect_types(v: &serde_json::Value, out: &mut BTreeSet<String>) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(t) = m.get("type").and_then(|t| t.as_str()) {
                out.insert(t.trim_start_matches("minecraft:").to_owned());
            }
            m.values().for_each(|x| collect_types(x, out));
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_types(x, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_round_trips_and_summarizes() {
        let mut files = BTreeMap::new();
        files.insert("data/minecraft/worldgen/biome/plains.json".into(), "{}".into());
        files.insert(
            "data/minecraft/worldgen/density_function/overworld/depth.json".into(),
            r#"{"type":"minecraft:add","argument1":{"type":"minecraft:gradient"},"argument2":"minecraft:overworld/offset"}"#.into(),
        );
        let pack = RawPack { format: FORMAT, version: "26.3".into(), files, overworld_parameters: Some(r#"{"entries":[{},{}]}"#.into()) };
        let dir = std::env::temp_dir().join(format!("mcmap-pack-test-{}", std::process::id()));
        let path = dir.join("t.mcpack");
        pack.write(&path).unwrap();
        let back = RawPack::read(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let s = back.summary();
        assert_eq!((s.biomes, s.density_functions, s.overworld_parameter_entries), (1, 1, Some(2)));
        assert_eq!(s.density_function_types.into_iter().collect::<Vec<_>>(), ["add", "gradient"]);
    }

    #[test]
    fn only_worldgen_json_is_wanted() {
        assert!(wanted("data/minecraft/worldgen/noise/temperature.json"));
        assert!(wanted("data/minecraft/tags/worldgen/biome/is_ocean.json"));
        assert!(!wanted("data/minecraft/loot_table/chests/x.json"));
        assert!(!wanted("assets/minecraft/textures/block/stone.png"));
    }
}
