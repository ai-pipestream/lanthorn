# Layout Regimes Implementation Plan (Phase 1 of 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the from-scratch-every-turn global layout with stable **incremental local placement** per turn, and repurpose `relayout_auto` into the on-demand **re-tidy** pipeline (`sort → optimize`), deleting the constrained-stress engine and both repair passes.

**Architecture:** `Mapper::observe` stops calling the global solver and instead places only the new room relative to the previous room (shift-beyond on collision); existing rooms never move. `relayout_auto` keeps its name and signature but its body becomes a fresh pipeline: per-axis longest-path layering ("sort") followed by bounded crossing reduction measured against the real lane router ("optimize"). All external callers (`router.rs` tests, the app acceptance gate) keep working because the function name and signature are unchanged.

**Tech Stack:** Rust 2021, workspace crate `mapper`. Tests are `#[cfg(test)]` modules. Run with `cargo test -p mapper`. Lint with `cargo clippy -p mapper --all-targets` (the workspace treats warnings as errors — no dead code allowed).

## Global Constraints

- Rooms stay on **integer grid cells** (`Room.pos: Option<(i32,i32)>`). No fractional coordinates anywhere.
- **Determinism is mandatory:** same graph / same event stream ⇒ byte-identical positions on every call. No RNG, no `HashMap` iteration order in anything that affects positions — use `BTreeMap`/`BTreeSet`/sorted `Vec`.
- **Rooms never overlap:** no two rooms may share a cell after any operation.
- **Single plane in this phase.** Segments do not exist yet (Phase 3). Treat the whole graph as one coordinate plane. Portal directions (`Up/Down/In/Out`) and `Unknown` place via `nearest_free_cell` adjacent to the previous room, exactly as the current code's non-compass handling — this stays forward-compatible with segments.
- **Planar directions** = those where `grid_offset(dir).is_some()`: `N S E W NE NW SE SW`. **Portal/unknown** = `grid_offset(dir).is_none()`: `Up Down In Out Unknown`.
- `relayout_auto(graph: &mut MapGraph)` keeps its exact public signature and name.
- Per CLAUDE.md: surgical changes only; do not refactor adjacent code; match existing style.

**Do NOT run `git checkout`, `git restore`, `git stash`, or any command that discards working-tree changes** — prior sessions lost uncommitted work this way. Commit forward only.

---

### Task 1: Incremental local placement core

**Files:**
- Create: `crates/mapper/src/layout/incremental.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (add `mod incremental;` and re-export)
- Test: in `crates/mapper/src/layout/incremental.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Consumes: `crate::graph::{MapGraph, RoomId}`, `crate::direction::{Direction, grid_offset}`, `crate::layout::{occupied_cells, nearest_free_cell}`.
- Produces:
  - `pub fn place_incremental(graph: &mut MapGraph, prev: RoomId, dest: RoomId, dir: Direction)` — places `dest` relative to `prev`. Precondition: `prev` has a `pos`. If `dest` already has a `pos`, it is a no-op (returns immediately). Planar `dir`: ideal cell = `prev.pos + grid_offset(dir)`; if the ideal cell is occupied and `dir` is a **cardinal**, shift-beyond opens it; if occupied and `dir` is a **diagonal**, fall back to `nearest_free_cell` biased toward the ideal. Portal/unknown `dir`: `nearest_free_cell` from a cell adjacent to `prev` (start at `prev.pos`).
  - `fn shift_beyond(graph: &mut MapGraph, ideal: (i32,i32), step: (i32,i32))` — for a cardinal unit `step` (exactly one of dx/dy is ±1, the other 0), translate every placed room whose cell lies at or beyond `ideal` along the `step` axis by one `step`, so `ideal` becomes free. "At or beyond" means: for `step=(1,0)`, every room with `cell.0 >= ideal.0`; for `step=(-1,0)`, `cell.0 <= ideal.0`; for `step=(0,1)`, `cell.1 >= ideal.1`; for `step=(0,-1)`, `cell.1 <= ideal.1`.

