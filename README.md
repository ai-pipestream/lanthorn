# babelmap

**Play interactive fiction in your terminal while babelmap draws the map for you — live, as you explore.**

babelmap is a Z-machine interpreter with a built-in *automapper*. Load a story
file (Zork, the Infocom catalog, modern Inform games), play it in a clean TUI,
and watch a room-and-connection map assemble itself from your movements. No
graph paper, no manual annotation — every room you enter and every exit you take
is placed and routed automatically, then continuously tidied into a readable
layout.

```
babelmap path/to/story.z5
```

---

## What it is

babelmap is a Rust workspace of four crates:

| Crate | Responsibility |
|-------|----------------|
| `zvm` | A from-scratch Z-machine virtual machine — executes story files, standard Quetzal save/restore. |
| `mapper` | A VM-agnostic map model: rooms, connections, layered 2-D layout, overlap removal, edge routing. Serializable. |
| `app` | The `babelmap` TUI binary (ratatui + crossterm): play loop, live map rendering, all interactive features. |
| `zvm-cli` | A headless command-line runner for the VM (no map), useful for testing and scripting. |

The interpreter and the mapper are deliberately decoupled: the VM reports *where
you are*, and the mapper turns the stream of locations and movements into a
spatial graph without knowing anything about the Z-machine.

**Supported story versions:** Z-machine v3, v4, v5, v7, and v8. (v6 is graphical
and unsupported; v1/v2 are not supported.)

---

## Features by category

### Interpreter (the Z-machine)
- Full play of v3/v4/v5/v7/v8 story files.
- Standard **Quetzal** save/restore — interchangeable with other interpreters.
- Story dictionary introspection (powers verb/noun autocomplete).

### Live automapping
- **Automatic room placement** as you explore — each new location is positioned
  relative to where you came from.
- **Connection routing** between rooms with overlap removal, so the map stays
  readable as it grows.
- **Layered maps** for multi-level areas, with manual layer controls.
- **Background tidy** — the layout re-optimizes itself as you discover rooms.
  Configurable: after every room (default), only on overlap, debounced every few
  rooms, or off (`background_tidy`).
- **Animated layout diagnostics** — step through the relayout algorithm stage by
  stage, each move described ("moved 180 to clear overlap with 193"), to see and
  debug exactly how the map is built.

### Map navigation & inspection
- **Mouse support** — click a room for a story-info panel (name, notes, exits,
  objects); right-click for layout diagnostics; middle-drag to pan.
- **Mouse wheel** pans the map (Shift = horizontal, Ctrl = zoom) and scrolls the
  transcript.
- **Room inspector** overlay — id, name, layer, position, and per-edge
  dropped-constraint flags for understanding layout decisions.
- Pane focus with clear visual highlighting; Tab / Shift-Tab cycle the layout
  (split, map-only, transcript-only).

### Playing aids
- **Verb/noun menu** — a two-pane token palette of common verbs and in-scope
  nouns; pick tokens to build a command (multi-noun via prepositions).
- **Tab autocomplete** from the story's dictionary plus nouns mentioned in the
  current room, with a live suggestion line.
- **Inventory panel** — a toggleable strip of carried items.
- **Reset** — restart the story from the beginning (with confirmation) while
  keeping the accumulated map.

### Saves & persistence
- **`.babelmap` archives** — a single file bundling the map, the game save, and
  metadata. By default a story starts fog-of-war (only what you've explored);
  opt into a shared default map with `use_default_map`.
- **Multiple named save slots** with a saves-manager modal (load / save-as /
  delete), each slot tracking name, turn count, and timestamp.
- **Import / export standard saves** — exchange standard Quetzal `.qzl`/`.sav`
  files with other interpreters via the saves manager (a built-in file browser
  picks the file/destination). Importing keeps your accumulated map.
- **Auto-save** (per turn) and **auto-load** (resume on launch) — both
  configurable.

### Customization
- **Configurable symbols** — room outlines, arrows, portal icons, path glyphs,
  and box styles (thin / thick / double / ascii / borderless). Pick presets or
  override individual glyphs.
- **Symbol gallery** — a live-preview modal for browsing and combining symbol
  presets, saved back to your config.
- **Color schemes** — recolor rooms, connectors, and chrome from a
  [Ghostty](https://ghostty.org) theme file or a built-in (mono / high-contrast /
  tomorrow-night), with per-element overrides. Defaults to your terminal colors.
- **Configurable keymap** via a leader-key model: a configurable prefix
  (default `Ctrl+K`) opens a sticky **hotkey dialog** listing every command;
  any command can be made directly available or routed through the dialog.

### Configuration
- TOML config at `~/.babelmap/config.toml` plus command-line flags
  (`--user-dir`, `--config`); CLI overrides the file, which overrides defaults.
- Configurable babelmap home directory.

---

## Installation & usage

Requires a Rust toolchain.

```bash
# Build
cargo build --release

# Run a story
cargo run --release -p app -- path/to/story.z5
# or, after building:
./target/release/babelmap path/to/story.z5
```

Press the hotkey prefix (default `Ctrl+K`) in-app to see the full, grouped list
of commands and their bindings.

### Configuration

babelmap reads `~/.babelmap/config.toml` (override the directory with
`--user-dir`, or point at a specific file with `--config`). Every setting has a
default, so the file is optional. Command-line flags take precedence over the
config file, which takes precedence over built-in defaults.

---

## Development

```bash
cargo build --workspace          # build everything
cargo test --workspace           # run the full test suite
cargo run -p zvm-cli -- story.z5 # headless VM runner (no map)
```

The crates are layered `zvm` → `mapper` → `app`; `zvm-cli` is a thin VM
front-end. The mapper has no dependency on the VM, so layout logic can be tested
in isolation.
