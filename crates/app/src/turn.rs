//! Turn lifecycle: apply a completed game turn to the UI + mapper, run post-turn
//! bookkeeping / persistence, and post-process resumed and game-driven turns.
//! Extracted verbatim from `main.rs` (SQ-0306) as a pure move — no behavior
//! change. Helper fns these rely on stay in `main.rs` (referenced via `crate::`);
//! the Wave 1 invariant calls (`graph_gen` bumps after `apply_turn`, transcript
//! generation bumps inside `push_*`) move intact inside the bodies.

use std::time::Duration;

use mapper::mapper::Mapper;
use ratatui::layout::Rect;

use app::archive::load_archive;
use app::engine::Engine;
use app::tidy::{cleanup_overlaps_layer_silent, tidy_layer_silent};
use app::session::{apply_turn, TurnResult};
use app::state::{AppState, SoundPulse, TidyJob, TidyKind, TranscriptKind};
use app::storage::default_state_path;

use crate::engine_helpers::{restore_error_msg, zvm_session_opt, zvm_session_opt_mut};
use crate::ingame_io::{open_filename_modal, open_ingame_saves};
use crate::{
    format_rfc3339, game_echoes_command, map_pane_dims, reobserve_location, save_archive_meta,
    should_bg_tidy, PaneRects,
};