**Algorithm notes (read before coding):**
- `shift_beyond` preserves relative arrangement of the shifted set (rigid translation of a half-plane), which is the stability property we want. After one shift the ideal cell is guaranteed free (everything on/over the line moved away by one).
- Diagonal collision is rare and shift-beyond along a diagonal is ill-defined; `nearest_free_cell` from the ideal cell is the defined fallback.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    fn g_with(prev: RoomId, prev_pos: (i32, i32)) -> MapGraph {
        let mut g = MapGraph::new();
        g.upsert_room(prev, "prev".into());
        g.set_pos(prev, prev_pos);
        g
    }

    #[test]
    fn places_planar_room_at_compass_offset() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "n".into());
        place_incremental(&mut g, 1, 2, Direction::N);
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)));
    }

    #[test]
    fn places_diagonal_room_at_diagonal_cell() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "ne".into());
        place_incremental(&mut g, 1, 2, Direction::NE);
        assert_eq!(g.room(2).unwrap().pos, Some((1, -1)));
    }

    #[test]
    fn already_placed_dest_is_noop() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "x".into());
        g.set_pos(2, (5, 5));
        place_incremental(&mut g, 1, 2, Direction::N);
        assert_eq!(g.room(2).unwrap().pos, Some((5, 5)), "revisit must not move a placed room");
    }

    #[test]
    fn shift_beyond_opens_occupied_cardinal_cell() {
        // prev at (0,0); a blocker already sits north at (0,-1).
        let mut g = g_with(1, (0, 0));
        g.upsert_room(9, "blocker".into());
        g.set_pos(9, (0, -1));
        g.upsert_room(2, "n".into());
        place_incremental(&mut g, 1, 2, Direction::N);
        // New room lands truthfully at (0,-1); the blocker is shifted further north.
        assert_eq!(g.room(2).unwrap().pos, Some((0, -1)));
        assert_eq!(g.room(9).unwrap().pos, Some((0, -2)), "blocker shifted beyond");
        // prev did not move (it is south of the ideal line).
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)));
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn portal_dir_places_adjacent_without_overlap() {
        let mut g = g_with(1, (0, 0));
        g.upsert_room(2, "down".into());
        place_incremental(&mut g, 1, 2, Direction::Down);
        let p2 = g.room(2).unwrap().pos.unwrap();
        assert_ne!(p2, (0, 0), "must not land on prev");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p mapper incremental:: 2>&1 | tail -20`
Expected: FAIL — `place_incremental` / `shift_beyond` not found.

- [ ] **Step 3: Implement `incremental.rs`**

```rust
//! Incremental local placement — the per-turn layout regime.
//!
//! Places one newly discovered room relative to the previous room, in the
//! compass direction of the move, shifting only the rooms "beyond" the
//! insertion point on collision (Trizbort's strategy). Existing rooms
//! otherwise never move, so the map is stable turn-to-turn.

use crate::direction::{grid_offset, Direction};
use crate::graph::{MapGraph, RoomId};

use super::{nearest_free_cell, occupied_cells};

/// Place `dest` relative to `prev` via `dir`. See module/interface docs.
pub fn place_incremental(graph: &mut MapGraph, prev: RoomId, dest: RoomId, dir: Direction) {
    // Revisit / loop-closure: never move an already-placed room.
    if graph.room(dest).and_then(|r| r.pos).is_some() {
        return;
    }
    let prev_pos = match graph.room(prev).and_then(|r| r.pos) {
        Some(p) => p,
        None => return, // caller guarantees prev is placed; defensive no-op
    };

    match grid_offset(dir) {
        Some(delta) => {
            let ideal = (prev_pos.0 + delta.0, prev_pos.1 + delta.1);
            let occupied = occupied_cells(graph);
            if !occupied.contains(&ideal) {
                graph.set_pos(dest, ideal);
                return;
            }
            // Occupied. Cardinal → shift-beyond opens the ideal cell.
            let is_cardinal = (delta.0 == 0) ^ (delta.1 == 0);
            if is_cardinal {
                shift_beyond(graph, ideal, delta);
                graph.set_pos(dest, ideal);
            } else {
                // Diagonal fallback: nearest free cell from the ideal.
                let occ = occupied_cells(graph);
                let cell = nearest_free_cell(&occ, ideal);
                graph.set_pos(dest, cell);
            }
        }
        None => {
            // Portal / unknown: nearest free cell starting from prev.
            let occ = occupied_cells(graph);
            let cell = nearest_free_cell(&occ, prev_pos);
            graph.set_pos(dest, cell);
        }
    }
}

/// Translate every placed room at or beyond `ideal` along the `step` axis by
/// one `step`, opening `ideal`. `step` must be a cardinal unit vector.
fn shift_beyond(graph: &mut MapGraph, ideal: (i32, i32), step: (i32, i32)) {
    let ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    for id in ids {
        if let Some(pos) = graph.room(id).and_then(|r| r.pos) {
            let beyond = match step {
                (1, 0) => pos.0 >= ideal.0,
                (-1, 0) => pos.0 <= ideal.0,
                (0, 1) => pos.1 >= ideal.1,
                (0, -1) => pos.1 <= ideal.1,
                _ => false,
            };
            if beyond {
                graph.set_pos(id, (pos.0 + step.0, pos.1 + step.1));
            }
        }
    }
}
```

- [ ] **Step 4: Wire the module in `layout/mod.rs`**

After the existing `mod routability;` line (near line 34), add:

```rust
mod incremental;
pub use incremental::place_incremental;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mapper incremental:: 2>&1 | tail -20`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/mapper/src/layout/incremental.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): incremental local room placement with shift-beyond"
```

---

### Task 2: Switch `observe` to incremental placement

**Files:**
- Modify: `crates/mapper/src/mapper.rs:12-43` (`observe`, `set_mode`)
- Test: `crates/mapper/src/mapper.rs` `#[cfg(test)]` module (add stability tests)

**Interfaces:**
- Consumes: `place_incremental` (Task 1), `crate::layout::{relayout_auto, LayoutMode}`.
- Produces: behavior — after `observe`, the new room is placed incrementally and **no existing room moves** (except shift-beyond when its cell was the insertion point). First-ever room → `(0,0)`. After placement, the new edge's `distorted` flag is set by re-running the existing `mark_distorted` sweep over the whole graph. (`mark_distorted` is currently private in `layout/mod.rs`; expose it as `pub(crate) fn mark_distorted` — see Step 3.)

**Algorithm notes:**
- `observe` currently: upsert room, add edge if moved, set current, and (Auto) `relayout_auto`. New behavior: upsert, then if first room set `(0,0)`, else if moved place incrementally, add edge, set current, then mark distortion. No global relayout on the per-turn path.
- `set_mode(Manual)` no longer needs to relayout — positions are accumulated state and already set. Remove that call.

- [ ] **Step 1: Write the failing tests**

```rust
// add inside mapper.rs tests module
#[test]
fn first_room_anchors_at_origin() {
    let mut m = Mapper::default();
    m.observe(1, "Start", None);
    assert_eq!(m.graph.room(1).unwrap().pos, Some((0, 0)));
}

#[test]
fn incremental_observe_does_not_move_existing_rooms() {
    use crate::direction::Direction;
    let mut m = Mapper::default();
    m.observe(1, "A", None);
    m.observe(2, "B", Some(Direction::E)); // east of A
    let a = m.graph.room(1).unwrap().pos.unwrap();
    let b = m.graph.room(2).unwrap().pos.unwrap();
    m.observe(3, "C", Some(Direction::E)); // east of B
    // A and B must not have moved (C is placed past them, not into them).
    assert_eq!(m.graph.room(1).unwrap().pos.unwrap(), a, "A stayed put");
    assert_eq!(m.graph.room(2).unwrap().pos.unwrap(), b, "B stayed put");
    assert!(m.graph.room(3).unwrap().pos.unwrap().0 > b.0, "C is east of B");
}

#[test]
fn revisit_adds_edge_without_moving_rooms() {
    use crate::direction::Direction;
    let mut m = Mapper::default();
    m.observe(1, "A", None);
    m.observe(2, "B", Some(Direction::N));
    let snapshot: Vec<_> = m.graph.rooms().map(|r| (r.id, r.pos)).collect();
    // walk back south to A (already-placed room)
    m.observe(1, "A", Some(Direction::S));
    let after: Vec<_> = m.graph.rooms().map(|r| (r.id, r.pos)).collect();
    assert_eq!(snapshot, after, "returning to a placed room moves nothing");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p mapper --lib mapper:: 2>&1 | tail -25`
Expected: FAIL — `first_room_anchors_at_origin` (pos is None until layout runs) and the stability tests (current code relayouts and moves rooms).

- [ ] **Step 3: Expose `mark_distorted` and rewrite `observe` / `set_mode`**

In `crates/mapper/src/layout/mod.rs`, change the signature of `mark_distorted` (around line 295) from `fn mark_distorted` to:

```rust
pub(crate) fn mark_distorted(graph: &mut MapGraph, dropped: &BTreeSet<usize>) {
```

In `crates/mapper/src/mapper.rs`, change the import (line 3) to:

```rust
use crate::layout::{place_incremental, relayout_auto, LayoutMode};
use crate::layout::mark_distorted;
use std::collections::BTreeSet;
```

Replace `observe` (lines 12-25) with:

```rust
pub fn observe(&mut self, location: RoomId, name: &str, via: Option<Direction>) {
    self.graph.upsert_room(location, name.to_string());
    let prev = self.graph.current();
    match prev {
        None => {
            // First room ever: anchor at the origin.
            if self.graph.room(location).and_then(|r| r.pos).is_none() {
                self.graph.set_pos(location, (0, 0));
            }
        }
        Some(prev_id) => {
            if location != prev_id {
                let edge_dir = via.unwrap_or(Direction::Unknown);
                self.graph.add_edge(prev_id, edge_dir, location);
                if self.mode == LayoutMode::Auto {
                    place_incremental(&mut self.graph, prev_id, location, edge_dir);
                }
            }
        }
    }
    self.graph.set_current(location);
    if self.mode == LayoutMode::Auto {
        // Re-evaluate distortion over the whole graph (cheap); no relayout.
        mark_distorted(&mut self.graph, &BTreeSet::new());
    }
}
```

In `set_mode` (lines 35-43), remove the `relayout_auto` call so it becomes:

```rust
pub fn set_mode(&mut self, mode: LayoutMode) {
    if self.mode == mode {
        return;
    }
    self.mode = mode;
}
```

The `relayout_auto` import in `mapper.rs` is now used only by... nothing in this file. Remove `relayout_auto` from the `use` (keep `place_incremental`, `LayoutMode`). If the compiler flags `relayout_auto` unused, delete it from the import line.

- [ ] **Step 4: Run the mapper test suite**

Run: `cargo test -p mapper --lib 2>&1 | tail -30`
Expected: the three new tests PASS. Some **pre-existing** `mapper.rs` tests may need updating:
- `manual_mode_freezes_and_allows_nudge` — switches to Manual then observes room 3. With incremental placement gone-in-Manual, room 3 gets no pos. The test reads `before` (room 2 pos) which now exists from incremental placement in Auto before the switch — should still pass. If room 3's later `nudge`/assertions fail, adjust only as needed to reflect: Manual mode does not place new rooms.

Fix any pre-existing test that asserted the *old* relayout behavior, changing assertions to the incremental reality (new room in compass direction; existing rooms unmoved). Do not weaken the no-overlap assertions.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/mapper.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): observe places incrementally; no per-turn relayout"
```

---

### Task 3: Re-tidy "sort" — per-axis longest-path layering; delete the stress engine

**Files:**
- Create: `crates/mapper/src/layout/sort.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (replace `relayout_auto` body; drop `mod vpsc/stress/constraints/routability`; delete `seed_layout`; rewrite obsolete tests)
- Delete: `crates/mapper/src/layout/vpsc.rs`, `stress.rs`, `constraints.rs`, `routability.rs`
- Test: `crates/mapper/src/layout/sort.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `connected_components` (already in mod.rs — change to `pub(crate)`), `nearest_free_cell`, `grid_offset`, `crate::graph::{MapGraph, RoomId, Connection}`.
- Produces:
  - `pub(crate) fn layer_axis(n: usize, order_edges: &[(usize, usize)]) -> Vec<i32>` — longest-path layering for one axis. `order_edges` are `(lo, hi)` index pairs meaning "coord[lo] < coord[hi]". Deterministically breaks cycles (skip an edge that would close one, processing in slice order). Returns each node's integer coordinate = longest predecessor chain length.
  - `pub(crate) fn sort_layout(graph: &MapGraph) -> BTreeMap<RoomId,(i32,i32)>` — per-component longest-path layering on both axes from planar edges, packed left-to-right, overlaps resolved with `nearest_free_cell`, anchored so the lowest id sits at `(0,0)`.

**Algorithm notes:**
- For each connected component, map room ids → dense indices `0..n`.
- Build X order-edges from planar edges: for edge with `grid_offset = (dx,dy)`, if `dx>0` push `(origin_idx, dest_idx)`; if `dx<0` push `(dest_idx, origin_idx)`. Build Y order-edges with `dy`: `dy>0` → `(origin_idx,dest_idx)`; `dy<0` → `(dest_idx,origin_idx)`. (Diagonals contribute to both.)
- `layer_axis`: build adjacency from accepted edges only; an edge is accepted unless it would create a cycle (detected by a DFS reachability check `hi ⇒ lo` before adding). Then longest-path: process nodes in topological order, `coord[v] = max(coord[u]+1 for u→v, else 0)`.
- Pack components: offset each component's X so its min-x sits just right of the previous component's max-x + 1; normalize each component's min-y to 0.
- Resolve residual collisions in ascending room-id order via `nearest_free_cell`.
- Anchor: translate so the global lowest room id lands at `(0,0)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use std::collections::BTreeSet;

    #[test]
    fn layer_axis_chain_increments() {
        // 0<1<2 : longest paths 0,1,2
        let coords = layer_axis(3, &[(0, 1), (1, 2)]);
        assert_eq!(coords, vec![0, 1, 2]);
    }

    #[test]
    fn layer_axis_breaks_cycle_deterministically() {
        // 0<1, 1<2, 2<0 (cycle) — last edge dropped; no panic, finite coords.
        let coords = layer_axis(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(coords.len(), 3);
        assert!(coords[1] > coords[0] && coords[2] > coords[1]);
    }

    #[test]
    fn sort_layout_places_north_above_and_east_right() {
        let mut g = MapGraph::new();
        for id in 1..=3 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2); // 2 north of 1
        g.add_edge(1, Direction::E, 3); // 3 east of 1
        let pos = sort_layout(&g);
        assert!(pos[&2].1 < pos[&1].1, "north room above");
        assert!(pos[&3].0 > pos[&1].0, "east room right");
        // no overlap
        let cells: Vec<_> = pos.values().collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }

    #[test]
    fn sort_layout_anchors_lowest_id_at_origin() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        let pos = sort_layout(&g);
        assert_eq!(pos[&1], (0, 0));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p mapper sort:: 2>&1 | tail -20`
Expected: FAIL — module/functions absent.

- [ ] **Step 3: Implement `sort.rs`**

```rust
//! Re-tidy "sort" stage: per-axis longest-path layering from compass edges.
//!
//! Each planar edge imposes an ordering on one or both axes (east → larger x,
//! north → smaller y). We build a DAG per axis (dropping cycle-closing edges)
//! and assign integer coordinates by longest path, giving a compact grid that
//! honours every non-contradictory compass relation.

