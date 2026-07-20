// Test fixtures build structs by defaulting then setting a few fields; silence
// the pedantic lint in tests only (see the matching attribute in lib.rs).
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use mapper::mapper::Mapper;
use mapper::render::{render as render_map_data, render_layer};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::Terminal;

use app::export_dot::export_dot;
use app::export_svg::export_svg;
use app::map_dump::render_dump;
use app::archive::{load_archive, save_archive_meta};
use app::input::{apply_action, apply_text_entry, key_to_command, mouse_to_action, Action, KeyResolve};
use app::tidy::should_bg_tidy;
use app::persist_files::{list_saves, restore_game};
use app::render::dialog::{DialogRects, DialogStyle};
use app::render::hints_panel::{hint_key_routes, HintKeyKind, HintsPanelRects};
use app::render::verbmenu::draw_verb_menu;
use app::render::inspector::{draw_inspector, room_diagnostics};
use app::render::map::{pulse_border_color, render_map_layered, room_screen_rects, sound_pulse_color};
use app::render::paneframe::{build_layer_segments, draw_framed, draw_header_plain, draw_top_inset, InsetSegment};
use app::render::tidy_panel::draw_tidy_panel;
use mapper::graph::RoomId;
use mapper::layer::LayerId;
use app::render::room_info::draw_room_info;
use app::render::screen::render_story_pane;
use app::render::draw_str_clipped;
use app::engine::Engine;
use app::session::{apply_turn, TurnResult};
use app::hints;
use app::keymap::Context;
use app::render::hintbar::{hint_bar, ANIM_HINTS, GAME_HINTS, MAP_HINTS};
use app::slash;
use app::state::{AppState, FbMode, FileBrowserState, Focus, Layout, RoomPanelMode, SavesState};

mod engine_helpers;
mod ingame_io;
mod lifecycle;
mod loop_tick;
mod overlays;
mod picker_ui;
mod reset;
mod slash_dispatch;
mod startup;
mod turn;

use crate::slash_dispatch::dispatch_slash_outcome;
use crate::ingame_io::{
    delete_save_confirmed, handle_save_as, open_ingame_saves, resolve_filename_request,
    resolve_ingame_dialog,
};
use crate::reset::reset_game;
use crate::engine_helpers::{
    engine_supports_save, engine_tag, glulx_session_opt_mut, restore_error_msg, restore_from_file,
    zvm_session_mut, zvm_session_opt, zvm_session_opt_mut, RestoreOutcome,
};

// ── Terminal restore helpers ──────────────────────────────────────────────────

/// Restore the terminal to cooked mode and leave the alternate screen.
/// Called both on clean exit and from the panic hook.
/// DisableMouseCapture MUST be issued here so both paths release the mouse.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

/// Set by an external termination signal; the main loops poll
/// [`termination_requested`] and restore the terminal + exit at a safe point.
static TERMINATE: std::sync::OnceLock<std::sync::Arc<std::sync::atomic::AtomicBool>> =
    std::sync::OnceLock::new();

/// Register handlers for external termination signals so a `kill` (SIGTERM), a
/// closed controlling terminal (SIGHUP), or an out-of-band SIGINT/SIGQUIT
/// restores the terminal instead of leaving it in raw mode + the alternate
/// screen with mouse capture on. The handlers only set an atomic flag (an
/// async-signal-safe operation); the actual `restore_terminal()` runs from the
/// main loop at a safe point. No-op on non-Unix (Windows has no SIGTERM/SIGHUP,
/// and its console resets on process exit). Idempotent.
fn install_termination_handlers() {
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
        // In raw mode ISIG is off, so interactive Ctrl-C/Ctrl-\ arrive as
        // keystrokes, not signals; these fire only on an out-of-band kill or the
        // controlling terminal closing.
        for sig in [SIGTERM, SIGHUP, SIGINT, SIGQUIT] {
            let _ = signal_hook::flag::register(sig, std::sync::Arc::clone(&flag));
        }
    }
    let _ = TERMINATE.set(flag);
}

/// True once an external termination signal has been received.
fn termination_requested() -> bool {
    TERMINATE
        .get()
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed))
}

/// If an external termination signal arrived, restore the terminal and exit.
/// Called at the top of each interactive loop so a signal never leaves the
/// terminal wrecked.
fn exit_if_terminated() {
    if termination_requested() {
        restore_terminal();
        std::process::exit(130);
    }
}

/// Install a panic hook that restores the terminal, writes the panic and a
/// backtrace to a durable `crash.log`, and then prints the panic message.
///
/// The durable file matters because the panic message is printed to stderr
/// only *after* `LeaveAlternateScreen`, where the terminal's alternate-screen
/// restore can hide or overwrite it — so a real crash could otherwise leave no
/// visible trace. The log survives that teardown. (An abort — OOM, stack
/// overflow, double-panic — bypasses this hook entirely and leaves no entry;
/// an empty `crash.log` after a crash is itself evidence of an abort.)
fn install_panic_hook(user_dir: std::path::PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let log_path = user_dir.join("crash.log");
        let path = match write_crash_log(&log_path, info, &backtrace) {
            Ok(()) => log_path,
            // Fall back to the temp dir if the user dir isn't writable.
            Err(_) => {
                let tmp = std::env::temp_dir().join("babelmap-crash.log");
                let _ = write_crash_log(&tmp, info, &backtrace);
                tmp
            }
        };
        eprintln!("babelmap crashed — details written to {}", path.display());
        default_hook(info);
    }));
}

/// Append one panic record (message + backtrace) to `path`.
fn write_crash_log(
    path: &std::path::Path,
    info: &std::panic::PanicHookInfo<'_>,
    backtrace: &std::backtrace::Backtrace,
) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "\n=== babelmap panic ===\n{info}\n\nbacktrace:\n{backtrace}")
}

/// Directory holding per-game save archives (`.babelmap`, default + named) and
/// the game's own standard `.qzl` saves. Kept separate from the map
/// directory. Defaults to `config.user_dir/saves`.
fn saves_dir(user_dir: &std::path::Path) -> std::path::PathBuf {
    user_dir.join("saves")
}

/// Persist the live look (`state.colors`/`state.symbols`) to the user's personal
/// style file and repoint `config.toml`'s `style` key at it, then re-resolve so the
/// live look matches the self-contained file just written.
fn save_style_and_repoint(state: &mut AppState, user_dir: &std::path::Path) {
    let style_path = app::style::personal_style_path(user_dir);
    let _ = app::style::write_style_full(&style_path, &state.colors, &state.symbols);
    state.config.style = Some(style_path.to_string_lossy().into_owned());
    let _ = app::config::write_config(user_dir, &state.config);

    // Re-resolve from the now-self-contained style file (style.toml is the single source).
    let (base, _w1) = app::style::load_style(state.config.style.as_deref(), user_dir);
    let (cs, set, _w2) = app::style::resolve(&base, user_dir);
    state.colors = cs;
    state.symbols = set;
}

// ── Draw helper ───────────────────────────────────────────────────────────────

/// Both pane inner-content rects returned by `draw_frame`.
/// `map` is `Rect::default()` when the layout hides the map (TranscriptFull).
/// `room_rects` maps each visible room to its drawn bounding rect in screen coords.
/// `layer_tabs` pairs each visible layer tab with its hit-rect (click switches layers).
/// `dialog` holds the last-drawn dialog chrome rects for mouse hit-testing.
struct PaneRects {
    map: Rect,
    story: Rect,
    room_rects: Vec<(RoomId, Rect)>,
    /// Hit-rects for each layer tab, paired with the layer id; the mouse
    /// handler hit-tests these to switch the viewed layer on click.
    layer_tabs: Vec<(LayerId, Rect)>,
    /// Active dialog chrome rects (when a dialog is open).
    pub dialog: Option<DialogRects>,
    /// Hit-rects for the aux-storage prompt (when open).
    pub aux_dialog: Option<app::render::aux_dialog::AuxDialogRects>,
    /// Hit-rects for the reset dialog (when open).
    pub reset_dialog: Option<app::render::reset_dialog::ResetDialogRects>,
    /// Hit-rects for the Scott-only game-over dialog (when open).
    pub game_over: Option<app::render::game_over_dialog::GameOverDialogRects>,
    /// Hit-rects for the save-name dialog (when open).
    pub save_name_dialog: Option<app::render::save_name_dialog::SaveNameDialogRects>,
    /// Hit-rects for the generic text-entry dialog (when open).
    pub text_entry: Option<app::render::text_entry_dialog::TextEntryDialogRects>,
    /// Hit-rects for the confirm-delete dialog (when open).
    pub confirm_delete: Option<app::render::confirm_delete_dialog::ConfirmDeleteDialogRects>,
    /// Hit-rects for the quit dialog (when open).
    pub quit_dialog: Option<app::render::quit_dialog::QuitDialogRects>,
    /// Hit-rects for the launch dialog (when open).
    pub launch_dialog: Option<app::render::launch_dialog::LaunchDialogRects>,
    /// Hit-rects for the hints panel (when open).
    pub hints_panel: Option<HintsPanelRects>,
    /// Hit-rects for the verb dock's token rows and section headers (when open).
    pub verb_menu: app::render::verbmenu::VerbMenuHits,
    /// Per-frame map from rendered story-pane cell `(col, row)` → Glk hyperlink
    /// value. Built during transcript render; the mouse handler hit-tests these
    /// on click to deliver the hyperlink event. Empty when nothing on screen is
    /// linked. Story-pane cells share the Glk screen frame, so these coords are
    /// directly click-comparable.
    pub transcript_links: Vec<((u16, u16), u32)>,
    /// Largest meaningful `transcript_scroll` this frame (total wrapped rows −
    /// viewport). The loop clamps `state.transcript_scroll` to this so the view
    /// can't over-scroll past the top.
    pub transcript_max_scroll: u16,
    /// Visible transcript rows this frame (the transcript viewport height). Used
    /// to size a PageUp/PageDown step.
    pub transcript_viewport_rows: u16,
    /// Total wrapped transcript rows this frame. Cached so a command turn can
    /// measure how many rows its output added (the [more] pager, SQ-0404).
    pub transcript_total_rows: u16,
    /// List-row viewport of the open selection-list modal this frame, synced to
    /// `AppState.modal_list_viewport` so nav actions can window/animate. 0 when
    /// no list modal is open.
    pub modal_list_viewport: usize,
}

/// The map render model for one frame: either borrowed from the per-frame cache
/// (the live graph, keyed by generation + layer) or freshly built and owned (the
/// replay / tidy-animation graphs, which `graph_gen` does not track). Derefs to
/// `&RenderMap` so the draw call sites are unchanged. (SQ-0305)
enum FrameRenderMap<'a> {
    Cached(std::cell::Ref<'a, mapper::render::RenderMap>),
    Owned(mapper::render::RenderMap),
}

impl std::ops::Deref for FrameRenderMap<'_> {
    type Target = mapper::render::RenderMap;
    fn deref(&self) -> &Self::Target {
        match self {
            FrameRenderMap::Cached(r) => r,
            FrameRenderMap::Owned(o) => o,
        }
    }
}

/// The base panel border style from the theme — bold `:active` when the panel
/// has input focus, plain `panel.border` otherwise (SQ-0309 §2a).
fn panel_border(theme: &app::theme::resolve::Theme, focused: bool) -> Style {
    theme.get(if focused { "panel.border:active" } else { "panel.border" }).style
}

/// The panel border box style from the theme (SQ-0309 §2a): all sides use
/// `panel.border`'s resolved `BorderStyle` (single-line by default). Replaces the
/// legacy per-panel `*_border_sides` so the unified frame is theme-driven.
fn panel_border_sides(theme: &app::theme::resolve::Theme) -> app::render::paneframe::PaneSides {
    app::render::paneframe::PaneSides::all(
        theme.get("panel.border").border.unwrap_or(app::render::paneframe::BorderStyle::None),
    )
}

