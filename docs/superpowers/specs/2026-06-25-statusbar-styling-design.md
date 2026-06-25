# Status Bar Styling — Simplified tmux-Style Segment Bar — Design

**Date:** 2026-06-25
**Status:** Draft, pending user review
**TODO:** #76 ("Stylize score bar"), the status-bar slice of the UI-styling theme. (#82 per-side borders is a separate sub-design.)

## Goal

Replace the fixed reversed-video status line (one `statusbar` style for the
whole row) with a **configurable, tmux-style segment bar**: a frame rule plus an
ordered list of content segments, each with its own text template, placement,
and style. Zero config reproduces today's look exactly.

## Background (current state)

- The top row of the story pane is the status bar, drawn by
  `render_status_content` (`render/transcript.rs`). It fills the row with the
  single `statusbar` style (reversed-video by default), draws `format_status`'s
  **left** part (location) flush-left, the **right** part (`"Score: X  Moves: Y"`
  or `"HH:MM"`) flush-right, and overlays a `[filter: story]` / `[filter: meta]`
  indicator at the far right.
- `format_status(&StatusLine) -> (left, right)` reads `machine.status_line()`:
  `StatusLine { location, right: ScoreTurns { score, turns } | Time { hours, minutes } }`.
- An optional frame already exists: when `status_header_style != None`, the bar
  is boxed (3 rows) via `draw_pane_frame`, colored by the `status_header` style.
- Transient `status_msg` (e.g. "saved") overrides the whole bar, left-aligned.
- `style.toml` parsing lives in `style.rs`; top-level array/table sections
  (`[[transcript.rule]]`) already coexist with `[colors]` selectors. Resolved
  style lives on `ColorScheme`.

## Design

### 1. Data model

A new **top-level `[statusbar]` block** in `style.toml` (a sibling of
`[[transcript.rule]]`, NOT under `[colors]`):

```toml
[statusbar]
border    = "single"     # frame rule: none|single|double|thick|picture-frame
border_fg = "cyan"       # frame color

[[statusbar.segment]]
text  = "{location}"
align = "left"
fg    = "cyan"
bold  = true

[[statusbar.segment]]
text  = "Score: {score}  Moves: {moves}"
align = "right"

[[statusbar.segment]]
text  = "{filter}"
align = "right"
fg    = "dark-gray"
```

- The existing **`[colors].statusbar` selector stays** as the bar's **base
  style** (the row fill + default fg/attrs). Each segment's style is **patched
  over** that base (a segment overriding only the properties it sets).
- `border` / `border_fg` drive the bar's frame by mapping onto the existing
  `status_header_style` / `status_header` fields (reusing the current boxing
  path). `[statusbar].border` takes precedence over a bare `status_header`
  selector when both are present; the old selectors remain for back-compat.

### 2. Segments

Each `[[statusbar.segment]]` is `{ text, align, fg?, bg?, bold?, italic? }`:

- **`text`** — a template of literal text mixed with `{placeholder}` tokens.
- **`align`** — `left` | `center` | `right`: the **cluster** the segment joins
  (default `left`). Alignment positions the *segment*, not text within it.
- **Style** — `fg`/`bg`/`bold`/`italic`, patched over the base `statusbar` style.

**Placeholders** (resolved per turn):

