# Animate Room Placement — Design

**Quest:** SQ-0217 — Add room placement to map animations

**Goal:** Expand the existing `AnimateTidy` layout-diagnostics animation so it also
replays the earlier stages of layout — the graph build and the room-by-room
incremental placement — before the current SMACOF → cleanup → repair → stack →
cleanup stages, giving one coherent narrative from "nothing" to the tidied map.

## Background

The diagnostics animation is a **discrete snapshot stepper**, not smooth tweening:

- `Action::AnimateTidy` (`crates/app/src/input.rs:2001-2007`) calls
  `run_tidy_pipeline(&mut mapper.graph, layer)` and stores the returned frames in
  `state.tidy_anim` (`TidyAnim`, `crates/app/src/state.rs:388-438`).
- `run_tidy_pipeline` (`crates/app/src/input.rs:1454-1633`) clones the active
  layer's subgraph, runs each tidy stage through an `*_observed` variant that
  pushes a `TidyFrame` per move, then writes the final positions/distortion flags
  back into the live graph. **The live map is mutated up front**; frames are
  cloned playback material. It is scoped to a single active layer
  (`state.active_layer`).
- `TidyFrame` (`crates/app/src/state.rs:375-382`) = `{ label, graph: MapGraph,
  description, stats: TidyStats, stage_start }`. Playback ticks in the main loop's
  no-input branch (`crates/app/src/main.rs:2254`, hardcoded 700 ms dwell); the map
  renderer draws `frame.graph` instead of the live graph while `tidy_anim` is set.

Today the first captured stage is SMACOF (`relayout_auto_observed`), by which point
every room already sits at the position the greedy incremental placement gave it
during play. Nothing shows the map being *built* or *placed*.

## Feasibility (confirmed in code)

- **Discovery order is reconstructable from the graph alone.** Room ids are game
  location values or FNV name-hashes (`crates/app/src/roomid.rs:18-26`), NOT
  monotonic, and `MapGraph.rooms` is a `BTreeMap` (`crates/mapper/src/graph.rs:37`).
  But `conns: Vec<Connection>` preserves **edge-insertion order** as
  `(origin, dir, dest)` (`graph.rs:28-34`, `add_edge` at `graph.rs:148-154`). That
  is exactly the input incremental placement needs — no dependency on
  `state.history` / `record_turn_history`.
- **`place_incremental` is deterministic and replayable**
  (`crates/mapper/src/layout/incremental.rs:14-66`, `pub`): given
  `(graph, prev, dest, dir)` with `prev` already placed, it sets `dest`'s position.
  Re-running it from an empty graph over the ordered edge list reproduces the
  incremental layout.
- No mapper changes are required — `place_incremental` is already public.

## Approach (chosen)

**Reconstruct-from-scratch inside the pipeline.** `run_tidy_pipeline` gains two
front phases that run on a freshly rebuilt graph, then hands the placed graph to
the existing tidy stages, and writes back as today. Rejected alternatives:
history-driven replay (needs history enabled, extra plumbing, redundant) and a
visual-only preamble (causes a jump between raw-incremental placement and the
already-tidied live positions feeding SMACOF).

## Phase sequence

| Phase | Frames | Map pane | Caption (`description`) |
|-------|--------|----------|-------------------------|
| **Build** | **1** (a single stop) | connection **manifest** text | `"Graph built: 12 rooms, 15 connections"` |
| **Placement** | 1 per room | rooms popping in at initial cells | `"placed room 5 (Hall) N of room 3 at (0,1)"` |
| SMACOF | unchanged | relaxing | existing |
| Cleanup / Repair / Stack / Cleanup | unchanged | — | existing |

The first frame of Build and of Placement sets `stage_start = true` with labels
`"Build"` and `"Placement"` so the transport line names them like existing stages.
The two new phases pass a default/empty `TidyStats`.

## Components

### 1. `TidyFrame` gains a manifest field — `crates/app/src/state.rs`

Add `pub manifest: Option<Vec<String>>` to `TidyFrame` (default `None`). When
`Some`, the renderer draws these lines in the map pane instead of rooms; the
frame's `graph` is the fully-built (position-less) graph. All existing frame
construction sites set `manifest: None`.

### 2. Replay helper — `crates/app/src/input.rs`

New app-side helper beside `run_tidy_pipeline` (call it
`replay_build_and_placement(sub: &MapGraph, frames: &mut Vec<TidyFrame>) -> MapGraph`):

