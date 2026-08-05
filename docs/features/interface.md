# Interface: navigation, playing aids & story picker

[← back to README](../../README.md)

The map draws itself, but you still have to drive it. babelmap gives you a
mouse-driven, copy-anything, keyboard-fast terminal cockpit for reading the map,
inspecting the machine, and firing commands — without ever leaving the story.

## Map navigation & inspection
- **Mouse support** — left-click a room to pop its info panel (name, notes,
  exits, objects); right-click a room for layout diagnostics; middle-drag
  anywhere to pan the whole map around. Neither panel interrupts the game: they
  are corner overlays, so the keyboard stays on the story prompt and you can keep
  typing and pressing Enter with one open — handy for watching a room's exit card
  fill in as you walk. Close one with **Esc**, its **✕**, or a click on empty
  map space. On a layer showing the [matrix view](mapping.md#mazes-the-matrix-view)
  the same click selects a row — and a click on a destination cell jumps the
  selection to the room it names.
- **Mouse wheel** pans the map (hold Shift for horizontal, Ctrl to zoom) and
  scrolls every other scrollable surface too — the transcript and the lists
  inside modals (saves, file browser, gallery, hotkey dialog, …).
- **Select & copy text** — left-drag across the story pane to select transcript
  text, highlighted live as you drag; let go and it lands on your system
  clipboard via the OSC 52 terminal escape — so a selection copies cleanly even
  over SSH, with no clipboard library in the loop. Each row is clamped to the
  story pane's columns, so a drag never scoops up the map beside the text.
- **Drag a pane boundary to resize it** — grab the divider between the story and
  map panes, or the top edge of the inventory dock or the command band, and drag.
  The boundary lights up as the pointer crosses it, the panes follow the pointer
  live, and the new size is written to `config.toml` when you let go. What you
  press the button on decides what the drag means: a drag that starts on a
  boundary only resizes, and a text selection that starts in the transcript keeps
  selecting even when it crosses one. For the keyboard, **F3** (or
  `/resize-panes`) enters resize mode — **Tab** cycles which boundary is live,
  the arrows move it, `0` resets, **Esc** leaves.
- **Room inspector overlay** — id, name, layer, position, and the per-edge
  dropped-constraint flags, so you can see *why* the layout engine placed a room
  where it did. The panel stays open while you keep playing.
- **The map never takes the keyboard.** Every keystroke goes to the story, so a
  key always means the same thing — you never have to look at which pane is
  "active" before pressing an arrow. The map is driven alongside your typing
  instead: `Shift+Arrow` pans, the mouse pans/zooms/selects, and zoom and
  centring live on the `Ctrl+P` leader panel's **Map** group. **Tab** / **Shift-Tab** are only
  live when the debug inspector is open, where they step through its windows.
  Show or hide the map entirely with `/toggle-map`.

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
a live **Disassembly** column anchored on the PC, a
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
- **Where the PC parks, and how to get somewhere interesting.** The panel
  refreshes between turns — and between turns a Glulx story is *always* suspended
  in the same spot: the `@glk glk_select` inside Inform's Glk veneer, a
  three-instruction shim (`copy sp, L0` / `glk #0xc0, L0, L4` / `return L4`) that
  pops the Glk argument count, dispatches, and returns. So the PC anchor reports
  the same address every single turn (`00049a` in *Coloratura*, `00103c` in
  *Counterfeit Monkey*), and the instructions around it are dispatch glue rather
  than story logic. That is the machine's honest state, not a mis-decode: it
  really is parked there, and the shim really does disassemble that way. To land
  in the game's own code, click a **Call Stack** `ret=……` address — the frames
  beneath the veneer carry real return PCs into the story. PC-follow re-anchors
  on the next refresh, so re-click after each turn.
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
- **`[more]` paging, the way the originals did it** — whenever a turn's output
  runs past the story pane (a long description, a boot banner, a hint page,
  even a "press any key" dump), the view stops at the *first* fresh screenful
  with a reverse-video `[more]` bar instead of scrolling you straight to the
  end. Space/PgDn/↓/Enter page onward; while the game is waiting on a single
  keypress, *every* key pages and nothing reaches the game until you've caught
  up — the key that lands at the bottom is consumed by the bar, the next one
  answers the game, exactly like a real Infocom interpreter. Menus that clear
  and repaint start you at the top; output that fits shows no bar; babelmap's
  own output (`/help` and friends) never pages; and a game that asks for
  `[MORE]` suppression (Zork Zero's demo mode) gets it. The bar is themeable
  via `more_prompt`.
- **The command band** (**F2**, or `/open-command-band`) — a Journey-style
  bottom dock that builds a command by pointing, never typing a word. Columns
  fill in left to right as the phrase narrows: **VERB**, then **WHAT — here**
  and **WHAT — carried**, then a **WITH…/IN…/TO…** column for verbs that take
  two objects. Each verb declares its shape, so only the columns that can come
  next are offered; the rest stay dimmed until they are reachable.

  The object columns are **live**: they read the running story's object tree and
  refresh every turn, so taking something moves it from *here* to *carried* as
  you watch. (Glulx and Scott have no object tree yet, so *here* degrades to a
  clearly-labelled **WHAT — seen** list scraped from recent output.)

  Nothing ever fires a turn by itself. A grammatically complete phrase *arms*
  the phrase line (`Enter: send` lights up) and waits for **Enter** or a click
  on the line — including the one-click quick-action row along the bottom
  (`n s e w · up down · in out · look inventory wait again`), whose picks also
  just fill the phrase.

  It is a dock, not a modal: the story prompt stays live underneath, paste keeps
  working, and graphical v6 keeps its artwork. **Tab** hands the keyboard to the
  story input for free typing with the band still on screen, and back. While the
  band has the keyboard, **←/→** move between columns, **↑/↓** within one,
  typing filters the active column, **Backspace** clears a filter character and
  then un-picks the last token, and **Esc** steps back one level per press
  (filter → phrase → close). Everything visible is clickable and the wheel
  scrolls whichever column is under the pointer. While it is open it subsumes
  the inventory dock — the *carried* column IS your inventory — which returns
  when you close it.

  Its height, its verb grammar and its quick row are all configurable under
  `[command_band]` in `config.toml`; resize mode targets its height.
- **Tab autocomplete** from the story's own dictionary plus the nouns mentioned
  in the current room, shown the way your shell shows it: the rest of the word
  appears in dim ghost text right under the caret as you type. **Tab** cycles
  forward through the candidates, **Shift-Tab** back, and **→** at the end of the
  line takes the one on offer. Because the hint lives on the prompt row itself,
  nothing shifts when a completion appears or vanishes — the prompt stays put
  even when it is the very last line in the pane.
- **The command palette** (type `/`) keeps its own presentation: a bracketed
  candidate strip below the prompt, since command names match anywhere in the
  word — `/config` finds `open-config` — and there is no single tail to ghost.
  **Tab**/**Shift-Tab** cycle it, the bracket tracks the name on the command
  line, and the strip scrolls sideways to keep the active candidate in view.
  Give it a border with the `suggestion_line` style selector to float it as a
  boxed popup.
- **Command history** — press **↑**/**↓** at the prompt to recall and re-run
  earlier commands, shell-style. History persists across sessions inside the
  `.babelmap` archive; turn recording off with `record_history = false`.
- **Readline-style line editing** at the story prompt: `Ctrl+A`/`Ctrl+E` jump to
  the start/end of the line, `Ctrl+U` clears back to the start, `Ctrl+K` clears
  forward to the end, and `Ctrl+W` deletes the word behind the caret — the same
  shortcuts your shell uses. Only live while you're actually typing a command
  (not mid-`read_char` prompt), so they never steal a keystroke the game expects.
- **Keyboard map navigation** — **Shift+Arrow pans the map** without leaving the
  command line, and keeps panning *during the tidy animation*, where
  the plain arrows step through the layout stages instead. Zoom (`+`/`-`)
  and centring (`0`) are on the `Ctrl+P` leader panel's **Map** group. During that animation
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

Once a story's metadata has been fetched, a **RATING** column carries IFDB's
community average with the number of votes behind it — `3.8 (226)`, the plain
number to one decimal, no star glyphs to squint at. The vote count is there
because a lone `5.0` and a `5.0` over three hundred ratings are not the same
claim. A game nobody has rated, or one you haven't fetched yet, leaves the cell
empty rather than pretending to a damning `0.0`; press `r` to sweep the library
and fill them in. RATING is the first column to step aside on a narrow pane, so
it never crowds the title or author.

The list sorts by **title**, **author**, **year**, **rating**, or **type** —
click a column header, press `s` to cycle the column, or `d` to flip the
direction. Sorting by rating parks every unrated story at the bottom in both
directions, and breaks ties between equal averages by how many people voted, so
a 4.6 from two hundred players outranks a 4.6 from three. `i` or
`Tab` slides in a themeable **info panel** for the highlighted story:
format/version/release/serial, IFID, author/year/genre, a blurb, feature flags,
bundled resources, and saves. It animates per the `animation` config, starts
closed each launch, and refuses to open on terminals too narrow to hold both
the list and the panel. When the panel is open and its content overflows
(including a long, word-wrapped blurb), scroll it with the wheel over the panel
or `Shift`+`↑`/`↓`/PgUp/PgDn — plain arrows keep navigating the list — and the
scroll resets whenever the highlighted story changes.

`↑`/`↓`/`j`/`k`/PgUp/PgDn/Home/End navigate, `Enter` or a click opens the story,
`q`/`Esc` quits back to the shell. The badge glyphs are yours to change: set
`badge_zcode`, `badge_glulx`, `badge_blorb`, `badge_save`, `badge_hint` or
`badge_hint_available` under `[elements]` in `style.toml` — they default to the
plain letters `Z`/`G`/`B`/`S`/`H`/`h`, and a patched font can swap in real icons.
They live beside the `story_badge` selector that colours them. The badge cluster,
sortable headers, and info panel are all themeable through
`story_badge`, `story_header`/`story_header:active` (the
active sort column), `story_author`, `story_year`, `story_rating` (the IFDB
average and vote count in the RATING column), `story_no_metadata` (the
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
  its download links and opens a small chooser to pick one — including when
  there is only a single playable file, so you always get to see what you are
  about to fetch. Each file in the chooser carries IFDB's own description of it on the
  line below — "Release 16: latest version of the game.", "Competition
  version" — which is often the only way to tell the candidates apart, since a
  game may well list several files under the *same* filename. A file the
  library directory already holds is marked `✓ … · already downloaded` (you can
  still download it again; it lands beside the original under a new name). The
  file lands in the current library directory, the list refreshes,
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
  `ifdb_download_present`/`ifdb_attribution` style selectors (the two download
  selectors carry the row's `⭳`/`✓` glyph, so a theme can change it).
  Both lists scroll the way the story list does — the cursor moves inside the
  visible window and only scrolls it once it reaches an edge — and `Home`/`End`
  and `PageUp`/`PageDown` work throughout.
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
  and a missing audio device show a short status line instead. On an image,
  `+`/`=` and `-` (or the wheel) step an integer zoom in/out — 1×, 2×, 3×, …,
  nearest-neighbour scaled so old low-res art stays crisp instead of blurring;
  `0` resets to fit. Past-native zoom centre-crops rather than shrinking back
  down, so postage-stamp 320×200-era art can be blown up to fill the modal.
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
