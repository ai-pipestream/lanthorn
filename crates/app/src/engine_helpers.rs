//! Engine downcast/utility helpers: the escape-hatch downcasts to the concrete
//! Z-machine / Glulx sessions behind a `dyn Engine`, plus the host restore
//! dispatch and save-support guard. Extracted verbatim from `main.rs` (SQ-0306)
//! as a pure move — no behavior change. Shared across the binary's modules
//! (main.rs, loop_tick.rs, turn.rs, startup.rs, lifecycle.rs) via `crate::`.

use app::archive::load_archive;
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::session::GameSession;

/// Escape hatch: borrow the concrete Z-machine `GameSession` behind a
/// `dyn Engine`, mutably.
///
/// Used ONLY by the persistence layer — archive save/restore and the
/// saved-screen snapshot — because the on-disk archive format serializes the
/// Z-machine `ScreenState` and cannot change without breaking compatibility
/// with existing saves (a no-behavior-change requirement). Everything else
/// (gameplay, render, input, introspection, `save_state`/`restore_state`,
/// `current_location`, aux) goes through the neutral `Engine` trait.
pub(crate) fn zvm_session_mut(engine: &mut dyn Engine) -> &mut GameSession {
    engine
        .as_any_mut()
        .downcast_mut::<GameSession>()
        .expect("babelmap drives a Z-machine GameSession")
}

/// Non-panicking downcast to the Z-machine session: `Some` for a Z-code game,
/// `None` for Glulx. The archive-save paths use it to source the **zvm-only**
/// `screen.json` (`Some(&z.machine.screen)` for the Z-machine, `None` for Glulx —
/// whose display lives inside its `EngineSave`); the save itself routes through
/// the engine-neutral `Engine::save_state` for both engines.
pub(crate) fn zvm_session_opt(engine: &dyn Engine) -> Option<&GameSession> {
    engine.as_any().downcast_ref::<GameSession>()
}

/// Mutable non-panicking downcast to the Z-machine session: `Some` for a Z-code
/// game, `None` for Glulx. Used to reinstate the zvm-only `screen.json` after an
/// archive restore without panicking on a Glulx engine.
pub(crate) fn zvm_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GameSession> {
    engine.as_any_mut().downcast_mut::<GameSession>()
}

/// Non-panicking downcast to the Glulx session: `Some` for a Glulx game, `None`
/// for Z-code. Used to read the armed Glk timer interval.
pub(crate) fn glulx_session_opt(engine: &dyn Engine) -> Option<&GlulxSession> {
    engine.as_any().downcast_ref::<GlulxSession>()
}

/// Mutable non-panicking downcast to the Glulx session: `Some` for a Glulx
/// game, `None` for Z-code. Used to deliver Glk sound-notify events.
pub(crate) fn glulx_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GlulxSession> {
    engine.as_any_mut().downcast_mut::<GlulxSession>()
}

/// The engine tag (`"zmachine"` / `"glulx"`) of the running engine, for wrapping
/// raw same-engine save bytes (e.g. a rewind/replay snapshot) into an
/// [`app::engine::EngineSave`] before `restore_state`.
pub(crate) fn engine_tag(engine: &dyn Engine) -> &'static str {
    if engine.as_any().is::<GlulxSession>() {
        app::glulx_session::GLULX_ENGINE
    } else {
        app::session::ZMACHINE_ENGINE
    }
}

/// Convert an [`app::engine::EngineError`] from `restore_state` into a graceful
/// player-facing message (no panic): a foreign-engine save names both engines, a
/// bad Z-machine save keeps the historical "different story" wording.
pub(crate) fn restore_error_msg(e: app::engine::EngineError) -> String {
    use app::engine::EngineError;
    match e {
        EngineError::EngineMismatch { expected, found } => format!(
            "this save was written by the \"{found}\" engine, but babelmap is running the \"{expected}\" engine"
        ),
        EngineError::BadSave(msg) if msg.contains("SaveMismatch") => {
            "save is for a different story".to_string()
        }
        EngineError::BadSave(msg) => format!("restore failed: {msg}"),
    }
}

/// Outcome of [`restore_from_file`]: either the pending `@save`/`@restore`
/// descriptor was completed (`.qzl` game save — caller just re-observes the
/// current location), or a full session was resumed from a Save State
/// archive (`.babelmap` — caller also applies its mapper/screen/transcript/aux).
pub(crate) enum RestoreOutcome {
    DescriptorCompleted,
    Resumed(Box<app::archive::ArchiveContents>),
}

/// Restore `path` into `session`, dispatching on its extension (SQ-0227): a
/// `.qzl` game save completes the pending `@save` descriptor
/// (`Engine::restore_game_save`); anything else (`.babelmap`) resumes a full
/// Save State (`Engine::restore_state`). This is the fix for the SQ-0163
/// regression — every host restore path used to call `restore_state`
/// unconditionally, landing the VM on the descriptor instead of past it.
/// Shared by every host load/restore site (saves-manager Load, `/restore-state`,
/// and a `.babelmap` picked from the in-game restore picker).
pub(crate) fn restore_from_file(path: &std::path::Path, session: &mut dyn Engine) -> Result<RestoreOutcome, String> {
    if app::persist_files::is_game_save(path) {
        let bytes = app::archive::read_quetzal_from_file(path).map_err(|e| e.to_string())?;
        session.restore_game_save(&bytes).map_err(restore_error_msg)?;
        Ok(RestoreOutcome::DescriptorCompleted)
    } else {
        let ac = load_archive(path).map_err(|e| e.to_string())?;
        session.restore_state(&ac.engine_save()).map_err(restore_error_msg)?;
        Ok(RestoreOutcome::Resumed(Box::new(ac)))
    }
}

