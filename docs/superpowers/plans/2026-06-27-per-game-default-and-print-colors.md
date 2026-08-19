# Per-Game Default Freeze + print-colors — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or execute directly task-by-task with TDD. Steps use checkbox (`- [ ]`) syntax.

**Goal:** (A) make per-game style files freeze fields explicitly set to terminal-default by representing them with the existing `reset` token; (B) add a `/print-colors [color]` command that prints the live color scheme to the transcript, with an optional actual-color rendering mode backed by a general per-line transcript style override.

**Specs:** `docs/superpowers/specs/2026-06-27-per-game-default-reset-design.md` and `docs/superpowers/specs/2026-06-27-print-colors-command-design.md`.

**Architecture:** Five independent TDD tasks. A1–A2 fix the reset round-trip (parse alias + editor stores `reset`). B1 adds the transcript per-line style override; B2 adds the `describe_scheme` formatter; B3 adds the command + wiring. Tasks A and B are independent; within B, B3 depends on B1 and B2.

## Global Constraints

- 0 warnings from `cargo build -p app` and full `cargo test -p app` green (confirm EVERY "test result:" line shows 0 failed; the lib binary has ~758 tests) before each commit.
- Commit-only on local `main`; one commit per task. No push.
- Commit body has NO backticks; end every commit with exactly:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Do NOT edit TODO.md. Surgical changes. Style-editor tests use `open_style_editor_hermetic`.
- Use `git -C /Volumes/Videos/Source/lanthorn` for git.

---

### Task A1: `parse_color_value` aliases `default` to `Color::Reset` + round-trip freeze test

**Files:** Modify `crates/app/src/colors.rs` (`parse_color_value`); Test in `crates/app/src/style.rs` (`mod tests`) and `crates/app/src/colors.rs` (`mod tests`).

**Interfaces:** `parse_color_value(value, scheme) -> Option<Color>` unchanged signature. Relies on existing `merge`, `resolve`, `color_to_str(Reset) == "reset"`, `decl_to_style`.

- [ ] **Step 1: Write the failing tests.**

In `crates/app/src/colors.rs` `mod tests`:

```rust
#[test]
fn parse_color_value_maps_default_and_reset_to_reset() {
    let gs = GhosttyScheme::default();
    assert_eq!(parse_color_value("default", &gs), Some(Color::Reset));
    assert_eq!(parse_color_value("reset", &gs), Some(Color::Reset));
}
```

In `crates/app/src/style.rs` `mod tests` (the round-trip freeze regression):

```rust
#[test]
fn per_game_reset_freezes_over_global_color() {
    // Global sets room.fg = white; per-game sets room.fg = reset (the editor's
    // serialized form of an explicit "default"). Merge must let per-game win, and
    // resolve must produce a terminal-default (Reset) fg, NOT white.
    let global = parse_style_toml("[colors]\n\"room\" = { fg = \"white\" }\n[symbols]\n").unwrap();
    let per_game = parse_style_toml("[colors]\n\"room\" = { fg = \"reset\" }\n[symbols]\n").unwrap();
    let merged = merge(&global, &per_game);
    let dir = std::env::temp_dir();
    let (cs, _set, _w) = resolve(&merged, &dir);
    assert_eq!(cs.room_normal.fg, Some(ratatui::style::Color::Reset),
        "per-game reset must win over the global color and resolve to terminal default");
}
```

(If `GhosttyScheme::default()` is not available in the colors test module, build the scheme the same way the neighboring `parse_color_value_*` tests do.)

- [ ] **Step 2: Run tests, verify they fail.**

Run: `cargo test -p app --lib parse_color_value_maps_default_and_reset_to_reset per_game_reset_freezes_over_global_color`
Expected: `parse_color_value_maps_default_and_reset_to_reset` FAILS (`"default"` currently returns `None`); the freeze test FAILS (room fg resolves to `White` because `"default"`/omitted doesn't win — wait, here per-game uses `"reset"` which already parses, so this test may already pass; if it passes, keep it as a guard and note that A1's behavior change is only the `"default"` alias).

- [ ] **Step 3: Implement the alias.**

In `crates/app/src/colors.rs` `parse_color_value`, in the scheme-relative role `match v { ... }` block (the one with `"background"`/`"foreground"`), add a `default` arm:

```rust
    match v {
        "background" => return Some(scheme.background),
        "foreground" => return Some(scheme.foreground),
        "default" => return Some(Color::Reset),
        _ => {}
    }
```

- [ ] **Step 4: Run tests, verify pass.** Run the Step 2 command; expected PASS.

- [ ] **Step 5: Full suite + commit.**

