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
use app::storage::default_state_path;
use app::input::{apply_action, apply_tidy_result, key_to_command, mouse_to_action, should_bg_tidy, style_dialog_action, tidy_layer_silent, Action, ApplyTidyOutcome, KeyResolve};
use app::persist_files::{delete_save, list_saves, load_map, save_game_named, restore_game, save_named};
use app::render::config_screen::draw_config_screen;
use app::render::style_editor::{draw_style_editor, StyleEditorRects};
use app::render::dialog::{DialogRects, DialogStyle};
use app::render::filebrowser::draw_file_browser;
use app::render::gallery::draw_gallery;
use app::render::hints_panel::{draw_hints_panel, HintsPanelRects};
use app::render::launch_dialog::draw_launch_dialog;
use app::render::quit_dialog::draw_quit_dialog;
use app::render::aux_dialog::draw_aux_dialog;
use app::render::reset_dialog::draw_reset_dialog;
use app::render::hotkeys::draw_hotkey_dialog;
use app::render::verbmenu::draw_verb_menu;
use app::render::inspector::{draw_inspector, room_diagnostics};
use app::render::map::{pulse_border_color, render_map_layered, room_screen_rects, sound_pulse_color, SOUND_PULSE_MS};
use app::render::paneframe::{build_layer_segments, draw_framed, draw_header_plain, draw_top_inset, InsetSegment};
use app::render::tidy_panel::draw_tidy_panel;
use mapper::graph::RoomId;
use mapper::layer::LayerId;
use app::render::room_info::draw_room_info;
use app::render::saves::draw_saves;
use app::render::file_picker::draw_file_picker;
use app::render::history::draw_history;
use app::render::screen::render_story_pane;
use app::render::draw_str_clipped;
use app::engine::Engine;
use app::glulx_session::GlulxSession;
use app::session::{apply_turn, GameSession, TurnResult};
use app::export::export_transcript;
use app::hints;
use app::slash::{self, SlashOutcome, TranscriptFilterArg};
use app::state::{AppState, FbMode, FileBrowserState, Focus, Layout, PromptKind, RoomPanelMode, SavesState, SoundPulse, TidyJob, TranscriptFilter, TranscriptKind};

mod picker_ui;
mod startup;

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

// ── Hint bar ─────────────────────────────────────────────────────────────────

use app::keymap::{Context, HotkeyLayout, KeyMap};

/// Priority-ordered command-string lists for the bottom hint bar.
/// Commands are included only when directly available in the current context.
/// `tidy-map` is intentionally excluded from all lists.
const GAME_HINTS: &[&str] = &[
    "toggle-focus",
    "save-state",
    "restore-state",
];

const MAP_HINTS: &[&str] = &[
    "toggle-focus",
    "pan-map -1 0",
    "zoom-map in",
    "select-room next",
    "center-map",
];

const ANIM_HINTS: &[&str] = &[
    "anim-step forward",
    "anim-play",
    "anim-exit",
    "pan-map -1 0",
    "zoom-map in",
];

/// Short hint-bar label for a command-string: "zoom-map in" -> "zoom map in".
fn hint_label(cmd_str: &str) -> String {
    cmd_str.replace('-', " ")
}

/// Build the hint bar string for the given context from the live keymap and layout.
///
/// For each command-string in `priority`, an entry is included only if all three hold:
/// 1. `layout.is_direct_name(cmd)` — the command is directly available, not dialog-only.
/// 2. `keymap.primary_key(name)` returns a KeySpec `k`.
/// 3. `keymap.lookup(&k, ctx) == Some(cmd)` — pressing `k` in `ctx` resolves to `cmd`.
///
/// Each surviving entry renders as "{k.label()}: {label}"; entries join with " | ".
/// If the joined string exceeds `width` characters, it is truncated and "…" appended.
pub fn hint_bar(
    keymap: &KeyMap,
    layout: &HotkeyLayout,
    ctx: Context,
    priority: &[&str],
    width: usize,
) -> String {
    let entries: Vec<String> = priority
        .iter()
        .filter_map(|&cmd| {
            // Gate 1: command must be directly available (not dialog-only).
            if !layout.is_direct_name(cmd) {
                return None;
            }
            // Gate 2: command must have a primary key binding.
            let name = cmd.split_whitespace().next().unwrap_or("");
            let k = keymap.primary_key(name)?;
            // Gate 3: pressing that key in this context must resolve back to this command.
            if keymap.lookup(&k, ctx) != Some(cmd) {
                return None;
            }
            let label = hint_label(cmd);
            Some(format!("{}: {}", k.label(), label))
        })
        .collect();

    let joined = entries.join(" | ");

    // Truncate to width (char-count aware), appending "…" if needed.
    if width == 0 {
        return String::new();
    }
    let char_count = joined.chars().count();
    if char_count <= width {
        joined
    } else {
        // Find the byte offset after (width - 1) chars to leave room for "…".
        let truncate_at = width.saturating_sub(1);
        let byte_pos = joined
            .char_indices()
            .nth(truncate_at)
            .map(|(i, _)| i)
            .unwrap_or(joined.len());
        format!("{}…", &joined[..byte_pos])
    }
}

// ── Draw helper ───────────────────────────────────────────────────────────────

/// Both pane inner-content rects returned by `draw_frame`.
/// `map` is `Rect::default()` when the layout hides the map (TranscriptFull).
/// `story` is `Rect::default()` when the layout hides the story (MapFull).
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
    /// Hit-rects for the save-name dialog (when open).
    pub save_name_dialog: Option<app::render::save_name_dialog::SaveNameDialogRects>,
    /// Hit-rects for the quit dialog (when open).
    pub quit_dialog: Option<app::render::quit_dialog::QuitDialogRects>,
    /// Hit-rects for the launch dialog (when open).
    pub launch_dialog: Option<app::render::launch_dialog::LaunchDialogRects>,
    /// Hit-rects for the hints panel (when open).
    pub hints_panel: Option<HintsPanelRects>,
    /// Hit-rects for the style-editor board (when open).
    pub style_editor: Option<StyleEditorRects>,
    /// Hit-rects for the verb dock's token rows and section headers (when open).
    pub verb_menu: app::render::verbmenu::VerbMenuHits,
    /// Hit-rects for the glyph-picker modal (when open).
    pub glyph_picker: Option<app::render::glyph_picker::GlyphPickerRects>,
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
    /// to size a PageUp/PageDown step. 0 when no transcript is shown (MapFull).
    pub transcript_viewport_rows: u16,
    /// List-row viewport of the open selection-list modal this frame, synced to
    /// `AppState.modal_list_viewport` so nav actions can window/animate. 0 when
    /// no list modal is open.
    pub modal_list_viewport: usize,
}

/// Escape hatch: borrow the concrete Z-machine `GameSession` behind a
/// `dyn Engine`, mutably.
///
/// Used ONLY by the persistence layer — archive save/restore and the
/// saved-screen snapshot — because the on-disk archive format serializes the
/// Z-machine `ScreenState` and cannot change without breaking compatibility
/// with existing saves (a no-behavior-change requirement). Everything else
/// (gameplay, render, input, introspection, `save_state`/`restore_state`,
/// `current_location`, aux) goes through the neutral `Engine` trait.
fn zvm_session_mut(engine: &mut dyn Engine) -> &mut GameSession {
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
fn zvm_session_opt(engine: &dyn Engine) -> Option<&GameSession> {
    engine.as_any().downcast_ref::<GameSession>()
}

/// Mutable non-panicking downcast to the Z-machine session: `Some` for a Z-code
/// game, `None` for Glulx. Used to reinstate the zvm-only `screen.json` after an
/// archive restore without panicking on a Glulx engine.
fn zvm_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GameSession> {
    engine.as_any_mut().downcast_mut::<GameSession>()
}

/// Non-panicking downcast to the Glulx session: `Some` for a Glulx game, `None`
/// for Z-code. Used to read the armed Glk timer interval.
fn glulx_session_opt(engine: &dyn Engine) -> Option<&GlulxSession> {
    engine.as_any().downcast_ref::<GlulxSession>()
}

/// Mutable non-panicking downcast to the Glulx session: `Some` for a Glulx
/// game, `None` for Z-code. Used to deliver Glk sound-notify events.
fn glulx_session_opt_mut(engine: &mut dyn Engine) -> Option<&mut GlulxSession> {
    engine.as_any_mut().downcast_mut::<GlulxSession>()
}

/// The engine tag (`"zmachine"` / `"glulx"`) of the running engine, for wrapping
/// raw same-engine save bytes (e.g. a rewind/replay snapshot) into an
/// [`app::engine::EngineSave`] before `restore_state`.
fn engine_tag(engine: &dyn Engine) -> &'static str {
    if engine.as_any().is::<GlulxSession>() {
        app::glulx_session::GLULX_ENGINE
    } else {
        app::session::ZMACHINE_ENGINE
    }
}

