# Gallery Options (Nerd Font arrow/portal families + preview fidelity) — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO item:** "Enhance the symbol gallery with many more options" (#18) — Nerd Font Material Design arrow families, corner arrows, 4 distinct portal icons, and a higher-fidelity gallery preview.
**Depends on:** the symbol/style system (merged). New presets are selectable + persist via the existing gallery → `style.toml` path. No `mapper`/`zvm` changes.
**Touches:** `crates/app/src/symbols.rs` (new `Arrows`/`PortalGlyphs` presets), `crates/app/src/render/gallery.rs` (preview upgrade). Possibly `state.rs` only if the preview needs a richer scene helper.

## Goal

Add Nerd Font Material Design (`md-`) arrow families and a distinct 4-icon portal family to the gallery, and upgrade the live preview so a preset choice visibly demonstrates corners, portals, and a real path — not just one arrow.

## 1. New arrow presets

The `Arrows` struct already has 8 slots (`north south east west` + `ne nw se sw`) and the renderer already draws diagonal arrows for diagonal connectors. Add **four** new presets to `Arrows::preset_names()`/`preset()` (existing `filled`/`line`/`nerdfont` stay):

| preset | cardinal up/down/left/right (MDI codepoints, verified) |
|---|---|
| `nf-bold` | `arrow-up-bold` F0737 / `arrow-down-bold` F072E / `arrow-left-bold` F0731 / `arrow-right-bold` F0734 |
| `nf-box` | `arrow-up-bold-box` F0738 / F072F / F0732 / F0735 |
| `nf-circle` | `arrow-up-bold-circle` F005F / F0047 / F004F / F0056 |
| `nf-outline` | `arrow-up-bold-outline` F09C7 / F09BF / F09C0 / F09C2 |

