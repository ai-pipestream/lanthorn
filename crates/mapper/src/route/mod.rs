//! Logical lane router: routes each drawn edge through reserved lanes in the gaps
//! between rooms, on a doubled-coordinate gap lattice (room cell (c,r) -> (2c,2r);
//! channels live on odd coordinates). Pixel-free — emits lane indices + per-channel
//! lane counts that the renderer turns into gap widths.

use std::collections::BTreeMap;
use crate::direction::{grid_offset, layout_offset, opposite, Direction};
use crate::graph::{Connection, MapGraph, RoomId};
use crate::router::{route_side, Side};

/// A routing channel: `H(r)` is the horizontal gap below room-row `r` (line y=2r+1);
/// `V(c)` is the vertical gap right of room-column `c` (line x=2c+1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel { H(i32), V(i32) }

/// One laned long-run of a connector inside a channel. `start<=end` is the doubled-coord
/// extent along the channel's free axis (x for H, y for V).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaneSeg { pub channel: Channel, pub lane: u16, pub start: i32, pub end: i32 }

/// A fully-routed connector (one per drawn connection; reciprocal pairs collapsed).
#[derive(Debug, Clone)]
pub struct RoutedConnector {
    pub origin: RoomId,
    pub dest: RoomId,
    pub distorted: bool,
    pub exit: Side,
    pub entry: Side,
    pub points: Vec<(i32, i32)>, // doubled-coord polyline, centre→…→centre
    pub segs: Vec<LaneSeg>,      // laned long-runs (filled by lane assignment)
    /// Slot index of this connector's exit anchor among all connectors sharing the
    /// origin room's exit side (0,1,2,…). Used by the renderer to offset the departure
    /// anchor along the box edge so two connectors on one side don't collide.
    pub exit_slot: u16,
    /// Slot index of this connector's entry anchor among all connectors sharing the
    /// destination room's entry side.
    pub entry_slot: u16,
    /// true when this connector represents a collapsed true-opposite pair
    /// `a→dir→b` + `b→opposite(dir)→a`; the renderer draws a far-end arrow only for these.
    pub reciprocal: bool,
    /// The origin edge's compass direction (so the renderer can pick a diagonal corner).
    pub exit_dir: crate::direction::Direction,
    /// The paired back-edge's compass direction, set only when a bidirectional pairing
    /// collapsed into this connector (used for the far-end diagonal corner). `None` otherwise.
    pub entry_dir: Option<crate::direction::Direction>,
    /// True for a multi-edge MERGE STUB: an extra edge between an already-connected pair whose
    /// polyline routes from its box-edge exit and ENDS on the trunk (a T-junction) instead of
    /// drawing its own line to the destination. The renderer draws only its departure arrow.
    pub merge: bool,
}

/// The logical route plan: connectors plus per-channel lane counts.
#[derive(Debug, Clone, Default)]
pub struct RoutePlan {
    pub connectors: Vec<RoutedConnector>,
    pub h_lanes: BTreeMap<i32, u16>,
    pub v_lanes: BTreeMap<i32, u16>,
}

/// Room cell (c,r) → doubled-coordinate centre (2c, 2r).
pub fn cell_to_doubled(cell: (i32, i32)) -> (i32, i32) {
    (cell.0 * 2, cell.1 * 2)
}

/// One doubled-step out of the box on `side` (the exit/entry stub point).
pub fn exit_point(cell: (i32, i32), side: Side) -> (i32, i32) {
    let (x, y) = cell_to_doubled(cell);
    match side {
        Side::Right => (x + 1, y),
        Side::Left => (x - 1, y),
        Side::Top => (x, y - 1),
        Side::Bottom => (x, y + 1),
    }
}

/// Pick the destination's entry side: the side of `b_cell` geometrically facing
/// `a_cell` (dominant axis of the offset; horizontal wins ties).
fn entry_side(a_cell: (i32, i32), b_cell: (i32, i32)) -> Side {
    let dx = a_cell.0 - b_cell.0;
    let dy = a_cell.1 - b_cell.1;
    if dx.abs() >= dy.abs() {
        if dx >= 0 { Side::Right } else { Side::Left }
    } else if dy >= 0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// Snap a stub point `p` (one even coord when the side is vertical/horizontal) onto the
/// all-odd gap lattice by stepping its even axis toward `toward` (the other endpoint).
fn snap_to_lattice(p: (i32, i32), toward: (i32, i32)) -> (i32, i32) {
    let sx = if p.0 % 2 == 0 { if toward.0 >= p.0 { 1 } else { -1 } } else { 0 };
    let sy = if p.1 % 2 == 0 { if toward.1 >= p.1 { 1 } else { -1 } } else { 0 };
    (p.0 + sx, p.1 + sy)
}

/// L-orientation of a connector's gap-lattice route: which leg comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Orient { HorizontalFirst, VerticalFirst }

/// Build the doubled-coord polyline for one connector: centre → exit stub → lattice →
/// horizontal run → vertical run → lattice → entry stub → centre. Horizontal-first by
/// default; see `build_points_orient` to choose the other L.
fn build_points(a_cell: (i32, i32), exit: Side, b_cell: (i32, i32), entry: Side) -> Vec<(i32, i32)> {
    build_points_orient(a_cell, exit, b_cell, entry, Orient::HorizontalFirst)
}

/// Like `build_points` but lets the caller pick the L orientation. Both orientations keep
/// the gap-lattice invariant (long runs on odd coords, never through a room) and the
/// `ea==eb` adjacent-straight short-circuit. Horizontal-first corners at `(gb.x, ga.y)`;
/// vertical-first corners at `(ga.x, gb.y)`.
fn build_points_orient(
    a_cell: (i32, i32),
    exit: Side,
    b_cell: (i32, i32),
    entry: Side,
    orient: Orient,
) -> Vec<(i32, i32)> {
    let ca = cell_to_doubled(a_cell);
    let cb = cell_to_doubled(b_cell);
    let ea = exit_point(a_cell, exit);
    let eb = exit_point(b_cell, entry);
    // Adjacent facing rooms: the exit stub and entry stub are the SAME lattice cell
    // (the shared doorway between the two boxes). Route straight through it with no
    // lattice dip — there is no channel run, hence no lane. Dipping into the channel
    // and back to the same point would draw a dangling out-and-back tail.
    if ea == eb {
        return vec![ca, ea, cb];
    }
    let ga = snap_to_lattice(ea, cb); // first all-odd point
    let gb = snap_to_lattice(eb, ca); // last all-odd point
    // L on the gap lattice. Horizontal-first: ga → (gb.x, ga.y) → gb. Vertical-first:
    // ga → (ga.x, gb.y) → gb. Both stay on the all-odd lattice (ga, gb both all-odd).
    let corner = match orient {
        Orient::HorizontalFirst => (gb.0, ga.1),
        Orient::VerticalFirst => (ga.0, gb.1),
    };
    let mut pts = vec![ca, ea, ga];
    if corner != ga { pts.push(corner); }
    if gb != corner { pts.push(gb); }
    pts.push(eb);
    pts.push(cb);
    // Drop consecutive duplicates.
    pts.dedup();
    // Merge collinear consecutive points: a degenerate L (zero-length leg) leaves three
    // collinear points, which would split one straight channel run into two windows and
    // hence two LaneSegs that could be assigned different lanes — drawing the single line
    // as a diagonal. Collapse them so each straight run is one segment.
    merge_collinear(&mut pts);
    pts
}

/// Build a multi-edge MERGE STUB polyline (doubled coords): from origin `a`'s `exit` side, route
/// orthogonally to the trunk's junction point (`trunk[len-2]`, the last point before the trunk's
/// destination centre) and END there, so the stub terminates ON the trunk as a T-junction rather
/// than drawing its own line to the destination. Returns `centre → exit-stub → [corner] → junction`.
fn merge_stub_points(a: (i32, i32), exit: Side, trunk: &[(i32, i32)]) -> Option<Vec<(i32, i32)>> {
    if trunk.len() < 2 {
        return None;
    }
    let junction = trunk[trunk.len() - 2];
    let ca = cell_to_doubled(a);
    let ea = exit_point(a, exit);
    // Horizontal-first L: move in x to the junction's column at the exit-stub's row, then in y.
    // The corner lands in a gutter (away from the origin box) for the cardinal exit sides.
    let corner = (junction.0, ea.1);
    let mut pts = vec![ca, ea];
    if corner != ea && corner != junction {
        pts.push(corner);
    }
    if *pts.last().unwrap() != junction {
        pts.push(junction);
    }
    pts.dedup();
    merge_collinear(&mut pts);
    if pts.len() < 2 {
        return None;
    }
    Some(pts)
}

/// The room cells a straight axis-aligned segment from `p` to `q` passes through (inclusive).
fn line_cells(p: (i32, i32), q: (i32, i32)) -> Vec<(i32, i32)> {
    let dx = (q.0 - p.0).signum();
    let dy = (q.1 - p.1).signum();
    let mut v = vec![p];
    let mut c = p;
    while c != q {
        c = (c.0 + dx, c.1 + dy);
        v.push(c);
    }
    v
}

/// Try to route `a → b` as a DIRECT path along the room grid (no channel dip): leave `a` on its
/// `exit` side, run straight to the single turning corner, then straight to `b`. The exit side
/// fixes the first leg's axis (East/West → run horizontally to b's column then turn; North/South
/// → run vertically to b's row then turn), so the corner is determined. Returns the doubled-coord
/// polyline (on even, room-grid coords — carries no lane) plus the entry side, but ONLY when the
/// exit faces toward `b` and EVERY cell both legs pass through (besides `a`/`b`) is empty.
/// Returns None when the path is blocked or the exit points away from `b` — then the caller dips
/// into the lane channels. Subsumes the straight (zero-turn) case when `b` is on `a`'s row/column.
fn direct_route(
    a: (i32, i32),
    exit: Side,
    b: (i32, i32),
    occupied: &std::collections::BTreeSet<(i32, i32)>,
) -> Option<(Side, Vec<(i32, i32)>)> {
    use std::cmp::Ordering::{Equal, Greater, Less};
    // `corner` is where the path turns; `entry` is the side of `b` it arrives on.
    let (corner, entry) = match exit {
        Side::Right => {
            if b.0 <= a.0 { return None; } // exit east but b not east → would route backwards
            ((b.0, a.1), match b.1.cmp(&a.1) { Greater => Side::Top, Less => Side::Bottom, Equal => Side::Left })
        }
        Side::Left => {
            if b.0 >= a.0 { return None; }
            ((b.0, a.1), match b.1.cmp(&a.1) { Greater => Side::Top, Less => Side::Bottom, Equal => Side::Right })
        }
        Side::Bottom => {
            if b.1 <= a.1 { return None; }
            ((a.0, b.1), match b.0.cmp(&a.0) { Greater => Side::Left, Less => Side::Right, Equal => Side::Top })
        }
        Side::Top => {
            if b.1 >= a.1 { return None; }
            ((a.0, b.1), match b.0.cmp(&a.0) { Greater => Side::Left, Less => Side::Right, Equal => Side::Bottom })
        }
    };
    // Every cell on both legs except the endpoints must be empty.
    let blocked = line_cells(a, corner)
        .into_iter()
        .chain(line_cells(corner, b))
        .any(|c| c != a && c != b && occupied.contains(&c));
    if blocked {
        return None;
    }
    let mut pts = vec![cell_to_doubled(a), exit_point(a, exit)];
    if corner != b {
        pts.push(cell_to_doubled(corner)); // the single turn; absent when b is aligned (straight)
    }
    pts.push(exit_point(b, entry));
    pts.push(cell_to_doubled(b));
    pts.dedup(); // adjacent rooms collapse the two stub points into one shared doorway
    Some((entry, pts))
}

/// Remove interior points that lie on the straight line between their neighbours (same x
/// or same y on both sides), so each maximal straight run is a single polyline segment.
fn merge_collinear(pts: &mut Vec<(i32, i32)>) {
    let mut i = 1;
    while i + 1 < pts.len() {
        let (a, b, c) = (pts[i - 1], pts[i], pts[i + 1]);
        let collinear = (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1);
        if collinear {
            pts.remove(i);
        } else {
            i += 1;
        }
    }
}

/// Count how many times polyline `a` crosses polyline `b`: a lattice cell where a HORIZONTAL
/// run of one polyline passes through the INTERIOR of a VERTICAL run of the other (or vice
/// versa). The crossing cell must be the strict interior of the run it cuts ACROSS — so a
/// connector's turning corner landing mid-span of another connector's straight run counts (it
/// renders as a `┼`), while two connectors merely touching corner-to-corner (each at its own
/// run end) does not. Room-exit stubs are already excluded by `long_runs`. This is the metric
/// the greedy router minimizes.
fn count_crossings(a: &[(i32, i32)], b: &[(i32, i32)]) -> usize {
    // A horizontal run (covering x∈[hs,he] at y=hy) cuts ACROSS a vertical run (at x=vx,
    // covering y∈[vs,ve]) when the horizontal run reaches x=vx (closed) AND hy is in the
    // STRICT interior of the vertical run's y-extent (the vertical run passes straight
    // through the crossing cell). Counting it for the run that passes straight through means
    // corner-on-interior crossings are caught without double-counting the symmetric case.
    fn hv_cross(h: &[(Channel, i32, i32)], v: &[(Channel, i32, i32)]) -> usize {
        let mut n = 0;
        for &(hc, hs, he) in h {
            let Channel::H(r) = hc else { continue };
            let hy = 2 * r + 1;
            for &(vc, vs, ve) in v {
                let Channel::V(c) = vc else { continue };
                let vx = 2 * c + 1;
                // horizontal reaches vx (closed) and cuts the vertical run's interior in y.
                if hs <= vx && vx <= he && vs < hy && hy < ve { n += 1; }
            }
        }
        n
    }
    let ra = long_runs(a);
    let rb = long_runs(b);
    hv_cross(&ra, &rb) + hv_cross(&rb, &ra)
}

/// True if polylines `a` and `b` share a PARALLEL collinear overlap that the lane system
/// cannot separate into a clean crossing: two runs on the SAME channel (same H-line or same
/// V-line) whose extents overlap by more than a single shared turning corner. Such an
/// overlap renders as a line-on-line stomp (the renderer's no-overlap gate rejects it), so a
/// greedy candidate that introduces one is disqualified.
fn has_parallel_overlap(a: &[(i32, i32)], b: &[(i32, i32)]) -> bool {
    let ra = long_runs(a);
    let rb = long_runs(b);
    for &(ca, sa, ea) in &ra {
        for &(cb, sb, eb) in &rb {
            if ca == cb {
                // overlap length of the two closed extents on the shared channel line
                let lo = sa.max(sb);
                let hi = ea.min(eb);
                if hi - lo >= 1 { return true; } // >1 cell of shared collinear run
            }
        }
    }
    false
}

/// Every straight collinear run of a polyline as `(is_horizontal, line, start, end)`, over ALL grid
/// lines — room rows/columns (even coords) AND channels (odd). Unlike `long_runs` (channels only),
/// this also sees DIRECT routes, which run on room grid lines and carry no lane.
fn all_runs(points: &[(i32, i32)]) -> Vec<(bool, i32, i32, i32)> {
    let mut runs = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a.1 == b.1 && a.0 != b.0 {
            runs.push((true, a.1, a.0.min(b.0), a.0.max(b.0)));
        } else if a.0 == b.0 && a.1 != b.1 {
            runs.push((false, a.0, a.1.min(b.1), a.1.max(b.1)));
        }
    }
    runs
}

