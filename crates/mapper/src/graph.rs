use std::collections::BTreeMap;

use crate::direction::Direction;
use crate::layer::{LayerId, LayerMeta, MAIN_LAYER};

pub type RoomId = u16;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub label_override: Option<String>,
    pub notes: String,
    pub pos: Option<(i32, i32)>,
    #[serde(default)]
    pub layer: crate::layer::LayerId,
}

impl Room {
    pub fn label(&self) -> &str {
        match &self.label_override {
            Some(l) => l.as_str(),
            None => self.name.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Connection {
    pub origin: RoomId,
    pub dir: Direction,
    pub dest: RoomId,
    pub distorted: bool,
}

#[derive(Debug, Clone)]
pub struct MapGraph {
    rooms: BTreeMap<RoomId, Room>,
    conns: Vec<Connection>,
    current: Option<RoomId>,
    layers: BTreeMap<LayerId, LayerMeta>,
    next_layer_id: LayerId,
}

impl Default for MapGraph {
    fn default() -> Self {
        let mut layers = BTreeMap::new();
        layers.insert(MAIN_LAYER, LayerMeta::main());
        Self { rooms: BTreeMap::new(), conns: Vec::new(), current: None, layers, next_layer_id: 1 }
    }
}

impl MapGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct a `MapGraph` from persisted vecs. Builds the internal `BTreeMap` keyed by id.
    pub fn from_parts(
        rooms: Vec<Room>,
        connections: Vec<Connection>,
        current: Option<RoomId>,
        layers: BTreeMap<LayerId, LayerMeta>,
        next_layer_id: LayerId,
    ) -> Self {
        let rooms = rooms.into_iter().map(|r| (r.id, r)).collect();
        let mut layers = layers;
        if layers.is_empty() {
            layers.insert(MAIN_LAYER, LayerMeta::main());
        }
        let next_layer_id = next_layer_id.max(1);
        Self { rooms, conns: connections, current, layers, next_layer_id }
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

    pub fn layer_of(&self, id: RoomId) -> LayerId {
        self.rooms.get(&id).map(|r| r.layer).unwrap_or(MAIN_LAYER)
    }

    pub fn set_room_layer(&mut self, id: RoomId, layer: LayerId) {
        if let Some(r) = self.rooms.get_mut(&id) { r.layer = layer; }
    }

    pub fn rooms_in_layer(&self, layer: LayerId) -> Vec<RoomId> {
        let mut v: Vec<RoomId> = self.rooms.values().filter(|r| r.layer == layer).map(|r| r.id).collect();
        v.sort();
        v
    }

    pub fn layers(&self) -> &BTreeMap<LayerId, LayerMeta> { &self.layers }

    pub fn layer_name(&self, layer: LayerId) -> &str {
        self.layers.get(&layer).map(|m| m.name.as_str()).unwrap_or("")
    }

    pub fn set_layer_name(&mut self, layer: LayerId, name: String) {
        if let Some(m) = self.layers.get_mut(&layer) { m.name = name; }
    }

    pub fn new_layer(&mut self, parent: Option<LayerId>, name: String) -> LayerId {
        let id = self.next_layer_id;
        self.next_layer_id += 1;
        self.layers.insert(id, LayerMeta { name, parent });
        id
    }

    pub fn remove_layer(&mut self, layer: LayerId) {
        if layer != MAIN_LAYER { self.layers.remove(&layer); }
    }

    pub fn next_layer_id(&self) -> LayerId { self.next_layer_id }

    pub fn upsert_room(&mut self, id: RoomId, name: String) -> &mut Room {
        use std::collections::btree_map::Entry;
        match self.rooms.entry(id) {
            Entry::Occupied(e) => {
                e.into_mut().name = name;
            }
            Entry::Vacant(e) => {
                e.insert(Room {
                    id,
                    name,
                    label_override: None,
                    notes: String::new(),
                    pos: None,
                    layer: 0,
                });
            }
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

    /// Set the grid position of a room. Used by the layout engine.
    pub fn set_pos(&mut self, id: RoomId, pos: (i32, i32)) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.pos = Some(pos);
        }
    }

    /// Clear the grid position of a room (set to None). Used by the layout engine
    /// to reset positions before a full re-derivation.
    pub fn clear_pos(&mut self, id: RoomId) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.pos = None;
        }
    }

    /// Mark a connection as distorted by index. Used by the layout engine when a room
    /// cannot be placed at its preferred compass offset (collision).
    pub fn set_conn_distorted(&mut self, idx: usize, distorted: bool) {
        if let Some(conn) = self.conns.get_mut(idx) {
            conn.distorted = distorted;
        }
    }

    /// Set or clear the label_override for a room.
    pub fn set_label_override(&mut self, id: RoomId, label: Option<String>) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.label_override = label;
        }
    }

    /// Set the notes for a room.
    pub fn set_notes(&mut self, id: RoomId, notes: String) {
        if let Some(room) = self.rooms.get_mut(&id) {
            room.notes = notes;
        }
    }

    /// Remove the connection with key (origin, dir). Returns true if removed.
    pub fn remove_connection(&mut self, origin: RoomId, dir: Direction) -> bool {
        let before = self.conns.len();
        self.conns.retain(|c| !(c.origin == origin && c.dir == dir));
        self.conns.len() < before
    }

    /// Change the direction of the edge keyed (origin, old) to (origin, new).
    /// If an edge with key (origin, new) already exists, refuses and returns false.
    /// Returns true if the relabel happened.
    pub fn relabel_connection(&mut self, origin: RoomId, old: Direction, new: Direction) -> bool {
        // Refuse if a connection with (origin, new) already exists.
        if self.conns.iter().any(|c| c.origin == origin && c.dir == new) {
            return false;
        }
        if let Some(conn) = self.conns.iter_mut().find(|c| c.origin == origin && c.dir == old) {
            conn.dir = new;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;

    #[test]
    fn rooms_default_to_main_layer_and_can_move() {
        use crate::layer::MAIN_LAYER;
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_name(MAIN_LAYER), "Main");
        let l = g.new_layer(Some(MAIN_LAYER), "Basement".into());
        g.set_room_layer(2, l);
        assert_eq!(g.layer_of(2), l);
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1]);
        assert_eq!(g.rooms_in_layer(l), vec![2]);
        assert_eq!(g.layer_name(l), "Basement");
    }

    #[test]
    fn new_layer_ids_are_unique_and_main_cannot_be_removed() {
        let mut g = MapGraph::new();
        let a = g.new_layer(None, "A".into());
        let b = g.new_layer(None, "B".into());
        assert_ne!(a, b);
        g.remove_layer(crate::layer::MAIN_LAYER); // no-op
        assert_eq!(g.layer_name(crate::layer::MAIN_LAYER), "Main");
    }

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
