# Reset Dialog + Room-Number Visibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the y/n reset text-prompt with a dialog-chrome modal carrying an opt-in "Also clear the map" checkbox, and hide room numbers by default with a runtime toggle (portal icons move to the freed row).

**Architecture:** A shared `reset_game(session, mapper, state, story_bytes, clear_map)` helper drives both the dialog confirm and the typed `/reset`. The reset dialog reuses `render/dialog.rs` chrome plus a small checkbox row it renders/hit-tests itself. Room-number visibility is a persisted config bool + runtime `AppState` flag read by `draw_box_room`/`draw_portal_icons`, toggled by a new `Command`/`Action`.

**Tech Stack:** Rust, ratatui 0.29, the existing dialog/keymap/config/symbol systems.

## Global Constraints

- No `mapper`/`zvm` changes (mapper's `MapGraph::new()` is reused, not modified).
- Build + `cargo test --workspace` green AND warning-clean (`cargo build --workspace` emits no `warning:`) after every task.
- Defaults: reset checkbox **unchecked** (map kept); `show_room_numbers` default **false** (hidden).
- `Esc` and `[X]` both Cancel/close every modal (uniform with existing dialogs).
- Typed `/reset` and `/reset map` stay **immediate** (no dialog); they call the same `reset_game` helper so map-clear semantics match the checkbox.
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Spec (source of truth — read it): `docs/superpowers/specs/2026-06-24-reset-dialog-and-room-numbers-design.md`.

## File structure
- **Modify `crates/app/src/state.rs`** — `reset_dialog: bool`, `reset_clear_map: bool`, `show_room_numbers: bool`; add `reset_dialog` to `any_overlay_open()`.
- **Modify `crates/app/src/main.rs`** — extract `reset_game(...)` helper from the `ConfirmReset` arm; open the dialog from the `ResetGame` action; route mouse/keyboard for the dialog; remove `PromptKind::ConfirmReset`; route slash `/reset` through `reset_game`.
- **Modify `crates/app/src/render/dialog.rs`** — add `ButtonId::Reset`.
- **Create `crates/app/src/render/reset_dialog.rs`** — `draw_reset_dialog(state) -> ResetDialogRects` (chrome + checkbox row + buttons) and its hit-rects.
- **Modify `crates/app/src/config.rs`** — `show_room_numbers: bool` (default false), resolve + write.
- **Modify `crates/app/src/keymap.rs`** — `Command::ToggleRoomNumbers` (kebab `toggle_room_numbers`) → `Action::ToggleRoomNumbers`.
- **Modify `crates/app/src/input.rs`** — `Action::ToggleRoomNumbers` flips `state.show_room_numbers`.
- **Modify `crates/app/src/render/map.rs`** — `draw_box_room`/`draw_portal_icons` honor `show_room_numbers`.
- **Modify `crates/app/src/render/config_screen.rs`** — a bool toggle row for `show_room_numbers`.

---

### Task 1: Shared `reset_game` helper + reset-dialog state

**Files:** Modify `crates/app/src/main.rs`, `crates/app/src/state.rs`.

**Interfaces — Produces:**
- `AppState.reset_dialog: bool` (default false), `AppState.reset_clear_map: bool` (default false); `any_overlay_open()` returns true when `reset_dialog`.
- In `main.rs`: `fn reset_game(session: &mut app::session::GameSession, mapper: &mut Mapper, state: &mut AppState, story_bytes: &[u8], clear_map: bool)` — rebuilds the session from `story_bytes` (same logic as today's `ConfirmReset` arm: reset turns/input/suggestions/transcript+transcript_kinds, push banner, re-seed + select start room); when `clear_map` is true, resets the accumulated map to empty BEFORE re-seeding (the same effect `/reset map` produces today via `*mapper = Mapper::default()`).

- [ ] **Step 1: Write the failing test** (in `state.rs` tests for the overlay flag)
```rust
#[test]
fn reset_dialog_counts_as_overlay() {
    let mut s = AppState::default();
    assert!(!s.any_overlay_open());
    s.reset_dialog = true;
    assert!(s.any_overlay_open(), "reset_dialog open => any_overlay_open true");
}
```
- [ ] **Step 2: Run, confirm fail** (`reset_dialog` field missing).
- [ ] **Step 3: Implement** the three `AppState` fields (defaults false) and add `|| self.reset_dialog` to `any_overlay_open()`. Then extract `reset_game(...)` in `main.rs`: move the body of the `PromptKind::ConfirmReset` confirmed-branch (`handle_saves_prompt`) into the free function, parameterized by `clear_map`. Read the current `ConfirmReset` arm first; preserve every step (session rebuild, `state.turns=0`, input/suggestions clear, `state.transcript.clear()` AND `state.transcript_kinds.clear()`, push banner, `apply_turn` re-seed, `select_room`). For `clear_map`: clear the map before the re-seed so only the start room remains (mirror what the slash `/reset map` path does today — find it and reuse the identical mechanism). Leave the OLD `ConfirmReset` arm calling `reset_game(.., clear_map=false)` for now (Task 3 removes the prompt).
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(reset): shared reset_game helper + reset-dialog state".

---

### Task 2: Render the reset dialog (chrome + checkbox)

**Files:** Create `crates/app/src/render/reset_dialog.rs`; Modify `crates/app/src/render/dialog.rs` (add `ButtonId::Reset`), `crates/app/src/render/mod.rs` (or `lib.rs`) to declare the module.

**Interfaces — Consumes:** `state.reset_dialog`, `state.reset_clear_map`, `state.colors.dialog*`. **Produces:**
- `ButtonId::Reset` added to the enum in `dialog.rs`.
- `pub struct ResetDialogRects { pub area: Rect, pub close: Option<Rect>, pub checkbox: Rect, pub reset: Option<Rect>, pub cancel: Option<Rect> }`.
- `pub fn draw_reset_dialog(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<ResetDialogRects>` — returns `None` when `!state.reset_dialog` or the area is too small. Builds a `DialogSpec` (title `"Reset game?"`, `show_close: true`, buttons `[Reset, Cancel]`), calls `draw_dialog`, then draws into `content`: a one/two-line body ("Restart the story from the beginning."), a blank row, and a checkbox row `[x] Also clear the map` / `[ ] Also clear the map` reflecting `state.reset_clear_map`. The checkbox row's drawn span is returned as `checkbox`. Map the `draw_dialog` button rects (`ButtonId::Reset`/`Cancel`) and `close` into the returned struct.

- [ ] **Step 1: Write the failing test** (TestBackend)
```rust
#[test]
fn reset_dialog_renders_title_checkbox_and_buttons() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut state = crate::state::AppState::default();
    state.reset_dialog = true;
    state.reset_clear_map = false;
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut rects = None;
    terminal.draw(|f| { rects = draw_reset_dialog(&state, f.area(), f.buffer_mut()); }).unwrap();
    let r = rects.expect("dialog should render when reset_dialog is set");
    assert!(r.close.is_some() && r.reset.is_some() && r.cancel.is_some());
    let all: String = terminal.backend().buffer().content().iter()
        .flat_map(|c| c.symbol().chars()).collect();
    assert!(all.contains("Reset game?"), "title present");
    assert!(all.contains("Also clear the map"), "checkbox label present");
    assert!(all.contains("[ ]"), "unchecked box shown when reset_clear_map is false");
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `ButtonId::Reset`, `ResetDialogRects`, and `draw_reset_dialog` (read `render/gallery.rs`'s use of `draw_dialog` as the reference for building the spec/style from `state.colors`). Declare `pub mod reset_dialog;`.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(reset): reset dialog render with checkbox + Reset/Cancel".

---

### Task 3: Wire the dialog — open, input, remove the text prompt

**Files:** Modify `crates/app/src/main.rs`.

**Interfaces — Consumes:** `draw_reset_dialog`/`ResetDialogRects`, `reset_game`, `state.reset_dialog`/`reset_clear_map`. **Produces:** `ResetGame` opens the dialog; the dialog handles mouse + keyboard; `PromptKind::ConfirmReset` removed; slash `/reset` routes through `reset_game`.

- [ ] **Step 1: Write the failing test** (a pure helper for the keyboard decision so the run loop stays out of the test)
```rust
// in main.rs:
// enum ResetDialogAction { None, ToggleClear, Confirm, Cancel }
// fn reset_dialog_key(code: KeyCode) -> ResetDialogAction { ... }
#[test]
fn reset_dialog_key_mapping() {
    use crossterm::event::KeyCode;
    assert!(matches!(reset_dialog_key(KeyCode::Esc), ResetDialogAction::Cancel));
    assert!(matches!(reset_dialog_key(KeyCode::Char('c')), ResetDialogAction::Cancel));
    assert!(matches!(reset_dialog_key(KeyCode::Enter), ResetDialogAction::Confirm));
    assert!(matches!(reset_dialog_key(KeyCode::Char('r')), ResetDialogAction::Confirm));
    assert!(matches!(reset_dialog_key(KeyCode::Char(' ')), ResetDialogAction::ToggleClear));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement:**
  - Change the `ResetGame` action handling so it sets `state.reset_dialog = true; state.reset_clear_map = false;` instead of starting the `ConfirmReset` prompt.
  - In the render path, after other overlays, call `draw_reset_dialog(&state, area, buf)` and stash the returned rects for hit-testing (same pattern other modals use).
  - In the event loop, when `state.reset_dialog`: route keys via `reset_dialog_key` (Confirm → `reset_game(.., state.reset_clear_map)` then `state.reset_dialog=false`; Cancel → `state.reset_dialog=false`; ToggleClear → flip `state.reset_clear_map`); route mouse clicks against the rects (checkbox → toggle, Reset → confirm, Cancel/[X] → close); swallow clicks outside the dialog (centered modal).
  - Remove `PromptKind::ConfirmReset` and its arms in `handle_saves_prompt` and the prompt render (the dialog replaces it).
  - Route the slash `SlashOutcome::Reset { map }` path through `reset_game(.., clear_map = map)` instead of its inlined copy (dedup; behavior identical).
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(reset): open dialog from ResetGame, mouse/key handling, drop text prompt".

---

### Task 4: Room-number config + state + toggle command

**Files:** Modify `crates/app/src/config.rs`, `crates/app/src/state.rs`, `crates/app/src/keymap.rs`, `crates/app/src/input.rs`.

**Interfaces — Produces:** `Config.show_room_numbers: bool` (default false); `AppState.show_room_numbers: bool` (seeded from config); `Command::ToggleRoomNumbers` (kebab `toggle_room_numbers`) → `Action::ToggleRoomNumbers`; the action flips `state.show_room_numbers`.

- [ ] **Step 1: Write the failing test**
```rust
// config.rs tests
#[test]
fn config_show_room_numbers_default_false_and_round_trips() {
    assert_eq!(Config::default().show_room_numbers, false);
    let cfg: Config = toml::from_str("show_room_numbers = true\n").unwrap();
    assert_eq!(cfg.show_room_numbers, true);
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the config field (default false; read in `Config::resolve`; write via the format-preserving writer — follow an existing bool flag like `use_default_map`/`auto_save`). Add `AppState.show_room_numbers` seeded from config where the runtime config is applied. Add `Command::ToggleRoomNumbers` to the enum + `to_action()` + `name()` (`"toggle_room_numbers"`) + the short hint label + `ALL_COMMANDS`, and `Action::ToggleRoomNumbers` whose handler does `state.show_room_numbers = !state.show_room_numbers;`. (Mirror `ToggleAlignment` end-to-end.)
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(rooms): show_room_numbers config + state + toggle command".

---

### Task 5: Render — hide `#id`, move portal icons to the freed row

**Files:** Modify `crates/app/src/render/map.rs`, `crates/app/src/render/config_screen.rs`.

**Interfaces — Consumes:** `state.show_room_numbers`. **Produces:** in `draw_box_room`, the `#id` row-3 text is drawn only when `show_room_numbers`; `draw_portal_icons` places icons on the far-right interior column when numbers are shown, and horizontally along interior row 3 (centered, clipped to the 9-wide interior) when hidden. A `show_room_numbers` toggle row appears in the F2 config screen.

- [ ] **Step 1: Write the failing test** (TestBackend on a Boxes-zoom room)
```rust
// Build a one-room RenderMap at Boxes zoom; render twice.
// (Construct from the existing map-render test helpers — read the nearby tests first.)
#[test]
fn room_number_visibility_toggles_id_and_icon_placement() {
    // with show_room_numbers = false: the "#<id>" text is ABSENT from the room cells,
    //   and a portal icon appears on the bottom interior row.
    // with show_room_numbers = true: "#<id>" appears on interior row 3,
    //   and the portal icon appears on the far-right interior column.
    // Assert the buffer contents for both cases.
}
```
(Turn this into a concrete test using the existing map-render test scaffolding in `map.rs` — match how `renders_current_room_highlighted_into_buffer` builds its scene.)
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the `show_room_numbers` branch in `draw_box_room` (skip the `#id` row-3 draw when false) and the placement branch in `draw_portal_icons` (thread `show_room_numbers` through its call at the `render_map` site; bottom-row horizontal layout when hidden, right-column when shown). Add the config-screen bool row (mirror an existing toggle row).
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(rooms): hide #id by default, relocate portal icons to freed row".

---

## Self-Review

**Spec coverage:**
- Reset dialog with checkbox + Reset/Cancel + Esc/[X] → Tasks 2, 3. ✅
- Default unchecked = map kept; checked clears map; shared helper with `/reset` → Tasks 1, 3. ✅
- `reset_dialog` in `any_overlay_open`; remove `ConfirmReset` → Tasks 1, 3. ✅
- `show_room_numbers` default-false config + runtime flag + toggle command → Task 4. ✅
- Hide `#id`, move portal icons to bottom row when hidden → Task 5. ✅
- Config-screen row → Task 5. ✅

**Placeholder scan:** Task 1 and Task 5 say "read the current arm / nearby tests first" — concrete pointers to existing code the implementer mirrors, not vague directives; the behavior is pinned by the listed steps and tests. Task 5's Step 1 is a sketch the implementer concretizes against the existing `map.rs` render-test scaffolding (named: `renders_current_room_highlighted_into_buffer`).

**Type consistency:** `reset_dialog`/`reset_clear_map`/`show_room_numbers` (AppState), `reset_game`, `ResetDialogRects`/`draw_reset_dialog`, `ButtonId::Reset`, `reset_dialog_key`/`ResetDialogAction`, `Command::ToggleRoomNumbers`/`Action::ToggleRoomNumbers`/`"toggle_room_numbers"`, `Config.show_room_numbers` — consistent across tasks.

## Notes for the executor
- Tasks 1–3 (reset dialog) and Tasks 4–5 (room numbers) are independent; either pair can land first. Within each pair, order matters (state/logic → render → wire).
- The trickiest step is Task 1's `reset_game` extraction — keep it behavior-identical to the current `ConfirmReset` arm; the only new behavior is the `clear_map` branch (already proven by the existing `/reset map`).
