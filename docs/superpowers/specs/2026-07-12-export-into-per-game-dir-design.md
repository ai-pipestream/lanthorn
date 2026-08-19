# SQ-0288 — `/export` commands land in the per-game dir — Design

**Status:** design (user co-designed all decisions)
**Date:** 2026-07-12
**Depends on:** SQ-0284 (per-game dir + `story_key`/`game_dir`; done)
**Branch:** stacked on `sq-0284-storage-layout`

## Problem

The `/export` slash commands don't follow the SQ-0284 per-game-dir convention:
- `export-svg` / `export-dot` / `export-dump` write **fixed** paths `~/.lanthorn/maps/<ifid>.{svg,dot,map.txt}` and take **no** path argument (`slash.rs:362-370` → `Action::Export{Svg,Dot,Dump}` → `main.rs:3656-3695`, paths built at `main.rs:1933-1936`).
- `export-transcript [file]` already takes an optional `/`-aware path but defaults into a **different** place, `~/.lanthorn/exports/transcript-<stamp>.txt` (`slash.rs:340-342` → `SlashOutcome::Export` → `main.rs:4395-4412`, resolver `export::export_transcript` at `export.rs:11-28`).

So exports scatter across `maps/` and `exports/`, keyed by IFID, ignoring the per-game dir where SQ-0284 put everything else.

## Goal

All four export commands write into the story's **per-game dir** `<base>/<story-key>/` (the same `game_dir` used for saves/sidecars), with fixed default filenames that overwrite, and an optional path argument resolved the SQ-0284 way.

## Design

### Default filenames (no argument) — fixed, overwrite

| Command | Default output |
|---|---|
| `export-svg` | `<game_dir>/map.svg` |
| `export-dot` | `<game_dir>/map.dot` |
| `export-dump` | `<game_dir>/map.txt` |
| `export-transcript` | `<game_dir>/transcript.txt` |

Re-exporting overwrites the same file (one "current" artifact per type per game — matches SQ-0284's `default.*` singleton spirit). `<base>` honors `--data-dir` (it's the same `game_dir` as saves), so exports sit beside `default.lanthorn`/`default.aux`/etc.

### Optional `[file]` argument — SQ-0284 resolution

Same rule as SQ-0284's interactive `@save` (`resolve_save_input`) and the existing `export_transcript` resolver:
- **bare name** (no separator, e.g. `beforetroll`) → `<game_dir>/beforetroll.<ext>` (the format's extension appended if absent).
- **path with a separator / absolute** (e.g. `/tmp/x.svg`, `../maps/x`) → honored **verbatim**.

All three map commands gain this optional arg; `export-transcript` keeps its arg but its default and bare-name base move from `exports/` to `game_dir`.

### Wiring

- `slash.rs`: change `export-svg|dot|dump` from `Action::Export*` (no arg) to carry `Option<String>` (mirror `export-transcript`'s `dispatch: |a| …(a.first().map(...))`), updating each `usage` to `"export-svg [file]"` etc. `COMMANDS.len()` stays 55 (no commands added/removed — the `slash.rs:657` assertion is unchanged).
- A shared resolver in `export.rs`: `resolve_export_path(dest: Option<&str>, game_dir: &Path, default_name: &str) -> PathBuf` implementing the rule above (bare → `game_dir/name` with the default's extension appended if the name has none; separator → verbatim; none → `game_dir/default_name`). `export_transcript` is refactored to use it (default `"transcript.txt"`, base `game_dir` instead of `exports_dir`/stamp).
- Handlers thread the already-computed `game_dir` (session setup, `main.rs:1844`) into the map-export Action handlers (`main.rs:3656-3695`, currently using the fixed `svg_path`/`dot_path`/`dump_path`) and the transcript `SlashOutcome::Export` handler (`main.rs:4395-4412`, currently using `exports_dir`/`stamp`). The pure renderers (`render_svg`/`render_dot`/`render_dump`, all `graph → String`) are unchanged. Each handler still pushes a notice with the resolved path (use `abbreviate_home`).
- The old fixed `svg_path`/`dot_path`/`dump_path` (`main.rs:1933-1936`) and the `maps/<ifid>` defaults are removed (the map dir may still hold other artifacts, but exports no longer target it). The `exports/` dir + `format_stamp` for transcript default are dropped from this path.

## Non-goals

- No new unified `/export <what>` command — the four per-format commands stay.
- No new artifact types (game state, room list, etc.).
- No change to the pure renderers or the map/transcript data.
- No file-browser "save-as" picker — the string argument + verbatim-path escape hatch is the destination-entry mechanism (a richer picker is out of scope).

## Testing

- `export::resolve_export_path`: `None` → `game_dir/<default_name>`; bare name → `game_dir/<name>.<ext>` (ext appended when missing, preserved when present); separator/absolute → verbatim.
- `export::export_transcript` (refactored): default lands at `game_dir/transcript.txt`; bare name → `game_dir/<name>`; path → verbatim.
- Handler-level (extend an existing export test if one exists, else a focused unit on the resolver): each map format's default resolves to `map.<ext>` in the game dir.
- Regression: the pure renderers and `combined_saves`/save paths are untouched.

## Rollout

No migration. Update `docs/persistence.md` / the export docs (and `README` if it lists `/export`) to say exports land in the per-game dir with fixed default names + the optional path arg. Note the default-location change (was `maps/`/`exports/`).
