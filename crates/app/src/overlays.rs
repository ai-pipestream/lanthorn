//! Modal/overlay dispatch: the z-ordered draw ladder over the graphics-free
//! dialog area, plus the [`Overlay`] trait that unifies the common-dialog
//! modals (aux / reset / save-name / text-entry / confirm-delete / quit /
//! launch) for both drawing and run-loop input decoding (SQ-0307). The richer
//! non-dialog modals (hotkey / gallery / saves / …) keep their bespoke draw
//! branches. Each branch/impl calls a render or key fn that already lives in
//! `app::render::*`; this module owns only the dispatch + the returned hit-rects.

use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use app::engine::ScreenModel;
use app::input::cycle_focus;
use app::render::aux_dialog::{aux_dialog_key_focused, draw_aux_dialog, AuxDialogAction, AuxDialogRects};
use app::render::config_screen::draw_config_screen;
use app::render::confirm_delete_dialog::{
    confirm_delete_key_focused, draw_confirm_delete_dialog, ConfirmDeleteAction, ConfirmDeleteDialogRects,
};
use app::render::dialog::DialogRects;
use app::render::filebrowser::draw_file_browser;
use app::render::file_picker::draw_file_picker;
use app::render::gallery::draw_gallery;
use app::render::glyph_picker::GlyphPickerRects;
use app::render::hints_panel::{draw_hints_panel, HintsPanelRects};
use app::render::history::draw_history;
use app::render::hotkeys::draw_hotkey_dialog;
use app::render::launch_dialog::{
    draw_launch_dialog, launch_dialog_key_focused, LaunchDialogAction, LaunchDialogRects,
};
use app::render::quit_dialog::{draw_quit_dialog, quit_dialog_key_focused, QuitDialogAction, QuitDialogRects};
use app::render::reset_dialog::{draw_reset_dialog, reset_dialog_key_focused, ResetDialogAction, ResetDialogRects};
use app::render::save_name_dialog::{draw_save_name_dialog, save_name_dialog_key, SaveNameAction, SaveNameDialogRects};
use app::render::saves::draw_saves;
use app::render::style_editor::{draw_style_editor, StyleEditorRects};
use app::render::text_entry_dialog::{
    draw_text_entry_dialog, text_entry_dialog_key, TextEntryAction, TextEntryDialogRects,
};
use app::state::{AppState, OverlayState};

use crate::PaneRects;

/// Per-overlay hit-rects produced by [`draw_all`]. These are the values the
/// dialog-area ladder used to write into `draw_frame` locals that end up in
/// `PaneRects`; `draw_frame` splices them back in unchanged.
pub(crate) struct OverlayRects {
    /// Last-drawn dialog chrome rects. Seeded by the pre-ladder tidy-panel /
    /// room-info / inspector draws in `draw_frame` and conditionally overwritten
    /// by the higher-z-order dialogs in the ladder.
    pub dialog: Option<DialogRects>,
    pub aux_dialog: Option<AuxDialogRects>,
    pub reset_dialog: Option<ResetDialogRects>,
    pub save_name_dialog: Option<SaveNameDialogRects>,
    pub text_entry: Option<TextEntryDialogRects>,
    pub confirm_delete: Option<ConfirmDeleteDialogRects>,
    pub quit_dialog: Option<QuitDialogRects>,
    pub launch_dialog: Option<LaunchDialogRects>,
    pub hints_panel: Option<HintsPanelRects>,
    pub style_editor: Option<StyleEditorRects>,
    pub glyph_picker: Option<GlyphPickerRects>,
}

