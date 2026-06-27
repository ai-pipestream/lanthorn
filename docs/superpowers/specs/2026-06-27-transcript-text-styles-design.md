# Transcript honors Z-machine `set_text_style` (per-span) — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/app`

## Goal

Render the game's `set_text_style` emphasis (bold / italic / reverse-video) in
the main scrolling transcript, exactly — a bold word inside a normal sentence
shows only that word bold. Today the transcript is styled once per logical line
(by category + rules), so Z-machine in-text emphasis is dropped. The engine
already carries the style to the sink via the `Output::print_styled` seam
(shipped with the zvm-cli parity work); the app's `CaptureSink` just ignores it.

Exact per-span fidelity, persisted across save/reload. The style runs are stored
**alongside** the existing `Vec<String>` transcript (a parallel per-line run
list) rather than restructuring transcript storage — so search and the rest of
the pipeline keep operating on plain strings.

## Background (current pipeline)

- `CaptureSink { text: String }` (`session.rs:44`) only implements `print`
  (appends); it inherits the default `print_styled` (ignores style).
  `take_transcript()` (`session.rs:131`) drains it through `strip_read_prompt`.
  `submit`/`submit_char` → `finish_turn` → `TurnResult { transcript: String, … }`.
- `state.transcript: Vec<String>` + parallel `transcript_kinds: Vec<TranscriptKind>`
  (`Story|Input|Meta|Warning`) + `transcript_styles: Vec<Option<Style>>`
  (a per-line override; only used by tests in production). `push_transcript_kind`
  splits on `'\n'`, pushing one entry per line.
- Render: `wrap_lines_kinded(transcript, kinds, styles, width) -> Vec<(String,
  TranscriptKind, Style)>` resolves ONE `Style` per logical line; the draw loop
  (`render/transcript.rs:909`) draws each wrapped row with `draw_str_clipped`
  (or `draw_str_highlighted` when searching) — one style for the whole row.
- Persistence: `TranscriptData { lines, kinds }` (`archive.rs:60`) saved to
  `transcript.json` filtered to `Story|Input`; loaded back and assigned directly
  to `state.transcript`/`transcript_kinds`.
- `apply_text_style(base, bits) -> Style` (`render/upper_window.rs:21`) maps bits
  → modifiers (bold `0x02`, reverse `0x01`; italic `0x04` and fixed `0x08`
  currently unmapped). This is the upper window's per-cell styler.

## Design

### 1. Style-run type and storage

```rust
// state.rs
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StyleRun {
    pub start: usize, // char offset within the line (inclusive)
    pub end: usize,   // char offset (exclusive)
    pub bits: u8,     // Z-machine text_style bits (1=reverse,2=bold,4=italic,8=fixed)
}
```

Add `pub transcript_runs: Vec<Vec<StyleRun>>` parallel to `transcript` (one
`Vec<StyleRun>` per line; **empty** for the overwhelmingly common unstyled line).
We persist the **bits**, not a resolved `Style` (bits are stable and
serde-friendly; the resolved color comes from the base style at render time).
`transcript`, `transcript_kinds`, `transcript_styles` are unchanged.

### 2. Capture — `CaptureSink` records runs

```rust
pub struct CaptureSink { pub text: String, pub runs: Vec<(usize, u8)> } // (char_count, bits) per chunk
```

- `print(s)` → append `s`; push `(s.chars().count(), 0)`.
- `print_styled(s, style)` → append `s`; push `(s.chars().count(), style)`.
- `take_styled() -> (String, Vec<(usize, u8)>)` drains both.

