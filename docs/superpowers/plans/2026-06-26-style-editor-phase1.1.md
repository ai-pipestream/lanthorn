# Live Style Editor — Phase 1.1 Polish — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Close the Phase 1 whole-branch-review follow-ups: let custom-hex/MRU set background (not just foreground), add keyboard access to the swatch grid, scroll the board so off-screen selectors are reachable, add a `/style` alias, make the focused attribute-chip cursor visually distinct, and tighten color validation/persistence.

**Architecture:** Small, surgical changes to the existing style editor (`state.rs` `StyleEditorState`, `input.rs` handlers/keys, `render/style_editor.rs`, `main.rs` mouse block, `slash.rs`, `style_mru.rs`). No new modes.

**Tech Stack:** Rust, ratatui, the existing style editor (Phase 1, merged at `4377488`).

## Global Constraints

- Commit trailers on EVERY commit body (no backticks anywhere in bodies — zsh):
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
- Per task: full `cargo test -p app` (lib + bin + headless) green and `cargo build -p app` **0 warnings** before committing. Run the FULL suite.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md`.
- Editor chrome stays themeable (no hard-coded colors) except swatch cells (which are intentionally their literal palette color).

### Verified current state (from the codebase)

- `StyleEditorState` (`state.rs:513`): `{ doc, preview, selectors, active, focus: StyleFocus, custom_buf: String, mru: Vec<String>, attr_cursor: usize }`. `enum StyleFocus { Board, Fg, Bg, Custom, Attrs }`. `enum AttrKind { Bold, Italic, Underline, Dim, Reversed }` (`input.rs:42`).
- `style_editor_key_to_action` (`input.rs:1065`): Custom-focus early block (`1072–1091`) — printable→`StyleCustomChar`, Backspace→`StyleCustomBackspace`, **Enter commits `StyleSetColor{is_bg:false}` (hardcoded, line 1084)**. Main block (`1093–1115`): Up/Down→`StyleNav`, Tab/BackTab→`StyleFocusCycle`, Left/Right(Attrs)→`StyleAttrChipNav`, Space(Attrs)→`StyleToggleAttr`, s→`StyleSave`, r→`StyleReset`, Esc→`StyleEditorCancel`.
- Actions (`input.rs:207–228`): `StyleSetColor{is_bg:bool, value:Option<String>}`, `StyleFocusCycle(i32)`, `StyleNav(i32)`, `StyleAttrChipNav(i32)`, `StyleToggleAttr(AttrKind)`, `StyleCustomChar(char)`, `StyleCustomBackspace`, `StyleSave`, `StyleReset`, `StyleEditorCancel`, `OpenStyleEditor`. Handlers: `StyleSetColor` (`2235`), `StyleFocusCycle` (`2212`), `StyleSave` (`2263`).
- `open_style_editor` (`input.rs:2629`) seeds the state.
- `StyleEditorRects` (`render/style_editor.rs:33`): `{ samples, attr_chips, dialog, fg_swatches, bg_swatches, mru_rects, custom_rect }`.
- Board renderer (`render/style_editor.rs:116`): walks `SELECTOR_GROUPS`, draws header + rows, **breaks at `board_area.bottom()` with NO scroll offset**. `draw_swatch_row` (`332`) draws 17 cells (16 `ANSI_NAMES` + default), pushes 17 rects. Chip loop (`280–317`): `flag_on` highlight takes precedence over the cursor (line ~299).
- Mouse block (`main.rs:1780–1868`): samples→active, chips→`StyleToggleAttr`, fg_swatches→`StyleSetColor{is_bg:false}`, bg_swatches→`StyleSetColor{is_bg:true}`, **mru_rects→`StyleSetColor{is_bg:false}` (hardcoded, line 1844)**, custom_rect→set focus Custom, dialog→`style_dialog_action`.
- Slash registry (`slash.rs`): `CURATED` table (`63–236`) of `{ name, help, build }`; `parse` (`253`) checks CURATED first, then kebab `Command::from_name`. Aliases exist (e.g. `"q"`→quit). No `/style` entry yet.
- `style_mru::is_valid_color_token` (`style_mru.rs:15`) accepts bare 6-hex without `#`; `save_mru` (`35`) hand-rolls TOML.

