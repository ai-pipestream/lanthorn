# Live Style Editor — Phase 2 (border & glyph editor) — Design

**Date:** 2026-06-26
**Status:** Approved design (pending spec review) → implementation plan next
**Scope:** Phase 2 of the live style editor. Phase 1 (colors/attributes) and Phase 1.1 (polish) are merged.

## Goal

Let the user edit a bordered element's **border type** and override the **glyph of any individual side or corner**, interactively, with live preview — extending the Phase 1 editor. Picking a glyph happens in a **character-range picker** (curated Unicode blocks + a custom range), with an MRU-32 of recent glyphs.

## Background (current border infrastructure — already wired)

The border rendering is fully custom (direct buffer drawing, not ratatui `Block`), so there are no engine limits.

- **`paneframe.rs`**: `enum BorderStyle { None, Single, Double, Thick, PictureFrame }`; `Glyphs { tl, top, tr, side, bl, br }` consts (`SINGLE`/`DOUBLE`/`THICK`) + the special `PictureFrame` ramp; `draw_framed()` (the entry point), `draw_pane_frame_sides()` (per-side rendering), `corner_glyph()` (adaptive corner by side weight), `draw_top_inset()` (header strip).
- **`Decl`** (`style.rs`) already carries `style`, `style_top/bottom/left/right` (per-side *weight*-style names), `header` (bool), `shadow` (bool). `resolve_sides()` turns the per-side names into a `PaneSides`; `apply_color_decls` wires this for `map_border`, `story_border`, `status_header`, `input_line`, `upper_window_border`.
- **`ColorScheme`** carries, per bordered element, a color `Style` PLUS a `BorderStyle` (e.g. `map_border_style`), a `PaneSides`, and `*_header_on`/`dialog_shadow_on` flags.
- **Bordered selectors:** `map_border`, `story_border`, `dialog`, `upper_window_border`, `status_header`, `input_line`.
- **Phase 1 editor** (`StyleEditorState`, the property pane, the swatch picker + MRU sidecar) is the surface this extends. ratatui is 0.29.

**The gap Phase 2 fills:** today a side can be set to a preset *weight* ("thick"), but NOT to an arbitrary glyph; corners are adaptive only; there is no `Rounded` type and no glyph picker.

## Design overview

Add arbitrary per-zone (4 sides + 4 corners) **glyph overrides** to the border model, edit them from a **border box in the property pane** (chosen in brainstorming), pick glyphs in a **character-range picker modal**, and render with override-aware precedence. The live theme is never mutated until Save (same as Phase 1).

## Components

### 1. Data model & `style.toml`

- Add 8 optional fields to `Decl`: `glyph_top`, `glyph_bottom`, `glyph_left`, `glyph_right` (side fills) and `glyph_tl`, `glyph_tr`, `glyph_bl`, `glyph_br` (corners), each `Option<String>` holding exactly one (single-width) glyph. They coexist with the existing `style`/`style_*` weight fields.
- `style.toml` example: `map_border = { style = "single", glyph_top = "═", glyph_tl = "╔" }`.
- `resolve` carries them into a new per-element `PaneGlyphs { top, bottom, left, right, tl, tr, bl, br: Option<String> }`, stored on `ColorScheme` alongside the existing `PaneSides` for each bordered element (e.g. `map_border_glyphs`). Unset zones are `None`.

### 2. Border editor (property pane, for bordered selectors)

