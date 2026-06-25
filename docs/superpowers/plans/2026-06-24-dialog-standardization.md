# Dialog Standardization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every button-bearing modal a uniform keyboard model — a default (confirm) button that is underlined and starts focused so Enter triggers it, Tab/Shift-Tab cycle button focus, Esc always cancels — while leaving navigation panels' existing keys intact.

**Architecture:** A shared change to `render/dialog.rs` (DialogSpec gains `default` + `focus`; `draw_dialog` underlines the default and highlights the focused button) plus a `cycle_focus` helper and one `AppState.dialog_focus` field. Each modal's existing key handler gains Tab/Shift-Tab → cycle, Enter → activate the focused button, and passes `default`/`focus` into its `draw_dialog` call.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28. Tests are `#[test]` in-module, run with `cargo test -p app`.

## Global Constraints

- Esc and the clickable `✕` always cancel/close every modal; `✕` is NOT in the Tab ring. (verbatim from spec)
- Existing letter accelerators (`r`/`c`/`s`/`q`/`n`) and mouse button clicks keep working. (verbatim from spec)
- Text-input modals (prompt, hints input, config path edit) keep Enter = submit field, Esc = cancel; their typing behavior must not change. (verbatim from spec)
- **Option A:** text prompts stay minimal (field + Enter/Esc, no button row) — no task touches `prompt_key_to_action`.
- **Navigation-panel exception:** `verb_menu` (Tab/Shift-Tab = pane nav) and `file_browser` (Enter = open dir / select) keep their existing keys; they gain the underlined default button as a visual/mouse affordance only — no Tab-focus ring and no Enter override for them.
- Focus index is always clamped to `0..buttons.len()`; opening any modal resets `dialog_focus` to the default button's index.
- After each task: `cargo test -p app` green and `cargo build -p app` shows 0 warnings.

---

### Task 1: Shared focus chrome (DialogSpec + draw_dialog + cycle_focus)

**Files:**
- Modify: `crates/app/src/render/dialog.rs` (DialogSpec struct ~60-65; draw_dialog button loop ~149-181)
- Modify: `crates/app/src/input.rs` (add `cycle_focus` helper near other internal helpers)
- Modify: `crates/app/src/state.rs` (add `dialog_focus` field to AppState + default)
- Test: same files (in-module `#[test]`)

**Interfaces:**
- Produces:
  - `DialogSpec` gains `pub default: Option<ButtonId>` and `pub focus: Option<usize>`.
  - `pub fn cycle_focus(idx: usize, len: usize, delta: i32) -> usize` in `input.rs`.
  - `AppState.dialog_focus: usize` (reset to a button index when a modal opens).

- [ ] **Step 1: Add the failing draw_dialog test**

In `crates/app/src/render/dialog.rs` tests module, add:

```rust
#[test]
fn dialog_underlines_default_and_highlights_focus() {
    use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier}};
    let full = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(full);
    let st = DialogStyle {
        frame: Style::default(),
        box_style: BorderStyle::Single,
        title: Style::default(),
        button: Style::default(),
        button_active: Style::default().add_modifier(Modifier::REVERSED),
        shadow: Style::default(),
        shadow_on: false,
    };
    let spec = DialogSpec {
        title: "T",
        placement: Placement::Centered { w: 30, h: 6 },
        buttons: &[
            DialogButton { id: ButtonId::Save, label: "Save" },
            DialogButton { id: ButtonId::Cancel, label: "Cancel" },
        ],
        show_close: true,
        default: Some(ButtonId::Save),
        focus: Some(1),
    };
    let rects = draw_dialog(&mut buf, &spec, &st);
    // The Save (default) button label cells carry UNDERLINED.
    let (_, save_rect) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Save).unwrap();
    let save_cell = buf.cell((save_rect.x + 2, save_rect.y)).unwrap(); // inside "[ "
    assert!(save_cell.style().add_modifier.contains(Modifier::UNDERLINED),
        "default button must be underlined");
    // The Cancel (focused idx 1) button cells carry REVERSED (button_active).
    let (_, cancel_rect) = rects.buttons.iter().find(|(id, _)| *id == ButtonId::Cancel).unwrap();
    let cancel_cell = buf.cell((cancel_rect.x + 2, cancel_rect.y)).unwrap();
    assert!(cancel_cell.style().add_modifier.contains(Modifier::REVERSED),
        "focused button must use button_active");
}
```