`take_transcript` keeps stripping the read prompt; because `strip_read_prompt`
removes a **trailing** prompt, the run list is clamped to the final char length
(drop/truncate runs past the end). `TurnResult` gains
`transcript_runs: Vec<(usize, u8)>` (chunk list for the turn's text), produced in
`finish_turn` from `take_styled`. Turns with no styling yield all-zero chunks.

### 3. Push path — split chunk runs into per-line `StyleRun`s

A new `push_transcript_runs(&mut self, text: &str, kind: TranscriptKind, runs:
&[(usize, u8)])`:
- Walks `text` splitting on `'\n'` (as today), and in lockstep walks the chunk
  list, emitting per-line `Vec<StyleRun>` with char offsets relative to each
  line (newlines consume a chunk char but produce no run). Adjacent equal-bit
  spans are merged; zero-bit spans are omitted (empty vec when a line has no
  emphasis).
- Pushes line + kind + `None` style + the line's runs, keeping all four vectors
  length-synced. `push_transcript_kind`/`_styled` push an empty runs vec (and
  `transcript_runs` self-heals to `transcript.len()` like `transcript_styles`).

Only the **game-turn** push sites use `push_transcript_runs` (main.rs:1917/2230
and the restart-banner turn output in input.rs); every other site (banners,
echoes, status/save/restore messages, meta/warning) keeps its current call and
contributes empty runs.

### 4. Style mapping (shared)

Promote the bit→modifier mapping to a shared `pub(crate) fn apply_text_style(base:
Style, bits: u8) -> Style` (move from `upper_window.rs` to `render/mod.rs`,
re-used by both the upper window and the transcript) and add italic:
`0x04 → Modifier::ITALIC`; `0x08` (fixed-pitch) ignored. Order: apply over the
base, so a span = base color + the game's modifiers.

### 5. Wrapping carries runs

`wrap_line`/`wrap_line_hanging` gain offset-aware variants (or a returned
per-row source char range) so each wrapped row knows its `[start,end)` char span
in the original line. `wrap_lines_kinded` becomes:

```rust
pub(crate) fn wrap_lines_kinded(transcript, kinds, styles, runs, width)
    -> Vec<(String, TranscriptKind, Style, Vec<StyleRun>)>
```

For each wrapped row, intersect the line's `StyleRun`s with the row's source
range and re-base their offsets to the row start (clamped to the row's char
length). Word-wrap dropping a break space is fine — the row covers `[start,end)`
and the trailing space is excluded.

### 6. Render — per-span draw

Replace the row draw with one that applies spans:
`draw_str_runs(buf, x, y, text, base_style, &runs, search?, clip)`:
- For each char in the row, the style is `apply_text_style(base_style, bits)`
  where `bits` is the covering run's bits (0 if none) — drawn per contiguous span
  (same segmenting `draw_str_highlighted` already does).
- When search is active, a query match overrides with `search_highlight_style`
  for the matched chars (search affordance wins); spans apply to the rest. The
  empty-runs case is identical to today's `draw_str_clipped` (single style), so
  unstyled lines render byte-for-byte as now.

### 7. Persistence

`TranscriptData` gains `#[serde(default)] runs: Vec<Vec<StyleRun>>`. On save,
the runs are filtered in lockstep with the `Story|Input` lines (same filter as
lines/kinds). On load, older archives (no `runs`) default to empty per line;
`load_archive`/`ArchiveContents` carry `transcript_runs`, and the restore site
(main.rs:1122) assigns `state.transcript_runs` alongside `transcript`/`kinds`
(resized to match when absent).

## Testing

- `apply_text_style`: bits → modifiers incl. italic; fixed ignored; composes
  over a base color.
- `push_transcript_runs`: chunk list `[(2,2),(3,0)]` over "ab cde" → line runs
  `[{0,2,bold}]`; multi-line text splits runs per line with correct per-line
  offsets; all-zero chunks → empty runs; runs stay length-synced.
- Capture: `CaptureSink::print_styled` records `(len,bits)`; `take_styled`
  drains; prompt-strip clamps trailing runs.
- Wrap: a line `"AAAAA BBBBB"` with a bold run on `BBBBB`, wrapped narrow, yields
  rows whose re-based runs cover the right chars on each row.
- Render: a row with a mid-line bold run draws that span bold and the rest in the
  base style (assert the buffer cells' modifiers); empty runs == today's output;
  search highlight still wins on matches.
- Persistence: `TranscriptData` round-trips runs; a runs-less JSON loads as empty
  (back-compat); save filters runs in lockstep with `Story|Input`.

## Out of scope

- Fixed-pitch (`0x08`) rendering (the TUI is already monospaced) — ignored.
- Mapping italic to an alternative (underline/color) for terminals without
  italic — render `Modifier::ITALIC`; terminal support is the terminal's affair.
- A config toggle to disable honoring game styles — can be added later; default
  is to honor them.
- zvm-cli already styles its lower window via SGR (shipped); unaffected here.

## Global constraints

- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test -p app`
  green per task.
- Unstyled transcript lines (the common case) must render and persist exactly as
  today — `transcript_runs` empty, `draw_str_runs` collapses to the current
  single-style draw, `TranscriptData` round-trips identically when runs are empty.
- Old `.babelmap` archives (no `runs`) load unchanged (serde default empty).
- Commit-only on local `main`; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Surgical changes; do not edit `TODO.md` during the wave.
