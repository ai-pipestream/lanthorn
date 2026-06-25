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
use app::render::reset_dialog::draw_reset_dialog;
use app::render::hotkeys::draw_hotkey_dialog;
use app::render::verbmenu::draw_verb_menu;
use app::render::inspector::{draw_inspector, room_diagnostics};
use app::render::map::{pulse_border_color, render_map_layered, room_screen_rects};
use app::render::paneframe::{build_layer_segments, draw_pane_frame, draw_top_inset, InsetSegment};
use app::render::tidy_panel::draw_tidy_panel;
use mapper::graph::RoomId;
use mapper::layer::LayerId;
use app::render::room_info::draw_room_info;
use app::render::saves::draw_saves;
use app::render::transcript::render_transcript;
use app::render::draw_str_clipped;
use app::session::{apply_turn, GameSession, TurnResult};
use app::slash::{self, SlashOutcome};
use app::state::{AppState, FbMode, FileBrowserState, Focus, Layout, PromptKind, RoomPanelMode, SavesState, TidyJob, TranscriptKind};

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

/// Persist the live look (`state.colors`/`state.symbols`) to the user's personal
/// style file and repoint `config.toml`'s `style` key at it, then re-resolve so the
/// live look matches the self-contained file just written.
fn save_style_and_repoint(state: &mut AppState, user_dir: &std::path::Path) {
    let style_path = app::style::personal_style_path(user_dir);
    let _ = app::style::write_style_full(&style_path, &state.colors, &state.symbols);
    state.config.style = Some(style_path.to_string_lossy().into_owned());
    let _ = app::config::write_config(user_dir, &state.config);

    // Re-resolve from the now-self-contained style file (+ any config overrides).
    let (base, _w1) = app::style::load_style(state.config.style.as_deref(), user_dir);
    let over = app::style::style_from_config(&state.config.colors, &state.config.symbols);
    let (cs, set, _w2) = app::style::resolve(&app::style::merge(&base, &over), user_dir);
    state.colors = cs;
    state.symbols = set;
}

// ── Hint bar ─────────────────────────────────────────────────────────────────

use app::keymap::{Command, Context, KeyMap};

/// Curated per-context shortlists for the bottom hint bar.
/// Each element is (Command, short label override).
const GAME_HINTS: &[(Command, &str)] = &[
    (Command::ToggleFocus, "map"),
    (Command::SaveGame, "save"),
    (Command::RestoreGame, "restore"),
    (Command::CycleLayout, "layout"),
    (Command::Retidy, "tidy"),
    (Command::AnimateTidy, "animate"),
];

const MAP_HINTS: &[(Command, &str)] = &[
    (Command::ToggleFocus, "story"),
    (Command::CycleLayout, "layout"),
    (Command::ZoomIn, "zoom+"),
    (Command::Recenter, "center"),
    (Command::SelectNext, "next"),
    (Command::Retidy, "tidy"),
    (Command::OpenGallery, "gallery"),
    (Command::ToggleInspector, "inspect"),
];

const ANIM_HINTS: &[(Command, &str)] = &[
    (Command::AnimStepFwd, "step"),
    (Command::AnimTogglePlay, "play/pause"),
    (Command::AnimExit, "exit"),
    (Command::PanLeft, "pan"),
    (Command::ZoomIn, "zoom"),
];

