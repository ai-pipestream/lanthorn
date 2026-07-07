# Up/Down Placed Like N/S — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Up/Down connections lay out like weight-1 N/S directional hints (shove ordinary rooms, yield to reciprocal N/S), without changing their rendering, layer-cutting, reciprocal-collapse, or never-distorted behavior.

**Architecture:** Introduce a layout-only `layout_offset(dir)` (= `grid_offset`, plus Up→(0,-1)/Down→(0,1)). Switch the placement/scoring code from `grid_offset` to `layout_offset` so up/down attract toward the N/S cell at weight 1; keep `grid_offset` (which returns `None` for Up/Down) in rendering, routing, `planar_region`, `mark_distorted`, and the reciprocal back-edge checks, so those are all unchanged. `place_incremental` shoves like a cardinal; the late `stack_updown_rooms` stage is retired.

**Tech Stack:** Rust workspace. `mapper` crate (zero-dep layout) + `app` crate (tidy pipeline lives in `crates/app/src/render/map.rs` and `crates/app/src/input.rs`).

## Global Constraints

- `zvm`/`gvm` stay zero-dependency. `mapper` stays zero-dependency (this work adds only pure functions there).
- `grid_offset(Up/Down)` MUST stay `None`. Do NOT change `grid_offset`.
- Reciprocal weighting (`RECIPROCAL_WEIGHT`) stays keyed on `grid_offset` — up/down never earn reciprocal weight, even when both an Up and a Down edge exist between two rooms.
- `mark_distorted`, the router (`side_for`), `planar_region`, `route_topology`'s compass filter, and `draw_portal_connectors` stay on `grid_offset` — rendering (dotted lines + up/down glyphs, no arrows), layer-cutting, and reciprocal-collapse are unchanged.
- Up/down are a *soft* hint: never marked distorted (guaranteed because `mark_distorted` gates on `grid_offset` first). Up/down vs a one-way cardinal is a weight-1 tie; they bow only to reciprocal N/S.
- Every task ends green: `cargo test -p mapper` and `cargo test -p app`.

---

## Task 1: Add `layout_offset` helper

**Files:**
- Modify: `crates/mapper/src/direction.rs` (add fn after `grid_offset`, which ends at line 70)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub fn layout_offset(d: Direction) -> Option<(i32, i32)>` — `Up => (0,-1)`, `Down => (0,1)`, everything else delegates to `grid_offset`.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/mapper/src/direction.rs`:

```rust
#[test]
fn layout_offset_maps_updown_to_ns_but_grid_offset_stays_none() {
    use super::{grid_offset, layout_offset, Direction};
    // Up/Down get an N/S layout offset...
    assert_eq!(layout_offset(Direction::Up), Some((0, -1)));
    assert_eq!(layout_offset(Direction::Down), Some((0, 1)));
    // ...but grid_offset is untouched (still None) — rendering/layers rely on this.
    assert_eq!(grid_offset(Direction::Up), None);
    assert_eq!(grid_offset(Direction::Down), None);
    // Compass delegates to grid_offset.
    assert_eq!(layout_offset(Direction::N), grid_offset(Direction::N));
    assert_eq!(layout_offset(Direction::E), grid_offset(Direction::E));
    // In/Out/Unknown remain None.
    assert_eq!(layout_offset(Direction::In), None);
    assert_eq!(layout_offset(Direction::Unknown), None);
}
```

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test -p mapper layout_offset_maps_updown_to_ns_but_grid_offset_stays_none`
Expected: FAIL — `layout_offset` not found.

- [ ] **Step 3: Add the function**

In `crates/mapper/src/direction.rs`, immediately after `grid_offset` (after its closing brace on line 70):

```rust
/// Layout-only directional offset: like [`grid_offset`], but Up/Down also carry a
/// vertical N/S offset (Up → north, Down → south). Used ONLY by the layout,
/// placement, and directional-scoring code so up/down lay out like N/S. Rendering,
/// routing, layer-cutting, and `mark_distorted` keep using `grid_offset` (which
/// returns None for Up/Down), so up/down still draw as dotted portal stubs and are
/// never marked distorted.
pub fn layout_offset(d: Direction) -> Option<(i32, i32)> {
    match d {
        Direction::Up => Some((0, -1)),
        Direction::Down => Some((0, 1)),
        _ => grid_offset(d),
    }
}
```

- [ ] **Step 4: Run it — verify it passes**

Run: `cargo test -p mapper layout_offset_maps_updown_to_ns_but_grid_offset_stays_none`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/direction.rs
git commit -m "feat(mapper): add layout_offset (up/down -> N/S) for layout-only use"
```