```
cargo test -p app && cargo build -p app
git add crates/app/src/colors.rs crates/app/src/style.rs
git commit -m "feat(app): parse default as Color::Reset; per-game reset freezes over global"
```

---

### Task A2: Editor stores `reset` for the "default" selection + swatch highlight recognizes it

**Files:** Modify `crates/app/src/main.rs` (fg/bg swatch default-cell clicks), `crates/app/src/input.rs` (`StyleSwatchPick`, `StyleCommitCustom`), `crates/app/src/render/style_editor.rs` (`draw_swatch_row` default-cell `is_selected`). Tests in `crates/app/src/input.rs` `mod tests`.

**Interfaces:** Consumes `Action::StyleSetColor { is_bg, value: Option<String> }`. After this task, picking "default" sets the active selector's `Decl.fg`/`bg` to `Some("reset")`.

- [ ] **Step 1: Write the failing tests** in `crates/app/src/input.rs` `mod tests`:

```rust
#[test]
fn swatch_pick_default_cell_sets_reset() {
    let mut s = AppState::default();
    open_style_editor_hermetic(&mut s);
    {
        let ed = s.style_editor.as_mut().unwrap();
        ed.color_target = false; // fg
        ed.swatch_cursor = crate::style_mru::ANSI_NAMES.len(); // the "default" cell
    }
    apply_action(Action::StyleSwatchPick, &mut s, &mut mapper::mapper::Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    let sel = ed.selectors[ed.active].to_string();
    assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.as_deref()), Some("reset"),
        "picking the default swatch cell stores the reset token");
}

#[test]
fn custom_commit_default_stores_reset() {
    let mut s = AppState::default();
    open_style_editor_hermetic(&mut s);
    {
        let ed = s.style_editor.as_mut().unwrap();
        ed.color_target = false; // fg
        ed.custom_buf = "default".into();
    }
    apply_action(Action::StyleCommitCustom, &mut s, &mut mapper::mapper::Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    let sel = ed.selectors[ed.active].to_string();
    assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.as_deref()), Some("reset"),
        "typing default in the custom field stores the reset token");
}
```

- [ ] **Step 2: Run, verify fail.** Run: `cargo test -p app --lib swatch_pick_default_cell_sets_reset custom_commit_default_stores_reset` — expected FAIL (both currently produce `None`).

- [ ] **Step 3: Implement.**

`crates/app/src/main.rs`, the fg swatch loop (the `let value = if i < ANSI_NAMES.len() { Some(...) } else { None };` near "Fg swatch row") — change the `else` branch to `Some("reset".to_string())`. Do the same in the bg swatch loop just below it. (Both currently read `} else { None };` → `} else { Some("reset".to_string()) };`.)

`crates/app/src/input.rs` `Action::StyleSwatchPick` handler — currently:
```rust
        Action::StyleSwatchPick => {
            if let Some(ed) = &state.style_editor {
                let is_bg = ed.color_target;
                let cur = ed.swatch_cursor;
                let value = crate::style_mru::ANSI_NAMES.get(cur).map(|s| s.to_string());
                ...
```
Change the `value` line to map the default cell to `reset`:
```rust
                let value = if cur == crate::style_mru::ANSI_NAMES.len() {
                    Some("reset".to_string())
                } else {
                    crate::style_mru::ANSI_NAMES.get(cur).map(|s| s.to_string())
                };
```

`crates/app/src/input.rs` `Action::StyleCommitCustom` handler — currently:
```rust
                    let value = if ed.custom_buf == "default" { None } else { Some(ed.custom_buf.clone()) };
```
Change to:
```rust
                    let value = if ed.custom_buf == "default" { Some("reset".to_string()) } else { Some(ed.custom_buf.clone()) };
```

`crates/app/src/render/style_editor.rs` `draw_swatch_row`, the default cell:
```rust
        let is_selected = current_val == "default";
```
Change to:
```rust
        let is_selected = current_val == "default" || current_val == "reset";
```

- [ ] **Step 4: Run, verify pass.** Run the Step 2 command — expected PASS.

- [ ] **Step 5: Full suite + commit.**

```
cargo test -p app && cargo build -p app
git add crates/app/src/main.rs crates/app/src/input.rs crates/app/src/render/style_editor.rs
git commit -m "feat(app): editor default selection stores reset so per-game defaults freeze"
```

---

### Task B1: Per-line transcript style override

**Files:** Modify `crates/app/src/state.rs` (new `transcript_styles` field + both `Default`/constructor sites + `push_transcript_kind` self-heal + new `push_transcript_styled`); Modify `crates/app/src/render/transcript.rs` (`filtered_styles` read). Tests in `crates/app/src/state.rs` `mod tests`.

