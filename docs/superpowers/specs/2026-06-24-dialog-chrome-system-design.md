# Dialog Chrome System — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO items:** #48 (common configurable dialog styling + clickable OK/Cancel), #15 (clickable [X] on all dialogs/docks), #19 (gallery redesign), #17 (command-panel bleed), drop-shadow option, cursor-hide.
**Depends on:** #43 shareable style file (wave17) — dialog styling is expressed as new `style.toml` selectors. Implement AFTER #43 merges.
**Touches:** new `crates/app/src/render/dialog.rs`; all `crates/app/src/render/*` modal modules; `colors.rs`/`style.rs` (new dialog fields + selectors); `symbols.rs`/`config.rs` (`dialog_box_style`); `main.rs` (draw_frame rects + cursor guard); `input.rs` (mouse hit-test + uniform ESC close); `state.rs` (overlay-open helper). No `mapper`/`zvm` changes.

## Goal

Replace the 9 ad-hoc, hardcoded-color modals with ONE shared, themeable dialog chrome: a centered (or positioned), opaque, bordered, titled frame with a clickable close **[X]**, an optional bottom **button row**, and an optional **drop-shadow** — all styled from `style.toml`. Unify close behavior (**ESC == [X]**), fix the cursor/background bleed, and add mouse hit-testing for the X and buttons.

Current state (from investigation): modals hardcode colors (Black vs DarkGray, Cyan/Yellow borders), centering math is duplicated in 5 modules, only some have borders, there is no clickable-region infra beyond rooms, and the story cursor (`_`, drawn when `focus==Game`) can bleed through a modal.

## Component — `crates/app/src/render/dialog.rs` (new)

```rust
pub enum Placement { Centered { w: u16, h: u16 }, Positioned(Rect) }

pub struct DialogButton { pub id: ButtonId, pub label: &'static str }
// ButtonId is a small enum: Save, Cancel, Ok, Done, Close (extend as needed).

pub struct DialogSpec<'a> {
    pub title: &'a str,
    pub placement: Placement,
    pub buttons: &'a [DialogButton], // empty = no button row
    pub show_close: bool,            // top-right [X]
    pub shadow: bool,                // drop-shadow
}

pub struct DialogRects {
    pub area: Rect,                  // the dialog frame rect (for mouse "inside dialog?" tests)
    pub content: Rect,              // inner area the modal draws into
    pub close: Option<Rect>,        // [X] hit-rect
    pub buttons: Vec<(ButtonId, Rect)>,
}

pub fn centered_rect(area: Rect, w: u16, h: u16) -> Rect;
pub fn draw_dialog(buf: &mut Buffer, spec: &DialogSpec, colors: &ColorScheme, symbols: &SymbolSet) -> DialogRects;
```

`draw_dialog`:
1. Resolve `area` from `placement` (centered via `centered_rect`, else the given Rect).
2. If `spec.shadow`: paint a 1-cell offset shaded region (bottom edge + right edge, offset +1/+1) using the shadow style FIRST (so the frame sits on top). Skipped for `Positioned` corner overlays unless explicitly set.
3. Fill `area` with the OPAQUE dialog background (`Style::reset().bg(...)`) — this is the bleed fix; it also covers the help-bar row where the dialog overlaps.
4. Draw the border using the configured `dialog_box_style` glyphs + the dialog border color, and the title in the top border using the title style.
5. If `show_close`: draw `[X]` (or `✕`) at the top-right inside the border; record its `close` rect.
6. If `buttons` non-empty: draw a bottom button row (right-aligned), each button `[ Label ]` in the button style; record each `(id, rect)`. (A "focused"/hover button style exists for future keyboard/hover use.)
7. Return `DialogRects`. The caller renders its content inside `content`.

`centered_rect` replaces the duplicated centering in saves/filebrowser/config/hotkeys/verbmenu.

## Tiered button policy

- **Confirm/edit dialogs** — config screen, text-entry prompts: buttons `[Save]`/`[Cancel]` (or `[Ok]`/`[Cancel]` for prompts). `[Save]`→the modal's confirm action, `[Cancel]`/`[X]`/ESC→its cancel action.
- **List/action modals** — saves, gallery, file browser, verb menu, hotkey dialog: `show_close` [X] + a `[Done]` button; keep their existing action keys + footer hints. [X]/`[Done]`/ESC→close.
- **Read-only corner overlays** — room info, inspector, tidy panel: `Placement::Positioned` (keep their corner spot), `show_close` [X] only, no button row, no shadow. [X]/ESC→close panel.

## Uniform close — ESC == [X], `q` freed

