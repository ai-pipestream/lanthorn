/// Render the engine's text-grid (upper) window atop the story pane transcript.
///
/// Public entry point: `draw_upper_window` — reads a neutral [`GridWindow`] from
/// the engine's `ScreenModel` and delegates to the testable `draw_grid` helper.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::colors::ColorScheme;
use crate::engine::{GridCell, GridWindow};
use crate::render::paneframe::{draw_framed, BorderStyle};

/// Resolve a grid cell's game colour into a ratatui [`Style`], honoring
/// reverse-video by swapping fg/bg exactly once.
///
/// Reverse video (style bit `0x01`) is realised as a single fg/bg swap here —
/// NOT via the ratatui `REVERSED` modifier — so the upper window has one
/// consistent reverse mechanism. The remaining style bits (bold/italic) are
/// applied with the reverse bit masked out so `apply_text_style` does not also
/// add `REVERSED` (which would double-swap and cancel).
fn cell_style(cell: zvm::screen::Cell, scheme: &ColorScheme, honor_game_colours: bool) -> Style {
    let mut fg = if honor_game_colours { crate::render::resolve_zcolour(cell.fg, scheme) } else { ratatui::style::Color::Reset };
    let mut bg = if honor_game_colours { crate::render::resolve_zcolour(cell.bg, scheme) } else { ratatui::style::Color::Reset };
    if cell.style & 0x01 != 0 {
        std::mem::swap(&mut fg, &mut bg); // reverse video swaps colour
    }
    let base = Style::default().fg(fg).bg(bg);
    crate::render::apply_text_style(base, cell.style & !0x01)
}

/// Convert a neutral [`GridCell`] (packed colour) into a `zvm::screen::Cell`
/// (typed `ZColour`) for [`cell_style`].
fn grid_cell_to_zvm(cell: GridCell) -> zvm::screen::Cell {
    zvm::screen::Cell {
        ch: cell.ch,
        style: cell.style,
        fg: crate::state::unpack_zcolour(cell.fg),
        bg: crate::state::unpack_zcolour(cell.bg),
    }
}

// ── Core grid renderer ────────────────────────────────────────────────────────

