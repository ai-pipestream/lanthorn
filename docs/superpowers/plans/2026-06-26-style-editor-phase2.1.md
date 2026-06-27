# Live Style Editor — Phase 2.1 Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the deferred non-blocking minors from the Phase 2 style-editor review (border-focus gating, real mini-box glyphs, applicable header/shadow chips, picture-frame picker no-op, invalid-pending feedback) and add a scrollbar to the selector list.

**Architecture:** Surgical polish to the existing live style editor. Input gating lives in `crates/app/src/input.rs` (the `Action::Style*` handlers and `style_editor_key_to_action`); rendering lives in `crates/app/src/render/style_editor.rs` and `crates/app/src/render/glyph_picker.rs`. No new modules, no new state fields except a stored scroll value if needed. The render functions return rect structs (`StyleEditorRects`, `GlyphPickerRects`) that tests assert against.

**Tech Stack:** Rust 2021; ratatui 0.29 (`Scrollbar`, `ScrollbarState`, `ScrollbarOrientation`, `StatefulWidget`); crate `app`.

## Global Constraints

- 0 compiler warnings and a green full `cargo test -p app` after every task.
- Commit-only on local `main`; do not push, merge, or branch. Do not edit `TODO.md` during the wave.
- Commit message ends with these two trailer lines (no backticks anywhere in the commit body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Surgical changes only; match existing style.
- Every styleable element must be themeable via the existing style system — the new scrollbar uses the existing `scrollbar` `ColorScheme` field (`state.colors.scrollbar`); do not hard-code its style.
- The 6 bordered selectors are exactly: `map_border`, `story_border`, `dialog`, `upper_window_border`, `status_header`, `input_line` (per `is_bordered_selector` in input.rs). The 5 PANE selectors are those minus `dialog`.

---

### Task 1: Gate `StyleFocus::Border` to bordered selectors

**Files:**
- Modify: `crates/app/src/input.rs` (`Action::StyleFocusCycle` handler ~2313; `Action::StyleNav` handler ~2289)
- Test: `crates/app/src/input.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `is_bordered_selector(sel: &str) -> bool` (already `pub fn` in input.rs ~2985); `crate::state::StyleFocus` (variants `Board, Fg, Bg, Custom, Attrs, Border`); `ed.selectors: Vec<&'static str>`, `ed.active: usize`, `ed.focus: StyleFocus`.
- Produces: no new public items; only behavior change — Border is never the focus on a non-bordered selector.

- [ ] **Step 1: Write the failing test**

Add to input.rs tests. Construct a style-editor `AppState` the same way the existing style-editor input tests in this file do (locate one with `rg -n "state.style_editor = Some" crates/app/src/input.rs` and mirror its setup). The test sets the active selector to a NON-bordered one and a bordered one and checks focus cycling:

```rust
#[test]
fn border_focus_only_on_bordered_selectors() {
    use crate::state::StyleFocus;
    // Build a style editor state (mirror existing style-editor test setup).
    let mut state = make_style_editor_state_for_test();
    let ed = state.style_editor.as_mut().unwrap();

    // Pick a NON-bordered selector (e.g. one whose name is not in is_bordered_selector).
    let non_bordered_idx = ed.selectors.iter().position(|s| !crate::input::is_bordered_selector(s)).unwrap();
    ed.active = non_bordered_idx;
    ed.focus = StyleFocus::Attrs;
    // Cycling forward from Attrs on a non-bordered selector must wrap to Board, never Border.
    apply_action(Action::StyleFocusCycle(1), &mut state, &mut make_test_mapper());
    assert_eq!(state.style_editor.as_ref().unwrap().focus, StyleFocus::Board,
        "non-bordered selector must skip Border focus");

    // On a bordered selector, cycling forward from Attrs reaches Border.
    let ed = state.style_editor.as_mut().unwrap();
    let bordered_idx = ed.selectors.iter().position(|s| crate::input::is_bordered_selector(s)).unwrap();
    ed.active = bordered_idx;
    ed.focus = StyleFocus::Attrs;
    apply_action(Action::StyleFocusCycle(1), &mut state, &mut make_test_mapper());
    assert_eq!(state.style_editor.as_ref().unwrap().focus, StyleFocus::Border,
        "bordered selector must reach Border focus");

    // Navigating from a bordered selector (on Border focus) to a non-bordered one drops Border focus.
    let ed = state.style_editor.as_mut().unwrap();
    ed.active = bordered_idx;
    ed.focus = StyleFocus::Border;
    // Move selection until it lands on a non-bordered selector.
    apply_action(Action::StyleNav(1), &mut state, &mut make_test_mapper());
    let ed = state.style_editor.as_ref().unwrap();
    if !crate::input::is_bordered_selector(ed.selectors[ed.active]) {
        assert_ne!(ed.focus, StyleFocus::Border, "Border focus must drop on a non-bordered selector");
    }
}
```

Note: `make_style_editor_state_for_test` / `make_test_mapper` are placeholders for whatever setup the existing tests use — reuse the real helpers/inline setup already present in input.rs tests. Do NOT invent new helpers if the file already has the pattern inline; inline it.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app border_focus_only_on_bordered_selectors`
Expected: FAIL (Border is currently reachable on every selector).

- [ ] **Step 3: Implement the gate**

In `Action::StyleFocusCycle(d)`, build the order conditionally on whether the active selector is bordered:

```rust
Action::StyleFocusCycle(d) => {
    if let Some(ed) = &mut state.style_editor {
        use crate::state::StyleFocus;
        let bordered = is_bordered_selector(ed.selectors[ed.active]);
        let order: &[StyleFocus] = if bordered {
            &[StyleFocus::Board, StyleFocus::Fg, StyleFocus::Bg, StyleFocus::Custom, StyleFocus::Attrs, StyleFocus::Border]
        } else {
            &[StyleFocus::Board, StyleFocus::Fg, StyleFocus::Bg, StyleFocus::Custom, StyleFocus::Attrs]
        };
        let cur = order.iter().position(|f| *f == ed.focus).unwrap_or(0) as i32;
        let n = order.len() as i32;
        ed.focus = order[((cur + d).rem_euclid(n)) as usize];
        match ed.focus {
            StyleFocus::Fg => ed.color_target = false,
            StyleFocus::Bg => ed.color_target = true,
            StyleFocus::Custom => {
                if ed.custom_buf.is_empty() {
                    ed.custom_buf = "#".to_string();
                }
            }
            _ => {}
        }
    }
}
```

In `Action::StyleNav(d)`, drop a stale Border focus when the new selector is not bordered:

```rust
Action::StyleNav(d) => {
    if let Some(ed) = &mut state.style_editor {
        let n = ed.selectors.len() as i32;
        ed.active = ((ed.active as i32 + d).rem_euclid(n.max(1))) as usize;
        if ed.focus == crate::state::StyleFocus::Border
            && !is_bordered_selector(ed.selectors[ed.active])
        {
            ed.focus = crate::state::StyleFocus::Board;
        }
    }
}
```

Also defensively guard the Border-focus key actions so a non-bordered selector can never act on them even if reached by some other path. In `style_editor_key_to_action` (input.rs ~1189-1210), the border-focus branch is entered when `focus == StyleFocus::Border`; since focus can no longer BE Border on a non-bordered selector, no change is required there — but add a one-line comment at that branch noting the invariant ("// Border focus only occurs on bordered selectors; see StyleFocusCycle/StyleNav gating.").

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app border_focus_only_on_bordered_selectors`
Expected: PASS. Then `cargo test -p app` (full) and `cargo build -p app` → 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/input.rs
git commit -m "feat(app): style editor — gate Border focus to bordered selectors"
```

---

### Task 2: Real mini-box glyphs + applicable header/shadow chips

**Files:**
- Modify: `crates/app/src/render/style_editor.rs` (`zone_glyph` closure ~393-444 and its caller ~487; header/shadow chip render ~498-523)
- Test: `crates/app/src/render/style_editor.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `active_decl: Option<&Decl>` with `glyph_top/bottom/left/right/tl/tr/bl/br: Option<String>`, `header: Option<bool>`, `shadow: Option<bool>`; the active selector name `ed.selectors[ed.active]`; `StyleEditorRects { border_header: Option<Rect>, border_shadow: Option<Rect>, .. }`.
- Produces: `zone_glyph` returns `String`; `border_header` is `Some` only for the 5 pane selectors; `border_shadow` is `Some` only for `dialog`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn header_shadow_chips_are_selector_appropriate() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    // Render the style editor with a PANE bordered selector active, then with dialog.
    // Mirror the existing style_editor render tests' setup (rg -n "draw_style_editor" tests).
    let area = Rect::new(0, 0, 120, 40);

    // PANE selector (e.g. map_border): header chip present, shadow chip absent.
    let mut buf = Buffer::empty(area);
    let state = make_style_editor_state_with_selector("map_border");
    let rects = draw_style_editor(&state, area, &mut buf);
    assert!(rects.border_header.is_some(), "pane selector shows header chip");
    assert!(rects.border_shadow.is_none(), "pane selector hides shadow chip");

    // dialog selector: shadow chip present, header chip absent.
    let mut buf2 = Buffer::empty(area);
    let state2 = make_style_editor_state_with_selector("dialog");
    let rects2 = draw_style_editor(&state2, area, &mut buf2);
    assert!(rects2.border_shadow.is_some(), "dialog shows shadow chip");
    assert!(rects2.border_header.is_none(), "dialog hides header chip");
}

