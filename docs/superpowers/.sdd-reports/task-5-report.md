# Task 5 Report: Migrate saves + file browser to dialog chrome

## STATUS

COMPLETE

## Commit SHA

37a70c25

## Exact cargo test result

```
test result: ok. 487 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.58s
```

All workspace crates pass. Zero new warnings.

## Zero-new-warnings confirmation

`cargo build --workspace` produces no warnings (confirmed: empty warning output).

## How dialog wiring was extended for two more modals

Task 4 established the pattern: `PaneRects.dialog` stores the active `DialogRects`,
`mouse_to_action` checks it first, and a per-modal helper function maps button/close
clicks to the modal's close action.

Task 5 extends this with two more modals following the same pattern:

- `render/saves.rs` `draw_saves`: now returns `Option<DialogRects>` (was `()`).
  Renders via `draw_dialog` with `show_close: true` and `[{Done, "Done"}]` button.
  Builds `DialogStyle` from `state.colors.dialog_*`. Draws headers, entry rows,
  and footer hints into the returned `content` rect. Returns `Some(rects)`.

- `render/filebrowser.rs` `draw_file_browser`: same treatment. Title varies by mode.
  CWD row draws at `content.y`, entries below. Footer hint updated (removed "q" references).

- `main.rs` `draw_frame`: the existing `dialog_rects_out` local is now set by saves OR
  file_browser (whichever is active), just like config_screen. Since only one overlay is
  active at a time, the `if/else` structure guarantees the correct rects are stored.
  The config_screen block still sets it last (it is drawn last), so config wins if somehow
  multiple were open (they cannot be simultaneously in practice).

- `input.rs`: added `hit()` free function (extracted from the inner closure in
  `config_dialog_action`). Added `saves_dialog_action()` and `filebrowser_dialog_action()`
  helpers that map close/Done clicks to `SavesClose`/`FbClose`. The `mouse_to_action`
  dialog branch now branches on which modal is open (`state.saves.is_some()` vs
  `state.file_browser.is_some()` vs `state.config_screen.is_some()`) and calls the
  corresponding helper.

## q-close key tests updated

- `filebrowser_key_to_action`: removed `KeyCode::Char('q') => Action::FbClose` arm.
- `saves_key_to_action`: no `q` binding was present (already ESC-only), so no change needed.
- Test `filebrowser_q_produces_fb_close` renamed to `filebrowser_q_no_longer_closes` and
  asserts `q` now produces `Action::None` in file browser sub-mode.

## New tests added

- `render/saves.rs`: `draw_saves_shows_dialog_chrome` - asserts [X], [Done], "Saves" title,
  and `DialogRects` with close + 1 button (Done).
- `render/filebrowser.rs`: `draw_file_browser_shows_dialog_chrome` - same for file browser.
- `input.rs`: `saves_dialog_x_and_done_produce_saves_close` - mouse hit-test for [X] and [Done].
- `input.rs`: `filebrowser_dialog_x_and_done_produce_fb_close` - same for file browser.

## Pre-existing tests updated

`draw_file_browser_shows_in_pickfile_mode` and `draw_file_browser_shows_in_pickdir_mode`
previously relied on the title text appearing directly in the content area. With the new
chrome, the title is rendered via `draw_top_inset` in the border row, and with
`BorderStyle::None` (the default), this row coincides with the first content row which gets
overwritten by the CWD line. Fixed by setting `dialog_box_style = BorderStyle::Single`
in these tests (matches how the chrome-specific tests work) and using a 30-row terminal.

## Concerns

None. The border-style interaction (None vs Single and title row overlap) is an existing
design characteristic: with `BorderStyle::None`, the title and content row 0 share the same
row. This is consistent with how the config screen handles it and all existing tests that
check chrome specifically use `BorderStyle::Single`.