- [ ] **Step 2: Run it; expect a compile failure**

Run: `cargo test -p app dialog_underlines_default_and_highlights_focus 2>&1 | tail -5`
Expected: FAIL — `DialogSpec` has no field `default` / `focus`.

- [ ] **Step 3: Add the DialogSpec fields**

In `crates/app/src/render/dialog.rs`, extend the struct:

```rust
pub struct DialogSpec<'a> {
    pub title: &'a str,
    pub placement: Placement,
    pub buttons: &'a [DialogButton],
    pub show_close: bool,
    /// The confirm button: rendered underlined; Enter triggers it by default.
    pub default: Option<ButtonId>,
    /// Index into `buttons` to highlight with `button_active` (Tab focus).
    pub focus: Option<usize>,
}
```

- [ ] **Step 4: Style each button by default/focus in the draw loop**

In `draw_dialog`, replace the per-button draw in the `for btn in spec.buttons.iter().rev()` loop so the style is computed per button. Track the original index because the loop is reversed:

```rust
let n = spec.buttons.len();
for (rev_i, btn) in spec.buttons.iter().rev().enumerate() {
    let orig_i = n - 1 - rev_i;
    let label_chars = btn.label.chars().count() as u16;
    let btn_width = 4 + label_chars;
    if col < btn_width || col.saturating_sub(btn_width) < pane.content.x {
        break;
    }
    col = col.saturating_sub(btn_width);
    let bx = col;

    // Focused button uses button_active; default button is underlined.
    let mut style = if spec.focus == Some(orig_i) { st.button_active } else { st.button };
    if spec.default == Some(btn.id) {
        style = style.add_modifier(ratatui::style::Modifier::UNDERLINED);
    }

    let btn_str = format!("[ {} ]", btn.label);
    let mut draw_x = bx;
    for ch in btn_str.chars() {
        if draw_x < pane.content.right() {
            if let Some(cell) = buf.cell_mut((draw_x, button_row_y)) {
                let mut tmp = [0u8; 4];
                cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
            }
            draw_x += 1;
        }
    }
    button_rects.push((btn.id, Rect::new(bx, button_row_y, btn_width, 1)));
}
```

- [ ] **Step 5: Fix existing draw_dialog call sites to set the new fields**

Every existing `DialogSpec { … }` literal must add `default: None, focus: None,` (no behavior change yet). Find them:

Run: `grep -rn "DialogSpec {" crates/app/src | grep -v "render/dialog.rs"`
Add `default: None, focus: None,` to each literal (and the one in dialog.rs tests at ~215).

- [ ] **Step 6: Run the test; expect PASS**

