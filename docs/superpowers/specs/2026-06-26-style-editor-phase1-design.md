# Live Style Editor — Phase 1 (colors & attributes) — Design

**Date:** 2026-06-26
**Status:** Approved design (pending spec review) → implementation plan next
**Scope:** Phase 1 of a larger live style editor. Phase 2 (per-element border type, side/corner glyph overrides, the character-range picker) is a separate later cycle.

## Goal

Let the user edit lanthorn's TUI theme **interactively, in-app, with live preview**, instead of hand-editing `style.toml` and pressing reload. Phase 1 covers the **foreground/background colors and the five text attributes** (bold, italic, underline, dim, reverse) of every styleable element, via a **click-to-edit** surface: click an element on a preview board, then set its properties from swatches/toggles, and see the change immediately.

## Background (existing pieces this builds on)

- **Selectors → `ColorScheme`.** ~46 string selectors (`SELECTOR_FIELDS` in `style.rs`) map to ~40 `Style` fields on `ColorScheme` (`apply_color_decls`). Each selector carries `fg`, `bg`, and the five attributes.
- **`style.toml` model.** `parse_style_toml(text) -> StyleDoc` parses per-selector declarations (a `Decl { fg, bg, bold, italic, underline, dim, reversed }`, all optional) plus a symbols section. `merge` overlays docs; `write_style_full` exports a full doc; `decl_to_style(decl, scheme)` resolves a `Decl` to a ratatui `Style`; `colors::parse_color_value(str, scheme)` resolves a color string (named ANSI, `#hex` truecolor, or a ghostty-palette ref).
- **Live reload.** `reload::reload_style(state)` re-reads `user_dir/style.toml`, re-resolves it (IFID-aware), and swaps the live `ColorScheme` into `state.colors`. Wired to `Action::ReloadStyle`.
- **Config screen.** `config_screen` is the precedent UI: a `ConfigScreenState { working, selected }` working-copy editor opened as a full-screen mode, navigated with arrows, with Save/Cancel. The style editor mirrors this shape.

## Design overview

A new **full-screen editor mode** that edits a working **`StyleDoc`** (the declarations), not the resolved `ColorScheme`. The board previews the working doc by resolving it; Save writes the doc to `style.toml` and applies it via the existing reload path. Editing declarations (the same thing `style.toml` stores) makes Save a clean round-trip with no lossy `ColorScheme`→text conversion.

```
 user edits a property
        │
        ▼
 working StyleDoc (in StyleEditorState)
        │  resolve (parse pipeline → ColorScheme)
        ▼
 preview ColorScheme  ─────────────► board re-renders live
        │
        ├─ Save  → write StyleDoc into user_dir/style.toml (preserve untouched
        │           selectors) → reload_style → live theme updates
        ├─ Cancel→ discard working doc (live theme untouched)
        └─ Reset → revert active selector (or all) to the built-in default
```

## Components

### 1. `StyleEditorState` (new, on `AppState`)

