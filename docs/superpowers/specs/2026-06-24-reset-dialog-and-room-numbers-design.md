# Reset Dialog + Room-Number Visibility — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO items:** #50 ("When using reset from the command palette a dialog should be shown that allows the user to optionally reset the map; /reset command should support [map] as an option") and #51 ("Hide room numbers by default; when hidden move portal icons to the bottom row of the room; create a command to turn room numbers on/off").
**Depends on:** the shared dialog chrome (`render/dialog.rs`, merged) and the symbol/render system (merged).
**Sequencing prereq:** both the in-flight **slash-commands** wave and **gallery-options** wave must be merged to `main` BEFORE this is implemented. This spec edits `main.rs`, `config.rs`, and `state.rs`, which the slash wave also edits — implementing in parallel would conflict. (The gallery wave is file-disjoint but is queued ahead.)

These are two independent features bundled into one spec (the user bundled them); each gets its own self-contained plan task(s) and could ship separately.

---

## Feature A — Reset confirmation dialog (#50)

### Current behavior
`ResetGame` (F5 / hotkey dialog) sets a `PromptKind::ConfirmReset` text sub-mode that renders a `Reset game? (y/n)` line; `handle_saves_prompt` (`main.rs:1293`) reads `y/yes` and rebuilds the session from the original story bytes, **keeping the map untouched**. The typed slash `/reset` / `/reset map` (slash wave) dispatch immediately and do NOT use this prompt.

### Target behavior
Replace the `ConfirmReset` text prompt with a proper modal using the shared dialog chrome. The dialog offers a **checkbox** to also clear the accumulated map.

- **Layout** (rendered via `draw_dialog`):
  ```
  ┌─ Reset game? ──────────────[X]┐
  │ Restart the story from the    │
  │ beginning.                    │
  │                               │
  │ [ ] Also clear the map        │
  │                               │
  │        [Reset]   [Cancel]     │
  └───────────────────────────────┘
  ```
