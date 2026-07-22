pub mod aux_dialog;
pub mod bitfont;
pub mod config_screen;
pub mod upper_window;
pub mod debug_panel;
pub mod dialog;
pub mod dump_harness;
pub mod game_over_dialog;
pub mod paneframe;
pub mod panel;
pub mod filebrowser;
pub mod graphics;
pub mod hintbar;
pub mod hints_panel;
pub mod history;
pub mod inline_image;
pub mod hotkeys;
pub mod inspector;
pub mod inventory_dock;
pub mod launch_dialog;
pub mod map;
pub mod quit_dialog;
pub mod reset_dialog;
pub mod save_name_dialog;
pub mod text_entry_dialog;
pub mod confirm_delete_dialog;
pub mod room_info;
pub mod saves;
pub mod file_picker;
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

/// Resolve one colour channel (fg or bg) through the per-Glk-style theme layer
/// (SQ-0331). Returns the colour to explicitly set, or `None` to leave the base
/// element style's channel untouched (so an unset element channel keeps
/// inheriting — preserving byte-identity for the all-Normal Z-machine path).
///
/// - `game_set`: the run/cell's own resolved colour when the game set one (else
///   `None`). Gated by `honor`: when `honor` is off the game layer is IGNORED
///   entirely (garglk "disable game modifications"), NOT merely ranked below the
///   slot.
/// - `slot`: the theme's `glk_styles[wintype][style]` channel (USER/THEME
///   styling — applies in BOTH gate states).
/// - `base`: the per-app-element base channel (`transcript` / `upper_window`).
///
/// Chain: `honor ? game_set.or(slot).or(base) : slot.or(base)`.
pub(crate) fn resolve_glk_channel(
    game_set: Option<Color>,
    slot: Option<Color>,
    base: Option<Color>,
    honor: bool,
) -> Option<Color> {
    if honor {
        game_set.or(slot).or(base)
    } else {
        slot.or(base)
    }
}

/// The 11 standard Glk style names (index = `glk_Style` number), matching the
/// registry `glk.buffer.<name>` / `glk.grid.<name>` selectors (SQ-0309 §3).
pub(crate) const GLK_STYLE_NAMES: [&str; 11] = [
    "normal", "emphasized", "preformatted", "header", "subheader", "alert",
    "note", "blockquote", "input", "user1", "user2",
];

/// The registry theme's modifier delta for one Glk style slot — the canonical
/// typography (Emphasized→italic, Header/Subheader/Alert→bold, Note→italic).
/// `grid` selects the grid (status/upper-window) row vs the text-buffer row.
pub(crate) fn glk_theme_modifiers(
    scheme: &crate::colors::ColorScheme,
    grid: bool,
    glk: usize,
) -> ratatui::style::Modifier {
    let name = GLK_STYLE_NAMES.get(glk).copied().unwrap_or("normal");
    let win = if grid { "grid" } else { "buffer" };
    scheme.theme.get(&format!("glk.{win}.{name}")).style.add_modifier
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
    fn resolve_glk_channel_honours_chain_and_gate() {
        let (red, blue, green) = (Color::Red, Color::Blue, Color::Green);
        // honor ON: game > slot > base.
        assert_eq!(resolve_glk_channel(Some(red), Some(blue), Some(green), true), Some(red));
        assert_eq!(resolve_glk_channel(None, Some(blue), Some(green), true), Some(blue));
        assert_eq!(resolve_glk_channel(None, None, Some(green), true), Some(green));
        assert_eq!(resolve_glk_channel(None, None, None, true), None);
        // honor OFF: game colour IGNORED entirely; slot > base still apply.
        assert_eq!(resolve_glk_channel(Some(red), Some(blue), Some(green), false), Some(blue));
        assert_eq!(resolve_glk_channel(Some(red), None, Some(green), false), Some(green));
        assert_eq!(resolve_glk_channel(Some(red), None, None, false), None);
    }

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
