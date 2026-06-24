# Task 4 Report: Config screen adopts dialog chrome + button mouse routing

## STATUS: COMPLETE

## Commit SHA
(see below after commit)

## Exact cargo test result line
test result: ok. 483 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.88s

## Zero-new-warnings confirmation
Build produces no warnings: "Finished `dev` profile" with no warning lines.

## Per-modal button->action mapping structure

The mapping is implemented as a small per-modal helper function in input.rs:

    fn config_dialog_action(rects: &DialogRects, col: u16, row: u16) -> Option<Action>

It checks the close rect (->ConfigCancel) and button rects (Save->ConfigSave, Cancel->ConfigCancel). The caller in mouse_to_action branches on `state.config_screen.is_some()` to select which helper to use. Later modals add their own analogous helper function (e.g. `saves_dialog_action`, `gallery_dialog_action`) and a matching `state.saves.is_some()` branch.

This keeps mapping extensible without a dispatch table, while staying minimal. A closure or trait object could generalize it further if 8 modals become unwieldy, but 2-3 parallel match arms are the simplest thing that works.

## mouse_to_action call sites updated
- 1 site in main.rs (the production event loop)
- 13 sites in input.rs test module (all existing tests updated to pass &None)

Total: 14 call sites updated.

## Changes made

### render/config_screen.rs
- Refactored draw_config_screen to use draw_dialog (DialogSpec with Save/Cancel buttons, show_close:true, Placement::Centered)
- Returns Option<DialogRects> instead of ()
- Draws config rows into returned content rect
- Added draw_config_screen_shows_chrome render test

### main.rs
- Added `use app::render::dialog::DialogRects`
- Added `dialog: Option<DialogRects>` field to PaneRects
- Populated dialog_rects_out from draw_config_screen return value
- Updated last_panes initialization to include dialog: None
- Updated mouse_to_action call to pass &last_panes.dialog

### input.rs
- Added config_dialog_action() helper mapping close/Save/Cancel rects to Actions
- Updated mouse_to_action signature: added `dialog: &Option<DialogRects>` param
- When dialog is Some: hit-tests close/buttons first, swallows all other clicks
- Updated 13 existing test call sites to pass &None
- Added config_dialog_button_clicks_map_to_actions test
- Added config_esc_maps_to_config_cancel test (ESC was already routing to ConfigCancel via config_screen_key_to_action; confirmed no change needed)

## ESC alignment
ESC was already mapped to ConfigCancel in config_screen_key_to_action. No change needed.

## Concerns
None. The implementation is straightforward. The only subtlety was that with BorderStyle::None (the terminal_default), the dialog title is drawn at the same y-coord as content.y, so the render test sets dialog_box_style=Single to get the title on the border row (separate from content). The production behavior with the style.toml default ("single") is correct.
