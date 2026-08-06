//! Tangle detection against a maze EMBEDDED in ordinary geography (SQ-0683).
//!
//! `unit_tests/zork1_maze_map.json` is a verbatim copy of the `map.json` inside a babelmap archive:
//! one player's partial mapping of Zork I, mid-game, standing in the maze. Nothing here is
//! hand-peeled — the maze shares its layer ("Cellar") with the Cellar, the Troll Room, the Gallery,
//! the Studio and East of Chasm, which is the state the `/mark-maze-layer` offer exists to notice.
//! That is exactly what the shipped detector could not see: it clustered by whole connected
//! component, and the component scores 0.45 while the maze inside it scores 0.83.
//!
//! Every number asserted here is measured off the file, and the ones the thresholds were calibrated
//! against are recomputed from it rather than quoted.

use std::collections::{BTreeMap, BTreeSet};

use mapper::direction::{opposite, Direction};
use mapper::graph::{MapGraph, RoomId};
use mapper::layer::{LayerId, MAIN_LAYER};
use mapper::matrix::{self, TANGLE_ASYMMETRY, TANGLE_MIN_RETURNS, TANGLE_MIN_ROOMS};

/// The layer holding the Cellar region AND the maze — the layer the player never split.
const CELLAR_LAYER: LayerId = 2;

fn zork1() -> MapGraph {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../unit_tests/zork1_maze_map.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} must be readable: {e}", path.display()));
    mapper::persist::from_json(&json).expect("the fixture is a valid map file").graph
}

/// Room id by row label, e.g. `"Maze 3"`.
fn by_label(g: &MapGraph, layer: LayerId) -> BTreeMap<String, RoomId> {
    matrix::labels(g, layer).row.into_iter().map(|(id, l)| (l, id)).collect()
}

fn names(g: &MapGraph, rooms: &BTreeSet<RoomId>) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for &id in rooms {
        *out.entry(g.room(id).unwrap().label().to_string()).or_insert(0) += 1;
    }
    out
}

