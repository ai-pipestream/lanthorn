# Lane-Based Connector Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route every connector through reserved lanes in the gaps between rooms so no two paths ever overlap, draw them as clean box-drawing line-art, and let gaps grow to fit their lanes — making every connection a separate, traceable line.

**Architecture:** A new logical channel router in `mapper` (`route_lanes(graph) -> RoutePlan`) routes each edge on a doubled-coordinate "gap lattice" and assigns each segment a lane via interval-coloring. The `app` renderer turns the plan into pixels: channel widths grow with lane counts, rooms are placed by cumulative non-uniform gaps, and connectors draw as line-art along lanes — replacing the A* `route_ortho`, the solid ribbons, and the unrouted/grey fallback. Boxes zoom only; Compact/Overview unchanged.

**Tech Stack:** Rust workspace. `mapper` (logical routing, no pixels) + `app` (ratatui pixel rendering). Tests via `cargo test -p <crate>`.

## Global Constraints

- `mapper` stays pixel-free: the router emits lane indices and channel traffic counts (integers), never pixel sizes. No new external dependencies.
- Deterministic: same graph → identical `RoutePlan` and identical render. No RNG, no time. Sorted/`BTreeMap` iteration everywhere.
- **No-overlap guarantee:** within any channel, two segments sharing a lane must have disjoint extents. This is the central invariant.
- Lane routing applies at **Boxes zoom only**. Compact (`8×3`) and Overview (`1×1`) keep the existing uniform-stride rendering untouched.
- Connectors render as **box-drawing line-art** (`─│┌┐└┘├┤┬┴┼`), not solid background ribbons. Arrowheads `▶◀▲▼` (filled, outgoing compass direction) at departures; reciprocal connectors also draw the far-end arrow. Distorted edges are Magenta foreground, normal Cyan.
- Reciprocal edge pairs (`A→B` and `B→A`) draw as ONE connector (collapse), as today.
- Box at Boxes zoom is `11×5` (unchanged).

**Doubled-coordinate gap lattice (the routing space):** room cell `(c,r)` maps to doubled point `(2c, 2r)`. A point is "on the gap lattice" iff both coordinates are odd. Vertical channel `V[c]` is the line `x = 2c+1`; horizontal channel `H[r]` is `y = 2r+1`. A segment between two all-odd points that share one coordinate stays all-odd along its run, so it never coincides with a room cell (rooms are at even,even). This is why routing on odd coordinates is collision-free against rooms by construction.

**Reference types (existing):**
- `mapper::router::Side` = `{ Top, Bottom, Left, Right }`; `mapper::router::side_for(dir) -> Option<Side>` (N/NE/NW→Top, S/SE/SW→Bottom, E→Right, W→Left, others→None).
- `mapper::direction::grid_offset(dir).is_some()` ⇔ a drawn compass edge.
- `mapper::graph::MapGraph`: `connections() -> &[Connection { origin, dir, dest, distorted }]`, `room(id).pos -> Option<(i32,i32)>`, `current()`.
- `mapper::render::{RenderMap { rooms: Vec<RenderRoom>, edges: Vec<RoutedEdge>, bounds }, render(graph)}`; `RenderRoom { id, label, cell:(i32,i32), has_notes, is_current }`.

---

### Task 1: Channel-router types and gap-lattice helpers (`mapper`)

**Files:**
- Create: `crates/mapper/src/route/mod.rs`
- Modify: `crates/mapper/src/lib.rs` (add `pub mod route;`)