**Interfaces:** Produces `AppState.transcript_styles: Vec<Option<ratatui::style::Style>>` and `pub fn push_transcript_styled(&mut self, text: &str, kind: TranscriptKind, style: Style)`. `push_transcript_kind` keeps the override length-synced.

- [ ] **Step 1: Write the failing test** in `crates/app/src/state.rs` `mod tests`:

```rust
#[test]
fn transcript_styles_track_and_self_heal() {
    use ratatui::style::{Color, Style};
    let mut s = AppState::default();
    s.push_transcript_kind("a", TranscriptKind::Meta);
    let cyan = Style::new().fg(Color::Cyan);
    s.push_transcript_styled("b", TranscriptKind::Meta, cyan);
    assert_eq!(s.transcript.len(), s.transcript_styles.len(), "lengths stay equal");
    assert_eq!(s.transcript_styles[0], None, "plain push has no override");
    assert_eq!(s.transcript_styles[1], Some(cyan), "styled push records the style");

    // Simulate a wholesale reassignment that leaves transcript_styles short.
    s.transcript = vec!["x".into(), "y".into(), "z".into()];
    s.transcript_kinds = vec![TranscriptKind::Story; 3];
    s.push_transcript_kind("w", TranscriptKind::Meta); // must self-heal
    assert_eq!(s.transcript.len(), s.transcript_styles.len(), "self-heal re-aligns lengths");
}
```

- [ ] **Step 2: Run, verify fail.** Run: `cargo test -p app --lib transcript_styles_track_and_self_heal` — expected FAIL (field/method missing → compile error).

- [ ] **Step 3: Implement.**

Add the field to the `AppState` struct (near `transcript_kinds`):
```rust
    /// Optional per-line render-style override, parallel to `transcript`. In-memory
    /// only (not persisted). `None` = use the line's per-kind style. Kept length-
    /// synced by `push_transcript_kind`; read defensively by the renderer.
    pub transcript_styles: Vec<Option<ratatui::style::Style>>,
```

Add `transcript_styles: Vec::new(),` to the `Default` impl (next to `transcript_kinds: Vec::new(),` ~line 901) AND `transcript_styles: vec![],` to the other constructor (~line 1718, next to `transcript: vec![],`).

Update `push_transcript_kind` (and add the styled variant) at state.rs ~1107:
```rust
    pub fn push_transcript_kind(&mut self, text: &str, kind: TranscriptKind) {
        self.transcript_styles.resize(self.transcript.len(), None); // self-heal alignment
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(None);
        }
    }

    /// Append lines with the given kind and an explicit per-line render style.
    pub fn push_transcript_styled(&mut self, text: &str, kind: TranscriptKind, style: ratatui::style::Style) {
        self.transcript_styles.resize(self.transcript.len(), None); // self-heal alignment
        for line in text.split('\n') {
            self.transcript.push(line.to_owned());
            self.transcript_kinds.push(kind);
            self.transcript_styles.push(Some(style));
        }
    }
```

In `crates/app/src/render/transcript.rs`, the `filtered_styles` map (~line 828) currently computes a style per visible line from its kind. Wrap it so an override wins. Replace:
```rust
    let filtered_styles: Vec<Style> = visible_indices
        .iter()
        .zip(filtered_kinds.iter())
        .map(|(&i, kind)| match kind {
            TranscriptKind::Story   => state.colors.resolve_story_style(&state.transcript[i], room_name),
            TranscriptKind::Input   => state.colors.transcript_input,
            TranscriptKind::Meta    => state.colors.transcript_meta,
            TranscriptKind::Warning => state.colors.transcript_warning,
        })
        .collect();
```
with:
```rust
    let filtered_styles: Vec<Style> = visible_indices
        .iter()
        .zip(filtered_kinds.iter())
        .map(|(&i, kind)| {
            if let Some(ov) = state.transcript_styles.get(i).copied().flatten() {
                return ov;
            }
            match kind {
                TranscriptKind::Story   => state.colors.resolve_story_style(&state.transcript[i], room_name),
                TranscriptKind::Input   => state.colors.transcript_input,
                TranscriptKind::Meta    => state.colors.transcript_meta,
                TranscriptKind::Warning => state.colors.transcript_warning,
            }
        })
        .collect();
```

- [ ] **Step 4: Run, verify pass.** Run the Step 2 command — expected PASS.

- [ ] **Step 5: Full suite + commit.**

