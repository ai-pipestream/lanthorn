# Interface: navigation, playing aids & story picker

[← back to README](../../README.md)

## Map navigation & inspection
- **Mouse support** — click a room for a story-info panel (name, notes, exits,
  objects); right-click for layout diagnostics; middle-drag to pan.
- **Mouse wheel** pans the map (Shift = horizontal, Ctrl = zoom) and scrolls
  every scrollable surface — the transcript and the lists inside modals (saves,
  file browser, gallery, hotkey dialog, …).
- **Select & copy text** — left-drag over the story pane to select transcript
  text (highlighted as you drag); release copies it to your system clipboard via
  the OSC 52 terminal escape, so it works over SSH with no clipboard library.
  Each row is clamped to the story pane's columns, so a selection never grabs the
  map alongside the text.
- **Room inspector** overlay — id, name, layer, position, and per-edge
  dropped-constraint flags for understanding layout decisions.
- Pane focus with clear visual highlighting; Tab / Shift-Tab cycle the layout
  (split, map-only, transcript-only).

## Debug inspector (Z-machine)

`/debug` turns the map pane into a live **Z-machine debug inspector** — a
built-in debugger that follows the running story in real time.

![The debug inspector: live disassembly, call stack, and opcode hover help](../debug-inspector.png)

- **Live disassembly** that tracks the program counter, with a `PC` divider
  marking the next instruction to execute.
- **Tabbed views** across three windows: Disassembly / Globals · Locals /
  Objects / Dictionary · Call Stack / Stack / Memory — Tab moves between windows,
  `↔` switches the section within a window, `↑`/`↓` scroll.
- **Opcode hover help** — hover an instruction and a tooltip decodes the opcode
  and its operands (what each argument is, where the result lands).
- **Clickable operands** — addresses in the disassembly are underlined and jump
  to their target; `g` recenters on the PC, `r` toggles a full-width view.
- Select-and-copy works inside the inspector just like the transcript. `Esc`
  closes it and restores the map.

## Playing aids
- **Verb/noun menu** — a two-pane token palette of common verbs and in-scope
  nouns; pick tokens to build a command (multi-noun via prepositions).
- **Tab autocomplete** from the story's dictionary plus nouns mentioned in the
  current room. A live suggestion line shows the candidates with the active one
  bracketed: **Tab** cycles forward and **Shift-Tab** backward, the bracket
  always tracks the word currently on the command line, and the line scrolls
  horizontally to keep the highlighted candidate visible when the list overflows
  the width.
