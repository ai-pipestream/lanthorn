# Transcript Text Styles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the game's `set_text_style` (bold/italic/reverse) exactly in the app transcript — mid-line emphasis on just the styled span — and persist it across save/reload.

**Architecture:** A per-line `Vec<StyleRun>` (char ranges + style bits) stored parallel to the existing `Vec<String>` transcript, fed by the `Output::print_styled` seam through the capture path, re-based across word-wrap, drawn per-span, and serialized into `transcript.json`. Text storage stays `Vec<String>` so search/persistence keep working on strings. Unstyled lines (the common case) behave byte-for-byte as today.

**Tech Stack:** Rust, ratatui, serde. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-27-transcript-text-styles-design.md`

## Global Constraints

- 0 warnings (`cargo build`, `cargo doc -p app --no-deps`) + full `cargo test -p app` green after every task.
- Unstyled lines must render and persist EXACTLY as today: empty `transcript_runs`, `draw_str_runs` collapses to the current single-style draw (matching `draw_str_clipped`/`draw_str_highlighted`), and `TranscriptData` round-trips identically when runs are empty. Old `.lanthorn` archives (no `runs`) load unchanged.
- Commit-only on local `main`; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Do not edit `TODO.md`.

## Reference: exact current code (from a fresh read — verify before editing)

- `session.rs`: `CaptureSink { pub text: String }` (~:44), `print` impl (~:59), `take_text` (~:54), `take_transcript` via `strip_read_prompt` (~:131), `TurnResult { transcript: String, … }` (~:74), `submit`/`submit_char` → `finish_turn` (~:181).
- `state.rs`: `transcript: Vec<String>`, `transcript_kinds: Vec<TranscriptKind>`, `transcript_styles: Vec<Option<Style>>` (~:705); `push_transcript` (~:1219), `push_transcript_kind` (~:1224), `push_transcript_styled` (~:1234); `TranscriptKind {Story,Input,Meta,Warning}` (~:165).
- `render/transcript.rs`: `wrap_lines_kinded(transcript,kinds,styles,width) -> Vec<(String,TranscriptKind,Style)>` (~:305); render draw loop (~:909) calling `draw_str_highlighted`(search)/`draw_str_clipped`; `wrap_line` (~:206), `wrap_line_hanging` (~:265), `draw_str_highlighted` (~:369).
- `render/mod.rs`: `draw_str_clipped(buf,x,y,s,style,area)` (~:49), `draw_char_clipped`.
- `render/upper_window.rs`: `apply_text_style(base,bits)->Style` (~:21), used (~:144).
- `archive.rs`: `TranscriptData { lines, kinds }` (~:60), save filter `Story|Input` (~:207), load (~:324), `ArchiveContents { transcript, transcript_kinds, … }` (~:128).
- `main.rs`: game-turn pushes (~:1917 first-turn output, ~:2230 after-command output); restore assign `state.transcript = lines; state.transcript_kinds = kinds;` (~:1122). `input.rs`: restart-banner turn output push (~:5667).

---

## Task 1: `StyleRun` type + shared `apply_text_style` (add italic)

**Files:** Modify `crates/app/src/state.rs`, `crates/app/src/render/mod.rs`, `crates/app/src/render/upper_window.rs`.

**Interfaces:**
- Produces: `state::StyleRun { start: usize, end: usize, bits: u8 }` (serde); `render::apply_text_style(base: Style, bits: u8) -> Style` (`pub(crate)`, with italic).

- [ ] **Step 1: Failing test** in `render/mod.rs`

```rust
#[cfg(test)]
mod text_style_tests {
    use super::*;
    use ratatui::style::{Modifier, Style};

