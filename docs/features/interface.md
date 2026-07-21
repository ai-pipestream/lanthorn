# Interface: navigation, playing aids & story picker

[← back to README](../../README.md)

The map draws itself, but you still have to drive it. babelmap gives you a
mouse-driven, copy-anything, keyboard-fast terminal cockpit for reading the map,
inspecting the machine, and firing commands — without ever leaving the story.

## Map navigation & inspection
- **Mouse support** — left-click a room to pop its info panel (name, notes,
  exits, objects); right-click a room for layout diagnostics; middle-drag
  anywhere to pan the whole map around.
- **Mouse wheel** pans the map (hold Shift for horizontal, Ctrl to zoom) and
  scrolls every other scrollable surface too — the transcript and the lists
  inside modals (saves, file browser, gallery, hotkey dialog, …).
- **Select & copy text** — left-drag across the story pane to select transcript
  text, highlighted live as you drag; let go and it lands on your system
  clipboard via the OSC 52 terminal escape — so a selection copies cleanly even
  over SSH, with no clipboard library in the loop. Each row is clamped to the
  story pane's columns, so a drag never scoops up the map beside the text.
- **Room inspector overlay** — id, name, layer, position, and the per-edge
  dropped-constraint flags, so you can see *why* the layout engine placed a room
  where it did.
- **Pane focus** is always visibly highlighted. **Tab** / **Shift-Tab** step
  keyboard focus through the panes — story pane ↔ map, and, when the debug
  inspector is open, each of its windows in turn. Show or hide the map entirely
  with `/toggle-map`.

## Debug inspector (Z-machine)

`/debug` turns the map pane into a live **Z-machine debug inspector** — a
built-in debugger that follows the running story instruction by instruction.

![The debug inspector: live disassembly, call stack, and opcode hover help](../debug-inspector.png)

- **Live disassembly** that tracks the program counter, with a `PC` divider
  marking the next instruction about to execute.
- **Three tabbed windows.** A full-height **Disassembly** column fills the left.
  The right stacks two tabbed windows: a top window (**Globals** by default,
  plus **Locals**, **Objects**, and **Dictionary**) and a bottom window (**Call
  Stack**, **Stack**, and **Memory**). **Tab** / **Shift-Tab** move focus one
  window at a time — the story pane and each debug window are stops in the same
  cycle — **←**/**→** switch the sub-tab inside the focused window, and
  **↑**/**↓** scroll it.
- **Opcode hover help** — hover an instruction and a tooltip decodes the opcode
  and every operand: what each argument is, and where the result lands.
- **Click-to-jump operands** — addresses in the disassembly are underlined and
  jump to their target (code, memory, object, global, or local); `g` recenters
  on the PC, and `r` cycles the disassembly render mode (Full → Basic → Raw). In
  the Memory tab, `:` or `/` opens an address box that also accepts a variable
  token (`sp`, `g44`, `local10`).
- Select-and-copy works inside the inspector exactly as it does in the
  transcript. `Esc` closes it and restores the map.

## Playing aids
- **Verb/noun menu** — a two-pane token palette of common verbs and in-scope
  nouns; pick tokens to assemble a command (multi-noun sentences via
  prepositions), never typing a word.
- **Tab autocomplete** from the story's own dictionary plus the nouns mentioned
  in the current room. A live suggestion line shows the candidates with the
  active one bracketed: **Tab** cycles forward, **Shift-Tab** back, the bracket
  always tracks the word on the command line, and the line scrolls sideways to
  keep the highlighted candidate in view when the list overflows the width.
