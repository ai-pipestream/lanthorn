# Async Background Tidy + Pulsing Map Border — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via feedback Q&A) — queued.
**Feedback items:** "Background tidy on edge add/update" (item 4), "minimal-case overlap" (item 3, subsumed), "tidy progress indicator" (item 5, refined to a pulsing border).
**Touches:** `crates/app/src/main.rs` (run loop), `crates/app/src/state.rs`, `crates/app/src/input.rs` (`should_bg_tidy`), `crates/app/src/render/map.rs` (border color). Possibly small `crates/mapper` derive additions (`Clone`/`Send`) ONLY if the graph isn't already cloneable/sendable — no algorithm changes.

## Goals

1. **Trigger fix (item 4 / 3):** Re-tidy whenever a turn leaves the active layer with an overlap or a distorted edge — not only when a new room is discovered. This fixes the 180/80 minimal-overlap case (a second, conflicting edge on a known pair). The layout already guarantees no-overlap after a relayout and already bends conflicting edges (distorted), so simply running a tidy resolves it — no mapper layout changes.
2. **Async tidy (item 5):** Run the background tidy on a worker thread so the render loop stays live during it.
3. **Pulsing border (item 5 UI):** While a tidy is running, the map pane's border smoothly pulses between red and green; when it completes, the border returns to its normal color. (No progress bar this round.)

## Part 1 — Trigger on overlap, every turn

Today (main.rs run loop, Auto mode only) the overlap/distorted signal is computed **only** when `background_tidy == OnOverlap`; `EveryRoom`/`Debounced` key off `new_room` alone (`should_bg_tidy`, input.rs).

Change: compute the overlap/distorted signal **every turn** (reuse the existing `occupied_cells_in_layer(...)` count `< rooms_in_layer` test, plus the active-layer distorted-edge scan), and feed it into `should_bg_tidy` for ALL modes:

