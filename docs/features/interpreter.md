# Interpreter (Z-machine, Glulx & Scott Adams)

[← back to README](../../README.md)

babelmap runs three from-scratch, zero-dependency virtual machines: a Z-machine
(`zvm`), a Glulx engine (`gvm`), and a Scott Adams / ScottFree engine (`scott`).
The format is auto-detected from the file, and all three feed the same host
features below.

- **Scott Adams (ScottFree `.dat` / SAGA)** — the classic illustrated 8-bit
  adventures (*Adventureland*, *Pirate Adventure*, …), including their vector
  line-art room graphics, played through the same TUI and live automap.

- Full play of v3/v4/v5/v7/v8 Z-machine story files.
- Standard **Quetzal** save/restore — interchangeable with other interpreters.
- Story dictionary introspection (powers verb/noun autocomplete).
- **v4+ upper-window screen model** — cursor-addressed status lines and forms
  (e.g. Bureaucracy's licence application) render in a fixed grid atop the
  transcript, and `read_char` keystrokes are forwarded so forms are fillable in
  place. The game sees a fixed, configurable virtual screen
  (`virtual_screen_cols`/`virtual_screen_rows`, default 80×24); the viewport
  auto-follows the cursor when the pane is smaller. The virtual window is
  themeable (`upper_window`, `upper_window_border`, `virtual_window_border`).
  During a `read_char` prompt keystrokes go to the game; the hotkey prefix
  (default `Ctrl+K`) stays reserved.
- **Sound effects** — the `sound_effect` opcode's two built-in bleeps (#1 high /
  #2 low) play as real synthesized tones, and Blorb `Snd ` resources (#≥3) play
  as sampled audio (AIFF, Ogg, or ProTracker MOD), in both the `app` TUI and
  `zvm-cli`. Sound resources come from the story file itself if it's a Blorb,
  else a sibling `.blb`/`.blorb` next to it. The story-pane border still flashes
  in distinct, themeable colors (`sound_beep_high` / `sound_beep_low`) as a
  complementary/accessibility cue on every bleep — and the only cue when sound
  is disabled. Controlled by `enable_sound` (default on) + `volume` (0-100,
  default 100); toggle with the `/toggle-sound` command or `F2` settings row,
  adjust with `/volume <0-100>`; `/play-sound <resource-id>` plays a Blorb
  `Snd ` resource on demand (a diagnostic for verifying the audio path).
  `zvm-cli` takes `--no-sound` and
  `--volume <0-100>`. Unimplemented-opcode warnings surface in the transcript
  as meta lines (hidden by `/filter story`) rather than on stderr.
- **Glulx sound** — Glk sound channels (`glk_schannel_*`) play a blorb's
  AIFF/OGG/MOD `Snd ` resources, with per-channel volume and sound-finished
  (notify) events, complementing the Z-machine `@sound_effect` support above.
  Sound always plays on the local device babelmap runs on; see
  [`docs/remote-sound.md`](../remote-sound.md) for routing audio from a
  remote/SSH session back to your machine.
- **Timed / interrupt input** — v4+ `read` and `read_char` `time`+`routine`
  operands are honored: while waiting for input the game's interrupt routine is
  called every N tenths of a second (real-time clocks, countdowns — e.g. Border
  Zone), and can end the read. Controlled by `honor_timed_input` (default on) +
  the `/toggle-timed-input` command and settings row; `zvm-cli` takes
  `--no-timed-input`. The VM stays zero-dependency — the wall clock lives in the
  hosts.
- **Game-driven colour** — v5+ `set_colour` and v6 `set_true_colour` are honored.
  The standard palette (black/red/green/…) maps onto your colour scheme's palette,
  so a game's "red" is *your* red rather than a hard-coded shade; v6 greys and
  true-colour render as exact RGB. Colour and reverse-video apply in both the
  transcript and the upper-window grid. Controlled by `honor_game_colours`
  (default **on**) — toggle it in the F2 settings screen, or turn it off to keep
  your theme owning every colour. `zvm-cli` renders the same colours as ANSI SGR
  and accepts `--no-game-colours` to opt out. **Glulx/Glk** games get the same
  treatment: `stylehint_TextColor`/`BackColor`/`ReverseColor` are honored and
  rendered at full 24-bit RGB fidelity in both hosts, under the same
  `honor_game_colours` gate (`gvm-cli` also takes `--no-game-colours`).
- **Interpreter number** — the story header's interpreter number (byte `0x1E`)
  defaults to **1 (DECSystem-20)** for v1–5 and **6 (IBM PC)** for v6, matching
  Frotz. This is what makes colour appear: several Infocom games (notably Beyond
  Zork) only emit colour on a non-IBM interpreter and fall back to reverse-video
  under IBM PC. Override it with the app's `interpreter_number` config key or
  `zvm-cli -I N` / `--interpreter N` (e.g. `-I 6` to select the IBM PC path,
  which draws Beyond Zork's map box and cursor arrows as CP437 character
  graphics instead of Font 3).
- **Glk line-input terminators** — the Glulx engine honors
  `glk_set_terminators_line_event`: a game can register special keys (Escape and
  the function keys `Func1`–`Func12`) that end line input, and the terminating
  keycode is reported back in the line event's second value (`val2`; `0` for a
  normal Enter). `glk_gestalt(gestalt_LineTerminators/LineTerminatorKey)` answers
  truthfully so games can probe support.
- **Accelerated-function interception** — large Glulx games (e.g. CounterfeitMonkey)
  reach the first prompt substantially faster: well-known Inform veneer functions
  set up via `accelfunc` are recognized and executed natively instead of through
  full VM dispatch. On by default; disable with `--no-accel` (`gvm-cli` and the app).
- **Floating-point math (Glulx)** — the full single-precision floating-point
  opcode set (conversions, arithmetic, `sqrt`/`exp`/`log`/`pow`, trigonometry, and
  the fuzzy comparisons `jfeq`…`jisinf`) is implemented, so games that compute with
  floats — for instance CounterfeitMonkey's in-game graphics scaling — run instead
  of faulting. `glk_gestalt(gestalt_Float)` answers truthfully.
- **Graceful VM crash reporting** — when a story faults (out-of-bounds memory,
  stack under/overflow, an unimplemented opcode), the game halts with a call-frame
  stack trace — the faulting PC and opcode plus each frame's return address and
  locals — instead of taking the interpreter down. In the app the trace appears
  inline in the transcript and the app **stays interactive** (the map, scrollback,
  and a deliberate quit still work); a durable copy is written to
  `~/.babelmap/crash.log`. `zvm-cli`/`gvm-cli` print it to stderr (exit 70).
