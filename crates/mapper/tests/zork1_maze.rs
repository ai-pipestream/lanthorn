//! Matrix labelling against a maze EMBEDDED in ordinary geography (SQ-0683, SQ-0685).
//!
//! `unit_tests/zork1_maze_map.json` is a verbatim copy of the `map.json` inside a babelmap archive:
//! one player's partial mapping of Zork I, mid-game, standing in the maze. Nothing here is
//! hand-peeled — the maze shares its layer ("Cellar") with the Cellar, the Troll Room, the Gallery,
//! the Studio and East of Chasm, and the player never flagged it, because flagging a maze is a
//! manual act (`/mark-maze-layer`).
//!
//! Every number asserted here is measured off the file rather than quoted.

use std::collections::{BTreeMap, BTreeSet};

use mapper::graph::{MapGraph, RoomId};
use mapper::layer::{LayerId, MAIN_LAYER};
use mapper::matrix;

/// The layer holding the Cellar region AND the maze — the layer the player never split.
const CELLAR_LAYER: LayerId = 2;

fn zork1() -> MapGraph {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/zork1_maze_map.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file").graph
}

fn names(g: &MapGraph, rooms: &BTreeSet<RoomId>) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for &id in rooms {
        *out.entry(g.room(id).unwrap().label().to_string()).or_insert(0) += 1;
    }
    out
}

/// The shape of the fixture, pinned first and loudly: if it is ever replaced these change and
/// every other assertion here is suspect.
#[test]
fn the_fixture_is_a_maze_sharing_a_layer_with_ordinary_rooms() {
    let g = zork1();
    assert_eq!(g.rooms_in_layer(MAIN_LAYER).len(), 19, "the above-ground map");
    let cellar = g.rooms_in_layer(CELLAR_LAYER);
    assert_eq!(cellar.len(), 12, "the Cellar layer");
    assert_eq!(
        g.layers().get(&CELLAR_LAYER).map(|l| l.name.as_str()),
        Some("Cellar"),
        "the player named it for the room they peeled it around, not for the maze"
    );
    assert!(!g.layer_is_maze(CELLAR_LAYER), "and never marked it a maze — that is done by hand");

    let by_name = names(&g, &cellar.iter().copied().collect());
    assert_eq!(by_name.get("Maze"), Some(&5));
    assert_eq!(by_name.get("Dead End"), Some(&2));
    for tidy in ["Cellar", "The Troll Room", "Gallery", "Studio", "East of Chasm"] {
        assert_eq!(by_name.get(tidy), Some(&1), "{tidy} shares the layer with the maze");
    }

    let here = g.current().expect("the save was taken standing somewhere");
    assert_eq!(g.layer_of(here), CELLAR_LAYER, "and it was in the maze");
    assert_eq!(matrix::labels(&g, CELLAR_LAYER).row_of(here), "Maze 3");
}

/// SQ-0685: `zork1_maze_map.json` predates the persisted discovery sequence too — no room in it
/// carries a `seq`. On load the backfill assigns each room's seq from its POSITION IN THE FILE'S
/// `rooms` ARRAY, and numbering follows that array order rather than ascending room id. Computed
/// straight off the raw file so this pins the mechanism, not today's numbers.
#[test]
fn seq_backfill_numbers_maze_rooms_in_the_files_own_array_order() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/zork1_maze_map.json");
    let json = std::fs::read_to_string(&path).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&json).expect("fixture is valid JSON");
    let rooms = raw["rooms"].as_array().expect("rooms is an array");
    assert!(
        rooms.iter().all(|r| r.get("seq").is_none()),
        "fixture predates SQ-0685 and must carry no `seq` field, or this test proves nothing"
    );
    let expected: Vec<RoomId> = rooms
        .iter()
        .filter(|r| r["name"] == "Maze" && r["layer"].as_u64() == Some(CELLAR_LAYER as u64))
        .map(|r| r["id"].as_u64().unwrap() as RoomId)
        .collect();
    assert_eq!(expected.len(), 5, "sanity: five Maze rooms in the file, all on the Cellar layer");

    let g = zork1();
    let lbl = matrix::labels(&g, CELLAR_LAYER);
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
