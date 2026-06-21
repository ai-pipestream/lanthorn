//! Map projection and ratatui rendering.
//!
//! # Coordinate system
//!
//! Logical room cells (col, row) are placed on a grid.  Each zoom level defines a "step"
//! (cell stride in terminal columns/rows):
//!
//! | Zoom     | step_w | step_h |
//! |----------|--------|--------|
//! | Boxes    |  29    |  17    |
//! | Compact  |  12    |   5    |
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
/// | Boxes   | 29×17 | 21×11 | 8 cols / 6 rows         |
/// | Compact | 12×5  | 8×3   | 4 cols / 2 rows         |
/// | Overview| 2×2   | 1×1   | — (single glyph)        |
///
/// The 21×11 box (both odd) is ~2:1 width:height so it reads as square given the
/// terminal's ~1:2 cell aspect, and odd dims centre the side anchors on the box.
fn zoom_box_size(zoom: Zoom) -> (u16, u16) {
    match zoom {
        Zoom::Boxes => (21, 11),
        Zoom::Compact => (8, 3),
        Zoom::Overview => (1, 1),
    }
}

// ── Virtual map space ─────────────────────────────────────────────────────────
//
// The whole map is built in a scroll-independent "virtual" coordinate space where
// a room at logical cell (c, r) sits at pixel (c * step_w, r * step_h). Rooms and
// connectors are placed and routed here ONCE, regardless of scroll, so the routes
// never change as the view pans. Scrolling is then a pure translate-and-clip blit:
// screen = virtual + (area.origin - scroll * step).

/// An integer rectangle in virtual map space (coordinates may be negative).
#[derive(Clone, Copy)]
struct VRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl VRect {
    fn right(&self) -> i32 {
        self.x + self.w
    }
    fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// Virtual top-left pixel of a room cell: `cell * step` (no scroll, no area offset).
fn cell_to_virtual(cell: (i32, i32), zoom: Zoom) -> (i32, i32) {
    let (sw, sh) = zoom_steps(zoom);
    (cell.0 * sw, cell.1 * sh)
}

// ── Box-geometry routing helpers ──────────────────────────────────────────────

/// Return the gutter cell just outside `rect` on the given side, at the side's midpoint.
///
/// - Right: (rect.right(), rect.y + rect.h/2)
/// - Left:  (rect.x - 1,   rect.y + rect.h/2)
/// - Top:   (rect.x + rect.w/2, rect.y - 1)
/// - Bottom:(rect.x + rect.w/2, rect.bottom())
fn side_anchor(rect: VRect, side: Side) -> (i32, i32) {
    match side {
        Side::Right  => (rect.right(),        rect.y + rect.h / 2),
        Side::Left   => (rect.x - 1,          rect.y + rect.h / 2),
        Side::Top    => (rect.x + rect.w / 2, rect.y - 1),
        Side::Bottom => (rect.x + rect.w / 2, rect.bottom()),
    }
}

/// Orientation bits for a cell occupied by an earlier-routed connector.
/// A cell may carry both (a corner of one path, or a perpendicular crossing of two).
const HORIZ: u8 = 1;
const VERT: u8 = 2;

/// Build an obstacle-aware orthogonal path from `dep` to `arr` whose first step leaves in
/// `dep_side`'s direction.
///
/// Uses A* on the integer screen grid. `blocked` contains all screen cells that are
/// interior to other room boxes plus a 1-cell halo (excluding this edge's own rooms), so
/// a routed path keeps clearance from rooms. `paths` maps each cell occupied by an
/// earlier-routed connector to its orientation bits (`HORIZ`/`VERT`). The router enforces,
/// as HARD constraints against earlier paths:
///   - no overlap: never run along a cell already carrying the same orientation;
///   - no running alongside: never sit orthogonally adjacent-and-parallel to an existing
///     path of the same orientation (a 1-cell gap is required);
///   - crossings are perpendicular straight-throughs only: an existing path cell may be
///     entered only across its orientation, and the new path may not turn on it.
///
/// Returns `Some(path)` for a clean Tier-1 route honouring all the above constraints, or
/// `None` if no clean route exists — there is no overlap-permitting fallback, so a routed
/// connector never overlaps. The caller decides how to render an unrouted edge.
fn route_ortho(
    dep: (i32, i32),
    arr: (i32, i32),
    dep_side: Side,
    blocked: &std::collections::HashSet<(i32, i32)>,
    paths: &std::collections::HashMap<(i32, i32), u8>,
) -> Option<Vec<(i32, i32)>> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    if dep == arr {
        return Some(vec![dep]);
    }

    // Map dep_side to the forced first-step delta.
    let first_delta: (i32, i32) = match dep_side {
        Side::Right  => (1, 0),
        Side::Left   => (-1, 0),
        Side::Top    => (0, -1),
        Side::Bottom => (0, 1),
    };

    // Search bounds: bounding box of dep/arr expanded by 24 so a path can detour well
    // around intervening rooms, capped at 400×400.
    let min_x = dep.0.min(arr.0) - 24;
    let min_y = dep.1.min(arr.1) - 24;
    let mut max_x = dep.0.max(arr.0) + 24;
    let mut max_y = dep.1.max(arr.1) + 24;
    if max_x - min_x > 400 { max_x = min_x + 400; }
    if max_y - min_y > 400 { max_y = min_y + 400; }

    // State: (cell, incoming_dir)
    type State = ((i32, i32), Option<(i32, i32)>);