/// True if two polylines share a parallel collinear run of more than one cell on the same grid line
/// — a line-on-line stomp the renderer can't separate. Sees room-line runs too (see `all_runs`), so
/// it catches two DIRECT routes hugging the same room row/column, which `has_parallel_overlap` (which
/// only inspects laneable channel runs) misses.
///
/// Only each route's INTERIOR is compared — the first and last segment are the room-exit doorway
/// stubs. Two connectors entering the same room legitimately share its doorway (rendered as parallel
/// arrows out one box side), so that shared stub must not read as a stomp.
fn polylines_overlap(a: &[(i32, i32)], b: &[(i32, i32)]) -> bool {
    let ai: &[(i32, i32)] = if a.len() > 2 { &a[1..a.len() - 1] } else { &[] };
    let bi: &[(i32, i32)] = if b.len() > 2 { &b[1..b.len() - 1] } else { &[] };
    for &(ah, al, a_s, a_e) in &all_runs(ai) {
        for &(bh, bl, b_s, b_e) in &all_runs(bi) {
            if ah == bh && al == bl && a_s.max(b_s) < a_e.min(b_e) {
                return true; // >1 cell of shared collinear run on the same line
            }
        }
    }
    false
}

/// The destination side a ONE-WAY connector arrives on, fixed by its compass direction so the
/// arrow lands on the matching side regardless of the rooms' exact offset. A cardinal enters the
/// side opposite the one it left — it reads as travelling straight through (N enters from the
/// south, S from the north, E from the west, W from the east). A diagonal enters a chosen cardinal
/// side: NW→N, NE→E, SE→S, SW→W. Up/Down enter opposite the side they departed, same as a
/// cardinal (Up→Bottom, Down→Top). The remaining non-planar directions (In/Out/Unknown, already
/// filtered from `compass`) → None.
fn oneway_entry_side(dir: Direction) -> Option<Side> {
    Some(match dir {
        Direction::N => Side::Bottom,
        Direction::S => Side::Top,
        Direction::E => Side::Left,
        Direction::W => Side::Right,
        Direction::NW => Side::Top,
        Direction::NE => Side::Right,
        Direction::SE => Side::Bottom,
        Direction::SW => Side::Left,
        Direction::Up => Side::Bottom,
        Direction::Down => Side::Top,
        Direction::In | Direction::Out | Direction::Unknown => return None,
    })
}

/// The candidate entry sides for a NON-reciprocal connector from `a` to `b`: the
/// geometric entry side plus the next-nearest side that still faces the origin, in a
/// deterministic order. The geometric side is always first (the default), so when no
/// alternative reduces crossings the chosen route is unchanged.
fn entry_side_alternatives(a_cell: (i32, i32), b_cell: (i32, i32)) -> Vec<Side> {
    let primary = entry_side(a_cell, b_cell);
    let dx = a_cell.0 - b_cell.0;
    let dy = a_cell.1 - b_cell.1;
    // The secondary side is the dominant-of-the-other-axis side that still faces the
    // origin. Only offer it when the origin is genuinely off-axis on that axis.
    let secondary = if dx.abs() >= dy.abs() {
        if dy > 0 { Some(Side::Bottom) } else if dy < 0 { Some(Side::Top) } else { None }
    } else if dx > 0 { Some(Side::Right) } else if dx < 0 { Some(Side::Left) } else { None };
    let mut out = vec![primary];
    if let Some(s) = secondary { if s != primary { out.push(s); } }
    out
}

/// Route every drawn (compass) edge into a connector polyline. True reciprocal pairs
/// (`a→dir→b` + `b→opposite(dir)→a`) collapse to one connector; which member is kept
/// depends on insertion order in `connections()` (the first-seen direction is drawn).
pub fn route_topology(graph: &MapGraph) -> Vec<RoutedConnector> {
    // Build the connectors two ways and keep the one with fewer total crossings:
    //   - `default`: every connector on its canonical horizontal-first / geometric-entry route;
    //   - `greedy`: each connector picks, sequentially, the candidate route that crosses the
    //     fewest ALREADY-placed connectors.
    // Greedy is a local heuristic and can occasionally do worse than the canonical layout on
    // a given graph; keeping the better of the two guarantees crossing reduction is never a
    // regression. Both are deterministic, so the tiebreak (prefer `default` on an exact tie)
    // keeps the whole routine deterministic.
    let default = route_topology_with(graph, false);
    // Accept the greedy reroute only if it BOTH lowers the render-faithful crossing total AND
    // introduces NO new parallel line-on-line overlap pair beyond those already present (and
    // handled cleanly) in the canonical `default` layout. The default is the long-standing
    // overlap-free render, so any overlap it already contains is renderer-safe; greedy must
    // not add a new one. Otherwise we keep `default`. This makes crossing reduction a strict,
    // never-regressing improvement and keeps the renderer's no-overlap gate green.
    let greedy = route_topology_with(graph, true);
    if total_crossings(&greedy) < total_crossings(&default)
        && !greedy_adds_overlap(&default, &greedy)
    {
        greedy
    } else {
        default
    }
}

/// A dirty shared cell: the lattice cell plus the sorted identities of the connectors that
/// collide there, used to compare overlaps across two candidate layouts.
type DirtyCell = ((i32, i32), Vec<(RoomId, RoomId)>);

/// Every lattice cell shared by ≥2 connectors that is NOT a clean perpendicular crossing
/// (a single horizontal pass-through + a single vertical pass-through). The full polyline
/// trace is walked, so room-exit stubs and turning corners are included — exactly the cells
/// where a parallel run, corner-on-corner, T-stomp, or ≥3-way share would render as an
/// illegal overlap. Returned keyed by the involved connectors' `(origin,dest)` identities so
/// the two layouts can be compared connector-for-connector.
fn dirty_shared_cells(conns: &[RoutedConnector]) -> std::collections::BTreeSet<DirtyCell> {
    use std::collections::{BTreeSet, HashMap};
    const N: u8 = 1; const S: u8 = 2; const E: u8 = 4; const W: u8 = 8;
    let mut cells: HashMap<(i32, i32), HashMap<usize, u8>> = HashMap::new();
    for (ci, c) in conns.iter().enumerate() {
        for w in c.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dx = (b.0 - a.0).signum();
            let dy = (b.1 - a.1).signum();
            let bit = if dx > 0 { E } else if dx < 0 { W } else if dy > 0 { S } else { N };
            let mut cur = a;
            loop {
                *cells.entry(cur).or_default().entry(ci).or_insert(0) |= bit;
                if cur == b { break; }
                cur = (cur.0 + dx, cur.1 + dy);
            }
        }
    }
    let mut dirty = BTreeSet::new();
    let (horiz, vert) = (E | W, N | S);
    for (cell, per_conn) in &cells {
        if per_conn.len() < 2 { continue; }
        let masks: Vec<u8> = per_conn.values().copied().collect();
        // A clean perpendicular crossing: exactly two connectors, one straight-horizontal and
        // one straight-vertical.
        let clean_cross = per_conn.len() == 2
            && ((masks[0] == horiz && masks[1] == vert) || (masks[0] == vert && masks[1] == horiz));
        // A pure-parallel overlap: every contributor is the SAME single straight orientation
        // (all horizontal or all vertical, none turning here). The lane system separates these
        // onto distinct pixel lines, so they are NOT a render overlap.
        let all_h = masks.iter().all(|&m| m == E || m == W || m == horiz);
        let all_v = masks.iter().all(|&m| m == N || m == S || m == vert);
        let parallel_lane_separable = all_h || all_v;
        if !clean_cross && !parallel_lane_separable {
            let mut ids: Vec<(RoomId, RoomId)> =
                per_conn.keys().map(|&ci| (conns[ci].origin, conns[ci].dest)).collect();
            ids.sort();
            dirty.insert((*cell, ids));
        }
    }
    dirty
}

