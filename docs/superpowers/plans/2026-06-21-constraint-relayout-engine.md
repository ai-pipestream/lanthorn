# Constraint-Solver Re-tidy Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the re-tidy layout (`mapper::layout::relayout_auto`) compute positions with constrained stress majorization (VPSC + SMACOF) by default, keeping the longest-path sort as a large-graph fallback.

**Architecture:** Restore the previously-reviewed, deleted engine (`vpsc.rs`, `stress.rs`, `constraints.rs`) from git history, then rewire `relayout_auto` to: seed from the longest-path sort, build per-axis separation constraints from compass edges (dropping cycle-closing ones → `distorted`), run SMACOF with per-axis VPSC projection, snap to the integer grid, resolve residual overlaps, pack components, anchor, and mark distortion. Above `MAX_NODES` rooms, fall back to the sort. The per-turn incremental regime and the app router-measured cleanup are untouched.

**Tech Stack:** Rust 2021, workspace crate `mapper`. Tests are `#[cfg(test)]` modules. Run with `cargo test -p mapper`. Lint with `cargo clippy --workspace --all-targets`.

## Global Constraints

- Rooms stay on **integer grid cells** (`Room.pos: Option<(i32,i32)>`). The solver works in `f64` internally and snaps to `i32` at the end.
- **Determinism mandatory:** same graph ⇒ byte-identical layout. Fixed iteration order (sorted ids / array-index order); VPSC ties break by lowest constraint index; SMACOF axis order fixed (x then y); seed is the deterministic sort; no RNG. (`constraints.rs` uses a `HashMap` only for id→index *lookup*, never iteration — determinism holds.)
- **Rooms never overlap:** no two rooms share a cell after `relayout_auto`.
- **Distortion policy — minimize then mark:** constraints that close a directed cycle on an axis are dropped and their connection indices fed to `mark_distorted`; no forced spreading to chase zero distortion.
- **Constants (verbatim):** `GAP = 1.0`, `ITERS = 60`, `MAX_NODES = 400`.
- `relayout_auto(graph: &mut MapGraph)` keeps its exact public name and signature. Per-turn incremental placement, the app `cleanup_overlaps`, persistence, and the `Shift+R`/`Ctrl+T` trigger are unchanged.
- **`git show <rev>:<path> > <path>` is the restore mechanism** for the deleted files — it reads a blob from history into a new working file and is explicitly allowed. Do NOT run `git checkout`/`git restore`/`git stash` (they discard working-tree changes). Commit forward only.

---

### Task 1: Restore the engine and make it the default re-tidy

**Files:**
- Create (restore from history): `crates/mapper/src/layout/vpsc.rs`, `crates/mapper/src/layout/stress.rs`, `crates/mapper/src/layout/constraints.rs`
- Modify: `crates/mapper/src/layout/mod.rs` (declare modules; add constants; rewrite `relayout_auto`)

**Interfaces:**
- Consumes (from restored files):
  - `vpsc::Constraint { left: usize, right: usize, gap: f64 }` and `vpsc::solve_axis(desired: &[f64], weight: &[f64], constraints: &[Constraint]) -> Vec<f64>`.
  - `stress::all_pairs_dist(n: usize, adjacency: &[Vec<usize>]) -> Vec<Vec<f64>>` and `stress::stress_layout(n, dist: &[Vec<f64>], x_constraints: &[Constraint], y_constraints: &[Constraint], seed: &[(f64,f64)], iters: usize) -> Vec<(f64,f64)>`.
  - `constraints::build_axis_constraints(graph: &MapGraph, ids: &[RoomId], gap: f64) -> AxisConstraints { x: Vec<Constraint>, y: Vec<Constraint>, dropped: BTreeSet<usize> }`.
- Consumes (already in `mod.rs`): `connected_components(graph, &ids) -> Vec<Vec<RoomId>>` (pub(crate)), `nearest_free_cell`, `mark_distorted(graph, &BTreeSet<usize>)` (pub(crate)), `sort::sort_layout(graph) -> BTreeMap<RoomId,(i32,i32)>`.
- Produces: `relayout_auto` now uses the constraint engine for `≤ MAX_NODES` rooms, sort fallback above it.

- [ ] **Step 1: Restore the three engine files from history and run their own tests**