    // Heap entries: (Reverse(f_cost), g_cost, cell, incoming_dir)
    // Using i32 for costs; all costs are non-negative.
    let manhattan = |a: (i32, i32), b: (i32, i32)| -> i32 {
        (a.0 - b.0).abs() + (a.1 - b.1).abs()
    };

    type HeapEntry = (Reverse<i32>, i32, (i32, i32), Option<(i32, i32)>);
    let neighbors: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

    // A* that always honours room `blocked` clearance, plus the path-vs-path rules carried
    // in `pr`. Returns None when no route is found, so the caller can degrade.
    let astar = |pr: &std::collections::HashMap<(i32, i32), u8>| -> Option<Vec<(i32, i32)>> {
        let start_cell = (dep.0 + first_delta.0, dep.1 + first_delta.1);
        if (blocked.contains(&start_cell) && start_cell != arr)
            || start_cell.0 < min_x
            || start_cell.0 > max_x
            || start_cell.1 < min_y
            || start_cell.1 > max_y
        {
            return None;
        }
        let start_dir = Some(first_delta);
        let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
        heap.push((Reverse(1 + manhattan(start_cell, arr)), 1, start_cell, start_dir));
        let mut visited: std::collections::HashSet<State> = std::collections::HashSet::new();
        let mut parent: std::collections::HashMap<State, State> = std::collections::HashMap::new();
        parent.insert((start_cell, start_dir), (dep, None));

        while let Some((_, g, cell, inc_dir)) = heap.pop() {
            let state: State = (cell, inc_dir);
            if !visited.insert(state) {
                continue;
            }
            if visited.len() > 20000 {
                break;
            }
            if cell == arr {
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
                return Some(path_rev);
            }
            for &delta in &neighbors {
                let next = (cell.0 + delta.0, cell.1 + delta.1);
                if next.0 < min_x || next.0 > max_x || next.1 < min_y || next.1 > max_y {
                    continue;
                }
                if next != arr && blocked.contains(&next) {
                    continue;
                }
                // Hard path-interaction rules (skipped for the final hop into `arr`).
                if next != arr {
                    let our_bit = if delta.1 == 0 { HORIZ } else { VERT };
                    // (a) no overlap with a same-orientation path.
                    if pr.get(&next).copied().unwrap_or(0) & our_bit != 0 {
                        continue;
                    }
                    // (b) no turning onto a path cell (crossings are straight-through only).
                    let turning = inc_dir.is_some() && inc_dir != Some(delta);
                    if turning && pr.get(&cell).copied().unwrap_or(0) != 0 {
                        continue;
                    }
                    // (c) no running alongside a same-orientation path (keep a 1-cell gap).
                    let alongside = if our_bit == HORIZ {
                        pr.get(&(next.0, next.1 - 1)).copied().unwrap_or(0) & HORIZ != 0
                            || pr.get(&(next.0, next.1 + 1)).copied().unwrap_or(0) & HORIZ != 0
                    } else {
                        pr.get(&(next.0 - 1, next.1)).copied().unwrap_or(0) & VERT != 0
                            || pr.get(&(next.0 + 1, next.1)).copied().unwrap_or(0) & VERT != 0
                    };
                    if alongside {
                        continue;
                    }
                }
                let next_dir = Some(delta);
                let next_state: State = (next, next_dir);
                if visited.contains(&next_state) {
                    continue;
                }
                let turn_cost: i32 = if inc_dir.is_some() && inc_dir != Some(delta) { 2 } else { 0 };
                let next_g = g + 1 + turn_cost;
                let next_f = next_g + manhattan(next, arr);
                if let std::collections::hash_map::Entry::Vacant(e) = parent.entry(next_state) {
                    e.insert((cell, inc_dir));
                    heap.push((Reverse(next_f), next_g, next, next_dir));
                }
            }
        }
        None
    };

    // Clean route only: full room-clearance + path-vs-path rules. If A* cannot find
    // one, the edge has no clean channel — return None so the renderer can flag it as
    // unrouted rather than draw an overlapping fallback.
    astar(paths)
}

/// Pick the side of `dest_rect` to connect an undiscovered (non-reciprocal) arrival to:
/// the geometrically nearest side to `dep` that is NOT already `occupied` by one of the
/// destination's departures or another arrival. Falls back to the nearest side if every
/// side is occupied. This keeps an arriving line off the centre anchor that a departure
/// arrow (or another arrival) already uses, so they don't collide.
fn nearest_free_side(dest_rect: VRect, dep: (i32, i32), occupied: &[Side]) -> Side {
    let mut sides = [Side::Top, Side::Bottom, Side::Left, Side::Right];
    sides.sort_by_key(|&s| {
        let a = side_anchor(dest_rect, s);
        let dx = a.0 - dep.0;
        let dy = a.1 - dep.1;
        dx * dx + dy * dy
    });
    sides
        .iter()
        .find(|s| !occupied.contains(s))
        .copied()
        .unwrap_or(sides[0])
}

