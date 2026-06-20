# Crossing-Aware Layout Repair — Design Spec

**Date:** 2026-06-20
**Branch:** `fix-tui-runtime`
**Status:** Approved (approach) — awaiting spec review
**Builds on:** `2026-06-20-layout-routability-repair-design.md` (Milestone 5)

## Goal

Extend the layout's routability repair so it also **minimizes path crossings**, not
just room-blocking. Today every edge can route around rooms (0 unrouted), yet two
edges sharing the cramped `#74/#25/#76` corner still cross once (a legal 90° meet).
The repair is blind to this because it models only "is a lane free of rooms,"
never the paths themselves. After this change the hill-climb keeps improving past
the all-routable point — e.g. it shifts `#25` down so `25→76` is a straight west
shot and `74→25` comes down into `#25`'s top, dropping the crossing to zero.

## Background

Milestone 5 made overlaps and run-alongside structurally impossible (Tier-1 router
rules) and added `repair_routability`, whose score is `(unroutable_count,
displacement)`. On the real A129 map a diagnostic confirms: **0 true overlaps, 0
alongside-room, exactly 1 perpendicular crossing**. The crossing is *permitted* by
the renderer (perpendicular meets are legal) but the user wants it gone. The repair
stops at 0 unroutable and never considers the crossing.

The user's worked example — "move `#25` down and let `74→25` enter from the top" —
generalizes to: make placement **crossing-aware**. The arrival-from-top need not be
engineered: the renderer's existing `nearest_free_side` already picks `#25`'s top for
`74→25` once `#25` sits below `#74`'s row. So only the *placement* objective changes.

## Decisions (from brainstorming)

1. **Route model:** reuse the layout's grid BFS — have it return the shortest lane
   it finds, and count crossings among those grid paths. Stays entirely in `mapper`;
   it is an approximation of the renderer's A* (which may detour differently), good
   enough to drive placement.
2. **Goal:** minimize total crossings as far as the hill-climb reaches; a genuinely
   non-planar map keeps an unavoidable residual (rendered as a clean 90° meet).
3. Rooms may move more between turns (accepted) — placement stays deterministic and
   on integer cells.

## Architecture

A localized change inside `crates/mapper/src/layout/routability.rs`, plus the score
and candidate-set in `repair_routability`. `relayout_auto`, the renderer, and
arrival-side logic are untouched.

```
edge_routable(...) -> bool          ───►  edge_path(...) -> Option<Vec<cell>>
                                          (returns the BFS shortest lane, or None;
                                           edge_routable becomes edge_path(..).is_some())

repair_routability score:
   (unroutable_count, displacement)
        ──►  (unroutable_count, crossings, total_length, displacement)   [lexicographic]

repair_routability candidate set:
   endpoints of unroutable edges
        ──►  endpoints of unroutable edges  ∪  endpoints of edges that cross
             (empty set ⇒ done)
```

## Component 1 — `edge_path` (BFS that returns the lane)

`edge_routable` is generalized to return the path so the same search powers both the
routable check and the crossing model:

```rust
/// Shortest orthogonal lane from `origin` to `dest` whose first step is in `dir`,
/// avoiding every occupied cell except origin/dest, bounded to `bbox`. `None` if no
/// clean lane exists. Non-compass dirs (no first step) return `None` here — callers
/// exclude them from the drawn/crossing model (they are stubs, not paths).
fn edge_path(origin, dest, dir, occupied, bbox) -> Option<Vec<(i32,i32)>>
```

- Same BFS as today, but tracks a parent map and reconstructs the cell path on
  reaching `dest`. Deterministic (fixed neighbor order N/E/S/W, first-found).
- `edge_routable` is kept as `edge_path(...).is_some()` (so Milestone 5's tests and
  the `repair` routability term are unchanged in meaning).

## Component 2 — Crossing model

```rust
/// The set of drawn paths for the current positions, with reciprocal pairs collapsed.
fn drawn_paths(graph, pos) -> Vec<Vec<(i32,i32)>>
```

- One entry per **drawn connector**. A reciprocal pair (`a→b` and `b→a` both present)
  is the *same drawn path* in the renderer, so it contributes **one** entry — dedup by
  the unordered `{a,b}` pair, keeping the lower-origin-id direction. This prevents a
  reciprocal pair (e.g. `76↔74`) from registering as a fully-overlapping false crossing.
- Only compass edges with a `Some` path are included; unroutable edges contribute none.

```rust
/// Count of grid cells shared by two or more distinct drawn paths, excluding room
/// cells (a shared endpoint room is not a crossing). One perpendicular meet = 1;
/// a parallel overlap run = many (strongly penalized, though Tier-1 prevents those
/// in the final render — this only steers placement).
fn crossing_count(paths: &[Vec<(i32,i32)>], room_cells: &BTreeSet<(i32,i32)>) -> usize
```

