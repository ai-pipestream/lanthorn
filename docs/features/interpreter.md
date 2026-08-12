# Interpreter (Z-machine, Glulx & Scott Adams)

[← back to README](../../README.md)

Point babelmap at a story and it works out the format from the file itself and
boots the right engine — you never choose. Under the hood are three from-scratch,
zero-dependency virtual machines written clean-room in Rust: a Z-machine
(`zvm`), a Glulx engine (`gvm`), and a Scott Adams / ScottFree engine (`scott`).
All three converge on one neutral screen model, so the host features below —
sound, colour, timed input, crash-proofing — light up no matter which you're
playing.

- **Z-machine** (`zvm`) — the Infocom canon and decades of Inform 6, in story-file
  versions **v3/v4/v5/v6/v7/v8**, including graphical **v6** — verified in depth
  against *Zork Zero*, whose pictures and text composite together on
  image-capable terminals, with the same engine targeting the wider v6
  catalogue (*Shogun*, *Journey*, *Arthur*). See [Graphical v6](v6-graphics.md)
  for how. (v1/v2 are not supported.)
- **Glulx** (`gvm`) — modern Inform 7, with a complete **Glk 0.7.6** layer verified
  against the standard Glulx/Glk test suites, an accelerated Inform veneer, and the
  full floating-point opcode set. It targets Glulx spec 3.1.3 and reports every
  capability it does and doesn't have honestly through `gestalt`.
- **Scott Adams** (ScottFree `.dat`) — the classic 8-bit text adventures
  (*Adventureland*, *Pirate Adventure*, …), played through the same TUI and live
  automap as everything else. Room illustrations render when the game ships as a
  Blorb with PNG artwork (drawn by the graphics pipeline); babelmap plays the
  `.dat` text engine and shows the bundled images — it does **not** decode the
  original SAGA line-draw graphics format.

## What counts as a story file

Point babelmap at whatever the game arrived in and it digs the story out itself.

- **Bare images** — `.z3`–`.z8`, `.ulx`, and Scott Adams `.dat` are read straight.
- **Blorb containers** — `.zblorb`/`.gblorb`/`.blorb`/`.blb` yield their executable
  chunk, and the same file's `Pict`/`Snd `/`Data` resources become the game's art
  and audio. A resources-only Blorb sitting *beside* the story counts too.
- **ZIP archives** — the first `.z3`/`.z5`/`.z8` entry is unwrapped in memory.
- **Amiga `.adf` disk images** — the original release floppy, played as it shipped.

That last one is worth its own paragraph. Infocom's Amiga releases came on 880 KB
floppies, and the disk images those turned into are still how the graphical
titles circulate in their native form. Hand babelmap one — `babelmap "Zork
Zero_Disk1.adf"` — and it mounts the AmigaDOS filesystem (both OFS and FFS),
walks it, and plays what it finds. No unpacking step, no loose files, nothing to
rename.

AmigaOS has no filename extensions to go by, and while Infocom's convention was
`Story.data` beside `Pic.data`, the convention is not a promise — one Zork Zero
disk lists a file in its own manifest that was never written to it. So babelmap
identifies the story by **content**: a Z-machine header whose version, memory
map, serial, and declared length all agree with the bytes actually present. The
two saved games sitting on the Zork Zero disk look superficially like v3 stories
and are rejected on exactly those grounds. Conventional names only break a tie if
a disk somehow offers two candidates; a disk with none — the plain AmigaDOS boot
floppy that ships as Disk 0 — says so instead of booting a system library.

The artwork comes along for free: a native Infocom picture archive on the *same*
image is that story's art, because a shared floppy is as strong a guarantee of
pairing as a Blorb is. Loose archives are a different matter — babelmap will use
one, but only if you name it, and it never guesses from a filename. See
[Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws).