Slot mapping: `north`=up, `south`=down, `east`=right, `west`=left. Encode as `'\u{Fxxxx}'` (matching the existing nerdfont preset's style).

### Corner (diagonal) arrows

Diagonal slots map: `nw`=top-left, `ne`=top-right, `sw`=bottom-left, `se`=bottom-right. **Use the family's native MDI diagonal glyph when it exists; otherwise fall back to the Unicode diagonal** (`nw='↖' ne='↗' se='↘' sw='↙'`, as the current `nerdfont` preset does).

- `nf-box` has native diagonals (verified): `nw`=`arrow-top-left-bold-box` F1968, `ne`=`arrow-top-right-bold-box` F196A, `sw`=`arrow-bottom-left-bold-box` F1964, `se`=`arrow-bottom-right-bold-box` F1966.
- `nf-bold` / `nf-circle` / `nf-outline`: MDI does not uniformly ship plain/circle/outline diagonal arrows. The implementer resolves `arrow-{top-left,top-right,bottom-left,bottom-right}-bold-outline` (and -bold) against the Nerd Fonts `glyphnames.json` / MDI webfont CSS; if a name is absent, that family's diagonal slots use the Unicode fallback. (Outline likely has diagonals; bold/circle likely fall back — confirm at impl, do not invent codepoints.)

## 2. New portal preset

Add a `nerdfont-stairs` preset to `PortalGlyphs::preset_names()`/`preset()` with 4 DISTINCT direction icons (the decided mapping):
- `up` = `stairs-up`
- `down` = `stairs-down`
- `in_` = `location-enter`
- `out` = `exit-run`
- `marker` = `\u{F111}` (reuse the nerdfont circle), `unknown` = `\u{F059}` (question-circle), `path`/`path_h` stay box-drawing (`┊`/`┄`).

Codepoints for `stairs-up`/`stairs-down`/`location-enter`/`exit-run` are pinned at implementation from the MDI webfont CSS / Nerd Fonts `glyphnames.json` by name (the targeted fetch in design couldn't confirm them — resolve and verify). `ladder` is documented as an optional per-glyph override (`portal.up`/`portal.down`) for users who prefer it.

## 3. Codepoint resolution + validation (de-risking)

Glyph NAMES (above) are authoritative; the `md-` Nerd Font glyphs reuse the MDI codepoint. The implementer:
1. Resolves each named glyph → `'\u{Fxxxx}'` from `glyphnames.json` (or the MDI webfont CSS) — using the verified seeds in this doc and looking up the rest.
2. Adds a unit test asserting EVERY char in every new preset is a **single-width** char (reuse the existing `is_wide_estimate`/override-validation path) so a wrong/wide codepoint can't silently break alignment.
3. For any diagonal that has no native family glyph, uses the Unicode fallback (tested).

## 4. Gallery preview upgrade

`render/gallery.rs` `draw_preview` currently draws a 7×3 box + one `path.ew` + one `marker` + one `east` arrow. Replace it with a higher-fidelity scene rendered in the CURRENTLY SELECTED styles (box/arrows/portal/path), sized to the preview pane:
- A real room **box** drawn in the selected `BoxStyle`.
- **All 8 arrows**: the 4 cardinal exits on the box's sides + the 4 corner arrows at the box corners (so corner glyphs are visible and comparable across presets).
- A **longer multi-segment path** using the selected `PathGlyphs`: a run of `ew`/`ns` straights, at least two corners (`se`/`sw`/`ne`/`nw`), and one junction (`nse`/`ews`/…) — so path style is actually demonstrated, not a single dash.
- **All 4 portal icons** (up/down/in/out) from the selected portal preset, labeled or positioned distinctly so the 4 senses are visible at once.
Keep it within the preview pane bounds; degrade gracefully (skip elements that don't fit) on small panes.

## 5. Integration

- New presets appear in the existing gallery categories (Arrows, Portals) — no new category, no `GalleryState` change beyond the larger `preset_names()` lists (the selection index already indexes `preset_names()`).
- Selecting them persists through the existing `symbol_config()` → style-file write path (per the standing "keep style.toml current" rule — the new preset names are valid `[symbols]` `arrow_set`/`portal_icons` values).
- **Defaults unchanged:** `filled` arrows + `ascii` portals remain the defaults; the new families are opt-in selections.

## Testing

- Each new arrow/portal preset: `preset(name)` returns the expected struct; every glyph is single-width (the validation test).
- `preset_names()` includes the 4 new arrow names + the new portal name, in a stable order.
- Diagonal fallback: a family without a native diagonal yields the Unicode diagonal in `ne/nw/se/sw`.
- Round-trip: selecting a new preset in the gallery writes the right `arrow_set`/`portal_icons` name to the style file and re-resolves to the same `SymbolSet`.
- Preview render (TestBackend): with a selected box+arrows+portal+path, the preview buffer contains a box corner glyph, ≥1 corner-arrow glyph, ≥2 distinct path glyphs, and the 4 portal glyphs (assert the chars appear).

## Out of scope / non-goals

- New arrow DIRECTIONS or portal SENSES (the 8 arrow slots + 4 portal senses suffice).
- Changing the renderer's slot/precedence logic (`portal_slot`, `diagonal_arrow`) — only the glyph TABLES grow.
- Bundling/shipping a Nerd Font; users still need a Nerd Font installed for these to render (the ASCII/Unicode presets remain the default).
- `mapper`/`zvm` changes.

## Risks & limitations (accepted)

- **Codepoint accuracy:** mitigated by the verified seed table + name-based resolution + the single-width validation test (a wrong codepoint shows as a missing/box glyph but never breaks layout).
- **Diagonal coverage gaps:** handled by the per-family native-or-Unicode-fallback rule.
- **Font dependency:** without a Nerd Font installed, the new presets render as tofu; acceptable (they're opt-in; defaults are font-agnostic).

## Sources (codepoint research)

- Material Design Icons webfont CSS (codepoints; reused by Nerd Font `md-`): https://github.com/Templarian/MaterialDesign-Webfont
- Material Design Icons library (icon search / codepoints): https://pictogrammers.com/library/mdi/
- Nerd Fonts glyph names: https://github.com/ryanoasis/nerd-fonts (`glyphnames.json`)
