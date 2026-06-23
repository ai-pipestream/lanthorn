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
use crate::graph::{MapGraph, RoomId};
use crate::router::RoutedEdge;

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

/// Portal-badge edges for connections leaving `layer` to another layer. Empty in Phase 1.
pub fn interlayer_badges(_graph: &MapGraph, _layer: LayerId) -> Vec<RoutedEdge> {
    Vec::new()
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
}
