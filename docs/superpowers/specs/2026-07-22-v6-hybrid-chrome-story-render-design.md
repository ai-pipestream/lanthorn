# v6 Hybrid Render — Pixel Chrome + Terminal Story (design)

**Quest:** SQ-0186 (v6 graphical Z-machine). Supersedes the "whole pane as one
pixel image" approach of the Phase 1c spec
(`2026-07-22-v6-phase1c-pixel-render-design.md`) for the *rendered result*; it
reuses that work's infrastructure (native-resolution advertising via the Blorb
`Reso` chunk, the embedded bitmap font, `set_margins`, the picture-canvas
plumbing). Quality follow-ups tracked under SQ-0450.

## Problem

Rendering the entire v6 story pane as one rasterized image (Phase 1c) fought the
grain of the app: it re-rasterized the main story text with a bitmap font
(losing crisp glyphs, selection, scrollback, the `[more]` pager, and inline-image
support), and it couldn't place text where the game wanted it. Iterating on Zork
Zero surfaced three stubborn symptoms — text overflowing the frame's right
column, status text ("Banquet Hall", "Moves:", score) scattered over the story,
and story text drawn on top of the top banner — all of which trace to the same
root: **v6 positions everything in pixels, and a single-raster model has to
reproduce that pixel layout perfectly or it looks wrong.**

## Approach

Split the v6 pane into two rendering regimes that each play to their strengths:

- **Chrome** — the decorative frame (borders, banners, background pictures) and
  the game-positioned status text (score, location). Rendered as a **scaled,
  pixel-aspect-ratio-accurate image** (an embedded bitmap font rasterizes the
  status text). This is where pixel fidelity matters.
- **Story** — the main scrolling text window. Rendered with the app's **normal
  terminal transcript renderer** — real terminal font, selection, scrollback, the
  `[more]` pager, and **inline graphics** — inside a cell rectangle positioned to
  sit within the frame.

The two regimes are **disjoint by construction**: the chrome fills a border ring
around a central **story viewport**, and the story text fills the viewport. They
never contend for the same terminal cells, so both render natively with no
image-over-text z-order conflict.

This is generic for v6 (no per-game constants) and degrades cleanly: with no
image protocol, it falls back to the existing Phase 1b cell composite.

### Render modes

Three selectable modes (config `v6_render`, default `hybrid`):

- **`hybrid`** (default) — the chrome-ring + terminal-story split described here.
- **`raster`** — the whole pane as one scaled pixel image (chrome + story text
  rasterized with the bitmap font). Deliberately **feature-limited** (no terminal
  selection/scrollback/inline-image in the story), retained as the faithful
  pixel-look option and a fallback where the hybrid ring is unwanted. It reuses
  the same window classification and clear-interior placement, so text still
  lands in the frame's interior — it just draws the story as bitmap glyphs into
  the one canvas instead of leaving a viewport for terminal text.
- **`cell`** — the existing Phase 1b cell composite, used automatically when no
  image protocol is available (regardless of the configured mode).

`hybrid` and `raster` share everything up to the story region; they diverge only
in how the story area is drawn (terminal viewport vs bitmap into the canvas).

### Why this is the right shape

- The story text is the bulk of what the player reads; keeping it in the terminal
  renderer preserves every text feature the app already has.
- Pixel work is confined to the border, which is small, static per room, and
  genuinely pixel-authored.
- The scrollbar / `[more]` / selection metrics that the all-image model broke
  come back for free, because the story region *is* a normal transcript render.
- The "status scattered over the story" and "text over the banner" symptoms
  vanish: status lives in the chrome ring (outside the viewport), and the
  viewport is by definition the frame's clear interior (below the banner).

## Coordinate & scaling model

Three spaces:

- **Native pixel space** `N_w × N_h` — the resolution advertised to the game (the
  Blorb `Reso` standard window; Zork0 = 320×200; fallback 320×200). Every v6
  window rect, cursor, and picture is in native px. Already wired (Phase 1c).
