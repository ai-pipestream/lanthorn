# v6 Phase 1c — Pixel-Canvas Render (design)

**Quest:** SQ-0186 (v6 graphical Z-machine). Follows Phase 1a (engine) + Phase 1b
(cell-quantized layered render). Companion follow-up: SQ-0450 (graphics quality:
scaling + terminal-font text).

## Problem

Phase 1b composites the v6 story pane by quantizing everything to the character
grid: graphics windows are sampled into per-cell background colours, text windows
paint their non-blank cells on top ("cell-text-wins"). A screenshot of Zork Zero
exposed the ceiling of that model:

- The ornate ~6px brown border draws as coarse low-res colour blocks — a v6
  address in pixels rounds to whole cells, so sub-cell art is lost.
- The main text is inset from the border by a few pixels (< 1 cell). Cell
  quantization rounds that inset to zero, so text overlaps the frame instead of
  sitting inside it.

The two coordinate systems don't reconcile at cell granularity: v6 places both
pictures **and** text at pixel coordinates, and the picture art carries sub-cell
detail. Any renderer that snaps to cells loses fidelity that a real v6 interpreter
keeps.

## Approach

Composite the **entire v6 story pane as one RGBA pixel canvas** at the terminal's
true device resolution, then display it as a **single terminal image** (kitty /
sixel / iterm, via the existing `ratatui-image` picker). Graphics windows are
blitted at exact pixel coordinates; text (every window, including the scrolling
main window and the live input line) is rasterized into the canvas with an
**embedded bitmap font**. Because graphics and text share one pixel surface, they
align at pixel precision — no cell snapping, no text-over-image z-order conflict.

When no image protocol is available (a plain terminal), fall back to the existing
Phase 1b cell-quantized layered render, unchanged.

This is the "true interpreter" model: the v6 pane becomes WYSIWYG pixels, the way
Frotz/Gargoyle draw it. It is deliberately a first working cut — quality knobs
(picture up/downscaling, aspect correction, richer text styling, a real terminal
font) are deferred to SQ-0450, per the user's "get it working, then improve
quality" direction.

### Why one image including the main text

In kitty and friends, a placed image is composited **over** the cell text beneath
it (default z-order). Zork Zero's border is a full-screen background graphics
window that sits behind *everything*, including the main scrolling text — so the
main text's cells are covered by the image region. Rendering that text as ordinary
ratatui cells would put it *under* the image and hide it. The only way the text
survives is to rasterize it **into** the canvas. Hence the whole pane — graphics,
upper-window grids, the main scrolling window, and the input line — is one image.

The cost is that, for v6 stories, the pane loses the native transcript niceties
(selection, search highlight, per-run styling, the scrollbar chrome). That is an
accepted first-cut tradeoff for a brand-new capability and is called out as a
follow-up, not a regression for any existing (v1–5 / Glulx) path, which are all
untouched.

## Coordinate model

Three spaces, related by fixed ratios:

- **Game space** — what the VM addresses. The v6 font cell is
  `V6_FONT_WIDTH × V6_FONT_HEIGHT = 8 × 8` game px (existing zvm constants). The
  VM is told the screen is `pane_cols·8 × pane_rows·8` game px; it draws pictures
  and positions windows in this space. Each text cell is 8×8 game px.
- **Device space** — real terminal pixels. One terminal cell is `cw × ch` device
  px, from `picker.font_size()` (`FontSize { width, height }`, e.g. 9×19 or 10×20).
- **Canvas** — the master RGBA image, built at device resolution:
  `W = pane_cols·cw`, `H = pane_rows·ch`.

Game→device transform is per-axis scaling: `sx = cw/8`, `sy = ch/8`. Equivalently,
game cell *n* maps to device cell *n* (the cell grids are 1:1); only sub-cell
offsets scale. A game pixel at `(gx, gy)` lands at device `(gx·cw/8, gy·ch/8)`.
Integer arithmetic throughout: `device_x = gx * cw / 8` (no floats).

A grid cell of window *w* at grid `(col, row)`, where the window origin is game
`(win.x_coord, win.y_coord)`, occupies device rect:

```
x = win.x_coord * cw / 8 + col * cw
y = win.y_coord * ch / 8 + row * ch
w = cw,  h = ch
```

i.e. each text cell is exactly one terminal cell (`cw × ch`) — glyphs stay crisp.

## Components

### 1. Embedded bitmap font — `crates/app/src/render/bitfont.rs`