#[test]
fn mini_box_renders_actual_override_glyph() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    // A bordered selector whose working Decl has glyph_top = "═".
    let state = make_style_editor_state_with_top_override("map_border", "═");
    let _ = draw_style_editor(&state, area, &mut buf);
    // The chosen glyph must appear somewhere in the buffer (the mini-box top zone),
    // and the placeholder bullet must NOT be used for that override.
    let mut found = false;
    for y in 0..area.height {
        for x in 0..area.width {
            if buf[(x, y)].symbol() == "═" { found = true; }
        }
    }
    assert!(found, "mini border-box must render the actual override glyph, not a placeholder");
}
```

Note: `make_style_editor_state_with_selector` / `make_style_editor_state_with_top_override` are placeholders — reuse the real render-test setup already in this file (find with `rg -n "draw_style_editor\(" crates/app/src/render/style_editor.rs` in the test module) and set the active selector / working Decl glyph field inline.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app header_shadow_chips_are_selector_appropriate mini_box_renders_actual_override_glyph`
Expected: FAIL (both chips always render; override renders `•`).

- [ ] **Step 3: Implement (b) the real glyph**

Change the `zone_glyph` closure return type from `&'static str` to `String`. In the override branch return the real value instead of `"•"`; in the default branch return the existing default glyph as `.to_string()`:

