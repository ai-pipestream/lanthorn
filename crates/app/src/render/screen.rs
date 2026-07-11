//! Story-pane renderer over the engine-neutral [`ScreenModel`] tree.
//!
//! One renderer draws both engines. The **simple** case — a single text-grid
//! over a single text-buffer (the Z-machine shape), or a lone buffer — routes to
//! the existing `draw_upper_window` + `render_transcript` path, so the Z-machine
//! output stays byte-identical. Any richer Glulx tree (multiple/other windows)
//! uses the generic recursive path: `Pair` splits the rect and recurses, `Grid`
//! leaves draw positioned cells, the **primary** `Buffer` leaf draws through the
//! transcript renderer (keeping search / persistence / styling), extra buffers
//! draw their inline content, and `Blank`/graphics leaves are placeholders.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::colors::ColorScheme;
use crate::engine::{BufferWindow, Introspect, ScreenModel, StatusModel, WinNode};
use crate::render::transcript::{draw_str_runs, render_transcript, visible_wrapped_lines_kinded};
use crate::render::upper_window::{draw_grid, draw_upper_window, grid_border_overhead};
use crate::state::{AppState, TranscriptKind};

/// Metrics the story-pane render reports back for scrollbar / mouse routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryPaneMetrics {
    /// Whether the (primary) transcript drew a scrollbar gutter.
    pub scrollbar: bool,
    /// The largest meaningful `transcript_scroll` value.
    pub max_scroll: u16,
    /// The transcript viewport height (rows).
    pub viewport_rows: u16,
    /// Per-frame map from rendered cell `(col, row)` → Glk hyperlink value, for
    /// hit-testing a mouse click to its link. Empty when nothing is linked.
    pub links: Vec<((u16, u16), u32)>,
}

/// Tally `(grids, buffers, others)` leaf windows in the tree.
fn count_leaves(node: &WinNode) -> (u32, u32, u32) {
    match node {
        WinNode::Grid(_) => (1, 0, 0),
        WinNode::Buffer(_) => (0, 1, 0),
        WinNode::Blank => (0, 0, 1),
        // A Graphics leaf can't use the simple text path — counts as "other",
        // forcing the generic path.
        WinNode::Graphics(_) => (0, 0, 1),
        WinNode::Pair { first, second, .. } => {
            let a = count_leaves(first);
            let b = count_leaves(second);
            (a.0 + b.0, a.1 + b.1, a.2 + b.2)
        }
    }
}

/// True for the Z-machine shape (and a lone-buffer Glulx game): ≤1 grid, ≤1
/// buffer, no other leaves — drawn through the existing grid/transcript path.
fn is_simple(model: &ScreenModel) -> bool {
    let (grids, buffers, others) = count_leaves(&model.root);
    others == 0 && grids <= 1 && buffers <= 1 && grids + buffers >= 1
}

/// The game's live input colour (fg/bg) for the input line, or None when
/// colours are off or the game left both channels Default (theme-neutral).
fn game_input_style(model: &ScreenModel, state: &AppState) -> Option<ratatui::style::Style> {
    if !state.config.honor_game_colours {
        return None;
    }
    let fg = crate::state::unpack_zcolour(model.fg);
    let bg = crate::state::unpack_zcolour(model.bg);
    if matches!(fg, zvm::screen::ZColour::Default) && matches!(bg, zvm::screen::ZColour::Default) {
        return None;
    }
    let mut s = ratatui::style::Style::new();
    if !matches!(fg, zvm::screen::ZColour::Default) {
        s = s.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, zvm::screen::ZColour::Default) {
        s = s.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    Some(s)
}

/// Render the engine's screen into the story-pane `area`, returning scrollbar /
/// scroll metrics for the (primary) transcript.
pub fn render_story_pane(
    model: &ScreenModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> StoryPaneMetrics {
    // Paint the story-pane background with the game's current background
    // (theme-safe: only the story pane, never the map/chrome; only a concrete,
    // honoured background — Default keeps the theme).
    if state.config.honor_game_colours {
        let bg = crate::state::unpack_zcolour(model.bg);
        if !matches!(bg, zvm::screen::ZColour::Default) {
            let bg_color = crate::render::resolve_zcolour(bg, &state.colors);
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(ratatui::style::Style::new().bg(bg_color));
                    }
                }
            }
        }
    }

    let gi = game_input_style(model, state);

    if is_simple(model) {
        // Byte-identical Z-machine path: the upper grid (if any) over the
        // transcript.
        let used = match model.grid() {
            Some(grid) => draw_upper_window(grid, char_mode, &state.colors, area, buf, state.config.honor_game_colours),
            None => 0,
        };
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let (scrollbar, max_scroll, links) = render_transcript(&model.status, introspect, state, tarea, buf, gi);
        return StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: tarea.height, links };
    }

    // Generic multi-window path.
    let metrics = render_node(&model.root, &model.status, char_mode, introspect, state, area, buf, gi);

    // Prune the graphics protocol cache to only the windows still live in the
    // tree, so a closed window's stale cache entry can't be matched by a
    // reopened window reusing the same id (SQ-0174).
    let mut live = std::collections::HashSet::new();
    collect_graphics_ids(&model.root, &mut live);
    state.graphics_render.borrow_mut().retain_live(&live);

    metrics.unwrap_or(StoryPaneMetrics { scrollbar: false, max_scroll: 0, viewport_rows: area.height, links: Vec::new() })
}

