# Status Bar Styling — Configurable Segment Bar — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fixed reversed-video status line with a configurable segment bar — a `[statusbar]` block of ordered, placeholder-driven segments in three alignment clusters — plus a self-contained `write_style_full` export and a documented `style.example.toml`.

**Architecture:** A new top-level `[statusbar]` block parses into `RawStatusBar`/`RawSegment` (style.rs), resolves into a `StatusBarLayout` of `StatusSegment`s stored on `ColorScheme` (colors.rs), and renders via two pure helpers — placeholder resolution and three-cluster packing — that `render_status_content` (render/transcript.rs) drives. Zero config falls back to a built-in default layout that reproduces today's bar exactly.

**Tech Stack:** Rust, ratatui 0.29, `toml` (parse), `toml_edit` (format-preserving write).

## Global Constraints

- Commit trailers on every commit (body, no backticks anywhere in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Zero compiler warnings; remove any symbol your change orphans.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- `ColorScheme` derives `PartialEq`/`Clone` — every new field type must be `PartialEq`/`Clone` so the derive holds.
- Zero `[statusbar]` config must render byte-for-byte like today's bar: location flush-left; `Score: {score}  Moves: {moves}` or `HH:MM` flush-right; ` [filter: story]`/` [filter: meta]` at the far right; reversed-video base; no frame.
- Placeholders: `{location} {score} {moves} {time} {turns} {title} {filter}`; unknown token → empty string (never literal braces). `{turns}` = `AppState.turns`; `{moves}` = the game's `ScoreTurns.turns`.
- Visibility: a segment with NO placeholder is always shown; a segment WITH placeholder(s) is shown iff at least one resolves non-empty.
- Truncation order: drop center cluster → truncate left cluster → preserve right cluster (clip right only if nothing else fits).
- `write_style_full` must export BOTH the `[statusbar]` segments AND the `[[transcript.rule]]` array; the frame round-trips via the existing `status_header` selector export (do NOT re-emit it in the statusbar block).
- Run `cargo test -p app` after every task: 0 failures, 0 warnings.

---

### Task 1: ColorScheme data model — Align, StatusSegment, StatusBarLayout

**Files:**
- Modify: `crates/app/src/colors.rs` (new types after `CompiledRule` ~26; `ColorScheme` field after `transcript_rules:` ~243; defaults in `terminal_default` ~305 and `from_ghostty` ~441; tests)

**Interfaces:**
- Produces: `pub enum Align { Left, Center, Right }` (derives `Debug, Clone, Copy, PartialEq, Eq`); `pub struct StatusSegment { pub text: String, pub align: Align, pub style: Style }` (derives `Debug, Clone, PartialEq`); `pub struct StatusBarLayout { pub segments: Vec<StatusSegment> }` (derives `Debug, Clone, PartialEq`) with a `Default` that is the **built-in default segment set**; `ColorScheme.statusbar_layout: StatusBarLayout`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/colors.rs`, inside `mod tests`, add:

```rust
#[test]
fn statusbar_layout_default_reproduces_today() {
    let l = StatusBarLayout::default();
    // location (left), Score/Moves (right), time (right), filter (right).
    assert_eq!(l.segments.len(), 4);
    assert_eq!(l.segments[0].text, "{location}");
    assert!(matches!(l.segments[0].align, Align::Left));
    assert_eq!(l.segments[1].text, "Score: {score}  Moves: {moves}");
    assert!(matches!(l.segments[1].align, Align::Right));
    assert_eq!(l.segments[2].text, "{time}");
    assert_eq!(l.segments[3].text, " {filter}");
    // All built-in segments carry no per-segment override (render in base style).
    assert!(l.segments.iter().all(|s| s.style == Style::default()));
    // ColorScheme carries the default layout.
    assert_eq!(ColorScheme::terminal_default().statusbar_layout, StatusBarLayout::default());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app statusbar_layout_default_reproduces_today`
Expected: compile error (types/field missing).

- [ ] **Step 3: Define the types**

In `crates/app/src/colors.rs`, after the `CompiledRule` block (and its `impl PartialEq`), add:

```rust
/// Which alignment cluster a status-bar segment belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// One resolved status-bar segment: a text template, its cluster, and the style
/// patched over the base `statusbar` style.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSegment {
    pub text: String,
    pub align: Align,
    pub style: Style,
}

/// The ordered list of status-bar segments. `Default` is the built-in layout that
/// reproduces today's bar (location left; score/moves or clock right; filter right).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusBarLayout {
    pub segments: Vec<StatusSegment>,
}

impl Default for StatusBarLayout {
    fn default() -> Self {
        let seg = |text: &str, align: Align| StatusSegment {
            text: text.to_string(),
            align,
            style: Style::default(),
        };
        StatusBarLayout {
            segments: vec![
                seg("{location}", Align::Left),
                seg("Score: {score}  Moves: {moves}", Align::Right),
                seg("{time}", Align::Right),
                seg(" {filter}", Align::Right),
            ],
        }
    }
}
```

- [ ] **Step 4: Add the `ColorScheme` field**

In the `ColorScheme` struct, after `pub transcript_rules: Vec<CompiledRule>,` add:

```rust
    /// The status-bar segment layout (default reproduces today's bar).
    pub statusbar_layout: StatusBarLayout,