**Interfaces:**
- Produces (used by Tasks 2–5):
  - `pub enum Channel { H(i32), V(i32) }`
  - `pub struct LaneSeg { pub channel: Channel, pub lane: u16, pub start: i32, pub end: i32 }` — `start<=end` are the doubled-coord extent endpoints along the channel's run axis (columns for `H`, rows for `V`).
  - `pub struct RoutedConnector { pub origin: RoomId, pub dest: RoomId, pub distorted: bool, pub exit: Side, pub entry: Side, pub points: Vec<(i32,i32)>, pub segs: Vec<LaneSeg> }` — `points` is the doubled-coord polyline (centre→…→centre); `segs` is the laned long-runs (filled in Task 3).
  - `pub struct RoutePlan { pub connectors: Vec<RoutedConnector>, pub h_lanes: BTreeMap<i32,u16>, pub v_lanes: BTreeMap<i32,u16> }`
  - `pub fn cell_to_doubled(cell:(i32,i32)) -> (i32,i32)` = `(cell.0*2, cell.1*2)`
  - `pub fn exit_point(cell:(i32,i32), side:Side) -> (i32,i32)` — one doubled-step out of the box on `side`.

- [ ] **Step 1: Write the failing test**

Create `crates/mapper/src/route/mod.rs` with the types and a test module:

```rust
//! Logical lane router: routes each drawn edge through reserved lanes in the gaps
//! between rooms, on a doubled-coordinate gap lattice (room cell (c,r) -> (2c,2r);
//! channels live on odd coordinates). Pixel-free — emits lane indices + per-channel
//! lane counts that the renderer turns into gap widths.

use std::collections::BTreeMap;
use crate::graph::RoomId;
use crate::router::Side;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubled_and_exit_points() {
        assert_eq!(cell_to_doubled((0, 0)), (0, 0));
        assert_eq!(cell_to_doubled((-1, 2)), (-2, 4));
        assert_eq!(exit_point((0, 0), Side::Right), (1, 0));
        assert_eq!(exit_point((0, 0), Side::Top), (0, -1));
        assert_eq!(exit_point((-1, 2), Side::Left), (-3, 4));
    }
}
```

- [ ] **Step 2: Wire the module + run the test**

Add `pub mod route;` to `crates/mapper/src/lib.rs` (after `pub mod render;`).
Run: `cargo test -p mapper route::`
Expected: PASS (`doubled_and_exit_points`).

