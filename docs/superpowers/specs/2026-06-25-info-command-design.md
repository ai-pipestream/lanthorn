# `/info` Diagnostic Command — Design

**Date:** 2026-06-25
**Status:** DEFERRED (shelved — design captured for future implementation; not scheduled)
**Living spec:** the Content section below is a growing list — whenever an
interesting/diagnostic piece of state turns up while working on lanthorn, add it
to the relevant section here so the eventual `/info` surfaces it.

## Goal

A diagnostic command that dumps a useful, sectioned snapshot of lanthorn's state
into the transcript — for users (and us) to see at a glance which style files are
active, what background tasks are doing, and the story/VM facts.

## Design

### Command

- **`/info`** → dump **both** sections · **`/info vm`** → VM/story only ·
  **`/info app`** → lanthorn only · unknown argument → a short usage error.
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
- **Feature usage (engine-feature recorder)** — added 2026-06-30. The accurate
  way to know which engine features a story actually exercises: a lightweight,
  always-on **usage recorder** in each VM (a set/counter updated in the opcode +
  `@glk` dispatch, generalizing the existing once-per-opcode `diagnostics`
  pattern), accumulated across the session and surfaced here. Record the
  *interesting, operand-specific* events, not every opcode — e.g.:
  - Z-machine: `set_font 3` (Font 3 graphics) and Font 4; `buffer_mode` off;
    `set_text_style` styles used; `set_colour`; timed/interrupt input
    (`read`/`read_char` with time+routine); terminating-chars table; sound
    (`sound_effect` / Blorb); upper-window split / `set_window` usage; v3 status
    line vs v4+ host-managed; CP437 / IBM-PC graphics path taken; Unicode
    (`print_unicode` / translation table); save/restore/undo; `throw`/`catch`.
  - Glulx: which `@glk` selectors fired; which `@gestalt` / `glk_gestalt`
    capabilities the game queried; file/resource streams; acceleration; float ops;
    `@restart`; timer/mouse/hyperlink/graphics/sound events requested.
  Operand-specific because the recorder sees runtime values (font 3 vs 1) — which
  static analysis cannot. Caveat: a recorder only reflects the **paths played**;
  full coverage needs a walkthrough.
  - *Static complement (for the story-list view, not `/info`):* a cheap up-front
    header scan (version, Flags 1/2, Unicode table, terminating-chars table) is
    reliable for a catalog column; a deeper opcode-reference scan via the decoder
    (`zvm/cpu/decode.rs`) is only approximate (code/data intermixing, computed
    calls, no operand values) — "references `set_font`", not "uses font 3".
  - *Coverage-matrix follow-on (separate sub-project):* run stories under scripted
    walkthroughs (ties into the deferred command-replay / `input_stream` idea) with
    the recorder on → a *story × feature* matrix. Answers "which story files
    exercise which features" for test-fixture selection + engine validation
    (complements the 2026-06-30 feature-gap audits).

**`app` — lanthorn:**
- **Terminal default colours (OSC 10/11 probe)** — added 2026-07-28 (SQ-0510
  round 3). The parsed `state.term_default_colors.fg`/`.bg` RGBs plus the RAW
  probe reply bytes (escaped), and which layer the v6 raster ink/page pair
  actually resolved from (theme pair / OSC pair / hardcoded fallback,
  `v6_default_pair` in `render/screen.rs`). Needed because raster colour bugs
  on a real TTY are undebuggable headless — the probe answer differs per
  terminal and can't be reproduced in tests.
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
- **v6 magnification (`v6_pixel_lock`)** — added 2026-08-20 (SQ-0936). Whether the
  lock is on; the launch's per-axis `art_scale` (`state.v6_art_scale`) and the
  ladder step `1 / gcd` of it; the frame's letterbox scale
  (`state.v6_image_scale`) and which rung that is; and — the reason this is here
  at all — `state.v6_scale_lock_fallback`, set when the lock was asked for and the
  pane could not hold even the smallest rung, so the frame fell back to free
  scaling. That fallback is deliberately silent on the game screen (every
  too-small decision in lanthorn degrades rather than blocks), which leaves
  `/info` as the only place a player can find out why their artwork is soft
  despite the setting being on.
- **Build:** lanthorn version / build info

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