---

## Task 2: `place_incremental` shoves up/down like a cardinal

**Files:**
- Modify: `crates/mapper/src/layout/incremental.rs` (import line 8; delta computation lines 32-37; guard lines 48-49)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `layout_offset` (Task 1).
- Behavior change: an Up/Down move now calls `shift_beyond` to claim the directly north/south cell, translating rooms beyond it aside (previously it yielded to `nearest_free_cell`).

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/mapper/src/layout/incremental.rs` (use whatever `MapGraph` construction the neighboring tests in this file use; the essential assertions are the positions):

```rust
#[test]
fn updown_shoves_like_a_cardinal() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    // A at origin; an ordinary room X already sits directly north of A at (0,-1).
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "X".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));

    // Discover B by going Up from A. B should CLAIM (0,-1) and shove X to (0,-2),
    // instead of yielding to a nearby free cell.
    g.upsert_room(3, "B".into());
    place_incremental(&mut g, 1, 3, Direction::Up);

    assert_eq!(g.room(3).unwrap().pos, Some((0, -1)), "Up dest lands directly north of A");
    assert_eq!(g.room(2).unwrap().pos, Some((0, -2)), "the ordinary room was shoved aside");
}
```

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test -p mapper updown_shoves_like_a_cardinal`
Expected: FAIL — B yields to a free cell (e.g. `(1,-1)` or `(0,-2)` while X stays), X not shoved.

- [ ] **Step 3: Implement**

In `crates/mapper/src/layout/incremental.rs`:

Change the import on line 8 from:
```rust
use crate::direction::{grid_offset, Direction};
```
to:
```rust
use crate::direction::{layout_offset, Direction};
```

Replace the `updown`/`delta` block (lines 32-37):
```rust
    let updown = matches!(dir, Direction::Up | Direction::Down);
    let delta = grid_offset(dir).or(match dir {
        Direction::Up => Some((0, -1)),  // hint: directly north
        Direction::Down => Some((0, 1)), // hint: directly south
        _ => None,
    });
```
with:
```rust
    let delta = layout_offset(dir);
```

Change the guard (lines 48-49) from:
```rust
            let is_cardinal = (delta.0 == 0) ^ (delta.1 == 0);
            if is_cardinal && !updown {
```
to:
```rust
            let is_cardinal = (delta.0 == 0) ^ (delta.1 == 0);
            if is_cardinal {
```

(`Direction` is still used by the signature and the remaining code; `grid_offset` is no longer referenced in this file — the import swap removes it. If the compiler flags `Direction` as unused, keep it only if still referenced; do not add `#[allow]`.)

- [ ] **Step 4: Run tests — verify green**