/// Render one frame. Returns both pane inner-content rects so the event loop
/// can route mouse events and make accurate `recenter_on` calls.
fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    engine: &dyn Engine,
    mapper: &Mapper,
    state: &AppState,
) -> std::io::Result<PaneRects> {
    let mut map_area = Rect::default();
    let mut story_area = Rect::default();
    let mut room_rects_out: Vec<(RoomId, Rect)> = Vec::new();
    let mut layer_tabs_out: Vec<(LayerId, Rect)> = Vec::new();
    let mut dialog_rects_out: Option<DialogRects> = None;
    let mut overlay_rects: Option<overlays::OverlayRects> = None;
    let mut verb_hits = app::render::verbmenu::VerbMenuHits::default();
    let mut modal_list_viewport: usize = 0;
    let mut transcript_max_scroll: u16 = 0;
    let mut transcript_viewport_rows: u16 = 0;
    let mut transcript_total_rows: u16 = 0;
    let mut transcript_links_out: Vec<((u16, u16), u32)> = Vec::new();

    terminal.draw(|f| {
        let full = f.area();
        let buf = f.buffer_mut();
        // The engine-neutral screen model for this frame (status + window tree).
        let screen_model = engine.screen();
        // During replay the map shows the reconstructed snapshot for the selected turn.
        let replay_graph: Option<mapper::graph::MapGraph> = state.overlays.replay.as_ref().map(|r| {
            let snap = state
                .history
                .get(r.idx)
                .map(|rec| rec.turn)
                .and_then(|turn| app::history::map_at_turn(&state.history, turn))
                .and_then(|json| mapper::persist::from_json(json).ok());
            // Replaying a turn before the first map snapshot has no recorded
            // map — show an empty map, never the live (future) graph.
            snap.map(|m| m.graph).unwrap_or_default()
        });

        // During tidy-animation playback the map shows the current captured stage, not the live graph.
        // The live graph's routed model is memoized on (graph_gen, layer) — see `cached_map_render` —
        // so an animation / transcript / mouse-move redraw of an unchanged map skips re-routing.
        // Replay and tidy-animation graphs are not tracked by `graph_gen`, so they are built fresh.
        // `frame_layer`, not `active_layer(g)`: an animation frame's graph is a layer SUBGRAPH and
        // cannot be asked which layer it is — it always answers main, and the map draws blank
        // (SQ-0359).
        let layer = state.frame_layer(&mapper.graph, replay_graph.as_ref());
        let rm = if let Some(g) = &replay_graph {
            FrameRenderMap::Owned(render_layer(g, layer))
        } else {
            match &state.tidy_anim {
                Some(anim) => FrameRenderMap::Owned(render_layer(&anim.current().graph, layer)),
                None => {
                    FrameRenderMap::Cached(state.cached_map_render(layer, &mapper.graph))
                }
            }
        };

        // ── Inventory dock: reserve a bottom band (above the help row) that
        // slides up when toggled, sized from the item list + slide fraction.
        let inv_visible = state.show_inventory || state.inv_dock.active();
        let inv_items: Vec<String> = if inv_visible {
            app::render::transcript::inventory_items(state.player_obj, &state.inventory_fallback, engine.introspect())
        } else {
            Vec::new()
        };
        let pane_layout = app::layout::compute_pane_layout(full, state, inv_items.len());

        // While any background map job is in flight — a tidy relayout or the
        // async re-route worker (SQ-0379) — the map pane border pulses between red
        // and green, overriding the normal (focused/unfocused) border color.
        let map_border_override: Option<ratatui::style::Color> =
            state.map_job_pulse_elapsed().map(pulse_border_color);

        // Resolve the story-border color: a live sound pulse overrides the fg.
        let story_border_style = {
            let base = panel_border(&state.colors.theme, state.focus == Focus::Game);
            match &state.sound_pulse {
                Some(p) => {
                    let beep_color = match p.kind {
                        app::state::BeepKind::High => state
                            .colors
                            .theme
                            .get("sound_beep_high")
                            .style
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        app::state::BeepKind::Low => state
                            .colors
                            .theme
                            .get("sound_beep_low")
                            .style
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(60, 140, 220)),
                    };
                    let normal = base.fg.unwrap_or(ratatui::style::Color::Reset);
                    match sound_pulse_color(beep_color, normal, p.started.elapsed()) {
                        Some(c) => base.fg(c),
                        None => base,
                    }
                }
                None => base,
            }
        };

        if state.debug.is_some() {
            // ── Debug inspector (tiled): story pane drawn normally; the map slot
            // renders the debug region instead of the map — no room rects, layer
            // tabs, or tidy pulse (those belong to the real map). `pane_layout`
            // already reserved a right-hand rect for this (`compute_pane_layout`
            // treats debug-active as `Layout::Split`).
            let resize_split_hl = state.resize_mode && state.resize_target == app::state::ResizeTarget::StoryMap;
            let story_border_color = if resize_split_hl { state.colors.theme.get("panel.border:active").style } else { story_border_style };
            let story_fp = draw_framed(buf, pane_layout.story, panel_border_sides(&state.colors.theme), &state.colors.story_border_glyphs, story_border_color, state.colors.story_header_on);
            let c = story_fp.content;
            let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
            transcript_max_scroll = m.max_scroll;
            transcript_viewport_rows = m.viewport_rows;
            transcript_total_rows = m.total_rows;
            transcript_links_out = m.links;
            if let Some(hrect) = story_fp.header {
                let segs = [InsetSegment { text: &state.title, active: false }];
                if story_fp.header_bordered {
                    draw_top_inset(buf, hrect, &segs, state.colors.theme.get("story_title").style, state.colors.theme.get("story_title").style);
                } else {
                    draw_header_plain(buf, hrect, &segs, state.colors.theme.get("story_title").style, state.colors.theme.get("story_title").style);
                }
            }
            story_area = story_fp.content;

            app::render::debug_panel::draw_debug_panel(state, pane_layout.map, buf);
            map_area = pane_layout.map;

            // Story pane dims when the debug region has focus (mirrors the map's
            // focus-dim rule below).
            if state.focus == Focus::Map {
                dim_area(buf, story_fp.content);
            }
        } else {
            match state.layout {
                Layout::TranscriptFull => {
                    let story_fp = draw_framed(buf, pane_layout.story, panel_border_sides(&state.colors.theme), &state.colors.story_border_glyphs, story_border_style, state.colors.story_header_on);
                    let c = story_fp.content;
                    let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
                    transcript_max_scroll = m.max_scroll;
                    transcript_viewport_rows = m.viewport_rows;
                    transcript_total_rows = m.total_rows;
                    transcript_links_out = m.links;
                    if let Some(hrect) = story_fp.header {
                        let segs = [InsetSegment { text: &state.title, active: false }];
                        if story_fp.header_bordered {
                            draw_top_inset(buf, hrect, &segs, state.colors.theme.get("story_title").style, state.colors.theme.get("story_title").style);
                        } else {
                            draw_header_plain(buf, hrect, &segs, state.colors.theme.get("story_title").style, state.colors.theme.get("story_title").style);
                        }
                    }
                    story_area = story_fp.content;
                    map_area = Rect::default();
                }
                Layout::Split => {
                    // Split 50/50 horizontally with bordered blocks (no divider column).
                    // In resize mode, the StoryMap target covers this whole split, so
                    // both borders pick up the `focused_border` accent to show it's live.
                    let resize_split_hl = state.resize_mode && state.resize_target == app::state::ResizeTarget::StoryMap;
                    let story_border_color = if resize_split_hl { state.colors.theme.get("panel.border:active").style } else { story_border_style };
                    let map_border_color = if resize_split_hl { state.colors.theme.get("panel.border:active").style } else { panel_border(&state.colors.theme, state.focus == Focus::Map) };
                    let story_fp = draw_framed(buf, pane_layout.story, panel_border_sides(&state.colors.theme), &state.colors.story_border_glyphs, story_border_color, state.colors.story_header_on);
                    let c = story_fp.content;
                    let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
                    transcript_max_scroll = m.max_scroll;
                    transcript_viewport_rows = m.viewport_rows;
                    transcript_total_rows = m.total_rows;
                    transcript_links_out = m.links;
                    if let Some(hrect) = story_fp.header {
                        let segs = [InsetSegment { text: &state.title, active: false }];
                        if story_fp.header_bordered {
                            draw_top_inset(buf, hrect, &segs, state.colors.theme.get("story_title").style, state.colors.theme.get("story_title").style);
                        } else {
                            draw_header_plain(buf, hrect, &segs, state.colors.theme.get("story_title").style, state.colors.theme.get("story_title").style);
                        }
                    }
                    story_area = story_fp.content;

                    let map_fp = draw_framed(buf, pane_layout.map, panel_border_sides(&state.colors.theme), &state.colors.map_border_glyphs, map_border_color, state.colors.map_header_on);
                    render_map_layered(&rm, &mapper.graph, state, map_fp.content, buf);
                    if let Some(anim) = &state.tidy_anim {
                        let tidy_ds = make_dialog_style(state);
                        if let Some(dr) = draw_tidy_panel(anim.current(), map_fp.content, buf, &tidy_ds) {
                            dialog_rects_out = Some(dr);
                        }
                    }
                    map_area = map_fp.content;
                    // Overlay layer tabs
                    {
                        // The tab strip names every layer, so it reads the LIVE graph — never an
                        // animation frame. A frame is a `layer_subgraph`, whose `layers()` is always
                        // main-only, so asking it made the tidied layer's own tab vanish mid-animation
                        // (SQ-0359). `layer` (from `frame_layer`) marks the active tab.
                        let graph = if let Some(g) = &replay_graph { g } else { &mapper.graph };
                        let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                        let active_layer = layer;
                        let owned_segs = build_layer_segments(&layer_ids, active_layer,
                        |id| format!("{}({})", graph.layer_name(id), graph.rooms_in_layer(id).len()));
                        let inset_segs: Vec<_> = owned_segs.iter().map(|s| s.as_inset()).collect();
                        if let Some(hrect) = map_fp.header {
                            let tab_rects = if map_fp.header_bordered {
                                draw_top_inset(buf, hrect, &inset_segs, state.colors.theme.get("panel.tab").style, state.colors.theme.get("panel.tab:active").style)
                            } else {
                                draw_header_plain(buf, hrect, &inset_segs, state.colors.theme.get("panel.tab").style, state.colors.theme.get("panel.tab:active").style)
                            };
                            layer_tabs_out = layer_ids.into_iter().zip(tab_rects).collect();
                        }
                    }
                    // Apply pulsing border color overlay when a tidy job is in flight
                    if let Some(pulse_color) = map_border_override {
                        let pulse_style = Style::default().fg(pulse_color);
                        for cy in pane_layout.map.y..pane_layout.map.bottom() {
                            if let Some(c) = buf.cell_mut((pane_layout.map.x, cy)) { c.set_style(pulse_style); }
                            if let Some(c) = buf.cell_mut((pane_layout.map.right().saturating_sub(1), cy)) { c.set_style(pulse_style); }
                        }
                        for cx in pane_layout.map.x..pane_layout.map.right() {
                            if let Some(c) = buf.cell_mut((cx, pane_layout.map.y)) { c.set_style(pulse_style); }
                            if let Some(c) = buf.cell_mut((cx, pane_layout.map.bottom().saturating_sub(1))) { c.set_style(pulse_style); }
                        }
                    }

                    // While the async map-render worker runs, list each phase it has
                    // started in the map's top-right corner so the source of any map
                    // update delay is visible; the trace clears when the job lands
                    // (SQ-0379). The inner content rect keeps it off the pulsing border.
                    if state.map_render_in_flight() {
                        let area = map_fp.content;
                        let style = state.colors.theme.get("panel.tab").style;
                        for (i, step) in state.render_steps_snapshot().iter().enumerate() {
                            let y = area.y + i as u16;
                            if y >= area.bottom() { break; }
                            let w = (step.chars().count() as u16).min(area.width);
                            let x = area.right().saturating_sub(w);
                            buf.set_stringn(x, y, step, w as usize, style);
                        }
                    }

                    // Map pane is NEVER dimmed (always full brightness).
                    // Story pane dims when map has focus.
                    if state.focus == Focus::Map {
                        dim_area(buf, story_fp.content);
                    }
                }
            }
        }

        // Compute room screen rects for accurate mouse hit-testing. Skipped while
        // the debug inspector occupies the map slot — `map_area` is the debug
        // rect, not a real map, so there is nothing to hit-test.
        room_rects_out = if map_area.height > 0 && state.debug.is_none() {
            room_screen_rects(&rm, state, map_area)
        } else {
            Vec::new()
        };

        // ── Room panel overlay (map-only; skipped while the debug inspector is
        // tiled into the map slot) ──────────────────────────────────────────────
        if map_area.height > 0 && state.debug.is_none() {
            if let Some(panel) = state.room_panel {
                let graph = if let Some(g) = &replay_graph {
                    g
                } else {
                    match &state.tidy_anim {
                        Some(anim) => &anim.current().graph,
                        None => &mapper.graph,
                    }
                };
                let panel_ds = make_dialog_style(state);
                match panel.mode {
                    RoomPanelMode::Info => {
                        let current_room = graph.current();
                        // Objects in the room come from the engine's introspection
                        // (unavailable during tidy-anim playback → empty).
                        let room_objects: Vec<String> = if state.tidy_anim.is_none() {
                            engine.introspect().map(|i| i.room_objects(panel.id)).unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        if let Some(dr) = draw_room_info(graph, &room_objects, panel.id, current_room, map_area, buf, &panel_ds) {
                            dialog_rects_out = Some(dr);
                        }
                    }
                    RoomPanelMode::Diagnostics => {
                        if let Some(diag) = room_diagnostics(graph, panel.id) {
                            if let Some(dr) = draw_inspector(&diag, map_area, buf, &panel_ds) {
                                dialog_rects_out = Some(dr);
                            }
                        }
                    }
                }
            }
        }

        // ── Inventory dock panel ──────────────────────────────────────────────
        if pane_layout.inv_dock.height > 0 {
            let inv_resize_hl = state.resize_mode && state.resize_target == app::state::ResizeTarget::InvDock;
            app::render::inventory_dock::draw_inventory_dock(&inv_items, pane_layout.inv_dock, &state.colors, inv_resize_hl, buf);
        }

        // ── Verb dock panel ────────────────────────────────────────────────────
        if pane_layout.verb_dock.width > 0 {
            draw_verb_menu(state, pane_layout.verb_dock, buf, &mut modal_list_viewport, &mut verb_hits);
        }

        // ── Change 2: draw help bar in bottom row ─────────────────────────────
        let help_style = state.colors.theme.get("help_bar").style;
        let help_text = if state.overlays.config_screen.is_some() {
            "\u{2191}\u{2193} move  \u{2190}\u{2192}/Space change  s save  Esc cancel".to_string()
        } else if state.overlays.verb_menu.is_some() {
            "Verb Menu | Tab/\u{2190}\u{2192}: pane | \u{2191}\u{2193}: move | Enter/Space: pick | Esc: close".to_string()
        } else if state.overlays.file_browser.as_ref().map(|fb| fb.mode == FbMode::PickFile).unwrap_or(false) {
            "Import Save | \u{2191}\u{2193}: move | Enter: open/import | Esc: cancel".to_string()
        } else if state.overlays.saves.is_some() {
            "Saves | \u{2191}\u{2193}: select | Enter: load | s: save-as | d: delete | i: import | Esc: close".to_string()
        } else if let Some(anim) = &state.tidy_anim {
            // Playback status: stage progress + the transport controls.
            let f = anim.current();
            let prefix = format!(
                "Tidy [{}/{}] {}{}",
                anim.idx + 1,
                anim.frames.len(),
                f.label,
                if anim.playing { " \u{25b6}" } else { "" },
            );
            let hint_width = (pane_layout.help_row.width as usize).saturating_sub(prefix.chars().count() + 3);
            let hints = hint_bar(&state.keymap, &state.hotkeys, Context::Anim, ANIM_HINTS, hint_width);
            format!("{} | {}", prefix, hints)
        } else if state.resize_mode {
            use app::state::ResizeTarget;
            let t = match state.resize_target {
                ResizeTarget::StoryMap => "story/map",
                ResizeTarget::InvDock => "inventory",
            };
            format!("Resize [{t}] | Tab: pane | arrows: adjust | 0: reset | Esc: done")
        } else {
            let leader_hint = format!("{}: menu", state.hotkeys.prefix.label());
            // Reserve room for the leader hint + " | " separator so the composed
            // row doesn't overflow help_row.width (mirrors the tidy_anim branch).
            let w = (pane_layout.help_row.width as usize).saturating_sub(leader_hint.chars().count() + 3);
            let rest = match state.focus {
                Focus::Game => hint_bar(&state.keymap, &state.hotkeys, Context::Global, GAME_HINTS, w),
                Focus::Map if state.debug.is_some() => {
                    // Show the live disassembly mode in the `r:` hint entry.
                    let mode = state.debug.as_ref().map(|p| p.disasm_mode_label()).unwrap_or("full");
                    let hints: Vec<(&str, &str)> = app::render::hintbar::DEBUG_HINTS.iter()
                        .map(|&(k, v)| if k == "r" { ("r", mode) } else { (k, v) }).collect();
                    app::render::hintbar::literal_hint_bar(&hints, w)
                }
                Focus::Map => hint_bar(&state.keymap, &state.hotkeys, Context::Map, MAP_HINTS, w),
            };
            if rest.is_empty() {
                leader_hint
            } else {
                format!("{} | {}", leader_hint, rest)
            }
        };
        // Fill help row with reversed style, then draw text.
        for x in pane_layout.help_row.x..pane_layout.help_row.right() {
            if let Some(cell) = buf.cell_mut((x, pane_layout.help_row.y)) {
                cell.set_symbol(" ").set_style(help_style);
            }
        }
        draw_str_clipped(buf, pane_layout.help_row.x, pane_layout.help_row.y, &help_text, help_style, pane_layout.help_row);

        // The z-ordered modal/overlay ladder now lives in `overlays::draw_all`
        // (SQ-0306). It seeds `dialog` from the pre-ladder map/story draws
        // (tidy panel / room info / inspector) and returns the per-overlay
        // hit-rects that `draw_frame` splices into `PaneRects` below.
        overlay_rects = Some(overlays::draw_all(
            state,
            &screen_model,
            story_area,
            full,
            buf,
            dialog_rects_out.take(),
            &mut modal_list_viewport,
        ));

        // Story-pane text-selection highlight + copy extraction now happen inside
        // render_middle (render/transcript.rs), which has the full wrapped-row set
        // and can select text beyond the visible viewport. (SQ-0197)
        //
        // The former bottom-bar map-edit prompts are now the text-entry modal drawn
        // by the overlay ladder in the graphics-free dialog area (SQ-0307).

        // Notification toasts overlay everything (including modals): drawn last so
        // they always sit topmost in the top-right corner. (SQ-0176)
        app::render::transcript::render_notifications(buf, full, state);
    })?;

    // The draw closure runs exactly once, so the overlay ladder always ran.
    let overlay_rects = overlay_rects.expect("draw_frame closure runs exactly once");
    Ok(PaneRects { map: map_area, story: story_area, room_rects: room_rects_out, layer_tabs: layer_tabs_out, dialog: overlay_rects.dialog, aux_dialog: overlay_rects.aux_dialog, reset_dialog: overlay_rects.reset_dialog, game_over: overlay_rects.game_over, save_name_dialog: overlay_rects.save_name_dialog, text_entry: overlay_rects.text_entry, confirm_delete: overlay_rects.confirm_delete, quit_dialog: overlay_rects.quit_dialog, launch_dialog: overlay_rects.launch_dialog, hints_panel: overlay_rects.hints_panel, verb_menu: verb_hits, transcript_links: transcript_links_out, transcript_max_scroll, transcript_viewport_rows, transcript_total_rows, modal_list_viewport })
}

// ── File-browser entry action helper ─────────────────────────────────────────

/// Decoded action when Enter is pressed in the file browser.
enum FbEntryAction {
    /// Navigate into the given directory.
    CdInto(std::path::PathBuf),
    /// Import the given file.
    ImportFile(std::path::PathBuf),
}

// ── main ──────────────────────────────────────────────────────────────────────

/// Toggle the opt-in style.toml file-watcher on/off, updating the status line.
fn toggle_style_watch(
    state: &mut app::state::AppState,
    watcher: &mut Option<app::watch::StyleWatcher>,
) {
    if watcher.is_some() {
        *watcher = None;
        state.set_status("style watch off");
    } else if let Some(p) =
        app::reload::resolved_style_path(state.config.style.as_deref(), &state.config.user_dir)
    {
        *watcher = app::watch::start(&p);
        if let Some(w) = watcher.as_mut() {
            w.also_watch(&state.game_dir);
        }
        state.set_status(if watcher.is_some() {
            "style watch on"
        } else {
            "style watch: no file to watch"
        });
    } else {
        state.set_status("style watch: no file to watch");
    }
}

/// Run a map-export Action (SVG/DOT/dump) into the per-game dir. Returns true if
/// `action` was a map-export action (so callers fall through otherwise). Mirrors
/// the resolve→create_dir_all→render→write→notice logic that was inline at the
/// main-loop Action::Export* arms (SQ-0297: slash commands never reached that
/// match, so this is shared so both the slash and key-dispatch paths export).
fn handle_map_export(
    action: &Action,
    game_dir: &std::path::Path,
    mapper: &Mapper,
    state: &mut AppState,
) -> bool {
    match action {
        Action::ExportSvg(dest) => {
            let path = app::export::resolve_export_path(dest.as_deref(), game_dir, "map.svg");
            if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
            let rm = render_map_data(&mapper.graph);
            match export_svg(&path, &rm) {
                Ok(()) => state.push_notice(&format!("[SVG exported to {}]", abbreviate_home(&path))),
                Err(e) => state.push_notice(&format!("[SVG export failed: {}]", e)),
            }
            true
        }
        Action::ExportDot(dest) => {
            let path = app::export::resolve_export_path(dest.as_deref(), game_dir, "map.dot");
            if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
            match export_dot(&path, &mapper.graph) {
                Ok(()) => state.push_notice(&format!(
                    "[DOT exported to {} — render with: dot -Tsvg {} -o map.svg]",
                    abbreviate_home(&path),
                    abbreviate_home(&path)
                )),
                Err(e) => state.push_notice(&format!("[DOT export failed: {}]", e)),
            }
            true
        }
        Action::ExportMap(dest) => {
            let path = app::export::resolve_export_path(dest.as_deref(), game_dir, "map.txt");
            if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
            match std::fs::write(&path, render_dump(&mapper.graph)) {
                Ok(()) => state.push_notice(&format!("[map dump written to {}]", abbreviate_home(&path))),
                Err(e) => state.push_notice(&format!("[map dump failed: {}]", e)),
            }
            true
        }
        _ => false,
    }
}

/// Abbreviate a leading $HOME in a path to `~` for display.
fn abbreviate_home(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if let Some(rest) = s.strip_prefix(&home) { return format!("~{rest}"); }
        }
    }
    s
}

