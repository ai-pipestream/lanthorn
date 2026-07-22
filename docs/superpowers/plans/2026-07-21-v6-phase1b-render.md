# v6 Phase 1b — Render + Text Routing (z-ordered layered) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Put Zork0 on screen. Route v6 text into its window grids, composite pictures per window, and render the windows as a z-ordered layered stack (cell-text-wins) — graphics background, text on top.

**Architecture:** Builds on Phase 1a (`ScreenState.v6` window table + `pending_pictures` + injected `picture_dims`, all committed on this branch). Three pieces: (1) zvm `print_text` routes to the current v6 window's grid; (2) app drains `pending_pictures` into per-v6-window `Canvas`es via the existing `PictSource`; (3) a new `WinNode::Layered` variant + `render_node` arm draws windows in z-order at absolute cell rects. Input needs no change (engine-generic).

**Tech Stack:** Rust. `zvm` stays zero-dep (Task 1 only touches text routing, no new deps). App reuses `graphics.rs`/`render/graphics.rs`.

**Spec:** `docs/superpowers/specs/2026-07-21-v6-phase1-windows-design.md` §6–8 (render sections corrected 2026-07-21 to layered).

## Global Constraints

- **`zvm` stays zero-dependency** (Task 1).
- **Additive to v3–8.** v1–5/v7/v8 text routing + rendering byte-identical; all v6 logic gated on `screen.v6` / a v6 branch. The existing Z-machine "simple path" (`is_simple`) must still fire for v1–5.
- **cell-text-wins:** text grids paint only non-blank cells (transparent elsewhere) so the graphics layer shows through gaps.
- Font cell size is `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` (8×8, Phase 1a) — the pixel→cell divisor; tune in Task 5 if Zork0's proportions look off.

## Design reference (locked here)

**Pixel→cell:** a v6 window's absolute cell rect = `(x_coord/FW, y_coord/FH, x_size/FW, y_size/FH)`. The grid was already cell-sized at `window_size` time (`grid.rows == y_size/FH`, `grid.cols == x_size/FW`), so cols/rows come straight from `grid`; only the *position* needs dividing.

**New render types** (`crates/app/src/engine.rs`):
```rust
/// One window placed at an absolute cell rect within the story pane, for the
/// v6 z-ordered layered composite. Drawn in list order (background first).
pub struct PositionedWindow {
    pub x: u16, pub y: u16, pub w: u16, pub h: u16, // absolute cell rect in the pane
    pub node: WinNode,                               // Grid / Buffer / Graphics leaf
}
// new WinNode variant:
//   Layered(Vec<PositionedWindow>)
```

**z-order:** graphics windows first, then text windows by window number (0 body, 1 strip, …). cell-text-wins ⇒ text painted after graphics; blank grid cells are transparent.

---

### Task 1: zvm — route v6 text to the current window's grid

**Files:** Modify `crates/zvm/src/cpu/exec.rs` `print_text` (~:2209-2270). Test: `exec.rs`.

**Interfaces:** Produces: for a v6 story, printed text lands in `v6.windows[v6.current].grid` (windows 1–7) or streams to the transcript (window 0). v1–5 unchanged.

- [ ] **Step 1: Write the failing test** — build a v6 machine, `set_window(1)`, `set_cursor` to (1,1), `print_text("HI")`, assert `v6.windows[1].grid.cell(1,1).ch == 'H'`. Also assert window-0 text still streams (a `set_window(0)` + print reaches the Output sink, not a grid).

- [ ] **Step 2: Run — expect FAIL** (text currently streams regardless of v6 window).

- [ ] **Step 3: Implement** — at the top of `print_text`, before the legacy `current_window == 1` check, add:
```rust
if let Some(v6) = self.screen.v6.as_ref() {
    let cur = v6.current;
    if cur >= 1 {
        // grid window: write chars into v6.windows[cur].grid at its cursor,
        // reusing the per-cell loop currently hardcoded to screen.upper
        // (grow the grid if the game draws past its current rows).
        // ...write to v6.windows[cur].grid, advance v6.windows[cur].x_cursor/y_cursor...
        return;
    }
    // cur == 0: fall through to the buffered stream path below (window 0 = main).
}
```
Generalize the existing `screen.upper` write loop (lines ~2231-2250) to operate on `v6.windows[cur].grid` and the per-window cursor. Keep font-3 handling. v1–5 path (`self.screen.v6` is None) is the untouched `else`.

- [ ] **Step 4: Run — expect PASS**, full `cargo test -p zvm --lib` green (v1–5 regression).

