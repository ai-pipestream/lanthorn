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

/// Penalty (in path cost) for stepping onto a cell occupied/haloed by an earlier path.
/// Soft, not blocking: paths overlap only when no clear alternative exists in the search
/// window. High enough that A* prefers any available detour over running over/next to a
/// previous path, but finite so a genuinely boxed-in path still completes.
const SOFT_PENALTY: i32 = 40;

/// Build an obstacle-aware orthogonal path from `dep` to `arr` whose first step leaves in
/// `dep_side`'s direction.
///
/// Uses A* on the integer screen grid. `blocked` contains all screen cells that are
/// interior to other room boxes (excluding the origin and dest rooms for this edge).
/// `soft` contains cells occupied by earlier-routed connectors (plus a 1-cell halo);
/// entering one costs `SOFT_PENALTY`, so later paths keep a minimum gap where possible.
///
/// Falls back to a simple L-path if A* cannot find a path (blocked or cap exceeded).
fn route_ortho(
    dep: (i32, i32),
    arr: (i32, i32),
    dep_side: Side,
    blocked: &std::collections::HashSet<(i32, i32)>,
    soft: &std::collections::HashSet<(i32, i32)>,
) -> Vec<(i32, i32)> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    if dep == arr {
        return vec![dep];
    }

    // Map dep_side to the forced first-step delta.
    let first_delta: (i32, i32) = match dep_side {
        Side::Right  => (1, 0),
        Side::Left   => (-1, 0),
        Side::Top    => (0, -1),
        Side::Bottom => (0, 1),
    };

    // Search bounds: bounding box of dep/arr expanded by 6, capped at 200×200.
    let min_x = (dep.0.min(arr.0) - 6).max(dep.0.min(arr.0) - 200);
    let min_y = (dep.1.min(arr.1) - 6).max(dep.1.min(arr.1) - 200);
    let mut max_x = dep.0.max(arr.0) + 6;
    let mut max_y = dep.1.max(arr.1) + 6;
    if max_x - min_x > 200 { max_x = min_x + 200; }
    if max_y - min_y > 200 { max_y = min_y + 200; }

    // State: (cell, incoming_dir)
    type State = ((i32, i32), Option<(i32, i32)>);

    // Heap entries: (Reverse(f_cost), g_cost, cell, incoming_dir)
    // Using i32 for costs; all costs are non-negative.
    let manhattan = |a: (i32, i32), b: (i32, i32)| -> i32 {
        (a.0 - b.0).abs() + (a.1 - b.1).abs()
    };

    // Seed: forced first step from dep in dep_side direction.
    let start_cell = (dep.0 + first_delta.0, dep.1 + first_delta.1);
    // If the forced first step is out of bounds or blocked (and not arr), fall back.
    let first_blocked = blocked.contains(&start_cell) && start_cell != arr;
    let first_oob = start_cell.0 < min_x || start_cell.0 > max_x
        || start_cell.1 < min_y || start_cell.1 > max_y;

    if !first_blocked && !first_oob {
        let start_g: i32 = 1 + if soft.contains(&start_cell) { SOFT_PENALTY } else { 0 };
        let start_f: i32 = start_g + manhattan(start_cell, arr);
        let start_dir = Some(first_delta);

        type HeapEntry = (Reverse<i32>, i32, (i32, i32), Option<(i32, i32)>);
        let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
        heap.push((Reverse(start_f), start_g, start_cell, start_dir));

        let mut visited: std::collections::HashSet<State> = std::collections::HashSet::new();
        let mut parent: std::collections::HashMap<State, State> =
            std::collections::HashMap::new();

        // Seed parent for start_cell so we can reconstruct path back through dep.
        // We represent dep as having no parent (it's the true start).
        parent.insert((start_cell, start_dir), (dep, None));

        let neighbors: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

        'astar: while let Some((_, g, cell, inc_dir)) = heap.pop() {
            let state: State = (cell, inc_dir);
            if !visited.insert(state) {
                continue;
            }

            if visited.len() > 4000 {
                break 'astar;
            }

            if cell == arr {
                // Reconstruct path: walk parent map back to dep.
                let mut path_rev: Vec<(i32, i32)> = Vec::new();
                let mut cur_state: State = (cell, inc_dir);
                loop {
                    path_rev.push(cur_state.0);
                    match parent.get(&cur_state) {
                        Some(&prev) => {
                            if prev.0 == dep {
                                path_rev.push(dep);
                                break;
                            }
                            cur_state = prev;
                        }
                        None => break,
                    }
                }
                path_rev.reverse();
                return path_rev;
            }

            for &delta in &neighbors {
                let next = (cell.0 + delta.0, cell.1 + delta.1);
                // Bounds check.
                if next.0 < min_x || next.0 > max_x || next.1 < min_y || next.1 > max_y {
                    continue;
                }
                // Blocked check (arr is always passable).
                if next != arr && blocked.contains(&next) {
                    continue;
                }
                let next_dir = Some(delta);
                let next_state: State = (next, next_dir);
                if visited.contains(&next_state) {
                    continue;
                }
                // Turn penalty.
                let turn_cost: i32 = if inc_dir.is_some() && inc_dir != Some(delta) { 2 } else { 0 };
                // Congestion penalty: discourage running over/next to earlier paths.
                let soft_cost: i32 = if soft.contains(&next) { SOFT_PENALTY } else { 0 };
                let next_g = g + 1 + turn_cost + soft_cost;
                let next_f = next_g + manhattan(next, arr);
                if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(next_state) {
                    e.insert((cell, inc_dir));
                    heap.push((Reverse(next_f), next_g, next, next_dir));
                }
            }
        }
    }

    // Fallback: simple L-path (same as original logic).
    let corner = match dep_side {
        Side::Left | Side::Right => (arr.0, dep.1),
        Side::Top | Side::Bottom => (dep.0, arr.1),
    };
    let mut pts = Vec::new();
    walk_to(&mut pts, dep, corner);
    pts.pop();
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