/// Recursively render a tree node into `area`. Returns the primary buffer's
/// metrics when this subtree contains it.
fn render_node(
    node: &WinNode,
    status: &StatusModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    game_input: Option<ratatui::style::Style>,
) -> Option<StoryPaneMetrics> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    match node {
        WinNode::Pair { vertical, split, first, second } => {
            let (a1, a2) = pair_areas(*vertical, split.fixed, first, second, &state.colors, area);
            let m1 = render_node(first, status, char_mode, introspect, state, a1, buf, game_input);
            let m2 = render_node(second, status, char_mode, introspect, state, a2, buf, game_input);
            m1.or(m2)
        }
        WinNode::Grid(g) => {
            let show_cursor = char_mode && g.cursor_active;
            draw_grid(g, g.active_rows, g.cursor, show_cursor, &state.colors, area, buf, state.config.honor_game_colours);
            None
        }
        WinNode::Buffer(b) => {
            if b.primary {
                let (scrollbar, max_scroll, links) =
                    render_transcript(status, introspect, state, area, buf, game_input);
                Some(StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: area.height, links })
            } else {
                render_inline_buffer(b, state, area, buf);
                None
            }
        }
        WinNode::Blank => {
            fill(area, buf, &state.colors);
            None
        }
        WinNode::Graphics(gw) => {
            if let Some(picker) = state.game_picker.as_ref() {
                state.graphics_render.borrow_mut().render(picker, gw, area, state.colors.graphics, buf);
            } else {
                fill(area, buf, &state.colors);
            }
            None
        }
    }
}

/// The region a modal dialog should center within: the whole `frame`, minus any
/// Glulx graphics windows.
///
/// Graphics windows are painted through the terminal's own image protocol
/// (kitty/sixel), which draws on top of whatever cells they cover — so a dialog
/// centered over a graphics window is obscured in the real terminal even though
/// it was written into the buffer afterward. This returns the largest rectangle
/// of `frame` that touches no graphics window, so a dialog still spans the story
/// text and the map together where the geometry allows, avoiding only the
/// graphics. `story_area` is where the window tree is laid out (graphics live
/// inside it); pass an empty rect when the story pane isn't shown.
///
/// With no graphics windows this returns `frame` unchanged (today's behavior).
pub fn dialog_bounds(model: &ScreenModel, colors: &ColorScheme, story_area: Rect, frame: Rect) -> Rect {
    let mut graphics: Vec<Rect> = Vec::new();
    collect_graphics_rects(&model.root, colors, story_area, &mut graphics);
    let mut bounds = frame;
    for g in graphics {
        bounds = subtract_rect(bounds, g);
    }
    bounds
}

/// Walk the tree assigning each leaf its terminal rect (exactly as `render_node`
/// does, including grid border-row borrowing), collecting every graphics leaf's rect.
fn collect_graphics_rects(node: &WinNode, colors: &ColorScheme, area: Rect, out: &mut Vec<Rect>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match node {
        WinNode::Pair { vertical, split, first, second } => {
            let (a1, a2) = pair_areas(*vertical, split.fixed, first, second, colors, area);
            collect_graphics_rects(first, colors, a1, out);
            collect_graphics_rects(second, colors, a2, out);
        }
        WinNode::Graphics(_) => out.push(area),
        WinNode::Grid(_) | WinNode::Buffer(_) | WinNode::Blank => {}
    }
}

