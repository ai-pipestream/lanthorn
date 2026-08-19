# v6 Phase 1c — Pixel-Canvas Render Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the v6 story pane as one device-resolution RGBA canvas — graphics blitted at exact pixel coords, all text rasterized via an embedded bitmap font — shown as a single terminal image, with the Phase 1b cell composite as the no-image-protocol fallback.

**Architecture:** `v6_screen_model` keeps building `WinNode::Layered`, now carrying game-pixel rects. When a picker (image protocol) exists, `render_node`'s `Layered` arm builds one master canvas via `build_v6_canvas` and draws it through a cached single-image renderer; otherwise it runs the unchanged Phase 1b cell path.

**Tech Stack:** Rust, `image` crate (RgbaImage), `ratatui` / `ratatui-image` (Picker/Protocol), `font8x8` (CC0, no_std embedded bitmap font).

## Global Constraints

- Crates `zvm`, `gvm`, `scott` stay ZERO external dependencies. New deps (`font8x8`) go in the `app` crate only.
- v1–v5 and Glulx render paths must stay byte-identical: the new behaviour is reachable ONLY when `screen.v6.is_some()` (→ `WinNode::Layered`) AND `state.game_picker.is_some()`.
- Stage git files EXPLICITLY by path; never `git add -A`.
- Run the FULL `cargo test` (workspace) before declaring a task done, not just `-p app`.
- Commit trailers on every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh`
  and `Quest: SQ-0186` (final task: `Confirm: SQ-0186`).
- Coordinate transform is integer-only: device = game × cellpx / 8. Font cell = 8×8 game px (`zvm::screen::V6_FONT_WIDTH`/`V6_FONT_HEIGHT`).
- No back-compat shims (pre-release).

---

### Task 1: Embedded bitmap font + `blit_glyph`

**Files:**
- Create: `crates/app/src/render/bitfont.rs`
- Modify: `crates/app/src/render/mod.rs` (add `pub mod bitfont;`)
- Modify: `crates/app/Cargo.toml` (add `font8x8 = "0.3"`)

**Interfaces:**
- Produces: `pub fn blit_glyph(canvas: &mut image::RgbaImage, glyph: char, px: u32, py: u32, cw: u32, ch: u32, fg: image::Rgba<u8>, bg: Option<image::Rgba<u8>>)`

- [ ] **Step 1: Add the dependency**

Run: `cargo add font8x8@0.3 -p app`
Expected: `font8x8` appears under `[dependencies]` in `crates/app/Cargo.toml`.

- [ ] **Step 2: Write the failing tests**

Create `crates/app/src/render/bitfont.rs`:

```rust
//! Embedded CC0 8×8 ASCII bitmap font (`font8x8`), rasterized into an RGBA
//! canvas for the v6 pixel composite (Phase 1c). Glyphs are scaled
//! nearest-neighbour to fill a `cw × ch` device-pixel cell so text stays
//! legible at terminal cell sizes (~9×19). A taller/native font is SQ-0450.

use font8x8::UnicodeFonts;
use image::{Rgba, RgbaImage};