A public-domain **8×8** ASCII bitmap font (CC0), embedded as a compile-time const
table — no runtime asset, no network. Primary source: the `font8x8` crate (CC0,
`#![no_std]`, zero transitive deps) added to the **app** crate only (zvm/gvm/scott
stay zero-dep). If the crate is unavailable at build time, vendor the printable
ASCII subset (0x20–0x7E) as a `[[u8; 8]; N]` const in this file — the data is CC0.

API:

```rust
/// Blit one glyph into `canvas` with its top-left at device pixel (px, py),
/// scaled (nearest-neighbour) to fit `cw × ch` device px, painting set bits in
/// `fg` over `bg` (bg skipped when `None`, leaving the canvas — for transparent
/// text over graphics). Out-of-range / unprintable chars draw as blank.
pub fn blit_glyph(canvas: &mut image::RgbaImage, ch_: char, px: u32, py: u32,
                  cw: u32, ch: u32, fg: image::Rgba<u8>, bg: Option<image::Rgba<u8>>);
```

8×8 upscaled to a ~9×19 cell is blocky but legible; a taller/native font is
SQ-0450. The glyph is scaled so text fills the cell height (readability) rather
than sitting 8px tall in a 19px cell.

### 2. Engine-neutral pixel model

Phase 1b's `WinNode::Layered(Vec<PositionedWindow>)` carries positions in **cells**
(`v6_screen_model` divides `x_coord/y_coord` by the font size), discarding the
sub-cell offset Phase 1c needs. Extend `PositionedWindow` with the game-pixel rect
so the same node feeds both paths:

```rust
pub struct PositionedWindow {
    pub x: u16, pub y: u16, pub w: u16, pub h: u16,   // cell rect (Phase 1b fallback)
    pub x_px: u16, pub y_px: u16,                     // game-pixel origin (Phase 1c)
    pub w_px: u16, pub h_px: u16,                     // game-pixel size   (Phase 1c)
    pub node: WinNode,
}
```

`v6_screen_model` fills the pixel fields from the raw `win.x_coord`/`y_coord`/
`x_size`/`y_size` (no division). The cell fields keep their current derivation so
the Phase 1b fallback is byte-identical. `WinNode::Layered` is unchanged; no new
variant.

### 3. The compositor — `crates/app/src/render/v6_canvas.rs`

A pure builder that turns the layered items + inputs into the master canvas:

```rust
pub fn build_v6_canvas(
    items: &[PositionedWindow],   // z-ordered: graphics first, then text windows
    pane_cells: (u16, u16),       // (cols, rows) of the story pane
    cell_px: (u16, u16),          // (cw, ch) from picker.font_size()
    bg: image::Rgba<u8>,          // pane background (packed model bg)
    main_text: &MainText,         // window-0 visible wrapped lines + input line + cursor
    colors: &ColorScheme,         // theme → default fg / resolve packed colours
) -> image::RgbaImage
```

Steps, in order (later steps paint over earlier — same z-order as Phase 1b):

1. Allocate `W = cols·cw`, `H = rows·ch`, fill with `bg`.
2. **Graphics layers** (`WinNode::Graphics`): blit each window's picture canvas
   (game-px `Arc<RgbaImage>`) into the master at its device rect
   `(x_px·cw/8, y_px·ch/8)` sized `(w_px·cw/8, h_px·ch/8)`, nearest-neighbour,
   honouring source alpha (transparent picture px leave the master unchanged).
3. **Upper text windows** (`WinNode::Grid`, windows 1–7): for each non-blank cell,
   `blit_glyph` at the cell's device rect with the cell's resolved fg/bg (a set
   cell bg paints its cell; an unset bg leaves the graphics/background showing).
4. **Main window** (`WinNode::Buffer { primary }`, window 0): rasterize the
   transcript's already-computed visible wrapped lines plus the live input line
   and a block cursor, via `blit_glyph`, into window 0's device rect. Text uses
   the theme fg over transparent bg (the background window shows through gaps).

The picture-canvas allocation stays bounded by the existing `CANVAS_PX_CAP`
(Phase 1b). The master canvas is bounded by the pane size × cell px (the pane is
already clamped to the terminal), so no new unbounded allocation.

`MainText` is a small struct the render layer fills from the transcript before
calling the builder (visible lines as `Vec<String>` from the existing
`visible_lines`/wrapping helpers, the input string, and the caret column/row). The
builder never touches `AppState` — it is unit-testable in isolation.

### 4. Render hook — `render_node`'s `Layered` arm (screen.rs)

