/// Render the Z-machine upper-window grid atop the story pane transcript.
///
/// Public entry point: `draw_upper_window` — reads from `Machine` and delegates
/// to the testable `draw_grid` helper.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use zvm::cpu::exec::Machine;
use zvm::screen::UpperWindow;

use crate::colors::ColorScheme;
use crate::render::paneframe::{draw_framed, BorderStyle};

// ── Style helpers ─────────────────────────────────────────────────────────────

/// Apply Z-machine text_style bits on top of a base `Style`.
///
/// ZMSD §8.7.2 operand values: 1 = reverse-video, 2 = bold, 4 = italic,
/// 8 = fixed-pitch.  The VM stores the raw game operand verbatim, so test
/// the exact value bits (0x01 for reverse, 0x02 for bold).
fn apply_text_style(base: Style, text_style: u8) -> Style {
    let mut s = base;
    if text_style & 0x02 != 0 {
        s = s.add_modifier(Modifier::BOLD);
    }
    if text_style & 0x01 != 0 {
        s = s.add_modifier(Modifier::REVERSED);
    }
    s
}

// ── Core grid renderer ────────────────────────────────────────────────────────

/// Draw the upper-window grid into `area`.
///
/// - `upper`: the grid to render (from `machine.screen.upper`).
/// - `upper_rows`: the active row count (`machine.screen.upper_window_rows`).
/// - `cursor`: 1-based (row, col) of the upper-window cursor.
/// - `show_cursor`: when true, mark the cursor cell (e.g. while the game is
///   awaiting input in the upper window) so forms show where typing lands.
/// - `colors`: resolved color scheme.
/// - `area`: target rectangle in the buffer.
///
/// Returns the number of terminal rows consumed (0 when `upper_rows == 0`).
pub fn draw_grid(
    upper: &UpperWindow,
    upper_rows: u16,
    cursor: (u16, u16),
    show_cursor: bool,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
) -> u16 {
    if upper_rows == 0 || area.height == 0 || area.width == 0 {
        return 0;
    }

    let border_style = colors.virtual_window_border;
    let content_style = colors.upper_window;
    let border_color = colors.upper_window_border;

    // How many terminal rows does the border frame consume? Derive from which
    // horizontal sides are actually present (per-side aware), so a top/bottom-only
    // or left/right-only frame reserves the right number of rows.
    let sides = colors.upper_window_border_sides;
    let border_overhead: u16 =
        (if sides.top != BorderStyle::None { 1 } else { 0 })
        + (if sides.bottom != BorderStyle::None { 1 } else { 0 });

    // Total terminal rows needed: grid rows + optional border.
    // Clamp to the available area height.
    let needed = upper_rows.saturating_add(border_overhead).min(area.height);

    // The upper window is the game's screen (`upper.cols` wide) — NOT the pane.
    // Size the region to the game screen width (+ side borders) and CENTER it in
    // the pane, so a game that centers its own content (e.g. Bureaucracy's
    // full-width forms / status) lines up under our border instead of being
    // stretched to the pane edge. When the pane is narrower than the game screen,
    // use the full pane width and left-align (the col-offset scroll below handles
    // the overflow).
    let border_cols: u16 =
        (if sides.left != BorderStyle::None { 1 } else { 0 })
        + (if sides.right != BorderStyle::None { 1 } else { 0 });
    let uw_w = upper.cols.saturating_add(border_cols).min(area.width).max(1);
    let x_off = area.width.saturating_sub(uw_w) / 2;

    // Carve out the centered top region for the upper window.
    let uw_area = Rect::new(area.x + x_off, area.y, uw_w, needed);

    // Draw the optional border and get the inner content rect.
    let frame = draw_framed(buf, uw_area, border_style, colors.upper_window_border_sides, border_color, false);
    let content = frame.content;

    if content.height == 0 || content.width == 0 {
        return needed;
    }

    // Viewport auto-follow: scroll so cursor stays visible.
    // cursor is 1-based; convert to 0-based for arithmetic.
    let (crow, ccol) = (
        cursor.0.saturating_sub(1),
        cursor.1.saturating_sub(1),
    );
    // Row viewport offset: scroll down so cursor row is visible.
    let row_offset: u16 = if crow >= content.height {
        crow.saturating_sub(content.height - 1)
    } else {
        0
    };
    // Col viewport offset: scroll right so cursor col is visible.
    let col_offset: u16 = if ccol >= content.width {
        ccol.saturating_sub(content.width - 1)
    } else {
        0
    };

    // Fill content area with background style.
    for dy in 0..content.height {
        for dx in 0..content.width {
            let bx = content.x + dx;
            let by = content.y + dy;
            if let Some(cell) = buf.cell_mut((bx, by)) {
                cell.set_symbol(" ").set_style(content_style);
            }
        }
    }

    // Render each visible grid cell.
    // Grid rows/cols are 1-based; viewport offsets are 0-based.
    for dy in 0..content.height {
        let grid_row = dy + row_offset + 1; // 1-based
        if grid_row > upper_rows {
            break;
        }
        for dx in 0..content.width {
            let grid_col = dx + col_offset + 1; // 1-based
            if grid_col > upper.cols {
                break;
            }
            let cell = upper.cell(grid_row, grid_col);
            let bx = content.x + dx;
            let by = content.y + dy;
            if let Some(buf_cell) = buf.cell_mut((bx, by)) {
                let style = apply_text_style(content_style, cell.style);
                let mut ch_buf = [0u8; 4];
                buf_cell.set_symbol(cell.ch.encode_utf8(&mut ch_buf)).set_style(style);
            }
        }
    }

    // Cursor: toggle reverse-video on the cell under the (offset-adjusted)
    // cursor so it stays visible over both normal and already-reversed cells.
    if show_cursor && crow >= row_offset && ccol >= col_offset {
        let cur_dy = crow - row_offset;
        let cur_dx = ccol - col_offset;
        if cur_dy < content.height && cur_dx < content.width {
            if let Some(c) = buf.cell_mut((content.x + cur_dx, content.y + cur_dy)) {
                if c.modifier.contains(Modifier::REVERSED) {
                    c.modifier.remove(Modifier::REVERSED);
                } else {
                    c.modifier.insert(Modifier::REVERSED);
                }
            }
        }
    }

    needed
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the Z-machine upper-window grid into the top of `area`, returning the
/// number of story-pane rows consumed (0 when the upper window is inactive).
///
/// `char_mode` is true when the game is awaiting a keypress; combined with the
/// upper window being the current window, it decides whether to show the
/// cursor (so in-place forms reveal where typed characters land).
pub fn draw_upper_window(
    machine: &Machine,
    char_mode: bool,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
) -> u16 {
    let screen = &machine.screen;
    let show_cursor = char_mode && screen.current_window == 1;
    draw_grid(
        &screen.upper,
        screen.upper_window_rows,
        (screen.cursor_row, screen.cursor_col),
        show_cursor,
        colors,
        area,
        buf,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{buffer::Buffer, layout::Rect};
    use zvm::screen::UpperWindow;

    fn make_colors() -> ColorScheme {
        ColorScheme::terminal_default()
    }

    /// Build a 2-row × 5-col UpperWindow with "HI" starting at (1,1).
    fn make_upper_hi() -> UpperWindow {
        let mut w = UpperWindow::default();
        w.resize(2, 5);
        w.put(1, 1, 'H', 0);
        w.put(1, 2, 'I', 0);
        w
    }

    #[test]
    fn draws_grid_cells_and_consumes_rows() {
        let upper = make_upper_hi();
        let colors = make_colors();
        // Area taller than the grid so no scrolling needed.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        // Use BorderStyle::None to avoid border overhead for simplicity.
        let mut colors_no_border = colors.clone();
        colors_no_border.virtual_window_border = BorderStyle::None;
        colors_no_border.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors_no_border, area, &mut buf);

        // Should consume exactly 2 rows (grid height, no border).
        assert_eq!(consumed, 2, "consumed rows should equal upper_window_rows");

        // cols=5 is centered in the 20-wide pane (no border): x_off = (20-5)/2 = 7.
        assert_eq!(buf.cell((7, 0)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((8, 0)).unwrap().symbol(), "I");
    }

    #[test]
    fn upper_window_centered_at_game_screen_width_not_pane_width() {
        // Regression (bug #79): a game-screen-width upper window must render at its
        // own width centered in a wider pane — not stretched to the pane, which
        // made Bureaucracy's border too wide and its centered content off-place.
        let mut upper = UpperWindow::default();
        upper.resize(1, 10); // game screen is 10 cols wide
        upper.put(1, 1, 'A', 0);
        upper.put(1, 10, 'Z', 0); // content spans the full game screen

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Pane is 30 wide; the 10-col upper window should center: x_off=(30-10)/2=10.
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf);
        assert_eq!(buf.cell((10, 0)).unwrap().symbol(), "A", "left edge of the game screen at x=10");
        assert_eq!(buf.cell((19, 0)).unwrap().symbol(), "Z", "right edge at x=19 (10..19)");
        // Nothing drawn outside the centered 10-col region.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), " ", "no content stretched to the pane left edge");
    }

    #[test]
    fn returns_zero_when_upper_window_inactive() {
        let upper = make_upper_hi();
        let colors = make_colors();
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        // upper_rows = 0 means inactive.
        let consumed = draw_grid(&upper, 0, (1, 1), false, &colors, area, &mut buf);
        assert_eq!(consumed, 0);
    }

    #[test]
    fn border_adds_overhead() {
        let upper = make_upper_hi();
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::Single;
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf);

        // 2 grid rows + 2 border rows = 4 total.
        assert_eq!(consumed, 4);
        // cols=5 + 2 side borders = 7, centered in 20: x_off=(20-7)/2=6, content.x=7.
        // Grid content starts at row 1 (inside the top border).
        assert_eq!(buf.cell((7, 1)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((8, 1)).unwrap().symbol(), "I");
    }

    #[test]
    fn viewport_scrolls_when_cursor_exceeds_height() {
        let mut upper = UpperWindow::default();
        upper.resize(5, 5);
        // Put 'A' at row 5 (last row, 1-based).
        upper.put(5, 1, 'A', 0);

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Only 3 rows available, but cursor is at row 5.
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 5, (5, 1), false, &colors, area, &mut buf);
        assert_eq!(consumed, 3);

        // Row offset = cursor_row-1 - (height-1) = 4 - 2 = 2.
        // 'A' is at grid row 5, displayed at terminal row 2 (0-based within content).
        // cols=5 centered in 10 (no border): x_off=(10-5)/2=2, so col 2.
        assert_eq!(buf.cell((2, 2)).unwrap().symbol(), "A");
    }

    #[test]
    fn bold_and_reverse_style_applied() {
        let mut upper = UpperWindow::default();
        upper.resize(1, 3);
        // ZMSD §8.7.2 operand values: 1 = reverse-video, 2 = bold
        upper.put(1, 1, 'X', 0x02); // bold
        upper.put(1, 2, 'Y', 0x01); // reverse-video

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3.
        let x_cell = buf.cell((3, 0)).unwrap();
        assert!(x_cell.modifier.contains(Modifier::BOLD), "X should be bold");

        let y_cell = buf.cell((4, 0)).unwrap();
        assert!(y_cell.modifier.contains(Modifier::REVERSED), "Y should be reversed");
    }

    #[test]
    fn cursor_cell_is_marked_when_show_cursor() {
        let mut upper = UpperWindow::default();
        upper.resize(2, 5);
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        let area = Rect::new(0, 0, 10, 3);

        // cols=5 centered in 10 (no border): x_off=2; cursor (row 2, col 3) →
        // content (1,2) → buffer (4,1).
        // With show_cursor=false the cursor cell is a plain (non-reversed) space.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), false, &colors, area, &mut buf);
        assert!(
            !buf.cell((4, 1)).unwrap().modifier.contains(Modifier::REVERSED),
            "no cursor mark when show_cursor=false"
        );

        // With show_cursor=true the cell under (row 2, col 3) is reversed.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), true, &colors, area, &mut buf);
        assert!(
            buf.cell((4, 1)).unwrap().modifier.contains(Modifier::REVERSED),
            "cursor cell should be reverse-video when show_cursor=true"
        );
    }
}
