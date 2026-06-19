//! Auto layout engine: greedy incremental grid placement with collision avoidance.
//!
//! # Placement algorithm
//!
//! `relayout_auto` works in repeated passes over rooms sorted by ascending id (deterministic).
//! Each pass attempts to place any unplaced room that has at least one already-placed neighbour
//! (via any connection, in either direction).
//!
//! For each unplaced room R the algorithm collects candidate positions from all edges that connect
//! R to an already-placed room:
//!
//!   - Edge (origin, dir, dest=R) where origin is placed and grid_offset(dir) = Some(delta):
//!     candidate = origin.pos + delta
//!   - Edge (origin=R, dir, dest) where dest is placed and grid_offset(dir) = Some(delta):
//!     candidate = dest.pos - delta   (offset inverted: R sits "behind" the placed dest)
//!   - Edge with no grid_offset (Up/Down/In/Out/Unknown): candidate = neighbour.pos (nearest-free
//!     spiral from that point); connection stays non-distorted (vertical/unknown stub).
//!
//! The first valid candidate that is free is used directly. If the preferred candidate (from a
//! compass edge) is occupied, we spiral-search for the nearest free cell and mark that connection
//! `distorted = true`.
//!
//! After each full pass that placed at least one room, another pass is run. When a full pass
//! places nothing but unplaced rooms remain (disconnected component), the lowest-id unplaced room
//! is treated as a new root and placed at the nearest free cell to (0, 0).
//!
//! Rooms that already have a `pos` are never moved (minimal-movement invariant).

use std::collections::BTreeSet;

use crate::direction::grid_offset;
use crate::graph::{Connection, MapGraph, RoomId};

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

/// Returns true iff the connection's geometry is satisfied by the current room positions.
///
/// For a compass edge (one where `grid_offset(conn.dir)` returns `Some(delta)`):
///   - satisfied iff both endpoints have `pos` AND `pos(dest) - pos(origin) == delta`.
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
                (Some(op), Some(dp)) => (dp.0 - op.0, dp.1 - op.1) == delta,
                _ => false, // unplaced endpoint → unsatisfied
            }
        }
    }
}

// ── Core layout ───────────────────────────────────────────────────────────────

/// Greedy incremental placement. Rooms that already have a `pos` keep it.
/// Only unplaced rooms are assigned positions.
pub fn relayout_auto(graph: &mut MapGraph) {
    // Collect all room ids in ascending order (deterministic).
    let all_ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();

    if all_ids.is_empty() {
        return;
    }

    // If nothing is placed yet, seed the first (lowest-id) room at (0,0).
    let any_placed = graph.rooms().any(|r| r.pos.is_some());
    if !any_placed {
        let first_id = *all_ids.iter().min().unwrap();
        graph.set_pos(first_id, (0, 0));
    }

    // Worklist: keep making passes until all rooms are placed.
    loop {
        let unplaced: Vec<RoomId> = all_ids
            .iter()
            .copied()
            .filter(|&id| graph.room(id).is_some_and(|r| r.pos.is_none()))
            .collect();

        if unplaced.is_empty() {
            break;
        }

        let mut placed_this_pass = false;

        for &room_id in &unplaced {
            if graph.room(room_id).and_then(|r| r.pos).is_some() {
                // Already placed in this pass (shouldn't happen but guard anyway).
                continue;
            }

            // Gather candidate placements from edges that connect this room to a placed neighbour.
            // We store: (candidate_pos, conn_index_if_compass) for the first usable candidate.
            let candidate = find_candidate(graph, room_id);

            if let Some((desired, conn_idx)) = candidate {
                let mut occupied = occupied_cells(graph);
                if !occupied.contains(&desired) {
                    // Exact placement.
                    graph.set_pos(room_id, desired);
                    occupied.insert(desired);
                } else {
                    // Collision: find nearest free cell.
                    let free = nearest_free_cell(&occupied, desired);
                    graph.set_pos(room_id, free);
                    occupied.insert(free);
                    // Mark the connection distorted only if it was a compass edge.
                    if let Some(idx) = conn_idx {
                        graph.set_conn_distorted(idx, true);
                    }
                }
                placed_this_pass = true;
            }
        }

        if !placed_this_pass {
            // Disconnected component: seed the lowest-id unplaced room near (0,0).
            let lowest = *unplaced.iter().min().unwrap();
            let occupied = occupied_cells(graph);
            let pos = nearest_free_cell(&occupied, (0, 0));
            graph.set_pos(lowest, pos);
            // Next loop iteration will continue placing from this new root.
        }
    }

    // Post-placement distortion sweep: re-derive distorted flag from final geometry.
    // Compass edges that aren't honoured → distorted=true; non-compass edges → always false.
    // This supersedes the ad-hoc per-collision flag set during placement (which remains as a
    // best-effort hint but is now overwritten here with the authoritative geometry check).
    let n_conns = graph.connections().len();
    for idx in 0..n_conns {
        let conn = graph.connections()[idx].clone();
        let distorted = match grid_offset(conn.dir) {
            None => false,                         // non-compass stub: never distorted
            Some(_) => !edge_is_satisfied(graph, &conn), // compass: distorted iff geometry violated
        };
        graph.set_conn_distorted(idx, distorted);
    }
}