Run: `cargo test -p app dialog_underlines_default_and_highlights_focus 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 7: Add cycle_focus + its test**

In `crates/app/src/input.rs` add:

```rust
/// Cycle a button-focus index by `delta` (+1 Tab, -1 Shift-Tab), wrapping within
/// `0..len`. Returns 0 when `len` is 0.
pub(crate) fn cycle_focus(idx: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let next = idx as i32 + delta;
    next.rem_euclid(len as i32) as usize
}
```

In the `input.rs` tests module:

```rust
#[test]
fn cycle_focus_wraps_both_ways() {
    assert_eq!(cycle_focus(0, 3, 1), 1);
    assert_eq!(cycle_focus(2, 3, 1), 0); // wrap forward
    assert_eq!(cycle_focus(0, 3, -1), 2); // wrap backward
    assert_eq!(cycle_focus(5, 0, 1), 0); // empty
}
```

- [ ] **Step 8: Add AppState.dialog_focus**

In `crates/app/src/state.rs`, add `pub dialog_focus: usize,` to `AppState` and `dialog_focus: 0,` to its `Default` impl (near the other overlay fields around line 711).

- [ ] **Step 9: Run all app tests; expect green, 0 warnings**

Run: `cargo test -p app 2>&1 | grep "test result"` then `cargo build -p app 2>&1 | grep -c warning`
Expected: all pass; `0`.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/render/dialog.rs crates/app/src/input.rs crates/app/src/state.rs
git commit -m "feat(dialog): default-underline + focus-highlight chrome and cycle_focus helper"
```

---

### Task 2: Confirm dialogs in main.rs (reset, quit, launch)

**Files:**
- Modify: `crates/app/src/main.rs` — `reset_dialog_key` (~2110), `quit_dialog_key` (~2246), `launch_dialog_key` (~2267), and the three render call sites that build their `DialogSpec` and dispatch the `*_DialogAction`.

**Interfaces:**
- Consumes: `cycle_focus` (Task 1), `AppState.dialog_focus`, `DialogSpec.default`/`focus`.

These three handlers currently map keys directly to a `*DialogAction`. Add Tab/Shift-Tab focus cycling and make Enter activate the focused button, while keeping the existing accelerator keys.

Button order (Tab ring) and default per the spec table:
- reset: `[Reset, Cancel]`, default `Reset`.
- quit: `[Save & quit, Quit, Cancel]`, default `Save & quit`.
- launch: `[Resume, New game]`, default `Resume`.

- [ ] **Step 1: Failing test — Tab moves focus then Enter fires the focused action (reset)**

In `crates/app/src/main.rs` tests:

```rust
#[test]
fn reset_dialog_tab_then_enter_fires_focused() {
    use crossterm::event::KeyCode;
    // buttons: [Reset(0), Cancel(1)], default focus 0.
    // Tab -> focus 1 (Cancel); Enter on focus 1 -> Cancel.
    let mut focus = 0usize;
    focus = crate::input::cycle_focus(focus, 2, 1);
    assert_eq!(focus, 1);
    let act = reset_dialog_key_focused(KeyCode::Enter, focus);
    assert!(matches!(act, ResetDialogAction::Cancel));
}
```

- [ ] **Step 2: Run; expect compile failure (no `reset_dialog_key_focused`)**

Run: `cargo test -p app reset_dialog_tab_then_enter_fires_focused 2>&1 | tail -5`
Expected: FAIL — function not found.

- [ ] **Step 3: Add a focus-aware variant and route Tab**

Add alongside `reset_dialog_key`:

```rust
/// Reset-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button and keeps
/// the existing accelerators.
fn reset_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => ResetDialogAction::ToggleClear,
        KeyCode::Enter => match focus {
            1 => ResetDialogAction::Cancel,
            _ => ResetDialogAction::Confirm, // focus 0 = Reset (default)
        },
        _ => ResetDialogAction::None,
    }
}
```

In the reset-dialog event-loop arm, before calling the handler, intercept Tab/BackTab:

```rust
match key.code {
    crossterm::event::KeyCode::Tab =>
        state.dialog_focus = crate::input::cycle_focus(state.dialog_focus, 2, 1),
    crossterm::event::KeyCode::BackTab =>
        state.dialog_focus = crate::input::cycle_focus(state.dialog_focus, 2, -1),
    code => match reset_dialog_key_focused(code, state.dialog_focus) {
        /* existing ResetDialogAction handling */
    },
}
```

- [ ] **Step 4: Run the test; expect PASS**