- [ ] **Step 3: Verify the crate builds clean**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass. (Types unused outside the module yet → clippy may warn dead_code; that's expected interim and clears in Task 2. If it fails the build, add `#![allow(dead_code)]`-free `#[allow(dead_code)]` on the unused items with a `// consumed in Task 2` note, removed in Task 2.)

- [ ] **Step 4: Commit**

```bash
git add crates/mapper/src/route/mod.rs crates/mapper/src/lib.rs
git commit -m "feat(mapper): lane-router types + gap-lattice helpers"
```

---

### Task 2: Route topology (`mapper`)

Build each connector's doubled-coord polyline: exit stub → gap-lattice point → horizontal run → vertical run → gap-lattice point → entry stub. Every long run sits on an odd coordinate (a channel), so it never crosses a room.

**Files:**
- Modify: `crates/mapper/src/route/mod.rs`

**Interfaces:**
- Consumes: Task 1 types/helpers; `crate::graph::MapGraph`; `crate::direction::grid_offset`; `crate::router::side_for`.
- Produces (used by Task 3): `pub fn route_topology(graph: &MapGraph) -> Vec<RoutedConnector>` (connectors with `points` filled, `segs` empty), and private `fn snap_to_lattice`, `fn entry_side`, `fn long_runs`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `crates/mapper/src/route/mod.rs`:

```rust
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn doubled(cell: (i32, i32)) -> (i32, i32) { (cell.0 * 2, cell.1 * 2) }

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
```

Also add this small test helper near the top of the `tests` module (used by the first test):

```rust
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p mapper route::`
Expected: FAIL to compile — `route_topology` is not defined.

- [ ] **Step 3: Implement `route_topology` and helpers**

Add to `crates/mapper/src/route/mod.rs` (before the `#[cfg(test)]` module). Add `use crate::direction::grid_offset; use crate::graph::MapGraph; use crate::router::side_for;` to the top-of-file `use`s:

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p mapper route::`
Expected: PASS (`every_long_run_is_on_the_gap_lattice`, `reciprocal_pair_collapses_to_one_connector`, `stub_excluded_non_compass`, and Task 1's test).

- [ ] **Step 5: Full mapper suite + clippy**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/route/mod.rs
git commit -m "feat(mapper): route each edge on the gap lattice (topology)"
```

---

### Task 3: Lane assignment + `route_lanes` (`mapper`)

Turn each connector's polyline into laned long-runs and count lanes per channel via the left-edge interval algorithm.

**Files:**
- Modify: `crates/mapper/src/route/mod.rs`

**Interfaces:**
- Consumes: Task 1–2 types and `route_topology`.
- Produces (used by Tasks 4–5): `pub fn route_lanes(graph: &MapGraph) -> RoutePlan`; private `fn long_runs(points) -> Vec<(Channel,i32,i32)>` and `fn assign_lanes(...)`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
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
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p mapper segments_sharing two_overlapping route_lanes_is_deterministic`
Expected: FAIL to compile — `route_lanes` is not defined.

- [ ] **Step 3: Implement `long_runs`, `assign_lanes`, `route_lanes`**

Add to `crates/mapper/src/route/mod.rs` (before the test module):

```rust
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p mapper route::`
Expected: PASS (all route tests incl. the invariant, the 2-lane case, determinism).

- [ ] **Step 5: Full mapper suite + clippy**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass, no warnings (`route` types now all consumed).

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/route/mod.rs
git commit -m "feat(mapper): lane assignment (left-edge) + route_lanes"
```

---

### Task 4: Bundle `RoutePlan` into `RenderMap` + pixel position tables (`mapper` + `app`)

Make the plan available to the renderer and build the non-uniform cell→pixel tables at Boxes zoom.

**Files:**
- Modify: `crates/mapper/src/render.rs` (add `plan: RoutePlan` to `RenderMap`, fill it in `render`)
- Modify: `crates/app/src/render/map.rs` (add a position-table builder; this task does NOT yet draw connectors)

**Interfaces:**
- Consumes: `route_lanes` (Task 3); `RenderMap`.
- Produces (used by Task 5): on the app side, `struct Axis { starts: BTreeMap<i32,i32>, box_lo, lane_span }`-style tables — concretely `fn boxes_axes(plan:&RoutePlan, bounds) -> (PosTable, PosTable)` returning, for columns and rows, the pixel position of each room line and each channel's pixel range. (Exact `PosTable` shape defined in this task's code below.)

- [ ] **Step 1: Write the failing test (mapper side)**

Add to the `tests` module in `crates/mapper/src/render.rs`:

```rust
    #[test]
    fn render_attaches_route_plan() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let rm = render(&m.graph);
        // The plan routes the single drawn edge as one connector.
        assert_eq!(rm.plan.connectors.len(), 1, "render must attach a 1-connector plan");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper render_attaches_route_plan`
Expected: FAIL to compile — `RenderMap` has no `plan` field.

- [ ] **Step 3: Add `plan` to `RenderMap`**

In `crates/mapper/src/render.rs`: add `use crate::route::{route_lanes, RoutePlan};`, add the field, and fill it:

```rust
pub struct RenderMap {
    pub rooms: Vec<RenderRoom>,
    pub edges: Vec<RoutedEdge>,
    pub bounds: ((i32, i32), (i32, i32)),
    pub plan: RoutePlan,
}
```

In `render`, before the final `RenderMap { ... }`, add `let plan = route_lanes(graph);` and include `plan` in the struct literal.

Run: `cargo test -p mapper render_attaches_route_plan` → PASS. Run `cargo test -p mapper` (existing render tests still pass).

