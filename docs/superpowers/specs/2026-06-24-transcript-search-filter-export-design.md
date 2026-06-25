# Transcript Search / Filter / Export — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO item:** #52 — "SEARCH the console/transcript output (find + highlight/jump), EXPORT the console to a plain text file, and FILTER the transcript by category (only STORY, only META, or BOTH)."
**Depends on:** the `TranscriptKind { Story, Meta }` tagging + the slash-command system (both merged). No `mapper`/`zvm` changes.
**Touches:** `crates/app/src/slash.rs` (3 new commands), `crates/app/src/state.rs` (filter + search state + a visible-lines helper), `crates/app/src/main.rs` (slash handling + search-nav keys), `crates/app/src/render/transcript.rs` (filter, highlight, indicators), a small export module.

## Goal

Make the transcript searchable, filterable by category, and exportable to a text file — all driven through the slash-command syntax that just shipped, so there are no new modes to learn: `/search <query>`, `/filter story|meta|both`, `/export [file]`.

## Interaction model

All three are **slash commands** (curated table entries, status-line feedback). They consume the existing `/`-prefix infrastructure.

### `/search <query>`
- Case-insensitive substring search over the **currently visible** transcript lines (i.e. after the active filter).
- Highlights every match in the transcript, scrolls to the first match, and shows `"N matches"` (or `"no matches"`) on the status line.
- Activates a transient **search-navigation state** (`search_query.is_some()`): while active, the bottom line shows `search: <query>  [i/N]  n:next N:prev  Esc:clear`, and the keys:
  - `n` → next match, `N` → previous match (wraps; auto-scrolls the transcript so the match is visible),
  - `Esc` → clear the search (drop highlight, leave the transcript where it is), resume normal input.
  - While search-nav is active, `n`/`N`/`Esc` are intercepted *before* the game input line; all other keys clear the search and are processed normally (so typing a new game command implicitly exits search). This keeps the modal footprint tiny.
- `/search` with **no query** repeats the last search (re-resolves matches against the current transcript/filter and jumps to the next).
- Matches are recomputed if the transcript grows while a search is active (new lines just extend the match set on next navigation).

### `/filter story | meta | both`
- Sets which categories render: `both` (default), `story` (game output only), `meta` (slash output, `/help`, app messages only).
- Unknown/missing arg → status-line error `filter: use story | meta | both`.
- When the filter is **not** `both`, a small indicator `[filter: story]` / `[filter: meta]` shows on the status line so the filtered state is never invisible. `both` shows no indicator (no clutter).
- The filter affects the rendered transcript, the scrollback extent, search scope, and export contents (one shared **visible-lines** view, below).

### `/export [file]`
- Writes the **currently visible** lines (honoring the active filter) as **plain text** (no `[STORY]`/`[META]` tags) to a file.
- Default path: `~/.babelmap/exports/transcript-<UTC-timestamp>.txt` (the exports dir is created if missing; timestamp from `std::time`, format `YYYYMMDD-HHMMSS`). The status line shows the written path.
- `/export <file>` overrides the destination: an absolute/relative path is used as-is; a bare name (no `/`) is written into the default exports dir. Parent dirs are created. On write error, a status-line error is shown.

## Architecture

### Shared visible-lines view (the linchpin)
A single helper on `AppState` (or a small free fn taking `&transcript, &kinds, filter`) returns the indices of transcript lines that pass the active filter:

```rust
pub enum TranscriptFilter { Both, Story, Meta }   // default Both

/// Indices into `self.transcript` that pass `self.transcript_filter`, in order.
pub fn visible_transcript_indices(&self) -> Vec<usize>
```

Render, scroll, search, and export ALL operate on this list, so they stay consistent by construction. (Story/Meta come from the existing parallel `transcript_kinds` vec.)

