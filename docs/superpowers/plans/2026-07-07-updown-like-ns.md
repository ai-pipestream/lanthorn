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

## Task 9: Collapse matching up/down pairs (reciprocal), with N/S slot priority + both-end glyphs

Refinement after visual testing (see the spec's updated Phase 2 reciprocal rules). This REVISES Task 6's "never collapse" and Task 7's "departure-glyph only".

**Files:**
- Modify: `crates/mapper/src/route/mod.rs` (`back_edge_idx` pairing guard; `assign_side_slots` for N/S-priority ordering)
- Modify: `crates/app/src/render/map.rs` (`render_lane_connectors` far-end glyph for reciprocal up/down)
- Test: `crates/mapper/src/route/mod.rs` tests + `crates/app/src/render/map.rs` tests

**Interfaces:**
- Behavior: a matching `Up(A→B)`+`Down(B→A)` pair collapses to ONE `RoutedConnector` with `reciprocal = true`; an unmatched one-way up/down stays `reciprocal = false`. Up/down still NEVER pair with a compass edge, and compass pairing stays byte-identical. A reciprocal N/S connector outranks a reciprocal up/down for the center slot on a shared side. The layout weight of up/down is unchanged (still weight-1; do NOT touch the scoring/constraint code).

- [ ] **Step 1: Update the Task-6 pairing test and add the slot-priority + render tests**

The Task-6 test `reciprocal_updown_pair_is_not_collapsed` asserted a matching pair produces TWO connectors — that is now WRONG. Replace it with `reciprocal_updown_pair_collapses_to_one` asserting ONE connector with `reciprocal == true`. Add a mapper test that a room with BOTH a reciprocal N/S and a reciprocal up/down gives the N/S connector the center slot (slot 0) and the up/down connector a non-zero (fanned) slot on that side. Add an app render test that a reciprocal up/down connector draws the up glyph on the lower room's top border AND the down glyph on the upper room's bottom border. Keep the invariant test that up/down never earns reciprocal weight in LAYOUT (unchanged Phase-1 behavior). Use the real `route_lanes`/`RoutedConnector` API and the slot field names (`exit_slot`/`entry_slot`).

- [ ] **Step 2: Run — verify the new/updated tests fail**

Run: `cargo test -p mapper reciprocal_updown` and the new slot test.
Expected: FAIL (today a matching pair produces two connectors, `reciprocal == false`).

- [ ] **Step 3: Implement**

Read `back_edge_idx`, the pairing sites, `assign_side_slots`, and `render_lane_connectors` (reciprocal far-end block) FIRST.
1. In `back_edge_idx` (`route/mod.rs`), revise the Task-6 guard so: for a compass `c`, back-edge candidates stay compass-only (exclude up/down — preserves compass identity); for an Up/Down `c`, allow ONLY the opposite up/down edge between the same two rooms (`opposite(Up)=Down`) as the back-edge, so a matching pair collapses (`reciprocal=true`). An up/down `c` must never pair with a compass edge.
2. In `assign_side_slots` (`route/mod.rs`), order slot assignment on a side so a reciprocal N/S (compass) connector gets slot 0 before any up/down connector — the up/down yields to a fanned slot. (Sort the side's connectors so compass-reciprocal precede up/down; keep existing ordering among compass connectors.)
3. In `render_lane_connectors` (`crates/app/src/render/map.rs`), for a reciprocal up/down connector draw the up/down glyph at BOTH ends: the far-end (arrival) block that currently draws `arr_ch` for `reciprocal` connectors must, for an up/down connector, use the up/down glyph derived from `entry_dir` (the arrival direction) instead of an arrow. Departure glyph handling from Task 7 stays.

Do NOT change any layout scoring/constraint code (`mod.rs`/`constraints.rs`) — up/down must stay weight-1 in placement so a reciprocal N/S still shoves the up/down room aside.

- [ ] **Step 4: Run — verify green**

Run: `cargo test -p mapper` and `cargo test -p app` (the 4 Phase-2-deferred layout tests stay `#[ignore]`).
Expected: all pass; compass routing unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/route/mod.rs crates/app/src/render/map.rs
git commit -m "feat: collapse matching up/down pairs (reciprocal) with N/S slot priority"
```

## Phase 2 notes for the implementer

- The trickiest part of Task 7 is mixing **dotted** up/down bodies with **solid** compass bodies in `render_lane_connectors`' shared per-cell mask. Prefer selecting the glyph set per-connector (draw up/down connector segments with the dotted set) over a global change; if the shared-mask junction logic makes per-cell selection hard, draw up/down connector bodies in a small dedicated pass using their `segs`, and keep compass rendering unchanged. Whatever you choose, compass connectors must render byte-identically to before.
- Do NOT change `side_for`, `route_all`, `planar_region`, or `mark_distorted` — Phase 2 keeps `grid_offset(Up/Down) == None`; only the lane router (`route_side` + `layout_offset` filter) sees up/down as sided.

---

# Phase 3 — Refinements after visual testing (SQ-0219 de-dup, #1 shove-not-cross, #3 lock-in)

See the spec's "Phase 3" section. Three refinements from visual testing of Phase 2. Do these AFTER Tasks 1-9. Quests: SQ-0216 (Tasks 10, 13, 14) and SQ-0219 (Tasks 11, 12).

## Phase 3 Global Constraints (in addition to the top-of-plan constraints)

- **#1 is scoped to up/down-involved crossings only.** Compass-vs-compass crossing behavior MUST stay byte-identical. `illegal` stays the hard primary key of the tidy — a move that reduces up/down crossings but raises illegal overlaps is rejected.
- **SQ-0219 suppresses the up/down connector but the room's up/down glyph MUST still show in every view** (Boxes/numbers/labels), so vertical access still reads.
- **In/Out stay ignored** (never routed) and keep their room mid-slot icons — do not change In/Out handling.
- **#3 is a lock-in only** — no behavior change; just a regression test.
- Compass routing/layout/rendering stays byte-identical (Phase 1/2 invariants hold).

---

## Task 10: Lock in "up glyph on north border, down glyph on south border" (#3, regression only)

**Files:**
- Test: `crates/mapper/src/route/mod.rs` tests (router-level side invariant)
- Test: `crates/app/src/render/map.rs` tests (render-level border-row invariant)

**Interfaces:**
- Consumes: `crate::router::route_side` (Task 6), the reciprocal up/down rendering (Task 9). No production code changes — this task is a pure regression guard confirming an already-true property.

- [ ] **Step 1: Write the regression tests**

Add to the `#[cfg(test)]` module in `crates/mapper/src/route/mod.rs`:

```rust
#[test]
fn route_side_puts_up_on_top_and_down_on_bottom() {
    use crate::direction::Direction;
    use crate::router::route_side;
    use crate::router::Side;
    // Up always departs the NORTH (top) border; Down always the SOUTH (bottom).
    assert_eq!(route_side(Direction::Up), Some(Side::Top));
    assert_eq!(route_side(Direction::Down), Some(Side::Bottom));
}
```

(Match the real import paths for `Side`/`route_side` — adjust the `use` lines to wherever they live if these are wrong, but do not change the assertions.)

Add to the tests module in `crates/app/src/render/map.rs` (match how the neighboring render tests build a graph, render at Boxes zoom, and scan the buffer):

```rust
#[test]
fn reciprocal_updown_glyphs_sit_on_north_and_south_borders() {
    // A at origin, B directly north, joined by a reciprocal Up/Down pair.
    // The up glyph must land on a TOP border row (north side) and the down glyph
    // on a BOTTOM border row (south side) — never swapped, never on a left/right side.
    use mapper::direction::Direction;
    use mapper::graph::MapGraph;

    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::Up, 2);
    g.add_edge(2, Direction::Down, 1);

    let state = AppState::default();
    let rm = mapper::render::render(&g);
    let area = Rect::new(0, 0, 60, 30);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    // Find the up glyph and the down glyph, record their rows.
    let up = state.symbols.portal.up;    // default '↑'
    let down = state.symbols.portal.down; // default '↓'
    let mut up_row = None;
    let mut down_row = None;
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let s = buf.get(x, y).symbol();
            if s.chars().next() == Some(up) { up_row = Some(y); }
            if s.chars().next() == Some(down) { down_row = Some(y); }
        }
    }
    let (up_row, down_row) = (up_row.expect("up glyph present"), down_row.expect("down glyph present"));
    // B is north of A, so the up glyph (on B's south / A's north border region) must be
    // strictly below the down glyph? No — the up glyph marks the LOWER room's top border
    // and the down glyph marks the UPPER room's bottom border, and the upper room is north
    // (smaller y). So the down glyph (upper room, bottom border) is ABOVE the up glyph
    // (lower room, top border): down_row < up_row.
    assert!(down_row < up_row,
        "down glyph (upper room's south border) sits above the up glyph (lower room's north border): down_row={down_row} up_row={up_row}");
}
```

(The `symbols.portal.up/down` accessors and `AppState::default()` Boxes setup must match the real API — mirror the Task 7/9 render tests exactly. If those tests recenter/scroll, do the same here. The essential assertion is the vertical ordering of the two glyphs; if the exact rows differ from the comment's reasoning once you see the real geometry, keep the assertion that encodes "up glyph on the lower room's top border, down glyph on the upper room's bottom border" and fix the direction of the inequality to match the real layout — do not weaken it to "just present".)

- [ ] **Step 2: Run — verify they pass as-is (property already holds)**

Run: `cargo test -p mapper route_side_puts_up_on_top_and_down_on_bottom`
Run: `cargo test -p app reciprocal_updown_glyphs_sit_on_north_and_south_borders`
Expected: BOTH PASS with no production change (this is a lock-in of already-correct behavior). If the render test fails, the glyph geometry differs from the reasoning above — adjust the inequality to match the real north/south layout (still asserting up=north-room-top, down=south... i.e. the upper room is north), and note it; do NOT change production code unless the glyphs are genuinely on the wrong (swapped or left/right) borders, which would make this a real bug to escalate.

- [ ] **Step 3: Commit**

```bash
git add crates/mapper/src/route/mod.rs crates/app/src/render/map.rs
git commit -m "test: lock in up=north-border / down=south-border invariant (SQ-0216)"
```

---

## Task 11: Suppress the up/down connector when a compass edge shares the room pair (SQ-0219, mapper)

**Files:**
- Modify: `crates/mapper/src/route/mod.rs` (`route_topology_with` working set ~635; add a pre-pass computing compass-covered pairs; skip up/down edges on those pairs)
- Test: `crates/mapper/src/route/mod.rs` tests

**Interfaces:**
- Consumes: `grid_offset` (compass test) and the existing `route_lanes`/`RoutedConnector` API.
- Produces: when an unordered room pair has ≥1 compass edge (`grid_offset(dir).is_some()`), any Up/Down edge on that same pair produces **no** connector (no trunk, no merge stub). Pairs with only up/down edges are unaffected. In/Out unchanged (never routed).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)]` module in `crates/mapper/src/route/mod.rs`:

```rust
#[test]
fn compass_edge_suppresses_updown_connector_on_same_pair() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    // A--North-->B AND A--Up-->B on the same pair: only the compass path is drawn.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::N, 2);
    g.add_edge(1, Direction::Up, 2);

    let plan = route_lanes(&g);
    let updown: Vec<_> = plan.connectors.iter()
        .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
        .collect();
    assert_eq!(updown.len(), 0, "the up/down connector is suppressed when a compass edge shares the pair");
    let compass: Vec<_> = plan.connectors.iter()
        .filter(|c| matches!(c.exit_dir, Direction::N | Direction::S))
        .collect();
    assert_eq!(compass.len(), 1, "the compass connector is still drawn");
}

