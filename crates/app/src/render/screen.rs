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

use crate::engine::{BufferWindow, Introspect, ScreenModel, StatusModel, WinNode};
use crate::render::transcript::{draw_str_runs, render_transcript, visible_wrapped_lines_kinded};
use crate::render::upper_window::{draw_grid, draw_upper_window};
use crate::state::{AppState, TranscriptKind};

/// Metrics the story-pane render reports back for scrollbar / mouse routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoryPaneMetrics {
    /// Whether the (primary) transcript drew a scrollbar gutter.
    pub scrollbar: bool,
    /// The largest meaningful `transcript_scroll` value.
    pub max_scroll: u16,
    /// The transcript viewport height (rows).
    pub viewport_rows: u16,
}

/// Tally `(grids, buffers, others)` leaf windows in the tree.
fn count_leaves(node: &WinNode) -> (u32, u32, u32) {
    match node {
        WinNode::Grid(_) => (1, 0, 0),
        WinNode::Buffer(_) => (0, 1, 0),
        WinNode::Blank => (0, 0, 1),
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
    if is_simple(model) {
        // Byte-identical Z-machine path: the upper grid (if any) over the
        // transcript.
        let used = match model.grid() {
            Some(grid) => draw_upper_window(grid, char_mode, &state.colors, area, buf, state.config.honor_game_colours),
            None => 0,
        };
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let (scrollbar, max_scroll) = render_transcript(&model.status, introspect, state, tarea, buf);
        return StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: tarea.height };
    }

    // Generic multi-window path.
    let metrics = render_node(&model.root, &model.status, char_mode, introspect, state, area, buf);
    metrics.unwrap_or(StoryPaneMetrics { scrollbar: false, max_scroll: 0, viewport_rows: area.height })
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
) -> Option<StoryPaneMetrics> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    match node {
        WinNode::Pair { vertical, split, first, second } => {
            let (a1, a2) = split_area(area, *vertical, split.fixed);
            let m1 = render_node(first, status, char_mode, introspect, state, a1, buf);
            let m2 = render_node(second, status, char_mode, introspect, state, a2, buf);
            m1.or(m2)
        }
        WinNode::Grid(g) => {
            let show_cursor = char_mode && g.cursor_active;
            draw_grid(g, g.active_rows, g.cursor, show_cursor, &state.colors, area, buf, state.config.honor_game_colours);
            None
        }
        WinNode::Buffer(b) => {
            if b.primary {
                let (scrollbar, max_scroll) =
                    render_transcript(status, introspect, state, area, buf);
                Some(StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: area.height })
            } else {
                render_inline_buffer(b, state, area, buf);
                None
            }
        }
        WinNode::Blank => {
            fill(area, buf, &state.colors);
            None
        }
    }
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

/// Draw an inline (non-primary) buffer window's wrapped, styled lines.
fn render_inline_buffer(b: &BufferWindow, state: &AppState, area: Rect, buf: &mut Buffer) {
    fill(area, buf, &state.colors);
    if b.lines.is_empty() {
        return;
    }
    let base = state.colors.transcript;
    let kinds = vec![TranscriptKind::Story; b.lines.len()];
    let styles = vec![base; b.lines.len()];
    let (rows, _total) = visible_wrapped_lines_kinded(
        &b.lines,
        &kinds,
        &styles,
        &b.runs,
        area.height as usize,
        b.scroll,
        area.width,
    );
    for (i, (line, _kind, style, runs)) in rows.iter().enumerate() {
        draw_str_runs(buf, area.x, area.y + i as u16, line, *style, runs, None, area, state.config.honor_game_colours.then_some(&state.colors));
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
        };
        assert!(is_simple(&zm));
        // Lone buffer: simple.
        let lone = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
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

    #[test]
    fn inline_buffer_renders_styled_runs() {
        let mut b = inline_buffer("abCD");
        b.runs = vec![vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0 }]];
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
        let (sb, ms) = render_transcript(&model.status, None, &state, tarea, &mut buf_b);

        assert_eq!(buf_a, buf_b, "the simple path must be byte-identical to the legacy path");
        assert_eq!((ma.scrollbar, ma.max_scroll, ma.viewport_rows), (sb, ms, tarea.height));
    }
}