- **Pane device space** — the story pane is `P_cols × P_rows` cells; one cell is
  `C_w × C_h` device px (`picker.font_size()`). Pane device size =
  `P_cols·C_w × P_rows·C_h`.
- **Terminal cell space** — where the story text renders.

**Uniform scale (pixel-aspect-ratio accurate):**

```
s = min( P_cols·C_w / N_w ,  P_rows·C_h / N_h )
scaled_w = N_w · s     scaled_h = N_h · s
off_x = (P_cols·C_w − scaled_w) / 2      (letterbox centering, device px)
off_y = (P_rows·C_h − scaled_h) / 2
```

`s` is the **same factor in x and y**, so the frame is never stretched — only
scaled and centered, with letterbox margins (pane background) if the pane aspect
differs from the native aspect. A native point `(x, y)` maps to device
`(off_x + x·s, off_y + y·s)`.

The **story text keeps the terminal's own cell aspect** (it is not scaled — it is
ordinary terminal text). So the frame carries the game's pixel aspect and the
text carries the terminal's; that is correct, because they are different
rendering modes.

## Components

### 1. Window classification — `classify_windows`

Generic v6 rule, no hardcoded window numbers beyond the primary convention:

- **Story window** = the primary buffered/scrolling window (v6 window 0, already
  marked `Buffer { primary: true }` by `v6_screen_model`). Exactly one.
- **Chrome windows** = every other live window (graphics windows and the
  upper/grid windows the game cursor-positions status into).

Pictures drawn **into the story window** (Zork0 draws two small ones at its
top-left) render as **inline transcript images** via the existing
`inline_image` path. Pictures in **chrome windows** composite into the chrome
canvas. (Pixel-precise placement of story-window pictures is a refinement —
SQ-0450.)

### 2. Chrome canvas — `build_chrome_canvas(chrome_windows, native_size) -> RgbaImage`

A native-resolution `N_w × N_h` RGBA canvas, **transparent by default**:

1. Composite the chrome-window pictures, **preserving picture-on-picture draw
   order** (later draws land on top), each at its native pixel rect honoring
   source alpha:
   - *Within a window*, the existing per-window picture canvas already composites
     in draw order — this is how a compass is built up: a base plus a stack of
     small transparent direction-indicator images drawn over it at the same spot
     (Zork0 draws ~8 of them into its top window). That must render as the base
     with the lit directions on top.
   - *Across overlapping chrome windows* (e.g. a compass window layered over the
     banner-frame window), composite in the order the windows were last drawn to,
     so a later overlay lands on top of an earlier background. Non-overlapping
     chrome windows are order-independent.

   This picture stacking is the chrome plane's z-order stress test (see Testing).
2. Rasterize each chrome window's **grid text** (status) with the embedded bitmap
   font at native pixel positions: cell `(col, row)` → native
   `(win.x_coord + col·FONT_W, win.y_coord + row·FONT_H)`, colored by the cell's
   resolved fg/bg. The game positions this text with `set_cursor` (pixel
   coordinates, sometimes edge-relative); it is rendered wherever the VM's grid
   places it — **not** clamped to the window's declared pixel height. (Zork0's
   window 1 is declared 5px tall but the game legitimately writes status across
   ~11 rows via the cursor; clamping is what previously hid the status. In this
   model the status lands in the chrome ring, not over the story, so it renders
   at full extent safely.)

The story window's rect is left untouched (transparent) — that transparent
interior is what the story viewport occupies.

### 3. Story viewport — `story_viewport(story_rect, chrome_canvas, s, off, cell_px) -> Rect`

The cell rectangle where the story text renders — **the largest cell-aligned rect
inside the story window that overlaps no opaque chrome pixel.** This single
mechanism clears the banner (top) and the side columns (left/right) uniformly, no
per-game tuning:

1. Map the story window's native rect → device rect via `(s, off)`.
2. Inset each edge inward while the outermost row/column of the rect still
   overlaps an opaque chrome pixel (alpha ≥ threshold) within the story's device
   span — i.e. shrink until every edge is in the clear interior.
