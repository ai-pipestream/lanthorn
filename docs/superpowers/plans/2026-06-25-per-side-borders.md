# Per-Side Pane Borders + Header Decoupling — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each pane border specify its four sides independently and decouple the title/layer-tab header strip from the top border, via a single `draw_framed` helper that the render sites call uniformly.

**Architecture:** `paneframe.rs` grows a `PaneSides` (four per-side `BorderStyle`s), a side-aware `draw_pane_frame_sides`, a borderless `draw_header_plain`, and a unifying `draw_framed` that picks the composited vs per-side path and computes header placement. `colors.rs` carries a resolved `PaneSides` per pane plus header bools; `style.rs` parses/exports the new `style_<side>`/`header` `Decl` keys; the render sites swap `draw_pane_frame`+`draw_top_inset` for `draw_framed`+conditional header.

**Tech Stack:** Rust, ratatui 0.29.

## Global Constraints

- Commit trailers on every commit (body, no backticks anywhere in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Zero compiler warnings; remove any symbol your change orphans.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- `ColorScheme` derives `PartialEq`/`Clone`; every new field type must be `PartialEq`/`Clone`/`Copy` where used in it.
- Per-side values are limited to `none/single/double/thick`; a per-side `picture-frame` warns and falls back to the base style. When base `style = picture-frame`, per-side overrides are ignored (composited whole frame).
- Corner rule: a corner draws only where two present sides meet; mixed styles → heavier wins (`thick > double > single`); a single present side extends its straight glyph (`─/━/═` horizontal, `│/┃/║` vertical); neither → blank.
- Content insets by 1 only on bordered sides (2 all-round for picture-frame).
- Header (`header = true|false`, default `true`) applies to `story_border` (title) and `map_border` (layer-tabs); inert elsewhere. Placement: header-on + top present → strip in the top border row; header-on + top absent → strip on a reclaimed first content row, plain (no border glyphs); header-off → no strip, content uses the full inner area.
- Scope: `map_border`, `story_border`, `status_header`, `input_line`, `upper_window_border`. Dialogs (`dialog`) stay whole-frame — `draw_pane_frame` unchanged for them. All existing `draw_pane_frame` callers keep working.
- `write_style_full` round-trip must stay lossless; existing whole-frame round-trips stay green.
- Run `cargo test -p app` after every task: 0 failures, 0 warnings.

---

### Task 1: PaneSides + side-aware frame drawing

**Files:**
- Modify: `crates/app/src/render/paneframe.rs` (new types/fns after `draw_pane_frame` ~201; tests)

**Interfaces:**
- Produces: `pub struct PaneSides { pub top: BorderStyle, pub bottom: BorderStyle, pub left: BorderStyle, pub right: BorderStyle }` (derives `Debug, Clone, Copy, PartialEq, Eq`) + `PaneSides::all(BorderStyle) -> PaneSides`; `pub fn draw_pane_frame_sides(buf: &mut Buffer, area: Rect, sides: PaneSides, color: Style) -> PaneFrame`.
- Consumes: `SINGLE`/`DOUBLE`/`THICK` glyph consts, `PaneFrame`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/render/paneframe.rs`, inside `mod tests`, add:

```rust
#[test]
fn pane_sides_all_and_left_right_only() {
    let s = PaneSides::all(BorderStyle::Single);
    assert_eq!(s.top, BorderStyle::Single);
    assert_eq!(s.bottom, BorderStyle::Single);

    // left+right only: vertical bars, no corners, content keeps full height.
    let sides = PaneSides { top: BorderStyle::None, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::Single };
    let area = Rect::new(0, 0, 10, 4);
    let mut buf = Buffer::empty(area);
    let frame = draw_pane_frame_sides(&mut buf, area, sides, Style::default());
    // sides present on every row; corners are NOT drawn (no ┌ at 0,0).
    assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "│");
    assert_eq!(buf.cell((9, 0)).unwrap().symbol(), "│");
    assert_eq!(buf.cell((0, 3)).unwrap().symbol(), "│");
    // content inset 1 left + 1 right, full height (top/bottom open).
    assert_eq!(frame.content, Rect::new(1, 0, 8, 4));
    // no top border → top_inset has zero height.
    assert_eq!(frame.top_inset.height, 0);
}

#[test]
fn draw_sides_corner_when_two_meet_and_heavier_wins() {
    // top thick + left single → top-left corner uses the thick corner (heavier wins).
    let sides = PaneSides { top: BorderStyle::Thick, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::None };
    let area = Rect::new(0, 0, 6, 4);
    let mut buf = Buffer::empty(area);
    let frame = draw_pane_frame_sides(&mut buf, area, sides, Style::default());
    assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┏"); // thick tl corner
    // top row interior is the thick horizontal; left side is the single vertical.
    assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "━");
    assert_eq!(buf.cell((0, 2)).unwrap().symbol(), "│");
    // content inset 1 top + 1 left only.
    assert_eq!(frame.content, Rect::new(1, 1, 5, 3));
    // top present → top_inset spans the top row between the left inset and the right edge.
    assert_eq!(frame.top_inset.y, 0);
    assert_eq!(frame.top_inset.height, 1);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app pane_sides_all_and_left_right_only draw_sides_corner_when_two_meet_and_heavier_wins`
Expected: compile error (types/fn missing).

- [ ] **Step 3: Add `PaneSides` and the glyph/corner helpers**

In `crates/app/src/render/paneframe.rs`, after the `draw_pane_frame` function (~201), add:

```rust
/// Per-side border styles for one pane. A side of `None` is omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSides {
    pub top: BorderStyle,
    pub bottom: BorderStyle,
    pub left: BorderStyle,
    pub right: BorderStyle,
}

