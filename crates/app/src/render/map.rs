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
use mapper::direction::Direction;
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


/// Diagonal arrow glyphs (swappable named constants; e.g. to `◥◤◣◢`).
const DIAG_NE: &str = "↗";
const DIAG_NW: &str = "↖";
const DIAG_SE: &str = "↘";
const DIAG_SW: &str = "↙";

/// Arrow glyph for a diagonal departure/arrival (caller guards with `is_diagonal`).
fn diagonal_arrow(dir: Direction) -> &'static str {
    match dir {
        Direction::NE => DIAG_NE,
        Direction::NW => DIAG_NW,
        Direction::SE => DIAG_SE,
        Direction::SW => DIAG_SW,
        _ => DIAG_NE, // unreachable when guarded by is_diagonal
    }
}

/// The box-corner cell (virtual pixels) for a diagonal direction: NE→top-right, NW→top-left,
/// SE→bottom-right, SW→bottom-left.
fn corner_anchor(cols: &PosTable, rows: &PosTable, cell: (i32, i32), dir: Direction) -> (i32, i32) {
    let bx = cols.room_pixel(cell.0);
    let by = rows.room_pixel(cell.1);
    match dir {
        Direction::NE => (bx + BOX_W - 1, by),
        Direction::NW => (bx, by),
        Direction::SE => (bx + BOX_W - 1, by + BOX_H - 1),
        Direction::SW => (bx, by + BOX_H - 1),
        _ => (bx + BOX_W / 2, by), // unreachable when guarded by is_diagonal
    }
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

    // ── 2. Stub (portal) edges at non-Boxes zoom keep the bare-label `draw_stub`; Boxes zoom draws
    //       the in-room portal-icon overlay after the rooms (below).
    for edge in &rm.edges {
        if edge.is_stub && !boxes {
            draw_stub(edge, &placed, off_x, off_y, area, buf);
        }
    }

    // ── 3. Boxes zoom: draw line-art connectors along their assigned lanes, on top of
    //       the rooms drawn below them in step 2.
    let mut arrowheads: Vec<((i32, i32), &'static str, bool)> = Vec::new();
    if let Some((cols, rows)) = &axes {
        arrowheads = render_lane_connectors(&rm.plan, cols, rows, (off_x, off_y), area, buf);
    }

    if boxes {
        draw_portal_connectors(rm, &placed, off_x, off_y, area, buf);
    }

    // ── 4. Draw rooms on top of the line-art (translate + clip) ───────────────
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        let sx = vx + off_x;
        let sy = vy + off_y;
        draw_room(room, state, zoom, sx, sy, area, buf);
    }

    // Portal-icon overlay (Boxes zoom), drawn after the rooms so icons sit on the box. In
    // normal view the icons go on the interior right column; in portal view (show_portal_labels)
    // they move onto the border and the destination names float outside the box.
    if boxes {
        draw_portal_icons(rm, &placed, state, state.show_portal_labels, (off_x, off_y), area, buf);
    }

    // ── 5. Draw departure/arrival arrowheads LAST, so each embeds in the room ─
    //       border it sits on (replacing the box-edge glyph, pointing outward).
    // Portal view hides the cardinal connector arrowheads so only portal icons sit on borders.
    if !state.show_portal_labels {
        draw_connector_arrows(&arrowheads, (off_x, off_y), area, buf);
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
    // A merge stub may legitimately collapse to just centre→junction (2 points) — it still must
    // render its box-edge exit arrow and a short line to the junction. Every other connector needs
    // centre + interior + centre (3 points).
    if pix.len() < if conn.merge { 2 } else { 3 } {
        return None;
    }

    // The connector runs centre→…→centre. A line must not be drawn inside a room box, so
    // trim the two room centres. In their place, anchor each end on the box's edge cell for
    // that side (the doorway just outside the box), displaced along the edge by the slot, so
    // the line visibly touches both rooms even when the channel is wider than the lane it
    // runs in, and two connectors sharing a side land on distinct cells.
    let origin_cell = (conn.points[0].0.div_euclid(2), conn.points[0].1.div_euclid(2));
    let dep_anchor = if mapper::direction::is_diagonal(conn.exit_dir) {
        corner_anchor(cols, rows, origin_cell, conn.exit_dir)
    } else {
        box_edge_anchor(cols, rows, origin_cell, conn.exit, conn.exit_slot)
    };

    // The connector leaves the box straight out at 90° (a perpendicular stub on the anchor's own
    // row/col), then steps along the edge into the first interior channel point. Distinct slots
    // give distinct border cells; the straight connector on each side keeps slot 0 (centre), so a
    // displaced connector crosses it as a single clean ┼ instead of a corner stomp.
    let first_interior = pix[1];
    let dep_bridge = attach_bridge(dep_anchor, first_interior, conn.exit);

    let mut inner_v: Vec<(i32, i32)> = Vec::with_capacity(pix.len() + 6);
    inner_v.push(dep_anchor);
    inner_v.extend_from_slice(&dep_bridge);
    let arr_anchor = if conn.merge {
        // A merge stub ENDS ON the trunk at the junction (`pix.last()`), not at a destination box —
        // no arrival anchor or bridge; the line simply reaches the junction (a T-junction).
        inner_v.extend_from_slice(&pix[1..]);
        *pix.last().unwrap()
    } else {
        let last = conn.points[conn.points.len() - 1];
        let dest_cell = (last.0.div_euclid(2), last.1.div_euclid(2));
        let aa = match conn.entry_dir {
            Some(d) if mapper::direction::is_diagonal(d) => corner_anchor(cols, rows, dest_cell, d),
            _ => box_edge_anchor(cols, rows, dest_cell, conn.entry, conn.entry_slot),
        };
        let last_interior = pix[pix.len() - 2];
        let arr_bridge = attach_bridge(aa, last_interior, conn.entry);
        inner_v.extend_from_slice(&pix[1..pix.len() - 1]);
        for &p in arr_bridge.iter().rev() {
            inner_v.push(p);
        }
        inner_v.push(aa);
        aa
    };
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

        let dep_glyph = if mapper::direction::is_diagonal(conn.exit_dir) {
            diagonal_arrow(conn.exit_dir)
        } else {
            arrow_for_departure(conn.exit)
        };
        arrowheads.push((plot.dep_anchor, dep_glyph, conn.distorted));
        // Far-end arrow only for true reciprocal connectors (collapsed opposite pairs).
        if conn.reciprocal {
            let arr_glyph = match conn.entry_dir {
                Some(d) if mapper::direction::is_diagonal(d) => diagonal_arrow(d),
                _ => arrow_for_departure(conn.entry),
            };
            arrowheads.push((plot.arr_anchor, arr_glyph, conn.distorted));
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

// ── Portal badges ─────────────────────────────────────────────────────────────

/// Portal direction glyphs. Named so a font that renders a variant better is a one-line swap.
const PORTAL_UP: &str = "↑";
const PORTAL_DOWN: &str = "↓";
const PORTAL_IN: &str = "⊙";
const PORTAL_OUT: &str = "⊗";
const PORTAL_UNKNOWN: &str = "?";

/// Glyph for a non-planar (portal) direction. Shared by the map badge and the dump legend.
pub(crate) fn portal_glyph(dir: Direction) -> &'static str {
    match dir {
        Direction::Up => PORTAL_UP,
        Direction::Down => PORTAL_DOWN,
        Direction::In => PORTAL_IN,
        Direction::Out => PORTAL_OUT,
        _ => PORTAL_UNKNOWN,
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

/// Dotted-line glyphs for Up/Down portal connectors.
const DOTTED_V: char = '┊';
const DOTTED_H: char = '┄';

/// Draw Up/Down portal links that no compass connector already covers. When the two rooms are
/// directly grid-adjacent in the portal direction (a clean stack), draw one short dotted connector
/// between them. When the portal room was YIELDED far from its partner, a full connecting line would
/// run across the map and overwrite the paths in between — so instead draw a short dotted stub plus
/// the `↑`/`↓` glyph on EACH room's border: the start room points out in the portal direction, the
/// end room points back. Stubs are drawn before the rooms, so any stub cell that falls under an
/// adjacent box is harmlessly covered. A reciprocal Up/Down pair is handled once, from the Up side.
fn draw_portal_connectors(
    rm: &RenderMap,
    placed: &std::collections::HashMap<RoomId, VRect>,
    off_x: i32,
    off_y: i32,
    area: Rect,
    buf: &mut Buffer,
) {
    let style = Style::new().fg(Color::Cyan);
    let interiors: Vec<VRect> = placed.values().copied().collect();
    let in_interior = |x: i32, y: i32| {
        interiors
            .iter()
            .any(|r| x > r.x && x < r.x + BOX_W - 1 && y > r.y && y < r.y + BOX_H - 1)
    };
    let cell: std::collections::HashMap<RoomId, (i32, i32)> =
        rm.rooms.iter().map(|r| (r.id, r.cell)).collect();

    for edge in &rm.edges {
        if !edge.is_stub {
            continue;
        }
        let up = match edge.dir {
            Direction::Up => true,
            Direction::Down => false,
            _ => continue, // In/Out/Unknown get no dotted line
        };
        // A reciprocal Up/Down pair is handled once, from the Up side: skip the Down edge when a
        // matching Up edge (dest→Up→origin) exists.
        if !up
            && rm
                .edges
                .iter()
                .any(|e| e.dir == Direction::Up && e.origin == edge.dest && e.dest == edge.origin)
        {
            continue;
        }
        // Skip when a compass connector already joins the pair (either direction).
        let joined = rm.edges.iter().any(|e| {
            !e.is_stub
                && ((e.origin == edge.origin && e.dest == edge.dest)
                    || (e.origin == edge.dest && e.dest == edge.origin))
        });
        if joined {
            continue;
        }
        let (Some(&o), Some(&t)) = (placed.get(&edge.origin), placed.get(&edge.dest)) else {
            continue;
        };
        let (Some(&oc), Some(&dc)) = (cell.get(&edge.origin), cell.get(&edge.dest)) else {
            continue;
        };
        let dy = if up { -1 } else { 1 };
        if dc == (oc.0, oc.1 + dy) {
            // Cleanly stacked: one short dotted connector. Vertical-first L from the origin border
            // to the target's mid-row, clipped out of room interiors. Drawn on the right column
            // (BOX_W - 2), aligned with the in-room up/down portal arrow icons.
            let ocx = o.x + BOX_W - 2;
            let start_y = if up { o.y - 1 } else { o.y + BOX_H };
            let tcx = t.x + BOX_W - 2;
            let tcy = t.y + BOX_H / 2;
            for y in start_y.min(tcy)..=start_y.max(tcy) {
                if !in_interior(ocx, y) {
                    put_char(buf, ocx + off_x, y + off_y, DOTTED_V, style, area);
                }
            }
            for x in ocx.min(tcx)..=ocx.max(tcx) {
                if !in_interior(x, tcy) {
                    put_char(buf, x + off_x, tcy + off_y, DOTTED_H, style, area);
                }
            }
        } else {
            // Yielded: a stub + glyph on each room instead of a long, path-stomping line. The start
            // room points out in the portal direction; the end room points back the opposite way.
            portal_stub(buf, o, up, off_x, off_y, area, style);
            portal_stub(buf, t, !up, off_x, off_y, area, style);
        }
    }
}

/// Draw a one-cell dotted stub plus the `↑`/`↓` glyph just outside a room box — above it (`up`) or
/// below — marking a yielded Up/Down portal without drawing a full connecting line. The stub sits on
/// the box's right column (`BOX_W - 2`), aligned with the in-room `↑`/`↓` portal icons.
fn portal_stub(buf: &mut Buffer, b: VRect, up: bool, off_x: i32, off_y: i32, area: Rect, style: Style) {
    let cx = b.x + BOX_W - 2 + off_x;
    let (dot_y, tip_y, glyph) = if up {
        (b.y - 1, b.y - 2, PORTAL_UP)
    } else {
        (b.y + BOX_H, b.y + BOX_H + 1, PORTAL_DOWN)
    };
    put_char(buf, cx, dot_y + off_y, DOTTED_V, style, area);
    put_str(buf, cx, tip_y + off_y, glyph, style, area);
}

/// In-room icon slot for a portal direction: 0 = row 1 (Up), 1 = row 2 (mid: In/Out/Unknown),
/// 2 = row 3 (Down). Cardinal directions have no portal slot.
fn portal_slot(dir: Direction) -> Option<usize> {
    match dir {
        Direction::Up => Some(0),
        Direction::Down => Some(2),
        Direction::In | Direction::Out | Direction::Unknown => Some(1),
        _ => None,
    }
}

/// Mid-slot precedence when a room has several of In/Out/Unknown (lower wins): In ▸ Out ▸ Unknown.
fn mid_precedence(dir: Direction) -> u8 {
    match dir {
        Direction::In => 0,
        Direction::Out => 1,
        _ => 2, // Unknown
    }
}

/// One room's portal icon choices: three slots (Up / Mid / Down), each holding an optional
/// `(glyph, dest_label)` pair chosen with `mid_precedence` for the shared mid slot.
type PortalSlots<'a> = [Option<(&'a str, Option<&'a str>)>; 3];

/// Draw in-room portal indicators at Boxes zoom as a post-room overlay (so icons sit on top of
/// the box interior). Each room's portal (stub) edges map to a right-interior-column slot:
/// Up→row 1, In/Out/Unknown→row 2 (middle, by `mid_precedence`), Down→row 3. Default = the
/// direction glyph in that slot's far-right interior cell. When `show_labels` is set, the
/// portal's destination name is drawn right-aligned on that row with the icon pinned far-right.
/// In the default view an up-portal claims the upper-right corner, shifting the `●` notes marker
/// one cell left so both stay visible.
fn draw_portal_icons(
    rm: &RenderMap,
    placed: &std::collections::HashMap<RoomId, VRect>,
    state: &AppState,
    show_labels: bool,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    use std::collections::HashMap;
    let (off_x, off_y) = offset;
    // Per room, the chosen (glyph, dest_label) for each of the 3 slots; mid slot by precedence.
    let mut chosen: HashMap<RoomId, PortalSlots<'_>> = HashMap::new();
    let mut mid_rank: HashMap<RoomId, u8> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub {
            continue;
        }
        let Some(slot) = portal_slot(edge.dir) else { continue };
        let glyph = portal_glyph(edge.dir);
        let label = edge.dest_label.as_deref();
        let slots = chosen.entry(edge.origin).or_insert([None, None, None]);
        if slot == 1 {
            let rank = mid_precedence(edge.dir);
            let cur = mid_rank.entry(edge.origin).or_insert(u8::MAX);
            if rank < *cur {
                *cur = rank;
                slots[1] = Some((glyph, label));
            }
        } else if slots[slot].is_none() {
            slots[slot] = Some((glyph, label));
        }
    }

    let icon_col = BOX_W - 2; // far-right interior column (normal view)
    for room in &rm.rooms {
        let Some(slots) = chosen.get(&room.id) else { continue };
        let Some(&rect) = placed.get(&room.id) else { continue };
        let style = room_style(room, state);
        let (bx, by) = (rect.x, rect.y);
        if show_labels {
            // Portal view: icons move onto the border; destination names float OUTSIDE the box.
            if let Some((glyph, label)) = slots[0] {
                put_str(buf, bx + BOX_W / 2 + off_x, by + off_y, glyph, style, area); // top border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by - 1 + off_y, name, style, area); // above
                }
            }
            if let Some((glyph, label)) = slots[2] {
                put_str(buf, bx + BOX_W / 2 + off_x, by + BOX_H - 1 + off_y, glyph, style, area); // bottom border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by + BOX_H + off_y, name, style, area); // below
                }
            }
            if let Some((glyph, label)) = slots[1] {
                put_str(buf, bx + BOX_W - 1 + off_x, by + 2 + off_y, glyph, style, area); // right border
                // Unknown has no target semantics → glyph only, no floating name.
                if glyph != PORTAL_UNKNOWN {
                    if let Some(name) = label {
                        put_str(buf, bx + BOX_W + off_x, by + 2 + off_y, name, style, area); // right
                    }
                }
            }
        } else {
            // Normal view: directional icons in the interior right column.
            for (slot, cell) in slots.iter().enumerate() {
                let Some((glyph, _label)) = cell else { continue };
                let row = by + 1 + slot as i32;
                put_str(buf, bx + icon_col + off_x, row + off_y, glyph, style, area);
                if slot == 0 && room.has_notes {
                    put_char(buf, bx + icon_col - 1 + off_x, row + off_y, '●', style, area);
                }
            }
        }
    }
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
            draw_box_room(room, sx, sy, base_style, state.show_alignment, area, buf);
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

/// Word-wrap `s` into up to two lines no wider than `width` (break on spaces; a single
/// over-long word, or overflow past two lines, is truncated to `width`).
fn wrap_two(s: &str, width: usize) -> [String; 2] {
    let mut lines = [String::new(), String::new()];
    let mut idx = 0;
    for word in s.split_whitespace() {
        if idx >= 2 {
            break;
        }
        if lines[idx].is_empty() {
            lines[idx] = word.chars().take(width).collect();
        } else if lines[idx].chars().count() + 1 + word.chars().count() <= width {
            lines[idx].push(' ');
            lines[idx].push_str(word);
        } else {
            idx += 1;
            if idx < 2 {
                lines[idx] = word.chars().take(width).collect();
            }
        }
    }
    lines
}

/// Center `s` within `width` columns (truncated to `width` if longer).
fn center(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        return s.chars().take(width).collect();
    }
    let pad = width - len;
    let left = pad / 2;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(pad - left))
}

