//! The direction matrix: what the map knows about a layer as a TABLE rather than a drawing
//! (SQ-0666).
//!
//! Inside a maze the player's knowledge is not geometry, it is a direction table per room:
//! "west from here goes to that room, and the way back is north". A compass layout of Adventure's
//! all-alike maze marks ~62% of its edges distorted, because compass geometry is not what a maze
//! is. This module turns the graph into the table, leaving every glyph, colour and column width to
//! the renderer — the mapper crate stays pure.
//!
//! Everything here is derived; nothing is stored. Row order, numbering and tags are all functions
//! of the graph, so they survive a save/load round-trip without a single new persisted field.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::{opposite, Direction};
use crate::graph::{MapGraph, RoomId};
use crate::layer::LayerId;

/// The twelve travel directions, in the order the matrix columns them.
///
/// ALL twelve, always — an untried cell in any direction may be exactly what full exploration
/// needs, so none are hidden however empty the column looks. `Unknown` is not among them: it is a
/// bucket for non-compass passages (xyzzy, pray), not a direction you can type at a compass.
pub const MATRIX_DIRS: [Direction; 12] = [
    Direction::N,
    Direction::S,
    Direction::E,
    Direction::W,
    Direction::NE,
    Direction::NW,
    Direction::SE,
    Direction::SW,
    Direction::Up,
    Direction::Down,
    Direction::In,
    Direction::Out,
];

/// What the map knows about ONE room's ONE direction.
///
/// The renderer maps these to glyphs (`⇄` / `→5⇠w` / `⇢9` / `↩` / `⇱out` / `_` / `·`); the
/// distinctions themselves are graph facts and belong here, where they can be tested against a
/// real map without a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCell {
    /// The compass inverse returns: go `dir`, come back by `opposite(dir)`. The rarest cell in a
    /// maze — 2 of 47 edges in the reference map.
    Reciprocal { dest: RoomId },
    /// Leads to `dest`, and the way back is known but is NOT the compass inverse. The row is
    /// self-contained: you can read the round trip without leaving it.
    ReturnBy { dest: RoomId, back: Direction },
    /// Leads to `dest`; no way back is known at all.
    OneWay { dest: RoomId },
    /// Leads back into this very room — the classic "west leads back here".
    SelfLoop,
    /// Leaves the layer. The destination is footnoted rather than tagged, because it has no row
    /// in this table to point at.
    LeavesLayer { dest: RoomId },
    /// Tried, and no path was found. A later OBSERVED loop upgrades this to [`MatrixCell::SelfLoop`];
    /// nothing infers one from the probe alone.
    Probed,
    /// Never tried — the exploration frontier.
    Untried,
}

impl MatrixCell {
    /// The room this cell points at, when it points at one. `None` for the three cells that name
    /// no destination (`SelfLoop` points at the row's own room, so it names nothing new).
    pub fn dest(&self) -> Option<RoomId> {
        match self {
            MatrixCell::Reciprocal { dest }
            | MatrixCell::ReturnBy { dest, .. }
            | MatrixCell::OneWay { dest }
            | MatrixCell::LeavesLayer { dest } => Some(*dest),
            MatrixCell::SelfLoop | MatrixCell::Probed | MatrixCell::Untried => None,
        }
    }

    /// True for the two cells that mark unexplored ground (`_` and `·`) — what the frontier style
    /// dims.
    pub fn is_frontier(&self) -> bool {
        matches!(self, MatrixCell::Probed | MatrixCell::Untried)
    }
}