use std::collections::{BTreeMap, BTreeSet};

use crate::direction::grid_offset;
use crate::graph::{MapGraph, RoomId};

use super::{connected_components, nearest_free_cell};

/// Longest-path layering for one axis. `order_edges` = (lo,hi) meaning coord[lo] < coord[hi].
/// Cycle-closing edges (processed in slice order) are skipped.
pub(crate) fn layer_axis(n: usize, order_edges: &[(usize, usize)]) -> Vec<i32> {
    // Accept edges that don't close a cycle (hi cannot already reach lo).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for &(lo, hi) in order_edges {
        if lo == hi {
            continue;
        }
        if reaches(&adj, hi, lo) {
            continue; // would close a cycle → drop
        }
        adj[lo].push(hi);
    }
    // Longest path via memoised DFS over the DAG.
    let mut coord = vec![0_i32; n];
    let mut state = vec![0_u8; n]; // 0=unvisited,1=in-progress,2=done
    for s in 0..n {
        longest(s, &adj, &mut coord, &mut state);
    }
    coord
}

fn reaches(adj: &[Vec<usize>], from: usize, target: usize) -> bool {
    let mut stack = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(v) = stack.pop() {
        if v == target {
            return true;
        }
        if !seen.insert(v) {
            continue;
        }
        stack.extend(adj[v].iter().copied());
    }
    false
}

