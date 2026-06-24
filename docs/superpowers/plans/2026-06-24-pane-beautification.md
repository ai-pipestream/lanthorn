# Pane Beautification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give each pane a configurable border/box style (none/single/double/thick/picture-frame) with the picture-frame + adventure title + map layer-tabs as the DEFAULT, all themed via the #43 style file.

**Architecture:** A new pure `render/paneframe.rs` owns border rendering (incl. the notched picture-frame) and the centered top-border inset (title/tabs + overflow + hit-rects). New `*_border`/title/tab/header/input style selectors plug into the #43 `style.rs` system. `main.rs`/`transcript.rs` call the new helpers; defaults set picture-frame.

**Tech Stack:** Rust, ratatui 0.29, the existing `style.rs`/`colors.rs`/`symbols.rs` style system (merged #43).

## Global Constraints

- The new chrome is the **DEFAULT**: `DEFAULT_STYLE_TOML` sets `map_border = picture-frame`, `story_border = picture-frame`; map shows layer tabs, story shows the adventure title. `map_border`/`story_border = none` reproduces the previous plain panes. `status_header`/`input_line` default plain.
- Picture-frame is glyph-exact per the spec (see Task 2). Content area = `cols 2..=w-3, rows 2..=h-3`. Panes with `w<7` or `h<7` degrade to `single`/`none`, never panic.
- New selectors land in the #43 system: fixed selector set, `DEFAULT_STYLE_TOML`, gallery/config writers, `write_style_full`.
- No `mapper`/`zvm` changes. Build + `cargo test --workspace` must be green and warning-clean after every task (currently fully warning-clean — add no new warnings).
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Spec: `docs/superpowers/specs/2026-06-24-pane-beautification-design.md` (source of truth; read it).

## File structure
- **Create `crates/app/src/render/paneframe.rs`** — `BorderStyle`, `draw_pane_frame`, `draw_top_inset`, `InsetSegment`, `PaneFrame`.
- **Modify `crates/app/src/render/mod.rs`** — `pub mod paneframe;`.
- **Modify `crates/app/src/style.rs`/`colors.rs`/`symbols.rs`/`config.rs`** — new selectors + a `BorderSpec` (style name + color) for the border selectors.
- **Modify `crates/app/src/main.rs`** — render pane borders via `draw_pane_frame`; overlay map layer tabs + story title; thread tab hit-rects into `PaneRects`.
- **Modify `crates/app/src/render/transcript.rs`** — status-header + input-line boxing; render into `content`.
- **Modify `crates/app/src/session.rs`** — capture the opening-banner first-significant line.

---

### Task 1: BorderStyle + standard-border renderer

**Files:** Create `crates/app/src/render/paneframe.rs`; Modify `crates/app/src/render/mod.rs`.

**Interfaces — Produces:**
- `pub enum BorderStyle { None, Single, Double, Thick, PictureFrame }`
- `pub fn parse_border_style(s: &str) -> BorderStyle` (unknown → `Single`)
- `pub struct PaneFrame { pub area: Rect, pub content: Rect, pub top_inset: Rect }`
- `pub fn draw_pane_frame(buf: &mut Buffer, area: Rect, style: BorderStyle, color: Style) -> PaneFrame` — draws the perimeter for None/Single/Double/Thick (PictureFrame added in Task 2; for now treat PictureFrame as Single). `content` = the rect inside the border (== `area` for None, else inset 1 on each side). `top_inset` = the top border row between the corners (`x+1..x+w-1` at `y`), or the title row for None.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn single_border_perimeter_and_content() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let area = Rect::new(0, 0, 6, 4);
    let mut buf = Buffer::empty(area);
    let f = draw_pane_frame(&mut buf, area, BorderStyle::Single, Style::default());
    assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┌");
    assert_eq!(buf.cell((5,0)).unwrap().symbol(), "┐");
    assert_eq!(buf.cell((0,3)).unwrap().symbol(), "└");
    assert_eq!(buf.cell((5,3)).unwrap().symbol(), "┘");
    assert_eq!(f.content, Rect::new(1,1,4,2));
}