/// Apply a completed game-turn `result` from a submitted command line: echo the
/// command, push its transcript, advance the mapper, run post-turn bookkeeping /
/// auto-save / background tidy, and recenter on the current room. Shared by the
/// normal `SubmitCommand` path and the terminator-key submit gate (SQ-0188).
/// Returns `true` if the app should exit after this turn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_command_turn(
    cmd: &str,
    result: TurnResult,
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &mut dyn Engine,
    game_dir: &std::path::Path,
    ifid: &str,
    arc_file: &std::path::Path,
    map_area: Rect,
    bg_tidy_counter: &mut u32,
) -> bool {
    if result.erase_lower { state.mark_screen_clear(); }
    // Some games echo the typed command themselves at the start of their turn
    // output (e.g. CounterfeitMonkey prints it back in bold). Adding our own echo
    // on top would show the command twice, so detect that and skip ours. Most
    // games don't self-echo, so they still get our echo below.
    let self_echo = game_echoes_command(&result.transcript, cmd);
    // When the game self-echoes AND we're inline with the `>` as the last line,
    // fold the game's echo onto that prompt line (below) so it reads `>look` at
    // the prompt, with the game's own styling — instead of a detached line.
    let merge_echo = self_echo && !state.config.command_bar && state.last_transcript_line_is_story();
    if self_echo {
        // Game provides the echo; add nothing of our own.
    } else if state.config.command_bar || !state.last_transcript_line_is_story() {
        // Command-bar mode, or inline mode where the game's `>` is NOT the last
        // line (e.g. a `/help` Meta dump intervened): echo on its own line so we
        // never corrupt non-prompt scrollback.
        state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
    } else {
        // Inline mode: the game's own `>` is already the last transcript line;
        // append the typed command so `>look` persists in scrollback.
        state.append_to_last_transcript_line(cmd);
    }
    let before_push = state.transcript.len();
    if result.transcript_elems.is_empty() {
        state.push_transcript_runs(&result.transcript, TranscriptKind::Story, &result.transcript_runs);
    } else {
        app::state::apply_transcript_elems(state, &result.transcript_elems);
    }
    if merge_echo && state.transcript.len() > before_push {
        // Fold the game's own echo (its first output line) onto the `>` prompt.
        // The game printed the echo in the default colour; preserve the current
        // page colours on the folded line rather than resetting it to the theme.
        let prevailing = state.prevailing_run_colour_before(before_push);
        state.merge_line_into_previous(before_push);
        if let Some((fg, bg)) = prevailing {
            state.fill_line_default_colours(before_push - 1, fg, bg);
        }
    }
    apply_turn_events(state, &result);
    flush_screen_trace(&state.config.user_dir, session, state.config.trace.screen);
    flush_v6_trace(&state.config.user_dir, session, state.config.trace.v6);
    // [more] pager (SQ-0404): arm the pager for this command's output only when
    // the game is now awaiting a normal LINE command — never on a screen clear
    // (menu takeover) or when it switched to char/event input (a keypress menu or
    // "press any key"), where a keypress must reach the game, not page the pane.
    // The next render measures the rows added and engages if it overflowed.
    if !result.erase_lower && session.pending_input() == app::session::InputKind::Line {
        state.pager.arm(state.last_transcript_total_rows);
    } else {
        state.pager.disarm();
    }
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }

    // Capture room + connection counts before apply_turn, to detect
    // whether THIS turn actually changed the graph (a non-mutating
    // command like "look" leaves both unchanged).
    let rooms_before = mapper.graph.rooms().count();
    let conns_before = mapper.graph.connections().len();

    apply_turn(mapper, cmd, &result);

    // Bump the graph generation ONLY when the turn actually changed the map's
    // routed geometry (a room or connection added/removed). This invalidates the
    // map render memo (forcing a re-route) and marks any in-flight tidy result
    // stale. A step between already-placed rooms changes neither, so it must NOT
    // bump — otherwise every step re-routes the whole map and pauses gameplay on
    // large explored maps (SQ-0378). The current-room highlight and any in-place
    // relabel are refreshed cheaply at draw time (see `cached_map_render`), with
    // no re-route.
    if mapper.graph.rooms().count() != rooms_before
        || mapper.graph.connections().len() != conns_before
    {
        state.graph_gen = state.graph_gen.wrapping_add(1);
    }

    // Game-initiated (v4+) save/restore: open the saves dialog in
    // in-game mode and defer auto-save/history capture until the
    // resume completes (the turn is still in flight).
    if let Some(io) = result.pending_io {
        open_ingame_saves(io, game_dir, state);
        return false;
    }

    // Game create_by_prompt: open the filename modal and defer bookkeeping until the
    // resume completes (the turn is still in flight, like the save/restore path).
    if let Some(req) = session.pending_filename() {
        open_filename_modal(req, &*session, state);
        return false;
    }

    // ── Post-turn bookkeeping (history / inventory / auto-save) ──
    post_turn_bookkeeping(
        state, mapper, &*session, &result, cmd,
        rooms_before, conns_before, ifid, arc_file,
    );
    persist_aux_after_turn(session, state, game_dir);
    persist_vfs_after_turn(session, state, game_dir);

    // Background map maintenance: a geometry change (new room/connection) is the
    // ONLY thing that can require re-layout, so all of it runs on a worker thread —
    // NO routing or overlap cleanup ever touches the interpreter thread (SQ-0379).
    // A bare "look"/"inventory" changes no geometry, so it schedules nothing and
    // pays nothing here. The `background_tidy` setting decides whether the job is a
    // FULL relayout (aesthetics) or overlap-cleanup-only; cleanup runs either way,
    // so the map is never left overlapping regardless of the setting. Only runs in
    // Auto layout mode.
    //
    // Abort-and-replace on rapid movement: if a job is already in flight when the
    // next room arrives, we DROP its handle (the thread detaches and finishes into
    // the void — its result is discarded by the gen check) and spawn a fresh job for
    // the new state, so the latest geometry always wins immediately instead of
    // queueing behind a stale computation. Threads can't be force-killed, but only
    // the newest job is tracked/joined; concurrent stragglers are bounded by the
    // input rate. (This job is spawned once per command; the per-frame render worker
    // still coalesces — see `spawn_render_job` — to avoid a per-frame thread storm.)
    if mapper.mode == mapper::layout::LayoutMode::Auto {
        let new_room = mapper.graph.rooms().count() > rooms_before;
        let new_conn = mapper.graph.connections().len() > conns_before;
        let changed = new_room || new_conn;
        if changed {
            let active_layer = state.active_layer(&mapper.graph);
            // Overlap/distortion signal → decides FULL relayout vs. cleanup-only.
            let cells = mapper::layout::occupied_cells_in_layer(&mapper.graph, active_layer);
            let total_rooms = mapper.graph.rooms_in_layer(active_layer).len();
            let has_overlap = cells.len() < total_rooms;
            let has_distorted = mapper.graph.connections().iter().any(|c| {
                c.distorted
                    && mapper.graph.layer_of(c.origin) == active_layer
                    && mapper.graph.layer_of(c.dest) == active_layer
            });
            let overlap = has_overlap || has_distorted;
            let full = should_bg_tidy(
                state.config.background_tidy, new_room, overlap, changed, bg_tidy_counter,
            );
            let kind = if full { TidyKind::Full } else { TidyKind::Cleanup };
            let graph_clone = mapper.graph.clone();
            let gen = state.graph_gen;
            let handle = std::thread::spawn(move || {
                let mut g = graph_clone;
                match kind {
                    TidyKind::Full => tidy_layer_silent(&mut g, active_layer),
                    TidyKind::Cleanup => cleanup_overlaps_layer_silent(&mut g, active_layer),
                }
                g
            });
            state.tidy_job = Some(TidyJob {
                handle,
                layer: active_layer,
                gen,
                started: std::time::Instant::now(),
                kind,
            });
        }
    }

    // Clear any manual layer browse override so the view follows the player.
    state.set_viewed_layer(None);

    // Select and recenter on the current room.
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        if let Some(room) = mapper.graph.room(rid) {
            if let Some(pos) = room.pos {
                let (pw, ph) = map_pane_dims(map_area);
                state.recenter_on(pos, pw, ph);
            }
        }
    }

    // Scott Adams games auto-terminate via the VM's quit (opcode 63) on win or
    // loss. Rather than let a clean Scott quit exit the whole app, keep it alive
    // and raise the game-over dialog (the final message stays in the transcript
    // behind it). Every other engine keeps exiting on a clean quit.
    let should_exit = should_exit_on_turn(&result, state);
    let is_scott = crate::engine_helpers::engine_tag(session) == "scott";

    // If the debug inspector is open, refresh its snapshot from the VM state
    // this turn just produced (globals/objects/PC may have moved).
    if let Some(p) = &mut state.debug {
        if let Some(dbg) = session.debugger() {
            p.refresh(dbg);
        }
    }

    intercept_scott_game_over(should_exit, is_scott, state)
}

/// Fold a Scott clean quit into the game-over overlay. When the turn would exit
/// the app (`should_exit`) AND the engine is Scott, open the game-over dialog and
/// keep the app alive (return `false`). For every other case return `should_exit`
/// unchanged, so Z-machine/Glulx keep exiting on a clean `@quit`/`glk_exit`.
fn intercept_scott_game_over(should_exit: bool, is_scott: bool, state: &mut AppState) -> bool {
    if should_exit && is_scott {
        state.overlays.game_over = true;
        state.overlays.dialog_focus = 0;
        false
    } else {
        should_exit
    }
}

