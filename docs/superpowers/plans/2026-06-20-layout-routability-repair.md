# Layout Routability Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the auto-layout shift rooms so every drawn edge has a clean orthogonal routing channel, and make the renderer enforce no-overlap as a hard invariant (drawing a distinct "unrouted" line where it genuinely cannot route).

**Architecture:** Add a deterministic hill-climb repair step to `mapper::layout::relayout_auto` (after collision-resolve, before anchoring) that moves rooms into free grid cells until the count of un-routable compass edges is minimized. Then strip the overlap-permitting fallback tiers from the renderer's `route_ortho` so it returns `Option` (clean route or `None`), and render `None` edges as a distinct dimmed ribbon.

**Tech Stack:** Rust workspace. `mapper` crate (no external deps beyond serde). `app` crate (ratatui 0.29). Tests via `cargo test -p <crate>`.

## Global Constraints

- Rooms stay on **integer grid cells**; "alignment relaxed" never means fractional coordinates. `cell_to_virtual`, persistence, DOT export, dump format, and the zvm bridge are untouched.
- Layout must remain **deterministic**: same graph → identical positions. No RNG, no `Math.random`, no time. The existing `relayout_is_deterministic` test must keep passing.
- Arrows keep the **typed command direction** (departure side = the compass side). The repair does not change arrow logic.
- Drawn edges = compass edges only (`grid_offset(dir).is_some()`). Non-compass dirs (Up/Down/In/Out/Unknown) are stubs and are excluded from routability entirely.
- No room overlap may ever be introduced (every existing overlap-invariant test must keep passing).
- `route_ortho` keeps **only its clean Tier-1 A\*** — the Tier-2 (drop path rules) and Tier-3 (overlap-permitting L) fallbacks are removed.

**Reference — `Direction`/`grid_offset`** (`crates/mapper/src/direction.rs`): cardinals map to unit offsets (`N=(0,-1)`, `S=(0,1)`, `E=(1,0)`, `W=(-1,0)`); diagonals to `(±1,±1)`; `Up/Down/In/Out/Unknown → None`.

---

### Task 1: Routability predicate

**Files:**
- Create: `crates/mapper/src/layout/routability.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (add `mod routability;` beside the existing `mod vpsc; mod constraints; mod stress;` at lines 31–33)

**Interfaces:**
- Consumes: `crate::direction::{grid_offset, Direction}`, `crate::graph::{MapGraph, RoomId}`.
- Produces (used by Task 2):
  - `fn first_steps(dir: Direction) -> Vec<(i32, i32)>`
  - `fn edge_routable(origin: (i32,i32), dest: (i32,i32), dir: Direction, occupied: &std::collections::BTreeMap<(i32,i32), RoomId>, bbox: (i32,i32,i32,i32)) -> bool`
  - `const BBOX_MARGIN: i32 = 2;`

- [ ] **Step 1: Create the file with the predicate and its tests**

Create `crates/mapper/src/layout/routability.rs`:

```rust
//! Grid-level routability: does a drawn edge have a clean orthogonal channel?
//!
//! Models each room as a single-cell obstacle. An edge is routable iff a BFS from
//! the origin cell — forced to take its first step in the edge's compass direction —
//! reaches the destination cell without entering any other room's cell. A clear grid
//! cell corresponds to a full empty 29×17 render stride (≫ the 21×11 box), so a
//! grid-level channel implies a render-level channel for room obstacles. Path-vs-path
//! congestion is out of scope (handled by the renderer's unrouted-line fallback).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::direction::{grid_offset, Direction};
use crate::graph::RoomId;

/// BFS search bound: the room bounding box is expanded by this many cells so a
/// route may detour just outside the outermost rooms.
pub const BBOX_MARGIN: i32 = 2;

/// The unit first-step deltas allowed when leaving the origin: a cardinal dir gives
/// exactly its own step; a diagonal gives each of its two axis components; a
/// non-compass dir gives none (caller treats those as "routable" — they aren't drawn).
pub fn first_steps(dir: Direction) -> Vec<(i32, i32)> {
    match grid_offset(dir) {
        None => Vec::new(),
        Some((dx, dy)) => {
            let mut v = Vec::new();
            if dx != 0 {
                v.push((dx.signum(), 0));
            }
            if dy != 0 {
                v.push((0, dy.signum()));
            }
            v
        }
    }
}