impl PaneSides {
    /// All four sides set to one style.
    pub fn all(style: BorderStyle) -> PaneSides {
        PaneSides { top: style, bottom: style, left: style, right: style }
    }
}

/// "Weight" used to pick a corner glyph when two adjacent sides differ:
/// thick > double > single > none.
fn border_weight(s: BorderStyle) -> u8 {
    match s {
        BorderStyle::Thick => 3,
        BorderStyle::Double => 2,
        BorderStyle::Single => 1,
        _ => 0, // None / PictureFrame never reach the per-side corner path
    }
}

fn glyphs_for(style: BorderStyle) -> &'static Glyphs {
    match style {
        BorderStyle::Double => &DOUBLE,
        BorderStyle::Thick => &THICK,
        _ => &SINGLE,
    }
}

/// Which corner of the frame, for `corner_glyph`.
#[derive(Clone, Copy)]
enum Corner { Tl, Tr, Bl, Br }

/// The glyph for one corner, given its horizontal (top/bottom) and vertical
/// (left/right) adjacent side styles. Both present → corner glyph of the heavier
/// style; only horizontal → that horizontal glyph; only vertical → that vertical
/// glyph; neither → a space.
fn corner_glyph(h: BorderStyle, v: BorderStyle, which: Corner) -> &'static str {
    let h_on = h != BorderStyle::None;
    let v_on = v != BorderStyle::None;
    match (h_on, v_on) {
        (true, true) => {
            let g = if border_weight(h) >= border_weight(v) { glyphs_for(h) } else { glyphs_for(v) };
            match which { Corner::Tl => g.tl, Corner::Tr => g.tr, Corner::Bl => g.bl, Corner::Br => g.br }
        }
        (true, false) => glyphs_for(h).top,
        (false, true) => glyphs_for(v).side,
        (false, false) => " ",
    }
}
```

- [ ] **Step 4: Add `draw_pane_frame_sides`**

In the same file, after the helpers above, add:

```rust
/// Draw a pane frame with independent per-side styles. Each present side draws
/// its straight glyph; corners resolve via `corner_glyph`; `content` is inset by
/// 1 only on sides that have a border; `top_inset` is the top row (between the
/// left/right insets) only when the top side is present, else zero-height.
pub fn draw_pane_frame_sides(buf: &mut Buffer, area: Rect, sides: PaneSides, color: Style) -> PaneFrame {
    if area.width < 2 || area.height < 2 {
        let top_inset = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        return PaneFrame { area, content: area, top_inset };
    }
    let x = area.x;
    let y = area.y;
    let right = x + area.width - 1;
    let bottom = y + area.height - 1;
    let on = |s: BorderStyle| s != BorderStyle::None;

    // Horizontal runs (top/bottom): straight glyph between the corners.
    if on(sides.top) {
        let g = glyphs_for(sides.top);
        for cx in (x + 1)..right {
            if let Some(c) = buf.cell_mut((cx, y)) { c.set_symbol(g.top).set_style(color); }
        }
    }
    if on(sides.bottom) {
        let g = glyphs_for(sides.bottom);
        for cx in (x + 1)..right {
            if let Some(c) = buf.cell_mut((cx, bottom)) { c.set_symbol(g.top).set_style(color); }
        }
    }
    // Vertical runs (left/right).
    if on(sides.left) {
        let g = glyphs_for(sides.left);
        for cy in (y + 1)..bottom {
            if let Some(c) = buf.cell_mut((x, cy)) { c.set_symbol(g.side).set_style(color); }
        }
    }
    if on(sides.right) {
        let g = glyphs_for(sides.right);
        for cy in (y + 1)..bottom {
            if let Some(c) = buf.cell_mut((right, cy)) { c.set_symbol(g.side).set_style(color); }
        }
    }
    // Corners.
    let set = |buf: &mut Buffer, px: u16, py: u16, sym: &str| {
        if sym != " " {
            if let Some(c) = buf.cell_mut((px, py)) { c.set_symbol(sym).set_style(color); }
        }
    };
    set(buf, x, y, corner_glyph(sides.top, sides.left, Corner::Tl));
    set(buf, right, y, corner_glyph(sides.top, sides.right, Corner::Tr));
    set(buf, x, bottom, corner_glyph(sides.bottom, sides.left, Corner::Bl));
    set(buf, right, bottom, corner_glyph(sides.bottom, sides.right, Corner::Br));

    // Content: inset 1 on each bordered side only.
    let l = if on(sides.left) { 1 } else { 0 };
    let r = if on(sides.right) { 1 } else { 0 };
    let t = if on(sides.top) { 1 } else { 0 };
    let b = if on(sides.bottom) { 1 } else { 0 };
    let content = Rect::new(
        x + l,
        y + t,
        area.width.saturating_sub(l + r),
        area.height.saturating_sub(t + b),
    );

    // top_inset only valid when the top side is present.
    let top_inset = if on(sides.top) {
        let inset_x = x + 1;
        let inset_w = right.saturating_sub(inset_x);
        Rect::new(inset_x, y, inset_w, 1)
    } else {
        Rect::new(x, y, 0, 0)
    };

    PaneFrame { area, content, top_inset }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app pane_sides_all_and_left_right_only draw_sides_corner_when_two_meet_and_heavier_wins`
Expected: PASS.

- [ ] **Step 6: Run the full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/render/paneframe.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): PaneSides + draw_pane_frame_sides (per-side borders + corners)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: Borderless header strip + the unifying `draw_framed`

**Files:**
- Modify: `crates/app/src/render/paneframe.rs` (`draw_header_plain`, `FramedPane`, `draw_framed`; tests)

**Interfaces:**
- Consumes: `draw_pane_frame_sides` (Task 1), `draw_pane_frame`/`draw_picture_frame`, `draw_top_inset`, `InsetSegment`.
- Produces: `pub fn draw_header_plain(buf, row: Rect, segments: &[InsetSegment], base: Style, active: Style) -> Vec<Rect>`; `pub struct FramedPane { pub content: Rect, pub header: Option<Rect>, pub header_bordered: bool }`; `pub fn draw_framed(buf, area: Rect, base: BorderStyle, sides: PaneSides, color: Style, header_on: bool) -> FramedPane`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/render/paneframe.rs`, inside `mod tests`, add:

```rust
#[test]
fn draw_header_plain_centers_without_brackets() {
    let row = Rect::new(0, 0, 11, 1);
    let mut buf = Buffer::empty(Rect::new(0, 0, 11, 1));
    let rects = draw_header_plain(&mut buf, row, &[InsetSegment { text: "AB", active: false }], Style::default(), Style::default());
    let line: String = (0..11u16).map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
    // "AB" centered, no ┫/┣/┃ brackets anywhere.
    assert!(line.contains("AB"), "got {:?}", line);
    assert!(!line.contains('┫') && !line.contains('┣') && !line.contains('┃'), "no brackets: {:?}", line);
    assert_eq!(rects.len(), 1);
}

#[test]
fn draw_framed_header_placement_matrix() {
    let area = Rect::new(0, 0, 12, 6);

    // header on + top present → header is the top border row (bordered).
    let mut b1 = Buffer::empty(area);
    let f1 = draw_framed(&mut b1, area, BorderStyle::Single, PaneSides::all(BorderStyle::Single), Style::default(), true);
    assert!(f1.header_bordered);
    assert_eq!(f1.header.unwrap().y, 0);
    assert_eq!(f1.content, Rect::new(1, 1, 10, 4));

    // header on + top none → header on reclaimed first content row (plain); content drops a row.
    let sides_no_top = PaneSides { top: BorderStyle::None, bottom: BorderStyle::Single, left: BorderStyle::Single, right: BorderStyle::Single };
    let mut b2 = Buffer::empty(area);
    let f2 = draw_framed(&mut b2, area, BorderStyle::None, sides_no_top, Style::default(), true);
    assert!(!f2.header_bordered);
    let h2 = f2.header.unwrap();
    assert_eq!(h2.height, 1);
    // content starts one row below the header row.
    assert_eq!(f2.content.y, h2.y + 1);

    // header off → no header; content uses the inner area.
    let mut b3 = Buffer::empty(area);
    let f3 = draw_framed(&mut b3, area, BorderStyle::Single, PaneSides::all(BorderStyle::Single), Style::default(), false);
    assert!(f3.header.is_none());
    assert_eq!(f3.content, Rect::new(1, 1, 10, 4));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app draw_header_plain_centers_without_brackets draw_framed_header_placement_matrix`
Expected: compile error (fns/struct missing).

- [ ] **Step 3: Add `draw_header_plain`**

In `crates/app/src/render/paneframe.rs`, add:

```rust
/// Draw header segments centered on a single plain row (no border brackets),
/// returning per-segment hit-rects. Used for a borderless header (top side off).
/// Segments are joined with two spaces; the active one uses `active` style.
pub fn draw_header_plain(buf: &mut Buffer, row: Rect, segments: &[InsetSegment], base: Style, active: Style) -> Vec<Rect> {
    if segments.is_empty() || row.width == 0 || row.height == 0 {
        return segments.iter().map(|_| Rect::default()).collect();
    }
    let widths: Vec<usize> = segments.iter().map(|s| s.text.chars().count()).collect();
    let sep = 2usize;
    let total: usize = widths.iter().sum::<usize>() + sep * segments.len().saturating_sub(1);
    let avail = row.width as usize;
    let leading = if total <= avail { (avail - total) / 2 } else { 0 };
    let mut cx = row.x + leading as u16;
    let mut rects = vec![Rect::default(); segments.len()];
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            for _ in 0..sep {
                if let Some(c) = buf.cell_mut((cx, row.y)) { c.set_symbol(" ").set_style(base); }
                cx += 1;
            }
        }
        let style = if seg.active { active } else { base };
        let start = cx;
        for ch in seg.text.chars() {
            if let Some(c) = buf.cell_mut((cx, row.y)) { c.set_symbol(&ch.to_string()).set_style(style); }
            cx += 1;
        }
        rects[i] = Rect::new(start, row.y, cx - start, 1);
    }
    rects
}
```

- [ ] **Step 4: Add `FramedPane` and `draw_framed`**

In the same file, add:

```rust
/// The result of drawing a pane frame: where content goes, and (optionally) where
/// the header strip should be drawn and whether that row is a border row.
#[derive(Debug, Clone, Copy)]
pub struct FramedPane {
    pub content: Rect,
    /// Where to draw the header strip, or `None` when no header is shown.
    pub header: Option<Rect>,
    /// True when `header` is a top border row (use `draw_top_inset`); false when it
    /// is a reclaimed plain content row (use `draw_header_plain`).
    pub header_bordered: bool,
}

/// Draw a pane frame choosing the composited path for `picture-frame` or the
/// per-side path otherwise, and resolve header placement from `header_on`.
pub fn draw_framed(buf: &mut Buffer, area: Rect, base: BorderStyle, sides: PaneSides, color: Style, header_on: bool) -> FramedPane {
    if base == BorderStyle::PictureFrame {
        let frame = draw_pane_frame(buf, area, BorderStyle::PictureFrame, color);
        return FramedPane {
            content: frame.content,
            header: if header_on { Some(frame.top_inset) } else { None },
            header_bordered: true,
        };
    }
    let frame = draw_pane_frame_sides(buf, area, sides, color);
    let top_present = sides.top != BorderStyle::None;
    if !header_on {
        FramedPane { content: frame.content, header: None, header_bordered: false }
    } else if top_present {
        FramedPane { content: frame.content, header: Some(frame.top_inset), header_bordered: true }
    } else {
        // Reclaim the first content row for a plain header; content drops one row.
        let c = frame.content;
        if c.height == 0 {
            FramedPane { content: c, header: None, header_bordered: false }
        } else {
            let header = Rect::new(c.x, c.y, c.width, 1);
            let content = Rect::new(c.x, c.y + 1, c.width, c.height - 1);
            FramedPane { content, header: Some(header), header_bordered: false }
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app draw_header_plain_centers_without_brackets draw_framed_header_placement_matrix`
Expected: PASS.

- [ ] **Step 6: Run the full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/render/paneframe.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): draw_header_plain + draw_framed (path select + header placement)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: ColorScheme — per-side fields + header bools

