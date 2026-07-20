# babelmap

[![Made with Side-Quest](https://img.shields.io/badge/Made%20with-Side--Quest-f97316)](https://github.com/sharkusk/side-quest)

**Play interactive fiction in your terminal while babelmap draws the map for you — live, as you explore.**

> ⚠️ **Alpha software.** babelmap is under active development and considered
> **alpha** — expect rough edges and breaking changes. Formats are not yet
> stable: the `config.toml` and `style.toml` schemas (and on-disk save data) may
> change between versions.

![babelmap's cover-gallery view: a grid of story covers beside a metadata info panel](docs/cover-gallery.png)

babelmap is a terminal interactive-fiction interpreter with a built-in *automapper*.
Load a story — the Infocom catalog and Z-machine classics like *Zork*, modern
Inform 7 / Glulx games, or a classic Scott Adams text adventure — play it in a
clean TUI, and watch a room-and-connection map assemble itself from your
movements. No graph paper, no manual annotation: every room you enter and every
exit you take is placed, routed, and de-overlapped automatically, then
continuously tidied into a readable layout.

```
babelmap path/to/story.z5
```

---

## A quick tour

### Three engines, one player

Point babelmap at any supported story and it detects the format and picks the
right engine — you never choose. Under the hood are **three brand-new, clean-room
virtual machines written from scratch in pure Rust** — no forks of Frotz or
Glulxe, no C bindings, zero runtime dependencies:

- **Z-machine** (v3/v4/v5/v7/v8) — the Infocom canon and decades of Inform 6
  games, including the v4+ cursor-addressed upper window, timed/interrupt input,
  and per-title header tuning.
- **Glulx** — modern Inform 7 games, with an accelerated Inform veneer, full
  float opcodes, and a complete **Glk 0.7.6** layer verified against the standard
  Glulx/Glk test suites.
- **Scott Adams** (ScottFree `.dat`) — the classic 8-bit text adventures. When a
  game is packaged as a **Blorb with PNG artwork**, its illustrations render too
  (via the image pipeline below); babelmap plays the `.dat` text engine and shows
  the bundled images — it doesn't decode the original SAGA line-draw format.

![A Scott Adams text adventure with its Blorb-bundled PNG artwork, playing beside its live map](docs/scott-adams-graphics.png)

### Live automapping

The mapper watches the stream of locations and movements and turns it into a
spatial graph — rooms boxed, exits routed, overlaps removed, multi-level areas
split into switchable **layers** (the `Main / Cellar / Maze` tabs across the top
of the map). The current room glows; the whole layout re-tidies itself as you
discover more. It's completely engine-agnostic: the same map grows whether you're
playing *Zork*, *Counterfeit Monkey*, or *Adventureland*.

![babelmap playing Zork I with a live automap of the Great Underground Empire](docs/automapping.png)

### Pictures in your terminal

Cover art, in-game Glulx graphics windows, and inline images in the text render
with your terminal's best protocol — **Kitty, iTerm2, or Sixel** — and fall back
to a universal Unicode half-block renderer everywhere else.

![In-game graphics rendered with the Kitty graphics protocol](docs/kitty-graphics.png)

### Multi-window games, faithfully

Games that split the screen into multiple Glk windows — status panes, quote
boxes, side-by-side layouts — are laid out as the author intended, right in the
terminal. **Glulx story colours are fully supported**, too: a game's `garglk`
window and text colours (like the coloured panes in the screenshot below) render
faithfully at 24-bit RGB.

![A Glulx game using a multi-window Glk layout with story-set colours](docs/multi-window-layout.png)

### A Z-machine debugger, built in

Type `/debug` and the map pane becomes a live **debug inspector**: a running
disassembly that tracks the PC, tabbed views of Globals, Locals, Objects,
Dictionary, the Call Stack, and Memory — plus **hover help** that decodes the
opcode under your cursor and clickable operands that jump to their address.

![The built-in Z-machine debug inspector: live disassembly, call stack, and opcode hover help](docs/debug-inspector.png)

### Browse your library

Launch a directory instead of a file to open the **story picker**. Two view
modes — a sortable, badged **list** or a `g` **cover-gallery grid** (shown in the
banner at the top of this page) — each paired with a live **info panel**: the
selected game's cover art, full metadata (author, year, genre, blurb), format,
and IFID, fetched on demand from IFDB and cached per game. The type badges even
tell the three engines apart at a glance (`Z5`, `Scott`, `G3.1.2`).

![The story picker's list view: a sortable, badged catalogue beside the info panel — the cover art here is drawn with the universal Unicode half-block fallback renderer](docs/story-list.png)

---

## Every headline feature

- **Three engines** — Z-machine (v3/v4/v5/v7/v8), Glulx (Inform 7), and Scott
  Adams (ScottFree), auto-detected from the file. Full Glk 0.7.6 support for
  Glulx: file/resource streams, date/time with real local timezones, sound
  channels with pause and volume ramps, echo streams, and Unicode normalization.
  → [interpreter](docs/features/interpreter.md)
- **Live automapping** — rooms and connections placed, routed, and de-overlapped
  as you explore, across layered multi-level areas, and continuously re-tidied.
  Works for every engine. → [mapping](docs/features/mapping.md)
- **Built-in debug inspector** — a live Z-machine disassembler with PC tracking,
  Globals/Locals/Objects/Dictionary/Call-Stack/Memory tabs, opcode hover help,
  and click-to-jump operands. Open it with `/debug`.
  → [interface](docs/features/interface.md)
- **Graphics** — cover art, in-game Glulx graphics windows, and inline images,
  rendered with the terminal's best protocol (Kitty / iTerm2 / Sixel) and a
  universal half-block fallback. → [interface](docs/features/interface.md)
- **Sound** — Z-machine `sound_effect` bleeps and Blorb sampled audio, plus Glulx
  Glk sound channels with per-channel volume and finish events (AIFF/Ogg/MOD).
  → [interpreter](docs/features/interpreter.md) · [remote audio](docs/remote-sound.md)
- **Game-driven colour & text styling** — `set_colour` / `set_true_colour` and Glk
  style hints honored at 24-bit RGB, with per-span bold/italic/reverse emphasis.
  → [interpreter](docs/features/interpreter.md) · [customization](docs/features/customization.md)
- **Rewind & replay** — step back through a recorded per-turn history with the map
  reconstructed at each moment, and resume from any earlier turn. → [saves](docs/features/saves.md)
- **A full TUI** — mouse support, select-and-copy, verb/noun menu, dictionary
  autocomplete, inventory strip, command history, in-game Invisiclues hints,
  animated top-right notification toasts (with a `dump-notifications` recall), and
  transcript search / filter / export. → [interface](docs/features/interface.md)
- **Saves & persistence** — self-contained `.babelmap` Save States (map + VM
  state + metadata), named slots, Quetzal import/export, auto-save/auto-load, and
  Glulx games' external file storage (Glk file streams) auto-persisting across
  sessions. → [saves](docs/features/saves.md) · [persistence model](docs/persistence.md)
- **Deeply themeable** — a small role palette (7 roots) that the whole UI derives
  from, first-class styling for all 11 standard Glk styles, per-game looks, a
  templated status bar, and a fully configurable keymap. Edit a fully-commented,
  auto-seeded `style.toml` and apply changes live with `reload-style` (or
  auto-reload on save). → [customization](docs/features/customization.md)
- **Story picker** — browse a directory with type/artifact badges, a sortable
  author/year list or a `g` cover-gallery grid, an info side-panel with full
  metadata and cover art, and on-demand IFDB metadata fetch, cached per game.
  → [interface](docs/features/interface.md)
- **Robust** — a faulting story halts with a call-frame stack trace (written to
  `~/.babelmap/crash.log`) while the app stays interactive, instead of taking the
  interpreter down. → [interpreter](docs/features/interpreter.md)

For the full, exhaustive feature list, see **[`docs/features/`](docs/features/)**. For the
interactive-fiction standards babelmap implements (Z-Machine, Glulx, Glk, Quetzal, Blorb,
Treaty of Babel), see **[`docs/standards.md`](docs/standards.md)**.

**Supported story formats:** Z-machine v3, v4, v5, v7, and v8; Glulx; and Scott
Adams (ScottFree `.dat`). (Z-machine v6 is graphical and unsupported;
v1/v2 are not supported.) Story files load raw, from a `.zip`, or from a **Blorb**
container (`.zblorb`/`.blorb`/`.gblorb`).

---

## Under the hood

babelmap is a Rust workspace of three from-scratch, zero-dependency virtual
machines — Z-machine, Glulx, and Scott Adams — plus a VM-agnostic automapper, all
tied together by a terminal UI. The interpreter and the mapper are deliberately
decoupled (a VM reports *where you are*; the mapper builds the map), and every
engine renders through one neutral screen model so a single renderer draws them
all.

For the crate layout, the **"Glk only for Glulx, native for Z-machine and Scott"**
I/O decision and why it was made, and how the engines converge on one renderer,
see **[`docs/architecture.md`](docs/architecture.md)**.

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

# Point it at a directory to open the story picker instead
./target/release/babelmap ~/if-games/
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

Saves and sidecars live under `~/.babelmap/saves/<story-filename>.save/` by
default; pass `--data-dir <path>` to store them elsewhere. → [persistence
model](docs/persistence.md)

---

## Development

```bash
cargo build --workspace          # build everything
cargo test --workspace           # fast suite (a few slow tests are skipped)
cargo test --workspace -- --include-ignored  # everything, incl. slow tests
cargo run -p zvm-cli -- story.z5   # DOS-style CLI player (no map)
cargo run -p gvm-cli -- story.ulx  # DOS-style Glulx CLI player (no map)
cargo run -p scott-cli -- story.dat # DOS-style Scott Adams CLI player (no map)
```

A few slow full-game Glulx walkthroughs (Kerkerkruip, Counterfeit Monkey) are
marked `#[ignore]` so the default `cargo test` stays quick; pass
`--include-ignored` to run them (CI should). Doctests are disabled workspace-wide
(`doctest = false`) — there are none, and the rustdoc pass cost seconds.

`zvm-cli` / `gvm-cli` / `scott-cli` render a basic DOS-style screen (pinned status
line / upper window via ANSI when interactive, clearing the screen on start) and
degrade to a clean line stream when piped. Interactively they do single-key input
(arrow/function keys decoded for `read_char` menus) and `[MORE]` paging on long
output; saves and aux/VFS sidecars persist per game under
`<story-dir>/<story-filename>.save/` by default (`--data-dir <path>` overrides the
base). A bare filename typed at the **player's** SAVE/RESTORE prompt (e.g. `quick`)
lands in that per-game directory; a path-bearing value (e.g. `/tmp/x.qzl`) is
honored verbatim. A Glulx game's *own* fixed-name saves (its init cache, autosave,
undo) are written and read there **silently** — no prompt — so e.g. Counterfeit
Monkey auto-restores its startup cache on relaunch and skips its long init. On
piped stdin, a `read_char` menu exits cleanly at true EOF instead of spinning.
The flags `--no-status` (byte-identical lower-stream output), `--no-aux`, and
`--no-more` keep the headless test harness deterministic; `--no-sound`
disables audio and `--volume <0-100>` sets the master volume.

The `audio` crate carries two default-on features: `playback` (real output via
`rodio`) and `mod-music` (ProTracker `.mod` playback via `mod_player`, requires
`playback`). Build with `--no-default-features` to get a compile-time no-op
backend for headless/CI environments; `--no-default-features --features
playback` keeps AIFF/Ogg sample playback without MOD support. With `playback`
on, a missing audio device at runtime degrades to silence rather than erroring.

---

## License

babelmap is released under the **BSD 3-Clause License** — see [`LICENSE`](LICENSE).
