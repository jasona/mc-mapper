//! cargo xtask extract --version 26.3     build the version pack from your own client jar
//! cargo xtask oracle -- biome <seed> <x> <z>   run the game's own biome code (see java/oracle)
//! cargo xtask agent [--version 26.3]     build java/agent into target/java/mcmap-agent.jar and
//!                                        check its names against that version's jar
//! cargo xtask atlas [--art art/seed-map] write the marker crop table next to the art (not in git)

use clap::{Parser, Subcommand};
use mcmap_data::install::{Install, VersionFiles};
use mcmap_data::java;
use mcmap_data::pack::{self, RawPack};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

#[derive(Parser)]
struct Cli {
    /// The .minecraft directory (default: $MCMAP_MINECRAFT_DIR, else ~/.minecraft).
    #[arg(long, global = true)]
    minecraft: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build a version pack from the installed client jar.
    Extract {
        #[arg(long, default_value = "26.3")]
        version: String,
        /// Skip the oracle dump of the Overworld biome table (no Java needed).
        #[arg(long)]
        no_oracle: bool,
        /// Where to write the pack (default: the user data directory).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the oracle harness against a version's own classes.
    Oracle {
        #[arg(long, default_value = "26.3")]
        version: String,
        /// biome <seed> <x> <z> [y|surface] | climate ... | grid ... | params <out.json>
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Build the agent jar and check its names against a version's client jar.
    Agent {
        #[arg(long, default_value = "26.3")]
        version: String,
    },
    /// Cut the marker atlas into sprites and write a crop table next to it.
    Atlas {
        #[arg(long, default_value = "art/seed-map")]
        art: PathBuf,
    },
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn install(cli: &Cli) -> Result<Install, String> {
    cli.minecraft.clone().map(Install::at).or_else(Install::default_location).ok_or_else(|| "no .minecraft directory; pass --minecraft".into())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let r = match &cli.cmd {
        Cmd::Extract { version, no_oracle, out } => extract(&cli, version, *no_oracle, out.clone()),
        Cmd::Oracle { version, args } => install(&cli).and_then(|i| {
            let v = i.version(version)?;
            oracle(&i, &v, args).map(|_| ())
        }),
        Cmd::Agent { version } => agent(&cli, version),
        Cmd::Atlas { art } => atlas(art),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn game_classpath(i: &Install, v: &VersionFiles) -> Result<Vec<PathBuf>, String> {
    let (cp, missing) = i.classpath(v)?;
    if !missing.is_empty() {
        eprintln!("warning: {} libraries listed by {} are not on disk (first: {})", missing.len(), v.name, missing[0].display());
    }
    Ok(cp)
}

/// Build java/oracle against the version's jar and run it. Returns stdout.
fn oracle(i: &Install, v: &VersionFiles, args: &[String]) -> Result<String, String> {
    let home = java::java_home(i, v)?;
    let cp = game_classpath(i, v)?;
    let out = root().join("target/java/oracle").join(&v.name);
    java::compile(&home, &root().join("java/oracle/src"), &out, &cp, None)?;
    let mut full = vec![out];
    full.extend(cp);
    let o = Command::new(java::tool(&home, "java"))
        .arg("-cp")
        .arg(java::join(&full))
        .arg("mcmap.oracle.Oracle")
        .args(args)
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("java: {e}"))?;
    let text = String::from_utf8_lossy(&o.stdout).into_owned();
    print!("{text}");
    if !o.status.success() {
        return Err(format!("oracle exited with {}", o.status));
    }
    Ok(text)
}

fn extract(cli: &Cli, version: &str, no_oracle: bool, out: Option<PathBuf>) -> Result<(), String> {
    let i = install(cli)?;
    let v = i.version(version)?;
    let file = std::fs::File::open(&v.jar).map_err(|e| format!("{}: {e}", v.jar.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    for n in 0..zip.len() {
        let mut entry = zip.by_index(n).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        if pack::wanted(&name) {
            let mut text = String::new();
            entry.read_to_string(&mut text).map_err(|e| format!("{name}: {e}"))?;
            files.insert(name, text);
        }
    }
    let overworld_parameters = if no_oracle {
        None
    } else {
        let tmp = root().join("target/java").join(format!("params-{version}.json"));
        std::fs::create_dir_all(tmp.parent().unwrap()).map_err(|e| e.to_string())?;
        oracle(&i, &v, &["params".into(), tmp.to_string_lossy().into_owned()])?;
        Some(std::fs::read_to_string(&tmp).map_err(|e| e.to_string())?)
    };
    if !no_oracle {
        // Template geometry for the structure engine (start pieces, ruined portals): it lives
        // in NBT files, so the oracle reads it with the game's own template manager.
        let tmp = root().join("target/java").join(format!("templates-{version}.json"));
        oracle(&i, &v, &["templates".into(), tmp.to_string_lossy().into_owned()])?;
        files.insert(pack::TEMPLATES.to_owned(), std::fs::read_to_string(&tmp).map_err(|e| e.to_string())?);
    }
    let p = RawPack { format: pack::FORMAT, version: version.to_owned(), files, overworld_parameters };
    let path = out.unwrap_or_else(|| pack::pack_path(version));
    p.write(&path)?;
    let s = p.summary();
    println!("wrote {} ({} files)", path.display(), p.files.len());
    println!(
        "  {} biomes, {} structures, {} structure sets, {} noises, {} density functions, Overworld table: {}",
        s.biomes,
        s.structures,
        s.structure_sets,
        s.noises,
        s.density_functions,
        s.overworld_parameter_entries.map(|n| format!("{n} entries")).unwrap_or_else(|| "not dumped".into())
    );
    println!("  density-function types: {}", s.density_function_types.into_iter().collect::<Vec<_>>().join(", "));
    Ok(())
}

fn agent(cli: &Cli, version: &str) -> Result<(), String> {
    let i = install(cli)?;
    let v = i.version(version)?;
    let home = java::java_home(&i, &v)?;
    let dir = root().join("java/agent");
    let classes = root().join("target/java/agent/classes");
    // Release 21 so the same jar loads on any game from 1.20.5 (Java 21) onward.
    java::compile(&home, &dir.join("src"), &classes, &[], Some(21))?;
    let jar = root().join("target/java/mcmap-agent.jar");
    let st = Command::new(java::tool(&home, "jar"))
        .args(["--create", "--file"])
        .arg(&jar)
        .arg("--manifest")
        .arg(dir.join("manifest.txt"))
        .arg("-C")
        .arg(&classes)
        .arg(".")
        .status()
        .map_err(|e| format!("jar: {e}"))?;
    if !st.success() {
        return Err("jar failed".into());
    }
    println!("built {}", jar.display());
    let mut cp = vec![jar];
    cp.extend(game_classpath(&i, &v)?);
    let st = Command::new(java::tool(&home, "java"))
        .arg("-cp")
        .arg(java::join(&cp))
        .arg("mcmap.agent.SelfCheck")
        .status()
        .map_err(|e| format!("java: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("the agent's names don't all exist in {version}")) }
}

fn atlas(art: &Path) -> Result<(), String> {
    use mcmap_tiles::atlas::{segment, Rgba, MARKER_ALPHA};
    let src = art.join("map/marker-atlas.webp");
    let img = Rgba::decode(&std::fs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?)?;
    let rects = segment(&img, MARKER_ALPHA, 0);
    let table = serde_json::json!({
        "file": "map/marker-atlas.webp",
        "note": "Cut by `cargo xtask atlas` on alpha. Labels are assigned by hand; index order is rows top to bottom, left to right.",
        "rectangles": rects.iter().enumerate().map(|(n, r)| serde_json::json!({"index": n, "label": null, "x": r.x, "y": r.y, "width": r.w, "height": r.h})).collect::<Vec<_>>(),
    });
    let out = art.join("map/marker-crops.json");
    if out.exists() {
        return Err(format!("{} exists; it may hold hand labels. Move it aside to regenerate.", out.display()));
    }
    std::fs::write(&out, serde_json::to_string_pretty(&table).unwrap()).map_err(|e| e.to_string())?;
    println!("wrote {} sprites to {}", rects.len(), out.display());
    Ok(())
}
