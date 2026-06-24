//! Input → `Action` mapping and application.
//!
//! # Focus routing
//! `key_to_action` applies bindings in this strict precedence order:
//! 1. Ctrl+Q / Ctrl+C → Quit (always wins, even during a prompt).
//! 2. Prompt active → route to prompt only; all other keys (Tab, Ctrl+S/R/E/L,
//!    …) are absorbed as `Action::None` so the prompt cannot be escaped by a
//!    global shortcut.
//! 3. Remaining globals (Ctrl+S/R/E/L, Tab) — only reached when no prompt.
//! 4. Per-focus routing (Game / Map).
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

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    /// Toggle the full-screen help overlay.
    ToggleHelp,
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
}

// ── key_to_action ─────────────────────────────────────────────────────────────

/// Map a crossterm `KeyEvent` to an `Action` given the current `AppState`.
///
/// Routing order (preserved from the original hardcoded dispatch):
/// 1. **Quit** — Ctrl+Q / Ctrl+C → `Quit`, hardwired, always wins.
/// 2. **Prompt active** — all input consumed by the prompt sub-mode; no
///    global shortcuts escape (Tab, Ctrl+S, …) are absorbed as `None`.
/// 3. **Tidy-anim active** — KeyMap lookup in `Context::Anim`; no fallthrough
///    to Global.  Unmatched keys → `None`.
/// 4. **Tab** (no modifiers) — stateful autocomplete-or-ToggleFocus special
///    case, hardwired exactly as before.
/// 5. **Ctrl modifier** — KeyMap lookup in `Context::Global`; unmatched → `None`.
/// 6. **Game focus** — `game_key_to_action` (text entry, hardwired);
///    if it returns `None`, fall through to a `Context::Global` KeyMap lookup
///    so that non-ctrl Global bindings (e.g. F1→ToggleHelp) reach Game focus.
/// 7. **Map focus** — KeyMap lookup in `Context::Map` (falls through to Global).
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

    // 3b. Gallery sub-mode: when gallery is open, route to gallery keys.
    if state.gallery.is_some() {
        return gallery_key_to_action(key);
    }

    // 3c. Saves-manager sub-mode: when saves modal is open, route to saves keys.
    if state.saves.is_some() {
        return saves_key_to_action(key);
    }

    // 4. Tab (no modifiers): stateful autocomplete-or-ToggleFocus (hardwired).
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

    // 5. Ctrl modifier: Global KeyMap lookup; unmatched → None.
    if ctrl {
        let spec = KeySpec::from_key_event(key);
        return match state.keymap.lookup(&spec, Context::Global) {
            Some(cmd) => cmd.to_action(),
            None => Action::None,
        };
    }

    // 6 & 7. Per-focus routing.
    let spec = KeySpec::from_key_event(key);
    match state.focus {
        Focus::Game => {
            // Text entry is hardwired (printable chars, Enter, Backspace, Shift+Arrows,
            // Home, PageUp/Down).  Non-printable / unmatched keys fall through to a
            // Global KeyMap lookup so that e.g. F1→ToggleHelp is reachable from Game.
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
            // Map context falls through to Global on miss.
            match state.keymap.lookup(&spec, Context::Map) {
                Some(cmd) => cmd.to_action(),
                None => Action::None,
            }
        }
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
        Action::ToggleInspector => state.show_inspector = !state.show_inspector,

        Action::RenameRoom => {
            if let Some(id) = state.selected_room {
                state.prompt = Some(Prompt {
                    kind: PromptKind::RenameRoom(id),
                    buffer: String::new(),
                });
            }
        }
        Action::RenameLayer => {
            let layer = state.active_layer(&mapper.graph);
            let current_name = mapper.graph.layer_name(layer).to_owned();
            state.prompt = Some(Prompt {
                kind: PromptKind::RenameLayer(layer),
                buffer: current_name,
            });
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

        Action::ToggleHelp => {
            state.show_help = !state.show_help;
        }

        // ── Saves-manager actions ─────────────────────────────────────────────

        Action::OpenSaves => {
            // The list must be populated by the caller (main.rs has dir + ifid).
            // apply_action only sets up the state; the caller refreshes the list
            // via AppState::open_saves_modal after apply_action returns.
            // If already open, do nothing.
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
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::Retidy));
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
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::RenameLayer));
    }

    #[test]
    fn global_shortcuts_work_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('r'))), Action::RestoreGame));
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
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('R'))), Action::Retidy));
        // plain 'r' is still RenameRoom (no clash with the new shift binding).
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

    #[test]
    fn ctrl_a_toggles_alignment_overlay() {
        let s = AppState::default();
        assert!(!s.show_alignment, "off by default");
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::ToggleAlignment));
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(s.show_alignment, "toggled on");
        apply_action(Action::ToggleAlignment, &mut s, &mut m);
        assert!(!s.show_alignment, "toggled off");
    }

    #[test]
    fn ctrl_p_toggles_portal_labels() {
        let s = AppState::default();
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('p'))),
            Action::TogglePortalLabels
        ));
        let mut s = AppState::default();
        let mut m = mapper::mapper::Mapper::default();
        assert!(!s.show_portal_labels, "default off");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(s.show_portal_labels, "Ctrl+P turns labels on");
        apply_action(Action::TogglePortalLabels, &mut s, &mut m);
        assert!(!s.show_portal_labels, "Ctrl+P toggles back off");
    }

    #[test]
    fn bracket_keys_cycle_layer_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CycleLayer(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CycleLayer(-1)));
    }

    #[test]
    fn shift_p_peels_and_shift_m_merges_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
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
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInspector));
    }

    #[test]
    fn i_in_game_focus_is_input_char_not_inspector() {
        let s = AppState::default(); // game focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::InputChar('i')));
    }

    #[test]
    fn toggle_inspector_flips_show_inspector() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.show_inspector, "off by default");
        apply_action(Action::ToggleInspector, &mut s, &mut m);
        assert!(s.show_inspector, "toggled on");
        apply_action(Action::ToggleInspector, &mut s, &mut m);
        assert!(!s.show_inspector, "toggled off");
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
        // Ctrl globals work from game focus
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('r'))), Action::RestoreGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::ExportSvg));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::ExportDot));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('d'))), Action::ExportDump));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::CycleLayout));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::Retidy));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('y'))), Action::AnimateTidy));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::ToggleAlignment));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('p'))), Action::TogglePortalLabels));
        // Ctrl+Arrows nudge from game focus
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::NudgeSelected(-1, 0)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::NudgeSelected(1, 0)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Up)), Action::NudgeSelected(0, -1)));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Down)), Action::NudgeSelected(0, 1)));
        // Tab → ToggleFocus (no input, no suggestions)
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
        // Text entry
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')));

        // ── Map focus ─────────────────────────────────────────────────────────
        let mut s = AppState::default();
        s.toggle_focus(); // Map
        // Plain arrows pan
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::Pan(0, 1)));
        // Shift arrows pan
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::Pan(1, 0)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
        // hjkl pan
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('k'))), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('l'))), Action::Pan(1, 0)));
        // Zoom
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('='))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Map commands
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('c'))), Action::Recenter));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::SelectNext));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::SelectPrev));
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
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::ToggleFocus));
        // Map falls through to Global: ctrl globals work in map focus
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::SaveGame));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::NudgeSelected(-1, 0)));
        // Tab → ToggleFocus in map focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));

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
        // Game focus.
        let s = AppState::default();
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::OpenSaves));
        // Map focus.
        let mut s = AppState::default();
        s.toggle_focus();
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
}
