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

// ── VerbMenuNavKind ───────────────────────────────────────────────────────────

/// Navigation kind for `Action::VerbMenuNav`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerbMenuNavKind {
    /// Move the selection up by one in the current pane.
    Up,
    /// Move the selection down by one in the current pane.
    Down,
    /// Switch to the next pane (Tab / Right).
    NextPane,
    /// Switch to the previous pane (Shift+Tab / Left).
    PrevPane,
}

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
    /// Jump to the next (+1) or previous (-1) stage_start frame in the animation.
    AnimStageJump(i32),
    /// Zoom the map in (more detail).
    ZoomIn,
    /// Zoom the map out (less detail).
    ZoomOut,
    /// Reset zoom to the default level (Boxes) and clear the char-pan offset.
    ZoomReset,
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
    /// Export all current settings to the personal style file and repoint config
    /// (handled by main.rs); leaves the gallery open.
    GalleryExportStyle,
    /// Toggle the inventory strip at the bottom of the story pane.
    ToggleInventory,
    /// Cycle the UI layout in reverse (Split → MapFull → TranscriptFull → Split).
    CycleLayoutReverse,
    /// Open a confirmation prompt to reset the game to its opening state (keeps map).
    ResetGame,
    /// Open the verb/item token-palette modal, building the noun list from the current room and inventory.
    OpenVerbMenu,
    /// Navigate within the verb menu: `Tab`/`Left`/`Right` switches pane; `Up`/`Down` moves selection.
    VerbMenuNav(VerbMenuNavKind),
    /// Pick the currently-selected token: append it (+ a space) to `state.input`.
    VerbMenuPick,
    /// Close the verb menu, leaving `state.input` intact.
    VerbMenuClose,
    /// Open the config screen modal.
    OpenConfig,
    /// Navigate the config screen by delta (-1 = up, +1 = down).
    ConfigNav(i32),
    /// Toggle the selected bool field in the working config.
    ConfigToggle,
    /// Cycle an enum/choice field in the working config by delta (-1 or +1).
    ConfigCycle(i32),
    /// Begin text-editing the selected path field.
    ConfigEdit,
    /// Save the working config: apply to state.config, re-resolve symbols/colors, write file.
    ConfigSave,
    /// Cancel the config screen without saving.
    ConfigCancel,
    /// Open the file browser in PickDir mode to choose a directory for export.
    SavesExport,
    /// Open the file browser in PickFile mode to import a .qzl/.sav file.
    SavesImport,
    /// Navigate the file browser by delta (-1 = up, +1 = down).
    FbNav(i32),
    /// Activate the selected file-browser entry (cd into dir or import file).
    FbEnter,
    /// Choose the current directory as the export target (PickDir mode).
    FbChooseDir,
    /// Close the file browser without acting.
    FbClose,
    /// No binding found — no-op.
    None,
    // ── Mouse actions ─────────────────────────────────────────────────────────
    /// Activate a specific pane (left-click on pane background).
    ActivatePane(crate::state::Focus),
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
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Left => return Action::AnimStageJump(-1),
                KeyCode::Right => return Action::AnimStageJump(1),
                _ => {}
            }
        }
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

    // 5.1. File-browser sub-mode: when the browser is open, route to browser keys.
    if state.file_browser.is_some() {
        return filebrowser_key_to_action(key);
    }

    // 5.5. Verb-menu sub-mode: when the token palette is open, route to verb-menu keys.
    if state.verb_menu.is_some() {
        return verb_menu_key_to_action(key);
    }

    // 5.7. Config-screen sub-mode: when config screen is open, route to config keys.
    if state.config_screen.is_some() {
        return config_screen_key_to_action(key);
    }

    // 6. Hotkey dialog open: route to dialog handler.
    if state.hotkey_dialog {
        return hotkey_dialog_key_to_action(state, key);
    }

    // 6.5. Room panel close: Esc while a panel is open.
    // Comes after steps 2-6 (prompt/anim/gallery/saves/hotkey_dialog checks) so those
    // modes still take priority, but before the prefix key and normal dispatch.
    if state.room_panel.is_some() && key.modifiers == KeyModifiers::NONE {
        if key.code == KeyCode::Esc {
            return Action::CloseRoomPanel;
        }
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

/// Test whether (col, row) is inside `rect`.
fn hit(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
}

/// Per-modal button-to-action mapping for the config screen.
/// Maps a `ButtonId` click (or the close [X] hit) to the appropriate `Action`.
fn config_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::ConfigCancel);
        }
    }

    // Check buttons
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Save   => Action::ConfigSave,
                ButtonId::Cancel => Action::ConfigCancel,
                _                => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the saves manager.
fn saves_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::SavesClose);
        }
    }

    // Check buttons: Done → SavesClose
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::SavesClose,
                _              => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the file browser.
fn filebrowser_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::FbClose);
        }
    }

    // Check buttons: Done → FbClose
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::FbClose,
                _              => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the verb menu.
fn verbmenu_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::VerbMenuClose);
        }
    }

    // Check buttons: Done → VerbMenuClose
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::VerbMenuClose,
                _              => Action::None,
            });
        }
    }

    None
}

/// Per-modal action mapping for the room-info panel ([X] → CloseRoomPanel).
fn roominfo_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::CloseRoomPanel);
        }
    }
    None
}

/// Per-modal action mapping for the inspector panel ([X] → CloseRoomPanel).
fn inspector_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::CloseRoomPanel);
        }
    }
    None
}

/// Per-modal action mapping for the tidy panel ([X] → AnimExit).
fn tidy_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::AnimExit);
        }
    }
    None
}

/// Per-modal button-to-action mapping for the hotkey dialog.
fn hotkeys_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::CloseHotkeyDialog);
        }
    }

    // Check buttons: Done → CloseHotkeyDialog
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::CloseHotkeyDialog,
                _              => Action::None,
            });
        }
    }

    None
}

/// Per-modal button-to-action mapping for the gallery.
fn gallery_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::GalleryClose);
        }
    }

    // Check buttons: Done → GalleryClose
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Done => Action::GalleryClose,
                _              => Action::None,
            });
        }
    }

    None
}