`pub style_editor: Option<StyleEditorState>` (mirrors `config_screen: Option<ConfigScreenState>`), holding:
- `doc: StyleDoc` — the working declarations (seeded from the currently-resolved style: the user's `style.toml` if present, else the built-in default doc).
- `selectors: Vec<&'static str>` — the ordered, grouped selector list shown on the board (from `SELECTOR_FIELDS`, arranged into sections).
- `active: usize` — index of the active selector.
- `focus: EditorFocus` — which sub-area has keyboard focus (board list vs fg-swatches vs bg-swatches vs custom-entry vs attribute-chips).
- `custom_buf: String` — the in-progress `#hex` entry.
- `mru: Vec<String>` — the shared custom-color MRU (newest-first, deduped, capped 16; see §4). Loaded from / saved to the sidecar.

`open_style_editor(state)` seeds `doc` from the live resolved doc and pushes the mode; the editor is entered via a `/style` slash command and a keybind (exact key chosen during planning).

### 2. The board (preview + selector picker)

A single widget that is BOTH the live preview and the selection surface. It renders labeled **sample** renderings of every element, grouped into sections (e.g. **Map**: room / room:current / room:selected / connector / connector:portal / map_border / map_layer_tab…; **Transcript**: transcript / :input / :meta / :warning / :location / :system / suggestion / input:prompt / input:text…; **Chrome**: statusbar / helpbar / story_border / story_title / scrollbar…; **Dialogs**: dialog / :title / :button / :button:active / :shadow…; **Misc**: warning_marker / meta_marker / loc_indicator / sound_beep_high/low / upper_window…), each drawn with the **working** scheme so edits show instantly.

- Each sample occupies a **hit-rect**; a mouse click selects that selector. Keyboard up/down moves `active` through the grouped list. The active sample is highlighted.
- The board scrolls when the sections exceed the viewport.
- Samples that are normally transient (dialogs, beeps, warnings) are drawn as static representative snippets so every selector is reachable regardless of game state.

### 3. Property editor (right pane)

Shows the **active** selector's editable properties, driven by the working `Decl`:
- **fg** and **bg**: a swatch grid of the 16 ANSI named colors + a `default` cell (clears the color), a `custom [#hex]` entry, and the shared **MRU-16** row (§4). Clicking a swatch / MRU cell / committing a valid custom sets `Decl.fg`/`Decl.bg` to that color's canonical token (ANSI name, `#hex`, or unset for `default`).
- **attributes**: five toggle chips `[B][I][U][dim][rev]` mapping to `Decl.bold/italic/underline/dim/reversed`.
- All controls are mouse-clickable and keyboard-navigable (Tab/arrows move focus; Space/Enter activate).

Every property change updates the working `Decl`, which re-resolves the preview board on the next frame.

### 4. Color picker + shared MRU

- **Swatch grid:** the 16 ANSI colors (canonical names) + a `default` cell. Selecting writes the name (or clears the field).
- **Custom entry:** `#rrggbb` (truecolor) validated via `colors::parse_color_value`. Invalid input is rejected with an inline hint and does NOT change the `Decl` or the MRU.
- **MRU:** ONE shared list used by BOTH fg and bg pickers. On a successful custom commit, the value is inserted at the front, de-duplicated, capped at 16. Persisted across sessions in a small sidecar `user_dir/style_editor.toml` (`recent_colors = ["#..", ...]`), loaded when the editor opens, saved when it closes (or on each MRU change). ANSI swatch picks do NOT enter the MRU (it is the *custom*-color recents).

### 5. Persistence / Save

- **Save:** serialize the working `StyleDoc` into `user_dir/style.toml`, preserving selectors/keys the editor didn't touch (edit-in-place on the parsed doc rather than a blind full overwrite), then call `reload_style(state)` so the live `state.colors` updates. Save target is the **global** `style.toml`; per-game theme files are out of scope for v1.
- **Cancel:** drop `style_editor`; the live theme is unchanged (it was never mutated — only the working doc was).
- **Reset:** revert the active selector's `Decl` (or, with a modifier/explicit control, all selectors) to the built-in default doc's declaration.

### 6. The editor's own chrome is themeable

The editor UI (its panels, the active-row highlight, swatch borders, buttons) is drawn with the existing dialog/style selectors — **no hard-coded colors** — per the project rule that every UI element is themeable. (It styles itself with the very scheme it edits.)

## Data flow summary

1. Open → seed working `StyleDoc` from the resolved style; load MRU sidecar.
2. Click a sample → set `active` selector. Edit fg/bg/attrs → mutate the working `Decl`.
3. Each frame → resolve working doc → `ColorScheme` → render the board (live preview) + property pane.
4. Save → write doc to `style.toml` (preserving untouched content) → `reload_style` → live theme updates → close. Cancel → discard. Reset → revert active (or all).

## Error handling

- **Invalid custom hex** → rejected with an inline hint; `Decl` and MRU unchanged.
- **`style.toml` write failure on Save** → surface a transcript/status message; keep the editor open so the user can retry; the live theme already reflects the (unsaved) edits only after a successful Save+reload, so a failed save leaves the on-disk file and live theme consistent with the last good state.
- **Corrupt/unparseable existing `style.toml`** → the editor seeds from the current resolved scheme (which `reload_style` already falls back to the embedded default for), so the editor always opens with a valid working doc.
- **Resolve never panics** → reuses the existing parse/apply pipeline, which already tolerates unknown selectors (collected as warnings) and bad values.

## Testing strategy

- **Unit (deterministic):**
  - Working-doc round-trip: seed a `StyleDoc`, edit a selector's fg/bg/attrs, resolve → assert the `ColorScheme` field changed as expected; serialize → re-parse → assert the edit persisted and untouched selectors are preserved.
  - Color token handling: ANSI name / `#hex` / `default` map to the right `Decl` value and resolve via `parse_color_value`; invalid hex is rejected.
  - MRU: insert/dedup/cap-16/newest-first; sidecar save+load round-trip.
  - Hit-testing: a click at a sample's rect resolves to the correct selector; out-of-bounds clicks are no-ops.
- **Render tests:** the board renders a sample for each selector group; the active sample is highlighted; the property pane shows the active selector's values. (Mirror existing `render/config_screen` and style tests.)

## Out of scope (Phase 2 and beyond)

- Per-element **border type** and per-side/per-corner **glyph overrides**; the **character-range picker** and its MRU-32. (Separate cycle.)
- Non-border **symbols** (portal glyphs, room markers, box-drawing set) editing.
- **Per-game** theme files; RGB-slider color picking; importing/exporting named theme presets.

## Open questions

None blocking. Exact keybind for `/style`, the precise selector grouping/labels on the board, and the sidecar filename may be refined during planning.