Run: `cargo test -p mapper updown_shoves_like_a_cardinal`
Expected: PASS.
Run: `cargo test -p mapper`
Expected: all pass (existing incremental tests still green — a compass move's behavior is unchanged since `layout_offset` equals `grid_offset` for compass).

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/layout/incremental.rs
git commit -m "feat(mapper): place_incremental shoves up/down like a cardinal"
```

---

## Task 3: Up/down count as weight-1 N/S directional hints in scoring + constraints

**Files:**
- Modify: `crates/mapper/src/layout/constraints.rs` (import line 7; directional loop offset at line 86)
- Modify: `crates/mapper/src/layout/mod.rs` (import line 29; primary-offset calls at lines 159, 281, 317, 352, 380)
- Test: `crates/mapper/src/layout/mod.rs` tests and/or `constraints.rs` tests

**Interfaces:**
- Consumes: `layout_offset` (Task 1).
- Behavior: `edge_is_satisfied`, `edges_respected_at`, `room_side_score`, `room_alignment_score`, `directional_hint_score`, and the `build_axis_constraints` directional loop use `layout_offset` for an edge's *primary* offset, so an up/down edge counts as a satisfied N/S side at weight 1. The reciprocal back-edge presence checks (`edges_respected_at` ~292-295, `room_side_score` ~325-328, `room_alignment_score` ~360-363) and `mark_distorted` (~570) and `room_compass_degree` (~400) stay on `grid_offset`.

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/mapper/src/layout/mod.rs`:

```rust
#[test]
fn directional_hint_score_counts_updown_as_ns() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    // B is directly north of A, reached by Up. Its N/S side is satisfied.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::Up, 2);
    // The single Up edge (dest on the north side of origin) counts as one satisfied side.
    // (directional_hint_score is side-only — it does NOT require exact column alignment.)
    assert_eq!(directional_hint_score(&g), 1, "an Up edge whose dest is north scores as satisfied");

    // Move B to the WRONG side (south of A): the Up hint (expects north) is unsatisfied.
    g.set_pos(2, (0, 3));
    assert_eq!(directional_hint_score(&g), 0, "an Up edge whose dest is south is unsatisfied");
}

#[test]
fn updown_edge_is_not_reciprocal_weighted() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    // Build A with a single Up edge to a room directly north.
    let updown_score = {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::Up, 2);
        g.add_edge(2, Direction::Down, 1); // an Up+Down pair must NOT double-count
        room_side_score(&g, 1)
    };
    // Same geometry, but a REAL reciprocal N/S pair (A--N-->B, B--S-->A), which DOES
    // earn RECIPROCAL_WEIGHT.
    let reciprocal_ns_score = {
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.set_pos(1, (0, 0));
        g.set_pos(2, (0, -1));
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        room_side_score(&g, 1)
    };
    // The up/down pair must score STRICTLY LESS than the reciprocal N/S pair — it never
    // gets the reciprocal doubling (reciprocal detection stays keyed on grid_offset,
    // which is None for up/down).
    assert!(updown_score < reciprocal_ns_score,
        "up/down (={updown_score}) must score below a reciprocal N/S pair (={reciprocal_ns_score})");
}

#[test]
fn mark_distorted_never_marks_updown() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;
    use std::collections::BTreeSet;
    // An Up edge that is NOT axis-aligned would be "unsatisfied" per edge_is_satisfied,
    // but mark_distorted gates on grid_offset (None for Up) and must leave it undistorted.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (5, 5)); // wildly off-axis
    g.add_edge(1, Direction::Up, 2);
    mark_distorted(&mut g, &BTreeSet::new());
    assert!(!g.connections()[0].distorted, "up/down is never marked distorted");
}
```

(If `room_side_score` / `directional_hint_score` / `mark_distorted` are private, these tests live in the same module and can call them directly. Match the exact return type in the asserts — if a score returns `usize`, the literals above are correct; if it returns something else, adjust the literal, not the intent.)

- [ ] **Step 2: Run them — verify they fail**

Run: `cargo test -p mapper directional_hint_score_counts_updown_as_ns updown_edge_is_not_reciprocal_weighted mark_distorted_never_marks_updown`
Expected: the first two FAIL (up/down currently ignored → scores 0), the third PASSES already (up/down already never distorted — it guards the invariant across this change).

- [ ] **Step 3: Switch primary-offset calls to `layout_offset`**

In `crates/mapper/src/layout/constraints.rs`:
- Import line 7: change `use crate::direction::grid_offset;` to `use crate::direction::layout_offset;`
- Directional loop, line 86: change
  ```rust
      let Some((dx, dy)) = grid_offset(conn.dir) else {
  ```
  to
  ```rust
      let Some((dx, dy)) = layout_offset(conn.dir) else {
  ```

In `crates/mapper/src/layout/mod.rs`:
- Import line 29: change `use crate::direction::{grid_offset, Direction};` to `use crate::direction::{grid_offset, layout_offset, Direction};` (both are needed — reciprocal checks, `mark_distorted`, and `room_compass_degree` keep `grid_offset`).
- `edge_is_satisfied`, line 159: change the `grid_offset(conn.dir)` in its `match` to `layout_offset(conn.dir)`.
- `edges_respected_at`, line 281: change `let Some(delta) = grid_offset(c.dir) else { continue };` to `layout_offset(c.dir)`.
- `room_side_score`, line 317: change the primary `grid_offset(c.dir)` to `layout_offset(c.dir)`.
- `room_alignment_score`, line 352: change the primary `grid_offset(c.dir)` to `layout_offset(c.dir)`.
- `directional_hint_score`, line 380: change `let Some(delta) = grid_offset(c.dir) else { return false };` to `layout_offset(c.dir)`.

DO NOT change the reciprocal back-edge presence checks (`grid_offset(r.dir).is_some()` at ~292-295, ~325-328, ~360-363), `mark_distorted` (~570), or `room_compass_degree` (~400). They stay on `grid_offset`.

- [ ] **Step 4: Run tests — verify green**

Run: `cargo test -p mapper directional_hint_score_counts_updown_as_ns updown_edge_is_not_reciprocal_weighted mark_distorted_never_marks_updown`
Expected: all PASS.
Run: `cargo test -p mapper`
Expected: all pass. If a pre-existing test now fails because it asserted up/down were ignored by a score, that is an intended behavior change — update that test to reflect up/down counting at weight 1, and note it in the report. Do NOT weaken an assertion about reciprocal weighting or distortion.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/layout/constraints.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): count up/down as weight-1 N/S directional hints in layout"
```

---

## Task 4: Retire `stack_updown_rooms` and dead `exact_alignment_count`

**Files:**
- Modify: `crates/app/src/render/map.rs` (delete `stack_updown_rooms` 1947-2083, `try_stack_dest_at` 2085-2224, `exact_alignment_count` 2225-2234, `stack_updown_rooms_observed` 2236-2261; delete the stack-specific tests at ~3077, 3098, 3147, 3175, 3199, 3219, 3240, 3260)
- Modify: `crates/app/src/input.rs` (`run_tidy_pipeline`: remove the `stack_updown_rooms_observed` import ~1576 and its call block ~1664; `tidy_layer_silent`: remove the `stack_updown_rooms` import ~1762 and its call ~1768)
- Test: add one integration test in `crates/app/src/input.rs` tests

**Interfaces:**
- Consumes: the unified up/down handling from Tasks 2-3 (up/down now placed and repaired as N/S by the general pipeline).
- Removes: `stack_updown_rooms`, `stack_updown_rooms_observed`, `try_stack_dest_at`, `exact_alignment_count`. Locate them by symbol name (line numbers shift as you edit); confirm with `grep -rn "stack_updown_rooms\|try_stack_dest_at\|exact_alignment_count" crates/` that the only remaining hits after deletion are gone.

- [ ] **Step 1: Write the failing/guarding integration test**

Add to the `#[cfg(test)] mod tests` in `crates/app/src/input.rs` (use `run_tidy_pipeline`, which is already tested there; match the neighboring tests' `Mapper`/`MapGraph` construction):

