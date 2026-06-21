//! Re-tidy "sort" stage: per-axis longest-path layering from compass edges.
//!
//! Each planar edge imposes an ordering on one or both axes (east → larger x,
//! north → smaller y). We build a DAG per axis (dropping cycle-closing edges)
//! and assign integer coordinates by longest path, giving a compact grid that
//! honours every non-contradictory compass relation.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::grid_offset;
use crate::graph::{MapGraph, RoomId};

use super::{connected_components, nearest_free_cell};

/// Longest-path layering for one axis. `order_edges` = (lo,hi) meaning coord[lo] < coord[hi].
/// Cycle-closing edges (processed in slice order) are skipped.
pub(crate) fn layer_axis(n: usize, order_edges: &[(usize, usize)]) -> Vec<i32> {
    // Accept edges that don't close a cycle (hi cannot already reach lo).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(lo, hi) in order_edges {
        if lo == hi {
            continue;
        }
        if reaches(&adj, hi, lo) {
            continue; // would close a cycle → drop
        }
        adj[lo].push(hi);
    }
    // Longest path via Kahn's topological sort + relaxation.
    // in_degree[v] = number of predecessors in the DAG.
    let mut in_degree = vec![0usize; n];
    for successors in &adj {
        for &w in successors {
            in_degree[w] += 1;
        }
    }
    // Queue starts with all sources (in-degree 0).
    let mut queue: std::collections::VecDeque<usize> =
        (0..n).filter(|&v| in_degree[v] == 0).collect();
    let mut coord = vec![0_i32; n];
    while let Some(v) = queue.pop_front() {
        for &w in &adj[v] {
            if coord[v] + 1 > coord[w] {
                coord[w] = coord[v] + 1;
            }
            in_degree[w] -= 1;
            if in_degree[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    coord
}

fn reaches(adj: &[Vec<usize>], from: usize, target: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(v) = stack.pop() {
        if v == target {
            return true;
        }
        if !seen.insert(v) {
            continue;
        }
        stack.extend(adj[v].iter().copied());
    }
    false
}

/// Full per-component layering, packing, overlap resolution, and origin anchor.
pub(crate) fn sort_layout(graph: &MapGraph) -> BTreeMap<RoomId, (i32, i32)> {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    let mut pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    if ids.is_empty() {
        return pos;
    }
    let components = connected_components(graph, &ids);
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0;

    for comp in &components {
        let n = comp.len();
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let mut xe: Vec<(usize, usize)> = Vec::new();
        let mut ye: Vec<(usize, usize)> = Vec::new();
        for c in graph.connections() {
            let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) else {
                continue;
            };
            if let Some((dx, dy)) = grid_offset(c.dir) {
                if dx > 0 { xe.push((a, b)); } else if dx < 0 { xe.push((b, a)); }
                if dy > 0 { ye.push((a, b)); } else if dy < 0 { ye.push((b, a)); }
            }
        }
        let xs = layer_axis(n, &xe);
        let ys = layer_axis(n, &ye);

        // Normalise this component to its own origin, then pack to the right.
        let min_x = *xs.iter().min().unwrap();
        let min_y = *ys.iter().min().unwrap();
        let mut max_x_used = pack_x;
        for (i, &id) in comp.iter().enumerate() {
            let desired = (pack_x + xs[i] - min_x, ys[i] - min_y);
            let cell = nearest_free_cell(&occupied, desired);
            occupied.insert(cell);
            pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2;
    }

    // Anchor the lowest-id room at (0,0).
    if let Some(&(ax, ay)) = pos.get(&ids[0]) {
        for p in pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use std::collections::BTreeSet;

    #[test]
    fn layer_axis_chain_increments() {
        // 0<1<2 : longest paths 0,1,2
        let coords = layer_axis(3, &[(0, 1), (1, 2)]);
        assert_eq!(coords, vec![0, 1, 2]);
    }

    #[test]
    fn layer_axis_breaks_cycle_deterministically() {
        // 0<1, 1<2, 2<0 (cycle) — last edge dropped; no panic, finite coords.
        let coords = layer_axis(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(coords.len(), 3);
        assert!(coords[1] > coords[0] && coords[2] > coords[1]);
    }

    #[test]
    fn sort_layout_places_north_above_and_east_right() {
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2); // 2 north of 1
        g.add_edge(1, Direction::E, 3); // 3 east of 1
        let pos = sort_layout(&g);
        assert!(pos[&2].1 < pos[&1].1, "north room above");
        assert!(pos[&3].0 > pos[&1].0, "east room right");
        // no overlap
        let cells: Vec<_> = pos.values().collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn sort_layout_anchors_lowest_id_at_origin() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        let pos = sort_layout(&g);
        assert_eq!(pos[&1], (0, 0));
    }
}
