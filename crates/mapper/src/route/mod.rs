//! Logical lane router: routes each drawn edge through reserved lanes in the gaps
//! between rooms, on a doubled-coordinate gap lattice (room cell (c,r) -> (2c,2r);
//! channels live on odd coordinates). Pixel-free — emits lane indices + per-channel
//! lane counts that the renderer turns into gap widths.

use std::collections::BTreeMap;
use crate::direction::grid_offset;
use crate::graph::{MapGraph, RoomId};
use crate::router::{side_for, Side};

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

/// Route every drawn (compass) edge. When `greedy` is false, each connector takes its
/// canonical route (horizontal-first L, geometric entry side). When true, each connector is
/// routed sequentially and picks — among its candidate routes (both L orientations, plus the
/// alternative facing entry side for non-reciprocal connectors) — the one that crosses the
/// fewest already-placed connectors, with a deterministic integer tiebreak.
fn route_topology_with(graph: &MapGraph, greedy: bool) -> Vec<RoutedConnector> {
    // Draw every compass edge on its OWN exit side, collapsing ANY bidirectional pair
    // (a→b together with b→a, regardless of the two directions) into a single connector
    // drawn once: the renderer puts an arrow at each end, each pointing out that end's own
    // compass side. An exact-opposite pair is the straight-line special case. Edges with no
    // reverse partner stay single one-arrow connectors.
    let compass: Vec<&crate::graph::Connection> = graph
        .connections()
        .iter()
        .filter(|c| grid_offset(c.dir).is_some())
        .collect();
    let mut drawn: std::collections::BTreeSet<(RoomId, RoomId)> = std::collections::BTreeSet::new();
    let mut out: Vec<RoutedConnector> = Vec::new();
    for c in &compass {
        // A reverse edge between the same room pair (any direction) makes this connector
        // bidirectional; the first-seen edge of the pair draws it once.
        let back = compass
            .iter()
            .find(|p| p.origin == c.dest && p.dest == c.origin)
            .copied();
        let has_reciprocal = back.is_some();
        if has_reciprocal && drawn.contains(&(c.dest, c.origin)) {
            continue; // the reciprocal partner already drew this pair
        }
        let (Some(a), Some(b)) = (graph.room(c.origin).and_then(|r| r.pos),
                                  graph.room(c.dest).and_then(|r| r.pos)) else { continue; };
        let Some(exit) = side_for(c.dir) else { continue; };

        // Generate candidate routes. The exit side is fixed by the edge's compass direction;
        // we vary the L orientation and (for non-reciprocal connectors, in greedy mode only)
        // the destination entry side among the sides still facing the origin. Candidate order
        // is fixed and integer-tiebroken, so the choice is deterministic.
        // A bidirectional connector enters the far room on the side ITS OWN back-edge departs,
        // so the far-end arrow points out the true compass direction (an exact-opposite pair
        // resolves to the opposite side = a straight reciprocal). Non-bidirectional connectors
        // use the geometric entry side (greedy mode probes facing alternatives).
        let entry_choices: Vec<Side> = match back.and_then(|bk| side_for(bk.dir)) {
            Some(s) => vec![s],
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
        });
        drawn.insert((c.origin, c.dest));
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
        endpoints.push((c.dest, c.entry, false, ci));
    }
    // Group by (room, side); assign slots within each group. A connector that runs STRAIGHT
    // through this side (a collinear polyline — its other end is axis-aligned with this room)
    // is given slot 0 (the side centre) ahead of weaving connectors, so it stays on a clean
    // straight line and the weaving ones take the offset cells. This keeps the renderer's
    // perpendicular stubs from forcing a straight connector to jog across a weaving one's
    // corner. Ties break by (connector index, is_exit) for determinism.
    // Endpoint within a (room, side) group: (straight-through?, is_exit, connector index).
    type Endpoint = (bool, bool, usize);
    let mut by_side: BTreeMap<(RoomId, Side), Vec<Endpoint>> = BTreeMap::new();
    for (room, side, is_exit, ci) in endpoints {
        let straight = is_collinear(&connectors[ci].points);
        by_side.entry((room, side)).or_default().push((straight, is_exit, ci));
    }
    for (_key, mut members) in by_side {
        // `straight` first (false sorts before true, so negate via Reverse).
        members.sort_by_key(|&(straight, is_exit, ci)| (std::cmp::Reverse(straight), ci, is_exit));
        for (slot, (_straight, is_exit, ci)) in members.into_iter().enumerate() {
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
        for id in 1..=4 { g.upsert_room(id, "r".into()); }
        // 1->2 and 3->4 both run horizontally across the same row band, overlapping in x.
        g.set_pos(1, (0, 0)); g.set_pos(2, (4, 0));
        g.set_pos(3, (1, 0)); g.set_pos(4, (3, 0));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(3, Direction::E, 4);
        let plan = route_lanes(&g);
        let max_h = plan.h_lanes.values().copied().max().unwrap_or(0);
        assert!(max_h >= 2, "two overlapping horizontal runs need ≥2 lanes; got {max_h}");
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
        g.set_pos(3, (2, 1));
        g.add_edge(1, Direction::E, 2); // exits room 1 on its Right
        g.add_edge(3, Direction::W, 1); // enters room 1 from the right (entry Right on room 1)
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
        for id in [1, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        // A: 1 -E-> 2, a short horizontal pair.
        g.set_pos(1, (0, 0));
        g.set_pos(2, (2, 0));
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
    fn stub_excluded_non_compass() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0));
        g.add_edge(1, Direction::Up, 2); // non-compass stub
        assert!(route_topology(&g).is_empty(), "non-compass edges are not routed");
    }
}

