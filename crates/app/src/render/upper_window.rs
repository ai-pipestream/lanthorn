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
use crate::render::paneframe::{draw_pane_frame, BorderStyle};

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
/// - `colors`: resolved color scheme.
/// - `area`: target rectangle in the buffer.
///
/// Returns the number of terminal rows consumed (0 when `upper_rows == 0`).
pub fn draw_grid(
    upper: &UpperWindow,
    upper_rows: u16,
    cursor: (u16, u16),
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

    // How many terminal rows does the border frame consume?
    let border_overhead: u16 = if border_style != BorderStyle::None { 2 } else { 0 };

    // Total terminal rows needed: grid rows + optional border.
    // Clamp to the available area height.
    let needed = upper_rows.saturating_add(border_overhead).min(area.height);

    // Carve out the top `needed` rows of area for the upper window.
    let uw_area = Rect::new(area.x, area.y, area.width, needed);

    // Draw the optional border and get the inner content rect.
    let frame = draw_pane_frame(buf, uw_area, border_style, border_color);
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

    needed
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the Z-machine upper-window grid into the top of `area`, returning the
/// number of story-pane rows consumed (0 when the upper window is inactive).
pub fn draw_upper_window(
    machine: &Machine,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
) -> u16 {
    let screen = &machine.screen;
    draw_grid(
        &screen.upper,
        screen.upper_window_rows,
        (screen.cursor_row, screen.cursor_col),
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

        let consumed = draw_grid(&upper, 2, (1, 1), &colors_no_border, area, &mut buf);

        // Should consume exactly 2 rows (grid height, no border).
        assert_eq!(consumed, 2, "consumed rows should equal upper_window_rows");

        // 'H' should appear at (0,0), 'I' at (1,0) — i.e. col 0, col 1 of row 0.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((1, 0)).unwrap().symbol(), "I");
    }

    #[test]
    fn returns_zero_when_upper_window_inactive() {
        let upper = make_upper_hi();
        let colors = make_colors();
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        // upper_rows = 0 means inactive.
        let consumed = draw_grid(&upper, 0, (1, 1), &colors, area, &mut buf);
        assert_eq!(consumed, 0);
    }

    #[test]
    fn border_adds_overhead() {
        let upper = make_upper_hi();
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::Single;
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 2, (1, 1), &colors, area, &mut buf);

        // 2 grid rows + 2 border rows = 4 total.
        assert_eq!(consumed, 4);
        // Grid content starts at row 1 (inside border).
        assert_eq!(buf.cell((1, 1)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((2, 1)).unwrap().symbol(), "I");
    }

    #[test]
    fn viewport_scrolls_when_cursor_exceeds_height() {
        let mut upper = UpperWindow::default();
        upper.resize(5, 5);
        // Put 'A' at row 5 (last row, 1-based).
        upper.put(5, 1, 'A', 0);

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;

        // Only 3 rows available, but cursor is at row 5.
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 5, (5, 1), &colors, area, &mut buf);
        assert_eq!(consumed, 3);

        // Row offset = cursor_row-1 - (height-1) = 4 - 2 = 2.
        // 'A' is at grid row 5, displayed at terminal row 2 (0-based within content).
        // So it should appear at buffer row 2.
        assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "A");
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

        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), &colors, area, &mut buf);

        let x_cell = buf.cell((0, 0)).unwrap();
        assert!(x_cell.modifier.contains(Modifier::BOLD), "X should be bold");

        let y_cell = buf.cell((1, 0)).unwrap();
        assert!(y_cell.modifier.contains(Modifier::REVERSED), "Y should be reversed");
    }
}
