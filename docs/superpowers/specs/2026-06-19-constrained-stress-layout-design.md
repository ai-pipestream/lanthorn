# Constrained Stress-Majorization Layout — Design Spec

**Date:** 2026-06-19
**Status:** Approved (design); ready for implementation plan
**Scope:** `mapper` crate layout engine only. No `zvm`/`app` API changes (room positions stay `(i32,i32)` grid cells).

## Goal

Replace lanthorn's greedy BFS grid layout with **constrained stress majorization**: minimize neato's stress objective (even, topology-respecting spacing) **subject to hard separation constraints** derived from the compass directions we already capture (N/S/E/W and diagonals). This guarantees compass directions are honored wherever geometrically feasible, pins global orientation (north = up), and degrades gracefully on contradictory maps by reusing the existing `distorted` flag.

## Background — current state

`crates/mapper/src/layout.rs::relayout_auto` clears all positions and re-derives them by BFS from the lowest-id root of each connected component, placing each neighbor at `pair_offset` grid deltas and spiral-searching `nearest_free_cell` on collision. This is fast and deterministic but produces overlaps and poor spacing on dense graphs, and has no global spacing objective. Positions are integer grid cells (`Room.pos: Option<(i32,i32)>`); the `app` renderer maps `cell → screen` via a per-zoom step, and persistence stores the grid cell. **All of that stays unchanged** — the new engine still outputs integer grid cells.

Helpers to reuse: `nearest_free_cell`, `occupied_cells`, `edge_is_satisfied`, `grid_offset`, `LayoutMode`. Helpers to retire from the Auto path: the `pair_offset`-based BFS placement (kept only as the deterministic *seed*, see Pipeline step 3).

## Decisions (from brainstorming)

- **Re-solve from scratch each turn** from a deterministic seed (not warm-started). Determinism is therefore mandatory: same graph ⇒ identical layout, every call.
- **Hard constraints** (constrained stress majorization), not a soft penalty blend.
- **Solve in continuous space, snap to the integer grid** at the end. Renderer/persistence/manual-nudge unchanged.
- **Replace the Auto algorithm in place.** `relayout_auto` keeps its signature and call sites; Manual mode and `nudge` are untouched.
- Cycle-breaker (dropped contradictory constraints) **feeds the existing `distorted` flag**.

## Architecture — new `crates/mapper/src/layout/` module directory

Convert `layout.rs` into a `layout/` module (its public surface — `relayout_auto`, `LayoutMode`, `occupied_cells`, `nearest_free_cell`, `edge_is_satisfied`, `pair_offset` — re-exported from `layout/mod.rs` so `crate::layout::X` paths keep working).

- **`layout/vpsc.rs`** — Variable Placement with Separation Constraints: the 1-D projection solver. Pure, no graph knowledge. The one intricate piece.
- **`layout/constraints.rs`** — build axis-separated separation constraints from compass edges; deterministically DAG-ify each axis (drop cycle-closing constraints); report dropped edges.
- **`layout/stress.rs`** — constrained stress-majorization driver: all-pairs BFS distances, SMACOF iteration, per-axis VPSC projection.
- **`layout/mod.rs`** — `relayout_auto` orchestration, grid snapping, overlap resolution, component packing, `distorted` marking, plus the retained helpers (`nearest_free_cell`, `occupied_cells`, `edge_is_satisfied`, `pair_offset`, `LayoutMode`). The **current greedy BFS placement is extracted verbatim into `seed_layout(graph) -> BTreeMap<RoomId,(i32,i32)>`** and reused only to seed the solver (step 3) and as the `MAX_NODES` fallback — its behavior is preserved, just no longer the final output.

## Data structures (solver-internal, `f64`)

```text
Variable { desired: f64, weight: f64, position: f64, offset: f64, block: usize }
Constraint { left: usize, right: usize, gap: f64, active: bool }   // right - left >= gap
Block { vars: Vec<usize>, position: f64, active_in: Vec<usize>, active_out: Vec<usize> }
```

Node identity inside the solver is a dense index `0..n` (one component at a time); a `Vec<RoomId>` maps index → room id for write-back.

## Constraint construction (`constraints.rs`)

For each connection with `grid_offset(dir) = Some((dx, dy))` (cardinals + diagonals; `Up/Down/In/Out/Unknown` contribute none):

- `dx > 0` (east component): **X** constraint `x[dest] - x[origin] >= GAP`
- `dx < 0` (west): `x[origin] - x[dest] >= GAP`
- `dy > 0` (south): **Y** constraint `y[dest] - y[origin] >= GAP`
- `dy < 0` (north): `y[origin] - y[dest] >= GAP`

(`GAP = 1.0`, one ideal length.) A diagonal contributes one X and one Y constraint.

**DAG-ify (deterministic cycle break):** process connections in array order. For each axis maintain a constraint graph; add a candidate constraint only if it does **not** create a directed cycle (incremental cycle check via DFS/visited over current edges). A constraint that would close a cycle is **dropped**, and its connection index is recorded in a `dropped: BTreeSet<usize>` set. An edge whose any axis constraint is dropped is a distortion candidate. This guarantees each axis's constraint set is a DAG ⇒ feasible.

## VPSC solver (`vpsc.rs`)

Solve, per axis, the projection QP:

```
minimize  Σ_i weight_i · (position_i − desired_i)²
subject to  position[c.right] − position[c.left] ≥ c.gap   for all constraints c
```

