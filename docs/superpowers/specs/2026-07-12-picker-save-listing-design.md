# SQ-0285 — Story-picker save listing (files + on-disk paths) — Design

**Status:** design (awaiting user review)
**Date:** 2026-07-12
**Depends on:** SQ-0284 (the per-game dir + `story_key`; done)
**Branch:** stacked on `sq-0284-storage-layout`

## Problem

The story picker's info panel already has a **Saves** section, but it only lists `.babelmap` Save States by name/turn/date (`main.rs:1591`, fed by `StoryAux.saves` = `persist_files::list_saves`). It does **not** show:
- **where** the saves live on disk (the per-game dir path) — the SQ-0284 layout made this worth surfacing,
- the story's **`.qzl` in-game saves**,
- the **sidecars** (`default.aux` / `default.glkvfs`).

The user wants the browser to show a selected story's save files **and their on-disk paths**.

## Goals

1. In the info panel's Saves section, show the story's **per-game directory path** as a header, so each listed file's full path is unambiguous (`<dir>/<filename>`).
2. List **both** user-facing save types: `.babelmap` Save States (as today) **and** `.qzl` in-game saves.
3. Add a **secondary line** listing the present sidecars (`default.aux`, `default.glkvfs`).
4. Everything is read-only display; no new actions. Keys off the same SQ-0284 per-game dir already resolved in `resolve_aux`.

## Non-goals

- No open/delete/rename actions from the panel (display only).
- No map-export or hint files (hints already have a badge; maps are out of scope).
- No change to storage/keying — reuses SQ-0284's `game_dir`/`story_key`.

## Design

### Rendering (info panel Saves section)

For a flat per-game dir, repeating the long absolute dir on every row is noise. Show it **once** as a header, then each file by filename (full path = header + filename):

```
Saves · ~/.babelmap/saves/Zork1.z5/
 (default)   turn 42 · 2026-07-12   default.babelmap
 quicksave   turn 40 · 2026-07-11   quicksave.babelmap
 quick       2026-07-11             quick.qzl
Sidecars: default.aux · default.glkvfs
```

- Header line: `Saves · <game_dir>` (abbreviate `$HOME` → `~`). Shown whenever the dir has any save OR sidecar.
- `.babelmap` rows: `<name>  turn <n> · <date>  <filename>` (name/turn/date as today; **add the filename**).
- `.qzl` rows: `<name>  <date>  <filename>` (bare Quetzal has no turn/Meta — use the file mtime for the date).
- Sidecars: one secondary line `Sidecars: <names present>` (`default.aux` and/or `default.glkvfs`), omitted if neither exists.
- Ordering: `.babelmap` first (default slot, then named newest-first, as `list_saves` already sorts), then `.qzl` (newest-first by mtime).
- Empty state: if no saves and no sidecars, show nothing (as today) — no empty header.

> _Open display choice for review:_ dir-as-header + filenames (recommended, above) vs. a full absolute path on every row. Recommending the header form — clearer and fits the narrow panel. Say the word if you'd rather see the full path per line.

### Data (`StoryAux` in `picker.rs`)

Extend `StoryAux` (resolved lazily per highlight in `resolve_aux`) with:
- `game_dir: PathBuf` — the resolved `<data_base>/<story_key>/` (so the panel can render the header without recomputing).
- `qzl_saves: Vec<QzlInfo>` where `QzlInfo { path: PathBuf, name: String, modified: String }` (name = filename stem; modified = mtime formatted `YYYY-MM-DD`, empty if unavailable). Add a `persist_files::list_qzl(game_dir) -> Vec<QzlInfo>` (move/generalize the existing `main.rs::list_qzl`, which already enumerates `*.qzl` with mtime, into `persist_files` so both the saves-manager and the picker use one implementation).
- `sidecars: Vec<&'static str>` (or `Vec<String>`) — which of `default.aux` / `default.glkvfs` exist in `game_dir`.

`SaveInfo` already carries `path`; the panel just starts rendering the filename from it.

`resolve_aux` gains the `game_dir` it already computes (line 168) into the returned struct, plus the `list_qzl` call and two `Path::exists` checks for the sidecars. Cheap; still lazy per-highlight.

## Testing

- `persist_files::list_qzl`: enumerates `<game_dir>/*.qzl` (name = stem, mtime date), skips `.babelmap`; empty dir → empty; sorted newest-first.
- `picker::resolve_aux`: for a temp game dir containing `default.babelmap` + `quick.qzl` + `default.aux`, the returned `StoryAux` has the right `game_dir`, one `.babelmap` save, one `qzl_save`, and `["default.aux"]` sidecars.
- `draw_info_panel`: extend the existing `info_panel_renders_metadata_features_and_resources` (main.rs:7199) — with an aux carrying a `.babelmap` save + a `.qzl` save + a sidecar, assert the rendered buffer contains the dir header, both filenames, and the `Sidecars:` line.

## Rollout

Pure additive UI + one function move (`list_qzl` → `persist_files`). No storage/format change, no migration. If `README`/`docs` describe the picker panel, add a line that it now shows saves + their location; otherwise no docs change needed (minor UI addition).
