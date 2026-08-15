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
            // Rebuild through the SAME construction startup uses, not the bare
            // `GameSession::new` (SQ-0546). A v6 game needs all of it before and
            // after its boot run: the Pict dimension table (`picture_data` is
            // called DURING boot, inside the constructor), the Blorb `Reso`
            // standard window that sizes the 640×400 unit screen its windows and
            // hardcoded art align to, the host default colour pair, then the Pict
            // source and a boot-picture flush to drain the art the boot drew
            // before that source existed. Restarting without them left Shogun
            // with a mis-sized status band and no inline graphics at all.
            // SQ-0734: a restart re-resolves all three tiers, so the archive the
            // per-game sidecar names is still the one in force afterwards. The
            // profile is NOT re-derived from it — `state.config.interpreter_profile`
            // below is the one boot settled, and a restart re-runs the same story
            // on the same machine.
            // SQ-0789/0791: `pictures_override` carries a choice made for THIS
            // launch and never written down (`--pictures`, or the launch dialog
            // with its checkbox left clear). Without it a restart would re-read
            // only the sidecar and silently swap the art back to the Blorb.
            let over = if state.config.images {
                app::graphics::PictureOverride::resolve_with_session(
                    story_path,
                    &state.game_dir,
                    state.config.pictures_override.as_deref(),
                )
            } else {
                app::graphics::PictureOverride::Unset
            };
            let named_art_std_window = over.std_window();
            let mut picts = if state.config.images {
                // The story's own entry on the medium, carried by the config for
                // exactly this moment (SQ-0876).
                app::graphics::PictSource::resolve_with_override(
                    story_path,
                    over,
                    state.config.disk_entry.clone().as_deref(),
                )
            } else {
                app::graphics::PictSource::new(None)
            };
            // SQ-0719: a restart re-runs the same story on the same machine, so
            // the interpreter profile resolved at boot supplies the same three
            // answers it did there — the standard window a native Amiga archive
            // has no chunk to declare, the machine's own default colours, and its
            // interpreter number. IBM PC (every Blorb-sourced story) supplies
            // none of them and this is the prior code exactly.
            let profile = state.config.interpreter_profile;
            // SQ-0816: and the same dither preference the launch resolved, so a
            // restart does not quietly change what the artwork looks like.
            picts.set_fuse_dither(state.config.fuse_art_dither);
            let picture_dims = picts.all_pict_dims();
            // The same four links `startup.rs` resolves, in the same order, so a
            // restart comes back on the screen the launch settled — including
            // the archive's own picture space, which is the standard Macintosh's
            // 480×300 when the mono archive is the one mounted (SQ-0838).
            let v6_screen_px = picts
                .std_window()
                .or(named_art_std_window)
                .or_else(|| picts.native_std_window())
                .or_else(|| profile.std_window());
            // SQ-0790: and the density the art arrives at, so a restart of a
            // story playing its EGA rendition comes back with the same geometry
            // it booted with. `None` for every Blorb-sourced story.
            let v6_art_scale = picts.art_scale();
            let host_default_colours = if state.config.honor_game_colours {
                profile.default_colours().or_else(|| {
                    app::colors::host_default_colour_pair(
                        state.colors.theme.get("transcript").style,
                        state.term_default_colors.fg.map(|c| (c.0[0], c.0[1], c.0[2])),
                        state.term_default_colors.bg.map(|c| (c.0[0], c.0[1], c.0[2])),
                    )
                })
            } else {
                None
            };
            GameSession::new_with_art_scale(
                bytes,
                state.config.honor_game_colours,
                state.config.enable_sound,
                state.config.interpreter_number.or_else(|| profile.interpreter_number()),
                // Keep boot tracing across a restart in a `--debug` session, as
                // the Glulx arm below does.
                state.persist_debug_trace,
                picture_dims,
                v6_screen_px,
                v6_art_scale,
                host_default_colours, None,
                // A restart re-draws the seed the same way the launch did
                // (SQ-0811): a pinned `random_seed` replays the same game, and an
                // unpinned one deals a fresh one — which is what restarting a
                // randomised game is FOR.
                Some(state.config.effective_random_seed()),
            )
            .map_err(|e| format!("{e:?}"))
            .map(|mut new_session| {
                new_session.machine.undo_cap = state.config.undo_levels;
                new_session.set_pict_source(Some(picts));
                new_session.flush_boot_pictures();
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
            // Preserve the per-game borderless-windows override across @restart
            // (SQ-0341); an explicit per-game value wins over a garglk.ini
            // `wborder`, else garglk's, else off (SQ-0344).
            let borderless = app::styles::read_per_game_borderless(game_dir)
                .or_else(|| state.garglk_overlay.as_ref().and_then(|o| o.borderless))
                .unwrap_or(false);
            GlulxSession::new_in(
                game_dir.to_path_buf(),
                bytes,
                state.config.virtual_screen_cols.unwrap_or(app::config::FALLBACK_SCREEN_COLS) as u32,
                state.config.virtual_screen_rows.unwrap_or(app::config::FALLBACK_SCREEN_ROWS) as u32,
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
                // Keep the debug inspector's boot-tracing across @restart when a
                // `--debug` session is active, so the restarted boot is captured too.
                state.persist_debug_trace,
                // Re-seeded exactly as the launch was (SQ-0811) — see the zvm arm.
                Some(state.config.effective_random_seed()),
            )
            .map_err(|e| format!("{e:?}"))
            .map(|new_session| {
                *session
                    .as_any_mut()
                    .downcast_mut::<GlulxSession>()
                    .expect("restart re-runs the same Glulx story") = new_session;
            })
        }
        Ok(app::hints::LoadedStory::Scott(bytes)) => app::scott_session::ScottSession::new_with_trace(
            bytes,
            resolve_pict_blorb(story_path, state.config.images),
            false,
            // Re-seeded exactly as the launch was (SQ-0811) — see the zvm arm.
            Some(state.config.effective_random_seed()),
        )
        .map(|new_session| {
                *session
                    .as_any_mut()
                    .downcast_mut::<app::scott_session::ScottSession>()
                    .expect("restart re-runs the same Scott story") = new_session;
            }),
        Err(e) => Err(format!("{e}")),
    };
    match rebuilt {
        Ok(()) => {
            // The rebuilt session defaults strip_prompt=true; re-apply the config
            // choice so an in-game restart keeps the inline prompt in inline mode.
            session.set_strip_prompt(state.config.command_bar);
            let start_loc = session.current_location();
            state.reset_sound_sidecars();
            // A restart is a new game: the death the old one left unresolved died with it, and so
            // did the `tried` record a fatal move there might still owe. Carried across, an
            // outstanding death would swallow the first room change of the fresh game — which is
            // the seed below, or the first passage the player walks. (SQ-0671, SQ-0673)
            state.death_watch = app::session::DeathWatch::default();
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
                    pictures: Vec::new(),
                    transcript_elems: Vec::new(),
                    prose_retired: None,
                };
                apply_turn(mapper, "", &seed_result, &mut state.death_watch);
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
    /// SQ-0546: restarting a v6 story must rebuild it the way LAUNCH does.
    ///
    /// `reset_game`'s Z-machine arm used the bare `GameSession::new`, which
    /// carries no Pict dimension table, no Blorb `Reso` standard window and no
    /// host colour pair, and never attached a Pict source or flushed the boot
    /// pictures. A v6 game needs every one of those: `picture_data` is answered
    /// DURING the boot run inside the constructor, the `Reso` window sizes the
    /// 640×400 unit screen its windows and hardcoded art align to, and the art
    /// drawn during boot has to be drained once afterwards. So `/reset-game` on
    /// Shogun came back with a mis-sized status band and no graphics at all
    /// (user report at the TTY, 2026-07-28).
    ///
    /// Pins the observable half: after a reset the v6 screen model is present at
    /// its native size and the boot art has been rasterized.
    #[test]
    fn reset_game_rebuilds_a_v6_story_with_its_pictures_and_screen() {
        use app::engine::{Engine, WinNode};
        let story = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/shogun-r322-s890706.z6");
        let Ok(bytes) = std::fs::read(&story) else {
            eprintln!("SKIP: gitignored story missing at {}", story.display());
            return;
        };
        // Build the session the way launch does, so the "before" state is real.
        let mut picts = app::graphics::PictSource::new(
            blorb::resolve_resource_blorb(&story).map(|(b, _)| b),
        );
        let dims = picts.all_pict_dims();
        let std_window = picts.std_window();
        let mut s = app::session::GameSession::new_with_trace(
            bytes.clone(), true, false, None, false, dims, std_window, None, None
        )
        .expect("Shogun boots");
        s.set_pict_source(Some(picts));
        s.flush_boot_pictures();
        let mut engine: Box<dyn Engine> = Box::new(s);

        let native_before = match &engine.screen().root {
            WinNode::Layered(items) => app::render::v6_layout::native_extent(items),
            other => panic!("expected a v6 Layered root before reset, got {other:?}"),
        };
        assert_eq!(native_before, (640, 400), "launch sizes the v6 unit screen");

        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        state.config.images = true;
        state.turns = 5;
        super::reset_game(
            &mut *engine, &mut mapper, &mut state, &bytes, &story,
            std::path::Path::new(""), false, false,
        );

        assert_eq!(state.turns, 0, "restart resets the turn counter");
        let model = engine.screen();
        let items = match &model.root {
            WinNode::Layered(items) => items,
            other => panic!("v6 story must still present a Layered root after reset, got {other:?}"),
        };
        assert_eq!(
            app::render::v6_layout::native_extent(items),
            native_before,
            "the restarted v6 screen keeps its native size (the Reso standard window \
             reached the constructor)"
        );
        // The boot art was rasterized again: at least one graphics window carries
        // opaque pixels. Without the Pict source + boot flush every canvas is empty.
        fn painted(node: &WinNode) -> bool {
            match node {
                WinNode::Graphics(g) => g.canvas.pixels().any(|p| p.0[3] != 0),
                WinNode::Pair { first, second, .. } => painted(first) || painted(second),
                WinNode::Layered(items) => items.iter().any(|i| painted(&i.node)),
                _ => false,
            }
        }
        assert!(
            painted(&model.root),
            "the restarted boot re-drew its graphics (Pict source attached + boot flush)"
        );
    }

    /// SQ-0673: a restart is a new game, so the death watch goes with the old one.
    ///
    /// It carries two things across turns — the `tried` record a fatal move may still have to
    /// take back, and an unresolved death waiting for a resurrection — and both are claims about
    /// a game that no longer exists. Left set, the outstanding death would swallow the first
    /// passage the fresh game walked, which is exactly the move that seeds the new map.
    #[test]
    fn reset_game_clears_the_death_watch() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        let Ok(bytes) = std::fs::read(&fixture) else { return };
        let session = app::session::GameSession::new(bytes.clone(), true, false, None)
            .expect("zcode session");
        let mut engine: Box<dyn app::engine::Engine> = Box::new(session);
        let mut mapper = mapper::mapper::Mapper::default();
        let mut state = app::state::AppState::default();
        state.death_watch = app::session::DeathWatch {
            pending_tried: Some((7, mapper::direction::Direction::N)),
            unresolved: true,
        };
        super::reset_game(
            &mut *engine,
            &mut mapper,
            &mut state,
            &bytes,
            &fixture,
            std::path::Path::new(""),
            false,
            false,
        );
        assert_eq!(
            state.death_watch,
            app::session::DeathWatch::default(),
            "the restarted game inherits no outstanding death"
        );
    }

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