#[test]
fn lone_updown_pair_is_unaffected_by_suppression() {
    use crate::direction::Direction;
    use crate::graph::MapGraph;

    // Only an Up edge, no compass edge on this pair: the up/down connector survives.
    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::Up, 2);

    let plan = route_lanes(&g);
    let updown: Vec<_> = plan.connectors.iter()
        .filter(|c| matches!(c.exit_dir, Direction::Up | Direction::Down))
        .collect();
    assert_eq!(updown.len(), 1, "a pair with no compass edge keeps its up/down connector");
}
```

- [ ] **Step 2: Run — verify the first fails**

Run: `cargo test -p mapper compass_edge_suppresses_updown_connector_on_same_pair lone_updown_pair_is_unaffected_by_suppression`
Expected: `compass_edge_suppresses...` FAILS (today the up/down edge still produces a connector/merge-stub); `lone_updown...` PASSES (guards the no-regression case).

- [ ] **Step 3: Implement**

Read `route_topology_with` (working-set filter ~635, the per-edge loop, `back_edge_idx`, and the `trunk_points`/merge-stub branch ~656-683) FIRST. Then:

- Before the per-edge routing loop, compute the set of unordered pairs covered by a compass edge:
  ```rust
  use std::collections::BTreeSet;
  let compass_pairs: BTreeSet<(RoomId, RoomId)> = graph.connections().iter()
      .filter(|c| grid_offset(c.dir).is_some())
      .map(|c| { let (a, b) = (c.from, c.to); if a <= b { (a, b) } else { (b, a) } })
      .collect();
  ```
  (Match the real field names for a connection's endpoints — the working set already derives a pair key at ~656; reuse that exact key derivation so the `BTreeSet` key and the loop's pair key are identical. Confirm the `RoomId` type and `grid_offset` import.)
- In the per-edge loop, when the current edge `c` is Up/Down (`matches!(c.dir, Direction::Up | Direction::Down)`) and its pair key is in `compass_pairs`, `continue` — emit no connector for it.

Keep In/Out excluded as before (they never enter the working set). Do not change compass or lone-up/down handling.

- [ ] **Step 4: Run — verify green**

Run: `cargo test -p mapper compass_edge_suppresses_updown_connector_on_same_pair lone_updown_pair_is_unaffected_by_suppression`
Expected: both PASS.
Run: `cargo test -p mapper`
Expected: all pass (the 4 app-side deferred tests are in the app crate, unaffected here). If a routing test asserted a connector for an up/down edge that shares a compass pair, that is the intended change — update it and note it.

- [ ] **Step 5: Commit**

```bash
git add crates/mapper/src/route/mod.rs
git commit -m "feat(mapper): compass edge suppresses the up/down connector on the same pair (SQ-0219)"
```

---

## Task 12: Keep the room's up/down glyph when its connector is suppressed (SQ-0219, app)

**Files:**
- Modify: `crates/app/src/render/map.rs` (`draw_portal_icons` ~1214-1318 — the default/numbers-view branches ~1297-1315)
- Test: `crates/app/src/render/map.rs` tests

**Interfaces:**
- Consumes: Task 11 (up/down connector suppressed for compass-covered pairs); `rm.plan.connectors` and `rm.edges`.
- Behavior: in the default and numbers views, a room joined to a neighbor by both a compass and an up/down edge (whose up/down connector Task 11 suppressed) still shows the room-level up/down border glyph. When an up/down connector IS present (lone up/down pair), no room-level glyph is added (the connector carries it) — no double-draw.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `crates/app/src/render/map.rs` (mirror the neighboring render-test setup):

```rust
#[test]
fn deduped_updown_pair_still_shows_room_glyph() {
    // A--North-->B AND A--Up-->B: Task 11 suppresses the up/down connector, but the
    // rooms must still show the up/down glyph so vertical access reads.
    use mapper::direction::Direction;
    use mapper::graph::MapGraph;

    let mut g = MapGraph::new();
    g.upsert_room(1, "A".into());
    g.upsert_room(2, "B".into());
    g.set_pos(1, (0, 0));
    g.set_pos(2, (0, -1));
    g.add_edge(1, Direction::N, 2);
    g.add_edge(1, Direction::Up, 2);

    let state = AppState::default(); // default/Boxes view, numbers per default
    let rm = mapper::render::render(&g);
    let area = Rect::new(0, 0, 60, 30);
    let mut buf = Buffer::empty(area);
    render_map(&rm, &state, area, &mut buf);

    let up = state.symbols.portal.up;
    let text: String = buf.content.iter().flat_map(|c| c.symbol().chars()).collect();
    assert!(text.contains(up), "the up glyph still shows on the room border even though the connector was suppressed");
}
```

- [ ] **Step 2: Run — verify it fails**

Run: `cargo test -p app deduped_updown_pair_still_shows_room_glyph`
Expected: FAIL — with the connector suppressed (Task 11) and the default view drawing no room-level up/down icon, the up glyph is absent.

- [ ] **Step 3: Implement**

Read `draw_portal_icons` (~1214-1318), especially the numbers/default branches (~1297-1315) and the portal-label branch (~1273-1286) that already draws the top=↑ / bottom=↓ border glyphs. Then, in the default/numbers branches:

- For an up/down stub edge (`rm.edges`, `is_stub`, `dir ∈ {Up, Down}`), determine whether its pair was de-duped: the plan has a **compass** connector for that pair but **no up/down** connector. Compute the pair key the same way Task 11 does. If de-duped, draw the room-level up/down border glyph for that direction (top border cell for Up, bottom border cell for Down — reuse the portal-label branch's placement at `bx + BOX_W/2`), guarded so it is not drawn when an up/down connector for the pair is present (lone up/down pair — the connector already shows it).

Keep In/Out/Unknown handling exactly as-is.

- [ ] **Step 4: Run — verify green**

Run: `cargo test -p app deduped_updown_pair_still_shows_room_glyph`
Expected: PASS.
Run: `cargo test -p app up_connector_draws_updown_glyph_on_border_not_arrow`
Expected: PASS (lone up/down pair still shows exactly one up glyph via the connector — confirm no double-draw).
Run: `cargo test -p app`
Expected: all pass (the 4 deferred tests remain `#[ignore]` until Task 14).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): keep room up/down glyph when the connector is suppressed (SQ-0219)"
```

---

## Task 13: Up/down paths shove rooms apart instead of crossing (#1, scoped up/down crossing pressure)

**Files:**
- Modify: `crates/app/src/render/map.rs` (`overlap_stats` ~1546-1589 add third field; `cleanup_overlaps_observed` gate ~1672 + acceptance ~1705 + key ~1708/1682; `compact_empty_lines_observed` revert guard ~1862)
- Test: `crates/app/src/render/map.rs` tests

**Interfaces:**
- Consumes: up/down lane connectors (Task 6) already feed `plan.connectors`.
- Produces: `overlap_stats` returns `(illegal, updown_crossings, crossings)` (a 3-tuple; `updown_crossings ⊆ crossings`). The tidy moves rooms to drive `updown_crossings` toward 0 even when `illegal == 0`, at second priority after `illegal`. Compass-vs-compass crossings are untouched.

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `crates/app/src/render/map.rs`. Build a fixture where an up path is forced to cross a horizontal compass corridor unless a room moves (mirror the neighboring cleanup-test construction — `run_tidy_pipeline`/`cleanup_overlaps` on a `MapGraph`). The essential assertions:

```rust
#[test]
fn updown_crossing_is_counted_but_compass_crossing_is_not() {
    // Construct one graph whose plan has an up/down connector crossing a horizontal
    // compass connector, and one whose plan has only a compass×compass crossing.
    // overlap_stats must count the first in updown_crossings and NOT the second.
    // (Build both via the same helpers the existing overlap_stats tests use.)
    // ... assert stats_updown_case.1 >= 1  (updown_crossings field)
    // ... assert stats_compass_case.1 == 0
}

