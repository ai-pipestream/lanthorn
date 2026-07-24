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
- **Execution coverage.** Once a line's address runs it turns blue and stays
  blue for the rest of the session, so you build up a map of what has actually
  executed; a `|` gutter bar additionally marks just the lines the *last* command
  ran. Launching with `--debug` opens the inspector automatically and traces from
  the very first boot instruction — capturing the game's start-up code a mid-game
  `/debug` would miss — and saves the accumulated coverage per story, so a later
  `--debug` (or a plain `/debug`) starts with the earlier runs' lines already blue.
- Select-and-copy works inside the inspector exactly as it does in the
  transcript. `Esc` closes it and restores the map.

### Scott Adams inspector

`/debug` (and `--debug`) work for Scott Adams stories too — the inspector
retargets itself to the way a Scott game actually thinks. There is no program
counter here; a Scott game *is* its **action table**, so the inspector puts that
table front and centre and drops the sections that only make sense for a
register machine (no call stack, eval stack, or linear memory).

- **Actions** (the left column) decompiles the action table one rule per line —
  `VERB NOUN  if CONDITIONS -> COMMANDS`, with items, rooms, flags, and messages
  resolved to their names. `r` still cycles Full → Basic (mnemonics, raw
  operands) → Raw (the bare numeric verb/noun/condition/command tuples), and
  hovering a rule expands it to the full `IF …` / `THEN …` listing.
- **Coverage, Scott-style.** Instead of executed program counters, the blue tier
  and `|` gutter mark **actions that have fired** — cumulatively (blue) and on
  the last command (the bar). An action whose verb and noun matched your command
  but was stopped by a failing guard is flagged inline with a `✗cond` suffix, and
  its hover names the condition slot that blocked it — a quick answer to "why
  didn't that work?". `--debug` traces from boot, so the opening auto-events are
  captured, and coverage persists per story exactly as for the Z-machine.
- **The right-hand tabs** carry Scott's world: **State** (current and saved room,
  lamp fuel, darkness, the live counter, set flags, and what's carried),
  **Items** (every object with its start location), **Vocab** (the verb and noun
  vocabularies with their synonyms), and **World** (every room with its exits,
  followed by the message table).

### Glulx inspector

