# Interface: navigation, playing aids & story picker

[← back to README](../../README.md)

The map draws itself, but you still have to drive it. babelmap gives you a
mouse-driven, copy-anything, keyboard-fast terminal cockpit for reading the map,
inspecting the machine, and firing commands — without ever leaving the story.

## Map navigation & inspection
- **Mouse support** — left-click a room to point the [room dock](#the-room-dock)
  at it; right-click a room for its layout diagnostics; middle-drag anywhere to
  pan the whole map around. The dock never interrupts the game: it reserves rows
  at the bottom of the map pane rather than covering anything, so the keyboard
  stays on the story prompt and you can keep typing and pressing Enter with it
  up — handy for watching a room's exit card fill in as you walk. On a layer
  showing the [matrix view](mapping.md#mazes-the-matrix-view) the same click
  selects a row — and a click on a destination cell jumps the selection to the
  room it names.
- **Mouse wheel** pans the map (hold Shift for horizontal, Ctrl to zoom) and
  scrolls every other scrollable surface too — the transcript and the lists
  inside modals (saves, file browser, gallery, config, command palette, the
  IFDB search results, the command band's columns, …). On a list the wheel
  scrolls *the list*, not the
  cursor: the highlight stays on the row you left it on and the rows slide
  under it, and only when the window would carry it off the screen does it
  come along, riding the top or bottom row. The keys work the other way round
  — `↑`/`↓` move the cursor and the list follows it — which is why a wheel is
  for browsing and an arrow is for choosing. A list that already fits its
  window has nothing to scroll, and the wheel there does nothing at all — and
  when that list is inside a dialog, the notch stops there rather than quietly
  scrolling whatever is behind it.
- **A scrollbar that gets out of the way** — every scrollable surface draws the
  same bar, and it is drawn as *colour*, not as a glyph: thumb and track are
  background fills, so a line of prose ending one column short of it has a clean
  gutter instead of a full block leaning on it. In the **story pane** the bar
  also auto-hides. It appears when you actually scroll — wheel, `PageUp`/
  `PageDown` and the other scroll keys — holds for a moment, then fades out. New
  game text never summons it (that would flash a bar at you every turn), and
  nothing reflows when it goes: the story bar lives in the pane's margin band,
  outside the text. Modals keep theirs permanently, because a modal's gutter is
  taken out of its list width and hiding it there *would* reflow the list. Tune
  it with `scrollbar_hide_ms` / `scrollbar_fade_ms` under `[animation]`, or set
  `scrollbar_hide_ms = 0` to keep the story bar up for good.
- **Select & copy text** — left-drag across the story pane to select transcript
  text, highlighted live as you drag; let go and it lands on your system
  clipboard via the OSC 52 terminal escape — so a selection copies cleanly even
  over SSH, with no clipboard library in the loop. Each row is clamped to the
  story pane's columns, so a drag never scoops up the map beside the text.
- **Drag a pane boundary to resize it** — grab the divider between the story and
  map panes, or the top edge of the inventory dock, the command band or the room
  dock, and drag.
  The boundary lights up as the pointer crosses it, the panes follow the pointer
  live, and the new size is written to `config.toml` when you let go. What you
  press the button on decides what the drag means: a drag that starts on a
  boundary only resizes, and a text selection that starts in the transcript keeps
  selecting even when it crosses one. For the keyboard, **F3** (or
  `/resize-panes`) enters resize mode — **Tab** cycles which boundary is live,
  the arrows move it, `0` resets, **Esc** leaves.
- **The room dock** — one panel at the bottom of the map pane describing one
  room, opened with `k` from the leader panel or `/toggle-room-dock`. It has two
  bodies:
  - **Room** — the room's notes, its [exit card](mapping.md#room-card) in the
    matrix vocabulary, and the objects the engine can see there. The card spends
    the dock's WIDTH rather than its height: the twelve travel directions lay
    out in up to three columns — cardinals, diagonals, portals — so the whole
    card is four rows on a normal map pane and falls back to the single column
    on a narrow one.
  - **Diagnostics** — id, layer, grid position, and the per-edge
    dropped-constraint flags, so you can see *why* the layout engine placed a
    room where it did. `/toggle-inspector` opens straight onto this body, and
    flips back to Room when the dock is already up.

  The two names sit in the dock's tab strip — the same strip, and the same
  click, as the map pane's layer tabs: click either name to switch bodies.

  **It follows you by default.** With nothing selected the dock describes the
  room you are standing in and updates every move — the header says `⌖ following`.
  Click a room to **pin** it (`⊙ pinned`); the dock then holds that room while
  you walk on. Pinning is just selecting, so the map highlight and the matrix
  cross-highlight always agree with the dock. **Unpin** — back to following — by
  clicking the pinned room again, clicking empty map space, or pressing **Esc**;
  a second **Esc** closes the dock. It is not a modal, so it costs you nothing to
  leave up: it never takes the keyboard and it never hides the prompt.
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
  via `more_prompt`. "First fresh screenful" is measured against the rows that
  actually carry prose — the bar's own row, a v3 status line, the optional
  command bar and a suggestion strip are none of them readable, and counting
  them let a line slip past the pause (SQ-0823). And "fresh" allows for a page
  that starts on the row the game stopped on: Arthur's InvisiClues print a `1> `
  prompt, wait for a key, and then print the page *after* that prompt, on that
  row — so it is part of the new page, and the pause shows it rather than
  scrolling its heading away (SQ-0823).
- **The command band** (**F2**, or `/open-command-band`) — a Journey-style
  bottom dock that builds a command by pointing, and suggests one as you type
  (it never takes the keyboard from the prompt — see "typing always wins"
  below). It is a
  borderless strip, not a framed panel: columns fill in left to right as the
  phrase narrows — **VERB** (its column is unlabelled — self-evident, and its
  list starts right on the row the label would have used, so it shows one
  more entry than the columns beside it), then **WHAT — here** and
  **WHAT — carried**, then a **WITH…/IN…/TO…** column for verbs that take two
  objects. Each verb declares its shape, so only the columns that can come
  next are offered; the rest stay dimmed until they are reachable.

  The object columns are **live**: they read the running story's object tree and
  refresh every turn, so taking something moves it from *here* to *carried* as
  you watch. (Glulx and Scott have no object tree yet, so *here* degrades to a
  clearly-labelled **WHAT — seen** list scraped from recent output.) An
  empty *here*/*carried* column says so explicitly rather than sitting blank.

  *Here* means **what you can see**, not "what the room object happens to
  contain". It includes things resting on a supporter or sitting in an open
  container — Zork I's kitchen lists the sack and the bottle on the table, both
  of which are children of the *table* — and the shared scenery a room names but
  does not own, like the window Behind House. It never lists the contents of a
  closed container: the lunch and the garlic inside the brown sack stay hidden
  until you open it, and the leaflet stays in the mailbox until you do. The
  Z-machine gives attributes no fixed meaning, so which attribute means "open"
  and which property lists a room's scenery are recovered per story from its own
  object table; when a story cannot be read confidently, *here* falls back to
  the room's direct contents rather than guessing.

  Composing happens directly on the real story input line — a pick appends
  its word there, merging with anything you already typed — so nothing ever
  fires a turn by itself except the quick actions below; everything else
  sends the ordinary way, with **Enter** on that line, which NEVER picks a
  row — it always sends exactly what you typed. A **double-click** on a word
  row is pick-then-fire: the first click appends the word as usual, the
  second (same row, within the double-click window) submits the composed
  prompt — so the last word of a phrase can be click-clicked straight into
  the game.

  **Typing always wins.** The band never takes the keyboard for text: letters,
  Backspace and paste go to the story prompt exactly as they do with the band
  closed. What the band DOES claim is column navigation: there is always a
  **current column** — the dividers flanking it carry the accent — and
  **Tab**/**Shift-Tab** step it across
  whichever columns are reachable. As you type, the closest match in the
  *current* column highlights (matching a later word of a name too, so `do`
  finds `iron door` once *here*/*carried* is current); **↑**/**↓** highlight a
  row within it directly, the first press only arming the highlight without
  moving it. **Tab** unifies the two: with nothing highlighted it just moves
  to the next column, but with a row highlighted — typed or arrowed — it picks
  that row and advances, exactly like a click. **Shift-Tab** always just
  moves, even with something highlighted. **←**/**→** are the ordinary caret
  keys on the prompt; the band doesn't claim them. **Esc** clears an armed
  **↑**/**↓** highlight first, then closes the band — and **F2**/
  `open-command-band` is a toggle, so it always closes the band too, Esc ladder
  or not.

  The one-click quick actions (`n`/`s`/`e`/`w`/`ne`/`nw`/`se`/`sw`,
  `up`/`down`/`in`/`out`, `look`, `inventory`, `wait`, `again` by default) are
  the one exception: a click submits AT ONCE, no Enter, and never disturbs a
  phrase you're mid-composing. When the band is wide enough they draw as a
  block on its left edge — the compass rose (eight points around an inert
  centre dot) on top, with everything else in the quick list flowing in as
  many rows as it needs BELOW the rose, only as wide as its widest row; a
  narrower band falls back to the older single-line row along the bottom
  instead. Either way every point and word is its own click target, and the
  quick block is **mouse-only** — hovering one (with either layout) reverses
  it, distinct from a picked column row's own highlight, but no keyboard
  gesture reaches it; command history (**Ctrl+↑**/**Ctrl+↓**, or plain
  **↑**/**↓** with the band closed) is always available instead. Single-cell
  `│` dividers separate the quick block from the columns and every column from
  its neighbour.

  It is a dock, not a modal: the story prompt stays live underneath, paste keeps
  working, and graphical v6 keeps its artwork. Everything visible is clickable,
  and the wheel scrolls whichever column is under the pointer — the column you
  are looking at, not the one the band is pointing at, and its rows slide under
  their highlight by the [same rule every other list follows](#map-navigation--inspection).
  Neither the band's attention nor the other three columns move with it. While it is open
  it subsumes the inventory dock — the *carried* column IS your inventory —
  which returns when you close it.

  Its height, its verb grammar and its quick list are all configurable under
  `[command_band]` in `config.toml`; resize mode targets its height. The
  compass-rose/flat-row choice is not configurable — it is computed from the
  band's actual width every frame. A band shorter than the quick block's full
  height (rose plus every word row) still draws the whole rose and simply
  clips the word rows it has no room for; resize the band taller to see them
  all.
- **Tab autocomplete** from the story's own dictionary plus the nouns mentioned
  in the current room, shown the way your shell shows it: the rest of the word
  appears in dim ghost text right under the caret as you type. **Tab** cycles
  forward through the candidates, **Shift-Tab** back, and **→** at the end of the
  line takes the one on offer. (With the command band open, Tab completes from
  the *band's* highlight instead — one completion source at a time.) Because the
  hint lives on the prompt row itself,
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
  earlier commands, shell-style (**Ctrl+↑**/**Ctrl+↓** work too, and are the
  only way to reach it while the command band is open, since plain **↑**/**↓**
  belong to the band's own row navigation there). History persists across
  sessions inside the `.babelmap` archive; turn recording off with
  `record_history = false`.
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
engine and version at a glance — `Z5`, `Z5 (blorb)`, `Z6 (ADF)`, `G3.1.2`,
`Scott`, or `Scott (blorb)` — so all three engines are told apart on sight. Two
artifact badges ride beside it: an existing **Save** and a **Hint** file — the
hint badge is uppercase (`H`) when a hint file is present locally and lowercase
(`h`) when none is local but a matching *InvisiClues* can be downloaded with `H`
(see below). (Blorb-wrapped stories advertise that with the `(blorb)` suffix on
the type label rather than a separate badge.)

The container is part of that label, so a story you're playing off its original
release floppy reads `Z6 (ADF)` off an Amiga disk, `Z6 (HFS)` off a Macintosh
one, `Z6 (DOS)` off a PC floppy, `Z4 (ST)` off an Atari ST one or
`Z5 (ProDOS)` off an Apple II disk, and is never
mistaken for a loose story file. The disk says so, not the
filename: the suffix comes from the mount that found the story inside the image,
so a floppy named anything at all is labelled for the filesystem it actually
carries, and a plain story file that happens to be called `.adf` is not labelled
at all.

**Every release medium babelmap can mount, the picker offers.** The scan decides
which files are worth opening from the same format table the mount reads, so a
shelf of `.ima`, `.img`, `.st` and `.2mg` floppies lists beside the `.adf`s and
the `.z5`s rather than being playable only by name. That is a pre-filter and nothing
more — a `.img` that turns out to be a holiday photo is opened, found not to be a
disk at all, and never shown.

**And a row is a game, not a file.** An Amiga release came one story to a disk,
but a compilation does not: `Infocom Compilation 1` carries six, `floppy2.ima`
six more, and the *Lost Treasures* Apple II volumes four or five apiece. Each of
them is its own row — its own title, its own `Z3 (ST)`/`Z5 (ProDOS)` type, its
own release and serial, its own saves and its own cover — so you pick *Leather
Goddesses of Phobos* the way you pick anything else in the list, by name, and
Enter opens that game rather than whichever story on the disk happened to be the
largest file. About thirty games across the six *Lost Treasures* volumes alone
were unreachable from this screen before; sort, search and the info panel all
work on them now because they are ordinary rows.

A disk holding one story is untouched by any of this: one row, opened by path,
exactly as before. Where the title tables know a build, the row is titled from
it (*Sherlock: The Riddle of the Crown Jewels*); where they do not, the row takes
the name the disk itself gives the file (`LEATHRGODDESSES`), because the image's
own filename names the box and would read the same on every row. The info
panel's file line names both — `…(Disk 6 of 7).2mg:LEATHRGODDESSES` — so it is
always clear which game on which image you are looking at.

### A multi-disk release is one collection

Those compilations mostly came as *sets*: seven Apple II volumes for *The Lost
Treasures of Infocom*, nine Atari ST floppies, `floppy1.ima` through
`floppy5.ima`. babelmap treats a set as one shelf of games rather than as a pile
of disks, and it works out which files belong together from their names — files
in one directory, sharing a disk-image extension, with identical names except
for one run of digits that counts 1, 2, 3…

**Name any one volume and you get the whole release.** `babelmap disk1.img` opens
the browser on all eleven games across `disk1`–`disk4`, not the single story that
one image happens to hold. `babelmap "Lost Treasures … (Disk 1 of 7).2mg"` opens
all thirty — and that one used to be an error, because the Apple II press puts a
launcher on disk 1 and no story at all. Once you're in, it behaves like any
library: pick a game, play it, `/quit-to-library` comes back to the same shelf.

**And a game the set carries twice is listed once.** These collections overlap:
`Infocom Compilation 5` stores its games as flat files and `Compilation 8` in
per-game directories, and both carry the very same Trinity — release 11, serial
860509. So do Lurking Horror, Moonmist, Stationfall, Cutthroats and Hitchhiker's.
Listing every disk's contents gave 39 rows for 33 games; matching on the story's
IFID gives you each game once, off the first disk that offers it, with all its
saves and metadata intact.

Folding is deliberately narrow. It happens **only within one release**, and only
between rows that are the *same build* down to the release, serial and checksum.
Zork Zero's release 296 on a Macintosh volume, 366 on an Amiga floppy and 393 on
the DOS media are three different games as far as this is concerned, and stay
three rows — as does that same 393 sitting on `floppy5.ima`, on the 360K DOS
press and on a loose `.z6`, because those are four separate things you chose to
keep. Nothing outside a set is ever merged.

A set that turns out to hold only **one** game gets the opposite treatment, and
for the same reason: it doesn't need a menu, but its disks do belong to each
other, so its artwork is shared across them. That is what the DOS presses of
*Zork Zero* need — the 360K one puts the story alone on disk 2 with CGA on disk 1
and EGA on disk 3, so booting the story disk drew nothing at all until babelmap
learned to read the whole release. A set with two or more games gets the browser
instead and keeps each disk's art on that disk; see
[Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws).

Recognition is cautious on purpose, since wrongly merging two collections is
worse than not spotting one. `adv01.dat` … `adv13.dat` are thirteen separate
Scott Adams games and stay that way — they aren't disk images. Zork Zero's 360K
and 720K DOS presses both label their disks `(Disk 1)`, `(Disk 2)`, and remain
two sets, because the run that differs between them is `360`/`720` — a capacity,
not a disk number. `disk*.img` and `floppy*.ima` are two families and two sets.
Years like `(1993)` are never mistaken for disk numbers, `Zork I`/`II`/`III` are
words rather than digits, and a set whose first disk you don't have isn't
detected at all — you'll still see every game, just listed disk by disk.

When you launch from a directory this way, `/quit-to-library` drops the current
story and returns you to the picker to choose another (honouring the usual
save-on-quit prompt) — `/quit` still exits babelmap outright. Launched against a
single story file, there's no library to return to, so `/quit-to-library` just
says so.

Every key on this screen is **rebindable**. The picker runs before there is a
game to act on, so it has its own layer in the one command registry — its own
context, its own verbs (`play-story`, `toggle-gallery`, `sort-library`, and the
rest) — and `[keymap.browser]` in `config.toml` moves any of them; see
[Customization](customization.md). The hint bar along the bottom is generated
from those bindings rather than written out by hand, so it names the key you
actually have bound and quietly stops advertising anything you unbind.

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
bundled resources, detected artwork, and saves. When the file on disk is a *container* — an Amiga
floppy, a blorb, a zip — the size line names the game's own size beside the
file's, because the container's length is not the game's: every `.adf` is 880 KB
whether it holds Zork I or Shogun. Plain story files show one size, as before.
It animates per the `animation` config, starts
closed each launch, and refuses to open on terminals too narrow to hold both
the list and the panel. When the panel is open and its content overflows
(including a long, word-wrapped blurb), scroll it with the wheel over the panel
or `Shift`+`↑`/`↓`/PgUp/PgDn — plain arrows keep navigating the list — and the
scroll resets whenever the highlighted story changes.

An **Artwork** block lists the native picture archives detected for that story —
`zork0.mg1  MCGA  503 pictures` — with an arrow against whichever one the game's
own `config.toml` names. It is inventory, not a control: nothing here is
selectable, and choosing between them is the launch-options dialog's job. Both
read the same detector, so the panel can never advertise a rendition the dialog
won't offer. A game with no detected archives shows no block at all. See
[choosing which artwork a game draws](v6-graphics.md#three-ways-to-say-it) for
what "detected" means and how to name an archive the detector can't see.

`↑`/`↓`/`j`/`k`/PgUp/PgDn/Home/End navigate, `Enter` or a click opens the story,
`q`/`Esc` quits back to the shell.

**Shift-Enter** opens the story's **launch options** instead of launching it —
the boot-time choices babelmap can only honour *before* a game starts: which
picture archive to draw its art from, and which machine to present itself as.
(`o` does the same, for terminals that can't tell Shift-Enter from plain Enter,
and so does double-right-clicking a row.) Plain Enter is untouched, so you only
meet the dialog when you ask for it. It offers the archives detected for *that*
story — the same list the info panel shows — plus a line reminding you that an
archive under some other name is still reachable by naming it outright. Inside
it, `↑`/`↓` move between choices,
`Space` picks the one under the cursor or flips a checkbox, `Tab`/`Shift-Tab`
move between the buttons, `Enter` plays and `Esc` backs out. Its choices always
fit the dialog, so a wheel notch over it has nothing to scroll — and it is eaten
there rather than sliding the story list around behind the dialog. Everything applies
to that launch alone unless you tick *Save as this game's default*, which writes
your changes — and only your changes — to the game's own `config.toml`. See
[choosing which artwork a game draws](v6-graphics.md#three-ways-to-say-it).

The badge glyphs are yours to change: set
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
`:cover`) style selectors. The Artwork block has its own pair —
`story_info_artwork` for the detected archives and `story_info_artwork:active`
for the one in use — and the launch-options dialog's warnings carry
`dialog.launch_caveat`.

- **Search & download from IFDB.** Press `/` to open the **IFDB search** modal.
  It opens straight onto a **"Popular on IFDB"** browse list — highly-rated
  games with enough ratings to mean something, in IFDB's own confidence-ranked
  order — so there's something to explore before you type a word. Start typing
  a title or author and hit `Enter` to run a real search instead; the browse
  list stays visible while you type and is only replaced once your search
  returns. **Tab**/**Shift-Tab** toggle focus between the `Search:` field and
  the list — so a half-typed query can be parked while you go back to arrow
  through the results, and picked up again where you left it. babelmap queries IFDB's public search API (in the background — the
  picker never freezes) and lists the matching games with their author,
  rating, and year. `↑`/`↓` (or `j`/`k`) move — the wheel scrolls the results
  under the highlight instead — and `Enter` on a game fetches
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
  metadata always wins over anything fetched. What you fetch here follows you
  into the game: the story pane's border title is resolved from exactly this
  chain, so once a title is known the library and the pane always agree on it.
- **Download hints.** For a highlighted game with no local hint file but a known
  *InvisiClues* release, press `H` to download one beside the story — the live
  IF Archive SLAG collection is preferred, with the Internet Archive's copy of
  the waitingforgo set as a fallback for games SLAG doesn't cover (together
  ~50 Infocom and other titles). The download runs in the background, the file
  is validated as a real Z-machine story before it lands, and the **Hint** badge
  lights the moment it finishes. Which clues belong to which game is decided by
  the *story's* identity — the release and serial the mounted image carries —
  not by what the file on disk is called, so an Amiga floppy named for its box
  (`Zork I - The Great Underground Empire.adf`, which spells `zork1` nowhere)
  finds its InvisiClues just as the bare story file does, and so does a clues
  file already sitting beside it. Filenames are consulted only for games the
  identity table doesn't name.
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
  drive a 2D cursor, PgUp/PgDn jump a screen of rows, the wheel scrolls the grid
  a whole row of tiles at a time (the highlight holds its column and rides the
  top or bottom row rather than being dragged along), and a click (or second
  click) selects (or opens) a cover. The info
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
- **One rule for scaling every picture.** Cover art, gallery tiles, the resource
  preview, in-game Glulx graphics and inline transcript pictures all go through
  the same resampler, and it picks its filter by the direction the picture is
  *moving* rather than by taste. Growing replicates whole pixels, so a 320×200
  title card blown up to fill a pane arrives with the palette it left with — the
  "crisp, not blurry" that pixel art is famous for. Shrinking averages the area
  each destination pixel covers, so a jacket scan reduced sevenfold into the info
  panel keeps all seven rows instead of one, and a dithered shadow fuses into the
  colour it was always standing in for instead of breaking into speckle. Pictures
  with cut-out edges — Zork Zero's drop caps and room icons, a Glulx card
  stencilled out of its background — are averaged on *associated* colour, so a
  transparent neighbour lends its coverage and not the invisible black behind it,
  and no dark hairline creeps around the cut. Each of these surfaces used to pick
  its own filter, and covers in particular were shrunk by throwing rows away.
