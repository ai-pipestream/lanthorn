use std::io::stdout;
use std::time::Duration;

use crossterm::event::{poll, read, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use mapper::mapper::Mapper;
use mapper::render::render as render_map_data;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction as LayoutDir, Layout as RatatuiLayout, Rect};
use ratatui::Terminal;

use app::export_svg::export_svg;
use app::ifid::{compute_ifid, map_path};
use app::input::{apply_action, key_to_action, Action};
use app::persist_files::{load_map, restore_game, save_game, save_map};
use app::render::map::render_map;
use app::render::transcript::render_transcript;
use app::render::draw_str_clipped;
use app::session::{apply_turn, GameSession, TurnResult};
use app::state::{AppState, Layout};

// ── Terminal restore helpers ──────────────────────────────────────────────────

/// Restore the terminal to cooked mode and leave the alternate screen.
/// Called both on clean exit and from the panic hook.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen);
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
/// 1. `$BABELMAP_MAP_DIR` environment variable.
/// 2. `$HOME/.babelmap/maps`.
/// 3. `./.babelmap/maps` (fallback when HOME is unset).
fn map_dir() -> std::path::PathBuf {
    if let Ok(d) = std::env::var("BABELMAP_MAP_DIR") {
        return std::path::PathBuf::from(d);
    }
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(base).join(".babelmap").join("maps")
}

// ── Draw helper ───────────────────────────────────────────────────────────────