/// Format the one-line loading indicator shown while a (possibly large) story
/// boots to its first prompt. `frame` is the spinner glyph for this tick. Large
/// Glulx games (e.g. Counterfeit Monkey at ~11 MB) take several seconds to reach
/// the first prompt; without this the normal terminal sits frozen and looks hung.
fn loading_line(name: &str, bytes: usize, frame: char) -> String {
    format!("babelmap: loading {name} ({:.1} MB) {frame}", bytes as f64 / 1_048_576.0)
}

fn main() {
    // Run the linear setup phase (arg/config parse, story load, engine + mapper
    // build, initial state seeding, terminal setup) in `startup::boot`; `main()`
    // owns the event loop below over the returned handles (SQ-0306).
    let startup::BootResult {
        mut session,
        mut mapper,
        mut state,
        mut terminal,
        game_dir,
        ifid,
        arc_file,
        story_bytes,
        story_path,
        data_base,
    } = startup::boot();

    // ── 5. Event loop ─────────────────────────────────────────────────────────

    // Track the last-known pane rects for accurate recenter_on calls and mouse routing.
    // Initialized to a zero-sized default; updated by every draw_frame call.
    let mut last_panes = PaneRects { map: Rect::default(), story: Rect::default(), room_rects: Vec::new(), layer_tabs: Vec::new(), dialog: None, aux_dialog: None, reset_dialog: None, game_over: None, save_name_dialog: None, text_entry: None, confirm_delete: None, quit_dialog: None, launch_dialog: None, hints_panel: None, verb_menu: Default::default(), transcript_links: Vec::new(), transcript_max_scroll: 0, transcript_viewport_rows: 0, transcript_total_rows: 0, modal_list_viewport: 0 };

    // Debounce counter for BackgroundTidy::Debounced mode.
    let mut bg_tidy_counter: u32 = 0;

    // Glulx re-arrange debounce (SQ-0201). The Glulx VM starts on a fixed virtual
    // screen; once the real story-pane size is known (and whenever it changes: a
    // terminal resize, a map/sidebar toggle) we report it and deliver a Glk
    // Arrange so graphics windows repaint at the new size — but only after the
    // size settles, so a drag doesn't run the game's redraw on every tick.
    // `vm_story_size` = size last reported to the VM; `story_size_seen` = size at
    // the previous frame; `resize_dirty` = when the size last moved.
    let mut vm_story_size: Option<(u16, u16)> = None;
    let mut story_size_seen: Option<(u16, u16)> = None;
    let mut resize_dirty: Option<std::time::Instant> = None;

    // Poll FPS while a background tidy is in flight.
    const TIDY_POLL_MS: u64 = 33;

    // Optional style.toml file-watcher (opt-in via watch_style; toggled by /watch).
    let mut style_watcher: Option<app::watch::StyleWatcher> = None;
    let mut watch_dirty: Option<std::time::Instant> = None;
    if state.config.watch_style {
        if let Some(p) =
            app::reload::resolved_style_path(state.config.style.as_deref(), &state.config.user_dir)
        {
            style_watcher = app::watch::start(&p);
            if let Some(w) = style_watcher.as_mut() {
                w.also_watch(&state.game_dir);
            }
        }
    }

    // From here on the app drives the game through the engine-neutral trait
    // (`session` was boxed at construction: a GameSession for Z-code, a
    // GlulxSession for Glulx). The Z-machine-specific setup above runs behind a
    // downcast so the Glulx path skips it.

    // Input-burst coalescing: when a read event still has more events queued
    // behind it, defer the redraw until the queue drains. A stream of mouse
    // motion events (or a paste) then costs ONE redraw instead of one per event.
    let mut skip_draw = false;

    // Dirty-flag redraw gate (SQ-0305): the loop wakes every ~50ms (faster while
    // animating/timing) but the UI only changes when something observable happens.
    // Redraw only when `needs_redraw` is set (or an animation is active); an idle
    // app then does ~zero work per tick. The flag is set wherever the loop did
    // something — an event was dispatched, a background poller applied a change, a
    // deadline fired — and left false only on the pure poll-timeout no-op path.
    // First frame always draws. The poll deadlines are UNCHANGED: this gates the
    // draw, not the tick.
    let mut needs_redraw = true;

    'event_loop: loop {
        // Restore the terminal + exit if an external termination signal arrived
        // (SIGTERM/SIGHUP/out-of-band SIGINT); the poll below wakes at least every
        // ~50ms, so this is checked promptly.
        exit_if_terminated();

        // ── Pre-input pollers (SQ-0306) ───────────────────────────────────────
        // The per-iteration housekeeping that runs BEFORE the draw/poll: each
        // independent pollable subsystem lives in `loop_tick` and returns its
        // redraw contribution, OR-ed into `needs_redraw` here (order preserved).
        needs_redraw |= loop_tick::poll_style_watch(&mut state, &style_watcher, &mut watch_dirty);
        loop_tick::sync_theme_colours(&state, &mut *session);
        needs_redraw |= loop_tick::poll_glulx_resize(
            &mut *session,
            &last_panes,
            &mut story_size_seen,
            &mut resize_dirty,
            &mut vm_story_size,
        );
        needs_redraw |= loop_tick::poll_tidy_jobs(&mut state, &mut mapper, &last_panes);
        needs_redraw |= state.poll_render_job();
        needs_redraw |= loop_tick::refresh_engine_input(&mut state, &*session);
        needs_redraw |= loop_tick::expire_sound_and_settle_dock(&mut state);

        // Draw — unless we're mid-drain of an input burst (skip_draw), in which
        // case the deferred redraw happens once the queue empties. last_panes and
        // the panes-derived clamps below simply carry over from the last real
        // frame during the burst (layout is stable within a burst).
        // Redraw gate (SQ-0305): skip the draw entirely when nothing changed and
        // no animation is in flight. `skip_draw` still coalesces an input burst
        // (and, when it fires, leaves `needs_redraw` set so the deferred frame
        // draws once the queue empties). An active animation always draws so its
        // tween keeps stepping.
        if !std::mem::take(&mut skip_draw) && (needs_redraw || state.has_active_animation()) {
        needs_redraw = false;
        match draw_frame(&mut terminal, &*session, &mapper, &state) {
            Ok(panes) => {
                // Clamp scrollback to what the frame can actually show, so an
                // over-scroll past the top doesn't accumulate (and lag on the
                // way back down).
                state.transcript_scroll = state.transcript_scroll.min(panes.transcript_max_scroll);
                // [more] pager (SQ-0404): a command turn armed with the pre-turn
                // wrapped-row total; now this frame knows the after-total + the
                // viewport, so decide whether the output overflowed and, if so,
                // park the view at the FIRST new screenful instead of the bottom.
                if let Some(before) = state.pager.pending_before_rows.take() {
                    match app::pager::activation_target(
                        before,
                        panes.transcript_total_rows,
                        panes.transcript_viewport_rows,
                    ) {
                        Some(target) => {
                            state.scroll_transcript_to(target.min(panes.transcript_max_scroll));
                            state.pager.active = true;
                        }
                        None => state.pager.active = false,
                    }
                }
                // Cache this frame's total so the next command turn can measure how
                // many wrapped rows its own output added (the pager arm). (SQ-0404)
                state.last_transcript_total_rows = panes.transcript_total_rows;
                // Carry this frame's modal list viewport so the next nav action
                // can window/animate the open selection-list modal.
                state.modal_list_viewport = panes.modal_list_viewport;
                // Replay's idx is the source of truth; keep its (animated) list
                // scroll following it. Skip while a scroll is easing so the tween
                // isn't restarted each frame; select() is a no-op once settled.
                let anim = state.config.animation.clone();
                let hist_len = state.history.len();
                if let Some(r) = &mut state.overlays.replay {
                    if !r.scroll.has_active_animation() {
                        r.scroll.len(hist_len);
                        r.scroll.select(r.idx, state.modal_list_viewport, &anim);
                    }
                }
                last_panes = panes;
            }
            Err(e) => {
                restore_terminal();
                eprintln!("babelmap: draw error: {}", e);
                std::process::exit(1);
            }
        }
        }

        // Poll for a key event. Use a shorter timeout while a tidy job is in flight
        // so the pulsing border animates at ~30fps; otherwise use the normal 50ms.
        // When a timed-input deadline is armed, clamp further so the loop wakes in
        // time to fire the interrupt — the normal cadence stays the ceiling, so
        // this is a no-op when no timer is running (regression guard).
        let sound_active = !state.sound_routines.is_empty()
            || !state.glulx_sound_notify.is_empty()
            || !state.glulx_volume_notify.is_empty()
            // A notify-less ramp still needs the loop to keep waking so it can
            // step the sink gain smoothly (glulx_volume_notify may be empty).
            || !state.glulx_volume_ramp.is_empty();
        let timer_active = state.glulx_timer_next_fire.is_some();
        // Continuous story-pane selection auto-scroll: while a drag is held at an
        // edge and that direction can still scroll, keep the loop live so it steps
        // one wrapped row per frame even without new mouse events. Goes quiet once
        // the scroll hits its limit (so we don't busy-spin) or the drag releases. (SQ-0197)
        let selecting_at_edge = state.selection.is_some() && state.selection_edge != 0 && {
            if let Some(g) = state.transcript_geom.get() {
                let max_scroll = g.total_rows.saturating_sub(g.area.height as usize) as u16;
                if state.selection_edge < 0 { state.transcript_scroll < max_scroll }
                else { state.transcript_scroll > 0 }
            } else { false }
        };
        let base_poll_ms = if state.has_active_animation() || sound_active || timer_active || selecting_at_edge { TIDY_POLL_MS } else { 50 };
        // Clamp to whichever clock is due first: the Z-machine timed-input deadline,
        // the Glulx Glk-timer deadline, or the soonest pending Sound2 volume-ramp
        // completion (any may be `None`/empty).
        let next_volume_deadline = state.glulx_volume_notify.values().map(|(t, _)| *t).min();
        let next_deadline = [state.input_deadline, state.glulx_timer_next_fire, next_volume_deadline]
            .into_iter()
            .flatten()
            .min();
        let poll_ms = match next_deadline {
            Some(dl) => {
                let remaining = dl.saturating_duration_since(std::time::Instant::now()).as_millis() as u64;
                remaining.min(base_poll_ms).max(1)
            }
            None => base_poll_ms,
        };
        let event_ready = match poll(Duration::from_millis(poll_ms)) {
            Ok(r) => r,
            Err(e) => {
                restore_terminal();
                eprintln!("babelmap: poll error: {}", e);
                std::process::exit(1);
            }
        };

        if !event_ready {
            // Any animation in flight this tick (scroll/dock/list eases, sound
            // pulse, pending tidy jobs) needs a redraw — both while it tweens and
            // for the one frame where it settles (has_active_animation flips false
            // only after finalize below). (SQ-0305)
            if state.has_active_animation() {
                needs_redraw = true;
            }
            // Story-pane selection held at an edge with no new mouse event: step the
            // auto-scroll one wrapped row and let the next iteration redraw. (SQ-0197)
            if selecting_at_edge {
                app::input::apply_selection_autoscroll(&mut state);
                needs_redraw = true;
            }
            // Timed-input interrupt: the deadline elapsed with no key pressed. Run
            // the game's interrupt routine and apply its output through the same
            // path a char-mode keypress uses; the next loop iteration redraws
            // unconditionally, so no explicit redraw flag is needed. If the read
            // continues, Step 2 above re-arms the deadline next iteration from
            // `pending_timeout()`; if the routine aborted the read, it returns
            // `None` and the timer simply stops.
            if let Some(dl) = state.input_deadline {
                if std::time::Instant::now() >= dl {
                    if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                        let result = zs.run_timed_interrupt();
                        // Fired: disarm so the next armed iteration re-arms fresh at
                        // now + interval (otherwise the elapsed deadline would refire
                        // immediately every iteration).
                        state.input_deadline = None;
                        needs_redraw = true; // interrupt ran → repaint any output
                        if turn::apply_game_driven_result(
                            &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                        ) {
                            break;
                        }
                    }
                }
            }
            // Glulx Glk timer tick: the interval elapsed with no key pressed.
            // Deliver an evtype_Timer to the game and apply its output; disarm so
            // the next armed iteration re-arms fresh at now + interval (mirroring
            // the input-deadline refire guard above).
            if let Some(dl) = state.glulx_timer_next_fire {
                if std::time::Instant::now() >= dl {
                    state.glulx_timer_next_fire = None;
                    needs_redraw = true; // timer event delivered → repaint any output
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        let result = gs.deliver_timer();
                        if turn::apply_game_driven_result(
                            &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                        ) {
                            break;
                        }
                    }
                }
            }
            // Poll for finished sampled sounds and fire their finish-routines.
            let done: Vec<u32> = state.audio.as_mut().map(|b| b.finished()).unwrap_or_default();
            if !done.is_empty() {
                needs_redraw = true; // finish-routine output / channel state changed
            }
            for id in done {
                // Always forget the number->id mapping for a finished sound, even
                // one with no finish routine.
                state.sound_ids.retain(|_, v| *v != id);
                if let Some(routine) = state.sound_routines.remove(&id) {
                    if routine != 0 {
                        if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                            let result = zs.run_sound_finish(routine);
                            if turn::apply_game_driven_result(
                                &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                            ) {
                                break 'event_loop;
                            }
                        }
                    }
                }
                // Glulx sound-notify: a finished channel delivers Evtype_SoundNotify.
                if let Some((snd, notify)) = state.glulx_sound_notify.remove(&id) {
                    state.glulx_channels.retain(|_, v| *v != id);
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        let result = gs.sound_notify(snd, notify);
                        if turn::apply_game_driven_result(
                            &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                        ) {
                            break 'event_loop;
                        }
                    }
                }
            }
            // Glulx Sound2 volume-ramp completion: a gradual set_volume_ext whose
            // duration has elapsed delivers an evtype_VolumeNotify. The host owns
            // the ramp clock (mirroring the sound-finish notify above); deliver every
            // due one, newest-driven output redrawn next iteration.
            let now = std::time::Instant::now();
            // Step any in-flight Sound2 volume ramp toward its target (host owns
            // the ramp clock). Pure audio — no redraw needed.
            state.advance_volume_ramps(now);
            let due_volume: Vec<(u32, u32)> = state
                .glulx_volume_notify
                .iter()
                .filter(|(_, (deadline, _))| *deadline <= now)
                .map(|(&chan, &(_, notify))| (chan, notify))
                .collect();
            if !due_volume.is_empty() {
                needs_redraw = true;
            }
            for (chan, notify) in due_volume {
                state.glulx_volume_notify.remove(&chan);
                if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                    let result = gs.volume_notify(notify);
                    if turn::apply_game_driven_result(
                        &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                    ) {
                        break 'event_loop;
                    }
                }
            }
            // No key this tick — advance the tidy animation if one is playing. The next loop
            // iteration redraws, so an advanced frame appears without waiting for input.
            if let Some(anim) = &mut state.tidy_anim {
                // Short auto-play dwell — stepping is mostly done manually with the
                // arrow keys, so the delay only needs to be long enough to follow.
                // `tick` returns true only when a frame actually advanced — redraw
                // just then, so a paused/holding anim still idles. (SQ-0305)
                if anim.tick(Duration::from_millis(100)) {
                    needs_redraw = true;
                }
            }
            if let Some(r) = &mut state.overlays.replay {
                // Likewise: redraw only when the auto-play cursor advanced a turn.
                if r.tick(Duration::from_millis(700), state.history.len()) {
                    needs_redraw = true;
                }
            }
            // Finalize a completed smooth-scroll: snap the logical offset to the
            // target and drop the animation. The next iteration redraws.
            let done_to = state
                .scroll_anim
                .as_ref()
                .filter(|a| a.done())
                .map(|a| a.target());
            if let Some(to) = done_to {
                state.transcript_scroll = to as u16;
                state.scroll_anim = None;
            }
            // Finalize each open scrollable surface's animation likewise. Each
            // finalize reports whether it just cleared a running anim; OR that
            // into needs_redraw so the frame at the settled offset paints once.
            // A list/dock anim can reach done() *during* the poll wait above, so
            // the `has_active_animation()` check earlier this iteration already
            // read false — without this the settle frame would be gated off and
            // the list would land ~1 row short (or a dock leave a sliver). (SQ-0305)
            if let Some(s) = &mut state.overlays.saves { needs_redraw |= s.scroll.finalize_if_done(); }
            if let Some(fb) = &mut state.overlays.file_browser { needs_redraw |= fb.scroll.finalize_if_done(); }
            if let Some(cs) = &mut state.overlays.config_screen { needs_redraw |= cs.scroll.finalize_if_done(); }
            if let Some(vm) = &mut state.overlays.verb_menu {
                needs_redraw |= vm.verb_scroll.finalize_if_done();
                needs_redraw |= vm.noun_scroll.finalize_if_done();
                needs_redraw |= vm.prep_scroll.finalize_if_done();
            }
            if let Some(r) = &mut state.overlays.replay { needs_redraw |= r.scroll.finalize_if_done(); }
            if let Some(h) = &mut state.overlays.hints { needs_redraw |= h.finalize_scroll_if_done(); }
            // Docks slide via a Tween that goes inactive (not dropped) at done();
            // finalize drops the finished tween and forces the settle frame so a
            // just-opened dock paints fully and a closing inv_dock loses its last
            // sliver. (verb_dock CLOSE is separately covered by settle_verb_dock
            // dropping the drawer content next iteration.) (SQ-0305)
            needs_redraw |= state.inv_dock.finalize_if_done();
            needs_redraw |= state.verb_dock.finalize_if_done();
            continue;
        }

        let event = match read() {
            Ok(e) => e,
            Err(e) => {
                restore_terminal();
                eprintln!("babelmap: read error: {}", e);
                std::process::exit(1);
            }
        };

        // An event was read and will be dispatched (key/mouse/paste/resize, or a
        // dialog/overlay intercept) — the frame may change, so redraw next pass.
        // Biasing to over-draw here is deliberate: a swallowed key costs one extra
        // frame; a missed redraw is a visible bug. (SQ-0305)
        needs_redraw = true;

        // If more input is already queued behind this event, defer the next
        // redraw so the whole burst collapses into a single frame. Cleared at
        // the draw gate once the queue empties (poll(ZERO) == false).
        skip_draw = poll(Duration::ZERO).unwrap_or(false);

        // ── Common-dialog overlay intercept ladder (SQ-0307) ──────────────────
        // The aux / reset / save-name / text-entry / confirm-delete / quit /
        // launch modals share one decode+apply seam. The top-most open overlay
        // (priority order aux ▸ reset ▸ save-name ▸ text-entry ▸ confirm-delete ▸
        // quit ▸ launch — exactly the old if-ladder) decodes the event through
        // its `Overlay` impl, applying pure focus / field / checkbox changes in
        // place, and returns an `OverlayAct` for the game-affecting side effects
        // to run here where session / mapper / paths are in scope. Swallows the
        // events its overlay does not handle, then `continue`s.
        if let Some(ov) = overlays::topmost_common_dialog(&state.overlays) {
            if let Event::Resize(_, _) = &event { let _ = terminal.clear(); continue; }
            let outcome = match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => ov.key(&mut state, k),
                Event::Mouse(m) => ov.mouse(&mut state, m, &last_panes),
                _ => overlays::OverlayOutcome::Consumed,
            };
            if let overlays::OverlayOutcome::Act(act) = outcome {
                use overlays::OverlayAct;
                match act {
                    OverlayAct::AuxArchive => {
                        let mode = app::config::AuxStorage::Archive;
                        state.overlays.aux_prompt = false;
                        state.config.aux_storage = mode;
                        let user_dir = state.config.user_dir.clone();
                        let _ = app::config::write_config(&user_dir, &state.config);
                        session.clear_aux_dirty();
                    }
                    OverlayAct::AuxGlobal => {
                        let mode = app::config::AuxStorage::Global;
                        state.overlays.aux_prompt = false;
                        state.config.aux_storage = mode;
                        let user_dir = state.config.user_dir.clone();
                        let _ = app::config::write_config(&user_dir, &state.config);
                        let _ = app::aux_store::write_global_aux(&game_dir, session.aux_data());
                        session.clear_aux_dirty();
                    }
                    OverlayAct::ResetConfirm => {
                        let clear = state.overlays.reset_clear_map;
                        let delete = state.overlays.reset_delete_data;
                        state.overlays.reset_dialog = false;
                        reset_game(&mut *session, &mut mapper, &mut state, &story_bytes, &story_path, &game_dir, clear, delete);
                    }
                    OverlayAct::ResetCancel => {
                        state.overlays.reset_dialog = false;
                    }
                    OverlayAct::GameOverPlayAgain => {
                        // Plain restart: keep the accumulated map and saved data.
                        state.overlays.game_over = false;
                        reset_game(&mut *session, &mut mapper, &mut state, &story_bytes, &story_path, &game_dir, false, false);
                    }
                    OverlayAct::GameOverRestore => {
                        // Close the game-over overlay and open the saves manager (the
                        // Save State restore flow — same entry point as Action::OpenSaves).
                        state.overlays.game_over = false;
                        let entries = combined_saves(&game_dir);
                        state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
                        state.overlays.dialog_focus = 0;
                    }
                    OverlayAct::GameOverQuit => {
                        break 'event_loop;
                    }
                    OverlayAct::SaveNameSubmit => {
                        // Empty names are rejected (dialog stays open); valid names
                        // go through the shared handle_save_as save path.
                        let value = state
                            .overlays.save_name_dialog
                            .as_ref()
                            .map(|d| d.field.value.clone())
                            .unwrap_or_default();
                        if value.trim().is_empty() {
                            if let Some(d) = state.overlays.save_name_dialog.as_mut() { d.active = false; }
                            state.push_notice("[Save name cannot be empty]");
                        } else {
                            state.overlays.save_name_dialog = None;
                            handle_save_as(
                                value, &game_dir, &ifid, &mut mapper, &mut *session, &mut state,
                            );
                            let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                                || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                            turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                            turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                            if quit { break; }
                        }
                    }
                    OverlayAct::SaveNameCancel => {
                        state.overlays.save_name_dialog = None;
                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                        if quit { break; }
                    }
                    OverlayAct::TextEntrySubmit => {
                        // A CreateFile submit hops through filename_submitted → resume
                        // here; map-edit / config submits leave nothing pending.
                        if let Some(dlg) = state.overlays.text_entry.take() {
                            apply_text_entry(dlg, &mut state, &mut mapper);
                        }
                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                        if quit { break; }
                    }
                    OverlayAct::TextEntryCancel => {
                        // A cancelled CreateFile leaves pending_filename set with no
                        // dialog open → resolve_filename_request treats it as NULL.
                        state.overlays.text_entry = None;
                        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                        if quit { break; }
                    }
                    OverlayAct::ConfirmDelete(confirmed) => {
                        if let Some(path) = state.overlays.confirm_delete_save.take() {
                            delete_save_confirmed(&path, confirmed, &game_dir, &mut state);
                        }
                        // Return the saves manager (still open behind us) to default focus.
                        state.overlays.dialog_focus = 0;
                    }
                    OverlayAct::QuitSave => {
                        state.overlays.quit_dialog = false;
                        lifecycle::quit_dialog_save(&*session, &mapper, &state, &ifid, &arc_file);
                        break;
                    }
                    OverlayAct::QuitQuit => {
                        break;
                    }
                    OverlayAct::QuitCancel => {
                        state.overlays.quit_dialog = false;
                    }
                    OverlayAct::LaunchResume => {
                        if let Some((save, lines, kinds, screen)) = state.pending_resume.take() {
                            state.overlays.launch_dialog = false;
                            turn::apply_launch_resume(&save, lines, kinds, screen, &mut *session, &mut mapper, &mut state, &last_panes, &arc_file);
                        }
                    }
                    OverlayAct::LaunchNewGame => {
                        state.overlays.launch_dialog = false;
                        state.pending_resume = None;
                    }
                }
            }
            continue;
        }

        // ── Hints panel intercept — before normal action routing ──────────────
        // When the hints panel is open, route keyboard/mouse directly here and
        // continue (swallowing events the panel does not handle).
        if state.overlays.hints.is_some() {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    use crossterm::event::KeyCode;
                    match hint_key_routes(k.code) {
                        HintKeyKind::Close => {
                            state.overlays.hints = None;
                        }
                        HintKeyKind::ToSession => {
                            match k.code {
                                KeyCode::Enter => {
                                    if let Some(ref mut hs) = state.overlays.hints {
                                        let line = std::mem::take(&mut hs.input);
                                        let app::state::HintSource::Zcode(ref mut vm) = hs.source;
                                        let result = vm.submit(&line);
                                        for l in result.transcript.split('\n') {
                                            hs.transcript.push(l.to_owned());
                                        }
                                        hs.scroll = 0;
                                        hs.scroll_anim = None;
                                    }
                                }
                                KeyCode::Backspace => {
                                    if let Some(ref mut hs) = state.overlays.hints {
                                        hs.input.pop();
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if let Some(ref mut hs) = state.overlays.hints {
                                        hs.input.push(c);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let pt = ratatui::layout::Position { x: m.column, y: m.row };
                        if let Some(hp) = &last_panes.hints_panel {
                            let in_close = hp.close.is_some_and(|r| r.contains(pt));
                            if in_close {
                                state.overlays.hints = None;
                            }
                            // Clicks inside the dialog but not on close: swallow.
                        }
                    } else if let Some(d) = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert) {
                        // Wheel drives the hint transcript's own scroll. The panel
                        // is intercepted before mouse_to_action, so resolve the
                        // direction (and mouse_wheel_invert) via the shared helper.
                        let max = last_panes.hints_panel.as_ref().map_or(0, |hp| hp.max_scroll);
                        let anim = state.config.animation.clone();
                        if let Some(hs) = &mut state.overlays.hints {
                            // Wheel up (d < 0) → older content (increase scroll),
                            // matching the story transcript's wheel direction.
                            hs.scroll_by(if d < 0 { 1 } else { -1 }, max, &anim);
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
                _ => {}
            }
            continue;
        }

        // ── Search-nav intercept — before normal action routing ───────────────
        // When a search is active and no modal is open, intercept the configured
        // back/forward keys and Esc to navigate matches.  Any other key clears
        // the search state and then falls through to normal processing below.
        if state.search_query.is_some() && !state.any_overlay_open() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    use crossterm::event::KeyCode;
                    let key_back = state.config.search.key_back;
                    let key_forward = state.config.search.key_forward;
                    match k.code {
                        KeyCode::Char(c) if c == key_back => {
                            if let Some(pos) = state.search_next(false) {
                                let total_vis = state.visible_transcript_indices().len();
                                let pane_rows = if last_panes.story.height > 0 {
                                    last_panes.story.height as usize
                                } else {
                                    24
                                };
                                state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                            }
                            continue;
                        }
                        KeyCode::Char(c) if c == key_forward => {
                            if let Some(pos) = state.search_next(true) {
                                let total_vis = state.visible_transcript_indices().len();
                                let pane_rows = if last_panes.story.height > 0 {
                                    last_panes.story.height as usize
                                } else {
                                    24
                                };
                                state.transcript_scroll = scroll_for_match(pos, total_vis, pane_rows);
                            }
                            continue;
                        }
                        KeyCode::Esc => {
                            state.clear_search();
                            continue;
                        }
                        _ => {
                            // Any other key: clear search, then fall through to normal processing.
                            state.clear_search();
                        }
                    }
                }
            }
        }

        // ── Debug inspector (tiled): drive it while its region is focused ──────
        // Not a modal: only intercepts while the debug region holds `Focus::Map`,
        // and runs before the normal key→command dispatch so it pre-empts
        // Tab→ToggleFocus. `Esc` pops focus back to the story (panel stays open);
        // anything the panel doesn't consume falls through to normal dispatch.
        if state.debug.is_some() && state.focus == Focus::Map {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    // Esc normally pops focus straight back to the story. But while
                    // the Memory address-input line is open, Esc must cancel that
                    // input first (handled by `handle_key` below) rather than
                    // immediately leaving the debug region — so skip the early pop
                    // in that case and fall through to the normal handle_key call.
                    let editing_mem_addr = state.debug.as_ref().is_some_and(|p| p.mem_input.is_some());
                    if k.code == crossterm::event::KeyCode::Esc && !editing_mem_addr {
                        state.focus = Focus::Game; // pop back to typing; keep the panel open
                        continue;
                    }
                    // Tab / Shift-Tab drive the unified per-window focus cycle
                    // (story pane + each debug window), not the panel — so from the
                    // last debug window Tab returns to the story.
                    if k.code == crossterm::event::KeyCode::Tab {
                        state.cycle_focus(true);
                        continue;
                    }
                    if k.code == crossterm::event::KeyCode::BackTab {
                        state.cycle_focus(false);
                        continue;
                    }
                    // Focused-pane height for PageUp/PageDown paging, from the real
                    // debug-region rect (the map slot).
                    let vp = (last_panes.map.height.saturating_sub(2) / 2).max(1) as usize;
                    let dk = if let Some(dbg) = session.debugger() {
                        state.debug.as_mut().map(|p| { p.viewport = vp; p.handle_key(k.code, dbg) })
                    } else {
                        None
                    };
                    // Consume keys the panel handled; let anything it ignored fall through
                    // to normal global dispatch (e.g. quit).
                    if dk == Some(app::debug_panel::DebugKey::Consumed) { continue; }
                    if dk == Some(app::debug_panel::DebugKey::Close) {
                        state.debug = None;
                        state.focus = Focus::Game;
                        continue;
                    }
                    // Ignored → fall through (do not `continue`).
                }
            }
        }

        // ── Debug inspector (tiled): mouse ──────────────────────────────────────
        // Works regardless of focus, whenever the cursor is over the debug region
        // (the map slot while `state.debug` is `Some`). Runs before the big
        // `match event` below so it pre-empts the `Event::Mouse` arm's map
        // wheel/click handling, which would otherwise misfire against the blank
        // map area the debug region currently occupies.
        if state.debug.is_some() {
            if let Event::Mouse(m) = &event {
                use crossterm::event::{MouseButton, MouseEventKind};
                let region = last_panes.map;
                let in_region = region.width > 0 && m.column >= region.x && m.column < region.right()
                    && m.row >= region.y && m.row < region.bottom();
                if in_region {
                    let rects = app::debug_panel::window_rects(region);
                    let over = rects.iter().position(|r| m.column >= r.x && m.column < r.right()
                        && m.row >= r.y && m.row < r.bottom());
                    match m.kind {
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                            if let Some(i) = over {
                                let down = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert)
                                    .map(|d| d > 0).unwrap_or(false);
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() { p.scroll_active(i, down, dbg); }
                                }
                            }
                            continue; // pre-empt the map wheel arms
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            // Clickable code-address navigation (branch targets in
                            // Disasm, `fn@` frame addresses in Call Stack) takes
                            // priority over the tab/body-focus logic below.
                            let target = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::clickable_at(region, p, m.column, m.row));
                            if let Some(target) = target {
                                use app::debug_panel::ClickTarget;
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() {
                                        match target {
                                            ClickTarget::Code(a)   => p.goto(a, dbg),
                                            ClickTarget::Memory(a) => p.goto_memory(a, dbg),
                                            ClickTarget::Object(n) => p.goto_object(n, dbg),
                                            // `@local5`/`@g0f`/`@sp` — a variable holding
                                            // a memory address: jump to memory at its
                                            // current value (read now via the debugger).
                                            ClickTarget::MemVia(v) => {
                                                if let Some(addr) = dbg.var_value(v) {
                                                    p.goto_memory(addr as u32, dbg);
                                                }
                                            }
                                            // `obj#local5`/`obj#g0f`/`obj#sp` — a variable
                                            // holding an object number: expand that object.
                                            ClickTarget::ObjVia(v) => {
                                                if let Some(obj) = dbg.var_value(v) {
                                                    p.goto_object(obj, dbg);
                                                }
                                            }
                                            // Plain variables are hover-only (see the
                                            // `Moved` arm); `clickable_at` doesn't return
                                            // them, but the match must cover them.
                                            ClickTarget::Global(_) | ClickTarget::Local(_) | ClickTarget::Stack => {}
                                        }
                                    }
                                    state.focus = Focus::Map;
                                }
                                continue;
                            }
                            // Object-tree expand/collapse click (Objects tab) — same
                            // priority as the code-address clicks above.
                            let obj_target = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::objects_click_at(region, p, m.column, m.row));
                            if let Some(obj) = obj_target {
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() { p.toggle_object(obj, dbg); }
                                    state.focus = Focus::Map;
                                }
                                continue;
                            }
                            // Call-stack frame expand/collapse click (Call Stack
                            // tab). The `fn@` address-click above runs first and
                            // `continue`s on hit, so it keeps priority over toggle.
                            let frame_target = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::stack_click_at(region, p, m.column, m.row));
                            if let Some(idx) = frame_target {
                                if let Some(dbg) = session.debugger() {
                                    if let Some(p) = state.debug.as_mut() { p.toggle_frame(idx, dbg); }
                                    state.focus = Focus::Map;
                                }
                                continue;
                            }
                            if let Some((w, t)) = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::tab_at(region, p, m.column, m.row)) {
                                if let Some(p) = state.debug.as_mut() { p.activate_tab(w, t); }
                                state.focus = Focus::Map;
                            } else if let Some((win, pt)) = app::debug_panel::debug_point_at(region, m.column, m.row) {
                                // Click in a window's body starts a text selection there (SQ-0420).
                                if let Some(p) = state.debug.as_mut() {
                                    p.focus_window(win);
                                    p.sel = Some((win, app::clipboard::Selection::new(pt)));
                                    p.selection_text.borrow_mut().take();
                                }
                                state.focus = Focus::Map;
                            } else if let Some(i) = over {
                                if let Some(p) = state.debug.as_mut() { p.focus_window(i); }
                                state.focus = Focus::Map;
                            }
                            continue;
                        }
                        MouseEventKind::Moved => {
                            // Hovering a variable operand (`gNN`/`localN`/`sp`) in
                            // the Disassembly shows a floating value tooltip at the
                            // cursor; moving off clears it. Borrows don't overlap:
                            // `as_ref` for the hit-test, then `session.debugger()`,
                            // then `as_mut`.
                            let hv = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::hover_var_at(region, p, m.column, m.row));
                            if let Some((var, acol, arow)) = hv {
                                let value = session.debugger().and_then(|dbg| dbg.var_value(var));
                                if let Some(p) = state.debug.as_mut() {
                                    p.hover = Some(app::debug_panel::HoverTip::for_var(var, value, acol, arow));
                                }
                                continue;
                            }
                            // Otherwise, hovering an opcode mnemonic shows its help.
                            let hh = state.debug.as_ref()
                                .and_then(|p| app::debug_panel::hover_help_at(region, p, m.column, m.row));
                            match hh {
                                Some((addr, acol, arow)) => {
                                    let lines = session.debugger().and_then(|dbg| dbg.describe_line(addr));
                                    if let Some(p) = state.debug.as_mut() {
                                        p.hover = lines.map(|l| app::debug_panel::HoverTip::for_lines(l, acol, arow));
                                    }
                                }
                                None => { if let Some(p) = state.debug.as_mut() { p.hover = None; } }
                            }
                            continue;
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            // Extend the active selection, clamped to its window (SQ-0420).
                            if let Some(p) = state.debug.as_mut() {
                                if let Some((win, sel)) = p.sel.as_mut() {
                                    sel.head = app::debug_panel::debug_point_clamped(region, *win, m.column, m.row);
                                }
                            }
                            continue;
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            // Release: copy the selected text (published by render) via
                            // OSC 52, then clear the selection (SQ-0420). Mirrors the
                            // story pane's EndSelection copy.
                            let copied = state.debug.as_mut().and_then(|p| {
                                let real = matches!(p.sel, Some((_, s)) if !s.is_empty());
                                p.sel = None;
                                if real { p.selection_text.borrow_mut().take() } else { None }
                            });
                            if let Some(text) = copied {
                                if !text.trim().is_empty() {
                                    use std::io::Write;
                                    let seq = app::clipboard::osc52_copy_sequence(&text);
                                    let mut out = std::io::stdout();
                                    let _ = out.write_all(seq.as_bytes());
                                    let _ = out.flush();
                                    state.push_transcript_internal(
                                        &format!("Copied {} chars to clipboard", text.chars().count()),
                                        app::state::TranscriptKind::Meta,
                                    );
                                }
                            }
                            continue;
                        }
                        _ => { continue; } // swallow other mouse over the region
                    }
                }
            }
        }

        // ── Config-screen Tab focus intercept ────────────────────────────────
        // Ring length 2: [Save(0), Cancel(1)].
        if state.overlays.config_screen.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        crossterm::event::KeyCode::Tab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 2, 1),
                        crossterm::event::KeyCode::BackTab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 2, -1),
                        _ => {}
                    }
                }
            }
        }

        // ── Saves Tab focus intercept ─────────────────────────────────────────
        // Ring length 1: [Done(0)].
        if state.overlays.saves.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        crossterm::event::KeyCode::Tab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 1, 1),
                        crossterm::event::KeyCode::BackTab =>
                            state.overlays.dialog_focus = app::input::cycle_focus(state.overlays.dialog_focus, 1, -1),
                        _ => {}
                    }
                }
            }
        }

        // ── Char-input mode gate ──────────────────────────────────────────────
        // When the Z-machine is waiting for a single keypress (read_char) and no
        // overlay is open, forward the keystroke directly to the VM — unless it is
        // the hotkey-dialog prefix (Ctrl+K) or any Ctrl/Alt combo. Those are
        // reserved for app routing so the user can always escape (quit, hotkeys)
        // out of a read_char form; only plain keypresses become game input.
        if state.char_mode && !state.any_overlay_open() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    use crossterm::event::KeyModifiers;
                    let spec = app::keymap::KeySpec::from_key_event(*k);
                    // Ctrl/Alt combos (hotkeys, quit, etc.) are never game input —
                    // let them fall through to app routing so the user can always
                    // escape a read_char form. Only plain keypresses reach the VM.
                    let app_combo = k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                    if spec != state.hotkeys.prefix && !app_combo {
                        // Map to a neutral KeyInput and forward; the engine
                        // converts it (ZSCII for the Z-machine) and returns None
                        // for keys with no input meaning, which are ignored.
                        if let Some(result) = app::engine::key_event_to_input(*k)
                            .and_then(|ki| {
                                app::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("input_key({ki:?})"));
                                session.submit_key(ki)
                            })
                        {
                            if turn::apply_game_driven_result(
                                &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                            ) {
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
        }

        // ── Line-terminator key gate (SQ-0188) ────────────────────────────────
        // While the Z-machine is waiting for a *line* read, a special key the game
        // lists in its v5 terminating-characters table (arrows / function keys)
        // submits the current input line with THAT ZSCII terminator, instead of the
        // key's normal app behavior. Only plain (no Shift/Ctrl/Alt) arrows + F-keys
        // are candidates; every other key — and any non-terminator arrow/F-key —
        // falls through unchanged so it keeps its app behavior (history/scroll/pan).
        if !state.any_overlay_open()
            && zvm_session_opt(&*session).is_some_and(|z| z.pending_input() == app::session::InputKind::Line)
        {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    use crossterm::event::KeyModifiers;
                    let plain = !k.modifiers.intersects(
                        KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT,
                    );
                    if plain {
                        let term = app::engine::key_event_to_input(*k)
                            .and_then(|ki| zvm_session_opt(&*session).and_then(|z| z.line_key_terminator(&ki)));
                        if let Some(term) = term {
                            let cmd = state.take_input();
                            if !cmd.is_empty() {
                                state.record_command(&cmd);
                            }
                            state.turns += 1;
                            state.unsaved_progress = true;
                            let result = zvm_session_opt_mut(&mut *session)
                                .expect("z-machine line read is pending")
                                .submit_line_with_terminator(&cmd, term);
                            if turn::finish_command_turn(
                                &cmd, result, &mut state, &mut mapper, &mut *session,
                                &game_dir, &ifid, &arc_file, last_panes.map, &mut bg_tidy_counter,
                            ) {
                                break;
                            }
                            continue 'event_loop;
                        }
                    }
                }
            }
        }

        // Route event to an Action.
        let action = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                match key_to_command(&state, k) {
                    KeyResolve::Action(a) => a,
                    KeyResolve::Command(s, ctx) => {
                        let close_leader = state.overlays.hotkey_dialog;
                        let outcome = slash::parse_in_context(&s, state.config.command_prefix, ctx);
                        let should_break = dispatch_slash_outcome(
                            outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                            &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                            last_panes.map, last_panes.story, true,
                        );
                        if close_leader {
                            state.overlays.hotkey_dialog = false;
                        }
                        lifecycle::flush_pending_config_write(&mut state);
                        if should_break {
                            break;
                        }
                        continue 'event_loop;
                    }
                    KeyResolve::None => Action::None,
                }
            }
            Event::Mouse(m) => {
                // Glk mouse input: a left-Down inside a mouse-watching Glulx window
                // is delivered to the game as an Evtype_MouseInput, not a UI action.
                // Only left-Down is diverted (Glk mouse is click-only, so the Drag/Up
                // selection events still arrive but fire no StartSelection and are
                // harmless no-ops); glk_mouse_target enforces no-overlay + inside a
                // watching window and computes the window-relative coordinates.
                // Glk hyperlink input: a left-Down on a linked transcript cell whose
                // owning window has an active hyperlink request is delivered to the
                // game as an Evtype_Hyperlink carrying the cell's link value. A link
                // cell is a more specific target than a general mouse-window click, so
                // this runs first; a non-link click (or no watching window) falls
                // through to the mouse path, then to the app's own handling.
                if matches!(m.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)) {
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        if let Some(&(_, link)) = last_panes
                            .transcript_links
                            .iter()
                            .find(|((cx, cy), _)| *cx == m.column && *cy == m.row)
                        {
                            if link != 0 {
                                let windows = gs.hyperlink_windows();
                                if !windows.is_empty() {
                                    let s = last_panes.story;
                                    if let Some(win) = app::glulx_session::glk_hyperlink_window(
                                        state.any_overlay_open(),
                                        m.column, m.row,
                                        (s.x, s.y, s.width, s.height),
                                        &windows,
                                    ) {
                                        let result = gs.deliver_hyperlink(win, link);
                                        if turn::apply_game_driven_result(
                                            &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                                        ) {
                                            break 'event_loop;
                                        }
                                        continue 'event_loop;
                                    }
                                }
                            }
                        }
                    }
                }
                if matches!(m.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)) {
                    if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                        let windows = gs.mouse_windows();
                        if !windows.is_empty() {
                            let s = last_panes.story;
                            let target = app::glulx_session::glk_mouse_target(
                                state.any_overlay_open(),
                                m.column, m.row,
                                (s.x, s.y, s.width, s.height),
                                &windows,
                                gs.char_pixels(),
                            );
                            if let Some((win, vx, vy)) = target {
                                let result = gs.deliver_mouse(win, vx, vy);
                                if turn::apply_game_driven_result(
                                    &mut state, &mut mapper, &result, &game_dir, last_panes.map, &*session,
                                ) {
                                    break 'event_loop;
                                }
                                continue 'event_loop;
                            }
                        }
                    }
                }
                // Map layer tab: a left-click on a layer tab selects that layer as the
                // viewed one (hit-rects captured per frame in last_panes.layer_tabs).
                if !state.any_overlay_open() {
                    if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = m.kind {
                        let hit = last_panes.layer_tabs.iter().find(|(_, r)| {
                            r.width > 0 && r.height > 0
                                && m.column >= r.x && m.column < r.right()
                                && m.row >= r.y && m.row < r.bottom()
                        });
                        if let Some(&(layer, _)) = hit {
                            apply_action(Action::SetViewedLayer(layer), &mut state, &mut mapper);
                            continue 'event_loop;
                        }
                    }
                }
                // Verb dock: click a token to insert it; click a header to focus that section; click the
                // story pane to return keyboard focus there (then fall through to normal story handling).
                if state.overlays.verb_menu.is_some() {
                    if let crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) = m.kind {
                        let inside = |r: &ratatui::layout::Rect| {
                            r.width > 0 && r.height > 0 && m.column >= r.x && m.column < r.right() && m.row >= r.y && m.row < r.bottom()
                        };
                        if let Some((pane, idx, _)) = last_panes.verb_menu.rows.iter().find(|(_, _, r)| inside(r)).copied() {
                            apply_action(Action::VerbMenuClickToken(pane, idx), &mut state, &mut mapper);
                            continue 'event_loop;
                        }
                        if let Some((pane, _)) = last_panes.verb_menu.headers.iter().find(|(_, r)| inside(r)).copied() {
                            apply_action(Action::VerbMenuFocusPane(pane), &mut state, &mut mapper);
                            continue 'event_loop;
                        }
                        if inside(&last_panes.story) {
                            if let Some(vm) = &mut state.overlays.verb_menu { vm.story_focused = true; }
                            // fall through: normal story-pane click handling (selection) still runs below.
                        }
                    }
                }
                mouse_to_action(&state, m, last_panes.map, last_panes.story, &last_panes.room_rects, &last_panes.dialog)
            }
            // Resize: continue so the next draw uses the updated terminal size.
            // Resize: force a full repaint so no stale cells survive the size change.
            Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
            _ => continue,
        };

        // ToggleWatch is run-loop-only (owns the watcher): intercept before dispatch.
        if matches!(action, Action::ToggleWatch) {
            toggle_style_watch(&mut state, &mut style_watcher);
            continue;
        }

        // Snapshot working config before apply_action clears it on ConfigSave.
        let config_to_save = if matches!(action, Action::ConfigSave) {
            state.overlays.config_screen.as_ref().map(|cs| cs.working.clone())
        } else {
            None
        };
        // Mouse capture is established once at startup; note its pre-save value so a
        // settings-screen change can be applied to the live terminal below.
        let mouse_before_save = state.config.mouse;
        // Likewise note command_bar so a settings-screen toggle re-applies the
        // session's prompt-stripping live (else render mode and strip_prompt desync
        // until the next @restart).
        let command_bar_before_save = state.config.command_bar;

        match action {
            // ── Caller-handled actions ─────────────────────────────────────────

            Action::Quit => {
                if should_prompt_save_on_quit(&state) {
                    state.overlays.quit_dialog = true;
                    state.overlays.dialog_focus = 0;
                } else {
                    break;
                }
            }

            // Story-pane selection released: copy the text extracted by render from
            // the full wrapped transcript (off-screen rows included) via OSC 52.
            Action::EndSelection => {
                state.selection = None;
                state.selection_edge = 0;
                let copied = state.selection_text.borrow_mut().take();
                if let Some(text) = copied {
                    if !text.trim().is_empty() {
                        use std::io::Write;
                        let seq = app::clipboard::osc52_copy_sequence(&text);
                        let mut out = std::io::stdout();
                        let _ = out.write_all(seq.as_bytes());
                        let _ = out.flush();
                        // Report the copy as a meta line in the story output rather
                        // than a status-bar message (which has no natural dismissal).
                        state.push_transcript_internal(
                            &format!("Copied {} chars to clipboard", text.chars().count()),
                            app::state::TranscriptKind::Meta,
                        );
                    }
                }
                continue;
            }

            Action::SubmitCommand(_) => {
                // A Glulx game waiting on a timer/mouse/hyperlink event only has no
                // line request pending: Enter has nothing to submit. Swallow it
                // (keeping the typed buffer intact for the real prompt) rather than
                // feed a stray line the VM would only diagnose.
                if session.pending_input() == app::session::InputKind::Event {
                    continue;
                }

                // Normal game-focus command submission.
                // Clear input line and echo command.
                let cmd = state.take_input();

                // An empty cmd (Enter on a blank line) is still submitted to the
                // game, which decides what a blank line means (re-prompt / "I beg
                // your pardon?"), matching other interpreters (SQ-0265). Only skip
                // history recording and slash routing for it — an empty line is
                // neither worth a history entry nor a slash command.
                if !cmd.is_empty() {
                    // Record into the shell-style command history (game + slash
                    // alike), deduping consecutive repeats and capping the list.
                    state.record_command(&cmd);

                    // ── Slash-command interception ────────────────────────────
                    // If the input starts with the configured prefix, route it as
                    // an app command; do NOT call session.submit, increment turns,
                    // or push a "> cmd" story line.
                    if is_slash(&cmd, state.config.command_prefix) {
                        // Strip the leading prefix character before parsing.
                        let body = &cmd[state.config.command_prefix.len_utf8()..];
                        let outcome = slash::parse(body, state.config.command_prefix);
                        let should_break = dispatch_slash_outcome(
                            outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                            &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                            last_panes.map, last_panes.story, false,
                        );
                        lifecycle::flush_pending_config_write(&mut state);
                        if should_break {
                            break;
                        }
                        continue;
                    }
                }

                // Increment the session turn counter. Progress now exists that
                // isn't captured in a Save State (drives the quit prompt).
                state.turns += 1;
                state.unsaved_progress = true;

                app::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("input_line({cmd:?})"));
                let result = session.submit(&cmd);
                if turn::finish_command_turn(
                    &cmd, result, &mut state, &mut mapper, &mut *session,
                    &game_dir, &ifid, &arc_file, last_panes.map, &mut bg_tidy_counter,
                ) {
                    break;
                }
            }

            Action::SaveGame => {
                // Dead post-unification: keys now route through SlashOutcome::Save. Retained as a no-cost match arm.
                // Bundle map + game into a single .babelmap archive, with turn metadata.
                let (location, score) = crate::engine_helpers::save_summary(&*session, &state);
                let meta = app::archive::Meta {
                    format_version: app::archive::CURRENT_FORMAT_VERSION,
                    ifid: Some(ifid.clone()),
                    name: None,
                    turns: state.turns,
                    saved_at: {
                        use std::time::{SystemTime, UNIX_EPOCH};
                        let secs = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        // Re-use a simple format: delegate to persist_files helper would be
                        // cleaner but it's private; inline the same logic here.
                        format_rfc3339(secs)
                    },
                    location,
                    score,
                };
                match save_archive_meta(&arc_file, &mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.transcript_para, &state.history, &state.command_history) {
                    Ok(()) => {
                        state.push_notice(&format!(
                            "[Game saved to {}]",
                            arc_file.display()
                        ));
                    }
                    Err(e) => {
                        state.push_notice(&format!("[Save failed: {}]", e));
                    }
                }
            }

            Action::RestoreGame => {
                // Dead post-unification: keys now route through SlashOutcome::Load. Retained as a no-cost match arm.
                // Restore map + game from the .babelmap archive.
                match load_archive(&arc_file) {
                    Ok(ac) => {
                        let restore_err = session.restore_state(&ac.engine_save()).map_err(restore_error_msg);
                        match restore_err {
                            Ok(()) => {
                                if let Some(scr) = ac.screen.clone() {
                                    if let Some(z) = zvm_session_opt_mut(&mut *session) { z.machine.screen = scr; }
                                }
                                if state.config.aux_storage != app::config::AuxStorage::Global {
                                    session.set_aux_data(ac.aux.clone());
                                }
                                mapper = ac.mapper;
                                state.transcript = ac.transcript;
                                state.clear_anchor = None;
                                state.transcript_kinds = ac.transcript_kinds;
                                state.transcript_runs = ac.transcript_runs;
                                state.transcript_para = ac.transcript_para;
                                state.reset_transcript_sidecars();
                                state.history = ac.history;
                                state.command_history = ac.command_history;
                                // After restore, re-observe current location.
                                reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                                state.push_notice(&format!(
                                    "[Game restored from {}]",
                                    arc_file.display()
                                ));
                            }
                            Err(e) => {
                                state.push_notice(&format!("[Restore failed: {}]", e));
                            }
                        }
                    }
                    Err(e) => {
                        state.push_notice(&format!("[Restore failed: {}]", e));
                    }
                }
            }

            // SQ-0297: shared with the slash-command path via handle_map_export
            // (dispatch_slash_outcome never reaches this match).
            a @ (Action::ExportSvg(_) | Action::ExportDot(_) | Action::ExportMap(_)) => {
                handle_map_export(&a, &game_dir, &mapper, &mut state);
            }

            // ── Saves-manager actions ─────────────────────────────────────────

            Action::OpenSaves => {
                // Populate the saves list (both .babelmap Save States and .qzl
                // game saves — SQ-0227 Task 3) and open the modal.
                let entries = combined_saves(&game_dir);
                state.overlays.saves = Some(SavesState { entries, scroll: Default::default() });
                state.overlays.dialog_focus = 0;
            }

            Action::SavesImport => {
                // Close saves modal and open file browser in PickFile mode.
                // Start in this story's per-game dir (where its saves live, honoring
                // --data-dir), falling back to the data base then the user dir.
                state.overlays.saves = None;
                let start_dir = if game_dir.is_dir() {
                    game_dir.clone()
                } else if data_base.is_dir() {
                    data_base.clone()
                } else {
                    state.config.user_dir.clone()
                };
                state.overlays.file_browser = Some(FileBrowserState::build(start_dir, FbMode::PickFile));
            }

            Action::FbEnter => {
                // Handle file-browser Enter: cd into dir or import file.
                let fb_action = state.overlays.file_browser.as_ref().and_then(|fb| {
                    fb.entries.get(fb.scroll.selected).map(|e| {
                        if e.is_dir {
                            let new_path = if e.name == ".." {
                                fb.cwd.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| fb.cwd.clone())
                            } else {
                                fb.cwd.join(&e.name)
                            };
                            FbEntryAction::CdInto(new_path)
                        } else {
                            FbEntryAction::ImportFile(fb.cwd.join(&e.name))
                        }
                    })
                });
                match fb_action {
                    Some(FbEntryAction::CdInto(path)) => {
                        if let Some(fb) = &mut state.overlays.file_browser {
                            fb.cd(path);
                        }
                    }
                    Some(FbEntryAction::ImportFile(path)) => {
                        state.overlays.file_browser = None;
                        if !engine_supports_save(&*session) {
                            state.set_status("Restore is not supported for Glulx games yet");
                            continue;
                        }
                        match restore_game(&path, &mut zvm_session_mut(&mut *session).machine) {
                            Ok(()) => {
                                // Re-observe current location (same as RestoreGame/SavesLoad).
                                reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                                state.push_notice(&format!("[Imported: {}]", path.display()));
                            }
                            Err(e) => {
                                state.push_notice(&format!("[Import failed: {}]", e));
                            }
                        }
                    }
                    None => {}
                }
            }

            Action::SavesLoad => {
                // Load the selected save (archive → mapper + machine restore).
                // Clone path and name to release the borrow on state.overlays.saves before mutating state.
                let load_info = state.overlays.saves.as_ref().and_then(|s| {
                    s.entries.get(s.scroll.selected).map(|e| (e.path.clone(), e.name.clone()))
                });

                // In-game restore of a .qzl game save: feed Quetzal bytes back
                // into the suspended VM, completing the @restore descriptor
                // (unchanged). A .babelmap Save State picked here instead falls
                // through below to a full session resume (SQ-0227 Task 3).
                if state.ingame_io == Some(app::session::PendingIo::Restore)
                    && load_info.as_ref().is_some_and(|(path, _)| app::persist_files::is_game_save(path))
                {
                    let Some((path, entry_name)) = load_info else { continue };
                    state.overlays.saves = None;
                    state.ingame_io = None;
                    let result = match app::archive::read_quetzal_from_file(&path) {
                        Ok(bytes) => {
                            state.push_notice(&format!("[Game restored from {}]", entry_name));
                            session.resume_restore(Some(&bytes))
                        }
                        Err(e) => {
                            state.push_notice(&format!("[Restore failed: {}]", e));
                            session.resume_restore(None)
                        }
                    };
                    let quit = turn::finish_resumed_turn(result, &mut mapper, &mut state, &*session, &game_dir, &ifid, last_panes.map);
                    turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                    turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                    if let Some(io) = state.ingame_io {
                        open_ingame_saves(io, &game_dir, &mut state);
                    }
                    if quit { break; }
                    continue;
                }

                // Host Load (also reached for a .babelmap picked while an
                // in-game @restore is pending: that fully resumes, abandoning
                // the pending call; on failure the pending @restore is still
                // answered with resume_restore(None) so the VM isn't left
                // blocked waiting for a result).
                let ingame_restore_pending = state.ingame_io == Some(app::session::PendingIo::Restore);
                if let Some((path, entry_name)) = load_info {
                    match restore_from_file(&path, &mut *session) {
                        Ok(RestoreOutcome::DescriptorCompleted) => {
                            state.overlays.saves = None;
                            reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                            state.push_notice(&format!("[Game restored from {}]", entry_name));
                        }
                        Ok(RestoreOutcome::Resumed(ac)) => {
                            state.ingame_io = None;
                            if let Some(scr) = ac.screen.clone() {
                                if let Some(z) = zvm_session_opt_mut(&mut *session) { z.machine.screen = scr; }
                            }
                            if state.config.aux_storage != app::config::AuxStorage::Global {
                                session.set_aux_data(ac.aux.clone());
                            }
                            mapper = ac.mapper;
                            state.transcript = ac.transcript;
                            state.clear_anchor = None;
                            state.transcript_kinds = ac.transcript_kinds;
                            state.transcript_runs = ac.transcript_runs;
                            state.transcript_para = ac.transcript_para;
                            state.reset_transcript_sidecars();
                            state.history = ac.history;
                            // Named-slot archives carry no command history; only
                            // adopt it when present so a slot load doesn't wipe it.
                            if !ac.command_history.is_empty() {
                                state.command_history = ac.command_history;
                            }
                            // Restore turn counter from the loaded archive.
                            state.turns = ac.meta.turns;
                            // Re-observe current location.
                            reobserve_location(&mut state, &mut mapper, &*session, last_panes.map);
                            state.push_notice(&format!("[Loaded save: {}]", entry_name));
                            state.overlays.saves = None;
                        }
                        Err(e) => {
                            state.push_notice(&format!("[Load failed: {}]", e));
                            if ingame_restore_pending {
                                state.overlays.saves = None;
                                state.ingame_io = None;
                                let result = session.resume_restore(None);
                                let quit = turn::finish_resumed_turn(result, &mut mapper, &mut state, &*session, &game_dir, &ifid, last_panes.map);
                                turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                                turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
                                if let Some(io) = state.ingame_io {
                                    open_ingame_saves(io, &game_dir, &mut state);
                                }
                                if quit { break; }
                                continue;
                            }
                        }
                    }
                }
            }

            // ── Replay/rewind: linear resume from the selected turn ────────────
            Action::ReplayResume => {
                if let Some(r) = state.overlays.replay.take() {
                    if r.idx < state.history.len() {
                        let plan = app::history::resume_plan(&state.history, r.idx);
                        // History snapshots come from the running engine; wrap them
                        // with its tag so restore_state accepts them (both engines).
                        let es = app::engine::EngineSave::new(engine_tag(&*session), 1, plan.save.clone());
                        match session.restore_state(&es) {
                            Ok(()) => {
                                if let Some(json) = &plan.map_json {
                                    if let Ok(m) = mapper::persist::from_json(json) {
                                        mapper = m;
                                    }
                                }
                                // Linear: discard later turns.
                                state.history.truncate(r.idx + 1);
                                let (lines, kinds) =
                                    app::history::rebuild_transcript(&state.history, r.idx);
                                state.transcript = lines;
                                state.clear_anchor = None;
                                state.transcript_kinds = kinds;
                                // History replay carries no style runs; keep the
                                // parallel vecs length-synced (unstyled, left rows).
                                state.transcript_runs = vec![Vec::new(); state.transcript.len()];
                                state.transcript_para = vec![app::state::ParaFmt::default(); state.transcript.len()];
                                state.reset_transcript_sidecars();
                                state.turns = plan.turn;
                                state.unsaved_progress = false; // resumed a past (saved) turn
                                state.graph_gen = state.graph_gen.wrapping_add(1);
                                // Re-observe current location (mirror the restore path).
                                if let Some(snap) = session.current_location() {
                                    let rid = snap.number as mapper::graph::RoomId;
                                    let restore_result = TurnResult {
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
                                    apply_turn(&mut mapper, "", &restore_result);
                                    state.set_viewed_layer(None);
                                    state.select_room(Some(rid));
                                }
                                state.push_notice(&format!("[Resumed from turn {}]", plan.turn));
                            }
                            Err(e) => {
                                state.push_notice(&format!("[Resume failed: {}]", restore_error_msg(e)));
                            }
                        }
                    }
                }
            }

            // ── Open hints panel ──────────────────────────────────────────────
            Action::OpenHints => {
                let sp = story_path.clone();
                let id = ifid.clone();
                let ud = state.config.user_dir.clone();
                open_hints(&mut state, &sp, &id, &ud);
            }

            // Page the transcript by one screenful. Resolved here because it needs
            // the last-rendered transcript viewport height and max scroll.
            Action::TranscriptScrollPage(dir) => {
                let target = app::input::page_scroll(
                    state.transcript_scroll,
                    dir,
                    last_panes.transcript_viewport_rows,
                    last_panes.transcript_max_scroll,
                );
                state.scroll_transcript_to(target);
            }
            // [more] pager (SQ-0404): page one screen toward the bottom; reaching
            // the bottom (offset 0) catches up and exits the pager.
            Action::PagerAdvance => {
                let target = app::input::page_scroll(
                    state.transcript_scroll,
                    -1,
                    last_panes.transcript_viewport_rows,
                    last_panes.transcript_max_scroll,
                );
                state.scroll_transcript_to(target);
                if target == 0 {
                    state.pager.active = false;
                }
            }
            // [more] pager: jump straight to the newest output and exit.
            Action::PagerDismiss => {
                state.scroll_transcript_to(0);
                state.pager.active = false;
            }

            // ── apply_action handles everything else ───────────────────────────
            other => {
                apply_action(other, &mut state, &mut mapper);
            }
        }

        // After apply_action: if a sound toggle / config save flipped enable_sound,
        // sync the running Glulx VM's Sound gestalt so games that re-check
        // gestalt_Sound per play (e.g. sensory.blorb's gong) honor the change.
        if let Some(on) = state.pending_vm_sound.take() {
            if let Some(gs) = glulx_session_opt_mut(&mut *session) {
                gs.set_sound(on);
            }
        }

        // After dispatch: resume an in-game (v4+) save/restore whose dialog was
        // just confirmed (flag-hop) or cancelled (overlay closed without confirm).
        let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
            || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
        turn::persist_aux_after_turn(&mut *session, &mut state, &game_dir);
        turn::persist_vfs_after_turn(&mut *session, &state, &game_dir);
        if quit {
            break;
        }

        // After apply_action: if resize mode was just exited or reset, persist the
        // (possibly changed) pane sizes to config.toml. Also covers the
        // `KeyResolve::Command` dispatch path via the `flush_pending_config_write`
        // calls placed right before its `continue`s above.
        lifecycle::flush_pending_config_write(&mut state);

        // After apply_action: if config screen was just saved, write the resolved look
        // to the personal style file and repoint config.toml at it.
        if let Some(cfg_to_write) = config_to_save {
            let user_dir = cfg_to_write.user_dir.clone();
            save_style_and_repoint(&mut state, &user_dir);
            // Apply a mouse-capture change live so the setting takes effect without a
            // restart (matching how audio/colours apply live on save).
            if cfg_to_write.mouse != mouse_before_save {
                let _ = if cfg_to_write.mouse {
                    execute!(stdout(), EnableMouseCapture)
                } else {
                    execute!(stdout(), DisableMouseCapture)
                };
            }
            // Re-apply prompt stripping live so toggling the command bar on/off in
            // Settings takes effect on the next turn without a restart (inline mode
            // keeps the game's `>`, command-bar mode strips it).
            if cfg_to_write.command_bar != command_bar_before_save {
                session.set_strip_prompt(cfg_to_write.command_bar);
            }
        }

    }

    // ── 6. Exit: restore terminal + (optional) autosave ───────────────────────

    restore_terminal();

    lifecycle::exit_auto_save(&*session, &mapper, &state, &ifid, &arc_file);
}

