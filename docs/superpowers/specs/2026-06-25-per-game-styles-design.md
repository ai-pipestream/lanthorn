# Per-Game Style Overrides — Design

**Date:** 2026-06-25
**Status:** Draft, pending user review
**Depends on:** Live Style Reload (`reload_style` is the single resolution point) and #82 — implement after both merge.

## Goal

Let each game have its own style customization layered on the global `style.toml`,
so different adventures can have different looks. Per-game overrides are thin,
hand-authored files keyed by IFID, merged over the global style.

## Background (current state)

- After the live-reload work, styling resolves through one path: `reload_style`
  reads the global `style.toml` (the pointer in `config.style`), `parse_style_toml
  → resolve`, and swaps `state.colors` / `state.symbols`. It runs at startup, on
  `/reload`, and on a file-watch event.
- Other per-game state is keyed by IFID: saves are `<saves_dir>/<ifid>.babelmap` /
  `<ifid>-<slug>.babelmap`; hint-file associations and the `.babelmap` map archive
  are likewise per-IFID. The run loop already holds the current `ifid` and the
  resolved adventure `title` (`state.title`).
- `merge(base: &StyleDoc, over: &StyleDoc) -> StyleDoc` already implements
  layered styling: `[colors]` selectors merge per-key (over wins per selector),
  `[symbols]` merge per-field + overrides union, `transcript_rules` and the
  `status_bar` block replace-if-the-override-has-any.

## Design

### 1. Storage & auto-load

- A game's override lives at **`user_dir/styles/<ifid>.toml`** (a new `styles/`
  directory beside `saves/`).
- When a game starts, the active style is **`merge(global, per_game)`** then
  `resolve`, where `per_game` is `styles/<ifid>.toml` parsed if it exists (else an
  empty `StyleDoc`, i.e. global only). This is computed inside `reload_style`, so
  startup / `/reload` / file-watch all produce the merged result.

### 2. Merge scope — what per-game can override

Per-game files override **everything** `style.toml` owns, via the existing
`merge(global, per_game)` semantics — no carve-outs:

- `[colors]` selectors merge per-key (a per-game selector overrides just that one).
- `[symbols]` presets merge per-field; overrides union.
- `[[transcript.rule]]` replaces-if-present (story-line coloring is game-specific
  — e.g. paint "grue" red in Zork).
- `[statusbar]` replaces-if-present — a per-game status/score bar (segments,
  border) for that adventure; absent → the global bar.

This is plain `merge(global, per_game)`; no per-game-specific merge variant is
needed.

### 3. Scaffold command — `/game-style`

- New `Command::GameStyle`, exposed as `/game-style` (bindable via the keymap).
- If `styles/<ifid>.toml` does **not** exist, create it (creating `styles/` as
  needed), seeded with a header:

  ```toml
  # Per-game style override for: <title>
  # IFID: <ifid>
  # Layers on the global style.toml. See style.example.toml for the full schema.
  # Anything style.toml supports works here (colors, symbols, transcript rules,
  # statusbar) and overrides the global value for this game only.

  [colors]
  # "room:current" = { fg = "yellow" }
  ```

  Report `created styles/<ifid>.toml` on the status line.
- If the file already exists, do **not** overwrite — report `per-game style:
  styles/<ifid>.toml` (so the user can open it). Either way, the styling is not
  re-applied by this command alone; the user edits the file and `/reload` (or the
  watcher) applies it.

### 4. Resolution through `reload_style`

`reload_style` gains the current IFID (passed from the run loop) and the merge:

```
reload_style(state, user_dir, ifid):
  global_doc = parse global style.toml (pointer in config.style)   # Failed → keep current
  per_game   = parse user_dir/styles/<ifid>.toml if it exists      # Failed → keep current
  merged     = merge(global_doc, per_game)                         # empty per_game → global only
  (cs, set, warnings) = resolve(merged, user_dir)
  state.colors = cs; state.symbols = set
  → Reloaded { warnings }
```

A parse error in **either** file keeps the current look and surfaces a Warning
line (consistent with the live-reload error model). When no per-game file exists,
the result equals today's global-only resolution.

### 5. File-watch interaction

When `watch_style` is on, the watcher watches **both** the global `style.toml`
and `user_dir/styles/<ifid>.toml`. Watching the `styles/` directory (not just the
file) covers the per-game file being created mid-session (e.g. by `/game-style`).
A change to either triggers the merged `reload_style`.

## Architecture / components

- `crates/app/src/reload.rs` (from the live-reload feature): `reload_style` gains an
  `ifid: &str` parameter; parse the per-game path `user_dir/styles/<ifid>.toml` (if
  present) and `merge(global, per_game)` before `resolve`. No per-game merge variant
  is needed — plain `merge` already does the right thing.
- `crates/app/src/persist_files.rs` (or a small `styles.rs`): `pub fn
  per_game_style_path(user_dir, ifid) -> PathBuf` = `user_dir/styles/<ifid>.toml`;
  `pub fn scaffold_per_game_style(user_dir, ifid, title) -> io::Result<(PathBuf,
  bool)>` (returns the path and whether it was newly created; never overwrites).
- `crates/app/src/keymap.rs` / command enum: add `GameStyle` (kebab `game-style`).
- `crates/app/src/slash.rs`: `/game-style` → `Command::GameStyle`.
- `crates/app/src/apply_action.rs` / `main.rs`: handle `GameStyle` (scaffold +
  status); startup + `/reload` + watch call `reload_style(.., ifid)`.
- `crates/app/src/main.rs`: the file-watcher (from live-reload) also watches the
  `styles/` dir; pass `ifid` into every `reload_style` call.

## Error handling

- Per-game file parse error → keep current look, one Warning line (same as global).
- `styles/` directory missing → created on scaffold; absent per-game file → global
  only (no error).
- `/game-style` when the file exists → reports the path, no overwrite.

## Testing

- `per_game_style_path` = `user_dir/styles/<ifid>.toml`.
- `scaffold_per_game_style`: creates the file + `styles/` dir with the title/IFID
  header when absent (returns `created = true`); returns `created = false` and
  leaves contents intact when it already exists.
- `merge`: a per-game `[colors]` selector overrides global per-key; a per-game
  `[symbols]` override merges; a per-game `[[transcript.rule]]` replaces; a per-game
  `[statusbar]` replaces the global bar (segments + border) for that game.
- `reload_style` with IFID: with a per-game file present, the resolved
  `ColorScheme` reflects global + per-game (e.g. global `transcript` fg overridden
  by the per-game file); with no per-game file, equals global-only resolution;
  per-game parse error → current look kept.
- Slash: `/game-style` → `Command::GameStyle`.
- Watch: enabling watch registers both the global file and the `styles/` dir
  (assert the watched paths include both).

## Out of scope (deferred)

- Capturing the current look into a per-game file by diff or snapshot (the command
  only scaffolds; overrides are hand-authored).
- Per-game keymap / non-style config.
- Bundling the per-game style inside the `.babelmap` archive (separate file only).
