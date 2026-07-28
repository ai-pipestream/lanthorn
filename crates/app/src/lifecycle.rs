//! Exit / quit persistence paths: exit auto-save, the quit-dialog "Save State &
//! quit" snapshot, and the pending config-write flush. Extracted verbatim from
//! `main.rs` (SQ-0306) as a pure move — no behavior change. The SQ-0283
//! save/restore-pending guards and the auto-save gate move intact inside the
//! bodies. Helper fns these rely on stay in `main.rs` (referenced via `crate::`).

use mapper::mapper::Mapper;

use app::engine::Engine;
use app::state::AppState;

use crate::engine_helpers::zvm_session_opt;
use crate::format_rfc3339;

/// Save on exit ONLY when auto_save is enabled. With auto_save off (the default),
/// nothing is saved automatically — the user controls saving via the quit prompt's
/// "Save State & quit", the /save-state command, or named save slots. This keeps
/// "Quit without saving" honest and avoids silently overwriting an explicit save
/// point on exit.
/// Exit auto-save is engine-neutral: the save routes through Engine::save_state
/// (Quetzal for zvm, the gvm snapshot for Glulx); screen.json is written for
/// zvm only.
/// Skip while a Glulx in-game @save/@restore is suspended, awaiting host I/O:
/// snapshotting mid-suspension would capture the un-popped @save call stub,
/// and restore_state never pops it -> a corrupted stack on a later Save State
/// restore (SQ-0283 carry-forward fix). The in-game save the player was
/// already making is the relevant persistence in that case.
pub(crate) fn exit_auto_save(
    session: &dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if !state.config.auto_save || session.is_saveload_pending() {
        return;
    }
    let (location, score) = crate::engine_helpers::save_summary(session, state);
    let exit_meta = app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: None,
        turns: state.turns,
        saved_at: format_rfc3339(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        location,
        score,
    };
    let v6_pics = zvm_session_opt(session).map(|z| z.pictures_png()).unwrap_or_default();
    match app::archive::save_archive_meta_pics(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), exit_meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para, &state.transcript_images, &state.history, &state.command_history, &v6_pics) {
        Ok(()) => {
            eprintln!("babelmap: map saved to {}", arc_file.display());
        }
        Err(e) => {
            eprintln!("babelmap: warning: could not save to {}: {}", arc_file.display(), e);
        }
    }
}

/// Quit-dialog "Save State & quit" host snapshot, extracted from the quit-dialog
/// keyboard and mouse handlers so the guard below is unit-testable.
/// Skip while a Glulx in-game @save/@restore is suspended, awaiting host I/O:
/// snapshotting mid-suspension would capture the un-popped @save call stub, and
/// restore_state never pops it -> a corrupted stack on a later Save State
/// restore (SQ-0283 carry-forward fix). The in-game save the player was already
/// making is the relevant persistence in that case; the dialog still proceeds
/// to quit either way.
pub(crate) fn quit_dialog_save(
    session: &dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if session.is_saveload_pending() {
        return;
    }
    let (location, score) = crate::engine_helpers::save_summary(session, state);
    let meta = app::archive::Meta {
        format_version: app::archive::CURRENT_FORMAT_VERSION,
        ifid: Some(ifid.to_string()),
        name: None,
        turns: state.turns,
        saved_at: format_rfc3339(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        ),
        location,
        score,
    };
    let v6_pics = zvm_session_opt(session).map(|z| z.pictures_png()).unwrap_or_default();
    let _ = app::archive::save_archive_meta_pics(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para, &state.transcript_images, &state.history, &state.command_history, &v6_pics);
}

// ── Pending config-write flush ────────────────────────────────────────────────

