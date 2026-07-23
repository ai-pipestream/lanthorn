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

/// 1:1 opaque-over blit of `src` into `dst` at `(dx, dy)`, clipped to the
/// `max_w × max_h` box anchored at `(dx, dy)` (a v6 window's pixel box).
pub(crate) fn blit_clipped(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, max_w: u32, max_h: u32) {
    let w = src.width().min(max_w);
    let h = src.height().min(max_h);
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..h {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        for ox in 0..w {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let p = *src.get_pixel(ox, oy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

/// Like [`blit_clipped`], but starts reading `src` at row `src_y` — for a
/// margin float partially scrolled off the top of the story view.
pub(crate) fn blit_clipped_src(dst: &mut RgbaImage, src: &RgbaImage, dx: u32, dy: u32, src_y: u32, max_w: u32, max_h: u32) {
    let w = src.width().min(max_w);
    let h = src.height().saturating_sub(src_y).min(max_h);
    let (dstw, dsth) = (dst.width(), dst.height());
    for oy in 0..h {
        let ty = dy + oy;
        if ty >= dsth {
            break;
        }
        for ox in 0..w {
            let tx = dx + ox;
            if tx >= dstw {
                break;
            }
            let p = *src.get_pixel(ox, src_y + oy);
            if p[3] >= 128 {
                dst.put_pixel(tx, ty, Rgba([p[0], p[1], p[2], 255]));
            }
        }
    }
}

/// The v6 font cell size in game pixels — matches `zvm::screen::V6_FONT_WIDTH`.
const FONT: u32 = 8;

/// A window-0 inline picture (drop-cap, room icon) floated at the left margin
/// of the story text: anchored to a wrapped display row, indenting the rows
/// beside it. `row` is relative to the visible window and may be negative when
/// the float has partially scrolled off the top.
#[derive(Debug, Clone)]
pub struct RasterFloat {
    pub row: i32,
    pub rows: u16,
    pub indent_cols: u16,
    pub img: std::sync::Arc<RgbaImage>,
}

/// The story (primary) window's rasterizable content: visible wrapped lines
/// (oldest-first), the live input line, and the caret column. `awaiting` gates
/// the input line + block cursor (drawn only when the game has host focus).
/// `floats` carries the window-0 inline pictures anchored within the visible
/// rows — blitted at the left margin with text indented beside them.
#[derive(Debug, Default, Clone)]
pub struct MainText {
    pub lines: Vec<String>,
    pub input: String,
    pub cursor_col: u16,
    pub awaiting: bool,
    pub floats: Vec<RasterFloat>,
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

/// The v6 window list split into the story window, the story window's own
/// picture (the room illustration — story content, NOT chrome), and everything
/// else (chrome), in input order.
pub struct V6Layout<'a> {
    pub story: Option<&'a PositionedWindow>,
    /// The primary window's Graphics entry (window 0's picture canvas — a room
    /// illustration). It belongs to the story, so it is rendered inside the story
    /// region rather than composited as absolute chrome over the frame.
    pub story_gfx: Option<&'a PositionedWindow>,
    pub chrome: Vec<&'a PositionedWindow>,
}

/// Classify `items`: the first primary `Buffer` becomes `story`; window 0's own
/// `Graphics` entry becomes `story_gfx` (story content); every other entry (in
/// input order) goes into `chrome`. With no primary `Buffer`, `story` is `None`
/// and non-window-0 graphics/grids are chrome.
pub fn classify_windows(items: &[PositionedWindow]) -> V6Layout<'_> {
    let mut story = None;
    let mut story_gfx = None;
    let mut chrome = Vec::new();
    for pw in items {
        if story.is_none() && matches!(&pw.node, WinNode::Buffer(b) if b.primary) {
            story = Some(pw);
        } else if story_gfx.is_none() && matches!(&pw.node, WinNode::Graphics(g) if g.win == 0) {
            story_gfx = Some(pw);
        } else {
            chrome.push(pw);
        }
    }
    V6Layout { story, story_gfx, chrome }
}

/// The story window's own background colour (set by the game via
/// `set_colour`), resolved to an opaque RGBA for filling the story rect
/// before floats/text. `None` when the game set no colour — the caller then
/// leaves the rect transparent (the theme backdrop shows through, unchanged
/// from before this colour handling existed).
pub fn story_bg_rgba(story: Option<&PositionedWindow>, colors: &ColorScheme) -> Option<Rgba<u8>> {
    let WinNode::Buffer(b) = &story?.node else { return None };
    // `bg`, when `Some`, always packs a non-Default channel (see
    // `state::pack_zcolour`), so the fallback here is never actually used —
    // it exists only to satisfy `packed_to_rgba`'s signature.
    Some(packed_to_rgba(b.bg?, Rgba([0, 0, 0, 255]), colors))
}

pub(crate) fn fill_cell(canvas: &mut RgbaImage, px: u32, py: u32, cw: u32, ch: u32, color: Rgba<u8>) {
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
///
/// A `px_texts` run's `style` bit 1 (reverse) swaps its resolved fg/bg: the
/// glyph ink is drawn in the run's (window) background colour and a solid
/// block in the run's foreground colour is painted behind it — reverse always
/// paints an opaque block (there is no "transparent ink"), so a run whose
/// colours are unset falls back to `default_bg`/`default_fg` respectively
/// rather than leaving the swapped-in channel transparent.
pub fn build_chrome_canvas(
    chrome: &[&PositionedWindow],
    native: (u16, u16),
    default_fg: Rgba<u8>,
    default_bg: Rgba<u8>,
    colors: &ColorScheme,
) -> RgbaImage {
    let mut canvas = RgbaImage::new(native.0 as u32, native.1 as u32);

    // Pass 1 — Graphics entries, in list order. The window canvas is authored in
    // native game pixels (pictures drawn at their native size/coords), so blit it
    // 1:1 at the window origin — never scaled — and clip to the window's pixel
    // box (ZMSD §8: plotting is always clipped to the window; a canvas can be
    // larger than the current box when the window has since shrunk).
    for it in chrome {
        if let WinNode::Graphics(gwn) = &it.node {
            let src = &gwn.canvas;
            blit_clipped(&mut canvas, src, it.x_px as u32, it.y_px as u32, it.w_px.max(1) as u32, it.h_px.max(1) as u32);
        }
    }

    // Pass 2 — Grid (status) entries, in list order. A v6 grid with
    // pixel-positioned runs draws those at their EXACT game pixel positions
    // (Zork Zero's banner text sits at rows 6/14, on the ribbon art — cell
    // quantization would snap it to the banner's top edge); the cell grid is
    // the fallback for grids without them.
    for it in chrome {
        if let WinNode::Grid(g) = &it.node {
            let ox = it.x_px as u32;
            let oy = it.y_px as u32;
            if !g.px_texts.is_empty() {
                // A packed colour is EXPLICIT only when the game named a real
                // colour: ZColour::Default (0) and Standard 0/1 ("current"/
                // "default", ZMSD §8.3.1) are not choices, they're inheritance.
                // Reverse video over frame art (Zork0's ribbon labels) with
                // only inherited colours must NOT paint an opaque block — the
                // original renders dark ink directly ON the art. A block is
                // painted only when the game explicitly chose colours.
                let explicit = |packed: u32| packed != 0 && !((packed >> 24) == 1 && (packed & 0xFF) <= 1);
                for t in &g.px_texts {
                    let (fg, bg) = if t.style & 1 != 0 {
                        if explicit(t.fg) || explicit(t.bg) {
                            // Real colour pair: swap and paint the block.
                            (packed_to_rgba(t.bg, default_bg, colors), Some(packed_to_rgba(t.fg, default_fg, colors)))
                        } else {
                            // Inherited colours: dark ink on the art, no block.
                            (default_bg, None)
                        }
                    } else {
                        (
                            packed_to_rgba(t.fg, default_fg, colors),
                            explicit(t.bg).then(|| packed_to_rgba(t.bg, default_bg, colors)),
                        )
                    };
                    let py = oy + (t.y.max(1) as u32 - 1);
                    for (i, ch) in t.text.chars().enumerate() {
                        let px = ox + (t.x.max(1) as u32 - 1) + i as u32 * FONT;
                        crate::render::bitfont::blit_glyph(&mut canvas, ch, px, py, FONT, FONT, fg, bg);
                    }
                }
                continue;
            }
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

    let cell_left = cell_left.min(pane_cells.0.saturating_sub(1));
    let cell_top = cell_top.min(pane_cells.1.saturating_sub(1));
    let width = width.min(pane_cells.0.saturating_sub(cell_left));
    let height = height.min(pane_cells.1.saturating_sub(cell_top));

    ratatui::layout::Rect { x: cell_left, y: cell_top, width, height }
}

/// The story viewport cell rect (relative to the pane's top-left cell) for the
/// HYBRID render mode: the win0 box (`story` x_px/y_px/w_px/h_px, native game
/// pixels) mapped through the letterbox [`Scale`] to device pixels, then quantized
/// to whole cells rounding INWARD (ceil the top-left, floor the bottom-right) so
/// no surrounding chrome cell overlaps the terminal story region. Unlike
/// [`story_viewport`], this does NOT inset around opaque chrome pixels — the raw
/// window box is the viewport, and the chrome ring is drawn around it. Falls back
/// to the full pane when there is no story window.
pub fn story_viewport_box(
    story: Option<&PositionedWindow>,
    scale: &Scale,
    pane_cells: (u16, u16),
    cell_px: (u16, u16),
) -> ratatui::layout::Rect {
    let Some(story) = story else {
        return ratatui::layout::Rect { x: 0, y: 0, width: pane_cells.0, height: pane_cells.1 };
    };
    let left = story.x_px as f32;
    let top = story.y_px as f32;
    let right = (story.x_px as u32 + story.w_px as u32) as f32;
    let bottom = (story.y_px as u32 + story.h_px as u32) as f32;

    let dev_left = scale.off_x as f32 + left * scale.s;
    let dev_top = scale.off_y as f32 + top * scale.s;
    let dev_right = scale.off_x as f32 + right * scale.s;
    let dev_bottom = scale.off_y as f32 + bottom * scale.s;

    let cw_px = if cell_px.0 == 0 { 1 } else { cell_px.0 } as f32;
    let ch_px = if cell_px.1 == 0 { 1 } else { cell_px.1 } as f32;

    // Round INWARD: ceil the top-left, floor the bottom-right, so the viewport is
    // the largest whole-cell rect fully inside the win0 box.
    let cell_left = (dev_left / cw_px).ceil() as u16;
    let cell_top = (dev_top / ch_px).ceil() as u16;
    let cell_right = (dev_right / cw_px).floor() as u16;
    let cell_bottom = (dev_bottom / ch_px).floor() as u16;

    let width = cell_right.saturating_sub(cell_left).max(1);
    let height = cell_bottom.saturating_sub(cell_top).max(1);

    let cell_left = cell_left.min(pane_cells.0.saturating_sub(1));
    let cell_top = cell_top.min(pane_cells.1.saturating_sub(1));
    let width = width.min(pane_cells.0.saturating_sub(cell_left));
    let height = height.min(pane_cells.1.saturating_sub(cell_top));

    ratatui::layout::Rect { x: cell_left, y: cell_top, width, height }
}

/// The chrome RING cell rects around a story `viewport` inside a `pane`: up to
/// four non-overlapping rects (top, bottom, left, right) that exactly tile
/// `pane − viewport`. The top and bottom bands span the pane's full width (and so
/// own the corners); the left and right bands span only the viewport's vertical
/// extent. An edge-flush viewport omits that side's band; `viewport == pane`
/// yields an empty list. `viewport` is assumed to lie within `pane`; it is clamped
/// defensively. Both rects share one coordinate space (both absolute, or both
/// pane-relative).
pub fn chrome_bands(pane: ratatui::layout::Rect, viewport: ratatui::layout::Rect) -> Vec<ratatui::layout::Rect> {
    use ratatui::layout::Rect;
    // Clamp the viewport within the pane so the band arithmetic can't underflow.
    let vx = viewport.x.clamp(pane.x, pane.right());
    let vy = viewport.y.clamp(pane.y, pane.bottom());
    let vr = viewport.right().clamp(vx, pane.right());
    let vb = viewport.bottom().clamp(vy, pane.bottom());

    let mut out = vec![
        // Top band: full pane width, from the pane top down to the viewport top.
        Rect::new(pane.x, pane.y, pane.width, vy - pane.y),
        // Bottom band: full pane width, from the viewport bottom to the pane bottom.
        Rect::new(pane.x, vb, pane.width, pane.bottom() - vb),
        // Left band: the viewport's vertical span, from the pane left to the viewport left.
        Rect::new(pane.x, vy, vx - pane.x, vb - vy),
        // Right band: the viewport's vertical span, from the viewport right to the pane right.
        Rect::new(vr, vy, pane.right() - vr, vb - vy),
    ];
    out.retain(|r| r.width > 0 && r.height > 0);
    out
}

/// Rasterize `main`'s wrapped lines (then the input line + block cursor when
/// `main.awaiting`) into `canvas` starting at native px `(ox, oy)`, one glyph per
/// FONT×FONT cell, transparent glyph bg (draws over chrome/background art).
/// Clipped to `rows` lines and `cols` columns.
pub fn draw_story_text(canvas: &mut RgbaImage, main: &MainText, ox: u32, oy: u32, cols: u16, rows: u16, fg: Rgba<u8>) {
    let region_h = rows as u32 * FONT;
    // Floats first (text draws over/beside them). A float that has partially
    // scrolled off the top (row < 0) is drawn cropped from its own top.
    for f in &main.floats {
        let src = &*f.img;
        let crop_top = if f.row < 0 { (-f.row) as u32 * FONT } else { 0 };
        if crop_top >= src.height() {
            continue;
        }
        let dy = oy + (f.row.max(0) as u32) * FONT;
        let max_h = region_h.saturating_sub(dy - oy);
        blit_clipped_src(canvas, src, ox, dy, crop_top, cols as u32 * FONT, max_h);
    }
    let indent_at = |row: u32| -> u32 {
        main.floats
            .iter()
            .filter(|f| f.row <= row as i32 && (row as i32) < f.row + f.rows as i32)
            .map(|f| f.indent_cols as u32)
            .max()
            .unwrap_or(0)
    };
    let mut row = 0u32;
    for line in &main.lines {
        if row >= rows as u32 {
            return;
        }
        let indent = indent_at(row);
        let avail = (cols as u32).saturating_sub(indent);
        for (col, glyph) in line.chars().take(avail as usize).enumerate() {
            crate::render::bitfont::blit_glyph(canvas, glyph, ox + (indent + col as u32) * FONT, oy + row * FONT, FONT, FONT, fg, None);
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
    use crate::engine::{BorderPref, BufferWindow, GraphicsWindow, GridCell, GridWindow, PxText};
    use std::sync::Arc;

    fn grid_item(x_px: u16) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: Vec::new(),
            }),
        }
    }

    fn graphics_item(x_px: u16) -> PositionedWindow {
        graphics_item_win(x_px, 7)
    }

    fn graphics_item_win(x_px: u16, win: u32) -> PositionedWindow {
        let canvas = Arc::new(image::RgbaImage::new(1, 1));
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow { win, canvas, version: 0, upscale: false }),
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
        assert!(layout.story_gfx.is_none());
        assert_eq!(layout.chrome.len(), items.len());
    }

    #[test]
    fn window_zero_graphics_is_story_content_not_chrome() {
        // The primary window's own picture (window 0) is the room illustration —
        // story content, kept out of chrome so it renders inside the story region.
        let items = vec![
            graphics_item_win(1, 0), // window 0's illustration
            graphics_item_win(2, 7), // window 7 frame → chrome
            buffer_item(3, true),    // story
        ];
        let layout = classify_windows(&items);
        assert_eq!(layout.story.expect("story").x_px, 3);
        assert_eq!(layout.story_gfx.expect("story_gfx").x_px, 1);
        assert_eq!(layout.chrome.len(), 1, "only window 7 graphics is chrome");
        assert_eq!(layout.chrome[0].x_px, 2);
    }

    fn colors() -> ColorScheme {
        ColorScheme::default()
    }

    #[test]
    fn story_text_wraps_right_of_float_and_blits_it() {
        // Rows covered by a float are inset by its indent (text flows beside the
        // picture); rows past it are flush left; the float's pixels are blitted
        // at its anchored row.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT).any(|dy| (0..FONT).any(|dx| c.get_pixel(col * FONT + dx, row * FONT + dy)[3] > 0))
        };
        // A 16x16 opaque red image → float of 2 rows.
        let img = RgbaImage::from_pixel(16, 16, Rgba([200, 20, 20, 255]));
        let main = MainText {
            lines: vec!["AAAA".into(), "BBBB".into(), "CCCC".into()],
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, indent_cols: 3, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT, 5 * FONT);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]));
        // Rows 0-1 (beside float): glyph ink starts at column 3.
        assert!(cell_has_ink(&canvas, 0, 0), "float pixels occupy row 0 col 0");
        assert_eq!(*canvas.get_pixel(4, 4), Rgba([200, 20, 20, 255]), "float blitted at its row");
        assert!(cell_has_ink(&canvas, 3, 0), "row 0 col 3 inked (text beside the float)");
        assert!(cell_has_ink(&canvas, 3, 1), "row 1 col 3 inked (text beside the float)");
        // Row 2 (past the float): ink flush left.
        assert!(cell_has_ink(&canvas, 0, 2), "row 2 col 0 inked (flush left below float)");
    }

    #[test]
    fn story_text_scrolled_float_is_cropped_not_pinned() {
        // A float whose anchor scrolled above the view (row = -1) draws only its
        // remaining rows, cropped from its own top.
        let mut img = RgbaImage::new(8, 16);
        for y in 0..16 {
            // Top half green, bottom half blue — the visible part must be blue.
            let c = if y < 8 { Rgba([0, 200, 0, 255]) } else { Rgba([0, 0, 200, 255]) };
            for x in 0..8 { img.put_pixel(x, y, c); }
        }
        let main = MainText {
            lines: vec!["XXXX".into()],
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: -1, rows: 2, indent_cols: 2, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT, 3 * FONT);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 3, Rgba([255, 255, 255, 255]));
        assert_eq!(*canvas.get_pixel(4, 4), Rgba([0, 0, 200, 255]), "visible slice is the float's BOTTOM half");
    }

    #[test]
    fn chrome_graphics_blits_native_and_clips_to_window_box() {
        // The window canvas is authored in native game pixels: build_chrome_canvas
        // blits it 1:1 at the window origin (never scaled to the declared box) and
        // clips at the box edge (ZMSD §8: plotting is always clipped to the window).
        let mut src = image::RgbaImage::new(48, 43);
        src.put_pixel(40, 38, Rgba([10, 200, 30, 255])); // marker low in the canvas
        src.put_pixel(2, 2, Rgba([200, 10, 30, 255])); // marker near the top-left
        let win = |h_px: u16, canvas: image::RgbaImage| PositionedWindow {
            x: 0, y: 0, w: 40, h: 1,
            x_px: 4, y_px: 4, // window origin
            w_px: 320, h_px,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(GraphicsWindow {
                win: 1, canvas: Arc::new(canvas), version: 0, upscale: false,
            }),
        };
        // Box tall enough (40): both markers land 1:1 — never squashed.
        let tall = win(40, src.clone());
        let canvas = build_chrome_canvas(&[&tall], (100, 100), Rgba([0, 0, 0, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(canvas.get_pixel(6, 6)[3], 255, "top-left marker at native (6,6)");
        assert_eq!(canvas.get_pixel(44, 42)[3], 255, "low marker 1:1 at native (44,42)");
        // Box only 5 tall: content past the box clips; nothing squashes into it.
        let short = win(5, src);
        let canvas = build_chrome_canvas(&[&short], (100, 100), Rgba([0, 0, 0, 255]), Rgba([0, 0, 0, 255]), &colors());
        assert_eq!(canvas.get_pixel(6, 6)[3], 255, "top-left marker inside the box survives");
        assert_eq!(canvas.get_pixel(44, 42)[3], 0, "content below the 5px box is clipped");
        for y in 4..9 {
            assert_eq!(canvas.get_pixel(44, y)[3], 0, "no squashed copy inside the box (y={y})");
        }
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
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
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
        let c = build_chrome_canvas(&chrome, (20, 20), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
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
                px_texts: Vec::new(),
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let fg = Rgba([0, 255, 255, 255]);
        let c = build_chrome_canvas(&chrome, (40, 24), fg, Rgba([0, 0, 0, 255]), &colors());
        // cell (col=2,row=1) native px box is (26,12)..(34,20).
        assert!(
            (26..34).any(|x| (12..20).any(|y| *c.get_pixel(x, y) == fg)),
            "glyph fg pixels appear within the status cell's native box"
        );
    }

    // ── px_text colour + reverse-video (Lane C) ─────────────────────────────
    //
    // These probe the SOLID FILL colour behind a run, not individual glyph
    // pixels: a run whose text is a single space has no ink bits set, so its
    // whole FONT×FONT cell is exactly `blit_glyph`'s `bg` fill colour (or
    // fully transparent when `bg` is `None`) — a robust way to assert which
    // colour the resolver chose without depending on font-bitmap geometry.
    const RED: u32 = 0x03FF_0000; // True24 packed
    const BLUE: u32 = 0x0300_00FF; // True24 packed

    fn px_text_grid_item(text: &str, style: u8, fg: u32, bg: u32) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: vec![PxText { y: 1, x: 1, text: text.into(), style, fg, bg }],
            }),
        }
    }

    #[test]
    fn px_text_run_fills_its_cell_with_the_explicit_background() {
        let win = px_text_grid_item(" ", 0, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), Rgba([0, 0, 255, 255]), "cell filled with the run's bg (blue) at ({x},{y})");
            }
        }
    }

    #[test]
    fn px_text_reverse_swaps_the_fill_to_the_foreground_colour() {
        // Same run as above but with style bit 1 (reverse) set: the swap makes
        // the run's FOREGROUND (red) the fill colour instead of its background.
        let win = px_text_grid_item(" ", 1, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), Rgba([255, 0, 0, 255]), "reverse fill is the run's fg (red) at ({x},{y})");
            }
        }
    }

    #[test]
    fn px_text_reverse_with_inherited_colours_draws_dark_ink_no_block() {
        // The run never chose an explicit colour (fg=bg=0/Default): reverse
        // over frame art must NOT paint an opaque block — Zork0's ribbon
        // labels print in reverse with inherited colours and the original
        // shows dark ink directly ON the banner art (a block would erase it,
        // the black-box regression the user hit). A blank glyph therefore
        // leaves the canvas transparent; an inked glyph draws in default_bg.
        let win = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let default_fg = Rgba([10, 20, 30, 255]);
        let default_bg = Rgba([40, 50, 60, 255]);
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert_eq!(c.get_pixel(4, 4)[3], 0, "no block behind a blank reverse glyph with inherited colours");
        let win = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "reverse ink with inherited colours draws in the themed default_bg (dark on the art)"
        );
    }

    #[test]
    fn px_text_reverse_with_explicit_colours_paints_the_swapped_block() {
        // A run whose game explicitly chose colours DOES paint the swap block.
        let win = px_text_grid_item(" ", 1, RED, BLUE);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([1, 1, 1, 255]), Rgba([2, 2, 2, 255]), &colors());
        assert_eq!(c.get_pixel(4, 4)[3], 255, "explicit reverse paints an opaque block");
    }

    #[test]
    fn px_text_no_bg_stays_transparent_without_reverse() {
        // Regression guard: a run with no explicit bg (0/Default) and no
        // reverse style stays transparent — unchanged from before colour
        // handling existed, so frame art under status text still shows through.
        let win = px_text_grid_item(" ", 0, RED, 0);
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let c = build_chrome_canvas(&chrome, (8, 8), Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255]), &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(c.get_pixel(x, y)[3], 0, "no bg, no reverse ⇒ transparent at ({x},{y})");
            }
        }
    }

    // ── story region background fill (Lane C) ───────────────────────────────

    #[test]
    fn story_bg_rgba_resolves_the_windows_own_colour() {
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, bg: Some(BLUE), ..Default::default() }),
        };
        let color = story_bg_rgba(Some(&story), &colors()).expect("win0 set a bg colour");
        assert_eq!(color, Rgba([0, 0, 255, 255]));
    }

    #[test]
    fn story_bg_rgba_is_none_when_the_game_set_no_colour() {
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px: 8, h_px: 8, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        assert!(story_bg_rgba(Some(&story), &colors()).is_none(), "no game colour ⇒ None (caller leaves it transparent)");
    }

    #[test]
    fn story_bg_rgba_fills_the_clear_interior_rect() {
        // End-to-end through the same calls screen.rs makes: resolve the colour,
        // then fill_cell the story_clear_native rect with it.
        let story = PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 2, y_px: 2, w_px: 4, h_px: 4, left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, bg: Some(RED), ..Default::default() }),
        };
        let mut canvas = RgbaImage::new(8, 8);
        let (sx, sy, sw, sh) = story_clear_native(Some(&story), &canvas).expect("story window present");
        let color = story_bg_rgba(Some(&story), &colors()).expect("bg set");
        fill_cell(&mut canvas, sx, sy, sw, sh, color);
        for y in 2..6 {
            for x in 2..6 {
                assert_eq!(*canvas.get_pixel(x, y), Rgba([255, 0, 0, 255]), "story rect filled red at ({x},{y})");
            }
        }
        assert_eq!(canvas.get_pixel(0, 0)[3], 0, "outside the story rect stays transparent");
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

    // ── Hybrid render mode: story_viewport_box + chrome_bands ──────────────────

    #[test]
    fn story_viewport_box_maps_win0_box_inward_to_cells() {
        // Native 320×200 game, win0 box (43,39,234,160). Scale 1:1 (native px ==
        // device px), 8 px/cell. Rounding INWARD: left ceil(43/8)=6, top
        // ceil(39/8)=5, right floor((43+234)/8)=floor(277/8)=34,
        // bottom floor((39+160)/8)=floor(199/8)=24 → 28×19 cells at (6,5).
        let story = PositionedWindow { x_px: 43, y_px: 39, w_px: 234, h_px: 160, ..buffer_item(0, true) };
        let scale = uniform_scale((320, 200), (320, 200)); // s = 1.0, no offset
        assert_eq!(scale.s, 1.0);
        let rect = story_viewport_box(Some(&story), &scale, (40, 25), (8, 8));
        assert_eq!(rect, ratatui::layout::Rect { x: 6, y: 5, width: 28, height: 19 });
    }

    #[test]
    fn story_viewport_box_no_story_is_full_pane() {
        let scale = uniform_scale((320, 200), (320, 200));
        let rect = story_viewport_box(None, &scale, (40, 25), (8, 8));
        assert_eq!(rect, ratatui::layout::Rect { x: 0, y: 0, width: 40, height: 25 });
    }

    #[test]
    fn chrome_bands_tile_pane_minus_viewport_without_overlap() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        let viewport = Rect::new(6, 5, 28, 19); // interior, all four edges inset
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 4, "all four edges produce a band");
        // Non-overlap + exact tiling: every pane cell OUTSIDE the viewport is
        // covered exactly once; every viewport cell is covered zero times.
        let mut cover = vec![0u8; (pane.width as usize) * (pane.height as usize)];
        for b in &bands {
            for y in b.y..b.bottom() {
                for x in b.x..b.right() {
                    cover[y as usize * pane.width as usize + x as usize] += 1;
                }
            }
        }
        for y in 0..pane.height {
            for x in 0..pane.width {
                let inside_vp = (viewport.x..viewport.right()).contains(&x) && (viewport.y..viewport.bottom()).contains(&y);
                let c = cover[y as usize * pane.width as usize + x as usize];
                if inside_vp {
                    assert_eq!(c, 0, "viewport cell ({x},{y}) untouched by chrome bands");
                } else {
                    assert_eq!(c, 1, "chrome cell ({x},{y}) covered exactly once");
                }
            }
        }
    }

    #[test]
    fn chrome_bands_omit_flush_edges() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        // Viewport flush to the left and top edges → only bottom + right bands.
        let viewport = Rect::new(0, 0, 30, 20);
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 2, "left+top flush → those bands omitted");
        assert!(bands.iter().all(|b| b.x >= 30 || b.y >= 20), "remaining bands are the right/bottom ring");
    }

    #[test]
    fn chrome_bands_full_viewport_is_empty() {
        use ratatui::layout::Rect;
        let pane = Rect::new(0, 0, 40, 25);
        assert!(chrome_bands(pane, pane).is_empty(), "viewport == pane → no chrome");
    }

    #[test]
    fn chrome_bands_absolute_coords_offset_pane() {
        use ratatui::layout::Rect;
        // A pane not anchored at the origin: bands must tile pane − viewport in the
        // same absolute space (the hybrid path passes absolute rects).
        let pane = Rect::new(10, 4, 20, 12);
        let viewport = Rect::new(13, 6, 12, 6);
        let bands = chrome_bands(pane, viewport);
        assert_eq!(bands.len(), 4);
        for b in &bands {
            assert!(b.x >= pane.x && b.right() <= pane.right() && b.y >= pane.y && b.bottom() <= pane.bottom(),
                "band {b:?} stays inside the pane");
        }
    }
}
