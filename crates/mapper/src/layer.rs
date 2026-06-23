//! Map layers ("segments"): a manual organizing tool. Every room belongs to exactly
//! one layer (default `MAIN_LAYER`). Layers are created/destroyed only by explicit
//! peel/merge — never derived. See docs/superpowers/specs/2026-06-23-manual-map-layers-design.md.

use std::collections::{BTreeSet, VecDeque};

/// Stable layer identifier. Layer `0` (`MAIN_LAYER`) always exists.
pub type LayerId = u16;

/// The permanent base layer every room starts in.
pub const MAIN_LAYER: LayerId = 0;

/// Per-layer metadata: a display name and the layer it was peeled from (for merge default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayerMeta {
    pub name: String,
    pub parent: Option<LayerId>,
}

impl LayerMeta {
    /// Metadata for the base "Main" layer.
    pub fn main() -> Self {
        LayerMeta { name: "Main".to_string(), parent: None }
    }
}

use crate::direction::grid_offset;
use crate::graph::{Connection, MapGraph, RoomId};
use crate::router::{fine_cell, stub_label, RoutedEdge};

/// Rooms reachable from `start` through PLANAR edges only (cardinals + diagonals),
/// staying within `start`'s current layer. Portal edges (Up/Down/In/Out/Unknown) are
/// not traversed — they are the cut. Edges are treated as undirected for reachability.
pub fn planar_region(graph: &MapGraph, start: RoomId) -> BTreeSet<RoomId> {
    let layer = graph.layer_of(start);
    let mut seen = BTreeSet::new();
    seen.insert(start);
    let mut q = VecDeque::new();
    q.push_back(start);
    while let Some(cur) = q.pop_front() {
        for c in graph.connections() {
            if grid_offset(c.dir).is_none() {
                continue; // portal edge → cut
            }
            let other = if c.origin == cur {
                c.dest
            } else if c.dest == cur {
                c.origin
            } else {
                continue;
            };
            if graph.layer_of(other) == layer && seen.insert(other) {
                q.push_back(other);
            }
        }
    }
    seen
}

/// Peel `start`'s planar region into a fresh layer. Returns the new `LayerId`, or `None`
/// when the region already spans the whole source layer (nothing to separate).
pub fn peel_region(graph: &mut MapGraph, start: RoomId) -> Option<LayerId> {
    let src = graph.layer_of(start);
    let region = planar_region(graph, start);
    let whole_layer: BTreeSet<RoomId> = graph.rooms_in_layer(src).into_iter().collect();
    if region == whole_layer {
        return None;
    }
    let name = graph.room(start).map(|r| r.label().to_string()).unwrap_or_default();
    let new = graph.new_layer(Some(src), name);
    for id in region {
        graph.set_room_layer(id, new);
    }
    Some(new)
}

/// True iff the connection's endpoints are in different layers.
pub fn is_interlayer(graph: &MapGraph, conn: &Connection) -> bool {
    graph.layer_of(conn.origin) != graph.layer_of(conn.dest)
}

/// Fold every room in `layer` into its parent (or `MAIN_LAYER` if it has none) and drop
/// the layer's metadata. Returns the target layer. No-op (returns `MAIN_LAYER`) for `MAIN_LAYER`.
pub fn merge_layer(graph: &mut MapGraph, layer: LayerId) -> LayerId {
    if layer == MAIN_LAYER {
        return MAIN_LAYER;
    }
    let target = graph.layers().get(&layer).and_then(|m| m.parent).unwrap_or(MAIN_LAYER);
    for id in graph.rooms_in_layer(layer) {
        graph.set_room_layer(id, target);
    }
    graph.remove_layer(layer);
    target
}

