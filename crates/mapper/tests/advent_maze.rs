//! The matrix view against REAL player data (SQ-0666).
//!
//! `unit_tests/advent_maze_map.json` is a verbatim copy of the `map.json` inside a babelmap
//! archive: one player's partial mapping of Colossal Cave, mid-game, with the "all alike" maze
//! hand-peeled onto layer 1. Every number asserted here was measured from that file, not invented
//! — which is the point. A synthetic fixture would agree with whatever the classifier happens to
//! do; this one already disagreed with the drawn map before a line of this feature existed.

use std::collections::BTreeMap;

use mapper::direction::Direction;
use mapper::graph::{MapGraph, RoomId};
use mapper::matrix::{self, MatrixCell};

const MAZE_LAYER: mapper::layer::LayerId = 1;

fn advent() -> MapGraph {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file").graph
}

/// Room id by row label, e.g. `"Maze 11"` or `"Dead End, near Vending Machine"`.
fn by_label(g: &MapGraph) -> BTreeMap<String, RoomId> {
    matrix::labels(g, MAZE_LAYER).row.into_iter().map(|(id, l)| (l, id)).collect()
}

/// The statistics the design was argued from. If the fixture is ever replaced these change, and
/// every other assertion in this file is suspect — so they are pinned first and loudly.
#[test]
fn the_fixture_is_the_map_the_design_was_argued_from() {
    let g = advent();
    let m = matrix::build(&g, MAZE_LAYER);
    assert_eq!(m.rows.len(), 12, "12 rooms on the maze layer");

    let in_layer: Vec<_> = g
        .connections()
        .iter()
        .filter(|c| g.layer_of(c.origin) == MAZE_LAYER && g.layer_of(c.dest) == MAZE_LAYER)
        .collect();
    assert_eq!(in_layer.len(), 47, "47 in-layer edges");

    let mut recip = 0;
    let mut asym = 0;
    let mut none = 0;
    for r in &m.rows {
        for c in r.cells {
            match c {
                MatrixCell::Reciprocal { .. } => recip += 1,
                MatrixCell::ReturnBy { .. } => asym += 1,
                MatrixCell::OneWay { .. } => none += 1,
                _ => {}
            }
        }
    }
    assert_eq!((recip, asym, none), (2, 18, 27), "2 reciprocal, 18 return another way, 27 one-way");
    assert_eq!(recip + asym + none, 47);

    let distorted = in_layer.iter().filter(|c| c.distorted).count();
    assert_eq!(distorted, 29, "the drawn map is wrong about 29 of 47 edges — why the matrix exists");
}

/// The cells the design document names, read straight off the real map.
#[test]
fn the_cells_the_spec_names_classify_as_it_says() {
    let g = advent();
    let ids = by_label(&g);
    let dead_end = ids["Dead End, near Vending Machine"];
    let maze4 = ids["Maze 4"];
    let maze11 = ids["Maze 11"];

    // `_` — east was typed in the Dead End and there is no passage that way.
    assert_eq!(matrix::classify(&g, dead_end, Direction::E), MatrixCell::Probed);
    assert!(g.is_tried(dead_end, Direction::E), "and it really is on the tried list");
    // `·` — west was never tried there.
    assert_eq!(matrix::classify(&g, dead_end, Direction::W), MatrixCell::Untried);

    // The two reciprocal cells in the whole layer — both ends of one passage.
    assert_eq!(
        matrix::classify(&g, dead_end, Direction::N),
        MatrixCell::Reciprocal { dest: maze4 }
    );
    assert_eq!(
        matrix::classify(&g, maze4, Direction::S),
        MatrixCell::Reciprocal { dest: dead_end }
    );

    // `⇱out` — down from Maze 11 leaves the layer, into a room that has no row here.
    let out = matrix::classify(&g, maze11, Direction::Down);
    let MatrixCell::LeavesLayer { dest } = out else { panic!("expected ⇱out, got {out:?}") };
    assert_eq!(g.room(dest).unwrap().label(), "At West End of Long Hall");
    assert_ne!(g.layer_of(dest), MAZE_LAYER);

    // `→5⇠w` — Maze 1 north reaches Maze 5, which comes back by WEST, not south.
    let n = matrix::classify(&g, ids["Maze 1"], Direction::N);
    assert_eq!(n, MatrixCell::ReturnBy { dest: ids["Maze 5"], back: Direction::W });

    // `⇢9` — Maze 1 south reaches Maze 9 and nothing comes back.
    assert_eq!(
        matrix::classify(&g, ids["Maze 1"], Direction::S),
        MatrixCell::OneWay { dest: ids["Maze 9"] }
    );

    // `→9⇠e` — a passage whose two ends BOTH call themselves east. Only a table can say this.
    assert_eq!(
        matrix::classify(&g, ids["Maze 3"], Direction::E),
        MatrixCell::ReturnBy { dest: ids["Maze 9"], back: Direction::E }
    );
}

#[test]
fn the_here_marker_lands_on_the_room_the_save_was_taken_in() {
    let g = advent();
    let m = matrix::build(&g, MAZE_LAYER);
    let here = m.here.expect("the save was taken standing in the maze");
    assert_eq!(matrix::labels(&g, MAZE_LAYER).row_of(here), "Maze 11");
    assert_eq!(m.index_of(here), Some(11), "and it is the last of the twelve rows");
}

