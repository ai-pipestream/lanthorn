# Incremental, Segmented Automapping — Design Spec

**Date:** 2026-06-21
**Branch:** `fix-tui-runtime`
**Status:** Approved (design) — awaiting spec review
**Supersedes (layout engine):** `2026-06-19-constrained-stress-layout-design.md`,
`2026-06-20-layout-routability-repair-design.md`,
`2026-06-20-crossing-aware-repair-design.md`
**Builds on (unchanged):** `2026-06-20-lane-routing-design.md` (the lane router is kept as-is)

## Goal

Replace lanthorn's from-scratch-every-turn global layout (constrained stress
majorization + two repair passes) with a **two-regime** automapper that matches how
runtime IF mapping actually works:

1. **Incremental local placement** every turn — stable, online, the map does not jump.
2. **On-demand global re-tidy** — a fresh `sort → route → optimize` pipeline the user
   invokes when they want the map cleaned up.

Alongside the layout change, add three capabilities the current map lacks: **diagonal
directions** as first-class planar moves, **`up`/`down`/`in`/`out` as inter-segment
portals**, **multiple map segments** (auto-derived with manual override and a segment
tab/list), and an **opt-in Nerd Font glyph theme** with a portable Unicode default.

## Background — why a redesign

The current layout is five stacked layers, each patching the previous:
constrained stress majorization → routability repair → crossing-aware repair → lane
routing → per-edge fixes. The root fragility is structural, not a single bug:

- **Layout and routing are decoupled, so the layout plans with proxies.** It cannot run
  the real router, so it *guesses* what the renderer will do — `octant conflict` stands
  in for "crossings," grid-BFS `edge_routable` stands in for "has a channel." Every
  approximation that leaks becomes the next patch.
- **Two repairs that fight each other** over one lexicographic score, with a radius-3
  escape hatch bolted on because ±1 hill-climbing could not cross the valley between
  them.
- **It re-solves from scratch every turn**, violating lanthorn's own design requirement
  ("the map does not jump every turn"). Stability — the primary objective of dynamic
  graph layout — was never implemented.

### Research findings that shaped this design

(Deep-research pass, 2026-06-21; 21 sources, 25 claims verified 3-0.)

- **The canonical IF automapper (Trizbort) uses no global optimizer.** It places each
  new room at `current + compass_vector × spacing`, and on collision **shifts only the
  rooms beyond** the insertion point. A source-level read found zero force-directed /
  layered / stress code. Runtime IF mapping is inherently the **online** case (you
  cannot foresee future rooms), which is exactly why a local-incremental strategy fits
  and a global re-solve does not.
- **Stability / mental-map preservation is the primary goal of dynamic layout** —
  "existing nodes and edges should change as little as possible when the graph changes"
  (Coleman & Parker), formalized via orthogonal ordering / proximity / topology (Misue
  et al.). Orthogonal ordering *is* compass ordering.
- **Honest caveat:** as of 2013 no experiment conclusively showed mental-map
  preservation aids comprehension. Stability is a near-universal design heuristic, not a
  proven law — but it is also what every real tool and lanthorn's own spec already
  chose.

## Decisions (from brainstorming)

1. **Two regimes**, not one global solve (Option C — hybrid).
2. **Re-tidy engine:** a fresh `sort → route → optimize` pipeline (clean break from the
   stress solver), not a reuse of the existing engine.
3. **Re-tidy trigger:** explicit key (`R`) **plus** a non-blocking suggestion when
   distortion/crossings exceed a threshold.
4. **Incremental collision rule:** **shift rooms beyond** (Trizbort behavior) so the new
   room lands truthfully in the compass direction; existing rooms otherwise never move.
5. **Diagonals are planar** grid moves (corner departure); **`up`/`down`/`in`/`out` are
   non-planar portals**.
6. **Segments = planar-connected components**, auto-derived, with manual override to
   peel a subset into a new segment or merge a segment back into another.
7. **Display:** current room's segment + a segment tab/list to view others.
8. **Glyph theme:** opt-in Nerd Font with a portable Unicode default; the `.map.txt`
   dump is always Unicode.
