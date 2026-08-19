# Map Loads With Lanthorn State + `/load_map` Command — Design

**Date:** 2026-07-08
**Status:** Approved for planning

## Goal

A map must never auto-load on its own. It loads **only** as part of a `.lanthorn`
archive's state (whether that state is auto-resumed at startup or loaded
explicitly). Standalone map files enter **only** on demand via a new
`/load_map <path>` command.

## Current behavior (the problem)

Startup (`crates/app/src/main.rs:1736-1786`) loads the map from three places, all
too eager:

1. From the per-story `.lanthorn` archive — `ac.mapper` is adopted
   **unconditionally** (`main.rs:1771`) whenever the archive exists, *independent*
   of `cfg.auto_load` (which today gates only the game-state VM restore). So even
   when you start a fresh game (`auto_load = false`), the accumulated map still
   loads.
2. From a legacy standalone `<ifid>.map.json` (`main.rs:1780`, migration fallback).
3. From `cfg.use_default_map` (`main.rs:1783`), a shared default map.

Paths 2 and 3 are standalone-map auto-loads; path 1 loads the map decoupled from
the state it belongs to.

The explicit-load paths are already correct and need **no change**: Ctrl+R
(`main.rs:3314`), saves-list restore/load (`main.rs:3542`, `3576`), and `/load`
(`main.rs:3967`) all adopt the archive's embedded map together with the state.

## Design

### 1. Couple the startup map load to the state load

At startup, adopt the archive's map under the **same `cfg.auto_load` gate** as the
game-state restore, so state and map load together as one unit:

- `auto_load = true` (default) → restore VM state **and** `ac.mapper`.
- `auto_load = false` → fresh game **and** `Mapper::default()` (blank map).

This is the one behavior change: a fresh start no longer inherits the old map.

### 2. Remove all standalone auto-load

- Delete the `else if map_file.exists() { load_map(...) }` branch (`main.rs:1780`).
- Delete the `else if cfg.use_default_map { load_map(...) }` branch (`main.rs:1783`).
- The startup `else` becomes simply `Mapper::default()`.
- **Remove the `use_default_map` config field entirely** and every site that
  references it: the struct field + doc comment, the `Default` impl, the
  `resolve()` merge, `write_config()`, the config-screen row list + value renderer
  + toggle handlers (renumbering the positional config-screen rows), and any tests.
- Any standalone-map **write** paths orphaned by this (e.g. the exit-save
  `save_map` fallback around `main.rs:3819`, and the `map_path`/`map_dir` helpers if
  they fall fully unused) are removed as part of the cleanup — verified via the
  compiler's dead-code/unused warnings. `load_map` stays (used by `/load_map`).

### 3. New `/load_map <path>` command

The only way a standalone map enters. Registered in the `slash::COMMANDS` registry
(verb-noun; the single source for commands) with a new `Action`.

- **Argument:** a filesystem path. Expand a leading `~` to the home directory
  (slash-command args are not shell-expanded), then resolve relative to cwd.
- **Behavior:** parse the file (`load_map` / `mapper::persist::from_json`); on
  success, **replace** the current session's mapper with the loaded one and trigger
  a redraw. Positions come from the file, so no relayout is forced. No confirmation
  prompt — the current map is safe in the archive if it was saved.
- **Errors:** a missing file or parse failure shows a one-line status/toast message
  (the app's existing slash-error surface); it never panics and leaves the current
  map untouched.
- **Usage string / help:** `/load_map <path>` — "Load a standalone map file into
  the current session."

## Components / files

- `crates/app/src/main.rs` — gate the archive map load on `cfg.auto_load`; delete
  the two standalone-map startup branches; remove orphaned standalone-map writes.
- `crates/app/src/config.rs` — remove the `use_default_map` field and all its
  plumbing.
- `crates/app/src/render/config_screen.rs` + `crates/app/src/input.rs` — remove the
  `use_default_map` config-screen row + toggle handlers; renumber positional rows.
- `crates/app/src/slash.rs` (+ the `Action` enum and its handler in `input.rs`) —
  register `/load_map` and implement the load-replace-redraw action.
- `crates/app/src/persist_files.rs` — reuse `load_map`; remove `save_map` only if it
  becomes fully unused.

## Testing

- **Startup coupling:** with a `.lanthorn` archive present, `auto_load = true` yields
  the archive's map; `auto_load = false` yields an empty map. (Unit-test the load
  decision, or a focused integration around the startup mapper selection.)
- **No standalone auto-load:** a bare `<ifid>.map.json` on disk is ignored at
  startup (no map loaded when no archive and `auto_load = false`).
- **`use_default_map` removed:** config parses/round-trips without it; a config TOML
  that still contains `use_default_map` is ignored gracefully (serde default).
- **`/load_map`:** loading a valid map file replaces the current session map (assert
  a room from the file is present); `~` is expanded; a missing/invalid path leaves
  the current map unchanged and reports an error, no panic.
- Full `cargo test -p app` + `cargo test -p mapper` stay green.

## Out of scope

- A file-browser picker for `/load_map` (path argument only for now).
- Any change to the explicit save/restore/`/load` paths (already correct).
- Merging a loaded map into the current one (it replaces).
