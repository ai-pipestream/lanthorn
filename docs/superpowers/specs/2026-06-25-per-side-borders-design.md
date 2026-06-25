# Per-Side Pane Borders + Header Decoupling — Design

**Date:** 2026-06-25
**Status:** Draft, pending user review
**TODO:** #82 ("specify each border top/bottom/left/right independently"), the last border slice of the UI-styling theme.

## Goal

Let each pane border specify its four sides (top/bottom/left/right) independently
— enabling left/right-only, top-only, and mixed-glyph frames — and decouple the
title / layer-tab **header strip** from the top border so a pane can show a title
with no border (and vice-versa).

## Background (current state)

- `render/paneframe.rs` owns the border model. `BorderStyle` =
  `None | Single | Double | Thick | PictureFrame`.
  `draw_pane_frame(buf, area, style, color) -> PaneFrame` draws all four sides +
  corners as one whole-frame style and returns `PaneFrame { area, content,
  top_inset }`. `content` is `area` inset by 1 on every side (2 for
  picture-frame); `top_inset` is the top border row where titles / layer-tabs are
  drawn via `draw_top_inset`.
- `PictureFrame` is a composited look (a ramped lower-block top, thin side-blocks,
  and a nested inner single-line frame) — inherently a full four-side frame.
- The title / layer-tab strip is drawn into `top_inset`, which only exists when a
  top border is drawn. The map's layer-tabs are functional (clickable layer
  switching), not just decoration.
- Style selectors that route through `draw_pane_frame`: `map_border`,
  `story_border`, `status_header`, `input_line`, `upper_window_border`. Each takes
  one `style = "<name>"` in `style.toml`, resolved onto a single `*_style:
  BorderStyle` field on `ColorScheme`. Dialogs (`dialog`) use the same
  `draw_pane_frame` but are modal boxes.
- `Decl` (style.rs) carries the per-selector properties; border selectors read its
  `style` field. `write_style_full` exports each border selector's `style` name so
  a self-contained file round-trips.

## Design

### 1. Per-side specification

Each border selector gains four optional per-side keys plus the existing base
`style`:

```toml
# left/right only (base off, sides on)
"map_border"  = { style = "none", style_left = "single", style_right = "single", fg = "cyan" }

# mixed glyphs (base single, heavier top)
"story_border" = { style = "single", style_top = "thick" }

# top only
"input_line"  = { style = "none", style_top = "single" }
```

- A side's **effective style** = its `style_<side>` override if set, else the base
  `style`. `"none"` omits that side.
- Per-side values are limited to `none | single | double | thick`. A per-side value
  of `"picture-frame"` is invalid for a single side → **warns** and falls back to
  the base style.
- The base `style` keeps its current meaning and default; with no per-side keys a
  selector behaves exactly as today.

### 2. Picture-frame interaction

`picture-frame` is **whole-frame only**. When the base `style = "picture-frame"`,
all per-side overrides are **ignored** and the full composited frame draws as
today. (Per-side keys are line-style only; picture-frame's ramp/inner-frame
compositing does not decompose into independent sides.)

### 3. Corner rendering

A corner cell is resolved from its two adjacent sides:

- **Both sides present** → the corner glyph (`┌/┐/└/┘`). When the two sides differ
  in style, the **heavier wins**: `thick > double > single`.
- **Only the horizontal side present** (top or bottom) → that side's horizontal
  glyph (`─/━/═`) extends into the corner.
- **Only the vertical side present** (left or right) → that side's vertical glyph
  (`│/┃/║`) extends into the corner.
- **Neither present** → blank.

This yields clean partials: left+right only → two full-height bars, no corners;
top only → one full-width line; an L-shape (top+left) → a single `┌` corner.

### 4. Content inset

`content` is inset by 1 **only on sides that have a border** (0 on omitted sides),
so the content area grows into any open edge. Picture-frame still insets 2 on all
sides (whole frame). This is computed from the resolved per-side presence.

### 5. Header decoupling + `header` switch

The title / layer-tab strip is decoupled from the top border line and gated by a
new per-pane boolean key **`header`** (default `true`). Placement:

