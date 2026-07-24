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
/// A packed z-colour (see [`crate::state::pack_zcolour`]) is EXPLICIT only when
/// the game named a real colour. `ZColour::Default` (0) and Standard 0/1
/// ("current"/"default", ZMSD §8.3.1) are not choices — they're inheritance —
/// so they are NOT explicit and the theme keeps the channel. Standard 2-9 and
/// every True/True24 value ARE explicit. Shared by the raster block-paint
/// decision and the cell colour paths so both gate identically. (SQ-0487/0488)
pub(crate) fn packed_explicit(packed: u32) -> bool {
    packed != 0 && !((packed >> 24) == 1 && (packed & 0xFF) <= 1)
}

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
    color_to_rgba(crate::render::resolve_zcolour(z, colors), fallback)
}

/// Resolve a ratatui [`Color`] to an opaque RGBA for the pixel canvas. The cell
/// path renders NAMED ANSI colours (the terminal_default palette maps Standard
/// 2–9 to `Color::Red`/`Color::Blue`/… — the terminal draws them directly), but
/// the raster canvas needs concrete bytes: mapping only `Color::Rgb` dropped
/// every palette colour to the fallback, so Zork Zero's compass-direction
/// letters blitted in the default ink instead of their own colour (SQ-0480). The
/// 16 base ANSI colours resolve to the standard VGA RGB values; `Reset` and
/// `Indexed` (no canonical RGB here) fall back.
pub(crate) fn color_to_rgba(c: ratatui::style::Color, fallback: Rgba<u8>) -> Rgba<u8> {
    use ratatui::style::Color;
    let (r, g, b) = match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
        Color::Reset | Color::Indexed(_) => return fallback,
    };
    Rgba([r, g, b, 255])
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

/// The v6 font cell size in game pixels — matches `zvm::screen::V6_FONT_WIDTH`
/// / `V6_FONT_HEIGHT`. The cell is NON-SQUARE (8×16, SQ-0479): X quantizes by
/// `FONT_W`, Y by `FONT_H`. Glyph masters are 8×8; `blit_glyph` fills the 8×16
/// cell by nearest-neighbour vertical doubling (DOS-authentic).
const FONT_W: u32 = 8;
const FONT_H: u32 = 16;

