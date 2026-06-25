# Transcript Text Styling by Category — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make transcript text themeable by category (Story / Input / Meta / Warning), add user + built-in story sub-styling rules, per-category gutters, and fix the resize redraw artifact that corrupts the gutter.

**Architecture:** Expand `TranscriptKind` to four source-tagged variants; tag the echoed command as Input and VM diagnostics as Warning at their emit sites. Add per-category `Style` fields and `transcript:*` selectors to the `ColorScheme`/style system. Story lines pass through an ordered rule list (user regex rules → built-in location/system → base `transcript`, first-match whole-line patch). Gutter glyphs move into `SymbolSet`. A `terminal.clear()` on `Event::Resize` forces a full repaint.

**Tech Stack:** Rust, ratatui 0.29, `regex` crate (new dependency), TOML via `toml`/`toml_edit`.

## Global Constraints

- Commit trailers on every commit (body must contain, verbatim, no backticks anywhere in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Zero compiler warnings — the workspace builds clean; remove any symbol your change orphans.
- Do NOT push or merge; commit locally only.
- `TranscriptKind` is `serde`-serialized into save archives — adding variants is backward-compatible (old saves only contain `Story`/`Meta`); do not rename or reorder existing variants.
- Defaults (terminal_default): `transcript:input` fg = Cyan, `transcript:meta` fg = DarkGray, `transcript:warning` fg = Yellow, `transcript:location` = bold only, `transcript:system` fg = DarkGray, `warning_marker` fg = Yellow. `meta_gutter` glyph = `▏`, `warning_gutter` glyph = `!`.
- Built-in location match reuses `zvm::location::status_name_matches(line, room_name)` (equality or word-boundary leading prefix). Built-in system match = trimmed line fully bracketed (`[` … `]`).
- `style.toml` schema + `write_style_full` round-trip must stay green. `transcript_rules` are NOT exported by `write_style_full` (user-authored content, out of scope for export).
- Keep `git -C` / absolute paths; do not `cd`.
- Run the full app suite after each task: `cargo test -p app` (and `cargo test -p zvm` if zvm touched). Target: 0 failures, 0 warnings.

---

### Task 1: Data model — four kinds, filter mapping, emit-site tagging, current_room_name

**Files:**
- Modify: `crates/app/src/state.rs` (`TranscriptKind` enum ~149; `visible_transcript_indices` ~890; `AppState` struct + `Default`)
- Modify: `crates/app/src/main.rs` (echo site ~1552; `apply_turn_events` ~2599)
- Modify: `crates/app/src/render/transcript.rs` (`wrap_lines_kinded` ~156; render-loop `match kind` ~616) — minimal updates to keep the build exhaustive/green

**Interfaces:**
- Produces: `TranscriptKind::{Story, Input, Meta, Warning}`; `AppState.current_room_name: Option<String>`; filter mapping (Story→{Story,Input}, Meta→{Meta,Warning}); wrap widths (Meta|Warning → `width - META_GUTTER`, Story|Input → `width`).
- Consumes: existing `push_transcript_kind`, `META_GUTTER`, `META_MARKER`, `state.colors.meta_marker`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/state.rs`, inside `mod tests`, add:

```rust
#[test]
fn filter_maps_input_with_story_and_warning_with_meta() {
    let mut s = AppState::default();
    s.push_transcript("story0");
    s.push_transcript_kind("> go north", TranscriptKind::Input);
    s.push_transcript_kind("meta", TranscriptKind::Meta);
    s.push_transcript_kind("warn", TranscriptKind::Warning);
    s.transcript_filter = TranscriptFilter::Story;
    assert_eq!(s.visible_transcript_indices(), vec![0, 1]); // Story + Input
    s.transcript_filter = TranscriptFilter::Meta;
    assert_eq!(s.visible_transcript_indices(), vec![2, 3]); // Meta + Warning
    s.transcript_filter = TranscriptFilter::Both;
    assert_eq!(s.visible_transcript_indices(), vec![0, 1, 2, 3]);
}

#[test]
fn current_room_name_defaults_none() {
    let s = AppState::default();
    assert_eq!(s.current_room_name, None);
}
```

In `crates/app/src/render/transcript.rs`, inside `mod tests`, add:

```rust
#[test]
fn input_uses_full_width_warning_wraps_like_meta() {
    let line = vec!["abcdefgh".to_string()];
    // Input: full width 8 (no gutter) → unsplit.
    let i = wrap_lines_kinded(&line, &[TranscriptKind::Input], 8);
    assert_eq!(i.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
    // Warning: wraps to width-2 = 6 like Meta.
    let w = wrap_lines_kinded(&line, &[TranscriptKind::Warning], 8);
    assert_eq!(w.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), vec!["abcdef", "gh"]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app filter_maps_input_with_story_and_warning_with_meta current_room_name_defaults_none input_uses_full_width_warning_wraps_like_meta`
Expected: compile error (missing variants/field) or assertion failure.

- [ ] **Step 3: Expand the enum**

In `crates/app/src/state.rs`, replace the `TranscriptKind` enum (the doc comment may stay; update it):

```rust
/// Category tag for each transcript entry.
///
/// `Story` = game output. `Input` = the player's echoed command. `Meta` =
/// app/slash output. `Warning` = VM diagnostics. The `/filter` view is coarse
/// (story = Story+Input, meta = Meta+Warning); the styling is per-variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptKind {
    Story,
    Input,
    Meta,
    Warning,
}
```

- [ ] **Step 4: Update the filter mapping**

In `visible_transcript_indices`, replace the `match self.transcript_filter` body:

```rust
match self.transcript_filter {
    TranscriptFilter::Both => true,
    TranscriptFilter::Story => matches!(kind, TranscriptKind::Story | TranscriptKind::Input),
    TranscriptFilter::Meta => matches!(kind, TranscriptKind::Meta | TranscriptKind::Warning),
}
```

- [ ] **Step 5: Add the `current_room_name` field**

In `crates/app/src/state.rs`, add to the `AppState` struct (near `loc_method`):

```rust
    /// The current room's display name (from `TurnResult.location`), retained
    /// across turns. Drives the built-in `transcript:location` story rule.
    pub current_room_name: Option<String>,
```

In `impl Default for AppState`, add (near `loc_method: None,`):

```rust
            current_room_name: None,
```

- [ ] **Step 6: Tag the echoed command as Input**

In `crates/app/src/main.rs`, change the echo at ~1552:

```rust
                state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
```

(Replaces `state.push_transcript(&format!("> {}", cmd));`. `TranscriptKind` is already imported in this file.)

- [ ] **Step 7: Tag diagnostics as Warning and set current_room_name**

In `crates/app/src/main.rs`, replace the body of `apply_turn_events` (~2599):

```rust
fn apply_turn_events(state: &mut AppState, result: &TurnResult) {
    for line in &result.diagnostics {
        state.push_transcript_kind(line, app::state::TranscriptKind::Warning);
    }
    if let Some(kind) = result.beep {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
    state.loc_method = result.location_method.or(state.loc_method);
    // Retain the previous name when this turn has no location signal.
    if let Some(loc) = &result.location {
        state.current_room_name = Some(loc.name.clone());
    }
}
```

- [ ] **Step 8: Update the wrap widths (keep exhaustive)**

In `crates/app/src/render/transcript.rs`, in `wrap_lines_kinded`, replace the `let w = match kind { … }` block:

```rust
            let w = match kind {
                TranscriptKind::Meta | TranscriptKind::Warning => width.saturating_sub(META_GUTTER),
                TranscriptKind::Story | TranscriptKind::Input => width,
            };
```

- [ ] **Step 9: Update the render-loop match (minimal, keep green)**

In `crates/app/src/render/transcript.rs`, in `render_middle`, replace the `match kind { … }` block (~616) so all four variants are handled. For now Input renders like Story and Warning renders like Meta (full styling comes in Task 7):

```rust
        match kind {
            // META and WARNING get the gutter marker; text indented past it.
            TranscriptKind::Meta | TranscriptKind::Warning => {
                draw_str_clipped(buf, area.x, row_y, META_MARKER, marker_style, area);
                if has_search {
                    draw_str_highlighted(buf, area.x + META_GUTTER, row_y, line, normal_style, &query_lower, search_highlight_style, area);
                } else {
                    draw_str_clipped(buf, area.x + META_GUTTER, row_y, line, normal_style, area);
                }
            }
            TranscriptKind::Story | TranscriptKind::Input => {
                if has_search {
                    draw_str_highlighted(buf, area.x, row_y, line, normal_style, &query_lower, search_highlight_style, area);
                } else {
                    draw_str_clipped(buf, area.x, row_y, line, normal_style, area);
                }
            }
        }
```

- [ ] **Step 10: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 failures, 0 warnings.

- [ ] **Step 11: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/state.rs crates/app/src/main.rs crates/app/src/render/transcript.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): four transcript kinds + filter mapping + room-name tracking

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: ColorScheme — per-category Style fields, CompiledRule, regex dependency

**Files:**
- Modify: `crates/app/Cargo.toml` (add `regex`)
- Modify: `crates/app/src/colors.rs` (`CompiledRule` type; `ColorScheme` fields; `terminal_default`; `from_ghostty`; tests)

**Interfaces:**
- Produces: `ColorScheme.transcript_input/transcript_meta/transcript_warning/transcript_location/transcript_system/warning_marker: Style`; `ColorScheme.transcript_rules: Vec<CompiledRule>`; `pub struct CompiledRule { pub pattern: String, pub regex: regex::Regex, pub style: Style }` with manual `PartialEq` (compares `pattern` + `style`, not the compiled `Regex`).
- Consumes: nothing new.

- [ ] **Step 1: Add the regex dependency**

In `crates/app/Cargo.toml`, under `[dependencies]`, add:

```toml
regex = "1"
```

- [ ] **Step 2: Write the failing tests**

In `crates/app/src/colors.rs`, inside `mod tests`, add:

```rust
#[test]
fn terminal_default_transcript_category_styles() {
    let cs = ColorScheme::terminal_default();
    assert_eq!(cs.transcript_input.fg, Some(Color::Cyan));
    assert_eq!(cs.transcript_meta.fg, Some(Color::DarkGray));
    assert_eq!(cs.transcript_warning.fg, Some(Color::Yellow));
    assert!(cs.transcript_location.add_modifier.contains(Modifier::BOLD));
    assert_eq!(cs.transcript_location.fg, None); // bold-only, inherits base fg
    assert_eq!(cs.transcript_system.fg, Some(Color::DarkGray));
    assert_eq!(cs.warning_marker.fg, Some(Color::Yellow));
    assert!(cs.transcript_rules.is_empty());
}

#[test]
fn compiled_rule_eq_ignores_regex_object() {
    let a = CompiledRule { pattern: "^>".into(), regex: regex::Regex::new("^>").unwrap(), style: Style::new().fg(Color::Red) };
    let b = CompiledRule { pattern: "^>".into(), regex: regex::Regex::new("^>").unwrap(), style: Style::new().fg(Color::Red) };
    assert_eq!(a, b);
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p app terminal_default_transcript_category_styles compiled_rule_eq_ignores_regex_object`
Expected: compile error (missing type/fields).

- [ ] **Step 4: Define `CompiledRule`**

In `crates/app/src/colors.rs`, after the `use` block (the file already imports `ratatui::style::{Color, Modifier, Style}`), add:

```rust
use regex::Regex;

/// A compiled user transcript-styling rule: a regex matched whole-line against
/// Story text, plus the `Style` patched over the base `transcript` style on a
/// match. `PartialEq` compares the source `pattern` and `style` only — the
/// compiled `Regex` has no `PartialEq`, and two rules with the same pattern are
/// equal by construction.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub pattern: String,
    pub regex: Regex,
    pub style: Style,
}

impl PartialEq for CompiledRule {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern && self.style == other.style
    }
}
```

- [ ] **Step 5: Add the `ColorScheme` fields**

In `crates/app/src/colors.rs`, add to the `ColorScheme` struct (after `loc_indicator`):

```rust
    /// Player input echo text.
    pub transcript_input: Style,
    /// Meta (app/slash) text.
    pub transcript_meta: Style,
    /// VM warning text.
    pub transcript_warning: Style,
    /// Built-in story rule: room-name / location header line.
    pub transcript_location: Style,
    /// Built-in story rule: bracketed system line.
    pub transcript_system: Style,
    /// Gutter marker style for warning lines.
    pub warning_marker: Style,
    /// Compiled user story-styling rules, in evaluation order.
    pub transcript_rules: Vec<CompiledRule>,
```

- [ ] **Step 6: Set the defaults in `terminal_default`**

In `ColorScheme::terminal_default`, add to the struct literal (after `loc_indicator: …`):

```rust
            transcript_input: Style::new().fg(Color::Cyan),
            transcript_meta: Style::new().fg(Color::DarkGray),
            transcript_warning: Style::new().fg(Color::Yellow),
            transcript_location: Style::new().add_modifier(Modifier::BOLD),
            transcript_system: Style::new().fg(Color::DarkGray),
            warning_marker: Style::new().fg(Color::Yellow),
            transcript_rules: Vec::new(),
```

- [ ] **Step 7: Set the defaults in `from_ghostty`**

In `ColorScheme::from_ghostty`, add to the struct literal (after `loc_indicator: …`):

```rust
            transcript_input: Style::new().fg(scheme.palette[6]),
            transcript_meta: Style::new().fg(scheme.palette[8]),
            transcript_warning: Style::new().fg(scheme.palette[3]),
            transcript_location: Style::new().add_modifier(Modifier::BOLD),
            transcript_system: Style::new().fg(scheme.palette[8]),
            warning_marker: Style::new().fg(scheme.palette[3]),
            transcript_rules: Vec::new(),
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (Existing `resolve_empty_doc_equals_terminal_default` and `write_style_full_is_self_contained` stay green: both sides carry identical defaults and empty rules.)

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/Cargo.toml crates/app/src/colors.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): per-category transcript Style fields + CompiledRule (regex)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: Style selectors — register, apply, and export the six new selectors

**Files:**
- Modify: `crates/app/src/style.rs` (`SELECTOR_FIELDS` ~115; `apply_color_decls` ~158; `write_style_full` ~734; tests)

**Interfaces:**
- Consumes: `ColorScheme.transcript_input/meta/warning/location/system`, `warning_marker` (Task 2).
- Produces: selectors `transcript:input`, `transcript:meta`, `transcript:warning`, `transcript:location`, `transcript:system`, `warning_marker` — parse, apply, and round-trip through `write_style_full`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn transcript_category_selectors_parse_and_apply() {
    let doc = parse_style_toml(
        "[colors]\n\
         \"transcript:input\" = { fg = \"green\" }\n\
         \"transcript:meta\" = { fg = \"blue\" }\n\
         \"transcript:warning\" = { fg = \"red\" }\n\
         \"transcript:location\" = { bold = true }\n\
         \"transcript:system\" = { fg = \"magenta\" }\n\
         \"warning_marker\" = { fg = \"red\" }\n"
    ).unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    assert!(warnings.is_empty(), "{warnings:?}");
    use ratatui::style::{Color, Modifier};
    assert_eq!(cs.transcript_input.fg, Some(Color::Green));
    assert_eq!(cs.transcript_meta.fg, Some(Color::Blue));
    assert_eq!(cs.transcript_warning.fg, Some(Color::Red));
    assert!(cs.transcript_location.add_modifier.contains(Modifier::BOLD));
    assert_eq!(cs.transcript_system.fg, Some(Color::Magenta));
    assert_eq!(cs.warning_marker.fg, Some(Color::Red));
}

#[test]
fn write_style_full_round_trips_transcript_categories() {
    use ratatui::style::Color;
    let dir = std::env::temp_dir().join(format!("babelmap-style-tcat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tcat.toml");
    let mut cs = crate::colors::ColorScheme::terminal_default();
    cs.transcript_input = Style::new().fg(Color::Green);
    cs.transcript_warning = Style::new().fg(Color::Magenta);
    let set = crate::symbols::SymbolSet::resolve(&crate::config::SymbolConfig::default());
    write_style_full(&path, &cs, &set).unwrap();
    let doc = parse_style_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let (cs2, _set2, _w) = resolve(&doc, &dir);
    assert_eq!(cs2.transcript_input.fg, Some(Color::Green));
    assert_eq!(cs2.transcript_warning.fg, Some(Color::Magenta));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app transcript_category_selectors_parse_and_apply write_style_full_round_trips_transcript_categories`
Expected: FAIL — `transcript_category_selectors_parse_and_apply` reports unknown-selector warnings; round-trip mismatches.

- [ ] **Step 3: Register the selectors**

In `crates/app/src/style.rs`, add to `SELECTOR_FIELDS` (after `"transcript",`):

```rust
    "transcript:input",
    "transcript:meta",
    "transcript:warning",
    "transcript:location",
    "transcript:system",
    "warning_marker",
```

- [ ] **Step 4: Apply the selectors**

In `apply_color_decls`, add match arms (after the `"transcript"` arm):

```rust
            "transcript:input"    => cs.transcript_input = cs.transcript_input.patch(style),
            "transcript:meta"     => cs.transcript_meta = cs.transcript_meta.patch(style),
            "transcript:warning"  => cs.transcript_warning = cs.transcript_warning.patch(style),
            "transcript:location" => cs.transcript_location = cs.transcript_location.patch(style),
            "transcript:system"   => cs.transcript_system = cs.transcript_system.patch(style),
            "warning_marker"      => cs.warning_marker = cs.warning_marker.patch(style),
```

- [ ] **Step 5: Export the selectors in `write_style_full`**

In `write_style_full`, after the `"transcript"` insert (~752), add:

```rust
    doc.colors.selectors.insert("transcript:input".to_string(),    style_to_decl(&cs.transcript_input));
    doc.colors.selectors.insert("transcript:meta".to_string(),     style_to_decl(&cs.transcript_meta));
    doc.colors.selectors.insert("transcript:warning".to_string(),  style_to_decl(&cs.transcript_warning));
    doc.colors.selectors.insert("transcript:location".to_string(), style_to_decl(&cs.transcript_location));
    doc.colors.selectors.insert("transcript:system".to_string(),   style_to_decl(&cs.transcript_system));
    doc.colors.selectors.insert("warning_marker".to_string(),      style_to_decl(&cs.warning_marker));
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): transcript category + warning_marker style selectors

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: User rules — parse `[[transcript.rule]]`, compile in resolve, merge

**Files:**
- Modify: `crates/app/src/style.rs` (`RawRule` type; `StyleDoc` field; `parse_style_toml`; `merge`; `resolve`; tests)

**Interfaces:**
- Consumes: `Decl`, `decl_to_style`, `CompiledRule` (Task 2), `colors::resolve_base` (returns the active `GhosttyScheme`).
- Produces: `StyleDoc.transcript_rules: Vec<RawRule>`; `pub struct RawRule { pub pattern: String, pub decl: Decl }`; `resolve` populates `cs.transcript_rules`, pushing a warning and skipping any rule whose regex fails to compile.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/style.rs`, inside `mod tests`, add:

```rust
#[test]
fn transcript_rules_parse_compile_in_order() {
    let text = r##"
[colors]
[[transcript.rule]]
match = "^>.*"
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"
fg = "red"
"##;
    let doc = parse_style_toml(text).unwrap();
    assert_eq!(doc.transcript_rules.len(), 2);
    assert_eq!(doc.transcript_rules[0].pattern, "^>.*");
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(cs.transcript_rules.len(), 2);
    assert!(cs.transcript_rules[0].regex.is_match("> go north"));
    assert!(cs.transcript_rules[1].regex.is_match("A lurking GRUE!"));
    use ratatui::style::Color;
    assert_eq!(cs.transcript_rules[0].style.fg, Some(Color::Magenta));
}

#[test]
fn invalid_transcript_rule_warns_and_skips() {
    let text = r##"
[colors]
[[transcript.rule]]
match = "("
fg = "red"

[[transcript.rule]]
match = "ok"
fg = "green"
"##;
    let doc = parse_style_toml(text).unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    assert_eq!(warnings.len(), 1, "exactly one invalid-regex warning: {warnings:?}");
    assert_eq!(cs.transcript_rules.len(), 1, "valid rule still loads");
    assert!(cs.transcript_rules[0].regex.is_match("ok"));
}

#[test]
fn merge_replaces_transcript_rules_when_override_has_any() {
    let mut base = StyleDoc::default();
    base.transcript_rules.push(RawRule { pattern: "a".into(), decl: Decl::default() });
    let mut over = StyleDoc::default();
    over.transcript_rules.push(RawRule { pattern: "b".into(), decl: Decl::default() });
    let m = merge(&base, &over);
    assert_eq!(m.transcript_rules.len(), 1);
    assert_eq!(m.transcript_rules[0].pattern, "b");
    // Empty override keeps base rules.
    let m2 = merge(&base, &StyleDoc::default());
    assert_eq!(m2.transcript_rules[0].pattern, "a");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app transcript_rules_parse_compile_in_order invalid_transcript_rule_warns_and_skips merge_replaces_transcript_rules_when_override_has_any`
Expected: compile error (missing type/field).

- [ ] **Step 3: Define `RawRule` and add the `StyleDoc` field**

In `crates/app/src/style.rs`, add the type (near `StyleDoc`):

```rust
/// A raw (uncompiled) user transcript-styling rule from `[[transcript.rule]]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawRule {
    /// The regex source string (from the rule's `match` key).
    pub pattern: String,
    /// The fg/bg/bold/italic style fields applied on a match.
    pub decl: Decl,
}
```

In the `StyleDoc` struct, add a field:

```rust
    /// User story-styling rules from `[[transcript.rule]]`, in file order.
    pub transcript_rules: Vec<RawRule>,
```

- [ ] **Step 4: Parse the rules in `parse_style_toml`**

In `parse_style_toml`, after the `[symbols]` block and before `Ok(StyleDoc { … })`, add:

```rust
    let mut transcript_rules: Vec<RawRule> = Vec::new();
    if let Some(toml::Value::Table(tr_table)) = root.get("transcript") {
        if let Some(toml::Value::Array(rules)) = tr_table.get("rule") {
            for item in rules {
                if let toml::Value::Table(rt) = item {
                    let pattern = rt
                        .get("match")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if pattern.is_empty() {
                        continue; // a rule with no `match` is skipped
                    }
                    let decl = parse_decl_from_table(rt);
                    transcript_rules.push(RawRule { pattern, decl });
                }
            }
        }
    }
```

Then change the return to:

```rust
    Ok(StyleDoc { colors, symbols, transcript_rules })
```

- [ ] **Step 5: Carry rules through `merge`**

In `merge`, before the final `StyleDoc { … }` return, add:

```rust
    let transcript_rules = if over.transcript_rules.is_empty() {
        base.transcript_rules.clone()
    } else {
        over.transcript_rules.clone()
    };
```

And update the returned struct to include `transcript_rules`:

```rust
    StyleDoc {
        colors: StyleColors { scheme, selectors },
        symbols,
        transcript_rules,
    }
```

- [ ] **Step 6: Compile the rules in `resolve`**

In `resolve`, after `warnings.extend(selector_warnings);` and before resolving symbols, add:

```rust
    // Compile user transcript rules; an invalid regex warns and is skipped.
    for r in &doc.transcript_rules {
        match regex::Regex::new(&r.pattern) {
            Ok(rx) => cs.transcript_rules.push(crate::colors::CompiledRule {
                pattern: r.pattern.clone(),
                regex: rx,
                style: decl_to_style(&r.decl, &gs),
            }),
            Err(e) => warnings.push(format!("invalid transcript rule regex '{}': {}", r.pattern, e)),
        }
    }
```

- [ ] **Step 7: Fix the other `StyleDoc` literal**

`style_from_config` (~426) constructs `StyleDoc { colors: …, symbols: … }` as a literal, which no longer compiles with the new field. Update it:

```rust
pub fn style_from_config(colors: &StyleColors, symbols: &StyleSymbols) -> StyleDoc {
    StyleDoc {
        colors: colors.clone(),
        symbols: symbols.clone(),
        transcript_rules: Vec::new(),
    }
}
```

(Config-embedded rules are not supported — only `style.toml` `[[transcript.rule]]`. `StyleDoc::default()` and `write_style_full`'s `StyleDoc::default()` already cover the new field via `Default`.)

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): parse and compile user [[transcript.rule]] regex rules

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: Story style resolution — `resolve_story_style` with built-in location/system

**Files:**
- Modify: `crates/app/src/colors.rs` (`impl ColorScheme` method + tests)

**Interfaces:**
- Consumes: `cs.transcript`, `cs.transcript_location`, `cs.transcript_system`, `cs.transcript_rules` (Tasks 2/4); `zvm::location::status_name_matches`.
- Produces: `ColorScheme::resolve_story_style(&self, line: &str, room_name: Option<&str>) -> Style` — first user rule match wins, else built-in location, else built-in system, else base `transcript`; each match patches over the base.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/colors.rs`, inside `mod tests`, add:

```rust
#[test]
fn resolve_story_style_precedence_and_patch() {
    use ratatui::style::{Color, Modifier};
    let mut cs = ColorScheme::terminal_default(); // transcript fg = White
    // A user rule that only sets bold (no fg) → patch keeps base fg.
    cs.transcript_rules.push(CompiledRule {
        pattern: "^>".into(),
        regex: regex::Regex::new("^>").unwrap(),
        style: Style::new().add_modifier(Modifier::BOLD),
    });

    // 1. User rule wins, patch semantics: bold added, base White fg kept.
    let s = cs.resolve_story_style("> go north", Some("West of House"));
    assert!(s.add_modifier.contains(Modifier::BOLD));
    assert_eq!(s.fg, Some(Color::White));

    // 2. Built-in location: line equals room name → bold (transcript_location).
    let loc = cs.resolve_story_style("West of House", Some("West of House"));
    assert!(loc.add_modifier.contains(Modifier::BOLD));

    // 2b. Boundary guard: "Hall" line vs room "Hallway" must NOT match location.
    let no_loc = cs.resolve_story_style("Hall", Some("Hallway"));
    assert!(!no_loc.add_modifier.contains(Modifier::BOLD));
    assert_eq!(no_loc, cs.transcript); // falls through to base

    // 3. Built-in system: bracketed line → transcript_system (DarkGray).
    let sys = cs.resolve_story_style("[Your score just went up by ten points.]", None);
    assert_eq!(sys.fg, Some(Color::DarkGray));

    // 4. No match → base transcript.
    assert_eq!(cs.resolve_story_style("plain prose", None), cs.transcript);

    // 5. None room name → location never matches.
    assert_eq!(cs.resolve_story_style("West of House", None), cs.transcript);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app resolve_story_style_precedence_and_patch`
Expected: compile error (no such method).

- [ ] **Step 3: Implement `resolve_story_style`**

In `crates/app/src/colors.rs`, add an `impl ColorScheme` block (or extend the existing one) with:

```rust
impl ColorScheme {
    /// Resolve the style for one Story line: first matching user rule wins, else
    /// the built-in location rule (line matches `room_name`), else the built-in
    /// system rule (whole line bracketed), else the base `transcript` style.
    /// A match patches its style over `transcript` (overriding only set fields).
    pub fn resolve_story_style(&self, line: &str, room_name: Option<&str>) -> Style {
        for rule in &self.transcript_rules {
            if rule.regex.is_match(line) {
                return self.transcript.patch(rule.style);
            }
        }
        if let Some(name) = room_name {
            if zvm::location::status_name_matches(line, name) {
                return self.transcript.patch(self.transcript_location);
            }
        }
        let t = line.trim();
        if t.len() >= 2 && t.starts_with('[') && t.ends_with(']') {
            return self.transcript.patch(self.transcript_system);
        }
        self.transcript
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p app resolve_story_style_precedence_and_patch`
Expected: PASS.

- [ ] **Step 5: Run the full app suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/colors.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): ColorScheme::resolve_story_style with built-in location/system

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: SymbolSet — meta_gutter / warning_gutter glyph slots

**Files:**
- Modify: `crates/app/src/symbols.rs` (`SymbolSet` struct + `Default` + `resolve` + `apply_override` + tests)
- Modify: `crates/app/src/style.rs` (`write_style_full` symbol-override export ~860)

**Interfaces:**
- Produces: `SymbolSet.meta_gutter: char` (default `▏`), `SymbolSet.warning_gutter: char` (default `!`); override keys `gutter.meta`, `gutter.warning`; exported by `write_style_full`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/symbols.rs`, inside `mod tests`, add:

```rust
#[test]
fn gutter_glyph_defaults_and_overrides() {
    let s = SymbolSet::default();
    assert_eq!(s.meta_gutter, '▏');
    assert_eq!(s.warning_gutter, '!');
    // resolve(default) keeps defaults.
    assert_eq!(SymbolSet::resolve(&crate::config::SymbolConfig::default()), SymbolSet::default());
    // overrides apply.
    let mut cfg = crate::config::SymbolConfig::default();
    cfg.overrides.insert("gutter.meta".into(), "|".into());
    cfg.overrides.insert("gutter.warning".into(), "*".into());
    let r = SymbolSet::resolve(&cfg);
    assert_eq!(r.meta_gutter, '|');
    assert_eq!(r.warning_gutter, '*');
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app gutter_glyph_defaults_and_overrides`
Expected: compile error (missing fields).

- [ ] **Step 3: Add the fields to `SymbolSet`**

In `crates/app/src/symbols.rs`, add to the `SymbolSet` struct (after `portal: PortalGlyphs,`):

```rust
    /// Gutter marker glyph for META transcript lines.
    pub meta_gutter: char,
    /// Gutter marker glyph for WARNING transcript lines.
    pub warning_gutter: char,
```

- [ ] **Step 4: Set the defaults**

In `impl Default for SymbolSet`, add to the returned struct (after `portal: PortalGlyphs { … },`):

```rust
            meta_gutter: '▏',
            warning_gutter: '!',
```

- [ ] **Step 5: Initialize them in `resolve`**

In `SymbolSet::resolve`, add to the `let mut s = SymbolSet { … }` literal (after `portal: …`):

```rust
            meta_gutter: SymbolSet::default().meta_gutter,
            warning_gutter: SymbolSet::default().warning_gutter,
```

- [ ] **Step 6: Handle the override keys**

In `apply_override`, add match arms (before the `_ => {}` catch-all):

```rust
        "gutter.meta"      => s.meta_gutter = ch,
        "gutter.warning"   => s.warning_gutter = ch,
```

Note: `!` and `|` and `*` are single-byte ASCII, accepted by the existing width validation in `resolve`.

- [ ] **Step 7: Export the slots in `write_style_full`**

In `crates/app/src/style.rs`, in `write_style_full`, after the portal overrides (`ov.insert("portal.marker", …)`), add:

```rust
    ov.insert("gutter.meta".to_string(),    set.meta_gutter.to_string());
    ov.insert("gutter.warning".to_string(), set.warning_gutter.to_string());
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (`write_style_full_is_self_contained` exercises the new override round-trip via `SymbolSet::resolve`.)

- [ ] **Step 9: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/symbols.rs crates/app/src/style.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): SymbolSet meta_gutter / warning_gutter glyph slots

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: Render wiring — per-kind styles, gutter glyphs, story rules

**Files:**
- Modify: `crates/app/src/render/transcript.rs` (`render_middle` render loop ~611; remove now-unused `META_MARKER` const ~150; tests)

**Interfaces:**
- Consumes: `ColorScheme.resolve_story_style` (Task 5), `transcript_input/meta/warning`, `meta_marker`, `warning_marker`; `SymbolSet.meta_gutter/warning_gutter` (Task 6); `AppState.current_room_name` (Task 1).
- Produces: final per-kind rendering — Story (rule-resolved style, no gutter), Input (`transcript_input`, no gutter), Meta (`meta_gutter` glyph + `meta_marker`, `transcript_meta` text), Warning (`warning_gutter` glyph + `warning_marker`, `transcript_warning` text).

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/render/transcript.rs`, inside `mod tests`, add:

```rust
#[test]
fn render_kinds_draw_their_own_styles_and_gutters() {
    use ratatui::style::Color;
    let machine = minimal_machine();
    let mut state = AppState::default();
    state.push_transcript_kind("> go north", TranscriptKind::Input);
    state.push_transcript_kind("app message", TranscriptKind::Meta);
    state.push_transcript_kind("VAR 0x15 unimplemented", TranscriptKind::Warning);
    state.focus = Focus::Game;

    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    render_transcript(&machine, &state, area, &mut buf);

    // Find the row index (1..8) for each tagged line by its first glyph / content.
    let row_text = |y: u16| -> String {
        (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect()
    };
    // Locate the warning row by its gutter glyph '!' in column 0.
    let warn_y = (1u16..9).find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("!"))
        .expect("warning gutter '!' must appear in column 0");
    // Warning gutter cell uses warning_marker (Yellow).
    assert_eq!(buf.cell((0, warn_y)).unwrap().style().fg, Some(Color::Yellow));
    // Warning text is indented past the 2-col gutter and uses transcript_warning (Yellow).
    assert_eq!(buf.cell((2, warn_y)).unwrap().style().fg, Some(Color::Yellow));

    // Meta row: gutter glyph '▏' in column 0.
    let meta_y = (1u16..9).find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("▏"))
        .expect("meta gutter '▏' must appear in column 0");
    assert_eq!(buf.cell((2, meta_y)).unwrap().style().fg, Some(Color::DarkGray)); // transcript_meta

    // Input row: no gutter (text at column 0), cyan fg.
    let input_y = (1u16..9).find(|&y| row_text(y).starts_with("> go north"))
        .expect("input line must render at column 0");
    assert_eq!(buf.cell((0, input_y)).unwrap().style().fg, Some(Color::Cyan)); // transcript_input
}