/// True iff a clean orthogonal channel exists from `origin` to `dest` whose first
/// step is in `dir`, treating every occupied cell except `origin`/`dest` as an
/// obstacle. BFS is bounded to `bbox = (min_x, min_y, max_x, max_y)` inclusive.
pub fn edge_routable(
    origin: (i32, i32),
    dest: (i32, i32),
    dir: Direction,
    occupied: &BTreeMap<(i32, i32), RoomId>,
    bbox: (i32, i32, i32, i32),
) -> bool {
    let steps = first_steps(dir);
    if steps.is_empty() || origin == dest {
        return true;
    }
    let (min_x, min_y, max_x, max_y) = bbox;
    let in_box =
        |c: (i32, i32)| c.0 >= min_x && c.0 <= max_x && c.1 >= min_y && c.1 <= max_y;
    let blocked = |c: (i32, i32)| c != origin && c != dest && occupied.contains_key(&c);

    let mut seen: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut q: VecDeque<(i32, i32)> = VecDeque::new();
    for (dx, dy) in steps {
        let c = (origin.0 + dx, origin.1 + dy);
        if in_box(c) && !blocked(c) && seen.insert(c) {
            q.push_back(c);
        }
    }
    while let Some(cur) = q.pop_front() {
        if cur == dest {
            return true;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nxt = (cur.0 + dx, cur.1 + dy);
            if in_box(nxt) && !blocked(nxt) && seen.insert(nxt) {
                q.push_back(nxt);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(cells: &[((i32, i32), RoomId)]) -> BTreeMap<(i32, i32), RoomId> {
        cells.iter().copied().collect()
    }

    #[test]
    fn first_steps_cardinal_and_diagonal() {
        assert_eq!(first_steps(Direction::W), vec![(-1, 0)]);
        assert_eq!(first_steps(Direction::N), vec![(0, -1)]);
        assert_eq!(first_steps(Direction::NE), vec![(1, 0), (0, -1)]);
        assert!(first_steps(Direction::Up).is_empty());
    }

    #[test]
    fn adjacent_edge_is_routable() {
        // origin (0,0) -W-> dest (-1,0): the west neighbour IS the destination.
        let occupied = occ(&[((0, 0), 1), ((-1, 0), 2)]);
        assert!(edge_routable((0, 0), (-1, 0), Direction::W, &occupied, (-3, -3, 3, 3)));
    }

    #[test]
    fn blocked_departure_cell_is_unroutable() {
        // origin (0,0) -W-> dest (-1,1) with room #74 at (-1,0) blocking due west.
        // The only first step (west) lands on a room, so the edge cannot leave cleanly.
        let occupied = occ(&[((0, 0), 25), ((-1, 0), 74), ((-1, 1), 76)]);
        assert!(!edge_routable((0, 0), (-1, 1), Direction::W, &occupied, (-3, -3, 3, 3)));
    }

    #[test]
    fn clear_lane_around_is_routable_after_shift() {
        // origin (0,1) -W-> dest (-1,1): west neighbour is the destination, clear.
        let occupied = occ(&[((0, 1), 25), ((-1, 0), 74), ((-1, 1), 76)]);
        assert!(edge_routable((0, 1), (-1, 1), Direction::W, &occupied, (-3, -3, 3, 3)));
    }
}
```

- [ ] **Step 2: Wire the module in**

In `crates/mapper/src/layout/mod.rs`, add `mod routability;` to the module declarations (currently lines 31–33):

```rust
mod vpsc;
mod constraints;
mod stress;
mod routability;
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test -p mapper routability`
Expected: PASS (4 tests: `first_steps_cardinal_and_diagonal`, `adjacent_edge_is_routable`, `blocked_departure_cell_is_unroutable`, `clear_lane_around_is_routable_after_shift`).

- [ ] **Step 4: Verify the whole crate still builds clean**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass, no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/layout/routability.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): grid-level edge routability predicate"
```

---

### Task 2: Repair search + integration

**Files:**
- Modify: `crates/mapper/src/layout/routability.rs` (add `repair_routability` + helpers + tests)
- Modify: `crates/mapper/src/layout/mod.rs` (call repair inside `relayout_auto`, after the `for comp in &components` loop, before the "Anchor the lowest-id room" block at line ~387)

**Interfaces:**
- Consumes: `edge_routable`, `BBOX_MARGIN` (Task 1); `crate::direction::grid_offset`; `crate::graph::MapGraph`.
- Produces (used by `relayout_auto`):
  - `pub fn repair_routability(graph: &MapGraph, pos: &mut std::collections::BTreeMap<RoomId, (i32, i32)>)`

- [ ] **Step 1: Write the failing integration test**

Add to the `tests` module in `crates/mapper/src/layout/mod.rs` (after the existing tests, before the closing `}` at line 671):

```rust
    #[test]
    fn repair_opens_channel_for_a129_corner() {
        // The exact #25/#74/#76 failure: three mutually inconsistent compass hints.
        // After relayout, every compass edge must be routable, and 25->W->76 must
        // become a clean (non-distorted) west shot once #25 shifts off #74's row.
        use crate::direction::Direction;
        let mut g = crate::graph::MapGraph::new();
        for (id, name) in [(25, "Canyon View"), (74, "Clearing"), (76, "Forest")] {
            g.upsert_room(id, name.into());
        }
        g.add_edge(74, Direction::E, 25);
        g.add_edge(74, Direction::S, 76);
        g.add_edge(25, Direction::W, 76);
        relayout_auto(&mut g);

        // Build the occupancy + bbox the way repair does, then assert all 3 edges route.
        let pos: std::collections::BTreeMap<RoomId, (i32, i32)> =
            g.rooms().filter_map(|r| r.pos.map(|p| (r.id, p))).collect();
        let occ: std::collections::BTreeMap<(i32, i32), RoomId> =
            pos.iter().map(|(&id, &c)| (c, id)).collect();
        let xs: Vec<i32> = pos.values().map(|p| p.0).collect();
        let ys: Vec<i32> = pos.values().map(|p| p.1).collect();
        let bb = (
            xs.iter().min().unwrap() - super::routability::BBOX_MARGIN,
            ys.iter().min().unwrap() - super::routability::BBOX_MARGIN,
            xs.iter().max().unwrap() + super::routability::BBOX_MARGIN,
            ys.iter().max().unwrap() + super::routability::BBOX_MARGIN,
        );
        for c in g.connections() {
            assert!(
                super::routability::edge_routable(pos[&c.origin], pos[&c.dest], c.dir, &occ, bb),
                "edge {}-{:?}->{} must be routable after repair; positions {pos:?}",
                c.origin, c.dir, c.dest
            );
        }
        // 25->76 is now geometrically truthful → not distorted.
        let e = g.connections().iter().find(|c| c.origin == 25 && c.dest == 76).unwrap();
        assert!(!e.distorted, "25->W->76 should be a clean west shot after repair; pos {pos:?}");
    }

    #[test]
    fn repair_terminates_on_impossible_mutual_south() {
        // A -S-> B and B -S-> A can't both be true. Repair must terminate, leave no
        // overlap, and leave at least one of the two edges distorted (unsatisfiable).
        use crate::direction::Direction;
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::S, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap");
        assert!(g.connections().iter().any(|c| c.distorted), "one mutual-S edge stays distorted");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p mapper repair_opens_channel_for_a129_corner`
Expected: FAIL — `repair_routability` is not yet called, so the cramped L leaves `25->76` un-routable (and likely distorted).

- [ ] **Step 3: Implement `repair_routability` + helpers**

Append to `crates/mapper/src/layout/routability.rs` (after `edge_routable`, before the `#[cfg(test)]` module). Note the new `use` for `MapGraph`:

Change the imports at the top of the file from
```rust
use crate::graph::RoomId;
```
to
```rust
use crate::graph::{MapGraph, RoomId};
```

Then add:

```rust
/// Max hill-climb passes. A backstop: each accepted move strictly lowers the score,
/// so the loop terminates well before this on real maps.
const MAX_REPAIR_PASSES: usize = 30;

fn occupied_map(pos: &BTreeMap<RoomId, (i32, i32)>) -> BTreeMap<(i32, i32), RoomId> {
    pos.iter().map(|(&id, &c)| (c, id)).collect()
}

fn bbox_of(pos: &BTreeMap<RoomId, (i32, i32)>) -> (i32, i32, i32, i32) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for &(x, y) in pos.values() {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    (min_x - BBOX_MARGIN, min_y - BBOX_MARGIN, max_x + BBOX_MARGIN, max_y + BBOX_MARGIN)
}

/// Number of drawn edges with no clean channel under `pos`.
fn unroutable_count(
    pos: &BTreeMap<RoomId, (i32, i32)>,
    drawn: &[(RoomId, RoomId, Direction)],
) -> usize {
    let occ = occupied_map(pos);
    let bb = bbox_of(pos);
    drawn
        .iter()
        .filter(|&&(o, d, dir)| !edge_routable(pos[&o], pos[&d], dir, &occ, bb))
        .count()
}

/// Total L1 displacement of `pos` from the pre-repair `stress` positions (the
/// deterministic tiebreaker — keeps the search from drifting needlessly).
fn displacement(
    pos: &BTreeMap<RoomId, (i32, i32)>,
    stress: &BTreeMap<RoomId, (i32, i32)>,
) -> i64 {
    pos.iter()
        .map(|(id, &(x, y))| {
            let (sx, sy) = stress[id];
            ((x - sx).abs() + (y - sy).abs()) as i64
        })
        .sum()
}

/// Greedily shift rooms into free grid cells until the number of un-routable drawn
/// edges can no longer be reduced. Score is lexicographic `(unroutable, displacement)`;
/// only strictly-improving moves are accepted, so the search is deterministic and
/// terminates. Drawn edges are compass edges (the only ones rendered as paths).
pub fn repair_routability(graph: &MapGraph, pos: &mut BTreeMap<RoomId, (i32, i32)>) {
    let drawn: Vec<(RoomId, RoomId, Direction)> = graph
        .connections()
        .iter()
        .filter(|c| grid_offset(c.dir).is_some())
        .map(|c| (c.origin, c.dest, c.dir))
        .collect();
    if drawn.is_empty() {
        return;
    }
    let stress = pos.clone();

    for _ in 0..MAX_REPAIR_PASSES {
        let base = (unroutable_count(pos, &drawn), displacement(pos, &stress));
        if base.0 == 0 {
            break;
        }
        let occ_now = occupied_map(pos);
        let bb = bbox_of(pos);
        let mut best: Option<(RoomId, (i32, i32), (usize, i64))> = None;

        for &(o, d, dir) in &drawn {
            // Only try to fix edges that are currently un-routable.
            if edge_routable(pos[&o], pos[&d], dir, &occ_now, bb) {
                continue;
            }
            for cand in [o, d] {
                let from = pos[&cand];
                for (dx, dy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
                    let to = (from.0 + dx, from.1 + dy);
                    if pos.values().any(|&p| p == to) {
                        continue; // occupied → would overlap
                    }
                    let mut trial = pos.clone();
                    trial.insert(cand, to);
                    let s = (unroutable_count(&trial, &drawn), displacement(&trial, &stress));
                    if s < base && best.as_ref().map_or(true, |&(_, _, bs)| s < bs) {
                        best = Some((cand, to, s));
                    }
                }
            }
        }

        match best {
            Some((room, to, _)) => {
                pos.insert(room, to);
            }
            None => break, // no strictly-improving move
        }
    }
}
```

- [ ] **Step 4: Call repair from `relayout_auto`**

In `crates/mapper/src/layout/mod.rs`, inside `relayout_auto`, insert the repair call after the `for comp in &components { … }` loop closes (line ~385) and before the `// Anchor the lowest-id room at (0,0)` comment (line ~387):

```rust
    // Open routing channels: shift rooms so every drawn edge has a clean lane.
    routability::repair_routability(graph, &mut final_pos);

    // Anchor the lowest-id room at (0,0) for a stable reference.
```

- [ ] **Step 5: Run the new tests to verify they pass**

Run: `cargo test -p mapper repair_`
Expected: PASS (`repair_opens_channel_for_a129_corner`, `repair_terminates_on_impossible_mutual_south`).

- [ ] **Step 6: Run the full mapper suite (determinism + no-overlap regressions)**

Run: `cargo test -p mapper && cargo clippy -p mapper`
Expected: all pass (including `relayout_is_deterministic`, `rooms_never_overlap_random_walk`, `contradictory_geometry_marks_distorted_not_overlap`), no clippy warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/mapper/src/layout/routability.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): repair layout so every drawn edge has a routing channel"
```

---

### Task 3: `route_ortho` returns `Option` (Tier-1 only)

**Files:**
- Modify: `crates/app/src/render/map.rs` — `route_ortho` (lines 134–302), its render-loop caller (line ~607), and six test callers (1191, 1215, 1353, 1364, 1425, 1435).

**Interfaces:**
- Produces (used by Task 4):
  - `fn route_ortho(dep: (i32,i32), arr: (i32,i32), dep_side: Side, blocked: &HashSet<(i32,i32)>, paths: &HashMap<(i32,i32), u8>) -> Option<Vec<(i32,i32)>>` — `Some(path)` is a clean Tier-1 route; `None` means no clean route exists.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/app/src/render/map.rs` (next to the other `route_*` tests, e.g. after `route_keeps_gap_from_earlier_path` ~line 1375):

```rust
    #[test]
    fn route_ortho_returns_none_when_boxed_in() {
        // A full vertical wall of blocked cells between dep and arr leaves no clean
        // route → route_ortho must report None rather than an overlapping fallback.
        let mut blocked = std::collections::HashSet::new();
        for y in -30..30 {
            blocked.insert((2, y));
        }
        let no_paths = std::collections::HashMap::new();
        let r = route_ortho((0, 0), (10, 0), Side::Right, &blocked, &no_paths);
        assert!(r.is_none(), "a fully walled-off edge must return None; got {r:?}");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app route_ortho_returns_none_when_boxed_in`
Expected: FAIL to compile — `route_ortho` currently returns `Vec`, so `r.is_none()` is a type error (and today it would return an L fallback, not `None`).

- [ ] **Step 3: Change `route_ortho` to Tier-1-only `Option`**

In `crates/app/src/render/map.rs`:

(a) Change the signature return type (line 134–140):

```rust
fn route_ortho(
    dep: (i32, i32),
    arr: (i32, i32),
    dep_side: Side,
    blocked: &std::collections::HashSet<(i32, i32)>,
    paths: &std::collections::HashMap<(i32, i32), u8>,
) -> Option<Vec<(i32, i32)>> {
```

(b) Change the `dep == arr` early return (line ~144) from `return vec![dep];` to:

```rust
    if dep == arr {
        return Some(vec![dep]);
    }
```

(c) Replace the tier block (lines 272–301, from the `// Tier 1: full clearance …` comment through the closing of the `l_h`/`l_v` logic) with a single clean-route return:

```rust
    // Clean route only: full room-clearance + path-vs-path rules. If A* cannot find
    // one, the edge has no clean channel — return None so the renderer can flag it as
    // unrouted rather than draw an overlapping fallback.
    astar(paths)
}
```

This deletes the `empty_paths`, `l_via`, `count_blocked`, `l_h`, and `l_v` locals (they were Tier-2/Tier-3 only). Keep everything above the Tier comment (the `astar` closure definition) unchanged.

- [ ] **Step 4: Update the render-loop caller**

At line ~607, the caller currently is:

```rust
        let path = route_ortho(dep, arr, dep_side, &blocked, &paths);
```

For this task, keep the renderer compiling by unwrapping to the previous behaviour's shape — Task 4 replaces this with the unrouted-line handling. Change it to:

```rust
        let path = match route_ortho(dep, arr, dep_side, &blocked, &paths) {
            Some(p) => p,
            None => continue, // Task 4 replaces this with an unrouted-line render
        };
```

- [ ] **Step 5: Update the six test callers to expect a clean route**

Each of these currently binds `let path = route_ortho(...);` / `let p1 = route_ortho(...);` / `let p2 = route_ortho(...);` and asserts on a clean path. Append `.expect("clean Tier-1 route")` to each call so they unwrap the `Option`:

- Line 1191 (`route_avoids_blocked_box`): `let path = route_ortho(dep, arr, Side::Right, &blocked, &std::collections::HashMap::new()).expect("clean Tier-1 route");`
- Line 1215 (`route_straight_when_clear`): same `.expect("clean Tier-1 route")` suffix.
- Line 1353 (`route_keeps_gap_from_earlier_path`, `p1`): `.expect("clean Tier-1 route")`.
- Line 1364 (`route_keeps_gap_from_earlier_path`, `p2`): `.expect("clean Tier-1 route")` — the gap-keeping detour is itself a Tier-1 route, so it still succeeds.
- Line 1425 (`route_crosses_perpendicular_straight_through`, `p1`): `.expect("clean Tier-1 route")`.
- Line 1435 (`route_crosses_perpendicular_straight_through`, `p2`): `.expect("clean Tier-1 route")`.

- [ ] **Step 6: Run the route tests**

Run: `cargo test -p app route_`
Expected: PASS — `route_ortho_returns_none_when_boxed_in` now returns `None`; the six updated callers still find clean routes.

- [ ] **Step 7: Run the full app suite**

Run: `cargo test -p app && cargo clippy -p app`
Expected: all pass, no clippy warnings. (The render loop's `continue`-on-`None` is temporary; any visual edge that can't route is simply skipped until Task 4.)

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "refactor(app): route_ortho returns Option (clean route only, no overlap fallback)"
```

---

### Task 4: Render unrouted edges as a distinct line

**Files:**
- Modify: `crates/app/src/render/map.rs` — add `PATH_BG_UNROUTED` style + `unrouted_l` helper, replace the Task-3 `continue` with unrouted accumulation, and blit unrouted cells distinctly.

**Interfaces:**
- Consumes: `route_ortho -> Option<...>` (Task 3); existing `walk_to`, `Side`, `in_area`, `cell_to_virtual` offsets, `arrowheads`, `path_cells`.
- Produces: a `PATH_BG_UNROUTED` ribbon (distinct `Color::DarkGray` background) for edges with no clean route; these cells are NOT added to the `paths` occupancy map.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/app/src/render/map.rs`:

```rust
    #[test]
    fn unroutable_edge_renders_distinct_and_keeps_clean_edges_cyan() {
        // Force an edge that cannot route: put a room directly on the only departure
        // lane. We set positions explicitly (bypassing relayout) so the renderer must
        // confront an un-routable edge and draw it as the distinct DarkGray ribbon.
        use mapper::graph::MapGraph;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::style::Color;

        let mut g = MapGraph::new();
        for id in [1u16, 2, 3] {
            g.upsert_room(id, "r".into());
        }
        // 1 at origin; 2 due west of 1 but blocked by 3 sitting between them.
        g.set_pos(1, (0, 0));
        g.set_pos(3, (-1, 0)); // blocks the west departure lane of edge 1->W->2
        g.set_pos(2, (-2, 0));
        g.add_edge(1, mapper::direction::Direction::W, 2); // boxed-in by room 3
        g.add_edge(1, mapper::direction::Direction::S, 3); // a clean edge for contrast
        let rm = mapper::render::render(&g);

        let area = Rect::new(0, 0, 200, 120);
        let mut buf = Buffer::empty(area);
        let mut state = AppState::default();
        state.zoom = Zoom::Boxes;
        state.scroll = (-4, -4);
        render_map(&rm, &state, area, &mut buf);

        let mut has_unrouted = false;
        let mut has_clean = false;
        for y in 0..area.height {
            for x in 0..area.width {
                match buf.cell((x, y)).map(|c| c.bg) {
                    Some(Color::DarkGray) => has_unrouted = true,
                    Some(Color::Cyan) => has_clean = true,
                    _ => {}
                }
            }
        }
        assert!(has_unrouted, "the boxed-in edge must render as a distinct DarkGray ribbon");
        assert!(has_clean, "the clean edge must still render as a normal Cyan ribbon");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app unroutable_edge_renders_distinct_and_keeps_clean_edges_cyan`
Expected: FAIL — the boxed-in edge is currently `continue`d (skipped), so no DarkGray cell exists.

- [ ] **Step 3: Add the unrouted style and L helper**

In `crates/app/src/render/map.rs`, add the style constant beside `PATH_BG_DISTORTED` (line ~454):

```rust
/// Ribbon background for an edge with no clean route — visibly distinct (dimmed) so a
/// rare routing failure is obvious rather than mistaken for a normal connector.
const PATH_BG_UNROUTED: Style = Style::new().bg(Color::DarkGray);
```

Add the L-builder near `walk_to` (after line ~319):

```rust
/// A simple two-segment L from `dep` to `arr` whose first step leaves on `dep_side`.
/// Used only to give an UNROUTED edge a visible (flagged) shape; it may overlap and is
/// never recorded in the path-occupancy map.
fn unrouted_l(dep: (i32, i32), arr: (i32, i32), dep_side: Side) -> Vec<(i32, i32)> {
    let corner = match dep_side {
        Side::Left | Side::Right => (arr.0, dep.1), // horizontal-first
        Side::Top | Side::Bottom => (dep.0, arr.1), // vertical-first
    };
    let mut pts = Vec::new();
    walk_to(&mut pts, dep, corner);
    pts.pop();
    walk_to(&mut pts, corner, arr);
    pts
}
```

- [ ] **Step 4: Accumulate unrouted cells in the render loop**

Near where `path_cells` is declared (the accumulators above the edge loop, ~line 470), add a sibling set:

```rust
    // Cells belonging to UNROUTED edges (no clean channel). Rendered distinctly and
    // deliberately NOT added to `paths`, so they never constrain later clean routes.
    let mut unrouted_cells: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
```

Replace the Task-3 caller block (the `match route_ortho … None => continue` from Task 3, line ~607) with:

```rust
        match route_ortho(dep, arr, dep_side, &blocked, &paths) {
            Some(path) => {
                // Record this clean path's cells with orientation, so later edges keep
                // clearance and may only cross it perpendicularly.
                for w in path.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let bit = if a.1 == b.1 { HORIZ } else { VERT };
                    *paths.entry(a).or_insert(0) |= bit;
                    *paths.entry(b).or_insert(0) |= bit;
                }
                for &(x, y) in &path {
                    let entry = path_cells.entry((x, y)).or_insert(true);
                    *entry = *entry && edge.distorted;
                }
            }
            None => {
                // No clean channel: draw a distinct, flagged L (may overlap); do NOT add
                // it to `paths` occupancy.
                for &(x, y) in &unrouted_l(dep, arr, dep_side) {
                    unrouted_cells.insert((x, y));
                }
            }
        }
```

Note: this **replaces** the existing `let path = route_ortho(...)` line AND the two follow-up `for` loops that recorded into `paths` and `path_cells` (Task 3 left those loops below the caller — fold them into the `Some` arm exactly as shown, and delete the now-duplicated standalone loops at lines ~571–583).

- [ ] **Step 5: Blit unrouted cells distinctly**

After the clean-ribbon blit loop (the `for (&(vx, vy), &all_distorted) in &path_cells` block, ~line 638), add an unrouted blit that skips any cell already painted by a clean path:

```rust
    // Blit unrouted ribbons (translate virtual → screen, clip). A clean path always
    // wins a shared cell, so skip cells already in path_cells.
    for &(vx, vy) in &unrouted_cells {
        if path_cells.contains_key(&(vx, vy)) {
            continue;
        }
        let (sx, sy) = (vx + off_x, vy + off_y);
        if in_area(sx, sy, area) {
            if let Some(cell) = buf.cell_mut((sx as u16, sy as u16)) {
                cell.set_symbol(" ").set_style(PATH_BG_UNROUTED);
            }
        }
    }
```

- [ ] **Step 6: Run the new test**

Run: `cargo test -p app unroutable_edge_renders_distinct_and_keeps_clean_edges_cyan`
Expected: PASS — a `DarkGray` ribbon and a `Cyan` ribbon both appear.

- [ ] **Step 7: Run the full workspace + clippy**

Run: `cargo test -p app && cargo test -p mapper && cargo clippy --workspace`
Expected: all pass, no clippy warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): render unrouted edges as a distinct dimmed ribbon"
```

---

## Self-Review

**Spec coverage:**
- Routability predicate (spec Component 1) → Task 1. ✓
- Repair search + `relayout_auto` integration (Component 2) → Task 2. ✓
- `route_ortho` Tier-1-only `Option` (Component 3, renderer invariant) → Task 3. ✓
- Distinct unrouted-line rendering (Component 3, decision 4) → Task 4. ✓
- Arrows keep typed direction (decision 2) → untouched by all tasks (Global Constraints). ✓
- Repaired edges un-distort via existing `mark_distorted` → asserted in Task 2 `repair_opens_channel_for_a129_corner`. ✓
- Determinism (Global Constraint) → Task 2 Step 6 runs `relayout_is_deterministic`. ✓
- Impossible-hint residual (spec Edge Cases) → Task 2 `repair_terminates_on_impossible_mutual_south`. ✓

**Placeholder scan:** No TBD/TODO; every code step shows complete code. The Task-3 `None => continue` is explicitly labelled temporary and replaced in Task 4 Step 4. ✓

**Type consistency:** `route_ortho` returns `Option<Vec<(i32,i32)>>` consistently from Task 3 onward (caller, six test callers, Task 4 `match`). `edge_routable`/`first_steps`/`BBOX_MARGIN` signatures match between Task 1 (definition) and Task 2 (use). `repair_routability(&MapGraph, &mut BTreeMap<RoomId,(i32,i32)>)` matches between definition (Task 2 Step 3) and call site (Task 2 Step 4). `PATH_BG_UNROUTED` / `unrouted_l` defined and used in Task 4. ✓
