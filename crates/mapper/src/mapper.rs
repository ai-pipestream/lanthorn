use std::collections::BTreeSet;
use crate::direction::{Direction, parse_direction};
use crate::graph::{MapGraph, RoomId};
use crate::layout::{nearest_free_cell, occupied_cells, place_incremental, LayoutMode};
use crate::layout::mark_distorted;

#[derive(Debug, Default)]
pub struct Mapper {
    pub graph: MapGraph,
    pub mode: LayoutMode,
}

impl Mapper {
    pub fn observe(&mut self, location: RoomId, name: &str, via: Option<Direction>) {
        self.graph.upsert_room(location, name.to_string());
        let prev = self.graph.current();
        match prev {
            None => {
                // First room ever: anchor at the origin.
                if self.graph.room(location).and_then(|r| r.pos).is_none() {
                    self.graph.set_pos(location, (0, 0));
                }
            }
            Some(prev_id) => {
                if location != prev_id {
                    let edge_dir = via.unwrap_or(Direction::Unknown);
                    self.graph.add_edge(prev_id, edge_dir, location);
                    // Drop a now-redundant `?` stub: fires whether the Unknown came first and a
                    // directional move just followed, or a directional edge already existed and
                    // this move was Unknown. Edge hygiene is independent of layout mode. (SQ-0220)
                    self.graph.collapse_unknown_edges();
                    if self.mode == LayoutMode::Auto {
                        place_incremental(&mut self.graph, prev_id, location, edge_dir);
                    }
                }
            }
        }
        self.graph.set_current(location);
        if self.mode == LayoutMode::Auto {
            // Re-evaluate distortion over the whole graph (cheap); no relayout.
            mark_distorted(&mut self.graph, &BTreeSet::new());
        }
    }

    pub fn observe_command(&mut self, location: RoomId, name: &str, command: &str) {
        self.observe(location, name, parse_direction(command));
    }

    /// Record an *involuntary* relocation — the current room changed, but NOT via a
    /// real passage the player walked (e.g. death + resurrection, or a teleport that
    /// drops the player somewhere unrelated to the command they typed). Move the
    /// current pointer to `location` without minting any edge, so a typed "north"
    /// that got the player killed never mints a false N-edge to the resurrection
    /// room. A previously-unseen resurrection room is added and placed at a free
    /// cell (so it is visible but disconnected); an already-known room keeps its
    /// position. (SQ-0259)
    pub fn observe_relocation(&mut self, location: RoomId, name: &str) {
        self.graph.upsert_room(location, name.to_string());
        let prev = self.graph.current();
        if self.graph.room(location).and_then(|r| r.pos).is_none() {
            match prev {
                // First room ever seen (defensive): anchor at the origin.
                None => self.graph.set_pos(location, (0, 0)),
                // New resurrection room: drop it at a free cell near the room we
                // died in, visible but with no edge asserting a connection.
                Some(prev_id) => {
                    let from = self.graph.room(prev_id).and_then(|r| r.pos).unwrap_or((0, 0));
                    let cell = nearest_free_cell(&occupied_cells(&self.graph), from);
                    self.graph.set_pos(location, cell);
                }
            }
        }
        self.graph.set_current(location);
        if self.mode == LayoutMode::Auto {
            mark_distorted(&mut self.graph, &BTreeSet::new());
        }
    }

    /// Switch layout mode.
    /// Setting the same mode is a no-op.
    pub fn set_mode(&mut self, mode: LayoutMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
    }

    /// Move room `id` to cell `to` (Manual mode only).
    /// Returns false (no-op) in Auto mode.
    /// Returns true if the room moved or was already at `to`.
    /// Only moves if `to` is free (not occupied by another room).
    pub fn nudge(&mut self, id: RoomId, to: (i32, i32)) -> bool {
        if self.mode != LayoutMode::Manual {
            return false;
        }
        // Build occupied set excluding the room's own current cell so nudging to
        // its own cell is treated as free (no-op that returns true), and to allow
        // a true move to a currently self-occupied cell.
        let own_pos = self.graph.room(id).and_then(|r| r.pos);
        let mut occupied = occupied_cells(&self.graph);
        if let Some(p) = own_pos {
            occupied.remove(&p);
        }
        if occupied.contains(&to) {
            return false;
        }
        self.graph.set_pos(id, to);
        true
    }

    /// Set or clear the label_override for a room.
    pub fn rename_room(&mut self, id: RoomId, label: Option<String>) {
        self.graph.set_label_override(id, label);
    }