/// True if `greedy` introduces a dirty (non-clean-crossing) shared cell that the canonical
/// `default` layout does not already contain. The default is the long-standing overlap-free
/// render, so any dirty share it already has is renderer-safe; greedy must not add a new one.
/// Compared by `(cell, involved-connector-identities)` so a pre-existing safe share is not
/// counted against greedy.
fn greedy_adds_overlap(default: &[RoutedConnector], greedy: &[RoutedConnector]) -> bool {
    let base = dirty_shared_cells(default);
    dirty_shared_cells(greedy).iter().any(|d| !base.contains(d))
}

/// Render-faithful total crossing count for a set of connectors: lanes are assigned exactly
/// as the renderer does, then a `┼` is counted wherever a horizontal run of one connector and
/// a vertical run of a DIFFERENT connector meet at a lattice cell (closed extents — corners
/// included), since after lane offsetting two parallel runs sit on distinct pixel lines and a
/// perpendicular crosser cuts each. This mirrors the renderer's per-cell `┼` detection, so
/// minimizing it tracks the visible crossing count rather than a raw-lattice proxy.
fn total_crossings(conns: &[RoutedConnector]) -> usize {
    // (connector idx, channel, start, end) for every long run, with a stable run id.
    let mut runs: Vec<(usize, Channel, i32, i32)> = Vec::new(); // (run id, channel, s, e)
    let mut owner: Vec<usize> = Vec::new(); // run id → connector idx
    for (ci, c) in conns.iter().enumerate() {
        for (ch, s, e) in long_runs(&c.points) {
            runs.push((owner.len(), ch, s, e));
            owner.push(ci);
        }
    }
    let (lane_of, _counts) = assign_lanes(&runs);
    // Split runs into horizontals and verticals, carrying connector idx and lane.
    let mut horiz: Vec<(usize, u16, i32, i32, i32)> = Vec::new(); // (conn, lane, y, xs, xe)
    let mut vert: Vec<(usize, u16, i32, i32, i32)> = Vec::new(); // (conn, lane, x, ys, ye)
    for &(id, ch, s, e) in &runs {
        let lane = lane_of[&id];
        match ch {
            Channel::H(r) => horiz.push((owner[id], lane, 2 * r + 1, s, e)),
            Channel::V(c) => vert.push((owner[id], lane, 2 * c + 1, s, e)),
        }
    }
    let mut n = 0;
    for &(hc, _hl, hy, hxs, hxe) in &horiz {
        for &(vc, _vl, vx, vys, vye) in &vert {
            if hc == vc { continue; } // a connector never crosses itself
            // Closed-extent perpendicular meeting: the horizontal spans vx and the vertical
            // spans hy. Distinct connectors meeting here render as a single ┼.
            if hxs <= vx && vx <= hxe && vys <= hy && hy <= vye {
                n += 1;
            }
        }
    }
    n
}

/// The unconsumed reverse edge (b→a) paired with edge `ci`, preferring the exact compass-opposite
/// so the trunk keeps its natural straight / single-L shape, falling back to the first reverse edge.
///
/// Up/Down edges pair ONLY with the exact opposite Up/Down edge between the same two rooms
/// (`Up`'s only possible back-edge is `Down`, and vice versa — there is no "first reverse edge"
/// fallback since that single candidate IS the exact opposite). A matching pair collapses into
/// one reciprocal connector (SQ-0216 revises Task 6's "never collapse" decision after visual
/// testing). An Up/Down `c` never pairs with a compass edge, and a compass `c` never pairs with
/// an Up/Down edge either (the fallback would otherwise pair e.g. `1→N→2` with an unrelated
/// `2→Down→1`, which are not geometrically opposite) — compass pairing stays byte-identical.
fn back_edge_idx(
    compass: &[&Connection],
    ci: usize,
    c: &Connection,
    consumed: &std::collections::BTreeSet<usize>,
) -> Option<usize> {
    if matches!(c.dir, Direction::Up | Direction::Down) {
        return compass
            .iter()
            .enumerate()
            .find(|&(pi, p)| {
                pi != ci
                    && !consumed.contains(&pi)
                    && p.origin == c.dest
                    && p.dest == c.origin
                    && p.dir == opposite(c.dir)
            })
            .map(|(pi, _)| pi);
    }
    let is_back = |pi: usize, p: &Connection| {
        pi != ci
            && !consumed.contains(&pi)
            && p.origin == c.dest
            && p.dest == c.origin
            && !matches!(p.dir, Direction::Up | Direction::Down)
    };
    compass
        .iter()
        .enumerate()
        .find(|&(pi, p)| is_back(pi, p) && p.dir == opposite(c.dir))
        .or_else(|| compass.iter().enumerate().find(|&(pi, p)| is_back(pi, p)))
        .map(|(pi, _)| pi)
}

/// Manhattan length of a polyline in doubled coords — used to rank competing direct routes.
fn path_len(pts: &[(i32, i32)]) -> i32 {
    pts.windows(2).map(|w| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs()).sum()
}

/// Decide which connectors may keep a straight DIRECT route when several would collide on the same
/// room line. Two direct routes on one room line cannot be separated (a direct route carries no
/// lane), so at most one survives; the LONGER one wins (its detour through channels would be the
/// ugliest) and the shorter ones are returned as "losers" that must weave instead. Without this the
/// outcome is order-dependent — whichever edge is listed first keeps the line — which can force a
/// long path to weave around a short one (e.g. 76↔78 weaving aside for the short 180↔78).
fn direct_route_losers(
    compass: &[&Connection],
    graph: &MapGraph,
    occupied: &std::collections::BTreeSet<(i32, i32)>,
) -> std::collections::BTreeSet<usize> {
    // One representative per room pair — the first listed edge, matching where the main loop attempts
    // the pair's direct route (reverse/extra edges are consumed or become merge stubs there).
    let mut seen_pairs: std::collections::BTreeSet<(RoomId, RoomId)> = std::collections::BTreeSet::new();
    let empty = std::collections::BTreeSet::new();
    let mut cands: Vec<(usize, Vec<(i32, i32)>)> = Vec::new();
    for (ci, c) in compass.iter().enumerate() {
        let pair = (c.origin.min(c.dest), c.origin.max(c.dest));
        if !seen_pairs.insert(pair) {
            continue;
        }
        let (Some(a), Some(b)) = (graph.room(c.origin).and_then(|r| r.pos),
                                  graph.room(c.dest).and_then(|r| r.pos)) else { continue };
        let Some(exit) = route_side(c.dir) else { continue };
        let required_entry =
            back_edge_idx(compass, ci, c, &empty).and_then(|pi| route_side(compass[pi].dir));
        if let Some((sentry, pts)) = direct_route(a, exit, b, occupied) {
            if required_entry.is_none_or(|req| req == sentry) {
                cands.push((ci, pts));
            }
        }
    }
    // Longest first (index breaks ties deterministically); a candidate that collides with an already
    // accepted winner becomes a loser.
    cands.sort_by(|x, y| path_len(&y.1).cmp(&path_len(&x.1)).then(x.0.cmp(&y.0)));
    let mut accepted: Vec<Vec<(i32, i32)>> = Vec::new();
    let mut losers = std::collections::BTreeSet::new();
    for (ci, pts) in cands {
        if accepted.iter().any(|w| polylines_overlap(&pts, w)) {
            losers.insert(ci);
        } else {
            accepted.push(pts);
        }
    }
    losers
}