```bash
git show 824f8eb~1:crates/mapper/src/layout/vpsc.rs        > crates/mapper/src/layout/vpsc.rs
git show 824f8eb~1:crates/mapper/src/layout/constraints.rs > crates/mapper/src/layout/constraints.rs
git show 824f8eb~1:crates/mapper/src/layout/stress.rs      > crates/mapper/src/layout/stress.rs
```

These files are self-contained and carry their own `#[cfg(test)]` tests. Do not edit their bodies.

- [ ] **Step 2: Declare the modules and add constants in `mod.rs`**

In `crates/mapper/src/layout/mod.rs`, find the existing module declarations (`mod sort;`, `mod incremental;`) and add alongside them:

```rust
mod vpsc;
mod constraints;
mod stress;
```

Near the top of `mod.rs` (where module constants live), add:

```rust
/// Separation gap and ideal edge length (in grid cells).
const GAP: f64 = 1.0;
/// Fixed SMACOF iterations (determinism + bounded cost).
const ITERS: usize = 60;
/// Above this room count, skip the O(ITERS·n²) solve and use the longest-path sort.
const MAX_NODES: usize = 400;
```

- [ ] **Step 3: Run the restored engine unit tests (verify the restore compiled)**

Run: `cargo test -p mapper vpsc:: constraints:: stress:: 2>&1 | tail -20`
Expected: the restored tests pass (e.g. `single_constraint_pushes_to_gap`, `contradiction_drops_one_constraint`, `east_constraint_orders_x`, `deterministic`).

- [ ] **Step 4: Write the failing integration test**

In the `mod.rs` `#[cfg(test)]` module, add:

```rust
    #[test]
    fn constraint_engine_places_reciprocal_due_north() {
        // Reciprocal N/S pair → B due north of A (same column), via the constraint engine.
        let mut g = crate::graph::MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.add_edge(1, Direction::N, 2);
        g.add_edge(2, Direction::S, 1);
        relayout_auto(&mut g);
        let pa = g.room(1).unwrap().pos.unwrap();
        let pb = g.room(2).unwrap().pos.unwrap();
        assert!(pb.1 < pa.1, "B must be north of A: {pb:?} vs {pa:?}");
        assert_eq!(pb.0, pa.0, "no E/W constraint → B stays in A's column");
        // no overlap
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len());
    }
```

- [ ] **Step 5: Run it to confirm it fails**

