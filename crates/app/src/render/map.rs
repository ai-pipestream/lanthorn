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
use crate::symbols::{BoxStyle, SymbolSet};

// ── Pulsing border ────────────────────────────────────────────────────────────

/// Pulse frequency in Hz (cycles per second) for the background-tidy border animation.
pub const PULSE_HZ: f64 = 1.0;
/// Red endpoint of the pulse (220, 60, 60).
const PULSE_RED: (u8, u8, u8) = (220, 60, 60);
/// Green endpoint of the pulse (60, 200, 90).
const PULSE_GREEN: (u8, u8, u8) = (60, 200, 90);

/// Compute the pulsed map-border color for a given elapsed time since job spawn.
///
/// The color oscillates between `PULSE_RED` and `PULSE_GREEN` at `PULSE_HZ` Hz
/// using a sine-based lerp:
///   f = (sin(t * TAU * PULSE_HZ) + 1) / 2  →  [0, 1]
///
/// At `elapsed = 0` (phase 0, sin = 0) the result is the midpoint.
/// At quarter-period (sin = 1, f = 1) the result is the green endpoint.
/// At three-quarter-period (sin = -1, f = 0) the result is the red endpoint.
///
/// Called only when a tidy job is in flight; the caller picks `normal` when idle.
pub fn pulse_border_color(elapsed: std::time::Duration) -> Color {
    let t = elapsed.as_secs_f64();
    let f = ((t * std::f64::consts::TAU * PULSE_HZ).sin() + 1.0) / 2.0;
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    Color::Rgb(
        lerp(PULSE_RED.0, PULSE_GREEN.0),
        lerp(PULSE_RED.1, PULSE_GREEN.1),
        lerp(PULSE_RED.2, PULSE_GREEN.2),
    )
}

/// Duration of the one-shot story-border flash for a `sound_effect` bleep.
pub const SOUND_PULSE_MS: u64 = 500;

/// Extract RGB channels from a `Color`, or `None` for non-RGB colors
/// (named/indexed/Reset have no fixed RGB to interpolate toward).
fn rgb_of(c: Color) -> Option<(u8, u8, u8)> {
    if let Color::Rgb(r, g, b) = c {
        Some((r, g, b))
    } else {
        None
    }
}

/// One-shot fade for a sound bleep: full `beep` color at `elapsed == 0`, lerping
/// toward `normal` as `elapsed` approaches `SOUND_PULSE_MS`. Returns `None` once
/// the window has elapsed (the caller then clears the pulse and the border
/// renders normally). When `normal` is not an RGB color (e.g. a terminal/named
/// border color), fade toward a dimmed copy of the beep color instead.
pub fn sound_pulse_color(
    beep: Color,
    normal: Color,
    elapsed: std::time::Duration,
) -> Option<Color> {
    let ms = elapsed.as_millis() as u64;
    if ms >= SOUND_PULSE_MS {
        return None;
    }
    let (br, bg, bb) = rgb_of(beep).unwrap_or((255, 180, 40));
    let (nr, ng, nb) = rgb_of(normal).unwrap_or((br / 4, bg / 4, bb / 4));
    let f = ms as f64 / SOUND_PULSE_MS as f64; // 0.0 -> 1.0 across the window
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    Some(Color::Rgb(lerp(br, nr), lerp(bg, ng), lerp(bb, nb)))
}

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
pub(crate) const MIN_GUTTER: i32 = 2;
/// Boxes-zoom box size (matches `zoom_box_size(Zoom::Boxes)`), in cells.
pub(crate) const BOX_W: i32 = 11;
pub(crate) const BOX_H: i32 = 5;

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


/// Arrow glyph for a diagonal departure/arrival (caller guards with `is_diagonal`).
fn diagonal_arrow(dir: Direction, arrows: &crate::symbols::Arrows) -> char {
    match dir {
        Direction::NE => arrows.ne,
        Direction::NW => arrows.nw,
        Direction::SE => arrows.se,
        Direction::SW => arrows.sw,
        _ => arrows.ne, // unreachable when guarded by is_diagonal
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
fn arrow_for_departure(dep_side: Side, arrows: &crate::symbols::Arrows) -> char {
    match dep_side {
        Side::Right  => arrows.east,
        Side::Left   => arrows.west,
        Side::Top    => arrows.north,
        Side::Bottom => arrows.south,
    }
}

/// True if screen cell `(sx, sy)` lies inside `area`.
fn in_area(sx: i32, sy: i32, area: Rect) -> bool {
    sx >= area.x as i32 && sx < area.right() as i32 && sy >= area.y as i32 && sy < area.bottom() as i32
}

/// Style for a room given the current selection/current state.
///
/// When a room is BOTH current AND selected, combine both states: use the
/// selected background with the REVERSED modifier from room_current so the
/// room is visually distinct from either state alone.
fn room_style(room: &RenderRoom, state: &AppState) -> Style {
    let is_selected = state.selected_room == Some(room.id);
    if room.is_current && is_selected {
        state.colors.room_selected.add_modifier(Modifier::REVERSED)
    } else if room.is_current {
        state.colors.room_current
    } else if is_selected {
        state.colors.room_selected
    } else {
        state.colors.room_normal
    }
}

// ── cell_to_screen / screen_to_cell / room_at_cell ───────────────────────────

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

/// Map an absolute screen coordinate back to a logical room cell — the exact
/// inverse of `cell_to_screen`.
///
/// `cell.x = (screen.x - area.x) / step_w + scroll.x` (integer division).
/// The result is a grid cell; whether a room actually occupies it is determined
/// separately by `room_at_cell`.
pub fn screen_to_cell(screen: (i32, i32), zoom: Zoom, scroll: (i32, i32), area: Rect) -> (i32, i32) {
    let (step_w, step_h) = zoom_steps(zoom);
    let cx = (screen.0 - area.x as i32).div_euclid(step_w) + scroll.0;
    let cy = (screen.1 - area.y as i32).div_euclid(step_h) + scroll.1;
    (cx, cy)
}

/// Return the screen-space bounding `Rect` for every room in `rm`, clipped to
/// `area`. Uses the same offset logic as `render_map` so click hit-testing is
/// pixel-accurate at all zoom levels, including the non-uniform Boxes layout.
///
/// Rooms whose box falls completely outside `area` are omitted. Rooms that are
/// only partially visible are clipped to `area`.
pub fn room_screen_rects(
    rm: &mapper::render::RenderMap,
    state: &crate::state::AppState,
    area: Rect,
) -> Vec<(mapper::graph::RoomId, Rect)> {
    let zoom = state.zoom;
    let scroll = state.scroll;
    let (bw, bh) = zoom_box_size(zoom);

    let boxes = matches!(zoom, crate::state::Zoom::Boxes);
    let axes = boxes.then(|| boxes_axes(&rm.plan, rm.bounds));
    let (off_x, off_y) = match &axes {
        Some((cols, rows)) => (
            area.x as i32 - cols.room_pixel(scroll.0) + state.char_pan.0,
            area.y as i32 - rows.room_pixel(scroll.1) + state.char_pan.1,
        ),
        None => {
            let (step_w, step_h) = zoom_steps(zoom);
            (area.x as i32 - scroll.0 * step_w + state.char_pan.0,
             area.y as i32 - scroll.1 * step_h + state.char_pan.1)
        }
    };
    let room_virtual = |cell: (i32, i32)| -> (i32, i32) {
        match &axes {
            Some((cols, rows)) => (cols.room_pixel(cell.0), rows.room_pixel(cell.1)),
            None => cell_to_virtual(cell, zoom),
        }
    };

    let mut rects = Vec::with_capacity(rm.rooms.len());
    for room in &rm.rooms {
        let (vx, vy) = room_virtual(room.cell);
        let sx = vx + off_x;
        let sy = vy + off_y;
        // Skip completely off-screen rooms.
        if sx >= area.right() as i32
            || sy >= area.bottom() as i32
            || sx + bw as i32 <= area.x as i32
            || sy + bh as i32 <= area.y as i32
        {
            continue;
        }
        // Clamp to area.
        let rx = (sx.max(area.x as i32)) as u16;
        let ry = (sy.max(area.y as i32)) as u16;
        let rx2 = ((sx + bw as i32).min(area.right() as i32)) as u16;
        let ry2 = ((sy + bh as i32).min(area.bottom() as i32)) as u16;
        if rx2 <= rx || ry2 <= ry {
            continue;
        }
        rects.push((room.id, Rect::new(rx, ry, rx2 - rx, ry2 - ry)));
    }
    rects
}

/// Return the `RoomId` of the room in `layer` at grid `cell`, or `None` if no
/// placed room sits at exactly that cell.  Clicks in the gutter between boxes
/// (where `pos` would fall on a non-integer part of the grid) naturally land on
/// a cell that no room occupies, so they return `None`.
pub fn room_at_cell(
    graph: &mapper::graph::MapGraph,
    layer: mapper::layer::LayerId, // LayerId is u8 (pub type alias in mapper)
    cell: (i32, i32),
) -> Option<RoomId> {
    for id in graph.rooms_in_layer(layer) {
        if let Some(room) = graph.room(id) {
            if room.pos == Some(cell) {
                return Some(id);
            }
        }
    }
    None
}

// ── Styles ────────────────────────────────────────────────────────────────────
//
// Room and connector styles are now read from `state.colors` at render time
// rather than from compile-time constants.  The constants have been removed.
// See `room_style()` and the connector-drawing functions for usage.

// ── render_map ────────────────────────────────────────────────────────────────

/// Draw the map from `rm` into `buf` for `area`, using view state from `state`.
///
/// The whole map is built in scroll-independent virtual space (see `VRect`) and
/// blitted to the screen with a single translation, so panning never re-routes
/// connectors — the routes are identical at every scroll offset.
pub fn render_map(rm: &RenderMap, state: &AppState, area: Rect, buf: &mut Buffer) {
    let zoom = state.zoom;
    let scroll = state.scroll;

    // Build-frame manifest: when the active tidy frame carries a manifest, draw it
    // as text in the map pane and skip room drawing. Overflow past the pane is
    // truncated (diagnostic view).
    if let Some(anim) = &state.tidy_anim {
        if let Some(lines) = anim.current().manifest.as_ref() {
            // The tidy transport panel overlays the top-left of the map pane (see
            // draw_tidy_panel); start the manifest below it, when the panel is drawn,
            // so the panel doesn't cover the connection list.
            let top = if area.width >= crate::render::tidy_panel::PANEL_W
                && area.height >= crate::render::tidy_panel::PANEL_H
            {
                crate::render::tidy_panel::PANEL_H
            } else {
                0
            };
            let avail_h = area.height.saturating_sub(top);
            for (i, line) in lines.iter().take(avail_h as usize).enumerate() {
                let clamped: String = line.chars().take(area.width as usize).collect();
                put_str(buf, area.x as i32, (area.y + top) as i32 + i as i32, &clamped,
                    state.colors.transcript, area);
            }
            return;
        }
    }

    // Overview zoom: one glyph per room, no connectors. Uniform stride.
    if matches!(zoom, crate::state::Zoom::Overview) {
        let (step_w, step_h) = zoom_steps(zoom);
        let off_x = area.x as i32 - scroll.0 * step_w + state.char_pan.0;
        let off_y = area.y as i32 - scroll.1 * step_h + state.char_pan.1;
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
            area.x as i32 - cols.room_pixel(scroll.0) + state.char_pan.0,
            area.y as i32 - rows.room_pixel(scroll.1) + state.char_pan.1,
        ),
        None => {
            let (step_w, step_h) = zoom_steps(zoom);
            (area.x as i32 - scroll.0 * step_w + state.char_pan.0,
             area.y as i32 - scroll.1 * step_h + state.char_pan.1)
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
            draw_stub(edge, &placed, off_x, off_y, area, buf, state.colors.connector);
        }
    }

    // ── 3. Boxes zoom: draw line-art connectors along their assigned lanes, on top of
    //       the rooms drawn below them in step 2.
    let mut arrowheads: Vec<Arrowhead> = Vec::new();
    if let Some((cols, rows)) = &axes {
        arrowheads = render_lane_connectors(&rm.plan, cols, rows, (off_x, off_y), area, buf, &state.symbols.arrows, &state.symbols.path, &state.symbols.portal, &state.colors);
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
        draw_portal_icons(rm, &placed, state, state.show_portal_labels, state.show_room_numbers, (off_x, off_y), area, buf);
    }

    // ── 5. Draw departure/arrival arrowheads LAST, so each embeds in the room ─
    //       border it sits on (replacing the box-edge glyph, pointing outward).
    // Portal view hides the cardinal connector arrowheads so only portal icons sit on borders.
    if !state.show_portal_labels {
        let current_room = rm.rooms.iter().find(|r| r.is_current).map(|r| r.id);
        draw_connector_arrows(&arrowheads, (off_x, off_y), area, buf, &state.colors, state.selected_room, current_room);
        // Restore the room-level up/down glyph for any pair whose up/down connector was
        // suppressed (Task 11): the compass arrowhead just drawn on the shared border cell
        // would otherwise be the only vertical indicator, so re-stamp the glyph over it.
        if boxes {
            draw_deduped_updown_border_glyphs(rm, &placed, state, (off_x, off_y), area, buf);
            if let Some((cols, rows)) = &axes {
                draw_secondary_markers(rm, cols, rows, state, (off_x, off_y), area, buf);
            }
        }
    }
}

// ── Layer tab strip ───────────────────────────────────────────────────────────

/// Draw a one-row layer tab strip at the top of `area` and return the remaining body area.
///
/// Draws nothing (returns `area` unchanged) when:
/// - fewer than 2 non-empty layers exist (single-layer maps are visually unchanged), or
/// - zoom is `Overview`.
///
/// Each non-empty layer is rendered as `name(count)` with a space separator.  The active
/// layer is highlighted with reverse-video.  All drawing is clipped to the strip row.
pub fn draw_layer_strip(
    graph: &mapper::graph::MapGraph,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> Rect {
    use crate::render::draw_str_clipped;

    // Skip in Overview zoom.
    if matches!(state.zoom, crate::state::Zoom::Overview) {
        return area;
    }
    if area.height == 0 {
        return area;
    }

    // Collect non-empty layers in sorted order.
    let mut layers: Vec<_> = graph.layers().keys().copied()
        .filter(|&l| !graph.rooms_in_layer(l).is_empty())
        .collect();
    layers.sort_unstable();

    // Only draw when there are 2+ non-empty layers.
    if layers.len() < 2 {
        return area;
    }

    let active = state.active_layer(graph);
    let strip_y = area.y;
    let strip_area = Rect { x: area.x, y: strip_y, width: area.width, height: 1 };

    // Clear the strip row first.
    let normal_style = Style::new();
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, strip_y)) {
            cell.set_symbol(" ").set_style(normal_style);
        }
    }

    let active_style = Style::new().add_modifier(Modifier::REVERSED);
    let mut x = area.x;
    for layer_id in &layers {
        let name = graph.layer_name(*layer_id);
        let count = graph.rooms_in_layer(*layer_id).len();
        let label = format!(" {}({}) ", name, count);
        let style = if *layer_id == active { active_style } else { normal_style };
        // Clip label to available width.
        let remaining = area.right().saturating_sub(x);
        if remaining == 0 {
            break;
        }
        draw_str_clipped(buf, x, strip_y, &label, style, strip_area);
        x = x.saturating_add(label.chars().count() as u16);
    }

    // Return the area below the strip.
    if area.height <= 1 {
        Rect { x: area.x, y: area.y, width: area.width, height: 0 }
    } else {
        Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height - 1 }
    }
}