/// Route every drawn (compass) edge. When `greedy` is false, each connector takes its
/// canonical route (horizontal-first L, geometric entry side). When true, each connector is
/// routed sequentially and picks — among its candidate routes (both L orientations, plus the
/// alternative facing entry side for non-reciprocal connectors) — the one that crosses the
/// fewest already-placed connectors, with a deterministic integer tiebreak.
fn route_topology_with(graph: &MapGraph, greedy: bool) -> Vec<RoutedConnector> {
    // Draw every compass edge on its OWN exit side, collapsing ONE bidirectional pairing
    // (a→b together with a single b→a, regardless of the two directions) into a single
    // connector: the renderer puts an arrow at each end, each pointing out that end's own
    // compass side. An exact-opposite pair is the straight-line special case. ADDITIONAL
    // edges between the same room pair (e.g. a third direction, 239→S→77 alongside
    // 77→E→239 and 239→N→77) stay separate, so every distinct direction remains visible.
    // Unordered room pairs that already have a compass edge (grid_offset is Some — the true
    // 8-way compass directions, unlike layout_offset which also covers Up/Down). When the same
    // pair also has an up/down edge, the compass path is the only one drawn: the up/down edge
    // still contributes its layout hint elsewhere, but produces no routed connector here. Computed
    // BEFORE the working-set slice below so the up/down edge can be excluded from the working set
    // itself, rather than skipped mid-loop — a mid-loop skip would still leave the up/down edge in
    // the `compass` slice fed to `direct_route_losers`, which picks the FIRST-LISTED edge per pair
    // as that pair's representative for the longest-straight-line contest; a suppressed up/down
    // edge listed first would steal that slot from the real compass edge, corrupting which
    // connector wins a contested direct room-line and breaking the "compass routing stays
    // byte-identical" invariant (SQ-0219 fix).
    let compass_pairs: std::collections::BTreeSet<(RoomId, RoomId)> = graph
        .connections()
        .iter()
        .filter(|c| grid_offset(c.dir).is_some())
        .map(|c| (c.origin.min(c.dest), c.origin.max(c.dest)))
        .collect();
    let compass: Vec<&crate::graph::Connection> = graph
        .connections()
        .iter()
        .filter(|c| {
            layout_offset(c.dir).is_some()
                && !(matches!(c.dir, Direction::Up | Direction::Down)
                    && compass_pairs.contains(&(c.origin.min(c.dest), c.origin.max(c.dest))))
        })
        .collect();
    // Edge indices already drawn — either as their own connector or consumed as the
    // back-edge of an earlier bidirectional pairing.
    let mut consumed: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    let occupied: std::collections::BTreeSet<(i32, i32)> =
        graph.rooms().filter_map(|r| r.pos).collect();
    // Resolve direct-route line collisions up front so the LONGER connector keeps the straight line
    // and shorter rivals weave — independent of edge listing order.
    let direct_losers = direct_route_losers(&compass, graph, &occupied);
    let mut out: Vec<RoutedConnector> = Vec::new();
    // The trunk polyline for each unordered room pair, recorded when its connector is built.
    // A later edge between the same pair becomes a MERGE STUB that joins this trunk.
    let mut trunk_points: BTreeMap<(RoomId, RoomId), Vec<(i32, i32)>> = BTreeMap::new();
    for (ci, c) in compass.iter().enumerate() {
        if consumed.contains(&ci) {
            continue; // already drawn as an earlier pair's back-edge
        }
        let pair = (c.origin.min(c.dest), c.origin.max(c.dest));
        // Extra edge between an already-connected pair → merge stub: route from this edge's exit
        // side to a junction on the trunk and end there (a T-junction). Only the box-edge
        // departure arrow is drawn; no independent line reaches the destination.
        if let Some(trunk) = trunk_points.get(&pair) {
            if let (Some(a), Some(exit)) =
                (graph.room(c.origin).and_then(|r| r.pos), route_side(c.dir))
            {
                if let Some(pts) = merge_stub_points(a, exit, trunk) {
                    out.push(RoutedConnector {
                        origin: c.origin,
                        dest: c.dest,
                        distorted: c.distorted,
                        exit,
                        entry: exit, // unused — no arrival is drawn for a merge stub
                        points: pts,
                        segs: Vec::new(),
                        exit_slot: 0,
                        entry_slot: 0,
                        reciprocal: false,
                        exit_dir: c.dir,
                        entry_dir: None,
                        merge: true,
                    });
                }
            }
            consumed.insert(ci);
            continue;
        }
        // Pair with an UNCONSUMED reverse edge (b→a) between the same rooms, if any. PREFER the
        // geometric reciprocal — the reverse edge whose direction is the exact compass-opposite of
        // this edge — so the trunk keeps its natural straight / single-L shape and its junction lands
        // at the doorway between the rooms. Pairing with an arbitrary reverse edge would force the
        // trunk to enter the far room from that edge's side, bending it up-and-over and leaving the
        // remaining same-pair edges as degenerate merge stubs (their junction ends up on the wrong
        // side, collapsing the stub to a zero-width line). Fall back to the first unconsumed reverse
        // edge when no exact opposite exists. Exactly one pairing collapses; further edges between
        // the pair draw on their own (as merge stubs).
        let back_idx = back_edge_idx(&compass, ci, c, &consumed);
        let back = back_idx.map(|pi| compass[pi]);
        let has_reciprocal = back.is_some();
        let (Some(a), Some(b)) = (graph.room(c.origin).and_then(|r| r.pos),
                                  graph.room(c.dest).and_then(|r| r.pos)) else { continue; };
        let Some(exit) = route_side(c.dir) else { continue; };

        // Direct route: if the path from `a` to `b` is clear along the room grid, draw it as a
        // straight line / single-corner L instead of dipping into channels — fewer turns, no
        // weaving around empty cells. A direct route keeps its natural clean entry: only a reciprocal
        // pair constrains it (the far arrow must point out the back-edge's side). A one-way edge does
        // NOT force the ruled side onto a clean direct route — for an aligned room the natural entry
        // already matches the rule, and for an offset room forcing it would wrap the connector around
        // the box. The ruled side applies to the dipped channel route below. Also reject a direct run
        // that stomps an already-routed connector on a shared room line (it carries no lane).
        let required_entry = back.and_then(|bk| route_side(bk.dir));
        // The destination side a one-way edge prefers when it must dip into the channels.
        let ruled_entry = match back {
            Some(bk) => route_side(bk.dir),
            None => oneway_entry_side(c.dir),
        };
        if let Some((sentry, pts)) = direct_route(a, exit, b, &occupied) {
            // Reject if this straight run stomps an already-placed connector, or if the pre-pass
            // ruled this the shorter rival on a contested room line (so it weaves and the longer
            // connector keeps the line).
            let stomps_existing = out.iter().any(|o| polylines_overlap(&pts, &o.points));
            let is_loser = direct_losers.contains(&ci);
            if required_entry.is_none_or(|req| req == sentry) && !stomps_existing && !is_loser {
                out.push(RoutedConnector {
                    origin: c.origin,
                    dest: c.dest,
                    distorted: c.distorted,
                    exit,
                    entry: sentry,
                    points: pts,
                    segs: Vec::new(),
                    exit_slot: 0,
                    entry_slot: 0,
                    reciprocal: has_reciprocal,
                    exit_dir: c.dir,
                    entry_dir: back.map(|bk| bk.dir),
                    merge: false,
                });
                trunk_points.insert(pair, out.last().unwrap().points.clone());
                consumed.insert(ci);
                if let Some(pi) = back_idx {
                    consumed.insert(pi);
                }
                continue;
            }
        }

        // Generate candidate routes. The exit side is fixed by the edge's compass direction;
        // we vary the L orientation and (for non-reciprocal connectors, in greedy mode only)
        // the destination entry side among the sides still facing the origin. Candidate order
        // is fixed and integer-tiebroken, so the choice is deterministic.
        // A bidirectional connector enters the far room on the side ITS OWN back-edge departs, so
        // the far-end arrow points out the true compass direction (an exact-opposite pair resolves
        // to the opposite side = a straight reciprocal). A one-way connector enters on the side
        // fixed by its own direction (`oneway_entry_side`). Either way the entry side is determined,
        // so greedy only varies the L orientation; the `entry_side` fallbacks cover the (filtered)
        // non-planar case for safety.
        let entry_choices: Vec<Side> = match ruled_entry {
            // Reciprocal pairs are fixed to the back-edge side. A one-way edge prefers its ruled
            // side first, but keeps the geometric facing sides as fallbacks so the greedy can still
            // dodge a stomp when the ruled side is blocked (rather than forcing an illegal overlap).
            Some(s) if back.is_some() => vec![s],
            Some(s) => {
                let mut v = vec![s];
                for alt in entry_side_alternatives(a, b) {
                    if !v.contains(&alt) {
                        v.push(alt);
                    }
                }
                v
            }
            None if greedy => entry_side_alternatives(a, b),
            None => vec![entry_side(a, b)],
        };
        let orients: &[Orient] = if greedy {
            &[Orient::HorizontalFirst, Orient::VerticalFirst]
        } else {
            &[Orient::HorizontalFirst]
        };
        // (rank, entry, points): `rank` bakes the deterministic preference — earlier entry
        // choices (geometric first) and HorizontalFirst orientation are preferred on ties.
        type Candidate = (usize, Side, Vec<(i32, i32)>);
        let mut candidates: Vec<Candidate> = Vec::new();
        for (ei, &entry) in entry_choices.iter().enumerate() {
            for (oi, &orient) in orients.iter().enumerate() {
                // Default mode emits the canonical horizontal-first route via `build_points`;
                // greedy mode also probes the vertical-first alternative.
                let pts = match orient {
                    Orient::HorizontalFirst => build_points(a, exit, b, entry),
                    Orient::VerticalFirst => build_points_orient(a, exit, b, entry, orient),
                };
                candidates.push((ei * 2 + oi, entry, pts));
            }
        }
        // Pick the candidate with the fewest crossings against already-placed connectors;
        // ties break by the preference rank, then by the points (fully deterministic).
        let chosen = candidates
            .into_iter()
            .map(|(rank, entry, pts)| {
                // Disqualify candidates that introduce a parallel line-on-line overlap the
                // lane system can't resolve (renderer rejects these): make it the PRIMARY key
                // so an overlap-free route always wins. Among overlap-free candidates, minimize
                // the crossings this candidate adds against the already-placed connectors; ties
                // break by preference rank (geometric entry, horizontal-first), then the points.
                let overlaps = out.iter().filter(|o| has_parallel_overlap(&pts, &o.points)).count();
                let crosses: usize = out.iter().map(|o| count_crossings(&pts, &o.points)).sum();
                (overlaps, crosses, rank, entry, pts)
            })
            .min_by(|x, y| {
                x.0.cmp(&y.0)
                    .then(x.1.cmp(&y.1))
                    .then(x.2.cmp(&y.2))
                    .then(x.4.cmp(&y.4))
            })
            .expect("at least one candidate route");

        out.push(RoutedConnector {
            origin: c.origin,
            dest: c.dest,
            distorted: c.distorted,
            exit,
            entry: chosen.3,
            points: chosen.4,
            segs: Vec::new(),
            exit_slot: 0,
            entry_slot: 0,
            reciprocal: has_reciprocal,
            exit_dir: c.dir,
            entry_dir: back.map(|bk| bk.dir),
            merge: false,
        });
        trunk_points.insert(pair, out.last().unwrap().points.clone());
        consumed.insert(ci);
        if let Some(pi) = back_idx {
            consumed.insert(pi); // the paired back-edge is now drawn too
        }
    }
    out
}

/// Extract the long runs of a doubled-coord polyline as (channel, start, end) with
/// start<=end, skipping the room-cell endpoints and zero-length steps. A horizontal run
/// (constant odd y) → `H((y-1)/2)`; a vertical run (constant odd x) → `V((x-1)/2)`.
fn long_runs(points: &[(i32, i32)]) -> Vec<(Channel, i32, i32)> {
    let mut runs = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a.1 == b.1 && a.1 % 2 != 0 && a.0 != b.0 {
            // horizontal run on channel H[(y-1)/2]
            let (s, e) = (a.0.min(b.0), a.0.max(b.0));
            runs.push((Channel::H((a.1 - 1).div_euclid(2)), s, e));
        } else if a.0 == b.0 && a.0 % 2 != 0 && a.1 != b.1 {
            let (s, e) = (a.1.min(b.1), a.1.max(b.1));
            runs.push((Channel::V((a.0 - 1).div_euclid(2)), s, e));
        }
        // steps touching even coords are the stubs (room↔lattice); they carry no lane.
    }
    runs
}

/// Left-edge interval colouring per channel. Returns, for each input run, the lane it was
/// assigned, plus the per-channel lane count. Runs are processed in a deterministic order
/// (by channel, then start, then end) so assignment is stable.
fn assign_lanes(
    runs: &[(usize, Channel, i32, i32)], // (connector-run id, channel, start, end)
) -> (std::collections::HashMap<usize, u16>, BTreeMap<Channel, u16>) {
    use std::collections::HashMap;
    // bucket by channel
    let mut by_ch: BTreeMap<Channel, Vec<(usize, i32, i32)>> = BTreeMap::new();
    for &(id, ch, s, e) in runs {
        by_ch.entry(ch).or_default().push((id, s, e));
    }
    let mut lane_of: HashMap<usize, u16> = HashMap::new();
    let mut counts: BTreeMap<Channel, u16> = BTreeMap::new();
    for (ch, mut items) in by_ch {
        items.sort_by_key(|&(_, s, e)| (s, e));
        // lane_end[l] = current right edge occupied on lane l (exclusive comparison).
        let mut lane_end: Vec<i32> = Vec::new();
        for (id, s, e) in items {
            // first lane whose last extent ends strictly before s
            let lane = match lane_end.iter().position(|&end| end < s) {
                Some(l) => { lane_end[l] = e; l }
                None => { lane_end.push(e); lane_end.len() - 1 }
            };
            lane_of.insert(id, lane as u16);
        }
        counts.insert(ch, lane_end.len() as u16);
    }
    (lane_of, counts)
}