/// Map a crossterm `MouseEvent` to an `Action` given the current `AppState`, the
/// bounding rects of the map and story panes, the pre-computed room screen
/// rects (needed for pixel-accurate room hit-testing on left/right clicks), and
/// the active dialog chrome rects (if a dialog is open).
///
/// When `dialog` is `Some`, dialog hit-testing runs FIRST:
/// - close [X] click → the active modal's close action
/// - button click → the button's mapped action
/// - any click OUTSIDE the dialog `area` → swallowed (Action::None)
/// Only when no dialog is open does normal map/room routing apply.
///
/// Returns `Action::None` for events outside both panes or with no binding.
pub fn mouse_to_action(
    state: &AppState,
    m: MouseEvent,
    map: ratatui::layout::Rect,
    story: ratatui::layout::Rect,
    room_rects: &[(mapper::graph::RoomId, ratatui::layout::Rect)],
    dialog: &Option<crate::render::dialog::DialogRects>,
) -> Action {
    let col = m.column;
    let row = m.row;
    let ctrl = m.modifiers.contains(KeyModifiers::CONTROL);
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);

    // ── Dialog chrome hit-testing (checked FIRST) ─────────────────────────────
    if let Some(rects) = dialog {
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            // Corner overlays (room panel, tidy panel): only intercept the [X] click;
            // all other clicks fall through to normal map/room routing below.
            // But if a centered modal is also open (stacked on top), it takes
            // priority and must swallow all outside clicks.
            let centered_open = state.gallery.is_some() || state.config_screen.is_some()
                || state.saves.is_some() || state.file_browser.is_some()
                || state.verb_menu.is_some() || state.hotkey_dialog;
            let is_corner_overlay = !centered_open
                && (state.room_panel.is_some() || state.tidy_anim.is_some());

            if state.gallery.is_some() {
                if let Some(action) = gallery_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.config_screen.is_some() {
                if let Some(action) = config_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.saves.is_some() {
                if let Some(action) = saves_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.file_browser.is_some() {
                if let Some(action) = filebrowser_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.verb_menu.is_some() {
                if let Some(action) = verbmenu_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.hotkey_dialog {
                if let Some(action) = hotkeys_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.room_panel.as_ref().map(|p| matches!(p.mode, crate::state::RoomPanelMode::Info)).unwrap_or(false) {
                if let Some(action) = roominfo_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.room_panel.as_ref().map(|p| matches!(p.mode, crate::state::RoomPanelMode::Diagnostics)).unwrap_or(false) {
                if let Some(action) = inspector_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.tidy_anim.is_some() {
                if let Some(action) = tidy_dialog_action(rects, col, row) {
                    return action;
                }
            }

            // Corner overlays: don't swallow other clicks — let normal routing handle them.
            if is_corner_overlay {
                // fall through to normal routing below
            } else {
                // Centered modal: swallow all other clicks.
                return Action::None;
            }
        } else {
            // For non-left-click events (wheel/drag): swallow unless a corner overlay
            // is active and no centered modal is stacked on top.
            let centered_open = state.gallery.is_some() || state.config_screen.is_some()
                || state.saves.is_some() || state.file_browser.is_some()
                || state.verb_menu.is_some() || state.hotkey_dialog;
            let is_corner_overlay = !centered_open
                && (state.room_panel.is_some() || state.tidy_anim.is_some());
            if !is_corner_overlay {
                return Action::None;
            }
        }
    }

    // ── Normal routing (no dialog open) ──────────────────────────────────────

    let in_map = map.width > 0 && map.height > 0
        && col >= map.x && col < map.right()
        && row >= map.y && row < map.bottom();
    let in_story = story.width > 0 && story.height > 0
        && col >= story.x && col < story.right()
        && row >= story.y && row < story.bottom();

    match m.kind {
        // ── Left-click in story: activate game pane ───────────────────────────
        MouseEventKind::Down(MouseButton::Left) if in_story => {
            Action::ActivatePane(crate::state::Focus::Game)
        }
        // ── Left-click in map ─────────────────────────────────────────────────
        MouseEventKind::Down(MouseButton::Left) if in_map => {
            match room_at_screen(room_rects, col, row) {
                // Room hit: show its info panel (apply_action also sets Focus::Map).
                Some(id) => Action::ShowRoomInfo(id),
                // Empty map gutter: activate map focus and close any open panel.
                None => Action::ActivatePane(crate::state::Focus::Map),
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
    // ESC always closes the hotkey dialog (same as [X]).
    if key.code == KeyCode::Esc {
        return Action::CloseHotkeyDialog;
    }

    let spec = KeySpec::from_key_event(key);

    // Prefix key closes the dialog.
    if spec == state.hotkeys.prefix {
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
        KeyCode::Char('e') if key.modifiers == KeyModifiers::NONE => Action::SavesExport,
        KeyCode::Char('i') if key.modifiers == KeyModifiers::NONE => Action::SavesImport,
        KeyCode::Esc => Action::SavesClose,
        _ => Action::None,
    }
}

// ── Internal: file-browser key routing ───────────────────────────────────────

/// Hardwired file-browser sub-mode keys.
fn filebrowser_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::FbNav(-1),
        KeyCode::Down => Action::FbNav(1),
        KeyCode::Enter => Action::FbEnter,
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::FbChooseDir,
        KeyCode::Esc => Action::FbClose,
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
        KeyCode::Esc => Action::GalleryClose,
        KeyCode::Char('o') | KeyCode::Char('O') => Action::GalleryExportStyle,
        _ => Action::None,
    }
}

// ── Internal: verb-menu key routing ──────────────────────────────────────────

/// Hardwired verb-menu sub-mode keys (not rebindable).
///
/// Tab / Right  → next pane; Shift+Tab / Left → prev pane;
/// Up / Down    → move within pane;
/// Enter/Space  → pick selected token;
/// Esc          → close menu (q freed).
fn verb_menu_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Tab => Action::VerbMenuNav(VerbMenuNavKind::NextPane),
        KeyCode::BackTab => Action::VerbMenuNav(VerbMenuNavKind::PrevPane),
        KeyCode::Right => Action::VerbMenuNav(VerbMenuNavKind::NextPane),
        KeyCode::Left => Action::VerbMenuNav(VerbMenuNavKind::PrevPane),
        KeyCode::Up => Action::VerbMenuNav(VerbMenuNavKind::Up),
        KeyCode::Down => Action::VerbMenuNav(VerbMenuNavKind::Down),
        KeyCode::Enter | KeyCode::Char(' ') => Action::VerbMenuPick,
        KeyCode::Esc => Action::VerbMenuClose,
        _ => Action::None,
    }
}

// ── Internal: config-screen key routing ──────────────────────────────────────

fn config_screen_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::ConfigNav(-1),
        KeyCode::Down => Action::ConfigNav(1),
        KeyCode::Left => Action::ConfigCycle(-1),
        KeyCode::Right => Action::ConfigCycle(1),
        KeyCode::Char(' ') | KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
            Action::ConfigToggle
        }
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::ConfigSave,
        KeyCode::Esc => Action::ConfigCancel,
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
    use crate::render::map::{cleanup_overlaps_observed, compact_empty_lines_observed, repair_directional_hints_observed, stack_updown_rooms_observed};
    use crate::state::TidyFrame;
    use mapper::layout::TidyStats;

    const MAX_TIDY_FRAMES: usize = 2000;

    let mut sub = graph.layer_subgraph(layer);
    let mut frames: Vec<TidyFrame> = Vec::new();

    let mut pipe_overlaps: u32 = 0;
    let mut pipe_hints: u32 = 0;
    let mut pipe_rooms_moved: u32 = 0;
    let mut pipe_constraints: u32 = 0;

    // "before" frame
    frames.push(TidyFrame {
        label: "before".into(),
        graph: sub.clone(),
        description: "Initial state before tidy pipeline.".into(),
        stats: TidyStats::default(),
        stage_start: true,
    });

    // Layout stages via relayout_auto_observed
    mapper::layout::relayout_auto_observed(&mut sub, Some(&mut |g: &mapper::graph::MapGraph, label: &str, desc: &str, s: &TidyStats| {
        pipe_rooms_moved = s.rooms_moved;
        pipe_constraints = s.constraints_dropped;
        if frames.len() < MAX_TIDY_FRAMES {
            frames.push(TidyFrame {
                label: label.into(),
                graph: g.clone(),
                description: desc.into(),
                stats: TidyStats {
                    rooms_moved: s.rooms_moved,
                    constraints_dropped: s.constraints_dropped,
                    overlaps_resolved: pipe_overlaps,
                    hints_repaired: pipe_hints,
                },
                stage_start: true,
            });
        }
    }));

    // First cleanup_overlaps pass
    {
        let mut first = true;
        cleanup_overlaps_observed(&mut sub, 3, 40, Some(&mut |g, _label, desc, _s| {
            pipe_overlaps += 1;
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "cleanup_overlaps".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                });
                first = false;
            }
        }));
    }

    // repair_directional_hints
    {
        let mut first = true;
        repair_directional_hints_observed(&mut sub, 3, 40, Some(&mut |g, _label, desc, _s| {
            pipe_hints += 1;
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "repair_hints".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                });
                first = false;
            }
        }));
    }

    // stack_updown_rooms
    stack_updown_rooms_observed(&mut sub, Some(&mut |g, _label, desc, _s| {
        if frames.len() < MAX_TIDY_FRAMES {
            frames.push(TidyFrame {
                label: "stack_updown".into(),
                graph: g.clone(),
                description: desc.into(),
                stats: TidyStats {
                    rooms_moved: pipe_rooms_moved,
                    constraints_dropped: pipe_constraints,
                    overlaps_resolved: pipe_overlaps,
                    hints_repaired: pipe_hints,
                },
                stage_start: true,
            });
        }
    }));

    // Second cleanup_overlaps pass
    {
        let mut first = true;
        cleanup_overlaps_observed(&mut sub, 3, 40, Some(&mut |g, _label, desc, _s| {
            pipe_overlaps += 1;
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "cleanup_overlaps".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                });
                first = false;
            }
        }));
    }

    // compact_empty_lines
    {
        let mut first = true;
        compact_empty_lines_observed(&mut sub, Some(&mut |g, _label, desc, _s| {
            if frames.len() < MAX_TIDY_FRAMES {
                frames.push(TidyFrame {
                    label: "compact".into(),
                    graph: g.clone(),
                    description: desc.into(),
                    stats: TidyStats {
                        rooms_moved: pipe_rooms_moved,
                        constraints_dropped: pipe_constraints,
                        overlaps_resolved: pipe_overlaps,
                        hints_repaired: pipe_hints,
                    },
                    stage_start: first,
                });
                first = false;
            }
        }));
    }

    // Frame cap: if the layout is extremely large, frames are silently truncated at MAX_TIDY_FRAMES.

    // Write the tidied positions back into the live graph for this layer's rooms.
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

/// Outcome of attempting to apply a finished async tidy job to the real graph.
pub enum ApplyTidyOutcome {
    /// Positions were applied; caller should recenter if needed.
    Applied,
    /// Job result was stale (graph changed mid-tidy); caller should re-trigger.
    Stale,
}

/// Pure helper: apply the positions from a finished tidy worker to the real graph,
/// guarded by a generation check.
///
/// If `job_gen == current_gen` the worker's final room positions (and distortion flags)
/// are written into `real_graph` for every room that still exists, and `Applied` is
/// returned.  If the generations differ the result is discarded and `Stale` is returned;
/// the caller must re-trigger a fresh tidy.
///
/// Extracted for unit-testability; does not spawn threads.
pub fn apply_tidy_result(
    real_graph: &mut mapper::graph::MapGraph,
    tidied: mapper::graph::MapGraph,
    layer: mapper::layer::LayerId,
    job_gen: u64,
    current_gen: u64,
) -> ApplyTidyOutcome {
    if job_gen != current_gen {
        return ApplyTidyOutcome::Stale;
    }

    // Copy final positions from the tidied clone back into the real graph.
    for id in real_graph.rooms_in_layer(layer) {
        if let Some(p) = tidied.room(id).and_then(|r| r.pos) {
            real_graph.set_pos(id, p);
        }
    }

    // Copy distortion flags.
    let n = real_graph.connections().len();
    for idx in 0..n {
        let c = real_graph.connections()[idx].clone();
        if real_graph.layer_of(c.origin) == layer && real_graph.layer_of(c.dest) == layer {
            if let Some(sc) = tidied.connections().iter()
                .find(|s| s.origin == c.origin && s.dir == c.dir && s.dest == c.dest)
            {
                real_graph.set_conn_distorted(idx, sc.distorted);
            }
        }
    }

    ApplyTidyOutcome::Applied
}

