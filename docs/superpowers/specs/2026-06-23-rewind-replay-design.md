# Rewind / Replay — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued; large, implement in phases. Touches `archive.rs`, `config.rs`, `state.rs`, `main.rs`, `input.rs`, `keymap.rs`, new `render/history.rs`.
**TODO item:** "Optional game state rewind, resume from specific state, and step by step replay. Integrated into zipped savefile." (L30)

## Goal

Optionally record a per-turn history of the game (VM state + map), stored in the `.babelmap` archive, and let the player **replay** turn-by-turn, **rewind**, and **resume** from any past turn (linear — resuming discards later turns).

## Decisions

- **Full history** (every turn recorded) when enabled.
- **Rewind the map too:** restoring a past turn restores both the VM state and the map as it stood then.
- **Linear resume:** resuming from turn N truncates history to `[0..=N]` and continues live.
- **Optional:** gated by a config flag `record_history: bool` (default `false`) — capturing a Quetzal save + (sometimes) a map snapshot each turn has overhead, so it is opt-in.

## Data model

```rust
// crates/app/src/history.rs (new)
pub struct TurnRecord {
    pub turn: u32,
    pub command: String,
    pub save: Vec<u8>,             // Quetzal snapshot of the VM AFTER this turn
    pub map_snapshot: Option<String>, // serialized Mapper, ONLY when the map changed this turn
    pub transcript: String,        // the game output for this turn
}
```
`AppState.history: Vec<TurnRecord>`. Map snapshots are stored only on turns where the map changed (room count / graph differs); a rewind to turn N reconstructs the map from the **latest `map_snapshot` at-or-before N** (so storage is ~#map-changes, not #turns).

## Capture (event loop, `main.rs`)

After the existing per-turn work (`session.submit(&cmd)` → `apply_turn` → bg-tidy), when `cfg.record_history`:
- push a `TurnRecord { turn: state.turns, command: cmd.clone(), save: session.machine.save_quetzal(), map_snapshot: (changed).then(|| mapper::persist::to_json of mapper), transcript: result.transcript.clone() }`.
- "changed" = the map's room count (or a cheap graph hash) differs from the previous record's effective map.
The `record_history=false` path adds nothing.

## Archive persistence (`archive.rs`)

Extend the `.babelmap` zip with history entries (bump `Meta.format_version` to 2; v1 archives load with empty history):
- `history/index.json` — `Vec<{ turn, command, has_map: bool }>` (the per-turn metadata + ordering).
- `history/turn-NNNN.sav` — the Quetzal bytes for each turn.
- `history/turn-NNNN.map.json` — the map snapshot for turns that have one.
- `history/turn-NNNN.txt` — the turn transcript.
`save_archive` writes these when history is non-empty; `load_archive` reads them back into `Vec<TurnRecord>` (added to `ArchiveContents`). Reuses the existing `ZipWriter`/entry pattern. Old archives (no `history/`) → empty history, unchanged behavior.

## Replay / rewind UI (`render/history.rs` + a sub-mode)

A modal opened by `Command::OpenHistory` (added to the keymap, default key + the hotkey "View"/"Files" group). `AppState.replay: Option<ReplayState { idx: usize, playing: bool }>` (the selected turn index into `history`).

While open (a sub-mode in `key_to_action`, like saves/gallery):
- **List + preview:** show the turn list (turn# + command), the selected turn's `transcript`, and render the MAP for the selected turn using its reconstructed map snapshot (a preview graph, the same way tidy-anim renders a frame's graph instead of the live map — `draw_frame` already supports rendering an alternate graph during `tidy_anim`; add an analogous "replay preview graph" path).
- **Transport:** `←/→` step one turn (replay); `Space` auto-play (advance on a timer, like `TidyAnim::tick`); `Esc`/`q` close (back to live, no change).
- **Resume:** `Enter` (or `r`) → **resume from the selected turn**: `session.machine.restore_quetzal(&record.save)`, set `mapper` to the reconstructed map snapshot at-or-before the turn, **truncate `history` to `[0..=idx]`**, reset the on-screen transcript to the history up to that turn, set `state.turns`, clear `replay`, and continue live. (Linear.)

## Config (`config.rs`)

`#[serde(default)] pub record_history: bool` (default false) on `Config`. (Documented: enabling records per-turn snapshots into the `.babelmap`, growing the file.)

## Keymap

`Command::OpenHistory` + `Action::OpenHistory` (opens the modal, seeding `replay` at the last turn) + default binding; appears in the hotkey dialog.

## Phasing (each independently testable)

1. **Capture infra** — `history.rs` `TurnRecord` + `AppState.history` + the config flag + per-turn capture in `main.rs` (map-snapshot-on-change). No UI yet. Tests: a few turns produce records with saves; map_snapshot present only on map-change turns; flag off → no records.
2. **Archive persistence** — `save_archive`/`load_archive` round-trip the history (new zip entries; `Meta.format_version=2`; v1 back-compat → empty history). Tests: round-trip N records; old archive loads with empty history.
3. **Replay/rewind modal** — `render/history.rs`, the sub-mode, `Command::OpenHistory`, the preview-graph render path, and the linear resume. Tests: stepping changes the selected turn + preview; resume restores VM+map and truncates history; modal render shows the turn list.

## Testing

- `history` capture: a turn appends a record with `save` non-empty and `transcript`; `map_snapshot` Some only when a room was added.
- Map reconstruction: `map_at_turn(history, n)` returns the latest snapshot at-or-before n.
- Archive round-trip (phase 2): write an archive with history, reload, records equal (saves byte-identical, commands, map snapshots, transcripts); a v1 archive → empty history.
- Resume (phase 3): from turn k, `restore_quetzal` is called with `history[k].save`, mapper equals the reconstructed snapshot, `history.len() == k+1`.
- Sub-mode keys: `←/→` move `replay.idx`; `Enter` resumes; `Esc` closes without change.
- Headless smoke test still passes.

## Out of scope / non-goals

- Branching / undo-trees (resume is linear).
- Editing past turns or injecting commands during replay.
- Compressing snapshots beyond the zip's Deflate (Quetzal is already a diff vs the story; map snapshots are only-on-change).
- `mapper`/`zvm` changes (both used read-only: `to_json`/`from_json`, `save_quetzal`/`restore_quetzal`).

## Risks & limitations (accepted)

- **Archive size:** full per-turn Quetzal saves grow the `.babelmap`; mitigated by Deflate + map-snapshot-on-change, and the feature is opt-in (`record_history=false` default).
- **Memory:** the full `history` lives in `AppState`; for very long sessions this is many Quetzal blobs in RAM. Acceptable for IF session lengths; a future cap could spill to disk.
- **Resume fidelity:** restoring the VM is exact (Quetzal); the on-screen transcript is reconstructed from stored per-turn text, which should match what the player saw.
- **`record_history` toggled mid-game:** history starts recording from when it is enabled; earlier turns are absent. Documented.
