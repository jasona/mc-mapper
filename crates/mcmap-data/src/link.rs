//! What the seed field accepts: a number, any other text (hashed the way the game hashes a typed
//! seed), or a seed-map link such as `...seed-map#seed=123&dimension=overworld&x=10&z=-20&zoom=1`.

use crate::Dimension;

/// The game's rule for the seed box: a number is used as is; other text uses `String.hashCode`.
/// `None` for empty input (the game would pick a random seed).
pub fn seed_from_text(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(v) = t.parse::<i64>() {
        return Some(v);
    }
    let mut h: i32 = 0;
    for u in t.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(u as i32);
    }
    Some(h as i64)
}

/// The parts of a pasted link the map can use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub seed: Option<i64>,
    pub dimension: Option<Dimension>,
    pub x: Option<f64>,
    pub z: Option<f64>,
    pub version: Option<String>,
}

/// Parse a link's query or fragment (`key=value` pairs after `?` or `#`). `None` if the text has
/// no `seed=`, so plain seeds fall through to [`seed_from_text`].
pub fn parse_link(text: &str) -> Option<Link> {
    let t = text.trim();
    if !t.contains("seed=") {
        return None;
    }
    let params = t.split(['?', '#']).skip(1).flat_map(|part| part.split('&'));
    let mut l = Link::default();
    for p in params {
        let Some((k, v)) = p.split_once('=') else { continue };
        let v = v.trim();
        match k {
            "seed" => l.seed = seed_from_text(&percent_decode(v)),
            "dimension" => l.dimension = Dimension::from_key(v),
            "x" => l.x = v.parse().ok(),
            "z" => l.z = v.parse().ok(),
            // "java_26_3" style platform names.
            "platform" => l.version = v.strip_prefix("java_").map(|s| s.replace('_', ".")),
            _ => {}
        }
    }
    l.seed.map(|_| l)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_text_and_empty() {
        assert_eq!(seed_from_text(" -7556262131344217808 "), Some(-7556262131344217808));
        // "hello".hashCode() in Java is 99162322.
        assert_eq!(seed_from_text("hello"), Some(99162322));
        assert_eq!(seed_from_text("   "), None);
    }

    #[test]
    fn links() {
        let l = parse_link("https://www.chunkbase.com/apps/seed-map#seed=-7556262131344217808&platform=java_26_3&dimension=overworld&x=2839&z=-1418&zoom=1.08").unwrap();
        assert_eq!(l.seed, Some(-7556262131344217808));
        assert_eq!(l.dimension, Some(Dimension::Overworld));
        assert_eq!((l.x, l.z), (Some(2839.0), Some(-1418.0)));
        assert_eq!(l.version.as_deref(), Some("26.3"));
        assert_eq!(parse_link("12345"), None);
        assert_eq!(parse_link("x.com/?seed=my%20world").unwrap().seed, seed_from_text("my world"));
    }
}