/// Variant of [`render_map`] that also draws the layer tab strip when multiple layers exist.
///
/// Production callers (`main.rs`, `map_dump.rs`) should use this function.
/// Tests that call [`render_map`] directly are unaffected.
///
/// The in-content strip is suppressed when `state.colors.map_border_style != BorderStyle::None`,
/// Descriptive label for the room-detection method shown in the map corner.
pub(crate) fn loc_method_label(m: zvm::location::LocationMethod) -> &'static str {
    use zvm::location::LocationMethod::*;
    match m {
        GlobalVar0 => "via status variable",
        PlayerParent => "via player object",
        StatusName => "via name match",
        NameOnly => "via name (unlinked)",
        RoomHeading => "via room heading",
    }
}

/// because in that case the border carries layer tabs via `draw_top_inset` and drawing the
/// in-content strip would produce a double indicator and consume a content row.
pub fn render_map_layered(
    rm: &RenderMap,
    graph: &mapper::graph::MapGraph,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) {
    use crate::render::paneframe::BorderStyle;
    let body_area = if state.colors.map_border_style == BorderStyle::None {
        draw_layer_strip(graph, state, area, buf)
    } else {
        area
    };
    render_map(rm, state, body_area, buf);

    // Detection-method indicator: bottom-right corner, hidden by default.
    if state.show_loc_method {
        if let Some(m) = state.loc_method {
            let label = loc_method_label(m);
            let w = label.chars().count() as u16;
            if area.width >= 1 && area.height >= 1 {
                let y = area.bottom() - 1;
                let x = area.right().saturating_sub(w.min(area.width));
                let style = state.colors.loc_indicator;
                for (cx, ch) in (x..).zip(label.chars()) {
                    if cx >= area.right() {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        let mut b = [0u8; 4];
                        cell.set_symbol(ch.encode_utf8(&mut b)).set_style(style);
                    }
                }
            }
        }
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
fn glyph_for(mask: u8, path: &crate::symbols::PathGlyphs) -> Option<char> {
    Some(match mask {
        m if m == DIR_E | DIR_W => path.ew,
        m if m == DIR_N | DIR_S => path.ns,
        m if m == DIR_S | DIR_E => path.se,
        m if m == DIR_S | DIR_W => path.sw,
        m if m == DIR_N | DIR_E => path.ne,
        m if m == DIR_N | DIR_W => path.nw,
        m if m == DIR_N | DIR_S | DIR_E => path.nse,
        m if m == DIR_N | DIR_S | DIR_W => path.nsw,
        m if m == DIR_E | DIR_W | DIR_S => path.ews,
        m if m == DIR_E | DIR_W | DIR_N => path.ewn,
        m if m == DIR_N | DIR_E | DIR_S | DIR_W => path.nesw,
        // A bare stub end (single direction) — render as the matching straight glyph so
        // the line visibly reaches the box edge rather than vanishing.
        m if m == DIR_E || m == DIR_W => path.ew,
        m if m == DIR_N || m == DIR_S => path.ns,
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

/// A departure/arrival glyph queued by `render_lane_connectors` for `draw_connector_arrows` to
/// paint on top of the rooms: `(virtual pixel, glyph string, distorted, is_portal, owning room,
/// shared)`. `is_portal` selects `colors.portal_connector` for up/down glyphs instead of
/// `colors.connector`/`colors.connector_distorted`. `shared` selects `colors.shared_path` for a
/// connector that collapsed secondary compass directions into itself.
type Arrowhead = ((i32, i32), String, bool, bool, RoomId, bool);

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
/// (and reciprocal arrival) arrowheads as `(virtual pixel, glyph, distorted, is_portal, room_id)`.
/// The arrowheads are NOT drawn here: each sits ON a room's border cell, so the caller draws them
/// AFTER the rooms (which render on top of the line-art) so the arrow replaces the box-edge glyph.
///
/// Up/Down connectors (`exit_dir == Up | Down`) are lane-routed like any compass connector but
/// render differently: their body uses the portal's DOTTED glyphs (not the shared solid set),
/// styled with `colors.portal_connector` (not `colors.connector`/`colors.connector_distorted` —
/// up/down are never distorted), and their departure anchor carries the up/down glyph instead of
/// an arrowhead. They accumulate into a SEPARATE per-cell mask (`updown_cells`) from the compass
/// connectors' `cells` map, so compass crossings/turns are computed exactly as before — up/down
/// never contributes to or reads a compass cell's mask. A matching Up/Down pair now collapses to
/// one RECIPROCAL connector (SQ-0216): the far-end block below draws the up/down glyph (derived
/// from `entry_dir`) at the arrival end too, instead of an arrowhead, so both ends show their own
/// glyph — styled `colors.portal_connector` just like the departure end.
fn render_lane_connectors(
    plan: &RoutePlan,
    cols: &PosTable,
    rows: &PosTable,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
    arrows: &crate::symbols::Arrows,
    path: &crate::symbols::PathGlyphs,
    portal: &crate::symbols::PortalGlyphs,
    colors: &crate::colors::ColorScheme,
) -> Vec<Arrowhead> {
    let (off_x, off_y) = offset;

    // Per-cell accumulated direction mask. ORing masks means a perpendicular crossing of
    // two connectors (one ─, one │) combines to ┼; a connector revisiting its own cell is
    // idempotent and harmless. Compass connectors accumulate in `cells`; up/down connectors
    // accumulate separately in `updown_cells` so the two never mix (dotted vs solid glyphs).
    let mut cells: std::collections::HashMap<(i32, i32), u8> =
        std::collections::HashMap::new();
    let mut updown_cells: std::collections::HashMap<(i32, i32), u8> =
        std::collections::HashMap::new();
    // Dotted glyph set for up/down connector bodies: straight runs read as dotted; any turn
    // glyph falls back to the solid corner set (up/down routes like N/S so may still turn).
    let dotted_path = crate::symbols::PathGlyphs {
        ns: portal.path,
        ew: portal.path_h,
        ..*path
    };
    // Arrowheads: (virtual pixel, glyph string, distorted, is_portal, owning room id). Returned
    // for the caller to draw on top of the rooms (the arrow embeds in the room border). Up/down
    // glyphs are flagged `is_portal` so the caller styles them with `colors.portal_connector`
    // instead of `colors.connector`/`colors.connector_distorted`.
    let mut arrowheads: Vec<Arrowhead> = Vec::new();

    for conn in plan.connectors.iter() {
        let Some(plot) = plot_connector(conn, cols, rows) else { continue };
        let is_updown = matches!(conn.exit_dir, Direction::Up | Direction::Down);
        let has_secondary = !conn.secondary_exit.is_empty() || !conn.secondary_entry.is_empty();
        // Up/down connectors always use the portal selector (they're never distorted);
        // a connector with collapsed secondaries uses the brighter shared_path color;
        // compass connectors otherwise keep their existing connector/connector_distorted styling.
        let style = if is_updown {
            colors.portal_connector
        } else if has_secondary {
            colors.shared_path
        } else if conn.distorted {
            colors.connector_distorted
        } else {
            colors.connector
        };
        let (cell_map, glyphs) = if is_updown {
            (&mut updown_cells, &dotted_path)
        } else {
            (&mut cells, path)
        };

        for (c, mask) in &plot.cells {
            let (sx, sy) = (c.0 + off_x, c.1 + off_y);
            if !in_area(sx, sy, area) {
                continue;
            }
            let entry = cell_map.entry(*c).or_insert(0);
            *entry |= *mask;
            let glyph_s = glyph_for(*entry, glyphs).unwrap_or('·').to_string();
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(&glyph_s).set_style(style);
            }
        }

        let dep_ch = if is_updown {
            match conn.exit_dir {
                Direction::Up => portal.up,
                Direction::Down => portal.down,
                _ => unreachable!("is_updown guards to Up | Down"),
            }
        } else if mapper::direction::is_diagonal(conn.exit_dir) {
            diagonal_arrow(conn.exit_dir, arrows)
        } else {
            arrow_for_departure(conn.exit, arrows)
        };
        arrowheads.push((plot.dep_anchor, dep_ch.to_string(), conn.distorted, is_updown, conn.origin, has_secondary));
        // Far-end glyph only for true reciprocal connectors (collapsed opposite pairs). An
        // up/down reciprocal draws its own up/down glyph (from the back-edge's direction) at
        // the far end too, same as the departure end, rather than an arrow.
        if conn.reciprocal {
            let arr_ch = match conn.entry_dir {
                Some(Direction::Up) if is_updown => portal.up,
                Some(Direction::Down) if is_updown => portal.down,
                Some(d) if mapper::direction::is_diagonal(d) => diagonal_arrow(d, arrows),
                _ => arrow_for_departure(conn.entry, arrows),
            };
            arrowheads.push((plot.arr_anchor, arr_ch.to_string(), conn.distorted, is_updown, conn.dest, has_secondary));
        }
    }
    arrowheads
}

/// Draw the embedded-in-border arrowheads (from [`render_lane_connectors`]) on top of the rooms.
///
/// Each arrowhead carries the `RoomId` of the room it belongs to.  The arrow sits on the
/// room's border, so its background is painted to match that room box's border background —
/// for normal, current, selected, and current+selected rooms alike.  This mirrors
/// `room_style`'s precedence.  The current room reverses only its interior, so its border
/// (where the arrow sits) is not reverse-video and its background is the style's plain `bg`.
/// The arrow glyph foreground is always the connector/path color — `colors.portal_connector`
/// for up/down glyphs (`is_portal`), otherwise `colors.connector`/`colors.connector_distorted`.
fn draw_connector_arrows(
    arrowheads: &[Arrowhead],
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
    colors: &crate::colors::ColorScheme,
    selected_room: Option<RoomId>,
    current_room: Option<RoomId>,
) {
    let (off_x, off_y) = offset;
    for (pos, glyph, distorted, is_portal, room_id, shared) in arrowheads {
        let (vx, vy) = *pos;
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            let connector_style = if *is_portal {
                colors.portal_connector
            } else if *shared {
                colors.shared_path
            } else if *distorted {
                colors.connector_distorted
            } else {
                colors.connector
            };
            let connector_fg = connector_style.fg;
            // Pick the room box's base style with the same precedence as room_style, then
            // derive its VISIBLE background (REVERSED swaps fg/bg at render time).
            let is_sel = selected_room == Some(*room_id);
            let is_cur = current_room == Some(*room_id);
            let base = if is_cur && is_sel {
                colors.room_selected
            } else if is_cur {
                colors.room_current
            } else if is_sel {
                colors.room_selected
            } else {
                colors.room_normal
            };
            // The arrow sits on the box border, which is never reverse-video, so the
            // visible background is the style's plain `bg`.
            let visible_bg = base.bg;
            // Start from reset so no prior highlight bleeds through, then set the matching bg
            // and the connector fg.
            let mut style = Style::reset();
            if let Some(bg) = visible_bg {
                style = style.bg(bg);
            }
            if let Some(fg) = connector_fg {
                style = style.fg(fg);
            }
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }
    }
}

/// Arrow glyph for a compass Direction (used by secondary markers). Up/Down never
/// appear here (they are not collapsed into compass secondaries).
fn arrow_for_direction(dir: Direction, arrows: &crate::symbols::Arrows) -> char {
    match dir {
        Direction::N => arrows.north,
        Direction::S => arrows.south,
        Direction::E => arrows.east,
        Direction::W => arrows.west,
        Direction::NE => arrows.ne,
        Direction::NW => arrows.nw,
        Direction::SE => arrows.se,
        Direction::SW => arrows.sw,
        _ => arrows.north, // unreachable: secondaries are compass only
    }
}

/// Stamp collapsed secondary directions as arrow glyphs on the box interior, one cell
/// inward from the retained connector's arrowhead (stacking further inward for multiples).
/// Boxes zoom only; caller passes the axis tables. Color is `shared_path`.
fn draw_secondary_markers(
    rm: &RenderMap,
    cols: &PosTable,
    rows: &PosTable,
    state: &AppState,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    let (off_x, off_y) = offset;
    let style = state.colors.shared_path;
    let arrows = &state.symbols.arrows;
    let cell_of = |id: RoomId| rm.rooms.iter().find(|r| r.id == id).map(|r| r.cell);

    // The arrowhead anchor + the inward step toward the box interior, chosen the SAME way
    // `plot_connector` places the connector's own arrowhead: a diagonal end sits on the box
    // CORNER (`corner_anchor`) and steps diagonally inward; a cardinal end sits on the side
    // (`box_edge_anchor` at its slot) and steps perpendicular. Using `box_edge_anchor`
    // unconditionally would strand a diagonal connector's markers on a side midpoint while its
    // arrow sits at the corner — i.e. adrift inside the room.
    let anchor_inward = |cell: (i32, i32), side: Side, slot: u16, diag: Option<Direction>|
     -> ((i32, i32), (i32, i32), i32) {
        match diag {
            Some(d) if mapper::direction::is_diagonal(d) => {
                let a = corner_anchor(cols, rows, cell, d);
                let inw = match d {
                    Direction::NE => (-1, 1),
                    Direction::NW => (1, 1),
                    Direction::SE => (-1, -1),
                    Direction::SW => (1, -1),
                    _ => (0, 0),
                };
                (a, inw, (BOX_W - 2).min(BOX_H - 2))
            }
            _ => {
                let a = box_edge_anchor(cols, rows, cell, side, slot);
                let inw = match side {
                    Side::Right => (-1, 0),
                    Side::Left => (1, 0),
                    Side::Top => (0, 1),
                    Side::Bottom => (0, -1),
                };
                let depth = match side {
                    Side::Left | Side::Right => BOX_W - 2,
                    Side::Top | Side::Bottom => BOX_H - 2,
                };
                (a, inw, depth)
            }
        }
    };

    let stamp = |dirs: &[Direction], anchor: (i32, i32), inw: (i32, i32), depth: i32,
                 buf: &mut Buffer| {
        for (k, dir) in dirs.iter().enumerate() {
            let step = k as i32 + 1;
            if step > depth {
                break; // interior full (never happens for the realistic ≤2 case)
            }
            let ch = arrow_for_direction(*dir, arrows);
            put_char(buf, anchor.0 + inw.0 * step + off_x, anchor.1 + inw.1 * step + off_y,
                ch, style, area);
        }
    };

    for conn in &rm.plan.connectors {
        if !conn.secondary_exit.is_empty() {
            if let Some(cell) = cell_of(conn.origin) {
                let (a, inw, depth) = anchor_inward(cell, conn.exit, conn.exit_slot, Some(conn.exit_dir));
                stamp(&conn.secondary_exit, a, inw, depth, buf);
            }
        }
        if !conn.secondary_entry.is_empty() {
            if let Some(cell) = cell_of(conn.dest) {
                let (a, inw, depth) = anchor_inward(cell, conn.entry, conn.entry_slot, conn.entry_dir);
                stamp(&conn.secondary_entry, a, inw, depth, buf);
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
    connector_style: Style,
) {
    let Some(&origin_rect) = placed.get(&edge.origin) else {
        return;
    };
    let label = edge.label.as_deref().unwrap_or("?");
    // Top-right gutter: just right of the box, at the top row.
    let lx = origin_rect.right() + off_x;
    let ly = origin_rect.y + off_y;
    put_str(buf, lx, ly, label, connector_style, area);
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
/// `(glyph_char, dest_label)` pair chosen with `mid_precedence` for the shared mid slot.
type PortalSlots<'a> = [Option<(char, Option<&'a str>)>; 3];

/// True when `pair`'s up/down connector was suppressed (Task 11, SQ-0219) because the same
/// room pair also has a compass connector — the plan draws only the compass path, so no
/// connector-border glyph exists for the up/down link. Callers must then draw the room-level
/// up/down glyph themselves so vertical access still reads. False when the pair has its own
/// up/down connector (it already carries the glyph — drawing it again would double-draw).
fn updown_pair_deduped(plan: &RoutePlan, pair: (RoomId, RoomId)) -> bool {
    let key = |c: &mapper::route::RoutedConnector| (c.origin.min(c.dest), c.origin.max(c.dest));
    let has_compass = plan
        .connectors
        .iter()
        .any(|c| key(c) == pair && mapper::direction::grid_offset(c.exit_dir).is_some());
    let has_updown = plan
        .connectors
        .iter()
        .any(|c| key(c) == pair && matches!(c.exit_dir, Direction::Up | Direction::Down));
    has_compass && !has_updown
}

/// Re-draw a room's up/down border glyph(s) (numbers/default views only) for any pair whose
/// up/down connector was suppressed (Task 11, SQ-0219 — see `updown_pair_deduped`), using the
/// portal-label branch's placement idiom (top border for Up, bottom border for Down, centered
/// at `bx + BOX_W / 2`). Must run AFTER `draw_connector_arrows`: for a straight compass
/// connector between column/row-aligned rooms, its arrowhead lands on this exact shared border
/// cell, and since that's the only way vertical access still reads once the connector no
/// longer draws it, the up/down glyph needs to win that cell. The portal-label view already
/// draws every up/down glyph itself (deduped or not) and suppresses connector arrows entirely,
/// so it never needs this pass — callers must guard on `!state.show_portal_labels`.
fn draw_deduped_updown_border_glyphs(
    rm: &RenderMap,
    placed: &std::collections::HashMap<RoomId, VRect>,
    state: &AppState,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    let (off_x, off_y) = offset;
    let sym_portal = &state.symbols.portal;
    let mut updown_dest: std::collections::HashMap<(RoomId, Direction), RoomId> =
        std::collections::HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub || !matches!(edge.dir, Direction::Up | Direction::Down) {
            continue;
        }
        updown_dest.entry((edge.origin, edge.dir)).or_insert(edge.dest);
    }
    for room in &rm.rooms {
        let Some(&rect) = placed.get(&room.id) else { continue };
        let style = room_style(room, state);
        let (bx, by) = (rect.x, rect.y);
        if let Some(&dest) = updown_dest.get(&(room.id, Direction::Up)) {
            let pair = (room.id.min(dest), room.id.max(dest));
            if updown_pair_deduped(&rm.plan, pair) {
                put_str(buf, bx + BOX_W / 2 + off_x, by + off_y, &sym_portal.up.to_string(), style, area); // top border
            }
        }
        if let Some(&dest) = updown_dest.get(&(room.id, Direction::Down)) {
            let pair = (room.id.min(dest), room.id.max(dest));
            if updown_pair_deduped(&rm.plan, pair) {
                put_str(buf, bx + BOX_W / 2 + off_x, by + BOX_H - 1 + off_y, &sym_portal.down.to_string(), style, area); // bottom border
            }
        }
    }
}

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
    show_room_numbers: bool,
    offset: (i32, i32),
    area: Rect,
    buf: &mut Buffer,
) {
    use std::collections::HashMap;
    let (off_x, off_y) = offset;
    let sym_portal = &state.symbols.portal;

    // Helper: map a direction to the configured portal glyph char.
    let dir_glyph = |dir: Direction| -> char {
        match dir {
            Direction::Up => sym_portal.up,
            Direction::Down => sym_portal.down,
            Direction::In => sym_portal.in_,
            Direction::Out => sym_portal.out,
            _ => sym_portal.unknown,
        }
    };

    // Per room, the chosen (glyph_char, dest_label) for each of the 3 slots; mid slot by precedence.
    let mut chosen: HashMap<RoomId, PortalSlots<'_>> = HashMap::new();
    let mut mid_rank: HashMap<RoomId, u8> = HashMap::new();
    for edge in &rm.edges {
        if !edge.is_stub {
            continue;
        }
        if edge.dir == Direction::Unknown {
            continue; // Unknown edges are non-spatial (e.g. death/respawn) — show no portal icon
        }
        let Some(slot) = portal_slot(edge.dir) else { continue };
        let glyph_ch = dir_glyph(edge.dir);
        let label = edge.dest_label.as_deref();
        let slots = chosen.entry(edge.origin).or_insert([None, None, None]);
        if slot == 1 {
            let rank = mid_precedence(edge.dir);
            let cur = mid_rank.entry(edge.origin).or_insert(u8::MAX);
            if rank < *cur {
                *cur = rank;
                slots[1] = Some((glyph_ch, label));
            }
        } else if slots[slot].is_none() {
            slots[slot] = Some((glyph_ch, label));
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
            if let Some((glyph_ch, label)) = slots[0] {
                let gs = glyph_ch.to_string();
                put_str(buf, bx + BOX_W / 2 + off_x, by + off_y, &gs, style, area); // top border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by - 1 + off_y, name, style, area); // above
                }
            }
            if let Some((glyph_ch, label)) = slots[2] {
                let gs = glyph_ch.to_string();
                put_str(buf, bx + BOX_W / 2 + off_x, by + BOX_H - 1 + off_y, &gs, style, area); // bottom border
                if let Some(name) = label {
                    put_str(buf, bx + off_x, by + BOX_H + off_y, name, style, area); // below
                }
            }
            if let Some((glyph_ch, label)) = slots[1] {
                let gs = glyph_ch.to_string();
                put_str(buf, bx + BOX_W - 1 + off_x, by + 2 + off_y, &gs, style, area); // right border
                // Unknown has no target semantics → glyph only, no floating name.
                if glyph_ch != sym_portal.unknown {
                    if let Some(name) = label {
                        put_str(buf, bx + BOX_W + off_x, by + 2 + off_y, name, style, area); // right
                    }
                }
            }
        } else if show_room_numbers {
            // Numbers shown: directional icon in the interior right column. Up/Down (slots 0/2)
            // now show their glyph on the connector's border anchor instead (see
            // `render_lane_connectors`), so only the mid slot (In/Out/Unknown) still draws here —
            // and the notes marker (drawn by `draw_room` at this same row/col) is no longer
            // overwritten, so it no longer needs to shift.
            if let Some((glyph_ch, _label)) = slots[1] {
                let gs = glyph_ch.to_string();
                let row = by + 1 + 1; // mid slot's row
                put_str(buf, bx + icon_col + off_x, row + off_y, &gs, style, area);
            }
        } else {
            // Numbers hidden: the mid-slot icon (In/Out/Unknown) on interior row 3, centered
            // within the 9-wide interior. Up/Down (slots 0/2) now show their glyph on the
            // connector's border anchor instead (see `render_lane_connectors`).
            if let Some((glyph_ch, _label)) = slots[1] {
                let iw = (BOX_W - 2) as usize; // interior width = 9
                put_str(buf, bx + 1 + off_x, by + 3 + off_y, &center(&glyph_ch.to_string(), iw), style, area);
            }
        }
    }
}

// ── Room drawing ──────────────────────────────────────────────────────────────

/// Pick the outline `BoxStyle` for a room given its flags.
///
/// Precedence: current > portal > selected > normal.
fn outline_for(
    sym: &SymbolSet,
    is_current: bool,
    has_portal: bool,
    selected: bool,
) -> &BoxStyle {
    if is_current { &sym.room_current }
    else if has_portal { &sym.room_portal }
    else if selected { &sym.room_selected }
    else { &sym.room_normal }
}

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
    let selected = state.selected_room == Some(room.id);

    match zoom {
        Zoom::Overview => {
            put_char(buf, sx, sy, '■', base_style, area);
        }
        Zoom::Compact => {
            draw_compact_room(room, sx, sy, base_style, &state.symbols, selected, area, buf);
        }
        Zoom::Boxes => {
            draw_box_room(room, sx, sy, base_style, &state.symbols, selected, state.show_alignment, state.show_room_numbers, area, buf);
        }
    }
}

/// Draw a compact (10×4 step) room: 8×3 box with label row.
///
/// Box is 8 cols wide × 3 rows tall (step 10×4, gutter = 2 cols right, 1 row bottom).
/// Normal rooms use rounded corners; current room uses a heavy border with a
/// REVERSED interior (the border itself stays non-reversed).
fn draw_compact_room(
    room: &RenderRoom,
    sx: i32,
    sy: i32,
    style: Style,
    sym: &SymbolSet,
    selected: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let (bw, bh) = zoom_box_size(Zoom::Compact); // (8, 3)
    let (bw, bh) = (bw as i32, bh as i32);
    let is_current = style.add_modifier.contains(Modifier::REVERSED);

    // The current room reverses only its interior; keep its border non-reversed.
    let mut border_style = style;
    border_style.add_modifier.remove(Modifier::REVERSED);

    let bs = outline_for(sym, is_current, room.has_layer_portal, selected);
    let (tl, tr, bl, br, h, v) = (bs.tl, bs.tr, bs.bl, bs.br, bs.h, bs.v);

    // Top border
    put_char(buf, sx, sy, tl, border_style, area);
    for dx in 1..bw - 1 {
        put_char(buf, sx + dx, sy, h, border_style, area);
    }
    put_char(buf, sx + bw - 1, sy, tr, border_style, area);

    // Middle row: sides + label (inner width = bw - 2 = 6)
    let label_width = (bw - 2) as usize; // 6
    let label: String = room.label.chars().take(label_width).collect();
    put_char(buf, sx, sy + 1, v, border_style, area);
    put_str(buf, sx + 1, sy + 1, &label, style, area);
    put_char(buf, sx + bw - 1, sy + 1, v, border_style, area);

    // Bottom border
    put_char(buf, sx, sy + bh - 1, bl, border_style, area);
    for dx in 1..bw - 1 {
        put_char(buf, sx + dx, sy + bh - 1, h, border_style, area);
    }
    put_char(buf, sx + bw - 1, sy + bh - 1, br, border_style, area);
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
/// Current room: heavy border (┏ ┓ ┗ ┛ ━ ┃) with a REVERSED interior; the
/// border glyphs themselves are drawn non-reversed.
/// Selected room: yellow style (SELECTED_STYLE).
/// Notes: ● marker in top-right inner corner (row 1, col bw-2).
fn draw_box_room(
    room: &RenderRoom,
    sx: i32,
    sy: i32,
    style: Style,
    sym: &SymbolSet,
    selected: bool,
    show_alignment: bool,
    show_room_numbers: bool,
    area: Rect,
    buf: &mut Buffer,
) {
    let (w, h) = zoom_box_size(Zoom::Boxes); // (11, 5)
    let (w, h) = (w as i32, h as i32);
    let is_current = style.add_modifier.contains(Modifier::REVERSED);

    // The current room reverses only its interior; its border keeps the plain
    // (non-reversed) style so the heavy outline stays readable.
    let mut border_style = style;
    border_style.add_modifier.remove(Modifier::REVERSED);

    // Box outline picked by precedence: current > portal > selected > normal.
    let bs = outline_for(sym, is_current, room.has_layer_portal, selected);
    let (tl, tr, bl, br, horiz, vert) = (bs.tl, bs.tr, bs.bl, bs.br, bs.h, bs.v);

    // Top border
    put_char(buf, sx, sy, tl, border_style, area);
    for dx in 1..w - 1 {
        put_char(buf, sx + dx, sy, horiz, border_style, area);
    }
    put_char(buf, sx + w - 1, sy, tr, border_style, area);

    // Inner rows (h=5 → rows 1, 2, 3 are interior: 1=name wrap, 2=name wrap, 3=#id + align)
    for dy in 1..h - 1 {
        put_char(buf, sx, sy + dy, vert, border_style, area);
        // Fill interior with spaces (for background/style)
        for dx in 1..w - 1 {
            put_char(buf, sx + dx, sy + dy, ' ', style, area);
        }
        put_char(buf, sx + w - 1, sy + dy, vert, border_style, area);
    }

    // Room name word-wrapped + centered across the first two interior rows.
    let iw = (w - 2) as usize; // interior width (9)
    let name_lines = wrap_two(&room.label, iw);
    put_str(buf, sx + 1, sy + 1, &center(&name_lines[0], iw), style, area);
    put_str(buf, sx + 1, sy + 2, &center(&name_lines[1], iw), style, area);

    // Row 3: #id (centered), with alignment diagnostics appended when enabled.
    // Only drawn when show_room_numbers is true; when hidden, the row is freed for portal icons.
    if show_room_numbers {
        let mut row3 = format!("#{}", room.id);
        if show_alignment && !room.align_code.is_empty() {
            row3.push(' ');
            row3.push_str(&room.align_code);
        }
        put_str(buf, sx + 1, sy + 3, &center(&row3, iw), style, area);
    }

    // Notes marker in top-right inner corner (row 1, col w-2).
    if room.has_notes {
        put_char(buf, sx + w - 2, sy + 1, sym.portal.marker, style, area);
    }

    // Bottom border
    put_char(buf, sx, sy + h - 1, bl, border_style, area);
    for dx in 1..w - 1 {
        put_char(buf, sx + dx, sy + h - 1, horiz, border_style, area);
    }
    put_char(buf, sx + w - 1, sy + h - 1, br, border_style, area);
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

/// True unless moving room `id` to `cell` would disturb a well-placed Up/Down relationship: an Up
/// room must stay north of its partner (a Down room south), and a room currently stacked in its
/// partner's COLUMN must stay in that column. This stops overlap cleanup from sacrificing a stacked
/// portal room — flipping its side OR dragging it off-column — to clear an overlap; other rooms can
/// move instead. Only currently-good relationships are protected; an already-broken one imposes
/// nothing.
fn move_keeps_updown_sides(
    graph: &mapper::graph::MapGraph,
    id: mapper::graph::RoomId,
    cell: (i32, i32),
) -> bool {
    use mapper::direction::Direction;
    for c in graph.connections() {
        let req = match c.dir {
            Direction::Up => -1,  // dest north of origin (dest.y - origin.y < 0)
            Direction::Down => 1, // dest south of origin
            _ => continue,
        };
        if c.origin != id && c.dest != id {
            continue;
        }
        let (Some(o0), Some(d0)) = (
            graph.room(c.origin).and_then(|r| r.pos),
            graph.room(c.dest).and_then(|r| r.pos),
        ) else {
            continue;
        };
        let o = if c.origin == id { cell } else { o0 };
        let d = if c.dest == id { cell } else { d0 };
        // Side: a currently-correct side must stay correct.
        if (d0.1 - o0.1).signum() == req && (d.1 - o.1).signum() != req {
            return false;
        }
        // Column: a room currently stacked in its partner's column must stay in it.
        if d0.0 == o0.0 && d.0 != o.0 {
            return false;
        }
    }
    true
}

/// Per-room axis lock derived from reciprocal cardinal chains, mirroring the hard equality the
/// VPSC solver (`relayout_auto`) enforces. A room in a reciprocal N/S chain (shares a column) is
/// COLUMN-locked — a greedy cleanup move may only change its Y, never its X; a room in a reciprocal
/// E/W chain (shares a row) is ROW-locked — only its X, never its Y. A room in BOTH is fully pinned.
/// The greedy stages would otherwise break a reciprocal by sliding a locked room off its shared axis
/// to clear an overlap; this is a hard constraint (same spirit as `move_keeps_updown_sides`), so an
/// overlap that can only be cleared by moving a reciprocal room off-axis is left as a residual.
///
/// Precomputed ONCE per cleanup call: chains are a pure function of the graph's connections, which
/// the greedy passes never mutate (they only `set_pos`), so the lock never changes mid-cleanup.
/// Returns `(x_locked, y_locked)` per room; absent rooms are unrestricted.
fn reciprocal_axis_locks(
    graph: &mapper::graph::MapGraph,
) -> std::collections::HashMap<mapper::graph::RoomId, (bool, bool)> {
    let chains = mapper::layout::detect_chains(graph);
    let mut locks: std::collections::HashMap<mapper::graph::RoomId, (bool, bool)> =
        std::collections::HashMap::new();
    for &id in chains.ns.keys() {
        locks.entry(id).or_default().0 = true; // N/S chain → column-locked (X fixed)
    }
    for &id in chains.ew.keys() {
        locks.entry(id).or_default().1 = true; // E/W chain → row-locked (Y fixed)
    }
    locks
}

/// Nudge rooms (bounded Chebyshev `radius`, ≤ `max_passes` passes) until the rendered
/// plan has zero illegal overlaps, secondarily fewer crossings. Deterministic, no overlap,
/// integer cells. Existing position is restored on every rejected trial.
pub(crate) fn cleanup_overlaps(graph: &mut mapper::graph::MapGraph, radius: i32, max_passes: usize) {
    cleanup_overlaps_observed(graph, radius, max_passes, None);
}

/// Observer for animated tidy passes: `(graph, kind, detail, stats)` per step.
type TidyObserver<'a> = &'a mut dyn FnMut(&mapper::graph::MapGraph, &str, &str, &mapper::layout::TidyStats);

pub(crate) fn cleanup_overlaps_observed(
    graph: &mut mapper::graph::MapGraph,
    radius: i32,
    max_passes: usize,
    mut obs: Option<TidyObserver>,
) {
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

    let mut stats = mapper::layout::TidyStats::default();
    let locks = reciprocal_axis_locks(graph);

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

        type Key = (usize, usize, usize, usize, usize, mapper::graph::RoomId, usize);
        let mut best: Option<(Key, mapper::graph::RoomId, (i32, i32))> = None;
        for &id in &room_ids {
            let Some(orig) = graph.room(id).and_then(|r| r.pos) else { continue };
            // Reciprocal N/S rooms are column-locked (X fixed), E/W rooms row-locked (Y fixed).
            let (x_locked, y_locked) = locks.get(&id).copied().unwrap_or((false, false));
            let score_orig = mapper::layout::room_side_score(graph, id);
            let align_orig = mapper::layout::room_alignment_score(graph, id);
            let degree = mapper::layout::room_compass_degree(graph, id);
            for (move_idx, &(dy, dx)) in moves.iter().enumerate() {
                // Skip any candidate that would slide a reciprocal-locked room off its shared axis.
                if (x_locked && dx != 0) || (y_locked && dy != 0) {
                    continue;
                }
                let trial = (orig.0 + dx, orig.1 + dy);
                if graph.rooms().any(|r| r.id != id && r.pos == Some(trial)) {
                    continue;
                }
                if !move_keeps_updown_sides(graph, id, trial) {
                    continue;
                }
                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                let score_trial = mapper::layout::room_side_score(graph, id);
                let align_trial = mapper::layout::room_alignment_score(graph, id);
                graph.set_pos(id, orig);
                if score_trial < score_orig {
                    continue;
                }
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
            Some((_, id, trial)) => {
                let orig = graph.room(id).and_then(|r| r.pos).unwrap_or(trial);
                graph.set_pos(id, trial);
                if let Some(ref mut cb) = obs {
                    stats.overlaps_resolved += 1;
                    let name = graph.room(id).map(|r| r.name.as_str()).unwrap_or("?").to_owned();
                    let desc = format!(
                        "Overlap cleanup: moved room {} ({}) from {:?} to {:?} to clear overlap.",
                        id, name, orig, trial
                    );
                    cb(graph, "cleanup_overlaps", &desc, &stats);
                }
            }
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
    repair_directional_hints_observed(graph, radius, max_passes, None);
}

pub(crate) fn repair_directional_hints_observed(
    graph: &mut mapper::graph::MapGraph,
    radius: i32,
    max_passes: usize,
    mut obs: Option<TidyObserver>,
) {
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

    let mut stats = mapper::layout::TidyStats::default();
    let locks = reciprocal_axis_locks(graph);

    for _ in 0..max_passes {
        let base = render_overlap_stats(graph);
        let base_score = mapper::layout::directional_hint_score(graph);

        let room_ids: Vec<mapper::graph::RoomId> =
            graph.rooms().filter(|r| r.pos.is_some()).map(|r| r.id).collect();

        type Key = (std::cmp::Reverse<usize>, usize, usize, usize, mapper::graph::RoomId, usize);
        let mut best: Option<(Key, mapper::graph::RoomId, (i32, i32))> = None;
        for &id in &room_ids {
            let Some(orig) = graph.room(id).and_then(|r| r.pos) else { continue };
            // Reciprocal N/S rooms are column-locked (X fixed), E/W rooms row-locked (Y fixed).
            let (x_locked, y_locked) = locks.get(&id).copied().unwrap_or((false, false));
            let align_orig = mapper::layout::room_alignment_score(graph, id);
            let degree = mapper::layout::room_compass_degree(graph, id);
            for (move_idx, &(dy, dx)) in moves.iter().enumerate() {
                // Skip any candidate that would slide a reciprocal-locked room off its shared axis.
                if (x_locked && dx != 0) || (y_locked && dy != 0) {
                    continue;
                }
                let trial = (orig.0 + dx, orig.1 + dy);
                if graph.rooms().any(|r| r.id != id && r.pos == Some(trial)) {
                    continue;
                }
                graph.set_pos(id, trial);
                let s = render_overlap_stats(graph);
                let score = mapper::layout::directional_hint_score(graph);
                let align_trial = mapper::layout::room_alignment_score(graph, id);
                graph.set_pos(id, orig);
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
            Some((_, id, trial)) => {
                let orig = graph.room(id).and_then(|r| r.pos).unwrap_or(trial);
                graph.set_pos(id, trial);
                if let Some(ref mut cb) = obs {
                    stats.hints_repaired += 1;
                    let name = graph.room(id).map(|r| r.name.as_str()).unwrap_or("?").to_owned();
                    let desc = format!(
                        "Repair hint: moved room {} ({}) from {:?} to {:?} to restore directional edge.",
                        id, name, orig, trial
                    );
                    cb(graph, "repair_hints", &desc, &stats);
                }
            }
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
    compact_empty_lines_observed(graph, None);
}

pub(crate) fn compact_empty_lines_observed(
    graph: &mut mapper::graph::MapGraph,
    mut obs: Option<TidyObserver>,
) {
    let stats = mapper::layout::TidyStats::default();

    for is_x in [true, false] {
        let mut floor = i32::MIN;
        loop {
            let coords: std::collections::BTreeSet<i32> = graph
                .rooms()
                .filter_map(|r| r.pos.map(|p| if is_x { p.0 } else { p.1 }))
                .collect();
            let (Some(&min), Some(&max)) = (coords.iter().next(), coords.iter().next_back()) else {
                break;
            };
            let Some(empty) = ((min + 1)..max).find(|c| !coords.contains(c) && *c > floor) else {
                break;
            };
            let rooms: Vec<(mapper::graph::RoomId, (i32, i32))> =
                graph.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
            let before = render_overlap_stats(graph).0;
            for &(id, p) in &rooms {
                let c = if is_x { p.0 } else { p.1 };
                if c > empty {
                    graph.set_pos(id, if is_x { (p.0 - 1, p.1) } else { (p.0, p.1 - 1) });
                }
            }
            if render_overlap_stats(graph).0 > before {
                for (id, p) in rooms {
                    graph.set_pos(id, p);
                }
                floor = empty;
            } else {
                if let Some(ref mut cb) = obs {
                    let axis = if is_x { "column" } else { "row" };
                    let desc = format!(
                        "Compact: collapsed empty {} at coordinate {}.",
                        axis, empty
                    );
                    cb(graph, "compact", &desc, &stats);
                }
            }
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
    fn up_connector_draws_updown_glyph_on_border_not_arrow() {
        // A at origin, B directly north, reached by Up. At Boxes zoom the Up connector
        // must render the up glyph (default '↑') somewhere on the border between them,
        // and must NOT render a filled N arrow ('▲') for that vertical link.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let state = AppState::default(); // Boxes zoom by default
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(text.contains('↑'), "the Up connector shows the up glyph on the border");
        assert!(!text.contains('▲'), "the Up connector must NOT render a filled N arrow");
    }

    #[test]
    fn deduped_updown_pair_still_shows_room_glyph() {
        // A--North-->B AND A--Up-->B: Task 11 suppresses the up/down connector, but the
        // rooms must still show the up/down glyph so vertical access reads.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::Up, 2);

        let state = AppState::default(); // default/Boxes view, numbers per default
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let up = state.symbols.portal.up;
        let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(text.contains(up), "the up glyph still shows on the room border even though the connector was suppressed");
    }

    #[test]
    fn reciprocal_updown_connector_draws_glyph_at_both_ends() {
        // Task 9 (SQ-0216): a reciprocal up/down connector draws its glyph at BOTH ends —
        // the up glyph on the lower room's (departure) top border, the down glyph on the
        // upper room's (arrival) bottom border — never an arrow at the far end. Build a
        // routed one-way Up connector via the real pipeline, then patch its metadata to
        // simulate the router's collapse (`reciprocal = true`, `entry_dir = Some(Down)`) so
        // this exercises `render_lane_connectors`'s far-end block directly, independent of
        // whether the router itself pairs the edge.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into()); // lower room
        g.upsert_room(2, "B".into()); // upper room (north of A)
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let rm = mapper::render::render(&g);
        let mut plan = rm.plan.clone();
        let conn = plan
            .connectors
            .iter_mut()
            .find(|c| c.exit_dir == Direction::Up)
            .expect("routed Up connector");
        conn.reciprocal = true;
        conn.entry_dir = Some(Direction::Down);

        let (cols, rows) = boxes_axes(&plan, rm.bounds);
        let area = Rect::new(0, 0, 60, 30);
        let offset = (
            area.x as i32 - cols.room_pixel(rm.bounds.0 .0),
            area.y as i32 - rows.room_pixel(rm.bounds.0 .1),
        );
        let mut buf = Buffer::empty(area);
        let state = AppState::default();
        let arrowheads = render_lane_connectors(
            &plan,
            &cols,
            &rows,
            offset,
            area,
            &mut buf,
            &state.symbols.arrows,
            &state.symbols.path,
            &state.symbols.portal,
            &state.colors,
        );

        let dep = arrowheads.iter().find(|(_, _, _, _, room, _)| *room == 1).expect("A's departure glyph");
        let arr = arrowheads.iter().find(|(_, _, _, _, room, _)| *room == 2).expect("B's arrival glyph");
        assert_eq!(dep.1, "↑", "A (lower room) shows the up glyph on its top border");
        assert_eq!(arr.1, "↓", "B (upper room) shows the down glyph on its bottom border, not an arrow");
    }

    #[test]
    fn reciprocal_updown_glyphs_sit_on_north_and_south_borders() {
        // Task 10 (SQ-0216, regression lock-in): A at origin, B directly north, joined by a
        // real reciprocal Up/Down pair. The up glyph must land on a TOP border row (north
        // side, A's border) and the down glyph on a BOTTOM border row (south side, B's
        // border) — never swapped, never on a left/right side.
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        let rm = render(&g);
        let mut state = AppState::default();
        state.zoom = Zoom::Boxes;
        state.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // Find the up glyph and the down glyph, record their rows.
        let up = state.symbols.portal.up; // default '↑'
        let down = state.symbols.portal.down; // default '↓'
        let mut up_row = None;
        let mut down_row = None;
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                let s = buf.cell((x, y)).expect("cell in area").symbol();
                if s.chars().next() == Some(up) {
                    up_row = Some(y);
                }
                if s.chars().next() == Some(down) {
                    down_row = Some(y);
                }
            }
        }
        let (up_row, down_row) = (up_row.expect("up glyph present"), down_row.expect("down glyph present"));
        // B is north of A (A is the lower room, B the upper room). The up glyph marks A's
        // (lower room's) top border; the down glyph marks B's (upper room's) bottom border.
        // Since the upper room sits at a smaller screen row than the lower room, the down
        // glyph's row is ABOVE the up glyph's row: down_row < up_row.
        assert!(
            down_row < up_row,
            "down glyph (upper room's south border) sits above the up glyph (lower room's north border): down_row={down_row} up_row={up_row}"
        );
    }

    #[test]
    fn loc_method_label_strings() {
        use zvm::location::LocationMethod::*;
        assert_eq!(loc_method_label(GlobalVar0), "via status variable");
        assert_eq!(loc_method_label(PlayerParent), "via player object");
        assert_eq!(loc_method_label(StatusName), "via name match");
        assert_eq!(loc_method_label(NameOnly), "via name (unlinked)");
        assert_eq!(loc_method_label(RoomHeading), "via room heading");
    }

    #[test]
    fn indicator_drawn_bottom_right_when_enabled() {
        use mapper::graph::MapGraph;
        let g = MapGraph::default();
        let rm = mapper::render::render(&g);
        let mut state = AppState::default();
        state.show_loc_method = true;
        state.loc_method = Some(zvm::location::LocationMethod::StatusName);
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_map_layered(&rm, &g, &state, area, &mut buf);
        // The label "via name match" ends at the bottom-right; check its last char.
        let row = area.bottom() - 1;
        let last = buf.cell((area.right() - 1, row)).unwrap().symbol().to_string();
        assert_eq!(last, "h", "expected the 'h' of 'via name match' in the corner");
    }

    #[test]
    fn cleanup_reduces_overlaps_keeping_updown_protected_rooms_aligned() {
        // The A129 house. With correct up/down placement (SQ-0216 #3), room 26 sits SOUTHEAST of
        // 25 — its Up edge (26→Up→25) marks it Y-constrained in the align stage, so it is NOT
        // flattened onto 25's row — and it stacks a protected up/down column with 27
        // (26→Down→27, 27→Up→26 ⇒ 27 directly below 26). cleanup_overlaps must keep those
        // hard-protected up/down rooms in place (`move_keeps_updown_sides`) while nudging
        // unprotected rooms to clear what overlaps it can.
        //
        // HARD-PROTECT DECISION (SQ-0216 #3): up/down placement is inviolable. On this dense
        // 26/27/136 cluster the protected 26↔27 up/down lane used to leave 2 illegal connector
        // overlaps unclearable by cleanup's greedy single-room search. SQ-0222 removed that residual
        // at its source: the straight up/down line now keeps its center slot instead of jogging
        // across the weaving compass connector, so the cluster routes cleanly and cleanup reaches 0
        // WITHOUT moving any protected up/down room. This test still fails if overlaps reappear
        // (routing/cleanup regressed) or the protected up/down column breaks.
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
        // SQ-0222: the 26/27/136 cluster now routes cleanly, so cleanup clears every illegal overlap.
        assert_eq!(render_overlap_stats(&g).0, 0,
            "cleanup clears all illegal overlaps while keeping protected up/down rooms in place");
        let p = |id: u16| g.room(id).unwrap().pos.unwrap();
        // Up/down-protected column stays aligned: 27 stays directly below 26 (26→Down→27).
        assert_eq!(p(26).0, p(27).0, "26/27 up/down column must stay aligned: 26={:?} 27={:?}", p(26), p(27));
        assert!(p(27).1 > p(26).1, "27 stays south of 26 (below it in the up/down lane)");
        // 26's Up edge to 25 stays satisfied: 25 north of 26, and directional x-order 74<25<26.
        assert!(p(25).1 < p(26).1, "25 stays north of 26 (26→Up→25): 25={:?} 26={:?}", p(25), p(26));
        assert!(p(25).0 > p(74).0 && p(26).0 > p(25).0, "directional x-order 74<25<26 preserved");
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

        // The current room reverses only its interior: the top-left corner (a border
        // cell) is NOT reversed, while an interior cell (one in, one down) IS.
        let border = buf.cell((cx, cy)).expect("border cell should exist");
        assert!(
            !border.modifier.contains(Modifier::REVERSED),
            "current room border cell must NOT be REVERSED; got modifier={:?}",
            border.modifier
        );
        let interior = buf.cell((cx + 1, cy + 1)).expect("interior cell should exist");
        assert!(
            interior.modifier.contains(Modifier::REVERSED),
            "current room interior cell should have REVERSED modifier; got modifier={:?}",
            interior.modifier
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
        let mut state = AppState::default(); // Boxes zoom
        state.show_room_numbers = true; // enable to see #id on row 3
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
    fn cleanup_keeps_updown_protected_column_chain_aligned() {
        // cleanup_overlaps must keep protected COLUMN chains aligned through overlap resolution.
        //
        // Two hard-protected columns are guarded here:
        //  - the up/down lane 26→Down→27 / 27→Up→26 (`move_keeps_updown_sides`), and
        //  - (SQ-0216 reciprocal-compass lock) the reciprocal N/S pair 74 S->76 / 76 N->74.
        // An earlier build lacked the reciprocal-compass lock, so cleanup's greedy search would
        // shift the then-unprotected 76 one column west to cut crossings, breaking 74<->76. With the
        // reciprocal lock, 76 is column-locked and can only slide along 74's column, so both columns
        // now survive cleanup. We verify both, plus that all illegal overlaps clear (SQ-0222).
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
        assert_eq!(p(&g,26).0, p(&g,27).0, "precondition: relayout column-aligns the 26↔27 up/down lane");
        cleanup_overlaps(&mut g, 3, 40);
        assert_eq!(render_overlap_stats(&g).0, 0,
            "cleanup clears all illegal overlaps (SQ-0222 clean routing) while protecting the up/down column");
        assert_eq!(p(&g,26).0, p(&g,27).0,
            "27 must stay directly below 26 after cleanup (up/down-protected): 26={:?} 27={:?}", p(&g,26), p(&g,27));
        assert!(p(&g,27).1 > p(&g,26).1, "27 stays south of 26 in the up/down lane");
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "76 must stay on 74's column after cleanup (reciprocal N/S locked): 74={:?} 76={:?}", p(&g,74), p(&g,76));
    }

    #[test]
    fn repair_puts_78_west_of_180_after_retidy() {
        // The full Retidy flow (relayout -> cleanup_overlaps -> repair_directional_hints) on A129
        // must leave 78 west of 180 (the 180->W->78 hint). With the length-priority router,
        // cleanup_overlaps now settles this ordering directly; repair_directional_hints stays in the
        // flow as the safety net that recovers the hint on inputs where a post-solve stage
        // sacrifices it.
        //
        // With SQ-0222 clean routing this dense fixture clears to zero illegal overlaps; repair must
        // keep it at zero (introduce none). It must also leave the hard-protected columns intact:
        // the 26↔27 up/down lane and (with the reciprocal-compass lock) the reciprocal N/S pair
        // 74<->76, which is now column-locked through the whole flow.
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
        let p = |g: &MapGraph, id: u16| g.room(id).unwrap().pos.unwrap();
        assert!(p(&g,78).0 < p(&g,180).0,
            "retidy must place 78 west of 180: 78={:?} 180={:?}", p(&g,78), p(&g,180));
        assert_eq!(render_overlap_stats(&g).0, 0,
            "repair keeps all illegal overlaps cleared (SQ-0222 clean routing)");
        assert_eq!(p(&g,26).0, p(&g,27).0,
            "repair must not knock the up/down-protected 26↔27 column off alignment: 26={:?} 27={:?}", p(&g,26), p(&g,27));
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "repair must keep the reciprocal N/S pair 74<->76 column-locked: 74={:?} 76={:?}", p(&g,74), p(&g,76));
    }

    #[test]
    fn yielded_updown_pair_draws_a_lane_connector_not_a_stub() {
        // Up/Down pair placed far apart (yielded from a clean stack). Task 6 routes Up/Down as a
        // full lane connector regardless of adjacency, so the pair now draws a routed dotted line
        // (not the old draw_portal_connectors/portal_stub right-column stub), plus the up/down
        // glyphs on each room's border.
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
        assert!(count("┊") + count("┄") >= 1, "the routed Up/Down connector body is dotted");
        assert!(count("↑") >= 1, "up glyph present on a border/icon");
        assert!(count("↓") >= 1, "down glyph present on a border/icon");
    }

    #[test]
    fn updown_connector_uses_portal_connector_color_not_connector() {
        // Regression (SQ-0216 review finding): up/down connectors must style their dotted body
        // AND their up/down border glyphs with `colors.portal_connector`, not the generic
        // `colors.connector` used by compass connectors. Build a map with BOTH an up/down pair
        // (far apart so the body draws a routed dotted line, not just a direct bridge) and an
        // unrelated compass connector, set `portal_connector` and `connector` to distinct
        // colors, and assert each connector kind picked up the right one.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (3, 2)); // far from (0,-1) — yielded, forces a routed body
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        g.upsert_room(3, "C".into());
        g.upsert_room(4, "D".into());
        g.set_pos(3, (0, 4));
        g.set_pos(4, (1, 4));
        g.add_edge(3, Direction::E, 4);

        let rm = render(&g);
        let mut st = AppState::default();
        st.zoom = Zoom::Boxes;
        st.scroll = rm.bounds.0;
        st.colors.connector = Style::new().fg(Color::Green);
        st.colors.portal_connector = Style::new().fg(Color::Rgb(10, 20, 30));
        let area = Rect::new(0, 0, 100, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);

        let portal_fg = st.colors.portal_connector.fg;
        let connector_fg = st.colors.connector.fg;
        assert_ne!(portal_fg, connector_fg, "test colors must be distinct to be meaningful");

        // Every dotted body glyph and up/down border glyph must use portal_connector's fg.
        let mut found_dotted = false;
        let mut found_updown_glyph = false;
        for cell in buf.content.iter() {
            match cell.symbol() {
                "┊" | "┄" => {
                    found_dotted = true;
                    assert_eq!(cell.fg, portal_fg.unwrap(), "dotted up/down body must use portal_connector fg");
                }
                "↑" | "↓" => {
                    found_updown_glyph = true;
                    assert_eq!(cell.fg, portal_fg.unwrap(), "up/down border glyph must use portal_connector fg");
                }
                _ => {}
            }
        }
        assert!(found_dotted, "expected at least one dotted up/down body glyph");
        assert!(found_updown_glyph, "expected at least one up/down border glyph");

        // The unrelated compass connector (C -E-> D) must still use `colors.connector`.
        let mut found_compass_arrow = false;
        for cell in buf.content.iter() {
            if cell.symbol() == "▶" {
                found_compass_arrow = true;
                assert_eq!(cell.fg, connector_fg.unwrap(), "compass arrowhead must keep colors.connector fg");
                assert_ne!(cell.fg, portal_fg.unwrap(), "compass arrowhead must not use portal_connector fg");
            }
        }
        assert!(found_compass_arrow, "expected the compass connector's ▶ arrowhead");
    }

    #[test]
    fn cleanup_guard_protects_a_stacked_updown_room() {
        // 2 is up from 1 and stacked directly in its column (both x=0), north of it. The guard must
        // forbid moving 2 south of 1, moving 1 north of 2, and dragging 2 off 1's column — while
        // still allowing 2 to move vertically within the column (staying north).
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "p".into());
        g.upsert_room(2, "u".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // 2 north of 1, same column
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        assert!(!move_keeps_updown_sides(&g, 2, (0, 1)), "must forbid moving the up room SOUTH");
        assert!(!move_keeps_updown_sides(&g, 1, (0, -5)), "must forbid moving the partner NORTH of it");
        assert!(!move_keeps_updown_sides(&g, 2, (3, -2)), "must forbid dragging it off the column");
        assert!(move_keeps_updown_sides(&g, 2, (0, -3)), "moving it up within the column is fine");
    }

    #[test]
    fn reciprocal_axis_locks_classify_ns_ew_and_cross_rooms() {
        // reciprocal_axis_locks encodes the VPSC hard equality the greedy cleanup must respect:
        // a reciprocal N/S pair (share a column) → column-locked (x_locked, Y free); a reciprocal
        // E/W pair (share a row) → row-locked (y_locked, X free); a room in BOTH → fully pinned;
        // a non-reciprocal room → absent (unrestricted).
        use mapper::graph::MapGraph;
        use Direction::*;
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4, 5] {
            g.upsert_room(id, "r".into());
        }
        // 1<->2 reciprocal N/S (1 N->2, 2 S->1): column chain.
        g.add_edge(1, N, 2);
        g.add_edge(2, S, 1);
        // 3<->4 reciprocal E/W (3 E->4, 4 W->3): row chain.
        g.add_edge(3, E, 4);
        g.add_edge(4, W, 3);
        // 2 is ALSO reciprocal E/W with 3 (2 E->3, 3 W->2): 2 is a cross-chain (both) room.
        g.add_edge(2, E, 3);
        g.add_edge(3, W, 2);
        // 5 has only a one-way edge — no reciprocal, so no lock.
        g.add_edge(1, W, 5);

        let locks = reciprocal_axis_locks(&g);
        assert_eq!(locks.get(&1).copied(), Some((true, false)), "1 is N/S-reciprocal only → column-locked");
        assert_eq!(locks.get(&2).copied(), Some((true, true)), "2 is in an N/S AND an E/W chain → fully pinned");
        assert_eq!(locks.get(&3).copied(), Some((false, true)), "3 is E/W-reciprocal (with 2 and 4) only → row-locked");
        assert_eq!(locks.get(&4).copied(), Some((false, true)), "4 is E/W-reciprocal only → row-locked");
        assert_eq!(locks.get(&5).copied(), None, "5 has no reciprocal edge → unrestricted");
    }

    #[test]
    fn cleanup_locks_reciprocal_ns_pair_to_its_shared_column() {
        // SQ-0216: the greedy overlap cleanup must honor the reciprocal N/S hard equality the VPSC
        // solver enforces — a room in a reciprocal N/S chain is COLUMN-locked and may only slide
        // along its shared column, never off it. On this dense A129 fixture the reciprocal pair
        // 74<->76 (74 S->76, 76 N->74) shares a column after relayout; WITHOUT the lock, cleanup's
        // greedy search shifts the (then-unprotected) 76 one column WEST to cut crossings, breaking
        // the reciprocal (verified: 76 moves from x=-1 to x=-2). WITH the lock, 76 can only move in
        // Y, so it stays on 74's column. All illegal overlaps clear (SQ-0222 clean routing) with the
        // reciprocal pair still column-locked — the lock constrains 76 without leaving any residual.
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
        assert_eq!(p(&g,74).0, p(&g,76).0, "precondition: relayout column-aligns the 74<->76 reciprocal N/S pair");
        cleanup_overlaps(&mut g, 3, 40);
        assert_eq!(p(&g,74).0, p(&g,76).0,
            "76 must stay on 74's column after cleanup (reciprocal N/S locked): 74={:?} 76={:?}", p(&g,74), p(&g,76));
        assert!(p(&g,76).1 > p(&g,74).1, "76 stays south of 74 (only slid along the shared column, if at all)");
        assert_eq!(render_overlap_stats(&g).0, 0, "all illegal overlaps clear (SQ-0222) with the reciprocal N/S pair still locked");
    }

    #[test]
    fn cleanup_keeps_reciprocal_ew_chain_on_its_row() {
        // Row-lock analog to cleanup_locks_reciprocal_ns_pair_to_its_shared_column: a room in a
        // reciprocal E/W chain (shares a row) is ROW-locked — cleanup may change only its X, never
        // its Y. This asserts the symmetric guarantee on the same A129 fixture: the reciprocal E/W
        // chain 74<->79<->203<->193 stays on one shared row through overlap cleanup. (Up/Down
        // connectors are inherently vertical, so this dense fixture happens to apply no off-row
        // pressure here — the guard nonetheless pins the symmetric row-lock the code applies
        // identically to N/S; see reciprocal_axis_locks_classify_ns_ew_and_cross_rooms.)
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
        let ew_row = [74u16, 79, 203, 193];
        let r0 = p(&g, 74).1;
        assert!(ew_row.iter().all(|&id| p(&g, id).1 == r0),
            "precondition: relayout row-aligns the reciprocal E/W chain");
        cleanup_overlaps(&mut g, 3, 40);
        let r = p(&g, 74).1;
        for &id in &ew_row {
            assert_eq!(p(&g, id).1, r,
                "reciprocal E/W room {id} must stay on the shared row after cleanup: {:?}", p(&g, id));
        }
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
    fn compact_preserves_directional_order_introducing_no_overlap() {
        // Full A129 Retidy flow plus compaction: 78 stays west of 180, the hard-protected 26↔27
        // up/down column stays aligned, compaction introduces no illegal overlap (SQ-0222 clean
        // routing keeps the cluster clear), and no fully-empty interior column/row is left behind.
        //
        // With the SQ-0216 reciprocal-compass lock, "76 stays under 74" holds again: 76 is
        // column-locked to its reciprocal N/S partner 74 through the whole flow. We assert that,
        // the still-guaranteed directional order (78 west of 180), the hard-protected 26↔27 up/down
        // column, and zero illegal overlaps.
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
        assert_eq!(p(&g,26).0, p(&g,27).0, "26↔27 up/down column stays aligned through compaction");
        assert_eq!(p(&g,74).0, p(&g,76).0, "reciprocal N/S pair 74<->76 stays column-locked through compaction");
        assert_eq!(render_overlap_stats(&g).0, 0,
            "compaction introduces no illegal overlap (SQ-0222 clean routing keeps the cluster clear)");
        // Compaction must leave only GUTTER lines — an empty interior column/row remains only when
        // collapsing it would create an illegal overlap (e.g. the column a long direct route runs up).
        // Any empty interior line that could still collapse cleanly is a compaction miss.
        let collapsible = |g: &MapGraph, is_x: bool, line: i32| -> bool {
            let mut t = g.clone();
            let before = render_overlap_stats(&t).0;
            let rooms: Vec<_> = t.rooms().map(|r| (r.id, r.pos.unwrap())).collect();
            for (id, pos) in rooms {
                let c = if is_x { pos.0 } else { pos.1 };
                if c > line {
                    t.set_pos(id, if is_x { (pos.0 - 1, pos.1) } else { (pos.0, pos.1 - 1) });
                }
            }
            render_overlap_stats(&t).0 <= before
        };
        let xs: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().0).collect();
        let ys: std::collections::BTreeSet<i32> = g.rooms().map(|r| r.pos.unwrap().1).collect();
        for (is_x, set) in [(true, &xs), (false, &ys)] {
            let (min, max) = (*set.iter().next().unwrap(), *set.iter().next_back().unwrap());
            for line in (min + 1)..max {
                if !set.contains(&line) {
                    assert!(!collapsible(&g, is_x, line),
                        "empty interior {} {line} should have compacted (its collapse adds no overlap)",
                        if is_x { "column" } else { "row" });
                }
            }
        }
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
        let mut state = AppState::default(); // Boxes, align off
        state.show_room_numbers = true; // enable to verify #id on row 3
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
            st.show_room_numbers = true; // alignment codes ride the #id row
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
        let mut state = AppState::default(); // Boxes, scroll (0,0), labels off
        state.show_room_numbers = true; // right-column layout requires numbers shown
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // Box of room 1 is at screen (0,0); right interior column is col 9 (BOX_W-2).
        // In (non-spatial) still gets the mid-slot interior icon.
        assert_eq!(sym(9, 2), "⊙", "in icon in middle-right interior (row 2)");
        // Up/Down no longer draw an interior icon — they show their glyph on the connector's
        // border anchor instead (top/bottom centre of the box, col 5 = BOX_W/2).
        assert_ne!(sym(9, 1), "↑", "up icon leaves the upper-right interior");
        assert_ne!(sym(9, 3), "↓", "down icon leaves the lower-right interior");
        assert_eq!(sym(5, 0), "↑", "up glyph on the top border centre");
        assert_eq!(sym(5, 4), "↓", "down glyph on the bottom border centre");
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
        let mut state = AppState::default(); // Boxes zoom, scroll (0,0), labels off
        state.show_room_numbers = true; // right-column layout requires numbers shown
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        // col 9 = BOX_W - 2 = 11 - 2 = 9; row 2 = mid slot
        assert_eq!(sym(9, 2), "⊙", "In beats Out in mid slot: expected ⊙, got '{}'", sym(9, 2));
    }

    #[test]
    fn portal_icon_up_no_longer_shifts_notes_marker() {
        // The Up icon used to claim the same interior cell as the notes marker (upper-right
        // corner), forcing the marker to shift one cell left. Now Up shows its glyph on the
        // connector's border anchor instead, so the interior cell is free and the notes marker
        // stays in its normal (unshifted) spot.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Attic".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.set_notes(1, "stuff".into());
        g.add_edge(1, Direction::Up, 2);
        let rm = render(&g);
        let mut state = AppState::default();
        state.show_room_numbers = true; // right-column layout requires numbers shown
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
        assert_eq!(sym(9, 1), "●", "notes marker stays put; the interior up icon is gone");
        assert_eq!(sym(5, 0), "↑", "up glyph now appears on the top border centre");
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
    fn unknown_portal_draws_no_icon_or_name() {
        // An Unknown-direction edge is non-spatial (e.g. a death/respawn the game gave no direction
        // for), so it draws no portal icon and no destination name in either view.
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
        let count = |s: &str| buf.content.iter().filter(|c| c.symbol() == s).count();
        assert_eq!(count("?"), 0, "an Unknown portal draws no ? icon");
        // No destination name to the right of room 1's box (row 2, the portal-label region).
        let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();
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
    fn compass_and_updown_on_same_pair_both_draw_without_illegal_overlap() {
        // SQ-0224 reverses SQ-0219's suppression: when a compass edge AND an up/down edge join the
        // same room pair, BOTH draw (the compass path plus a dotted up/down body), and the
        // room-level up glyph still renders on the border. Even in the same-axis case here (N
        // compass + Up, vertically adjacent — the worst case, both vertical) the two connectors
        // share the side at distinct slots and must not form an ILLEGAL overlap.
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (1, 0)); // due north of room 1
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(1, Direction::N, 2); // a compass connector also joins the pair
        assert_eq!(render_overlap_stats(&g).0, 0,
            "both-drawn same-pair connectors must not form an illegal overlap");
        let rm = render(&g);
        let mut st = AppState::default();
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let has_dotted = buf.content.iter().any(|c| matches!(c.symbol(), "┊" | "┄"));
        assert!(has_dotted, "the up/down connector now draws its dotted body alongside the compass path");
        let has_up_glyph = buf.content.iter().any(|c| c.symbol() == "↑");
        assert!(has_up_glyph, "the room still shows the up glyph on its border");
    }

    #[test]
    fn interlayer_badge_dest_label_appears_in_portal_view() {
        // Build a two-layer graph: Hall (1) and Study (2) on MAIN_LAYER, linked by a Down
        // portal from Hall to Cellar (3). Cellar + Wine (4) are peeled into a new layer.
        // Rendering MAIN_LAYER in portal view must show the destination layer name ("Cellar")
        // floating beside Hall's box — confirming inter-layer stubs render their dest_label.
        use mapper::graph::MapGraph;
        use mapper::layer::{peel_region, MAIN_LAYER};
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Study".into());
        g.upsert_room(3, "Cellar".into());
        g.upsert_room(4, "Wine".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.set_pos(3, (0, 1));
        g.set_pos(4, (1, 1));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        peel_region(&mut g, 3).expect("cellar + wine must peel into a new layer");
        // render_layer builds the MAIN_LAYER sub-graph and appends inter-layer badge stubs.
        let rm = mapper::render::render_layer(&g, MAIN_LAYER);
        // At least one inter-layer badge stub with a dest_label must be present.
        assert!(
            rm.edges.iter().any(|e| e.is_stub && e.dest_label.as_deref().is_some()),
            "render_layer must include inter-layer badge stubs with dest_label"
        );
        let mut st = AppState::default();
        st.show_portal_labels = true; // portal view floats destination names outside boxes
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 120, 60);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        // The dest_label is "<room> · <layer>": e.g. "Cellar · Cellar". The layer name
        // assigned by peel_region is the first-room label. Assert that "Cellar" appears
        // somewhere in the buffer (both the room name and layer name contain it).
        let all_text: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            all_text.contains("Cellar"),
            "inter-layer badge dest_label must appear in portal view; buffer text: '{}'",
            all_text.chars().filter(|c| !c.is_whitespace()).collect::<String>()
        );
    }

    #[test]
    fn layer_portal_room_gets_double_line_outline() {
        // A room with an outgoing portal to another layer renders with a double-line box
        // outline (╔═╗ … ║) instead of the rounded one, so cross-layer exits read at a glance.
        use mapper::graph::MapGraph;
        use mapper::layer::{peel_region, MAIN_LAYER};
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        peel_region(&mut g, 2).expect("peel cellar into its own layer");
        let rm = mapper::render::render_layer(&g, MAIN_LAYER);
        assert!(
            rm.rooms.iter().find(|r| r.id == 1).unwrap().has_layer_portal,
            "Hall owns the outgoing cross-layer portal"
        );
        let mut st = AppState::default(); // Boxes zoom
        st.scroll = rm.bounds.0;
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &st, area, &mut buf);
        let all_text: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert!(
            all_text.contains('╔') && all_text.contains('║'),
            "the layer-portal room must render with a double-line outline"
        );
    }

    #[test]
    fn path_and_portal_use_symbol_set() {
        // Two rooms connected N-S: glyph_for should produce the NS path char at the connector.
        // Also: a room with notes should show the portal.marker glyph.
        use mapper::graph::MapGraph;
        use crate::symbols::SymbolSet;
        use crate::config::SymbolConfig;

        // --- Path glyph test: two horizontally-connected rooms produce EW path segments ---
        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);

        let mut state = AppState::default();
        state.scroll = (0, 0);
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        // The EW connector between the two rooms should have '─' (light path) somewhere
        let has_ew = buf.content.iter().any(|c| c.symbol() == "─");
        assert!(has_ew, "default light path: EW connector must use '─'");

        // With heavy preset, EW should be '━'
        let mut cfg = SymbolConfig::default();
        cfg.path_style = "heavy".into();
        state.symbols = SymbolSet::resolve(&cfg);
        let mut buf2 = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf2);
        let has_heavy_ew = buf2.content.iter().any(|c| c.symbol() == "━");
        assert!(has_heavy_ew, "heavy path preset: EW connector must use '━'");

        // --- Portal marker test: a room with notes shows portal.marker ---
        let mut g2 = MapGraph::new();
        g2.upsert_room(10, "A".into());
        g2.set_pos(10, (0, 0));
        g2.set_notes(10, "some notes".into());
        let rm2 = mapper::render::render(&g2);
        state.symbols = SymbolSet::default();
        let mut buf3 = Buffer::empty(area);
        render_map(&rm2, &state, area, &mut buf3);
        let has_marker = buf3.content.iter().any(|c| c.symbol() == "●");
        assert!(has_marker, "default portal.marker '●' must appear for room with notes");
    }

    #[test]
    fn arrow_uses_symbol_set() {
        // room1(0,0) →E→ room2(1,0): with default symbols the departure arrow is '▶';
        // with arrow_set = "line" it becomes '→'.
        use mapper::graph::MapGraph;
        use crate::symbols::SymbolSet;
        use crate::config::SymbolConfig;

        let mut g = MapGraph::new();
        g.upsert_room(1, "R1".into());
        g.upsert_room(2, "R2".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let rm = mapper::render::render(&g);

        // Default: '▶' at the departure arrow cell (10, 2)
        let mut state = AppState::default();
        state.scroll = (0, 0);
        let area = Rect::new(0, 0, 80, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        assert_eq!(
            buf.cell((10, 2)).map(|c| c.symbol()),
            Some("▶"),
            "default symbols: east departure arrow must be '▶'"
        );

        // Line preset: '→' at the same cell
        let mut cfg = SymbolConfig::default();
        cfg.arrow_set = "line".into();
        state.symbols = SymbolSet::resolve(&cfg);
        let mut buf2 = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf2);
        assert_eq!(
            buf2.cell((10, 2)).map(|c| c.symbol()),
            Some("→"),
            "line preset: east departure arrow must be '→'"
        );
    }

    #[test]
    fn room_outline_uses_symbol_set() {
        // Default symbols: a normal (non-current, non-portal) room at cell (0,0) with
        // scroll (0,0) and Boxes zoom renders its top-left corner as '╭'.
        use mapper::graph::MapGraph;
        use crate::symbols::SymbolSet;
        use crate::config::SymbolConfig;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        // Room 1 is not current, not a portal room.
        let rm = mapper::render::render(&g);

        // --- Default symbols: expect '╭' at (0,0) ---
        let mut state = AppState::default(); // SymbolSet::default() inside
        state.scroll = (0, 0);
        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        assert_eq!(
            buf.cell((0, 0)).map(|c| c.symbol()),
            Some("╭"),
            "default symbols must render normal room top-left as rounded corner"
        );

        // --- ASCII preset: expect '+' at (0,0) ---
        let mut cfg = SymbolConfig::default();
        cfg.box_style = "ascii".into();
        state.symbols = SymbolSet::resolve(&cfg);
        let mut buf2 = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf2);
        assert_eq!(
            buf2.cell((0, 0)).map(|c| c.symbol()),
            Some("+"),
            "ascii preset must render normal room top-left as '+'"
        );
    }

    // ── screen_to_cell / room_at_cell tests ───────────────────────────────────

    /// screen_to_cell is the exact inverse of cell_to_screen for placed rooms.
    #[test]
    fn screen_to_cell_inverts_cell_to_screen() {
        use crate::state::Zoom;
        use ratatui::layout::Rect;

        for zoom in [Zoom::Boxes, Zoom::Compact, Zoom::Overview] {
            let scroll = (2, 3);
            let area = Rect::new(5, 2, 100, 50);
            let cell = (4, 5);

            // Forward: cell → screen.
            let screen = cell_to_screen(cell, zoom, scroll, area).expect("should be in area");

            // Inverse: screen → cell.
            let back = screen_to_cell((screen.0 as i32, screen.1 as i32), zoom, scroll, area);
            assert_eq!(
                back, cell,
                "screen_to_cell should invert cell_to_screen for zoom {:?}: cell {:?} -> screen {:?} -> back {:?}",
                zoom, cell, screen, back
            );
        }
    }

    #[test]
    fn screen_to_cell_with_zero_scroll_and_origin_area() {
        use crate::state::Zoom;
        use ratatui::layout::Rect;

        let zoom = Zoom::Compact; // step = (12, 5)
        let scroll = (0, 0);
        let area = Rect::new(0, 0, 80, 40);

        // A click at screen (24, 10) should land in cell (2, 2).
        let cell = screen_to_cell((24, 10), zoom, scroll, area);
        assert_eq!(cell, (2, 2));

        // A click at (0, 0) lands at (0, 0).
        let cell0 = screen_to_cell((0, 0), zoom, scroll, area);
        assert_eq!(cell0, (0, 0));
    }

    /// room_at_cell finds a placed room and returns None for an empty cell.
    #[test]
    fn room_at_cell_finds_placed_room() {
        use mapper::graph::MapGraph;
        use mapper::layer::MAIN_LAYER;

        let mut g = MapGraph::new();
        g.upsert_room(1, "Start".into());
        g.upsert_room(2, "North".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));

        // Room 1 is at (0,0).
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (0, 0)), Some(1));
        // Room 2 is at (0,-1).
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (0, -1)), Some(2));
        // (1, 0) has no room.
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (1, 0)), None);
        // (0, 1) has no room.
        assert_eq!(room_at_cell(&g, MAIN_LAYER, (0, 1)), None);
    }

    /// room_screen_rects returns non-empty rects within the area, and hit-testing
    /// a click at each rect's centre finds the correct room.
    #[test]
    fn room_screen_rects_basic_hit_test() {
        use crate::state::{AppState, Zoom};
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;
        use ratatui::layout::Rect;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.add_edge(1, mapper::direction::Direction::E, 2);

        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.scroll = (0, 0);

        let area = Rect::new(0, 0, 80, 40);
        let rm = render_layer(&g, mapper::layer::MAIN_LAYER);
        let rects = room_screen_rects(&rm, &state, area);

        // Both rooms must appear.
        assert_eq!(rects.len(), 2, "both rooms must have screen rects");

        // Every rect must be fully within the area.
        for (_, r) in &rects {
            assert!(r.x >= area.x, "rect left must be within area");
            assert!(r.y >= area.y, "rect top must be within area");
            assert!(r.right() <= area.right(), "rect right must be within area");
            assert!(r.bottom() <= area.bottom(), "rect bottom must be within area");
            assert!(r.width > 0 && r.height > 0, "rect must have positive dimensions");
        }

        // Hit-testing: a click at each rect's centre must find that room.
        for (id, r) in &rects {
            let cx = r.x + r.width / 2;
            let cy = r.y + r.height / 2;
            let hit = rects.iter()
                .find(|(_, rect)| cx >= rect.x && cx < rect.right() && cy >= rect.y && cy < rect.bottom())
                .map(|(rid, _)| *rid);
            assert_eq!(hit, Some(*id), "click at centre of room {:?} rect must hit that room", id);
        }
    }

    // ── Item 1: char_pan shifts room screen rects ─────────────────────────────

    /// char_pan should shift room screen rects by the same offset so that
    /// mouse hit-testing remains accurate after a drag pan.
    #[test]
    fn char_pan_shifts_room_screen_rects() {
        use crate::state::{AppState, Zoom};
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;
        use ratatui::layout::Rect;

        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.set_pos(1, (0, 0));
        let rm = render_layer(&g, mapper::layer::MAIN_LAYER);

        let area = Rect::new(0, 0, 80, 40);

        // Baseline: no char_pan.
        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.scroll = (0, 0);
        state.char_pan = (0, 0);
        let rects_base = room_screen_rects(&rm, &state, area);
        assert_eq!(rects_base.len(), 1);
        let (_, r0) = rects_base[0];

        // Apply char_pan = (5, 3).
        state.char_pan = (5, 3);
        let rects_shifted = room_screen_rects(&rm, &state, area);
        assert_eq!(rects_shifted.len(), 1);
        let (_, r1) = rects_shifted[0];

        assert_eq!(
            (r1.x as i32 - r0.x as i32, r1.y as i32 - r0.y as i32),
            (5, 3),
            "char_pan (5,3) should shift screen rect by exactly (5,3)"
        );
    }

    // ── Item 3: current+selected combined style ───────────────────────────────

    /// When a room is both current AND selected, room_style combines both states:
    /// it returns room_selected with REVERSED added (not just one or the other).
    #[test]
    fn room_style_current_and_selected_combines() {
        use mapper::render::RenderRoom;
        use crate::state::AppState;

        let room = RenderRoom {
            id: 1,
            cell: (0, 0),
            label: "Test".into(),
            is_current: true,
            has_layer_portal: false,
            has_notes: false,
            align_code: String::new(),
        };

        let mut state = AppState::default();
        state.selected_room = Some(1); // room is both current AND selected

        let style = room_style(&room, &state);

        // Must have REVERSED (from the combined path) AND use the selected base.
        assert!(
            style.add_modifier.contains(Modifier::REVERSED),
            "current+selected room must have REVERSED modifier; got {:?}",
            style
        );
        // The base must NOT be room_current alone (which would be REVERSED on its own style).
        // It should be room_selected with REVERSED added.
        let expected = state.colors.room_selected.add_modifier(Modifier::REVERSED);
        assert_eq!(style, expected, "current+selected must equal room_selected + REVERSED");
    }

    /// When a room is current but NOT selected, room_style returns room_current.
    #[test]
    fn room_style_current_only() {
        use mapper::render::RenderRoom;
        use crate::state::AppState;

        let room = RenderRoom {
            id: 2,
            cell: (0, 0),
            label: "Test".into(),
            is_current: true,
            has_layer_portal: false,
            has_notes: false,
            align_code: String::new(),
        };

        let mut state = AppState::default();
        state.selected_room = Some(99); // different room selected

        let style = room_style(&room, &state);
        assert_eq!(style, state.colors.room_current, "current-only room must use room_current style");
    }

    /// When a room is selected but NOT current, room_style returns room_selected.
    #[test]
    fn room_style_selected_only() {
        use mapper::render::RenderRoom;
        use crate::state::AppState;

        let room = RenderRoom {
            id: 3,
            cell: (0, 0),
            label: "Test".into(),
            is_current: false,
            has_layer_portal: false,
            has_notes: false,
            align_code: String::new(),
        };

        let mut state = AppState::default();
        state.selected_room = Some(3);

        let style = room_style(&room, &state);
        assert_eq!(style, state.colors.room_selected, "selected-only room must use room_selected style");
    }

    // ── Item 4: arrow color does not bleed selection bg ───────────────────────

    /// draw_connector_arrows must reset the cell background before applying the
    /// connector fg, so a selection-highlighted room border cell does not keep
    /// the selection bg color after the arrowhead is drawn (non-selected room case).
    #[test]
    fn arrow_style_resets_bg_for_non_selected_room() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        // Pre-paint the cell with a selection bg color to simulate a selected room border.
        let selection_bg = Color::Yellow;
        if let Some(cell) = buf.cell_mut((5, 5)) {
            cell.set_style(Style::new().bg(selection_bg));
        }
        assert_eq!(buf.cell((5, 5)).unwrap().bg, selection_bg);

        // Room 10's arrow; selected_room is None (no selection) — bg must be reset.
        let arrowheads: Vec<Arrowhead> = vec![((5, 5), ">".to_string(), false, false, 10, false)];
        let colors = ColorScheme::terminal_default();
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, None, None);

        let after_bg = buf.cell((5, 5)).unwrap().bg;
        assert_ne!(
            after_bg, selection_bg,
            "arrow draw must reset selection bg; bg is still Yellow after arrow"
        );
    }

    /// draw_connector_arrows must paint the cell background with the selected room's bg color
    /// when the arrow belongs to the currently selected room.
    #[test]
    fn arrow_style_selected_room_gets_room_bg() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        // Use a color scheme where room_selected has a distinct bg.
        let mut colors = ColorScheme::terminal_default();
        let selected_bg = Color::Cyan;
        colors.room_selected = Style::new().fg(Color::White).bg(selected_bg);
        // connector fg is Green so we can check it independently.
        colors.connector = Style::new().fg(Color::Green);

        // Arrow at (5, 5) belongs to room 7; room 7 is the selected room (not current).
        let arrowheads: Vec<Arrowhead> = vec![((5, 5), ">".to_string(), false, false, 7, false)];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, Some(7), None);

        let cell = buf.cell((5, 5)).unwrap();
        assert_eq!(
            cell.bg, selected_bg,
            "selected-room arrow must have the room_selected bg color as background"
        );
        assert_eq!(
            cell.fg,
            Color::Green,
            "selected-room arrow glyph fg must be the connector color"
        );
    }

    /// When the arrow belongs to a room that is BOTH current AND selected, the arrow sits on
    /// the room's border. The border is not reverse-video (only the interior is), so the arrow
    /// background matches the border's plain bg = room_selected.BG.
    #[test]
    fn arrow_style_current_and_selected_uses_reversed_bg() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        let mut colors = ColorScheme::terminal_default();
        // Distinct fg/bg so the reversed-swap is observable.
        colors.room_selected = Style::new().fg(Color::Magenta).bg(Color::Cyan);
        colors.connector = Style::new().fg(Color::Green);

        // Arrow at (5, 5) belongs to room 7; room 7 is BOTH selected AND current.
        let arrowheads: Vec<Arrowhead> = vec![((5, 5), ">".to_string(), false, false, 7, false)];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, Some(7), Some(7));

        let cell = buf.cell((5, 5)).unwrap();
        assert_eq!(
            cell.bg,
            Color::Cyan,
            "current+selected arrow bg must use room_selected.bg (the non-reversed border bg)"
        );
        assert_eq!(cell.fg, Color::Green, "arrow glyph fg must still be the connector color");
    }

    /// When the arrow belongs to the current room that is NOT selected, the arrow sits on the
    /// room's border. Only the interior is reverse-video, so the border (and thus the arrow)
    /// keeps room_current's plain background.
    #[test]
    fn arrow_style_current_only_matches_reversed_room_current_bg() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Modifier, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        let mut colors = ColorScheme::terminal_default();
        // room_current carries REVERSED, but the border it sits on is drawn non-reversed;
        // give it a distinct plain bg so the border background is observable.
        colors.room_current = Style::new().add_modifier(Modifier::REVERSED).fg(Color::Blue).bg(Color::Yellow);
        colors.connector = Style::new().fg(Color::Green);

        // Arrow at (5, 5) belongs to room 7; room 7 is the current room, NOT selected.
        let arrowheads: Vec<Arrowhead> = vec![((5, 5), ">".to_string(), false, false, 7, false)];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, None, Some(7));

        let cell = buf.cell((5, 5)).unwrap();
        assert_eq!(
            cell.bg,
            Color::Yellow,
            "current-only arrow bg must use room_current.bg (the non-reversed border bg)"
        );
        assert_eq!(cell.fg, Color::Green, "arrow glyph fg must still be the connector color");
    }

    /// draw_connector_arrows must NOT apply the selected room's bg to an arrow belonging
    /// to a different (non-selected) room, even when a selection is active.
    #[test]
    fn arrow_style_other_room_unaffected_by_selection() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::{Color, Style};
        use crate::colors::ColorScheme;

        let area = Rect::new(0, 0, 40, 20);
        let mut buf = Buffer::empty(area);

        let mut colors = ColorScheme::terminal_default();
        colors.room_selected = Style::new().fg(Color::White).bg(Color::Cyan);
        colors.connector = Style::new().fg(Color::Green);

        // Arrow belongs to room 5; selected room is 7 — different rooms.
        let arrowheads: Vec<Arrowhead> = vec![((5, 5), ">".to_string(), false, false, 5, false)];
        draw_connector_arrows(&arrowheads, (0, 0), area, &mut buf, &colors, Some(7), None);

        let cell = buf.cell((5, 5)).unwrap();
        assert_ne!(
            cell.bg,
            Color::Cyan,
            "arrow of a non-selected room must not get the selected room bg"
        );
    }

    // ── pulse_border_color ────────────────────────────────────────────────────

    /// At three-quarter period (sin = -1, f = 0) the result is the red endpoint.
    #[test]
    fn pulse_border_color_red_at_three_quarter_period() {
        use std::time::Duration;
        // Three-quarter period: sin = -1, f = 0 → pure red endpoint.
        let three_quarter = Duration::from_secs_f64(3.0 / (4.0 * PULSE_HZ));
        let color = pulse_border_color(three_quarter);
        assert_eq!(color, Color::Rgb(PULSE_RED.0, PULSE_RED.1, PULSE_RED.2),
            "at three-quarter period the border must be the red endpoint");
    }

    /// At quarter period (sin = 1, f = 1) the result is the green endpoint.
    #[test]
    fn pulse_border_color_green_at_quarter_period() {
        use std::time::Duration;
        // Quarter period: sin = 1, f = 1 → pure green endpoint.
        let quarter = Duration::from_secs_f64(1.0 / (4.0 * PULSE_HZ));
        let color = pulse_border_color(quarter);
        assert_eq!(color, Color::Rgb(PULSE_GREEN.0, PULSE_GREEN.1, PULSE_GREEN.2),
            "at quarter period the border must be the green endpoint");
    }

    /// The pulsing border smoke test: with a tidy_job active, the map border cell
    /// style differs from the idle border (which uses the normal focused_border color).
    #[test]
    fn tidy_job_active_border_color_differs_from_idle() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use ratatui::widgets::{Block, Borders};
        use ratatui::prelude::Widget;
        use ratatui::style::Style;
        use std::time::Duration;
        use crate::state::AppState;

        let state = AppState::default();
        let normal_border_color = state.colors.focused_border.fg.unwrap_or(Color::White);

        // At quarter period the pulse is the green endpoint.
        let quarter = Duration::from_secs_f64(1.0 / (4.0 * PULSE_HZ));
        let active_color = pulse_border_color(quarter);

        // The pulsed green color must differ from the normal idle color (Cyan).
        assert_ne!(normal_border_color, active_color,
            "pulsing border color at quarter period must differ from the normal border color");

        // Render smoke: draw a Block with each border style into a TestBackend and
        // verify the border cell fg differs between idle and active.
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();

            // Idle: normal border color.
            let idle_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(normal_border_color));
            idle_block.render(area, buf);
            let idle_cell_fg = buf.cell((0, 0)).map(|c| c.fg).unwrap_or(Color::Reset);

            // Active: pulsing color.
            let active_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(active_color));
            active_block.render(area, buf);
            let active_cell_fg = buf.cell((0, 0)).map(|c| c.fg).unwrap_or(Color::Reset);

            assert_ne!(idle_cell_fg, active_cell_fg,
                "rendered border cell fg must differ when tidy_job is active");
        }).unwrap();
    }

    // ── sound_pulse_color ──────────────────────────────────────────────────────

    #[test]
    fn sound_pulse_full_color_at_start() {
        let beep = Color::Rgb(255, 180, 40);
        let normal = Color::Rgb(0, 0, 0);
        let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(0));
        assert_eq!(c, Some(Color::Rgb(255, 180, 40)), "elapsed 0 => full beep color");
    }

    #[test]
    fn sound_pulse_fades_toward_normal_partway() {
        let beep = Color::Rgb(200, 0, 0);
        let normal = Color::Rgb(0, 0, 0);
        // Halfway through the window: roughly the midpoint between beep and normal.
        let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(SOUND_PULSE_MS / 2));
        match c {
            Some(Color::Rgb(r, _, _)) => assert!((90..=110).contains(&r), "expected ~100, got {r}"),
            other => panic!("expected an Rgb mid-fade color, got {other:?}"),
        }
    }

    #[test]
    fn sound_pulse_expires_after_window() {
        let beep = Color::Rgb(255, 180, 40);
        let normal = Color::Rgb(0, 0, 0);
        let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(SOUND_PULSE_MS));
        assert_eq!(c, None, "at/after the window the pulse is over");
    }

    #[test]
    fn sound_pulse_non_rgb_normal_fades_toward_dim_beep() {
        // When the border color is a named/terminal color (no RGB), fade toward a
        // dimmed copy of the beep color instead (spec fallback).
        let beep = Color::Rgb(200, 200, 200);
        let c = sound_pulse_color(beep, Color::Reset, std::time::Duration::from_millis(SOUND_PULSE_MS - 1));
        match c {
            Some(Color::Rgb(r, _, _)) => assert!(r < 200, "must fade below full beep, got {r}"),
            other => panic!("expected an Rgb color, got {other:?}"),
        }
    }

    // ── Fix 1: render_map_layered layer-strip suppression ─────────────────────

    /// Helper: build a two-layer graph (Hall on MAIN, Cellar peeled to a second layer).
    fn two_layer_graph() -> mapper::graph::MapGraph {
        use mapper::direction::Direction;
        use mapper::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 0));
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::Up, 1);
        mapper::layer::peel_region(&mut g, 2).expect("peel cellar");
        g
    }

    /// With a picture-frame border active (`map_border_style != None`) and 2+ layers,
    /// `render_map_layered` must NOT draw the in-content strip (no lost content row).
    /// The in-content strip uses REVERSED modifier on tab labels; with a border active,
    /// no REVERSED cells should appear in the content area row 0.
    #[test]
    fn render_map_layered_no_in_content_strip_when_border_present() {
        use crate::render::paneframe::BorderStyle;
        let g = two_layer_graph();
        let rm = mapper::render::render_layer(&g, mapper::layer::MAIN_LAYER);

        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);

        // State with a non-None border style (picture-frame).
        let mut state = AppState::default();
        state.colors.map_border_style = BorderStyle::PictureFrame;

        render_map_layered(&rm, &g, &state, area, &mut buf);

        // The strip would write REVERSED style to cells in row 0. With a border active,
        // the strip is suppressed so no REVERSED cells appear in row 0.
        // (render_map does not set REVERSED anywhere in the map content area.)
        let reversed_in_row0 = (area.x..area.right())
            .filter(|&x| {
                buf.cell((x, area.y))
                    .map(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            reversed_in_row0, 0,
            "with a non-None border, the in-content layer strip must NOT be drawn (no REVERSED cells in row 0)"
        );
    }

    /// With `map_border_style == None` and 2+ layers, `render_map_layered` MUST draw
    /// the in-content strip (fallback indicator for the borderless case).
    #[test]
    fn render_map_layered_draws_in_content_strip_when_no_border() {
        use crate::render::paneframe::BorderStyle;
        let g = two_layer_graph();
        let rm = mapper::render::render_layer(&g, mapper::layer::MAIN_LAYER);

        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);

        // State with None border style.
        let mut state = AppState::default();
        state.zoom = crate::state::Zoom::Boxes; // strip requires non-Overview
        state.colors.map_border_style = BorderStyle::None;

        render_map_layered(&rm, &g, &state, area, &mut buf);

        // The strip draws REVERSED on active tab cells in row 0.
        let reversed_in_row0 = (area.x..area.right())
            .filter(|&x| {
                buf.cell((x, area.y))
                    .map(|c| c.modifier.contains(ratatui::style::Modifier::REVERSED))
                    .unwrap_or(false)
            })
            .count();
        assert!(
            reversed_in_row0 > 0,
            "with BorderStyle::None, the in-content layer strip MUST be drawn (REVERSED tab cells expected in row 0)"
        );
    }

    #[test]
    fn room_number_visibility_toggles_id_and_icon_placement() {
        // Build a one-room scene at Boxes zoom with an Out portal stub (mid slot — Up/Down now
        // show their glyph on the connector's border anchor instead, so a non-spatial direction
        // is used here to exercise the still-interior mid-slot icon placement).
        // With show_room_numbers=false (default): the "#<id>" text is absent and the portal icon
        //   appears on interior row 3 (the freed row), centered horizontally.
        // With show_room_numbers=true: "#<id>" appears on interior row 3 and the portal icon
        //   appears on the far-right interior column (col BOX_W-2 = 9), mid-slot row (row 2).
        use mapper::graph::MapGraph;

        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cellar".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::Out, 2);
        let rm = render(&g);

        // Helper: render with a given show_room_numbers value and return the buffer.
        let render_buf = |show_room_numbers: bool| {
            let mut st = AppState::default(); // Boxes zoom, scroll (0,0), show_portal_labels off
            st.show_room_numbers = show_room_numbers;
            let area = Rect::new(0, 0, 80, 40);
            let mut buf = Buffer::empty(area);
            render_map(&rm, &st, area, &mut buf);
            buf
        };

        // ── show_room_numbers = false (default) ──────────────────────────────────
        {
            let buf = render_buf(false);
            let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();

            // Interior row 3 should NOT contain "#1".
            let row3: String = (1u16..=9).map(|x| sym(x, 3)).collect();
            assert!(
                !row3.contains("#1"),
                "show_room_numbers=false: #id must be absent from row 3; got '{row3}'"
            );

            // A portal glyph (⊗ for Out) should appear somewhere on interior row 3.
            let has_out_glyph = (1u16..=9).any(|x| sym(x, 3) == "⊗");
            assert!(
                has_out_glyph,
                "show_room_numbers=false: portal glyph '⊗' must appear on interior row 3; row3='{row3}'"
            );

            // The right interior column (col 9) on rows 1-3 should NOT have the out glyph.
            let right_col_has_glyph = (1u16..=3).any(|y| sym(9, y) == "⊗");
            assert!(
                !right_col_has_glyph,
                "show_room_numbers=false: portal glyph must NOT be in the right column"
            );
        }

        // ── show_room_numbers = true ──────────────────────────────────────────────
        {
            let buf = render_buf(true);
            let sym = |x: u16, y: u16| buf.cell((x, y)).map(|c| c.symbol().to_string()).unwrap_or_default();

            // Interior row 3 should contain "#1".
            let row3: String = (1u16..=9).map(|x| sym(x, 3)).collect();
            assert!(
                row3.contains("#1"),
                "show_room_numbers=true: #id must appear on row 3; got '{row3}'"
            );

            // The Out portal icon should be in the far-right interior column (col 9), mid-slot
            // row (row 2 = by + 1 + 1).
            assert_eq!(
                sym(9, 2), "⊗",
                "show_room_numbers=true: portal glyph '⊗' must be in the right interior column at row 2; got '{}'", sym(9, 2)
            );
        }
    }

    #[test]
    fn build_frame_manifest_drawn_in_map_pane() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use crate::state::{AppState, TidyAnim, TidyFrame};
        use mapper::graph::MapGraph;
        use mapper::layout::TidyStats;

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "Build".into(),
            graph: MapGraph::new(),
            description: "Graph built: 2 rooms, 1 connections".into(),
            stats: TidyStats::default(),
            stage_start: true,
            manifest: Some(vec!["Foyer \u{2192}N\u{2192} Hall".into()]),
        }]));

        // Empty render map, built with the same helper the neighboring tests use.
        let rm = mapper::render::render(&MapGraph::new());
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
        assert!(text.contains("Foyer"), "manifest line should be drawn in the map pane");
        assert!(text.contains("Hall"));
    }

    #[test]
    fn build_frame_manifest_starts_below_tidy_panel() {
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use crate::render::tidy_panel::PANEL_H;
        use crate::state::{AppState, TidyAnim, TidyFrame};
        use mapper::graph::MapGraph;
        use mapper::layout::TidyStats;

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "Build".into(),
            graph: MapGraph::new(),
            description: "Graph built: 2 rooms, 1 connections".into(),
            stats: TidyStats::default(),
            stage_start: true,
            manifest: Some(vec!["Foyer \u{2192}N\u{2192} Hall".into()]),
        }]));

        // Pane large enough for the tidy transport panel (>= PANEL_W x PANEL_H), so the
        // manifest must be offset below the panel rows instead of under it.
        let rm = mapper::render::render(&MapGraph::new());
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);

        let row = |y: u16| -> String {
            (0..area.width)
                .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                .collect()
        };
        for y in 0..PANEL_H {
            assert!(!row(y).contains("Foyer"), "manifest must not be drawn in panel row {y}");
        }
        assert!(row(PANEL_H).contains("Foyer"), "manifest should start at row PANEL_H");
    }

    #[test]
    fn shared_connector_line_uses_shared_path_color() {
        use crate::state::AppState;
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(68, "W".into());
        g.upsert_room(217, "S".into());
        g.set_pos(68, (0, 0));
        g.set_pos(217, (1, 1));
        for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                            (217, Direction::W, 68), (217, Direction::NW, 68)] {
            g.add_edge(o, d, dst);
        }
        let state = AppState::default(); // Boxes zoom by default
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        // At least one cell painted with the shared_path fg color exists (the shared line/arrow).
        // Compared via `cell.fg` (not `cell.style() ==`, which can never match a partially-set
        // Style: ratatui's `Cell::set_style` patches rather than replaces, so `Cell::style()`
        // always synthesizes concrete `bg`/`underline_color`, unlike `shared_path`'s bg: None).
        let shared_fg = state.colors.shared_path.fg.expect("shared_path has an fg color");
        let found = (0..area.width).flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf.cell((x, y)).map(|c| c.fg == shared_fg).unwrap_or(false));
        assert!(found, "the collapsed pair's shared path must paint with shared_path color");
    }

    #[test]
    fn secondary_marker_glyph_drawn_inside_room_in_shared_color() {
        use crate::state::AppState;
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(68, "W".into());
        g.upsert_room(217, "S".into());
        g.set_pos(68, (0, 0));
        g.set_pos(217, (1, 1));
        for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                            (217, Direction::W, 68), (217, Direction::NW, 68)] {
            g.add_edge(o, d, dst);
        }
        let state = AppState::default(); // Boxes zoom by default
        let south = state.symbols.arrows.south.to_string();
        let west = state.symbols.arrows.west.to_string();
        // Compared via `cell.fg` (not `cell.style() ==`), matching
        // `shared_connector_line_uses_shared_path_color` above: `Cell::set_style` patches
        // rather than replaces, so `Cell::style()` never equals a partially-set `Style`.
        let shared_fg = state.colors.shared_path.fg.expect("shared_path has an fg color");
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let has_glyph = |glyph: &str| (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .any(|(x, y)| buf.cell((x, y))
                .map(|c| c.symbol() == glyph && c.fg == shared_fg).unwrap_or(false));
        assert!(has_glyph(&south), "S secondary marker present in shared color");
        assert!(has_glyph(&west), "W secondary marker present in shared color");
    }

    #[test]
    fn secondary_marker_sits_next_to_the_retained_arrowhead() {
        // Regression: the marker must hug the retained connector's arrowhead. The house-ring
        // connectors are DIAGONAL, so the arrowhead sits on a box CORNER; the marker must anchor
        // to that corner (one cell inward), not to a side midpoint (which strands it inside the
        // room — the bug this test guards).
        use crate::state::AppState;
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        g.upsert_room(68, "W".into());
        g.upsert_room(217, "S".into());
        g.set_pos(68, (0, 0));
        g.set_pos(217, (1, 1));
        for (o, d, dst) in [(68, Direction::S, 217), (68, Direction::SE, 217),
                            (217, Direction::W, 68), (217, Direction::NW, 68)] {
            g.add_edge(o, d, dst);
        }
        let state = AppState::default();
        let se = state.symbols.arrows.se.to_string();       // retained SE departure arrowhead (at 68)
        let south = state.symbols.arrows.south.to_string(); // S secondary marker (at 68)
        let shared_fg = state.colors.shared_path.fg.expect("shared_path has an fg color");
        let rm = mapper::render::render(&g);
        let area = Rect::new(0, 0, 60, 30);
        let mut buf = Buffer::empty(area);
        render_map(&rm, &state, area, &mut buf);
        let find = |glyph: &str| (0..area.width as i32)
            .flat_map(|x| (0..area.height as i32).map(move |y| (x, y)))
            .find(|&(x, y)| buf.cell((x as u16, y as u16))
                .map(|c| c.symbol() == glyph && c.fg == shared_fg).unwrap_or(false));
        let arrow = find(&se).expect("SE departure arrowhead present");
        let marker = find(&south).expect("S secondary marker present");
        let (dx, dy) = ((arrow.0 - marker.0).abs(), (arrow.1 - marker.1).abs());
        assert!(dx <= 1 && dy <= 1 && (dx, dy) != (0, 0),
            "S marker must sit adjacent to the SE arrowhead, got arrow={arrow:?} marker={marker:?}");
    }

    #[test]
    fn house_ring_collapses_to_clean_lines_with_no_illegal_overlap() {
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        for (id, p) in [(143, (1, 2)), (89, (2, 3)), (217, (1, 4)), (68, (0, 3))] {
            g.upsert_room(id, "r".into());
            g.set_pos(id, p);
        }
        // Diamond ring: each adjacent pair reachable by a cardinal AND a diagonal, both ways.
        let edges = [
            (68, Direction::N, 143), (68, Direction::NE, 143),
            (143, Direction::S, 68), (143, Direction::SW, 68),
            (143, Direction::E, 89), (143, Direction::SE, 89),
            (89, Direction::W, 143), (89, Direction::NW, 143),
            (89, Direction::S, 217), (89, Direction::SW, 217),
            (217, Direction::N, 89), (217, Direction::NE, 89),
            (217, Direction::W, 68), (217, Direction::NW, 68),
            (68, Direction::S, 217), (68, Direction::SE, 217),
        ];
        for (o, d, dst) in edges { g.add_edge(o, d, dst); }
        let plan = mapper::route::route_lanes(&g);
        // One compass connector per ring pair.
        for pair in [(68, 143), (89, 143), (89, 217), (68, 217)] {
            let n = plan.connectors.iter()
                .filter(|c| (c.origin.min(c.dest), c.origin.max(c.dest)) == pair
                    && mapper::direction::grid_offset(c.exit_dir).is_some())
                .count();
            assert_eq!(n, 1, "pair {pair:?} must collapse to one compass connector");
        }
        // No illegal overlaps in the rendered result.
        assert_eq!(render_overlap_stats(&g).0, 0, "ring must render without illegal overlap");
    }
}
