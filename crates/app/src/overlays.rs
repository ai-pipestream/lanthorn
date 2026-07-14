//! Modal/overlay draw dispatch: the z-ordered ladder of `if state.X { … }`
//! overlay draws that centre within the graphics-free dialog area. Extracted
//! verbatim from `draw_frame` in `main.rs` (SQ-0306) as a pure move — no
//! behaviour change. Each branch calls a render fn that already lives in
//! `app::render::*`; this module only owns the dispatch + the returned
//! hit-rects. (Wave 3 will turn the ladder into an `Overlay` trait.)

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use app::engine::ScreenModel;
use app::render::aux_dialog::{draw_aux_dialog, AuxDialogRects};
use app::render::config_screen::draw_config_screen;
use app::render::dialog::DialogRects;
use app::render::filebrowser::draw_file_browser;
use app::render::gallery::draw_gallery;
use app::render::glyph_picker::GlyphPickerRects;
use app::render::hints_panel::{draw_hints_panel, HintsPanelRects};
use app::render::history::draw_history;
use app::render::hotkeys::draw_hotkey_dialog;
use app::render::launch_dialog::{draw_launch_dialog, LaunchDialogRects};
use app::render::quit_dialog::{draw_quit_dialog, QuitDialogRects};
use app::render::reset_dialog::{draw_reset_dialog, ResetDialogRects};
use app::render::save_name_dialog::SaveNameDialogRects;
use app::render::text_entry_dialog::{draw_text_entry_dialog, TextEntryDialogRects};
use app::render::confirm_delete_dialog::{draw_confirm_delete_dialog, ConfirmDeleteDialogRects};
use app::render::saves::draw_saves;
use app::render::style_editor::{draw_style_editor, StyleEditorRects};
use app::render::file_picker::draw_file_picker;
use app::state::AppState;

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
    let mut dialog_rects_out: Option<DialogRects> = dialog_seed;
    let mut aux_dialog_rects_out: Option<AuxDialogRects> = None;
    let mut reset_dialog_rects_out: Option<ResetDialogRects> = None;
    let mut save_name_dialog_rects_out: Option<SaveNameDialogRects> = None;
    let mut text_entry_rects_out: Option<TextEntryDialogRects> = None;
    let mut confirm_delete_rects_out: Option<ConfirmDeleteDialogRects> = None;
    let mut quit_dialog_rects_out: Option<QuitDialogRects> = None;
    let mut launch_dialog_rects_out: Option<LaunchDialogRects> = None;
    let mut hints_panel_rects_out: Option<HintsPanelRects> = None;
    let mut style_editor_rects_out: Option<StyleEditorRects> = None;
    let mut glyph_picker_rects_out: Option<GlyphPickerRects> = None;

    // Modal dialogs center within the graphics-free text region (story text +
    // map together), never over a Glulx graphics window — the terminal image
    // protocol would otherwise overpaint them (SQ-0203). No graphics → `full`.
    // Clamp to gvm's content bounding box so the graphics-rect walk matches the
    // clamped composite render (the snap-margin has no windows). (SQ-0303)
    let story_bbox = app::render::screen::content_bounds(screen_model, story_area);
    let dialog_area = app::render::screen::dialog_bounds(screen_model, story_bbox, full);

    // ── Hotkey dialog overlay — drawn over everything ─────────────────────
    if state.hotkey_dialog {
        dialog_rects_out = draw_hotkey_dialog(state, dialog_area, buf);
    }

    // ── Gallery overlay — drawn after hotkey dialog ───────────────────────
    if state.gallery.is_some() {
        if let Some(dr) = draw_gallery(state, dialog_area, buf) {
            dialog_rects_out = Some(dr);
        }
    }

    // ── Saves-manager overlay — drawn after gallery ───────────────────────
    if state.saves.is_some() {
        dialog_rects_out = draw_saves(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Replay/rewind overlay ─────────────────────────────────────────────
    if state.replay.is_some() {
        dialog_rects_out = draw_history(state, dialog_area, buf, modal_list_viewport);
    }

    // ── File-browser overlay — drawn after saves ──────────────────────────
    if state.file_browser.is_some() {
        dialog_rects_out = draw_file_browser(state, dialog_area, buf, modal_list_viewport);
    }

    // ── VFS file picker overlay (read-mode create_by_prompt) ──────────────
    if state.file_picker.is_some() {
        dialog_rects_out = draw_file_picker(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Config screen overlay — drawn after other modals ──────────────────
    if state.config_screen.is_some() {
        dialog_rects_out = draw_config_screen(state, dialog_area, buf, modal_list_viewport);
    }

    // ── Style editor overlay — full-screen, drawn after config screen ──────
    if state.style_editor.is_some() {
        style_editor_rects_out = draw_style_editor(state, dialog_area, buf);
    }

    // ── Glyph-picker modal — drawn over the style editor ──────────────────
    if state.glyph_picker.is_some() {
        glyph_picker_rects_out = app::render::glyph_picker::draw_glyph_picker(state, dialog_area, buf);
    }

    // ── Aux-storage prompt — drawn over everything ────────────────────────
    if state.aux_prompt {
        aux_dialog_rects_out = draw_aux_dialog(state, dialog_area, buf);
    }

    // ── Reset dialog overlay — drawn over everything ───────────────────────
    if state.reset_dialog {
        reset_dialog_rects_out = draw_reset_dialog(state, dialog_area, buf);
    }

    // ── Save-name dialog overlay — drawn over everything ───────────────────
    if state.save_name_dialog.is_some() {
        save_name_dialog_rects_out =
            app::render::save_name_dialog::draw_save_name_dialog(state, dialog_area, buf);
    }

    // ── Text-entry dialog overlay — drawn over everything ──────────────────
    if state.text_entry.is_some() {
        text_entry_rects_out = draw_text_entry_dialog(state, dialog_area, buf);
    }

    // ── Confirm-delete dialog overlay — drawn over the saves manager ───────
    if state.confirm_delete_save.is_some() {
        confirm_delete_rects_out = draw_confirm_delete_dialog(state, dialog_area, buf);
    }

    // ── Quit dialog overlay — drawn over everything ────────────────────────
    if state.quit_dialog {
        quit_dialog_rects_out = draw_quit_dialog(state, dialog_area, buf);
    }

    // ── Launch dialog overlay — drawn over everything ──────────────────────
    if state.launch_dialog {
        launch_dialog_rects_out = draw_launch_dialog(state, dialog_area, buf);
    }

    // ── Hints panel overlay — drawn after other overlays ───────────────────
    if state.hints.is_some() {
        hints_panel_rects_out = draw_hints_panel(state, dialog_area, buf);
    }

    OverlayRects {
        dialog: dialog_rects_out,
        aux_dialog: aux_dialog_rects_out,
        reset_dialog: reset_dialog_rects_out,
        save_name_dialog: save_name_dialog_rects_out,
        text_entry: text_entry_rects_out,
        confirm_delete: confirm_delete_rects_out,
        quit_dialog: quit_dialog_rects_out,
        launch_dialog: launch_dialog_rects_out,
        hints_panel: hints_panel_rects_out,
        style_editor: style_editor_rects_out,
        glyph_picker: glyph_picker_rects_out,
    }
}
