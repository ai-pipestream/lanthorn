//! SlashOutcome side-effect dispatch: the single switch that applies a parsed
//! `SlashOutcome` (from typed input or a key binding) against the live app
//! state, engine, and mapper. Extracted verbatim from `main.rs` (SQ-0306) as a
//! pure move — no behavior change. Touches binary-only helpers (save/restore
//! plumbing, reset, hints, transcript export), all reached via `crate::`.

use app::archive::save_archive_meta;
use app::engine::Engine;
use app::export::export_transcript;
use app::input::{apply_action, Action};
use app::persist_files::{load_map, save_named};
use app::slash::{self, SlashOutcome, TranscriptFilterArg};
use app::state::{AppState, TranscriptFilter, TranscriptKind};
use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use crate::engine_helpers::{restore_from_file, zvm_session_opt, zvm_session_opt_mut, RestoreOutcome};
use crate::reset::reset_game;
use crate::{
    combined_saves, format_rfc3339, handle_map_export, map_pane_dims, open_hints, reobserve_location,
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
            if handle_map_export(&a, game_dir, mapper, state) {
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
            for (line, style_opt) in app::style::describe_scheme(&state.colors) {
                match (actual, style_opt) {
                    (true, Some(style)) => state.push_transcript_internal_styled(&line, TranscriptKind::Meta, style),
                    _ => state.push_transcript_internal(&line, TranscriptKind::Meta),
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
        SlashOutcome::Save(name_opt) => {
            // Named save or default archive save.
            let result = match name_opt {
                Some(ref name) => {
                    save_named(game_dir, ifid, name, &*mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), state.turns, &state.transcript, &state.transcript_kinds, &state.transcript_runs)
                        .map(|()| format!("saved as \"{}\"", name))
                        .map_err(|e| format!("save failed: {}", e))
                }
                None => {
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
                    };
                    save_archive_meta(arc_file, &*mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.history, &state.command_history)
                        .map(|()| "saved".to_string())
                        .map_err(|e| format!("save failed: {}", e))
                }
            };
            match result {
                Ok(msg) => {
                    // Progress is now captured in a Save State — quitting is safe.
                    state.unsaved_progress = false;
                    state.set_status(msg);
                }
                Err(e) => state.set_status(e),
            }
        }
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
                    match restore_from_file(path, &mut *session) {
                        Ok(RestoreOutcome::DescriptorCompleted) => {
                            reobserve_location(state, mapper, &*session, map_rect);
                            state.set_status("restored");
                        }
                        Ok(RestoreOutcome::Resumed(ac)) => {
                            if let Some(scr) = ac.screen.clone() {
                                if let Some(z) = zvm_session_opt_mut(&mut *session) { z.machine.screen = scr; }
                            }
                            if state.config.aux_storage != app::config::AuxStorage::Global {
                                session.set_aux_data(ac.aux.clone());
                            }
                            *mapper = ac.mapper;
                            state.transcript = ac.transcript;
                            state.clear_anchor = None;
                            state.transcript_kinds = ac.transcript_kinds;
                            state.transcript_runs = ac.transcript_runs;
                            state.reset_transcript_sidecars();
                            state.history = ac.history;
                            if !ac.command_history.is_empty() {
                                state.command_history = ac.command_history;
                            }
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
                    if let Some(rid) = mapper.graph.current() {
                        state.select_room(Some(rid));
                        if let Some(pos) = mapper.graph.room(rid).and_then(|r| r.pos) {
                            let (pw, ph) = map_pane_dims(map_rect);
                            state.recenter_on(pos, pw, ph);
                        }
                    }
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
            if should_prompt_save_on_quit(state) {
                state.quit_dialog = true;
                state.dialog_focus = 0;
            } else {
                return true;
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
    }
    false
}
