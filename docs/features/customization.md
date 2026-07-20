# Customization & configuration

[← back to README](../../README.md)

## Customization

### Styling model — roles, panels, and Glk styles
`style.toml` is built on a small **role palette** that everything else derives
from, so a theme author sets a handful of colors and the whole app stays
coherent, while power users can still override any single selector.

- **7 roles** (`[roles]`) are the roots a theme actually sets: `text` (body ink),
  `chrome` (ink on a UI surface — bars/panels/upper window), `line`
  (lines, frames, rules, dividers), `accent` (highlights — links, selection,
  current room, tabs), `muted` (dim/secondary text), `alert` (warnings/errors),
  and `heading` (emphasized titles). Everything else is a **derivation** —
  `parent = "<role>"`
  plus an optional delta (fg/bg/bold/italic/underline/dim/reversed) — so a
  minimal theme that only touches `[roles]` still looks fully coherent.
- **Panels vs. windows.** *Panels* are the frames babelmap itself draws — the
  story pane, map, verb menu, debug inspector, and every dialog/overlay.
  *Windows* are the surfaces the story/VM generates (Glk buffer/grid/graphics
  windows, the v4+ upper window). Panels are host chrome and never honor game
  colors; windows do, subject to the resolution chain below. Every panel shares
  **one** border under `[panel]` instead of a per-panel selector: `panel.border`
  when unfocused, `panel.border:active` when it has focus (bold by default —
  today's cyan+bold focus highlight), `panel.background` for the body fill, and
  `panel.title` / `panel.tab` / `panel.tab:active` / `panel.tab_divider` /
  `panel.terminator_left` / `panel.terminator_right` for the title/tab strip
  inset in the top border (the debug inspector's window tabs and the map's
  layer tabs render through these same selectors). The map additionally sets
  its own canvas fill, `map.background`, since it isn't a Glk window.
- **The 11 standard Glk styles** — Normal, Emphasized, Preformatted, Header,
  Subheader, Alert, Note, BlockQuote, Input, User1, User2 — are first-class,
  addressable selectors under `[glk.buffer]` (text-buffer windows) and
  `[glk.grid]` (text-grid/status windows), each carrying fg/bg plus
  bold/italic/underline/reversed. Each style defaults to a role-derived look (a
  game that sets no styles renders identically to a role-only theme) but can be
  overridden per slot for full Glk fidelity. A window's background is, by
  definition, its `glk.<type>.normal.bg` (defaults to `text.bg` for buffer
  windows, `chrome.bg` for grid windows) — there's no separate background knob.
- **`[map]`** owns every map-domain selector: colors (`room`, `room_current`,
  `room_selected`, `connector`, `connector_distorted`, `connector_portal`,
  `shared_path`, `layer_cycle`, …) and the glyph-set presets that used to live
  in a standalone `[symbols]` section — `box_style` (rounded / thick / double /
  **solid** / **super-thick** / ascii / borderless), `arrow_set` (including Nerd
  Font Material Design families), `portal_icons` (including a 4-icon stairs
  set), `path_style` for cardinal (N/S/E/W) connectors, and a separate
  `portal_path_style` for vertical/portal (up/down/in/out) connectors so they
  can render distinctly (e.g. dotted). Individual glyphs are overridden with a
  `glyphs` sub-map on the selector they belong to — `glyphs = { tl = "+" }` on
  `map.room` for box corners, `glyphs = { north = "^" }` on `map.connector` for
  arrows — there is no separate override table.
- **`[debug]`** holds only the disassembly-specific selectors for the debug
  inspector (`pc` and the SQ-0428 confidence tiers `disasm_executed`
  / `disasm_rd` / `disasm_soft` / `disasm_data`); each tier carries both a line
  style and a gutter **`glyph`** (e.g. `disasm_executed`'s `|` mark), so the
  color and the mark are both themeable. The panel's frame/body/tabs come from
  the shared `[panel]` chrome above, and its opcode hover tooltip from the shared
  `[tooltip]` surface (below), not from `[debug]`.
- **Surfaces beyond `[panel]`.** Dialogs and tooltips are their own **surface**
  sections — a background + optional frame + the text on them — separate from
  `[panel]`. `[dialog]` styles the modal surface (`background`, its own `border`
  frame, `title`, `button` / `button:active`, `shadow`); `[tooltip]` styles every
  hover tooltip (`background` + an optional `border`, borderless by default). Keys
  in these sections are bare (`title = { parent = "accent" }`), like `[panel]`
  keys.

### Everyday customization
- **Room numbers** — room id numbers are hidden by default (portal icons take the
  freed bottom row); toggle them with the `toggle-room-numbers` command, persisted
  via the `show_room_numbers` setting.
- **Color schemes** — recolor rooms, connectors, and chrome from a
  [Ghostty](https://ghostty.org) theme file or a built-in (mono / high-contrast /
  tomorrow-night), with per-role and per-selector overrides. Defaults to your
  terminal colors. `print-colors` prints the active, resolved scheme to the
  transcript (`print-colors color` also renders each entry in its own color).
- **Configurable status bar** — the `[statusbar]` section builds the status line
  from templated segments assigned to a left / center / right cluster. Each
  segment can set its own style directly, or ride a role via `parent = "accent"`.
  Templates substitute live `{placeholder}` values — `{location}`, `{score}`,
  `{moves}`, `{time}`, `{turns}`, `{title}`, `{filter}` — so you can compose
  exactly the readout you want (e.g. `Score: {score}  Moves: {moves}`) instead
  of a fixed layout.
- **Animations** — smooth, eased transcript scrolling instead of instant jumps,
  configured under `[animation]` in `config.toml` (`enabled`, `easing`,
  `scroll_ms`). Set `enabled = false` for instant scrolling.
- **Transcript text styling** — color each transcript category independently via
  bare selectors — `transcript`, `transcript_input`, `transcript_meta`,
  `transcript_warning`, `transcript_system`, `transcript_crash` (`fg`/`bg`/
  `bold`/`italic`). Story lines also run through styling rules: built-in ones for
  the room-name **location** header (`transcript_location`) and bracketed
  **system** lines such as `[Your score just went up.]` (`transcript_system`),
  plus your own ordered `[[transcript.rule]]` regex rules in `style.toml` (e.g.
  paint every `grue` red). The meta/warning gutter glyphs are now the `glyph`
  attribute directly on `transcript_meta` / `transcript_warning` (e.g.
  `transcript_meta = { parent = "muted", glyph = "▏" }`) rather than a separate
  symbol override. On top of all that, the game's own **`set_text_style`**
  emphasis (bold / italic / reverse-video) is rendered per-span — a bold word
  inside a sentence shows just that word bold — layered over the category/rule
  colors and preserved across save/reload.
- **Tmux-style leader keymap**: a configurable prefix (default `Ctrl+K`) pops up
  a **reference panel** listing every command with an assigned single letter;
  pressing that letter runs the command and returns to normal — one keypress,
  then the panel closes (any unbound key or `Esc` just closes it). A small
  always-active set stays live outside the panel and is advertised in the bottom
  hint bar: Tab (focus), `Ctrl+S`/`Ctrl+R` (save/restore state), quit, and — in
  map focus — pan/zoom/select-room/center navigation. Leader letters are set per
  group under `[[hotkeys.group]]` in `config.toml` (`commands = ["t tidy-map",
  …]`; a bare `"tidy-map"` auto-assigns the first free letter), and the letter's
  color is themeable via the `hotkey_key` style selector. Direct key bindings
  still live in `[keymap.global]`, `[keymap.map]`, and `[keymap.anim]` as
  `"key" = "command args"` (each value a slash-command string the key runs); set
  `use_defaults = false` under `[keymap]` to clear the built-ins and define your
  own from scratch.
- **Decorated panes** — configurable per-pane borders (`none`/`single`/`double`/
  `thick`/`rounded`) via the shared `[panel]` chrome above: unfocused panels use
  `panel.border`, the focused one uses `panel.border:active`. The map's top
  border carries a centered **layer-tab strip** (active layer highlighted, via
  `panel.tab:active`); the story's top border shows the **adventure title**
  (taken from an override, the game's opening banner, or the filename). The
  status line and input prompt can be boxed too — all via `style.toml`.
- **Unified dialogs** — every modal (saves, file browser, config screen, verb
  menu, hotkey dialog, room/diagnostics panels) shares one themeable chrome:
  a bordered, titled, opaque frame with a clickable **✕**, mouse-clickable
  buttons, and an optional **drop-shadow**. The confirm button (OK / Save) is
  **underlined** and starts focused, so **Enter** triggers it; **Tab** / **Shift-Tab**
  (and **←** / **→** on the confirm dialogs) cycle focus through the other buttons
  (the focused one is highlighted) and Enter then fires whichever is focused. `Esc` and **✕** always close. Text-entry modals
  keep **Enter** = submit the field; the navigation panels (verb menu, file browser)
  keep their own keys and just show the default button underlined. Colors are
  configurable under the `[dialog]` surface section — `background`, `border` (the
  dialog's own frame), `title`, `button` / `button:active`, and `shadow` — and a
  modal's on-screen **placement** — centered (default) or anchored to any edge or
  corner with a margin — via `[dialog]`'s `placement` / `margin` keys.

### Editing your theme
All visual settings live in a standalone `style.toml`, referenced from
`config.toml` by `style = "<name or path>"` (the single styling source —
`config.toml` no longer carries style). There is no in-app editor: on first
run, if you have no `style.toml`, babelmap writes one to your user directory
**fully commented out** — every selector present, grouped by section (roles,
panels, Glk styles, map, debug, transcript rules, status bar), with a short
explanatory comment per section and every commented line already equal to the
built-in default. It never overwrites an existing file. Edit it, uncomment the
lines you want to change, save, and run **`reload-style`** to apply the change
live (a syntax error keeps the current look and warns instead of crashing); set
`watch_style = true` in `config.toml` (or run **`toggle-watch`**) to
auto-reload on every save instead. `style.example.toml` at the repo root is
generated from the same registry, so it always matches the seeded template.

**Per-game looks**: drop a `style.toml` into the game's own save directory
(`<data-base>/<story-key>.save/style.toml` — the same folder as its saves and
`map.txt`) to layer overrides on top of the global theme for just that game;
it's re-applied every time that story opens. There's no "Save Game" button —
you write the file directly.

**Resolution order**, most specific first: an *explicit* user per-game slot →
a garglk per-stream override → the game's own live style hints → the
`glk.*` slot (global theme, defaults, and any shipped `garglk.ini`) → that
slot's role → your terminal colors. The `honor_game_colours` setting gates the
two game-driven layers (per-stream override and live style hints); turn it off
to have your theme own every color regardless of what the game requests — see
[interpreter](interpreter.md) for the game-colour toggle itself. An explicit
per-game slot always wins over the game, even with game colours honored.

**Schema note (pre-release, breaking):** the `style.toml` schema described
above is new. An old-schema file (with top-level `[colors]` / `[symbols]`
sections) is left untouched — it is not auto-migrated or overwritten — but its
sections no longer apply; regenerate by deleting it and letting babelmap
re-seed the new template, or hand-write the new shape from
`style.example.toml`.

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