Run: `cargo test -p mapper constraint_engine_places_reciprocal_due_north 2>&1 | tail -15`
Expected: FAIL — `relayout_auto` still uses the sort (B may not be in A's exact column / different placement) OR compiles but the column assertion fails. (If it happens to pass on the current sort, proceed; Step 6 is still required to switch the engine.)

- [ ] **Step 6: Rewrite `relayout_auto` to use the constraint engine with sort fallback**

Replace the entire body of `relayout_auto` in `mod.rs` with:

```rust
pub fn relayout_auto(graph: &mut MapGraph) {
    let mut ids: Vec<RoomId> = graph.rooms().map(|r| r.id).collect();
    ids.sort_unstable();
    if ids.is_empty() {
        return;
    }

    // Large graphs: skip the O(ITERS·n²) solve and use the longest-path sort.
    if ids.len() > MAX_NODES {
        let pos = sort::sort_layout(graph);
        for (&id, &p) in &pos {
            graph.set_pos(id, p);
        }
        mark_distorted(graph, &BTreeSet::new());
        return;
    }

    // Seed from the longest-path sort (deterministic, roughly compass-ordered).
    let seed = sort::sort_layout(graph);

    let components = connected_components(graph, &ids);
    let mut dropped_all: BTreeSet<usize> = BTreeSet::new();
    let mut final_pos: BTreeMap<RoomId, (i32, i32)> = BTreeMap::new();
    let mut occupied: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut pack_x: i32 = 0;

    for comp in &components {
        let n = comp.len();
        let index: BTreeMap<RoomId, usize> =
            comp.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // Local undirected adjacency for BFS distances.
        let mut adj = vec![Vec::new(); n];
        for c in graph.connections() {
            if let (Some(&a), Some(&b)) = (index.get(&c.origin), index.get(&c.dest)) {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        let dist = stress::all_pairs_dist(n, &adj);
        let ac = constraints::build_axis_constraints(graph, comp, GAP);
        dropped_all.extend(ac.dropped.iter().copied());

        let seed_local: Vec<(f64, f64)> = comp
            .iter()
            .map(|&id| {
                let p = seed.get(&id).copied().unwrap_or((0, 0));
                (p.0 as f64, p.1 as f64)
            })
            .collect();

        let cont = stress::stress_layout(n, &dist, &ac.x, &ac.y, &seed_local, ITERS);
        let mut snapped: Vec<(i32, i32)> =
            cont.iter().map(|&(x, y)| (x.round() as i32, y.round() as i32)).collect();

        // Pack this component to the right of the previous, top-aligned.
        let min_x = snapped.iter().map(|p| p.0).min().unwrap();
        let min_y = snapped.iter().map(|p| p.1).min().unwrap();
        for p in &mut snapped {
            p.0 += pack_x - min_x;
            p.1 -= min_y;
        }

        // Resolve residual same-cell collisions in ascending room-id order.
        let mut max_x_used = pack_x;
        for (i, &id) in comp.iter().enumerate() {
            let cell = nearest_free_cell(&occupied, snapped[i]);
            occupied.insert(cell);
            final_pos.insert(id, cell);
            max_x_used = max_x_used.max(cell.0);
        }
        pack_x = max_x_used + 2; // 1-cell gap between components
    }

    // Anchor the lowest-id room at (0,0) for a stable reference.
    if let Some(&(ax, ay)) = final_pos.get(&ids[0]) {
        for p in final_pos.values_mut() {
            p.0 -= ax;
            p.1 -= ay;
        }
    }

    for (&id, &p) in &final_pos {
        graph.set_pos(id, p);
    }
    mark_distorted(graph, &dropped_all);
}
```

Ensure `mod.rs` imports cover `BTreeMap`/`BTreeSet` (it already uses `std::collections::{BTreeMap, BTreeSet, VecDeque}`) — add any missing to the existing `use`.

- [ ] **Step 7: Run the integration test + the full mapper suite + clippy**

Run: `cargo test -p mapper 2>&1 | tail -25`
Expected: `constraint_engine_places_reciprocal_due_north` PASSES.

Some pre-existing `mod.rs` tests that call `relayout_auto` (`places_rooms_by_compass_offsets`, `east_room_is_east`, `combined_offset_places_northeast`, `reciprocal_places_one_step_north`, `orientation_pinned_north_is_up`, `rooms_never_overlap_random_walk`, `contradictory_geometry_marks_distorted_not_overlap`, `disconnected_component_gets_placed`, `relayout_is_deterministic`, `dynamic_*`, `repair_terminates_on_impossible_mutual_south`) assert directional-sign + no-overlap + determinism — these should still hold under the constraint engine. If any asserts an exact legacy cell the new optimum legitimately changes, convert that single assertion to the directional inequality already present in the same test; do NOT weaken or delete the no-overlap / directional / determinism assertions.

Then: `cargo clippy --workspace --all-targets 2>&1 | tail -15` — Expected: no new warnings from the restored/changed files.

- [ ] **Step 8: Commit**

```bash
git add crates/mapper/src/layout/vpsc.rs crates/mapper/src/layout/stress.rs crates/mapper/src/layout/constraints.rs crates/mapper/src/layout/mod.rs
git commit -m "feat(mapper): constrained stress-majorization re-tidy engine (sort fallback >MAX_NODES)"
```

---

### Task 2: Prove the win, the fallback, and re-point the alignment tests

**Files:**
- Modify: `crates/mapper/src/layout/sort.rs` (re-point two tests that call `relayout_auto` to `sort_layout` so they keep testing the *fallback*)
- Test: `crates/mapper/src/layout/mod.rs` `#[cfg(test)]` (the win + fallback tests)

**Interfaces:**
- Consumes: `relayout_auto`, `sort::sort_layout` (both in scope from `mod.rs`/`sort.rs`).

**Why:** `sort.rs`'s `a129_alignment_straightens_pendant_edges` and `free_interior_chain_aligns_to_anchor_row` currently call `relayout_auto`, which now runs the constraint engine — so they no longer test the sort's alignment. Re-point them to `sort_layout` to keep alignment coverage on the fallback, and add a new test that the constraint engine beats the sort on the A129 house graph (the whole point).

- [ ] **Step 1: Re-point the two alignment tests to the sort fallback**

In `crates/mapper/src/layout/sort.rs`, in both `a129_alignment_straightens_pendant_edges` and `free_interior_chain_aligns_to_anchor_row`, replace the call `crate::layout::relayout_auto(&mut g);` with:

```rust
        let pos = sort_layout(&g);
        for (&id, &p) in &pos {
            g.set_pos(id, p);
        }
        super::mark_distorted(&mut g, &std::collections::BTreeSet::new());
```

(These tests then assert the sort+alignment fallback behavior directly. `mark_distorted` is `pub(crate)` in `mod.rs`; `sort_layout` is in the same module.) Run them:

Run: `cargo test -p mapper a129_alignment_straightens_pendant_edges free_interior_chain_aligns_to_anchor_row 2>&1 | tail -10`
Expected: PASS (they now exercise `sort_layout`).

- [ ] **Step 2: Write the failing "win" test (constraint engine beats the sort on A129)**

In the `mod.rs` `#[cfg(test)]` module, add a helper that builds the full 18-room A129 house graph and a test comparing distortion:

```rust
    fn a129_house_graph() -> crate::graph::MapGraph {
        let mut g = crate::graph::MapGraph::new();
        for id in [25u16,26,27,74,75,76,77,78,79,80,81,136,143,180,193,201,203,239] {
            g.upsert_room(id, "r".into());
        }
        use Direction::*;
        for (o, d, dst) in [
            (180,N,81),(81,W,180),(180,W,78),(78,N,143),(143,E,77),(77,S,74),(74,S,76),
            (76,W,78),(143,W,78),(78,S,76),(76,N,74),(74,E,25),(25,W,76),(74,W,79),(79,E,74),
            (25,E,26),(26,Up,25),(78,E,75),(77,E,239),(239,N,77),(77,Unknown,180),(180,S,80),
            (80,W,180),(80,E,79),(79,S,80),(79,N,81),(81,E,79),(80,S,76),(76,Unknown,180),
            (79,Unknown,180),(75,S,81),(75,W,78),(75,E,77),(239,S,77),(77,W,75),(75,N,143),
            (143,S,75),(26,Down,27),(27,N,136),(136,SW,27),(27,Up,26),(26,Unknown,180),
            (79,W,203),(203,W,193),(193,E,203),(203,E,79),(203,Up,201),(201,Down,203),
        ] { g.add_edge(o, d, dst); }
        g
    }

    #[test]
    fn constraint_engine_beats_sort_distortion_on_a129() {
        // Distortion under the longest-path sort fallback (sort_layout + mark_distorted).
        let mut g_sort = a129_house_graph();
        let pos = sort::sort_layout(&g_sort);
        for (&id, &p) in &pos { g_sort.set_pos(id, p); }
        mark_distorted(&mut g_sort, &BTreeSet::new());
        let sort_distorted = g_sort.connections().iter().filter(|c| c.distorted).count();

        // Distortion under the constraint engine (the default relayout_auto).
        let mut g_cons = a129_house_graph();
        relayout_auto(&mut g_cons);
        let cons_distorted = g_cons.connections().iter().filter(|c| c.distorted).count();

        assert!(
            cons_distorted < sort_distorted,
            "constraint engine must reduce distortion vs sort: constraint={cons_distorted}, sort={sort_distorted}",
        );
        // And it must not overlap rooms.
        let cells: Vec<_> = g_cons.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no room overlap under the constraint engine");
    }

    #[test]
    fn relayout_is_deterministic_under_constraint_engine() {
        let mut a = a129_house_graph();
        let mut b = a129_house_graph();
        relayout_auto(&mut a);
        relayout_auto(&mut b);
        let pa: Vec<_> = a.rooms().map(|r| (r.id, r.pos)).collect();
        let pb: Vec<_> = b.rooms().map(|r| (r.id, r.pos)).collect();
        assert_eq!(pa, pb, "constraint engine must be deterministic");
    }
```

- [ ] **Step 3: Run the win + determinism tests**

Run: `cargo test -p mapper constraint_engine_beats_sort_distortion_on_a129 relayout_is_deterministic_under_constraint_engine 2>&1 | tail -15`
Expected: PASS. If `constraint_engine_beats_sort_distortion_on_a129` does NOT pass (constraint distortion ≥ sort), do NOT weaken the assertion — report it: it means the engine/seed needs investigation (e.g. more `ITERS`, or the seed is fighting the constraints), which is a real finding for the controller, not a test to soften.

- [ ] **Step 4: Write the big-map fallback test**

```rust
    #[test]
    fn large_graph_uses_sort_fallback_without_overlap() {
        // A chain longer than MAX_NODES forces the fallback path; it must still place
        // every room with no overlap (and not run the O(n²) solve).
        let mut g = crate::graph::MapGraph::new();
        let count = (super::MAX_NODES + 5) as u16;
        for id in 1..=count { g.upsert_room(id, "r".into()); }
        for id in 1..count { g.add_edge(id, Direction::E, id + 1); }
        relayout_auto(&mut g);
        let placed = g.rooms().filter(|r| r.pos.is_some()).count();
        assert_eq!(placed, count as usize, "every room placed via fallback");
        let cells: Vec<_> = g.rooms().filter_map(|r| r.pos).collect();
        let set: BTreeSet<_> = cells.iter().collect();
        assert_eq!(cells.len(), set.len(), "no overlap in the fallback layout");
    }
```

Run: `cargo test -p mapper large_graph_uses_sort_fallback_without_overlap 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Verify the app cleanup gate still passes under the new engine**

Run: `cargo test -p app cleanup_clears_a129_illegal_overlaps 2>&1 | tail -10`
Expected: PASS — the constraint layout still feeds a clean routed plan after `cleanup_overlaps`. If it FAILS (the constraint layout produces overlaps the radius-3 cleanup can't fix), report it — it is a real interaction finding, not a test to weaken.

- [ ] **Step 6: Full workspace suite + clippy**

Run: `cargo test --workspace 2>&1 | grep -E 'test result|FAILED' | tail` and `cargo clippy --workspace --all-targets 2>&1 | tail -10`
Expected: all green; no new warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/mapper/src/layout/sort.rs crates/mapper/src/layout/mod.rs
git commit -m "test(mapper): constraint engine beats sort distortion on A129; fallback + determinism"
```

---

## Self-Review

**Spec coverage:**
- ✅ Constraint engine (VPSC+SMACOF) as default re-tidy — Task 1 (restored `vpsc`/`stress`/`constraints` + `relayout_auto` rewrite).
- ✅ Per-axis separation constraints + DAG-ify (cycle → distorted) — restored `constraints.rs`, fed to `mark_distorted` via `dropped_all`.
- ✅ Seed from sort, SMACOF + VPSC projection, snap to grid — `relayout_auto` body.
- ✅ Sort fallback above `MAX_NODES` — Task 1 Step 6 branch + Task 2 Step 4 test.
- ✅ Minimize-then-mark distortion policy — cycle-dropped indices → `mark_distorted`.
- ✅ Determinism, no-overlap, integer grid — Global Constraints + tests (`relayout_is_deterministic_under_constraint_engine`, overlap asserts).
- ✅ The win (distortion < sort on A129) — Task 2 Step 2.
- ✅ Two-regime / app cleanup unchanged; app gate still passes — Task 2 Step 5.
- ✅ Sort + alignment kept as fallback (alignment tests re-pointed to `sort_layout`) — Task 2 Step 1.

**Placeholder scan:** none — every step has exact commands/code.

**Type consistency:** `vpsc::Constraint{left,right,gap}`, `vpsc::solve_axis`, `stress::all_pairs_dist`, `stress::stress_layout`, `constraints::build_axis_constraints`→`AxisConstraints{x,y,dropped}`, `connected_components`, `nearest_free_cell`, `mark_distorted`, `sort::sort_layout` — names/signatures match the restored files and the existing `mod.rs`. `GAP`/`ITERS`/`MAX_NODES` defined once in Task 1 Step 2 and used consistently.

**Risk flagged to controller:** if `constraint_engine_beats_sort_distortion_on_a129` or `cleanup_clears_a129_illegal_overlaps` fails, that is a real finding (engine/seed tuning or layout↔cleanup interaction), not a test to soften — Task 2 Steps 3 and 5 say so explicitly.
