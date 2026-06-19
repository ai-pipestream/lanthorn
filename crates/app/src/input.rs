//! Input → `Action` mapping and application.
//!
//! # Focus routing
//! `key_to_action` checks global bindings first (Ctrl+Q/C/S/R/E/L, Tab), then
//! routes to the per-focus keymap.  While `state.prompt` is `Some` (text-entry
//! sub-mode), printable chars, Backspace, Enter and Esc are routed to the prompt
//! buffer regardless of focus.
//!
//! # Caller-handled actions
//! `apply_action` handles view/light-correction actions in-process.  The
//! following actions are LEFT FOR THE CALLER (the run loop) to handle and are
//! silently ignored by `apply_action`:
//!   - `SubmitCommand` — caller sends text to the Z-machine.
//!   - `SaveGame` / `RestoreGame` — caller performs I/O.
//!   - `ExportSvg` — caller writes file.
//!   - `Quit` — caller exits the event loop.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mapper::mapper::Mapper;

use crate::state::{AppState, Focus, Prompt, PromptKind};

// ── Action enum ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Caller: submit the contained command string to the Z-machine.
    SubmitCommand(String),
    /// Append a character to `state.input`.
    InputChar(char),
    /// Delete the last character from `state.input`.
    Backspace,
    /// Toggle between Game and Map focus.
    ToggleFocus,
    /// Cycle the UI layout (Split → TranscriptFull → MapFull → Split).
    CycleLayout,
    /// Zoom the map in (more detail).
    ZoomIn,
    /// Zoom the map out (less detail).
    ZoomOut,
    /// Pan the map scroll by (dx, dy).
    Pan(i32, i32),
    /// Re-center the map on the selected room.
    Recenter,
    /// Select the next room in sorted order.
    SelectNext,
    /// Select the previous room in sorted order.
    SelectPrev,
    /// Begin a rename-room prompt for the selected room.
    RenameRoom,
    /// Begin an edit-notes prompt for the selected room.
    EditNotes,
    /// Delete the first outgoing connection of the selected room.
    DeleteSelectedConnection,
    /// Begin a relabel-edge prompt for the first outgoing connection of the
    /// selected room.
    RelabelSelectedEdge,
    /// Nudge the selected room by (dx, dy) grid cells (Manual mode only).
    NudgeSelected(i32, i32),
    /// Caller: save the game.
    SaveGame,
    /// Caller: restore a saved game.
    RestoreGame,
    /// Caller: export the map as SVG.
    ExportSvg,
    /// Caller: exit the application.
    Quit,
    /// No binding found — no-op.
    None,
}

// ── key_to_action ─────────────────────────────────────────────────────────────

/// Map a crossterm `KeyEvent` to an `Action` given the current `AppState`.
///
/// Routing order:
/// 1. **Prompt active** — printable chars, Backspace, Enter, Esc go to the
///    prompt buffer (via dedicated prompt actions).  Global shortcuts still
///    work.
/// 2. **Global** (any focus) — Ctrl+Q/C → Quit; Ctrl+S → SaveGame;
///    Ctrl+R → RestoreGame; Ctrl+E → ExportSvg; Ctrl+L → CycleLayout;
///    plain Tab → ToggleFocus.
/// 3. **Game focus** — printable char → InputChar; Backspace → Backspace;
///    Enter → SubmitCommand.
/// 4. **Map focus** — navigation, zoom, select, edit bindings.
pub fn key_to_action(state: &AppState, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // ── Global shortcuts (checked before focus routing) ───────────────────
    if ctrl {
        return match key.code {
            KeyCode::Char('q') | KeyCode::Char('c') => Action::Quit,
            KeyCode::Char('s') => Action::SaveGame,
            KeyCode::Char('r') => Action::RestoreGame,
            KeyCode::Char('e') => Action::ExportSvg,
            KeyCode::Char('l') => Action::CycleLayout,
            _ => Action::None,
        };
    }
    if key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Tab {
        return Action::ToggleFocus;
    }

    // ── Prompt sub-mode (overrides focus routing for text keys) ───────────
    if state.prompt.is_some() {
        return prompt_key_to_action(key);
    }

    // ── Per-focus routing ─────────────────────────────────────────────────
    match state.focus {
        Focus::Game => game_key_to_action(state, key),
        Focus::Map => map_key_to_action(key),
    }
}