---

### Task 1: Foreground/background color target + keyboard swatch navigation

**Files:**
- Modify: `crates/app/src/state.rs` — add `color_target: bool` + `swatch_cursor: usize` to `StyleEditorState` (+ init in `open_style_editor`).
- Modify: `crates/app/src/input.rs` — `color_target` follows Fg/Bg focus; custom-commit + a new keyboard-swatch path use it; `Action::StyleCommitCustom`.
- Modify: `crates/app/src/main.rs` — fg/bg swatch clicks set `color_target`; MRU click uses `color_target`.
- Modify: `crates/app/src/render/style_editor.rs` — highlight `swatch_cursor` when focus is Fg/Bg.

**Interfaces:**
- Produces: `StyleEditorState.color_target: bool` (false=fg, true=bg), `StyleEditorState.swatch_cursor: usize` (0..=16; 16 = default cell), `Action::StyleCommitCustom`, `Action::StyleSwatchNav(i32)`, `Action::StyleSwatchPick`.

- [ ] **Step 1: Write the failing tests** (in `input.rs` tests):

```rust
#[test]
fn custom_commit_targets_bg_when_color_target_is_bg() {
    let mut s = AppState::default();
    open_style_editor(&mut s);
    {
        let ed = s.style_editor.as_mut().unwrap();
        ed.color_target = true; // bg
        ed.custom_buf = "#abcdef".into();
    }
    apply_action(Action::StyleCommitCustom, &mut s, &mut Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    let sel = ed.selectors[ed.active].to_string();
    assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.bg.clone()), Some("#abcdef".into()));
    assert!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.clone()).is_none());
}

#[test]
fn swatch_pick_sets_color_for_target_and_default_clears() {
    let mut s = AppState::default();
    open_style_editor(&mut s);
    { let ed = s.style_editor.as_mut().unwrap(); ed.color_target = false; ed.swatch_cursor = 16; } // default cell
    apply_action(Action::StyleSwatchPick, &mut s, &mut Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    let sel = ed.selectors[ed.active].to_string();
    // default cell clears fg
    assert!(ed.doc.colors.selectors.get(&sel).map_or(true, |d| d.fg.is_none()));
}
```

- [ ] **Step 2: Run them** → compile error (fields/actions missing).

- [ ] **Step 3: Add state fields + init**

In `state.rs` `StyleEditorState`: add `pub color_target: bool,` and `pub swatch_cursor: usize,`. In `open_style_editor` (`input.rs:2629`) init `color_target: false, swatch_cursor: 0,`.

- [ ] **Step 4: color_target follows focus; commit/pick use it**

- In the `StyleFocusCycle` handler (`input.rs:2212`), after computing the new focus, set `color_target` when it lands on a color row: `match ed.focus { StyleFocus::Fg => ed.color_target = false, StyleFocus::Bg => ed.color_target = true, _ => {} }`.
- Replace the Custom-focus Enter-commit (`input.rs:1084`) to return `Action::StyleCommitCustom` (no hardcoded is_bg). Add the handler:

```rust
Action::StyleCommitCustom => {
    if let Some(ed) = &mut state.style_editor {
        if crate::style_mru::is_valid_color_token(&ed.custom_buf) {
            let is_bg = ed.color_target;
            let value = if ed.custom_buf == "default" { None } else { Some(ed.custom_buf.clone()) };
            // reuse the same set+mru+recompute path:
            let dir = state.config.user_dir.clone();
            apply_style_set_color(state, is_bg, value, &dir); // factor StyleSetColor body into a helper, or inline the same steps
            if let Some(ed) = &mut state.style_editor { ed.custom_buf.clear(); }
        }
    }
}
```

