# Constraint-Solver Re-tidy Engine — Design Spec

**Date:** 2026-06-21
**Branch:** `constraint-relayout`
**Status:** Approved (design) — awaiting spec review
**Supersedes (re-tidy layout only):** the longest-path sort as the *default* re-tidy engine (the sort is retained as a large-graph fallback).
**Resurrects/adapts:** `2026-06-19-constrained-stress-layout-design.md` (the VPSC + SMACOF engine deleted in `824f8eb`).

## Goal

Make the on-demand **re-tidy** layout (`relayout_auto`, invoked by `Shift+R`/`Ctrl+T`)
produce a properly compact, low-distortion map by replacing the per-axis longest-path sort
with **constrained stress majorization** (VPSC + SMACOF). Stress majorization pulls
connected rooms toward their ideal graph-distance on **both** axes simultaneously, so
single-axis edges stay straight and dense clusters (the Zork "house") lay out compactly —
the structural limitation the independent-axis sort cannot overcome. The longest-path sort
is kept as a fast fallback for very large graphs.

## Background

Phase 1 deleted the constrained-stress engine (`layout/vpsc.rs`, `stress.rs`,
`constraints.rs`) and replaced it with per-axis longest-path layering. The sort is
deterministic and gets compass *ordering* right, but because it assigns X and Y
independently it cannot keep connected rooms aligned on the perpendicular axis: a due-east
edge whose endpoints differ in row renders diagonal/distorted. The perpendicular-alignment
pass (added later) fixes pendant/free rooms but **cannot fix rooms constrained on both
axes** — the house core (`79/80/81/180/76`, plus interior `203/193`) stays distorted.

The deleted engine was never the real problem; the *architecture around it* was — it ran
**every turn** with fragile proxy-based repairs (octant / grid-BFS) that approximated the
renderer. lanthorn now has a clean two-regime model: stable per-turn **incremental
placement** + on-demand **re-tidy** + a **router-measured cleanup** in `app` that nudges
rooms against the REAL rendered plan. Running the constraint solver **only on re-tidy** and
feeding its output to that cleanup sidesteps every source of the original fragility.

## Decisions (from brainstorming)

1. **Constraint engine is the default re-tidy layout**; the longest-path sort is retained
   only as a fallback above `MAX_NODES` rooms (the solve is ~`O(ITERS·n²)`).
2. **No user-facing engine switching** (the pluggable-engines idea is deferred).
3. **Distortion policy: minimize, then mark.** The solver satisfies as many compass
   constraints as are jointly feasible on the grid; constraints that close a directed cycle
   on an axis are dropped and their edges marked `distorted` (drawn magenta), as today.
   No forced spreading/overlap to chase zero distortion.
4. Per-turn incremental placement, the app router-measured cleanup, distortion marking,
   and persistence are **unchanged**.

## Architecture

```
mapper::layout::relayout_auto(graph)        ← the re-tidy entry point (unchanged signature)
   if room_count ≤ MAX_NODES:
       stress::constrained_layout(graph)     ← NEW (resurrected VPSC + SMACOF)
   else:
       sort::sort_layout(graph)              ← kept fallback (incl. perpendicular alignment)
   → snap to integer grid, resolve residual overlaps, anchor lowest id at (0,0)
   → mark_distorted (records cycle-dropped + geometry-violating edges)
   (app, unchanged) render → cleanup_overlaps nudges rooms to a clean routed plan
```

New `crates/mapper/src/layout/` modules (resurrected and adapted):

- **`layout/vpsc.rs`** — Variable Placement with Separation Constraints: the 1-D projection
  QP solver (Dwyer/Marriott block merge/split). Pure, no graph knowledge. The intricate
  piece; isolated and unit-tested against hand-computed cases.
- **`layout/constraints.rs`** — build axis-separated separation constraints from compass
  edges; deterministically DAG-ify each axis (drop cycle-closing constraints); report
  dropped edge indices.
- **`layout/stress.rs`** — constrained stress-majorization driver: all-pairs BFS distances,
  SMACOF iteration, per-axis VPSC projection; exposes
  `constrained_layout(graph) -> BTreeMap<RoomId,(i32,i32)>`.

`layout/sort.rs` and `layout/incremental.rs` are **kept** unchanged.

## The engine (resurrected from `824f8eb`'s parent, adapted)

Per connected component (dense indices `0..n`):

1. **Distances:** BFS over the undirected planar adjacency ⇒ `d_ij` = hop count; weights
   `w_ij = 1/d_ij²`.
2. **Constraints (`constraints.rs`):** for each compass edge with `grid_offset = (dx,dy)`:
   `dx>0` → X constraint `x[dest] − x[origin] ≥ GAP`; `dx<0` → reversed; `dy>0` → Y
   `y[dest] − y[origin] ≥ GAP`; `dy<0` → reversed (`GAP = 1.0`). A diagonal contributes one
   X and one Y. **DAG-ify:** process edges in array order; add a constraint only if it does
   not create a directed cycle on that axis (incremental reachability check); a
   cycle-closing constraint is **dropped** and its connection index recorded in
   `dropped: BTreeSet<usize>`.
