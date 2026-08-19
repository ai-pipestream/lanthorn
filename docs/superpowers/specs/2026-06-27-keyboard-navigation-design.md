# Keyboard Navigation Completeness — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning

## Goal

Round out the app's keyboard navigation so the common expected behaviors all
work: reverse-cycling with Shift-Tab, full keyboard operation of dialog buttons,
PageUp/PageDown to read the story, and arrow-key recall of previous commands.
Four independent features.

## A. Shift-Tab reverse cycling

Today Tab (no modifiers) is intercepted in game focus: when the player is
mid-word and `state.suggestions` is non-empty, Tab → `Action::Autocomplete`
(apply the highlighted suggestion, advance to the next); otherwise Tab keeps its
`ToggleFocus` (next-pane) behavior (input.rs:435). Shift-Tab (`BackTab`) is the
symmetric pane-switch (`ToggleFocusBack`).

- Mirror the Tab intercept for `BackTab`: in game focus, mid-word with
  suggestions present, Shift-Tab → a new `Action::AutocompletePrev` that applies
  the highlighted suggestion and steps **backward** through `suggestions`
  (wrapping), the inverse of `Autocomplete`. Otherwise Shift-Tab keeps
  `ToggleFocusBack`.
- The `Autocomplete` handler (input.rs:1718) advances the suggestion index
  forward; `AutocompletePrev` decrements it (wrapping). Both apply the now-current
  suggestion to the input buffer the same way.
- General principle — wherever Tab cycles, Shift-Tab reverses:
  - Style editor: Tab is `StyleFocusCycle(1)` (input.rs:1163); add Shift-Tab →
    `StyleFocusCycle(-1)`.
  - Dialog buttons: handled uniformly in feature B.

## B. Dialog button keyboard navigation (uniform)

Some modals already cycle `state.dialog_focus` on Tab/BackTab and activate the
focused button on Enter (aux/launch dialogs, e.g. main.rs:1329). Make this
uniform across every shared-chrome modal:

- On open, `dialog_focus` is initialized to the dialog's default button index
  (the `DialogSpec.default` the modal already declares), so Enter without any Tab
  activates the sensible default.
- **Tab / Shift-Tab** move `dialog_focus` among the dialog's buttons, wrapping,
  via the existing `cycle_focus(focus, n_buttons, ±1)` helper. The shared chrome
  already renders the focused button (it is passed `focus: Some(state.dialog_focus)`).
- **Enter / Space** activate the focused button, producing the same `Action` a
  mouse click on that button yields. Each modal already has a button list (for
  rendering + mouse hit-testing) and a `ButtonId → Action` mapping (e.g.
  `style_dialog_action`); keyboard activation looks up `buttons[dialog_focus].id`
  and runs that mapping. Esc continues to cancel (equivalent to the close/cancel
  button).
- Single-button modals (Done/OK): Enter/Space activate the one button; Tab is a
  harmless wrap-to-self.
- Scope: every modal that renders via the shared `draw_dialog` chrome and exposes
  buttons. Modals with their own internal navigation that is NOT button-row based
  (e.g. the saves list, file browser, gallery grid, verb menu token panes) keep
  their existing item navigation; this feature governs the button ROW
  (Save/Cancel/Done/[X]) only. Where a modal has both (a list plus buttons), the
  button-row nav coexists with the list nav as it does today.

## C. PageUp/PageDown story scroll

In game focus today, `PageUp → Action::ZoomIn` and `PageDown → Action::ZoomOut`
(input.rs:1261-1262). Rebind them to scroll the story:

- `PageUp` → scroll the transcript up by one page; `PageDown` → down one page.
  A page is the transcript's visible row count from the last render, minus a
  1-line overlap for reading continuity, clamped to the existing
  `max_scroll`/0 bounds (the same clamp the wheel scroll uses).
- Implement as `Action::TranscriptScrollPage(i8)` resolved where the last-rendered
  transcript viewport height is known (the run loop already tracks the largest
  meaningful `transcript_scroll` per frame; expose the visible-rows count
  alongside it if not already available).
- Remove `ZoomIn`/`ZoomOut` from PageUp/PageDown. Zoom stays on `+`/`=`/`-`/`0`,
  Ctrl+wheel, and `/zoom-map`. (No other key currently maps to PageUp/PageDown.)

## D. Arrow-key command history (persisted)

There is no command-line input history today (`state.history` is the unrelated
turn-replay record). Add a shell-style recall:

- New `AppState.command_history: Vec<String>` plus an in-memory navigation cursor
  and a saved draft. Records **every** non-empty submitted line — game commands
  and slash commands alike (the decided scope).
- On `Action::SubmitCommand(cmd)` (handler reached via main.rs:2136 / input.rs):
  if `cmd.trim()` is non-empty, append it to `command_history`, **skipping** a
  push that equals the current last entry (dedupe consecutive repeats); cap the
  list at 500 (drop oldest). Reset the navigation cursor to "past the newest."
- In game focus, plain (no-modifier) **Up** recalls the previous (older) entry
  into the input buffer; **Down** recalls the next (newer). The first Up saves the
  current in-progress input as the draft; stepping Down past the newest entry
  restores that draft. At the oldest entry, further Up is a no-op (stays).
  Shift+Up / Shift+Down still pan the map (unchanged).
- Persistence: store the (capped) `command_history` as a new entry in the
  `.lanthorn` archive (e.g. `command_history.json`, a JSON array of strings),
  written whenever the archive is written and read on game load. A missing entry
  is tolerated (→ empty history). It is per-game (each game's archive).

## Testing

- **A:** Shift-Tab in game focus with suggestions → `AutocompletePrev`; without
  suggestions → `ToggleFocusBack`. `AutocompletePrev` steps the suggestion index
  backward with wrap and applies the suggestion. Style-editor Shift-Tab →
  `StyleFocusCycle(-1)`.
- **B:** a representative multi-button modal: Tab/Shift-Tab cycle `dialog_focus`
  with wrap; Enter on focus index *i* yields the same action as a mouse click on
  button *i*; `dialog_focus` initializes to the default button; Space behaves like
  Enter. A single-button modal: Enter activates it.
- **C:** `TranscriptScrollPage(+1)`/`(-1)` move `transcript_scroll` by
  (visible_rows − 1), clamped to `[0, max_scroll]`; PageUp/PageDown in game focus
  map to it and no longer zoom.
- **D:** submit dedupe + cap; Up/Down recall sequence including draft save/restore
  and oldest/newest boundaries; archive round-trip (`command_history` written and
  re-read; missing entry → empty); ALL submitted input (game + slash) is recorded.

## Out of scope

- **Animated smooth scroll** — deferred to the planned styleable animation
  framework, which should own easing/speed/enable uniformly (and would apply to
  page scroll and the mouse wheel). Note: a character-cell TUI can only animate at
  line granularity; true sub-cell/pixel smooth scroll is not achievable here.
- Mouse-wheel support on every scrollable window (a separate, mouse-focused TODO
  item).
- Re-binding any of these via the keymap config in non-default ways (the keymap
  system already allows user rebinding; this spec sets the defaults).

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- New default key behaviors must not require Ctrl (Shift is acceptable for the
  reverse-cycling and map-pan cases, consistent with the existing Shift-Tab /
  Shift-Arrow conventions).
