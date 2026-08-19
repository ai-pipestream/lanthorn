# Style Editor Phase 2.2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the live style editor open over the active (global + per-game) style, save explicitly to either the global or the current game's style file, and fix three property-pane usability gaps (fg/bg target clarity, an obvious hex edit box, a stable color MRU).

**Architecture:** Six independent TDD tasks against the merged Phase 1/2/2.1 editor. Theme A (Tasks 2–4) reuses the existing `style::merge`, `style::write_style_full`, and `styles::per_game_style_path` plumbing — the editor doc becomes the merged active look on open, and a second run-loop save flag writes a self-contained per-game file. Theme B (Tasks 1, 5, 6) are localized changes to `style_mru.rs` and the property-pane renderer.

**Tech Stack:** Rust workspace; `app` crate (ratatui TUI, binary `lanthorn`). Tests via `cargo test -p app`. All style-editor tests use the Phase 2.1 hermetic helper `crate::input::open_style_editor_hermetic`.

## Global Constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main` (no push without explicit instruction); TDD wave.
- Commit trailers, every commit:
  - `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  - `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- No backticks in commit message bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- Every styleable element stays themeable; reuse existing selectors (no hard-coded styles, no new style selector).
- Per-game save writes the FULL self-contained look (not a diff); it does NOT repoint `config.style`.
- Style-editor tests must build the editor via `open_style_editor_hermetic` UNLESS the test deliberately writes style files on disk first (Tasks 2 and 3 build their own temp `user_dir`).

---

### Task 1: Stable color MRU

Replace the move-to-front `push_mru` with stable-position semantics: a re-used color keeps its slot, a new color appends to the end, and when full the oldest (front) entry is evicted. Fixes the two-entry cycling that makes swatch click targets unstable.

**Files:**
- Modify: `crates/app/src/style_mru.rs:59-63` (`push_mru`)
- Test: `crates/app/src/style_mru.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `pub fn push_mru(v: &mut Vec<String>, value: &str)` — unchanged signature, new behavior. `CAP` is the module const (16).

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block in `crates/app/src/style_mru.rs`:

```rust
#[test]
fn push_mru_keeps_position_of_existing() {
    let mut v = vec!["#aaaaaa".to_string(), "#bbbbbb".to_string()];
    push_mru(&mut v, "#aaaaaa"); // re-use the first entry
    assert_eq!(v, vec!["#aaaaaa".to_string(), "#bbbbbb".to_string()],
        "re-using an existing color must not reorder the list");
}

#[test]
fn push_mru_appends_new_to_end() {
    let mut v = vec!["#aaaaaa".to_string()];
    push_mru(&mut v, "#bbbbbb");
    assert_eq!(v, vec!["#aaaaaa".to_string(), "#bbbbbb".to_string()],
        "a new color must be appended at the end, not inserted at the front");
}