3. Snap the resulting device rect **inward** to whole terminal cells
   (`ceil` the top-left, `floor` the bottom-right to cell boundaries). Any sub-cell
   sliver between the frame and the text is covered by the chrome ring, so no gap
   shows.

The game's `set_margins` values corroborate this (Zork0: `left_margin=32` ≈ the
left column width) but the opaque-pixel scan is the authority, because it also
handles the top banner, which `set_margins` cannot express.

Result: a `Rect` in terminal-cell coordinates, guaranteed inside the frame.

### 4. Chrome ring render — up to 4 image bands around the viewport

The chrome must occupy the pane **around** the story viewport, never over it (an
anchored image placement, e.g. kitty, owns every cell in its rect — text under it
is lost). Render the ring as up to four image placements, each showing the
corresponding crop of the scaled chrome canvas:

```
┌───────────── top band ─────────────┐
│                                     │
├──────┬───────────────────┬──────────┤
│ left │   story viewport  │  right   │
│ band │  (terminal text)  │  band    │
├──────┴───────────────────┴──────────┤
│              bottom band             │
└──────────────────────────────────────┘
```

Each band is the chrome-canvas region under those pane cells, scaled by `s`,
drawn via the image protocol into that cell rect. Empty bands (viewport flush to
a pane edge) are skipped. Bands are cached per (content hash, band rect) like the
existing `GraphicsRender` cache. When the story viewport equals the whole pane
(no chrome), no bands are drawn.

### 5. Story render — the existing transcript path, into the viewport

Call the normal `render_transcript(...)` (or `render_node` for the primary
buffer) with the **story viewport** as its `area`. This yields crisp text,
scrollback, selection, the `[more]` pager, inline images, and — importantly —
real `StoryPaneMetrics` (scrollbar, `max_scroll`, `total_rows`, links), which the
all-image model had to stub as `None`.

## Data flow

```
zvm v6 window table + Reso native size
        │
        ▼
classify_windows ──► story window (window 0)     chrome windows (1..7)
        │                    │                            │
        │                    ▼                            ▼
        │           render_transcript            build_chrome_canvas
        │           into story viewport          (native px, transparent interior)
        │                    ▲                            │
        │   story_viewport ──┘ ◄── uniform scale s, offset, opaque-pixel scan
        │                                                 │
        ▼                                                 ▼
   pane rect ───────────────► compose: chrome ring (image bands) + story text
                                            │  (no picker → Phase 1b cell fallback)
                                            ▼
                                     terminal frame
```

## Edge cases & fallback

- **No image protocol** (`state.game_picker` is `None`): fall back to the
  existing Phase 1b cell composite (`WinNode::Layered` cell path), unchanged. The
  hybrid path activates only with a picker.
- **No chrome** (a v6 game with only a story window): the viewport is the whole
  pane; zero image bands; pure transcript render.
- **Story window absent / degenerate**: viewport falls back to the full pane
  (never empty), so text always has somewhere to go.
- **Chrome fully opaque over the story rect** (a game that fills the interior):
  the opaque-pixel scan yields a minimal viewport; clamp to a sane minimum (≥ a
  few cells) and let the chrome image show — pathological, acceptable.
- **Letterbox**: the pane area outside the scaled chrome is painted with the pane
  background (theme), matching the story bg.
- **v1–v5 / Glulx**: untouched — the hybrid path is reached only when
  `screen.v6.is_some()` and a picker exists.

## Worked example — Zork0 Banquet Hall (ground truth)

Native 320×200. Windows: 7 = full-screen frame `(0,0,320,200)` (top banner
Pict 5 at y=1..35, left column at x=1..37, right column at x=284..321,
transparent interior); 1 = status `(1,1,320,5)` carrying the compass — a base
plus ~8 overlapping transparent direction-indicator pictures drawn to the same
spot (available exits) — and grid status text "Banquet Hall", "Moves:", score
positioned by `set_cursor`; 0 = story `(6,6,310,192)`, `set_margins left=32
right=0`.

