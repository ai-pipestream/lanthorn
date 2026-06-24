# Task 5 Report: New Style Selectors + ColorScheme Fields + BorderSpec

## STATUS: COMPLETE

## Commit SHA
7fdd6945

## cargo test result
`test result: ok. 467 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` (app lib + integration); full workspace: all test suites green.

## Zero new warnings confirmation
`cargo build --workspace` produces no warnings.

## Changes Made

### crates/app/src/render/paneframe.rs
- Added `border_style_name(style: BorderStyle) -> &'static str` helper (inverse of `parse_border_style`).

### crates/app/src/colors.rs
- Added `use crate::render::paneframe::BorderStyle;`
- Added 7 new `Style` fields to `ColorScheme`: `map_border`, `story_border`, `story_title`, `map_layer_tab`, `map_layer_tab_active`, `status_header`, `input_line`.
- Added 2 `BorderStyle` fields: `map_border_style`, `story_border_style`.
- `terminal_default()`: new Style fields default to sensible colors (cyan borders, white text, etc.); `*_border_style` default to `BorderStyle::None`.
- `from_ghostty()`: new fields use palette[6] for borders/active-tab (matching existing `focused_border` color), fg for titles/tabs, default None for border styles.

### crates/app/src/style.rs
- Added `use crate::render::paneframe;`
- Extended `Decl` with `style: Option<String>` (serde default = None; ignored unless selector is a border selector).
- Updated `merge_decl` to merge the `style` field (over wins if set).
- Updated `parse_decl_from_table` to read the `style` key from TOML inline tables.
- Updated `style_to_decl` to set `style: None` (border callers override explicitly).
- Added 7 selectors to `SELECTOR_FIELDS`: `map_border`, `story_border`, `story_title`, `map_layer_tab`, `map_layer_tab_active`, `status_header`, `input_line`.
- Updated `apply_color_decls`: `map_border`/`story_border` also set `cs.map_border_style`/`cs.story_border_style` via `paneframe::parse_border_style` when the `style` key is present; remaining 5 new selectors patch their `ColorScheme` Style fields.
- Updated `DEFAULT_STYLE_TOML`: added `"map_border" = { style = "picture-frame" }` and `"story_border" = { style = "picture-frame" }`.
- Updated `write_style` to emit the `style` key from `Decl` (as the first key in the inline table for clarity).
- Updated `write_style_full`: emits all 7 new selectors; for `map_border`/`story_border` sets `d.style = Some(border_style_name(...))` so the round-trip is exact.

## How the per-selector `style` key was captured without breaking existing color selectors
The `style` field was added to `Decl` with `#[serde(default)]` so all existing color selectors that lack a `style` key deserialize correctly with `style: None`. The `parse_decl_from_table` function reads `style` from the TOML table but returns `None` if absent. The `apply_color_decls` match arm only reads `decl.style` for the `map_border`/`story_border` cases — other selectors ignore it entirely. Existing merge/round-trip tests all still pass because `style: None` in `style_to_decl` means non-border selectors emit no `style` key in TOML output, and the TOML reader silently ignores absent keys.

## write_style_full round-trip fidelity
Yes. `write_style_full` explicitly sets `d.style = Some(border_style_name(cs.*_border_style))` for `map_border`/`story_border` before inserting the decl. On re-parse, `parse_decl_from_table` reads it back, `apply_color_decls` calls `paneframe::parse_border_style`, and the `BorderStyle` is reproduced exactly. The existing `write_style_full_is_self_contained` test now also covers the new fields (it compares full `cs2 == cs`).

## Concerns
None. The `resolve_empty_doc_equals_terminal_default` test uses an empty `StyleDoc` (no selectors), so the new fields default to `terminal_default()` values and the equality check passes. The `DEFAULT_STYLE_TOML` sets `BorderStyle::PictureFrame` via selectors, which is only applied when that TOML is parsed — not for empty docs.
