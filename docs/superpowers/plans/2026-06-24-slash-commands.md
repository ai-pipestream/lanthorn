# Slash Commands Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route input lines starting with a configurable prefix (default `/`) as app commands (a curated parameterized table + every Command by kebab name), with quiet status-line feedback, `/help`, and `/`-mode Tab autocomplete.

**Architecture:** A pure `slash.rs` (parse + curated table + name set + help text) feeds the `main.rs` submit interception; a `command_prefix` config setting gates it; the autocomplete engine gains a `/`-mode; a transient `status_msg` renders on a status line; transcript entries gain a `Story|Meta` kind.

**Tech Stack:** Rust; the existing `keymap::Command` registry, `Action` enum, `complete.rs` autocomplete, `config::Config`.

## Global Constraints

- Prefix is `state.config.command_prefix` (a `char`, default `'/'`); ALL prefix checks read it (not a hardcoded `/`).
- A prefix-command submit must NOT call `session.submit`, NOT increment `state.turns`, NOT push a `> cmd` story line.
- Quiet feedback: visible-effect commands (pan/zoom/center/tidy/layer) just dispatch; save/load/reset + ALL errors set `status_msg` (status line, not transcript). `/help` prints to the transcript tagged `Meta`.
- Curated table entries WIN over the kebab `Command::from_name` fallback on name collision.
- No `mapper`/`zvm` changes. Build + `cargo test --workspace` green and warning-clean after every task.
- **Prereq:** wave19 (dialog chrome) must be merged first — this plan edits `main.rs`/`input.rs`/`transcript.rs`/`config.rs` which wave19 also touches.
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Spec: `docs/superpowers/specs/2026-06-24-slash-commands-design.md` (source of truth; read it).

## File structure
- **Create `crates/app/src/slash.rs`** — `SlashOutcome`, `parse`, the curated table + builders, `slash_names()`, `help_text()`.
- **Modify `lib.rs`** — `pub mod slash;`.
- **Modify `config.rs`** — `command_prefix: char`.
- **Modify `state.rs`** — `status_msg: Option<String>`; transcript-entry `TranscriptKind`.
- **Modify `main.rs`** — submit interception + caller-level handling.
- **Modify `input.rs`/`complete.rs`** — `/`-mode autocomplete.
- **Modify `render/transcript.rs`** — status-line render + Meta tagging.

---

### Task 1: slash.rs — parse + curated table + fallback

**Files:** Create `crates/app/src/slash.rs`; Modify `lib.rs`.

**Interfaces — Produces:**
- `pub enum SlashOutcome { Action(crate::input::Action), Message(String), Error(String), Help, Save(Option<String>), Load(Option<String>), Reset { map: bool }, Quit }`
  (Save/Load/Reset/Quit/Help are caller-handled; Action covers map/zoom/etc.; Message/Error set the status line.)
- `pub fn parse(body: &str) -> SlashOutcome` — `body` is the input WITHOUT the leading prefix. Whitespace-tokenize; empty → `Error("type /help for commands")`; match token0 against the curated table (run its builder over the args); else `keymap::Command::from_name(token0).map(|c| Action(c.to_action()))`; else `Error("unknown command: /<t0> — try /help")`.
- `pub fn slash_names() -> Vec<String>` — curated names ∪ `keymap::ALL_COMMANDS` kebab names (for autocomplete).
- `pub fn help_text() -> Vec<String>` — the `/help` lines.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn parse_curated_and_fallback_and_errors() {
    use crate::input::Action;
    assert!(matches!(parse("panh -1"), SlashOutcome::Action(Action::Pan(-1,0))));
    assert!(matches!(parse("panv 2"), SlashOutcome::Action(Action::Pan(0,2))));
    assert!(matches!(parse("zoom reset"), SlashOutcome::Action(Action::ZoomReset)));
    assert!(matches!(parse("save foo"), SlashOutcome::Save(Some(_))));
    assert!(matches!(parse("save"), SlashOutcome::Save(None)));
    assert!(matches!(parse("reset map"), SlashOutcome::Reset{map:true}));
    assert!(matches!(parse("reset"), SlashOutcome::Reset{map:false}));
    assert!(matches!(parse("quit"), SlashOutcome::Quit));
    assert!(matches!(parse("help"), SlashOutcome::Help));
    // fallback by kebab name:
    assert!(matches!(parse("open-config"), SlashOutcome::Action(_)));
    // errors:
    assert!(matches!(parse("panh"), SlashOutcome::Error(_)));   // missing arg
    assert!(matches!(parse("nope"), SlashOutcome::Error(_)));   // unknown
    assert!(matches!(parse(""), SlashOutcome::Error(_)));       // bare prefix
}

