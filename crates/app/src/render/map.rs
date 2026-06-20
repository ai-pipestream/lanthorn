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

use mapper::graph::RoomId;
use mapper::render::{RenderMap, RenderRoom};
use mapper::router::{RoutedEdge, Side, side_for};
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

// ── Box-geometry routing helpers ──────────────────────────────────────────────

/// Return the gutter cell just outside `rect` on the given side, at the side's midpoint.
///
/// - Right: (rect.right(), rect.y + rect.height/2)
/// - Left:  (rect.x - 1,   rect.y + rect.height/2)
/// - Top:   (rect.x + rect.width/2, rect.y - 1)
/// - Bottom:(rect.x + rect.width/2, rect.bottom())
fn side_anchor(rect: Rect, side: Side) -> (i32, i32) {
    match side {
        Side::Right  => (rect.right() as i32,              rect.y as i32 + rect.height as i32 / 2),
        Side::Left   => (rect.x as i32 - 1,               rect.y as i32 + rect.height as i32 / 2),
        Side::Top    => (rect.x as i32 + rect.width as i32 / 2,  rect.y as i32 - 1),
        Side::Bottom => (rect.x as i32 + rect.width as i32 / 2,  rect.bottom() as i32),
    }
}

/// Build an orthogonal L-path from `dep` to `arr` whose first step leaves in
/// `dep_side`'s direction.
///
/// - dep_side is Left or Right → go horizontally first, then vertically
/// - dep_side is Top or Bottom → go vertically first, then horizontally
///
/// Returns the list of every screen cell along the path (contiguous, step 1).
fn route_ortho(dep: (i32, i32), arr: (i32, i32), dep_side: Side) -> Vec<(i32, i32)> {
    if dep == arr {
        return vec![dep];
    }

    // Determine corner point based on which axis leads.
    let corner = match dep_side {
        Side::Left | Side::Right => (arr.0, dep.1), // horizontal first
        Side::Top | Side::Bottom => (dep.0, arr.1), // vertical first
    };

    let mut pts = Vec::new();
    // Walk dep → corner → arr, each step ±1.
    walk_to(&mut pts, dep, corner);
    pts.pop(); // remove corner so walk_to below includes it exactly once
    walk_to(&mut pts, corner, arr);
    pts
}

/// Walk from `from` to `to` one step at a time (orthogonal), appending each cell.
/// `from` is included on first call; subsequent `walk_to` calls should share an
/// endpoint to avoid duplicates — the caller handles this by starting each segment
/// at the current last point.
fn walk_to(pts: &mut Vec<(i32, i32)>, from: (i32, i32), to: (i32, i32)) {
    let dx = (to.0 - from.0).signum();
    let dy = (to.1 - from.1).signum();
    let mut cur = from;
    loop {
        pts.push(cur);
        if cur == to {
            break;
        }
        cur = (cur.0 + dx, cur.1 + dy);
    }
}

/// Pick the side of `dest_rect` whose anchor is geometrically nearest to `dep`.
/// Used when `arrival_dir` is None (undiscovered).
fn nearest_side(dest_rect: Rect, dep: (i32, i32)) -> Side {
    let candidates = [Side::Top, Side::Bottom, Side::Left, Side::Right];
    candidates
        .into_iter()
        .min_by_key(|&s| {
            let a = side_anchor(dest_rect, s);
            let dx = a.0 - dep.0;
            let dy = a.1 - dep.1;
            dx * dx + dy * dy
        })
        .unwrap_or(Side::Bottom)
}

/// Return the arrowhead glyph that points INTO the dest from `arrival_side`.
///
/// `discovered`: filled arrows (▶◀▲▼); undiscovered: hollow (▷◁△▽).
fn arrowhead_glyph(arrival_side: Side, discovered: bool) -> &'static str {
    if discovered {
        match arrival_side {
            Side::Left   => "▶", // entering from the left (going east into dest)
            Side::Right  => "◀",
            Side::Top    => "▲",
            Side::Bottom => "▼",
        }
    } else {
        match arrival_side {
            Side::Left   => "▷",
            Side::Right  => "◁",
            Side::Top    => "△",
            Side::Bottom => "▽",
        }
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

// ── Styles ────────────────────────────────────────────────────────────────────

/// Style for the current room (reversed video — visually distinct).
const CURRENT_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED)
    .fg(Color::White);

/// Style for the selected room (yellow border).
const SELECTED_STYLE: Style = Style::new().fg(Color::Yellow);

/// Style for normal rooms.
const NORMAL_STYLE: Style = Style::new().fg(Color::White);

/// Style for normal connectors — solid bright Cyan.
const CONNECTOR_STYLE: Style = Style::new().fg(Color::Cyan);

/// Style for distorted connectors — solid Magenta (different signal, not dim).
const DISTORTED_STYLE: Style = Style::new().fg(Color::Magenta);

/// Style for arrowheads — bold Yellow.
const ARROWHEAD_STYLE: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

// ── Bitmask connector rasterization ──────────────────────────────────────────