/// Convert an [`app::engine::EngineError`] from `restore_state` into a graceful
/// player-facing message (no panic): a foreign-engine save names both engines, a
/// bad Z-machine save keeps the historical "different story" wording.
fn restore_error_msg(e: app::engine::EngineError) -> String {
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
enum RestoreOutcome {
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
fn restore_from_file(path: &std::path::Path, session: &mut dyn Engine) -> Result<RestoreOutcome, String> {
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
fn engine_supports_save(engine: &dyn Engine) -> bool {
    engine.as_any().downcast_ref::<GameSession>().is_some()
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
    let mut aux_dialog_rects_out: Option<app::render::aux_dialog::AuxDialogRects> = None;
    let mut reset_dialog_rects_out: Option<app::render::reset_dialog::ResetDialogRects> = None;
    let mut save_name_dialog_rects_out: Option<app::render::save_name_dialog::SaveNameDialogRects> = None;
    let mut quit_dialog_rects_out: Option<app::render::quit_dialog::QuitDialogRects> = None;
    let mut launch_dialog_rects_out: Option<app::render::launch_dialog::LaunchDialogRects> = None;
    let mut hints_panel_rects_out: Option<HintsPanelRects> = None;
    let mut style_editor_rects_out: Option<StyleEditorRects> = None;
    let mut glyph_picker_rects_out: Option<app::render::glyph_picker::GlyphPickerRects> = None;
    let mut verb_hits = app::render::verbmenu::VerbMenuHits::default();
    let mut modal_list_viewport: usize = 0;
    let mut transcript_max_scroll: u16 = 0;
    let mut transcript_viewport_rows: u16 = 0;
    let mut transcript_links_out: Vec<((u16, u16), u32)> = Vec::new();

    terminal.draw(|f| {
        let full = f.area();
        let buf = f.buffer_mut();
        // The engine-neutral screen model for this frame (status + window tree).
        let screen_model = engine.screen();
        // During replay the map shows the reconstructed snapshot for the selected turn.
        let replay_graph: Option<mapper::graph::MapGraph> = state.replay.as_ref().map(|r| {
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
        let rm = if let Some(g) = &replay_graph {
            FrameRenderMap::Owned(render_layer(g, state.active_layer(g)))
        } else {
            match &state.tidy_anim {
                Some(anim) => {
                    let g = &anim.current().graph;
                    FrameRenderMap::Owned(render_layer(g, state.active_layer(g)))
                }
                None => {
                    let layer = state.active_layer(&mapper.graph);
                    FrameRenderMap::Cached(state.cached_map_render(layer, || render_layer(&mapper.graph, layer)))
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

        // When a background tidy job is in flight, the map pane border pulses between
        // red and green. This overrides the normal border color (focused or unfocused).
        let map_border_override: Option<ratatui::style::Color> = state.tidy_job.as_ref().map(|job| {
            pulse_border_color(job.started.elapsed())
        });

        // Resolve the story-border color: a live sound pulse overrides the fg.
        let story_border_style = {
            let base = state.colors.story_border;
            match &state.sound_pulse {
                Some(p) => {
                    let beep_color = match p.kind {
                        app::state::BeepKind::High => state
                            .colors
                            .sound_beep_high
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        app::state::BeepKind::Low => state
                            .colors
                            .sound_beep_low
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

        match state.layout {
            Layout::TranscriptFull => {
                let story_fp = draw_framed(buf, pane_layout.story, state.colors.story_border_style, state.colors.story_border_sides, &state.colors.story_border_glyphs, story_border_style, state.colors.story_header_on);
                let c = story_fp.content;
                let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
                transcript_max_scroll = m.max_scroll;
                transcript_viewport_rows = m.viewport_rows;
                transcript_links_out = m.links;
                if let Some(hrect) = story_fp.header {
                    let segs = [InsetSegment { text: &state.title, active: false }];
                    if story_fp.header_bordered {
                        draw_top_inset(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    } else {
                        draw_header_plain(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    }
                }
                story_area = story_fp.content;
                map_area = Rect::default();
            }
            Layout::MapFull => {
                let graph = if let Some(g) = &replay_graph {
                    g
                } else {
                    match &state.tidy_anim {
                        Some(anim) => &anim.current().graph,
                        None => &mapper.graph,
                    }
                };
                let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                let active_layer = state.active_layer(graph);
                let map_fp = draw_framed(buf, pane_layout.map, state.colors.map_border_style, state.colors.map_border_sides, &state.colors.map_border_glyphs, state.colors.map_border, state.colors.map_header_on);
                render_map_layered(&rm, &mapper.graph, state, map_fp.content, buf);
                if let Some(anim) = &state.tidy_anim {
                    let tidy_ds = make_dialog_style(state);
                    if let Some(dr) = draw_tidy_panel(anim.current(), map_fp.content, buf, &tidy_ds) {
                        dialog_rects_out = Some(dr);
                    }
                }
                map_area = map_fp.content;
                story_area = Rect::default();
                // Overlay layer tabs
                let owned_segs = build_layer_segments(&layer_ids, active_layer,
                    |id| format!("{}({})", graph.layer_name(id), graph.rooms_in_layer(id).len()));
                let inset_segs: Vec<_> = owned_segs.iter().map(|s| s.as_inset()).collect();
                if let Some(hrect) = map_fp.header {
                    let tab_rects = if map_fp.header_bordered {
                        draw_top_inset(buf, hrect, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active)
                    } else {
                        draw_header_plain(buf, hrect, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active)
                    };
                    layer_tabs_out = layer_ids.into_iter().zip(tab_rects).collect();
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
            }
            Layout::Split => {
                // Split 50/50 horizontally with bordered blocks (no divider column).
                // In resize mode, the StoryMap target covers this whole split, so
                // both borders pick up the `focused_border` accent to show it's live.
                let resize_split_hl = state.resize_mode && state.resize_target == app::state::ResizeTarget::StoryMap;
                let story_border_color = if resize_split_hl { state.colors.focused_border } else { story_border_style };
                let map_border_color = if resize_split_hl { state.colors.focused_border } else { state.colors.map_border };
                let story_fp = draw_framed(buf, pane_layout.story, state.colors.story_border_style, state.colors.story_border_sides, &state.colors.story_border_glyphs, story_border_color, state.colors.story_header_on);
                let c = story_fp.content;
                let m = render_story_pane(&screen_model, state.char_mode, engine.introspect(), state, c, buf);
                transcript_max_scroll = m.max_scroll;
                transcript_viewport_rows = m.viewport_rows;
                transcript_links_out = m.links;
                if let Some(hrect) = story_fp.header {
                    let segs = [InsetSegment { text: &state.title, active: false }];
                    if story_fp.header_bordered {
                        draw_top_inset(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    } else {
                        draw_header_plain(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    }
                }
                story_area = story_fp.content;

                let map_fp = draw_framed(buf, pane_layout.map, state.colors.map_border_style, state.colors.map_border_sides, &state.colors.map_border_glyphs, map_border_color, state.colors.map_header_on);
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
                    let graph = if let Some(g) = &replay_graph {
                        g
                    } else {
                        match &state.tidy_anim {
                            Some(anim) => &anim.current().graph,
                            None => &mapper.graph,
                        }
                    };
                    let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                    let active_layer = state.active_layer(graph);
                    let owned_segs = build_layer_segments(&layer_ids, active_layer,
                    |id| format!("{}({})", graph.layer_name(id), graph.rooms_in_layer(id).len()));
                    let inset_segs: Vec<_> = owned_segs.iter().map(|s| s.as_inset()).collect();
                    if let Some(hrect) = map_fp.header {
                        let tab_rects = if map_fp.header_bordered {
                            draw_top_inset(buf, hrect, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active)
                        } else {
                            draw_header_plain(buf, hrect, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active)
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

                // Map pane is NEVER dimmed (always full brightness).
                // Story pane dims when map has focus.
                if state.focus == Focus::Map {
                    dim_area(buf, story_fp.content);
                }
            }
        }

        // Compute room screen rects for accurate mouse hit-testing.
        room_rects_out = if map_area.height > 0 {
            room_screen_rects(&rm, state, map_area)
        } else {
            Vec::new()
        };

        // ── Room panel overlay ────────────────────────────────────────────────
        if map_area.height > 0 {
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
        let help_style = state.colors.help_bar;
        let help_text = if state.config_screen.is_some() {
            "\u{2191}\u{2193} move  \u{2190}\u{2192}/Space change  s save  Esc cancel".to_string()
        } else if state.verb_menu.is_some() {
            "Verb Menu | Tab/\u{2190}\u{2192}: pane | \u{2191}\u{2193}: move | Enter/Space: pick | Esc: close".to_string()
        } else if state.file_browser.as_ref().map(|fb| fb.mode == FbMode::PickFile).unwrap_or(false) {
            "Import Save | \u{2191}\u{2193}: move | Enter: open/import | Esc: cancel".to_string()
        } else if state.saves.is_some() {
            "Saves | \u{2191}\u{2193}: select | Enter: load | s: save-as | d: delete | i: import | Esc: close".to_string()
        } else if state.gallery.is_some() {
            "Symbol Gallery | \u{2191}\u{2193}: preset | \u{2190}\u{2192}: category | Esc/Enter: close".to_string()
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
        } else if let Some(prompt) = &state.prompt {
            // Show prompt label with instructions when a prompt is active.
            let label = match &prompt.kind {
                PromptKind::RenameRoom(_) => "Rename",
                PromptKind::EditNotes(_) => "Notes",
                PromptKind::RelabelEdge(_, _) => "Direction",
                PromptKind::RenameLayer(_) => "Layer name",
                PromptKind::ConfirmDeleteSave(_) => "Delete? (y/n)",
                PromptKind::ConfigEditPath { .. } => "Config path",
                PromptKind::CreateFile => "Filename",
            };
            format!("{}: type text | Enter: apply | Esc: cancel", label)
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

        // Modal dialogs center within the graphics-free text region (story text +
        // map together), never over a Glulx graphics window — the terminal image
        // protocol would otherwise overpaint them (SQ-0203). No graphics → `full`.
        // Clamp to gvm's content bounding box so the graphics-rect walk matches the
        // clamped composite render (the snap-margin has no windows). (SQ-0303)
        let story_bbox = app::render::screen::content_bounds(&screen_model, story_area);
        let dialog_area = app::render::screen::dialog_bounds(&screen_model, story_bbox, full);

        // ── Hotkey dialog overlay — drawn over everything ─────────────────────
        if state.hotkey_dialog {
            dialog_rects_out = draw_hotkey_dialog(state, dialog_area, buf);
        }

        // ── Gallery overlay — drawn after hotkey dialog ───────────────────────
        if state.gallery.is_some() {
            if let Some(dr) = draw_gallery(state, dialog_area, buf) {
                dialog_rects_out = Some(dr);
            }
        }

        // ── Saves-manager overlay — drawn after gallery ───────────────────────
        if state.saves.is_some() {
            dialog_rects_out = draw_saves(state, dialog_area, buf, &mut modal_list_viewport);
        }

        // ── Replay/rewind overlay ─────────────────────────────────────────────
        if state.replay.is_some() {
            dialog_rects_out = draw_history(state, dialog_area, buf, &mut modal_list_viewport);
        }

        // ── File-browser overlay — drawn after saves ──────────────────────────
        if state.file_browser.is_some() {
            dialog_rects_out = draw_file_browser(state, dialog_area, buf, &mut modal_list_viewport);
        }

        // ── VFS file picker overlay (read-mode create_by_prompt) ──────────────
        if state.file_picker.is_some() {
            dialog_rects_out = draw_file_picker(state, dialog_area, buf, &mut modal_list_viewport);
        }

        // ── Config screen overlay — drawn after other modals ──────────────────
        if state.config_screen.is_some() {
            dialog_rects_out = draw_config_screen(state, dialog_area, buf, &mut modal_list_viewport);
        }

        // ── Style editor overlay — full-screen, drawn after config screen ──────
        if state.style_editor.is_some() {
            style_editor_rects_out = draw_style_editor(state, dialog_area, buf);
        }

        // ── Glyph-picker modal — drawn over the style editor ──────────────────
        if state.glyph_picker.is_some() {
            glyph_picker_rects_out = app::render::glyph_picker::draw_glyph_picker(state, dialog_area, buf);
        }

        // ── Aux-storage prompt — drawn over everything ────────────────────────
        if state.aux_prompt {
            aux_dialog_rects_out = draw_aux_dialog(state, dialog_area, buf);
        }

        // ── Reset dialog overlay — drawn over everything ───────────────────────
        if state.reset_dialog {
            reset_dialog_rects_out = draw_reset_dialog(state, dialog_area, buf);
        }

        // ── Save-name dialog overlay — drawn over everything ───────────────────
        if state.save_name_dialog.is_some() {
            save_name_dialog_rects_out =
                app::render::save_name_dialog::draw_save_name_dialog(state, dialog_area, buf);
        }

        // ── Quit dialog overlay — drawn over everything ────────────────────────
        if state.quit_dialog {
            quit_dialog_rects_out = draw_quit_dialog(state, dialog_area, buf);
        }

        // ── Launch dialog overlay — drawn over everything ──────────────────────
        if state.launch_dialog {
            launch_dialog_rects_out = draw_launch_dialog(state, dialog_area, buf);
        }

        // ── Hints panel overlay — drawn after other overlays ───────────────────
        if state.hints.is_some() {
            hints_panel_rects_out = draw_hints_panel(state, dialog_area, buf);
        }

        // Story-pane text-selection highlight + copy extraction now happen inside
        // render_middle (render/transcript.rs), which has the full wrapped-row set
        // and can select text beyond the visible viewport. (SQ-0197)

        // ── Prompt overlay — map-editing prompts overlay the map. (The save-name
        // prompt is no longer a bottom-bar prompt; it is a common-dialog modal that
        // renders in the graphics-free dialog area — see save_name_dialog.) ───────
        if let Some(prompt) = &state.prompt {
            let overlay_area = if map_area.height > 0 {
                map_area
            } else if story_area.height > 0 {
                story_area
            } else {
                pane_layout.panes_area()
            };
            if overlay_area.height > 0 {
                let y = overlay_area.bottom() - 1;
                let label = match &prompt.kind {
                    PromptKind::RenameRoom(_) => "Rename: ",
                    PromptKind::EditNotes(_) => "Notes:  ",
                    PromptKind::RelabelEdge(_, _) => "Dir:    ",
                    PromptKind::RenameLayer(_) => "Layer:  ",
                    PromptKind::ConfirmDeleteSave(_) => "Del y/n:",
                    PromptKind::ConfigEditPath { .. } => "Path:   ",
                    PromptKind::CreateFile => "File:   ",
                };
                let line = format!("{}{}_", label, prompt.buffer);
                let overlay_style = Style::default().add_modifier(Modifier::REVERSED);
                draw_str_clipped(buf, overlay_area.x, y, &line, overlay_style, overlay_area);
            }
        }
    })?;

    Ok(PaneRects { map: map_area, story: story_area, room_rects: room_rects_out, layer_tabs: layer_tabs_out, dialog: dialog_rects_out, aux_dialog: aux_dialog_rects_out, reset_dialog: reset_dialog_rects_out, save_name_dialog: save_name_dialog_rects_out, quit_dialog: quit_dialog_rects_out, launch_dialog: launch_dialog_rects_out, hints_panel: hints_panel_rects_out, style_editor: style_editor_rects_out, verb_menu: verb_hits, glyph_picker: glyph_picker_rects_out, transcript_links: transcript_links_out, transcript_max_scroll, transcript_viewport_rows, modal_list_viewport })
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
            w.also_watch(&state.config.user_dir.join("styles"));
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
        Action::ExportDump(dest) => {
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
    let mut last_panes = PaneRects { map: Rect::default(), story: Rect::default(), room_rects: Vec::new(), layer_tabs: Vec::new(), dialog: None, aux_dialog: None, reset_dialog: None, save_name_dialog: None, quit_dialog: None, launch_dialog: None, hints_panel: None, style_editor: None, verb_menu: Default::default(), glyph_picker: None, transcript_links: Vec::new(), transcript_max_scroll: 0, transcript_viewport_rows: 0, modal_list_viewport: 0 };

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
                w.also_watch(&state.config.user_dir.join("styles"));
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

        // ── Style watch: drain events, debounce, then reload ──────────────────
        if let Some(w) = &style_watcher {
            let mut saw = false;
            while w.rx.try_recv().is_ok() { saw = true; }
            if saw { watch_dirty = Some(std::time::Instant::now()); }
        } else {
            // Watch turned off: drop any pending debounce so it can't fire later.
            watch_dirty = None;
        }
        if app::watch::due(watch_dirty, std::time::Instant::now(), Duration::from_millis(200)) {
            watch_dirty = None;
            needs_redraw = true; // style reload changes colours/status → repaint
            match app::reload::reload_style(&mut state) {
                app::reload::ReloadOutcome::Reloaded { warnings } => {
                    for wn in &warnings {
                        state.push_transcript_internal(wn, TranscriptKind::Warning);
                    }
                    state.set_status("style reloaded (watch)");
                }
                app::reload::ReloadOutcome::Failed { msg } => {
                    state.push_transcript_internal(
                        &format!("style reload failed: {}", msg),
                        TranscriptKind::Warning,
                    );
                }
            }
        }

        // ── Glulx re-arrange on settled story-pane size (SQ-0201) ─────────────
        // Uses last frame's story rect (one-frame lag is fine). Runs BEFORE the
        // draw so the resized graphics show on the next frame. Glulx-only; the
        // Z-machine renders its own fixed virtual screen into the pane.
        if session.as_any().is::<GlulxSession>() {
            let now = std::time::Instant::now();
            let cur = (last_panes.story.width, last_panes.story.height);
            if cur.0 > 0 && cur.1 > 0 {
                if Some(cur) != story_size_seen {
                    story_size_seen = Some(cur);
                    resize_dirty = Some(now); // size moved; (re)start the settle timer
                }
                if Some(cur) != vm_story_size
                    && app::watch::due(resize_dirty, now, Duration::from_millis(150))
                {
                    resize_dirty = None;
                    vm_story_size = Some(cur);
                    needs_redraw = true; // Glulx graphics repaint at the new size
                    if let Some(gs) = session.as_any_mut().downcast_mut::<GlulxSession>() {
                        gs.resize(cur.0 as u32, cur.1 as u32);
                    }
                }
            }
        }

        // ── Background tidy job: poll and apply ───────────────────────────────
        // Check whether the in-flight tidy job has finished. Do this BEFORE the
        // draw so the first fully-drawn frame after completion shows the new layout.
        if state.tidy_job.as_ref().is_some_and(|j| j.handle.is_finished()) {
            needs_redraw = true; // tidy result applied (or re-triggered) → map changes
            let job = state.tidy_job.take().unwrap();
            let current_gen = state.graph_gen;
            let active_layer = job.layer;
            match job.handle.join() {
                Ok(tidied) => {
                    match apply_tidy_result(&mut mapper.graph, tidied, active_layer, job.gen, current_gen) {
                        ApplyTidyOutcome::Applied => {
                            state.bump_graph_gen(); // tidied layout applied → invalidate map memo (SQ-0305)
                            // Re-center on the current room if it moved.
                            if let Some(rid) = mapper.graph.current() {
                                if let Some(room) = mapper.graph.room(rid) {
                                    if let Some(pos) = room.pos {
                                        let (pw, ph) = map_pane_dims(last_panes.map);
                                        state.recenter_on(pos, pw, ph);
                                    }
                                }
                            }
                        }
                        ApplyTidyOutcome::Stale => {
                            // Graph changed mid-tidy: re-trigger a fresh tidy immediately.
                            let active_layer2 = state.active_layer(&mapper.graph);
                            let graph_clone = mapper.graph.clone();
                            let gen2 = state.graph_gen;
                            let handle2 = std::thread::spawn(move || {
                                let mut g = graph_clone;
                                tidy_layer_silent(&mut g, active_layer2);
                                g
                            });
                            state.tidy_job = Some(TidyJob {
                                handle: handle2,
                                layer: active_layer2,
                                gen: gen2,
                                started: std::time::Instant::now(),
                            });
                        }
                    }
                }
                Err(_) => {
                    // Worker panicked: discard result, leave graph as-is. Do not crash.
                }
            }
        }

        // ── Tidy-animation build job: poll and install ────────────────────────
        // The `animate-tidy` command builds its frames off-thread. When the worker
        // finishes, apply the tidied graph (staleness-guarded) and install the anim.
        // Unlike the background tidy above, a stale result is simply discarded — the
        // user asked for one animation, so we do NOT re-trigger a fresh build.
        if state.anim_build_job.as_ref().is_some_and(|j| j.handle.is_finished()) {
            needs_redraw = true; // anim build installed / graph applied → repaint
            let job = state.anim_build_job.take().unwrap();
            let current_gen = state.graph_gen;
            state.status_msg = None;
            if let Ok((frames, tidied)) = job.handle.join() {
                match apply_tidy_result(&mut mapper.graph, tidied, job.layer, job.gen, current_gen) {
                    ApplyTidyOutcome::Applied => {
                        // Instant re-tidy (animate=false) and the anim's final settle both
                        // land the tidied graph here — invalidate the map memo so the live
                        // path shows it (and does not SNAP BACK when the anim ends). (SQ-0305)
                        state.bump_graph_gen();
                        // `animate-tidy` plays the captured frames; the instant `tidy-map`
                        // re-tidy (animate=false) applies the tidied graph without an
                        // animation — it only used the off-thread build for the progress
                        // bar. (SQ-0261)
                        if job.animate {
                            state.tidy_anim = Some(app::state::TidyAnim::new(frames));
                        }
                        // Re-center on the current room if it moved (mirrors the tidy_job path).
                        if let Some(rid) = mapper.graph.current() {
                            if let Some(room) = mapper.graph.room(rid) {
                                if let Some(pos) = room.pos {
                                    let (pw, ph) = map_pane_dims(last_panes.map);
                                    state.recenter_on(pos, pw, ph);
                                }
                            }
                        }
                    }
                    ApplyTidyOutcome::Stale => {
                        // Graph changed during the build: discard the frames and the
                        // tidied result. Do not install an animation or apply a stale graph.
                    }
                }
            }
        }

        // Update char_mode flag so the renderer hides the prompt during read_char.
        let prev_char_mode = state.char_mode;
        let prev_event_wait = state.event_wait;
        state.char_mode = matches!(session.pending_input(), app::session::InputKind::Char);
        // A Glulx timer/mouse/hyperlink-only glk_select: hide the prompt too (no
        // typed input is requested), but unlike char_mode do NOT forward keys to
        // the game — the timer clock / click delivers the event instead.
        state.event_wait = matches!(session.pending_input(), app::session::InputKind::Event);
        // A prompt-visibility transition changes the frame even with no new input.
        if state.char_mode != prev_char_mode || state.event_wait != prev_event_wait {
            needs_redraw = true;
        }

        // Re-arm the timed-input deadline each iteration. Only while the game is
        // actually awaiting input (no dialog/overlay/prompt covering the pane) and
        // honoring timers; `pending_timeout()` is `None` for an untimed read, so
        // this is a no-op for the vast majority of games (regression guard). Timed
        // input is a Z-machine-only concept (ZMSD): `zvm_session_opt` is `None` for
        // a Glulx engine, so the timer never arms there.
        let timer_interval = zvm_session_opt(&*session)
            .and_then(|s| s.pending_timeout())
            .map(|(t, _)| Duration::from_millis(t as u64 * 100));
        let should_arm = state.config.honor_timed_input
            && !state.any_overlay_open()
            && timer_interval.is_some();
        state.input_deadline = next_input_deadline(
            state.input_deadline,
            should_arm,
            timer_interval.unwrap_or(Duration::ZERO),
            std::time::Instant::now(),
        );

        // Re-arm the Glulx Glk timer-events clock (glk_request_timer_events) — the
        // Glulx analogue of `input_deadline`, and independent of it. Armed only
        // when a Glulx game has requested a timer interval and no overlay covers
        // the pane; uses the same arm-once semantics (`next_input_deadline`) so the
        // deadline holds steady until it fires (the fire path below re-arms fresh).
        let glk_timer_interval = glulx_session_opt(&*session).and_then(|s| s.timer_interval());
        let should_arm_glk_timer = !state.any_overlay_open() && glk_timer_interval.is_some();
        state.glulx_timer_next_fire = next_input_deadline(
            state.glulx_timer_next_fire,
            should_arm_glk_timer,
            glk_timer_interval.unwrap_or(Duration::ZERO),
            std::time::Instant::now(),
        );

        // Expire a finished sound pulse so the story border returns to normal.
        if let Some(p) = &state.sound_pulse {
            if p.started.elapsed().as_millis() as u64 >= SOUND_PULSE_MS {
                state.sound_pulse = None;
                needs_redraw = true; // border returns to normal → repaint once
            }
        }

        // Clear the verb-menu content once its slide-out has fully settled
        // (drawer pattern: content persists during the close animation).
        let had_verb_menu = state.verb_menu.is_some();
        state.settle_verb_dock();
        if had_verb_menu && state.verb_menu.is_none() {
            needs_redraw = true; // drawer content dropped → repaint the cleared pane
        }

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
                // Carry this frame's modal list viewport so the next nav action
                // can window/animate the open selection-list modal.
                state.modal_list_viewport = panes.modal_list_viewport;
                // Replay's idx is the source of truth; keep its (animated) list
                // scroll following it. Skip while a scroll is easing so the tween
                // isn't restarted each frame; select() is a no-op once settled.
                let anim = state.config.animation.clone();
                let hist_len = state.history.len();
                if let Some(r) = &mut state.replay {
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
        let sound_active = !state.sound_routines.is_empty() || !state.glulx_sound_notify.is_empty();
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
        // Clamp to whichever clock is due first: the Z-machine timed-input deadline
        // or the Glulx Glk-timer deadline (either may be `None`).
        let next_deadline = [state.input_deadline, state.glulx_timer_next_fire]
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
                        if apply_game_driven_result(
                            &mut state, &mut mapper, &result, &game_dir, last_panes.map,
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
                        if apply_game_driven_result(
                            &mut state, &mut mapper, &result, &game_dir, last_panes.map,
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
                            if apply_game_driven_result(
                                &mut state, &mut mapper, &result, &game_dir, last_panes.map,
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
                        if apply_game_driven_result(
                            &mut state, &mut mapper, &result, &game_dir, last_panes.map,
                        ) {
                            break 'event_loop;
                        }
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
            if let Some(r) = &mut state.replay {
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
            if let Some(s) = &mut state.saves { needs_redraw |= s.scroll.finalize_if_done(); }
            if let Some(fb) = &mut state.file_browser { needs_redraw |= fb.scroll.finalize_if_done(); }
            if let Some(cs) = &mut state.config_screen { needs_redraw |= cs.scroll.finalize_if_done(); }
            if let Some(vm) = &mut state.verb_menu {
                needs_redraw |= vm.verb_scroll.finalize_if_done();
                needs_redraw |= vm.noun_scroll.finalize_if_done();
                needs_redraw |= vm.prep_scroll.finalize_if_done();
            }
            if let Some(r) = &mut state.replay { needs_redraw |= r.scroll.finalize_if_done(); }
            if let Some(h) = &mut state.hints { needs_redraw |= h.finalize_scroll_if_done(); }
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

        // ── Aux-storage prompt intercept — before normal action routing ───────
        // When the first-use aux-storage prompt is open, route events here and
        // continue (swallowing events the dialog does not handle).
        if state.aux_prompt {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Right =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, 1),
                        crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Left =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, -1),
                        code => match aux_dialog_key_focused(code, state.dialog_focus) {
                            AuxDialogAction::Archive => {
                                let mode = app::config::AuxStorage::Archive;
                                state.aux_prompt = false;
                                state.config.aux_storage = mode;
                                let user_dir = state.config.user_dir.clone();
                                let _ = app::config::write_config(&user_dir, &state.config);
                                session.clear_aux_dirty();
                            }
                            AuxDialogAction::Global => {
                                let mode = app::config::AuxStorage::Global;
                                state.aux_prompt = false;
                                state.config.aux_storage = mode;
                                let user_dir = state.config.user_dir.clone();
                                let _ = app::config::write_config(&user_dir, &state.config);
                                let _ = app::aux_store::write_global_aux(&game_dir, session.aux_data());
                                session.clear_aux_dirty();
                            }
                            AuxDialogAction::None => {}
                        },
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseEventKind, MouseButton};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let col = m.column;
                        let row = m.row;
                        let pt = ratatui::layout::Position { x: col, y: row };
                        if let Some(ad) = &last_panes.aux_dialog {
                            let in_close   = ad.close.is_some_and(|r| r.contains(pt));
                            let in_archive = ad.archive.is_some_and(|r| r.contains(pt));
                            let in_global  = ad.global.is_some_and(|r| r.contains(pt));
                            let in_dialog  = ad.area.contains(pt);
                            if in_close || (!in_archive && !in_global && !in_dialog) {
                                // Close button or click outside → Archive (conservative default).
                                let mode = app::config::AuxStorage::Archive;
                                state.aux_prompt = false;
                                state.config.aux_storage = mode;
                                let user_dir = state.config.user_dir.clone();
                                let _ = app::config::write_config(&user_dir, &state.config);
                                session.clear_aux_dirty();
                            } else if in_archive {
                                let mode = app::config::AuxStorage::Archive;
                                state.aux_prompt = false;
                                state.config.aux_storage = mode;
                                let user_dir = state.config.user_dir.clone();
                                let _ = app::config::write_config(&user_dir, &state.config);
                                session.clear_aux_dirty();
                            } else if in_global {
                                let mode = app::config::AuxStorage::Global;
                                state.aux_prompt = false;
                                state.config.aux_storage = mode;
                                let user_dir = state.config.user_dir.clone();
                                let _ = app::config::write_config(&user_dir, &state.config);
                                let _ = app::aux_store::write_global_aux(&game_dir, session.aux_data());
                                session.clear_aux_dirty();
                            }
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
                _ => {}
            }
            continue;
        }

        // ── Reset dialog intercept — before normal action routing ─────────────
        // When the reset dialog is open, route keyboard/mouse directly here and
        // continue (swallowing events the dialog does not handle).
        if state.reset_dialog {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Right =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 4, 1),
                        crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Left =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 4, -1),
                        code => match reset_dialog_key_focused(code, state.dialog_focus) {
                            ResetDialogAction::Confirm => {
                                let clear = state.reset_clear_map;
                                let delete = state.reset_delete_data;
                                state.reset_dialog = false;
                                reset_game(&mut *session, &mut mapper, &mut state, &story_bytes, &story_path, &game_dir, clear, delete);
                            }
                            ResetDialogAction::Cancel => {
                                state.reset_dialog = false;
                            }
                            ResetDialogAction::ToggleClearMap => {
                                state.reset_clear_map = !state.reset_clear_map;
                            }
                            ResetDialogAction::ToggleDeleteData => {
                                state.reset_delete_data = !state.reset_delete_data;
                            }
                            ResetDialogAction::None => {}
                        },
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseEventKind, MouseButton};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let col = m.column;
                        let row = m.row;
                        let pt = ratatui::layout::Position { x: col, y: row };
                        if let Some(rd) = &last_panes.reset_dialog {
                            // Check buttons and close in order: close > reset > cancel > checkbox.
                            let in_close = rd.close.is_some_and(|r| r.contains(pt));
                            let in_reset = rd.reset.is_some_and(|r| r.contains(pt));
                            let in_cancel = rd.cancel.is_some_and(|r| r.contains(pt));
                            let in_checkbox = rd.checkbox.contains(pt);
                            let in_checkbox_data = rd.checkbox_data.contains(pt);
                            let in_dialog = rd.area.contains(pt);
                            if in_close || in_cancel {
                                state.reset_dialog = false;
                            } else if in_reset {
                                let clear = state.reset_clear_map;
                                let delete = state.reset_delete_data;
                                state.reset_dialog = false;
                                reset_game(&mut *session, &mut mapper, &mut state, &story_bytes, &story_path, &game_dir, clear, delete);
                            } else if in_checkbox {
                                state.reset_clear_map = !state.reset_clear_map;
                            } else if in_checkbox_data {
                                state.reset_delete_data = !state.reset_delete_data;
                            } else if !in_dialog {
                                // Click outside the dialog: swallow (do nothing, keep dialog open).
                            }
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
                _ => {}
            }
            continue;
        }

        // ── Save-name dialog intercept — before normal action routing ─────────
        // A common-dialog modal with a caret text field. Focus ring: 0 = field,
        // 1 = Save, 2 = Cancel. The field opens with a greyed date-time default
        // (active = false); Tab/→/edit-keys adopt it for editing, typing starts
        // fresh, Enter on the untouched placeholder saves the default. Submit reuses
        // the handle_save_as save path (the retired bottom-bar prompt is gone).
        if state.save_name_dialog.is_some() {
            let mut do_save = false;
            let mut do_cancel = false;
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    use crossterm::event::{KeyCode, KeyModifiers};
                    // Suppress Ctrl/Alt/Super-modified printable chars (accelerators,
                    // not text). Everything else routes through the state machine.
                    let ctrl_char = matches!(k.code, KeyCode::Char(_))
                        && k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
                    if !ctrl_char {
                        let focus = state.dialog_focus;
                        let dlg = state.save_name_dialog.as_mut().unwrap();
                        let (act, new_focus) = save_name_dialog_key(k.code, dlg, focus);
                        state.dialog_focus = new_focus;
                        match act {
                            SaveNameAction::Save => do_save = true,
                            SaveNameAction::Cancel => do_cancel = true,
                            SaveNameAction::None => {}
                        }
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let pt = ratatui::layout::Position { x: m.column, y: m.row };
                        if let Some(sd) = &last_panes.save_name_dialog {
                            let in_close = sd.close.is_some_and(|r| r.contains(pt));
                            let in_save = sd.save.is_some_and(|r| r.contains(pt));
                            let in_cancel = sd.cancel.is_some_and(|r| r.contains(pt));
                            let in_field = sd.field.is_some_and(|r| r.contains(pt));
                            let in_dialog = sd.area.contains(pt);
                            if in_close || in_cancel {
                                do_cancel = true;
                            } else if in_save {
                                do_save = true;
                            } else if in_field {
                                // Focus + activate the field (caret to end).
                                state.dialog_focus = 0;
                                if let Some(dlg) = state.save_name_dialog.as_mut() {
                                    dlg.active = true;
                                    dlg.field.end();
                                }
                            } else if !in_dialog {
                                // Click outside: swallow, keep the dialog open.
                            }
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
                _ => {}
            }

            // Resolve a submit/cancel outside the dialog borrow. Empty names are
            // rejected (the dialog stays open); valid names reuse handle_saves_prompt.
            if do_save {
                let value = state
                    .save_name_dialog
                    .as_ref()
                    .map(|d| d.field.value.clone())
                    .unwrap_or_default();
                if value.trim().is_empty() {
                    if let Some(d) = state.save_name_dialog.as_mut() { d.active = false; }
                    state.push_notice("[Save name cannot be empty]");
                } else {
                    state.save_name_dialog = None;
                    handle_save_as(
                        value, &game_dir, &ifid, &mut mapper, &mut *session, &mut state,
                    );
                    let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                        || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                    persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                    persist_vfs_after_turn(&mut *session, &game_dir);
                    if quit { break; }
                }
            } else if do_cancel {
                state.save_name_dialog = None;
                let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                    || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                persist_vfs_after_turn(&mut *session, &game_dir);
                if quit { break; }
            }
            continue;
        }

        // ── Quit dialog intercept — before normal action routing ──────────────
        // When the quit dialog is open, route keyboard/mouse directly here and
        // continue (swallowing events the dialog does not handle).
        if state.quit_dialog {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Right =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 3, 1),
                        crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Left =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 3, -1),
                        code => match quit_dialog_key_focused(code, state.dialog_focus) {
                            QuitDialogAction::Save => {
                                state.quit_dialog = false;
                                quit_dialog_save(&*session, &mapper, &state, &ifid, &arc_file);
                                break;
                            }
                            QuitDialogAction::Quit => {
                                break;
                            }
                            QuitDialogAction::Cancel => {
                                state.quit_dialog = false;
                            }
                            QuitDialogAction::None => {}
                        },
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let pt = ratatui::layout::Position { x: m.column, y: m.row };
                        if let Some(qd) = &last_panes.quit_dialog {
                            let in_close = qd.close.is_some_and(|r| r.contains(pt));
                            let in_save = qd.save.is_some_and(|r| r.contains(pt));
                            let in_quit = qd.quit.is_some_and(|r| r.contains(pt));
                            let in_cancel = qd.cancel.is_some_and(|r| r.contains(pt));
                            let in_dialog = qd.area.contains(pt);
                            if in_save {
                                state.quit_dialog = false;
                                quit_dialog_save(&*session, &mapper, &state, &ifid, &arc_file);
                                break;
                            } else if in_quit {
                                break;
                            } else if in_close || in_cancel {
                                state.quit_dialog = false;
                            } else if !in_dialog {
                                // Click outside: swallow (keep dialog open).
                            }
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
                _ => {}
            }
            continue;
        }

        // ── Launch dialog intercept — before normal action routing ────────────
        // When the launch dialog is open, route keyboard/mouse directly here and
        // continue (swallowing events the dialog does not handle).
        if state.launch_dialog {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Right =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, 1),
                        crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Left =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, -1),
                        code => match launch_dialog_key_focused(code, state.dialog_focus) {
                            LaunchDialogAction::Resume => {
                                if let Some((save, lines, kinds, screen)) = state.pending_resume.take() {
                                    state.launch_dialog = false;
                                    apply_launch_resume(&save, lines, kinds, screen, &mut *session, &mut mapper, &mut state, &last_panes, &arc_file);
                                }
                            }
                            LaunchDialogAction::NewGame => {
                                state.launch_dialog = false;
                                state.pending_resume = None;
                            }
                            LaunchDialogAction::None => {}
                        },
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let pt = ratatui::layout::Position { x: m.column, y: m.row };
                        if let Some(ld) = &last_panes.launch_dialog {
                            let in_resume = ld.resume.is_some_and(|r| r.contains(pt));
                            let in_new_game = ld.new_game.is_some_and(|r| r.contains(pt));
                            let in_close = ld.close.is_some_and(|r| r.contains(pt));
                            let in_dialog = ld.area.contains(pt);
                            if in_resume {
                                if let Some((save, lines, kinds, screen)) = state.pending_resume.take() {
                                    state.launch_dialog = false;
                                    apply_launch_resume(&save, lines, kinds, screen, &mut *session, &mut mapper, &mut state, &last_panes, &arc_file);
                                }
                            } else if in_new_game || in_close {
                                // [X] (close) and [New game] both discard the save.
                                state.launch_dialog = false;
                                state.pending_resume = None;
                            } else if !in_dialog {
                                // Click outside: swallow (keep dialog open).
                            }
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); continue; }
                _ => {}
            }
            continue;
        }

        // ── Hints panel intercept — before normal action routing ──────────────
        // When the hints panel is open, route keyboard/mouse directly here and
        // continue (swallowing events the panel does not handle).
        if state.hints.is_some() {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    use crossterm::event::KeyCode;
                    match hint_key_routes(k.code) {
                        HintKeyKind::Close => {
                            state.hints = None;
                        }
                        HintKeyKind::ToSession => {
                            match k.code {
                                KeyCode::Enter => {
                                    if let Some(ref mut hs) = state.hints {
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
                                    if let Some(ref mut hs) = state.hints {
                                        hs.input.pop();
                                    }
                                }
                                KeyCode::Char(c) => {
                                    if let Some(ref mut hs) = state.hints {
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
                                state.hints = None;
                            }
                            // Clicks inside the dialog but not on close: swallow.
                        }
                    } else if let Some(d) = app::input::wheel_delta(m.kind, state.config.mouse_wheel_invert) {
                        // Wheel drives the hint transcript's own scroll. The panel
                        // is intercepted before mouse_to_action, so resolve the
                        // direction (and mouse_wheel_invert) via the shared helper.
                        let max = last_panes.hints_panel.as_ref().map_or(0, |hp| hp.max_scroll);
                        let anim = state.config.animation.clone();
                        if let Some(hs) = &mut state.hints {
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

        // ── Glyph-picker intercept — modal over the style editor ─────────────
        // When the glyph picker is open, route all keyboard events here and
        // continue (swallowing events the picker doesn't handle).
        if state.glyph_picker.is_some() {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    use crossterm::event::KeyCode;
                    match k.code {
                        KeyCode::Esc => {
                            // In custom-entry focus: exit focus only; otherwise cancel picker.
                            if state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                if let Some(p) = &mut state.glyph_picker {
                                    p.custom_focus = false;
                                }
                            } else {
                                apply_action(Action::GlyphPickerCancel, &mut state, &mut mapper);
                            }
                        }
                        KeyCode::Enter => {
                            // In custom-entry focus: commit the typed range (custom_start already
                            // updated on each digit) and exit focus so the grid is browsable.
                            if state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                if let Some(p) = &mut state.glyph_picker {
                                    p.custom_focus = false;
                                }
                            } else {
                                apply_action(Action::GlyphPickerPick, &mut state, &mut mapper);
                            }
                        }
                        KeyCode::Delete | KeyCode::Backspace => {
                            if state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                apply_action(Action::GlyphPickerCustomBackspace, &mut state, &mut mapper);
                            } else {
                                // Clear the pending selection (revert to grid cursor).
                                if let Some(p) = &mut state.glyph_picker {
                                    p.pending = None;
                                }
                            }
                        }
                        KeyCode::Left => {
                            if !state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                apply_action(Action::GlyphPickerNav(-1), &mut state, &mut mapper);
                            }
                        }
                        KeyCode::Right => {
                            if !state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                apply_action(Action::GlyphPickerNav(1), &mut state, &mut mapper);
                            }
                        }
                        KeyCode::Up => {
                            if !state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                apply_action(Action::GlyphPickerNav(-(app::input::GLYPH_GRID_COLS as i32)), &mut state, &mut mapper);
                            }
                        }
                        KeyCode::Down => {
                            if !state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                apply_action(Action::GlyphPickerNav(app::input::GLYPH_GRID_COLS as i32), &mut state, &mut mapper);
                            }
                        }
                        KeyCode::Char(',') | KeyCode::Char('[') => {
                            apply_action(Action::GlyphPickerBlock(-1), &mut state, &mut mapper);
                        }
                        KeyCode::Char('.') | KeyCode::Char(']') => {
                            apply_action(Action::GlyphPickerBlock(1), &mut state, &mut mapper);
                        }
                        KeyCode::Char(c) => {
                            if state.glyph_picker.as_ref().is_some_and(|p| p.custom_focus) {
                                // In custom-entry mode: only hex digits are accepted.
                                if c.is_ascii_hexdigit() {
                                    apply_action(Action::GlyphPickerCustomChar(c), &mut state, &mut mapper);
                                }
                                // Non-hex chars swallowed (modal intercept).
                            } else {
                                apply_action(Action::GlyphPickerChar(c), &mut state, &mut mapper);
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(m) => {
                    use crossterm::event::{MouseEventKind, MouseButton};
                    if m.kind == MouseEventKind::Down(MouseButton::Left) {
                        let pt = ratatui::layout::Position { x: m.column, y: m.row };
                        if let Some(gp) = &last_panes.glyph_picker {
                            // Close button.
                            if gp.close.is_some_and(|r| r.contains(pt)) {
                                apply_action(Action::GlyphPickerCancel, &mut state, &mut mapper);
                            // Glyph cells: set pending + pick.
                            } else {
                                let mut picked = false;
                                for (g, r) in &gp.glyphs {
                                    if r.contains(pt) {
                                        apply_action(Action::GlyphPickerChar(g.chars().next().unwrap_or(' ')), &mut state, &mut mapper);
                                        apply_action(Action::GlyphPickerPick, &mut state, &mut mapper);
                                        picked = true;
                                        break;
                                    }
                                }
                                if !picked {
                                    for (g, r) in &gp.mru {
                                        if r.contains(pt) {
                                            apply_action(Action::GlyphPickerChar(g.chars().next().unwrap_or(' ')), &mut state, &mut mapper);
                                            apply_action(Action::GlyphPickerPick, &mut state, &mut mapper);
                                            picked = true;
                                            break;
                                        }
                                    }
                                }
                                if !picked {
                                    if gp.blocks_prev.is_some_and(|r| r.contains(pt)) {
                                        apply_action(Action::GlyphPickerBlock(-1), &mut state, &mut mapper);
                                    } else if gp.blocks_next.is_some_and(|r| r.contains(pt)) {
                                        apply_action(Action::GlyphPickerBlock(1), &mut state, &mut mapper);
                                    } else if gp.clear.is_some_and(|r| r.contains(pt)) {
                                        apply_action(Action::GlyphPickerClear, &mut state, &mut mapper);
                                    } else if gp.custom.is_some_and(|r| r.contains(pt)) {
                                        apply_action(Action::GlyphPickerCustomFocus, &mut state, &mut mapper);
                                    }
                                    // Clicks outside modal area: swallow (modal is top).
                                }
                            }
                        }
                    }
                }
                Event::Resize(_, _) => { let _ = terminal.clear(); }
                _ => {}
            }
            continue;
        }

        // ── Config-screen Tab focus intercept ────────────────────────────────
        // Ring length 2: [Save(0), Cancel(1)].
        if state.config_screen.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        crossterm::event::KeyCode::Tab =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, 1),
                        crossterm::event::KeyCode::BackTab =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, -1),
                        _ => {}
                    }
                }
            }
        }

        // ── Saves Tab focus intercept ─────────────────────────────────────────
        // Ring length 1: [Done(0)].
        if state.saves.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    match k.code {
                        crossterm::event::KeyCode::Tab =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 1, 1),
                        crossterm::event::KeyCode::BackTab =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 1, -1),
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
                            .and_then(|ki| session.submit_key(ki))
                        {
                            if apply_game_driven_result(
                                &mut state, &mut mapper, &result, &game_dir, last_panes.map,
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
                            state.status_msg = None;
                            state.turns += 1;
                            state.unsaved_progress = true;
                            let result = zvm_session_opt_mut(&mut *session)
                                .expect("z-machine line read is pending")
                                .submit_line_with_terminator(&cmd, term);
                            if finish_command_turn(
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
                        let close_leader = state.hotkey_dialog;
                        let outcome = slash::parse_in_context(&s, state.config.command_prefix, ctx);
                        let should_break = dispatch_slash_outcome(
                            outcome, &mut state, &mut mapper, &mut *session, &mut style_watcher,
                            &game_dir, &ifid, &arc_file, &story_bytes, &story_path,
                            last_panes.map, last_panes.story, true,
                        );
                        if close_leader {
                            state.hotkey_dialog = false;
                        }
                        flush_pending_config_write(&mut state);
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
                                        if apply_game_driven_result(
                                            &mut state, &mut mapper, &result, &game_dir, last_panes.map,
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
                                if apply_game_driven_result(
                                    &mut state, &mut mapper, &result, &game_dir, last_panes.map,
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
                if state.verb_menu.is_some() {
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
                            if let Some(vm) = &mut state.verb_menu { vm.story_focused = true; }
                            // fall through: normal story-pane click handling (selection) still runs below.
                        }
                    }
                }
                // Style-editor board: intercept left-clicks on sample rows and property pane.
                if state.style_editor.is_some() {
                    // Holds a dialog-button action that must flow through the normal
                    // run-loop path (so the style_save flag fires save_style_and_repoint).
                    let mut click_action = Action::None;
                    if matches!(m.kind, crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)) {
                        if let Some(rects) = &last_panes.style_editor {
                            // Helper: is the cursor inside a rect?
                            let hit = |rect: &ratatui::layout::Rect| {
                                rect.width > 0 && rect.height > 0
                                    && m.column >= rect.x && m.column < rect.right()
                                    && m.row >= rect.y && m.row < rect.bottom()
                            };

                            // Sample board: set active selector.
                            for (idx, rect) in &rects.samples {
                                if hit(rect) {
                                    if let Some(ed) = &mut state.style_editor {
                                        ed.active = *idx;
                                    }
                                    continue 'event_loop;
                                }
                            }

                            // Attribute chips.
                            for (kind, rect) in &rects.attr_chips {
                                if hit(rect) {
                                    let kind = *kind;
                                    apply_action(Action::StyleToggleAttr(kind), &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }

                            // Fg swatch row (17 rects: 0-15 = ANSI, 16 = default).
                            for (i, rect) in rects.fg_swatches.iter().enumerate() {
                                if hit(rect) {
                                    if let Some(ed) = &mut state.style_editor { ed.color_target = false; }
                                    let value = if i < app::style_mru::ANSI_NAMES.len() {
                                        Some(app::style_mru::ANSI_NAMES[i].to_string())
                                    } else {
                                        Some("reset".to_string())
                                    };
                                    apply_action(Action::StyleSetColor { is_bg: false, value }, &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }

                            // Bg swatch row.
                            for (i, rect) in rects.bg_swatches.iter().enumerate() {
                                if hit(rect) {
                                    if let Some(ed) = &mut state.style_editor { ed.color_target = true; }
                                    let value = if i < app::style_mru::ANSI_NAMES.len() {
                                        Some(app::style_mru::ANSI_NAMES[i].to_string())
                                    } else {
                                        Some("reset".to_string())
                                    };
                                    apply_action(Action::StyleSetColor { is_bg: true, value }, &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }

                            // MRU row.
                            for (i, rect) in rects.mru_rects.iter().enumerate() {
                                if hit(rect) {
                                    let hex = state.style_editor.as_ref()
                                        .and_then(|ed| ed.mru.get(i).cloned());
                                    if let Some(hex) = hex {
                                        let is_bg = state.style_editor.as_ref().is_some_and(|e| e.color_target);
                                        apply_action(Action::StyleSetColor { is_bg, value: Some(hex) }, &mut state, &mut mapper);
                                    }
                                    continue 'event_loop;
                                }
                            }

                            // Custom hex entry cell → switch focus to Custom.
                            if let Some(rect) = &rects.custom_rect {
                                if hit(rect) {
                                    use app::state::StyleFocus;
                                    if let Some(ed) = &mut state.style_editor {
                                        ed.focus = StyleFocus::Custom;
                                        if ed.custom_buf.is_empty() {
                                            ed.custom_buf = "#".to_string();
                                        }
                                    }
                                    continue 'event_loop;
                                }
                            }

                            // Border zone cells.
                            for (zone, rect) in &rects.border_zones {
                                if hit(rect) {
                                    let zone = *zone;
                                    apply_action(Action::StyleOpenGlyphPicker(zone), &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }
                            // Border type cycle arrows.
                            if let Some(rect) = &rects.border_type_prev {
                                if hit(rect) {
                                    apply_action(Action::StyleBorderTypeCycle(-1), &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }
                            if let Some(rect) = &rects.border_type_next {
                                if hit(rect) {
                                    apply_action(Action::StyleBorderTypeCycle(1), &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }
                            // Header/shadow toggles.
                            if let Some(rect) = &rects.border_header {
                                if hit(rect) {
                                    apply_action(Action::StyleBorderToggleHeader, &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }
                            if let Some(rect) = &rects.border_shadow {
                                if hit(rect) {
                                    apply_action(Action::StyleBorderToggleShadow, &mut state, &mut mapper);
                                    continue 'event_loop;
                                }
                            }

                            // Dialog buttons: Save / Cancel / close [X].
                            // These must reach the run-loop action path so the style_save
                            // flag fires and save_style_and_repoint writes style.toml.
                            if let Some(act) = style_dialog_action(&rects.dialog, m.column, m.row) {
                                click_action = act;
                            }
                        }
                    }
                    // Wheel drives the selector list via mouse_to_action's
                    // modal-precedence branch; swallow all other unhandled mouse
                    // events. Dialog-button actions flow through the run-loop path.
                    if matches!(m.kind, crossterm::event::MouseEventKind::ScrollUp | crossterm::event::MouseEventKind::ScrollDown) {
                        mouse_to_action(&state, m, last_panes.map, last_panes.story, &last_panes.room_rects, &last_panes.dialog)
                    } else {
                        click_action
                    }
                } else {
                    mouse_to_action(&state, m, last_panes.map, last_panes.story, &last_panes.room_rects, &last_panes.dialog)
                }
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

        // Note whether this action closes the gallery (persist the look afterward).
        let gallery_cfg_on_close = matches!(action, Action::GalleryClose | Action::GalleryApply);

        // Note whether this action is the on-demand "Output all settings" export.
        let export_style_now = matches!(action, Action::GalleryExportStyle);

        // Note whether this action is a style-editor save (for post-apply disk write).
        let style_save = matches!(action, Action::StyleSave);
        let style_save_game = matches!(action, Action::StyleSaveGame);

        // Snapshot working config before apply_action clears it on ConfigSave.
        let config_to_save = if matches!(action, Action::ConfigSave) {
            state.config_screen.as_ref().map(|cs| cs.working.clone())
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
                    state.quit_dialog = true;
                    state.dialog_focus = 0;
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

            Action::SubmitCommand(cmd) => {
                // When a prompt is active, SubmitCommand is the Enter sentinel;
                // route to apply_action to apply the prompt to the mapper.
                if state.prompt.is_some() {
                    apply_action(Action::SubmitCommand(cmd), &mut state, &mut mapper);
                    // Handle any saves-manager or reset prompt that was submitted.
                    if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
                        handle_saves_prompt(kind, buf, &game_dir, &mut state);
                    }
                    // Resume an in-game save/restore if this prompt resolved it.
                    let quit = resolve_ingame_dialog(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map)
                        || resolve_filename_request(&mut *session, &mut mapper, &mut state, &game_dir, &ifid, last_panes.map);
                    persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                    persist_vfs_after_turn(&mut *session, &game_dir);
                    if quit { break; }
                    continue;
                }

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
                        flush_pending_config_write(&mut state);
                        if should_break {
                            break;
                        }
                        continue;
                    }
                }

                // Clear any transient status message on a real game turn.
                state.status_msg = None;

                // Increment the session turn counter. Progress now exists that
                // isn't captured in a Save State (drives the quit prompt).
                state.turns += 1;
                state.unsaved_progress = true;

                let result = session.submit(&cmd);
                if finish_command_turn(
                    &cmd, result, &mut state, &mut mapper, &mut *session,
                    &game_dir, &ifid, &arc_file, last_panes.map, &mut bg_tidy_counter,
                ) {
                    break;
                }
            }

            Action::SaveGame => {
                // Dead post-unification: keys now route through SlashOutcome::Save. Retained as a no-cost match arm.
                // Bundle map + game into a single .babelmap archive, with turn metadata.
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
                };
                match save_archive_meta(&arc_file, &mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.history, &state.command_history) {
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
            a @ (Action::ExportSvg(_) | Action::ExportDot(_) | Action::ExportDump(_)) => {
                handle_map_export(&a, &game_dir, &mapper, &mut state);
            }

            // ── Saves-manager actions ─────────────────────────────────────────

            Action::OpenSaves => {
                // Populate the saves list (both .babelmap Save States and .qzl
                // game saves — SQ-0227 Task 3) and open the modal.
                let entries = combined_saves(&game_dir);
                state.saves = Some(SavesState { entries, scroll: Default::default() });
                state.dialog_focus = 0;
            }

            Action::SavesImport => {
                // Close saves modal and open file browser in PickFile mode.
                // Start in this story's per-game dir (where its saves live, honoring
                // --data-dir), falling back to the data base then the user dir.
                state.saves = None;
                let start_dir = if game_dir.is_dir() {
                    game_dir.clone()
                } else if data_base.is_dir() {
                    data_base.clone()
                } else {
                    state.config.user_dir.clone()
                };
                state.file_browser = Some(FileBrowserState::build(start_dir, FbMode::PickFile));
            }

            Action::FbEnter => {
                // Handle file-browser Enter: cd into dir or import file.
                let fb_action = state.file_browser.as_ref().and_then(|fb| {
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
                        if let Some(fb) = &mut state.file_browser {
                            fb.cd(path);
                        }
                    }
                    Some(FbEntryAction::ImportFile(path)) => {
                        state.file_browser = None;
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
                // Clone path and name to release the borrow on state.saves before mutating state.
                let load_info = state.saves.as_ref().and_then(|s| {
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
                    state.saves = None;
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
                    let quit = finish_resumed_turn(result, &mut mapper, &mut state, &*session, &game_dir, &ifid, last_panes.map);
                    persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                    persist_vfs_after_turn(&mut *session, &game_dir);
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
                            state.saves = None;
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
                            state.saves = None;
                        }
                        Err(e) => {
                            state.push_notice(&format!("[Load failed: {}]", e));
                            if ingame_restore_pending {
                                state.saves = None;
                                state.ingame_io = None;
                                let result = session.resume_restore(None);
                                let quit = finish_resumed_turn(result, &mut mapper, &mut state, &*session, &game_dir, &ifid, last_panes.map);
                                persist_aux_after_turn(&mut *session, &mut state, &game_dir);
                                persist_vfs_after_turn(&mut *session, &game_dir);
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
                if let Some(r) = state.replay.take() {
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
                                // parallel vec length-synced (unstyled rows).
                                state.transcript_runs = vec![Vec::new(); state.transcript.len()];
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

            // ── apply_action handles everything else ───────────────────────────
            other => {
                apply_action(other, &mut state, &mut mapper);
            }
        }

        // After apply_action: check for saves-manager or reset prompt that was submitted.
        // (This covers the case where apply_action routed a saves/reset prompt submit.)
        if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
            handle_saves_prompt(kind, buf, &game_dir, &mut state);
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
        persist_aux_after_turn(&mut *session, &mut state, &game_dir);
        persist_vfs_after_turn(&mut *session, &game_dir);
        if quit {
            break;
        }

        // After apply_action: if resize mode was just exited or reset, persist the
        // (possibly changed) pane sizes to config.toml. Also covers the
        // `KeyResolve::Command` dispatch path via the `flush_pending_config_write`
        // calls placed right before its `continue`s above.
        flush_pending_config_write(&mut state);

        // After apply_action: if gallery was just closed, write the resolved look to
        // the personal style file and repoint config.toml at it.
        if gallery_cfg_on_close {
            let user_dir = state.config.user_dir.clone();
            save_style_and_repoint(&mut state, &user_dir);
        }

        // After apply_action: if the "Output all settings" button was pressed, sync the
        // live gallery selections, then write_style_full + repoint on demand (gallery
        // stays open).
        if export_style_now {
            if let Some(g) = state.gallery.as_ref() {
                state.symbols = app::symbols::SymbolSet::resolve(&g.symbol_config());
            }
            let user_dir = state.config.user_dir.clone();
            save_style_and_repoint(&mut state, &user_dir);
        }

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

        // After apply_action: if the style editor was just saved, write the live
        // colors (already set by the handler) to the personal style file and repoint.
        if style_save {
            let user_dir = state.config.user_dir.clone();
            save_style_and_repoint(&mut state, &user_dir);
        }

        // After apply_action: if Save Game Style was used, write the live look
        // self-contained to the current game's per-game style file.
        if style_save_game {
            let user_dir = state.config.user_dir.clone();
            if !state.ifid.is_empty() {
                let _ = app::styles::save_per_game_style(
                    &user_dir, &state.ifid, &state.colors, &state.symbols,
                );
            }
        }
    }

    // ── 6. Exit: restore terminal + (optional) autosave ───────────────────────

    restore_terminal();

    exit_auto_save(&*session, &mapper, &state, &ifid, &arc_file);
}

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
fn exit_auto_save(
    session: &dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if !state.config.auto_save || session.is_saveload_pending() {
        return;
    }
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
    };
    match save_archive_meta(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), exit_meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.history, &state.command_history) {
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
fn quit_dialog_save(
    session: &dyn Engine,
    mapper: &Mapper,
    state: &app::state::AppState,
    ifid: &str,
    arc_file: &std::path::Path,
) {
    if session.is_saveload_pending() {
        return;
    }
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
    let _ = save_archive_meta(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.history, &state.command_history);
}

// ── Pending config-write flush ────────────────────────────────────────────────

/// Write `state.config` to `config.toml` if `pending_config_write` is set, then
/// clear the flag. Called after both key-dispatch paths (`KeyResolve::Action`
/// and `KeyResolve::Command`, the latter via `dispatch_slash_outcome`) so a
/// resize-reset/exit persists regardless of which path handled the key.
fn flush_pending_config_write(state: &mut AppState) {
    if state.pending_config_write {
        let user_dir = state.config.user_dir.clone();
        let _ = app::config::write_config(&user_dir, &state.config);
        state.pending_config_write = false;
    }
}

// ── Reset helper ──────────────────────────────────────────────────────────────

/// Rebuild the session from `story_bytes`, reset all ephemeral state, and
/// re-seed the mapper with the start room.  When `clear_map` is true, the
/// accumulated map is wiped first (same effect as `/reset map`) so only the
/// start room remains after the re-seed.
/// Handle a parsed `SlashOutcome` from either typed input or a key dispatch.
///
/// Both the typed-command path and the keybinding path resolve to a
/// `SlashOutcome` and funnel through here so the two share one behaviour. The
/// run loop owns the actual loop, so the `Quit` outcome cannot `break` directly:
/// this returns `true` when the loop should break (a non-dialog quit).
#[allow(clippy::too_many_arguments)]
fn dispatch_slash_outcome(
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
            // Source-aware: a key press (e.g. F5) opens the confirmation dialog with
            // its checkboxes; a typed `/reset-game [map] [data]` acts immediately.
            if from_key {
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

/// Resolve the Pict/graphics blorb for a story the same way at launch and
/// restart: path-based (self-contained blorb, same-stem sidecar, or dir scan).
fn resolve_pict_blorb(story_path: &std::path::Path, images: bool) -> Option<blorb::Blorb> {
    if images {
        blorb::resolve_sound_blorb(story_path).map(|(b, _)| b)
    } else {
        None
    }
}

fn reset_game(
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
            GlulxSession::new_in(
                game_dir.to_path_buf(),
                bytes,
                state.config.virtual_screen_cols as u32,
                state.config.virtual_screen_rows as u32,
                state.config.acceleration,
                state.config.images,
                state.config.enable_sound,
                char_px,
                pict_blorb,
                &carry_vfs,
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

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Handle a submitted saves-manager delete-confirm prompt.
/// Called after apply_action stores the prompt in `state.saves_prompt_submitted`.
fn handle_saves_prompt(
    kind: PromptKind,
    buf: String,
    dir: &std::path::Path,
    state: &mut AppState,
) {
    match kind {
        PromptKind::ConfirmDeleteSave(path) => {
            let confirmed = matches!(buf.trim().to_lowercase().as_str(), "y" | "yes");
            if confirmed {
                match delete_save(&path) {
                    Ok(()) => {
                        state.push_notice("[Save deleted]");
                        if let Some(s) = &mut state.saves {
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
        _ => {} // other prompt kinds are handled elsewhere
    }
}

/// Handle a submitted save name (host "Save State" slot or in-game `@save`).
/// Called directly from the save-name dialog submit. On success it refreshes the
/// saves list; a host save also clears `unsaved_progress`, while an in-game save
/// sets `ingame_resume_save` so the run loop resumes the VM. An empty name or a
/// write error re-opens the dialog when in-game so the user can retry.
fn handle_save_as(
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
            state.save_name_dialog = Some(app::state::SaveNameDialog::new(
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
                let bytes = glulx_session_opt(&*session).map(|g| g.save_quetzal()).unwrap_or_default();
                app::persist_files::save_game_named_bytes(dir, &buf, &bytes).map(|_| ())
            }
        }
    } else {
        // Host "Save State" named slot -> rich .babelmap archive.
        save_named(dir, ifid, &buf, mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), state.turns, &state.transcript, &state.transcript_kinds, &state.transcript_runs)
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
            if let Some(s) = &mut state.saves {
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
                state.save_name_dialog = Some(app::state::SaveNameDialog::new(
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

/// Apply a completed game-turn `result` from a submitted command line: echo the
/// command, push its transcript, advance the mapper, run post-turn bookkeeping /
/// auto-save / background tidy, and recenter on the current room. Shared by the
/// normal `SubmitCommand` path and the terminator-key submit gate (SQ-0188).
/// Returns `true` if the app should exit after this turn.
#[allow(clippy::too_many_arguments)]
fn finish_command_turn(
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
    if let Some(note) = &result.info {
        state.push_transcript(note);
    }

    // Capture room + connection counts before apply_turn, to detect
    // whether THIS turn actually changed the graph (a non-mutating
    // command like "look" leaves both unchanged).
    let rooms_before = mapper.graph.rooms().count();
    let conns_before = mapper.graph.connections().len();

    apply_turn(mapper, cmd, &result);

    // Bump the graph generation so any in-flight tidy result is detected as stale.
    state.graph_gen = state.graph_gen.wrapping_add(1);

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
    persist_vfs_after_turn(session, game_dir);

    // Background tidy: silently re-tidy the active layer when the
    // configured mode calls for it. Only runs in Auto layout mode.
    // Overlap signal is computed for ALL modes (not only OnOverlap).
    if mapper.mode == mapper::layout::LayoutMode::Auto {
        let new_room = mapper.graph.rooms().count() > rooms_before;
        let active_layer = state.active_layer(&mapper.graph);
        // Always compute overlap so all modes can react to it.
        let cells = mapper::layout::occupied_cells_in_layer(&mapper.graph, active_layer);
        let total_rooms = mapper.graph.rooms_in_layer(active_layer).len();
        let has_overlap = cells.len() < total_rooms;
        let has_distorted = mapper.graph.connections().iter().any(|c| {
            c.distorted
                && mapper.graph.layer_of(c.origin) == active_layer
                && mapper.graph.layer_of(c.dest) == active_layer
        });
        let overlap = has_overlap || has_distorted;
        // Only auto-tidy on turns that actually changed the graph, so a
        // bare "look" (overlap persists, graph unchanged) doesn't pulse.
        let new_conn = mapper.graph.connections().len() > conns_before;
        let changed = new_room || new_conn;
        if should_bg_tidy(state.config.background_tidy, new_room, overlap, changed, bg_tidy_counter) {
            // Spawn a worker thread only if no job is currently in flight (coalesce).
            if state.tidy_job.is_none() {
                let graph_clone = mapper.graph.clone();
                let gen = state.graph_gen;
                let handle = std::thread::spawn(move || {
                    let mut g = graph_clone;
                    tidy_layer_silent(&mut g, active_layer);
                    g
                });
                state.tidy_job = Some(TidyJob {
                    handle,
                    layer: active_layer,
                    gen,
                    started: std::time::Instant::now(),
                });
            }
            // If a job is already in flight we skip spawning; the gen check after
            // join will detect the stale result and re-trigger as needed.
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

    should_exit_on_turn(&result, state)
}

fn open_ingame_saves(
    io: app::session::PendingIo,
    game_dir: &std::path::Path,
    state: &mut AppState,
) {
    use app::session::PendingIo;
    state.ingame_io = Some(io);
    state.dialog_focus = 0;
    match io {
        PendingIo::Save => {
            // The game asked to SAVE: ask where via the save-name dialog. On submit
            // -> resume_save(true); on cancel -> resume_save(false) (handled in the
            // cancel resolver, which now watches save_name_dialog).
            state.save_name_dialog = Some(app::state::SaveNameDialog::new(
                app::persist_files::default_save_name(),
                true,
            ));
        }
        PendingIo::Restore => {
            // The game asked to RESTORE: list babelmap saves + plain .qzl files.
            let entries = combined_saves(game_dir);
            state.saves = Some(SavesState { entries, scroll: Default::default() });
        }
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
        if let Err(e) = save_archive_meta(arc_file, mapper, &session.save_state(), zvm_session_opt(session).map(|z| &z.machine.screen), session.aux_data(), meta, &state.transcript, &state.transcript_kinds, &state.transcript_runs, &state.history, &state.command_history) {
            state.push_notice(&format!("[Auto-save failed: {}]", e));
        }
    }
}

/// After a turn, persist the VM's aux table if it changed.  Archive mode is
/// already covered by the per-turn auto-save (`save_archive_meta` embeds it);
/// global mode writes the per-game file here.  `Ask` opens the first-use
/// prompt dialog (Task 6) and leaves `aux_dirty` set for the dialog to resolve.
fn persist_aux_after_turn(
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
            state.aux_prompt = true; // resolve in the dialog; leave aux_dirty set
            state.dialog_focus = 0;
        }
    }
}

/// Flush the Glulx Glk file VFS to its per-story sidecar when it changed this
/// turn. Dirty-gated; a no-op for the Z-machine (whose `vfs_dirty` default is
/// always false). Mirrors `persist_aux_after_turn`.
fn persist_vfs_after_turn(
    session: &mut dyn Engine,
    game_dir: &std::path::Path,
) {
    if !session.vfs_dirty() {
        return;
    }
    let _ = app::vfs_store::write_vfs(game_dir, &session.vfs_bytes());
    session.clear_vfs_dirty();
}

/// Post-process a TurnResult produced by `session.resume_*`: render output,
/// re-observe the location, recenter, run post-turn bookkeeping, and record a
/// *chained* in-game I/O if the resume itself suspended on another
/// `@save`/`@restore`. Returns true if the app should quit. Mirrors the
/// post-turn block in the `submit` path.
fn finish_resumed_turn(
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

/// Resolve a pending in-game save/restore after the dialog interaction:
/// (1) a flag-hopped successful SAVE resumes the VM; (2) an in-game overlay that
/// closed without a confirm is treated as a cancel and resumes with failure.
/// Re-opens the dialog for a chained request. Returns true if the app should quit.
fn resolve_ingame_dialog(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    game_dir: &std::path::Path,
    ifid: &str,
    map_area: Rect,
) -> bool {
    use app::session::PendingIo;

    // (1) SAVE confirmed in handle_saves_prompt (flag-hop): resume here.
    if let Some(wrote_ok) = state.ingame_resume_save.take() {
        state.ingame_io = None;
        let result = session.resume_save(wrote_ok);
        let quit = finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
        if let Some(io) = state.ingame_io {
            open_ingame_saves(io, game_dir, state);
        }
        return quit;
    }

    // (2) Cancel: an in-game overlay closed without a confirm.
    if let Some(io) = state.ingame_io {
        let overlay_open = match io {
            PendingIo::Save => state.save_name_dialog.is_some(),
            PendingIo::Restore => state.saves.is_some(),
        };
        if !overlay_open {
            state.ingame_io = None;
            let result = match io {
                PendingIo::Save => session.resume_save(false),
                PendingIo::Restore => session.resume_restore(None),
            };
            state.push_notice("[In-game save/restore cancelled]");
            let quit = finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
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
fn open_filename_modal(req: app::session::FilenameReq, session: &dyn Engine, state: &mut AppState) {
    state.pending_filename = Some(req);
    match app::state::filename_modal_for(req, session.file_names().len()) {
        app::state::FilenameModal::NamePrompt => {
            state.prompt = Some(app::state::Prompt {
                kind: app::state::PromptKind::CreateFile,
                buffer: String::new(),
            });
        }
        app::state::FilenameModal::Picker => {
            state.file_picker = Some(app::state::FilePickerState::new(session.file_names()));
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
fn resolve_filename_request(
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
        let quit = finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
        if let Some(io) = state.ingame_io {
            open_ingame_saves(io, game_dir, state);
        }
        return quit;
    }
    // Modal closed without a submit (Esc) while a request is still pending -> cancel.
    if state.pending_filename.is_some()
        && !matches!(&state.prompt, Some(p) if p.kind == app::state::PromptKind::CreateFile)
        && state.file_picker.is_none()
    {
        state.pending_filename = None;
        let result = session.resume_filename(None);
        state.push_notice("[create_by_prompt cancelled]");
        let quit = finish_resumed_turn(result, mapper, state, session, game_dir, ifid, map_area);
        if let Some(io) = state.ingame_io {
            open_ingame_saves(io, game_dir, state);
        }
        return quit;
    }
    false
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

// ── Hints panel keyboard routing ──────────────────────────────────────────────

/// Routing decision for a key pressed while the hints panel is open.
enum HintKeyKind {
    /// Close the hints panel (Esc).
    Close,
    /// Route the key to the hint sub-session.
    ToSession,
}

/// Map a key code to a HintKeyKind.
/// Esc → Close; everything else → ToSession.
fn hint_key_routes(code: crossterm::event::KeyCode) -> HintKeyKind {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => HintKeyKind::Close,
        _ => HintKeyKind::ToSession,
    }
}

// ── Slash-command helper ──────────────────────────────────────────────────────

/// Return true when `input` starts with the configured command `prefix` char.
fn is_slash(input: &str, prefix: char) -> bool {
    input.starts_with(prefix)
}

// ── Reset dialog keyboard routing ─────────────────────────────────────────────

/// Action to take when a key is pressed while the reset dialog is open.
enum ResetDialogAction {
    None,
    ToggleClearMap,
    ToggleDeleteData,
    Confirm,
    Cancel,
}

/// Map a key code to a ResetDialogAction (focus-agnostic accelerators only).
/// Esc and 'c' cancel; Enter and 'r' confirm; Space toggles the clear-map box.
#[cfg_attr(not(test), allow(dead_code))]
fn reset_dialog_key(code: crossterm::event::KeyCode) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Enter | KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => ResetDialogAction::ToggleClearMap,
        _ => ResetDialogAction::None,
    }
}

/// Reset-dialog keys with focus. Tab/BackTab are handled by the caller (which
/// mutates dialog_focus over a 4-slot ring: 0 = clear-map checkbox, 1 = delete-data
/// checkbox, 2 = Reset, 3 = Cancel). Space toggles the focused checkbox; Enter
/// activates the focused button (or confirms when a checkbox is focused); 'r'/'c'
/// stay as confirm/cancel accelerators.
fn reset_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => match focus {
            0 => ResetDialogAction::ToggleClearMap,
            1 => ResetDialogAction::ToggleDeleteData,
            _ => ResetDialogAction::None,
        },
        KeyCode::Enter => match focus {
            3 => ResetDialogAction::Cancel,
            _ => ResetDialogAction::Confirm, // checkboxes and Reset (focus 2) confirm
        },
        _ => ResetDialogAction::None,
    }
}

// ── Save-name dialog keyboard routing ─────────────────────────────────────────

/// What a key press resolves to in the save-name dialog.
enum SaveNameAction {
    None,
    Save,
    Cancel,
}

/// Apply one key to the save-name dialog's field state machine and focus ring
/// (0 = text field, 1 = Save, 2 = Cancel). Mutates `dlg.field`/`dlg.active` in
/// place and returns the resolved action plus the new focus slot. Callers gate
/// out Ctrl/Alt-modified printable chars before calling.
///
/// Field-focused behavior (see SQ-0289 table): a greyed placeholder (`active =
/// false`) is adopted for editing by Tab/→/Home/End/←/Backspace/Delete (without
/// advancing focus for Tab/→); typing a printable char starts fresh; Enter on the
/// placeholder saves the default; in active mode Enter saves a non-empty value and
/// reverts an empty one to the placeholder.
fn save_name_dialog_key(
    code: crossterm::event::KeyCode,
    dlg: &mut app::state::SaveNameDialog,
    focus: usize,
) -> (SaveNameAction, usize) {
    use crossterm::event::KeyCode;
    if focus == 0 {
        // ── Text field focused ──
        match code {
            KeyCode::Esc => return (SaveNameAction::Cancel, focus),
            KeyCode::Enter => {
                if dlg.active && dlg.field.value.is_empty() {
                    dlg.active = false; // empty: revert to placeholder
                    return (SaveNameAction::None, focus);
                }
                return (SaveNameAction::Save, focus); // placeholder saves default
            }
            KeyCode::BackTab => return (SaveNameAction::None, 2), // reverse-wrap to Cancel
            KeyCode::Tab => {
                if dlg.active {
                    return (SaveNameAction::None, 1); // advance to Save
                }
                dlg.active = true; // adopt default for editing
                dlg.field.end();
            }
            KeyCode::Right => {
                if dlg.active {
                    dlg.field.right();
                } else {
                    dlg.active = true; // adopt default
                    dlg.field.end();
                }
            }
            KeyCode::Left => {
                if !dlg.active {
                    dlg.active = true;
                    dlg.field.end();
                }
                dlg.field.left();
            }
            KeyCode::Home => {
                dlg.active = true;
                dlg.field.home();
            }
            KeyCode::End => {
                dlg.active = true;
                dlg.field.end();
            }
            KeyCode::Backspace => {
                if !dlg.active {
                    dlg.active = true;
                    dlg.field.end();
                }
                dlg.field.backspace();
            }
            KeyCode::Delete => {
                if !dlg.active {
                    dlg.active = true;
                    dlg.field.end();
                }
                dlg.field.delete();
            }
            KeyCode::Char(c) => {
                if dlg.active {
                    dlg.field.insert(c);
                } else {
                    // Typing on the placeholder starts fresh.
                    dlg.field.set(String::new(), false);
                    dlg.field.insert(c);
                    dlg.active = true;
                }
            }
            _ => {}
        }
        (SaveNameAction::None, focus)
    } else {
        // ── Save / Cancel button focused ──
        match code {
            KeyCode::Esc => (SaveNameAction::Cancel, focus),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if focus == 1 {
                    (SaveNameAction::Save, focus)
                } else {
                    (SaveNameAction::Cancel, focus)
                }
            }
            KeyCode::Tab | KeyCode::Right => (SaveNameAction::None, app::input::cycle_focus(focus, 3, 1)),
            KeyCode::BackTab | KeyCode::Left => (SaveNameAction::None, app::input::cycle_focus(focus, 3, -1)),
            _ => (SaveNameAction::None, focus),
        }
    }
}

// ── Aux-storage prompt keyboard routing ──────────────────────────────────────

/// Action to take when a key is pressed while the aux-storage prompt is open.
enum AuxDialogAction {
    None,
    Archive,
    Global,
}

/// Aux-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button.
/// Esc defaults to Archive (conservative: always resolves the prompt).
fn aux_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> AuxDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => AuxDialogAction::Archive, // conservative default
        KeyCode::Enter => match focus {
            1 => AuxDialogAction::Global,
            _ => AuxDialogAction::Archive, // focus 0 = Archive (default)
        },
        _ => AuxDialogAction::None,
    }
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
    if state.hints.is_some() {
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
                            state.hints = Some(app::state::HintSession {
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
                            state.hints = Some(app::state::HintSession {
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

/// Action to take when a key is pressed while the quit dialog is open.
enum QuitDialogAction {
    None,
    Save,
    Quit,
    Cancel,
}

/// Map a key code to a QuitDialogAction.
/// 's' or Enter → Save State & quit; 'q' → Quit without saving; Esc or 'c' → Cancel.
#[cfg_attr(not(test), allow(dead_code))]
fn quit_dialog_key(code: crossterm::event::KeyCode) -> QuitDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Char('s') | KeyCode::Enter => QuitDialogAction::Save,
        KeyCode::Char('q') => QuitDialogAction::Quit,
        KeyCode::Esc | KeyCode::Char('c') => QuitDialogAction::Cancel,
        _ => QuitDialogAction::None,
    }
}

/// Quit-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button and keeps
/// the existing accelerators.
fn quit_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> QuitDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => QuitDialogAction::Cancel,
        KeyCode::Char('s') => QuitDialogAction::Save,
        KeyCode::Char('q') => QuitDialogAction::Quit,
        KeyCode::Enter => match focus {
            1 => QuitDialogAction::Quit,
            2 => QuitDialogAction::Cancel,
            _ => QuitDialogAction::Save, // focus 0 = Save & quit (default)
        },
        _ => QuitDialogAction::None,
    }
}

// ── Launch dialog helpers ─────────────────────────────────────────────────────

/// Action to take when a key is pressed while the launch dialog is open.
enum LaunchDialogAction {
    None,
    Resume,
    NewGame,
}

/// Map a key code to a LaunchDialogAction.
/// 'r' or Enter → Resume; 'n' or Esc → New game.
#[cfg_attr(not(test), allow(dead_code))]
fn launch_dialog_key(code: crossterm::event::KeyCode) -> LaunchDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Char('r') | KeyCode::Enter => LaunchDialogAction::Resume,
        KeyCode::Char('n') | KeyCode::Esc => LaunchDialogAction::NewGame,
        _ => LaunchDialogAction::None,
    }
}

/// Launch-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button and keeps
/// the existing accelerators.
fn launch_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> LaunchDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc => LaunchDialogAction::NewGame,
        KeyCode::Char('r') => LaunchDialogAction::Resume,
        KeyCode::Char('n') => LaunchDialogAction::NewGame,
        KeyCode::Enter => match focus {
            1 => LaunchDialogAction::NewGame,
            _ => LaunchDialogAction::Resume, // focus 0 = Resume (default)
        },
        _ => LaunchDialogAction::None,
    }
}

/// Apply a pending resume: restore the VM save, set transcript, re-observe location.
///
/// Mirrors the Action::RestoreGame path exactly (restore_quetzal, set transcript,
/// apply_turn to re-observe current room, set_viewed_layer(None), select_room, recenter).
fn apply_launch_resume(
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
            // vec length-synced (unstyled rows).
            state.transcript_runs = vec![Vec::new(); state.transcript.len()];
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
        let crash = state.colors.transcript_crash;
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
fn apply_game_driven_result(
    state: &mut AppState,
    mapper: &mut Mapper,
    result: &TurnResult,
    game_dir: &std::path::Path,
    map_area: Rect,
) -> bool {
    if result.erase_lower { state.mark_screen_clear(); }
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
    apply_turn(mapper, "", result);
    // Game-initiated (v4+) save/restore: open the saves dialog in in-game mode
    // and defer the rest of the turn.
    if let Some(io) = result.pending_io {
        open_ingame_saves(io, game_dir, state);
        return false;
    }
    state.graph_gen = state.graph_gen.wrapping_add(1);
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
fn next_input_deadline(
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::{aux_dialog_key_focused, dim_area, hint_bar, hint_key_routes, is_slash, launch_dialog_key, launch_dialog_key_focused, quit_dialog_key, quit_dialog_key_focused, reset_dialog_key, reset_dialog_key_focused, save_name_dialog_key, scroll_for_match, should_prompt_save_on_quit, AuxDialogAction, HintKeyKind, LaunchDialogAction, QuitDialogAction, ResetDialogAction, SaveNameAction};
    use super::{ANIM_HINTS, GAME_HINTS, MAP_HINTS};
    use app::keymap::{Context, HotkeyLayout, KeyMap};
    use app::render::paneframe::{draw_pane_frame, draw_top_inset, InsetSegment, PaneGlyphs};

    // ── SQ-0289: save-name dialog field state machine ─────────────────────────

    use crossterm::event::KeyCode;
    use app::state::SaveNameDialog;

    fn sn_dialog() -> SaveNameDialog {
        SaveNameDialog::new("2026-07-13 1432".to_string(), false)
    }

    #[test]
    fn sn_placeholder_adopts_default_on_tab_and_right_without_advancing_focus() {
        for code in [KeyCode::Tab, KeyCode::Right] {
            let mut d = sn_dialog();
            let (act, focus) = save_name_dialog_key(code, &mut d, 0);
            assert!(matches!(act, SaveNameAction::None));
            assert_eq!(focus, 0, "adopting the default must not advance focus");
            assert!(d.active, "field becomes active");
            assert_eq!(d.field.value, "2026-07-13 1432", "value keeps the default");
            assert_eq!(d.field.cursor, d.field.char_len(), "caret at end");
        }
    }

    #[test]
    fn sn_typing_on_placeholder_starts_fresh() {
        let mut d = sn_dialog();
        let (act, focus) = save_name_dialog_key(KeyCode::Char('h'), &mut d, 0);
        assert!(matches!(act, SaveNameAction::None));
        assert_eq!(focus, 0);
        assert!(d.active);
        assert_eq!(d.field.value, "h");
        assert_eq!(d.field.cursor, 1);
        // Subsequent chars insert normally.
        save_name_dialog_key(KeyCode::Char('i'), &mut d, 0);
        assert_eq!(d.field.value, "hi");
    }

    #[test]
    fn sn_enter_on_placeholder_saves_the_default() {
        let mut d = sn_dialog();
        let (act, _) = save_name_dialog_key(KeyCode::Enter, &mut d, 0);
        assert!(matches!(act, SaveNameAction::Save));
        assert_eq!(d.field.value, "2026-07-13 1432", "the default is what gets saved");
    }

    #[test]
    fn sn_enter_active_empty_reverts_to_placeholder_and_nonempty_saves() {
        let mut d = sn_dialog();
        // Emptying the field: type then delete it all.
        save_name_dialog_key(KeyCode::Char('x'), &mut d, 0);
        save_name_dialog_key(KeyCode::Backspace, &mut d, 0);
        assert!(d.field.value.is_empty() && d.active);
        let (act, _) = save_name_dialog_key(KeyCode::Enter, &mut d, 0);
        assert!(matches!(act, SaveNameAction::None), "empty Enter does not save");
        assert!(!d.active, "reverts to placeholder");
        // Now type a real name and Enter → Save.
        save_name_dialog_key(KeyCode::Char('a'), &mut d, 0);
        let (act2, _) = save_name_dialog_key(KeyCode::Enter, &mut d, 0);
        assert!(matches!(act2, SaveNameAction::Save));
        assert_eq!(d.field.value, "a");
    }

    #[test]
    fn sn_backspace_and_home_adopt_default_for_editing() {
        let mut d = sn_dialog();
        let (_, _) = save_name_dialog_key(KeyCode::Backspace, &mut d, 0);
        assert!(d.active, "backspace on placeholder adopts");
        assert_eq!(d.field.value, "2026-07-13 143", "then deletes the last char");

        let mut d2 = sn_dialog();
        save_name_dialog_key(KeyCode::Home, &mut d2, 0);
        assert!(d2.active);
        assert_eq!(d2.field.cursor, 0, "Home adopts then moves to start");
    }

    #[test]
    fn sn_field_focus_tab_and_shifttab_move_to_buttons() {
        // Editing mode: Tab advances to Save (1); Shift-Tab wraps to Cancel (2).
        let mut d = sn_dialog();
        d.active = true;
        let (act, focus) = save_name_dialog_key(KeyCode::Tab, &mut d, 0);
        assert!(matches!(act, SaveNameAction::None));
        assert_eq!(focus, 1);
        let (_, focus2) = save_name_dialog_key(KeyCode::BackTab, &mut d, 0);
        assert_eq!(focus2, 2, "Shift-Tab from the field reverse-wraps to Cancel");
    }

    #[test]
    fn sn_active_right_moves_cursor_not_focus() {
        let mut d = sn_dialog();
        d.active = true;
        d.field.home();
        let (act, focus) = save_name_dialog_key(KeyCode::Right, &mut d, 0);
        assert!(matches!(act, SaveNameAction::None));
        assert_eq!(focus, 0, "Right in edit mode stays on the field");
        assert_eq!(d.field.cursor, 1);
    }

    #[test]
    fn sn_esc_cancels_from_field_or_button() {
        let mut d = sn_dialog();
        assert!(matches!(save_name_dialog_key(KeyCode::Esc, &mut d, 0).0, SaveNameAction::Cancel));
        assert!(matches!(save_name_dialog_key(KeyCode::Esc, &mut d, 1).0, SaveNameAction::Cancel));
    }

    #[test]
    fn sn_buttons_activate_and_cycle() {
        let mut d = sn_dialog();
        // Save button (focus 1): Enter/Space → Save.
        assert!(matches!(save_name_dialog_key(KeyCode::Enter, &mut d, 1).0, SaveNameAction::Save));
        assert!(matches!(save_name_dialog_key(KeyCode::Char(' '), &mut d, 1).0, SaveNameAction::Save));
        // Cancel button (focus 2): Enter → Cancel.
        assert!(matches!(save_name_dialog_key(KeyCode::Enter, &mut d, 2).0, SaveNameAction::Cancel));
        // Tab cycles 1 → 2 → 0; Shift-Tab reverses.
        assert_eq!(save_name_dialog_key(KeyCode::Tab, &mut d, 1).1, 2);
        assert_eq!(save_name_dialog_key(KeyCode::Tab, &mut d, 2).1, 0);
        assert_eq!(save_name_dialog_key(KeyCode::BackTab, &mut d, 1).1, 0);
    }

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

        assert!(super::handle_map_export(&Action::ExportDump(None), &dir, &mapper, &mut state));
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
            is_default: false,
        };
        let mut v = vec![
            mk("old", "2026-06-01T10:00:00Z"),
            mk("legacy", ""),
            mk("new", "2026-07-09T12:00:00Z"),
            mk("mid", "2026-06-30T08:00:00Z"),
        ];
        // Same comparator combined_saves uses (RFC3339 sorts chronologically).
        v.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        let order: Vec<&str> = v.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(order, vec!["new", "mid", "old", "legacy"],
            "newest first; untimestamped/legacy saves sort to the bottom");
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

    // ── SQ-0227 Task 3: restore dispatch on file extension ──────────────────────
    //
    // `restore_from_file` is the dispatch shared by every host restore site
    // (saves-manager Load, `/restore-state`, and a `.babelmap` picked from the
    // in-game restore picker). Regression proof for SQ-0163: every host
    // restore path used to call `restore_state` (resume) unconditionally, so
    // a host restore of an in-game `@save` (`.qzl`) landed the VM on the
    // descriptor instead of past it.

    /// Minimal v4 story: `read_char` (store->G0) at 0x40, then `@save` (store
    /// form, ->G0) at 0x44, then `quit` at 0x46. Mirrors session.rs's
    /// (crate-private) `read_char_then_save_v4` fixture, duplicated here
    /// since this test lives in the separate `app` *binary* crate.
    fn read_char_then_save_v4_story() -> Vec<u8> {
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

    // SQ-0260: the launch-dialog auto-resume must restore the saved turn counter.
    // The stash it works from carries no turn count, so apply_launch_resume reads
    // it from the archive (like the interactive restore) instead of leaving it 0.
    #[test]
    fn launch_resume_restores_the_turn_counter_sq0260() {
        use app::engine::Engine;
        use app::session::GameSession;

        // A Save State (.babelmap) written with a non-zero turn count.
        let sess = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let save = sess.save_state();
        let arc = std::env::temp_dir().join(format!("bm-sq260-{}.babelmap", std::process::id()));
        let meta = app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None,
            name: None,
            turns: 42,
            saved_at: String::new(),
        };
        app::archive::save_archive_meta(
            &arc, &mapper::mapper::Mapper::default(), &save, None,
            &std::collections::BTreeMap::new(), meta, &[], &[], &[], &[], &[],
        ).expect("write .babelmap with turns=42");

        // Fresh session + default state (turns start at 0), then launch-resume.
        let mut fresh = GameSession::new(read_char_then_save_v4_story(), true, false, None).expect("new");
        let mut state = app::state::AppState::default();
        let mut mapper = mapper::mapper::Mapper::default();
        let panes = super::PaneRects {
            map: ratatui::layout::Rect::default(), story: ratatui::layout::Rect::default(),
            room_rects: Vec::new(), layer_tabs: Vec::new(), dialog: None, aux_dialog: None,
            reset_dialog: None, save_name_dialog: None, quit_dialog: None, launch_dialog: None, hints_panel: None,
            style_editor: None, verb_menu: Default::default(), glyph_picker: None,
            transcript_links: Vec::new(), transcript_max_scroll: 0, transcript_viewport_rows: 0,
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

    #[test]
    fn resolve_pict_blorb_finds_sidecar_for_bare_ulx() {
        // Regression test for SQ-0173: restart's Pict-blorb resolution must find
        // a same-stem sidecar .blorb for a bare .ulx the same path-based way as
        // launch (blorb::resolve_sound_blorb), not the old bytes-only
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

        // Build a minimal FORM/IFRS blorb with a Pict (PNG) resource and a Snd
        // resource. resolve_sound_blorb's same-stem sidecar step only accepts a
        // sidecar that has_sounds(), so a Snd entry is required even though only
        // the Pict resource matters for this test (mirrors blorb::lib's own
        // build_blorb test helper).
        fn build_sidecar_blorb(png: &[u8]) -> Vec<u8> {
            let res: [(&[u8; 4], u32, &[u8; 4], &[u8]); 2] =
                [(b"Pict", 0, b"PNG ", png), (b"Snd ", 1, b"FORM", b"xy")];
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

    // ── TestBackend: map pane shows picture-frame top-left by default ──────────

    /// Verify that the DEFAULT_STYLE_TOML-resolved ColorScheme configures
    /// `map_border_style` as picture-frame, and that rendering it produces the
    /// block outer border (ramp corner ▁, thin side ▕) and the inner-frame content.
    #[test]
    fn map_pane_default_shows_picture_frame_corner() {
        // Resolve the default look from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf, area, cs.map_border_style, &PaneGlyphs::default(), cs.map_border);
        // DEFAULT_STYLE_TOML sets map_border to picture-frame: top outer row is the
        // lower-block ramp (▁ at the corner), the sides are thin one-eighth blocks.
        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            "▁",
            "default map border (from DEFAULT_STYLE_TOML) must be picture-frame (ramp ▁ at top-left)"
        );
        assert_eq!(
            buf.cell((0, 3)).unwrap().symbol(),
            "▕",
            "picture-frame left side must be the thin ▕ block"
        );
        // Content lives inside the inner frame (20-4=16, 10-4=6).
        assert_eq!(frame.content, Rect::new(2, 2, 16, 6));
    }

    // ── TestBackend: story pane shows adventure title in picture-frame border ─────

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
        let frame = draw_pane_frame(&mut buf, area, cs.story_border_style, &PaneGlyphs::default(), cs.story_border);

        // Overlay the adventure title (single centered segment, not active).
        draw_top_inset(
            &mut buf,
            frame.top_inset,
            &[InsetSegment { text: "ZORK I", active: false }],
            cs.story_title,
            cs.story_title,
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

    // ── hint_bar ───────────────────────────────────────────────────────────────

    #[test]
    fn hint_line_map_contains_zoom_with_plus_key() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        // With default keymap: zoom-map in primary key is '+'; short label is "zoom map in".
        assert!(line.contains("+: zoom"), "expected '+: zoom' in '{line}'");
    }

    #[test]
    fn map_hint_bar_excludes_dialog_only_commands() {
        // Regression (#11): gallery/inspector/layout moved to the Ctrl+K dialog
        // after the leader-key change; the hint bar must NOT advertise their dead
        // direct keys. The is_direct filter excludes them (they are dialog-only).
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        assert!(!line.contains("gallery"), "must not advertise gallery (dialog-only): {line}");
        assert!(!line.contains("inspector"), "must not advertise inspector (dialog-only): {line}");
        assert!(!line.contains("layout"), "must not advertise cycle-layout (dialog-only): {line}");
        // The working direct keys ARE present.
        assert!(line.contains("Tab: toggle focus"), "focus toggle present: {line}");
        assert!(line.contains("+: zoom"), "zoom present: {line}");
    }

    #[test]
    fn hint_line_game_contains_save_state() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        // Ctrl+S → save-state; short label is "save state".
        assert!(line.contains("Ctrl+S: save state"), "expected 'Ctrl+S: save state' in '{line}'");
        // cycle-layout was trimmed out of the always-active set (SQ-0202); it's
        // leader-only now and must not appear in the Game hint bar.
        assert!(!line.contains("cycle"), "Game hint bar must not contain 'cycle': '{line}'");
    }

    #[test]
    fn leader_hint_advertises_ctrl_k_menu() {
        // The bottom-bar default branch prepends "{prefix.label()}: menu" ahead
        // of the hint_bar output (SQ-0202). Pin the exact construction here since
        // the help-row assembly itself lives inline in the render loop.
        let layout = HotkeyLayout::default();
        let leader_hint = format!("{}: menu", layout.prefix.label());
        assert_eq!(leader_hint, "Ctrl+K: menu");
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
        assert!(s.hotkey_dialog, "hotkey_dialog should be true after OpenHotkeyDialog");
    }

    #[test]
    fn prefix_key_closes_hotkey_dialog() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use app::input::{apply_action, key_to_action, Action};
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        s.hotkey_dialog = true;
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
        assert!(!s.hotkey_dialog, "hotkey_dialog should be false after CloseHotkeyDialog");
    }

    #[test]
    fn apply_open_gallery_clears_hotkey_dialog() {
        use app::input::{apply_action, Action};
        use app::state::AppState;
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        s.hotkey_dialog = true;
        apply_action(Action::OpenGallery, &mut s, &mut Mapper::default());
        assert!(!s.hotkey_dialog, "OpenGallery should clear hotkey_dialog");
        assert!(s.gallery.is_some(), "gallery should be open");
    }

    // ── hint_bar invariant: no dead keys, no tidy, is_direct gating, truncation ─

    #[test]
    fn hint_bar_never_contains_tidy() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let map_line = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        let game_line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        assert!(
            !map_line.to_lowercase().contains("tidy") && !map_line.to_lowercase().contains("retidy"),
            "map hint bar must not contain tidy/retidy; got: '{map_line}'"
        );
        assert!(
            !game_line.to_lowercase().contains("tidy") && !game_line.to_lowercase().contains("retidy"),
            "game hint bar must not contain tidy/retidy; got: '{game_line}'"
        );
    }

    #[test]
    fn hint_bar_no_dead_keys_all_entries_resolve_back() {
        // Every entry shown must pass the round-trip check: lookup(primary_key(cmd), ctx) == Some(cmd).
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        for (ctx, hints) in [
            (Context::Map, MAP_HINTS),
            (Context::Global, GAME_HINTS),
            (Context::Anim, ANIM_HINTS),
        ] {
            for &cmd in hints {
                if !layout.is_direct_name(cmd) {
                    continue;
                }
                let name = cmd.split_whitespace().next().unwrap_or("");
                if let Some(k) = km.primary_key(name) {
                    let resolved = km.lookup(&k, ctx);
                    if resolved == Some(cmd) {
                        // This entry would be shown — verify label format.
                        let entry = format!("{}: {}", k.label(), super::hint_label(cmd));
                        let bar = hint_bar(&km, &layout, ctx, hints, 200);
                        assert!(
                            bar.contains(&entry),
                            "bar for {ctx:?} should contain '{entry}'; got: '{bar}'"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn hint_bar_drops_non_direct_command() {
        use std::collections::HashSet;
        // Build a layout where zoom-map in is NOT direct (dialog-only), but toggle-focus IS.
        let mut direct: HashSet<String> = HashSet::new();
        direct.insert("toggle-focus".into());
        direct.insert("center-map".into());
        direct.insert("select-room next".into());
        // zoom-map in intentionally NOT in direct set.
        let layout = HotkeyLayout {
            prefix: "ctrl+k".parse().unwrap(),
            direct,
            groups: vec![],
        };
        let km = KeyMap::default();
        let bar = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        assert!(
            !bar.contains("zoom"),
            "zoom-map in should be absent when not direct; got: '{bar}'"
        );
        // toggle-focus IS direct, so it should appear.
        assert!(
            bar.contains("focus"),
            "toggle-focus should still appear when direct; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_truncates_at_width() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        // Use a very narrow width (10 chars) — the full bar is much longer.
        let bar = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 10);
        let char_count = bar.chars().count();
        assert!(
            char_count <= 10,
            "bar must not exceed width=10; got {char_count} chars: '{bar}'"
        );
        assert!(
            bar.ends_with('…'),
            "truncated bar must end with ellipsis; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_no_truncation_when_wide_enough() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        // Use a very generous width — no truncation expected.
        let bar = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 1000);
        assert!(
            !bar.ends_with('…'),
            "bar should not be truncated at width=1000; got: '{bar}'"
        );
    }

    #[test]
    fn hint_bar_shows_short_registry_labels() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        // center-map is direct and bound to 'c' in Map; its short label is "center map".
        assert!(line.contains("center map"), "hint bar should show the short label 'center map', got: {line}");
        // The full description sentence must NOT appear.
        assert!(!line.contains("re-center the map"), "hint bar must not show the long description");
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

        // Use a 30x15 area (large enough for picture-frame: requires w>=7, h>=7).
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

    // ── reset_dialog_key_mapping ──────────────────────────────────────────────

    #[test]
    fn reset_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        assert!(matches!(reset_dialog_key(KeyCode::Esc), ResetDialogAction::Cancel));
        assert!(matches!(reset_dialog_key(KeyCode::Char('c')), ResetDialogAction::Cancel));
        assert!(matches!(reset_dialog_key(KeyCode::Enter), ResetDialogAction::Confirm));
        assert!(matches!(reset_dialog_key(KeyCode::Char('r')), ResetDialogAction::Confirm));
        assert!(matches!(reset_dialog_key(KeyCode::Char(' ')), ResetDialogAction::ToggleClearMap));
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

    // ── quit_dialog_key ───────────────────────────────────────────────────────

    #[test]
    fn quit_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        assert!(matches!(quit_dialog_key(KeyCode::Char('s')), QuitDialogAction::Save));
        assert!(matches!(quit_dialog_key(KeyCode::Enter), QuitDialogAction::Save));
        assert!(matches!(quit_dialog_key(KeyCode::Char('q')), QuitDialogAction::Quit));
        assert!(matches!(quit_dialog_key(KeyCode::Esc), QuitDialogAction::Cancel));
        assert!(matches!(quit_dialog_key(KeyCode::Char('c')), QuitDialogAction::Cancel));
        assert!(matches!(quit_dialog_key(KeyCode::Char('x')), QuitDialogAction::None));
        assert!(matches!(quit_dialog_key(KeyCode::Left), QuitDialogAction::None));
    }

    // ── launch_dialog_key ─────────────────────────────────────────────────────

    #[test]
    fn launch_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        assert!(matches!(launch_dialog_key(KeyCode::Char('r')), LaunchDialogAction::Resume));
        assert!(matches!(launch_dialog_key(KeyCode::Enter), LaunchDialogAction::Resume));
        assert!(matches!(launch_dialog_key(KeyCode::Char('n')), LaunchDialogAction::NewGame));
        assert!(matches!(launch_dialog_key(KeyCode::Esc), LaunchDialogAction::NewGame));
        assert!(matches!(launch_dialog_key(KeyCode::Char('x')), LaunchDialogAction::None));
        assert!(matches!(launch_dialog_key(KeyCode::Left), LaunchDialogAction::None));
    }

    // ── launch_dialog counts as overlay ──────────────────────────────────────

    #[test]
    fn launch_dialog_counts_as_overlay() {
        let mut s = app::state::AppState::default();
        assert!(!s.any_overlay_open(), "default state has no overlay");
        s.launch_dialog = true;
        assert!(s.any_overlay_open(), "launch_dialog true => any_overlay_open true");
        s.launch_dialog = false;
        assert!(!s.any_overlay_open(), "launch_dialog false => any_overlay_open false");
    }

    // ── hint_key_routes ───────────────────────────────────────────────────────

    #[test]
    fn hint_panel_keys_close_on_esc_else_route() {
        use crossterm::event::KeyCode;
        assert!(matches!(hint_key_routes(KeyCode::Esc), HintKeyKind::Close));
        assert!(matches!(hint_key_routes(KeyCode::Char('a')), HintKeyKind::ToSession));
    }

    /// Regression: Enter must route to the hint session input (ToSession), not Close.
    /// The hints panel has a text input; Enter submits that input regardless of any
    /// default-button decoration on the Close button.
    #[test]
    fn hints_enter_submits_input_not_close() {
        use crossterm::event::KeyCode;
        let routed = hint_key_routes(KeyCode::Enter);
        assert!(
            matches!(routed, HintKeyKind::ToSession),
            "Enter must be routed to the hint session input (ToSession), not Close"
        );
    }

    // ── reset_dialog_tab_then_enter_fires_focused ─────────────────────────────

    #[test]
    fn reset_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // Focus ring (4): 0 = clear-map checkbox, 1 = delete-data checkbox,
        // 2 = Reset, 3 = Cancel. Default focus 0.
        let mut focus = 0usize;
        // Space on focus 0 toggles the clear-map checkbox.
        assert!(matches!(reset_dialog_key_focused(KeyCode::Char(' '), focus), ResetDialogAction::ToggleClearMap));
        // Tab -> focus 1 (delete-data checkbox); Space toggles delete-data.
        focus = app::input::cycle_focus(focus, 4, 1);
        assert_eq!(focus, 1);
        assert!(matches!(reset_dialog_key_focused(KeyCode::Char(' '), focus), ResetDialogAction::ToggleDeleteData));
        // Enter on a focused checkbox confirms.
        assert!(matches!(reset_dialog_key_focused(KeyCode::Enter, focus), ResetDialogAction::Confirm));
        // Tab to focus 3 (Cancel); Enter cancels.
        focus = app::input::cycle_focus(focus, 4, 1); // 2 = Reset
        focus = app::input::cycle_focus(focus, 4, 1); // 3 = Cancel
        assert_eq!(focus, 3);
        assert!(matches!(reset_dialog_key_focused(KeyCode::Enter, focus), ResetDialogAction::Cancel));
        // Enter on focus 2 (Reset) confirms.
        assert!(matches!(reset_dialog_key_focused(KeyCode::Enter, 2), ResetDialogAction::Confirm));
    }

    // ── aux_dialog_key_mapping ────────────────────────────────────────────────

    #[test]
    fn aux_dialog_key_mapping() {
        use crossterm::event::KeyCode;
        // Esc → Archive (conservative default so prompt always resolves).
        assert!(matches!(aux_dialog_key_focused(KeyCode::Esc, 0), AuxDialogAction::Archive));
        assert!(matches!(aux_dialog_key_focused(KeyCode::Esc, 1), AuxDialogAction::Archive));
        // Enter on focus 0 → Archive; Enter on focus 1 → Global.
        assert!(matches!(aux_dialog_key_focused(KeyCode::Enter, 0), AuxDialogAction::Archive));
        assert!(matches!(aux_dialog_key_focused(KeyCode::Enter, 1), AuxDialogAction::Global));
        // Other keys → None.
        assert!(matches!(aux_dialog_key_focused(KeyCode::Char('x'), 0), AuxDialogAction::None));
    }

    // ── aux_dialog_tab_then_enter_fires_global ────────────────────────────────

    #[test]
    fn aux_dialog_tab_then_enter_fires_global() {
        use crossterm::event::KeyCode;
        // buttons: [Archive(0), Global(1)], default focus 0.
        // Tab -> focus 1 (Global); Enter on focus 1 -> Global.
        let mut focus = 0usize;
        focus = app::input::cycle_focus(focus, 2, 1);
        assert_eq!(focus, 1);
        let act = aux_dialog_key_focused(KeyCode::Enter, focus);
        assert!(matches!(act, AuxDialogAction::Global));
    }

    // ── quit_dialog_tab_then_enter_fires_focused ──────────────────────────────

    #[test]
    fn quit_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // buttons: [Save State & quit(0), Quit(1), Cancel(2)], default focus 0.
        // Tab -> focus 1 (Quit); Enter on focus 1 -> Quit.
        let mut focus = 0usize;
        focus = app::input::cycle_focus(focus, 3, 1);
        assert_eq!(focus, 1);
        let act = quit_dialog_key_focused(KeyCode::Enter, focus);
        assert!(matches!(act, QuitDialogAction::Quit));
    }

    // ── launch_dialog_tab_then_enter_fires_focused ────────────────────────────

    #[test]
    fn launch_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // buttons: [Resume(0), New game(1)], default focus 0.
        // Tab -> focus 1 (New game); Enter on focus 1 -> NewGame.
        let mut focus = 0usize;
        focus = app::input::cycle_focus(focus, 2, 1);
        assert_eq!(focus, 1);
        let act = launch_dialog_key_focused(KeyCode::Enter, focus);
        assert!(matches!(act, LaunchDialogAction::NewGame));
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
        s.hotkey_dialog = true;
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
            transcript_elems: Vec::new(),
        }
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

    #[test]
    fn apply_turn_events_halts_and_logs_on_fault() {
        let tmp = std::env::temp_dir().join(format!("babelmap-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).expect("create temp user_dir");
        let mut state = app::state::AppState::default();
        state.config.user_dir = tmp.clone();

        let result = fault_test_result(true, Some(vec!["some fault line".to_string()]));
        super::apply_turn_events(&mut state, &result);

        assert!(state.vm_halted, "a fault must set vm_halted");
        assert!(state.status_msg.is_some(), "a fault must set a user-visible status");

        let log = std::fs::read_to_string(tmp.join("crash.log")).expect("crash.log written");
        assert!(log.contains("gvm runtime fault"), "crash.log must record the fault header");
        assert!(log.contains("some fault line"), "crash.log must record the fault line");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
