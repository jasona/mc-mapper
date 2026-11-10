//! The game session: running clients found by their main class (Linux: /proc), the launcher's
//! quick-play record of the world last joined, and attaching the agent with `jcmd`.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    pub pid: u32,
    pub version: Option<String>,
    /// The `java` executable the game runs on; `jcmd` sits beside it.
    pub java: PathBuf,
    /// `--gameDir`, where `launcher_quick_play.json` lives.
    pub game_dir: Option<PathBuf>,
}

const MAIN: &str = "net.minecraft.client.main.Main";

pub fn running() -> Vec<Game> {
    let mut out = Vec::new();
    #[cfg(target_os = "linux")]
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        let args: Vec<String> = raw.split(|&b| b == 0).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
        if !args.iter().any(|a| a == MAIN) {
            continue;
        }
        let arg = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
        let version = arg("--version");
        let game_dir = arg("--gameDir").map(PathBuf::from);
        let java = std::fs::read_link(e.path().join("exe")).unwrap_or_else(|_| PathBuf::from(&args[0]));
        out.push(Game { pid, version, java, game_dir });
    }
    out
}

/// The world the launcher last started, from `launcher_quick_play.json` (version 2:
/// `quickPlayData.<account>[]` with `epochLastPlayedTimeMs`, `id`, `name` and
/// `javaInstance.game.type` of `realms`, `multiplayer` or `singleplayer`).
#[derive(Clone, Debug, PartialEq)]
pub struct QuickPlay {
    pub kind: String,
    pub id: String,
    pub name: Option<String>,
    pub played_ms: i64,
}

impl QuickPlay {
    /// A key for the seed store: `realms:<id>`, `multiplayer:<id>`.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
}

pub fn quick_play(game_dir: &Path) -> Option<QuickPlay> {
    let text = std::fs::read_to_string(game_dir.join("launcher_quick_play.json")).ok()?;
    parse_quick_play(&text)
}

pub fn parse_quick_play(text: &str) -> Option<QuickPlay> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut best: Option<QuickPlay> = None;
    for list in v.get("quickPlayData")?.as_object()?.values() {
        for e in list.as_array().into_iter().flatten() {
            let Some(kind) = e.pointer("/javaInstance/game/type").and_then(|t| t.as_str()) else { continue };
            let Some(id) = e.get("id").and_then(|t| t.as_str()) else { continue };
            let played_ms = e.get("epochLastPlayedTimeMs").and_then(|t| t.as_i64()).unwrap_or(0);
            if best.as_ref().is_none_or(|b| played_ms > b.played_ms) {
                best = Some(QuickPlay { kind: kind.to_owned(), id: id.to_owned(), name: e.get("name").and_then(|t| t.as_str()).map(str::to_owned), played_ms });
            }
        }
    }
    best
}

/// Load the agent into a running game with the game's own `jcmd` (`JVMTI.agent_load`). Options
/// use `key:value` because `jcmd` swallows `key=value`. Returns jcmd's output.
pub fn attach(game: &Game, jar: &Path, port: u16, token: &str) -> Result<String, String> {
    let jcmd = game.java.parent().ok_or("the game's java has no directory")?.join("jcmd");
    let out = Command::new(&jcmd)
        .arg(game.pid.to_string())
        .arg("JVMTI.agent_load")
        .arg(jar)
        .arg(format!("port:{port},token:{token}"))
        .output()
        .map_err(|e| format!("{}: {e}", jcmd.display()))?;
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() || !said.contains("return code: 0") {
        return Err(format!("jcmd did not load the agent: {}", said.trim()));
    }
    Ok(said.trim().to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn quick_play_picks_the_latest_world() {
        let t = r#"{"quickPlayData":{"123":[
            {"epochLastPlayedTimeMs":10,"id":"old","javaInstance":{"game":{"type":"multiplayer"}},"name":"Old"},
            {"epochLastPlayedTimeMs":20,"id":"r1","javaInstance":{"configId":"x","game":{"type":"realms","gamemode":"survival"}},"name":"My Realm"},
            {"epochLastPlayedTimeMs":30,"id":"nogame","javaInstance":{"configId":"x"}}]},"version":2}"#;
        let q = super::parse_quick_play(t).unwrap();
        assert_eq!((q.kind.as_str(), q.id.as_str(), q.name.as_deref()), ("realms", "r1", Some("My Realm")));
        assert_eq!(q.key(), "realms:r1");
    }

    #[test]
    fn scanning_does_not_fail_without_a_game() {
        // No client runs in CI; the scan must simply come back empty.
        let _ = super::running();
    }
}