/// Blit one glyph into `canvas`, top-left at device pixel `(px, py)`, scaled to
/// `cw × ch` device px. Set bits paint `fg`; clear bits paint `bg` when `Some`
/// (skipped when `None`, leaving the canvas — transparent text over graphics).
/// Unprintable / out-of-font chars paint only `bg` (a blank cell). Blits are
/// clipped to the canvas bounds.
pub fn blit_glyph(
    canvas: &mut RgbaImage,
    glyph: char,
    px: u32,
    py: u32,
    cw: u32,
    ch: u32,
    fg: Rgba<u8>,
    bg: Option<Rgba<u8>>,
) {
    let bits = font8x8::BASIC_FONTS.get(glyph); // Option<[u8; 8]>
    let (cwidth, cheight) = (canvas.width(), canvas.height());
    for dy in 0..ch {
        let oy = py + dy;
        if oy >= cheight {
            break;
        }
        let row = (dy * 8 / ch) as usize; // nearest source row
        for dx in 0..cw {
            let ox = px + dx;
            if ox >= cwidth {
                break;
            }
            let col = (dx * 8 / cw) as u32; // nearest source col
            // font8x8 packs each row LSB = leftmost column.
            let on = bits.map_or(false, |g| g[row] & (1 << col) != 0);
            if on {
                canvas.put_pixel(ox, oy, fg);
            } else if let Some(b) = bg {
                canvas.put_pixel(ox, oy, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_paints_only_bg() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 255]));
        blit_glyph(&mut c, ' ', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), Some(Rgba([9, 9, 9, 255])));
        // No set bits → every pixel is the bg fill, none is fg.
        assert!(c.pixels().all(|p| *p == Rgba([9, 9, 9, 255])));
    }

    #[test]
    fn glyph_sets_some_fg_pixels() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, 'A', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None);
        // 'A' has set bits → at least one fg pixel, and transparent bg elsewhere.
        assert!(c.pixels().any(|p| *p == Rgba([255, 0, 0, 255])), "A has fg pixels");
        assert!(c.pixels().any(|p| p[3] == 0), "unset bits stay transparent (bg=None)");
    }

    #[test]
    fn transparent_bg_leaves_canvas_on_clear_bits() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
        blit_glyph(&mut c, '.', 0, 0, 8, 8, Rgba([255, 255, 255, 255]), None);
        // A '.' is mostly clear; those cells keep the original canvas colour.
        assert!(c.pixels().any(|p| *p == Rgba([1, 2, 3, 255])), "clear bits keep canvas");
    }

    #[test]
    fn out_of_range_char_is_blank() {
        let mut c = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, '\u{2588}', 0, 0, 8, 8, Rgba([255, 0, 0, 255]), None);
        assert!(c.pixels().all(|p| p[3] == 0), "unknown glyph paints nothing with bg=None");
    }

    #[test]
    fn scales_up_to_fill_cell() {
        // 8×8 glyph blitted into a 16×16 cell must touch the lower-right quadrant.
        let mut c = RgbaImage::from_pixel(16, 16, Rgba([0, 0, 0, 0]));
        blit_glyph(&mut c, 'M', 0, 0, 16, 16, Rgba([255, 0, 0, 255]), None);
        assert!(
            (8..16).any(|y| (0..16).any(|x| c.get_pixel(x, y)[3] == 255)),
            "scaled glyph reaches the lower half of the cell"
        );
    }
}
```

- [ ] **Step 3: Add `pub mod bitfont;` to `crates/app/src/render/mod.rs`** (alongside the other `pub mod` lines).

- [ ] **Step 4: Run the tests, verify the bit order**

Run: `cargo test -p app bitfont`
Expected: PASS. If `glyph_sets_some_fg_pixels` or `scales_up_to_fill_cell` fail, the `font8x8` bit convention differs — try `g[row] & (1 << (7 - col))` (horizontal flip) and/or swap the row index. Adjust the single shift/index line until an asymmetric glyph like `'F'` renders upright (add a temporary debug print of the 8×8 grid if needed, then remove it). Do NOT change the tests.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/bitfont.rs crates/app/src/render/mod.rs crates/app/Cargo.toml Cargo.lock
git commit -m "feat(app): embedded 8x8 bitmap font + blit_glyph (v6 Phase 1c)

Quest: SQ-0186
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh"
```

---

### Task 2: Game-pixel rects on `PositionedWindow`

**Files:**
- Modify: `crates/app/src/engine.rs` (`PositionedWindow` struct, ~line 236)
- Modify: `crates/app/src/session.rs` (`v6_screen_model`, ~line 631)
- Modify: `crates/app/src/render/screen.rs` (any `PositionedWindow { .. }` literals in tests, ~line 1959)

**Interfaces:**
- Consumes: existing `PositionedWindow { x, y, w, h, node }`.
- Produces: `PositionedWindow { x, y, w, h, x_px, y_px, w_px, h_px, node }` where the `_px` fields are game-pixel coords (font cell = 8 px), for Task 3.

- [ ] **Step 1: Write the failing test**

Add to `crates/app/tests/zork0_v6_windows.rs` (or the module holding the Layered assertion). If a helper builds the model from a booted Zork Zero, assert the pixel fields are populated and preserve sub-cell offsets:

```rust
#[test]
fn v6_positioned_windows_carry_game_pixel_rects() {
    // Boot Zork Zero far enough to open its windows (reuse this file's harness).
    let model = /* existing helper that returns the v6 ScreenModel */;
    let items = match &model.root {
        lanthorn::engine::WinNode::Layered(items) => items,
        other => panic!("expected Layered, got {other:?}"),
    };
    assert!(!items.is_empty(), "v6 model has positioned windows");
    // Every item's pixel rect is consistent with its cell rect at 8 px/cell
    // (cell = px / 8), and pixel size is nonzero for a live window.
    for it in items {
        assert_eq!(it.x, it.x_px / 8, "cell x derived from px x");
        assert_eq!(it.y, it.y_px / 8, "cell y derived from px y");
        assert!(it.w_px > 0 && it.h_px > 0, "live window has nonzero pixel size");
    }
}
```

