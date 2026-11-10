//! Finds running Minecraft clients by their main class (Linux: /proc).

use std::path::PathBuf;

pub struct Game {
    pub pid: u32,
    pub version: Option<String>,
    /// The `java` executable the game runs on; `jcmd` sits beside it.
    pub java: PathBuf,
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
        let version = args.iter().position(|a| a == "--version").and_then(|i| args.get(i + 1)).cloned();
        let java = std::fs::read_link(e.path().join("exe")).unwrap_or_else(|_| PathBuf::from(&args[0]));
        out.push(Game { pid, version, java });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn scanning_does_not_fail_without_a_game() {
        // No client runs in CI; the scan must simply come back empty.
        let _ = super::running();
    }
}
