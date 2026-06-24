# Queued briefs — small leaves (L10 shift-tab, L22 reset)

Two tiny features, briefed (not full specs). Each has ONE micro-decision flagged for confirmation at dispatch. Both touch `input.rs`/`keymap.rs`/`state.rs`/`main.rs`, so they queue serial with the other loop/input work.

---

## L10 — Shift-Tab cycles windows backward

Today: `Tab` → `ToggleFocus` (Game ↔ Map focus, 2 states); `CycleLayout` (`Ctrl+L`) cycles the 3 layouts Split → TranscriptFull → MapFull → Split (`state.cycle_layout()`).

**DECISION (confirm at dispatch):** what does "windows … in backwards order" cycle?
- **(Recommended) Reverse the 3-state LAYOUT cycle** — `Shift+Tab` → a new `Command::CycleLayoutReverse` (MapFull → TranscriptFull → Split → MapFull). "Backwards order" is only meaningful for the 3-state layout; this is the useful interpretation.
- Alternative: reverse FOCUS — with only 2 focus states this is identical to `Tab`, so it adds nothing.

**Implementation (layout interpretation):**
- `state.rs`: add `cycle_layout_reverse()` (the inverse of `cycle_layout()`).
- `keymap.rs`: `Command::CycleLayoutReverse` (+ `to_action`/`name`/`label`/`context`=Map or Global; default binding `Shift+Tab`). Note: `Shift+Tab` arrives as `KeyCode::BackTab` in crossterm — bind to that.
- `input.rs`: `Action::CycleLayoutReverse` → `state.cycle_layout_reverse()`.
- TEST: `cycle_layout_reverse` from Split → MapFull → TranscriptFull → Split; `BackTab` → `Action::CycleLayoutReverse`.

---

## L22 — Reset game and start over (with confirmation)

Re-initialize the Z-machine from the original story bytes, back to the opening state, behind a confirmation prompt.

**DECISION (confirm at dispatch):** the MAP on reset?
- **(Recommended) Keep the accumulated map** — reset only the game; your explored map stays (it is your knowledge). Player returns to the start room.
- Alternative: clear the map too for a fully fresh run (`Mapper::default()`).

**Implementation:**
- `main.rs` keeps the original `story_bytes` (used at startup) — reuse them to rebuild the session: `session = GameSession::new(story_bytes.clone())?`, then re-seed the starting room (the same opening `apply_turn("", &seed_result)` the startup path does). Reset `state.turns = 0`, clear the input/transcript as the startup does, and (if keeping the map) leave `mapper` untouched; else `mapper = Mapper::default()`.
- Confirmation: reuse the prompt sub-mode — `Command::ResetGame` → `Action::ResetGame` opens a `PromptKind::ConfirmReset` prompt ("Reset game? (y/n)"); on a `y`/Enter confirm, perform the reset; Esc/other cancels. Mirror the saves-manager `ConfirmDeleteSave` flow.
- `keymap.rs`: `Command::ResetGame` (+ default key; dialog group). `state.rs`: `PromptKind::ConfirmReset`.
- TEST: `ResetGame` opens the confirm prompt; confirming rebuilds the session (turn counter 0, current room is the start); the prompt machinery routes y/Enter vs Esc. (A fixture-backed test with minizork.z3 can assert the post-reset current location equals the opening room.)

---

## Footprint (both)
`keymap.rs`, `input.rs`, `state.rs`, `main.rs`. zvm/mapper used as-is. Do them in one small track once the loop/input files are free. Confirm the two flagged decisions before dispatch.