Algorithm (Dwyer/Marriott block-merging — the standard VPSC `project`):

1. Each variable in its own singleton block; `block.position = desired`.
2. **satisfy:** repeatedly find the most-violated constraint (`gap − (pos[right] − pos[left]) > tol`); merge the two blocks containing its endpoints into one block, recomputing the merged block's optimal position as the weight-weighted average of `desired_i − offset_i`, with per-variable offsets fixed by the active constraint chain. Mark the merging constraint `active`. Stop when no constraint is violated beyond `tol`.
3. **split (improve):** for each block, compute Lagrange multipliers of its active constraints; if any is negative (the constraint is pulling against the objective), split the block there and re-position the two halves. Repeat until no negative multipliers.
4. Write back `position_i = block.position + offset_i`.

Determinism: "most-violated" ties broken by lowest constraint index; block iteration in index order; `tol = 1e-9`. Complexity ≈ `O(c log c)` per axis per iteration.

## Stress-majorization driver (`stress.rs`)

Per connected component (indices `0..n`):

1. **Distances:** BFS from every node over the *undirected* adjacency ⇒ `d_ij` = hop count (finite within a component). `w_ij = 1 / d_ij²` for `i ≠ j`.
2. **Seed:** deterministic initial positions from the existing greedy grid placement (already roughly oriented), in ideal-length units.
3. **Iterate** `ITERS` times (default 60), and for each axis `a ∈ {x, y}` **in that fixed order**:
   - Unconstrained stress target (Guttman/localized SMACOF) for each `i`:
     `z_i = ( Σ_{j≠i} w_ij · ( p_j[a] + d_ij · (p_i[a] − p_j[a]) / ‖p_i − p_j‖ ) ) / ( Σ_{j≠i} w_ij )`
     (use full 2-D `‖p_i − p_j‖`; guard `‖·‖ = 0` by skipping the term).
   - Build `Variable`s with `desired = z_i`, `weight = Σ_j w_ij`; project onto axis `a`'s constraints via VPSC; assign results back to `p_i[a]`.
4. Optional early-exit if the max coordinate move in an iteration `< 1e-6` (deterministic; purely a speed optimization).

## Orchestration, snapping, components (`layout/mod.rs::relayout_auto`)

1. Clear positions. Find connected components (existing logic), ascending root-id order.
2. For each component: build constraints (record dropped edges globally), run the stress driver ⇒ continuous positions.
3. **Snap to grid:** positions are in ideal-length (≈ one cell) units ⇒ round each coord to nearest `i32`.
4. **Resolve residual overlaps:** in ascending room-id order, if a cell is taken, move to `nearest_free_cell`.
5. **Pack components:** place component *k* so its grid bounding box sits to the right of component *k−1* with a 1-cell gap (deterministic by root id).
6. **Anchor:** translate the whole layout so the lowest-id room is at `(0,0)` (stable reference; matches current behavior).
7. **Mark `distorted`:** for each connection, `distorted = (index ∈ dropped) OR (grid_offset(dir).is_some() AND !edge_is_satisfied(...))` evaluated on the final grid. Non-compass edges: `distorted = false`.

## Parameters (constants, tunable)

`GAP = 1.0`, `IDEAL_LENGTH = 1.0`, `ITERS = 60`, `tol = 1e-9`, early-exit move `< 1e-6`. A hard node cap (e.g. skip stress and fall back to the seed grid above `MAX_NODES = 400`) bounds worst-case `O(ITERS · n²)` cost; document the fallback.

## Testing strategy

- **`vpsc.rs`:** hand-computed projections — single constraint pushes apart to exactly `gap`; chain of three; already-feasible input unchanged; weights bias the merged position correctly.
- **`constraints.rs`:** each direction → correct axis/sign; diagonal → both axes; contradictory loop (`A→N→B→N→C→N→A`) drops exactly one constraint and records its edge.
- **`stress.rs` / end-to-end `relayout_auto`:** `A→N→B` ⇒ B strictly north (`y_B < y_A`); `A→E→B` ⇒ east; combined `A→N→B` + `B→W→A` ⇒ B north-east; reciprocal `A→N→B`+`B→S→A` ⇒ B due north; **no two rooms share a cell** (random-walk + dense fixtures); **determinism** (two calls byte-identical); **orientation pinned** (run twice on the same graph, north stays −y; no rotation); contradictory loop marks ≥1 edge `distorted` without overlap; disconnected components all placed without overlap.
- Adapt existing `layout.rs` tests: exact-position assertions become directional/relational assertions where the new optimum differs, keeping the no-overlap and distorted-on-contradiction guarantees.

## Out of scope / non-goals

- No change to `app` rendering, persistence, IFID, save/restore, or the Graphviz DOT export.
- No continuous (sub-cell) positions in the stored model.
- No warm-start/incremental layout (explicitly chosen: from-scratch each turn).
- No edge-routing changes — the `app` connector router is unaffected; it consumes whatever grid cells this produces.

## Risks

- **VPSC correctness** is the main risk (the block split/merge is subtle). Mitigation: isolate in `vpsc.rs` with thorough unit tests against hand-computed cases before wiring into the driver.
- **Cost** on very large maps: bounded by `MAX_NODES` fallback to the seed grid.
- **Test churn** in `layout.rs`: existing exact-position tests must be re-expressed relationally; semantics (direction, no-overlap, distorted-on-contradiction) are preserved.
