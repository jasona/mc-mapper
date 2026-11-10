# MC Mapper

A local Minecraft Java seed map in Rust, drawn with [Rune](https://github.com/anomalyco/rune), that will
later show your live position from the running game. The plan is in
[`docs/implementation-plan.md`](docs/implementation-plan.md).

Status: **M1** (26.3 surface biomes). The map draws real surface biomes for a seed. The Rust biome core
matches the game's own classes bit for bit (climate floats and biomes over 4.5M cells, 6 seeds) and the
reference capture cell for cell. Structures, the depth layer and live position in the window come next.

## Layout

| Path | What |
| --- | --- |
| `crates/mcmap-data` | `WorldId`, versions, launcher-install discovery, version-pack format, Java build helpers |
| `crates/mcmap-worldgen` | Xoroshiro, Perlin/NormalNoise (single precision), climate density functions, the biome RTree |
| `crates/mcmap-tiles` | Tile pyramid and `f64` camera, checkerboard tiles, atlas segmentation |
| `crates/mcmap-live` | `PositionSource` seam, agent protocol and socket, F3+C parser, replay source |
| `crates/mcmap-app` | The window (`mcmap`). The only crate that depends on Rune (pinned rev) |
| `crates/mcmap-cli` | `mcmap-cli spike`, `listen`, `find`, `render`, `calibrate` |
| `xtask` | `cargo run -q -p xtask -- extract`, `oracle`, `agent`, `atlas` |
| `java/oracle` | Dev-only harness that runs the game's own biome code from your client jar |
| `java/agent` | The read-only in-game agent, plus a stand-in game for testing attach |

The folder is not a git repo yet: `git init -b main && git add -A && git commit -m "M0"`. The
`.gitignore` already keeps `art/`, packs and build output out.

`art/` (Chunkbase-derived, personal use only) and version packs (Mojang's data) are git-ignored. Keep
them out of any public repo.

## Requirements

Linux x64 with a working Vulkan driver, `libxkbcommon-dev` and `libfontconfig-dev`. `rustup` installs
the pinned toolchain (1.99.1, matching the Rune rev) on first build. Java comes from the launcher's own
runtime in `~/.minecraft/runtime`, so no separate JDK is needed. Override with `MCMAP_MINECRAFT_DIR`
and `MCMAP_JAVA_HOME`.

## Commands

Dev tasks run through the `xtask` crate. For the short form `cargo xtask ...`, add this alias to
`.cargo/config.toml` (Claude couldn't write that folder remotely):

```toml
[alias]
xtask = "run -q -p xtask --"
```

```sh
cargo test --workspace                        # all headless tests
cargo run -p mcmap-app --release              # the map window (art from ./art/seed-map, pack from the data dir)
MCMAP_SEED=<seed> cargo run -p mcmap-app --release   # another seed (default: the reference capture's)
cargo run -p mcmap-cli --release -- render --seed <seed> --center 0,0 --ppb 0.25 --out biomes.png
cargo run -p mcmap-cli --release -- calibrate        # the reference capture, cell for cell
cargo run -p mcmap-app --release -- --snap out.png --scene home --scale 2   # headless snapshot
                                              # scenes: home, zoomed-out, zoomed-in, far
cargo run -q -p xtask -- extract --version 26.3            # version pack -> ~/.local/share/mcmap/packs/26.3.mcpack
cargo run -q -p xtask -- oracle -- biome -7556262131344217808 2839 -1418 surface
cargo run -q -p xtask -- oracle -- grid <seed> <x0> <z0> <w> <h> <step> out.bin surface
cargo run -q -p xtask -- agent                             # build target/java/mcmap-agent.jar and self-check it
cargo run -q -p xtask -- oracle -- climgrid <seed> <x0> <z0> <w> <h> <step> out.bin   # diff-test data
cargo run -q -p xtask -- atlas                             # art/seed-map/map/marker-crops.json (labels by hand)
```

Run `extract` once before the map: without a pack the window falls back to the checkerboard. The
differential test is `MCMAP_PACK=<pack> MCMAP_DIFF=<dir of climgrid files> cargo test -p mcmap-worldgen
--release --test oracle -- --ignored` (see the test's header for file naming).

Map controls: drag to pan, mouse wheel zooms about the pointer, two-finger trackpad pans,
Ctrl + wheel or pinch zooms.

## The live-position spike (run this with the game open)

This is the M0 gate that only your machine can run: attach the agent to the real game and see your
position.

1. `cargo run -q -p xtask -- agent`. It builds the jar with the launcher's `javac` and prints
   `ok: every name the agent reads exists in this client`.
2. Start Minecraft 26.3 from the launcher as usual and join your Realm (or any world).
3. `cargo run -p mcmap-cli -- spike --seconds 30`

Expected: `jcmd: ... return code: 0`, then `map: agent connected: mcmap-agent/0.1 on 26.3`, then a
line every half second like
`minecraft:overworld 2839.50 71.00 -1418.23 yaw -90.0  server <Realm name> (realm)  zoomSeed <n>  (20.0 fixes/s)`.

What it does: finds the game by its main class in `/proc`, runs the game's own `jcmd` with
`JVMTI.agent_load`, and listens on `127.0.0.1:47123`. The agent only reads your own player through
reflection and sends nothing to any server. Java prints a warning about a dynamically loaded agent
in the game's output; that is expected (JEP 451).

Notes:

- The agent stays loaded until the game exits. A second `spike` in the same game session gets no
  fixes (the agent keeps the first session's token); restart the game to repeat it.
- The agent writes `target/java/mcmap-agent.log`, one line per second, and logs any missing name.
- If `jcmd` refuses, the fallback is a JVM argument. In the launcher: Installations, edit the 26.3
  installation, More options, JVM arguments, append
  `-javaagent:/home/jason/coding/mc-mapper/target/java/mcmap-agent.jar=port:47123,token:spike`,
  then run `cargo run -p mcmap-cli -- listen --token spike` and start the game.
- Agent options use `key:value` because `jcmd` swallows `key=value` arguments.
