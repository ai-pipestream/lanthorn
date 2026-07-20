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
  story invokes when the player types `SAVE` / `RESTORE`. On both engines this
  produces a portable, standard **Quetzal** file — on the Z-machine, Quetzal
  proper; on Glulx, standard **Glulx-Quetzal**.

The two are genuinely different files with different contents — keep the names
straight.

**A third engine, Scott Adams, has no Layer 1 at all.** The Scott VM has no
in-game Quetzal `@save`/`@restore` suspension protocol; its in-game `SAVE GAME`
action (opcode 71) instead routes to a host **Save State** snapshot (Layer 2),
and it keeps no Layer 3 sidecar. Wherever this page says "both engines" it means
the Z-machine and Glulx — the two engines with an in-game `@save`/`@restore`
(Layer 1); their in-game saves are Quetzal (`.qzl`) and Glulx (`.glksave`)
respectively.

## Layer 1 — the game's own save (`@save` / `@restore`)

Player-initiated, from inside the story ("Type SAVE to save your position").

- **Z-machine:** babelmap writes a bare, standard **Quetzal** save (`.qzl`) — the
  same format other interpreters (e.g. `dfrotz`) read and write, all versions
  including v3 branch-form `@save`/`@restore`. VM state only: no map, no screen,
  no transcript. Implemented in `crates/zvm/src/quetzal.rs`.
- **Glulx:** babelmap writes a bare, standard **Glulx-Quetzal** save
  (`machine.save_quetzal()`) — `IFhd`/`CMem`/`Stks`/`MAll` only, no `GReg`, no
  `Glk ` chunk. VM state only: no map, no screen, no VFS embed. Per the Glulx
  spec, `@save` pushes a call stub before suspending, so PC and FramePtr are
  recovered from the stack on restore rather than serialized as registers; the
  save is the same shape as the Z-machine's `.qzl`. Implemented in
  `crates/gvm/src/exec.rs` (`save_quetzal`/`restore_quetzal`). Round-trip is
  verified internally (gvm unit tests); cross-interpreter golden-file interop is
  tracked separately under SQ-0229.

  **Two kinds of Glulx `@save`/`@restore`, routed by how the game made the
  fileref (SQ-0296).** The VM carries the target file's name and a `by_prompt`
  flag to the host on each save/restore request (`Machine::pending_saveload_request`
  → `SaveLoadRequest { name, by_prompt, restore }`):
  - **Player SAVE/RESTORE verb** — the game opens the file with
    `glk_fileref_create_by_prompt` (`by_prompt = true`). The host surfaces its
    save UI: `gvm-cli` prompts `Save to file:` / `Restore from file:`; the app
    opens its saves dialog. Lands as `<slug>.qzl`.
  - **The game's OWN fixed-name saves** — `glk_fileref_create_by_name` /
    `create_by_usage` (`by_prompt = false`), e.g. Counterfeit Monkey's
    `_Counterfeit_Monkey-startup-data` init cache, its autosave, and undo slots.
    These are serviced **silently and automatically** — no prompt, no UI. `@save`
    writes `<game-dir>/<name>.qzl`; `@restore` reads that fixed path if present,
    else fails cleanly so the game runs its init. Because the file persists in the
    per-game `.save` dir, the next launch's boot `@restore` finds it and the game
    skips its (multi-second) init — measured for CM: ~3.5s first launch vs ~0.9s
    on relaunch. These internal `_`-prefixed files are hidden from the player
    saves list (app) and never prompt (both hosts).

## Layer 2 — host Save State / Restore State (emulator snapshot)

babelmap's own save-anywhere snapshot, explicit and per-slot. Triggered by Ctrl+S
/ `/save-state`, the named-slot saves manager, and the "Save State & quit" prompt.

It captures the **entire machine plus babelmap's session context**: VM state, the
Glk window/stream tree and screen, the map, the transcript, turn history, and
metadata. Crucially it **includes the entire Glk file VFS** — every file a Glulx
game has written through Glk file streams — embedded in the `Glk ` snapshot
(`crates/gvm/src/glk.rs`, `GLK_SNAPSHOT_VERSION = 6`; the VFS has been embedded
since v4, SQ-0277, and restore still accepts v4 onward).

Save States are bundled into a self-contained `.babelmap` archive
(`crates/app/src/archive.rs`). Inside the archive the engine-tagged VM save is
`game.glksave` for Glulx and `game.qzl` otherwise (the `save_ext` fallback, so
the Z-machine's Quetzal and the Scott VM's `Vm::snapshot` blob both land as
`game.qzl` — the recorded engine tag, not the extension, tells them apart on
restore). This is Scott Adams' **only** persistence layer: with no in-game
Quetzal save and no sidecar, its in-game `SAVE GAME` and the host Ctrl+S both
write here. Named slots, auto-save (per turn) and auto-load (resume on launch)
all operate on this layer.

## Layer 3 — automatic per-story persistence (no explicit save)

