# Dialog Standardization Design

**Status:** Draft for review
**Date:** 2026-06-24

## Goal

Give every modal overlay a consistent keyboard model: a **default (confirm)
button** that is **underlined** and **starts focused** so **Enter** triggers it;
**Tab / Shift-Tab** cycle focus through the modal's buttons (the focused button is
highlighted); **Esc** always cancels/closes; mouse clicks and existing letter
accelerators are unchanged.

This applies to **all twelve overlays**. Text-input modals reconcile the rule by
treating "submit the input" as their confirm action (Enter already does this), so
nothing they do today breaks.

## Behavior model

1. **Default button (confirm).** Each modal designates one button as its confirm
   action (OK / Save / a semantic equivalent like Resume or Reset). It is rendered
   **underlined** at all times so the Enter-default is discoverable, and focus
   starts on it.
2. **Enter** activates the **currently focused** button. Because focus starts on
   the confirm button, Enter confirms by default. After Tab moves focus, Enter
   activates whatever is focused.
3. **Tab / Shift-Tab** move focus forward / backward through the modal's button
   ring (wrapping). The focused button renders with `button_active`; all others
   with `button`. The default button additionally carries an underline.
4. **Esc** always cancels/closes the modal (unchanged from today). The `✕` close
   glyph remains clickable and equivalent to Esc; it is **not** part of the Tab
   ring.
5. **Mouse** clicks on any button work exactly as today.
6. **Letter accelerators** that exist today (`r`/`c`/`s`/`q`/`n`) are retained as
   direct shortcuts.

### Text-input modals (prompt, hints, config path edit)

These own a text field where **Enter already submits** — that submit *is* the
confirm action. The field keeps focus by default; **Enter submits**, **Esc
cancels**. Tab moves focus from the field to the button row (where present) and
back. No existing typing behavior changes.

## Shared mechanism

- **`render/dialog.rs`** — `DialogSpec` gains two optional fields:
  - `default: Option<ButtonId>` — the confirm button (rendered underlined).
  - `focus: Option<usize>` — index into `buttons` to highlight with `button_active`.
  `draw_dialog` underlines the default button's label (via `Modifier::UNDERLINED`
  on `DialogStyle.button` for that one button) and highlights the focused one.
- **Focus state** — each modal that has more than one button stores a
  `focus_idx: usize` in its overlay state (or a single shared
  `AppState.dialog_focus: usize` reset whenever a modal opens). A small helper
  `cycle_focus(idx, len, delta) -> usize` lives in `input.rs` and is unit-tested
  once.
- **Per-modal key handlers** (listed below) gain: Tab/Shift-Tab → `cycle_focus`;
  Enter → activate `buttons[focus_idx]`. The existing Esc and accelerator arms
  stay.

## Per-modal table (target behavior)

| Overlay | Buttons (Tab ring) | Default (underlined, Enter) | Esc | Notes / change | Handler |
|---|---|---|---|---|---|
| gallery | `OK` | OK = persist selection to config + close | close (keep live session changes) | replace `Done` with `OK`; Enter currently just closes | `input.rs` `gallery_key_to_action` |
| saves | `Load`, `Save as`, `Delete`, `Done` | Load selected slot | close | promote the row actions into a Tab ring; today they are letter/loose keys | `input.rs` `saves_key_to_action` |
| file_browser | `Select`, `Cancel` | Select = current entry (file → choose; dir → open) | close | Enter still opens dirs / selects files; add Tab-reachable `Cancel` | `input.rs` `filebrowser_key_to_action` |
| config_screen | `Save`, `Cancel` | Save | cancel | already has Save/Cancel; add focus + underline | `input.rs` `config_screen_key_to_action` |
| verb_menu | `OK`, `Done` | OK = apply built command | close | Enter/Space still pick tokens; OK applies | `input.rs` `verb_menu_key_to_action` |
| hotkey_dialog | `Done` | Done = close | close | read-only; single button, Enter closes | `input.rs` `hotkey_dialog_key_to_action` |
| room_panel | `OK` | OK = close | close | add a centered OK (= close); today `✕` only | `input.rs:312` |
| tidy_anim | `OK` | OK = close | close | add a centered OK (= close); Space still toggles play | `input.rs:263` |
| prompt | *(see decision)* | Enter = submit field | cancel | one-line editor — see open decision below | `input.rs` `prompt_key_to_action` |
| reset_dialog | `Reset`, `Cancel` | Reset | cancel | already semantic buttons; add focus + underline | `main.rs` `reset_dialog_key` |
| quit_dialog | `Save & quit`, `Quit`, `Cancel` | Save & quit | cancel | add focus + underline | `main.rs` `quit_dialog_key` |
| launch_dialog | `Resume`, `New game` | Resume | new game (Esc) | add focus + underline | `main.rs` `launch_dialog_key` |
| hints | `Close` | Enter = submit hint input | close | text input; Close is the only button | `main.rs` `hint_key_routes` |

## Resolved decision

**Text prompts** (rename room, edit notes, relabel, rename layer, save-as,
export-save-name, confirm-delete) stay **minimal** — a one-line field with `Enter`
submit and `Esc` cancel, no button row (**Option A**). A one-line editor's
confirm-on-Enter is universal; an OK/Cancel row on a single field is visual noise.
`ConfirmDeleteSave` follows the same rule: `Enter` confirms the delete, `Esc`
cancels. Prompts are therefore the deliberate exception to the "every modal has a
button row" rule.

## Error handling

No new failure modes. Focus index is always clamped to `0..buttons.len()`; opening
a modal resets focus to the default button's index.

## Testing

- `cycle_focus` helper: forward/backward wrap unit tests.
- `draw_dialog`: renders underline on the default button; renders `button_active`
  on the focused button; no underline when `default` is `None`.
- Per-modal key tests (one each): Tab advances focus; Enter on default fires the
  confirm action; Enter after Tab fires the focused action; Esc still cancels.
- Regression: existing accelerator keys (`r`/`c`/`s`/`q`/`n`) still work.

## Out of scope

- Restyling button visuals beyond focus highlight + default underline.
- Changing what each confirm action *does* (only how it is triggered/shown).
- Adding new buttons to panels beyond the single OK/close where noted.