/// Draw a boxes (19×11 step) room: bordered box 11 wide × 5 tall.
///
/// Layout (11 cols × 5 rows, within a 19×11 step):
///   Row 0: ╭─────────╮  (or ┏━━━━━━━━━┓ for current room)
///   Row 1: │  name   │  (first word-wrap line, centered)
///   Row 2: │  name2  │  (second word-wrap line, centered)
///   Row 3: │  #id    │  (unique room id, centered; align code appended when enabled)
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
    show_alignment: bool,
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

    // Inner rows (h=5 → rows 1, 2, 3 are interior: 1=name wrap, 2=name wrap, 3=#id + align)
    for dy in 1..h - 1 {
        put_char(buf, sx, sy + dy, vert, style, area);
        // Fill interior with spaces (for background/style)
        for dx in 1..w - 1 {
            put_char(buf, sx + dx, sy + dy, ' ', style, area);
        }
        put_char(buf, sx + w - 1, sy + dy, vert, style, area);
    }

    // Room name word-wrapped + centered across the first two interior rows.
    let iw = (w - 2) as usize; // interior width (9)
    let name_lines = wrap_two(&room.label, iw);
    put_str(buf, sx + 1, sy + 1, &center(&name_lines[0], iw), style, area);
    put_str(buf, sx + 1, sy + 2, &center(&name_lines[1], iw), style, area);

    // Row 3: #id (centered), with alignment diagnostics appended when enabled.
    let mut row3 = format!("#{}", room.id);
    if show_alignment && !room.align_code.is_empty() {
        row3.push(' ');
        row3.push_str(&room.align_code);
    }
    put_str(buf, sx + 1, sy + 3, &center(&row3, iw), style, area);

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
    let mut expected = [ns, ew];
    expected.sort_unstable();
    let (mut illegal, mut crossings) = (0usize, 0usize);
    for per_conn in owners.values() {
        if per_conn.len() < 2 {
            continue;
        }
        // Merge junction: every connector meeting at this cell belongs to the SAME unordered room
        // pair (a trunk plus its merge stubs joining it). That is a legal T-junction, not an overlap.
        let same_pair = {
            let mut pairs = per_conn.keys().map(|&ci| {
                let c = &plan.connectors[ci];
                (c.origin.min(c.dest), c.origin.max(c.dest))
            });
            let first = pairs.next().unwrap();
            pairs.all(|p| p == first)
        };
        if same_pair {
            continue;
        }
        let mut masks: Vec<u8> = per_conn.values().copied().collect();
        masks.sort_unstable();
        if per_conn.len() == 2 && masks == expected {
            crossings += 1;
        } else {
            illegal += 1;
        }
    }
    (illegal, crossings)
}

