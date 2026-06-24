# Task 8 Report: Status-header + Input-line Boxing + Default/Opt-out Snapshots

## STATUS: COMPLETE

## Commit SHA
`520d2152`

## Cargo Test Result
`test result: ok. 471 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.69s` (app crate)
Full workspace: all test suites green (11 suites, 0 failures).

## Zero New Warnings Confirmation
`cargo build --workspace` emits zero warnings. The build was warning-clean before and remains so after.

## Gap Fix: Two New BorderStyle Fields

### Fields Added to ColorScheme (colors.rs)
- `pub status_header_style: BorderStyle` — default `BorderStyle::None` in both `terminal_default()` and `from_ghostty()`
- `pub input_line_style: BorderStyle` — default `BorderStyle::None` in both constructors

### Wiring in apply_color_decls (style.rs)
The `status_header` and `input_line` selectors in `apply_color_decls` were extended: when the parsed `Decl` carries a `style` key, `parse_border_style` is called and the result is stored in `cs.status_header_style` / `cs.input_line_style` respectively. This mirrors the existing pattern for `map_border_style` and `story_border_style`.

### write_style_full (style.rs)
`write_style_full` now emits the `style` key for `status_header` and `input_line` selectors when their border style is non-None (matches the map/story pattern).

## How Status/Input Boxing Is Drawn (transcript.rs)

`render_transcript` was refactored into three helpers:

1. `render_status_content` — fills the status region with the reversed-video style and draws location+score/time text
2. `render_input_content` — draws the `> input` prompt and cursor
3. `render_middle` — draws inventory strip, suggestion line, and scrolling transcript body

In `render_transcript`:
- If `status_header_style != None` AND `area.height >= 5`: draws a 3-row box around the status region via `draw_pane_frame(buf, status_region, status_style_kind, cs.status_header)`, then renders the status content into `frame.content` (the inner 1-row rect). Otherwise: renders the status content directly onto the 1-row status region (plain reversed bar, unchanged behavior).
- Same pattern for the input line using `input_line_style` and `cs.input_line`.
- The middle area (transcript + inventory + suggestions) shrinks to fit between the (possibly expanded) status and input regions.

## Default Renders Plain; None Opts Out

- `ColorScheme::terminal_default()` sets `status_header_style = None` and `input_line_style = None`.
- `DEFAULT_STYLE_TOML` does not set `status_header` or `input_line` selectors, so the default resolves to `None` for both.
- With `None`, `render_transcript` renders the status bar exactly as before (plain reversed bar) and the input line exactly as before (plain `> ` prompt).
- `draw_pane_frame` with `BorderStyle::None` returns `content == area` with no border glyphs drawn — this is the existing opt-out path for map/story panes as well.

## Three New Tests

All in `crates/app/src/render/transcript.rs`:

1. **`status_header_plain_by_default_boxed_when_styled`**: Verifies that with `status_header_style = None` the top row has REVERSED modifier and no corner glyphs; with `status_header_style = Single` the top-left is `┌`, the content row (col 1, row 1) has REVERSED, and the bottom-left is `└`.

2. **`input_line_plain_by_default`**: Verifies that with `input_line_style = None` the bottom row contains `> go north` and no corner glyphs appear in the bottom 3 rows.

3. **`panes_none_reproduce_plain_borderless`**: Calls `draw_pane_frame` directly with `BorderStyle::None` for both a simulated map and story area; asserts `content == area` and that no border glyphs appear in the buffer (the opt-out path for pane borders).

## Concerns
None. The refactoring preserves all 468 pre-existing tests and adds 3 new ones. The default render path is unchanged (None border style follows the original code paths exactly).
