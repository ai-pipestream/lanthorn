# Customization & configuration

[← back to README](../../README.md)

## Customization

Almost every pixel babelmap paints is yours to repaint. Colours, borders, box
glyphs, the status line, the keymap, even the easing curve on a scroll — all of
it lives in two plain TOML files you can edit and reload without leaving the
game. This page walks the knobs from the ones you'll reach for first to the ones
that let you rebuild the whole look from scratch.

### Styling model — roles, panels, and Glk styles
Set seven colours and you've themed the entire app. That's the payoff of
`style.toml`'s **role palette**: a handful of roots that everything else derives
from, so a coherent theme falls out of almost no typing — while power users can
still reach in and override any single selector by name.

- **7 roles** (`[roles]`) are the roots a theme actually sets: `text` (body ink),
  `chrome` (ink on a UI surface — bars/panels/upper window), `line`
  (lines, frames, rules, dividers), `accent` (highlights — links, selection,
  current room, tabs), `muted` (dim/secondary text), `alert` (warnings/errors),
  and `heading` (emphasized titles). Everything else is a **derivation** —
  `parent = "<role>"`
  plus an optional delta (fg/bg/bold/italic/underline/dim/reversed) — so a
  minimal theme that only touches `[roles]` still looks fully coherent.
- **Panels vs. windows.** *Panels* are the frames babelmap itself draws — the
  story pane, map, command band, debug inspector, and every dialog/overlay.
  *Windows* are the surfaces the story/VM generates (Glk buffer/grid/graphics
  windows, the v4+ upper window). Panels are host chrome and never honor game
  colors; windows do, subject to the resolution chain below. Every panel shares
  **one** border under `[panel]` instead of a per-panel selector: `panel.border`
  when unfocused, `panel.border:active` when it has focus (bold by default —
  today's cyan+bold focus highlight), `panel.background` for the body fill, and
  `panel.title` / `panel.tab` / `panel.tab:active` / `panel.tab_divider` /
  `panel.terminator_left` / `panel.terminator_right` for the title/tab strip
  inset in the top border (every framed pane — story, map, dialogs, the command
  band and inventory dock, the debug inspector's window tabs, the story-list info
  panel — renders through this one shared panel component and these same
  selectors). The story pane's strip text is the resolved adventure title,
  with the story's filename appended in parentheses when it differs from the
  title (e.g. `Journey: The Quest Begins (journey-r83-s890706.z6)`) — a bare
  filename with no known title (or a file already named after it) shows with
  no parenthetical. The title is the *same* one the story browser lists, drawn
  from the same metadata in the same order — a blorb's own iFiction record, then
  the fetched IFDB details cached beside your saves, then babelmap's bundled
  title tables — so a game can't be *Anchorhead* in the library and `anchor` in
  the pane. The game's opening banner is consulted only after all of those, and
  the filename is the last resort it was always meant to be. A story mounted off
  a **disk image** always names its `.adf` there, however neatly the box-spelled
  filename matches the title: a floppy carries a different *release* of the game,
  and which one you're playing is exactly what the border should tell you.
  The strip's bracket caps and divider track the pane's border
  style by default (`┤ … ├` on single, `┫ … ┣` on thick, `╡ … ╞` on double);
  set `panel.terminator_left` / `panel.terminator_right` / `panel.tab_divider`
  to a `glyph` to override any of them. The map additionally sets
  its own canvas fill, `map.background`, since it isn't a Glk window.
  The one *window* frame babelmap can draw itself is the box around a v4+ game's
  status/upper window, and it answers to `upper_window_border` in `[elements]` —
  the selector that colours it carries its shape too. It is **off by default**:
  the status line sits flush against the story, and the whole pane is the screen
  the game is told it has. Ask for a box with `style = "single"` (or
  `double`/`thick`/`rounded`), reach for `style_top` / `style_bottom` /
  `style_left` / `style_right` to rule one edge at a time, and remember that
  every side you turn on costs the story a row or a column. Don't reach for
  `[statusbar]`'s `border` for this — that frames babelmap's own status bar, not
  the game's window.
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
  can render distinctly (dotted by default). `diagonal_corners = false` turns
  the half-diagonal corner stubs (🮠🮡🮢🮣) back into plain orthogonal exits, for
  fonts without Unicode 13 Legacy Computing coverage. Individual glyphs are
  overridden one slot at a time in a `[map.overrides]` table keyed by slot name
  — `"room.normal.tl" = "+"`, `"arrow.north" = "^"`, `"path.diag_ul" = "/"`.
- **`[debug]`** holds only the disassembly-specific selectors for the debug
  inspector: `pc` and the four confidence tiers that shade how sure the
  disassembler is that a byte is really code. The defaults read as a risk
  gradient — **blue** verified, **yellow** medium, **red** high-risk:
  - **`disasm_executed`** (blue) — the line's address has *ever* run. Ground
    truth; it wins over any static guess and stays blue for the rest of the
    session (cumulative coverage). Its `|` gutter mark is separate: it flags only
    the lines that ran during the *last* command, so the bar tracks the most
    recent turn while the colour accumulates.
  - **`disasm_rd`** (yellow) — hard-discovered code: reached by recursive descent
    from a constant call target or the initial PC, or later confirmed by execution.
  - **`disasm_soft`** (red) — a linear-scan guess that hasn't been verified yet —
    the "don't fully trust this" tier.
  - **`disasm_data`** — bytes shown as `.byte`, not decoded as code at all
    (muted; it's not a risk level).

  Each tier carries both a line style and a gutter **`glyph`** (e.g.
  `disasm_executed`'s `|` mark; the others default to a blank space — set
  `disasm_soft = { glyph = "?" }` to flag guesses), so the colour and the mark
  are both themeable. The panel's frame/body/tabs come from the shared `[panel]`
  chrome above, and its opcode hover tooltip from the shared `[tooltip]` surface
  (below), not from `[debug]`.
- **Surfaces beyond `[panel]`.** Dialogs and tooltips are their own **surface**
  sections — a background + optional frame + the text on them — separate from
  `[panel]`. `[dialog]` styles the modal surface (`background`, its own `border`
  frame, `title`, `button` / `button:active`, `shadow`); `[tooltip]` styles every
  hover tooltip (`background` + an optional `border`, borderless by default). Keys
  in these sections are bare (`title = { parent = "accent" }`), like `[panel]`
  keys. The story picker's **IFDB search** modal (`/`) reuses this `[dialog]`
  chrome and adds five `[elements]` selectors for its contents: `ifdb_result` (a
  game/file row), `ifdb_result_selected` (the highlighted row, accent + bold +
  reversed), `ifdb_result_meta` (the rating/year tail and hint line),
  `ifdb_download_marker` (the ⭳ glyph on a download option), and
  `ifdb_attribution` (the "Results from IFDB" credit line). The **saves manager**
  adds two more for its Type column: `saves_portable` (accent, and its `glyph`
  supplies the `↗` mark on a save another interpreter can read) and
  `saves_host_only` (muted — a host snapshot that stays put).

### Everyday customization
Below the full role system sit the knobs most people actually touch — the small
switches that make babelmap feel like yours without opening the whole registry.

- **Room numbers** — room id numbers are hidden by default (portal icons take the
  freed bottom row); flip them on with the `toggle-room-numbers` command,
  persisted via the `show_room_numbers` setting.
- **Color schemes** — recolor rooms, connectors, and chrome from a
  [Ghostty](https://ghostty.org) theme file or a built-in (mono / high-contrast /
  tomorrow-night), with per-role and per-selector overrides. Defaults to your
  terminal colors — genuinely so: with no scheme set, babelmap asks the terminal
  for its own default foreground and background (OSC 10/11, at startup) and hands
  the answer to the `chrome` role, so the status bar, upper window and dialog
  surfaces sit on your terminal's page rather than a black one. A terminal that
  declines to answer, or answers only half, falls back to the built-in dark
  palette rather than mixing a real ink into a guessed page. `print-colors` prints
  the active, resolved scheme to the transcript (`print-colors color` also renders
  each entry in its own color).
- **Configurable status bar** — the `[statusbar]` section builds the status line
  from templated segments assigned to a left / center / right cluster. Each
  segment can set its own style directly, or ride a role via `parent = "accent"`.
  Templates substitute live `{placeholder}` values — `{location}`, `{score}`,
  `{moves}`, `{time}`, `{turns}`, `{title}`, `{filter}` — so you can compose
  exactly the readout you want (e.g. `Score: {score}  Moves: {moves}`) instead
  of a fixed layout.
- **Animations** — the transcript glides to its new position on an easing curve
  instead of snapping there. Tune it under `[animation]` in `config.toml`
  (`enabled`, `easing`, `scroll_ms`), or set `enabled = false` (or `scroll_ms =
  0`) to have every scroll land instantly. The same section holds the story
  pane's auto-hiding scrollbar: `scrollbar_hide_ms` (default 1500) is how long
  the bar stays up after you scroll — `0` keeps it up permanently — and
  `scrollbar_fade_ms` (default 300) how long it takes to fade away, `0` for a
  clean pop. Its two colours are yours as well: `scrollbar` paints the thumb and
  `scrollbar_track` the channel it runs in, both as background fills rather than
  glyphs, so nothing crowds the prose beside them.
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
- **Tmux-style leader keymap**: a configurable prefix (default `Ctrl+P`) pops up
  a **reference panel** of frequent map-editing verbs, each on a **mnemonic
  single letter** — `t`idy, `a`nimate, `p`eel, `m`erge, `c`ycle-layer, `r`ename
  room, `n`otes, `d`elete connection, `e`dge relabel, `i`nventory, portal
  `l`abels, `v`erb menu, `+`/`-` zoom, `0` centre map, `s`ettings, `h`istory,
  reset `g`ame — grouped as
  Layout / Layers / Edit / View / Map / Session. Pressing a letter runs the command and
  returns to normal — one keypress, then the panel closes (any unbound key or
  `q`/`Esc` just closes it; `q` is deliberately left unassigned so it closes).
  The long tail (exports, pane resizing, `rename-layer`, `toggle-map`,
  `toggle-inspector`, `toggle-alignment`, …) lives in the `/` command palette
  below rather than the panel. A small always-active set stays live outside the
  panel and is advertised in the bottom hint bar: `Ctrl+S`/`Ctrl+R`
  (save/restore state), quit, and `Shift+Arrow` to pan the map — all of which
  work while you type, since the map never takes the keyboard. Tab appears there only while the debug inspector is open, which is
  the one thing it still steps through.
  Leader letters are set per group under `[[hotkeys.group]]` in
  `config.toml` (`commands = ["t tidy-map", …]`; a bare `"tidy-map"` auto-assigns
  the first free letter), and the letter's color is themeable via the
  `hotkey_key` style selector. Direct key bindings
  still live in `[keymap.global]`, `[keymap.map]` (reached only while the debug
  inspector holds the right-hand pane; it ships no defaults of its own), and
  `[keymap.anim]` as
  `"key" = "command args"` — the **key on the left**, the command it runs on the
  right, spelled the way the registry spells it (hyphenated: `save-state`,
  `zoom-map in`). Bind one command to two keys by writing two entries. Get the two
  sides the wrong way round and the entry is skipped with a warning at game start
  that says so and quotes the corrected line. Set `use_defaults = false` under
  `[keymap]` to clear the built-ins and define your own from scratch.

  Two things worth knowing before you pick a key. A binding for a command outside
  the always-available `direct` set only fires from the story prompt, not from map
  focus and not with a Ctrl modifier — that set is what "available without opening
  the leader panel" means. And while a story is waiting on a single keypress
  (menus, "press any key"), every *plain* key goes to the game; only Ctrl and Alt
  combos are held back for babelmap. So a diagnostic you want reachable at any
  moment wants a Ctrl binding:

  ```toml
  [keymap.global]
  "ctrl+d" = "dump-windows"
  "ctrl+g" = "dump-cells"
  ```
- **Command palette** — press `/` at an empty prompt (or `/` inside the leader
  panel) to open a fuzzy search over every command; its rows theme via five
  `[elements]` selectors: `palette_query` (the input line), `palette_name` (a
  command name), `palette_match` (the fuzzy-matched characters, accent + bold by
  default), `palette_desc` (the one-line help), and `palette_selected` (the
  highlighted row). Its frame reuses the shared `[dialog]` chrome.
- **Command band** — the band's own parts theme via three `[elements]`
  selectors: `band.column_header` / `band.column_header:active`, `band.quick`
  (the one-click words, rose or flat row) and `band.group_label` (in-column
  labels and the `(nothing visible)` placeholder). Its rows — and the armed
  quick word — reuse `dialog.list_selected`; it draws no frame, and borrows
  `panel.border:active`'s colour for its whole fill while resize mode is
  targeting it.
- **Decorated panes** — configurable per-pane borders (`none`/`single`/`double`/
  `thick`/`rounded`) via the shared `[panel]` chrome above: unfocused panels use
  `panel.border`, the focused one uses `panel.border:active`. The map's top
  border carries a centered **layer-tab strip** (active layer highlighted, via
  `panel.tab:active`); the story's top border shows the **adventure title**
  (taken from an override, the game's opening banner, or the filename). The
  status line and input prompt can be boxed too — all via `style.toml`.
- **Unified dialogs** — every modal (saves, file browser, config screen,
  hotkey dialog, room/diagnostics panels) shares one themeable chrome:
  a bordered, titled, opaque frame with a clickable **✕**, mouse-clickable
  buttons, and an optional **drop-shadow**. The confirm button (OK / Save) is
  **underlined** and starts focused, so **Enter** triggers it; **Tab** / **Shift-Tab**
  (and **←** / **→** on the confirm dialogs) cycle focus through the other buttons
  (the focused one is highlighted) and Enter then fires whichever is focused. `Esc` and **✕** always close. Text-entry modals
  keep **Enter** = submit the field; the navigation panels (file browser, saves)
  keep their own keys and just show the default button underlined. Colors are
  configurable under the `[dialog]` surface section — `background`, `border` (the
  dialog's own frame), `title`, `button` / `button:active`, and `shadow` — and a
  modal's on-screen **placement** — centered (default) or anchored to any edge or
  corner with a margin — via `[dialog]`'s `placement` / `margin` keys.

### Editing your theme
The file *is* the editor. All visual settings live in a standalone `style.toml`,
referenced from `config.toml` by `style = "<name or path>"` (the single styling
source — `config.toml` carries no style of its own). On first run, if you have no
`style.toml`, babelmap seeds one in your user directory **fully commented out**:
every selector is there, grouped by section (roles, panels, Glk styles, map,
debug, transcript rules, status bar), each with a short explanatory comment, and
every commented line already spelling out the built-in default — so the seeded
file is a working reference you edit in place, not a blank page. It never
overwrites an existing file. Uncomment the lines you want to change, save, and
run **`reload-style`** to see the change live (a syntax error keeps the current
look and warns you instead of crashing); flip `watch_style = true` in
`config.toml` (or run **`toggle-watch`**) and every save reloads on its own.
`style.example.toml` at the repo root is generated from the same registry, so it
always matches the seeded template.

**Per-game looks**: drop a `style.toml` into the game's own save directory
(`<data-base>/<story-key>.save/style.toml` — the same folder as its saves and
`map.txt`) to layer overrides on top of the global theme for just that game;
it's re-applied every time that story opens. There's no "Save Game" button —
you write the file directly.

**Per-game settings**: alongside that style file, a game's save directory can hold
its own `config.toml` — a separate, deliberately tiny sidecar carrying at most
`honor_game_colours`, `borderless_windows` and `show_map`. It is written for you
when you toggle one of those for a story (`/game-colours`, `/borderless`, hiding
the map), and it is a *sparse override layer*, not a copy of your global config:
bare uncommented lines, only the keys that differ, and the file is deleted once
nothing is overridden. An absent key means "inherit the global value" — which is
why babelmap never seeds the annotated template into a game directory, and why you
shouldn't either: every line you uncommented would become a per-game override
pinning that value for that story.

**Resolution order**, most specific first: an *explicit* user per-game slot →
a garglk per-stream override → the game's own live style hints → the
`glk.*` slot (global theme, defaults, and any shipped `garglk.ini`) → that
slot's role → your terminal colors. The `honor_game_colours` setting gates the
two game-driven layers (per-stream override and live style hints); turn it off
to have your theme own every color regardless of what the game requests — see
[interpreter](interpreter.md) for the game-colour toggle itself. An explicit
per-game slot always wins over the game, even with game colours honored.

**garglk.ini import**: if a `garglk.ini` (or `<story>.ini`) sits beside the
story, babelmap reads the section matching that game and imports what a terminal
can honor — its `tcolor`/`gcolor`/`linkcolor`/`bordercolor`/`windowcolor`
palette, `stylehint` (→ `honor_game_colours`), the text-window margins
(`tmarginx`/`tmarginy`, converted from pixels to character cells with a nominal
8×16 cell), and the inter-window border width (`wborderx`/`wbordery` → the
borderless-windows toggle: `0` → borderless). Colours layer per the resolution
order above. The text margin and border toggle are applied at runtime — nothing
is written back to any sidecar — and, consistent with `honor_game_colours`, an
explicit per-game `config.toml` value always wins over the garglk.ini (the text
margin has no per-game key today, so garglk overrides only your global default).

**Schema note (pre-release, breaking):** the `style.toml` schema described
above is new. An old-schema file (with top-level `[colors]` / `[symbols]`
sections) is left untouched — it is not auto-migrated or overwritten — but its
sections no longer apply; regenerate by deleting it and letting babelmap
re-seed the new template, or hand-write the new shape from
`style.example.toml`.

## Configuration
- TOML config at `~/.babelmap/config.toml` plus command-line flags
  (`--user-dir`, `--config`); CLI overrides the file, which overrides defaults.
- **The config file documents itself.** On first run babelmap seeds
  `config.toml` the same way it seeds `style.toml`: every setting it reads is
  listed, grouped and commented, with the value shown being the **default** — so
  the whole surface is browsable from the file instead of only from the source,
  and uncommenting a line as-is changes nothing. Where a default can't be written
  down (an unset path, or a value babelmap picks per story) the line is marked as
  an example, because uncommenting *that* does change behaviour. An existing
  config is never overwritten, and later edits from the settings screen preserve
  your comments.
- **A broken config file says so.** TOML is parsed as one document, so a single
  stray character — an unclosed quote, a stray bracket — costs you every setting
  in the file, not just the line it's on. The same is true of a value babelmap
  can't use (`volume = 300`, `auto_load = "yes"`): the file is valid TOML, but
  the *config* isn't, and it is dropped just as wholesale. babelmap names the
  file and shows the error at startup instead of quietly running on defaults,
  and it refuses to save settings over a file it couldn't read, so the text you
  need in order to find the mistake is never overwritten. Fix the file (or move
  it aside and let babelmap seed a fresh one) and saving resumes.
- **Settings are written atomically.** Every file babelmap owns — `config.toml`,
  saves and archives, the aux/VFS sidecars — is built beside its target and moved
  into place in one step, so a crash, a power cut, or a kill during a write leaves
  the previous file intact rather than a truncated one.
- **Default story directory** — `default_story_dir` is opened when babelmap is
  launched with no path argument. The first time you point babelmap at a
  directory on the command line without one set, it offers to remember that
  directory as the default (writing it to the config file); after that, a bare
  `babelmap` opens the story picker there. With no argument and no default set,
  babelmap prints how to fix it and exits.
- **Virtual screen size** — `virtual_screen_cols` / `virtual_screen_rows` pin the
  screen dimensions reported to the game. Leave them **unset** (the default) and
  babelmap reports the story pane's real measured size and re-reports it whenever
  you resize the terminal, so a v4+ game's cursor-addressed forms and status
  displays fill the pane and line up with the prose. Set one to reproduce a game's
  original fixed layout (say `virtual_screen_cols = 80`) — a pinned width narrower
  than the pane is drawn centred, and a pinned width wider than it scrolls to
  follow the cursor. Version 6 stories ignore both: they lay out on their own
  fixed pixel screen, which babelmap scales into whatever pane it has.
- `undo_levels` (default 16) — how many in-memory undo states the Z-machine
  keeps for the game's own UNDO command (0 disables undo).
- **Command band** — the `[command_band]` section configures the point-and-click
  phrase builder (see [Interface](interface.md#playing-aids); not to be confused
  with the unrelated top-level `command_bar` boolean, which moves the *typed*
  prompt into a persistent bar). `height` (default 5) is the band's rows — it
  draws no frame, so every one of them is content — clamped to 3–11 and to
  whatever the screen can spare;
  resize mode writes this key. `auto_open` (default false) opens the band with
  the story. `verbs` REPLACES the built-in verb table and `extra_verbs` adds to
  whichever table is in force — same entry shape either way,
  `{ word = "unlock", arity = "pair", prep = "with" }`, where `arity` is one of
  `solo` (complete on its own), `object` (one object, required), `object_opt`
  (one object, optional) or `pair` (two objects joined by `prep`, which also
  names that column). An `extra_verbs` entry whose word already exists re-shapes
  it rather than duplicating it, so that is how you fix one built-in verb's
  grammar. `quick` replaces the one-click quick-action row. An unrecognised
  `arity` is reported in the transcript and that entry is skipped, never
  silently reinterpreted.
- **v6 story rendering** — `v6_render` selects how graphical v6 titles (*Zork Zero*,
  *Shogun*, …) draw their story pane on an image-capable terminal: `hybrid`
  (the default) keeps the story text as real terminal text inside an image
  chrome ring; `raster` bakes the whole pane — frame, status, and story text —
  into one scaled pixel image instead; `frameless` drops the decorative frame
  entirely and shows the story as a normal full-pane terminal transcript (full
  size, native scrollback) with compact status/command bands pinned to the pane
  edges, any beside-the-story picture column, and inline pictures — the most
  legible mode, at the cost of the compass and border art. It also cycles
  in the settings screen, and `/set-v6-render` switches modes live mid-game
  (session-only) for quick comparisons. (Applies only to graphical v6 stories;
  other games are unaffected.) See [Graphical v6](v6-graphics.md) for the full
  picture.
- **v6 arrow keys** — `v6_arrow_keys` (default `true`) controls whether arrow
  keypresses are forwarded to a v6 story as movement input; set it `false` (in
  config.toml or the settings screen) to withhold them so arrows drive babelmap's own
  scrollback recall / map panning instead. Only v6 stories are affected — v1-5
  and Glulx games always get arrows. See [Graphical v6](v6-graphics.md#arrow-keys-movement-or-map-panning-your-call).
- **Story text margins** — `text_margin_x` / `text_margin_y` (default 0) reserve
  blank columns on each side / rows top and bottom *inside* the story text pane,
  for a little breathing room around the transcript. The margin applies to the
  text buffer only — the upper-window status line stays flush — and adjusts in
  the settings screen with `←` / `→`. A game's imported `garglk.ini` margin (below)
  overrides this default while that story is open.
- **In-app config screen** — pop the leader panel (default `Ctrl+P`) and press
  the `open-config` key for a settings modal covering the common options, with an
  explicit Save (writes the config file, comments and layout preserved) and
  Cancel; changes apply live.
- **Portable home** — everything babelmap keeps (config, style, saves, sidecars)
  lives under `~/.babelmap` by default; point `--user-dir` somewhere else to
  relocate the whole home, or `--data-dir` to split just the saves and sidecars
  off on their own.
