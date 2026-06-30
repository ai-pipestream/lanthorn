# Z-Machine Colour Support — Design

**Date:** 2026-06-30
**Status:** APPROVED — ready for implementation plan
**Scope:** Sub-project 1 of 2. This spec covers **Z-machine** colour
(`set_colour` / `set_true_colour` → per-cell fg/bg). Glulx/Glk colour (style
hints → per-style-class colour) is a separate follow-on sub-project, not
covered here.

## Goal

Honor game-driven colour in the Z-machine engine and render it in all three
clients (zvm-cli, gvm-cli is out of scope here, app), routed through the user's
existing colour scheme so it harmonizes with the theme rather than overriding
it. Configurable via a single `honor_game_colours` toggle, **default ON for all
clients**.

## Background / current state

Colour is declined everywhere today, by design:

- `set_colour` (2OP:0x1B, `zvm/cpu/exec.rs:420`) — graceful no-op.
- `set_true_colour` (EXT:0x05, `zvm/cpu/exec.rs:1216`) — graceful no-op.
- `init_header_caps` (`zvm/src/screen.rs:~260`) — **clears** the Flags 1 "colour
  available" bit (bit 0), so games fall back to reverse video.
- The upper-window `Cell { ch: char, style: u8 }` (`zvm/src/screen.rs:39`)
  carries no colour.
- `Output::print_styled(s, style: u8)` (`zvm/src/io.rs:18`) carries only the
  text-style bitmask.
- The app's `ColorScheme` already exposes a **16-colour ANSI palette**
  (`palette: [Color; 16]`, `app/src/colors.rs:110`), loaded from the Ghostty
  theme / built-in scheme. This is the hook that lets game standard colours map
  onto the user's theme.

## Key decision: colour routes through the user's palette

The reason colour can default ON in the app (reversing the earlier TODO
default-OFF assumption) is that the **8 standard Z-machine colours map onto the
user's themed palette, not raw RGB**:

| Z-colour | Name    | App (`scheme.palette[i]`) | CLI (SGR fg / bg) |
|----------|---------|---------------------------|-------------------|
| 2        | black   | palette[0]                | 30 / 40           |
| 3        | red     | palette[1]                | 31 / 41           |
| 4        | green   | palette[2]                | 32 / 42           |
| 5        | yellow  | palette[3]                | 33 / 43           |
| 6        | blue    | palette[4]                | 34 / 44           |
| 7        | magenta | palette[5]                | 35 / 45           |
| 8        | cyan    | palette[6]                | 36 / 46           |
| 9        | white   | palette[7]                | 37 / 47           |

So a game asking for "red" gets *the user's* red. Colour expresses through the
theme.

**The v6 greys (10 light, 11 medium, 12 dark)** are in scope but render as
**fixed RGB shades, not palette entries.** The scheme's 16-colour palette has no
faithful home for three distinct greys: mapping light grey onto the white slot
(palette[7]) collides with colour 9 (white), and the only true grey in an ANSI
palette is bright-black (palette[8]) — there is no slot for a grey *between*
black and white. So greys take the exact-colour path instead:

| Z-colour | Name        | RGB       |
|----------|-------------|-----------|
| 10       | light grey  | `#B0B0B0` |
| 11       | medium grey | `#808080` |
| 12       | dark grey   | `#505050` |

The other path that bypasses the palette is `set_true_colour`'s 15-bit RGB — an
exact colour with no named slot. Like the greys it renders as literal
`Color::Rgb(r,g,b)` (app) / 24-bit truecolor SGR `38;2;r;g;b` / `48;2;r;g;b`
(CLI). Rare (v6-era); acceptable that it bypasses the theme palette.

## Semantics (ZMSD §8.3.1, §15)

### `set_colour(foreground, background)` — 2OP:0x1B (v5+)

Per-channel **replace with sentinels** — NOT cumulative:

- `0` = **leave that channel unchanged**. Must not clobber the other channel.
- `1` = **default** — the interpreter's default fg/bg (→ `ZColour::Default`).
- `2..=12` = standard palette + v6 greys → `ZColour::Standard(n)`. The host
  resolver maps 2–9 onto the scheme palette and 10–12 onto fixed grey RGB
  (tables above).
