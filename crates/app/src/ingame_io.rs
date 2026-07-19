//! In-game save/restore and `create_by_prompt` filename plumbing: the modal
//! open/resolve helpers that serve game-initiated SAVE/RESTORE and filename
//! requests (everything except the dialog RENDERING, which lives in render/).
//! Extracted verbatim from `main.rs` (SQ-0306) as a pure move — no behavior
//! change. `finish_resumed_turn` and the `combined_saves`/persistence helpers
//! stay in their homes and are reached via `crate::`.

use app::engine::Engine;
use app::persist_files::{delete_save, list_saves, save_game_named, save_named};
use app::state::{AppState, SavesState};
use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use crate::engine_helpers::{glulx_session_opt, zvm_session_opt};
use crate::{combined_saves, turn};

/// Resolve the confirm-delete dialog for the selected save. `confirmed` deletes
/// it (refreshing the open saves list); otherwise the save is kept. Byte-identical
/// to the retired y/n `handle_saves_prompt`. (SQ-0307)
pub(crate) fn delete_save_confirmed(
    path: &std::path::Path,
    confirmed: bool,
    dir: &std::path::Path,
    state: &mut AppState,
) {
    if confirmed {
        match delete_save(path) {
            Ok(()) => {
                state.push_notice("[Save deleted]");
                if let Some(s) = &mut state.overlays.saves {
                    s.entries = list_saves(dir);
                    // Re-clamp the selection/offset to the new entry count.
                    s.scroll.len(s.entries.len());
                }
            }
            Err(e) => {
                state.push_notice(&format!("[Delete failed: {}]", e));
            }
        }
    } else {
        state.push_notice("[Delete cancelled]");
    }
}

/// Handle a submitted save name (host "Save State" slot or in-game `@save`).
/// Called directly from the save-name dialog submit. On success it refreshes the
/// saves list; a host save also clears `unsaved_progress`, while an in-game save
/// sets `ingame_resume_save` so the run loop resumes the VM. An empty name or a
/// write error re-opens the dialog when in-game so the user can retry.
pub(crate) fn handle_save_as(
    buf: String,
    dir: &std::path::Path,
    ifid: &str,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    state: &mut AppState,
) {
    let ingame = state.ingame_io == Some(app::session::PendingIo::Save);
    if buf.is_empty() {
        state.push_notice("[Save name cannot be empty]".to_string().as_str());
        // In-game: stay pending — re-open the dialog so the user can retry.
        if ingame {
            state.overlays.save_name_dialog = Some(app::state::SaveNameDialog::new(
                app::persist_files::default_save_name(),
                true,
            ));
        }
        return;
    }
    let result = if ingame {
        // Game @save -> bare standard in-game save file (VM state only,
        // call-stub resume). The Z-machine writes standard descriptor-PC
        // Quetzal; Glulx writes `save_quetzal()` bytes (both land as
        // `<ifid>-<slug>.qzl` so the in-game restore picker lists them).
        match zvm_session_opt(&*session) {
            Some(z) => save_game_named(dir, &buf, &z.machine).map(|_| ()),
            None => {
                // Glulx writes its own Quetzal; other host-snapshot engines
                // (Scott, which has no game-native save format) write their VM
                // snapshot, exactly what `restore_game_save` feeds back on load.
                let bytes = match glulx_session_opt(&*session) {
                    Some(g) => g.save_quetzal(),
                    None => session.save_state().bytes,
                };
                app::persist_files::save_game_named_bytes(dir, &buf, &bytes).map(|_| ())
            }
        }
    } else {
        // Host "Save State" named slot -> rich .babelmap archive.
        let (location, score) = crate::engine_helpers::save_summary(&*session, state);
        save_named(dir, ifid, &buf, mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), state.turns, location, score, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para)
    };
    match result {
        Ok(()) => {
            state.push_notice(&format!("[Saved as: {}]", buf));
            // A host Save-State named slot captures the current progress
            // (an in-game @save writes a .qzl, a different mechanism).
            if !ingame {
                state.unsaved_progress = false;
            }
            // Refresh saves list.
            if let Some(s) = &mut state.overlays.saves {
                s.entries = list_saves(dir);
            }
            // In-game SAVE: flag-hop so the run loop resumes the VM
            // (resume + recenter need session/mapper/last_panes scope).
            if ingame {
                state.ingame_resume_save = Some(true);
            }
        }
        Err(e) => {
            state.push_notice(&format!("[Save failed: {}]", e));
            // In-game: stay pending — re-open the dialog so the user can retry.
            if ingame {
                state.overlays.save_name_dialog = Some(app::state::SaveNameDialog::new(
                    app::persist_files::default_save_name(),
                    true,
                ));
            }
        }
    }
}

