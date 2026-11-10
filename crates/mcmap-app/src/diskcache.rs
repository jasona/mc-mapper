//! Biome tiles on disk. A tile's biome ids depend only on the pack, seed, dimension, preset and
//! tile key, so a cached tile never goes stale; the cache only needs a size bound. Each tile is
//! zstd-compressed `u16` ids (little-endian, row by row), a few KiB.
//!
//! The root is `$MCMAP_CACHE_DIR`, else `$XDG_CACHE_HOME/mcmap`, else `~/.cache/mcmap`.
//! `MCMAP_DISK_CACHE=0` turns it off. At startup the oldest tiles (by last use) are deleted
//! until the cache is under [`MAX_BYTES`].

use mcmap_data::{Dimension, Preset};
use mcmap_tiles::TileKey;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Size bound of the whole cache.
pub const MAX_BYTES: u64 = 1 << 30;

const MAGIC: &[u8; 4] = b"MCT1";

pub fn root() -> Option<PathBuf> {
    if cfg!(test) || std::env::var_os("MCMAP_DISK_CACHE").is_some_and(|v| v == "0") {
        return None;
    }
    if let Some(d) = std::env::var_os("MCMAP_CACHE_DIR") {
        return Some(d.into());
    }
    if let Some(d) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(Path::new(&d).join("mcmap"));
    }
    std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache/mcmap"))
}

#[derive(Clone, Debug)]
pub struct TileCache {
    dir: PathBuf,
}

impl TileCache {
    pub fn at(dir: PathBuf) -> TileCache {
        TileCache { dir }
    }

    /// The cache of one world. `version` and `names` (the biome table the ids index) key the
    /// pack, so a pack with another table never reads these ids.
    pub fn for_world(version: &str, names: &[String], seed: i64, dimension: Dimension, preset: Preset) -> Option<TileCache> {
        let table = mcmap_data::seeds::sha256(names.join(",").as_bytes());
        let table: String = table[..4].iter().map(|b| format!("{b:02x}")).collect();
        let dim = dimension.key().trim_start_matches("minecraft:").to_owned();
        let preset = match preset {
            Preset::Normal => "default",
            Preset::LargeBiomes => "large_biomes",
            Preset::Amplified => "amplified",
        };
        Some(TileCache::at(root()?.join("tiles").join(format!("{version}-{table}")).join(dim).join(preset).join(seed.to_string())))
    }

    #[cfg(test)]
    fn path(&self, key: TileKey) -> PathBuf {
        self.path_of(key, "zst")
    }

    fn path_of(&self, key: TileKey, ext: &str) -> PathBuf {
        self.dir.join(key.level.to_string()).join(format!("{}_{}.{ext}", key.x, key.z))
    }

    /// The tile's ids, if cached and whole.
    pub fn get(&self, key: TileKey, len: usize) -> Option<Vec<u16>> {
        let raw = self.read(key, "zst", len * 2)?;
        Some(raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
    }

    /// Store a tile. Failures are ignored: the cache is only a speed-up.
    pub fn put(&self, key: TileKey, ids: &[u16]) {
        let raw: Vec<u8> = ids.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.write(key, "zst", &raw);
    }

    /// The tile's hillshade, if cached.
    pub fn get_shade(&self, key: TileKey, len: usize) -> Option<Vec<u8>> {
        self.read(key, "shade.zst", len)
    }

    pub fn put_shade(&self, key: TileKey, shade: &[u8]) {
        self.write_file(&self.path_of(key, "shade.zst"), shade);
    }

    /// World-level cached blob (e.g. spawn + stronghold rings).
    pub fn get_blob(&self, name: &str) -> Option<Vec<u8>> {
        self.read_file(&self.dir.join(format!("{name}.zst")), None)
    }

    pub fn put_blob(&self, name: &str, raw: &[u8]) {
        self.write_file(&self.dir.join(format!("{name}.zst")), raw);
    }

    fn read(&self, key: TileKey, ext: &str, len: usize) -> Option<Vec<u8>> {
        self.read_file(&self.path_of(key, ext), Some(len))
    }

    fn write(&self, key: TileKey, ext: &str, raw: &[u8]) {
        self.write_file(&self.path_of(key, ext), raw);
    }

    fn read_file(&self, p: &Path, expected_len: Option<usize>) -> Option<Vec<u8>> {
        let bytes = std::fs::read(p).ok()?;
        let raw = zstd::decode_all(bytes.strip_prefix(MAGIC)?).ok()?;
        if expected_len.is_some_and(|len| raw.len() != len) {
            return None;
        }
        // Mark it used, for the size bound.
        if let Ok(f) = std::fs::File::options().append(true).open(p) {
            let _ = f.set_modified(SystemTime::now());
        }
        Some(raw)
    }

    fn write_file(&self, p: &Path, raw: &[u8]) {
        let Some(dir) = p.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let Ok(z) = zstd::encode_all(raw, 3) else { return };
        let tmp = p.with_extension(format!("tmp{}", std::process::id()));
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&z);
        if std::fs::write(&tmp, out).is_ok() && std::fs::rename(&tmp, p).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Delete the least recently used tiles until the cache under `root` is at most `max` bytes.
/// Returns (files, bytes) deleted.
pub fn prune(root: &Path, max: u64) -> (usize, u64) {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                files.push((m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len(), e.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    let (mut n, mut freed) = (0, 0);
    for (_, len, p) in files {
        if total <= max {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
            freed += len;
            n += 1;
        }
    }
    (n, freed)
}

/// Prune the cache on a background thread.
pub fn prune_in_background() {
    if let Some(r) = root() {
        let _ = std::thread::Builder::new().name("mcmap-cache-prune".into()).spawn(move || {
            prune(&r.join("tiles"), MAX_BYTES);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_round_trip_and_prune_drops_the_oldest() {
        let dir = std::env::temp_dir().join(format!("mcmap-cache-test-{}", std::process::id()));
        let c = TileCache::at(dir.clone());
        let ids: Vec<u16> = (0..256 * 256).map(|i| (i / 300) as u16).collect();
        let k = |x| TileKey { level: 0, x, z: -3 };
        assert!(c.get(k(1), ids.len()).is_none());
        c.put(k(1), &ids);
        assert_eq!(c.get(k(1), ids.len()).as_deref(), Some(&ids[..]));
        // A tile of another size is not this one.
        assert!(c.get(k(1), 10).is_none());
        c.put(k(2), &ids);
        let old = SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().append(true).open(c.path(k(1))).unwrap().set_modified(old).unwrap();
        let one = std::fs::metadata(c.path(k(2))).unwrap().len();
        let (n, _) = prune(&dir, one);
        assert_eq!(n, 1);
        assert!(c.get(k(1), ids.len()).is_none());
        assert!(c.get(k(2), ids.len()).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