Run: `cargo test -p app reset_dialog_tab_then_enter_fires_focused 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Pass default + focus into the reset DialogSpec**

At the reset-dialog render call site, set `default: Some(ButtonId::Reset), focus: Some(state.dialog_focus),` on the `DialogSpec`.

- [ ] **Step 6: Reset dialog_focus to 0 when the dialog opens**

Wherever `state.reset_dialog = true;` is set, also set `state.dialog_focus = 0;`.

- [ ] **Step 7: Repeat Steps 1-6 for quit (3 buttons, default Save & quit) and launch (2 buttons, default Resume)**

`quit_dialog_key_focused(code, focus)`: Enter → focus 0 = SaveQuit, 1 = Quit, 2 = Cancel; keep `s`/`q`/`c`/Esc. Tab ring length 3. DialogSpec `default: Some(ButtonId::Save)` (the "Save & quit" button id), reset `dialog_focus = 0` on open.

`launch_dialog_key_focused(code, focus)`: Enter → focus 0 = Resume, 1 = NewGame; keep `r`/`n`/Esc. Tab ring length 2. DialogSpec `default: Some(ButtonId::Resume)`, reset on open.

Add one `*_tab_then_enter_fires_focused` test per dialog mirroring Step 1.

- [ ] **Step 8: Run all app tests; commit**

Run: `cargo test -p app 2>&1 | grep "test result"`; `cargo build -p app 2>&1 | grep -c warning`
Expected: pass; `0`.

```bash
git add crates/app/src/main.rs
git commit -m "feat(dialog): Tab focus + Enter-activates-focused for reset/quit/launch dialogs"
```

---

### Task 3: config_screen + saves (input.rs button dialogs)

**Files:**
- Modify: `crates/app/src/input.rs` — `config_screen_key_to_action` (~897), `saves_key_to_action` (~831), and their render call sites in `main.rs`.

**Interfaces:**
- Consumes: Task 1 helpers/fields.

config_screen Tab ring `[Save, Cancel]`, default `Save`. saves Tab ring `[Load, Save as, Delete, Done]`, default `Load`.

- [ ] **Step 1: Failing test (config_screen) — Tab then Enter fires focused**

```rust
#[test]
fn config_screen_tab_then_enter_fires_cancel() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut s = AppState::default();
    s.config_screen = Some(Default::default());
    s.dialog_focus = crate::input::cycle_focus(0, 2, 1); // focus Cancel
    let a = config_screen_key_to_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), s.dialog_focus);
    assert!(matches!(a, Action::ConfigScreenCancel));
}
```

(Use the actual `config_screen` constructor / `Action` cancel variant names found in the file; adjust the assertion to the real variant.)

- [ ] **Step 2: Run; expect failure (signature/variant mismatch)**

Run: `cargo test -p app config_screen_tab_then_enter_fires_cancel 2>&1 | tail -5`
Expected: FAIL.

- [ ] **Step 3: Thread `focus: usize` into the handler; Enter picks the focused button**

Change `config_screen_key_to_action(key)` to `config_screen_key_to_action(key, focus: usize)`. Keep the existing `s`/Enter-in-path-edit/Esc arms; for a plain Enter in normal mode, map by focus (0 = Save, 1 = Cancel). Intercept Tab/BackTab in the event loop to cycle `dialog_focus` (len 2). Do the same for `saves_key_to_action(key, focus)` (len 4; Enter → Load/SaveAs/Delete/Done by focus, keep existing select keys).

- [ ] **Step 4: Run the test; expect PASS**

Run: `cargo test -p app config_screen_tab_then_enter_fires_cancel 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Set default + focus on both DialogSpecs; reset dialog_focus on open**

config_screen: `default: Some(ButtonId::Save)`; saves: `default: Some(ButtonId::Ok)` if Load uses `ButtonId::Ok`, else add/choose the matching id. Reset `dialog_focus = 0` where each opens. Add a `saves_tab_then_enter` test mirroring Step 1.

- [ ] **Step 6: Run all app tests; commit**

