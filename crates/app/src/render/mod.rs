pub mod gallery;
pub mod hotkeys;
pub mod inspector;
pub mod map;
pub mod room_info;
pub mod saves;
pub mod tidy_panel;
pub mod transcript;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

// ── Shared clipped drawing helpers ────────────────────────────────────────────

/// Write a single char into the buffer, clipped to `area`.
pub fn draw_char_clipped(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    ch: char,
    style: Style,
    area: Rect,
) {
    if x < area.x || x >= area.right() || y < area.y || y >= area.bottom() {
        return;
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        let mut s = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut s)).set_style(style);
    }
}

/// Write a string into the buffer starting at (x, y), clipped to `area` width.
pub fn draw_str_clipped(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    s: &str,
    style: Style,
    area: Rect,
) {
    if y < area.y || y >= area.bottom() {
        return;
    }
    for (cx, ch) in (x..).zip(s.chars()) {
        if cx >= area.right() {
            break;
        }
        draw_char_clipped(buf, cx, y, ch, style, area);
    }
}

/// Like `draw_char_clipped` but accepts signed screen coordinates, so callers
/// working in a virtual (scroll-translated) space can pass positions that fall
/// off the left/top of `area` and have them clipped instead of underflowing.
pub fn put_char(buf: &mut Buffer, x: i32, y: i32, ch: char, style: Style, area: Rect) {
    if x < area.x as i32 || x >= area.right() as i32 || y < area.y as i32 || y >= area.bottom() as i32 {
        return;
    }
    if let Some(cell) = buf.cell_mut((x as u16, y as u16)) {
        let mut s = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut s)).set_style(style);
    }
}

/// Like `draw_str_clipped` but accepts a signed start coordinate (see `put_char`).
pub fn put_str(buf: &mut Buffer, x: i32, y: i32, s: &str, style: Style, area: Rect) {
    if y < area.y as i32 || y >= area.bottom() as i32 {
        return;
    }
    for (i, ch) in s.chars().enumerate() {
        put_char(buf, x + i as i32, y, ch, style, area);
    }
}
