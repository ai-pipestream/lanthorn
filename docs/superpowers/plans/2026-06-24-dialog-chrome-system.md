# Dialog Chrome System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One shared, themeable dialog chrome (centered/opaque/bordered frame + clickable [X] + tiered OK/Cancel/Save/Done buttons + optional drop-shadow) adopted by all 9 modals, with uniform ESC==[X] close, the cursor/bleed fixes, and the gallery redesign.

**Architecture:** A new `render/dialog.rs` builds on wave18's `render/paneframe.rs` (`draw_pane_frame`, `draw_top_inset`, `BorderStyle`): `draw_dialog` fills an opaque bg, draws the bordered frame + centered title, an [X], an optional button row, and an optional shadow, returning `DialogRects` (hit-rects). New `dialog*` style selectors plug into the #43 system. Phase 1 builds the component + proves it on the config screen; Phase 2 migrates the rest + mouse hit-testing + gallery redesign.

**Tech Stack:** Rust, ratatui 0.29; `render/paneframe.rs` (merged wave18); the `style.rs`/`colors.rs` style system.

## Global Constraints

- ESC and a click on [X] route to the SAME close action per modal (edit/confirm dialogs → Cancel/discard; list/action → close). Remove `q`-as-close from saves/filebrowser/verbmenu/hotkey-dialog (q freed; do not rebind it here).
- All modal colors come from `state.colors.dialog_*` (no hardcoded modal colors). The dialog border uses a configurable `dialog_box_style` (a `BorderStyle`). Drop-shadow off by default.
- Opaque bg via `Style::reset().bg(...)` over the full dialog rect (incl. the help-bar row it overlaps) — the bleed fix. The logical `_` cursor is suppressed whenever any overlay is open.
- New selectors integrate with the #43 system: SELECTOR_FIELDS, `DEFAULT_STYLE_TOML`, gallery/config writers, `write_style_full`.
- Reuse `paneframe::{draw_pane_frame, draw_top_inset, BorderStyle, parse_border_style, border_style_name}`. Do NOT duplicate border drawing.
- No `mapper`/`zvm` changes. Build + `cargo test --workspace` green and warning-clean after every task (currently warning-clean; add none).
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Spec: `docs/superpowers/specs/2026-06-24-dialog-chrome-system-design.md` (source of truth; read it).

## File structure
- **Create `crates/app/src/render/dialog.rs`** — `centered_rect`, `ButtonId`, `DialogButton`, `Placement`, `DialogSpec`, `DialogRects`, `draw_dialog`.
- **Modify `render/mod.rs`** — `pub mod dialog;`.
- **Modify `style.rs`/`colors.rs`** — `dialog*` selectors + `ColorScheme` fields + `dialog_box_style`.
- **Modify `state.rs`** — `any_overlay_open()`.
- **Modify `render/transcript.rs`** — suppress cursor when overlay open.
- **Modify each `render/*` modal + `main.rs` + `input.rs`** — adopt the chrome, mouse hit-testing, ESC==X, remove q-close, gallery redesign.

---

## PHASE 1 — component + config-screen proof

### Task 1: dialog.rs — centered_rect + draw_dialog + DialogRects

**Files:** Create `crates/app/src/render/dialog.rs`; Modify `render/mod.rs`.

