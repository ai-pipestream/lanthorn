# Crossing-Aware Layout Repair — Design Spec

**Date:** 2026-06-20
**Branch:** `fix-tui-runtime`
**Status:** Approved (approach) — awaiting spec review
**Builds on:** `2026-06-20-layout-routability-repair-design.md` (Milestone 5)

## Goal

Extend the layout's routability repair so it also **minimizes path crossings** by
moving rooms — not just opening room-blocking. After this change the repair shifts a
room whose edges fan the same way (the `#74/#25/#76` corner) so the crossing
disappears: `#25` drops to `(0,2)`, `25→76` becomes a clean west shot, and `74→25`
comes down into `#25`'s top.

## Background & key finding

Milestone 5 made overlaps and run-alongside structurally impossible and added
`repair_routability` scored on `(unroutable_count, displacement)`. On the real A129
map a diagnostic shows **0 overlaps, 0 alongside-room, exactly 1 perpendicular
crossing** — `74→25` crosses `25→76` near `#25`. The repair stops at 0 unroutable and
never sees the crossing.

**A discarded model:** routing each edge with an independent grid BFS and counting
shared cells does NOT see this crossing. Traced directly: at `{25:(0,0), 74:(-1,1),
76:(-1,2)}` the two BFS shortest paths don't share a cell, so the model reports 0
crossings where the render shows 1. The crossing is a *render-granularity* effect
(21×11 boxes, departure/arrival anchors, sequential routing) that a 1-cell-per-room
grid model can't represent. So we do **not** model paths at all.

**The signal that works (empirically verified).** Rendering the real A129 graph with
`#25` forced to each position and counting crossing cells:

| `#25` position | rendered crossings |
|----------------|--------------------|
| `(0,0)` (current) | **1** |
| `(0,1)` | 0 (but re-blocks `25→76` routability) |
| `(0,2)` | 0 |
| `(0,3)` | 0 |
| `(1,2)` | 0 |

The crossing exists at exactly one spot, and the cause is purely geometric: at
`(0,0)` both of `#25`'s neighbors (`#74`, `#76`) lie in the **same compass octant**
(SW), so both edges fan into the same corner. At `(0,2)` they split (NW + W). This is
captured by a **per-room same-octant neighbor count** — sign geometry only, no
routing, no renderer replication.

## Decisions (from brainstorming, revised after the finding above)

1. **Crossing model:** per-room **side/octant conflict** — for each room, count pairs
   of neighbors that sit in the same compass octant `(sign(dx), sign(dy))`. This is
   the user-chosen "per-room side conflicts," made concrete as octant geometry.
2. **Minimize crossings** globally; a non-planar residual is acceptable.
3. **Moves must be multi-cell.** The greedy ±1 hill-climb cannot move `#25` from
   `(0,0)` to `(0,2)` because the intermediate `(0,1)` raises the primary
   (routability) term — a valley the climb can't cross one step at a time. Candidate
   moves therefore target any free cell within a small Chebyshev radius.
4. Rooms may move more between turns (accepted); placement stays deterministic and on
   integer cells.

## Architecture

A localized change inside `crates/mapper/src/layout/routability.rs`. `relayout_auto`,
the renderer, persistence, and arrival-side logic are untouched.

```
NEW  octant(dx,dy) -> (i8,i8)                         // (sign(dx), sign(dy))
NEW  side_conflicts(graph, pos) -> usize              // per-room same-octant neighbour pairs
NEW  conflict_rooms(graph, pos) -> BTreeSet<RoomId>   // rooms in any conflicting pair (room + the two neighbours)

repair_routability score:
   (unroutable_count, displacement)
        ──►  (unroutable_count, side_conflicts, displacement)   [lexicographic]

repair_routability candidate rooms:
   endpoints of unroutable edges
        ──►  endpoints of unroutable edges  ∪  conflict_rooms
             (empty ⇒ done)

repair_routability candidate moves:
   the four ±1 neighbours
        ──►  every free cell within Chebyshev radius MOVE_RADIUS (= 3)
```

`edge_routable` / `BBOX_MARGIN` / `MAX_REPAIR_PASSES` / `occupied_map` / `bbox_of` /
`unroutable_count` / `displacement` are unchanged from Milestone 5.

## Component 1 — Octant + conflict count

```rust
/// Compass octant of a direction vector: (sign(dx), sign(dy)), each in {-1,0,1}.
/// Two neighbours of a room with the same octant fan the same way → crossing pressure.
fn octant(dx: i32, dy: i32) -> (i8, i8) {
    (dx.signum() as i8, dy.signum() as i8)
}

/// A room's distinct neighbour rooms via DRAWN (compass) edges, either direction.
/// Reciprocal pairs collapse naturally (a neighbour is counted once).
fn neighbours(graph: &MapGraph, r: RoomId) -> BTreeSet<RoomId> { … }

/// Total per-room same-octant neighbour conflicts. For each room, for each unordered
/// pair of its placed neighbours, +1 if both lie in the same octant relative to it.
fn side_conflicts(graph: &MapGraph, pos: &BTreeMap<RoomId,(i32,i32)>) -> usize { … }
```