/// Classify `dir` out of `room`.
///
/// A REAL destination beats a self-loop on the same key: the graph deliberately keeps both when it
/// has seen both (see [`MapGraph::add_edge`]), and a passage that demonstrably leads somewhere is
/// the more useful of the two facts. `↩` therefore means "the only thing this direction ever did
/// was bring me back".
pub fn classify(graph: &MapGraph, room: RoomId, dir: Direction) -> MatrixCell {
    if dir == Direction::Unknown {
        return MatrixCell::Untried;
    }
    let dest = graph
        .connections()
        .iter()
        .find(|c| c.origin == room && c.dir == dir && c.dest != room)
        .map(|c| c.dest);
    let Some(dest) = dest else {
        if graph.self_loops(room).contains(&dir) {
            return MatrixCell::SelfLoop;
        }
        // No edge at all: the room's own record of what has been TYPED here is the only thing
        // that separates a wall from unexplored ground.
        return if graph.is_tried(room, dir) { MatrixCell::Probed } else { MatrixCell::Untried };
    };
    if graph.layer_of(dest) != graph.layer_of(room) {
        return MatrixCell::LeavesLayer { dest };
    }
    if graph.connections().iter().any(|c| c.origin == dest && c.dir == opposite(dir) && c.dest == room)
    {
        return MatrixCell::Reciprocal { dest };
    }
    // Any other direction that comes back. Scanned in column order so the answer is stable
    // whatever order the edges were minted in.
    for back in MATRIX_DIRS {
        if graph.connections().iter().any(|c| c.origin == dest && c.dir == back && c.dest == room) {
            return MatrixCell::ReturnBy { dest, back };
        }
    }
    MatrixCell::OneWay { dest }
}

/// Every `(room, direction)` in the graph whose passage ARRIVES at `target` — the answer to "how
/// do I get back here", and the set the matrix bolds when a row is selected.
///
/// Directed, and deliberately not filtered by layer: a way in from outside the layer is still a
/// way in, and the caller decides which of these it can actually draw.
pub fn entrances(graph: &MapGraph, target: RoomId) -> Vec<(RoomId, Direction)> {
    graph
        .connections()
        .iter()
        .filter(|c| c.dest == target && c.dir != Direction::Unknown)
        .map(|c| (c.origin, c.dir))
        .collect()
}

// ── Display naming ────────────────────────────────────────────────────────────

/// Row labels and cell tags for a layer's rooms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatrixLabels {
    /// What the label column spells out: `"Maze 3"`, `"Dead End, near Vending Machine"`.
    pub row: BTreeMap<RoomId, String>,
    /// The short reference a destination cell prints: `"3"`, `"DE"`. At most three characters, so
    /// a cell stays inside its column.
    pub tag: BTreeMap<RoomId, String>,
}

impl MatrixLabels {
    pub fn row_of(&self, id: RoomId) -> &str {
        self.row.get(&id).map(String::as_str).unwrap_or("")
    }
    pub fn tag_of(&self, id: RoomId) -> &str {
        self.tag.get(&id).map(String::as_str).unwrap_or("")
    }
}

/// Initials for a room name, upper case, at most three: `"Dead End, near Vending Machine"` → `DE`.
///
/// Only the part before the first comma is used (the tail of an IF room name is nearly always a
/// qualifier), and only capitalised words contribute, which drops the `of`/`the`/`near` filler
/// without a stop-word list.
fn initials(name: &str) -> String {
    let head = name.split(',').next().unwrap_or(name);
    let mut out = String::new();
    for w in head.split_whitespace() {
        let Some(c) = w.chars().next() else { continue };
        if !c.is_uppercase() {
            continue;
        }
        out.push(c);
        if out.chars().count() == 3 {
            break;
        }
    }
    if out.is_empty() {
        out = head.chars().filter(|c| c.is_alphanumeric()).take(3).collect::<String>().to_uppercase();
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

/// Number the rooms of `layer` for display.
///
/// Rooms that SHARE a display name are numbered in row order — eleven rooms called "Maze" become
/// "Maze 1".."Maze 11" — because eleven identical rows is exactly the problem the matrix exists to
/// solve. The numbering is display-only: identity stays the room id, which never changes, so the
/// numbers are the same after a save/load and a room never renumbers under the player.
///
/// A uniquely-named room gets its initials instead of a number, so cells can point at it without
/// stealing a number from the numbered group.
pub fn labels(graph: &MapGraph, layer: LayerId) -> MatrixLabels {
    let ids = graph.rooms_in_layer(layer);
    let name_of = |id: RoomId| graph.room(id).map(|r| r.label().to_string()).unwrap_or_default();

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for &id in &ids {
        *counts.entry(name_of(id)).or_insert(0) += 1;
    }
    // When only ONE name repeats, its numbers are unambiguous on their own ("5" can only be
    // Maze 5). With two repeating names the numbers would collide, so both carry initials.
    let repeating = counts.values().filter(|&&n| n > 1).count();

    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = MatrixLabels::default();
    for &id in &ids {
        let name = name_of(id);
        let (row, tag) = if counts.get(&name).copied().unwrap_or(0) > 1 {
            let n = seen.entry(name.clone()).and_modify(|v| *v += 1).or_insert(1);
            let tag = if repeating == 1 {
                n.to_string()
            } else {
                format!("{}{}", initials(&name), n)
            };
            (format!("{name} {n}"), tag)
        } else {
            (name.clone(), initials(&name))
        };
        out.row.insert(id, row);
        out.tag.insert(id, tag);
    }

    // Force tags unique: two differently-named rooms can still share initials. The LATER row
    // yields, so adding a room never renames one the player has already learned.
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for &id in &ids {
        let base = out.tag.get(&id).cloned().unwrap_or_default();
        if taken.insert(base.clone()) {
            continue;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{base}{n}");
            if taken.insert(candidate.clone()) {
                out.tag.insert(id, candidate);
                break;
            }
            n += 1;
        }
    }
    out
}

// ── The table ─────────────────────────────────────────────────────────────────

/// One room's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRow {
    pub room: RoomId,
    /// The display label, already numbered (`"Maze 3"`).
    pub label: String,
    /// The short reference other rows' cells use for this room (`"3"`).
    pub tag: String,
    /// One cell per [`MATRIX_DIRS`] entry, same order.
    pub cells: [MatrixCell; 12],
}

/// A layer as a direction table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    pub layer: LayerId,
    pub rows: Vec<MatrixRow>,
    pub labels: MatrixLabels,
    /// The room the player is standing in, when it is in this layer.
    pub here: Option<RoomId>,
}