- Classify: story = window 0; chrome = windows 1 & 7.
- Chrome canvas: frame + rasterized status; interior transparent.
- Story viewport (opaque-pixel scan): left inset ≈ column right edge 37, right
  inset ≈ column left edge 284, **top inset ≈ banner bottom 35** (the piece
  `set_margins` couldn't give), bottom ≈ 200 → native `~[38..284] × [35..198]`,
  scaled by `s` and snapped to cells.
- Render: story text (terminal font) in that viewport; chrome ring (banner top
  band + column side bands) around it; status renders in the top band.

## Testing

Unit (pure, deterministic — no terminal):

- `classify_windows`: the primary buffer is the story window; all others chrome.
- `build_chrome_canvas`: frame pixels opaque, story-window interior transparent,
  status glyph pixels present at expected native coords.
- **Picture-on-picture z-order (compass stress test):** a stack of overlapping
  transparent images drawn to the same spot in draw order composites base-first,
  last-on-top — the later (indicator) image's opaque pixels win over the earlier
  (base) ones, and both windows' contributions land in the correct order when a
  compass window overlays a banner window. Mirrors Zork0's compass + direction
  indicators.
- `story_viewport`: given a synthetic chrome canvas with an opaque border ring,
  the returned cell rect is the clear interior, snapped inward, never overlapping
  an opaque pixel; full-pane when no chrome.
- Uniform scale + letterbox offset math (aspect-preserved; centered).
- Band decomposition: 4 bands tile the ring exactly and none overlaps the
  viewport.

Integration (Zork0, headless, skip-if-absent):

- Classify Zork0 → story = window 0, chrome = {1, 7}.
- Story viewport is strictly inside the frame: its native-mapped rect contains no
  opaque chrome pixel; its top is below the banner; its sides are between the
  columns.
- No-picker path still produces the Phase 1b `Layered` cell model.

Visual (user, `confirm`): a real kitty/sixel terminal — undistorted scaled frame,
status legible in the border, crisp terminal story text inside the frame with
working scrollback/selection, inline pictures in the story. Headless can't judge
the composited terminal image.

## Scope / non-goals

- **In:** window classification; uniform aspect-preserving scale + letterbox;
  chrome canvas (frame + status raster) with transparent interior; clear-interior
  story viewport; the `raster` mode (whole-pane image, story as bitmap) as a
  retained feature-limited option; the `hybrid` mode (chrome ring image bands +
  terminal story render with scrollback + inline images) as default; the
  `v6_render` mode selector; `cell` fallback with no picker; unit + integration +
  visual-confirm tests.
- **Out (SQ-0450 / later):** pixel-precise placement of pictures drawn into the
  story window (currently inline); per-image `Reso` scaling ratios; sub-cell
  precision at the story/frame seam beyond snapping; edge-relative status cursor
  nuances if any game needs them; `make_menu` / mouse (Phase 2); multiple
  simultaneous story windows.

## Files

- Create: `crates/app/src/render/v6_layout.rs` — `classify_windows`,
  `build_chrome_canvas`, `story_viewport`, scale/offset + band helpers.
- Modify: `crates/app/src/render/screen.rs` — v6 (`WinNode::Layered`) arm: picker
  present → hybrid render (chrome bands + `render_transcript` in the viewport,
  returning real metrics); else Phase 1b cell fallback.
- Modify: `crates/app/src/render/graphics.rs` — draw a scaled chrome-canvas crop
  into a cell band (generalize/replace `draw_v6_canvas`).
- Reuse: `render/bitfont.rs` (status raster), `render/transcript.rs` (story
  text), `render/inline_image.rs` (story inline graphics), the `Reso`/native-size
  and `set_margins` plumbing from Phase 1c.
- Retire/repurpose: `crates/app/src/render/v6_canvas.rs` — its whole-pane
  compositor is replaced; the reusable helpers (glyph blit wrappers, packed-colour
  resolution, scaled blit) move into `v6_layout.rs` or `bitfont.rs`.
- Test: unit tests in `v6_layout.rs`; integration in
  `crates/app/tests/zork0_v6_windows.rs`.
```