| Token        | Source                                                        |
|--------------|---------------------------------------------------------------|
| `{location}` | `StatusLine.location`                                         |
| `{score}`    | `ScoreTurns.score` (empty on clock games)                    |
| `{moves}`    | `ScoreTurns.turns` (the game's move count; empty on clock games) |
| `{time}`     | `Time` formatted `HH:MM` (empty on score games)              |
| `{turns}`    | `AppState.turns` (babelmap's session command counter)        |
| `{title}`    | `AppState.title` (resolved adventure title)                  |
| `{filter}`   | `[filter: story]` / `[filter: meta]`, or empty when `Both`   |

An unknown `{token}` resolves to **empty** (never rendered literally), so typos
fail quiet rather than printing braces.

**Visibility rule:**

- A segment with **no placeholder** (pure literal, e.g. a `│` separator) is
  **always displayed**.
- A segment with **one or more placeholders** is displayed **iff at least one of
  its placeholders resolves to a non-empty value**; it is hidden when **all** are
  empty. So `Score: {score}  Moves: {moves}` vanishes on a clock game, `{time}`
  vanishes on a score game, and a literal separator never vanishes.

**Sizing:** each visible segment is sized to its **resolved content width**
(placeholders substituted, measured in display columns). No fixed widths, no
auto-padding — spacing and separators are literal text in `text`. A hidden
segment contributes zero width.

### 3. Layout & truncation

Three clusters packed independently:

```
[ {location}                 {title}                 Score: 10  Moves: 5  [filter] ]
  └ left: flush-left ┘     └ center: centered ┘     └──── right: flush-right ────┘
```

- **Left** cluster: its segments concatenate left-to-right starting at the left
  edge.
- **Right** cluster: its segments concatenate in declared order, the group
  packed flush against the right edge.
- **Center** cluster: its segments concatenate, the group centered in the gap
  between the left and right clusters.

**Truncation** when the three clusters cannot all fit on one row, in this order:
1. Drop the **center** cluster entirely.
2. Truncate the **left** cluster (clip to the space remaining before the right
   cluster).
3. The **right** cluster is preserved last (score/moves/clock are short and the
   information users most want visible — matches today's behavior where location
   truncates but score/moves stay).

If even the right cluster cannot fit, it is clipped to the row width.

### 4. Defaults, back-compat, and overrides

- **No `[statusbar]` block** → a built-in default segment set reproduces today's
  look exactly:
  - left: `{location}`
  - right: `Score: {score}  Moves: {moves}` (auto-hides on clock games)
  - right: `{time}` (auto-hides on score games)
  - right: ` {filter}`
  - base style = the existing `statusbar` selector (reversed-video by default);
    no frame (`border = none`).
  Zero-config users see no visual change.
- **`status_msg` override** is unchanged: while a transient message is set it
  overrides all segments, drawn left-aligned in the base style.
- **`{filter}`** is exposed as a placeable field; the default bar pins it to the
  right exactly as today.

### 5. Architecture / components

- `crates/app/src/style.rs`:
  - New raw types: `RawSegment { text: String, align: String, decl: Decl }` and
    `RawStatusBar { border: Option<String>, border_fg: Option<String>, segments:
    Vec<RawSegment> }`; add `status_bar: RawStatusBar` to `StyleDoc`.
  - `parse_style_toml` reads the top-level `[statusbar]` table + its
    `[[statusbar.segment]]` array (mirrors the `[[transcript.rule]]` parser).
  - `merge`: an override `[statusbar]` with any segments replaces the base's
    segments (same replace-if-present rule as transcript rules); `border` /
    `border_fg` use `or` semantics.
  - `resolve`: compile `RawStatusBar` into a resolved `StatusBarLayout` (segments
    with a resolved `Style`, `Align`, and the parsed `text`), stored on
    `ColorScheme`; map `border`/`border_fg` onto `status_header_style` /
    `status_header`. Unknown `align` warns and defaults to `left`.
- `crates/app/src/colors.rs`:
  - New `pub enum Align { Left, Center, Right }`, `pub struct StatusSegment {
    text: String, align: Align, style: Style }`, and `pub struct StatusBarLayout
    { segments: Vec<StatusSegment> }`.
  - `ColorScheme` gains `pub statusbar_layout: StatusBarLayout`; `terminal_default`
    / `from_ghostty` populate the built-in default segment set (§4). `Style`/enum
    fields are `PartialEq`, so `ColorScheme` keeps its derive.
- `crates/app/src/render/transcript.rs`:
  - Placeholder resolution: a pure helper `resolve_placeholders(text, &fields) ->
    Option<String>` returning `None` when the segment must hide (has placeholders,
    all empty), `Some(resolved)` otherwise, where `fields` is a small struct built
    from `StatusLine` + `AppState`.
  - Cluster packing: a pure helper that takes the visible `(text, style, align)`
    rows + row width and returns draw positions, applying the truncation order.
    Unit-testable without a `Buffer`.
  - `render_status_content` rewritten to: short-circuit on `status_msg`
    (unchanged), else resolve each segment, drop hidden ones, pack clusters, draw.
- `write_style_full` **exports** the authored UI styling so the file is fully
  self-contained:
  - The `[statusbar]` block's **segments** (each `text` / `align` + style via
    `style_to_decl`). The frame is NOT re-emitted here — it already round-trips
    through the existing `status_header` selector export (onto which
    `border`/`border_fg` map), so emitting it twice is avoided.
  - The **`[[transcript.rule]]`** array (each rule's `match` = `pattern` + style
    via `style_to_decl`). This corrects the currently-merged transcript-styling
    feature, where `write_style_full` skipped the rules — a small cross-feature
    change included here so the "self-contained export" guarantee is complete for
    all transcript/statusbar styling at once.
  - Round-trip holds: for `terminal_default` (empty rules, default segments) the
    export reproduces the same `ColorScheme`; for custom rules/segments the
    `pattern`/`text`/`align`/style decompose and re-resolve identically.

## Error handling

- Unknown placeholder → empty string (no literal braces).
- Unknown `align` value → warning, treated as `left`.
- Invalid `border` name → reuses `parse_border_style` (unknown → `none`).
- A `[statusbar]` block with zero segments → falls back to the built-in default
  segment set (an empty bar is never useful; treat as "unset").
- Width 0 / height 0 region → draw nothing (as today).

## Testing

- Parse: `[statusbar]` + `[[statusbar.segment]]` array round-trips into
  `RawStatusBar`; `border`/`border_fg`/per-segment style read correctly.
- Placeholder resolution: each token resolves from a synthetic `StatusLine` +
  `AppState`; unknown token → empty; `{filter}` reflects the active filter.
- Visibility: pure-literal segment always shown; all-empty-placeholder segment
  hidden; mixed (one empty, one non-empty placeholder) shown.
- Clock vs score: default right segments switch correctly (`Score…Moves` hides on
  a `Time` status, `{time}` hides on a `ScoreTurns` status).
- Cluster packing: left flush-left, right flush-right, center centered in the
  gap; multiple segments in one cluster concatenate in order.
- Truncation: center dropped first, then left truncates, right preserved; right
  clipped only when nothing else fits.
- Defaults: with no `[statusbar]` block the rendered row matches today's bar
  (location left; `Score: X  Moves: Y` or `HH:MM` right; filter indicator right).
- `status_msg` still overrides all segments.
- Border: `border = "single"` boxes the bar (3 rows) with `border_fg`.
- `write_style_full` round-trip: a `ColorScheme` carrying a **custom** statusbar
  segment list AND a custom `[[transcript.rule]]` exports, re-parses, and resolves
  back to an equal `ColorScheme` (segments, rules, and styles all preserved). The
  existing `terminal_default` round-trip stays green.

## Out of scope (deferred)

- Per-side borders for the bar (top/bottom/left/right) — that is sub-design #82.
- Text-alignment *within* a fixed-width segment (segments are content-sized).
- Dynamic/conditional segments beyond the empty-placeholder auto-hide rule
  (no `#{?…}` conditionals like full tmux).
- Click targets on segments (e.g. click the filter segment to cycle filters).
- Authoring the statusbar block from the **gallery** UI (it is exported by
  `write_style_full` and hand-editable in `style.toml`, but no live-preview editor
  is added).