```

- [ ] **Step 5: Set the default in both constructors**

In `terminal_default`, after `transcript_rules: Vec::new(),` add:

```rust
            statusbar_layout: StatusBarLayout::default(),
```

In `from_ghostty`, after its `transcript_rules: Vec::new(),` add the same line:

```rust
            statusbar_layout: StatusBarLayout::default(),
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (Existing `ColorScheme` equality/round-trip tests stay green: both sides carry the identical default layout.)

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/colors.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): StatusBarLayout data model on ColorScheme

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: Parse `[statusbar]` — RawStatusBar/RawSegment, StyleDoc field, merge

**Files:**
- Modify: `crates/app/src/style.rs` (`RawSegment`/`RawStatusBar` near `RawRule` ~309; `StyleDoc` field ~302; `parse_style_toml` ~435; `merge` ~355; `style_from_config` ~476; tests)

**Interfaces:**
- Consumes: `Decl`, `parse_decl_from_table`.
- Produces: `pub struct RawSegment { pub text: String, pub align: String, pub decl: Decl }` (derives `Debug, Clone, Default, PartialEq`); `pub struct RawStatusBar { pub border: Option<String>, pub border_fg: Option<String>, pub segments: Vec<RawSegment> }` (same derives); `StyleDoc.status_bar: RawStatusBar`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn statusbar_block_parses_segments_and_border() {
    let text = r##"
[statusbar]
border = "single"
border_fg = "cyan"

[[statusbar.segment]]
text = "{location}"
align = "left"
fg = "cyan"
bold = true

[[statusbar.segment]]
text = "Score: {score}"
align = "right"
"##;
    let doc = parse_style_toml(text).unwrap();
    assert_eq!(doc.status_bar.border.as_deref(), Some("single"));
    assert_eq!(doc.status_bar.border_fg.as_deref(), Some("cyan"));
    assert_eq!(doc.status_bar.segments.len(), 2);
    assert_eq!(doc.status_bar.segments[0].text, "{location}");
    assert_eq!(doc.status_bar.segments[0].align, "left");
    assert_eq!(doc.status_bar.segments[0].decl.fg.as_deref(), Some("cyan"));
    assert_eq!(doc.status_bar.segments[0].decl.bold, Some(true));
    assert_eq!(doc.status_bar.segments[1].align, "right");
}

#[test]
fn merge_replaces_statusbar_segments_when_override_has_any() {
    let mut base = StyleDoc::default();
    base.status_bar.segments.push(RawSegment { text: "a".into(), align: "left".into(), decl: Decl::default() });
    let mut over = StyleDoc::default();
    over.status_bar.segments.push(RawSegment { text: "b".into(), align: "right".into(), decl: Decl::default() });
    over.status_bar.border = Some("double".into());
    let m = merge(&base, &over);
    assert_eq!(m.status_bar.segments.len(), 1);
    assert_eq!(m.status_bar.segments[0].text, "b");
    assert_eq!(m.status_bar.border.as_deref(), Some("double"));
    // Empty override keeps base segments.
    let m2 = merge(&base, &StyleDoc::default());
    assert_eq!(m2.status_bar.segments[0].text, "a");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app statusbar_block_parses_segments_and_border merge_replaces_statusbar_segments_when_override_has_any`
Expected: compile error (types/field missing).

- [ ] **Step 3: Define the raw types**

In `crates/app/src/style.rs`, after the `RawRule` struct, add:

```rust
/// A raw (uncompiled) status-bar segment from `[[statusbar.segment]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawSegment {
    /// Text template (literal text mixed with `{placeholder}` tokens).
    pub text: String,
    /// Cluster name: `left` | `center` | `right` (unknown → `left` at resolve).
    pub align: String,
    /// The fg/bg/bold/italic style fields for this segment.
    pub decl: Decl,
}

/// A raw `[statusbar]` block: optional frame + ordered segments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawStatusBar {
    pub border: Option<String>,
    pub border_fg: Option<String>,
    pub segments: Vec<RawSegment>,
}
```

- [ ] **Step 4: Add the `StyleDoc` field**

In the `StyleDoc` struct, after `pub transcript_rules: Vec<RawRule>,` add:

```rust
    /// The status-bar block from `[statusbar]` / `[[statusbar.segment]]`.
    pub status_bar: RawStatusBar,
```

- [ ] **Step 5: Parse the block in `parse_style_toml`**

In `parse_style_toml`, after the `transcript_rules` loop and before `Ok(StyleDoc { … })`, add:

```rust
    let mut status_bar = RawStatusBar::default();
    if let Some(toml::Value::Table(sb)) = root.get("statusbar") {
        status_bar.border = sb.get("border").and_then(toml::Value::as_str).map(str::to_string);
        status_bar.border_fg = sb.get("border_fg").and_then(toml::Value::as_str).map(str::to_string);
        if let Some(toml::Value::Array(segs)) = sb.get("segment") {
            for item in segs {
                if let toml::Value::Table(st) = item {
                    let text = st.get("text").and_then(toml::Value::as_str).unwrap_or("").to_string();
                    let align = st.get("align").and_then(toml::Value::as_str).unwrap_or("left").to_string();
                    let decl = parse_decl_from_table(st);
                    status_bar.segments.push(RawSegment { text, align, decl });
                }
            }
        }
    }
```

Then change the return to:

```rust
    Ok(StyleDoc { colors, symbols, transcript_rules, status_bar })