/// Collect the window ids of all live graphics windows in the tree.
fn collect_graphics_ids(node: &WinNode, out: &mut std::collections::HashSet<u32>) {
    match node {
        WinNode::Graphics(gw) => {
            out.insert(gw.win);
        }
        WinNode::Pair { first, second, .. } => {
            collect_graphics_ids(first, out);
            collect_graphics_ids(second, out);
        }
        _ => {}
    }
}

/// Remove `g` from `bounds` by a guillotine cut, keeping the largest remaining
/// rectangle. If `g` doesn't overlap `bounds`, `bounds` is returned unchanged.
fn subtract_rect(bounds: Rect, g: Rect) -> Rect {
    let ix = g.x.max(bounds.x);
    let iy = g.y.max(bounds.y);
    let ir = g.right().min(bounds.right());
    let ib = g.bottom().min(bounds.bottom());
    if ix >= ir || iy >= ib {
        return bounds; // no overlap
    }
    // The four rectangles of `bounds` lying outside the overlap band.
    let left = Rect::new(bounds.x, bounds.y, ix.saturating_sub(bounds.x), bounds.height);
    let right = Rect::new(ir, bounds.y, bounds.right().saturating_sub(ir), bounds.height);
    let above = Rect::new(bounds.x, bounds.y, bounds.width, iy.saturating_sub(bounds.y));
    let below = Rect::new(bounds.x, ib, bounds.width, bounds.bottom().saturating_sub(ib));
    [left, right, above, below]
        .into_iter()
        .max_by_key(|r| r.width as u32 * r.height as u32)
        .unwrap_or(bounds)
}

/// Split `area` into `(first, second)`: a vertical pair stacks first-on-top
/// (first gets `fixed` rows); a horizontal pair places first-on-left (first gets
/// `fixed` cols). `fixed` is clamped to the available extent.
fn split_area(area: Rect, vertical: bool, fixed: u16) -> (Rect, Rect) {
    if vertical {
        let h = fixed.min(area.height);
        let first = Rect::new(area.x, area.y, area.width, h);
        let second = Rect::new(area.x, area.y + h, area.width, area.height - h);
        (first, second)
    } else {
        let w = fixed.min(area.width);
        let first = Rect::new(area.x, area.y, w, area.height);
        let second = Rect::new(area.x + w, area.y, area.width - w, area.height);
        (first, second)
    }
}

/// Split `area` for a `Pair`, granting a stacked (vertical) active grid child
/// the extra rows its border chrome needs — borrowed from its sibling — so the
/// chrome isn't squished into the grid's exact Glk allotment (SQ-0200). This
/// mirrors the simple path, where the framed grid takes its border rows from the
/// transcript below. Horizontal splits and non-grid children are unaffected.
///
/// Used by both `render_node` (to draw) and `collect_graphics_rects` (so
/// `dialog_bounds` sees graphics where they're actually rendered).
fn pair_areas(
    vertical: bool,
    split_fixed: u16,
    first: &WinNode,
    second: &WinNode,
    colors: &ColorScheme,
    area: Rect,
) -> (Rect, Rect) {
    let (mut a1, mut a2) = split_area(area, vertical, split_fixed);
    if vertical {
        let overhead = grid_border_overhead(colors);
        if overhead > 0 {
            if grid_is_active(first) {
                let take = overhead.min(a2.height);
                a1.height += take;
                a2.y += take;
                a2.height -= take;
            } else if grid_is_active(second) {
                let take = overhead.min(a1.height);
                a1.height -= take;
                a2.y -= take;
                a2.height += take;
            }
        }
    }
    (a1, a2)
}

/// True for a text-grid leaf that will actually draw (has active rows).
fn grid_is_active(node: &WinNode) -> bool {
    matches!(node, WinNode::Grid(g) if g.active_rows > 0)
}