- `Off` → never (unchanged).
- `EveryRoom` → tidy on `new_room || overlap`.
- `OnOverlap` → tidy on `overlap` (unchanged).
- `Debounced` → still counts `new_room` toward the debounce, BUT also tidy immediately if `overlap` (an overlap shouldn't wait for the debounce counter).

`should_bg_tidy(mode, new_room, overlap, counter)` already takes `overlap`; only the call site (always pass the real overlap) and the `EveryRoom`/`Debounced` arms change.

## Part 2 — Async tidy on a worker thread

Replace the synchronous `tidy_layer_silent(&mut graph, layer)` call (in the trigger path only — the manual/animated `Retidy`/`AnimateTidy` paths stay as they are) with a worker-thread model:

- **State (AppState):** add a tidy-job handle, e.g.
  ```rust
  pub struct TidyJob {
      pub handle: std::thread::JoinHandle<MapGraph>, // worker returns the tidied graph clone
      pub layer: i32,
      pub gen: u64,                                  // graph generation at snapshot time
      pub started: std::time::Instant,               // drives the pulse phase
  }
  pub tidy_job: Option<TidyJob>,
  pub graph_gen: u64,   // bumped whenever the real graph is mutated (each applied turn)
  ```
- **Start:** when the trigger fires AND no job is in flight, clone the graph (or the active layer's relevant state), bump nothing, record `graph_gen` as the job's `gen`, and `std::thread::spawn` a worker that runs the same relayout `tidy_layer_silent` uses, on the CLONE, and returns the tidied clone. Requires `MapGraph: Clone + Send` (add derives if missing — verify first).
- **Poll/apply (each loop iteration):** if `tidy_job` is `Some` and `handle.is_finished()`, join it. If the job's `gen == graph_gen` (graph unchanged since snapshot), copy the worker's final positions into the real graph for rooms that still exist, then re-center on the current room if it moved off-screen (same as today's post-tidy recenter). If `gen != graph_gen` (graph changed mid-tidy), DISCARD the stale result and immediately re-trigger a fresh tidy. Clear `tidy_job`.
- **Coalescing:** while a job is in flight, do NOT spawn a second. If a later turn also wants a tidy, the `gen` check after completion handles staleness (discard + re-run). Keep a simple `dirty` notion via the `gen` comparison; no queue of jobs.
- **No shared mutable state:** the worker only touches its own clone and returns it; the real graph is read/written solely on the main thread. This is the whole safety argument.

## Part 3 — Render loop stays live (animate the pulse)

The pulse needs frames to advance while waiting. If the loop currently blocks on `event::read()`, change it to: while `tidy_job.is_some()`, poll with a short timeout (e.g. `event::poll(Duration::from_millis(33))` ≈ 30 fps) and redraw on timeout so the border animates; when idle (no job), the existing blocking/poll behavior is fine. (If the loop already polls with a timeout, just ensure the timeout is short enough while a job runs.)

## Part 4 — Pulsing border color

In `render/map.rs` where the map pane's border/block is drawn:

- If `state.tidy_job.is_some()`, compute a phase from `started.elapsed()` (e.g. `t = elapsed_secs; f = (sin(t * TAU * PULSE_HZ) + 1)/2` with `PULSE_HZ ≈ 1.0`) and set the border color to a lerp between red `(220,60,60)` and green `(60,200,90)` by `f`. Use a smooth `Color::Rgb` interpolation.
- Else, use the normal border color (current behavior / `state.colors`).
- Only the BORDER color changes; interior rendering is untouched. Keep the focused/unfocused title behavior intact.

`PULSE_HZ`, the two endpoint colors, and the poll FPS are small named constants.

## Testing

- `should_bg_tidy`: `EveryRoom` returns true when `overlap=true, new_room=false`; `Debounced` returns true immediately on `overlap` regardless of counter; `Off` always false; `OnOverlap` unchanged. (Pure function — easy unit tests.)
- Overlap-signal plumbing: a small test that the run-loop helper computes `overlap=true` for a layer with a duplicate cell / a distorted active-layer edge. (Extract the signal into a testable helper if it isn't already.)
- Border color: a pure helper `pulse_border_color(elapsed: Duration) -> Color` (or `(normal, Some(job_started)) -> Color`) unit-tested: at phase 0 ≈ red endpoint, at half-period ≈ green endpoint, and `None`/idle returns the normal color. Keep the lerp/phase math in a pure function so it's testable without a worker.
- Async lifecycle: a test that exercises the snapshot→apply path with a fake/immediate worker if feasible (e.g. factor the "apply tidied positions if gen matches, else discard" into a pure function `apply_tidy_result(real, result, gen, cur_gen)` and unit-test both the matching and stale-gen branches). The thread spawn itself need not be tested.
- A render (TestBackend) check that with `tidy_job = Some(..)` the map border cell style differs from the idle border (smoke).
- Manual-verify (note in report, not unit-tested): the 180/80 repro now separates after the conflicting edge, and the border visibly pulses on a large map tidy.

## Out of scope / non-goals

- Progress BAR (deferred; only the pulsing border this round).
- Changing the manual `Retidy` / animated `AnimateTidy` paths (they stay synchronous + frame-based as today).
- Any mapper layout ALGORITHM change (item 3 needs none — overlap-free + bent-distorted is already guaranteed by a relayout). Only `Clone`/`Send` derives if strictly required.
- Cancellation of an in-flight tidy (we coalesce via the `gen` check instead).

## Risks & limitations (accepted)

- **Stale results:** handled by the `gen` check (discard + re-run if the graph changed since snapshot). Turns are human-paced, so collisions are rare.
- **Clone cost:** cloning the graph each tidy is cheap relative to the relayout itself (bounded by MAX_NODES = 400).
- **Pulse on fast tidies:** for small maps tidy may finish in one or two frames, so the pulse barely shows — acceptable (it's meant for the big-map case the user cited).
- **Worker panics:** if the worker thread panics, join returns `Err`; treat as "discard result, clear job, leave graph as-is" and do not crash the app.