/// Anchor point on `side` of `rect` for a NON-reciprocal arrival, offset from the side's
/// centre by `index` slots. The centre of every side is reserved for departures (whose
/// outgoing arrow sits there), so an arriving line that isn't a verified reciprocal must
/// land beside the centre — otherwise a centred arrival is indistinguishable from a
/// departure and you can't tell which way an edge actually goes. `index` (0-based count of
/// arrivals already on this side) walks outward in alternating +/- slots so multiple
/// arrivals on one side don't collide.
fn arrival_anchor(rect: VRect, side: Side, index: i32) -> (i32, i32) {
    // Quarter-of-side step keeps the offset clearly off-centre yet on the box edge.
    let step = match side {
        Side::Top | Side::Bottom => (rect.w / 4).max(1),
        Side::Left | Side::Right => (rect.h / 4).max(1),
    };
    let k = index / 2 + 1; // 1,1,2,2,3,3,…
    let sign = if index % 2 == 0 { 1 } else { -1 };
    let off = sign * k * step;
    match side {
        Side::Right => {
            let y = (rect.y + rect.h / 2 + off).clamp(rect.y, rect.bottom() - 1);
            (rect.right(), y)
        }
        Side::Left => {
            let y = (rect.y + rect.h / 2 + off).clamp(rect.y, rect.bottom() - 1);
            (rect.x - 1, y)
        }
        Side::Top => {
            let x = (rect.x + rect.w / 2 + off).clamp(rect.x, rect.right() - 1);
            (x, rect.y - 1)
        }
        Side::Bottom => {
            let x = (rect.x + rect.w / 2 + off).clamp(rect.x, rect.right() - 1);
            (x, rect.bottom())
        }
    }
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

/// A simple two-segment L from `dep` to `arr` whose first step leaves on `dep_side`.
/// Used only to give an UNROUTED edge a visible (flagged) shape; it may overlap and is
/// never recorded in the path-occupancy map.
fn unrouted_l(dep: (i32, i32), arr: (i32, i32), dep_side: Side) -> Vec<(i32, i32)> {
    let corner = match dep_side {
        Side::Left | Side::Right => (arr.0, dep.1), // horizontal-first
        Side::Top | Side::Bottom => (dep.0, arr.1), // vertical-first
    };
    let mut pts = Vec::new();
    walk_to(&mut pts, dep, corner);
    pts.pop();
    walk_to(&mut pts, corner, arr);
    pts
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

/// True if screen cell `(sx, sy)` lies inside `area`.
fn in_area(sx: i32, sy: i32, area: Rect) -> bool {
    sx >= area.x as i32 && sx < area.right() as i32 && sy >= area.y as i32 && sy < area.bottom() as i32
}

/// Style for a room given the current selection/current state.
fn room_style(room: &RenderRoom, state: &AppState) -> Style {
    if room.is_current {
        CURRENT_STYLE
    } else if state.selected_room == Some(room.id) {
        SELECTED_STYLE
    } else {
        NORMAL_STYLE
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

/// Ribbon background for an edge with no clean route — visibly distinct (dimmed) so a
/// rare routing failure is obvious rather than mistaken for a normal connector.
const PATH_BG_UNROUTED: Style = Style::new().bg(Color::DarkGray);

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
///
/// The whole map is built in scroll-independent virtual space (see [`VRect`]) and
/// blitted to the screen with a single translation, so panning never re-routes
/// connectors — the routes are identical at every scroll offset.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;
    let (step_w, step_h) = zoom_steps(zoom);

    // Virtual → screen translation: screen = virtual + offset.
    let off_x = area.x as i32 - scroll.0 * step_w;
    let off_y = area.y as i32 - scroll.1 * step_h;

    // Overview zoom: one glyph per room, no connectors.
    if matches!(zoom, crate::state::Zoom::Overview) {
        for room in &rm.rooms {
            let (vx, vy) = cell_to_virtual(room.cell, zoom);
            put_char(buf, vx + off_x, vy + off_y, '■', room_style(room, state), area);
        }
        return;
    }

    // ── 1. Place ALL rooms in virtual space (independent of scroll/area) ──────
    let (bw, bh) = zoom_box_size(zoom);
    let mut placed: std::collections::HashMap<RoomId, VRect> =
        std::collections::HashMap::new();
    for room in &rm.rooms {
        let (vx, vy) = cell_to_virtual(room.cell, zoom);
        placed.insert(room.id, VRect { x: vx, y: vy, w: bw as i32, h: bh as i32 });
    }

    // ── 2. Route ALL connectors in virtual space (computed once, scroll-free) ─
    let mut path_cells: std::collections::HashMap<(i32, i32), bool> =
        std::collections::HashMap::new();
    // Cells belonging to UNROUTED edges (no clean channel). Rendered distinctly and
    // deliberately NOT added to `paths`, so they never constrain later clean routes.
    let mut unrouted_cells: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();
    // Cells occupied by already-routed connectors, keyed to their orientation bits, so
    // later edges can keep clearance and cross only at perpendicular straight-throughs.
    let mut paths: std::collections::HashMap<(i32, i32), u8> = std::collections::HashMap::new();
    let mut drawn: std::collections::HashSet<(RoomId, RoomId)> =
        std::collections::HashSet::new();

    // The sides each room uses for its OUTGOING compass edges (departure arrows sit there).
    // A non-reciprocal arrival avoids these sides so it doesn't land on a departure anchor.
    let mut dep_sides: std::collections::HashMap<RoomId, Vec<Side>> =
        std::collections::HashMap::new();
    for e in &rm.edges {
        if e.is_stub {
            continue;
        }
        if let Some(s) = side_for(e.dir) {
            dep_sides.entry(e.origin).or_default().push(s);
        }
    }
    // Sides already claimed by arrivals into each room (so two arrivals don't collide).
    let mut used_arr: std::collections::HashMap<RoomId, Vec<Side>> =
        std::collections::HashMap::new();

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
        let arr = match dest_back_side {
            // Confirmed reciprocal: both ends are departures, so arrive at the matched
            // side's centre (one shared, centred path with an arrow at each end).
            Some(s) => side_anchor(dest_rect, s),
            // Non-reciprocal: connect to the nearest side of dest not already used by a
            // departure or a prior arrival, off-centre so the side's centre stays reserved
            // for departures (and the arrival reads as an arrival, not an exit).
            None => {
                let mut occupied = dep_sides.get(&edge.dest).cloned().unwrap_or_default();
                if let Some(u) = used_arr.get(&edge.dest) {
                    occupied.extend(u.iter().copied());
                }
                let s = nearest_free_side(dest_rect, dep, &occupied);
                let idx = used_arr
                    .get(&edge.dest)
                    .map(|u| u.iter().filter(|&&x| x == s).count() as i32)
                    .unwrap_or(0);
                used_arr.entry(edge.dest).or_default().push(s);
                arrival_anchor(dest_rect, s, idx)
            }
        };

        // Build blocked set: EVERY room box (including this edge's own origin and dest)
        // expanded by a 1-cell halo, so a connector keeps clearance from rooms and can
        // never pass through one — including the destination, which it must approach from
        // outside rather than cutting across to reach a far-side anchor.
        let mut blocked: std::collections::HashSet<(i32, i32)> = placed
            .values()
            .flat_map(|&rect| {
                let x0 = rect.x - 1;
                let y0 = rect.y - 1;
                let x1 = rect.right(); // one column past the box (halo)
                let y1 = rect.bottom(); // one row past the box (halo)
                (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
            })
            .collect();

        // Never block this connection's own exit/entry lanes: clear the departure and
        // arrival anchors plus their orthogonal neighbours so routing can always begin
        // and end even though the origin/dest boxes are otherwise blocked.
        for &(px, py) in &[dep, arr] {
            blocked.remove(&(px, py));
            blocked.remove(&(px + 1, py));
            blocked.remove(&(px - 1, py));
            blocked.remove(&(px, py + 1));
            blocked.remove(&(px, py - 1));
        }

        // Route the path with hard clearance + perpendicular-crossing-only rules vs
        // earlier paths.
        match route_ortho(dep, arr, dep_side, &blocked, &paths) {
            Some(path) => {
                // Record this clean path's cells with orientation, so later edges keep
                // clearance and may only cross it perpendicularly.
                for w in path.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let bit = if a.1 == b.1 { HORIZ } else { VERT };
                    *paths.entry(a).or_insert(0) |= bit;
                    *paths.entry(b).or_insert(0) |= bit;
                }
                for &(x, y) in &path {
                    let entry = path_cells.entry((x, y)).or_insert(true);
                    *entry = *entry && edge.distorted;
                }
            }
            None => {
                // No clean channel: draw a distinct, flagged L (may overlap); do NOT add
                // it to `paths` occupancy.
                for &(x, y) in &unrouted_l(dep, arr, dep_side) {
                    unrouted_cells.insert((x, y));
                }
            }
        }

        // Outgoing arrow at the origin's departure anchor `dep`.
        arrowheads.push((dep, arrow_for_departure(dep_side), edge.distorted));

        // Outgoing arrow at the destination, only when a return trip is known:
        // it points outward from `dest` back toward this room.
        if let Some(back_side) = dest_back_side {
            arrowheads.push((arr, arrow_for_departure(back_side), edge.distorted));
        }

        drawn.insert((edge.origin, edge.dest));
    }

    // ── 3. Blit ribbons (translate virtual → screen, clip to area) ───────────
    for (&(vx, vy), &all_distorted) in &path_cells {
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            let style = if all_distorted { PATH_BG_DISTORTED } else { PATH_BG };
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }

    // Blit unrouted ribbons (translate virtual → screen, clip). A clean path always
    // wins a shared cell, so skip cells already in path_cells.
    for &(vx, vy) in &unrouted_cells {
        if path_cells.contains_key(&(vx, vy)) {
            continue;
        }
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(" ").set_style(PATH_BG_UNROUTED);
            }
        }
    }

    // Blit arrowheads embedded in the ribbon.
    for ((vx, vy), glyph, distorted) in arrowheads {
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            let style = if distorted { PATH_ARROW_DISTORTED } else { PATH_ARROW };
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }
    }

    // Stub edges (translate + clip).
    for edge in &rm.edges {
        if edge.is_stub {
            draw_stub(edge, &placed, off_x, off_y, area, buf);
        }
    }

    // ── 4. Draw rooms on top (translate + clip) ──────────────────────────────
    for room in &rm.rooms {
        let (vx, vy) = cell_to_virtual(room.cell, zoom);
        draw_room(room, state, zoom, vx + off_x, vy + off_y, area, buf);
    }
}