/// Draw the upper-window grid into `area`.
///
/// - `upper`: the neutral grid to render (the `ScreenModel`'s text-grid window).
/// - `upper_rows`: the active row count (`GridWindow::active_rows`).
/// - `cursor`: 1-based (row, col) of the grid cursor.
/// - `show_cursor`: when true, mark the cursor cell (e.g. while the game is
///   awaiting input in the upper window) so forms show where typing lands.
/// - `colors`: resolved color scheme.
/// - `area`: target rectangle in the buffer.
///
/// Returns the number of terminal rows consumed (0 when `upper_rows == 0`).
pub fn draw_grid(
    upper: &GridWindow,
    upper_rows: u16,
    cursor: (u16, u16),
    show_cursor: bool,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
    honor_game_colours: bool,
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
    let frame = draw_framed(buf, uw_area, border_style, colors.upper_window_border_sides, &colors.upper_window_border_glyphs, border_color, false);
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
                let style = cell_style(grid_cell_to_zvm(cell), colors, honor_game_colours);
                let mut ch_buf = [0u8; 4];
                buf_cell.set_symbol(cell.ch.encode_utf8(&mut ch_buf)).set_style(style);
            }
        }
    }

    // Cursor: resolve the cell's normal style then apply the terminal-level
    // REVERSED modifier on top. Using the modifier (not the bit-flip trick)
    // keeps the cursor visible even on default (Reset/Reset) cells, where
    // swapping fg with bg is a no-op and the cursor would disappear.
    if show_cursor && crow >= row_offset && ccol >= col_offset {
        let cur_dy = crow - row_offset;
        let cur_dx = ccol - col_offset;
        if cur_dy < content.height && cur_dx < content.width {
            let grid_row = cur_dy + row_offset + 1; // 1-based
            let grid_col = cur_dx + col_offset + 1; // 1-based
            let cur = upper.cell(grid_row, grid_col);
            let style = cell_style(grid_cell_to_zvm(cur), colors, honor_game_colours)
                .add_modifier(ratatui::style::Modifier::REVERSED);
            if let Some(c) = buf.cell_mut((content.x + cur_dx, content.y + cur_dy)) {
                c.set_style(style);
            }
        }
    }

    needed
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Draw the engine's text-grid (upper) window into the top of `area`, returning
/// the number of story-pane rows consumed (0 when the grid is inactive).
///
/// `char_mode` is true when the game is awaiting a keypress; combined with the
/// grid being the engine's currently selected window (`GridWindow::cursor_active`),
/// it decides whether to show the cursor (so in-place forms reveal where typed
/// characters land).
pub fn draw_upper_window(
    grid: &GridWindow,
    char_mode: bool,
    colors: &ColorScheme,
    area: Rect,
    buf: &mut Buffer,
    honor_game_colours: bool,
) -> u16 {
    let show_cursor = char_mode && grid.cursor_active;
    draw_grid(
        grid,
        grid.active_rows,
        grid.cursor,
        show_cursor,
        colors,
        area,
        buf,
        honor_game_colours,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};
    use ratatui::{buffer::Buffer, layout::Rect};

    #[test]
    fn upper_cell_colour_resolves_and_reverse_swaps() {
        use zvm::screen::{Cell, ZColour};
        let mut scheme = ColorScheme::default();
        scheme.palette[1] = Color::Rgb(200, 0, 0); // red   (Standard(3) -> palette[1])
        scheme.palette[4] = Color::Rgb(0, 0, 200); // blue  (Standard(6) -> palette[4])
        // no reverse: fg=red, bg=blue
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) },
            &scheme,
            true,
        );
        assert_eq!(s.fg, Some(Color::Rgb(200, 0, 0)));
        assert_eq!(s.bg, Some(Color::Rgb(0, 0, 200)));
        // reverse (style 0x01): fg/bg swapped exactly once
        let r = cell_style(
            Cell { ch: 'x', style: 0x01, fg: ZColour::Standard(3), bg: ZColour::Standard(6) },
            &scheme,
            true,
        );
        assert_eq!(r.fg, Some(Color::Rgb(0, 0, 200)));
        assert_eq!(r.bg, Some(Color::Rgb(200, 0, 0)));
    }

    fn make_colors() -> ColorScheme {
        ColorScheme::terminal_default()
    }

    /// Build a 2-row × 5-col grid with "HI" starting at (1,1).
    fn make_upper_hi() -> GridWindow {
        let mut w = GridWindow::default();
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

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors_no_border, area, &mut buf, true);

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
        let mut upper = GridWindow::default();
        upper.resize(1, 10); // game screen is 10 cols wide
        upper.put(1, 1, 'A', 0);
        upper.put(1, 10, 'Z', 0); // content spans the full game screen

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Pane is 30 wide; the 10-col upper window should center: x_off=(30-10)/2=10.
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true);
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
        let consumed = draw_grid(&upper, 0, (1, 1), false, &colors, area, &mut buf, true);
        assert_eq!(consumed, 0);
    }

    #[test]
    fn border_adds_overhead() {
        let upper = make_upper_hi();
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::Single;
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true);

        // 2 grid rows + 2 border rows = 4 total.
        assert_eq!(consumed, 4);
        // cols=5 + 2 side borders = 7, centered in 20: x_off=(20-7)/2=6, content.x=7.
        // Grid content starts at row 1 (inside the top border).
        assert_eq!(buf.cell((7, 1)).unwrap().symbol(), "H");
        assert_eq!(buf.cell((8, 1)).unwrap().symbol(), "I");
    }

    #[test]
    fn viewport_scrolls_when_cursor_exceeds_height() {
        let mut upper = GridWindow::default();
        upper.resize(5, 5);
        // Put 'A' at row 5 (last row, 1-based).
        upper.put(5, 1, 'A', 0);

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // Only 3 rows available, but cursor is at row 5.
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 5, (5, 1), false, &colors, area, &mut buf, true);
        assert_eq!(consumed, 3);

        // Row offset = cursor_row-1 - (height-1) = 4 - 2 = 2.
        // 'A' is at grid row 5, displayed at terminal row 2 (0-based within content).
        // cols=5 centered in 10 (no border): x_off=(10-5)/2=2, so col 2.
        assert_eq!(buf.cell((2, 2)).unwrap().symbol(), "A");
    }

    #[test]
    fn bold_and_reverse_style_applied() {
        use zvm::screen::ZColour;
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        // ZMSD §8.7.2 operand values: 1 = reverse-video, 2 = bold
        upper.put(1, 1, 'X', 0x02); // bold
        upper.put(1, 2, 'Y', 0x01); // reverse-video
        // Give Y distinct logical fg/bg so the reverse-swap is observable.
        upper.cells[1].fg = crate::state::pack_zcolour(ZColour::Standard(3)); // -> palette[1]
        upper.cells[1].bg = crate::state::pack_zcolour(ZColour::Standard(6)); // -> palette[4]

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        colors.palette[1] = Color::Rgb(200, 0, 0);
        colors.palette[4] = Color::Rgb(0, 0, 200);

        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3.
        let x_cell = buf.cell((3, 0)).unwrap();
        assert!(x_cell.modifier.contains(Modifier::BOLD), "X should be bold");

        // Reverse video is now a single fg/bg swap — NOT the REVERSED modifier.
        let y_cell = buf.cell((4, 0)).unwrap();
        assert!(
            !y_cell.modifier.contains(Modifier::REVERSED),
            "reverse uses the fg/bg swap, not the REVERSED modifier"
        );
        assert_eq!(y_cell.fg, Color::Rgb(0, 0, 200), "Y fg is the swapped bg");
        assert_eq!(y_cell.bg, Color::Rgb(200, 0, 0), "Y bg is the swapped fg");
    }

    #[test]
    fn cursor_cell_inverts_via_reverse_swap_when_show_cursor() {
        use zvm::screen::ZColour;
        let mut upper = GridWindow::default();
        upper.resize(2, 5);
        // Give the cell under the cursor distinct logical fg/bg so the invert
        // (a single fg/bg swap) is observable.
        upper.put(2, 3, 'C', 0);
        let idx = (2 - 1) * 5 + (3 - 1);
        upper.cells[idx].fg = crate::state::pack_zcolour(ZColour::Standard(3)); // -> palette[1]
        upper.cells[idx].bg = crate::state::pack_zcolour(ZColour::Standard(6)); // -> palette[4]

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        colors.palette[1] = Color::Rgb(200, 0, 0);
        colors.palette[4] = Color::Rgb(0, 0, 200);
        let area = Rect::new(0, 0, 10, 3);

        // cols=5 centered in 10 (no border): x_off=2; cursor (row 2, col 3) →
        // content (1,2) → buffer (4,1).
        // With show_cursor=false the cursor cell shows its logical fg/bg order.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), false, &colors, area, &mut buf, true);
        let c = buf.cell((4, 1)).unwrap();
        assert_eq!(c.fg, Color::Rgb(200, 0, 0), "no cursor: fg is logical");
        assert_eq!(c.bg, Color::Rgb(0, 0, 200), "no cursor: bg is logical");

        // With show_cursor=true the cursor cell carries REVERSED modifier on top
        // of its logical colours (no swap in the stored fg/bg values).
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), true, &colors, area, &mut buf, true);
        let c = buf.cell((4, 1)).unwrap();
        assert!(c.modifier.contains(Modifier::REVERSED), "cursor cell must carry REVERSED modifier");
        assert_eq!(c.fg, Color::Rgb(200, 0, 0), "cursor cell fg is logical (REVERSED handles invert)");
        assert_eq!(c.bg, Color::Rgb(0, 0, 200), "cursor cell bg is logical (REVERSED handles invert)");
    }

    /// Regression guard: cursor on a DEFAULT cell (fg == bg == ZColour::Default,
    /// resolved to Color::Reset) must still be visible. The old bit-flip path
    /// swapped Reset with Reset (a no-op), making the cursor invisible. The fixed
    /// path applies Modifier::REVERSED so the terminal inverts whatever the cell
    /// happens to look like.
    #[test]
    fn cursor_on_default_cell_carries_reversed_modifier() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        // Cell (1,2) keeps its default colours (ZColour::Default -> Color::Reset).
        upper.put(1, 2, ' ', 0);

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 2), true, &colors, area, &mut buf, true);

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            c.modifier.contains(Modifier::REVERSED),
            "cursor on a default (Reset/Reset) cell must carry REVERSED so it stays visible"
        );
    }
}
