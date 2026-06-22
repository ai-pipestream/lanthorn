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
/// Gap between the box edge (doorway) and lane 0, so channel runs never graze the box edge
/// where same-side departure/arrival anchors live.
const LANE_BASE: i32 = 1;
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

    /// Total pixel extent from the first room's box-left to just past the last room's
    /// trailing channel. This is the minimum pixel span needed to draw all rooms and
    /// their inter-room channels without clipping.
    pub fn total_pixels(&self) -> i32 {
        let last = self.room_pixel(self.hi);
        last + self.box_dim + self.channel_span(self.hi)
    }

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
    // Reserve LANE_BASE before lane 0 plus LANE_SPACING per additional lane, so the widest
    // lane (LANE_BASE + (lanes-1)*LANE_SPACING) stays inside the channel. Empty channels keep
    // MIN_GUTTER so adjacent boxes never touch.
    if lanes == 0 {
        MIN_GUTTER
    } else {
        (LANE_BASE + (lanes as i32 - 1) * LANE_SPACING + 1).max(MIN_GUTTER)
    }
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
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();
    if let Some((cols, rows)) = &axes {
        arrowheads = render_lane_connectors(&rm.plan, cols, rows, (off_x, off_y), area, buf);
    }

    // Stub edges (translate + clip).
    for edge in &rm.edges {
        if edge.is_stub {
            draw_stub(edge, &placed, off_x, off_y, area, buf);
        }
    }

    // ── 3. Draw rooms on top of the line-art (translate + clip) ───────────────
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        draw_room(room, state, zoom, vx + off_x, vy + off_y, area, buf);
    }

    // ── 4. Draw departure/arrival arrowheads LAST, so each embeds in the room ─
    //       border it sits on (replacing the box-edge glyph, pointing outward).
    draw_connector_arrows(&arrowheads, (off_x, off_y), area, buf);
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

/// Resolve the lane a connector point runs on within `channel`, by finding the `LaneSeg`
/// whose channel AND doubled-coord extent (`start..=end`) contains the point's position
/// along that channel's free axis. A single connector legitimately has TWO segments in the
/// same channel on different lanes (one per run), so a per-channel-index lookup is wrong —
/// it would collapse both runs onto one lane and draw them overlapping.
fn seg_lane(segs: &[mapper::route::LaneSeg], channel: mapper::route::Channel, along: i32) -> u16 {
    segs.iter()
        .find(|s| s.channel == channel && s.start <= along && along <= s.end)
        .map(|s| s.lane)
        .unwrap_or(0)
}

/// Map a doubled-coord polyline point to its virtual pixel, resolving each odd (channel)
/// coordinate's lane against THIS connector's lane segments by extent.
fn lane_pixel(
    pt: (i32, i32),
    cols: &PosTable,
    rows: &PosTable,
    segs: &[mapper::route::LaneSeg],
) -> (i32, i32) {
    use mapper::route::Channel;
    let (dx, dy) = pt;
    // x: even 2c → box-column centre; odd 2c+1 → channel V[c]. Lane 0 sits ONE cell into the
    // gutter (room_pixel + BOX_W + LANE_BASE), NOT on the box-edge doorway, so a channel run
    // never grazes the box edge where departure/arrival anchors live (otherwise an arriving
    // lane-0 line would run right alongside every same-side departure anchor). Each further
    // lane steps LANE_SPACING deeper. The departure/arrival anchors bridge to lane 0 across
    // the doorway cell, so lines still visibly touch the box.
    let px = if dx.rem_euclid(2) == 0 {
        let c = dx.div_euclid(2);
        cols.room_pixel(c) + BOX_W / 2
    } else {
        let c = (dx - 1).div_euclid(2);
        // A V(c) run varies along y; pick the segment whose y-extent contains dy.
        let lane = seg_lane(segs, Channel::V(c), dy) as i32;
        cols.room_pixel(c) + BOX_W + LANE_BASE + lane * LANE_SPACING
    };
    let py = if dy.rem_euclid(2) == 0 {
        let r = dy.div_euclid(2);
        rows.room_pixel(r) + BOX_H / 2
    } else {
        let r = (dy - 1).div_euclid(2);
        // An H(r) run varies along x; pick the segment whose x-extent contains dx.
        let lane = seg_lane(segs, Channel::H(r), dx) as i32;
        rows.room_pixel(r) + BOX_H + LANE_BASE + lane * LANE_SPACING
    };
    (px, py)
}