- Only compass edges (`grid_offset(dir).is_some()`) define neighbours (non-compass
  stubs aren't drawn as routed paths).
- A pair counts at most once per room; the measure is symmetric and deterministic
  (`BTreeSet` ordering).

## Component 2 — Repair hill-climb (extended)

```rust
const MOVE_RADIUS: i32 = 3;

pub fn repair_routability(graph: &MapGraph, pos: &mut BTreeMap<RoomId,(i32,i32)>)
```

- **Score** (lexicographic, lower better):
  `(unroutable_count(pos,drawn), side_conflicts(graph,pos), displacement(pos,stress))`.
- **Candidate rooms** each pass: endpoints of currently-unroutable edges ∪
  `conflict_rooms` (each room in a same-octant pair, plus its two conflicting
  neighbours). If empty (0 unroutable AND 0 conflicts) → terminate.
- **Candidate moves:** for each candidate room, every currently-free cell within
  Chebyshev radius `MOVE_RADIUS`, enumerated in fixed `(dy,dx)` order. Keep the single
  strictly-improving move with the best score (first-found wins ties).
- **Termination:** strict improvement on a lexicographic integer tuple bounded below,
  plus the existing `MAX_REPAIR_PASSES` backstop. Deterministic: fixed iteration
  order, integer score, no RNG.

Worked example (matches the empirical table): A129 after Milestone 5 sits at
`#25(0,0)`, `#74(-1,1)`, `#76(-1,2)` — 0 unroutable, **1 conflict** (`#25`'s
neighbours `#74`,`#76` both SW). Candidate rooms = `{25,74,76}`. Among free cells
within radius 3, moving `#25→(0,2)` gives `(0 unroutable, 0 conflicts, displacement
2)` — strict improvement. `(0,1)` is rejected (raises `unroutable_count`: `#74` would
be due-west of `#25`, re-blocking `25→76`). The closest 0/0 cell wins via the
displacement tiebreak → `#25` lands at `(0,2)`, the user's fix.

## Determinism

Unchanged from Milestone 5: connections are a `Vec`, positions a `BTreeMap`, neighbour
sets are `BTreeSet`, score is integer-only, moves accepted only on strict improvement
with a first-found tiebreak in fixed order. Same graph → identical layout. The
existing `relayout_is_deterministic` test must keep passing.

## Edge Cases

- **Reciprocal pairs** (`76↔74`, `79↔80`): a neighbour is counted once via the
  distinct-neighbour set — no double counting, no false self-conflict.
- **Non-compass stubs:** excluded from `neighbours` — unaffected.
- **Genuinely-unavoidable clustering** (a hub room with 3+ neighbours forced into one
  octant): the climb reaches a local min with conflicts > 0 and terminates; the
  residual crossing renders as a legal 90° meet (still no overlap).
- **Valley between positions:** handled by `MOVE_RADIUS` multi-cell moves (decision 3).
- **Approximation:** the octant count is a *proxy* for rendered crossings, not a
  pixel-exact count. Placement is steered by the proxy; the renderer remains the
  source of truth and still never overlaps. The end-to-end render test (below) is the
  acceptance gate.

## Testing

`mapper` (`routability.rs`):
- `octant`: signs map correctly (e.g. `octant(-1,2) == (-1,1)`, `octant(-1,0) == (-1,0)`).
- `side_conflicts`: a room with two neighbours in the same octant → 1; in different
  octants → 0; a reciprocal pair contributes one neighbour (no self-conflict).
- **A129 corner (direct, on `pos`):** feed `{25:(0,0),74:(-1,1),76:(-1,2)}`; assert
  `side_conflicts == 1` before repair, then `repair_routability` yields **0
  unroutable AND 0 conflicts**, `#25` moved off `(0,0)` to row ≥ 2, no room overlap.
- **A129 full graph via `relayout_auto`:** assert final layout has 0 unroutable and 0
  side-conflicts; `#25` is not at the same row as both `#74` and `#76`.
- Determinism preserved (existing test); Milestone-5 routability tests still pass.

`app` (render, end-to-end — the real acceptance gate):
- Re-render the full A129 graph through `relayout_auto`; assert **0 unrouted
  (DarkGray) cells AND 0 perpendicular-crossing ribbon cells** in the rendered buffer
  (the same crossing-cell scan used in the empirical probe).

## Out of Scope

- Any path/route modelling in the layout (the octant proxy needs none).
- Porting the renderer's router into the layout.
- Arrival-side as an explicit variable (the renderer's existing `nearest_free_side`
  lands `74→25` on `#25`'s top once placement is fixed — confirmed by the probe).
- Any change to persistence, DOT export, dump format, zvm bridge, or the renderer's
  Tier-1 rules / unrouted-line fallback.

## Limitations (accepted)

- The octant count is a geometric proxy: it targets the dominant cause (a room whose
  neighbours fan one way) but won't catch every conceivable rendered crossing (e.g.
  two edges from different rooms meeting in open space). The end-to-end render test
  guards the real result for the reported case; other cases iterate if they arise.
- "Minimize conflicts" with radius-3 moves can relocate rooms more between turns.
- Cost: each trial move recomputes `unroutable_count` (E BFS) and `side_conflicts`
  (O(rooms·deg²)); candidates × radius-area × passes. Bounded by `MAX_NODES` and
  `MAX_REPAIR_PASSES`; revisit only if dense maps feel slow.