```
cargo test -p app && cargo build -p app
git add crates/app/src/state.rs crates/app/src/render/transcript.rs
git commit -m "feat(app): per-line transcript style override (self-healing, in-memory)"
```

---

### Task B2: `describe_scheme` formatter

**Files:** Modify `crates/app/src/style.rs` (new `describe_scheme`). Test in `crates/app/src/style.rs` `mod tests`.

**Interfaces:** Produces `pub fn describe_scheme(cs: &colors::ColorScheme) -> Vec<(String, Option<ratatui::style::Style>)>`. Consumes `SELECTOR_GROUPS`, `style_for_selector`, `color_to_str`.

- [ ] **Step 1: Write the failing test** in `crates/app/src/style.rs` `mod tests`:

```rust
#[test]
fn describe_scheme_lists_selectors_with_styles() {
    let cs = colors::ColorScheme::terminal_default();
    let lines = describe_scheme(&cs);
    let texts: Vec<&str> = lines.iter().map(|(t, _)| t.as_str()).collect();
    assert!(texts.iter().any(|t| t.contains("Map")), "group title present");
    assert!(texts.iter().any(|t| t.contains("room:") && t.contains("fg=white") && t.contains("bg=reset")),
        "room line shows fg=white bg=reset");
    assert!(texts.iter().any(|t| t.contains("connector:") && t.contains("fg=cyan")),
        "connector line shows fg=cyan");
    assert!(texts.iter().any(|t| t.contains("map_layer_tab_active:") && t.contains("bold")),
        "an attribute is listed");
    // A selector line carries Some(style) equal to style_for_selector.
    let conn = lines.iter().find(|(t, _)| t.contains("connector:") && !t.contains("distorted") && !t.contains("portal")).unwrap();
    assert_eq!(conn.1, Some(style_for_selector(&cs, "connector")), "selector line carries its style");
    // A header line carries None.
    let hdr = lines.iter().find(|(t, _)| t.contains("Map") && !t.contains(":")).unwrap();
    assert_eq!(hdr.1, None, "group header has no style");
}
```

- [ ] **Step 2: Run, verify fail.** Run: `cargo test -p app --lib describe_scheme_lists_selectors_with_styles` — expected FAIL (function missing).

- [ ] **Step 3: Implement** in `crates/app/src/style.rs` (place near `style_for_selector`):

```rust
/// Describe the resolved scheme as printable lines: a header per SELECTOR_GROUPS
/// group (style `None`), then one line per selector
/// `  <selector>: fg=<fg> bg=<bg><attrs>` carrying that selector's resolved Style.
/// `border` (no color field) is skipped.
pub fn describe_scheme(cs: &colors::ColorScheme) -> Vec<(String, Option<Style>)> {
    let mut out: Vec<(String, Option<Style>)> = Vec::new();
    for (title, selectors) in SELECTOR_GROUPS {
        out.push((format!("── {title} ──"), None));
        for sel in *selectors {
            if *sel == "border" { continue; }
            let st = style_for_selector(cs, sel);
            let fg = st.fg.map(color_to_str).unwrap_or_else(|| "default".to_string());
            let bg = st.bg.map(color_to_str).unwrap_or_else(|| "default".to_string());
            let mut attrs: Vec<&str> = Vec::new();
            if st.add_modifier.contains(Modifier::BOLD) { attrs.push("bold"); }
            if st.add_modifier.contains(Modifier::ITALIC) { attrs.push("italic"); }
            if st.add_modifier.contains(Modifier::UNDERLINED) { attrs.push("underline"); }
            if st.add_modifier.contains(Modifier::DIM) { attrs.push("dim"); }
            if st.add_modifier.contains(Modifier::REVERSED) { attrs.push("reversed"); }
            let attr_str = if attrs.is_empty() { String::new() } else { format!(" {}", attrs.join(",")) };
            out.push((format!("  {sel}: fg={fg} bg={bg}{attr_str}"), Some(st)));
        }
    }
    out
}
```

(Ensure `Modifier` and `Style` are in scope in `style.rs` — they are already used by `decl_to_style`/`style_to_decl`.)

- [ ] **Step 4: Run, verify pass.** Run the Step 2 command — expected PASS.

- [ ] **Step 5: Full suite + commit.**

```
cargo test -p app && cargo build -p app
git add crates/app/src/style.rs
git commit -m "feat(app): describe_scheme formats the resolved color scheme"
```

---

### Task B3: `/print-colors [color]` command + wiring

**Files:** Modify `crates/app/src/slash.rs` (new `CommandSpec` + `SlashOutcome::PrintColors { actual: bool }` + count assertion/comment + tests), `crates/app/src/main.rs` (`dispatch_slash_outcome` handling). Tests in `crates/app/src/slash.rs` and a render test in `crates/app/src/render/transcript.rs`.