// ── Internal: prompt key routing ──────────────────────────────────────────────

/// When a prompt is active, all input (except global shortcuts already handled)
/// is consumed by the prompt buffer.  We reuse the Action enum to signal intent:
///   - InputChar → append to prompt buffer
///   - Backspace → delete from prompt buffer
///   - SubmitCommand("") → sentinel: Enter pressed (apply_action checks prompt)
///   - ToggleFocus → sentinel: Esc pressed (apply_action cancels prompt)
fn prompt_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Enter => {
            // Sentinel: empty string signals "apply prompt now".  apply_action
            // reads the actual buffer from state.prompt.
            Action::SubmitCommand(String::new())
        }
        KeyCode::Esc => Action::ToggleFocus, // re-used as cancel signal
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE
                || key.modifiers == KeyModifiers::SHIFT =>
        {
            Action::InputChar(c)
        }
        _ => Action::None,
    }
}

// ── Internal: game focus ──────────────────────────────────────────────────────

fn game_key_to_action(state: &AppState, key: KeyEvent) -> Action {
    match key.code {
        // Enter submits the current input buffer content as the command.
        KeyCode::Enter => Action::SubmitCommand(state.input.clone()),
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE
                || key.modifiers == KeyModifiers::SHIFT =>
        {
            Action::InputChar(c)
        }
        _ => Action::None,
    }
}

// ── Internal: map focus ───────────────────────────────────────────────────────

fn map_key_to_action(key: KeyEvent) -> Action {
    let shift = key.modifiers == KeyModifiers::SHIFT;

    macro_rules! plain {
        () => {
            key.modifiers == KeyModifiers::NONE
        };
    }

    match key.code {
        // Shift+Arrows → NudgeSelected
        KeyCode::Left if shift => Action::NudgeSelected(-1, 0),
        KeyCode::Right if shift => Action::NudgeSelected(1, 0),
        KeyCode::Up if shift => Action::NudgeSelected(0, -1),
        KeyCode::Down if shift => Action::NudgeSelected(0, 1),

        // Arrows → Pan
        KeyCode::Left if plain!() => Action::Pan(-1, 0),
        KeyCode::Right if plain!() => Action::Pan(1, 0),
        KeyCode::Up if plain!() => Action::Pan(0, -1),
        KeyCode::Down if plain!() => Action::Pan(0, 1),

        KeyCode::Char('h') if plain!() => Action::Pan(-1, 0),
        KeyCode::Char('l') if plain!() => Action::Pan(1, 0),
        KeyCode::Char('k') if plain!() => Action::Pan(0, -1),
        KeyCode::Char('j') if plain!() => Action::Pan(0, 1),

        KeyCode::Char('+') | KeyCode::Char('=') if plain!() => Action::ZoomIn,
        // '+' can arrive as Shift+'=' on some terminals.
        KeyCode::Char('+') if shift => Action::ZoomIn,

        KeyCode::Char('-') if plain!() => Action::ZoomOut,

        KeyCode::Char('c') if plain!() => Action::Recenter,
        KeyCode::Char('n') if plain!() => Action::SelectNext,
        KeyCode::Char('N') if shift => Action::SelectPrev,
        KeyCode::Char('r') if plain!() => Action::RenameRoom,
        KeyCode::Char('o') if plain!() => Action::EditNotes,
        KeyCode::Char('d') if plain!() => Action::DeleteSelectedConnection,
        KeyCode::Char('e') if plain!() => Action::RelabelSelectedEdge,

        KeyCode::Esc => Action::ToggleFocus,

        _ => Action::None,
    }
}

// ── apply_action ──────────────────────────────────────────────────────────────

