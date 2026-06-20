# Layout Routability Repair — Design Spec

**Date:** 2026-06-20
**Branch:** `fix-tui-runtime`
**Status:** Approved (design) — awaiting spec review

## Goal

Eliminate connector-over-room and connector-over-connector overlaps by making
the layout produce positions in which **every drawn edge has a clean orthogonal
routing channel**, shifting rooms (alignment relaxed) where the stress solve
leaves an edge un-routable. The renderer then enforces no-overlap as a hard
invariant: it routes only cleanly, and where it genuinely cannot, it draws a
visibly-distinct "unrouted" connector instead of a silently-overlapping line.

## Background / Root Cause

The recurring failure (`ZCODE-88-840726-A129`, rooms `#25/#74/#76`) is **not** a
routing bug and **not** a spacing problem (doubling the grid stride was tested —
it only lengthens the bad detour). It is a *placement* problem:

- Three mutually inconsistent compass hints — `74→E→25`, `74→S→76`, `25→W→76` —
  cannot be jointly satisfied on a grid, so the stress solve cramps the rooms
  into an L with `#74` wedged directly between `#25` and `#76`.
- The `25→W→76` edge must leave `#25` going west (its arrow side), but `#74`
  occupies the cell due west. No departure side yields a clean route, so
  `route_ortho` degrades to a fallback tier that **permits overlap** (Tier 2
  drops path-vs-path rules; Tier 3's L ignores path overlaps).

The fix the user identified: let the layout **discard a contradictory hint and
shift a room** (e.g. `#25` down one row) so `25→76` becomes a clean straight
west shot — which also makes the `◀` arrow *truthful*, un-distorting the edge.

## Decisions (from brainstorming)

1. **Approach:** Routability-repair in the layout engine (not lane routing, not
   constraint-discard-only).
2. **Arrows:** Always show the typed command direction (`◀` for "west"), even if
   a still-distorted edge's path bends. The departure side = the compass side.
3. **Repair freedom:** Optimize for best routing — rooms may move freely from
   their stress-solved positions (primary objective = fewest un-routable edges;
   displacement is only a deterministic tiebreaker).
4. **Genuinely-unroutable edge:** The renderer draws a visibly-distinct
   (dashed/dimmed) connector that flags "couldn't be placed cleanly", rather
   than a normal-looking overlap.

## Architecture

Two independent changes that meet at the no-overlap invariant:

```
mapper::layout::relayout_auto
   … stress solve → snap → pack → collision-resolve  (unchanged)
   → repair_routability(graph, &mut final_pos, occupied)   ← NEW
   → anchor lowest-id at (0,0)                              (unchanged)
   → mark_distorted                                         (unchanged; now re-
                                                              evaluates repaired
                                                              geometry, so fixed
                                                              edges drop the flag)
```

```
app::render::map::route_ortho
   Tier 1 (clean: room clearance + path rules)   (unchanged)
   Tier 2 (drop path rules)                        ← REMOVED
   Tier 3 (overlap-permitting L)                   ← REMOVED
   → on Tier-1 failure, return an explicit "unrouted" marker          ← NEW
app::render::map::render_map
   → draw unrouted edges as a distinct dashed/dimmed ribbon            ← NEW
```

Rooms stay on **integer grid cells** throughout. "Alignment relaxed" means a
room may occupy a row/column none of its neighbors share — *not* fractional
coordinates. `cell_to_virtual`, persistence, and the zvm bridge are untouched.

## Component 1 — Routability predicate (`mapper/src/layout/routability.rs`)

```rust
/// True iff edge `origin →dir→ dest` has a clean orthogonal channel on the grid:
/// a BFS from `origin`'s cell whose FIRST step is in `dir`'s direction reaches
/// `dest`'s cell without entering any other room's cell. Non-compass dirs
/// (grid_offset == None) are stubs — never drawn as paths — and return true.
fn edge_routable(
    origin: (i32, i32),
    dest: (i32, i32),
    dir: Direction,
    occupied: &BTreeMap<(i32, i32), RoomId>, // cell → room
    origin_id: RoomId,
    dest_id: RoomId,
    bbox: (i32, i32, i32, i32),              // min_x,min_y,max_x,max_y + margin
) -> bool
```

Rules:
- Obstacles = every occupied cell **except** `origin` and `dest`.
- First step: for a cardinal `dir`, exactly that unit step; for a diagonal
  `dir` (NE/NW/SE/SW), either of its two non-zero axis components.
- BFS is bounded to `bbox` (the component's bounding box expanded by a small
  margin, e.g. 2) so it terminates and a far-flung detour doesn't count as
  "routable" through unbounded empty space.
- Models **rooms only**, at grid granularity. A clear grid cell = a full empty
  `29×17` stride at render granularity (≫ the `21×11` box), so a grid-level
  channel implies a render-level channel *for room obstacles*. It does **not**
  model path-vs-path congestion — see Limitations.

## Component 2 — Repair search (`mapper/src/layout/routability.rs`)

```rust
/// Greedily shift rooms (into free grid cells) until no further reduction in the
/// number of un-routable drawn edges is possible. Mutates `pos`.
pub fn repair_routability(
    graph: &MapGraph,
    pos: &mut BTreeMap<RoomId, (i32, i32)>,
)
```

Algorithm (deterministic hill-climb):
- `drawn_edges` = connections with `grid_offset(dir).is_some()` (compass edges;
  the only ones rendered as paths). Distorted-or-not is irrelevant here — a
  distorted edge is still drawn and still needs a channel.
- **Score** (lexicographic, lower is better): `(unroutable_count, total_displacement)`
  where `total_displacement` = Σ|pos − stress_pos|₁ over all rooms (the stress
  positions captured before repair). Primary term dominates; displacement is a
  pure tiebreaker so the search is deterministic and doesn't drift needlessly.
- **Loop** up to `MAX_REPAIR_PASSES` (e.g. 30):
  - Recompute the set of un-routable drawn edges. If empty → done.
  - For each un-routable edge (ordered by connection index), gather **candidate
    moves**: each of `{origin, dest}` moved by one cell in each of N/S/E/W into
    a currently-free cell. (Multi-cell shifts emerge across passes.)
  - Evaluate each candidate's score on a trial `pos`. Keep the single best move
    of the pass that **strictly** improves the global score.
  - Apply it; update `occupied`. If no candidate strictly improves, `break`.
- Strict-improvement + bounded score (`unroutable ≥ 0`, displacement ≥ 0) ⇒
  termination. Cap is a backstop.

Runs only on the `≤ MAX_NODES` path (the existing large-graph branch keeps the
seed grid untouched, as today).

## Component 3 — Renderer no-overlap invariant (`app/src/render/map.rs`)

- `route_ortho` keeps **only Tier 1**. Its signature gains an `Option` return
  (or a sentinel) meaning "no clean route":
  ```rust
  fn route_ortho(...) -> Option<Vec<(i32, i32)>>   // None = unrouted
  ```
- `render_map`: when `route_ortho` returns `None`, the edge is **unrouted**.
  Draw it as a distinct ribbon — dashed glyphs / a dimmed style
  (`PATH_BG_UNROUTED`) — along a best-effort straight L so it's visibly present
  but unmistakably flagged. It does NOT participate in the `paths` occupancy map
  (it's the acknowledged exception), and carries no embedded arrow ambiguity:
  the typed-direction arrow still sits at its departure anchor.
- All existing clean edges are unchanged.

## Data Flow

`relayout_auto` → repair mutates `final_pos` → positions persist to
`graph` as today → `mark_distorted` re-runs on repaired geometry → `render(graph)`
→ `render_map` routes each edge (Tier 1 only) → clean ribbon or flagged unrouted
ribbon.

## Determinism

- Repair captures stress positions once, iterates edges by connection index,
  candidates in fixed N/S/E/W order, accepts only strict score improvements with
  the displacement tiebreaker. No RNG. Same graph → same layout, preserving the
  existing `relayout_is_deterministic` guarantee.

## Edge Cases

- **Impossible hint (mutual-S, A↔B both S):** repair cannot make both routable;
  after the cap one stays un-routable → `mark_distorted` flags it → renderer
  draws the distinct unrouted line. Test asserts termination + exactly the
  expected residual.
- **Diagonal compass dirs:** first-step allowed along either axis component.
- **Non-compass stubs (Up/Down/In/Out/Unknown):** excluded from `drawn_edges`;
  unaffected (still rendered as labelled stubs).
- **Disconnected components:** routability is naturally per-component (no edges
  cross), so repair within global `occupied` is safe and overlap-free.

## Testing

Mapper (`layout/routability.rs` + `layout/mod.rs`):
- `edge_routable` true for a clear adjacent edge; false when a third room
  occupies the departure-direction cell.
- **The exact `#25/#74/#76` graph**: after `relayout_auto`, every compass edge
  is routable, and `25→76` is no longer distorted (it became a clean west shot).
- Mutual-S pair: `relayout_auto` terminates; exactly one of the two edges
  remains un-routable/distorted; no overlap.
- Determinism preserved (existing test still passes; add one over a graph that
  triggers repair).
- No room overlap after repair (extends existing overlap invariants).

Renderer (`render/map.rs`):
- `route_ortho` returns `None` instead of an overlapping path when boxed in
  (replaces the old Tier-2/3 behaviour in `route_keeps_gap_from_earlier_path`
  and friends — update expectations).
- An unrouted edge renders with the distinct style and never writes into the
  `paths` occupancy set (so it can't be "run alongside" by a later edge).

## Out of Scope

- Path-vs-path congestion repair. The layout models room obstacles only; if two
  edges genuinely contend for one channel and one fails Tier 1, it renders as an
  unrouted line. Lane/channel routing (the heavier option) is deferred.
- Continuous (fractional) room coordinates — rooms stay on the integer grid.
- Any change to persistence, zvm bridge, DOT export, or the dump format.

## Limitations (documented, accepted)

- Grid-level routability is a sound *necessary* check for room obstacles but not
  a complete guarantee at render granularity once multiple paths interact. The
  renderer's unrouted-line fallback is the honest backstop for the residual.
- "Optimize for best routing" can relocate rooms noticeably between turns as the
  map grows; accepted per decision 3.