This layer needs **no player action and no Save State**. babelmap keeps a small
per-story sidecar that it loads when the story opens and flushes after each turn
(only when it changed). It is what makes a game's own external-storage files
survive a plain quit — quit the game normally, relaunch, and the data is still
there. For example, Kerkerkruip's persistent scores/preferences stick across
sessions.

- **Z-machine — aux data.** Games that use the v5 `@save` / `@restore`
  auxiliary-file mechanism (save/restore of a memory table to a named external
  file) persist to `<base>/<story-key>.save/default.aux` — in the app
  (`crates/app/src/aux_store.rs`) and in `zvm-cli` (`ZAUX` format,
  `crates/zvm-cli/src/auxiliary.rs`), each keyed by the story filename, not IFID.
- **Glulx — the Glk file VFS (new, SQ-0278).** Every file a Glulx game writes
  through Glk file streams now auto-persists to
  `<base>/<story-key>.save/default.glkvfs` — in the app
  (`crates/app/src/vfs_store.rs`) and in `gvm-cli`
  (`crates/gvm-cli/src/main.rs`), both keyed by the story filename. The blob is
  the files-only `GVFS` codec (`gvm::glk::encode_files` / `decode_files`): magic
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

## Storage layout (SQ-0284)

All three hosts — app, `zvm-cli`, `gvm-cli` — store saves and sidecars in a
flat **per-game directory**, one directory per story, holding everything for
that game side by side:

```
<base>/<story-key>.save/
    default.aux        # Z-machine aux sidecar (Layer 3)
    default.glkvfs     # Glulx VFS sidecar (Layer 3)
    default.babelmap   # the auto/singleton Save State slot (Layer 2)
    <slug>.babelmap     # named Save States (Layer 2, app only)
    <slug>.qzl           # in-game @save files (Layer 1)
    style.toml          # per-game style override (app only, layered over global)
    config.toml         # per-game non-style overrides (honor/borders/map panel)
```

`<story-key>` is the story's own **filename** (basename including extension,
sanitized to filesystem-safe characters) — *not* the IFID. The same story
file always maps to the same directory, and different files (even the same
game shipped as `.z5` vs `.zblorb`) get separate directories. The IFID is
still computed and used for the story's *title* and for interpreter-hint
association, but it no longer keys any storage path.

The directory name carries a **`.save` suffix** (`<story-key>.save`, e.g.
`Zork1.z5.save/`) so it can never collide with the story file itself — this
matters for `zvm-cli`/`gvm-cli`, whose default `<base>` is the story's own
directory, where a directory named exactly `Zork1.z5` would collide with the
file `Zork1.z5` (SQ-0294).

`<base>` — the directory containing all per-game directories — defaults
differently per host, and every host accepts `--data-dir <path>` to override
it:

- **app** — `~/.babelmap/saves` (i.e. `<user_dir>/saves`; follows
  `--user-dir` unless `--data-dir` is also given).
- **`zvm-cli` / `gvm-cli`** — the story file's own directory (so a story run
  from `~/games/zork1.z5` gets `~/games/zork1.z5/...`).

A save named `default` is a **reserved slug** — the app rejects an attempt to
create a named Save State or in-game save called `default`, since that name
is claimed by the auto/singleton slot.

### Interactive `@save` / `@restore` in the CLIs

When `zvm-cli` / `gvm-cli` prompt for a filename on the **player's** SAVE /
RESTORE verb (a `glk_fileref_create_by_prompt` fileref in Glulx; always in the
Z-machine), a **bare name** (no path separator, e.g. `@save quick`) resolves
into the per-game directory — `<base>/<story-key>.save/quick.qzl` — matching the
`.qzl` extension automatically. A **path-bearing value** (e.g.
`@save /tmp/x.qzl`) is honored verbatim, bypassing the per-game directory
entirely.

A Glulx game's **own** fixed-name saves (`glk_fileref_create_by_name`, e.g. CM's
init cache) do **not** prompt: `gvm-cli` writes/reads `<story-key>.save/<name>.qzl`
silently (see Layer 1, SQ-0296).

### Map/transcript exports (SQ-0288)