```bash
git add crates/app/src/input.rs crates/app/src/main.rs
git commit -m "feat(dialog): Tab focus + Enter-activates-focused for config and saves dialogs"
```

---

### Task 4: gallery — OK button + Enter confirm

**Files:**
- Modify: `crates/app/src/input.rs` — `gallery_key_to_action` (~861), gallery action handling (`GalleryClose`, add `GalleryApply`).
- Modify: `crates/app/src/render/gallery.rs` — button set (replace `Done` with `OK`).
- Modify: `crates/app/src/main.rs` — gallery DialogSpec `default`/`focus`.

**Interfaces:**
- Consumes: Task 1. Produces: `Action::GalleryApply` (persist current selection to the personal style/config, then close).

> Scope note: this task covers only the #2 standardization for the gallery (OK button + Enter = apply/confirm, underlined default). The separate #3 work (remember the active selection on open; centre the OK button) is NOT in this plan.

- [ ] **Step 1: Failing test — Enter maps to GalleryApply**

```rust
#[test]
fn gallery_enter_is_apply() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let a = gallery_key_to_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(a, Action::GalleryApply));
}
```

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p app gallery_enter_is_apply 2>&1 | tail -5`
Expected: FAIL — no `GalleryApply` / Enter arm.

- [ ] **Step 3: Add the Action variant, the key arm, and the handler**

Add `GalleryApply` to the `Action` enum (near `GalleryExportStyle`). In `gallery_key_to_action`, add `KeyCode::Enter => Action::GalleryApply,`. In `apply_action`, add a `GalleryApply` arm that persists the current `GalleryState::symbol_config()` (mirror what `GalleryExportStyle`/save-to-personal-style does today) and then `state.gallery = None;`.

- [ ] **Step 4: Run the test; expect PASS**

Run: `cargo test -p app gallery_enter_is_apply 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Replace the gallery `Done` button with `OK`; set default/focus**

In `render/gallery.rs` (or the gallery DialogSpec in `main.rs`) change the button from `ButtonId::Done`/"Done" to `ButtonId::Ok`/"OK", set `default: Some(ButtonId::Ok)`, `focus: Some(0)`, and reset `dialog_focus = 0` when the gallery opens. Update any test asserting the "Done" label.

- [ ] **Step 6: Run all app tests; commit**

```bash
git add crates/app/src/input.rs crates/app/src/render/gallery.rs crates/app/src/main.rs
git commit -m "feat(dialog): gallery gains an OK button; Enter applies the selection"
```

---

### Task 5: read-only / single-button panels (room_panel, tidy_anim, hotkey_dialog)

**Files:**
- Modify: `crates/app/src/input.rs` (room_panel Esc check ~312; hotkey handler ~779; tidy_anim handling ~263) and their render call sites.

**Interfaces:**
- Consumes: Task 1.

Each gains a single centered confirm button that equals "close": room_panel `[OK]`, tidy_anim `[OK]`, hotkey_dialog already has `[Done]`. The default button is underlined and Enter closes the panel (these have no text input and no competing Enter use).

- [ ] **Step 1: Failing test — Enter closes the room panel**

```rust
#[test]
fn room_panel_enter_closes() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut s = AppState::default();
    s.room_panel = Some(Default::default()); // use the real constructor
    handle_room_panel_key(&mut s, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(s.room_panel.is_none(), "Enter must close the room panel");
}
```

(Match the real room_panel field/constructor and the existing close path.)

- [ ] **Step 2: Run; expect failure**

Run: `cargo test -p app room_panel_enter_closes 2>&1 | tail -5`
Expected: FAIL (Enter not handled).

- [ ] **Step 3: Make Enter close room_panel and tidy_anim; add the OK button to each DialogSpec**

At the room_panel key check (currently Esc only), add `KeyCode::Enter` to the close branch. Same for tidy_anim (keep Space = toggle play). Add `ButtonId::Ok`/"OK" to each panel's DialogSpec with `default: Some(ButtonId::Ok), focus: Some(0)`. For hotkey_dialog, set `default: Some(ButtonId::Done)` and ensure Enter closes it.

