//! mcmap-cli spike                 find the running game, attach the agent, print your position
//! mcmap-cli listen --token T      only listen (when the agent was loaded another way)
//! mcmap-cli find                  show the running game process and its Java runtime
//! mcmap-cli render --seed S ...   surface biome map to a PNG
//! mcmap-cli calibrate --seed S    compare the biome core with a reference capture

mod biomes;
mod game;

use clap::{Parser, Subcommand};
use mcmap_live::agent::{AgentSource, DEFAULT_PORT};
use mcmap_live::{spawn, Fix};
use std::path::PathBuf;
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the running Minecraft client(s).
    Find,
    /// Attach the agent to the running game and print fixes as they arrive.
    Spike {
        /// The agent jar (build it with `cargo run -q -p xtask -- agent`).
        #[arg(long, default_value = "target/java/mcmap-agent.jar")]
        jar: PathBuf,
        /// Which game process, when more than one is running.
        #[arg(long)]
        pid: Option<u32>,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Stop after this many seconds (0: run until Ctrl-C).
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
    /// Listen for an agent that was loaded some other way (for example -javaagent).
    Listen {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        #[arg(long)]
        token: String,
        #[arg(long, default_value_t = 0)]
        seconds: u64,
    },
    /// Render the surface biome map of a seed to a PNG.
    Render {
        #[arg(long, allow_hyphen_values = true)]
        seed: i64,
        /// Centre block "x,z".
        #[arg(long, default_value = "0,0", allow_hyphen_values = true)]
        center: String,
        /// Pixels per block (0.25 is one pixel per 4x4 cell).
        #[arg(long, default_value_t = 0.25)]
        ppb: f64,
        #[arg(long, default_value_t = 1024)]
        width: u32,
        #[arg(long, default_value_t = 1024)]
        height: u32,
        #[arg(long, default_value = "26.3")]
        version: String,
        /// Version pack (default: the data directory's pack for --version).
        #[arg(long)]
        pack: Option<PathBuf>,
        #[arg(long, default_value = "biomes.png")]
        out: PathBuf,
    },
    /// Compare the biome core with a reference capture (default: the M0 capture's seed and view).
    Calibrate {
        #[arg(long, default_value_t = -7556262131344217808, allow_hyphen_values = true)]
        seed: i64,
        #[arg(long, default_value = "art/seed-map/reference/biomes.png")]
        reference: PathBuf,
        #[arg(long, default_value = "2839,-1418", allow_hyphen_values = true)]
        center: String,
        #[arg(long, default_value_t = 1.25)]
        ppb: f64,
        #[arg(long, default_value = "26.3")]
        version: String,
        #[arg(long)]
        pack: Option<PathBuf>,
    },
}

fn parse_center(s: &str) -> Result<[f64; 2], String> {
    let (x, z) = s.split_once(',').ok_or("centre is \"x,z\"")?;
    Ok([x.trim().parse().map_err(|_| "bad x")?, z.trim().parse().map_err(|_| "bad z")?])
}