```rust
#[test]
fn reciprocal_ns_keeps_column_when_updown_contends() {
    use mapper::direction::Direction;
    use mapper::graph::MapGraph;

    // A--N-->B and B--S-->A  (reciprocal N/S pair, should share a column).
    // A also has an Up exit to C, which contends for the cell north of A.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.upsert_room(3, "C".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.set_pos(3, (0, -1)); // deliberately conflicting to force the tidy to resolve
    g.add_edge(1, Direction::N, 2);
    g.add_edge(2, Direction::S, 1);
    g.add_edge(1, Direction::Up, 3);

    let layer = g.layer_of(1);
    let _ = run_tidy_pipeline(&mut g, layer);

    // The reciprocal pair keeps its shared column; the up/down room yields off it.
    let a = g.room(1).unwrap().pos.unwrap();
    let b = g.room(2).unwrap().pos.unwrap();
    let c = g.room(3).unwrap().pos.unwrap();
    assert_eq!(a.0, b.0, "reciprocal N/S pair shares a column");
    assert_ne!(c, b, "the up/down room does not sit on top of the reciprocal neighbor");
}
```

- [ ] **Step 2: Run it — verify it passes with the stack stage still present**

Run: `cargo test -p app reciprocal_ns_keeps_column_when_updown_contends`
Expected: PASS (the property should already hold after Tasks 2-3; this test guards that removing the stack stage does not regress it).