- [ ] **Step 5: Commit** — stage `crates/zvm/src/cpu/exec.rs`. Message: `feat(zvm): route v6 text to the current window's grid` + trailers (Quest: SQ-0186 / Co-Authored-By / Claude-Session).

---

### Task 2: app — drain pending_pictures into per-window canvases

**Files:** Modify `crates/app/src/session.rs` (`GameSession` canvas store, `TurnResult.pictures`, `drain_turn`), `crates/app/src/turn.rs` (consume), possibly `state.rs`. Test: app.

**Interfaces:** Consumes Phase 1a `pending_pictures` + `zcode_pict_source`. Produces: `GameSession` holds `HashMap<u8, Canvas>` keyed by v6 window, updated each turn from draw events.

- [ ] **Step 1: Write the failing test** — an app test: construct a v6 session with a `zcode_pict_source` (from Zork0.blb or a synthetic 1-Pict blorb), drive a turn that issues `draw_picture(n, win, y, x)`, assert the session's canvas store has an entry for `win` whose canvas is non-blank.

- [ ] **Step 2: Run — expect FAIL** (no canvas store; pending_pictures dead in app).

- [ ] **Step 3: Implement**
  - Add `pictures_canvas: std::collections::HashMap<u8, crate::graphics::Canvas>` to `GameSession` (init empty).
  - In `drain_turn`, `let pictures = std::mem::take(&mut self.machine.pending_pictures);` and carry it in `TurnResult` (new `pub pictures: Vec<PictureEvent>` field — import the zvm type).
  - Apply the events: for each `PictureEvent{number, window, x, y, erase}`, resolve `self.pict_source.as_mut()?.image(number)` (or the shared `zcode_pict_source`; wire it onto `GameSession` if not already — Phase 1a left it on `AppState`, so either move a clone/handle to the session or apply in `turn.rs` where `state.zcode_pict_source` is reachable). Draw into `pictures_canvas.entry(window).or_insert_with(|| Canvas::new(win_px_w, win_px_h)).draw_image(&img, x, y, scale)`; `erase` → `erase_rect`. Bump canvas version.
  - DECISION for the implementer: the cleanest place to apply may be `turn.rs::apply_turn_events` (has `state.zcode_pict_source` + the session). Keep the canvas store where the adapter (Task 4) can read it — if that's the session, thread the pict source into the session; if `AppState`, have the adapter read from `AppState`. Pick one, document it. (Recommended: canvas store + pict source both on `GameSession` so `screen()` is self-contained.)

- [ ] **Step 4: Run — expect PASS**, `cargo test -p app`.

- [ ] **Step 5: Commit** — stage the touched app files. Message: `feat(app): drain v6 draw_picture events into per-window canvases` + trailers.

---

### Task 3: app — WinNode::Layered variant + render_node arm

**Files:** Modify `crates/app/src/engine.rs` (`PositionedWindow`, `WinNode::Layered`), `crates/app/src/render/screen.rs` (`render_node` arm, `is_simple` unaffected). Test: `render/screen.rs`.

**Interfaces:** Produces: a render path that draws an ordered list of positioned windows at absolute cell rects, cell-text-wins. Consumed by Task 4.

- [ ] **Step 1: Write the failing test** — build a `WinNode::Layered` with a full-area `Graphics` (solid canvas) then a small `Grid` with one non-blank cell on top; render into a `Buffer`; assert the grid's char is at its absolute rect AND a graphics cell shows through at a blank-grid location (cell-text-wins). Use `Picker::halfblocks()` (deterministic, no TTY) like the existing graphics tests.

- [ ] **Step 2: Run — expect FAIL** (no `Layered` variant).

- [ ] **Step 3: Implement**
  - `engine.rs`: add `PositionedWindow` (per the design reference) + `WinNode::Layered(Vec<PositionedWindow>)`. Update any exhaustive `match` on `WinNode` (grep) with the new arm.
  - `render/screen.rs`: `render_node`'s `Layered(items)` arm iterates `items` in order; for each, compute the sub-`Rect` = `area` offset by `(x,y)` clamped to `area`, size `(w,h)`; recurse `render_node` into that sub-rect for the leaf. For `Grid` leaves in the layered path, paint only non-blank cells (add/period a "transparent blanks" mode — either a flag on the grid render or a dedicated layered-grid draw that skips `ch == ' '` with default bg). Graphics + Buffer leaves render as usual (graphics already skip empty canvas regions).
  - `is_simple` (screen.rs:66) is unaffected — a `Layered` root is not simple, and `content_size` will be nonzero anyway.