**Files:**
- Modify: `crates/app/src/colors.rs` (`ColorScheme` fields; `terminal_default` ~337; `from_ghostty` ~474; tests)

**Interfaces:**
- Consumes: `PaneSides` (Task 1).
- Produces: `ColorScheme.{map_border_sides, story_border_sides, status_header_sides, input_line_sides, upper_window_border_sides}: PaneSides`; `ColorScheme.{story_header_on, map_header_on}: bool`. Defaults: each `*_sides = PaneSides::all(*_style)`; headers `true`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/colors.rs`, inside `mod tests`, add:

```rust
#[test]
fn border_sides_default_to_all_of_base_and_headers_on() {
    use crate::render::paneframe::{PaneSides, BorderStyle};
    let cs = ColorScheme::terminal_default();
    assert_eq!(cs.map_border_sides, PaneSides::all(cs.map_border_style));
    assert_eq!(cs.story_border_sides, PaneSides::all(cs.story_border_style));
    assert_eq!(cs.status_header_sides, PaneSides::all(cs.status_header_style));
    assert_eq!(cs.input_line_sides, PaneSides::all(cs.input_line_style));
    assert_eq!(cs.upper_window_border_sides, PaneSides::all(cs.virtual_window_border));
    assert!(cs.story_header_on);
    assert!(cs.map_header_on);
    // None base → all sides None.
    assert_eq!(cs.map_border_sides, PaneSides::all(BorderStyle::None));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app border_sides_default_to_all_of_base_and_headers_on`
Expected: compile error (fields missing).

- [ ] **Step 3: Add the fields**

In `crates/app/src/colors.rs`, the import line already has `use crate::render::paneframe::BorderStyle;`. Add `PaneSides` to it:

```rust
use crate::render::paneframe::{BorderStyle, PaneSides};
```

In the `ColorScheme` struct, after `pub virtual_window_border: BorderStyle,` (~276) add:

```rust
    /// Per-side border styles (default = all of the matching base `*_style`).
    pub map_border_sides: PaneSides,
    pub story_border_sides: PaneSides,
    pub status_header_sides: PaneSides,
    pub input_line_sides: PaneSides,
    pub upper_window_border_sides: PaneSides,
    /// Whether the story title / map layer-tab header strip is shown.
    pub story_header_on: bool,
    pub map_header_on: bool,
```

- [ ] **Step 4: Set defaults in `terminal_default`**

In `terminal_default`, after `virtual_window_border: BorderStyle::Single,` add:

```rust
            map_border_sides: PaneSides::all(BorderStyle::None),
            story_border_sides: PaneSides::all(BorderStyle::None),
            status_header_sides: PaneSides::all(BorderStyle::None),
            input_line_sides: PaneSides::all(BorderStyle::None),
            upper_window_border_sides: PaneSides::all(BorderStyle::Single),
            story_header_on: true,
            map_header_on: true,
```

(These match the existing base `*_style` defaults: map/story/status/input = None, upper window = Single.)

- [ ] **Step 5: Set defaults in `from_ghostty`**

In `from_ghostty`, after its `virtual_window_border: BorderStyle::Single,` add the identical block:

```rust
            map_border_sides: PaneSides::all(BorderStyle::None),
            story_border_sides: PaneSides::all(BorderStyle::None),
            status_header_sides: PaneSides::all(BorderStyle::None),
            input_line_sides: PaneSides::all(BorderStyle::None),
            upper_window_border_sides: PaneSides::all(BorderStyle::Single),
            story_header_on: true,
            map_header_on: true,
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/colors.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): per-side PaneSides + header bools on ColorScheme

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: Parse + apply per-side / header keys