(If the test file lacks a reusable model helper, add a minimal one mirroring the existing boot-and-build-model setup already used by the Layered assertion in this file.)

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app --test zork0_v6_windows v6_positioned_windows_carry_game_pixel_rects`
Expected: FAIL (no fields `x_px`/`y_px`/`w_px`/`h_px`).

- [ ] **Step 3: Add the fields to `PositionedWindow`**

In `crates/app/src/engine.rs`, extend the struct (keep existing doc comment; add the pixel fields):

```rust
pub struct PositionedWindow {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    /// Game-pixel origin/size (font cell = 8 px) for the Phase 1c pixel
    /// composite. `x`/`y`/`w`/`h` above are the cell-quantized rect used by the
    /// Phase 1b fallback; these preserve the sub-cell offset it discards.
    pub x_px: u16,
    pub y_px: u16,
    pub w_px: u16,
    pub h_px: u16,
    pub node: WinNode,
}
```

- [ ] **Step 4: Populate them in `v6_screen_model`**

In `crates/app/src/session.rs`, inside the per-window loop, the raw pixel coords are already read as `win.x_coord`, `win.y_coord`, `win.x_size`, `win.y_size`. Set both the graphics-entry and text-entry `PositionedWindow` literals to include:

```rust
x_px: win.x_coord,
y_px: win.y_coord,
w_px: win.x_size,
h_px: win.y_size,
```

(The existing `x`/`y` remain `win.x_coord / V6_FONT_WIDTH` etc.; `w`/`h` remain `cols`/`rows`.)

- [ ] **Step 5: Fix any other `PositionedWindow` literals**

Run: `cargo build -p app --tests 2>&1 | grep -n "missing field" | head`
Update each flagged literal (e.g. the `background`/`foreground` test builders near `crates/app/src/render/screen.rs:1959`) to add the four `_px` fields. For test fixtures, mirror the cell values scaled by 8 (e.g. `x_px: x * 8`, `w_px: w * 8`) so they stay self-consistent.

- [ ] **Step 6: Run tests**

Run: `cargo test -p app`
Expected: PASS (new test + all existing Phase 1b tests unchanged).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/engine.rs crates/app/src/session.rs crates/app/src/render/screen.rs crates/app/tests/zork0_v6_windows.rs
git commit -m "feat(app): carry game-pixel rects on PositionedWindow (v6 Phase 1c)

Quest: SQ-0186
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh"
```

---

### Task 3: The pixel compositor `build_v6_canvas`

**Files:**
- Create: `crates/app/src/render/v6_canvas.rs`
- Modify: `crates/app/src/render/mod.rs` (add `pub mod v6_canvas;`)

**Interfaces:**
- Consumes: `PositionedWindow` (Task 2), `WinNode::{Graphics, Grid, Buffer}`, `blit_glyph` (Task 1), `crate::colors::ColorScheme`, `crate::render::resolve_zcolour`.
- Produces:
  - `pub struct MainText { pub lines: Vec<String>, pub input: String, pub cursor_col: u16 }`
  - `pub fn build_v6_canvas(items: &[PositionedWindow], pane_cells: (u16, u16), cell_px: (u16, u16), bg: image::Rgba<u8>, default_fg: image::Rgba<u8>, main: &MainText, colors: &crate::colors::ColorScheme) -> image::RgbaImage`

- [ ] **Step 1: Write the module with failing tests**

Create `crates/app/src/render/v6_canvas.rs`:

```rust
//! Phase 1c pixel compositor: build one device-resolution RGBA canvas for the
//! whole v6 story pane — graphics blitted at exact pixel coords, all text
//! (upper windows + main scrolling window + input line) rasterized via the
//! embedded bitmap font. `render_node`'s `Layered` arm draws the result as one
//! terminal image when an image protocol is available; otherwise it falls back
//! to the Phase 1b cell composite. See
//! docs/superpowers/specs/2026-07-22-v6-phase1c-pixel-render-design.md.

use image::{Rgba, RgbaImage};

use crate::colors::ColorScheme;
use crate::engine::{PositionedWindow, WinNode};
use crate::render::bitfont::blit_glyph;

/// Window-0 (main scrolling window) content the compositor rasterizes: the
/// visible wrapped lines (oldest-first, top-to-bottom), the live input line, and
/// the caret column within the input line.
#[derive(Debug, Default, Clone)]
pub struct MainText {
    pub lines: Vec<String>,
    pub input: String,
    pub cursor_col: u16,
}

/// Resolve a packed z-colour (see `crate::state::pack_zcolour`) to an opaque
/// RGBA. `0` (Default) → `fallback`. True24 → its RGB. Palette/standard colours
/// resolve through the theme; anything that doesn't reduce to a concrete RGB
/// falls back (v1 — richer palette handling is SQ-0450).
fn packed_to_rgba(packed: u32, fallback: Rgba<u8>, colors: &ColorScheme) -> Rgba<u8> {
    if packed == 0 {
        return fallback;
    }
    let tag = packed >> 24;
    if tag == 3 {
        let v = packed & 0x00FF_FFFF;
        return Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]);
    }
    // Standard(n)=tag 1, True(v)=tag 2 → reconstruct the ZColour and resolve via
    // the scheme; use the concrete RGB when the theme yields one, else fallback.
    let z = match tag {
        1 => zvm::screen::ZColour::Standard((packed & 0xFF) as u8),
        2 => zvm::screen::ZColour::True((packed & 0xFFFF) as u16),
        _ => return fallback,
    };
    match crate::render::resolve_zcolour(z, colors) {
        ratatui::style::Color::Rgb(r, g, b) => Rgba([r, g, b, 255]),
        _ => fallback,
    }
}

/// Blit a game-pixel source canvas into `dst` at device rect
/// `(dx, dy, dw, dh)`, nearest-neighbour, honouring source alpha (transparent
/// source px leave `dst`). Clipped to `dst` bounds.
fn blit_scaled(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, dw: u32, dh: u32) {
    let (sw, sh) = (src.width(), src.height());
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return;
    }
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..dh {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        let sy = (oy * sh / dh).min(sh - 1);
        for ox in 0..dw {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let sx = (ox * sw / dw).min(sw - 1);
            let p = *src.get_pixel(sx, sy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

pub fn build_v6_canvas(
    items: &[PositionedWindow],
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
    bg: Rgba<u8>,
    default_fg: Rgba<u8>,
    main: &MainText,
    colors: &ColorScheme,
) -> RgbaImage {
    let (cols, rows) = (pane_cells.0 as u32, pane_cells.1 as u32);
    let (cw, ch) = (cell_px.0 as u32, cell_px.1 as u32);
    let w = (cols * cw).max(1);
    let h = (rows * ch).max(1);
    let mut canvas = RgbaImage::from_pixel(w, h, bg);

    // game px → device px: multiply by cellpx / 8 (font cell = 8 game px).
    let gx = |g: u16| g as u32 * cw / 8;
    let gy = |g: u16| g as u32 * ch / 8;

    // Layers in list order: graphics (background) first, then text on top.
    for it in items {
        match &it.node {
            WinNode::Graphics(gwn) => {
                blit_scaled(
                    &mut canvas,
                    &gwn.canvas,
                    gx(it.x_px),
                    gy(it.y_px),
                    gx(it.w_px).max(1),
                    gy(it.h_px).max(1),
                );
            }
            WinNode::Grid(g) => {
                let ox = gx(it.x_px);
                let oy = gy(it.y_px);
                for row in 0..g.rows {
                    for col in 0..g.cols {
                        let idx = row as usize * g.cols as usize + col as usize;
                        let Some(cell) = g.cells.get(idx) else { continue };
                        if cell.ch == '\0' || cell.ch == ' ' {
                            // Blank: paint the cell bg only if the game set one,
                            // else leave the background/graphics showing.
                            if cell.bg != 0 {
                                let b = packed_to_rgba(cell.bg, bg, colors);
                                fill_cell(&mut canvas, ox + col as u32 * cw, oy + row as u32 * ch, cw, ch, b);
                            }
                            continue;
                        }
                        let fg = packed_to_rgba(cell.fg, default_fg, colors);
                        let cellbg = (cell.bg != 0).then(|| packed_to_rgba(cell.bg, bg, colors));
                        blit_glyph(&mut canvas, cell.ch, ox + col as u32 * cw, oy + row as u32 * ch, cw, ch, fg, cellbg);
                    }
                }
            }
            WinNode::Buffer(b) if b.primary => {
                // Main scrolling window: rasterize visible lines + input line,
                // transparent bg (the background window shows through gaps).
                let ox = gx(it.x_px);
                let oy = gy(it.y_px);
                let win_rows = it.h_px as u32 * ch / 8 / ch; // = rows spanned
                draw_text_block(&mut canvas, &main.lines, &main.input, main.cursor_col, ox, oy, cw, ch, win_rows, default_fg);
            }
            _ => {}
        }
    }
    canvas
}

fn fill_cell(canvas: &mut RgbaImage, px: u32, py: u32, cw: u32, ch: u32, color: Rgba<u8>) {
    let (w, h) = (canvas.width(), canvas.height());
    for y in py..(py + ch).min(h) {
        for x in px..(px + cw).min(w) {
            canvas.put_pixel(x, y, color);
        }
    }
}

/// Rasterize a block of text lines (then the input line + block cursor) into the
/// canvas at device origin `(ox, oy)`, one line per `ch` rows, transparent bg.
#[allow(clippy::too_many_arguments)]
fn draw_text_block(
    canvas: &mut RgbaImage,
    lines: &[String],
    input: &str,
    cursor_col: u16,
    ox: u32,
    oy: u32,
    cw: u32,
    ch: u32,
    max_rows: u32,
    fg: Rgba<u8>,
) {
    let mut row = 0u32;
    for line in lines {
        if row >= max_rows {
            return;
        }
        for (col, glyph) in line.chars().enumerate() {
            blit_glyph(canvas, glyph, ox + col as u32 * cw, oy + row * ch, cw, ch, fg, None);
        }
        row += 1;
    }
    if row < max_rows {
        // Input line on the next row.
        for (col, glyph) in input.chars().enumerate() {
            blit_glyph(canvas, glyph, ox + col as u32 * cw, oy + row * ch, cw, ch, fg, None);
        }
        // Block cursor at the caret column.
        fill_cell(canvas, ox + cursor_col as u32 * cw, oy + row * ch, cw, ch, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{BufferWindow, GraphicsWindow, GridCell, GridWindow, WinNode};
    use std::sync::Arc;

    fn colors() -> ColorScheme {
        ColorScheme::default()
    }

    fn grid_item(x_px: u16, y_px: u16, cols: u16, rows: u16, cells: Vec<GridCell>) -> PositionedWindow {
        PositionedWindow {
            x: x_px / 8, y: y_px / 8, w: cols, h: rows,
            x_px, y_px, w_px: cols * 8, h_px: rows * 8,
            node: WinNode::Grid(GridWindow {
                cols, rows, cells, active_rows: rows, cursor: (0, 0), cursor_active: false,
                border: crate::engine::BorderPref::Unspecified, bg: None, fg: None, reverse: false,
            }),
        }
    }

    fn blank_cell() -> GridCell {
        GridCell { ch: ' ', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }
    }

    #[test]
    fn fills_background() {
        let bg = Rgba([10, 20, 30, 255]);
        let c = build_v6_canvas(&[], (4, 3), (8, 16), bg, Rgba([255; 4]), &MainText::default(), &colors());
        assert_eq!(c.dimensions(), (32, 48));
        assert_eq!(*c.get_pixel(0, 0), bg);
        assert_eq!(*c.get_pixel(31, 47), bg);
    }

    #[test]
    fn graphics_blits_at_sub_cell_pixel_offset() {
        // A 8×8 solid-red game-px canvas at game x=4 (half a cell) lands at
        // device x = 4*8/8 = 4 — NOT snapped to a cell boundary.
        let src = RgbaImage::from_pixel(8, 8, Rgba([200, 0, 0, 255]));
        let item = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 4, y_px: 0, w_px: 8, h_px: 8,
            node: WinNode::Graphics(GraphicsWindow { win: 7, canvas: Arc::new(src), version: 1, upscale: false }),
        };
        let c = build_v6_canvas(&[item], (4, 2), (8, 8), Rgba([0, 0, 0, 255]), Rgba([255; 4]), &MainText::default(), &colors());
        assert_eq!(*c.get_pixel(4, 0), Rgba([200, 0, 0, 255]), "red starts at device x=4");
        assert_eq!(*c.get_pixel(3, 0), Rgba([0, 0, 0, 255]), "device x=3 still background");
    }

    #[test]
    fn grid_glyph_paints_fg_in_its_cell_and_leaves_neighbours() {
        let mut cells = vec![blank_cell(); 2];
        cells[0] = GridCell { ch: 'A', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let item = grid_item(0, 0, 2, 1, cells);
        let fg = Rgba([0, 255, 0, 255]);
        let c = build_v6_canvas(&[item], (2, 1), (8, 8), Rgba([0, 0, 0, 255]), fg, &MainText::default(), &colors());
        // Cell 0 has fg pixels; cell 1 (a space, bg=0) stays background.
        assert!((0..8).any(|y| (0..8).any(|x| *c.get_pixel(x, y) == fg)), "A rendered in cell 0");
        assert!((0..8).all(|y| (8..16).all(|x| *c.get_pixel(x, y) == Rgba([0, 0, 0, 255]))), "cell 1 untouched");
    }

    #[test]
    fn main_window_rasterizes_text_and_cursor() {
        let item = PositionedWindow {
            x: 0, y: 0, w: 6, h: 3, x_px: 0, y_px: 0, w_px: 48, h_px: 24,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let main = MainText { lines: vec!["hi".into()], input: "go".into(), cursor_col: 2 };
        let fg = Rgba([255, 255, 0, 255]);
        let c = build_v6_canvas(&[item], (6, 3), (8, 8), Rgba([0, 0, 0, 255]), fg, &main, &colors());
        // Line 0 has glyph pixels; the cursor block sits on row 1 at col 2.
        assert!((0..8).any(|y| (0..16).any(|x| *c.get_pixel(x, y) == fg)), "line 0 text drawn");
        assert_eq!(*c.get_pixel(16, 8), fg, "cursor block at row 1 col 2");
    }
}
```

