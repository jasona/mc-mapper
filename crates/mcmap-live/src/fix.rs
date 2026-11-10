use mcmap_data::Dimension;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::SystemTime;

/// What a source knows about which world the player is in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldHint {
    /// Server or Realm name, as the client lists it.
    pub server: Option<String>,
    pub realm: bool,
    /// `BiomeManager.biomeZoomSeed`: SHA-256 of the seed, which the client does receive.
    pub zoom_seed: Option<i64>,
    /// The real seed, when the source knows it (singleplayer saves).
    pub seed: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Replay,
    Clipboard,
    Agent,
}

/// One position report.
#[derive(Clone, Debug, PartialEq)]
pub struct Fix {
    pub world: WorldHint,
    /// `None` for a dimension the map doesn't know (a data-pack dimension); `dimension_key` has it.
    pub dimension: Option<Dimension>,
    pub dimension_key: String,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub taken_at: SystemTime,
    pub source: SourceKind,
}

/// Where a source sends fixes. `send` returns false once the map has gone away.
#[derive(Clone)]
pub struct FixSink(Sender<Fix>);

impl FixSink {
    pub fn send(&self, fix: Fix) -> bool {
        self.0.send(fix).is_ok()
    }
}

#[derive(Clone, Default)]
pub struct StopToken(Arc<AtomicBool>);

impl StopToken {
    pub fn stopped(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Stops a running source when told to or when dropped.
pub struct Stopper {
    token: StopToken,
    thread: Option<JoinHandle<()>>,
}

impl Stopper {
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        self.token.0.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Stopper {
    fn drop(&mut self) {
        self.halt();
    }
}

pub trait PositionSource: Send + 'static {
    /// Runs on its own thread until `stop` is set or the sink closes. Sources poll `stop` at least
    /// every 100 ms.
    fn run(self: Box<Self>, out: FixSink, stop: StopToken);
}

/// Run a source on its own thread.
pub fn spawn(source: Box<dyn PositionSource>) -> (Receiver<Fix>, Stopper) {
    let (tx, rx) = mpsc::channel();
    let token = StopToken::default();
    let t = token.clone();
    let thread = std::thread::Builder::new()
        .name("mcmap-position-source".into())
        .spawn(move || source.run(FixSink(tx), t))
        .expect("spawn position source");
    (rx, Stopper { token, thread: Some(thread) })
}