**Files:**
- Modify: `crates/app/src/style.rs` (`Decl` struct; `parse_decl_from_table`; `apply_color_decls` border arms ~193; tests)

**Interfaces:**
- Consumes: `paneframe::{PaneSides, parse_border_style, BorderStyle}`; the new `ColorScheme` fields (Task 3).
- Produces: `Decl.{style_top, style_bottom, style_left, style_right: Option<String>, header: Option<bool>}`; `apply_color_decls` computes each pane's `*_sides` and the header bools; a helper `resolve_sides(base: BorderStyle, decl: &Decl) -> (PaneSides, Vec<String>)`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn per_side_overrides_and_header_apply() {
    use crate::render::paneframe::BorderStyle;
    let doc = parse_style_toml(
        "[colors]\n\
         \"map_border\" = { style = \"none\", style_left = \"single\", style_right = \"single\" }\n\
         \"story_border\" = { style = \"single\", style_top = \"thick\", header = false }\n"
    ).unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    assert!(warnings.is_empty(), "{warnings:?}");
    // map: base none, left/right single.
    assert_eq!(cs.map_border_sides.top, BorderStyle::None);
    assert_eq!(cs.map_border_sides.left, BorderStyle::Single);
    assert_eq!(cs.map_border_sides.right, BorderStyle::Single);
    // story: base single, top thick, header off.
    assert_eq!(cs.story_border_sides.top, BorderStyle::Thick);
    assert_eq!(cs.story_border_sides.left, BorderStyle::Single);
    assert!(!cs.story_header_on);
}