/// coord[v] = longest predecessor chain ending at v = max(coord[u]+1).
/// Computed as: coord[v] = max over successors handled by relaxation from sources.
fn longest(v: usize, adj: &[Vec<usize>], coord: &mut [i32], state: &mut [u8]) {
    if state[v] == 2 {
        return;
    }
    state[v] = 1;
    for &w in &adj[v] {
        longest(w, adj, coord, state);
        if coord[v] + 1 > coord[w] {
            coord[w] = coord[v] + 1;
        }
    }
    state[v] = 2;
}

/// Full per-component layering, packing, overlap resolution, and origin anchor.
pub(crate) fn sort_layout(graph: &MapGraph) -> BTreeMap<RoomId, (i32, i32)> {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    let mut pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    if ids.is_empty() {
        return pos;
    }
    let components = connected_components(graph, &ids);
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0;

    for comp in &components {
        let n = comp.len();
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        let mut xe: Vec<(usize, usize)> = Vec::new();
        let mut ye: Vec<(usize, usize)> = Vec::new();
        for c in graph.connections() {
            let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) else {
                continue;
            };
            if let Some((dx, dy)) = grid_offset(c.dir) {
                if dx > 0 { xe.push((a, b)); } else if dx < 0 { xe.push((b, a)); }
                if dy > 0 { ye.push((a, b)); } else if dy < 0 { ye.push((b, a)); }
            }
        }
        let xs = layer_axis(n, &xe);
        let ys = layer_axis(n, &ye);

        // Normalise this component to its own origin, then pack to the right.
        let min_x = *xs.iter().min().unwrap();
        let min_y = *ys.iter().min().unwrap();
        let mut max_x_used = pack_x;
        for (i, &id) in comp.iter().enumerate() {
            let desired = (pack_x + xs[i] - min_x, ys[i] - min_y);
            let cell = nearest_free_cell(&occupied, desired);
            occupied.insert(cell);
            pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2;
    }

    // Anchor the lowest-id room at (0,0).
    if let Some(&(ax, ay)) = pos.get(&ids[0]) {
        for p in pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }
    pos
}
```

- [ ] **Step 4: Rewire `relayout_auto` and delete the stress engine**

In `crates/mapper/src/layout/mod.rs`:

1. Change the module declarations near line 31-34 from:
```rust
mod vpsc;
mod constraints;
mod stress;
mod routability;
mod incremental;
```
to:
```rust
mod sort;
mod incremental;
```
2. Make `connected_components` callable from `sort.rs`: change `fn connected_components` (line 186) to `pub(crate) fn connected_components`.
3. Delete `seed_layout` (lines ~226-290) and the now-unused constants `GAP`, `ITERS` (keep `MAX_NODES` only if still used; if not, delete it too). Delete the `pair_offset` function **only if** nothing else references it — check with `grep -rn pair_offset crates`; `router.rs`/tests may use it. If referenced, keep it.
4. Replace the entire body of `relayout_auto` (lines ~313-403) with:
```rust
pub fn relayout_auto(graph: &mut MapGraph) {
    let ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    if ids.is_empty() {
        return;
    }
    let pos = sort::sort_layout(graph);
    for (&id, &p) in &pos {
        graph.set_pos(id, p);
    }
    mark_distorted(graph, &BTreeSet::new());
}
```
5. Delete the four files:
```bash
git rm crates/mapper/src/layout/vpsc.rs crates/mapper/src/layout/stress.rs crates/mapper/src/layout/constraints.rs crates/mapper/src/layout/routability.rs
```
6. In the `layout/mod.rs` `#[cfg(test)]` module, delete the tests that referenced the deleted engine/repairs: `repair_opens_channel_for_a129_corner` and `repair_terminates_on_impossible_mutual_south` (they call `super::routability::*`). Keep and, where needed, relax the others to relational assertions: `places_rooms_by_compass_offsets`, `rooms_never_overlap_random_walk`, `relayout_is_deterministic`, `disconnected_component_gets_placed`, `contradictory_geometry_marks_distorted_not_overlap`, `combined_offset_places_northeast`, `reciprocal_places_one_step_north`, `east_room_is_east`, `orientation_pinned_north_is_up`, `dynamic_layout_re_derives_from_scratch`, `dynamic_relayout_updates_positions`. For each, keep the no-overlap and directional-sign assertions; if one asserts an exact cell that the new layering changes, convert it to the directional inequality already present in the same test.