- [ ] **Step 2: Add `pub mod v6_canvas;` to `crates/app/src/render/mod.rs`.**

- [ ] **Step 3: Run the tests**

Run: `cargo test -p app v6_canvas`
Expected: PASS. If a field name in the `GridWindow`/`GridCell`/`BufferWindow` literals mismatches, fix the literal to match the real struct (do not change assertions). `win_rows` in the `Buffer` arm simplifies to the window's row span; if clippy flags the redundant arithmetic, replace with `it.h_px as u32 / 8`.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/render/v6_canvas.rs crates/app/src/render/mod.rs
git commit -m "feat(app): v6 pixel compositor build_v6_canvas (Phase 1c)

Quest: SQ-0186
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh"
```

---

### Task 4: Single-image renderer + `Layered` arm hook

**Files:**
- Modify: `crates/app/src/render/graphics.rs` (add a cached single-image draw to `GraphicsRender`)
- Modify: `crates/app/src/render/screen.rs` (`Layered` arm: picker → image path)

**Interfaces:**
- Consumes: `build_v6_canvas`, `MainText` (Task 3), `state.game_picker`, `state.graphics_render`, `state.transcript`, `state.input`, `crate::render::transcript::wrap_line`.
- Produces: `GraphicsRender::draw_v6_canvas(&mut self, picker: &Picker, canvas: &image::RgbaImage, area: Rect, buf: &mut Buffer)`.

- [ ] **Step 1: Add the cached single-image draw to `GraphicsRender`**

In `crates/app/src/render/graphics.rs`, add a field to the struct and a method. Change:

```rust
#[derive(Default)]
pub struct GraphicsRender {
    cache: std::collections::HashMap<u32, (u64, u16, u16, Protocol)>,
}
```

to add a v6 single-image slot:

```rust
#[derive(Default)]
pub struct GraphicsRender {
    cache: std::collections::HashMap<u32, (u64, u16, u16, Protocol)>,
    /// One-image cache for the v6 pixel composite (Phase 1c), keyed on a content
    /// hash + area so unchanged frames reuse the uploaded protocol.
    v6: Option<(u64, u16, u16, Protocol)>,
}
```

Add the method (below `render`):

```rust
/// Draw a pre-composited v6 canvas as ONE terminal image filling `area`
/// (canvas is sized to the pane's device pixels, so it fits at native size —
/// no letterbox). Cached on a content hash so identical frames don't
/// re-encode/upload.
pub fn draw_v6_canvas(&mut self, picker: &Picker, canvas: &image::RgbaImage, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canvas.as_raw().hash(&mut h);
    let hash = h.finish();
    let fresh = matches!(&self.v6, Some((v, w, ht, _)) if *v == hash && *w == area.width && *ht == area.height);
    if !fresh {
        let img = image::DynamicImage::ImageRgba8(canvas.clone());
        match picker.new_protocol(img, Size::new(area.width, area.height), Resize::Fit(None)) {
            Ok(p) => self.v6 = Some((hash, area.width, area.height, p)),
            Err(_) => return,
        }
    }
    if let Some((_, _, _, proto)) = &self.v6 {
        let sz = proto.size();
        let w = sz.width.min(area.width);
        let ht = sz.height.min(area.height);
        let dest = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - ht) / 2, w, ht);
        Image::new(proto).render(dest, buf);
    }
}
```

- [ ] **Step 2: Write a unit test for the image renderer**

Add to the `tests` module in `graphics.rs`:

```rust
#[test]
fn draw_v6_canvas_caches_on_content_hash() {
    let picker = Picker::halfblocks();
    let mut gr = GraphicsRender::default();
    let area = Rect::new(0, 0, 4, 2);
    let mut buf = Buffer::empty(area);
    let canvas = image::RgbaImage::from_pixel(32, 32, image::Rgba([1, 2, 3, 255]));
    gr.draw_v6_canvas(&picker, &canvas, area, &mut buf);
    assert!(gr.v6.is_some(), "first draw builds + caches the protocol");
    let (hash0, _, _, _) = gr.v6.as_ref().unwrap();
    let hash0 = *hash0;
    // Same content → same hash (no rebuild churn on identical frames).
    gr.draw_v6_canvas(&picker, &canvas, area, &mut buf);
    assert_eq!(gr.v6.as_ref().unwrap().0, hash0, "identical canvas keeps the cached entry");
}
```

- [ ] **Step 3: Run it**

Run: `cargo test -p app -- graphics::tests::draw_v6_canvas`
Expected: PASS.

- [ ] **Step 4: Hook the `Layered` arm in `screen.rs`**

In `render_node`'s `WinNode::Layered(items)` arm (~line 392), branch on the picker BEFORE the existing per-item cell loop:

```rust
WinNode::Layered(items) => {
    // Phase 1c: with an image protocol, composite the whole v6 pane as one
    // device-resolution RGBA canvas and draw it as a single terminal image
    // (graphics at exact pixel coords, all text rasterized). Without a
    // picker, fall through to the Phase 1b cell composite below.
    if let Some(picker) = state.game_picker.as_ref() {
        let f = picker.font_size();
        let cell_px = (f.width, f.height);
        let pane_cells = (area.width, area.height);
        let bg = rgba_from_packed(model_bg_of(state), state); // see helper below
        let default_fg = theme_fg_rgba(state);
        let main = build_main_text(state, items, area);
        let canvas = crate::render::v6_canvas::build_v6_canvas(
            items, pane_cells, cell_px, bg, default_fg, &main, &state.colors,
        );
        state.graphics_render.borrow_mut().draw_v6_canvas(picker, &canvas, area, buf);
        return None; // v6 main-window scroll metrics are a follow-up (SQ-0450)
    }
    // …existing Phase 1b per-item cell loop unchanged…
}
```

Add these small helpers near the bottom of `screen.rs` (module-private):

```rust
/// Resolve a themed style's colour to an opaque RGBA for the pixel canvas.
fn style_bg_rgba(style: ratatui::style::Style, fallback: image::Rgba<u8>) -> image::Rgba<u8> {
    match style.bg {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => fallback,
    }
}
fn style_fg_rgba(style: ratatui::style::Style, fallback: image::Rgba<u8>) -> image::Rgba<u8> {
    match style.fg {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => fallback,
    }
}

