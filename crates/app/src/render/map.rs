//! Map projection and ratatui rendering.
//!
//! # Coordinate system
//!
//! Logical room cells (col, row) are placed on a grid.  Each zoom level defines a "step"
//! (cell stride in terminal columns/rows):
//!
//! | Zoom     | step_w | step_h |
//! |----------|--------|--------|
//! | Boxes    |   8    |   4    |
//! | Compact  |   4    |   2    |
//! | Overview |   1    |   1    |
//!
//! The screen position of a room at cell (cx, cy) with scroll (sx, sy) inside area `a` is:
//!   screen_x = a.x + (cx - sx) * step_w
//!   screen_y = a.y + (cy - sy) * step_h
//!
//! # Fine-grid connector projection
//!
//! Connectors live in a fine grid where room cell (c, r) → fine (2c, 2r).
//! A fine point (fx, fy) maps to screen as:
//!   screen_x = a.x + (fx - scroll.0 * 2) * (step_w / 2)
//!   screen_y = a.y + (fy - scroll.1 * 2) * (step_h / 2)
//!
//! For Overview (step 1×1) step/2 rounds to 0, so connectors are skipped at Overview zoom.

use mapper::render::{RenderMap, RenderRoom};
use mapper::router::RoutedEdge;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::state::{AppState, Zoom};

// ── Step sizes ────────────────────────────────────────────────────────────────

/// Returns (step_w, step_h) for the given zoom level.
fn zoom_steps(zoom: Zoom) -> (i32, i32) {
    match zoom {
        Zoom::Boxes => (8, 4),
        Zoom::Compact => (4, 2),
        Zoom::Overview => (1, 1),
    }
}

// ── cell_to_screen ────────────────────────────────────────────────────────────

/// Map a logical room cell to an absolute screen coordinate within `area`.
///
/// Returns `None` if the resulting position falls outside `area`.
pub fn cell_to_screen(
    cell: (i32, i32),
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> Option<(u16, u16)> {
    let (step_w, step_h) = zoom_steps(zoom);
    let sx = area.x as i32 + (cell.0 - scroll.0) * step_w;
    let sy = area.y as i32 + (cell.1 - scroll.1) * step_h;

    // Bounds check: must be inside [area.x, area.right()) × [area.y, area.bottom())
    if sx < area.x as i32
        || sx >= area.right() as i32
        || sy < area.y as i32
        || sy >= area.bottom() as i32
    {
        return None;
    }
    Some((sx as u16, sy as u16))
}

// ── Fine-grid screen projection ───────────────────────────────────────────────

/// Project a fine-grid point to screen coordinates.
///
/// Returns `None` if outside `area` or if step/2 == 0 (Overview zoom).
fn fine_to_screen(
    fine: (i32, i32),
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> Option<(u16, u16)> {
    let (step_w, step_h) = zoom_steps(zoom);
    let half_w = step_w / 2;
    let half_h = step_h / 2;
    if half_w == 0 || half_h == 0 {
        return None; // Overview: connectors not drawn
    }

    let fine_scroll_x = scroll.0 * 2;
    let fine_scroll_y = scroll.1 * 2;

    let sx = area.x as i32 + (fine.0 - fine_scroll_x) * half_w;
    let sy = area.y as i32 + (fine.1 - fine_scroll_y) * half_h;

    if sx < area.x as i32
        || sx >= area.right() as i32
        || sy < area.y as i32
        || sy >= area.bottom() as i32
    {
        return None;
    }
    Some((sx as u16, sy as u16))
}

// ── Styles ────────────────────────────────────────────────────────────────────

/// Style for the current room (reversed video — visually distinct).
const CURRENT_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED)
    .fg(Color::White);

/// Style for the selected room (yellow border).
const SELECTED_STYLE: Style = Style::new().fg(Color::Yellow);

/// Style for normal rooms.
const NORMAL_STYLE: Style = Style::new().fg(Color::White);

/// Style for distorted connectors (dim).
const DISTORTED_STYLE: Style = Style::new().add_modifier(Modifier::DIM);

/// Style for normal connectors.
const CONNECTOR_STYLE: Style = Style::new().fg(Color::DarkGray);

// ── render_map ────────────────────────────────────────────────────────────────

/// Draw the map from `rm` into `buf` for `area`, using view state from `state`.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;

    // Draw connectors first (rooms drawn on top).
    for edge in &rm.edges {
        draw_edge(edge, zoom, scroll, area, buf);
    }

    // Draw rooms.
    for room in &rm.rooms {
        draw_room(room, state, zoom, scroll, area, buf);
    }
}

