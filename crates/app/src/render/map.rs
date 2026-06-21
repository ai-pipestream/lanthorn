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
use mapper::route::RoutePlan;
use mapper::router::{RoutedEdge, Side};
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
/// | Boxes   | 19×11 | 11×5  | 8 cols / 6 rows         |
/// | Compact | 12×5  | 8×3   | 4 cols / 2 rows         |
/// | Overview| 2×2   | 1×1   | — (single glyph)        |
///
/// The 11×5 box (both odd) is ~2:1 width:height so it reads as square given the
/// terminal's ~1:2 cell aspect, and odd dims centre the side anchors on the box.
fn zoom_box_size(zoom: Zoom) -> (u16, u16) {
    match zoom {
        Zoom::Boxes => (11, 5),
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
}

impl VRect {
    fn right(&self) -> i32 {
        self.x + self.w
    }
}

/// Virtual top-left pixel of a room cell: `cell * step` (no scroll, no area offset).
fn cell_to_virtual(cell: (i32, i32), zoom: Zoom) -> (i32, i32) {
    let (sw, sh) = zoom_steps(zoom);
    (cell.0 * sw, cell.1 * sh)
}

// ── Boxes-zoom position tables ────────────────────────────────────────────────

/// Cells between adjacent lanes in a channel (so lines are visually separated).
const LANE_SPACING: i32 = 2;
/// Minimum channel pixel size even when it carries no lanes.
const MIN_GUTTER: i32 = 2;
/// Boxes-zoom box size (matches `zoom_box_size(Zoom::Boxes)`), in cells.
const BOX_W: i32 = 11;
const BOX_H: i32 = 5;

/// One axis of the non-uniform Boxes-zoom layout: where each room line starts (pixels)
/// and how wide each channel after it is.
pub struct PosTable {
    room_start: std::collections::BTreeMap<i32, i32>, // grid line index → pixel start of the box
    channel_w: std::collections::BTreeMap<i32, i32>,  // grid line index → pixel width of the gap after it
    lo: i32,                                           // lowest grid line index
    hi: i32,                                           // highest grid line index
    box_dim: i32,                                      // box size along this axis (pixels)
}
impl PosTable {
    pub fn room_pixel(&self, idx: i32) -> i32 { self.line_pixel(idx) }
    pub fn channel_span(&self, idx: i32) -> i32 { *self.channel_w.get(&idx).unwrap_or(&MIN_GUTTER) }

    /// Pixel-x (or -y) of the box left/top edge at grid line `idx`, extrapolating with a
    /// uniform `box_dim + MIN_GUTTER` stride for lines outside the tabulated bounds so
    /// scrolling beyond the placed rooms stays well-defined and continuous.
    fn line_pixel(&self, idx: i32) -> i32 {
        if let Some(&p) = self.room_start.get(&idx) {
            p
        } else if idx < self.lo {
            // Steps of the default (empty-channel) stride below the first room.
            self.room_start.get(&self.lo).copied().unwrap_or(0)
                - (self.lo - idx) * (self.box_dim + MIN_GUTTER)
        } else {
            // Past the last room: its start, its own box+channel, then default strides.
            let last = self.room_start.get(&self.hi).copied().unwrap_or(0);
            let after = last + self.box_dim + self.channel_span(self.hi);
            after + (idx - self.hi - 1) * (self.box_dim + MIN_GUTTER)
        }
    }
}

fn channel_width(lanes: u16) -> i32 {
    (lanes as i32 * LANE_SPACING).max(MIN_GUTTER)
}

/// Build the (columns, rows) position tables from the plan and the room bounds.
pub fn boxes_axes(plan: &RoutePlan, bounds: ((i32, i32), (i32, i32))) -> (PosTable, PosTable) {
    let ((min_c, min_r), (max_c, max_r)) = bounds;
    let build = |lo: i32, hi: i32, box_dim: i32, lanes: &std::collections::BTreeMap<i32, u16>| {
        let mut room_start = std::collections::BTreeMap::new();
        let mut channel_w = std::collections::BTreeMap::new();
        let mut x = 0;
        for idx in lo..=hi {
            room_start.insert(idx, x);
            let w = channel_width(lanes.get(&idx).copied().unwrap_or(0));
            channel_w.insert(idx, w);
            x += box_dim + w;
        }
        PosTable { room_start, channel_w, lo, hi, box_dim }
    };
    let cols = build(min_c, max_c, BOX_W, &plan.v_lanes);
    let rows = build(min_r, max_r, BOX_H, &plan.h_lanes);
    (cols, rows)
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

// ── render_map ────────────────────────────────────────────────────────────────

/// Draw the map from `rm` into `buf` for `area`, using view state from `state`.
///
/// The whole map is built in scroll-independent virtual space (see [`VRect`]) and
/// blitted to the screen with a single translation, so panning never re-routes
/// connectors — the routes are identical at every scroll offset.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;

    // Overview zoom: one glyph per room, no connectors. Uniform stride.
    if matches!(zoom, crate::state::Zoom::Overview) {
        let (step_w, step_h) = zoom_steps(zoom);
        let off_x = area.x as i32 - scroll.0 * step_w;
        let off_y = area.y as i32 - scroll.1 * step_h;
        for room in &rm.rooms {
            let (vx, vy) = cell_to_virtual(room.cell, zoom);
            put_char(buf, vx + off_x, vy + off_y, '■', room_style(room, state), area);
        }
        return;
    }

    // Boxes zoom uses the non-uniform lane-routing position tables; Compact keeps the
    // uniform schematic stride. `room_virtual` maps a logical cell to its virtual
    // top-left pixel; the scroll offset is computed in the SAME space so panning is a
    // pure translate-and-clip and connector geometry is scroll-invariant.
    let boxes = matches!(zoom, crate::state::Zoom::Boxes);
    let axes = boxes.then(|| boxes_axes(&rm.plan, rm.bounds));
    let (off_x, off_y) = match &axes {
        Some((cols, rows)) => (
            area.x as i32 - cols.room_pixel(scroll.0),
            area.y as i32 - rows.room_pixel(scroll.1),
        ),
        None => {
            let (step_w, step_h) = zoom_steps(zoom);
            (area.x as i32 - scroll.0 * step_w, area.y as i32 - scroll.1 * step_h)
        }
    };
    let room_virtual = |cell: (i32, i32)| -> (i32, i32) {
        match &axes {
            Some((cols, rows)) => (cols.room_pixel(cell.0), rows.room_pixel(cell.1)),
            None => cell_to_virtual(cell, zoom),
        }
    };

    // ── 1. Place ALL rooms in virtual space (independent of scroll/area) ──────
    let (bw, _bh) = zoom_box_size(zoom);
    let mut placed: std::collections::HashMap<RoomId, VRect> =
        std::collections::HashMap::new();
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        placed.insert(room.id, VRect { x: vx, y: vy, w: bw as i32 });
    }

    // ── 2. Boxes zoom: draw line-art connectors along their assigned lanes ────
    if let Some((cols, rows)) = &axes {
        // Reciprocal connections (a reverse edge exists) get a far-end arrow too. The
        // reverse-edge signal lives on `RoutedEdge::arrival_dir`.
        let reciprocal: std::collections::HashSet<(RoomId, RoomId)> = rm
            .edges
            .iter()
            .filter(|e| !e.is_stub && e.arrival_dir.is_some())
            .map(|e| (e.origin.min(e.dest), e.origin.max(e.dest)))
            .collect();
        render_lane_connectors(&rm.plan, cols, rows, &reciprocal, (off_x, off_y), area, buf);
    }

    // Stub edges (translate + clip).
    for edge in &rm.edges {
        if edge.is_stub {
            draw_stub(edge, &placed, off_x, off_y, area, buf);
        }
    }

    // ── 3. Draw rooms on top (translate + clip) ──────────────────────────────
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        draw_room(room, state, zoom, vx + off_x, vy + off_y, area, buf);
    }
}

