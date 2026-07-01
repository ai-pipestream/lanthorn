# Game Colour & Upper-Window Rendering — Design

**Date:** 2026-06-30
**Status:** Draft (for review)

## Problem

Now that a colour-capable interpreter number is the default (BeyondZork and
peers emit real `set_colour`), four rendering bugs surfaced. Root causes were
confirmed by capturing the engine/host boundary (PTY capture of zvm-cli output
+ engine cell-colour dump, plus a read of both hosts' render paths):

1. **Grey background behind/around text.** The game sets a black background
   (`set_colour` → `current_bg = Standard(2)`), but neither host paints it
   across the screen. Erase paths (`ESC[2J`, per-row `ESC[2K`) and scrolled
   blank areas fall back to the terminal/theme default. Only the exact glyph
   cells that carry a non-Default bg get colour.
2. **Menu selection not rendered.** BeyondZork's Character Setup menu is drawn
   in the upper window and highlights the selected row **by colour** (captured
   cells: `Standard(3)` red / `Standard(9)` white / `Standard(8)` cyan on
   `Standard(2)` black — no reverse-video style bit). Both hosts lose that
   colour (see #4), so every row looks identical.
3. **Cursor stuck behind the upper window after character selection.** After
   the menu, the pinned upper region settles at ~12 rows and the input cursor
   is left inside/below it. **Out of scope for this spec** — tracked separately
   as a region/cursor-geometry bug.
4. **Score box not red.** The status/score box lives in the upper window and is
   drawn red (`Standard(3)`). zvm-cli's `upper_row_ansi` drops it; the app maps
   it to `Color::Reset`.

### Confirmed root causes

**zvm-cli** (`crates/zvm-cli/src/screen.rs`):
- `upper_row_ansi` (:121-147) reads **only `cell.style`** via `sgr_set`; it never
  reads `cell.fg`/`cell.bg`. Upper-window colour is discarded (bugs 2, 4).
- `erase` (:316-322), `start` (:303-309), and the per-row `ESC[2K` in `render`
  (:266) clear with **no preceding background SGR**, so clears use the terminal
  ambient colour (bug 1). Lower-window runs always close with `ESC[0m`, so
  padding/blank areas are never painted.

**app** (`crates/app/src/`):
- `resolve_zcolour` (`render/mod.rs:58-71`) maps `Standard(2..=9)` →
  `scheme.palette[n-2]`, but `ColorScheme::terminal_default().palette` is
  **all `Color::Reset`** (`colors.rs:411`). So without a Ghostty theme loaded,
  every standard game colour (including black bg and the menu highlight)
  resolves to "terminal decides" and is invisible (bugs 2, 4, and the "not
  black" part of 1). The upper-window path (`render/upper_window.rs:20-40`)
  *does* read `cell.fg`/`cell.bg` correctly — the palette is the failure.
- The app never reads `machine.screen.current_bg` (zero hits repo-wide) and
  never fills the transcript area / line padding with a background — only glyph
  cells with a non-Default `StyleRun.bg` get a bg (bug 1).

**engine** (`crates/zvm`): the header default-colour bytes 0x2C (default bg) and
0x2D (default fg) are never written or read; `current_fg`/`current_bg` seed to
`ZColour::Default`. There is no interpreter-provided default colour.

## Goal

BeyondZork (and colour v5+ games generally) render colour faithfully in both
hosts: the menu selection is visibly highlighted, the score box is red, and the
game's chosen background is painted across the screen — matching Frotz.

## Decisions (from review)

- **Background model: header defaults + paint, theme-safe seeding.** The engine
  provides interpreter default colours via the header and hosts paint the
  current background across cleared/blank regions — BUT the interpreter default
  is seeded **host-neutral** (`ZColour::Default`), NOT a forced Frotz black. A
  game's background is honored only when the game sets it explicitly (BeyondZork
  does), so themes are preserved for games that don't. See Component 1.
- **App: paint only the story-output pane, keep the map themed.** The
  `current_bg` fill is scoped to the story/transcript pane (and its upper-window
  content area). The automap pane and app chrome keep the theme background. See
  Component 5.
- **Scope: bugs 1, 2, 4 now.** Bug 3 (cursor/region) is investigated and fixed
  separately afterward.

## Design

### Component 1 — Engine (`zvm`): no change required

Under the theme-safe decision, `current_fg`/`current_bg` already default to
`ZColour::Default` (host-decides), and colour-honoring games (BeyondZork) set
their background explicitly via `set_colour`, which the hosts already receive
through `current_bg`. So **the engine needs no change** for bugs 1/2/4 — all
fixes live in the two hosts. Writing the header default-colour bytes (0x2C/0x2D)
would be spec polish with no visible effect here and is deliberately left out
(YAGNI). `zvm`/`gvm` untouched.

### Component 2 — zvm-cli upper-window colour (bugs 2, 4)

- `upper_row_ansi` emits full per-cell attributes: build each run from
  `TextAttrs { style: cell.style, fg: cell.fg, bg: cell.bg }` via the existing
  `sgr_open`, gated on `honor_game_colours` (mirror `StdoutOutput::print_attr`'s
  substitution to `Default` when colours are off). Runs break when any of
  style/fg/bg changes, and close with `ESC[0m`.
- Because the score box / menu now carry a real bg, pad each rendered row to the
  full upper-window width with the row's background so the coloured bar spans
  the line (not just the text).

### Component 3 — zvm-cli background paint (bug 1)

- Thread the engine's `current_bg` (and `current_fg`) to the host. When a
  concrete (non-Default) background is active and colours are honored:
  - `erase` emits the background SGR before `ESC[2J` so the full-screen clear
    fills with it.
  - the per-row `ESC[2K` in `render` is preceded by the background SGR.
  - lower-window lines paint to end-of-line with the background (emit the bg
    SGR + `ESC[K` so trailing width is filled), so scrolled/padded areas match.

### Component 4 — app palette (bugs 2, 4, and "not black")

- `ColorScheme::terminal_default().palette` maps to the **16 concrete ANSI named
  colours** (`Color::Black, Color::Red, …, Color::White` and bright variants)
  instead of `Color::Reset`. Then `Standard(2)`→black, `Standard(3)`→red, … the
  menu highlight and score box render, and black is black. A loaded Ghostty
  theme still overrides via its parsed palette (unchanged).

### Component 5 — app background paint, story pane only (bug 1)

- The app reads `machine.screen.current_bg`; when non-Default and colours are
  honored, **only the story-output pane** — the transcript area and its
  upper-window content area (`render/screen.rs` story-pane path +
  `render/transcript.rs`) — fills its whole rect with that background before
  drawing glyphs, so padding and blank rows are painted.
- The **automap pane and app chrome are NOT touched** — they keep the theme
  background. The fill is scoped to the story pane's `Rect`, so the map stays
  themed regardless of the game's background colour.

## Data Flow

```
engine: header 0x2C/0x2D → current_bg/current_fg (Frotz default: black/white)
        set_colour → current_bg/current_fg (game overrides)
             │
   ┌─────────┴─────────────────────────────┐
zvm-cli                                    app
  upper_row_ansi emits cell.fg/bg           upper_window cell_style reads fg/bg
  erase/EL/line paint current_bg            palette → concrete ANSI (black=black)
                                            transcript/area fill = current_bg
        (both gated on honor_game_colours; F2 toggles it off → theme/terminal)
```

## Testing

- **engine:** unchanged (no engine task).
- **zvm-cli:** `upper_row_ansi` on a row with `Cell{fg:Standard(3),bg:Standard(2)}`
  emits `31`+`40`; honor off emits neither; a row is padded to width with bg.
  `erase` with `current_bg=Standard(2)` includes `40` before `2J`.
- **app:** `resolve_zcolour(Standard(2), terminal_default)` → `Color::Black` (not
  Reset); `resolve_zcolour(Standard(3), …)` → `Color::Red`. Transcript fill uses
  `current_bg` when non-Default.
- **Manual (both hosts):** BeyondZork — menu selection visibly highlighted, score
  box red, screen background black; `honor_game_colours` off restores theme.

## Constraints

- `zvm`/`gvm` stay zero-dependency; `gvm` untouched.
- Cross-platform (Windows/Linux/macOS).
- 0 warnings + full workspace suite green per task.
- `honor_game_colours` (F2, default on) remains the single gate that turns all
  of this off and restores theme/terminal colours.

## Files Touched (host-only)

- `crates/zvm-cli/src/screen.rs` — `upper_row_ansi` fg/bg + width pad;
  `erase`/`render`/line background paint; tests.
- `crates/zvm-cli/src/main.rs` — thread `current_bg` into the paint paths.
- `crates/app/src/colors.rs` — `terminal_default().palette` → concrete ANSI.
- `crates/app/src/render/screen.rs` (+ `transcript.rs`) — story-pane-scoped
  background fill from `current_bg` (map/chrome untouched); tests.
