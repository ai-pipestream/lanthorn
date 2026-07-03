# Glulx Graphics Windows — Design Spec

**Date:** 2026-07-03
**Status:** Approved for planning
**Feature:** In-game Glulx graphics windows (`wintype_Graphics`) — the first slice of in-game graphics support — plus a master "no images" toggle.

## Goal

Let a Glulx game open a `wintype_Graphics` window and draw into it (filled
rectangles + Blorb images), rendered in the terminal with the best available
graphics protocol and a universal half-block fallback. Add a single master
switch that disables all image rendering (this feature **and** the existing
story-picker cover art) for text-only operation.

## Background

In Glulx, all graphics go through Glk (the VM has no graphics opcodes — they are
`@glk` calls). Today `gvm` stubs the entire Glk graphics surface: every graphics
`@glk` selector hits a return-0 catch-all, `WinType::from_arg` rejects
`wintype_Graphics` (5), and `glk_gestalt` reports no graphics — so games detect
"no graphics" and run text-only. This feature implements the graphics-window
subset of that surface. It reuses the image infrastructure built for cover art
(`blorb` decode, `app::cover::decode`, `ratatui-image` rendering).

Graphics support has three parts: a shared foundation, **graphics windows**
(this spec), and **inline/margin buffer images** (a later slice). Glk's
`gestalt_DrawImage` takes a window-type argument, so this slice can advertise
graphics-window support while cleanly declining text-buffer image support; a
later slice adds the latter without disturbing this one.

## Scope

**In scope (this slice)**
- Open `wintype_Graphics` windows, both fixed (pixel) and proportional splits.
- Drawing ops: `glk_window_fill_rect`, `glk_window_erase_rect`,
  `glk_window_set_background_color`, `glk_window_clear` (graphics → erase to bg),
  `glk_image_draw`, `glk_image_draw_scaled`, `glk_image_get_info`.
- `glk_gestalt` reporting: `Graphics`, `DrawImage(wintype_Graphics)`,
  `GraphicsTransparency`.
- `glk_window_get_size` returns pixel dimensions for graphics windows.
- Events: `evtype_Redraw` (on graphics-window open/resize) and `evtype_Arrange`
  (on layout change) — many games draw only after receiving these.
- Rendering: per-window RGBA canvas → `ratatui-image` (protocol + half-block
  fallback).
- Master "no images" toggle (config + CLI flag) gating this feature **and** the
  existing cover-art preview.

**Out of scope (explicitly deferred to later slices)**
- Inline/margin images in text-buffer windows (`imagealign_*`,
  `glk_window_flow_break`) — Surface A.
- Mouse input in graphics windows (`glk_request_mouse_event` returning pixel
  coordinates).
- Blorb `Reso` (per-image scale ratios) and `APal` (adaptive palette) chunk
  handling; images are drawn at their intrinsic or game-specified size.
- Z-machine V6 graphics (unrelated; V6 is rejected at load).

## Decisions (locked from brainstorming)

| Question | Decision |
|---|---|
| First surface | Graphics windows (Surface B); inline images + mouse deferred. |
| Op/event set | Core-complete: open/fill/erase/bg/clear/draw/draw_scaled/get_info/gestalt + Redraw/Arrange. Mouse deferred. |
| Rendering | Per-window RGBA canvas composited by the app, rendered via `ratatui-image`. |
| Image ownership | `gvm` passes resource **numbers** + pixel geometry to the backend; the **app** decodes/composites/resolves. `gvm` never touches pixels. |
| Pixel↔cell scale | `char_pixels` from the terminal font metrics (`Picker::font_size()`), fallback to a fixed ratio. |
| "No images" toggle | Master switch (`config.images` + `--no-images`) disabling in-game graphics **and** cover art. |
| Verification | Unit tests + a synthetic Glulx graphics fixture (mandatory); a real-game story smoke **if** a graphical Glulx game exists in the library (optional). |

## Global Constraints

- **`gvm` and `zvm` stay zero-dependency.** `gvm` does no image decoding or
  compositing — it passes resource numbers, colors, and pixel geometry to the
  backend, and carries only a plain `bool` for the graphics-enabled gate.
- All new image decode/compositing lives in the **`app` crate** (which already
  depends on `image`/`ratatui-image`).
- **Every failure path is silent** — no panics; missing/undecodable images and
  out-of-range coordinates degrade to no-op or clip.
- **Cross-platform** (Windows/Linux/macOS): the half-block fallback must render
  graphics windows on any terminal; protocols are progressive enhancement.