#[test]
fn render_story_location_line_is_bold() {
    use ratatui::style::Modifier;
    let machine = minimal_machine();
    let mut state = AppState::default();
    state.current_room_name = Some("West of House".to_string());
    state.push_transcript("West of House"); // Story
    state.focus = Focus::Game;

    let area = Rect::new(0, 0, 40, 10);
    let mut buf = Buffer::empty(area);
    render_transcript(&machine, &state, area, &mut buf);

    let y = (1u16..9).find(|&y| {
        let row: String = (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
        row.starts_with("West of House")
    }).expect("location line must render");
    assert!(buf.cell((0, y)).unwrap().modifier.contains(Modifier::BOLD), "location header must be bold");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app render_kinds_draw_their_own_styles_and_gutters render_story_location_line_is_bold`
Expected: FAIL — Input not cyan (still base), Warning gutter still `▏`/meta_marker, location not bold.

- [ ] **Step 3: Replace the render-loop match with full per-kind styling**

In `crates/app/src/render/transcript.rs`, in `render_middle`, replace the entire `for (i, (line, kind)) in lines.iter().enumerate() { … }` body's `match kind { … }` with:

```rust
        match kind {
            TranscriptKind::Story => {
                let style = state.colors.resolve_story_style(line, state.current_room_name.as_deref());
                if has_search {
                    draw_str_highlighted(buf, area.x, row_y, line, style, &query_lower, search_highlight_style, area);
                } else {
                    draw_str_clipped(buf, area.x, row_y, line, style, area);
                }
            }
            TranscriptKind::Input => {
                let style = state.colors.transcript_input;
                if has_search {
                    draw_str_highlighted(buf, area.x, row_y, line, style, &query_lower, search_highlight_style, area);
                } else {
                    draw_str_clipped(buf, area.x, row_y, line, style, area);
                }
            }
            TranscriptKind::Meta => {
                let glyph = state.symbols.meta_gutter.to_string();
                draw_str_clipped(buf, area.x, row_y, &glyph, state.colors.meta_marker, area);
                let style = state.colors.transcript_meta;
                if has_search {
                    draw_str_highlighted(buf, area.x + META_GUTTER, row_y, line, style, &query_lower, search_highlight_style, area);
                } else {
                    draw_str_clipped(buf, area.x + META_GUTTER, row_y, line, style, area);
                }
            }
            TranscriptKind::Warning => {
                let glyph = state.symbols.warning_gutter.to_string();
                draw_str_clipped(buf, area.x, row_y, &glyph, state.colors.warning_marker, area);
                let style = state.colors.transcript_warning;
                if has_search {
                    draw_str_highlighted(buf, area.x + META_GUTTER, row_y, line, style, &query_lower, search_highlight_style, area);
                } else {
                    draw_str_clipped(buf, area.x + META_GUTTER, row_y, line, style, area);
                }
            }
        }
```

- [ ] **Step 4: Remove the now-unused `META_MARKER` const**

In `crates/app/src/render/transcript.rs`, delete the `META_MARKER` constant (~150):

```rust
/// The gutter glyph drawn beside META (app/slash) output.
pub(crate) const META_MARKER: &str = "▏";
```

Keep `META_GUTTER` (still used for the 2-col indent). Two locals in `render_middle` are now orphaned by this change — remove whichever the compiler flags:
- `let marker_style = state.colors.meta_marker;` (~605) — each gutter arm now reads the marker style directly.
- The `normal_style: Style` parameter of `render_middle` — the loop no longer uses it (each kind resolves its own style; Story's base comes from `resolve_story_style`). If `cargo build` warns it is unused, rename it to `_normal_style` in the `render_middle` signature. Do NOT touch `normal_style` in `render_transcript` — it is still used by `render_input_content`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/render/transcript.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): render transcript per-kind styles, gutters, story rules

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 8: Resize redraw fix — full clear on `Event::Resize`

**Files:**
- Modify: `crates/app/src/main.rs` (`Event::Resize` arms at ~947, ~1028, ~1084, ~1142, ~1284)

**Interfaces:**
- Consumes: `terminal` (in scope in the run loop, declared ~773).
- Produces: every `Event::Resize` arm forces a full repaint via `terminal.clear()` before continuing, eliminating stale cells (the reported gutter artifact).

This task is mechanical terminal I/O; it is verified by build + the existing suite (no new unit test — the symptom is a live-terminal redraw artifact that the buffer-level tests cannot reproduce). Manual verification step included.

- [ ] **Step 1: Update every `Event::Resize` arm**

In `crates/app/src/main.rs`, locate each `Event::Resize(_, _)` arm. There are two shapes:

```rust
                Event::Resize(_, _) => { continue; }
```
and
```rust
            Event::Resize(_, _) => continue,
```

Replace each with a clear-then-continue. For the block form:

```rust
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
```

For the expression form (the main loop, ~1284):

```rust
            // Resize: force a full repaint so no stale cells survive the size change.
            Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
```

Apply to all five sites (~947, ~1028, ~1084, ~1142, ~1284). `terminal` is in scope at each (the run loop owns it). If the compiler reports `terminal` is not in scope at a given site, leave that one unchanged and note it in the task report.

- [ ] **Step 2: Build and run the suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean, 0 warnings; suite PASS.

- [ ] **Step 3: Manual verification (record in the task report)**

Run `cargo run -p app -- crates/zvm/tests/fixtures/minizork.z3` (or any story), trigger a VM warning / meta line long enough to wrap, then resize the terminal narrower and wider. Confirm the gutter and wrapped text stay aligned with no leftover characters in the gutter columns. Record the observation in the report. (If a graphical terminal is unavailable in the execution environment, state that manual verification is deferred to the human and the code change matches the standard ratatui clear-on-resize remedy.)

- [ ] **Step 4: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/main.rs
git -C /Volumes/Videos/Source/babelmap commit -m "fix(app): clear terminal on resize to drop stale gutter cells

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Tasks are ordered by dependency: 1 (data model) → 2 (ColorScheme fields) → 3 (selectors) → 4 (user rules) → 5 (story resolution) → 6 (gutter glyphs) → 7 (render wiring) → 8 (resize fix). Task 6 and Task 8 are independent of the 3→4→5 chain and may be reordered, but keep 7 last among the render-affecting tasks.
- After every task: `cargo test -p app` must be green with zero warnings before committing.
- `README.md` is committed and kept current; if any user-facing styling docs exist there, the final review should flag whether the new `transcript:*` selectors, `[[transcript.rule]]`, and `gutter.*` overrides need a doc line. (No README task is included here; raise it at final review.)
- `TODO.md` is gitignored — never stage it.