/// Render `graph` and return its (illegal_overlaps, crossings).
pub(crate) fn render_overlap_stats(graph: &mapper::graph::MapGraph) -> (usize, usize) {
    let rm = mapper::render::render(graph);
    let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
    overlap_stats(&rm.plan, &cols, &rows)
}

/// Nudge rooms (bounded Chebyshev `radius`, ≤ `max_passes` passes) until the rendered
/// plan has zero illegal overlaps, secondarily fewer crossings. Deterministic, no overlap,
/// integer cells. Existing position is restored on every rejected trial.
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

        let room_ids: Vec<mapper::graph::RoomId> = graph
            .rooms()
            .filter(|r| r.pos.is_some())
            .map(|r| r.id)
            .collect();

        // Pick the single GLOBALLY best move this pass. Key (minimized):
        //   (resulting overlaps, exact-alignment the moved room loses, side-hints the moved room
        //    loses, resulting crossings, that room's compass degree, id, move index)
        // Overlaps first guarantees progress. Then `align_broken` — exact row/column alignment the
        // moved room would lose (reciprocal-chain-weighted): this protects a 2-room column/row chain
        // that the side-only `broken` cannot see, since `room_side_score` scores "below-and-west of
        // X" the same as "exactly below X". Then `broken` (side hints), crossings, and a low-degree /
        // low-id room as the final hint-neutral tiebreak.
        type Key = (usize, usize, usize, usize, usize, mapper::graph::RoomId, usize);
        let mut best: Option<(Key, mapper::graph::RoomId, (i32, i32))> = None;
        for &id in &room_ids {
            let Some(orig) = graph.room(id).and_then(|r| r.pos) else { continue };
            let score_orig = mapper::layout::room_side_score(graph, id);
            let align_orig = mapper::layout::room_alignment_score(graph, id);
            let degree = mapper::layout::room_compass_degree(graph, id);
            for (move_idx, &(dy, dx)) in moves.iter().enumerate() {
                let trial = (orig.0 + dx, orig.1 + dy);
                if graph.rooms().any(|r| r.id != id && r.pos == Some(trial)) {
                    continue;
                }
                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                let score_trial = mapper::layout::room_side_score(graph, id);
                let align_trial = mapper::layout::room_alignment_score(graph, id);
                graph.set_pos(id, orig); // restore; the winner is committed after the scan
                if (s.0, s.1) < (base.0, base.1) {
                    let align_broken = align_orig.saturating_sub(align_trial);
                    let broken = score_orig.saturating_sub(score_trial);
                    let key: Key = (s.0, align_broken, broken, s.1, degree, id, move_idx);
                    if best.as_ref().is_none_or(|(bk, _, _)| key < *bk) {
                        best = Some((key, id, trial));
                    }
                }
            }
        }

        match best {
            Some((_, id, trial)) => graph.set_pos(id, trial),
            None => break,
        }
    }
}