- Any other value (incl. v6 `-1` "pixel under cursor") → treat as `0` (no
  change) in v1.
- A v6 optional 3rd "window" operand is ignored.

### `set_true_colour(foreground, background)` — EXT:0x05 (v5+/v6)

Same channel model, different sentinels (signed 16-bit operands):

- `-1` = **default** (→ `ZColour::Default`).
- `-2` = **leave unchanged**.
- `0..=0x7FFF` = 15-bit RGB `0bbbbbgggggrrrrr` (5/5/5) → `ZColour::True(v)`.
- `-3` (v6 transparent bg) and other negatives → treat as "leave unchanged".

### Interaction with text styles

`reverse video` (style bit `0x01`) **swaps fg and bg at render time.** Cells and
the output stream store colour in *logical* (un-swapped) order; each renderer
swaps fg/bg when the reverse bit is set. This keeps colour + reverse correct
together and matches the existing reverse handling (the new colour just feeds
the swap). Bold/italic/fixed are unaffected by colour.

## Data model

### zvm

```rust
/// A Z-machine colour channel value (logical, pre-reverse-swap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZColour {
    /// Interpreter default (colour 1 / true-colour -1): host maps to
    /// terminal default / scheme fg or bg.
    Default,
    /// Standard palette index 2..=9 (black..white) + v6 greys 10..=12.
    /// 2..=9 resolve to the scheme palette; 10..=12 to fixed grey RGB.
    Standard(u8),
    /// 15-bit RGB (0bbbbbgggggrrrrr) from set_true_colour.
    True(u16),
}

impl Default for ZColour {
    fn default() -> Self { ZColour::Default }
}
```

- `ScreenState` (`zvm/src/screen.rs:93`) gains:
  ```rust
  pub current_fg: ZColour, // default ZColour::Default
  pub current_bg: ZColour, // default ZColour::Default
  ```
  These are transient display state — **NOT** serialised into Quetzal saves
  (same treatment as `current_font`).
- `Cell` (`zvm/src/screen.rs:39`) gains `pub fg: ZColour, pub bg: ZColour`
  (default `ZColour::Default`). `UpperWindow::put` takes fg/bg alongside style.
- `set_colour` / `set_true_colour` update `current_fg`/`current_bg` honoring the
  sentinels above (the no-op arms at `exec.rs:420` / `exec.rs:1216` are
  replaced).
- When the engine writes an upper-window cell (the `set_cursor`/print path that
  currently records `style`), it records `current_fg`/`current_bg` too.

### Output sink seam

Introduce a small attribute bundle so the sink learns colour without a wide
positional signature:

```rust
/// Text attributes for one styled run (logical colour, pre-reverse-swap).
#[derive(Debug, Clone, Copy, Default)]
pub struct TextAttrs {
    pub style: u8,      // existing reverse/bold/italic/fixed bitmask
    pub fg: ZColour,
    pub bg: ZColour,
}

pub trait Output: Any {
    fn print(&mut self, s: &str);
    // New primary styled entry point. Default delegates to the old
    // print_styled(style) so existing overrides keep working until updated.
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        self.print_styled(s, attrs.style);
    }
    fn print_styled(&mut self, s: &str, _style: u8) { self.print(s); }
    // ... set_buffer_mode, as_any, as_any_mut unchanged ...
}
```

The engine's lower-window print path calls `print_attr` with the current
attrs. `BufferOutput` needs no change (inherits the default). CLI and app sinks
override `print_attr`.

## Capability gating

`honor_game_colours` (host config, **default `true`** for all clients) gates
**both** advertisement and rendering, consistently:

- Threaded into the VM at construction so `init_header_caps` **sets** Flags 1
  bit 0 (colour available) when on, **clears** it when off (today's behavior).
  Mechanism: a field on the constructed `Machine` / a parameter to whatever
  currently calls `init_header_caps`; the exact wiring is a plan detail.
- Threaded into each renderer so colour is drawn only when on. When off, the
  engine still tracks colour cheaply but the header bit is clear (so games use
  reverse video) and renderers ignore any colour that was set — today's
  theme-owns-everything look is preserved exactly.

The flag is one value, read in both places, so a game is never told colour is
available and then have its colour dropped.

## Rendering

### CLI (zvm-cli)

`style_wrap` / the `print_attr` override builds an SGR sequence from
`TextAttrs`, on a TTY only (piped output stays plain, as today):

- style bits → existing SGR (1 bold, 3 italic, 7 reverse, …).
- `fg`: `Default` → `39`; `Standard(2..=9)` → `30 + (n-2)`; `Standard(10..=12)`
  → `38;2;r;g;b` from the grey RGB table; `True(v)` → `38;2;r;g;b` (expand 5-bit
  channels to 8-bit).
- `bg`: `Default` → `49`; `Standard(2..=9)` → `40 + (n-2)`; `Standard(10..=12)`
  → `48;2;r;g;b`; `True(v)` → `48;2;r;g;b`.
- Reverse: prefer emitting SGR `7` and letting the terminal swap, so fg/bg SGR
  stay in logical order. (Do **not** also pre-swap, or it double-swaps.)
- Reset with `0` at run end, as today.

### App

A `ZColour` → ratatui `Color` resolver, given the active `ColorScheme`:

- `Default` → `Color::Reset`.
- `Standard(2..=9)` → `scheme.palette[(n-2) as usize]`.
- `Standard(10..=12)` → `Color::Rgb` from the grey RGB table.
- `True(v)` → `Color::Rgb(r, g, b)`.

Apply in two places, gated by `honor_game_colours`:

- **Upper window** (`app/src/render/upper_window.rs`): resolve each cell's
  fg/bg, then apply the existing `apply_text_style(base, bits)` for bold/italic;
  for the reverse bit, swap the resolved fg/bg (instead of, or in addition to,
  the existing REVERSED modifier handling — pick one mechanism so it swaps once).
- **Transcript / lower window** (`app/src/render/transcript.rs` + the styled-run
  model in `state.rs`): the styled run already carries `style: u8`; extend it to
  carry `fg`/`bg` (`ZColour`) and resolve at render time.

When `honor_game_colours` is off, both paths skip colour resolution and behave
exactly as today.

## Config

- `honor_game_colours: bool` added to the app `Config` (default `true`).
- CLI flag override (e.g. `--no-game-colours` / `--game-colours`) for zvm-cli,
  matching the existing CLI option style; default on.
- App: surfaced in the F2 settings modal as a scalar toggle (follows the
  existing settings pattern; a plan detail).

## Testing (TDD)

zvm unit tests:

- `set_colour(0, x)` leaves fg unchanged, sets bg; `set_colour(x, 0)` symmetric.
- `set_colour(1, 1)` → both `ZColour::Default`.
- `set_colour(3, 6)` → `Standard(3)` / `Standard(6)`.
- `set_colour(10, 12)` → `Standard(10)` / `Standard(12)` (v6 greys accepted).
- `set_true_colour(-2, -2)` leaves both unchanged; `-1` → `Default`; a 15-bit
  value → `True(v)` with correct channel expansion.
- Upper-window cell records current fg/bg.
- `init_header_caps`: colour bit **set** when honor flag on, **clear** when off.

CLI tests:

- `TextAttrs` → SGR mapping for default / standard / true, fg and bg.
- Reverse emits SGR 7 once (no double-swap).
- Piped (non-TTY) output stays plain.

App tests:

- `ZColour` → `Color` resolver maps `Standard(2..=9)` onto `scheme.palette`,
  `Standard(10..=12)` onto fixed grey `Rgb`, `Default` → `Reset`, `True` → `Rgb`.
- Reverse swaps resolved fg/bg exactly once.
- `honor_game_colours = false` → renderers ignore colour (theme unchanged).

## Out of scope (v1)

- Glulx/Glk colour (style hints) — separate sub-project.
- `set_colour -1` (pixel under cursor), transparent backgrounds (v6 bg `-3`).
- Per-game persistence of game-set colours (transient, like font).
- gvm-cli colour (lands with the Glulx sub-project).
```
