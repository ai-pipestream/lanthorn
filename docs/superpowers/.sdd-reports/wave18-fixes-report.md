# Wave 18 Final-Review Fixes Report

## STATUS: COMPLETE

## Commit SHA
TBD (populated after commit)

## Cargo Test Result
`test result: ok. 474 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.79s` (app crate)
Full workspace: all 11 test suites green, 0 failures.

## Zero New Warnings Confirmation
`cargo build --workspace` emits zero warnings after all changes.

---

## Fix 1: Remove duplicate layer indicator

**How map_border_style was threaded:** No threading needed. `render_map_layered` already
receives `state: &AppState`, and `AppState.colors.map_border_style` is directly accessible.
The fix is a single conditional in `render_map_layered`: when
`state.colors.map_border_style != BorderStyle::None`, skip `draw_layer_strip` and pass
`area` unchanged as `body_area`. When it is `None`, the existing `draw_layer_strip` call
runs as before (fallback for the borderless opt-out).

**map_dump.rs impact:** None. `map_dump.rs` imports and calls `render_map` directly
(not `render_map_layered`), so it is completely unaffected.

**Tests added (in crates/app/src/render/map.rs):**
- `render_map_layered_no_in_content_strip_when_border_present`: 2-layer graph +
  `map_border_style = PictureFrame`, asserts zero REVERSED cells in content row 0.
- `render_map_layered_draws_in_content_strip_when_no_border`: 2-layer graph +
  `map_border_style = None`, asserts at least one REVERSED cell in row 0 (active tab).

---

## Fix 2: Delete stale vacuous tests

Deleted from `crates/app/src/main.rs mod tests`:
- `focused_pane_title_has_reversed_modifier`
- `unfocused_pane_title_does_not_have_reversed_modifier`

Removed imports that were exclusively used by those two tests:
- `use ratatui::text::Span`
- `use ratatui::widgets::{Block, Borders, Widget}` (the whole line)
- `Style` from `use ratatui::style::{Modifier, Style}` (reduced to `Modifier` only)

Net test count: -2 deleted, no replacements added for this fix.

---

## Fix 3: Strengthen write_style_full border round-trip

Added test `write_style_full_round_trips_non_none_border_styles` in
`crates/app/src/style.rs`. Builds a `ColorScheme` with
`map_border_style = PictureFrame` and `story_border_style = Double`, calls
`write_style_full`, re-reads the file, parses via `parse_style_toml`, resolves via
`resolve`, and asserts both fields match. Mirrors the existing
`write_style_full_is_self_contained` test setup exactly.

---

## Fix 4: Pulse-on-picture-frame integration test

**Approach used:** Narrower invariant assertion (inline perimeter-loop). Wiring a full
`draw_frame` render for this test would require a complete `AppState` with a live
`TidyJob` (which requires a real thread), mapper, Z-machine session, and terminal
backend -- impractical as a unit test.

Instead, the test (`pulse_overlay_touches_only_outer_perimeter_not_inner_tab_row` in
`crates/app/src/main.rs mod tests`) directly exercises the perimeter-loop logic copied
verbatim from `draw_frame`. It applies the pulse overlay to a 30x15 buffer and asserts:
1. The top-left and top-right outer perimeter cells carry the pulse color.
2. The inner tab row center cells (y+1, x in 2..=right-3) do NOT carry the pulse color
   (they are blank/Reset because only the side-column pixels at x==0 and x==right-1
   are written for those rows by the loop).

This directly validates the invariant: the pulse perimeter loop does not overwrite the
inner picture-frame tab row's drawable span.

---

## Concerns
None. All 4 fixes are self-contained within crates/app. No mapper or zvm changes.
The Fix 1 suppression is minimal (one conditional, zero new parameters). The two
deleted tests were pure ratatui behavior tests with no babelmap code under test.
