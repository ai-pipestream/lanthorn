use std::io::stdout;
use std::time::Duration;

use crossterm::event::{poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use mapper::mapper::Mapper;
use mapper::render::{render as render_map_data, render_layer};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction as LayoutDir, Layout as RatatuiLayout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui::Terminal;

use clap::Parser;

use app::config::{resolve, Cli};
use app::export_dot::export_dot;
use app::export_svg::export_svg;
use app::map_dump::render_dump;
use app::archive::{load_archive, save_archive, save_archive_meta};
use app::ifid::{archive_path, compute_ifid, map_path};
use app::input::{apply_action, key_to_action, mouse_to_action, Action};
use app::persist_files::{delete_save, list_saves, load_map, save_map, save_named};
use app::render::gallery::draw_gallery;
use app::render::hotkeys::draw_hotkey_dialog;
use app::render::inspector::{draw_inspector, room_diagnostics};
use app::render::map::render_map_layered;
use app::render::room_info::draw_room_info;
use app::render::saves::draw_saves;
use app::render::transcript::render_transcript;
use app::render::draw_str_clipped;
use app::session::{apply_turn, GameSession, TurnResult};
use app::state::{AppState, Focus, Layout, PromptKind, RoomPanelMode, SavesState};

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
struct PaneRects {
    map: Rect,
    story: Rect,
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

        // Focus indicator: the pane receiving keys gets a bright bold border and a ▸
        // marker in its title (with a reverse-video title bar); the other pane keeps
        // the default border.
        let focused_border = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
        let pane = |title: &str, focused: bool| {
            let block = Block::default().borders(Borders::ALL);
            if focused {
                let title_span = ratatui::text::Span::styled(
                    format!("\u{25b8} {title}"),
                    Style::default().add_modifier(Modifier::REVERSED),
                );
                block.title(title_span).border_style(focused_border)
            } else {
                block.title(title.to_string())
            }
        };

        match state.layout {
            Layout::TranscriptFull => {
                let block = pane("Story", state.focus == Focus::Game);
                let inner = block.inner(main_area);
                block.render(main_area, buf);
                render_transcript(&session.machine, state, inner, buf);
                story_area = inner;
                map_area = Rect::default();
            }
            Layout::MapFull => {
                let block = pane("Map", state.focus == Focus::Map);
                let inner = block.inner(main_area);
                block.render(main_area, buf);
                render_map_layered(&rm, &mapper.graph, state, inner, buf);
                map_area = inner; // use inner for recenter math
                story_area = Rect::default();
            }
            Layout::Split => {
                // Split 50/50 horizontally with bordered blocks (no divider column).
                let chunks = RatatuiLayout::default()
                    .direction(LayoutDir::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(main_area);

                let transcript_block = pane("Story", state.focus == Focus::Game);
                let transcript_inner = transcript_block.inner(chunks[0]);
                transcript_block.render(chunks[0], buf);
                render_transcript(&session.machine, state, transcript_inner, buf);
                story_area = transcript_inner;

                let map_block = pane("Map", state.focus == Focus::Map);
                let map_inner = map_block.inner(chunks[1]);
                map_block.render(chunks[1], buf);
                render_map_layered(&rm, &mapper.graph, state, map_inner, buf);
                map_area = map_inner; // use inner for recenter math

                // Dim the unfocused pane's inner content so the eye is drawn to the active pane.
                match state.focus {
                    Focus::Game => dim_area(buf, map_inner),
                    Focus::Map => dim_area(buf, transcript_inner),
                }
            }
        }

        // ── Room panel overlay ────────────────────────────────────────────────
        if map_area.height > 0 {
            if let Some(panel) = state.room_panel {
                let graph = match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                };
                match panel.mode {
                    RoomPanelMode::Info => {
                        let current_room = graph.current();
                        let mem = if state.tidy_anim.is_none() {
                            Some(&session.machine.mem)
                        } else {
                            None
                        };
                        draw_room_info(graph, mem, panel.id, current_room, map_area, buf);
                    }
                    RoomPanelMode::Diagnostics => {
                        if let Some(diag) = room_diagnostics(graph, panel.id) {
                            draw_inspector(&diag, map_area, buf);
                        }
                    }
                }
            }
        }

        // ── Change 2: draw help bar in bottom row ─────────────────────────────
        let help_style = Style::default().add_modifier(Modifier::REVERSED);
        let help_text = if state.saves.is_some() {
            "Saves | \u{2191}\u{2193}: select | Enter: load | s: save-as | d: delete | Esc: close".to_string()
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
            draw_hotkey_dialog(state, full, buf);
        }

        // ── Gallery overlay — drawn after hotkey dialog ───────────────────────
        if state.gallery.is_some() {
            draw_gallery(state, full, buf);
        }

        // ── Saves-manager overlay — drawn after gallery ───────────────────────
        if state.saves.is_some() {
            draw_saves(state, full, buf);
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
                };
                let line = format!("{}{}_", label, prompt.buffer);
                let overlay_style = Style::default().add_modifier(Modifier::REVERSED);
                draw_str_clipped(buf, overlay_area.x, y, &line, overlay_style, overlay_area);
            }
        }
    })?;

    Ok(PaneRects { map: map_area, story: story_area })
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
    state.symbols = app::symbols::SymbolSet::resolve(&cfg.symbols);
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

    // Seed autocomplete with the story's parser vocabulary (room nouns are added live).
    state.dict_words = zvm::dictionary::load(&session.machine.mem).words(&session.machine.mem);

    // Push the game's opening banner.
    let banner = session.take_transcript();
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
    // Defaults are used as a fallback; updated after each successful draw.
    let mut last_panes = PaneRects { map: Rect::default(), story: Rect::default() };

    loop {
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

        // Poll for a key event with a short timeout so we stay responsive.
        let event_ready = match poll(Duration::from_millis(50)) {
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

        // Route event to an Action.
        let action = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => key_to_action(&state, k),
            Event::Mouse(m) => {
                // Pick the live graph for hit-testing (tidy-anim shows a frozen subgraph).
                let graph = match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                };
                mouse_to_action(&state, m, last_panes.map, last_panes.story, graph)
            }
            // Resize: continue so the next draw uses the updated terminal size.
            Event::Resize(_, _) => continue,
            _ => continue,
        };

        // Snapshot gallery config before apply_action clears it on GalleryClose.
        let gallery_cfg_on_close = if matches!(action, Action::GalleryClose) {
            state.gallery.as_ref().map(|g| g.symbol_config())
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
                    // Handle any saves-manager prompt that was submitted.
                    if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
                        handle_saves_prompt(
                            kind, buf, &dir, &ifid, &mut mapper, &mut session, &mut state,
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

                // Increment the session turn counter.
                state.turns += 1;

                let result = session.submit(&cmd);
                state.push_transcript(&format!("> {}", cmd));
                state.push_transcript(&result.transcript);
                if let Some(note) = &result.info {
                    state.push_transcript(note);
                }

                apply_turn(&mut mapper, &cmd, &result);

                // Per-turn auto-save (when enabled). Non-fatal: failure is shown in the
                // transcript status line so the player is aware but the loop continues.
                if cfg.auto_save {
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

        // After apply_action: check for saves-manager prompt that was submitted.
        // (This covers the case where apply_action routed a saves prompt submit.)
        if let Some((kind, buf)) = state.saves_prompt_submitted.take() {
            handle_saves_prompt(kind, buf, &dir, &ifid, &mut mapper, &mut session, &mut state);
        }

        // After apply_action: if gallery was just closed, persist the selections to config.
        if let Some(sym_cfg) = gallery_cfg_on_close {
            let _ = app::config::write_symbols(&cfg.user_dir, &sym_cfg);
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

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Handle a submitted saves-manager prompt (SaveAs or ConfirmDeleteSave).
/// Called after apply_action stores the prompt in `state.saves_prompt_submitted`.
fn handle_saves_prompt(
    kind: PromptKind,
    buf: String,
    dir: &std::path::Path,
    ifid: &str,
    mapper: &mut Mapper,
    session: &mut app::session::GameSession,
    state: &mut AppState,
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Modifier, Style};
    use ratatui::text::Span;
    use ratatui::widgets::{Block, Borders, Widget};

    use super::{dim_area, hint_line, hint_line_game};
    use app::keymap::{Context, KeyMap};

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

    // ── Focused-pane title carries REVERSED ───────────────────────────────────

    /// Build a block the same way `pane()` does for the focused case, render it,
    /// and check that the title cell carries REVERSED.
    #[test]
    fn focused_pane_title_has_reversed_modifier() {
        // Reproduce what pane() does when focused=true.
        let title_span = Span::styled(
            "\u{25b8} Story",
            Style::default().add_modifier(Modifier::REVERSED),
        );
        let block = Block::default().borders(Borders::ALL).title(title_span);

        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);

        // The title starts at x=1 on the top border row (y=0) inside the block.
        // The first title character (▸) should carry REVERSED.
        let title_cell = buf.cell((1, 0)).expect("title cell should exist");
        assert!(
            title_cell.modifier.contains(Modifier::REVERSED),
            "focused title cell should have REVERSED modifier; got {:?}",
            title_cell.modifier
        );
    }

    #[test]
    fn unfocused_pane_title_does_not_have_reversed_modifier() {
        // Reproduce what pane() does when focused=false.
        let block = Block::default().borders(Borders::ALL).title("Story".to_string());

        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        block.render(area, &mut buf);

        let title_cell = buf.cell((1, 0)).expect("title cell should exist");
        assert!(
            !title_cell.modifier.contains(Modifier::REVERSED),
            "unfocused title cell should NOT have REVERSED; got {:?}",
            title_cell.modifier
        );
    }

    // ── Split layout: dim unfocused, leave focused undimmed ───────────────────

    /// This test exercises the split-layout dimming logic by simulating what
    /// draw_frame does: render content into two inner rects, then call dim_area
    /// on the unfocused one. It verifies that cells in the unfocused inner rect
    /// have DIM and cells in the focused inner rect do NOT.
    #[test]
    fn split_layout_unfocused_pane_is_dimmed_focused_is_not() {
        // Two side-by-side "panes" within a wider buffer.
        let full = Rect::new(0, 0, 20, 5);
        let _left_inner = Rect::new(1, 1, 8, 3);   // story inner area (focused; not dimmed)
        let right_inner = Rect::new(11, 1, 8, 3);  // map inner area

        let mut buf = Buffer::empty(full);

        // Simulate Focus::Game: dim the map (right) pane, not the transcript (left).
        dim_area(&mut buf, right_inner);

        // Focused pane (left) inner cells should NOT have DIM.
        for y in 1..4 {
            for x in 1..9 {
                assert!(
                    !buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "focused pane cell ({x},{y}) should NOT have DIM"
                );
            }
        }

        // Unfocused pane (right) inner cells should all have DIM.
        for y in 1..4 {
            for x in 11..19 {
                assert!(
                    buf.cell((x, y)).unwrap().modifier.contains(Modifier::DIM),
                    "unfocused pane cell ({x},{y}) should have DIM"
                );
            }
        }
    }
}