**Note on `combined_offset_places_northeast` / `reciprocal_places_one_step_north`:** the new sort uses pure axis ordering, so a reciprocal `N`+`S` pair yields B one step north in the same column (test still holds), and `N`+`W` yields B north-east (holds). If `pair_offset` was deleted, these tests don't use it directly — they use `relayout_auto` — so they remain valid.

- [ ] **Step 5: Build and run the full mapper suite**

Run: `cargo test -p mapper 2>&1 | tail -30`
Expected: PASS. Then `cargo clippy -p mapper --all-targets 2>&1 | tail -15` — Expected: no warnings (no dead code from the deletions).

Fix fallout: `router.rs` tests call `relayout_auto` then assert routing properties — they should still pass since rooms are placed and directional. If any assert an exact legacy position, convert to the directional inequality.

- [ ] **Step 6: Commit**

```bash
git add -A crates/mapper/src/layout crates/mapper/src/mapper.rs
git commit -m "feat(mapper): re-tidy sort via longest-path layering; remove stress engine + repairs"
```

---

### Task 4: Re-tidy "optimize" — bounded crossing reduction via the lane router

**Files:**
- Create: `crates/mapper/src/layout/optimize.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (`mod optimize;`; call it inside `relayout_auto` after sort)
- Test: `crates/mapper/src/layout/optimize.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::route::route_lanes`, `crate::route::{Channel, RoutePlan}`, `crate::graph::{MapGraph, RoomId}`, `sort` output positions.
- Produces:
  - `pub(crate) fn count_plan_crossings(plan: &RoutePlan) -> usize` — number of logical perpendicular crossings between distinct connectors. A crossing occurs when an `H(r)` segment of connector A (spanning columns `[s,e]`) and a `V(c)` segment of connector B (spanning rows `[s,e]`), A≠B, satisfy `s_h <= c <= e_h` AND `s_v <= r <= e_v`.
  - `pub(crate) fn optimize_layout(graph: &MapGraph, pos: &mut BTreeMap<RoomId,(i32,i32)>)` — bounded, deterministic: repeatedly try swapping the cells of two rooms that share an axis layer (same x, or same y) and are adjacent in the other axis; keep a swap only if it **strictly** reduces `count_plan_crossings` of the resulting plan; stop when no swap helps or after `MAX_OPT_PASSES = 8` passes. Then compact away empty rows/columns.

**Algorithm notes:**
- To evaluate a candidate, write the trial positions into a cloned graph (or set/restore positions), call `route_lanes`, count crossings, and compare. Always restore on reject.
- Determinism: iterate candidate room pairs in ascending `(id_a, id_b)` order; accept the first strictly-improving swap each pass.
- Compaction: collect used columns, remap to a dense `0..k` preserving order; same for rows. Pure integer, deterministic.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use std::collections::BTreeMap;

    #[test]
    fn count_crossings_zero_on_simple_pair() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (1, 0)); // adjacent east — straight connector, no crossing
        g.add_edge(1, Direction::E, 2);
        let plan = crate::route::route_lanes(&g);
        assert_eq!(count_plan_crossings(&plan), 0);
    }

    #[test]
    fn optimize_is_deterministic_and_no_overlap() {
        let mut g = MapGraph::new();
        for id in 1..=4 { g.upsert_room(id, "r".into()); }
        g.add_edge(1, Direction::N, 2);
        g.add_edge(1, Direction::E, 3);
        g.add_edge(2, Direction::E, 4);
        let mut pos: BTreeMap<_, _> =
            [(1u16, (0, 0)), (2, (0, -1)), (3, (1, 0)), (4, (1, -1))].into_iter().collect();
        let mut pos2 = pos.clone();
        optimize_layout(&g, &mut pos);
        optimize_layout(&g, &mut pos2);
        assert_eq!(pos, pos2, "optimize is deterministic");
        let cells: Vec<_> = pos.values().collect();
        let set: std::collections::BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap after optimize");
    }

    #[test]
    fn compaction_removes_empty_columns() {
        let mut g = MapGraph::new();
        g.upsert_room(1, "a".into());
        g.upsert_room(2, "b".into());
        g.add_edge(1, Direction::E, 2);
        // positions with a gap column at x=1 (rooms at 0 and 2)
        let mut pos: BTreeMap<_, _> = [(1u16, (0, 0)), (2, (2, 0))].into_iter().collect();
        optimize_layout(&g, &mut pos);
        let xs: std::collections::BTreeSet<i32> = pos.values().map(|p| p.0).collect();
        assert_eq!(xs, [0, 1].into_iter().collect(), "columns compacted to 0,1");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p mapper optimize:: 2>&1 | tail -20`
