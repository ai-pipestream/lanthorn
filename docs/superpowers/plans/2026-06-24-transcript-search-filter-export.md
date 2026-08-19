# Transcript Search / Filter / Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `/search`, `/filter`, and `/export` slash commands over the transcript — case-insensitive search with `n`/`N` navigation, Story/Meta category filtering, and plain-text export — all sharing one filtered "visible-lines" view.

**Architecture:** A `TranscriptFilter` + `visible_transcript_indices()` view on `AppState` is the single source of which lines are live; render, scroll, search, and export all read it. Search state (`search_query`/`search_matches`/`search_idx`) and pure helpers live on `AppState`. The three commands are new `SlashOutcome` variants handled in `main.rs`. A small `export` module writes the file.

**Tech Stack:** Rust, ratatui 0.29; the merged `TranscriptKind`/slash/config systems.

## Global Constraints
- No `mapper`/`zvm` changes. Build + `cargo test --workspace` green AND warning-clean after every task.
- The transcript line/kind invariant holds: `transcript.len() == transcript_kinds.len()` (already true; do not break it).
- Defaults: filter `Both`; `[search] start_backward = true`, `key_back = 'n'`, `key_forward = 'N'`.
- Search is **case-insensitive substring** only. Search/filter are **runtime** state (not persisted); the three `[search]` settings ARE persisted config.
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Spec (source of truth — read it): `docs/superpowers/specs/2026-06-24-transcript-search-filter-export-design.md`.

## File structure
- **Modify `crates/app/src/state.rs`** — `TranscriptFilter`, `transcript_filter`, search state + helpers (`visible_transcript_indices`, `run_search`, `search_next`, `clear_search`).
- **Modify `crates/app/src/config.rs`** — `[search]` table (`SearchConfig { start_backward, key_back, key_forward }`).
- **Modify `crates/app/src/slash.rs`** — curated `search`/`filter`/`export`; new `SlashOutcome` variants; `help_text`.
- **Create `crates/app/src/export.rs`** — `export_transcript(...)`.
- **Modify `crates/app/src/main.rs`** — handle the three outcomes; search-nav keys.
- **Modify `crates/app/src/render/transcript.rs`** — render the filtered view; highlight matches; status counter + filter indicator.

---

### Task 1: Filter — `TranscriptFilter` + visible-lines view + render integration

**Files:** Modify `crates/app/src/state.rs`, `crates/app/src/render/transcript.rs`.

**Interfaces — Produces:**
- `pub enum TranscriptFilter { Both, Story, Meta }` (default `Both`); `AppState.transcript_filter: TranscriptFilter`.
- `pub fn visible_transcript_indices(&self) -> Vec<usize>` — indices into `self.transcript` whose `transcript_kinds[i]` passes the filter (`Both` → all; `Story`/`Meta` → matching kind), in order.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn visible_transcript_indices_respects_filter() {
    let mut s = AppState::default();
    s.push_transcript("story0");
    s.push_transcript_kind("meta1", TranscriptKind::Meta);
    s.push_transcript("story2");
    s.transcript_filter = TranscriptFilter::Both;
    assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2]);
    s.transcript_filter = TranscriptFilter::Story;
    assert_eq!(s.visible_transcript_indices(), vec![0, 2]);
    s.transcript_filter = TranscriptFilter::Meta;
    assert_eq!(s.visible_transcript_indices(), vec![1]);
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the enum + field + helper. Then in `render/transcript.rs` `render_middle`: build the filtered `lines: Vec<String>` and `kinds: Vec<TranscriptKind>` from `state.visible_transcript_indices()` (clone the selected lines/kinds), and pass THOSE to the existing `visible_wrapped_lines_kinded(&lines, &kinds, rows, scroll, width)` instead of the raw `state.transcript`/`state.transcript_kinds`. (The kind-aware wrap + gutter marker stay exactly as-is; they just receive the filtered slice.)
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(transcript): TranscriptFilter + visible-lines view, filtered render".

---

### Task 2: `[search]` config table