`/debug` (and `--debug`) light up Glulx stories too — and here the inspector is
in its element, because Glulx *is* a register machine. The full layout survives:
a live **Disassembly** column that follows the PC instruction by instruction, a
real **Call Stack** and **Eval Stack**, the innermost frame's **Locals**, and a
**Memory** hex view you can jump anywhere in with `:` (raw Glulx addresses,
absolute — the ROM/RAM boundary is flagged with a `<RAM>` marker so you always
know which side you're on).

- **A disassembler that discovers.** Glulx code isn't laid out for a reader, so
  the inspector maps the image first: it follows the call graph from the start
  function, then type-validates a linear scan of the rest. Every instruction is
  tinted by confidence — solid for code reached from the start function, dimmer
  for a scan-only guess — and any address the story *actually executes* is
  promoted to certain on the spot. Call, branch, `jumpabs`, `streamstr`, and
  `glk` operands are annotated inline: a call shows its target, a `glk` shows the
  named selector (`glk_window_open`), a string print shows a snippet of the text.
- **Three repurposed tabs** trade the Z-machine's object world for Glulx's:
  **Functions** lists every discovered routine with its entry address, `C0`/`C1`
  calling convention, local count, confidence tier, and — for the well-known
  accelerated routines the VM shortcuts natively — an `[accel: Z__Region]` badge.
  **Strings** lists the discovered string objects (plain, compressed, or Unicode)
  with a decoded preview. **Glk** shows the live window tree, the same snapshot
  `/dump-windows` prints. Each row leads with a clickable address: a Functions
  row jumps straight to that routine in the Disassembly; a Strings row jumps the
  Memory pane. Call Stack return addresses (`ret=……`) are click-to-jump too, for
  the same Disassembly target.
- **Coverage and boot tracing** work exactly as elsewhere: the blue tier marks
  instructions ever executed, the `|` gutter marks the last turn, and `--debug`
  traces from the very first boot instruction (so an I7 game's lengthy startup is
  captured, which a later `/debug` toggle would miss) and persists coverage per
  story. Discovery is lazy — it runs once, the first time you open the inspector,
  and never touches a normal launch.

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
- **Readline-style line editing** at the story prompt: `Ctrl+A`/`Ctrl+E` jump to
  the start/end of the line, `Ctrl+U` clears back to the start, `Ctrl+K` clears
  forward to the end, and `Ctrl+W` deletes the word behind the caret — the same
  shortcuts your shell uses. Only live while you're actually typing a command
  (not mid-`read_char` prompt), so they never steal a keystroke the game expects.
- **Keyboard map navigation** — arrow keys and `hjkl` pan the map (and `+`/`-`/`0`
  zoom, `c` center) when it holds focus. **Shift+Arrow pans the map from either
  focus** — story or map — and keeps panning *during the tidy animation*, where
  the plain arrows step through the layout stages instead. During that animation
  `Ctrl+←`/`Ctrl+→` jump a whole stage at a time. `Ctrl+Q` (or `Ctrl+C`) quits
  from anywhere, even mid-prompt.
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
- **Command palette** — a fuzzy-searchable popup over *every* registry command,
  reachable even where no prompt exists (modals, the debug pane). Press `/` at an
  empty story prompt, or `/` inside the leader dialog (`Ctrl+P`), to summon it.
  It owns its own input line: type to filter — matching is subsequence fuzzy,
  ranked prefix › word-boundary › scattered, with the matched characters lit up —
  then keep typing past the command name to pass arguments. **↑/↓** move the
  selection (wrapping), **Tab** completes the highlighted name, **Enter** runs it
  (through the same dispatch a typed command uses), and **Esc** closes — returning
  to the leader dialog when that's where you came from, or to your untouched
  prompt otherwise. Click a row to run it, wheel to scroll, `[X]`/outside to close.
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

- **Search & download from IFDB.** Press `/` to open the **IFDB search** modal.
  It opens straight onto a **"Popular on IFDB"** browse list — highly-rated
  games with enough ratings to mean something, in IFDB's own confidence-ranked
  order — so there's something to explore before you type a word. Start typing
  a title or author and hit `Enter` to run a real search instead; the browse
  list stays visible while you type and is only replaced once your search
  returns. babelmap queries IFDB's public search API (in the background — the
  picker never freezes) and lists the matching games with their author,
  rating, and year. `↑`/`↓` (or `j`/`k`) move, and `Enter` on a game fetches
  its download links: if there's a single directly-playable story file it
  downloads at once, and if there are several a small chooser lets you pick
  one. The file lands in the current library directory, the list refreshes,
  and the cursor jumps to your new story with a "Downloaded …" note. Only
  files babelmap can actually open are offered (`.z3`–`.z8`, `.ulx`,
  `.gblorb`/`.zblorb`/`.blorb`/`.blb`, `.dat`); zips and executables are
  skipped — press `o` on a game with no direct story file to open its IFDB
  page in your browser instead. `Esc` backs out a level: from a typed
  search's results it returns to the "Popular on IFDB" list, and from that
  list it closes the modal. Downloads are capped at 16 MiB, filenames are
  sanitised, and an existing file is never overwritten (a `-2`, `-3`, … suffix
  is added). A "Results from IFDB" line credits the source, and every request
  carries babelmap's User-Agent, honouring IFDB's low-volume, user-driven API
  terms (search, browse, and downloads happen only when you ask — the browse
  list is one extra request per modal open, not a poll). The modal reuses the
  themeable `dialog.*` chrome plus the
  `ifdb_result`/`ifdb_result:selected`/`ifdb_result_meta`/`ifdb_download_marker`/
  `ifdb_attribution` style selectors.
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