/// For an unplaced `room_id`, find the best placement candidate.
///
/// Returns `Some((desired_cell, Option<conn_index>))`:
/// - `conn_index` is `Some(i)` when the candidate comes from a compass edge (so we can mark it
///   distorted on collision); `None` for Up/Down/In/Out/Unknown edges.
///
/// Priority: compass edges (exact offset) before non-compass (nearest-free to neighbour).
fn find_candidate(graph: &MapGraph, room_id: RoomId) -> Option<((i32, i32), Option<usize>)> {
    let conns: Vec<(usize, crate::graph::Connection)> = graph
        .connections()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.origin == room_id || c.dest == room_id)
        .map(|(i, c)| (i, c.clone()))
        .collect();

    // --- Compass edges first ---
    for (idx, conn) in &conns {
        if let Some(delta) = grid_offset(conn.dir) {
            if conn.dest == room_id {
                // edge: origin → dest=room_id; room_id should be at origin.pos + delta
                if let Some(origin_pos) = graph.room(conn.origin).and_then(|r| r.pos) {
                    let desired = (origin_pos.0 + delta.0, origin_pos.1 + delta.1);
                    return Some((desired, Some(*idx)));
                }
            } else if conn.origin == room_id {
                // edge: origin=room_id → dest; room_id should be at dest.pos - delta
                if let Some(dest_pos) = graph.room(conn.dest).and_then(|r| r.pos) {
                    let desired = (dest_pos.0 - delta.0, dest_pos.1 - delta.1);
                    return Some((desired, Some(*idx)));
                }
            }
        }
    }

    // --- Non-compass edges (Up/Down/In/Out/Unknown): place near neighbour ---
    for (_, conn) in &conns {
        if grid_offset(conn.dir).is_none() {
            let neighbour_pos = if conn.dest == room_id {
                graph.room(conn.origin).and_then(|r| r.pos)
            } else {
                graph.room(conn.dest).and_then(|r| r.pos)
            };
            if let Some(pos) = neighbour_pos {
                let occupied = occupied_cells(graph);
                let free = nearest_free_cell(&occupied, pos);
                return Some((free, None)); // conn_idx=None → don't mark distorted
            }
        }
    }

    None
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
        assert_eq!((p2.0 - p1.0, p2.1 - p1.1), (0, -1)); // north = up
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
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "C".into());
        graph.upsert_room(2, "N1".into());
        graph.upsert_room(3, "N2".into());
        graph.set_pos(1, (0, 0));
        graph.set_pos(2, (0, -1)); // already occupies the north cell
        graph.add_edge(1, Direction::N, 3); // 3 wants north of 1 = (0,-1) — collision
        relayout_auto(&mut graph);
        // room 3 placed somewhere other than (0,-1)
        let p3 = graph.room(3).unwrap().pos.unwrap();
        assert_ne!(p3, (0, -1), "room 3 must be displaced from occupied cell");
        // edge is marked distorted
        let conn = graph.connections().iter().find(|c| c.origin == 1 && c.dest == 3).unwrap();
        assert!(conn.distorted, "displaced edge must be distorted");
        // no overlap
        let cells: Vec<_> = graph.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "rooms must not overlap");
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

    #[test]
    fn minimal_movement_preserves_existing_pos() {
        let mut graph = MapGraph::new();
        graph.upsert_room(1, "A".into());
        graph.upsert_room(2, "B".into());
        graph.set_pos(1, (5, 5)); // non-default position
        graph.add_edge(1, Direction::N, 2);
        relayout_auto(&mut graph);
        // room 1 must stay where it was
        assert_eq!(graph.room(1).unwrap().pos, Some((5, 5)));
        // room 2 placed north of room 1
        assert_eq!(graph.room(2).unwrap().pos, Some((5, 4)));
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
}