// ── Reset helper ──────────────────────────────────────────────────────────────

// Rebuild the session from `story_bytes`, reset all ephemeral state, and
// re-seed the mapper with the start room.  When `clear_map` is true, the
// accumulated map is wiped first (same effect as `/reset map`) so only the
// start room remains after the re-seed.

/// Resolve the Pict/graphics blorb for a story the same way at launch and
/// restart: path-based (self-contained blorb, same-stem sidecar, or dir scan).
fn resolve_pict_blorb(story_path: &std::path::Path, images: bool) -> Option<blorb::Blorb> {
    if images {
        blorb::resolve_resource_blorb(story_path).map(|(b, _)| b)
    } else {
        None
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Whether the game echoed the just-submitted command itself at the start of its
/// turn output (e.g. CounterfeitMonkey prints the command back in bold). Compared
/// case-insensitively against the leading non-whitespace text, and only when the
/// echo ends at a boundary (so `go` doesn't match a response starting `gospel`),
/// so we don't add a second, redundant echo. An empty command never matches.
fn game_echoes_command(transcript: &str, cmd: &str) -> bool {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return false;
    }
    let mut head = transcript.trim_start().chars();
    for cc in cmd.chars() {
        match head.next() {
            Some(hc) if hc.eq_ignore_ascii_case(&cc) => {}
            _ => return false,
        }
    }
    // The command must be followed by a boundary, not more word characters.
    match head.next() {
        None => true,
        Some(c) => !c.is_alphanumeric(),
    }
}

/// The current story's saves for the saves manager: `.babelmap` Save States and
/// `.qzl` game saves in `game_dir` merged into one list, sorted newest-first by
/// save time. RFC3339 timestamps sort chronologically as strings; untimestamped/
/// legacy saves (empty timestamp) sort to the bottom.
fn combined_saves(game_dir: &std::path::Path) -> Vec<app::persist_files::SaveInfo> {
    let mut entries = list_saves(game_dir);
    entries.extend(app::persist_files::list_qzl(game_dir));
    entries.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    entries
}

/// Format a Unix timestamp (seconds since epoch) as an RFC3339 UTC string.
fn format_rfc3339(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd_main(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month, day, hour, min, sec)
}

fn days_to_ymd_main(mut days: u64) -> (u64, u64, u64) {
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Return (width, height) of the map pane, defaulting to (80, 24) when zero.
fn map_pane_dims(area: Rect) -> (u16, u16) {
    let w = if area.width == 0 { 80 } else { area.width };
    let h = if area.height == 0 { 24 } else { area.height };
    (w, h)
}

/// Re-observe the VM's current location after a restore/resume: fold the room into the
/// map, deselect the viewed layer, select the room, and recenter the map pane on it.
/// Produces no transcript output. Shared by every host restore/resume arm.
fn reobserve_location(
    state: &mut AppState,
    mapper: &mut Mapper,
    session: &dyn Engine,
    map_rect: Rect,
) {
    // Every caller is a restore/resume/import: the live state now equals a saved
    // one, so there is no unsaved progress to warn about on quit.
    state.unsaved_progress = false;
    // The caller has just swapped in a restored/imported mapper (or is about to
    // re-observe into it); invalidate the map render memo so the loaded map shows
    // this frame instead of the pre-restore one. Unconditional so even the
    // no-current-location early-return below still invalidates. (SQ-0305)
    state.bump_graph_gen();
    let Some(snap) = session.current_location() else { return };
    let rid = snap.number as mapper::graph::RoomId;
    let restore_result = TurnResult {
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
    apply_turn(mapper, "", &restore_result);
    state.set_viewed_layer(None);
    state.select_room(Some(rid));
    if let Some(room) = mapper.graph.room(rid) {
        if let Some(pos) = room.pos {
            let (pw, ph) = map_pane_dims(map_rect);
            state.recenter_on(pos, pw, ph);
        }
    }
}

/// Build a `DialogStyle` from the current app colors.
/// Note: `BorderStyle::None` is coerced to `Single` inside `draw_dialog`.
fn make_dialog_style(state: &AppState) -> DialogStyle {
    DialogStyle::from_colors(&state.colors)
}

/// Apply `Modifier::DIM` to every cell in `area` of `buf`.
/// Called after a pane's content is rendered to de-emphasise the unfocused pane.
fn dim_area(buf: &mut ratatui::buffer::Buffer, area: Rect) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(cell.style().add_modifier(Modifier::DIM));
            }
        }
    }
}

