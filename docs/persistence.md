# The persistence model

[← back to README](../README.md) · see also [Saves & persistence (feature highlights)](features/saves.md)

babelmap persists game progress at three distinct layers. They coexist and serve
different purposes: the game's own save, the host emulator snapshot, and an
automatic per-story layer that needs no explicit save at all. This page explains
what each one captures, when it triggers, and what survives.

## Terminology

- **Save State / Restore State** — the *host* (emulator) snapshot. Engine-neutral,
  save-anywhere, captures the whole machine. Invoked with Ctrl+S / `/save-state`
  and Ctrl+R / `/restore-state`. This is babelmap's own mechanism, not something
  the game knows about.
- **`@save` / `@restore`** — the *game's* own in-game save, the standard path a
  story invokes when the player types `SAVE` / `RESTORE`. On the Z-machine this
  produces a portable **Quetzal** file; on Glulx it produces the host snapshot blob.

The two are genuinely different files with different contents — keep the names
straight.

## Layer 1 — the game's own save (`@save` / `@restore`)

Player-initiated, from inside the story ("Type SAVE to save your position").

- **Z-machine:** babelmap writes a bare, standard **Quetzal** save (`.qzl`) — the
  same format other interpreters (e.g. `dfrotz`) read and write, all versions
  including v3 branch-form `@save`/`@restore`. VM state only: no map, no screen,
  no transcript. Implemented in `crates/zvm/src/quetzal.rs`.
- **Glulx:** the game's `@save`/`@restore` is served by the **host Save State**
  blob (`machine.save_state()`) — the same snapshot format Layer 2 uses. In
  `gvm-cli` this is written next to the story as `<story>.glksave`.

## Layer 2 — host Save State / Restore State (emulator snapshot)

babelmap's own save-anywhere snapshot, explicit and per-slot. Triggered by Ctrl+S
/ `/save-state`, the named-slot saves manager, and the "Save State & quit" prompt.

It captures the **entire machine plus babelmap's session context**: VM state, the
Glk window/stream tree and screen, the map, the transcript, turn history, and
metadata. Crucially it **includes the entire Glk file VFS** — every file a Glulx
game has written through Glk file streams — via the SQ-0277 v4 `Glk ` snapshot
(`crates/gvm/src/glk.rs`, `GLK_SNAPSHOT_VERSION = 4`).

Save States are bundled into a self-contained `.babelmap` archive
(`crates/app/src/archive.rs`). Inside the archive the engine-tagged VM save is
`game.qzl` for the Z-machine and `game.glksave` for Glulx. Named slots, auto-save
(per turn) and auto-load (resume on launch) all operate on this layer.

## Layer 3 — automatic per-story persistence (no explicit save)

This layer needs **no player action and no Save State**. babelmap keeps a small
per-story sidecar that it loads when the story opens and flushes after each turn
(only when it changed). It is what makes a game's own external-storage files
survive a plain quit — quit the game normally, relaunch, and the data is still
there. For example, Kerkerkruip's persistent scores/preferences stick across
sessions.

- **Z-machine — aux data.** Games that use the v5 `@save` / `@restore`
  auxiliary-file mechanism (save/restore of a memory table to a named external
  file) persist to
  `<save_dir>/<ifid>.aux` in the app (`crates/app/src/aux_store.rs`) and to
  `<story>.aux` next to the story in `zvm-cli` (`ZAUX` format,
  `crates/zvm-cli/src/aux.rs`).
- **Glulx — the Glk file VFS (new, SQ-0278).** Every file a Glulx game writes
  through Glk file streams now auto-persists. The app keys it by IFID at
  `<save_dir>/<ifid>.gvfs` (`crates/app/src/vfs_store.rs`); `gvm-cli` keys it by
  story path at `<story>.glkvfs` (`crates/gvm-cli/src/main.rs`). The blob is the
  files-only `GVFS` codec (`gvm::glk::encode_files` / `decode_files`): magic
  `GVFS` + version `1` + length-prefixed name→bytes entries, big-endian, fully
  tolerant of a corrupt or foreign file (it just resets to empty, never panics).
  Session-scoped Glk temp files (VFS keys beginning with `__temp_`) are
  deliberately **not** persisted.

  Loaded at story-open (`main.rs`, alongside the aux load) and flushed per-turn
  dirty-gated (`persist_vfs_after_turn`), exactly mirroring the aux store. This is
  the automatic, no-explicit-save counterpart to Layer 2: Save State already
  embeds the full VFS per-slot, but Layer 3 is what preserves those files when the
  player never saves at all.

Deleting the sidecar (or `--no-aux` in the CLIs) resets the game's stored data.

## Where each thing lands

`<save_dir>` is babelmap's saves directory under the user's config/data dir.
`<ifid>` is the app's deterministic per-story key (`compute_ifid`), used for both
engines. The CLIs have no IFID and key on the story path instead.

| Layer | Engine | Host | File |
|-------|--------|------|------|
| 1 — game's `@save`/`@restore` | Z-machine | app | Quetzal `.qzl` (VM state only) |
| 1 — game's `@save`/`@restore` | Z-machine | `zvm-cli` | Quetzal `.qzl` |
| 1 — game's `@save`/`@restore` | Glulx | `gvm-cli` | `<story>.glksave` (host snapshot) |
| 2 — Save State / Restore State | Z-machine | app | `.babelmap` archive (`game.qzl` inside) |
| 2 — Save State / Restore State | Glulx | app | `.babelmap` archive (`game.glksave` inside; embeds full Glk VFS) |
| 3 — auto per-story (aux) | Z-machine | app | `<save_dir>/<ifid>.aux` |
| 3 — auto per-story (aux) | Z-machine | `zvm-cli` | `<story>.aux` (`ZAUX`) |
| 3 — auto per-story (Glk VFS) | Glulx | app | `<save_dir>/<ifid>.gvfs` (`GVFS`) |
| 3 — auto per-story (Glk VFS) | Glulx | `gvm-cli` | `<story>.glkvfs` (`GVFS`) |

## Known limitations (Glk file VFS)

Two SQ-0277 limitations remain in effect and apply to the VFS at every layer:

- **`create_by_prompt` uses a fixed per-usage name** — there is no interactive
  file picker yet, so "save to a file" prompts resolve to a single fixed name per
  usage class rather than a player-chosen filename (tracked as SQ-0279).
- **Text-mode newline translation is omitted** — Glk text-mode file streams are
  stored verbatim, with no platform newline translation.
