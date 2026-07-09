pub mod aux_dialog;
pub mod config_screen;
pub mod upper_window;
pub mod dialog;
pub mod paneframe;
pub mod filebrowser;
pub mod gallery;
pub mod graphics;
pub mod hints_panel;
pub mod history;
pub mod inline_image;
pub mod hotkeys;
pub mod inspector;
pub mod inventory_dock;
pub mod launch_dialog;
pub mod map;
pub mod quit_dialog;
pub mod glyph_picker;
pub mod reset_dialog;
pub mod style_editor;
pub mod room_info;
pub mod saves;
pub mod screen;
pub mod scroll;
pub mod tidy_panel;
pub mod transcript;
pub mod verbmenu;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use zvm::screen::{ZColour, grey_rgb, rgb15_to_888};

use crate::colors::ColorScheme;

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

/// Map a Z-machine colour to a ratatui `Color` via the user's theme palette.
///
/// - `ZColour::Default` → `Color::Reset` (let the terminal decide)
/// - `ZColour::Standard(2..=9)` → `scheme.palette[n - 2]` (ANSI colours routed
///   through the active theme so the user's Ghostty palette applies)
/// - `ZColour::Standard(10..=12)` → fixed grey RGB via `grey_rgb(n)`
/// - `ZColour::True(v)` → exact 15-bit RGB via `rgb15_to_888(v)`
pub(crate) fn resolve_zcolour(c: ZColour, scheme: &ColorScheme) -> Color {
    match c {
        ZColour::Default => Color::Reset,
        ZColour::Standard(n @ 2..=9) => scheme.palette[(n - 2) as usize],
        ZColour::Standard(n) => {
            let (r, g, b) = grey_rgb(n);
            Color::Rgb(r, g, b)
        }
        ZColour::True(v) => {
            let (r, g, b) = rgb15_to_888(v);
            Color::Rgb(r, g, b)
        }
        ZColour::True24(v) => {
            Color::Rgb(((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)
        }
    }
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
    use ratatui::style::{Color, Modifier, Style};

    #[test]
    fn resolve_zcolour_maps_palette_grey_true_default() {
        use zvm::screen::ZColour;
        let mut scheme = ColorScheme::default();
        scheme.palette[1] = Color::Rgb(10, 20, 30); // "red" slot
        assert_eq!(resolve_zcolour(ZColour::Standard(3), &scheme), Color::Rgb(10, 20, 30));
        assert_eq!(resolve_zcolour(ZColour::Default, &scheme), Color::Reset);
        assert_eq!(resolve_zcolour(ZColour::Standard(11), &scheme), Color::Rgb(0x80, 0x80, 0x80));
        assert_eq!(resolve_zcolour(ZColour::True(0x7FFF), &scheme), Color::Rgb(255, 255, 255));
        // True24 carries an exact 24-bit RGB (Glulx stylehint colour).
        assert_eq!(resolve_zcolour(ZColour::True24(0x0011_2233), &scheme), Color::Rgb(0x11, 0x22, 0x33));
    }

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
