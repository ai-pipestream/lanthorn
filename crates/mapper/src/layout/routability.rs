//! Grid-level routability: does a drawn edge have a clean orthogonal channel?
//!
//! Models each room as a single-cell obstacle. An edge is routable iff a BFS from
//! the origin cell — forced to take its first step in the edge's compass direction —
//! reaches the destination cell without entering any other room's cell. A clear grid
//! cell corresponds to a full empty 29×17 render stride (≫ the 21×11 box), so a
//! grid-level channel implies a render-level channel for room obstacles. Path-vs-path
//! congestion is handled by the lane router, which assigns each connector to a reserved
//! lane in the inter-room channels so connectors never overlap (no unrouted fallback).

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

/// Candidate moves reach any free cell within this Chebyshev radius, so the climb can
/// cross a routability valley (a one-cell step that worsens the primary term) to reach
/// a strictly-better cell beyond it.
const MOVE_RADIUS: i32 = 3;

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

/// Compass octant of a direction vector: (sign(dx), sign(dy)), each in {-1,0,1}.
/// Two neighbours of a room sharing an octant fan the same way → crossing pressure.
fn octant(dx: i32, dy: i32) -> (i8, i8) {
    (dx.signum() as i8, dy.signum() as i8)
}

/// Distinct neighbour rooms of `r` via DRAWN (compass) edges, either direction.
/// Reciprocal pairs collapse naturally — a neighbour is listed once.
fn neighbours(graph: &MapGraph, r: RoomId) -> BTreeSet<RoomId> {
    let mut ns = BTreeSet::new();
    for c in graph.connections() {
        if grid_offset(c.dir).is_none() {
            continue;
        }
        if c.origin == r {
            ns.insert(c.dest);
        } else if c.dest == r {
            ns.insert(c.origin);
        }
    }
    ns
}