### State (`state.rs`)
- `transcript_filter: TranscriptFilter` (default `Both`).
- `search_query: Option<String>` (None = inactive).
- `search_matches: Vec<usize>` — indices **into the visible list** of lines that contain the query.
- `search_idx: usize` — current position within `search_matches`.
- Helpers: `visible_transcript_indices()`, `run_search(query)` (lowercases, fills `search_matches`/`search_idx`, returns count), `search_next(forward: bool)` (advances `search_idx` with wrap, returns the target visible-line row for scrolling), `clear_search()`.

### Slash (`slash.rs`)
- Curated entries `search`, `filter`, `export`. New `SlashOutcome` variants (caller-handled, like `Save`/`Load`):
  - `Search(Option<String>)`, `Filter(TranscriptFilterArg)`, `Export(Option<String>)`.
  - `help_text()` gains the three with their prefix.

### main.rs
- Handle the new outcomes: `Search` → `state.run_search` + scroll + status `"N matches"`; `Filter` → set `state.transcript_filter` + status; `Export` → build text from visible lines + write file + status (path or error).
- Search-nav keys: when `state.search_query.is_some()`, intercept `n`/`N`/`Esc` (and "any other key clears") before the normal input path.

### Render (`transcript.rs`)
- Iterate `visible_transcript_indices()` instead of the raw transcript for the scrollback body; map `transcript_scroll` onto the visible list.
- When `search_query` is set, highlight every case-insensitive occurrence of the query within each rendered line (a distinct highlight style; the *current* match gets a stronger style).
- Status line: show the search counter `[i/N]` + nav hint while search-active, and the `[filter: …]` indicator when filter ≠ Both. Reuse the existing `status_msg`/status-row plumbing from the slash wave.

### Export module
- A small `export_transcript(lines: &[String], dest: Option<&str>, exports_dir: &Path) -> io::Result<PathBuf>` (pure-ish; takes the already-filtered lines and resolves the destination per the rules above). Unit-testable without a terminal.

## Testing
- `visible_transcript_indices`: Both → all; Story → only Story rows; Meta → only Meta rows (build a transcript with mixed kinds).
- `run_search`/`search_next`: matches are the right visible indices; `search_next` wraps forward/back; case-insensitive; no-match → empty + a clear status.
- Filter parse: `story|meta|both` set the enum; bad arg → error outcome.
- `export_transcript`: bare name → exports dir; explicit path → as-is; returns the path; file contents equal the (filtered) lines joined by newlines; missing dir is created (use a tempdir).
- Slash parse: `/search foo`, `/search`, `/filter meta`, `/export`, `/export out.txt` map to the right `SlashOutcome` variants (extend the existing slash parser test).
- Render (TestBackend): with filter=Story only Story lines appear; with an active search the query substring is rendered with the highlight style and the `[i/N]` counter shows.

## Out of scope / non-goals
- Regex or fuzzy search (plain case-insensitive substring only).
- Searching anything other than the story transcript (not the map, not room notes).
- Persisting the filter/search across sessions (runtime only; not written to config).
- Rich export formats (HTML/markdown) — plain text only.
- A separate search dialog/overlay (slash-driven by decision).

## Risks & limitations (accepted)
- **Search-nav modality:** while a search is active, `n`/`N` are reserved for navigation; the "any other key clears" rule keeps this from trapping the user, and the bottom-line hint makes the state explicit.
- **Highlight + filter interaction:** matches are computed over the filtered view, so changing the filter while a search is active re-scopes matches (recomputed on next `/search`/navigation).
- **Large transcripts:** substring scan is O(lines × query) per search — fine for interactive transcripts; no indexing needed.

## Sources
- Transcript storage + kinds: `crates/app/src/state.rs` (`transcript: Vec<String>`, `transcript_kinds: Vec<TranscriptKind>`).
- Transcript render + status row: `crates/app/src/render/transcript.rs`.
- Slash system: `crates/app/src/slash.rs` (`SlashOutcome`, curated table, `help_text`), handled in `main.rs`.