- [ ] **Step 3: Delete the stack stage and its dead helper**

In `crates/app/src/render/map.rs`, delete the four functions (locate by name): `stack_updown_rooms`, `try_stack_dest_at`, `exact_alignment_count`, `stack_updown_rooms_observed` (contiguous block ~1947-2261, including their doc comments). Delete the stack-specific tests in the `#[cfg(test)]` block that call `stack_updown_rooms(...)` (the hits around lines 3077-3260).

In `crates/app/src/input.rs`:
- `run_tidy_pipeline`: remove `stack_updown_rooms_observed` from the `use` on line ~1576, and delete its call block (the `stack_updown_rooms_observed(&mut sub, Some(&mut |g, _label, desc, _s| { ... }))` frame-pushing block, ~1664 through its closing `}));`). The two `cleanup_overlaps` passes around it remain; the pipeline now goes ...repair_directional_hints → (second) cleanup_overlaps → compact.
- `tidy_layer_silent`: remove `stack_updown_rooms` from the `use` on line ~1762 and delete the `stack_updown_rooms(&mut sub);` call on line ~1768.

- [ ] **Step 4: Build, grep, and test**

Run: `grep -rn "stack_updown_rooms\|try_stack_dest_at\|exact_alignment_count" crates/`
Expected: no matches remain.
Run: `cargo test -p app`
Expected: all pass, including `reciprocal_ns_keeps_column_when_updown_contends`. If a deleted test was the only coverage of some behavior now handled by the unified path, that is expected; do not re-add stack-specific tests.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs crates/app/src/input.rs
git commit -m "refactor(app): retire stack_updown_rooms; up/down handled by unified N/S path"
```

---

## Task 5: Docs

**Files:**
- Modify: `docs/features/mapping.md` (the "Nautical directions" / up-down area, or wherever vertical handling is described)

**Interfaces:** none (docs only).

- [ ] **Step 1: Update the mapping docs**

In `docs/features/mapping.md`, add or adjust a bullet to state that vertical (up/down) connections are laid out like N/S neighbors — placed directly north (up) / south (down) and shifting ordinary rooms aside, while yielding to reciprocal N/S adjacencies — and are drawn as dotted connectors with up/down symbols rather than arrows. Match the surrounding markdown style. Example bullet:

```markdown
- **Vertical connections laid out like N/S** — up/down moves place the new room
  directly north (up) / south (down) of its neighbor, shifting ordinary rooms aside
  just like a compass move, but yielding to confirmed reciprocal N/S adjacencies.
  They render as dotted connectors with up/down symbols (or a stairs glyph set),
  never as arrows, and never as "distorted" red edges.