/// Write `state.config` to `config.toml` if `pending_config_write` is set, then
/// clear the flag. Called after both key-dispatch paths (`KeyResolve::Action`
/// and `KeyResolve::Command`, the latter via `dispatch_slash_outcome`) so a
/// resize-reset/exit persists regardless of which path handled the key.
pub(crate) fn flush_pending_config_write(state: &mut AppState) {
    if state.pending_config_write {
        let user_dir = state.config.user_dir.clone();
        let _ = app::config::write_config(&user_dir, &state.config);
        state.pending_config_write = false;
    }
}

#[cfg(test)]
mod tests {
    /// Engine stand-in whose in-game @save/@restore never resolves (mirrors a
    /// mid-suspension Glulx session). `save_state`/`aux_data` are left
    /// `unreachable!()`: the exit auto-save guard (SQ-0283 Task 6 carry-forward
    /// fix) must never reach them while a save/restore is pending -- reaching
    /// either would be the very bug (a snapshot capturing the un-popped @save
    /// call stub) the guard exists to prevent.
    struct SaveloadPendingEngine;

    impl app::engine::Engine for SaveloadPendingEngine {
        fn submit(&mut self, _command: &str) -> app::session::TurnResult { unreachable!() }
        fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> { unreachable!() }
        fn take_transcript(&mut self) -> String { unreachable!() }
        fn pending_input(&self) -> app::session::InputKind { unreachable!() }
        fn resume_save(&mut self, _wrote_ok: bool) -> app::session::TurnResult { unreachable!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult { unreachable!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> app::engine::ScreenModel { unreachable!() }
        fn save_state(&self) -> app::engine::EngineSave {
            unreachable!("exit_auto_save must not snapshot while a save/restore is pending")
        }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn is_saveload_pending(&self) -> bool { true }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            unreachable!("exit_auto_save must not read aux data while a save/restore is pending")
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) { unreachable!() }
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    fn exit_auto_save_skips_snapshot_while_a_save_is_pending() {
        // SQ-0283 carry-forward fix: a host save_state() snapshot captured while
        // a Glulx in-game @save is suspended would embed the un-popped @save call
        // stub; restore_state never pops it, corrupting the stack on a later Save
        // State restore. exit_auto_save must skip entirely (not call save_state)
        // when Engine::is_saveload_pending() is true, even with auto_save on.
        let engine = SaveloadPendingEngine;
        let mut state = app::state::AppState::default();
        state.config.auto_save = true;
        let mapper = mapper::mapper::Mapper::default();
        let arc_file = std::env::temp_dir().join(format!("bm-t6-pending-{}.babelmap", std::process::id()));
        let _ = std::fs::remove_file(&arc_file);

        // Must not panic (save_state()/aux_data() are unreachable!()) and must not
        // write the archive file.
        super::exit_auto_save(&engine, &mapper, &state, "ZCODE-1", &arc_file);

        assert!(!arc_file.exists(), "exit auto-save must not write while a save/restore is pending");
        let _ = std::fs::remove_file(&arc_file);
    }

    #[test]
    fn quit_dialog_save_skips_snapshot_while_a_save_is_pending() {
        // SQ-0283 review fix: the quit-dialog "Save State & quit" path was an
        // unguarded save_state() reachable while a Glulx in-game @save is
        // suspended (Ctrl+Q wins even over an open SaveAs prompt). Mirrors
        // exit_auto_save_skips_snapshot_while_a_save_is_pending above but for the
        // extracted quit_dialog_save helper, which has no auto_save gate.
        let engine = SaveloadPendingEngine;
        let state = app::state::AppState::default();
        let mapper = mapper::mapper::Mapper::default();
        let arc_file = std::env::temp_dir().join(format!("bm-t6-quit-pending-{}.babelmap", std::process::id()));
        let _ = std::fs::remove_file(&arc_file);

        // Must not panic (save_state()/aux_data() are unreachable!()) and must not
        // write the archive file.
        super::quit_dialog_save(&engine, &mapper, &state, "ZCODE-1", &arc_file);

        assert!(!arc_file.exists(), "quit-dialog save must not write while a save/restore is pending");
        let _ = std::fs::remove_file(&arc_file);
    }
}
