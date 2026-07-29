//! Map layers ("segments"): a manual organizing tool. Every room belongs to exactly
//! one layer (default `MAIN_LAYER`). Layers are created/destroyed only by explicit
//! peel/merge — never derived. See docs/superpowers/specs/2026-06-23-manual-map-layers-design.md.

use std::collections::{BTreeSet, VecDeque};

/// Stable layer identifier. Layer `0` (`MAIN_LAYER`) always exists.
pub type LayerId = u16;

/// The permanent base layer every room starts in.
pub const MAIN_LAYER: LayerId = 0;

/// Per-layer metadata: a display name and the layer it was peeled from (for merge default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LayerMeta {
    pub name: String,
    pub parent: Option<LayerId>,
}

impl LayerMeta {
    /// Metadata for the base "Main" layer.
    pub fn main() -> Self {
        LayerMeta { name: "Main".to_string(), parent: None }
    }
}

use crate::direction::{grid_offset, opposite, Direction};
use crate::graph::{Connection, MapGraph, RoomId};
use crate::router::{fine_cell, stub_label, RoutedEdge};

/// Rooms reachable from `start` through PLANAR edges only (cardinals + diagonals),
/// staying within `start`'s current layer. Portal edges (Up/Down/In/Out/Unknown) are
/// not traversed — they are the cut. Edges are treated as undirected for reachability.
pub fn planar_region(graph: &MapGraph, start: RoomId) -> BTreeSet<RoomId> {
    region_with_cut(graph, start, &|_| false)
}

/// [`planar_region`], plus `cut`: any connection it accepts is severed too, on top of the
/// portal-edge cut. Lets a caller name its own seam (SQ-0360).
fn region_with_cut(
    graph: &MapGraph,
    start: RoomId,
    cut: &dyn Fn(&Connection) -> bool,
) -> BTreeSet<RoomId> {
    let layer = graph.layer_of(start);
    let mut seen = BTreeSet::new();
    seen.insert(start);
    let mut q = VecDeque::new();
    q.push_back(start);
    while let Some(cur) = q.pop_front() {
        for c in graph.connections() {
            if grid_offset(c.dir).is_none() || cut(c) {
                continue; // portal edge, or the caller's own seam → cut
            }
            let other = if c.origin == cur {
                c.dest
            } else if c.dest == cur {
                c.origin
            } else {
                continue;
            };
            if graph.layer_of(other) == layer && seen.insert(other) {
                q.push_back(other);
            }
        }
    }
    seen
}

/// Why a peel could not happen. Each variant is a distinct thing to tell the player — a peel that
/// refuses without saying why reads as a broken command (SQ-0360).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeelRefusal {
    /// The region already spans its whole layer: there is nothing to separate it FROM.
    WholeLayer,
    /// The room has no passage that way.
    NoSuchPassage,
    /// The passage exists, but its two ends stay connected by some other route, so cutting it
    /// separates nothing.
    NotASeam,
    /// The passage leads out of the layer. Peeling divides a layer; it does not cross one.
    LeavesLayer,
}

/// Peel at a NAMED seam: sever the `dir` passage out of `from`, and peel `from`'s OWN region into
/// a fresh layer under the one it left. Returns the new `LayerId` (SQ-0360).
///
/// [`peel_region`] can only cut where a portal edge already divides a layer, so it is powerless on
/// a layer that is one connected sprawl — Zork's underground being 35 rooms of solid compass maze.
/// This lets the player say where the boundary is instead of waiting for one to exist.
///
/// It peels `from`'s side, exactly as [`peel_region`] peels `start`'s. Taking the FAR side instead
/// made one command mean two opposite things depending on which end of the passage you stood at —
/// and worse, left the peeled-from room behind while its neighbours moved, so the new layer became
/// a SIBLING of the rooms it connects to rather than a child of them. Merging then threw those
/// rooms up to a layer they have no connection to at all (SQ-0364).
///
/// The cut takes the passage's RECIPROCAL with it. A passage is normally two connections
/// (`A -E-> B` and `B -W-> A`), so severing only the named one leaves the back-edge holding the two
/// halves together and no seam could ever cut. It is deliberately just that pair, not every edge
/// between the rooms: if `A` also reaches `B` another way, that is a second passage, the boundary
/// is not real, and `NotASeam` says so.
pub fn peel_at_edge(
    graph: &mut MapGraph,
    from: RoomId,
    dir: Direction,
) -> Result<LayerId, PeelRefusal> {
    peel_across(graph, from, dir, false)
}