/// Post-turn bookkeeping shared by the normal `submit` path and the resumed
/// in-game save/restore path: opt-in rewind/replay capture, inventory tracking,
/// and per-turn auto-save. `rooms_before`/`conns_before` are the graph sizes
/// captured before this turn's `apply_turn` (to detect a map change). `cmd` is
/// the player's command (empty string for a resumed in-game I/O turn).
fn post_turn_bookkeeping(
    state: &mut AppState,
    mapper: &Mapper,
    session: &dyn Engine,
    result: &TurnResult,
    cmd: &str,
    rooms_before: usize,
    conns_before: usize,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    // ── Rewind/replay capture (opt-in) ────────────────────────────
    // Skip the quit turn: the VM has terminated, so its snapshot has
    // no replayable state — recording it just adds a junk final turn.
    if state.config.record_turn_history && !result.quit {
        let map_changed = mapper.graph.rooms().count() != rooms_before
            || mapper.graph.connections().len() != conns_before;
        app::history::record_turn(
            &mut state.history,
            state.turns,
            cmd,
            session.save_state().bytes,
            mapper,
            map_changed,
            &result.transcript,
        );
    }

    // ── Inventory tracking ────────────────────────────────────────
    {
        use app::inventory::{detect_player_obj, parse_inventory_output};

        let current_loc = session.current_location()
            .map(|s| s.number)
            .unwrap_or(0);

        if current_loc != 0 {
            // Objects whose parent is the current room, via the engine's
            // introspection (the same object-tree walk as before).
            let objects_here: std::collections::BTreeSet<u16> = session
                .introspect()
                .map(|i| i.children_of(current_loc))
                .unwrap_or_default();

            // Lock the player object. Prefer the reliable name-based
            // lookup (the object short-named "you"/"yourself"/… — present
            // in most games incl. v3 Zork as obj #30) so the inventory
            // panel reads the LIVE object tree from turn one and reflects
            // take/drop immediately. Fall back to the movement heuristic
            // for games whose player object isn't named.
            if state.player_obj.is_none() {
                state.player_obj = session.introspect().and_then(|i| i.player_object())
                    .or_else(|| detect_player_obj(
                        state.prev_location,
                        &state.prev_objects_here,
                        current_loc,
                        &objects_here,
                    ));
            }

            // Update tracking for next turn.
            state.prev_location = Some(current_loc);
            state.prev_objects_here = objects_here;
        }

        // If the submitted command was an inventory command, parse the output.
        let cmd_norm = cmd.trim().to_lowercase();
        if cmd_norm == "i" || cmd_norm == "inv" || cmd_norm == "inventory" {
            state.inventory_fallback = parse_inventory_output(&result.transcript);
        }
    }

    // Per-turn auto-save (when enabled). Non-fatal: failure is shown in the
    // transcript status line so the player is aware but the loop continues.
    // Engine-neutral: the save routes through Engine::save_state (Quetzal for
    // zvm, the gvm snapshot for Glulx); screen.json is written for zvm only.
    if state.config.auto_save {
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
        if let Err(e) = save_archive_meta(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para, &state.history, &state.command_history) {
            state.push_notice(&format!("[Auto-save failed: {}]", e));
        }
    }
}

/// Drain the engine's `screen` trace and, when `on`, append it to trace.log.
/// Always drains (so the buffer never grows while the section is off between a
/// runtime toggle). (trace feature)
pub(crate) fn flush_screen_trace(user_dir: &std::path::Path, session: &mut dyn Engine, on: bool) {
    let lines = session.take_screen_trace();
    if on {
        app::trace::write(user_dir, app::trace::Section::Screen, &lines);
    }
}

/// When `on` and the story is v6, append this turn's `v6` window/picture-canvas
/// state snapshot to trace.log. Unlike `flush_screen_trace`, there is no buffer
/// to drain — the snapshot reads live state directly — so this is skipped
/// entirely (no snapshot built) when the section is off. (trace feature)
pub(crate) fn flush_v6_trace(user_dir: &std::path::Path, session: &mut dyn Engine, on: bool) {
    if !on {
        return;
    }
    if let Some(lines) = session.v6_snapshot() {
        app::trace::write(user_dir, app::trace::Section::V6, &lines);
    }
}

/// After a turn, persist the VM's aux table if it changed.  Archive mode is
/// already covered by the per-turn auto-save (`save_archive_meta` embeds it);
/// global mode writes the per-game file here.  `Ask` opens the first-use
/// prompt dialog (Task 6) and leaves `aux_dirty` set for the dialog to resolve.
pub(crate) fn persist_aux_after_turn(
    session: &mut dyn Engine,
    state: &mut AppState,
    game_dir: &std::path::Path,
) {
    if !session.aux_dirty() {
        return;
    }
    match state.config.aux_storage {
        app::config::AuxStorage::Global => {
            let _ = app::aux_store::write_global_aux(game_dir, session.aux_data());
            session.clear_aux_dirty();
        }
        app::config::AuxStorage::Archive => {
            session.clear_aux_dirty(); // archive auto-save already embedded it
        }
        app::config::AuxStorage::Ask => {
            state.overlays.aux_prompt = true; // resolve in the dialog; leave aux_dirty set
            state.overlays.dialog_focus = 0;
        }
    }
}

/// Flush the Glulx Glk file VFS to its per-story sidecar when it changed this
/// turn. Dirty-gated; a no-op for the Z-machine (whose `vfs_dirty` default is
/// always false). Mirrors `persist_aux_after_turn`.
pub(crate) fn persist_vfs_after_turn(
    session: &mut dyn Engine,
    state: &AppState,
    game_dir: &std::path::Path,
) {
    if !session.vfs_dirty() {
        return;
    }
    let bytes = session.vfs_bytes();
    let _ = app::vfs_store::write_vfs(game_dir, &bytes);
    session.clear_vfs_dirty();
    app::trace::hostio(&state.config.user_dir, state.config.trace.hostio,
        format!("vfs_write({} bytes)", bytes.len()));
}