9. **"Size by exits" stage dropped** (YAGNI — the lane router's side-slots and dynamic
   gaps already absorb high-degree rooms; boxes stay fixed 11×5).

## Architecture at a glance

```
Per turn (default):
    incremental local placement (per segment)  ──► lane router ──► render(theme)

On 'R' (or accepted suggestion):
    global re-tidy of current segment (sort → route → optimize)  ──► lane router ──► render(theme)
```

The lane router (`2026-06-20-lane-routing-design.md`) is **unchanged** and used by both
paths. **Removed entirely:** the constrained stress-majorization engine
(`layout/vpsc.rs`, `layout/stress.rs`, `layout/constraints.rs`), the routability repair,
and the crossing-aware repair (`layout/routability.rs`).

## Direction model

Two classes, decided by a single predicate (extends the existing `grid_offset`):

- **Planar** — `N S E W NE NW SE SW`. Real moves on a segment's 2D grid.
  `grid_offset(dir)` returns the cell delta (diagonals return both axes nonzero, as
  today). Cardinals depart from the box edge midpoint; **diagonals depart from the box
  corner**. The lane router routes both; either is marked `distorted` when the final
  placed geometry does not satisfy the compass relation. A **distorted diagonal** is
  drawn like a distorted cardinal — corner-departure arrow, orthogonal lane route.
- **Portal** — `Up Down In Out` and `Unknown`. No grid offset. Link a room to a room in
  another segment (or, when both endpoints are already co-planar, the same segment).
  Rendered as **labeled stubs**, never as grid paths.

## Segment model

- **Definition:** a segment is a maximal set of rooms connected by **planar edges only**.
  Portal edges do not bind rooms together, so `down` into a fresh room starts a new
  segment that then grows as the player explores it with cardinal/diagonal moves. Two
  genuinely disconnected planar components are separate segments even with no portal
  between them.
- **Per-segment coordinates:** each segment has its own `(0,0)` plane; a room's `pos` is
  its cell *within its segment*. Both regimes operate per-segment. A merged segment may
  be planar-disconnected internally — handled by component packing (sub-parts placed
  adjacent), the same mechanism today uses for disconnected components.
- **Identity & naming:** each segment has a stable id. Display name defaults to its entry
  room's name (the portal destination that first created it); user-renameable. The start
  area is segment 1.
- **Derivation & merges:** segment membership is derived from the planar subgraph and
  recomputed as edges are added. A later planar edge bridging two components **merges**
  their segments. A portal whose endpoints are already co-planar is just an
  intra-segment stub (no new segment).
- **Manual overrides:** a per-room optional `segment_override: Option<SegmentId>`.
  *Peel subset → new segment* tags selected rooms with a fresh override id; *merge back
  to source* reassigns them to a target segment. Derivation respects overrides, so they
  survive re-derivation and reload.

## Regime 1 — Incremental local placement (per turn)

Room positions are **authoritative accumulated state** (see Persistence). On a turn:

- Determine the player's recognized direction `dir` (or none) and the new current room
  `dest`, as today.
- **First room ever** → place at `(0,0)` in a new segment.
- **`dest` already placed** (loop closure, revisit, non-Euclidean return) → add/update
  the edge only; **no room moves**. If `dir` is planar and `dest`'s existing position
  does not satisfy `dir`, the edge is marked `distorted`. If `dir` is a portal and the
  endpoints are in different segments, it is an inter-segment portal.
- **`dest` is new and `dir` is planar** → place `dest` at `current.pos +
  grid_offset(dir)` in `current`'s segment. If the target cell's box footprint is
  occupied, **shift-beyond:** translate every room whose cell lies past the ideal cell in
  the `dir` direction by one step, repeat until the footprint is clear, then place
  `dest`. Existing rooms otherwise never move.
- **`dest` is new and `dir` is a portal** → create `dest` in a **new segment** at that
  segment's `(0,0)`; the portal edge links the two segments.