- **Themeable UI:** the graphics-window default background gets a `graphics`
  style selector (ColorScheme field + `style.rs` selector + render apply), per
  the project's styling rule.
- **Master toggle default is ON** (`images = true`); `--no-images` / `images =
  false` disables all image rendering and wins over `--image-protocol`.
- Clippy stays at 0 warnings; the existing full suite stays green.

## Architecture

Four layers with narrow seams. The load-bearing decision is the image-ownership
seam: **`gvm` speaks resource numbers + geometry; the app owns pixels.**

### Layer 1 — `gvm`: Glk model + `@glk` dispatch (zero-dep)

- `WinType` gains `Graphics`; `from_arg(5)`/`to_arg → 5`.
- `Model::window_open` accepts `Graphics`. Window layout: a **fixed** graphics
  split carries a pixel size, converted to cells via the backend's
  `char_pixels`; a **proportional** split is a percentage, like text windows.
- `@glk` dispatch implements the graphics selectors (the `0x00E0`–`0x00EB`
  block) by calling the new `GlkBackend` methods, passing resource numbers,
  colors (24-bit `0xRRGGBB`), and pixel coordinates/sizes.
- `glk_window_get_size` for a graphics window returns pixels = `cells ×
  char_pixels`.
- `glk_gestalt`, **gated on the graphics-enabled flag**: `Graphics → 1`,
  `DrawImage` with arg `wintype_Graphics → 1`, `DrawImage` with arg
  `wintype_TextBuffer → 0` (deferred slice), `GraphicsTransparency → 1`. When
  the flag is off, all return 0 and `window_open(Graphics)` fails — games run
  text-only.
- Event generation: emit `evtype_Redraw` when a graphics window is first laid
  out or its size changes, and `evtype_Arrange` on any layout change.
- The graphics-enabled `bool` is passed at `Machine`/`Model` construction
  (alongside the existing acceleration flag).

### Layer 2 — `GlkBackend` trait extension (in `gvm`, no-op defaults)

New methods, each with a no-op/`None` default so `zvm` and existing backends
are unaffected:

```
fn char_pixels(&self) -> (u32, u32);                 // default e.g. (1, 1)
fn image_info(&mut self, resnum: u32) -> Option<(u32, u32)>;
fn graphics_fill_rect(&mut self, win: u32, color: u32, left: i32, top: i32, w: u32, h: u32);
fn graphics_erase_rect(&mut self, win: u32, left: i32, top: i32, w: u32, h: u32);
fn graphics_set_background(&mut self, win: u32, color: u32);
fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>);
```

`window_open`/`window_clear`/`window_layout` already exist; graphics windows are
handled within them by window type. `TestBackend` records these calls so unit
tests can assert dispatch, args, and geometry.

### Layer 3 — `app` `AppGlk` backend

- Per-graphics-window state: an `image::RgbaImage` **canvas** plus its
  background color, keyed by window id. Canvas resolution = `window_cells ×
  char_pixels`.
- **Resource resolution:** at game start, if the story is a Blorb, `AppGlk`
  holds its `Pict` resources (the parsed `Blorb` or a resnum→bytes map).
  `image_info`/`graphics_draw_image` resolve `resnum` → decode (reuse
  `app::cover::decode`) → cache the decoded image by resnum → composite.
- `graphics_fill_rect`/`erase_rect` write solid color rectangles into the
  canvas (clipped to bounds); `graphics_draw_image` blits/scales the decoded
  image at `(x, y)` (honoring alpha for transparency); `set_background` records
  the bg (used by `erase_rect`/`window_clear`).
- `char_pixels` derives from the session `ratatui-image` `Picker`'s font size,
  fallback to a fixed ratio.
- `AppGlk`'s graphics windows surface as `WinNode::Graphics` leaves for
  rendering (mirroring how text windows already become `WinNode`s).

### Layer 4 — `app` render

- `WinNode` gains a `Graphics` variant carrying a handle to the window's canvas
  (and a version counter for cache invalidation).
- `render/screen.rs` `render_node` renders a graphics leaf: build-or-reuse a
  `ratatui-image` protocol from the canvas fitted to the leaf's rect, then draw
  it — a per-window protocol cache keyed by `(canvas version, rect)`, the same
  pattern as `CoverState`. Cells not covered by the canvas use the `graphics`
  style.
- One shared session `Picker` (as with cover art), built once, honoring
  `--image-protocol`.

## Master "no images" toggle

- **Config/CLI:** `config.images: bool` (default `true`; TOML `images = false`)
  plus a `--no-images` CLI flag forcing it off. Follows the existing
  config-bool + CLI-override pattern.