/// Post-process a TurnResult produced by `session.resume_*`: render output,
/// re-observe the location, recenter, run post-turn bookkeeping, and record a
/// *chained* in-game I/O if the resume itself suspended on another
/// `@save`/`@restore`. Returns true if the app should quit. Mirrors the
/// post-turn block in the `submit` path.
pub(crate) fn finish_resumed_turn(
    result: TurnResult,
    mapper: &mut Mapper,
    state: &mut AppState,
    session: &dyn Engine,
    game_dir: &std::path::Path,
    ifid: &str,
    map_area: Rect,
) -> bool {
    state.push_transcript(&result.transcript);
    apply_turn_events(state, &result);
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }
    // Capture graph sizes before apply_turn so bookkeeping can detect a change.
    let rooms_before = mapper.graph.rooms().count();
    let conns_before = mapper.graph.connections().len();
    apply_turn(mapper, "", &result);
    state.graph_gen = state.graph_gen.wrapping_add(1);
    state.set_viewed_layer(None);
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        if let Some(room) = mapper.graph.room(rid) {
            if let Some(pos) = room.pos {
                let (pw, ph) = map_pane_dims(map_area);
                state.recenter_on(pos, pw, ph);
            }
        }
    }
    // Captured before the partial move below (of `result.pending_io`) makes a
    // subsequent whole-struct borrow of `result` a borrow-checker error.
    let should_exit = should_exit_on_turn(&result, state);
    // A chained request: the resumed turn suspended on another @save/@restore.
    // Mirror the submit path, which defers bookkeeping until the chain resolves;
    // run bookkeeping only when this turn finished without chaining.
    if let Some(io) = result.pending_io {
        state.ingame_io = Some(io);
    } else if let Some(req) = session.pending_filename() {
        // The resumed turn chained straight into a create_by_prompt.
        open_filename_modal(req, session, state);
    } else {
        let arc_file = default_state_path(game_dir);
        post_turn_bookkeeping(state, mapper, session, &result, "", rooms_before, conns_before, ifid, &arc_file);
    }
    should_exit
}

/// Apply a pending resume: restore the VM save, set transcript, re-observe location.
///
/// Mirrors the Action::RestoreGame path exactly (restore_quetzal, set transcript,
/// apply_turn to re-observe current room, set_viewed_layer(None), select_room, recenter).
pub(crate) fn apply_launch_resume(
    save: &app::engine::EngineSave,
    lines: Vec<String>,
    kinds: Vec<TranscriptKind>,
    screen: Option<zvm::screen::ScreenState>,
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    last_panes: &PaneRects,
    arc_file: &std::path::Path,
) {
    match session.restore_state(save) {
        Ok(()) => {
            // The resumed game's map is part of its archive state — load it alongside.
            if let Ok(ac) = load_archive(arc_file) {
                *mapper = ac.mapper;
                // Restore the turn counter from the same archive the map came from.
                // The launch-resume stash omits it, so without this the count would
                // reset to 0 on resume (SQ-0260) — mirrors the interactive restore.
                state.turns = ac.meta.turns;
            }
            // Reinstate the saved screen too (mirrors the auto-load path, zvm-only),
            // so a once-split game's upper window/status line shows after resuming.
            if let Some(scr) = screen {
                if let Some(z) = zvm_session_opt_mut(&mut *session) { z.machine.screen = scr; }
            }
            state.transcript = lines;
            state.clear_anchor = None;
            state.transcript_kinds = kinds;
            // The launch-resume stash carries no style runs; keep the parallel
            // vecs length-synced (unstyled, left/no-indent rows).
            state.transcript_runs = vec![Vec::new(); state.transcript.len()];
            state.transcript_para = vec![app::state::ParaFmt::default(); state.transcript.len()];
            state.reset_transcript_sidecars();
            // Re-observe current location (same as Action::RestoreGame).
            reobserve_location(state, mapper, &*session, last_panes.map);
            state.push_notice("[Game resumed from save.]");
        }
        Err(e) => {
            state.push_notice(&format!("[Resume failed: {}]", restore_error_msg(e)));
        }
    }
}

// ── Game-driven input helpers (char-mode keypress, timed-input interrupt) ──────

/// Append a gvm runtime fault (diagnostics + fault trace) to `user_dir/crash.log`.
/// A fault ends the game via a silent `Quit`, so this makes the failure durable
/// regardless of terminal state. IO errors are ignored (best-effort logging).
fn log_gvm_fault(user_dir: &std::path::Path, fault: &[String], diagnostics: &[String]) {
    use std::io::Write as _;
    let Ok(mut f) =
        std::fs::OpenOptions::new().create(true).append(true).open(user_dir.join("crash.log"))
    else {
        return;
    };
    let _ = writeln!(f, "\n=== gvm runtime fault (game halted) ===");
    for d in diagnostics {
        let _ = writeln!(f, "diag: {d}");
    }
    for line in fault {
        let _ = writeln!(f, "{line}");
    }
}

/// Whether a turn result should terminate the app: only a CLEAN game exit
/// (glk_exit) does. A VM fault halts the game but keeps the app alive.
fn should_exit_on_turn(result: &TurnResult, state: &AppState) -> bool {
    result.quit && result.fault.is_none() && !state.vm_halted
}

