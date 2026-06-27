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
| `zvm-cli` | A standalone DOS-style command-line interpreter (no map): pinned v3 status line / v4+ upper window, save/restore, cross-session aux tables, single-key input, terminal-bell bleeps — and, piped, a clean deterministic harness for testing/scripting. |

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
- **Mouse wheel** pans the map (Shift = horizontal, Ctrl = zoom) and scrolls
  every scrollable surface — the transcript and the lists inside modals (saves,
  file browser, gallery, hotkey dialog, …).
- **Room inspector** overlay — id, name, layer, position, and per-edge
  dropped-constraint flags for understanding layout decisions.
- Pane focus with clear visual highlighting; Tab / Shift-Tab cycle the layout
  (split, map-only, transcript-only).

### Playing aids
- **Verb/noun menu** — a two-pane token palette of common verbs and in-scope
  nouns; pick tokens to build a command (multi-noun via prepositions).
- **Tab autocomplete** from the story's dictionary plus nouns mentioned in the
  current room. A live suggestion line shows the candidates with the active one
  bracketed: **Tab** cycles forward and **Shift-Tab** backward, the bracket
  always tracks the word currently on the command line, and the line scrolls
  horizontally to keep the highlighted candidate visible when the list overflows
  the width.
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
  app commands by name: `/save-game`, `/load-game`, `/reset-game [map]`,
  `/pan-map <dx> <dy>`, `/zoom-map in|out|reset`, `/center-map`, `/tidy-map`,
  `/cycle-layer next|prev`. `/help` lists all commands grouped by category;
  `/help <command>` shows one command's usage and description. Tab autocomplete
  over the names and quiet status-line feedback.
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
  `/print-colors` prints the active scheme to the transcript (optionally
  rendering each entry in its own color).
- **Live style editor** — a full-screen click-to-edit editor (`F3` or `/style`)
  for the entire theme: pick any element from a preview board, then set its
  foreground/background from a swatch grid (ANSI palette, custom hex, or
  terminal-default) and toggle bold / italic / underline / dim / reverse;
  bordered elements get per-side border types and per-zone glyph overrides via a
  glyph picker. Edits preview live. It is fully keyboard-navigable — **Tab** /
  **Shift-Tab** move between the fields and on through the **Save Global** /
  **Save Game** / **Cancel** buttons (each its own tab stop), **Enter** activates
  the focused button — and equally mouse-driven. Saving writes `style.toml` or a
  per-game style.
- **Animations** — smooth, eased transcript scrolling instead of instant jumps,
  configured under `[animation]` in `config.toml` (`enabled`, `easing`,
  `scroll_ms`). Set `enabled = false` for instant scrolling.
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
  Key bindings live in `[keymap.global]`, `[keymap.map]`, and `[keymap.anim]`
  sections of `config.toml` as `"key" = "command args"` — each value is a
  slash command string (with any arguments) that the key will run. Set
  `use_defaults = false` under `[keymap]` to clear all built-in bindings and
  define your own from scratch.
- **Shareable style files** — all visual settings (colors + symbols) live in a
  standalone `style.toml`, referenced from `config.toml` by `style = "<name or
  path>"` (the single styling source — `config.toml` no longer carries style). Colors
  use a CSS-ish element→properties format (`fg`/`bg`/`bold`/…). Customizing in
  the gallery writes your personal `~/.babelmap/style.toml`, and
  the gallery can export a self-contained style file to hand to someone else.
  See `style.example.toml` at the repo root for a fully-commented reference of
  every selector, the `[[transcript.rule]]` story rules, the `[statusbar]`
  segment bar, and the `[symbols]` overrides.
  Changes apply live: `/reload` re-reads `style.toml`, and `watch_style = true`
  in `config.toml` auto-reloads on save (`/watch` toggles it at runtime).
  Per-game looks: run `/game-style` to scaffold `~/.babelmap/styles/<ifid>.toml`,
  edit it, and `/reload` — it layers over `style.toml` for that game only
  (including its own statusbar / transcript rules). The watcher picks up the
  styles dir once it exists, so the very first file create may need one `/reload`.
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
  style are configurable under the `dialog*` style selectors, and their on-screen
  **placement** — centered (default) or anchored to any edge or corner with a
  margin — via the `dialog` selector's `placement` / `margin` keys.

### Configuration
- TOML config at `~/.babelmap/config.toml` plus command-line flags
  (`--user-dir`, `--config`); CLI overrides the file, which overrides defaults.
- **Virtual screen size** — `virtual_screen_cols` / `virtual_screen_rows`
  (default 80 × 24) set the fixed screen dimensions reported to the game; v4+
  cursor-addressed games (forms, status displays) want a roomy story pane.
- `undo_levels` (default 16) — how many in-memory undo states the Z-machine
  keeps for the game's own UNDO command (0 disables undo).
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
cargo run -p zvm-cli -- story.z5 # DOS-style CLI player (no map)
```

`zvm-cli` renders a basic DOS-style screen (pinned status line / upper window
via ANSI when interactive, clearing the screen on start) and degrades to a clean
line stream when piped. Interactively it does single-key input (arrow/function
keys decoded for `read_char` menus) and `[MORE]` paging on long output; aux save
tables persist per game by IFID. The flags `--no-status` (byte-identical
lower-stream output), `--no-aux`, and `--no-more` keep the headless test harness
deterministic.

The crates are layered `zvm` → `mapper` → `app`; `zvm-cli` is a thin VM
front-end. The mapper has no dependency on the VM, so layout logic can be tested
in isolation.
