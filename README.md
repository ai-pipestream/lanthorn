# babelmap

**Play interactive fiction in your terminal while babelmap draws the map for you — live, as you explore.**

babelmap is an interactive-fiction interpreter with a built-in *automapper*. Load
a story file — the Infocom catalog and Z-machine classics (Zork), or modern
Inform 7 / Glulx games — play it in a clean TUI, and watch a room-and-connection
map assemble itself from your movements. No graph paper, no manual annotation:
every room you enter and every exit you take is placed and routed automatically,
then continuously tidied into a readable layout.

```
babelmap path/to/story.z5
```

---

## What it is

babelmap is a Rust workspace. The interpreter and the mapper are deliberately
decoupled: a VM reports *where you are*, and the mapper turns the stream of
locations and movements into a spatial graph without knowing anything about the
underlying engine.

| Crate | Responsibility |
|-------|----------------|
| `zvm` | A from-scratch Z-machine virtual machine — executes story files, standard Quetzal save/restore. Zero-dependency. |
| `gvm` | A Glulx virtual machine (Glk I/O) for modern Inform 7 games — accelerated Inform veneer, full float opcodes. Zero-dependency. |
| `mapper` | A VM-agnostic map model: rooms, connections, layered 2-D layout, overlap removal, edge routing. Serializable. |
| `app` | The `babelmap` TUI binary (ratatui + crossterm): play loop, live map rendering, all interactive features. |
| `zvm-cli` / `gvm-cli` | Standalone DOS-style command-line players (no map): save/restore, single-key input, terminal-bell bleeps — and, piped, a clean deterministic harness for testing/scripting. |
| `blorb` | Blorb container parsing — bundled story, cover art, and sound/image resources. |
| `audio` | Sound playback (rodio) — synthesized bleeps and sampled AIFF / Ogg / ProTracker MOD. |

**Supported story formats:** Z-machine v3, v4, v5, v7, and v8, and Glulx. (Z-machine
v6 is graphical and unsupported; v1/v2 are not supported.) Story files load raw,
from a `.zip`, or from a **Blorb** container (`.zblorb`/`.blorb`/`.gblorb`).

---

## Highlights

- **Two engines** — full Z-machine (v3/v4/v5/v7/v8) *and* Glulx (Inform 7) play,
  including the v4+ cursor-addressed upper-window screen model, timed/interrupt
  input, and per-title header tuning. → [interpreter](docs/features/interpreter.md)
- **Live automapping** — rooms and connections placed, routed, and de-overlapped
  automatically as you explore, across layered multi-level areas, and continuously
  re-tidied. Works for Z-machine *and* Glulx/Inform 7 games. → [mapping](docs/features/mapping.md)
- **Sound** — Z-machine `sound_effect` bleeps and Blorb sampled audio, plus Glulx
  Glk sound channels with per-channel volume and finish events (AIFF/Ogg/MOD).
  → [interpreter](docs/features/interpreter.md) · [remote audio](docs/remote-sound.md)
- **Graphics** — cover art, in-game Glulx graphics windows, and inline images in
  text, rendered with the terminal's best protocol (Kitty / iTerm2 / Sixel) and a
  universal half-block fallback. → [interface](docs/features/interface.md)
- **Game-driven colour & text styling** — `set_colour` / `set_true_colour` and Glk
  style hints honored at 24-bit RGB, with per-span bold/italic/reverse emphasis.
  → [interpreter](docs/features/interpreter.md) · [customization](docs/features/customization.md)
- **Rewind & replay** — step back through a recorded per-turn history with the map
  reconstructed at each moment, and resume from any earlier turn. → [saves](docs/features/saves.md)
- **A full TUI** — mouse support, select-and-copy, verb/noun menu, dictionary
  autocomplete, inventory strip, command history, in-game Invisiclues hints, and
  transcript search / filter / export. → [interface](docs/features/interface.md)
- **Saves & persistence** — self-contained `.babelmap` Save States (map + VM
  state + metadata), named slots, Quetzal import/export, auto-save/auto-load. → [saves](docs/features/saves.md)
- **Deeply themeable** — a live click-to-edit style editor, symbol gallery,
  shareable `style.toml` files, per-game looks, a templated status bar, and a
  fully configurable keymap. → [customization](docs/features/customization.md)
- **Story picker** — launch a directory to browse your games with type/artifact
  badges, an info side-panel, and cover art. → [interface](docs/features/interface.md)
- **Robust** — a faulting story halts with a call-frame stack trace (written to
  `~/.babelmap/crash.log`) while the app stays interactive, instead of taking the
  interpreter down. → [interpreter](docs/features/interpreter.md)

For the full, exhaustive feature list, see **[`docs/features/`](docs/features/)**.

---

## Installation & usage

Requires a Rust toolchain. On Linux, the `playback` audio feature (on by
default) needs ALSA development headers to build: `libasound2-dev`
(Debian/Ubuntu) or `alsa-lib-devel` (Fedora).

```bash
# Build
cargo build --release

# Run a story
cargo run --release -p app -- path/to/story.z5
# or, after building:
./target/release/babelmap path/to/story.z5
```

You type commands at the story's own inline `>` prompt in the transcript, the
way a classic terminal interpreter works. Prefer a dedicated input line pinned
to the bottom instead? Set `command_bar = true` in the config (or toggle it on
the Settings screen).

Press the leader key (default `Ctrl+K`) in-app to pop up a reference panel of
every command; press the single letter shown beside one to run it and return to
play (tmux-style). A few essentials (Tab, `Ctrl+S`/`Ctrl+R`, quit) and map
navigation stay always-active and are listed in the bottom bar.

### Configuration

babelmap reads `~/.babelmap/config.toml` (override the directory with
`--user-dir`, or point at a specific file with `--config`). Every setting has a
default, so the file is optional. Command-line flags take precedence over the
config file, which takes precedence over built-in defaults. See
[customization & configuration](docs/features/customization.md) for the settings.

---

## Development

```bash
cargo build --workspace          # build everything
cargo test --workspace           # run the full test suite
cargo run -p zvm-cli -- story.z5 # DOS-style CLI player (no map)
cargo run -p gvm-cli -- story.ulx # DOS-style Glulx CLI player (no map)
```

`zvm-cli` / `gvm-cli` render a basic DOS-style screen (pinned status line / upper
window via ANSI when interactive, clearing the screen on start) and degrade to a
clean line stream when piped. Interactively they do single-key input (arrow/function
keys decoded for `read_char` menus) and `[MORE]` paging on long output; aux save
tables persist per game by IFID. On piped stdin, a `read_char` menu exits cleanly
at true EOF instead of spinning. The flags `--no-status` (byte-identical
lower-stream output), `--no-aux`, and `--no-more` keep the headless test harness
deterministic; `--no-sound` disables audio and `--volume <0-100>` sets the master
volume.

The crates are layered `zvm`/`gvm` → `mapper` → `app`; the CLIs are thin VM
front-ends. The mapper has no dependency on any VM, so layout logic can be tested
in isolation, and the VM crates stay zero-dependency (image/audio/resource types
live in `app`, `blorb`, and `audio`).

The `audio` crate carries two default-on features: `playback` (real output via
`rodio`) and `mod-music` (ProTracker `.mod` playback via `mod_player`, requires
`playback`). Build with `--no-default-features` to get a compile-time no-op
backend for headless/CI environments; `--no-default-features --features
playback` keeps AIFF/Ogg sample playback without MOD support. With `playback`
on, a missing audio device at runtime degrades to silence rather than erroring.