/// The cells (in virtual space) one connector writes, each with the direction-bit mask it
/// contributes there, plus its departure/arrival arrowhead anchors. This is the single
/// source of truth for connector plotting: the renderer ORs these per-cell masks into the
/// shared buffer, and tests re-derive per-connector ownership from the same geometry.
struct ConnectorPlot {
    cells: Vec<((i32, i32), u8)>,
    dep_anchor: (i32, i32),
    arr_anchor: (i32, i32),
}

/// Compute the virtual cells + per-cell masks a single connector occupies.
fn plot_connector(conn: &mapper::route::RoutedConnector, cols: &PosTable, rows: &PosTable) -> Option<ConnectorPlot> {
    // Convert the doubled polyline to a virtual-pixel polyline, resolving each point's lane
    // against this connector's segments by channel + extent (a connector may have two runs
    // in one channel on different lanes).
    let pix: Vec<(i32, i32)> = conn
        .points
        .iter()
        .map(|&p| lane_pixel(p, cols, rows, &conn.segs))
        .collect();
    if pix.len() < 3 {
        return None;
    }

    // The connector runs centre→…→centre. A line must not be drawn inside a room box, so
    // trim the two room centres. In their place, anchor each end on the box's edge cell for
    // that side (the doorway just outside the box), displaced along the edge by the slot, so
    // the line visibly touches both rooms even when the channel is wider than the lane it
    // runs in, and two connectors sharing a side land on distinct cells.
    let origin_cell = (conn.points[0].0.div_euclid(2), conn.points[0].1.div_euclid(2));
    let last = conn.points[conn.points.len() - 1];
    let dest_cell = (last.0.div_euclid(2), last.1.div_euclid(2));
    let dep_anchor = box_edge_anchor(cols, rows, origin_cell, conn.exit, conn.exit_slot);
    let arr_anchor = box_edge_anchor(cols, rows, dest_cell, conn.entry, conn.entry_slot);

    // The arrow anchor sits ON the box border. Each connector leaves the box straight out at 90°
    // (a perpendicular stub on the anchor's own row/col), then steps along the edge into the
    // first/last interior channel point. Distinct slots give distinct border cells; the straight
    // connector on each side keeps slot 0 (centre), so a displaced connector crosses it as a
    // single clean ┼ instead of a corner stomp.
    let first_interior = pix[1];
    let last_interior = pix[pix.len() - 2];
    let dep_bridge = attach_bridge(dep_anchor, first_interior, conn.exit);
    let arr_bridge = attach_bridge(arr_anchor, last_interior, conn.entry);

    let mut inner_v: Vec<(i32, i32)> = Vec::with_capacity(pix.len() + 6);
    inner_v.push(dep_anchor);
    inner_v.extend_from_slice(&dep_bridge);
    inner_v.extend_from_slice(&pix[1..pix.len() - 1]);
    for &p in arr_bridge.iter().rev() { inner_v.push(p); }
    inner_v.push(arr_anchor);
    inner_v.dedup();
    let inner = &inner_v[..];
    if inner.is_empty() {
        return None;
    }

    // Walk the inner polyline cell-by-cell.
    let mut run: Vec<(i32, i32)> = Vec::new();
    for w in inner.windows(2) {
        let (a, b) = (w[0], w[1]);
        debug_assert!(a.0 == b.0 || a.1 == b.1, "bridge must be orthogonal: {a:?}->{b:?}");
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
    // Remove out-and-back spurs: a slot-offset anchor whose stub centre sits one cell off the
    // run's natural direction can leave a 1-cell dead-end (…A,B,A…). Collapse them so the
    // line is a clean path with no dangling tail that would clip a neighbour.
    let mut changed = true;
    while changed && run.len() >= 3 {
        changed = false;
        let mut i = 1;
        while i + 1 < run.len() {
            if run[i - 1] == run[i + 1] {
                run.remove(i + 1);
                run.remove(i);
                changed = true;
            } else {
                i += 1;
            }
        }
    }

    let mut cells = Vec::with_capacity(run.len());
    for i in 0..run.len() {
        let c = run[i];
        let mut mask = 0u8;
        if i > 0 {
            mask |= dir_bit(c, run[i - 1]);
        }
        if i + 1 < run.len() {
            mask |= dir_bit(c, run[i + 1]);
        }
        cells.push((c, mask));
    }
    Some(ConnectorPlot { cells, dep_anchor, arr_anchor })
}

/// Draw every plan connector as box-drawing line-art along its lanes, and RETURN the departure
/// (and reciprocal arrival) arrowheads as `(virtual pixel, glyph, distorted)`. The arrowheads
/// are NOT drawn here: each sits ON a room's border cell, so the caller draws them AFTER the
/// rooms (which render on top of the line-art) so the arrow replaces the box-border glyph.
fn render_lane_connectors(
    plan: &RoutePlan,
    cols: &PosTable,
    rows: &PosTable,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) -> Vec<((i32, i32), &'static str, bool)> {
    let (off_x, off_y) = offset;

    // Per-cell accumulated direction mask. ORing masks means a perpendicular crossing of
    // two connectors (one ─, one │) combines to ┼; a connector revisiting its own cell is
    // idempotent and harmless.
    let mut cells: std::collections::HashMap<(i32, i32), u8> =
        std::collections::HashMap::new();
    // Arrowheads: (virtual pixel, glyph, distorted). Returned for the caller to draw on top
    // of the rooms (the arrow embeds in the room border).
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();

    for conn in plan.connectors.iter() {
        let Some(plot) = plot_connector(conn, cols, rows) else { continue };
        let color = if conn.distorted { Color::Magenta } else { Color::Cyan };

        for (c, mask) in &plot.cells {
            let (sx, sy) = (c.0 + off_x, c.1 + off_y);
            if !in_area(sx, sy, area) {
                continue;
            }
            let entry = cells.entry(*c).or_insert(0);
            *entry |= *mask;
            let glyph = glyph_for(*entry).unwrap_or("·");
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(Style::new().fg(color));
            }
        }

        // Arrowhead at the origin departure anchor, pointing out along the exit side.
        arrowheads.push((plot.dep_anchor, arrow_for_departure(conn.exit), conn.distorted));
        // Far-end arrow only for true reciprocal connectors (collapsed opposite pairs).
        if conn.reciprocal {
            arrowheads.push((plot.arr_anchor, arrow_for_departure(conn.entry), conn.distorted));
        }
    }
    arrowheads
}

