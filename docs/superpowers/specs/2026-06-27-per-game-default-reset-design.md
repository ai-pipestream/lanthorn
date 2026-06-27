# Per-Game "Default" Fields — Freeze via the `reset` Token

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Fixes:** the Phase 2.2 follow-up — per-game style files are not truly "frozen"
for fields explicitly set to terminal-default.

## Problem

Per-game style files are written self-contained (every selector, via
`write_style_full`) but loaded via `merge(global, per_game)`, where `merge_decl`
is field-level (`over.fg.clone().or(base.fg.clone())`). A terminal-default color
serializes as an **omitted** key, and under merge an omitted field means "inherit
global." So a per-game field the user explicitly set to terminal-default
re-inherits the global color on reload — the explicit "default" silently reverts.

### Root cause

"Default" is represented as the **absence** of a color, not a value:

- The editor's "default" swatch stores `value: None` (e.g. `main.rs:1994`,
  `input.rs` `StyleSwatchPick`, and `StyleCommitCustom` maps a typed `"default"`
  to `None`).
- `decl_to_style` (`style.rs:75`) only sets `Style.fg` when `parse_color_value`
  returns `Some`. `parse_color_value("default")` returns `None` (not a recognized
  token), so the resolved `Style.fg` is **unset**.
- In the resolved `ColorScheme`, three intents collapse to the same `fg = None`:
  "never specified," "explicitly default," and "unrecognized token." They are
  indistinguishable.
- `write_style_full` serializes `fg: s.fg.map(color_to_str)` → a `None` fg is
  omitted. The "explicitly default" intent is lost *before* serialization.

### The key asset already in the codebase

There is already an **active** terminal-default representation that round-trips:
the `reset` token ↔ `Color::Reset`.

- `parse_color_value("reset")` → `Some(Color::Reset)` (`colors.rs:751`).
- `color_to_str(Color::Reset)` → `"reset"` (`style.rs`).
- `Color::Reset` renders as an explicit "reset to terminal default" escape.

So `Color::Reset` survives save → load → merge, while `None` does not. The only
reason "default" leaks is that the editor stores `None` (passive) instead of
`Some("reset")` (active).

## Design

Represent the user's explicit "default" choice as the existing `reset` token so it
survives the round-trip and wins at merge. Keep the merge model (thin
hand-authored overlays still work).

### 1. Editor "default" selection stores `reset`

Every place the editor currently produces `value: None` for the "default"
selection instead produces `Some("reset".to_string())`:

- The fg swatch-grid "default" cell click (`main.rs`, the `i >= ANSI_NAMES.len()`
  branch in the fg swatch hit-test).
- The bg swatch-grid "default" cell click (`main.rs:1994` neighborhood, the bg
  branch).
- The keyboard `StyleSwatchPick` (`input.rs`): when the swatch cursor is on the
  "default" cell (index `== ANSI_NAMES.len()`), produce `Some("reset")` instead of
  the `ANSI_NAMES.get(cur)` → `None` fallthrough.
- The custom-field commit `StyleCommitCustom` (`input.rs`): a typed `"default"`
  maps to `Some("reset")` instead of `None`.

Effect: picking "default" sets `Decl.fg = Some("reset")` (resp. `bg`). On save,
`write_style_full` emits `fg = "reset"`; on reload, `merge_decl` yields
`Some("reset")` (per-game wins over a global color); `decl_to_style("reset")` →
`Color::Reset` → terminal default renders. The field is frozen.

### 2. Alias `"default"` → `Color::Reset` in `parse_color_value`

Add `"default"` as a synonym for `Color::Reset` in `parse_color_value`
(`colors.rs`), so a `"default"` token from a hand-edited file or any legacy path
resolves to an explicit terminal-default rather than silently unset. The editor
and serializer standardize on `"reset"` (because `color_to_str(Reset)` →
`"reset"`); `"default"` is accepted on input for friendliness. `"default"` is
already accepted by `is_valid_color_token`.

### 3. Swatch-grid "default" cell highlight recognizes `reset`

The property pane's swatch row highlights the cell matching the active value
(`fg_val`/`bg_val` from `active_decl.fg.unwrap_or("default")`). The "default" cell
(index `ANSI_NAMES.len()`) must be shown as the current selection when the value
is `"reset"` (the new stored form) as well as `"default"` or absent. Update the
default-cell match in `draw_swatch_row` so `"reset"` and `"default"` both map to
the default cell.

### What does NOT change

- The merge model and thin hand-authored per-game overlays — unchanged. A
  per-game file that simply omits a field still inherits global (that is the
  intended thin-overlay behavior). Only an *explicit* default now writes `reset`.
- The global style format's normal case: untouched terminal-default fields stay
  omitted (they resolve to `None`, serialized as omitted). Only fields the user
  explicitly set to default write `reset`. No blanket bloat.
- No new sentinel token, no on-disk migration. Existing files (omitted defaults)
  keep loading as before; `"reset"` and `"default"` both parse.

## Testing

- **Round-trip freeze (the core regression test):** build a global doc with
  `room.fg = white`; build a per-game doc with `room.fg = "reset"`;
  `merge(global, per_game)` then `resolve` yields a `room` style whose fg is
  `Color::Reset` (terminal default), NOT white. (Would fail today if the per-game
  field were omitted.)
- **Serializer:** a `ColorScheme` whose `room` fg is `Color::Reset` →
  `write_style_full` / `style_to_decl` emits `fg = "reset"` for `room` (not
  omitted). Re-parsing yields `Decl.fg == Some("reset")`.
- **`parse_color_value`:** `"default"` and `"reset"` both → `Some(Color::Reset)`.
- **Editor default selection:** dispatching the keyboard `StyleSwatchPick` on the
  default cell, and `StyleCommitCustom` with `custom_buf == "default"`, set the
  active selector's `Decl.fg` (resp. `bg`) to `Some("reset")` (not `None`). Use
  the hermetic `open_style_editor_hermetic` helper.
- **Swatch highlight:** with `Decl.fg = Some("reset")`, the rendered swatch row
  marks the "default" cell as the current selection (TestBackend buffer scrape or
  the existing swatch-rect assertions).

## Out of scope

- Standalone (non-merge) per-game loading — not needed; the `reset` token makes
  the merge model correct.
- A thin-vs-self-contained file marker — not needed.
- Rewriting existing on-disk files — no migration; both forms parse.

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- Reuse existing tokens (`reset`); no new style selector or sentinel.
