//! Input → `Action` mapping and application.
//!
//! # Focus routing
//! `key_to_action` applies bindings in this strict precedence order:
//! 1. Ctrl+Q / Ctrl+C → Quit (always wins, even during a prompt).
//! 2. Prompt active → route to prompt only; all other keys absorbed as None.
//! 3. Tidy-anim sub-mode → KeyMap lookup in Anim context; no fallthrough.
//! 4. Gallery sub-mode → gallery_key_to_action.
//! 5. Saves-manager sub-mode → saves_key_to_action.
//! 6. Hotkey dialog open → hotkey_dialog_key_to_action.
//! 7. Key == hotkeys.prefix → OpenHotkeyDialog.
//! 8. Tab (no modifiers) → autocomplete-or-ToggleFocus special case.
//! 9. Ctrl modifier → Global KeyMap lookup, filtered by hotkeys.is_direct.
//! 10. Per-focus routing:
//!     - Game: game_key_to_action, then Global fallthrough.
//!     - Map: Map context lookup, filtered by hotkeys.is_direct (direct commands only).
//!
//! While `state.prompt` is `Some` (text-entry sub-mode), printable chars,
//! Backspace, Enter and Esc are routed to the prompt buffer; everything else is
//! absorbed.
//!
//! # Caller-handled actions
//! `apply_action` handles view/light-correction actions in-process.  The
//! following actions are LEFT FOR THE CALLER (the run loop) to handle and are
//! silently ignored by `apply_action`:
//!   - `SubmitCommand` — caller sends text to the Z-machine.
//!   - `SaveGame` / `RestoreGame` — caller performs I/O.
//!   - `ExportSvg` — caller writes file.
//!   - `Quit` — caller exits the event loop.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use mapper::mapper::Mapper;

use crate::complete::{room_words_from_text, suggest};
use crate::keymap::{Context, KeySpec};
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
    /// Re-tidy the Auto layout: re-derive room positions (sort) then clean overlaps.
    /// No-op in Manual mode (positions are user-controlled and frozen).
    Retidy,
    /// Run the tidy pipeline and start animated playback of its stages (Auto only).
    AnimateTidy,
    /// Step the tidy animation by N frames (negative = back); pauses playback.
    AnimStep(i32),
    /// Toggle the tidy animation between playing and paused.
    AnimTogglePlay,
    /// Exit tidy-animation playback back to the live map.
    AnimExit,
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
    /// Begin a rename-layer prompt for the active layer.
    RenameLayer,
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
    /// Caller: export the map as a Graphviz DOT graph.
    ExportDot,
    /// Caller: write an annotatable text/ASCII map dump.
    ExportDump,
    /// Toggle the in-box alignment code overlay (Ctrl+A).
    ToggleAlignment,
    /// Toggle portal destination name labels beside in-room portal icons (Ctrl+P).
    TogglePortalLabels,
    /// Toggle the per-room diagnostics inspector overlay (map focus, `i` key).
    ToggleInspector,
    /// Caller: exit the application.
    Quit,
    /// Cycle the viewed layer by `delta` steps over the sorted non-empty layer list (clamped at ends).
    CycleLayer(i32),
    /// Peel the selected (or current) room's region into a new child layer.
    PeelLayer,
    /// Merge the active layer into its parent layer.
    MergeLayer,
    /// Advance autocomplete to the next suggestion, applying the current one to
    /// the input buffer (game focus, Tab key — only when a partial word is being
    /// typed AND suggestions are available; otherwise Tab keeps its ToggleFocus
    /// role).
    Autocomplete,
    /// Open the hotkey dialog overlay.
    OpenHotkeyDialog,
    /// Close the hotkey dialog overlay.
    CloseHotkeyDialog,
    /// Open the saves-manager modal (loads the save list).
    OpenSaves,
    /// Navigate the saves list by delta (-1 = up, +1 = down).
    SavesNav(i32),
    /// Load the selected save (caller-handled).
    SavesLoad,
    /// Begin a SaveAs prompt for a new named save (sets up the prompt sub-mode).
    SavesSaveAs,
    /// Begin a confirm-delete prompt for the selected save (sets up the prompt sub-mode).
    SavesDelete,
    /// Close the saves-manager modal without acting.
    SavesClose,
    /// Open the symbol gallery modal.
    OpenGallery,
    /// Move to next preset in the current gallery category.
    GalleryNext,
    /// Move to previous preset in the current gallery category.
    GalleryPrev,
    /// Switch to the next gallery category.
    GalleryCategoryNext,
    /// Switch to the previous gallery category.
    GalleryCategoryPrev,
    /// Close the gallery and persist selections to config (persistence handled by main.rs).
    GalleryClose,
    /// No binding found — no-op.
    None,
    // ── Mouse actions ─────────────────────────────────────────────────────────
    /// Show the story-info panel for `RoomId` (left-click on a room).
    ShowRoomInfo(mapper::graph::RoomId),
    /// Show the diagnostics panel for `RoomId` (right-click on a room).
    ShowRoomDiagnostics(mapper::graph::RoomId),
    /// Close the open room panel (click on map gutter).
    CloseRoomPanel,
    /// Begin a middle-button drag-pan gesture at terminal cell (col, row).
    BeginDragPan(u16, u16),
    /// Continue a middle-button drag-pan gesture at terminal cell (col, row).
    DragPanTo(u16, u16),
    /// End a middle-button drag-pan gesture.
    EndDragPan,
    /// Scroll the transcript by delta lines (positive = down, negative = up).
    TranscriptScroll(i32),
}

// ── key_to_action ─────────────────────────────────────────────────────────────

/// Map a crossterm `KeyEvent` to an `Action` given the current `AppState`.
///
/// Routing order:
/// 1. Ctrl+Q / Ctrl+C → Quit (hardwired, always wins).
/// 2. Prompt active → prompt_key_to_action; everything else absorbed.
/// 3. Tidy-anim active → Anim context lookup; no fallthrough.
/// 4. Gallery open → gallery_key_to_action.
/// 5. Saves modal open → saves_key_to_action.
/// 6. Hotkey dialog open → hotkey_dialog_key_to_action.
/// 7. Key == hotkeys.prefix → OpenHotkeyDialog.
/// 8. Tab (no modifiers) → autocomplete-or-ToggleFocus.
/// 9. Ctrl modifier → Global KeyMap lookup, filtered by hotkeys.is_direct.
/// 10. Per-focus routing:
///     - Game: game_key_to_action, then Global fallthrough (non-ctrl non-printable).
///     - Map: Map context lookup, filtered by hotkeys.is_direct.
pub fn key_to_action(state: &AppState, key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // 1. Quit always wins — even while a prompt is active.
    if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c')) {
        return Action::Quit;
    }

    // 2. Prompt sub-mode: consume all keys; only prompt-relevant ones produce
    //    an action, everything else (Tab, Ctrl+S/R/E/L, …) is absorbed.
    if state.prompt.is_some() {
        return prompt_key_to_action(key);
    }

    // 3. Tidy-animation sub-mode: KeyMap lookup in Anim context; no fallthrough.
    if state.tidy_anim.is_some() {
        let spec = KeySpec::from_key_event(key);
        return match state.keymap.lookup(&spec, Context::Anim) {
            Some(cmd) => cmd.to_action(),
            None => Action::None,
        };
    }

    // 4. Gallery sub-mode: when gallery is open, route to gallery keys.
    if state.gallery.is_some() {
        return gallery_key_to_action(key);
    }

    // 5. Saves-manager sub-mode: when saves modal is open, route to saves keys.
    if state.saves.is_some() {
        return saves_key_to_action(key);
    }

    // 6. Hotkey dialog open: route to dialog handler.
    if state.hotkey_dialog {
        return hotkey_dialog_key_to_action(state, key);
    }

    // 7. Prefix key → open the hotkey dialog.
    let spec = KeySpec::from_key_event(key);
    if spec == state.hotkeys.prefix {
        return Action::OpenHotkeyDialog;
    }

    // 8. Tab (no modifiers): stateful autocomplete-or-ToggleFocus (hardwired).
    if key.modifiers == KeyModifiers::NONE && key.code == KeyCode::Tab {
        // Autocomplete takes priority over focus-toggle when: game is focused,
        // the player is mid-word (non-empty partial), AND suggestions exist.
        // In all other cases Tab keeps its existing ToggleFocus behaviour.
        if state.focus == Focus::Game
            && !state.current_partial().is_empty()
            && !state.suggestions.is_empty()
        {
            return Action::Autocomplete;
        }
        return Action::ToggleFocus;
    }

    // 9. Ctrl modifier: Global KeyMap lookup, filtered by is_direct — same rule
    //    as Map context. A command is reachable directly iff it is in the direct
    //    set, regardless of whether it uses a Ctrl modifier.
    if ctrl {
        return match state.keymap.lookup(&spec, Context::Global) {
            Some(cmd) if state.hotkeys.is_direct(cmd) => cmd.to_action(),
            _ => Action::None,
        };
    }

    // 10. Per-focus routing.
    match state.focus {
        Focus::Game => {
            // Text entry is hardwired (printable chars, Enter, Backspace, Shift+Arrows,
            // Home, PageUp/Down). Non-printable / unmatched keys fall through to a
            // Global KeyMap lookup so that non-ctrl global bindings reach Game focus.
            let a = game_key_to_action(state, key);
            if a != Action::None {
                return a;
            }
            // Global fallthrough for non-ctrl non-Tab non-printable keys.
            match state.keymap.lookup(&spec, Context::Global) {
                Some(cmd) => cmd.to_action(),
                None => Action::None,
            }
        }
        Focus::Map => {
            // Map context lookup with direct filter: only return the action if the
            // command is in the direct (always-available) set. Dialog-only commands
            // return None when the dialog is closed.
            match state.keymap.lookup(&spec, Context::Map) {
                Some(cmd) if state.hotkeys.is_direct(cmd) => cmd.to_action(),
                Some(_) => Action::None,
                None => Action::None,
            }
        }
    }
}

