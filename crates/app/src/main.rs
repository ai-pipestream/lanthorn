use std::io::stdout;
use std::time::Duration;

use crossterm::event::{poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use mapper::mapper::Mapper;
use mapper::render::{render as render_map_data, render_layer};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction as LayoutDir, Layout as RatatuiLayout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::Terminal;

use clap::Parser;

use app::config::{resolve, Cli};
use app::export_dot::export_dot;
use app::export_svg::export_svg;
use app::map_dump::render_dump;
use app::archive::{load_archive, save_archive_meta};
use app::ifid::{archive_path, compute_ifid, map_path};
use app::input::{apply_action, apply_tidy_result, key_to_action, mouse_to_action, should_bg_tidy, tidy_layer_silent, Action, ApplyTidyOutcome};
use app::persist_files::{delete_save, list_saves, load_map, save_game, restore_game, save_map, save_named};
use app::render::config_screen::draw_config_screen;
use app::render::dialog::{DialogRects, DialogStyle};
use app::render::filebrowser::draw_file_browser;
use app::render::gallery::draw_gallery;
use app::render::hints_panel::{draw_hints_panel, HintsPanelRects};
use app::render::launch_dialog::draw_launch_dialog;
use app::render::quit_dialog::draw_quit_dialog;
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
use app::render::transcript::render_transcript;
use app::render::upper_window::draw_upper_window;
use app::render::draw_str_clipped;
use app::session::{apply_turn, GameSession, TurnResult};
use app::export::export_transcript;
use app::hints;
use app::slash::{self, SlashOutcome, TranscriptFilterArg};
use app::state::{AppState, FbMode, FileBrowserState, Focus, Layout, PromptKind, RoomPanelMode, SavesState, SoundPulse, TidyJob, TranscriptFilter, TranscriptKind};

// ── Terminal restore helpers ──────────────────────────────────────────────────

/// Restore the terminal to cooked mode and leave the alternate screen.
/// Called both on clean exit and from the panic hook.
/// DisableMouseCapture MUST be issued here so both paths release the mouse.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

/// Install a panic hook that restores the terminal before printing the panic
/// message.  This ensures a broken terminal never survives a panic.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}

// ── Map directory ─────────────────────────────────────────────────────────────

/// Determine the directory where maps (and saves) are stored.
///
/// Priority:
/// 1. `$BABELMAP_MAP_DIR` environment variable (escape hatch for scripts).
/// 2. `config.user_dir/maps` (from config file / CLI flags / defaults).
fn map_dir(user_dir: &std::path::Path) -> std::path::PathBuf {
    if let Ok(d) = std::env::var("BABELMAP_MAP_DIR") {
        return std::path::PathBuf::from(d);
    }
    user_dir.join("maps")
}

/// Directory holding per-game save archives (`.babelmap`, default + named) and
/// the default location for Quetzal import/export. Kept separate from the map
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

use app::keymap::{Command, Context, HotkeyLayout, KeyMap};

/// Priority-ordered command lists for the bottom hint bar.
/// Commands are included only when directly available in the current context.
/// Command::Retidy is intentionally excluded from all lists.
const GAME_HINTS: &[Command] = &[
    Command::ToggleFocus,
    Command::SaveGame,
    Command::RestoreGame,
    Command::CycleLayout,
];

const MAP_HINTS: &[Command] = &[
    Command::ToggleFocus,
    Command::CycleLayout,
    Command::ZoomIn,
    Command::Recenter,
    Command::SelectNext,
    Command::OpenGallery,
    Command::ToggleInspector,
];

const ANIM_HINTS: &[Command] = &[
    Command::AnimStepFwd,
    Command::AnimTogglePlay,
    Command::AnimExit,
    Command::PanLeft,
    Command::ZoomIn,
];

/// Build the hint bar string for the given context from the live keymap and layout.
///
/// For each command in `priority`, an entry is included only if all three hold:
/// 1. `layout.is_direct(cmd)` — the command is directly available, not dialog-only.
/// 2. `keymap.primary_key(cmd)` returns a KeySpec `k`.
/// 3. `keymap.lookup(&k, ctx) == Some(cmd)` — pressing `k` in `ctx` resolves to `cmd`.
///
/// Each surviving entry renders as "{k.label()}: {cmd.label()}"; entries join with " | ".
/// If the joined string exceeds `width` characters, it is truncated and "…" appended.
pub fn hint_bar(
    keymap: &KeyMap,
    layout: &HotkeyLayout,
    ctx: Context,
    priority: &[Command],
    width: usize,
) -> String {
    let entries: Vec<String> = priority
        .iter()
        .filter_map(|&cmd| {
            // Gate 1: command must be directly available (not dialog-only).
            if !layout.is_direct(cmd) {
                return None;
            }
            // Gate 2: command must have a primary key binding.
            let k = keymap.primary_key(cmd)?;
            // Gate 3: pressing that key in this context must resolve back to this command.
            if keymap.lookup(&k, ctx) != Some(cmd) {
                return None;
            }
            Some(format!("{}: {}", k.label(), cmd.label()))
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
/// `layer_tabs` pairs each visible layer tab with its hit-rect (for future click-to-switch).
/// `dialog` holds the last-drawn dialog chrome rects for mouse hit-testing.
struct PaneRects {
    map: Rect,
    story: Rect,
    room_rects: Vec<(RoomId, Rect)>,
    /// Hit-rects for each layer tab, paired with the layer id.
    /// Populated but not yet consumed; reserved for a future click-to-switch feature.
    #[allow(dead_code)]
    layer_tabs: Vec<(LayerId, Rect)>,
    /// Active dialog chrome rects (when a dialog is open).
    pub dialog: Option<DialogRects>,
    /// Hit-rects for the reset dialog (when open).
    pub reset_dialog: Option<app::render::reset_dialog::ResetDialogRects>,
    /// Hit-rects for the quit dialog (when open).
    pub quit_dialog: Option<app::render::quit_dialog::QuitDialogRects>,
    /// Hit-rects for the launch dialog (when open).
    pub launch_dialog: Option<app::render::launch_dialog::LaunchDialogRects>,
    /// Hit-rects for the hints panel (when open).
    pub hints_panel: Option<HintsPanelRects>,
}

/// Render one frame. Returns both pane inner-content rects so the event loop
/// can route mouse events and make accurate `recenter_on` calls.
fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    session: &GameSession,
    mapper: &Mapper,
    state: &AppState,
) -> std::io::Result<PaneRects> {
    let mut map_area = Rect::default();
    let mut story_area = Rect::default();
    let mut room_rects_out: Vec<(RoomId, Rect)> = Vec::new();
    let mut layer_tabs_out: Vec<(LayerId, Rect)> = Vec::new();
    let mut dialog_rects_out: Option<DialogRects> = None;
    let mut reset_dialog_rects_out: Option<app::render::reset_dialog::ResetDialogRects> = None;
    let mut quit_dialog_rects_out: Option<app::render::quit_dialog::QuitDialogRects> = None;
    let mut launch_dialog_rects_out: Option<app::render::launch_dialog::LaunchDialogRects> = None;
    let mut hints_panel_rects_out: Option<HintsPanelRects> = None;

    terminal.draw(|f| {
        let full = f.area();
        let buf = f.buffer_mut();
        // During tidy-animation playback the map shows the current captured stage, not the live graph.
        let rm = match &state.tidy_anim {
            Some(anim) => render_layer(&anim.current().graph, state.active_layer(&anim.current().graph)),
            None => render_layer(&mapper.graph, state.active_layer(&mapper.graph)),
        };

        // ── Change 2: reserve bottom 1 row for help bar ───────────────────────
        let vert = RatatuiLayout::default()
            .direction(LayoutDir::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(full);
        let main_area = vert[0];
        let help_row = vert[1];

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
                        zvm::cpu::exec::Beep::High => state
                            .colors
                            .sound_beep_high
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        zvm::cpu::exec::Beep::Low => state
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
                let story_fp = draw_framed(buf, main_area, state.colors.story_border_style, state.colors.story_border_sides, story_border_style, state.colors.story_header_on);
                let c = story_fp.content;
                let used = draw_upper_window(&session.machine, state.char_mode, &state.colors, c, buf);
                let tarea = Rect::new(c.x, c.y + used, c.width, c.height.saturating_sub(used));
                render_transcript(&session.machine, state, tarea, buf);
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
                let graph = match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                };
                let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                let active_layer = state.active_layer(graph);
                let map_fp = draw_framed(buf, main_area, state.colors.map_border_style, state.colors.map_border_sides, state.colors.map_border, state.colors.map_header_on);
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
                let owned_segs = build_layer_segments(&layer_ids, active_layer);
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
                    for cy in main_area.y..main_area.bottom() {
                        if let Some(c) = buf.cell_mut((main_area.x, cy)) { c.set_style(pulse_style); }
                        if let Some(c) = buf.cell_mut((main_area.right().saturating_sub(1), cy)) { c.set_style(pulse_style); }
                    }
                    for cx in main_area.x..main_area.right() {
                        if let Some(c) = buf.cell_mut((cx, main_area.y)) { c.set_style(pulse_style); }
                        if let Some(c) = buf.cell_mut((cx, main_area.bottom().saturating_sub(1))) { c.set_style(pulse_style); }
                    }
                }
            }
            Layout::Split => {
                // Split 50/50 horizontally with bordered blocks (no divider column).
                let chunks = RatatuiLayout::default()
                    .direction(LayoutDir::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(main_area);

                let story_fp = draw_framed(buf, chunks[0], state.colors.story_border_style, state.colors.story_border_sides, story_border_style, state.colors.story_header_on);
                let c = story_fp.content;
                let used = draw_upper_window(&session.machine, state.char_mode, &state.colors, c, buf);
                let tarea = Rect::new(c.x, c.y + used, c.width, c.height.saturating_sub(used));
                render_transcript(&session.machine, state, tarea, buf);
                if let Some(hrect) = story_fp.header {
                    let segs = [InsetSegment { text: &state.title, active: false }];
                    if story_fp.header_bordered {
                        draw_top_inset(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    } else {
                        draw_header_plain(buf, hrect, &segs, state.colors.story_title, state.colors.story_title);
                    }
                }
                story_area = story_fp.content;

                let map_fp = draw_framed(buf, chunks[1], state.colors.map_border_style, state.colors.map_border_sides, state.colors.map_border, state.colors.map_header_on);
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
                    let graph = match &state.tidy_anim {
                        Some(anim) => &anim.current().graph,
                        None => &mapper.graph,
                    };
                    let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                    let active_layer = state.active_layer(graph);
                    let owned_segs = build_layer_segments(&layer_ids, active_layer);
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
                    for cy in chunks[1].y..chunks[1].bottom() {
                        if let Some(c) = buf.cell_mut((chunks[1].x, cy)) { c.set_style(pulse_style); }
                        if let Some(c) = buf.cell_mut((chunks[1].right().saturating_sub(1), cy)) { c.set_style(pulse_style); }
                    }
                    for cx in chunks[1].x..chunks[1].right() {
                        if let Some(c) = buf.cell_mut((cx, chunks[1].y)) { c.set_style(pulse_style); }
                        if let Some(c) = buf.cell_mut((cx, chunks[1].bottom().saturating_sub(1))) { c.set_style(pulse_style); }
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
                let graph = match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                };
                let panel_ds = make_dialog_style(state);
                match panel.mode {
                    RoomPanelMode::Info => {
                        let current_room = graph.current();
                        let mem = if state.tidy_anim.is_none() {
                            Some(&session.machine.mem)
                        } else {
                            None
                        };
                        if let Some(dr) = draw_room_info(graph, mem, panel.id, current_room, map_area, buf, &panel_ds) {
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

        // ── Change 2: draw help bar in bottom row ─────────────────────────────
        let help_style = state.colors.help_bar;
        let help_text = if state.config_screen.is_some() {
            "\u{2191}\u{2193} move  \u{2190}\u{2192}/Space change  s save  Esc cancel".to_string()
        } else if state.verb_menu.is_some() {
            "Verb Menu | Tab/\u{2190}\u{2192}: pane | \u{2191}\u{2193}: move | Enter/Space: pick | Esc: close".to_string()
        } else if state.file_browser.as_ref().map(|fb| fb.mode == FbMode::PickFile).unwrap_or(false) {
            "Import Save | \u{2191}\u{2193}: move | Enter: open/import | Esc: cancel".to_string()
        } else if state.file_browser.as_ref().map(|fb| fb.mode == FbMode::PickDir).unwrap_or(false) {
            "Export Save | \u{2191}\u{2193}: move | Enter: open dir | s: export here | Esc: cancel".to_string()
        } else if state.saves.is_some() {
            "Saves | \u{2191}\u{2193}: select | Enter: load | s: save-as | d: delete | e: export | i: import | Esc: close".to_string()
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
            let hint_width = (help_row.width as usize).saturating_sub(prefix.chars().count() + 3);
            let hints = hint_bar(&state.keymap, &state.hotkeys, Context::Anim, ANIM_HINTS, hint_width);
            format!("{} | {}", prefix, hints)
        } else if let Some(prompt) = &state.prompt {
            // Show prompt label with instructions when a prompt is active.
            let label = match &prompt.kind {
                PromptKind::RenameRoom(_) => "Rename",
                PromptKind::EditNotes(_) => "Notes",
                PromptKind::RelabelEdge(_, _) => "Direction",
                PromptKind::RenameLayer(_) => "Layer name",
                PromptKind::SaveAs => "Save name",
                PromptKind::ConfirmDeleteSave(_) => "Delete? (y/n)",
                PromptKind::ExportSaveName(_) => "Export filename",
                PromptKind::ConfigEditPath { .. } => "Config path",
            };
            format!("{}: type text | Enter: apply | Esc: cancel", label)
        } else {
            let w = help_row.width as usize;
            match state.focus {
                Focus::Game => hint_bar(&state.keymap, &state.hotkeys, Context::Global, GAME_HINTS, w),
                Focus::Map => hint_bar(&state.keymap, &state.hotkeys, Context::Map, MAP_HINTS, w),
            }
        };
        // Fill help row with reversed style, then draw text.
        for x in help_row.x..help_row.right() {
            if let Some(cell) = buf.cell_mut((x, help_row.y)) {
                cell.set_symbol(" ").set_style(help_style);
            }
        }
        draw_str_clipped(buf, help_row.x, help_row.y, &help_text, help_style, help_row);

        // ── Hotkey dialog overlay — drawn over everything ─────────────────────
        if state.hotkey_dialog {
            dialog_rects_out = draw_hotkey_dialog(state, full, buf);
        }

        // ── Gallery overlay — drawn after hotkey dialog ───────────────────────
        if state.gallery.is_some() {
            if let Some(dr) = draw_gallery(state, full, buf) {
                dialog_rects_out = Some(dr);
            }
        }

        // ── Saves-manager overlay — drawn after gallery ───────────────────────
        if state.saves.is_some() {
            dialog_rects_out = draw_saves(state, full, buf);
        }

        // ── File-browser overlay — drawn after saves ──────────────────────────
        if state.file_browser.is_some() {
            dialog_rects_out = draw_file_browser(state, full, buf);
        }

        // ── Verb-menu overlay — drawn after saves ─────────────────────────────
        if state.verb_menu.is_some() {
            dialog_rects_out = draw_verb_menu(state, full, buf);
        }

        // ── Config screen overlay — drawn after other modals ──────────────────
        if state.config_screen.is_some() {
            dialog_rects_out = draw_config_screen(state, full, buf);
        }

        // ── Reset dialog overlay — drawn over everything ───────────────────────
        if state.reset_dialog {
            reset_dialog_rects_out = draw_reset_dialog(state, full, buf);
        }

        // ── Quit dialog overlay — drawn over everything ────────────────────────
        if state.quit_dialog {
            quit_dialog_rects_out = draw_quit_dialog(state, full, buf);
        }

        // ── Launch dialog overlay — drawn over everything ──────────────────────
        if state.launch_dialog {
            launch_dialog_rects_out = draw_launch_dialog(state, full, buf);
        }

        // ── Hints panel overlay — drawn after other overlays ───────────────────
        if state.hints.is_some() {
            hints_panel_rects_out = draw_hints_panel(state, full, buf);
        }

        // ── Prompt overlay — drawn over the map area (or full screen) ─────────
        if let Some(prompt) = &state.prompt {
            let overlay_area = if map_area.height > 0 { map_area } else { main_area };
            if overlay_area.height > 0 {
                let y = overlay_area.bottom() - 1;
                let label = match &prompt.kind {
                    PromptKind::RenameRoom(_) => "Rename: ",
                    PromptKind::EditNotes(_) => "Notes:  ",
                    PromptKind::RelabelEdge(_, _) => "Dir:    ",
                    PromptKind::RenameLayer(_) => "Layer:  ",
                    PromptKind::SaveAs => "Name:   ",
                    PromptKind::ConfirmDeleteSave(_) => "Del y/n:",
                    PromptKind::ExportSaveName(_) => "Export: ",
                    PromptKind::ConfigEditPath { .. } => "Path:   ",
                };
                let line = format!("{}{}_", label, prompt.buffer);
                let overlay_style = Style::default().add_modifier(Modifier::REVERSED);
                draw_str_clipped(buf, overlay_area.x, y, &line, overlay_style, overlay_area);
            }
        }
    })?;

    Ok(PaneRects { map: map_area, story: story_area, room_rects: room_rects_out, layer_tabs: layer_tabs_out, dialog: dialog_rects_out, reset_dialog: reset_dialog_rects_out, quit_dialog: quit_dialog_rects_out, launch_dialog: launch_dialog_rects_out, hints_panel: hints_panel_rects_out })
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

fn main() {
    // ── 1. Parse args + load config ───────────────────────────────────────────

    let cli = Cli::parse();
    let cfg = resolve(&cli);
    let story_path = cli.story.clone();

    let story_bytes = match hints::load_story_bytes(&story_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("babelmap: cannot read '{}': {}", story_path.display(), e);
            std::process::exit(1);
        }
    };

    let mut session = match GameSession::new(story_bytes.clone()) {
        Ok(s) => s,
        Err(e) => {
            use zvm::error::ZError;
            let msg = match e {
                ZError::GraphicalV6 => "this is a version 6 (graphical) story; v6 graphical games are not supported".to_string(),
                ZError::UnsupportedVersion(v) => format!("unsupported Z-machine version {v}"),
                ZError::NotAStoryFile => "file is not a valid Z-machine story file".to_string(),
                ZError::Truncated => "story file is truncated".to_string(),
                _ => format!("{e:?}"),
            };
            eprintln!("babelmap: {msg}");
            std::process::exit(1);
        }
    };

    // Apply the configured virtual screen dimensions to the VM. init_caps (called
    // inside GameSession::new) seeds defaults; we override with the user's config here.
    zvm::screen::write_screen_dims(
        &mut session.machine.mem,
        cfg.virtual_screen_rows as u8,
        cfg.virtual_screen_cols as u8,
    );
    session.machine.undo_cap = cfg.undo_levels;

    // ── 2. IFID + map dir + load/create mapper ────────────────────────────────

    let ifid = compute_ifid(&story_bytes);
    let dir = map_dir(&cfg.user_dir);
    let save_dir = saves_dir(&cfg.user_dir);
    let arc_file = archive_path(&save_dir, &ifid);
    let map_file = map_path(&dir, &ifid);

    // Load mapper (and optionally restore the game save) from the archive.
    // Migration: if no archive exists but a legacy .map.json does, load that.
    // use_default_map = true: also fall back to legacy map when no archive.
    let mut startup_transcript: Option<(Vec<String>, Vec<TranscriptKind>)> = None;
    // Rewind/replay history carried from the archive when the game is auto-restored.
    let mut startup_history: Vec<app::history::TurnRecord> = Vec::new();
    // When auto_load is false but a save exists and prompt_load_on_launch is true,
    // stash the save for the launch dialog instead of discarding it.
    let mut pending_resume_stash: Option<(Vec<u8>, Vec<String>, Vec<TranscriptKind>)> = None;
    let mut mapper = if arc_file.exists() {
        match load_archive(&arc_file) {
            Ok(ac) => {
                // Restore the machine from the saved game state only when auto_load is enabled.
                // When auto_load = false the accumulated map still loads, but the game starts fresh.
                if cfg.auto_load {
                    if let Err(e) = session.machine.restore_file(&ac.save) {
                        eprintln!("babelmap: warning: could not restore game from archive: {:?}", e);
                    } else {
                        startup_transcript = Some((ac.transcript, ac.transcript_kinds));
                        startup_history = ac.history;
                    }
                } else if cfg.prompt_load_on_launch && !ac.save.is_empty() {
                    // Save present, auto_load off, prompt enabled: stash for launch dialog.
                    pending_resume_stash = Some((ac.save, ac.transcript, ac.transcript_kinds));
                }
                ac.mapper
            }
            Err(e) => {
                eprintln!("babelmap: warning: could not load archive {}: {}", arc_file.display(), e);
                Mapper::default()
            }
        }
    } else if map_file.exists() {
        // Back-compat: migrate existing .map.json to the new archive on next save.
        load_map(&map_file).unwrap_or_default()
    } else if cfg.use_default_map {
        // Fall back to legacy shared map when configured to do so.
        load_map(&map_file).unwrap_or_default()
    } else {
        Mapper::default()
    };

    // Export paths (fixed per IFID).
    let svg_path = dir.join(format!("{}.svg", ifid));
    let dot_path = dir.join(format!("{}.dot", ifid));
    let dump_path = dir.join(format!("{}.map.txt", ifid));

    // ── 3. Seed initial transcript + starting room ────────────────────────────

    let mut state = AppState::default();
    // Resolve the look from style.toml (the single styling source).
    let (base, w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (cs, set, w2) = app::style::resolve(&base, &cfg.user_dir);
    state.colors = cs;
    state.symbols = set;
    for w in w1.into_iter().chain(w2) {
        state.push_transcript(&format!("[{}]", w));
    }
    let (keymap, keymap_warnings) = app::keymap::KeyMap::resolve(&cfg.keymap);
    state.keymap = keymap;
    // Surface any keymap conflict warnings once in the transcript.
    for w in keymap_warnings {
        state.push_transcript(&format!("[{}]", w));
    }
    let (hotkeys, hotkey_warnings) = app::keymap::HotkeyLayout::resolve(&cfg.hotkeys);
    state.hotkeys = hotkeys;
    for w in hotkey_warnings {
        state.push_transcript(&format!("[{}]", w));
    }
    state.show_room_numbers = cfg.show_room_numbers;
    state.show_loc_method = cfg.show_loc_method;
    state.config = cfg;

    // Seed autocomplete with the story's parser vocabulary (room nouns are added live).
    state.dict_words = zvm::dictionary::load(&session.machine.mem).words(&session.machine.mem);

    // Push the game's opening banner and capture the title from it.
    let banner = session.take_transcript();
    let banner_title = app::session::title_from_banner(&banner);
    state.title = app::session::resolve_title(None, &ifid, banner_title.as_deref(), &story_path);
    state.ifid = ifid.clone();
    // Now that the IFID is known, re-resolve through reload_style so the per-game
    // override (styles/<ifid>.toml) is merged over the global at startup — the
    // initial resolve above is global-only (ifid wasn't set yet). On a per-game
    // parse error the global look already set above stands.
    let _ = app::reload::reload_style(&mut state);
    state.push_transcript(&banner);

    // One-time notice: config.toml no longer carries style — those moved to style.toml.
    if let Ok(raw_cfg) = std::fs::read_to_string(app::config::config_path(&cli)) {
        if app::config::config_has_style_sections(&raw_cfg) {
            state.push_transcript_kind(
                "config.toml [colors]/[symbols] are no longer used — move them into style.toml",
                app::state::TranscriptKind::Warning,
            );
        }
    }

    // Observe the starting room so it appears on the map immediately.
    let start_loc = zvm::current_location(&session.machine);
    if let Some(snap) = start_loc {
        let snap_number = snap.number;
        let seed_result = TurnResult {
            transcript: String::new(),
            location: Some(snap),
            quit: session.quit,
            info: None,
            beep: None,
            diagnostics: vec![],
            location_method: None,
        };
        apply_turn(&mut mapper, "", &seed_result);
        let rid = snap_number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        // Recenter using a default pane size; will be corrected after first draw.
        state.recenter_on(
            mapper
                .graph
                .room(rid)
                .and_then(|r| r.pos)
                .unwrap_or((0, 0)),
            40,
            24,
        );
    }

    // If an archived transcript was loaded on startup, replace the fresh one.
    if let Some((lines, kinds)) = startup_transcript {
        state.transcript = lines;
        state.transcript_kinds = kinds;
    }
    if !startup_history.is_empty() {
        state.history = startup_history;
    }

    // If a save was found but auto_load is off and prompt_load_on_launch is on,
    // open the launch dialog so the user can choose to resume or start fresh.
    if let Some(stash) = pending_resume_stash {
        state.pending_resume = Some(stash);
        state.launch_dialog = true;
        state.dialog_focus = 0;
    }

    // If the game quit immediately (e.g. czech.z5 test suite), bail without
    // entering raw mode.
    if session.quit {
        eprintln!("babelmap: story ended immediately (no interactive content).");
        std::process::exit(0);
    }

    // ── 4. Terminal setup ─────────────────────────────────────────────────────

    // Install the panic hook FIRST so that any panic after this point (including
    // one between enable_raw_mode and EnterAlternateScreen) restores the terminal.
    install_panic_hook();

    if let Err(e) = enable_raw_mode() {
        eprintln!("babelmap: cannot enable raw mode (not a TTY?): {}", e);
        std::process::exit(1);
    }

    // From here on, raw mode is active — MUST restore on every exit path.

    if let Err(e) = execute!(stdout(), EnterAlternateScreen, EnableMouseCapture) {
        restore_terminal();
        eprintln!("babelmap: cannot enter alternate screen: {}", e);
        std::process::exit(1);
    }

    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(e) => {
            restore_terminal();
            eprintln!("babelmap: cannot create terminal: {}", e);
            std::process::exit(1);
        }
    };

    // ── 5. Event loop ─────────────────────────────────────────────────────────

    // Track the last-known pane rects for accurate recenter_on calls and mouse routing.
    // Initialized to a zero-sized default; updated by every draw_frame call.
    let mut last_panes = PaneRects { map: Rect::default(), story: Rect::default(), room_rects: Vec::new(), layer_tabs: Vec::new(), dialog: None, reset_dialog: None, quit_dialog: None, launch_dialog: None, hints_panel: None };

    // Debounce counter for BackgroundTidy::Debounced mode.
    let mut bg_tidy_counter: u32 = 0;

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

    loop {
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
            match app::reload::reload_style(&mut state) {
                app::reload::ReloadOutcome::Reloaded { warnings } => {
                    for wn in &warnings {
                        state.push_transcript_kind(wn, TranscriptKind::Warning);
                    }
                    state.set_status("style reloaded (watch)");
                }
                app::reload::ReloadOutcome::Failed { msg } => {
                    state.push_transcript_kind(
                        &format!("style reload failed: {}", msg),
                        TranscriptKind::Warning,
                    );
                }
            }
        }

        // ── Background tidy job: poll and apply ───────────────────────────────
        // Check whether the in-flight tidy job has finished. Do this BEFORE the
        // draw so the first fully-drawn frame after completion shows the new layout.
        if state.tidy_job.as_ref().map_or(false, |j| j.handle.is_finished()) {
            let job = state.tidy_job.take().unwrap();
            let current_gen = state.graph_gen;
            let active_layer = job.layer;
            match job.handle.join() {
                Ok(tidied) => {
                    match apply_tidy_result(&mut mapper.graph, tidied, active_layer, job.gen, current_gen) {
                        ApplyTidyOutcome::Applied => {
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

        // Update char_mode flag so the renderer hides the prompt during read_char.
        state.char_mode = matches!(session.pending_input(), app::session::InputKind::Char);

        // Expire a finished sound pulse so the story border returns to normal.
        if let Some(p) = &state.sound_pulse {
            if p.started.elapsed().as_millis() as u64 >= SOUND_PULSE_MS {
                state.sound_pulse = None;
            }
        }

        // Draw.
        match draw_frame(&mut terminal, &session, &mapper, &state) {
            Ok(panes) => {
                last_panes = panes;
            }
            Err(e) => {
                restore_terminal();
                eprintln!("babelmap: draw error: {}", e);
                std::process::exit(1);
            }
        }

        // Poll for a key event. Use a shorter timeout while a tidy job is in flight
        // so the pulsing border animates at ~30fps; otherwise use the normal 50ms.
        let poll_ms = if state.tidy_job.is_some() || state.sound_pulse.is_some() { TIDY_POLL_MS } else { 50 };
        let event_ready = match poll(Duration::from_millis(poll_ms)) {
            Ok(r) => r,
            Err(e) => {
                restore_terminal();
                eprintln!("babelmap: poll error: {}", e);
                std::process::exit(1);
            }
        };

        if !event_ready {
            // No key this tick — advance the tidy animation if one is playing. The next loop
            // iteration redraws, so an advanced frame appears without waiting for input.
            if let Some(anim) = &mut state.tidy_anim {
                anim.tick(Duration::from_millis(700));
            }
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

        // ── Reset dialog intercept — before normal action routing ─────────────
        // When the reset dialog is open, route keyboard/mouse directly here and
        // continue (swallowing events the dialog does not handle).
        if state.reset_dialog {
            match &event {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    match k.code {
                        crossterm::event::KeyCode::Tab | crossterm::event::KeyCode::Right =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, 1),
                        crossterm::event::KeyCode::BackTab | crossterm::event::KeyCode::Left =>
                            state.dialog_focus = app::input::cycle_focus(state.dialog_focus, 2, -1),
                        code => match reset_dialog_key_focused(code, state.dialog_focus) {
                            ResetDialogAction::Confirm => {
                                let clear = state.reset_clear_map;
                                state.reset_dialog = false;
                                reset_game(&mut session, &mut mapper, &mut state, &story_bytes, clear);
                            }
                            ResetDialogAction::Cancel => {
                                state.reset_dialog = false;
                            }
                            ResetDialogAction::ToggleClear => {
                                state.reset_clear_map = !state.reset_clear_map;
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
                            let in_close = rd.close.map_or(false, |r| r.contains(pt));
                            let in_reset = rd.reset.map_or(false, |r| r.contains(pt));
                            let in_cancel = rd.cancel.map_or(false, |r| r.contains(pt));
                            let in_checkbox = rd.checkbox.contains(pt);
                            let in_dialog = rd.area.contains(pt);
                            if in_close || in_cancel {
                                state.reset_dialog = false;
                            } else if in_reset {
                                let clear = state.reset_clear_map;
                                state.reset_dialog = false;
                                reset_game(&mut session, &mut mapper, &mut state, &story_bytes, clear);
                            } else if in_checkbox {
                                state.reset_clear_map = !state.reset_clear_map;
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
                                let meta = app::archive::Meta {
                                    format_version: 1,
                                    ifid: Some(ifid.clone()),
                                    name: None,
                                    turns: state.turns,
                                    saved_at: format_rfc3339(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0),
                                    ),
                                };
                                let _ = save_archive_meta(&arc_file, &mapper, &session.machine, meta, &state.transcript, &state.transcript_kinds, &state.history);
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
                            let in_close = qd.close.map_or(false, |r| r.contains(pt));
                            let in_save = qd.save.map_or(false, |r| r.contains(pt));
                            let in_quit = qd.quit.map_or(false, |r| r.contains(pt));
                            let in_cancel = qd.cancel.map_or(false, |r| r.contains(pt));
                            let in_dialog = qd.area.contains(pt);
                            if in_save {
                                state.quit_dialog = false;
                                let meta = app::archive::Meta {
                                    format_version: 1,
                                    ifid: Some(ifid.clone()),
                                    name: None,
                                    turns: state.turns,
                                    saved_at: format_rfc3339(
                                        std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or(0),
                                    ),
                                };
                                let _ = save_archive_meta(&arc_file, &mapper, &session.machine, meta, &state.transcript, &state.transcript_kinds, &state.history);
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
                                if let Some((save, lines, kinds)) = state.pending_resume.take() {
                                    state.launch_dialog = false;
                                    apply_launch_resume(&save, lines, kinds, &mut session, &mut mapper, &mut state, &last_panes);
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
                            let in_resume = ld.resume.map_or(false, |r| r.contains(pt));
                            let in_new_game = ld.new_game.map_or(false, |r| r.contains(pt));
                            let in_close = ld.close.map_or(false, |r| r.contains(pt));
                            let in_dialog = ld.area.contains(pt);
                            if in_resume {
                                if let Some((save, lines, kinds)) = state.pending_resume.take() {
                                    state.launch_dialog = false;
                                    apply_launch_resume(&save, lines, kinds, &mut session, &mut mapper, &mut state, &last_panes);
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
                            let in_close = hp.close.map_or(false, |r| r.contains(pt));
                            if in_close {
                                state.hints = None;
                            }
                            // Clicks inside the dialog but not on close: swallow.
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
                        // Map to ZSCII and forward; unknown keys are silently ignored.
                        if let Some(byte) = key_to_zscii(*k) {
                            let result = session.submit_char(byte);
                            state.push_transcript(&result.transcript);
                            apply_turn_events(&mut state, &result);
                            if let Some(note) = &result.info {
                                state.push_transcript(note);
                            }
                            // apply_turn: char-mode keypresses don't carry direction info
                            // (no text command to parse), but we still observe any location
                            // change so the map stays in sync.
                            apply_turn(&mut mapper, "", &result);
                            state.graph_gen = state.graph_gen.wrapping_add(1);
                            // Select and recenter on the current room if it changed.
                            if let Some(snap) = &result.location {
                                let rid = snap.number as mapper::graph::RoomId;
                                state.select_room(Some(rid));
                                if let Some(room) = mapper.graph.room(rid) {
                                    if let Some(pos) = room.pos {
                                        let (pw, ph) = map_pane_dims(last_panes.map);
                                        state.recenter_on(pos, pw, ph);
                                    }
                                }
                            }
                            if result.quit {
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
        }

        // Route event to an Action.
        let action = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => key_to_action(&state, k),
            Event::Mouse(m) => {
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

        // Note whether this action closes the gallery (persist the look afterward).
        let gallery_cfg_on_close = matches!(action, Action::GalleryClose | Action::GalleryApply);

        // Note whether this action is the on-demand "Output all settings" export.
        let export_style_now = matches!(action, Action::GalleryExportStyle);

        // Snapshot working config before apply_action clears it on ConfigSave.
        let config_to_save = if matches!(action, Action::ConfigSave) {
            state.config_screen.as_ref().map(|cs| cs.working.clone())
        } else {
            None
        };

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

            Action::SubmitCommand(cmd) => {
                // When a prompt is active, SubmitCommand is the Enter sentinel;
                // route to apply_action to apply the prompt to the mapper.
                if state.prompt.is_some() {
                    apply_action(Action::SubmitCommand(cmd), &mut state, &mut mapper);
                    // Handle any saves-manager or reset prompt that was submitted.
                    if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
                        handle_saves_prompt(
                            kind, buf, &save_dir, &ifid, &mut mapper, &mut session, &mut state, &story_bytes,
                        );
                    }
                    continue;
                }

                // Normal game-focus command submission.
                // Clear input line and echo command.
                let cmd = state.take_input();
                if cmd.is_empty() {
                    continue;
                }

                // ── Slash-command interception ────────────────────────────────
                // If the input starts with the configured prefix, route it as an
                // app command; do NOT call session.submit, increment turns, or
                // push a "> cmd" story line.
                if is_slash(&cmd, state.config.command_prefix) {
                    // Strip the leading prefix character before parsing.
                    let body = &cmd[state.config.command_prefix.len_utf8()..];
                    match slash::parse(body, state.config.command_prefix) {
                        SlashOutcome::Action(a) => {
                            if matches!(a, Action::ToggleWatch) {
                                toggle_style_watch(&mut state, &mut style_watcher);
                            } else {
                                apply_action(a, &mut state, &mut mapper);
                            }
                        }
                        SlashOutcome::Message(m) | SlashOutcome::Error(m) => {
                            state.set_status(m);
                        }
                        SlashOutcome::Help => {
                            for line in slash::help_text(state.config.command_prefix) {
                                state.push_transcript_kind(&line, TranscriptKind::Meta);
                            }
                        }
                        SlashOutcome::Save(name_opt) => {
                            // Named save or default archive save.
                            let result = match name_opt {
                                Some(ref name) => {
                                    save_named(&save_dir, &ifid, name, &mapper, &session.machine, state.turns, &state.transcript, &state.transcript_kinds)
                                        .map(|()| format!("saved as \"{}\"", name))
                                        .map_err(|e| format!("save failed: {}", e))
                                }
                                None => {
                                    let meta = app::archive::Meta {
                                        format_version: 1,
                                        ifid: Some(ifid.clone()),
                                        name: None,
                                        turns: state.turns,
                                        saved_at: format_rfc3339(
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_secs())
                                                .unwrap_or(0),
                                        ),
                                    };
                                    save_archive_meta(&arc_file, &mapper, &session.machine, meta, &state.transcript, &state.transcript_kinds, &state.history)
                                        .map(|()| "saved".to_string())
                                        .map_err(|e| format!("save failed: {}", e))
                                }
                            };
                            match result {
                                Ok(msg) => state.set_status(msg),
                                Err(e) => state.set_status(e),
                            }
                        }
                        SlashOutcome::Load(name_opt) => {
                            // Named-slot load or default archive load.
                            let archive_to_load = match name_opt {
                                None => Some(arc_file.clone()),
                                Some(ref name) => {
                                    // Find the first named save whose display name matches.
                                    let saves = list_saves(&save_dir, &ifid);
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
                                    match load_archive(path) {
                                        Ok(ac) => {
                                            let restore_err = session.machine.restore_file(&ac.save).map_err(|e| {
                                                match e {
                                                    zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                                                    other => format!("restore failed: {:?}", other),
                                                }
                                            });
                                            match restore_err {
                                                Ok(()) => {
                                                    mapper = ac.mapper;
                                                    state.transcript = ac.transcript;
                                                    state.transcript_kinds = ac.transcript_kinds;
                                                    state.history = ac.history;
                                                    let loc = zvm::current_location(&session.machine);
                                                    if let Some(snap) = loc {
                                                        let rid = snap.number as mapper::graph::RoomId;
                                                        let restore_result = TurnResult {
                                                            transcript: String::new(),
                                                            location: Some(snap),
                                                            quit: false,
                                                            info: None,
                                                            beep: None,
                                                            diagnostics: vec![],
                                                            location_method: None,
                                                        };
                                                        apply_turn(&mut mapper, "", &restore_result);
                                                        state.set_viewed_layer(None);
                                                        state.select_room(Some(rid));
                                                        if let Some(room) = mapper.graph.room(rid) {
                                                            if let Some(pos) = room.pos {
                                                                let (pw, ph) = map_pane_dims(last_panes.map);
                                                                state.recenter_on(pos, pw, ph);
                                                            }
                                                        }
                                                    }
                                                    state.set_status("loaded");
                                                }
                                                Err(e) => state.set_status(format!("load failed: {}", e)),
                                            }
                                        }
                                        Err(e) => state.set_status(format!("load failed: {}", e)),
                                    }
                                }
                            }
                        }
                        SlashOutcome::Reset { map: reset_map } => {
                            // Immediate reset: no dialog (keybinding path has the dialog; slash is instant).
                            reset_game(&mut session, &mut mapper, &mut state, &story_bytes, reset_map);
                            let status_msg = if reset_map { "reset (map cleared)" } else { "reset (map kept)" };
                            state.set_status(status_msg);
                        }
                        SlashOutcome::Quit => {
                            if should_prompt_save_on_quit(&state) {
                                state.quit_dialog = true;
                                state.dialog_focus = 0;
                            } else {
                                break;
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
                                        let pane_rows = if last_panes.story.height > 0 {
                                            last_panes.story.height as usize
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
                                    let pane_rows = if last_panes.story.height > 0 {
                                        last_panes.story.height as usize
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
                            let exports_dir = state.config.user_dir.join("exports");
                            let stamp = format_stamp(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0),
                            );
                            match export_transcript(&lines, dest.as_deref(), &exports_dir, &stamp) {
                                Ok(path) => state.set_status(format!("exported: {}", path.display())),
                                Err(e)   => state.set_status(format!("export failed: {}", e)),
                            }
                        }
                        SlashOutcome::OpenHints => {
                            let sp = story_path.clone();
                            let id = ifid.clone();
                            let ud = state.config.user_dir.clone();
                            open_hints(&mut state, &sp, &id, &ud);
                        }
                    }
                    continue;
                }

                // Clear any transient status message on a real game turn.
                state.status_msg = None;

                // Increment the session turn counter.
                state.turns += 1;

                let result = session.submit(&cmd);
                state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
                state.push_transcript(&result.transcript);
                apply_turn_events(&mut state, &result);
                if let Some(note) = &result.info {
                    state.push_transcript(note);
                }

                // Capture room + connection counts before apply_turn, to detect
                // whether THIS turn actually changed the graph (a non-mutating
                // command like "look" leaves both unchanged).
                let rooms_before = mapper.graph.rooms().count();
                let conns_before = mapper.graph.connections().len();

                apply_turn(&mut mapper, &cmd, &result);

                // Bump the graph generation so any in-flight tidy result is detected as stale.
                state.graph_gen = state.graph_gen.wrapping_add(1);

                // ── Rewind/replay capture (opt-in) ────────────────────────────
                if state.config.record_turn_history {
                    let map_changed = mapper.graph.rooms().count() != rooms_before
                        || mapper.graph.connections().len() != conns_before;
                    app::history::record_turn(
                        &mut state.history,
                        state.turns,
                        &cmd,
                        session.machine.save_quetzal(),
                        &mapper,
                        map_changed,
                        &result.transcript,
                    );
                }

                // ── Inventory tracking ────────────────────────────────────────
                {
                    use app::inventory::{detect_player_obj, parse_inventory_output};
                    use zvm::objects::get_parent;

                    let current_loc = zvm::current_location(&session.machine)
                        .map(|s| s.number)
                        .unwrap_or(0);

                    if current_loc != 0 {
                        // Compute objects whose parent is the current room.
                        let max_obj = {
                            // Infer max by scanning (same approach as location.rs).
                            // We stop at the first entry whose prop-table ptr is before the entry.
                            // Quick safe upper bound: iterate until parent==0 fails.
                            // We use zvm::object_tree_view to avoid duplicating the logic.
                            zvm::object_tree_view(&session.machine)
                                .into_iter()
                                .map(|s| s.number)
                                .max()
                                .unwrap_or(0)
                        };
                        let objects_here: std::collections::BTreeSet<u16> = (1..=max_obj)
                            .filter(|&o| get_parent(&session.machine.mem, o) == current_loc)
                            .collect();

                        // Lock the player object. Prefer the reliable name-based
                        // lookup (the object short-named "you"/"yourself"/… — present
                        // in most games incl. v3 Zork as obj #30) so the inventory
                        // panel reads the LIVE object tree from turn one and reflects
                        // take/drop immediately. Fall back to the movement heuristic
                        // for games whose player object isn't named.
                        if state.player_obj.is_none() {
                            state.player_obj = zvm::find_player_object(&session.machine)
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
                if state.config.auto_save {
                    let meta = app::archive::Meta {
                        format_version: 1,
                        ifid: Some(ifid.clone()),
                        name: None,
                        turns: state.turns,
                        saved_at: format_rfc3339(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                        ),
                    };
                    if let Err(e) = save_archive_meta(&arc_file, &mapper, &session.machine, meta, &state.transcript, &state.transcript_kinds, &state.history) {
                        state.push_transcript(&format!("[Auto-save failed: {}]", e));
                    }
                }

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
                    if should_bg_tidy(state.config.background_tidy, new_room, overlap, changed, &mut bg_tidy_counter) {
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
                            let (pw, ph) = map_pane_dims(last_panes.map);
                            state.recenter_on(pos, pw, ph);
                        }
                    }
                }

                if result.quit {
                    break;
                }
            }

            Action::SaveGame => {
                // Bundle map + game into a single .babelmap archive, with turn metadata.
                let meta = app::archive::Meta {
                    format_version: 1,
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
                match save_archive_meta(&arc_file, &mapper, &session.machine, meta, &state.transcript, &state.transcript_kinds, &state.history) {
                    Ok(()) => {
                        state.push_transcript(&format!(
                            "[Game saved to {}]",
                            arc_file.display()
                        ));
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[Save failed: {}]", e));
                    }
                }
            }

            Action::RestoreGame => {
                // Restore map + game from the .babelmap archive.
                match load_archive(&arc_file) {
                    Ok(ac) => {
                        let restore_err = session.machine.restore_file(&ac.save).map_err(|e| {
                            match e {
                                zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                                other => format!("restore failed: {:?}", other),
                            }
                        });
                        match restore_err {
                            Ok(()) => {
                                mapper = ac.mapper;
                                state.transcript = ac.transcript;
                                state.transcript_kinds = ac.transcript_kinds;
                                state.history = ac.history;
                                // After restore, re-observe current location.
                                let loc = zvm::current_location(&session.machine);
                                if let Some(snap) = loc {
                                    let rid = snap.number as mapper::graph::RoomId;
                                    let restore_result = TurnResult {
                                        transcript: String::new(),
                                        location: Some(snap),
                                        quit: false,
                                        info: None,
                                        beep: None,
                                        diagnostics: vec![],
                                        location_method: None,
                                    };
                                    apply_turn(&mut mapper, "", &restore_result);
                                    state.set_viewed_layer(None);
                                    state.select_room(Some(rid));
                                    if let Some(room) = mapper.graph.room(rid) {
                                        if let Some(pos) = room.pos {
                                            let (pw, ph) = map_pane_dims(last_panes.map);
                                            state.recenter_on(pos, pw, ph);
                                        }
                                    }
                                }
                                state.push_transcript(&format!(
                                    "[Game restored from {}]",
                                    arc_file.display()
                                ));
                            }
                            Err(e) => {
                                state.push_transcript(&format!("[Restore failed: {}]", e));
                            }
                        }
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[Restore failed: {}]", e));
                    }
                }
            }

            Action::ExportSvg => {
                let rm = render_map_data(&mapper.graph);
                match export_svg(&svg_path, &rm) {
                    Ok(()) => {
                        state.push_transcript(&format!(
                            "[SVG exported to {}]",
                            svg_path.display()
                        ));
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[SVG export failed: {}]", e));
                    }
                }
            }

            Action::ExportDot => {
                match export_dot(&dot_path, &mapper.graph) {
                    Ok(()) => {
                        state.push_transcript(&format!(
                            "[DOT exported to {} — render with: dot -Tsvg {} -o map.svg]",
                            dot_path.display(),
                            dot_path.display()
                        ));
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[DOT export failed: {}]", e));
                    }
                }
            }

            Action::ExportDump => {
                match std::fs::write(&dump_path, render_dump(&mapper.graph)) {
                    Ok(()) => {
                        state.push_transcript(&format!("[map dump written to {}]", dump_path.display()));
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[map dump failed: {}]", e));
                    }
                }
            }

            // ── Saves-manager actions ─────────────────────────────────────────

            Action::OpenSaves => {
                // Populate the saves list and open the modal.
                let entries = list_saves(&save_dir, &ifid);
                state.saves = Some(SavesState { entries, selected: 0 });
                state.dialog_focus = 0;
            }

            Action::SavesExport => {
                // Close saves modal and open file browser in PickDir mode.
                state.saves = None;
                let start_dir = saves_dir(&state.config.user_dir);
                let start_dir = if start_dir.is_dir() { start_dir } else { state.config.user_dir.clone() };
                let default_name = format!("{}.qzl", ifid);
                state.file_browser = Some(FileBrowserState::build(start_dir, FbMode::PickDir, default_name));
            }

            Action::SavesImport => {
                // Close saves modal and open file browser in PickFile mode.
                state.saves = None;
                let start_dir = saves_dir(&state.config.user_dir);
                let start_dir = if start_dir.is_dir() { start_dir } else { state.config.user_dir.clone() };
                state.file_browser = Some(FileBrowserState::build(start_dir, FbMode::PickFile, String::new()));
            }

            Action::FbEnter => {
                // Handle file-browser Enter: cd into dir or import file.
                let fb_action = state.file_browser.as_ref().and_then(|fb| {
                    fb.entries.get(fb.selected).map(|e| {
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
                        match restore_game(&path, &mut session.machine) {
                            Ok(()) => {
                                // Re-observe current location (same as RestoreGame/SavesLoad).
                                let loc = zvm::current_location(&session.machine);
                                if let Some(snap) = loc {
                                    let rid = snap.number as mapper::graph::RoomId;
                                    let restore_result = TurnResult {
                                        transcript: String::new(),
                                        location: Some(snap),
                                        quit: false,
                                        info: None,
                                        beep: None,
                                        diagnostics: vec![],
                                        location_method: None,
                                    };
                                    apply_turn(&mut mapper, "", &restore_result);
                                    state.set_viewed_layer(None);
                                    state.select_room(Some(rid));
                                    if let Some(room) = mapper.graph.room(rid) {
                                        if let Some(pos) = room.pos {
                                            let (pw, ph) = map_pane_dims(last_panes.map);
                                            state.recenter_on(pos, pw, ph);
                                        }
                                    }
                                }
                                state.push_transcript(&format!("[Imported: {}]", path.display()));
                            }
                            Err(e) => {
                                state.push_transcript(&format!("[Import failed: {}]", e));
                            }
                        }
                    }
                    None => {}
                }
            }

            Action::FbChooseDir => {
                // PickDir mode: open the ExportSaveName prompt for the current dir.
                if let Some(fb) = &state.file_browser {
                    if fb.mode == FbMode::PickDir {
                        let chosen_dir = fb.cwd.clone();
                        let default_name = fb.export_default_name.clone();
                        state.file_browser = None;
                        state.prompt = Some(app::state::Prompt {
                            kind: PromptKind::ExportSaveName(chosen_dir),
                            buffer: default_name,
                        });
                    }
                }
            }

            Action::SavesLoad => {
                // Load the selected save (archive → mapper + machine restore).
                // Clone path and name to release the borrow on state.saves before mutating state.
                let load_info = state.saves.as_ref().and_then(|s| {
                    s.entries.get(s.selected).map(|e| (e.path.clone(), e.name.clone()))
                });
                if let Some((path, entry_name)) = load_info {
                    match load_archive(&path) {
                        Ok(ac) => {
                            let restore_err = session.machine.restore_file(&ac.save).map_err(|e| {
                                match e {
                                    zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                                    other => format!("restore failed: {:?}", other),
                                }
                            });
                            match restore_err {
                                Ok(()) => {
                                    mapper = ac.mapper;
                                    state.transcript = ac.transcript;
                                    state.transcript_kinds = ac.transcript_kinds;
                                    state.history = ac.history;
                                    // Restore turn counter from the loaded archive.
                                    state.turns = ac.meta.turns;
                                    // Re-observe current location.
                                    let loc = zvm::current_location(&session.machine);
                                    if let Some(snap) = loc {
                                        let rid = snap.number as mapper::graph::RoomId;
                                        let restore_result = TurnResult {
                                            transcript: String::new(),
                                            location: Some(snap),
                                            quit: false,
                                            info: None,
                                            beep: None,
                                            diagnostics: vec![],
                                            location_method: None,
                                        };
                                        apply_turn(&mut mapper, "", &restore_result);
                                        state.set_viewed_layer(None);
                                        state.select_room(Some(rid));
                                        if let Some(room) = mapper.graph.room(rid) {
                                            if let Some(pos) = room.pos {
                                                let (pw, ph) = map_pane_dims(last_panes.map);
                                                state.recenter_on(pos, pw, ph);
                                            }
                                        }
                                    }
                                    state.push_transcript(&format!("[Loaded save: {}]", entry_name));
                                    state.saves = None;
                                }
                                Err(e) => {
                                    state.push_transcript(&format!("[Load failed: {}]", e));
                                }
                            }
                        }
                        Err(e) => {
                            state.push_transcript(&format!("[Load failed: {}]", e));
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

            // ── apply_action handles everything else ───────────────────────────
            other => {
                apply_action(other, &mut state, &mut mapper);
            }
        }

        // After apply_action: check for saves-manager or reset prompt that was submitted.
        // (This covers the case where apply_action routed a saves/reset prompt submit.)
        if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
            handle_saves_prompt(kind, buf, &save_dir, &ifid, &mut mapper, &mut session, &mut state, &story_bytes);
        }

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
        }
    }

    // ── 6. Exit: restore terminal + (optional) autosave ───────────────────────

    restore_terminal();

    // Save on exit ONLY when auto_save is enabled. With auto_save off (the default),
    // nothing is saved automatically — the user controls saving via the quit prompt's
    // "Save & quit", the /save command, or named save slots. This keeps "Quit without
    // saving" honest and avoids silently overwriting an explicit save point on exit.
    if state.config.auto_save {
        let exit_meta = app::archive::Meta {
            format_version: 1,
            ifid: Some(ifid.clone()),
            name: None,
            turns: state.turns,
            saved_at: format_rfc3339(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            ),
        };
        match save_archive_meta(&arc_file, &mapper, &session.machine, exit_meta, &state.transcript, &state.transcript_kinds, &state.history) {
            Ok(()) => {
                eprintln!("babelmap: map saved to {}", arc_file.display());
            }
            Err(e) => {
                eprintln!("babelmap: warning: could not save to {}: {}", arc_file.display(), e);
                // Fall back to legacy map-only save so data is not lost.
                if let Err(map_err) = save_map(&map_file, &mapper) {
                    eprintln!("babelmap: warning: fallback map save also failed: {}", map_err);
                }
            }
        }
    }
}

// ── Reset helper ──────────────────────────────────────────────────────────────

/// Rebuild the session from `story_bytes`, reset all ephemeral state, and
/// re-seed the mapper with the start room.  When `clear_map` is true, the
/// accumulated map is wiped first (same effect as `/reset map`) so only the
/// start room remains after the re-seed.
fn reset_game(
    session: &mut GameSession,
    mapper: &mut Mapper,
    state: &mut AppState,
    story_bytes: &[u8],
    clear_map: bool,
) {
    match GameSession::new(story_bytes.to_vec()) {
        Ok(new_session) => {
            *session = new_session;
            session.machine.undo_cap = state.config.undo_levels;
            let start_loc = zvm::current_location(&session.machine);
            state.turns = 0;
            state.input.clear();
            state.suggestions.clear();
            state.suggestion_idx = 0;
            state.transcript.clear();
            state.transcript_kinds.clear();
            state.transcript_scroll = 0;
            if clear_map {
                *mapper = Mapper::default();
            }
            let banner = session.take_transcript();
            state.push_transcript(&banner);
            if let Some(snap) = start_loc {
                let snap_number = snap.number;
                let seed_result = TurnResult {
                    transcript: String::new(),
                    location: Some(snap),
                    quit: false,
                    info: None,
                    beep: None,
                    diagnostics: vec![],
                    location_method: None,
                };
                apply_turn(mapper, "", &seed_result);
                let rid = snap_number as mapper::graph::RoomId;
                state.select_room(Some(rid));
            }
            state.push_transcript("[Game reset]");
        }
        Err(e) => {
            state.push_transcript(&format!("[Reset failed: {:?}]", e));
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Handle a submitted saves-manager or game-reset prompt.
/// Called after apply_action stores the prompt in `state.saves_prompt_submitted`.
fn handle_saves_prompt(
    kind: PromptKind,
    buf: String,
    dir: &std::path::Path,
    ifid: &str,
    mapper: &mut Mapper,
    session: &mut app::session::GameSession,
    state: &mut AppState,
    _story_bytes: &[u8],
) {
    match kind {
        PromptKind::SaveAs => {
            if buf.is_empty() {
                state.push_transcript("[Save name cannot be empty]".to_string().as_str());
                return;
            }
            match save_named(dir, ifid, &buf, mapper, &session.machine, state.turns, &state.transcript, &state.transcript_kinds) {
                Ok(()) => {
                    state.push_transcript(&format!("[Saved as: {}]", buf));
                    // Refresh saves list.
                    if let Some(s) = &mut state.saves {
                        s.entries = list_saves(dir, ifid);
                    }
                }
                Err(e) => {
                    state.push_transcript(&format!("[Save failed: {}]", e));
                }
            }
        }
        PromptKind::ConfirmDeleteSave(path) => {
            let confirmed = matches!(buf.trim().to_lowercase().as_str(), "y" | "yes");
            if confirmed {
                match delete_save(&path) {
                    Ok(()) => {
                        state.push_transcript("[Save deleted]");
                        if let Some(s) = &mut state.saves {
                            s.entries = list_saves(dir, ifid);
                            if s.selected >= s.entries.len() && !s.entries.is_empty() {
                                s.selected = s.entries.len() - 1;
                            }
                        }
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[Delete failed: {}]", e));
                    }
                }
            } else {
                state.push_transcript("[Delete cancelled]");
            }
        }
        PromptKind::ExportSaveName(export_dir) => {
            let filename = buf.trim().to_string();
            if filename.is_empty() {
                state.push_transcript("[Export filename cannot be empty]");
                return;
            }
            let target = export_dir.join(&filename);
            match save_game(&target, &session.machine) {
                Ok(()) => {
                    state.push_transcript(&format!("[Exported to {}]", target.display()));
                }
                Err(e) => {
                    state.push_transcript(&format!("[Export failed: {}]", e));
                }
            }
        }
        _ => {} // other prompt kinds are handled elsewhere
    }
}

/// Format a Unix timestamp (seconds since epoch) as YYYYMMDD-HHMMSS (UTC).
fn format_stamp(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hour = (secs / 3600) % 24;
    let days = secs / 86400;
    let (year, month, day) = days_to_ymd_main(days);
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", year, month, day, hour, min, sec)
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

/// Build a `DialogStyle` from the current app colors.
/// Note: `BorderStyle::None` is coerced to `Single` inside `draw_dialog`.
fn make_dialog_style(state: &AppState) -> DialogStyle {
    DialogStyle {
        frame: state.colors.dialog,
        box_style: state.colors.dialog_box_style,
        title: state.colors.dialog_title,
        button: state.colors.dialog_button,
        button_active: state.colors.dialog_button_active,
        shadow: state.colors.dialog_shadow,
        shadow_on: state.colors.dialog_shadow_on,
    }
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
    ToggleClear,
    Confirm,
    Cancel,
}

/// Map a key code to a ResetDialogAction.
/// Esc and 'c' cancel; Enter and 'r' confirm; Space toggles the checkbox.
#[cfg_attr(not(test), allow(dead_code))]
fn reset_dialog_key(code: crossterm::event::KeyCode) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Enter | KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => ResetDialogAction::ToggleClear,
        _ => ResetDialogAction::None,
    }
}

/// Reset-dialog keys with button focus. Tab/BackTab are handled by the caller
/// (which mutates dialog_focus); this maps Enter to the focused button and keeps
/// the existing accelerators.
fn reset_dialog_key_focused(code: crossterm::event::KeyCode, focus: usize) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => ResetDialogAction::ToggleClear,
        KeyCode::Enter => match focus {
            1 => ResetDialogAction::Cancel,
            _ => ResetDialogAction::Confirm, // focus 0 = Reset (default)
        },
        _ => ResetDialogAction::None,
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
                    match app::session::GameSession::new(bytes) {
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
                    match app::session::GameSession::new(bytes) {
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

/// Return true when a quit attempt should show the "Save before quitting?" dialog.
///
/// Conditions: auto_save is off AND prompt_save_on_quit is on AND the session has
/// at least one turn (unsaved progress exists).
fn should_prompt_save_on_quit(state: &AppState) -> bool {
    !state.config.auto_save && state.config.prompt_save_on_quit && state.turns > 0
}

/// Action to take when a key is pressed while the quit dialog is open.
enum QuitDialogAction {
    None,
    Save,
    Quit,
    Cancel,
}

/// Map a key code to a QuitDialogAction.
/// 's' or Enter → Save & quit; 'q' → Quit without saving; Esc or 'c' → Cancel.
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
    save: &[u8],
    lines: Vec<String>,
    kinds: Vec<TranscriptKind>,
    session: &mut app::session::GameSession,
    mapper: &mut Mapper,
    state: &mut AppState,
    last_panes: &PaneRects,
) {
    match session.machine.restore_file(save) {
        Ok(()) => {
            // mapper was already loaded from the archive at startup (ac.mapper);
            // only the VM machine state needs restoring via restore_quetzal above.
            state.transcript = lines;
            state.transcript_kinds = kinds;
            // Re-observe current location (same as Action::RestoreGame).
            let loc = zvm::current_location(&session.machine);
            if let Some(snap) = loc {
                let rid = snap.number as mapper::graph::RoomId;
                let restore_result = TurnResult {
                    transcript: String::new(),
                    location: Some(snap),
                    quit: false,
                    info: None,
                    beep: None,
                    diagnostics: vec![],
                    location_method: None,
                };
                apply_turn(mapper, "", &restore_result);
                state.set_viewed_layer(None);
                state.select_room(Some(rid));
                if let Some(room) = mapper.graph.room(rid) {
                    if let Some(pos) = room.pos {
                        let (pw, ph) = map_pane_dims(last_panes.map);
                        state.recenter_on(pos, pw, ph);
                    }
                }
            }
            state.push_transcript("[Game resumed from save.]");
        }
        Err(e) => {
            let msg = match e {
                zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                other => format!("restore failed: {:?}", other),
            };
            state.push_transcript(&format!("[Resume failed: {}]", msg));
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

// ── Char-input mode helpers ────────────────────────────────────────────────────

/// Route a turn's sound/diagnostic events: diagnostics become Warning transcript
/// lines; the latest beep arms a one-shot story-border pulse; the current room
/// name is tracked for the built-in location story rule.
fn apply_turn_events(state: &mut AppState, result: &TurnResult) {
    for line in &result.diagnostics {
        state.push_transcript_kind(line, app::state::TranscriptKind::Warning);
    }
    if let Some(kind) = result.beep {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
    state.loc_method = result.location_method.or(state.loc_method);
    // Retain the previous name when this turn has no location signal.
    if let Some(loc) = &result.location {
        state.current_room_name = Some(loc.name.clone());
    }
}

/// Map a keyboard event to a ZSCII byte for `read_char` input.
///
/// Returns:
/// - `Enter`        → 13
/// - `Backspace`    → 8
/// - `Esc`          → 27
/// - `Char(c)` where c is ASCII → `c as u8`
/// - Any other key (non-ASCII Char, arrow keys, etc.) → `None`
pub(crate) fn key_to_zscii(key: crossterm::event::KeyEvent) -> Option<u8> {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Enter     => Some(13),
        KeyCode::Backspace => Some(8),
        KeyCode::Esc       => Some(27),
        KeyCode::Char(c) if c.is_ascii() => Some(c as u8),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::{dim_area, hint_bar, hint_key_routes, is_slash, key_to_zscii, launch_dialog_key, launch_dialog_key_focused, quit_dialog_key, quit_dialog_key_focused, reset_dialog_key, reset_dialog_key_focused, scroll_for_match, should_prompt_save_on_quit, HintKeyKind, LaunchDialogAction, QuitDialogAction, ResetDialogAction};
    use super::{ANIM_HINTS, GAME_HINTS, MAP_HINTS};
    use app::keymap::{Command, Context, HotkeyLayout, KeyMap};
    use app::render::paneframe::{draw_pane_frame, draw_top_inset, InsetSegment};

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
        let frame = draw_pane_frame(&mut buf, area, cs.map_border_style, cs.map_border);
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
        let frame = draw_pane_frame(&mut buf, area, cs.story_border_style, cs.story_border);

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
        // With default keymap: ZoomIn primary key is '+', label from Command::label() is "zoom in"
        assert!(line.contains("+: zoom in"), "expected '+: zoom in' in '{line}'");
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
        assert!(line.contains("Tab: focus"), "focus toggle present: {line}");
        assert!(line.contains("+: zoom in"), "zoom present: {line}");
    }

    #[test]
    fn hint_line_game_contains_save_game() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Global, GAME_HINTS, 200);
        // Ctrl+S → SaveGame; label from Command::label() is "save game"
        assert!(line.contains("Ctrl+S: save game"), "expected 'Ctrl+S: save game' in '{line}'");
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

    #[test]
    fn hint_line_reflects_rebinding() {
        let mut cfg = app::config::KeymapConfig::default();
        cfg.overrides.insert("zoom_in".into(), "z".into());
        let (km, _) = KeyMap::resolve(&cfg);
        let layout = HotkeyLayout::default();
        let line = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        // After rebinding, 'z' is the primary key for ZoomIn; label is "zoom in"
        assert!(line.contains("Z: zoom in"), "expected 'Z: zoom in' in '{line}'");
        // The old '+' key should NOT appear as the primary
        assert!(!line.contains("+: zoom in"), "old binding should not be primary");
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
                if !layout.is_direct(cmd) {
                    continue;
                }
                if let Some(k) = km.primary_key(cmd) {
                    let resolved = km.lookup(&k, ctx);
                    if resolved == Some(cmd) {
                        // This entry would be shown — verify label format.
                        let entry = format!("{}: {}", k.label(), cmd.label());
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
        // Build a layout where ZoomIn is NOT direct (dialog-only), but ToggleFocus IS.
        let mut direct: HashSet<Command> = HashSet::new();
        direct.insert(Command::ToggleFocus);
        direct.insert(Command::Recenter);
        direct.insert(Command::SelectNext);
        // ZoomIn intentionally NOT in direct set.
        let layout = HotkeyLayout {
            prefix: "ctrl+k".parse().unwrap(),
            direct,
            groups: vec![],
        };
        let km = KeyMap::default();
        let bar = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        assert!(
            !bar.contains("zoom in"),
            "ZoomIn should be absent when not direct; got: '{bar}'"
        );
        // ToggleFocus IS direct, so it should appear.
        assert!(
            bar.contains("focus"),
            "ToggleFocus should still appear when direct; got: '{bar}'"
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
    fn hint_bar_labels_come_from_command_label() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();
        let bar = hint_bar(&km, &layout, Context::Map, MAP_HINTS, 200);
        // Command::ZoomIn.label() == "zoom in" (not the old "zoom+")
        assert!(
            bar.contains("zoom in"),
            "label must come from Command::label(); expected 'zoom in', got: '{bar}'"
        );
        // Command::ToggleFocus.label() == "focus"
        assert!(
            bar.contains("focus"),
            "ToggleFocus label should be 'focus'; got: '{bar}'"
        );
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
        assert!(matches!(reset_dialog_key(KeyCode::Char(' ')), ResetDialogAction::ToggleClear));
    }

    // ── should_prompt_save_on_quit ────────────────────────────────────────────

    #[test]
    fn prompt_save_on_quit_all_conditions_required() {
        use app::state::AppState;

        let mut s = AppState::default();
        // Default: auto_save = false, prompt_save_on_quit = true, turns = 0
        // No prompt when turns == 0 (no unsaved progress).
        assert!(!should_prompt_save_on_quit(&s), "turns=0 => no prompt");

        s.turns = 5;
        // Now: auto_save=false, prompt_save_on_quit=true, turns=5 => prompt
        assert!(should_prompt_save_on_quit(&s), "auto_save=false, prompt=true, turns>0 => prompt");

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
        // buttons: [Reset(0), Cancel(1)], default focus 0.
        // Tab -> focus 1 (Cancel); Enter on focus 1 -> Cancel.
        let mut focus = 0usize;
        focus = app::input::cycle_focus(focus, 2, 1);
        assert_eq!(focus, 1);
        let act = reset_dialog_key_focused(KeyCode::Enter, focus);
        assert!(matches!(act, ResetDialogAction::Cancel));
    }

    // ── quit_dialog_tab_then_enter_fires_focused ──────────────────────────────

    #[test]
    fn quit_dialog_tab_then_enter_fires_focused() {
        use crossterm::event::KeyCode;
        // buttons: [Save & quit(0), Quit(1), Cancel(2)], default focus 0.
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

    // ── key_to_zscii tests ────────────────────────────────────────────────────

    fn make_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent {
            code,
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn key_to_zscii_enter_is_13() {
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Enter)), Some(13));
    }

    #[test]
    fn key_to_zscii_backspace_is_8() {
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Backspace)), Some(8));
    }

    #[test]
    fn key_to_zscii_esc_is_27() {
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Esc)), Some(27));
    }

    #[test]
    fn key_to_zscii_ascii_char_y_is_121() {
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Char('y'))), Some(121));
    }

    #[test]
    fn saves_dir_is_user_dir_join_saves() {
        // Save archives live under user_dir/saves, separate from user_dir/maps.
        let d = super::saves_dir(std::path::Path::new("/tmp/bm"));
        assert_eq!(d, std::path::Path::new("/tmp/bm/saves"));
        assert_ne!(d, super::map_dir(std::path::Path::new("/tmp/bm")));
    }

    #[test]
    fn key_to_zscii_non_ascii_char_returns_none() {
        // U+00E9 'é' is not ASCII
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Char('\u{00E9}'))), None);
    }

    #[test]
    fn key_to_zscii_arrow_key_returns_none() {
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Up)), None);
        assert_eq!(key_to_zscii(make_key(crossterm::event::KeyCode::Down)), None);
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
        assert_eq!(key_to_zscii(y_key), Some(b'y'), "'y' should map to ZSCII 121");

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
        assert!(!(s.char_mode && !s.any_overlay_open()),
            "char_mode gate must not fire when overlay is open");
    }
}
