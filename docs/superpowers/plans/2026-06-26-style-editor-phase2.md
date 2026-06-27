# Live Style Editor — Phase 2 (border & glyph editor) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Let the user set a bordered element's border type (incl. a new `rounded`) and override the glyph of any individual side or corner, via a border box in the property pane + a character-range glyph picker, with live preview and `style.toml` persistence.

**Architecture:** Add 8 per-zone glyph-override fields to `Decl` → a `PaneGlyphs` carried on `ColorScheme`; thread an override layer through the existing custom border renderers (`draw_pane_frame_sides` + the uniform `draw_pane_frame`) with precedence override>side-style>base; add the border sub-editor to the Phase-1 property pane and a glyph-picker modal (mirroring the `reset_dialog` modal pattern) with an MRU-32 sidecar.

**Tech Stack:** Rust, ratatui 0.29, the existing `paneframe.rs` border renderer + Phase 1/1.1 style editor.

Design reference: `docs/superpowers/specs/2026-06-26-style-editor-phase2-design.md`.

## Global Constraints

- Commit trailers on EVERY commit body (no backticks anywhere in bodies — zsh):
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
- Per task: full `cargo test -p app` (lib + bin + headless) green and `cargo build -p app` **0 warnings** before committing. Run the FULL suite.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md`.
- Editor/picker chrome stays themeable (no hard-coded colors) except glyph/swatch cells (their literal content).
- **Single-width glyphs only** — reject double-width / zero-width glyphs (they break 1-cell border alignment).
- Bordered selectors: `map_border`, `story_border`, `dialog`, `upper_window_border`, `status_header`, `input_line`.

### Verified current interfaces (use as-is)

- `paneframe.rs`: `pub enum BorderStyle { None, Single, Double, Thick, PictureFrame }` (`:8`); `struct Glyphs { tl, top, tr, side, bl, br: &'static str }` (`:49`) + consts `SINGLE`/`DOUBLE`/`THICK` (`:58`); `fn glyphs_for(BorderStyle) -> &'static Glyphs` (`:242`); `pub fn parse_border_style(&str) -> BorderStyle` (`:16`); `fn border_weight(BorderStyle) -> u8` (`:233`); `fn corner_glyph(h, v: BorderStyle, which: Corner) -> &'static str` (`:258`); `pub struct PaneSides { top, bottom, left, right: BorderStyle }` (`:207`); `pub fn draw_pane_frame_sides(buf, area: Rect, sides: PaneSides, color: Style) -> PaneFrame` (`:276`); `pub fn draw_pane_frame(buf, area: Rect, style: BorderStyle, color: Style) -> PaneFrame` (`:134`); `pub fn draw_framed(buf, area, base: BorderStyle, sides: PaneSides, color: Style, header_on: bool) -> FramedPane` (`:700`); a `border_style_name(BorderStyle) -> &str` used by `write_style_full`.
- `Decl` (`style.rs:22`): fg/bg/bold/italic/underline/dim/reversed + `style`, `style_top/bottom/left/right`, `header`, `shadow` (all `Option<...>`).
- `apply_color_decls` border arms (`style.rs:326–379`) read `decl.style`/`style_*`/`header`/`shadow` and write `cs.{map,story,status_header,input_line,upper_window}_…` border fields + `cs.dialog_box_style`/`dialog_shadow_on`. `resolve_sides(base, decl) -> (PaneSides, Vec<String>)` (`:270`).
- `ColorScheme` border fields (`colors.rs`): `map_border_style/sides`, `story_border_style/sides`, `status_header_style/sides`, `input_line_style/sides`, `dialog_box_style`, `virtual_window_border`, `upper_window_border_sides`, `*_header_on`, `dialog_shadow_on`.
- `write_style_full` (`style.rs:~1080`) emits border fields via `decorate_sides(&mut Decl, base, sides)` (`:1112`) + per-selector blocks (`:1119+`).
- Editor: `StyleEditorState` (`state.rs:513`) `{ doc, preview, selectors, active, focus: StyleFocus, custom_buf, mru, attr_cursor, color_target, swatch_cursor }`; `enum StyleFocus { Board, Fg, Bg, Custom, Attrs }` (`:502`); `StyleEditorRects` (`render/style_editor.rs:33`); property pane render (`render/style_editor.rs:190–337`, `PROP_W = 40`); `draw_swatch_row` (`:354`); `style_editor_key_to_action` (`input.rs:1071`); `Action::Style*` (`input.rs:207–234`); `apply_style_set_color` (`input.rs:2710`); style-editor mouse block (`main.rs:1780–1878`).
- Modal template `reset_dialog`: flag `state.reset_dialog` (`state.rs:778`); `draw_reset_dialog` called at `main.rs:613`; key-intercept `main.rs:1387`; mouse `main.rs:1417`; opened at `input.rs:2402`.
- `style_mru.rs`: `push_mru` (`:23`), `load_mru` (`:29`), `save_mru` (`:37`), `is_valid_color_token`, `ANSI_NAMES`; sidecar `user_dir/style_editor.toml` with `recent_colors = [...]`.

---

### Task 1: Data model — `Decl` glyph fields + `PaneGlyphs` + apply/write round-trip

**Files:**
- Modify: `crates/app/src/style.rs` — 8 `Decl` glyph fields; read them in the border arms of `apply_color_decls`; emit them in `write_style_full`.
- Modify: `crates/app/src/render/paneframe.rs` — `pub struct PaneGlyphs { top, bottom, left, right, tl, tr, bl, br: Option<String> }` (`#[derive(Debug, Clone, Default, PartialEq, Eq)]`).
- Modify: `crates/app/src/colors.rs` — add `*_glyphs: PaneGlyphs` fields for the 6 bordered elements + init (all `Default`).

**Interfaces:**
- Produces: `Decl.glyph_top/bottom/left/right/tl/tr/bl/br: Option<String>`; `paneframe::PaneGlyphs`; `ColorScheme.{map_border,story_border,status_header,input_line,upper_window_border,dialog}_glyphs: PaneGlyphs`.

- [ ] **Step 1: Write the failing test** (in `style.rs` tests):

```rust
#[test]
fn glyph_overrides_parse_resolve_and_round_trip() {
    let toml = r#"[colors]
"map_border" = { style = "single", glyph_top = "═", glyph_tl = "╔" }
"#;
    let doc = parse_style_toml(toml).unwrap();
    let d = doc.colors.selectors.get("map_border").unwrap();
    assert_eq!(d.glyph_top.as_deref(), Some("═"));
    assert_eq!(d.glyph_tl.as_deref(), Some("╔"));
    // resolve carries them onto the ColorScheme
    let (cs, _set, _w) = resolve(&doc, std::path::Path::new("."));
    assert_eq!(cs.map_border_glyphs.top.as_deref(), Some("═"));
    assert_eq!(cs.map_border_glyphs.tl.as_deref(), Some("╔"));
    // write_style_full → re-parse preserves them
    let dir = std::env::temp_dir().join(format!("bm-glyph-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("style.toml");
    write_style_full(&path, &cs, &crate::symbols::SymbolSet::default()).unwrap();
    let doc2 = parse_style_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let d2 = doc2.colors.selectors.get("map_border").unwrap();
    assert_eq!(d2.glyph_top.as_deref(), Some("═"));
    assert_eq!(d2.glyph_tl.as_deref(), Some("╔"));
    let _ = std::fs::remove_dir_all(&dir);
}
```

(Confirm the exact `SymbolSet::default()` / `write_style_full` symbol arg from an existing `write_style_full` test and match it.)

- [ ] **Step 2: Run it** → compile error (fields missing).

- [ ] **Step 3: Add the fields**

- In `paneframe.rs` (near `PaneSides`):
```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneGlyphs {
    pub top: Option<String>,
    pub bottom: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub tl: Option<String>,
    pub tr: Option<String>,
    pub bl: Option<String>,
    pub br: Option<String>,
}
```
- In `Decl` (`style.rs:22`) add (after `style_right`): `pub glyph_top: Option<String>, pub glyph_bottom: Option<String>, pub glyph_left: Option<String>, pub glyph_right: Option<String>, pub glyph_tl: Option<String>, pub glyph_tr: Option<String>, pub glyph_bl: Option<String>, pub glyph_br: Option<String>,` (confirm `Decl` derives `Default`/`Deserialize`; the serde rename for these keys is the field name — `glyph_top` etc. — matching the `style.toml` keys).
- In `ColorScheme` (`colors.rs`): add `pub map_border_glyphs: PaneGlyphs,` and the same for `story_border_glyphs`, `status_header_glyphs`, `input_line_glyphs`, `upper_window_border_glyphs`, `dialog_glyphs`. Import `PaneGlyphs`. Init all to `PaneGlyphs::default()` in `terminal_default()` and `from_ghostty(...)` (grep both constructors — every field must be initialized).

- [ ] **Step 4: Read glyphs in `apply_color_decls`; helper to map Decl→PaneGlyphs**

Add a helper in `style.rs`:
```rust
fn decl_glyphs(decl: &Decl) -> crate::render::paneframe::PaneGlyphs {
    crate::render::paneframe::PaneGlyphs {
        top: decl.glyph_top.clone(), bottom: decl.glyph_bottom.clone(),
        left: decl.glyph_left.clone(), right: decl.glyph_right.clone(),
        tl: decl.glyph_tl.clone(), tr: decl.glyph_tr.clone(),
        bl: decl.glyph_bl.clone(), br: decl.glyph_br.clone(),
    }
}
```
In each border arm of `apply_color_decls`, after the existing body, add `cs.<elem>_glyphs = decl_glyphs(decl);` — for `map_border` (`cs.map_border_glyphs`), `story_border`, `status_header`, `input_line`, `upper_window_border`, and `dialog` (`cs.dialog_glyphs`). (Use whole-replace, not merge — the editor always writes the full set; an unset zone is `None`.)

- [ ] **Step 5: Emit glyphs in `write_style_full`**

Add a helper mirroring `decorate_sides`:
```rust
fn decorate_glyphs(d: &mut Decl, g: &crate::render::paneframe::PaneGlyphs) {
    d.glyph_top = g.top.clone(); d.glyph_bottom = g.bottom.clone();
    d.glyph_left = g.left.clone(); d.glyph_right = g.right.clone();
    d.glyph_tl = g.tl.clone(); d.glyph_tr = g.tr.clone();
    d.glyph_bl = g.bl.clone(); d.glyph_br = g.br.clone();
}
```
In each border emission block (`map_border` `:1119`, `story_border`, `status_header`, `input_line`, `upper_window_border`, and the `dialog` block), call `decorate_glyphs(&mut d, &cs.<elem>_glyphs);` before inserting. (Confirm the TOML serializer writes `Option<String>` fields only when `Some` — match how `style_top` etc. are emitted; the `Decl`→TOML path already skips `None`.)

- [ ] **Step 6: Run the test + full suite** → PASS, 0 warnings.

- [ ] **Step 7: Commit** (`feat(app): per-side/corner glyph-override data model (Decl + PaneGlyphs + ColorScheme)`).

---

### Task 2: Rendering — `BorderStyle::Rounded` + per-cell glyph-override resolution

**Files:**
- Modify: `crates/app/src/render/paneframe.rs` — `Rounded` variant + glyphs; thread `&PaneGlyphs` through `draw_pane_frame_sides`, `draw_pane_frame`, `draw_framed`; per-cell resolution.
- Modify: all call sites of those three fns (grep) to pass the element's `*_glyphs`.

**Interfaces:**
- Consumes: `PaneGlyphs` (Task 1), `ColorScheme.*_glyphs`.
- Produces: `BorderStyle::Rounded`; new signatures `draw_pane_frame_sides(buf, area, sides, glyphs: &PaneGlyphs, color)`, `draw_pane_frame(buf, area, style, glyphs: &PaneGlyphs, color)`, `draw_framed(buf, area, base, sides, glyphs: &PaneGlyphs, color, header_on)`.

- [ ] **Step 1: Write the failing test** (in `paneframe.rs` tests):

```rust
#[test]
fn glyph_override_beats_style_and_base() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let area = Rect::new(0, 0, 6, 4);
    let mut buf = Buffer::empty(area);
    let sides = PaneSides { top: BorderStyle::Single, bottom: BorderStyle::Single, left: BorderStyle::Single, right: BorderStyle::Single };
    let glyphs = PaneGlyphs { top: Some("═".into()), tl: Some("╔".into()), ..Default::default() };
    draw_pane_frame_sides(&mut buf, area, sides, &glyphs, Style::default());
    // top edge cell uses the override "═", not the single "─"; tl corner uses "╔"
    assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "═");
    assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔");
    // an un-overridden corner falls back to the adaptive single glyph
    assert_eq!(buf.cell((5, 0)).unwrap().symbol(), "┐");
}

#[test]
fn rounded_border_uses_rounded_corners() {
    assert_eq!(glyphs_for(BorderStyle::Rounded).tl, "╭");
    assert_eq!(glyphs_for(BorderStyle::Rounded).br, "╯");
    assert_eq!(parse_border_style("rounded"), BorderStyle::Rounded);
}
```

- [ ] **Step 2: Run it** → FAIL/compile error.

- [ ] **Step 3: Add `Rounded`**

- `enum BorderStyle` += `Rounded`. Add `const ROUNDED: Glyphs = Glyphs { tl: "╭", top: "─", tr: "╮", side: "│", bl: "╰", br: "╯" };`. In `glyphs_for`, add `BorderStyle::Rounded => &ROUNDED`. In `parse_border_style`, add `"rounded" => BorderStyle::Rounded`. In `border_style_name`, add `BorderStyle::Rounded => "rounded"`. In `border_weight`, treat `Rounded => 1` (same as Single, for adaptive corners).

- [ ] **Step 4: Thread `&PaneGlyphs` + per-cell resolution**

Add a tiny resolver and use it at every cell write:
```rust
#[inline]
fn cell_sym<'a>(over: &'a Option<String>, fallback: &'a str) -> &'a str {
    over.as_deref().unwrap_or(fallback)
}
```
- `draw_pane_frame_sides`: add param `glyphs: &PaneGlyphs`. Top edge cells → `cell_sym(&glyphs.top, g.top)`; bottom → `cell_sym(&glyphs.bottom, g.top)`; left → `cell_sym(&glyphs.left, g.side)`; right → `cell_sym(&glyphs.right, g.side)`. Corners: `set(buf, x, y, cell_sym(&glyphs.tl, corner_glyph(sides.top, sides.left, Corner::Tl)))`, and the same for tr/bl/br. (Keep the `sym != " "` guard in `set`.)
- `draw_pane_frame` (uniform): add param `glyphs: &PaneGlyphs`. Apply `cell_sym` to each cell write (top/bottom edges → `glyphs.top`; corners → `glyphs.tl/tr/bl/br`; sides → `glyphs.left` for the left column, `glyphs.right` for the right column).
- `draw_framed`: add param `glyphs: &PaneGlyphs`; pass it through to whichever of the two it calls.

- [ ] **Step 5: Update all call sites**

Grep `draw_framed(`, `draw_pane_frame_sides(`, `draw_pane_frame(` across `crates/app/src`. At each, pass the element's glyphs:
- map pane (`main.rs:~360`): `&state.colors.map_border_glyphs`.
- story pane (`main.rs:~337`): `&state.colors.story_border_glyphs`.
- dialog (`render/dialog.rs:~135`): `&st.glyphs` / `&state.colors.dialog_glyphs` (thread it to the dialog draw; if the dialog render fn doesn't have the ColorScheme, pass the glyphs through its style struct).
- status_header / input_line / upper_window_border call sites: their respective `*_glyphs`.
- Any test call sites: pass `&PaneGlyphs::default()`.
(Adding the param makes every caller a compile error until updated — that enumerates them for you.)

- [ ] **Step 6: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 7: Commit** (`feat(app): render per-side/corner glyph overrides + rounded border type`).

---

### Task 3: Glyph MRU sidecar + single-width validation

**Files:**
- Modify: `crates/app/src/style_mru.rs` — `recent_glyphs` (CAP 32) load/save/push + `is_valid_glyph` (single-width).

**Interfaces:**
- Produces: `load_glyph_mru(dir) -> Vec<String>`, `save_glyph_mru(dir, &[String])`, `push_glyph_mru(&mut Vec<String>, &str)`, `is_valid_glyph(&str) -> bool` (exactly one char, display width 1).

- [ ] **Step 1: Write the failing tests**:

```rust
#[test]
fn glyph_mru_caps_32_dedups_newest_first() {
    let mut v = Vec::new();
    for i in 0..40u32 { push_glyph_mru(&mut v, &char::from_u32(0x2500 + i).unwrap().to_string()); }
    assert_eq!(v.len(), 32);
    push_glyph_mru(&mut v, &v[5].clone()); // existing → front, no dup
    assert_eq!(v.iter().filter(|x| **x == v[0]).count(), 1);
}
#[test]
fn is_valid_glyph_rejects_multibyte_width_and_empty() {
    assert!(is_valid_glyph("═"));
    assert!(is_valid_glyph("█"));
    assert!(!is_valid_glyph(""), "empty rejected");
    assert!(!is_valid_glyph("ab"), "more than one char rejected");
    assert!(!is_valid_glyph("世"), "double-width rejected");
}
#[test]
fn glyph_mru_sidecar_round_trips() {
    let dir = std::env::temp_dir().join(format!("bm-gmru-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    save_glyph_mru(&dir, &["═".into(), "║".into()]).unwrap();
    assert_eq!(load_glyph_mru(&dir), vec!["═".to_string(), "║".into()]);
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run them** → FAIL.

- [ ] **Step 3: Implement.** Add `const GLYPH_CAP: usize = 32;`. `push_glyph_mru` mirrors `push_mru` but caps at 32. `load_glyph_mru`/`save_glyph_mru` mirror the color versions but key `recent_glyphs` in the SAME sidecar `style_editor.toml` (read/merge: when saving, preserve the `recent_colors` key and vice-versa — simplest: read the existing table, set the one key, write both keys back; OR write both MRUs whenever either changes from the editor — confirm the editor saves both on close and have `save_glyph_mru` write a file containing BOTH `recent_colors` (unchanged-passthrough) and `recent_glyphs`. To avoid clobbering, make a single `save_mrus(dir, colors, glyphs)` OR have each save read-existing-then-merge. Implement read-existing-then-merge to be safe.). `is_valid_glyph`:
```rust
pub fn is_valid_glyph(s: &str) -> bool {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => unicode_width::UnicodeWidthChar::width(c) == Some(1),
        _ => false,
    }
}
```
(If `unicode-width` is not already a dependency, either add it to `crates/app/Cargo.toml` — it's a tiny, standard, dependency-light crate — OR implement a compact East-Asian-wide range check inline. Check Cargo.toml first; prefer the crate if the project already vendors width logic elsewhere, else the inline range check to avoid a new dep. Note your choice in the report.)

- [ ] **Step 4: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 5: Commit** (`feat(app): glyph MRU-32 sidecar + single-width glyph validation`).

---

### Task 4: Glyph-picker modal

**Files:**
- Create: `crates/app/src/render/glyph_picker.rs` — `draw_glyph_picker` + `GlyphPickerRects`.
- Modify: `crates/app/src/state.rs` — `GlyphPickerState` + `AppState.glyph_picker: Option<GlyphPickerState>`.
- Modify: `crates/app/src/render/mod.rs` — register module.
- Modify: `crates/app/src/main.rs` — draw call + key-intercept + mouse (mirror `reset_dialog`).
- Modify: `crates/app/src/input.rs` — actions to open/navigate/commit the picker; on commit set the target zone's glyph + `push_glyph_mru` + recompute preview.

**Interfaces:**
- Consumes: `is_valid_glyph`, `push_glyph_mru`/`load_glyph_mru` (Task 3); `Decl.glyph_*` (Task 1); `recompute_style_preview`.
- Produces: `GlyphPickerState { target_selector: String, target_zone: BorderZone, block: usize, custom_start: Option<u32>, cursor: usize, mru: Vec<String> }`; `enum BorderZone { Top, Bottom, Left, Right, Tl, Tr, Bl, Br }`; `Action::StyleOpenGlyphPicker(BorderZone)`, `Action::GlyphPickerNav(i32)`, `Action::GlyphPickerBlock(i32)`, `Action::GlyphPickerPick`, `Action::GlyphPickerClear`, `Action::GlyphPickerCancel`, `Action::GlyphPickerChar(char)` (codepoint/custom entry).

- [ ] **Step 1: Write the failing test** (commit logic; in `input.rs` tests):

```rust
#[test]
fn glyph_picker_pick_sets_zone_glyph_and_mru() {
    let mut s = AppState::default();
    open_style_editor(&mut s);
    // active selector forced to a bordered one:
    {
        let ed = s.style_editor.as_mut().unwrap();
        ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap();
    }
    apply_action(Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top), &mut s, &mut Mapper::default());
    assert!(s.glyph_picker.is_some());
    // pretend the cursor is on "═" via the codepoint entry path:
    apply_action(Action::GlyphPickerChar('═'), &mut s, &mut Mapper::default()); // sets pending char
    apply_action(Action::GlyphPickerPick, &mut s, &mut Mapper::default());
    assert!(s.glyph_picker.is_none(), "pick closes the picker");
    let ed = s.style_editor.as_ref().unwrap();
    assert_eq!(ed.doc.colors.selectors.get("map_border").and_then(|d| d.glyph_top.clone()), Some("═".into()));
    assert!(ed.mru.iter().any(|_| true)); // color mru unaffected; glyph mru lives on the picker/sidecar
}
```

(Adjust the exact pick mechanism to your implementation — the assertion that matters: opening with a zone, picking a valid glyph sets `doc...glyph_<zone>` and closes; `GlyphPickerClear` sets it to `None`.)

- [ ] **Step 2: Run it** → compile error.

- [ ] **Step 3: State + actions + open/commit handlers**

- `state.rs`: add `enum BorderZone { Top, Bottom, Left, Right, Tl, Tr, Bl, Br }` and `struct GlyphPickerState { target_selector: String, target_zone: BorderZone, block: usize, custom_start: Option<u32>, cursor: usize, pending: Option<String>, mru: Vec<String> }`; `AppState.glyph_picker: Option<GlyphPickerState>` (init `None`).
- `input.rs`: add the actions. `StyleOpenGlyphPicker(zone)` captures `target_selector = ed.selectors[ed.active]`, loads the glyph MRU, opens `glyph_picker`. `GlyphPickerPick` validates the selected glyph via `is_valid_glyph`, writes it into the target selector's `Decl.glyph_<zone>` (a small `set_zone_glyph(decl, zone, Some(g))`), `push_glyph_mru`, closes the picker, `recompute_style_preview`. `GlyphPickerClear` sets the zone to `None` + recompute + close. `GlyphPickerCancel` closes without change. Nav/Block/Char update the picker grid/cursor/custom entry.
- A helper `fn set_zone_glyph(decl: &mut Decl, zone: BorderZone, g: Option<String>)` maps the zone to the right `glyph_*` field.

- [ ] **Step 4: Render the picker** (`render/glyph_picker.rs`)

Mirror `reset_dialog`'s modal shape (`draw_dialog` chrome, themed). Draw: a block-name header with ◀▶, a glyph grid for the current block (Box Drawing U+2500–257F, Block Elements U+2580–259F, Geometric Shapes U+25A0–25FF, Arrows U+2190–21FF, or the `custom_start` range), an MRU-32 row, a `custom: U+____` entry, and a `clear` button. Record hit-rects in `GlyphPickerRects { area, close, glyphs: Vec<(String, Rect)>, mru: Vec<(String, Rect)>, blocks_prev, blocks_next, clear, custom }`. Skip drawing non-single-width cells (or grey them). Register `pub mod glyph_picker;` in `render/mod.rs`; call `draw_glyph_picker` in `draw_frame` AFTER the style editor (so it overlays).

- [ ] **Step 5: Key + mouse intercept** (`main.rs`, mirror `reset_dialog`)

Add an `if state.glyph_picker.is_some() { ... }` key-intercept block BEFORE the style-editor key handling: arrows → `GlyphPickerNav`, `[`/`]` or `,`/`.` → `GlyphPickerBlock(∓1)`, Enter → `GlyphPickerPick`, `Delete`/`Backspace`→ clear/codepoint-edit, typed chars when in custom entry → `GlyphPickerChar`, Esc → `GlyphPickerCancel`. Add a mouse block hit-testing `GlyphPickerRects` (glyph cell → set cursor+pick; mru cell → pick; block arrows; clear; close). The picker swallows all events while open (it's modal over the editor).

- [ ] **Step 6: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 7: Commit** (`feat(app): glyph-picker modal (blocks + custom range + MRU-32 + single-width)`).

---

### Task 5: Border sub-editor in the property pane

**Files:**
- Modify: `crates/app/src/state.rs` — `StyleFocus::Border` + a `border_zone: usize` cursor on `StyleEditorState`.
- Modify: `crates/app/src/render/style_editor.rs` — for bordered selectors, render the type cycle + 8-zone border box + header/shadow toggles; extend `StyleEditorRects`.
- Modify: `crates/app/src/input.rs` — `is_bordered_selector`, border-type cycle action, zone nav, open-picker on a zone, header/shadow toggle, clear-zone.
- Modify: `crates/app/src/main.rs` — mouse hit-tests for the border box (zones/type/toggles).

**Interfaces:**
- Consumes: `Action::StyleOpenGlyphPicker(BorderZone)` (Task 4); `BorderStyle`/`parse_border_style`/`border_style_name` (Task 2); `ColorScheme.*_glyphs` for marking overridden zones; `recompute_style_preview`.
- Produces: `Action::StyleBorderTypeCycle(i32)`, `Action::StyleBorderZoneNav(i32)`, `Action::StyleBorderToggleHeader`, `Action::StyleBorderToggleShadow`, `Action::StyleBorderClearZone`; `fn is_bordered_selector(&str) -> bool`.

- [ ] **Step 1: Write the failing test**:

```rust
#[test]
fn border_type_cycle_updates_decl_style() {
    let mut s = AppState::default();
    open_style_editor(&mut s);
    { let ed = s.style_editor.as_mut().unwrap();
      ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap(); }
    apply_action(Action::StyleBorderTypeCycle(1), &mut s, &mut Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    let st = ed.doc.colors.selectors.get("map_border").and_then(|d| d.style.clone());
    assert!(st.is_some(), "cycling sets the border style name on the decl");
}
#[test]
fn is_bordered_selector_covers_the_six() {
    for sel in ["map_border","story_border","dialog","upper_window_border","status_header","input_line"] {
        assert!(crate::input::is_bordered_selector(sel), "{sel} is bordered");
    }
    assert!(!crate::input::is_bordered_selector("transcript"));
}
```

- [ ] **Step 2: Run them** → compile error / FAIL.

- [ ] **Step 3: `is_bordered_selector` + border-type cycle + zone nav + toggles + clear**

- `is_bordered_selector(sel)` → the six names.
- `StyleBorderTypeCycle(d)`: cycle `decl.style` over `["none","single","double","rounded","thick","picture-frame"]` (read current from `decl.style` or default "single"), set the name, recompute.
- `StyleBorderZoneNav(d)`: move `ed.border_zone` over 0..8 (the 8 zones) with wrap.
- `StyleBorderClearZone`: set the active zone's `glyph_*` to `None`, recompute.
- `StyleBorderToggleHeader` / `StyleBorderToggleShadow`: flip `decl.header` / `decl.shadow`, recompute (header only for pane selectors; shadow only for dialog).
- Map `ed.border_zone` (0..8) to a `BorderZone` for `StyleOpenGlyphPicker`.

- [ ] **Step 4: Render the border sub-editor**

In the property pane, when `is_bordered_selector(active)`, after the color/attr rows render: a `type: ◀ <name> ▶` line; a small box (e.g. 9×3 or larger) with the 8 zones drawn from the live preview's border glyphs, the zone at `border_zone` highlighted (themed), each a hit-rect; zones with an override marked; the header/shadow toggle chips where applicable. When type is `picture-frame`, grey the zones. Extend `StyleEditorRects` with `border_zones: Vec<(BorderZone, Rect)>`, `border_type_prev/next: Option<Rect>`, `border_header: Option<Rect>`, `border_shadow: Option<Rect>`.

- [ ] **Step 5: Keys + mouse**

- `style_editor_key_to_action`: add `StyleFocus::Border` handling — Left/Right → `StyleBorderZoneNav`, Enter → `StyleOpenGlyphPicker(zone)`, `t`/`[`/`]` → `StyleBorderTypeCycle`, `Delete` → `StyleBorderClearZone`, `h`/`d` → header/shadow toggles. Ensure `StyleFocusCycle` includes `Border` in the focus ring only for bordered selectors (or always, harmless for others).
- `main.rs` style-editor mouse block: hit-test the new border rects (zone → `StyleOpenGlyphPicker`, type arrows → `StyleBorderTypeCycle`, header/shadow → toggles).

- [ ] **Step 6: Build, test, headless smoke + manual** → full suite + headless PASS, 0 warnings.

Manual (not gating): `/style` → pick `map_border` → cycle type to rounded (preview corners become ╭╮╰╯) → click the top zone → glyph picker → pick `═` → the map sample's top border shows `═` → Save → `style.toml` has `glyph_top = "═"`.

- [ ] **Step 7: Commit** (`feat(app): border sub-editor in the style editor property pane`).

---

## Notes for the executor

- **Dependency order:** 1 → 2 → 3 → 4 → 5. All `cargo test -p app`, full suite, 0 warnings before each commit.
- **Live theme untouched until Save** — same invariant as Phase 1; all border/glyph edits mutate the working `doc`/`preview`, never `state.colors`, until `StyleSave`.
- **Both render paths:** Task 2 must thread `PaneGlyphs` through BOTH `draw_pane_frame_sides` (pane selectors) AND `draw_pane_frame` (dialog). Factor the per-cell `cell_sym` resolver so they stay consistent.
- **Adding a param breaks all callers** (Task 2) — that compile error is your checklist of call sites; pass the element's `*_glyphs` (or `&PaneGlyphs::default()` in tests).
- **Single-width only** (Task 3/4): the picker must never set a multi-width glyph (it would misalign the border). Validate on every commit path.
- **Line numbers** are from a snapshot — confirm by grep before editing.
- `TODO.md` is gitignored — never stage it. No README change required (the editor surfaces it); add one only if asked.
