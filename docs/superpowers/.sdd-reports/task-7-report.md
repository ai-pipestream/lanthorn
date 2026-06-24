# Task 7 Report: Story Pane Border + Centered Adventure Title

## STATUS: COMPLETE

## Commit SHA
d3848f81

## Cargo Test Result
`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out` (story pane test alone)
Full workspace: all tests pass (468 + 14 + 3 + 159 + 1 + 153 + 2 = 800+ tests), zero failures.

## Zero-New-Warnings Confirmation
`cargo build --workspace` completes with `Finished` and no warning lines. Warning-clean.

## How the Transcript Render Was Repointed Into frame.content

Both story-pane layout branches in draw_frame (TranscriptFull and Split) previously used the ratatui Block renderer via a pane() closure to draw the border and then called render_transcript with the Block's inner rect.

Task 7 replaced that approach:

1. draw_pane_frame(buf, story_area, cs.story_border_style, cs.story_border) draws the border and returns a PaneFrame{content, top_inset}.
2. render_transcript(&session.machine, state, story_frame.content, buf) renders the transcript content directly into frame.content (the rect inside the border), rather than the raw story area.
3. draw_top_inset(buf, story_frame.top_inset, &[InsetSegment{text: &state.title, active: false}], cs.story_title, cs.story_title) overlays the centered adventure title in the top border row.

The dim_area call in the Split branch was also updated from transcript_inner (the old variable name) to story_frame.content so that unfocused dimming still applies to the correct rect.

The now-unused pane() closure (and its focused_border variable, Block, Borders, Widget top-level imports) were removed. InsetSegment was added to the paneframe import line.

## Whether Status/Input Rows Still Fit

The status row, transcript body, suggestion line, inventory strip, and input line are all rendered by render_transcript into the content rect, which for picture-frame is inset by 2 on all sides (cols 2..=w-3, rows 2..=h-3). All internal layout in render_transcript is relative to the passed-in area rect, so the rows fit correctly inside the border. No content draws outside frame.content or over the border.

## Test Added

story_pane_shows_title_in_border_by_default in crates/app/src/main.rs tests module:
- Resolves the DEFAULT_STYLE_TOML ColorScheme (same path as startup).
- Draws the story pane frame and title overlay on a 40x15 TestBackend buffer.
- Asserts buf.cell((0,0)) == "┏" (picture-frame top-left corner).
- Asserts that row 1 (the picture-frame inner top border row) contains "ZORK I".

## Concerns

None. The implementation is straightforward. The only non-trivial change was removing the old pane() Block approach entirely; the focused-border styling (REVERSED title, colored border) was part of that old approach and is now superseded by the draw_pane_frame + draw_top_inset pipeline. Focus dimming in Split layout is preserved via dim_area on story_frame.content.
