use mapper::mapper::Mapper;
use mapper::render::render;
use mapper::persist::{to_json, from_json};
use mapper::direction::Direction;

#[test]
fn small_session_builds_consistent_map() {
    let mut m = Mapper::default();
    // A grid: West of House (1) - E -> Kitchen (2) - N -> Attic (3); back W from Kitchen to 1 (non-reciprocal? no, reciprocal)
    m.observe_command(1, "West of House", "look");
    m.observe_command(2, "Kitchen", "east");
    m.observe_command(3, "Attic", "north");
    m.observe_command(2, "Kitchen", "south");   // back down (non-reciprocal vs 'north'? 2->3 was N, 3->2 is S = reciprocal-opposite)
    m.observe_command(1, "West of House", "west"); // 2->1 W; reciprocal of 1->2 E

    // graph shape
    assert_eq!(m.graph.rooms().count(), 3);
    // no overlapping rooms after layout
    let cells: Vec<_> = m.graph.rooms().filter_map(|r| r.pos).collect();
    let set: std::collections::BTreeSet<_> = cells.iter().collect();
    assert_eq!(cells.len(), set.len());

    // render reflects current room (last observed = 1)
    let rm = render(&m.graph);
    assert!(rm.rooms.iter().find(|r| r.id == 1).unwrap().is_current);

    // persistence round-trips the whole thing
    let j = to_json(&m);
    let m2 = from_json(&j).unwrap();
    assert_eq!(m2.graph.connections(), m.graph.connections());

    // a non-compass detour adds a room joined by an Unknown edge (never lost)
    let mut m3 = m;
    m3.observe_command(99, "Mysterious Void", "pray");
    assert!(m3.graph.room(99).is_some());
    assert!(m3.graph.connections().iter().any(|c| c.dir == Direction::Unknown && c.dest == 99));
}
