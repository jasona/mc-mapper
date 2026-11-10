//! Replays a recorded path: newline-delimited `{"t": seconds, "dim": "...", "x", "y", "z", "yaw"}`.
//! Drives the marker in headless tests and demos before any real source exists.

use crate::fix::{Fix, FixSink, PositionSource, SourceKind, StopToken, WorldHint};
use mcmap_data::Dimension;
use serde::Deserialize;
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Debug, Deserialize)]
pub struct Step {
    pub t: f64,
    #[serde(default = "overworld")]
    pub dim: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[serde(default)]
    pub yaw: f32,
}

fn overworld() -> String {
    "minecraft:overworld".into()
}

pub struct ReplaySource {
    pub steps: Vec<Step>,
    /// Playback speed; 0 sends every step at once.
    pub speed: f64,
    pub looped: bool,
}

impl ReplaySource {
    pub fn parse(text: &str) -> Result<ReplaySource, String> {
        let steps = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
            .collect::<Result<Vec<Step>, _>>()?;
        Ok(ReplaySource { steps, speed: 1.0, looped: false })
    }
}

impl Step {
    pub fn fix(&self) -> Fix {
        Fix {
            world: WorldHint::default(),
            dimension: Dimension::from_key(&self.dim),
            dimension_key: self.dim.clone(),
            pos: [self.x, self.y, self.z],
            yaw: self.yaw,
            pitch: 0.0,
            taken_at: SystemTime::now(),
            source: SourceKind::Replay,
        }
    }
}

impl PositionSource for ReplaySource {
    fn run(self: Box<Self>, out: FixSink, stop: StopToken) {
        loop {
            let start = Instant::now();
            for s in &self.steps {
                if self.speed > 0.0 {
                    let due = Duration::from_secs_f64(s.t.max(0.0) / self.speed);
                    while start.elapsed() < due {
                        if stop.stopped() {
                            return;
                        }
                        std::thread::sleep((due - start.elapsed()).min(Duration::from_millis(50)));
                    }
                }
                if stop.stopped() || !out.send(s.fix()) {
                    return;
                }
            }
            if !self.looped || self.steps.is_empty() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn;

    #[test]
    fn replays_in_order() {
        let mut src = ReplaySource::parse("{\"t\":0,\"x\":0,\"y\":64,\"z\":0}\n\n{\"t\":0.05,\"dim\":\"minecraft:the_nether\",\"x\":8,\"y\":70,\"z\":-8,\"yaw\":90}\n").unwrap();
        src.speed = 10.0;
        let (rx, _stop) = spawn(Box::new(src));
        let a = rx.recv().unwrap();
        let b = rx.recv().unwrap();
        assert_eq!(a.pos, [0.0, 64.0, 0.0]);
        assert_eq!((b.dimension, b.pos[0]), (Some(Dimension::Nether), 8.0));
        assert!(rx.recv().is_err(), "the source ends after the last step");
    }
}
