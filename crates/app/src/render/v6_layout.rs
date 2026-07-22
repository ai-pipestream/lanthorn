//! v6 layout classification: split the engine's flat window list into the
//! single scrolling story window (a primary `Buffer`) and everything else
//! (chrome — frame graphics, status grids, etc.). Pure classification, no
//! rendering (Phase 1a).

use image::{Rgba, RgbaImage};

use crate::colors::ColorScheme;
use crate::engine::{PositionedWindow, WinNode};

/// Resolve a packed z-colour (see `crate::state::pack_zcolour`) to an opaque
/// RGBA. `0` (Default) → `fallback`. True24 → its RGB. Palette/standard colours
/// resolve through the theme; anything that doesn't reduce to a concrete RGB
/// falls back (v1 — richer palette handling is SQ-0450).
pub(crate) fn packed_to_rgba(packed: u32, fallback: Rgba<u8>, colors: &ColorScheme) -> Rgba<u8> {
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
pub(crate) fn blit_scaled(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, dw: u32, dh: u32) {
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

/// The v6 font cell size in game pixels — matches `zvm::screen::V6_FONT_WIDTH`.
const FONT: u32 = 8;

/// The story (primary) window's rasterizable content: visible wrapped lines
/// (oldest-first), the live input line, and the caret column. `awaiting` gates
/// the input line + block cursor (drawn only when the game has host focus).
#[derive(Debug, Default, Clone)]
pub struct MainText {
    pub lines: Vec<String>,
    pub input: String,
    pub cursor_col: u16,
    pub awaiting: bool,
}

/// The native screen extent (max window bottom-right) in game pixels; min 1×1.
pub fn native_extent(items: &[PositionedWindow]) -> (u16, u16) {
    let mut w = 1u16;
    let mut h = 1u16;
    for it in items {
        w = w.max(it.x_px.saturating_add(it.w_px));
        h = h.max(it.y_px.saturating_add(it.h_px));
    }
    (w, h)
}

/// The v6 window list split into the one story window and the rest (chrome),
/// in input order.
pub struct V6Layout<'a> {
    pub story: Option<&'a PositionedWindow>,
    pub chrome: Vec<&'a PositionedWindow>,
}

/// Classify `items`: the first primary `Buffer` becomes `story`; every other
/// entry (in input order) goes into `chrome`. With no primary `Buffer`,
/// `story` is `None` and all entries are chrome.
pub fn classify_windows(items: &[PositionedWindow]) -> V6Layout<'_> {
    let mut story = None;
    let mut chrome = Vec::new();
    for pw in items {
        if story.is_none() && matches!(&pw.node, WinNode::Buffer(b) if b.primary) {
            story = Some(pw);
        } else {
            chrome.push(pw);
        }
    }
    V6Layout { story, chrome }
}

fn fill_cell(canvas: &mut RgbaImage, px: u32, py: u32, cw: u32, ch: u32, color: Rgba<u8>) {
    let (w, h) = (canvas.width(), canvas.height());
    for y in py..(py + ch).min(h) {
        for x in px..(px + cw).min(w) {
            canvas.put_pixel(x, y, color);
        }
    }
}

/// Build the CHROME image: one native-resolution RGBA canvas containing only
/// the frame graphics and status text (everything `classify_windows` put in
/// `chrome`). The story region and any gaps stay fully transparent — a later
/// task scales this canvas to the pane and layers it over the story text.
///
/// Two passes, in list order, frame graphics behind status text: Graphics
/// entries are blitted first (later entries draw over earlier ones only where
/// opaque, giving correct z-order for overlapping frame art like Zork Zero's
/// compass); Grid entries are rasterized second, one glyph per `FONT × FONT`
/// native-pixel cell, drawing every row regardless of the window's pixel
/// height (a v6 status grid can legitimately exceed its pixel box).
pub fn build_chrome_canvas(
    chrome: &[&PositionedWindow],
    native: (u16, u16),
    default_fg: Rgba<u8>,
    colors: &ColorScheme,
) -> RgbaImage {
    let mut canvas = RgbaImage::new(native.0 as u32, native.1 as u32);

    // Pass 1 — Graphics entries, in list order.
    for it in chrome {
        if let WinNode::Graphics(gwn) = &it.node {
            blit_scaled(&mut canvas, &gwn.canvas, it.x_px as u32, it.y_px as u32, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
        }
    }

    // Pass 2 — Grid (status) entries, in list order.
    for it in chrome {
        if let WinNode::Grid(g) = &it.node {
            let ox = it.x_px as u32;
            let oy = it.y_px as u32;
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let idx = row as usize * g.cols as usize + col as usize;
                    let Some(cell) = g.cells.get(idx) else { continue };
                    let px = ox + col as u32 * FONT;
                    let py = oy + row as u32 * FONT;
                    if cell.ch == '\0' || cell.ch == ' ' {
                        if cell.bg != 0 {
                            let b = packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors);
                            fill_cell(&mut canvas, px, py, FONT, FONT, b);
                        }
                        continue;
                    }
                    let fg = packed_to_rgba(cell.fg, default_fg, colors);
                    let cellbg = (cell.bg != 0).then(|| packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors));
                    crate::render::bitfont::blit_glyph(&mut canvas, cell.ch, px, py, FONT, FONT, fg, cellbg);
                }
            }
        }
    }

    canvas
}

