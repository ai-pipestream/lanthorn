//! In-game save/restore and `create_by_prompt` filename plumbing: the modal
//! open/resolve helpers that serve game-initiated SAVE/RESTORE and filename
//! requests (everything except the dialog RENDERING, which lives in render/).
//! Extracted verbatim from `main.rs` (SQ-0306) as a pure move — no behavior
//! change. `finish_resumed_turn` and the `combined_saves`/persistence helpers
//! stay in their homes and are reached via `crate::`.

use app::archive::SaveTrigger;
use app::engine::Engine;
use app::persist_files::{delete_save, list_saves, save_named};
use app::state::{AppState, SavesState};
use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use crate::engine_helpers::zvm_session_opt;
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
/// saves list and clears `unsaved_progress` (both triggers capture the same
/// archive); an in-game save also sets `ingame_resume_save` so the run loop
/// resumes the VM. An empty name or a write error re-opens the dialog when
/// in-game so the user can retry.
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
    // ONE writer for both triggers (SQ-0531): an in-game `@save` and a host
    // "Save State" named slot both produce the same `.babelmap` archive — map,
    // transcript (art included), screen, aux and all — so an in-game save is no
    // longer a lesser save. What differs is the game bytes riding inside, and
    // `Meta::trigger` is what records which convention they follow.
    let trigger = if ingame { SaveTrigger::Ingame } else { SaveTrigger::HostState };
    let save = app::persist_files::game_save_bytes(&*session, trigger);
    let (location, score) = crate::engine_helpers::save_summary(&*session, state);
    // SQ-0588: the display list travels with every host save, not just the
    // auto-save paths — an archive written without it restores art that can never
    // be recoloured.
    let (v6_pics, v6_display, v6_diags) = crate::engine_helpers::v6_save_payload(&mut *session);
    for d in &v6_diags { state.note_v6_save(d); }
    let result = save_named(dir, ifid, &buf, trigger, mapper, &save, zvm_session_opt(&*session).map(|z| &z.machine.screen), &v6_pics, v6_display.as_ref(), session.aux_data(), state.turns, location, score, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para, &state.transcript_images);
    match result {
        Ok(()) => {
            state.push_notice(&format!("[Saved as: {}]", buf));
            // Either trigger captures the current progress: an in-game @save
            // now writes the same archive with the same session content, so
            // nagging about unsaved work the player just saved would be a lie.
            state.unsaved_progress = false;
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use app::archive::SaveTrigger;
    use app::engine::Engine;
    use app::session::{GameSession, PendingIo};
    use mapper::direction::Direction;
    use mapper::mapper::Mapper;

    const IFID: &str = "ZCODE-1-TEST00-0531";

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "babelmap-sq0531-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fresh minizork session, parked at its opening prompt.
    fn minizork() -> GameSession {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        let story = std::fs::read(&fixture).expect("minizork.z3 fixture");
        GameSession::new(story, true, false, None).expect("new minizork")
    }

    /// App state carrying a recognisable session: transcript lines and the turn
    /// counter an archive is supposed to bring back.
    fn state_with_session(io: Option<PendingIo>) -> app::state::AppState {
        let mut s = app::state::AppState::default();
        s.ingame_io = io;
        s.unsaved_progress = true;
        s.turns = 11;
        s.push_transcript_internal("West of House", app::state::TranscriptKind::Story);
        s.push_transcript_internal(
            "Opening the small mailbox reveals a leaflet.",
            app::state::TranscriptKind::Story,
        );
        s
    }

    fn mapper_with_room() -> Mapper {
        let mut m = Mapper::default();
        m.observe(1, "West of House", None);
        m.observe(2, "North of House", Some(Direction::N));
        m
    }

    /// Drive minizork to its `@save` suspension, so `save_quetzal()` is
    /// descriptor-PC — exactly the state the host writes a game save from.
    fn at_ingame_save() -> GameSession {
        let mut sess = minizork();
        assert!(!sess.submit("open mailbox").quit);
        let r = sess.submit("save");
        assert_eq!(r.pending_io, Some(PendingIo::Save), "'save' reaches @save");
        sess
    }

    // ── The unified writer ───────────────────────────────────────────────────

    #[test]
    fn ingame_save_writes_a_full_archive_whose_game_bytes_stay_interchange_grade() {
        // The heart of SQ-0531: `@save` no longer writes a bare `.qzl`. It writes
        // the SAME `.babelmap` a host Save State writes — map, transcript, screen
        // — while the bytes sealed inside stay byte-identical to
        // `Machine::save_quetzal()`, so unzipping `game.qzl` still hands another
        // interpreter a standard Quetzal save.
        let dir = temp_dir("ingame-write");
        let mut sess = at_ingame_save();
        let expected = sess.machine.save_quetzal();
        let mut state = state_with_session(Some(PendingIo::Save));
        let mut mapper = mapper_with_room();

        super::handle_save_as("chapter one".into(), &dir, IFID, &mut mapper, &mut sess, &mut state);

        let path = dir.join("chapter-one.babelmap");
        assert!(path.exists(), "@save writes the archive");
        assert!(!dir.join("chapter-one.qzl").exists(), "and no bare .qzl beside it");

        let meta = app::archive::read_archive_meta(&path).expect("meta");
        assert_eq!(meta.trigger, SaveTrigger::Ingame, "the trigger round-trips through meta.json");
        assert!(meta.trigger.is_portable(), "an @save archive is advertised as portable");
        assert_eq!(meta.turns, 11);

        assert_eq!(
            app::archive::read_quetzal_from_file(&path).expect("inner bytes"),
            expected,
            "game.qzl is byte-identical to machine.save_quetzal()"
        );

        // The whole session rode along — this is what the old bare .qzl could not do.
        let ac = app::archive::load_archive(&path).expect("load_archive");
        assert_eq!(ac.transcript, state.transcript, "the scrollback is in the archive");
        assert_eq!(ac.mapper.graph.rooms().count(), mapper.graph.rooms().count(), "so is the map");
        assert!(ac.screen.is_some(), "so is the Z-machine screen");

        // Surrounding in-game behaviour is untouched: the flag-hop that resumes
        // the VM fires, and the dialog is not re-opened.
        assert_eq!(state.ingame_resume_save, Some(true));
        assert!(state.overlays.save_name_dialog.is_none());
        // An in-game @save writes the same archive as a host Save State, so it
        // clears the unsaved-work flag too (SQ-0531) — no quit-time nag about
        // progress the player just saved in full.
        assert!(!state.unsaved_progress, "in-game @save captures progress");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_save_state_writes_the_same_archive_with_the_hoststate_trigger() {
        let dir = temp_dir("host-write");
        let mut sess = minizork();
        let expected = sess.save_state().bytes;
        let mut state = state_with_session(None); // not in-game -> host Save State
        let mut mapper = mapper_with_room();

        super::handle_save_as("chapter one".into(), &dir, IFID, &mut mapper, &mut sess, &mut state);

        let path = dir.join("chapter-one.babelmap");
        let meta = app::archive::read_archive_meta(&path).expect("meta");
        assert_eq!(meta.trigger, SaveTrigger::HostState);
        assert!(!meta.trigger.is_portable(), "a between-turns snapshot is NOT advertised as portable");
        assert_eq!(app::archive::read_quetzal_from_file(&path).unwrap(), expected);

        // Host-save bookkeeping is unchanged by the unification.
        assert!(!state.unsaved_progress, "a host Save State captures progress");
        assert_eq!(state.ingame_resume_save, None, "no suspended VM to resume");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_name_and_write_errors_still_reopen_the_dialog_in_game() {
        // Format unification must not disturb the retry loop.
        let dir = temp_dir("retry");
        let mut sess = at_ingame_save();
        let mut state = state_with_session(Some(PendingIo::Save));
        let mut mapper = Mapper::default();

        super::handle_save_as(String::new(), &dir, IFID, &mut mapper, &mut sess, &mut state);
        assert!(state.overlays.save_name_dialog.is_some(), "empty name re-opens the dialog");
        assert_eq!(state.ingame_resume_save, None, "and does not resume the VM");

        // "default" is the reserved slug -> the writer errors -> retry.
        state.overlays.save_name_dialog = None;
        super::handle_save_as("default".into(), &dir, IFID, &mut mapper, &mut sess, &mut state);
        assert!(state.overlays.save_name_dialog.is_some(), "a write error re-opens the dialog");
        assert_eq!(state.ingame_resume_save, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── Restore: both containers, both directions ────────────────────────────

    #[test]
    fn ingame_restore_of_a_babelmap_round_trips_the_game() {
        // Write an @save archive, play on, then answer the game's own @restore
        // with that archive. Oracle: the same probe command after the restore
        // reproduces the pre-restore transcript exactly.
        let dir = temp_dir("ingame-restore");
        let mut sess = at_ingame_save();
        let mut state = state_with_session(Some(PendingIo::Save));
        let mut mapper = mapper_with_room();
        super::handle_save_as("slot".into(), &dir, IFID, &mut mapper, &mut sess, &mut state);
        let path = dir.join("slot.babelmap");

        let _ = sess.resume_save(true); // @save completes; the game runs on
        let t1 = sess.submit("north").transcript;

        let r = sess.submit("restore");
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "'restore' reaches @restore");
        let bytes = app::archive::read_quetzal_from_file(&path).expect("inner bytes");
        sess.resume_restore(Some(&bytes));

        let t2 = sess.submit("north").transcript;
        assert_eq!(t2, t1, "in-game @restore of a .babelmap resumes the save point");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingame_restore_of_a_foreign_raw_qzl_still_works() {
        // The interchange path must not regress: a bare Quetzal file carried in
        // from another interpreter still answers the game's @restore.
        let dir = temp_dir("foreign");
        let mut sess = at_ingame_save();
        let foreign = dir.join("from-frotz.qzl");
        std::fs::write(&foreign, sess.machine.save_quetzal()).unwrap();

        let _ = sess.resume_save(true);
        let t1 = sess.submit("north").transcript;

        let r = sess.submit("restore");
        assert_eq!(r.pending_io, Some(PendingIo::Restore));
        let bytes = app::archive::read_quetzal_from_file(&foreign).expect("raw bytes");
        sess.resume_restore(Some(&bytes));

        let t2 = sess.submit("north").transcript;
        assert_eq!(t2, t1, "a foreign raw .qzl still restores in-game");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_load_dispatches_on_the_trigger_not_the_extension() {
        use crate::engine_helpers::{restore_from_file, RestoreOutcome};

        let dir = temp_dir("host-load");
        let mut mapper = mapper_with_room();

        // (1) An @save archive -> descriptor completion, sidecars included.
        let mut sess = at_ingame_save();
        let mut state = state_with_session(Some(PendingIo::Save));
        super::handle_save_as("game-slot".into(), &dir, IFID, &mut mapper, &mut sess, &mut state);
        let ingame_path = dir.join("game-slot.babelmap");

        // (2) A host Save State archive -> full resume.
        let mut host_sess = minizork();
        let mut host_state = state_with_session(None);
        super::handle_save_as("host-slot".into(), &dir, IFID, &mut mapper, &mut host_sess, &mut host_state);
        let host_path = dir.join("host-slot.babelmap");

        // (3) A bare foreign .qzl -> descriptor completion, no sidecars.
        let raw = dir.join("foreign.qzl");
        std::fs::write(&raw, at_ingame_save().machine.save_quetzal()).unwrap();

        let mut fresh = minizork();
        match restore_from_file(&ingame_path, &mut fresh).expect("load @save archive") {
            RestoreOutcome::DescriptorCompleted(Some(ac)) => {
                assert_eq!(ac.transcript, state.transcript, "the archive's scrollback comes back too");
                assert_eq!(ac.meta.turns, 11);
            }
            _ => panic!("an @save archive must complete the descriptor, not resume"),
        }

        let mut fresh2 = minizork();
        assert!(
            matches!(restore_from_file(&host_path, &mut fresh2), Ok(RestoreOutcome::Resumed(_))),
            "a host Save State archive still resumes the whole session"
        );

        let mut fresh3 = minizork();
        assert!(
            matches!(restore_from_file(&raw, &mut fresh3), Ok(RestoreOutcome::DescriptorCompleted(None))),
            "a bare .qzl still completes the descriptor with no sidecars"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_saves_list_reports_the_trigger_for_every_kind() {
        let dir = temp_dir("listing");
        let mut mapper = mapper_with_room();

        let mut sess = at_ingame_save();
        let mut state = state_with_session(Some(PendingIo::Save));
        super::handle_save_as("from the game".into(), &dir, IFID, &mut mapper, &mut sess, &mut state);

        let mut host_sess = minizork();
        let mut host_state = state_with_session(None);
        super::handle_save_as("from the host".into(), &dir, IFID, &mut mapper, &mut host_sess, &mut host_state);

        std::fs::write(dir.join("from-frotz.qzl"), b"carried in from elsewhere").unwrap();

        let entries = crate::combined_saves(&dir);
        let find = |n: &str| {
            entries.iter().find(|e| e.name == n).unwrap_or_else(|| panic!("{n} should be listed"))
        };
        assert_eq!(find("from the game").trigger, SaveTrigger::Ingame);
        assert_eq!(find("from the host").trigger, SaveTrigger::HostState);
        assert_eq!(find("from-frotz").trigger, SaveTrigger::Ingame, "a bare .qzl IS a game save");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