/// Pure decision function for background-tidy mode. Extracted for unit-testability.
///
/// - `mode`: the configured `BackgroundTidy` value.
/// - `new_room`: whether this turn discovered at least one new room.
/// - `overlap`: whether the active layer has a room overlap or distorted edge after
///   incremental placement (fed to all modes now, not only `OnOverlap`).
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
        BackgroundTidy::EveryRoom => new_room || overlap,
        BackgroundTidy::OnOverlap => overlap,
        BackgroundTidy::Debounced => {
            // An overlap fires immediately without waiting for the debounce counter.
            if overlap {
                *counter = 0;
                return true;
            }
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
                    // apply_prompt returns the prompt back for saves-manager and config kinds.
                    if let Some(returned) = apply_prompt(p, mapper) {
                        match &returned.kind {
                            crate::state::PromptKind::ConfigEditPath { field } => {
                                if let Some(cs) = &mut state.config_screen {
                                    match field {
                                        crate::state::ConfigPathField::UserDir => {
                                            cs.working.user_dir = std::path::PathBuf::from(&returned.buffer);
                                        }
                                        crate::state::ConfigPathField::ColorsScheme => {
                                            cs.working.colors.scheme = if returned.buffer.is_empty() {
                                                None
                                            } else {
                                                Some(returned.buffer.clone())
                                            };
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Saves-manager prompt submitted: store for the caller to act on.
                                state.saves_prompt_submitted =
                                    Some((returned.kind, returned.buffer));
                            }
                        }
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
        Action::CycleLayoutReverse => state.cycle_layout_reverse(),
        Action::ZoomIn => state.zoom_in(),
        Action::ZoomOut => state.zoom_out(),
        Action::ZoomReset => state.zoom_reset(),
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

        Action::AnimStageJump(d) => {
            if let Some(anim) = &mut state.tidy_anim {
                let current = anim.idx;
                let n = anim.frames.len();
                if d > 0 {
                    if let Some(next) = ((current + 1)..n).find(|&i| anim.frames[i].stage_start) {
                        anim.idx = next;
                        anim.playing = false;
                    }
                } else if current > 0 {
                    if let Some(prev) = (0..current).rev().find(|&i| anim.frames[i].stage_start) {
                        anim.idx = prev;
                        anim.playing = false;
                    }
                }
            }
        }

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

        // ── File-browser actions ──────────────────────────────────────────────

        // SavesExport, SavesImport, FbEnter, FbChooseDir are caller-handled.

        Action::FbNav(delta) => {
            if let Some(fb) = &mut state.file_browser {
                if !fb.entries.is_empty() {
                    let len = fb.entries.len() as i32;
                    fb.selected = ((fb.selected as i32 + delta).rem_euclid(len)) as usize;
                }
            }
        }

        Action::FbClose => {
            state.file_browser = None;
        }

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

        Action::ActivatePane(focus) => {
            state.focus = focus;
            // Activating the map with no specific room selected clears the panel;
            // activating the game pane also clears any open room panel so the
            // story view is unobstructed.
            state.room_panel = None;
            state.show_inspector = false;
        }

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
                let dx = col as i32 - drag.last.0 as i32;
                let dy = row as i32 - drag.last.1 as i32;
                drag.last = (col, row);
                // Grab-and-drag: dragging right scrolls left (subtract delta).
                // Accumulate directly into char_pan for 1-character precision panning.
                state.char_pan.0 -= dx;
                state.char_pan.1 -= dy;
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

        Action::ToggleInventory => {
            state.show_inventory = !state.show_inventory;
        }

        Action::OpenVerbMenu => {
            state.hotkey_dialog = false;
            let nouns = build_verb_menu_nouns(state, mapper);
            state.verb_menu = Some(crate::state::VerbMenuState {
                pane: crate::state::VerbMenuPane::Verbs,
                verb_idx: 0,
                noun_idx: 0,
                prep_idx: 0,
                nouns,
            });
        }

        Action::VerbMenuNav(kind) => {
            use crate::state::VerbMenuPane;
            use crate::render::verbmenu::{VERB_MENU_VERBS, VERB_MENU_PREPS};
            if let Some(vm) = &mut state.verb_menu {
                match kind {
                    VerbMenuNavKind::NextPane => {
                        vm.pane = match vm.pane {
                            VerbMenuPane::Verbs => VerbMenuPane::Nouns,
                            VerbMenuPane::Nouns => VerbMenuPane::Preps,
                            VerbMenuPane::Preps => VerbMenuPane::Verbs,
                        };
                    }
                    VerbMenuNavKind::PrevPane => {
                        vm.pane = match vm.pane {
                            VerbMenuPane::Verbs => VerbMenuPane::Preps,
                            VerbMenuPane::Nouns => VerbMenuPane::Verbs,
                            VerbMenuPane::Preps => VerbMenuPane::Nouns,
                        };
                    }
                    VerbMenuNavKind::Up => {
                        match vm.pane {
                            VerbMenuPane::Verbs => {
                                let n = VERB_MENU_VERBS.len();
                                if n > 0 {
                                    vm.verb_idx = vm.verb_idx.saturating_sub(1);
                                }
                            }
                            VerbMenuPane::Nouns => {
                                vm.noun_idx = vm.noun_idx.saturating_sub(1);
                            }
                            VerbMenuPane::Preps => {
                                let n = VERB_MENU_PREPS.len();
                                if n > 0 {
                                    vm.prep_idx = vm.prep_idx.saturating_sub(1);
                                }
                            }
                        }
                    }
                    VerbMenuNavKind::Down => {
                        match vm.pane {
                            VerbMenuPane::Verbs => {
                                let n = VERB_MENU_VERBS.len();
                                if n > 0 {
                                    vm.verb_idx = (vm.verb_idx + 1).min(n - 1);
                                }
                            }
                            VerbMenuPane::Nouns => {
                                let n = vm.nouns.len();
                                if n > 0 {
                                    vm.noun_idx = (vm.noun_idx + 1).min(n - 1);
                                }
                            }
                            VerbMenuPane::Preps => {
                                let n = VERB_MENU_PREPS.len();
                                if n > 0 {
                                    vm.prep_idx = (vm.prep_idx + 1).min(n - 1);
                                }
                            }
                        }
                    }
                }
            }
        }

        Action::VerbMenuPick => {
            use crate::render::verbmenu::{VERB_MENU_VERBS, VERB_MENU_PREPS};
            if let Some(vm) = &state.verb_menu {
                let token = vm.selected_token(VERB_MENU_VERBS, VERB_MENU_PREPS).to_owned();
                if !token.is_empty() {
                    state.input.push_str(&token);
                    state.input.push(' ');
                }
            }
        }

        Action::VerbMenuClose => {
            state.verb_menu = None;
        }

        // ── Config screen actions ─────────────────────────────────────────────

        Action::OpenConfig => {
            state.hotkey_dialog = false;
            let working = clone_config(&state.config);
            state.config_screen = Some(crate::state::ConfigScreenState {
                working,
                selected: 0,
            });
        }

        Action::ConfigNav(delta) => {
            if let Some(cs) = &mut state.config_screen {
                let n = CONFIG_ROW_COUNT as i32;
                cs.selected = ((cs.selected as i32 + delta).rem_euclid(n)) as usize;
            }
        }

        Action::ConfigToggle => {
            if state.config_screen.is_some() {
                // Split the borrow: take the selected row, then call helper.
                let selected = state.config_screen.as_ref().map(|cs| cs.selected).unwrap_or(0);
                config_toggle_or_edit(selected, state);
            }
        }

        Action::ConfigCycle(delta) => {
            if let Some(cs) = &mut state.config_screen {
                config_cycle(&mut cs.working, cs.selected, delta);
            }
        }

        Action::ConfigEdit => {
            if let Some(cs) = &state.config_screen {
                let field = config_path_field(cs.selected);
                if let Some(f) = field {
                    let current = match &f {
                        crate::state::ConfigPathField::UserDir => cs.working.user_dir.to_string_lossy().to_string(),
                        crate::state::ConfigPathField::ColorsScheme => cs.working.colors.scheme.clone().unwrap_or_default(),
                    };
                    state.prompt = Some(crate::state::Prompt {
                        kind: crate::state::PromptKind::ConfigEditPath { field: f },
                        buffer: current,
                    });
                }
            }
        }

        Action::ConfigSave => {
            if let Some(cs) = state.config_screen.take() {
                state.config = clone_config(&cs.working);
                // Re-resolve the live look through the style pipeline: style-file
                // base ⊕ the config override sections edited on this screen.
                let (base, _w1) =
                    crate::style::load_style(cs.working.style.as_deref(), &cs.working.user_dir);
                let over = crate::style::style_from_config(&cs.working.colors, &cs.working.symbols);
                let (colors, set, _w2) =
                    crate::style::resolve(&crate::style::merge(&base, &over), &cs.working.user_dir);
                state.colors = colors;
                state.symbols = set;
                // The style-file write + config repoint is caller-handled
                // (main.rs snapshots working before this runs).
            }
        }

        Action::ConfigCancel => {
            state.config_screen = None;
        }

        Action::ResetGame => {
            // Open a confirmation prompt; the caller (main.rs) performs the actual reset
            // when the prompt is submitted with y/yes.
            state.hotkey_dialog = false;
            state.prompt = Some(crate::state::Prompt {
                kind: crate::state::PromptKind::ConfirmReset,
                buffer: String::new(),
            });
        }

        // Caller-handled: silently ignored.
        Action::SubmitCommand(_)
        | Action::SaveGame
        | Action::RestoreGame
        | Action::ExportSvg
        | Action::ExportDot
        | Action::ExportDump
        | Action::SavesLoad
        | Action::SavesExport
        | Action::SavesImport
        | Action::FbEnter
        | Action::FbChooseDir
        | Action::GalleryExportStyle
        | Action::Quit => {}

        Action::None => {}
        // Note: OpenHotkeyDialog and CloseHotkeyDialog are handled above.
    }
}

// ── Suggestion recompute ──────────────────────────────────────────────────────

/// Build the noun list for the verb menu: dedup(room nouns ∪ inventory names),
/// sorted alphabetically.  Room nouns come from the last 20 transcript lines (same
/// source as autocomplete); inventory names come from `list_inventory` when
/// `state.player_obj` is known, or from `state.inventory_fallback` otherwise.
fn build_verb_menu_nouns(state: &AppState, _mapper: &Mapper) -> Vec<String> {
    use std::collections::HashSet;

    // Room words from recent transcript.
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

    let mut seen: HashSet<String> = HashSet::new();
    let mut nouns: Vec<String> = Vec::new();

    for w in room_words {
        if seen.insert(w.clone()) {
            nouns.push(w);
        }
    }

    // Inventory items — prefer list_inventory when player_obj is known.
    let inv_names: Vec<String> = if let Some(p) = state.player_obj {
        // We need access to the Z-machine memory here but apply_action only takes Mapper.
        // The mapper does not hold machine memory, so we fall back to inventory_fallback
        // (which is populated each turn). This is the same graceful path as player_obj=None.
        let _ = p;
        state.inventory_fallback.clone()
    } else {
        state.inventory_fallback.clone()
    };

    for name in inv_names {
        let lower = name.to_lowercase();
        if seen.insert(lower.clone()) {
            nouns.push(lower);
        }
    }

    nouns.sort_unstable();
    nouns
}

/// Return up to `limit` slash command names from `names` whose prefix matches
/// `body_token` (case-insensitive). Results are sorted alphabetically.
pub(crate) fn slash_suggestions(body_token: &str, names: &[String], limit: usize) -> Vec<String> {
    if body_token.is_empty() || limit == 0 {
        return Vec::new();
    }
    let lower = body_token.to_lowercase();
    let mut matches: Vec<String> = names
        .iter()
        .filter(|n| n.to_lowercase().starts_with(&lower) && n.to_lowercase() != lower)
        .cloned()
        .collect();
    matches.sort_unstable();
    matches.dedup();
    matches.truncate(limit);
    matches
}

/// Recompute `state.suggestions` from `state.dict_words`, the room words
/// extracted from `state.transcript`, and the current partial word being typed.
/// Called internally after every input character change in game focus.
///
/// When the input starts with `state.config.command_prefix`, completes the
/// first token after the prefix from `slash::slash_names()` instead of the
/// dictionary.
pub(crate) fn recompute_suggestions(state: &mut AppState) {
    const SUGGESTION_LIMIT: usize = 6;
    let prefix = state.config.command_prefix;
    // Check if the whole input starts with the command prefix.
    if state.input.starts_with(prefix) {
        // Extract the body (everything after the prefix).
        let body = &state.input[prefix.len_utf8()..];
        // Complete only the first token (before any space).
        let first_token = body.split_whitespace().next().unwrap_or("");
        // Only offer completions while the user is still on the first token
        // (no space yet in the body, or trailing chars still form the first word).
        let body_has_space = body.contains(' ');
        if body_has_space {
            // Command name already chosen; no further name completions.
            state.suggestions.clear();
            return;
        }
        let names = crate::slash::slash_names();
        state.suggestions = slash_suggestions(first_token, &names, SUGGESTION_LIMIT);
        return;
    }
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
        // Saves-manager, export, game-reset, and config-path prompts: return to the caller to act on.
        PromptKind::SaveAs
        | PromptKind::ConfirmDeleteSave(_)
        | PromptKind::ConfirmReset
        | PromptKind::ExportSaveName(_)
        | PromptKind::ConfigEditPath { .. } => {
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

// ── Config screen helpers ─────────────────────────────────────────────────────

/// Number of rows in the config screen.
pub(crate) const CONFIG_ROW_COUNT: usize = 11;

/// Clone a Config (Config derives Clone, this is a convenience wrapper for tests).
pub(crate) fn clone_config(cfg: &crate::config::Config) -> crate::config::Config {
    cfg.clone()
}

/// Return the ConfigPathField for a row, if the row is a path type.
fn config_path_field(row: usize) -> Option<crate::state::ConfigPathField> {
    match row {
        0 => Some(crate::state::ConfigPathField::UserDir),
        6 => Some(crate::state::ConfigPathField::ColorsScheme),
        _ => None,
    }
}

/// Apply ConfigToggle to the selected row: toggle bool, advance enum by 1, or open path edit.
fn config_toggle_or_edit(selected: usize, state: &mut AppState) {
    match selected {
        0 => {
            // user_dir — open path edit prompt.
            let current = state.config_screen.as_ref()
                .map(|cs| cs.working.user_dir.to_string_lossy().to_string())
                .unwrap_or_default();
            state.prompt = Some(crate::state::Prompt {
                kind: crate::state::PromptKind::ConfigEditPath {
                    field: crate::state::ConfigPathField::UserDir,
                },
                buffer: current,
            });
        }
        1 => { if let Some(cs) = &mut state.config_screen { cs.working.use_default_map = !cs.working.use_default_map; } }
        2 => { if let Some(cs) = &mut state.config_screen { cs.working.auto_load = !cs.working.auto_load; } }
        3 => { if let Some(cs) = &mut state.config_screen { cs.working.auto_save = !cs.working.auto_save; } }
        4 => { if let Some(cs) = &mut state.config_screen { cs.working.record_history = !cs.working.record_history; } }
        5 => { if let Some(cs) = &mut state.config_screen { config_cycle_background_tidy(&mut cs.working.background_tidy, 1); } }
        6 => {
            // colors.scheme — cycle through preset names + None.
            if let Some(cs) = &mut state.config_screen {
                config_cycle_colors_scheme(&mut cs.working.colors.scheme, 1);
            }
        }
        7 => { if let Some(cs) = &mut state.config_screen { config_cycle_preset(crate::symbols::BoxStyle::preset_names(), &mut cs.working.symbols.box_style, 1); } }
        8 => { if let Some(cs) = &mut state.config_screen { config_cycle_preset(crate::symbols::Arrows::preset_names(), &mut cs.working.symbols.arrow_set, 1); } }
        9 => { if let Some(cs) = &mut state.config_screen { config_cycle_preset(crate::symbols::PortalGlyphs::preset_names(), &mut cs.working.symbols.portal_icons, 1); } }
        10 => { if let Some(cs) = &mut state.config_screen { config_cycle_preset(crate::symbols::PathGlyphs::preset_names(), &mut cs.working.symbols.path_style, 1); } }
        _ => {}
    }
}

/// Cycle a BackgroundTidy enum value by delta.
fn config_cycle_background_tidy(val: &mut crate::config::BackgroundTidy, delta: i32) {
    use crate::config::BackgroundTidy::*;
    let variants = [Off, EveryRoom, OnOverlap, Debounced];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}

/// Cycle the colors.scheme through: None, "mono", "high-contrast", "tomorrow-night", and back.
fn config_cycle_colors_scheme(scheme: &mut Option<String>, delta: i32) {
    let presets: &[&str] = &["mono", "high-contrast", "tomorrow-night"];
    let current_idx = match scheme.as_deref() {
        None => -1i32,
        Some(s) => presets.iter().position(|p| *p == s).map(|i| i as i32).unwrap_or(-1),
    };
    let n = presets.len() as i32;
    let next = current_idx + delta;
    if next < 0 || next >= n {
        *scheme = None;
    } else {
        *scheme = Some(presets[next as usize].to_string());
    }
}

/// Cycle an optional string preset value through the preset_names() list by delta.
///
/// `None` is treated as the first preset; the result is always `Some`, so an
/// explicit gallery/config edit pins the preset in the style file.
fn config_cycle_preset(names: &[&str], val: &mut Option<String>, delta: i32) {
    let n = names.len() as i32;
    if n == 0 { return; }
    let pos = val
        .as_deref()
        .and_then(|v| names.iter().position(|p| *p == v))
        .unwrap_or(0) as i32;
    *val = Some(names[((pos + delta).rem_euclid(n)) as usize].to_string());
}

/// Apply ConfigCycle to the selected row.
fn config_cycle(working: &mut crate::config::Config, row: usize, delta: i32) {
    use crate::symbols::{Arrows, BoxStyle, PathGlyphs, PortalGlyphs};
    match row {
        0 => {} // path: no cycling
        1 => working.use_default_map = !working.use_default_map,
        2 => working.auto_load = !working.auto_load,
        3 => working.auto_save = !working.auto_save,
        4 => working.record_history = !working.record_history,
        5 => config_cycle_background_tidy(&mut working.background_tidy, delta),
        6 => config_cycle_colors_scheme(&mut working.colors.scheme, delta),
        7 => config_cycle_preset(BoxStyle::preset_names(), &mut working.symbols.box_style, delta),
        8 => config_cycle_preset(Arrows::preset_names(), &mut working.symbols.arrow_set, delta),
        9 => config_cycle_preset(PortalGlyphs::preset_names(), &mut working.symbols.portal_icons, delta),
        10 => config_cycle_preset(PathGlyphs::preset_names(), &mut working.symbols.path_style, delta),
        _ => {}
    }
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
    fn esc_closes_room_panel_when_open() {
        use crate::state::{RoomPanel, RoomPanelMode};
        // With a room panel open, Esc produces CloseRoomPanel (q-close removed).
        let mut s = AppState::default();
        s.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::CloseRoomPanel),
            "Esc with room panel open must produce CloseRoomPanel");
        // q no longer closes the room panel.
        assert!(!matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::CloseRoomPanel),
            "q must no longer close the room panel");
        // With no room panel, Esc does NOT produce CloseRoomPanel.
        s.room_panel = None;
        assert!(!matches!(key_to_action(&s, key(KeyCode::Esc)), Action::CloseRoomPanel),
            "Esc with no room panel must not produce CloseRoomPanel");
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
        let build = || {
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
        assert!(anim.frames.len() >= 2, "at least before + one layout stage frame");
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
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false };
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
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false };
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
        // 'q' no longer closes the dialog (q-close removed from hotkey dialog)
        assert!(!matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::CloseHotkeyDialog));
        s.hotkey_dialog = false;

        // ── Anim sub-mode ─────────────────────────────────────────────────────
        let mut s = AppState::default();
        s.focus = Focus::Map;
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false };
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
        // Enter no longer closes the gallery; ESC/[X]/[Done] are the close paths.
        assert!(!matches!(key_to_action(&s, key(KeyCode::Enter)), Action::GalleryClose),
            "Enter must no longer close the gallery");
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
    fn q_no_longer_closes_hotkey_dialog() {
        // q-close removed from hotkey dialog; q now falls through to keymap lookup.
        let mut s = AppState::default();
        s.hotkey_dialog = true;
        // 'q' is not bound to any command in the keymap, so it should produce None
        // (not CloseHotkeyDialog).
        let action = key_to_action(&s, key(KeyCode::Char('q')));
        assert!(
            !matches!(action, Action::CloseHotkeyDialog),
            "q should no longer close the hotkey dialog"
        );
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
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None);
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
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None);
        assert!(
            matches!(action, Action::ShowRoomDiagnostics(2)),
            "right-down on room cell should produce ShowRoomDiagnostics(2), got {:?}", action
        );
    }

    #[test]
    fn left_down_on_gutter_produces_activate_map_pane() {
        use crossterm::event::MouseEventKind;
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact; // step = (12, 5)
        s.scroll = (0, 0);
        // Room is at cell (0,0), box is 8 wide so cols 0..8 hit the room.
        // Click at col 50 misses the room entirely.
        let rects = room_rects_for_compact(1, (0, 0), map_rect());

        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 50, 0, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &rects, &None);
        assert!(
            matches!(action, Action::ActivatePane(Focus::Map)),
            "left-down on map gutter should produce ActivatePane(Map), got {:?}", action
        );
    }

    #[test]
    fn left_down_in_story_produces_activate_game_pane() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 85, 5, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(
            matches!(action, Action::ActivatePane(Focus::Game)),
            "left-down in story pane should produce ActivatePane(Game), got {:?}", action
        );
    }

    #[test]
    fn apply_activate_pane_sets_focus_and_clears_panel() {
        use crate::state::{RoomPanel, RoomPanelMode};
        let mut s = AppState::default(); // starts Focus::Game
        let mut m = Mapper::default();

        // Pre-open a room panel.
        s.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        s.show_inspector = true;

        // ActivatePane(Game) sets game focus and clears the panel.
        apply_action(Action::ActivatePane(Focus::Game), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Game, "ActivatePane(Game) must set focus to Game");
        assert!(s.room_panel.is_none(), "ActivatePane must clear room_panel");
        assert!(!s.show_inspector, "ActivatePane must clear show_inspector");

        // ActivatePane(Map) sets map focus.
        apply_action(Action::ActivatePane(Focus::Map), &mut s, &mut m);
        assert_eq!(s.focus, Focus::Map, "ActivatePane(Map) must set focus to Map");
    }

    #[test]
    fn scroll_up_in_map_produces_pan_up() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::Pan(0, -1)), "scroll up in map without modifier -> Pan(0,-1)");
    }

    #[test]
    fn scroll_down_in_map_produces_pan_down() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollDown, 10, 10, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::Pan(0, 1)), "scroll down in map without modifier -> Pan(0,1)");
    }

    #[test]
    fn scroll_up_with_shift_pans_left() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::SHIFT);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::Pan(-1, 0)), "scroll up + Shift -> Pan(-1,0)");
    }

    #[test]
    fn scroll_up_with_ctrl_zooms_in() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::ScrollUp, 10, 10, KeyModifiers::CONTROL);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::ZoomIn), "scroll up + Ctrl -> ZoomIn");
    }

    #[test]
    fn scroll_in_story_produces_transcript_scroll() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m_up = mouse_event(MouseEventKind::ScrollUp, 85, 5, KeyModifiers::NONE);
        let action_up = mouse_to_action(&s, m_up, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action_up, Action::TranscriptScroll(-1)), "scroll up in story -> TranscriptScroll(-1)");

        let m_dn = mouse_event(MouseEventKind::ScrollDown, 85, 5, KeyModifiers::NONE);
        let action_dn = mouse_to_action(&s, m_dn, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action_dn, Action::TranscriptScroll(1)), "scroll down in story -> TranscriptScroll(1)");
    }

    #[test]
    fn middle_down_produces_begin_drag_pan() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let m = mouse_event(MouseEventKind::Down(MouseButton::Middle), 20, 15, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action, Action::BeginDragPan(20, 15)), "middle-down -> BeginDragPan");
    }

    #[test]
    fn middle_drag_and_up_produce_drag_actions() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        let drag = mouse_event(MouseEventKind::Drag(MouseButton::Middle), 25, 18, KeyModifiers::NONE);
        let up = mouse_event(MouseEventKind::Up(MouseButton::Middle), 25, 18, KeyModifiers::NONE);
        assert!(matches!(mouse_to_action(&s, drag, map_rect(), story_rect(), &[], &None), Action::DragPanTo(25, 18)));
        assert!(matches!(mouse_to_action(&s, up, map_rect(), story_rect(), &[], &None), Action::EndDragPan));
    }

    // ── Drag-pan accumulator tests ────────────────────────────────────────────

    #[test]
    fn drag_pan_accumulates_and_pans_at_step_boundary() {
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        let mut m = Mapper::default();

        // Begin at (10, 10).
        apply_action(Action::BeginDragPan(10, 10), &mut s, &mut m);
        assert!(s.drag.is_some(), "drag state should be set after BeginDragPan");

        // New behavior: drag goes directly into char_pan at 1-char precision.
        // Drag 11 columns right: char_pan.0 = -11, scroll unchanged.
        apply_action(Action::DragPanTo(21, 10), &mut s, &mut m); // dx=11
        assert_eq!(s.char_pan.0, -11, "11-col drag should set char_pan.0 to -11");
        assert_eq!(s.scroll, (0, 0), "scroll should not change during drag");

        // Drag 1 more column right: char_pan.0 = -12, scroll still unchanged.
        apply_action(Action::DragPanTo(22, 10), &mut s, &mut m); // dx=1
        assert_eq!(s.char_pan.0, -12, "additional 1-col drag should set char_pan.0 to -12");
        assert_eq!(s.scroll, (0, 0), "scroll must remain unchanged (char_pan handles it)");
    }

    #[test]
    fn drag_pan_sub_step_movement_does_not_pan() {
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Boxes;
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(0, 0), &mut s, &mut m);
        // Move 5 cols right: goes into char_pan, scroll unchanged.
        apply_action(Action::DragPanTo(5, 0), &mut s, &mut m);
        assert_eq!(s.scroll, (0, 0), "scroll must not change; char_pan absorbs the delta");
        assert_eq!(s.char_pan.0, -5, "char_pan.0 should be -5 after 5-col drag");
    }

    #[test]
    fn drag_pan_grab_and_drag_direction() {
        // Drag LEFT should move content right: char_pan.0 increases (positive).
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(20, 0), &mut s, &mut m);
        // Drag left by 12 columns: dx = -12, char_pan.0 = -(-12) = 12.
        apply_action(Action::DragPanTo(8, 0), &mut s, &mut m);
        assert_eq!(s.char_pan.0, 12, "dragging left should set char_pan.0 positive (content follows grab)");
        assert_eq!(s.scroll.0, 0, "scroll must not change; char_pan handles the delta");
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
    fn should_bg_tidy_every_room_follows_new_room_or_overlap() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        // Fires on new room.
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, true, false, &mut c));
        // Fires on overlap even without a new room.
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, false, true, &mut c));
        // No new room and no overlap: no fire.
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

    #[test]
    fn should_bg_tidy_debounced_fires_immediately_on_overlap() {
        use crate::config::BackgroundTidy;
        // Overlap fires immediately regardless of debounce counter value.
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::Debounced, false, true, &mut c),
            "overlap should fire immediately even without a new room");
        assert_eq!(c, 0, "counter is reset when overlap fires");

        // Even with a partially-accumulated counter, overlap fires immediately.
        let mut c = 2u32;
        assert!(should_bg_tidy(BackgroundTidy::Debounced, false, true, &mut c),
            "overlap fires even with a non-zero counter");
        assert_eq!(c, 0, "counter is reset when overlap fires");
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

    // ── apply_tidy_result ─────────────────────────────────────────────────────

    #[test]
    fn apply_tidy_result_matching_gen_writes_positions() {
        use mapper::direction::Direction;
        // Build a two-room graph, run tidy on a clone (simulating worker output),
        // then apply to the original.
        let mut real = mapper::graph::MapGraph::new();
        real.upsert_room(1, "A".into());
        real.upsert_room(2, "B".into());
        real.add_edge(1, Direction::E, 2);
        real.add_edge(2, Direction::W, 1);

        let mut tidied = real.clone();
        tidy_layer_silent(&mut tidied, mapper::layer::MAIN_LAYER);
        let tidied_pos1 = tidied.room(1).and_then(|r| r.pos);
        let tidied_pos2 = tidied.room(2).and_then(|r| r.pos);

        let gen = 42u64;
        let outcome = apply_tidy_result(&mut real, tidied, mapper::layer::MAIN_LAYER, gen, gen);
        assert!(matches!(outcome, ApplyTidyOutcome::Applied), "matching gen should return Applied");
        assert_eq!(real.room(1).and_then(|r| r.pos), tidied_pos1, "position 1 must be applied");
        assert_eq!(real.room(2).and_then(|r| r.pos), tidied_pos2, "position 2 must be applied");
    }

    #[test]
    fn apply_tidy_result_stale_gen_discards_result() {
        use mapper::direction::Direction;
        let mut real = mapper::graph::MapGraph::new();
        real.upsert_room(1, "A".into());
        real.upsert_room(2, "B".into());
        real.add_edge(1, Direction::E, 2);
        real.add_edge(2, Direction::W, 1);

        // Force known positions on the real graph so we can confirm they are NOT overwritten.
        real.set_pos(1, (100, 100));
        real.set_pos(2, (200, 200));

        let mut tidied = real.clone();
        tidy_layer_silent(&mut tidied, mapper::layer::MAIN_LAYER);

        let job_gen = 5u64;
        let current_gen = 6u64; // graph changed mid-tidy
        let outcome = apply_tidy_result(&mut real, tidied, mapper::layer::MAIN_LAYER, job_gen, current_gen);
        assert!(matches!(outcome, ApplyTidyOutcome::Stale), "differing gen should return Stale");
        // Positions must be untouched.
        assert_eq!(real.room(1).and_then(|r| r.pos), Some((100, 100)), "stale result must not overwrite position 1");
        assert_eq!(real.room(2).and_then(|r| r.pos), Some((200, 200)), "stale result must not overwrite position 2");
    }

    // ── Leaf 1: CycleLayoutReverse ────────────────────────────────────────────

    #[test]
    fn apply_action_cycle_layout_reverse() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(matches!(s.layout, Layout::Split));
        apply_action(Action::CycleLayoutReverse, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::MapFull));
        apply_action(Action::CycleLayoutReverse, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::TranscriptFull));
        apply_action(Action::CycleLayoutReverse, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::Split));
    }

    // ── Leaf 2: ResetGame prompt open + confirm/cancel ────────────────────────

    #[test]
    fn reset_game_action_opens_confirm_reset_prompt() {
        use crate::state::{AppState, PromptKind};
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(s.prompt.is_none());
        apply_action(Action::ResetGame, &mut s, &mut m);
        assert!(s.prompt.is_some(), "ResetGame must open a prompt");
        let p = s.prompt.as_ref().unwrap();
        assert!(matches!(p.kind, PromptKind::ConfirmReset), "prompt kind must be ConfirmReset");
    }

    #[test]
    fn reset_game_prompt_routing_confirm_and_cancel() {
        use crate::state::{AppState, Prompt, PromptKind};
        // Confirm path: Enter (SubmitCommand) → saves_prompt_submitted = Some((ConfirmReset, buf))
        {
            let mut s = AppState::default();
            let mut m = Mapper::default();
            s.prompt = Some(Prompt { kind: PromptKind::ConfirmReset, buffer: "y".to_owned() });
            apply_action(Action::SubmitCommand(String::new()), &mut s, &mut m);
            assert!(s.prompt.is_none(), "prompt should be cleared after submission");
            assert!(
                s.saves_prompt_submitted.is_some(),
                "saves_prompt_submitted should be set on ConfirmReset submission"
            );
            let (kind, buf) = s.saves_prompt_submitted.take().unwrap();
            assert!(matches!(kind, PromptKind::ConfirmReset));
            assert_eq!(buf, "y");
        }
        // Cancel path: Esc (ToggleFocus) → prompt cleared, no saves_prompt_submitted
        {
            let mut s = AppState::default();
            let mut m = Mapper::default();
            s.prompt = Some(Prompt { kind: PromptKind::ConfirmReset, buffer: String::new() });
            apply_action(Action::ToggleFocus, &mut s, &mut m);
            assert!(s.prompt.is_none(), "Esc must cancel the prompt");
            assert!(s.saves_prompt_submitted.is_none(), "no submission on cancel");
        }
    }

    // ── Leaf 2: minizork fixture reset test ───────────────────────────────────

    #[test]
    fn minizork_reset_restores_opening_room_and_clears_turns() {
        use crate::session::{apply_turn, GameSession, TurnResult};
        use zvm::current_location;

        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3");
        if !fixture_path.exists() {
            return; // fixture absent — skip
        }
        let story_bytes = std::fs::read(&fixture_path).expect("read minizork.z3");

        // Build the initial session and seed the start room.
        let mut session = GameSession::new(story_bytes.clone()).expect("GameSession::new");
        let mut mapper = Mapper::default();
        let mut state = crate::state::AppState::default();

        let start_loc = current_location(&session.machine);
        let start_room_number = start_loc.as_ref().map(|s| s.number);
        if let Some(snap) = start_loc {
            let snap_number = snap.number;
            let seed_result = TurnResult {
                transcript: String::new(),
                location: Some(snap),
                quit: false,
                info: None,
            };
            apply_turn(&mut mapper, "", &seed_result);
            state.select_room(Some(snap_number as mapper::graph::RoomId));
        }
        let banner = session.take_transcript();
        state.push_transcript(&banner);

        // Simulate some game turns to advance state.
        let r1 = session.submit("look");
        state.push_transcript(&r1.transcript);
        state.turns = 5;

        // Rebuild session from story_bytes (what handle_saves_prompt does on confirm).
        let mut new_session = GameSession::new(story_bytes.clone()).expect("GameSession::new for reset");
        let new_start_loc = current_location(&new_session.machine);
        let new_room_number = new_start_loc.as_ref().map(|s| s.number);

        // Reset state fields exactly as handle_saves_prompt does.
        state.turns = 0;
        state.input.clear();
        state.suggestions.clear();
        state.suggestion_idx = 0;
        state.transcript.clear();
        state.transcript_scroll = 0;
        let new_banner = new_session.take_transcript();
        state.push_transcript(&new_banner);
        if let Some(snap) = new_start_loc {
            let snap_number = snap.number;
            let seed_result = TurnResult {
                transcript: String::new(),
                location: Some(snap),
                quit: false,
                info: None,
            };
            apply_turn(&mut mapper, "", &seed_result);
            state.select_room(Some(snap_number as mapper::graph::RoomId));
        }

        // Assert post-reset invariants.
        assert_eq!(state.turns, 0, "turn counter must be 0 after reset");
        assert_eq!(
            new_room_number, start_room_number,
            "post-reset current location must equal opening room"
        );
        // Mapper is kept (rooms are still in the graph).
        assert!(mapper.graph.rooms().count() > 0, "mapper must still have rooms after reset");
    }

    // ── Verb menu tests ───────────────────────────────────────────────────────

    /// Helper: open the verb menu with a specific noun list.
    fn open_verb_menu_with_nouns(state: &mut AppState, nouns: Vec<String>) {
        state.verb_menu = Some(crate::state::VerbMenuState {
            pane: crate::state::VerbMenuPane::Verbs,
            verb_idx: 0,
            noun_idx: 0,
            prep_idx: 0,
            nouns,
        });
    }

    #[test]
    fn verb_menu_pick_appends_token_and_space() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string()]);

        // Select "unlock" (index 6 in VERB_MENU_VERBS).
        // Pick from Verbs pane (default on open) at verb_idx=0 → "look".
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input, "look ");

        // Pick again → "look look ".
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input, "look look ");
    }

    #[test]
    fn verb_menu_pick_builds_unlock_door_with_key() {
        use crate::render::verbmenu::VERB_MENU_VERBS;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        let nouns = vec!["door".to_string(), "key".to_string()];
        open_verb_menu_with_nouns(&mut s, nouns);

        // Find indices.
        let unlock_idx = VERB_MENU_VERBS.iter().position(|&v| v == "unlock").expect("unlock in verbs");
        let with_idx = crate::render::verbmenu::VERB_MENU_PREPS.iter().position(|&p| p == "with").expect("with in preps");

        // 1. Pick "unlock" from Verbs pane.
        s.verb_menu.as_mut().unwrap().verb_idx = unlock_idx;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input, "unlock ");

        // 2. Switch to Nouns pane and pick "door" (noun_idx=0).
        s.verb_menu.as_mut().unwrap().pane = crate::state::VerbMenuPane::Nouns;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input, "unlock door ");

        // 3. Switch to Preps pane and pick "with".
        s.verb_menu.as_mut().unwrap().pane = crate::state::VerbMenuPane::Preps;
        s.verb_menu.as_mut().unwrap().prep_idx = with_idx;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input, "unlock door with ");

        // 4. Switch back to Nouns and pick "key" (noun_idx=1).
        s.verb_menu.as_mut().unwrap().pane = crate::state::VerbMenuPane::Nouns;
        s.verb_menu.as_mut().unwrap().noun_idx = 1;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input, "unlock door with key ");
    }

    #[test]
    fn verb_menu_close_leaves_input_intact() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.input = "unlock door ".to_string();
        open_verb_menu_with_nouns(&mut s, vec![]);
        apply_action(Action::VerbMenuClose, &mut s, &mut mapper);
        assert!(s.verb_menu.is_none(), "menu should be closed");
        assert_eq!(s.input, "unlock door ", "input must be preserved");
    }

    #[test]
    fn verb_menu_nav_tab_and_arrows_switch_pane() {
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        assert_eq!(s.verb_menu.as_ref().unwrap().pane, VerbMenuPane::Verbs);

        // Tab → Nouns.
        let a = key_to_action(&s, key(KeyCode::Tab));
        assert!(matches!(a, Action::VerbMenuNav(VerbMenuNavKind::NextPane)));

        // Right → same.
        let a2 = key_to_action(&s, key(KeyCode::Right));
        assert!(matches!(a2, Action::VerbMenuNav(VerbMenuNavKind::NextPane)));

        // Left → PrevPane.
        let a3 = key_to_action(&s, key(KeyCode::Left));
        assert!(matches!(a3, Action::VerbMenuNav(VerbMenuNavKind::PrevPane)));

        // Up/Down → move within pane.
        let a4 = key_to_action(&s, key(KeyCode::Up));
        assert!(matches!(a4, Action::VerbMenuNav(VerbMenuNavKind::Up)));
        let a5 = key_to_action(&s, key(KeyCode::Down));
        assert!(matches!(a5, Action::VerbMenuNav(VerbMenuNavKind::Down)));
    }

    #[test]
    fn verb_menu_nav_up_down_moves_index() {
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string(), "mailbox".to_string()]);
        assert_eq!(s.verb_menu.as_ref().unwrap().verb_idx, 0);

        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Down), &mut s, &mut mapper);
        assert_eq!(s.verb_menu.as_ref().unwrap().verb_idx, 1);

        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.verb_menu.as_ref().unwrap().verb_idx, 0);

        // Up at 0 stays at 0 (saturating).
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.verb_menu.as_ref().unwrap().verb_idx, 0);

        // Switch to Nouns, move down.
        s.verb_menu.as_mut().unwrap().pane = VerbMenuPane::Nouns;
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Down), &mut s, &mut mapper);
        assert_eq!(s.verb_menu.as_ref().unwrap().noun_idx, 1);
    }

    #[test]
    fn verb_menu_esc_closes() {
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);

        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::VerbMenuClose), "Esc closes the menu");
    }

    #[test]
    fn verb_menu_q_no_longer_closes() {
        // q-close removed from verb menu; q now produces None in this sub-mode.
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        let a = key_to_action(&s, key(KeyCode::Char('q')));
        assert!(matches!(a, Action::None), "q should no longer close the verb menu");
    }

    #[test]
    fn open_verb_menu_key_m_routes_to_open_verb_menu() {
        use crate::keymap::{KeyMap, KeySpec};
        use crossterm::event::KeyCode;
        let km = KeyMap::default();
        let spec = KeySpec { code: KeyCode::Char('m'), ctrl: false, shift: false, alt: false };
        let cmd = km.lookup(&spec, crate::keymap::Context::Global);
        assert_eq!(cmd, Some(crate::keymap::Command::OpenVerbMenu), "m should be bound to OpenVerbMenu");
        assert!(matches!(crate::keymap::Command::OpenVerbMenu.to_action(), Action::OpenVerbMenu));
    }

    #[test]
    fn open_verb_menu_in_view_dialog_group() {
        use crate::keymap::HotkeyLayout;
        let layout = HotkeyLayout::default();
        let view_group = layout.groups.iter().find(|(title, _)| title == "View");
        assert!(view_group.is_some(), "View group should exist");
        let (_, cmds) = view_group.unwrap();
        assert!(cmds.contains(&crate::keymap::Command::OpenVerbMenu), "OpenVerbMenu should be in View group");
    }

    #[test]
    fn open_verb_menu_action_opens_modal() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        assert!(s.verb_menu.is_none());
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        assert!(s.verb_menu.is_some(), "verb_menu should be Some after OpenVerbMenu");
        assert!(matches!(s.verb_menu.as_ref().unwrap().pane, crate::state::VerbMenuPane::Verbs));
    }

    #[test]
    fn noun_list_deduplicates_room_and_inventory() {
        // A noun that appears in both room transcript and inventory_fallback
        // should appear exactly once.
        let mut s = AppState::default();
        // Push transcript text with "mailbox" as a room word.
        s.push_transcript("There is a small mailbox here.");
        // Also have "mailbox" in inventory fallback.
        s.inventory_fallback = vec!["mailbox".to_string()];
        let mut mapper = Mapper::default();
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        let nouns = &s.verb_menu.as_ref().unwrap().nouns;
        let mailbox_count = nouns.iter().filter(|n| n.as_str() == "mailbox").count();
        assert_eq!(mailbox_count, 1, "mailbox should appear exactly once in nouns (dedup)");
    }

    // ── File-browser sub-mode key tests ───────────────────────────────────────

    /// Build a state with saves open (for testing e/i dispatch).
    fn state_with_saves_for_fb_tests() -> AppState {
        let mut s = AppState::default();
        s.saves = Some(crate::state::SavesState { entries: Vec::new(), selected: 0 });
        s
    }

    /// Build a state with the file browser open.
    fn state_with_filebrowser(mode: crate::state::FbMode) -> AppState {
        use crate::state::FileBrowserState;
        let mut s = AppState::default();
        let tmp = std::env::temp_dir();
        s.file_browser = Some(FileBrowserState::build(tmp, mode, "test.qzl".to_string()));
        s
    }

    #[test]
    fn saves_e_opens_export_browser_action() {
        let s = state_with_saves_for_fb_tests();
        let a = key_to_action(&s, key(KeyCode::Char('e')));
        assert!(matches!(a, Action::SavesExport), "e in saves sub-mode should produce SavesExport");
    }

    #[test]
    fn saves_i_opens_import_browser_action() {
        let s = state_with_saves_for_fb_tests();
        let a = key_to_action(&s, key(KeyCode::Char('i')));
        assert!(matches!(a, Action::SavesImport), "i in saves sub-mode should produce SavesImport");
    }

    #[test]
    fn filebrowser_esc_produces_fb_close() {
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::FbClose), "Esc in file browser should produce FbClose");
    }

    #[test]
    fn filebrowser_q_no_longer_closes() {
        // q-close removed from file browser; q now produces None in this sub-mode.
        let s = state_with_filebrowser(crate::state::FbMode::PickDir);
        let a = key_to_action(&s, key(KeyCode::Char('q')));
        assert!(matches!(a, Action::None), "q should no longer close the file browser");
    }

    #[test]
    fn filebrowser_up_down_navigate() {
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::FbNav(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::FbNav(1)));
    }

    #[test]
    fn filebrowser_enter_produces_fb_enter() {
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(matches!(a, Action::FbEnter), "Enter in file browser should produce FbEnter");
    }

    #[test]
    fn filebrowser_s_produces_fb_choose_dir() {
        let s = state_with_filebrowser(crate::state::FbMode::PickDir);
        let a = key_to_action(&s, key(KeyCode::Char('s')));
        assert!(matches!(a, Action::FbChooseDir), "s in file browser should produce FbChooseDir");
    }

    #[test]
    fn fb_close_action_clears_file_browser() {
        let mut s = state_with_filebrowser(crate::state::FbMode::PickFile);
        assert!(s.file_browser.is_some());
        apply_action(Action::FbClose, &mut s, &mut Mapper::default());
        assert!(s.file_browser.is_none(), "FbClose should clear file_browser");
    }

    #[test]
    fn fb_nav_wraps_around() {
        let mut s = state_with_filebrowser(crate::state::FbMode::PickFile);
        // We need at least one entry — the tmp dir should have ".." if not root.
        if let Some(fb) = &s.file_browser {
            if fb.entries.is_empty() {
                return; // nothing to navigate
            }
        }
        // Move up from 0 should wrap to last entry.
        apply_action(Action::FbNav(-1), &mut s, &mut Mapper::default());
        if let Some(fb) = &s.file_browser {
            assert_eq!(fb.selected, fb.entries.len() - 1, "nav -1 from 0 should wrap to last");
        }
    }

    // ── Item 1: char-granular drag pan ────────────────────────────────────────

    /// DragPanTo accumulates into char_pan at 1-character resolution.
    /// A drag delta of N columns shifts char_pan.0 by -N (grab-and-drag semantics).
    #[test]
    fn drag_pan_to_accumulates_char_pan() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Start drag at (10, 10).
        apply_action(Action::BeginDragPan(10, 10), &mut s, &mut m);
        // Drag 5 columns right, 3 rows down.
        apply_action(Action::DragPanTo(15, 13), &mut s, &mut m);
        assert_eq!(
            s.char_pan,
            (-5, -3),
            "drag right+down by (5,3) should set char_pan to (-5,-3)"
        );
        // Continue dragging 2 columns left.
        apply_action(Action::DragPanTo(13, 13), &mut s, &mut m);
        assert_eq!(
            s.char_pan,
            (-3, -3),
            "additional drag left by 2 should update char_pan to (-3,-3)"
        );
    }

    /// Ending the drag clears state.drag but leaves char_pan intact.
    #[test]
    fn end_drag_pan_leaves_char_pan() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::BeginDragPan(5, 5), &mut s, &mut m);
        apply_action(Action::DragPanTo(8, 5), &mut s, &mut m);
        assert_eq!(s.char_pan, (-3, 0));
        apply_action(Action::EndDragPan, &mut s, &mut m);
        assert!(s.drag.is_none(), "EndDragPan should clear drag state");
        assert_eq!(s.char_pan, (-3, 0), "EndDragPan must not reset char_pan");
    }

    /// Build a minimal MouseEvent for testing.
    fn mouse_left_click(col: u16, row: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn config_dialog_button_clicks_map_to_actions() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::ConfigScreenState;

        // Build known rects:
        // dialog area at (10, 5, 40, 15)
        // close at (48, 5, 1, 1)  — just inside top-right
        // Save button at (20, 19, 8, 1)
        // Cancel button at (29, 19, 10, 1)
        let rects = DialogRects {
            area:    Rect::new(10, 5, 40, 15),
            content: Rect::new(11, 7, 38, 10),
            close:   Some(Rect::new(48, 5, 1, 1)),
            buttons: vec![
                (ButtonId::Save,   Rect::new(20, 19, 8,  1)),
                (ButtonId::Cancel, Rect::new(29, 19, 10, 1)),
            ],
        };

        // State with config_screen open (so dialog routing knows which modal).
        let mut state = AppState::default();
        let working = crate::input::clone_config(&state.config);
        state.config_screen = Some(ConfigScreenState { working, selected: 0 });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → ConfigCancel
        let a = mouse_to_action(&state, mouse_left_click(48, 5), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::ConfigCancel), "close click should produce ConfigCancel, got {:?}", a);

        // Save button → ConfigSave
        let a = mouse_to_action(&state, mouse_left_click(22, 19), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::ConfigSave), "Save button should produce ConfigSave, got {:?}", a);

        // Cancel button → ConfigCancel
        let a = mouse_to_action(&state, mouse_left_click(32, 19), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::ConfigCancel), "Cancel button should produce ConfigCancel, got {:?}", a);

        // Click outside dialog area → swallowed (Action::None)
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside-dialog click should be swallowed (None), got {:?}", a);
    }

    #[test]
    fn config_esc_maps_to_config_cancel() {
        // ESC in config screen should produce ConfigCancel (same as [X] and Cancel button).
        let mut s = AppState::default();
        let working = crate::input::clone_config(&s.config);
        s.config_screen = Some(crate::state::ConfigScreenState { working, selected: 0 });
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::ConfigCancel), "ESC in config screen should produce ConfigCancel");
    }

    #[test]
    fn saves_dialog_x_and_done_produce_saves_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        let rects = DialogRects {
            area:    Rect::new(10, 5, 40, 15),
            content: Rect::new(11, 7, 38, 10),
            close:   Some(Rect::new(48, 5, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(40, 19, 8, 1))],
        };

        let mut state = AppState::default();
        state.saves = Some(SavesState {
            entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.babelmap"),
                name: "a".into(),
                turns: 0,
                saved_at: String::new(),
                is_default: false,
            }],
            selected: 0,
        });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → SavesClose
        let a = mouse_to_action(&state, mouse_left_click(48, 5), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::SavesClose), "saves [X] click should produce SavesClose, got {:?}", a);

        // Done button → SavesClose
        let a = mouse_to_action(&state, mouse_left_click(42, 19), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::SavesClose), "saves [Done] click should produce SavesClose, got {:?}", a);

        // Click outside → swallowed
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside saves dialog should be swallowed, got {:?}", a);
    }

    #[test]
    fn filebrowser_dialog_x_and_done_produce_fb_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};

        let rects = DialogRects {
            area:    Rect::new(8, 4, 50, 18),
            content: Rect::new(9, 6, 48, 13),
            close:   Some(Rect::new(56, 4, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(48, 21, 8, 1))],
        };

        let mut state = AppState::default();
        let tmp = std::env::temp_dir();
        state.file_browser = Some(crate::state::FileBrowserState::build(
            tmp,
            crate::state::FbMode::PickFile,
            String::new(),
        ));

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → FbClose
        let a = mouse_to_action(&state, mouse_left_click(56, 4), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::FbClose), "filebrowser [X] click should produce FbClose, got {:?}", a);

        // Done button → FbClose
        let a = mouse_to_action(&state, mouse_left_click(50, 21), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::FbClose), "filebrowser [Done] click should produce FbClose, got {:?}", a);

        // Click outside → swallowed
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside filebrowser dialog should be swallowed, got {:?}", a);
    }

    #[test]
    fn verbmenu_dialog_x_and_done_produce_verb_menu_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::VerbMenuState;

        let rects = DialogRects {
            area:    Rect::new(0, 0, 80, 24),
            content: Rect::new(1, 2, 78, 20),
            close:   Some(Rect::new(78, 0, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(70, 23, 8, 1))],
        };

        let mut state = AppState::default();
        state.verb_menu = Some(VerbMenuState {
            pane: crate::state::VerbMenuPane::Verbs,
            verb_idx: 0,
            noun_idx: 0,
            prep_idx: 0,
            nouns: vec![],
        });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → VerbMenuClose
        let a = mouse_to_action(&state, mouse_left_click(78, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::VerbMenuClose), "verb menu [X] click should produce VerbMenuClose, got {:?}", a);

        // Done button → VerbMenuClose
        let a = mouse_to_action(&state, mouse_left_click(72, 23), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::VerbMenuClose), "verb menu [Done] click should produce VerbMenuClose, got {:?}", a);
    }

    #[test]
    fn hotkey_dialog_x_and_done_produce_close_hotkey_dialog() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};

        let rects = DialogRects {
            area:    Rect::new(10, 5, 60, 30),
            content: Rect::new(11, 7, 58, 26),
            close:   Some(Rect::new(68, 5, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(60, 34, 8, 1))],
        };

        let mut state = AppState::default();
        state.hotkey_dialog = true;

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → CloseHotkeyDialog
        let a = mouse_to_action(&state, mouse_left_click(68, 5), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::CloseHotkeyDialog), "hotkey dialog [X] click should produce CloseHotkeyDialog, got {:?}", a);

        // Done button → CloseHotkeyDialog
        let a = mouse_to_action(&state, mouse_left_click(62, 34), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::CloseHotkeyDialog), "hotkey dialog [Done] click should produce CloseHotkeyDialog, got {:?}", a);
    }

    // ── ESC == [X] sweep ─────────────────────────────────────────────────────────

    /// Table test: for every modal, ESC and a [X] click must yield the SAME close Action.
    ///
    /// Each entry: (modal_name, set-up closure, ESC-Action, close-Action-from-X-click).
    ///
    /// We build a DialogRects with a known close rect at (99, 0) and call
    /// key_to_action for ESC and mouse_to_action for a click at (99, 0).
    /// Both must match the expected close action.
    #[test]
    fn esc_equals_x_click_for_every_modal() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::{GalleryState, SavesState, VerbMenuState};
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];

        // Helper to build a DialogRects with [X] at (99, 0) and one Done button
        let make_rects = || DialogRects {
            area:    Rect::new(5, 0, 70, 24),
            content: Rect::new(6, 1, 68, 20),
            close:   Some(Rect::new(99, 0, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(90, 23, 8, 1))],
        };

        // 1. Gallery: ESC → GalleryClose, [X] → GalleryClose
        {
            let mut s = AppState::default();
            s.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::GalleryClose),
                "gallery ESC should produce GalleryClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::GalleryClose),
                "gallery [X] click should produce GalleryClose, got {:?}", x_action);
        }

        // 2. Saves: ESC → SavesClose, [X] → SavesClose
        {
            let mut s = AppState::default();
            s.saves = Some(SavesState { entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.babelmap"), name: "a".into(), turns: 0,
                saved_at: String::new(), is_default: false,
            }], selected: 0 });
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::SavesClose),
                "saves ESC should produce SavesClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::SavesClose),
                "saves [X] click should produce SavesClose, got {:?}", x_action);
        }

        // 3. File browser: ESC → FbClose, [X] → FbClose
        {
            let mut s = AppState::default();
            s.file_browser = Some(crate::state::FileBrowserState::build(
                std::env::temp_dir(), crate::state::FbMode::PickFile, String::new(),
            ));
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::FbClose),
                "file browser ESC should produce FbClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::FbClose),
                "file browser [X] click should produce FbClose, got {:?}", x_action);
        }

        // 4. Verb menu: ESC → VerbMenuClose, [X] → VerbMenuClose
        {
            let mut s = AppState::default();
            s.verb_menu = Some(VerbMenuState {
                pane: crate::state::VerbMenuPane::Verbs,
                verb_idx: 0, noun_idx: 0, prep_idx: 0, nouns: vec![],
            });
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::VerbMenuClose),
                "verb menu ESC should produce VerbMenuClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::VerbMenuClose),
                "verb menu [X] click should produce VerbMenuClose, got {:?}", x_action);
        }

        // 5. Config screen: ESC → ConfigCancel, [X] → ConfigCancel
        {
            let mut s = AppState::default();
            let working = clone_config(&s.config);
            s.config_screen = Some(crate::state::ConfigScreenState { working, selected: 0 });
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::ConfigCancel),
                "config screen ESC should produce ConfigCancel, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::ConfigCancel),
                "config screen [X] click should produce ConfigCancel, got {:?}", x_action);
        }

        // 6. Hotkey dialog: ESC → CloseHotkeyDialog, [X] → CloseHotkeyDialog
        {
            let mut s = AppState::default();
            s.hotkey_dialog = true;
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::CloseHotkeyDialog),
                "hotkey dialog ESC should produce CloseHotkeyDialog, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::CloseHotkeyDialog),
                "hotkey dialog [X] click should produce CloseHotkeyDialog, got {:?}", x_action);
        }
    }

    /// Assert no modal key handler still binds q to a close action.
    #[test]
    fn no_modal_binds_q_to_close() {
        use crate::state::{GalleryState, SavesState, VerbMenuState};
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        // Gallery: q → not GalleryClose
        {
            let mut s = AppState::default();
            s.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::GalleryClose),
                "q must not close the gallery");
        }

        // Saves: q → not SavesClose
        {
            let mut s = AppState::default();
            s.saves = Some(SavesState { entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.babelmap"), name: "a".into(), turns: 0,
                saved_at: String::new(), is_default: false,
            }], selected: 0 });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::SavesClose),
                "q must not close the saves modal");
        }

        // File browser: q → not FbClose
        {
            let mut s = AppState::default();
            s.file_browser = Some(crate::state::FileBrowserState::build(
                std::env::temp_dir(), crate::state::FbMode::PickFile, String::new(),
            ));
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::FbClose),
                "q must not close the file browser");
        }

        // Verb menu: q → not VerbMenuClose
        {
            let mut s = AppState::default();
            s.verb_menu = Some(VerbMenuState {
                pane: crate::state::VerbMenuPane::Verbs,
                verb_idx: 0, noun_idx: 0, prep_idx: 0, nouns: vec![],
            });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::VerbMenuClose),
                "q must not close the verb menu");
        }

        // Config screen: q → not ConfigCancel
        {
            let mut s = AppState::default();
            let working = clone_config(&s.config);
            s.config_screen = Some(crate::state::ConfigScreenState { working, selected: 0 });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::ConfigCancel),
                "q must not cancel the config screen");
        }

        // Room panel: q → not CloseRoomPanel
        {
            use crate::state::{RoomPanel, RoomPanelMode};
            let mut s = AppState::default();
            s.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::CloseRoomPanel),
                "q must not close the room panel");
        }
    }

    /// Assert gallery [X] and [Done] clicks produce GalleryClose.
    #[test]
    fn gallery_dialog_x_and_done_produce_gallery_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::GalleryState;

        let rects = DialogRects {
            area:    Rect::new(5, 3, 70, 24),
            content: Rect::new(6, 5, 68, 19),
            close:   Some(Rect::new(73, 3, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(65, 26, 8, 1))],
        };

        let mut state = AppState::default();
        state.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // Close [X] → GalleryClose
        let a = mouse_to_action(&state, mouse_left_click(73, 3), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::GalleryClose), "gallery [X] click should produce GalleryClose, got {:?}", a);

        // Done button → GalleryClose
        let a = mouse_to_action(&state, mouse_left_click(67, 26), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::GalleryClose), "gallery [Done] click should produce GalleryClose, got {:?}", a);

        // Click outside → swallowed (None)
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::None), "outside gallery dialog should be swallowed, got {:?}", a);
    }

    /// Regression: a centered modal (gallery) stacked on top of an open corner
    /// overlay (room_panel) must swallow all outside-dialog clicks.  Without the
    /// fix, is_corner_overlay was true even when a centered modal was open, so the
    /// outside click fell through to ShowRoomInfo / ActivatePane.
    #[test]
    fn centered_modal_swallows_outside_clicks_even_with_room_panel_open() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::{GalleryState, RoomPanel, RoomPanelMode};
        use crate::state::Zoom;

        // Build a real map rect and room_rects so a click at (0,0) would normally
        // produce ShowRoomInfo if the dialog were not open.
        let map_r = map_rect();   // Rect::new(0,0,80,40)
        let story_r = story_rect();
        let live_room_rects = room_rects_for_compact(1, (0, 0), map_r);

        // Confirm that without any dialog open, clicking (0,0) hits the room.
        {
            let s = AppState::default();
            let a = mouse_to_action(&s, mouse_left_click(0, 0), map_r, story_r, &live_room_rects, &None);
            assert!(
                matches!(a, Action::ShowRoomInfo(1)),
                "sanity: without dialog, click on room should be ShowRoomInfo(1), got {:?}", a
            );
        }

        // Now open BOTH room_panel (corner overlay) AND gallery (centered modal).
        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        state.gallery = Some(GalleryState { category_idx: 0, selections: [0; 4] });

        // The dialog rects represent the gallery centered dialog (not covering (0,0)).
        let dialog = Some(DialogRects {
            area:    Rect::new(5, 3, 70, 24),
            content: Rect::new(6, 5, 68, 19),
            close:   Some(Rect::new(73, 3, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(65, 26, 8, 1))],
        });

        // Click OUTSIDE the gallery dialog (at (0,0), which is on the room).
        // Must be swallowed — NOT ShowRoomInfo or ActivatePane.
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map_r, story_r, &live_room_rects, &dialog);
        assert!(
            matches!(a, Action::None),
            "outside-gallery click with room_panel also open must be swallowed (None), got {:?}", a
        );
    }

    // ── slash_suggestions tests ───────────────────────────────────────────────

    #[test]
    fn slash_suggestions_filter_by_prefix() {
        let names = vec!["panh".to_string(),"panv".to_string(),"zoom".to_string(),"open-config".to_string()];
        let s = slash_suggestions("pa", &names, 6);
        assert!(s.contains(&"panh".to_string()) && s.contains(&"panv".to_string()));
        assert!(!s.contains(&"zoom".to_string()));
    }
}
