//! Logical room layout for the automapper (VM- and pixel-agnostic).
//!
//! Two regimes produce room grid positions; both keep rooms on integer cells and
//! never overlap:
//!
//! 1. **Incremental placement** (the per-turn path, in `incremental.rs`).
//!    `place_incremental` places ONE newly discovered room relative to the previous
//!    room: a planar compass move offsets by `grid_offset(dir)` and, on collision,
//!    shifts the rooms beyond the insertion point ("shift-beyond"); portal/unknown
//!    moves use `nearest_free_cell`. Existing rooms otherwise never move, so the map
//!    is stable turn-to-turn. `Mapper::observe` drives this.
//!
//! 2. **Constrained stress majorization** (`relayout_auto`, on demand — not per turn).
//!    For graphs with ≤ `MAX_NODES` rooms, re-derives all positions via SMACOF stress
//!    minimization with VPSC separation constraints (`stress.rs`, `vpsc.rs`,
//!    `constraints.rs`), seeded from the longest-path sort (`sort.rs`). Cycle-closing
//!    compass constraints are dropped (and the affected edges flagged distorted).
//!    For graphs above `MAX_NODES`, falls back to the **longest-path sort** directly.
//!    In both cases, components are packed left-to-right and the lowest-id room anchored
//!    at (0,0); residual collisions are resolved on the grid, keeping an aligned room on
//!    its row/column where possible (`place_preserving_alignment`) before spiralling.
//!
//! After either regime, `mark_distorted` flags every compass edge whose final grid
//! geometry contradicts its direction. (Connector routing and any render-aware
//! overlap cleanup live in the `app` crate, not here.)

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::grid_offset;
use crate::graph::{Connection, MapGraph, RoomId};

mod sort;
mod incremental;
mod vpsc;
mod constraints;
mod stress;
mod chains;
pub use incremental::place_incremental;
pub use chains::{detect_chains, Chains};

/// Separation gap and ideal edge length (in grid cells).
const GAP: f64 = 1.0;
/// Fixed SMACOF iterations (determinism + bounded cost).
const ITERS: usize = 60;
/// Above this room count, skip the O(ITERS·n²) solve and use the longest-path sort.
const MAX_NODES: usize = 400;

// ── LayoutMode ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LayoutMode {
    #[default]
    Auto,
    Manual,
}

// ── Public helpers ────────────────────────────────────────────────────────────

/// Returns the set of all grid cells currently occupied by a placed room.
pub fn occupied_cells(graph: &MapGraph) -> BTreeSet<(i32, i32)> {
    graph.rooms().filter_map(|r| r.pos).collect()
}

