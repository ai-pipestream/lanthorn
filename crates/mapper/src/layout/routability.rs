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
use crate::graph::{MapGraph, RoomId};

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

/// Max hill-climb passes. A backstop: each accepted move strictly lowers the score,
/// so the loop terminates well before this on real maps.
const MAX_REPAIR_PASSES: usize = 30;

fn occupied_map(pos: &BTreeMap<RoomId, (i32, i32)>) -> BTreeMap<(i32, i32), RoomId> {
    pos.iter().map(|(&id, &c)| (c, id)).collect()
}

fn bbox_of(pos: &BTreeMap<RoomId, (i32, i32)>) -> (i32, i32, i32, i32) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for &(x, y) in pos.values() {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    (min_x - BBOX_MARGIN, min_y - BBOX_MARGIN, max_x + BBOX_MARGIN, max_y + BBOX_MARGIN)
}

/// Number of drawn edges with no clean channel under `pos`.
fn unroutable_count(
    pos: &BTreeMap<RoomId, (i32, i32)>,
    drawn: &[(RoomId, RoomId, Direction)],
) -> usize {
    let occ = occupied_map(pos);
    let bb = bbox_of(pos);
    drawn
        .iter()
        .filter(|&&(o, d, dir)| !edge_routable(pos[&o], pos[&d], dir, &occ, bb))
        .count()
}

/// Total L1 displacement of `pos` from the pre-repair `stress` positions (the
/// deterministic tiebreaker — keeps the search from drifting needlessly).
fn displacement(
    pos: &BTreeMap<RoomId, (i32, i32)>,
    stress: &BTreeMap<RoomId, (i32, i32)>,
) -> i64 {
    pos.iter()
        .map(|(id, &(x, y))| {
            let (sx, sy) = stress[id];
            ((x - sx).abs() + (y - sy).abs()) as i64
        })
        .sum()
}

/// Greedily shift rooms into free grid cells until the number of un-routable drawn
/// edges can no longer be reduced. Score is lexicographic `(unroutable, displacement)`;
/// only strictly-improving moves are accepted, so the search is deterministic and
/// terminates. Drawn edges are compass edges (the only ones rendered as paths).
pub fn repair_routability(graph: &MapGraph, pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    let drawn: Vec<(RoomId, RoomId, Direction)> = graph
        .connections()
        .iter()
        .filter(|c| grid_offset(c.dir).is_some())
        .map(|c| (c.origin, c.dest, c.dir))
        .collect();
    if drawn.is_empty() {
        return;
    }
    let stress = pos.clone();

    for _ in 0..MAX_REPAIR_PASSES {
        let base = (unroutable_count(pos, &drawn), displacement(pos, &stress));
        if base.0 == 0 {
            break;
        }
        let occ_now = occupied_map(pos);
        let bb = bbox_of(pos);
        // (room to move, target cell, score)
        type BestMove = Option<(RoomId, (i32, i32), (usize, i64))>;
        let mut best: BestMove = None;

        for &(o, d, dir) in &drawn {
            // Only try to fix edges that are currently un-routable.
            if edge_routable(pos[&o], pos[&d], dir, &occ_now, bb) {
                continue;
            }
            for cand in [o, d] {
                let from = pos[&cand];
                for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                    let to = (from.0 + dx, from.1 + dy);
                    if pos.values().any(|&p| p == to) {
                        continue; // occupied → would overlap
                    }
                    let mut trial = pos.clone();
                    trial.insert(cand, to);
                    let s = (unroutable_count(&trial, &drawn), displacement(&trial, &stress));
                    if s < base && best.as_ref().is_none_or(|&(_, _, bs)| s < bs) {
                        best = Some((cand, to, s));
                    }
                }
            }
        }

        match best {
            Some((room, to, _)) => {
                pos.insert(room, to);
            }
            None => break, // no strictly-improving move
        }
    }
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

    #[test]
    fn repair_shifts_room_to_open_blocked_channel() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // The cramped #25/#74/#76 corner as the stress solver can produce it inside a
        // larger graph: #74 sits due west of #25, blocking 25->W->76's only departure.
        let mut g = MapGraph::new();
        for (id, name) in [(25u16, "Canyon View"), (74, "Clearing"), (76, "Forest")] {
            g.upsert_room(id, name.into());
        }
        g.add_edge(74, Direction::E, 25);
        g.add_edge(74, Direction::S, 76);
        g.add_edge(25, Direction::W, 76);

        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 0)), (74, (-1, 0)), (76, (-1, 1))].into_iter().collect();

        let all_routable = |pos: &BTreeMap<RoomId, (i32, i32)>| {
            let occ: BTreeMap<(i32, i32), RoomId> =
                pos.iter().map(|(&id, &c)| (c, id)).collect();
            let xs: Vec<i32> = pos.values().map(|p| p.0).collect();
            let ys: Vec<i32> = pos.values().map(|p| p.1).collect();
            let bb = (
                xs.iter().min().unwrap() - BBOX_MARGIN,
                ys.iter().min().unwrap() - BBOX_MARGIN,
                xs.iter().max().unwrap() + BBOX_MARGIN,
                ys.iter().max().unwrap() + BBOX_MARGIN,
            );
            g.connections()
                .iter()
                .all(|c| edge_routable(pos[&c.origin], pos[&c.dest], c.dir, &occ, bb))
        };

        // Before repair: 25->W->76 is blocked by #74 due west.
        assert!(!all_routable(&pos), "precondition: the cramped corner has an unroutable edge");

        repair_routability(&g, &mut pos);

        // After repair: every edge has a clean channel, and no two rooms overlap.
        assert!(all_routable(&pos), "repair must open a channel for every edge; got {pos:?}");
        let cells: std::collections::BTreeSet<_> = pos.values().collect();
        assert_eq!(cells.len(), pos.len(), "repair must not overlap rooms");
    }
}
