use crate::direction::{Direction, parse_direction};
use crate::graph::{MapGraph, RoomId};
use crate::layout::{occupied_cells, relayout_auto, LayoutMode};

#[derive(Debug, Default)]
pub struct Mapper {
    pub graph: MapGraph,
    pub mode: LayoutMode,
}

impl Mapper {
    pub fn observe(&mut self, location: RoomId, name: &str, via: Option<Direction>) {
        self.graph.upsert_room(location, name.to_string());
        let prev = self.graph.current();
        if let Some(prev_id) = prev {
            if location != prev_id {
                let edge_dir = via.unwrap_or(Direction::Unknown);
                self.graph.add_edge(prev_id, edge_dir, location);
            }
        }
        self.graph.set_current(location);
        if self.mode == LayoutMode::Auto {
            relayout_auto(&mut self.graph);
        }
    }

    pub fn observe_command(&mut self, location: RoomId, name: &str, command: &str) {
        self.observe(location, name, parse_direction(command));
    }

    /// Switch layout mode.
    /// Auto→Manual: run relayout_auto first so every room has a pos, then freeze.
    /// Manual→Auto: re-enable auto relayout (existing positions are kept as starting point).
    /// Setting the same mode is a no-op.
    pub fn set_mode(&mut self, mode: LayoutMode) {
        if self.mode == mode {
            return;
        }
        if mode == LayoutMode::Manual {
            relayout_auto(&mut self.graph);
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
}