#[test]
fn per_side_picture_frame_warns_and_falls_back() {
    let doc = parse_style_toml(
        "[colors]\n\"map_border\" = { style = \"single\", style_top = \"picture-frame\" }\n"
    ).unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    use crate::render::paneframe::BorderStyle;
    // invalid per-side picture-frame → falls back to base (single) + warns.
    assert_eq!(cs.map_border_sides.top, BorderStyle::Single);
    assert!(warnings.iter().any(|w| w.contains("picture-frame")), "{warnings:?}");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app per_side_overrides_and_header_apply per_side_picture_frame_warns_and_falls_back`
Expected: compile error / assertion failure (fields + logic missing).

- [ ] **Step 3: Add the `Decl` fields**

In `crates/app/src/style.rs`, in the `Decl` struct, after the `style` field add:

```rust
    /// Per-side border overrides (border selectors only): each names a line style
    /// (none/single/double/thick). A side falls back to `style` when unset.
    #[serde(default)]
    pub style_top: Option<String>,
    #[serde(default)]
    pub style_bottom: Option<String>,
    #[serde(default)]
    pub style_left: Option<String>,
    #[serde(default)]
    pub style_right: Option<String>,
    /// Whether the pane's header strip is shown (story_border / map_border only).
    #[serde(default)]
    pub header: Option<bool>,
```

- [ ] **Step 4: Read them in `parse_decl_from_table`**

In `parse_decl_from_table`, add to the returned `Decl { … }`:

```rust
        style_top:    t.get("style_top").and_then(toml::Value::as_str).map(str::to_string),
        style_bottom: t.get("style_bottom").and_then(toml::Value::as_str).map(str::to_string),
        style_left:   t.get("style_left").and_then(toml::Value::as_str).map(str::to_string),
        style_right:  t.get("style_right").and_then(toml::Value::as_str).map(str::to_string),
        header:       t.get("header").and_then(toml::Value::as_bool),
```

- [ ] **Step 4b: Complete the other `Decl` literals (so the crate compiles)**

Adding fields to `Decl` breaks the two other full `Decl { … }` literals. Update both.

In `merge_decl`, add per-field merge for the new fields (each `over.or(base)`):

```rust
        style_top:    over.style_top.clone().or(base.style_top.clone()),
        style_bottom: over.style_bottom.clone().or(base.style_bottom.clone()),
        style_left:   over.style_left.clone().or(base.style_left.clone()),
        style_right:  over.style_right.clone().or(base.style_right.clone()),
        header:       over.header.or(base.header),
```

In `style_to_decl` (the color-only inverse), initialize the new fields to `None` — the border export sets them explicitly later:

```rust
        style_top: None,
        style_bottom: None,
        style_left: None,
        style_right: None,
        header: None,
```

- [ ] **Step 5: Add the `resolve_sides` helper**

In `crates/app/src/style.rs`, near `apply_color_decls`, add:

```rust
/// Resolve a base border style + per-side overrides into a `PaneSides`. Each side
/// uses its `style_<side>` override (parsed as a line style) or falls back to
/// `base`. A per-side value of `picture-frame` is invalid → warns, uses `base`.
fn resolve_sides(base: paneframe::BorderStyle, decl: &Decl) -> (paneframe::PaneSides, Vec<String>) {
    let mut warnings = Vec::new();
    let side = |ov: &Option<String>, warnings: &mut Vec<String>| -> paneframe::BorderStyle {
        match ov {
            None => base,
            Some(s) => {
                if s == "picture-frame" {
                    warnings.push(format!("per-side 'picture-frame' is invalid; using base style"));
                    base
                } else {
                    paneframe::parse_border_style(s)
                }
            }
        }
    };
    let sides = paneframe::PaneSides {
        top: side(&decl.style_top, &mut warnings),
        bottom: side(&decl.style_bottom, &mut warnings),
        left: side(&decl.style_left, &mut warnings),
        right: side(&decl.style_right, &mut warnings),
    };
    (sides, warnings)
}
```

- [ ] **Step 6: Compute sides + header in `apply_color_decls`**

In `apply_color_decls`, the existing border arms set `cs.*_style = parse_border_style(s)` from `decl.style`. Extend each of the five border arms to also resolve sides + header. Replace the `"map_border"`, `"story_border"`, `"status_header"`, `"input_line"`, and `"upper_window_border"` arms with the versions below (the others are unchanged). Use the base that each arm already computes; when `decl.style` is absent, base = the field's current value.

```rust
            "map_border" => {
                cs.map_border = cs.map_border.patch(style);
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.map_border_style);
                cs.map_border_style = base;
                let (sides, w) = resolve_sides(base, decl); warnings.extend(w);
                cs.map_border_sides = sides;
                if let Some(h) = decl.header { cs.map_header_on = h; }
            }
            "story_border" => {
                cs.story_border = cs.story_border.patch(style);
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.story_border_style);
                cs.story_border_style = base;
                let (sides, w) = resolve_sides(base, decl); warnings.extend(w);
                cs.story_border_sides = sides;
                if let Some(h) = decl.header { cs.story_header_on = h; }
            }
            "status_header" => {
                cs.status_header = cs.status_header.patch(style);
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.status_header_style);
                cs.status_header_style = base;
                let (sides, w) = resolve_sides(base, decl); warnings.extend(w);
                cs.status_header_sides = sides;
            }
            "input_line" => {
                cs.input_line = cs.input_line.patch(style);
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.input_line_style);
                cs.input_line_style = base;
                let (sides, w) = resolve_sides(base, decl); warnings.extend(w);
                cs.input_line_sides = sides;
            }
            "upper_window_border" => {
                cs.upper_window_border = cs.upper_window_border.patch(style);
                let base = decl.style.as_deref().map(paneframe::parse_border_style).unwrap_or(cs.virtual_window_border);
                cs.virtual_window_border = base;
                let (sides, w) = resolve_sides(base, decl); warnings.extend(w);
                cs.upper_window_border_sides = sides;
            }