`total_length` = Σ path lengths over `drawn_paths` (rewards short, direct routes;
a tiebreak below crossings so it never trades a crossing for a shorter path).

## Component 3 — Repair hill-climb (extended)

```rust
pub fn repair_routability(graph: &MapGraph, pos: &mut BTreeMap<RoomId,(i32,i32)>)
```

- **Score** (lexicographic, lower better): `(unroutable_count, crossings,
  total_length, displacement)`. Computed by routing all drawn edges under a trial
  `pos`, collapsing reciprocals, counting conflicts.
- **Candidate rooms** each pass: endpoints of currently-unroutable edges ∪ endpoints
  of edges whose path shares a non-room cell with another path. If this set is empty
  (0 unroutable AND 0 crossings), terminate. For each candidate room, try one-cell
  moves N/S/E/W into a free cell; keep the single strictly-improving move with the
  best score (first-found wins ties).
- **Termination:** strict improvement on a lexicographic tuple bounded below by
  `(0,0,minlen,0)`, plus `MAX_REPAIR_PASSES` backstop. Deterministic: fixed iteration
  order, integer score, no RNG.

Worked example (verified by hand): A129 after Milestone 5 sits at `#25(0,0)`,
`#74(-1,1)`, `#76(-1,2)`, 0 unroutable, 1 crossing. Candidate set = {endpoints of the
two crossing edges} = {25,74,76}. Moving `#25` to `(0,2)` gives 0 unroutable, **0
crossings** (`25→76` straight west; `74→25` down into `#25`'s top) — strict
improvement, accepted. `#25→(0,1)` is rejected (it re-blocks `25→76`: `#74` would be
due-west of `#25`, raising `unroutable_count`). The two objectives cooperate.

## Determinism

Unchanged guarantees from Milestone 5: connections are a `Vec`, positions a
`BTreeMap`, score is integer-only, moves accepted only on strict improvement with
first-found tiebreak. BFS path reconstruction is deterministic. Same graph →
identical layout. The existing `relayout_is_deterministic` test must keep passing.

## Edge Cases

- **Reciprocal pairs:** collapsed to one drawn path (above) — no false self-crossing.
- **Shared destination:** two edges into one room share that room cell; excluded from
  `crossing_count` (room cells aren't crossings).
- **Non-planar / unavoidable crossing:** hill-climb reaches a local min with crossings
  > 0; it terminates and leaves the residual (a clean 90° meet, already legal).
- **Non-compass stubs:** excluded from `drawn_paths` (no first step) — unaffected.
- **Approximation gap:** the BFS lane is the layout's model, not the renderer's A*
  route. Placement is steered by the model; the renderer remains the source of truth
  for the drawn pixels (and still never overlaps). Documented, accepted.

## Testing

`mapper` (`routability.rs`):
- `edge_path` returns a contiguous lane with the forced first step; `None` when
  blocked; `edge_routable` still agrees (`is_some()`).
- `crossing_count`: two perpendicular paths sharing one cell → 1; a reciprocal pair
  → 0 (collapsed); two disjoint paths → 0.
- **A129 corner (direct, on `pos`):** feed `{25:(0,0),74:(-1,1),76:(-1,2)}`; assert
  ≥1 crossing before repair, then `repair_routability` yields **0 unroutable AND 0
  crossings**, `#25` moved off row 0, no room overlap.
- **A129 full graph via `relayout_auto`:** assert final layout has 0 unroutable and
  0 crossings among drawn paths.
- Determinism preserved; reciprocal pair never drives rooms apart (regression: a
  simple `a↔b` reciprocal map keeps them adjacent).

`app` (render, end-to-end):
- Re-render the full A129 graph; assert 0 unrouted (DarkGray) cells **and** 0
  perpendicular-crossing ribbon cells in the corner.

## Out of Scope

- Porting the real A* router into the layout (the chosen model is the BFS lane).
- Making arrival-side an explicit decision variable (the renderer's existing
  `nearest_free_side` already lands `74→25` on the top once placement is fixed).
- Any change to persistence, DOT export, dump format, zvm bridge, or the renderer's
  Tier-1 rules / unrouted-line fallback.

## Limitations (accepted)

- Crossings are minimized via a grid-path proxy, not the renderer's exact routes, so
  a rare residual crossing in the drawn map is possible even at a model-0 minimum.
- "Minimize crossings" can relocate rooms more between turns as the map grows.
- Cost: each trial move now routes all drawn edges (E BFS per trial). Bounded by
  `MAX_NODES` and `MAX_REPAIR_PASSES`; revisit only if dense maps feel slow.