/// Draw the z-ordered modal/overlay ladder over the current frame.
///
/// `dialog_seed` carries the `dialog` rect already set by `draw_frame`'s
/// pre-ladder map/story draws (tidy panel, room info, inspector); the ladder's
/// higher-z-order dialogs overwrite it when open. `modal_list_viewport` is the
/// shared list-modal viewport (also written by the verb dock earlier in the
/// frame), threaded by `&mut`. Branch order is z-order — preserved exactly.
pub(crate) fn draw_all(
    state: &AppState,
    screen_model: &ScreenModel,
    story_area: Rect,
    full: Rect,
    buf: &mut Buffer,
    dialog_seed: Option<DialogRects>,
    modal_list_viewport: &mut usize,
) -> OverlayRects {
    let mut out = OverlayRects {
        dialog: dialog_seed,
        aux_dialog: None,
        reset_dialog: None,
        save_name_dialog: None,
        text_entry: None,
        confirm_delete: None,
        quit_dialog: None,
        launch_dialog: None,
        hints_panel: None,
        style_editor: None,
        glyph_picker: None,
    };

    // Modal dialogs center within the graphics-free text region (story text +
    // map together), never over a Glulx graphics window — the terminal image
    // protocol would otherwise overpaint them (SQ-0203). No graphics → `full`.
    // Clamp to gvm's content bounding box so the graphics-rect walk matches the
    // clamped composite render (the snap-margin has no windows). (SQ-0303)
    let story_bbox = app::render::screen::content_bounds(screen_model, story_area);
    let dialog_area = app::render::screen::dialog_bounds(screen_model, story_bbox, full);

    // ── Richer non-dialog modals — z-ordered, not (yet) on the Overlay trait ──

    // ── Hotkey dialog overlay — drawn over everything ─────────────────────
    if state.overlays.hotkey_dialog {
        out.dialog = draw_hotkey_dialog(state, dialog_area, buf);
    }

    // ── Gallery overlay — drawn after hotkey dialog ───────────────────────
    if state.overlays.gallery.is_some() {
        if let Some(dr) = draw_gallery(state, dialog_area, buf) {
            out.dialog = Some(dr);
        }
    }

    // ── Saves-manager overlay — drawn after gallery ───────────────────────
    if state.overlays.saves.is_some() {
        out.dialog = draw_saves(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Replay/rewind overlay ─────────────────────────────────────────────
    if state.overlays.replay.is_some() {
        out.dialog = draw_history(state, dialog_area, buf, modal_list_viewport);
    }

    // ── File-browser overlay — drawn after saves ──────────────────────────
    if state.overlays.file_browser.is_some() {
        out.dialog = draw_file_browser(state, dialog_area, buf, modal_list_viewport);
    }

    // ── VFS file picker overlay (read-mode create_by_prompt) ──────────────
    if state.overlays.file_picker.is_some() {
        out.dialog = draw_file_picker(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Config screen overlay — drawn after other modals ──────────────────
    if state.overlays.config_screen.is_some() {
        out.dialog = draw_config_screen(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Style editor overlay — full-screen, drawn after config screen ──────
    if state.overlays.style_editor.is_some() {
        out.style_editor = draw_style_editor(state, dialog_area, buf);
    }

    // ── Glyph-picker modal — drawn over the style editor ──────────────────
    if state.overlays.glyph_picker.is_some() {
        out.glyph_picker = app::render::glyph_picker::draw_glyph_picker(state, dialog_area, buf);
    }

    // ── Common-dialog modals — drawn through the `Overlay` trait in the exact
    // z-order of the input ladder (aux → reset → save-name → text-entry →
    // confirm-delete → quit → launch). Each impl draws over `dialog_area` and
    // stashes its own hit-rects into `out`. (SQ-0307)
    for ov in COMMON_DIALOGS {
        if ov.is_open(&state.overlays) {
            ov.draw(state, dialog_area, buf, &mut out);
        }
    }

    // ── Hints panel overlay — drawn after the common dialogs ───────────────
    if state.overlays.hints.is_some() {
        out.hints_panel = draw_hints_panel(state, dialog_area, buf);
    }

    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Overlay trait — unifies the common-dialog modals (SQ-0307)
// ═══════════════════════════════════════════════════════════════════════════
//
// The aux / reset / save-name / text-entry / confirm-delete / quit / launch
// modals used to be seven copy-pasted `if state.<flag> { match &event { … }
// continue }` blocks in the run loop. They now share one decode+apply seam:
// [`topmost_common_dialog`] picks the highest-priority open overlay (the exact
// ladder order below), its [`Overlay`] impl decodes the event — applying pure
// focus / checkbox / text-field changes in place via `&mut AppState` — and
// returns an [`OverlayOutcome`]. Game-affecting side effects (reset, save,
// quit, resume, …) need the loop's `session` / `mapper` / path context, so they
// surface as an [`OverlayAct`] the run loop applies. Draw stays here too, routed
// through [`Overlay::draw`] by `draw_all`.
//
// The `Overlay` impls live in this dispatch module rather than the lib
// `render::*` modules (where the per-dialog key/hit-decode fns they delegate to
// live) because `mouse` hit-tests against [`PaneRects`], a bin-only type the lib
// cannot see. The lib key fns (`*_key_focused`, `*_dialog_key`) are unchanged.

/// A semantic command a common-dialog overlay decoded from an event, for the run
/// loop to apply with its `session` / `mapper` / path context in scope. Pure
/// state changes (focus, checkbox, text field) are applied inside the `Overlay`
/// methods and never surface here.
pub(crate) enum OverlayAct {
    AuxArchive,
    AuxGlobal,
    ResetConfirm,
    ResetCancel,
    SaveNameSubmit,
    SaveNameCancel,
    TextEntrySubmit,
    TextEntryCancel,
    ConfirmDelete(bool),
    QuitSave,
    QuitQuit,
    QuitCancel,
    LaunchResume,
    LaunchNewGame,
}

/// The result of routing one event to the top-most open common-dialog overlay.
pub(crate) enum OverlayOutcome {
    /// Fully handled inside the overlay (focus moved, checkbox toggled, field
    /// edited, click swallowed) — the run loop does nothing further.
    Consumed,
    /// The overlay decoded a game-affecting command for the run loop to apply.
    Act(OverlayAct),
}

/// Identity of a common-dialog overlay — used by the priority-order unit test.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[allow(dead_code)] // introspection hook exercised only by the ladder-order test
pub(crate) enum OverlayKind {
    Aux,
    Reset,
    SaveName,
    TextEntry,
    ConfirmDelete,
    Quit,
    Launch,
}

pub(crate) trait Overlay {
    /// This overlay's identity (for the priority-order test).
    #[allow(dead_code)] // exercised only by the ladder-order test
    fn kind(&self) -> OverlayKind;
    /// Is this overlay currently open? Priority is the [`COMMON_DIALOGS`] order.
    fn is_open(&self, ov: &OverlayState) -> bool;
    /// Draw over `area` (the graphics-free dialog region), stashing hit-rects in `out`.
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects);
    /// Decode a key press, applying pure state changes in place.
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome;
    /// Decode a mouse event, applying pure state changes in place.
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome;
}

/// The common-dialog overlays in strict input-priority / draw z-order (highest
/// priority first): aux ▸ reset ▸ save-name ▸ text-entry ▸ confirm-delete ▸ quit
/// ▸ launch. This is the exact order of the old run-loop `if`-ladder.
pub(crate) const COMMON_DIALOGS: &[&dyn Overlay] = &[
    &AuxOverlay,
    &ResetOverlay,
    &SaveNameOverlay,
    &TextEntryOverlay,
    &ConfirmDeleteOverlay,
    &QuitOverlay,
    &LaunchOverlay,
];

/// The highest-priority open common-dialog overlay, or `None`. Pure over
/// [`OverlayState`]; the run loop routes events to it and `draw_all` draws it.
pub(crate) fn topmost_common_dialog(ov: &OverlayState) -> Option<&'static dyn Overlay> {
    COMMON_DIALOGS.iter().copied().find(|o| o.is_open(ov))
}

/// Left-button-down screen position, or `None` for any other mouse event.
fn left_down(m: &MouseEvent) -> Option<Position> {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Position { x: m.column, y: m.row }),
        _ => None,
    }
}

/// True for a Ctrl/Alt/Super-modified printable char (an accelerator, not text).
fn is_ctrl_char(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char(_))
        && key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
}

// ── Aux-storage prompt ─────────────────────────────────────────────────────
struct AuxOverlay;
impl Overlay for AuxOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Aux }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.aux_prompt }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.aux_dialog = draw_aux_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match aux_dialog_key_focused(code, state.overlays.dialog_focus) {
                AuxDialogAction::Archive => OverlayOutcome::Act(OverlayAct::AuxArchive),
                AuxDialogAction::Global => OverlayOutcome::Act(OverlayAct::AuxGlobal),
                AuxDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(ad) = &panes.aux_dialog else { return OverlayOutcome::Consumed };
        let in_close = ad.close.is_some_and(|r| r.contains(pt));
        let in_archive = ad.archive.is_some_and(|r| r.contains(pt));
        let in_global = ad.global.is_some_and(|r| r.contains(pt));
        let in_dialog = ad.area.contains(pt);
        if in_close || in_archive || (!in_global && !in_dialog) {
            OverlayOutcome::Act(OverlayAct::AuxArchive)
        } else if in_global {
            OverlayOutcome::Act(OverlayAct::AuxGlobal)
        } else {
            OverlayOutcome::Consumed
        }
    }
}

