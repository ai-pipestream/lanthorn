# Saves & persistence

[← back to README](../../README.md)

- **`.babelmap` Save States** — the emulator's own snapshot (Ctrl+S / `/save-state`
  and Ctrl+R / `/restore-state`), a single file bundling the map, VM state, screen,
  and transcript. By default a story starts fog-of-war (only what you've explored);
  opt into a shared default map with `use_default_map`.
- **Multiple named save slots** with a saves-manager modal (load / save-as /
  delete), each slot tracking name, turn count, and timestamp.
- **Import standard saves** — bring in a standard Quetzal `.qzl`/`.sav` game
  save from another interpreter via the saves manager's built-in file browser,
  keeping your accumulated map. Going the other way, the story's own `SAVE`
  already writes the portable standard `.qzl` (see below).
- **Standard in-game save/restore, all versions** — when a story runs its own
  `save`/`restore`, babelmap writes and reads a bare standard Quetzal `.qzl` save
  (the same format other interpreters use), now including v3 (Zork-era) games'
  branch-form `@save`/`@restore`. This is a genuinely different file from the
  emulator's `.babelmap` Save State: the game's `.qzl` holds VM state only, while
  the `.babelmap` Save State also carries the map, screen, and transcript. These
  standard `.qzl` saves are interoperability-tested against `dfrotz` in both
  directions (babelmap reads dfrotz's saves and vice-versa); run the live suite
  with `scripts/gen-interop-goldens.sh` or `cargo test -p zvm --test save_interop
  -- --ignored`. Glulx now also writes a real, standard in-game save via its own
  `@save`/`@restore` — VM state only, distinct from `.babelmap` — the same shape
  as the Z-machine's `.qzl`. Its round-trip is verified internally, but Glulx
  *cross-interpreter* save interop isn't golden-tested yet (tracked in SQ-0229).
- **Auto-save** (per turn) and **auto-load** (resume on launch) — both
  configurable.
- **Glulx external file storage** — a Glulx game's own Glk files (its
  transcripts, command recordings, and data files) are kept in an in-memory VFS
  that auto-persists per story across sessions, so those files survive a plain
  quit with no explicit save. When a game asks where to write
  (`create_by_prompt`), babelmap prompts for a name; when it asks which file to
  read, it shows a picker of the story's existing files. These files ride inside
  `.babelmap` Save States too. → [persistence model](../persistence.md)
- **Rewind / replay / resume** — with `record_turn_history` on, babelmap records
  a per-turn history (the game save plus a map snapshot and the transcript) into
  the `.babelmap` archive. Press **F4** (or `/open-history`) to open the replay
  modal: step or auto-play through every past turn with the map reconstructed as
  it looked at that moment, then resume the game from any earlier turn — undo that
  reaches back further than the game's own UNDO, and survives across sessions.