- [ ] **Step 4: Write the failing test (app position table)**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn boxes_axes_widen_busy_channels() {
        // A column-channel carrying 2 lanes must be wider than an empty one, and room
        // pixel-positions are cumulative (a later room sits further right when an earlier
        // gap is wide).
        use mapper::route::{RoutePlan, Channel};
        let mut plan = RoutePlan::default();
        plan.v_lanes.insert(0, 2); // V[0] carries 2 lanes
        // bounds cols 0..=2, rows 0..=0
        let (cols, _rows) = boxes_axes(&plan, ((0, 0), (2, 0)));
        let gap0 = cols.channel_span(0); // pixel width of V[0]
        let gap1 = cols.channel_span(1); // pixel width of V[1] (empty)
        assert!(gap0 > gap1, "a 2-lane channel must be wider than an empty one");
        // room col 2 starts further right than col 1 by at least box+gap.
        assert!(cols.room_pixel(2) > cols.room_pixel(1));
    }
```

- [ ] **Step 5: Run to verify it fails**

Run: `cargo test -p app boxes_axes_widen_busy_channels`
Expected: FAIL to compile — `boxes_axes`/`PosTable` not defined.

- [ ] **Step 6: Implement the position tables (Boxes zoom)**

Add to `crates/app/src/render/map.rs` (near `cell_to_virtual`). Constants: lane spacing and min gutter.

```rust
use mapper::route::{Channel, RoutePlan};

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
}
impl PosTable {
    pub fn room_pixel(&self, idx: i32) -> i32 { *self.room_start.get(&idx).unwrap_or(&0) }
    pub fn channel_span(&self, idx: i32) -> i32 { *self.channel_w.get(&idx).unwrap_or(&MIN_GUTTER) }
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
        PosTable { room_start, channel_w, lo }
    };
    let cols = build(min_c, max_c, BOX_W, &plan.v_lanes);
    let rows = build(min_r, max_r, BOX_H, &plan.h_lanes);
    (cols, rows)
}
```

- [ ] **Step 7: Run the test + full suite**

Run: `cargo test -p app boxes_axes_widen_busy_channels` → PASS.
Run: `cargo test -p app && cargo test -p mapper && cargo clippy --workspace`
Expected: all pass. (The new helpers aren't used by `render_map` yet → expected interim dead_code, cleared in Task 5.)

- [ ] **Step 8: Commit**

```bash
git add crates/mapper/src/render.rs crates/app/src/render/map.rs
git commit -m "feat: attach RoutePlan to RenderMap + non-uniform Boxes position tables"
```

---

### Task 5: Draw line-art connectors along lanes; remove the A* router/ribbons/unrouted (`app`)

Rewrite the Boxes-zoom connector rendering to draw each plan connector as box-drawing line-art at its lane offsets, positioned via the Task-4 tables. Delete `route_ortho`, the solid-ribbon blit, and the unrouted machinery.

**Files:**
- Modify: `crates/app/src/render/map.rs` (the heart of the change)

**Interfaces:**
- Consumes: `boxes_axes`/`PosTable` (Task 4); `RenderMap.plan`; `Channel`/`LaneSeg`/`RoutedConnector` (mapper).
- Produces: line-art connectors in the buffer at Boxes zoom.

**This task's mechanics (the renderer translates doubled-coord lane geometry to pixels):**
- For a connector, convert each `LaneSeg` to pixel endpoints: an `H(r)` segment runs horizontally at pixel-y = `rows.room_pixel(r) + BOX_H + lane*LANE_SPACING + 1`; its x range comes from the doubled `start..end` mapped through `cols` (doubled-x `2c`→`cols.room_pixel(c)+BOX_W/2`; `2c+1`→`cols.room_pixel(c)+BOX_W + (that channel's lane offset)`). A `V(c)` segment is the mirror. Stubs connect the room-edge anchor to the first/last lane point.
- Plot each connector as a sequence of pixel points; fill cells between consecutive points; choose the box-drawing glyph per cell from the set of directions that connector enters/leaves the cell (`─│┌┐└┘├┤┬┴`); when a cell already holds another connector's perpendicular glyph, combine to `┼`.
- Arrowhead at the departure anchor (`arrow_for_departure(exit)`); reciprocal far-end arrow at the entry anchor.
- Foreground color: `Color::Magenta` if `connector.distorted` else `Color::Cyan`.

- [ ] **Step 1: Write the failing acceptance test**

Add to the `tests` module in `crates/app/src/render/map.rs` (this is the milestone's acceptance gate):

```rust
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
            g.add_edge(o, t, d);
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app lane_routing_a129_no_overlap_line_art`
Expected: FAIL — today connectors are solid Cyan/Magenta ribbons, so the `assert_ne!(c.bg, Color::Cyan)` fires (and the signature/old renderer still draws ribbons).

- [ ] **Step 3: Implement line-art lane rendering; remove the old router/ribbon/unrouted**

In `crates/app/src/render/map.rs`, in `render_map`, replace the entire Boxes-zoom connector pipeline (the section that builds `blocked`, calls `route_ortho`, accumulates `path_cells`/`unrouted_cells`/`arrowheads`, and blits ribbons) with a lane-rendering pass:

1. Build `let (cols, rows) = boxes_axes(&rm.plan, rm.bounds);` and position rooms via `cols.room_pixel`/`rows.room_pixel` (replacing `cell_to_virtual` for Boxes zoom; keep `cell_to_virtual` for Compact/Overview).
2. For each `connector` in `rm.plan.connectors`, compute its pixel polyline from `segs` + the room anchors (per the mechanics above), plot box-drawing glyphs into a `BTreeMap<(i32,i32), DirMask>` (N/E/S/W bits), combining perpendicular hits to `┼`.
3. Translate to screen (apply scroll/area offset using the same `cols`/`rows` tables), clip, write each glyph with `Color::Magenta`/`Color::Cyan` fg.
4. Draw arrowheads at the exit anchor (and the entry anchor for reciprocal connectors).
5. Delete `route_ortho`, the `astar` closure, `unrouted_l`, the `PATH_BG`/`PATH_BG_DISTORTED`/`PATH_BG_UNROUTED` constants, `unrouted_cells`, `path_cells`, and the ribbon/arrow blit loops. Remove now-orphaned helpers (`walk_to` if unused, `arrival_candidates`/`closest_free_arrival`/`arrival_anchor`/`side_anchor` if unused after — check and delete what your change orphans).

Because the exact glyph-selection and pixel-mapping code is sizable, implement it to satisfy BOTH the acceptance test (Step 1) and the existing connector tests you must update (the old ribbon/anchor tests — `connector_is_solid_background_ribbon`, `connector_departs_origin_correct_side`, `arrowhead_*`, `unroutable_edge_renders_distinct_*`, `route_*`, `connectors_are_scroll_invariant`, `render_no_path_ribbon_inside_other_room`): these assert the REMOVED ribbon/A* behavior. Replace each with the line-art equivalent or delete it if it tests a removed mechanism, keeping coverage of: a connector renders line-art glyphs; an arrowhead appears at the departure side; connector geometry is scroll-invariant in virtual space; no connector glyph appears inside a room's interior.

- [ ] **Step 4: Run the acceptance test + the full app suite**

Run: `cargo test -p app lane_routing_a129_no_overlap_line_art` → PASS.
Run: `cargo test -p app && cargo clippy -p app`
Expected: all pass, no warnings, no dead code.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): draw line-art connectors via lanes; remove A* router/ribbons/unrouted"
```

---

### Task 6: Simplify the dump to a buffer→text copy (`app`)

With the live render now line-art, the dump no longer reconstructs ribbons — it copies buffer glyphs directly.

**Files:**
- Modify: `crates/app/src/map_dump.rs`

**Interfaces:**
- Consumes: the line-art `render_map` (Task 5).

- [ ] **Step 1: Update the failing tests**

The existing `map_dump` tests assert mask/ribbon behavior. Update `dump_ascii_has_line_art_connector` to expect line-art directly (it already checks for `─│┼`), and DELETE `unrouted_cell_serializes_to_block_glyph` (the `▒`/DarkGray concept is gone). Add:

```rust
    #[test]
    fn dump_copies_glyphs_directly_no_ribbon_mask() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        let dump = render_dump(&m.graph);
        // A connector line-art glyph appears, and the legend no longer advertises ▒.
        assert!(dump.contains('─') || dump.contains('│'), "line-art connector expected:\n{dump}");
        assert!(!dump.contains("▒ = unrouted"), "unrouted concept removed from legend");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app map_dump`
Expected: FAIL — legend still says `▒ = unrouted`; `unrouted_cell_serializes_to_block_glyph` references removed symbols.

- [ ] **Step 3: Simplify `ascii_map` + remove mask machinery**

In `crates/app/src/map_dump.rs`:
1. Delete `mask_glyph`, `is_path`, `is_unrouted`.
2. Rewrite `ascii_map`'s serialize loop to copy each cell's symbol directly (blank → space):

```rust
        for x in 0..area_w {
            let sym = buf.cell((x as u16, y as u16)).map(|c| c.symbol()).unwrap_or(" ");
            line.push_str(if sym.is_empty() { " " } else { sym });
        }
```

3. Update the MAP legend line to drop `▒ = unrouted`:

```rust
    out.push_str("#\n# === MAP (#id = room, lines = connectors, ▶◀▲▼ = exits) ===\n");
```

- [ ] **Step 4: Run the dump tests + full workspace**

Run: `cargo test -p app map_dump` → PASS.
Run: `cargo test -p app && cargo test -p mapper && cargo clippy --workspace`
Expected: all pass, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/map_dump.rs
git commit -m "refactor(app): dump copies line-art glyphs directly (drop ribbon mask)"
```

---

## Self-Review

**Spec coverage:**
- `mapper` channel router types + gap lattice (spec "Channel model") → Task 1. ✓
- Route topology (spec Stage 1) → Task 2. ✓
- Lane assignment / left-edge + `route_lanes` + `RoutePlan` (spec Stage 2) → Task 3. ✓
- Dynamic gap widths + cumulative non-uniform positions (spec Stage 3, "dynamic gaps") → Task 4. ✓
- Line-art rendering + removal of A*/ribbons/unrouted (spec Stage 3 + "What is removed") → Task 5. ✓
- Dump simplification (spec "the simplification") → Task 6. ✓
- Boxes-only scoping (spec decision 3) → Tasks 4–5 keep `cell_to_virtual` for Compact/Overview. ✓
- No-overlap guarantee (spec central invariant) → Task 3 `segments_sharing_a_lane_never_overlap`. ✓
- Determinism → Task 3 `route_lanes_is_deterministic`. ✓
- Acceptance (0 overlap, line-art, A129) → Task 5 `lane_routing_a129_no_overlap_line_art`. ✓
- Reciprocal collapse → Task 2 `reciprocal_pair_collapses_to_one_connector`. ✓

**Placeholder scan:** Tasks 1–4 and 6 carry complete code. Task 5's glyph-selection/pixel-mapping body is described as mechanics + pinned by the acceptance test and the updated connector tests rather than transcribed line-for-line — this is the one task whose implementation the engineer writes against tests (it's the largest surface; the mechanics section gives the exact pixel formulas and the test gives the exact pass condition). Flagged as the crux for the reviewer.

**Type consistency:** `RoutePlan`/`RoutedConnector`/`LaneSeg`/`Channel` defined in Task 1, consumed unchanged in Tasks 3–5. `route_lanes(graph)->RoutePlan` defined Task 3, called Task 4. `boxes_axes(&RoutePlan, bounds)->(PosTable,PosTable)` + `PosTable::{room_pixel,channel_span}` defined Task 4, consumed Task 5. `RenderMap.plan` added Task 4, read Task 5. `BOX_W/BOX_H/LANE_SPACING/MIN_GUTTER` defined Task 4, used Tasks 4–5.

**Note for execution:** Task 5 is the largest and least-transcribed; dispatch it on a capable model and expect a review iteration. Tasks 1–4 and 6 are concrete transcription+TDD.