/// Build the main-window text block for the pixel composite: the newest visible
/// wrapped transcript lines that fit the primary window's rows, plus the live
/// input line and caret column.
fn build_main_text(state: &AppState, items: &[PositionedWindow], _area: Rect) -> crate::render::v6_canvas::MainText {
    use crate::render::transcript::wrap_line;
    // Primary window's cell size (cols/rows) drives wrapping + row budget.
    let prim = items.iter().find(|it| matches!(&it.node, WinNode::Buffer(b) if b.primary));
    let (cols, rows) = prim.map(|it| (it.w.max(1), it.h.max(1))).unwrap_or((1, 1));
    // Wrap all transcript lines to the window width, keep the newest `rows-1`
    // wrapped rows (leave one row for the input line).
    let mut wrapped: Vec<String> = Vec::new();
    for line in &state.transcript {
        wrapped.extend(wrap_line(line, cols));
    }
    let budget = rows.saturating_sub(1) as usize;
    let start = wrapped.len().saturating_sub(budget);
    let lines = wrapped[start..].to_vec();
    let input = state.input.value.clone();
    let cursor_col = input.chars().count().min(cols as usize - 1) as u16;
    crate::render::v6_canvas::MainText { lines, input, cursor_col }
}
```

For `bg`/`default_fg`, resolve from the model + theme at the call site:

```rust
let theme_bg = state.colors.theme.get("transcript").style;
let bg = style_bg_rgba(theme_bg, image::Rgba([0, 0, 0, 255]));
let default_fg = style_fg_rgba(theme_bg, image::Rgba([220, 220, 220, 255]));
```

(Replace the `model_bg_of`/`rgba_from_packed`/`theme_fg_rgba` placeholders in the arm sketch with these two lines — they are the concrete implementation. Use `model.bg` only if it is already resolved to RGB; the theme transcript style is the reliable source.)

- [ ] **Step 5: Verify the arm compiles and the field/method names match**

Run: `cargo build -p app --tests 2>&1 | tail -20`
Fix any name mismatches (`state.input.value`, `state.transcript`, `it.w`/`it.h`, `state.colors.theme.get`) against the real definitions. Confirm `wrap_line` is `pub(crate)` and reachable; if not, use the module-visible wrapper already used by `render_inline_buffer`.

- [ ] **Step 6: Full test run**

Run: `cargo test -p app`
Expected: PASS (existing Phase 1b tests still green — they run without a picker, so they hit the fallback path).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/render/graphics.rs crates/app/src/render/screen.rs
git commit -m "feat(app): draw v6 pane as one pixel image when a picker exists (Phase 1c)

Quest: SQ-0186
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh"
```