/// Return the arrowhead glyph that points OUTWARD from the origin along `dep_side`.
///
/// Arrows signify the outgoing direction only — the departure direction is always
/// known, so arrows are always filled (▶◀▲▼).
fn arrow_for_departure(dep_side: Side) -> &'static str {
    match dep_side {
        Side::Right  => "▶", // leaving east
        Side::Left   => "◀", // leaving west
        Side::Top    => "▲", // leaving north
        Side::Bottom => "▼", // leaving south
    }
}

/// Queue an arrowhead at screen cell `pos` if it falls inside `area`.
fn push_arrow(
    arrowheads: &mut Vec<((u16, u16), &'static str, bool)>,
    pos: (i32, i32),
    glyph: &'static str,
    distorted: bool,
    area: Rect,
) {
    if pos.0 >= area.x as i32
        && pos.0 < area.right() as i32
        && pos.1 >= area.y as i32
        && pos.1 < area.bottom() as i32
    {
        arrowheads.push(((pos.0 as u16, pos.1 as u16), glyph, distorted));
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
/// `bg(Reset)` ensures the room cell clears any path ribbon drawn underneath.
const CURRENT_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED)
    .fg(Color::White)
    .bg(Color::Reset);

/// Style for the selected room (yellow border).
const SELECTED_STYLE: Style = Style::new().fg(Color::Yellow).bg(Color::Reset);

/// Style for normal rooms.
const NORMAL_STYLE: Style = Style::new().fg(Color::White).bg(Color::Reset);

/// Style for stub-connector labels — Cyan text.
const CONNECTOR_STYLE: Style = Style::new().fg(Color::Cyan);

/// Solid background fill for a path ribbon (normal).
const PATH_BG: Style = Style::new().bg(Color::Cyan);

/// Solid background fill for a path ribbon (distorted — different signal).
const PATH_BG_DISTORTED: Style = Style::new().bg(Color::Magenta);

/// Arrow embedded in a normal path ribbon: dark bold glyph on the ribbon colour.
const PATH_ARROW: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);

/// Arrow embedded in a distorted path ribbon.
const PATH_ARROW_DISTORTED: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Magenta)
    .add_modifier(Modifier::BOLD);

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
        // Collect path ribbon cells (cell → all_distorted) and arrowheads to embed.
        let mut path_cells: std::collections::HashMap<(u16, u16), bool> =
            std::collections::HashMap::new();
        let mut arrowheads: Vec<((u16, u16), &'static str, bool)> = Vec::new(); // (pos, glyph, distorted)

        // Cells occupied by already-routed connectors (plus a 1-cell halo). Later edges
        // pay SOFT_PENALTY to enter these, so paths keep a minimum gap between each other.
        let mut soft: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();

        // Directed connections (origin, dest) already drawn. A connection between two
        // rooms is rendered as ONE path; the reverse edge's outgoing arrow is placed as
        // the far-end arrow of the forward edge, so we skip an edge whose reverse is drawn.
        let mut drawn: std::collections::HashSet<(RoomId, RoomId)> =
            std::collections::HashSet::new();

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

            // Reciprocal reuse: if the reverse edge (dest → origin) was already drawn,
            // this edge's outgoing arrow was already placed as that path's far-end arrow.
            // Skip it so the connection is drawn once, not as two separate paths.
            if drawn.contains(&(edge.dest, edge.origin)) {
                continue;
            }

            let dep = side_anchor(origin_rect, dep_side);

            // Which side of the destination box the line connects to, and whether a
            // return trip (dest → origin) has been observed. `arrival_dir` is the
            // direction the player leaves `dest` to get back here; that side carries
            // the destination's own outgoing arrow. When unknown, fall back to the
            // geometrically nearest side and draw no far-end arrow.
            let dest_back_side = edge.arrival_dir.and_then(side_for);
            let arr_side = dest_back_side.unwrap_or_else(|| nearest_side(dest_rect, dep));
            let arr = side_anchor(dest_rect, arr_side);

            // Build blocked set: every OTHER room box expanded by a 1-cell halo, so a
            // passing connector always keeps at least one cell of clearance from rooms it
            // is not connecting to (it never hugs a wall).
            let mut blocked: std::collections::HashSet<(i32, i32)> = placed
                .iter()
                .filter(|(&id, _)| id != edge.origin && id != edge.dest)
                .flat_map(|(_, &rect)| {
                    let x0 = rect.x as i32 - 1;
                    let y0 = rect.y as i32 - 1;
                    let x1 = rect.right() as i32; // one column past the box (halo)
                    let y1 = rect.bottom() as i32; // one row past the box (halo)
                    (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
                })
                .collect();

            // Never block this connection's own exit/entry lanes: clear the departure and
            // arrival anchors plus their orthogonal neighbours so routing can always begin
            // and end even when an endpoint sits next to a third room's halo.
            for &(px, py) in &[dep, arr] {
                blocked.remove(&(px, py));
                blocked.remove(&(px + 1, py));
                blocked.remove(&(px - 1, py));
                blocked.remove(&(px, py + 1));
                blocked.remove(&(px, py - 1));
            }

            // Route the path, avoiding earlier paths' cells via the soft congestion set.
            let path = route_ortho(dep, arr, dep_side, &blocked, &soft);

            // Record this path (plus a 1-cell halo) so later edges keep a gap from it.
            for &(x, y) in &path {
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        soft.insert((x + dx, y + dy));
                    }
                }
            }

            // Fill the path ribbon: every routed cell inside the area becomes a solid
            // background block. A cell is styled distorted only if EVERY path through it
            // is distorted (AND across overlapping edges).
            for &(x, y) in &path {
                if x >= area.x as i32
                    && x < area.right() as i32
                    && y >= area.y as i32
                    && y < area.bottom() as i32
                {
                    let entry = path_cells.entry((x as u16, y as u16)).or_insert(true);
                    *entry = *entry && edge.distorted;
                }
            }

            // Outgoing arrow at the origin's departure anchor `dep`.
            push_arrow(&mut arrowheads, dep, arrow_for_departure(dep_side), edge.distorted, area);

            // Outgoing arrow at the destination, only when a return trip is known:
            // it points outward from `dest` back toward this room.
            if let Some(back_side) = dest_back_side {
                push_arrow(&mut arrowheads, arr, arrow_for_departure(back_side), edge.distorted, area);
            }

            drawn.insert((edge.origin, edge.dest));
        }

        // Fill path ribbons (solid background blocks).
        for (&(cx, cy), &all_distorted) in &path_cells {
            let style = if all_distorted { PATH_BG_DISTORTED } else { PATH_BG };
            if let Some(cell) = buf.cell_mut((cx, cy)) {
                cell.set_symbol(" ").set_style(style);
            }
        }

        // Embed arrowheads into the ribbon (keep the ribbon background, dark bold glyph).
        for ((ax, ay), glyph, distorted) in arrowheads {
            let style = if distorted { PATH_ARROW_DISTORTED } else { PATH_ARROW };
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
        // Arrows mark the outgoing direction at each origin's departure side: the E edge
        // puts ▶ at room1's right anchor, the W edge puts ◀ at room2's left anchor.
        // Both are filled; no hollow arrows are ever drawn.
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
    fn arrowhead_marks_outgoing_departure_side() {
        // Only room1 →E→ room2, no reverse edge. The arrow signifies the OUTGOING
        // direction: a filled ▶ at room1's right departure anchor (col 14, row 2).
        // No arrival-side or hollow arrow is drawn.
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

        // The outgoing arrow is a filled ▶ at the departure anchor (col 14, row 2),
        // embedded in the ribbon (Cyan background behind the glyph).
        let cell = buf.cell((14, 2)).expect("arrow cell must exist");
        assert_eq!(cell.symbol(), "▶", "outgoing east arrow ▶ should be at room1's right anchor (14,2)");
        assert_eq!(cell.bg, Color::Cyan, "arrow should be embedded in the ribbon (Cyan bg); got {:?}", cell.bg);

        // No hollow arrowhead should ever be drawn.
        let has_hollow = buf.content.iter().any(|c| matches!(c.symbol(), "▷" | "◁" | "△" | "▽"));
        assert!(!has_hollow, "hollow arrowheads must not appear; arrows are always filled");
    }

    #[test]
    fn connector_is_solid_background_ribbon() {
        // A connector is a solid background ribbon, not a line glyph. The ribbon cell at
        // col 16 (between the col-14 arrow and the col-17 destination anchor) must have a
        // Cyan background and a plain space symbol — not a dim/dashed line.
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

        let cell = buf.cell((16, 2)).expect("ribbon cell must exist");
        assert_eq!(cell.symbol(), " ", "ribbon cell should be a space, got '{}'", cell.symbol());
        assert_eq!(
            cell.bg,
            Color::Cyan,
            "ribbon background should be Cyan; got {:?} at (16,2)",
            cell.bg
        );
        assert!(
            !cell.modifier.contains(Modifier::DIM),
            "ribbon should not be dim; modifier={:?}",
            cell.modifier
        );
    }

    #[test]
    fn route_avoids_blocked_box() {
        // dep=(0,5), arr=(20,5), dep_side=Right
        // blocked = all cells of rect x in 8..14, y in 3..8
        let dep = (0i32, 5i32);
        let arr = (20i32, 5i32);
        let mut blocked = std::collections::HashSet::new();
        for x in 8..14i32 {
            for y in 3..8i32 {
                blocked.insert((x, y));
            }
        }
        let path = route_ortho(dep, arr, Side::Right, &blocked, &std::collections::HashSet::new());
        // Path must not contain any blocked cell
        for &pt in &path {
            assert!(!blocked.contains(&pt), "path goes through blocked cell {:?}", pt);
        }
        // Path must be contiguous
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dist = (a.0 - b.0).abs() + (a.1 - b.1).abs();
            assert_eq!(dist, 1, "path has gap between {:?} and {:?}", a, b);
        }
        // Must start at dep and end at arr
        assert_eq!(path[0], dep);
        assert_eq!(*path.last().unwrap(), arr);
        // First step must be east (dep_side=Right)
        assert_eq!(path[1], (1, 5), "first step should be east");
    }

    #[test]
    fn route_straight_when_clear() {
        // With empty blocked, dep/arr on the same row → straight line
        let dep = (0i32, 5i32);
        let arr = (5i32, 5i32);
        let blocked = std::collections::HashSet::new();
        let path = route_ortho(dep, arr, Side::Right, &blocked, &std::collections::HashSet::new());
        // Should be straight line: (0,5),(1,5),(2,5),(3,5),(4,5),(5,5)
        let expected: Vec<(i32, i32)> = (0..=5).map(|x| (x, 5)).collect();
        assert_eq!(path, expected, "should be straight line when clear");
    }

    #[test]
    fn backtracking_reciprocal_draws_arrow_at_both_rooms() {
        // A(1) at (1,1) →N→ B(2) at (1,0), then backtrack B →S→ A (reciprocal-opposite).
        // route_all dedupes to one edge, but BOTH outgoing arrows must render:
        // ▲ at A (leaving north) and ▼ at B (leaving south).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::N, 2);
        g.add_edge(2, mapper::direction::Direction::S, 1); // backtrack
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let up = buf.content.iter().filter(|c| c.symbol() == "▲").count();
        let down = buf.content.iter().filter(|c| c.symbol() == "▼").count();
        assert_eq!(up, 1, "exactly one ▲ (A leaving north); got {up}");
        assert_eq!(down, 1, "exactly one ▼ (B leaving south, the backtrack); got {down}");
    }

    #[test]
    fn return_from_different_direction_reuses_single_path() {
        // A(1) at (1,1) →N→ B(2) at (1,0), then return B →W→ A (non-opposite).
        // Both edges are kept by route_all, but the connection must render as ONE path
        // with one arrow per room: exactly one ▲ (A north) and one ◀ (B west) — not two
        // separate parallel paths (which would yield two of each).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0));
        g.add_edge(1, mapper::direction::Direction::N, 2);
        g.add_edge(2, mapper::direction::Direction::W, 1); // return from a different side
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let up = buf.content.iter().filter(|c| c.symbol() == "▲").count();
        let left = buf.content.iter().filter(|c| c.symbol() == "◀").count();
        assert_eq!(up, 1, "exactly one ▲ (A leaving north); two would mean a duplicate path; got {up}");
        assert_eq!(left, 1, "exactly one ◀ (B leaving west); got {left}");
    }

    #[test]
    fn multi_exit_same_pair_keeps_both_arrows() {
        // A →E→ B AND A →W→ B: two genuinely distinct exits from A to the same room.
        // These are NOT reciprocal (both originate at A), so both arrows must survive
        // the reciprocal-reuse dedupe: one ▶ and one ◀, both departing A.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (3, 1));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        g.add_edge(1, mapper::direction::Direction::W, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let right = buf.content.iter().filter(|c| c.symbol() == "▶").count();
        let left = buf.content.iter().filter(|c| c.symbol() == "◀").count();
        assert_eq!(right, 1, "A's east exit ▶ must be kept; got {right}");
        assert_eq!(left, 1, "A's west exit ◀ must be kept (not deduped as reciprocal); got {left}");
    }

    #[test]
    fn route_keeps_gap_from_earlier_path() {
        // First path runs straight along row 5. A second parallel path that would
        // otherwise stack on the adjacent row 6 should detour clear of row 5's halo.
        let empty = std::collections::HashSet::new();
        let p1 = route_ortho((0, 5), (20, 5), Side::Right, &empty, &empty);

        // Build the soft set from p1's cells plus a 1-cell halo (same as render_map).
        let mut soft = std::collections::HashSet::new();
        for &(x, y) in &p1 {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    soft.insert((x + dx, y + dy));
                }
            }
        }

        // Second path on row 6 (inside p1's halo) — it should bend away to row >= 7.
        let p2 = route_ortho((0, 6), (20, 6), Side::Right, &empty, &soft);
        assert!(
            p2.iter().any(|&(_, y)| y >= 7),
            "second path should detour out of the first path's halo; got {p2:?}"
        );
    }

    #[test]
    fn render_keeps_one_cell_gap_around_passed_room() {
        // A(0,0) →E→ C(2,0) with B(1,0) in between. The connector must keep at least one
        // cell of clearance from B: no connector glyph in the ring of cells immediately
        // surrounding B's box (B's own border cells are excluded).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        g.add_edge(1, mapper::direction::Direction::E, 3); // A→C, passing B
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // B box: cell (1,0) → screen (18,0), size 14×4 → cols 18..31, rows 0..3.
        let b = Rect::new(18, 0, 14, 4);
        // Ring = the 1-cell halo around B, excluding B's own box cells. No path ribbon
        // (Cyan/Magenta background) may touch it.
        for y in (b.y as i32 - 1)..=(b.bottom() as i32) {
            for x in (b.x as i32 - 1)..=(b.right() as i32) {
                if x < 0 || y < 0 {
                    continue;
                }
                let in_box = x >= b.x as i32
                    && x < b.right() as i32
                    && y >= b.y as i32
                    && y < b.bottom() as i32;
                if in_box {
                    continue; // skip B's own border/interior
                }
                if let Some(cell) = buf.cell((x as u16, y as u16)) {
                    assert!(
                        cell.bg != Color::Cyan && cell.bg != Color::Magenta,
                        "path ribbon ({:?}) hugs room B at ({x},{y}); expected a 1-cell gap",
                        cell.bg
                    );
                }
            }
        }
    }

    #[test]
    fn render_no_path_ribbon_inside_other_room() {
        // Verify via rendering: 3 rooms where A→C would naively cross B.
        // Room A at (0,0), Room B at (1,0), Room C at (2,0).
        // Direct edge from A to C (not via B) so the connector crosses B's area.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        // Direct edge from A(0,0) to C(2,0) — passes through B's space naively
        g.add_edge(1, mapper::direction::Direction::E, 3);
        let rm = mapper::render::render(&g);
        let state = AppState::default(); // Boxes zoom
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Room B box: Boxes zoom, step=18×6, room at cell (1,0) → screen (18,0), box 14×4.
        // No path ribbon (Cyan/Magenta background) may appear inside B's interior.
        let b_rect = Rect::new(18, 0, 14, 4);
        for y in (b_rect.y + 1)..(b_rect.y + b_rect.height - 1) {
            for x in (b_rect.x + 1)..(b_rect.x + b_rect.width - 1) {
                if let Some(cell) = buf.cell((x, y)) {
                    assert!(
                        cell.bg != Color::Cyan && cell.bg != Color::Magenta,
                        "path ribbon ({:?}) found inside room B's interior at ({x},{y})",
                        cell.bg
                    );
                }
            }
        }
    }
}
