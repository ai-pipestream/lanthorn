# Task 6 Report: Migrate verb menu + hotkey dialog

## STATUS: COMPLETE

## Commit SHA

TBD (committed below)

## Cargo test result

`test result: ok. 493 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`

## Zero-new-warnings confirmation

`cargo build --workspace` produces 0 warnings.
`cargo clippy -p app --lib` shows 33 warnings (identical count to pre-task baseline).
No new warnings were introduced.

## Files changed

- `crates/app/src/render/verbmenu.rs`: Migrated `draw_verb_menu` to return `Option<DialogRects>` via `draw_dialog` (centered, `show_close:true`, `[Done]`). Colors from `state.colors.dialog_*`. Column layout (verb/noun/prep) drawn into returned `content` rect. Helper functions `draw_pane_header` and `draw_list` updated to accept `state` for dialog colors.
- `crates/app/src/render/hotkeys.rs`: Migrated `draw_hotkey_dialog` to return `Option<DialogRects>` via `draw_dialog` (centered, `show_close:true`, `[Done]`). Removed `draw_border`/`draw_char` local functions (replaced by `draw_pane_frame` via `draw_dialog`). Colors from `state.colors.dialog_*`. The opaque `Style::reset()` fill from `draw_dialog` fixes the current-room-color bleed (issue #17).
- `crates/app/src/main.rs`: Updated `draw_frame` to capture `dialog_rects_out` from both `draw_verb_menu` and `draw_hotkey_dialog`. Updated verb menu help-bar hint to remove `q` reference ("Esc/q: close" -> "Esc: close").
- `crates/app/src/input.rs`: Added `verbmenu_dialog_action` (close/Done -> `VerbMenuClose`) and `hotkeys_dialog_action` (close/Done -> `CloseHotkeyDialog`). Extended the `mouse_to_action` dialog branch to include verb_menu and hotkey_dialog modals. Removed `q`-as-close from `verb_menu_key_to_action` and `hotkey_dialog_key_to_action`.

## Bleed test passes

`render::hotkeys::tests::draw_hotkey_dialog_bg_opaque_over_map_cell` — confirms the dialog opaque background clears the REVERSED modifier and Red background from a pre-filled map cell at the center of the hotkey dialog area.

## q-close tests updated

- `verb_menu_esc_and_q_close` RENAMED to `verb_menu_esc_closes` (asserts only Esc closes)
- NEW test `verb_menu_q_no_longer_closes` asserts `q` produces `Action::None`
- `q_closes_hotkey_dialog_action` RENAMED to `q_no_longer_closes_hotkey_dialog` (asserts q does NOT produce CloseHotkeyDialog)
- Inline assertion in the large map-focus integration test updated from `assert!(matches!(...CloseHotkeyDialog))` to `assert!(!matches!(...CloseHotkeyDialog))`

## New tests added

- `render::verbmenu::tests::verb_menu_shows_dialog_chrome` - verifies title, [X], [Done], DialogRects returned
- `render::hotkeys::tests::draw_hotkey_dialog_shows_dialog_chrome` - verifies [X], [Done], DialogRects returned
- `render::hotkeys::tests::draw_hotkey_dialog_bg_opaque_over_map_cell` - verifies bleed fix (#17)
- `input::tests::verbmenu_dialog_x_and_done_produce_verb_menu_close` - mouse [X]/[Done] -> VerbMenuClose
- `input::tests::hotkey_dialog_x_and_done_produce_close_hotkey_dialog` - mouse [X]/[Done] -> CloseHotkeyDialog
- `input::tests::verb_menu_q_no_longer_closes` - q is not close
- `input::tests::q_no_longer_closes_hotkey_dialog` - q is not close

## Notes on dialog_box_style default

The default `ColorScheme::dialog_box_style` is `BorderStyle::None`. Tests that check for visible title text (which requires a Single or similar border to avoid content overwriting the title row) explicitly set `state.colors.dialog_box_style = BorderStyle::Single`. This matches the pattern established by `draw_saves_shows_dialog_chrome`.

## Concerns

None. The implementation follows the exact pattern from Tasks 4-5 (saves/filebrowser). The bleed fix is structural -- `draw_dialog` does `Style::reset().patch(...)` over the entire dialog area, which clears any pre-existing bg/REVERSED modifier. The verb menu now also goes through the same opaque fill path.
