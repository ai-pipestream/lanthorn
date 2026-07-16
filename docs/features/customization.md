# Customization & configuration

[← back to README](../../README.md)

## Customization
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
- **Configurable status bar** — the `[statusbar]` section builds the status line
  from templated segments assigned to a left / center / right cluster, each with
  its own style. Templates substitute live `{placeholder}` values — `{location}`,
  `{score}`, `{moves}`, `{time}` — so you can compose exactly the readout you want
  (e.g. `Score: {score}  Moves: {moves}`) instead of a fixed layout.
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
  `meta_marker` / `warning_marker` selectors. On top of all that, the game's own
  **`set_text_style`** emphasis (bold / italic / reverse-video) is rendered
  per-span — a bold word inside a sentence shows just that word bold — layered
  over the category/rule colors and preserved across save/reload.
- **Tmux-style leader keymap**: a configurable prefix (default `Ctrl+K`) pops up
  a **reference panel** listing every command with an assigned single letter;
  pressing that letter runs the command and returns to normal — one keypress,
  then the panel closes (any unbound key or `Esc` just closes it). A small
  always-active set stays live outside the panel and is advertised in the bottom
  hint bar: Tab (focus), `Ctrl+S`/`Ctrl+R` (save/restore state), quit, and — in
  map focus — pan/zoom/select-room/center navigation. Leader letters are set per
  group under `[[hotkeys.group]]` in `config.toml` (`commands = ["t tidy-map",
  …]`; a bare `"tidy-map"` auto-assigns the first free letter), and the letter's
  color is themeable via the `hotkey:key` style selector. Direct key bindings
  still live in `[keymap.global]`, `[keymap.map]`, and `[keymap.anim]` as
  `"key" = "command args"` (each value a slash-command string the key runs); set
  `use_defaults = false` under `[keymap]` to clear the built-ins and define your
  own from scratch.
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
  Per-game looks: use the style editor's **Save Game Style** button to write the
  live look to the game's own `style.toml` in its save directory
  (`<base>/<story-key>.save/style.toml`); it layers over the global `style.toml`
  for that game only (including its own statusbar / transcript rules) and is
  re-applied every time that story opens. A per-game field left at the terminal
  default stays default on reload instead of silently re-inheriting the global
  colour.
- **Decorated panes** — configurable per-pane borders (`none`/`single`/`double`/
  `thick`/`rounded`). The map and story panes both default to a single-line
  border. The map's top border carries
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

## Configuration
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