// ── mouse_to_action ───────────────────────────────────────────────────────────

/// Find the first room whose bounding rect contains screen cell `(col, row)`.
fn room_at_screen(
    room_rects: &[(mapper::graph::RoomId, ratatui::layout::Rect)],
    col: u16,
    row: u16,
) -> Option<mapper::graph::RoomId> {
    room_rects
        .iter()
        .find(|(_, rect)| col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom())
        .map(|(id, _)| *id)
}

/// Map a crossterm `MouseEvent` to an `Action` given the current `AppState`, the
/// bounding rects of the map and story panes, and the pre-computed room screen
/// rects (needed for pixel-accurate room hit-testing on left/right clicks).
///
/// Returns `Action::None` for events outside both panes or with no binding.
pub fn mouse_to_action(
    state: &AppState,
    m: MouseEvent,
    map: ratatui::layout::Rect,
    story: ratatui::layout::Rect,
    room_rects: &[(mapper::graph::RoomId, ratatui::layout::Rect)],
) -> Action {
    let col = m.column;
    let row = m.row;
    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);

    let in_map = map.width > 0 && map.height > 0
        && col >= map.x && col < map.right()
        && row >= map.y && row < map.bottom();
    let in_story = story.width > 0 && story.height > 0
        && col >= story.x && col < story.right()
        && row >= story.y && row < story.bottom();

    match m.kind {
        // ── Left-click in map ─────────────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Left) if in_map => {
            match room_at_screen(room_rects, col, row) {
                Some(id) => Action::ShowRoomInfo(id),
                None => Action::CloseRoomPanel,
            }
        }
        // ── Right-click in map ────────────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Right) if in_map => {
            match room_at_screen(room_rects, col, row) {
                Some(id) => Action::ShowRoomDiagnostics(id),
                None => Action::CloseRoomPanel,
            }
        }
        // ── Middle-button: drag-pan ───────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Middle) if in_map => {
            Action::BeginDragPan(col, row)
        }
        MouseEventKind::Drag(MouseButton::Middle) => {
            Action::DragPanTo(col, row)
        }
        MouseEventKind::Up(MouseButton::Middle) => {
            Action::EndDragPan
        }
        // ── Wheel in map: pan or zoom ─────────────────────────────────────────
        MouseEventKind::ScrollUp if in_map => {
            if ctrl {
                Action::ZoomIn
            } else if shift {
                Action::Pan(-1, 0)
            } else {
                Action::Pan(0, -1)
            }
        }
        MouseEventKind::ScrollDown if in_map => {
            if ctrl {
                Action::ZoomOut
            } else if shift {
                Action::Pan(1, 0)
            } else {
                Action::Pan(0, 1)
            }
        }
        MouseEventKind::ScrollLeft if in_map => Action::Pan(-1, 0),
        MouseEventKind::ScrollRight if in_map => Action::Pan(1, 0),
        // ── Wheel in story: scroll transcript ────────────────────────────────
        MouseEventKind::ScrollUp if in_story => Action::TranscriptScroll(-1),
        MouseEventKind::ScrollDown if in_story => Action::TranscriptScroll(1),
        // ── Everything else ───────────────────────────────────────────────────
        _ => Action::None,
    }
}

// ── Internal: hotkey dialog key routing ───────────────────────────────────────

/// When the hotkey dialog is open, route keys to either close the dialog or
/// fire the bound command action. The dialog closes itself when a sub-mode
/// opens (handled in apply_action).
fn hotkey_dialog_key_to_action(state: &AppState, key: KeyEvent) -> Action {
    let spec = KeySpec::from_key_event(key);

    // Prefix key closes the dialog.
    if spec == state.hotkeys.prefix {
        return Action::CloseHotkeyDialog;
    }

    // 'q' with no modifiers also closes.
    if key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE {
        return Action::CloseHotkeyDialog;
    }

    // Look up the key across all contexts (Global, Map, Anim) so that commands
    // in any context can be triggered from the dialog.
    if let Some(cmd) = state.keymap.lookup_any(&spec) {
        return cmd.to_action();
    }

    Action::None
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

// ── Internal: saves-manager key routing ───────────────────────────────────────

/// Hardwired saves-manager sub-mode keys (not rebindable, like prompt and anim).
fn saves_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::SavesNav(-1),
        KeyCode::Down => Action::SavesNav(1),
        KeyCode::Enter => Action::SavesLoad,
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::SavesSaveAs,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::SavesDelete,
        KeyCode::Esc => Action::SavesClose,
        _ => Action::None,
    }
}

// ── Internal: gallery key routing ─────────────────────────────────────────────

fn gallery_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::GalleryPrev,
        KeyCode::Down => Action::GalleryNext,
        KeyCode::Left => Action::GalleryCategoryPrev,
        KeyCode::Right => Action::GalleryCategoryNext,
        KeyCode::Esc | KeyCode::Enter => Action::GalleryClose,
        _ => Action::None,
    }
}

// ── Internal: game focus ──────────────────────────────────────────────────────

