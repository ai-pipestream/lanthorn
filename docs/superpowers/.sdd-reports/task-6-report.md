# Task 6 Report: Map pane border + centered layer tabs (wiring)

## STATUS: COMPLETE

## Commit SHA
f88fc190

## cargo test result
`cargo test --workspace`: 11 test suites, all `ok`. 0 failed. No warnings.
Total tests: 468 (app lib) + 13 (app bin) + 3 + 159 + 1 + 153 + 2 = 799 passing.

## Zero-new-warnings confirmation
`cargo build -p app 2>&1` and `cargo test --workspace 2>&1` produce zero warnings.
The `layer_tabs` field is annotated with `#[allow(dead_code)]` because it is populated
but not yet consumed (mouse click wiring deferred per plan).

## LayerId type and layer accessors
- `LayerId = u16` (from `mapper::layer` in `crates/mapper/src/layer.rs`, line 8)
- Layer list: `graph.layers().keys().copied().collect::<Vec<LayerId>>()` where
  `graph.layers()` returns `&BTreeMap<LayerId, LayerMeta>` (sorted ascending by id)
- Active layer: `state.active_layer(graph)` (defined in `state.rs`; follows
  `state.viewed_layer` if set and valid, else current room's layer, else `MAIN_LAYER = 0`)
- The tidy-animation case uses the animation frame's graph for layer queries (same
  pattern as the pre-existing `render_layer` call)

## Files changed
- `crates/app/src/render/paneframe.rs`: Added `LayerSegment { text: String, active: bool }`,
  `build_layer_segments(layers: &[LayerId], active_layer: LayerId) -> Vec<LayerSegment>`,
  `LayerSegment::as_inset()` convenience converter. Test `layer_tab_segments_mark_active`
  added VERBATIM from plan.
- `crates/app/src/main.rs`:
  - Added imports for `build_layer_segments`, `draw_pane_frame`, `draw_top_inset`,
    and `mapper::layer::LayerId`
  - `PaneRects` gains `#[allow(dead_code)] layer_tabs: Vec<(LayerId, Rect)>`
  - `draw_frame` populates `layer_tabs_out` from tab hit-rects zipped with layer ids
  - MapFull and Split layout branches: map border now rendered via `draw_pane_frame`
    (using `cs.map_border_style` / `cs.map_border`); map content rendered into
    `frame.content`; layer tabs overlaid via `draw_top_inset` into `frame.top_inset`
  - Pulsing tidy border: reimplemented as a direct border-cell style overlay (the old
    `map_pane` closure was removed since `Block` is no longer used for the map pane)
  - TestBackend test `map_pane_default_shows_picture_frame_corner` added: resolves
    DEFAULT_STYLE_TOML, calls `draw_pane_frame`, asserts top-left cell is `┏`

## Mouse hit-test consumers of PaneRects
`PaneRects` is a private struct used only in `main.rs`. The only consumer of its fields
is the event loop, which reads `last_panes.map`, `last_panes.story`, and
`last_panes.room_rects`. The `layer_tabs` field is not yet wired to mouse handling
(deferred per plan). No external consumers were updated.

## Concerns / notes
- The `map_border_style` default in `ColorScheme::terminal_default()` is `BorderStyle::None`
  (for backward compat when no style file is loaded). Picture-frame is the default only when
  DEFAULT_STYLE_TOML is applied (startup does this). The TestBackend test therefore uses
  `parse_style_toml(DEFAULT_STYLE_TOML)` + `resolve()` to get the picture-frame scheme,
  rather than `AppState::default()` which gives `terminal_default()`.
- The pulse-border overlay for background tidy jobs is reimplemented as a direct cell-style
  pass over the outer border row/column cells. This is functionally equivalent to the old
  `map_pane` Block approach but works with the new `draw_pane_frame` border.
- `TranscriptFull` layout has no map pane and was not changed (correct).
- The story pane in Split and TranscriptFull still uses the ratatui `Block` pattern
  (Task 7 will replace it with `draw_pane_frame`).