Expected: FAIL — module absent.

- [ ] **Step 3: Implement `optimize.rs`**

```rust
//! Re-tidy "optimize" stage: bounded crossing reduction + compaction.
//!
//! Measures crossings against the REAL lane router (not a proxy), then swaps
//! same-layer adjacent rooms only when it strictly reduces the count. Finally
//! compacts empty rows/columns.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{MapGraph, RoomId};
use crate::route::{route_lanes, Channel, RoutePlan};

const MAX_OPT_PASSES: usize = 8;

/// Logical perpendicular crossings between distinct connectors in `plan`.
pub(crate) fn count_plan_crossings(plan: &RoutePlan) -> usize {
    // Collect (connector_index, H segs) and (connector_index, V segs).
    let mut h: Vec<(usize, i32, i32, i32)> = Vec::new(); // (conn, row r, col start, col end)
    let mut v: Vec<(usize, i32, i32, i32)> = Vec::new(); // (conn, col c, row start, row end)
    for (ci, conn) in plan.connectors.iter().enumerate() {
        for seg in &conn.segs {
            let (lo, hi) = (seg.start.min(seg.end), seg.start.max(seg.end));
            match seg.channel {
                Channel::H(r) => h.push((ci, r, lo, hi)),
                Channel::V(c) => v.push((ci, c, lo, hi)),
            }
        }
    }
    let mut count = 0;
    for &(ci, r, hs, he) in &h {
        for &(cj, c, vs, ve) in &v {
            if ci == cj {
                continue;
            }
            if hs <= c && c <= he && vs <= r && r <= ve {
                count += 1;
            }
        }
    }
    // Each unordered crossing counted once per (h,v) pair; distinct connectors,
    // so no double counting beyond the intended single meet.
    count
}

/// Bounded, deterministic crossing-reduction swaps + compaction. Mutates `pos`.
pub(crate) fn optimize_layout(graph: &MapGraph, pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    for _ in 0..MAX_OPT_PASSES {
        let base = plan_crossings_for(graph, pos);
        if base == 0 {
            break;
        }
        let mut improved = false;
        let ids: Vec<RoomId> = pos.keys().copied().collect();
        'outer: for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                let (pa, pb) = (pos[&a], pos[&b]);
                // Only swap rooms sharing a layer (same column or same row).
                if pa.0 != pb.0 && pa.1 != pb.1 {
                    continue;
                }
                pos.insert(a, pb);
                pos.insert(b, pa);
                if plan_crossings_for(graph, pos) < base {
                    improved = true;
                    break 'outer;
                }
                // reject
                pos.insert(a, pa);
                pos.insert(b, pb);
            }
        }
        if !improved {
            break;
        }
    }
    compact(pos);
}

/// Route the graph with `pos` applied and count crossings; positions restored.
fn plan_crossings_for(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> usize {
    let mut g = clone_with_pos(graph, pos);
    let plan = route_lanes(&g);
    let _ = &mut g;
    count_plan_crossings(&plan)
}

fn clone_with_pos(graph: &MapGraph, pos: &BTreeMap<RoomId, (i32, i32)>) -> MapGraph {
    let rooms: Vec<_> = graph
        .rooms()
        .map(|r| {
            let mut r = r.clone();
            if let Some(&p) = pos.get(&r.id) {
                r.pos = Some(p);
            }
            r
        })
        .collect();
    MapGraph::from_parts(rooms, graph.connections().to_vec(), graph.current())
}

/// Remap used columns and rows to dense 0..k preserving order.
fn compact(pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    let xs: BTreeSet<i32> = pos.values().map(|p| p.0).collect();
    let ys: BTreeSet<i32> = pos.values().map(|p| p.1).collect();
    let xmap: BTreeMap<i32, i32> = xs.iter().enumerate().map(|(i, &x)| (x, i as i32)).collect();
    let ymap: BTreeMap<i32, i32> = ys.iter().enumerate().map(|(i, &y)| (y, i as i32)).collect();
    for p in pos.values_mut() {
        *p = (xmap[&p.0], ymap[&p.1]);
    }
}
```