- **Command history** — press **↑**/**↓** at the prompt to recall and re-run
  earlier commands, shell-style. History persists across sessions inside the
  `.babelmap` archive; turn recording off with `record_history = false`.
- **Inventory strip** — a toggleable strip of your carried items along the
  bottom of the story pane.
- **Notification toasts** — status messages slide in at the top-right and fade
  after a few seconds, so a "map exported" or "style reloaded" note never
  interrupts the transcript. `/dump-notifications` replays the recent ones into
  the transcript if you missed a slide-by.
- **In-game hints** — `/open-hints` lays a hint panel over the story pane (the
  story pauses beneath it) that runs a companion *Invisiclues* `.z5` in a second
  Z-machine session, resizing with the pane. The panel renders the file's full
  split screen — its topic menu in the upper
  window with the clue text below — and forwards your keystrokes to it, so you
  drive the menu exactly as the file intends (arrows to move the highlight, plus
  whatever letters it prompts for, e.g. to pick a topic and reveal successive
  hints). `PageUp`/`PageDown` scroll back through the revealed clues in the lower
  window, and `Esc` closes. The hint file is auto-detected beside the story (or
  inside a sibling
  `.zip`), matched to *that* game by name so a multi-game folder never crosses
  wires, and remembered per game; if the story ships its own `HINT` command, the
  panel points you at that too. The downloaded *InvisiClues* files open on a
  "your screen is only N characters wide" banner (their menu names can be very
  long); babelmap skips it for you and drops you straight on the topic menu —
  turn `hint_skip_screen_warning = false` in the settings if you'd rather see it.
- **Reset** — restart the story from the top via a confirmation dialog with an
  opt-in "also clear the map" checkbox (the map is kept by default).
- **Slash commands** — type a leading prefix (default `/`, configurable) to run
  app commands by name: `/save-state`, `/restore-state`, `/reset-game [map]
  [data]`, `/pan-map <dx> <dy>`, `/zoom-map in|out|reset`, `/center-map`,
  `/tidy-map`, `/cycle-layer next|prev`, and more. `/help` lists every command
  grouped by category; `/help <command>` shows one command's usage and
  description. Names Tab-autocomplete, and feedback stays quiet on the status
  line.
- **Transcript search / filter / export** — `/search-transcript <query>`
  highlights matches (case-insensitive) and lands on the most recent; `n`/`N`
  step back/forward (configurable), `Esc` clears. A bare `/search-transcript`
  repeats the last query. `/filter-transcript story|meta|both` narrows the view
  to just game output (including your commands), just app/engine output, or
  everything. `/export-transcript [file]` writes the visible transcript to
  `transcript.txt` in the story's per-game directory by default (overwriting); a
  bare name lands beside it, a path-bearing value is honored verbatim — see
  [Storage layout](../persistence.md#storage-layout-sq-0284). Every transcript
  line is tagged by category — **story**, your **input** echo, **meta**
  (app/slash), and VM **warnings** — each independently themeable; meta and
  warning lines get their own configurable gutter markers (`▏` / `!`).
- **Map export** — `/export-svg [file]`, `/export-dot [file]`, and
  `/export-map [file]` write the map as an SVG, a Graphviz DOT graph, or an
  annotatable text/ASCII dump. Each defaults to a fixed name in the story's
  per-game directory (`map.svg` / `map.dot` / `map.txt`, overwriting); the
  optional `[file]` argument resolves the same way the transcript export does.

## Story picker
Point babelmap at a directory instead of a story file
(`babelmap path/to/stories/`) and it opens a picker of your whole library. Each
row shows the title (or filename), and a right-hand **TYPE** column names the
engine and version at a glance — `Z5`, `Z5 (blorb)`, `G3.1.2`, `Scott`, or
`Scott (blorb)` — so all three engines are told apart on sight. Two artifact
badges ride beside it: an existing **Save** and a **Hint** file — the hint badge
is uppercase (`H`) when a hint file is present locally and lowercase (`h`) when
none is local but a matching *InvisiClues* can be downloaded with `H` (see below).
(Blorb-wrapped stories advertise that with the `(blorb)` suffix on the type
label rather than a separate badge.)

When you launch from a directory this way, `/quit-to-library` drops the current
story and returns you to the picker to choose another (honouring the usual
save-on-quit prompt) — `/quit` still exits babelmap outright. Launched against a
single story file, there's no library to return to, so `/quit-to-library` just
says so.

The list sorts by **title**, **author**, **year**, or **type** — click a column
header, press `s` to cycle the column, or `d` to flip the direction. `i` or
`Tab` slides in a themeable **info panel** for the highlighted story:
format/version/release/serial, IFID, author/year/genre, a blurb, feature flags,
bundled resources, and saves. It animates per the `animation` config, starts
closed each launch, and refuses to open on terminals too narrow to hold both
the list and the panel. When the panel is open and its content overflows
(including a long, word-wrapped blurb), scroll it with the wheel over the panel
or `Shift`+`↑`/`↓`/PgUp/PgDn — plain arrows keep navigating the list — and the
scroll resets whenever the highlighted story changes.

`↑`/`↓`/`j`/`k`/PgUp/PgDn/Home/End navigate, `Enter` or a click opens the story,
`q`/`Esc` quits back to the shell. The badge glyphs are configurable under
`[symbols]` (`badge_save`/`badge_hint`/`badge_hint_available`, plus `badge_zcode`/
`badge_glulx`/`badge_blorb`), and the badge cluster, sortable headers, and info panel are all
themeable through the `story_badge`, `story_header`/`story_header:active` (the
active sort column), `story_author`, `story_year`, `story_no_metadata` (the
"(no metadata yet)" placeholder), `story_tile`/`story_tile:selected` (the
cover-gallery captions), and `story_info` (`:title`/`:label`/`:value`/`:blurb`/
`:cover`) style selectors.

- **Metadata fetch (IFDB).** Press `f` to fetch author/year/genre/description/
  cover art for the highlighted story from IFDB, or `r` to sweep the whole
  library (skipping any story already at the current fetch version); `Esc`
  cancels a running sweep. For a story whose IFID IFDB doesn't index, `u` lets
  you point it at an IFDB page by hand. Results are cached in a per-game sidecar,
  so a repeat `r` makes no network requests, and a blorb's own `IFmd`/`Fspc`
  metadata always wins over anything fetched.
- **Download hints.** For a highlighted game with no local hint file but a known
  *InvisiClues* release, press `H` to download one beside the story — the live
  IF Archive SLAG collection is preferred, with the Internet Archive's copy of
  the waitingforgo set as a fallback for games SLAG doesn't cover (together
  ~50 Infocom and other titles). The download runs in the background, the file
  is validated as a real Z-machine story before it lands, and the **Hint** badge
  lights the moment it finishes.
- **Cover art in the picker.** A blorb game with a frontispiece shows its cover
  right in the info panel, drawn with the terminal's best graphics protocol
  (Kitty / iTerm2 / Sixel) and a universal half-block fallback everywhere else.
  A story with no cover of its own borrows a fetched one once metadata has been
  pulled. Force a mode with
  `--image-protocol <auto|halfblocks|kitty|sixel|iterm2>`.
- **Preview bundled assets.** In the info panel's Resources list, image (`Pict`)
  and sound (`Snd`) rows are links — click one to pop a dismissible modal: an
  image renders fitted and centred; a sound plays once. Close it with `Esc`/
  `Enter`/`q`, the ✕, the Close button, or a click outside. Undecodable images
  and a missing audio device show a short status line instead.
- **Cover gallery.** Press `g` to trade the metadata list for a grid of cover
  thumbnails — as many ~16-column tiles as the pane is wide, each captioned with
  its title and the selected cover highlighted. Arrow keys or `h`/`j`/`k`/`l`
  drive a 2D cursor, PgUp/PgDn jump a screen of rows, the wheel scrolls a row at
  a time, and a click (or second click) selects (or opens) a cover. The info
  panel still toggles independently with `i`/`Tab`, `g` returns to the list, and
  the selection carries across both views.
- **In-game graphics (Glulx).** Games that open Glk graphics windows render
  their filled shapes and images right in the terminal, using the best graphics
  protocol (Kitty / iTerm2 / Sixel) with a half-block fallback. Disable all
  image rendering (in-game graphics *and* cover art) with `--no-images`.
- **Inline images in text.** Glk inline images placed in a text-buffer window
  (the main transcript or another buffer window) render as full-width blocks
  right in the flow of text — same protocol ladder, same fallback — and scroll
  along with the surrounding text. Themeable via the `inline_image` style
  selector.