/// Draw the embedded-in-border arrowheads (from [`render_lane_connectors`]) on top of the rooms.
fn draw_connector_arrows(
    arrowheads: &[((i32, i32), &'static str, bool)],
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    let (off_x, off_y) = offset;
    for &((vx, vy), glyph, distorted) in arrowheads {
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            let color = if distorted { Color::Magenta } else { Color::Cyan };
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(Style::new().fg(color));
            }
        }
    }
}


/// Map a per-(room, side) slot index to a signed offset ALONG the box edge so multiple
/// connectors on one side anchor on distinct cells. Slot 0 stays on the side centre;
/// further slots fan out symmetrically (+1, -1, +2, -2, …), clamped to `max` so anchors
/// never leave the box edge.
fn slot_offset(slot: u16, max: i32) -> i32 {
    let step = ((slot as i32) + 1) / 2;
    let signed = if slot % 2 == 1 { step } else { -step };
    signed.clamp(-max, max)
}

/// The virtual-pixel cell ON the box border at logical `cell` on `side`, displaced ALONG the
/// box edge by this connector's per-(room, side) `slot`. This is the cell where the outgoing
/// arrowhead is drawn — it REPLACES the box-border glyph (a `│` on a vertical side, a `─` on
/// a horizontal side), so the arrow reads as embedded in the room outline. The connector line
/// then continues perpendicular OUT from this cell (see [`attach_bridge`]).
///
/// Slots map to distinct INTERIOR rows/cols along the side (never the corners), so two
/// connectors sharing a side land on distinct border cells.
fn box_edge_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), side: Side, slot: u16) -> (i32, i32) {
    let bx = cols.room_pixel(cell.0);
    let by = rows.room_pixel(cell.1);
    let cx = bx + BOX_W / 2;
    let cy = by + BOX_H / 2;
    // Along a vertical side (Left/Right) the edge runs in y; offset rows, clamped so the
    // anchor stays on the box's interior rows (off the corners). Along a horizontal side
    // (Top/Bottom) offset cols likewise.
    let v_max = BOX_H / 2 - 1; // keep off the corners
    let h_max = BOX_W / 2 - 1;
    match side {
        Side::Right => (bx + BOX_W - 1, cy + slot_offset(slot, v_max)),
        Side::Left => (bx, cy + slot_offset(slot, v_max)),
        Side::Bottom => (cx + slot_offset(slot, h_max), by + BOX_H - 1),
        Side::Top => (cx + slot_offset(slot, h_max), by),
    }
}