impl Matrix {
    /// The row index of `room`, if it has one.
    pub fn index_of(&self, room: RoomId) -> Option<usize> {
        self.rows.iter().position(|r| r.room == room)
    }
}

/// Build the direction table for `layer`.
///
/// Rows are in the graph's stable room order (ascending room id). That order is a deliberate
/// stand-in for visit order: the map persists no visit counter, and a number that shuffled when a
/// map was reloaded would be worse than useless on a table whose whole job is to let the player
/// say "the one I called 7".
pub fn build(graph: &MapGraph, layer: LayerId) -> Matrix {
    let labels = labels(graph, layer);
    let rows = graph
        .rooms_in_layer(layer)
        .into_iter()
        .map(|id| MatrixRow {
            room: id,
            label: labels.row_of(id).to_string(),
            tag: labels.tag_of(id).to_string(),
            cells: MATRIX_DIRS.map(|d| classify(graph, id, d)),
        })
        .collect();
    let here = graph.current().filter(|&id| graph.layer_of(id) == layer);
    Matrix { layer, rows, labels, here }
}

// ── Tangle detection ──────────────────────────────────────────────────────────

/// A connected cluster of rooms whose passages mostly do NOT come back the way they went — the
/// shape of a maze (SQ-0666).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tangle {
    pub layer: LayerId,
    pub rooms: BTreeSet<RoomId>,
    /// In-cluster passages whose return trip is known at all.
    pub known_returns: usize,
    /// How many of those return by some direction OTHER than the compass inverse.
    pub asymmetric: usize,
}

impl Tangle {
    /// The share of known round trips that disagreed with the compass, in `0.0..=1.0`.
    pub fn asymmetry(&self) -> f32 {
        if self.known_returns == 0 {
            return 0.0;
        }
        self.asymmetric as f32 / self.known_returns as f32
    }
}

/// The share of KNOWN round trips that must disagree with the compass before a cluster is called a
/// tangle.
///
/// Measured against the reference map (a real, partial mapping of Colossal Cave): the all-alike
/// maze scores 0.90 (18 of 20 round trips return by another direction) and the ordinary cave
/// overworld beside it scores 0.56. 0.75 sits with clear air on both sides.
///
/// Note the denominator. "Non-reciprocal share over ALL edges" — the obvious reading — does not
/// separate the two at all: it is 0.96 for the maze and 0.82 for the overworld, because a passage
/// nobody has walked BACK through yet says nothing about geometry, only about how far exploration
/// has got. Only passages walked both ways are evidence.
pub const TANGLE_ASYMMETRY: f32 = 0.75;

