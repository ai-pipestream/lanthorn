use std::collections::BTreeMap;
use crate::graph::{Connection, MapGraph, Room, RoomId};
use crate::layer::{LayerId, LayerMeta};
use crate::mapper::Mapper;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistState {
    pub version: u32,
    pub rooms: Vec<Room>,
    pub connections: Vec<Connection>,
    pub current: Option<RoomId>,
    #[serde(default)]
    pub layers: BTreeMap<LayerId, LayerMeta>,
    #[serde(default)]
    pub next_layer_id: LayerId,
}

pub fn to_json(mapper: &Mapper) -> String {
    let state = PersistState {
        version: 1,
        rooms: mapper.graph.rooms().cloned().collect(),
        connections: mapper.graph.connections().to_vec(),
        current: mapper.graph.current(),
        layers: mapper.graph.layers().clone(),
        next_layer_id: mapper.graph.next_layer_id(),
    };
    serde_json::to_string_pretty(&state).expect("PersistState is always serializable")
}

pub fn from_json(s: &str) -> Result<Mapper, serde_json::Error> {
    let state: PersistState = serde_json::from_str(s)?;
    let mut graph = MapGraph::from_parts(
        state.rooms, state.connections, state.current, state.layers, state.next_layer_id,
    );
    // Collapse `?` stubs that a real directional edge already covers, so existing saved maps
    // clean up on load. (SQ-0220)
    graph.collapse_unknown_edges();
    // A loaded map has no walked arrival: the player has not moved yet this
    // session, so a bare peel falls back to the portal-seam search until they do.
    Ok(Mapper { graph, arrived_via: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::Mapper;
    use crate::direction::Direction;

    #[test]
    fn round_trips_layers() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let l = m.graph.new_layer(Some(0), "Basement".into());
        m.graph.set_room_layer(2, l);
        let json = to_json(&m);
        let m2 = from_json(&json).unwrap();
        assert_eq!(m2.graph.layer_of(2), l);
        assert_eq!(m2.graph.layer_name(l), "Basement");
        assert_eq!(m2.graph.next_layer_id(), m.graph.next_layer_id());
    }

    /// SQ-0666: the maze flag, the per-layer view mode and self-loop edges all have to survive
    /// the map file, or `/mark-maze-layer` and `/view-map` would be undone by every reload.
    #[test]
    fn round_trips_maze_flag_view_mode_and_self_loops() {
        use crate::layer::MapView;
        let mut m = Mapper::default();
        m.observe(1, "Maze", None);
        m.observe(2, "Maze", Some(Direction::N));
        let l = m.graph.new_layer(Some(0), "Maze".into());
        m.graph.set_room_layer(2, l);
        m.graph.set_layer_maze(l, true);
        m.graph.set_layer_view(l, Some(MapView::Drawn)); // an explicit choice AGAINST the default
        assert!(m.graph.add_self_loop(1, Direction::W));

        let m2 = from_json(&to_json(&m)).unwrap();
        assert!(m2.graph.layer_is_maze(l), "the maze flag survives");
        assert_eq!(
            m2.graph.layer_view(l),
            MapView::Drawn,
            "an explicit view choice survives and still beats the maze default"
        );
        assert_eq!(m2.graph.layer_view_choice(0), None, "an unchosen layer stays unchosen");
        assert_eq!(m2.graph.self_loops(1), vec![Direction::W], "the self-loop edge survives");
    }

    /// A layer written before SQ-0666 has neither field; it must load as an ordinary drawn,
    /// non-maze layer rather than failing the whole map.
    #[test]
    fn a_layer_saved_before_the_maze_flag_loads_as_a_plain_drawn_layer() {
        use crate::layer::MapView;
        let json = r#"{"version":1,
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0],"layer":1}],
            "connections":[],"current":1,
            "layers":{"0":{"name":"Main","parent":null},"1":{"name":"Cellar","parent":0}},
            "next_layer_id":2}"#;
        let m = from_json(json).expect("a pre-SQ-0666 map still loads");
        assert!(!m.graph.layer_is_maze(1));
        assert_eq!(m.graph.layer_view(1), MapView::Drawn);
    }

    /// Maps written before SQ-0600 carry a `"mode"` field that no longer exists
    /// on `PersistState`. Serde ignores unknown fields, so they still load —
    /// pinned here because a stray `deny_unknown_fields` would silently make
    /// every previously-saved map unreadable.
    #[test]
    fn a_map_saved_with_the_old_layout_mode_field_still_loads() {
        let json = r#"{"version":1,"mode":"Manual",
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1}"#;
        let m = from_json(json).expect("a pre-SQ-0600 map still loads");
        assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
        assert_eq!(m.graph.current(), Some(1));
    }

    #[test]
    fn from_json_collapses_redundant_unknown_edges() {
        // An existing save with a redundant `?` 1→2 (a real N 1→2 already covers it) plus a lone
        // `?` 2→3 (no known counterpart). Loading collapses the redundant one, keeps the lone one.
        // (SQ-0220)
        let json = r#"{"version":1,"mode":"Auto",
            "rooms":[
                {"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]},
                {"id":2,"name":"B","label_override":null,"notes":"","pos":[0,-1]},
                {"id":3,"name":"C","label_override":null,"notes":"","pos":[1,0]}],
            "connections":[
                {"origin":1,"dir":"Unknown","dest":2,"distorted":false},
                {"origin":1,"dir":"N","dest":2,"distorted":false},
                {"origin":2,"dir":"Unknown","dest":3,"distorted":false}],
            "current":1}"#;
        let m = from_json(json).unwrap();
        assert!(
            !m.graph.connections().iter().any(|c| c.origin == 1 && c.dir == Direction::Unknown),
            "the redundant Unknown 1→2 collapsed on load: {:?}", m.graph.connections()
        );
        assert!(
            m.graph.connections().iter().any(|c| c.origin == 2 && c.dir == Direction::Unknown && c.dest == 3),
            "the lone Unknown 2→3 (no known counterpart) survives load"
        );
        assert_eq!(m.graph.connections().len(), 2);
    }

    /// SQ-0632: a map file referencing room ids that do not exist (hand-edited, truncated, or
    /// corrupt) must be cleaned on load. Phantom endpoint ids otherwise enter layout components
    /// via `connected_components`' adjacency insertion, permanently wasting a grid cell and
    /// flagging a stray distorted edge; a dangling `current` misdraws the highlight forever.
    #[test]
    fn dangling_connections_and_current_are_dropped_on_load() {
        let json = r#"{"version":1,
            "rooms":[
                {"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]},
                {"id":2,"name":"B","label_override":null,"notes":"","pos":[1,0]}],
            "connections":[
                {"origin":1,"dir":"E","dest":2,"distorted":false},
                {"origin":1,"dir":"N","dest":99,"distorted":false},
                {"origin":98,"dir":"S","dest":1,"distorted":false}],
            "current":42}"#;
        let m = from_json(json).unwrap();
        assert_eq!(
            m.graph.connections().len(),
            1,
            "both connections with a phantom endpoint are dropped: {:?}",
            m.graph.connections()
        );
        assert_eq!(m.graph.connections()[0].dest, 2, "the valid edge survives");
        assert_eq!(m.graph.current(), None, "a current naming no room is reset");
    }

    #[test]
    fn legacy_save_without_layers_loads_as_main() {
        // A v1 save predating layers: no `layer` on rooms, no `layers`/`next_layer_id`.
        let json = r#"{"version":1,"mode":"Auto",
            "rooms":[{"id":1,"name":"A","label_override":null,"notes":"","pos":[0,0]}],
            "connections":[],"current":1}"#;
        let m = from_json(json).unwrap();
        assert_eq!(m.graph.layer_of(1), 0);
        assert_eq!(m.graph.layer_name(0), "Main");
    }

    #[test]
    fn round_trips_full_state() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        m.set_notes(2, "trees\nwith a newline".into()); // freeform notes incl newline
        m.rename_room(2, Some("Deep Forest".into()));
        let json = to_json(&m);
        let m2 = from_json(&json).unwrap();
        assert_eq!(m2.graph.room(2).unwrap().label(), "Deep Forest");
        assert_eq!(m2.graph.room(2).unwrap().notes, "trees\nwith a newline");
        assert_eq!(m2.graph.current(), Some(2));
        assert_eq!(m2.graph.connections(), m.graph.connections());
        assert_eq!(m2.graph.room(2).unwrap().pos, m.graph.room(2).unwrap().pos);
    }
}