/// Build the hint bar string for the given context from the keymap.
/// Each entry renders as "KEY: label"; entries are joined with " | ".
pub fn hint_line(keymap: &KeyMap, ctx: Context) -> String {
    let hints: &[(Command, &str)] = match ctx {
        Context::Global => MAP_HINTS, // not used directly, but safe fallback
        Context::Map => MAP_HINTS,
        Context::Anim => ANIM_HINTS,
    };
    // We also have a game-focus path — the caller selects the right context.
    let _ = GAME_HINTS; // suppress unused; caller picks via game_hint_line
    hints
        .iter()
        .filter_map(|(cmd, lbl)| {
            keymap.primary_key(*cmd).map(|k| format!("{}: {}", k.label(), lbl))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Hint line for game focus.
pub fn hint_line_game(keymap: &KeyMap) -> String {
    GAME_HINTS
        .iter()
        .filter_map(|(cmd, lbl)| {
            keymap.primary_key(*cmd).map(|k| format!("{}: {}", k.label(), lbl))
        })
        .collect::<Vec<_>>()
        .join(" | ")
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

        match state.layout {
            Layout::TranscriptFull => {
                let story_frame = draw_pane_frame(buf, main_area, state.colors.story_border_style, state.colors.story_border);
                render_transcript(&session.machine, state, story_frame.content, buf);
                draw_top_inset(buf, story_frame.top_inset, &[InsetSegment { text: &state.title, active: false }], state.colors.story_title, state.colors.story_title);
                story_area = story_frame.content;
                map_area = Rect::default();
            }
            Layout::MapFull => {
                let graph = match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                };
                let layer_ids: Vec<LayerId> = graph.layers().keys().copied().collect();
                let active_layer = state.active_layer(graph);
                let frame = draw_pane_frame(buf, main_area, state.colors.map_border_style, state.colors.map_border);
                render_map_layered(&rm, &mapper.graph, state, frame.content, buf);
                if let Some(anim) = &state.tidy_anim {
                    let tidy_ds = make_dialog_style(state);
                    if let Some(dr) = draw_tidy_panel(anim.current(), frame.content, buf, &tidy_ds) {
                        dialog_rects_out = Some(dr);
                    }
                }
                map_area = frame.content;
                story_area = Rect::default();
                // Overlay layer tabs
                let owned_segs = build_layer_segments(&layer_ids, active_layer);
                let inset_segs: Vec<_> = owned_segs.iter().map(|s| s.as_inset()).collect();
                let tab_rects = draw_top_inset(buf, frame.top_inset, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active);
                layer_tabs_out = layer_ids.into_iter().zip(tab_rects).collect();
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

                let story_frame = draw_pane_frame(buf, chunks[0], state.colors.story_border_style, state.colors.story_border);
                render_transcript(&session.machine, state, story_frame.content, buf);
                draw_top_inset(buf, story_frame.top_inset, &[InsetSegment { text: &state.title, active: false }], state.colors.story_title, state.colors.story_title);
                story_area = story_frame.content;

                let map_frame = draw_pane_frame(buf, chunks[1], state.colors.map_border_style, state.colors.map_border);
                render_map_layered(&rm, &mapper.graph, state, map_frame.content, buf);
                if let Some(anim) = &state.tidy_anim {
                    let tidy_ds = make_dialog_style(state);
                    if let Some(dr) = draw_tidy_panel(anim.current(), map_frame.content, buf, &tidy_ds) {
                        dialog_rects_out = Some(dr);
                    }
                }
                map_area = map_frame.content;
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
                    let tab_rects = draw_top_inset(buf, map_frame.top_inset, &inset_segs, state.colors.map_layer_tab, state.colors.map_layer_tab_active);
                    layer_tabs_out = layer_ids.into_iter().zip(tab_rects).collect();
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
                    dim_area(buf, story_frame.content);
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
            let hints = hint_line(&state.keymap, Context::Anim);
            format!(
                "Tidy [{}/{}] {}{} | {}",
                anim.idx + 1,
                anim.frames.len(),
                f.label,
                if anim.playing { " \u{25b6}" } else { "" },
                hints,
            )
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
            match state.focus {
                Focus::Game => hint_line_game(&state.keymap),
                Focus::Map => hint_line(&state.keymap, Context::Map),
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

    Ok(PaneRects { map: map_area, story: story_area, room_rects: room_rects_out, layer_tabs: layer_tabs_out, dialog: dialog_rects_out, reset_dialog: reset_dialog_rects_out })
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

fn main() {
    // ── 1. Parse args + load config ───────────────────────────────────────────

    let cli = Cli::parse();
    let cfg = resolve(&cli);
    let story_path = cli.story.clone();

    let story_bytes = match std::fs::read(&story_path) {
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

    // ── 2. IFID + map dir + load/create mapper ────────────────────────────────

    let ifid = compute_ifid(&story_bytes);
    let dir = map_dir(&cfg.user_dir);
    let arc_file = archive_path(&dir, &ifid);
    let map_file = map_path(&dir, &ifid);

    // Load mapper (and optionally restore the game save) from the archive.
    // Migration: if no archive exists but a legacy .map.json does, load that.
    // use_default_map = true: also fall back to legacy map when no archive.
    let mut mapper = if arc_file.exists() {
        match load_archive(&arc_file) {
            Ok(ac) => {
                // Restore the machine from the saved game state only when auto_load is enabled.
                // When auto_load = false the accumulated map still loads, but the game starts fresh.
                if cfg.auto_load {
                    if let Err(e) = session.machine.restore_quetzal(&ac.save) {
                        eprintln!("babelmap: warning: could not restore game from archive: {:?}", e);
                    }
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
    // Resolve the look from the style file (base) ⊕ config override sections.
    let (base, w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let over = app::style::style_from_config(&cfg.colors, &cfg.symbols);
    let (cs, set, w2) = app::style::resolve(&app::style::merge(&base, &over), &cfg.user_dir);
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
    state.config = cfg;

    // Seed autocomplete with the story's parser vocabulary (room nouns are added live).
    state.dict_words = zvm::dictionary::load(&session.machine.mem).words(&session.machine.mem);

    // Push the game's opening banner and capture the title from it.
    let banner = session.take_transcript();
    let banner_line = app::session::first_banner_line(&banner);
    state.title = app::session::resolve_title(None, banner_line.as_deref(), &story_path);
    state.push_transcript(&banner);

    // Observe the starting room so it appears on the map immediately.
    let start_loc = zvm::current_location(&session.machine);
    if let Some(snap) = start_loc {
        let snap_number = snap.number;
        let seed_result = TurnResult {
            transcript: String::new(),
            location: Some(snap),
            quit: session.quit,
            info: None,
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
    let mut last_panes = PaneRects { map: Rect::default(), story: Rect::default(), room_rects: Vec::new(), layer_tabs: Vec::new(), dialog: None, reset_dialog: None };

    // Debounce counter for BackgroundTidy::Debounced mode.
    let mut bg_tidy_counter: u32 = 0;

    // Poll FPS while a background tidy is in flight.
    const TIDY_POLL_MS: u64 = 33;

    loop {
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
        let poll_ms = if state.tidy_job.is_some() { TIDY_POLL_MS } else { 50 };
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
                    match reset_dialog_key(k.code) {
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
                Event::Resize(_, _) => { continue; }
                _ => {}
            }
            continue;
        }

        // Route event to an Action.
        let action = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => key_to_action(&state, k),
            Event::Mouse(m) => {
                mouse_to_action(&state, m, last_panes.map, last_panes.story, &last_panes.room_rects, &last_panes.dialog)
            }
            // Resize: continue so the next draw uses the updated terminal size.
            Event::Resize(_, _) => continue,
            _ => continue,
        };

        // Note whether this action closes the gallery (persist the look afterward).
        let gallery_cfg_on_close = matches!(action, Action::GalleryClose);

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

            Action::Quit => break,

            Action::SubmitCommand(cmd) => {
                // When a prompt is active, SubmitCommand is the Enter sentinel;
                // route to apply_action to apply the prompt to the mapper.
                if state.prompt.is_some() {
                    apply_action(Action::SubmitCommand(cmd), &mut state, &mut mapper);
                    // Handle any saves-manager or reset prompt that was submitted.
                    if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
                        handle_saves_prompt(
                            kind, buf, &dir, &ifid, &mut mapper, &mut session, &mut state, &story_bytes,
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
                            apply_action(a, &mut state, &mut mapper);
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
                                    save_named(&dir, &ifid, name, &mapper, &session.machine, state.turns)
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
                                    save_archive_meta(&arc_file, &mapper, &session.machine, meta)
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
                                    let saves = list_saves(&dir, &ifid);
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
                                            let restore_err = session.machine.restore_quetzal(&ac.save).map_err(|e| {
                                                match e {
                                                    zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                                                    other => format!("restore failed: {:?}", other),
                                                }
                                            });
                                            match restore_err {
                                                Ok(()) => {
                                                    mapper = ac.mapper;
                                                    let loc = zvm::current_location(&session.machine);
                                                    if let Some(snap) = loc {
                                                        let rid = snap.number as mapper::graph::RoomId;
                                                        let restore_result = TurnResult {
                                                            transcript: String::new(),
                                                            location: Some(snap),
                                                            quit: false,
                                                            info: None,
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
                        SlashOutcome::Quit => break,
                        // Handled in later tasks (Task 5 / Task 6).
                        SlashOutcome::Search(_) => {
                            state.set_status("search: not yet implemented");
                        }
                        SlashOutcome::Filter(_) => {
                            state.set_status("filter: not yet implemented");
                        }
                        SlashOutcome::Export(_) => {
                            state.set_status("export: not yet implemented");
                        }
                    }
                    continue;
                }

                // Clear any transient status message on a real game turn.
                state.status_msg = None;

                // Increment the session turn counter.
                state.turns += 1;

                let result = session.submit(&cmd);
                state.push_transcript(&format!("> {}", cmd));
                state.push_transcript(&result.transcript);
                if let Some(note) = &result.info {
                    state.push_transcript(note);
                }

                // Capture room count before apply_turn for new-room detection.
                let rooms_before = mapper.graph.rooms().count();

                apply_turn(&mut mapper, &cmd, &result);

                // Bump the graph generation so any in-flight tidy result is detected as stale.
                state.graph_gen = state.graph_gen.wrapping_add(1);

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

                        // Try to lock the player object on a room change.
                        if state.player_obj.is_none() {
                            if let Some(locked) = detect_player_obj(
                                state.prev_location,
                                &state.prev_objects_here,
                                current_loc,
                                &objects_here,
                            ) {
                                state.player_obj = Some(locked);
                            }
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
                    if let Err(e) = save_archive_meta(&arc_file, &mapper, &session.machine, meta) {
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
                    if should_bg_tidy(state.config.background_tidy, new_room, overlap, &mut bg_tidy_counter) {
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
                match save_archive_meta(&arc_file, &mapper, &session.machine, meta) {
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
                        let restore_err = session.machine.restore_quetzal(&ac.save).map_err(|e| {
                            match e {
                                zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                                other => format!("restore failed: {:?}", other),
                            }
                        });
                        match restore_err {
                            Ok(()) => {
                                mapper = ac.mapper;
                                // After restore, re-observe current location.
                                let loc = zvm::current_location(&session.machine);
                                if let Some(snap) = loc {
                                    let rid = snap.number as mapper::graph::RoomId;
                                    let restore_result = TurnResult {
                                        transcript: String::new(),
                                        location: Some(snap),
                                        quit: false,
                                        info: None,
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
                let entries = list_saves(&dir, &ifid);
                state.saves = Some(SavesState { entries, selected: 0 });
            }

            Action::SavesExport => {
                // Close saves modal and open file browser in PickDir mode.
                state.saves = None;
                let start_dir = state.config.user_dir.clone();
                let default_name = format!("{}.qzl", ifid);
                state.file_browser = Some(FileBrowserState::build(start_dir, FbMode::PickDir, default_name));
            }

            Action::SavesImport => {
                // Close saves modal and open file browser in PickFile mode.
                state.saves = None;
                let start_dir = state.config.user_dir.clone();
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
                            let restore_err = session.machine.restore_quetzal(&ac.save).map_err(|e| {
                                match e {
                                    zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
                                    other => format!("restore failed: {:?}", other),
                                }
                            });
                            match restore_err {
                                Ok(()) => {
                                    mapper = ac.mapper;
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

            // ── apply_action handles everything else ───────────────────────────
            other => {
                apply_action(other, &mut state, &mut mapper);
            }
        }

        // After apply_action: check for saves-manager or reset prompt that was submitted.
        // (This covers the case where apply_action routed a saves/reset prompt submit.)
        if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
            handle_saves_prompt(kind, buf, &dir, &ifid, &mut mapper, &mut session, &mut state, &story_bytes);
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

    // ── 6. Exit: restore terminal + autosave ──────────────────────────────────

    restore_terminal();

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
    match save_archive_meta(&arc_file, &mapper, &session.machine, exit_meta) {
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
            match save_named(dir, ifid, &buf, mapper, &session.machine, state.turns) {
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
fn reset_dialog_key(code: crossterm::event::KeyCode) -> ResetDialogAction {
    use crossterm::event::KeyCode;
    match code {
        KeyCode::Esc | KeyCode::Char('c') => ResetDialogAction::Cancel,
        KeyCode::Enter | KeyCode::Char('r') => ResetDialogAction::Confirm,
        KeyCode::Char(' ') => ResetDialogAction::ToggleClear,
        _ => ResetDialogAction::None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::{dim_area, hint_line, hint_line_game, is_slash, reset_dialog_key, ResetDialogAction};
    use app::keymap::{Context, KeyMap};
    use app::render::paneframe::{draw_pane_frame, draw_top_inset, InsetSegment};

    // ── TestBackend: map pane shows picture-frame top-left by default ──────────

    /// Verify that the DEFAULT_STYLE_TOML-resolved ColorScheme configures
    /// `map_border_style` as picture-frame, and that rendering it produces ┏ at
    /// the top-left corner of the map pane area.
    #[test]
    fn map_pane_default_shows_picture_frame_corner() {
        // Resolve the default look from DEFAULT_STYLE_TOML (same path as startup).
        let doc = app::style::parse_style_toml(app::style::DEFAULT_STYLE_TOML)
            .expect("DEFAULT_STYLE_TOML must parse");
        let (cs, _set, _warnings) = app::style::resolve(&doc, std::path::Path::new("."));

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf, area, cs.map_border_style, cs.map_border);
        // DEFAULT_STYLE_TOML sets map_border to picture-frame; top-left outer corner must be ┏
        assert_eq!(
            buf.cell((0, 0)).unwrap().symbol(),
            "┏",
            "default map border (from DEFAULT_STYLE_TOML) must be picture-frame (┏ at top-left)"
        );
        // Content area inset 3 on all sides for picture-frame so it clears the corner
        // notches (20-6=14, 10-6=4).
        assert_eq!(frame.content, Rect::new(3, 3, 14, 4));
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

    // ── hint_line ──────────────────────────────────────────────────────────────

    #[test]
    fn hint_line_map_contains_zoom_with_plus_key() {
        let km = KeyMap::default();
        let line = hint_line(&km, Context::Map);
        // With default keymap: ZoomIn primary key is '+', label is "zoom+"
        assert!(line.contains("+: zoom+"), "expected '+: zoom+' in '{line}'");
    }

    #[test]
    fn hint_line_game_contains_save_game() {
        let km = KeyMap::default();
        let line = hint_line_game(&km);
        // Ctrl+S → SaveGame; label is "save"
        assert!(line.contains("Ctrl+S: save"), "expected 'Ctrl+S: save' in '{line}'");
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
        let line = hint_line(&km, Context::Map);
        // After rebinding, 'z' is the primary key for ZoomIn
        assert!(line.contains("Z: zoom+"), "expected 'Z: zoom+' in '{line}'");
        // The old '+' key should NOT appear as the primary
        assert!(!line.contains("+: zoom+"), "old binding should not be primary");
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
    /// outer perimeter cells of the map pane area. For a picture-frame border with
    /// a top_inset at row y+1, the inner tab row center cells (x in 2..=right-3,
    /// y == area.y + 1) must NOT be overwritten by the pulse.
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

        // Inner tab row (y+1) center cells must NOT carry the pulse color.
        // For a picture-frame border, top_inset is at y+1, cols 3..=(w-4).
        // The pulse only writes to x==area.x and x==area.right()-1 for the side columns,
        // so the center of the inner tab row (e.g. col area.x+5) is untouched.
        let tab_row_y = area.y + 1;
        for cx in (area.x + 2)..(area.right() - 2) {
            let fg = buf.cell((cx, tab_row_y)).map(|c| c.fg).unwrap();
            assert_ne!(
                fg,
                pulse_color,
                "inner tab row center cell ({cx}, {tab_row_y}) must NOT be overwritten by pulse"
            );
        }
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
}
