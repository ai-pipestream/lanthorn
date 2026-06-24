# Pane Beautification — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO items:** #42 (map picture-frame border + simple-border option), #44 (book-style story border + centered adventure title), #45 (story header / status line styling), #46 (input-line styling), + map layer-tab strip.
**Depends on:** #43 shareable style file (merged). All new visual settings are `style.toml` selectors per the standing "keep style.toml current" instruction.
**Touches:** new `crates/app/src/render/paneframe.rs`; `main.rs` (pane border rendering); `render/transcript.rs` (status header, input line, story title); `colors.rs`/`style.rs`/`symbols.rs`/`config.rs` (new selectors + box-style keys); `session.rs` (capture opening banner for the title). No `mapper`/`zvm` changes.

## Goal

Give each pane region a configurable border/box style + colors, plus two decorative treatments: a **picture-frame** map border (notched nested corners) and a **book-style** story border (centered adventure title inset in the top border). The map border carries a centered **layer-tab strip**; the status line and input line get optional box styling. **Every default reproduces today's look** — beautification is opt-in via `style.toml`.

## Border styles (shared set)

A new `BorderStyle` choice used by both panes: `none | single | double | thick | picture-frame`.
- `single` `┌─┐│└┘`, `double` `╔═╗║╚╝`, `thick` `┏━┓┃┗┛` — standard single-rectangle borders (reuse the existing `BoxStyle` glyphs where possible).
- `none` — no border (today's panes are borderless content with a focused-title; keep that as the default until the user opts in).
- `picture-frame` — the composite renderer below.

Selectors: `map_border`, `story_border` (each: a border-style name + a color). Default for both = today's behavior (the existing focused/unfocused pane title + `focused_border` color), i.e. effectively `none`+title until changed.

### picture-frame renderer (exact)

A heavy outer frame with a light inner border that runs **flush** along the side edges but **notches away** from each corner. For a pane rect of width `w`, height `h` (w,h ≥ 7):

```
┏━━━━━━━━━━━━━━━━━┓      row 0:      outer top      ┏ + ━…━ + ┓
┃ ┌─────────────┐ ┃      row 1:      inner top, INSET 1 from each corner (space at col1 and col w-2)
┃┌┘             └┐┃      row 2:      corner L-notch: col1 ┌, col2 ┘ … col w-3 └, col w-2 ┐
┃│   content     │┃      rows 3..h-4: inner sides FLUSH at col1 │ and col w-2 │
┃│               │┃
┃└┐             ┌┘┃      row h-3:    bottom corner L-notch: col1 └, col2 ┐ … col w-3 ┌, col w-2 ┘
┃ └─────────────┘ ┃      row h-2:    inner bottom, INSET 1
┗━━━━━━━━━━━━━━━━━┛      row h-1:    outer bottom
```

- **Outer** heavy frame at the pane perimeter (cols 0 and w-1, rows 0 and h-1).
- **Inner** light border: vertical runs flush at col 1 and col w-2 (rows 2..h-3); horizontal runs at row 1 and row h-2 but inset 1 cell from each side (cols 2..w-3). The four **corner notches** are the L-steps (`┌┘`, `└┐`, etc.) connecting an inset horizontal run to a flush vertical run — they keep clear of the outer corners.
- **Content area** = `cols 2..=w-3, rows 2..=h-3` (fully inside the inner border; notches never intrude into it).
- All glyphs come from the configured border color; the inner/outer can share one color in v1.

`paneframe.rs` exposes:
```rust
pub enum BorderStyle { None, Single, Double, Thick, PictureFrame }
pub struct PaneFrame { pub area: Rect, pub content: Rect, pub top_inset: Rect /* the drawable span of the top border for title/tabs */ }
pub fn draw_pane_frame(buf, area, style, color, symbols) -> PaneFrame;
```
For `None`, `content == area` (minus any title row as today). The returned `top_inset` is the cells of the top border between the corners, used to overlay a centered title or tab strip.

## Top-border inset content (centered): title + layer tabs

Both the story title and the map layer-tab strip are drawn **centered** into the pane's `top_inset` (the top border line), bracketed so they read as inset into the border, e.g. `━━┫ … ┣━━`. A shared helper:
```rust
pub fn draw_top_inset(buf, top_inset: Rect, segments: &[InsetSegment], colors) -> Vec<(usize, Rect)>; // returns per-segment hit-rects
```
where each `InsetSegment { text, active: bool }`. Centered within `top_inset`; if the content is wider than `top_inset`, fall back to showing the active item ± neighbors with a `‹…›` overflow marker on the truncated side.

### Adventure title (#44 — story pane)

A single centered segment = the adventure name, styled via `story_title` (fg/bg/bold). **Source, layered (first that yields a value):**
1. explicit `title` override (a `title = "..."` key in the style/config),
2. **opening-banner heuristic** — the first significant line of the game's intro text, captured once at session start (`session.rs`): the first non-empty, non-prompt line, trimmed; if it's ALL-CAPS or title-cased treat it as the title (cap length ~40 chars),
3. the story **filename stem** (e.g. `zork1.z3` → `Zork1`).
Stored on `AppState` (computed at startup). Rendered centered in the story pane's top border.

### Map layer tabs (map pane)

Segments = one per map layer (label = the layer's id/name), drawn centered in the map pane's top border; the **active** layer's segment gets the `map_layer_tab:active` style (accent/reverse), inactive get `map_layer_tab`. Separator `┃` between tabs (style A). Overflow → active ± neighbors with `‹…›`. The returned hit-rects are stored (in `PaneRects`) so a click on a tab can later route to a switch-layer action (wiring the click is a small follow-up; v1 renders + records rects).

Selectors: `story_title`, `map_layer_tab`, `map_layer_tab:active`.

## Story header / status line (#45)

The status line (current location / score / moves row) gets optional box styling via a `status_header` selector (border-style + colors): default = today's plain reversed bar (`status_bar`); opt-in = a boxed header. Rendered in `transcript.rs`.

## Input line (#46)

The `>` prompt line gets optional box styling via an `input_line` selector (border-style + colors): default = today's plain prompt line; opt-in = a boxed input field. Rendered in `transcript.rs`. (Cursor handling unchanged.)

## Style integration (#43 system)

Add the new selectors to the fixed selector set, `DEFAULT_STYLE_TOML`, the gallery/config write paths, and `write_style_full`:
`map_border`, `story_border`, `story_title`, `map_layer_tab`, `map_layer_tab:active`, `status_header`, `input_line`.
Border-style choices (`map_border`/`story_border`/`status_header`/`input_line` = a style NAME) live alongside colors; a border-style value is one of `none|single|double|thick|picture-frame`. Defaults chosen so a fresh `style.toml`/no-style reproduces today's exact rendering.

## Components

- **`render/paneframe.rs` (new):** `BorderStyle`, `draw_pane_frame` (incl. the picture-frame notched renderer), `draw_top_inset` (centered title/tabs + overflow + hit-rects).
- **`main.rs`:** render the map + story pane borders via `draw_pane_frame`; overlay the map layer-tab strip and the story title via `draw_top_inset`; thread the tab hit-rects into `PaneRects`.
- **`render/transcript.rs`:** status-header box (`status_header`) + input-line box (`input_line`); render content into the frame's `content` rect.
- **`session.rs`:** capture the opening-banner first-significant-line at startup for the title source.
- **`colors.rs`/`style.rs`/`symbols.rs`/`config.rs`:** the new selectors + the border-style enum/parse + the `*::preset` plumbing.

## Testing

- picture-frame renderer: for a known `w,h`, the exact glyph grid above is produced; `content` rect is `cols 2..=w-3, rows 2..=h-3`; notches never overlap `content`; tiny panes (`w<7`/`h<7`) degrade to `single`/`none` without panic.
- border styles: each of single/double/thick/none draws the right perimeter glyphs and `content` rect.
- top-inset: a centered title is centered within `top_inset`; an over-wide tab strip shows active±neighbors + `‹…›`; hit-rects map to the right segment.
- title source layering: override > banner > filename (table test, incl. a banner that is all-caps title-cased, and a fallback to filename when the banner is empty).
- status header / input line: plain (default) vs boxed; content area shrinks correctly when boxed.
- style round-trip: the seven new selectors parse/resolve/write_style_full and appear in `DEFAULT_STYLE_TOML`; **a default (no-style) build renders byte-identically to today** (golden TestBackend snapshot of a sample frame).

## Out of scope / non-goals

- Wiring the layer-tab CLICK to switch layers (v1 renders + records hit-rects; the click handler is a small follow-up).
- The dialog/modal chrome (separate spec: dialog-chrome-system).
- Animated docks (#12/#14) and the scrollbar (#13).
- Per-edge different inner/outer border colors for picture-frame (one color in v1).
- `mapper`/`zvm` changes.

## Risks & limitations (accepted)

- **Picture-frame costs 2 cells of content** on each side (outer + inner); on small terminals the renderer degrades to `single`/`none` (tested) rather than clipping the map.
- **Banner heuristic is best-effort** — when no clean title line exists it falls back to the filename; an explicit `title` override always wins.
- **Tab overflow** with many layers is handled by active±neighbors + `‹…›`; full horizontal scrolling of tabs is out of scope.
- **Default-look parity** is load-bearing (golden snapshot test) so beautification stays opt-in.