- **In-game graphics:** the resolved value is passed into `gvm` as the
  graphics-enabled `bool`. Off → gestalt reports no graphics and
  `window_open(Graphics)` fails → text-only.
- **Cover art:** when off, the picker skips building its `Picker` and loading
  covers (gated on `cfg.images`), silencing the shipped cover-art preview too.
- **Composition:** `--image-protocol` selects *how* images render when on;
  `--no-images` / `images = false` is the master *off* and overrides any
  protocol.

## Data flow

```
game: @glk glk_window_fill_rect(win, color, l, t, w, h)
  → gvm @glk dispatch → backend.graphics_fill_rect(win, color, l, t, w, h)
  → AppGlk composites the rect into window `win`'s RGBA canvas (bumps version)
  → next frame: render_node builds/reuses a ratatui-image protocol from the
    canvas fitted to the window's rect and draws it

game: @glk glk_image_draw(win, resnum, x, y)
  → backend.graphics_draw_image(win, resnum, x, y, None)
  → AppGlk resolves resnum → decode+cache → composite at (x, y) → render

game: @glk glk_select(...)
  → gvm emits evtype_Redraw (window shown/resized) / evtype_Arrange (layout
    changed) → game redraws its canvas

images disabled (cfg.images = false):
  → gvm graphics-enabled = false → gestalt 0, window_open(Graphics) fails
  → picker builds no Picker, loads no covers
```

## Coordinate / sizing model

- `char_pixels = (cw, ch)` = terminal font-cell pixel size from
  `Picker::font_size()`, fallback to a fixed ratio (e.g. `8×16`).
- **Fixed** graphics window (`glk_window_open` with `winmethod_Fixed` + a pixel
  size `N`): laid out as `ceil(N / ch)` cells (or `/cw` for a vertical split);
  `glk_window_get_size` reports back `cells × char_pixels`.
- **Proportional** graphics window: a percentage split like text windows;
  `get_size` reports `cells × char_pixels`.
- Canvas resolution = `window_cells × char_pixels`, so the game's pixel
  coordinates map ~1:1 into the canvas. `ratatui-image` renders the canvas to
  the cell rect (protocol ≈ 1:1; half-blocks downsamples).

## Error handling (all silent, no panics)

- Story is a bare `.ulx` (no Blorb) or the `Pict` resource is missing:
  `image_info → None`, `graphics_draw_image → no-op`. `fill_rect`/`erase_rect`
  still work.
- Undecodable image bytes → no-op.
- Out-of-bounds fill/draw coordinates → clipped to the canvas.
- Terminal without a graphics protocol → half-block fallback (blocky but
  visible).
- Graphics disabled → gestalt/window_open decline; no drawing occurs.

## Testing

- **`gvm` unit (hand-built Glulx bytecode + `TestBackend`):** each graphics
  `@glk` selector dispatches to the right backend method with correct args and
  pixel geometry; `glk_gestalt` values with the flag on and off;
  `glk_window_get_size` pixel math; `evtype_Redraw`/`evtype_Arrange` emission on
  open/resize/layout change; `window_open(Graphics)` rejected when the flag is
  off.
- **`app` unit:** canvas compositing (`fill_rect` fills the right pixels,
  `erase_rect`/`window_clear` reset to bg, `draw_image` composites and scales,
  alpha honored); `AppGlk` resnum resolution + decode cache; `char_pixels`
  derivation; the `cfg.images = false` gate silences cover-art loading.
- **render test:** a graphics-window canvas renders (forced half-blocks) into
  its rect (`▀` present), like the cover-art render test.
- **synthetic fixture:** a small hand-assembled Glulx story that opens a
  graphics window, sets a background, fills a rect, and draws an image; driven
  headless (via `gvm-cli` or the app's headless harness) and asserted.
- **optional story-level:** if a graphical Glulx game is present in the story
  library, a smoke test that it opens a graphics window and runs without error.

## Verification

- Workspace builds; full suite passes (current baseline + new tests).
- `cargo clippy --workspace --all-targets` clean.
- `gvm`/`zvm` `[dependencies]` unchanged (zero-dep intact); no new deps
  (`image`/`ratatui-image` already in `app`).
- Manual: a graphical Glulx game shows its graphics window on a protocol-capable
  terminal and as half-blocks with `--image-protocol halfblocks`; `--no-images`
  runs the same game text-only and hides picker cover art.

## Follow-ups (not this slice)

- Surface A: inline/margin images in text-buffer windows.
- Mouse input in graphics windows.
- Blorb `Reso`/`APal` handling for spec-accurate image scaling.
