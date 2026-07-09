# Saves & persistence

[← back to README](../../README.md)

- **`.babelmap` archives** — a single file bundling the map, the game save, and
  metadata. By default a story starts fog-of-war (only what you've explored);
  opt into a shared default map with `use_default_map`.
- **Multiple named save slots** with a saves-manager modal (load / save-as /
  delete), each slot tracking name, turn count, and timestamp.
- **Import / export standard saves** — exchange standard Quetzal `.qzl`/`.sav`
  files with other interpreters via the saves manager (a built-in file browser
  picks the file/destination). Importing keeps your accumulated map.
- **Standard in-game save/restore, all versions** — when a story runs its own
  `save`/`restore`, babelmap writes and reads a standard Quetzal save (the same
  format other interpreters use), now including v3 (Zork-era) games' branch-form
  `@save`/`@restore`. This is separate from the emulator-style host snapshot
  (Ctrl+S/Ctrl+R and the `.babelmap` archive), which can save anywhere.
- **Auto-save** (per turn) and **auto-load** (resume on launch) — both
  configurable.
- **Rewind / replay / resume** — with `record_turn_history` on, babelmap records
  a per-turn history (the game save plus a map snapshot and the transcript) into
  the `.babelmap` archive. Press **F4** (or `/open-history`) to open the replay
  modal: step or auto-play through every past turn with the map reconstructed as
  it looked at that moment, then resume the game from any earlier turn — undo that
  reaches back further than the game's own UNDO, and survives across sessions.