```

Note: keep these arms' relative position; remove the OLD versions of these five arms so there is no duplicate match arm.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (`resolve_sides` warning string contains "picture-frame" — the test matches on it.)

- [ ] **Step 8: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): parse + apply per-side / header border keys

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: Export per-side + header in write_style_full

**Files:**
- Modify: `crates/app/src/style.rs` (`write_style_full` border-selector export; `write_style` inline-table emit; tests)

**Interfaces:**
- Consumes: `cs.*_sides`, `cs.*_header_on`, `paneframe::border_style_name`, `style_to_decl`.
- Produces: each border selector's exported `Decl` carries `style_<side>` for any side differing from the base and `header = false` when off; `write_style` emits those keys.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn write_style_full_round_trips_per_side_and_header() {
    use crate::render::paneframe::{BorderStyle, PaneSides};
    let dir = std::env::temp_dir().join(format!("babelmap-ps-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ps.toml");

    let mut cs = crate::colors::ColorScheme::terminal_default();
    // map: base none, left/right single.
    cs.map_border_style = BorderStyle::None;
    cs.map_border_sides = PaneSides { top: BorderStyle::None, bottom: BorderStyle::None, left: BorderStyle::Single, right: BorderStyle::Single };
    // story: base single, top thick, header off.
    cs.story_border_style = BorderStyle::Single;
    cs.story_border_sides = PaneSides { top: BorderStyle::Thick, bottom: BorderStyle::Single, left: BorderStyle::Single, right: BorderStyle::Single };
    cs.story_header_on = false;

    let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
    write_style_full(&path, &cs, &set).unwrap();
    let doc = parse_style_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let (cs2, _set2, _w) = resolve(&doc, &dir);

    assert_eq!(cs2.map_border_sides.left, BorderStyle::Single);
    assert_eq!(cs2.map_border_sides.top, BorderStyle::None);
    assert_eq!(cs2.story_border_sides.top, BorderStyle::Thick);
    assert!(!cs2.story_header_on);
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app write_style_full_round_trips_per_side_and_header`
Expected: FAIL — per-side/header not exported, so `cs2` loses them.

- [ ] **Step 3: Emit the new keys in `write_style`**

In `write_style`, in the selector inline-table loop (where `style`/`fg`/`bg`/… are inserted), after the `style` insert add:

```rust
            if let Some(st) = &decl.style_top    { itbl.insert("style_top",    toml_edit::Value::from(st.as_str())); }
            if let Some(st) = &decl.style_bottom { itbl.insert("style_bottom", toml_edit::Value::from(st.as_str())); }
            if let Some(st) = &decl.style_left   { itbl.insert("style_left",   toml_edit::Value::from(st.as_str())); }
            if let Some(st) = &decl.style_right  { itbl.insert("style_right",  toml_edit::Value::from(st.as_str())); }
            if decl.header == Some(false)        { itbl.insert("header",       toml_edit::Value::from(false)); }
```

- [ ] **Step 4: Populate the per-side/header overrides in `write_style_full`**

In `write_style_full`, the five border selectors are exported via a `Decl` whose `style` is set to the base border name. Add a helper that decorates that `Decl` with per-side overrides + header, and use it for the five panes. After `style_to_decl` is used for each border selector, set its side/header fields. Replace the existing `map_border`/`story_border` decl-building blocks and the `status_header`/`input_line`/`upper_window_border` ones with these (each builds `d`, sets base `style`, then per-side + header):