#[test]
fn none_border_content_is_full_area() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let area = Rect::new(0,0,6,4);
    let mut buf = Buffer::empty(area);
    let f = draw_pane_frame(&mut buf, area, BorderStyle::None, Style::default());
    assert_eq!(f.content, area);
}

#[test]
fn parse_border_style_known_and_unknown() {
    assert!(matches!(parse_border_style("double"), BorderStyle::Double));
    assert!(matches!(parse_border_style("picture-frame"), BorderStyle::PictureFrame));
    assert!(matches!(parse_border_style("bogus"), BorderStyle::Single));
}
```
- [ ] **Step 2: Run, confirm fail** (`cargo test -p app paneframe`).
- [ ] **Step 3: Implement** `BorderStyle`, `parse_border_style`, `PaneFrame`, `draw_pane_frame` for None/Single/Double/Thick (use glyph sets `┌─┐│└┘`, `╔═╗║╚╝`, `┏━┓┃┗┛`); `PictureFrame` falls through to Single for now. Add `pub mod paneframe;` to render/mod.rs.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(paneframe): BorderStyle + standard border renderer".

---

### Task 2: Picture-frame (notched nested) renderer

**Files:** Modify `crates/app/src/render/paneframe.rs`.

**Interfaces — Consumes:** Task 1's `draw_pane_frame`/`PaneFrame`. **Produces:** `draw_pane_frame` now renders `BorderStyle::PictureFrame` exactly per spec; for `w<7 || h<7` it falls back to `Single`.

The exact grid (heavy outer + inner light flush on sides, notched away from corners), content = `cols 2..=w-3, rows 2..=h-3`:
```
┏━━━━━━━┓
┃ ┌───┐ ┃
┃┌┘   └┐┃
┃│     │┃
┃└┐   ┌┘┃
┃ └───┘ ┃
┗━━━━━━━┛
```

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn picture_frame_exact_glyphs_and_content() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let area = Rect::new(0,0,9,8); // w=9,h=8
    let mut buf = Buffer::empty(area);
    let f = draw_pane_frame(&mut buf, area, BorderStyle::PictureFrame, Style::default());
    // outer corners
    assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┏");
    assert_eq!(buf.cell((8,0)).unwrap().symbol(), "┓");
    assert_eq!(buf.cell((0,7)).unwrap().symbol(), "┗");
    assert_eq!(buf.cell((8,7)).unwrap().symbol(), "┛");
    // inner top inset by 1 from corners (row 1: space at col1, ┌ at col2)
    assert_eq!(buf.cell((1,1)).unwrap().symbol(), " ");
    assert_eq!(buf.cell((2,1)).unwrap().symbol(), "┌");
    assert_eq!(buf.cell((6,1)).unwrap().symbol(), "┐");
    // corner notch row 2: col1 ┌, col2 ┘
    assert_eq!(buf.cell((1,2)).unwrap().symbol(), "┌");
    assert_eq!(buf.cell((2,2)).unwrap().symbol(), "┘");
    // inner side flush at col1 mid-rows
    assert_eq!(buf.cell((1,3)).unwrap().symbol(), "│");
    assert_eq!(f.content, Rect::new(2,2,5,4)); // cols 2..=6, rows 2..=5
}

#[test]
fn picture_frame_tiny_pane_degrades_to_single() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let area = Rect::new(0,0,5,5);
    let mut buf = Buffer::empty(area);
    let f = draw_pane_frame(&mut buf, area, BorderStyle::PictureFrame, Style::default());
    assert_eq!(buf.cell((0,0)).unwrap().symbol(), "┌"); // single, not ┏
    assert_eq!(f.content, Rect::new(1,1,3,3));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the picture-frame branch (outer heavy perimeter; inner top/bottom runs at rows 1 and h-2 spanning cols 2..=w-3; inner side runs at cols 1 and w-2 spanning rows 2..=h-3; the four L-notches at the corners; content `Rect::new(2,2,w-4,h-4)`). Tiny-pane guard → Single.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(paneframe): notched picture-frame border".

---

### Task 3: Centered top-border inset (title/tabs + overflow + hit-rects)

**Files:** Modify `crates/app/src/render/paneframe.rs`.

**Interfaces — Produces:**
- `pub struct InsetSegment<'a> { pub text: &'a str, pub active: bool }`
- `pub fn draw_top_inset(buf: &mut Buffer, top_inset: Rect, segments: &[InsetSegment], base: Style, active: Style) -> Vec<Rect>` — renders the segments centered within `top_inset`, bracketed `┫ … ┣` with `┃` separators between segments; active segments use `active` style, others `base`. If the rendered width exceeds `top_inset.width`, show the active segment ± neighbors with a leading/trailing `‹…›` marker on the truncated side(s). Returns the per-segment hit-rect (same order as `segments`, empty Rect for segments dropped by overflow).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn top_inset_centers_single_title() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let strip = Rect::new(0,0,20,1);
    let mut buf = Buffer::empty(Rect::new(0,0,20,1));
    let rects = draw_top_inset(&mut buf, strip, &[InsetSegment{text:"ZORK I", active:false}], Style::default(), Style::default());
    let row: String = (0..20).map(|x| buf.cell((x,0)).unwrap().symbol().to_string()).collect();
    assert!(row.contains("ZORK I"));
    // centered: leading filler before the bracket
    assert!(row.find("ZORK I").unwrap() > 3);
    assert_eq!(rects.len(), 1);
}

#[test]
fn top_inset_overflow_keeps_active_with_marker() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Style};
    let strip = Rect::new(0,0,9,1);
    let mut buf = Buffer::empty(Rect::new(0,0,9,1));
    let segs = [InsetSegment{text:"0",active:false},InsetSegment{text:"1",active:true},InsetSegment{text:"2",active:false},InsetSegment{text:"3",active:false}];
    let _ = draw_top_inset(&mut buf, strip, &segs, Style::default(), Style::default());
    let row: String = (0..9).map(|x| buf.cell((x,0)).unwrap().symbol().to_string()).collect();
    assert!(row.contains("1"));      // active shown
    assert!(row.contains("‹") || row.contains("…")); // overflow marker present
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `draw_top_inset` (compute full bracketed string; if it fits, center it in `top_inset` writing over the border cells; else build the active±neighbors window with `‹…›`). Record hit-rects.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(paneframe): centered top-border inset with overflow".

---

### Task 4: Adventure-title source (banner capture + layered resolve)

**Files:** Modify `crates/app/src/session.rs`; add the resolver near it or in paneframe.rs.

**Interfaces — Produces:**
- `pub fn first_banner_line(intro_text: &str) -> Option<String>` — first non-empty, non-`>`-prompt line, trimmed; return `None` if none; cap 40 chars.
- `pub fn resolve_title(override_name: Option<&str>, banner: Option<&str>, story_path: &Path) -> String` — first of: override, banner, else the story filename stem (no extension).
- `GameSession` (or `AppState`) captures the opening banner once at startup (the text printed before the first input) and stores it for the title.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn first_banner_line_skips_blank_and_prompt() {
    assert_eq!(first_banner_line("\n\nZORK I: The Great Underground Empire\nCopyright...\n> ").as_deref(),
               Some("ZORK I: The Great Underground Empire"));
    assert_eq!(first_banner_line("\n\n").as_deref(), None);
}

#[test]
fn resolve_title_prefers_override_then_banner_then_filename() {
    use std::path::Path;
    assert_eq!(resolve_title(Some("My Game"), Some("ZORK I"), Path::new("/x/zork1.z3")), "My Game");
    assert_eq!(resolve_title(None, Some("ZORK I"), Path::new("/x/zork1.z3")), "ZORK I");
    assert_eq!(resolve_title(None, None, Path::new("/x/zork1.z3")), "zork1");
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `first_banner_line` + `resolve_title`; capture the opening banner at session start (the accumulated transcript text before the first prompt) and store the resolved title on `AppState` (read the current session-start flow in `main.rs`/`session.rs` and add the capture there).
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(title): adventure-title source (override>banner>filename)".

---

### Task 5: New style selectors + ColorScheme fields + BorderSpec

**Files:** Modify `crates/app/src/style.rs`, `crates/app/src/colors.rs`, `crates/app/src/config.rs`.

**Interfaces — Produces:**
- `ColorScheme` gains: `map_border, story_border, story_title, map_layer_tab, map_layer_tab_active, status_header, input_line` (each a `Style`), plus `map_border_style: BorderStyle` and `story_border_style: BorderStyle` (the resolved border-style for each pane).
- A border selector in `style.toml` carries BOTH a `style` name and colors, e.g. `"map_border" = { style = "picture-frame", fg = "cyan" }`. Extend the selector handling so `map_border`/`story_border`/`status_header`/`input_line` read an optional `style = "<border-style>"` key (the others ignore it). Add these to `SELECTOR_FIELDS` (with the new fields) and to `apply_color_decls` (mapping each to its `ColorScheme` field; for the border ones, also set the `*_border_style`).
- `DEFAULT_STYLE_TOML`: add `"map_border" = { style = "picture-frame" }` and `"story_border" = { style = "picture-frame" }` (colors default to the existing border color).
- `write_style_full`: emit the new selectors (incl. the `style` key for border ones).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn resolve_sets_border_style_and_default_is_picture_frame() {
    // default doc (DEFAULT_STYLE_TOML) => picture-frame for both panes
    let doc = parse_style_toml(DEFAULT_STYLE_TOML).unwrap();
    let (cs, _set, _w) = resolve(&doc, std::path::Path::new("."));
    assert!(matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::PictureFrame));
    assert!(matches!(cs.story_border_style, crate::render::paneframe::BorderStyle::PictureFrame));
}

#[test]
fn border_selector_reads_style_and_color() {
    let doc = parse_style_toml("[colors]\n\"map_border\" = { style = \"double\", fg = \"cyan\" }\n").unwrap();
    let (cs, _s, _w) = resolve(&doc, std::path::Path::new("."));
    assert!(matches!(cs.map_border_style, crate::render::paneframe::BorderStyle::Double));
    assert_eq!(cs.map_border.fg, Some(ratatui::style::Color::Cyan));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the new `ColorScheme` fields (default them in `terminal_default`/`resolve_base` so an empty doc has sane values — `*_border_style` default `None` at the struct level but `DEFAULT_STYLE_TOML` sets picture-frame), extend `Decl` parsing to capture an optional `style` string for border selectors (a small `BorderSpec`), wire `SELECTOR_FIELDS` + `apply_color_decls` + `DEFAULT_STYLE_TOML` + `write_style_full`.
- [ ] **Step 4: Run full `cargo test --workspace`; confirm green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(style): pane border/title/tab/header/input selectors".

---

### Task 6: Map pane border + centered layer tabs (wiring)

**Files:** Modify `crates/app/src/main.rs` (draw_frame); add a tab-segment builder (paneframe.rs or main.rs).

**Interfaces — Consumes:** `draw_pane_frame`, `draw_top_inset`, `cs.map_border_style`, `cs.map_border`, `cs.map_layer_tab(_active)`. **Produces:** `PaneRects` gains `layer_tabs: Vec<(LayerId, Rect)>` (hit-rects, for a future click-to-switch); the map renders inside the frame's `content`.

- [ ] **Step 1: Write the failing test** (segment builder is the unit-testable part)
```rust
#[test]
fn layer_tab_segments_mark_active() {
    // build_layer_segments(layers, active) -> Vec<InsetSegment>
    let segs = build_layer_segments(&[0,1,2], 1);
    assert_eq!(segs.len(), 3);
    assert!(segs[1].active);
    assert!(!segs[0].active && !segs[2].active);
    assert_eq!(segs[0].text, "0");
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `build_layer_segments(layers, active) -> Vec<InsetSegment>`; in `draw_frame` render the map pane border via `draw_pane_frame(map_area, cs.map_border_style, cs.map_border)`, draw the map into `frame.content`, and overlay `draw_top_inset(frame.top_inset, &segments, cs.map_layer_tab, cs.map_layer_tab_active)`; store the returned hit-rects (paired with layer ids) in `PaneRects.layer_tabs`. (Read the current `PaneRects`/`draw_frame` and the layer accessors — `state.active_layer`, `graph` layers — first.)
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean. Add a TestBackend render test that the map pane shows the picture-frame top-left `┏` and an active tab by default.**
- [ ] **Step 5: Commit** — "feat(map): picture-frame border + centered layer tabs".

---

### Task 7: Story pane border + centered adventure title (wiring)

**Files:** Modify `crates/app/src/main.rs` and/or `crates/app/src/render/transcript.rs`.

**Interfaces — Consumes:** `draw_pane_frame`, `draw_top_inset`, `cs.story_border_style`, `cs.story_border`, `cs.story_title`, the resolved title (Task 4). **Produces:** story content renders inside the frame `content`; the title is centered in the story `top_inset`.

- [ ] **Step 1: Write the failing test** (TestBackend)
```rust
#[test]
fn story_pane_shows_title_in_border_by_default() {
    // render a sample story pane with title "ZORK I" using default (picture-frame) style;
    // assert the buffer's top row contains "ZORK I" and the top-left is ┏.
    // (construct AppState with title set, call the story-pane render path.)
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the story-pane border via `draw_pane_frame` and the title overlay via `draw_top_inset` (single segment = the resolved title), rendering the transcript into `frame.content`. Repoint the transcript draw to the inner `content` rect.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(story): book border with centered adventure title".

---

### Task 8: Status-header + input-line boxing + default/opt-out snapshots

**Files:** Modify `crates/app/src/render/transcript.rs`.

**Interfaces — Consumes:** `cs.status_header`, `cs.input_line` (each a Style + an optional border-style); `draw_pane_frame` for boxing.

- [ ] **Step 1: Write the failing tests** (TestBackend)
```rust
#[test]
fn status_header_plain_by_default_boxed_when_styled() {
    // default: status row is the plain reversed bar (no border glyphs).
    // when status_header style = single: the status row is wrapped in a box.
}

#[test]
fn input_line_plain_by_default() {
    // default: "> " prompt on a plain row (no border).
}

#[test]
fn panes_none_reproduce_plain_borderless() {
    // with map_border=none and story_border=none, the map/story panes have no border glyphs (opt-out).
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** optional boxing for the status header and input line (plain default; box when the selector's border-style != none), and verify the `none` opt-out path for the panes.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(story): optional status-header/input-line boxing + opt-out".

---

## Self-Review

**Spec coverage:**
- Border styles (none/single/double/thick/picture-frame) → Tasks 1, 2. ✅
- Picture-frame glyph-exact + content rect + tiny degrade → Task 2. ✅
- Centered top-inset + overflow + hit-rects → Task 3. ✅
- Adventure title layered source + banner capture → Task 4. ✅
- New selectors + ColorScheme fields + BorderSpec + DEFAULT picture-frame + write_style_full → Task 5. ✅
- Map border + centered layer tabs + hit-rects → Task 6. ✅
- Story border + centered title → Task 7. ✅
- Status header + input line boxing; default-new-look + none-opt-out snapshots → Task 8. ✅
- Depends on #43 (merged); no mapper/zvm → Global Constraints. ✅

**Placeholder scan:** Task 7/8 render tests describe the assertion in a comment where constructing the full AppState render path verbatim isn't possible without reading the current code — the implementer wires the existing story-pane render call (cited) and asserts the stated buffer condition; these are concrete behaviors, not vague directives.

**Type consistency:** `BorderStyle`, `PaneFrame{area,content,top_inset}`, `InsetSegment{text,active}`, `draw_pane_frame`, `draw_top_inset`, `parse_border_style`, `build_layer_segments`, `first_banner_line`, `resolve_title`, and the `ColorScheme` field names (`map_border`, `story_border`, `story_title`, `map_layer_tab`, `map_layer_tab_active`, `status_header`, `input_line`, `map_border_style`, `story_border_style`) are consistent across tasks.

## Notes for the executor
- Tasks 1–4 are pure (no style dependency) and independently testable. Task 5 wires the style system. Tasks 6–8 are integration (read the current `draw_frame`/`PaneRects`/`transcript.rs`/session-start code before editing).
- The TestBackend render assertions in Tasks 6–8 are the only partly-non-pure tests; lean on the pure helpers (Tasks 1–4) for the bulk of coverage.