/// A cluster smaller than this is not worth offering to peel — two rooms that disagree are a
/// quirk, not a maze.
pub const TANGLE_MIN_ROOMS: usize = 6;

/// Below this many known round trips the ratio is noise: three out of three proves nothing.
pub const TANGLE_MIN_RETURNS: usize = 8;

/// Clusters within `layer` that look like mazes, using the shipped thresholds.
pub fn tangles(graph: &MapGraph, layer: LayerId) -> Vec<Tangle> {
    tangles_with(graph, layer, TANGLE_ASYMMETRY, TANGLE_MIN_ROOMS, TANGLE_MIN_RETURNS)
}

/// [`tangles`] with the thresholds spelled out — the form the tests pin.
pub fn tangles_with(
    graph: &MapGraph,
    layer: LayerId,
    min_asymmetry: f32,
    min_rooms: usize,
    min_returns: usize,
) -> Vec<Tangle> {
    let ids = graph.rooms_in_layer(layer);
    let in_layer: BTreeSet<RoomId> = ids.iter().copied().collect();

    // Connected clusters over in-layer passages, treated as undirected. Self-loops and Unknown
    // edges connect nothing.
    let mut adjacency: BTreeMap<RoomId, Vec<RoomId>> = ids.iter().map(|&i| (i, Vec::new())).collect();
    for c in graph.connections() {
        if c.is_self_loop()
            || c.dir == Direction::Unknown
            || !in_layer.contains(&c.origin)
            || !in_layer.contains(&c.dest)
        {
            continue;
        }
        adjacency.entry(c.origin).or_default().push(c.dest);
        adjacency.entry(c.dest).or_default().push(c.origin);
    }

    let mut seen: BTreeSet<RoomId> = BTreeSet::new();
    let mut out = Vec::new();
    for &start in &ids {
        if !seen.insert(start) {
            continue;
        }
        let mut cluster = BTreeSet::new();
        cluster.insert(start);
        let mut q = VecDeque::from([start]);
        while let Some(cur) = q.pop_front() {
            for &next in adjacency.get(&cur).map(Vec::as_slice).unwrap_or(&[]) {
                if seen.insert(next) {
                    cluster.insert(next);
                    q.push_back(next);
                }
            }
        }

        let mut known_returns = 0usize;
        let mut asymmetric = 0usize;
        for c in graph.connections() {
            if c.is_self_loop()
                || c.dir == Direction::Unknown
                || !cluster.contains(&c.origin)
                || !cluster.contains(&c.dest)
            {
                continue;
            }
            let returns = graph
                .connections()
                .iter()
                .any(|b| b.origin == c.dest && b.dest == c.origin && b.dir != Direction::Unknown);
            if !returns {
                continue; // no round trip walked yet — no evidence either way
            }
            known_returns += 1;
            let inverse = graph
                .connections()
                .iter()
                .any(|b| b.origin == c.dest && b.dir == opposite(c.dir) && b.dest == c.origin);
            if !inverse {
                asymmetric += 1;
            }
        }

        let tangle = Tangle { layer, rooms: cluster, known_returns, asymmetric };
        if tangle.rooms.len() >= min_rooms
            && tangle.known_returns >= min_returns
            && tangle.asymmetry() >= min_asymmetry
        {
            out.push(tangle);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::MAIN_LAYER;

    /// A miniature of the reference map: a numbered group, one uniquely-named room, and one edge
    /// out of the layer.
    fn maze() -> (MapGraph, LayerId) {
        let mut g = MapGraph::new();
        for (id, n) in [
            (1u16, "Maze"),
            (2, "Maze"),
            (3, "Maze"),
            (4, "Dead End, near Vending Machine"),
            (9, "At West End of Long Hall"),
        ] {
            g.upsert_room(id, n.into());
        }
        let l = g.new_layer(Some(MAIN_LAYER), "Maze".into());
        for id in [1, 2, 3, 4] {
            g.set_room_layer(id, l);
        }
        g.add_edge(1, Direction::N, 2); // 2 returns by W, not S
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::E, 3); // no return known
        g.add_edge(3, Direction::S, 4); // reciprocal pair
        g.add_edge(4, Direction::N, 3);
        g.add_edge(2, Direction::Down, 9); // leaves the layer
        g.mark_tried(4, Direction::E); // tried east, hit a wall
        (g, l)
    }

    #[test]
    fn every_cell_of_the_vocabulary_classifies() {
        let (mut g, _l) = maze();
        assert_eq!(classify(&g, 1, Direction::N), MatrixCell::ReturnBy { dest: 2, back: Direction::W });
        assert_eq!(classify(&g, 1, Direction::E), MatrixCell::OneWay { dest: 3 });
        assert_eq!(classify(&g, 3, Direction::S), MatrixCell::Reciprocal { dest: 4 });
        assert_eq!(classify(&g, 4, Direction::N), MatrixCell::Reciprocal { dest: 3 });
        assert_eq!(classify(&g, 2, Direction::Down), MatrixCell::LeavesLayer { dest: 9 });
        assert_eq!(classify(&g, 4, Direction::E), MatrixCell::Probed, "tried east, no path");
        assert_eq!(classify(&g, 4, Direction::W), MatrixCell::Untried, "never tried west");
        assert_eq!(classify(&g, 1, Direction::Unknown), MatrixCell::Untried, "no column for `?`");

        // An observed loop upgrades a probe.
        assert!(g.add_self_loop(4, Direction::E));
        assert_eq!(classify(&g, 4, Direction::E), MatrixCell::SelfLoop, "the probe became a loop");

        // …but a real destination on the same key still wins: the loop is the fallback fact.
        g.add_edge(4, Direction::E, 1);
        assert_eq!(classify(&g, 4, Direction::E), MatrixCell::OneWay { dest: 1 });
        assert!(g.self_loops(4).contains(&Direction::E), "and the loop is not destroyed");
    }

    #[test]
    fn same_named_rooms_number_in_row_order_and_unique_names_get_initials() {
        let (g, l) = maze();
        let lbl = labels(&g, l);
        assert_eq!(lbl.row_of(1), "Maze 1");
        assert_eq!(lbl.row_of(3), "Maze 3");
        assert_eq!(lbl.tag_of(3), "3", "one repeating name → bare numbers");
        assert_eq!(lbl.row_of(4), "Dead End, near Vending Machine");
        assert_eq!(lbl.tag_of(4), "DE", "initials of the part before the comma, filler dropped");
    }

    /// The numbering is display-only and derived, so it must be identical after a round trip
    /// through the map file — a player who calls a room "7" must still find 7 tomorrow.
    #[test]
    fn numbering_survives_save_and_load() {
        let (g, l) = maze();
        let before = labels(&g, l);
        let m = crate::mapper::Mapper { graph: g, ..Default::default() };
        let json = crate::persist::to_json(&m);
        let m2 = crate::persist::from_json(&json).expect("round trip");
        assert_eq!(labels(&m2.graph, l), before);
    }

    #[test]
    fn two_repeating_names_disambiguate_their_numbers() {
        let mut g = MapGraph::new();
        for (id, n) in [(1u16, "Maze"), (2, "Maze"), (3, "Cave"), (4, "Cave")] {
            g.upsert_room(id, n.into());
        }
        let lbl = labels(&g, MAIN_LAYER);
        assert_eq!(lbl.tag_of(2), "M2");
        assert_eq!(lbl.tag_of(4), "C2", "a bare `2` would name two different rooms");
        assert_ne!(lbl.tag_of(2), lbl.tag_of(4));
    }

    #[test]
    fn colliding_initials_are_forced_apart_and_the_earlier_row_keeps_its_tag() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "Dark Room".into());
        g.upsert_room(2, "Damp Ruin".into()); // also "DR"
        let lbl = labels(&g, MAIN_LAYER);
        assert_eq!(lbl.tag_of(1), "DR", "the earlier row never renames");
        assert_eq!(lbl.tag_of(2), "DR2");
    }

    #[test]
    fn build_lays_out_twelve_columns_per_room_and_marks_here() {
        let (mut g, l) = maze();
        g.set_current(2);
        let m = build(&g, l);
        assert_eq!(m.rows.len(), 4, "only the layer's rooms");
        assert_eq!(m.rows[0].cells.len(), 12);
        assert_eq!(m.here, Some(2));
        assert_eq!(m.index_of(4), Some(3));
        // Nothing outside the layer gets a row, even though room 9 is a destination.
        assert!(m.rows.iter().all(|r| r.room != 9));

        g.set_current(9); // stand outside the layer
        assert_eq!(build(&g, l).here, None, "the here-marker belongs to the layer you are in");
    }

    #[test]
    fn entrances_answer_how_do_i_get_back_here() {
        let (g, _) = maze();
        let e = entrances(&g, 3);
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e.contains(&(1, Direction::E)));
        assert!(e.contains(&(4, Direction::N)));
        assert!(entrances(&g, 9).contains(&(2, Direction::Down)), "a way in from another layer counts");
    }

    // ── Tangle detection ──────────────────────────────────────────────────────

    /// Build a cluster of `rooms` rooms wired so that `asym` of its round trips return by a
    /// direction other than the inverse and the rest are reciprocal.
    fn ring(rooms: usize, asym: usize) -> MapGraph {
        let mut g = MapGraph::new();
        for i in 0..rooms {
            g.upsert_room(i as u16 + 1, format!("R{i}"));
        }
        for i in 0..rooms {
            let a = i as u16 + 1;
            let b = (i as u16 + 1) % rooms as u16 + 1;
            g.add_edge(a, Direction::E, b);
            // The return: W is the inverse (reciprocal); N is not (asymmetric).
            g.add_edge(b, if i < asym { Direction::N } else { Direction::W }, a);
        }
        g
    }

    #[test]
    fn a_tangle_needs_a_high_asymmetry_a_real_size_and_a_real_sample() {
        // 10 rooms, 9 of 10 round trips disagree with the compass → 0.9 ≥ 0.75.
        let g = ring(10, 9);
        let t = tangles(&g, MAIN_LAYER);
        assert_eq!(t.len(), 1, "the ring is one cluster and it is a tangle: {t:?}");
        assert_eq!(t[0].known_returns, 20, "each passage is counted from both ends");
        assert!(t[0].asymmetry() > 0.75);

        // Same size, mostly reciprocal → not a tangle.
        assert!(tangles(&ring(10, 3), MAIN_LAYER).is_empty(), "0.3 asymmetry is an ordinary map");

        // Wholly asymmetric but too small to be worth peeling.
        assert!(tangles(&ring(4, 4), MAIN_LAYER).is_empty(), "4 rooms is a quirk, not a maze");

        // Big and asymmetric, but almost nothing has been walked BOTH ways: no evidence yet.
        let mut sparse = MapGraph::new();
        for i in 1u16..=10 {
            sparse.upsert_room(i, format!("R{i}"));
        }
        for i in 1u16..10 {
            sparse.add_edge(i, Direction::E, i + 1); // one-way only: never a known return
        }
        sparse.add_edge(2, Direction::N, 1);
        assert!(
            tangles(&sparse, MAIN_LAYER).is_empty(),
            "one round trip is not a sample — an unexplored map is not a maze"
        );
    }

    /// The threshold must sit between the two real numbers it was calibrated on, or it is just a
    /// number: 0.90 for the reference maze, 0.56 for the ordinary cave layer beside it.
    #[test]
    fn the_threshold_separates_the_two_real_layers_it_was_calibrated_on() {
        // Measured off `unit_tests/advent_maze_map.json`; see `TANGLE_ASYMMETRY`'s doc and
        // `tests/advent_maze.rs`, which recomputes both from the file itself.
        let (reference_maze, reference_overworld) = (0.90f32, 0.56f32);
        assert!(TANGLE_ASYMMETRY < reference_maze, "the reference maze must clear it");
        assert!(TANGLE_ASYMMETRY > reference_overworld, "the reference overworld must not");
    }

    #[test]
    fn detection_is_per_cluster_not_per_layer() {
        // One layer holding a tangle AND a tidy region: only the tangle is reported.
        let mut g = ring(10, 9);
        for i in 100u16..110 {
            g.upsert_room(i, format!("Tidy{i}"));
        }
        for i in 100u16..109 {
            g.add_edge(i, Direction::E, i + 1);
            g.add_edge(i + 1, Direction::W, i);
        }
        let t = tangles(&g, MAIN_LAYER);
        assert_eq!(t.len(), 1);
        assert!(t[0].rooms.contains(&1) && !t[0].rooms.contains(&100));
    }
}