| `header` | top side | Result |
|----------|----------|--------|
| `true`   | present  | Strip in the top border row, flanked by border glyphs (today's look). `top_inset` = the border row. |
| `true`   | `none`   | Strip on a plain top **content** row (title / tabs, no border line); content starts one row below. |
| `false`  | present  | No strip; the top border row is a plain border line. Content unchanged. |
| `false`  | `none`   | No strip, no top border; the top row is content (fully clean pane). |

- `header` applies to the panes that have a header today: **`story_border`** (the
  adventure title) and **`map_border`** (the layer-tab strip). For other border
  selectors `header` is accepted but inert.
- When `header = true` and the top side is `none`, the pane renderer reserves the
  first inner row for the strip (content shrinks by one row) and draws the strip
  there with no border glyphs.

### 6. Scope

Per-side keys + `header` apply to the pane-frame selectors: **`map_border`,
`story_border`, `status_header`, `input_line`, `upper_window_border`**. Dialogs
(`dialog`) remain whole-frame (modal boxes; per-side adds no value and the
drop-shadow/button chrome assumes a full frame).

## Architecture / components

- `crates/app/src/render/paneframe.rs`:
  - New `pub struct PaneSides { pub top: BorderStyle, pub bottom: BorderStyle,
    pub left: BorderStyle, pub right: BorderStyle }` (derives `Debug, Clone, Copy,
    PartialEq, Eq`), where each side is a line style or `None`. A `PaneSides::all(s)`
    constructor fills all four with one style.
  - New `pub fn draw_pane_frame_sides(buf, area, sides: PaneSides, color) ->
    PaneFrame` — draws each present side, resolves corners (§3), computes `content`
    inset per present side (§4), and sets `top_inset` to the top row only when the
    top side is present (else a zero-height rect).
  - A corner-glyph helper `fn corner_glyph(h: BorderStyle, v: BorderStyle, which:
    Corner) -> &'static str` implementing §3 (heavier-wins).
  - `draw_pane_frame(buf, area, style, color)` is kept as a thin wrapper: for
    `PictureFrame` it takes the existing composited path; otherwise it delegates to
    `draw_pane_frame_sides(PaneSides::all(style), …)`. All current callers keep
    working unchanged.
- `crates/app/src/colors.rs`:
  - For each of the five pane selectors, add a resolved `*_sides: PaneSides` field
    (keep the existing `*_style: BorderStyle` as the base — picture-frame still
    rides on it). Add `story_header_on: bool` and `map_header_on: bool` (default
    `true`). `PaneSides`/`bool` are `PartialEq`/`Clone`, so `ColorScheme` keeps its
    derive. Both constructors default `*_sides = PaneSides::all(*_style)` and headers
    `true`.
- `crates/app/src/style.rs`:
  - `Decl` gains `style_top/style_bottom/style_left/style_right: Option<String>` and
    `header: Option<bool>` (only meaningful for border selectors, like `style`/
    `shadow`). `parse_decl_from_table` and the `StyleColors` deserialize read them.
  - `apply_color_decls`: for each border selector, resolve the base style, then
    compute `PaneSides` (each side = override-or-base, `parse_border_style`, with the
    picture-frame-per-side warning), set the pane's `*_sides`, and set the header
    bool from `decl.header` for story/map. Picture-frame base → `*_sides` left at
    `all(PictureFrame)` and the render path uses the composited frame.
  - `write_style_full`: export each border selector's base `style` (as today) plus
    `style_<side>` for any side that differs from the base, and `header = false`
    when off — so the file round-trips losslessly.
- Render integration:
  - The pane drawers choose the path by base style: `PictureFrame` → composited;
    else → `draw_pane_frame_sides(cs.*_sides, …)`.
  - `render/transcript.rs` (story pane title; status_header; input_line) and the
    map pane renderer apply the §5 header placement: draw the strip into
    `frame.top_inset` when the top side is present and `header_on`; when `header_on`
    and the top side is absent, reserve the first content row and draw the strip
    there; when `header_off`, draw no strip.

## Error handling

- Unknown per-side value → `parse_border_style` (unknown → `single`, matching the
  existing base behavior).
- `style_<side> = "picture-frame"` → warning, side falls back to the base style.
- A pane too small to draw a requested side degrades gracefully (the existing
  `draw_pane_frame` size-guards apply per side; an omitted side never draws).
- `header` on a non-header selector (e.g. `input_line`) parses but is inert.

## Testing

- Per-side resolution: `style = "none"` + `style_left/right = "single"` →
  `PaneSides { left: Single, right: Single, top/bottom: None }`; mixed
  `style = "single"` + `style_top = "thick"` → top Thick, rest Single.
- Picture-frame: base `picture-frame` ignores per-side overrides (sides stay the
  composited path); per-side `"picture-frame"` warns and uses the base.
- Corner glyphs: both-present picks heavier (thick+single → thick corner);
  horizontal-only extends `─`; vertical-only extends `│`; neither → blank.
- Render: left+right-only frame draws full-height side bars and no corners;
  top-only draws one full-width line; content inset reflects present sides (open
  edge reclaims a column/row).
- Header matrix (§5): the four `header` × top-side combinations place the strip /
  reserve a row / leave it clean as specified; `header = false` hides the title /
  layer-tabs; map layer-tabs render on a borderless top row when `header = true`
  and top is `none`.
- Round-trip: `write_style_full` exports per-side overrides + `header`, and a
  `ColorScheme` with a mixed per-side frame and `header = false` re-parses and
  resolves to an equal `ColorScheme`. The existing whole-frame round-trips stay
  green (uniform sides export just the base `style`).
- Back-compat: a selector with only `style = "<name>"` resolves to `PaneSides::all`
  and renders identically to today; default `ColorScheme` is unchanged.

## Out of scope (deferred)

- Per-side **color** (each side a different fg) — one `fg` per pane border, as
  today.
- Per-side borders for **dialogs** (whole-frame only).
- Picture-frame with dropped sides (whole-frame only).
- Independent placement of the title vs. the layer-tabs within one strip, or moving
  the header to the bottom — the strip stays a top-row feature.