---

### Task 5: Integration smoke, docs, quest status

**Files:**
- Modify: `crates/app/tests/zork0_v6_windows.rs` (smoke: pixel path builds a non-empty canvas)
- Modify: docs (features / architecture / README as they cover graphics) — accurate + engaging, major-feature level only
- Quest: `Confirm: SQ-0186`

- [ ] **Step 1: Write the integration smoke**

Add to `crates/app/tests/zork0_v6_windows.rs` a test that boots Zork Zero, builds the v6 `ScreenModel`, then calls `build_v6_canvas` directly with a synthetic `MainText` and a fixed cell size, asserting the canvas is the expected device size and is not uniformly the background (some window painted into it):

```rust
#[test]
fn zork0_v6_pixel_canvas_is_nonempty() {
    let model = /* existing helper → v6 ScreenModel after boot */;
    let items = match &model.root {
        lanthorn::engine::WinNode::Layered(v) => v,
        _ => panic!("expected Layered"),
    };
    let bg = image::Rgba([0, 0, 0, 255]);
    let main = lanthorn::render::v6_canvas::MainText::default();
    let colors = lanthorn::colors::ColorScheme::default();
    let canvas = lanthorn::render::v6_canvas::build_v6_canvas(
        items, (80, 24), (8, 8), bg, image::Rgba([255; 4]), &main, &colors,
    );
    assert_eq!(canvas.dimensions(), (640, 192));
    // At least one window painted something other than the flat background.
    assert!(canvas.pixels().any(|p| *p != bg), "v6 pixel canvas has painted content");
}
```

