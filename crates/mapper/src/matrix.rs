//! The direction matrix: what the map knows about a layer as a TABLE rather than a drawing
//! (SQ-0666).
//!
//! Inside a maze the player's knowledge is not geometry, it is a direction table per room:
//! "west from here goes to that room, and the way back is north". A compass layout of Adventure's
//! all-alike maze marks ~62% of its edges distorted, because compass geometry is not what a maze
//! is. This module turns the graph into the table, leaving every glyph, colour and column width to
//! the renderer — the mapper crate stays pure.
//!
//! Everything here is derived — row order, numbering and tags are all functions of the graph, so
//! nothing in this module itself needs a save format — but numbering does lean on one small fact
//! the graph persists: each room's discovery sequence (SQ-0685), stamped once at first upsert. A
//! room's *id* is not usable for this — for a Z-machine game it is the story's own object number,
//! unrelated to when the player found the room — so without a persisted visit order, numbering
//! that used id order instead would renumber every "Maze" room behind a newly-found low-id one.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

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

/// Border edges that ENTER `layer` from outside it — the doors a player could walk in through.
///
/// The mirror image of the `⇱out` cells [`classify`] already produces from the other side: there
/// the test is `layer_of(dest) != layer_of(room)` with `room` inside the layer; here it is
/// `layer_of(origin) != layer` with `dest` inside it. `⇱out` cells live in the table itself (the
/// origin has a row to sit on); an inbound edge's origin has no row here at all, so it can only
/// ever be a footnote — this is the query that footnote is built from.
///
/// Ordered by the entering room's row position ([`build`]'s row order — discovery-sequence order,
/// SQ-0685, not room id), then by [`MATRIX_DIRS`] column order, then by origin id: deterministic
/// however the edges were minted, so the block a caller builds from this is stable across a
/// save/load round trip.
pub fn inbound_border_edges(graph: &MapGraph, layer: LayerId) -> Vec<(RoomId, Direction, RoomId)> {
    let row_of: BTreeMap<RoomId, usize> =
        rooms_by_seq(graph, layer).into_iter().enumerate().map(|(i, id)| (id, i)).collect();
    let col_of = |d: Direction| MATRIX_DIRS.iter().position(|&x| x == d).unwrap_or(usize::MAX);

    let mut out: Vec<(RoomId, Direction, RoomId)> = graph
        .connections()
        .iter()
        .filter(|c| {
            c.dir != Direction::Unknown
                && graph.layer_of(c.dest) == layer
                && graph.layer_of(c.origin) != layer
        })
        .map(|c| (c.origin, c.dir, c.dest))
        .collect();
    out.sort_by_key(|&(origin, dir, dest)| {
        (row_of.get(&dest).copied().unwrap_or(usize::MAX), col_of(dir), origin)
    });
    out
}

