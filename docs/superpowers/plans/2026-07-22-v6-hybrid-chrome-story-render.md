# v6 Hybrid Chrome+Story Render — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the v6 pane as a scaled, pixel-aspect-accurate chrome (frame + status) around a story region, in two selectable modes — `raster` (story rasterized into the one image; feature-limited) and `hybrid` (story rendered by the normal terminal transcript) — plus the existing `cell` fallback.

**Architecture:** Two phases. **Phase A** builds the shared layout brain (window classification, clear-interior story viewport, draw-order chrome compositing incl. the compass) and ships the `raster` mode as a keeper. **Phase B** adds the `hybrid` mode (chrome image ring around a terminal story viewport) and makes it default. Design: `docs/superpowers/specs/2026-07-22-v6-hybrid-chrome-story-render-design.md`.

**Tech Stack:** Rust; `image` (RgbaImage), `ratatui`/`ratatui-image` (Picker/Protocol), the embedded `font8x8` bitmap font, the app's transcript renderer.

## Global Constraints

- Crates `zvm`, `gvm`, `scott` stay ZERO external dependencies. New deps stay in `app`.
- Non-v6 (v1–v5) and Glulx paths must stay byte-identical: the new behaviour is reachable ONLY when `screen.v6.is_some()` (→ `WinNode::Layered`) AND `state.game_picker.is_some()`; with no picker the `cell` fallback (existing Phase 1b path) runs unchanged.
- Stage git files EXPLICITLY by path; never `git add -A`.
- Run the FULL `cargo test` (workspace) before declaring a task done.
- Coordinate model: native pixel space from the Blorb `Reso` size (Zork0 320×200, fallback 320×200) is already advertised before boot (checkpoint `b41cdff3`). The v6 font cell is 8×8 game px (`zvm::screen::V6_FONT_WIDTH`/`_HEIGHT`). Uniform scale `s = min(pane_dev_w/N_w, pane_dev_h/N_h)`, same factor x and y, centered (letterbox).
- Commit trailers on every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh`
  and `Quest: SQ-0186` (final task: `Confirm: SQ-0186`).
- No back-compat shims (pre-release). Every new UI/config knob is themeable/config-documented per repo policy.
- **Render pixel math needs visual tuning.** Unit tests pin the *logic* (classification, viewport-is-inside-frame, scale factor, z-order). Exact sub-pixel placement is confirmed at a real terminal (a `confirm` item), not asserted headless. Do not over-fit tests to one screenshot.

**Baseline:** `b41cdff3` (native-res + Reso + `set_margins` + uniform upscale) is committed. `crates/app/src/render/v6_canvas.rs` holds the current whole-pane compositor (`build_v6_canvas`, `MainText`, `blit_glyph` wrappers, `packed_to_rgba`, `blit_scaled`) — Phase A moves/repurposes these into `v6_layout.rs`.

---

## PHASE A — Shared layout + `raster` mode

### Task A1: Window classification — `v6_layout::classify_windows`

**Files:**
- Create: `crates/app/src/render/v6_layout.rs`
- Modify: `crates/app/src/render/mod.rs` (`pub mod v6_layout;`)

**Interfaces:**
- Produces: `pub struct V6Layout<'a> { pub story: Option<&'a PositionedWindow>, pub chrome: Vec<&'a PositionedWindow> }` and `pub fn classify_windows(items: &[PositionedWindow]) -> V6Layout<'_>`.
- Rule: the story window is the single `WinNode::Buffer { primary: true }` entry; every other live entry (Grid/Graphics) is chrome. (A window may contribute both a Graphics entry and a text entry — both are classified independently; a Graphics entry is always chrome, a primary Buffer is the story.)

- [ ] **Step 1: Write the failing test** (in `v6_layout.rs`): build a `Vec<PositionedWindow>` with one primary `Buffer`, one `Grid`, one `Graphics`; assert `classify_windows` returns the Buffer as `story` and the other two as `chrome` (order preserved). A list with no primary Buffer → `story == None`, all entries chrome.
- [ ] **Step 2: Run it, verify it fails** (`cargo test -p app --lib v6_layout::`).
- [ ] **Step 3: Implement `classify_windows`** per the rule. Add `pub mod v6_layout;` to `mod.rs`.
- [ ] **Step 4: Run tests, then `cargo test -p app`** (whole crate still green).
- [ ] **Step 5: Commit** (`git add crates/app/src/render/v6_layout.rs crates/app/src/render/mod.rs`).

### Task A2: Chrome canvas — `v6_layout::build_chrome_canvas`

**Files:** Modify `crates/app/src/render/v6_layout.rs`. Move `packed_to_rgba`, `blit_scaled`, and the glyph-blit usage from `v6_canvas.rs` into `v6_layout.rs` (or keep shared helpers in `bitfont.rs`); delete the moved copies from `v6_canvas.rs` in Task A5.

**Interfaces:**
- Produces: `pub fn build_chrome_canvas(chrome: &[&PositionedWindow], native: (u16,u16), default_fg: image::Rgba<u8>, colors: &ColorScheme) -> image::RgbaImage`.
- The canvas is `native.0 × native.1`, fully transparent to start. It contains ONLY chrome (frame graphics + status text); the story window's rect is left transparent.

Compositing order (spec §Components/2):
1. Graphics entries, **in list order** (`items`/`chrome` preserves the model's order, which is the draw order the v6 model emits — graphics-first, ascending window; see Task A5 note on ordering). Blit each `Graphics` window's canvas at its native rect via `blit_scaled` honoring source alpha. Within a window the picture canvas is already draw-order-composited (the compass base + indicator stack).
2. Grid (status) text entries: rasterize each non-blank cell's glyph at native `(x_px + col·8, y_px + row·8)` with resolved fg/bg. **Do not clamp to the window's pixel height** — status legitimately exceeds it.

- [ ] **Step 1: Write failing tests:**
  - Frame opacity + transparent interior: a chrome `Graphics` window with an opaque border and transparent center → canvas opaque on the border pixels, transparent (alpha 0) in the center; a region with no chrome window stays transparent.
  - **Compass z-order stress test:** two chrome `Graphics` entries whose canvases overlap at the same native spot — a "base" (solid A) drawn first, an "indicator" (solid B with a transparent margin) drawn second — assert the overlap shows B where B is opaque (last-on-top) and A where B is transparent. Then a single window whose picture canvas already stacks base+indicator (pre-composited) renders identically. This proves later-drawn wins.
  - Status glyph: a chrome `Grid` with a cell `'A'` at (col=2,row=1) in a window at native (10,4) → fg pixels appear near native (10+16, 4+8).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement `build_chrome_canvas`.** Reuse `blit_scaled` (alpha-honoring) for graphics and `bitfont::blit_glyph` for status.
- [ ] **Step 4: `cargo test -p app`.**
- [ ] **Step 5: Commit.**

### Task A3: Story viewport — `v6_layout::story_viewport`

**Files:** Modify `crates/app/src/render/v6_layout.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub struct Scale { pub s: f32, pub off_x: u32, pub off_y: u32 }
  pub fn uniform_scale(native: (u16,u16), pane_dev: (u32,u32)) -> Scale;
  /// The cell rect (relative to the pane's top-left cell) where the story text
  /// goes: the largest cell-aligned rect inside the story window's device rect
  /// that touches no opaque chrome pixel. Falls back to the full pane when there
  /// is no story window.
  pub fn story_viewport(story: Option<&PositionedWindow>, chrome_canvas: &RgbaImage,
                        scale: &Scale, pane_cells: (u16,u16), cell_px: (u16,u16)) -> ratatui::layout::Rect;
  ```

Algorithm:
1. `uniform_scale`: `s = min(pane_dev.0 as f32 / native.0 as f32, pane_dev.1 as f32 / native.1 as f32)`; `off = ((pane_dev - native·s)/2)` per axis.
2. `story_viewport`: map the story native rect → device rect `(off + xy·s, wh·s)`. Inset each edge inward one native-pixel row/col at a time while that edge overlaps an opaque chrome pixel (alpha ≥ 128) anywhere along the story's span. Convert to pane-cell coordinates: top-left `ceil` to the next cell, bottom-right `floor` to the previous cell (snap inward). Guarantee ≥ 1×1 cells; if the story window is `None`, return the full pane rect.

- [ ] **Step 1: Write failing tests:**
  - `uniform_scale`: native 320×200 into 640×480 → s=2.0 (min(2.0, 2.4)), off_x=0, off_y=(480-400)/2=40.
  - `story_viewport` with a synthetic chrome canvas: an opaque border ring (top band + left/right columns) and a transparent interior, a story window spanning the whole native area → the returned cell rect is strictly inside the ring (its device projection contains no opaque pixel), snapped to cells, and its top is below the top band.
  - No story window → full pane rect.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement.** Keep the opaque-scan simple (per-edge shrink); it runs once per frame on a ~320×200 canvas.
- [ ] **Step 4: `cargo test -p app`.**
- [ ] **Step 5: Commit.**

### Task A4: `v6_render` mode config

**Files:**
- Modify: the app config module (find via `grep -n "virtual_screen_cols\|pub struct Config\|honor_game_colours" crates/app/src/*.rs`) — add `v6_render: V6RenderMode` with `#[derive]` default `Hybrid`, parsed from config.toml, and a settings-screen entry if the repo exposes such toggles (mirror an existing enum config like a layout/mode field).
- Add enum `pub enum V6RenderMode { Hybrid, Raster }` (serde/FromStr per the config's convention).

**Interfaces:**
- Consumes: nothing new.
- Produces: `state.config.v6_render`.

- [ ] **Step 1: Write failing test:** config round-trips `v6_render = "raster"` → `V6RenderMode::Raster`; default (absent) → `Hybrid`; unknown value → default (or error, matching the config's convention for other enums).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** following the exact pattern of an existing enum config field (find one; copy its parse/default/serialize + docs). Document the key in the config docs the repo keeps.
- [ ] **Step 4: `cargo test -p app`.**
- [ ] **Step 5: Commit.**

### Task A5: `raster` render — wire the mode + clear-interior story text

**Files:**
- Modify: `crates/app/src/render/screen.rs` (the `WinNode::Layered` arm)
- Modify: `crates/app/src/render/graphics.rs` (`draw_v6_canvas` stays: draws one scaled canvas to the pane — already uniform-upscale)
- Modify/retire: `crates/app/src/render/v6_canvas.rs` — replace `build_v6_canvas` with a thin `raster` composite that uses `v6_layout` (chrome canvas + story bitmap text in the viewport). Keep `MainText`/`build_main_text` (still needed to source the story text) or move them to `v6_layout`.

**Interfaces:**
- The `Layered` arm, when `state.game_picker.is_some()`:
  1. `layout = classify_windows(items)`; `native = native_size(items)` (max window extent, or the header 0x22/0x24).
  2. `chrome = build_chrome_canvas(layout.chrome, native, default_fg, colors)`.
  3. `scale = uniform_scale(native, pane_dev)`; `viewport = story_viewport(layout.story, &chrome, &scale, pane_cells, cell_px)`.
  4. Match `state.config.v6_render`:
     - `Raster` (and, until Phase B, `Hybrid`): draw the story text (bitmap, from `build_main_text` wrapped to the viewport's cell width) INTO the chrome canvas at the viewport's native region, then `draw_v6_canvas` the whole canvas scaled to the pane. Return `None` metrics (raster is feature-limited).
  5. No picker → existing Phase 1b cell composite.

The KEY change vs the checkpoint: story text is rasterized into the **viewport** region (the clear interior), not window 0's raw rect — this is what puts text below the banner and stops the status overlap.

- [ ] **Step 1: Write/adjust tests:** the existing `zork0_v6_pixel_canvas_is_nonempty` becomes a `raster`-mode test — assert the composited canvas is non-empty and that the story text region falls inside the classified chrome interior (no story glyph pixel lands on an opaque chrome pixel of the frame). Keep it tolerant (logic, not exact glyphs).
- [ ] **Step 2: Run, verify current behaviour, then implement the arm + `raster` composite.** Delete the now-dead `build_v6_canvas` body / repurpose.
- [ ] **Step 3: `cargo test -p app` and `cargo test` (workspace).**
- [ ] **Step 4: Commit.**

### Task A6: Phase A integration + docs + visual confirm

- [ ] **Step 1:** Extend `crates/app/tests/zork0_v6_windows.rs`: classify booted Zork0 → story = window 0, chrome = {1,7}; `story_viewport`'s native projection is inside the frame (top below the banner's opaque extent, sides between the columns).
- [ ] **Step 2:** `cargo test` (workspace) green; `cargo clippy -p app` clean.
- [ ] **Step 3:** Update docs (features/interpreter, standards) to describe v6 `raster` mode + the `v6_render` config; accurate + concise, major-feature level.
- [ ] **Step 4:** Commit (`Quest: SQ-0186`).
- [ ] **Step 5: VISUAL CONFIRM (user, `confirm`):** `babelmap stories/zork0-r393-s890714.z6` with `v6_render = raster` in a kitty/sixel terminal → undistorted scaled frame; compass shows the lit direction indicators over its base; status legible in the border; **story text sits inside the frame, below the banner, between the columns** (the top-margin + overlap fixes); blocky bitmap text is expected. Set `side-quest status SQ-0186 confirm`; relay any flavor line verbatim. Do NOT start Phase B until this looks right.

---

## PHASE B — `hybrid` mode (default)

> Detail firmed up after Phase A's visual confirm (the viewport rects and band geometry are validated there). Interfaces below are stable; pixel/band specifics may be tuned visually.

### Task B1: Chrome ring — draw the chrome AROUND the viewport

**Files:** Modify `crates/app/src/render/graphics.rs` (+ `v6_layout.rs` for band geometry).

**Interfaces:**
- Produces: `pub fn chrome_bands(pane: Rect, viewport: Rect) -> Vec<Rect>` — up to 4 cell rects (top, bottom, left, right) tiling `pane` minus `viewport`, none overlapping `viewport`.
- Produces: `GraphicsRender::draw_chrome_band(picker, chrome_canvas, scale, band: Rect, buf)` — render the crop of the scaled chrome canvas under `band`'s cells as one image placement in `band`. Cache per (content hash, band rect).

- [ ] **Step 1: Failing test** for `chrome_bands`: a viewport inset from the pane on all sides → 4 bands that exactly tile the ring, pairwise non-overlapping, none intersecting the viewport; viewport flush to an edge → that band omitted; viewport == pane → zero bands.
- [ ] **Step 2–5:** Implement band geometry + the per-band image draw (crop the scaled chrome canvas to the band's device region; render as an `Image` in the band's cells). Verify no band cell coincides with a viewport cell. `cargo test -p app`. Commit.

### Task B2: Terminal story in the viewport

**Files:** Modify `crates/app/src/render/screen.rs` (the `Layered` arm's `Hybrid` branch).

**Interfaces:** In the `Hybrid` branch: draw the chrome bands (B1), then call the primary-buffer render path (`render_transcript` via the existing primary-`Buffer` handling) with `area = viewport`. Return its real `StoryPaneMetrics` (scrollbar/scroll/links now work).

- [ ] **Step 1:** Test that with `v6_render = hybrid` + a picker, the arm returns `Some(metrics)` (not `None`) and does NOT rasterize story glyphs into the chrome canvas (the chrome canvas's viewport region stays transparent).
- [ ] **Step 2–5:** Implement; ensure the fallback (no picker) and `raster` branch are unchanged. `cargo test -p app`. Commit.

### Task B3: Inline graphics in the story

**Files:** Modify `crates/app/src/session.rs` (`v6_screen_model`) and/or the drain path so pictures drawn into the **story window** surface as inline transcript images (existing `inline_image` mechanism), while chrome-window pictures continue into the chrome canvas.

- [ ] **Step 1:** Test: a picture drawn into window 0 appears as an inline transcript image; a picture drawn into a chrome window does not. (Pixel-precise positioning of story pictures is out of scope — SQ-0450.)
- [ ] **Step 2–5:** Implement, `cargo test -p app`, commit.

### Task B4: Default to hybrid + integration + confirm

- [ ] **Step 1:** Flip the `v6_render` default to `Hybrid` (Task A4's enum default); the arm routes `Hybrid → hybrid`, `Raster → raster`.
- [ ] **Step 2:** `cargo test` (workspace) green; clippy clean; Zork0 integration asserts both modes reachable.
- [ ] **Step 3:** Update docs (both modes; default hybrid).
- [ ] **Step 4:** Commit (`Confirm: SQ-0186`).
- [ ] **Step 5: VISUAL CONFIRM (user):** `babelmap stories/zork0-r393-s890714.z6` (default hybrid) → scaled undistorted frame + compass + status in the border, and **crisp terminal story text** inside the frame with working scrollback/selection/`[more]` and inline pictures; `/set v6_render raster` (or config) switches to the pixel look; no-image terminal → cell fallback. Confirm on Arthur/Journey too if available. Set SQ-0186 `done` when both modes look right.

---

## Self-Review

- **Spec coverage:** classification (A1) ✓, chrome canvas + compass z-order (A2) ✓, clear-interior viewport + uniform scale (A3) ✓, mode selector (A4) ✓, raster mode incl. clear-interior fix (A5) ✓, chrome ring (B1) ✓, terminal story + metrics (B2) ✓, inline graphics (B3) ✓, default hybrid + fallback (B4) ✓, render modes (A4/A5/B4) ✓.
- **Type consistency:** `V6Layout`/`classify_windows`, `build_chrome_canvas`, `Scale`/`uniform_scale`/`story_viewport`, `chrome_bands`, `draw_chrome_band`, `V6RenderMode` — names consistent across tasks.
- **Phasing:** Phase A ships a correct-looking, testable `raster` mode (the layout brain), gated behind visual confirm before Phase B. Phase B is an isolated upgrade of only the story region. Non-v6/Glulx untouched throughout; `cell` fallback preserved.
- **Blind-build caveats (flag at review):** exact opaque-scan thresholds and band crops need visual tuning (A5/B1); the config enum must copy an existing field's exact pattern (A4); `render_transcript` reuse in a sub-rect (B2) must not disturb the non-v6 transcript path.