/// Round trips among `rooms`, counted from BOTH ends, and how many disagreed with the compass —
/// the detector's two statistics, recomputed here straight off the graph so the numbers the
/// thresholds are calibrated on are never just quoted from the implementation.
fn stats(g: &MapGraph, rooms: &BTreeSet<RoomId>) -> (usize, usize) {
    let (mut known, mut asym) = (0, 0);
    for c in g.connections() {
        if c.is_self_loop()
            || c.dir == Direction::Unknown
            || !rooms.contains(&c.origin)
            || !rooms.contains(&c.dest)
        {
            continue;
        }
        let back = |dir: Option<Direction>| {
            g.connections().iter().any(|b| {
                b.origin == c.dest
                    && b.dest == c.origin
                    && b.dir != Direction::Unknown
                    && dir.is_none_or(|d| b.dir == d)
            })
        };
        if !back(None) {
            continue;
        }
        known += 1;
        if !back(Some(opposite(c.dir))) {
            asym += 1;
        }
    }
    (known, asym)
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
    assert!(!g.layer_is_maze(CELLAR_LAYER), "and never marked it a maze — that is the offer's job");

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

/// The bug, as a number. The Cellar layer is ONE connected component — the maze hangs off the
/// Troll Room, which reciprocates tidily with the Cellar — so clustering by component averages
/// seven maze rooms together with five ordinary ones and lands under the bar. This is the
/// measurement that says a component is the wrong unit, not that the threshold is wrong.
#[test]
fn the_whole_component_scores_too_low_to_fire_which_is_why_the_unit_is_not_the_component() {
    let g = zork1();
    let component: BTreeSet<RoomId> = g.rooms_in_layer(CELLAR_LAYER).into_iter().collect();

    // One component: every room is reachable from the Cellar over in-layer passages.
    let mut reached = BTreeSet::from([by_label(&g, CELLAR_LAYER)["Cellar"]]);
    loop {
        let grown: BTreeSet<RoomId> = g
            .connections()
            .iter()
            .filter(|c| component.contains(&c.origin) && component.contains(&c.dest))
            .flat_map(|c| {
                [(c.origin, c.dest), (c.dest, c.origin)]
                    .into_iter()
                    .filter(|(a, _)| reached.contains(a))
                    .map(|(_, b)| b)
                    .collect::<Vec<_>>()
            })
            .collect();
        let before = reached.len();
        reached.extend(grown);
        if reached.len() == before {
            break;
        }
    }
    assert_eq!(reached, component, "the maze is not a component of its own — that is the problem");

    let (known, asym) = stats(&g, &component);
    assert_eq!((known, asym), (22, 10), "the component's own numbers");
    let ratio = asym as f32 / known as f32;
    assert!(
        ratio < TANGLE_ASYMMETRY,
        "0.45 is under the {TANGLE_ASYMMETRY} bar, so component clustering can never fire here \
         ({ratio})"
    );
}

/// The fix, as a number: growing the cluster through asymmetric round trips finds the maze inside
/// the layer, and it clears the bar with room to spare.
#[test]
fn the_maze_inside_the_cellar_layer_is_a_tangle() {
    let g = zork1();
    let t = matrix::tangles(&g, CELLAR_LAYER);
    assert_eq!(t.len(), 1, "one tangle on the layer: {t:?}");
    let t = &t[0];

    assert_eq!(t.rooms.len(), 7, "five Maze rooms and both Dead Ends: {:?}", names(&g, &t.rooms));
    assert_eq!(names(&g, &t.rooms), BTreeMap::from([("Maze".into(), 5), ("Dead End".into(), 2)]));
    let ids = by_label(&g, CELLAR_LAYER);
    for tidy in ["Cellar", "The Troll Room", "Gallery", "Studio", "East of Chasm"] {
        assert!(!t.rooms.contains(&ids[tidy]), "{tidy} is ordinary geography, not maze");
    }

    assert_eq!((t.known_returns, t.asymmetric), (12, 10), "counted from both ends");
    assert_eq!(stats(&g, &t.rooms), (t.known_returns, t.asymmetric), "and recomputed off the graph");
    assert!((t.asymmetry() - 0.833).abs() < 0.005, "asymmetry {}", t.asymmetry());
    assert!(t.asymmetry() >= TANGLE_ASYMMETRY && t.known_returns >= TANGLE_MIN_RETURNS);

    // The player is standing in it, which is what the /mark-maze-layer offer keys on.
    assert!(t.rooms.contains(&g.current().unwrap()));
}

/// The absorbed room. "Dead End 1" reciprocates perfectly with the maze room it hangs off — go
/// east, come back west — so no asymmetric evidence ever links it. It is still a maze room, and a
/// peel that left it behind would strand it; the cluster takes it because every passage it has
/// leads back into the maze. Its two reciprocal ends stay in the denominator, which is why the
/// tangle scores 0.83 rather than the seed's 1.00.
#[test]
fn a_dead_end_that_reciprocates_is_still_absorbed_into_the_maze() {
    let g = zork1();
    let ids = by_label(&g, CELLAR_LAYER);
    let (dead_end, maze5) = (ids["Dead End 1"], ids["Maze 5"]);
    assert_eq!(matrix::classify(&g, dead_end, Direction::W), matrix::MatrixCell::Reciprocal { dest: maze5 });
    assert_eq!(matrix::classify(&g, maze5, Direction::E), matrix::MatrixCell::Reciprocal { dest: dead_end });
    assert_eq!(stats(&g, &BTreeSet::from([dead_end, maze5])), (2, 0), "no asymmetric evidence at all");

    let t = &matrix::tangles(&g, CELLAR_LAYER)[0];
    assert!(t.rooms.contains(&dead_end), "absorbed anyway: it has nowhere else to go");
    let seed: BTreeSet<RoomId> = t.rooms.iter().copied().filter(|&r| r != dead_end).collect();
    assert_eq!(stats(&g, &seed), (10, 10), "the seed alone is wholly asymmetric");

    // The Troll Room also reciprocates with a maze room, but it has a foot in the tidy region
    // (the Cellar), so absorption stops there — it is the door to the maze, not part of it.
    let troll = ids["The Troll Room"];
    assert_eq!(
        matrix::classify(&g, troll, Direction::W),
        matrix::MatrixCell::Reciprocal { dest: ids["Maze 3"] },
        "west into the maze, east back out — the compass agrees, so no evidence links them"
    );
    assert!(!t.rooms.contains(&troll), "absorbing it would drag the Cellar in behind it");
    let with_troll: BTreeSet<RoomId> = t.rooms.iter().copied().chain([troll]).collect();
    let (known, asym) = stats(&g, &with_troll);
    assert!(
        (asym as f32 / known as f32) < TANGLE_ASYMMETRY,
        "and would sink the score below the bar ({asym}/{known})"
    );
}

/// The overworld must stay silent — same map file, same player, same partial exploration. The
/// interesting part is WHY: the ring of rooms around the white house is wholly asymmetric real
/// geography (north from West of House arrives North of House, and the way back is west), so
/// asymmetry alone cannot disqualify it. Only its size does, at 4 rooms against a bar of 6.
#[test]
fn the_overworld_is_not_a_maze_and_the_house_ring_is_the_closest_it_comes() {
    let g = zork1();
    assert!(matrix::tangles(&g, MAIN_LAYER).is_empty(), "the overworld does not fire");

    // Drop only the size bar and the near-miss appears, so the margin is measured, not assumed.
    let near = matrix::tangles_with(&g, MAIN_LAYER, TANGLE_ASYMMETRY, 2, TANGLE_MIN_RETURNS);
    assert_eq!(near.len(), 1, "one cluster clears asymmetry and sample size: {near:?}");
    assert_eq!(
        names(&g, &near[0].rooms).keys().cloned().collect::<Vec<_>>(),
        ["Behind House", "North of House", "South of House", "West of House"],
    );
    assert_eq!((near[0].known_returns, near[0].asymmetric), (8, 8), "1.00 — as tangled as it gets");
    assert!(near[0].rooms.len() < TANGLE_MIN_ROOMS, "4 rooms against a bar of {TANGLE_MIN_ROOMS}");

    // Nothing on the layer grows to maze size on evidence, however far the ratio bar is lowered.
    let any = matrix::tangles_with(&g, MAIN_LAYER, 0.0, 2, 0);
    assert!(
        any.iter().all(|t| t.rooms.len() < TANGLE_MIN_ROOMS),
        "the overworld's asymmetric links do not chain into a maze-sized cluster: {:?}",
        any.iter().map(|t| t.rooms.len()).collect::<Vec<_>>()
    );
}
