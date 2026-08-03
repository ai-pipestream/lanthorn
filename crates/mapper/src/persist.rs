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
