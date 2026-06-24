use std::io::stdout;
use std::time::Duration;

use crossterm::event::{poll, read, Event, KeyEventKind};
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
use app::ifid::{compute_ifid, map_path};
use app::input::{apply_action, key_to_action, Action};
use app::persist_files::{load_map, restore_game, save_game, save_map};
use app::render::inspector::{draw_inspector, room_diagnostics};
use app::render::map::render_map_layered;
use app::render::transcript::render_transcript;
use app::render::draw_str_clipped;
use app::session::{apply_turn, GameSession, TurnResult};
use app::state::{AppState, Focus, Layout, PromptKind};

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
    (Command::ToggleHelp, "help"),
];

const MAP_HINTS: &[(Command, &str)] = &[
    (Command::ToggleFocus, "story"),
    (Command::CycleLayout, "layout"),
    (Command::ZoomIn, "zoom+"),
    (Command::Recenter, "center"),
    (Command::SelectNext, "next"),
    (Command::Retidy, "tidy"),
    (Command::ToggleInspector, "inspect"),
    (Command::ToggleHelp, "help"),
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

/// Render one frame.  Returns the map pane rect (INNER content area) so the event
/// loop can use its dimensions for accurate `recenter_on` calls.
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
                map_area = Rect::default();
            }
            Layout::MapFull => {
                let block = pane("Map", state.focus == Focus::Map);
                let inner = block.inner(main_area);
                block.render(main_area, buf);
                render_map_layered(&rm, &mapper.graph, state, inner, buf);
                map_area = inner; // use inner for recenter math
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

        // ── Inspector overlay ─────────────────────────────────────────────────
        if state.show_inspector && map_area.height > 0 {
            if let Some(id) = state.selected_room {
                let graph = match &state.tidy_anim {
                    Some(anim) => &anim.current().graph,
                    None => &mapper.graph,
                };
                if let Some(diag) = room_diagnostics(graph, id) {
                    draw_inspector(&diag, map_area, buf);
                }
            }
        }

        // ── Change 2: draw help bar in bottom row ─────────────────────────────
        let help_style = Style::default().add_modifier(Modifier::REVERSED);
        let help_text = if let Some(anim) = &state.tidy_anim {
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
                };
                let line = format!("{}{}_", label, prompt.buffer);
                let overlay_style = Style::default().add_modifier(Modifier::REVERSED);
                draw_str_clipped(buf, overlay_area.x, y, &line, overlay_style, overlay_area);
            }
        }
    })?;

    Ok(map_area)
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
    let map_file = map_path(&dir, &ifid);

    let mut mapper = load_map(&map_file).unwrap_or_default();

    // Save-slot and export paths (fixed single slot per IFID).
    let save_slot = dir.join(format!("{}.qzl", ifid));
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

        // Change 3: handle resize by continuing the loop so the next draw uses
        // the updated terminal size (CrosstermBackend picks it up automatically).
        let key_event = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => k,
            Event::Resize(_, _) => continue,
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

                // Clear any manual layer browse override so the view follows the player.
                state.set_viewed_layer(None);

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
                            state.set_viewed_layer(None);
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
