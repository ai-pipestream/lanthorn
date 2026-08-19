# Hint System Phase 1 (+2 zip) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax. **Parallelism:** Task A (discovery, new file) is file-disjoint and may run concurrently with anything; Tasks B/D/E share `main.rs`/`keymap.rs` and serialize; Task C (panel render, new file) depends only on B's types. Dispatch disjoint tasks together.

**Goal:** A Hints modal mini-terminal that runs a companion Invisiclues `.z5` in a second Z-machine session, with discovery (remembered + sibling files + file-browser + built-in-HINT suggestion) and zip support for adventures/hint files.

**Architecture:** A `HintSource::Zcode(GameSession)` inside `AppState.hints: Option<HintSession>`; a `hints.rs` discovery+store module; a `render/hints_panel.rs` modal; `main.rs` opens/routes/closes the panel; the main game pauses while open.

**Tech Stack:** Rust, ratatui 0.29, the existing `GameSession`/dialog/file-browser/zip systems.

## Global Constraints
- No `mapper`/`zvm` changes. Build + `cargo test --workspace` green AND warning-clean after every task.
- The live `HintSession` is transient — NOT written into the `.lanthorn` archive. Only the per-IFID hint-file association persists (Task A's store).
- Spec (source of truth — read it): `docs/superpowers/specs/2026-06-24-hint-system-design.md`.
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

## File structure
- **Create `crates/app/src/hints.rs`** (Task A) — discovery, per-IFID store, `story_supports_hint`, zip helpers (Task E).
- **Modify `crates/app/src/state.rs`** (Task B) — `HintSource`/`HintSession`/`hints` + `any_overlay_open`.
- **Modify `keymap.rs`, `slash.rs`, `input.rs`** (Task B) — `Command::OpenHints`/`Action::OpenHints`/`/hint`.
- **Create `crates/app/src/render/hints_panel.rs`** (Task C) — `draw_hints_panel`.
- **Modify `crates/app/src/main.rs`** (Task D) — open/route/close + sub-session.
- **Modify `crates/app/src/main.rs`, `hints.rs`** (Task E) — zip story load + hint-in-zip discovery.

---

### Task A: Discovery + per-IFID store + built-in-HINT check (new file — PARALLEL-SAFE)

**Files:** Create `crates/app/src/hints.rs`; Modify `crates/app/src/lib.rs` (`pub mod hints;`).

**Interfaces — Produces (self-contained; no dependency on the state types):**
- `pub enum HintResolution { File(PathBuf), AskUser, None }`
- `pub fn resolve_hint_source(story_path: &Path, ifid: &str, index: &HintIndex) -> HintResolution` — step 1 remembered (`index.get(ifid)`), step 2 sibling files by name pattern; else `AskUser`.
- `pub fn hint_name_matches(file_name: &str) -> bool` — true for `*hint*`/`*clue*`/`*invisiclues*` with a `.z3/.z5/.z8` extension.
- `pub struct HintIndex(...)` + `pub fn load_hint_index(dir: &Path) -> HintIndex` / `pub fn save_hint_assoc(dir: &Path, ifid: &str, path: &Path) -> io::Result<()>` — a small TOML at `dir/hints/index.toml` mapping `ifid -> path`.
- `pub fn story_supports_hint<I: IntoIterator<Item = String>>(dictionary: I) -> bool` — true if the story dictionary contains `hint`/`hints` (case-insensitive).

- [ ] **Step 1: Write failing tests**
```rust
#[test]
fn hint_name_matches_patterns() {
    assert!(hint_name_matches("zork1.invisiclues.z5"));
    assert!(hint_name_matches("MyGame-hints.z5"));
    assert!(hint_name_matches("clues.z3"));
    assert!(!hint_name_matches("zork1.z5"));     // the story itself
    assert!(!hint_name_matches("hints.txt"));    // wrong extension
}
#[test]
fn story_supports_hint_detects_dictionary_word() {
    assert!(story_supports_hint(["look","hint","take"].map(String::from)));
    assert!(!story_supports_hint(["look","take"].map(String::from)));
}
#[test]
fn hint_index_round_trips() {
    let dir = std::env::temp_dir().join(format!("bm-hintidx-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    save_hint_assoc(&dir, "ZCODE-1", std::path::Path::new("/x/h.z5")).unwrap();
    let idx = load_hint_index(&dir);
    assert_eq!(idx.get("ZCODE-1"), Some(std::path::PathBuf::from("/x/h.z5")));
    let _ = std::fs::remove_dir_all(&dir);
}
#[test]
fn resolve_finds_sibling_then_asks() {
    // temp dir: story.z5 + story.hints.z5 -> File(story.hints.z5); without -> AskUser
    // (build the scene; assert both outcomes)
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the module + `pub mod hints;`. Use the `toml`/`toml_edit` already in deps for the index; `std::fs` for sibling scan.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(hints): discovery + per-IFID store + built-in-HINT check".

---

### Task B: Hint state + command + slash (foundation)

**Files:** Modify `crates/app/src/state.rs`, `keymap.rs`, `input.rs`, `slash.rs`.

**Interfaces — Produces:**
- `state.rs`: `pub enum HintSource { Zcode(crate::session::GameSession) }`; `pub struct HintSession { pub source: HintSource, pub transcript: Vec<String>, pub scroll: u16, pub input: String, pub label: String, pub builtin_hint: bool }`; `AppState.hints: Option<HintSession>`; `any_overlay_open()` includes `hints.is_some()`.
- `keymap.rs`: `Command::OpenHints` (kebab `open_hints`) → `Action::OpenHints`; add to `label()`, `name()`, `ALL_COMMANDS`, the context (Global). Mirror an existing simple command end-to-end.
- `slash.rs`: curated `hint` AND `hints` entries → a new `SlashOutcome::OpenHints` (caller-handled). `help_text` lists `/hint`.
- `input.rs`: `Action::OpenHints` variant (handler in Task D; here just the enum arm or a stub that sets a flag — keep it compiling).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn hints_panel_counts_as_overlay() {
    let mut s = AppState::default();
    assert!(!s.any_overlay_open());
    // construct a minimal HintSession or set via a test helper; assert any_overlay_open() true.
}
#[test]
fn slash_hint_parses_to_open_hints() {
    assert!(matches!(crate::slash::parse("hint", '/'), crate::slash::SlashOutcome::OpenHints));
    assert!(matches!(crate::slash::parse("hints", '/'), crate::slash::SlashOutcome::OpenHints));
}
```
- [ ] **Step 2–4:** confirm fail → implement → full test green + warning-clean.
- [ ] **Step 5: Commit** — "feat(hints): HintSession state + OpenHints command + /hint slash".

---

### Task C: Hints panel render (new file — depends only on Task B types)

**Files:** Create `crates/app/src/render/hints_panel.rs`; Modify `render/mod.rs`.

**Interfaces — Consumes:** `state.hints`, `state.colors.dialog*`. **Produces:** `pub struct HintsPanelRects { area, close, input }`; `pub fn draw_hints_panel(state, area, buf) -> Option<HintsPanelRects>` — returns None when `state.hints.is_none()`. Renders the dialog chrome (title = `HintSession.label`, `[X]`), the hint session's `transcript` (word-wrapped + scrolled — reuse `render/transcript.rs` wrap helpers), a `builtin_hint` suggestion line ("This game has its own hints — type HINT") when set, and the panel's own input line. Mirror `render/gallery.rs`/`reset_dialog.rs` for the chrome.

- [ ] **Step 1:** Failing TestBackend test: with a `HintSession{ label:"Hints: X", transcript:["pick a topic"], builtin_hint:true, .. }` set, `draw_hints_panel` returns rects and the buffer contains the title, the transcript text, the "type HINT" suggestion, and an input row.
- [ ] **Step 2–4:** fail → implement → green + warning-clean.
- [ ] **Step 5: Commit** — "feat(hints): hints panel render (mini-terminal chrome)".

---

### Task D: Open / route / close + sub-session (integration)

**Files:** Modify `crates/app/src/main.rs`.

**Interfaces — Consumes:** `hints::{resolve_hint_source, load_hint_index, save_hint_assoc, story_supports_hint, HintResolution}`, `GameSession::new`, `draw_hints_panel`, `state.hints`. **Produces:** the working panel.

- [ ] **Step 1: Write the failing test** (a pure routing helper)
```rust
// fn hint_key_routes_to_session(code: KeyCode) -> HintKeyKind { Close, ToSession }  (Esc -> Close, else ToSession)
#[test]
fn hint_panel_keys_close_on_esc_else_route() {
    assert!(matches!(hint_key_routes(KeyCode::Esc), HintKeyKind::Close));
    assert!(matches!(hint_key_routes(KeyCode::Char('a')), HintKeyKind::ToSession));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement:**
  - `Action::OpenHints`: if `state.hints.is_some()` no-op; else resolve — `story_supports_hint(dictionary)` sets a pending `builtin_hint`; `resolve_hint_source(...)` → `File(p)` → load bytes (Task E's `load_story_bytes` if available, else `fs::read`) → `GameSession::new` → build `HintSession` (label from filename/title, take the opening `take_transcript`), set `state.hints`; `AskUser` → open the file browser (reuse the existing modal) and, on pick, `save_hint_assoc` + start the session; `None` → status "no hints found".
  - Panel input intercept (BEFORE normal routing, gated on `state.hints.is_some()`, mirror the other modal intercepts): `Esc`/`[X]` → `state.hints = None` (drop the session); Enter → submit the panel input line to the hint `GameSession` (`session.submit` + append `take_transcript` to the hint transcript), clear the input; printable chars → append to the hint input; Backspace → pop. Mouse `[X]` → close. The MAIN game session is never advanced while the panel is open.
  - Render: call `draw_hints_panel` after other overlays; stash rects.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(hints): open/route/close the hints panel over a 2nd VM session".

---

### Task E: Zip support (Phase 2)

**Files:** Modify `crates/app/src/hints.rs`, `crates/app/src/main.rs`.

**Interfaces — Produces:** `pub fn load_story_bytes(path: &Path) -> io::Result<Vec<u8>>` (raw `.z*` OR the first story `.z*` inside a zip, detected by `PK\x03\x04` magic); `pub fn read_zip_entry(zip: &Path, pred: impl Fn(&str) -> bool) -> io::Result<Option<Vec<u8>>>`. Discovery gains step 3: if the story came from / has a sibling `.zip`, find a `hint_name_matches` entry inside it.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn load_story_bytes_handles_raw_and_zip() {
    // write raw z5 bytes -> load_story_bytes returns them;
    // pack the same bytes as entry "game.z5" in a zip -> load_story_bytes returns identical bytes.
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the helpers (reuse the `zip` crate already used by `archive.rs`); wire `load_story_bytes` into the `main.rs` story load (~line 491) and the hint-load path; add discovery step 3.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(hints): zip support for adventures and hint files".

---

## Self-Review
**Spec coverage:** Zcode source + modal mini-terminal → B/C/D ✅; discovery (remembered/sibling/file-browser) + per-IFID store + built-in-HINT → A/D ✅; zip (Phase 2) → E ✅; transient session / persisted association → A (store) + B (transient field) ✅; pluggable `HintSource` enum seam for UHS → B ✅.
**Placeholder scan:** Task A's `resolve_finds_sibling_then_asks` and Task C/D render/route tests are sketched against concrete outcomes; the implementer builds the temp-dir/TestBackend scene. Not vague.
**Type consistency:** `HintSource`/`HintSession`/`hints`, `HintResolution`/`resolve_hint_source`/`hint_name_matches`/`HintIndex`/`load_hint_index`/`save_hint_assoc`/`story_supports_hint`, `Command::OpenHints`/`Action::OpenHints`/`SlashOutcome::OpenHints`, `draw_hints_panel`/`HintsPanelRects`, `load_story_bytes`/`read_zip_entry` — consistent across tasks.

## Notes for the executor
- **Parallelism:** Task A is a new file disjoint from everything — run it concurrently with the #11 wave / other work. C depends on B's types; D depends on A+B+C; E depends on A. Dispatch A immediately; B next; then C in parallel with starting D's reading; E after A (can overlap D if it only touches its own helper region — but both touch main.rs, so serialize E after D).
- UHS (Phase 3) and online download are OUT of scope here (separate future plan); the `HintSource` enum is the seam.