**Interfaces — Produces:**
- `pub fn centered_rect(area: Rect, w: u16, h: u16) -> Rect` (clamped to area).
- `pub enum Placement { Centered { w: u16, h: u16 }, Positioned(Rect) }`
- `pub enum ButtonId { Save, Cancel, Ok, Done, Close }`
- `pub struct DialogButton { pub id: ButtonId, pub label: &'static str }`
- `pub struct DialogStyle { pub frame: Style, pub box_style: BorderStyle, pub title: Style, pub button: Style, pub button_active: Style, pub shadow: Style, pub shadow_on: bool }`
- `pub struct DialogSpec<'a> { pub title: &'a str, pub placement: Placement, pub buttons: &'a [DialogButton], pub show_close: bool }`
- `pub struct DialogRects { pub area: Rect, pub content: Rect, pub close: Option<Rect>, pub buttons: Vec<(ButtonId, Rect)> }`
- `pub fn draw_dialog(buf: &mut Buffer, spec: &DialogSpec, st: &DialogStyle) -> DialogRects` — (1) resolve `area` from placement; (2) if `st.shadow_on` paint a +1/+1 offset shadow (bottom+right, clamped to buffer); (3) fill `area` opaque with `Style::reset().patch(st.frame)`; (4) `draw_pane_frame(buf, area, st.box_style, st.frame)` for border; (5) overlay the centered title via `draw_top_inset` (single `InsetSegment{text:title,active:false}`, style `st.title`); (6) if `show_close` draw `✕` just inside the top-right border, record `close`; (7) if `buttons` non-empty, draw a right-aligned bottom button row `[ Label ]` each in `st.button`, record `(id, rect)`; (8) return `DialogRects` (content = frame content minus the button row).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn dialog_opaque_bg_covers_underlying_and_records_rects() {
    use ratatui::{buffer::Buffer, layout::Rect, style::{Style, Modifier, Color}};
    let full = Rect::new(0,0,40,12);
    let mut buf = Buffer::empty(full);
    // pre-fill a REVERSED cell where the dialog will sit
    buf.cell_mut((20,6)).unwrap().set_symbol("X").set_style(Style::new().add_modifier(Modifier::REVERSED));
    let st = DialogStyle{ frame: Style::new().bg(Color::Black), box_style: BorderStyle::Single, title: Style::default(), button: Style::default(), button_active: Style::default(), shadow: Style::default(), shadow_on:false };
    let spec = DialogSpec{ title:"Settings", placement: Placement::Centered{w:20,h:8}, buttons: &[DialogButton{id:ButtonId::Save,label:"Save"},DialogButton{id:ButtonId::Cancel,label:"Cancel"}], show_close:true };
    let r = draw_dialog(&mut buf, &spec, &st);
    // opaque: the covered cell no longer REVERSED
    assert!(!buf.cell((20,6)).unwrap().modifier.contains(Modifier::REVERSED));
    assert!(r.close.is_some());
    assert_eq!(r.buttons.len(), 2);
    assert!(r.content.width > 0 && r.content.height > 0);
}

#[test]
fn centered_rect_centers_and_clamps() {
    use ratatui::layout::Rect;
    assert_eq!(centered_rect(Rect::new(0,0,40,12), 20, 8), Rect::new(10,2,20,8));
    let big = centered_rect(Rect::new(0,0,10,4), 20, 8); // clamps to area
    assert!(big.width <= 10 && big.height <= 4);
}