```

- [ ] **Step 2: Verify build/tests unaffected**

Run: `cargo test -p app` and `cargo test -p mapper`
Expected: all pass (docs-only change).

- [ ] **Step 3: Commit**

```bash
git add docs/features/mapping.md
git commit -m "docs(mapping): note up/down now laid out like N/S"
```

---

## Notes for the implementer

- **Spec refinements baked in:** `exact_alignment_count` is deleted (dead once the stack stage is gone) rather than switched to `layout_offset`; `edge_is_satisfied` DOES switch to `layout_offset`, which is safe because `mark_distorted` gates on `grid_offset` first (Task 3's `mark_distorted_never_marks_updown` test guards this).
- **`room_compass_degree` intentionally stays on `grid_offset`** — it is a move-preference tiebreaker in overlap cleanup; up/down anchoring is already provided by the weight-1 side/alignment scores, so leaving compass degree compass-only bounds the change. If a smoke test shows up/down rooms getting shuffled by overlap cleanup, revisit this.
- **Controller smoke test (outside the plan):** after Task 5, run the app on a vertical-heavy story (e.g. `stories/zork1-r88-s840726.z3`), walk a shaft (up the tree / down to the cellar), and eyeball that vertical rooms read as clean N/S stacks, reciprocal compass rooms keep their alignment, and no up/down edge renders red.

---

# Phase 2 — Route up/down through the N/S lane system (rendering unification)

See the spec's "Phase 2" section. Goal: up/down render as dotted **lane** connectors (crossing-eliminated, border-centered) with the up/down glyph on the room border, replacing the separate portal-stub path. Do these AFTER Tasks 1-5 (they build on the completed layout unification and the `layout_offset` helper).

## Task 6: Route up/down through the lane router (mapper)

**Files:**
- Modify: `crates/mapper/src/router.rs` (add `route_side`, do NOT change `side_for`)
- Modify: `crates/mapper/src/route/mod.rs` (filter ~611; exit-side lookups ~572, 634, 672; reciprocal pairing ~527/667/805; `oneway_entry_side` ~358)
- Test: `crates/mapper/src/route/mod.rs` tests

**Interfaces:**
- Consumes: `mapper::direction::layout_offset` (Task 1).
- Produces: up/down connections now yield `RoutedConnector`s in the lane `RoutePlan` (`route_lanes`), with `exit_dir == Up|Down` and `reciprocal == false`; they are never collapsed with a reverse up/down edge. `side_for` (old stub router) is unchanged.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` module in `crates/mapper/src/route/mod.rs`. Use the crate's real lane entry point (`route_lanes(&graph) -> RoutePlan`, `route/mod.rs:914`) and `RoutedConnector` (`route/mod.rs:22-50`, fields `exit_dir: Direction`, `reciprocal: bool`). Match the exact field/accessor names in the real structs.

```rust
#[test]
fn updown_is_routed_as_a_lane_connector_not_collapsed() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    // A--Up-->B (B north of A). Up should now be a lane connector, not just a stub.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::Up, 2);

    let plan = route_lanes(&g);
    let up_connectors: Vec<_> = plan.connectors.iter()
        .filter(|c| matches!(c.exit_dir, Direction::Up))
        .collect();
    assert_eq!(up_connectors.len(), 1, "the Up edge produces one lane connector");
    assert!(!up_connectors[0].reciprocal, "up/down connectors are never reciprocal");
}

#[test]
fn reciprocal_updown_pair_is_not_collapsed() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    // A--Up-->B and B--Down-->A: must stay TWO connectors, never collapsed to one.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::Up, 2);
    g.add_edge(2, Direction::Down, 1);

    let plan = route_lanes(&g);
    let vertical: Vec<_> = plan.connectors.iter()
        .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
        .collect();
    assert_eq!(vertical.len(), 2, "the Up and Down edges are drawn separately, not collapsed");
    assert!(vertical.iter().all(|c| !c.reciprocal));
}
```