/// Route a turn's sound/diagnostic events: diagnostics become Warning transcript
/// lines; the latest beep arms a one-shot story-border pulse; the current room
/// name is tracked for the built-in location story rule.
fn apply_turn_events(state: &mut AppState, result: &TurnResult) {
    for line in &result.diagnostics {
        state.push_transcript_kind(line, app::state::TranscriptKind::Warning);
    }
    if let Some(lines) = &result.fault {
        let crash = state.colors.theme.get("transcript_crash").style;
        for line in lines {
            state.push_transcript_styled(line, app::state::TranscriptKind::Warning, crash);
        }
        state.push_transcript_styled("(game halted)", app::state::TranscriptKind::Warning, crash);
        // A gvm runtime fault ends the game via a silent Quit; if the app then
        // exits before this transcript is rendered, the error would vanish. Record
        // it durably so a "silent" crash always leaves a trace.
        log_gvm_fault(&state.config.user_dir, lines, &result.diagnostics);
        // Keep the app alive: a VM fault is not a clean glk_exit. The run loop's
        // exit checks all gate on `should_exit_on_turn`, which consults this flag.
        state.vm_halted = true;
        state.set_status("VM fault — the game has halted; you can review the map/transcript or quit.");
    }
    if let Some(kind) = result.sounds.iter().rev().find_map(|ev| match ev.number {
        1 => Some(app::state::BeepKind::High),
        2 => Some(app::state::BeepKind::Low),
        _ => None,
    }) {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
    // Audio is additive on top of the border pulse; gated inside play_turn_sounds.
    state.play_turn_sounds(&result.sounds);
    // Glulx Glk sound channels (empty for the Z-machine path).
    state.play_glulx_sound_ops(&result.glulx_sound_ops);
    state.loc_method = result.location_method.or(state.loc_method);
    // Retain the previous name when this turn has no location signal.
    if let Some(loc) = &result.location {
        state.current_room_name = Some(loc.name.clone());
    }
}

/// Apply a `TurnResult` produced by game-driven input that is not a full player
/// command submission — a char-mode (`read_char`) keypress or a timed-input
/// interrupt tick. Pushes transcript output (with style runs), routes
/// beep/location/diagnostic events, applies the mapper turn, opens a
/// game-initiated save/restore dialog if requested, and recenters on a location
/// change. Deliberately skips `post_turn_bookkeeping` (history/inventory/
/// auto-save): this is not a completed player turn. Returns `true` if the game
/// quit (the caller should break the event loop).
pub(crate) fn apply_game_driven_result(
    state: &mut AppState,
    mapper: &mut Mapper,
    result: &TurnResult,
    game_dir: &std::path::Path,
    map_area: Rect,
    session: &dyn Engine,
) -> bool {
    if result.erase_lower {
        // A game-driven screen clear is a menu redraw navigated by keystrokes —
        // Counterfeit Monkey's help menu clears the primary buffer and reprints
        // the whole menu on EVERY arrow press. Collapse the previous reprint by
        // truncating back to the prior clear anchor, so the reprints replace each
        // other instead of piling up invisibly in scrollback (hidden by the
        // SQ-0403 view-pin, then revealed when you exit the menu). (SQ-0407)
        if let Some(anchor) = state.clear_anchor {
            state.truncate_transcript(anchor);
        }
        state.mark_screen_clear();
    }
    if result.transcript_elems.is_empty() {
        state.push_transcript_runs(&result.transcript, TranscriptKind::Story, &result.transcript_runs);
    } else {
        app::state::apply_transcript_elems(state, &result.transcript_elems);
    }
    apply_turn_events(state, result);
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }
    // apply_turn: this input doesn't carry direction info (no text command to
    // parse), but we still observe any location change so the map stays in sync.
    let rooms_before = mapper.graph.rooms().count();
    let conns_before = mapper.graph.connections().len();
    apply_turn(mapper, "", result);
    // Game-initiated (v4+) save/restore: open the saves dialog in in-game mode
    // and defer the rest of the turn.
    if let Some(io) = result.pending_io {
        open_ingame_saves(io, game_dir, state);
        return false;
    }
    // Bump the graph generation ONLY when this game-driven turn actually changed
    // the routed geometry (a room/connection added). A char-input keypress —
    // menu navigation, a "press any key" prompt — changes nothing, so it must NOT
    // re-route the whole map on the main thread every keystroke (the Counterfeit
    // Monkey help-menu pause). Mirrors the line-input path's gate in
    // `finish_command_turn` (SQ-0378), which this path was missing. The current-
    // room highlight + recenter below still update cheaply without a re-route.
    // (SQ-0406)
    if mapper.graph.rooms().count() != rooms_before
        || mapper.graph.connections().len() != conns_before
    {
        state.graph_gen = state.graph_gen.wrapping_add(1);
    }
    // Select and recenter on the current room if it changed.
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        if let Some(room) = mapper.graph.room(rid) {
            if let Some(pos) = room.pos {
                let (pw, ph) = map_pane_dims(map_area);
                state.recenter_on(pos, pw, ph);
            }
        }
    }

    // If the debug inspector is open, refresh its snapshot from the VM state
    // this turn just produced (globals/objects/PC may have moved).
    if let Some(p) = &mut state.debug {
        if let Some(dbg) = session.debugger() {
            p.refresh(dbg);
        }
    }

    should_exit_on_turn(result, state)
}

