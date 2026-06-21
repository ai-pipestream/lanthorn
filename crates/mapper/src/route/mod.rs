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

/// Build the doubled-coord polyline for one connector: centre → exit stub → lattice →
/// horizontal run → vertical run → lattice → entry stub → centre.
fn build_points(a_cell: (i32, i32), exit: Side, b_cell: (i32, i32), entry: Side) -> Vec<(i32, i32)> {
    let ca = cell_to_doubled(a_cell);
    let cb = cell_to_doubled(b_cell);
    let ea = exit_point(a_cell, exit);
    let eb = exit_point(b_cell, entry);
    let ga = snap_to_lattice(ea, cb); // first all-odd point
    let gb = snap_to_lattice(eb, ca); // last all-odd point
    // L on the gap lattice: ga → corner(gb.x, ga.y) → gb (horizontal then vertical).
    let corner = (gb.0, ga.1);
    let mut pts = vec![ca, ea, ga];
    if corner != ga { pts.push(corner); }
    if gb != corner { pts.push(gb); }
    pts.push(eb);
    pts.push(cb);
    // Drop consecutive duplicates.
    pts.dedup();
    pts
}

/// Route every drawn (compass) edge into a connector polyline. Reciprocal pairs collapse
/// to one (keep the lower-origin-id direction).
pub fn route_topology(graph: &MapGraph) -> Vec<RoutedConnector> {
    let mut seen: std::collections::BTreeSet<(RoomId, RoomId)> = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for c in graph.connections() {
        if grid_offset(c.dir).is_none() {
            continue; // stub, not routed
        }
        let key = (c.origin.min(c.dest), c.origin.max(c.dest));
        if !seen.insert(key) {
            continue; // reciprocal partner already routed
        }
        let (Some(a), Some(b)) = (graph.room(c.origin).and_then(|r| r.pos),
                                  graph.room(c.dest).and_then(|r| r.pos)) else { continue; };
        let Some(exit) = side_for(c.dir) else { continue; };
        let entry = entry_side(a, b);
        out.push(RoutedConnector {
            origin: c.origin,
            dest: c.dest,
            distorted: c.distorted,
            exit,
            entry,
            points: build_points(a, exit, b, entry),
            segs: Vec::new(),
        });
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

/// Full logical route: topology + lane assignment.
pub fn route_lanes(graph: &MapGraph) -> RoutePlan {
    let mut connectors = route_topology(graph);

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