```

- [ ] **Step 6: Carry the block through `merge`**

In `merge`, after the `transcript_rules` block and before the `StyleDoc { … }` return, add:

```rust
    let status_bar = RawStatusBar {
        border: over.status_bar.border.clone().or(base.status_bar.border.clone()),
        border_fg: over.status_bar.border_fg.clone().or(base.status_bar.border_fg.clone()),
        segments: if over.status_bar.segments.is_empty() {
            base.status_bar.segments.clone()
        } else {
            over.status_bar.segments.clone()
        },
    };
```

And update the returned struct:

```rust
    StyleDoc {
        colors: StyleColors { scheme, selectors },
        symbols,
        transcript_rules,
        status_bar,
    }
```

- [ ] **Step 7: Fix the `style_from_config` literal**

In `style_from_config` (~476), add the new field to its `StyleDoc { … }` literal:

```rust
    StyleDoc {
        colors: colors.clone(),
        symbols: symbols.clone(),
        transcript_rules: Vec::new(),
        status_bar: RawStatusBar::default(),
    }
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): parse [statusbar] block into RawStatusBar + merge

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: Resolve `[statusbar]` — compile into StatusBarLayout, map frame

**Files:**
- Modify: `crates/app/src/style.rs` (`resolve` ~510; tests)

**Interfaces:**
- Consumes: `RawStatusBar`/`RawSegment` (Task 2); `colors::{Align, StatusSegment, StatusBarLayout}` (Task 1); `decl_to_style`, `paneframe::parse_border_style`, `colors::parse_color_value`.
- Produces: in `resolve`, `cs.statusbar_layout` is overwritten when the doc has segments; `cs.status_header_style` / `cs.status_header` are set from `border`/`border_fg`. Unknown `align` warns and falls back to `Left`. An empty segment list keeps the default layout.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn resolve_statusbar_segments_border_and_align() {
    use crate::colors::Align;
    let text = r##"
[statusbar]
border = "single"
border_fg = "cyan"
[[statusbar.segment]]
text = "{location}"
align = "left"
fg = "yellow"
[[statusbar.segment]]
text = "{title}"
align = "center"
[[statusbar.segment]]
text = "{score}"
align = "bogus"
"##;
    let doc = parse_style_toml(text).unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    // Three segments, with the unknown align defaulting to Left + a warning.
    assert_eq!(cs.statusbar_layout.segments.len(), 3);
    assert!(matches!(cs.statusbar_layout.segments[0].align, Align::Left));
    assert!(matches!(cs.statusbar_layout.segments[1].align, Align::Center));
    assert!(matches!(cs.statusbar_layout.segments[2].align, Align::Left));
    assert_eq!(cs.statusbar_layout.segments[0].style.fg, Some(ratatui::style::Color::Yellow));
    assert!(warnings.iter().any(|w| w.contains("align")), "unknown align warns: {warnings:?}");
    // border maps onto the existing status_header machinery.
    assert!(matches!(cs.status_header_style, crate::render::paneframe::BorderStyle::Single));
    assert_eq!(cs.status_header.fg, Some(ratatui::style::Color::Cyan));
}