- [ ] **Step 4: Run the test; expect PASS**

Run: `cargo test -p app room_panel_enter_closes 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 5: Run all app tests; commit**

```bash
git add crates/app/src/input.rs crates/app/src/main.rs
git commit -m "feat(dialog): read-only panels gain an underlined OK that closes on Enter"
```

---

### Task 6: navigation panels (verb_menu, file_browser) — visual default only

**Files:**
- Modify: `crates/app/src/render/*` / `main.rs` DialogSpec for verb_menu and file_browser.

**Interfaces:**
- Consumes: Task 1.

Per the Global Constraints navigation-panel exception, these keep ALL existing keys (verb_menu Tab = pane nav; file_browser Enter = open/select). The only change is rendering the default button underlined as a visual/mouse affordance: verb_menu default `OK` (apply), file_browser default `Select`. No Tab-focus ring, no Enter remap, no handler change.

- [ ] **Step 1: Test — verb_menu Tab still navigates panes (regression guard)**

```rust
#[test]
fn verb_menu_tab_still_navigates_panes() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let a = verb_menu_key_to_action(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(matches!(a, Action::VerbMenuNav(VerbMenuNavKind::NextPane)));
}
```

- [ ] **Step 2: Run; expect PASS already (no handler change)**

Run: `cargo test -p app verb_menu_tab_still_navigates_panes 2>&1 | tail -5`
Expected: PASS (this is a guard that the exception is honored).

- [ ] **Step 3: Set the underlined default on both DialogSpecs**

verb_menu DialogSpec: ensure an `OK` button exists with `default: Some(ButtonId::Ok)` (keep `Done`); `focus: None` (no focus ring). file_browser DialogSpec: `default: Some(ButtonId::Ok)` for its Select/confirm button (add one if absent), `focus: None`.

- [ ] **Step 4: Run all app tests; commit**

```bash
git add crates/app/src/main.rs crates/app/src/render
git commit -m "feat(dialog): underline the default button on verb-menu and file-browser (visual only)"
```

---

### Task 7: hints panel — underlined Close, Enter still submits input

**Files:**
- Modify: `crates/app/src/main.rs` — `hint_key_routes` (~2083) call site / hints DialogSpec.

**Interfaces:**
- Consumes: Task 1.

The hints panel has a text input: Enter submits a hint command (unchanged). Its single `Close` button becomes the underlined default; Esc and the existing input behavior are untouched.

- [ ] **Step 1: Regression test — Enter still routes to the hint input, not "close"**

```rust
#[test]
fn hints_enter_submits_input_not_close() {
    // hint_key_routes must report that Enter is consumed as input submission
    // while the hints panel is open (assert against its real return type).
    let routed = hint_key_routes(crossterm::event::KeyCode::Enter);
    assert!(routed, "Enter must be routed to the hint session input");
}
```

(Adjust to the real `hint_key_routes` signature/return.)

- [ ] **Step 2: Run; expect PASS (guard)**

Run: `cargo test -p app hints_enter_submits_input_not_close 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 3: Set the hints DialogSpec default to Close**

Set `default: Some(ButtonId::Close), focus: None` on the hints DialogSpec so the Close button renders underlined. No key handler change.

- [ ] **Step 4: Run all app tests; commit**

```bash
git add crates/app/src/main.rs
git commit -m "feat(dialog): underline the Close button on the hints panel"
```

---

### Task 8: README + style docs

**Files:**
- Modify: `README.md` (Unified dialogs bullet), confirm `style.toml` schema unaffected.

- [ ] **Step 1: Update the README "Unified dialogs" bullet** to mention Tab-cycled button focus, the underlined default button, and Enter = the focused/default confirm.

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: README — dialog Tab focus + underlined default button"
```
