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
  fill those forms in place. The game sees a fixed, configurable virtual screen
  (`virtual_screen_cols`/`virtual_screen_rows`, default 80×24); when the pane is
  smaller than that, the viewport auto-follows the cursor. The virtual window is
  themeable (`upper_window`, `upper_window_border`, `virtual_window_border`).
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
  key or `zvm-cli -I N` / `--interpreter N` — e.g. `-I 6` selects the IBM PC path,
  which draws Beyond Zork's map box and cursor arrows as CP437 character graphics
  instead of Font 3.
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
to opt out.

## Robustness

When a story faults — out-of-bounds memory, stack under/overflow, an unimplemented
opcode — it doesn't take the interpreter down with it. The game halts with a
call-frame stack trace (the faulting PC and opcode, plus each frame's return
address and locals). In the app the trace appears inline in the transcript and the
app **stays interactive**: the map, scrollback, and a deliberate quit all keep
working, and a durable copy lands in `~/.babelmap/crash.log`. `zvm-cli`/`gvm-cli`
print the trace to stderr and exit 70.
