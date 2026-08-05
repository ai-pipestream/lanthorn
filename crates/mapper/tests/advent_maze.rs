//! The matrix view and tangle detection against REAL player data (SQ-0666).
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

/// Detection has to fire on the maze and stay silent on the ordinary cave beside it — in the
/// SAME map file, explored by the same player to the same partial degree. That is the only
/// comparison that means anything, and it is why the threshold is measured over round trips
/// actually walked rather than over all edges.
#[test]
fn the_maze_is_a_tangle_and_the_cave_beside_it_is_not() {
    let g = advent();

    let maze = mapper::matrix::tangles(&g, MAZE_LAYER);
    assert_eq!(maze.len(), 1, "the maze layer is one tangled cluster: {maze:?}");
    assert_eq!(maze[0].rooms.len(), 12);
    assert!(maze[0].asymmetry() > 0.75, "asymmetry {}", maze[0].asymmetry());

    let main = mapper::matrix::tangles(&g, mapper::layer::MAIN_LAYER);
    assert!(main.is_empty(), "the overworld is not a maze: {main:?}");

    // The naive measure — non-reciprocal share over ALL edges — cannot tell them apart, which is
    // the whole reason the implemented one uses known round trips as its denominator.
    let share = |layer| {
        let e: Vec<_> = g
            .connections()
            .iter()
            .filter(|c| g.layer_of(c.origin) == layer && g.layer_of(c.dest) == layer)
            .collect();
        let recip = e
            .iter()
            .filter(|c| {
                e.iter().any(|b| {
                    b.origin == c.dest
                        && b.dest == c.origin
                        && b.dir == mapper::direction::opposite(c.dir)
                })
            })
            .count();
        1.0 - recip as f32 / e.len() as f32
    };
    assert!(share(MAZE_LAYER) > 0.9 && share(mapper::layer::MAIN_LAYER) > 0.8,
        "both layers look equally 'non-reciprocal' by the naive measure ({} vs {})",
        share(MAZE_LAYER), share(mapper::layer::MAIN_LAYER));
}