/// Draw an inline (non-primary) buffer window's wrapped, styled lines.
fn render_inline_buffer(b: &BufferWindow, state: &AppState, area: Rect, buf: &mut Buffer) {
    fill(area, buf, &state.colors);
    if b.lines.is_empty() {
        return;
    }
    let base = state.colors.transcript;
    let kinds = vec![TranscriptKind::Story; b.lines.len()];
    let styles = vec![base; b.lines.len()];
    // Inline images render as bands only when a game picker exists (same as the
    // transcript); `char_px` is that picker's cell pixel size for pixel-accurate
    // fit. Mirrors `render_middle`.
    let images_enabled = state.game_picker.is_some();
    let char_px = state
        .game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width, f.height)
        })
        .unwrap_or((1, 1));
    let (rows, _total, _first) = visible_wrapped_lines_kinded(
        &b.lines,
        &kinds,
        &styles,
        &b.runs,
        &b.images,
        char_px,
        images_enabled,
        area.height as usize,
        b.scroll,
        area.width,
        None,
    );
    for (i, wr) in rows.iter().enumerate() {
        let row_y = area.y + i as u16;
        // Inline-image band row: blit the strip for this row instead of text
        // (same branch as the transcript draw loop, Task 8).
        if crate::render::inline_image::try_blit_band_row(state, wr, area.x, area.width, row_y, buf) {
            continue;
        }
        draw_str_runs(buf, area.x, row_y, &wr.text, wr.style, &wr.runs, None, area, state.config.honor_game_colours.then_some(&state.colors));
    }
}