/// Nudge rooms to satisfy currently-VIOLATED directional hints — e.g. a one-way `W` edge whose dest
/// ended up east of its origin because a post-solve stage (contiguity ejection, collision spiral)
/// moved a room across it. Sibling to [`cleanup_overlaps`]; runs after it in the Retidy flow.
///
/// Greedy and bounded, like cleanup, but it OPTIMIZES `directional_hint_score` instead of overlaps:
/// each pass commits the single room move that most increases the total satisfied-hint count while
/// (a) not introducing any illegal connector overlap and (b) not breaking any exact row/column
/// alignment the moved room currently holds (`room_alignment_score`, so it never undoes the chain
/// alignment relayout/cleanup established). Only strict improvements are taken, so it converges.
pub(crate) fn repair_directional_hints(graph: &mut mapper::graph::MapGraph, radius: i32, max_passes: usize) {
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
        let base = render_overlap_stats(graph); // (illegal overlaps, crossings)
        let base_score = mapper::layout::directional_hint_score(graph);

        let room_ids: Vec<mapper::graph::RoomId> =
            graph.rooms().filter(|r| r.pos.is_some()).map(|r| r.id).collect();

        // Pick the single GLOBALLY best move this pass. Key (minimized):
        //   (Reverse(hint gain), resulting overlaps, resulting crossings, moved room's degree,
        //    id, move index)
        // Highest hint gain first; among equal gains, the move that leaves the fewest overlaps /
        // crossings and disturbs the lowest-degree room. A candidate is eligible only when it
        // STRICTLY raises the hint score, never raises illegal overlaps, and never lowers the moved
        // room's exact-alignment score (so it cannot knock a column/row chain apart).
        type Key = (std::cmp::Reverse<usize>, usize, usize, usize, mapper::graph::RoomId, usize);
        let mut best: Option<(Key, mapper::graph::RoomId, (i32, i32))> = None;
        for &id in &room_ids {
            let Some(orig) = graph.room(id).and_then(|r| r.pos) else { continue };
            let align_orig = mapper::layout::room_alignment_score(graph, id);
            let degree = mapper::layout::room_compass_degree(graph, id);
            for (move_idx, &(dy, dx)) in moves.iter().enumerate() {
                let trial = (orig.0 + dx, orig.1 + dy);
                if graph.rooms().any(|r| r.id != id && r.pos == Some(trial)) {
                    continue;
                }
                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                let score = mapper::layout::directional_hint_score(graph);
                let align_trial = mapper::layout::room_alignment_score(graph, id);
                graph.set_pos(id, orig); // restore; the winner is committed after the scan
                if score > base_score && s.0 <= base.0 && align_trial >= align_orig {
                    let gain = score - base_score;
                    let key: Key = (std::cmp::Reverse(gain), s.0, s.1, degree, id, move_idx);
                    if best.as_ref().is_none_or(|(bk, _, _)| key < *bk) {
                        best = Some((key, id, trial));
                    }
                }
            }
        }

        match best {
            Some((_, id, trial)) => graph.set_pos(id, trial),
            None => break,
        }
    }
}

/// Collapse the fully-empty interior rows and columns the tidy passes leave behind (e.g. a gap
/// opened when `repair_directional_hints` pushes a room out), shifting rooms together so the map
/// carries no wasted gap line. Runs last in the Retidy flow.
///
/// A collapse moves every room BEYOND the empty line one cell toward it, leaving the rest put. That
/// translates one half-plane uniformly, so every room keeps its relative order on both axes — all
/// directional and exact-alignment relationships survive — and no two rooms can share a cell. The
/// only thing a tighter layout can disturb is connector routing, so if the result raises illegal
/// overlaps the whole compaction is reverted (cosmetic tightening is never worth a new overlap).
pub(crate) fn compact_empty_lines(graph: &mut mapper::graph::MapGraph) {
    let before = render_overlap_stats(graph).0;
    let snapshot: Vec<(mapper::graph::RoomId, (i32, i32))> =
        graph.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();

    for is_x in [true, false] {
        loop {
            let coords: std::collections::BTreeSet<i32> = graph
                .rooms()
                .filter_map(|r| r.pos.map(|p| if is_x { p.0 } else { p.1 }))
                .collect();
            let (Some(&min), Some(&max)) = (coords.iter().next(), coords.iter().next_back()) else {
                break;
            };
            // Lowest empty interior line (strictly between the extremes); none → axis is dense.
            let Some(empty) = ((min + 1)..max).find(|c| !coords.contains(c)) else {
                break;
            };
            let rooms: Vec<(mapper::graph::RoomId, (i32, i32))> =
                graph.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
            for (id, p) in rooms {
                let c = if is_x { p.0 } else { p.1 };
                if c > empty {
                    graph.set_pos(id, if is_x { (p.0 - 1, p.1) } else { (p.0, p.1 - 1) });
                }
            }
        }
    }

    if render_overlap_stats(graph).0 > before {
        for (id, p) in snapshot {
            graph.set_pos(id, p);
        }
    }
}

/// Stack Up/Down rooms directly above/below their partner when it can be done without breaking a
/// chain. `grid_offset` returns None for Up/Down, so the solver gives those rooms no vertical
/// preference — they drift to wherever there is room. For each Up edge (`origin -Up-> dest`, so
/// `dest` is up = directly NORTH of `origin`) and each Down edge (`dest` directly SOUTH), if `dest`
/// is not already at the ideal cell, translate a closed set of rooms one step out to open it, then
/// place `dest` there.
///
/// The set is a TRANSITIVE closure so chains travel intact: seeded with the ideal column's occupants
/// on the ideal side, then closed under (a) E/W row-chain membership — a vertical shift would split a
/// row otherwise, so the whole row comes along — and (b) whatever sits in a shifting room's path. A
/// candidate commits only when no two rooms collide, no illegal overlap is added, and neither the
/// global side-hint nor the exact (chain) alignment score drops.
///
/// When a clean stack isn't possible, it still yields the room to the expected SIDE — an Up room
/// north of its partner, a Down room south — by relocating it (alone) to the nearest free, overlap-
/// and hint-safe cell on that side. Runs before compaction so vacated cells can be collapsed away.
pub(crate) fn stack_updown_rooms(graph: &mut mapper::graph::MapGraph) {
    use mapper::direction::Direction;
    use mapper::graph::RoomId;
    use std::collections::{BTreeMap, BTreeSet};

    let updown: Vec<(RoomId, RoomId, i32)> = graph
        .connections()
        .iter()
        .filter_map(|c| match c.dir {
            Direction::Up => Some((c.origin, c.dest, -1)),
            Direction::Down => Some((c.origin, c.dest, 1)),
            _ => None,
        })
        .collect();
    // Chains are topological (edge-derived), so identical for every candidate — compute once.
    let chains = mapper::layout::detect_chains(graph);

    for (origin, dest, dy) in updown {
        let Some(op) = graph.room(origin).and_then(|r| r.pos) else { continue };
        // Preferred target is directly in the portal direction; if that cell can't be opened, the two
        // diagonal-adjacent cells (NW/NE for Up, SW/SE for Down) still seat the room beside its
        // partner rather than flinging it across the map.
        let targets = [
            (op.0, op.1 + dy),     // directly N (Up) / S (Down)
            (op.0 - 1, op.1 + dy), // NW / SW
            (op.0 + 1, op.1 + dy), // NE / SE
        ];
        // Try directly N/S first, then the diagonals; the first that opens (or where the room
        // already sits) wins. try_stack_dest_at no-ops when dest is already at that cell, so a
        // diagonally-placed room still gets a chance to move to the preferred directly-in-line cell.
        if targets
            .iter()
            .any(|&t| try_stack_dest_at(graph, dest, origin, t, dy, &chains))
        {
            continue; // seated at the first openable adjacent cell
        }

        // None of the adjacent cells could be opened — yield, but keep the room on the expected SIDE
        // (Up -> north, Down -> south) at the nearest free, overlap- and hint-safe cell. If it is
        // already on that side, leave it.
        let center = targets[0];
        let pos: BTreeMap<RoomId, (i32, i32)> =
            graph.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
        let Some(dp0) = graph.room(dest).and_then(|r| r.pos) else { continue };
        let on_side = |c: (i32, i32)| if dy < 0 { c.1 < op.1 } else { c.1 > op.1 };
        if on_side(dp0) {
            continue;
        }
        let occupied: BTreeSet<(i32, i32)> =
            pos.iter().filter(|&(&id, _)| id != dest).map(|(_, &p)| p).collect();
        let base_ov = render_overlap_stats(graph).0;
        let base_side = mapper::layout::directional_hint_score(graph);
        let base_align = exact_alignment_count(graph);
        const YIELD_RADIUS: i32 = 10;
        let mut cands: Vec<(i32, i32)> = Vec::new();
        for ddy in -YIELD_RADIUS..=YIELD_RADIUS {
            for ddx in -YIELD_RADIUS..=YIELD_RADIUS {
                let c = (center.0 + ddx, center.1 + ddy);
                if on_side(c) && !occupied.contains(&c) {
                    cands.push(c);
                }
            }
        }
        cands.sort_unstable_by_key(|&(x, y)| {
            ((x - center.0).abs() + (y - center.1).abs(), (x - center.0).abs(), x, y)
        });
        for c in cands {
            graph.set_pos(dest, c);
            if render_overlap_stats(graph).0 <= base_ov
                && mapper::layout::directional_hint_score(graph) >= base_side
                && exact_alignment_count(graph) >= base_align
            {
                break; // committed at the nearest acceptable on-side cell
            }
            graph.set_pos(dest, dp0); // not acceptable -- restore and keep searching
        }
    }
}