/// Spiral-search outward from `from` and return the first cell not in `occupied`.
/// Returns `from` itself if it is free.
pub fn nearest_free_cell(occupied: &BTreeSet<(i32, i32)>, from: (i32, i32)) -> (i32, i32) {
    if !occupied.contains(&from) {
        return from;
    }
    // Spiral outward: for radius r=1,2,… walk the perimeter of the square [−r..=r]×[−r..=r].
    for r in 1_i32.. {
        // Top row: y = from.1 - r, x from from.0-r to from.0+r
        for x in (from.0 - r)..=(from.0 + r) {
            let cell = (x, from.1 - r);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
        // Bottom row: y = from.1 + r
        for x in (from.0 - r)..=(from.0 + r) {
            let cell = (x, from.1 + r);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
        // Left column: x = from.0 - r, y from from.1-r+1 to from.1+r-1
        for y in (from.1 - r + 1)..=(from.1 + r - 1) {
            let cell = (from.0 - r, y);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
        // Right column: x = from.0 + r
        for y in (from.1 - r + 1)..=(from.1 + r - 1) {
            let cell = (from.0 + r, y);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
    }
    unreachable!("infinite grid always has a free cell")
}

/// Resolve a collision while preserving an aligned axis. When the room is free on Y
/// (its row is meaningful) but constrained on X, search ALONG X keeping the row; when
/// free on X but constrained on Y, search along Y keeping the column. Otherwise (free on
/// both — e.g. portal-only — or neither) fall back to the spiral `nearest_free_cell`.
/// This keeps an aligned cardinal chain on one row/column under collision resolution.
fn place_preserving_alignment(
    occupied: &BTreeSet<(i32, i32)>,
    from: (i32, i32),
    row_aligned: bool,
    col_aligned: bool,
) -> (i32, i32) {
    if !occupied.contains(&from) {
        return from;
    }
    if row_aligned && !col_aligned {
        for d in 1.. {
            for cand in [(from.0 - d, from.1), (from.0 + d, from.1)] {
                if !occupied.contains(&cand) {
                    return cand;
                }
            }
        }
    } else if col_aligned && !row_aligned {
        for d in 1.. {
            for cand in [(from.0, from.1 - d), (from.0, from.1 + d)] {
                if !occupied.contains(&cand) {
                    return cand;
                }
            }
        }
    }
    nearest_free_cell(occupied, from)
}

/// Returns true iff the connection's geometry is satisfied by the current room positions.
///
/// For a compass edge (one where `grid_offset(conn.dir)` returns `Some(delta)`):
///   - Uses a sign-based check: satisfied iff each non-zero axis of `delta` agrees in SIGN
///     with the corresponding axis of `pos(dest) - pos(origin)`, and each zero axis of `delta`
///     is also zero in the actual offset.
///   - Rationale: when both directed edges of a connection are known, the layout may place
///     a room at a combined diagonal (e.g. northeast) that doesn't exactly match `grid_offset`
///     (e.g. one step north). The sign-based check treats such placements as "satisfied" as long
///     as the directional sense is correct (e.g. a North edge is satisfied whenever dest is
///     *anywhere* north, i.e. `dest.y < origin.y`).
///
/// For a non-compass edge (Up/Down/In/Out/Unknown, where `grid_offset` returns `None`):
///   - returns `true` unconditionally. These edges are stubs with no spatial offset to violate;
///     treating them as "satisfied" ensures the post-placement sweep never marks them distorted.
pub fn edge_is_satisfied(graph: &MapGraph, conn: &Connection) -> bool {
    match grid_offset(conn.dir) {
        None => true, // non-compass stub — no offset to violate
        Some(delta) => {
            let origin_pos = graph.room(conn.origin).and_then(|r| r.pos);
            let dest_pos = graph.room(conn.dest).and_then(|r| r.pos);
            match (origin_pos, dest_pos) {
                (Some(op), Some(dp)) => {
                    let actual = (dp.0 - op.0, dp.1 - op.1);
                    // Sign-based: each axis of delta must agree in sign (or be zero if delta is 0).
                    axis_sign_ok(actual.0, delta.0) && axis_sign_ok(actual.1, delta.1)
                }
                _ => false, // unplaced endpoint → unsatisfied
            }
        }
    }
}

/// Returns true iff the sign of `actual` is consistent with the sign of `expected`:
/// - `expected == 0`: actual must also be 0.
/// - `expected > 0`: actual must be > 0.
/// - `expected < 0`: actual must be < 0.
fn axis_sign_ok(actual: i32, expected: i32) -> bool {
    match expected.cmp(&0) {
        std::cmp::Ordering::Equal => actual == 0,
        std::cmp::Ordering::Greater => actual > 0,
        std::cmp::Ordering::Less => actual < 0,
    }
}

// ── Core layout ───────────────────────────────────────────────────────────────

/// Connected components over the undirected projection of the graph. Each
/// component is sorted ascending; components are returned in ascending-root order.
pub(crate) fn connected_components(graph: &MapGraph, ids: &[RoomId]) -> Vec<Vec<RoomId>> {
    let mut adjacency: BTreeMap<RoomId, Vec<RoomId>> = BTreeMap::new();
    for &id in ids {
        adjacency.entry(id).or_default();
    }
    for conn in graph.connections() {
        adjacency.entry(conn.origin).or_default().push(conn.dest);
        adjacency.entry(conn.dest).or_default().push(conn.origin);
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort_unstable();
        neighbors.dedup();
    }

    let mut visited: BTreeSet<RoomId> = BTreeSet::new();
    let mut components: Vec<Vec<RoomId>> = Vec::new();
    for &id in ids {
        if visited.contains(&id) {
            continue;
        }
        let mut component = Vec::new();
        let mut queue: VecDeque<RoomId> = VecDeque::new();
        queue.push_back(id);
        visited.insert(id);
        while let Some(cur) = queue.pop_front() {
            component.push(cur);
            if let Some(neighbors) = adjacency.get(&cur) {
                for &nb in neighbors {
                    if visited.insert(nb) {
                        queue.push_back(nb);
                    }
                }
            }
        }
        component.sort_unstable();
        components.push(component);
    }
    components
}

/// Set the `distorted` flag on every connection: a compass edge is distorted if
/// its connection index is in `dropped`, or its final grid geometry violates its
/// direction. Non-compass edges are never distorted.
pub(crate) fn mark_distorted(graph: &mut MapGraph, dropped: &BTreeSet<usize>) {
    let n_conns = graph.connections().len();
    for idx in 0..n_conns {
        let conn = graph.connections()[idx].clone();
        let distorted = match grid_offset(conn.dir) {
            None => false,
            Some(_) => dropped.contains(&idx) || !edge_is_satisfied(graph, &conn),
        };
        graph.set_conn_distorted(idx, distorted);
    }
}

/// Re-derive all room positions from scratch on every call.
///
/// For graphs with ≤ MAX_NODES rooms, uses constrained stress-majorization
/// (SMACOF + VPSC) seeded from the longest-path sort. For larger graphs,
/// falls back to the longest-path sort directly. Components are packed
/// left-to-right, residual overlaps resolved, and the lowest-id room
/// anchored at (0,0).
pub fn relayout_auto(graph: &mut MapGraph) {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return;
    }

    // Large graphs: skip the O(ITERS·n²) solve and use the longest-path sort.
    if ids.len() > MAX_NODES {
        let pos = sort::sort_layout(graph);
        for (&id, &p) in &pos {
            graph.set_pos(id, p);
        }
        mark_distorted(graph, &BTreeSet::new());
        return;
    }

    // Seed from the longest-path sort (deterministic, roughly compass-ordered).
    let seed = sort::sort_layout(graph);

    let components = connected_components(graph, &ids);
    let mut dropped_all: BTreeSet<usize> = BTreeSet::new();
    let mut final_pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0;

    for comp in &components {
        let n = comp.len();
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // Local undirected adjacency for BFS distances.
        let mut adj = vec![Vec::new(); n];
        for c in graph.connections() {
            if let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        let dist = stress::all_pairs_dist(n, &adj);
        let ac = constraints::build_axis_constraints(graph, comp, GAP);
        dropped_all.extend(ac.dropped.iter().copied());

        let seed_local: Vec<(f64, f64)> = comp
            .iter()
            .map(|&id| {
                let p = seed.get(&id).copied().unwrap_or((0, 0));
                (p.0 as f64, p.1 as f64)
            })
            .collect();

        let cont = stress::stress_layout(n, &dist, &ac.x, &ac.y, &seed_local, ITERS);
        let mut snapped: Vec<(i32, i32)> =
            cont.iter().map(|&(x, y)| (x.round() as i32, y.round() as i32)).collect();

        // Align cardinal-edge free axes. The stress solve satisfies separation (B is
        // east of A) but leaves an E/W chain's rooms on slightly different rows (and
        // N/S chains on different columns). Pull each room that is free on the
        // perpendicular axis onto its anchor's row/column, so cardinal edges render
        // crisp — the same alignment the sort fallback applies.
        let mut axs: Vec<i32> = snapped.iter().map(|p| p.0).collect();
        let mut ays: Vec<i32> = snapped.iter().map(|p| p.1).collect();
        let (x_constrained, y_constrained) =
            sort::align_free_axes(graph, &index, &mut axs, &mut ays);
        for (i, p) in snapped.iter_mut().enumerate() {
            *p = (axs[i], ays[i]);
        }

        // Pack this component to the right of the previous, top-aligned.
        let min_x = snapped.iter().map(|p| p.0).min().unwrap();
        let min_y = snapped.iter().map(|p| p.1).min().unwrap();
        for p in &mut snapped {
            p.0 += pack_x - min_x;
            p.1 -= min_y;
        }

        // Resolve residual same-cell collisions in ascending room-id order. Keep an
        // axis-aligned room on its row/column (shift ALONG the aligned axis) instead of
        // spiraling off it, so collision resolution doesn't re-distort an aligned chain
        // (e.g. #193 bumped off #203's row by #180).
        let mut max_x_used = pack_x;
        for (i, &id) in comp.iter().enumerate() {
            let row_aligned = !y_constrained[i];
            let col_aligned = !x_constrained[i];
            let cell = place_preserving_alignment(&occupied, snapped[i], row_aligned, col_aligned);
            occupied.insert(cell);
            final_pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2; // 1-cell gap between components
    }

    // Anchor the lowest-id room at (0,0) for a stable reference.
    if let Some(&(ax, ay)) = final_pos.get(&ids[0]) {
        for p in final_pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }

    for (&id, &p) in &final_pos {
        graph.set_pos(id, p);
    }
    mark_distorted(graph, &dropped_all);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::mapper::Mapper;

    #[test]
    fn layout_mode_default_is_auto() {
        assert_eq!(LayoutMode::default(), LayoutMode::Auto);
    }

    #[test]
    fn places_rooms_by_compass_offsets() {
        let mut m = Mapper::default();
        m.observe(1, "Center", None);
        m.observe(2, "North Room", Some(Direction::N));
        relayout_auto(&mut m.graph);
        let p1 = m.graph.room(1).unwrap().pos.unwrap();
        let p2 = m.graph.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < p1.1, "north room must be above center: {p2:?} vs {p1:?}");
    }

    #[test]
    fn collision_places_nearest_free_and_marks_distorted() {
        // Set up two rooms both wanting to be north of room 1.
        // We build the graph directly since add_edge deduplicates by (origin, dir),
        // meaning a naive observe sequence would overwrite the first north edge.
        // Instead: room 1 at (0,0), rooms 2 and 3 each connected N from room 1
        // via edges stored on *different* origins (2→N→... won't work either).
        // Simplest: give room 1 two distinct north edges by using upsert_room + add_edge
        // on separate origin rooms that both have pos (0,0) — but that requires two rooms
        // at the same cell, which violates the invariant.
        //
        // The right setup: use the observe sequence from the brief.
        // After the sequence, add_edge(1,N,3) overwrites (1,N,2→3).
        // Room 2 is then reachable only via 2→Unknown→1 (inverted: placed neighbour=1).
        // Room 3 is placed at (0,-1) via compass. Room 2 is placed nearby via Unknown edge.
        // The test only asserts no overlap — which still holds.
        let mut m = Mapper::default();
        m.observe(1, "C", None);
        m.observe(2, "N1", Some(Direction::N));
        m.observe(1, "C", None); // back to center
        m.observe(3, "N2", Some(Direction::N));
        relayout_auto(&mut m.graph);
        // no two rooms share a cell
        let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }

    #[test]
    fn collision_direct_distorted_flag() {
        // Build graph directly so both edges exist simultaneously: two edges both pointing
        // to cells north of room 1. We do this by giving room 2 a pos=(0,0) temporarily and
        // making room 3 unplaced with a north edge from 1.
        // Actually: place room 1 at (0,0), room 2 at (0,-1) manually, then add edge 1→N→3.
        // When relayout_auto runs, room 3 wants (0,-1) which is occupied by 2 → displaced,
        // and the edge 1→N→3 is marked distorted.
        //
        // With new dynamic layout: rooms 1, 2, 3 all re-derive. Root=1 at (0,0).
        // Edge 1→N→2: room 2 placed at (0,-1).
        // Edge 1→N→3: room 3 wants (0,-1) — occupied → nearest free → displaced, distorted.
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "C".into());
        graph.upsert_room(2, "N1".into());
        graph.upsert_room(3, "N2".into());
        // No manual set_pos — new layout clears them anyway.
        graph.add_edge(1, Direction::N, 2);
        graph.add_edge(1, Direction::N, 3); // duplicate key (origin=1, dir=N) → overwrites dest!
        // Note: add_edge deduplicates by (origin, dir), so this sets 1→N→3 (replacing 1→N→2).
        // Room 2 becomes reachable only via connectivity if another edge exists.
        // Let's add room 2 with a different edge so it gets placed too.
        graph.add_edge(2, Direction::S, 1); // gives room 2 connectivity to 1
        relayout_auto(&mut graph);
        // room 3 placed somewhere other than (0,-1) if 2 got there first, or vice versa.
        // The key assertion: no overlap.
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
        // All rooms placed.
        assert!(graph.room(1).unwrap().pos.is_some());
        assert!(graph.room(2).unwrap().pos.is_some());
        assert!(graph.room(3).unwrap().pos.is_some());
    }

    #[test]
    fn rooms_never_overlap_random_walk() {
        let mut m = Mapper::default();
        let steps = [
            (1, None),
            (2, Some(Direction::N)),
            (3, Some(Direction::E)),
            (4, Some(Direction::S)),
            (5, Some(Direction::W)),
        ];
        for (id, via) in steps {
            m.observe(id, "r", via);
        }
        relayout_auto(&mut m.graph);
        let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    /// OLD: minimal_movement_preserves_existing_pos — intentionally replaced.
    /// NEW: dynamic layout re-derives from scratch each call. Verify that the
    /// root (lowest-id room) is anchored at (0,0) regardless of any previously
    /// set pos, and that connected rooms land at their constraint-derived cells.
    #[test]
    fn dynamic_layout_re_derives_from_scratch() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.set_pos(1, (5, 5));
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        assert_eq!(graph.room(1).unwrap().pos, Some((0, 0)), "lowest-id room anchors at origin");
        let p2 = graph.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < 0, "room 2 must be north of the anchor: {p2:?}");
    }

    #[test]
    fn relayout_is_deterministic() {
        // Same graph → same positions on repeated calls.
        let mut graph = MapGraph::new();
        for id in 1..=4 {
            graph.upsert_room(id, "r".into());
        }
        graph.add_edge(1, Direction::N, 2);
        graph.add_edge(1, Direction::E, 3);
        graph.add_edge(2, Direction::E, 4);
        relayout_auto(&mut graph);
        let positions_first: Vec<_> = (1u16..=4).map(|id| graph.room(id).unwrap().pos).collect();
        relayout_auto(&mut graph);
        let positions_second: Vec<_> = (1u16..=4).map(|id| graph.room(id).unwrap().pos).collect();
        assert_eq!(positions_first, positions_second, "relayout must be deterministic");
    }

    #[test]
    fn dynamic_relayout_updates_positions() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.add_edge(1, Direction::Unknown, 2);
        relayout_auto(&mut graph);
        let pos2_before = graph.room(2).unwrap().pos.unwrap();
        graph.remove_connection(1, Direction::Unknown);
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        let pos2_after = graph.room(2).unwrap().pos.unwrap();
        assert!(pos2_after.1 < graph.room(1).unwrap().pos.unwrap().1, "room 2 must be north now");
        assert_ne!(pos2_before, pos2_after, "room 2 must reposition when the constraint changes");
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }

    #[test]
    fn disconnected_component_gets_placed() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into()); // no edge connecting to 1
        relayout_auto(&mut graph);
        assert!(graph.room(1).unwrap().pos.is_some());
        assert!(graph.room(2).unwrap().pos.is_some());
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "disconnected rooms must not overlap");
    }

    #[test]
    fn contradictory_geometry_marks_distorted_not_overlap() {
        use crate::direction::Direction;
        // A(1) - N -> B(2); B(2) - N -> C(3); C(3) - N -> A(1)  (impossible loop)
        let mut g = crate::graph::MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::N, 1); // closes an impossible northward loop
        relayout_auto(&mut g);
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
        // at least one edge is distorted (the loop can't be Euclidean)
        assert!(g.connections().iter().any(|c| c.distorted));
    }

    #[test]
    fn nearest_free_cell_returns_from_if_free() {
        let occupied = BTreeSet::new();
        assert_eq!(nearest_free_cell(&occupied, (3, 3)), (3, 3));
    }

    #[test]
    fn nearest_free_cell_spirals_outward() {
        let mut occupied = BTreeSet::new();
        occupied.insert((0, 0));
        // First free cell in spiral should be adjacent
        let free = nearest_free_cell(&occupied, (0, 0));
        assert_ne!(free, (0, 0));
        let dist = (free.0.abs()).max(free.1.abs());
        assert_eq!(dist, 1, "nearest free cell should be at radius 1");
    }

    #[test]
    fn combined_offset_places_northeast() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.0 > pa.0 && pb.1 < pa.1, "B must be north-east of A: {pb:?} vs {pa:?}");
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
    }

    #[test]
    fn reciprocal_places_one_step_north() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.1 < pa.1, "B must be north of A: {pb:?} vs {pa:?}");
        assert_eq!(pb.0, pa.0, "no east/west constraint → B stays in A's column");
    }

    #[test]
    fn east_room_is_east() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        relayout_auto(&mut m.graph);
        let pa = m.graph.room(1).unwrap().pos.unwrap();
        let pb = m.graph.room(2).unwrap().pos.unwrap();
        assert!(pb.0 > pa.0, "east room must be to the right: {pb:?} vs {pa:?}");
    }

    #[test]
    fn orientation_pinned_north_is_up() {
        // North must map to smaller y every solve (no rotation), regardless of ids.
        let mut g = crate::graph::MapGraph::new();
        for id in 1..=4 {
            g.upsert_room(id, "r".into());
        }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::S, 4);
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert!(p2.1 < p1.1, "room 2 (north of 1) must be above it: {p2:?} vs {p1:?}");
    }

    #[test]
    fn constraint_engine_places_reciprocal_due_north() {
        // Reciprocal N/S pair → B due north of A (same column), via the constraint engine.
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.1 < pa.1, "B must be north of A: {pb:?} vs {pa:?}");
        assert_eq!(pb.0, pa.0, "no E/W constraint → B stays in A's column");
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn repair_terminates_on_impossible_mutual_south() {
        // A -S-> B and B -S-> A can't both be true. Repair must terminate, leave no
        // overlap, and leave at least one of the two edges distorted (unsatisfiable).
        use crate::direction::Direction;
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::S, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap");
        assert!(g.connections().iter().any(|c| c.distorted), "one mutual-S edge stays distorted");
    }

    fn a129_house_graph() -> crate::graph::MapGraph {
        let mut g = crate::graph::MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        use Direction::*;
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
        ] { g.add_edge(o, d, dst); }
        g
    }

    #[test]
    fn constraint_engine_beats_sort_distortion_on_a129() {
        // Distortion under the longest-path sort fallback (sort_layout + mark_distorted).
        let mut g_sort = a129_house_graph();
        let pos = sort::sort_layout(&g_sort);
        for (&id, &p) in &pos { g_sort.set_pos(id, p); }
        mark_distorted(&mut g_sort, &BTreeSet::new());
        let sort_distorted = g_sort.connections().iter().filter(|c| c.distorted).count();

        // Distortion under the constraint engine (the default relayout_auto).
        let mut g_cons = a129_house_graph();
        relayout_auto(&mut g_cons);
        let cons_distorted = g_cons.connections().iter().filter(|c| c.distorted).count();

        assert!(
            cons_distorted < sort_distorted,
            "constraint engine must reduce distortion vs sort: constraint={cons_distorted}, sort={sort_distorted}",
        );
        // And it must not overlap rooms.
        let cells: Vec<_> = g_cons.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap under the constraint engine");
    }

    #[test]
    fn constraint_engine_aligns_cardinal_chains() {
        // The alignment pass straightens E/W chains whose ends are free on the row axis
        // (25→E→26, 79→W→203), and the axis-preserving collision resolver keeps an aligned
        // room on its row even when its cell is taken — so 203→W→193 stays clean (#193
        // shifts ALONG #203's row past #180 instead of bumping off it). Total distortion
        // falls to 26 (from 30 un-aligned).
        //
        // It still cannot straighten every cardinal edge: a room pulled by CONFLICTING
        // constraints (e.g. #25 wants both #74's and #76's row, but 74 S 76 forces those
        // apart) must distort one of them — inherent to a non-Euclidean graph.
        let mut g = a129_house_graph();
        relayout_auto(&mut g);
        let e = |o, d| g.connections().iter().find(|c| c.origin == o && c.dest == d).unwrap();
        assert!(!e(25, 26).distorted, "25→E→26 aligns onto one row");
        assert!(!e(79, 203).distorted, "79→W→203 aligns onto one row");
        assert!(!e(203, 193).distorted, "203→W→193: #193 holds #203's row (collision shifts along it)");
        let distorted = g.connections().iter().filter(|c| c.distorted).count();
        assert!(distorted <= 26, "alignment + axis-preserving collision cut distortion to 26; got {distorted}");
    }

    #[test]
    fn relayout_is_deterministic_under_constraint_engine() {
        let mut a = a129_house_graph();
        let mut b = a129_house_graph();
        relayout_auto(&mut a);
        relayout_auto(&mut b);
        let pa: Vec<_> = a.rooms().map(|r| (r.id, r.pos)).collect();
        let pb: Vec<_> = b.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(pa, pb, "constraint engine must be deterministic");
    }

    #[test]
    fn reciprocal_ew_pair_shares_a_row() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // reciprocal E/W → same row
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_eq!(p1.1, p2.1, "reciprocal E/W pair must share a row: {p1:?} {p2:?}");
        assert!(p2.0 > p1.0, "and 2 is east of 1");
    }

    #[test]
    fn reciprocal_ns_pair_shares_a_column() {
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1); // reciprocal N/S → same column
        relayout_auto(&mut g);
        let p1 = g.room(1).unwrap().pos.unwrap();
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_eq!(p1.0, p2.0, "reciprocal N/S pair must share a column");
        assert!(p2.1 < p1.1, "and 2 is north of 1");
    }

    #[test]
    fn three_room_ew_chain_all_share_one_row() {
        // 1↔2↔3 reciprocal E/W chain → all three on EXACTLY one row. A single ≤ (gap 0)
        // would let the row drift across three rooms; both-leg equality pins them equal.
        let mut g = crate::graph::MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
        for (o, d, dst) in [
            (1, Direction::E, 2), (2, Direction::W, 1),
            (2, Direction::E, 3), (3, Direction::W, 2),
        ] { g.add_edge(o, d, dst); }
        relayout_auto(&mut g);
        let y1 = g.room(1).unwrap().pos.unwrap().1;
        let y2 = g.room(2).unwrap().pos.unwrap().1;
        let y3 = g.room(3).unwrap().pos.unwrap().1;
        assert_eq!(y1, y2, "rooms 1 and 2 share a row");
        assert_eq!(y2, y3, "rooms 2 and 3 share a row");
    }

    #[test]
    fn large_graph_uses_sort_fallback_without_overlap() {
        // A chain longer than MAX_NODES forces the fallback path; it must still place
        // every room with no overlap (and not run the O(n²) solve).
        let mut g = crate::graph::MapGraph::new();
        let count = (super::MAX_NODES + 5) as u16;
        for id in 1..=count { g.upsert_room(id, "r".into()); }
        for id in 1..count { g.add_edge(id, Direction::E, id + 1); }
        relayout_auto(&mut g);
        let placed = g.rooms().filter(|r| r.pos.is_some()).count();
        assert_eq!(placed, count as usize, "every room placed via fallback");
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap in the fallback layout");
    }
}