/// `layer`'s rooms in DISCOVERY order (SQ-0685): ascending [`crate::graph::Room::seq`], the true
/// first-visit order, rather than [`MapGraph::rooms_in_layer`]'s ascending room id — for a
/// Z-machine game the id is the story's own object number and has no relationship to when the
/// player found the room. This is the row order [`build`] draws and the order [`labels`] numbers
/// same-named rooms in, so a room's number never moves when a lower-id duplicate is found later.
///
/// Starts from [`MapGraph::rooms_in_layer`] (ascending id) purely to seed a deterministic sort;
/// every room's `seq` is unique in practice, so that seed order never actually shows through.
fn rooms_by_seq(graph: &MapGraph, layer: LayerId) -> Vec<RoomId> {
    let mut ids = graph.rooms_in_layer(layer);
    ids.sort_by_key(|&id| graph.room(id).map(|r| r.seq).unwrap_or(u64::MAX));
    ids
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
/// Rooms that SHARE a display name are numbered in DISCOVERY order (SQ-0685) — eleven rooms called
/// "Maze" become "Maze 1".."Maze 11" in the order the player first walked into each of them —
/// because eleven identical rows is exactly the problem the matrix exists to solve, and the
/// player's own mental numbering is built while exploring, not sorted by the story's object table.
/// A number is minted the moment a room is first discovered and never changes afterward: finding a
/// LOWER-id duplicate later does not renumber the ones already found, because numbering was never
/// keyed on id in the first place. The numbering is otherwise display-only — identity stays the
/// room id — so it is stable across a save/load round trip exactly as before.
///
/// A uniquely-named room gets its initials instead of a number, so cells can point at it without
/// stealing a number from the numbered group.
pub fn labels(graph: &MapGraph, layer: LayerId) -> MatrixLabels {
    let ids = rooms_by_seq(graph, layer);
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
/// Rows are in DISCOVERY order (SQ-0685): ascending [`crate::graph::Room::seq`], the same order
/// [`labels`] numbers same-named rooms in, so a row's position and the number in its own label
/// always agree — row 1 really is "Maze 1". That order is persisted (each room's `seq`, stamped
/// once at first upsert, plus the graph's `next_seq` counter), so a map reload draws the rows in
/// exactly the same order and a player's "the one I called 7" keeps meaning the same room.
pub fn build(graph: &MapGraph, layer: LayerId) -> Matrix {
    let labels = labels(graph, layer);
    let rows = rooms_by_seq(graph, layer)
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

/// A cluster of rooms whose passages mostly do NOT come back the way they went — the shape of a
/// maze (SQ-0666).
///
/// `rooms` is the MAZE, not the connected component it sits in (SQ-0683): a maze embedded in
/// ordinary geography — Zork's maze hanging off the Troll Room, on the same layer as the Cellar
/// and the Gallery — is one component with the tidy rooms, and averaging the two together buries
/// the maze. See [`tangles_with`] for how the cluster is grown.
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
/// Calibrated on two real, partial player maps, and every number below was measured from the
/// fixtures rather than argued: `unit_tests/advent_maze_map.json` (Colossal Cave, maze hand-peeled
/// onto its own layer) and `unit_tests/zork1_maze_map.json` (Zork I, maze still sharing the Cellar
/// layer with ordinary geography — the case SQ-0683 fixed).
///
/// | dataset / layer | cluster | rooms | round trips | asymmetry |
/// |---|---|---:|---:|---:|
/// | Advent, maze layer | the maze | 6 | 12 | 1.00 |
/// | Advent, overworld | biggest | 3 | 5 | 0.60 |
/// | Zork I, Cellar layer | the maze | 7 | 12 | 0.83 |
/// | Zork I, Main layer | biggest (the house ring) | 4 | 8 | 1.00 |
///
/// 0.75 has clear air under both mazes. The house ring is the instructive miss: circling a Zork
/// house really is all-asymmetric geography (north from West of House arrives North of House, and
/// the way back is west), so asymmetry alone cannot disqualify it — [`TANGLE_MIN_ROOMS`] is what
/// holds it off, at 4 rooms against a bar of 6.
///
/// Note the denominator. "Non-reciprocal share over ALL edges" — the obvious reading — does not
/// separate maze from overworld at all: it is 0.96 for the Advent maze and 0.82 for the overworld,
/// because a passage nobody has walked BACK through yet says nothing about geometry, only about how
/// far exploration has got. Only passages walked both ways are evidence.
pub const TANGLE_ASYMMETRY: f32 = 0.75;

/// A cluster smaller than this is not worth offering to peel — two rooms that disagree are a
/// quirk, not a maze. Also the only bar that stops a small ring of ordinary rooms whose returns
/// all disagree with the compass (see [`TANGLE_ASYMMETRY`]'s table).
pub const TANGLE_MIN_ROOMS: usize = 6;

/// Below this many known round trips the ratio is noise: three out of three proves nothing.
/// Counted per END, so a passage walked both ways contributes 2 — eight is four round trips.
pub const TANGLE_MIN_RETURNS: usize = 8;

/// Clusters within `layer` that look like mazes, using the shipped thresholds.
pub fn tangles(graph: &MapGraph, layer: LayerId) -> Vec<Tangle> {
    tangles_with(graph, layer, TANGLE_ASYMMETRY, TANGLE_MIN_ROOMS, TANGLE_MIN_RETURNS)
}

/// [`tangles`] with the thresholds spelled out — the form the tests pin.
///
/// Clusters are grown through EVIDENCE, not through adjacency (SQ-0683). Two rooms join a cluster
/// when a walked round trip between them disagreed with the compass — go west, come back north.
/// Growing by plain adjacency instead makes the whole connected component the unit, and a maze
/// hanging off ordinary geography then dilutes into it and can never fire: the user's Zork I
/// Cellar layer is one 12-room component scoring 0.45, while the 7 maze rooms inside it score
/// 0.83. Only a layer whose maze has already been peeled onto a layer of its own — the Advent
/// fixture — ever fired under component clustering, which is exactly the case that needs no offer.
///
/// A cluster then ABSORBS any room whose every in-layer passage leads into it: a dead end reachable
/// only from the maze is part of the maze, however tidily its one passage reciprocates (Zork I's
/// second Dead End). Absorption cannot reach past the tangle into a tidy region, because a room
/// with one foot outside keeps its whole region out — and if it ever did, the ratio would tell on
/// it, since the stats below are measured over the FINAL room set.
///
/// Stats count every round trip among cluster members, reciprocal ones included: the denominator
/// is what keeps the ratio honest, and a maze that turns out to be half-reciprocal should stop
/// looking like a maze. A passage walked one way only is no evidence either way and counts in
/// neither column. Both ends of a walked passage count, so a round trip contributes 2.
///
/// Clusters are disjoint (seeds are disjoint, and an absorbed room's neighbours all lie in one
/// seed's cluster) and returned in ascending order of their lowest room id.
pub fn tangles_with(
    graph: &MapGraph,
    layer: LayerId,
    min_asymmetry: f32,
    min_rooms: usize,
    min_returns: usize,
) -> Vec<Tangle> {
    let ids = graph.rooms_in_layer(layer);
    let in_layer: BTreeSet<RoomId> = ids.iter().copied().collect();

    // In-layer passages only. Self-loops and Unknown edges say nothing about geometry.
    let edges: Vec<(RoomId, Direction, RoomId)> = graph
        .connections()
        .iter()
        .filter(|c| {
            !c.is_self_loop()
                && c.dir != Direction::Unknown
                && in_layer.contains(&c.origin)
                && in_layer.contains(&c.dest)
        })
        .map(|c| (c.origin, c.dir, c.dest))
        .collect();
    let directed: HashSet<(RoomId, Direction, RoomId)> = edges.iter().copied().collect();
    // Ordered pairs with a passage: `walked.contains(&(b, a))` is "the way back is known".
    let walked: HashSet<(RoomId, RoomId)> = edges.iter().map(|&(o, _, d)| (o, d)).collect();

    let mut neighbours: BTreeMap<RoomId, BTreeSet<RoomId>> = BTreeMap::new();
    // Seed links: the round trip was walked AND came back by some other direction.
    let mut seeded: BTreeMap<RoomId, BTreeSet<RoomId>> = BTreeMap::new();
    for &(origin, dir, dest) in &edges {
        neighbours.entry(origin).or_default().insert(dest);
        neighbours.entry(dest).or_default().insert(origin);
        if !walked.contains(&(dest, origin)) || directed.contains(&(dest, opposite(dir), origin)) {
            continue;
        }
        seeded.entry(origin).or_default().insert(dest);
        seeded.entry(dest).or_default().insert(origin);
    }

    let mut seen: BTreeSet<RoomId> = BTreeSet::new();
    let mut out = Vec::new();
    for &start in &ids {
        if !seeded.contains_key(&start) || !seen.insert(start) {
            continue;
        }
        let mut cluster = BTreeSet::from([start]);
        let mut q = VecDeque::from([start]);
        while let Some(cur) = q.pop_front() {
            for &next in seeded.get(&cur).into_iter().flatten() {
                if seen.insert(next) {
                    cluster.insert(next);
                    q.push_back(next);
                }
            }
        }

        // Absorb the rooms that hang off it, to a fixed point (a dead end behind a dead end).
        loop {
            let pendants: Vec<RoomId> = ids
                .iter()
                .copied()
                .filter(|id| !cluster.contains(id))
                .filter(|id| {
                    neighbours.get(id).is_some_and(|n| !n.is_empty() && n.is_subset(&cluster))
                })
                .collect();
            if pendants.is_empty() {
                break;
            }
            cluster.extend(pendants);
        }

        let mut known_returns = 0usize;
        let mut asymmetric = 0usize;
        for &(origin, dir, dest) in &edges {
            if !cluster.contains(&origin) || !cluster.contains(&dest) {
                continue;
            }
            if !walked.contains(&(dest, origin)) {
                continue; // no round trip walked yet — no evidence either way
            }
            known_returns += 1;
            if !directed.contains(&(dest, opposite(dir), origin)) {
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

    /// SQ-0685: the reported bug, reproduced directly. Numbering by ascending room id renumbers
    /// EVERY duplicate behind a newly-found lower-id one — for a Z-machine game the id is the
    /// story's own object number and has nothing to do with when the player found the room.
    /// Falsified against HEAD: reverting the `rooms_by_seq` ordering in `labels`/`build` back to
    /// plain `rooms_in_layer` makes `after.row_of(5)` come back `"Maze 2"` here, reproducing the
    /// exact symptom reported.
    #[test]
    fn a_lower_id_duplicate_discovered_later_does_not_renumber_earlier_rooms() {
        let mut g = MapGraph::new();
        // Two "Maze" rooms first, both with ids HIGHER than the one found later.
        g.upsert_room(5, "Maze".into()); // seq 0
        g.upsert_room(7, "Maze".into()); // seq 1
        let before = labels(&g, MAIN_LAYER);
        assert_eq!(before.row_of(5), "Maze 1");
        assert_eq!(before.row_of(7), "Maze 2");

        g.upsert_room(2, "Maze".into()); // a LOWER id than both, discovered third → seq 2
        let after = labels(&g, MAIN_LAYER);
        assert_eq!(after.row_of(5), "Maze 1", "room 5 keeps its number: id 2 arriving later must not move it");
        assert_eq!(after.row_of(7), "Maze 2", "nor must it move room 7");
        assert_eq!(after.row_of(2), "Maze 3", "the newcomer takes the next ordinal, whatever its id");

        // A revisit (upsert on an already-known room) must not re-mint its ordinal either.
        g.upsert_room(5, "Maze".into());
        assert_eq!(labels(&g, MAIN_LAYER).row_of(5), "Maze 1", "a revisit does not renumber");
    }

    /// `build`'s row order and `labels`' numbering have to agree — row 1 really is "Maze 1" — so
    /// both must be driven by the same discovery-sequence order, not room id (SQ-0685).
    #[test]
    fn build_row_order_follows_discovery_order_not_room_id() {
        let mut g = MapGraph::new();
        g.upsert_room(5, "E".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(9, "I".into());
        let m = build(&g, MAIN_LAYER);
        assert_eq!(
            m.rows.iter().map(|r| r.room).collect::<Vec<_>>(),
            vec![5, 2, 9],
            "rows are in the order the rooms were discovered, not ascending id"
        );
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

    /// The mirror of `⇱out`: an edge whose ORIGIN is outside the layer and whose destination is
    /// inside it. `maze()` already has one crossing the other way (room 2 → room 9, `Down`); this
    /// gives room 9 a way back IN, which must show up here and not be confused with the outbound
    /// one.
    #[test]
    fn inbound_border_edges_are_the_doors_into_the_layer() {
        let (mut g, l) = maze();
        g.add_edge(9, Direction::W, 1); // enters at the first row
        g.add_edge(9, Direction::S, 3); // enters at the third row
        let edges = inbound_border_edges(&g, l);
        assert_eq!(
            edges,
            vec![(9, Direction::W, 1), (9, Direction::S, 3)],
            "ordered by the entering room's row position, not insertion order"
        );
        // The existing OUTBOUND crossing (2 → 9) must never appear as inbound.
        assert!(edges.iter().all(|&(o, _, d)| o != 2 || d != 9));
        assert!(inbound_border_edges(&g, l).iter().all(|&(o, _, _)| g.layer_of(o) != l));
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

    /// A tidy region of `n` rooms in a chain, ids from `first`, every passage reciprocal.
    fn tidy(g: &mut MapGraph, first: u16, n: u16) {
        for i in first..first + n {
            g.upsert_room(i, format!("Tidy{i}"));
        }
        for i in first..first + n - 1 {
            g.add_edge(i, Direction::E, i + 1);
            g.add_edge(i + 1, Direction::W, i);
        }
    }

    /// Round trips among `rooms` and how many disagreed with the compass, computed independently
    /// of [`tangles_with`] so a test can ask what a room set WOULD have scored.
    fn stats(g: &MapGraph, rooms: &[RoomId]) -> (usize, usize) {
        let (mut known, mut asym) = (0, 0);
        for c in g.connections() {
            if c.is_self_loop() || !rooms.contains(&c.origin) || !rooms.contains(&c.dest) {
                continue;
            }
            if !g.connections().iter().any(|b| b.origin == c.dest && b.dest == c.origin) {
                continue;
            }
            known += 1;
            if !g
                .connections()
                .iter()
                .any(|b| b.origin == c.dest && b.dir == opposite(c.dir) && b.dest == c.origin)
            {
                asym += 1;
            }
        }
        (known, asym)
    }

    #[test]
    fn a_tangle_needs_a_high_asymmetry_a_real_size_and_a_real_sample() {
        // 10 rooms, 9 of 10 round trips disagree with the compass → 0.9 ≥ 0.75.
        let g = ring(10, 9);
        let t = tangles(&g, MAIN_LAYER);
        assert_eq!(t.len(), 1, "the ring is one cluster and it is a tangle: {t:?}");
        assert_eq!(t[0].rooms.len(), 10);
        assert_eq!(t[0].known_returns, 20, "each passage is counted from both ends");
        assert!(t[0].asymmetry() > 0.75);

        // Mostly reciprocal: the asymmetric evidence chains four rooms and stops there.
        assert!(tangles(&ring(10, 3), MAIN_LAYER).is_empty(), "0.3 asymmetry is an ordinary map");
        assert_eq!(
            tangles_with(&ring(10, 3), MAIN_LAYER, 0.0, 2, 0)
                .iter()
                .map(|t| t.rooms.len())
                .collect::<Vec<_>>(),
            vec![4],
            "and the cluster it does form is nowhere near maze size"
        );

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

    /// The seed links say where the cluster REACHES; the ratio is still judged over every round
    /// trip inside it. A cluster strung together by asymmetric passages but shot through with
    /// reciprocal ones is ordinary geography with a few oddities, and must not fire.
    #[test]
    fn reciprocal_round_trips_inside_a_cluster_still_count_against_it() {
        let mut g = ring(10, 9);
        for i in 1u16..=5 {
            // Extra passages between rooms already in the cluster, all agreeing with the compass.
            g.add_edge(i, Direction::Up, i + 5);
            g.add_edge(i + 5, Direction::Down, i);
        }
        let all = tangles_with(&g, MAIN_LAYER, 0.0, 2, 0);
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].rooms.len(), all[0].known_returns, all[0].asymmetric), (10, 30, 18));
        assert!(all[0].asymmetry() < TANGLE_ASYMMETRY, "0.60: {}", all[0].asymmetry());
        assert!(tangles(&g, MAIN_LAYER).is_empty(), "so it is not offered as a maze");
    }

    /// The bug SQ-0683 fixed, in miniature: a maze wired into ordinary geography by one tidy
    /// passage. Clustered by connected component the two average together and NOTHING fires —
    /// which was the shipped behaviour, and why a real Zork I maze could never be detected.
    #[test]
    fn a_maze_embedded_in_ordinary_geography_is_found_without_its_surroundings() {
        let mut g = ring(10, 9);
        tidy(&mut g, 100, 10);
        g.add_edge(1, Direction::S, 100); // the door between them, and it reciprocates
        g.add_edge(100, Direction::N, 1);

        let everything: Vec<RoomId> = g.rooms_in_layer(MAIN_LAYER);
        assert_eq!(everything.len(), 20, "one layer, one connected component");
        let (known, asym) = stats(&g, &everything);
        assert!(
            (asym as f32 / known as f32) < TANGLE_ASYMMETRY,
            "the component as a whole is under the bar ({asym}/{known}) — the old unit was wrong"
        );

        let t = tangles(&g, MAIN_LAYER);
        assert_eq!(t.len(), 1, "the maze inside it is still found: {t:?}");
        assert_eq!(t[0].rooms, (1..=10).collect::<BTreeSet<RoomId>>(), "and it is exactly the maze");
        assert_eq!((t[0].known_returns, t[0].asymmetric), (20, 18));
    }

    /// What the cluster takes with it. A dead end whose only passage reciprocates has no
    /// asymmetric evidence at all, but it hangs off the maze and nowhere else, so it belongs to it
    /// — a peel that left it behind would strand it. A room with a foot in the tidy region keeps
    /// its whole region out.
    #[test]
    fn a_room_that_hangs_off_the_tangle_is_absorbed_and_one_with_a_foot_outside_is_not() {
        let mut g = ring(10, 9);
        g.upsert_room(200, "Dead End".into());
        g.add_edge(1, Direction::Up, 200);
        g.add_edge(200, Direction::Down, 1);
        tidy(&mut g, 100, 3);
        g.add_edge(1, Direction::S, 100);
        g.add_edge(100, Direction::N, 1);

        let t = tangles(&g, MAIN_LAYER);
        assert_eq!(t.len(), 1);
        assert!(t[0].rooms.contains(&200), "the dead end comes along: {:?}", t[0].rooms);
        assert!(!t[0].rooms.contains(&100), "the door into the tidy region does not");
        assert!(!t[0].rooms.contains(&101), "nor anything behind it");
        assert_eq!(
            (t[0].known_returns, t[0].asymmetric),
            (22, 18),
            "and the absorbed room's tidy round trip is counted honestly, against the ratio"
        );
    }

    /// The thresholds must sit clear of the real numbers they were calibrated on, or they are just
    /// numbers. Measured off `unit_tests/advent_maze_map.json` and `unit_tests/zork1_maze_map.json`
    /// — see `TANGLE_ASYMMETRY`'s table, and `tests/advent_maze.rs` / `tests/zork1_maze.rs`, which
    /// recompute every one of these from the files themselves.
    #[test]
    fn the_thresholds_sit_clear_of_the_real_clusters_they_were_calibrated_on() {
        let (advent_maze, zork_maze) = (1.00f32, 0.83f32);
        assert!(TANGLE_ASYMMETRY < zork_maze && TANGLE_ASYMMETRY < advent_maze, "both mazes fire");

        let advent_overworld = 0.60f32;
        assert!(TANGLE_ASYMMETRY > advent_overworld, "the cave beside the maze does not");

        // Zork's overworld is the one asymmetry cannot rule out: circling the white house really
        // does come back another way. Its size is the whole margin.
        let (zork_house_ring, zork_house_rooms) = (1.00f32, 4usize);
        assert!(zork_house_ring > TANGLE_ASYMMETRY, "which is why the size bar exists");
        assert!(zork_house_rooms < TANGLE_MIN_ROOMS, "and why it is above four");
    }
}