#[test]
fn push_mru_evicts_oldest_when_full() {
    let mut v: Vec<String> = (0..CAP).map(|i| format!("#0000{:02x}", i)).collect();
    let oldest = v[0].clone();
    let newest = "#ffffff".to_string();
    push_mru(&mut v, &newest);
    assert_eq!(v.len(), CAP, "length stays capped");
    assert!(!v.contains(&oldest), "the oldest (front) entry is evicted when full");
    assert_eq!(v.last().unwrap(), &newest, "the new color lands at the end");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app --lib push_mru_`
Expected: `push_mru_appends_new_to_end` and `push_mru_keeps_position_of_existing` FAIL (current code inserts at front / reorders).

- [ ] **Step 3: Write the implementation**

Replace `push_mru` (`crates/app/src/style_mru.rs:59-63`) with:

```rust
pub fn push_mru(v: &mut Vec<String>, value: &str) {
    if v.iter().any(|x| x == value) {
        return; // already present: keep its position stable
    }
    if v.len() >= CAP {
        v.remove(0); // full: evict the oldest (front) entry
    }
    v.push(value.to_string()); // new color goes to the end
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app --lib push_mru_`
Expected: PASS (3 tests).

- [ ] **Step 5: Full suite + commit**

Run: `cargo test -p app` (expect green) and `cargo build -p app` (expect 0 warnings).

```bash
git add crates/app/src/style_mru.rs
git commit -m "feat(app): stable color MRU (append + keep position + ring-evict)"
```

---

### Task 2: Open editor over the active (merged) style

`open_style_editor` currently loads the global style only. Make it load `merge(global, per-game)` when the current game has a per-game override, so the editor shows the live look.

**Files:**
- Modify: `crates/app/src/input.rs:3050-3053` (`open_style_editor`, the doc-load line)
- Test: `crates/app/src/input.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::style::load_style(Option<&str>, &Path) -> (StyleDoc, Vec<String>)`; `crate::styles::per_game_style_path(&Path, &str) -> PathBuf`; `crate::style::parse_style_toml(&str) -> Result<StyleDoc, String>`; `crate::style::merge(&StyleDoc, &StyleDoc) -> StyleDoc`. `state.ifid: String`.
- Produces: `open_style_editor` now seeds `ed.doc` from the merged active style.

- [ ] **Step 1: Write the failing test**

This test deliberately writes files on disk, so it builds its own temp `user_dir` instead of using the hermetic helper. Add to `mod tests` in `crates/app/src/input.rs`:

```rust
#[test]
fn editor_opens_over_merged_per_game_style() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join(format!("bm-merge-open-{}-{}", std::process::id(), n));
    let styles_dir = dir.join("styles");
    std::fs::create_dir_all(&styles_dir).unwrap();
    // Global: room fg = white, connector fg = cyan.
    std::fs::write(
        dir.join("style.toml"),
        "[colors]\n\"room\" = { fg = \"white\" }\n\"connector\" = { fg = \"cyan\" }\n[symbols]\n",
    ).unwrap();
    // Per-game override for IFID: room fg = red (connector untouched).
    let ifid = "ZCODE-1-ABCDEF-0001";
    std::fs::write(
        styles_dir.join(format!("{ifid}.toml")),
        "[colors]\n\"room\" = { fg = \"red\" }\n[symbols]\n",
    ).unwrap();

    let mut s = AppState::default();
    s.config.user_dir = dir;
    s.config.style = None; // load global from user_dir/style.toml
    s.ifid = ifid.to_string();
    open_style_editor(&mut s);

    let ed = s.style_editor.as_ref().unwrap();
    assert_eq!(
        ed.doc.colors.selectors.get("room").and_then(|d| d.fg.as_deref()),
        Some("red"),
        "per-game override wins for room",
    );
    assert_eq!(
        ed.doc.colors.selectors.get("connector").and_then(|d| d.fg.as_deref()),
        Some("cyan"),
        "global value survives for non-overridden connector",
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib editor_opens_over_merged_per_game_style`
Expected: FAIL — `room` fg is `white` (global only), because `open_style_editor` ignores the per-game file.

- [ ] **Step 3: Write the implementation**

In `crates/app/src/input.rs`, replace the doc-load line in `open_style_editor` (currently line 3052):

```rust
    let (doc, _warnings) = crate::style::load_style(state.config.style.as_deref(), &user_dir);
```

with the merged load (mirrors `reload.rs`):

```rust
    let (global, _warnings) = crate::style::load_style(state.config.style.as_deref(), &user_dir);
    // Layer the per-game override (user_dir/styles/<ifid>.toml) over the global so
    // the editor opens showing the live look. A missing or unparseable per-game
    // file falls back to the global doc.
    let doc = if !state.ifid.is_empty() {
        let pg_path = crate::styles::per_game_style_path(&user_dir, &state.ifid);
        match std::fs::read_to_string(&pg_path) {
            Ok(text) => match crate::style::parse_style_toml(&text) {
                Ok(over) => crate::style::merge(&global, &over),
                Err(_) => global,
            },
            Err(_) => global,
        }
    } else {
        global
    };
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib editor_opens_over_merged_per_game_style`
Expected: PASS.

- [ ] **Step 5: Full suite + commit**

Run: `cargo test -p app` (green) and `cargo build -p app` (0 warnings).

```bash
git add crates/app/src/input.rs
git commit -m "feat(app): style editor opens over the active merged per-game style"
```

---

### Task 3: Save Game Style — per-game write helper, action, run-loop wiring

Add the ability to save the editor's look to the current game's per-game file. A testable writer lives in `styles.rs`; a new `Action::StyleSaveGame` applies the look live and closes the editor (guarding when no game is loaded); the run loop performs the disk write.

**Files:**
- Modify: `crates/app/src/styles.rs` (add `save_per_game_style`)
- Modify: `crates/app/src/input.rs:232` (add `Action::StyleSaveGame` variant) and `:2399` neighborhood (add the handler)
- Modify: `crates/app/src/main.rs:2097` (add `style_save_game` flag) and `:2716-2721` (add the per-game write)
- Test: `crates/app/src/styles.rs` (`mod tests`) and `crates/app/src/input.rs` (`mod tests`)

**Interfaces:**
- Consumes: `crate::style::write_style_full(&Path, &ColorScheme, &SymbolSet) -> std::io::Result<()>`; `per_game_style_path`; `state.ifid`, `state.colors`, `state.symbols`.
- Produces:
  - `crate::styles::save_per_game_style(user_dir: &Path, ifid: &str, colors: &crate::colors::ColorScheme, symbols: &crate::symbols::SymbolSet) -> std::io::Result<PathBuf>`
  - `crate::input::Action::StyleSaveGame`
  - run-loop `style_save_game` flag that calls the writer.

- [ ] **Step 1: Write the failing test (writer)**

Add to the `mod tests` block in `crates/app/src/styles.rs`:

```rust
#[test]
fn save_per_game_writes_self_contained_and_roundtrips() {
    let dir = tmp("save_pg");
    let ifid = "ZCODE-1-ABCDEF-0001";
    let colors = crate::colors::ColorScheme::terminal_default();
    let symbols = crate::symbols::SymbolSet::default();
    let path = save_per_game_style(&dir, ifid, &colors, &symbols).unwrap();
    assert_eq!(path, per_game_style_path(&dir, ifid));
    assert!(path.is_file(), "per-game style file is written");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("[colors]"), "file is a self-contained style doc");
    // Self-contained: parses back without error.
    crate::style::parse_style_toml(&text).unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib save_per_game_writes_self_contained_and_roundtrips`
Expected: FAIL — `save_per_game_style` does not exist (compile error).

- [ ] **Step 3: Write the writer**

Add to `crates/app/src/styles.rs` (after `per_game_style_path`):

```rust
/// Write the live look self-contained to the current game's per-game style file
/// (`user_dir/styles/<ifid>.toml`), creating `styles/` if needed. Returns the path
/// written. Does NOT repoint `config.style`; the file is merged over the global
/// style on the next reload.
pub fn save_per_game_style(
    user_dir: &Path,
    ifid: &str,
    colors: &crate::colors::ColorScheme,
    symbols: &crate::symbols::SymbolSet,
) -> std::io::Result<PathBuf> {
    let path = per_game_style_path(user_dir, ifid);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::style::write_style_full(&path, colors, symbols)?;
    Ok(path)
}
```

- [ ] **Step 4: Run writer test to verify it passes**

Run: `cargo test -p app --lib save_per_game_writes_self_contained_and_roundtrips`
Expected: PASS.

- [ ] **Step 5: Write the failing test (action guard + close)**

Add to `mod tests` in `crates/app/src/input.rs`:

```rust
#[test]
fn style_save_game_guards_no_game_and_closes_with_game() {
    // No game loaded: editor stays open (cannot save a per-game style).
    let mut s = AppState::default();
    open_style_editor_hermetic(&mut s); // ifid is empty by default
    assert!(s.ifid.is_empty());
    apply_action(Action::StyleSaveGame, &mut s, &mut mapper::mapper::Mapper::default());
    assert!(s.style_editor.is_some(), "no game: Save Game Style is a no-op, editor stays open");

    // Game loaded: applies the look live and closes the editor.
    let mut s2 = AppState::default();
    open_style_editor_hermetic(&mut s2);
    s2.ifid = "ZCODE-1-ABCDEF-0001".to_string();
    apply_action(Action::StyleSaveGame, &mut s2, &mut mapper::mapper::Mapper::default());
    assert!(s2.style_editor.is_none(), "with game: Save Game Style closes the editor");
}
```

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo test -p app --lib style_save_game_guards_no_game_and_closes_with_game`
Expected: FAIL — `Action::StyleSaveGame` does not exist (compile error).

- [ ] **Step 7: Add the action variant + handler**

In `crates/app/src/input.rs`, add the variant after `StyleSave` (line 232):

```rust
    /// Save the style editor to the current game's per-game style file and close.
    StyleSaveGame,
```

Add the handler immediately after the `Action::StyleSave => { ... }` arm (ends near line 2407):

```rust
        Action::StyleSaveGame => {
            if state.ifid.is_empty() {
                state.set_status("no game loaded");
            } else if let Some(ed) = state.style_editor.take() {
                let dir = state.config.user_dir.clone();
                let _ = crate::style_mru::save_mru(&dir, &ed.mru);
                let (cs, set, _w) = crate::style::resolve(&ed.doc, &dir);
                state.colors = cs;
                state.symbols = set;
            }
        }
```

- [ ] **Step 8: Wire the run-loop disk write**

In `crates/app/src/main.rs`, after the `style_save` flag (line 2097) add:

```rust
        let style_save_game = matches!(action, Action::StyleSaveGame);
```

After the `if style_save { ... }` block (ends ~line 2721) add:

```rust
        // After apply_action: if Save Game Style was used, write the live look
        // self-contained to the current game's per-game style file.
        if style_save_game {
            let user_dir = state.config.user_dir.clone();
            if !state.ifid.is_empty() {
                let _ = app::styles::save_per_game_style(
                    &user_dir, &state.ifid, &state.colors, &state.symbols,
                );
            }
        }
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p app --lib style_save_game_guards_no_game_and_closes_with_game save_per_game_writes_self_contained_and_roundtrips`
Expected: PASS (2 tests).

- [ ] **Step 10: Full suite + commit**

Run: `cargo test -p app` (green) and `cargo build -p app` (0 warnings).

```bash
git add crates/app/src/styles.rs crates/app/src/input.rs crates/app/src/main.rs
git commit -m "feat(app): Save Game Style writes a self-contained per-game style file"
```

---

### Task 4: Save buttons (Global/Game) + remove create-game-style command

Replace the editor's single "Save" button with "Save Global Style" and "Save Game Style"; wire the new button to `Action::StyleSaveGame`; add a `g` keyboard shortcut; and remove the now-redundant `create-game-style` command and `Action::GameStyle`.

**Files:**
- Modify: `crates/app/src/render/dialog.rs:28-39` (add `ButtonId::SaveGame`)
- Modify: `crates/app/src/render/style_editor.rs:86-89` (button labels)
- Modify: `crates/app/src/input.rs:635` (`style_dialog_action` mapping), `:1213` (key handler), `:85` + `:1797` (remove `Action::GameStyle` variant + handler), `:3623` (remove the GameStyle test call)
- Modify: `crates/app/src/slash.rs:312-314` (remove `create-game-style` CommandSpec) and the registry-count + help tests (~`:595-620`)
- Test: `crates/app/src/input.rs` and `crates/app/src/slash.rs`

**Interfaces:**
- Consumes: `Action::StyleSaveGame` (Task 3); `crate::render::dialog::ButtonId`; `style_dialog_action`.
- Produces: `ButtonId::SaveGame`; editor button row `[Save Global Style][Save Game Style][Cancel]`; registry count 47.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/input.rs` `mod tests`, add a button-mapping test. The editor builds `DialogRects` via `draw_dialog`; mirror the existing `style_dialog_action_buttons` test (around line 6366) by constructing a `DialogRects` with the three buttons and asserting the SaveGame rect maps to `StyleSaveGame`:

```rust
#[test]
fn style_dialog_action_maps_save_game() {
    use crate::render::dialog::{ButtonId, DialogRects};
    use ratatui::layout::Rect;
    let save_rect     = Rect { x: 10, y: 5, width: 18, height: 1 };
    let savegame_rect = Rect { x: 30, y: 5, width: 16, height: 1 };
    let cancel_rect   = Rect { x: 48, y: 5, width: 10, height: 1 };
    let rects = DialogRects {
        area: Rect::default(),
        content: Rect::default(),
        close: None,
        buttons: vec![
            (ButtonId::Save,     save_rect),
            (ButtonId::SaveGame, savegame_rect),
            (ButtonId::Cancel,   cancel_rect),
        ],
    };
    assert!(matches!(style_dialog_action(&rects, 31, 5), Some(Action::StyleSaveGame)),
        "clicking Save Game Style maps to StyleSaveGame");
    assert!(matches!(style_dialog_action(&rects, 11, 5), Some(Action::StyleSave)),
        "clicking Save Global Style maps to StyleSave");
}
```

In `crates/app/src/slash.rs` `mod tests`, update the existing registry-count assertion (`assert_eq!(COMMANDS.len(), 48, ...)` at ~line 605) from `48` to `47`, and update the descriptive comment above it (~lines 603-604) from `48 commands` / `Style 5` to `47 commands` / `Style 4`. Then add:

```rust
#[test]
fn create_game_style_command_removed() {
    assert!(find_command("create-game-style").is_none(),
        "create-game-style is replaced by the Save Game Style button");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app --lib style_dialog_action_maps_save_game create_game_style_command_removed`
Expected: FAIL — `ButtonId::SaveGame` and the removal don't exist yet (compile errors / count mismatch).

- [ ] **Step 3: Add the ButtonId + relabel/extend the buttons**

In `crates/app/src/render/dialog.rs`, add `SaveGame,` to the `ButtonId` enum (after `Save,`):

```rust
pub enum ButtonId {
    Save,
    SaveGame,
    Cancel,
    Ok,
    Done,
    Close,
    Reset,
    Resume,
    NewGame,
    Archive,
    Global,
}
```

In `crates/app/src/render/style_editor.rs:86-89`, change the buttons:

```rust
    let buttons = &[
        DialogButton { id: ButtonId::Save,     label: "Save Global Style" },
        DialogButton { id: ButtonId::SaveGame, label: "Save Game Style"   },
        DialogButton { id: ButtonId::Cancel,   label: "Cancel"            },
    ];
```

- [ ] **Step 4: Map the button + add the key**

In `crates/app/src/input.rs` `style_dialog_action` (line 635), add the SaveGame arm:

```rust
                ButtonId::Save     => Action::StyleSave,
                ButtonId::SaveGame => Action::StyleSaveGame,
                ButtonId::Cancel   => Action::StyleEditorCancel,
                _                  => Action::None,
```

In the editor key handler, add a `g` shortcut right after the `s` = StyleSave line (line 1213):

```rust
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::StyleSave,
        KeyCode::Char('g') if key.modifiers == KeyModifiers::NONE => Action::StyleSaveGame,
```

- [ ] **Step 5: Remove create-game-style + Action::GameStyle**

- In `crates/app/src/slash.rs`, delete the `create-game-style` `CommandSpec` block (lines 312-314).
- In `crates/app/src/input.rs`, delete the `GameStyle,` enum variant (line 85) and the `Action::GameStyle => { ... }` handler (lines 1797-1810) and the GameStyle test invocation at line 3623 (remove that test or the line that calls `apply_action(Action::GameStyle, ...)`; if the enclosing test exists only to exercise GameStyle, delete the whole test).
- In `crates/app/src/slash.rs` tests, remove any assertion referencing `create-game-style` (e.g. the `/create-game-style` help-listing assertion around line 617 and the category assertion around line 601).

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p app --lib style_dialog_action_maps_save_game create_game_style_command_removed`
Expected: PASS.

- [ ] **Step 7: Full suite + commit**

Run: `cargo test -p app` (green) and `cargo build -p app` (0 warnings). If the build flags an unused import or a non-exhaustive match from removing `GameStyle`, fix it surgically.

```bash
git add crates/app/src/render/dialog.rs crates/app/src/render/style_editor.rs crates/app/src/input.rs crates/app/src/slash.rs
git commit -m "feat(app): Save Global/Game Style buttons; remove create-game-style command"
```

---

### Task 5: fg/bg target indicator on the shared color region

Make it unambiguous which target (fg or bg) the shared custom-hex field and MRU strip affect: mark the active target's fg/bg label and tag the custom row, both following `ed.color_target` (`false` = fg, `true` = bg).

**Files:**
- Modify: `crates/app/src/render/style_editor.rs:272-303` (fg/bg label rows) and `:329-339` (custom row prefix)
- Test: `crates/app/src/render/style_editor.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ed.color_target: bool`.
- Produces: rendered `▸` marker on the active target's label; custom-row prefix ` hex →fg ` / ` hex →bg `.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/app/src/render/style_editor.rs` (mirror the existing render tests that use `TestBackend` and `draw_style_editor`):

```rust
#[test]
fn property_pane_shows_fg_bg_target_indicator() {
    let mut s = AppState::default();
    crate::input::open_style_editor_hermetic(&mut s);
    let area = Rect::new(0, 0, 120, 60);

    // Default target is fg.
    let mut buf = Buffer::empty(area);
    let _ = draw_style_editor(&s, area, &mut buf);
    let fg_text: String = buf.content().iter().flat_map(|c| c.symbol().chars()).collect();
    assert!(fg_text.contains("\u{2192}fg"), "custom row tags the fg target by default");

    // Switch target to bg.
    s.style_editor.as_mut().unwrap().color_target = true;
    let mut buf2 = Buffer::empty(area);
    let _ = draw_style_editor(&s, area, &mut buf2);
    let bg_text: String = buf2.content().iter().flat_map(|c| c.symbol().chars()).collect();
    assert!(bg_text.contains("\u{2192}bg"), "custom row tags the bg target when color_target is bg");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib property_pane_shows_fg_bg_target_indicator`
Expected: FAIL — no `→fg`/`→bg` tag in the custom row.

- [ ] **Step 3: Implement the indicator**

In `crates/app/src/render/style_editor.rs`, change the fg label row (around line 274) to mark the active target:

```rust
        // Row 1: fg label.
        if prop.height > 1 {
            let fg_focused = ed.focus == StyleFocus::Fg;
            let fg_lbl_style = if fg_focused { active_style } else { normal_style };
            let fg_mark = if !ed.color_target { "\u{25b8}" } else { " " }; // ▸ marks active target
            crate::render::draw_str_clipped(
                buf, prop.x, prop.y + 1,
                &format!("{}fg: {}", fg_mark, fg_val), fg_lbl_style, prop,
            );
        }
```

Change the bg label row (around line 290):

```rust
        // Row 4: bg label.
        if prop.height > 4 {
            let bg_focused = ed.focus == StyleFocus::Bg;
            let bg_lbl_style = if bg_focused { active_style } else { normal_style };
            let bg_mark = if ed.color_target { "\u{25b8}" } else { " " }; // ▸ marks active target
            crate::render::draw_str_clipped(
                buf, prop.x, prop.y + 4,
                &format!("{}bg: {}", bg_mark, bg_val), bg_lbl_style, prop,
            );
        }
```

Change the custom-row prefix (around line 330). Use `chars().count()` for the prefix width since the arrow is multi-byte but single-column:

```rust
            let prefix = if ed.color_target { " hex \u{2192}bg " } else { " hex \u{2192}fg " };
            let prefix_w = prefix.chars().count() as u16;
```

(Leave the rest of the custom-row body unchanged in this task; the field box is Task 6.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib property_pane_shows_fg_bg_target_indicator`
Expected: PASS.

- [ ] **Step 5: Full suite + commit**

Run: `cargo test -p app` (green) and `cargo build -p app` (0 warnings).

```bash
git add crates/app/src/render/style_editor.rs
git commit -m "feat(app): fg/bg target indicator on the shared color controls"
```

---

### Task 6: Obvious hex edit box

Render the custom hex row as a bracketed edit field with a visible cursor when focused, instead of bare `# <buf>` text.

**Files:**
- Modify: `crates/app/src/render/style_editor.rs:329-339` (custom-row body)
- Test: `crates/app/src/render/style_editor.rs` (`mod tests`)

**Interfaces:**
- Consumes: `ed.custom_buf: String`, `ed.focus == StyleFocus::Custom`, the `prefix`/`prefix_w` from Task 5.
- Produces: a `[ <buf>▏ ]` field; `custom_rect` covers the bracketed interior.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/app/src/render/style_editor.rs`:

```rust
#[test]
fn custom_hex_renders_bracketed_box_with_cursor_when_focused() {
    let mut s = AppState::default();
    crate::input::open_style_editor_hermetic(&mut s);
    {
        let ed = s.style_editor.as_mut().unwrap();
        ed.focus = crate::state::StyleFocus::Custom;
        ed.custom_buf = "#ab12cd".to_string();
    }
    let area = Rect::new(0, 0, 120, 60);
    let mut buf = Buffer::empty(area);
    let _ = draw_style_editor(&s, area, &mut buf);
    let text: String = buf.content().iter().flat_map(|c| c.symbol().chars()).collect();
    assert!(text.contains("[ #ab12cd"), "hex field is drawn as a bracketed box");
    assert!(text.contains("\u{258f}"), "a cursor glyph shows when the custom field is focused");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib custom_hex_renders_bracketed_box_with_cursor_when_focused`
Expected: FAIL — current row renders bare `# <buf>`, no brackets/cursor.

- [ ] **Step 3: Implement the edit box**

In `crates/app/src/render/style_editor.rs`, replace the custom-row body (the lines that build `custom_text` and `custom_rect`, around 332-338) with:

```rust
            let max_buf_w = prop.right().saturating_sub(prop.x + prefix_w + 4) as usize; // 4 = "[ " + " ]"
            let buf_display: String = ed.custom_buf.chars().take(max_buf_w).collect();
            let cursor = if custom_focused { "\u{258f}" } else { "" }; // ▏
            let field = format!("[ {}{} ]", buf_display, cursor);
            let custom_text = format!("{}{}", prefix, field);
            crate::render::draw_str_clipped(buf, prop.x, custom_y, &custom_text, cstyle, prop);
            // Hit-rect covers the bracketed field (interior + brackets).
            let field_w = field.chars().count() as u16;
            custom_rect = Some(Rect::new(prop.x + prefix_w, custom_y, field_w, 1));
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib custom_hex_renders_bracketed_box_with_cursor_when_focused`
Expected: PASS.

- [ ] **Step 5: Full suite + commit**

Run: `cargo test -p app` (green) and `cargo build -p app` (0 warnings).

```bash
git add crates/app/src/render/style_editor.rs
git commit -m "feat(app): obvious bracketed hex edit box in the style editor"
```

---

## Notes for the executor

- **Deliberate spec refinement (Task 4):** the spec describes the Save Game Style button as "drawn dimmed" when no game is loaded. Dimming would require an `enabled` field on `DialogButton`, which has 25 construction sites across 16 files — out of proportion for one button. Instead the button is always visible and the `Action::StyleSaveGame` handler guards (`no game loaded` status, editor stays open). This preserves the spec's "no-op + status" behavior; only the grey-out is dropped. Flagged for the reviewer.
- **Render tests (Tasks 5, 6):** the plan's render tests use the same form as the existing tests in `render/style_editor.rs` — `let mut buf = Buffer::empty(area); draw_style_editor(&s, area, &mut buf);` then scrape `buf.content()`. `Rect` and `Buffer` are already in scope in that test module.
- Every style-editor test uses `open_style_editor_hermetic` except Tasks 2 and 3's file-on-disk tests, which build their own temp `user_dir`.