/// Whether the active engine is the Z-machine `GameSession` required by the
/// standard `.qzl`/`.sav` Quetzal **import** path.
///
/// That path reaches the concrete session via [`zvm_session_mut`]/[`zvm_session_opt`]
/// (which PANIC / return `None` on any other engine) and reads raw Quetzal saves
/// from other interpreters — Z-machine-only until cross-interpreter Glulx Quetzal exists.
/// They check this first and bail gracefully when it returns `false` (a Glulx
/// game). The `.babelmap` archive save/restore/restart paths no longer need it:
/// they route through the engine-neutral `Engine::save_state`/`restore_state`
/// and work for both engines.
pub(crate) fn engine_supports_save(engine: &dyn Engine) -> bool {
    engine.as_any().downcast_ref::<GameSession>().is_some()
}

#[cfg(test)]
mod tests {
    // ── SQ-0227 Task 3: restore dispatch on file extension ──────────────────────
    //
    // `restore_from_file` is the dispatch shared by every host restore site
    // (saves-manager Load, `/restore-state`, and a `.babelmap` picked from the
    // in-game restore picker). Regression proof for SQ-0163: every host
    // restore path used to call `restore_state` (resume) unconditionally, so
    // a host restore of an in-game `@save` (`.qzl`) landed the VM on the
    // descriptor instead of past it.

    use crate::tests::read_char_then_save_v4_story;

    #[test]
    fn restore_from_file_completes_qzl_descriptor_and_resumes_babelmap_sq0163() {
        use app::engine::Engine;
        use app::session::{GameSession, InputKind, PendingIo};

        // In-game @save: suspend with pending_save set (descriptor PC), and
        // capture the .qzl bytes exactly as save_game_named does (Task 2) --
        // while pending_save is still set, before resume_save runs.
        let mut sess = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save));
        let qzl_bytes = sess.machine.save_quetzal();
        let _ = sess.resume_save(true); // host "wrote" the .qzl; @save completes, VM runs to quit.

        let qzl_path = std::env::temp_dir().join(format!("bm-t3-{}.qzl", std::process::id()));
        std::fs::write(&qzl_path, &qzl_bytes).unwrap();

        // HOST restore of that .qzl (the SQ-0163 regression scenario): must
        // dispatch to descriptor completion, not a resume.
        let mut fresh = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let outcome = super::restore_from_file(&qzl_path, &mut fresh).expect("restore .qzl game save");
        assert!(matches!(outcome, super::RestoreOutcome::DescriptorCompleted));
        assert_eq!(fresh.machine.global(0), 2, "descriptor completion stores 2 into G0 (SQ-0163 fix)");
        // SQ-0233: the host .qzl restore now runs FORWARD past the @save
        // descriptor to the game's next input (like the in-game @restore),
        // instead of parking on the save-verb tail (which dropped the first
        // typed command). This minimal story quits right after @save, so it runs
        // to quit; a real game lands at its next read (covered by
        // session::tests::game_save_restore_via_manager_accepts_next_command).
        assert_ne!(fresh.machine.state.pc, 0x46,
            "restore runs forward past the @save descriptor, not parked on it (SQ-0233)");
        let _ = std::fs::remove_file(&qzl_path);

        // Contrast: a Save State (.babelmap) is resume-PC convention --
        // captured at an input prompt, no pending @save. The dispatch must
        // instead do a full session resume, landing exactly at the saved PC.
        let sess2 = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        assert_eq!(sess2.pending_input(), InputKind::Char);
        let pc_before_restore = sess2.machine.state.pc;
        let save = sess2.save_state();

        let babelmap_path = std::env::temp_dir().join(format!("bm-t3-{}.babelmap", std::process::id()));
        app::archive::save_archive(&babelmap_path, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), &[], &[], &[], &[], &[]).expect("write .babelmap");

        let mut fresh2 = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let outcome2 = super::restore_from_file(&babelmap_path, &mut fresh2).expect("restore .babelmap Save State");
        assert!(matches!(outcome2, super::RestoreOutcome::Resumed(_)));
        assert_eq!(fresh2.machine.state.pc, pc_before_restore, "resume convention: lands exactly at the saved PC, not the @save descriptor");
        assert_eq!(fresh2.machine.global(0), 0, "resume: @save never ran, G0 untouched (contrast with descriptor completion's 2 above)");
        let _ = std::fs::remove_file(&babelmap_path);
    }

    // ── Graceful no-panic guards for non-Z-machine (Glulx) engines ──────────────

    /// Minimal non-Z-machine `Engine` stand-in. The guard helper only inspects
    /// `as_any`, so every gameplay/persistence method is left `unreachable!()`:
    /// a guarded path that reaches one would be the very panic we are preventing.
    struct NotZmachineEngine;

    impl app::engine::Engine for NotZmachineEngine {
        fn submit(&mut self, _command: &str) -> app::session::TurnResult { unreachable!() }
        fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> { unreachable!() }
        fn take_transcript(&mut self) -> String { unreachable!() }
        fn pending_input(&self) -> app::session::InputKind { unreachable!() }
        fn resume_save(&mut self, _wrote_ok: bool) -> app::session::TurnResult { unreachable!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult { unreachable!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> app::engine::ScreenModel { unreachable!() }
        fn save_state(&self) -> app::engine::EngineSave { unreachable!() }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> { unreachable!() }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> { unreachable!() }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) { unreachable!() }
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> { None }
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    fn engine_supports_save_false_for_non_zmachine() {
        let engine: Box<dyn app::engine::Engine> = Box::new(NotZmachineEngine);
        assert!(
            !super::engine_supports_save(&*engine),
            "a non-Z-machine engine must report no save support so guards short-circuit"
        );
    }
}