**Interfaces:** Consumes `describe_scheme` (B2), `push_transcript_styled`/`push_transcript_kind` (B1), live `state.colors`.

- [ ] **Step 1: Write the failing tests.**

In `crates/app/src/slash.rs` `mod tests`: update the count assertion `assert_eq!(COMMANDS.len(), 47, ...)` to `48` and the descriptive comment (Style 4 → 5, 47 → 48). Add:

```rust
#[test]
fn print_colors_command_parses_flag() {
    assert!(find_command("print-colors").is_some());
    assert!(matches!(parse("print-colors", '/'), SlashOutcome::PrintColors { actual: false }));
    assert!(matches!(parse("print-colors color", '/'), SlashOutcome::PrintColors { actual: true }));
}
```

In `crates/app/src/render/transcript.rs` `mod tests`, a render test (mirror the existing transcript render tests' harness):

```rust
#[test]
fn transcript_color_override_paints_line_in_its_style() {
    use ratatui::style::{Color, Style};
    let machine = /* build/get a Machine fixture as the neighboring tests do */;
    let mut state = AppState::default();
    state.push_transcript_styled("connector sample", TranscriptKind::Meta, Style::new().fg(Color::Cyan));
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    let _ = render_transcript(&machine, &state, area, &mut buf);
    // Find a cell of the line and assert its fg is Cyan (the override), not transcript_meta.
    let found = (0..area.height).any(|y| (0..area.width).any(|x| {
        let c = &buf[(x, y)];
        c.symbol().starts_with('c') && c.style().fg == Some(Color::Cyan)
    }));
    assert!(found, "color override paints the line in its style");
}
```

(Use the same `Machine`/state construction the other `render_transcript_*` tests in this file use; if they build a fixture machine via a helper, reuse it. The essential assertion is that a cell of the styled line carries `fg == Some(Color::Cyan)`.)

- [ ] **Step 2: Run, verify fail.** Run: `cargo test -p app --lib print_colors_command_parses_flag transcript_color_override_paints_line_in_its_style` — expected FAIL (variant/command missing; the count test fails until updated).

- [ ] **Step 3: Implement.**

In `crates/app/src/slash.rs`, add the variant to `enum SlashOutcome`:
```rust
    /// Print the resolved color scheme to the transcript. `actual` = render each
    /// selector line in its own style instead of the plain meta color.
    PrintColors { actual: bool },
```
Add the command in the `// ── Style ──` block of `COMMANDS`:
```rust
    CommandSpec { name: "print-colors", category: Category::Style, context: Context::Global,
        usage: "print-colors [color]", description: "print the current color scheme (color = actual colors)",
        dispatch: |a| SlashOutcome::PrintColors { actual: a.first() == Some(&"color") } },
```

In `crates/app/src/main.rs` `dispatch_slash_outcome`, add an arm next to `SlashOutcome::Help` (~line 2811):
```rust
        SlashOutcome::PrintColors { actual } => {
            for (line, style_opt) in app::style::describe_scheme(&state.colors) {
                match (actual, style_opt) {
                    (true, Some(style)) => state.push_transcript_styled(&line, crate::state::TranscriptKind::Meta, style),
                    _ => state.push_transcript_kind(&line, crate::state::TranscriptKind::Meta),
                }
            }
        }
```
(Match the exact `TranscriptKind` path/style used by the neighboring `SlashOutcome::Help` arm — if it refers to `TranscriptKind::Meta` via a different path, use that.)

- [ ] **Step 4: Run, verify pass.** Run the Step 2 command — expected PASS. Then full suite.

- [ ] **Step 5: Full suite + commit.**

```
cargo test -p app && cargo build -p app
git add crates/app/src/slash.rs crates/app/src/main.rs
git commit -m "feat(app): print-colors command prints the live scheme, with a color mode"
```

---

## Notes for the executor

- Order: A1, A2, B1, B2, B3. A and B are independent; B3 needs B1 and B2.
- If the A1 freeze test passes before the alias change (because it uses the `reset` token, which already parses), keep it — it is a real end-to-end guard. The parse test is the one that proves A1's behavior change.
- For the render test in B3, reuse the exact `Machine`/fixture construction the other `render_transcript_*` tests in `render/transcript.rs` use; do not invent a new fixture.
- After all five tasks: `cargo doc -p app --no-deps` should still be 0 warnings (it was just cleaned); if any new public item triggers a doc-link warning, fix the doc comment.
