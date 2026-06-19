use std::collections::BTreeMap;

use crate::direction::Direction;

pub type RoomId = u16;

#[derive(Debug, Clone)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub label_override: Option<String>,
    pub notes: String,
    pub pos: Option<(i32, i32)>,
}

impl Room {
    pub fn label(&self) -> &str {
        match &self.label_override {
            Some(l) => l.as_str(),
            None => self.name.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub origin: RoomId,
    pub dir: Direction,
    pub dest: RoomId,
    pub distorted: bool,
}

#[derive(Debug, Default)]
pub struct MapGraph {
    rooms: BTreeMap<RoomId, Room>,
    conns: Vec<Connection>,
    current: Option<RoomId>,
}

impl MapGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn room(&self, id: RoomId) -> Option<&Room> {
        self.rooms.get(&id)
    }

    pub fn rooms(&self) -> impl Iterator<Item = &Room> {
        self.rooms.values()
    }

    pub fn connections(&self) -> &[Connection] {
        &self.conns
    }

    pub fn current(&self) -> Option<RoomId> {
        self.current
    }

    pub fn upsert_room(&mut self, id: RoomId, name: String) -> &mut Room {
        if self.rooms.contains_key(&id) {
            self.rooms.get_mut(&id).unwrap().name = name;
        } else {
            self.rooms.insert(id, Room {
                id,
                name,
                label_override: None,
                notes: String::new(),
                pos: None,
            });
        }
        self.rooms.get_mut(&id).unwrap()
    }

    pub fn add_edge(&mut self, origin: RoomId, dir: Direction, dest: RoomId) {
        if let Some(conn) = self.conns.iter_mut().find(|c| c.origin == origin && c.dir == dir) {
            conn.dest = dest;
        } else {
            self.conns.push(Connection { origin, dir, dest, distorted: false });
        }
    }

    pub fn set_current(&mut self, id: RoomId) {
        self.current = Some(id);
    }

    pub fn room_mut_notes(&mut self, id: RoomId, notes: &str) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.notes = notes.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    #[test]
    fn distinct_ids_same_name_are_distinct_rooms() {
        let mut g = MapGraph::new();
        g.upsert_room(10, "Forest".into());
        g.upsert_room(11, "Forest".into());
        assert_eq!(g.rooms().count(), 2);
        assert_eq!(g.room(10).unwrap().label(), "Forest");
    }

    #[test]
    fn revisit_same_id_updates_not_duplicates() {
        let mut g = MapGraph::new();
        g.upsert_room(10, "Dark Room".into());
        g.room_mut_notes(10, "has lamp"); // helper or set notes directly in test
        g.upsert_room(10, "Lit Room".into()); // name changed (light came on)
        assert_eq!(g.rooms().count(), 1);
        assert_eq!(g.room(10).unwrap().name, "Lit Room");
        assert_eq!(g.room(10).unwrap().notes, "has lamp"); // edits preserved
    }

    #[test]
    fn directed_edge_no_symmetry_and_dedup() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::W, 1); // non-reciprocal back-edge
        g.add_edge(1, Direction::N, 2); // duplicate key → still one
        assert_eq!(g.connections().len(), 2);
    }
}
