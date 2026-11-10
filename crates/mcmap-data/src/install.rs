//! Finds what the vanilla launcher installed: game directory, version jars and manifests, the
//! game's libraries and the Java runtime the launcher ships. Read-only.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// A `.minecraft` directory.
#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
}

#[derive(Debug)]
pub struct VersionFiles {
    pub name: String,
    pub jar: PathBuf,
    pub manifest: PathBuf,
}

impl Install {
    /// `$MCMAP_MINECRAFT_DIR`, else `~/.minecraft` (Linux and macOS launchers' default on Linux).
    pub fn default_location() -> Option<Install> {
        if let Some(dir) = std::env::var_os("MCMAP_MINECRAFT_DIR") {
            return Some(Install { root: dir.into() });
        }
        let home = std::env::var_os("HOME")?;
        Some(Install { root: Path::new(&home).join(".minecraft") })
    }

    pub fn at(root: impl Into<PathBuf>) -> Install {
        Install { root: root.into() }
    }

    pub fn version(&self, name: &str) -> Result<VersionFiles, String> {
        let dir = self.root.join("versions").join(name);
        let jar = dir.join(format!("{name}.jar"));
        let manifest = dir.join(format!("{name}.json"));
        for p in [&jar, &manifest] {
            if !p.is_file() {
                return Err(format!("{} not found; is version {name} installed in {}?", p.display(), self.root.display()));
            }
        }
        Ok(VersionFiles { name: name.to_owned(), jar, manifest })
    }

    /// Installed version names, sorted.
    pub fn versions(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.root.join("versions"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// The game's classpath for this version: every library the manifest allows on Linux, then the
    /// client jar. Libraries that are not on disk are skipped and returned separately.
    pub fn classpath(&self, v: &VersionFiles) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
        let manifest = read_json(&v.manifest)?;
        let mut have = Vec::new();
        let mut missing = Vec::new();
        for lib in manifest["libraries"].as_array().into_iter().flatten() {
            if !rules_allow(&lib["rules"]) {
                continue;
            }
            let Some(path) = lib["downloads"]["artifact"]["path"].as_str() else { continue };
            let p = self.root.join("libraries").join(path);
            if p.is_file() { have.push(p) } else { missing.push(p) }
        }
        have.push(v.jar.clone());
        Ok((have, missing))
    }

    /// The Java runtime the launcher uses for this version (`javaVersion.component`), with its
    /// `bin/` directory, where `java`, `javac` and `jcmd` live.
    pub fn java_home(&self, v: &VersionFiles) -> Result<PathBuf, String> {
        let manifest = read_json(&v.manifest)?;
        let component = manifest["javaVersion"]["component"].as_str().ok_or("manifest names no javaVersion.component")?;
        let home = self.root.join("runtime").join(component).join(os_dir()).join(component);
        if home.join("bin").join(exe("java")).is_file() {
            Ok(home)
        } else {
            Err(format!("launcher Java runtime not found at {}", home.display()))
        }
    }
}

pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() }
}

fn os_dir() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "mac-os-arm64" } else { "mac-os" }
    } else {
        "windows-x64"
    }
}

fn os_name() -> &'static str {
    if cfg!(target_os = "linux") { "linux" } else if cfg!(target_os = "macos") { "osx" } else { "windows" }
}

fn read_json(p: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", p.display()))
}

/// The launcher's library rules: the last matching rule wins, and no rules means allowed.
fn rules_allow(rules: &Value) -> bool {
    let Some(rules) = rules.as_array() else { return true };
    let mut allowed = false;
    for r in rules {
        let os = &r["os"];
        let matches = os.is_null()
            || (os["name"].as_str().is_none_or(|n| n == os_name()) && os["arch"].as_str().is_none_or(|a| a == std::env::consts::ARCH));
        if matches {
            allowed = r["action"].as_str() == Some("allow");
        }
    }
    allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules_follow_the_launcher() {
        assert!(rules_allow(&Value::Null));
        assert!(rules_allow(&json!([{ "action": "allow" }])));
        assert_eq!(rules_allow(&json!([{ "action": "allow", "os": { "name": "linux" } }])), cfg!(target_os = "linux"));
        assert_eq!(rules_allow(&json!([{ "action": "allow", "os": { "name": "osx" } }])), cfg!(target_os = "macos"));
        assert!(!rules_allow(&json!([{ "action": "allow" }, { "action": "disallow", "os": { "name": os_name() } }])));
    }
}
