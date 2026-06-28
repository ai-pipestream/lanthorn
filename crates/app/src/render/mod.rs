pub mod aux_dialog;
pub mod config_screen;
pub mod upper_window;
pub mod dialog;
pub mod paneframe;
pub mod filebrowser;
pub mod gallery;
pub mod hints_panel;
pub mod history;
pub mod hotkeys;
pub mod inspector;
pub mod launch_dialog;
pub mod map;
pub mod quit_dialog;
pub mod glyph_picker;
pub mod reset_dialog;
pub mod style_editor;
pub mod room_info;
pub mod saves;
pub mod screen;
pub mod tidy_panel;
pub mod transcript;
pub mod verbmenu;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

// ── Shared text-style mapping ─────────────────────────────────────────────────

/// Layer Z-machine text-style bits (ZMSD §8.7.1: 1=reverse, 2=bold, 4=italic,
/// 8=fixed-pitch) over a base style. Fixed-pitch is ignored (already monospaced).
pub(crate) fn apply_text_style(base: Style, bits: u8) -> Style {
    let mut s = base;
    if bits & 0x02 != 0 {
        s = s.add_modifier(Modifier::BOLD);
    }
    if bits & 0x01 != 0 {
        s = s.add_modifier(Modifier::REVERSED);
    }
    if bits & 0x04 != 0 {
        s = s.add_modifier(Modifier::ITALIC);
    }
    s
}

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

#[cfg(test)]
mod text_style_tests {
    use super::*;
    use ratatui::style::{Modifier, Style};

    #[test]
    fn apply_text_style_maps_all_bits() {
        let b = Style::default();
        assert!(apply_text_style(b, 0x02).add_modifier.contains(Modifier::BOLD));
        assert!(apply_text_style(b, 0x01).add_modifier.contains(Modifier::REVERSED));
        assert!(apply_text_style(b, 0x04).add_modifier.contains(Modifier::ITALIC));
        // fixed-pitch (0x08) adds nothing; 0 is a no-op
        assert_eq!(apply_text_style(b, 0x08), b);
        assert_eq!(apply_text_style(b, 0x00), b);
        // composes: bold+italic
        let bi = apply_text_style(b, 0x06).add_modifier;
        assert!(bi.contains(Modifier::BOLD) && bi.contains(Modifier::ITALIC));
    }
}
