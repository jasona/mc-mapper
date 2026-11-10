# MC Mapper

A local Minecraft Java seed map in Rust, drawn with [Rune](https://github.com/anomalyco/rune), that will
later show your live position from the running game. The plan is in
[`docs/implementation-plan.md`](docs/implementation-plan.md).

Status: **M0** (spike, data and oracle). The map draws a stand-in checkerboard pyramid, not biomes yet.

## Layout

| Path | What |
| --- | --- |
| `crates/mcmap-data` | `WorldId`, versions, launcher-install discovery, version-pack format, Java build helpers |
| `crates/mcmap-worldgen` | Biome generation (M1). Only the `BiomeSampler` seam so far |
| `crates/mcmap-tiles` | Tile pyramid and `f64` camera, checkerboard tiles, atlas segmentation |
| `crates/mcmap-live` | `PositionSource` seam, agent protocol and socket, F3+C parser, replay source |
| `crates/mcmap-app` | The window (`mcmap`). The only crate that depends on Rune (pinned rev) |
| `crates/mcmap-cli` | `mcmap-cli spike`, `listen`, `find` |
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
cargo run -p mcmap-app --release              # the map window (reads art from ./art/seed-map)
cargo run -p mcmap-app --release -- --snap out.png --scene home --scale 2   # headless snapshot
                                              # scenes: home, zoomed-out, zoomed-in, far
cargo run -q -p xtask -- extract --version 26.3            # version pack -> ~/.local/share/mcmap/packs/26.3.mcpack
cargo run -q -p xtask -- oracle -- biome -7556262131344217808 2839 -1418 surface
cargo run -q -p xtask -- oracle -- grid <seed> <x0> <z0> <w> <h> <step> out.bin surface
cargo run -q -p xtask -- agent                             # build target/java/mcmap-agent.jar and self-check it
cargo run -q -p xtask -- atlas                             # art/seed-map/map/marker-crops.json (labels by hand)
```

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
