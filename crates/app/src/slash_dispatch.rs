//! SlashOutcome side-effect dispatch: the single switch that applies a parsed
//! `SlashOutcome` (from typed input or a key binding) against the live app
//! state, engine, and mapper. Extracted verbatim from `main.rs` (SQ-0306) as a
//! pure move — no behavior change. Touches binary-only helpers (save/restore
//! plumbing, reset, hints, transcript export), all reached via `crate::`.

use app::archive::save_archive_meta_pics;
use app::engine::Engine;
use app::export::export_transcript;
use app::input::{apply_action, Action};
use app::persist_files::{load_map, save_named};
use app::slash::{self, SlashOutcome, TranscriptFilterArg};
use app::state::{AppState, ExitTarget, Focus, SavesState, TranscriptFilter, TranscriptKind};
use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use crate::engine_helpers::{apply_archive_state, restore_from_file, zvm_session_opt, RestoreOutcome};
use crate::reset::reset_game;
use crate::{
    combined_saves, format_rfc3339, handle_map_export, open_hints, reobserve_location,
    scroll_for_match, should_prompt_save_on_quit, toggle_style_watch,
};

/// Handle a parsed `SlashOutcome` from either typed input or a key dispatch.
///
/// Both the typed-command path and the keybinding path resolve to a
/// `SlashOutcome` and funnel through here so the two share one behaviour. The
/// run loop owns the actual loop, so the `Quit` outcome cannot `break` directly:
/// this returns `true` when the loop should break (a non-dialog quit).
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_slash_outcome(
    outcome: SlashOutcome,
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    style_watcher: &mut Option<app::watch::StyleWatcher>,
    game_dir: &std::path::Path,
    ifid: &str,
    arc_file: &std::path::Path,
    story_bytes: &[u8],
    story_path: &std::path::Path,
    map_rect: Rect,
    story_rect: Rect,
    from_key: bool,
) -> bool {
    match outcome {
        SlashOutcome::Action(a) => {
            if matches!(a, Action::OpenSaves) {
                // OpenSaves needs `game_dir` to populate the saves list, which
                // `apply_action` (state + mapper only) can't do — it just resets
                // the modal's focus. Build the populated dialog here so the
                // command/slash path (e.g. `restore-state`) actually opens it.
                let entries = combined_saves(game_dir);
                apply_action(Action::OpenSaves, state, mapper);
                state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
            } else if handle_map_export(&a, game_dir, mapper, state) {
                // handled
            } else if matches!(a, Action::ToggleWatch) {
                toggle_style_watch(state, style_watcher);
            } else {
                apply_action(a, state, mapper);
            }
        }
        SlashOutcome::Message(m) | SlashOutcome::Error(m) => {
            state.set_status(m);
        }
        SlashOutcome::Help => {
            for line in slash::help_text(state.config.command_prefix) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::PrintColors { actual } => {
            for (line, style_opt) in app::theme::describe_theme(&state.colors.theme) {
                match (actual, style_opt) {
                    (true, Some(style)) => state.push_transcript_internal_styled(&line, TranscriptKind::Meta, style),
                    _ => state.push_transcript_internal(&line, TranscriptKind::Meta),
                }
            }
            // SQ-0510 diagnostics: what the OSC 10/11 startup probe actually
            // captured — the input to the v6 raster ink/page pair resolution.
            let fmt = |c: Option<image::Rgba<u8>>| match c {
                Some(image::Rgba([r, g, b, _])) => format!("rgb({r},{g},{b})"),
                None => "unanswered".to_string(),
            };
            let td = state.term_default_colors;
            state.push_transcript_internal(
                &format!("terminal defaults (OSC 10/11 probe): fg {} · bg {}", fmt(td.fg), fmt(td.bg)),
                TranscriptKind::Meta,
            );
        }
        SlashOutcome::DumpWindows => {
            // A v6 story reports one block per window, merging the game's window
            // table, the model, and where the last frame put each on the terminal —
            // the three things that have to agree (SQ-0585). The engine owns the
            // first two; the render mapping lives here, so hand it over.
            //
            // The mapping is the LAST GAME FRAME's, not the live one (SQ-0756). This
            // command is reached through the palette or a hotkey dialog, so the frame
            // in `v6_cell_map` is always the modal's, in which the game's windows are
            // all "NOT DRAWN this frame" — the one thing a reader is here to learn.
            // The game halves stay live: a modal runs no game code, so the window
            // table and the model still describe the frame being reported.
            let (frame, frame_line) = state.v6_dump_frame();
            let cells = frame.as_ref().map(|f| f.cells.clone()).unwrap_or_default();
            let history: Vec<String> = state
                .v6_path_log
                .borrow()
                .iter()
                .map(|(label, n)| if *n > 1 { format!("{label} x{n}") } else { label.clone() })
                .collect();
            let mut out: Vec<String> = match session.as_any().downcast_ref::<app::session::GameSession>() {
                Some(gs) if !gs.v6_window_dump(&cells).is_empty() => gs.v6_window_dump(&cells),
                _ => session.window_dump(),
            };
            out.push(frame_line);
            // The ring's plan and clip belong to the frame that drew them, so they
            // ride the same snapshot; with no game frame recorded there is nothing
            // honest to print (SQ-0756).
            let clip = match frame.as_ref() {
                None => "unavailable — no game frame recorded".to_string(),
                Some(f) => match f.ring_clip {
                    Some((art, row)) if art == u16::MAX => {
                        format!("plan {}, ring clipped at row {row} — NO opaque art found in the canvas", f.ring_plan)
                    }
                    Some((art, row)) => format!(
                        "plan {}, ring clipped at row {row} (art opaque down to native y={art})",
                        f.ring_plan
                    ),
                    None => format!("plan {}, ring not clipped", f.ring_plan),
                },
            };
            out.push(format!("  ring: {clip}"));
            // SQ-0588: windows whose recorded ops did not reproduce their canvas at
            // save time. Each is a draw path we are not recording — the archive fell
            // back to a PNG for it, so it restores correctly but cannot be recoloured
            // by a later palette change.
            let save_diags: Vec<String> = state.v6_save_log.borrow().clone();
            for msg in &save_diags {
                out.push(format!("  save: {msg}"));
            }
            // SQ-0593: the character-cell pixel size we hand the game. EVERYTHING a
            // Glulx game sizes in pixels is derived from it — advent.blb's toolbar
            // among them — so a wrong value here makes the game's own artwork come out
            // the wrong size, with nothing downstream to blame. Two ways it can be
            // wrong, and this line tells them apart: with no image protocol we fall
            // back to a hardcoded 8x16 regardless of the real font, and where there IS
            // one the Picker reports LOGICAL points, which on a 2x HiDPI display are
            // half the device pixels the terminal actually paints (the same mismatch
            // that keeps pixel-precise mouse reporting switched off, see startup.rs).
            // Compare `implies` against the real window size to tell which.
            let (raw_w, raw_h, src) = match state.game_picker.as_ref() {
                Some(p) => {
                    let f = p.font_size();
                    (f.width as u32, f.height as u32, "terminal-reported")
                }
                None => (8, 16, "FALLBACK — no image protocol, real font size unknown"),
            };
            // What the game actually sees, after SQ-0593 scaling. Reporting the raw
            // terminal value alone was misleading the moment the divisor existed.
            let (cw, ch) = state.config.glk_pixel_scale.apply((raw_w, raw_h));
            out.push(format!(
                "  cell size: terminal says {raw_w}x{raw_h} px ({src}); glk_pixel_scale \
                 → game sees {cw}x{ch}; pane {}x{} cells implies {}x{} px to the game",
                story_rect.width, story_rect.height,
                cw * story_rect.width as u32, ch * story_rect.height as u32
            ));
            let encodes = state.graphics_render.borrow().band_encodes;
            out.push(format!("  band uploads since launch: {encodes}"));
            let bands = state.graphics_render.borrow().band_log.clone();
            for b in bands {
                out.push(format!("  {b}"));
            }
            if !history.is_empty() {
                out.push(format!("  recent render paths (oldest first): {}", history.join(" · ")));
            }
            for line in &out {
                state.push_transcript_internal(line, TranscriptKind::Meta);
            }
            // …and the same text to a file, because the on-screen copy cannot be
            // copied: a v6 pane is drawn out of kitty unicode placeholder glyphs, and
            // a terminal selection over the dump takes them with it — the user's paste
            // came back placeholder-dense and truncated mid-field (SQ-0756). The log
            // is readable from another terminal while the game is still running.
            let msg = match app::export::append_window_dump(&state.config.user_dir, &out) {
                Ok(p) => format!("  [dump appended to {} — copy it from there, not off the screen]", crate::abbreviate_home(&p)),
                Err(e) => format!("  [dump log failed: {e}]"),
            };
            state.push_transcript_internal(&msg, TranscriptKind::Meta);
        }
        SlashOutcome::DumpCells => {
            // The rendered cells, glyphs AND styling, as plain text (SQ-0761). Only
            // the file gets the grid: it is two lines per terminal row, so echoing it
            // into the transcript would scroll the very frame the next capture is
            // meant to describe — and the transcript cannot be copied off a v6 pane
            // anyway, which is the whole reason SQ-0756 started writing files.
            let snapshot = state.last_frame_cells.borrow().clone();
            let msg = match snapshot {
                None => "[dump-cells] no frame has been drawn yet — nothing to dump".to_string(),
                Some(frame) => {
                    let lines = frame.lines();
                    match app::export::append_cell_dump(&state.config.user_dir, &lines) {
                        Ok(p) => format!(
                            "[dump-cells] {} rows x {} cols of glyphs + styling appended to {} \
                             — copy it from there, not off the screen",
                            frame.buf.area.height,
                            frame.buf.area.width,
                            crate::abbreviate_home(&p)
                        ),
                        Err(e) => format!("[dump-cells] log failed: {e}"),
                    }
                }
            };
            state.push_transcript_internal(&msg, TranscriptKind::Meta);
        }
        SlashOutcome::ToggleDebug => toggle_debug(state, session),
        SlashOutcome::DumpNotifications => {
            let history = state.notifications.history().to_vec();
            state.push_transcript_internal("[notifications]", TranscriptKind::Meta);
            if history.is_empty() {
                state.push_transcript_internal("  (none)", TranscriptKind::Meta);
            } else {
                for line in history {
                    state.push_transcript_internal(&format!("  {line}"), TranscriptKind::Meta);
                }
            }
        }
        SlashOutcome::PlaySound(None) => {
            for line in app::state::format_sound_resource_list(state.sound_blorb.as_ref()) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::PlaySound(Some(n)) => {
            let mut report = app::state::PlaySoundReport {
                number: n,
                enable_sound: state.config.enable_sound,
                backend_present: state.audio.is_some(),
                blorb_present: state.sound_blorb.is_some(),
                ..Default::default()
            };
            if let Some(blorb) = &state.sound_blorb {
                if let Some((bytes, kind)) = blorb.sound(n) {
                    report.resource = Some((kind, bytes.len()));
                    if let Some(fmt) = app::state::sound_kind_to_format(kind) {
                        report.format = Some(fmt);
                        if let Some(backend) = state.audio.as_mut() {
                            report.sound_id = backend.play_sample(bytes, fmt, 8, 1);
                        }
                    }
                }
            }
            for line in app::state::format_play_sound_report(&report) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::Save(name_opt) => match name_opt {
            Some(ref name) => match app::persist_files::named_save_path(game_dir, name) {
                // SQ-0648: `/save <name>` clobbering an existing file with no
                // warning is exactly the bug — including a cross-name slugify
                // collision, which is why the prompt names the EXISTING
                // save's display name rather than echoing what was just typed.
                Ok(path) => match app::persist_files::existing_save_display_name(&path) {
                    Some(existing_name) => {
                        state.overlays.confirm_overwrite_save = Some(app::state::ConfirmOverwriteSave {
                            path,
                            existing_name,
                            pending: app::state::PendingOverwrite::Slash(name.clone()),
                        });
                        state.overlays.dialog_focus = 1; // Cancel default
                    }
                    None => {
                        let result = write_named_save(game_dir, ifid, name, mapper, session, state);
                        apply_slash_save_result(result, session, state);
                    }
                },
                Err(e) => state.set_status(format!("save failed: {}", e)),
            },
            None => {
                // The default archive slot is the auto/quick-save equivalent —
                // never a name the player typed — so it never prompts (SQ-0648).
                // SQ-0588: the display list travels with this save too — this is
                // the interactive Save State path, and an archive written
                // without it restores art that can never be recoloured.
                let (v6_pics, v6_display, v6_diags) = crate::engine_helpers::v6_save_payload(&mut *session);
                for d in &v6_diags { state.note_v6_save(d); }
                let (location, score) = crate::engine_helpers::save_summary(&*session, state);
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
                    trigger: app::archive::SaveTrigger::HostState,
                };
                let result = save_archive_meta_pics(arc_file, &*mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para, &state.transcript_images, &state.history, &state.command_history, &v6_pics, v6_display.as_ref())
                    .map(|()| "saved".to_string())
                    .map_err(|e| format!("save failed: {}", e));
                apply_slash_save_result(result, session, state);
            }
        },
        SlashOutcome::Load(name_opt) => {
            // Named-slot load or default archive load. Named slots may be a
            // .babelmap Save State or a .qzl game save (SQ-0227 Task 3).
            let archive_to_load = match name_opt {
                None => Some(arc_file.to_path_buf()),
                Some(ref name) => {
                    // Find the first named save whose display name matches.
                    let saves = combined_saves(game_dir);
                    saves.into_iter()
                        .find(|e| !e.is_default && e.name.to_lowercase() == name.to_lowercase())
                        .map(|e| e.path)
                }
            };
            match archive_to_load {
                None => {
                    state.set_status("load failed: no save found with that name");
                }
                Some(ref path) => {
                    let restore_outcome = restore_from_file(path, &mut *session);
                    app::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("restore_state({})", path.display()));
                    match restore_outcome {
                        Ok(RestoreOutcome::DescriptorCompleted(ac)) => {
                            // An in-game @save archive carries the whole session
                            // alongside its game bytes (SQ-0531); a bare .qzl has
                            // nothing but the bytes.
                            if let Some(ac) = ac {
                                apply_archive_state(*ac, &mut *session, mapper, state);
                            }
                            reobserve_location(state, mapper, &*session, map_rect);
                            state.set_status("restored");
                        }
                        Ok(RestoreOutcome::Resumed(ac)) => {
                            apply_archive_state(*ac, &mut *session, mapper, state);
                            reobserve_location(state, mapper, &*session, map_rect);
                            state.set_status("loaded");
                        }
                        Err(e) => state.set_status(format!("load failed: {}", e)),
                    }
                }
            }
        }
        SlashOutcome::LoadMap(path) => {
            let full = app::colors::expand_path(&path, &std::env::current_dir().unwrap_or_default());
            match load_map(&full) {
                Some(m) => {
                    *mapper = m;
                    state.bump_graph_gen(); // imported map replaced the graph → invalidate memo (SQ-0305)
                    state.set_viewed_layer(None);
                    // A whole new graph switches the active layer to whatever the loaded map's
                    // current room sits on — route it through the same layer-switch recenter as
                    // cycling/tab-clicking/peel/merge, so a loaded map with no `pos` on its
                    // current room (or none at all) still lands somewhere sane (SQ-0672).
                    app::input::recenter_for_active_layer(state, &mapper.graph);
                    state.set_status(format!("loaded map: {}", full.display()));
                }
                None => state.set_status(format!("load-map failed: {}", full.display())),
            }
        }
        SlashOutcome::Reset { map: reset_map, data: reset_data } => {
            // A key press (e.g. F5) or a bare `/reset-game` opens the confirmation
            // dialog with its map/data checkboxes; an explicit-token form
            // (`/reset-game map`, `data`, or both) acts immediately as typed.
            if from_key || (!reset_map && !reset_data) {
                apply_action(Action::ResetGame, state, mapper);
            } else {
                reset_game(session, mapper, state, story_bytes, story_path, game_dir, reset_map, reset_data);
                let mut status_msg = String::from("reset");
                if reset_map { status_msg.push_str(" (map cleared)"); }
                if reset_data { status_msg.push_str(" (data deleted)"); }
                state.set_status(&status_msg);
            }
        }
        SlashOutcome::Quit => {
            // A plain quit resolves the loop to Exit. Set it explicitly so a
            // prior `/quit-to-library` that opened (then was superseded by) this
            // path can't leave the target pointing at the library. (SQ-0435)
            state.exit_target = ExitTarget::Exit;
            if should_prompt_save_on_quit(state) {
                state.overlays.quit_dialog = true;
                state.overlays.dialog_focus = 0;
            } else {
                return true;
            }
        }
        SlashOutcome::QuitToLibrary => {
            // Only meaningful when launched from a directory (a picker exists).
            if !state.launched_from_library {
                state.set_status("No story library — launched with a single file");
            } else {
                // Return-to-library mirrors Quit's save handling, but resolves
                // the loop to the library instead of exiting. The break sites
                // read `state.exit_target`. (SQ-0435)
                state.exit_target = ExitTarget::Library;
                if should_prompt_save_on_quit(state) {
                    state.overlays.quit_dialog = true;
                    state.overlays.dialog_focus = 0;
                } else {
                    return true;
                }
            }
        }
        SlashOutcome::Search(q_opt) => {
            let query_to_run: Option<String> = match q_opt {
                Some(q) => Some(q),
                None => state.search_query.clone(),
            };
            match query_to_run {
                None => {
                    state.set_status("search: no previous search");
                }
                Some(query) => {
                    let count = state.run_search(&query, state.config.search.start_backward);
                    if count == 0 {
                        state.set_status("search: no matches");
                    } else {
                        state.set_status(format!("search: {} match{}", count, if count == 1 { "" } else { "es" }));
                        // Scroll to the current match.
                        let pos = state.search_matches[state.search_idx];
                        let total_vis = state.visible_transcript_indices().len();
                        let pane_rows = if story_rect.height > 0 {
                            story_rect.height as usize
                        } else {
                            24
                        };
                        state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                    }
                }
            }
        }
        SlashOutcome::Filter(arg) => {
            state.transcript_filter = match arg {
                TranscriptFilterArg::Both  => TranscriptFilter::Both,
                TranscriptFilterArg::Story => TranscriptFilter::Story,
                TranscriptFilterArg::Meta  => TranscriptFilter::Meta,
            };
            let label = match state.transcript_filter {
                TranscriptFilter::Both  => "both",
                TranscriptFilter::Story => "story",
                TranscriptFilter::Meta  => "meta",
            };
            // If a search is active, recompute it against the new filter
            // so highlights and the [i/N] hint stay consistent.
            if let Some(query) = state.search_query.clone() {
                let count = state.run_search(&query, state.config.search.start_backward);
                if count > 0 {
                    let pos = state.search_matches[state.search_idx];
                    let total_vis = state.visible_transcript_indices().len();
                    let pane_rows = if story_rect.height > 0 {
                        story_rect.height as usize
                    } else {
                        24
                    };
                    state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                }
            }
            state.set_status(format!("filter: {}", label));
        }
        SlashOutcome::Export(dest) => {
            let lines: Vec<String> = state
                .visible_transcript_indices()
                .into_iter()
                .map(|i| state.transcript[i].clone())
                .collect();
            match export_transcript(&lines, dest.as_deref(), game_dir) {
                Ok(path) => state.set_status(format!("exported: {}", path.display())),
                Err(e)   => state.set_status(format!("export failed: {}", e)),
            }
        }
        SlashOutcome::OpenHints => {
            let ud = state.config.user_dir.clone();
            open_hints(state, story_path, ifid, &ud);
        }
        SlashOutcome::HelpCommand(name) => {
            for line in slash::help_for_command(state.config.command_prefix, &name) {
                state.push_transcript_internal(&line, TranscriptKind::Meta);
            }
        }
        SlashOutcome::SetGameColours(opt) => {
            // Persist the per-game override (or clear it on `auto`), then recompute
            // the live look + honor precedence from disk (SQ-0318). reload_style
            // applies `per_game > garglk.ini > global`, so an explicit choice wins
            // and `auto` falls back to garglk/global.
            match app::styles::write_per_game_honor(game_dir, opt) {
                Ok(()) => {
                    let _ = app::reload::reload_style(state);
                    // Follow through to the running Z-machine: future set_colour
                    // ops and the Flags1 colour capability track the new setting.
                    // (Render-side gates handle colours already recorded.)
                    if let Some(zs) = session.as_any_mut().downcast_mut::<app::session::GameSession>() {
                        zs.machine.set_honor_game_colours(state.config.honor_game_colours);
                    }
                    let label = match opt {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "auto",
                    };
                    state.push_transcript_internal(
                        &format!(
                            "game colours: {label} (honor_game_colours = {})",
                            state.config.honor_game_colours
                        ),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-game-colours failed: {e}")),
            }
        }
        SlashOutcome::SetGameBorderless(opt) => {
            // Persist the per-game borderless override (or clear it on `auto`),
            // then apply it live: the running Glulx session relayouts so windows
            // abut (or regain their border gutters) immediately. (SQ-0341)
            match app::styles::write_per_game_borderless(game_dir, opt) {
                Ok(()) => {
                    let effective =
                        app::styles::read_per_game_borderless(game_dir).unwrap_or(false);
                    if let Some(gs) = session.as_any_mut().downcast_mut::<app::glulx_session::GlulxSession>() {
                        gs.set_borderless(effective);
                    }
                    let label = match opt {
                        Some(true) => "off (borderless)",
                        Some(false) => "on",
                        None => "auto (on)",
                    };
                    state.push_transcript_internal(
                        &format!("window borders: {label}"),
                        TranscriptKind::Meta,
                    );
                }
                Err(e) => state.set_status(format!("set-game-borders failed: {e}")),
            }
        }
        SlashOutcome::SetV6Render(opt) => {
            // Live, session-only switch for testing the three v6 looks; the
            // config screen remains the persistence path. Bare cycles.
            use app::config::V6RenderMode;
            let next = opt.unwrap_or(match state.config.v6_render {
                V6RenderMode::Hybrid => V6RenderMode::Raster,
                V6RenderMode::Raster => V6RenderMode::Frameless,
                V6RenderMode::Frameless => V6RenderMode::Hybrid,
            });
            state.config.v6_render = next;
            let label = match next {
                V6RenderMode::Hybrid => "hybrid",
                V6RenderMode::Raster => "raster",
                V6RenderMode::Frameless => "frameless",
            };
            state.push_transcript_internal(
                &format!("v6 render: {label} (session only)"),
                TranscriptKind::Meta,
            );
        }
        SlashOutcome::Trace(arg) => {
            match arg {
                None => {
                    state.set_status(format!("[trace: {}]", state.config.trace.active_list()));
                }
                Some(list) => {
                    let (sections, unknown) = app::trace::TraceSections::parse(&list);
                    state.config.trace = sections;
                    session.set_trace_screen(sections.screen);
                    if sections.any() && !unknown.is_empty() {
                        state.set_status(format!("[trace: {} — ignored: {}]", sections.active_list(), unknown.join(",")));
                    } else if !unknown.is_empty() {
                        state.set_status(format!("[trace: unknown section(s): {}]", unknown.join(",")));
                    } else {
                        state.set_status(format!("[trace: {}]", sections.active_list()));
                    }
                }
            }
        }
    }
    false
}

/// Write a named Save State via the slash `/save <name>` path (SQ-0648).
///
/// Factored out of the `SlashOutcome::Save(Some(name))` arm so the
/// overwrite-confirm resolver (`main.rs`'s `OverlayAct::ConfirmOverwrite`
/// handler, reached once the player answers the prompt) can perform the SAME
/// write without re-deriving it. Always writes — the existence check that
/// decides whether to prompt lives in the caller, not here.
pub(crate) fn write_named_save(
    game_dir: &std::path::Path,
    ifid: &str,
    name: &str,
    mapper: &Mapper,
    session: &mut dyn Engine,
    state: &mut AppState,
) -> Result<String, String> {
    // SQ-0588: the display list travels with every host save — an archive
    // written without it restores art that can never be recoloured.
    let (v6_pics, v6_display, v6_diags) = crate::engine_helpers::v6_save_payload(session);
    for d in &v6_diags { state.note_v6_save(d); }
    let (location, score) = crate::engine_helpers::save_summary(&*session, state);
    save_named(
        game_dir, ifid, name, app::archive::SaveTrigger::HostState, mapper, &session.save_state(),
        zvm_session_opt(&*session).map(|z| &z.machine.screen), &v6_pics, v6_display.as_ref(),
        session.aux_data(), state.turns, location, score, &state.transcript, &state.transcript_kinds,
        &state.transcript_runs, &state.transcript_para, &state.transcript_images,
    )
        .map(|()| format!("saved as \"{}\"", name))
        .map_err(|e| format!("save failed: {}", e))
}

/// Apply the result of a slash-path save write (named or default archive) to
/// `state`: clear the unsaved-progress flag and post the status line on
/// success, trace it, or just post the error on failure. Shared by the direct
/// write and the overwrite-confirm resolver (SQ-0648).
pub(crate) fn apply_slash_save_result(result: Result<String, String>, session: &mut dyn Engine, state: &mut AppState) {
    match result {
        Ok(msg) => {
            // Progress is now captured in a Save State — quitting is safe.
            state.unsaved_progress = false;
            if state.config.trace.hostio {
                app::trace::hostio(&state.config.user_dir, true, format!("save_state({} bytes)", session.save_state().bytes.len()));
            }
            state.set_status(msg);
        }
        Err(e) => state.set_status(e),
    }
}

/// Toggle the Z-machine debug inspector pane: activate it at the engine's current PC
/// (focusing the region), or deactivate it (map returns, focus back to the game).
/// Factored out of the `ToggleDebug` arm so it's testable without a full
/// `dispatch_slash_outcome` call.
fn toggle_debug(state: &mut AppState, session: &mut dyn Engine) {
    if state.debug.is_some() {
        state.debug = None;
        state.focus = Focus::Game;
        session.set_debug_trace(false);
    } else if session.debugger().is_some() {
        session.set_debug_trace(true);
        // Seed cumulative coverage from a prior `--debug` run's sidecar so a casual
        // `/debug` immediately shows the blue "ever executed" lines (SQ-0449).
        // Idempotent — re-seeding an already-populated set is harmless.
        let loaded = app::pcset_store::read_pcs(&state.game_dir);
        if !loaded.is_empty() {
            session.seed_executed_pcs(&loaded);
        }
        let dbg = session.debugger().expect("checked above");
        let mut panel = app::debug_panel::DebugPanelState::new(dbg.pc());
        panel.apply_engine_layout(dbg);
        panel.refresh(dbg);
        state.debug = Some(panel);
        state.focus = Focus::Map;
    } else {
        state.push_transcript_internal(
            "debugger not available for this engine", TranscriptKind::Meta);
    }
}

#[cfg(test)]
mod debug_dispatch_tests {
    use super::*;
    use app::engine::{Debugger, EngineError, EngineSave, KeyInput, LocationInfo, ScreenModel};
    use app::session::{InputKind, TurnResult};
    use std::any::Any;
    use std::collections::BTreeMap;

    /// A minimal `Engine` double: every method the debug-open path doesn't touch
    /// panics if called, so a wiring mistake fails loudly instead of silently.
    struct MockEngine {
        has_debugger: bool,
        aux: BTreeMap<String, Vec<u8>>,
    }

    struct MockDebugger;
    impl Debugger for MockDebugger {
        fn pc(&self) -> u32 { 0x1234 }
        fn disassemble(&self, _addr: u32, _lines: usize) -> Vec<String> { vec!["1234  nop".into()] }
        fn disassemble_raw(&self, _addr: u32, _lines: usize) -> Vec<String> { vec!["1234: b4  0OP:0x04".into()] }
        fn disassemble_basic(&self, _addr: u32, _lines: usize) -> Vec<String> { vec!["1234  loadw #0abc".into()] }
        fn next_instr(&self, addr: u32) -> u32 { addr + 1 }
        fn prev_instr(&self, addr: u32) -> u32 { addr.saturating_sub(1) }
        fn executed_pcs(&self) -> std::collections::HashSet<u32> { std::collections::HashSet::new() }
        fn stack_lines(&self) -> Vec<String> { Vec::new() }
        fn eval_stack_lines(&self) -> Vec<String> { Vec::new() }
        fn locals_lines(&self) -> Vec<String> { Vec::new() }
        fn globals_lines(&self) -> Vec<String> { Vec::new() }
        fn object_tree_lines(&self) -> Vec<String> { Vec::new() }
        fn dictionary_lines(&self) -> Vec<String> { Vec::new() }
        fn memory_hex(&self, _addr: u32, _rows: usize) -> Vec<String> { Vec::new() }
        fn memory_len(&self) -> u32 { 0x10000 }
        fn object_detail(&self, _obj: u16) -> Vec<String> { Vec::new() }
        fn frame_locals(&self, _idx: usize) -> Vec<String> { Vec::new() }
        fn var_value(&self, _var: u8) -> Option<u16> { None }
    }

    impl Engine for MockEngine {
        fn submit(&mut self, _command: &str) -> TurnResult { unimplemented!() }
        fn submit_key(&mut self, _key: KeyInput) -> Option<TurnResult> { unimplemented!() }
        fn take_transcript(&mut self) -> String { unimplemented!() }
        fn pending_input(&self) -> InputKind { InputKind::Line }
        fn resume_save(&mut self, _wrote_ok: bool) -> TurnResult { unimplemented!() }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> TurnResult { unimplemented!() }
        fn has_quit(&self) -> bool { false }
        fn screen(&self) -> ScreenModel { unimplemented!() }
        fn save_state(&self) -> EngineSave { EngineSave::new("mock", 1, Vec::new()) }
        fn restore_state(&mut self, _save: &EngineSave) -> Result<(), EngineError> { Ok(()) }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), EngineError> { Ok(()) }
        fn aux_data(&self) -> &BTreeMap<String, Vec<u8>> { &self.aux }
        fn set_aux_data(&mut self, data: BTreeMap<String, Vec<u8>>) { self.aux = data; }
        fn aux_dirty(&self) -> bool { false }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<LocationInfo> { None }
        fn as_any(&self) -> &dyn Any { self }
        fn as_any_mut(&mut self) -> &mut dyn Any { self }
        fn debugger(&self) -> Option<&dyn Debugger> {
            if self.has_debugger { Some(&MockDebugger) } else { None }
        }
    }

    #[test]
    fn toggle_debug_opens_panel_when_engine_has_debugger() {
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: true, aux: BTreeMap::new() };
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_some());
        let panel = state.debug.as_ref().unwrap();
        assert_eq!(panel.disasm_addr, 0x1234);
        assert_eq!(state.focus, Focus::Map);
    }

    #[test]
    fn toggle_debug_closes_panel_when_already_open() {
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: true, aux: BTreeMap::new() };
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_some());
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_none());
        assert_eq!(state.focus, Focus::Game);
    }

    #[test]
    fn toggle_debug_reports_when_engine_has_no_debugger() {
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        toggle_debug(&mut state, &mut engine);
        assert!(state.debug.is_none());
        assert!(state.transcript.iter().any(|l| l.contains("debugger not available")));
    }

    /// Drive `dispatch_slash_outcome` for a quit-like outcome against a minimal
    /// environment (the `QuitToLibrary`/`Quit` paths touch only `state`), and
    /// return the "should break" signal. (SQ-0435)
    fn dispatch_quit_like(state: &mut AppState, outcome: SlashOutcome) -> bool {
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let dir = std::path::Path::new("/tmp/babelmap-sq0435-test");
        dispatch_slash_outcome(
            outcome, state, &mut mapper, &mut engine, &mut style_watcher,
            dir, "IFIDTEST", dir, &[], dir,
            Rect::default(), Rect::default(), false,
        )
    }

    #[test]
    fn quit_to_library_without_library_sets_status_and_does_not_break() {
        let mut state = AppState::default();
        state.launched_from_library = false;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::QuitToLibrary);
        assert!(!should_break, "no library → the loop must not break");
        assert_eq!(state.exit_target, ExitTarget::Exit, "target must stay Exit");
        assert!(!state.overlays.quit_dialog, "no save prompt should open");
        assert!(
            state.notifications.history().iter().any(|m| m.contains("No story library")),
            "a status explaining the missing library should be shown"
        );
    }

    #[test]
    fn quit_to_library_with_library_and_no_unsaved_progress_breaks_to_library() {
        let mut state = AppState::default();
        state.launched_from_library = true;
        // Default config: auto_save off, no unsaved progress → no save prompt.
        state.unsaved_progress = false;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::QuitToLibrary);
        assert!(should_break, "no unsaved progress → break straight to the library");
        assert_eq!(state.exit_target, ExitTarget::Library, "target must be Library");
        assert!(!state.overlays.quit_dialog, "no save prompt with no unsaved progress");
    }

    #[test]
    fn quit_to_library_with_unsaved_progress_opens_prompt_targeting_library() {
        let mut state = AppState::default();
        state.launched_from_library = true;
        // Force the save prompt: prompt_save_on_quit defaults on; add progress.
        state.config.auto_save = false;
        state.config.prompt_save_on_quit = true;
        state.unsaved_progress = true;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::QuitToLibrary);
        assert!(!should_break, "opening the save prompt does not break yet");
        assert!(state.overlays.quit_dialog, "the save prompt should open");
        assert_eq!(state.exit_target, ExitTarget::Library, "prompt resolution must target Library");
    }

    #[test]
    fn quit_sets_exit_target_to_exit() {
        let mut state = AppState::default();
        // Even after a prior quit-to-library set the target, a plain Quit resets it.
        state.launched_from_library = true;
        state.exit_target = ExitTarget::Library;
        state.unsaved_progress = false;
        let should_break = dispatch_quit_like(&mut state, SlashOutcome::Quit);
        assert!(should_break, "no unsaved progress → quit breaks immediately");
        assert_eq!(state.exit_target, ExitTarget::Exit, "quit must resolve to Exit");
    }

    // ── SQ-0648: overwrite confirmation on the slash `/save <name>` path ───────

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bm-sq0648-slash-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A save-as target that already exists must open the overwrite-confirm
    /// overlay INSTEAD of writing — same rule as the dialog path, reached here
    /// through `/save <name>`. The prompt names the EXISTING save, so a
    /// cross-name slugify collision ("Before Troll" / "before, troll!" both
    /// land on `before-troll.babelmap`) is visible rather than reading as a
    /// harmless same-name re-save.
    #[test]
    fn slash_save_named_prompts_before_overwriting_an_existing_target() {
        let dir = temp_dir("prompt");
        let mut mapper = Mapper::default();

        // Seed "Before Troll" through the very writer `/save` itself uses.
        let mut seed_engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut seed_state = AppState::default();
        super::write_named_save(&dir, "IFIDTEST", "Before Troll", &mapper, &mut seed_engine, &mut seed_state)
            .expect("seed write succeeds");
        let path = dir.join("before-troll.babelmap");
        let original_bytes = std::fs::read(&path).expect("seed archive written");

        // `/save "before, troll!"` — a DIFFERENT typed name, same target file.
        let mut state = AppState::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let arc_file = dir.join("default.babelmap");
        let should_break = dispatch_slash_outcome(
            SlashOutcome::Save(Some("before, troll!".to_string())),
            &mut state, &mut mapper, &mut engine, &mut style_watcher,
            &dir, "IFIDTEST", &arc_file, &[], &dir,
            Rect::default(), Rect::default(), false,
        );
        assert!(!should_break);

        // Nothing was written — byte-identical to the seed.
        let after_bytes = std::fs::read(&path).expect("archive still there");
        assert_eq!(after_bytes, original_bytes, "no write until the overwrite is confirmed");

        let pending = state.overlays.confirm_overwrite_save.as_ref().expect("confirm overlay opens");
        assert_eq!(pending.path, path);
        assert_eq!(
            pending.existing_name, "Before Troll",
            "the prompt must name the save ALREADY there, not the name just typed"
        );
        assert_eq!(pending.pending, app::state::PendingOverwrite::Slash("before, troll!".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `write_named_save` is exactly what the run loop's
    /// `OverlayAct::ConfirmOverwrite(true)` handler calls once the player
    /// confirms — calling it directly here mirrors that resumed write and
    /// proves it actually replaces the file.
    #[test]
    fn slash_write_named_save_overwrites_when_the_overwrite_is_confirmed() {
        let dir = temp_dir("confirm");
        let mapper = Mapper::default();

        let mut seed_engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut seed_state = AppState::default();
        super::write_named_save(&dir, "IFIDTEST", "Before Troll", &mapper, &mut seed_engine, &mut seed_state)
            .expect("seed write succeeds");
        let path = dir.join("before-troll.babelmap");
        let original_bytes = std::fs::read(&path).expect("seed archive written");

        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut state = AppState::default();
        let result = super::write_named_save(&dir, "IFIDTEST", "before, troll!", &mapper, &mut engine, &mut state);
        assert!(result.is_ok(), "{result:?}");

        let new_bytes = std::fs::read(&path).expect("archive overwritten");
        assert_ne!(new_bytes, original_bytes, "the confirmed overwrite actually wrote");
        let meta = app::archive::read_archive_meta(&path).expect("meta");
        assert_eq!(meta.name.as_deref(), Some("before, troll!"), "the file now belongs to the new name");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The default archive slot (`/save` with no name) is the quick-save
    /// equivalent, never a name the player typed — it must keep overwriting
    /// silently even when the slot already holds an earlier save, exactly like
    /// the per-turn auto-save (SQ-0648).
    #[test]
    fn slash_save_default_archive_never_prompts_even_when_it_already_exists() {
        let dir = temp_dir("default-slot");
        let arc_file = dir.join("default.babelmap");

        let seed_meta = app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 1, saved_at: String::new(), location: None, score: None,
            trigger: app::archive::SaveTrigger::HostState,
        };
        app::archive::save_archive_meta(
            &arc_file, &Mapper::default(), &EngineSave::new("mock", 1, vec![1, 2, 3]), None,
            &BTreeMap::new(), seed_meta, &[], &[], &[], &[], &[], &[],
        ).expect("seed default.babelmap");
        let before = std::fs::read(&arc_file).expect("seed archive written");

        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        let mut engine = MockEngine { has_debugger: false, aux: BTreeMap::new() };
        let mut style_watcher: Option<app::watch::StyleWatcher> = None;
        let should_break = dispatch_slash_outcome(
            SlashOutcome::Save(None),
            &mut state, &mut mapper, &mut engine, &mut style_watcher,
            &dir, "IFIDTEST", &arc_file, &[], &dir,
            Rect::default(), Rect::default(), false,
        );
        assert!(!should_break);
        assert!(
            state.overlays.confirm_overwrite_save.is_none(),
            "the default archive slot must never open the overwrite-confirm overlay"
        );
        let after = std::fs::read(&arc_file).expect("archive still there");
        assert_ne!(after, before, "it wrote silently, straight over the existing slot");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
