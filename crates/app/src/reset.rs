//! Game restart/reset: rebuild the engine from the original story bytes via the
//! same factory used at startup, optionally clearing the accumulated map and/or
//! deleting the game's AUTO persistent data. Extracted verbatim from `main.rs`
//! (SQ-0306) as a pure move — no behavior change.

use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::hints;
use app::session::{apply_turn, GameSession, TurnResult};
use app::state::AppState;
use mapper::mapper::Mapper;

use crate::engine_helpers::zvm_session_mut;
use crate::resolve_pict_blorb;

pub(crate) fn reset_game(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    story_bytes: &[u8],
    story_path: &std::path::Path,
    game_dir: &std::path::Path,
    clear_map: bool,
    delete_data: bool,
) {
    // Delete the game's AUTO persistent data BEFORE rebuilding so the fresh boot
    // re-initializes: the on-disk sidecars go now, and the in-memory VFS carried
    // into the Glulx rebuild is suppressed below (an empty carry_vfs).
    if delete_data {
        app::storage::delete_auto_persistent(game_dir);
    }
    // Rebuild the engine from the original story bytes via the same factory used
    // at startup: classify the executable, then replace the concrete session in
    // place (restart re-runs the SAME story, so the engine type is unchanged).
    let rebuilt: Result<(), String> = match hints::extract_story(story_bytes.to_vec()) {
        Ok(app::hints::LoadedStory::ZCode(bytes)) => {
            // Match the prior in-place restart exactly (no screen-dim write) so a
            // Z-machine restart stays byte-for-byte identical.
            GameSession::new(bytes, state.config.honor_game_colours, state.config.enable_sound, state.config.interpreter_number).map_err(|e| format!("{e:?}")).map(|mut new_session| {
                new_session.machine.undo_cap = state.config.undo_levels;
                *zvm_session_mut(session) = new_session;
            })
        }
        Ok(app::hints::LoadedStory::Glulx(bytes)) => {
            // Restart re-resolves the Pict Blorb the same path-based way as launch
            // (self-contained blorb, same-stem sidecar, or dir scan), and reuses
            // the stored game Picker for char-cell size, so graphics come back
            // enabled per config.images — matching the initial launch even for a
            // bare .ulx with a sidecar .blorb.
            let char_px = state
                .game_picker
                .as_ref()
                .map(|p| {
                    let f = p.font_size();
                    (f.width as u32, f.height as u32)
                })
                .unwrap_or((8, 16));
            let pict_blorb = resolve_pict_blorb(story_path, state.config.images);
            // Carry the current in-memory Glk file VFS (e.g. CM's boot cache,
            // kept in sync with the sidecar) into the restarted session so the
            // fresh boot still sees it (SQ-0290). When delete_data is set, carry
            // an EMPTY VFS instead so the game boots with no cache and re-runs its
            // full initialization (deleting default.glkvfs on disk is not enough —
            // the cache also lives in memory and would otherwise be carried over).
            let carry_vfs = if delete_data { Vec::new() } else { session.vfs_bytes() };
            // Preserve the per-game borderless-windows override across @restart (SQ-0341).
            let borderless =
                app::styles::read_per_game_borderless(game_dir).unwrap_or(false);
            GlulxSession::new_in(
                game_dir.to_path_buf(),
                bytes,
                state.config.virtual_screen_cols as u32,
                state.config.virtual_screen_rows as u32,
                state.config.acceleration,
                state.config.images,
                state.config.enable_sound,
                borderless,
                char_px,
                pict_blorb,
                &carry_vfs,
                // The live theme's rendered colours, in place before the fresh
                // boot probes glk_style_measure (SQ-0315).
                app::glk_backend::theme_style_colours(&state.colors),
            )
            .map_err(|e| format!("{e:?}"))
            .map(|new_session| {
                *session
                    .as_any_mut()
                    .downcast_mut::<GlulxSession>()
                    .expect("restart re-runs the same Glulx story") = new_session;
            })
        }
        Err(e) => Err(format!("{e}")),
    };
    match rebuilt {
        Ok(()) => {
            // The rebuilt session defaults strip_prompt=true; re-apply the config
            // choice so an in-game restart keeps the inline prompt in inline mode.
            session.set_strip_prompt(state.config.command_bar);
            let start_loc = session.current_location();
            state.reset_sound_sidecars();
            state.turns = 0;
            state.unsaved_progress = false; // restart: fresh game, nothing to save
            state.vm_halted = false;
            state.input.clear();
            state.suggestions.clear();
            state.suggestion_idx = 0;
            state.suggestion_active = false;
            state.transcript.clear();
            state.clear_anchor = None;
            state.transcript_kinds.clear();
            state.transcript_runs.clear();
            state.transcript_para.clear();
            state.transcript_scroll = 0;
            if clear_map {
                *mapper = Mapper::default();
            }
            // Glulx returns ordered elements (text + any startup images); the
            // Z-machine returns empty and uses the flat string path.
            let banner_elems = session.take_transcript_elems();
            if banner_elems.is_empty() {
                let banner = session.take_transcript();
                state.push_transcript(&banner);
            } else {
                app::state::apply_transcript_elems(state, &banner_elems);
            }
            if let Some(snap) = start_loc {
                let snap_number = snap.number;
                let seed_result = TurnResult {
                    transcript: String::new(),
                    transcript_runs: Vec::new(),
                    location: Some(snap),
                    quit: false,
                    erase_lower: false,
                    info: None,
                    sounds: Vec::new(),
                    glulx_sound_ops: Vec::new(),
                    diagnostics: vec![],
                    fault: None,
                    location_method: None,
                    pending_io: None,
                    timed_out: false,
                    transcript_elems: Vec::new(),
                };
                apply_turn(mapper, "", &seed_result);
                let rid = snap_number as mapper::graph::RoomId;
                state.select_room(Some(rid));
            }
            // Reset cleared and/or re-seeded the mapper graph — invalidate the map
            // memo so the fresh map (not the previous game's) shows. (SQ-0305)
            state.bump_graph_gen();
            state.push_notice("[Game reset]");
        }
        Err(e) => {
            state.push_notice(&format!("[Reset failed: {e}]"));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reset_game_rebuilds_zcode_engine() {
        // Restart rebuilds a working Z-machine engine via the story factory and
        // resets the turn counter.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let mut engine: Box<dyn app::engine::Engine> =
            Box::new(app::session::GameSession::new(bytes.clone(), true, false, None).expect("zcode session"));
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        // Inline-prompt mode (command_bar off): the rebuilt session must inherit
        // strip_prompt=false so @restart doesn't revert to stripping the game's `>`.
        state.config.command_bar = false;
        state.turns = 5;
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, std::path::Path::new(""), false, false);
        assert_eq!(state.turns, 0, "restart resets the turn counter");
        assert!(engine.as_any().is::<app::session::GameSession>(),
            "still a Z-machine session after restart");
        assert!(
            !engine.as_any().downcast_ref::<app::session::GameSession>().unwrap().strip_prompt(),
            "restart re-applies inline-prompt mode (strip_prompt stays false)"
        );
    }

    #[test]
    fn reset_game_bumps_graph_gen() {
        // Reset re-seeds the mapper graph via the production path; it must bump
        // graph_gen so the map render memo invalidates and the fresh map — not the
        // previous game's — is drawn this frame. (SQ-0305)
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let mut engine: Box<dyn app::engine::Engine> =
            Box::new(app::session::GameSession::new(bytes.clone(), true, false, None).expect("zcode session"));
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        let before = state.graph_gen;
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, std::path::Path::new(""), false, false);
        assert_ne!(state.graph_gen, before, "reset must bump graph_gen to invalidate the map memo");
    }

    #[test]
    fn reset_game_rebuilds_glulx_engine() {
        // Restart routes Glulx through the factory too (no "not supported"): a
        // fresh GlulxSession replaces the old one and the turn counter resets.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gvm-cli/tests/fixtures/glulxercise.ulx");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let mut engine: Box<dyn app::engine::Engine> = Box::new(
            app::glulx_session::GlulxSession::new(bytes.clone(), 80, 24, true, false, false, (1, 1), None, &[])
                .expect("glulx session"),
        );
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        // config.images defaults true, so restart drives the graphics-enabled
        // rebuild branch: the fixture .ulx has no sidecar .blorb, so
        // resolve_pict_blorb resolves to None and graphics_enabled = true is
        // threaded in — the rebuild must succeed without panicking.
        assert!(state.config.images, "default config enables images");
        state.turns = 5;
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, std::path::Path::new(""), false, false);
        assert_eq!(state.turns, 0, "restart resets the turn counter for Glulx");
        assert!(engine.as_any().is::<app::glulx_session::GlulxSession>(),
            "still a Glulx session after restart");
    }

    #[test]
    fn reset_game_with_delete_data_removes_auto_sidecars() {
        // delete_data = true wipes the three AUTO sidecars in game_dir before the
        // rebuild, while keeping the player's named/in-game saves.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };

        let game_dir = std::env::temp_dir()
            .join(format!("babelmap-reset-delete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&game_dir);
        std::fs::create_dir_all(&game_dir).unwrap();
        for f in ["default.glkvfs", "default.aux", "default.babelmap"] {
            std::fs::write(game_dir.join(f), b"x").unwrap();
        }
        std::fs::write(game_dir.join("myslot.babelmap"), b"x").unwrap();
        std::fs::write(game_dir.join("quick.qzl"), b"x").unwrap();

        let mut engine: Box<dyn app::engine::Engine> =
            Box::new(app::session::GameSession::new(bytes.clone(), true, false, None).expect("zcode session"));
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        super::reset_game(&mut *engine, &mut mapper, &mut state, &bytes, &fixture, &game_dir, false, true);

        for f in ["default.glkvfs", "default.aux", "default.babelmap"] {
            assert!(!game_dir.join(f).exists(), "{f} should be deleted by delete_data");
        }
        assert!(game_dir.join("myslot.babelmap").exists(), "named save kept");
        assert!(game_dir.join("quick.qzl").exists(), "in-game save kept");

        let _ = std::fs::remove_dir_all(&game_dir);
    }
}