    /// Set the notes for a room.
    pub fn set_notes(&mut self, id: RoomId, notes: String) {
        self.graph.set_notes(id, notes);
    }

    /// Remove the connection with key (origin, dir). Returns true if removed.
    pub fn delete_connection(&mut self, origin: RoomId, dir: Direction) -> bool {
        self.graph.remove_connection(origin, dir)
    }

    /// Change the direction of the edge (origin, old) to (origin, new).
    /// Returns true if changed. Refuses (returns false) if (origin, new) already exists.
    pub fn relabel_edge(&mut self, origin: RoomId, old: Direction, new: Direction) -> bool {
        self.graph.relabel_connection(origin, old, new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    #[test]
    fn first_observation_sets_current_no_edge() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        assert_eq!(m.graph.current(), Some(1));
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn compass_move_creates_directed_edge() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        assert_eq!(m.graph.connections(), &[crate::graph::Connection{origin:1,dir:Direction::N,dest:2,distorted:false}]);
        assert_eq!(m.graph.current(), Some(2));
    }

    #[test]
    fn noncompass_move_creates_unknown_edge_room_not_lost() {
        let mut m = Mapper::default();
        m.observe(1, "Cave Mouth", None);
        m.observe_command(2, "Secret Grotto", "xyzzy"); // teleport
        assert!(m.graph.room(2).is_some());
        assert_eq!(m.graph.connections()[0].dir, Direction::Unknown);
    }

    #[test]
    fn observe_collapses_unknown_when_directional_edge_appears() {
        // Unknown arrives first (a non-directional move), then the same passage is later walked
        // with a compass command — the redundant `?` 1→2 collapses. (SQ-0220)
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe_command(2, "B", "xyzzy"); // (1, Unknown, 2)
        assert!(
            m.graph.connections().iter().any(|c| c.dir == Direction::Unknown && c.origin == 1 && c.dest == 2),
            "the Unknown 1→2 exists before a directional edge appears"
        );
        m.observe_command(1, "A", "south"); // walk back: (2, S, 1) — reverse, does not collapse
        m.observe_command(2, "B", "north"); // forward directional: (1, N, 2) → Unknown collapses
        assert!(
            !m.graph.connections().iter().any(|c| c.dir == Direction::Unknown),
            "the redundant Unknown 1→2 collapsed once the N edge appeared: {:?}", m.graph.connections()
        );
        assert!(m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::N && c.dest == 2));
    }

    #[test]
    fn observe_unknown_does_not_persist_when_directional_edge_exists() {
        // A directional edge 1→2 already exists; a later non-directional move over the same
        // passage must not leave a lingering `?` stub. (SQ-0220)
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe_command(2, "B", "north"); // (1, N, 2)
        m.observe_command(1, "A", "south"); // (2, S, 1)
        m.observe_command(2, "B", "xyzzy"); // Unknown 1→2, immediately collapsed
        assert!(
            !m.graph.connections().iter().any(|c| c.dir == Direction::Unknown),
            "an Unknown 1→2 must not persist alongside the existing N edge: {:?}", m.graph.connections()
        );
        assert_eq!(m.graph.current(), Some(2));
    }

    #[test]
    fn relocation_updates_current_without_minting_edge() {
        // Grue death: walk A→(down)→Cellar, then a typed move kills the player and
        // resurrects them in a brand-new Forest. The relocation must move current to
        // Forest but create NO edge (no false passage Cellar→Forest). (SQ-0259)
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let edges_before = m.graph.connections().len();
        m.observe_relocation(3, "Forest");
        assert_eq!(m.graph.current(), Some(3), "current follows the player to the resurrection room");
        assert_eq!(m.graph.connections().len(), edges_before, "an involuntary relocation mints no edge");
        assert!(m.graph.room(3).is_some(), "resurrection room is added to the map");
        assert!(m.graph.room(3).unwrap().pos.is_some(), "resurrection room is placed so it renders");
    }