/// Draw a stub connector label in the top-right gutter cell outside the origin box.
/// `off_x`/`off_y` translate the origin's virtual rect into screen space.
fn draw_stub(
    edge: &RoutedEdge,
    placed: &std::collections::HashMap<RoomId, VRect>,
    off_x: i32,
    off_y: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    let Some(&origin_rect) = placed.get(&edge.origin) else {
        return;
    };
    let label = edge.label.as_deref().unwrap_or("?");
    // Top-right gutter: just right of the box, at the top row.
    let lx = origin_rect.right() + off_x;
    let ly = origin_rect.y + off_y;
    put_str(buf, lx, ly, label, CONNECTOR_STYLE, area);
}

// ── Room drawing ──────────────────────────────────────────────────────────────

/// Draw a room at screen top-left `(sx, sy)` (already translated from virtual space;
/// may be partially or fully off-area — drawing is clipped per cell).
fn draw_room(
    room: &RenderRoom,
    state: &AppState,
    zoom: Zoom,
    sx: i32,
    sy: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    let base_style = room_style(room, state);

    match zoom {
        Zoom::Overview => {
            put_char(buf, sx, sy, '■', base_style, area);
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
    sx: i32,
    sy: i32,
    style: Style,
    area: Rect,
    buf: &mut Buffer,
) {
    let (bw, bh) = zoom_box_size(Zoom::Compact); // (8, 3)
    let (bw, bh) = (bw as i32, bh as i32);
    let is_current = style.add_modifier.contains(Modifier::REVERSED);

    let (tl, tr, bl, br, h, v) = if is_current {
        ('┏', '┓', '┗', '┛', '━', '┃')
    } else {
        ('╭', '╮', '╰', '╯', '─', '│')
    };

    // Top border
    put_char(buf, sx, sy, tl, style, area);
    for dx in 1..bw - 1 {
        put_char(buf, sx + dx, sy, h, style, area);
    }
    put_char(buf, sx + bw - 1, sy, tr, style, area);

    // Middle row: sides + label (inner width = bw - 2 = 6)
    let label_width = (bw - 2) as usize; // 6
    let label: String = room.label.chars().take(label_width).collect();
    put_char(buf, sx, sy + 1, v, style, area);
    put_str(buf, sx + 1, sy + 1, &label, style, area);
    put_char(buf, sx + bw - 1, sy + 1, v, style, area);

    // Bottom border
    put_char(buf, sx, sy + bh - 1, bl, style, area);
    for dx in 1..bw - 1 {
        put_char(buf, sx + dx, sy + bh - 1, h, style, area);
    }
    put_char(buf, sx + bw - 1, sy + bh - 1, br, style, area);
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
    sx: i32,
    sy: i32,
    style: Style,
    area: Rect,
    buf: &mut Buffer,
) {
    let (w, h) = zoom_box_size(Zoom::Boxes); // (14, 4)
    let (w, h) = (w as i32, h as i32);
    let is_current = style.add_modifier.contains(Modifier::REVERSED);

    let (tl, tr, bl, br, horiz, vert) = if is_current {
        ('┏', '┓', '┗', '┛', '━', '┃')
    } else {
        ('╭', '╮', '╰', '╯', '─', '│')
    };

    // Top border
    put_char(buf, sx, sy, tl, style, area);
    for dx in 1..w - 1 {
        put_char(buf, sx + dx, sy, horiz, style, area);
    }
    put_char(buf, sx + w - 1, sy, tr, style, area);

    // Inner rows (h=4 → rows 1 and 2 are interior)
    for dy in 1..h - 1 {
        put_char(buf, sx, sy + dy, vert, style, area);
        // Fill interior with spaces (for background/style)
        for dx in 1..w - 1 {
            put_char(buf, sx + dx, sy + dy, ' ', style, area);
        }
        put_char(buf, sx + w - 1, sy + dy, vert, style, area);
    }

    // Label on row 1 (first inner row), up to w-2 chars.
    let label_width = (w - 2) as usize;
    let label: String = room.label.chars().take(label_width).collect();
    put_str(buf, sx + 1, sy + 1, &label, style, area);

    // Unique room id (object number) on row 2, so rooms can be referenced. Only when
    // the box is tall enough that row 2 is interior (Boxes zoom).
    if h > 3 {
        let id_str: String = format!("#{}", room.id).chars().take(label_width).collect();
        put_str(buf, sx + 1, sy + 2, &id_str, style, area);
    }

    // Notes marker ● in top-right inner corner (row 1, col w-2).
    if room.has_notes {
        put_char(buf, sx + w - 2, sy + 1, '●', style, area);
    }

    // Bottom border
    put_char(buf, sx, sy + h - 1, bl, style, area);
    for dx in 1..w - 1 {
        put_char(buf, sx + dx, sy + h - 1, horiz, style, area);
    }
    put_char(buf, sx + w - 1, sy + h - 1, br, style, area);
}

// ── Clipped drawing helpers ───────────────────────────────────────────────────

use super::{put_char, put_str};

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
        let area = Rect::new(0, 0, 80, 80);

        // Cell (0,0) with no scroll at Boxes → screen (0,0), inside area.
        let on = cell_to_screen((0, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(on, Some((0, 0)));

        // Cell (1,0) at Boxes → x = 0 + (1-0)*29 = 29
        let right = cell_to_screen((1, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(right, Some((29, 0)));

        // Cell (0,1) at Boxes → y = 0 + (1-0)*17 = 17
        let down = cell_to_screen((0, 1), Zoom::Boxes, (0, 0), area);
        assert_eq!(down, Some((0, 17)));

        // Far off-area cell.
        let off = cell_to_screen((1000, 1000), Zoom::Boxes, (0, 0), area);
        assert!(off.is_none());

        // Scroll pushes cell off-screen: scroll=(1,0) so cell (0,0) → x = 0+(0-1)*29 = -29 → None.
        let scrolled_off = cell_to_screen((0, 0), Zoom::Boxes, (1, 0), area);
        assert!(scrolled_off.is_none());

        // Compact zoom: step 12×5 → cell (1,1) → (12, 5)
        let compact = cell_to_screen((1, 1), Zoom::Compact, (0, 0), area);
        assert_eq!(compact, Some((12, 5)));

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

    #[test]
    fn room_box_shows_id() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(7, "Hall".into());
        g.set_pos(7, (0, 0));
        let rm = render(&g);
        let state = AppState::default(); // Boxes zoom
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The unique id "#7" is drawn on row 2 (under the label) at cols 1..3.
        let row2: String = (1u16..=3)
            .map(|x| buf.cell((x, 2)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(row2.contains("#7"), "row 2 should show the room id '#7'; got '{row2}'");
    }

    // connector_has_corner_glyph: removed — called build_connector_mask which is gone;
    // superseded by new tests in Task 4.

    // connector_has_arrowhead_at_dest: removed — arrowhead rendering is stubbed out in Task 1;
    // superseded by new tests in Task 4.

    // connector_is_contiguous_no_gaps: segment_screen_points unit portion removed (function gone);
    // full-render connector assertions superseded by new tests in Task 4.

    #[test]
    fn connector_departs_origin_correct_side() {
        // room1 at (0,0) →E→ room2 at (1,0). Boxes zoom (box 21×21, step 29×29).
        // room1 box: VRect{x:0,y:0,w:21,h:21}. Right-side anchor: col=21, row=10.
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

        // The departure anchor for room1→E is Right side: col=21, row=10.
        // It must NOT be a space and NOT have a room box glyph from room1 (room1 cols 0..20).
        let dep_col = 21u16;
        let dep_row = 5u16;
        let sym = buf.cell((dep_col, dep_row)).map(|c| c.symbol()).unwrap_or(" ");
        assert_ne!(sym, " ", "departure gutter cell ({dep_col},{dep_row}) should have a connector glyph");
        assert!(
            dep_col >= 21, // outside room1 box (cols 0..20)
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

        // The outgoing arrow is a filled ▶ at the departure anchor (col 21, row 10),
        // embedded in the ribbon (Cyan background behind the glyph).
        let cell = buf.cell((21, 5)).expect("arrow cell must exist");
        assert_eq!(cell.symbol(), "▶", "outgoing east arrow ▶ should be at room1's right anchor (21,5)");
        assert_eq!(cell.bg, Color::Cyan, "arrow should be embedded in the ribbon (Cyan bg); got {:?}", cell.bg);

        // No hollow arrowhead should ever be drawn.
        let has_hollow = buf.content.iter().any(|c| matches!(c.symbol(), "▷" | "◁" | "△" | "▽"));
        assert!(!has_hollow, "hollow arrowheads must not appear; arrows are always filled");
    }

    #[test]
    fn connector_is_solid_background_ribbon() {
        // A connector is a solid background ribbon, not a line glyph. Room1 right anchor
        // is (21,10), room2 left anchor (28,10); the straight ribbon runs row 10. The cell
        // at (24,10) must have a Cyan background and a plain space symbol — not a line.
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

        let cell = buf.cell((24, 5)).expect("ribbon cell must exist");
        assert_eq!(cell.symbol(), " ", "ribbon cell should be a space, got '{}'", cell.symbol());
        assert_eq!(
            cell.bg,
            Color::Cyan,
            "ribbon background should be Cyan; got {:?} at (24,5)",
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
        let path = route_ortho(dep, arr, Side::Right, &blocked, &std::collections::HashMap::new()).expect("clean Tier-1 route");
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
        let path = route_ortho(dep, arr, Side::Right, &blocked, &std::collections::HashMap::new()).expect("clean Tier-1 route");
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
        g.set_pos(1, (1, 0));
        g.set_pos(2, (3, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        g.add_edge(1, mapper::direction::Direction::W, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let right = buf.content.iter().filter(|c| c.symbol() == "▶").count();
        let left = buf.content.iter().filter(|c| c.symbol() == "◀").count();
        assert_eq!(right, 1, "A's east exit ▶ must be kept; got {right}");
        assert_eq!(left, 1, "A's west exit ◀ must be kept (not deduped as reciprocal); got {left}");
    }

    #[test]
    fn connectors_are_scroll_invariant() {
        // The routed connector geometry must be identical at every scroll offset —
        // scrolling is a pure translate-and-clip, never a re-layout. Render the same
        // map at two scrolls, map each ribbon cell back to virtual space, and assert
        // the two virtual ribbon sets are equal.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);
        g.add_edge(2, mapper::direction::Direction::E, 3);
        let rm = mapper::render::render(&g);

        let area = Rect::new(0, 0, 120, 40);
        let (sw, sh) = (29i32, 17i32); // Boxes stride

        let virtual_ribbon = |scroll: (i32, i32)| -> std::collections::BTreeSet<(i32, i32)> {
            let mut st = AppState::default();
            st.scroll = scroll;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            let off = (-scroll.0 * sw, -scroll.1 * sh);
            let mut set = std::collections::BTreeSet::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    let c = buf.cell((x, y)).unwrap();
                    if c.bg == Color::Cyan || c.bg == Color::Magenta {
                        set.insert((x as i32 - off.0, y as i32 - off.1));
                    }
                }
            }
            set
        };

        // Both scrolls keep all three rooms fully on-screen, so nothing clips away.
        let a = virtual_ribbon((0, 0));
        let b = virtual_ribbon((-1, -1));
        assert!(!a.is_empty(), "expected some ribbon cells");
        assert_eq!(
            a, b,
            "connector geometry must be scroll-independent (identical in virtual space)"
        );
    }

    #[test]
    fn route_keeps_gap_from_earlier_path() {
        // First path runs straight along row 5 (HORIZ). A second horizontal path that
        // would otherwise run alongside it on the adjacent row 6 must bend away to keep a
        // 1-cell gap (it may not sit parallel-adjacent to an existing same-orientation path).
        let no_blocked = std::collections::HashSet::new();
        let no_paths = std::collections::HashMap::new();
        let p1 = route_ortho((0, 5), (20, 5), Side::Right, &no_blocked, &no_paths).expect("clean Tier-1 route");

        // Record p1's cells with their orientation (same as render_map).
        let mut paths: std::collections::HashMap<(i32, i32), u8> = std::collections::HashMap::new();
        for w in p1.windows(2) {
            let (a, b) = (w[0], w[1]);
            let bit = if a.1 == b.1 { HORIZ } else { VERT };
            *paths.entry(a).or_insert(0) |= bit;
            *paths.entry(b).or_insert(0) |= bit;
        }

        let p2 = route_ortho((0, 6), (20, 6), Side::Right, &no_blocked, &paths).expect("clean Tier-1 route");
        // p2 detours to row >= 7 (row 6 would run alongside p1's row 5).
        assert!(
            p2.iter().any(|&(_, y)| y >= 7),
            "second path must keep a gap from the first (detour to row >=7); got {p2:?}"
        );
        // p2 must never overlap p1 (no shared cell on row 5).
        assert!(
            !p2.iter().any(|&(_, y)| y == 5),
            "second path must not overlap the first; got {p2:?}"
        );
    }

    #[test]
    fn route_ortho_returns_none_when_boxed_in() {
        // A full vertical wall of blocked cells between dep and arr leaves no clean
        // route → route_ortho must report None rather than an overlapping fallback.
        let mut blocked = std::collections::HashSet::new();
        for y in -30..30 {
            blocked.insert((2, y));
        }
        let no_paths = std::collections::HashMap::new();
        let r = route_ortho((0, 0), (10, 0), Side::Right, &blocked, &no_paths);
        assert!(r.is_none(), "a fully walled-off edge must return None; got {r:?}");
    }

    #[test]
    fn arrival_avoids_occupied_side() {
        // A non-reciprocal arrival must not land on a side the destination already uses
        // for a departure (or another arrival) — that's the "two paths on one arrow" bug.
        let dest = VRect { x: 0, y: 0, w: 21, h: 11 };
        let dep = (-10, 5); // due west of dest → nearest side is Left

        // Nothing occupied → the geometrically nearest side (Left).
        assert_eq!(nearest_free_side(dest, dep, &[]), Side::Left);

        // Left occupied (a departure sits there) → must pick a different side.
        let s = nearest_free_side(dest, dep, &[Side::Left]);
        assert_ne!(s, Side::Left, "arrival must avoid the occupied departure side; got {s:?}");

        // Left + the next-nearest also occupied → still avoids both.
        let s2 = nearest_free_side(dest, dep, &[Side::Left, s]);
        assert!(s2 != Side::Left && s2 != s, "arrival must avoid all occupied sides; got {s2:?}");
    }

    #[test]
    fn arrival_anchor_is_off_centre() {
        // The centre of a side is reserved for departures; a non-reciprocal arrival must
        // land beside it. Box 21×11: Top centre x = 10, Left/Right centre y = 5.
        let r = VRect { x: 0, y: 0, w: 21, h: 11 };

        // Top side: off-centre in x (centre would be x=10), still on the box edge row.
        let (tx, ty) = arrival_anchor(r, Side::Top, 0);
        assert_ne!(tx, r.x + r.w / 2, "top arrival must be off the centre column");
        assert_eq!(ty, r.y - 1, "top arrival sits on the row above the box");
        assert!(tx >= r.x && tx < r.right(), "top arrival stays on the box edge");

        // Right side: off-centre in y (centre would be y=5).
        let (rx, ry) = arrival_anchor(r, Side::Right, 0);
        assert_eq!(rx, r.right(), "right arrival sits on the column right of the box");
        assert_ne!(ry, r.y + r.h / 2, "right arrival must be off the centre row");

        // Consecutive indices on the same side land on distinct cells (no collision).
        let a0 = arrival_anchor(r, Side::Top, 0);
        let a1 = arrival_anchor(r, Side::Top, 1);
        assert_ne!(a0, a1, "two arrivals on one side must not share a cell");
    }

    #[test]
    fn route_crosses_perpendicular_straight_through() {
        // A horizontal path on row 5; a vertical path crossing it should pass straight
        // through the crossing cell (perpendicular crossings ARE allowed), not detour.
        let no_blocked = std::collections::HashSet::new();
        let no_paths = std::collections::HashMap::new();
        let p1 = route_ortho((0, 5), (20, 5), Side::Right, &no_blocked, &no_paths).expect("clean Tier-1 route");
        let mut paths: std::collections::HashMap<(i32, i32), u8> = std::collections::HashMap::new();
        for w in p1.windows(2) {
            let (a, b) = (w[0], w[1]);
            let bit = if a.1 == b.1 { HORIZ } else { VERT };
            *paths.entry(a).or_insert(0) |= bit;
            *paths.entry(b).or_insert(0) |= bit;
        }

        // Vertical path down column 10 from row 0 to row 10, crossing p1 at (10,5).
        let p2 = route_ortho((10, 0), (10, 10), Side::Bottom, &no_blocked, &paths).expect("clean Tier-1 route");
        let expected: Vec<(i32, i32)> = (0..=10).map(|y| (10, y)).collect();
        assert_eq!(p2, expected, "perpendicular crossing should go straight through; got {p2:?}");
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

        // B box: cell (1,0) → screen (29,0), size 21×21 → cols 29..49, rows 0..20.
        let b = Rect::new(29, 0, 21, 11);
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

        // Room B box: Boxes zoom, step=29×29, room at cell (1,0) → screen (29,0), box 21×21.
        // No path ribbon (Cyan/Magenta background) may appear inside B's interior.
        let b_rect = Rect::new(29, 0, 21, 11);
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

    #[test]
    fn unroutable_edge_renders_distinct_and_keeps_clean_edges_cyan() {
        // Force an edge that cannot route: erect an unbroken wall of rooms between
        // rooms 1 and 2 so route_ortho has no clean channel and must flag the edge
        // as unrouted (DarkGray ribbon). We set positions explicitly (bypassing
        // relayout) so the renderer must confront an un-routable edge.
        //
        // NOTE: Compact zoom is used because step_h == halo_h (5 == 5), so consecutive
        // rooms in the same column leave NO vertical gap — a single column of 13 rooms
        // creates an impenetrable wall covering the full A* search-space in y.
        // Boxes zoom (step_h=17, halo_h=13) would leave 4-row gaps that the A* can
        // thread through, so the wall cannot be built with Boxes zoom alone.
        use mapper::graph::MapGraph;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Color;

        let mut g = MapGraph::new();
        // Room 1: origin (east side of wall).
        // Room 2: destination (west of wall — unroutable from room 1).
        // Room 3: clean destination to the east of room 1.
        g.upsert_room(1, "r".into());
        g.upsert_room(2, "r".into());
        g.upsert_room(3, "r".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (-6, 0)); // west of the wall
        g.set_pos(3, (1, 0));  // east of origin — clean path exists

        // Wall of 13 rooms at column -2 (Compact: step_h=5, halo_h=5 → no gaps).
        // Halo covers x=-25..-16 for ALL y in the A* search bounds (-24..+24 around dep/arr).
        for k in -6i32..=6 {
            let id = 4 + (k + 6) as u16; // IDs 4..16
            g.upsert_room(id, "w".into());
            g.set_pos(id, (-2, k));
        }

        g.add_edge(1, mapper::direction::Direction::W, 2); // blocked by wall → unrouted
        g.add_edge(1, mapper::direction::Direction::E, 3); // clear path → clean Cyan ribbon
        let rm = mapper::render::render(&g);

        let area = Rect::new(0, 0, 200, 120);
        let mut buf = Buffer::empty(area);
        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.scroll = (-4, -4);
        render_map(&rm, &state, area, &mut buf);

        let mut has_unrouted = false;
        let mut has_clean = false;
        for y in 0..area.height {
            for x in 0..area.width {
                match buf.cell((x, y)).map(|c| c.bg) {
                    Some(Color::DarkGray) => has_unrouted = true,
                    Some(Color::Cyan) => has_clean = true,
                    _ => {}
                }
            }
        }
        assert!(has_unrouted, "the boxed-in edge must render as a distinct DarkGray ribbon");
        assert!(has_clean, "the clean edge must still render as a normal Cyan ribbon");
    }

    #[test]
    fn a129_full_map_renders_without_crossing_or_unrouted() {
        // The real ZCODE-88-840726-A129 graph: after relayout_auto (with crossing-aware
        // repair) the rendered map must have NO unrouted (DarkGray) ribbon and NO
        // perpendicular-crossing ribbon cell — the corner the user kept reporting.
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Color;

        let mut g = MapGraph::new();
        for (id, name) in [(25, "Canyon View"), (74, "Clearing"), (76, "Forest"),
                           (79, "Behind House"), (80, "South of House"), (180, "West of House")] {
            g.upsert_room(id, name.to_string());
        }
        g.add_edge(180, mapper::direction::Direction::S, 80);
        g.add_edge(80, mapper::direction::Direction::E, 79);
        g.add_edge(79, mapper::direction::Direction::S, 80);
        g.add_edge(80, mapper::direction::Direction::S, 76);
        g.add_edge(76, mapper::direction::Direction::N, 74);
        g.add_edge(74, mapper::direction::Direction::S, 76);
        g.add_edge(74, mapper::direction::Direction::E, 25);
        g.add_edge(25, mapper::direction::Direction::W, 76);
        g.set_current(76);
        relayout_auto(&mut g);

        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 300, 200);
        let mut buf = Buffer::empty(area);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = (-7, -7);
        render_map(&rm, &st, area, &mut buf);

        let is_ribbon = |b: &Buffer, x: i32, y: i32| {
            if x < 0 || y < 0 || x >= 300 || y >= 200 { return false; }
            matches!(
                b.cell((x as u16, y as u16)).map(|c| c.bg),
                Some(Color::Cyan) | Some(Color::Magenta) | Some(Color::DarkGray)
            )
        };
        let (mut unrouted, mut crossings) = (0, 0);
        for y in 0..200i32 {
            for x in 0..300i32 {
                if matches!(buf.cell((x as u16, y as u16)).map(|c| c.bg), Some(Color::DarkGray)) {
                    unrouted += 1;
                }
                if is_ribbon(&buf, x, y)
                    && is_ribbon(&buf, x - 1, y) && is_ribbon(&buf, x + 1, y)
                    && is_ribbon(&buf, x, y - 1) && is_ribbon(&buf, x, y + 1)
                {
                    crossings += 1;
                }
            }
        }
        assert_eq!(unrouted, 0, "no edge may render unrouted (DarkGray)");
        assert_eq!(crossings, 0, "no perpendicular crossing may remain in the corner");
    }
}