/// Assign per-(room, side) slot indices to every connector endpoint so that two
/// connectors sharing one room side (e.g. an arrival and a departure, or two departures)
/// anchor on distinct cells along that side instead of colliding on the side centre.
///
/// Each connector has an exit endpoint at `(origin, exit)` and an entry endpoint at
/// `(dest, entry)`. Endpoints are grouped by `(room, side)` and assigned 0,1,2,… in a
/// deterministic order (by room, side, then connector index) so rendering is stable.
/// True if a doubled-coord polyline is a single straight segment (all points share one axis),
/// i.e. the connector runs straight through with no turn.
fn is_collinear(points: &[(i32, i32)]) -> bool {
    points.iter().all(|p| p.0 == points[0].0) || points.iter().all(|p| p.1 == points[0].1)
}

fn assign_side_slots(connectors: &mut [RoutedConnector]) {
    // Collect every endpoint as (room, side, is_exit, connector index).
    let mut endpoints: Vec<(RoomId, Side, bool, usize)> = Vec::new();
    for (ci, c) in connectors.iter().enumerate() {
        endpoints.push((c.origin, c.exit, true, ci));
        // A merge stub ends on the trunk (not at the destination box), so it has no arrival
        // endpoint to slot.
        if !c.merge {
            endpoints.push((c.dest, c.entry, false, ci));
        }
    }
    // Group by (room, side); assign slots within each group.
    //
    // Compass-only sides keep the historical order: a connector that runs STRAIGHT through the
    // side (a collinear polyline — its other end is axis-aligned with this room) takes the center
    // slot ahead of weaving connectors, so it stays a clean straight line while the weaving ones
    // take the offset cells; ties break by (connector index, is_exit). This is preserved
    // byte-for-byte (compass identity).
    //
    // On a side that ALSO hosts an Up/Down endpoint, the priority is refined (SQ-0222):
    //   1. an AXIS-reciprocal compass connector (reciprocal AND cardinal-opposite exit/entry:
    //      N<->S or E<->W — the column/row-locked case) keeps the center (SQ-0216 intent); then
    //   2. a STRAIGHT-through connector keeps the center (so a straight Up/Down line is not forced
    //      to jog across a weaving one-way compass connector, which manufactures an overlap); then
    //   3. compass before Up/Down, then connector index, for determinism.
    // The blanket SQ-0216 "Up/Down always yields to compass" was too broad: it displaced a
    // straight Up/Down line for a NON-reciprocal weaving compass connector. Gating the refined
    // order to sides that actually host an Up/Down endpoint keeps every compass-only side
    // byte-identical.
    // Endpoint: (is up/down?, axis-reciprocal compass?, straight-through?, is_exit, connector index).
    type Endpoint = (bool, bool, bool, bool, usize);
    let mut by_side: BTreeMap<(RoomId, Side), Vec<Endpoint>> = BTreeMap::new();
    for (room, side, is_exit, ci) in endpoints {
        let straight = is_collinear(&connectors[ci].points);
        let is_updown = matches!(connectors[ci].exit_dir, Direction::Up | Direction::Down);
        let axis_recip = connectors[ci].reciprocal
            && matches!(
                (connectors[ci].exit_dir, connectors[ci].entry_dir),
                (Direction::N, Some(Direction::S))
                    | (Direction::S, Some(Direction::N))
                    | (Direction::E, Some(Direction::W))
                    | (Direction::W, Some(Direction::E))
            );
        by_side.entry((room, side)).or_default().push((is_updown, axis_recip, straight, is_exit, ci));
    }
    for (_key, mut members) in by_side {
        let has_updown = members.iter().any(|&(is_updown, ..)| is_updown);
        members.sort_by_key(|&(is_updown, axis_recip, straight, is_exit, ci)| {
            // `axis_recip` only applies on sides that host an Up/Down endpoint; elsewhere it is
            // neutralized so the key reduces to the historical `(is_updown, Reverse(straight), ci,
            // is_exit)` — every compass-only side stays byte-identical. On a mixed side the order is
            // axis-reciprocal-compass → straight-through → compass-before-Up/Down → index.
            let axis_key = std::cmp::Reverse(has_updown && axis_recip);
            (axis_key, std::cmp::Reverse(straight), is_updown, ci, is_exit)
        });
        for (slot, (.., is_exit, ci)) in members.into_iter().enumerate() {
            if is_exit {
                connectors[ci].exit_slot = slot as u16;
            } else {
                connectors[ci].entry_slot = slot as u16;
            }
        }
    }
}

