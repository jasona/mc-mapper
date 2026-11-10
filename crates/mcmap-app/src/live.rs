//! The live position, wired up: the agent socket, the F3+C clipboard watch, an optional replay,
//! and a session thread that finds the running game every few seconds and attaches the agent
//! to it with `jcmd`. Every fix goes into one channel; notes and session changes into another.
//! Each sender wakes the window.
//!
//! The agent keeps the token it was first loaded with until the game exits, so the map keeps
//! one token on disk (`<data dir>/agent-token`) and a restarted map reconnects to an agent that
//! is already loaded.

use mcmap_live::agent::{AgentSource, DEFAULT_PORT};
use mcmap_live::clipboard::{self, ClipboardSource, Watch};
use mcmap_live::replay::ReplaySource;
use mcmap_live::session::{self, Game, QuickPlay};
use mcmap_live::{spawn, Fix, Stopper};
use rune::Waker;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub game: Game,
    pub quick_play: Option<QuickPlay>,
}

pub enum Event {
    /// A line for the status bar.
    Note(String),
    /// The running game changed (`None`: it exited).
    Session(Option<Session>),
    /// Nothing happened; the window refreshes the age of the last fix.
    Tick,
}

pub struct Live {
    pub events: Receiver<Event>,
    pub watch: Watch,
    _stoppers: Vec<Stopper>,
}

/// The agent jar: `$MCMAP_AGENT_JAR`, else `target/java/mcmap-agent.jar` under the current
/// directory or beside the executable's target directory (`xtask agent` builds it there).
pub fn agent_jar() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MCMAP_AGENT_JAR") {
        return Some(PathBuf::from(p));
    }
    let mut c = vec![PathBuf::from("target/java/mcmap-agent.jar")];
    if let Ok(exe) = std::env::current_exe() {
        // target/release/mcmap -> target/java/mcmap-agent.jar
        if let Some(t) = exe.parent().and_then(|p| p.parent()) {
            c.push(t.join("java/mcmap-agent.jar"));
        }
    }
    c.into_iter().find(|p| p.is_file()).and_then(|p| p.canonicalize().ok())
}

/// The token the agent must present, kept across runs.
pub fn token() -> String {
    let path = mcmap_data::pack::data_dir().join("agent-token");
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim();
        if !t.is_empty() {
            return t.to_owned();
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let seed = format!("{}-{}-{:?}", now.as_nanos(), std::process::id(), std::thread::current().id());
    let d = mcmap_data::seeds::sha256(seed.as_bytes());
    let t: String = d[..12].iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, &t);
    t
}

fn forward(rx: Receiver<Fix>, tx: Sender<Fix>, wake: Waker) {
    std::thread::spawn(move || {
        while let Ok(f) = rx.recv() {
            if tx.send(f).is_err() {
                return;
            }
            wake();
        }
    });
}

/// Start every source. Fixes go to `fixes`.
pub fn start(wake: Waker, fixes: Sender<Fix>, replay: Option<ReplaySource>) -> Live {
    let (etx, events) = channel();
    let mut stoppers = Vec::new();
    let note = {
        let etx = etx.clone();
        let wake = wake.clone();
        move |s: String| {
            let _ = etx.send(Event::Note(s));
            wake();
        }
    };

    if let Some(r) = replay {
        let (rx, stop) = spawn(Box::new(r));
        forward(rx, fixes.clone(), wake.clone());
        stoppers.push(stop);
    }

    let token = token();
    let src = AgentSource::bind(DEFAULT_PORT, token.clone()).or_else(|_| AgentSource::bind(0, token.clone()));
    let port = match src {
        Ok(mut src) => {
            let port = src.port();
            let n = note.clone();
            src.on_status = Some(Box::new(move |s| n(s.to_owned())));
            let (rx, stop) = spawn(Box::new(src));
            forward(rx, fixes.clone(), wake.clone());
            stoppers.push(stop);
            Some(port)
        }
        Err(e) => {
            note(format!("Agent socket: {e}"));
            None
        }
    };

    let watch = clipboard::probe();
    if watch == Watch::Available {
        let (rx, stop) = spawn(Box::new(ClipboardSource::default()));
        forward(rx, fixes, wake.clone());
        stoppers.push(stop);
    }

    // The session thread: find the game, attach once per process, report changes.
    let auto = std::env::var_os("MCMAP_AUTO_ATTACH").is_none_or(|v| v != "0");
    std::thread::Builder::new()
        .name("mcmap-session".into())
        .spawn(move || {
            let mut attached: BTreeSet<u32> = BTreeSet::new();
            let mut last: Option<Session> = None;
            loop {
                let games = session::running();
                let now = games.into_iter().next().map(|g| {
                    let quick_play = g.game_dir.as_deref().and_then(session::quick_play);
                    Session { game: g, quick_play }
                });
                if now != last {
                    if etx.send(Event::Session(now.clone())).is_err() {
                        return;
                    }
                    wake();
                    last = now.clone();
                }
                if let (Some(s), Some(port)) = (&now, port)
                    && auto
                    && !attached.contains(&s.game.pid)
                {
                    attached.insert(s.game.pid);
                    match agent_jar() {
                        None => note("Game found; build the agent with `cargo run -q -p xtask -- agent` for live position".into()),
                        Some(jar) => match session::attach(&s.game, &jar, port, &token) {
                            Ok(_) => note(format!("Agent attached to the game (pid {})", s.game.pid)),
                            Err(e) => note(e),
                        },
                    }
                }
                if etx.send(Event::Tick).is_err() {
                    return;
                }
                wake();
                std::thread::sleep(Duration::from_secs(3));
            }
        })
        .expect("spawn session thread");

    Live { events, watch, _stoppers: stoppers }
}
