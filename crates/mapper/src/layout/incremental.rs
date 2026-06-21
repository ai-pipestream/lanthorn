//! Incremental local placement — the per-turn layout regime.
//!
//! Places one newly discovered room relative to the previous room, in the
//! compass direction of the move, shifting only the rooms "beyond" the
//! insertion point on collision (Trizbort's strategy). Existing rooms
//! otherwise never move, so the map is stable turn-to-turn.

use crate::direction::{grid_offset, Direction};
use crate::graph::{MapGraph, RoomId};

use super::{nearest_free_cell, occupied_cells};

/// Place `dest` relative to `prev` via `dir`. See module/interface docs.
pub fn place_incremental(graph: &mut MapGraph, prev: RoomId, dest: RoomId, dir: Direction) {
    // Revisit / loop-closure: never move an already-placed room.
    if graph.room(dest).and_then(|r| r.pos).is_some() {
        return;
    }
    let prev_pos = match graph.room(prev).and_then(|r| r.pos) {
        Some(p) => p,
        None => return, // caller guarantees prev is placed; defensive no-op
    };

    match grid_offset(dir) {
        Some(delta) => {
            let ideal = (prev_pos.0 + delta.0, prev_pos.1 + delta.1);
            let occupied = occupied_cells(graph);
            if !occupied.contains(&ideal) {
                graph.set_pos(dest, ideal);
                return;
            }
            // Occupied. Cardinal → shift-beyond opens the ideal cell.
            let is_cardinal = (delta.0 == 0) ^ (delta.1 == 0);
            if is_cardinal {
                shift_beyond(graph, ideal, delta);
                graph.set_pos(dest, ideal);
            } else {
                // Diagonal fallback: nearest free cell from the ideal.
                let occ = occupied_cells(graph);
                let cell = nearest_free_cell(&occ, ideal);
                graph.set_pos(dest, cell);
            }
        }
        None => {
            // Portal / unknown: nearest free cell starting from prev.
            let occ = occupied_cells(graph);
            let cell = nearest_free_cell(&occ, prev_pos);
            graph.set_pos(dest, cell);
        }
    }
}

/// Translate every placed room at or beyond `ideal` along the `step` axis by
/// one `step`, opening `ideal`. `step` must be a cardinal unit vector.
fn shift_beyond(graph: &mut MapGraph, ideal: (i32, i32), step: (i32, i32)) {
    let ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    for id in ids {
        if let Some(pos) = graph.room(id).and_then(|r| r.pos) {
            let beyond = match step {
                (1, 0) => pos.0 >= ideal.0,
                (-1, 0) => pos.0 <= ideal.0,
                (0, 1) => pos.1 >= ideal.1,
                (0, -1) => pos.1 <= ideal.1,
                _ => false,
            };
            if beyond {
                graph.set_pos(id, (pos.0 + step.0, pos.1 + step.1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn g_with(prev: RoomId, prev_pos: (i32, i32)) -> MapGraph {
        let mut g = MapGraph::new();
        g.upsert_room(prev, "prev".into());
        g.set_pos(prev, prev_pos);
        g
    }

    #[test]
    fn places_planar_room_at_compass_offset() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "n".into());
        place_incremental(&mut g, 1, 2, Direction::N);
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)));
    }

    #[test]
    fn places_diagonal_room_at_diagonal_cell() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "ne".into());
        place_incremental(&mut g, 1, 2, Direction::NE);
        assert_eq!(g.room(2).unwrap().pos, Some((1, -1)));
    }

    #[test]
    fn already_placed_dest_is_noop() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "x".into());
        g.set_pos(2, (5, 5));
        place_incremental(&mut g, 1, 2, Direction::N);
        assert_eq!(g.room(2).unwrap().pos, Some((5, 5)), "revisit must not move a placed room");
    }

    #[test]
    fn shift_beyond_opens_occupied_cardinal_cell() {
        // prev at (0,0); a blocker already sits north at (0,-1).
        let mut g = g_with(1, (0, 0));
        g.upsert_room(9, "blocker".into());
        g.set_pos(9, (0, -1));
        g.upsert_room(2, "n".into());
        place_incremental(&mut g, 1, 2, Direction::N);
        // New room lands truthfully at (0,-1); the blocker is shifted further north.
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)));
        assert_eq!(g.room(9).unwrap().pos, Some((0, -2)), "blocker shifted beyond");
        // prev did not move (it is south of the ideal line).
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)));
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn portal_dir_places_adjacent_without_overlap() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "down".into());
        place_incremental(&mut g, 1, 2, Direction::Down);
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_ne!(p2, (0, 0), "must not land on prev");
    }

    #[test]
    fn shift_beyond_multi_blocker_column_no_overlap() {
        // prev at (0,0); two blockers stacked north at (0,-1) and (0,-2).
        let mut g = g_with(1, (0, 0));
        g.upsert_room(8, "b1".into());
        g.set_pos(8, (0, -1));
        g.upsert_room(9, "b2".into());
        g.set_pos(9, (0, -2));
        g.upsert_room(2, "n".into());
        place_incremental(&mut g, 1, 2, Direction::N);
        // New room lands at (0,-1); both blockers shifted further north; no overlap.
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)));
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap with stacked blockers");
    }
}
