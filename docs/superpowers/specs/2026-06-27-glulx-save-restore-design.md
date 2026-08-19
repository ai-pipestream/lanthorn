# Glulx Save/Restore (lanthorn's own archives) — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crates:** `crates/gvm` (the Glk-model-in-save fix) + `crates/app` (routing)

## Goal

Make Glulx games **save and restore** in lanthorn — quick save/restore, named
saves, restore-from-archive, and auto-load-on-launch — by (1) making a `gvm`
snapshot **self-contained** (it currently omits the Glk window model, breaking
cross-session restore) and (2) routing the app's archive save/restore through
the engine-neutral `Engine::save_state`/`restore_state` (which already exist).
Turns the "not supported for Glulx" guards into working saves.

**Out of scope (parked):** cross-interpreter Glulx Quetzal (sharing saves with
Glulxe/Lectrote) — bundled with the future `@save`/`@restore` Glk-file-stream
work; gvm's self-contained snapshot is fine for lanthorn's own archives. Standard
`.qzl`/`.sav` import/export stays Z-machine-only.

## Background

- `Engine::save_state() -> EngineSave` / `restore_state(&EngineSave)` exist
  (3b-i), engine-tagged with a foreign-engine guard. zvm wraps Quetzal; Glulx
  wraps `gvm::save_state`. **For zvm the bytes equal today's Quetzal**, so
  existing archives stay loadable.
- `gvm::save_state` (2c) writes `FORM IFZS`: `IFhd`/`CMem`/`Stks`/`MAll`/`GReg` —
  RAM, stack, heap, registers. It was built **before** the Glk model (3a), so it
  does **not** serialize `glk::Model` (the window tree + streams + current
  stream/style). Same-session restore survives (the live `Model` is untouched);
  cross-session restore breaks (a fresh session has no windows).
- The app archive (`archive.rs`) is Z-machine-coupled: `save_archive_meta(&machine,…)`
  calls `machine.save_quetzal()` + serializes the Z-machine `ScreenState`
  (`screen.json`). Entries: `game.sav`, `screen.json`, `engine.txt` (tag, 3b-i),
  `map.json`, `transcript.json`, etc. The user-save handlers reach
  `zvm_session(...).machine` (guarded to bail for Glulx today).

## Design

### Phase A — `gvm`: include the Glk model in the snapshot

Add a `Glk ` chunk to `save_state`/`restore_state` serializing `glk::Model`:
- the window tree (each `Window`: id slot, `WinType`, rock, its stream id, and
  the pair-window fields — split direction/method/size/children),
- `root`, `cur_stream`, `cur_style`,
- the streams (`Stream`: id slot, kind, for **memory** streams the buffer
  address/length/positions and rock; window streams reference their window),
- the **text-grid** windows' cells + cursor (cheap; restores the status display).

The text-**buffer** windows' scrollback is **not** stored here — the primary
buffer is lanthorn's transcript (persisted separately via `transcript.json`);
extra buffer windows redraw (documented minor gap). `restore_state` rebuilds the
`Model` so the restored VM's window/stream references are valid. Back-compat: a
snapshot without a `Glk ` chunk restores with an empty model (old gvm saves) —
non-fatal. `GlulxSession::save_state` is unchanged (it already wraps
`gvm::save_state`, which now includes the model).

### Phase B — `app`: route archive save/restore through `Engine`

- `save_archive_meta` (and `save_named`/`save_archive`) take the **`EngineSave`**
  (from `engine.save_state()`) + the engine tag instead of `&machine`: write
  `EngineSave.bytes` → `game.sav`, the tag → `engine.txt`. For zvm this produces
  the **same `game.sav`** (Quetzal) as today.
- `screen.json` becomes a **zvm-only** extra: written/restored only when the
  engine is the Z-machine (via the `as_any` escape hatch), for v4+ visual
  continuity. Glulx's display lives inside its `EngineSave` (the `Glk ` chunk),
  so it does not need `screen.json`.
- Restore: read `game.sav` + `engine.txt` → `EngineSave` → `engine.restore_state()`;
  for zvm, also apply `screen.json` if present. The foreign-engine guard already
  refuses loading a Glulx save into a Z-machine session and vice versa, with a
  graceful message.
- **Remove the Glulx guards** on the now-working paths: quick save/restore
  (Ctrl+S/Ctrl+R), named save / save-as, restore-from-archive, the saves-manager
  load, replay-resume, launch "Resume", and the per-turn / on-exit autosave
  (they now go through `Engine` and work for both engines).
- **Restart**: route through the session **factory** (rebuild `Box<dyn Engine>`
  from the original story bytes via the Task-1 routing of 3b-ii) — engine-agnostic;
  remove its Glulx guard.
- **Keep** the Glulx guard ONLY on standard `.qzl`/`.sav` **import/export**
  (Quetzal interchange) — that stays Z-machine-only until cross-interpreter Glulx
  Quetzal exists.

### Back-compat

Existing `.lanthorn` archives: `game.sav` = raw Quetzal, `engine.txt` = `zmachine`
(or absent → default `zmachine`), `screen.json` present. These load unchanged:
`EngineSave { zmachine, <Quetzal> }` → `GameSession::restore_state` → `restore_file`,
plus `screen.json`. No migration needed.

## Testing

- **gvm:** a program opens windows + a memory stream, mutates state, `save_state`,
  resets to a **fresh** machine, `restore_state` → the window tree + streams +
  grid cells + cur_stream/style are restored; output then routes to the right
  windows (the cross-session case); a snapshot missing `Glk ` restores with an
  empty model (back-compat); same-session round-trip still exact.
- **app:** `save_archive_meta` for a zvm engine writes a `game.sav` byte-identical
  to the pre-change Quetzal path + `screen.json`; for a Glulx engine writes the
  tagged `EngineSave` and **no** `screen.json`. A round-trip: save a Glulx game's
  archive, build a fresh session, restore → state matches; the foreign-engine
  guard fires across engines. Existing-archive load (raw Quetzal, no `Glk `)
  still works. The guards are removed only from the engine-agnostic paths;
  `.qzl` import/export still guards Glulx.

## Global constraints

- `gvm` stays zero-dep; the `Glk ` chunk format is documented in `GLULX_NOTES.md`.
- Z-machine save/restore/restart is **byte-for-byte unchanged** (zvm `EngineSave.bytes`
  == today's Quetzal; `screen.json` still written/restored for zvm).
- Old `.lanthorn` archives and old gvm snapshots load unchanged.
- 0 warnings + full `cargo test --workspace` green per task.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