/// Peel at the passage the player just WALKED IN THROUGH: sever the `from -dir-> dest` pair and
/// peel `dest`'s region — `dest` being the room the player now stands in (SQ-0552).
///
/// A bare `peel-layer` means "separate the area I just entered from the one behind me", so the
/// seam is the incoming edge. [`peel_at_edge`] cannot express it: the passage may be one-way, and
/// even when it is not, the way back is often not the reciprocal — Adventure's maze is entered
/// SOUTH from the Long Hall and left by going DOWN, so cutting the current room's north exit cuts
/// a maze-internal edge and separates nothing.
///
/// The kept side is still the region containing the room the player is standing in, exactly as for
/// [`peel_at_edge`] and [`peel_region`] — only the seam differs. Keeping the other end instead
/// would leave the player looking at a layer they are not in, the mirror of the SQ-0364 mistake.
pub fn peel_at_arrival(
    graph: &mut MapGraph,
    from: RoomId,
    dir: Direction,
) -> Result<LayerId, PeelRefusal> {
    peel_across(graph, from, dir, true)
}

/// Shared body of [`peel_at_edge`] and [`peel_at_arrival`]: cut the `from -dir-> dest` passage and
/// peel the region on one of its ends into a fresh layer. `keep_far` picks which end is kept —
/// `dest`'s region when true, `from`'s when false.
fn peel_across(
    graph: &mut MapGraph,
    from: RoomId,
    dir: Direction,
    keep_far: bool,
) -> Result<LayerId, PeelRefusal> {
    if grid_offset(dir).is_none() {
        // A portal is already a cut: `peel_region` is the operation for those.
        return Err(PeelRefusal::NoSuchPassage);
    }
    let dest = graph
        .connections()
        .iter()
        .find(|c| c.origin == from && c.dir == dir)
        .map(|c| c.dest)
        .ok_or(PeelRefusal::NoSuchPassage)?;
    let src = graph.layer_of(from);
    if graph.layer_of(dest) != src {
        return Err(PeelRefusal::LeavesLayer);
    }
    let back = opposite(dir);
    let (keep, shed) = if keep_far { (dest, from) } else { (from, dest) };
    let region = region_with_cut(graph, keep, &|c: &Connection| {
        (c.origin == from && c.dir == dir) || (c.origin == dest && c.dir == back)
    });
    if region.contains(&shed) {
        return Err(PeelRefusal::NotASeam);
    }
    let name = graph.room(keep).map(|r| r.label().to_string()).unwrap_or_default();
    let new = graph.new_layer(Some(src), name);
    for id in region {
        graph.set_room_layer(id, new);
    }
    Ok(new)
}

/// Peel `start`'s planar region into a fresh layer. Returns the new `LayerId`, or
/// [`PeelRefusal::WholeLayer`] when the region already spans the whole source layer (nothing to
/// separate). To divide a layer that has no portal seam in it, name one with [`peel_at_edge`].
pub fn peel_region(graph: &mut MapGraph, start: RoomId) -> Result<LayerId, PeelRefusal> {
    let src = graph.layer_of(start);
    let region = planar_region(graph, start);
    let whole_layer: BTreeSet<RoomId> = graph.rooms_in_layer(src).into_iter().collect();
    if region == whole_layer {
        return Err(PeelRefusal::WholeLayer);
    }
    let name = graph.room(start).map(|r| r.label().to_string()).unwrap_or_default();
    let new = graph.new_layer(Some(src), name);
    for id in region {
        graph.set_room_layer(id, new);
    }
    Ok(new)
}

/// True iff the connection's endpoints are in different layers.
pub fn is_interlayer(graph: &MapGraph, conn: &Connection) -> bool {
    graph.layer_of(conn.origin) != graph.layer_of(conn.dest)
}

/// Fold every room in `layer` into its parent (or `MAIN_LAYER` if it has none) and drop
/// the layer's metadata. Returns the target layer. No-op (returns `MAIN_LAYER`) for `MAIN_LAYER`.
pub fn merge_layer(graph: &mut MapGraph, layer: LayerId) -> LayerId {
    if layer == MAIN_LAYER {
        return MAIN_LAYER;
    }
    let target = graph.layers().get(&layer).and_then(|m| m.parent).unwrap_or(MAIN_LAYER);
    for id in graph.rooms_in_layer(layer) {
        graph.set_room_layer(id, target);
    }
    graph.remove_layer(layer);
    target
}