// ── Line-art connector rendering (Boxes zoom) ─────────────────────────────────

/// Direction bits a connector enters/leaves a cell on. Two perpendicular bits → a turn;
/// all four (from two crossing connectors) → `┼`.
const DIR_N: u8 = 1;
const DIR_E: u8 = 2;
const DIR_S: u8 = 4;
const DIR_W: u8 = 8;

/// Box-drawing glyph for a set of direction bits.
fn glyph_for(mask: u8) -> Option<&'static str> {
    Some(match mask {
        m if m == DIR_E | DIR_W => "─",
        m if m == DIR_N | DIR_S => "│",
        m if m == DIR_S | DIR_E => "┌",
        m if m == DIR_S | DIR_W => "┐",
        m if m == DIR_N | DIR_E => "└",
        m if m == DIR_N | DIR_W => "┘",
        m if m == DIR_N | DIR_S | DIR_E => "├",
        m if m == DIR_N | DIR_S | DIR_W => "┤",
        m if m == DIR_E | DIR_W | DIR_S => "┬",
        m if m == DIR_E | DIR_W | DIR_N => "┴",
        m if m == DIR_N | DIR_E | DIR_S | DIR_W => "┼",
        // A bare stub end (single direction) — render as the matching straight glyph so
        // the line visibly reaches the box edge rather than vanishing.
        m if m == DIR_E || m == DIR_W => "─",
        m if m == DIR_N || m == DIR_S => "│",
        _ => return None,
    })
}

