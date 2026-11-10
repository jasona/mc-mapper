//! Building and running the dev-only Java pieces (the oracle harness and the agent) with the JDK
//! the launcher already ships, so no separate Java install is needed.

use crate::install::{exe, Install, VersionFiles};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `$MCMAP_JAVA_HOME`, else the launcher's runtime for this version.
pub fn java_home(install: &Install, v: &VersionFiles) -> Result<PathBuf, String> {
    if let Some(h) = std::env::var_os("MCMAP_JAVA_HOME") {
        return Ok(h.into());
    }
    install.java_home(v)
}

pub fn tool(home: &Path, name: &str) -> PathBuf {
    home.join("bin").join(exe(name))
}

fn java_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            java_sources(&p, out)
        } else if p.extension().is_some_and(|x| x == "java") {
            out.push(p)
        }
    }
}

/// Compile every `.java` under `src` into `out` with `classpath`, if any source is newer than
/// the stamp file.
pub fn compile(home: &Path, src: &Path, out: &Path, classpath: &[PathBuf], release: Option<u32>) -> Result<(), String> {
    let mut sources = Vec::new();
    java_sources(src, &mut sources);
    if sources.is_empty() {
        return Err(format!("no Java sources under {}", src.display()));
    }
    let stamp = out.join(".stamp");
    let newest = sources.iter().filter_map(|p| p.metadata().and_then(|m| m.modified()).ok()).max();
    let built = stamp.metadata().and_then(|m| m.modified()).ok();
    if let (Some(n), Some(b)) = (newest, built)
        && b >= n {
            return Ok(());
        }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(tool(home, "javac"));
    cmd.arg("-nowarn").arg("-d").arg(out);
    if let Some(r) = release {
        cmd.arg("--release").arg(r.to_string());
    }
    if !classpath.is_empty() {
        cmd.arg("-cp").arg(join(classpath));
    }
    cmd.args(&sources);
    let status = cmd.status().map_err(|e| format!("javac: {e}"))?;
    if !status.success() {
        return Err(format!("javac failed for {}", src.display()));
    }
    std::fs::write(&stamp, b"").map_err(|e| e.to_string())
}

pub fn join(paths: &[PathBuf]) -> String {
    let sep = if cfg!(windows) { ";" } else { ":" };
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(sep)
}