- [ ] **Step 4: Run — expect PASS**, `cargo test -p app`, clippy clean.

- [ ] **Step 5: Commit** — stage `crates/app/src/engine.rs`, `crates/app/src/render/screen.rs`. Message: `feat(app): WinNode::Layered z-ordered render (cell-text-wins)` + trailers.

---

### Task 4: app — v6 adapter builds the layered model

**Files:** Modify `crates/app/src/session.rs` (`screen_model_from_machine` → v6 branch, or a `GameSession::screen` v6 path with canvas access). Test: `session.rs`.

**Interfaces:** Consumes Tasks 2–3. Produces: for a v6 story, `screen()` returns a `ScreenModel { root: WinNode::Layered(...), content_size: nonzero, .. }` built from `screen.v6` + the canvas store.

- [ ] **Step 1: Write the failing test** — a synthetic v6 machine with windows 0/1/7 sized/positioned + a canvas for window 7; call the adapter; assert the returned `Layered` list has graphics-first order, correct absolute cell rects (pixel÷8), window 0 as `Buffer{primary:true}`, window 1 as `Grid`, window 7 as `Graphics`, and `content_size != (0,0)`.

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement** — a v6 branch (`if machine.screen.v6.is_some()`) that:
  - iterates `v6.windows[0..8]`, skips zero-size windows;
  - for each, cell rect = `(x_coord/FW, y_coord/FH, grid.cols, grid.rows)`;
  - window 0 → `PositionedWindow{ node: Buffer{primary:true, ..} }`; windows 1–7 with grid content → `Grid`; windows with a canvas in the store → `Graphics` (from `canvas.arc()`/`version`);
  - order: graphics entries first, then text by window number;
  - `content_size` = the pane cell extent (max right/bottom of the rects, or the header screen cols/rows).
  - Because the adapter needs the canvas store, make the v6 path a `GameSession` method (or pass the store in); keep the v1–5 `screen_model_from_machine` path exactly as-is for non-v6.

- [ ] **Step 4: Run — expect PASS**, `cargo test -p app`, clippy clean.

- [ ] **Step 5: Commit** — stage `crates/app/src/session.rs`. Message: `feat(app): v6 layered screen-model adapter` + trailers.

---

### Task 5: end-to-end wiring, font tuning, TTY smoke

**Files:** Possibly small wiring in `main.rs`/`loop_tick.rs`/`turn.rs`; `crates/zvm/src/screen.rs` (font const, if tuned). Test: manual TTY + any headless end-to-end assert.

**Interfaces:** The whole path lit up: launch Zork0 → windows render layered → pictures appear → text in the right regions.

- [ ] **Step 1: End-to-end headless assert** (if feasible) — extend `crates/app/tests/zork0_v6_windows.rs`: after driving Zork0, call `session.screen()` and assert the model is `Layered` with ≥1 Graphics and ≥1 text window at plausible rects.

- [ ] **Step 2: Verify the render path is reached** — trace/confirm `main.rs`'s per-frame `engine.screen()` → `render_story_pane` → `render_node` hits the `Layered` arm for a v6 story (no simple-path short-circuit).

- [ ] **Step 3: Font tuning** — if Zork0's proportions look wrong at 8×8 in the TTY smoke, adjust `V6_FONT_WIDTH`/`V6_FONT_HEIGHT` (e.g. 8×16) and re-check; document the chosen value.

- [ ] **Step 4: Commit** any wiring/tuning. Message: `feat(app): wire v6 layered render end-to-end` + trailers.

- [ ] **Step 5: TTY SMOKE (user)** — launch Zork0 in a real terminal: the title screen / bordered art should render (window 7 background), status/text in their regions (windows 0/1), and stepping should update them. Record in to-verify.

---

## Final verification

- [ ] `cargo test -p zvm -p app` green; clippy clean; zvm zero-dep.
- [ ] v1–5 regression: `is_simple` still fires for v3–5; their render byte-identical.
- [ ] Zork0 headless: `screen()` returns a plausible `Layered` model.
- [ ] Zork0 TTY (user): renders recognizably.

## Definition of done

Zork0 renders: layered windows, pictures composited, text routed to the right windows, cell-text-wins. `zvm` zero-dep, v1–5 byte-identical. Phase 1 (1a+1b) complete → merge; Phase 2 (menus/mouse/sound/margins) scoped next. Known limit carried forward: `make_menu` still stubbed, so Zork0's function-key *menu* isn't interactive yet (Phase 2).
