# Glulx / Glk Stylehint Colour — Design

**Date:** 2026-07-01
**Status:** Implemented (branch `feat/glulx-colour`)

## Problem

The Z-machine path renders game-driven colour (`set_colour`/`set_true_colour`)
in both hosts, gated by `honor_game_colours`. The **Glulx** path had none:
`glk_stylehint_set` was a no-op, `GlkStyle` carried no colour, `gvm-cli`'s
`sgr_set` emitted only attribute SGR, and the app's `AppGlk` hardcoded
`ZColour::Default` for every chunk. Glulx games that colour their text or
status window (via Glk style hints) rendered monochrome.

## Goal

Roll the Z-machine colour learnings into Glulx: honour a game's Glk stylehint
colours in both hosts, under the same `honor_game_colours` gate — at **full
24-bit fidelity** (Glk colour is 24-bit; do not downsample).

## Model: Glk stylehints vs Z-machine `set_colour`

The Z-machine sets a *current* fg/bg directly. Glk instead sets **hints per
(window-type, style-class)**: `glk_stylehint_set(wintype, style, hint, value)`.
The colour hints are `stylehint_TextColor`(7) and `stylehint_BackColor`(8) —
24-bit `0xRRGGBB` — plus `stylehint_ReverseColor`(9). Text is later printed in
some style; the interpreter resolves that (window-type, style) → colour.

## Decisions

- **Resolve in `gvm`, carry across the backend seam.** The Glk `Model` stores a
  hint table `[wintype-row][style] -> StyleColour { fg, bg, reverse }` (row 0 =
  buffer, row 1 = grid; `wintype_AllTypes` writes both). At the output funnel it
  resolves the current (window-type, style) to a `StyleColour` and passes it via
  **new** `GlkBackend::put_text_attr`/`grid_put_attr` methods that DEFAULT to the
  colourless `put_text`/`grid_put`. This mirrors the Z-machine
  `Output::print_attr` seam — backends opt in, none break.
- **Full 24-bit fidelity via `ZColour::True24(u32)`.** The app's neutral colour
  currency is `zvm::screen::ZColour`, whose `True(u16)` is the Z-machine's 15-bit
  colour. Rather than downsample Glk's 24-bit RGB into 15 bits, add a `True24(u32)`
  variant carrying exact `0xRRGGBB`. Packed as tag `3<<24 | rgb` (the low 24 bits
  are free), resolved to `Color::Rgb`, and rendered as truecolor SGR.
- **`honor_game_colours` gate, both hosts.** `gvm-cli`'s `TerminalBackend` and the
  app's `AppGlk` each hold a `honor` flag (default on), threaded from
  `cfg.honor_game_colours` / a `--no-game-colours` CLI flag. When off, colour and
  the reverse hint are dropped; only style-class attributes remain.
- **Reverse hint:** `gvm-cli` emits SGR 7; the app ORs the reverse style bit
  (0x01). Consistent with how each host already renders reverse-video.

## Data flow

```
game: glk_stylehint_set(wintype, style, TextColor/BackColor/ReverseColor, val)
        → Model.style_hints[row][style]
game prints in `style` → exec resolves Model.style_colour(wintype, style)
        → StyleColour { fg, bg, reverse }  (24-bit RGB)
             │
   ┌─────────┴───────────────────────────────┐
 gvm-cli TerminalBackend                     app AppGlk
   put_text_attr/grid_put_attr                put_text_attr/grid_put_attr
   → 24-bit truecolor SGR (38;2 / 48;2,       → ZColour::True24 in buffer log
     SGR 7 for reverse), honor-gated            + grid cells → transcript runs /
                                                 grid nodes → Color::Rgb, honor-gated
```

## Components

1. **`gvm` (`glk.rs`, `exec.rs`)** — `StyleColour` type; `Model.style_hints` +
   `set_style_hint`/`clear_style_hint`/`style_colour`; wire `glk_stylehint_set`
   /`clear`; resolve + call the attr seam at the output funnel.
2. **`gvm-cli` (`glk_term.rs`, `main.rs`)** — grid cells + pending-word carry
   colour; `sgr_open` emits truecolor; `honor` flag + `--no-game-colours`.
3. **app (`glk_backend.rs`, `glulx_session.rs`, `main.rs`)** — `AppGlk` records
   colour in buffer log / grid cells and surfaces it in `take_transcript`
   /`grid_node`; `ZColour::True24` added to `zvm` and plumbed through
   `pack/unpack_zcolour`, `resolve_zcolour`, and the `zvm-cli` SGR path; `honor`
   threaded from config.

## Constraints

- `gvm`/`zvm` stay zero-dependency (`StyleColour`/`True24` are plain data).
- Z-machine colour path unchanged (`True24` is purely additive).
- Cross-platform; full workspace suite green, no new warnings.

## Known limitation

The app resolves colour at **record** time (into the log/cells), not render
time, so toggling `honor_game_colours` via F2 mid-game does not retroactively
recolour already-printed text — it applies to subsequent output. (The Z-machine
resolves at render time.) Acceptable for now; a render-time resolve is a
follow-up if live toggling of past output is wanted.