/// Open the saves dialog in "in-game" mode for a game-initiated save/restore.
/// SAVE: prompt for a save name (reuses the save-name dialog). RESTORE: open the
/// saves list, including plain *.qzl files alongside *.babelmap saves.
pub(crate) fn open_ingame_saves(
    io: app::session::PendingIo,
    game_dir: &std::path::Path,
    state: &mut AppState,
) {
    use app::session::PendingIo;
    state.ingame_io = Some(io);
    state.overlays.dialog_focus = 0;
    match io {
        PendingIo::Save => {
            // The game asked to SAVE: ask where via the save-name dialog. On submit
            // -> resume_save(true); on cancel -> resume_save(false) (handled in the
            // cancel resolver, which now watches save_name_dialog).
            state.overlays.save_name_dialog = Some(app::state::SaveNameDialog::new(
                app::persist_files::default_save_name(),
                true,
            ));
        }
        PendingIo::Restore => {
            // The game asked to RESTORE: list babelmap saves + plain .qzl files.
            let entries = combined_saves(game_dir);
            state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
        }
    }
}

/// Resolve a pending in-game save/restore after the dialog interaction:
/// (1) a flag-hopped successful SAVE resumes the VM; (2) an in-game overlay that
/// closed without a confirm is treated as a cancel and resumes with failure.
/// Re-opens the dialog for a chained request. Returns true if the app should quit.
pub(crate) fn resolve_ingame_dialog(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    game_dir: &std::path::Path,
    ifid: &str,
    map_area: Rect,
) -> bool {
    use app::session::PendingIo;

    // (1) SAVE confirmed in handle_save_as (flag-hop): resume here.
    if let Some(wrote_ok) = state.ingame_resume_save.take() {
        state.ingame_io = None;
        let result = session.resume_save(wrote_ok);
        let quit = turn::finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
        if let Some(io) = state.ingame_io {
            open_ingame_saves(io, game_dir, state);
        }
        return quit;
    }

    // (2) Cancel: an in-game overlay closed without a confirm.
    if let Some(io) = state.ingame_io {
        let overlay_open = match io {
            PendingIo::Save => state.overlays.save_name_dialog.is_some(),
            PendingIo::Restore => state.overlays.saves.is_some(),
        };
        if !overlay_open {
            state.ingame_io = None;
            let result = match io {
                PendingIo::Save => session.resume_save(false),
                PendingIo::Restore => session.resume_restore(None),
            };
            state.push_notice("[In-game save/restore cancelled]");
            let quit = turn::finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
            if let Some(io) = state.ingame_io {
                open_ingame_saves(io, game_dir, state);
            }
            return quit;
        }
    }

    false
}

/// Open the right modal for a game `create_by_prompt` filename request: a name-entry
/// prompt (write / append / read-write), a file picker (read with existing files —
/// Task 5), or an immediate cancel (read with no files). Sets AppState; the resolver
/// later calls `resume_filename`.
pub(crate) fn open_filename_modal(req: app::session::FilenameReq, session: &dyn Engine, state: &mut AppState) {
    state.pending_filename = Some(req);
    match app::state::filename_modal_for(req, session.file_names().len()) {
        app::state::FilenameModal::NamePrompt => {
            state.overlays.dialog_focus = 0;
            state.overlays.text_entry =
                Some(app::state::TextEntryDialog::new(app::state::TextEntryKind::CreateFile, ""));
        }
        app::state::FilenameModal::Picker => {
            state.overlays.file_picker = Some(app::state::FilePickerState::new(session.file_names()));
        }
        app::state::FilenameModal::AutoCancel => {
            state.pending_filename = None;
            state.filename_submitted = Some(None);
        }
    }
}

/// Resume a suspended `create_by_prompt` once the player entered a name / cancelled
/// via the flag-hop (`state.filename_submitted`), or cancelled by closing the modal
/// (Esc leaves `pending_filename` set with no CreateFile prompt open). Mirrors
/// `resolve_ingame_dialog`. Returns true if the app should quit.
pub(crate) fn resolve_filename_request(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    game_dir: &std::path::Path,
    ifid: &str,
    map_area: Rect,
) -> bool {
    if let Some(choice) = state.filename_submitted.take() {
        state.pending_filename = None;
        let result = session.resume_filename(choice);
        let quit = turn::finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
        if let Some(io) = state.ingame_io {
            open_ingame_saves(io, game_dir, state);
        }
        return quit;
    }
    // Modal closed without a submit (Esc) while a request is still pending -> cancel.
    if state.pending_filename.is_some()
        && !matches!(&state.overlays.text_entry, Some(d) if d.kind == app::state::TextEntryKind::CreateFile)
        && state.overlays.file_picker.is_none()
    {
        state.pending_filename = None;
        let result = session.resume_filename(None);
        state.push_notice("[create_by_prompt cancelled]");
        let quit = turn::finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
        if let Some(io) = state.ingame_io {
            open_ingame_saves(io, game_dir, state);
        }
        return quit;
    }
    false
}
