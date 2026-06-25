# `/info` Diagnostic Command — Design

**Date:** 2026-06-25
**Status:** DEFERRED (shelved — design captured for future implementation; not scheduled)
**Living spec:** the Content section below is a growing list — whenever an
interesting/diagnostic piece of state turns up while working on babelmap, add it
to the relevant section here so the eventual `/info` surfaces it.

## Goal

A diagnostic command that dumps a useful, sectioned snapshot of babelmap's state
into the transcript — for users (and us) to see at a glance which style files are
active, what background tasks are doing, and the story/VM facts.

## Design

### Command

- **`/info`** → dump **both** sections · **`/info vm`** → VM/story only ·
  **`/info app`** → babelmap only · unknown argument → a short usage error.
- Output is a **snapshot** at invocation time, pushed into the scrolling transcript
  as **Meta** lines, sectioned with headers (mirrors `/help`'s slash→Meta-lines
  pattern). Searchable with `/search`, exportable with `/export`. Re-run to refresh.
- Not live-updating in v1. A live modal panel (`/info`-as-overlay that updates
  background-task status in real time) is a clean follow-on if the snapshot proves
  insufficient.

### Content (living list)

**`vm` — the Z-machine / story:**
- Story file path · resolved title · IFID
- Z-machine version · release number · serial code · story file size
- Current location (room name) · detection method (`loc_method`) · player-object
  lock state
- Game move count (from the status line)
- VM diagnostics: which unimplemented opcodes have been hit this session
  (`warned_var_opcodes` / `diagnostics`)

**`app` — babelmap:**
- **Styling:** resolved `style.toml` path (or built-in/`default`) · base scheme ·
  per-game `styles/<ifid>.toml` (present? active?)
- **Background tasks:** style watcher on/off + watched paths (style.toml +
  `styles/`) · whether a debounced reload is pending · background tidy job
  running/idle + graph generation · sound-pulse active
- **Map:** room count · layer count · current/viewed layer · map archive path
  (exists?) · graph generation
- **Persistence:** `user_dir` · `save_dir` · auto-save/auto-load state · default
  archive exists? · named-save count for this IFID
- **Config:** virtual screen size · command prefix · keymap source · `undo_levels`
- **Build:** babelmap version / build info

### Architecture (sketch)

- A new `info` module: a pure-ish `build_report(scope, machine, state, mapper,
  watch_on, …) -> Vec<String>` (testable against synthetic state). `scope ∈
  {Vm, App, Both}`.
- `slash::parse` maps `/info [vm|app]` → `SlashOutcome::Info(scope)`. The main loop
  (which holds `machine`/`mapper`/`state` plus the run-loop watcher local) calls
  `build_report` and pushes each line as `TranscriptKind::Meta` — same pattern as
  `/save`/`/export`.
- Release/serial come from direct header-byte reads (header 0x02; serial
  0x12–0x17). Room/layer counts from `graph.rooms().count()` / `graph.layers().len()`
  / `graph.current()`.

## Decisions settled

- Name: `/info` (not `/init`).
- One command with an optional `vm`/`app` argument (no separate `/vm-info`/`/app-info`).
- Snapshot into the transcript (not a live modal — deferred).
- v1 drops "last reload result/warnings" (not currently stored on state; easy to
  add later if a `last_reload` field is introduced).

## Out of scope (for v1)

- A live-updating modal panel.
- Persisting/exporting the report separately from `/export`.
- Per-line styling beyond the Meta category.