fn game_key_to_action(state: &AppState, key: KeyEvent) -> Action {
    let shift = key.modifiers == KeyModifiers::SHIFT;
    match key.code {
        // Map navigation is available WITHOUT leaving the story line: Shift+Arrows
        // pan, and the non-typeable Home / PageUp / PageDown centre and zoom. None of
        // these clash with typing a command (arrows/Home/PageX aren't printable, and
        // Shift+Arrow is distinct from a Shift+letter capital).
        KeyCode::Left if shift => Action::Pan(-1, 0),
        KeyCode::Right if shift => Action::Pan(1, 0),
        KeyCode::Up if shift => Action::Pan(0, -1),
        KeyCode::Down if shift => Action::Pan(0, 1),
        KeyCode::Home => Action::Recenter,
        KeyCode::PageUp => Action::ZoomIn,
        KeyCode::PageDown => Action::ZoomOut,
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

// ── Tidy pipeline ─────────────────────────────────────────────────────────────

/// Run the auto-tidy pipeline on the given `layer`, returning a labelled snapshot of the
/// sub-graph after each stage (frame 0 is the pre-tidy state). The tidied positions are written
/// back into `graph` for every room in `layer`; all other rooms are untouched. Caller must be in
/// Auto mode.
pub(crate) fn run_tidy_pipeline(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
) -> Vec<crate::state::TidyFrame> {
    use crate::render::map::{cleanup_overlaps, compact_empty_lines, repair_directional_hints, stack_updown_rooms};
    use crate::state::TidyFrame;

    let mut sub = graph.layer_subgraph(layer);
    let mut frames = vec![TidyFrame { label: "before".into(), graph: sub.clone() }];
    let snap = |g: &mapper::graph::MapGraph, label: &str, frames: &mut Vec<TidyFrame>| {
        frames.push(TidyFrame { label: label.into(), graph: g.clone() });
    };

    mapper::layout::relayout_auto(&mut sub);
    snap(&sub, "relayout", &mut frames);
    cleanup_overlaps(&mut sub, 3, 40);
    snap(&sub, "cleanup overlaps", &mut frames);
    // Recover directional hints a post-solve stage sacrificed (e.g. a room ejected across a one-way
    // edge), without re-introducing overlaps.
    repair_directional_hints(&mut sub, 3, 40);
    snap(&sub, "repair hints", &mut frames);
    // Stack Up/Down rooms on the correct side of their partner (north for Up, south for Down),
    // accepting a temporary overlap where the area is dense.
    stack_updown_rooms(&mut sub);
    snap(&sub, "stack up/down", &mut frames);
    // Clear any overlap the stacking introduced by moving OTHER rooms — the cleanup is
    // direction-aware, so it won't drag the Up/Down room back to the wrong side.
    cleanup_overlaps(&mut sub, 3, 40);
    snap(&sub, "cleanup overlaps", &mut frames);
    // Collapse any fully-empty rows/columns the shuffling left behind.
    compact_empty_lines(&mut sub);
    snap(&sub, "compact", &mut frames);

    // Write the tidied positions back into the live graph for this layer's rooms.
    for id in graph.rooms_in_layer(layer) {
        // pos is always Some after relayout_auto; the guard is defensive.
        if let Some(p) = sub.room(id).and_then(|r| r.pos) {
            graph.set_pos(id, p);
        }
    }

    // relayout_auto set distorted flags on sub for this layer's geometry; copy them
    // back so the live map's distortion coloring is fresh (positions alone are not enough).
    let n = graph.connections().len();
    for idx in 0..n {
        let c = graph.connections()[idx].clone();
        if graph.layer_of(c.origin) == layer && graph.layer_of(c.dest) == layer {
            if let Some(sc) = sub.connections().iter()
                .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
            {
                graph.set_conn_distorted(idx, sc.distorted);
            }
        }
    }

    frames
}

/// Run the same tidy pipeline stages as `run_tidy_pipeline` but discard the
/// animation frames. The final positions and distortion flags are written back
/// into `graph` exactly as `run_tidy_pipeline` does, but no frame snapshots are
/// allocated. Use this for silent background re-tidy where playback is not wanted.
pub fn tidy_layer_silent(
    graph: &mut mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
) {
    use crate::render::map::{cleanup_overlaps, compact_empty_lines, repair_directional_hints, stack_updown_rooms};

    let mut sub = graph.layer_subgraph(layer);
    mapper::layout::relayout_auto(&mut sub);
    cleanup_overlaps(&mut sub, 3, 40);
    repair_directional_hints(&mut sub, 3, 40);
    stack_updown_rooms(&mut sub);
    cleanup_overlaps(&mut sub, 3, 40);
    compact_empty_lines(&mut sub);

    // Write final positions back into the live graph.
    for id in graph.rooms_in_layer(layer) {
        if let Some(p) = sub.room(id).and_then(|r| r.pos) {
            graph.set_pos(id, p);
        }
    }

    // Write distortion flags back.
    let n = graph.connections().len();
    for idx in 0..n {
        let c = graph.connections()[idx].clone();
        if graph.layer_of(c.origin) == layer && graph.layer_of(c.dest) == layer {
            if let Some(sc) = sub.connections().iter()
                .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
            {
                graph.set_conn_distorted(idx, sc.distorted);
            }
        }
    }
}

/// Pure decision function for background-tidy mode. Extracted for unit-testability.
///
/// - `mode`: the configured `BackgroundTidy` value.
/// - `new_room`: whether this turn discovered at least one new room.
/// - `overlap`: whether the active layer has a room overlap or distorted edge after
///   incremental placement (only meaningful for `OnOverlap`).
/// - `counter`: mutable debounce counter; incremented on each new room, reset when
///   a tidy fires under `Debounced`.
///
/// Returns true when a background re-tidy should be triggered.
pub fn should_bg_tidy(
    mode: crate::config::BackgroundTidy,
    new_room: bool,
    overlap: bool,
    counter: &mut u32,
) -> bool {
    use crate::config::BackgroundTidy;
    match mode {
        BackgroundTidy::Off => false,
        BackgroundTidy::EveryRoom => new_room,
        BackgroundTidy::OnOverlap => overlap,
        BackgroundTidy::Debounced => {
            if new_room {
                *counter += 1;
                if *counter >= crate::config::BG_TIDY_DEBOUNCE {
                    *counter = 0;
                    return true;
                }
            }
            false
        }
    }
}

// ── Gallery helpers ───────────────────────────────────────────────────────────

/// Return the number of presets for the given gallery category index.
fn preset_count(cat: usize) -> usize {
    use crate::symbols::{Arrows, BoxStyle, PathGlyphs, PortalGlyphs};
    match cat {
        0 => BoxStyle::preset_names().len(),
        1 => Arrows::preset_names().len(),
        2 => PortalGlyphs::preset_names().len(),
        _ => PathGlyphs::preset_names().len(),
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
                    // apply_prompt returns the prompt back for saves-manager kinds.
                    if let Some(saves_prompt) = apply_prompt(p, mapper) {
                        // Saves-manager prompt submitted: store for the caller to act on.
                        state.saves_prompt_submitted =
                            Some((saves_prompt.kind, saves_prompt.buffer));
                    }
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
        Action::InputChar(c) => {
            state.push_input_char(c);
            // Recompute suggestions after every character typed in game focus.
            if state.focus == Focus::Game {
                recompute_suggestions(state);
                state.suggestion_idx = 0;
            }
        }
        Action::Backspace => {
            state.backspace();
            // Recompute suggestions after deletion in game focus.
            if state.focus == Focus::Game {
                recompute_suggestions(state);
                state.suggestion_idx = 0;
            }
        }
        Action::Autocomplete => {
            // Apply the currently-highlighted suggestion to the input buffer,
            // replacing the partial word being typed. Then advance the index
            // so repeated Tab cycles through candidates.
            if !state.suggestions.is_empty() {
                let idx = state.suggestion_idx % state.suggestions.len();
                let completion = state.suggestions[idx].clone();
                // Replace the partial word at the end of input with the completion.
                let partial_len = state.current_partial().len();
                let new_len = state.input.len() - partial_len;
                state.input.truncate(new_len);
                state.input.push_str(&completion);
                // Advance index for next Tab press (cycles).
                state.suggestion_idx = (idx + 1) % state.suggestions.len();
            }
        }
        Action::ToggleFocus => state.toggle_focus(),
        Action::CycleLayout => state.cycle_layout(),
        Action::ZoomIn => state.zoom_in(),
        Action::ZoomOut => state.zoom_out(),
        Action::Pan(dx, dy) => state.pan(dx, dy),
        Action::Recenter => apply_recenter(state, mapper),
        Action::SelectNext => select_adjacent(state, mapper, 1),
        Action::SelectPrev => select_adjacent(state, mapper, -1),

        Action::PeelLayer => {
            if let Some(room) = state.selected_room.or_else(|| mapper.graph.current()) {
                if let Some(new) = mapper::layer::peel_region(&mut mapper.graph, room) {
                    state.set_viewed_layer(Some(new));
                }
            }
        }
        Action::MergeLayer => {
            let active = state.active_layer(&mapper.graph);
            mapper::layer::merge_layer(&mut mapper.graph, active); // merges into parent (Task 10)
            state.set_viewed_layer(None);
        }

        // Re-tidy: re-derive the clean Auto layout (constrained stress majorization,
        // or the longest-path sort for very large maps), then nudge rooms so the lane
        // router has no illegal overlaps. Honours compass ordering the greedy per-turn
        // placement can't. No-op in Manual mode — those positions are user-owned.
        Action::Retidy => {
            if mapper.mode == mapper::layout::LayoutMode::Auto {
                let layer = state.active_layer(&mapper.graph);
                run_tidy_pipeline(&mut mapper.graph, layer);
            }
        }

        Action::AnimateTidy => {
            if mapper.mode == mapper::layout::LayoutMode::Auto {
                let layer = state.active_layer(&mapper.graph);
                let frames = run_tidy_pipeline(&mut mapper.graph, layer);
                state.tidy_anim = Some(crate::state::TidyAnim::new(frames));
            }
        }

        Action::AnimStep(d) => {
            if let Some(anim) = &mut state.tidy_anim {
                anim.step(d as isize);
            }
        }

        Action::AnimTogglePlay => {
            if let Some(anim) = &mut state.tidy_anim {
                anim.toggle_play();
            }
        }

        Action::AnimExit => state.tidy_anim = None,

        Action::CycleLayer(delta) => {
            let mut ids: Vec<_> = mapper.graph.layers().keys().copied()
                .filter(|&l| !mapper.graph.rooms_in_layer(l).is_empty())
                .collect();
            ids.sort_unstable();
            if !ids.is_empty() {
                let cur = state.active_layer(&mapper.graph);
                let i = ids.iter().position(|&l| l == cur).unwrap_or(0) as i32;
                let j = (i + delta).clamp(0, ids.len() as i32 - 1) as usize;
                state.set_viewed_layer(Some(ids[j]));
            }
        }

        Action::ToggleAlignment => state.show_alignment = !state.show_alignment,
        Action::TogglePortalLabels => state.show_portal_labels = !state.show_portal_labels,
        Action::ToggleInspector => {
            // Toggle: if a Diagnostics panel is already open for the selected room, close it;
            // otherwise open Diagnostics for the selected room. Keyboard path shares room_panel.
            use crate::state::{RoomPanel, RoomPanelMode};
            if let Some(id) = state.selected_room {
                let already_open = matches!(
                    state.room_panel,
                    Some(RoomPanel { id: pid, mode: RoomPanelMode::Diagnostics }) if pid == id
                );
                if already_open {
                    state.room_panel = None;
                    state.show_inspector = false;
                } else {
                    state.room_panel = Some(RoomPanel { id, mode: RoomPanelMode::Diagnostics });
                    state.show_inspector = true;
                }
            } else {
                // No selected room: toggle off.
                state.room_panel = None;
                state.show_inspector = false;
            }
        }

        Action::RenameRoom => {
            if let Some(id) = state.selected_room {
                state.hotkey_dialog = false;
                state.prompt = Some(Prompt {
                    kind: PromptKind::RenameRoom(id),
                    buffer: String::new(),
                });
            }
        }
        Action::RenameLayer => {
            let layer = state.active_layer(&mapper.graph);
            let current_name = mapper.graph.layer_name(layer).to_owned();
            state.hotkey_dialog = false;
            state.prompt = Some(Prompt {
                kind: PromptKind::RenameLayer(layer),
                buffer: current_name,
            });
        }
        Action::EditNotes => {
            if let Some(id) = state.selected_room {
                state.hotkey_dialog = false;
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
                    state.hotkey_dialog = false;
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

        Action::OpenHotkeyDialog => {
            state.hotkey_dialog = true;
            // Close other overlays if open.
            state.gallery = None;
            state.saves = None;
        }

        Action::CloseHotkeyDialog => {
            state.hotkey_dialog = false;
        }

        // ── Saves-manager actions ─────────────────────────────────────────────

        Action::OpenSaves => {
            // The list must be populated by the caller (main.rs has dir + ifid).
            // apply_action only sets up the state; the caller refreshes the list
            // via AppState::open_saves_modal after apply_action returns.
            // If already open, do nothing.
            state.hotkey_dialog = false;
        }

        Action::SavesNav(delta) => {
            if let Some(s) = &mut state.saves {
                if !s.entries.is_empty() {
                    let len = s.entries.len() as i32;
                    s.selected = ((s.selected as i32 + delta).rem_euclid(len)) as usize;
                }
            }
        }

        // SavesLoad, SavesSaveAs, SavesDelete: state-only pre-work here;
        // the actual I/O is caller-handled.

        Action::SavesSaveAs => {
            // Open the name-entry prompt; on submit the caller performs the save.
            state.hotkey_dialog = false;
            state.prompt = Some(crate::state::Prompt {
                kind: crate::state::PromptKind::SaveAs,
                buffer: String::new(),
            });
        }

        Action::SavesDelete => {
            // Open the confirm-delete prompt for the selected entry.
            if let Some(s) = &state.saves {
                if let Some(entry) = s.entries.get(s.selected) {
                    let path = entry.path.clone();
                    state.hotkey_dialog = false;
                    state.prompt = Some(crate::state::Prompt {
                        kind: crate::state::PromptKind::ConfirmDeleteSave(path),
                        buffer: String::new(),
                    });
                }
            }
        }

        Action::SavesClose => {
            state.saves = None;
        }

        // SavesLoad is caller-handled.

        Action::OpenGallery => {
            state.hotkey_dialog = false;
            state.gallery = Some(crate::state::GalleryState {
                category_idx: 0,
                selections: [0, 0, 0, 0],
            });
        }

        Action::GalleryNext => {
            if let Some(g) = &mut state.gallery {
                let cat = g.category_idx;
                let count = preset_count(cat);
                g.selections[cat] = (g.selections[cat] + 1) % count;
            }
            if let Some(g) = &state.gallery {
                state.symbols = crate::symbols::SymbolSet::resolve(&g.symbol_config());
            }
        }

        Action::GalleryPrev => {
            if let Some(g) = &mut state.gallery {
                let cat = g.category_idx;
                let count = preset_count(cat);
                g.selections[cat] = (g.selections[cat] + count - 1) % count;
            }
            if let Some(g) = &state.gallery {
                state.symbols = crate::symbols::SymbolSet::resolve(&g.symbol_config());
            }
        }

        Action::GalleryCategoryNext => {
            if let Some(g) = &mut state.gallery {
                g.category_idx = (g.category_idx + 1) % 4;
            }
        }

        Action::GalleryCategoryPrev => {
            if let Some(g) = &mut state.gallery {
                g.category_idx = (g.category_idx + 3) % 4;
            }
        }

        Action::GalleryClose => {
            if let Some(g) = state.gallery.take() {
                state.symbols = crate::symbols::SymbolSet::resolve(&g.symbol_config());
                // Persistence is handled by the caller (main.rs detects GalleryClose).
            }
        }

        // ── Mouse room-panel actions ──────────────────────────────────────────

        Action::ShowRoomInfo(id) => {
            use crate::state::{RoomPanel, RoomPanelMode};
            state.room_panel = Some(RoomPanel { id, mode: RoomPanelMode::Info });
            state.selected_room = Some(id);
            state.show_inspector = false;
            // Switch to Map focus so the selected room is rendered as selected (not dimmed).
            state.focus = Focus::Map;
        }

        Action::ShowRoomDiagnostics(id) => {
            use crate::state::{RoomPanel, RoomPanelMode};
            state.room_panel = Some(RoomPanel { id, mode: RoomPanelMode::Diagnostics });
            state.selected_room = Some(id);
            state.show_inspector = true;
            // Switch to Map focus so the selected room is rendered as selected (not dimmed).
            state.focus = Focus::Map;
        }

        Action::CloseRoomPanel => {
            state.room_panel = None;
            state.show_inspector = false;
        }

        // ── Mouse drag-pan actions ────────────────────────────────────────────

        Action::BeginDragPan(col, row) => {
            use crate::state::DragState;
            state.drag = Some(DragState { last: (col, row), acc_x: 0, acc_y: 0 });
        }

        Action::DragPanTo(col, row) => {
            if let Some(drag) = &mut state.drag {
                let (step_w, step_h) = state.zoom.steps();
                let dx = col as i32 - drag.last.0 as i32;
                let dy = row as i32 - drag.last.1 as i32;
                drag.acc_x += dx;
                drag.acc_y += dy;
                drag.last = (col, row);
                // Pan by whole grid cells (grab-and-drag: dragging right scrolls left).
                while drag.acc_x >= step_w {
                    state.scroll.0 -= 1;
                    drag.acc_x -= step_w;
                }
                while drag.acc_x <= -step_w {
                    state.scroll.0 += 1;
                    drag.acc_x += step_w;
                }
                while drag.acc_y >= step_h {
                    state.scroll.1 -= 1;
                    drag.acc_y -= step_h;
                }
                while drag.acc_y <= -step_h {
                    state.scroll.1 += 1;
                    drag.acc_y += step_h;
                }
            }
        }

        Action::EndDragPan => {
            state.drag = None;
        }

        // ── Transcript scroll ─────────────────────────────────────────────────

        Action::TranscriptScroll(delta) => {
            if delta < 0 {
                state.transcript_scroll = state.transcript_scroll.saturating_sub((-delta) as u16);
            } else {
                state.transcript_scroll = state.transcript_scroll.saturating_add(delta as u16);
            }
        }

        // Caller-handled: silently ignored.
        Action::SubmitCommand(_)
        | Action::SaveGame
        | Action::RestoreGame
        | Action::ExportSvg
        | Action::ExportDot
        | Action::ExportDump
        | Action::SavesLoad
        | Action::Quit => {}

        Action::None => {}
        // Note: OpenHotkeyDialog and CloseHotkeyDialog are handled above.
    }
}

// ── Suggestion recompute ──────────────────────────────────────────────────────

/// Recompute `state.suggestions` from `state.dict_words`, the room words
/// extracted from `state.transcript`, and the current partial word being typed.
/// Called internally after every input character change in game focus.
pub(crate) fn recompute_suggestions(state: &mut AppState) {
    const SUGGESTION_LIMIT: usize = 6;
    let partial = state.current_partial().to_owned();
    if partial.is_empty() {
        state.suggestions.clear();
        return;
    }
    // Extract room words from the last few transcript lines (recent context).
    let room_text: String = state
        .transcript
        .iter()
        .rev()
        .take(20)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    let room_words = room_words_from_text(&room_text);
    state.suggestions = suggest(&state.dict_words, &room_words, &partial, SUGGESTION_LIMIT);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Apply a completed prompt to the mapper.
/// Returns the prompt back if it was a saves-manager kind (caller handles it).
fn apply_prompt(prompt: Prompt, mapper: &mut Mapper) -> Option<Prompt> {
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
        PromptKind::RenameLayer(id) => {
            mapper.graph.set_layer_name(id, prompt.buffer);
        }
        // Saves-manager prompts: return the prompt so the caller can act on it.
        PromptKind::SaveAs | PromptKind::ConfirmDeleteSave(_) => {
            return Some(prompt);
        }
    }
    None
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

    /// Regression: re-tidy must not let `cleanup_overlaps` move a room to a cell that breaks
    /// its own satisfied compass hints. In the A129 map, #180 must sit NW of #80 and SW of #81
    /// (from `180 S 80`+`80 W 180` and `180 N 81`+`81 W 180`). `relayout_auto` places it there;
    /// before the cleanup guard, the overlap pass shoved #180 into #80's column and below it.
    #[test]
    fn retidy_keeps_180_north_west_of_80_and_south_west_of_81() {
        use mapper::direction::Direction::*;
        let mut g = mapper::graph::MapGraph::new();
        for id in [25u16, 26, 27, 74, 75, 76, 77, 78, 79, 80, 81, 88, 136, 143, 180, 193, 201, 203, 239] {
            g.upsert_room(id, "r".into());
        }
        for (o, d, dst) in [
            (180, N, 81), (81, W, 180), (180, W, 78), (78, N, 143), (143, E, 77), (77, S, 74),
            (74, S, 76), (76, W, 78), (143, W, 78), (78, S, 76), (76, N, 74), (74, E, 25),
            (25, W, 76), (74, W, 79), (79, E, 74), (25, E, 26), (26, Up, 25), (78, E, 75),
            (77, E, 239), (239, N, 77), (77, Unknown, 180), (180, S, 80), (80, W, 180),
            (80, E, 79), (79, S, 80), (79, N, 81), (81, E, 79), (80, S, 76), (76, Unknown, 180),
            (79, Unknown, 180), (75, S, 81), (75, W, 78), (75, E, 77), (239, S, 77), (77, W, 75),
            (75, N, 143), (143, S, 75), (26, Down, 27), (27, N, 136), (136, SW, 27), (27, Up, 26),
            (26, Unknown, 180), (79, W, 203), (203, W, 193), (193, E, 203), (203, E, 79),
            (203, Up, 201), (201, Down, 203), (25, Unknown, 180), (239, W, 77), (81, N, 75),
            (25, Down, 26), (75, Up, 88), (88, Down, 75), (143, Unknown, 180),
        ] {
            g.add_edge(o, d, dst);
        }
        mapper::layer::peel_region(&mut g, 27); // the user's scenario: 27/136 in their own layer
        run_tidy_pipeline(&mut g, 0);
        let p = |id: u16| g.room(id).unwrap().pos.unwrap();
        let (a, b, c) = (p(180), p(80), p(81));
        assert!(a.0 < b.0 && a.1 < b.1, "180 {a:?} must be NW of 80 {b:?}");
        assert!(a.0 < c.0 && a.1 > c.1, "180 {a:?} must be SW of 81 {c:?}");
    }

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
    fn game_focus_has_map_shortcuts() {
        let s = AppState::default(); // Game focus (story line)
        // Map navigation works without leaving the story line.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Home)), Action::Recenter));
        assert!(matches!(key_to_action(&s, key(KeyCode::PageUp)), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::PageDown)), Action::ZoomOut));
        // Retidy (Ctrl+T) is not in the direct set: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None));
        // Typing still reaches the command line (plain and shifted/capital letters).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::InputChar('N')));
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

    // ── Prompt-precedence tests ───────────────────────────────────────────────

    /// Helper: build an AppState with an active RenameRoom prompt.
    fn state_with_rename_prompt() -> AppState {
        let mut s = AppState::default();
        s.toggle_focus(); // Map
        s.select_room(Some(1));
        s.prompt = Some(crate::state::Prompt {
            kind: crate::state::PromptKind::RenameRoom(1),
            buffer: String::new(),
        });
        s
    }

    #[test]
    fn ctrl_q_quits_during_prompt() {
        let s = state_with_rename_prompt();
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('q'))),
            Action::Quit
        ));
    }

    #[test]
    fn tab_ignored_during_prompt() {
        let s = state_with_rename_prompt();
        // Tab must be absorbed (Action::None), NOT ToggleFocus.
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Tab)),
            Action::None
        ));
    }

    #[test]
    fn ctrl_s_ignored_during_prompt() {
        let s = state_with_rename_prompt();
        // Ctrl+S must be absorbed (Action::None), NOT SaveGame.
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('s'))),
            Action::None
        ));
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
    fn shift_arrows_pan_and_ctrl_arrows_nudge_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus(); // map focus
        // Shift+Arrows pan (consistent with game focus and animation playback).
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        // Nudging the selected room relocated to Ctrl+Arrows (handled globally).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::NudgeSelected(-1, 0)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::NudgeSelected(1, 0)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Up)), Action::NudgeSelected(0, -1)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Down)), Action::NudgeSelected(0, 1)));
    }

    #[test]
    fn map_focus_select_next_prev() {
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::SelectNext));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::SelectPrev));
    }

    #[test]
    fn shift_n_starts_layer_rename_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // RenameLayer is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::None));
        // Returns the action when dialog is open.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::RenameLayer));
    }

    #[test]
    fn global_shortcuts_work_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus();
        // Direct commands fire without the dialog.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('r'))), Action::RestoreGame));
        // Non-direct commands return None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('d'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::None));
        // Non-direct commands fire from the dialog.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::ExportSvg));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::ExportDot));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('d'))), Action::ExportDump));
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
    fn shift_r_in_map_focus_is_retidy() {
        let mut s = AppState::default();
        s.toggle_focus(); // → Map
        // Retidy and RenameRoom are dialog-only: return None when dialog is closed.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('R'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::None));
        // Return actions when dialog is open.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('R'))), Action::Retidy));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
    }

    #[test]
    fn retidy_rederives_clean_layout_in_auto() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::E)); // hint: 2 east of 1
        // Scramble so 2 sits WEST of 1, contradicting the hint (mimics greedy drift).
        m.graph.set_pos(1, (5, 5));
        m.graph.set_pos(2, (0, 0));
        apply_action(Action::Retidy, &mut s, &mut m);
        let p1 = m.graph.room(1).unwrap().pos.unwrap();
        let p2 = m.graph.room(2).unwrap().pos.unwrap();
        assert!(p2.0 > p1.0, "after retidy room 2 must be east of room 1: {p2:?} vs {p1:?}");
    }

    #[test]
    fn retidy_is_noop_in_manual() {
        use mapper::layout::LayoutMode;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::E));
        m.set_mode(LayoutMode::Manual);
        m.graph.set_pos(1, (5, 5));
        m.graph.set_pos(2, (0, 0)); // deliberately contradicts the hint
        apply_action(Action::Retidy, &mut s, &mut m);
        assert_eq!(m.graph.room(1).unwrap().pos, Some((5, 5)), "Manual: retidy must not move rooms");
        assert_eq!(m.graph.room(2).unwrap().pos, Some((0, 0)));
    }

    #[test]
    fn animate_tidy_captures_frames_and_lands_on_instant_retidy() {
        use mapper::direction::Direction::E;
        // Two mappers with identical scrambled input: one animated, one instant-tidied.
        let mut build = || {
            let mut m = Mapper::default();
            m.observe(1, "A", None);
            m.observe(2, "B", Some(E));
            m.observe(3, "C", Some(E));
            m.graph.set_pos(1, (5, 5));
            m.graph.set_pos(2, (0, 0));
            m.graph.set_pos(3, (2, 9));
            m
        };
        let (mut s_anim, mut m_anim) = (AppState::default(), build());
        let (mut s_inst, mut m_inst) = (AppState::default(), build());
        apply_action(Action::AnimateTidy, &mut s_anim, &mut m_anim);
        apply_action(Action::Retidy, &mut s_inst, &mut m_inst);

        let anim = s_anim.tidy_anim.expect("animation populated");
        assert_eq!(anim.frames.len(), 7, "before + 6 stages");
        assert_eq!(anim.idx, 0, "starts on the first frame");
        // Final frame and the live graph match the instant-tidy result room-for-room.
        for id in [1u16, 2, 3] {
            let inst = m_inst.graph.room(id).unwrap().pos;
            assert_eq!(anim.frames.last().unwrap().graph.room(id).unwrap().pos, inst);
            assert_eq!(m_anim.graph.room(id).unwrap().pos, inst);
        }
    }

    #[test]
    fn animate_tidy_is_noop_in_manual() {
        use mapper::layout::LayoutMode;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.set_mode(LayoutMode::Manual);
        apply_action(Action::AnimateTidy, &mut s, &mut m);
        assert!(s.tidy_anim.is_none(), "Manual: no animation started");
    }

    #[test]
    fn anim_submode_routes_transport_keys_and_exits() {
        use crate::state::{TidyAnim, TidyFrame};
        let mut s = AppState::default();
        s.focus = crate::state::Focus::Map;
        // No animation: arrows pan as usual (not stepping).
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::Pan(..)));
        // Animation active: arrows step, Space toggles, Esc exits.
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new() };
        s.tidy_anim = Some(TidyAnim::new(vec![frame("a"), frame("b")]));
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::AnimStep(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::AnimStep(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(' '))), Action::AnimTogglePlay));
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::AnimExit));
        // The map stays scrollable during playback: hjkl / Shift+arrows pan, +/- zoom.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Exit clears playback.
        apply_action(Action::AnimExit, &mut s, &mut Mapper::default());
        assert!(s.tidy_anim.is_none());
    }

    #[test]
    fn anim_step_clamps_pauses_and_holds_at_end() {
        use crate::state::{TidyAnim, TidyFrame};
        use std::time::Duration;
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new() };
        let mut a = TidyAnim::new(vec![frame("a"), frame("b"), frame("c")]);
        assert!(a.playing && a.idx == 0);
        a.step(-1); // clamps at 0, and a manual step pauses
        assert_eq!(a.idx, 0);
        assert!(!a.playing, "manual step pauses playback");
        a.step(5); // clamps to last frame
        assert_eq!(a.idx, 2);
        // A paused, end-of-range animation never advances on tick.
        assert!(!a.tick(Duration::from_millis(0)));
        assert_eq!(a.idx, 2);
    }

    #[test]
    fn prompt_flow_rename_room() {
        // Set up a mapper with one room.
        let mut mapper = Mapper::default();
        mapper.observe(1, "Dark Room", None);

        let mut state = AppState::default();
        state.toggle_focus(); // Map
        state.select_room(Some(1));

        // RenameRoom is dialog-only: 'r' returns None when dialog is closed.
        assert!(matches!(key_to_action(&state, key(KeyCode::Char('r'))), Action::None));

        // With dialog open, 'r' → RenameRoom action → prompt becomes active.
        state.hotkey_dialog = true;
        let a = key_to_action(&state, key(KeyCode::Char('r')));
        assert!(matches!(a, Action::RenameRoom));
        apply_action(a, &mut state, &mut mapper);
        // apply_action clears hotkey_dialog when opening a sub-mode
        assert!(!state.hotkey_dialog, "hotkey_dialog cleared when prompt opens");
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

    #[test]
    fn ctrl_a_toggles_alignment_overlay() {
        // toggle_alignment is dialog-only: Ctrl+A returns None when dialog closed.
        let s = AppState::default();
        assert!(!s.show_alignment, "off by default");
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::None));
        // The action itself still works when dispatched directly.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(s.show_alignment, "toggled on");
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(!s.show_alignment, "toggled off");
    }

    #[test]
    fn ctrl_p_toggles_portal_labels() {
        // toggle_portal_labels is dialog-only: Ctrl+P returns None when dialog closed.
        let s = AppState::default();
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::None
        ));
        // The action itself still works when dispatched directly.
        let mut s = AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        assert!(!s.show_portal_labels, "default off");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(s.show_portal_labels, "TogglePortalLabels turns labels on");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(!s.show_portal_labels, "TogglePortalLabels toggles back off");
    }

    #[test]
    fn bracket_keys_cycle_layer_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // CycleLayer is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::None));
        // Returns actions when dialog is open.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CycleLayer(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CycleLayer(-1)));
    }

    #[test]
    fn shift_p_peels_and_shift_m_merges_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // PeelLayer/MergeLayer are dialog-only: return None when dialog is closed.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::None));
        // Return actions when dialog is open.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::PeelLayer));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::MergeLayer));
    }

    #[test]
    fn retidy_refreshes_distorted_flags_after_layer_scoped_tidy() {
        // Regression: run_tidy_pipeline was writing back ONLY positions from the sub-graph,
        // discarding the freshly-computed distorted flags. This test fails RED before the fix
        // (the forced-true flag on a satisfied edge stays true) and GREEN after.
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        use mapper::layout::edge_is_satisfied;

        // Build a small acyclic single-layer compass graph: 1 -E-> 2 -E-> 3 with
        // reciprocal W edges so all compass edges are satisfiable.
        let mut g = MapGraph::new();
        g.upsert_room(1, "A".into());
        g.upsert_room(2, "B".into());
        g.upsert_room(3, "C".into());
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        g.add_edge(2, Direction::E, 3);
        g.add_edge(3, Direction::W, 2);

        // Force a WRONG distorted flag on index 0 (edge 1→E→2).
        // After tidy this edge will be satisfied, so the correct flag is false.
        // Before the fix the stale true remains; after the fix it is corrected to false.
        g.set_conn_distorted(0, true);

        run_tidy_pipeline(&mut g, mapper::layer::MAIN_LAYER);

        // After tidy every compass connection's distorted flag must match the geometry.
        for conn in g.connections() {
            if mapper::direction::grid_offset(conn.dir).is_some() {
                let expected = !edge_is_satisfied(&g, conn);
                assert_eq!(
                    conn.distorted, expected,
                    "distorted flag stale on edge {:?}: got {} want {}",
                    conn, conn.distorted, expected,
                );
            }
        }
    }

    #[test]
    fn retidy_only_moves_the_active_layer() {
        use mapper::graph::MapGraph;
        use mapper::direction::Direction;
        let mut g = MapGraph::new();
        // Layer 0: a 3-room tangle that relayout will move.
        g.upsert_room(1, "A".into()); g.set_pos(1, (0, 0));
        g.upsert_room(2, "B".into()); g.set_pos(2, (5, 5));
        g.add_edge(1, Direction::E, 2);
        g.add_edge(2, Direction::W, 1);
        // Layer 1: a room with a fixed position that must NOT move.
        let l = g.new_layer(Some(0), "Other".into());
        g.upsert_room(9, "X".into()); g.set_room_layer(9, l); g.set_pos(9, (3, 3));
        let _frames = run_tidy_pipeline(&mut g, l); // tidy the OTHER layer
        assert_eq!(g.room(1).unwrap().pos, Some((0, 0)), "layer-0 room 1 untouched");
        assert_eq!(g.room(2).unwrap().pos, Some((5, 5)), "layer-0 room 2 untouched");
        // Room 9 is the only room in layer l → relayout anchors it at the origin.
        assert_eq!(g.room(9).unwrap().pos, Some((0, 0)), "lone room in tidied layer is anchored");
    }

    // ── Autocomplete / Tab precedence tests ───────────────────────────────────

    #[test]
    fn tab_is_toggle_focus_with_empty_input() {
        // Game focus, empty input, no suggestions → Tab is ToggleFocus.
        let s = AppState::default(); // focus = Game, input = "", suggestions = []
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn tab_is_toggle_focus_with_input_but_no_suggestions() {
        // Game focus, non-empty partial, but no suggestions (dict not loaded) →
        // Tab is still ToggleFocus.
        let mut s = AppState::default();
        s.input = "nor".to_string();
        // suggestions is empty by default
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn tab_is_autocomplete_when_suggestions_available() {
        // Game focus, non-empty partial, suggestions populated → Tab is Autocomplete.
        let mut s = AppState::default();
        s.input = "nor".to_string();
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::Autocomplete));
    }

    #[test]
    fn tab_is_toggle_focus_in_map_focus_even_with_suggestions() {
        // Map focus: Tab always toggles focus regardless of suggestions.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.suggestions = vec!["north".to_string()]; // not relevant for map focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn autocomplete_action_replaces_partial_word() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input = "go nor".to_string();
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::Autocomplete, &mut s, &mut m);
        // "nor" should be replaced with "north" (index 0 suggestion).
        assert_eq!(s.input, "go north");
        // Index should advance to 1 for next Tab.
        assert_eq!(s.suggestion_idx, 1);
    }

    #[test]
    fn autocomplete_cycles_on_repeated_tab() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input = "go nor".to_string();
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        // First Tab: north
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input, "go north");
        assert_eq!(s.suggestion_idx, 1);
        // Second Tab: northeast
        s.input = "go nor".to_string(); // simulate user going back to partial
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input, "go northeast");
        assert_eq!(s.suggestion_idx, 0); // wrapped
    }

    #[test]
    fn typing_resets_suggestion_index() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Pre-load some suggestions and set idx > 0.
        s.input = "no".to_string();
        s.dict_words = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 1;
        // Type another character: should recompute suggestions and reset idx to 0.
        apply_action(Action::InputChar('r'), &mut s, &mut m);
        assert_eq!(s.suggestion_idx, 0);
        // Suggestions should now match "nor".
        assert!(s.suggestions.iter().any(|w| w.starts_with("nor")));
    }

    // ── Inspector toggle tests ─────────────────────────────────────────────────

    #[test]
    fn i_in_map_focus_yields_toggle_inspector() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // ToggleInspector is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::None));
        // Returns the action when dialog is open.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInspector));
    }

    #[test]
    fn i_in_game_focus_is_input_char_not_inspector() {
        let s = AppState::default(); // game focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::InputChar('i')));
    }

    #[test]
    fn toggle_inspector_opens_diagnostics_for_selected_room() {
        use crate::state::{RoomPanel, RoomPanelMode};
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Without a selected room, ToggleInspector is a no-op (room_panel stays None).
        apply_action(Action::ToggleInspector, &mut s, &mut m);
        assert!(s.room_panel.is_none(), "no room selected: panel should stay None");
        assert!(!s.show_inspector, "no room selected: show_inspector stays false");

        // With a selected room, ToggleInspector opens a Diagnostics panel.
        s.select_room(Some(42));
        apply_action(Action::ToggleInspector, &mut s, &mut m);
        assert_eq!(
            s.room_panel,
            Some(RoomPanel { id: 42, mode: RoomPanelMode::Diagnostics }),
            "should open Diagnostics panel for selected room"
        );
        assert!(s.show_inspector, "show_inspector should be true when panel opens");

        // Second toggle closes it.
        apply_action(Action::ToggleInspector, &mut s, &mut m);
        assert!(s.room_panel.is_none(), "second toggle should close the panel");
        assert!(!s.show_inspector, "show_inspector should be false after closing");
    }

    #[test]
    fn n_p_select_still_work_after_inspector_added() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::SelectNext));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::SelectPrev));
    }

    #[test]
    fn pan_keys_still_work_after_inspector_added() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('k'))), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('l'))), Action::Pan(1, 0)));
    }

    // ── Equivalence guard for the KeyMap refactor ──────────────────────────────

    /// This test encodes the CURRENT (pre-refactor) behavior of key_to_action for
    /// a representative sample across all contexts. It must pass both before and
    /// after the Task 4 refactor. If it fails after the refactor, the KeyMap
    /// defaults or lookup semantics diverge from today — fix the data, not the test.
    #[test]
    fn key_to_action_equivalence_sample() {
        use crate::state::{TidyAnim, TidyFrame};

        // ── Game focus (default) ──────────────────────────────────────────────
        let s = AppState::default(); // focus = Game
        // Direct ctrl commands work without the dialog.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('r'))), Action::RestoreGame));
        // Non-direct ctrl commands return None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('d'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('y'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('p'))), Action::None));
        // Ctrl+Arrows nudge from game focus
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::NudgeSelected(-1, 0)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::NudgeSelected(1, 0)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Up)), Action::NudgeSelected(0, -1)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Down)), Action::NudgeSelected(0, 1)));
        // Tab → ToggleFocus (no input, no suggestions)
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
        // Text entry
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')));

        // ── Map focus (dialog closed) ──────────────────────────────────────────
        let mut s = AppState::default();
        s.toggle_focus(); // Map
        // Plain arrows pan (direct)
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::Pan(0, 1)));
        // Shift arrows pan (direct)
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        // hjkl pan (direct)
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('k'))), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('l'))), Action::Pan(1, 0)));
        // Zoom (direct)
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('='))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Direct map commands
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('c'))), Action::Recenter));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::SelectNext));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::SelectPrev));
        // Dialog-only map commands: return None when dialog is closed
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('R'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('o'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('d'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('e'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::None));
        // Esc → ToggleFocus (direct, always works)
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::ToggleFocus));
        // Direct ctrl globals work in map focus
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::NudgeSelected(-1, 0)));
        // Tab → ToggleFocus in map focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));

        // ── Map focus (dialog open) ───────────────────────────────────────────
        s.hotkey_dialog = true;
        // Dialog-only commands now work
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::RenameLayer));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::PeelLayer));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::MergeLayer));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('R'))), Action::Retidy));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CycleLayer(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CycleLayer(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('o'))), Action::EditNotes));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('d'))), Action::DeleteSelectedConnection));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('e'))), Action::RelabelSelectedEdge));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInspector));
        // 'q' closes the dialog (not gallery/open-gallery)
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::CloseHotkeyDialog));
        s.hotkey_dialog = false;

        // ── Anim sub-mode ─────────────────────────────────────────────────────
        let mut s = AppState::default();
        s.focus = Focus::Map;
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new() };
        s.tidy_anim = Some(TidyAnim::new(vec![frame("a"), frame("b")]));
        // Step
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::AnimStep(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::AnimStep(1)));
        // Play/pause
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(' '))), Action::AnimTogglePlay));
        // Exit
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::AnimExit));
        assert!(matches!(key_to_action(&s, key(KeyCode::Enter)), Action::AnimExit));
        // Pan in anim: Shift+arrows + hjkl
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        // Zoom in anim
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Anim does NOT fall through to Global: unknown key → None
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::None));
    }

    #[test]
    fn gallery_submode_routes_arrow_keys() {
        use crate::state::GalleryState;
        let mut s = AppState::default();
        s.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::GalleryPrev));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::GalleryNext));
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::GalleryCategoryPrev));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::GalleryCategoryNext));
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::GalleryClose));
        assert!(matches!(key_to_action(&s, key(KeyCode::Enter)), Action::GalleryClose));
    }

    #[test]
    fn gallery_next_wraps_and_updates_symbols() {
        use crate::state::GalleryState;
        use crate::symbols::BoxStyle;
        let mut s = AppState::default();
        let n = BoxStyle::preset_names().len();
        s.gallery = Some(GalleryState { category_idx: 0, selections: [n - 1, 0, 0, 0] });
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::GalleryNext, &mut s, &mut m);
        assert_eq!(s.gallery.as_ref().unwrap().selections[0], 0, "wraps to 0");
        // symbols should be updated live
        let expected = crate::symbols::SymbolSet::resolve(&s.gallery.as_ref().unwrap().symbol_config());
        assert_eq!(s.symbols, expected);
    }

    #[test]
    fn gallery_close_clears_state() {
        use crate::state::GalleryState;
        let mut s = AppState::default();
        s.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::GalleryClose, &mut s, &mut m);
        assert!(s.gallery.is_none());
    }

    #[test]
    fn open_gallery_key_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus(); // Map
        // OpenGallery is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('g'))), Action::None));
        // Returns the action when dialog is open.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('g'))), Action::OpenGallery));
    }

    // ── Saves-manager sub-mode tests ──────────────────────────────────────────

    fn state_with_saves_open() -> AppState {
        use crate::state::{SavesState};
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;
        let mut s = AppState::default();
        s.saves = Some(SavesState {
            entries: vec![
                SaveInfo {
                    path: PathBuf::from("/tmp/default.babelmap"),
                    name: "(default)".to_string(),
                    turns: 0,
                    saved_at: String::new(),
                    is_default: true,
                },
                SaveInfo {
                    path: PathBuf::from("/tmp/named.babelmap"),
                    name: "before-troll".to_string(),
                    turns: 10,
                    saved_at: "2026-06-18T10:00:00Z".to_string(),
                    is_default: false,
                },
            ],
            selected: 0,
        });
        s
    }

    #[test]
    fn saves_submode_up_down_navigates() {
        let mut s = state_with_saves_open();
        // Down moves selection from 0 to 1.
        let a = key_to_action(&s, key(KeyCode::Down));
        assert!(matches!(a, Action::SavesNav(1)));
        apply_action(a, &mut s, &mut Mapper::default());
        assert_eq!(s.saves.as_ref().unwrap().selected, 1);
        // Up moves back to 0.
        let a = key_to_action(&s, key(KeyCode::Up));
        assert!(matches!(a, Action::SavesNav(-1)));
        apply_action(a, &mut s, &mut Mapper::default());
        assert_eq!(s.saves.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn saves_submode_s_opens_save_as_prompt() {
        let mut s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Char('s')));
        assert!(matches!(a, Action::SavesSaveAs));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.prompt.is_some(), "SavesSaveAs should open the prompt sub-mode");
        assert!(matches!(
            s.prompt.as_ref().unwrap().kind,
            crate::state::PromptKind::SaveAs
        ));
    }

    #[test]
    fn saves_submode_d_opens_confirm_delete_prompt() {
        let mut s = state_with_saves_open();
        // Select entry 1 (the named save).
        s.saves.as_mut().unwrap().selected = 1;
        let a = key_to_action(&s, key(KeyCode::Char('d')));
        assert!(matches!(a, Action::SavesDelete));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.prompt.is_some(), "SavesDelete should open the confirm prompt");
        assert!(matches!(
            s.prompt.as_ref().unwrap().kind,
            crate::state::PromptKind::ConfirmDeleteSave(_)
        ));
    }

    #[test]
    fn saves_submode_esc_closes_modal() {
        let mut s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::SavesClose));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.saves.is_none(), "Esc should close the saves modal");
    }

    #[test]
    fn saves_submode_enter_produces_saves_load() {
        let s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::SavesLoad));
    }

    #[test]
    fn ctrl_o_opens_saves_in_game_and_map_focus() {
        // open_saves is not in the direct set: Ctrl+O returns None when dialog is closed.
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::None));
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::None));
        // It fires from the dialog.
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::OpenSaves));
    }

    #[test]
    fn saves_nav_wraps_around() {
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;
        let mut s = AppState::default();
        s.saves = Some(SavesState {
            entries: vec![
                SaveInfo { path: PathBuf::from("/tmp/a.babelmap"), name: "a".into(), turns: 0, saved_at: String::new(), is_default: false },
                SaveInfo { path: PathBuf::from("/tmp/b.babelmap"), name: "b".into(), turns: 0, saved_at: String::new(), is_default: false },
            ],
            selected: 1,
        });
        // Down from last wraps to first.
        apply_action(Action::SavesNav(1), &mut s, &mut Mapper::default());
        assert_eq!(s.saves.as_ref().unwrap().selected, 0, "should wrap to 0 after last");
        // Up from first wraps to last.
        apply_action(Action::SavesNav(-1), &mut s, &mut Mapper::default());
        assert_eq!(s.saves.as_ref().unwrap().selected, 1, "should wrap to last");
    }

    // ── Hotkey dialog dispatch tests ──────────────────────────────────────────

    #[test]
    fn dialog_closed_dialog_only_cmd_returns_none() {
        // In map focus with dialog closed, a dialog-only command returns None.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // Retidy is bound to Shift+R in Map context but is NOT direct.
        assert!(matches!(
            key_to_action(&s, shift(KeyCode::Char('R'))),
            Action::None
        ));
        // ToggleInspector ('i') is also dialog-only.
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Char('i'))),
            Action::None
        ));
    }

    #[test]
    fn dialog_closed_direct_cmd_still_works() {
        // In map focus with dialog closed, direct commands still work.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // SelectNext ('n') is in the direct set.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::SelectNext));
        // PanLeft ('h') is in the direct set.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        // Recenter ('c') is in the direct set.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('c'))), Action::Recenter));
    }

    #[test]
    fn prefix_opens_hotkey_dialog_action() {
        // Ctrl+K in any non-dialog state → OpenHotkeyDialog.
        let s = AppState::default(); // game focus
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('k'))),
            Action::OpenHotkeyDialog
        ));
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('k'))),
            Action::OpenHotkeyDialog
        ));
    }

    #[test]
    fn prefix_closes_hotkey_dialog_action() {
        // Ctrl+K when dialog is open → CloseHotkeyDialog.
        let mut s = AppState::default();
        s.hotkey_dialog = true;
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('k'))),
            Action::CloseHotkeyDialog
        ));
    }

    #[test]
    fn q_closes_hotkey_dialog_action() {
        // 'q' with no modifiers when dialog is open → CloseHotkeyDialog.
        let mut s = AppState::default();
        s.hotkey_dialog = true;
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Char('q'))),
            Action::CloseHotkeyDialog
        ));
    }

    #[test]
    fn dialog_open_dialog_only_cmd_fires() {
        // When dialog is open, Shift+R in map focus fires Retidy.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('R'))), Action::Retidy));
        // ToggleInspector fires too.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInspector));
    }

    #[test]
    fn apply_open_hotkey_dialog_sets_flag() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.hotkey_dialog);
        apply_action(Action::OpenHotkeyDialog, &mut s, &mut m);
        assert!(s.hotkey_dialog);
        apply_action(Action::CloseHotkeyDialog, &mut s, &mut m);
        assert!(!s.hotkey_dialog);
    }

    #[test]
    fn open_saves_clears_hotkey_dialog() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.hotkey_dialog = true;
        apply_action(Action::OpenSaves, &mut s, &mut m);
        assert!(!s.hotkey_dialog, "OpenSaves should clear the hotkey dialog");
    }

    // ── is_direct as sole direct-vs-prefix determiner ─────────────────────────

    /// Promoting a command via config makes it reachable directly (dialog closed).
    #[test]
    fn direct_config_promotes_retidy_to_direct() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: Some(vec!["retidy".into()]),
            group: vec![HotkeyGroupConfig {
                title: "Layout".into(),
                commands: vec!["retidy".into()],
            }],
        };
        let (layout, _) = crate::keymap::HotkeyLayout::resolve(&cfg);
        let mut s = AppState::default();
        s.hotkeys = layout;
        s.focus = Focus::Map;
        // With dialog closed: retidy is now direct → fires.
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::Retidy),
            "promoted retidy should fire directly (dialog closed)"
        );
    }

    /// With the default layout (retidy NOT in direct): closed dialog → None,
    /// open dialog → Retidy.
    #[test]
    fn default_layout_retidy_is_dialog_only() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // Closed dialog: Ctrl+T returns None.
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None),
            "retidy should NOT fire directly with default layout (dialog closed)"
        );
        // Open dialog: Ctrl+T fires Retidy.
        s.hotkey_dialog = true;
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::Retidy),
            "retidy should fire from the hotkey dialog"
        );
    }

    // ── mouse_to_action tests ─────────────────────────────────────────────────

    fn mouse_event(
        kind: crossterm::event::MouseEventKind,
        col: u16,
        row: u16,
        modifiers: KeyModifiers,
    ) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent { kind, column: col, row, modifiers }
    }

    fn map_rect() -> ratatui::layout::Rect {
        ratatui::layout::Rect::new(0, 0, 80, 40)
    }

    fn story_rect() -> ratatui::layout::Rect {
        ratatui::layout::Rect::new(80, 0, 40, 40)
    }

    /// Build a room_rects slice for a single room at a given cell using Compact zoom.
    fn room_rects_for_compact(id: u16, cell: (i32, i32), area: ratatui::layout::Rect) -> Vec<(mapper::graph::RoomId, ratatui::layout::Rect)> {
        use crate::state::{AppState, Zoom};
        use crate::render::map::room_screen_rects;
        use mapper::graph::MapGraph;
        use mapper::render::render_layer;
        use mapper::layer::MAIN_LAYER;

        let mut g = MapGraph::new();
        g.upsert_room(id, "Room".into());
        g.set_pos(id, cell);

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        s.scroll = (0, 0);

        let rm = render_layer(&g, MAIN_LAYER);
        room_screen_rects(&rm, &s, area)
    }

    #[test]
    fn left_down_on_room_cell_produces_show_room_info() {
        use crossterm::event::MouseEventKind;
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step = (12, 5)
        s.scroll = (0, 0);

        // Room 1 at cell (0,0). Build room_rects using render pipeline.
        let rects = room_rects_for_compact(1, (0, 0), map_rect());

        // Click at (0,0) which is inside the Compact box (8x3).
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 0, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects);
        assert!(
            matches!(action, Action::ShowRoomInfo(1)),
            "left-down on room cell should produce ShowRoomInfo(1), got {:?}", action
        );
    }

    #[test]
    fn right_down_on_room_cell_produces_show_room_diagnostics() {
        use crossterm::event::MouseEventKind;
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        s.scroll = (0, 0);

        let rects = room_rects_for_compact(2, (0, 0), map_rect());

        let m = mouse_event(MouseEventKind::Down(MouseButton::Right), 0, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects);
        assert!(
            matches!(action, Action::ShowRoomDiagnostics(2)),
            "right-down on room cell should produce ShowRoomDiagnostics(2), got {:?}", action
        );
    }

    #[test]
    fn left_down_on_gutter_produces_close_room_panel() {
        use crossterm::event::MouseEventKind;
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step = (12, 5)
        s.scroll = (0, 0);
        // Room is at cell (0,0), box is 8 wide so cols 0..8 hit the room.
        // Click at col 50 misses the room entirely.
        let rects = room_rects_for_compact(1, (0, 0), map_rect());

        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 50, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects);
        assert!(
            matches!(action, Action::CloseRoomPanel),
            "left-down on gutter should produce CloseRoomPanel, got {:?}", action
        );
    }

    #[test]
    fn scroll_up_in_map_produces_pan_up() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[]);
        assert!(matches!(action, Action::Pan(0, -1)), "scroll up in map without modifier -> Pan(0,-1)");
    }

    #[test]
    fn scroll_down_in_map_produces_pan_down() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollDown, 10, 10, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[]);
        assert!(matches!(action, Action::Pan(0, 1)), "scroll down in map without modifier -> Pan(0,1)");
    }

    #[test]
    fn scroll_up_with_shift_pans_left() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::SHIFT);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[]);
        assert!(matches!(action, Action::Pan(-1, 0)), "scroll up + Shift -> Pan(-1,0)");
    }

    #[test]
    fn scroll_up_with_ctrl_zooms_in() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::CONTROL);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[]);
        assert!(matches!(action, Action::ZoomIn), "scroll up + Ctrl -> ZoomIn");
    }

    #[test]
    fn scroll_in_story_produces_transcript_scroll() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m_up = mouse_event(MouseEventKind::ScrollUp, 85, 5, KeyModifiers::NONE);
        let action_up = mouse_to_action(&s, m_up, map_rect(), story_rect(), &[]);
        assert!(matches!(action_up, Action::TranscriptScroll(-1)), "scroll up in story -> TranscriptScroll(-1)");

        let m_dn = mouse_event(MouseEventKind::ScrollDown, 85, 5, KeyModifiers::NONE);
        let action_dn = mouse_to_action(&s, m_dn, map_rect(), story_rect(), &[]);
        assert!(matches!(action_dn, Action::TranscriptScroll(1)), "scroll down in story -> TranscriptScroll(1)");
    }

    #[test]
    fn middle_down_produces_begin_drag_pan() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::Down(MouseButton::Middle), 20, 15, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[]);
        assert!(matches!(action, Action::BeginDragPan(20, 15)), "middle-down -> BeginDragPan");
    }

    #[test]
    fn middle_drag_and_up_produce_drag_actions() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let drag = mouse_event(MouseEventKind::Drag(MouseButton::Middle), 25, 18, KeyModifiers::NONE);
        let up = mouse_event(MouseEventKind::Up(MouseButton::Middle), 25, 18, KeyModifiers::NONE);
        assert!(matches!(mouse_to_action(&s, drag, map_rect(), story_rect(), &[]), Action::DragPanTo(25, 18)));
        assert!(matches!(mouse_to_action(&s, up, map_rect(), story_rect(), &[]), Action::EndDragPan));
    }

    // ── Drag-pan accumulator tests ────────────────────────────────────────────

    #[test]
    fn drag_pan_accumulates_and_pans_at_step_boundary() {
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step_w=12, step_h=5
        let mut m = Mapper::default();

        // Begin at (10, 10).
        apply_action(Action::BeginDragPan(10, 10), &mut s, &mut m);
        assert!(s.drag.is_some(), "drag state should be set after BeginDragPan");

        // Move less than one step_w (11 columns) — no pan yet, scroll unchanged.
        apply_action(Action::DragPanTo(21, 10), &mut s, &mut m); // dx=11 < step_w=12
        assert_eq!(s.scroll, (0, 0), "sub-step move should not pan");

        // Move one more column to cross step_w=12 total.
        apply_action(Action::DragPanTo(22, 10), &mut s, &mut m); // dx=1, total acc=12
        // Grab-and-drag: dragging right means content moves right (scroll decreases).
        assert_eq!(s.scroll.0, -1, "crossing step_w rightward should scroll left by 1");
        assert_eq!(s.scroll.1, 0, "y scroll should be unchanged");
    }

    #[test]
    fn drag_pan_sub_step_movement_does_not_pan() {
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Boxes; // step_w=19, step_h=11
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(0, 0), &mut s, &mut m);
        // Move 5 cols right — less than step_w=19.
        apply_action(Action::DragPanTo(5, 0), &mut s, &mut m);
        assert_eq!(s.scroll, (0, 0), "sub-step movement should not pan");
    }

    #[test]
    fn drag_pan_grab_and_drag_direction() {
        // Drag LEFT should scroll RIGHT (content moves right = scroll.0 increases).
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step_w=12
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(20, 0), &mut s, &mut m);
        // Drag left by 12+ columns.
        apply_action(Action::DragPanTo(8, 0), &mut s, &mut m); // dx = -12
        assert_eq!(s.scroll.0, 1, "dragging left should scroll right (content follows grab)");
    }

    #[test]
    fn end_drag_pan_clears_state() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::BeginDragPan(0, 0), &mut s, &mut m);
        assert!(s.drag.is_some());
        apply_action(Action::EndDragPan, &mut s, &mut m);
        assert!(s.drag.is_none(), "EndDragPan should clear drag state");
    }

    #[test]
    fn show_room_info_sets_focus_to_map() {
        // ShowRoomInfo should switch focus to Map so the room is rendered as selected.
        let mut s = AppState::default(); // starts as Focus::Game
        assert_eq!(s.focus, Focus::Game);
        let mut m = Mapper::default();
        apply_action(Action::ShowRoomInfo(1), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Map, "ShowRoomInfo must set focus to Map");
        assert_eq!(s.selected_room, Some(1));
    }

    #[test]
    fn show_room_diagnostics_sets_focus_to_map() {
        // ShowRoomDiagnostics should switch focus to Map so the room renders selected.
        let mut s = AppState::default(); // starts as Focus::Game
        let mut m = Mapper::default();
        apply_action(Action::ShowRoomDiagnostics(2), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Map, "ShowRoomDiagnostics must set focus to Map");
        assert_eq!(s.selected_room, Some(2));
    }

    // ── should_bg_tidy ────────────────────────────────────────────────────────

    #[test]
    fn should_bg_tidy_off_always_false() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(!should_bg_tidy(BackgroundTidy::Off, true, true, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::Off, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_every_room_follows_new_room() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, true, false, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::EveryRoom, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_on_overlap_follows_overlap() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::OnOverlap, false, true, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::OnOverlap, true, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_debounced_fires_every_k_new_rooms() {
        use crate::config::{BackgroundTidy, BG_TIDY_DEBOUNCE};
        let mut c = 0u32;
        // First K-1 new rooms should not fire.
        for _ in 0..BG_TIDY_DEBOUNCE - 1 {
            assert!(!should_bg_tidy(BackgroundTidy::Debounced, true, false, &mut c));
        }
        // K-th new room fires and resets counter.
        assert!(should_bg_tidy(BackgroundTidy::Debounced, true, false, &mut c));
        assert_eq!(c, 0, "counter resets after Debounced fires");
        // No new room: never fires.
        assert!(!should_bg_tidy(BackgroundTidy::Debounced, false, false, &mut c));
    }

    // ── tidy_layer_silent ─────────────────────────────────────────────────────

    #[test]
    fn tidy_layer_silent_single_room_noop() {
        // A single-room layer should not panic and leave the room with a position.
        let mut g = mapper::graph::MapGraph::new();
        g.upsert_room(1, "Room".into());
        tidy_layer_silent(&mut g, 0);
        // Room should still exist.
        assert!(g.room(1).is_some());
    }

    #[test]
    fn tidy_layer_silent_leaves_graph_in_same_final_state_as_run_tidy_pipeline() {
        // Build a small two-room graph; run both paths and compare final positions.
        use mapper::direction::Direction;
        let make_graph = || {
            let mut g = mapper::graph::MapGraph::new();
            g.upsert_room(1, "A".into());
            g.upsert_room(2, "B".into());
            g.add_edge(1, Direction::E, 2);
            g.add_edge(2, Direction::W, 1);
            g
        };

        let mut g_pipeline = make_graph();
        run_tidy_pipeline(&mut g_pipeline, 0);

        let mut g_silent = make_graph();
        tidy_layer_silent(&mut g_silent, 0);

        let pos = |g: &mapper::graph::MapGraph, id| g.room(id).and_then(|r| r.pos);
        assert_eq!(pos(&g_pipeline, 1), pos(&g_silent, 1), "room 1 position must match");
        assert_eq!(pos(&g_pipeline, 2), pos(&g_silent, 2), "room 2 position must match");
    }
}