Each modal declares ONE close action. **Both ESC and a click on [X] route to that exact action** — no per-modal divergence. **Remove `q`-as-close** from saves/filebrowser/verbmenu/hotkey-dialog; `q` is freed for reuse (decision deferred to #16 modifier-free keys). For edit/confirm dialogs the close action is Cancel/discard; for list/action modals it is plain close.

## Styling — new `style.toml` selectors + ColorScheme fields

New `ColorScheme` fields (replacing all hardcoded modal colors): `dialog` (bg + border fg), `dialog_title`, `dialog_button`, `dialog_button_active`, `dialog_shadow`.

New style-file selectors (per the #43 system; added to the fixed selector set, `DEFAULT_STYLE_TOML`, gallery/config, and `write_style_full`):
`dialog` (bg + border), `dialog:title`, `dialog:button`, `dialog:button:active`, `dialog:shadow`.

New symbol preset key `dialog_box_style` (a `BoxStyle` preset, reused from rooms' set: rounded/thick/double/ascii/borderless) so dialogs can differ from room boxes. Drop-shadow is configured by `dialog:shadow` (color) + a `dialog_shadow` on/off flag (off by default) + a 1-cell offset (fixed v1).

All 9 modal modules stop hardcoding colors and read `state.colors.dialog_*`; they call `draw_dialog` for their frame and render content into `content`.

## Mouse hit-testing

- `draw_frame` (main.rs) already returns `PaneRects { map, story, room_rects }`. Add `dialog_rects: Option<DialogRects>` — set when a modal/overlay is drawn (the modal's `draw_*` returns its `DialogRects`, or `draw_frame` calls `draw_dialog` centrally for it).
- `mouse_to_action` gains the `dialog_rects` param and checks it FIRST: a left-click on `close` → the active modal's close action; on a button → the mapped action; a click OUTSIDE `area` while a modal is open is swallowed (no map/room hit-through). Each modal supplies a small `ButtonId → Action` map (e.g. config: `Save→ConfigSave`, `Cancel→ConfigCancel`, close→`ConfigCancel`).

## Cursor + bleed fixes (#17, #19)

- **Cursor:** add `AppState::any_overlay_open()` (true if any modal/dialog/dock state is active) and guard the logical `_` cursor in `transcript.rs`: render it only when `focus==Game && !state.any_overlay_open()`. Fixes the gallery cursor bleed.
- **Bleed:** the chrome's full-`area` opaque `Style::reset().bg(...)` (step 3) guarantees no underlying bg/REVERSED bleeds — fixes the command-panel (#17) and gallery bleed.

## Gallery redesign (#19)

Gallery becomes a **centered bordered dialog** via the chrome (not full-screen): title bar, [X] + `[Done]`, opaque bg, its content (the two-pane preview/picker) rendered into `content`. Removes its Black-vs-DarkGray inconsistency and the bleed/cursor issues.

## Phasing (for the implementation plan)

- **Phase 1:** `dialog.rs` (centered_rect, draw_dialog, DialogRects), the new ColorScheme fields + `style.toml` dialog selectors + `dialog_box_style`, the cursor guard (`any_overlay_open`), and adopt the chrome in the **config screen** (proof: border/title/[Save]/[Cancel]/[X], mouse routing for its buttons). Drop-shadow rendering included but optional.
- **Phase 2:** migrate the remaining modals (saves, file browser, verb menu, hotkey dialog, room info, inspector, tidy panel) + the **gallery redesign**; wire `dialog_rects` mouse hit-testing for all; remove `q`-close; uniform ESC==[X].

## Testing

- `centered_rect` math (even/odd, clamps to area).
- `draw_dialog` (TestBackend): returns correct `close`/`button`/`content` rects; opaque bg covers a pre-filled REVERSED cell underneath; border uses the configured box style; shadow cells painted when `shadow=true` and absent when false.
- Mouse: `mouse_to_action` maps a click on `close` → close action, on a button → its action, and swallows outside-clicks while a modal is open.
- Cursor: with an overlay open, the `_` is not rendered even when `focus==Game`.
- Per migrated modal: a render test that the title, border, and [X] appear and colors come from `state.colors.dialog_*` (not hardcoded).
- Style: the new `dialog*` selectors round-trip through `style.toml` (parse/resolve/write_style_full) and appear in `DEFAULT_STYLE_TOML`.
- ESC and [X] produce the same close action for each modal (table test over the modal set).

## Out of scope / non-goals

- Keyboard button navigation (Tab between buttons) — buttons are a mouse affordance; existing keys unchanged.
- Re-theming individual modal CONTENT layouts beyond frame/title/buttons.
- Making corner overlays (room info/inspector/tidy) centered — they keep their position.
- Animated dialog open/close (the inventory/verb-menu slide-up docks are separate items #12/#14).
- `mapper`/`zvm` changes.

## Risks & limitations (accepted)

- **Broad migration:** 9 modules adopt the chrome — phased (config screen first) to de-risk; each migration is a small, independently testable change.
- **Mouse plumbing change:** adding `dialog_rects` threads one Option through `draw_frame`→`mouse_to_action`; mechanical, covered by tests.
- **Gallery is the hardest retrofit** (full-screen→dialog) — done last in Phase 2.
- **Shadow on small terminals:** the +1/+1 offset can clip at the screen edge; clamp the shadow to the buffer bounds (no panic).