/// Apply a view or light-correction action to `state` and/or `mapper`.
///
/// **Caller-handled actions** (silently ignored here — the run loop must act on
/// them): `SubmitCommand` (game focus), `SaveGame`, `RestoreGame`, `ExportSvg`,
/// `Quit`.
///
/// **Prompt sub-mode** — while `state.prompt` is `Some`:
///   - `InputChar(c)` appends to `state.prompt.buffer`.
///   - `Backspace` pops from `state.prompt.buffer`.
///   - `SubmitCommand` (the Enter sentinel from `prompt_key_to_action`) applies
///     the buffer to the mapper and clears `state.prompt`.
///   - `ToggleFocus` (the Esc sentinel) cancels the prompt without applying.
///
/// **Edge rule for DeleteSelectedConnection / RelabelSelectedEdge**: operates on
/// the *first* outgoing connection of the selected room as returned by
/// `mapper.graph.connections()` in iteration order (stable insertion order).
/// If the room has no connections, the operation is a no-op.
///
/// **Recenter**: calls `state.recenter_on(room_pos, 80, 24)` — a default pane
/// size used when the render pane size is not yet available.  The run loop
/// should call `state.recenter_on` with the real pane size when it knows it.
pub fn apply_action(action: Action, state: &mut AppState, mapper: &mut Mapper) {
    // ── Prompt sub-mode ───────────────────────────────────────────────────
    if state.prompt.is_some() {
        match action {
            Action::InputChar(c) => {
                if let Some(p) = &mut state.prompt {
                    p.buffer.push(c);
                }
            }
            Action::Backspace => {
                if let Some(p) = &mut state.prompt {
                    p.buffer.pop();
                }
            }
            // Enter sentinel: apply prompt to mapper then clear.
            Action::SubmitCommand(_) => {
                if let Some(p) = state.prompt.take() {
                    apply_prompt(p, mapper);
                }
            }
            // Esc sentinel: cancel without applying.
            Action::ToggleFocus => {
                state.prompt = None;
            }
            _ => {} // global actions that reached here are handled by caller
        }
        return;
    }

    // ── Normal action dispatch ────────────────────────────────────────────
    match action {
        Action::InputChar(c) => state.push_input_char(c),
        Action::Backspace => state.backspace(),
        Action::ToggleFocus => state.toggle_focus(),
        Action::CycleLayout => state.cycle_layout(),
        Action::ZoomIn => state.zoom_in(),
        Action::ZoomOut => state.zoom_out(),
        Action::Pan(dx, dy) => state.pan(dx, dy),
        Action::Recenter => apply_recenter(state, mapper),
        Action::SelectNext => select_adjacent(state, mapper, 1),
        Action::SelectPrev => select_adjacent(state, mapper, -1),

        Action::RenameRoom => {
            if let Some(id) = state.selected_room {
                state.prompt = Some(Prompt {
                    kind: PromptKind::RenameRoom(id),
                    buffer: String::new(),
                });
            }
        }
        Action::EditNotes => {
            if let Some(id) = state.selected_room {
                state.prompt = Some(Prompt {
                    kind: PromptKind::EditNotes(id),
                    buffer: String::new(),
                });
            }
        }
        Action::RelabelSelectedEdge => {
            if let Some(id) = state.selected_room {
                // Find the first outgoing connection for this room.
                if let Some(conn) =
                    mapper.graph.connections().iter().find(|c| c.origin == id)
                {
                    let old_dir = conn.dir;
                    state.prompt = Some(Prompt {
                        kind: PromptKind::RelabelEdge(id, old_dir),
                        buffer: String::new(),
                    });
                }
            }
        }
        Action::DeleteSelectedConnection => {
            if let Some(id) = state.selected_room {
                // Delete the first outgoing connection for this room.
                if let Some(conn) =
                    mapper.graph.connections().iter().find(|c| c.origin == id).cloned()
                {
                    mapper.delete_connection(conn.origin, conn.dir);
                }
            }
        }
        Action::NudgeSelected(dx, dy) => {
            if let Some(id) = state.selected_room {
                if let Some(room) = mapper.graph.room(id) {
                    if let Some(pos) = room.pos {
                        let target = (pos.0 + dx, pos.1 + dy);
                        mapper.nudge(id, target);
                    }
                }
            }
        }

        // Caller-handled: silently ignored.
        Action::SubmitCommand(_)
        | Action::SaveGame
        | Action::RestoreGame
        | Action::ExportSvg
        | Action::Quit => {}

        Action::None => {}
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Apply a completed prompt to the mapper.
fn apply_prompt(prompt: Prompt, mapper: &mut Mapper) {
    match prompt.kind {
        PromptKind::RenameRoom(id) => {
            let label = if prompt.buffer.is_empty() {
                None
            } else {
                Some(prompt.buffer)
            };
            mapper.rename_room(id, label);
        }
        PromptKind::EditNotes(id) => {
            mapper.set_notes(id, prompt.buffer);
        }
        PromptKind::RelabelEdge(id, old_dir) => {
            // Parse the user's input as a direction name.
            if let Some(new_dir) = mapper::direction::parse_direction(&prompt.buffer) {
                mapper.relabel_edge(id, old_dir, new_dir);
            }
        }
    }
}

/// Select the next (+1) or previous (-1) room, cycling through all room ids in
/// ascending sorted order.
fn select_adjacent(state: &mut AppState, mapper: &Mapper, delta: i32) {
    let ids: Vec<_> = mapper.graph.rooms().map(|r| r.id).collect();
    if ids.is_empty() {
        return;
    }
    // ids come from BTreeMap iteration so they are already sorted ascending.
    let new_id = match state.selected_room {
        None => {
            if delta >= 0 {
                ids[0]
            } else {
                *ids.last().unwrap()
            }
        }
        Some(current) => {
            let idx = ids.iter().position(|&id| id == current).unwrap_or(0);
            let len = ids.len() as i32;
            let next = ((idx as i32) + delta).rem_euclid(len) as usize;
            ids[next]
        }
    };
    state.select_room(Some(new_id));
}

/// Re-center the map on the selected room's grid position.
/// Uses a fallback pane size of 80×24 when the real render size is unavailable.
fn apply_recenter(state: &mut AppState, mapper: &Mapper) {
    if let Some(id) = state.selected_room {
        if let Some(room) = mapper.graph.room(id) {
            if let Some(pos) = room.pos {
                // TODO(run-loop): pass the real pane size when available.
                state.recenter_on(pos, 80, 24);
                return;
            }
        }
    }
    // No selected room or no position: recenter on origin.
    state.recenter_on((0, 0), 80, 24);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use mapper::mapper::Mapper;

    use super::*;
    use crate::state::AppState;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Build a plain (no-modifier) Press KeyEvent.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Build a Ctrl+key Press KeyEvent.
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Build a Shift+key Press KeyEvent.
    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // ── Brief-required tests ──────────────────────────────────────────────────

    #[test]
    fn game_focus_builds_and_submits_command() {
        let mut s = AppState::default(); // Focus::Game
        for c in "north".chars() {
            let a = key_to_action(&s, key(KeyCode::Char(c)));
            assert!(matches!(a, Action::InputChar(_)));
            if let Action::InputChar(ch) = a {
                s.push_input_char(ch);
            }
        }
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::SubmitCommand(ref c) if c == "north"));
    }

    #[test]
    fn map_focus_pan_and_zoom() {
        let mut s = AppState::default();
        s.toggle_focus(); // Map
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
    }

    #[test]
    fn ctrl_q_quits_in_any_focus() {
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
    }

    // ── Additional tests ──────────────────────────────────────────────────────

    #[test]
    fn map_focus_arrow_pan() {
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::Pan(0, 1)));
    }

    #[test]
    fn map_focus_shift_arrows_nudge() {
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Left)),
            Action::NudgeSelected(-1, 0)
        ));
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Right)),
            Action::NudgeSelected(1, 0)
        ));
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Up)),
            Action::NudgeSelected(0, -1)
        ));
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Down)),
            Action::NudgeSelected(0, 1)
        ));
    }

    #[test]
    fn map_focus_select_next_prev() {
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::SelectNext));
        // 'N' is Shift+n; crossterm sends Char('N') with SHIFT modifier
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Char('N'))),
            Action::SelectPrev
        ));
    }

    #[test]
    fn global_shortcuts_work_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('r'))), Action::RestoreGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::ExportSvg));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::CycleLayout));
    }

    #[test]
    fn tab_toggles_focus() {
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn esc_toggles_focus_in_map() {
        let mut s = AppState::default();
        s.toggle_focus(); // → Map
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::ToggleFocus));
    }

    #[test]
    fn apply_action_pan_accumulates() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::Pan(2, -1), &mut s, &mut m);
        apply_action(Action::Pan(-1, 3), &mut s, &mut m);
        assert_eq!(s.scroll, (1, 2));
    }

    #[test]
    fn apply_action_toggle_focus() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ToggleFocus, &mut s, &mut m);
        assert_eq!(s.focus, crate::state::Focus::Map);
        apply_action(Action::ToggleFocus, &mut s, &mut m);
        assert_eq!(s.focus, crate::state::Focus::Game);
    }

    #[test]
    fn apply_action_select_cycles_rooms() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::N));
        m.observe(3, "C", Some(mapper::direction::Direction::E));

        // No selection yet: SelectNext picks first (id=1).
        apply_action(Action::SelectNext, &mut s, &mut m);
        assert_eq!(s.selected_room, Some(1));

        apply_action(Action::SelectNext, &mut s, &mut m);
        assert_eq!(s.selected_room, Some(2));

        apply_action(Action::SelectPrev, &mut s, &mut m);
        assert_eq!(s.selected_room, Some(1));
    }

    #[test]
    fn prompt_flow_rename_room() {
        // Set up a mapper with one room.
        let mut mapper = Mapper::default();
        mapper.observe(1, "Dark Room", None);

        let mut state = AppState::default();
        state.toggle_focus(); // Map
        state.select_room(Some(1));

        // Press 'r' → RenameRoom action → prompt becomes active.
        let a = key_to_action(&state, key(KeyCode::Char('r')));
        assert!(matches!(a, Action::RenameRoom));
        apply_action(a, &mut state, &mut mapper);
        assert!(state.prompt.is_some());
        assert!(matches!(
            state.prompt.as_ref().unwrap().kind,
            PromptKind::RenameRoom(1)
        ));

        // Type "Lit Room" into the prompt.
        for c in "Lit Room".chars() {
            let k = if c == ' ' {
                key(KeyCode::Char(' '))
            } else {
                key(KeyCode::Char(c))
            };
            let a = key_to_action(&state, k);
            apply_action(a, &mut state, &mut mapper);
        }
        assert_eq!(state.prompt.as_ref().unwrap().buffer, "Lit Room");

        // Press Enter → apply prompt → mapper updated, prompt cleared.
        let a = key_to_action(&state, key(KeyCode::Enter));
        apply_action(a, &mut state, &mut mapper);
        assert!(state.prompt.is_none());
        assert_eq!(mapper.graph.room(1).unwrap().label(), "Lit Room");
    }

    #[test]
    fn prompt_esc_cancels_without_applying() {
        let mut mapper = Mapper::default();
        mapper.observe(1, "Original", None);

        let mut state = AppState::default();
        state.toggle_focus();
        state.select_room(Some(1));

        // Open rename prompt, type something, then Esc.
        apply_action(Action::RenameRoom, &mut state, &mut mapper);
        apply_action(Action::InputChar('X'), &mut state, &mut mapper);
        assert_eq!(state.prompt.as_ref().unwrap().buffer, "X");

        // Esc cancels.
        apply_action(Action::ToggleFocus, &mut state, &mut mapper);
        assert!(state.prompt.is_none());
        // Room name unchanged.
        assert_eq!(mapper.graph.room(1).unwrap().label(), "Original");
    }

    #[test]
    fn game_focus_enter_returns_submit_command_with_current_input() {
        let mut s = AppState::default();
        // Pre-populate input.
        s.push_input_char('g');
        s.push_input_char('o');
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::SubmitCommand(ref c) if c == "go"));
    }
}
