/// Zoom-independent render model for the map.
///
/// Produces a `RenderMap` from a `MapGraph`: placed rooms projected into `RenderRoom`s,
/// routed edges via `route_all`, and grid bounds for viewport sizing/scrolling.
///
/// # Unplaced rooms
///
/// Rooms without a `pos` (possible in Manual mode for a freshly observed room) are skipped
/// from `rooms` since they have no grid cell to render. They may still appear as edge endpoints
/// in `edges` only if their partner room is also placed; however, `route_all` already skips
/// connections where either endpoint is unplaced.
use crate::graph::{MapGraph, RoomId};
use crate::router::{route_all, RoutedEdge};

/// A single room's render data in grid coordinates.
#[derive(Debug, Clone)]
pub struct RenderRoom {
    pub id: RoomId,
    pub label: String,
    /// Logical grid cell `(col, row)`.
    pub cell: (i32, i32),
    /// True when the room has non-empty notes.
    pub has_notes: bool,
    /// True when this room is the current room.
    pub is_current: bool,
}

/// The complete zoom-independent render description of the map.
#[derive(Debug, Clone)]
pub struct RenderMap {
    pub rooms: Vec<RenderRoom>,
    pub edges: Vec<RoutedEdge>,
    /// `(min_cell, max_cell)` over placed room cells, for the TUI to size/scroll.
    /// Both components satisfy `min <= max`. Empty graph → `((0,0),(0,0))`.
    pub bounds: ((i32, i32), (i32, i32)),
}

/// Build a `RenderMap` from `graph`.
pub fn render(graph: &MapGraph) -> RenderMap {
    let current = graph.current();

    let rooms: Vec<RenderRoom> = graph
        .rooms()
        .filter_map(|room| {
            let cell = room.pos?; // skip unplaced rooms
            Some(RenderRoom {
                id: room.id,
                label: room.label().to_string(),
                cell,
                has_notes: !room.notes.is_empty(),
                is_current: Some(room.id) == current,
            })
        })
        .collect();

    let bounds = if rooms.is_empty() {
        ((0, 0), (0, 0))
    } else {
        let min_col = rooms.iter().map(|r| r.cell.0).min().unwrap();
        let max_col = rooms.iter().map(|r| r.cell.0).max().unwrap();
        let min_row = rooms.iter().map(|r| r.cell.1).min().unwrap();
        let max_row = rooms.iter().map(|r| r.cell.1).max().unwrap();
        ((min_col, min_row), (max_col, max_row))
    };

    let edges = route_all(graph);

    RenderMap { rooms, edges, bounds }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::Mapper;
    use crate::direction::Direction;

    #[test]
    fn render_marks_current_and_notes_and_bounds() {
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.set_notes(1, "start".into());
        let rm = render(&m.graph);
        assert_eq!(rm.rooms.len(), 2);
        let a = rm.rooms.iter().find(|r| r.id == 1).unwrap();
        assert!(a.has_notes);
        let b = rm.rooms.iter().find(|r| r.id == 2).unwrap();
        assert!(b.is_current); // current is the last-observed room (2)
        assert!(rm.bounds.0 .0 <= rm.bounds.1 .0); // min <= max
    }

    #[test]
    fn empty_graph_returns_zero_bounds_and_empty_rooms() {
        use crate::graph::MapGraph;
        let g = MapGraph::new();
        let rm = render(&g);
        assert!(rm.rooms.is_empty());
        assert!(rm.edges.is_empty());
        assert_eq!(rm.bounds, ((0, 0), (0, 0)));
    }

    #[test]
    fn unplaced_room_is_skipped() {
        use crate::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Placed".into());
        g.set_pos(1, (0, 0));
        g.upsert_room(2, "Unplaced".into()); // no pos
        let rm = render(&g);
        assert_eq!(rm.rooms.len(), 1);
        assert_eq!(rm.rooms[0].id, 1);
    }

    #[test]
    fn single_room_bounds_are_equal_min_max() {
        use crate::graph::MapGraph;
        let mut g = MapGraph::new();
        g.upsert_room(1, "Solo".into());
        g.set_pos(1, (3, -2));
        let rm = render(&g);
        assert_eq!(rm.bounds, ((3, -2), (3, -2)));
    }
}
