# Tidy-Animation Diagnostics — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued; touches `mapper` layout internals + the app cleanup passes + the anim UI. Larger than the other queued items; implement in phases (see Phasing).
**TODO item:** "Add more steps in animation for various layout tasks and cleanup tasks to help diagnose, describe which algorithm is running in each part of the step" (L13).

## Goal

Turn the 7-frame opaque tidy animation into a fine-grained, self-describing trace: break the single `relayout` step into its algorithm stages, snapshot **each individual room move** inside the cleanup passes, and show for every frame the **algorithm + what it does + live stats** (rooms moved, overlaps resolved, constraints dropped).

## Current state

`run_tidy_pipeline` (app/input.rs:383) builds `Vec<TidyFrame>` with 7 coarse frames; `TidyFrame { label, graph }` (state.rs); `relayout_auto` (mapper) is one opaque call; cleanup passes (`cleanup_overlaps`, `repair_directional_hints`, `stack_updown_rooms`, `compact_empty_lines`, in app/render/map.rs) snapshot only their final state. The anim banner (main.rs:243) shows `Tidy [i/n] {label}`.

## Approach — an observer callback (no duplicated logic)

Each layout/cleanup function gains an **optional observer**; when `None` (normal gameplay) there is zero overhead, when `Some` (building the animation) it is invoked at each stage/move with a label, a description, and the live stats. The function bodies stay single-sourced.

```rust
// mapper (e.g. layout/mod.rs)
pub struct TidyStats {
    pub rooms_moved: u32, pub overlaps_resolved: u32,
    pub constraints_dropped: u32, pub hints_repaired: u32,
}
// Observer: called with the CURRENT graph + a labelled, described step + cumulative stats.
pub type TidyObserver<'a> = &'a mut dyn FnMut(&MapGraph, &str /*label*/, &str /*description*/, &TidyStats);

pub fn relayout_auto_observed(graph: &mut MapGraph, obs: Option<TidyObserver>);
pub fn relayout_auto(graph: &mut MapGraph) { relayout_auto_observed(graph, None) } // unchanged callers
```

The same pattern for the app cleanup passes: `cleanup_overlaps_observed(graph, passes, budget, obs)`, etc.; the existing `cleanup_overlaps(...)` calls the observed form with `None`.

## What gets emitted

### Layout stages (`relayout_auto_observed`, mapper) — stage-level
After each internal stage, emit a snapshot + description:
1. **Seed (longest-path sort)** — "Longest-path layering: integer coords per axis from compass edges." stats: rooms placed.
2. **Stress majorization (SMACOF ×60)** — "Stress majorization: places rooms by graph-theoretic distance under VPSC compass-separation constraints." stats: constraints_dropped (cycle-closing edges).
3. **Axis-align** — "Align free axes: pull single-axis-free rooms onto their neighbour's row/column so cardinal edges render straight."
4. **Contiguify** — "Contiguity: eject foreign rooms interleaved within a chain's span."
5. **Pack + collision resolve** — "Pack components left-to-right; resolve residual same-cell collisions, keeping aligned rooms on their line." stats: rooms_moved.

(The 60 stress iterations are continuous, not discrete moves, so stress is one stage — optionally a few iteration checkpoints, but NOT per-iteration frames.)

### Cleanup moves (app passes) — move-level
`cleanup_overlaps`, `repair_directional_hints`, `stack_updown_rooms`: emit a frame **per room move**, each described with the reason and the running stats, e.g.:
- cleanup_overlaps: "Overlap cleanup: moved room 180 (West of House) from (-5,1) to (-6,1) to clear overlap with 193." stats: overlaps_resolved++.
- repair_directional_hints: "Repair hint: moved room 77 to restore the E edge 77→239." stats: hints_repaired++.
- stack_updown_rooms: "Stack up/down: placed 201 north of 203 (Up)."
`compact_empty_lines`: emit per collapsed row/column (or once with a count).

## Frame & UI changes

- **`TidyFrame`** (state.rs) gains `pub description: String` and `pub stats: TidyStats` (the `graph` + short `label` stay). `run_tidy_pipeline` builds frames by passing an observer closure that pushes a `TidyFrame` per callback.
- **Playback UI:** the bottom banner keeps `Tidy [i/n] {label}`; the **description + stats** render in a panel (a few lines) over the map during anim playback (reuse the overlay-drawing approach; a new `render/tidy_panel.rs` or extend the anim banner to a multi-line box). The existing transport (←→ step, Space play/pause, Esc exit, pan/zoom) is unchanged.
- **Step granularity navigation:** with move-level frames there may be many; `←/→` step one frame; (optional) `Shift+←/→` jump to the next *stage* boundary (a frame flagged `is_stage_start`). Include a `stage_start: bool` on `TidyFrame` for this.

## Footprint

- `mapper`: `relayout_auto_observed` + `TidyStats`/`TidyObserver` (instrument the stages in `layout/mod.rs`; the sub-stage functions already exist — `sort_layout`, `stress_layout`, `align_free_axes`, `contiguify` — call the observer between them).
- `crates/app/src/render/map.rs`: `*_observed` variants of the cleanup passes (instrument the move sites).
- `crates/app/src/input.rs`: `run_tidy_pipeline` builds rich frames via observers.
- `crates/app/src/state.rs`: `TidyFrame` fields; `crates/app/src/main.rs` / a new `render/tidy_panel.rs`: the description panel.

## Testing

- `relayout_auto_observed` with a recording observer emits the 5 stage labels in order; `relayout_auto` (no observer) is byte-identical to today (existing layout tests unchanged).
- A cleanup pass with an observer emits one step per move, with `overlaps_resolved`/`hints_repaired` incrementing; without an observer the result graph is identical.
- `run_tidy_pipeline` produces frames whose `description` is non-empty and whose `stage_start` flags mark the stage boundaries; the final graph equals today's tidied result.
- Panel render test (TestBackend): the description + a stat line appear during playback.

## Phasing (implement in this order; each independently testable)

1. **mapper observer** — `TidyStats`, `TidyObserver`, `relayout_auto_observed` emitting the 5 stage frames; `relayout_auto` delegates. (mapper-only; no app changes.)
2. **cleanup observers** — `*_observed` variants emitting per-move frames + stats (app/render/map.rs).
3. **rich frames** — `TidyFrame.description/stats/stage_start`; `run_tidy_pipeline` wires the observers.
4. **UI** — the description/stats panel + optional stage-jump navigation.

## Out of scope / non-goals

- Per-stress-iteration frames (stress is one stage).
- Instrumenting incremental per-turn placement (`observe`/`place_incremental`) — this is for the on-demand tidy animation only.
- Editing/replaying the layout (read-only diagnostic).
- `mapper` algorithm changes (instrumentation only — the layout result must not change).

## Risks & limitations (accepted)

- **Frame count & memory:** move-level snapshots clone the (per-layer) graph per move — potentially hundreds of frames on a busy cleanup. The active layer is usually small; still, cap the trace (e.g. `MAX_TIDY_FRAMES`, log when truncated) and snapshot only the active layer (already the case). A future optimization could store position-diffs instead of full graph clones.
- **Observer plumbing** threads an optional parameter through several functions; the `None` path must stay allocation-free so normal gameplay/background-tidy is unaffected.
- **Description text** is developer-facing diagnostic prose, not localized.
