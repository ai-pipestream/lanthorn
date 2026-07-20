/// Render the engine's text-grid (upper) window atop the story pane transcript.
///
/// Public entry point: `draw_upper_window` — reads a neutral [`GridWindow`] from
/// the engine's `ScreenModel` and delegates to the testable `draw_grid` helper.
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::colors::ColorScheme;
use crate::engine::{BorderPref, GridCell, GridWindow};
use crate::render::paneframe::{draw_framed, BorderStyle, PaneSides};

/// Resolve a grid cell's game colour into a ratatui [`Style`], mirroring the
/// mechanism used by `draw_str_runs` in the transcript renderer.
///
/// Reverse video (style bit `0x01`) is realised via the ratatui `REVERSED`
/// modifier so the terminal performs exactly one swap. fg/bg are applied in
/// logical order (pre-reverse) for non-Default channels when
/// `honor_game_colours` is true; Default channels inherit from the theme base.
fn cell_style(cell: zvm::screen::Cell, glk_style: u8, scheme: &ColorScheme, honor_game_colours: bool, bg_override: Option<u32>) -> Style {
    use zvm::screen::ZColour;
    // Use the theme upper_window content style as the base (consistent with the
    // blank-fill path in draw_grid, and with how transcript.rs draws styled runs).
    // A per-window background override (Glulx window colour, SQ-0328) replaces the
    // theme bg here so a Default-bg game cell shows the window's own colour.
    let mut base = scheme.theme.get("upper_window").style;
    if let Some(rgb) = bg_override {
        base = base.bg(crate::render::resolve_zcolour(ZColour::True24(rgb), scheme));
    }
    // apply_text_style adds REVERSED for bit 0x01, BOLD for 0x02, ITALIC for 0x04.
    // The terminal performs exactly one swap for the REVERSED modifier — no manual
    // fg/bg swap here (which would be a no-op for Default/Reset channels, C1 bug).
    let mut s = crate::render::apply_text_style(base, cell.style);
    // Per-channel colour resolution (SQ-0331): game-set cell colour (gated by
    // honor_game_colours), then the theme's per-Glk-style slot (grid = row 1),
    // then the element base. Mirrors draw_str_runs in transcript.rs exactly.
    let glk = scheme.glk_styles[1].get(glk_style as usize).copied().unwrap_or_default();
    let game = |c: ZColour| -> Option<ratatui::style::Color> {
        (!matches!(c, ZColour::Default)).then(|| crate::render::resolve_zcolour(c, scheme))
    };
    if let Some(c) = crate::render::resolve_glk_channel(game(cell.fg), glk.fg, base.fg, honor_game_colours) {
        s = s.fg(c);
    }
    if let Some(c) = crate::render::resolve_glk_channel(game(cell.bg), glk.bg, base.bg, honor_game_colours) {
        s = s.bg(c);
    }
    let glk_mods =
        crate::render::glk_theme_modifiers(scheme, true, glk_style as usize) | glk.add_modifier;
    if !glk_mods.is_empty() {
        s = s.add_modifier(glk_mods);
    }
    s
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

/// The per-side border styles `draw_grid` will actually use for `grid`, after
/// applying the game's border PRESENCE over the theme's glyph/colour (SQ-0286):
/// - `NoBorder` (an explicit Glulx `winmethod_NoBorder`) draws no sides, whatever
///   the theme;
/// - `Border` (an explicit Glulx `winmethod_Border`) uses the theme's sides, or
///   falls back to a single-line full frame if the theme disabled every side, so
///   the game's request is visibly honored;
/// - `Unspecified` (Z-machine, default, parentless root) defers entirely to the
///   theme — frameless when the theme turns the border off.
pub fn resolved_grid_sides(grid: &GridWindow, colors: &ColorScheme) -> PaneSides {
    match grid.border {
        BorderPref::NoBorder => PaneSides::all(BorderStyle::None),
        BorderPref::Border => {
            if colors.upper_window_border_sides.any_on() {
                colors.upper_window_border_sides
            } else {
                PaneSides::all(BorderStyle::Single)
            }
        }
        BorderPref::Unspecified => colors.upper_window_border_sides,
    }
}

/// Terminal rows the grid's border chrome adds on top of its content rows
/// (top + bottom borders), honoring the game's border presence (SQ-0286). The
/// generic multi-window path widens a stacked grid's allotment by this much so
/// the chrome isn't squished into the grid's exact Glk split (SQ-0200);
/// `draw_grid` sizes its own frame with it too.
pub fn grid_border_overhead(grid: &GridWindow, colors: &ColorScheme) -> u16 {
    let sides = resolved_grid_sides(grid, colors);
    (if sides.top != BorderStyle::None { 1 } else { 0 })
        + (if sides.bottom != BorderStyle::None { 1 } else { 0 })
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
    links: &mut Vec<((u16, u16), u32)>,
) -> u16 {
    if upper_rows == 0 || area.height == 0 || area.width == 0 {
        return 0;
    }

    // Per-window background override (Glulx window colour, SQ-0328): when the grid
    // carries its own `bg`, the content fill and each cell's default background use
    // it instead of the theme's `upper_window` bg. `None` (Z-machine simple path,
    // default) leaves the behaviour byte-identical.
    let uw = colors.theme.get("upper_window").style;
    let mut content_style = match upper.bg {
        Some(rgb) => uw.bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), colors)),
        None => uw,
    };
    // If the game reversed the grid's Normal style with no explicit colours
    // (Counterfeit Monkey's menu sets ReverseColor on every grid style), the empty
    // fill must invert the theme base too — otherwise the unwritten cells show the
    // non-reversed base (white) while the reverse-video text is dark. (SQ-0403)
    if upper.reverse {
        content_style = content_style.add_modifier(ratatui::style::Modifier::REVERSED);
    }
    let border_color = colors.theme.get("upper_window_border").style;

    // Resolve which sides to frame. The game controls border PRESENCE (SQ-0286);
    // the theme controls only the glyph + colour (see `resolved_grid_sides`).
    let sides = resolved_grid_sides(upper, colors);

    // How many terminal rows does the border frame consume? (top + bottom sides).
    let border_overhead: u16 = grid_border_overhead(upper, colors);

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
    let frame = draw_framed(buf, uw_area, sides, &colors.upper_window_border_glyphs, border_color, false);
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
                let mut style = cell_style(grid_cell_to_zvm(cell), cell.glk_style, colors, honor_game_colours, upper.bg);
                // Glk hyperlink affordance: layer the themeable `hyperlink` colour
                // and an underline on top, and record the cell for click hit-testing.
                // Mirrors the transcript path in `draw_str_runs`. (SQ-0258)
                if cell.link != 0 {
                    if honor_game_colours {
                        style = style.patch(colors.theme.get("hyperlink").style);
                    }
                    style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
                    links.push(((bx, by), cell.link));
                }
                let mut ch_buf = [0u8; 4];
                buf_cell.set_symbol(cell.ch.encode_utf8(&mut ch_buf)).set_style(style);
            }
        }
    }

    // Cursor: XOR bit 0x01 into the cell's style before calling cell_style, so
    // apply_text_style reflects the toggled reverse. Cursor on a normal cell adds
    // REVERSED (inverts, visible); cursor on an already-reverse cell removes it
    // (contrasts its reversed neighbours, still visually distinct). Exactly one
    // terminal swap in every case.
    //
    // ratatui's set_style uses insert-semantics for modifiers, so we reset the
    // buffer cell's modifier first to make the cursor's style authoritative —
    // otherwise REVERSED painted by the game-cell loop above would persist when
    // the XOR removes it (cursor on an already-reverse cell).
    if show_cursor && crow >= row_offset && ccol >= col_offset {
        let cur_dy = crow - row_offset;
        let cur_dx = ccol - col_offset;
        if cur_dy < content.height && cur_dx < content.width {
            let grid_row = cur_dy + row_offset + 1; // 1-based
            let grid_col = cur_dx + col_offset + 1; // 1-based
            let cur_cell = upper.cell(grid_row, grid_col);
            let mut cur_zvm = grid_cell_to_zvm(cur_cell);
            cur_zvm.style ^= 0x01; // toggle reverse bit
            let style = cell_style(cur_zvm, cur_cell.glk_style, colors, honor_game_colours, upper.bg);
            if let Some(c) = buf.cell_mut((content.x + cur_dx, content.y + cur_dy)) {
                c.modifier = ratatui::style::Modifier::empty(); // clear before re-apply
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
    links: &mut Vec<((u16, u16), u32)>,
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
        links,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};
    use ratatui::{buffer::Buffer, layout::Rect};

    #[test]
    fn upper_cell_colour_resolves_and_reverse_uses_modifier() {
        use zvm::screen::{Cell, ZColour};
        let mut scheme = ColorScheme::default();
        scheme.palette[1] = Color::Rgb(200, 0, 0); // red   (Standard(3) -> palette[1])
        scheme.palette[4] = Color::Rgb(0, 0, 200); // blue  (Standard(6) -> palette[4])
        // no reverse: fg=red, bg=blue (logical order, no REVERSED modifier)
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) },
            0,
            &scheme,
            true,
            None,
        );
        assert_eq!(s.fg, Some(Color::Rgb(200, 0, 0)));
        assert_eq!(s.bg, Some(Color::Rgb(0, 0, 200)));
        assert!(!s.add_modifier.contains(Modifier::REVERSED), "no REVERSED for style=0");
        // reverse (style 0x01): REVERSED modifier set, fg/bg stay in logical order —
        // the terminal performs the single swap via the modifier.
        let r = cell_style(
            Cell { ch: 'x', style: 0x01, fg: ZColour::Standard(3), bg: ZColour::Standard(6) },
            0,
            &scheme,
            true,
            None,
        );
        assert!(r.add_modifier.contains(Modifier::REVERSED), "REVERSED modifier for style=0x01");
        assert_eq!(r.fg, Some(Color::Rgb(200, 0, 0)), "fg stays logical (not swapped)");
        assert_eq!(r.bg, Some(Color::Rgb(0, 0, 200)), "bg stays logical (not swapped)");
    }

    /// A grid cell's Glk style class selects the theme's per-style colour slot
    /// from the GRID row (row 1) — applying in both gate states, while a Normal
    /// cell (unseeded row 1) inherits the `upper_window` element base (SQ-0331).
    #[test]
    fn cell_style_grid_glk_style_slot_uses_row1() {
        use zvm::screen::{Cell, ZColour};
        let mut scheme = ColorScheme::default();
        scheme.glk_styles[1][4] = Style::default().fg(Color::Green);
        // Subheader (glk_style 4) grid cell, no game colour → slot green, honor OFF.
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            4, &scheme, false, None,
        );
        assert_eq!(s.fg, Some(Color::Green), "grid Subheader → row-1 slot");
        // Normal (glk_style 0) cell → element base (upper_window fg, None by default).
        let n = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            0, &scheme, false, None,
        );
        assert_eq!(n.fg, scheme.theme.get("upper_window").style.fg, "grid Normal → upper_window element base");
    }

    /// Alert (glk_style 5) in a grid window renders bold — the registry theme's
    /// canonical Alert modifier, applied unconditionally (SQ-0309 §3).
    #[test]
    fn glk_grid_alert_renders_bold() {
        use zvm::screen::{Cell, ZColour};
        let scheme = ColorScheme::default();
        let s = cell_style(
            Cell { ch: 'x', style: 0, fg: ZColour::Default, bg: ZColour::Default },
            5, &scheme, false, None,
        );
        assert!(s.add_modifier.contains(ratatui::style::Modifier::BOLD), "Alert grid cell renders bold");
    }

    /// C1 regression guard: a reverse cell with DEFAULT colours (fg==bg==ZColour::Default)
    /// must carry Modifier::REVERSED even when honor_game_colours is ON. The previous code
    /// used mem::swap(Reset, Reset) = no-op, then masked bit 0x01, making the inversion
    /// invisible for the most common case (status bars that invert without set_colour).
    #[test]
    fn reverse_cell_with_default_colours_carries_reversed() {
        use zvm::screen::{Cell, ZColour};
        let scheme = ColorScheme::default();
        let s = cell_style(
            Cell { ch: ' ', style: 0x01, fg: ZColour::Default, bg: ZColour::Default },
            0,
            &scheme,
            true,
            None,
        );
        assert!(
            s.add_modifier.contains(Modifier::REVERSED),
            "C1: reverse cell with default colours must carry REVERSED modifier"
        );
    }

    /// C1 regression guard (draw_grid level): the REVERSED modifier must reach the
    /// buffer cell for a grid cell that is reverse-video with default colours.
    #[test]
    fn draw_grid_reverse_default_cell_has_reversed_in_buffer() {
        let mut upper = GridWindow::default(); // border Unspecified → theme decides
        upper.resize(1, 3);
        upper.put(1, 2, 'X', 0x01); // reverse, default colors

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            c.modifier.contains(Modifier::REVERSED),
            "draw_grid: reverse cell with default colours must carry REVERSED in the buffer"
        );
    }

    /// A grid cell carrying a Glk hyperlink must render underlined AND be recorded
    /// in the cell→link map so a click can be hit-tested to it. (SQ-0258)
    #[test]
    fn draw_grid_hyperlinked_cell_underlines_and_maps_to_link() {
        let mut upper = GridWindow::default(); // border Unspecified → theme decides
        upper.resize(1, 3);
        // Put a plain char, then stamp a link on cell (1,2) directly.
        upper.put(1, 2, 'L', 0);
        upper.cells[1].link = 77;

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        let mut links: Vec<((u16, u16), u32)> = Vec::new();
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut links);

        assert!(
            buf.cell((4, 0)).unwrap().modifier.contains(Modifier::UNDERLINED),
            "a linked grid cell must render underlined"
        );
        assert_eq!(links, vec![((4, 0), 77)], "the linked cell is recorded at its buffer position");
    }

    /// SQ-0328: a grid carrying its own `bg` (a Glulx per-window background)
    /// fills its content cells with that colour instead of the theme's
    /// `upper_window` bg. A grid with `bg = None` is byte-identical to today.
    #[test]
    fn draw_grid_window_bg_fills_override_colour() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        upper.bg = Some(0x0012_3456); // packed 0x00RRGGBB window background

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; a content cell at x=4,y=0.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        assert_eq!(
            buf.cell((4, 0)).unwrap().style().bg,
            Some(Color::Rgb(0x12, 0x34, 0x56)),
            "the grid's own window bg fills the content cells"
        );
    }

    /// SQ-0286 (a): a `BorderPref::NoBorder` grid draws NO frame even when the
    /// theme has every border side on — the content sits flush at row 0 and no
    /// border row is consumed.
    #[test]
    fn draw_grid_noborder_suppresses_frame_despite_theme() {
        let mut upper = make_upper_hi(); // 2×5 grid, "HI" at (1,1)
        upper.border = BorderPref::NoBorder;

        // Theme with ALL border sides on — normally this frames the grid.
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::Single;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::Single);

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // No border overhead: exactly the 2 grid rows, content flush at row 0.
        assert_eq!(consumed, 2, "a NoBorder grid consumes only its content rows");
        // cols=5 centered in 20 with no border cols: x_off = (20-5)/2 = 7.
        assert_eq!(buf.cell((7, 0)).unwrap().symbol(), "H", "content sits at row 0, no top border");
        assert_eq!(buf.cell((8, 0)).unwrap().symbol(), "I");
        // No box-drawing corner anywhere on the top row.
        for x in 0..20 {
            assert_ne!(buf.cell((x, 0)).unwrap().symbol(), "┌", "no frame corner drawn for a NoBorder grid");
        }
    }

    /// SQ-0286 (b): a `BorderPref::Border` grid (an explicit Glulx `winmethod_Border`)
    /// whose theme disabled every side still renders a frame — the fallback
    /// single-line box — so the game's border request is visibly honored.
    #[test]
    fn draw_grid_bordered_forces_frame_when_theme_off() {
        let mut upper = make_upper_hi();
        upper.border = BorderPref::Border; // game explicitly requested a border

        // Theme with EVERY side off.
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // Fallback single frame: top+bottom overhead added (2 grid rows + 2).
        assert_eq!(consumed, 4, "the forced fallback frame adds top+bottom border rows");
        // border_cols = 2 → uw_w = 7, x_off = (20-7)/2 = 6; top-left corner at (6,0).
        assert_eq!(buf.cell((6, 0)).unwrap().symbol(), "┌", "fallback single-line corner is drawn");
        // Content is pushed inside the frame: 'H' now at (7,1), not (7,0).
        assert_eq!(buf.cell((7, 1)).unwrap().symbol(), "H", "content sits inside the forced frame");
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
        let upper = make_upper_hi(); // border Unspecified → theme decides
        let colors = make_colors();
        // Area taller than the grid so no scrolling needed.
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        // Use BorderStyle::None to avoid border overhead for simplicity.
        let mut colors_no_border = colors.clone();
        colors_no_border.virtual_window_border = BorderStyle::None;
        colors_no_border.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors_no_border, area, &mut buf, true, &mut Vec::new());

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
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(buf.cell((10, 0)).unwrap().symbol(), "A", "left edge of the game screen at x=10");
        assert_eq!(buf.cell((19, 0)).unwrap().symbol(), "Z", "right edge at x=19 (10..19)");
        // Nothing drawn outside the centered 10-col region.
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), " ", "no content stretched to the pane left edge");
    }

    /// SQ-0403: a grid whose Normal style is reverse-video with no explicit
    /// colours (Counterfeit Monkey's menu sets ReverseColor on every grid style)
    /// must fill its EMPTY cells reversed too, so the whole window reads dark to
    /// match the reverse-video text — not a white fill with dark text islands.
    #[test]
    fn reverse_grid_fills_empty_cells_reversed() {
        let mut upper = GridWindow::default();
        upper.resize(2, 5);
        upper.reverse = true; // game reversed the grid's Normal style
        upper.put(1, 1, 'X', 0); // one written non-reverse cell; the rest are empty fill

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // An empty fill cell (row 1, col 5 → cols=5 centered in 10: x_off=2, so x=6) is REVERSED.
        assert!(
            buf.cell((6, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "a reverse-Normal grid must fill empty cells reversed so the window reads dark"
        );
    }

    #[test]
    fn returns_zero_when_upper_window_inactive() {
        let upper = make_upper_hi();
        let colors = make_colors();
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        // upper_rows = 0 means inactive.
        let consumed = draw_grid(&upper, 0, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
        assert_eq!(consumed, 0);
    }

    #[test]
    fn border_adds_overhead() {
        let upper = make_upper_hi();
        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::Single;
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);

        let consumed = draw_grid(&upper, 2, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

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

        let consumed = draw_grid(&upper, 5, (5, 1), false, &colors, area, &mut buf, true, &mut Vec::new());
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
        // Give Y distinct logical fg/bg so the colour-handling is observable.
        upper.cells[1].fg = crate::state::pack_zcolour(ZColour::Standard(3)); // -> palette[1]
        upper.cells[1].bg = crate::state::pack_zcolour(ZColour::Standard(6)); // -> palette[4]

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);
        colors.palette[1] = Color::Rgb(200, 0, 0);
        colors.palette[4] = Color::Rgb(0, 0, 200);

        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 1), false, &colors, area, &mut buf, true, &mut Vec::new());

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3.
        let x_cell = buf.cell((3, 0)).unwrap();
        assert!(x_cell.modifier.contains(Modifier::BOLD), "X should be bold");

        // Reverse video uses the REVERSED modifier (not a manual fg/bg swap).
        // The terminal performs exactly one swap via the modifier. fg/bg remain
        // in logical order in the buffer.
        let y_cell = buf.cell((4, 0)).unwrap();
        assert!(
            y_cell.modifier.contains(Modifier::REVERSED),
            "reverse uses the REVERSED modifier, not a manual fg/bg swap"
        );
        assert_eq!(y_cell.fg, Color::Rgb(200, 0, 0), "Y fg stays logical (not swapped)");
        assert_eq!(y_cell.bg, Color::Rgb(0, 0, 200), "Y bg stays logical (not swapped)");
    }

    /// Cursor on a normal (non-reverse) cell: XOR toggles bit 0x01 ON, producing
    /// a REVERSED modifier with logical fg/bg order preserved for the terminal to swap.
    #[test]
    fn cursor_on_nonreverse_cell_adds_reversed_modifier() {
        use zvm::screen::ZColour;
        let mut upper = GridWindow::default();
        upper.resize(2, 5);
        // Give the cell under the cursor distinct game colours so the logical
        // ordering (fg/bg not swapped in buffer) can be verified.
        upper.put(2, 3, 'C', 0); // style=0 (normal, non-reverse)
        let idx = 5 + (3 - 1);
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
        draw_grid(&upper, 2, (2, 3), false, &colors, area, &mut buf, true, &mut Vec::new());
        let c = buf.cell((4, 1)).unwrap();
        assert_eq!(c.fg, Color::Rgb(200, 0, 0), "no cursor: fg is logical");
        assert_eq!(c.bg, Color::Rgb(0, 0, 200), "no cursor: bg is logical");

        // With show_cursor=true: XOR 0^1=1 → REVERSED modifier, fg/bg remain logical.
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 2, (2, 3), true, &colors, area, &mut buf, true, &mut Vec::new());
        let c = buf.cell((4, 1)).unwrap();
        assert!(c.modifier.contains(Modifier::REVERSED), "cursor on normal cell adds REVERSED modifier");
        assert_eq!(c.fg, Color::Rgb(200, 0, 0), "cursor fg stays logical (terminal swaps via REVERSED)");
        assert_eq!(c.bg, Color::Rgb(0, 0, 200), "cursor bg stays logical (terminal swaps via REVERSED)");
    }

    /// Cursor on an already-reverse cell: XOR toggles bit 0x01 OFF, removing the
    /// REVERSED modifier so the cursor cell appears normal while its reversed neighbours
    /// remain inverted — the cursor is still visually distinct.
    #[test]
    fn cursor_on_reverse_cell_toggles_reverse_off() {
        let mut upper = GridWindow::default();
        upper.resize(1, 3);
        upper.put(1, 2, 'R', 0x01); // style=reverse (0x01)

        let mut colors = make_colors();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = crate::render::paneframe::PaneSides::all(BorderStyle::None);

        // cols=3 centered in 10 (no border): x_off=(10-3)/2=3; col 2 -> buf x=4.
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        draw_grid(&upper, 1, (1, 2), true, &colors, area, &mut buf, true, &mut Vec::new());

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            !c.modifier.contains(Modifier::REVERSED),
            "cursor on an already-reverse cell must XOR-toggle reverse OFF"
        );
    }

    /// Cursor on a DEFAULT cell (fg == bg == ZColour::Default) must be visible.
    /// XOR toggles bit 0x01 ON (style 0→1), producing REVERSED so the terminal
    /// inverts whatever colours the cell inherits — the cursor is always visible.
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
        draw_grid(&upper, 1, (1, 2), true, &colors, area, &mut buf, true, &mut Vec::new());

        let c = buf.cell((4, 0)).unwrap();
        assert!(
            c.modifier.contains(Modifier::REVERSED),
            "cursor on a default (Reset/Reset) cell must carry REVERSED so it stays visible"
        );
    }
}
