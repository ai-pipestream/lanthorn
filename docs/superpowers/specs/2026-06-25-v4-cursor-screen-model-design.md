# v4+ Cursor-Addressed Screen Model — Design

**Status:** Approved; PARKED behind the v4+ opcode-completeness pass
**Date:** 2026-06-25

> **Prerequisite (decided 2026-06-25):** a v4+ opcode-completeness pass runs first —
> it implements `scan_table`, `copy_table`, `print_table`, `get_cursor`, `erase_line`
> and turns the silent VAR fallthrough into a warning. `get_cursor`/`erase_line` are
> therefore implemented there, not here; this spec consumes them. Without that pass,
> v4+ games (incl. Bureaucracy) malfunction regardless of the screen model.

## Goal

Render the Z-machine **upper window** (a cursor-addressed character grid) and drive
real-time `read_char` input, so v4+ status lines display correctly and cursor-addressed
forms — e.g. Bureaucracy's licence-application form — are fillable in place. lanthorn
today implements only the v3 model (scrolling text + a derived status line); the v4+
windowed model is stubbed (window/cursor *state* is tracked, but there is no grid, no
rendering, and only line input).

## Background (from investigation)

- VM: `ScreenState` tracks `current_window`, `cursor_row/col`, `upper_window_rows`,
  `text_style`, but cursor-positioned writes fall into the linear output stream — there
  is **no upper-window grid**. `read_char` (opcode 0x16) **is** supported at the VM level
  (suspends with an input request), as is `read` (line).
- App: never reads or renders the upper window; the session exposes only line input
  (`submit(&str)`). So v4+ status lines are not shown and `read_char` forms cannot work.
- A prior fix already seeds the screen-dimension header bytes (`write_screen_dims`,
  default 24×80) so size-sensitive games boot instead of aborting "[Screen too small.]".

## Design

### 1. Fixed virtual screen

The game always sees a **fixed, configurable virtual screen**, independent of the terminal
size, so its layout never desyncs on resize.

- New config keys: `virtual_screen_cols` (default **80**) and `virtual_screen_rows`
  (default **24**).
- On story load the app calls `Machine::set_screen_dims(rows, cols)` with these values,
  overriding the init-time default. The size is fixed for the session.
- Resize never changes what the game sees; only the viewport into the grid changes
  (section 3).

### 2. VM: upper-window grid

Add an upper-window character grid to the VM (in `ScreenState` or an adjacent struct):

- A `cols`-wide grid of `upper_window_rows` rows; each cell is `{ ch: char, style: u8 }`
  where `style` is the Z-machine text-style bitmask (bit1 bold, bit2 italic, bit3 fixed,
  bit4 reverse).
- Output routing: when `current_window == 1`, printed characters are written into the grid
  at `(cursor_row, cursor_col)` using the current `text_style`, advancing the cursor and
  clamping to the window bounds (no scroll within the upper window — standard behavior).
  When `current_window == 0`, output streams to the transcript exactly as today.
- `split_window N` allocates/resizes the grid to N rows (× `cols`); rows beyond the new
  size are dropped; `N == 0` removes the upper window. `set_window` switches routing.
  `set_cursor r c` moves the cursor (clamped). `erase_window`/`erase_line` clear the grid
  or the current line. `erase_window -1` also unsplits (rows → 0).
- New accessors on `Machine`: `upper_window() -> UpperWindowView` (rows of cells + size),
  `cursor() -> (u16, u16)`, and `pending_input() -> Option<InputKind>` (`Line` | `Char`).

### 3. App: rendering

New module `crates/app/src/render/upper_window.rs`.

- When `upper_window_rows > 0`, the story pane splits: the **top region is the fixed,
  non-scrolling upper-window grid**; the scrolling transcript renders below it. The map
  pane is unchanged.
- Each cell renders with its style (reverse-video and bold honored). The window is themed
  (section 5).
- **Viewport:** the grid is `virtual_cols` wide. If the story pane is narrower or shorter
  than the virtual size, the viewport clips and **auto-follows the game's cursor** (keeps
  the active field on-screen during input). A status-line hint ("widen for full form")
  shows while the grid exceeds the pane. No form corruption on resize — the virtual grid
  is constant; only the viewport offset changes.

### 4. Input

- The session exposes the VM's pending input kind. On `InputKind::Char` (a `read_char`),
  the event loop forwards the **next single keystroke** to the VM via a new
  `GameSession::submit_char(key)` (bypassing the bottom line buffer); the game echoes the
  char by printing into the grid, which re-renders. `InputKind::Line` play is unchanged
  (bottom line buffer + submit on Enter).
- The bottom input prompt is hidden during `read_char`; a cursor block renders in the
  upper window at the game's cursor position.
- **Escape-hatch:** during `read_char` mode keystrokes go to the game, EXCEPT the
  configurable hotkey prefix (default Ctrl-K), which still opens lanthorn's controls so
  the user is never trapped.

### 5. Theming

The virtual window is themeable via `style.toml`, consistent with the existing chrome:

- `upper_window` — text fg/bg (the window's default cell colors; per-cell reverse/bold
  from the game's `text_style` still apply on top).
- `upper_window_border` — border color.
- `virtual_window_border` — a `BorderStyle` key (`none`/`single`/`double`/`thick`/
  `picture-frame`), **default `single`** so the upper window is visually delineated from
  the scrolling transcript. The border is drawn around the rendered grid region; with
  `none`, just a background fill.
- Selectors integrate into the existing `colors.rs`/`style.rs` selector list and the
  `style.toml` schema (kept current).

### 6. Status lines (free win)

v4+ status lines are drawn by the game into the upper window's top row each turn, so they
render correctly once the grid exists — fixing status-line display for v4+ games broadly,
not just form games.

## Error handling

- Grid writes and cursor moves clamp to bounds; an out-of-range `set_cursor` is clamped,
  never panics.
- Virtual screen ≥ the configured size always; size-sensitive games therefore never see
  "too small" at the default 80×24.
- Resize during a form: virtual size unchanged, viewport re-clips/auto-follows; no corruption.
- `submit_char` when the VM is not awaiting a char is a no-op.

## Testing

- **VM:** write-at-cursor lands in the grid cell; cursor advance / wrap / clamp; `split_window`
  sizes the grid and `erase_window -1` unsplits; window routing (window 0 → transcript,
  window 1 → grid); `read_char` suspend/resume round-trip; `text_style` recorded per cell.
- **App:** render the grid to a test buffer (including a reverse-video status row and the
  themed border); char-mode forwards a single key and hides the bottom prompt; viewport
  auto-follows the cursor when the grid exceeds the pane; Ctrl-K still escapes during char mode.

## Scope (YAGNI)

**In:** upper-window grid + cursor addressing, `read_char` real-time input, text styles
(bold/reverse), the fixed configurable virtual screen, themeable virtual window, v4+
status-line rendering.

**Out:** lower-window cursor addressing, timed/interrupt input (`read` with a timeout
routine), v6 graphics / multiple windows, true font-pixel metrics (font stays 1×1).