#[test]
fn resolve_no_statusbar_keeps_default_layout() {
    let (cs, _set, _w) = resolve(&StyleDoc::default(), std::path::Path::new("."));
    assert_eq!(cs.statusbar_layout, crate::colors::StatusBarLayout::default());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app resolve_statusbar_segments_border_and_align resolve_no_statusbar_keeps_default_layout`
Expected: FAIL — segments not applied, no align warning, border unset.

- [ ] **Step 3: Compile the statusbar in `resolve`**

In `resolve`, after the transcript-rules compile loop and before `// Step 4: resolve symbols.`, add:

```rust
    // Compile the [statusbar] block. Segments replace the default layout only when
    // present; an empty block keeps the built-in default (today's bar).
    if !doc.status_bar.segments.is_empty() {
        let mut segments = Vec::with_capacity(doc.status_bar.segments.len());
        for raw in &doc.status_bar.segments {
            let align = match raw.align.as_str() {
                "left" => crate::colors::Align::Left,
                "center" => crate::colors::Align::Center,
                "right" => crate::colors::Align::Right,
                other => {
                    warnings.push(format!("unknown statusbar align '{}'; using left", other));
                    crate::colors::Align::Left
                }
            };
            segments.push(crate::colors::StatusSegment {
                text: raw.text.clone(),
                align,
                style: decl_to_style(&raw.decl, &gs),
            });
        }
        cs.statusbar_layout = crate::colors::StatusBarLayout { segments };
    }
    // The frame maps onto the existing status_header fields (reuses the boxing path).
    if let Some(b) = &doc.status_bar.border {
        cs.status_header_style = paneframe::parse_border_style(b);
    }
    if let Some(c) = &doc.status_bar.border_fg {
        if let Some(color) = colors::parse_color_value(c, &gs) {
            cs.status_header = cs.status_header.fg(color);
        }
    }
```

(`paneframe` and `colors` are already imported at the top of `style.rs`.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): resolve [statusbar] into StatusBarLayout + map frame

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: Render — placeholder resolution, cluster packing, render rewrite

**Files:**
- Modify: `crates/app/src/render/transcript.rs` (new pure helpers + `render_status_content` rewrite ~432; the two call sites ~393/395; tests)

**Interfaces:**
- Consumes: `colors::{Align, StatusSegment}`, `AppState.{colors, status_msg, transcript_filter, turns, title}`, `machine.status_line()`, `StatusRight`, `truncate_line`, `draw_str_clipped`.
- Produces: `struct StatusFields { … }`; `fn resolve_placeholders(text: &str, f: &StatusFields) -> Option<String>` (`None` = hide); `fn pack_status_clusters(visible: &[(String, Style, Align)], width: usize) -> Vec<(u16, String, Style)>` (draw ops `(x_col, text, style)`); `render_status_content(machine, state, buf, region)` rewritten.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/render/transcript.rs`, inside `mod tests`, add:

```rust
use crate::colors::Align;

fn fields_score() -> StatusFields {
    StatusFields {
        location: "West of House".into(),
        score: Some("10".into()),
        moves: Some("5".into()),
        time: None,
        turns: "7".into(),
        title: "Zork".into(),
        filter: String::new(),
    }
}

#[test]
fn resolve_placeholders_substitutes_and_hides() {
    let f = fields_score();
    assert_eq!(resolve_placeholders("Score: {score}  Moves: {moves}", &f).as_deref(), Some("Score: 10  Moves: 5"));
    // pure literal always shown
    assert_eq!(resolve_placeholders(" | ", &f).as_deref(), Some(" | "));
    // all-empty placeholder segment hides (time is None on a score game)
    assert_eq!(resolve_placeholders("{time}", &f), None);
    // mixed: one empty, one non-empty placeholder → shown
    assert_eq!(resolve_placeholders("{time}{location}", &f).as_deref(), Some("West of House"));
    // unknown token → empty; all-empty → hidden
    assert_eq!(resolve_placeholders("{bogus}", &f), None);
    // turns vs moves are distinct
    assert_eq!(resolve_placeholders("{turns}/{moves}", &f).as_deref(), Some("7/5"));
}

#[test]
fn pack_clusters_positions_and_truncates() {
    let s = Style::default();
    let mk = |t: &str, a: Align| (t.to_string(), s, a);
    // width 30: left "abc"(0), right "XY"(28)
    let ops = pack_status_clusters(&[mk("abc", Align::Left), mk("XY", Align::Right)], 30);
    let left = ops.iter().find(|(_, t, _)| t == "abc").unwrap();
    let right = ops.iter().find(|(_, t, _)| t == "XY").unwrap();
    assert_eq!(left.0, 0);
    assert_eq!(right.0, 28); // 30 - 2
    // center centered in the gap between left end (3) and right start (28): gap 25, center "cc"(2) at 3 + (25-2)/2 = 14
    let ops2 = pack_status_clusters(&[mk("abc", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 30);
    let center = ops2.iter().find(|(_, t, _)| t == "cc").unwrap();
    assert_eq!(center.0, 14);
    // narrow width 6: right "XY" preserved at 4; center dropped; left "abcdef" truncated to 4 ("abcd")
    let ops3 = pack_status_clusters(&[mk("abcdef", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 6);
    assert!(ops3.iter().all(|(_, t, _)| t != "cc"), "center dropped under pressure");
    let right3 = ops3.iter().find(|(x, _, _)| *x == 4).unwrap();
    assert_eq!(right3.1, "XY");
    let left3 = ops3.iter().find(|(x, _, _)| *x == 0).unwrap();
    assert_eq!(left3.1, "abcd"); // truncated to the 4 cols before the right cluster
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app resolve_placeholders_substitutes_and_hides pack_clusters_positions_and_truncates`
Expected: compile error (helpers/struct missing).

- [ ] **Step 3: Add the placeholder resolver**

In `crates/app/src/render/transcript.rs`, after the `format_status` function, add:

```rust
/// The field values available to status-bar segment templates for one turn.
pub(crate) struct StatusFields {
    pub location: String,
    pub score: Option<String>,
    pub moves: Option<String>,
    pub time: Option<String>,
    pub turns: String,
    pub title: String,
    pub filter: String,
}

fn status_field_value<'a>(f: &'a StatusFields, name: &str) -> &'a str {
    match name {
        "location" => &f.location,
        "score" => f.score.as_deref().unwrap_or(""),
        "moves" => f.moves.as_deref().unwrap_or(""),
        "time" => f.time.as_deref().unwrap_or(""),
        "turns" => &f.turns,
        "title" => &f.title,
        "filter" => &f.filter,
        _ => "", // unknown token → empty
    }
}

/// Resolve a segment's `{placeholder}` template against `f`.
///
/// Returns `Some(resolved)` for a pure-literal segment or one with at least one
/// non-empty placeholder; returns `None` (hide the segment) when the template
/// contains placeholders that ALL resolve to empty. An unterminated `{` is
/// treated as a literal brace.
pub(crate) fn resolve_placeholders(text: &str, f: &StatusFields) -> Option<String> {
    let mut out = String::new();
    let mut had_placeholder = false;
    let mut any_nonempty = false;
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let name = &after[..close];
            had_placeholder = true;
            let val = status_field_value(f, name);
            if !val.is_empty() {
                any_nonempty = true;
            }
            out.push_str(val);
            rest = &after[close + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    if had_placeholder && !any_nonempty {
        None
    } else {
        Some(out)
    }
}
```

- [ ] **Step 4: Add the cluster packer**

In `crates/app/src/render/transcript.rs`, after `resolve_placeholders`, add:

```rust
/// Pack visible `(text, style, align)` segments into draw ops `(x_col, text, style)`.
///
/// Left cluster packs from the left edge; right cluster packs flush against the
/// right edge; center cluster centers in the gap between them. Truncation when
/// space runs short: drop the center cluster, then truncate the left cluster to
/// the space before the right cluster, preserving the right cluster (clipped only
/// if it alone exceeds the width). `x_col` is relative to the region's left edge.
pub(crate) fn pack_status_clusters(
    visible: &[(String, ratatui::style::Style, crate::colors::Align)],
    width: usize,
) -> Vec<(u16, String, ratatui::style::Style)> {
    use crate::colors::Align;
    let cw = |s: &str| s.chars().count();
    let pick = |a: Align| -> Vec<&(String, ratatui::style::Style, Align)> {
        visible.iter().filter(|(_, _, sa)| *sa == a).collect()
    };
    let left = pick(Align::Left);
    let center = pick(Align::Center);
    let right = pick(Align::Right);
    let sum = |v: &[&(String, ratatui::style::Style, Align)]| v.iter().map(|(t, _, _)| cw(t)).sum::<usize>();
    let left_w = sum(&left);
    let right_w = sum(&right);
    let center_w = sum(&center);

    let mut ops: Vec<(u16, String, ratatui::style::Style)> = Vec::new();

    // RIGHT cluster: flush right, declared order, clipped to the row.
    let right_start = width.saturating_sub(right_w);
    {
        let mut x = right_start;
        for (t, s, _) in &right {
            let avail = width.saturating_sub(x);
            if avail == 0 { break; }
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    // LEFT cluster: flush left, truncated to the space before the right cluster.
    let left_budget = right_start;
    {
        let mut x = 0usize;
        for (t, s, _) in &left {
            if x >= left_budget { break; }
            let avail = left_budget - x;
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    // CENTER cluster: only when it fits in the gap; otherwise dropped.
    let gap_start = left_w;
    let gap_end = right_start;
    if gap_end > gap_start && center_w <= gap_end - gap_start {
        let mut x = gap_start + (gap_end - gap_start - center_w) / 2;
        for (t, s, _) in &center {
            let avail = gap_end.saturating_sub(x);
            if avail == 0 { break; }
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    ops
}
```

- [ ] **Step 5: Run the helper tests to verify they pass**

Run: `cargo test -p app resolve_placeholders_substitutes_and_hides pack_clusters_positions_and_truncates`
Expected: PASS.

- [ ] **Step 6: Write the failing render test**

In `crates/app/src/render/transcript.rs`, inside `mod tests`, add:

```rust
#[test]
fn render_status_default_bar_matches_today() {
    // With no custom [statusbar], the bar shows location left and the filter
    // indicator right; score/moves come from the (empty) minimal machine.
    let machine = minimal_machine();
    let mut state = AppState::default();
    state.transcript_filter = crate::state::TranscriptFilter::Story;

    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    render_transcript(&machine, &state, area, &mut buf);

    let row: String = (0..40u16)
        .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
        .collect();
    // filter indicator pinned right (default bar includes ` {filter}`).
    assert!(row.contains("[filter: story]"), "default bar must show the filter indicator: {:?}", row);
    // status row keeps the reversed-video base fill.
    assert!(buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED));
}
```

- [ ] **Step 7: Run it to verify it fails**

Run: `cargo test -p app render_status_default_bar_matches_today`
Expected: FAIL — today's `render_status_content` draws the filter indicator differently / the rewrite isn't in yet. (If it happens to pass before the rewrite, the rewrite must keep it passing.)

- [ ] **Step 8: Rewrite `render_status_content`**

In `crates/app/src/render/transcript.rs`, replace the whole `render_status_content` function and update its two call sites in `render_transcript`.

Replace the call sites (~393/395) — both currently pass
`(machine, buf, <region>, state.colors.status_bar, state.status_msg.as_deref(), state.transcript_filter)` — with:

```rust
        render_status_content(machine, state, buf, frame.content);
```
and
```rust
        render_status_content(machine, state, buf, status_region);
```

Replace the function body:

```rust
/// Draw the status bar into `region`.
///
/// When `state.status_msg` is `Some`, it overrides all segments and renders the
/// transient message left-aligned in the base style. Otherwise each segment in
/// `state.colors.statusbar_layout` is resolved (placeholders substituted, empty
/// ones hidden), styled (base patched with the segment style), and packed into
/// left/center/right clusters.
fn render_status_content(
    machine: &Machine,
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    let base = state.colors.status_bar;
    let status_y = region.y;
    let w = region.width as usize;

    // Fill the row with the base style.
    for x in region.x..region.right() {
        if let Some(cell) = buf.cell_mut((x, status_y)) {
            cell.set_symbol(" ").set_style(base);
        }
    }

    // Transient status message overrides the segments.
    if let Some(msg) = state.status_msg.as_deref() {
        let msg_trunc = truncate_line(msg, w);
        draw_str_clipped(buf, region.x, status_y, msg_trunc, base, region);
        return;
    }

    // Build the field values for this turn.
    let sl = machine.status_line();
    let (score, moves, time) = match sl.right {
        StatusRight::ScoreTurns { score, turns } => (Some(score.to_string()), Some(turns.to_string()), None),
        StatusRight::Time { hours, minutes } => (None, None, Some(format!("{:02}:{:02}", hours, minutes))),
    };
    let filter = match state.transcript_filter {
        TranscriptFilter::Both => String::new(),
        TranscriptFilter::Story => "[filter: story]".to_string(),
        TranscriptFilter::Meta => "[filter: meta]".to_string(),
    };
    let fields = StatusFields {
        location: sl.location,
        score,
        moves,
        time,
        turns: state.turns.to_string(),
        title: state.title.clone(),
        filter,
    };

    // Resolve + style + drop hidden segments.
    let visible: Vec<(String, Style, crate::colors::Align)> = state
        .colors
        .statusbar_layout
        .segments
        .iter()
        .filter_map(|seg| {
            resolve_placeholders(&seg.text, &fields).map(|txt| (txt, base.patch(seg.style), seg.align))
        })
        .collect();

    // Pack into clusters and draw.
    for (x, txt, style) in pack_status_clusters(&visible, w) {
        draw_str_clipped(buf, region.x + x, status_y, &txt, style, region);
    }
}
```

Note: this removes the old `format_status`-based two-part draw and the separate filter-indicator overlay (the filter is now the ` {filter}` segment). `format_status` may become unused — if `cargo build` warns it is dead, delete it and its `#[cfg(test)]` callers' references are in its own unit tests (`format_status_score_turns`, `format_status_time`); keep those tests only if `format_status` is retained. If you delete `format_status`, also delete those two tests.

- [ ] **Step 9: Run the full suite to verify it passes**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 10: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/render/transcript.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): render configurable status-bar segments (clusters + placeholders)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: Export — write_style emits transcript rules + statusbar; write_style_full populates them

**Files:**
- Modify: `crates/app/src/style.rs` (`write_style` ~704; `write_style_full` ~940; tests)

**Interfaces:**
- Consumes: `cs.transcript_rules` (`CompiledRule { pattern, style }`), `cs.statusbar_layout` (`StatusSegment { text, align, style }`), `style_to_decl`, `colors::Align`.
- Produces: `write_style` writes `[[transcript.rule]]` and `[[statusbar.segment]]` from `doc.transcript_rules` / `doc.status_bar`; `write_style_full` populates both from the `ColorScheme` before writing. Frame is NOT emitted in the statusbar block (round-trips via `status_header`).

- [ ] **Step 1: Write the failing test**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn write_style_full_round_trips_statusbar_and_transcript_rules() {
    use crate::colors::{Align, StatusSegment, StatusBarLayout};
    use ratatui::style::Color;
    let dir = std::env::temp_dir().join(format!("babelmap-sb-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sb.toml");

    let mut cs = crate::colors::ColorScheme::terminal_default();
    // A custom transcript rule.
    cs.transcript_rules.push(crate::colors::CompiledRule {
        pattern: "(?i)grue".into(),
        regex: regex::Regex::new("(?i)grue").unwrap(),
        style: Style::new().fg(Color::Red),
    });
    // A custom statusbar layout.
    cs.statusbar_layout = StatusBarLayout {
        segments: vec![
            StatusSegment { text: "{location}".into(), align: Align::Left, style: Style::new().fg(Color::Cyan) },
            StatusSegment { text: "{title}".into(), align: Align::Center, style: Style::default() },
            StatusSegment { text: "Score {score}".into(), align: Align::Right, style: Style::new().fg(Color::Yellow) },
        ],
    };
    let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
    write_style_full(&path, &cs, &set).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let doc = parse_style_toml(&text).unwrap();
    let (cs2, _set2, _w) = resolve(&doc, &dir);

    // Transcript rule survived.
    assert_eq!(cs2.transcript_rules.len(), 1);
    assert_eq!(cs2.transcript_rules[0].pattern, "(?i)grue");
    assert_eq!(cs2.transcript_rules[0].style.fg, Some(Color::Red));
    // Statusbar layout survived (text, align, style).
    assert_eq!(cs2.statusbar_layout.segments.len(), 3);
    assert_eq!(cs2.statusbar_layout.segments[0].text, "{location}");
    assert!(matches!(cs2.statusbar_layout.segments[1].align, Align::Center));
    assert_eq!(cs2.statusbar_layout.segments[2].style.fg, Some(Color::Yellow));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app write_style_full_round_trips_statusbar_and_transcript_rules`
Expected: FAIL — rules and segments are not exported, so `cs2` loses them.

- [ ] **Step 3: Emit the blocks in `write_style`**

In `crates/app/src/style.rs`, in `write_style`, after the `[symbols]` block and before `std::fs::write(path, tdoc.to_string())`, add:

```rust
    // ── [[transcript.rule]] ─────────────────────────────────────────────────────
    {
        // Remove any existing transcript table, then rewrite from the doc.
        tdoc.remove("transcript");
        if !doc.transcript_rules.is_empty() {
            let mut arr = toml_edit::ArrayOfTables::new();
            for r in &doc.transcript_rules {
                let mut t = toml_edit::Table::new();
                t["match"] = toml_edit::value(r.pattern.as_str());
                if let Some(fg) = &r.decl.fg { t["fg"] = toml_edit::value(fg.as_str()); }
                if let Some(bg) = &r.decl.bg { t["bg"] = toml_edit::value(bg.as_str()); }
                if r.decl.bold == Some(true) { t["bold"] = toml_edit::value(true); }
                if r.decl.italic == Some(true) { t["italic"] = toml_edit::value(true); }
                arr.push(t);
            }
            let mut transcript = toml_edit::Table::new();
            transcript.insert("rule", toml_edit::Item::ArrayOfTables(arr));
            tdoc.insert("transcript", toml_edit::Item::Table(transcript));
        }
    }

    // ── [statusbar] ─────────────────────────────────────────────────────────────
    {
        tdoc.remove("statusbar");
        let sb = &doc.status_bar;
        if sb.border.is_some() || sb.border_fg.is_some() || !sb.segments.is_empty() {
            let mut table = toml_edit::Table::new();
            if let Some(b) = &sb.border { table["border"] = toml_edit::value(b.as_str()); }
            if let Some(c) = &sb.border_fg { table["border_fg"] = toml_edit::value(c.as_str()); }
            if !sb.segments.is_empty() {
                let mut arr = toml_edit::ArrayOfTables::new();
                for seg in &sb.segments {
                    let mut t = toml_edit::Table::new();
                    t["text"] = toml_edit::value(seg.text.as_str());
                    t["align"] = toml_edit::value(seg.align.as_str());
                    if let Some(fg) = &seg.decl.fg { t["fg"] = toml_edit::value(fg.as_str()); }
                    if let Some(bg) = &seg.decl.bg { t["bg"] = toml_edit::value(bg.as_str()); }
                    if seg.decl.bold == Some(true) { t["bold"] = toml_edit::value(true); }
                    if seg.decl.italic == Some(true) { t["italic"] = toml_edit::value(true); }
                    arr.push(t);
                }
                table.insert("segment", toml_edit::Item::ArrayOfTables(arr));
            }
            tdoc.insert("statusbar", toml_edit::Item::Table(table));
        }
    }
```

- [ ] **Step 4: Populate the doc in `write_style_full`**

In `write_style_full`, just before the final `write_style(path, &doc)` call (~940), add:

```rust
    // Export user transcript rules (CompiledRule → RawRule).
    for rule in &cs.transcript_rules {
        doc.transcript_rules.push(RawRule {
            pattern: rule.pattern.clone(),
            decl: style_to_decl(&rule.style),
        });
    }
    // Export the statusbar segments (StatusSegment → RawSegment). The frame is NOT
    // re-emitted here; it round-trips through the status_header selector export.
    for seg in &cs.statusbar_layout.segments {
        doc.status_bar.segments.push(RawSegment {
            text: seg.text.clone(),
            align: seg.align.as_str().to_string(),
            decl: style_to_decl(&seg.style),
        });
    }
```

- [ ] **Step 5: Add `Align::as_str`**

In `crates/app/src/colors.rs`, after the `Align` enum, add:

```rust
impl Align {
    /// The lowercase config name for this alignment.
    pub fn as_str(&self) -> &'static str {
        match self {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
        }
    }
}
```

- [ ] **Step 6: Run the full suite to verify it passes**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (`write_style_full_is_self_contained` and the transcript-category round-trip stay green: `terminal_default` has empty rules and the default layout; exporting the default layout's segments re-resolves to the same layout — equal.)

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs crates/app/src/colors.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): export statusbar + transcript rules in write_style_full

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: Documentation — commented style.example.toml + README pointer + parse test

**Files:**
- Create: `style.example.toml` (repo root)
- Modify: `README.md` (Customization section — one-line pointer)
- Modify: `crates/app/src/style.rs` (a test that the example parses+resolves with zero warnings)

**Interfaces:**
- Consumes: `parse_style_toml`, `resolve`. No new types.

- [ ] **Step 1: Create the annotated example file**

Create `style.example.toml` at the repo root:

```toml
# babelmap style reference — copy to ~/.babelmap/style.toml and edit.
# Point config.toml at it with:  style = "~/.babelmap/style.toml"
#
# Color values accept: a named color (cyan, dark-gray, light-blue, …),
# palette:N (0-15 from the active scheme), #rrggbb hex, a 256-index ("17"),
# or background / foreground (the scheme's bg/fg).

[colors]
# Optional base scheme: a built-in (mono / high-contrast / tomorrow-night) or a
# path to a Ghostty theme file. Omit to use your terminal's colors.
# scheme = "tomorrow-night"

# Each selector is an inline table of fg / bg / bold / italic / underline / dim /
# reversed. Border selectors also take style = "<border>"; the dialog selector
# also takes shadow = true.

# ── Map ──────────────────────────────────────────────────────────────────────
"room"                 = { fg = "white" }       # normal room
"room:current"         = { reversed = true }    # the room you're in
"room:selected"        = { fg = "yellow" }      # cursor-highlighted room
"connector"            = { fg = "cyan" }        # normal connector
"connector:distorted"  = { fg = "magenta" }     # one-way / distorted
"connector:portal"     = { fg = "cyan" }        # up/down/in/out portals
"map_border"           = { style = "picture-frame", fg = "cyan" }
"map_layer_tab"        = { fg = "dark-gray" }   # inactive layer tab
"map_layer_tab_active" = { fg = "cyan", bold = true }
"loc_indicator"        = { fg = "dark-gray" }   # room-detection method indicator

# ── Story / transcript ───────────────────────────────────────────────────────
"transcript"           = { fg = "white" }       # base game text
"transcript:input"     = { fg = "cyan" }        # your echoed commands
"transcript:meta"      = { fg = "dark-gray" }   # app / slash output
"transcript:warning"   = { fg = "yellow" }      # VM warnings
"transcript:location"  = { bold = true }        # room-name header lines
"transcript:system"    = { fg = "dark-gray" }   # [bracketed] system lines
"meta_marker"          = { fg = "dark-gray" }   # meta gutter color
"warning_marker"       = { fg = "yellow" }      # warning gutter color
"story_border"         = { style = "single", fg = "cyan" }
"story_title"          = { fg = "white" }
"suggestion"           = { fg = "dark-gray" }   # autocomplete line

# ── Status & input chrome ────────────────────────────────────────────────────
"statusbar"            = { reversed = true }    # status-bar base style (see [statusbar])
"input_line"           = { }                    # set style = "single" to box it
"helpbar"              = { reversed = true }

# ── Dialogs / overlays ───────────────────────────────────────────────────────
"dialog"               = { style = "single", bg = "black", shadow = true }
"dialog:title"         = { fg = "cyan" }
"dialog:button"        = { fg = "white" }
"dialog:button:active" = { fg = "black", bg = "cyan" }
"dialog:shadow"        = { bg = "dark-gray" }
"upper_window"         = { }                    # v4+ game-drawn screen
"upper_window_border"  = { style = "single", fg = "cyan" }

# ── Sound-event border pulse ─────────────────────────────────────────────────
"sound_beep_high"      = { fg = "#ffb428" }
"sound_beep_low"       = { fg = "#3c8cdc" }

# ── Story-line styling rules ─────────────────────────────────────────────────
# Each rule matches a Story line by regex (whole-line) and patches its style.
# Rules are tried in order; the first match wins, ahead of the built-in
# location/system rules.
[[transcript.rule]]
match = "^>.*"          # your command echo
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"
fg = "red"

# ── Status bar ───────────────────────────────────────────────────────────────
# The bar is an ordered list of segments in three clusters (align = left | center
# | right). Omit [statusbar] entirely to keep the built-in default bar.
#
# Placeholders: {location} {score} {moves} {time} {turns} {title} {filter}
#   {moves} = the game's move count; {turns} = your session command count;
#   {filter} = the active /filter indicator (empty when showing both).
# A segment with no placeholder is always shown; a segment whose placeholders all
# resolve empty is hidden (so score segments vanish on clock games and vice-versa).
[statusbar]
border = "none"          # none | single | double | thick | picture-frame
# border_fg = "cyan"

[[statusbar.segment]]
text  = "{location}"
align = "left"
fg    = "cyan"
bold  = true

[[statusbar.segment]]
text  = "Score: {score}  Moves: {moves}"
align = "right"

[[statusbar.segment]]
text  = "{time}"
align = "right"

[[statusbar.segment]]
text  = " {filter}"
align = "right"
fg    = "dark-gray"

# ── Symbols ──────────────────────────────────────────────────────────────────
# Presets per category; override individual glyphs under [symbols.overrides].
[symbols]
box_style    = "rounded"   # rounded | thick | double | solid | super-thick | ascii | borderless
arrow_set    = "filled"    # filled | line | nerdfont | nf-bold | nf-box | nf-circle | nf-outline
portal_icons = "ascii"     # ascii | nerdfont | nerdfont-stairs
path_style   = "light"     # light | heavy | dotted

[symbols.overrides]
# Single-char overrides for any glyph slot. Examples:
"gutter.meta"    = "▏"     # meta transcript gutter marker
"gutter.warning" = "!"     # warning transcript gutter marker
# "arrow.north"  = "^"
# "room.normal.tl" = "+"
```

- [ ] **Step 2: Add the parse-clean test**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn style_example_toml_parses_and_resolves_clean() {
    // The repo-root style.example.toml is the user-facing reference; it must
    // parse and resolve with zero warnings so the docs cannot drift from the code.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../style.example.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let doc = parse_style_toml(&text).expect("style.example.toml must parse");
    let (_cs, _set, warnings) = resolve(&doc, path.parent().unwrap());
    assert!(warnings.is_empty(), "style.example.toml resolved with warnings: {warnings:?}");
}
```

- [ ] **Step 3: Run it to verify it passes**

Run: `cargo test -p app style_example_toml_parses_and_resolves_clean`
Expected: PASS (0 warnings). If it fails on a warning, fix the offending line in `style.example.toml` — a real selector/option name was mistyped.

- [ ] **Step 4: Add the README pointer**

In `README.md`, in the **Customization** section, find the "Shareable style files" bullet (it mentions `style.toml` / `style = "<name or path>"`) and append one sentence to it:

```
  See `style.example.toml` at the repo root for a fully-commented reference of
  every selector, the `[[transcript.rule]]` story rules, the `[statusbar]`
  segment bar, and the `[symbols]` overrides.
```

- [ ] **Step 5: Run the full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add style.example.toml README.md crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "docs: commented style.example.toml reference + README pointer

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Dependency order: 1 (data model) → 2 (parse) → 3 (resolve) → 4 (render) → 5 (export) → 6 (docs). Run them in order; each ends green (`cargo test -p app`, 0 warnings) before committing.
- The `style.example.toml` parse test (Task 6) is the guard that keeps the docs honest — if a later change renames a selector, that test fails until the example is updated.
- `README.md` is committed and kept current; `TODO.md` is gitignored — never stage it.
- If `format_status` becomes dead after Task 4's rewrite, delete it together with its two unit tests (`format_status_score_turns`, `format_status_time`); do not leave it unused (zero-warning rule).