```rust
let zone_glyph = |zone: BorderZone| -> String {
    let override_g: Option<&str> = active_decl.and_then(|d| match zone {
        BorderZone::Top    => d.glyph_top.as_deref(),
        BorderZone::Bottom => d.glyph_bottom.as_deref(),
        BorderZone::Left   => d.glyph_left.as_deref(),
        BorderZone::Right  => d.glyph_right.as_deref(),
        BorderZone::Tl     => d.glyph_tl.as_deref(),
        BorderZone::Tr     => d.glyph_tr.as_deref(),
        BorderZone::Bl     => d.glyph_bl.as_deref(),
        BorderZone::Br     => d.glyph_br.as_deref(),
    });
    if let Some(g) = override_g {
        g.to_string()
    } else {
        // existing per-zone default glyph for the current style — return it as String:
        // (keep the existing default-glyph match arms, but append .to_string())
        default_zone_glyph(zone).to_string()
    }
};
```

Keep whatever the current default-glyph logic is (the match that produced the `&'static str` defaults) — just convert its result to `String`. The caller at ~487 currently does `zone_glyph(zone)` and formats `" {} "`; that still works with a `String` (it implements `Display`). If the caller binds the result to a `&str`, change it to own the `String`.

- [ ] **Step 4: Implement (c) selector-appropriate chips**

In the header/shadow chip render block (~498-523), gate each chip on the active selector. Compute the active selector once near the top of that block:

```rust
let active_sel = ed.selectors[ed.active];
let is_dialog = active_sel == "dialog";
// header applies to pane selectors (all bordered EXCEPT dialog); shadow applies to dialog only.
let show_header = crate::input::is_bordered_selector(active_sel) && !is_dialog;
let show_shadow = is_dialog;
```