// ── Slash-command helper ──────────────────────────────────────────────────────

/// Return true when `input` starts with the configured command `prefix` char.
fn is_slash(input: &str, prefix: char) -> bool {
    input.starts_with(prefix)
}

// ── Quit dialog helpers ───────────────────────────────────────────────────────

// ── Hints open helper ─────────────────────────────────────────────────────────

/// Open the hints panel for the current story, resolving the hint source.
///
/// If a panel is already open this is a no-op.  Discovery order:
/// 1. Remembered per-IFID association.
/// 2. Sibling hint file.
/// 3. Inside a sibling ZIP.
/// 4. AskUser: status message + TODO for file-browser wiring.
/// 5. None: status "no hints found".
fn open_hints(
    state: &mut AppState,
    story_path: &std::path::Path,
    ifid: &str,
    user_dir: &std::path::Path,
) {
    if state.overlays.hints.is_some() {
        return;
    }

    // Built-in HINT detection: check story dictionary for "hint"/"hints".
    // state.dict_words is populated at startup from the story's Z-machine dictionary.
    let builtin_hint = hints::story_supports_hint(state.dict_words.iter().cloned());

    let index = hints::load_hint_index(user_dir);
    let resolution = hints::resolve_hint_source(story_path, ifid, &index);

    match resolution {
        hints::HintResolution::File(p) => {
            match hints::load_story_bytes(&p) {
                Ok(bytes) => {
                    match app::session::GameSession::new(bytes, state.config.honor_game_colours, false, state.config.interpreter_number) {
                        Ok(mut vm) => {
                            vm.machine.undo_cap = state.config.undo_levels;
                            let opening = vm.take_transcript();
                            let transcript: Vec<String> =
                                opening.split('\n').map(|l| l.to_owned()).collect();
                            let label = p
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("Hints")
                                .to_owned();
                            state.overlays.hints = Some(app::state::HintSession {
                                source: app::state::HintSource::Zcode(vm),
                                transcript,
                                scroll: 0,
                                scroll_anim: None,
                                input: String::new(),
                                label,
                                builtin_hint,
                            });
                        }
                        Err(e) => {
                            state.set_status(format!("hints: failed to load hint VM: {:?}", e));
                        }
                    }
                }
                Err(e) => {
                    state.set_status(format!("hints: cannot read hint file: {}", e));
                }
            }
        }
        hints::HintResolution::ZipEntry { zip_path, entry } => {
            let pred = |name: &str| name == entry;
            match hints::read_zip_entry(&zip_path, pred) {
                Ok(Some(bytes)) => {
                    match app::session::GameSession::new(bytes, state.config.honor_game_colours, false, state.config.interpreter_number) {
                        Ok(mut vm) => {
                            vm.machine.undo_cap = state.config.undo_levels;
                            let opening = vm.take_transcript();
                            let transcript: Vec<String> =
                                opening.split('\n').map(|l| l.to_owned()).collect();
                            let label = entry.rsplit('/').next().unwrap_or(&entry).to_owned();
                            state.overlays.hints = Some(app::state::HintSession {
                                source: app::state::HintSource::Zcode(vm),
                                transcript,
                                scroll: 0,
                                scroll_anim: None,
                                input: String::new(),
                                label,
                                builtin_hint,
                            });
                        }
                        Err(e) => {
                            state.set_status(format!("hints: failed to load hint VM: {:?}", e));
                        }
                    }
                }
                Ok(None) => {
                    state.set_status("hints: hint entry not found in zip");
                }
                Err(e) => {
                    state.set_status(format!("hints: cannot read zip entry: {}", e));
                }
            }
        }
        hints::HintResolution::AskUser => {
            // TODO: wire the file browser to pick a hint file (.z3/.z5/.z8), then call
            // save_hint_assoc(user_dir, ifid, &picked) and restart as File path above.
            // For now, surface a status message so the user knows what to do.
            state.set_status(
                "no hint file found — place <story>.hints.z5 next to the story, or use /hints <path>",
            );
        }
        hints::HintResolution::None => {
            state.set_status("no hints found");
        }
    }
}