// ── Reset dialog ───────────────────────────────────────────────────────────
struct ResetOverlay;
impl Overlay for ResetOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Reset }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.reset_dialog }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.reset_dialog = draw_reset_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 4, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 4, -1);
                OverlayOutcome::Consumed
            }
            code => match reset_dialog_key_focused(code, state.overlays.dialog_focus) {
                ResetDialogAction::Confirm => OverlayOutcome::Act(OverlayAct::ResetConfirm),
                ResetDialogAction::Cancel => OverlayOutcome::Act(OverlayAct::ResetCancel),
                ResetDialogAction::ToggleClearMap => {
                    state.overlays.reset_clear_map = !state.overlays.reset_clear_map;
                    OverlayOutcome::Consumed
                }
                ResetDialogAction::ToggleDeleteData => {
                    state.overlays.reset_delete_data = !state.overlays.reset_delete_data;
                    OverlayOutcome::Consumed
                }
                ResetDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(rd) = &panes.reset_dialog else { return OverlayOutcome::Consumed };
        // Check buttons and close in order: close > reset > cancel > checkbox.
        let in_close = rd.close.is_some_and(|r| r.contains(pt));
        let in_reset = rd.reset.is_some_and(|r| r.contains(pt));
        let in_cancel = rd.cancel.is_some_and(|r| r.contains(pt));
        let in_checkbox = rd.checkbox.contains(pt);
        let in_checkbox_data = rd.checkbox_data.contains(pt);
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::ResetCancel)
        } else if in_reset {
            OverlayOutcome::Act(OverlayAct::ResetConfirm)
        } else if in_checkbox {
            state.overlays.reset_clear_map = !state.overlays.reset_clear_map;
            OverlayOutcome::Consumed
        } else if in_checkbox_data {
            state.overlays.reset_delete_data = !state.overlays.reset_delete_data;
            OverlayOutcome::Consumed
        } else {
            // Click outside the dialog (or its interior): swallow, keep it open.
            OverlayOutcome::Consumed
        }
    }
}