1. Read `sub.connections()` in insertion order → ordered `(origin, dir, dest)` list.
   **Anchor** = origin of the first connection, or the sole room if there are no
   connections.
2. **Build (silent construct + 1 frame):** start an empty `MapGraph`; add the
   anchor (`pos = None`); for each edge in order `upsert_room(dest)` +
   `add_edge(origin, dir, dest)` (still `pos = None`). Emit **one** frame:
   `graph = rebuild.clone()`, `manifest = Some(lines)` where `lines` is one entry
   per connection (`"3 →N→ 5"`, using room names), `description = "Graph built: N
   rooms, M connections"`, `label = "Build"`, `stage_start = true`.
3. **Placement (per-room frames):** `set_pos(anchor, (0,0))`, emit a frame; then
   for each edge in order call `place_incremental(&mut rebuild, origin, dest, dir)`
   and emit a frame with `description = "placed room {dest} ({name}) {dir} of room
   {origin} at {pos}"`, `manifest = None`, `label = "Placement"`, `stage_start`
   true only on the first. Origins are always already placed (same invariant the
   live path relies on).
4. Return `rebuild` (fully placed) for the existing stages to consume.

`run_tidy_pipeline` changes: instead of seeding `sub` from the live layer subgraph
and running stages on it, it calls `replay_build_and_placement` to produce the
front frames and the placed `rebuild`, then runs the existing
SMACOF/cleanup/repair/stack/cleanup stages **on `rebuild`**, then writes
`rebuild`'s positions/flags back to the live graph exactly as today.

### 3. Manifest rendering — `crates/app/src/render/map.rs`

Where the map renderer draws the active `tidy_anim` frame's graph, add: if the
current frame's `manifest` is `Some(lines)`, render those lines as left-aligned
text inside the map pane (scrolled/clamped to the pane, themeable via an existing
transcript/text selector) and skip room drawing. Position-less graphs already draw
nothing, so no room-skip special-case is otherwise needed.

## Layer scoping & edge cases

- **Per active layer**, unchanged. Within-layer replay only sees same-layer edges
  (cross-layer up/down edges are already excluded from the subgraph). This is
  independent of SQ-0216, which concerns the *live* up/down placement rule.
- **Disconnected rooms** in the layer (not reached by the edge walk): after the
  main walk, place each leftover room via the existing fallback
  (`nearest_free_cell`), emitting one placement frame each. They are omitted from
  the Build connection manifest (they have no in-layer edge) but still counted in
  the "N rooms" total in the Build summary.
- **Single-room layer** (no connections): Build frame with an empty connection
  manifest and `"Graph built: 1 room, 0 connections"`, then one Placement frame
  (anchor at origin), then tidy stages.
- **Empty layer**: no frames added beyond whatever the existing stages produce
  (which is nothing) — the animation is a no-op, as today.
- Snapping only; no tweening; no fractional-position renderer work.

## Write-back, pacing, config

- Final write-back to the live graph is unchanged.
- **For the common case (no prior background tidy; live positions == raw
  incremental) the reconstruction reproduces those positions exactly, so the final
  live layout is byte-identical to today.** It can differ only after a prior tidy
  (SMACOF converging from a different start) — the same re-tidy caveat that already
  applies to `AnimateTidy` / `Retidy`.
- `MAX_TIDY_FRAMES = 2000` stays as the global cap. Build+placement add ~1 + N
  frames for an N-room layer — well under the cap.
- Dwell stays the hardcoded 700 ms; no new config. `Action::AnimExit`
  (`input.rs:2021`) and the existing transport controls handle the longer sequence
  unchanged.

## Testing

- **Frame order:** graph A →N→ B →E→ C ⇒ frames are `Build×1` → `Placement×3` →
  existing stages, in that order.
- **Build frame:** exactly one; `manifest` is `Some` with one line per connection;
  no room in its `graph` has a position; `description` reports the right room/conn
  counts.
- **Placement frames:** count == room count; each frame's positions match a direct
  `place_incremental` replay over the same ordered edge list.
- **Regression:** for a raw-incremental graph (no prior tidy) the final
  written-back positions equal today's `run_tidy_pipeline` result.
- **Single-room layer** edge case: `Build×1` (empty manifest) → `Placement×1` →
  stages.