- **`dest` is new and `dir` is unknown** → place `dest` at the nearest free cell adjacent
  to `current` in `current`'s segment; the edge renders as an unknown-direction labeled
  stub (today's behavior).
- **Mark `distorted`:** any planar edge whose endpoints' final relative position does not
  match its direction.

Determinism: integer grid, fixed shift order, no RNG. Same event stream → same map.

## Regime 2 — Global re-tidy (explicit `R` + suggestion)

A full from-scratch reshuffle of the **currently-viewed segment** (allowed to move every
room in that segment — the user asked). Fresh pipeline:

1. **Sort (positioning).** Build per-axis constraint DAGs from the segment's planar
   edges: E/W components constrain x-order, N/S components constrain y-order, diagonals
   feed both axes. Break cycles deterministically (process edges in index order; an edge
   that would close a directed cycle on an axis is dropped and its edge marked
   `distorted`). Assign integer coordinates by **longest-path layering** per axis. This
   is the grid-native "2D sort" — no continuous solve, no proxies.
2. **Route.** Reuse the existing lane router unchanged.
3. **Optimize.** Bounded, deterministic crossing reduction: within each axis layer, swap
   adjacent rooms when it **strictly** reduces the rendered crossing count (measured via
   the lane router's plan); then compact away empty rows/columns. Strict-improvement +
   bounded passes ⇒ termination.

The result overwrites that segment's authoritative positions and persists. Re-tidy never
touches other segments.

**Suggestion:** when a segment's distortion + crossing count exceeds a threshold, the UI
surfaces a non-blocking hint (e.g. a status note "map tangled — press R to tidy"). The
player still chooses; nothing reshuffles on its own.

## Display

- The map pane shows the **current room's segment**. Following a portal in-game
  (player moves `down`) auto-switches the view to the new current room's segment.
- A **segment tab/list** (names + room counts, current highlighted) lets the player view
  any segment without moving the player; a key cycles/selects entries.
- **Portals** in the shown segment render as **labeled stubs** carrying the
  "where does this lead" description: a direction glyph + destination
  (`↓ Cellar`, `↑ Attic`, `in → Kitchen`). The destination label is the target room's
  name (and, when it is in another segment, that segment's name).

## Glyph theming

A `GlyphTheme` abstraction supplies every glyph the renderer emits — box corners/edges,
connector lines and junctions, arrowheads, portal markers (up/down/in/out), and status
markers (current-room, note, distorted). Two themes:

- **Unicode** (default, fully portable): today's box-drawing + geometric arrows; portals
  as arrow-stubs-with-text; markers as plain Unicode (`●`, `!`).
- **Nerd Font** (opt-in, persisted setting): keeps box-drawing for lines (already
  best-in-class) but upgrades the parts that benefit — nicer box borders/arrowheads, and
  the real wins as true icons: stairs-up / stairs-down for `up`/`down`, a door for
  `in`/`out`, a "you are here" marker, a note marker, a distinct distorted marker.

Two hard rules: the **`.map.txt` dump always renders with the Unicode theme** (shareable
anywhere), and the Nerd theme uses **only single-cell-width glyphs** (double-width PUA
icons that would break grid alignment are excluded). The theme is a glyph-lookup swap
with no layout impact; all three regimes route through one renderer.

## Persistence model change

In Auto mode, room positions move from "derived & cached from the constraint graph" to
**primary accumulated state**: built incrementally, persisted per segment, and reloaded
*as-is* with no layout run on load. Re-tidy is the only thing that bulk-rewrites them.
Manual mode is unchanged (frozen positions).

Persisted additions: per-segment room positions, segment overrides (manual peel/merge),
segment renames, and the selected glyph theme. Auto segment membership is re-derived
cheaply on load. View state (zoom, scroll, viewed-segment-when-not-current) stays
unpersisted.

## What is removed

- `layout/vpsc.rs`, `layout/stress.rs`, `layout/constraints.rs` (the constrained stress
  engine).
- `layout/routability.rs` (routability repair + crossing-aware repair).
- The from-scratch `relayout_auto` global solve on the per-turn path.

## What is kept

- The lane router (`route_lanes`, `RoutePlan`, side-slots, dynamic gaps) — unchanged.
- Arrow semantics (outgoing compass direction; reciprocal far-end arrow).
- `grid_offset`, `nearest_free_cell`, `occupied_cells`, component packing, Manual mode,
  `nudge`.
- Compact/Overview zoom rendering (uniform stride), the Boxes lane rendering, and the
  `.map.txt` dump pipeline (now theme-aware, Unicode-locked).

## Testing strategy

**Incremental placement (`mapper`):**
- New planar room lands at the compass-offset cell; diagonal lands in the diagonal cell.
- Shift-beyond opens space with zero overlap and moves only rooms past the insertion
  point.
- Revisit / loop-closure adds an edge and moves no room; contradictory planar return is
  marked `distorted`.
- Determinism: same event stream → byte-identical map.

**Directions:**
- `A NE B` → B in NE cell, corner departure; an unsatisfiable diagonal marks `distorted`
  and still routes.

**Segments:**
- `down` into a new room starts a new segment at its own origin; cardinal exploration
  grows it.
- A later planar edge bridging two components merges their segments.
- A portal whose endpoints are already co-planar stays an intra-segment stub.
- Peel + merge overrides reassign membership and persist across reload/re-derivation.

**Re-tidy (`mapper`):**
- Per-axis cycle-break drops exactly the contradictory edge.
- Longest-path coordinates satisfy every non-dropped compass order.
- Crossing-reduction never increases the rendered crossing count; terminates.
- Re-tidy overwrites only the viewed segment; round-trips through persistence.

**Stability (the key new guarantee, `app`/`mapper` end-to-end):**
- Replay a session; assert that on each turn only shift-beyond-affected rooms move — the
  map does not jump.

**Display & theming (`app`):**
- Portals render as labeled stubs with correct destination text; following one
  auto-switches the viewed segment; the tab/list shows all segments.
- Nerd theme swaps glyphs without changing any cell positions; Unicode theme unchanged;
  the dump is Unicode regardless of the active theme; no double-width glyph in the Nerd
  set.

## Implementation phasing (for the plan, not separate specs)

One design doc; the implementation plan ships each phase as working software:

1. **Layout regimes** — incremental local placement + re-tidy pipeline; remove the
   stress engine and repairs. (Single-segment behavior preserved.)
2. **Directions** — diagonals as planar moves; portal classification.
3. **Segments** — auto-derivation, per-segment coordinates, current-segment view.
4. **Segment editing + tabs** — peel/merge overrides, segment tab/list, portal labels.
5. **Glyph theming** — `GlyphTheme` abstraction, Unicode default, Nerd Font opt-in,
   Unicode-locked dump.

## Out of scope / non-goals

- "Size by exits" variable node sizing (dropped; lane router absorbs degree).
- All-segments-tiled display (chose current-segment + tabs).
- Automatic re-tidy (chose explicit + suggestion).
- Auto-detecting Nerd Font support (chose explicit opt-in).
- Changing the lane router, persistence file format beyond the additions above, the zvm
  bridge, or the Quetzal save path.
- A full freehand map editor (the manual overrides are limited to peel/merge/rename).

## Risks & limitations (accepted)

- **Greedy accumulation.** Incremental placement can accumulate awkward geometry over a
  long session; the on-demand re-tidy is the deliberate escape hatch.
- **Stability is a heuristic**, not an empirically proven benefit (2013 caveat) — but it
  matches lanthorn's spec and every real tool.
- **Crossing-reduction in re-tidy is bounded**, not optimal; legal perpendicular
  crossings may remain.
- **Nerd Font glyph selection** depends on terminal/font rendering; the single-cell-width
  rule mitigates but cannot guarantee every terminal renders every icon identically.
- **Segment membership churn:** merging/splitting segments relabels ids; renames are
  preserved via overrides, but downstream references to a segment id must tolerate
  re-derivation.
