# zvm-cli Basic Screen Model (DOS-equivalent upper window + v3 status line) — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/zvm-cli` (frontend only — no `zvm` engine changes)

## Goal

Make `zvm-cli` a basic-but-complete interpreter equivalent to the old DOS
Infocom games: render the Z-machine **upper window** (v4+ menus/forms such as
the Leather Goddesses hint screen, Bureaucracy's form) and the **v3 status
line** (location + score/turns or time) as a pinned region at the top of the
screen, with the lower window scrolling below. This fixes the known bug where
`hint` produces no visible output under `zvm-cli` (the hint menu renders into
the upper window, which the CLI currently drops), and brings `zvm-cli` up to
DOS-equivalent feature parity.

The engine already tracks the entire screen model; this is purely a frontend
rendering addition in `zvm-cli`.

## Background (current code)

- `zvm` screen model (`crates/zvm/src/screen.rs`): `ScreenState` holds
  `upper: UpperWindow` (grid of `Cell { ch: char, style: u8 }`, `cols`/`rows`,
  `cell(row, col)`, 1-based), `upper_window_rows: u16` (0 = no split),
  `current_window: u8` (0 = lower, 1 = upper), `cursor_row`/`cursor_col`
  (1-based), `text_style: u8`, `show_status_requested: bool`. Opcodes
  `split_window`/`set_window`/`set_cursor`/`erase_window`/`set_text_style`
  already maintain all of this (`crates/zvm/src/cpu/exec.rs`). Text printed
  while `current_window == 1` lands in the grid and never reaches the `Output`
  sink.
- v3 status line: `Machine::status_line() -> StatusLine { location: String,
  right: StatusRight }`, where `StatusRight = ScoreTurns { score: i16, turns:
  u16 } | Time { hours: u8, minutes: u8 }` (`compute_status_line`). v3
  `show_status` (0OP:0x0C) sets `screen.show_status_requested`.
- Sound bleeps: `sound_effect` (VAR:0x15) already records the built-in bleeps
  into `Machine::pending_beeps: Vec<Beep>` (`Beep::High` = sound #1,
  `Beep::Low` = sound #2) for the host to drain; sampled sounds (#≥3) only push
  a diagnostic. `zvm-cli` never drains `pending_beeps`, so bleeps are silent.
- Header: `Machine::init_caps()` (called in `build_machine`) seeds 80×24 screen
  dims (`DEFAULT_SCREEN_COLS = 80`, `DEFAULT_SCREEN_ROWS = 24`) and clears the
  "status line not available" bit. These are the game-visible dimensions.
- `crates/zvm-cli/src/main.rs`: `StdoutOutput` writes the lower-window stream
  directly to stdout. The host loop steps the machine and handles
  `NeedLine`/`NeedChar`/`Quit`/`Restart`/`SaveRequest`/`RestoreRequest`. The
  upper window and v3 status line are never read or shown.
- `zvm-cli` is zero-dependency (depends only on `zvm`) and doubles as a headless
  regression/debugging harness (piped stdin → captured stdout).

## Design

A new module `crates/zvm-cli/src/screen.rs` owns all screen-model rendering.
`main.rs` calls into it; `StdoutOutput` continues to stream the lower window
unchanged. **No engine changes.**

### 1. Game-visible dimensions stay fixed (determinism)

Game-visible header dims remain **80×24** (today's `init_caps` default). They
are **never** derived from the real terminal, so game behavior (word-wrap,
header values the game reads) is identical and reproducible across terminals
and CI. The real terminal row count is used **only** for the cosmetic TTY
scroll-region bottom (§3); it never touches game state.

### 2. TTY detection and mode selection

`std::io::IsTerminal` (std, zero-dep) on stdout selects the mode at startup:

- **Interactive (stdout is a TTY):** pinned ANSI region (§3).
- **Headless (piped/redirected):** inline plain-text block (§4).

The `--no-status` flag (§5) forces headless behavior with the block suppressed,
i.e. byte-for-byte today's lower-stream-only output.

### 3. Interactive rendering — pinned top region (ANSI)

A `ScreenView` struct tracks whether "screen mode" is active and the last
scroll-region height, and emits ANSI control strings to stdout.

- **Active region height** `top_rows`:
  - story version < 4 (v1–v3): always `1` (the status line is shown from the
    first prompt, as DOS interpreters do); a redraw is also triggered on
    `show_status_requested`.
  - v4+ with a split: `screen.upper_window_rows`.
  - otherwise `0` (no region; behaves like today).
- **Entering / resizing the region** (when `top_rows` changes from its previous
  value, including the first time it becomes > 0): set the scroll region so the
  lower window scrolls below the pinned rows:
  `ESC [ {top_rows+1} ; {term_rows} r`, then move the cursor into the lower
  region. `term_rows` comes from §6 (default 24). A v4+ split that grows or
  shrinks re-emits this with the new height.
- **Redraw the top region** (cursor saved/restored so lower-window flow is
  undisturbed): `ESC 7` (save cursor); for each row `r` in `1..=top_rows`:
  `ESC [ {r} ; 1 H`, `ESC [ 2K` (clear line), then the row's text with SGR:
  - v4+ rows: each `screen.upper.cell(r, c)` for `c in 1..=80`, mapping
    `Cell.style` bits to SGR (`0x01` → reverse `ESC[7m`, `0x02` → bold
    `ESC[1m`), reset (`ESC[0m`) at row end.
  - v3 row 1: reverse-video (`ESC[7m`) full-width bar — `location` left, the
    right field (`"score/turns"` or `"HH:MM"`) right-aligned, space-padded to
    80, `ESC[0m`.
  Then `ESC 8` (restore cursor).
- **Leaving screen mode** (`erase_window(-1)` unsplit → `top_rows` back to 0, or
  at quit): reset scroll region `ESC [ r`, cursor to bottom row.

### 4. Headless rendering — inline plain-text block (default)

No ANSI. Before each input prompt, compute the top region as a plain-text
block (each row's cells as chars; the v3 status row as
`"{location}  {right}"`; trailing spaces trimmed; a single trailing newline).
Maintain a cached `last_block: Option<String>`; emit the block to stdout only
when it is non-empty **and differs** from the last emitted block (dedupe →
deterministic, low-noise transcripts). The lower-window stream is otherwise
byte-identical to today.

### 5. `--no-status` flag

Parsed in `main` alongside the story path (`--no-status` or `--lower-only`).
When set: never emit the top region in any mode (no ANSI, no inline block);
output is byte-for-byte identical to today's `zvm-cli`. For golden-transcript
diffing and quiet engine debugging.

### 6. Terminal size

`term_rows`/`term_cols` are needed only for the cosmetic TTY scroll region.
Resolution order, all zero-dep: `stty size` via `std::process::Command`
(parse `"rows cols"`) → env `LINES`/`COLUMNS` → default **24×80**. Failure at
any step falls through to the default; never panics. (Game-visible dims remain
fixed 80×24 per §1 regardless.)

### 7. Loop integration & redraw cadence

In `main.rs`, hold a `ScreenView`. Redraw the top region **just before
blocking for input** (`NeedLine`/`NeedChar`) and whenever
`screen.show_status_requested` is set (then clear the flag) — by that point the
VM has finished its output burst, so the grid/status are fully populated. At
`Quit` (and before `process::exit`), leave screen mode (§3) so the terminal is
restored. `Restart` resets the `ScreenView` (new machine, region cleared).

After each `step()` (next to the existing `diagnostics` drain), drain
`machine.pending_beeps` and emit the bleep bytes (§8).

Input stays **cooked line input** (echoes in the lower region). The
`ScreenView` render functions are pure (`&ScreenState`/`&StatusLine` + dims →
`String`); `main` writes the returned strings to stdout.

### 8. Sound bleep (terminal BEL)

After each `machine.step()`, drain `machine.pending_beeps`. For each bleep,
emit a terminal bell byte `\x07` to stdout **only when stdout is a TTY** (§2) —
the DOS PC-speaker bleep, basic form. When piped/non-TTY, drain and discard (no
`\x07` in captured output). The high/low pitch distinction is not reproduced (a
terminal bell is a single tone). Independent of `--no-status` (audio, not the
status display); sampled/Blorb sounds remain unsupported (engine still records
a diagnostic for those). A pure helper `bleep_bytes(count, is_tty) -> &str`
returns `"\x07"`-repeated or empty.

## Testing

All render logic is pure string production — unit-tested in
`crates/zvm-cli/src/screen.rs` against hand-built `ScreenState`/`StatusLine`,
no real terminal:

- v3 status row: location left + `ScoreTurns`/`Time` right-aligned, padded to
  80, reverse-video SGR present; a long location truncates to width.
- Upper-grid row emit: cells rendered in order; `style` bit `0x01` → reverse
  SGR, `0x02` → bold SGR; SGR reset at row end.
- Scroll-region setup string for `top_rows = N`, `term_rows = M` equals
  `ESC[{N+1};{M}r`; teardown equals `ESC[r`.
- Inline block: built from a populated grid; dedupe returns `None`/empty on an
  unchanged region and the block on change.
- `term_rows` resolution: env `LINES` parsed; malformed/missing falls to
  default 24 (inject via a parse helper that takes the raw `stty`/env strings,
  so no real `stty` call in tests).
- `--no-status`: arg parsing sets the suppress flag; with it set, the top
  region producer yields nothing in both modes.
- `bleep_bytes`: N bleeps with `is_tty = true` → N `\x07`; `is_tty = false` →
  empty regardless of count.

A headless integration check (piped story) confirms the inline block appears
for a v3 game's status line and that `--no-status` reproduces the legacy
output.

## Out of scope (this feature)

- Raw single-key input for `read_char` menu navigation (cooked input means a
  key press needs Enter) — possible future enhancement.
- `[MORE]` paging at screen bottom.
- Lower-window text styling (bold/italic/reverse on the scrolling stream) —
  would require threading `text_style` through the `Output` sink; the top
  region styles are honored, the lower stream stays plain.
- Sampled / Blorb sound (numbers ≥ 3), distinct bleep pitches, graphics, v6,
  and the automap — `zvm-cli` stays a text interpreter (only the built-in
  bleeps ring a single terminal BEL per §8).
- Adapting game-visible screen dims to the real terminal (kept fixed 80×24).
- Save/restore and aux ("global state") persistence. The `save`/`restore`
  opcodes are already handled in `zvm-cli` (file-based Quetzal) and are
  unaffected by this rendering change. Cross-session persistence of the v5
  `save/restore table … name` aux tables (`machine.aux_data` / `aux_dirty`) is
  a **separate** frontend feature (its own spec) — not part of the screen
  model.

## Global constraints

- 0 warnings (`cargo build`, `cargo doc`) + full `cargo test` green per task.
- `zvm-cli` stays **zero-dependency** (std only; ANSI is bytes, `IsTerminal` is
  std, terminal size via `stty`/env). No new crates.
- **No `zvm` engine changes** — frontend only.
- Default (no `--no-status`, piped) keeps the lower-window stream byte-identical
  to today except for the added, deduped inline status/upper block; with
  `--no-status` it is byte-for-byte identical to today.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