/// One portal badge per connection that LEAVES `layer` — i.e. whose ORIGIN is in
/// `layer` and dest is elsewhere. Anchored at the origin; carries the destination room
/// name and destination layer name. Emitting only outgoing edges means a reciprocal
/// up/down pair (`A↓B` + `B↑A`) shows a single down glyph on A and a single up glyph on
/// B, instead of both glyphs on both rooms.
pub fn interlayer_badges(graph: &MapGraph, layer: LayerId) -> Vec<RoutedEdge> {
    let mut out = Vec::new();
    for c in graph.connections() {
        if !is_interlayer(graph, c) || graph.layer_of(c.origin) != layer {
            continue;
        }
        let (here, there, dir) = (c.origin, c.dest, c.dir);
        let Some(here_pos) = graph.room(here).and_then(|r| r.pos) else { continue };
        let fine = fine_cell(here_pos);
        let dest_layer = graph.layer_of(there);
        let dest_lbl = graph
            .room(there)
            .map(|r| format!("{} · {}", r.label(), graph.layer_name(dest_layer)));
        out.push(RoutedEdge {
            origin: here,
            dest: there,
            dir,
            points: vec![fine, (fine.0, fine.1 - 1)],
            distorted: false,
            is_stub: true,
            label: Some(stub_label(dir).to_string()),
            arrival_dir: None,
            dest_label: dest_lbl,
            is_interlayer: true, // interlayer_badges only ever emits cross-layer edges
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn two_floors() -> MapGraph {
        // Floor 1: 1-E-2 (planar). 1-Down-3 (portal) to floor 2: 3-E-4 (planar).
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Hall"), (2, "Study"), (3, "Cellar"), (4, "Wine")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3);
        g.add_edge(3, Direction::Up, 1);
        g.add_edge(3, Direction::E, 4);
        g.add_edge(4, Direction::W, 3);
        g
    }

    #[test]
    fn planar_region_stops_at_portals() {
        let g = two_floors();
        let region = planar_region(&g, 3);
        let want: BTreeSet<RoomId> = [3, 4].into_iter().collect();
        assert_eq!(region, want, "down-portal cuts the cellar off from the upper floor");
    }

    #[test]
    fn peel_region_moves_floor_to_new_layer() {
        let mut g = two_floors();
        let l = peel_region(&mut g, 3).expect("a proper sub-region peels");
        assert_eq!(g.layer_of(3), l);
        assert_eq!(g.layer_of(4), l);
        assert_eq!(g.layer_of(1), MAIN_LAYER);
        assert_eq!(g.layer_of(2), MAIN_LAYER);
        assert_eq!(g.layer_name(l), "Cellar");
    }

    // ── SQ-0360: peel at a named seam ────────────────────────────────────────

    /// A chain A-B-C-D with no portal in it: `peel_region` is powerless (one region), but naming
    /// the B→C passage cuts there. This is Zork's Cellar in miniature — 35 rooms of compass maze
    /// with no portal seam to find.
    #[test]
    fn naming_a_passage_cuts_a_layer_that_has_no_portal_seam() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
            g.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4)] {
            g.add_edge(a, Direction::E, b);
            g.add_edge(b, Direction::W, a);
        }
        assert_eq!(
            peel_region(&mut g, 1),
            Err(PeelRefusal::WholeLayer),
            "one connected region: the automatic peel has nothing to cut on"
        );

        // Peel from B, cutting east: B's OWN side leaves — the same side `peel_region` would take.
        let l = peel_at_edge(&mut g, 2, Direction::E).expect("the B→C passage is a seam");
        assert_eq!(g.layers()[&l].parent, Some(MAIN_LAYER), "the new layer hangs off the one it left");
        assert_eq!(g.layer_name(l), "B", "named for the room peeled, as peel_region names for `start`");
        assert_eq!(g.rooms_in_layer(l), vec![1, 2], "B's side leaves, B included");
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![3, 4], "the far side stays put");

        // Standing at the other end of the SAME passage peels the other side — and each time it is
        // the side you are standing on, never a coin flip about which end you happened to pick.
        let mut g2 = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C"), (4, "D")] {
            g2.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3), (3, 4)] {
            g2.add_edge(a, Direction::E, b);
            g2.add_edge(b, Direction::W, a);
        }
        let l2 = peel_at_edge(&mut g2, 3, Direction::W).expect("same seam, other end");
        assert_eq!(g2.rooms_in_layer(l2), vec![3, 4], "C's side leaves, C included");
    }

    /// SQ-0364: peeling at a seam must leave the new layer a CHILD of the rooms it still connects
    /// to, so merging puts it back where its connections are.
    ///
    /// Taking the FAR side broke that: the room you peeled FROM stayed behind while its neighbours
    /// moved, making the new layer a sibling of everything it touches. Zork's case — peel east from
    /// a Maze room and the Maze stays on the Cellar while the other 24 rooms leave — then merged
    /// the Maze up to Main, a layer it has no connection to whatsoever.
    #[test]
    fn peeling_at_a_seam_then_merging_puts_the_rooms_back_where_they_were() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "Troll Room"), (2, "Maze"), (3, "Maze2")] {
            g.upsert_room(id, n.into());
        }
        for (a, b) in [(1, 2), (2, 3)] {
            g.add_edge(a, Direction::W, b);
            g.add_edge(b, Direction::E, a);
        }
        // Everything starts on a sub-layer, as Zork's underground does.
        let cellar = g.new_layer(Some(MAIN_LAYER), "Cellar".into());
        for id in [1, 2, 3] {
            g.set_room_layer(id, cellar);
        }

        // Peel the Maze by standing IN it and cutting the way back out.
        let maze = peel_at_edge(&mut g, 2, Direction::E).expect("seam");
        assert_eq!(g.rooms_in_layer(maze), vec![2, 3], "the Maze leaves, both its rooms");
        assert_eq!(g.rooms_in_layer(cellar), vec![1], "the Troll Room stays");
        assert_eq!(
            g.layers()[&maze].parent,
            Some(cellar),
            "the Maze hangs off the Cellar — where the room it still connects to lives"
        );

        // So merging returns it to its connections, not to some layer it never touched.
        let back = merge_layer(&mut g, maze);
        assert_eq!(back, cellar);
        assert_eq!(g.layer_of(2), cellar, "the Maze is back beside the Troll Room");
        assert_ne!(g.layer_of(2), MAIN_LAYER, "and never lands on Main, which it has no edge to");
    }

    /// The cut must take the passage's RECIPROCAL with it. Severing only `B -E-> C` would leave
    /// `C -W-> B` holding the halves together, and no seam could ever cut.
    #[test]
    fn cutting_a_passage_severs_its_back_edge_too() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1); // the reciprocal
        let l = peel_at_edge(&mut g, 1, Direction::E).expect("a lone passage is a seam");
        assert_eq!(g.rooms_in_layer(l), vec![1], "A peels itself off; B stays");
    }

    // ── SQ-0552: peel at the passage walked IN through ───────────────────────

    /// Adventure's maze in miniature: entered SOUTH from the Long Hall, left by going DOWN.
    /// The way back is neither the reciprocal of the way in nor even a planar edge, so
    /// [`peel_at_edge`] on the current room cannot name the passage that was walked — its north
    /// exit is another maze room, and cutting that separates nothing. Naming the INCOMING edge
    /// cuts the real boundary, and still peels the side the player is standing on.
    #[test]
    fn a_one_way_entrance_peels_from_the_passage_walked_in_through() {
        let mut g = MapGraph::new();
        for (id, n) in [(1, "At West End of Long Hall"), (2, "Maze"), (3, "Maze"), (4, "Maze")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::S, 2); // walked in
        g.add_edge(2, Direction::Down, 1); // the way back — a portal, and not the reciprocal
        // Maze innards, cross-linked as a maze is: cutting any one of these leaves a way round.
        g.add_edge(2, Direction::N, 3);
        g.add_edge(3, Direction::S, 2);
        g.add_edge(2, Direction::E, 4);
        g.add_edge(4, Direction::N, 3);

        assert_eq!(
            peel_at_edge(&mut g, 2, Direction::N),
            Err(PeelRefusal::NotASeam),
            "cutting the current room's north exit cuts a maze-internal edge — the old behaviour"
        );

        let l = peel_at_arrival(&mut g, 1, Direction::S).expect("the walked passage IS the boundary");
        assert_eq!(g.rooms_in_layer(l), vec![2, 3, 4], "the maze leaves — the side the player is on");
        assert_eq!(g.rooms_in_layer(MAIN_LAYER), vec![1], "the hall behind stays put");
        assert_eq!(g.layer_name(l), "Maze", "named for the room entered, not the one left");
        assert_eq!(
            g.layers()[&l].parent,
            Some(MAIN_LAYER),
            "and hangs off the layer it left, so merging puts it back beside the hall"
        );
    }

    /// A passage whose ends stay connected another way is not a boundary, and says so.
    #[test]
    fn a_passage_with_a_way_round_is_not_a_seam() {
        // A→B directly, and A→C→B as well: cutting A-B separates nothing.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::N, 3);
        g.add_edge(3, Direction::E, 2);
        assert_eq!(peel_at_edge(&mut g, 1, Direction::E), Err(PeelRefusal::NotASeam));
        assert_eq!(g.layers().len(), 1, "and nothing was peeled");
    }

    #[test]
    fn peel_at_edge_needs_a_planar_passage_inside_the_layer() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::Down, 2); // a portal: already a cut, so not this command's job
        assert_eq!(peel_at_edge(&mut g, 1, Direction::Down), Err(PeelRefusal::NoSuchPassage));
        assert_eq!(peel_at_edge(&mut g, 1, Direction::W), Err(PeelRefusal::NoSuchPassage), "no such exit");

        // A passage that already leaves the layer divides nothing within it.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "A"), (2, "B"), (3, "C")] {
            g.upsert_room(id, n.into());
        }
        g.add_edge(1, Direction::Down, 2);
        g.add_edge(2, Direction::E, 3);
        let l = peel_region(&mut g, 2).expect("B/C peel off on the portal");
        assert_eq!(g.layer_of(2), l);
        assert_eq!(peel_at_edge(&mut g, 1, Direction::Down), Err(PeelRefusal::NoSuchPassage));
    }

    #[test]
    fn peel_whole_layer_is_noop() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        assert_eq!(peel_region(&mut g, 1), Err(PeelRefusal::WholeLayer), "region is the whole layer → refused, and it says why");
    }

    #[test]
    fn reciprocal_crossing_shows_one_glyph_per_side() {
        // A reciprocal 1↓3 / 3↑1 across a peeled boundary must NOT draw both glyphs on
        // both rooms: the upper room shows only its down exit, the lower only its up exit.
        let mut g = two_floors();
        g.set_pos(1, (0, 0));
        g.set_pos(3, (0, 0));
        let l = peel_region(&mut g, 3).expect("peel cellar");

        let up = interlayer_badges(&g, MAIN_LAYER);
        assert_eq!(up.len(), 1, "Hall side shows exactly one crossing badge");
        assert_eq!(up[0].origin, 1);
        assert_eq!(up[0].dir, Direction::Down, "Hall shows its DOWN exit to the cellar");

        let down = interlayer_badges(&g, l);
        assert_eq!(down.len(), 1, "Cellar side shows exactly one crossing badge");
        assert_eq!(down[0].origin, 3);
        assert_eq!(down[0].dir, Direction::Up, "Cellar shows its UP exit to the hall");
    }

    #[test]
    fn interlayer_badges_are_per_edge() {
        // Two staircases between the same two layers must yield two distinct badges.
        let mut g = MapGraph::new();
        for (id, n) in [(1, "HallN"), (2, "HallS"), (3, "CellarN"), (4, "CellarS")] {
            g.upsert_room(id, n.into());
        }
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, 1));
        g.add_edge(1, Direction::E, 2); // keep them in one planar region up top
        g.add_edge(2, Direction::W, 1);
        g.add_edge(1, Direction::Down, 3); // staircase A
        g.add_edge(2, Direction::Down, 4); // staircase B
        let l = peel_region(&mut g, 3).expect("peel cellar");
        g.set_room_layer(4, l); // ensure both cellar rooms in the new layer
        g.set_pos(3, (0, 0));
        g.set_pos(4, (0, 1));
        let up = interlayer_badges(&g, MAIN_LAYER);
        assert_eq!(up.len(), 2, "two independent staircases → two badges");
        assert!(up.iter().all(|e| e.is_stub));
        // peel_region(3) named the layer after room 3's label ("CellarN"); badge text is
        // "<dest room> · <dest layer>". Assert the shape, not a brittle literal.
        assert!(up.iter().all(|e| e.dest_label.as_deref().is_some_and(|s| s.contains(" · "))));
        assert!(up.iter().any(|e| e.dest_label.as_deref() == Some("CellarN · CellarN")));
    }

    #[test]
    fn merge_layer_folds_into_parent_and_removes_meta() {
        let mut g = two_floors();
        let l = peel_region(&mut g, 3).unwrap(); // parent = MAIN_LAYER
        let target = merge_layer(&mut g, l);
        assert_eq!(target, MAIN_LAYER);
        assert_eq!(g.layer_of(3), MAIN_LAYER);
        assert_eq!(g.layer_of(4), MAIN_LAYER);
        assert!(!g.layers().contains_key(&l), "merged layer's metadata is removed");
    }

    #[test]
    fn merge_main_is_noop() {
        let mut g = two_floors();
        assert_eq!(merge_layer(&mut g, MAIN_LAYER), MAIN_LAYER);
        assert!(g.layers().contains_key(&MAIN_LAYER));
    }
}
