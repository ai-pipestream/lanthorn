//! Map projection and ratatui rendering.
//!
//! # Coordinate system
//!
//! Logical room cells (col, row) are placed on a grid.  Each zoom level defines a "step"
//! (cell stride in terminal columns/rows):
//!
//! | Zoom     | step_w | step_h |
//! |----------|--------|--------|
//! | Boxes    |  18    |   6    |
//! | Compact  |  10    |   4    |
//! | Overview |   2    |   2    |
//!
//! The screen position of a room at cell (cx, cy) with scroll (sx, sy) inside area `a` is:
//!   screen_x = a.x + (cx - sx) * step_w
//!   screen_y = a.y + (cy - sy) * step_h
//!
//! # Fine-grid connector projection
//!
//! Connectors live in a fine grid where room cell (c, r) → fine (2c, 2r).
//! A fine point (fx, fy) maps to screen as:
//!   screen_x = a.x + (fx - scroll.0 * 2) * (step_w / 2) + gutter_offset_x
//!   screen_y = a.y + (fy - scroll.1 * 2) * (step_h / 2) + gutter_offset_y
//!
//! For Overview zoom, connectors are skipped (step/2=1 but single-glyph boxes fill the cell).

use mapper::render::{RenderMap, RenderRoom};
use mapper::router::RoutedEdge;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::state::{AppState, Zoom};

// ── Step sizes and box dimensions ─────────────────────────────────────────────

/// Returns (step_w, step_h) for the given zoom level.
fn zoom_steps(zoom: Zoom) -> (i32, i32) {
    zoom.steps()
}

/// Returns (box_w, box_h): the visual size of a room box drawn within one cell step.
///
/// The box is drawn SMALLER than the step so there is a gutter on the right/bottom
/// where connector glyphs are visible between adjacent rooms.
///
/// | Zoom    | step  | box   | gutter (right / bottom) |
/// |---------|-------|-------|-------------------------|
/// | Boxes   | 18×6  | 14×4  | 4 cols / 2 rows         |
/// | Compact | 10×4  | 8×3   | 2 cols / 1 row          |
/// | Overview| 2×2   | 1×1   | — (single glyph)        |
fn zoom_box_size(zoom: Zoom) -> (u16, u16) {
    match zoom {
        Zoom::Boxes => (14, 4),
        Zoom::Compact => (8, 3),
        Zoom::Overview => (1, 1),
    }
}