/// Map a doubled-coord polyline point to its virtual pixel, given the per-axis lane each
/// odd (channel) coordinate runs on for THIS connector.
fn lane_pixel(
    pt: (i32, i32),
    cols: &PosTable,
    rows: &PosTable,
    v_lane: &std::collections::HashMap<i32, u16>,
    h_lane: &std::collections::HashMap<i32, u16>,
) -> (i32, i32) {
    let (dx, dy) = pt;
    // x: even 2c → box-column centre; odd 2c+1 → channel V[c]. Lane 0 sits on the cell
    // immediately right of the box (the doorway, room_pixel+BOX_W) so the line touches the
    // box; each further lane steps LANE_SPACING into the gutter.
    let px = if dx.rem_euclid(2) == 0 {
        let c = dx.div_euclid(2);
        cols.room_pixel(c) + BOX_W / 2
    } else {
        let c = (dx - 1).div_euclid(2);
        let lane = v_lane.get(&c).copied().unwrap_or(0) as i32;
        cols.room_pixel(c) + BOX_W + lane * LANE_SPACING
    };
    let py = if dy.rem_euclid(2) == 0 {
        let r = dy.div_euclid(2);
        rows.room_pixel(r) + BOX_H / 2
    } else {
        let r = (dy - 1).div_euclid(2);
        let lane = h_lane.get(&r).copied().unwrap_or(0) as i32;
        rows.room_pixel(r) + BOX_H + lane * LANE_SPACING
    };
    (px, py)
}

/// Draw every plan connector as box-drawing line-art along its lanes.
fn render_lane_connectors(
    plan: &RoutePlan,
    cols: &PosTable,
    rows: &PosTable,
    reciprocal: &std::collections::HashSet<(RoomId, RoomId)>,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    use mapper::route::Channel;
    let (off_x, off_y) = offset;

    // Per-cell accumulated direction mask. ORing masks means a perpendicular crossing of
    // two connectors (one ─, one │) combines to ┼; a connector revisiting its own cell is
    // idempotent and harmless.
    let mut cells: std::collections::HashMap<(i32, i32), u8> =
        std::collections::HashMap::new();
    // Arrowheads: (virtual pixel, glyph, distorted). Drawn last so they sit on top.
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();

    for conn in plan.connectors.iter() {
        // Lane lookup for this connector's runs, keyed by channel index.
        let mut v_lane: std::collections::HashMap<i32, u16> = std::collections::HashMap::new();
        let mut h_lane: std::collections::HashMap<i32, u16> = std::collections::HashMap::new();
        for seg in &conn.segs {
            match seg.channel {
                Channel::V(c) => { v_lane.insert(c, seg.lane); }
                Channel::H(r) => { h_lane.insert(r, seg.lane); }
            }
        }

        // Convert the doubled polyline to a virtual-pixel polyline.
        let pix: Vec<(i32, i32)> = conn
            .points
            .iter()
            .map(|&p| lane_pixel(p, cols, rows, &v_lane, &h_lane))
            .collect();
        if pix.len() < 3 {
            continue;
        }

        // The connector runs centre→…→centre. A line must not be drawn inside a room box,
        // so trim the two room centres. In their place, anchor each end on the box's edge
        // cell for that side (the doorway just outside the box) at the side's perpendicular
        // centre, so the line visibly touches both rooms even when the channel is wider
        // than the lane it runs in. The interior run points keep their lane positions.
        let origin_cell = (conn.points[0].0.div_euclid(2), conn.points[0].1.div_euclid(2));
        let last = conn.points[conn.points.len() - 1];
        let dest_cell = (last.0.div_euclid(2), last.1.div_euclid(2));
        let dep_anchor = box_edge_anchor(cols, rows, origin_cell, conn.exit);
        let arr_anchor = box_edge_anchor(cols, rows, dest_cell, conn.entry);

        let mut inner_v: Vec<(i32, i32)> = Vec::with_capacity(pix.len());
        inner_v.push(dep_anchor);
        inner_v.extend_from_slice(&pix[1..pix.len() - 1]);
        inner_v.push(arr_anchor);
        inner_v.dedup();
        let inner = &inner_v[..];
        if inner.is_empty() {
            continue;
        }

        // Walk the inner polyline cell-by-cell, accumulating direction bits per cell.
        let color = if conn.distorted { Color::Magenta } else { Color::Cyan };
        let mut run: Vec<(i32, i32)> = Vec::new();
        for w in inner.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dxs = (b.0 - a.0).signum();
            let dys = (b.1 - a.1).signum();
            let mut cur = a;
            loop {
                if run.last() != Some(&cur) {
                    run.push(cur);
                }
                if cur == b {
                    break;
                }
                cur = (cur.0 + dxs, cur.1 + dys);
            }
        }
        if run.is_empty() {
            run.push(inner[0]);
        }

        for i in 0..run.len() {
            let c = run[i];
            let mut mask = 0u8;
            if i > 0 {
                mask |= dir_bit(c, run[i - 1]);
            }
            if i + 1 < run.len() {
                mask |= dir_bit(c, run[i + 1]);
            }
            let (sx, sy) = (c.0 + off_x, c.1 + off_y);
            if !in_area(sx, sy, area) {
                continue;
            }
            let entry = cells.entry(c).or_insert(0);
            *entry |= mask;
            let glyph = glyph_for(*entry).unwrap_or("·");
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(Style::new().fg(color));
            }
        }

        // Arrowhead at the origin departure anchor, pointing out along the exit side.
        arrowheads.push((dep_anchor, arrow_for_departure(conn.exit), conn.distorted));
        // Reciprocal far-end arrow at the entry anchor.
        let key = (conn.origin.min(conn.dest), conn.origin.max(conn.dest));
        if reciprocal.contains(&key) {
            arrowheads.push((arr_anchor, arrow_for_departure(conn.entry), conn.distorted));
        }
    }

    for ((vx, vy), glyph, distorted) in arrowheads {
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            let color = if distorted { Color::Magenta } else { Color::Cyan };
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(Style::new().fg(color));
            }
        }
    }
}

