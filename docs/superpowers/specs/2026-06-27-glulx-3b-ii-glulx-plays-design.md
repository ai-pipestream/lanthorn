# Glulx 3b-ii: Glulx Plays in the TUI — Design

**Date:** 2026-06-27
**Status:** Approved (design settled in the 3b discussion), ready for planning
**Crate:** `crates/app` (+ `gvm`/`blorb` consumed)
**Depends on:** 3b-i (the `Engine` trait + neutral `ScreenModel`/`KeyInput`/
`Introspect`/`EngineSave`) merged.

## Goal

Make babelmap **play Glulx (`.ulx`/`.gblorb`) games** in the TUI: a
`GlulxSession` implementing `Engine`, an app-side `GlkBackend` that maps Glk's
window tree onto the neutral `ScreenModel`, a **generic multi-window renderer**
for the story pane (with the **map alongside**), Glk input via the existing
turn cycle, and `.gblorb` routing. Automapping for Glulx is **SP4** — Glulx
games **play** here; mapping comes later (`current_location` returns `None`,
`introspect` returns `None`, so the map/play-aids simply stay quiet for Glulx).

## Design

### 1. `.gblorb` / `.ulx` routing

`extract_story` (hints.rs) currently errors on a `GLUL` executable. Change that
branch to surface the Glulx bytes (kind + bytes) so session creation can route:
Z-code → `GameSession`; Glulx → `GlulxSession`. Raw `.ulx` (not Blorb) → Glulx.
The picker's accepted extensions add `.ulx`/`.gblorb`.

### 2. `GlulxSession` (implements `Engine`)

Wraps a `gvm::Machine` configured with an **app `GlkBackend`**, plus pending
input state. The Z-machine's synchronous `submit` model fits gvm directly: a turn
**drives the gvm step loop** (like `gvm-cli`'s `drive()`) until the next
`glk_select` input request (or Quit), accumulating output.

- `submit(line)`: deliver `line` to the pending Glk **line** request, then step
  gvm until the next `NeedLine`/`NeedChar`/Quit; return a `TurnResult` whose
  `transcript` + `transcript_runs` come from the backend's text-buffer output
  (Glk styles → style-bit runs), `quit` set on exit, `beep`/`location_method`
  `None`.
- `submit_key(KeyInput)`: convert to a **Glk keycode** (`key_to_glk`), deliver to
  the pending **char** request, step. Returns `None` for keys with no Glk meaning
  (mirrors the zvm `Option<TurnResult>` behavior).
- `pending_input()`: `Line`/`Char` from the current Glk request.
- `screen()`: the neutral `ScreenModel` the backend maintains (below).
- `save_state()`/`restore_state()`: `gvm` `save_state`/`restore_state` bytes in an
  `EngineSave { engine: "glulx", … }`. The 3b-i foreign-engine restore guard
  already prevents loading a Glulx save into a Z-machine session and vice versa.
- `introspect()` / `current_location()`: `None`/`None` (SP4).
- `resume_save`/`resume_restore`: host-mediated save/restore (the app writes the
  bytes); Glk file-stream `@save`/`@restore` wiring is a separate `gvm` follow-up.

### 3. App `GlkBackend` → neutral `ScreenModel`

An app implementation of `gvm`'s `GlkBackend` that translates Glk display calls
into the neutral `ScreenModel` window tree (3b-i types):
- Glk **pair** windows → `WinNode::Pair { vertical, split, … }`.
- Glk **text-grid** windows → `GridWindow` (cells + cursor + logical size).
- Glk **text-buffer** windows → `BufferWindow` (accumulated lines + per-span
  style runs from Glk styles + scroll).
- Glk **graphics/blank** windows → `Blank` placeholder (text TUI; out of scope).
- Window open/close/arrange rebuild the tree; the backend is told the **story-pane
  size** (see §4) so Glk computes child rects within it.
Glk styles (Emphasized/Header/…) map to the same text-style bits the transcript
runs already use, so emphasis renders for free.

### 4. Generic multi-window renderer (the new rendering)

Render an arbitrary `ScreenModel` tree into the **story pane** (the screen region
not given to the map — today's Split/MapFull/TranscriptFull layout is unchanged;
the map keeps its pane):
- `Pair` → split the rect (`vertical`/`split` ratio or fixed rows) and recurse.
- `Grid` leaf → positioned style cells, with the **viewport** over the logical
  grid (reuse the v4+ auto-follow logic).
- `Buffer` leaf → wrapped, scrolled, styled lines — reuse the transcript renderer
  (`wrap_lines_kinded` + `draw_str_runs`), one scrollback per buffer window.
- The **Z-machine path is unchanged**: its 2-node tree renders through this same
  code (already validated in 3b-i), so there is one renderer for both engines.
- Re-derived each frame from `engine.screen()`, so dynamic open/close/resize just
  works. babelmap tells the engine the story-pane size each frame (the map is
  bolted on the side, untouched).

### 5. Input

The app already routes keystrokes to the engine (`submit`/`submit_key`) and has
char-mode (keystrokes → game when awaiting char input). For Glulx: `submit_key`
runs `key_to_glk` (a small `KeyInput` → Glk keycode table: Return/Delete/Tab/
arrows/Func1-12/Page/Home/End/Esc + chars). Line input uses the command line as
today. Glk line *terminators*/pre-fill are a small optional extra (defer if a
target game doesn't need them).

### 6. Save / load

`.babelmap` archives already carry the engine tag (3b-i). A Glulx game's save is
the `gvm` snapshot under the `"glulx"` tag; the restore guard refuses a
mismatched engine. (The Z-machine `ScreenState` archive entry is Z-specific and
stays so; Glulx archives don't write it.)

## Testing

- `extract_story`/routing: a `.gblorb` (GLUL) yields Glulx bytes → a
  `GlulxSession`; a `.zblorb` still yields a `GameSession`.
- `GlulxSession`: a hand-assembled Glulx program that opens a grid + buffer,
  prints, requests a line, and on input echoes it → `submit("hi")` returns the
  expected transcript + runs; `screen()` is the right 2-window tree; `submit_key`
  maps arrows to Glk keycodes; `save_state`/`restore_state` round-trips under the
  `"glulx"` tag; the foreign-engine guard fires across engines.
- App `GlkBackend`: Glk window open/arrange builds the right `ScreenModel`;
  styles → runs; grid cells + cursor.
- Renderer: a 3-window tree (grid + two buffers) renders into nested rects;
  the Z-machine 2-node tree still renders byte-identical (3b-i tests stay green).
- An end-to-end smoke: load a small real `.ulx`/`.gblorb`, submit a couple of
  commands, assert sane transcript output (no panic, output present).

## Out of scope (3b-ii)

- **Automapping for Glulx** (Inform 7 location/object detection) → **SP4**.
- Glulx **play-aids** (autocomplete/verb-menu/inventory — need Inform 7
  introspection) → SP4; `introspect()` is `None` now, aids degrade gracefully.
- **Graphics/sound** windows → out (text TUI).
- Glk **file-stream `@save`/`@restore`** + cross-interpreter Quetzal → separate
  `gvm` follow-ups.

## Global constraints

- The Z-machine path stays byte-identical (the 3b-i tests + render equivalence
  remain green); the new renderer must reproduce the 2-node case exactly.
- 0 warnings + full `cargo test --workspace` green per task. `app` may now
  depend on `gvm` + `blorb` (add to `crates/app/Cargo.toml`).
- The trait surface stays engine-neutral; Glk specifics live only inside the
  `GlulxSession`/app-`GlkBackend`.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