Disk images are first-class in the library too: point babelmap at a directory of
them and the picker's TYPE column names the container alongside the format —
`Z6 (ADF)` — from the same content-based identification, so a floppy is never
listed as a bare story file. See [Story picker](interface.md#story-picker).

### A floppy is a different release

Worth knowing before you compare two runs: the disk image is not the same story
as the `.z6` sitting beside it. It is a different **build** of the game, and the
builds do not always behave alike. *Journey*'s floppy is release 30; the bare
story file is release 83 — and where r83 narrates through window 0, r30 narrates
through window 2. A screen rule that is right on one of them can be wrong on the
other, which is exactly what happened once.

What each medium carries, measured across the collection:

| Title | Amiga floppy | Bare story file |
| --- | --- | --- |
| Journey | release 30, serial 890322 | release 83, serial 890706 |
| Zork Zero | release 366, serial 890323 | release 393, serial 890714 |
| Shogun | release 295, serial 890321 | release 322, serial 890706 |
| Arthur | release 54, serial 890606 | release 74, serial 890714 |
| Beyond Zork | release 57, serial 871221 | release 57, serial 871221 |
| Zork I | release 88, serial 840726 | release 88, serial 840726 |
| Zork II | release 48, serial 840904 | release 48, serial 840904 |
| Zork III | release 17, serial 840727 | release 17, serial 840727 |
| Zork: The Undiscovered Underground | release 16, serial 970828 | — |

Every graphical title ships a *different* build on its floppy; the v3/v5 ones
ship the same build on both media. A resource `.blb` beside a story is never a
third build — it holds artwork and no executable, so the release you play is
decided entirely by the file you open.

The practical rule, and the one the interpreter's own tests follow: a report made
on a disk image is reproduced on that disk image, and a finding names the release
it was measured on. `crates/app/tests/suites/real_media_releases.rs` pins this whole
table, plus the frame each build lays out, so an upgraded fixture announces
itself instead of quietly rebasing someone's investigation.

## Z-machine

- **Standard Quetzal save/restore** — the game's own SAVE/RESTORE writes and reads
  the interchange Quetzal format, so a save you make here opens in Frotz and vice
  versa.
- **Story-dictionary introspection** — babelmap reads the game's built-in word list
  and turns it into live verb/noun autocomplete, so you type `exam` and the game's
  actual vocabulary completes it.
- **v4+ upper-window screen model** — cursor-addressed status lines and full-screen
  forms (Bureaucracy's infamous licence application, for one) render in a fixed
  grid pinned atop the transcript, and `read_char` keystrokes are forwarded so you
  fill those forms in place. The game is told the story pane's **real** size — the
  standard asks the interpreter to keep the current height and width in the header
  and lets it change them whenever it likes, so babelmap measures the pane and
  re-measures it on every terminal resize. A game's full-width form therefore lines
  up column-for-column with the prose beside it instead of floating in a fixed
  80-column box. Pin a fixed screen with `virtual_screen_cols`/`virtual_screen_rows`
  if you want a game's original layout back; when the pane is smaller than a pinned
  screen, the viewport auto-follows the cursor. The virtual window is themeable
  from `[elements]`: `upper_window` inks its cells, and `upper_window_border`
  both colours and shapes the frame around them. That frame is off by default —
  the bar sits flush against the story and the game keeps every row and column
  of the pane — so set `style = "single"` (or `double`/`thick`/`rounded`) if you
  want it boxed.
  During a `read_char` prompt keystrokes go to the game; only the hotkey prefix
  (default `Ctrl+P`) stays reserved.
- **Timed / interrupt input** — v4+ `read` and `read_char` honor their `time`+
  `routine` operands, so real-time games keep ticking while you think: the game's
  interrupt routine fires every N tenths of a second (countdowns and clocks — the
  bomb in Border Zone) and can cut the read short. Controlled by
  `honor_timed_input` (default on), the `/toggle-timed-input` command, and the
  settings row; `zvm-cli` takes `--no-timed-input`. The VM stays zero-dependency —
  the wall clock lives in the hosts, not the interpreter.
- **Interpreter number** — the story header's interpreter number (byte `0x1E`)
  defaults to **1 (DECSystem-20)**, following Frotz's rule (6 / IBM PC only for
  v6). This byte is what unlocks colour on several Infocom games: Beyond Zork, for
  instance, only emits colour to a non-IBM interpreter and falls back to
  reverse-video under IBM PC. Override it with the app's `interpreter_number` config
  key, `babelmap --interpreter-number N`, or `zvm-cli -I N` / `--interpreter N` —
  e.g. `6` selects the IBM PC path, which draws Beyond Zork's map box and cursor
  arrows as CP437 character graphics instead of Font 3. The `--interpreter-number`
  flag applies to one run only and is never written back to your config, so probing
  a game's behaviour can't quietly pin one machine for every story afterwards —
  unless you then set the value in the settings screen, which is a decision rather
  than a flag and persists like any other setting. Setting that row back to
  **default** removes the key, restoring the per-version rule on the next launch. The
  values are ZMSD §11.1.3's:

  | | | | |
  |---|---|---|---|
  | 1 DECSystem-20 | 4 Amiga | 7 Commodore 128 | 10 Apple IIgs |
  | 2 Apple IIe | 5 Atari ST | 8 Commodore 64 | 11 Tandy Color |
  | 3 Macintosh | 6 IBM PC | 9 Apple IIc | |
- **Interpreter profiles — the whole machine, not one byte.** Byte `0x1E` is not
  the only thing that makes a machine. A Version 6 game that reads it goes on to
  ask about the screen it has, the colours the interpreter calls default, and what
  "red" looks like here — and answering one of those as an Amiga while answering
  the rest as an IBM PC produces a machine that never existed. So the answers
  travel together as a named **profile**.

  **IBM PC** is the default and is simply what babelmap has always done: the
  Frotz interpreter-number rule above, the resource file's own declared art
  resolution, your terminal's colours reported as the interpreter defaults, and
  ZMSD §8.3.1's colour table.

  **Amiga** is the sibling, and it selects itself: a story booted straight out of
  an `.adf` release floppy came off an Amiga, so babelmap presents one — 
  interpreter number 4, the Amiga's 320×200 standard window (which is what makes
  the artwork in a native `Pic.data` archive scale onto the 640×400 screen, since
  that format has no `Reso` chunk to declare it), a medium grey page and white ink
  reported as the interpreter's defaults, and the palette Infocom's own Amiga
  interpreter loaded — a slightly darker green and blue than the standard's, and
  its own three Version 6 greys.

  The artwork can select the machine too, and it sits between the two. If you
  name a picture archive for a game — the `pictures` key described under
  [Choosing which artwork a game draws](v6-graphics.md#choosing-which-artwork-a-game-draws) —
  then you have said which machine's rendition you want to look at, and babelmap
  presents that machine: a `Pic.data` is an Amiga, an `.MG1`/`.EG1`/`.CG1` is an
  IBM PC. It reads that from the file's *contents*, never its extension, since
  the two containers are structurally different and a renamed file would
  otherwise lie about which machine you asked for. (The Macintosh wrote the same
  container as the Amiga and cannot be told apart from it in general, so it is
  not claimed to be.) MCGA, EGA and CGA are three video cards in one machine, so
  all three name the IBM PC and none of them moves byte `0x1E`; what a card does
  change is how densely its artwork was stored, which is
  [the art's business rather than the machine's](v6-graphics.md#choosing-which-artwork-a-game-draws).
  The character cell is 8×16 on every profile — EGA's own 640×200 mode on an 8×8
  cell is the same 80×25 grid — so no rendition alters the screen a game is
  handed. Setting `interpreter_number` yourself names the
  machine outright and outranks both, so `interpreter_number = 4` gets you
  the whole Amiga rather than just the byte — which is the point: a number that
  changed what games did without changing the machine it implied was never a
  useful thing to be able to set.

  You can set it per game as well as globally. The
  [launch-options dialog](v6-graphics.md#three-ways-to-say-it) — **Shift-Enter**
  on a story in the browser — shows the number your art choice implies *and where
  it came from*, lets you pin a different one for that launch, and will write
  `interpreter_number` into the game's own `config.toml` if you tick the box.
  Most specific first: the dialog's choice for this launch, then
  `--interpreter-number`, then the game's sidecar, then the global config, then
  the inference above. It belongs in a *launch* dialog rather than the settings
  screen because header byte `$1E` is read by the story itself at boot — a game
  that has already started has already made decisions from it, so offering to
  change it mid-session would be offering something babelmap cannot deliver.

  Authenticity can cost readability — *Zork Zero* under an Amiga picks a colour
  scheme that was easy on a 1989 monitor and is merely adequate in a modern
  terminal. There is no separate switch for that on purpose: `honor_game_colours`
  already decides whether the game's colour choices are honoured at all, so
  turning it off hands the screen back to your theme, profile or no profile.
- **v6 graphical stories** — babelmap boots and plays graphical v6 titles,
  verified against *Zork Zero*'s full frame. On an image-capable terminal
  (Kitty / iTerm2 / Sixel) the game's chrome — the decorative frame, status
  line, and per-room compass — renders as one scaled, **pixel-aspect-accurate**
  image (uniform scaling, letterboxed, never stretched); the game itself lays
  this out by querying invisible "placement" pictures, which babelmap answers
  from the Blorb's own dimension data. The `v6_render` setting (see
  Customization) picks how the story text is drawn: the default `hybrid` mode
  keeps it as real, crisp terminal text inside the chrome; `raster` bakes it
  into the pixel image instead, bitmap-font style. Without an image protocol,
  v6 falls back to a character-cell rendering. Full depth — the three render
  modes, inline drop-caps, pixel-positioned status text and colour — is in
  [Graphical v6](v6-graphics.md). (v6's menu and mouse opcodes are not yet
  wired up.)

## Glulx

- **External files** — Glulx games persist their own data through Glk file streams;
  a game's fixed-name saves and caches are read and written for it silently. (See
  [saves](saves.md) for how this dovetails with babelmap's Save States.)
- **Accelerated-function interception** — big Glulx games reach the first prompt
  dramatically faster. Well-known Inform veneer functions the game registers via
  `accelfunc` are recognized and run natively instead of grinding through full VM
  dispatch, so a heavyweight like Counterfeit Monkey stops making you wait through
  its startup. On by default; disable with `--no-accel` (`gvm-cli` and the app).
- **Floating-point math** — the complete float opcode set is implemented, in both
  single **and** double precision: conversions, arithmetic, `sqrt`/`exp`/`log`/
  `pow`, trigonometry, and the fuzzy comparisons `jfeq`…`jisinf`. Games that
  compute with floats — Counterfeit Monkey's in-game graphics scaling, say — run
  instead of faulting, and the `gestalt` opcode answers `Float` and `Double`
  truthfully so a game can probe first.
- **Line-input terminators** — babelmap honors `glk_set_terminators_line_event`, so
  a game can register special keys (Escape and the function keys `Func1`–`Func12`)
  that end a line of input; the terminating keycode comes back in the line event's
  second value (`val2`; `0` for a normal Enter).
  `glk_gestalt(gestalt_LineTerminators/LineTerminatorKey)` answers truthfully so
  games can check before relying on it.

## Sound

- **Z-machine** — the `sound_effect` opcode's two built-in bleeps (#1 high / #2 low)
  play as real synthesized tones, and Blorb `Snd ` resources (#≥3) play as sampled
  audio (AIFF, Ogg, or ProTracker MOD), in both the `app` TUI and `zvm-cli`. Sound
  resources come from the story file itself if it's a Blorb, else from a sibling
  `.blb`/`.blorb` next to it. On every bleep the story-pane border also flashes in
  a distinct, themeable colour (`sound_beep_high` / `sound_beep_low`) — a
  complementary and accessibility cue, and the *only* cue when sound is off.
  Controlled by `enable_sound` (default on) and `volume` (0–100, default 100);
  toggle it with `/toggle-sound` or the `F2` settings row, adjust it with
  `/volume <0-100>`, and use `/play-sound <resource-id>` to fire a Blorb `Snd `
  resource on demand for verifying the audio path. Both the `app` and `zvm-cli`
  take `--no-sound` to start muted for a single run (leaving `enable_sound`
  untouched); `zvm-cli` also takes `--volume <0-100>`.
- **Glulx** — Glk sound channels (`glk_schannel_*`) play a Blorb's AIFF/Ogg/MOD
  `Snd ` resources with per-channel volume (including gradual volume ramps) and
  sound-finished notify events, so music and effects behave the way the author
  wired them.

Sound always plays on the local device babelmap runs on; to route audio from a
remote/SSH session back to your own machine, see
[`docs/remote-sound.md`](../remote-sound.md). Unimplemented-opcode warnings
surface in the transcript as meta lines (hidden by `/filter story`) rather than
spilling onto stderr.

## Game-driven colour

When a game asks for colour, babelmap gives it colour — on your terms. The
Z-machine's v5+ `set_colour` and `set_true_colour` are honored: the standard
palette (black/red/green/…) maps onto *your* colour scheme, so a game's "red" is
your red rather than a hard-coded shade, while greys and true-colour render as
exact 24-bit RGB. Colour and reverse-video apply in both the transcript and the
upper-window grid. **Glulx/Glk** games get the same treatment —
`stylehint_TextColor`/`BackColor`/`ReverseColor` render at full 24-bit fidelity.

It all sits under one switch, `honor_game_colours` (default **on**): flip it in the
F2 settings screen to let your theme own every colour instead. `zvm-cli` and
`gvm-cli` render the same colours as ANSI SGR and both accept `--no-game-colours`
to opt out, as does setting `NO_COLOR` to a non-empty value.

One thing turns it off for you. A game drawing **two-colour (CGA) artwork** is
told the interpreter has no colours, because it has none — that artwork is a
stencil whose own white is paint and whose transparency is meant to show your
background through, and a story that thinks it is on a colour display paints over
both. See [The colours come with the card](v6-graphics.md#the-colours-come-with-the-card).
It applies to that story only and is never written back to your config, so
choosing a `.cg1` once cannot quietly strip the colours from every other game.

## Plain text, for screen readers

All three CLIs accept **`--screen-reader`** (alias `--plain`), and select it
automatically under `TERM=dumb`. It emits no escape sequences at all: no colour,
no cursor addressing, no scroll region, no pinned status line, no alternate
screen — just linear, append-only text a screen reader can follow and scrollback
can review. `[MORE]` paging goes too, since a blocking prompt that hides the rest
of the output behind a keypress is the shape a reader copes with worst. Line
editing and echo go back to the terminal, so the reader announces typed
characters and the user's familiar editing keys work.

What would otherwise be spatial arrives in reading order instead: the Z-machine
status line and upper window come through as ordinary lines, and Glk TextGrid
windows stream inline, deduped so an unchanged status bar doesn't repeat every
turn. **Menus** get more than that — see below.

**The status line is not narrated every turn.** A Z-machine v3 status line
carries a move counter, so it differs on every single turn and no amount of
change-detection will suppress it — measured, Ballyhoo repeats it on four turns
out of four. Screen-reader mode therefore leaves it out and lets you ask with `/status`.
`--show-status` puts it back if you would rather have it whenever the story
updates it.

The suppression goes by *size*, and only a one-row region is treated as chrome.
Anything taller is content the game means you to read: the Infocom releases with
integrated InvisiClues draw their hint menus in the upper window — Planetfall's
is twelve chapter headings and a `RETURN = See hint / Q = Resume story` legend —
and Lost Pig's HELP menu and Bureaucracy's licence-application form are the same
shape. Those always come through. **`--story-only`** is the blunt instrument for
anyone who wants the whole upper window gone, menus included — it is deliberately
a separate, stronger switch, and it works with or without `--plain`. `gvm-cli`
takes it too, where it suppresses every Glk grid window. (Scott has no status
window to suppress: its room block *is* the story.)

The status also lands in the right place. A game writes its prompt last and
without a trailing newline, and the host only learns the turn is over when the
game asks for input — so a naive host can only append the status *after* the
prompt, giving `> In the Wings   Score: 0`, which reads as though the prompt were
showing you a room. In this mode the prompt is held back until the status has
gone out, so a turn reads description, then status, then prompt. `/status`
answers the same way, and puts the prompt back after itself.

`scott-cli` drops its em-dash divider rule in this mode. It stands in for the
boundary a real Scott terminal drew between its two windows, and a reader either
announces thirty-odd em-dashes one at a time or swallows the line — neither of
which conveys a boundary.

### Menus are numbered, and a move is one line

A menu is a rectangle the game repaints: a list with a `>` parked on the current
item and a legend saying which keys move it. Sighted, the marker jumps and
nothing else happens. Linearised, *every repaint is a fresh block of text* —
measured, `N` at Planetfall's InvisiClues menu read out sixteen lines, and
Arthur's read out twenty-three, on every single press, to say that a `>` had
moved down one row. Followable, but not usable.

So in screen-reader mode the host recognises the repaint. A menu is read out
**once**, host-numbered:

```
                               INVISICLUES (tm)
 N = Next                                                     P = Previous
 RETURN = See hint                                        Q = Resume story

[menu — type a number to jump, Enter to select]
>1. THE FEINSTEIN
 2. THE POD TRIP
 3. THE DORMITORY
 …
```

and after that a marker move is announced in one line:

```
>3. THE DORMITORY (3 of 12)
```

**Detection is a mechanical diff, not a guess about content.** The host keeps the
last block emitted from each source (the Z-machine upper window; each Glk grid;
the Glk story stream) and compares. If two blocks differ *only* in where the
marker sits — same items, same headers, same legend — that is navigation. Any
other difference is content and is emitted in full, unchanged. A status line
whose text changed, a menu that scrolled, a form that gained a field: all differ
somewhere other than the marker column, so none of them is ever swallowed. This
is the whole safety argument, and it is pinned by tests on both engines.

**Which lines get numbers** is decided by shape: an item is a non-blank line
whose text begins at the same column as the marked line's, with nothing but
blanks and marker characters in front of it. That is exactly the items in all
three measured menus and none of their furniture — Arthur's centred title
(column 20), its `N = next item` legend (column 1) and its `(more)` pagination
hint (column 4) are all left unnumbered, as are Planetfall's title and two
legend rows. The rule errs towards numbering more lines rather than fewer: an
over-numbered header is an annoyance, an unreachable item is a dead end. (The
alternative — numbering only the lines the marker has been seen on — renumbers
the menu under the player as they explore it.) A list the game repaints twice
into one block, as Counterfeit Monkey's does, counts once.

**Typing a number jumps to that item.** The host cannot teleport the marker — the
game owns it — so it walks the menu with the game's own keys: `n`/`p` when the
legend names them (`N = Next`, `P = previous item`), else Down/Up (ZSCII 129/130
for the Z-machine, `keycode_Up`/`keycode_Down` for Glk). It steers rather than
counting: press, read where the marker actually landed, decide again — because
Arthur's `N` steps straight over its unselectable section headings, and a
press-count worked out in advance would sail past the item you asked for. The
landing is announced in the move format; the intermediate steps are silent; and
an ordinal the marker will not stop on gives up and reports where it ended
instead of pressing forever.

Numbers are only intercepted while a menu is open, and only for an ordinal the
menu actually has. Everything else — `n`, `p`, Enter, `q`, a digit at an ordinary
prompt — reaches the game untouched.

**`/menu`** re-reads the open menu, numbered, on demand, and says
`[no menu is open]` when there isn't one. It is the `/status` precedent, and
because screen-reader mode leaves the terminal cooked, a menu "keypress" is
really a whole line terminated by Enter — so `/menu` and multi-digit jumps work
at a menu's own prompt, not just at a line prompt. (That termination rule is not
a choice: it is the shape of the read. Raw mode would deliver `1` then `2` with
no way to tell `12` from item 1 followed by item 2.)

None of this applies outside `--screen-reader`. On a terminal the menu is painted
in place and nothing repeats; on a plain pipe the output is a transcript that
stays byte-identical.

### Score changes are announced

Quietening the status line takes the score with it, and the score is the part
that carries news — a sighted player watches it tick over, a listener would have
to keep asking. So in screen-reader mode a score that *moves* is announced above
the prompt:

```
You put the gold idol on the pedestal.

[Score 1, up 1]
>
```

Only on change, never on the first sighting (the score you started with is not an
event), and words rather than `+1`, because a reader announces "plus" only at
higher punctuation settings.

Where the number comes from differs sharply by format, and two of the three are
exact while the rest is pattern-matching:

| | source | |
|---|---|---|
| Z-machine v1–v3 | global 2, which the standard reserves for the score (ZMSD §8.2) | exact |
| Z-machine v4+ | the status line the game drew | recovered from text |
| Glulx | the Glk grid window — Glk has no concept of a score at all | recovered from text |
| Scott Adams | treasures deposited in the treasure room, recounted each turn | exact |

The text-recovery cases look for a `Score: N` field and take the last one on the
line, so a room called "Score Board" doesn't become your score. A game that words
it differently — "Points", a bare number, a translated status line — simply isn't
matched, and the announcement stays silent rather than reporting a wrong figure.
A Z-machine *time* game has a clock where the score would be, and is correctly
never announced.

### `/status`

Status text reaches a listener only when the game chooses to write it, and then
it scrolls away; a sighted player re-reads a pinned line for free. All three CLIs
answer **`/status`** at any line prompt with the current status — the Z-machine
status line or upper window, the Glk grid windows, or a Scott room block — and
the game never sees the command. The leading slash is what makes intercepting it
safe: no interactive-fiction parser gives `/` a meaning, so no game verb is
shadowed, and babelmap's own TUI already spells host commands that way. (A `char`
prompt — "press any key" — takes the keypress as itself; `/status` is a line
command. `/menu` is the exception, because a menu *is* a char prompt and a
command that could not be typed at one would be useless.)

This is the same output path piped/redirected use has always taken — kept honest
by the test harnesses that read it — so `--screen-reader` mostly makes it *selectable*
without giving up an interactive terminal. `NO_COLOR` deliberately does **not**
imply this mode: [the convention](https://no-color.org/) is about colour, and
someone who sets it has not asked to lose their status line.

> **Not yet validated with a real screen reader.** The escape output, input
> paths, and TTY gating are measured; NVDA/Orca/VoiceOver behaviour is not. If
> you use one, we would like to hear how this goes.

## `[MORE]` paging in the CLIs

A turn that prints more than a screenful used to scroll its own beginning away in
`gvm-cli` and `scott-cli`; only `zvm-cli` paused. All three now stop at the
bottom of a page with a reverse-video `[MORE]` bar and wait for a key, the way
the original interpreters did and the way the TUI already did. `--no-more`
(alias `--no-page`) turns it off.

Paging requires **both** ends to be a terminal — pausing for a key that a pipe
will never send is a hang, which is why the headless harnesses never see it — and
is off in `--screen-reader` by choice. `gvm-cli` pages only its streaming story
window; a game using several buffer windows is painted as fixed panels with their
own scrollback, so there is no bottom of the page to stop at.

## Robustness

When a story faults — out-of-bounds memory, stack under/overflow, an unimplemented
opcode — it doesn't take the interpreter down with it. The game halts with a
call-frame stack trace (the faulting PC and opcode, plus each frame's return
address and locals). In the app the trace appears inline in the transcript and the
app **stays interactive**: the map, scrollback, and a deliberate quit all keep
working, and a durable copy lands in `~/.babelmap/crash.log`. `zvm-cli`/`gvm-cli`
print the trace to stderr and exit 70.