/// Eleven rooms called "Maze" is the problem the numbering solves; the twelfth room keeps its own
/// name and gets initials so cells can point at it without stealing a number.
#[test]
fn eleven_identical_names_become_eleven_distinct_rows() {
    let g = advent();
    let l = matrix::labels(&g, MAZE_LAYER);
    for n in 1..=11 {
        assert!(l.row.values().any(|r| r == &format!("Maze {n}")), "Maze {n} is missing");
    }
    let ids = by_label(&g);
    assert_eq!(l.tag_of(ids["Maze 7"]), "7");
    assert_eq!(l.tag_of(ids["Dead End, near Vending Machine"]), "DE");
    let tags: std::collections::BTreeSet<_> = l.tag.values().collect();
    assert_eq!(tags.len(), 12, "every row is pointable-at by a unique tag");
}

/// The numbers are the vocabulary the player uses out loud ("go back to 7"). They are derived,
/// not stored, so the round trip through the map file is the thing that could break them.
#[test]
fn the_numbering_is_identical_after_a_save_and_load() {
    let g = advent();
    let before = matrix::labels(&g, MAZE_LAYER);
    let mut m = mapper::mapper::Mapper::default();
    m.graph = g;
    let m2 = mapper::persist::from_json(&mapper::persist::to_json(&m)).expect("round trip");
    assert_eq!(matrix::labels(&m2.graph, MAZE_LAYER), before);
}

/// SQ-0685: `advent_maze_map.json` predates the persisted discovery sequence entirely — no room in
/// it carries a `seq` field. On load, [`mapper::graph::MapGraph::from_parts`] backfills each room's
/// seq from its POSITION IN THE FILE'S `rooms` ARRAY (insertion order, i.e. the true historical
/// first-visit order), and numbering follows that. The expectation here is computed straight off
/// the raw file, not hardcoded, so this pins the mechanism rather than today's numbers.
#[test]
fn seq_backfill_numbers_maze_rooms_in_the_files_own_array_order() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/advent_maze_map.json");
    let json = std::fs::read_to_string(&path).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&json).expect("fixture is valid JSON");
    let rooms = raw["rooms"].as_array().expect("rooms is an array");
    assert!(
        rooms.iter().all(|r| r.get("seq").is_none()),
        "fixture predates SQ-0685 and must carry no `seq` field, or this test proves nothing"
    );
    let expected: Vec<RoomId> = rooms
        .iter()
        .filter(|r| r["name"] == "Maze")
        .map(|r| r["id"].as_u64().unwrap() as RoomId)
        .collect();
    assert_eq!(expected.len(), 11, "sanity: eleven Maze rooms in the file");

    let g = advent();
    let lbl = matrix::labels(&g, MAZE_LAYER);
    let mut numbered: Vec<(u32, RoomId)> = lbl
        .row
        .iter()
        .filter_map(|(&id, row)| row.strip_prefix("Maze ").and_then(|n| n.parse().ok()).map(|n| (n, id)))
        .collect();
    numbered.sort();
    let actual: Vec<RoomId> = numbered.into_iter().map(|(_, id)| id).collect();
    assert_eq!(
        actual, expected,
        "numbering must equal the array order of the same-named rooms in the file, not ascending id"
    );
}

/// "How do I get back to Maze 4?" — the set the matrix bolds when a row is selected.
#[test]
fn entrances_to_a_room_are_every_cell_that_arrives_at_it() {
    let g = advent();
    let ids = by_label(&g);
    let maze4 = ids["Maze 4"];
    let e = matrix::entrances(&g, maze4);
    assert_eq!(e.len(), 6, "six ways into Maze 4: {e:?}");
    for want in [
        (ids["Dead End, near Vending Machine"], Direction::N),
        (ids["Maze 3"], Direction::Up),
        (ids["Maze 5"], Direction::S),
        (ids["Maze 6"], Direction::W),
        (ids["Maze 7"], Direction::E),
        (ids["Maze 10"], Direction::N),
    ] {
        assert!(e.contains(&want), "{want:?} is a way in: {e:?}");
    }
    // Every entrance really does classify as pointing back here.
    for (from, dir) in e {
        assert_eq!(matrix::classify(&g, from, dir).dest(), Some(maze4));
    }
}

/// The one door INTO the maze layer in the whole fixture — the mirror of the `⇱out` crossing at
/// Maze 11. It happens to land on the same passage from the other end: "At West End of Long
/// Hall" leads back in via `S`, distinct from the `Down` that leaves.
#[test]
fn inbound_border_edges_finds_the_one_door_into_the_maze() {
    let g = advent();
    let ids = by_label(&g);
    let long_hall = g
        .connections()
        .iter()
        .find(|c| g.layer_of(c.origin) == MAZE_LAYER && g.layer_of(c.dest) != MAZE_LAYER)
        .map(|c| c.dest)
        .expect("the maze has an outbound crossing");
    assert_eq!(g.room(long_hall).unwrap().label(), "At West End of Long Hall");

    let inbound = matrix::inbound_border_edges(&g, MAZE_LAYER);
    assert_eq!(inbound, vec![(long_hall, Direction::S, ids["Maze 11"])], "{inbound:?}");
}
