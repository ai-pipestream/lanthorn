# Queued brief — Wire the `.lanthorn` archive into save/load (finish TODO L7)

**Status:** QUEUED — dispatch only AFTER the `wave3-keymap` (L8) track merges to `main` (this brief edits `main.rs` startup + `config.rs`, which L8 also touches). Create a fresh worktree off the post-L8 `main`.

**Depends on:** `crates/app/src/archive.rs` (already on `main`: `save_archive(path, &Mapper, &Machine)`, `load_archive(path) -> ArchiveContents { mapper, save, meta }`) and Track B's `Config`.

## Goal
Make the single-file `.lanthorn` archive the primary persistence container (map + game save together), and make the legacy "shared default map" behavior optional via config. Today the map (`mapper`) and the game save (Quetzal) are written to SEPARATE files; this unifies them.

## Current wiring (read these exact spots in `crates/app/src/main.rs`)
- Startup: `let map_file = map_path(&dir, &ifid);` (~262) then `let mut mapper = load_map(&map_file).unwrap_or_default();` (~264).
- `Action::SaveGame` (~451): `save_game(&save_slot, &session.machine)` — Quetzal to a separate slot. Read how `save_slot` is derived just above.
- `Action::RestoreGame` (~466): `restore_game(&save_slot, &mut session.machine)`.
- On exit (~553): `save_map(&map_file, &mapper)`.
`persist_files.rs` has `save_map`/`load_map` (map JSON) and `save_game`/`restore_game` (Quetzal). `ifid::map_path(base, ifid)` builds the per-story path.

## Decided behavior (implement this; flag in report if you deviate)
- **Archive path:** `<dir>/<ifid>.lanthorn` (add `fn archive_path(base, ifid)` to `ifid.rs` mirroring `map_path`, or derive inline).
- **Save (`Ctrl+S` SaveGame, and on exit):** write `archive::save_archive(&archive_path, &mapper, &session.machine)` — map + game in one file. Keep showing the existing "saved to …" message. (You may keep writing the legacy `.map.json` on exit too, OR stop — see config flag below.)
- **Load (startup, and `Ctrl+R` RestoreGame):** if `<ifid>.lanthorn` exists → `archive::load_archive`, use `ac.mapper` and `session.machine.restore_quetzal(&ac.save)`. RestoreGame restores the game half from the archive.
- **`use_default_map` config flag** (add `#[serde(default)] pub use_default_map: bool` to `Config`, default `false`):
  - `false` (default): a story with NO archive starts with an EMPTY map (`Mapper::default()`) — you only see what you explore (fog-of-war, self-contained).
  - `true`: fall back to the legacy shared map — load `load_map(&map_path(&dir,&ifid))` so a pre-existing/accumulated default map shows un-explored rooms.
- **Back-compat / migration:** if no `.lanthorn` exists but a legacy `<ifid>.map.json` does, load that map (so existing users keep their maps); the next save writes the `.lanthorn`. The game-save slot may stay as-is for back-compat, but new saves go in the archive.

## Footprint
`crates/app/src/main.rs` (startup load + SaveGame/RestoreGame handlers + exit save), `crates/app/src/config.rs` (additive `use_default_map`), `crates/app/src/ifid.rs` (archive_path helper), possibly a small helper in `persist_files.rs`. Reuse `archive.rs` — do NOT reimplement bundling. Do NOT touch `mapper`, `render/`, `input.rs`, `keymap.rs`.

## TDD
- A persistence round-trip test (can live in `archive.rs` tests or a new `persist_files` test): build a `Mapper` + `Machine`, `save_archive` then `load_archive`, assert the map and the restored game state match. (The archive module already has a round-trip test — extend for the wiring helpers if you add any.)
- A config test: `use_default_map` defaults to `false`; parses from TOML.
- Manual-ish: the headless smoke test still passes.
- `cargo test -p app` and `cargo build -p app` must pass.

## Report back
Branch; commit SHAs; how save/load now flow through the archive; the `use_default_map` semantics implemented; migration handling; test results; any deviation from the decided behavior (especially anything about the legacy `.map.json`/game-slot you kept or dropped, and why).

## Open decision for controller review
Whether to STOP writing the legacy `.map.json` once `.lanthorn` is the container, or keep both during a migration window. Default: keep writing `.map.json` only when `use_default_map = true`; otherwise the archive is the sole container. Confirm at review.