- [ ] **Step 2: Run them — verify they fail**

Run: `cargo test -p mapper updown_is_routed_as_a_lane_connector_not_collapsed reciprocal_updown_pair_is_not_collapsed`
Expected: FAIL — today up/down are filtered out of the lane router (0 connectors).

- [ ] **Step 3: Implement**

Read the four functions first. Then:

1. In `crates/mapper/src/router.rs`, add (leave `side_for` untouched):
```rust
/// Like [`side_for`], but also gives Up/Down a routed box side (Up→Top, Down→Bottom).
/// Used ONLY by the lane router so vertical connectors get lanes + border anchors;
/// the old stub router (`route_all`) keeps using `side_for` (None for up/down).
pub fn route_side(dir: Direction) -> Option<Side> {
    match dir {
        Direction::Up => Some(Side::Top),
        Direction::Down => Some(Side::Bottom),
        _ => side_for(dir),
    }
}
```
2. In `crates/mapper/src/route/mod.rs`: change the working-set filter (~611) from `grid_offset(c.dir).is_some()` to `crate::direction::layout_offset(c.dir).is_some()`. Replace the exit-side lookups at ~572, 634, 672 (`side_for(c.dir)`) with `crate::router::route_side(c.dir)`.
3. Guard reciprocal pairing so up/down never pair: in `back_edge_idx` (~527) and/or the pairing sites (~667, ~805), skip pairing when `c.dir` is `Up`/`Down` (and never treat an up/down edge as another edge's back-edge). Ensure the emitted up/down connector has `reciprocal = false`.
4. Extend `oneway_entry_side` (~358) so `Up => Some(Side::Bottom)` (enters dest from below), `Down => Some(Side::Top)`.

Keep `In`/`Out`/`Unknown` excluded (`layout_offset` returns None for them, and `route_side` delegates to `side_for` = None), so they remain stubs.

- [ ] **Step 4: Run tests — verify green**

Run: `cargo test -p mapper updown_is_routed_as_a_lane_connector_not_collapsed reciprocal_updown_pair_is_not_collapsed`
Expected: PASS.
Run: `cargo test -p mapper`
Expected: all pass. If a routing test asserted up/down were absent from the plan, that is an intended change — update it and note it.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/router.rs crates/mapper/src/route/mod.rs
git commit -m "feat(mapper): route up/down through the lane system (no reciprocal collapse)"
```

---

## Task 7: Render up/down as dotted lane connectors with a border glyph; remove the portal-stub path (app)

**Files:**
- Modify: `crates/app/src/render/map.rs` — `render_lane_connectors` (~897-953) to draw up/down dotted + border glyph; delete `draw_portal_connectors` (~1134-1214) and `portal_stub` (~1219-1239) and their call in `render_map` (~524-526); make `draw_portal_icons` (~1272+) skip `Up`/`Down`.
- Test: `crates/app/src/render/map.rs` tests

**Interfaces:**
- Consumes: Task 6's lane connectors carrying `exit_dir == Up|Down`.
- Behavior: a Boxes-zoom up/down connector renders with a dotted body and the up/down glyph on the departure border; the old right-column portal stubs are gone; In/Out/Unknown still get in-room icons.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/app/src/render/map.rs` (match how neighboring render tests build a `MapGraph`, render at Boxes zoom, and scan the buffer — e.g. `mapper::render::render(&g)` + `render_map(&rm, &state, area, &mut buf)`):

```rust
#[test]
fn up_connector_draws_updown_glyph_on_border_not_arrow() {
    // A at origin, B directly north, reached by Up. At Boxes zoom the Up connector
    // must render the up glyph (default '↑') somewhere on the border between them,
    // and must NOT render a filled N arrow ('▲') for that vertical link.
    use mapper::direction::Direction;
    use mapper::graph::MapGraph;

    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::Up, 2);

    let state = AppState::default(); // Boxes zoom by default
    let rm = mapper::render::render(&g);
    let area = Rect::new(0, 0, 60, 30);
    let mut buf = Buffer::empty(area);
    // recenter so both rooms are on-screen if the neighboring tests do so; match their setup.
    render_map(&rm, &state, area, &mut buf);

    let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
    assert!(text.contains('↑'), "the Up connector shows the up glyph on the border");
}
```

(If the default portal up glyph is not `↑` in `AppState::default().symbols`, assert the actual configured `state.symbols.portal.up`. Build the on-screen setup to match the neighboring `render_map` tests — recenter/scroll as they do.)

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test -p app up_connector_draws_updown_glyph_on_border_not_arrow`
Expected: FAIL — up/down currently render as right-column dotted stubs (the glyph is placed via `portal_stub`, not on the lane border; and after Task 6 they may render as a solid N arrow until this task special-cases them).

- [ ] **Step 3: Implement**

Read `render_lane_connectors`, `arrow_for_departure`, `glyph_for`, `draw_portal_connectors`, `portal_stub`, and `draw_portal_icons` first. Then:
1. In `render_lane_connectors`, when `matches!(conn.exit_dir, Direction::Up | Direction::Down)`: (a) draw the connector body with the dotted glyph set (the portal dotted chars `sym.portal.path`/`path_h`, or a dotted `PathGlyphs`) instead of the shared solid `path`; (b) push the up/down glyph (`sym.portal.up` for Up, `sym.portal.down` for Down) as the departure "arrowhead" instead of `arrow_for_departure(conn.exit, arrows)`. Because these connectors have `reciprocal == false`, the far-end arrow block is already skipped.
2. Delete `draw_portal_connectors` and `portal_stub`, and remove their call from `render_map` (~524-526).
3. In `draw_portal_icons`, skip `Direction::Up` and `Direction::Down` (they now show a border glyph); keep drawing `In`/`Out`/`Unknown` icons.
4. Remove or update any `#[cfg(test)]` tests that asserted the old `draw_portal_connectors`/`portal_stub` right-column behavior.

- [ ] **Step 4: Run tests — verify green**

Run: `cargo test -p app up_connector_draws_updown_glyph_on_border_not_arrow`
Expected: PASS.
Run: `cargo test -p app`
Expected: all pass. Deleted portal-stub tests are expected; do not re-add them.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): render up/down as dotted lane connectors with border glyphs"
```

---

## Task 8: Docs (Phase 2)

**Files:**
- Modify: `docs/features/mapping.md` (the "Vertical connections laid out like N/S" bullet added in Task 5)

- [ ] **Step 1: Update the bullet**

Adjust the Task 5 bullet so it states up/down connectors are **routed through the same lane system as N/S** (crossing-eliminated, anchored at the middle of the top/bottom border, pushed aside by a reciprocal N/S), with the up/down symbol drawn on the room border — rendered as dotted lines rather than arrows. Keep the surrounding style.

- [ ] **Step 2: Verify**

Run: `cargo test -p app` and `cargo test -p mapper`
Expected: all pass (docs-only).

- [ ] **Step 3: Commit**

```bash
git add docs/features/mapping.md
git commit -m "docs(mapping): note up/down now route through the N/S lane system"
```

## Phase 2 notes for the implementer

- The trickiest part of Task 7 is mixing **dotted** up/down bodies with **solid** compass bodies in `render_lane_connectors`' shared per-cell mask. Prefer selecting the glyph set per-connector (draw up/down connector segments with the dotted set) over a global change; if the shared-mask junction logic makes per-cell selection hard, draw up/down connector bodies in a small dedicated pass using their `segs`, and keep compass rendering unchanged. Whatever you choose, compass connectors must render byte-identically to before.
- Do NOT change `side_for`, `route_all`, `planar_region`, or `mark_distorted` — Phase 2 keeps `grid_offset(Up/Down) == None`; only the lane router (`route_side` + `layout_offset` filter) sees up/down as sided.