    #[test]
    fn apply_text_style_maps_all_bits() {
        let b = Style::default();
        assert!(apply_text_style(b, 0x02).add_modifier == Modifier::BOLD || apply_text_style(b, 0x02).add_modifier.contains(Modifier::BOLD));
        assert!(apply_text_style(b, 0x01).add_modifier.contains(Modifier::REVERSED));
        assert!(apply_text_style(b, 0x04).add_modifier.contains(Modifier::ITALIC));
        // fixed-pitch (0x08) adds nothing; 0 is a no-op
        assert_eq!(apply_text_style(b, 0x08), b);
        assert_eq!(apply_text_style(b, 0x00), b);
        // composes: bold+italic
        let bi = apply_text_style(b, 0x06).add_modifier;
        assert!(bi.contains(Modifier::BOLD) && bi.contains(Modifier::ITALIC));
    }
}
```

- [ ] **Step 2: Run → fail** (`no function apply_text_style`).

- [ ] **Step 3: Add the shared mapper** in `render/mod.rs`

```rust
use ratatui::style::{Modifier, Style};

/// Layer Z-machine text-style bits (ZMSD §8.7.1: 1=reverse, 2=bold, 4=italic,
/// 8=fixed-pitch) over a base style. Fixed-pitch is ignored (already monospaced).
pub(crate) fn apply_text_style(base: Style, bits: u8) -> Style {
    let mut s = base;
    if bits & 0x02 != 0 { s = s.add_modifier(Modifier::BOLD); }
    if bits & 0x01 != 0 { s = s.add_modifier(Modifier::REVERSED); }
    if bits & 0x04 != 0 { s = s.add_modifier(Modifier::ITALIC); }
    s
}
```

- [ ] **Step 4: Use it from the upper window** — delete the private `apply_text_style` in `render/upper_window.rs` and call `crate::render::apply_text_style(content_style, cell.style)` at its use site. (The upper-window suite must stay green — italic now also applies there, which is correct.)

- [ ] **Step 5: Add `StyleRun`** in `state.rs`

```rust
/// A run of characters in a transcript line carrying Z-machine text-style bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StyleRun {
    pub start: usize, // char offset within the line, inclusive
    pub end: usize,   // char offset, exclusive
    pub bits: u8,     // 1=reverse, 2=bold, 4=italic, 8=fixed
}
```

- [ ] **Step 6: Run + commit**

```bash
cargo test -p app   # green, 0 warnings
git add crates/app/src/state.rs crates/app/src/render/mod.rs crates/app/src/render/upper_window.rs
git commit  # feat(app): shared apply_text_style (with italic) + StyleRun type
```

---

## Task 2: `transcript_runs` field + `push_transcript_runs`

**Files:** Modify `crates/app/src/state.rs`.

**Interfaces:**
- Produces: `state.transcript_runs: Vec<Vec<StyleRun>>`; `push_transcript_runs(&mut self, text: &str, kind: TranscriptKind, chunks: &[(usize, u8)])`.

- [ ] **Step 1: Failing tests** in `state.rs` tests

```rust
    #[test]
    fn push_runs_extracts_per_line_spans() {
        let mut s = AppState::default();
        // "ab cde": 2 bold chars, then 4 plain ("ab"=bold? chunks below)
        // chunks: ("ab",bold) ("c",0) ... use a clear case:
        s.push_transcript_runs("ab cd", TranscriptKind::Story, &[(2, 0x02), (3, 0)]);
        assert_eq!(s.transcript.last().unwrap(), "ab cd");
        assert_eq!(s.transcript_runs.last().unwrap(), &vec![StyleRun { start: 0, end: 2, bits: 0x02 }]);
        // lengths stay synced
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert_eq!(s.transcript.len(), s.transcript_kinds.len());
    }

    #[test]
    fn push_runs_splits_across_newlines() {
        let mut s = AppState::default();
        // "A\nB" with bold on 'A' and 'B'; newline chunk char between
        s.push_transcript_runs("A\nB", TranscriptKind::Story, &[(1, 0x02), (1, 0), (1, 0x02)]);
        let n = s.transcript.len();
        assert_eq!(s.transcript[n - 2], "A");
        assert_eq!(s.transcript[n - 1], "B");
        assert_eq!(s.transcript_runs[n - 2], vec![StyleRun { start: 0, end: 1, bits: 0x02 }]);
        assert_eq!(s.transcript_runs[n - 1], vec![StyleRun { start: 0, end: 1, bits: 0x02 }]);
    }

    #[test]
    fn push_runs_all_plain_is_empty() {
        let mut s = AppState::default();
        s.push_transcript_runs("hello", TranscriptKind::Story, &[(5, 0)]);
        assert!(s.transcript_runs.last().unwrap().is_empty());
    }

    #[test]
    fn push_kind_keeps_runs_synced_empty() {
        let mut s = AppState::default();
        s.push_transcript_kind("x", TranscriptKind::Meta);
        assert_eq!(s.transcript.len(), s.transcript_runs.len());
        assert!(s.transcript_runs.last().unwrap().is_empty());
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Add the field** to `AppState` (declaration near `transcript_styles`) and `Default` (`transcript_runs: Vec::new()`).

- [ ] **Step 4: Self-heal in the existing push methods** — in `push_transcript_kind` and `push_transcript_styled`, before the loop add `self.transcript_runs.resize(self.transcript.len(), Vec::new());` and in the loop `self.transcript_runs.push(Vec::new());` (alongside the existing pushes).

- [ ] **Step 5: Implement `push_transcript_runs`**

```rust
pub fn push_transcript_runs(&mut self, text: &str, kind: TranscriptKind, chunks: &[(usize, u8)]) {
    self.transcript_styles.resize(self.transcript.len(), None);
    self.transcript_runs.resize(self.transcript.len(), Vec::new());

    // Walk text by char while consuming the (char_count, bits) chunk list in
    // lockstep, building per-line StyleRuns (offsets relative to each line).
    let mut chunk_iter = chunks.iter().copied();
    let (mut rem, mut bits) = chunk_iter.next().unwrap_or((usize::MAX, 0));
    let mut next_bits = |rem: &mut usize, bits: &mut u8| {
        while *rem == 0 {
            match chunk_iter.next() {
                Some((c, b)) => { *rem = c; *bits = b; }
                None => { *rem = usize::MAX; *bits = 0; break; }
            }
        }
    };
    next_bits(&mut rem, &mut bits);

    for line in text.split('\n') {
        let mut runs: Vec<StyleRun> = Vec::new();
        let mut col = 0usize;
        for _ch in line.chars() {
            next_bits(&mut rem, &mut bits);
            if bits != 0 {
                match runs.last_mut() {
                    Some(r) if r.end == col && r.bits == bits => r.end = col + 1,
                    _ => runs.push(StyleRun { start: col, end: col + 1, bits }),
                }
            }
            col += 1;
            rem = rem.saturating_sub(1);
        }
        // Consume the newline character's chunk position (if any).
        next_bits(&mut rem, &mut bits);
        rem = rem.saturating_sub(1);

        self.transcript.push(line.to_owned());
        self.transcript_kinds.push(kind);
        self.transcript_styles.push(None);
        self.transcript_runs.push(runs);
    }
}
```

(Implementer: verify the chunk/newline bookkeeping against the tests; the
invariant is that the chunk char-count covers every char of `text` INCLUDING the
`\n` separators, matching how `CaptureSink` records `(s.chars().count(), bits)`.)

- [ ] **Step 6: Run + commit**

```bash
git add crates/app/src/state.rs
git commit  # feat(app): transcript_runs + push_transcript_runs (per-line style spans)
```

---

## Task 3: Capture runs through `CaptureSink` → `TurnResult` → push sites

**Files:** Modify `crates/app/src/session.rs`, `crates/app/src/main.rs`, `crates/app/src/input.rs`.

**Interfaces:**
- Consumes: `push_transcript_runs`.
- Produces: `CaptureSink { text, runs: Vec<(usize, u8)> }`, `take_styled`; `TurnResult.transcript_runs: Vec<(usize, u8)>`.

- [ ] **Step 1: Failing test** in `session.rs` tests

```rust
    #[test]
    fn capture_sink_records_style_runs() {
        use zvm::io::Output;
        let mut s = CaptureSink::new();
        s.print("ab");
        s.print_styled("CD", 0x02);
        let (text, runs) = s.take_styled();
        assert_eq!(text, "abCD");
        assert_eq!(runs, vec![(2, 0), (2, 0x02)]);
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Extend `CaptureSink`**

```rust
pub struct CaptureSink {
    pub text: String,
    pub runs: Vec<(usize, u8)>, // (char_count, text_style bits) per print chunk
}
impl CaptureSink {
    fn new() -> Self { CaptureSink { text: String::new(), runs: Vec::new() } }
    pub fn take_styled(&mut self) -> (String, Vec<(usize, u8)>) {
        (std::mem::take(&mut self.text), std::mem::take(&mut self.runs))
    }
    pub fn take_text(&mut self) -> String { self.take_styled().0 }
}
impl Output for CaptureSink {
    fn print(&mut self, s: &str) {
        self.runs.push((s.chars().count(), 0));
        self.text.push_str(s);
    }
    fn print_styled(&mut self, s: &str, style: u8) {
        self.runs.push((s.chars().count(), style));
        self.text.push_str(s);
    }
    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
```

- [ ] **Step 4: Carry runs on the turn** — add `pub transcript_runs: Vec<(usize, u8)>` to `TurnResult`. In `finish_turn` (and the `take_transcript` path), drain via `take_styled`. After `strip_read_prompt` shortens the text by `k` trailing chars, clamp the chunk list so its total char-count equals the final text length (trim the last chunk(s)). Add a helper `clamp_runs(runs, char_len) -> Vec<(usize,u8)>`. Populate `TurnResult.transcript_runs` with the clamped chunks; default to all-plain when nothing styled.

- [ ] **Step 5: Wire the game-turn push sites** — where the code currently does `state.push_transcript(&result.transcript)` for GAME output (main.rs ~:1917 and ~:2230; input.rs ~:5667 restart turn output), switch to `state.push_transcript_runs(&result.transcript, TranscriptKind::Story, &result.transcript_runs)`. Leave every OTHER push (banners, `>` echo, info notes, save/restore/status messages, meta/warnings) unchanged — they contribute empty runs.

- [ ] **Step 6: Run + manual check** — `cargo test -p app` green, 0 warnings. (Styled game output now records runs; unstyled output unchanged.)

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/main.rs crates/app/src/input.rs
git commit  # feat(app): capture set_text_style runs through CaptureSink and TurnResult
```

---

## Task 4: Word-wrap carries per-row source ranges; `wrap_lines_kinded` re-bases runs

**Files:** Modify `crates/app/src/render/transcript.rs`.

**Interfaces:**
- Produces: `wrap_line_ranges(line, width) -> Vec<(String, usize, usize)>` (row text + `[start,end)` char offsets in the original); `wrap_lines_kinded(transcript, kinds, styles, runs, width) -> Vec<(String, TranscriptKind, Style, Vec<StyleRun>)>`.

- [ ] **Step 1: Failing tests**

```rust
    #[test]
    fn wrap_line_ranges_round_trips_word_wrap() {
        // Same row strings as wrap_line, plus correct source char ranges.
        let rows = wrap_line_ranges("AAAAA BBBBB", 5);
        assert_eq!(rows.iter().map(|(s,_,_)| s.clone()).collect::<Vec<_>>(),
                   wrap_line("AAAAA BBBBB", 5));
        // first row covers chars 0..5, second covers 6..11 (break space dropped)
        assert_eq!((rows[0].1, rows[0].2), (0, 5));
        assert_eq!((rows[1].1, rows[1].2), (6, 11));
    }

    #[test]
    fn wrap_lines_kinded_rebases_runs_per_row() {
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 6, end: 11, bits: 0x02 }]]; // bold "BBBBB"
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &runs, 5);
        // row 0 ("AAAAA", 0..5) → no runs; row 1 ("BBBBB", 6..11) → bold 0..5
        assert!(out[0].3.is_empty());
        assert_eq!(out[1].3, vec![StyleRun { start: 0, end: 5, bits: 0x02 }]);
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement `wrap_line_ranges`** — refactor `wrap_line` to track, for each emitted row, the char offset range in the source. The row's visible text equals the source chars `[start,end)` (word-wrap drops only the break space between rows; hard-broken long words split into contiguous ranges). Make `wrap_line` delegate: `wrap_line(l, w) = wrap_line_ranges(l, w).into_iter().map(|(s,_,_)| s).collect()` so existing behavior/tests are preserved.

- [ ] **Step 4: Thread runs through `wrap_lines_kinded`** — add the `runs: &[Vec<StyleRun>]` parameter and the 4th tuple element. For `Story|Input` rows (word-wrapped), intersect the line's `StyleRun`s with each row's `[start,end)` and re-base offsets to the row start (clamp to row length); drop empty intersections. For `Meta|Warning` (hanging-wrapped, always app-generated → empty runs), emit empty run vecs. Update the call site that builds the wrapped list to pass `&state.transcript_runs`.

- [ ] **Step 5: Run + commit** — `cargo test -p app` green, 0 warnings.

```bash
git add crates/app/src/render/transcript.rs
git commit  # feat(app): wrap re-bases style runs onto wrapped rows
```

---

## Task 5: Render per-span (`draw_str_runs`)

**Files:** Modify `crates/app/src/render/transcript.rs`.

**Interfaces:**
- Consumes: `render::apply_text_style`, `StyleRun`.
- Produces: `draw_str_runs(buf, x, y, text, base_style, runs, search, area)` used by the transcript draw loop in place of `draw_str_clipped`/`draw_str_highlighted`.

- [ ] **Step 1: Failing tests** (render into a `Buffer`, assert cell modifiers)

```rust
    #[test]
    fn draw_str_runs_applies_span_modifier() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Modifier, Style}};
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let runs = vec![StyleRun { start: 2, end: 4, bits: 0x02 }]; // bold chars 2..4
        draw_str_runs(&mut buf, 0, 0, "abcdef", Style::default(), &runs, None, area);
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(2, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buf[(4, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn draw_str_runs_empty_matches_clipped() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 10, 1);
        let mut a = Buffer::empty(area);
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut a, 0, 0, "hello", Style::default(), &[], None, area);
        crate::render::draw_str_clipped(&mut b, 0, 0, "hello", Style::default(), area);
        assert_eq!(a, b, "empty runs render identically to draw_str_clipped");
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Implement `draw_str_runs`**

```rust
use crate::render::apply_text_style;

/// Draw `text` applying per-char style: base + the covering run's bits. When
/// `search` is Some((query_lower, highlight_style)), characters inside a query
/// match use `highlight_style` instead (search affordance wins). Empty runs +
/// no search == draw_str_clipped.
pub(crate) fn draw_str_runs(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    base_style: ratatui::style::Style,
    runs: &[StyleRun],
    search: Option<(&str, ratatui::style::Style)>,
    area: ratatui::layout::Rect,
) {
    if y < area.y || y >= area.bottom() { return; }
    // Precompute which char indices fall inside a search match.
    let hi: Vec<bool> = match search {
        Some((q, _)) if !q.is_empty() => highlight_mask(text, q),
        _ => Vec::new(),
    };
    let mut col = x;
    for (i, ch) in text.chars().enumerate() {
        if col >= area.right() { break; }
        let style = if hi.get(i).copied().unwrap_or(false) {
            search.unwrap().1
        } else {
            let bits = runs.iter().find(|r| i >= r.start && i < r.end).map(|r| r.bits).unwrap_or(0);
            apply_text_style(base_style, bits)
        };
        crate::render::draw_char_clipped(buf, col, y, ch, style, area);
        col += 1;
    }
}
```

`highlight_mask(text, query_lower) -> Vec<bool>` reuses the lowercasing/substring
logic from `draw_str_highlighted` (factor it out, or inline) to mark matched char
positions. Ensure `draw_str_runs(.., &[], None, ..)` is byte-identical to
`draw_str_clipped` and `draw_str_runs(.., &[], Some(..), ..)` matches
`draw_str_highlighted` (a test for each).

- [ ] **Step 4: Use it in the draw loop** — in `render_middle` (~:909), replace the `if has_search { draw_str_highlighted(...) } else { draw_str_clipped(...) }` with a single `draw_str_runs(buf, text_x, row_y, line, *style, runs, has_search.then(|| (&query_lower[..], search_highlight_style)), body_area)`, where `runs` is the row's `Vec<StyleRun>` (4th tuple element from `wrap_lines_kinded`). Gutter glyphs are unchanged.

- [ ] **Step 5: Run + commit** — `cargo test -p app` green, 0 warnings.

```bash
git add crates/app/src/render/transcript.rs
git commit  # feat(app): render transcript per-span (game text styles), search-aware
```

---

## Task 6: Persist runs in `transcript.json`

**Files:** Modify `crates/app/src/archive.rs`, `crates/app/src/main.rs`.

**Interfaces:**
- Produces: `TranscriptData { lines, kinds, runs }`; `ArchiveContents.transcript_runs`.

- [ ] **Step 1: Failing tests** in `archive.rs` tests

```rust
    #[test]
    fn transcript_data_round_trips_runs() {
        let td = TranscriptData {
            lines: vec!["a".into(), "b".into()],
            kinds: vec![TranscriptKind::Story, TranscriptKind::Input],
            runs: vec![vec![StyleRun { start: 0, end: 1, bits: 0x02 }], vec![]],
        };
        let json = serde_json::to_string(&td).unwrap();
        let back: TranscriptData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.runs, td.runs);
    }

    #[test]
    fn old_transcript_json_loads_with_empty_runs() {
        // JSON without a "runs" field (older archive)
        let json = r#"{"lines":["x"],"kinds":["Story"]}"#;
        let td: TranscriptData = serde_json::from_str(json).unwrap();
        assert!(td.runs.is_empty());
    }
```

- [ ] **Step 2: Run → fail.**

- [ ] **Step 3: Extend `TranscriptData`**

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TranscriptData {
    lines: Vec<String>,
    kinds: Vec<crate::state::TranscriptKind>,
    #[serde(default)]
    runs: Vec<Vec<crate::state::StyleRun>>,
}
```

- [ ] **Step 4: Save in lockstep** — in the save filter (`Story|Input`), zip `transcript_runs` alongside `transcript`/`transcript_kinds` and collect the filtered `runs` into `TranscriptData`. (Signature of the save fn gains `transcript_runs: &[Vec<StyleRun>]`; pass `&state.transcript_runs` at the call site.)

- [ ] **Step 5: Load + restore** — `load_archive` reads `td.runs` (empty when absent); `ArchiveContents` gains `pub transcript_runs: Vec<Vec<StyleRun>>`. At the restore site (main.rs ~:1122) assign `state.transcript_runs = runs;` next to `transcript`/`transcript_kinds` (when the archive predates runs, build an empty `Vec::new()` per line so lengths stay synced — `vec![Vec::new(); lines.len()]`).

- [ ] **Step 6: Run + commit** — `cargo test -p app` green, 0 warnings.

```bash
git add crates/app/src/archive.rs crates/app/src/main.rs
git commit  # feat(app): persist transcript style runs in transcript.json (back-compatible)
```

---

## Self-review checklist (run before final review)

- Unstyled path unchanged: `draw_str_runs(&[], None)` == `draw_str_clipped`; `draw_str_runs(&[], Some)` == `draw_str_highlighted`; empty-runs `TranscriptData` JSON identical to before (aside from an empty `runs` array).
- Only game-turn output pushes runs; banners/echoes/status/meta/warning push empty.
- `transcript_runs` stays length-synced with `transcript` through every push and on restore.
- Runs re-base correctly across word-wrap; Meta/Warning (hanging) always empty.
- italic now also renders in the upper window (shared mapper) — upper-window suite green.
- Old archives (no `runs`) load; new archives round-trip runs.
- 0 warnings; `cargo test -p app` green.