**Files:** Modify `crates/app/src/config.rs`.

**Interfaces — Produces:** `Config.search: SearchConfig` where `pub struct SearchConfig { pub start_backward: bool, pub key_back: char, pub key_forward: char }` with defaults `true`/`'n'`/`'N'`. Chars deserialize first-char-of-string (reuse the `command_prefix` char-deserialize approach already in `config.rs`). Read in `Config::resolve`; written by the format-preserving writer under a `[search]` table.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn search_config_defaults_and_round_trip() {
    let d = Config::default();
    assert_eq!(d.search.start_backward, true);
    assert_eq!(d.search.key_back, 'n');
    assert_eq!(d.search.key_forward, 'N');
    let cfg: Config = toml::from_str("[search]\nstart_backward = false\nkey_forward = \"j\"\n").unwrap();
    assert_eq!(cfg.search.start_backward, false);
    assert_eq!(cfg.search.key_forward, 'j');
    assert_eq!(cfg.search.key_back, 'n'); // default kept
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `SearchConfig` (with `#[serde(default)]` on the field and per-field defaults via a `Default` impl + the char-from-string deserializer), wire it into `Config`, `resolve`, and `write_config` (emit a `[search]` table). Mirror the existing `command_prefix` char handling.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(search): [search] config table (direction + nav keys)".

---

### Task 3: Search state + pure helpers

**Files:** Modify `crates/app/src/state.rs`.

**Interfaces — Consumes:** `visible_transcript_indices`. **Produces:**
- `AppState.search_query: Option<String>`, `search_matches: Vec<usize>` (positions **within the visible-index list** of lines containing the query), `search_idx: usize`.
- `pub fn run_search(&mut self, query: &str, start_backward: bool) -> usize` — lowercases query + each visible line; fills `search_matches` (positions in the visible list whose line contains the query); sets `search_idx` to the LAST match if `start_backward` else the FIRST; sets `search_query = Some(query)`; returns match count (0 → also `search_query=None`? No — keep query set so the bottom line shows "no matches"; but `search_matches` empty).
- `pub fn search_next(&mut self, forward: bool) -> Option<usize>` — moves `search_idx` by ±1 with wrap (`forward=false` → toward start, `true` → toward end); returns the **visible-list position** of the new current match (for the caller to convert to a scroll offset), or `None` if no matches.
- `pub fn clear_search(&mut self)` — `search_query=None`, clears matches.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn run_search_direction_and_next_wrap() {
    let mut s = AppState::default();
    for t in ["alpha", "beta", "alpha again", "gamma", "ALPHA"] { s.push_transcript(t); }
    // matches for "alpha" at visible positions 0, 2, 4 (case-insensitive)
    let n = s.run_search("alpha", true); // start backward → last match
    assert_eq!(n, 3);
    assert_eq!(s.search_matches, vec![0, 2, 4]);
    assert_eq!(s.search_idx, 2); // index into search_matches → position 4
    // n = back
    assert_eq!(s.search_next(false), Some(2)); // now at match position 2
    // forward wraps from 2 → 4 → back to 0
    let _ = s.search_next(true); // → 4
    assert_eq!(s.search_next(true), Some(0)); // wrap to first
    let f = s.run_search("alpha", false); // start forward → first match
    assert_eq!(f, 3);
    assert_eq!(s.search_idx, 0);
    s.clear_search();
    assert!(s.search_query.is_none() && s.search_matches.is_empty());
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the fields + helpers. Matching is over `visible_transcript_indices()` lines, case-insensitive (`to_lowercase().contains`). `search_matches` stores positions in the visible list (0-based), not raw transcript indices.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(search): search state + run_search/search_next/clear_search".

---

### Task 4: Slash commands — `/search`, `/filter`, `/export`

**Files:** Modify `crates/app/src/slash.rs`.

**Interfaces — Produces:** new `SlashOutcome` variants `Search(Option<String>)`, `Filter(TranscriptFilterArg)`, `Export(Option<String>)` where `pub enum TranscriptFilterArg { Both, Story, Meta }` (slash.rs-local; main.rs maps it to `state::TranscriptFilter`). Curated entries: `search` (rest-of-line = query, empty → `Search(None)`), `filter` (arg `story|meta|both`; bad/missing → `Error`), `export` (optional filename → `Export(Some/None)`). `help_text(prefix)` lists all three.

- [ ] **Step 1: Write the failing test** (extend the existing slash parser test style)
```rust
#[test]
fn parse_search_filter_export() {
    assert!(matches!(parse("search twisty maze", '/'), SlashOutcome::Search(Some(q)) if q == "twisty maze"));
    assert!(matches!(parse("search", '/'), SlashOutcome::Search(None)));
    assert!(matches!(parse("filter meta", '/'), SlashOutcome::Filter(TranscriptFilterArg::Meta)));
    assert!(matches!(parse("filter both", '/'), SlashOutcome::Filter(TranscriptFilterArg::Both)));
    assert!(matches!(parse("filter nope", '/'), SlashOutcome::Error(_)));
    assert!(matches!(parse("export", '/'), SlashOutcome::Export(None)));
    assert!(matches!(parse("export out.txt", '/'), SlashOutcome::Export(Some(f)) if f == "out.txt"));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the variants + curated builders. `search`'s query is the whitespace-trimmed remainder after the command word (preserve internal spaces). Add the three to `help_text` and `slash_names`.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(slash): /search /filter /export outcomes + parsing".

---

### Task 5: Export module + `/filter` & `/export` handling

**Files:** Create `crates/app/src/export.rs`; Modify `crates/app/src/lib.rs` (`pub mod export;`), `crates/app/src/main.rs`.

**Interfaces — Produces:** `pub fn export_transcript(lines: &[String], dest: Option<&str>, exports_dir: &Path, stamp: &str) -> std::io::Result<PathBuf>` — resolves the destination: `dest=None` → `exports_dir/transcript-<stamp>.txt`; `dest=Some(name)` with no `/` → `exports_dir/name`; `dest=Some(path)` containing `/` → that path as-is. Creates parent dirs; writes `lines.join("\n") + "\n"`; returns the written path. **Consumes (main.rs):** `state.transcript_filter`, `visible_transcript_indices`, `SlashOutcome::{Filter, Export}`.

- [ ] **Step 1: Write the failing test** (tempdir)
```rust
#[test]
fn export_transcript_resolves_dest_and_writes() {
    let dir = std::env::temp_dir().join(format!("lanthorn-export-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let lines = vec!["a".to_string(), "b".to_string()];
    let p1 = export_transcript(&lines, None, &dir, "20260624-120000").unwrap();
    assert_eq!(p1, dir.join("transcript-20260624-120000.txt"));
    assert_eq!(std::fs::read_to_string(&p1).unwrap(), "a\nb\n");
    let p2 = export_transcript(&lines, Some("out.txt"), &dir, "x").unwrap();
    assert_eq!(p2, dir.join("out.txt"));
    let _ = std::fs::remove_dir_all(&dir);
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `export.rs`. In `main.rs`: handle `SlashOutcome::Filter(arg)` → set `state.transcript_filter` (map the arg) + status `filter: <mode>`; handle `SlashOutcome::Export(dest)` → build the filtered lines (`visible_transcript_indices` → `state.transcript[i].clone()`), compute the exports dir (the lanthorn user dir + `exports/`) and a `stamp` from `std::time::SystemTime` (format `YYYYMMDD-HHMMSS`; a helper is fine), call `export_transcript`, set status to the written path or the error. (Use the existing user-dir resolution in main.rs for the base path.)
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(transcript): export module + /filter and /export handling".

---

### Task 6: `/search` handling, search-nav keys, highlight + indicators

**Files:** Modify `crates/app/src/main.rs`, `crates/app/src/render/transcript.rs`.

**Interfaces — Consumes:** `run_search`/`search_next`/`clear_search`, `config.search`, `search_query`/`search_matches`/`search_idx`. **Produces:** the live search experience.

- [ ] **Step 1: Write the failing test** (a pure scroll-to-match helper + a render assertion)
```rust
// in main.rs (or state.rs): given the current match's visible-list position and the
// total visible-row count + pane rows, compute the scroll so the match row is visible.
// fn scroll_for_match(match_visible_pos: usize, total_visible: usize, pane_rows: usize) -> u16
#[test]
fn scroll_for_match_brings_row_into_view() {
    // a match near the top of a long transcript scrolls back far enough to show it
    assert_eq!(scroll_for_match(0, 100, 10), /* expected scroll putting line 0 in view */ 90);
    // a match at the bottom needs no scroll
    assert_eq!(scroll_for_match(99, 100, 10), 0);
}
```
(Refine the exact arithmetic against the existing `visible_wrapped_lines_kinded` windowing — `scroll` counts wrapped rows from the bottom; this helper maps a logical visible-line position to that scroll. Read the windowing before finalizing the formula and assertion.)
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement:**
  - `main.rs` `SlashOutcome::Search(q)`: `q=Some` → `state.run_search(q, cfg.search.start_backward)`, scroll to the current match, status `"N matches"`/`"no matches"`; `q=None` → repeat last (`run_search(last_query, start_backward)` — keep the last query in `search_query` even after nav; if none, status `no previous search`).
  - Search-nav keys (when `state.search_query.is_some()`, before normal input): pressed key == `cfg.search.key_back` → `search_next(false)` + scroll; == `cfg.search.key_forward` → `search_next(true)` + scroll; `Esc` → `clear_search()`; ANY other key → `clear_search()` then fall through to normal processing.
  - `render/transcript.rs`: when `search_query` is set, highlight every case-insensitive occurrence of the query within each rendered row using a highlight style (add a `search_highlight` style — either reuse an existing accent or a new `meta_marker`-style selector is NOT required; a hardcoded reversed/yellow is acceptable, but prefer `state.colors` if a fitting field exists). Draw the bottom hint `search: <q>  [i/N]  <back>:back <fwd>:fwd  Esc:clear` (reuse the suggestion/status row area), and a `[filter: story|meta]` indicator when `transcript_filter != Both`.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(search): /search handling, n/N nav, match highlight + indicators".

---

## Self-Review

**Spec coverage:**
- `/search` (case-insensitive, backward-default, forward option, n/N+Esc, no-arg repeat) → Tasks 3, 6 (+ config Task 2). ✅
- `/filter story|meta|both` + indicator → Tasks 1, 5, 6. ✅
- `/export` auto-path honoring filter + filename override → Task 5. ✅
- Shared visible-lines view driving render/scroll/search/export → Task 1 (view), consumed by 3/5/6. ✅
- `[search]` config (start_backward, key_back, key_forward) → Task 2. ✅
- Slash commands as the entry points → Task 4. ✅

**Placeholder scan:** Task 6's `scroll_for_match` test has a `/* expected */` note because the exact constant depends on the existing windowing — the step explicitly directs reading `visible_wrapped_lines_kinded` first and finalizing the formula+assertion; that's a concrete derivation, not a vague directive. All other tests are concrete.

**Type consistency:** `TranscriptFilter`/`transcript_filter`/`visible_transcript_indices`, `SearchConfig`/`search.{start_backward,key_back,key_forward}`, `search_query`/`search_matches`/`search_idx`, `run_search(query,start_backward)`/`search_next(forward)`/`clear_search`, `SlashOutcome::{Search,Filter,Export}`/`TranscriptFilterArg`, `export_transcript(lines,dest,dir,stamp)` — consistent across tasks.

## Notes for the executor
- Tasks 1→3 are the data spine (filter view → search state); 4 is pure parsing; 5 (export+filter) and 6 (search+render) are the integration. Order matters: 1 before 3/5/6; 2 before 6.
- The kind-aware wrap + gutter marker in `render/transcript.rs` already exist — Task 1 feeds them the FILTERED slice; do not rewrite them.
- Keep search/filter as runtime state; only the three `[search]` settings persist.