Then wrap the header-chip render (the `border_header = Some(...)` branch) in `if show_header { ... }` and the shadow-chip render (the `border_shadow = Some(...)` branch) in `if show_shadow { ... }`. When not shown, leave the corresponding rect `None` (its default).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p app header_shadow_chips_are_selector_appropriate mini_box_renders_actual_override_glyph`
Expected: PASS. Then `cargo test -p app` (full) and `cargo build -p app` → 0 warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/render/style_editor.rs
git commit -m "feat(app): style editor — real mini-box glyphs + selector-appropriate header/shadow chips"
```

---

### Task 3: Picture-frame zones no-op the picker + invalid-pending feedback

**Files:**
- Modify: `crates/app/src/input.rs` (`Action::StyleOpenGlyphPicker` handler ~2474-2491)
- Modify: `crates/app/src/render/glyph_picker.rs` (`draw_glyph_picker` ~46; add an invalid-pending hint)
- Test: both files' test modules

**Interfaces:**
- Consumes: the active working `Decl` and its `style: Option<String>` (border type; `"picture-frame"` is the composite type), `GlyphPickerState { pending: Option<String>, .. }`, `crate::style_mru::is_valid_glyph(&str) -> bool`, `state.colors` (warning/text styles).
- Produces: `state.glyph_picker` stays `None` when opening a picker for a picture-frame selector; the picker modal shows an invalid-pending warning hint.

- [ ] **Step 1: Write the failing tests**

```rust
// in input.rs tests
#[test]
fn picture_frame_zone_does_not_open_picker() {
    use crate::render::paneframe::BorderZone;
    // Build a style editor state whose active bordered selector's working Decl has style = "picture-frame".
    let mut state = make_style_editor_state_picture_frame("map_border");
    apply_action(Action::StyleOpenGlyphPicker(BorderZone::Top), &mut state, &mut make_test_mapper());
    assert!(state.glyph_picker.is_none(), "picture-frame zones must not open the glyph picker");

    // Sanity: a non-picture-frame bordered selector DOES open it.
    let mut state2 = make_style_editor_state_with_selector("map_border"); // default style (single)
    apply_action(Action::StyleOpenGlyphPicker(BorderZone::Top), &mut state2, &mut make_test_mapper());
    assert!(state2.glyph_picker.is_some(), "non-picture-frame selector opens the picker");
}
```

```rust
// in glyph_picker.rs tests
#[test]
fn invalid_pending_shows_warning_hint() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    // A glyph picker whose pending is an invalid (e.g. double-width) glyph.
    let state = make_glyph_picker_state_with_pending("漢"); // double-width => invalid
    let _ = draw_glyph_picker(&state, area, &mut buf);
    // The modal must render the "single-width only" hint somewhere.
    let text: String = buffer_text(&buf); // join all cell symbols row by row (helper or inline)
    assert!(text.contains("single-width"), "invalid pending must show a single-width-only hint");
}
```

Note: the `make_*` helpers are placeholders — reuse the existing setup in each file's test module (find with `rg -n "GlyphPickerState \{" crates/app/src` and the existing `draw_glyph_picker(` test). `buffer_text` is whatever buffer-to-string approach existing render tests use (inline a double loop over `buf[(x,y)].symbol()` if no helper exists).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app picture_frame_zone_does_not_open_picker invalid_pending_shows_warning_hint`
Expected: FAIL (picker opens regardless; no hint).

- [ ] **Step 3: Implement (d) the picture-frame no-op**

In `Action::StyleOpenGlyphPicker(zone)`, skip constructing the picker when the active selector's border type is picture-frame. `apply_action` returns `()`, so do NOT `return` an Action — wrap the existing picker-construction in `if !is_picture_frame { ... }` (the picture-frame case simply leaves `state.glyph_picker` as `None`). The border type is the working `Decl.style` for the active selector (mirror the render's detection: `style.as_deref().unwrap_or("single") == "picture-frame"`). Extract `is_picture_frame` from `ed` in the SAME scope where the existing handler already extracts data out of `ed` (e.g. alongside its `let target_selector = ed.selectors[ed.active].to_string();`), so the immutable `&state.style_editor` borrow ends before the `state.glyph_picker = Some(...)` assignment — matching the existing borrow pattern:

```rust
Action::StyleOpenGlyphPicker(zone) => {
    // Read everything needed out of the immutable style_editor borrow first
    // (mirror the existing extraction the handler already does).
    let open = state.style_editor.as_ref().map(|ed| {
        let active_sel = ed.selectors[ed.active];
        // Picture-frame is a composite border; per-zone glyph overrides don't apply.
        let is_picture_frame = ed.working_decl_for(active_sel)
            .and_then(|d| d.style.as_deref())
            .unwrap_or("single") == "picture-frame";
        (active_sel.to_string(), is_picture_frame /*, plus whatever else the existing code extracts */)
    });
    if let Some((target_selector, is_picture_frame /*, .. */)) = open {
        if !is_picture_frame {
            // ... existing picker-construction code (builds GlyphPickerState and
            //     assigns state.glyph_picker = Some(...)) unchanged ...
        }
        // picture-frame: leave state.glyph_picker as None (no-op).
    }
}
```

`working_decl_for` is a placeholder for the real accessor the handler already uses to read the active working `Decl` — reuse that exact path from the existing code at ~2474. The key requirement: the no-op must happen BEFORE any `state.glyph_picker = Some(...)`.

- [ ] **Step 4: Implement (e) the invalid-pending hint**

In `draw_glyph_picker`, after the existing pending/grid rendering, render a one-line hint when `picker.pending` is `Some(p)` and `!crate::style_mru::is_valid_glyph(p)`. Place it on a spare row near the custom/clear area (e.g. reuse a row above `clear_y`, or extend the header line). Use a warning style derived from the theme (e.g. `state.colors.transcript_warning` if present, else a red fg over the modal bg — match how other warnings are styled in this codebase; find with `rg -n "warning" crates/app/src/render`):

```rust
if let Some(p) = picker.pending.as_deref() {
    if !crate::style_mru::is_valid_glyph(p) {
        let hint = format!("'{p}' invalid — single-width only");
        let warn_style = /* theme warning style */;
        // draw on an available content row (do not overwrite the grid):
        crate::render::draw_str_clipped(buf, content.x, /* a free row y */, &hint, warn_style, content);
    }
}
```

Pick a row that does not collide with the grid/MRU/custom/clear rows already used (grid_start_y..; mru_y = bottom-3; custom_y = bottom-2; clear_y = bottom-1). If no free row exists, place the hint in the header area (header_y) appended after the block name. Keep it within `content`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p app picture_frame_zone_does_not_open_picker invalid_pending_shows_warning_hint`
Expected: PASS. Then `cargo test -p app` (full) and `cargo build -p app` → 0 warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/input.rs crates/app/src/render/glyph_picker.rs
git commit -m "feat(app): style editor — picture-frame zones no-op the picker; invalid-pending warning hint"
```

---

### Task 4: Scrollbar on the selector list

**Files:**
- Modify: `crates/app/src/render/style_editor.rs` (board render ~100-196; imports)
- Test: `crates/app/src/render/style_editor.rs` test module

**Interfaces:**
- Consumes: the board `Rect`, the already-computed scroll offset (~141-148), `max_scroll`, the total visual-line count, `state.colors.scrollbar` (the existing themeable scrollbar `Style`).
- Produces: a vertical scrollbar drawn on the board's right edge, shown only when the list overflows the visible rows.
- Pattern to mirror: `crates/app/src/render/transcript.rs:883-914` (`ScrollbarState::new(content_len).position(pos)`, `Scrollbar::new(ScrollbarOrientation::VerticalRight).render(area, buf, &mut sb_state, state.colors.scrollbar)`; imports at transcript.rs:14).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn selector_list_draws_scrollbar_when_overflowing() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    // A SHORT editor area forces the (long) selector list to overflow.
    let area = Rect::new(0, 0, 120, 12);
    let mut buf = Buffer::empty(area);
    let state = make_style_editor_state_default(); // full selector list (~39 selectors)
    let _ = draw_style_editor(&state, area, &mut buf);
    // The scrollbar occupies the rightmost column of the board pane; assert at least one
    // non-blank cell exists in that column (the scrollbar track/thumb glyphs).
    // Determine the board's right edge the same way the render splits content (board_w).
    let board_right_col = /* board.x + board.width - 1, computed as the render does */;
    let mut drew = false;
    for y in 0..area.height {
        let sym = buf[(board_right_col, y)].symbol();
        if sym != " " && !sym.is_empty() { drew = true; }
    }
    assert!(drew, "selector list must draw a scrollbar when it overflows");
}
```

Note: compute `board_right_col` exactly as the render computes the board rect (reuse the same `board_w`/content split). If asserting a specific column is brittle, instead scan all columns in the right ~1-2 cols of the board pane. Mirror the existing style_editor render-test setup for `make_style_editor_state_default`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app selector_list_draws_scrollbar_when_overflowing`
Expected: FAIL (no scrollbar drawn today).