- **Command history** — press **↑**/**↓** at the prompt to recall and re-run
  previous commands, shell-style. History persists across sessions inside the
  `.babelmap` archive; disable recording with `record_history = false`.
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
  app commands by name: `/save-state`, `/restore-state`, `/reset-game [map]`,
  `/pan-map <dx> <dy>`, `/zoom-map in|out|reset`, `/center-map`, `/tidy-map`,
  `/cycle-layer next|prev`. `/help` lists all commands grouped by category;
  `/help <command>` shows one command's usage and description. Tab autocomplete
  over the names and quiet status-line feedback.
- **Transcript search / filter / export** — `/search <query>` highlights matches
  (case-insensitive) and lands on the most recent; `n`/`N` step back/forward
  (configurable), `Esc` clears. `/filter story|meta|both` shows only game output
  (including your commands), only app/engine output, or both. `/export-transcript
  [file]` writes the visible transcript to `transcript.txt` in the story's
  per-game directory by default (overwriting); a bare name lands beside it, a
  path-bearing value is honored verbatim — see [Storage layout](../persistence.md#storage-layout-sq-0284).
  Every transcript line carries a category — **story**, your **input** echo,
  **meta** (app/slash), and VM **warnings** — each independently themeable;
  meta and warning lines are set off with their own configurable gutter markers
  (`▏` / `!`).
- **Map export** — `/export-svg [file]`, `/export-dot [file]`, and
  `/export-dump [file]` write the map as SVG, a Graphviz DOT graph, or an
  annotatable text/ASCII dump. Each defaults to a fixed name in the story's
  per-game directory (`map.svg` / `map.dot` / `map.txt`, overwriting); the
  optional `[file]` argument resolves the same way as the transcript export.

## Story picker
Launching with a directory instead of a story file (`babelmap path/to/stories/`)
opens a picker: each row shows the title/filename plus right-aligned badges —
story type (**Z**/**G**) and present artifacts (bundled/sibling **B**lorb,
existing **S**ave, available **H**int file). The list is sortable by title,
author, or year — click a column header (or press `s`/`d`) to change the
column/direction. `i` or `Tab` slides in a themeable info side-panel for the
highlighted story (format/version/release/serial, IFID, author/year/genre,
a blurb, feature flags, bundled resources, and saves), animated per the
`animation` config and closed by default each launch; it refuses to open on
terminals too narrow for both list and panel. The badge glyphs are configurable
in `[symbols]` (`badge_zcode`/`badge_glulx`/`badge_blorb`/`badge_save`/`badge_hint`),
and the badge cluster, sortable column headers, and info panel are themeable via
the `story_badge`, `story_header`/`story_header:active` (the active sort
column), `story_author`, `story_year`, `story_no_metadata` (the "(no metadata
yet)" placeholder), `story_tile`/`story_tile:selected` (the cover-gallery
captions), and `story_info` (`:title`/`:label`/`:value`/`:blurb`/`:cover`)
style selectors. `↑`/`↓`/`j`/`k`/PgUp/PgDn/Home/End
navigate, `Enter` or a click opens the story, `q`/`Esc` quits back to the shell. When
the panel is open and its content overflows — including a long, word-wrapped
blurb — scroll it with the mouse wheel over the panel or `Shift`+`↑`/`↓`/PgUp/PgDn
(plain arrow/PgUp/PgDn keep navigating the list); the scroll position resets
whenever the highlighted story changes.

- **Metadata fetch (IFDB).** Press `f` to fetch author/year/genre/description/
  cover art for the highlighted story from IFDB, or `r` to sweep every story in
  the library (skipping any already at the current fetch version); `Esc`
  cancels a running sweep. Results are cached in a per-game sidecar so a repeat
  `r` makes no network requests. A blorb's own `IFmd`/`Fspc` always take
  precedence over fetched data.
- **Cover art in the story picker.** Blorb games with a frontispiece show their
  cover image in the picker's info panel, using the terminal's best graphics
  protocol (Kitty / iTerm2 / Sixel) with a universal half-block fallback. A
  story with no frontispiece of its own shows a fetched cover instead, once
  metadata has been fetched. Force a mode with
  `--image-protocol <auto|halfblocks|kitty|sixel|iterm2>`.
- **Preview bundled assets.** In the info panel's Resources list, image (`Pict`)
  and sound (`Snd`) rows render as links — click one to pop a dismissible modal:
  an image renders fitted and centred; a sound plays once. Close it with `Esc`/
  `Enter`/`q`, the ✕ in the corner, the Close button, or a click outside.
  Undecodable images and a missing audio device show a short status line instead.
- **Cover gallery.** Press `g` to swap the metadata list for a grid of cover
  thumbnails (auto-fitting as many ~16-column tiles as the pane is wide, each
  with the title captioned below and the selected cover highlighted). Arrow
  keys or `h`/`j`/`k`/`l` move a 2D cursor, PgUp/PgDn jump a screen of rows,
  the wheel scrolls a row at a time, and a click (or second click) selects (or
  opens) a cover. The info panel still toggles independently with `i`/`Tab`,
  and `g` returns to the list; the selection carries across both views.
- **In-game graphics (Glulx).** Games that open graphics windows now render
  their filled shapes and images in the terminal, using the best graphics
  protocol (Kitty / iTerm2 / Sixel) with a half-block fallback. Disable all
  image rendering (in-game graphics *and* cover art) with `--no-images`.
- **Inline images in text.** Glk inline images placed in a text-buffer window
  (the main transcript or another buffer window) render as full-width blocks
  right in the flow of text, honoring the terminal's best graphics protocol
  (Kitty / iTerm2 / Sixel) with a half-block fallback, and scroll along with
  the surrounding text. Themeable via the `inline_image` style selector.