The app's `/export-svg`, `/export-dot`, `/export-map`, and `/export-transcript`
commands write into the same per-game directory, using fixed default names —
`map.svg`, `map.dot`, `map.txt`, `transcript.txt` — overwriting on repeat
export. Each takes an optional `[file]` argument that resolves the same way as
`@save`/`@restore` above: a **bare name** lands in `<base>/<story-key>.save/`
(the format's extension is appended if the name has none), a **path-bearing
value** is honored verbatim.

### No migration (alpha)

There is **no migration** from the old IFID-keyed layout. Saves and sidecars
previously written as `<save_dir>/<ifid>.babelmap`, `<ifid>.aux`, `<ifid>.gvfs`,
etc. are orphaned — babelmap will not find or move them automatically. If you
have saves from before this change, either re-create them under the new
layout or manually move the files into the new `<base>/<story-key>.save/`
directory (renaming to the `default.*` / `<slug>.*` names above as needed).

## Where each thing lands

| Layer | Engine | Host | File |
|-------|--------|------|------|
| 1 — game's `@save`/`@restore` | Z-machine | app | `<base>/<story-key>.save/<slug>.qzl` (VM state only) |
| 1 — game's `@save`/`@restore` | Z-machine | `zvm-cli` | `<base>/<story-key>.save/<slug>.qzl` (bare name) or verbatim path |
| 1 — player SAVE verb (`create_by_prompt`) | Glulx | app | `<base>/<story-key>.save/<slug>.qzl` (VM state only) |
| 1 — player SAVE verb (`create_by_prompt`) | Glulx | `gvm-cli` | `<base>/<story-key>.save/<slug>.qzl` (bare name) or verbatim path |
| 1 — game's own save (`create_by_name`, SQ-0296) | Glulx | app & `gvm-cli` | `<base>/<story-key>.save/<name>.qzl` — silent, no prompt; hidden from the saves list |
| 2 — Save State / Restore State | Z-machine | app | `<base>/<story-key>.save/default.babelmap` or `<slug>.babelmap` (`game.qzl` inside) |
| 2 — Save State / Restore State | Glulx | app | `<base>/<story-key>.save/default.babelmap` or `<slug>.babelmap` (`game.glksave` inside; embeds full Glk VFS) |
| 2 — Save State / Restore State | Scott Adams | app | `<base>/<story-key>.save/default.babelmap` or `<slug>.babelmap` (`game.qzl` inside = `Vm::snapshot` blob; Scott's only layer) |
| 3 — auto per-story (aux) | Z-machine | app | `<base>/<story-key>.save/default.aux` |
| 3 — auto per-story (aux) | Z-machine | `zvm-cli` | `<base>/<story-key>.save/default.aux` (`ZAUX`) |
| 3 — auto per-story (Glk VFS) | Glulx | app | `<base>/<story-key>.save/default.glkvfs` (`GVFS`) |
| 3 — auto per-story (Glk VFS) | Glulx | `gvm-cli` | `<base>/<story-key>.save/default.glkvfs` (`GVFS`) |
| export — `/export-svg`\|`-dot`\|`-dump`\|`-transcript` | either | app | `<base>/<story-key>.save/map.svg`\|`map.dot`\|`map.txt`\|`transcript.txt` (bare `[file]` arg) or verbatim path |

`<base>` and `<story-key>` are as defined in [Storage layout](#storage-layout-sq-0284)
above.

## `create_by_prompt` naming (SQ-0279)

`glk_fileref_create_by_prompt` suspends the VM for a host-chosen name rather than
resolving to a fixed per-usage slot. Write / append / read-write modes open a
name-entry prompt; read mode opens a picker over the story's existing Glk files.
The named file lives in the VFS like any other Glk file, so it auto-persists
per-story through the Layer 3 sidecar (`default.glkvfs`) and is embedded in
Layer 2 Save States — there is no separate on-disk file. `gvm-cli` prompts for the
name on stdin (blank cancels). This matches the layering above: a game reaching for
`create_by_prompt` is writing an *external named file*, which by the game's own
choice belongs in the automatic per-story (global) layer, not a save slot.

**Exception — `fileusage_SavedGame`.** A `create_by_prompt` stream opened for
saved-game usage does **not** resolve into a VFS slot at all: it's a host
conduit (`StreamKind::Null`) that discards writes and reads EOF, with no
`self.files` entry and nothing persisted to `default.glkvfs` or embedded in a
Save State. The library's post-`@save` verification is satisfied without storing
bytes: `note_stream_write` credits the stream (and records the slot's byte
length), so `glk_fileref_does_file_exist` reports the slot exists and a reopen +
seek-to-end reports the true save size — CM's SAVE verb otherwise printed "Save
failed." (SQ-0292). The game's `@save`/`@restore` always reaches the opcode —
even on a first-ever restore, with no prior save this session — and the *host*
decides success by writing/reading the actual `.qzl` (Layer 1, above). Net: the
VFS (Layer 3) now holds only the game's genuine external files — transcripts,
command recordings, and data files — never saves.

**Game-managed vs. player-prompted (SQ-0296).** The above concerns the
*player's* verb (`create_by_prompt`). A game that saves to a **fixed-name**
fileref (`create_by_name`/`create_by_usage` — CM's `_Counterfeit_Monkey-startup-data`
init cache, its autosave, undo slots) is routed differently: the host writes/reads
`<game-dir>/<name>.qzl` **silently, with no prompt**, keyed by the fileref name
the VM now reports (`SaveLoadRequest.by_prompt = false`). This is what makes CM's
boot cache auto-restore on relaunch (skipping its long init) and removes the
spurious boot prompts. Note a Glulx game may open such a slot as a `Data`-usage
VFS `File` stream rather than a `SavedGame` `Null` stream — CM does — so the
name/`by_prompt` routing covers both stream kinds.

## Known limitations (Glk file VFS)

- **The read picker is not usage-filtered** — it lists *all* of the story's VFS
  files, not only those matching the requested Glk usage class, because the `GVFS`
  codec does not record a per-file usage tag.
- **Text-mode newline translation is omitted** — Glk text-mode file streams are
  stored verbatim, with no platform newline translation.
