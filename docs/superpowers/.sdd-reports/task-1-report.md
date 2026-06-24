# Task 1 Report: dialog.rs — centered_rect + draw_dialog + DialogRects

## STATUS: COMPLETE

## Commit SHA
(see below — commit made after this file was written)

## Test Result
cargo test --workspace: 477 passed (app crate) + all other crates green; 0 failed; 0 ignored.

Three new dialog tests all pass:
- dialog_opaque_bg_covers_underlying_and_records_rects
- centered_rect_centers_and_clamps
- dialog_shadow_paints_offset_cells_when_on

## Zero New Warnings
Confirmed: grep for "warning:" on full cargo test output returns empty. Workspace was warning-clean before and remains so.

## Implementation Notes
- Created crates/app/src/render/dialog.rs with all required types and draw_dialog.
- Added pub mod dialog; to crates/app/src/render/mod.rs.
- draw_dialog: resolves area via centered_rect or Placement::Positioned; paints shadow cells with clamped arithmetic (no overflow/panic); fills area opaque via Style::reset().patch(st.frame); calls draw_pane_frame for border; calls draw_top_inset for centered title; draws close X just inside top-right; draws right-aligned button row; returns DialogRects with content = frame content minus button row height.
- buf.area() returns &Rect in ratatui 0.29; dereferenced with * to pass by value to centered_rect.

## Concerns
None. All three verbatim test functions pass. No mapper/zvm changes made.