    #[test]
    fn relocation_to_known_room_keeps_position_and_mints_no_edge() {
        // Resurrecting into an already-mapped room must not move it or connect it.
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "Forest", Some(Direction::N)); // Forest already known & placed
        let forest_pos = m.graph.room(2).unwrap().pos;
        m.observe(1, "A", Some(Direction::S)); // back to A; current = 1
        let edges_before = m.graph.connections().len();
        m.observe_relocation(2, "Forest"); // die in A, resurrect in the known Forest
        assert_eq!(m.graph.current(), Some(2));
        assert_eq!(m.graph.room(2).unwrap().pos, forest_pos, "a known resurrection room does not move");
        assert_eq!(m.graph.connections().len(), edges_before, "no false edge to the known room");
    }

    #[test]
    fn relocation_as_first_observation_anchors_origin() {
        let mut m = Mapper::default();
        m.observe_relocation(1, "Forest");
        assert_eq!(m.graph.current(), Some(1));
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn restated_same_room_no_edge() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(1, "Hall", Some(Direction::N)); // look/again — same room
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn manual_mode_freezes_and_allows_nudge() {
        use crate::layout::LayoutMode;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(crate::direction::Direction::N));
        m.set_mode(LayoutMode::Manual);
        let before = m.graph.room(2).unwrap().pos.unwrap();
        // a new observation in Manual mode must NOT move existing rooms
        m.observe(3, "C", Some(crate::direction::Direction::E));
        assert_eq!(m.graph.room(2).unwrap().pos.unwrap(), before);
        // nudge room 2 to a known free cell
        let free = (before.0 + 5, before.1 + 5);
        assert!(m.nudge(2, free));
        assert_eq!(m.graph.room(2).unwrap().pos, Some(free));
    }

    #[test]
    fn first_room_anchors_at_origin() {
        let mut m = Mapper::default();
        m.observe(1, "Start", None);
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
    }

    #[test]
    fn incremental_observe_does_not_move_existing_rooms() {
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E)); // east of A
        let a = m.graph.room(1).unwrap().pos.unwrap();
        let b = m.graph.room(2).unwrap().pos.unwrap();
        m.observe(3, "C", Some(Direction::E)); // east of B
        // A and B must not have moved (C is placed past them, not into them).
        assert_eq!(m.graph.room(1).unwrap().pos.unwrap(), a, "A stayed put");
        assert_eq!(m.graph.room(2).unwrap().pos.unwrap(), b, "B stayed put");
        assert!(m.graph.room(3).unwrap().pos.unwrap().0 > b.0, "C is east of B");
    }

    #[test]
    fn revisit_adds_edge_without_moving_rooms() {
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::N));
        let snapshot: Vec<_> = m.graph.rooms().map(|r| (r.id, r.pos)).collect();
        // walk back south to A (already-placed room)
        m.observe(1, "A", Some(Direction::S));
        let after: Vec<_> = m.graph.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(snapshot, after, "returning to a placed room moves nothing");
    }

    #[test]
    fn light_corrections() {
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe_command(2, "B", "xyzzy"); // unknown edge
        m.rename_room(2, Some("The Grotto".into()));
        assert_eq!(m.graph.room(2).unwrap().label(), "The Grotto");
        m.set_notes(2, "secret".into());
        assert_eq!(m.graph.room(2).unwrap().notes, "secret");
        assert!(m.relabel_edge(1, Direction::Unknown, Direction::Down));
        assert_eq!(m.graph.connections()[0].dir, Direction::Down);
        assert!(m.delete_connection(1, Direction::Down));
        assert_eq!(m.graph.connections().len(), 0);
    }

    #[test]
    fn observe_incremental_shift_beyond_is_not_global_relayout() {
        // Discriminates incremental placement from a from-scratch global solve.
        // Build: A at origin, B north of A. Return to A, then observe C north of A.
        // C's ideal cell (0,-1) is occupied by B, so shift-beyond moves the BLOCKER
        // (B) further north and places the newcomer (C) truthfully at (0,-1) while A
        // stays put. A global relayout never shifts a blocker like this, so these
        // exact coordinates can only come from the incremental path.
        use crate::direction::Direction;
        let mut m = Mapper::default();
        m.observe(1, "A", None);                 // (0,0)
        m.observe(2, "B", Some(Direction::N));    // (0,-1)
        m.observe(1, "A", Some(Direction::S));    // return to A; current=1, A does not move
        m.observe(3, "C", Some(Direction::N));    // N of A: (0,-1) occupied -> shift-beyond
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)), "A must stay at origin");
        assert_eq!(m.graph.room(3).unwrap().pos, Some((0, -1)), "newcomer C truthfully north of A");
        assert_eq!(m.graph.room(2).unwrap().pos, Some((0, -2)), "blocker B shifted beyond, not the newcomer");
        // no overlap
        let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }
}