#[test]
fn dialog_shadow_paints_offset_cells_when_on() {
    use ratatui::{buffer::Buffer, layout::Rect, style::{Style,Color}};
    let mut buf = Buffer::empty(Rect::new(0,0,40,12));
    let st = DialogStyle{ frame: Style::new().bg(Color::Black), box_style: BorderStyle::Single, title:Style::default(), button:Style::default(), button_active:Style::default(), shadow: Style::new().bg(Color::DarkGray), shadow_on:true };
    let spec = DialogSpec{ title:"T", placement: Placement::Centered{w:10,h:5}, buttons:&[], show_close:false };
    let r = draw_dialog(&mut buf, &spec, &st);
    // a cell just below-right of the frame carries the shadow bg
    let sx = r.area.right(); let sy = r.area.bottom();
    if sx < 40 && sy < 12 { assert_eq!(buf.cell((sx, sy)).unwrap().style().bg, Some(Color::DarkGray)); }
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `centered_rect`, the structs, and `draw_dialog` (reusing `paneframe::draw_pane_frame`/`draw_top_inset`). Add `pub mod dialog;` to render/mod.rs.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(dialog): shared dialog chrome (centered/opaque/border/[X]/buttons/shadow)".

---

### Task 2: dialog style selectors + ColorScheme fields

**Files:** Modify `style.rs`, `colors.rs`.

**Interfaces — Produces:** `ColorScheme` gains `dialog, dialog_title, dialog_button, dialog_button_active, dialog_shadow` (Styles) + `dialog_box_style: BorderStyle`. Selectors `dialog` (carries a `style` key for the box style + colors), `dialog:title`, `dialog:button`, `dialog:button:active`, `dialog:shadow` added to `SELECTOR_FIELDS` + `apply_color_decls` (the `dialog` selector's `style` key sets `dialog_box_style` via `parse_border_style`). `DEFAULT_STYLE_TOML`: `"dialog" = { style = "single" }` + sensible default colors (e.g. bg dark, border accent). `write_style_full` emits them (incl. the `style` key + a `dialog_shadow` on/off — represent shadow-on as the presence/`bold`? No: add a `dialog_shadow_on: bool` to ColorScheme, set when `dialog:shadow` has a non-default value OR via a `dialog` `shadow = true` key — keep it simple: a `shadow` bool key on the `dialog` selector).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn dialog_selectors_resolve_with_box_style_and_default() {
    let doc = parse_style_toml(DEFAULT_STYLE_TOML).unwrap();
    let (cs,_s,_w) = resolve(&doc, std::path::Path::new("."));
    assert!(matches!(cs.dialog_box_style, crate::render::paneframe::BorderStyle::Single));
    let d2 = parse_style_toml("[colors]\n\"dialog\" = { style = \"double\", bg = \"black\" }\n\"dialog:button\" = { fg = \"cyan\" }\n").unwrap();
    let (cs2,_s,_w) = resolve(&d2, std::path::Path::new("."));
    assert!(matches!(cs2.dialog_box_style, crate::render::paneframe::BorderStyle::Double));
    assert_eq!(cs2.dialog_button.fg, Some(ratatui::style::Color::Cyan));
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the fields, selectors, `DEFAULT_STYLE_TOML`, `write_style_full`. Default the new `ColorScheme` fields in `terminal_default`/`from_ghostty`.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(style): dialog chrome selectors".

---

### Task 3: Overlay-open cursor guard

**Files:** Modify `state.rs`, `render/transcript.rs`.

**Interfaces — Produces:** `AppState::any_overlay_open(&self) -> bool` (true if any modal/dialog/dock state is `Some`/active — gallery, saves, file_browser, config_screen, verb_menu, hotkey_dialog, room_panel, inspector/tidy, etc.). Read which AppState fields represent open overlays and OR them.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn any_overlay_open_reflects_state() {
    let mut s = AppState::default();
    assert!(!s.any_overlay_open());
    s.gallery = Some(Default::default()); // or the real constructor
    assert!(s.any_overlay_open());
}
```
(Adjust the field set to the real overlay fields.)
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** `any_overlay_open`; in `transcript.rs` change the cursor render guard from `focus==Game` to `focus==Game && !state.any_overlay_open()`.
- [ ] **Step 4: Run, confirm pass; add a transcript test that the `_` cursor is absent when an overlay is open; build clean.**
- [ ] **Step 5: Commit** — "feat(ui): suppress story cursor while an overlay is open".

---

### Task 4: Config screen adopts the chrome (proof) + button mouse routing

**Files:** Modify `render/config_screen.rs`, `input.rs`, `main.rs`, `state.rs` (PaneRects).

**Interfaces — Produces:** `PaneRects` gains `dialog: Option<DialogRects>` (the active dialog's rects). config screen renders via `draw_dialog` with `[Save]/[Cancel]` + `show_close`; its content draws into `content`. `mouse_to_action` gains the `dialog: &Option<DialogRects>` param and, when present, maps a click on `close`→`ConfigCancel`, on `Save`→`ConfigSave`, on `Cancel`→`ConfigCancel`, and SWALLOWS clicks outside `area`. ESC already cancels; ensure ESC==[X]==Cancel.

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn config_dialog_button_clicks_map_to_actions() {
    // build a DialogRects with known close + Save/Cancel rects; call the
    // dialog-aware mouse mapper; assert click on close->ConfigCancel,
    // Save rect->ConfigSave, Cancel rect->ConfigCancel, outside->no-op.
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the config-screen chrome adoption, `PaneRects.dialog`, and the dialog-aware branch in `mouse_to_action`. Read the current config_screen render + the saves/config mouse plumbing first.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean; add a render test that the config screen shows a border + title + [Save]/[Cancel]/[X] and reads colors from `state.colors.dialog_*`.**
- [ ] **Step 5: Commit** — "feat(config): adopt dialog chrome + button mouse routing".

---

## PHASE 2 — migrate the rest + gallery redesign

### Task 5: Migrate saves + file browser

**Files:** `render/saves.rs`, `render/filebrowser.rs`, `input.rs`, `main.rs`.

**Interfaces — Consumes:** `draw_dialog`, `PaneRects.dialog`, the dialog-aware `mouse_to_action`. **Produces:** both render via `draw_dialog` (centered, `show_close`, `[Done]`), keep their action keys + footer hints; [X]/[Done]/ESC → close; `q`-close removed. Their button/X rects flow through `PaneRects.dialog` to the mouse mapper.

- [ ] **Step 1–5 (TDD):** render test that saves + file browser show the bordered titled chrome with [X]/[Done] and `dialog_*` colors; a mouse test that [X]/[Done] click → close; remove `q`-as-close (update any saves/filebrowser key tests). Commit — "feat(saves,filebrowser): adopt dialog chrome".

---

### Task 6: Migrate verb menu + hotkey dialog (fixes command-panel bleed)

**Files:** `render/verbmenu.rs`, `render/hotkeys.rs`, `input.rs`, `main.rs`.

**Interfaces — Consumes:** as Task 5. **Produces:** both render via `draw_dialog` ([X]+[Done]); the opaque fill fixes the command-panel current-room bleed (#17); `q`-close removed; ESC==[X].

- [ ] **Step 1–5 (TDD):** render test that the hotkey dialog's bg is opaque over a pre-filled map cell (no bleed) and shows [X]; verb menu shows the chrome; mouse [X] closes. Commit — "feat(verbmenu,hotkeys): adopt dialog chrome; fix bleed".

---

### Task 7: Migrate corner overlays (room info, inspector, tidy panel)

**Files:** `render/room_info.rs`, `render/inspector.rs`, `render/tidy_panel.rs`, `input.rs`, `main.rs`.

**Interfaces — Consumes:** `draw_dialog` with `Placement::Positioned(their current rect)`, `show_close:true`, no buttons, no shadow. **Produces:** each keeps its corner position but gains the shared border + [X] + `dialog_*` colors; [X]/ESC closes.

- [ ] **Step 1–5 (TDD):** render test that each positioned overlay shows the shared border + [X] at its corner; mouse [X] click → close panel. Commit — "feat(overlays): adopt positioned dialog chrome + [X]".

---

### Task 8: Gallery redesign + uniform ESC==[X] sweep

**Files:** `render/gallery.rs`, `input.rs`, `main.rs`.

**Interfaces — Consumes:** `draw_dialog`. **Produces:** gallery becomes a CENTERED bordered dialog (not full-screen) via `draw_dialog` with title + [X]+[Done], content (the two-pane picker) into `content`; fixes its inconsistency + bleed + cursor. Final sweep: confirm EVERY modal closes identically on ESC and [X] (a table test over the modal set), and that no modal still binds `q` to close.

- [ ] **Step 1–5 (TDD):** render test that the gallery is centered + bordered (top-left `┌`/`┏`, not full-screen) with [X]/[Done]; a table test mapping ESC and [X] to the same close action for each modal; assert no `q`-close remains. Commit — "feat(gallery): centered dialog redesign + uniform ESC==[X]".

---

## Self-Review

**Spec coverage:**
- Shared `draw_dialog` (centered/opaque/border/[X]/buttons/shadow) + `centered_rect` + `DialogRects` → Task 1. ✅
- Dialog `style.toml` selectors + `dialog_box_style` + ColorScheme fields → Task 2. ✅
- Cursor suppress while overlay open → Task 3. ✅
- Mouse hit-testing (PaneRects.dialog + dialog-aware mouse_to_action) → Tasks 4–7. ✅
- Tiered buttons (config Save/Cancel; list/action [X]+[Done]; corner [X]-only) → Tasks 4,5,6,7. ✅
- Bleed fix (#17) → Tasks 1 (opaque) + 6 (hotkey dialog). ✅
- Gallery redesign (#19) → Task 8. ✅
- Uniform ESC==[X], q freed → Tasks 4–8 (sweep in 8). ✅
- Drop-shadow option → Tasks 1 (render) + 2 (style). ✅
- Reuse paneframe (no duplicate border drawing) → Global Constraints + Task 1. ✅

**Placeholder scan:** Phase-2 tasks (5–8) use a compact "Step 1–5 (TDD)" form with the concrete test + commit named, because they are structurally identical migrations of one component into one modal each; the per-modal render/mouse assertions are stated explicitly. No vague directives.

**Type consistency:** `centered_rect`, `Placement`, `ButtonId`, `DialogButton`, `DialogStyle`, `DialogSpec`, `DialogRects{area,content,close,buttons}`, `draw_dialog`, `any_overlay_open`, `PaneRects.dialog`, and the `ColorScheme` fields (`dialog, dialog_title, dialog_button, dialog_button_active, dialog_shadow, dialog_box_style`) are consistent across tasks. Reuses `paneframe::{draw_pane_frame, draw_top_inset, BorderStyle, parse_border_style}` from wave18.

## Notes for the executor
- Task 1 is the pure core (fully testable). Task 2 extends the #43 style system (mirror wave18's border-selector pattern). Tasks 4–8 are integration — read the current modal render + mouse plumbing before each.
- The mouse hit-test infra (`PaneRects.dialog` + dialog-aware `mouse_to_action`) is introduced in Task 4 and reused by 5–8; later tasks only add their modal's rects to the flow.