fn main() -> ExitCode {
    let r = match Cli::parse().cmd {
        Cmd::Find => find(),
        Cmd::Spike { jar, pid, port, seconds } => spike(jar, pid, port, seconds),
        Cmd::Listen { port, token, seconds } => listen(port, token, seconds),
        Cmd::Render { seed, center, ppb, width, height, version, pack, out } => parse_center(&center)
            .and_then(|c| biomes::load(&version, pack, seed).and_then(|s| biomes::render(&s, c, ppb, width, height, &out))),
        Cmd::Calibrate { seed, reference, center, ppb, version, pack } => parse_center(&center).and_then(|c| {
            let s = biomes::load(&version, pack, seed)?;
            if biomes::calibrate(&s, &reference, c, ppb)? { Ok(()) } else { Err("the biome core disagrees with the capture".into()) }
        }),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn find() -> Result<(), String> {
    let games = game::running();
    if games.is_empty() {
        return Err("no Minecraft client is running (looked for main class net.minecraft.client.main.Main)".into());
    }
    for g in games {
        println!("pid {}  version {}  java {}", g.pid, g.version.as_deref().unwrap_or("?"), g.java.display());
    }
    Ok(())
}

fn token() -> String {
    // Not cryptographic; it only keeps other local processes from feeding the map by accident.
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{:x}{:x}", t.as_nanos() as u64 ^ 0x9e37_79b9_7f4a_7c15, std::process::id())
}

fn spike(jar: PathBuf, pid: Option<u32>, port: u16, seconds: u64) -> Result<(), String> {
    let jar = jar.canonicalize().map_err(|e| format!("{}: {e}; run `cargo run -q -p xtask -- agent` first", jar.display()))?;
    let games = game::running();
    let g = match pid {
        Some(p) => games.into_iter().find(|g| g.pid == p).ok_or(format!("pid {p} is not a running Minecraft client"))?,
        None => match games.len() {
            0 => return Err("no Minecraft client is running; start the game first".into()),
            1 => games.into_iter().next().unwrap(),
            _ => return Err("several clients are running; pass --pid (see `mcmap-cli find`)".into()),
        },
    };
    let tok = token();
    // Listen before attaching so the agent's first connection lands.
    let mut src = AgentSource::bind(port, tok.clone()).map_err(|e| format!("port {port}: {e}"))?;
    src.on_status = Some(Box::new(|s| eprintln!("map: {s}")));
    let (rx, _stop) = spawn(Box::new(src));
    let jcmd = g.java.parent().unwrap().join("jcmd");
    println!("attaching {} to pid {} (version {}) with {}", jar.display(), g.pid, g.version.as_deref().unwrap_or("?"), jcmd.display());
    let out = Command::new(&jcmd)
        .arg(g.pid.to_string())
        .arg("JVMTI.agent_load")
        .arg(&jar)
        .arg(format!("port:{port},token:{tok}"))
        .output()
        .map_err(|e| format!("{}: {e}", jcmd.display()))?;
    let said = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    println!("jcmd: {}", said.trim());
    if !out.status.success() || !said.contains("return code: 0") {
        return Err("jcmd did not load the agent; see its output above. Fallback: add -javaagent to the launcher's JVM arguments (README)".into());
    }
    print_fixes(rx, seconds)
}

fn listen(port: u16, token: String, seconds: u64) -> Result<(), String> {
    let mut src = AgentSource::bind(port, token).map_err(|e| format!("port {port}: {e}"))?;
    src.on_status = Some(Box::new(|s| eprintln!("map: {s}")));
    println!("listening on 127.0.0.1:{}", src.port());
    let (rx, _stop) = spawn(Box::new(src));
    print_fixes(rx, seconds)
}

fn print_fixes(rx: std::sync::mpsc::Receiver<Fix>, seconds: u64) -> Result<(), String> {
    let end = (seconds > 0).then(|| Instant::now() + Duration::from_secs(seconds));
    let mut count = 0u64;
    let mut last_print = Instant::now() - Duration::from_secs(1);
    let started = Instant::now();
    while end.is_none_or(|e| Instant::now() < e) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(f) => {
                count += 1;
                if last_print.elapsed() >= Duration::from_millis(500) {
                    last_print = Instant::now();
                    println!(
                        "{} {:.2} {:.2} {:.2} yaw {:.1}  server {}{}  zoomSeed {}  ({:.1} fixes/s)",
                        f.dimension_key,
                        f.pos[0],
                        f.pos[1],
                        f.pos[2],
                        f.yaw,
                        f.world.server.as_deref().unwrap_or("-"),
                        if f.world.realm { " (realm)" } else { "" },
                        f.world.zoom_seed.map(|s| s.to_string()).unwrap_or("-".into()),
                        count as f64 / started.elapsed().as_secs_f64()
                    );
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    println!("{count} fixes");
    if count == 0 { Err("no fixes arrived; are you in a world? See the agent log (mcmap-agent.log next to the jar)".into()) } else { Ok(()) }
}