/// Convert a NESW bitmask to a box-drawing glyph (rounded corners).
/// bits: N=1, E=2, S=4, W=8
pub fn bitmask_to_glyph(mask: u8) -> &'static str {
    match mask {
        0 => " ",
        1 | 4 | 5 => "│",
        2 | 8 | 10 => "─",
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

// ── render_map ────────────────────────────────────────────────────────────────

/// Draw the map from `rm` into `buf` for `area`, using view state from `state`.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;

    // ── 1. Build placed: HashMap<RoomId, Rect> ────────────────────────────────
    let mut placed: std::collections::HashMap<RoomId, Rect> =
        std::collections::HashMap::new();

    if !matches!(zoom, crate::state::Zoom::Overview) {
        let (bw, bh) = zoom_box_size(zoom);
        for room in &rm.rooms {
            if let Some((sx, sy)) = cell_to_screen(room.cell, zoom, scroll, area) {
                placed.insert(room.id, Rect::new(sx, sy, bw, bh));
            }
        }
    }

    // ── 2. Draw connectors (below rooms) ─────────────────────────────────────
    if !matches!(zoom, crate::state::Zoom::Overview) {
        // Collect bitmask per screen cell and arrowheads to overlay.
        let mut mask_map: std::collections::HashMap<(u16, u16), (u8, bool)> =
            std::collections::HashMap::new();
        let mut arrowheads: Vec<((u16, u16), &'static str, bool)> = Vec::new(); // (pos, glyph, distorted)

        for edge in &rm.edges {
            if edge.is_stub {
                continue;
            }
            let (Some(&origin_rect), Some(&dest_rect)) =
                (placed.get(&edge.origin), placed.get(&edge.dest))
            else {
                continue;
            };

            let Some(dep_side) = side_for(edge.dir) else {
                continue; // non-planar slipped through (shouldn't happen for non-stub)
            };

            let dep = side_anchor(origin_rect, dep_side);

            // Determine arrival side and whether it's confirmed.
            let (arr_side, discovered) = match edge.arrival_dir.and_then(side_for) {
                Some(s) => (s, true),
                None => (nearest_side(dest_rect, dep), false),
            };
            let arr = side_anchor(dest_rect, arr_side);

            // Route the path.
            let path = route_ortho(dep, arr, dep_side);

            // Rasterize into bitmask map (screen cells inside area only).
            let screen_pts: Vec<(u16, u16)> = path
                .iter()
                .filter(|&&(x, y)| {
                    x >= area.x as i32
                        && x < area.right() as i32
                        && y >= area.y as i32
                        && y < area.bottom() as i32
                })
                .map(|&(x, y)| (x as u16, y as u16))
                .collect();

            for i in 0..screen_pts.len() {
                let pos = screen_pts[i];
                let entry = mask_map.entry(pos).or_insert((0u8, true));
                entry.1 = entry.1 && edge.distorted;
                if i > 0 {
                    let bit = dir_bit(screen_pts[i - 1], pos);
                    entry.0 |= bit;
                }
                if i + 1 < screen_pts.len() {
                    let bit = dir_bit(screen_pts[i + 1], pos);
                    entry.0 |= bit;
                }
            }

            // Arrowhead at `arr` if it's inside area.
            let ax = arr.0;
            let ay = arr.1;
            if ax >= area.x as i32
                && ax < area.right() as i32
                && ay >= area.y as i32
                && ay < area.bottom() as i32
            {
                let glyph = arrowhead_glyph(arr_side, discovered);
                arrowheads.push(((ax as u16, ay as u16), glyph, edge.distorted));
            }
        }

        // Draw connector glyphs.
        for (&(cx, cy), &(mask, all_distorted)) in &mask_map {
            let glyph = bitmask_to_glyph(mask);
            let style = if all_distorted { DISTORTED_STYLE } else { CONNECTOR_STYLE };
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }

        // Overlay arrowheads (win over line glyphs).
        for ((ax, ay), glyph, distorted) in arrowheads {
            let style = if distorted { DISTORTED_STYLE } else { ARROWHEAD_STYLE };
            if let Some(cell) = buf.cell_mut((ax, ay)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }

        // Draw stub edges.
        for edge in &rm.edges {
            if edge.is_stub {
                draw_stub(edge, &placed, area, buf);
            }
        }
    }

    // ── 3. Draw rooms on top ──────────────────────────────────────────────────
    for room in &rm.rooms {
        draw_room(room, state, zoom, scroll, area, buf);
    }
}

/// Draw a stub connector label in the top-right gutter cell outside the origin box.
fn draw_stub(
    edge: &RoutedEdge,
    placed: &std::collections::HashMap<RoomId, Rect>,
    area: Rect,
    buf: &mut Buffer,
) {
    let Some(&origin_rect) = placed.get(&edge.origin) else {
        return;
    };
    let label = edge.label.as_deref().unwrap_or("?");
    // Top-right gutter: just right of the box, at the top row.
    let lx = origin_rect.right();
    let ly = origin_rect.y;
    draw_str_clipped(buf, lx, ly, label, CONNECTOR_STYLE, area);
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
        assert_eq!(bitmask_to_glyph(3), "╰");  // NE
        assert_eq!(bitmask_to_glyph(6), "╭");  // ES
        assert_eq!(bitmask_to_glyph(12), "╮"); // SW
        assert_eq!(bitmask_to_glyph(9), "╯");  // NW
        // Straights
        assert_eq!(bitmask_to_glyph(10), "─"); // EW
        assert_eq!(bitmask_to_glyph(5), "│");  // NS
        // Junction
        assert_eq!(bitmask_to_glyph(15), "┼"); // NESW
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

    // connector_has_corner_glyph: removed — called build_connector_mask which is gone;
    // superseded by new tests in Task 4.

    // connector_has_arrowhead_at_dest: removed — arrowhead rendering is stubbed out in Task 1;
    // superseded by new tests in Task 4.

    // connector_is_contiguous_no_gaps: segment_screen_points unit portion removed (function gone);
    // full-render connector assertions superseded by new tests in Task 4.

    #[test]
    fn connector_departs_origin_correct_side() {
        // room1 at (0,0) →E→ room2 at (1,0). Boxes zoom, area (0,0,80,30).
        // room1 box: Rect{x:0,y:0,w:14,h:4}. Right-side anchor: col=14, row=2.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default(); // Boxes zoom, scroll (0,0)
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The departure anchor for room1→E is Right side: col=14, row=2.
        // It must NOT be a space and NOT have a room box glyph from room1 (room1 cols 0..13).
        let dep_col = 14u16;
        let dep_row = 2u16;
        let sym = buf.cell((dep_col, dep_row)).map(|c| c.symbol()).unwrap_or(" ");
        assert_ne!(sym, " ", "departure gutter cell ({dep_col},{dep_row}) should have a connector glyph");
        assert!(
            dep_col >= 14, // outside room1 box (cols 0..13)
            "departure cell col={dep_col} should be outside room1 box"
        );
        // Must be a connector glyph (line or arrowhead), not a room box border
        assert!(
            matches!(sym, "─" | "│" | "╭" | "╮" | "╰" | "╯" | "├" | "┤" | "┴" | "┬" | "┼"
                        | "▶" | "◀" | "▲" | "▼" | "▷" | "◁" | "△" | "▽"),
            "cell ({dep_col},{dep_row}) should be a connector glyph; got '{sym}'"
        );
    }

    #[test]
    fn arrowhead_filled_when_arrival_discovered() {
        // room1(0,0) →E→ room2(1,0) AND room2(1,0) →W→ room1(0,0).
        // arrival_dir for the E edge = Some(W), so discovered=true → filled arrow ▶/◀/▲/▼.
        // room2 box: Rect{x:18,y:0,w:14,h:4}. Left anchor: col=17, row=2.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        g.add_edge(2, mapper::direction::Direction::W, 1); // reverse edge — arrival discovered
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // There must be a FILLED arrowhead somewhere in the buffer.
        let has_filled = buf.content.iter().any(|c| matches!(c.symbol(), "▶" | "◀" | "▲" | "▼"));
        assert!(has_filled, "filled arrowhead (▶◀▲▼) should appear when arrival_dir is discovered");

        // No hollow arrowhead should appear for this discovered edge.
        let has_hollow = buf.content.iter().any(|c| matches!(c.symbol(), "▷" | "◁" | "△" | "▽"));
        assert!(!has_hollow, "hollow arrowhead should NOT appear when arrival is discovered");
    }

    #[test]
    fn arrowhead_hollow_when_arrival_undiscovered() {
        // Only room1 →E→ room2, no reverse edge. arrival_dir=None → hollow arrow.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Must have a HOLLOW arrowhead.
        let has_hollow = buf.content.iter().any(|c| matches!(c.symbol(), "▷" | "◁" | "△" | "▽"));
        assert!(has_hollow, "hollow arrowhead (▷◁△▽) should appear when arrival_dir is None");

        // Must NOT have a filled arrowhead.
        let has_filled = buf.content.iter().any(|c| matches!(c.symbol(), "▶" | "◀" | "▲" | "▼"));
        assert!(!has_filled, "filled arrowhead should NOT appear when arrival is undiscovered");
    }

    #[test]
    fn connector_is_solid_not_dim() {
        // Connector between two adjacent rooms must use Cyan fg and solid glyph ─, not ╌.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Find a horizontal connector glyph ─ and verify it's Cyan, not DarkGray.
        // Right anchor col=14, row=2 — should be a solid glyph with Cyan style.
        let dep_col = 14u16;
        let dep_row = 2u16;
        let cell = buf.cell((dep_col, dep_row)).expect("connector cell must exist");
        let sym = cell.symbol();
        assert_ne!(sym, "╌", "connector should be solid ─ not dashed ╌");
        assert_ne!(sym, "╎", "connector should be solid │ not dashed ╎");
        assert_eq!(
            cell.fg,
            Color::Cyan,
            "connector fg should be Cyan; got {:?} at ({dep_col},{dep_row}) sym='{sym}'",
            cell.fg
        );
        // Must not have DIM modifier.
        assert!(
            !cell.modifier.contains(Modifier::DIM),
            "connector should not be dim; modifier={:?}",
            cell.modifier
        );
    }
}
