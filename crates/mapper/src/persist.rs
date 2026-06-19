use crate::graph::{Connection, MapGraph, Room, RoomId};
use crate::layout::LayoutMode;
use crate::mapper::Mapper;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PersistState {
    pub version: u32,
    pub mode: LayoutMode,
    pub rooms: Vec<Room>,
    pub connections: Vec<Connection>,
    pub current: Option<RoomId>,
}

pub fn to_json(mapper: &Mapper) -> String {
    let state = PersistState {
        version: 1,
        mode: mapper.mode,
        rooms: mapper.graph.rooms().cloned().collect(),
        connections: mapper.graph.connections().to_vec(),
        current: mapper.graph.current(),
    };
    serde_json::to_string_pretty(&state).expect("PersistState is always serializable")
}

pub fn from_json(s: &str) -> Result<Mapper, serde_json::Error> {
    let state: PersistState = serde_json::from_str(s)?;
    let graph = MapGraph::from_parts(state.rooms, state.connections, state.current);
    Ok(Mapper { graph, mode: state.mode })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::Mapper;
    use crate::direction::Direction;
    use crate::layout::LayoutMode;

    #[test]
    fn round_trips_full_state() {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "Forest", Some(Direction::N));
        m.set_notes(2, "trees\nwith a newline".into()); // freeform notes incl newline
        m.rename_room(2, Some("Deep Forest".into()));
        m.set_mode(LayoutMode::Manual);
        let json = to_json(&m);
        let m2 = from_json(&json).unwrap();
        assert_eq!(m2.graph.room(2).unwrap().label(), "Deep Forest");
        assert_eq!(m2.graph.room(2).unwrap().notes, "trees\nwith a newline");
        assert_eq!(m2.graph.current(), Some(2));
        assert_eq!(m2.mode, LayoutMode::Manual);
        assert_eq!(m2.graph.connections(), m.graph.connections());
        assert_eq!(m2.graph.room(2).unwrap().pos, m.graph.room(2).unwrap().pos);
    }
}