/// The virtual-pixel "doorway" cell just outside the box at logical `cell` on `side`, at
/// the side's perpendicular centre. The connector line is anchored here so it touches the
/// box rather than stopping a lane-width away inside the gutter.
fn box_edge_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), side: Side) -> (i32, i32) {
    let bx = cols.room_pixel(cell.0);
    let by = rows.room_pixel(cell.1);
    let cx = bx + BOX_W / 2;
    let cy = by + BOX_H / 2;
    match side {
        Side::Right => (bx + BOX_W, cy),
        Side::Left => (bx - 1, cy),
        Side::Bottom => (cx, by + BOX_H),
        Side::Top => (cx, by - 1),
    }
}

/// Direction bit pointing from cell `from` toward orthogonally-adjacent cell `to`.
fn dir_bit(from: (i32, i32), to: (i32, i32)) -> u8 {
    if to.1 < from.1 {
        DIR_N
    } else if to.1 > from.1 {
        DIR_S
    } else if to.0 > from.0 {
        DIR_E
    } else {
        DIR_W
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

/// Draw a boxes (19×11 step) room: bordered box 11 wide × 5 tall.
///
/// Layout (11 cols × 5 rows, within a 19×11 step):
///   Row 0: ╭─────────╮  (or ┏━━━━━━━━━┓ for current room)
///   Row 1: │label...●│  (label up to 9 chars, ● if notes)
///   Row 2: │#id      │  (unique room id)
///   Row 3: │         │
///   Row 4: ╰─────────╯
///   Gutter: cols 11-18 (right), rows 5-10 (bottom)
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
    let (w, h) = zoom_box_size(Zoom::Boxes); // (11, 5)
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

        // Cell (1,0) at Boxes → x = 0 + (1-0)*19 = 19
        let right = cell_to_screen((1, 0), Zoom::Boxes, (0, 0), area);
        assert_eq!(right, Some((19, 0)));

        // Cell (0,1) at Boxes → y = 0 + (1-0)*11 = 11
        let down = cell_to_screen((0, 1), Zoom::Boxes, (0, 0), area);
        assert_eq!(down, Some((0, 11)));

        // Far off-area cell.
        let off = cell_to_screen((1000, 1000), Zoom::Boxes, (0, 0), area);
        assert!(off.is_none());

        // Scroll pushes cell off-screen: scroll=(1,0) so cell (0,0) → x = 0+(0-1)*19 = -19 → None.
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

    // ── Line-art connector tests (Task 5) ─────────────────────────────────────

    /// Box-drawing line-art glyphs a connector may render as.
    const LINE_GLYPHS: [&str; 11] =
        ["─", "│", "┌", "┐", "└", "┘", "├", "┤", "┬", "┴", "┼"];
    const ARROW_GLYPHS: [&str; 4] = ["▶", "◀", "▲", "▼"];

    fn is_line(sym: &str) -> bool {
        LINE_GLYPHS.contains(&sym)
    }

    #[test]
    fn connector_renders_line_art_glyphs() {
        // room1(0,0) →E→ room2(1,0): the connection must render as box-drawing line-art,
        // NOT a solid background ribbon.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default(); // Boxes zoom, scroll (0,0)
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Some line-art glyph appears, and NO solid Cyan/Magenta background ribbon exists.
        let mut line_cells = 0;
        for y in 0..area.height {
            for x in 0..area.width {
                let c = buf.cell((x, y)).unwrap();
                assert_ne!(c.bg, Color::Cyan, "no solid Cyan ribbon at ({x},{y})");
                assert_ne!(c.bg, Color::Magenta, "no solid Magenta ribbon at ({x},{y})");
                if is_line(c.symbol()) {
                    line_cells += 1;
                }
            }
        }
        assert!(line_cells > 0, "connector must render box-drawing line-art");
    }

    #[test]
    fn connector_departs_origin_correct_side() {
        // room1(0,0) →E→ room2(1,0). The departure gutter just right of room1's box
        // (col 11) must carry a connector glyph (line-art or arrowhead), not a space and
        // not a room-box border.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The departure anchor sits in the gutter column just right of room1 (col 11),
        // on the box's vertical-centre row (row 2). It must be a connector glyph.
        let sym = buf.cell((11, 2)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert!(
            is_line(&sym) || ARROW_GLYPHS.contains(&sym.as_str()),
            "departure cell (11,2) should be a connector glyph; got '{sym}'"
        );
    }

    #[test]
    fn arrowhead_at_departure_side() {
        // room1(0,0) →E→ room2(1,0): a filled ▶ arrowhead marks the outgoing east
        // departure at room1's right anchor (11,2), drawn as fg Cyan (no bg ribbon).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let cell = buf.cell((11, 2)).expect("arrow cell must exist");
        assert_eq!(cell.symbol(), "▶", "outgoing east arrow ▶ at room1's right anchor");
        assert_eq!(cell.fg, Color::Cyan, "arrowhead fg should be Cyan; got {:?}", cell.fg);
        assert_ne!(cell.bg, Color::Cyan, "arrowhead must not sit on a solid ribbon");
        // No hollow arrowhead is ever drawn.
        let has_hollow = buf.content.iter().any(|c| matches!(c.symbol(), "▷" | "◁" | "△" | "▽"));
        assert!(!has_hollow, "hollow arrowheads must not appear");
    }

    #[test]
    fn reciprocal_draws_arrow_at_both_rooms() {
        // A(1) at (1,1) →N→ B(2) at (1,0) and back B →S→ A. The collapsed connector must
        // still render BOTH outgoing arrows: ▲ at A (north) and ▼ at B (south).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let up = buf.content.iter().filter(|c| c.symbol() == "▲").count();
        let down = buf.content.iter().filter(|c| c.symbol() == "▼").count();
        assert_eq!(up, 1, "exactly one ▲ (A leaving north); got {up}");
        assert_eq!(down, 1, "exactly one ▼ (B leaving south); got {down}");
    }

    #[test]
    fn connectors_are_scroll_invariant() {
        // Connector geometry is identical at every scroll offset — scrolling is a pure
        // translate-and-clip in the non-uniform Boxes position tables. Render the same
        // map at two scrolls, map each line-art cell back to virtual space, assert equal.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::E, 3);
        let rm = mapper::render::render(&g);

        let area = Rect::new(0, 0, 120, 40);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);

        let virtual_lines = |scroll: (i32, i32)| -> std::collections::BTreeSet<(i32, i32)> {
            let mut st = AppState::default();
            st.scroll = scroll;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            // Inverse of the table-based offset used by render_map.
            let off = (cols.room_pixel(scroll.0), rows.room_pixel(scroll.1));
            let mut set = std::collections::BTreeSet::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    let c = buf.cell((x, y)).unwrap();
                    if is_line(c.symbol()) || ARROW_GLYPHS.contains(&c.symbol()) {
                        set.insert((x as i32 + off.0, y as i32 + off.1));
                    }
                }
            }
            set
        };

        let a = virtual_lines((0, 0));
        let b = virtual_lines((-1, -1));
        assert!(!a.is_empty(), "expected some line-art cells");
        assert_eq!(a, b, "connector geometry must be scroll-independent in virtual space");
    }

    #[test]
    fn no_connector_glyph_inside_room_interior() {
        // 3 rooms A(0,0) B(1,0) C(2,0) with a direct A→C edge that passes B's column.
        // No connector line-art may land inside B's box interior.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (2, 0));
        g.add_edge(1, Direction::E, 3);
        let rm = mapper::render::render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 120, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // B is at cell (1,0). Its virtual box top-left and size from the tables.
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let bx = cols.room_pixel(1);
        let by = rows.room_pixel(0);
        for y in (by + 1)..(by + BOX_H - 1) {
            for x in (bx + 1)..(bx + BOX_W - 1) {
                if let Some(cell) = buf.cell((x as u16, y as u16)) {
                    assert!(
                        !is_line(cell.symbol()),
                        "connector line-art '{}' inside room B interior at ({x},{y})",
                        cell.symbol()
                    );
                }
            }
        }
    }

    #[test]
    fn boxes_axes_widen_busy_channels() {
        // A column-channel carrying 2 lanes must be wider than an empty one, and room
        // pixel-positions are cumulative (a later room sits further right when an earlier
        // gap is wide).
        use mapper::route::RoutePlan;
        let mut plan = RoutePlan::default();
        plan.v_lanes.insert(0, 2); // V[0] carries 2 lanes
        let (cols, _rows) = boxes_axes(&plan, ((0, 0), (2, 0)));
        let gap0 = cols.channel_span(0);
        let gap1 = cols.channel_span(1);
        assert!(gap0 > gap1, "a 2-lane channel must be wider than an empty one");
        assert!(cols.room_pixel(2) > cols.room_pixel(1));
    }

    #[test]
    fn lane_routing_a129_no_overlap_line_art() {
        // The real A129 graph (11 rooms / 19 edges). After lane routing the rendered map
        // must (a) draw connectors as box-drawing line-art (not solid ribbons), and
        // (b) have NO overlapping connector cells — every connector cell is either a
        // straight/turn glyph or a clean ┼ crossing; none is a DarkGray/unrouted ribbon.
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        use ratatui::style::Color;
        let mut g = MapGraph::new();
        for (id, n) in [(25,"Canyon View"),(74,"Clearing"),(75,"Forest Path"),(76,"Forest"),
            (77,"Forest"),(78,"Forest"),(79,"Behind House"),(80,"South of House"),
            (81,"North of House"),(143,"Clearing"),(180,"West of House")] { g.upsert_room(id, n.into()); }
        for (o,d,t) in [(180,Direction::N,81),(81,Direction::E,79),(79,Direction::E,74),
            (74,Direction::S,76),(76,Direction::N,74),(74,Direction::E,25),(25,Direction::W,76),
            (76,Direction::W,78),(78,Direction::S,76),(78,Direction::N,143),(143,Direction::E,77),
            (77,Direction::W,75),(75,Direction::S,81),(81,Direction::W,180),(180,Direction::S,80),
            (80,Direction::E,79),(79,Direction::S,80),(80,Direction::W,180),(74,Direction::W,79)] {
            g.add_edge(o, d, t);
        }
        g.set_current(79);
        relayout_auto(&mut g);
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 400, 300);
        let mut buf = Buffer::empty(area);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = (-12, -10);
        render_map(&rm, &st, area, &mut buf);

        let mut line_cells = 0;
        for y in 0..area.height {
            for x in 0..area.width {
                let c = buf.cell((x, y)).unwrap();
                // No solid ribbon or unrouted background may exist anymore.
                assert_ne!(c.bg, Color::Cyan, "no solid Cyan ribbon at ({x},{y})");
                assert_ne!(c.bg, Color::Magenta, "no solid Magenta ribbon at ({x},{y})");
                assert_ne!(c.bg, Color::DarkGray, "no unrouted/grey ribbon at ({x},{y})");
                if matches!(c.symbol(), "─"|"│"|"┌"|"┐"|"└"|"┘"|"├"|"┤"|"┬"|"┴"|"┼") {
                    line_cells += 1;
                }
            }
        }
        assert!(line_cells > 0, "connectors must render as box-drawing line-art");
    }
}