```
if let Some(picker) = state.game_picker (image protocol available):
    build MainText from the transcript for the primary window
    canvas = build_v6_canvas(items, pane_cells, picker.font_size(), bg, main_text, colors)
    draw canvas as ONE image over `area` at native size (no letterbox), via a
    dedicated GraphicsRender-style path keyed on a content version so identical
    frames reuse the uploaded protocol
    return the StoryPaneMetrics for the primary window (scroll state unchanged)
else:
    (existing Phase 1b cell composite — unchanged)
```

The image is drawn at native size filling `area` exactly (canvas is sized to the
pane's device pixels), so there is no centering/letterbox. Caching: key on a cheap
content hash / monotonically-bumped version so per-keystroke frames that don't
change the canvas don't re-encode+upload; when the canvas changes (new text,
cursor blink, new picture) it re-uploads. Per-frame re-upload on change is
accepted for this first cut; upload-diffing is a perf follow-up.

## Data flow

```
zvm v6 screen state ─► session.v6_screen_model() ─► ScreenModel{ root: Layered(items with px rects) }
                                                            │
render_story_pane ─► render_node(Layered) ──┬─ picker? ─► build_v6_canvas ─► one terminal image
                                            └─ no picker ─► Phase 1b cell composite (unchanged)
```

## Error handling & edge cases

- **No opaque pixels / empty picture canvas** — blit nothing (bg / lower layers
  show). Same intent as Phase 1b's blank-window handling.
- **Window rect off the pane** — clip blits to the canvas bounds (saturating); an
  out-of-range origin simply contributes nothing.
- **Zero pane / zero cell size** — return without drawing (guard like the existing
  `area.width == 0` guards).
- **Unprintable / non-ASCII glyph** — blank cell (font covers 0x20–0x7E; Zork Zero
  text is ASCII). Latin-1 extension is a follow-up, not v1.
- **Fallback correctness** — when `game_picker` is `None`, behaviour is exactly
  Phase 1b; existing Phase 1b tests must still pass unchanged.

## Testing

Unit tests on the pure builder (`build_v6_canvas`) — no terminal, deterministic:

- A solid-fill picture window blits its colour to the expected device pixels
  (check a pixel at a known `(x_px·cw/8, y_px·ch/8)` offset).
- A grid cell with a known glyph produces set fg pixels within that cell's device
  rect and leaves neighbouring transparent cells showing the background.
- Main-window text rasterizes visible lines into window 0's rect; the input line
  and cursor appear on the bottom line.
- Sub-cell precision: a picture at game `x_coord = 4` (half a cell) lands at
  device `x = 4·cw/8`, *not* snapped to a cell boundary — the property Phase 1b
  could not express.
- `blit_glyph` unit tests: 'A' sets the expected bit pattern (scaled), a space
  paints nothing, an out-of-range char is blank.

Integration smoke (extends `crates/app/tests/zork0_v6_windows.rs` or a sibling):
Zork Zero boots headless; with a halfblocks picker present the render path builds
a non-empty canvas (a diagnostic hook returning the built canvas, or asserting the
image path is taken) — a fast on/off oracle that the pixel path engages and
produces pixels, without asserting exact art.

Full visual fidelity (does it *look* right) is the user's manual check in a real
kitty terminal — tracked as a `confirm` item, since a headless test cannot observe
the composited terminal image.

## Scope / non-goals (this phase)

- **In:** one-image pixel composite for v6 (graphics + all text + input); embedded
  8×8 font; pixel-precise positions; cell-render fallback; unit + smoke tests.
- **Out (SQ-0450 / later):** picture up/down-scaling & aspect correction; taller /
  native terminal font; bold/italic/reverse text styling in the raster; Latin-1
  glyphs; upload-diffing / partial-canvas updates; transcript selection/search
  inside the v6 image; `make_menu`/mouse (Phase 2).

## Files

- Create: `crates/app/src/render/bitfont.rs` (font + `blit_glyph`)
- Create: `crates/app/src/render/v6_canvas.rs` (`build_v6_canvas`, `MainText`)
- Modify: `crates/app/src/engine.rs` (pixel fields on `PositionedWindow`)
- Modify: `crates/app/src/session.rs` (`v6_screen_model` fills pixel fields)
- Modify: `crates/app/src/render/screen.rs` (`Layered` arm: picker → image path)
- Modify: `crates/app/src/render/mod.rs` (module decls)
- Modify: `crates/app/Cargo.toml` (add `font8x8`, if used)
- Test: unit tests in the new modules; smoke in `crates/app/tests/`
```
