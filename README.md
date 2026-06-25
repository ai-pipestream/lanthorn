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
- **v4+ upper-window screen model** — cursor-addressed status lines and forms
  (e.g. Bureaucracy's licence application) render in a fixed grid atop the
  transcript, and `read_char` keystrokes are forwarded so forms are fillable in
  place. The game sees a fixed, configurable virtual screen
  (`virtual_screen_cols`/`virtual_screen_rows`, default 80×24); the viewport
  auto-follows the cursor when the pane is smaller. The virtual window is
  themeable (`upper_window`, `upper_window_border`, `virtual_window_border`).
  During a `read_char` prompt keystrokes go to the game; the hotkey prefix
  (default `Ctrl+K`) stays reserved.
- **Sound effects** — the `sound_effect` opcode's two built-in bleeps (high #1 /
  low #2) flash the story-pane border in distinct, themeable colors
  (`sound_beep_high` / `sound_beep_low`); a brief one-shot fade. (Sampled sounds
  need Blorb audio, still on the roadmap.) Unimplemented-opcode warnings surface
  in the transcript as meta lines (hidden by `/filter story`) rather than on
  stderr.

### Live automapping
- **Automatic room placement** as you explore — each new location is positioned
  relative to where you came from.
- **v4+ room detection** — for v4/v5 games that don't expose the room in the
  classic v3 status variable (Hitchhiker, Bureaucracy, A Mind Forever Voyaging),
  the room is read from the status line and resolved to a game object — preferring
  the player object's room when the game re-parents the player (Inform), falling
  back to a name-only room otherwise. A hideable indicator in the map's
  bottom-right corner shows how the current room was found (`toggle-loc-method`,
  persisted via `show_loc_method`; styled by `loc_indicator`): `via player
  object`, `via name match`, `via name (unlinked)`, or `via status variable`.
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
- **In-game hints** — `/hint` opens a modal that runs a companion *Invisiclues*
  `.z5` in a second Z-machine session (the main game pauses): navigate its
  progressive hint menu, `Esc` to close. The hint file is auto-detected next to
  the story (or inside a sibling `.zip`) and remembered per game; if the story
  has its own `HINT` command, the panel suggests that too. (Adventures and hint
  files packaged in `.zip` archives are supported.)
- **Reset** — restart the story from the beginning via a confirmation dialog with
  an opt-in "also clear the map" checkbox (the map is kept by default).
- **Slash commands** — type a leading prefix (default `/`, configurable) to run
  app commands by name: `/save`, `/load`, `/reset [map]`, `/panh`, `/panv`,
  `/zoom`, `/center`, `/tidy`, `/layer`, plus every command by its kebab name.
  `/help` lists them, with Tab autocomplete over the names and quiet status-line
  feedback.
- **Transcript search / filter / export** — `/search <query>` highlights matches
  (case-insensitive) and lands on the most recent; `n`/`N` step back/forward
  (configurable), `Esc` clears. `/filter story|meta|both` shows only game output
  (including your commands), only app/engine output, or both. `/export [file]`
  writes the visible transcript to a text file (auto-named under
  `~/.babelmap/exports/` by default). Every transcript line carries a category —
  **story**, your **input** echo, **meta** (app/slash), and VM **warnings** — each
  independently themeable; meta and warning lines are set off with their own
  configurable gutter markers (`▏` / `!`).

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
  and box styles (rounded / thick / double / **solid** / **super-thick** block
  frames / ascii / borderless). Arrow presets include Nerd Font Material Design
  families (bold / box / circle / outline, with corner arrows) and portals include
  a distinct 4-icon stairs set. Pick presets or override individual glyphs.
- **Symbol gallery** — a live-preview modal for browsing and combining symbol
  presets: category tabs across the top (←→), the options list below (↑↓), and a
  rendered preview of the current combination; saved back to your config.
- **Room numbers** — room id numbers are hidden by default (portal icons take the
  freed bottom row); toggle them with the `toggle-room-numbers` command, persisted
  via the `show_room_numbers` setting.
- **Color schemes** — recolor rooms, connectors, and chrome from a
  [Ghostty](https://ghostty.org) theme file or a built-in (mono / high-contrast /
  tomorrow-night), with per-element overrides. Defaults to your terminal colors.
- **Transcript text styling** — color each transcript category independently via
  the `transcript`, `transcript:input`, `transcript:meta`, and `transcript:warning`
  selectors (`fg`/`bg`/`bold`/`italic`). Story lines also run through styling rules:
  built-in ones for the room-name **location** header (`transcript:location`) and
  bracketed **system** lines such as `[Your score just went up.]`
  (`transcript:system`), plus your own ordered `[[transcript.rule]]` regex rules in
  `style.toml` (e.g. paint every `grue` red). The meta/warning gutter glyphs come
  from the `gutter.meta` / `gutter.warning` symbol overrides and are colored by the
  `meta_marker` / `warning_marker` selectors.
- **Configurable keymap** via a leader-key model: a configurable prefix
  (default `Ctrl+K`) opens a sticky **hotkey dialog** listing every command;
  any command can be made directly available or routed through the dialog.
- **Shareable style files** — all visual settings (colors + symbols) live in a
  standalone `style.toml`, referenced from `config.toml` by `style = "<name or
  path>"` (the file is the base; `config.toml` sections override per-key). Colors
  use a CSS-ish element→properties format (`fg`/`bg`/`bold`/…). Customizing in
  the gallery or config screen writes your personal `~/.babelmap/style.toml`, and
  the gallery can export a self-contained style file to hand to someone else.
- **Decorated panes** — configurable per-pane borders (`none`/`single`/`double`/
  `thick`/a notched **picture-frame**). The map defaults to the picture-frame; the
  story pane defaults to a single-line border. The map's top border carries
  a centered **layer-tab strip** (active layer highlighted); the story's top
  border shows the **adventure title** (taken from an override, the game's opening
  banner, or the filename). The status line and input prompt can be boxed too —
  all via `style.toml`.
- **Unified dialogs** — every modal (gallery, saves, file browser, config screen,
  verb menu, hotkey dialog, room/diagnostics panels) shares one themeable chrome:
  a bordered, titled, opaque frame with a clickable **✕**, mouse-clickable
  buttons, and an optional **drop-shadow**. The confirm button (OK / Save) is
  **underlined** and starts focused, so **Enter** triggers it; **Tab** / **Shift-Tab**
  (and **←** / **→** on the confirm dialogs) cycle focus through the other buttons
  (the focused one is highlighted) and Enter then fires whichever is focused. `Esc` and **✕** always close. Text-entry modals
  keep **Enter** = submit the field; the navigation panels (verb menu, file browser)
  keep their own keys and just show the default button underlined. Colors/border
  style are configurable under the `dialog*` style selectors.

### Configuration
- TOML config at `~/.babelmap/config.toml` plus command-line flags
  (`--user-dir`, `--config`); CLI overrides the file, which overrides defaults.
- **Virtual screen size** — `virtual_screen_cols` / `virtual_screen_rows`
  (default 80 × 24) set the fixed screen dimensions reported to the game; v4+
  cursor-addressed games (forms, status displays) want a roomy story pane.
- **In-app config screen** (`F2`) — a settings modal for the common options
  with an explicit Save (writes the config file, format-preserving) and Cancel;
  changes apply live.
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
