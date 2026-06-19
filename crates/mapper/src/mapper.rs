use crate::direction::{Direction, parse_direction};
use crate::graph::{MapGraph, RoomId};

#[derive(Debug, Default)]
pub struct Mapper {
    pub graph: MapGraph,
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
    }

    pub fn observe_command(&mut self, location: RoomId, name: &str, command: &str) {
        self.observe(location, name, parse_direction(command));
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
}