/// Return true when a quit attempt should show the "Save state before quitting?" dialog.
///
/// Conditions: auto_save is off AND prompt_save_on_quit is on AND there is progress
/// not yet captured in a Save State (`unsaved_progress`) — so quitting right after a
/// Ctrl-S / save / load does not prompt.
fn should_prompt_save_on_quit(state: &AppState) -> bool {
    !state.config.auto_save && state.config.prompt_save_on_quit && state.unsaved_progress
}

// ── Scroll-to-match helper ────────────────────────────────────────────────────

/// Given a match at `match_visible_pos` (0-based) within `total_visible` visible rows,
/// return the `transcript_scroll` value that brings that row to the top of the viewport
/// (`pane_rows` high).
///
/// The windowing in `visible_wrapped_lines_kinded` uses:
///   end   = total_visible - scroll
///   start = end - pane_rows
/// So placing the match at the top of the viewport means:
///   end = match_visible_pos + pane_rows
///   scroll = total_visible - end = total_visible - match_visible_pos - pane_rows
/// Clamped to 0 when the match is near the bottom (no scrollback needed).
///
/// Limitation: this helper treats each logical visible line as one display row.
/// When a line wraps into multiple display rows the match may land slightly
/// off-screen; correct wrap-aware scrolling would require counting wrapped rows
/// for every line above the match, which is not done here.
fn scroll_for_match(match_visible_pos: usize, total_visible: usize, pane_rows: usize) -> u16 {
    total_visible
        .saturating_sub(match_visible_pos)
        .saturating_sub(pane_rows) as u16
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::{dim_area, is_slash, scroll_for_match, should_prompt_save_on_quit};
    use app::render::paneframe::{draw_pane_frame, draw_top_inset, InsetSegment, PaneGlyphs};

    // ── SQ-0297: map-export slash commands must actually write the file ────────

    #[test]
    fn handle_map_export_writes_the_file_into_the_game_dir() {
        use std::fs;
        use app::input::Action;
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let dir = std::env::temp_dir().join(format!("bm-handle-map-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let mapper = Mapper::default();
        let mut state = AppState::default();

        assert!(super::handle_map_export(&Action::ExportSvg(None), &dir, &mapper, &mut state));
        assert!(dir.join("map.svg").exists(), "SVG export must write map.svg into the game dir");

        assert!(super::handle_map_export(&Action::ExportDot(Some("mymap".into())), &dir, &mapper, &mut state));
        assert!(dir.join("mymap.dot").exists(), "DOT export with a bare-name arg must land in the game dir");

        assert!(super::handle_map_export(&Action::ExportMap(None), &dir, &mapper, &mut state));
        assert!(dir.join("map.txt").exists(), "dump export must write map.txt into the game dir");

        assert!(!super::handle_map_export(&Action::ToggleWatch, &dir, &mapper, &mut state),
            "a non-export action must not be treated as handled");

        let _ = fs::remove_dir_all(&dir);
    }

    // ── SQ-0230: list_qzl filters to the current story's game saves ─────────────

    #[test]
    fn list_qzl_lists_game_saves_in_game_dir_and_skips_babelmap() {
        use std::fs;
        // SQ-0284: all `.qzl` in a per-game dir belong to this story (no IFID
        // prefix filtering). `.babelmap` files are never picked up by list_qzl.
        let dir = std::env::temp_dir().join(format!("bm-listqzl-{}/Zork1.z5", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("slot1.qzl"), b"x").unwrap();
        fs::write(dir.join("slot1.babelmap"), b"x").unwrap();

        // combined_saves merges .babelmap + .qzl newest-first; here the .babelmap
        // has no valid archive so list_saves skips it, leaving the one game save.
        let combined: Vec<String> = super::combined_saves(&dir).iter().map(|s| s.name.clone()).collect();
        assert_eq!(combined, vec!["slot1".to_string()], "combined list includes the game save");

        let infos = app::persist_files::list_qzl(&dir);
        let names: Vec<String> = infos.iter().map(|s| s.name.clone()).collect();
        // The `.qzl` suffix is stripped to the slug for display; the `.babelmap`
        // is excluded from list_qzl.
        assert_eq!(names, vec!["slot1".to_string()]);
        // And they carry a save timestamp read from the file's mtime.
        assert!(!infos[0].saved_at.is_empty(), "game saves are timestamped from file mtime");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn combined_saves_sorts_newest_first_untimestamped_last() {
        let mk = |name: &str, ts: &str| app::persist_files::SaveInfo {
            path: std::path::PathBuf::from(format!("/tmp/{name}.qzl")),
            name: name.to_string(),
            turns: 0,
            saved_at: ts.to_string(),
            location: None, score: None, is_default: false,
        };
        let mut v = [mk("old", "2026-06-01T10:00:00Z"),
            mk("legacy", ""),
            mk("new", "2026-07-09T12:00:00Z"),
            mk("mid", "2026-06-30T08:00:00Z")];
        // Same comparator combined_saves uses (RFC3339 sorts chronologically).
        v.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        let order: Vec<&str> = v.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["new", "mid", "old", "legacy"],
            "newest first; untimestamped/legacy saves sort to the bottom");
    }

    /// Minimal v4 story: `read_char` (store->G0) at 0x40, then `@save` (store
    /// form, ->G0) at 0x44, then `quit` at 0x46. Mirrors session.rs's
    /// (crate-private) `read_char_then_save_v4` fixture, duplicated here
    /// since this test lives in the separate `app` *binary* crate. Shared by
    /// `engine_helpers`'s restore-dispatch test and `turn`'s resume tests.
    pub(crate) fn read_char_then_save_v4_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 4; // version 4 (0OP save/restore store form lives here)
        buf[0x04] = 0x04; buf[0x05] = 0x00; // high_mem_base = 0x0400
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial_pc = 0x0040
        buf[0x08] = 0x00; buf[0x09] = 0x80; // dictionary = 0x0080 (empty)
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf[0x0A] = 0x01; buf[0x0B] = 0x00; // object_table = 0x0100
        buf[0x0C] = 0x03; buf[0x0D] = 0x00; // global_vars = 0x0300
        buf[0x0E] = 0x04; buf[0x0F] = 0x00; // static_mem_base = 0x0400
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev_table = 0x0060
        buf[0x0040] = 0xF6; // VAR read_char
        buf[0x0041] = 0x7F; // type: small(01), omit(11), omit(11), omit(11)
        buf[0x0042] = 1;    // operand: device=1
        buf[0x0043] = 0x10; // store -> G0
        buf[0x0044] = 0xB5; // 0OP:0x05 save (store form)
        buf[0x0045] = 0x10; // store -> G0
        buf[0x0046] = 0xBA; // quit
        buf
    }

    #[test]
    fn game_echoes_command_detects_self_echo() {
        use super::game_echoes_command;
        // CounterfeitMonkey shape: the turn output starts with the command (bold),
        // then the response — case-insensitive, boundary-terminated.
        assert!(game_echoes_command("yes\n\nGood, you're conscious.", "yes"));
        assert!(game_echoes_command("YES\n\n...", "yes"), "case-insensitive");
        assert!(game_echoes_command("examine me\n\nYou see nothing special.", "examine me"));
        assert!(game_echoes_command("  look\nA room.", "look"), "leading whitespace ok");
        // Most games: the response does not start with the command → keep our echo.
        assert!(!game_echoes_command("You can't go that way.\n>", "north"));
        assert!(!game_echoes_command("", "look"), "empty output");
        assert!(!game_echoes_command("anything", ""), "empty command never matches");
        // Boundary: a command must not match a longer word it is a prefix of.
        assert!(!game_echoes_command("gospel music plays.", "go"));
    }

    #[test]
    fn resolve_pict_blorb_finds_sidecar_for_bare_ulx() {
        // Regression test for SQ-0173: restart's Pict-blorb resolution must find
        // a same-stem sidecar .blorb for a bare .ulx the same path-based way as
        // launch (blorb::resolve_resource_blorb), not the old bytes-only
        // blorb::Blorb::parse(story_bytes), which only ever finds images inside
        // a self-contained .gblorb.
        fn png_bytes() -> Vec<u8> {
            let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgb8(img)
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
                .unwrap();
            bytes
        }

        // Build an IFF chunk: type + BE len + data + pad-to-even.
        fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }

        // Build a minimal FORM/IFRS blorb with only a Pict (PNG) resource — no
        // sound. resolve_resource_blorb accepts a resource sidecar that carries
        // pictures OR sounds (SQ-0372), so a graphics-only sidecar like Beyond
        // Zork's `beyondzork.blb` resolves without needing a dummy Snd entry.
        fn build_sidecar_blorb(png: &[u8]) -> Vec<u8> {
            #[allow(clippy::type_complexity)]
            let res: [(&[u8; 4], u32, &[u8; 4], &[u8]); 1] =
                [(b"Pict", 0, b"PNG ", png)];
            let ridx_data_len = 4 + 12 * res.len();
            let first_res_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
            let mut offsets = Vec::new();
            let mut cursor = first_res_off;
            let mut body = Vec::new();
            for (_u, _n, ty, data) in res.iter() {
                offsets.push(cursor as u32);
                let c = chunk(ty, data);
                cursor += c.len();
                body.extend_from_slice(&c);
            }
            let mut ridx = Vec::new();
            ridx.extend_from_slice(&(res.len() as u32).to_be_bytes());
            for (i, (usage, number, _ty, _d)) in res.iter().enumerate() {
                ridx.extend_from_slice(*usage);
                ridx.extend_from_slice(&number.to_be_bytes());
                ridx.extend_from_slice(&offsets[i].to_be_bytes());
            }
            let ridx_chunk = chunk(b"RIdx", &ridx);
            let mut inner = Vec::new();
            inner.extend_from_slice(b"IFRS");
            inner.extend_from_slice(&ridx_chunk);
            inner.extend_from_slice(&body);
            let mut file = Vec::new();
            file.extend_from_slice(b"FORM");
            file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
            file.extend_from_slice(&inner);
            file
        }

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../gvm-cli/tests/fixtures/glulxercise.ulx");
        let Ok(ulx_bytes) = std::fs::read(&fixture) else { return };

        let dir = std::env::temp_dir().join(format!("bm-pictblorb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let ulx_path = dir.join("game.ulx");
        std::fs::write(&ulx_path, &ulx_bytes).expect("write game.ulx");
        let blorb_path = dir.join("game.blorb");
        std::fs::write(&blorb_path, build_sidecar_blorb(&png_bytes())).expect("write sidecar");

        assert!(
            super::resolve_pict_blorb(&ulx_path, true).is_some(),
            "sidecar .blorb next to a bare .ulx must resolve (regression: the old \
             bytes-only logic returned None for a non-self-contained story)"
        );
        assert!(
            super::resolve_pict_blorb(&ulx_path, false).is_none(),
            "images disabled must resolve to None regardless of sidecar"
        );

        let no_sidecar_dir =
            std::env::temp_dir().join(format!("bm-pictblorb-nosc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&no_sidecar_dir);
        std::fs::create_dir_all(&no_sidecar_dir).expect("create temp dir");
        let lone_ulx = no_sidecar_dir.join("lone.ulx");
        std::fs::write(&lone_ulx, &ulx_bytes).expect("write lone.ulx");
        assert!(
            super::resolve_pict_blorb(&lone_ulx, true).is_none(),
            "no sidecar present must resolve to None"
        );

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&no_sidecar_dir);
    }

    // ── TestBackend: map pane shows a single-line border by default ───────────

    /// SQ-0357: the map pane's default is a plain single-line border. It used to be an ornate
    /// picture-frame — a frame within a frame, which cost two columns and two rows of map to
    /// draw a second box around the first one.
    #[test]
    fn map_pane_default_is_a_single_line_border() {
        // Resolve the default look from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf, area, cs.map_border_style, &PaneGlyphs::default(), cs.theme.get("panel.border").style);
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "┌", "default map border is single-line");
        assert_eq!(buf.cell((0, 3)).unwrap().symbol(), "│");
        // Content is everything inside that one border — two more rows and columns of map than
        // the picture-frame left (which nested a second frame inside the first).
        assert_eq!(frame.content, Rect::new(1, 1, 18, 8));
    }

    // ── TestBackend: story pane shows adventure title in its border ───────────────

    /// Verify that the DEFAULT_STYLE_TOML-resolved ColorScheme configures
    /// story_border_style as single, that rendering it produces the ┌ outer
    /// corner at top-left, and that the adventure title appears in the top border row.
    #[test]
    fn story_pane_shows_title_in_border_by_default() {
        // Resolve the default look from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 40, 15);
        let mut buf = Buffer::empty(area);

        // Draw the story pane frame (same as draw_frame does).
        let frame = draw_pane_frame(&mut buf, area, cs.story_border_style, &PaneGlyphs::default(), cs.theme.get("panel.border").style);

        // Overlay the adventure title (single centered segment, not active).
        draw_top_inset(
            &mut buf,
            frame.top_inset,
            &[InsetSegment { text: "ZORK I", active: false }],
            cs.theme.get("story_title").style,
            cs.theme.get("story_title").style,
        );

        // DEFAULT_STYLE_TOML sets story_border to single; top-left outer corner must be ┌
        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            "┌",
            "default story border must be single (┌ at top-left)"
        );

        // The title "ZORK I" must appear somewhere in the top border row (row 0 for single).
        let title_row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).unwrap().symbol().to_string())
            .collect();
        assert!(
            title_row.contains("ZORK I"),
            "top border row must contain the adventure title 'ZORK I'; got: {:?}",
            title_row
        );
    }

    // ── Focus-aware panel border colour (SQ-0309 §2a) ──────────────────────────

    /// The pane with input focus renders its border BOLD via the theme's
    /// `panel.border:active`; the unfocused pane stays plain `panel.border`.
    /// Previously bold only appeared transiently during split-resize
    /// (`focused_border`) — this is now a persistent focus indicator.
    #[test]
    fn focused_pane_border_is_bold() {
        // Resolve the default theme from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 20, 10);

        // Story focused: story border bold, map border (unfocused) not bold.
        let mut story_buf = Buffer::empty(area);
        let story_style = super::panel_border(&cs.theme, true);
        draw_pane_frame(&mut story_buf, area, cs.story_border_style, &PaneGlyphs::default(), story_style);
        assert!(
            story_buf.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "focused story border must be bold (panel.border:active)"
        );

        let mut map_buf = Buffer::empty(area);
        let map_style = super::panel_border(&cs.theme, false);
        draw_pane_frame(&mut map_buf, area, cs.map_border_style, &PaneGlyphs::default(), map_style);
        assert!(
            !map_buf.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "unfocused map border must not be bold (panel.border)"
        );

        // Flip focus to the map: story loses bold, map gains it.
        let mut story_buf2 = Buffer::empty(area);
        let story_style2 = super::panel_border(&cs.theme, false);
        draw_pane_frame(&mut story_buf2, area, cs.story_border_style, &PaneGlyphs::default(), story_style2);
        assert!(
            !story_buf2.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "unfocused story border must not be bold (panel.border)"
        );

        let mut map_buf2 = Buffer::empty(area);
        let map_style2 = super::panel_border(&cs.theme, true);
        draw_pane_frame(&mut map_buf2, area, cs.map_border_style, &PaneGlyphs::default(), map_style2);
        assert!(
            map_buf2.cell((0, 0)).unwrap().modifier.contains(Modifier::BOLD),
            "focused map border must be bold (panel.border:active)"
        );
    }

    // ── Hotkey dialog tests ───────────────────────────────────────────────────

    #[test]
    fn prefix_key_opens_hotkey_dialog() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use app::input::{apply_action, key_to_action, Action};
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        // Default prefix is Ctrl+K
        let ctrlk = KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let action = key_to_action(&s, ctrlk);
        assert!(
            matches!(action, Action::OpenHotkeyDialog),
            "Ctrl+K should produce OpenHotkeyDialog"
        );
        apply_action(action, &mut s, &mut Mapper::default());
        assert!(s.overlays.hotkey_dialog, "hotkey_dialog should be true after OpenHotkeyDialog");
    }

    #[test]
    fn prefix_key_closes_hotkey_dialog() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use app::input::{apply_action, key_to_action, Action};
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let ctrlk = KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let action = key_to_action(&s, ctrlk);
        assert!(
            matches!(action, Action::CloseHotkeyDialog),
            "Ctrl+K when dialog open should produce CloseHotkeyDialog"
        );
        apply_action(action, &mut s, &mut Mapper::default());
        assert!(!s.overlays.hotkey_dialog, "hotkey_dialog should be false after CloseHotkeyDialog");
    }

    // ── dim_area ──────────────────────────────────────────────────────────────

    #[test]
    fn dim_area_sets_dim_on_all_cells() {
        let area = Rect::new(0, 0, 4, 3);
        let mut buf = Buffer::empty(area);
        // Pre-fill one cell with some content so we can check DIM ORs onto existing modifier.
        buf.cell_mut((1, 1)).unwrap().set_symbol("X");

        dim_area(&mut buf, area);

        for y in 0..3 {
            for x in 0..4 {
                let cell = buf.cell((x, y)).unwrap();
                assert!(
                    cell.modifier.contains(Modifier::DIM),
                    "cell ({x},{y}) should have DIM; modifier={:?}",
                    cell.modifier
                );
            }
        }
    }

    #[test]
    fn dim_area_does_not_affect_cells_outside_area() {
        let full = Rect::new(0, 0, 6, 4);
        let target = Rect::new(2, 1, 3, 2); // x:2..5, y:1..3
        let mut buf = Buffer::empty(full);

        dim_area(&mut buf, target);

        // Cells inside target have DIM.
        for y in 1..3 {
            for x in 2..5 {
                assert!(
                    buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "cell ({x},{y}) inside target should have DIM"
                );
            }
        }
        // Cells outside target do NOT have DIM.
        assert!(
            !buf.cell((0, 0)).unwrap().modifier.contains(Modifier::DIM),
            "cell (0,0) outside target should NOT have DIM"
        );
        assert!(
            !buf.cell((5, 3)).unwrap().modifier.contains(Modifier::DIM),
            "cell (5,3) outside target should NOT have DIM"
        );
    }

    // ── Split layout: dim unfocused, leave focused undimmed ───────────────────

    /// This test exercises the split-layout dimming logic by simulating what
    /// draw_frame does: render content into two inner rects, then call dim_area
    /// on the unfocused one. It verifies that cells in the unfocused inner rect
    /// have DIM and cells in the focused inner rect do NOT.
    ///
    /// New behavior (item 6): map pane is NEVER dimmed regardless of focus.
    /// Story pane dims only when map has focus.
    #[test]
    fn split_layout_unfocused_pane_is_dimmed_focused_is_not() {
        let full = Rect::new(0, 0, 20, 5);
        let left_inner = Rect::new(1, 1, 8, 3);   // story (transcript) inner area

        // Simulate Focus::Map: story pane dims, map pane stays bright.
        {
            let mut buf = Buffer::empty(full);
            dim_area(&mut buf, left_inner);

            // Story pane (left) inner cells should have DIM when map has focus.
            for y in 1..4 {
                for x in 1..9 {
                    assert!(
                        buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                        "story pane cell ({x},{y}) should have DIM when focus=Map"
                    );
                }
            }
            // Map pane (right) inner cells should NOT have DIM.
            for y in 1..4 {
                for x in 11..19 {
                    assert!(
                        !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                        "map pane cell ({x},{y}) should NOT have DIM when focus=Map"
                    );
                }
            }
        }

        // Simulate Focus::Game: neither pane is dimmed (map pane always stays bright).
        {
            let buf = Buffer::empty(full);
            // Focus::Game => no dim_area call at all (map is never dimmed)

            // Neither pane has DIM.
            for y in 1..4 {
                for x in 1..19 {
                    assert!(
                        !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                        "cell ({x},{y}) should NOT have DIM when focus=Game"
                    );
                }
            }
        }
    }

    /// Verify: map pane is never dimmed regardless of focus setting.
    #[test]
    fn map_pane_never_dimmed() {
        let full = Rect::new(0, 0, 20, 5);

        // Focus::Game: map pane should NOT be dimmed (we do NOT call dim_area on it).
        let buf = Buffer::empty(full);
        // The new code: "if state.focus == Focus::Map { dim_area(transcript_inner); }"
        // So for Focus::Game, we dim nothing. Map stays bright.
        for y in 1..4 {
            for x in 11..19 {
                assert!(
                    !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "map pane cell ({x},{y}) should NOT have DIM under Focus::Game"
                );
            }
        }

        // Focus::Map: only transcript is dimmed, map stays bright.
        let mut buf2 = Buffer::empty(full);
        let left_inner = Rect::new(1, 1, 8, 3);
        dim_area(&mut buf2, left_inner); // transcript dimmed
        // Map pane not touched
        for y in 1..4 {
            for x in 11..19 {
                assert!(
                    !buf2.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "map pane cell ({x},{y}) should NOT have DIM under Focus::Map either"
                );
            }
        }
    }

    // ── Fix 4: pulse overlay only touches outer perimeter ─────────────────────

    /// The pulse overlay (applied during a tidy job) writes the pulse color to the
    /// outer perimeter cells of the map pane area. The interior content cells (rows
    /// y+2.. , cols x+2..) must NOT be overwritten by the pulse, so the map body and
    /// its overlays keep their own styling.
    ///
    /// This test directly exercises the perimeter-loop invariant: identical to what
    /// draw_frame executes, extracted inline so it runs without a full render stack.
    #[test]
    fn pulse_overlay_touches_only_outer_perimeter_not_inner_tab_row() {
        use ratatui::style::{Color, Style};

        // Use a 30x15 area.
        let area = Rect::new(0, 0, 30, 15);
        let mut buf = Buffer::empty(area);

        // The pulse color to apply (distinct from default Reset).
        let pulse_color = Color::Rgb(60, 200, 90); // PULSE_GREEN
        let pulse_style = Style::default().fg(pulse_color);

        // Apply the pulse overlay exactly as draw_frame does.
        for cy in area.y..area.bottom() {
            if let Some(c) = buf.cell_mut((area.x, cy)) { c.set_style(pulse_style); }
            if let Some(c) = buf.cell_mut((area.right().saturating_sub(1), cy)) { c.set_style(pulse_style); }
        }
        for cx in area.x..area.right() {
            if let Some(c) = buf.cell_mut((cx, area.y)) { c.set_style(pulse_style); }
            if let Some(c) = buf.cell_mut((cx, area.bottom().saturating_sub(1))) { c.set_style(pulse_style); }
        }

        // Outer perimeter (top row y=0) must carry the pulse color.
        let top_left_fg = buf.cell((area.x, area.y)).map(|c| c.fg).unwrap();
        assert_eq!(
            top_left_fg,
            pulse_color,
            "top-left outer perimeter cell must carry pulse color"
        );
        let top_right_fg = buf.cell((area.right() - 1, area.y)).map(|c| c.fg).unwrap();
        assert_eq!(
            top_right_fg,
            pulse_color,
            "top-right outer perimeter cell must carry pulse color"
        );

        // Interior content cells (row y+2, cols x+2..right-2) must NOT carry the
        // pulse color: the pulse only writes the outer perimeter (cols x / right-1,
        // rows y / bottom-1), so the map body is untouched.
        let content_row_y = area.y + 2;
        for cx in (area.x + 2)..(area.right() - 2) {
            let fg = buf.cell((cx, content_row_y)).map(|c| c.fg).unwrap();
            assert_ne!(
                fg,
                pulse_color,
                "interior content cell ({cx}, {content_row_y}) must NOT be overwritten by pulse"
            );
        }
    }

    // ── scroll_for_match ──────────────────────────────────────────────────────

    #[test]
    fn scroll_for_match_brings_row_into_view() {
        // match at position 0 in 100 visible rows, pane is 10 rows tall.
        // scroll = 100 - 0 - 10 = 90  (places match at the top of the viewport).
        // Windowing check: end = 100 - 90 = 10, start = 0, match row 0 is in [0..10). OK.
        assert_eq!(scroll_for_match(0, 100, 10), 90);

        // match at position 99 (the very last row): scroll = 100 - 99 - 10 = -9 -> clamped to 0.
        // Windowing check: end = 100, start = 90, match row 99 is in [90..100). OK.
        assert_eq!(scroll_for_match(99, 100, 10), 0);

        // match in the middle: position 50, total 100, pane 10.
        // scroll = 100 - 50 - 10 = 40.
        // end = 100 - 40 = 60, start = 50. Match row 50 is at the top of [50..60). OK.
        assert_eq!(scroll_for_match(50, 100, 10), 40);

        // pane larger than transcript: match at 0, total 5, pane 10.
        // scroll = 5 - 0 - 10 = saturates to 0.
        assert_eq!(scroll_for_match(0, 5, 10), 0);
    }

    // ── is_slash ──────────────────────────────────────────────────────────────

    #[test]
    fn is_slash_uses_prefix() {
        assert!(is_slash("/save", '/'));
        assert!(!is_slash("look", '/'));
        assert!(is_slash(";help", ';'));
        assert!(!is_slash("/help", ';'));
    }

    // ── should_prompt_save_on_quit ────────────────────────────────────────────

    #[test]
    fn prompt_save_on_quit_all_conditions_required() {
        use app::state::AppState;

        let mut s = AppState::default();
        // Default: auto_save = false, prompt_save_on_quit = true, unsaved_progress = false
        // No prompt with no unsaved progress (fresh, or just saved/loaded).
        assert!(!should_prompt_save_on_quit(&s), "no unsaved progress => no prompt");

        s.unsaved_progress = true;
        // Now: auto_save=false, prompt_save_on_quit=true, unsaved_progress=true => prompt
        assert!(should_prompt_save_on_quit(&s), "unsaved progress => prompt");

        // Saving (or loading) clears the flag => no prompt right after a save.
        s.unsaved_progress = false;
        assert!(!should_prompt_save_on_quit(&s), "after a save/load => no prompt");

        s.unsaved_progress = true;
        s.config.auto_save = true;
        // auto_save=true => no prompt (game already saves automatically)
        assert!(!should_prompt_save_on_quit(&s), "auto_save=true => no prompt");

        s.config.auto_save = false;
        s.config.prompt_save_on_quit = false;
        // prompt_save_on_quit=false => no prompt (user opted out)
        assert!(!should_prompt_save_on_quit(&s), "prompt_save_on_quit=false => no prompt");
    }

    // ── launch_dialog counts as overlay ──────────────────────────────────────

    #[test]
    fn launch_dialog_counts_as_overlay() {
        let mut s = app::state::AppState::default();
        assert!(!s.any_overlay_open(), "default state has no overlay");
        s.overlays.launch_dialog = true;
        assert!(s.any_overlay_open(), "launch_dialog true => any_overlay_open true");
        s.overlays.launch_dialog = false;
        assert!(!s.any_overlay_open(), "launch_dialog false => any_overlay_open false");
    }

    // The former app-level `key_to_zscii` and its unit tests were relocated into
    // the zvm engine adapter as `GameSession::key_input_to_zscii` (tested in
    // session.rs); the neutral crossterm→KeyInput mapping is tested in engine.rs.

    #[test]
    fn saves_dir_is_user_dir_join_saves() {
        // Save archives live under user_dir/saves.
        let d = super::saves_dir(std::path::Path::new("/tmp/bm"));
        assert_eq!(d, std::path::Path::new("/tmp/bm/saves"));
    }

    // ── char-mode gate predicate test ─────────────────────────────────────────

    /// The gate fires iff: char_mode && !any_overlay_open && key != prefix &&
    /// no Ctrl/Alt modifier. Test with a default AppState (no overlays, no
    /// char_mode initially).
    #[test]
    fn char_mode_forwards_arrow_keys_to_the_story_not_the_caret() {
        // SQ-0354's safety property, and the reason caret editing cannot steal story-controlled
        // input: when the story asks for a single keypress, the run loop's char-mode gate forwards
        // the key straight to the VM and `continue`s — app routing (and therefore the caret keys)
        // never sees it. Assert the two halves the gate depends on.
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for code in [KeyCode::Left, KeyCode::Right, KeyCode::Home, KeyCode::End, KeyCode::Delete] {
            let k = KeyEvent::new(code, KeyModifiers::NONE);
            assert!(
                app::engine::key_event_to_input(k).is_some(),
                "{code:?} must be deliverable to the story as input",
            );
            // Plain keys are game input; only Ctrl/Alt combos are held back for app routing.
            assert!(
                !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT),
                "{code:?} is a plain key, so the gate forwards it",
            );
        }
    }

    #[test]
    fn char_mode_gate_predicate() {
        use app::state::AppState;
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

        // The forward-to-VM predicate mirrors the run-loop gate.
        let app_combo = |m: KeyModifiers| m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        let mut s = AppState::default();
        // char_mode false → gate should not fire.
        assert!(!s.char_mode, "default state is not char_mode");
        assert!(!s.any_overlay_open(), "default state has no overlay");

        // Simulate char_mode = true (as the run loop sets it from pending_input).
        s.char_mode = true;

        // A plain 'y' key: gate should accept it (not prefix, not overlay, no combo).
        let y_key = KeyEvent {
            code: KeyCode::Char('y'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let spec = app::keymap::KeySpec::from_key_event(y_key);
        let is_prefix = spec == s.hotkeys.prefix;
        assert!(!is_prefix, "'y' must not be the default prefix (Ctrl+K)");
        assert!(s.char_mode && !s.any_overlay_open() && !is_prefix && !app_combo(y_key.modifiers),
            "char_mode gate should fire for 'y' with no overlays");
        // 'y' maps to a neutral KeyInput the engine then converts to input.
        assert_eq!(app::engine::key_event_to_input(y_key), Some(app::engine::KeyInput::Char('y')));

        // Ctrl+Q (a quit binding) must NOT be forwarded to the VM — it falls
        // through to app routing so the user can escape the form.
        let ctrlq = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let spec_q = app::keymap::KeySpec::from_key_event(ctrlq);
        let is_prefix_q = spec_q == s.hotkeys.prefix;
        assert!(!(s.char_mode && !s.any_overlay_open() && !is_prefix_q && !app_combo(ctrlq.modifiers)),
            "char_mode gate must NOT fire for Ctrl+Q (a Ctrl combo)");

        // Ctrl+K (the default prefix): gate must NOT fire for it (falls through
        // to normal routing so the hotkey dialog still opens).
        let ctrlk = KeyEvent {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        let spec_k = app::keymap::KeySpec::from_key_event(ctrlk);
        let is_prefix_k = spec_k == s.hotkeys.prefix;
        assert!(is_prefix_k, "Ctrl+K must match the default prefix");
        // Gate condition false because is_prefix = true (and it is a Ctrl combo).
        assert!(!(s.char_mode && !s.any_overlay_open() && !is_prefix_k && !app_combo(ctrlk.modifiers)),
            "char_mode gate must NOT fire for the prefix key Ctrl+K");

        // If an overlay is open, the gate must not fire.
        s.overlays.hotkey_dialog = true;
        assert!(s.any_overlay_open(), "hotkey_dialog open => overlay open");
        assert!(!s.char_mode || s.any_overlay_open(),
            "char_mode gate must not fire when overlay is open");
    }

    #[test]
    fn loading_line_reports_name_size_and_frame() {
        let line = super::loading_line("CounterfeitMonkey-11.gblorb", 11_855_360, '/');
        assert!(line.contains("CounterfeitMonkey-11.gblorb"), "names the story");
        assert!(line.contains("11.3 MB"), "shows size in MB, got: {line}");
        assert!(line.ends_with('/'), "ends with the spinner frame glyph");
    }

}