3. **Seed:** initial positions from `sort::sort_layout` (already roughly compass-ordered),
   in ideal-length units — a good, deterministic starting point.
4. **Iterate `ITERS` times** (default 60); each iteration, for each axis `a ∈ {x,y}` in
   fixed order: compute the localized SMACOF stress target `z_i` for every node, then
   **project** onto axis `a`'s separation constraints via VPSC, writing back `p_i[a]`.
   Optional early-exit when the max coordinate move `< 1e-6` (speed only; deterministic).
5. **Snap** each continuous coordinate to the nearest `i32`.

**VPSC (`vpsc.rs`)** solves per axis: minimize `Σ w_i (pos_i − desired_i)²` subject to
`pos[right] − pos[left] ≥ gap`, via block merge (satisfy most-violated constraint, ties
broken by lowest index) + block split on negative Lagrange multipliers. `tol = 1e-9`.
Deterministic.

## Orchestration (`relayout_auto`, `layout/mod.rs`)

1. Find connected components (existing helper), ascending root-id order.
2. For each component (when `n ≤ MAX_NODES` total): build constraints (accumulate dropped
   indices), run the stress driver ⇒ continuous positions; snap to grid.
3. **Resolve residual same-cell collisions** in ascending room-id order via
   `nearest_free_cell` (existing).
4. **Pack components** left-to-right with a 1-cell gap (existing pattern).
5. **Anchor** the lowest-id room at `(0,0)`.
6. **`mark_distorted`** over the final grid: a compass edge is distorted if its index was
   dropped OR its final geometry violates its direction; non-compass edges never distorted.
7. Above `MAX_NODES`: skip the solve entirely and use `sort::sort_layout` (which already
   does its own packing/anchor); then `mark_distorted`.

## Parameters

`GAP = 1.0`, `IDEAL_LENGTH = 1.0`, `ITERS = 60`, `tol = 1e-9`, early-exit move `< 1e-6`,
`MAX_NODES = 400`. All constants, documented and tunable.

## Determinism

Same graph ⇒ byte-identical layout: integer grid output; component/edge/node iteration in
fixed (sorted / array-index) order; VPSC tie-breaks by lowest index; SMACOF axis order
fixed; seed is the deterministic sort; no RNG; no `HashMap` ordering affects positions.

## What is kept / removed

- **Kept:** `sort.rs` (+ perpendicular alignment) as the large-graph fallback;
  `incremental.rs`; the app `cleanup_overlaps`; `mark_distorted`; persistence; the
  `Shift+R`/`Ctrl+T` trigger and the per-turn light cleanup.
- **Added:** `vpsc.rs`, `constraints.rs`, `stress.rs`.
- **Removed:** nothing (the sort is demoted to fallback, not deleted).

## Testing strategy

`vpsc.rs` (hand-computed): single constraint pushes to exactly `gap`; chain of three;
already-feasible input unchanged; weights bias the merged block correctly.

`constraints.rs`: each direction → correct axis/sign; diagonal → both; contradictory loop
(`A→N→B→N→C→N→A`) drops exactly one constraint and records its edge index.

`stress.rs` / end-to-end `relayout_auto`:
- `A→N→B` ⇒ B strictly north; `A→E→B` ⇒ east; `A→N→B`+`B→W→A` ⇒ B north-east; reciprocal
  `A→N→B`+`B→S→A` ⇒ B due north (same column).
- **No two rooms share a cell** (random-walk + dense fixtures); **determinism** (two calls
  byte-identical); orientation pinned (north stays −y, no rotation).
- Contradictory loop marks ≥1 edge `distorted` without overlap; disconnected components all
  placed without overlap.
- **The win (regression):** on the A129 house graph (incl. interior `203/193/201`), total
  `distorted` count is **strictly lower** than the longest-path sort produces on the same
  graph, and the both-axes-constrained core (`79/80/81/180`) lays out more compactly
  (assert a concrete bound, e.g. bounding-box area or distorted-count threshold measured
  against the sort baseline in the same test).
- **Big-map fallback:** above `MAX_NODES`, `relayout_auto` uses the sort and still produces
  a valid, overlap-free layout (synthetic large graph).

`app`: the existing A129 cleanup gate (`cleanup_clears_a129_illegal_overlaps`) still passes
under the new engine (constraint layout → cleanup → 0 illegal overlaps).

## Risks

- **VPSC correctness** is the main risk (block split/merge is subtle). Mitigation: it is
  resurrected from previously-reviewed code and isolated with hand-computed unit tests
  before wiring into the driver.
- **Cost** on large maps: bounded by the `MAX_NODES` fallback.
- **Test churn**: a few `sort`/`mod` tests assert sort-specific positions; those keep
  testing the *fallback* path (call `sort_layout` directly) while end-to-end directional
  tests now exercise the constraint engine — update assertions to the new optimum where the
  layout legitimately differs, preserving every no-overlap and directional-sign guarantee.

## Out of scope

- User-facing engine switching / pluggable-engine registry (deferred).
- Warm-start / incremental constraint solving (re-tidy is a deliberate from-scratch
  reshuffle).
- Any change to the per-turn incremental regime, the app cleanup, persistence, the router,
  or the segment/diagonal/theming phases.