/// Try to seat `dest` at `ideal` (a cell adjacent to its Up/Down `origin` partner) without breaking
/// a chain: first by shifting `ideal`'s column out to open it (chains travel whole), then -- if
/// `ideal` is free but a lone move would break `dest`'s own compass edges -- by cluster-dragging
/// `dest` with its movable, unanchored compass-edge cluster. Commits and returns true on success;
/// otherwise leaves the graph unchanged and returns false. `dy` is the portal direction (-1 Up /
/// +1 Down). Guards: no collision, no new illegal overlap, no side-hint loss, no exact-align loss.
fn try_stack_dest_at(
    graph: &mut mapper::graph::MapGraph,
    dest: mapper::graph::RoomId,
    origin: mapper::graph::RoomId,
    ideal: (i32, i32),
    dy: i32,
    chains: &mapper::layout::Chains,
) -> bool {
    use mapper::graph::RoomId;
    use std::collections::{BTreeMap, BTreeSet};

    let pos: BTreeMap<RoomId, (i32, i32)> =
        graph.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
    if pos.get(&dest) == Some(&ideal) {
        return true; // already seated at this cell — nothing to do
    }
    let at = |cell: (i32, i32)| pos.iter().find(|&(_, &p)| p == cell).map(|(&id, _)| id);
    let base_ov = render_overlap_stats(graph).0;
    let base_side = mapper::layout::directional_hint_score(graph);
    let base_align = exact_alignment_count(graph);
    let guarded = |g: &mapper::graph::MapGraph| {
        let cells: Vec<(i32, i32)> = g.rooms().filter_map(|r| r.pos).collect();
        let distinct = cells.iter().collect::<BTreeSet<_>>().len() == cells.len();
        distinct
            && render_overlap_stats(g).0 <= base_ov
            && mapper::layout::directional_hint_score(g) >= base_side
            && exact_alignment_count(g) >= base_align
    };

    // 1) Place `dest` at `ideal`. If the cell is occupied, first open it by shifting its column out,
    // closing the set so chains move whole (E/W row-chain mates of a shifted room, plus whatever
    // sits directly in a shifting room's path). If the cell is already free, no shift is needed —
    // the seed stays empty and we just drop `dest` in.
    let mut s: BTreeSet<RoomId> = if at(ideal).is_some() {
        pos.iter()
            .filter(|&(&id, &p)| {
                id != dest && p.0 == ideal.0 && if dy < 0 { p.1 <= ideal.1 } else { p.1 >= ideal.1 }
            })
            .map(|(&id, _)| id)
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut work: Vec<RoomId> = s.iter().copied().collect();
    while let Some(r) = work.pop() {
        if let Some(&cid) = chains.ew.get(&r) {
            for &m in &chains.ew_members[cid] {
                if m != dest && s.insert(m) {
                    work.push(m);
                }
            }
        }
        let rp = pos[&r];
        if let Some(q) = at((rp.0, rp.1 + dy)) {
            if q != dest && s.insert(q) {
                work.push(q);
            }
        }
    }
    // Skip the shift only when the closure swept in `dest` or the partner -- still try cluster-drag.
    if !s.contains(&dest) && !s.contains(&origin) {
        for &id in &s {
            let p = pos[&id];
            graph.set_pos(id, (p.0, p.1 + dy));
        }
        graph.set_pos(dest, ideal);
        if guarded(graph) {
            return true;
        }
        for (&id, &p) in &pos {
            graph.set_pos(id, p);
        }
    }

    // 2) Cluster-drag: `ideal` is free, but a lone move would break dest's own compass edges. Move
    // `dest` with its movable, unanchored compass-edge cluster by the same delta, preserving them.
    if at(ideal).is_none() {
        let delta = (ideal.0 - pos[&dest].0, ideal.1 - pos[&dest].1);
        const CLUSTER_LIMIT: usize = 4;
        let mut cluster: BTreeSet<RoomId> = BTreeSet::new();
        cluster.insert(dest);
        let mut wl = vec![dest];
        let mut bail = false;
        while let Some(r) = wl.pop() {
            for c in graph.connections() {
                if mapper::direction::grid_offset(c.dir).is_none() {
                    continue; // only true compass edges anchor a relative position
                }
                let other = if c.origin == r {
                    c.dest
                } else if c.dest == r {
                    c.origin
                } else {
                    continue;
                };
                if cluster.insert(other) {
                    // Stop if the cluster would pull in the partner, an anchored chain member, or
                    // grow too large -- those can't be freely translated.
                    if other == origin
                        || cluster.len() > CLUSTER_LIMIT
                        || chains.ew.contains_key(&other)
                        || chains.ns.contains_key(&other)
                    {
                        bail = true;
                    }
                    wl.push(other);
                }
            }
            if bail {
                break;
            }
        }
        let drag: Vec<(RoomId, (i32, i32))> = cluster
            .iter()
            .map(|&id| (id, (pos[&id].0 + delta.0, pos[&id].1 + delta.1)))
            .collect();
        let collide = drag
            .iter()
            .any(|&(_, np)| pos.iter().any(|(&oid, &op)| !cluster.contains(&oid) && op == np));
        if !bail && cluster.len() > 1 && !collide {
            for &(id, np) in &drag {
                graph.set_pos(id, np);
            }
            if guarded(graph) {
                return true;
            }
            for (&id, &p) in &pos {
                graph.set_pos(id, p);
            }
        }
    }

    false
}
/// Total compass connections whose geometry is EXACTLY satisfied (axis-aligned) — the chain-health
/// metric the Up/Down stacker must not reduce. Up/Down/Unknown edges are always "satisfied" (no
/// grid offset), so they contribute a constant and do not affect before/after deltas.
fn exact_alignment_count(graph: &mapper::graph::MapGraph) -> usize {
    graph
        .connections()
        .iter()
        .filter(|c| mapper::layout::edge_is_satisfied(graph, c))
        .count()
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
    fn cleanup_clears_overlaps_without_knocking_aligned_rooms_off_row() {
        // The A129 house: relayout aligns 74→25→26 on one row, but the rendered plan has
        // illegal overlaps so cleanup_overlaps must nudge SOMETHING. A hint-aware cleanup clears
        // the overlaps by moving a low-cost room (one whose hints are already distorted) instead
        // of knocking the aligned 74/25/26 run off its row.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16, 26, 27, 74, 75, 76, 77, 78, 79, 80, 81, 136, 143, 180, 193, 201, 203, 239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180, N, 81), (81, W, 180), (180, W, 78), (78, N, 143), (143, E, 77), (77, S, 74), (74, S, 76),
            (76, W, 78), (143, W, 78), (78, S, 76), (76, N, 74), (74, E, 25), (25, W, 76), (74, W, 79), (79, E, 74),
            (25, E, 26), (26, Up, 25), (78, E, 75), (77, E, 239), (239, N, 77), (77, Unknown, 180), (180, S, 80),
            (80, W, 180), (80, E, 79), (79, S, 80), (79, N, 81), (81, E, 79), (80, S, 76), (76, Unknown, 180),
            (79, Unknown, 180), (75, S, 81), (75, W, 78), (75, E, 77), (239, S, 77), (77, W, 75), (75, N, 143),
            (143, S, 75), (26, Down, 27), (27, N, 136), (136, SW, 27), (27, Up, 26), (26, Unknown, 180),
            (79, W, 203), (203, W, 193), (193, E, 203), (203, E, 79), (203, Up, 201), (201, Down, 203),
        ] {
            g.add_edge(o, d, dst);
        }
        mapper::layout::relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        // Overlaps cleared.
        assert_eq!(render_overlap_stats(&g).0, 0, "cleanup must clear all illegal overlaps");
        // The 74→E→25→E→26 run stays on one row.
        let p = |id: u16| g.room(id).unwrap().pos.unwrap();
        assert_eq!(p(74).1, p(25).1, "74 and 25 must stay on one row: 74={:?} 25={:?}", p(74), p(25));
        assert_eq!(p(25).1, p(26).1, "25 and 26 must stay on one row: 25={:?} 26={:?}", p(25), p(26));
        assert!(p(25).0 > p(74).0 && p(26).0 > p(25).0, "row order 74<25<26 in x");
    }

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

        // The unique id "#7" is drawn centered on row 3 (moved off row 2).
        let row3: String = (1u16..=9)
            .map(|x| buf.cell((x, 3)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(row3.contains("#7"), "row 3 should show the room id '#7'; got '{row3}'");
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
    fn merge_stub_keeps_exit_arrow_when_polyline_collapses() {
        // Vertical N/S reciprocal trunk (A above B) plus an extra A→E→B edge: the E stub's
        // polyline can collapse to 2 points, but it must STILL render A's east exit arrow ▶
        // (regression: the < 3 guard dropped the whole connector, arrow included).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 2)); // B directly south of A
        g.add_edge(1, Direction::S, 2);
        g.add_edge(2, Direction::N, 1); // vertical trunk
        g.add_edge(1, Direction::E, 2); // extra same-pair edge → merge stub
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0, "no illegal overlap");
        let rm = mapper::render::render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let right = buf.content.iter().filter(|c| c.symbol() == "▶").count();
        assert!(right >= 1, "the extra E edge's box-edge exit arrow ▶ must still render");
    }

    #[test]
    fn adjacent_merge_renders_both_stub_lines_regardless_of_edge_order() {
        // Regression: #77 and #239 are ADJACENT (align=row) with a reciprocal E/W trunk plus extra
        // N and S edges. When the reverse edges are added so the geometric opposite (W) is NOT first,
        // the reciprocal pairing must still pick W so the trunk stays straight — otherwise the trunk
        // bends up-and-over and the S/W stubs collapse to degenerate zero-width lines (the "missing
        // southern path" bug). Both stub T-junctions and both south/north connecting lines must draw.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(77, "Forest".into());
        g.upsert_room(239, "Forest".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (1, 0)); // adjacent, no gap
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        g.add_edge(239, Direction::W, 77); // the geometric opposite, added LAST on purpose
        let rm = mapper::render::render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let count = |sym: &str| buf.content.iter().filter(|c| c.symbol() == sym).count();
        assert!(count("▲") >= 1, "239's N exit arrow ▲ present");
        assert!(count("▼") >= 1, "239's S exit arrow ▼ present");
        // Two distinct T-junctions where the N and S stubs join the straight trunk (┴ above, ┬ below).
        assert!(count("┴") >= 1 && count("┬") >= 1, "both N (┴) and S (┬) stubs join the trunk");
        // The southern stub's connecting line actually reaches below the boxes (└…┘ turn corners),
        // proving the S path is drawn and not collapsed to the bare arrow.
        assert!(count("└") >= 1 && count("┘") >= 1, "the south stub routes a visible line below #239");
    }

    #[test]
    fn multi_edge_merge_one_trunk_arrows_and_tjunction() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(77, "F".into());
        g.upsert_room(239, "G".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (2, 0)); // east of 77, a gap of one cell
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::W, 77); // reciprocal trunk
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        // The merge junction (trunk + its stubs) is exempted → no illegal overlaps.
        let (illegal, _) = render_overlap_stats(&g);
        assert_eq!(illegal, 0, "a same-pair merge junction must not count as an illegal overlap");
        // Render: #239 still shows its N (▲) and S (▼) box-edge exit arrows, and a T-junction
        // glyph appears where the stubs join the trunk.
        let rm = mapper::render::render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let count = |sym: &str| buf.content.iter().filter(|c| c.symbol() == sym).count();
        assert!(count("▲") >= 1, "239's N exit arrow ▲ present");
        assert!(count("▼") >= 1, "239's S exit arrow ▼ present");
        let tjuncts: usize = ["├", "┤", "┬", "┴", "┼"].iter().map(|s| count(s)).sum();
        assert!(tjuncts >= 1, "a T-junction glyph where the merge stubs join the trunk");
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
    fn cleanup_keeps_two_room_column_chain_aligned() {
        // Regression: relayout aligns the reciprocal N/S chain 74<->76 into one column (76 directly
        // below 74). The rendered plan has one illegal overlap, so cleanup_overlaps must nudge
        // SOMETHING — but it must NOT knock #76 off #74's column to do it (the "76 not below 74"
        // bug). The side-only hint score saw that move as free; the exact-alignment term forbids it.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert_eq!(p(&g,74).0, p(&g,76).0, "precondition: relayout column-aligns 74 and 76");
        cleanup_overlaps(&mut g, 3, 40);
        assert_eq!(render_overlap_stats(&g).0, 0, "cleanup still clears all illegal overlaps");
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "76 must stay directly below 74 after cleanup: 74={:?} 76={:?}", p(&g,74), p(&g,76));
        assert!(p(&g,76).1 > p(&g,74).1, "76 stays south of 74");
    }

    #[test]
    fn repair_puts_78_west_of_180_after_retidy() {
        // The full Retidy flow (relayout -> cleanup_overlaps -> repair_directional_hints) on A129.
        // The stress solver places 78 west of 180, but contiguity ejection shoves 180 across 78;
        // the directional repair pass must recover 180->W->78 (78 ends up west of 180) without
        // re-introducing overlaps or knocking 76 off 74's column.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert!(p(&g,78).0 > p(&g,180).0, "precondition: contiguity left 78 EAST of 180 (the bug)");
        repair_directional_hints(&mut g, 3, 40);
        assert!(p(&g,78).0 < p(&g,180).0,
            "repair must place 78 west of 180: 78={:?} 180={:?}", p(&g,78), p(&g,180));
        assert_eq!(render_overlap_stats(&g).0, 0, "repair must not introduce illegal overlaps");
        assert_eq!(p(&g,74).0, p(&g,76).0, "repair must not knock 76 off 74's column");
    }

    #[test]
    fn yielded_portal_draws_stubs_not_a_long_line() {
        // Up/Down pair placed far apart (yielded). Instead of a long dotted L that overwrites the
        // paths in between, each room gets a short dotted stub + ↑/↓ glyph: no horizontal dotted run
        // and only a couple of vertical dotted cells, not a spanning line.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (3, 2)); // far from (0,-1) — yielded, not stacked
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        let rm = render(&g);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let count = |s: &str| buf.content.iter().filter(|c| c.symbol() == s).count();
        assert_eq!(count("┄"), 0, "no horizontal dotted run — the long L-line is gone");
        assert!(count("┊") <= 4, "only short vertical stubs, not a spanning line: {}", count("┊"));
        assert!(count("┊") >= 1, "each yielded room gets a dotted stub");
        assert!(count("↑") >= 1, "up glyph present on a stub/icon");
        assert!(count("↓") >= 1, "down glyph present on a stub/icon");
    }

    #[test]
    fn stack_updown_pushes_free_occupant_to_place_up_room_above_partner() {
        // A (down) at (0,0); its up-room B is parked below at (0,2); a constraint-free room X sits
        // in the ideal cell (0,-1). The stacker pushes X up and puts B directly above A.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));   // A (down partner)
        g.set_pos(2, (0, 2));   // B (up room) parked below
        g.set_pos(3, (0, -1));  // X occupies the ideal cell, no compass edges
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        stack_updown_rooms(&mut g);
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)), "B stacked directly above A");
        assert_eq!(g.room(3).unwrap().pos, Some((0, -2)), "free occupant X pushed one further up");
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)), "A (the partner) does not move");
    }

    #[test]
    fn stack_updown_shifts_whole_row_chain_together() {
        // A (down) at (0,0); B (up) below at (0,2). The ideal cell (0,-1) is held by C, which is in a
        // reciprocal E/W chain with D (same row). The coordinated shift moves C AND D up together so
        // the row stays intact, opening (0,-1) for B.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 2));
        g.set_pos(3, (0, -1));  // C in ideal cell
        g.set_pos(4, (1, -1));  // D, east of C on the same row (chain mate)
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3); // reciprocal E/W chain C<->D
        stack_updown_rooms(&mut g);
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)), "B stacked directly above A");
        assert_eq!(g.room(3).unwrap().pos, Some((0, -2)), "C shifted up");
        assert_eq!(g.room(4).unwrap().pos, Some((1, -2)), "D shifted up WITH C — row stays intact");
        assert_eq!(g.room(3).unwrap().pos.unwrap().1, g.room(4).unwrap().pos.unwrap().1, "C,D one row");
    }

    #[test]
    fn stack_updown_uses_diagonal_when_directly_in_line_is_blocked() {
        // P (up partner) at (0,0). Directly north (0,-1) is X, which has a one-way exact E edge to Z
        // — shifting X up would break X->E->Z, so the directly-north cell can't be opened. NW (-1,-1)
        // is free, so the up room U seats there (diagonally adjacent) instead of yielding far away.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));   // P (partner)
        g.set_pos(2, (0, -1));  // X blocks directly-north
        g.set_pos(3, (1, -1));  // Z east of X (one-way exact → X can't shift up)
        g.set_pos(4, (0, 2));   // U (up room) parked far south
        g.add_edge(1, Direction::Up, 4);
        g.add_edge(4, Direction::Down, 1);
        g.add_edge(2, Direction::E, 3); // one-way
        stack_updown_rooms(&mut g);
        let p = |id: u16| g.room(id).unwrap().pos.unwrap();
        assert_eq!(p(2), (0, -1), "X (blocker) not pushed");
        assert!(p(4).1 < p(1).1, "U is north of P: U={:?} P={:?}", p(4), p(1));
        assert!((p(4).0 - p(1).0).abs() <= 1 && (p(4).1 - p(1).1).abs() <= 1,
            "U lands diagonally adjacent to P: U={:?} P={:?}", p(4), p(1));
        assert_ne!(p(4), (0, -1), "U did not steal X's cell");
    }

    #[test]
    fn stack_updown_cluster_drag_moves_leaf_partner_to_stack_in_line() {
        // A (up partner) at (0,0). B is DOWN from A, so it should sit at (0,1). But B is tied to a
        // leaf C by a diagonal (B is SW of C), and C shares A's column (x=0) — so B directly below A
        // would be due-south of C, breaking the SW. The cluster-drag moves {B,C} east together: B
        // lands directly below A and B stays SW of C.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));   // A
        g.set_pos(2, (-1, 1));  // B (down room): south of A but one west, to stay SW of C
        g.set_pos(3, (0, -2));  // C (leaf), same column as A
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        g.add_edge(3, Direction::SW, 2); // B is SW of C
        stack_updown_rooms(&mut g);
        let p = |id: u16| g.room(id).unwrap().pos.unwrap();
        assert_eq!(p(2), (0, 1), "B stacked directly below A");
        assert!(p(2).0 < p(3).0 && p(2).1 > p(3).1, "B stays SW of C: B={:?} C={:?}", p(2), p(3));
        assert_eq!(p(3), (1, -2), "leaf C dragged east to keep the SW link");
    }

    #[test]
    fn stack_updown_yields_up_room_northward_when_cannot_stack() {
        // A (down) at (0,0); B (up) parked SOUTH at (0,2). The ideal cell (0,-1) is held by C, which
        // has a ONE-WAY exact E edge to F — pushing C up would break C->E->F with nothing to fix it,
        // so no clean stack exists. The stacker must NOT push C, and must yield B to the NORTH side
        // of A (an Up room belongs north of its partner) rather than leaving it parked south.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 2));   // B starts SOUTH of A (wrong side)
        g.set_pos(3, (0, -1));  // C
        g.set_pos(4, (1, -1));  // F, east of C — exact, but one-way (no reciprocal → not a chain)
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(3, Direction::E, 4); // one-way only
        stack_updown_rooms(&mut g);
        assert_eq!(g.room(3).unwrap().pos, Some((0, -1)), "C is NOT pushed (would break C->E->F)");
        assert!(g.room(2).unwrap().pos.unwrap().1 < g.room(1).unwrap().pos.unwrap().1,
            "B (up room) yielded to the NORTH side of A: B={:?} A={:?}",
            g.room(2).unwrap().pos, g.room(1).unwrap().pos);
    }

    #[test]
    fn stack_updown_yields_down_room_southward() {
        // A (up partner) at (0,0); B is DOWN from A and parked NORTH at (0,-3) (wrong side) with the
        // ideal cell (0,1) blocked by a chain that can't move. B must yield to the SOUTH of A.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -3));  // B (down room) parked NORTH (wrong side)
        g.set_pos(3, (0, 1));   // C blocks the ideal cell below A
        g.set_pos(4, (1, 1));   // F east of C, one-way exact → C can't be pushed
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(3, Direction::E, 4);
        stack_updown_rooms(&mut g);
        assert!(g.room(2).unwrap().pos.unwrap().1 > g.room(1).unwrap().pos.unwrap().1,
            "B (down room) yielded to the SOUTH side of A: B={:?} A={:?}",
            g.room(2).unwrap().pos, g.room(1).unwrap().pos);
    }

    #[test]
    fn compact_collapses_empty_interior_column_and_row() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0)); // empty column at x=1
        g.set_pos(3, (0, 2)); // empty row at y=1
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::S, 3);
        compact_empty_lines(&mut g);
        // Column 1 and row 1 collapse: 2 moves to (1,0), 3 moves to (0,1). Order preserved.
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)));
        assert_eq!(g.room(2).unwrap().pos, Some((1, 0)), "empty column collapsed");
        assert_eq!(g.room(3).unwrap().pos, Some((0, 1)), "empty row collapsed");
        // No empty interior line remains.
        let xs: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().0).collect();
        let ys: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().1).collect();
        assert!((*xs.iter().next().unwrap()..*xs.iter().next_back().unwrap()).all(|x| xs.contains(&x)));
        assert!((*ys.iter().next().unwrap()..*ys.iter().next_back().unwrap()).all(|y| ys.contains(&y)));
    }

    #[test]
    fn compact_preserves_directional_order_no_overlap() {
        // Full A129 Retidy flow plus compaction: 78 stays west of 180, 76 stays under 74, overlaps
        // stay clear, and no fully-empty interior column/row is left behind.
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
            (239,W,77),(81,N,75),(25,Down,26),
        ] { g.add_edge(o, d, dst); }
        mapper::layout::relayout_auto(&mut g);
        cleanup_overlaps(&mut g, 3, 40);
        repair_directional_hints(&mut g, 3, 40);
        compact_empty_lines(&mut g);
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert!(p(&g,78).0 < p(&g,180).0, "78 stays west of 180 through compaction");
        assert_eq!(p(&g,74).0, p(&g,76).0, "76 stays under 74 through compaction");
        assert_eq!(render_overlap_stats(&g).0, 0, "compaction keeps overlaps clear");
        let xs: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().0).collect();
        let ys: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().1).collect();
        assert!((*xs.iter().next().unwrap()..*xs.iter().next_back().unwrap()).all(|x| xs.contains(&x)),
            "no empty interior column remains: {xs:?}");
        assert!((*ys.iter().next().unwrap()..*ys.iter().next_back().unwrap()).all(|y| ys.contains(&y)),
            "no empty interior row remains: {ys:?}");
    }

    #[test]
    fn repair_directional_hints_is_deterministic() {
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
        repair_directional_hints(&mut g1, 3, 40);
        repair_directional_hints(&mut g2, 3, 40);
        let p1: Vec<_> = g1.rooms().map(|r| (r.id, r.pos)).collect();
        let p2: Vec<_> = g2.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(p1, p2, "repair must be deterministic");
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

    #[test]
    fn box_name_wraps_centered_and_id_on_row3() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(7, "Rocky Ledge".into());
        g.set_pos(7, (0, 0));
        let rm = render(&g);
        let state = AppState::default(); // Boxes, align off
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let row = |y: u16| -> String {
            (0..11u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default()).collect()
        };
        // Name word-wraps across rows 1 and 2.
        assert!(row(1).contains("Rocky"), "row 1 has the first word: '{}'", row(1));
        assert!(row(2).contains("Ledge"), "row 2 has the second word: '{}'", row(2));
        // #id is on row 3 (moved off row 2).
        assert!(row(3).contains("#7"), "row 3 shows the id: '{}'", row(3));
        assert!(!row(2).contains("#7"), "id is no longer on row 2: '{}'", row(2));
        // Centered: a leading pad space after the left border on the name + id rows.
        assert!(row(1).starts_with("│ "), "name centered (leading pad): '{}'", row(1));
        assert!(row(3).starts_with("│ "), "id centered (leading pad): '{}'", row(3));
    }

    #[test]
    fn alignment_overlay_off_by_default_then_shows_code() {
        use mapper::graph::MapGraph;
        use mapper::layout::relayout_auto;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal → row chain
        relayout_auto(&mut g);
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 160, 60);
        let render_buf = |show: bool| {
            let mut st = AppState::default();
            st.zoom = Zoom::Boxes;
            st.scroll = rm.bounds.0;
            st.show_alignment = show;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf
        };
        let off = render_buf(false);
        let on = render_buf(true);
        assert_ne!(format!("{off:?}"), format!("{on:?}"), "overlay changes the buffer when on");
        // an 'R' appears somewhere only when on
        let has_r = |b: &Buffer| (0..area.width).any(|x| (0..area.height).any(|y|
            b.cell((x, y)).map(|c| c.symbol() == "R").unwrap_or(false)));
        assert!(!has_r(&off));
        assert!(has_r(&on), "row-chain code R appears when overlay on");
    }

    #[test]
    fn portal_glyphs_map_directions() {
        assert_eq!(portal_glyph(Direction::Up), "↑");
        assert_eq!(portal_glyph(Direction::Down), "↓");
        assert_eq!(portal_glyph(Direction::In), "⊙");
        assert_eq!(portal_glyph(Direction::Out), "⊗");
        assert_eq!(portal_glyph(Direction::Unknown), "?");
    }

    #[test]
    fn portal_icons_render_in_room_slots() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.upsert_room(3, "Cellar".into());
        g.upsert_room(4, "Vault".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // placed portal targets (route_all skips unplaced dests)
        g.set_pos(3, (0, 1));
        g.set_pos(4, (1, 0));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(1, Direction::In, 4);
        let rm = render(&g);
        let state = AppState::default(); // Boxes, scroll (0,0), labels off
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Box of room 1 is at screen (0,0); right interior column is col 9 (BOX_W-2).
        assert_eq!(sym(9, 1), "↑", "up icon in upper-right interior (row 1)");
        assert_eq!(sym(9, 2), "⊙", "in icon in middle-right interior (row 2)");
        assert_eq!(sym(9, 3), "↓", "down icon in lower-right interior (row 3)");
    }

    #[test]
    fn portal_mid_slot_in_beats_out() {
        // Room 1 has BOTH an In portal (→ room 2) and an Out portal (→ room 3).
        // The mid-slot precedence rule is In ▸ Out ▸ Unknown, so the middle-right interior
        // cell (col 9, row 2 of a box at screen (0,0)) must show ⊙ (In), not ⊗ (Out).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Inner".into());
        g.upsert_room(3, "Outer".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // placed so route_all processes this edge
        g.set_pos(3, (2, 0)); // placed so route_all processes this edge
        g.add_edge(1, Direction::In, 2);
        g.add_edge(1, Direction::Out, 3);
        let rm = render(&g);
        let state = AppState::default(); // Boxes zoom, scroll (0,0), labels off
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // col 9 = BOX_W - 2 = 11 - 2 = 9; row 2 = mid slot
        assert_eq!(sym(9, 2), "⊙", "In beats Out in mid slot: expected ⊙, got '{}'", sym(9, 2));
    }

    #[test]
    fn portal_icon_up_shifts_notes_marker() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.set_notes(1, "stuff".into());
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(sym(9, 1), "↑", "up icon claims the upper-right corner");
        assert_eq!(sym(8, 1), "●", "notes marker shifts one cell left of the up icon");
    }

    #[test]
    fn portal_view_moves_icons_to_border_and_floats_destinations() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Mid".into());    // portal owner
        g.upsert_room(2, "Attic".into());  // up target
        g.upsert_room(3, "Cellar".into()); // down target
        g.set_pos(1, (0, 1));
        g.set_pos(2, (0, 0));
        g.set_pos(3, (0, 2));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::Down, 3);
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.show_portal_labels = true;
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let bx = cols.room_pixel(0) - off.0;
        let by = rows.room_pixel(1) - off.1;
        let sym = |x: i32, y: i32| buf.cell((x as u16, y as u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Icons sit on the border (top/bottom centre), not the interior right column.
        assert_eq!(sym(bx + BOX_W / 2, by), "↑", "up icon on the top border centre");
        assert_eq!(sym(bx + BOX_W / 2, by + BOX_H - 1), "↓", "down icon on the bottom border centre");
        // Destinations float above / below the box.
        let above: String = (0..area.width).map(|x| sym(x as i32, by - 1)).collect();
        let below: String = (0..area.width).map(|x| sym(x as i32, by + BOX_H)).collect();
        assert!(above.contains("Attic"), "up destination floats above; got '{above}'");
        assert!(below.contains("Cellar"), "down destination floats below; got '{below}'");
        // The interior right-column icon is gone in portal view.
        assert_ne!(sym(bx + BOX_W - 2, by + 1), "↑", "icons leave the interior in portal view");
    }

    #[test]
    fn unknown_portal_in_portal_view_is_border_glyph_no_name() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "West of House".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Unknown, 2);
        let rm = render(&g);
        let mut state = AppState::default();
        state.show_portal_labels = true;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // ? sits on the RIGHT border (col BOX_W-1) at the middle row (row 2). Box is at (0,0).
        assert_eq!(sym((BOX_W - 1) as u16, 2), "?", "unknown portal shows ? on the right border");
        // No destination name to the right of the box on that row.
        let right: String = ((BOX_W as u16)..40).map(|x| sym(x, 2)).collect();
        assert!(!right.contains("West"), "unknown portal shows no destination name; got '{right}'");
    }

    #[test]
    fn diagonal_edge_draws_corner_arrow() {
        // 1 →SW→ 2 (room 2 south-west of room 1): ↙ replaces room 1's bottom-left corner.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let bx = cols.room_pixel(1) - off.0; // room 1 at col 1
        let by = rows.room_pixel(0) - off.1; // room 1 at row 0
        let sym = buf
            .cell((bx as u16, (by + BOX_H - 1) as u16))
            .map(|c| c.symbol().to_string())
            .unwrap_or_default();
        assert_eq!(sym, "↙", "SW edge draws ↙ at room 1's bottom-left corner");
    }

    #[test]
    fn reciprocal_diagonal_draws_corner_arrow_at_both_ends() {
        // 1 →SW→ 2 and 2 →NE→ 1 (true reciprocal): ↙ at room 1's bottom-left corner and
        // ↗ at room 2's top-right corner (the far end uses the back-edge direction).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        g.add_edge(2, Direction::NE, 1); // reciprocal
        let rm = render(&g);
        let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let off = (cols.room_pixel(rm.bounds.0 .0), rows.room_pixel(rm.bounds.0 .1));
        let sym = |x: i32, y: i32| buf.cell((x as u16, y as u16)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Origin (room 1): SW → bottom-left corner.
        let bx1 = cols.room_pixel(1) - off.0;
        let by1 = rows.room_pixel(0) - off.1;
        assert_eq!(sym(bx1, by1 + BOX_H - 1), "↙", "origin SW corner arrow");
        // Far end (room 2): NE back-edge → top-right corner.
        let bx2 = cols.room_pixel(0) - off.0;
        let by2 = rows.room_pixel(1) - off.1;
        assert_eq!(sym(bx2 + BOX_W - 1, by2), "↗", "far-end NE corner arrow");
    }

    #[test]
    fn portal_view_suppresses_connector_arrows() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = render(&g);
        let area = Rect::new(0, 0, 80, 30);
        let count_arrows = |show: bool| -> usize {
            let mut st = AppState::default();
            st.show_portal_labels = show;
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf.content.iter().filter(|c| matches!(c.symbol(), "▶" | "◀" | "▲" | "▼")).count()
        };
        assert!(count_arrows(false) > 0, "normal view draws connector arrowheads");
        assert_eq!(count_arrows(true), 0, "portal view suppresses connector arrowheads");
    }

    #[test]
    fn up_portal_draws_dotted_connector_when_no_compass_edge() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (0, 0)); // NW of room 1
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let has_dotted = buf.content.iter().any(|c| matches!(c.symbol(), "┊" | "┄"));
        assert!(has_dotted, "an Up portal with no compass edge draws a dotted connector");
    }

    #[test]
    fn up_portal_no_dotted_connector_when_compass_edge_joins_pair() {
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0)); // due north of room 1
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::N, 2); // a compass connector already joins the pair
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let has_dotted = buf.content.iter().any(|c| matches!(c.symbol(), "┊" | "┄"));
        assert!(!has_dotted, "no dotted line when a compass edge already joins the pair");
    }

}