(Factor the existing `StyleSetColor` handler body into a small `fn apply_style_set_color(state, is_bg, value, user_dir)` and call it from BOTH `StyleSetColor` and `StyleCommitCustom` — DRY. The MRU push (hex→`push_mru`) and `recompute_style_preview` live in that helper.)

- Add `Action::StyleSwatchNav(i32)` + `Action::StyleSwatchPick` handlers:

```rust
Action::StyleSwatchNav(d) => {
    if let Some(ed) = &mut state.style_editor {
        let n = crate::style_mru::ANSI_NAMES.len() as i32 + 1; // +1 for default cell
        ed.swatch_cursor = ((ed.swatch_cursor as i32 + d).rem_euclid(n)) as usize;
    }
}
Action::StyleSwatchPick => {
    if let Some(ed) = &state.style_editor {
        let is_bg = ed.color_target;
        let cur = ed.swatch_cursor;
        let value = crate::style_mru::ANSI_NAMES.get(cur).map(|s| s.to_string()); // None at index 16 = default
        let dir = state.config.user_dir.clone();
        apply_style_set_color(state, is_bg, value, &dir);
    }
}
```

- [ ] **Step 5: Keys for swatch nav in Fg/Bg focus**

In `style_editor_key_to_action` main block, when `focus == StyleFocus::Fg || focus == StyleFocus::Bg`: `Left`→`StyleSwatchNav(-1)`, `Right`→`StyleSwatchNav(1)`, `Enter`/`Char(' ')`→`StyleSwatchPick`. (Keep Attrs Left/Right→`StyleAttrChipNav` as-is; gate by focus.)

- [ ] **Step 6: Mouse + render**

- In `main.rs` MRU click (`~1844`), change `is_bg:false` to read the live target: dispatch `StyleSetColor{ is_bg: state.style_editor.as_ref().map_or(false,|e| e.color_target), value: Some(hex) }`. In the fg-swatch click set `color_target=false`, bg-swatch click set `color_target=true` (before/with dispatching the existing `StyleSetColor`).
- In `render/style_editor.rs` `draw_swatch_row`, highlight the cell at `swatch_cursor` when the row's slot matches focus (fg row highlighted only when `focus==Fg`, bg row only when `focus==Bg`) — pass `focus`/`swatch_cursor` into the row draw.

- [ ] **Step 7: Run full suite + 0 warnings; Commit** (`feat(app): style editor — fg/bg color target + keyboard swatch nav`).

---

### Task 2: Board scroll / keep-active-visible

**Files:**
- Modify: `crates/app/src/render/style_editor.rs` — window the group/selector list so the active selector is always visible; hit-rects account for the offset.

**Interfaces:** Consumes `ed.active`, `SELECTOR_GROUPS`, the board area height. No new state (stateless auto-follow).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn board_scrolls_to_keep_active_visible() {
    let mut s = AppState::default();
    crate::input::open_style_editor(&mut s);
    let n = s.style_editor.as_ref().unwrap().selectors.len();
    s.style_editor.as_mut().unwrap().active = n - 1; // last selector
    // Small area that cannot show all selectors at once:
    let area = Rect::new(0, 0, 90, 18);
    let mut buf = Buffer::empty(area);
    let rects = crate::render::style_editor::draw_style_editor(&s, area, &mut buf).expect("drawn");
    assert!(rects.samples.iter().any(|(i, _)| *i == n - 1),
        "the active (last) selector must be rendered with a hit-rect even on a short board");
}
```

- [ ] **Step 2: Run it** → FAIL (last selector below the fold, no rect).

- [ ] **Step 3: Implement auto-follow scroll**

In the board renderer: build the ordered list of visual lines (group headers + selector rows) with their selector indices; compute the visual line index of `ed.active`; compute a `scroll` (first visible line) so the active line is within `[scroll, scroll + visible_rows)` (clamp to `[0, total_lines - visible_rows]`); render only lines `scroll..scroll+visible_rows`; record sample hit-rects only for rendered selector rows (with the scrolled y). Keep group headers attached (a header for a group whose first visible row is mid-group may be omitted or repeated — simplest: skip headers that scroll off; do NOT record off-screen rows).

- [ ] **Step 4: Run the test + full suite** → PASS, 0 warnings.

- [ ] **Step 5: Commit** (`feat(app): style editor — scroll board to keep active selector visible`).

---

### Task 3: `/style` alias, distinct chip cursor, stricter validation

**Files:**
- Modify: `crates/app/src/slash.rs` — add a `/style` curated alias for `OpenStyleEditor`.
- Modify: `crates/app/src/render/style_editor.rs` — distinct focused-chip cursor style.
- Modify: `crates/app/src/style_mru.rs` — `is_valid_color_token` requires `#`; harden `save_mru`.