/// One portal badge per connection that crosses out of `layer`. Anchored at the
/// in-layer endpoint; carries the destination room name and destination layer name.
pub fn interlayer_badges(graph: &MapGraph, layer: LayerId) -> Vec<RoutedEdge> {
    let mut out = Vec::new();
    for c in graph.connections() {
        if !is_interlayer(graph, c) {
            continue;
        }
        // Determine the in-layer endpoint and the remote endpoint.
        let (here, there, dir) = if graph.layer_of(c.origin) == layer {
            (c.origin, c.dest, c.dir)
        } else if graph.layer_of(c.dest) == layer {
            (c.dest, c.origin, c.dir)
        } else {
            continue;
        };
        let Some(here_pos) = graph.room(here).and_then(|r| r.pos) else { continue };
        let fine = fine_cell(here_pos);
        let dest_layer = graph.layer_of(there);
        let dest_lbl = graph
            .room(there)
            .map(|r| format!("{} · {}", r.label(), graph.layer_name(dest_layer)));
        out.push(RoutedEdge {
            origin: here,
            dest: there,
            dir,
            points: vec![fine, (fine.0, fine.1 - 1)],
            distorted: false,
            is_stub: true,
            label: Some(stub_label(dir).to_string()),
            arrival_dir: None,
            dest_label: dest_lbl,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn two_floors() -> MapGraph {
        // Floor 1: 1-E-2 (planar). 1-Down-3 (portal) to floor 2: 3-E-4 (planar).
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Study"), (3, "Cellar"), (4, "Wine")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        g
    }

    #[test]
    fn planar_region_stops_at_portals() {
        let g = two_floors();
        let region = planar_region(&g, 3);
        let want: BTreeSet<RoomId> = [3, 4].into_iter().collect();
        assert_eq!(region, want, "down-portal cuts the cellar off from the upper floor");
    }

    #[test]
    fn peel_region_moves_floor_to_new_layer() {
        let mut g = two_floors();
        let l = peel_region(&mut g, 3).expect("a proper sub-region peels");
        assert_eq!(g.layer_of(3), l);
        assert_eq!(g.layer_of(4), l);
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_of(2), MAIN_LAYER);
        assert_eq!(g.layer_name(l), "Cellar");
    }

    #[test]
    fn peel_whole_layer_is_noop() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        assert_eq!(peel_region(&mut g, 1), None, "region is the whole layer → no-op");
    }

    #[test]
    fn interlayer_badges_are_per_edge() {
        // Two staircases between the same two layers must yield two distinct badges.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "HallN"), (2, "HallS"), (3, "CellarN"), (4, "CellarS")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::E, 2); // keep them in one planar region up top
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3); // staircase A
        g.add_edge(2, Direction::Down, 4); // staircase B
        let l = peel_region(&mut g, 3).expect("peel cellar");
        g.set_room_layer(4, l); // ensure both cellar rooms in the new layer
        g.set_pos(3, (0, 0));
        g.set_pos(4, (0, 1));
        let up = interlayer_badges(&g, MAIN_LAYER);
        assert_eq!(up.len(), 2, "two independent staircases → two badges");
        assert!(up.iter().all(|e| e.is_stub));
        // peel_region(3) named the layer after room 3's label ("CellarN"); badge text is
        // "<dest room> · <dest layer>". Assert the shape, not a brittle literal.
        assert!(up.iter().all(|e| e.dest_label.as_deref().is_some_and(|s| s.contains(" · "))));
        assert!(up.iter().any(|e| e.dest_label.as_deref() == Some("CellarN · CellarN")));
    }

    #[test]
    fn merge_layer_folds_into_parent_and_removes_meta() {
        let mut g = two_floors();
        let l = peel_region(&mut g, 3).unwrap(); // parent = MAIN_LAYER
        let target = merge_layer(&mut g, l);
        assert_eq!(target, MAIN_LAYER);
        assert_eq!(g.layer_of(3), MAIN_LAYER);
        assert_eq!(g.layer_of(4), MAIN_LAYER);
        assert!(!g.layers().contains_key(&l), "merged layer's metadata is removed");
    }

    #[test]
    fn merge_main_is_noop() {
        let mut g = two_floors();
        assert_eq!(merge_layer(&mut g, MAIN_LAYER), MAIN_LAYER);
        assert!(g.layers().contains_key(&MAIN_LAYER));
    }
}