/// A window-0 inline picture floated beside the story text: anchored to a
/// wrapped display row, reserving columns for the picture and narrowing the rows
/// beside it. `row` is relative to the visible window and may be negative when
/// the float has partially scrolled off the top.
///
/// The float side is expressed by the column fields (not an enum): a LEFT float
/// (Zork Zero's drop-cap) blits at `img_col == 0` with text pushed right
/// (`text_col == reserve_cols`); a RIGHT float (Shogun's opening picture, ZMSD
/// §15 margin picture) blits at `img_col` near the right edge with text flush
/// left (`text_col == 0`). Either way the wrap width on covered rows is
/// `cols - reserve_cols`.
#[derive(Debug, Clone)]
pub struct RasterFloat {
    pub row: i32,
    pub rows: u16,
    /// Columns removed from the text width on the rows this float covers.
    pub reserve_cols: u16,
    /// Column where each covered row's text begins.
    pub text_col: u16,
    /// Column where the picture is blitted.
    pub img_col: u16,
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
///
/// A window whose `w_px`/`h_px` is an unresolved size sentinel — a small
/// negative value stored as a large `u16` (Shogun leaks `0xFFFE` ≈ −2 into a
/// window's `x_size`, ballooning the extent to 65534×200 and the raster canvas
/// allocation with it, SQ-0481) — must not drive the extent. Any dimension with
/// the high bit set (`>= 0x8000`, i.e. negative as `i16`) is far past any real
/// v6 screen (~640 px) so it's treated as unresolved and skipped for that axis;
/// clamping here (presentation) keeps zvm storing window props verbatim for the
/// game to read back (ZMSD §8.8.3.2).
pub fn native_extent(items: &[PositionedWindow]) -> (u16, u16) {
    let mut w = 1u16;
    let mut h = 1u16;
    let resolved = |px: u16| (px as i16) >= 0; // high bit clear ⇒ a real size
    for it in items {
        if resolved(it.w_px) {
            w = w.max(it.x_px.saturating_add(it.w_px));
        }
        if resolved(it.h_px) {
            h = h.max(it.y_px.saturating_add(it.h_px));
        }
        // A window sized to zero can still hold painted text runs at their
        // screen-absolute pixel positions (Journey's height-0 command menu,
        // SQ-0492): its w_px/h_px don't reach the runs, so grow the extent to
        // cover them directly, or the chrome canvas clips the menu off the
        // bottom. Runs carry 1-based top-left coords; a glyph spans FONT×FONT.
        if let WinNode::Grid(g) = &it.node {
            for t in &g.px_texts {
                let n = t.text.chars().count() as u32;
                let right = (t.x.max(1) as u32 - 1) + n * FONT_W;
                let bottom = (t.y.max(1) as u32 - 1) + FONT_H;
                w = w.max(right.min(u16::MAX as u32) as u16);
                h = h.max(bottom.min(u16::MAX as u32) as u16);
            }
        }
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

/// Whether any pixel in the `w × h` box at `(px, py)` of `canvas` is opaque
/// (alpha ≥ 128). Used to tell a reverse-video run sitting ON frame art from one
/// over a clear background, so the art is preserved but a bare selection bar still
/// gets its highlight block (SQ-0487). Out-of-bounds pixels count as transparent.
pub(crate) fn region_has_opaque(canvas: &RgbaImage, px: u32, py: u32, w: u32, h: u32) -> bool {
    let (cw, ch) = (canvas.width(), canvas.height());
    for y in py..(py + h).min(ch) {
        for x in px..(px + w).min(cw) {
            if canvas.get_pixel(x, y)[3] >= 128 {
                return true;
            }
        }
    }
    false
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
                // colour (see `packed_explicit`): inherited colours + reverse
                // over frame art (Zork0's ribbon labels) must NOT paint an
                // opaque block — the original renders dark ink directly ON the
                // art. A block is painted only when the game chose colours.
                let explicit = packed_explicit;
                for t in &g.px_texts {
                    let px0 = t.x.max(1) as u32 - 1;
                    let py = t.y.max(1) as u32 - 1;
                    let (fg, bg) = if t.style & 1 != 0 {
                        if explicit(t.fg) || explicit(t.bg) {
                            // Real colour pair: swap and paint the block.
                            (packed_to_rgba(t.bg, default_bg, colors), Some(packed_to_rgba(t.fg, default_fg, colors)))
                        } else {
                            // Inherited colours + reverse: whether to paint a block
                            // depends on what's BEHIND the run (SQ-0487). Over opaque
                            // frame art (Zork0's ribbon labels) a block would erase the
                            // art, so draw dark ink (default_bg) directly on it, no
                            // block. Over a CLEAR background (Shogun's boot-menu
                            // selection bar — no art behind it) the highlight must be
                            // visible, so paint the swapped block: a solid default_fg
                            // bar with default_bg ink, INCLUDING the blank gap runs the
                            // game paints between the item's words (a reversed space
                            // then fills its cell — no more moth-eaten bar). Pass 1
                            // already blitted every graphics window, so the canvas
                            // shows the real art (or transparency) under this run.
                            let span_w = t.text.chars().count().max(1) as u32 * FONT_W;
                            if region_has_opaque(&canvas, px0, py, span_w, FONT_H) {
                                (default_bg, None)
                            } else {
                                (default_bg, Some(default_fg))
                            }
                        }
                    } else {
                        (
                            packed_to_rgba(t.fg, default_fg, colors),
                            explicit(t.bg).then(|| packed_to_rgba(t.bg, default_bg, colors)),
                        )
                    };
                    // Run coords are SCREEN-absolute 1-based pixels stamped at
                    // paint time (v6 paint semantics) — no window-origin
                    // offset: the window may have moved/shrunk since (Shogun
                    // turns its menu window into a 1-px caret after printing).
                    for (i, ch) in t.text.chars().enumerate() {
                        let px = px0 + i as u32 * FONT_W;
                        crate::render::bitfont::blit_glyph(&mut canvas, ch, px, py, FONT_W, FONT_H, fg, bg);
                    }
                }
                continue;
            }
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let idx = row as usize * g.cols as usize + col as usize;
                    let Some(cell) = g.cells.get(idx) else { continue };
                    let px = ox + col as u32 * FONT_W;
                    let py = oy + row as u32 * FONT_H;
                    if cell.ch == '\0' || cell.ch == ' ' {
                        if cell.bg != 0 {
                            let b = packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors);
                            fill_cell(&mut canvas, px, py, FONT_W, FONT_H, b);
                        }
                        continue;
                    }
                    let fg = packed_to_rgba(cell.fg, default_fg, colors);
                    let cellbg = (cell.bg != 0).then(|| packed_to_rgba(cell.bg, Rgba([0, 0, 0, 255]), colors));
                    crate::render::bitfont::blit_glyph(&mut canvas, cell.ch, px, py, FONT_W, FONT_H, fg, cellbg);
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
    let region_h = rows as u32 * FONT_H;
    // Floats first (text draws over/beside them). A float that has partially
    // scrolled off the top (row < 0) is drawn cropped from its own top. Blitted
    // at `img_col` (0 = left float; near the right edge = right float), clamped
    // to the columns from there to the region's right edge.
    for f in &main.floats {
        let src = &*f.img;
        let crop_top = if f.row < 0 { (-f.row) as u32 * FONT_H } else { 0 };
        if crop_top >= src.height() {
            continue;
        }
        let dy = oy + (f.row.max(0) as u32) * FONT_H;
        let max_h = region_h.saturating_sub(dy - oy);
        let img_x = ox + f.img_col as u32 * FONT_W;
        let max_w = (cols as u32).saturating_sub(f.img_col as u32) * FONT_W;
        blit_clipped_src(canvas, src, img_x, dy, crop_top, max_w, max_h);
    }
    // The active float's (reserved cols, text start col) for a given row — one
    // float is active at a time; when several overlap take the widest reserve.
    let float_at = |row: u32| -> (u32, u32) {
        main.floats
            .iter()
            .filter(|f| f.row <= row as i32 && (row as i32) < f.row + f.rows as i32)
            .map(|f| (f.reserve_cols as u32, f.text_col as u32))
            .max_by_key(|(reserve, _)| *reserve)
            .unwrap_or((0, 0))
    };
    let mut row = 0u32;
    let mut last_row_end = 0u32; // (text_col + text len) of the last drawn line
    for line in &main.lines {
        if row >= rows as u32 {
            return;
        }
        let (reserve, text_col) = float_at(row);
        let avail = (cols as u32).saturating_sub(reserve);
        let mut drawn = 0u32;
        for (col, glyph) in line.chars().take(avail as usize).enumerate() {
            crate::render::bitfont::blit_glyph(canvas, glyph, ox + (text_col + col as u32) * FONT_W, oy + row * FONT_H, FONT_W, FONT_H, fg, None);
            drawn = col as u32 + 1;
        }
        last_row_end = text_col + drawn;
        row += 1;
    }
    if main.awaiting {
        // The live input continues the game's kept prompt line (the last drawn
        // row — Zork Zero's "…HINT): >"), NOT a fresh row below it (SQ-0470a):
        // the caret sits right after the prompt. When the transcript ended on a
        // newline the last line is empty (`last_row_end == 0`) so the input
        // starts a clean row of its own, matching the terminal inline prompt.
        let input_row = row.saturating_sub(1);
        let start = last_row_end;
        if input_row < rows as u32 {
            for (i, glyph) in main.input.chars().enumerate() {
                let col = start + i as u32;
                if col >= cols as u32 {
                    break;
                }
                crate::render::bitfont::blit_glyph(canvas, glyph, ox + col * FONT_W, oy + input_row * FONT_H, FONT_W, FONT_H, fg, None);
            }
            let caret = (start + main.cursor_col as u32).min(cols.saturating_sub(1) as u32);
            fill_cell(canvas, ox + caret * FONT_W, oy + input_row * FONT_H, FONT_W, FONT_H, fg);
        }
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
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        // A 16×32 opaque red image → float of 2 rows (32px / FONT_H(16) = 2).
        let img = RgbaImage::from_pixel(16, 32, Rgba([200, 20, 20, 255]));
        let main = MainText {
            lines: vec!["AAAA".into(), "BBBB".into(), "CCCC".into()],
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, reserve_cols: 3, text_col: 3, img_col: 0, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]));
        // Rows 0-1 (beside float): glyph ink starts at column 3.
        assert!(cell_has_ink(&canvas, 0, 0), "float pixels occupy row 0 col 0");
        assert_eq!(*canvas.get_pixel(4, 20), Rgba([200, 20, 20, 255]), "float blitted at its row (spans y 0..32)");
        assert!(cell_has_ink(&canvas, 3, 0), "row 0 col 3 inked (text beside the float)");
        assert!(cell_has_ink(&canvas, 3, 1), "row 1 col 3 inked (text beside the float)");
        // Row 2 (past the float): ink flush left.
        assert!(cell_has_ink(&canvas, 0, 2), "row 2 col 0 inked (flush left below float)");
    }

    #[test]
    fn story_text_wraps_left_of_right_float_and_blits_it_right() {
        // A RIGHT float (Shogun's opening picture): text stays flush LEFT and is
        // narrowed to `cols - reserve_cols`; the picture blits at `img_col` near
        // the right edge; rows past the picture reclaim full width.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        // 10-col region; a 32×32 image → 4 cols wide, 2 rows tall; reserve 5 cols
        // (image + gutter), text confined to cols 0..5, image blits at col 6.
        let img = RgbaImage::from_pixel(32, 32, Rgba([20, 200, 20, 255]));
        let main = MainText {
            lines: vec!["AAAAAAAA".into(), "BBBB".into(), "CCCCCCCC".into()],
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: 0, rows: 2, reserve_cols: 5, text_col: 0, img_col: 6, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 10, 5, Rgba([255, 255, 255, 255]));
        // Row 0 text is flush left but clipped to the narrowed column (cols 0..5).
        assert!(cell_has_ink(&canvas, 0, 0), "row 0 col 0 inked (text flush left)");
        assert!(!cell_has_ink(&canvas, 5, 0), "row 0 col 5 blank (text narrowed away from the picture)");
        // The picture blits at col 6 (img_col), on the right.
        assert_eq!(*canvas.get_pixel(6 * FONT_W, 0), Rgba([20, 200, 20, 255]), "float blitted at img_col 6");
        // Row 2 (past the float) reclaims full width.
        assert!(cell_has_ink(&canvas, 6, 2), "row 2 col 6 inked (full width below the float)");
    }

    #[test]
    fn packed_standard_palette_colour_blits_its_own_rgb_not_default() {
        // SQ-0480: a run coloured with a Standard palette colour (the compass
        // letters) must blit in that colour, not the default ink. terminal_default
        // maps Standard(3) → Color::Red (a NAMED colour); packed_to_rgba must
        // resolve it to concrete RGB rather than dropping to the fallback.
        let colors = ColorScheme::terminal_default();
        let fallback = Rgba([1, 2, 3, 255]);
        // Standard(3): packed tag 1, value 3 (see state::pack_zcolour).
        let packed_std3 = (1u32 << 24) | 3;
        let got = packed_to_rgba(packed_std3, fallback, &colors);
        assert_ne!(got, fallback, "a palette colour must NOT fall back to the default ink");
        assert_eq!(got, Rgba([170, 0, 0, 255]), "Standard(3) → red resolves to concrete RGB");
        // And the full blit through build_chrome_canvas carries it: a space-only
        // run has no ink, so probe an inked glyph's fg by asserting SOME cell pixel
        // is the run's red.
        let win = px_text_grid_item("N", 0, packed_std3, 0);
        let c = build_chrome_canvas(&[&win], (8, 8), Rgba([200, 200, 200, 255]), Rgba([0, 0, 0, 255]), &colors);
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == Rgba([170, 0, 0, 255]))),
            "the compass glyph blits in its own red, not the default fg"
        );
    }

    #[test]
    fn native_extent_ignores_unresolved_size_sentinel() {
        // SQ-0481: a real 320×200 window plus a bogus window whose x_size leaked
        // the -2 sentinel (0xFFFE ≈ 65534). The sentinel must NOT balloon the
        // native extent (and thus the raster canvas allocation) — the real
        // 320×200 screen size stands.
        let real = || PositionedWindow { x_px: 0, y_px: 0, w_px: 320, h_px: 200, ..buffer_item(0, true) };
        let bogus = PositionedWindow { x_px: 0, y_px: 0, w_px: 0xFFFE, h_px: 200, ..grid_item(0) };
        assert_eq!(native_extent(&[real(), bogus]), (320, 200), "sentinel width excluded");
        // A sentinel HEIGHT is likewise ignored on its axis.
        let bogus_h = PositionedWindow { x_px: 0, y_px: 0, w_px: 320, h_px: 0xFFFD, ..grid_item(0) };
        assert_eq!(native_extent(&[real(), bogus_h]), (320, 200), "sentinel height excluded");
    }

    #[test]
    fn story_text_input_continues_the_prompt_row() {
        // SQ-0470a: the live input sits on the game's kept ">" prompt row,
        // appended right after it — NOT a fresh row below it.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        let main = MainText {
            lines: vec!["Room desc.".into(), ">".into()],
            input: "go".into(),
            cursor_col: 2,
            awaiting: true,
            floats: vec![],
        };
        let mut canvas = RgbaImage::new(20 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 20, 5, Rgba([255, 255, 255, 255]));
        // ">" is on row 1; input "go" appends after it at cols 1 and 2.
        assert!(cell_has_ink(&canvas, 1, 1), "input 'g' on the prompt row, after '>'");
        assert!(cell_has_ink(&canvas, 2, 1), "input 'o' on the prompt row");
        // Caret block after the input: col = 1 (\">\".len) + 2 (cursor) = 3.
        assert!(cell_has_ink(&canvas, 3, 1), "caret after the input on the prompt row");
        // The row BELOW the prompt is empty — input no longer drops a row.
        assert!(!(0..20).any(|col| cell_has_ink(&canvas, col, 2)), "nothing on the row below the prompt");
    }

    #[test]
    fn story_text_input_after_newline_starts_a_clean_row() {
        // When the transcript ended on a newline the last line is empty, so the
        // input starts a clean row of its own (col 0) — the universal rule that
        // makes SQ-0470a correct for both prompt and non-prompt endings.
        let cell_has_ink = |c: &RgbaImage, col: u32, row: u32| -> bool {
            (0..FONT_H).any(|dy| (0..FONT_W).any(|dx| c.get_pixel(col * FONT_W + dx, row * FONT_H + dy)[3] > 0))
        };
        let main = MainText {
            lines: vec!["Prose line.".into(), String::new()],
            input: "x".into(),
            cursor_col: 1,
            awaiting: true,
            floats: vec![],
        };
        let mut canvas = RgbaImage::new(20 * FONT_W, 5 * FONT_H);
        draw_story_text(&mut canvas, &main, 0, 0, 20, 5, Rgba([255, 255, 255, 255]));
        assert!(cell_has_ink(&canvas, 0, 1), "input on the empty last row at col 0");
        assert!(!(0..20).any(|col| cell_has_ink(&canvas, col, 2)), "not the row below");
    }

    #[test]
    fn story_text_scrolled_float_is_cropped_not_pinned() {
        // A float whose anchor scrolled above the view (row = -1) draws only its
        // remaining rows, cropped from its own top (one FONT_H = 16px row).
        let mut img = RgbaImage::new(8, 32);
        for y in 0..32 {
            // Top row (y<16) green, bottom row (y>=16) blue — the visible part,
            // after cropping the scrolled-off top FONT_H row, must be blue.
            let c = if y < 16 { Rgba([0, 200, 0, 255]) } else { Rgba([0, 0, 200, 255]) };
            for x in 0..8 { img.put_pixel(x, y, c); }
        }
        let main = MainText {
            lines: vec!["XXXX".into()],
            input: String::new(), cursor_col: 0, awaiting: false,
            floats: vec![RasterFloat { row: -1, rows: 2, reserve_cols: 2, text_col: 2, img_col: 0, img: Arc::new(img) }],
        };
        let mut canvas = RgbaImage::new(10 * FONT_W, 3 * FONT_H);
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
        // row 1, col 2 in a 3-col grid.
        cells[3 + 2] = GridCell { ch: 'A', style: 0, fg: 0, bg: 0, link: 0, glk_style: 0 };
        let win = PositionedWindow {
            x: 0, y: 0, w: 3, h: 2, x_px: 10, y_px: 4, w_px: 24, h_px: 32, left_margin: 0, right_margin: 0,
            node: WinNode::Grid(GridWindow {
                cols: 3, rows: 2, cells, active_rows: 2, cursor: (0, 0), cursor_active: false,
                border: BorderPref::Unspecified, bg: None, fg: None, reverse: false,
                px_texts: Vec::new(),
            }),
        };
        let chrome: Vec<&PositionedWindow> = vec![&win];
        let fg = Rgba([0, 255, 255, 255]);
        let c = build_chrome_canvas(&chrome, (40, 40), fg, Rgba([0, 0, 0, 255]), &colors());
        // cell (col=2,row=1) native px box: x = 10 + 2·FONT_W(8) = 26..34,
        // y = 4 + 1·FONT_H(16) = 20..36 (non-square 8×16 cell, SQ-0479).
        assert!(
            (26..34).any(|x| (20..36).any(|y| *c.get_pixel(x, y) == fg)),
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
    fn px_text_reverse_inherited_over_art_draws_dark_ink_no_block() {
        // The run never chose an explicit colour (fg=bg=0/Default) and sits OVER
        // opaque frame art: reverse video must NOT paint a block — Zork0's ribbon
        // labels print in reverse with inherited colours and the original shows dark
        // ink directly ON the banner art (a block would erase it, the black-box
        // regression the user hit). A blank glyph therefore leaves the art
        // untouched; an inked glyph draws in default_bg (dark) on the art. (SQ-0487
        // keeps this by testing the canvas is opaque behind the run.)
        let default_fg = Rgba([10, 20, 30, 255]);
        let default_bg = Rgba([40, 50, 60, 255]);
        let art_color = Rgba([200, 150, 100, 255]);
        // An opaque 8×8 art window behind the run (pass 1), then the reverse run.
        let art = graphics_window(0, 0, 8, 8, image::RgbaImage::from_pixel(8, 8, art_color));
        let blank = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&art, &blank];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert_eq!(*c.get_pixel(4, 4), art_color, "blank reverse glyph over art leaves the art (no block)");
        let inked = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&art, &inked];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "reverse ink over art draws in the themed default_bg (dark on the art)"
        );
    }

    #[test]
    fn px_text_reverse_inherited_over_clear_bg_paints_the_highlight_block() {
        // SQ-0487: the same inherited-colour reverse run over a CLEAR background
        // (Shogun's boot-menu selection bar — no frame art behind it) MUST paint the
        // swapped highlight block: a solid default_fg bar with default_bg ink. A
        // blank gap run between words fills its whole cell with the bar colour, so
        // the selection bar reads solid (not moth-eaten).
        let default_fg = Rgba([210, 210, 210, 255]);
        let default_bg = Rgba([12, 12, 12, 255]);
        // A blank reverse run (an inter-word gap) over the transparent canvas fills
        // its cell with the bar colour (default_fg).
        let gap = px_text_grid_item(" ", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&gap];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(*c.get_pixel(x, y), default_fg, "gap cell filled with the bar colour at ({x},{y})");
            }
        }
        // An inked reverse glyph paints the bar (default_fg) with dark (default_bg) ink.
        let glyph = px_text_grid_item("X", 1, 0, 0);
        let chrome: Vec<&PositionedWindow> = vec![&glyph];
        let c = build_chrome_canvas(&chrome, (8, 8), default_fg, default_bg, &colors());
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_fg)),
            "the highlight bar (default_fg) is painted behind the glyph"
        );
        assert!(
            (0..8).any(|x| (0..8).any(|y| *c.get_pixel(x, y) == default_bg)),
            "the glyph ink is drawn in default_bg (dark on the bright bar)"
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
                let in_side = !(8..32).contains(&x);
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