#[test]
fn tidy_shoves_rooms_to_clear_an_updown_crossing() {
    // A vertical up path crosses a horizontal corridor with zero illegal overlaps.
    // After the tidy, the up/down crossing is cleared by moving a room (updown_crossings == 0),
    // and no illegal overlap was introduced.
    // ... run the tidy; assert overlap_stats(...).1 == 0 and .0 == 0
}

#[test]
fn tidy_does_not_move_rooms_for_a_pure_compass_crossing() {
    // Two compass connectors cross cleanly, zero illegal overlaps. The tidy must NOT
    // relocate rooms for it (compass crossing stays a tiebreak, not a mover).
    // ... snapshot positions; run tidy; assert positions unchanged.
}
```

Write these concretely against the real `overlap_stats`/`render_overlap_stats` signature and the real tidy entry points once you read them (Step 3). The three assertions are the contract; fill in the fixture construction to match the existing crossing/overlap tests in this file.

- [ ] **Step 2: Run — verify they fail (or don't compile against the old 2-tuple)**

Run: `cargo test -p app updown_crossing_is_counted_but_compass_crossing_is_not tidy_shoves_rooms_to_clear_an_updown_crossing tidy_does_not_move_rooms_for_a_pure_compass_crossing`
Expected: FAIL — `overlap_stats` is still a 2-tuple (no `updown_crossings`), and the tidy does not move rooms for a crossing.

- [ ] **Step 3: Implement**

Read `overlap_stats` (~1546-1589), `render_overlap_stats` (~1592-1596), `cleanup_overlaps_observed` (~1649-1733, gate ~1672, acceptance ~1705, `Key`/key ~1682/1708), `compact_empty_lines_observed` (~1834-1878, revert ~1862), and the `session.rs:1181` caller FIRST. Then:

1. `overlap_stats`: return `(usize, usize, usize)` = `(illegal, updown_crossings, crossings)`. When a cell is classified a crossing (the existing `[ns, ew]` branch ~1582-1583), also test whether **either** owning connector is up/down; track that by tagging each connector's up/down-ness as it is plotted (`matches!(conn.exit_dir, Direction::Up | Direction::Down)`), and if either owner of the crossing cell is up/down, increment `updown_crossings` in addition to `crossings`. `illegal` counting is unchanged.
2. `render_overlap_stats`: forward the 3-tuple.
3. Update the two existing 2-tuple call sites you find (the tests you keep, and `session.rs:1181` which reads `.0` — now still `.0`, unaffected, but confirm the tuple index).
4. `cleanup_overlaps_observed`:
   - Gate (~1672): `if base.0 == 0 && base.1 == 0 { break; }` (continue while illegal OR up/down crossings remain).
   - `Key` type (~1682): insert the up/down-crossing count as the SECOND element: `(usize /*illegal*/, usize /*updown_crossings*/, usize /*align_broken*/, usize /*broken*/, usize /*crossings*/, usize /*degree*/, RoomId, usize)`.
   - Acceptance (~1705): `if (s.0, s.1, s.2) < (base.0, base.1, base.2) {` (accept a move that lowers illegal, then up/down crossings, then general crossings).
   - Key construction (~1708): `let key: Key = (s.0, s.1, align_broken, broken, s.2, degree, id, move_idx);`
   (Adjust every `s.1`→`s.2` where the old code meant the general crossing count, and every new `s.1` means up/down crossings. Read carefully — the old 2-tuple `.1` was general crossings; it is now `.2`.)
5. `compact_empty_lines_observed` (~1862): revert if `s.0 > base.0 || s.1 > base.1` (do not let compaction re-introduce an up/down crossing). The old check was `s.0 > base.0` only.
6. `repair_directional_hints_observed`: it gates on `s.0 <= base.0` and uses crossings as a tiebreak — update its tuple indices (`.1`→`.2` for general crossings) so it still compiles and behaves the same; do NOT add up/down-crossing pressure there (cleanup owns the shove). Note the index shift in the report.

- [ ] **Step 4: Run — verify green**

Run: `cargo test -p app updown_crossing_is_counted_but_compass_crossing_is_not tidy_shoves_rooms_to_clear_an_updown_crossing tidy_does_not_move_rooms_for_a_pure_compass_crossing`
Expected: all PASS.
Run: `cargo test -p app`
Expected: all pass EXCEPT the 4 still-`#[ignore]`d deferred tests (Task 14 handles them). Do not un-ignore them here. If any non-ignored test regresses on a compass-only layout, that is a real bug in the index shift — fix it so compass behavior is byte-identical.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs crates/app/src/session.rs
git commit -m "feat(app): tidy shoves rooms to clear up/down path crossings (scoped, SQ-0216)"
```

---

## Task 14: Un-ignore and refresh the 4 deferred layout tests + real-game smoke test (#1 follow-through)

**Files:**
- Modify: `crates/app/src/render/map.rs` (remove the 4 `#[ignore = "Phase 2: up/down now feed overlap_stats…"]` annotations; update each test's expected layout to the post-shove correct layout)

**Interfaces:**
- Consumes: Task 13 (scoped up/down crossing pressure resolves the B/A decision as A-scoped). The four tests: `cleanup_clears_overlaps_without_knocking_aligned_rooms_off_row`, `cleanup_keeps_two_room_column_chain_aligned`, `repair_puts_78_west_of_180_after_retidy`, `compact_preserves_directional_order_no_overlap`.

- [ ] **Step 1: Remove one `#[ignore]` and inspect the failure**

For the first deferred test, delete its `#[ignore = "Phase 2…"]` attribute and run it:
Run: `cargo test -p app cleanup_clears_overlaps_without_knocking_aligned_rooms_off_row -- --nocapture`
Read the actual vs expected positions. Because these fixtures contain up/down edges, the up/down crossing pressure (Task 13) may now legitimately move a room. Decide: is the new layout correct (up/down path no longer crossing, alignment preserved where it should be)? If yes, this is an expected assertion update; if the new layout is WRONG (a compass alignment the test guards was broken), that is a Task-13 regression to fix first — escalate rather than "fix" the test to match bad output.

- [ ] **Step 2: Update the assertion to the correct post-shove layout**

Update the test's expected coordinates to the new correct layout, keeping the property the test's NAME guarantees (e.g. `keeps_two_room_column_chain_aligned` must still assert the chain shares a column). Do not rename the test or weaken its guaranteed property.

- [ ] **Step 3: Repeat for the other three**

Remove each remaining `#[ignore]` in turn, inspect, and update the expected layout (or escalate a genuine regression). After all four:
Run: `grep -rn '#\[ignore = "Phase 2' crates/app/`
Expected: no matches.

- [ ] **Step 4: Full suite green**

Run: `cargo test -p app` and `cargo test -p mapper`
Expected: ALL pass, no ignored layout tests remaining.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "test(app): un-ignore + refresh the 4 layout tests for scoped up/down shove (SQ-0216)"
```

- [ ] **Step 6: Controller real-game smoke test (outside the plan, per project practice)**

After Task 14, the controller runs the app on a vertical-heavy story (e.g. `stories/zork1-r88-s840726.z3`), walks a shaft, and confirms: up/down paths no longer cross other paths where the tidy could make room; a room joined by both a compass and an up/down edge draws a single compass path and still shows its up/down symbol; compass-only regions look unchanged. Layout unit tests share the implementation's assumptions, so this smoke test is the real oracle.

## Phase 3 notes for the implementer

- **Tuple-index shift is the trap in Task 13.** `overlap_stats` goes from `(illegal, crossings)` to `(illegal, updown_crossings, crossings)`. Every existing `.1` that meant "general crossings" becomes `.2`. Grep every consumer (`overlap_stats(`, `render_overlap_stats(`) and fix each — a missed index silently changes compass tidy behavior.
- **Scope discipline (Task 13).** The ONLY new mover is the cleanup pass reacting to `updown_crossings`. Do not add up/down-crossing pressure to `repair_directional_hints` or make general `crossings` a mover — that would churn compass layouts (the rejected "general crossings" option).
- **SQ-0219 pair key must match on both sides (Tasks 11-12).** The router's suppression key and the renderer's "was this de-duped?" key must derive the unordered pair identically, or the glyph and the connector disagree.