/// Decide the timed-input deadline for this loop iteration. `should_arm` is true
/// while the game is awaiting timed input (honoring timers, no overlay covering
/// the pane, and a timed read pending). Arm ONCE at `now + interval` and KEEP the
/// existing deadline while still armed — re-arming every iteration would push the
/// deadline perpetually ahead of `now`, so `now >= deadline` could never become
/// true and the interrupt would never fire. Disarm (`None`) when not applicable;
/// the run loop also clears the deadline to `None` right after firing, so the next
/// armed iteration re-arms fresh at `now + interval`.
pub(crate) fn next_input_deadline(
    current: Option<std::time::Instant>,
    should_arm: bool,
    interval: Duration,
    now: std::time::Instant,
) -> Option<std::time::Instant> {
    if should_arm {
        Some(current.unwrap_or(now + interval))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    // ── screen-trace flush (drain-always, write-when-on) ────────────────────────

    /// A minimal `Engine` double whose `take_screen_trace` returns one line on
    /// its first call and is empty thereafter, so a test can observe both the
    /// write-when-on path and the always-drain (no regrowth) behavior. `v6` lets
    /// a test stand in for a v6-vs-non-v6 engine on `v6_snapshot`.
    struct TraceOnlyEngine {
        line: Option<String>,
        v6: Option<Vec<String>>,
    }

    impl app::engine::Engine for TraceOnlyEngine {
        fn submit(&mut self, _command: &str) -> app::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn submit_key(&mut self, _key: app::engine::KeyInput) -> Option<app::session::TurnResult> {
            unreachable!("not exercised by this test")
        }
        fn take_transcript(&mut self) -> String {
            String::new()
        }
        fn pending_input(&self) -> app::session::InputKind {
            unreachable!("not exercised by this test")
        }
        fn resume_save(&mut self, _wrote_ok: bool) -> app::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn resume_restore(&mut self, _data: Option<&[u8]>) -> app::session::TurnResult {
            unreachable!("not exercised by this test")
        }
        fn has_quit(&self) -> bool {
            false
        }
        fn screen(&self) -> app::engine::ScreenModel {
            unreachable!("not exercised by this test")
        }
        fn save_state(&self) -> app::engine::EngineSave {
            unreachable!("not exercised by this test")
        }
        fn restore_state(&mut self, _save: &app::engine::EngineSave) -> Result<(), app::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), app::engine::EngineError> {
            unreachable!("not exercised by this test")
        }
        fn take_screen_trace(&mut self) -> Vec<String> {
            self.line.take().into_iter().collect()
        }
        fn v6_snapshot(&self) -> Option<Vec<String>> {
            self.v6.clone()
        }
        fn aux_data(&self) -> &std::collections::BTreeMap<String, Vec<u8>> {
            unreachable!("not exercised by this test")
        }
        fn set_aux_data(&mut self, _data: std::collections::BTreeMap<String, Vec<u8>>) {
            unreachable!("not exercised by this test")
        }
        fn aux_dirty(&self) -> bool {
            false
        }
        fn clear_aux_dirty(&mut self) {}
        fn current_location(&self) -> Option<app::engine::LocationInfo> {
            None
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn flush_screen_trace_writes_when_on_and_drains() {
        let dir = std::env::temp_dir().join(format!("bm-flush-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut eng = TraceOnlyEngine { line: Some("@split_window(1)".to_string()), v6: None };

        super::flush_screen_trace(&dir, &mut eng, true);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.contains("[screen] "), "{body:?}");

        // Off → no further write, but the buffer must still be drained (no growth
        // while the section is toggled off between calls). Since take_screen_trace
        // is already empty after the first drain, a second flush is a no-op either
        // way; assert the log is unchanged.
        super::flush_screen_trace(&dir, &mut eng, false);
        let body2 = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert_eq!(body, body2, "off flush must not append");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── v6-trace flush (skip-when-off, snapshot-only-for-v6-engines) ────────────

    #[test]
    fn flush_v6_trace_writes_when_on_and_skips_when_off() {
        let dir = std::env::temp_dir().join(format!("bm-flush-v6-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut eng = TraceOnlyEngine {
            line: None,
            v6: Some(vec!["turn snapshot (current=7)".to_string()]),
        };

        // Off: nothing built, nothing written.
        super::flush_v6_trace(&dir, &mut eng, false);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.is_empty(), "off flush must not append: {body:?}");

        // On: the snapshot lines land, tagged.
        super::flush_v6_trace(&dir, &mut eng, true);
        let body2 = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(
            body2.contains("[v6]") && body2.contains("turn snapshot (current=7)"),
            "{body2:?}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flush_v6_trace_writes_nothing_for_an_engine_with_no_v6_model() {
        let dir = std::env::temp_dir().join(format!("bm-flush-v6-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut eng = TraceOnlyEngine { line: None, v6: None };

        super::flush_v6_trace(&dir, &mut eng, true);
        let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
        assert!(body.is_empty(), "no v6 model → nothing written: {body:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── Timed-input deadline arming (F1 regression) ─────────────────────────────

    #[test]
    fn timed_input_deadline_arms_once_and_does_not_recede() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let iv = Duration::from_millis(3000);

        // First armed iteration, no existing deadline: arm at t0 + interval.
        let d1 = super::next_input_deadline(None, true, iv, t0);
        assert_eq!(d1, Some(t0 + iv));

        // Later armed iterations MUST keep the original deadline, not push it
        // forward. This is the whole bug: re-arming to `now + interval` every
        // ~50ms iteration meant `now >= deadline` was never reached.
        let d2 = super::next_input_deadline(d1, true, iv, t0 + Duration::from_millis(50));
        assert_eq!(d2, d1, "armed deadline must not recede on later iterations");
        let d3 = super::next_input_deadline(d2, true, iv, t0 + Duration::from_millis(2999));
        assert_eq!(d3, d1, "still the original deadline right up until it elapses");

        // Not armed (overlay opened, timers off, or read ended): disarm.
        assert_eq!(super::next_input_deadline(d3, false, iv, t0 + Duration::from_millis(2999)), None);
        // Re-arm after a fire (deadline cleared to None): fresh at the new `now`.
        let t_fire = t0 + Duration::from_millis(3000);
        assert_eq!(super::next_input_deadline(None, true, iv, t_fire), Some(t_fire + iv));
    }

    #[test]
    fn glulx_glk_timer_arms_once_and_refires_each_interval() {
        use std::time::{Duration, Instant};
        // The Glulx Glk timer-events clock reuses `next_input_deadline`, so it has
        // the same arm-once/hold/re-arm-after-fire behavior as timed input. A 100ms
        // timer arms once and holds until it elapses, then re-arms fresh after the
        // fire path clears `glulx_timer_next_fire` to None.
        let t0 = Instant::now();
        let iv = Duration::from_millis(100);

        let d1 = super::next_input_deadline(None, true, iv, t0);
        assert_eq!(d1, Some(t0 + iv), "armed once at t0 + interval");
        let d2 = super::next_input_deadline(d1, true, iv, t0 + Duration::from_millis(30));
        assert_eq!(d2, d1, "holds steady across iterations until it fires");

        // Fire path sets glulx_timer_next_fire = None; next armed iteration re-arms
        // fresh at the fire instant + interval (periodic ticking).
        let t_fire = t0 + iv;
        assert_eq!(super::next_input_deadline(None, true, iv, t_fire), Some(t_fire + iv));

        // Timer canceled (interval None → should_arm false): disarm.
        assert_eq!(super::next_input_deadline(d2, false, iv, t0 + Duration::from_millis(30)), None);
    }

    // SQ-0260: the launch-dialog auto-resume must restore the saved turn counter.
    // The stash it works from carries no turn count, so apply_launch_resume reads
    // it from the archive (like the interactive restore) instead of leaving it 0.
    #[test]
    fn launch_resume_restores_the_turn_counter_sq0260() {
        use app::engine::Engine;
        use app::session::GameSession;

        // A Save State (.babelmap) written with a non-zero turn count.
        let sess = GameSession::new(crate::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let save = sess.save_state();
        let arc = std::env::temp_dir().join(format!("bm-sq260-{}.babelmap", std::process::id()));
        let meta = app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 42,
            saved_at: String::new(),
            location: None,
            score: None,
        };
        app::archive::save_archive_meta(
            &arc, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), meta, &[], &[], &[], &[], &[], &[],
        ).expect("write .babelmap with turns=42");

        // Fresh session + default state (turns start at 0), then launch-resume.
        let mut fresh = GameSession::new(crate::tests::read_char_then_save_v4_story(), true, false, None).expect("new");
        let mut state = app::state::AppState::default();
        let mut mapper = mapper::mapper::Mapper::default();
        let panes = crate::PaneRects {
            map: ratatui::layout::Rect::default(), story: ratatui::layout::Rect::default(),
            room_rects: Vec::new(), layer_tabs: Vec::new(), debug_tabs: Vec::new(), dialog: None, aux_dialog: None,
            reset_dialog: None, game_over: None, save_name_dialog: None, text_entry: None, confirm_delete: None, quit_dialog: None, launch_dialog: None, hints_panel: None,
            verb_menu: Default::default(),
            transcript_links: Vec::new(), transcript_max_scroll: 0, transcript_viewport_rows: 0,
            transcript_total_rows: 0,
            modal_list_viewport: 0,
        };
        assert_eq!(state.turns, 0, "a fresh AppState starts at turn 0");

        super::apply_launch_resume(
            &save, Vec::new(), Vec::new(), None,
            &mut fresh, &mut mapper, &mut state, &panes, &arc,
        );

        assert_eq!(state.turns, 42, "launch resume restores the saved turn count (SQ-0260)");
        let _ = std::fs::remove_file(&arc);
    }

    // ── gvm-fault survival (app must not silently exit on a VM runtime fault) ──

    fn fault_test_result(quit: bool, fault: Option<Vec<String>>) -> super::TurnResult {
        super::TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location: None,
            quit,
            erase_lower: false,
            info: None,
            sounds: Vec::new(),
            glulx_sound_ops: Vec::new(),
            diagnostics: vec![],
            fault,
            location_method: None,
            pending_io: None,
            timed_out: false,
            pictures: Vec::new(),
            transcript_elems: Vec::new(),
        }
    }

    fn game_driven_result(location: Option<zvm::ObjectSnapshot>) -> super::TurnResult {
        super::TurnResult {
            transcript: String::new(),
            transcript_runs: Vec::new(),
            location,
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
        }
    }

    fn clearing_result(text: &str) -> super::TurnResult {
        let mut r = game_driven_result(None);
        r.erase_lower = true;
        r.transcript = text.to_string();
        r
    }

    #[test]
    fn game_driven_screen_clear_collapses_menu_reprints() {
        // SQ-0407: a menu navigated by keystrokes (CM's help) clears + reprints the
        // whole menu into the primary buffer every keypress. Consecutive game-driven
        // clears must COLLAPSE — each reprint replaces the last instead of piling up
        // in scrollback — while pre-menu content is preserved.
        let tmp = std::env::temp_dir().join(format!("babelmap-collapse-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = app::state::AppState::default();
        state.config.user_dir = tmp.clone();
        let mut m = mapper::mapper::Mapper::default();
        let rect = ratatui::layout::Rect::new(0, 0, 20, 20);
        let eng = TraceOnlyEngine { line: None, v6: None };

        state.push_transcript("room description"); // pre-menu content

        // First menu draw (a clearing turn): appends MENU v1.
        super::apply_game_driven_result(&mut state, &mut m, &clearing_result("MENU v1"), &tmp, rect, &eng);
        assert!(state.transcript.iter().any(|l| l.contains("MENU v1")));
        let len_after_v1 = state.transcript.len();

        // Second draw (an arrow keypress): collapses v1, appends MENU v2.
        super::apply_game_driven_result(&mut state, &mut m, &clearing_result("MENU v2"), &tmp, rect, &eng);
        assert!(!state.transcript.iter().any(|l| l.contains("MENU v1")), "v1 must be collapsed, not stacked");
        assert!(state.transcript.iter().any(|l| l.contains("MENU v2")), "v2 present");
        assert!(state.transcript.iter().any(|l| l.contains("room description")), "pre-menu content preserved");
        assert!(state.transcript.len() <= len_after_v1, "transcript did not grow across reprints");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn game_driven_turn_bumps_graph_gen_only_on_new_geometry() {
        // SQ-0406: a char-input keypress (menu navigation / "press any key") is a
        // game-driven turn that changes no geometry — it must NOT bump graph_gen,
        // which would re-route the whole map on the main thread every keystroke
        // (the Counterfeit Monkey help-menu pause). A turn that reveals a NEW room
        // still must bump so the map updates.
        let tmp = std::env::temp_dir().join(format!("babelmap-gdr-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = app::state::AppState::default();
        state.config.user_dir = tmp.clone();
        let mut m = mapper::mapper::Mapper::default();
        m.observe(1, "Lab", None); // a known, placed room
        let gen0 = state.graph_gen;
        let rect = ratatui::layout::Rect::new(0, 0, 20, 20);
        let eng = TraceOnlyEngine { line: None, v6: None };

        // Re-reporting the SAME room (a menu keystroke) must not re-route.
        let same = game_driven_result(Some(zvm::ObjectSnapshot { number: 1, parent: 0, name: "Lab".into() }));
        super::apply_game_driven_result(&mut state, &mut m, &same, &tmp, rect, &eng);
        assert_eq!(state.graph_gen, gen0, "re-reporting a known room must not bump graph_gen");

        // Revealing a NEW room must bump (the map has to update).
        let moved = game_driven_result(Some(zvm::ObjectSnapshot { number: 2, parent: 0, name: "Hall".into() }));
        super::apply_game_driven_result(&mut state, &mut m, &moved, &tmp, rect, &eng);
        assert_ne!(state.graph_gen, gen0, "a new room on a game-driven turn must bump graph_gen");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn should_exit_on_turn_gates_on_clean_quit_only() {
        let mut state = app::state::AppState::default();

        // Clean glk_exit: quit, no fault, not already halted → exit.
        let clean = fault_test_result(true, None);
        assert!(super::should_exit_on_turn(&clean, &state));

        // VM fault: quit, fault present → do not exit.
        let fault = fault_test_result(true, Some(vec!["boom".to_string()]));
        assert!(!super::should_exit_on_turn(&fault, &state));

        // Already halted from a prior fault: even a fault-free quit (the VM is a
        // no-op once halted) must not re-trigger an exit.
        state.vm_halted = true;
        let post_halt = fault_test_result(true, None);
        assert!(!super::should_exit_on_turn(&post_halt, &state));

        // Not a quit at all → never exit regardless of vm_halted.
        state.vm_halted = false;
        let not_quit = fault_test_result(false, None);
        assert!(!super::should_exit_on_turn(&not_quit, &state));
    }

    // ── Scott-only game-over interception ────────────────────────────────────
    #[test]
    fn scott_clean_quit_raises_game_over_and_stays_alive() {
        let mut state = app::state::AppState::default();

        // A Scott engine on a quitting turn: raise the overlay, keep the app alive.
        let stay = super::intercept_scott_game_over(true, true, &mut state);
        assert!(!stay, "a Scott clean quit must NOT exit the app");
        assert!(state.overlays.game_over, "a Scott clean quit opens the game-over dialog");
        assert_eq!(state.overlays.dialog_focus, 0, "focus starts on the first button");

        // A non-Scott engine on a quitting turn: exit as before, no overlay.
        let mut state2 = app::state::AppState::default();
        let exit = super::intercept_scott_game_over(true, false, &mut state2);
        assert!(exit, "a non-Scott clean quit still exits the app");
        assert!(!state2.overlays.game_over, "non-Scott never opens the game-over dialog");

        // A Scott engine on a non-quitting turn: no exit, no overlay.
        let mut state3 = app::state::AppState::default();
        let exit3 = super::intercept_scott_game_over(false, true, &mut state3);
        assert!(!exit3, "a non-quitting turn never exits");
        assert!(!state3.overlays.game_over, "a non-quitting turn never opens the dialog");
    }

    #[test]
    fn apply_turn_events_halts_and_logs_on_fault() {
        let tmp = std::env::temp_dir().join(format!("babelmap-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create temp user_dir");
        let mut state = app::state::AppState::default();
        state.config.user_dir = tmp.clone();

        let result = fault_test_result(true, Some(vec!["some fault line".to_string()]));
        super::apply_turn_events(&mut state, &result);

        assert!(state.vm_halted, "a fault must set vm_halted");
        assert!(state.notifications.latest_text().is_some(), "a fault must set a user-visible notification");

        let log = std::fs::read_to_string(tmp.join("crash.log")).expect("crash.log written");
        assert!(log.contains("gvm runtime fault"), "crash.log must record the fault header");
        assert!(log.contains("some fault line"), "crash.log must record the fault line");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