**Interfaces:** none new (behavior fixes).

- [ ] **Step 1: Write the failing tests**

```rust
// in slash.rs tests:
#[test]
fn slash_style_opens_editor() {
    assert!(matches!(parse("style", '/'), Ok(Action::OpenStyleEditor)));
}

// in style_mru.rs tests:
#[test]
fn is_valid_requires_hash_for_hex() {
    assert!(is_valid_color_token("#a1b2c3"));
    assert!(!is_valid_color_token("a1b2c3"), "bare hex without # is rejected");
    assert!(is_valid_color_token("yellow"));
    assert!(is_valid_color_token("default"));
}
```

(Confirm the exact `parse` return type/`Action` path used by other curated entries — mirror an existing entry like the quit/help alias; the assertion shape may differ, e.g. it may return a slash-command enum rather than `Action` directly. Match the real signature.)

- [ ] **Step 2: Run them** → FAIL.

- [ ] **Step 3: Implement**

- `slash.rs`: add a `CURATED` entry `{ name: "style", help: "open the live style editor", build: /* mirror an existing zero-arg entry that yields Action::OpenStyleEditor */ }`. Use the same `build` closure shape the other curated entries use.
- `style_mru.rs` `is_valid_color_token`: require the `#` prefix —
  ```rust
  pub fn is_valid_color_token(s: &str) -> bool {
      if s == "default" || ANSI_NAMES.contains(&s) { return true; }
      match s.strip_prefix('#') {
          Some(hex) => hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
          None => false,
      }
  }
  ```
- `style_mru.rs` `save_mru`: harden by only writing tokens that pass `is_valid_color_token` (drop anything else) and keep the quoting (valid tokens never contain `"`/`\`), so a corrupt sidecar can't round-trip invalid TOML:
  ```rust
  let arr = v.iter().filter(|s| is_valid_color_token(s))
      .map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", ");
  ```
- `render/style_editor.rs` chip loop: when `prop_focused && is_chip_cursor`, render the cursor chip with a DISTINCT marker independent of `flag_on` — e.g. add `Modifier::UNDERLINED` (or wrap the label in `>` `<`) so the cursor is visible even on an off (flag=false) chip. Keep `flag_on` driving the on/off background via `active_style`.

- [ ] **Step 4: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 5: Commit** (`feat(app): style editor — /style alias, distinct chip cursor, stricter hex validation`).

---

## Notes for the executor

- **Dependency order:** 1 → 2 → 3 (independent enough, but keep order for clean review). All `cargo test -p app`, full suite, 0 warnings before each commit.
- **DRY the color-set path:** factor the `StyleSetColor` handler body into `apply_style_set_color(state, is_bg, value, user_dir)` in Task 1 and reuse it for `StyleCommitCustom` and `StyleSwatchPick` — do not duplicate the push-MRU + recompute logic three times.
- **Line numbers** are from a snapshot; confirm by grep before editing.
- **Keep keyboard + mouse parity:** after Task 1, both the mouse (swatch/MRU/custom clicks) and the keyboard (Fg/Bg focus + swatch nav + custom entry) can set BOTH fg and bg.
- `TODO.md` is gitignored — never stage it. No README change required.