// ── Save-name dialog (caret text field) ────────────────────────────────────
struct SaveNameOverlay;
impl Overlay for SaveNameOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::SaveName }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.save_name_dialog.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.save_name_dialog = draw_save_name_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        if is_ctrl_char(key) {
            return OverlayOutcome::Consumed;
        }
        let focus = state.overlays.dialog_focus;
        let dlg = state.overlays.save_name_dialog.as_mut().unwrap();
        let (act, new_focus) = save_name_dialog_key(key.code, dlg, focus);
        state.overlays.dialog_focus = new_focus;
        match act {
            SaveNameAction::Save => OverlayOutcome::Act(OverlayAct::SaveNameSubmit),
            SaveNameAction::Cancel => OverlayOutcome::Act(OverlayAct::SaveNameCancel),
            SaveNameAction::None => OverlayOutcome::Consumed,
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(sd) = &panes.save_name_dialog else { return OverlayOutcome::Consumed };
        let in_close = sd.close.is_some_and(|r| r.contains(pt));
        let in_save = sd.save.is_some_and(|r| r.contains(pt));
        let in_cancel = sd.cancel.is_some_and(|r| r.contains(pt));
        let in_field = sd.field.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::SaveNameCancel)
        } else if in_save {
            OverlayOutcome::Act(OverlayAct::SaveNameSubmit)
        } else if in_field {
            // Focus + activate the field (caret to end).
            state.overlays.dialog_focus = 0;
            if let Some(dlg) = state.overlays.save_name_dialog.as_mut() {
                dlg.active = true;
                dlg.field.end();
            }
            OverlayOutcome::Consumed
        } else {
            // Click outside: swallow, keep the dialog open.
            OverlayOutcome::Consumed
        }
    }
}