/// A uniform (aspect-preserving) letterbox scale from native game pixels to
/// pane device pixels, plus the device-pixel offset of the letterboxed area.
pub struct Scale {
    pub s: f32,
    pub off_x: u32,
    pub off_y: u32,
}

/// Compute the uniform letterbox scale that fits `native` game-pixel
/// dimensions into `pane_dev` device-pixel dimensions, centering the result.
pub fn uniform_scale(native: (u16, u16), pane_dev: (u32, u32)) -> Scale {
    let nw = if native.0 == 0 { 1 } else { native.0 as u32 } as f32;
    let nh = if native.1 == 0 { 1 } else { native.1 as u32 } as f32;
    let s = (pane_dev.0 as f32 / nw).min(pane_dev.1 as f32 / nh);
    let scaled_w = nw * s;
    let scaled_h = nh * s;
    let off_x = ((pane_dev.0 as f32 - scaled_w) / 2.0).max(0.0) as u32;
    let off_y = ((pane_dev.1 as f32 - scaled_h) / 2.0).max(0.0) as u32;
    Scale { s, off_x, off_y }
}

/// The story window's clear-interior rect in NATIVE game pixels: its native rect
/// inset (interleaved per-edge) until no edge overlaps an opaque chrome pixel.
/// `None` when there is no story window. May be zero-size if fully occluded.
///
/// Inset one native pixel at a time per edge, banner first then columns, but
/// *interleaved* round by round (rather than each edge run to completion before
/// the next starts): a story window can overlap chrome on both axes at once
/// (e.g. a banner AND side columns), and letting the top/bottom scan run to
/// completion against the still-full width would never see a "clear" row while
/// side-band columns persist down the whole height. Shrinking left/right a step
/// at a time alongside top/bottom lets each edge's scan range narrow in
/// lockstep, converging on the true clear interior.
pub fn story_clear_native(
    story: Option<&PositionedWindow>,
    chrome_canvas: &RgbaImage,
) -> Option<(u32, u32, u32, u32)> {
    let story = story?;
    let (cw, ch) = chrome_canvas.dimensions();
    let opaque = |x: u32, y: u32| -> bool { x < cw && y < ch && chrome_canvas.get_pixel(x, y)[3] >= 128 };
    let mut left = story.x_px as u32;
    let mut top = story.y_px as u32;
    let mut right = (story.x_px as u32 + story.w_px as u32).min(cw);
    let mut bottom = (story.y_px as u32 + story.h_px as u32).min(ch);
    loop {
        let mut changed = false;
        if top < bottom && (left..right).any(|x| opaque(x, top)) {
            top += 1;
            changed = true;
        }
        if bottom > top && (left..right).any(|x| opaque(x, bottom - 1)) {
            bottom -= 1;
            changed = true;
        }
        if left < right && (top..bottom).any(|y| opaque(left, y)) {
            left += 1;
            changed = true;
        }
        if right > left && (top..bottom).any(|y| opaque(right - 1, y)) {
            right -= 1;
            changed = true;
        }
        if !changed {
            break;
        }
    }
    Some((left, top, right.saturating_sub(left), bottom.saturating_sub(top)))
}

