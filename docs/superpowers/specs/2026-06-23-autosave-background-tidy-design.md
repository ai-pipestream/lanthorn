# Auto-save & Background Tidy — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued behind the hotkey-dialog track (shares `main.rs`/`config.rs`); also serial with the mouse track (both touch the event loop).
**TODO items:** "Optional auto-save (on each action) and auto-load with story start" (L21) and "Optional background map updates (background tidy on exploration)" (L29).

## Goal

Three optional, config-driven behaviors in the event loop: save the archive after every turn; control whether the game resumes from the archive at startup; and automatically re-tidy the map as new rooms are discovered.

## What already exists (don't rebuild)

- **Auto-load is already on:** startup loads `<ifid>.lanthorn` and calls `restore_quetzal` unconditionally when an archive exists (main.rs:359-362); the map always loads. L21 only makes the *game resume* optional and adds *per-turn* saving.
- **`save_archive_meta` / `load_archive`** exist (archive + saves work). Per-turn save reuses `save_archive_meta`.
- **`Mapper::observe`** places new rooms **incrementally** (`place_incremental`, stable, no relayout). L29 optionally runs a full re-tidy on top.
- **`run_tidy_pipeline(graph, layer)`** (app/input.rs) runs the auto-tidy and returns animation frames; the background path uses its final result WITHOUT animation playback.

## Config additions (extend Track B `Config`)

```rust
#[serde(default = "default_true")] pub auto_load: bool,   // default true (current behavior)
#[serde(default)]                  pub auto_save: bool,   // default false
#[serde(default)]                  pub background_tidy: BackgroundTidy,  // default EveryRoom

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTidy {
    Off,
    #[default] EveryRoom,   // re-tidy whenever a turn discovers a new room
    OnOverlap,              // re-tidy only when incremental placement caused an overlap/distortion
    Debounced,              // re-tidy once every K new rooms (K below)
}
```
TOML: `auto_save = true`, `auto_load = false`, `background_tidy = "on_overlap"`. `Debounced` uses a fixed `const BG_TIDY_DEBOUNCE: u32 = 5;` (a new room counter; re-tidy every 5).

> **Note for review:** `background_tidy` defaults to `EveryRoom` per the decision — this CHANGES today's behavior (no auto-tidy). Set `background_tidy = "off"` to keep the manual-only behavior. Flagged here so the default is a conscious choice.

## L21 — Auto-load (game resume optional)

At startup (main.rs ~359-362), when the archive loads: always use `ac.mapper` (the map). Gate the `restore_quetzal(&ac.save)` call on `cfg.auto_load`:
- `auto_load = true` (default): restore the game state — resume where you left off (today's behavior).
- `auto_load = false`: skip `restore_quetzal` — the game starts fresh (new playthrough), while the accumulated map still shows. The first `apply_turn("")` seed observe runs as it does today, so the current room is mapped.

## L21 — Auto-save (per turn)

In the event loop, right after the existing `apply_turn(&mut mapper, &cmd, &result)` (main.rs ~566), if `cfg.auto_save`: call `save_archive_meta(&arc_file, &mapper, &session.machine, meta)` (build `meta` the same way the exit-save path does, with the current `state.turns`). Failures are non-fatal (log to the status line, like the exit save's fallback). This is in addition to the existing exit-save and `Ctrl+S` quick-save.

## L29 — Background tidy

In the event loop, after `apply_turn` (which ran `observe`/`place_incremental`), decide whether to re-tidy based on `cfg.background_tidy` and whether this turn discovered a NEW room:
- Detect a new room by comparing `mapper.graph` room count before vs after `apply_turn` (capture the count before the call).
- **Off:** never.
- **EveryRoom:** if a new room was added this turn, re-tidy.
- **OnOverlap:** re-tidy only if, after incremental placement, the active layer has a room overlap OR a newly-distorted edge (check `mapper::layout::occupied_cells_in_layer` for duplicate cells / the `distorted` flags on the layer's connections).
- **Debounced:** increment a `new_rooms_since_tidy` counter on each new room; when it reaches `BG_TIDY_DEBOUNCE`, re-tidy and reset.

**Re-tidy execution (silent, no animation):** run the tidy on the **active layer** and apply the final positions/distortion directly — do NOT start a `TidyAnim` playback. Reuse `run_tidy_pipeline(&mut mapper.graph, active_layer)` and simply discard the returned frames (the pipeline already writes back the final positions and distortion to the graph), OR factor a `tidy_layer_silent(&mut graph, layer)` helper that runs the same stages without building frame snapshots. Keep the player's current room in view (re-center if it moved off-screen — reuse the existing `recenter_on`).

## Footprint

`crates/app/src/main.rs` (startup restore gate; post-turn auto-save; post-turn background-tidy + new-room detection), `crates/app/src/config.rs` (the three settings + `BackgroundTidy` enum + `default_true`). Possibly a small `tidy_layer_silent` helper next to `run_tidy_pipeline` in `crates/app/src/input.rs`. Do NOT modify `mapper` (reuse `relayout_auto`/`run_tidy_pipeline`/`occupied_cells_in_layer`).

## Testing

- Config: `auto_load` defaults true, `auto_save` false, `background_tidy` `EveryRoom`; each parses from TOML (`background_tidy = "on_overlap"` → `OnOverlap`).
- `tidy_layer_silent` (if added): runs the same stages as `run_tidy_pipeline` and leaves the graph in the same final state, returning no frames; a single-room layer is a no-op.
- Background-tidy decision logic (extract a pure `fn should_bg_tidy(mode, new_room: bool, overlap: bool, counter: &mut u32) -> bool`): `Off`→false always; `EveryRoom`→`new_room`; `OnOverlap`→`overlap`; `Debounced`→true only every K-th new room.
- Auto-load gate: a helper that, given `auto_load`, decides whether to restore — true→restore, false→skip (unit-test the boolean path; the actual `restore_quetzal` is integration-level).
- The headless smoke test still passes.

## Out of scope / non-goals

- Threaded/async tidy (it stays synchronous in the loop, just automatic).
- Auto-save throttling/debounce (every-turn writes are fine at IF turn rates); a future option could batch.
- Snapshot history / rewind (separate TODO L30).
- Changing the incremental placement itself.

## Risks & limitations (accepted)

- **`background_tidy = EveryRoom` by default** reorganizes the map on each new room (a full relayout can move existing rooms) — chosen for a maximally-clean map; `Off`/`OnOverlap` are the calmer options.
- **Per-turn auto-save** writes the archive each turn (a zip with map+save); negligible at IF turn rates but real I/O.
- **Auto-load default-on** means an existing archive resumes the game on launch; `auto_load = false` gives a fresh start with the map retained.