// ── Text-entry dialog (generic single-field) ───────────────────────────────
struct TextEntryOverlay;
impl Overlay for TextEntryOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::TextEntry }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.text_entry.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.text_entry = draw_text_entry_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        if is_ctrl_char(key) {
            return OverlayOutcome::Consumed;
        }
        let focus = state.overlays.dialog_focus;
        let dlg = state.overlays.text_entry.as_mut().unwrap();
        let (act, new_focus) = text_entry_dialog_key(key.code, &mut dlg.field, focus);
        state.overlays.dialog_focus = new_focus;
        match act {
            TextEntryAction::Submit => OverlayOutcome::Act(OverlayAct::TextEntrySubmit),
            TextEntryAction::Cancel => OverlayOutcome::Act(OverlayAct::TextEntryCancel),
            TextEntryAction::None => OverlayOutcome::Consumed,
        }
    }
    fn mouse(&self, state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(td) = &panes.text_entry else { return OverlayOutcome::Consumed };
        let in_close = td.close.is_some_and(|r| r.contains(pt));
        let in_ok = td.ok.is_some_and(|r| r.contains(pt));
        let in_cancel = td.cancel.is_some_and(|r| r.contains(pt));
        let in_field = td.field.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::TextEntryCancel)
        } else if in_ok {
            OverlayOutcome::Act(OverlayAct::TextEntrySubmit)
        } else if in_field {
            // Focus the field (caret to end).
            state.overlays.dialog_focus = 0;
            if let Some(dlg) = state.overlays.text_entry.as_mut() {
                dlg.field.end();
            }
            OverlayOutcome::Consumed
        } else {
            // Click outside: swallow, keep the dialog open.
            OverlayOutcome::Consumed
        }
    }
}

// ── Confirm-delete (two-button, over the saves manager) ────────────────────
struct ConfirmDeleteOverlay;
impl Overlay for ConfirmDeleteOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::ConfirmDelete }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.confirm_delete_save.is_some() }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.confirm_delete = draw_confirm_delete_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match confirm_delete_key_focused(code, state.overlays.dialog_focus) {
                ConfirmDeleteAction::Confirm => OverlayOutcome::Act(OverlayAct::ConfirmDelete(true)),
                ConfirmDeleteAction::Cancel => OverlayOutcome::Act(OverlayAct::ConfirmDelete(false)),
                ConfirmDeleteAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(cd) = &panes.confirm_delete else { return OverlayOutcome::Consumed };
        let in_close = cd.close.is_some_and(|r| r.contains(pt));
        let in_delete = cd.delete.is_some_and(|r| r.contains(pt));
        let in_cancel = cd.cancel.is_some_and(|r| r.contains(pt));
        if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::ConfirmDelete(false))
        } else if in_delete {
            OverlayOutcome::Act(OverlayAct::ConfirmDelete(true))
        } else {
            OverlayOutcome::Consumed
        }
    }
}

// ── Quit dialog ────────────────────────────────────────────────────────────
struct QuitOverlay;
impl Overlay for QuitOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Quit }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.quit_dialog }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.quit_dialog = draw_quit_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 3, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 3, -1);
                OverlayOutcome::Consumed
            }
            code => match quit_dialog_key_focused(code, state.overlays.dialog_focus) {
                QuitDialogAction::Save => OverlayOutcome::Act(OverlayAct::QuitSave),
                QuitDialogAction::Quit => OverlayOutcome::Act(OverlayAct::QuitQuit),
                QuitDialogAction::Cancel => OverlayOutcome::Act(OverlayAct::QuitCancel),
                QuitDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(qd) = &panes.quit_dialog else { return OverlayOutcome::Consumed };
        let in_close = qd.close.is_some_and(|r| r.contains(pt));
        let in_save = qd.save.is_some_and(|r| r.contains(pt));
        let in_quit = qd.quit.is_some_and(|r| r.contains(pt));
        let in_cancel = qd.cancel.is_some_and(|r| r.contains(pt));
        if in_save {
            OverlayOutcome::Act(OverlayAct::QuitSave)
        } else if in_quit {
            OverlayOutcome::Act(OverlayAct::QuitQuit)
        } else if in_close || in_cancel {
            OverlayOutcome::Act(OverlayAct::QuitCancel)
        } else {
            // Click outside: swallow (keep dialog open).
            OverlayOutcome::Consumed
        }
    }
}

