use std::collections::BTreeMap;

use crate::direction::Direction;
use crate::layer::{LayerId, LayerMeta, MapView, MAIN_LAYER};

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
    /// How the mapper worked out that the player was here, recorded the first
    /// time the room was discovered and kept thereafter (SQ-0527). Stored as the
    /// display label rather than an engine enum, so the map file stays readable
    /// and the mapper keeps no dependency on any particular VM crate. `None` for
    /// rooms mapped before this was recorded.
    #[serde(default)]
    pub loc_method: Option<String>,
    /// Compass directions the player has TYPED while standing in this room, whether or not the
    /// move worked (SQ-0391). What it answers is "where have I not tried yet?", so a direction
    /// that bounced off a wall counts as tried just as much as one that led somewhere — the map
    /// stops nagging about it either way.
    ///
    /// A `Vec` rather than a set because [`Direction`] is deliberately not `Ord`, and the list is
    /// at most eight long. Absent from older map files, hence `serde(default)`.
    #[serde(default)]
    pub tried: Vec<Direction>,
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

impl Connection {
    /// True when this edge leads back into the room it leaves — "west takes me back here"
    /// (SQ-0666). Recordable knowledge, but not GEOMETRY: it says nothing about where any room
    /// is, so layout, routing and distortion skip it (the drawn view shows it as a badge on the
    /// room box, the matrix view as `↩`). Feeding one to a compass-offset placer would ask
    /// where a room sits relative to itself, and answer "distorted" forever.
    pub fn is_self_loop(&self) -> bool {
        self.origin == self.dest
    }
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
    ///
    /// References are validated: a connection whose endpoint is not a room is dropped, and a
    /// `current` naming no room is reset. A hand-edited or corrupt map file could otherwise
    /// smuggle phantom ids into layout components (`connected_components` adjacency-inserts
    /// every connection endpoint), permanently wasting a grid cell and flagging a stray
    /// distorted edge (SQ-0632).
    pub fn from_parts(
        rooms: Vec<Room>,
        connections: Vec<Connection>,
        current: Option<RoomId>,
        layers: BTreeMap<LayerId, LayerMeta>,
        next_layer_id: LayerId,
    ) -> Self {
        let rooms: BTreeMap<RoomId, Room> = rooms.into_iter().map(|r| (r.id, r)).collect();
        let conns: Vec<Connection> = connections
            .into_iter()
            .filter(|c| rooms.contains_key(&c.origin) && rooms.contains_key(&c.dest))
            .collect();
        let current = current.filter(|id| rooms.contains_key(id));
        let mut layers = layers;
        if layers.is_empty() {
            layers.insert(MAIN_LAYER, LayerMeta::main());
        }
        let next_layer_id = next_layer_id.max(1);
        Self { rooms, conns, current, layers, next_layer_id }
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
        self.layers.insert(id, LayerMeta::new(name, parent));
        id
    }

    /// True when the player has flagged `layer` as a maze (SQ-0666).
    pub fn layer_is_maze(&self, layer: LayerId) -> bool {
        self.layers.get(&layer).is_some_and(|m| m.maze)
    }

    /// Flag/unflag `layer` as a maze. Returns the new value (unchanged for an unknown layer).
    /// The flag only decides the DEFAULT view, so a layer whose view the player chose by hand
    /// keeps that choice either way.
    pub fn set_layer_maze(&mut self, layer: LayerId, maze: bool) -> bool {
        if let Some(m) = self.layers.get_mut(&layer) {
            m.maze = maze;
        }
        self.layer_is_maze(layer)
    }

    /// The view `layer` draws in — the player's explicit choice, else the maze-flag default.
    pub fn layer_view(&self, layer: LayerId) -> MapView {
        self.layers.get(&layer).map(|m| m.effective_view()).unwrap_or_default()
    }

    /// The player's EXPLICIT view choice for `layer`, or `None` when they have not made one.
    pub fn layer_view_choice(&self, layer: LayerId) -> Option<MapView> {
        self.layers.get(&layer).and_then(|m| m.view)
    }