/// The cell rect (relative to the pane's top-left cell) where story text
/// goes: the largest cell-aligned rect inside the story window's device rect
/// that touches no opaque chrome pixel. Falls back to the full pane when
/// there is no story window.
pub fn story_viewport(
    story: Option<&PositionedWindow>,
    chrome_canvas: &image::RgbaImage,
    scale: &Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> ratatui::layout::Rect {
    let Some((left, top, w, h)) = story_clear_native(story, chrome_canvas) else {
        return ratatui::layout::Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let (right, bottom) = (left + w, top + h);

    let dev_left = scale.off_x as f32 + left as f32 * scale.s;
    let dev_top = scale.off_y as f32 + top as f32 * scale.s;
    let dev_right = scale.off_x as f32 + right as f32 * scale.s;
    let dev_bottom = scale.off_y as f32 + bottom as f32 * scale.s;

    let cw_px = if cell_px.0 == 0 { 1 } else { cell_px.0 } as f32;
    let ch_px = if cell_px.1 == 0 { 1 } else { cell_px.1 } as f32;

    let cell_left = (dev_left / cw_px).ceil() as u16;
    let cell_top = (dev_top / ch_px).ceil() as u16;
    let cell_right = (dev_right / cw_px).floor() as u16;
    let cell_bottom = (dev_bottom / ch_px).floor() as u16;

    let width = cell_right.saturating_sub(cell_left).max(1);
    let height = cell_bottom.saturating_sub(cell_top).max(1);

    let cell_left = cell_left.min(pane_cells.0.saturating_sub(1).max(0));
    let cell_top = cell_top.min(pane_cells.1.saturating_sub(1).max(0));
    let width = width.min(pane_cells.0.saturating_sub(cell_left));
    let height = height.min(pane_cells.1.saturating_sub(cell_top));

    ratatui::layout::Rect { x: cell_left, y: cell_top, width, height }
}

/// Rasterize `main`'s wrapped lines (then the input line + block cursor when
/// `main.awaiting`) into `canvas` starting at native px `(ox, oy)`, one glyph per
/// FONT×FONT cell, transparent glyph bg (draws over chrome/background art).
/// Clipped to `rows` lines and `cols` columns.
pub fn draw_story_text(canvas: &mut RgbaImage, main: &MainText, ox: u32, oy: u32, cols: u16, rows: u16, fg: Rgba<u8>) {
    let mut row = 0u32;
    for line in &main.lines {
        if row >= rows as u32 {
            return;
        }
        for (col, glyph) in line.chars().take(cols as usize).enumerate() {
            crate::render::bitfont::blit_glyph(canvas, glyph, ox + col as u32 * FONT, oy + row * FONT, FONT, FONT, fg, None);
        }
        row += 1;
    }
    if main.awaiting && row < rows as u32 {
        for (col, glyph) in main.input.chars().take(cols as usize).enumerate() {
            crate::render::bitfont::blit_glyph(canvas, glyph, ox + col as u32 * FONT, oy + row * FONT, FONT, FONT, fg, None);
        }
        fill_cell(canvas, ox + (main.cursor_col as u32).min(cols.saturating_sub(1) as u32) * FONT, oy + row * FONT, FONT, FONT, fg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{BorderPref, BufferWindow, GraphicsWindow, GridCell, GridWindow};
    use std::sync::Arc;

    fn grid_item(x_px: u16) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
            }),
        }
    }

    fn graphics_item(x_px: u16) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::new(1, 1));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas, version: 0, upscale: false }),
        }
    }

    fn buffer_item(x_px: u16, primary: bool) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary, ..Default::default() }),
        }
    }

    #[test]
    fn story_is_the_primary_buffer_and_chrome_preserves_order() {
        let items = vec![graphics_item(1), grid_item(2), buffer_item(3, true)];
        let layout = classify_windows(&items);
        let story = layout.story.expect("primary buffer found");
        assert!(matches!(&story.node, WinNode::Buffer(b) if b.primary));
        assert_eq!(story.x_px, 3);
        assert_eq!(layout.chrome.len(), 2);
        assert_eq!(layout.chrome[0].x_px, 1);
        assert_eq!(layout.chrome[1].x_px, 2);
    }

    #[test]
    fn no_primary_buffer_means_no_story_and_all_chrome() {
        let items = vec![grid_item(1), graphics_item(2), buffer_item(3, false)];
        let layout = classify_windows(&items);
        assert!(layout.story.is_none());
        assert_eq!(layout.chrome.len(), items.len());
    }

    fn colors() -> ColorScheme {
        ColorScheme::default()
    }

    fn graphics_window(x_px: u16, y_px: u16, w: u16, h: u16, canvas: image::RgbaImage) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w, h, x_px, y_px, w_px: w, h_px: h, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win: 0, canvas: Arc::new(canvas), version: 0, upscale: false }),
        }
    }

    #[test]
    fn frame_opaque_border_transparent_interior_and_outside_stays_transparent() {
        // 20x20 native canvas, one chrome Graphics window covering it whose
        // source canvas has an opaque 1px border ring and a transparent
        // center. The built chrome canvas should mirror that: opaque at the
        // border, transparent at the center, and transparent outside the
        // window (there is none here, but the whole canvas is checked).
        let mut src = image::RgbaImage::new(20, 20);
        for x in 0..20u32 {
            src.put_pixel(x, 0, Rgba([255, 255, 255, 255]));
            src.put_pixel(x, 19, Rgba([255, 255, 255, 255]));
        }
        for y in 0..20u32 {
            src.put_pixel(0, y, Rgba([255, 255, 255, 255]));
            src.put_pixel(19, y, Rgba([255, 255, 255, 255]));
        }
        let win = graphics_window(0, 0, 20, 20, src);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), &colors());
        assert_eq!(c.get_pixel(0, 0)[3], 255, "border pixel is opaque");
        assert_eq!(c.get_pixel(10, 10)[3], 0, "center is transparent");
    }

    #[test]
    fn later_graphics_entry_draws_over_earlier_through_its_transparent_margin() {
        // Two overlapping chrome Graphics entries at the same native spot
        // (4,4), 8x8 each: "base" solid colour A, then "indicator" solid
        // colour B on its left half and transparent on its right half.
        // Later-drawn wins where opaque; the base shows through the
        // indicator's transparent right half.
        let color_a = Rgba([200, 0, 0, 255]);
        let color_b = Rgba([0, 200, 0, 255]);
        let base = image::RgbaImage::from_pixel(8, 8, color_a);
        let mut indicator = image::RgbaImage::new(8, 8);
        for y in 0..8u32 {
            for x in 0..4u32 {
                indicator.put_pixel(x, y, color_b);
            }
        }
        let base_win = graphics_window(4, 4, 8, 8, base);
        let indicator_win = graphics_window(4, 4, 8, 8, indicator);
        let chrome: Vec<&PositionedWindow> = vec![&base_win, &indicator_win];
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), &colors());
        assert_eq!(*c.get_pixel(5, 8), color_b, "left half shows the indicator (last-drawn wins)");
        assert_eq!(*c.get_pixel(10, 8), color_a, "right half shows the base through the transparent margin");
    }

    #[test]
    fn status_grid_glyph_paints_fg_in_its_native_pixel_cell() {
        let mut cells = vec![GridCell { ch: ' ', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 }; 6];
        cells[1 * 3 + 2] = GridCell { ch: 'A', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let win = PositionedWindow {
            x: 0, y: 0, w: 3, h: 2, x_px: 10, y_px: 4, w_px: 24, h_px: 16, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                cols: 3, rows: 2, cells, active_rows: 2, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let fg = Rgba([0, 255, 255, 255]);
        let c = build_chrome_canvas(&chrome, (40, 24), fg, &colors());
        // cell (col=2,row=1) native px box is (26,12)..(34,20).
        assert!(
            (26..34).any(|x| (12..20).any(|y| *c.get_pixel(x, y) == fg)),
            "glyph fg pixels appear within the status cell's native box"
        );
    }

    #[test]
    fn uniform_scale_letterboxes() {
        let scale = uniform_scale((320, 200), (640, 480));
        assert_eq!(scale.s, 2.0);
        assert_eq!(scale.off_x, 0);
        assert_eq!(scale.off_y, 40);
    }

    #[test]
    fn story_viewport_clears_the_chrome_ring() {
        // 40x40 native canvas: opaque top band rows 0..8, opaque left cols
        // 0..8 and right cols 32..40 across all rows; interior transparent.
        let mut canvas = image::RgbaImage::new(40, 40);
        let opaque = Rgba([255, 255, 255, 255]);
        for y in 0..40u32 {
            for x in 0..40u32 {
                let in_band = y < 8;
                let in_side = x < 8 || x >= 32;
                if in_band || in_side {
                    canvas.put_pixel(x, y, opaque);
                }
            }
        }
        let story = buffer_item(0, true);
        // buffer_item defaults x_px/y_px to 0 and w_px/h_px to 8; override via
        // a fresh PositionedWindow spanning the whole native area.
        let story = PositionedWindow { x_px: 0, y_px: 0, w_px: 40, h_px: 40, ..story };
        let scale = uniform_scale((40, 40), (40, 40));
        let rect = story_viewport(Some(&story), &canvas, &scale, (40, 40), (1, 1));
        assert!(rect.x >= 8, "left edge clears the left band: x={}", rect.x);
        assert!(rect.y >= 8, "top edge clears the top band: y={}", rect.y);
        assert!(rect.x + rect.width <= 32, "right edge clears the right band: x+w={}", rect.x + rect.width);
        assert!(rect.width >= 1);
        assert!(rect.height >= 1);
    }

    #[test]
    fn story_viewport_no_story_is_full_pane() {
        let canvas = image::RgbaImage::new(40, 40);
        let scale = uniform_scale((40, 40), (40, 40));
        let rect = story_viewport(None, &canvas, &scale, (40, 40), (1, 1));
        assert_eq!(rect, ratatui::layout::Rect { x: 0, y: 0, width: 40, height: 40 });
    }
}