// ── Launch dialog (resume saved game at startup) ───────────────────────────
struct LaunchOverlay;
impl Overlay for LaunchOverlay {
    fn kind(&self) -> OverlayKind { OverlayKind::Launch }
    fn is_open(&self, ov: &OverlayState) -> bool { ov.launch_dialog }
    fn draw(&self, state: &AppState, area: Rect, buf: &mut Buffer, out: &mut OverlayRects) {
        out.launch_dialog = draw_launch_dialog(state, area, buf);
    }
    fn key(&self, state: &mut AppState, key: &KeyEvent) -> OverlayOutcome {
        match key.code {
            KeyCode::Tab | KeyCode::Right => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, 1);
                OverlayOutcome::Consumed
            }
            KeyCode::BackTab | KeyCode::Left => {
                state.overlays.dialog_focus = cycle_focus(state.overlays.dialog_focus, 2, -1);
                OverlayOutcome::Consumed
            }
            code => match launch_dialog_key_focused(code, state.overlays.dialog_focus) {
                LaunchDialogAction::Resume => OverlayOutcome::Act(OverlayAct::LaunchResume),
                LaunchDialogAction::NewGame => OverlayOutcome::Act(OverlayAct::LaunchNewGame),
                LaunchDialogAction::None => OverlayOutcome::Consumed,
            },
        }
    }
    fn mouse(&self, _state: &mut AppState, m: &MouseEvent, panes: &PaneRects) -> OverlayOutcome {
        let Some(pt) = left_down(m) else { return OverlayOutcome::Consumed };
        let Some(ld) = &panes.launch_dialog else { return OverlayOutcome::Consumed };
        let in_resume = ld.resume.is_some_and(|r| r.contains(pt));
        let in_new_game = ld.new_game.is_some_and(|r| r.contains(pt));
        let in_close = ld.close.is_some_and(|r| r.contains(pt));
        if in_resume {
            OverlayOutcome::Act(OverlayAct::LaunchResume)
        } else if in_new_game || in_close {
            // [X] (close) and [New game] both discard the save.
            OverlayOutcome::Act(OverlayAct::LaunchNewGame)
        } else {
            // Click outside: swallow (keep dialog open).
            OverlayOutcome::Consumed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The common-dialog priority ladder resolves to the exact z-order the old
    /// run-loop if-ladder used: aux ▸ reset ▸ save-name ▸ text-entry ▸
    /// confirm-delete ▸ quit ▸ launch. `topmost_common_dialog` must return the
    /// highest-priority open overlay regardless of which lower ones are also open.
    #[test]
    fn topmost_common_dialog_preserves_ladder_order() {
        // No overlay open → None.
        let ov = OverlayState::default();
        assert!(topmost_common_dialog(&ov).is_none());

        // Each overlay alone resolves to itself, in ladder order.
        #[allow(clippy::type_complexity)]
        let cases: &[(fn(&mut OverlayState), OverlayKind)] = &[
            (|o| o.aux_prompt = true, OverlayKind::Aux),
            (|o| o.reset_dialog = true, OverlayKind::Reset),
            (|o| o.save_name_dialog = Some(app::state::SaveNameDialog::new(String::new(), false)), OverlayKind::SaveName),
            (|o| o.text_entry = Some(app::state::TextEntryDialog::new(app::state::TextEntryKind::CreateFile, "")), OverlayKind::TextEntry),
            (|o| o.confirm_delete_save = Some(std::path::PathBuf::from("s.sav")), OverlayKind::ConfirmDelete),
            (|o| o.quit_dialog = true, OverlayKind::Quit),
            (|o| o.launch_dialog = true, OverlayKind::Launch),
        ];
        for (open, want) in cases {
            let mut o = OverlayState::default();
            open(&mut o);
            assert_eq!(topmost_common_dialog(&o).unwrap().kind(), *want);
        }

        // With several open at once, the highest-priority (earliest) wins.
        let mut o = OverlayState::default();
        o.launch_dialog = true;
        o.quit_dialog = true;
        o.reset_dialog = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Reset);

        // Drop reset → quit is now top-most (still above launch).
        o.reset_dialog = false;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Quit);

        // Aux sits above everything.
        o.aux_prompt = true;
        assert_eq!(topmost_common_dialog(&o).unwrap().kind(), OverlayKind::Aux);
    }
}