**Note:** `count_plan_crossings` and `optimize_layout` need `RoutedConnector.segs` populated — `route_lanes` does this. Confirm `LaneSeg` fields are `channel`, `lane`, `start`, `end` (they are, per `route/mod.rs`). The `lane` field is unused here; crossings are detected at channel granularity, which is sound because the lane router guarantees same-lane segments never overlap, so any perpendicular meet is a real `┼`.

- [ ] **Step 4: Call optimize inside `relayout_auto`**

In `layout/mod.rs`, add `mod optimize;` next to `mod sort;`, and update `relayout_auto`:

```rust
pub fn relayout_auto(graph: &mut MapGraph) {
    let ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    if ids.is_empty() {
        return;
    }
    let mut pos = sort::sort_layout(graph);
    optimize::optimize_layout(graph, &mut pos);
    for (&id, &p) in &pos {
        graph.set_pos(id, p);
    }
    mark_distorted(graph, &BTreeSet::new());
}
```

- [ ] **Step 5: Run full mapper suite + clippy**

Run: `cargo test -p mapper 2>&1 | tail -30 && cargo clippy -p mapper --all-targets 2>&1 | tail -15`
Expected: all PASS, no warnings.

- [ ] **Step 6: Verify the app still builds and its acceptance gate passes**