/// Total per-room same-octant neighbour conflicts: for each placed room, each
/// unordered pair of its placed neighbours that share an octant relative to it.
fn side_conflicts(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> usize {
    let mut total = 0;
    for (&r, &rp) in pos {
        let ns: Vec<RoomId> = neighbours(graph, r).into_iter().filter(|n| pos.contains_key(n)).collect();
        for i in 0..ns.len() {
            for j in (i + 1)..ns.len() {
                let a = pos[&ns[i]];
                let b = pos[&ns[j]];
                if octant(a.0 - rp.0, a.1 - rp.1) == octant(b.0 - rp.0, b.1 - rp.1) {
                    total += 1;
                }
            }
        }
    }
    total
}

/// Rooms involved in any same-octant conflict: the room itself plus the two
/// neighbours of each conflicting pair (the set the repair is allowed to move).
fn conflict_rooms(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> BTreeSet<RoomId> {
    let mut rooms = BTreeSet::new();
    for (&r, &rp) in pos {
        let ns: Vec<RoomId> = neighbours(graph, r).into_iter().filter(|n| pos.contains_key(n)).collect();
        for i in 0..ns.len() {
            for j in (i + 1)..ns.len() {
                let a = pos[&ns[i]];
                let b = pos[&ns[j]];
                if octant(a.0 - rp.0, a.1 - rp.1) == octant(b.0 - rp.0, b.1 - rp.1) {
                    rooms.insert(r);
                    rooms.insert(ns[i]);
                    rooms.insert(ns[j]);
                }
            }
        }
    }
    rooms
}

/// Greedily shift rooms (into free cells, within a small radius) until neither the
/// number of un-routable drawn edges nor the per-room same-octant conflict count can
/// be reduced. Score is lexicographic `(unroutable, side_conflicts, displacement)`;
/// only strictly-improving moves are accepted, so the search is deterministic and
/// terminates. Multi-cell moves let the climb cross a routability valley (e.g. #25
/// (0,0)→(0,2) past the blocked (0,1)).
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

    let score = |p: &BTreeMap<RoomId, (i32, i32)>| -> (usize, usize, i64) {
        (unroutable_count(p, &drawn), side_conflicts(graph, p), displacement(p, &stress))
    };

    for _ in 0..MAX_REPAIR_PASSES {
        let base = score(pos);

        // Candidate rooms: endpoints of un-routable edges ∪ rooms in a same-octant conflict.
        let occ_now = occupied_map(pos);
        let bb = bbox_of(pos);
        let mut cands: BTreeSet<RoomId> = BTreeSet::new();
        for &(o, d, dir) in &drawn {
            if !edge_routable(pos[&o], pos[&d], dir, &occ_now, bb) {
                cands.insert(o);
                cands.insert(d);
            }
        }
        cands.extend(conflict_rooms(graph, pos));
        if cands.is_empty() {
            break; // 0 unroutable AND 0 conflicts
        }

        // (room, target cell, score)
        type BestMove = Option<(RoomId, (i32, i32), (usize, usize, i64))>;
        let mut best: BestMove = None;
        for &room in &cands {
            let from = pos[&room];
            for dx in -MOVE_RADIUS..=MOVE_RADIUS {
                for dy in -MOVE_RADIUS..=MOVE_RADIUS {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let to = (from.0 + dx, from.1 + dy);
                    if pos.values().any(|&p| p == to) {
                        continue; // occupied → would overlap
                    }
                    let mut trial = pos.clone();
                    trial.insert(room, to);
                    let s = score(&trial);
                    if s < base && best.as_ref().is_none_or(|&(_, _, bs)| s < bs) {
                        best = Some((room, to, s));
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

    #[test]
    fn octant_is_sign_pair() {
        assert_eq!(octant(-1, 2), (-1, 1));
        assert_eq!(octant(-1, 0), (-1, 0));
        assert_eq!(octant(3, -4), (1, -1));
        assert_eq!(octant(0, 0), (0, 0));
    }

    #[test]
    fn side_conflicts_counts_same_octant_neighbour_pairs() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // #25 connects to #74 and #76 (drawn compass edges). At (0,0) both neighbours
        // are SW (same octant) → 1 conflict. Move #25 to (0,2): #74 is NW, #76 is W
        // (different octants) → 0 conflicts.
        let mut g = MapGraph::new();
        for id in [25u16, 74, 76] { g.upsert_room(id, "r".into()); }
        g.add_edge(74, Direction::E, 25);
        g.add_edge(74, Direction::S, 76);
        g.add_edge(25, Direction::W, 76);

        let crammed: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 0)), (74, (-1, 1)), (76, (-1, 2))].into_iter().collect();
        assert_eq!(side_conflicts(&g, &crammed), 1, "both #25 neighbours are SW");
        assert!(conflict_rooms(&g, &crammed).contains(&25), "the conflicted room is flagged");

        let spread: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 2)), (74, (-1, 1)), (76, (-1, 2))].into_iter().collect();
        assert_eq!(side_conflicts(&g, &spread), 0, "#74 is NW, #76 is W → no shared octant");
        assert!(conflict_rooms(&g, &spread).is_empty());
    }

    #[test]
    fn repair_removes_same_octant_crossing_pressure() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // The A129 corner after Milestone-5 routability repair: 0 unroutable, but #25's
        // two neighbours (#74, #76) both sit SW → 1 conflict. The crossing-aware repair
        // must drop conflicts to 0 by moving a room (the cheapest here is #76), without
        // re-introducing an unroutable edge or a room overlap. Which room moves is not
        // asserted — only that the conflict is gone and routability/overlap are preserved.
        let mut g = MapGraph::new();
        for id in [25u16, 74, 76] { g.upsert_room(id, "r".into()); }
        g.add_edge(74, Direction::E, 25);
        g.add_edge(74, Direction::S, 76);
        g.add_edge(25, Direction::W, 76);

        let mut pos: BTreeMap<RoomId, (i32, i32)> =
            [(25u16, (0, 0)), (74, (-1, 1)), (76, (-1, 2))].into_iter().collect();
        assert_eq!(side_conflicts(&g, &pos), 1, "precondition: the corner has a same-octant conflict");

        repair_routability(&g, &mut pos);

        assert_eq!(side_conflicts(&g, &pos), 0, "repair must remove the conflict; got {pos:?}");
        // Still routable, still no overlap.
        let occ: BTreeMap<(i32, i32), RoomId> = pos.iter().map(|(&id, &c)| (c, id)).collect();
        let xs: Vec<i32> = pos.values().map(|p| p.0).collect();
        let ys: Vec<i32> = pos.values().map(|p| p.1).collect();
        let bb = (
            xs.iter().min().unwrap() - BBOX_MARGIN, ys.iter().min().unwrap() - BBOX_MARGIN,
            xs.iter().max().unwrap() + BBOX_MARGIN, ys.iter().max().unwrap() + BBOX_MARGIN,
        );
        for c in g.connections() {
            assert!(edge_routable(pos[&c.origin], pos[&c.dest], c.dir, &occ, bb),
                "edge {}->{} must stay routable; {pos:?}", c.origin, c.dest);
        }
        let cells: BTreeSet<_> = pos.values().collect();
        assert_eq!(cells.len(), pos.len(), "no overlap");
        // The repair drives side_conflicts to 0 by moving whichever room is cheapest
        // (here #76, displacement 1), not necessarily #25 — the objective is conflict-
        // freeness, not a specific room. The full-graph render is the crossing gate (in app).
    }

    #[test]
    fn reciprocal_pair_is_one_neighbour_no_self_conflict() {
        use crate::direction::Direction;
        use crate::graph::MapGraph;
        // A reciprocal pair a<->b must contribute ONE neighbour each way, never a
        // self-conflict (a room with a single neighbour has no pair).
        let mut g = MapGraph::new();
        for id in [1u16, 2] { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1); // reciprocal
        let pos: BTreeMap<RoomId, (i32, i32)> = [(1u16, (0, 1)), (2, (0, 0))].into_iter().collect();
        assert_eq!(neighbours(&g, 1), [2u16].into_iter().collect());
        assert_eq!(side_conflicts(&g, &pos), 0, "one neighbour each → no pair → no conflict");
    }
}