    /// Set (or clear, with `None`) the player's explicit view choice for `layer`.
    pub fn set_layer_view(&mut self, layer: LayerId, view: Option<MapView>) {
        if let Some(m) = self.layers.get_mut(&layer) {
            m.view = view;
        }
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
                    layer: MAIN_LAYER,
                    loc_method: None,
                    tried: Vec::new(),
                });
            }
        }
        self.rooms.get_mut(&id).unwrap()
    }

    pub fn add_edge(&mut self, origin: RoomId, dir: Direction, dest: RoomId) {
        // A compass direction (or Up/Down/In/Out) can only lead one place, so those edges are
        // keyed (origin, dir) and a repeat observation updates the destination. Unknown is not a
        // direction but a bucket: a room can hold SEVERAL non-compass passages (xyzzy and pray
        // from the same room), so Unknown edges are keyed by the full (origin, dir, dest) triple —
        // a second passage adds a second `?` stub instead of silently erasing the first, and an
        // exact repeat is still one edge (SQ-0632). Layout already treats Unknown as non-spatial
        // (skipped in components, drawn as a stub), so multiples cost nothing there.
        //
        // A SELF-LOOP (`origin == dest`) is triple-keyed for the same reason (SQ-0666): "west
        // leads back here" is a fact about a maze, and it must neither erase a known passage
        // that shares its key nor be erased by one. Keeping both lets the graph hold the
        // contradiction honestly — the matrix view prefers the real destination and falls back
        // to `↩` — rather than silently picking a winner.
        if dir == Direction::Unknown || origin == dest {
            if !self.conns.iter().any(|c| c.origin == origin && c.dir == dir && c.dest == dest) {
                self.conns.push(Connection { origin, dir, dest, distorted: false });
            }
        } else if let Some(conn) =
            self.conns.iter_mut().find(|c| c.origin == origin && c.dir == dir && c.dest != c.origin)
        {
            conn.dest = dest;
        } else {
            self.conns.push(Connection { origin, dir, dest, distorted: false });
        }
    }

    /// Record that `dir` out of `room` leads back INTO `room` — an observed same-room arrival
    /// (SQ-0666). Returns false for an unknown room or a direction with no meaning
    /// ([`Direction::Unknown`], which is a bucket, not a passage).
    ///
    /// Only an observed arrival may call this. A direction that was TYPED and went nowhere is
    /// already recorded by [`MapGraph::mark_tried`] and shows as "tried, no path"; guessing that
    /// every such direction is a loop would invent passages out of walls.
    pub fn add_self_loop(&mut self, room: RoomId, dir: Direction) -> bool {
        if dir == Direction::Unknown || !self.rooms.contains_key(&room) {
            return false;
        }
        self.add_edge(room, dir, room);
        self.mark_tried(room, dir);
        true
    }

    /// The self-loop directions recorded for `room`, in connection order.
    pub fn self_loops(&self, room: RoomId) -> Vec<Direction> {
        self.conns
            .iter()
            .filter(|c| c.origin == room && c.dest == room)
            .map(|c| c.dir)
            .collect()
    }

    /// Drop every Unknown-direction edge whose room pair (same origin→dest) already carries a
    /// known-direction edge; the redundant `?` stub goes, the known edge stays. Reverse
    /// (dest→origin) edges do NOT count — a return trip is not guaranteed to be the geometric
    /// opposite (one-way passages, mazes), so no forward direction is ever inferred from it, and
    /// nothing is relabeled: the replacing direction already exists as its own edge. Unknown edges
    /// with no same-direction known counterpart are left untouched. Returns the number removed.
    /// (SQ-0220)
    pub fn collapse_unknown_edges(&mut self) -> usize {
        let known: std::collections::HashSet<(RoomId, RoomId)> = self
            .conns
            .iter()
            .filter(|c| c.dir != Direction::Unknown)
            .map(|c| (c.origin, c.dest))
            .collect();
        let before = self.conns.len();
        self.conns
            .retain(|c| c.dir != Direction::Unknown || !known.contains(&(c.origin, c.dest)));
        before - self.conns.len()
    }

    /// Give room `old` the id `new`, rewriting every reference to it (SQ-0526).
    ///
    /// The Glulx side identifies a room by hashing its printed NAME until it has
    /// worked out where the game keeps its `location` global, then switches to the
    /// room's real object address. The handful of rooms mapped during that
    /// learning window carry name-derived ids, and would otherwise reappear as
    /// duplicate nodes the moment the player walked back into them. Re-keying them
    /// keeps one node per room across the switch.
    ///
    /// Returns `false` and changes nothing when `old` is unknown, when the ids are
    /// equal, or when `new` is ALREADY a room — that last case is a merge, not a
    /// rename, and silently folding two mapped rooms together could destroy real
    /// structure. The caller is left with the duplicate rather than a guess.
    pub fn rekey_room(&mut self, old: RoomId, new: RoomId) -> bool {
        if old == new || !self.rooms.contains_key(&old) || self.rooms.contains_key(&new) {
            return false;
        }
        let Some(mut room) = self.rooms.remove(&old) else { return false };
        room.id = new;
        self.rooms.insert(new, room);
        for c in &mut self.conns {
            if c.origin == old {
                c.origin = new;
            }
            if c.dest == old {
                c.dest = new;
            }
        }
        // A rename can make two edges identical (both ends re-keyed onto the same
        // pair); keep the first of each.
        let mut seen: Vec<(RoomId, Direction, RoomId)> = Vec::new();
        self.conns.retain(|c| {
            let key = (c.origin, c.dir, c.dest);
            let fresh = !seen.contains(&key);
            if fresh {
                seen.push(key);
            }
            fresh
        });
        if self.current == Some(old) {
            self.current = Some(new);
        }
        true
    }

    /// Record how this room was detected, the FIRST time it is discovered
    /// (SQ-0527). Later visits leave it alone: the interesting fact is how the
    /// mapper first came to know the room, and a later visit may well be resolved
    /// by a weaker method (a name match rather than the object) without that
    /// saying anything new. A no-op for an unknown room.
    /// Record that `dir` was TYPED while standing in `id`, whether or not it moved the player
    /// (SQ-0391). Idempotent — a direction you try twice is still one tried direction.
    pub fn mark_tried(&mut self, id: RoomId, dir: Direction) {
        if let Some(r) = self.rooms.get_mut(&id) {
            if !r.tried.contains(&dir) {
                r.tried.push(dir);
            }
        }
    }

    /// The compass directions never typed in this room — what a player has left to explore
    /// (SQ-0391). Directions that LED somewhere are tried by definition, so an edge out counts
    /// even on a map loaded from before this was recorded.
    /// True when `dir` has been explored from `id`: typed there (worked or not), or carrying an
    /// edge that leads somewhere — the only signal a map saved before `tried` existed still has.
    pub fn is_tried(&self, id: RoomId, dir: Direction) -> bool {
        let typed = self.rooms.get(&id).is_some_and(|r| r.tried.contains(&dir));
        typed || self.conns.iter().any(|c| c.origin == id && c.dir == dir)
    }

    pub fn untried(&self, id: RoomId) -> Vec<Direction> {
        if !self.rooms.contains_key(&id) {
            return Vec::new();
        }
        crate::direction::UNTRIED_DIRS.iter().copied().filter(|d| !self.is_tried(id, *d)).collect()
    }

    /// Test-only: drop a room's recorded attempts, to stand in for a map file written before
    /// they were recorded.
    #[doc(hidden)]
    pub fn room_mut_tried_clear_for_test(&mut self, id: RoomId) {
        if let Some(r) = self.rooms.get_mut(&id) {
            r.tried.clear();
        }
    }

    pub fn set_loc_method(&mut self, id: RoomId, method: &str) {
        if let Some(r) = self.rooms.get_mut(&id) {
            if r.loc_method.is_none() {
                r.loc_method = Some(method.to_string());
            }
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

    /// Remove the connection(s) with key (origin, dir). Returns true if any was removed.
    /// For a real direction that is at most one edge; for `Unknown` — which may hold several
    /// stubs (see [`MapGraph::add_edge`]) — every `?` edge out of `origin` goes at once, as the
    /// key names no destination to tell them apart by.
    pub fn remove_connection(&mut self, origin: RoomId, dir: Direction) -> bool {
        let before = self.conns.len();
        self.conns.retain(|c| !(c.origin == origin && c.dir == dir));
        self.conns.len() < before
    }

    /// A sub-graph containing only `layer`'s rooms and the connections whose BOTH
    /// endpoints are in `layer`. Positions are preserved; `current` carries over only
    /// if the current room is in `layer`. Layer metadata is not copied (not needed for routing).
    pub fn layer_subgraph(&self, layer: LayerId) -> MapGraph {
        let in_layer: std::collections::BTreeSet<RoomId> =
            self.rooms.values().filter(|r| r.layer == layer).map(|r| r.id).collect();
        let rooms: BTreeMap<RoomId, Room> = self
            .rooms
            .values()
            .filter(|r| in_layer.contains(&r.id))
            .map(|r| (r.id, r.clone()))
            .collect();
        let conns: Vec<Connection> = self
            .conns
            .iter()
            .filter(|c| in_layer.contains(&c.origin) && in_layer.contains(&c.dest))
            .cloned()
            .collect();
        let current = self.current.filter(|id| in_layer.contains(id));
        let mut layers = BTreeMap::new();
        layers.insert(MAIN_LAYER, LayerMeta::main());
        MapGraph { rooms, conns, current, layers, next_layer_id: 1 }
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
    /// SQ-0526: re-keying a room must move every reference with it, and must
    /// refuse the cases where it would destroy structure.
    #[test]
    fn rekey_room_moves_edges_and_current_and_refuses_a_merge() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Hall".into());
        g.upsert_room(2, "Cave".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        g.set_current(1);

        assert!(g.rekey_room(1, 99), "a plain rename succeeds");
        assert!(g.room(1).is_none(), "the old id is gone");
        assert_eq!(g.room(99).map(|r| r.name.as_str()), Some("Hall"), "the room came with it");
        assert_eq!(g.room(99).map(|r| r.id), Some(99), "and its own id field was updated");
        assert_eq!(g.current(), Some(99), "the current pointer followed");
        assert!(
            g.connections().iter().any(|c| c.origin == 99 && c.dest == 2),
            "the outgoing edge followed: {:?}",
            g.connections()
        );
        assert!(
            g.connections().iter().any(|c| c.origin == 2 && c.dest == 99),
            "and the incoming edge: {:?}",
            g.connections()
        );

        assert!(!g.rekey_room(99, 99), "renaming to itself is a no-op");
        assert!(!g.rekey_room(1234, 5678), "an unknown room is a no-op");
        assert!(
            !g.rekey_room(99, 2),
            "re-keying ONTO an existing room would be a merge, not a rename, and must be refused"
        );
        assert_eq!(g.room(2).map(|r| r.name.as_str()), Some("Cave"), "the refused merge changed nothing");
    }

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

    /// SQ-0666: the maze flag only ever changes the DEFAULT view. A view the player picked by
    /// hand must survive being flagged and unflagged, or `/mark-maze-layer` would silently undo
    /// `/view-map`.
    #[test]
    fn the_maze_flag_moves_the_default_view_but_never_overrides_a_chosen_one() {
        use crate::layer::MapView;
        let mut g = MapGraph::new();
        let l = g.new_layer(None, "Maze".into());
        assert!(!g.layer_is_maze(l));
        assert_eq!(g.layer_view(l), MapView::Drawn, "an ordinary layer draws");
        assert_eq!(g.layer_view_choice(l), None, "and has made no choice");

        assert!(g.set_layer_maze(l, true));
        assert_eq!(g.layer_view(l), MapView::Matrix, "flagging a maze defaults it to the matrix");
        assert_eq!(g.layer_view_choice(l), None, "…without recording a choice the player never made");

        g.set_layer_view(l, Some(MapView::Drawn));
        assert_eq!(g.layer_view(l), MapView::Drawn, "an explicit choice beats the maze default");
        assert!(!g.set_layer_maze(l, false));
        assert_eq!(g.layer_view(l), MapView::Drawn, "and survives the flag going away again");

        g.set_layer_view(l, None);
        assert_eq!(g.layer_view(l), MapView::Drawn, "clearing the choice falls back to the default");

        // An unknown layer answers without panicking.
        assert!(!g.layer_is_maze(999));
        assert_eq!(g.layer_view(999), MapView::Drawn);
    }

    /// SQ-0666: "west leads back here" is a fact, and recording it must not cost the map a
    /// passage it already knew about on the same key.
    #[test]
    fn a_self_loop_is_recorded_beside_a_real_passage_not_instead_of_it() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Maze".into());
        g.upsert_room(2, "Maze".into());
        g.add_edge(1, Direction::W, 2);

        assert!(g.add_self_loop(1, Direction::W), "an observed loop on a used key is recordable");
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::W && c.dest == 2),
            "the real passage 1-W->2 is still there: {:?}",
            g.connections()
        );
        assert_eq!(g.self_loops(1), vec![Direction::W], "and the loop sits beside it");
        assert!(g.is_tried(1, Direction::W), "recording a loop marks the direction tried");

        g.add_self_loop(1, Direction::W); // an exact repeat
        assert_eq!(g.connections().len(), 2, "a repeated observation is still one loop");

        // …and the reverse: a later real destination on that key must not eat the loop.
        g.add_edge(1, Direction::W, 2);
        assert_eq!(g.self_loops(1), vec![Direction::W], "the loop survives a re-observed passage");

        assert!(!g.add_self_loop(1, Direction::Unknown), "`?` is a bucket, not a passage");
        assert!(!g.add_self_loop(404, Direction::N), "an unknown room records nothing");
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

    /// SQ-0632: a room can hold several non-compass passages (xyzzy AND pray). Keying Unknown
    /// edges by (origin, dir) alone made the second silently overwrite the first's destination,
    /// losing a recorded passage. Unknown edges to DIFFERENT destinations must coexist; an exact
    /// repeat is still one edge.
    #[test]
    fn a_room_keeps_multiple_unknown_passages() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Cave"), (2, "Grotto"), (3, "Chapel")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::Unknown, 2); // xyzzy
        g.add_edge(1, Direction::Unknown, 3); // pray, from the same room
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 2),
            "the xyzzy passage survives the second Unknown edge: {:?}",
            g.connections()
        );
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 3),
            "and the pray passage was recorded beside it"
        );
        g.add_edge(1, Direction::Unknown, 2); // repeat observation of the same passage
        assert_eq!(g.connections().len(), 2, "an exact repeat is still one edge");

        // A directional edge over one of the pairs collapses only THAT pair's stub.
        g.add_edge(1, Direction::N, 2);
        assert_eq!(g.collapse_unknown_edges(), 1);
        assert!(
            g.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown && c.dest == 3),
            "the other room's passage is untouched"
        );
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

    #[test]
    fn collapse_unknown_drops_redundant_same_pair_edge() {
        // A→B carries both an Unknown edge and a known N edge (same origin→dest). The Unknown
        // is redundant and is dropped; the known edge stays. (SQ-0220)
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::Unknown, 2);
        g.add_edge(1, Direction::N, 2);
        let removed = g.collapse_unknown_edges();
        assert_eq!(removed, 1, "the redundant Unknown A→B is removed");
        assert_eq!(g.connections().len(), 1);
        assert_eq!(g.connections()[0].dir, Direction::N, "the known edge survives");
    }

    #[test]
    fn collapse_unknown_keeps_reverse_only_and_lone_unknowns() {
        // A→B Unknown must survive when only the REVERSE B→A is directional (return trips are
        // not guaranteed to be the geometric opposite), and when it has no known counterpart.
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3, 4] { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::Unknown, 2); // reverse-only pair
        g.add_edge(2, Direction::S, 1); // return trip is directional, forward was Unknown
        g.add_edge(3, Direction::Unknown, 4); // lone Unknown, no known counterpart
        let removed = g.collapse_unknown_edges();
        assert_eq!(removed, 0, "neither Unknown has a same-origin→dest known edge");
        assert_eq!(g.connections().len(), 3);
    }

    #[test]
    fn collapse_unknown_ignores_known_edge_to_a_different_dest() {
        // A→B Unknown is not affected by a known A→C edge (same origin, different dest).
        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::Unknown, 2);
        g.add_edge(1, Direction::N, 3);
        let removed = g.collapse_unknown_edges();
        assert_eq!(removed, 0);
        assert_eq!(g.connections().len(), 2);
    }
}