/// Connector gutter offset added to fine_to_screen so that connector glyphs
/// land in the gutter columns/rows (not under a room box).
///
/// offset = box_size - step/2
fn connector_gutter_offset(zoom: Zoom) -> (i32, i32) {
    let (bw, bh) = zoom_box_size(zoom);
    let (sw, sh) = zoom_steps(zoom);
    (bw as i32 - sw / 2, bh as i32 - sh / 2)
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

/// Compute raw (unclipped) screen coordinates for a fine-grid point.
///
/// Returns the screen `(x, y)` using the same formula as `fine_to_screen` but without
/// bounds-checking against `area`.  Used by `segment_screen_points` so that endpoint
/// projections are available even when an endpoint lies just off the visible area.
fn fine_to_screen_raw(fine: (i32, i32), zoom: Zoom, scroll: (i32, i32), area: Rect) -> (i32, i32) {
    let (step_w, step_h) = zoom_steps(zoom);
    let half_w = step_w / 2;
    let half_h = step_h / 2;
    let fine_scroll_x = scroll.0 * 2;
    let fine_scroll_y = scroll.1 * 2;
    let (ox, oy) = connector_gutter_offset(zoom);
    let sx = area.x as i32 + (fine.0 - fine_scroll_x) * half_w + ox;
    let sy = area.y as i32 + (fine.1 - fine_scroll_y) * half_h + oy;
    (sx, sy)
}

/// Project a fine-grid point to screen coordinates.
///
/// Returns `None` if outside `area` or if zoom is Overview (connectors not drawn there).
///
/// The gutter offset is added so that connector glyphs (which traverse fine-grid
/// midpoints between adjacent rooms) land in the gutter columns/rows that are
/// left empty by the smaller room boxes, not under the box itself.
fn fine_to_screen(
    fine: (i32, i32),
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> Option<(u16, u16)> {
    if matches!(zoom, Zoom::Overview) {
        return None; // Overview: connectors not drawn
    }
    let (sx, sy) = fine_to_screen_raw(fine, zoom, scroll, area);
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

// ── Bitmask connector rasterization ──────────────────────────────────────────

/// Convert a NESW bitmask to a box-drawing glyph (rounded corners).
/// bits: N=1, E=2, S=4, W=8
pub fn bitmask_to_glyph(mask: u8, dashed: bool) -> &'static str {
    match mask {
        0 => " ",
        1 | 4 | 5 => {
            if dashed {
                "╎"
            } else {
                "│"
            }
        }
        2 | 8 | 10 => {
            if dashed {
                "╌"
            } else {
                "─"
            }
        }
        3 => "╰",   // NE
        6 => "╭",   // ES
        9 => "╯",   // NW
        12 => "╮",  // SW
        7 => "├",   // NES
        11 => "┴",  // NEW
        13 => "┤",  // NSW
        14 => "┬",  // ESW
        15 => "┼",  // NESW
        _ => " ",
    }
}

/// Given a "from" screen cell and "this" screen cell, return the direction bit
/// pointing from "this" toward "from":
/// - from is to the North of this (from.1 < this.1): N = 1
/// - from is to the East (from.0 > this.0): E = 2
/// - from is to the South (from.1 > this.1): S = 4
/// - from is to the West (from.0 < this.0): W = 8
fn dir_bit(from: (u16, u16), this: (u16, u16)) -> u8 {
    if from.1 < this.1 {
        1 // N
    } else if from.0 > this.0 {
        2 // E
    } else if from.1 > this.1 {
        4 // S
    } else {
        8 // W
    }
}

/// Return every screen cell between the screen projections of fine-grid points p0 and p1,
/// inclusive, stepping one screen cell at a time.
///
/// Segments are always orthogonal (dx==0 or dy==0 in fine space).  We project both
/// endpoints to raw (unclipped) screen coords via `fine_to_screen_raw`, then walk from
/// one to the other in screen space one cell at a time (±1 in x or y).  Each cell is
/// included only if it falls within `area`.  This guarantees a contiguous run of screen
/// cells with no gaps regardless of how many fine cells map to the same screen column/row.
fn segment_screen_points(
    p0: (i32, i32),
    p1: (i32, i32),
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> Vec<(u16, u16)> {
    let mut pts = Vec::new();

    // Raw (unclipped) screen coordinates of the two fine-grid endpoints.
    let (sx0, sy0) = fine_to_screen_raw(p0, zoom, scroll, area);
    let (sx1, sy1) = fine_to_screen_raw(p1, zoom, scroll, area);

    let sdx = (sx1 - sx0).signum();
    let sdy = (sy1 - sy0).signum();

    let mut cx = sx0;
    let mut cy = sy0;
    loop {
        // Include this screen cell only if it lies within the visible area.
        if cx >= area.x as i32
            && cx < area.right() as i32
            && cy >= area.y as i32
            && cy < area.bottom() as i32
        {
            pts.push((cx as u16, cy as u16));
        }
        if cx == sx1 && cy == sy1 {
            break;
        }
        cx += sdx;
        cy += sdy;
    }
    pts
}

/// Build a bitmask map for all non-stub routed edges.
/// Returns HashMap<(u16,u16), (u8, bool)> — (bitmask, all_distorted)
fn build_connector_mask(
    edges: &[RoutedEdge],
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> std::collections::HashMap<(u16, u16), (u8, bool)> {
    let mut map: std::collections::HashMap<(u16, u16), (u8, bool)> =
        std::collections::HashMap::new();

    if matches!(zoom, Zoom::Overview) {
        return map;
    }

    for edge in edges {
        if edge.is_stub {
            continue;
        }
        if edge.points.len() < 2 {
            continue;
        }

        // Walk consecutive pairs of fine-grid points
        for window in edge.points.windows(2) {
            let (p0, p1) = (window[0], window[1]);
            // Get all screen cells along this segment (inclusive p0..=p1)
            let screen_pts = segment_screen_points(p0, p1, zoom, scroll, area);
            // Walk consecutive screen cell pairs and set bitmasks
            for i in 0..screen_pts.len() {
                let pos = screen_pts[i];
                let entry = map.entry(pos).or_insert((0u8, true));
                // all_distorted only if all edges at this cell are distorted
                entry.1 = entry.1 && edge.distorted;

                if i > 0 {
                    let prev = screen_pts[i - 1];
                    // set bit on this cell pointing toward prev
                    let bit = dir_bit(prev, pos);
                    entry.0 |= bit;
                }
                if i < screen_pts.len() - 1 {
                    let next = screen_pts[i + 1];
                    // set bit on this cell pointing toward next
                    let bit = dir_bit(next, pos);
                    entry.0 |= bit;
                }
            }
        }
    }
    map
}

/// Collect arrowhead glyphs for all non-stub edges (one per edge, at last screen point).
fn collect_arrowheads(
    edges: &[RoutedEdge],
    zoom: Zoom,
    scroll: (i32, i32),
    area: Rect,
) -> Vec<((u16, u16), &'static str)> {
    if matches!(zoom, Zoom::Overview) {
        return Vec::new();
    }

    let mut arrows = Vec::new();
    for edge in edges {
        if edge.is_stub || edge.points.len() < 2 {
            continue;
        }
        // Last segment: from second-to-last point to last point
        let n = edge.points.len();
        let p_prev = edge.points[n - 2];
        let p_last = edge.points[n - 1];
        let dx = p_last.0 - p_prev.0;
        let dy = p_last.1 - p_prev.1;
        let glyph = if dx > 0 {
            "▸"
        } else if dx < 0 {
            "◂"
        } else if dy < 0 {
            "▴"
        } else {
            "▾"
        };
        // The arrowhead goes at the last point's screen position
        if let Some(screen_pos) = fine_to_screen(p_last, zoom, scroll, area) {
            arrows.push((screen_pos, glyph));
        }
    }
    arrows
}

// ── render_map ────────────────────────────────────────────────────────────────

/// Draw the map from `rm` into `buf` for `area`, using view state from `state`.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;

    // 1. Build bitmask map for all non-stub connectors
    let connector_map = build_connector_mask(&rm.edges, zoom, scroll, area);

    // 2. Render connector glyphs
    for (&(cx, cy), &(mask, all_distorted)) in &connector_map {
        let glyph = bitmask_to_glyph(mask, all_distorted);
        let style = if all_distorted {
            DISTORTED_STYLE
        } else {
            CONNECTOR_STYLE
        };
        if let Some(cell) = buf.cell_mut((cx, cy)) {
            cell.set_symbol(glyph).set_style(style);
        }
    }

    // 3. Overlay arrowheads
    let arrows = collect_arrowheads(&rm.edges, zoom, scroll, area);
    for ((ax, ay), glyph) in arrows {
        if let Some(cell) = buf.cell_mut((ax, ay)) {
            cell.set_symbol(glyph).set_style(CONNECTOR_STYLE);
        }
    }

    // 4. Draw stub edges
    for edge in &rm.edges {
        if edge.is_stub {
            draw_stub(edge, zoom, scroll, area, buf);
        }
    }

    // 5. Draw rooms on top
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
            draw_compact_room(room, sx, sy, base_style, area, buf);
        }
        Zoom::Boxes => {
            draw_box_room(room, sx, sy, base_style, area, buf);
        }
    }
}

/// Draw a compact (10×4 step) room: 8×3 box with label row.
///
/// Box is 8 cols wide × 3 rows tall (step 10×4, gutter = 2 cols right, 1 row bottom).
/// Normal rooms use rounded corners; current room uses heavy border with REVERSED style.
fn draw_compact_room(
    room: &RenderRoom,
    sx: u16,
    sy: u16,
    style: Style,
    area: Rect,
    buf: &mut Buffer,
) {
    let (bw, bh) = zoom_box_size(Zoom::Compact); // (8, 3)
    let is_current = style.add_modifier.contains(Modifier::REVERSED);

    let (tl, tr, bl, br, h, v) = if is_current {
        ('┏', '┓', '┗', '┛', '━', '┃')
    } else {
        ('╭', '╮', '╰', '╯', '─', '│')
    };

    // Top border
    draw_char_clipped(buf, sx, sy, tl, style, area);
    for dx in 1..bw - 1 {
        draw_char_clipped(buf, sx + dx, sy, h, style, area);
    }
    draw_char_clipped(buf, sx + bw - 1, sy, tr, style, area);

    // Middle row: sides + label (inner width = bw - 2 = 6)
    let label_width = (bw - 2) as usize; // 6
    let label: String = room.label.chars().take(label_width).collect();
    draw_char_clipped(buf, sx, sy + 1, v, style, area);
    draw_str_clipped(buf, sx + 1, sy + 1, &label, style, area);
    draw_char_clipped(buf, sx + bw - 1, sy + 1, v, style, area);

    // Bottom border
    draw_char_clipped(buf, sx, sy + bh - 1, bl, style, area);
    for dx in 1..bw - 1 {
        draw_char_clipped(buf, sx + dx, sy + bh - 1, h, style, area);
    }
    draw_char_clipped(buf, sx + bw - 1, sy + bh - 1, br, style, area);
}

/// Draw a boxes (18×6 step) room: bordered box 14 wide × 4 tall.
///
/// Layout (14 cols × 4 rows, within an 18×6 step):
///   Row 0: ╭────────────╮  (or ┏━━━━━━━━━━━━┓ for current room)
///   Row 1: │label......●│  (label up to 12 chars, ● if notes)
///   Row 2: │            │
///   Row 3: ╰────────────╯
///   Gutter: cols 14-17 (right), rows 4-5 (bottom)
///
/// Current room: heavy border (┏ ┓ ┗ ┛ ━ ┃) with REVERSED style.
/// Selected room: yellow style (SELECTED_STYLE).
/// Notes: ● marker in top-right inner corner (row 1, col bw-2).
fn draw_box_room(
    room: &RenderRoom,
    sx: u16,
    sy: u16,
    style: Style,
    area: Rect,
    buf: &mut Buffer,
) {
    let (w, h) = zoom_box_size(Zoom::Boxes); // (14, 4)
    let is_current = style.add_modifier.contains(Modifier::REVERSED);

    let (tl, tr, bl, br, horiz, vert) = if is_current {
        ('┏', '┓', '┗', '┛', '━', '┃')
    } else {
        ('╭', '╮', '╰', '╯', '─', '│')
    };

    // Top border
    draw_char_clipped(buf, sx, sy, tl, style, area);
    for dx in 1..w - 1 {
        draw_char_clipped(buf, sx + dx, sy, horiz, style, area);
    }
    draw_char_clipped(buf, sx + w - 1, sy, tr, style, area);

    // Inner rows (h=4 → rows 1 and 2 are interior)
    for dy in 1..h - 1 {
        draw_char_clipped(buf, sx, sy + dy, vert, style, area);
        // Fill interior with spaces (for background/style)
        for dx in 1..w - 1 {
            draw_char_clipped(buf, sx + dx, sy + dy, ' ', style, area);
        }
        draw_char_clipped(buf, sx + w - 1, sy + dy, vert, style, area);
    }

    // Label on row 1 (first inner row), up to w-2 = 12 chars.
    let label_width = (w - 2) as usize; // 12
    let label: String = room.label.chars().take(label_width).collect();
    draw_str_clipped(buf, sx + 1, sy + 1, &label, style, area);

    // Notes marker ● in top-right inner corner (row 1, col w-2).
    if room.has_notes {
        draw_char_clipped(buf, sx + w - 2, sy + 1, '●', style, area);
    }

    // Bottom border
    draw_char_clipped(buf, sx, sy + h - 1, bl, style, area);
    for dx in 1..w - 1 {
        draw_char_clipped(buf, sx + dx, sy + h - 1, horiz, style, area);
    }
    draw_char_clipped(buf, sx + w - 1, sy + h - 1, br, style, area);
}

// ── Stub drawing ──────────────────────────────────────────────────────────────

/// Draw a stub connector: dashed line from origin toward stub endpoint, label at midpoint.
fn draw_stub(edge: &RoutedEdge, zoom: Zoom, scroll: (i32, i32), area: Rect, buf: &mut Buffer) {
    if edge.points.len() < 2 {
        return;
    }
    let p0 = edge.points[0]; // origin fine cell
    let p1 = edge.points[1]; // stub end fine cell
    let style = CONNECTOR_STYLE;

    let dx = (p1.0 - p0.0).signum();
    let dy = (p1.1 - p0.1).signum();
    let h_glyph = "╌";
    let v_glyph = "╎";

    let mut cur = p0;
    let mut mid_screen: Option<(u16, u16)> = None;
    let mut count = 0i32;
    let total = (p1.0 - p0.0).abs().max((p1.1 - p0.1).abs()) + 1;

    loop {
        if let Some((sx, sy)) = fine_to_screen(cur, zoom, scroll, area) {
            // Draw the dashed glyph (not at first point which is under the room box)
            if count > 0 {
                let glyph = if dx != 0 { h_glyph } else { v_glyph };
                if let Some(cell) = buf.cell_mut((sx, sy)) {
                    cell.set_symbol(glyph).set_style(style);
                }
            }
            // Midpoint for label
            if count == total / 2 {
                mid_screen = Some((sx, sy));
            }
        }
        if cur == p1 {
            break;
        }
        cur = (cur.0 + dx, cur.1 + dy);
        count += 1;
    }

    // Draw label at midpoint (overwriting dashed glyph)
    if let Some((mx, my)) = mid_screen.or_else(|| fine_to_screen(p1, zoom, scroll, area)) {
        let label = edge.label.as_deref().unwrap_or("?");
        draw_str_clipped(buf, mx, my, label, style, area);
    }
}

// ── Clipped drawing helpers ───────────────────────────────────────────────────

use super::{draw_char_clipped, draw_str_clipped};

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

        // Cell (1,0) at Boxes → x = 0 + (1-0)*18 = 18
        let right = cell_to_screen((1, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(right, Some((18, 0)));

        // Cell (0,1) at Boxes → y = 0 + (1-0)*6 = 6
        let down = cell_to_screen((0, 1), Zoom::Boxes, (0, 0), area);
        assert_eq!(down, Some((0, 6)));

        // Far off-area cell.
        let off = cell_to_screen((1000, 1000), Zoom::Boxes, (0, 0), area);
        assert!(off.is_none());

        // Scroll pushes cell off-screen: scroll=(1,0) so cell (0,0) → x = 0+(0-1)*18 = -18 → None.
        let scrolled_off = cell_to_screen((0, 0), Zoom::Boxes, (1, 0), area);
        assert!(scrolled_off.is_none());

        // Compact zoom: step 10×4 → cell (1,1) → (10, 4)
        let compact = cell_to_screen((1, 1), Zoom::Compact, (0, 0), area);
        assert_eq!(compact, Some((10, 4)));

        // Overview zoom: step 2×2 → cell (5,3) → (10, 6)
        let overview = cell_to_screen((5, 3), Zoom::Overview, (0, 0), area);
        assert_eq!(overview, Some((10, 6)));
    }

    #[test]
    fn renders_current_room_highlighted_into_buffer() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        m.observe(2, "North", Some(Direction::N));
        let rm = render(&m.graph);
        // room 2 ("North") is placed at cell (0, -1) by the layout engine.
        // With default scroll (0,0) and Boxes zoom (step_h=6), its screen y = -6 (off screen).
        // Scroll up by 1 row so that cell (0,-1) maps to screen y=0.
        let mut state = AppState::default();
        state.scroll = (0, -1); // scroll y=-1 so cell (0,-1) → screen y = 0 + (-1-(-1))*6 = 0

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
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Count box-drawing and rounded corner characters.
        let box_drawing: usize = buf
            .content
            .iter()
            .filter(|c| {
                matches!(
                    c.symbol(),
                    "─" | "│" | "╭" | "╮" | "╰" | "╯" | "┏" | "┓" | "┗" | "┛" | "━" | "┃"
                )
            })
            .count();
        // The room boxes themselves use these chars too — we just need more than zero.
        assert!(box_drawing > 0, "should have box-drawing chars from rooms or connectors");

        // NEW: Boxes step (18,6), half=(9,3), box=(14,4), gutter ox = 14 - 9 = 5, oy = 4 - 3 = 1
        // Room "Start" at cell (0,0) → screen (0,0), box covers cols 0..13, rows 0..3.
        // Room "East"  at cell (1,0) → screen (18,0), box covers cols 18..31.
        // Gutter between boxes: cols 14-17.
        //
        // Connector fine(1,0) → sx = 0 + 1*9 + 5 = 14, sy = 0 + 0*3 + 1 = 1.
        let gutter_x = 14u16;
        let gutter_y = 1u16;
        let connector_cell = buf.cell((gutter_x, gutter_y));
        assert!(
            connector_cell.is_some(),
            "gutter cell ({gutter_x},{gutter_y}) should exist in buffer"
        );
        let sym = connector_cell.unwrap().symbol();
        assert!(
            matches!(sym, "─" | "│" | "╌" | "╎" | "▸" | "◂" | "▴" | "▾"),
            "connector glyph at ({gutter_x},{gutter_y}) should be a connector char; got '{sym}'"
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

        // Notes marker '●' should appear somewhere in the buffer.
        let has_notes_marker = buf.content.iter().any(|c| c.symbol() == "●");
        assert!(has_notes_marker, "notes marker '●' should be drawn for a room with notes");
    }

    #[test]
    fn recenter_keeps_cell_on_screen() {
        // After recenter_on(cell, pane_w, pane_h), cell_to_screen must return
        // Some((x,y)) that lies inside the area — proving the map is not blank.
        let area = Rect::new(40, 0, 40, 24); // right-half pane, x offset 40
        let cell = (0_i32, 0_i32);

        let mut state = AppState::default(); // Boxes zoom
        state.recenter_on(cell, area.width, area.height);

        let result = cell_to_screen(cell, state.zoom, state.scroll, area);
        assert!(
            result.is_some(),
            "cell_to_screen should return Some after recenter_on; scroll={:?}",
            state.scroll
        );
        let (sx, sy) = result.unwrap();
        assert!(
            sx >= area.x && sx < area.right() && sy >= area.y && sy < area.bottom(),
            "screen position ({sx},{sy}) should be inside area {area:?}"
        );
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

    #[test]
    fn bitmask_to_glyph_lookup() {
        // Corners
        assert_eq!(bitmask_to_glyph(3, false), "╰");  // NE
        assert_eq!(bitmask_to_glyph(6, false), "╭");  // ES
        assert_eq!(bitmask_to_glyph(12, false), "╮"); // SW
        assert_eq!(bitmask_to_glyph(9, false), "╯");  // NW
        // Straights
        assert_eq!(bitmask_to_glyph(10, false), "─"); // EW
        assert_eq!(bitmask_to_glyph(5, false), "│");  // NS
        // Junction
        assert_eq!(bitmask_to_glyph(15, false), "┼"); // NESW
        // Dashed straight
        assert_eq!(bitmask_to_glyph(10, true), "╌");
        assert_eq!(bitmask_to_glyph(5, true), "╎");
    }

    #[test]
    fn room_box_shows_label_at_boxes_zoom() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "West of House".into());
        g.set_pos(1, (0, 0));
        let rm = render(&g);
        let state = AppState::default(); // Boxes zoom
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The box is 14 wide × 4 tall at (0,0). Inner area is cols 1..13, rows 1..3.
        // Label "West of House" truncated to 12 chars = "West of Hous"
        // Should find 'W', 'e', 's', 't' at row 1, cols 1..4
        let row1_chars: String = (1u16..=12).map(|x| {
            buf.cell((x, 1)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')
        }).collect();
        assert!(row1_chars.contains("West"), "label row should contain 'West'; got '{row1_chars}'");
    }

    #[test]
    fn connector_has_corner_glyph() {
        // Build connector mask for a hand-crafted L edge
        let edge = mapper::router::RoutedEdge {
            origin: 1,
            dest: 2,
            dir: mapper::direction::Direction::E,
            points: vec![(1, 0), (3, 0), (3, 2)], // fine-grid L shape
            distorted: false,
            is_stub: false,
            label: None,
            arrival_dir: None,
        };
        let scroll = (0, 0);
        let zoom = Zoom::Boxes;
        let area = Rect::new(0, 0, 80, 30);
        let connector_map = build_connector_mask(&[edge], zoom, scroll, area);
        // The corner cell should have a non-straight mask (SW=12, or similar corner)
        let has_bend = connector_map.values().any(|&(mask, _)| {
            matches!(mask, 3 | 6 | 9 | 12) // NE, ES, NW, SW corners
        });
        assert!(has_bend, "L-shaped connector should produce a bend/corner in the bitmask map");
    }

    #[test]
    fn connector_has_arrowhead_at_dest() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        m.observe(2, "East", Some(Direction::E));
        let rm = render(&m.graph);
        let state = AppState::default(); // Boxes zoom, scroll (0,0)
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // One of these arrowhead glyphs should appear
        let has_arrow = buf.content.iter().any(|c| {
            matches!(c.symbol(), "▸" | "◂" | "▴" | "▾")
        });
        assert!(has_arrow, "connector to dest room should have an arrowhead glyph");
    }

    /// Unit-test `segment_screen_points` directly: for a horizontal fine segment whose
    /// screen endpoints are N cells apart, the function must return exactly N+1 cells,
    /// each adjacent (distance 1) to the previous — no gaps.
    #[test]
    fn connector_is_contiguous_no_gaps() {
        // Boxes zoom: step_w=18, half_w=9.
        // A fine segment from (0,0) to (2,0) spans two fine steps → 2*9 = 18 screen cols apart.
        // segment_screen_points must return 19 contiguous screen cells.
        let zoom = Zoom::Boxes;
        let scroll = (0, 0);
        let area = Rect::new(0, 0, 120, 40);

        let pts = segment_screen_points((0, 0), (2, 0), zoom, scroll, area);

        // All cells returned must be visible in the area.
        for &(x, y) in &pts {
            assert!(
                x >= area.x && x < area.right() && y >= area.y && y < area.bottom(),
                "cell ({x},{y}) outside area"
            );
        }

        // Must be non-empty.
        assert!(!pts.is_empty(), "segment_screen_points returned no points");

        // Consecutive cells must be exactly 1 apart (no gaps, no duplicates).
        for w in pts.windows(2) {
            let (x0, y0) = (w[0].0 as i32, w[0].1 as i32);
            let (x1, y1) = (w[1].0 as i32, w[1].1 as i32);
            let dist = (x1 - x0).abs() + (y1 - y0).abs();
            assert_eq!(
                dist, 1,
                "consecutive screen cells ({},{}) and ({},{}) are not adjacent (dist={dist})",
                w[0].0, w[0].1, w[1].0, w[1].1
            );
        }

        // Full-buffer render: place rooms 2 cells apart so the connector is a multi-cell
        // horizontal run and verify no space interrupts it.
        //
        // Manually build a graph with rooms at (0,0) and (2,0).
        // Boxes zoom: step_w=18, half_w=9, gutter_ox=5.
        // dep = fine(1,0) -> screen x = 1*9+5 = 14
        // arr = fine(3,0) -> screen x = 3*9+5 = 32
        // segment_screen_points walk produces cols 14..=32 inclusive (19 cells) at row 1.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0)); // two cells apart, not adjacent
        g.add_edge(1, mapper::direction::Direction::E, 2);
        let rm2 = render(&g);
        let state2 = AppState::default();
        let buf_area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(buf_area);
        render_map(&rm2, &state2, buf_area, &mut buf);

        // Connector row is row 1 (oy=1 for Boxes).
        // dep fine(1,0) -> screen x=14; arr fine(3,0) -> screen x=32 (arrowhead).
        // Check cols 14..=31 (before the arrowhead at 32) are all non-space.
        let connector_row = 1u16;
        let seg_start = 14u16;
        let seg_end = 31u16; // stop one before the arrowhead at col 32

        for col in seg_start..=seg_end {
            let sym = buf.cell((col, connector_row)).map(|c| c.symbol()).unwrap_or(" ");
            assert_ne!(
                sym, " ",
                "connector cell ({col},{connector_row}) is a space -- gap in multi-cell connector"
            );
        }
    }
}