(Reuse the file's existing boot harness; if items are all empty at the boot point the test uses, advance the VM a few more turns first — mirror however the existing Layered assertion reaches a populated model. Make the crate items reachable: ensure `pub mod v6_canvas;`/`pub mod bitfont;` are exported through `crate::render` and that `render` is `pub`.)

- [ ] **Step 2: Run it**

Run: `cargo test -p app --test zork0_v6_windows zork0_v6_pixel_canvas_is_nonempty`
Expected: PASS.

- [ ] **Step 3: Full workspace test**

Run: `cargo test`
Expected: PASS across the workspace.

- [ ] **Step 4: Update docs**

Update the user-facing docs that describe graphics/v6 support (per the repo's docs set — features page, architecture, and README graphics section). Describe v6 graphical support as: v6 stories now boot and render with pictures + text composited as a single pixel image on image-capable terminals (kitty/sixel/iterm), falling back to a cell-grid rendering elsewhere. Keep it accurate (don't claim menus/mouse — those are Phase 2) and match the README's lively tone. Major-feature level only; no per-fix noise.

- [ ] **Step 5: Commit docs + tests**

```bash
git add crates/app/tests/zork0_v6_windows.rs <the doc files you edited>
git commit -m "test+docs: v6 pixel-composite smoke + graphics docs (Phase 1c)

Quest: SQ-0186
Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01BseFHPDHxDQrvSQRa4Whsh"
```

- [ ] **Step 6: Mark the quest awaiting visual confirmation**

The pixel composite's fidelity can only be judged in a real image-capable terminal, so set SQ-0186 to `confirm` (not `done`) via the on-PATH CLI: `side-quest status SQ-0186 confirm` (or a `Confirm: SQ-0186` trailer, already on the Task 5 commit). Relay any flavor line verbatim.

---

## Self-Review

- **Spec coverage:** font (T1) ✓, pixel-coord model (T2) ✓, compositor incl. graphics/upper-text/main-text (T3) ✓, one-image render + picker gate + fallback (T4) ✓, tests + docs + confirm (T5) ✓. Coordinate transform (device = game·cellpx/8) implemented in `build_v6_canvas`/`blit_scaled`. ✓
- **Placeholder scan:** the T4 arm sketch uses named placeholders (`model_bg_of`, etc.) that Step 4 immediately replaces with concrete two-line resolution — flagged inline, not left dangling.
- **Type consistency:** `build_v6_canvas` signature identical in spec, T3, T4 call, T5 smoke. `MainText { lines, input, cursor_col }` consistent. `blit_glyph` signature identical in T1 and T3 use. `draw_v6_canvas` identical in T4 def + call.
- **Known blind-build risks (call out at final review):** `font8x8` bit order (T1 Step 4 verifies empirically); exact `GridWindow`/`GridCell`/`BufferWindow` field names in T3 test literals (fix-to-compile, don't touch assertions); `wrap_line` visibility (T4 Step 5). None affect the non-v6 paths.
```