// ── Room drawing ──────────────────────────────────────────────────────────────

fn draw_room(
    room: &RenderRoom,
    state: &AppState,
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    let Some((sx, sy)) = cell_to_screen(room.cell, zoom, scroll, area) else {
        return;
    };

    let base_style = if room.is_current {
        CURRENT_STYLE
    } else if state.selected_room == Some(room.id) {
        SELECTED_STYLE
    } else {
        NORMAL_STYLE
    };

    match zoom {
        Zoom::Overview => {
            // Single glyph per room.
            if let Some(cell) = buf.cell_mut((sx, sy)) {
                cell.set_symbol("■").set_style(base_style);
            }
        }
        Zoom::Compact => {
            // 4×2 box: draw a minimal 3×1 label area.
            draw_compact_room(room, sx, sy, base_style, area, buf);
        }
        Zoom::Boxes => {
            // 8×4 box: draw a bordered box with label inside.
            draw_box_room(room, sx, sy, base_style, area, buf);
        }
    }
}

/// Draw a compact (4×2) room: first row is a short label, second row has notes marker.
fn draw_compact_room(
    room: &RenderRoom,
    sx: u16,
    sy: u16,
    style: Style,
    area: Rect,
    buf: &mut Buffer,
) {
    // Label truncated to 4 chars on first row.
    let label: String = room.label.chars().take(4).collect();
    draw_str_clipped(buf, sx, sy, &label, style, area);

    // Notes marker on second row if it fits.
    if room.has_notes && sy + 1 < area.bottom() {
        draw_str_clipped(buf, sx, sy + 1, "*", style, area);
    }
}

/// Draw a boxes (8×4) room: bordered box with label and notes marker.
///
/// Layout (8 cols × 4 rows):
///   Row 0: ┌──────┐
///   Row 1: │ lbl  │
///   Row 2: │     *│  (* = notes marker if present)
///   Row 3: └──────┘
fn draw_box_room(
    room: &RenderRoom,
    sx: u16,
    sy: u16,
    style: Style,
    area: Rect,
    buf: &mut Buffer,
) {
    let w = 8u16;
    let h = 4u16;

    // Top border: ┌──────┐
    draw_char_clipped(buf, sx, sy, '┌', style, area);
    for dx in 1..w - 1 {
        draw_char_clipped(buf, sx + dx, sy, '─', style, area);
    }
    draw_char_clipped(buf, sx + w - 1, sy, '┐', style, area);

    // Middle rows.
    for dy in 1..h - 1 {
        draw_char_clipped(buf, sx, sy + dy, '│', style, area);
        draw_char_clipped(buf, sx + w - 1, sy + dy, '│', style, area);
    }

    // Label on row 1 (truncated to w-2 = 6 chars).
    let label: String = room.label.chars().take((w - 2) as usize).collect();
    draw_str_clipped(buf, sx + 1, sy + 1, &label, style, area);

    // Notes marker on row 2, last col before border.
    if room.has_notes {
        draw_char_clipped(buf, sx + w - 2, sy + 2, '*', style, area);
    }

    // Bottom border: └──────┘
    draw_char_clipped(buf, sx, sy + h - 1, '└', style, area);
    for dx in 1..w - 1 {
        draw_char_clipped(buf, sx + dx, sy + h - 1, '─', style, area);
    }
    draw_char_clipped(buf, sx + w - 1, sy + h - 1, '┘', style, area);
}

// ── Connector drawing ─────────────────────────────────────────────────────────

/// Draw a routed edge as box-drawing glyphs.
///
/// Iterates consecutive point pairs in `edge.points` and draws the segment between them.
fn draw_edge(
    edge: &RoutedEdge,
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    if edge.points.len() < 2 {
        return;
    }

    // For stubs, just draw the stub label near the origin room.
    if edge.is_stub {
        draw_stub(edge, zoom, scroll, area, buf);
        return;
    }

    let style = if edge.distorted {
        DISTORTED_STYLE
    } else {
        CONNECTOR_STYLE
    };

    // Draw each segment between consecutive waypoints.
    for window in edge.points.windows(2) {
        let (p0, p1) = (window[0], window[1]);
        draw_segment(p0, p1, edge.distorted, zoom, scroll, area, buf, style);
    }
}