- **Checkbox:** a clickable row rendered `[x] Also clear the map` / `[ ] Also clear the map`. Default **unchecked** (map kept — the safe default matching today's behavior). Clicking the row toggles it. This adds a small reusable **checkbox element** to the dialog chrome (a clickable hit-rect + a bool); it is the first checkbox in the chrome, so the implementation defines the minimal element (render string + hit-rect in `DialogRects`). Keyboard Tab-navigation between the checkbox and buttons is **out of scope** (it remains the existing separate "TAB navigation between buttons" TODO); the checkbox is mouse-toggle for now, plus `r`/`Enter` = Reset and `Esc`/`[X]`/`c` = Cancel as accelerators.
- **Buttons:** `[Reset]` (confirm) and `[Cancel]`. `Esc` and `[X]` both Cancel (uniform with all other modals).
- **On Reset:** run the existing session-rebuild logic (currently in `handle_saves_prompt`'s `ConfirmReset` arm — rebuild `GameSession` from `story_bytes`, reset turns/input/transcript, push banner, re-seed + select the start room). **If "Also clear the map" is checked**, clear the accumulated map FIRST so that after re-seeding only the starting room remains: reset the mapper's graph to an empty `MapGraph` (e.g. `state.graph = MapGraph::new()` / the mapper's clear path) before the `apply_turn` re-seed. When unchecked, the map is kept exactly as today.
- **On Cancel:** close the dialog, no state change (no `[Reset cancelled]` transcript spam is required, but a quiet status line is acceptable).

### Trigger routing
- **`ResetGame` action** (F5 / hotkey-dialog palette) → opens this dialog.
- **Typed `/reset` and `/reset map`** (slash wave) → act **immediately**, NO dialog (the user typed explicit intent). `/reset map` reuses the same map-clear path as the checkbox=checked case. The slash wave already wires `SlashOutcome::Reset{map}` to the reset handler; this feature factors the "rebuild session (+ optionally clear map)" logic into one shared helper that BOTH the dialog confirm and the slash path call, so the map-clear semantics are identical.

### State
- `AppState.reset_dialog: bool` (open/closed) and `AppState.reset_clear_map: bool` (checkbox). Add `reset_dialog` to `any_overlay_open()`. Remove the `PromptKind::ConfirmReset` variant and its prompt-mode handling (no backward compat needed).

### Out of scope (Feature A)
- Migrating the OTHER text prompts (Save-As, Rename) to the dialog chrome — those stay as-is (separate TODO).
- Tab/arrow keyboard navigation among dialog controls.

---

## Feature B — Room numbers hidden by default (#51)

### Current behavior
At `Boxes` zoom, `draw_box_room` (`map.rs:1397`) draws the room **name** word-wrapped on interior rows 1–2 and `#{id}` centered on interior **row 3** (`map.rs:1440`), with alignment diagnostics appended when `show_alignment` is on. Portal icons are drawn by `draw_portal_icons` on the **far-right interior column** (`icon_col = BOX_W-2`). `Compact`/`Overview` zoom show no id, so this feature is Boxes-zoom-only.

### Target behavior
- **New config setting `show_room_numbers: bool`, default `false`** (hidden). Lives in `config.toml` as a display setting (behavioral, not a visual style — so NOT in `style.toml`), alongside the other display flags. Persisted and round-tripped by the format-preserving config writer.
- **Runtime state `AppState.show_room_numbers: bool`**, initialized from config on load.
- **New toggle command:** a `keymap::Command` (kebab `toggle-room-numbers`) mapping to a new `Action::ToggleRoomNumbers`, available in the hotkey dialog and (via the slash fallback) as `/toggle-room-numbers`. Toggling flips `state.show_room_numbers` at runtime (does not rewrite config unless the user saves via the config screen).
- **Render change** in `draw_box_room` + `draw_portal_icons`:
  - When `show_room_numbers == true`: **current behavior** — `#id` on row 3, portal icons on the right interior column.
  - When `show_room_numbers == false`: **omit** the `#id` row-3 text entirely; **move the portal icons to interior row 3** (the freed bottom row), laid out horizontally (centered within the 9-wide interior). The right interior column is left clear for the room name.
  - `draw_portal_icons` takes `show_room_numbers` (or an enum placement param) to choose right-column vs bottom-row placement.
- **Alignment diagnostics interaction:** the `show_alignment` debug append rides on the `#id` row. When numbers are hidden, that row is used for icons, so the alignment code is not shown; a user debugging alignment turns room numbers back on. (No new behavior; just documented.)

### State / config
- `config.rs`: `show_room_numbers: bool` (default false), read in `Config::resolve` from the file, written by the format-preserving writer, surfaced as a row in the F2 config screen (a bool toggle row consistent with the other display flags).
- `state.rs`: `AppState.show_room_numbers: bool`, seeded from config.

### Out of scope (Feature B)
- Changing room numbers at Compact/Overview zoom (none shown there).
- Any change to how `#id` is used in the room inspector / diagnostics overlays (those still show the id textually).

---

## Testing

**Feature A:**
- A pure helper (e.g. `reset_game(session, mapper, state, story_bytes, clear_map: bool)`) is unit-testable: after `reset_game(..., clear_map=false)` the map graph room count is unchanged (start room present); after `clear_map=true` the graph contains only the re-seeded start room. (Rebuild-from-bytes may need a tiny story fixture; reuse an existing test fixture.)
- Dialog render (TestBackend): with `reset_dialog=true`, the buffer contains the title, the checkbox glyph reflecting `reset_clear_map`, and the `[Reset]`/`[Cancel]` button labels; the close `[X]` hit-rect is present.
- `any_overlay_open()` returns true when `reset_dialog` is set.
- Mouse: clicking the checkbox row toggles `reset_clear_map`; clicking `[Reset]`/`[Cancel]`/`[X]` does the right thing.

**Feature B:**
- `Config::default().show_room_numbers == false`; a `show_room_numbers = true` TOML round-trips.
- Render (TestBackend) of a Boxes-zoom room: with `show_room_numbers=false`, the `#id` text is absent and a portal glyph appears on the bottom interior row; with `true`, `#id` appears on row 3 and the icon appears on the right column.
- `Action::ToggleRoomNumbers` flips `state.show_room_numbers`.

---

## Risks & limitations (accepted)
- **Checkbox is the first dialog control of its kind** — kept intentionally minimal (mouse toggle + accelerators), no general focus/Tab system. If a later feature needs full keyboard control traversal, that's the existing separate TODO.
- **Bottom-row icon layout** must fit the 9-wide interior; with many portal senses on one room, icons are clipped to the interior width (documented, same clipping discipline as the existing right-column path).
- **Map-clear is destructive** within a run, but it's gated behind an explicit checkbox (default off) and the typed `/reset map`; the default reset keeps the map.

## Sources
- Current reset flow: `crates/app/src/main.rs:1280–1334` (`handle_saves_prompt` / `ConfirmReset`).
- Room render: `crates/app/src/render/map.rs:1397` (`draw_box_room`), `:1165` (`draw_portal_icons`).
- Dialog chrome: `crates/app/src/render/dialog.rs` (`draw_dialog`, `DialogRects`, `ButtonId`).
- Overlay pattern: `crates/app/src/state.rs:613` (`any_overlay_open`).