#[test]
fn slash_names_includes_curated_and_fallback() {
    let n = slash_names();
    assert!(n.iter().any(|s| s == "panh"));
    assert!(n.iter().any(|s| s == "open-config")); // a kebab Command name
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `SlashOutcome`, `parse`, the curated table (panh/panv/zoom/center/tidy/layer/save/load/reset/quit/help + aliases), `slash_names`, `help_text`; add `pub mod slash;` to lib.rs. Verify `keymap::ALL_COMMANDS`/`Command::name()`/`from_name`/`to_action` signatures before use.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(slash): parser + curated table + fallback".

---

### Task 2: config command_prefix + status_msg + transcript kind

**Files:** Modify `config.rs`, `state.rs`.

**Interfaces — Produces:**
- `Config.command_prefix: char` (default `'/'`); read in `Config::resolve` from the file (e.g. a `command_prefix = "/"` TOML string → first char).
- `AppState.status_msg: Option<String>`; a helper `set_status(&mut self, msg)`.
- `pub enum TranscriptKind { Story, Meta }`; the transcript store tags each entry. Minimal: change the push API to `push_transcript_kind(&mut self, text, kind)` and keep `push_transcript` = `push_transcript_kind(text, Story)`. Store kinds alongside lines (a parallel `Vec<TranscriptKind>` or a `{text,kind}` struct — pick the smaller diff vs the current transcript storage; read it first).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn config_reads_command_prefix() {
    let cfg: Config = toml::from_str("command_prefix = \";\"\n").unwrap();
    assert_eq!(cfg.command_prefix, ';');
    assert_eq!(Config::default().command_prefix, '/');
}

#[test]
fn transcript_tags_story_and_meta() {
    let mut s = AppState::default();
    s.push_transcript("West of House");          // Story
    s.push_transcript_kind("/help line", TranscriptKind::Meta);
    // assert the last entry's kind is Meta and the prior is Story (via whatever accessor exists)
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the config field (custom deserialize char-from-string or a `String`+first-char), `status_msg`/`set_status`, and `TranscriptKind` + the tagged push.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(slash): command_prefix config + status_msg + transcript kind".

---

### Task 3: main.rs submit interception + caller-level handling

**Files:** Modify `crates/app/src/main.rs`.

**Interfaces — Consumes:** `slash::{parse, SlashOutcome, help_text}`, `state.config.command_prefix`, `state.status_msg`, `apply_action`. **Produces:** the `SubmitCommand` handler routes prefix-input through slash instead of the VM.

- [ ] **Step 1: Write the failing test** (extract the routing decision into a small testable helper, since the run loop isn't unit-testable)
```rust
// in main.rs: fn is_slash(input: &str, prefix: char) -> bool { input.starts_with(prefix) }
#[test]
fn is_slash_uses_prefix() {
    assert!(is_slash("/save", '/'));
    assert!(!is_slash("look", '/'));
    assert!(is_slash(";help", ';'));
    assert!(!is_slash("/help", ';'));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** in the `SubmitCommand` arm (after `state.take_input()`, before `session.submit`): if `is_slash(&cmd, state.config.command_prefix)`, strip the prefix, `slash::parse(body)` and handle each `SlashOutcome`:
  - `Action(a)` → `apply_action(&mut state, a, …)` (dispatch like a keybinding);
  - `Message(m)`/`Error(m)` → `state.set_status(m)`;
  - `Help` → push each `help_text()` line as `Meta`;
  - `Save(name)`/`Load(name)`/`Reset{map}`/`Quit` → call the existing save/restore/reset/quit handlers (reuse the code the `SaveGame`/`RestoreGame`/`ResetGame`/`Quit` actions already run), set a status like `saved`/`loaded`/`reset`;
  - In ALL slash cases: do NOT `session.submit`, do NOT `turns += 1`, do NOT push `> cmd`.
  Else (no prefix): unchanged game flow.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(slash): route prefix input to app commands at submit".

---

### Task 4: `/`-mode autocomplete

**Files:** Modify `crates/app/src/input.rs` (`recompute_suggestions`), maybe `complete.rs`.

**Interfaces — Consumes:** `slash::slash_names()`, `state.config.command_prefix`. **Produces:** when the input starts with the prefix, Tab/suggestions complete the FIRST token from `slash_names()` (prefix-filtered) instead of the dictionary.

- [ ] **Step 1: Write the failing test**
```rust
// factor the choice into a pure helper:
// fn slash_suggestions(body_token: &str, names: &[String], limit: usize) -> Vec<String>
#[test]
fn slash_suggestions_filter_by_prefix() {
    let names = vec!["panh".to_string(),"panv".to_string(),"zoom".to_string(),"open-config".to_string()];
    let s = slash_suggestions("pa", &names, 6);
    assert!(s.contains(&"panh".to_string()) && s.contains(&"panv".to_string()));
    assert!(!s.contains(&"zoom".to_string()));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `slash_suggestions` + branch `recompute_suggestions`: `if state.input.starts_with(prefix) { complete the (single) token after the prefix from slash_names } else { existing dictionary path }`. (Once the name is complete + takes args, show the arg hint string — optional polish; the name completion is the required behavior.)
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(slash): prefix-mode Tab autocomplete over command names".

---

### Task 5: status-line render + Meta-tagged transcript

**Files:** Modify `crates/app/src/render/transcript.rs`.

**Interfaces — Consumes:** `state.status_msg`. **Produces:** the transient status message renders on a status line (reuse/extend the existing status-bar row); cleared on the next keypress/turn (clear it in the input/turn handlers).

- [ ] **Step 1: Write the failing test** (TestBackend)
```rust
#[test]
fn status_msg_renders_on_status_line() {
    // build AppState with status_msg = Some("saved"); render the transcript pane;
    // assert "saved" appears on the status row.
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the status-line render (when `status_msg.is_some()`, draw it on the status row with `state.colors.status_bar` or a dedicated style); ensure it's cleared on the next input char/turn (set `status_msg=None` in the InputChar/Submit handlers).
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(slash): render transient status message".

---

## Self-Review

**Spec coverage:**
- Curated table + kebab fallback + parser → Task 1. ✅
- Configurable prefix → Tasks 2 (config), 3 (interception), 4 (autocomplete). ✅
- Quiet feedback (status_msg; save/load/reset + errors; visible-effect silent) → Tasks 2, 3, 5. ✅
- `/help` → transcript tagged Meta → Tasks 2 (kind), 3 (push). ✅
- `/`-mode autocomplete → Task 4. ✅
- TranscriptKind Story|Meta tag → Task 2. ✅
- No turn increment / no `> cmd` / no session.submit on slash → Task 3. ✅
- No mapper/zvm → Global Constraints. ✅

**Placeholder scan:** Tasks 3 and 5 note "reuse the existing save/restore/reset/quit handlers" and "the existing status-bar row" — concrete pointers to current code the implementer reads, not vague directives; the routing decision + status render are pinned by tests.

**Type consistency:** `SlashOutcome`, `parse`, `slash_names`, `help_text`, `is_slash`, `slash_suggestions`, `command_prefix`, `status_msg`/`set_status`, `TranscriptKind{Story,Meta}`, `push_transcript_kind` consistent across tasks.

## Notes for the executor
- Task 1 is pure (the bulk of the testable surface). Tasks 3–5 are integration — read the current `SubmitCommand` handler, the save/restore/reset/quit action handlers, the transcript storage, and `recompute_suggestions` before editing.
- The `Reset{map:true}` path is feature #49 (reset also clears the map) — wire it to the reset handler with the map-reset branch.