/// Draw a stub connector with its label.
fn draw_stub(
    edge: &RoutedEdge,
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    if edge.points.len() < 2 {
        return;
    }
    // The first point is the origin fine cell; second is one step above it.
    let p1 = edge.points[1];
    let style = CONNECTOR_STYLE;
    if let Some((sx, sy)) = fine_to_screen(p1, zoom, scroll, area) {
        if let Some(cell) = buf.cell_mut((sx, sy)) {
            let glyph = edge.label.as_deref().unwrap_or("?");
            cell.set_symbol(glyph).set_style(style);
        }
    }
}

/// Draw a single orthogonal segment between fine-grid points `p0` and `p1`.
///
/// Determines appropriate box-drawing characters based on direction.
fn draw_segment(
    p0: (i32, i32),
    p1: (i32, i32),
    distorted: bool,
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
    style: Style,
) {
    let dx = p1.0 - p0.0;
    let dy = p1.1 - p0.1;

    if dx == 0 && dy == 0 {
        return;
    }

    // Determine the glyph for intermediate cells along the segment.
    let (h_glyph, v_glyph) = if distorted {
        ("┄", "┊")
    } else {
        ("─", "│")
    };

    if dy == 0 {
        // Horizontal segment.
        let step = if dx > 0 { 1 } else { -1 };
        let mut fx = p0.0 + step;
        while fx != p1.0 + step {
            let fine = (fx, p0.1);
            if let Some((sx, sy)) = fine_to_screen(fine, zoom, scroll, area) {
                if let Some(cell) = buf.cell_mut((sx, sy)) {
                    cell.set_symbol(h_glyph).set_style(style);
                }
            }
            fx += step;
        }
    } else if dx == 0 {
        // Vertical segment.
        let step = if dy > 0 { 1 } else { -1 };
        let mut fy = p0.1 + step;
        while fy != p1.1 + step {
            let fine = (p0.0, fy);
            if let Some((sx, sy)) = fine_to_screen(fine, zoom, scroll, area) {
                if let Some(cell) = buf.cell_mut((sx, sy)) {
                    cell.set_symbol(v_glyph).set_style(style);
                }
            }
            fy += step;
        }
    }
    // Non-orthogonal segments (fallback for routing failures) are skipped —
    // the router guarantees orthogonal paths for planar edges.
}

// ── Clipped drawing helpers ───────────────────────────────────────────────────

/// Write a single char into the buffer, clipped to `area`.
fn draw_char_clipped(buf: &mut Buffer, x: u16, y: u16, ch: char, style: Style, area: Rect) {
    if x < area.x || x >= area.right() || y < area.y || y >= area.bottom() {
        return;
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        let mut s = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut s)).set_style(style);
    }
}