```rust
    // Helper: set style_<side> on a Decl for any side that differs from `base`.
    fn decorate_sides(d: &mut Decl, base: crate::render::paneframe::BorderStyle, sides: crate::render::paneframe::PaneSides) {
        use crate::render::paneframe::border_style_name;
        if sides.top != base    { d.style_top    = Some(border_style_name(sides.top).to_string()); }
        if sides.bottom != base { d.style_bottom = Some(border_style_name(sides.bottom).to_string()); }
        if sides.left != base   { d.style_left   = Some(border_style_name(sides.left).to_string()); }
        if sides.right != base  { d.style_right  = Some(border_style_name(sides.right).to_string()); }
    }
    {
        let mut d = style_to_decl(&cs.map_border);
        d.style = Some(paneframe::border_style_name(cs.map_border_style).to_string());
        decorate_sides(&mut d, cs.map_border_style, cs.map_border_sides);
        if !cs.map_header_on { d.header = Some(false); }
        doc.colors.selectors.insert("map_border".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.story_border);
        d.style = Some(paneframe::border_style_name(cs.story_border_style).to_string());
        decorate_sides(&mut d, cs.story_border_style, cs.story_border_sides);
        if !cs.story_header_on { d.header = Some(false); }
        doc.colors.selectors.insert("story_border".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.status_header);
        if cs.status_header_style != paneframe::BorderStyle::None {
            d.style = Some(paneframe::border_style_name(cs.status_header_style).to_string());
        }
        decorate_sides(&mut d, cs.status_header_style, cs.status_header_sides);
        doc.colors.selectors.insert("status_header".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.input_line);
        if cs.input_line_style != paneframe::BorderStyle::None {
            d.style = Some(paneframe::border_style_name(cs.input_line_style).to_string());
        }
        decorate_sides(&mut d, cs.input_line_style, cs.input_line_sides);
        doc.colors.selectors.insert("input_line".to_string(), d);
    }
    {
        let mut d = style_to_decl(&cs.upper_window_border);
        d.style = Some(paneframe::border_style_name(cs.virtual_window_border).to_string());
        decorate_sides(&mut d, cs.virtual_window_border, cs.upper_window_border_sides);
        doc.colors.selectors.insert("upper_window_border".to_string(), d);
    }
```

Delete the OLD blocks that inserted these five selectors (above) so each is inserted exactly once. (`style_to_decl` already initializes the new `Decl` fields to `None` from Task 4, so the exported decls start clean and only the border export decorates them.)

- [ ] **Step 5: Run the test + full suite to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (`write_style_full_is_self_contained` and the other round-trips stay green: uniform sides export only the base `style`; default headers are on, so no `header` key is written.)

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): export per-side + header border keys in write_style_full

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: Render integration — use draw_framed at the pane sites

**Files:**
- Modify: `crates/app/src/main.rs` (`draw_frame`: story/map pane border + title/tabs at ~299/323/350/372)
- Modify: `crates/app/src/render/transcript.rs` (`render_transcript`: status_header + input_line frames ~389/401)
- Modify: `crates/app/src/render/upper_window.rs` (the virtual-window frame)

**Interfaces:**
- Consumes: `paneframe::{draw_framed, draw_header_plain, draw_top_inset, FramedPane}`; `cs.*_sides`, `cs.*_style`, `cs.story_header_on`, `cs.map_header_on`.
- Produces: each pane drawn via `draw_framed`; header strips via `draw_top_inset` (bordered) or `draw_header_plain` (plain); layer-tab hit-rects preserved.

This task is integration: it swaps `draw_pane_frame(...) + draw_top_inset(...)` for `draw_framed(...) + conditional header` at each pane site. The transformation is mechanical and identical in shape at every site. Apply it to the story pane, the map pane, the status_header frame, the input_line frame, and the upper-window frame. After it, run a render smoke test.

- [ ] **Step 1: Add the import in main.rs**

In `crates/app/src/main.rs`, extend the paneframe import (~36) to include the new items:

```rust
use app::render::paneframe::{build_layer_segments, draw_framed, draw_header_plain, draw_pane_frame, draw_top_inset, FramedPane, InsetSegment, PaneSides};
```

(Keep `draw_pane_frame` — dialogs still use it.)

- [ ] **Step 2: Story pane — split layout (~299)**

The current code draws the story frame with `draw_pane_frame(buf, story_region, state.colors.story_border_style, story_border_style)` then `draw_top_inset(buf, story_frame.top_inset, &[InsetSegment{ text:&state.title, active:false }], state.colors.story_title, state.colors.story_title);`. Replace the frame draw + the title inset with:

```rust
                let story_fp = draw_framed(buf, story_region, state.colors.story_border_style, state.colors.story_border_sides, story_border_style, state.colors.story_header_on);
                if let Some(hrect) = story_fp.header {
                    let segs = [InsetSegment { text: &state.title, active: false }];
                    if story_fp.header_bordered {
                        draw_top_inset(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    } else {
                        draw_header_plain(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    }
                }
```

Use `story_fp.content` everywhere the old `story_frame.content` was used for the story body. (`story_border_style` here is the resolved color `Style` already in scope at this site; keep passing it as the color.)

- [ ] **Step 3: Map pane — split layout (~323)**

The current code draws the map frame then `let tab_rects = draw_top_inset(buf, frame.top_inset, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active); layer_tabs_out = layer_ids.into_iter().zip(tab_rects).collect();`. Replace the frame draw + tab inset with:

```rust
                let map_fp = draw_framed(buf, map_region, state.colors.map_border_style, state.colors.map_border_sides, map_border_color, state.colors.map_header_on);
                if let Some(hrect) = map_fp.header {
                    let tab_rects = if map_fp.header_bordered {
                        draw_top_inset(buf, hrect, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active)
                    } else {
                        draw_header_plain(buf, hrect, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active)
                    };
                    layer_tabs_out = layer_ids.into_iter().zip(tab_rects).collect();
                }
```

Use `map_fp.content` for the map body. (`map_region` and `map_border_color` are the area + resolved color already at this site; match the existing variable names — if the local is named differently, e.g. `frame`/`map_area`, keep those names and only change the frame call + header block.)

- [ ] **Step 4: Story + map panes — full layout (~350 / ~372)**

Apply the identical transformation (Steps 2 and 3) to the second pair of call sites in the full-screen layout branch. Same code shape; only the surrounding `area` variable names differ — keep them.

- [ ] **Step 5: status_header + input_line frames (render/transcript.rs ~389/401)**

`render_transcript` boxes the status header and input line when their `*_style != None`. Replace each `draw_pane_frame(buf, <region>, <style_kind>, <color>)` with the per-side path (these panes have no header, so pass `header_on = false` and use `.content`):

For the status header:
```rust
        let frame = draw_framed(buf, status_region, status_style_kind, state.colors.status_header_sides, state.colors.status_header, false);
        render_status_content(machine, state, buf, frame.content);
```
For the input line:
```rust
        let frame = draw_framed(buf, input_region, input_style_kind, state.colors.input_line_sides, state.colors.input_line, false);
        render_input_content(machine, state, buf, frame.content, normal_style);
```
Add `draw_framed` to the paneframe `use` in `transcript.rs` (it currently imports `draw_pane_frame, BorderStyle`): `use crate::render::paneframe::{draw_framed, draw_pane_frame, BorderStyle};`.

- [ ] **Step 6: Upper-window frame (render/upper_window.rs)**

Find the `draw_pane_frame(buf, area, cs.virtual_window_border, cs.upper_window_border)` call and replace it with:
```rust
    let frame = draw_framed(buf, area, cs.virtual_window_border, cs.upper_window_border_sides, cs.upper_window_border, false);
```
(import `draw_framed` in that file's paneframe `use`), and use `frame.content` where the old frame content was used.

- [ ] **Step 7: Write a render smoke test**

In `crates/app/src/render/transcript.rs`, inside `mod tests`, add:

```rust
#[test]
fn status_header_left_right_only_draws_side_bars_no_top() {
    let machine = minimal_machine();
    let mut state = AppState::default();
    // base none, left/right single, large enough to box.
    state.colors.status_header_style = crate::render::paneframe::BorderStyle::None;
    state.colors.status_header_sides = crate::render::paneframe::PaneSides {
        top: crate::render::paneframe::BorderStyle::None,
        bottom: crate::render::paneframe::BorderStyle::None,
        left: crate::render::paneframe::BorderStyle::Single,
        right: crate::render::paneframe::BorderStyle::Single,
    };
    let area = Rect::new(0, 0, 40, 12);
    let mut buf = Buffer::empty(area);
    render_transcript(&machine, &state, area, &mut buf);
    // A side bar should appear in column 0 somewhere in the status region (rows 0..3),
    // and no top corner glyph at (0,0).
    assert_ne!(buf.cell((0, 0)).unwrap().symbol(), "┌");
}
```

- [ ] **Step 8: Run the full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. If a render call site used a differently-named local for the area/color/frame, adapt the variable names (the transformation shape is fixed; only identifiers vary).

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/main.rs crates/app/src/render/transcript.rs crates/app/src/render/upper_window.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): draw panes via draw_framed (per-side borders + header decoupling)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: Document in style.example.toml

**Files:**
- Modify: `style.example.toml` (repo root — extend the border selectors with per-side / header examples)

**Interfaces:** none (doc only; guarded by the existing `style_example_toml_parses_and_resolves_clean` test).

- [ ] **Step 1: Add commented examples**

In `style.example.toml`, under the map/story border lines, add commented per-side / header examples:

```toml
# Per-side borders: override individual sides (none/single/double/thick); a side
# falls back to `style`. `header = false` hides the title / layer-tab strip.
# "map_border"  = { style = "none", style_left = "single", style_right = "single" }
# "story_border" = { style = "single", style_top = "thick", header = false }
```

- [ ] **Step 2: Run the guard test**

Run: `cargo test -p app style_example_toml_parses_and_resolves_clean`
Expected: PASS (commented lines don't change resolution; the file still parses clean).

- [ ] **Step 3: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add style.example.toml
git -C /Volumes/Videos/Source/babelmap commit -m "docs: per-side border + header examples in style.example.toml

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Dependency order: 1 (sides drawing) → 2 (header + draw_framed) → 3 (colors fields) → 4 (parse/apply) → 5 (export) → 6 (render integration) → 7 (docs). Each ends green (`cargo test -p app`, 0 warnings) before committing.
- Task 6 is the integration task: the same `draw_pane_frame + draw_top_inset` → `draw_framed + conditional header` transformation at every pane site. The pure logic it relies on is fully unit-tested in Tasks 1–2; the smoke test guards wiring. If a call site's local variable names differ from those shown, keep the existing names and change only the frame call + header block.
- Dialogs keep using `draw_pane_frame` (whole-frame) — do not touch the dialog render.
- `README.md` is committed; `TODO.md` is gitignored — never stage it.