Run: `cargo test -p app lane_routing 2>&1 | tail -20`
Expected: PASS — the A129 no-overlap acceptance gate (`render/map.rs:1483`) still holds under the new layout.

- [ ] **Step 7: Commit**

```bash
git add crates/mapper/src/layout/optimize.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): re-tidy crossing-reduction optimize via lane router"
```

---

## Self-Review

**Spec coverage (against `2026-06-21-incremental-segmented-automapping-design.md`):**
- ✅ Incremental local placement + shift-beyond (Tasks 1-2) — spec "Regime 1".
- ✅ Re-tidy `sort → route → optimize` (Tasks 3-4) — spec "Regime 2" (route = the lane router, used for crossing measurement; positions feed it at render).
- ✅ "Size by exits" dropped — not implemented, per decision 9.
- ✅ Stress engine + both repairs removed (Task 3) — spec "What is removed".
- ✅ Determinism, no-overlap, integer grid — Global Constraints + per-task tests.
- ⏭️ Diagonals as first-class **rendering** (corner departure) — placement handles diagonal *cells* here (Task 1); corner-departure rendering is Phase 2.
- ⏭️ Segments, portal labels, tabs, glyph theme — Phases 3-5 (separate plans).
- ⏭️ Re-tidy `R` keybinding + suggestion — Phase 4 (UI). This plan exposes `relayout_auto` as the re-tidy entry point; wiring the key is later.
- ⏭️ Persistence of accumulated positions — already works (positions persist today via `persist.rs`); no format change needed in this phase. The *model* change (positions are now primary, not re-derived on load) is satisfied because nothing re-derives on load.

**Placeholder scan:** none — every code step has complete code; every test has assertions.

**Type consistency:** `place_incremental`, `sort_layout`, `layer_axis`, `count_plan_crossings`, `optimize_layout`, `relayout_auto` signatures are consistent across tasks; `mark_distorted` exposed `pub(crate)`; `connected_components` exposed `pub(crate)`; `LaneSeg`/`Channel`/`RoutePlan` field names match `route/mod.rs`.

**Open risk to flag to the controller:** Task 3 rewrites several legacy `layout/mod.rs` and `router.rs` tests. The reviewer must confirm none were weakened to vacuity — each must keep its no-overlap and directional-sign assertions. If a legacy test asserted an exact cell that the new layering legitimately changes, converting it to the directional inequality is correct; deleting the assertion entirely is not.