When the active selector is one of the six bordered selectors, the property pane shows the Phase-1 fg/bg color controls **plus** a border sub-editor:
- An overall **type cycle**: none / single / double / **rounded** / thick / picture-frame.
- A small **border box** with **8 clickable zones** (top/bottom/left/right edges + tl/tr/bl/br corners). A zone that has a glyph override is marked (e.g. highlighted). Clicking a zone opens the glyph picker (§3); the picked glyph is stored in the corresponding `glyph_*` field of the active `Decl`.
- **header** and **shadow** toggles where applicable (`header` for the pane selectors, `shadow` for `dialog`).
- Keyboard parity: navigate zones, Enter opens the picker, a "clear" key (e.g. `Delete`/`x`) removes the active zone's override (reverts to the type/side-derived glyph).
- When type is `picture-frame`, the 8 zones are greyed out (picture-frame is a special composite; per-zone overrides don't apply).

Each edit mutates the working `Decl`/doc and recomputes the live preview (the board sample for that selector restyles), exactly like Phase 1.

### 3. Glyph picker modal

A modal over the editor (mirrors the Phase-1 swatch picker's role, but for glyphs):
- A **grid** of the current block's glyphs; ◀▶ switches among **curated blocks**: Box Drawing (U+2500–257F), Block Elements (U+2580–259F), Geometric Shapes (U+25A0–25FF), Arrows (U+2190–21FF).
- A **custom-range** entry: type a start codepoint (`U+XXXX`) to browse an arbitrary range as a grid.
- An **MRU-32** row of recently-picked glyphs.
- A direct **char / `U+codepoint`** entry for one-off glyphs.
- A **clear / none** choice that removes the zone's override.
- Pick (click or Enter) → validates single-width, sets the zone glyph, pushes to the MRU-32, closes. Esc cancels (no change).

### 4. Rendering

- Each border cell resolves by **precedence**: explicit zone **glyph override** > **side-style** glyph (existing `PaneSides`) > **base-type** glyph. Side overrides fill the entire edge (repeated per cell); corner overrides are single cells (replacing the adaptive `corner_glyph`). This resolution is applied in **both** border render paths: the per-side pane renderer (`draw_pane_frame_sides`, used by `map_border`/`story_border`/`status_header`/`input_line`/`upper_window_border`) **and** the uniform dialog renderer (`draw_pane_frame`, used by `dialog`) — both gain a `PaneGlyphs` override argument. Factor the per-cell glyph resolution into one shared helper so the two paths stay consistent.
- Add `BorderStyle::Rounded` with glyphs `╭ ─ ╮ │ ╰ ╯` (and add `"rounded"` to `parse_border_style`).
- `picture-frame` is unchanged and ignores zone overrides (consistent with the editor greying them out).

### 5. Persistence

- `write_style_full` emits the 8 `glyph_*` fields for any selector that has them set (omitted when `None`), round-tripping through `parse_style_toml`.
- The **MRU-32 glyphs** persist in the existing editor sidecar (`user_dir/style_editor.toml`) as `recent_glyphs = [...]`, alongside the Phase-1 `recent_colors`. Loaded on editor open, saved on close (Save/Cancel), deduped, newest-first, capped at 32.

## Error handling

- **Double-width glyphs rejected.** A border cell is one column; a double-width (East-Asian-wide) or zero-width glyph would misalign the border. The picker's char/codepoint commit and any MRU/grid pick validate the glyph is single-width (display width == 1); invalid picks are refused with an inline hint and do not enter the MRU. (Width via a small lookup; no new heavy dependency — a compact East-Asian-width range check.)
- **Unrenderable codepoints / bad input** in the custom-range or codepoint entry → rejected with a hint; never panics.
- **Resolve/round-trip safety:** an unknown/empty `glyph_*` value is treated as "no override" (falls back to the type glyph); the parse pipeline already tolerates unknown content.
- Live theme untouched until Save (Cancel discards), as in Phase 1.

## Testing strategy

- **Data model:** `Decl` parses the 8 `glyph_*` fields; `resolve` carries them to `PaneGlyphs`; `write_style_full` → re-parse round-trips them; unset zones stay `None`.
- **Rendering:** cell-resolution precedence (override > side-style > base) for each of the 8 zones; `Rounded` produces `╭╮╰╯`; a side override fills the whole edge; a corner override replaces only that corner; picture-frame ignores overrides.
- **Picker:** block switching lists the right glyphs; custom-range start codepoint grids correctly; MRU-32 dedup/cap/newest-first + sidecar round-trip (`recent_glyphs`); char/`U+` parse; **double-width rejection**; clear/none removes the override.
- **Editor integration:** selecting a bordered selector shows the border box; clicking a zone routes to the picker; the type cycle updates the working doc + preview; non-bordered selectors still show the Phase-1 pane.
- Render tests mirror the existing `paneframe`/`style_editor` test patterns.

## Out of scope

- Drop-shadow for non-dialog panes (shadow stays dialog-only, as today).
- New named box-style presets beyond adding `rounded` (arbitrary patterns are achieved via per-zone glyph overrides, not new named styles).
- Editing the room-box symbol set (`symbols.rs` presets) — that's the map's room glyphs, a separate concern.
- A full Unicode browser (the curated blocks + custom range cover the need).

## Open questions

None blocking. The exact "clear override" keybinding, the precise curated block list, and the border-box zone layout in the pane may be refined during planning.