/// Full logical route: topology + lane assignment.
pub fn route_lanes(graph: &MapGraph) -> RoutePlan {
    let mut connectors = route_topology(graph);
    assign_side_slots(&mut connectors);

    // Flatten every connector's long runs into a global list with stable ids.
    let mut runs: Vec<(usize, Channel, i32, i32)> = Vec::new();
    let mut owner: Vec<(usize, Channel, i32, i32)> = Vec::new(); // (connector idx, channel, s, e)
    for (ci, c) in connectors.iter().enumerate() {
        for (ch, s, e) in long_runs(&c.points) {
            let id = runs.len();
            runs.push((id, ch, s, e));
            owner.push((ci, ch, s, e));
        }
    }
    let (lane_of, counts) = assign_lanes(&runs);

    // Attach lanes back onto each connector.
    for (id, (ci, ch, s, e)) in owner.into_iter().enumerate() {
        let lane = lane_of[&id];
        connectors[ci].segs.push(LaneSeg { channel: ch, lane, start: s, end: e });
    }

    let mut h_lanes = BTreeMap::new();
    let mut v_lanes = BTreeMap::new();
    for (ch, n) in counts {
        match ch {
            Channel::H(r) => { h_lanes.insert(r, n); }
            Channel::V(c) => { v_lanes.insert(c, n); }
        }
    }
    RoutePlan { connectors, h_lanes, v_lanes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use crate::router::side_for;

    #[test]
    fn segments_sharing_a_lane_never_overlap() {
        // Build a small congested graph and assert the core invariant across all channels.
        let mut g = MapGraph::new();
        for id in 1..=4 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (3, 0)); g.set_pos(3, (0, 1)); g.set_pos(4, (3, 1));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::E, 4); // a second horizontal run in a nearby channel
        g.add_edge(1, Direction::S, 3);
        let plan = route_lanes(&g);
        // Group segs by (channel, lane); within a group, extents must be pairwise disjoint.
        use std::collections::BTreeMap;
        let mut by_lane: BTreeMap<(Channel, u16), Vec<(i32, i32)>> = BTreeMap::new();
        for c in &plan.connectors {
            for s in &c.segs {
                by_lane.entry((s.channel, s.lane)).or_default().push((s.start, s.end));
            }
        }
        for ((ch, lane), mut ivs) in by_lane {
            ivs.sort();
            for w in ivs.windows(2) {
                assert!(w[0].1 < w[1].0,
                    "overlap in {ch:?} lane {lane}: {:?} and {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn assign_lanes_shares_a_lane_only_for_disjoint_runs() {
        // Three runs in ONE channel H(0): two disjoint ([0,2],[4,6]) may share lane 0;
        // one overlapping both ([1,5]) must take a new lane. Directly exercises the
        // core invariant (a graph-level test can't force a shared lane reliably).
        let ch = Channel::H(0);
        let runs = vec![(0usize, ch, 0, 2), (1usize, ch, 4, 6), (2usize, ch, 1, 5)];
        let (lane_of, counts) = assign_lanes(&runs);
        assert_eq!(lane_of[&0], 0, "first run → lane 0");
        assert_eq!(lane_of[&1], 0, "disjoint run shares lane 0");
        assert_eq!(lane_of[&2], 1, "overlapping run gets a new lane");
        assert_eq!(counts[&ch], 2, "channel needs 2 lanes");
        // Invariant: any two runs sharing a lane have disjoint extents.
        let mut by_lane: std::collections::BTreeMap<u16, Vec<(i32, i32)>> =
            std::collections::BTreeMap::new();
        for &(id, _, s, e) in &runs {
            by_lane.entry(lane_of[&id]).or_default().push((s, e));
        }
        for (_lane, mut ivs) in by_lane {
            ivs.sort();
            for w in ivs.windows(2) {
                assert!(w[0].1 < w[1].0, "same-lane runs must be disjoint: {:?} {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn two_overlapping_runs_get_two_lanes() {
        // Two connectors whose horizontal runs share a channel and overlap in extent must
        // land on different lanes → that channel reports lane_count 2.
        let mut g = MapGraph::new();
        for id in 1..=5 { g.upsert_room(id, "r".into()); }
        // 1->2 and 3->4 both run horizontally across the same row band, overlapping in x.
        // Room 5 fills the one cell between 3 and 4 so NEITHER pair can take the direct
        // straight-shot (empty-gap) route — both must dip into the H channel and share it.
        g.set_pos(1, (0, 0)); g.set_pos(2, (4, 0));
        g.set_pos(3, (1, 0)); g.set_pos(4, (3, 0));
        g.set_pos(5, (2, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::E, 4);
        let plan = route_lanes(&g);
        let max_h = plan.h_lanes.values().copied().max().unwrap_or(0);
        assert!(max_h >= 2, "two overlapping horizontal runs need ≥2 lanes; got {max_h}");
    }

    #[test]
    fn polylines_overlap_sees_room_line_stomp() {
        // Full routes (room-centre endpoints first/last) both running up room column x=-14 into the
        // room at (-14,-8): their INTERIORS overlap on the column for many cells.
        let a = vec![(-2, 6), (-3, 6), (-14, 6), (-14, -7), (-14, -8)];
        let b = vec![(-10, 2), (-11, 2), (-14, 2), (-14, -7), (-14, -8)];
        assert!(polylines_overlap(&a, &b), "two routes up room column -14 overlap on the interior");
        // The second route reaches the room one channel over (x=-13) and only joins the column for the
        // final doorway stub — interiors share no line, so it is NOT a stomp.
        let c = vec![(-10, 2), (-13, 2), (-13, -7), (-14, -7), (-14, -8)];
        assert!(!polylines_overlap(&a, &c), "sharing only the doorway stub is not a stomp");
        // Different column entirely: no shared line.
        let d = vec![(-10, 2), (-11, 2), (-12, 2), (-12, -7), (-12, -8)];
        assert!(!polylines_overlap(&a, &d), "different columns do not overlap");
    }

    #[test]
    fn two_direct_routes_to_one_room_do_not_stomp() {
        // A and B both lie east of X and would each direct-route west then up X's column, stomping.
        // The router must reject the second straight route and dip it into a lane instead, so no two
        // connectors share a parallel room-line run.
        let mut g = MapGraph::new();
        for id in 1..=3 {
            g.upsert_room(id, "r".into());
        }
        g.set_pos(1, (0, 0)); // X
        g.set_pos(2, (5, 3)); // A
        g.set_pos(3, (5, 1)); // B
        g.add_edge(2, Direction::W, 1);
        g.add_edge(3, Direction::W, 1);
        let plan = route_lanes(&g);
        for i in 0..plan.connectors.len() {
            for j in (i + 1)..plan.connectors.len() {
                assert!(
                    !polylines_overlap(&plan.connectors[i].points, &plan.connectors[j].points),
                    "connectors {i} and {j} stomp on a shared line: {:?} {:?}",
                    plan.connectors[i].points, plan.connectors[j].points
                );
            }
        }
    }

    #[test]
    fn longer_direct_route_keeps_the_contested_line() {
        // A (far) and B (near) both enter T from the south up T's column and cannot both run straight
        // up it. The LONGER connector (A) must keep the direct line; the shorter (B) weaves. Without
        // length priority the outcome is edge-order dependent and the long path can be the one to weave.
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); // T
        g.set_pos(2, (3, 4)); // A (far)
        g.set_pos(3, (1, 2)); // B (near)
        g.add_edge(2, Direction::W, 1);
        g.add_edge(3, Direction::W, 1);
        let routed = route_topology(&g);
        // T's column is the contested room line (doubled x=0). The winner runs straight up it; the
        // loser yields it and routes up an adjacent channel instead.
        let runs = |o: u16, d: u16| {
            let c = routed.iter().find(|c| c.origin == o && c.dest == d).unwrap();
            all_runs(if c.points.len() > 2 { &c.points[1..c.points.len() - 1] } else { &[] })
        };
        let on_column_0 = |rs: &[(bool, i32, i32, i32)]| rs.iter().any(|&(h, l, _, _)| !h && l == 0);
        assert!(on_column_0(&runs(2, 1)), "A (longer) keeps the straight run up T's column");
        assert!(!on_column_0(&runs(3, 1)), "B (shorter) yields the column, routing alongside it");
    }

    #[test]
    fn route_lanes_is_deterministic() {
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (2, 0)); g.set_pos(3, (0, 2));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(1, Direction::S, 3);
        let a = format!("{:?}", route_lanes(&g));
        let b = format!("{:?}", route_lanes(&g));
        assert_eq!(a, b);
    }

    #[test]
    fn reciprocal_pairs_with_geometric_opposite_keeping_merge_stubs_intact() {
        // Adjacent #77/#239 with a reciprocal E/W edge plus extra N and S edges, where the geometric
        // opposite (W) of the forward E edge is added LAST. The reciprocal must pair E with W (not the
        // first reverse edge N), so the trunk stays straight and the N/S edges become real C-shaped
        // merge stubs. Pairing with N would bend the trunk up-and-over and collapse the S/W stubs to
        // degenerate zero-width lines — the "missing southern path" bug.
        let mut g = MapGraph::new();
        g.upsert_room(77, "f".into());
        g.upsert_room(239, "g".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (1, 0)); // adjacent
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        g.add_edge(239, Direction::W, 77); // opposite of E, added last on purpose
        let plan = route_lanes(&g);
        let trunk = plan.connectors.iter().find(|c| !c.merge).expect("a trunk connector");
        assert!(is_collinear(&trunk.points), "trunk must stay a straight E/W line: {:?}", trunk.points);
        let stubs: Vec<_> = plan.connectors.iter().filter(|c| c.merge).collect();
        assert_eq!(stubs.len(), 2, "the two extra edges become merge stubs");
        for s in &stubs {
            assert!(s.points.len() >= 4,
                "{:?} stub must keep its C-path, not collapse to a degenerate line: {:?}",
                s.exit_dir, s.points);
        }
    }

    #[test]
    fn same_row_empty_gap_routes_straight() {
        // Two rooms on one row with an empty cell between them: the connector runs STRAIGHT
        // along the room row (no channel dip) — its polyline is collinear and carries no lane.
        let mut g = MapGraph::new();
        for id in 1..=2 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (2, 0)); // gap at (1,0) is empty
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1, "reciprocal pair collapses to one connector");
        let conn = &plan.connectors[0];
        assert!(is_collinear(&conn.points), "straight shot must be collinear: {:?}", conn.points);
        assert!(conn.segs.is_empty(), "straight shot carries no lane segments");
    }

    #[test]
    fn same_row_occupied_gap_dips() {
        // Same two rooms, but a third room fills the gap: the connector cannot run straight
        // through an occupied cell, so it dips into a channel — its polyline turns.
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (2, 0)); g.set_pos(3, (1, 0)); // gap occupied
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        let plan = route_lanes(&g);
        let conn = plan.connectors.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert!(!is_collinear(&conn.points), "occupied gap must dip into a channel: {:?}", conn.points);
    }

    /// Count direction changes (turns) in a doubled-coord polyline.
    fn turn_count(pts: &[(i32, i32)]) -> usize {
        pts.windows(3)
            .filter(|w| {
                let d1 = ((w[1].0 - w[0].0).signum(), (w[1].1 - w[0].1).signum());
                let d2 = ((w[2].0 - w[1].0).signum(), (w[2].1 - w[1].1).signum());
                d1 != d2
            })
            .count()
    }

    #[test]
    fn oneway_entry_side_follows_the_compass_rule() {
        use Direction::*;
        // Cardinals enter the side opposite the travel direction (straight through).
        assert_eq!(oneway_entry_side(N), Some(Side::Bottom));
        assert_eq!(oneway_entry_side(S), Some(Side::Top));
        assert_eq!(oneway_entry_side(E), Some(Side::Left));
        assert_eq!(oneway_entry_side(W), Some(Side::Right));
        // Diagonals: NW->N, NE->E, SE->S, SW->W.
        assert_eq!(oneway_entry_side(NW), Some(Side::Top));
        assert_eq!(oneway_entry_side(NE), Some(Side::Right));
        assert_eq!(oneway_entry_side(SE), Some(Side::Bottom));
        assert_eq!(oneway_entry_side(SW), Some(Side::Left));
        // Up/Down enter opposite the side they departed, same as a cardinal.
        assert_eq!(oneway_entry_side(Up), Some(Side::Bottom));
        assert_eq!(oneway_entry_side(Down), Some(Side::Top));
        // The remaining non-planar directions have no side.
        assert_eq!(oneway_entry_side(In), None);
        assert_eq!(oneway_entry_side(Out), None);
        assert_eq!(oneway_entry_side(Unknown), None);
    }

    #[test]
    fn oneway_cardinal_enters_the_matching_side() {
        // A one-way S edge into an aligned room enters from the north (Top); an E edge from the west.
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 3)); // due south of 1
        g.set_pos(3, (3, 0)); // due east of 1
        g.add_edge(1, Direction::S, 2);
        g.add_edge(1, Direction::E, 3);
        let plan = route_lanes(&g);
        let entry = |o: u16, d: u16| plan.connectors.iter().find(|c| c.origin == o && c.dest == d).unwrap().entry;
        assert_eq!(entry(1, 2), Side::Top, "S edge enters destination's north side");
        assert_eq!(entry(1, 3), Side::Left, "E edge enters destination's west side");
    }

    #[test]
    fn diagonal_clear_path_routes_single_corner() {
        // Room 2 is east AND south of room 1 with all intervening cells empty: the connector
        // turns exactly ONCE (a clean L straight along the room grid), not weaving through
        // channels with extra dips.
        let mut g = MapGraph::new();
        for id in 1..=2 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (3, 2)); // SE of 1, clear path
        g.add_edge(1, Direction::E, 2); // one-way; exits Right, then turns south to 2
        let plan = route_lanes(&g);
        let conn = &plan.connectors[0];
        assert_eq!(turn_count(&conn.points), 1, "clear diagonal path = single corner: {:?}", conn.points);
    }

    #[test]
    fn diagonal_blocked_path_dips_more() {
        // Same diagonal, but a room blocks the clean L's corner row: the connector cannot take
        // the single-corner direct route and must weave (more than one turn).
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0)); g.set_pos(2, (3, 2));
        g.set_pos(3, (2, 0)); // sits on the direct L's first leg (row 0)
        g.add_edge(1, Direction::E, 2);
        let plan = route_lanes(&g);
        let conn = plan.connectors.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert!(turn_count(&conn.points) >= 2, "blocked path must weave: {:?}", conn.points);
    }

    /// Expand a doubled-coord polyline into the doubled cells it passes through.
    fn trace_cells(points: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for w in points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dx = (b.0 - a.0).signum();
            let dy = (b.1 - a.1).signum();
            let mut cur = a;
            loop {
                if out.last() != Some(&cur) { out.push(cur); }
                if cur == b { break; }
                cur = (cur.0 + dx, cur.1 + dy);
            }
        }
        out
    }

    fn doubled(cell: (i32, i32)) -> (i32, i32) { (cell.0 * 2, cell.1 * 2) }

    #[test]
    fn doubled_and_exit_points() {
        assert_eq!(cell_to_doubled((0, 0)), (0, 0));
        assert_eq!(cell_to_doubled((-1, 2)), (-2, 4));
        assert_eq!(exit_point((0, 0), Side::Right), (1, 0));
        assert_eq!(exit_point((0, 0), Side::Top), (0, -1));
        assert_eq!(exit_point((-1, 2), Side::Left), (-3, 4));
    }

    #[test]
    fn every_long_run_is_on_the_gap_lattice() {
        // Two rooms, A(0,0) -E-> B(2,0): the connector's long runs must lie on odd
        // coordinates (channels), so they never touch a room cell (even,even).
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.add_edge(1, Direction::E, 2);
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1);
        let c = &conns[0];
        assert_eq!((c.origin, c.dest), (1, 2));
        // points form an orthogonal polyline (each step changes exactly one axis).
        for w in c.points.windows(2) {
            let same_x = w[0].0 == w[1].0;
            let same_y = w[0].1 == w[1].1;
            assert!(same_x ^ same_y, "polyline must be orthogonal; got {:?}->{:?}", w[0], w[1]);
        }
        // The interior run cells (excluding the two room centres at the ends) never land
        // on a room centre (even,even pair that equals a placed room's doubled cell).
        let room_cells: Vec<(i32, i32)> = vec![doubled((0, 0)), doubled((2, 0))];
        let cells = trace_cells(&c.points);
        for (i, &p) in cells.iter().enumerate() {
            let is_endpoint = i == 0 || i == cells.len() - 1;
            if !is_endpoint {
                assert!(!room_cells.contains(&p), "run passes through room cell {p:?}");
            }
        }
    }

    #[test]
    fn adjacent_facing_rooms_route_straight_no_dip() {
        // A(0,0) -E-> B(1,0): the two boxes are adjacent and facing, so the exit stub and
        // entry stub are the SAME shared-doorway cell. The route must go straight through it
        // (ca → ea → cb) with NO lattice dip — no out-and-back tail, and no channel run/lane.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        let plan = route_lanes(&g);
        assert_eq!(plan.connectors.len(), 1);
        let c = &plan.connectors[0];
        assert_eq!(c.points, vec![(0, 0), (1, 0), (2, 0)], "straight ca→ea→cb, no dip");
        assert!(c.segs.is_empty(), "an adjacent straight route needs no laned run; got {:?}", c.segs);
        // No cell is visited twice (no out-and-back).
        let cells = trace_cells(&c.points);
        let unique: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), unique.len(), "route must not revisit any cell");
    }

    #[test]
    fn degenerate_l_is_merged_to_single_run() {
        // A connector whose L has a zero-length leg leaves three collinear points; merge_collinear
        // collapses them so a straight channel run is ONE segment (not two on possibly different
        // lanes — the cause of a line rendering as a diagonal).
        let mut pts = vec![(0, 0), (1, 0), (1, 1), (1, 2), (1, 3), (2, 3)];
        merge_collinear(&mut pts);
        assert_eq!(pts, vec![(0, 0), (1, 0), (1, 3), (2, 3)], "collinear midpoints removed");
    }

    #[test]
    fn same_side_endpoints_get_distinct_slots() {
        // Room 1 has TWO endpoints on its Right side: a departure (1 -E-> 2) and an arrival
        // (3 -W-> 1, entering 1 from the right). They must receive distinct slot indices so
        // the renderer can offset their anchors onto separate cells along that side.
        let mut g = MapGraph::new();
        for id in [1, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.set_pos(3, (4, 0)); // east of 1, beyond 2: its direct west path to 1 is blocked by 2
        g.add_edge(1, Direction::E, 2); // exits room 1 on its Right
        g.add_edge(3, Direction::W, 1); // blocked by room 2 → dips, entering room 1 from the right
        let plan = route_lanes(&g);
        let mut slots = Vec::new();
        for c in &plan.connectors {
            if c.origin == 1 && c.exit == Side::Right { slots.push(c.exit_slot); }
            if c.dest == 1 && c.entry == Side::Right { slots.push(c.entry_slot); }
        }
        assert_eq!(slots.len(), 2, "two endpoints on room 1's Right side; got {slots:?}");
        assert_ne!(slots[0], slots[1], "same-side endpoints must get distinct slots");
    }

    #[test]
    fn reciprocal_pair_collapses_to_one_connector() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal
        assert_eq!(route_topology(&g).len(), 1, "reciprocal pair → one connector");
    }

    /// A bidirectional pair joined by NON-opposite directions collapses to ONE connector,
    /// each end's arrow on its own compass side: `1→N→2` + `2→E→1` → one reciprocal
    /// connector exiting room 1 on its north side and entering room 2 on its east side.
    #[test]
    fn non_opposite_bidirectional_pair_collapses() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -2));
        g.add_edge(1, Direction::N, 2); // 1→N→2
        g.add_edge(2, Direction::E, 1); // 2→E→1  (bidirectional, non-opposite)
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "bidirectional pair collapses to one connector; got {}", conns.len());
        assert!(conns[0].reciprocal, "collapsed bidirectional connector must be reciprocal");
        assert_eq!(conns[0].exit, side_for(Direction::N).unwrap(), "exits room 1 on its north side");
        assert_eq!(conns[0].entry, side_for(Direction::E).unwrap(), "enters room 2 on its east side");
    }

    /// Regression: a true opposite pair `1→N→2` + `2→S→1` collapses to exactly ONE
    /// connector and that connector must have `reciprocal == true`.
    #[test]
    fn true_opposite_pair_is_reciprocal() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -2));
        g.add_edge(1, Direction::N, 2); // 1→N→2
        g.add_edge(2, Direction::S, 1); // 2→S→1  (true opposite: S == opposite(N))
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "true opposite pair collapses to one connector");
        assert!(conns[0].reciprocal, "collapsed true-opposite connector must have reciprocal == true");
    }

    /// Regression (A129 #239↔#77): three edges between one pair — `1→E→2`, `2→N→1`,
    /// `2→S→1`. Exactly ONE pairing collapses (1↔2); the extra `2→S→1` must still be
    /// drawn as its own connector, not dropped because the room-pair was already seen.
    #[test]
    fn third_edge_between_pair_is_not_dropped() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -2));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::N, 1);
        g.add_edge(2, Direction::S, 1);
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 2, "one pairing collapses, the third edge stays; got {}", conns.len());
        assert_eq!(conns.iter().filter(|c| c.reciprocal).count(), 1, "exactly one reciprocal pairing");
        let s = conns.iter().find(|c| {
            c.origin == 2 && c.dest == 1 && c.exit == side_for(Direction::S).unwrap()
        });
        assert!(s.is_some(), "the third edge (2→S→1) must still be drawn, not dropped");
    }

    #[test]
    fn orientation_choice_reduces_crossings() {
        // Connector A is a short horizontal run H(0) on y=1 spanning x∈[1,3].
        let a = build_points((0, 0), Side::Right, (2, 0), Side::Left);
        // Connector B goes from (4,-2) to (0,2). The two L orientations route differently:
        //  - HORIZONTAL-FIRST drops its vertical leg at x=1, INSIDE A's x-span → crosses A.
        //  - VERTICAL-FIRST drops its vertical leg at x=7, OUTSIDE A's x-span → no crossing.
        let hf = build_points_orient((4, -2), Side::Left, (0, 2), Side::Top, Orient::HorizontalFirst);
        let vf = build_points_orient((4, -2), Side::Left, (0, 2), Side::Top, Orient::VerticalFirst);
        assert_eq!(count_crossings(&a, &hf), 1, "horizontal-first must cross A once");
        assert_eq!(count_crossings(&a, &vf), 0, "vertical-first must avoid A");
        // The greedy chooser, presented both orientations against the already-placed A, must
        // pick the non-crossing (vertical-first) route.
        let pick_hf = count_crossings(&a, &hf);
        let pick_vf = count_crossings(&a, &vf);
        assert!(pick_vf < pick_hf, "greedy should prefer the lower-crossing orientation");
    }

    #[test]
    fn greedy_picks_non_crossing_route_end_to_end() {
        // Two connectors whose default (horizontal-first) routes cross, but where rerouting
        // the second connector vertical-first removes the crossing cleanly. `route_topology`
        // must choose the lower-crossing, overlap-free route set.
        let mut g = MapGraph::new();
        for id in [1, 2, 3, 4, 5] { g.upsert_room(id, "r".into()); }
        // A: 1 -E-> 2, a short horizontal pair. Room 5 fills the cell between them so A must
        // DIP into a channel (no direct straight-shot), keeping its run crossable by B.
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.set_pos(5, (1, 0));
        g.add_edge(1, Direction::E, 2);
        // B: 3 -W-> 4 from the upper-right down to the left, off-axis so its L can flip.
        g.set_pos(3, (4, -2));
        g.set_pos(4, (0, 2));
        g.add_edge(3, Direction::W, 4);
        let crossings = |conns: &[RoutedConnector]| {
            let mut n = 0;
            for i in 0..conns.len() { for j in (i + 1)..conns.len() {
                n += count_crossings(&conns[i].points, &conns[j].points);
            } }
            n
        };
        // The canonical (non-greedy) routing DOES cross here…
        let default = route_topology_with(&g, false);
        assert!(crossings(&default) > 0, "this graph's default routing must cross (test is meaningful)");
        // …and `route_topology` (greedy + best-of) must pick the crossing-free route set.
        let chosen = route_topology(&g);
        assert_eq!(crossings(&chosen), 0, "route_topology must pick the crossing-free route set");
    }

    #[test]
    fn greedy_routing_is_deterministic() {
        // The same graph that triggers a greedy reroute must produce an identical RoutePlan
        // every time (fixed candidate order, integer tiebreaks, no RNG).
        let mut g = MapGraph::new();
        for id in [1, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
        g.set_pos(3, (4, -2));
        g.set_pos(4, (0, 2));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::W, 4);
        let a = format!("{:?}", route_lanes(&g));
        let b = format!("{:?}", route_lanes(&g));
        assert_eq!(a, b, "greedy routing must be deterministic");
    }

    #[test]
    fn extra_same_pair_edges_become_merge_stubs() {
        // 77↔239 E/W reciprocal trunk, plus 239→N→77 and 239→S→77 (the A129 tangle).
        let mut g = MapGraph::new();
        g.upsert_room(77, "a".into());
        g.upsert_room(239, "b".into());
        g.set_pos(77, (0, 0));
        g.set_pos(239, (2, 0)); // east of 77, a gap of one cell
        g.add_edge(77, Direction::E, 239);
        g.add_edge(239, Direction::W, 77); // reciprocal → the trunk
        g.add_edge(239, Direction::N, 77);
        g.add_edge(239, Direction::S, 77);
        let conns = route_topology(&g);
        let trunks: Vec<_> = conns.iter().filter(|c| !c.merge).collect();
        let stubs: Vec<_> = conns.iter().filter(|c| c.merge).collect();
        assert_eq!(trunks.len(), 1, "one trunk for the pair; got {}", trunks.len());
        assert_eq!(stubs.len(), 2, "the two extra edges are merge stubs; got {}", stubs.len());
        // Each stub carries its own box-edge exit direction (N / S).
        assert!(stubs.iter().any(|c| c.exit == side_for(Direction::N).unwrap()), "N stub");
        assert!(stubs.iter().any(|c| c.exit == side_for(Direction::S).unwrap()), "S stub");
        // Each stub ENDS on the trunk (its last point is one of the trunk's points).
        let trunk_pts = &trunks[0].points;
        for s in &stubs {
            let last = *s.points.last().unwrap();
            assert!(trunk_pts.contains(&last), "merge stub must end on the trunk: {last:?} not in {trunk_pts:?}");
        }
    }

    #[test]
    fn stub_excluded_non_compass() {
        // Up/Down are now routed as lane connectors (see `updown_is_routed_as_a_lane_connector_not_collapsed`
        // below); only the truly non-planar directions (In/Out/Unknown) remain excluded stubs.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::In, 2); // non-planar stub
        assert!(route_topology(&g).is_empty(), "non-planar edges are not routed");
    }

    #[test]
    fn connector_carries_diagonal_exit_dir() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1)); // SW of room 1
        g.add_edge(1, Direction::SW, 2);
        let conns = route_topology(&g);
        let c = conns.iter().find(|c| c.origin == 1 && c.dest == 2).unwrap();
        assert_eq!(c.exit_dir, Direction::SW);
        assert_eq!(c.entry_dir, None, "one-way edge has no back-edge dir");
    }

    #[test]
    fn reciprocal_diagonal_carries_both_dirs() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::SW, 2);
        g.add_edge(2, Direction::NE, 1); // true reciprocal
        let conns = route_topology(&g);
        assert_eq!(conns.len(), 1, "reciprocal diagonal collapses to one connector");
        let c = &conns[0];
        assert!(c.reciprocal);
        let dirs = [c.exit_dir, c.entry_dir.expect("reciprocal carries entry_dir")];
        assert!(
            dirs.contains(&Direction::SW) && dirs.contains(&Direction::NE),
            "both diagonal directions carried: {dirs:?}"
        );
    }

    #[test]
    fn updown_is_routed_as_a_lane_connector_not_collapsed() {
        // A--Up-->B (B north of A). Up should now be a lane connector, not just a stub.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let up_connectors: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up))
            .collect();
        assert_eq!(up_connectors.len(), 1, "the Up edge produces one lane connector");
        assert!(!up_connectors[0].reciprocal, "an unmatched one-way Up (no reverse Down) is not reciprocal");
    }

    #[test]
    fn reciprocal_updown_pair_collapses_to_one() {
        // A--Up-->B and B--Down-->A: after visual testing (SQ-0216) this REVISES Task 6's
        // "never collapse" decision — a matching Up/Down pair now collapses to ONE
        // reciprocal connector, same as a compass reciprocal pair.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);

        let plan = route_lanes(&g);
        let vertical: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(vertical.len(), 1, "a matching Up/Down pair collapses to one connector");
        assert!(vertical[0].reciprocal, "the collapsed connector is reciprocal");
        assert_eq!(vertical[0].entry_dir, Some(Direction::Down), "carries the back-edge's direction");
        // Regression guard: for an AXIS-ALIGNED pair (B due north of A), the connector must
        // still exit A's Top and enter B's Bottom — the axis-aligned case already worked
        // before SQ-0216's diagonal fix and must keep working after it.
        assert_eq!(vertical[0].exit, Side::Top, "exits A's north border");
        assert_eq!(vertical[0].entry, Side::Bottom, "enters B's south border");
    }

    #[test]
    fn reciprocal_updown_diagonal_enters_on_north_south_border() {
        // SQ-0216 bug repro: a reciprocal Up/Down pair placed DIAGONALLY (room 2 is
        // up-and-left of room 1, not sharing a row or column) must still enter the
        // destination on a north/south border (Top/Bottom) — never a geometric Left/Right
        // side picked by the diagonal cell offset. Before the fix, the reciprocal-collapse
        // branch computed the forced entry side via `side_for(bk.dir)`, which returns `None`
        // for Up/Down back-edges, so the code fell through to the generic geometric
        // `entry_side`, which picked Left/Right for a diagonal offset.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (1, 1));
        g.set_pos(2, (0, 0));
        g.add_edge(2, Direction::Down, 1);
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let vertical: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(vertical.len(), 1, "the diagonal Up/Down pair still collapses to one connector");
        let c = vertical[0];
        assert!(
            matches!(c.exit, Side::Top | Side::Bottom),
            "exit side must be a north/south border, got {:?}", c.exit
        );
        assert!(
            matches!(c.entry, Side::Top | Side::Bottom),
            "entry side must be a north/south border, not a geometric Left/Right pick, got {:?}", c.entry
        );
    }

    #[test]
    fn unmatched_oneway_updown_stays_non_reciprocal() {
        // A--Up-->B with no matching Down back-edge: stays one connector, non-reciprocal.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let vertical: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(vertical.len(), 1);
        assert!(!vertical[0].reciprocal, "an unmatched one-way up/down stays non-reciprocal");
    }

    #[test]
    fn updown_never_pairs_with_a_compass_edge() {
        // A--Up-->B and B--S-->A: B--S-->A is geometrically the "reverse" edge between the
        // same two rooms, but up/down must NEVER pair with a compass edge (only with the
        // exact opposite up/down direction). Since SQ-0219, sharing a pair with a compass
        // edge SUPPRESSES the up/down edge entirely rather than letting it draw as a
        // non-reciprocal connector — the compass path is the only one drawn for the pair.
        //
        // The Up edge is listed FIRST, before the S edge — the ordering that exposed a
        // regression where the up/down edge was skipped mid-loop (via `continue`) instead of
        // being excluded from the working set. That mid-loop skip let the trunk-claim slot for
        // the pair fall through to the S edge, flipping it from a merge stub (`merge == true`)
        // to a full trunk (`merge == false`, extra points) relative to the no-Up baseline —
        // violating the "compass routing stays byte-identical" constraint. Assert the surviving
        // S connector is IDENTICAL (merge/points/reciprocal) to the same graph without the Up
        // edge at all, which directly encodes that constraint.
        let mut with_up = MapGraph::new();
        with_up.upsert_room(1, "A".into());
        with_up.upsert_room(2, "B".into());
        with_up.set_pos(1, (0, 0));
        with_up.set_pos(2, (0, -1));
        with_up.add_edge(1, Direction::Up, 2);
        with_up.add_edge(2, Direction::S, 1);

        let plan = route_lanes(&with_up);
        let up = plan.connectors.iter().find(|c| c.exit_dir == Direction::Up);
        assert!(up.is_none(), "up/down must never pair with a compass edge — it is suppressed, not routed");
        let s_with_up = plan.connectors.iter().find(|c| c.exit_dir == Direction::S)
            .expect("the compass edge is still routed");

        let mut without_up = MapGraph::new();
        without_up.upsert_room(1, "A".into());
        without_up.upsert_room(2, "B".into());
        without_up.set_pos(1, (0, 0));
        without_up.set_pos(2, (0, -1));
        without_up.add_edge(2, Direction::S, 1);
        let baseline = route_lanes(&without_up);
        let s_baseline = baseline.connectors.iter().find(|c| c.exit_dir == Direction::S)
            .expect("the compass edge is routed in the baseline graph too");

        assert_eq!(s_with_up.merge, s_baseline.merge,
            "the S connector's trunk/merge-stub role must not change when a suppressed Up edge is present");
        assert_eq!(s_with_up.points, s_baseline.points,
            "the S connector's polyline must be byte-identical with or without the suppressed Up edge");
        assert_eq!(s_with_up.reciprocal, s_baseline.reciprocal,
            "the S connector's reciprocal flag must not change when a suppressed Up edge is present");
    }

    #[test]
    fn ns_reciprocal_outranks_updown_for_center_slot() {
        // Room 1 has BOTH a reciprocal N/S connector (to room 2, bent — off-column) and a
        // reciprocal Up/Down connector (to room 3, straight — due north) sharing its Top
        // side. Even though the Up/Down connector is the STRAIGHT one (which would otherwise
        // win the center slot on its own), the N/S reciprocal must still take the center slot
        // (0); the Up/Down reciprocal yields to a fanned slot.
        let mut g = MapGraph::new();
        for id in [1, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, -2)); // north-east: bent reciprocal N/S pair
        g.set_pos(3, (0, -2)); // due north: straight reciprocal Up/Down pair
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        g.add_edge(1, Direction::Up, 3);
        g.add_edge(3, Direction::Down, 1);

        let plan = route_lanes(&g);
        let top: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| c.origin == 1 && c.exit == Side::Top)
            .collect();
        assert_eq!(top.len(), 2, "expected both connectors sharing room 1's Top side: {top:?}");
        let ns = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::N | Direction::S))
            .expect("N/S connector on room 1's Top side");
        let updown = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .expect("up/down connector on room 1's Top side");
        assert_eq!(ns.exit_slot, 0, "reciprocal N/S takes the center slot");
        assert_ne!(updown.exit_slot, 0, "up/down yields to a fanned slot");
    }

    #[test]
    fn straight_updown_keeps_center_over_weaving_oneway_compass() {
        // Room 1 has a STRAIGHT Up/Down reciprocal to room 2 (due north) and a WEAVING
        // one-way compass edge N to room 3 (north-east; reverse edge SW, so bidirectional but
        // NOT a cardinal N/S reciprocal). Both exits land on room 1's Top side. Unlike a
        // reciprocal N/S (which keeps the center slot, see `ns_reciprocal_outranks_updown…`),
        // a non-axis-reciprocal weaving compass connector must YIELD the center to the straight
        // Up/Down line so the straight line does not jog across it (SQ-0222).
        let mut g = MapGraph::new();
        for id in [1, 2, 3] { g.upsert_room(id, "r".into()); }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1)); // due north: straight reciprocal Up/Down pair
        g.set_pos(3, (1, -1)); // north-east: weaving one-way compass (N out, SW back)
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1);
        g.add_edge(1, Direction::N, 3);
        g.add_edge(3, Direction::SW, 1);

        let plan = route_lanes(&g);
        let top: Vec<_> = plan
            .connectors
            .iter()
            .filter(|c| c.origin == 1 && c.exit == Side::Top)
            .collect();
        assert_eq!(top.len(), 2, "expected both connectors sharing room 1's Top side: {top:?}");
        let updown = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .expect("up/down connector on room 1's Top side");
        let compass = top
            .iter()
            .find(|c| matches!(c.exit_dir, Direction::N | Direction::S))
            .expect("compass connector on room 1's Top side");
        assert_eq!(updown.exit_slot, 0, "the straight up/down keeps the center slot");
        assert_ne!(compass.exit_slot, 0, "the weaving one-way compass yields to a fanned slot");
    }

    #[test]
    fn route_side_puts_up_on_top_and_down_on_bottom() {
        use crate::direction::Direction;
        use crate::router::route_side;
        use crate::router::Side;
        // Up always departs the NORTH (top) border; Down always the SOUTH (bottom).
        assert_eq!(route_side(Direction::Up), Some(Side::Top));
        assert_eq!(route_side(Direction::Down), Some(Side::Bottom));
    }

    #[test]
    fn compass_edge_suppresses_updown_connector_on_same_pair() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;

        // A--North-->B AND A--Up-->B on the same pair: only the compass path is drawn.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let updown: Vec<_> = plan.connectors.iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(updown.len(), 0, "the up/down connector is suppressed when a compass edge shares the pair");
        let compass: Vec<_> = plan.connectors.iter()
            .filter(|c| matches!(c.exit_dir, Direction::N | Direction::S))
            .collect();
        assert_eq!(compass.len(), 1, "the compass connector is still drawn");
    }

    #[test]
    fn suppressed_updown_does_not_perturb_contested_direct_route_priority() {
        // Regression for the SQ-0219 mid-loop-`continue` bug: `direct_route_losers` picks the
        // FIRST-LISTED edge per room pair as that pair's "representative" candidate for the
        // longest-straight-line contest. If a suppressed Up/Down edge is still present in the
        // `compass` slice passed to that pre-pass (merely skipped later, inside the main loop),
        // it steals the representative slot for its pair — so the REAL compass edge for that
        // pair is never entered into the contest, and can wrongly win (or lose) a contested
        // direct room-line against another edge's route. The fix must exclude the up/down edge
        // from the working set BEFORE `direct_route_losers` runs, not just skip it later.
        //
        // T(1) is the contested destination. A(2, far) and B(3, near) both want a straight W
        // line into T down its column. Per `longer_direct_route_keeps_the_contested_line`, the
        // LONGER connector (A) must keep the straight line; the shorter (B) must weave. B also
        // carries an Up edge to T (listed FIRST, sharing B's pair) that SQ-0219 suppresses.
        let build = |with_up: bool| {
            let mut g = MapGraph::new();
            for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
            g.set_pos(1, (0, 0)); // T
            g.set_pos(2, (3, 4)); // A (far)
            g.set_pos(3, (1, 2)); // B (near)
            if with_up {
                g.add_edge(3, Direction::Up, 1); // shares B's pair; listed first; suppressed
            }
            g.add_edge(3, Direction::W, 1); // B -> T, short contested line
            g.add_edge(2, Direction::W, 1); // A -> T, long contested line
            g
        };
        let with_up = route_lanes(&build(true));
        let without_up = route_lanes(&build(false));
        let b_with = with_up.connectors.iter().find(|c| c.origin == 3 && c.dest == 1)
            .expect("B->T connector is routed");
        let b_without = without_up.connectors.iter().find(|c| c.origin == 3 && c.dest == 1)
            .expect("B->T connector is routed");
        assert_eq!(b_with.merge, b_without.merge,
            "B->T's merge flag must not change based on a suppressed Up edge sharing its pair");
        assert_eq!(b_with.points, b_without.points,
            "B->T's polyline (and hence who wins the contested direct line) must be byte-identical \
             with or without the suppressed Up edge: {:?} vs {:?}", b_with.points, b_without.points);
        let a_with = with_up.connectors.iter().find(|c| c.origin == 2 && c.dest == 1)
            .expect("A->T connector is routed");
        let a_without = without_up.connectors.iter().find(|c| c.origin == 2 && c.dest == 1)
            .expect("A->T connector is routed");
        assert_eq!(a_with.points, a_without.points,
            "A->T's polyline must also be unaffected by B's suppressed Up edge: {:?} vs {:?}", a_with.points, a_without.points);
    }

    #[test]
    fn lone_updown_pair_is_unaffected_by_suppression() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;

        // Only an Up edge, no compass edge on this pair: the up/down connector survives.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);

        let plan = route_lanes(&g);
        let updown: Vec<_> = plan.connectors.iter()
            .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
            .collect();
        assert_eq!(updown.len(), 1, "a pair with no compass edge keeps its up/down connector");
    }
}