/// Render one frame.  Returns the map pane rect so the event loop can use its
/// dimensions for accurate `recenter_on` calls.
fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    session: &GameSession,
    mapper: &Mapper,
    state: &AppState,
) -> std::io::Result<Rect> {
    let mut map_area = Rect::default();

    terminal.draw(|f| {
        let full = f.area();
        let buf = f.buffer_mut();
        let rm = render_map_data(&mapper.graph);

        match state.layout {
            Layout::TranscriptFull => {
                render_transcript(&session.machine, state, full, buf);
                map_area = Rect::default();
            }
            Layout::MapFull => {
                map_area = full;
                render_map(&rm, state, full, buf);
            }
            Layout::Split => {
                // Split 50/50 horizontally (transcript left, map right).
                let chunks = RatatuiLayout::default()
                    .direction(LayoutDir::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(full);
                let transcript_area = chunks[0];
                map_area = chunks[1];
                render_transcript(&session.machine, state, transcript_area, buf);
                render_map(&rm, state, map_area, buf);
            }
        }

        // Prompt overlay — drawn over the map area (or full screen) as a
        // single bottom line showing kind and buffer.
        if let Some(prompt) = &state.prompt {
            let overlay_area = if map_area.height > 0 { map_area } else { full };
            if overlay_area.height > 0 {
                let y = overlay_area.bottom() - 1;
                let label = match &prompt.kind {
                    app::state::PromptKind::RenameRoom(_) => "Rename: ",
                    app::state::PromptKind::EditNotes(_) => "Notes:  ",
                    app::state::PromptKind::RelabelEdge(_, _) => "Dir:    ",
                };
                let line = format!("{}{}_", label, prompt.buffer);
                let style = ratatui::style::Style::default()
                    .add_modifier(ratatui::style::Modifier::REVERSED);
                draw_str_clipped(buf, overlay_area.x, y, &line, style, overlay_area);
            }
        }
    })?;

    Ok(map_area)
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() {
    // ── 1. Parse args ─────────────────────────────────────────────────────────

    let mut args = std::env::args().skip(1);
    let story_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("Usage: babelmap <story.z5>");
            std::process::exit(2);
        }
    };

    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("babelmap: cannot read '{}': {}", story_path, e);
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
    let dir = map_dir();
    let map_file = map_path(&dir, &ifid);

    let mut mapper = load_map(&map_file).unwrap_or_default();

    // Save-slot and SVG-export paths (fixed single slot per IFID).
    let save_slot = dir.join(format!("{}.qzl", ifid));
    let svg_path = dir.join(format!("{}.svg", ifid));

    // ── 3. Seed initial transcript + starting room ────────────────────────────

    let mut state = AppState::default();

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

    if let Err(e) = execute!(stdout(), EnterAlternateScreen) {
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

    // Track the last-known map pane size for accurate recenter_on calls.
    // Default is used as a fallback; updated after each successful draw.
    #[allow(unused_assignments)]
    let mut last_map_area = Rect::default();

    loop {
        // Draw.
        match draw_frame(&mut terminal, &session, &mapper, &state) {
            Ok(map_area) => {
                last_map_area = map_area;
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

        let key_event = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => k,
            _ => continue,
        };

        let action = key_to_action(&state, key_event);

        match action {
            // ── Caller-handled actions ─────────────────────────────────────────

            Action::Quit => break,

            Action::SubmitCommand(cmd) => {
                // When a prompt is active, SubmitCommand is the Enter sentinel;
                // route to apply_action to apply the prompt to the mapper.
                if state.prompt.is_some() {
                    apply_action(Action::SubmitCommand(cmd), &mut state, &mut mapper);
                    continue;
                }

                // Normal game-focus command submission.
                // Clear input line and echo command.
                let cmd = state.take_input();
                if cmd.is_empty() {
                    continue;
                }

                let result = session.submit(&cmd);
                state.push_transcript(&format!("> {}", cmd));
                state.push_transcript(&result.transcript);
                if let Some(note) = &result.info {
                    state.push_transcript(note);
                }

                apply_turn(&mut mapper, &cmd, &result);

                // Select and recenter on the current room.
                if let Some(snap) = &result.location {
                    let rid = snap.number as mapper::graph::RoomId;
                    state.select_room(Some(rid));
                    if let Some(room) = mapper.graph.room(rid) {
                        if let Some(pos) = room.pos {
                            let (pw, ph) = map_pane_dims(last_map_area);
                            state.recenter_on(pos, pw, ph);
                        }
                    }
                }

                if result.quit {
                    break;
                }
            }

            Action::SaveGame => {
                // v1: fixed single save slot next to the map file.
                match save_game(&save_slot, &session.machine) {
                    Ok(()) => {
                        state.push_transcript(&format!(
                            "[Game saved to {}]",
                            save_slot.display()
                        ));
                    }
                    Err(e) => {
                        state.push_transcript(&format!("[Save failed: {}]", e));
                    }
                }
            }

            Action::RestoreGame => {
                // v1: restore from the fixed single save slot.
                match restore_game(&save_slot, &mut session.machine) {
                    Ok(()) => {
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
                            state.select_room(Some(rid));
                            if let Some(room) = mapper.graph.room(rid) {
                                if let Some(pos) = room.pos {
                                    let (pw, ph) = map_pane_dims(last_map_area);
                                    state.recenter_on(pos, pw, ph);
                                }
                            }
                        }
                        state.push_transcript(&format!(
                            "[Game restored from {}]",
                            save_slot.display()
                        ));
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

            // ── apply_action handles everything else ───────────────────────────
            other => {
                apply_action(other, &mut state, &mut mapper);
            }
        }
    }

    // ── 6. Exit: restore terminal + autosave ──────────────────────────────────

    restore_terminal();

    match save_map(&map_file, &mapper) {
        Ok(()) => {
            eprintln!("babelmap: map saved to {}", map_file.display());
        }
        Err(e) => {
            eprintln!("babelmap: warning: could not save map to {}: {}", map_file.display(), e);
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Return (width, height) of the map pane, defaulting to (80, 24) when zero.
fn map_pane_dims(area: Rect) -> (u16, u16) {
    let w = if area.width == 0 { 80 } else { area.width };
    let h = if area.height == 0 { 24 } else { area.height };
    (w, h)
}
