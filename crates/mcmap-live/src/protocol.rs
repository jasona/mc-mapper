//! Wire formats: the agent's newline-delimited JSON (version 1) and the F3+C clipboard command.
//!
//! Agent to map, one JSON object per line:
//!
//! ```text
//! {"v":1,"type":"hello","token":"…","agent":"mcmap-agent/0.1","game":"26.3"}
//! {"v":1,"type":"pos","t":1762731000123,"dim":"minecraft:overworld","x":2839.5,"y":71.0,"z":-1418.2,
//!  "yaw":-90.0,"pitch":12.5,"server":"My Realm","realm":true,"zoomSeed":-123456789}
//! {"v":1,"type":"status","state":"menu"}            no world loaded (title screen)
//! {"v":1,"type":"status","state":"error","msg":"…"} a reflective lookup failed; msg names it
//! ```

use crate::fix::{Fix, SourceKind, WorldHint};
use mcmap_data::Dimension;
use serde::Deserialize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const VERSION: u32 = 1;

#[derive(Debug, PartialEq)]
pub enum Message {
    Hello { token: String, agent: String, game: Option<String> },
    Pos(Fix),
    Status { state: String, msg: Option<String> },
}

#[derive(Deserialize)]
struct Raw {
    v: u32,
    #[serde(rename = "type")]
    kind: String,
    token: Option<String>,
    agent: Option<String>,
    game: Option<String>,
    t: Option<u64>,
    dim: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    z: Option<f64>,
    yaw: Option<f32>,
    pitch: Option<f32>,
    server: Option<String>,
    realm: Option<bool>,
    #[serde(rename = "zoomSeed")]
    zoom_seed: Option<i64>,
    state: Option<String>,
    msg: Option<String>,
}

pub fn parse_line(line: &str) -> Result<Message, String> {
    let r: Raw = serde_json::from_str(line).map_err(|e| format!("bad agent line: {e}"))?;
    if r.v != VERSION {
        return Err(format!("agent speaks protocol {}, the map speaks {VERSION}", r.v));
    }
    match r.kind.as_str() {
        "hello" => Ok(Message::Hello {
            token: r.token.ok_or("hello without token")?,
            agent: r.agent.unwrap_or_default(),
            game: r.game,
        }),
        "pos" => {
            let dim = r.dim.ok_or("pos without dim")?;
            let (Some(x), Some(y), Some(z)) = (r.x, r.y, r.z) else { return Err("pos without x, y, z".into()) };
            Ok(Message::Pos(Fix {
                world: WorldHint { server: r.server, realm: r.realm.unwrap_or(false), zoom_seed: r.zoom_seed, seed: None },
                dimension: Dimension::from_key(&dim),
                dimension_key: dim,
                pos: [x, y, z],
                yaw: r.yaw.unwrap_or(0.0),
                pitch: r.pitch.unwrap_or(0.0),
                taken_at: r.t.map(|ms| UNIX_EPOCH + Duration::from_millis(ms)).unwrap_or_else(SystemTime::now),
                source: SourceKind::Agent,
            }))
        }
        "status" => Ok(Message::Status { state: r.state.unwrap_or_default(), msg: r.msg }),
        other => Err(format!("unknown agent message type {other:?}")),
    }
}

/// Parse what F3+C copies: `/execute in <dimension> run tp @s <x> <y> <z> <yaw> <pitch>`
/// (format string in the 26.3 client's `KeyboardHandler`).
pub fn parse_f3c(text: &str) -> Option<Fix> {
    let mut it = text.split_whitespace();
    if it.next()? != "/execute" || it.next()? != "in" {
        return None;
    }
    let dim = it.next()?.to_owned();
    if it.next()? != "run" || it.next()? != "tp" || it.next()? != "@s" {
        return None;
    }
    let mut n = || it.next().and_then(|v| v.parse::<f64>().ok());
    let (x, y, z, yaw, pitch) = (n()?, n()?, n()?, n()?, n()?);
    Some(Fix {
        world: WorldHint::default(),
        dimension: Dimension::from_key(&dim),
        dimension_key: dim,
        pos: [x, y, z],
        yaw: yaw as f32,
        pitch: pitch as f32,
        taken_at: SystemTime::now(),
        source: SourceKind::Clipboard,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_lines() {
        let hello = parse_line(r#"{"v":1,"type":"hello","token":"abc","agent":"mcmap-agent/0.1","game":"26.3"}"#).unwrap();
        assert_eq!(hello, Message::Hello { token: "abc".into(), agent: "mcmap-agent/0.1".into(), game: Some("26.3".into()) });
        let Message::Pos(f) = parse_line(r#"{"v":1,"type":"pos","t":1000,"dim":"minecraft:the_nether","x":1.5,"y":64,"z":-2.25,"yaw":90,"pitch":0,"server":"R","realm":true,"zoomSeed":-5}"#).unwrap() else { panic!() };
        assert_eq!(f.dimension, Some(Dimension::Nether));
        assert_eq!(f.pos, [1.5, 64.0, -2.25]);
        assert_eq!(f.world, WorldHint { server: Some("R".into()), realm: true, zoom_seed: Some(-5), seed: None });
        assert_eq!(f.taken_at, UNIX_EPOCH + Duration::from_secs(1));
        assert!(parse_line(r#"{"v":2,"type":"pos"}"#).unwrap_err().contains("protocol 2"));
        assert!(matches!(parse_line(r#"{"v":1,"type":"status","state":"menu"}"#), Ok(Message::Status { .. })));
    }

    #[test]
    fn parses_f3c() {
        let f = parse_f3c("/execute in minecraft:overworld run tp @s 2839.50 71.00 -1418.23 -90.15 12.30\n").unwrap();
        assert_eq!(f.dimension, Some(Dimension::Overworld));
        assert_eq!(f.pos, [2839.5, 71.0, -1418.23]);
        assert!((f.yaw + 90.15).abs() < 1e-4);
        assert!(parse_f3c("hello").is_none());
        assert!(parse_f3c("/execute in minecraft:overworld run tp @s 1 2").is_none());
    }
}
