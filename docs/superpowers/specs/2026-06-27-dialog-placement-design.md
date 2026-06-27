# Dialog Placement (Global, Style-Configurable) — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Sequencing:** touches `render/dialog.rs`, the modal render files, `style.rs`,
`colors.rs`. Implement AFTER the animation-engine wave (shared `render/`),
to avoid conflicts.

## Goal

Let `style.toml` control where dialogs/overlays appear — centered (today's
default), anchored to an edge or corner, with a margin — instead of always
centering. Designed so the planned dialog open/close **animation** plugs into the
same seam with no further modal changes.

## Background (current code)

- `render/dialog.rs`: `Placement::Centered { w, h }` (→ `centered_rect`) or
  `Placement::Positioned(Rect)` (absolute). `draw_dialog(buf, spec, ds)` maps the
  placement to a rect (dialog.rs:96).
- ~14 modals build a `DialogStyle { frame, box_style, glyphs, title, button,
  button_active, shadow, shadow_on }` **byte-for-byte identically** from
  `state.colors.dialog_*`, then pass `Placement::Centered { w, h }` (each computes
  its own content `w`/`h`). Two panels — room **inspector** and **room-info** —
  use `Positioned` because they anchor to a specific room (contextual).
- Dialog styling already resolves from `style.toml`'s `dialog` selector + non-color
  properties (`dialog_box_style`, `dialog_shadow_on`) into `ColorScheme`.

## Design

### 1. Centralize the dialog chrome (the future-proofing seam)

Add `DialogStyle::from_colors(cs: &ColorScheme) -> DialogStyle` and replace the
~14 identical inline `DialogStyle { … }` builds with `DialogStyle::from_colors(&state.colors)`.
This is a net code reduction and creates the single place where cross-cutting
dialog concerns (placement now, animation later) live.

### 2. Placement schema (style.toml)

Two optional keys on the `dialog` selector's `Decl`:

```toml
"dialog" = { style = "single", placement = "bottom", margin = 2 }
```

- `placement` token → `DialogPlacement` enum: `Center` (default) · `Top` ·
  `Bottom` · `Left` · `Right` · `TopLeft` · `TopRight` · `BottomLeft` ·
  `BottomRight`. Tokens: `center`, `top`, `bottom`, `left`, `right`, `top-left`,
  `top-right`, `bottom-left`, `bottom-right`. Unknown → `Center`.
- `margin: u16` (default `0`) = cells of gap from the anchored edge(s); ignored for
  `Center`. This is the TODO's "explicit offset" (offset from edge); absolute x,y
  coordinates are out of scope.

`Decl` gains `placement: Option<String>` and `margin: Option<u16>` (read only for
the `dialog` selector, like `style`/`header`/`shadow` are). `ColorScheme` gains
`dialog_placement: DialogPlacement` (default `Center`) and `dialog_margin: u16`
(default `0`), resolved from that `Decl`. No keys → `Center`/`0` → exactly today's
behavior.

### 3. Resolution + application

- Pure helper: `resolve_dialog_rect(placement: DialogPlacement, margin: u16,
  w: u16, h: u16, area: Rect) -> Rect`:
  - `Center` → `centered_rect(area, w, h)` (unchanged).
  - `Top`/`Bottom` → horizontally centered; `y = area.y + margin` /
    `area.bottom() - h - margin`.
  - `Left`/`Right` → vertically centered; `x = area.x + margin` /
    `area.right() - w - margin`.
  - Corners → both axes at the respective edges with `margin`.
  - Result clamped to stay within `area` (never off-screen).
- `from_colors` sets `placement`/`margin` on the `DialogStyle`.
- `draw_dialog`: the `Placement::Centered { w, h }` arm becomes
  `resolve_dialog_rect(ds.placement, ds.margin, w, h, buf_area)`. `Positioned`
  panels are unchanged (contextual, bypass placement).
- `write_style_full` emits `placement`/`margin` on the `dialog` selector when
  non-default.

### 4. How animation plugs into this seam later (informative, NOT this spec)

The dialog open/close effect will:
- reuse `resolve_dialog_rect(...)` as the animation **target** rect;
- add an optional `anim` hook to the centralized path (set in `from_colors` / from
  run-loop state) and, in `draw_dialog`, draw at `lerp(start_rect → target)` over
  the engine `Tween` (start = off-screen edge for slide, zero-size for grow).

Because chrome construction + rect resolution are centralized here, that work
touches only `from_colors` + `draw_dialog` + run-loop state — never the modals.

## Testing

- `resolve_dialog_rect`: `Center` equals `centered_rect` (behavior preserved); each
  edge/corner places the rect at the expected coordinates with `margin`; results
  clamp within `area`. Non-vacuous: assert exact x/y.
- `DialogStyle::from_colors` produces a value equal to the previous inline build
  for a known `ColorScheme` (a guard proving the refactor is behavior-preserving),
  and carries the resolved `placement`/`margin`.
- `Decl` parses `placement`/`margin`; `ColorScheme` resolves `dialog_placement`/
  `dialog_margin` (default `Center`/`0` when absent).
- `write_style_full` round-trips a non-default `placement`/`margin` on the `dialog`
  selector.

## Out of scope

- Per-dialog placement (the modals share one `dialog` selector; future work).
- Absolute x,y coordinates.
- Placement of the contextual `Positioned` panels (inspector, room-info).
- The animation itself — this spec only leaves the seam ready.

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- Default (no `placement`/`margin` keys) must reproduce today's centered behavior
  exactly; the `from_colors` refactor must be behavior-preserving.