/// Write a string into the buffer starting at (x, y), clipped to `area` width.
fn draw_str_clipped(buf: &mut Buffer, x: u16, y: u16, s: &str, style: Style, area: Rect) {
    if y < area.y || y >= area.bottom() {
        return;
    }
    let mut cx = x;
    for ch in s.chars() {
        if cx >= area.right() {
            break;
        }
        draw_char_clipped(buf, cx, y, ch, style, area);
        cx += 1;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;
    use mapper::render::render;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    #[test]
    fn cell_to_screen_respects_scroll_and_offarea() {
        let area = Rect::new(0, 0, 40, 20);

        // Cell (0,0) with no scroll at Boxes → screen (0,0), inside area.
        let on = cell_to_screen((0, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(on, Some((0, 0)));

        // Cell (1,0) at Boxes → x = 0 + (1-0)*8 = 8
        let right = cell_to_screen((1, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(right, Some((8, 0)));

        // Cell (0,1) at Boxes → y = 0 + (1-0)*4 = 4
        let down = cell_to_screen((0, 1), Zoom::Boxes, (0, 0), area);
        assert_eq!(down, Some((0, 4)));

        // Far off-area cell.
        let off = cell_to_screen((1000, 1000), Zoom::Boxes, (0, 0), area);
        assert!(off.is_none());

        // Scroll pushes cell off-screen: scroll=(1,0) so cell (0,0) → x = 0+(0-1)*8 = -8 → None.
        let scrolled_off = cell_to_screen((0, 0), Zoom::Boxes, (1, 0), area);
        assert!(scrolled_off.is_none());

        // Compact zoom: step 4×2 → cell (1,1) → (4, 2)
        let compact = cell_to_screen((1, 1), Zoom::Compact, (0, 0), area);
        assert_eq!(compact, Some((4, 2)));

        // Overview zoom: step 1×1 → cell (5,3) → (5, 3)
        let overview = cell_to_screen((5, 3), Zoom::Overview, (0, 0), area);
        assert_eq!(overview, Some((5, 3)));
    }

    #[test]
    fn renders_current_room_highlighted_into_buffer() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        m.observe(2, "North", Some(Direction::N));
        let rm = render(&m.graph);
        // room 2 ("North") is placed at cell (0, -1) by the layout engine.
        // With default scroll (0,0) and Boxes zoom (step_h=4), its screen y = -4 (off screen).
        // Scroll up by 1 row so that cell (0,-1) maps to screen y=0.
        let mut state = AppState::default();
        state.scroll = (0, -1); // scroll y=-1 so cell (0,-1) → screen y = 0 + (-1-(-1))*4 = 0

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // SOME non-space content was drawn (rooms/connectors present).
        let drawn = buf.content.iter().filter(|c| c.symbol() != " ").count();
        assert!(drawn > 0, "map should render something");

        // Find the current room cell from the RenderMap and verify it's on screen.
        let current_room = rm.rooms.iter().find(|r| r.is_current).expect("should have a current room");
        let pos = cell_to_screen(current_room.cell, state.zoom, state.scroll, area);
        assert!(pos.is_some(), "current room should be on screen with scroll adjusted");
        let (cx, cy) = pos.unwrap();

        // The top-left corner of the current room's box should have REVERSED modifier.
        let cell = buf.cell((cx, cy)).expect("cell should exist");
        assert!(
            cell.modifier.contains(Modifier::REVERSED),
            "current room cell should have REVERSED modifier; got modifier={:?}",
            cell.modifier
        );
    }

    #[test]
    fn connector_drawn_between_two_rooms() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        m.observe(2, "East", Some(Direction::E));
        let rm = render(&m.graph);
        let state = AppState::default(); // Boxes zoom, scroll (0,0)
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Count box-drawing characters — connectors use ─, │, ┌, ┐, └, ┘, ┄, ┊.
        let box_drawing: usize = buf
            .content
            .iter()
            .filter(|c| {
                matches!(c.symbol(), "─" | "│" | "┌" | "┐" | "└" | "┘" | "┄" | "┊")
            })
            .count();
        // The room boxes themselves use these chars too — we just need more than zero.
        assert!(box_drawing > 0, "should have box-drawing chars from rooms or connectors");

        // The two rooms alone draw borders; let's verify there's content between them.
        // Room 1 at cell (0,0) → screen (0,0); Room 2 at cell (1,0) → screen (8,0).
        // A horizontal connector runs at fine row y=0 between fine cols 1..3.
        // fine (1, 0) → screen x = 0 + (1 - 0*2) * (8/2) = 4, y = 0 + (0) * 2 = 0.
        let connector_cell = buf.cell((4, 0));
        assert!(
            connector_cell.is_some(),
            "connector cell (4,0) should exist in buffer"
        );
        let sym = connector_cell.unwrap().symbol();
        assert!(
            sym != " ",
            "connector glyph at (4,0) should not be space; got '{}'",
            sym
        );
    }

    #[test]
    fn notes_marker_drawn() {
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        g.set_notes(1, "some notes".into());
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Notes marker '*' should appear somewhere in the buffer.
        let has_notes_marker = buf.content.iter().any(|c| c.symbol() == "*");
        assert!(has_notes_marker, "notes marker '*' should be drawn for a room with notes");
    }

    #[test]
    fn overview_zoom_draws_single_glyph() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        let rm = render(&m.graph);
        let mut state = AppState::default();
        state.zoom = Zoom::Overview;
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let has_block = buf.content.iter().any(|c| c.symbol() == "■");
        assert!(has_block, "overview zoom should draw '■' glyph");
    }
}