/// Fill `area` with the transcript background style.
fn fill(area: Rect, buf: &mut Buffer, colors: &crate::colors::ColorScheme) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(colors.transcript);
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{GridWindow, Split};
    use crate::state::StyleRun;
    use ratatui::layout::Rect;

    fn grid_with(text: &str) -> GridWindow {
        let mut g = GridWindow::default();
        g.resize(1, text.chars().count() as u16);
        for (i, ch) in text.chars().enumerate() {
            g.put(1, i as u16 + 1, ch, 0);
        }
        g.active_rows = 1;
        g
    }

    fn inline_buffer(line: &str) -> BufferWindow {
        BufferWindow {
            lines: vec![line.to_string()],
            runs: vec![Vec::new()],
            images: vec![None],
            scroll: 0,
            primary: false,
        }
    }

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect()
    }

    #[test]
    fn is_simple_classifies_trees() {
        // Z-machine shape: grid over a (non-primary) buffer.
        let zm = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                first: Box::new(WinNode::Grid(GridWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
        };
        assert!(is_simple(&zm));
        // Lone buffer: simple.
        let lone = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
        };
        assert!(is_simple(&lone));
        // Two buffers: not simple.
        let two = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 10 },
                first: Box::new(WinNode::Buffer(BufferWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
        };
        assert!(!is_simple(&two));
    }

    #[test]
    fn split_area_vertical_and_horizontal() {
        let area = Rect::new(0, 0, 20, 10);
        let (top, bottom) = split_area(area, true, 3);
        assert_eq!(top, Rect::new(0, 0, 20, 3));
        assert_eq!(bottom, Rect::new(0, 3, 20, 7));
        let (left, right) = split_area(area, false, 8);
        assert_eq!(left, Rect::new(0, 0, 8, 10));
        assert_eq!(right, Rect::new(8, 0, 12, 10));
        // Oversized fixed clamps to the extent.
        let (l2, r2) = split_area(area, true, 99);
        assert_eq!(l2.height, 10);
        assert_eq!(r2.height, 0);
    }

    #[test]
    fn generic_renders_grid_and_two_inline_buffers_in_subrects() {
        // Grid (top row) over a left|right buffer split.
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                first: Box::new(WinNode::Grid(grid_with("STATUS"))),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 10 },
                    first: Box::new(WinNode::Buffer(inline_buffer("LEFT"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("RIGHT"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
        };
        assert!(!is_simple(&model));

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Grid "STATUS" drawn on the top row (centered in its 20-wide area).
        assert!(row_text(&buf, 0, 20).contains("STATUS"), "grid row: {:?}", row_text(&buf, 0, 20));
        // Row 1: LEFT buffer in cols [0,10), RIGHT buffer in cols [10,20).
        assert_eq!(row_text(&buf, 1, 4), "LEFT");
        let right = row_text(&buf, 1, 20);
        assert!(right[10..].contains("RIGHT"), "right buffer at col>=10: {:?}", right);
    }

    /// SQ-0200: in the generic multi-window path a bordered status grid must not
    /// be squished into its exact 1-row Glk split — it borrows its border rows
    /// from the sibling below (as the simple path does), so the chrome fits.
    #[test]
    fn generic_grid_borrows_border_rows_from_sibling() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        // status grid (1 row) over [graphics banner | primary buffer]; the
        // graphics leaf forces the generic path.
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                first: Box::new(WinNode::Grid(grid_with("HI"))),
                second: Box::new(WinNode::Pair {
                    vertical: true,
                    split: Split { fixed: 3 },
                    first: Box::new(graphics_node()),
                    second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
        };
        assert!(!is_simple(&model));

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::Single;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::Single);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Grid "HI" (2 cols) framed: uw_w = 2 + 2 borders = 4, centered in 20 →
        // x_off = 8, content at x=9. Top border row 0, content row 1.
        assert_ne!(buf.cell((8, 0)).unwrap().symbol(), " ", "top-left border corner drawn");
        assert_eq!(buf.cell((9, 1)).unwrap().symbol(), "H", "grid content sits inside the border, not squished");
        assert_eq!(buf.cell((10, 1)).unwrap().symbol(), "I");
    }

    #[test]
    fn inline_buffer_renders_styled_runs() {
        let mut b = inline_buffer("abCD");
        b.runs = vec![vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0, link: 0 }]];
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        render_inline_buffer(&b, &state, area, &mut buf);
        assert_eq!(row_text(&buf, 0, 4), "abCD");
        // 'C' (col 2) carries the bold modifier.
        assert!(buf.cell((2, 0)).unwrap().modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(!buf.cell((0, 0)).unwrap().modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn inline_buffer_pushes_text_below_image_band() {
        // lines a / <image> / b. With a picker present the image at index 1
        // expands into a multi-row band, pushing "b" below the row it occupies
        // when images are off. Halfblocks font is 10x20 px; a 16x48-px image at
        // width 10 fits to a 2x3-cell band, so "b" lands on row 1 + 3 = 4.
        let mut px = image::RgbaImage::new(16, 48);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 40, 60, 255]);
        }
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
        };
        let b = BufferWindow {
            lines: vec!["a".to_string(), String::new(), "b".to_string()],
            runs: vec![Vec::new(), Vec::new(), Vec::new()],
            images: vec![None, Some(dummy), None],
            scroll: 0,
            primary: false,
        };
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let area = Rect::new(0, 0, 10, 8);
        let mut buf = Buffer::empty(area);
        render_inline_buffer(&b, &state, area, &mut buf);
        assert_eq!(row_text(&buf, 0, 1), "a", "first text line stays on row 0");
        let b_row = (0..8).find(|&y| row_text(&buf, y, 1).starts_with('b'));
        assert_eq!(b_row, Some(4), "\"b\" pushed below the 3-row image band");
    }

    #[test]
    fn story_pane_fills_game_background() {
        use ratatui::style::Color;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        // honor_game_colours defaults to true.
        let mut model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
        };
        model.bg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(2)); // black
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);
        // A blank interior cell (the empty transcript body, not the bottom input
        // row) carries the game background (black).
        assert_eq!(buf.cell((0, 2)).unwrap().style().bg, Some(Color::Black),
            "story pane blank cell painted with game background");
    }

    /// The Z-machine 2-node tree must render byte-identical through
    /// `render_story_pane` vs. the direct `draw_upper_window` + `render_transcript`
    /// path it replaces.
    #[test]
    fn zmachine_two_node_tree_is_byte_identical() {
        use zvm::cpu::exec::Machine;
        // A minimal v3 machine → its neutral 2-node screen model.
        let story = {
            // Minimal valid v3 header (mirrors the render-test fixtures).
            let mut buf = vec![0u8; 0x0800];
            buf[0x00] = 3;
            buf[0x04] = 0x00; buf[0x05] = 0x40; // high mem base
            buf[0x06] = 0x00; buf[0x07] = 0x40; // initial pc
            buf[0x0A] = 0x00; buf[0x0B] = 0x80; // dict
            buf[0x0C] = 0x01; buf[0x0D] = 0x00; // object table
            buf[0x0E] = 0x03; buf[0x0F] = 0x00; // globals
            buf[0x08] = 0x04; buf[0x09] = 0x00; // static base
            buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev table
            buf[0x0081] = 4; // dict entry size
            buf[0x0040] = 0xba; // quit
            buf
        };
        let mem = zvm::memory::Memory::new(story).expect("minimal v3");
        let machine = Machine::new(mem);
        let model = crate::session::screen_model_from_machine(&machine);
        assert!(is_simple(&model), "Z-machine tree is the simple case");

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.push_transcript("You are in a room.");
        let area = Rect::new(0, 0, 40, 12);

        // Path A: render_story_pane.
        let mut buf_a = Buffer::empty(area);
        let ma = render_story_pane(&model, false, None, &state, area, &mut buf_a);

        // Path B: the exact code render_story_pane replaced.
        let mut buf_b = Buffer::empty(area);
        let used = draw_upper_window(model.grid().unwrap(), false, &state.colors, area, &mut buf_b, state.config.honor_game_colours);
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let (sb, ms, _) = render_transcript(&model.status, None, &state, tarea, &mut buf_b, None);

        assert_eq!(buf_a, buf_b, "the simple path must be byte-identical to the legacy path");
        assert_eq!((ma.scrollbar, ma.max_scroll, ma.viewport_rows), (sb, ms, tarea.height));
    }

    fn graphics_node() -> WinNode {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 1,
            canvas: std::sync::Arc::new(img),
            version: 1,
        })
    }

    fn model_with(root: WinNode) -> ScreenModel {
        ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0 }
    }

    fn dialog_colors() -> ColorScheme {
        crate::colors::ColorScheme::terminal_default()
    }

    #[test]
    fn dialog_bounds_returns_frame_when_no_graphics() {
        // A pure-text tree: no graphics → dialogs keep full-frame centering.
        let model = model_with(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }));
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, &dialog_colors(), Rect::new(0, 0, 20, 12), frame), frame);
    }

    #[test]
    fn dialog_bounds_excludes_left_graphics_sidebar_and_spans_map() {
        // Story pane (cols 0..20) = graphics sidebar (cols 0..10) | text buffer
        // (cols 10..20); the map occupies cols 20..40 of the frame. The dialog
        // region must be everything right of the graphics — text + map.
        let model = model_with(WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let story_area = Rect::new(0, 0, 20, 12);
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, &dialog_colors(), story_area, frame), Rect::new(10, 0, 30, 12));
    }

    #[test]
    fn dialog_bounds_excludes_top_graphics_band() {
        // Graphics banner (rows 0..3) over the text buffer; no map (TranscriptFull).
        let model = model_with(WinNode::Pair {
            vertical: true,
            split: Split { fixed: 3 },
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let area = Rect::new(0, 0, 20, 12);
        assert_eq!(dialog_bounds(&model, &dialog_colors(), area, area), Rect::new(0, 3, 20, 9));
    }

    #[test]
    fn dialog_bounds_ignores_graphics_when_story_pane_hidden() {
        // MapFull: story pane isn't laid out (empty), so graphics aren't on screen
        // and the dialog centers over the whole frame.
        let model = model_with(WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, &dialog_colors(), Rect::default(), frame), frame);
    }

    #[test]
    fn graphics_leaf_renders_pixels() {
        use ratatui::layout::Rect;
        use ratatui::buffer::Buffer;
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 50, 50, 255]));
        let gw = crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1 };
        let picker = ratatui_image::picker::Picker::halfblocks();
        let mut gr = crate::render::graphics::GraphicsRender::default();
        let area = Rect::new(0, 0, 12, 6);
        let mut buf = Buffer::empty(area);
        let style = ratatui::style::Style::default();
        gr.render(&picker, &gw, area, style, &mut buf);
        let has_pixels = (area.top()..area.bottom()).any(|y| (area.left()..area.right())
            .any(|x| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}")));
        assert!(has_pixels, "graphics canvas should render half-block pixels");
    }

    #[test]
    fn collect_graphics_ids_finds_every_graphics_leaf() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let other = WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 7,
            canvas: std::sync::Arc::new(img),
            version: 1,
        });
        let tree = WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            first: Box::new(graphics_node()), // win: 1
            second: Box::new(other),
        };
        let mut ids = std::collections::HashSet::new();
        collect_graphics_ids(&tree, &mut ids);
        assert_eq!(ids, std::collections::HashSet::from([1, 7]));
    }
}
