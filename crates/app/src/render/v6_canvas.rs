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
    /// True when the game is awaiting a typed command (host focus is on the
    /// game). Only then is the input line + block cursor rasterized — otherwise
    /// a title/keypress screen would show a stray caret block.
    pub awaiting: bool,
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

/// The v6 font cell size in game pixels — matches `zvm::screen::V6_FONT_WIDTH`/
/// `V6_FONT_HEIGHT` and the `write_screen_dims` v6 arm. Text is one glyph per
/// `FONT × FONT` game-pixel cell; the whole canvas is later scaled to the pane.
const FONT: u32 = 8;

/// Build the v6 story pane as one RGBA canvas in the game's **native pixel
/// space** (the resolution advertised to the game — the Blorb `Reso` standard
/// window, e.g. 320×200). Graphics are blitted at their exact pixel coords and
/// all text is rasterized at the native 8px font cell, so windows and hardcoded
/// pixel art share one coordinate system. The render layer scales this whole
/// canvas to fill the story pane (SQ-0186 Phase 1c).
pub fn build_v6_canvas(
    items: &[PositionedWindow],
    bg: Rgba<u8>,
    default_fg: Rgba<u8>,
    main: &MainText,
    colors: &ColorScheme,
) -> RgbaImage {
    // Canvas = the native screen extent (max window bottom-right). Window 7 is
    // typically the full-screen background, so this is the advertised size.
    let mut sw = 1u32;
    let mut sh = 1u32;
    for it in items {
        sw = sw.max(it.x_px as u32 + it.w_px as u32);
        sh = sh.max(it.y_px as u32 + it.h_px as u32);
    }
    let mut canvas = RgbaImage::from_pixel(sw, sh, bg);

    // Layers in list order: graphics (background) first, then text on top.
    for it in items {
        let ox = it.x_px as u32;
        let oy = it.y_px as u32;
        let lm = it.left_margin as u32; // set_margins text inset
        match &it.node {
            WinNode::Graphics(gwn) => {
                // 1:1 blit — the window canvas is already native game pixels.
                blit_scaled(&mut canvas, &gwn.canvas, ox, oy, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
            }
            WinNode::Grid(g) => {
                // Clamp to the window's pixel height: a v6 grid's row count can
                // exceed its pixel box (e.g. Zork0's window 1 is a 40×11 grid in
                // a 5px-tall window). Drawing all rows spills status text over the
                // main window — render only the rows that fit.
                let max_rows = (it.h_px as u32 / FONT).min(g.rows as u32) as u16;
                for row in 0..max_rows {
                    for col in 0..g.cols {
                        let idx = row as usize * g.cols as usize + col as usize;
                        let Some(cell) = g.cells.get(idx) else { continue };
                        let px = ox + lm + col as u32 * FONT;
                        let py = oy + row as u32 * FONT;
                        if cell.ch == '\0' || cell.ch == ' ' {
                            // Blank: paint the cell bg only if the game set one,
                            // else leave the background/graphics showing.
                            if cell.bg != 0 {
                                let b = packed_to_rgba(cell.bg, bg, colors);
                                fill_cell(&mut canvas, px, py, FONT, FONT, b);
                            }
                            continue;
                        }
                        let fg = packed_to_rgba(cell.fg, default_fg, colors);
                        let cellbg = (cell.bg != 0).then(|| packed_to_rgba(cell.bg, bg, colors));
                        blit_glyph(&mut canvas, cell.ch, px, py, FONT, FONT, fg, cellbg);
                    }
                }
            }
            WinNode::Buffer(b) if b.primary => {
                // Main scrolling window: rasterize visible lines + input line,
                // inset by the game's left margin (so text clears a border
                // frame), transparent bg (the background shows through gaps).
                let win_rows = it.h_px as u32 / FONT;
                draw_text_block(&mut canvas, &main.lines, &main.input, main.cursor_col, main.awaiting, ox + lm, oy, FONT, FONT, win_rows, default_fg);
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
    awaiting: bool,
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
    // Draw the input line + block cursor only while the game is awaiting a typed
    // command — otherwise a title/keypress screen shows a stray caret block.
    if awaiting && row < max_rows {
        for (col, glyph) in input.chars().enumerate() {
            blit_glyph(canvas, glyph, ox + col as u32 * cw, oy + row * ch, cw, ch, fg, None);
        }
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
            x_px, y_px, w_px: cols * 8, h_px: rows * 8, left_margin: 0, right_margin: 0,
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
    fn canvas_sized_to_native_window_extent_and_bg_filled() {
        // Canvas is the native pixel extent (max window bottom-right), not the
        // pane's device size. A 4×3-cell blank grid → 32×24 game px, all bg.
        let bg = Rgba([10, 20, 30, 255]);
        let item = grid_item(0, 0, 4, 3, vec![blank_cell(); 12]);
        let c = build_v6_canvas(&[item], bg, Rgba([255; 4]), &MainText::default(), &colors());
        assert_eq!(c.dimensions(), (32, 24));
        assert_eq!(*c.get_pixel(0, 0), bg);
        assert_eq!(*c.get_pixel(31, 23), bg);
    }

    #[test]
    fn graphics_blits_at_native_pixel_offset() {
        // A 8×8 solid-red canvas at game x=4 lands at canvas x=4, 1:1 (native
        // space — no cell quantization, no scaling).
        let src = RgbaImage::from_pixel(8, 8, Rgba([200, 0, 0, 255]));
        let item = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 4, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 7, canvas: Arc::new(src), version: 1, upscale: false }),
        };
        let c = build_v6_canvas(&[item], Rgba([0, 0, 0, 255]), Rgba([255; 4]), &MainText::default(), &colors());
        assert_eq!(*c.get_pixel(4, 0), Rgba([200, 0, 0, 255]), "red starts at canvas x=4");
        assert_eq!(*c.get_pixel(3, 0), Rgba([0, 0, 0, 255]), "x=3 still background");
    }

    #[test]
    fn grid_glyph_paints_fg_in_its_cell_and_leaves_neighbours() {
        let mut cells = vec![blank_cell(); 2];
        cells[0] = GridCell { ch: 'A', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let item = grid_item(0, 0, 2, 1, cells);
        let fg = Rgba([0, 255, 0, 255]);
        let c = build_v6_canvas(&[item], Rgba([0, 0, 0, 255]), fg, &MainText::default(), &colors());
        // Cell 0 has fg pixels; cell 1 (a space, bg=0) stays background.
        assert!((0..8).any(|y| (0..8).any(|x| *c.get_pixel(x, y) == fg)), "A rendered in cell 0");
        assert!((0..8).all(|y| (8..16).all(|x| *c.get_pixel(x, y) == Rgba([0, 0, 0, 255]))), "cell 1 untouched");
    }

    #[test]
    fn main_window_rasterizes_text_and_cursor() {
        let item = PositionedWindow {
            x: 0, y: 0, w: 6, h: 3, x_px: 0, y_px: 0, w_px: 48, h_px: 24, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let main = MainText { lines: vec!["hi".into()], input: "go".into(), cursor_col: 2, awaiting: true };
        let fg = Rgba([255, 255, 0, 255]);
        let c = build_v6_canvas(&[item], Rgba([0, 0, 0, 255]), fg, &main, &colors());
        // Line 0 has glyph pixels; the cursor block sits on row 1 at col 2 (x=16).
        assert!((0..8).any(|y| (0..16).any(|x| *c.get_pixel(x, y) == fg)), "line 0 text drawn");
        assert_eq!(*c.get_pixel(16, 8), fg, "cursor block at row 1 col 2");
    }

    #[test]
    fn main_window_text_is_inset_by_left_margin() {
        // set_margins(left=16) shifts window-0 text right by 16 px, so the caret
        // block for an empty input sits at x=16 (not x=0) — the fix that keeps
        // text inside a border frame.
        let item = PositionedWindow {
            x: 0, y: 0, w: 8, h: 2, x_px: 0, y_px: 0, w_px: 64, h_px: 16,
            left_margin: 16, right_margin: 8,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let main = MainText { lines: vec![], input: String::new(), cursor_col: 0, awaiting: true };
        let fg = Rgba([255, 0, 0, 255]);
        let c = build_v6_canvas(&[item], Rgba([0, 0, 0, 255]), fg, &main, &colors());
        assert_eq!(*c.get_pixel(16, 0), fg, "caret inset by the 16px left margin");
        assert_eq!(*c.get_pixel(0, 0), Rgba([0, 0, 0, 255]), "flush-left column stays background");
    }
}