- [ ] **Step 3: Implement the scrollbar**

Ensure the ratatui scrollbar imports are present at the top of style_editor.rs (add any missing ones):

```rust
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget};
```

After the board's selector rows are drawn (~196), draw a scrollbar on the board's right edge only when the content overflows. Use the already-computed `scroll` offset and the total visual-line count:

```rust
// total visual lines (headers + selectors) and visible row capacity:
let total_lines = visual_lines.len();
let visible_rows = board.height as usize;
if total_lines > visible_rows {
    let sb_area = Rect::new(board.right().saturating_sub(1), board.y, 1, board.height);
    let mut sb_state = ScrollbarState::new(total_lines).position(scroll as usize);
    StatefulWidget::render(
        Scrollbar::new(ScrollbarOrientation::VerticalRight),
        sb_area,
        buf,
        &mut sb_state,
    );
    // Apply the themed scrollbar style: the transcript path passes state.colors.scrollbar via
    // the Scrollbar's style setters — match that exact call shape from transcript.rs:900-907.
}
```

Match the EXACT scrollbar construction/style call used in transcript.rs (it threads `state.colors.scrollbar`); copy that call shape so the new scrollbar is themed identically (do not hard-code colors). If drawing the scrollbar in the board's last column would overwrite selector text, reserve one column: reduce the selector-row draw width by 1 when the scrollbar is shown (compute `scrollbar_visible` before the row loop and subtract 1 from the row text width), so the bar sits in its own gutter — mirror how transcript.rs reserves the scrollbar gutter column.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app selector_list_draws_scrollbar_when_overflowing`
Expected: PASS. Then `cargo test -p app` (full) and `cargo build -p app` → 0 warnings. Confirm the non-overflowing case still renders (no panic, no scrollbar) by also running the existing style_editor render tests: `cargo test -p app style_editor`.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/style_editor.rs
git commit -m "feat(app): style editor — scrollbar on the selector list (themed via scrollbar selector)"
```

---

## Self-Review

**Spec coverage:**
- (a) gate Border focus → Task 1. ✓
- (b) real mini-box glyph → Task 2. ✓
- (c) header/shadow chips per selector → Task 2. ✓
- (d) picture-frame picker no-op → Task 3. ✓
- (e) invalid-pending feedback → Task 3. ✓
- (f) selector-list scrollbar → Task 4. ✓

**Placeholder scan:** The `make_*_for_test` / `buffer_text` / `board_right_col` / `working_decl_for` identifiers are explicitly flagged as placeholders to be replaced by the real existing test setup and accessors (the implementer must locate and reuse them with the given `rg` hints). These are test-scaffolding and accessor-name unknowns, not logic placeholders — the behavior, assertions, and production code are concrete. The default-zone-glyph match in Task 2 is "keep the existing logic, convert to String" — the existing logic is in the file at the named lines.

**Type consistency:** `zone_glyph` returns `String` (Task 2) and its caller formats with `Display` — consistent. `is_bordered_selector(&str) -> bool` used in Tasks 1, 2, 3 with the same signature. `StyleEditorRects.border_header/border_shadow: Option<Rect>` asserted in Task 2 matches the struct at style_editor.rs:33-48. `state.colors.scrollbar` (Task 4) matches the transcript usage.

**Notes for the executor:** Tasks are independent in behavior but Tasks 2 and 4 both edit `render/style_editor.rs` (sequential, no conflict). Task 1 and Task 3 both edit `input.rs` (sequential). Reuse existing test setup/helpers rather than inventing new ones; if a render test helper doesn't exist, inline the buffer setup as the existing render tests in that file do.
