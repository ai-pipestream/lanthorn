//! The floating tooltip box: a few lines of text, drawn opaquely on top of
//! whatever is underneath, anchored to the thing the pointer is on.
//!
//! Lifted out of `render::debug_panel` when the border controls needed the same
//! box (SQ-1123). One implementation, so the debugger's value tip and a border
//! control's hint clamp, frame and paint identically — and both read the same
//! `tooltip.background` / `tooltip.border` selectors (§2d).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::draw_str_clipped;
use super::paneframe::{draw_pane_frame, BorderStyle, PaneGlyphs};
use crate::theme::resolve::Theme;

/// Draw `lines` as a floating box anchored at `(col, row)`.
///
/// **Placement.** The preferred spot is one row BELOW the anchor and starting at
/// its column, so the box never covers the cell the pointer is on — the thing
/// the reader is asking about stays visible while its explanation appears.
///
/// **Edges.** The box is clamped inside `area`: it slides LEFT when it would
/// overrun the right edge, and FLIPS to sit above the anchor when it would
/// overrun the bottom; then both origins are clamped up to `area`'s own. A box
/// that cannot fit in `area` at all is skipped rather than drawn partially, so
/// this never panics on a small pane.
///
/// Returns the rect painted, or `None` when nothing was drawn.
pub fn draw_tip(
    buf: &mut Buffer,
    area: Rect,
    col: u16,
    row: u16,
    lines: &[String],
    theme: &Theme,
) -> Option<Rect> {
    let inner = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let n = lines.len() as u16;
    if n == 0 {
        return None;
    }

    let style = theme.get("tooltip.background").style;
    // Optional frame (§2d): borderless by default; a themed `tooltip.border`
    // style wraps the box in a frame (colour + glyphs from that selector).
    let border = theme.get("tooltip.border");
    let box_style = border.border.unwrap_or(BorderStyle::None);
    let bordered = !matches!(box_style, BorderStyle::None);

    // Content is `inner` wide with one space of padding each side; a frame (when
    // set) adds one more cell all around.
    let pad_w = inner + 2;
    let (w, h) = if bordered { (pad_w + 2, n + 2) } else { (pad_w, n) };
    if area.width < w || area.height < h {
        return None;
    }

    let mut x = col;
    let mut y = row + 1;
    if x + w > area.right() {
        x = area.right().saturating_sub(w);
    }
    if y + h > area.bottom() {
        y = row.saturating_sub(h);
    }
    x = x.max(area.x);
    y = y.max(area.y);

    let box_rect = Rect::new(x, y, w, h);
    // Reset every cell the box covers before drawing: draw_char_clipped PATCHES
    // cell styles, so a modifier already on what is underneath (e.g. the
    // UNDERLINED on a clickable operand) would otherwise bleed through the
    // tooltip. A clean reset makes the box fully opaque.
    for yy in box_rect.y..box_rect.bottom() {
        for xx in box_rect.x..box_rect.right() {
            if let Some(cell) = buf.cell_mut((xx, yy)) {
                cell.reset();
            }
        }
    }
    // Fill the whole box with the tooltip background.
    let pad: String = " ".repeat(w as usize);
    for ry in box_rect.y..box_rect.bottom() {
        draw_str_clipped(buf, x, ry, &pad, style, box_rect);
    }
    // Frame in tooltip.border's colour, then position the text inside it.
    let (tx, ty) = if bordered {
        draw_pane_frame(buf, box_rect, box_style, &PaneGlyphs::default(), border.style);
        (x + 1, y + 1)
    } else {
        (x, y)
    };
    for (i, line) in lines.iter().enumerate() {
        draw_str_clipped(buf, tx + 1, ty + i as u16, line, style, box_rect);
    }
    Some(box_rect)
}
