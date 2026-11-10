//! Seeds the map has learned, per world.
//!
//! A Realm never sends its seed, but the client holds `BiomeManager.obfuscateSeed(seed)`:
//! `Hashing.sha256().hashLong(seed).asLong()`, the first 8 bytes (little-endian) of SHA-256 over
//! the seed's 8 little-endian bytes. The agent reports that value, so the map can check a typed
//! seed and recognise the world next time. Before the agent, worlds are keyed by the launcher's
//! quick-play record (Realm or server id).
//!
//! Stored as JSON at `<data dir>/worlds.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `BiomeManager.obfuscateSeed`.
pub fn zoom_seed(seed: i64) -> i64 {
    let d = sha256(&seed.to_le_bytes());
    i64::from_le_bytes(d[..8].try_into().unwrap())
}

/// SHA-256 (FIPS 180-4). Small inputs only; no streaming.
pub fn sha256(msg: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut data = msg.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&((msg.len() as u64) * 8).to_be_bytes());
    for block in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct KnownWorld {
    pub seed: i64,
    /// `zoom_seed(seed)`, kept so a lookup by the agent's value needs no hashing.
    pub zoom: i64,
    /// Display name: Realm or server name.
    #[serde(default)]
    pub name: Option<String>,
    /// The launcher's quick-play id for the world, when known.
    #[serde(default)]
    pub quick_play: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SeedStore {
    pub worlds: Vec<KnownWorld>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl SeedStore {
    pub fn default_path() -> PathBuf {
        crate::pack::data_dir().join("worlds.json")
    }

    /// Load from `path`; a missing or unreadable file is an empty store.
    pub fn load(path: PathBuf) -> SeedStore {
        let mut s: SeedStore = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        s.path = Some(path);
        s
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn by_zoom(&self, zoom: i64) -> Option<&KnownWorld> {
        self.worlds.iter().find(|w| w.zoom == zoom)
    }

    pub fn by_quick_play(&self, id: &str) -> Option<&KnownWorld> {
        self.worlds.iter().rev().find(|w| w.quick_play.as_deref() == Some(id))
    }

    /// Remember `seed`, merging with an entry for the same seed. Returns true when it changed.
    pub fn remember(&mut self, seed: i64, name: Option<String>, quick_play: Option<String>) -> bool {
        let zoom = zoom_seed(seed);
        if let Some(w) = self.worlds.iter_mut().find(|w| w.seed == seed) {
            let before = w.clone();
            if name.is_some() {
                w.name = name;
            }
            if quick_play.is_some() {
                w.quick_play = quick_play;
            }
            return *w != before;
        }
        // A quick-play id belongs to one world: a new seed for it replaces the old pairing.
        if let Some(q) = &quick_play {
            for w in &mut self.worlds {
                if w.quick_play.as_ref() == Some(q) {
                    w.quick_play = None;
                }
            }
        }
        self.worlds.push(KnownWorld { seed, zoom, name, quick_play });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_seed_matches_guava() {
        // Python: struct.unpack('<q', sha256(struct.pack('<q', s)).digest()[:8])
        assert_eq!(zoom_seed(0), 8794265229978523055);
        assert_eq!(zoom_seed(-7556262131344217808), -7717583307174374982);
        assert_eq!(zoom_seed(12345), 293737985876514017);
        let abc = sha256(b"abc");
        assert_eq!(abc[..4], [0xba, 0x78, 0x16, 0xbf]);
    }

    #[test]
    fn store_round_trip_and_lookups() {
        let dir = std::env::temp_dir().join(format!("mcmap-seeds-{}", std::process::id()));
        let path = dir.join("worlds.json");
        let mut s = SeedStore::load(path.clone());
        assert!(s.remember(42, Some("Realm".into()), Some("realm-1".into())));
        assert!(!s.remember(42, None, None));
        assert!(s.remember(7, None, Some("realm-1".into())));
        s.save().unwrap();
        let t = SeedStore::load(path);
        assert_eq!(t.by_zoom(zoom_seed(42)).unwrap().name.as_deref(), Some("Realm"));
        assert_eq!(t.by_quick_play("realm-1").unwrap().seed, 7);
        let _ = std::fs::remove_dir_all(dir);
    }
}