/// Build the orthogonal bridge from a border `anchor` out to its first/last `interior` channel
/// point, returning the single intermediate turn point (anchor and interior are NOT included),
/// or empty when they already line up.
///
/// The connector leaves the box PERPENDICULAR to `side` (a straight stub at 90°), running in the
/// ANCHOR's own column/row all the way out to the interior's perpendicular level, then steps
/// ALONG the edge into the interior. Keeping the perpendicular leg on the anchor's own
/// column/row (not the interior's) means the only along-edge move happens AT the interior — so
/// where a slot-displaced connector must cross a straight connector sitting on the side centre,
/// it crosses that centre line as a single straight pass, yielding a clean ┼ rather than a
/// corner-on-corner stomp.
fn attach_bridge(anchor: (i32, i32), interior: (i32, i32), side: Side) -> Vec<(i32, i32)> {
    let turn = match side {
        // Perpendicular axis = x: run in x at the anchor's row out to interior.x, then step in y.
        Side::Right | Side::Left => (interior.0, anchor.1),
        // Perpendicular axis = y: run in y at the anchor's column out to interior.y, then step x.
        Side::Top | Side::Bottom => (anchor.0, interior.1),
    };
    if turn == anchor || turn == interior {
        Vec::new()
    } else {
        vec![turn]
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

// ── Router-measured overlap cleanup ───────────────────────────────────────────

/// Count illegal connector overlaps and clean ┼ crossings in a rendered plan.
/// For each virtual cell, OR each connector's mask bits (a connector may write a cell
/// twice). A cell written by ≥2 DISTINCT connectors is a clean crossing ONLY if exactly
/// 2 connectors share it, one contributing exactly E|W and the other exactly N|S;
/// everything else (≥3 connectors, corner-on-corner, parallel run-alongside) is illegal.
/// Returns (illegal_count, clean_crossing_count). Counts are order-independent, so the
/// internal HashMap accumulation is deterministic in its RESULT.
#[allow(dead_code)]
pub(crate) fn overlap_stats(
    plan: &mapper::route::RoutePlan, cols: &PosTable, rows: &PosTable,
) -> (usize, usize) {
    use std::collections::{BTreeMap, HashMap};
    let mut owners: HashMap<(i32, i32), BTreeMap<usize, u8>> = HashMap::new();
    for (ci, conn) in plan.connectors.iter().enumerate() {
        if let Some(plot) = plot_connector(conn, cols, rows) {
            for (c, mask) in &plot.cells {
                *owners.entry(*c).or_default().entry(ci).or_insert(0) |= *mask;
            }
        }
    }
    let ew = DIR_E | DIR_W;
    let ns = DIR_N | DIR_S;
    let (mut illegal, mut crossings) = (0usize, 0usize);
    for per_conn in owners.values() {
        if per_conn.len() < 2 {
            continue;
        }
        let mut masks: Vec<u8> = per_conn.values().copied().collect();
        masks.sort_unstable();
        let mut expected = [ns, ew];
        expected.sort_unstable();
        if per_conn.len() == 2 && masks == expected {
            crossings += 1;
        } else {
            illegal += 1;
        }
    }
    (illegal, crossings)
}

/// Render `graph` and return its (illegal_overlaps, crossings).
#[allow(dead_code)]
fn render_overlap_stats(graph: &mapper::graph::MapGraph) -> (usize, usize) {
    let rm = mapper::render::render(graph);
    let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
    overlap_stats(&rm.plan, &cols, &rows)
}

/// Nudge rooms (bounded Chebyshev `radius`, ≤ `max_passes` passes) until the rendered
/// plan has zero illegal overlaps, secondarily fewer crossings. Deterministic, no overlap,
/// integer cells. Existing position is restored on every rejected trial.
#[allow(dead_code)]
pub(crate) fn cleanup_overlaps(graph: &mut mapper::graph::MapGraph, radius: i32, max_passes: usize) {
    // Gather move candidates in fixed order: increasing Chebyshev distance, then (dy, dx).
    let moves: Vec<(i32, i32)> = {
        let mut v = Vec::new();
        for dist in 1..=radius {
            let mut candidates: Vec<(i32, i32)> = (-dist..=dist)
                .flat_map(|dy| (-dist..=dist).map(move |dx| (dy, dx)))
                .filter(|&(dy, dx)| dy.abs().max(dx.abs()) == dist)
                .collect();
            candidates.sort_unstable();
            v.extend(candidates);
        }
        v
    };

    for _ in 0..max_passes {
        let base = render_overlap_stats(graph);
        if base.0 == 0 {
            break;
        }

        // Collect placed rooms in ascending id order.
        let room_ids: Vec<mapper::graph::RoomId> = graph
            .rooms()
            .filter(|r| r.pos.is_some())
            .map(|r| r.id)
            .collect();

        let mut improved = false;
        'room_loop: for &id in &room_ids {
            let orig = match graph.room(id).and_then(|r| r.pos) {
                Some(p) => p,
                None => continue,
            };

            for &(dy, dx) in &moves {
                let trial = (orig.0 + dx, orig.1 + dy);

                // Skip if another room occupies this cell.
                let occupied = graph.rooms().any(|r| r.id != id && r.pos == Some(trial));
                if occupied {
                    continue;
                }

                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                if (s.0, s.1) < (base.0, base.1) {
                    improved = true;
                    break 'room_loop;
                }
                // Restore original position.
                graph.set_pos(id, orig);
            }
        }

        if !improved {
            break;
        }
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
        // room1(0,0) →E→ room2(1,0): a filled ▶ arrowhead marks the outgoing east departure
        // EMBEDDED IN room1's right border. The box is 11 wide at x=0, so the right border is
        // column 10; the vertical-centre row is 2. The arrow replaces that border │ at (10,2),
        // drawn fg Cyan (no bg ribbon). The line then continues perpendicular out (col 11+).
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

        let cell = buf.cell((10, 2)).expect("arrow cell must exist");
        assert_eq!(cell.symbol(), "▶", "outgoing east arrow ▶ embedded in room1's right border");
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

    /// Per-virtual-cell connector ownership: for each cell, the list of (connector_index,
    /// per-connector direction-bit mask) pairs from every connector that wrote that cell.
    /// Re-derives plotting per connector from the same `plot_connector` geometry the renderer
    /// uses, so a cell shared by ≥2 distinct connectors is detectable with full per-connector
    /// mask information (not just the OR, which masks corner-on-corner collisions).
    fn connector_ownership(
        plan: &mapper::route::RoutePlan,
        cols: &PosTable,
        rows: &PosTable,
    ) -> std::collections::HashMap<(i32, i32), Vec<(usize, u8)>> {
        let mut owners: std::collections::HashMap<(i32, i32), Vec<(usize, u8)>> =
            std::collections::HashMap::new();
        for (ci, conn) in plan.connectors.iter().enumerate() {
            if let Some(plot) = plot_connector(conn, cols, rows) {
                for (c, mask) in &plot.cells {
                    owners.entry(*c).or_default().push((ci, *mask));
                }
            }
        }
        owners
    }

    /// Assert no virtual cell is written by ≥2 distinct connectors unless it is a TRUE
    /// perpendicular crossing: exactly 2 connectors, one contributing exactly E|W (horizontal
    /// straight) and the other exactly N|S (vertical straight). Corner-on-corner collisions
    /// (e.g. ┌ + ┘ or └ + ┐, which OR to all-four bits but are not traceable) and any cell
    /// with ≥3 connectors are rejected. Returns the number of clean ┼ crossings seen.
    fn assert_no_overlap(
        owners: &std::collections::HashMap<(i32, i32), Vec<(usize, u8)>>,
    ) -> usize {
        let ew = DIR_E | DIR_W;
        let ns = DIR_N | DIR_S;
        let mut crossings = 0;
        for (cell, entries) in owners {
            // Deduplicate by connector index (a connector may contribute the same cell twice
            // due to run deduplication; OR their masks together per connector).
            let mut per_conn: std::collections::BTreeMap<usize, u8> = std::collections::BTreeMap::new();
            for &(ci, mask) in entries {
                *per_conn.entry(ci).or_insert(0) |= mask;
            }
            if per_conn.len() >= 2 {
                let idx_list: Vec<usize> = per_conn.keys().copied().collect();
                let masks: Vec<u8> = per_conn.values().copied().collect();
                assert_eq!(
                    per_conn.len(), 2,
                    "cell {cell:?} shared by {n} connectors {idx_list:?} (masks={masks:?}) — \
                     only 2-connector perpendicular crossings are legal",
                    n = per_conn.len(),
                );
                // True perpendicular crossing: one connector carries E|W, the other N|S.
                // Sorted so the comparison is order-independent.
                let mut sorted_masks = masks.clone();
                sorted_masks.sort_unstable();
                let mut expected = [ns, ew];
                expected.sort_unstable();
                assert_eq!(
                    sorted_masks, expected,
                    "cell {cell:?} shared by connectors {idx_list:?} with masks {masks:?} is not \
                     a clean ┼ crossing — each contributor must be exactly E|W ({ew:#04b}) or \
                     N|S ({ns:#04b}); corner-on-corner turns are rejected",
                );
                crossings += 1;
            }
        }
        crossings
    }

    /// Collect every arrowhead cell with its owning connector index. Mirrors the renderer's
    /// logic in `render_lane_connectors`: every connector gets a departure arrow at its
    #[test]
    fn two_connectors_perpendicular_crossing_is_single_cross() {
        // A vertical connector (1 above 2) and a horizontal connector (3 left of 4) routed so
        // their long runs cross exactly once. The shared cell must be a single clean ┼.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (2, 0));
        g.set_pos(2, (2, 2));
        g.set_pos(3, (0, 1));
        g.set_pos(4, (4, 1));
        g.add_edge(1, Direction::S, 2);
        g.add_edge(3, Direction::E, 4);
        let rm = mapper::render::render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let owners = connector_ownership(&rm.plan, &cols, &rows);
        let crossings = assert_no_overlap(&owners);
        assert_eq!(crossings, 1, "the two perpendicular connectors must cross at exactly one ┼");

        // The rendered glyph at the crossing is ┼.
        let cross_cell = owners.iter()
            .find(|(_, entries)| {
                let unique: std::collections::BTreeSet<usize> = entries.iter().map(|&(ci, _)| ci).collect();
                unique.len() >= 2
            })
            .map(|(k, _)| *k).unwrap();
        let area = Rect::new(0, 0, 160, 80);
        let mut buf = Buffer::empty(area);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = rm.bounds.0;
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0.0), rows.room_pixel(rm.bounds.0.1));
        let (sx, sy) = (cross_cell.0 - off.0, cross_cell.1 - off.1);
        assert_eq!(
            buf.cell((sx as u16, sy as u16)).unwrap().symbol(), "┼",
            "the crossing cell must render as ┼",
        );
    }

    #[test]
    fn overlap_stats_clean_pair_is_zero() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0);
    }

    #[test]
    fn cleanup_clears_a129_illegal_overlaps() {
        // The real A129 graph: pure sort layout leaves an illegal corner overlap; the
        // router-measured cleanup must move rooms until zero illegal overlaps remain.
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let mut g = MapGraph::new();
        for (id, name) in [
            (74, "Clearing"), (75, "Forest Path"), (77, "Forest"), (78, "Forest"),
            (79, "Behind House"), (80, "South of House"), (81, "North of House"),
            (143, "Clearing"), (180, "West of House"), (239, "Forest"),
        ] { g.upsert_room(id, name.into()); }
        for (o, d, dst) in [
            (180, Direction::N, 81), (81, Direction::W, 180), (180, Direction::S, 80),
            (80, Direction::E, 79), (79, Direction::N, 81), (81, Direction::E, 79),
            (79, Direction::S, 80), (80, Direction::W, 180), (180, Direction::W, 78),
            (78, Direction::N, 143), (143, Direction::S, 75), (75, Direction::N, 143),
            (143, Direction::W, 78), (143, Direction::E, 77), (77, Direction::S, 74),
            (74, Direction::N, 77), (77, Direction::E, 239), (239, Direction::N, 77),
            (239, Direction::S, 77),
        ] { g.add_edge(o, d, dst); }
        relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0, "cleanup must clear all illegal overlaps on A129");
        // rooms still distinct cells
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap after cleanup");
    }

    #[test]
    fn cleanup_is_deterministic() {
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let build = || {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3, 4, 5] { g.upsert_room(id, "r".into()); }
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::N, 3);
            g.add_edge(3, Direction::W, 4);
            g.add_edge(4, Direction::S, 5);
            g.add_edge(5, Direction::E, 1);
            relayout_auto(&mut g);
            g
        };
        let mut g1 = build();
        let mut g2 = build();
        cleanup_overlaps(&mut g1, 3, 40);
        cleanup_overlaps(&mut g2, 3, 40);
        let p1: Vec<_> = g1.rooms().map(|r| (r.id, r.pos)).collect();
        let p2: Vec<_> = g2.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(p1, p2, "cleanup must be deterministic");
    }

    #[test]
    fn multi_lane_in_one_channel_resolves_per_segment() {
        // Regression for the CRITICAL bug: a connector with TWO runs in the SAME channel on
        // DIFFERENT lanes must map each run's points to its OWN lane, resolved by the segment
        // whose extent contains the point — not by a per-channel-index lookup that overwrites
        // and collapses both runs onto one lane (which drew two connectors overlapping).
        use mapper::route::{Channel, LaneSeg};
        let plan = mapper::route::RoutePlan::default();
        let (cols, rows) = boxes_axes(&plan, ((0, 0), (1, 0)));
        // Two V(0) runs at different y-extents on different lanes.
        let segs = vec![
            LaneSeg { channel: Channel::V(0), lane: 0, start: 1, end: 3 },
            LaneSeg { channel: Channel::V(0), lane: 1, start: 5, end: 7 },
        ];
        let p_lane0 = lane_pixel((1, 2), &cols, &rows, &segs); // odd x=1 → V(0), y=2 ∈ [1,3]
        let p_lane1 = lane_pixel((1, 6), &cols, &rows, &segs); // odd x=1 → V(0), y=6 ∈ [5,7]
        assert_ne!(
            p_lane0.0, p_lane1.0,
            "two runs in one channel on different lanes must map to different columns; \
             a per-channel-index map would collapse them (both {:?})",
            p_lane0.0,
        );
        assert_eq!(p_lane1.0 - p_lane0.0, LANE_SPACING, "lane 1 sits one LANE_SPACING beyond lane 0");
    }

}
