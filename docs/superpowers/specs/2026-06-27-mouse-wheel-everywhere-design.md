# Mouse-Wheel Scrolling on Every Scrollable Surface — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Sequencing:** edits `input.rs` (`mouse_to_action`) — a hot file shared with the
keyboard-navigation wave. Implement AFTER that wave merges, to avoid conflicts.

## Goal

The mouse wheel should scroll/navigate every surface that can scroll, not just
the map and transcript. Today wheel works on the map (pan/zoom), the transcript
(scroll), and the startup story picker (selection), but the in-game modal lists
(saves, file browser, gallery, replay/history, verb menu), the style-editor
selector list, and the hints panel ignore it.

## Background (current code)

- `mouse_to_action` (input.rs ~900-980) maps wheel events: `ScrollUp/Down if
  in_map` → pan/zoom; `ScrollUp/Down if in_story` → `TranscriptScroll(±1)`.
  Anything else → `Action::None`.
- `mouse_wheel_invert` is handled UPSTREAM: a normalize step (input.rs ~907-908)
  swaps ScrollUp/ScrollDown before `mouse_to_action` runs when the config flag is
  set. So `mouse_to_action` must NOT invert again.
- Each list modal already has an Up/Down navigation action: `StyleNav(i32)`,
  `SavesNav(i32)`, `FbNav(i32)`, `GalleryPrev`/`GalleryNext`,
  `VerbMenuNav(VerbMenuNavKind::Up|Down)`, and the replay/history modal's
  nav (via `history_key_to_action`).
- The startup story picker is a separate full-screen loop (main.rs ~820) that
  already moves its selection on the wheel — the model to mirror.
- The hints panel is a companion mini-terminal; confirm whether it has a scroll
  offset, and add a minimal one if absent.

## Design

### Rule

When an overlay/modal is open, the wheel drives THAT surface's vertical
navigation, taking precedence over the underlying map/story:

- wheel up → previous / up (toward the first item or older content)
- wheel down → next / down (toward the last item or newer content)

`mouse_wheel_invert` is already applied upstream, so map raw `ScrollUp` → up and
`ScrollDown` → down; do not invert again.

### Per-surface wheel mapping

| Open surface | Wheel up | Wheel down |
|---|---|---|
| Style-editor selector list | `StyleNav(-1)` | `StyleNav(1)` |
| Saves manager | `SavesNav(-1)` | `SavesNav(1)` |
| File browser | `FbNav(-1)` | `FbNav(1)` |
| Gallery | `GalleryPrev` | `GalleryNext` |
| Verb menu | `VerbMenuNav(Up)` | `VerbMenuNav(Down)` |
| Replay / history | history nav up | history nav down |
| Hints panel | scroll companion transcript up | scroll down |

Base surfaces unchanged: map (pan/zoom), transcript (`TranscriptScroll`), startup
picker (already wheel-driven).

### Mechanism

Add a single modal-open precedence branch to `mouse_to_action`, BEFORE the
existing `in_map`/`in_story` wheel arms: if an overlay is open (reuse the existing
overlay-open detection — e.g. `any_overlay_open(state)` or the per-modal `Option`
state fields), match the open modal to its wheel-nav action from the table above;
otherwise fall through to today's map/story behavior. One small arm per modal; the
list modals need no new scroll state (they reuse their Up/Down nav).

For the hints panel: if it lacks a transcript scroll offset, add a clamped
`hints_scroll` offset to its state and a `HintsScroll(i32)` action the wheel
drives (mirroring `TranscriptScroll`); the panel render applies the offset.

### Decisions baked in

- Wheel moves the **selection** in list modals (not a separate view scroll) —
  consistent with the existing story picker and simplest, since these modals have
  no separate view-scroll model.
- One item/line per wheel tick — matches the current transcript/picker cadence.
- `mouse_wheel_invert` honored (via the existing upstream swap).

## Testing

- For each modal, a `mouse_to_action` unit test: with that modal open, a
  `ScrollUp` event yields the surface's up/prev action and `ScrollDown` yields its
  down/next action. Non-vacuous: assert the exact action variant.
- An inversion test for one representative modal: with `mouse_wheel_invert` set,
  the produced actions swap (exercising the upstream normalize + the new arm).
- Precedence: with a modal open, a wheel event over the map/story area still
  drives the modal (not pan/scroll).
- Hints scroll (if added): `HintsScroll` clamps to `[0, max]`.

## Out of scope

- Rendering scrollbar widgets on the modal lists — this feature is wheel INPUT,
  not adding scrollbar UI; a separate enhancement.
- Smooth/animated scroll — deferred to the animation framework.
- Horizontal wheel beyond the map's existing `ScrollLeft/Right` pan.

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- Honor the existing `mouse_wheel_invert` config; do not double-invert.
