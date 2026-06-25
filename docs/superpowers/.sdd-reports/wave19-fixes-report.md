# wave19 Final-Review Fixes Report

## STATUS: COMPLETE

All 4 fixes applied, build clean, all tests green (506 app + 13 + 3 + 159 + 1 + 153 + 2 = 837 total across workspace), zero new warnings.

## Fix 1 — centered modal swallows outside-clicks even with corner overlay open

**Change in is_corner_overlay (applied in BOTH branches):**

Left-click branch (~line 634) and non-left-click/wheel branch (~line 683) now compute:
```rust
let centered_open = state.gallery.is_some() || state.config_screen.is_some()
    || state.saves.is_some() || state.file_browser.is_some()
    || state.verb_menu.is_some() || state.hotkey_dialog;
let is_corner_overlay = !centered_open
    && (state.room_panel.is_some() || state.tidy_anim.is_some());
```

**New regression test** (at end of test module in input.rs):
`centered_modal_swallows_outside_clicks_even_with_room_panel_open` -- sets up a real map_rect, room_rects_for_compact(1, (0,0), map_r), confirms without dialog the click produces ShowRoomInfo(1), then opens both room_panel and gallery simultaneously with a dialog rect that does not cover (0,0), and asserts the outside click returns Action::None.

## Fix 2 — ESC closes hotkey dialog

Added `if key.code == KeyCode::Esc { return Action::CloseHotkeyDialog; }` at the top of `hotkey_dialog_key_to_action`, before prefix/lookup_any handling.

Extended `esc_equals_x_click_for_every_modal` test: section 6 now asserts `key_to_action(&s, key(KeyCode::Esc))` returns `CloseHotkeyDialog` in addition to the existing [X] click assertion.

## Fix 3 — shadow-bool style round-trip test

Added `write_style_full_round_trips_dialog_shadow_and_box_style` in style.rs tests: sets `dialog_shadow_on = true` and `dialog_box_style = Double`, writes with `write_style_full`, re-parses with `parse_style_toml`, resolves, and asserts both fields survive.

## Fix 4 — consistent box_style coercion across all modals

Approach taken: **coerce inside draw_dialog, remove from make_dialog_style**.

The None->Single coercion was moved from `main.rs::make_dialog_style` into `render/dialog.rs::draw_dialog` (at the top, before area resolution). Now ALL 9 modals coerce identically via the single code path in `draw_dialog`. No modal draw functions were touched.

Files changed: `crates/app/src/input.rs`, `crates/app/src/style.rs`, `crates/app/src/main.rs`, `crates/app/src/render/dialog.rs`.
