//! Grid-level routability: does a drawn edge have a clean orthogonal channel?
//!
//! Models each room as a single-cell obstacle. An edge is routable iff a BFS from
//! the origin cell — forced to take its first step in the edge's compass direction —
//! reaches the destination cell without entering any other room's cell. A clear grid
//! cell corresponds to a full empty 29×17 render stride (≫ the 21×11 box), so a
//! grid-level channel implies a render-level channel for room obstacles. Path-vs-path
//! congestion is out of scope (handled by the renderer's unrouted-line fallback).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::{grid_offset, Direction};
use crate::graph::RoomId;

/// BFS search bound: the room bounding box is expanded by this many cells so a
/// route may detour just outside the outermost rooms.
pub const BBOX_MARGIN: i32 = 2;

/// The unit first-step deltas allowed when leaving the origin: a cardinal dir gives
/// exactly its own step; a diagonal gives each of its two axis components; a
/// non-compass dir gives none (caller treats those as "routable" — they aren't drawn).
pub fn first_steps(dir: Direction) -> Vec<(i32, i32)> {
    match grid_offset(dir) {
        None => Vec::new(),
        Some((dx, dy)) => {
            let mut v = Vec::new();
            if dx != 0 {
                v.push((dx.signum(), 0));
            }
            if dy != 0 {
                v.push((0, dy.signum()));
            }
            v
        }
    }
}

/// True iff a clean orthogonal channel exists from `origin` to `dest` whose first
/// step is in `dir`, treating every occupied cell except `origin`/`dest` as an
/// obstacle. BFS is bounded to `bbox = (min_x, min_y, max_x, max_y)` inclusive.
pub fn edge_routable(
    origin: (i32, i32),
    dest: (i32, i32),
    dir: Direction,
    occupied: &BTreeMap<(i32, i32), RoomId>,
    bbox: (i32, i32, i32, i32),
) -> bool {
    let steps = first_steps(dir);
    if steps.is_empty() || origin == dest {
        return true;
    }
    let (min_x, min_y, max_x, max_y) = bbox;
    let in_box =
        |c: (i32, i32)| c.0 >= min_x && c.0 <= max_x && c.1 >= min_y && c.1 <= max_y;
    let blocked = |c: (i32, i32)| c != origin && c != dest && occupied.contains_key(&c);

    let mut seen: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut q: VecDeque<(i32, i32)> = VecDeque::new();
    for (dx, dy) in steps {
        let c = (origin.0 + dx, origin.1 + dy);
        if in_box(c) && !blocked(c) && seen.insert(c) {
            q.push_back(c);
        }
    }
    while let Some(cur) = q.pop_front() {
        if cur == dest {
            return true;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nxt = (cur.0 + dx, cur.1 + dy);
            if in_box(nxt) && !blocked(nxt) && seen.insert(nxt) {
                q.push_back(nxt);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(cells: &[((i32, i32), RoomId)]) -> BTreeMap<(i32, i32), RoomId> {
        cells.iter().copied().collect()
    }

    #[test]
    fn first_steps_cardinal_and_diagonal() {
        assert_eq!(first_steps(Direction::W), vec![(-1, 0)]);
        assert_eq!(first_steps(Direction::N), vec![(0, -1)]);
        assert_eq!(first_steps(Direction::NE), vec![(1, 0), (0, -1)]);
        assert!(first_steps(Direction::Up).is_empty());
    }

    #[test]
    fn adjacent_edge_is_routable() {
        // origin (0,0) -W-> dest (-1,0): the west neighbour IS the destination.
        let occupied = occ(&[((0, 0), 1), ((-1, 0), 2)]);
        assert!(edge_routable((0, 0), (-1, 0), Direction::W, &occupied, (-3, -3, 3, 3)));
    }

    #[test]
    fn blocked_departure_cell_is_unroutable() {
        // origin (0,0) -W-> dest (-1,1) with room #74 at (-1,0) blocking due west.
        // The only first step (west) lands on a room, so the edge cannot leave cleanly.
        let occupied = occ(&[((0, 0), 25), ((-1, 0), 74), ((-1, 1), 76)]);
        assert!(!edge_routable((0, 0), (-1, 1), Direction::W, &occupied, (-3, -3, 3, 3)));
    }

    #[test]
    fn clear_lane_around_is_routable_after_shift() {
        // origin (0,1) -W-> dest (-1,1): west neighbour is the destination, clear.
        let occupied = occ(&[((0, 1), 25), ((-1, 0), 74), ((-1, 1), 76)]);
        assert!(edge_routable((0, 1), (-1, 1), Direction::W, &occupied, (-3, -3, 3, 3)));
    }
}
