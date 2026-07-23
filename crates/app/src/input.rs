//! Input → `Action` mapping and application.
//!
//! # Focus routing
//! `key_to_action` applies bindings in this strict precedence order:
//! 1. Ctrl+Q / Ctrl+C → Quit (always wins, even during a prompt).
//! 2. Prompt active → route to prompt only; all other keys absorbed as None.
//! 3. Tidy-anim sub-mode → KeyMap lookup in Anim context; no fallthrough.
//! 4. Saves-manager sub-mode → saves_key_to_action.
//! 6. Hotkey dialog open → hotkey_dialog_key_to_action.
//! 7. Key == hotkeys.prefix → OpenHotkeyDialog.
//! 8. Tab (no modifiers) → autocomplete-or-ToggleFocus special case.
//! 9. Ctrl modifier → Global KeyMap lookup, filtered by hotkeys.is_direct.
//! 10. Per-focus routing:
//!     - Game: game_key_to_action, then Global fallthrough.
//!     - Map: Map context lookup, filtered by hotkeys.is_direct (direct commands only).
//!
//! The former bottom-bar text-entry prompts are now the `text_entry` /
//! `confirm_delete_save` modals, whose key/mouse input the run-loop intercepts in
//! `main.rs` own directly (like the save-name dialog). This module only OPENS them
//! and provides `apply_text_entry` for the submit. (SQ-0307)
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
use mapper::direction::Direction;
use mapper::mapper::Mapper;

use crate::complete::{room_words_from_text, suggest};
use crate::keymap::{Context, KeySpec};
use crate::state::{AppState, Focus, TextEntryDialog, TextEntryKind};

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
    /// Page the current pane's selection up by one viewport (PageUp).
    PageUp,
    /// Page the current pane's selection down by one viewport (PageDown).
    PageDown,
    /// Jump to the first item in the current pane (Home).
    Home,
    /// Jump to the last item in the current pane (End).
    End,
}

// ── ResizeNavKind ─────────────────────────────────────────────────────────────

/// Navigation kind for `Action::ResizeNav`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeNavKind {
    /// Switch to the next visible pane (Tab).
    NextTarget,
    /// Switch to the previous visible pane (Shift+Tab).
    PrevTarget,
    /// Shrink the target horizontally (or its width).
    Left,
    /// Grow the target horizontally (or its width).
    Right,
    /// Grow the target vertically (or its height).
    Up,
    /// Shrink the target vertically (or its height).
    Down,
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
    /// Reverse of `ToggleFocus`: cycle window focus one step backward (Shift-Tab).
    CycleFocusBack,
    /// Toggle the map panel on/off (Split ↔ TranscriptFull).
    ToggleMap,
    /// Re-tidy the Auto layout: re-derive room positions (sort) then clean overlaps.
    /// No-op in Manual mode (positions are user-controlled and frozen).
    Retidy,
    /// Re-read style.toml and swap the live colors/symbols (keeps current look on error).
    ReloadStyle,
    /// Toggle the opt-in style.toml file-watcher (handled in the run loop).
    ToggleWatch,
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
    /// Move the input caret one char left.
    CursorLeft,
    /// Move the input caret one char right — or, at the end of the line with a suggestion
    /// showing, accept that suggestion (SQ-0354). See `apply_action`.
    CursorRight,
    /// Move the input caret to the start of the line.
    CursorHome,
    /// Move the input caret to the end of the line.
    CursorEnd,
    /// Delete the char AT the caret (Backspace deletes the one before it).
    DeleteChar,
    /// Put the input caret on the char under a click at this screen column/row.
    CursorToClick(u16, u16),
    /// Zoom the map in one VISIBLE step (more detail). A keypress must move the map.
    ZoomIn,
    /// Zoom the map out one VISIBLE step (less detail).
    ZoomOut,
    /// Zoom by `n` VISIBLE steps: positive in (toward Boxes), negative out (SQ-0355).
    ///
    /// The `zoom-map <n>` form. `ZoomIn`/`ZoomOut` are the one-step keys; this carries the
    /// magnitude the command's usage string has always promised.
    ZoomBy(i32),
    /// Zoom in one FINE step — the wheel's gesture (SQ-0350).
    ///
    /// Three fine steps make one visible step, so a fast ctrl+scroll cannot skip past the middle
    /// view. The keyboard uses `ZoomIn`/`ZoomOut` instead: one press, one visible change.
    ZoomInFine,
    /// Zoom out one FINE step — the wheel's gesture. See `ZoomInFine`.
    ZoomOutFine,
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
    /// Caller: export the map as SVG. `Some(dest)` is the optional `[file]` arg.
    ExportSvg(Option<String>),
    /// Caller: export the map as a Graphviz DOT graph. `Some(dest)` is the optional `[file]` arg.
    ExportDot(Option<String>),
    /// Caller: write an annotatable text/ASCII map dump. `Some(dest)` is the optional `[file]` arg.
    ExportMap(Option<String>),
    /// Toggle the in-box alignment code overlay (Ctrl+A).
    ToggleAlignment,
    /// Toggle portal destination name labels beside in-room portal icons (Ctrl+P).
    TogglePortalLabels,
    /// Toggle room-number (#id) visibility in Boxes-zoom room boxes.
    ToggleRoomNumbers,
    ToggleStatusBar,
    /// Toggle honoring the Z-machine's timed-input (`read`/`read_char` timers).
    ToggleTimedInput,
    /// Toggle audio playback (config.enable_sound).
    ToggleSound,
    /// Set the master audio volume 0..=100 (config.volume).
    SetVolume(u8),
    /// Toggle the room-detection-method indicator in the map corner.
    ToggleLocMethod,
    /// Toggle the per-room diagnostics inspector overlay (map focus, `i` key).
    ToggleInspector,
    /// Caller: exit the application.
    Quit,
    /// Cycle the viewed layer by `delta` steps over the sorted non-empty layer list (clamped at ends).
    CycleLayer(i32),
    /// Select a specific layer as the viewed one (a click on its map layer tab).
    SetViewedLayer(mapper::layer::LayerId),
    /// Peel the selected (or current) room's region into a new child layer.
    /// Peel a layer. `Some(dir)` cuts at that passage out of the selected room; `None` looks for a
    /// portal seam already dividing the layer (SQ-0360).
    PeelLayer(Option<Direction>),
    /// Merge the active layer into its parent layer.
    MergeLayer,
    /// Advance autocomplete to the next suggestion, applying the current one to
    /// the input buffer (game focus, Tab key — only when a partial word is being
    /// typed AND suggestions are available; otherwise Tab keeps its ToggleFocus
    /// role).
    Autocomplete,
    /// Step autocomplete to the previous suggestion, applying the current one to
    /// the input buffer (game focus, Shift-Tab key — the reverse of `Autocomplete`,
    /// active under the same mid-word-with-suggestions condition; otherwise
    /// Shift-Tab keeps its `ToggleFocus` role).
    AutocompletePrev,
    /// Recall the previous (older) command into the input buffer (game focus, Up).
    HistoryPrev,
    /// Recall the next (newer) command into the input buffer (game focus, Down).
    HistoryNext,
    /// Open the hotkey dialog overlay.
    OpenHotkeyDialog,
    /// Close the hotkey dialog overlay.
    CloseHotkeyDialog,
    /// Open the saves-manager modal (loads the save list).
    OpenSaves,
    /// Navigate the saves list by delta (-1 = up, +1 = down).
    SavesNav(i32),
    /// Page the saves list by one viewport (-1 = PageUp, +1 = PageDown).
    SavesPage(i32),
    /// Jump to the first save entry.
    SavesHome,
    /// Jump to the last save entry.
    SavesEnd,
    /// Load the selected save (caller-handled).
    SavesLoad,
    /// Begin a SaveAs prompt for a new named save (sets up the prompt sub-mode).
    SavesSaveAs,
    /// Begin a confirm-delete prompt for the selected save (sets up the prompt sub-mode).
    SavesDelete,
    /// Close the saves-manager modal without acting.
    SavesClose,
    /// Navigate the VFS file-picker list by delta (-1 = up, +1 = down).
    FilePickerNav(i32),
    /// Pick the selected VFS filename (caller-handled).
    FilePickerPick,
    /// Close the VFS file-picker modal without picking.
    FilePickerClose,
    /// Toggle the inventory strip at the bottom of the story pane.
    ToggleInventory,
    /// Open a confirmation prompt to reset the game to its opening state (keeps map).
    ResetGame,
    /// Open the verb/item token-palette modal, building the noun list from the current room and inventory.
    OpenVerbMenu,
    /// Navigate within the verb menu: `Tab`/`Left`/`Right` switches pane; `Up`/`Down` moves selection.
    VerbMenuNav(VerbMenuNavKind),
    /// Pick the currently-selected token: append it (+ a space) to `state.input`.
    VerbMenuPick,
    /// Insert the token at the given pane/index directly (mouse click), keeping
    /// the dock open and story focus unchanged; also moves that pane's highlight.
    VerbMenuClickToken(crate::state::VerbMenuPane, usize),
    /// Focus the given dock pane (mouse click on its header): sets `story_focused=false`.
    VerbMenuFocusPane(crate::state::VerbMenuPane),
    /// Close the verb menu, leaving `state.input` intact.
    VerbMenuClose,
    /// Enter interactive pane-resize mode, selecting the first visible pane.
    ResizePanes,
    /// Exit resize mode without resetting sizes.
    ResizeExit,
    /// Reset all pane sizes to their config defaults.
    ResizeReset,
    /// Navigate resize mode: `Tab`/`Shift+Tab` switches the target pane; arrows adjust it.
    ResizeNav(ResizeNavKind),
    /// Open the config screen modal.
    OpenConfig,
    /// Navigate the config screen by delta (-1 = up, +1 = down).
    ConfigNav(i32),
    /// Page the config screen list by one viewport (-1 = PageUp, +1 = PageDown).
    ConfigPage(i32),
    /// Jump to the first config row.
    ConfigHome,
    /// Jump to the last config row.
    ConfigEnd,
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
    /// Open the file browser in PickFile mode to import a .qzl/.sav file.
    SavesImport,
    /// Navigate the file browser by delta (-1 = up, +1 = down).
    FbNav(i32),
    /// Page the file-browser list by one viewport (-1 = PageUp, +1 = PageDown).
    FbPage(i32),
    /// Jump to the first file-browser entry.
    FbHome,
    /// Jump to the last file-browser entry.
    FbEnd,
    /// Activate the selected file-browser entry (cd into dir or import file).
    FbEnter,
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
    /// Begin a story-pane text selection at terminal cell (col, row).
    StartSelection(u16, u16),
    /// Extend the story-pane text selection to terminal cell (col, row).
    ExtendSelection(u16, u16),
    /// End the story-pane text selection (copies it to the clipboard).
    EndSelection,
    /// Scroll the transcript by delta lines (positive = down, negative = up).
    TranscriptScroll(i32),
    /// Page the transcript by one screenful (PageUp/PageDown). `+1` scrolls toward
    /// older lines, `-1` toward newer. Resolved by the run loop, which knows the
    /// last-rendered transcript viewport height and max scroll (see `page_scroll`).
    TranscriptScrollPage(i8),
    /// Advance the `[more]` pager one screen toward the bottom; catching up exits
    /// the pager (SQ-0404).
    PagerAdvance,
    /// Dismiss the `[more]` pager, jumping straight to the newest output (SQ-0404).
    PagerDismiss,
    /// Open the Hints panel. Real behavior wired in Task D; stub here keeps match exhaustive.
    OpenHints,
    /// Open the rewind/replay history modal (seeds `replay` at the last turn).
    OpenHistory,
    /// Step the replay selection by delta turns (-1 left, +1 right).
    ReplayStep(isize),
    /// Page the replay selection by one viewport (-1 = PageUp, +1 = PageDown).
    ReplayPage(i8),
    /// Toggle replay auto-play.
    ReplayTogglePlay,
    /// Close the replay modal (back to live, no change).
    ReplayClose,
    /// Resume the live game from the selected turn (caller-handled in main.rs).
    ReplayResume,
}

// ── key_to_command ────────────────────────────────────────────────────────────

/// The result of resolving a `KeyEvent` against the current `AppState`.
///
/// Hardwired keys, modal sub-modes, and per-focus text entry resolve directly
/// to an `Action`. KeyMap lookups resolve to a command-string plus the context
/// it was looked up in, so the run loop can dispatch it through
/// `slash::parse_in_context` exactly as if the user had typed the command.
#[derive(Debug)]
pub enum KeyResolve {
    /// A hardwired / modal / text-entry action to apply directly.
    Action(crate::input::Action),
    /// A keymap-resolved command string to dispatch through the slash parser,
    /// together with the context it was resolved in.
    Command(String, crate::keymap::Context),
    /// The key produced nothing.
    None,
}

/// Resolve a crossterm `KeyEvent` to a `KeyResolve` given the current `AppState`.
///
/// Routing order:
/// 1. Ctrl+Q / Ctrl+C → Quit (hardwired, always wins).
/// 2. (text-entry / confirm-delete modals are intercepted in the run loop.)
/// 3. Tidy-anim active → Anim context lookup (Ctrl+Left/Right stage-jump hardwired).
///    4-6. Modal sub-modes (saves/replay/file-browser/verb-menu/
///    config-screen/hotkey-dialog/room-panel) → their handlers (hardwired Actions).
/// 7. Key == hotkeys.prefix → OpenHotkeyDialog.
/// 8. Tab (no modifiers) → autocomplete-or-ToggleFocus.
/// 9. Ctrl modifier → Global KeyMap lookup, filtered by hotkeys.is_direct_name.
/// 10. Per-focus routing:
///     - Game: game_key_to_action, then Global fallthrough (non-ctrl non-printable).
///     - Map: Map context lookup, filtered by hotkeys.is_direct_name.
pub fn key_to_command(state: &AppState, key: KeyEvent) -> KeyResolve {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // 1. Quit always wins — even while a prompt is active.
    if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c')) {
        return KeyResolve::Action(Action::Quit);
    }

    // 2. The text-entry / confirm-delete modals are intercepted in the run loop
    //    before key routing (like the save-name dialog), so they never reach here.

    // 3. Tidy-animation sub-mode: KeyMap lookup in Anim context; no fallthrough.
    if state.tidy_anim.is_some() {
        if key.modifiers == KeyModifiers::CONTROL {
            match key.code {
                KeyCode::Left => return KeyResolve::Action(Action::AnimStageJump(-1)),
                KeyCode::Right => return KeyResolve::Action(Action::AnimStageJump(1)),
                _ => {}
            }
        }
        let spec = KeySpec::from_key_event(key);
        return match state.keymap.lookup(&spec, Context::Anim) {
            Some(s) => KeyResolve::Command(s.to_string(), Context::Anim),
            None => KeyResolve::None,
        };
    }

    // 4-6. Modal sub-modes: route to their handlers (all hardwired Actions).
    if state.overlays.saves.is_some() {
        return KeyResolve::Action(saves_key_to_action(key, state.overlays.dialog_focus));
    }
    if state.overlays.file_picker.is_some() {
        return KeyResolve::Action(file_picker_key_to_action(key));
    }
    if state.overlays.replay.is_some() {
        return KeyResolve::Action(history_key_to_action(key));
    }
    if state.overlays.file_browser.is_some() {
        return KeyResolve::Action(filebrowser_key_to_action(key));
    }
    // 6.6. Resize mode: Tab cycles the target pane, arrows adjust it, 0 resets,
    // Esc/Enter exits. Placed ABOVE the verb-menu intercept (SQ-0238) so resize
    // mode owns Tab/arrows/0/Esc/Enter even when the verb menu is open — the two
    // now coexist, with resize targeting the verb dock and Esc/Enter dropping
    // back to the still-open menu. (config_screen/hotkey_dialog below can never
    // be active together with resize mode, so their relative order is moot.)
    if state.resize_mode {
        return KeyResolve::Action(resize_mode_key_to_action(key));
    }
    if state.overlays.verb_menu.is_some() {
        if let Some(a) = verb_menu_intercept(key, state) {
            return KeyResolve::Action(a);
        }
        // else: fall through — the story input is live, so the key is handled normally.
    }
    if state.overlays.config_screen.is_some() {
        return KeyResolve::Action(config_screen_key_to_action(key, state.overlays.dialog_focus));
    }
    if state.overlays.hotkey_dialog {
        return hotkey_dialog_key_to_action(state, key);
    }

    // 6.5. Room panel close: Esc or Enter while a panel is open.
    // Comes after steps 2-6 (prompt/anim/saves/hotkey_dialog checks) so those
    // modes still take priority, but before the prefix key and normal dispatch.
    // Room panel is read-only (no text input), so Enter is safe as a close key.
    if state.room_panel.is_some() && key.modifiers == KeyModifiers::NONE
        && matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            return KeyResolve::Action(Action::CloseRoomPanel);
        }

    // [more] pager (SQ-0404): while it's showing, keypresses page the transcript
    // instead of reaching the game. Space/PgDn/↓/Enter advance one screen; any
    // other key jumps to the bottom and dismisses.
    if state.pager.active {
        return match key.code {
            KeyCode::Char(' ') | KeyCode::PageDown | KeyCode::Down | KeyCode::Enter => {
                KeyResolve::Action(Action::PagerAdvance)
            }
            _ => KeyResolve::Action(Action::PagerDismiss),
        };
    }

    // 7. Prefix key → open the hotkey dialog.
    let spec = KeySpec::from_key_event(key);
    if spec == state.hotkeys.prefix {
        return KeyResolve::Action(Action::OpenHotkeyDialog);
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
            return KeyResolve::Action(Action::Autocomplete);
        }
        return KeyResolve::Action(Action::ToggleFocus);
    }

    // 8b. Shift-Tab (BackTab): mid-word with suggestions in game focus →
    //     AutocompletePrev (the reverse of step 8's Autocomplete). Otherwise it
    //     reverses the per-window focus cycle (the mirror of step 8's ToggleFocus).
    if key.code == KeyCode::BackTab
        && state.focus == Focus::Game
        && !state.current_partial().is_empty()
        && !state.suggestions.is_empty()
    {
        return KeyResolve::Action(Action::AutocompletePrev);
    }
    if key.code == KeyCode::BackTab {
        return KeyResolve::Action(Action::CycleFocusBack);
    }

    // 9. Ctrl modifier: Global KeyMap lookup, filtered by is_direct_name — same
    //    rule as Map context. A command is reachable directly iff it is in the
    //    direct set, regardless of whether it uses a Ctrl modifier.
    if ctrl {
        return match state.keymap.lookup(&spec, Context::Global) {
            Some(s) if state.hotkeys.is_direct_name(s) => {
                KeyResolve::Command(s.to_string(), Context::Global)
            }
            _ => KeyResolve::None,
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
                return KeyResolve::Action(a);
            }
            // Global fallthrough for non-ctrl non-Tab non-printable keys.
            match state.keymap.lookup(&spec, Context::Global) {
                Some(s) => KeyResolve::Command(s.to_string(), Context::Global),
                None => KeyResolve::None,
            }
        }
        Focus::Map => {
            // Map context lookup with direct filter: only return the command if it
            // is in the direct (always-available) set. Dialog-only commands return
            // None when the dialog is closed.
            match state.keymap.lookup(&spec, Context::Map) {
                Some(s) if state.hotkeys.is_direct_name(s) => {
                    KeyResolve::Command(s.to_string(), Context::Map)
                }
                _ => KeyResolve::None,
            }
        }
    }
}

/// Backward-compatible shim: resolve a key straight to an `Action`.
///
/// Production dispatch consumes `key_to_command` directly so command-strings
/// flow through the slash parser. This wrapper is retained for tests and any
/// caller that only needs the `Action` form: command-strings that parse to a
/// plain `Action` are returned as such; Save/Load/Reset/Quit outcomes (and
/// parse errors) collapse to `Action::None`.
pub fn key_to_action(state: &AppState, key: KeyEvent) -> Action {
    match key_to_command(state, key) {
        KeyResolve::Action(a) => a,
        KeyResolve::Command(s, ctx) => {
            match crate::slash::parse_in_context(&s, state.config.command_prefix, ctx) {
                crate::slash::SlashOutcome::Action(a) => a,
                _ => Action::None,
            }
        }
        KeyResolve::None => Action::None,
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

/// Per-modal action mapping for the room-info panel ([X] → CloseRoomPanel).
fn roominfo_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::CloseRoomPanel);
        }
    }

    // Check buttons: Ok → CloseRoomPanel
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Ok => Action::CloseRoomPanel,
                _            => Action::None,
            });
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
    use crate::render::dialog::ButtonId;

    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::AnimExit);
        }
    }

    // Check buttons: Ok → AnimExit
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Ok => Action::AnimExit,
                _            => Action::None,
            });
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

/// Map a vertical mouse-wheel event to a step: `ScrollUp` → `-1`, `ScrollDown`
/// → `+1`, swapped when `invert`; `None` for non-wheel events. The single place
/// wheel direction and the `mouse_wheel_invert` preference are resolved, so no
/// surface re-implements the invert.
pub fn wheel_delta(kind: MouseEventKind, invert: bool) -> Option<isize> {
    let base = match kind {
        MouseEventKind::ScrollUp => -1,
        MouseEventKind::ScrollDown => 1,
        _ => return None,
    };
    Some(if invert { -base } else { base })
}

/// Map a crossterm `MouseEvent` to an `Action` given the current `AppState`, the
/// bounding rects of the map and story panes, the pre-computed room screen
/// rects (needed for pixel-accurate room hit-testing on left/right clicks), and
/// the active dialog chrome rects (if a dialog is open).
///
/// When `dialog` is `Some`, dialog hit-testing runs FIRST:
/// - close `[X]` click → the active modal's close action
/// - button click → the button's mapped action
/// - any click OUTSIDE the dialog `area` → swallowed (Action::None)
///   Only when no dialog is open does normal map/room routing apply.
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

    // Honor the user's wheel-direction preference: when mouse_wheel_invert is
    // set, swap scroll up/down (some terminals report "natural" scrolling).
    // Computed once here and reused by both the modal-precedence branch below
    // and the map/story wheel arms; never invert twice for one event.
    let kind = match (m.kind, state.config.mouse_wheel_invert) {
        (MouseEventKind::ScrollUp, true) => MouseEventKind::ScrollDown,
        (MouseEventKind::ScrollDown, true) => MouseEventKind::ScrollUp,
        (k, _) => k,
    };

    // ── Mouse-wheel precedence for open scrollable modals ─────────────────────
    // When a scrollable overlay is open, the wheel drives THAT surface's vertical
    // navigation, ahead of the underlying map/story and ahead of the dialog
    // chrome hit-testing below. Wheel up → previous/up; wheel down → next/down.
    // Each list modal reuses its existing Up/Down nav action (no new scroll
    // state). Corner overlays (room panel, tidy) are intentionally absent — the
    // wheel still pans the map under them, as before.
    // `kind` already has the single mouse_wheel_invert applied (above), so map it
    // to a direction with the shared helper and invert=false (never twice).
    let wheel_up = wheel_delta(kind, false).map(|d| d < 0);
    if let Some(up) = wheel_up {
        // Priority mirrors the keyboard modal routing order above.
        if state.overlays.config_screen.is_some() {
            return if up { Action::ConfigNav(-1) } else { Action::ConfigNav(1) };
        }
        if state.overlays.saves.is_some() {
            return if up { Action::SavesNav(-1) } else { Action::SavesNav(1) };
        }
        if state.overlays.replay.is_some() {
            return if up { Action::ReplayStep(-1) } else { Action::ReplayStep(1) };
        }
        if state.overlays.file_browser.is_some() {
            return if up { Action::FbNav(-1) } else { Action::FbNav(1) };
        }
        if state.overlays.verb_menu.is_some() {
            return if up {
                Action::VerbMenuNav(VerbMenuNavKind::Up)
            } else {
                Action::VerbMenuNav(VerbMenuNavKind::Down)
            };
        }
    }

    // ── Dialog chrome hit-testing (checked FIRST) ─────────────────────────────
    if let Some(rects) = dialog {
        if matches!(m.kind, MouseEventKind::Down(MouseButton::Left)) {
            // Corner overlays (room panel, tidy panel): only intercept the [X] click;
            // all other clicks fall through to normal map/room routing below.
            // But if a centered modal is also open (stacked on top), it takes
            // priority and must swallow all outside clicks.
            let centered_open = state.overlays.config_screen.is_some()
                || state.overlays.saves.is_some() || state.overlays.file_browser.is_some()
                || state.overlays.hotkey_dialog;
            let is_corner_overlay = !centered_open
                && (state.room_panel.is_some() || state.tidy_anim.is_some());

            if state.overlays.config_screen.is_some() {
                if let Some(action) = config_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.overlays.saves.is_some() {
                if let Some(action) = saves_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.overlays.file_browser.is_some() {
                if let Some(action) = filebrowser_dialog_action(rects, col, row) {
                    return action;
                }
            } else if state.overlays.hotkey_dialog {
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
            let centered_open = state.overlays.config_screen.is_some()
                || state.overlays.saves.is_some() || state.overlays.file_browser.is_some()
                || state.overlays.hotkey_dialog;
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

    match kind {
        // ── Left-down on the input line: place the caret ──────────────────────
        // Must precede the story arm below: the input line sits inside the story pane, so a click
        // on it would otherwise start a text selection instead of moving the caret (SQ-0354).
        MouseEventKind::Down(MouseButton::Left)
            if state.focus == Focus::Game && state.input_click_index(col, row).is_some() =>
        {
            Action::CursorToClick(col, row)
        }
        // ── Left-down in story: activate game pane + begin text selection ─────
        MouseEventKind::Down(MouseButton::Left) if in_story => {
            Action::StartSelection(col, row)
        }
        // ── Left-drag: extend an in-progress story selection ──────────────────
        MouseEventKind::Drag(MouseButton::Left) => {
            Action::ExtendSelection(col, row)
        }
        // ── Left-up: finish a story selection (copy on release) ───────────────
        MouseEventKind::Up(MouseButton::Left) => {
            Action::EndSelection
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
                Action::ZoomInFine
            } else if shift {
                Action::Pan(-1, 0)
            } else {
                Action::Pan(0, -1)
            }
        }
        MouseEventKind::ScrollDown if in_map => {
            if ctrl {
                Action::ZoomOutFine
            } else if shift {
                Action::Pan(1, 0)
            } else {
                Action::Pan(0, 1)
            }
        }
        MouseEventKind::ScrollLeft if in_map => Action::Pan(-1, 0),
        MouseEventKind::ScrollRight if in_map => Action::Pan(1, 0),
        // ── Wheel in story: scroll transcript ────────────────────────────────
        // Wheel up = scroll up into older history; wheel down = toward newest.
        MouseEventKind::ScrollUp if in_story => Action::TranscriptScroll(1),
        MouseEventKind::ScrollDown if in_story => Action::TranscriptScroll(-1),
        // ── Everything else ───────────────────────────────────────────────────
        _ => Action::None,
    }
}

// ── Internal: hotkey dialog key routing ───────────────────────────────────────

/// When the hotkey dialog is open, route keys to either close the dialog or
/// fire the bound command. The dialog closes itself when a sub-mode
/// opens (handled in apply_action).
fn hotkey_dialog_key_to_action(state: &AppState, key: KeyEvent) -> KeyResolve {
    // ESC or Enter always closes the hotkey dialog (same as [X] / [Done]).
    // Enter is handled before the leader-letter lookup to prevent the
    // Anim/AnimExit binding from firing when the hotkey dialog is open.
    if matches!(key.code, KeyCode::Esc | KeyCode::Enter) && key.modifiers == KeyModifiers::NONE {
        return KeyResolve::Action(Action::CloseHotkeyDialog);
    }

    let spec = KeySpec::from_key_event(key);

    // Prefix key closes the dialog.
    if spec == state.hotkeys.prefix {
        return KeyResolve::Action(Action::CloseHotkeyDialog);
    }

    // A bare character (no Ctrl/Alt; Shift allowed) is an authored leader
    // letter: fire its bound command, or close the dialog if unbound. Any
    // other key (Ctrl-combos, Alt-combos, function keys, arrows, etc.) also
    // closes the dialog. This gives tmux-style one-shot semantics: exactly
    // one keypress always resolves the dialog.
    if let KeyCode::Char(c) = key.code {
        if !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT) {
            return match state.hotkeys.leader_command(c) {
                Some(cmd) => {
                    let name = cmd.split_whitespace().next().unwrap_or("");
                    let ctx = crate::slash::find_command(name)
                        .map(|c| c.context)
                        .unwrap_or(Context::Global);
                    KeyResolve::Command(cmd.to_string(), ctx)
                }
                None => KeyResolve::Action(Action::CloseHotkeyDialog),
            };
        }
    }

    KeyResolve::Action(Action::CloseHotkeyDialog)
}

// ── Internal: saves-manager key routing ───────────────────────────────────────

/// Hardwired saves-manager sub-mode keys (not rebindable, like prompt and anim).
///
/// `focus` is the current button-focus index within the saves button ring:
///   0 = Done (close). Ring length is 1; the [Done] button is the only button.
/// Tab/BackTab are handled upstream (main.rs intercept) and never reach here.
/// The saves dialog has only one button (Done); Enter continues to load the
/// selected save (existing behavior) rather than activating the focused button,
/// since no Load button exists in the button row.
fn saves_key_to_action(key: KeyEvent, _focus: usize) -> Action {
    match key.code {
        KeyCode::Up => Action::SavesNav(-1),
        KeyCode::Down => Action::SavesNav(1),
        KeyCode::PageUp => Action::SavesPage(-1),
        KeyCode::PageDown => Action::SavesPage(1),
        KeyCode::Home => Action::SavesHome,
        KeyCode::End => Action::SavesEnd,
        KeyCode::Enter => Action::SavesLoad,
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::SavesSaveAs,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::SavesDelete,
        KeyCode::Char('i') if key.modifiers == KeyModifiers::NONE => Action::SavesImport,
        KeyCode::Esc => Action::SavesClose,
        _ => Action::None,
    }
}

/// Hardwired VFS file-picker sub-mode keys (not rebindable, like saves/anim).
fn file_picker_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::FilePickerNav(-1),
        KeyCode::Down => Action::FilePickerNav(1),
        KeyCode::Enter => Action::FilePickerPick,
        KeyCode::Esc => Action::FilePickerClose,
        _ => Action::None,
    }
}

/// Hardwired replay/rewind sub-mode keys (not rebindable, like saves/anim).
fn history_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Left => Action::ReplayStep(-1),
        KeyCode::Right => Action::ReplayStep(1),
        KeyCode::PageUp => Action::ReplayPage(-1),
        KeyCode::PageDown => Action::ReplayPage(1),
        KeyCode::Home => Action::ReplayStep(i32::MIN as isize),
        KeyCode::End => Action::ReplayStep(i32::MAX as isize),
        KeyCode::Char(' ') => Action::ReplayTogglePlay,
        KeyCode::Enter | KeyCode::Char('r') => Action::ReplayResume,
        KeyCode::Esc | KeyCode::Char('q') => Action::ReplayClose,
        _ => Action::None,
    }
}

// ── Internal: file-browser key routing ───────────────────────────────────────

/// Hardwired file-browser sub-mode keys.
fn filebrowser_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Up => Action::FbNav(-1),
        KeyCode::Down => Action::FbNav(1),
        KeyCode::PageUp => Action::FbPage(-1),
        KeyCode::PageDown => Action::FbPage(1),
        KeyCode::Home => Action::FbHome,
        KeyCode::End => Action::FbEnd,
        KeyCode::Enter => Action::FbEnter,
        KeyCode::Esc => Action::FbClose,
        _ => Action::None,
    }
}

// ── Internal: verb-menu key routing ──────────────────────────────────────────

/// Focus-aware key intercept for the verb dock. Returns `Some(action)` ONLY for
/// keys the dock consumes given the current focus, else `None` (the key falls
/// through and is handled by the always-live story input).
fn verb_menu_intercept(key: KeyEvent, state: &AppState) -> Option<Action> {
    // The hotkey-dialog leader prefix is never swallowed by the dock: it must
    // fall through so the player can open the leader palette (and reach
    // resize-panes → 'z') while the verb menu is open (SQ-0238). This guard is
    // first so it wins even if the prefix is bound to a key the dock would
    // otherwise consume (Tab/Esc/an arrow).
    if KeySpec::from_key_event(key) == state.hotkeys.prefix {
        return None;
    }

    // Tab / Shift-Tab cycle focus through the ring (incl. story) and Esc closes
    // the dock regardless of which element is focused.
    match key.code {
        KeyCode::Tab => return Some(Action::VerbMenuNav(VerbMenuNavKind::NextPane)),
        KeyCode::BackTab => return Some(Action::VerbMenuNav(VerbMenuNavKind::PrevPane)),
        KeyCode::Esc => return Some(Action::VerbMenuClose),
        _ => {}
    }

    // The remaining keys are only consumed when a dock pane is focused; while the
    // story is focused they belong to the game input (letters, Enter, ↑/↓ history).
    let story_focused = state.overlays.verb_menu.as_ref().map(|vm| vm.story_focused).unwrap_or(true);
    if story_focused {
        return None;
    }

    match key.code {
        KeyCode::Up => Some(Action::VerbMenuNav(VerbMenuNavKind::Up)),
        KeyCode::Down => Some(Action::VerbMenuNav(VerbMenuNavKind::Down)),
        KeyCode::PageUp => Some(Action::VerbMenuNav(VerbMenuNavKind::PageUp)),
        KeyCode::PageDown => Some(Action::VerbMenuNav(VerbMenuNavKind::PageDown)),
        KeyCode::Home => Some(Action::VerbMenuNav(VerbMenuNavKind::Home)),
        KeyCode::End => Some(Action::VerbMenuNav(VerbMenuNavKind::End)),
        KeyCode::Left => Some(Action::VerbMenuNav(VerbMenuNavKind::PrevPane)),
        KeyCode::Right => Some(Action::VerbMenuNav(VerbMenuNavKind::NextPane)),
        KeyCode::Enter | KeyCode::Char(' ') => Some(Action::VerbMenuPick),
        _ => None,
    }
}

// ── Internal: resize-mode key routing ────────────────────────────────────────

/// Hardwired resize-mode sub-mode keys (not rebindable).
///
/// Tab / Shift+Tab → next/prev visible pane; arrows → grow/shrink the target;
/// `0` → reset to defaults; Esc/Enter → exit.
fn resize_mode_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Tab => Action::ResizeNav(ResizeNavKind::NextTarget),
        KeyCode::BackTab => Action::ResizeNav(ResizeNavKind::PrevTarget),
        KeyCode::Left => Action::ResizeNav(ResizeNavKind::Left),
        KeyCode::Right => Action::ResizeNav(ResizeNavKind::Right),
        KeyCode::Up => Action::ResizeNav(ResizeNavKind::Up),
        KeyCode::Down => Action::ResizeNav(ResizeNavKind::Down),
        KeyCode::Char('0') => Action::ResizeReset,
        KeyCode::Esc | KeyCode::Enter => Action::ResizeExit,
        _ => Action::None,
    }
}

// ── Internal: config-screen key routing ──────────────────────────────────────

/// `focus` is the current button-focus index within the config-screen ring:
///   0 = Save, 1 = Cancel. Ring length is 2.
/// Tab/BackTab are handled upstream (main.rs intercept) and never reach here.
/// Enter activates the focused button; Space still toggles the selected row.
fn config_screen_key_to_action(key: KeyEvent, focus: usize) -> Action {
    match key.code {
        KeyCode::Up => Action::ConfigNav(-1),
        KeyCode::Down => Action::ConfigNav(1),
        KeyCode::PageUp => Action::ConfigPage(-1),
        KeyCode::PageDown => Action::ConfigPage(1),
        KeyCode::Home => Action::ConfigHome,
        KeyCode::End => Action::ConfigEnd,
        KeyCode::Left => Action::ConfigCycle(-1),
        KeyCode::Right => Action::ConfigCycle(1),
        KeyCode::Char(' ') if key.modifiers == KeyModifiers::NONE => Action::ConfigToggle,
        KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
            // Ring: [Save(0), Cancel(1)]. Enter activates the focused button.
            match focus {
                1 => Action::ConfigCancel,
                _ => Action::ConfigSave, // default: Save (focus 0)
            }
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
        // pan and the non-typeable Home recenters; PageUp/PageDown page the
        // transcript. None of these clash with typing a command (arrows/Home/PageX
        // aren't printable, and Shift+Arrow is distinct from a Shift+letter capital).
        KeyCode::Left if shift => Action::Pan(-1, 0),
        KeyCode::Right if shift => Action::Pan(1, 0),
        KeyCode::Up if shift => Action::Pan(0, -1),
        KeyCode::Down if shift => Action::Pan(0, 1),
        // Plain Up/Down recall command history (shell-style).
        KeyCode::Up if key.modifiers == KeyModifiers::NONE => Action::HistoryPrev,
        KeyCode::Down if key.modifiers == KeyModifiers::NONE => Action::HistoryNext,
        // Caret editing on the command line (SQ-0354). Plain Left/Right were unbound here and fell
        // through to the Global keymap; they are text keys the moment the line has a caret.
        //
        // Safe against story-controlled input: when the story asks for a single keypress the run
        // loop's char-mode gate forwards the key straight to the VM and never reaches this
        // function. Shift+Arrows still pan the map, and plain Up/Down still recall history.
        KeyCode::Left if key.modifiers == KeyModifiers::NONE => Action::CursorLeft,
        KeyCode::Right if key.modifiers == KeyModifiers::NONE => Action::CursorRight,
        KeyCode::Delete => Action::DeleteChar,
        // Home/End are the conventional line-editing pair, matching every other text entry in the
        // app. Home used to recenter the map; that keeps its `center-map` command and hotkey.
        KeyCode::Home => Action::CursorHome,
        KeyCode::End => Action::CursorEnd,
        // PageUp/PageDown page the transcript (toward older / newer). Zoom stays
        // on +/=/-/0, Ctrl+wheel, and /zoom-map.
        KeyCode::PageUp => Action::TranscriptScrollPage(1),
        KeyCode::PageDown => Action::TranscriptScrollPage(-1),
        // Enter submits the current input buffer content as the command.
        KeyCode::Enter => Action::SubmitCommand(state.input.value.clone()),
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

// ── Focus cycling ─────────────────────────────────────────────────────────────

/// Compute the new transcript scroll offset for a one-page step.
///
/// `dir > 0` scrolls toward older lines (increasing the offset); `dir < 0` toward
/// newer. The step is `viewport_rows - 1` (a one-line overlap for reading
/// continuity), floored at 1 so paging always progresses, and the result is
/// clamped to `[0, max_scroll]` — the same bounds the mouse-wheel scroll uses.
pub fn page_scroll(current: u16, dir: i8, viewport_rows: u16, max_scroll: u16) -> u16 {
    let next = crate::list_scroll::page_step(current as usize, dir as i32, viewport_rows as usize);
    (next.min(u16::MAX as usize) as u16).min(max_scroll)
}

/// Cycle a button-focus index by `delta` (+1 Tab, -1 Shift-Tab), wrapping within
/// `0..len`. Returns 0 when `len` is 0.
pub fn cycle_focus(idx: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let next = idx as i32 + delta;
    next.rem_euclid(len as i32) as usize
}

// ── apply_action ──────────────────────────────────────────────────────────────

/// Apply a view or light-correction action to `state` and/or `mapper`.
///
/// **Caller-handled actions** (silently ignored here — the run loop must act on
/// them): `SubmitCommand` (game focus), `SaveGame`, `RestoreGame`, `ExportSvg`,
/// `Quit`.
///
/// The former bottom-bar prompt sub-mode is gone: rename/notes/relabel/layer/
/// config-path/create-file open the `text_entry` modal (submit via
/// [`apply_text_entry`]), and delete-save opens the `confirm_delete_save` modal.
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
    // ── Normal action dispatch ────────────────────────────────────────────
    match action {
        Action::InputChar(c) => {
            state.push_input_char(c);
            // Recompute suggestions after every character typed in game focus.
            if state.focus == Focus::Game {
                recompute_suggestions(state);
                state.suggestion_idx = 0;
                state.suggestion_active = false;
            }
        }
        Action::Backspace => {
            state.backspace();
            // Recompute suggestions after deletion in game focus.
            if state.focus == Focus::Game {
                recompute_suggestions(state);
                state.suggestion_idx = 0;
                state.suggestion_active = false;
            }
        }
        Action::Autocomplete => {
            // Apply the currently-highlighted suggestion to the input buffer,
            // replacing the partial word being typed. Then advance the index
            // so repeated Tab cycles through candidates.
            if !state.suggestions.is_empty() {
                let len = state.suggestions.len();
                // First Tab applies the highlighted candidate (the preview at
                // `suggestion_idx`); only later presses advance so the bracket
                // stays on the word now in the input.
                if state.suggestion_active {
                    state.suggestion_idx = (state.suggestion_idx + 1) % len;
                } else {
                    state.suggestion_active = true;
                }
                let idx = state.suggestion_idx % len;
                let completion = state.suggestions[idx].clone();
                apply_completion(state, &completion);
            }
        }
        Action::AutocompletePrev => {
            // Inverse of Autocomplete: apply the currently-highlighted suggestion,
            // then step the index BACKWARD (wrapping) so repeated Shift-Tab cycles
            // through candidates in reverse.
            if !state.suggestions.is_empty() {
                let len = state.suggestions.len();
                // First Shift-Tab applies the highlighted candidate; only later
                // presses step backward so the bracket stays on the applied word.
                if state.suggestion_active {
                    state.suggestion_idx = (state.suggestion_idx + len - 1) % len;
                } else {
                    state.suggestion_active = true;
                }
                let idx = state.suggestion_idx % len;
                let completion = state.suggestions[idx].clone();
                apply_completion(state, &completion);
            }
        }
        Action::HistoryPrev => state.history_prev(),
        Action::HistoryNext => state.history_next(),
        Action::ToggleFocus => state.toggle_focus(),
        Action::CycleFocusBack => state.cycle_focus(false),
        Action::ToggleMap => {
            state.toggle_map();
            // Persist the map panel's on/off state per-game so it's restored the
            // next time this story opens (SQ-0304). No game_dir → no sidecar (and
            // keeps unit tests off the filesystem).
            if !state.game_dir.as_os_str().is_empty() {
                let show = state.layout == crate::state::Layout::Split;
                let _ = crate::styles::write_per_game_show_map(&state.game_dir, Some(show));
            }
        }
        Action::CursorLeft => state.input.left(),
        Action::CursorHome => state.input.home(),
        Action::CursorEnd => state.input.end(),
        Action::DeleteChar => {
            state.input.delete();
            recompute_suggestions(state);
        }
        Action::CursorRight => {
            // At the END of the line with a suggestion showing, Right ACCEPTS it (SQ-0354) — the
            // fish/zsh gesture. There is no text to the right to move onto, so the keystroke would
            // otherwise do nothing at all; taking the completion is the only useful reading.
            //
            // Applying it exactly as Tab does keeps the two paths honest: `suggestion_active`
            // flips, so the bracketed preview reads as applied rather than still just a preview.
            let at_end = state.input.cursor >= state.input.char_len();
            match state.suggestions.get(state.suggestion_idx % state.suggestions.len().max(1)) {
                Some(c) if at_end => {
                    let completion = c.clone();
                    state.suggestion_active = true;
                    apply_completion(state, &completion);
                }
                _ => state.input.right(),
            }
        }
        Action::CursorToClick(col, row) => {
            if let Some(idx) = state.input_click_index(col, row) {
                state.input.cursor = idx;
            }
        }
        Action::ZoomIn => state.zoom_in(),
        Action::ZoomOut => state.zoom_out(),
        Action::ZoomBy(n) => state.zoom_by(n),
        Action::ZoomInFine => state.zoom_in_fine(),
        Action::ZoomOutFine => state.zoom_out_fine(),
        Action::ZoomReset => state.zoom_reset(),
        Action::Pan(dx, dy) => state.pan(dx, dy),
        Action::Recenter => apply_recenter(state, mapper),
        Action::SelectNext => select_adjacent(state, mapper, 1),
        Action::SelectPrev => select_adjacent(state, mapper, -1),

        Action::PeelLayer(dir) => {
            let Some(room) = state.selected_room.or_else(|| mapper.graph.current()) else {
                state.notifications.push("peel-layer: no room selected");
                return;
            };
            // With a direction, the player names the seam; without one, peel looks for a portal
            // seam already dividing the layer (SQ-0360).
            let peeled = match dir {
                Some(d) => mapper::layer::peel_at_edge(&mut mapper.graph, room, d),
                None => mapper::layer::peel_region(&mut mapper.graph, room),
            };
            match peeled {
                Ok(new) => {
                    state.bump_graph_gen(); // rooms peeled into a new layer → invalidate memo (SQ-0305)
                    state.set_viewed_layer(Some(new));
                }
                // A refusal used to be silent, which reads as a broken command — say which of the
                // several quite different reasons it was (SQ-0360).
                Err(why) => state.notifications.push(peel_refusal_message(&mapper.graph, room, dir, why)),
            }
        }
        Action::MergeLayer => {
            let active = state.active_layer(&mapper.graph);
            let target = mapper::layer::merge_layer(&mut mapper.graph, active); // into its parent
            state.bump_graph_gen(); // layer merged into parent → invalidate memo (SQ-0305)
            // Follow the rooms to where they landed. Clearing the view instead sent it to whatever
            // layer the PLAYER happens to stand in — usually the top one — so a merge looked like
            // it had dumped the rooms there, when they had gone to the parent all along and were
            // simply off-screen. Mirrors peel, which pins the view to the layer it creates
            // (SQ-0361).
            state.set_viewed_layer(Some(target));
        }

        // Re-tidy: re-derive the clean Auto layout (constrained stress majorization,
        // or the longest-path sort for very large maps), then nudge rooms so the lane
        // router has no illegal overlaps. Honours compass ordering the greedy per-turn
        // placement can't. No-op in Manual mode — those positions are user-owned.
        Action::Retidy => {
            // Re-tidy off the main thread so the progress bar shows and the UI stays
            // live during a long tidy on a large map — same machinery as `animate-tidy`,
            // but `animate: false` so the run loop applies the tidied graph instantly
            // (no animation playback). Guard against a double-spawn while a build is in
            // flight or an animation is showing. (SQ-0261)
            if mapper.mode == mapper::layout::LayoutMode::Auto
                && state.anim_build_job.is_none()
                && state.tidy_anim.is_none()
            {
                let layer = state.active_layer(&mapper.graph);
                let mut g = mapper.graph.clone();
                let gen = state.graph_gen;
                let total = mapper.graph.rooms_in_layer(layer).len() + 8;
                let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let progress_clone = std::sync::Arc::clone(&progress);
                let handle = std::thread::spawn(move || {
                    let frames = crate::tidy::run_tidy_pipeline(&mut g, layer, Some(progress_clone));
                    (frames, g)
                });
                state.anim_build_job = Some(crate::state::AnimBuildJob {
                    handle,
                    layer,
                    gen,
                    started: std::time::Instant::now(),
                    progress,
                    total,
                    animate: false,
                });
                state.set_status("tidying map…");
            }
        }

        Action::ReloadStyle => {
            match crate::reload::reload_style(state) {
                crate::reload::ReloadOutcome::Reloaded { warnings } => {
                    for w in &warnings {
                        state.push_transcript_kind(w, crate::state::TranscriptKind::Warning);
                    }
                    state.set_status("style reloaded");
                }
                crate::reload::ReloadOutcome::Failed { msg } => {
                    state.push_transcript_kind(
                        &format!("style reload failed: {}", msg),
                        crate::state::TranscriptKind::Warning,
                    );
                    state.set_status("reload failed — keeping current style");
                }
            }
        }

        Action::ToggleWatch => { /* handled in the run loop (owns the watcher) */ }

        Action::AnimateTidy => {
            // Build the animation frames on a worker thread so the UI stays responsive
            // during the (potentially long) build. The run loop polls the job, applies the
            // tidied graph, and installs the animation when it finishes. Guard against a
            // double-spawn while one build is in flight or an animation is already showing.
            if mapper.mode == mapper::layout::LayoutMode::Auto
                && state.anim_build_job.is_none()
                && state.tidy_anim.is_none()
            {
                let layer = state.active_layer(&mapper.graph);
                let mut g = mapper.graph.clone();
                let gen = state.graph_gen;
                // Estimate the final frame count from the layer's room count (one placement
                // frame per room dominates), plus headroom for the fixed layout/cleanup stages.
                // Only an estimate — the real total isn't known until the build finishes.
                let total = mapper.graph.rooms_in_layer(layer).len() + 8;
                let progress = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let progress_clone = std::sync::Arc::clone(&progress);
                let handle = std::thread::spawn(move || {
                    let frames = crate::tidy::run_tidy_pipeline(&mut g, layer, Some(progress_clone));
                    (frames, g)
                });
                state.anim_build_job = Some(crate::state::AnimBuildJob {
                    handle,
                    layer,
                    gen,
                    started: std::time::Instant::now(),
                    progress,
                    total,
                    animate: true,
                });
                state.set_status("preparing tidy animation…");
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

        Action::SetViewedLayer(layer) => {
            // A click on a map layer tab selects that layer as the viewed one.
            // set_viewed_layer + active_layer tolerate a stale id (falls back).
            state.set_viewed_layer(Some(layer));
        }

        Action::ToggleAlignment => state.show_alignment = !state.show_alignment,
        Action::TogglePortalLabels => state.show_portal_labels = !state.show_portal_labels,
        Action::ToggleRoomNumbers => state.show_room_numbers = !state.show_room_numbers,
        Action::ToggleLocMethod => state.show_loc_method = !state.show_loc_method,
        Action::ToggleStatusBar => state.show_status_bar = !state.show_status_bar,
        Action::ToggleTimedInput => {
            state.config.honor_timed_input = !state.config.honor_timed_input;
            state.set_status(if state.config.honor_timed_input { "timed input on" } else { "timed input off" });
        }
        Action::ToggleSound => {
            state.config.enable_sound = !state.config.enable_sound;
            state.set_status(if state.config.enable_sound { "sound on" } else { "sound off" });
            if !state.config.enable_sound {
                state.reset_sound_sidecars();
            } else if state.audio.is_none() {
                state.audio = Some(audio::AudioBackend::new(state.config.volume));
            }
            // Sync the running Glulx VM's Sound gestalt (applied by the event loop).
            state.pending_vm_sound = Some(state.config.enable_sound);
        }
        Action::SetVolume(v) => {
            let v = v.min(100);
            state.config.volume = v;
            state.set_status(format!("volume {v}"));
            if let Some(b) = state.audio.as_mut() { b.set_volume(v); }
        }
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
                state.overlays.hotkey_dialog = false;
                state.overlays.dialog_focus = 0;
                // Opens empty: an empty submit clears the room's custom label.
                state.overlays.text_entry = Some(TextEntryDialog::new(TextEntryKind::RenameRoom(id), ""));
            }
        }
        Action::RenameLayer => {
            let layer = state.active_layer(&mapper.graph);
            let current_name = mapper.graph.layer_name(layer).to_owned();
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
            state.overlays.text_entry =
                Some(TextEntryDialog::new(TextEntryKind::RenameLayer(layer), current_name));
        }
        Action::EditNotes => {
            if let Some(id) = state.selected_room {
                state.overlays.hotkey_dialog = false;
                state.overlays.dialog_focus = 0;
                // Opens empty: submit replaces the room's notes (empty clears them).
                state.overlays.text_entry = Some(TextEntryDialog::new(TextEntryKind::EditNotes(id), ""));
            }
        }
        Action::RelabelSelectedEdge => {
            if let Some(id) = state.selected_room {
                // Find the first outgoing connection for this room.
                if let Some(conn) =
                    mapper.graph.connections().iter().find(|c| c.origin == id)
                {
                    let old_dir = conn.dir;
                    state.overlays.hotkey_dialog = false;
                    state.overlays.dialog_focus = 0;
                    state.overlays.text_entry =
                        Some(TextEntryDialog::new(TextEntryKind::RelabelEdge(id, old_dir), ""));
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
                    state.bump_graph_gen(); // edge removed → invalidate map memo (SQ-0305)
                }
            }
        }
        Action::NudgeSelected(dx, dy) => {
            if let Some(id) = state.selected_room {
                if let Some(room) = mapper.graph.room(id) {
                    if let Some(pos) = room.pos {
                        let target = (pos.0 + dx, pos.1 + dy);
                        mapper.nudge(id, target);
                        state.bump_graph_gen(); // room moved → invalidate map memo + hit-testing (SQ-0305)
                    }
                }
            }
        }

        Action::OpenHotkeyDialog => {
            state.overlays.hotkey_dialog = true;
            // Close other overlays if open.
            state.overlays.saves = None;
        }

        Action::CloseHotkeyDialog => {
            state.overlays.hotkey_dialog = false;
        }

        // ── Saves-manager actions ─────────────────────────────────────────────

        Action::OpenSaves => {
            // The list must be populated by the caller (main.rs has dir + ifid).
            // apply_action only sets up the state; the caller refreshes the list
            // via AppState::open_saves_modal after apply_action returns.
            // If already open, do nothing.
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
        }

        Action::SavesNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                if !s.entries.is_empty() {
                    let len = s.entries.len();
                    s.scroll.len(len);
                    // Preserve the existing wrap-around behavior via select().
                    let next = ((s.scroll.selected as i32 + delta).rem_euclid(len as i32)) as usize;
                    s.scroll.select(next, vp, &anim);
                }
            }
        }

        Action::SavesPage(dir) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                let len = s.entries.len();
                s.scroll.len(len);
                s.scroll.page(dir, vp, &anim);
            }
        }

        Action::SavesHome => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                s.scroll.len(s.entries.len());
                s.scroll.home(vp, &anim);
            }
        }

        Action::SavesEnd => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(s) = &mut state.overlays.saves {
                let len = s.entries.len();
                s.scroll.len(len);
                s.scroll.end(len, vp, &anim);
            }
        }

        // SavesLoad, SavesSaveAs, SavesDelete: state-only pre-work here;
        // the actual I/O is caller-handled.

        Action::SavesSaveAs => {
            // Open the save-name dialog (a common-dialog modal, not a bottom-bar
            // prompt); on submit the caller performs the host Save State save.
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
            state.overlays.save_name_dialog = Some(crate::state::SaveNameDialog::new(
                crate::persist_files::default_save_name(),
                false,
            ));
        }

        Action::SavesDelete => {
            // Open the two-button confirm-delete dialog for the selected entry;
            // focus starts on Cancel (index 1), the safe default.
            if let Some(s) = &state.overlays.saves {
                if let Some(entry) = s.entries.get(s.scroll.selected) {
                    let path = entry.path.clone();
                    state.overlays.hotkey_dialog = false;
                    state.overlays.dialog_focus = 1;
                    state.overlays.confirm_delete_save = Some(path);
                }
            }
        }

        Action::SavesClose => {
            state.overlays.saves = None;
        }

        // SavesLoad is caller-handled.

        // ── VFS file-picker actions ─────────────────────────────────────────────

        Action::FilePickerNav(delta) => {
            if let Some(fp) = &mut state.overlays.file_picker {
                if delta < 0 { fp.move_up() } else { fp.move_down() }
            }
        }
        Action::FilePickerPick => {
            if let Some(fp) = &state.overlays.file_picker {
                if let Some(name) = fp.selected() {
                    state.filename_submitted = Some(Some(name.to_string()));
                }
            }
            state.overlays.file_picker = None;
        }
        Action::FilePickerClose => {
            // Leave pending_filename set: the run loop's resolver treats a closed
            // picker with a still-pending request as a cancel (NULL fileref).
            state.overlays.file_picker = None;
        }

        // ── File-browser actions ──────────────────────────────────────────────

        // SavesImport, FbEnter are caller-handled.

        Action::FbNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                if !fb.entries.is_empty() {
                    let len = fb.entries.len();
                    fb.scroll.len(len);
                    // Preserve the existing wrap-around behavior via select().
                    let next = ((fb.scroll.selected as i32 + delta).rem_euclid(len as i32)) as usize;
                    fb.scroll.select(next, vp, &anim);
                }
            }
        }

        Action::FbPage(dir) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                let len = fb.entries.len();
                fb.scroll.len(len);
                fb.scroll.page(dir, vp, &anim);
            }
        }

        Action::FbHome => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                fb.scroll.len(fb.entries.len());
                fb.scroll.home(vp, &anim);
            }
        }

        Action::FbEnd => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(fb) = &mut state.overlays.file_browser {
                let len = fb.entries.len();
                fb.scroll.len(len);
                fb.scroll.end(len, vp, &anim);
            }
        }

        Action::FbClose => {
            state.overlays.file_browser = None;
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
                // Grab-and-drag: the content follows the cursor (dragging right
                // moves the map right). char_pan is added to the draw offset, so
                // add the delta directly. 1-character precision.
                state.char_pan.0 += dx;
                state.char_pan.1 += dy;
            }
        }

        Action::EndDragPan => {
            state.drag = None;
        }

        Action::StartSelection(col, row) => {
            // Left-down in the story also activates the game pane.
            state.focus = Focus::Game;
            if let Some(g) = state.transcript_geom.get() {
                if let Some(p) = screen_to_point(g, col, row) {
                    state.selection = Some(crate::clipboard::Selection::new(p));
                    state.selection_edge = 0;
                }
            }
        }

        Action::ExtendSelection(col, row) => {
            if let Some(g) = state.transcript_geom.get() {
                if let Some(sel) = &mut state.selection {
                    if let Some(p) = screen_to_point(g, col, row) { sel.head = p; }
                }
                // Edge detection for auto-scroll: pointer at/above top → -1; at/below bottom → +1.
                state.selection_edge = if row <= g.area.y { -1 }
                    else if row >= g.area.bottom().saturating_sub(1) { 1 }
                    else { 0 };
                // Step once now so a drag that reaches the edge scrolls even without a tick.
                apply_selection_autoscroll(state);
            }
        }

        // Copy is emitted by the run loop from state.selection_text; just clear here.
        Action::EndSelection => {
            state.selection = None;
            state.selection_edge = 0;
        }

        // ── Transcript scroll ─────────────────────────────────────────────────

        Action::TranscriptScroll(delta) => {
            let target = if delta < 0 {
                state.transcript_scroll.saturating_sub((-delta) as u16)
            } else {
                state.transcript_scroll.saturating_add(delta as u16)
            };
            state.scroll_transcript_to(target);
        }

        Action::ToggleInventory => {
            state.show_inventory = !state.show_inventory;
            state.inv_dock.toggle_to(state.show_inventory, false);
            state.inv_dock.arm(&state.config.animation);
        }

        Action::OpenVerbMenu => {
            state.overlays.hotkey_dialog = false;
            let nouns = build_verb_menu_nouns(state, mapper);
            state.overlays.verb_menu = Some(crate::state::VerbMenuState {
                pane: crate::state::VerbMenuPane::Verbs,
                verb_scroll: Default::default(),
                noun_scroll: Default::default(),
                prep_scroll: Default::default(),
                nouns,
                story_focused: true,
            });
            state.verb_dock.toggle_to(true, false);
            state.verb_dock.arm(&state.config.animation);
        }

        Action::VerbMenuNav(kind) => {
            use crate::state::VerbMenuPane;
            use crate::render::verbmenu::{VERB_MENU_VERBS, VERB_MENU_PREPS};
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(vm) = &mut state.overlays.verb_menu {
                match kind {
                    // Focus ring INCLUDING the story pane: Story → Verbs → Nouns
                    // → Preps → Story.
                    VerbMenuNavKind::NextPane => {
                        if vm.story_focused {
                            vm.story_focused = false;
                            vm.pane = VerbMenuPane::Verbs;
                        } else {
                            match vm.pane {
                                VerbMenuPane::Verbs => vm.pane = VerbMenuPane::Nouns,
                                VerbMenuPane::Nouns => vm.pane = VerbMenuPane::Preps,
                                VerbMenuPane::Preps => vm.story_focused = true,
                            }
                        }
                    }
                    VerbMenuNavKind::PrevPane => {
                        if vm.story_focused {
                            vm.story_focused = false;
                            vm.pane = VerbMenuPane::Preps;
                        } else {
                            match vm.pane {
                                VerbMenuPane::Preps => vm.pane = VerbMenuPane::Nouns,
                                VerbMenuPane::Nouns => vm.pane = VerbMenuPane::Verbs,
                                VerbMenuPane::Verbs => vm.story_focused = true,
                            }
                        }
                    }
                    // List movement within `vm.pane` (clamped). Reached from the
                    // keyboard only when a pane is focused (the ↑/↓ intercept is
                    // gated on `!story_focused`), but ALSO from the mouse wheel
                    // regardless of focus — so it must scroll unconditionally.
                    _ => {
                        let (scroll, len) = match vm.pane {
                            VerbMenuPane::Verbs => (&mut vm.verb_scroll, VERB_MENU_VERBS.len()),
                            VerbMenuPane::Nouns => (&mut vm.noun_scroll, vm.nouns.len()),
                            VerbMenuPane::Preps => (&mut vm.prep_scroll, VERB_MENU_PREPS.len()),
                        };
                        scroll.len(len);
                        match kind {
                            VerbMenuNavKind::Up => scroll.move_by(-1, vp, &anim),
                            VerbMenuNavKind::Down => scroll.move_by(1, vp, &anim),
                            VerbMenuNavKind::PageUp => scroll.page(-1, vp, &anim),
                            VerbMenuNavKind::PageDown => scroll.page(1, vp, &anim),
                            VerbMenuNavKind::Home => scroll.home(vp, &anim),
                            VerbMenuNavKind::End => scroll.end(len, vp, &anim),
                            VerbMenuNavKind::NextPane | VerbMenuNavKind::PrevPane => {}
                        }
                    }
                }
            }
        }

        Action::VerbMenuClickToken(pane, idx) => {
            use crate::state::VerbMenuPane;
            use crate::render::verbmenu::{VERB_MENU_VERBS, VERB_MENU_PREPS};
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(vm) = &mut state.overlays.verb_menu {
                let (token, scroll, len) = match pane {
                    VerbMenuPane::Verbs => (VERB_MENU_VERBS.get(idx).copied().map(str::to_string), &mut vm.verb_scroll, VERB_MENU_VERBS.len()),
                    VerbMenuPane::Nouns => (vm.nouns.get(idx).cloned(), &mut vm.noun_scroll, vm.nouns.len()),
                    VerbMenuPane::Preps => (VERB_MENU_PREPS.get(idx).copied().map(str::to_string), &mut vm.prep_scroll, VERB_MENU_PREPS.len()),
                };
                scroll.len(len);
                scroll.select(idx, vp, &anim);
                if let Some(t) = token { if !t.is_empty() { state.input.insert_str(&t); state.input.insert(' '); } }
            }
        }

        Action::VerbMenuFocusPane(pane) => {
            if let Some(vm) = &mut state.overlays.verb_menu {
                vm.story_focused = false;
                vm.pane = pane;
            }
        }

        Action::VerbMenuPick => {
            use crate::render::verbmenu::{VERB_MENU_VERBS, VERB_MENU_PREPS};
            if let Some(vm) = &state.overlays.verb_menu {
                let token = vm.selected_token(VERB_MENU_VERBS, VERB_MENU_PREPS).to_owned();
                if !token.is_empty() {
                    state.input.insert_str(&token);
                    state.input.insert(' ');
                }
            }
        }

        Action::VerbMenuClose => {
            // Drawer pattern: keep `verb_menu` (content) alive so the panel
            // visibly slides out; `settle_verb_dock` clears it once the
            // slide-out animation finishes.
            state.verb_dock.toggle_to(false, false);
            state.verb_dock.arm(&state.config.animation);
        }

        // ── Resize mode actions ───────────────────────────────────────────────

        Action::ResizePanes => {
            let visible = state.resize_targets_visible();
            if let Some(first) = visible.first() {
                state.overlays.hotkey_dialog = false;
                state.resize_mode = true;
                state.resize_target = *first;
            }
        }

        Action::ResizeExit => {
            state.resize_mode = false;
            state.pending_config_write = true;
        }

        Action::ResizeReset => {
            state.reset_pane_sizes();
            state.pending_config_write = true;
        }

        Action::ResizeNav(ResizeNavKind::NextTarget) => state.cycle_resize_target(true),
        Action::ResizeNav(ResizeNavKind::PrevTarget) => state.cycle_resize_target(false),

        Action::ResizeNav(dir) => {
            const STEP: u16 = 3;
            use ResizeNavKind::*;
            match state.resize_target {
                crate::state::ResizeTarget::StoryMap => match dir {
                    Left => state.pane_sizes.split_ratio = state.pane_sizes.split_ratio.saturating_sub(STEP).max(20),
                    Right => state.pane_sizes.split_ratio = (state.pane_sizes.split_ratio + STEP).min(80),
                    _ => {}
                },
                crate::state::ResizeTarget::InvDock => match dir {
                    Up => state.pane_sizes.inv_dock_pct = (state.pane_sizes.inv_dock_pct + STEP).min(80),
                    Down => state.pane_sizes.inv_dock_pct = state.pane_sizes.inv_dock_pct.saturating_sub(STEP).max(10),
                    _ => {}
                },
                // Verb dock is a left-docked column: Right grows its width, Left
                // shrinks it. Bounds mirror the split-ratio clamp; the column
                // clamp in `verb_dock_target_width` still applies at layout time.
                crate::state::ResizeTarget::VerbDock => match dir {
                    Left => state.pane_sizes.verb_dock_pct = state.pane_sizes.verb_dock_pct.saturating_sub(STEP).max(10),
                    Right => state.pane_sizes.verb_dock_pct = (state.pane_sizes.verb_dock_pct + STEP).min(80),
                    _ => {}
                },
            }
            state.config.split_ratio = state.pane_sizes.split_ratio;
            state.config.verb_dock_pct = state.pane_sizes.verb_dock_pct;
            state.config.inv_dock_pct = state.pane_sizes.inv_dock_pct;
        }

        // ── Config screen actions ─────────────────────────────────────────────

        Action::OpenConfig => {
            state.overlays.hotkey_dialog = false;
            state.overlays.dialog_focus = 0;
            let working = clone_config(&state.config);
            state.overlays.config_screen = Some(crate::state::ConfigScreenState {
                working,
                scroll: Default::default(),
            });
        }

        Action::ConfigNav(delta) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                let n = CONFIG_ROW_COUNT;
                cs.scroll.len(n);
                // Preserve the existing wrap-around behavior via select().
                let next = ((cs.scroll.selected as i32 + delta).rem_euclid(n as i32)) as usize;
                cs.scroll.select(next, vp, &anim);
            }
        }

        Action::ConfigPage(dir) => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.page(dir, vp, &anim);
            }
        }

        Action::ConfigHome => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.home(vp, &anim);
            }
        }

        Action::ConfigEnd => {
            let vp = state.modal_list_viewport;
            let anim = state.config.animation.clone();
            if let Some(cs) = &mut state.overlays.config_screen {
                cs.scroll.len(CONFIG_ROW_COUNT);
                cs.scroll.end(CONFIG_ROW_COUNT, vp, &anim);
            }
        }

        Action::ConfigToggle => {
            if state.overlays.config_screen.is_some() {
                // Split the borrow: take the selected row, then call helper.
                let selected = state.overlays.config_screen.as_ref().map(|cs| cs.scroll.selected).unwrap_or(0);
                config_toggle_or_edit(selected, state);
            }
        }

        Action::ConfigCycle(delta) => {
            if let Some(cs) = &mut state.overlays.config_screen {
                config_cycle(&mut cs.working, cs.scroll.selected, delta);
            }
        }

        Action::ConfigEdit => {
            if let Some(cs) = &state.overlays.config_screen {
                let field = config_path_field(cs.scroll.selected);
                if let Some(f) = field {
                    let current = match &f {
                        crate::state::ConfigPathField::UserDir => cs.working.user_dir.to_string_lossy().to_string(),
                    };
                    state.overlays.dialog_focus = 0;
                    state.overlays.text_entry = Some(TextEntryDialog::new(
                        TextEntryKind::ConfigEditPath { field: f },
                        current,
                    ));
                }
            }
        }

        Action::ConfigSave => {
            if let Some(cs) = state.overlays.config_screen.take() {
                state.config = clone_config(&cs.working);
                // The config screen edits the GLOBAL honor default; keep the
                // SQ-0318 base in sync so a later reload_style doesn't revert it
                // (a per-game override, if any, still wins on the next reload).
                state.honor_game_colours_base = state.config.honor_game_colours;
                if let Some(b) = state.audio.as_mut() {
                    b.set_volume(state.config.volume);
                } else if state.config.enable_sound {
                    state.audio = Some(audio::AudioBackend::new(state.config.volume));
                }
                if !state.config.enable_sound {
                    state.reset_sound_sidecars();
                }
                // Sync the running Glulx VM's Sound gestalt (applied by the event loop).
                state.pending_vm_sound = Some(state.config.enable_sound);
                // Reconcile the style file-watcher live (the run loop owns it).
                state.pending_watch_style = Some(state.config.watch_style);
                // Re-resolve the live look from style.toml (the single styling source).
                let (base, _w1) =
                    crate::style::load_style(cs.working.style.as_deref(), &cs.working.user_dir);
                let (colors, set, _w2) =
                    crate::style::resolve(&base, &cs.working.user_dir);
                state.colors = colors;
                state.symbols = set;
                // The style-file write + config repoint is caller-handled
                // (main.rs snapshots working before this runs).
            }
        }

        Action::ConfigCancel => {
            state.overlays.config_screen = None;
        }

        Action::ResetGame => {
            // Open the reset dialog; the caller (main.rs) handles confirm/cancel/clear-map.
            state.overlays.hotkey_dialog = false;
            state.overlays.reset_dialog = true;
            state.overlays.reset_clear_map = false;
            state.overlays.reset_delete_data = false;
            state.overlays.dialog_focus = 0;
        }

        // Caller-handled: silently ignored.
        Action::SubmitCommand(_)
        | Action::SaveGame
        | Action::RestoreGame
        | Action::ExportSvg(_)
        | Action::ExportDot(_)
        | Action::ExportMap(_)
        | Action::SavesLoad
        | Action::SavesImport
        | Action::FbEnter
        | Action::TranscriptScrollPage(_)
        | Action::PagerAdvance
        | Action::PagerDismiss
        | Action::Quit => {}

        // Caller-handled (needs session scope): opens the hints panel in main.rs.
        Action::OpenHints => {}

        // ── Replay / rewind actions ───────────────────────────────────────────

        Action::OpenHistory => {
            // Seed at the last turn; no-op when there is no history.
            state.overlays.hotkey_dialog = false;
            if !state.history.is_empty() {
                state.overlays.replay = Some(crate::state::ReplayState::new(state.history.len() - 1));
            }
        }

        Action::ReplayStep(delta) => {
            let len = state.history.len();
            if let Some(r) = &mut state.overlays.replay {
                r.step(delta, len);
            }
        }

        Action::ReplayPage(dir) => {
            let len = state.history.len();
            // Page by one list viewport (1-row overlap), clamped by step().
            let page = (state.modal_list_viewport.max(2) - 1) as isize;
            if let Some(r) = &mut state.overlays.replay {
                r.step(dir as isize * page, len);
            }
        }

        Action::ReplayTogglePlay => {
            if let Some(r) = &mut state.overlays.replay {
                r.toggle_play();
            }
        }

        Action::ReplayClose => {
            state.overlays.replay = None;
        }

        // ReplayResume is caller-handled in main.rs (needs the live session/VM).
        Action::ReplayResume => {}

        Action::None => {}
        // Note: OpenHotkeyDialog and CloseHotkeyDialog are handled above.
    }
}

// ── Story-pane selection helpers (SQ-0197) ────────────────────────────────────

/// Map a story-pane screen cell to an absolute wrapped-transcript Point, clamped to
/// the visible rows and the story band. `None` if geometry is degenerate. (SQ-0197)
pub(crate) fn screen_to_point(g: crate::clipboard::TranscriptGeom, col: u16, row: u16)
    -> Option<crate::clipboard::Point> {
    if g.area.width == 0 || g.area.height == 0 { return None; }
    let dy = row.saturating_sub(g.area.y).min(g.area.height.saturating_sub(1));
    let abs = (g.first_abs_row + dy as usize).min(g.total_rows.saturating_sub(1));
    let c = col.saturating_sub(g.area.x).min(g.area.width.saturating_sub(1));
    Some(crate::clipboard::Point { row: abs, col: c })
}

/// While a selection drag sits at an edge, scroll one wrapped row toward it and
/// advance the head in lockstep so the selection keeps growing. No-op at scroll
/// limits or when not selecting. (SQ-0197)
pub fn apply_selection_autoscroll(state: &mut AppState) {
    if state.selection_edge == 0 { return; }
    let Some(g) = state.transcript_geom.get() else { return };
    let max_scroll = g.total_rows.saturating_sub(g.area.height as usize) as u16;
    let cur = state.transcript_scroll;
    let next = if state.selection_edge < 0 { cur.saturating_add(1).min(max_scroll) }
               else { cur.saturating_sub(1) };
    if next == cur { return; } // at a limit
    state.scroll_transcript_to(next);
    if let Some(sel) = &mut state.selection {
        // Top edge reveals an older row above → head.row moves up by 1; bottom edge
        // reveals a newer row below → head.row moves down by 1.
        if state.selection_edge < 0 { sel.head.row = sel.head.row.saturating_sub(1); }
        else { sel.head.row = (sel.head.row + 1).min(g.total_rows.saturating_sub(1)); }
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

/// Return up to `limit` names from `names` containing `body_token` ANYWHERE (case-insensitive),
/// best match first. A name equal to the token is never offered — it is already typed in full.
///
/// Matching was prefix-only (SQ-0353). But command names are compound — `toggle-room-numbers`,
/// `select-room` — and the part anyone actually remembers is the noun, not the verb the name
/// happens to begin with. Typing the one word you could recall found nothing at all: `room` matched
/// no command, though four contain it.
///
/// Ranking is what keeps substring matching from becoming a free-for-all — the obvious answer must
/// still come first:
///   0. the name starts with the token (a plain prefix hit, the old behaviour),
///   1. a name-PART starts with it (right after a `-`) — `room` in `toggle-room-numbers`,
///   2. it appears mid-word — `map` in `unmappable`.
///      Alphabetical within a rank, so the list is stable and predictable as you type.
pub(crate) fn slash_suggestions(body_token: &str, names: &[String], limit: usize) -> Vec<String> {
    if body_token.is_empty() || limit == 0 {
        return Vec::new();
    }
    let lower = body_token.to_lowercase();
    let mut matches: Vec<(u8, String)> = names
        .iter()
        .filter_map(|n| {
            let ln = n.to_lowercase();
            if ln == lower {
                return None; // already typed in full — suggesting it back is noise
            }
            let at = ln.find(&lower)?;
            // `at` is a byte index; step back by CHAR so a non-ASCII token can't split one.
            let rank = match ln[..at].chars().next_back() {
                None => 0,       // prefix
                Some('-') => 1,  // start of a name-part
                Some(_) => 2,    // mid-word
            };
            Some((rank, n.clone()))
        })
        .collect();
    matches.sort_unstable();
    matches.dedup();
    matches.truncate(limit);
    matches.into_iter().map(|(_, n)| n).collect()
}

/// Apply `completion` to the input line, replacing whatever the caret is currently completing:
/// the whole slash-command name, or the partial word at the end. The caret lands after the
/// inserted text.
///
/// Shared by Tab and Shift-Tab, which carried a verbatim copy of this each (SQ-0354).
///
/// Lengths are counted in CHARS, not bytes. The byte arithmetic this replaces would panic outright
/// on a multi-byte partial word: `String::truncate` rejects a non-char boundary, and subtracting a
/// byte length from a byte length lands on one as soon as the word holds anything non-ASCII.
fn apply_completion(state: &mut AppState, completion: &str) {
    let prefix = state.config.command_prefix;
    // Slash-command suggestions hold the bare name (no prefix). When completing the first token of
    // a slash command, rebuild the line as prefix + name so the leading prefix survives.
    let is_slash_name = state.input.value.starts_with(prefix)
        && !state.input.value[prefix.len_utf8()..].contains(' ');
    if is_slash_name {
        state.input.clear();
        state.input.insert(prefix);
    } else {
        let keep = state.input.char_len() - state.current_partial().chars().count();
        state.input.truncate_chars(keep);
        state.input.end();
    }
    state.input.insert_str(completion);
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
    if state.input.value.starts_with(prefix) {
        // Extract the body (everything after the prefix).
        let body = &state.input.value[prefix.len_utf8()..];
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

/// Apply a submitted text-entry dialog. Byte-identical to the retired
/// `apply_prompt`, per kind (SQ-0307):
///   - map-edit kinds mutate the mapper and bump `graph_gen` so the edit shows
///     this frame instead of waiting for the next turn (the Wave-1 choke);
///   - `ConfigEditPath` writes the config-screen working copy;
///   - `CreateFile` flag-hops the chosen filename (empty → cancel) to the run
///     loop's `resolve_filename_request`.
///
/// Empty-value semantics are preserved per kind: RenameRoom empty clears the
/// custom label; EditNotes empty clears the notes; RelabelEdge empty (or an
/// unparseable direction) is a no-op; RenameLayer / ConfigEditPath store the
/// empty string; CreateFile empty cancels the request.
pub fn apply_text_entry(dlg: TextEntryDialog, state: &mut AppState, mapper: &mut Mapper) {
    let value = dlg.field.value;
    match dlg.kind {
        TextEntryKind::RenameRoom(id) => {
            let label = if value.is_empty() { None } else { Some(value) };
            mapper.rename_room(id, label);
            state.bump_graph_gen(); // a graph-mutating edit was applied (SQ-0305)
        }
        TextEntryKind::EditNotes(id) => {
            mapper.set_notes(id, value);
            state.bump_graph_gen();
        }
        TextEntryKind::RelabelEdge(id, old_dir) => {
            // Parse the user's input as a direction name.
            if let Some(new_dir) = mapper::direction::parse_direction(&value) {
                mapper.relabel_edge(id, old_dir, new_dir);
            }
            state.bump_graph_gen();
        }
        TextEntryKind::RenameLayer(id) => {
            mapper.graph.set_layer_name(id, value);
            state.bump_graph_gen();
        }
        TextEntryKind::ConfigEditPath { field } => {
            if let Some(cs) = &mut state.overlays.config_screen {
                match field {
                    crate::state::ConfigPathField::UserDir => {
                        cs.working.user_dir = std::path::PathBuf::from(&value);
                    }
                }
            }
        }
        TextEntryKind::CreateFile => {
            state.filename_submitted =
                Some(if value.trim().is_empty() { None } else { Some(value) });
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

/// Re-center the map on the selected room, or — with nothing selected — on the room the player is
/// currently in (SQ-0349).
///
/// The origin is only a last resort now. It used to be the fallback for "nothing selected", which
/// is an arbitrary corner of the map that need not hold a room at all: the common case (no
/// selection, just wanting the view back on yourself) threw the map somewhere useless. A selection
/// that has no position falls through to the current room for the same reason — it cannot be
/// centred on, but the origin is not a better answer than where the player is standing.
///
/// Centres against `state.map_pane_size` — the map pane as the renderer last measured it, since
/// `apply_action` never sees the run loop's `last_panes`. Falls back to 80×24 only before the first
/// frame, or while the map pane is hidden: `recenter_on` divides the pane by the zoom step, so a
/// guessed size puts the target off-centre on any real pane (SQ-0349).
/// Say why a peel refused, in the player's terms (SQ-0360).
///
/// The refusals are not variations on "no": a layer with no seam in it needs a DIFFERENT command
/// (`peel-layer <dir>`), while a passage that is not a seam means the boundary the player picked
/// is not real. Each message therefore names the room or passage at issue and, where there is a
/// way forward, points at it.
fn peel_refusal_message(
    graph: &mapper::graph::MapGraph,
    room: mapper::graph::RoomId,
    dir: Option<Direction>,
    why: mapper::layer::PeelRefusal,
) -> String {
    use mapper::layer::PeelRefusal as R;
    let here = graph.room(room).map(|r| r.label().to_string()).unwrap_or_else(|| format!("#{room}"));
    let layer = graph.layer_name(graph.layer_of(room));
    match why {
        R::WholeLayer => format!(
            "peel-layer: {layer} is one connected region — nothing to separate. \
             Use peel-layer <direction> to cut at a passage."
        ),
        R::NoSuchPassage => match dir {
            Some(d) => format!("peel-layer: {here} has no {d:?} passage."),
            None => format!("peel-layer: {here} has no passage to cut."),
        },
        R::NotASeam => format!(
            "peel-layer: {here}'s {:?} passage is not a boundary — both sides stay connected \
             another way.",
            dir.unwrap_or(Direction::Unknown)
        ),
        R::LeavesLayer => format!(
            "peel-layer: {here}'s {:?} passage already leaves {layer}. Peeling divides one layer.",
            dir.unwrap_or(Direction::Unknown)
        ),
    }
}

fn apply_recenter(state: &mut AppState, mapper: &Mapper) {
    let target = recenter_target(state, &mapper.graph);
    let (pw, ph) = state.map_pane_size.get().unwrap_or((80, 24));
    state.recenter_on(target, pw, ph);
}

/// The cell the map view should sit on: the selected room, else the room the player is in, else the
/// origin (SQ-0349).
///
/// The run loop's own auto-recentres (after a turn, after a tidy) deliberately aim at the current
/// room instead: the map follows the player as you play, and a selection lasts until your next
/// command. That is intended, not an oversight.
fn recenter_target(state: &AppState, graph: &mapper::graph::MapGraph) -> (i32, i32) {
    let pos_of = |id| graph.room(id).and_then(|r| r.pos);
    state
        .selected_room
        .and_then(pos_of)
        .or_else(|| graph.current().and_then(pos_of))
        .unwrap_or((0, 0))
}

// ── Config screen helpers ─────────────────────────────────────────────────────

/// Number of rows in the config screen — derived from the row list so it cannot drift.
pub(crate) const CONFIG_ROW_COUNT: usize = crate::render::config_screen::CONFIG_ROWS.len();

/// Clone a Config (Config derives Clone, this is a convenience wrapper for tests).
pub(crate) fn clone_config(cfg: &crate::config::Config) -> crate::config::Config {
    cfg.clone()
}

/// Return the ConfigPathField for a row, if the row is a path type.
fn config_path_field(row: usize) -> Option<crate::state::ConfigPathField> {
    match row {
        0 => Some(crate::state::ConfigPathField::UserDir),
        _ => None,
    }
}

/// Apply ConfigToggle to the selected row: toggle bool, advance enum by 1, or open path edit.
fn config_toggle_or_edit(selected: usize, state: &mut AppState) {
    match selected {
        0 => {
            // user_dir — open the text-entry path edit dialog.
            let current = state.overlays.config_screen.as_ref()
                .map(|cs| cs.working.user_dir.to_string_lossy().to_string())
                .unwrap_or_default();
            state.overlays.dialog_focus = 0;
            state.overlays.text_entry = Some(TextEntryDialog::new(
                TextEntryKind::ConfigEditPath { field: crate::state::ConfigPathField::UserDir },
                current,
            ));
        }
        1 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.auto_load = !cs.working.auto_load; } }
        2 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.auto_save = !cs.working.auto_save; } }
        3 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.prompt_save_on_quit = !cs.working.prompt_save_on_quit; } }
        4 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.prompt_load_on_launch = !cs.working.prompt_load_on_launch; } }
        5 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.show_room_numbers = !cs.working.show_room_numbers; } }
        6 => { if let Some(cs) = &mut state.overlays.config_screen { config_cycle_background_tidy(&mut cs.working.background_tidy, 1); } }
        7 => { if let Some(cs) = &mut state.overlays.config_screen { config_cycle_aux_storage(&mut cs.working.aux_storage, 1); } }
        8 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.honor_game_colours = !cs.working.honor_game_colours; } }
        9 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.honor_timed_input = !cs.working.honor_timed_input; } }
        10 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.enable_sound = !cs.working.enable_sound; } }
        12 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.mouse = !cs.working.mouse; } }
        13 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.command_bar = !cs.working.command_bar; } }
        14 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.mouse_wheel_invert = !cs.working.mouse_wheel_invert; } }
        15 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.show_loc_method = !cs.working.show_loc_method; } }
        16 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.show_status_bar = !cs.working.show_status_bar; } }
        17 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.watch_style = !cs.working.watch_style; } }
        18 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.record_turn_history = !cs.working.record_turn_history; } }
        21 => { if let Some(cs) = &mut state.overlays.config_screen { cs.working.hint_skip_screen_warning = !cs.working.hint_skip_screen_warning; } }
        24 => { if let Some(cs) = &mut state.overlays.config_screen { config_cycle_v6_render(&mut cs.working.v6_render, 1); } }
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

fn config_cycle_aux_storage(val: &mut crate::config::AuxStorage, delta: i32) {
    use crate::config::AuxStorage::*;
    let variants = [Ask, Archive, Global];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}

fn config_cycle_v6_render(val: &mut crate::config::V6RenderMode, delta: i32) {
    use crate::config::V6RenderMode::*;
    let variants = [Hybrid, Raster, Frameless];
    let pos = variants.iter().position(|v| v == val).unwrap_or(0) as i32;
    let n = variants.len() as i32;
    *val = variants[((pos + delta).rem_euclid(n)) as usize];
}

/// Apply ConfigCycle to the selected row.
fn config_cycle(working: &mut crate::config::Config, row: usize, delta: i32) {
    match row {
        0 => {} // path: no cycling
        1 => working.auto_load = !working.auto_load,
        2 => working.auto_save = !working.auto_save,
        3 => working.prompt_save_on_quit = !working.prompt_save_on_quit,
        4 => working.prompt_load_on_launch = !working.prompt_load_on_launch,
        5 => working.show_room_numbers = !working.show_room_numbers,
        6 => config_cycle_background_tidy(&mut working.background_tidy, delta),
        7 => config_cycle_aux_storage(&mut working.aux_storage, delta),
        8 => working.honor_game_colours = !working.honor_game_colours,
        9 => working.honor_timed_input = !working.honor_timed_input,
        10 => working.enable_sound = !working.enable_sound,
        11 => working.volume = (working.volume as i32 + delta * 5).clamp(0, 100) as u8,
        12 => working.mouse = !working.mouse,
        13 => working.command_bar = !working.command_bar,
        14 => working.mouse_wheel_invert = !working.mouse_wheel_invert,
        15 => working.show_loc_method = !working.show_loc_method,
        16 => working.show_status_bar = !working.show_status_bar,
        17 => working.watch_style = !working.watch_style,
        18 => working.record_turn_history = !working.record_turn_history,
        19 => working.undo_levels = (working.undo_levels as i32 + delta).clamp(0, 256) as usize,
        // Position 0 == None (babelmap's default); 1..=10 are explicit interpreter numbers.
        // ← from 1 returns to "default"; → from "default" goes to 1.
        20 => {
            let pos = (working.interpreter_number.map(|n| n as i32).unwrap_or(0) + delta).clamp(0, 10);
            working.interpreter_number = if pos == 0 { None } else { Some(pos as u8) };
        }
        21 => working.hint_skip_screen_warning = !working.hint_skip_screen_warning,
        22 => working.text_margin_x = (working.text_margin_x as i32 + delta).clamp(0, 8) as u16,
        23 => working.text_margin_y = (working.text_margin_y as i32 + delta).clamp(0, 8) as u16,
        24 => config_cycle_v6_render(&mut working.v6_render, delta),
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
        // Home is a TEXT key now that the command line has a caret (SQ-0354) — it jumps to the
        // start of the line, matching every other text entry in the app. Recenter keeps its
        // `center-map` command and its hotkey; it just gives up the bare Home key.
        assert!(matches!(key_to_action(&s, key(KeyCode::Home)), Action::CursorHome));
        assert!(matches!(key_to_action(&s, key(KeyCode::End)), Action::CursorEnd));
        // Shift+Arrows still pan, so map nav survives without leaving the story line.
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::CursorLeft));
        // PageUp/PageDown now page the transcript (older/newer), not zoom.
        assert!(matches!(key_to_action(&s, key(KeyCode::PageUp)), Action::TranscriptScrollPage(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::PageDown)), Action::TranscriptScrollPage(-1)));
        // Retidy (Ctrl+T) is not in the direct set: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None));
        // Typing still reaches the command line (plain and shifted/capital letters).
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::InputChar('n')));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::InputChar('N')));
    }

    #[test]
    fn active_more_pager_intercepts_keys() {
        // SQ-0404: while the [more] pager is showing, Space/PgDn/Down/Enter page
        // one screen; anything else dismisses to the bottom. Keys never reach the
        // game.
        let mut s = AppState::default();
        s.pager.active = true;
        for code in [KeyCode::Char(' '), KeyCode::PageDown, KeyCode::Down, KeyCode::Enter] {
            assert!(
                matches!(key_to_command(&s, key(code)), KeyResolve::Action(Action::PagerAdvance)),
                "{code:?} should advance the pager"
            );
        }
        for code in [KeyCode::Char('x'), KeyCode::Esc, KeyCode::Char('q')] {
            assert!(
                matches!(key_to_command(&s, key(code)), KeyResolve::Action(Action::PagerDismiss)),
                "{code:?} should dismiss the pager"
            );
        }
        // Inactive: the pager does not intercept — a plain letter reaches input.
        s.pager.active = false;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('x'))), Action::InputChar('x')));
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

    // The former "prompt-precedence" tests (Ctrl+Q/Tab/Ctrl+S during a bottom-bar
    // prompt) covered the retired `key_to_command` prompt sub-mode. The text-entry
    // modal is now driven by a run-loop intercept (like the save-name dialog), so
    // key_to_command no longer special-cases it; that intercept is exercised via
    // the `text_entry_dialog_key` unit tests in `render::text_entry_dialog`.

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
    fn plain_arrows_pan_and_fn_keys_nudge_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus(); // map focus
        // Plain arrows pan in map focus.
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::Pan(0, 1)));
        // Shift+Arrows no longer bound in map context.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::None));
        // Nudge via plain F6-F9 (direct, via Map->Global fallthrough).
        assert!(matches!(key_to_action(&s, key(KeyCode::F(6))), Action::NudgeSelected(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::F(7))), Action::NudgeSelected(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::F(8))), Action::NudgeSelected(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::F(9))), Action::NudgeSelected(0, 1)));
        // Ctrl+Arrows no longer nudge (not in keymap).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::None));
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
        // Open dialog: Shift+N is not an authored leader letter, so it now closes
        // the dialog instead; the authored leader letter 'n' fires RenameLayer.
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::RenameLayer));
    }

    #[test]
    fn global_shortcuts_work_in_map_focus() {
        let mut s = AppState::default();
        s.toggle_focus();
        // Direct commands fire without the dialog.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('q'))), Action::Quit));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('r'))), KeyResolve::Command(c, _) if c == "restore-state"));
        // Non-direct commands return None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('d'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::None));
        // Ctrl-combos always close the dialog now (never fire); the underlying
        // commands fire via their authored leader letters instead.
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('v'))), Action::ExportSvg(_)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('g'))), Action::ExportDot(_)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('u'))), Action::ExportMap(_)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('l'))), Action::ToggleMap));
    }

    #[test]
    fn leader_letter_fires_command() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        match key_to_command(&s, key(KeyCode::Char('t'))) {
            KeyResolve::Command(c, _) => assert_eq!(c, "tidy-map"),
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn leader_multiword_letter_fires_full_command() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        match key_to_command(&s, key(KeyCode::Char('c'))) {
            KeyResolve::Command(c, _) => assert_eq!(c, "cycle-layer next"),
            other => panic!("expected Command, got {other:?}"),
        }
    }

    #[test]
    fn leader_unbound_letter_closes() {
        // All 26 letters are authored leader letters (SQ-0237 claimed the last
        // two, 'z' and 'k'), so use a digit to exercise the "unbound" path.
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_command(&s, key(KeyCode::Char('1'))), KeyResolve::Action(Action::CloseHotkeyDialog)));
    }

    #[test]
    fn leader_ctrl_combo_closes_not_fires() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Action(Action::CloseHotkeyDialog)));
    }

    #[test]
    fn leader_esc_closes() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_command(&s, key(KeyCode::Esc)), KeyResolve::Action(Action::CloseHotkeyDialog)));
    }

    #[test]
    fn key_resolves_to_command_string() {
        // '+' in Map focus resolves to the zoom-map command string (not an Action).
        let mut s = AppState::default();
        s.focus = Focus::Map;
        match key_to_command(&s, key(KeyCode::Char('+'))) {
            KeyResolve::Command(c, ctx) => {
                assert_eq!(c, "zoom-map in");
                assert_eq!(ctx, crate::keymap::Context::Map);
            }
            other => panic!("expected Command, got {other:?}"),
        }
        // Hardwired Ctrl+Q stays an Action.
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('q'))), KeyResolve::Action(Action::Quit)));
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
    fn history_keys_step_resume_and_close() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let plain = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert!(matches!(history_key_to_action(plain(KeyCode::Left)), Action::ReplayStep(-1)));
        assert!(matches!(history_key_to_action(plain(KeyCode::Right)), Action::ReplayStep(1)));
        assert!(matches!(history_key_to_action(plain(KeyCode::Char(' '))), Action::ReplayTogglePlay));
        assert!(matches!(history_key_to_action(plain(KeyCode::Enter)), Action::ReplayResume));
        assert!(matches!(history_key_to_action(plain(KeyCode::Char('r'))), Action::ReplayResume));
        assert!(matches!(history_key_to_action(plain(KeyCode::Esc)), Action::ReplayClose));
        assert!(matches!(history_key_to_action(plain(KeyCode::Char('q'))), Action::ReplayClose));
    }

    #[test]
    fn replay_step_moves_idx_and_close_clears() {
        use crate::state::{AppState, ReplayState};
        use mapper::mapper::Mapper;
        let mut s = AppState::default();
        // Three records so idx 0..=2 are valid.
        let m = Mapper::default();
        for t in 1..=3 {
            crate::history::record_turn(&mut s.history, t, "x", vec![t as u8], &m, false, "");
        }
        s.overlays.replay = Some(ReplayState::new(2));
        apply_action(Action::ReplayStep(-1), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.replay.as_ref().unwrap().idx, 1);
        apply_action(Action::ReplayClose, &mut s, &mut Mapper::default());
        assert!(s.overlays.replay.is_none(), "Esc closes without change");
        assert_eq!(s.history.len(), 3, "close leaves history intact");
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
    fn toggle_loc_method_flips_state() {
        let mut s = AppState::default();
        assert!(!s.show_loc_method);
        apply_action(Action::ToggleLocMethod, &mut s, &mut mapper::mapper::Mapper::default());
        assert!(s.show_loc_method);
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
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::None));
        // Return actions when dialog is open, via their authored leader letters
        // ('t' for tidy-map/Retidy, 'r' for rename-room); Shift+R is no longer
        // an authored leader letter.
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::Retidy));
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
        // Retidy now builds off-thread (for the progress bar); drive the worker and
        // apply its tidied graph as the run loop would. (SQ-0261)
        drive_retidy(&mut s, &mut m);
        let p1 = m.graph.room(1).unwrap().pos.unwrap();
        let p2 = m.graph.room(2).unwrap().pos.unwrap();
        assert!(p2.0 > p1.0, "after retidy room 2 must be east of room 1: {p2:?} vs {p1:?}");
    }

    /// Test helper: `Retidy` spawns an off-thread build (SQ-0261). Join it and write
    /// the tidied graph back into `m`, mirroring the run loop's apply step, so tests
    /// can assert on the final layout synchronously. A no-op if no job was spawned
    /// (e.g. Manual mode, where Retidy is a no-op).
    fn drive_retidy(s: &mut AppState, m: &mut Mapper) {
        if let Some(job) = s.anim_build_job.take() {
            assert!(!job.animate, "Retidy must build with animate=false");
            let (_frames, tidied) = job.handle.join().expect("retidy worker completes");
            m.graph = tidied;
        }
    }

    // ── Map-memo invalidation: production paths bump graph_gen (SQ-0305) ──────────
    // The map render model is memoized on (graph_gen, viewed_layer). Any graph edit
    // that reaches the live path with an unchanged graph_gen paints a STALE MAP, so
    // each edit path must bump. These drive the real apply_action code, not a manual
    // bump, so a regression that drops a bump fails here.

    #[test]
    fn rename_room_prompt_submit_bumps_graph_gen() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "Old Name", None);
        s.select_room(Some(1));
        let dlg = crate::state::TextEntryDialog::new(
            crate::state::TextEntryKind::RenameRoom(1),
            "New Name",
        );
        let before = s.graph_gen;
        apply_text_entry(dlg, &mut s, &mut m);
        assert_eq!(m.graph.room(1).unwrap().label_override.as_deref(), Some("New Name"),
            "rename actually applied");
        assert_ne!(s.graph_gen, before, "renaming a room must invalidate the map memo");
    }

    #[test]
    fn nudge_selected_bumps_graph_gen() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.set_mode(mapper::layout::LayoutMode::Manual); // nudge is Manual-mode only
        m.graph.set_pos(1, (0, 0));
        s.select_room(Some(1));
        let before = s.graph_gen;
        apply_action(Action::NudgeSelected(1, 0), &mut s, &mut m);
        assert_eq!(m.graph.room(1).unwrap().pos, Some((1, 0)), "room actually moved");
        assert_ne!(s.graph_gen, before, "moving a room must invalidate the map memo + hit-testing");
    }

    #[test]
    fn delete_connection_bumps_graph_gen() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(mapper::direction::Direction::E)); // edge origin=1 --E--> 2
        assert!(m.graph.connections().iter().any(|c| c.origin == 1),
            "fixture must have an outgoing edge from room 1");
        s.select_room(Some(1));
        let before = s.graph_gen;
        apply_action(Action::DeleteSelectedConnection, &mut s, &mut m);
        assert!(!m.graph.connections().iter().any(|c| c.origin == 1), "edge actually deleted");
        assert_ne!(s.graph_gen, before, "deleting a connection must invalidate the map memo");
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
        // The instant re-tidy also builds off-thread now (progress bar); drive it to
        // completion so m_inst holds the tidied graph as the oracle. (SQ-0261)
        drive_retidy(&mut s_inst, &mut m_inst);

        // The frames are now built on a worker thread: AnimateTidy spawns a build job and
        // does NOT touch tidy_anim or the live graph synchronously.
        assert!(s_anim.tidy_anim.is_none(), "no synchronous animation install");
        let job = s_anim.anim_build_job.expect("build job spawned");
        assert!(job.total > 0, "build job carries a positive progress estimate");
        let progress = std::sync::Arc::clone(&job.progress);
        // Drive the async path by joining the worker (the run loop does this + installs).
        let (frames, tidied) = job.handle.join().expect("worker completes");
        assert!(frames.len() >= 2, "at least before + one layout stage frame");
        assert_eq!(
            progress.load(std::sync::atomic::Ordering::Relaxed),
            frames.len(),
            "progress counter is bumped once per emitted frame",
        );
        // The worker's frames and tidied graph match the instant-tidy result room-for-room.
        for id in [1u16, 2, 3] {
            let inst = m_inst.graph.room(id).unwrap().pos;
            assert_eq!(frames.last().unwrap().graph.room(id).unwrap().pos, inst);
            assert_eq!(tidied.room(id).unwrap().pos, inst);
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
        assert!(s.anim_build_job.is_none(), "Manual: no build job spawned");
    }

    #[test]
    fn animate_tidy_is_noop_when_build_or_anim_already_active() {
        use crate::state::{TidyAnim, TidyFrame};
        // A build job already in flight: AnimateTidy does not spawn a second one.
        {
            let mut s = AppState::default();
            let mut m = Mapper::default();
            m.observe(1, "A", None);
            apply_action(Action::AnimateTidy, &mut s, &mut m);
            assert!(s.anim_build_job.is_some(), "first invocation spawns a build job");
            let started = s.anim_build_job.as_ref().unwrap().started;
            apply_action(Action::AnimateTidy, &mut s, &mut m);
            assert_eq!(
                s.anim_build_job.as_ref().unwrap().started,
                started,
                "second invocation is a no-op (same job)"
            );
        }
        // An animation is already playing: AnimateTidy does not spawn a build job.
        {
            let mut s = AppState::default();
            let mut m = Mapper::default();
            m.observe(1, "A", None);
            s.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
                label: "x".into(),
                graph: m.graph.clone(),
                description: String::new(),
                stats: Default::default(),
                stage_start: true,
                manifest: None,
            }], mapper::layer::MAIN_LAYER));
            apply_action(Action::AnimateTidy, &mut s, &mut m);
            assert!(s.anim_build_job.is_none(), "no build job while an animation plays");
        }
    }

    #[test]
    fn anim_submode_routes_transport_keys_and_exits() {
        use crate::state::{TidyAnim, TidyFrame};
        let mut s = AppState::default();
        s.focus = crate::state::Focus::Map;
        // No animation: arrows pan as usual (not stepping).
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::Pan(..)));
        // Animation active: arrows step, Space toggles, Esc exits.
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false, manifest: None };
        s.tidy_anim = Some(TidyAnim::new(vec![frame("a"), frame("b")], mapper::layer::MAIN_LAYER));
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::AnimStep(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::AnimStep(1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(' '))), Action::AnimTogglePlay));
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::AnimExit));
        // The map stays scrollable during playback: hjkl pan (shift-arrows removed), +/- zoom.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::None));
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
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false, manifest: None };
        let mut a = TidyAnim::new(vec![frame("a"), frame("b"), frame("c")], mapper::layer::MAIN_LAYER);
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

        // With dialog open, 'r' → RenameRoom action → the text-entry dialog opens.
        state.overlays.hotkey_dialog = true;
        let a = key_to_action(&state, key(KeyCode::Char('r')));
        assert!(matches!(a, Action::RenameRoom));
        apply_action(a, &mut state, &mut mapper);
        // apply_action clears hotkey_dialog when opening the dialog.
        assert!(!state.overlays.hotkey_dialog, "hotkey_dialog cleared when the dialog opens");
        let dlg = state.overlays.text_entry.as_ref().expect("text-entry dialog opened");
        assert!(matches!(dlg.kind, crate::state::TextEntryKind::RenameRoom(1)));
        assert_eq!(state.overlays.dialog_focus, 0, "focus starts on the field");

        // Type "Lit Room" into the field via the dialog's key routing.
        for c in "Lit Room".chars() {
            let d = state.overlays.text_entry.as_mut().unwrap();
            crate::render::text_entry_dialog::text_entry_dialog_key(KeyCode::Char(c), &mut d.field, 0);
        }
        assert_eq!(state.overlays.text_entry.as_ref().unwrap().field.value, "Lit Room");

        // Submit (what the run loop does on Enter) → mapper updated, dialog cleared.
        let dlg = state.overlays.text_entry.take().unwrap();
        apply_text_entry(dlg, &mut state, &mut mapper);
        assert!(state.overlays.text_entry.is_none());
        assert_eq!(mapper.graph.room(1).unwrap().label(), "Lit Room");
    }

    #[test]
    fn prompt_esc_cancels_without_applying() {
        use crate::render::text_entry_dialog::{text_entry_dialog_key, TextEntryAction};
        let mut mapper = Mapper::default();
        mapper.observe(1, "Original", None);

        let mut state = AppState::default();
        state.toggle_focus();
        state.select_room(Some(1));

        // Open rename dialog, type something.
        apply_action(Action::RenameRoom, &mut state, &mut mapper);
        let d = state.overlays.text_entry.as_mut().unwrap();
        text_entry_dialog_key(KeyCode::Char('X'), &mut d.field, 0);
        assert_eq!(state.overlays.text_entry.as_ref().unwrap().field.value, "X");

        // Esc resolves to Cancel; the run loop drops the dialog without applying.
        let d = state.overlays.text_entry.as_mut().unwrap();
        let (act, _) = text_entry_dialog_key(KeyCode::Esc, &mut d.field, 0);
        assert!(matches!(act, TextEntryAction::Cancel));
        state.overlays.text_entry = None;
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
    fn toggle_sound_lazily_builds_backend_when_turned_on() {
        // Regression: launching with enable_sound = false never constructs an
        // AudioBackend (see main.rs), so flipping the config flag alone leaves
        // state.audio == None forever. ToggleSound must build the backend too.
        audio::disable_output_for_tests(); // ToggleSound builds a real backend; keep it silent
        let mut s = AppState::default();
        s.config.enable_sound = false;
        s.audio = None;
        let mut m = Mapper::default();
        apply_action(Action::ToggleSound, &mut s, &mut m);
        assert!(s.config.enable_sound, "ToggleSound should turn sound on");
        assert!(s.audio.is_some(), "ToggleSound should lazily construct the AudioBackend");
        // The toggle also queues a VM Sound-gestalt sync (drained by the event
        // loop) so a running Glulx game that re-checks gestalt_Sound honors it.
        assert_eq!(s.pending_vm_sound, Some(true), "turning sound on queues a gestalt sync to true");
        apply_action(Action::ToggleSound, &mut s, &mut m);
        assert!(!s.config.enable_sound, "second toggle turns sound off");
        assert_eq!(s.pending_vm_sound, Some(false), "turning sound off queues a gestalt sync to false");
    }

    #[test]
    fn config_save_queues_live_watch_style_reconcile() {
        // Saving the settings screen must hand the new watch_style to the run
        // loop (which owns the file-watcher) so the watcher starts/stops live.
        let mut s = AppState::default();
        let mut working = clone_config(&s.config);
        working.watch_style = true;
        s.overlays.config_screen =
            Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        apply_action(Action::ConfigSave, &mut s, &mut Mapper::default());
        assert_eq!(s.pending_watch_style, Some(true), "ConfigSave should queue a live watch reconcile");
        assert!(s.config.watch_style, "ConfigSave should apply the working config");
    }

    #[test]
    fn bracket_keys_cycle_layer_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // CycleLayer is dialog-only: returns None when dialog is closed.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::None));
        s.overlays.hotkey_dialog = true;
        // Brackets are not authored leader letters: they close the dialog now.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CloseHotkeyDialog));
        // The authored leader letter 'c' fires cycle-layer next instead.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('c'))), Action::CycleLayer(1)));
    }

    #[test]
    fn set_viewed_layer_action_selects_that_layer() {
        // A layer-tab click dispatches Action::SetViewedLayer(id); the handler
        // makes it the viewed layer.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        assert_eq!(s.viewed_layer, None);
        apply_action(Action::SetViewedLayer(2), &mut s, &mut mapper);
        assert_eq!(s.viewed_layer, Some(2), "SetViewedLayer selects the clicked layer");
    }

    #[test]
    fn shift_p_peels_and_shift_m_merges_in_map_focus() {
        let mut s = AppState::default();
        s.focus = Focus::Map;
        // PeelLayer/MergeLayer are dialog-only: return None when dialog is closed.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('P'))), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('M'))), Action::None));
        // Open dialog: authored leader letters 'p'/'m' fire the commands
        // (Shift+P/Shift+M are no longer authored leader letters).
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::PeelLayer(None)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('m'))), Action::MergeLayer));
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
        s.input.set("nor", true);
        // suggestions is empty by default
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));
    }

    #[test]
    fn tab_is_autocomplete_when_suggestions_available() {
        // Game focus, non-empty partial, suggestions populated → Tab is Autocomplete.
        let mut s = AppState::default();
        s.input.set("nor", true);
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
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::Autocomplete, &mut s, &mut m);
        // "nor" should be replaced with "north" (index 0 suggestion).
        assert_eq!(s.input.value, "go north");
        // The highlight stays on the applied candidate (index 0), so the bracket
        // matches the command line; the next Tab advances.
        assert_eq!(s.suggestion_idx, 0);
        assert!(s.suggestion_active);
    }

    #[test]
    fn autocomplete_slash_command_preserves_prefix() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Slash suggestions hold the bare command name (no prefix).
        s.input.set("/sav", true);
        s.suggestions = vec!["save".to_string(), "save-as".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::Autocomplete, &mut s, &mut m);
        // The leading prefix must survive completion.
        assert_eq!(s.input.value, "/save");
        assert_eq!(s.suggestion_idx, 0);
    }

    #[test]
    fn autocomplete_cycles_on_repeated_tab() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        // First Tab: north (applies the highlighted candidate, no advance).
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
        // Second Tab: advances to northeast, replacing the prior completion in
        // place (no need to retype the partial).
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input.value, "go northeast");
        assert_eq!(s.suggestion_idx, 1);
        // Third Tab: wraps back to north.
        apply_action(Action::Autocomplete, &mut s, &mut m);
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
    }

    #[test]
    fn autocomplete_highlight_tracks_command_line() {
        // Regression: the bracketed suggestion must highlight the word that is
        // currently on the command line, not the next one. Cycling forward with
        // repeated Tab keeps the highlight in sync at every step.
        use crate::render::transcript::format_suggestion_line;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string(), "nowhere".to_string()];
        s.suggestion_idx = 0;
        for expected in ["north", "northeast", "nowhere", "north"] {
            apply_action(Action::Autocomplete, &mut s, &mut m);
            assert_eq!(s.input.value, format!("go {expected}"));
            // The bracketed entry on the suggestion line equals the applied word.
            let line = format_suggestion_line(&s.suggestions, s.suggestion_idx);
            assert!(
                line.contains(&format!("[{expected}]")),
                "suggestion line {line:?} must bracket the word now on the command line ({expected})"
            );
        }
    }

    // ── Shift-Tab reverse cycling (feature A) ──────────────────────────────────

    #[test]
    fn shift_tab_is_autocomplete_prev_when_suggestions_available() {
        // Game focus, non-empty partial, suggestions populated → Shift-Tab is
        // AutocompletePrev (the inverse of Tab's Autocomplete).
        let mut s = AppState::default();
        s.input.set("nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::AutocompletePrev));
    }

    #[test]
    fn shift_tab_cycles_focus_back_without_suggestions() {
        // Game focus, partial typed but no suggestions → no AutocompletePrev, so
        // Shift-Tab reverses the per-window focus cycle.
        let mut s = AppState::default();
        s.input.set("nor", true);
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::CycleFocusBack));
    }

    #[test]
    fn shift_tab_cycles_focus_back_with_empty_input() {
        let s = AppState::default(); // Game focus, empty input, no suggestions
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::CycleFocusBack));
    }

    #[test]
    fn shift_tab_cycles_focus_back_in_map_focus() {
        // Not in Game focus, so the autocomplete intercept does not apply; Shift-Tab
        // reverses the window focus cycle.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.suggestions = vec!["north".to_string()];
        assert!(matches!(key_to_action(&s, key(KeyCode::BackTab)), Action::CycleFocusBack));
    }

    #[test]
    fn autocomplete_prev_action_replaces_partial_and_steps_back() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        // Applies the current (index 0) suggestion, like Autocomplete; the
        // highlight stays put so the bracket matches the command line.
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
        assert!(s.suggestion_active);
    }

    #[test]
    fn autocomplete_prev_cycles_backward_with_wrap() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("go nor", true);
        s.suggestions = vec!["north".to_string(), "northeast".to_string(), "nowhere".to_string()];
        s.suggestion_idx = 0;
        // First Shift-Tab: applies the highlighted index 0 (north), no step.
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        assert_eq!(s.input.value, "go north");
        assert_eq!(s.suggestion_idx, 0);
        // Second Shift-Tab: steps backward to index 2 (nowhere), replacing in place.
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        assert_eq!(s.input.value, "go nowhere");
        assert_eq!(s.suggestion_idx, 2);
    }

    #[test]
    fn autocomplete_prev_slash_command_preserves_prefix() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.input.set("/sav", true);
        s.suggestions = vec!["save".to_string(), "save-as".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::AutocompletePrev, &mut s, &mut m);
        assert_eq!(s.input.value, "/save");
        assert_eq!(s.suggestion_idx, 0);
    }

    #[test]
    fn typing_resets_suggestion_index() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Pre-load some suggestions and set idx > 0.
        s.input.set("no", true);
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
        s.overlays.hotkey_dialog = true;
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
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('r'))), KeyResolve::Command(c, _) if c == "restore-state"));
        // Non-direct ctrl commands return None when dialog is closed.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('e'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('g'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('d'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('l'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('y'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('a'))), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('p'))), Action::None));
        // Ctrl+Arrows no longer nudge (nudge moved to plain F6-F9).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Right)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Up)), Action::None));
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Down)), Action::None));
        // F6-F9 nudge via Global fallthrough from game focus.
        assert!(matches!(key_to_action(&s, key(KeyCode::F(6))), Action::NudgeSelected(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::F(7))), Action::NudgeSelected(1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::F(8))), Action::NudgeSelected(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::F(9))), Action::NudgeSelected(0, 1)));
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
        // Shift+Arrows no longer bound in map context.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::None));
        // hjkl pan (direct)
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('k'))), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('l'))), Action::Pan(1, 0)));
        // Zoom (direct); shift(+) alias removed, plain alternatives remain.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('='))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('+'))), Action::None));
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
        // Direct ctrl globals work in map focus (save/restore kept with ctrl).
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-state"));
        // Ctrl+Left no longer nudges (nudge moved to F6-F9).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));
        // F6-F9 nudge work in map focus via Global fallthrough.
        assert!(matches!(key_to_action(&s, key(KeyCode::F(6))), Action::NudgeSelected(-1, 0)));
        // Tab → ToggleFocus in map focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Tab)), Action::ToggleFocus));

        // ── Map focus (dialog open) ───────────────────────────────────────────
        s.overlays.hotkey_dialog = true;
        // Dialog-only commands now fire via their authored leader letters.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('n'))), Action::RenameLayer));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('p'))), Action::PeelLayer(None)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('m'))), Action::MergeLayer));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::Retidy));
        // Unauthored keys (shift-modified letters, brackets) close the dialog
        // instead of firing.
        assert!(matches!(key_to_action(&s, shift(KeyCode::Char('N'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(']'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('['))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('r'))), Action::RenameRoom));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('o'))), Action::EditNotes));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('d'))), Action::DeleteSelectedConnection));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('e'))), Action::RelabelSelectedEdge));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInspector));
        // 'q' is authored to toggle-portal-labels; it no longer closes the dialog.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('q'))), Action::TogglePortalLabels));
        s.overlays.hotkey_dialog = false;

        // ── Anim sub-mode ─────────────────────────────────────────────────────
        let mut s = AppState::default();
        s.focus = Focus::Map;
        let frame = |l: &str| TidyFrame { label: l.into(), graph: mapper::graph::MapGraph::new(), description: String::new(), stats: mapper::layout::TidyStats::default(), stage_start: false, manifest: None };
        s.tidy_anim = Some(TidyAnim::new(vec![frame("a"), frame("b")], mapper::layer::MAIN_LAYER));
        // Step
        assert!(matches!(key_to_action(&s, key(KeyCode::Left)), Action::AnimStep(-1)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Right)), Action::AnimStep(1)));
        // Play/pause
        assert!(matches!(key_to_action(&s, key(KeyCode::Char(' '))), Action::AnimTogglePlay));
        // Exit
        assert!(matches!(key_to_action(&s, key(KeyCode::Esc)), Action::AnimExit));
        assert!(matches!(key_to_action(&s, key(KeyCode::Enter)), Action::AnimExit));
        // Pan in anim: hjkl only (shift-arrows removed from Anim keymap).
        assert!(matches!(key_to_action(&s, shift(KeyCode::Left)), Action::None));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Right)), Action::None));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('h'))), Action::Pan(-1, 0)));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('j'))), Action::Pan(0, 1)));
        // Zoom in anim
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('+'))), Action::ZoomIn));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('-'))), Action::ZoomOut));
        // Anim does NOT fall through to Global: unknown key → None
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('s'))), Action::None));
    }

    // ── Saves-manager sub-mode tests ──────────────────────────────────────────

    fn state_with_saves_open() -> AppState {
        use crate::state::{SavesState};
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState {
            entries: vec![
                SaveInfo {
                    path: PathBuf::from("/tmp/default.babelmap"),
                    name: "(default)".to_string(),
                    turns: 0,
                    saved_at: String::new(),
                    location: None, score: None, is_default: true,
                },
                SaveInfo {
                    path: PathBuf::from("/tmp/named.babelmap"),
                    name: "before-troll".to_string(),
                    turns: 10,
                    saved_at: "2026-06-18T10:00:00Z".to_string(),
                    location: None, score: None, is_default: false,
                },
            ],
            scroll: Default::default(),
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
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 1);
        // Up moves back to 0.
        let a = key_to_action(&s, key(KeyCode::Up));
        assert!(matches!(a, Action::SavesNav(-1)));
        apply_action(a, &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 0);
    }

    #[test]
    fn saves_submode_s_opens_save_name_dialog() {
        let mut s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Char('s')));
        assert!(matches!(a, Action::SavesSaveAs));
        apply_action(a, &mut s, &mut Mapper::default());
        // SavesSaveAs now opens the common-dialog save-name modal (not a bottom-bar
        // prompt), prefilled with a greyed date-time default, focused on the field.
        assert!(s.overlays.text_entry.is_none(), "no text-entry dialog is opened for save-as");
        let dlg = s.overlays.save_name_dialog.as_ref().expect("save-name dialog opened");
        assert!(!dlg.active, "opens greyed (placeholder) until edited");
        assert!(!dlg.ingame, "host Save State context");
        assert!(!dlg.field.value.is_empty(), "prefilled with a default name");
        assert_eq!(s.overlays.dialog_focus, 0, "focus starts on the text field");
    }

    #[test]
    fn saves_submode_d_opens_confirm_delete_dialog() {
        let mut s = state_with_saves_open();
        // Select entry 1 (the named save).
        s.overlays.saves.as_mut().unwrap().scroll.selected = 1;
        let a = key_to_action(&s, key(KeyCode::Char('d')));
        assert!(matches!(a, Action::SavesDelete));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.confirm_delete_save.is_some(), "SavesDelete opens the confirm-delete dialog");
        assert_eq!(s.overlays.dialog_focus, 1, "focus starts on Cancel (the safe default)");
    }

    #[test]
    fn saves_submode_esc_closes_modal() {
        let mut s = state_with_saves_open();
        let a = key_to_action(&s, key(KeyCode::Esc));
        assert!(matches!(a, Action::SavesClose));
        apply_action(a, &mut s, &mut Mapper::default());
        assert!(s.overlays.saves.is_none(), "Esc should close the saves modal");
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
        s.overlays.hotkey_dialog = true;
        // Ctrl-combos always close the dialog now (never fire); it fires via
        // the authored leader letter 's' instead.
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Char('o'))), Action::CloseHotkeyDialog));
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('s'))), Action::OpenSaves));
    }

    #[test]
    fn saves_nav_wraps_around() {
        use crate::state::SavesState;
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState {
            entries: vec![
                SaveInfo { path: PathBuf::from("/tmp/a.babelmap"), name: "a".into(), turns: 0, saved_at: String::new(), location: None, score: None, is_default: false },
                SaveInfo { path: PathBuf::from("/tmp/b.babelmap"), name: "b".into(), turns: 0, saved_at: String::new(), location: None, score: None, is_default: false },
            ],
            scroll: Default::default(),
        });
        s.overlays.saves.as_mut().unwrap().scroll.selected = 1;
        // Down from last wraps to first.
        apply_action(Action::SavesNav(1), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 0, "should wrap to 0 after last");
        // Up from first wraps to last.
        apply_action(Action::SavesNav(-1), &mut s, &mut Mapper::default());
        assert_eq!(s.overlays.saves.as_ref().unwrap().scroll.selected, 1, "should wrap to last");
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
        s.overlays.hotkey_dialog = true;
        assert!(matches!(
            key_to_action(&s, ctrl(KeyCode::Char('k'))),
            Action::CloseHotkeyDialog
        ));
    }

    #[test]
    fn q_no_longer_closes_hotkey_dialog() {
        // q-close removed from hotkey dialog; q now falls through to keymap lookup.
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
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
        // When dialog is open, the authored leader letter 't' in map focus fires Retidy.
        let mut s = AppState::default();
        s.focus = Focus::Map;
        s.overlays.hotkey_dialog = true;
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::Retidy));
        // ToggleInspector fires too.
        assert!(matches!(key_to_action(&s, key(KeyCode::Char('i'))), Action::ToggleInspector));
    }

    #[test]
    fn apply_open_hotkey_dialog_sets_flag() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.overlays.hotkey_dialog);
        apply_action(Action::OpenHotkeyDialog, &mut s, &mut m);
        assert!(s.overlays.hotkey_dialog);
        apply_action(Action::CloseHotkeyDialog, &mut s, &mut m);
        assert!(!s.overlays.hotkey_dialog);
    }

    #[test]
    fn open_saves_clears_hotkey_dialog() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.overlays.hotkey_dialog = true;
        apply_action(Action::OpenSaves, &mut s, &mut m);
        assert!(!s.overlays.hotkey_dialog, "OpenSaves should clear the hotkey dialog");
    }

    // ── is_direct as sole direct-vs-prefix determiner ─────────────────────────

    /// Promoting a command via config makes it reachable directly (dialog closed).
    #[test]
    fn direct_config_promotes_retidy_to_direct() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: Some(vec!["tidy-map".into()]),
            group: vec![HotkeyGroupConfig {
                title: "Layout".into(),
                commands: vec!["tidy-map".into()],
            }],
        };
        let (layout, _) = crate::keymap::HotkeyLayout::resolve(&cfg);
        let mut s = AppState::default();
        s.hotkeys = layout;
        s.focus = Focus::Map;
        // tidy-map has no default keymap binding now (leader-only command), so a
        // user promoting it to direct via config would also bind a key for it.
        s.keymap.bindings.push((
            crate::keymap::KeySpec { code: KeyCode::Char('t'), ctrl: true, shift: false, alt: false },
            "tidy-map".to_string(),
            crate::keymap::Context::Global,
        ));
        // With dialog closed: tidy-map is now direct → fires.
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
        // Open dialog: Ctrl+T now closes the dialog (Ctrl-combos never fire);
        // the authored leader letter 't' fires Retidy instead.
        s.overlays.hotkey_dialog = true;
        assert!(
            matches!(key_to_action(&s, ctrl(KeyCode::Char('t'))), Action::CloseHotkeyDialog),
            "Ctrl-combos close the hotkey dialog rather than firing"
        );
        assert!(
            matches!(key_to_action(&s, key(KeyCode::Char('t'))), Action::Retidy),
            "retidy should fire from the hotkey dialog via leader letter 't'"
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
    fn mouse_wheel_invert_swaps_story_scroll_direction() {
        use crossterm::event::MouseEventKind;

        let mut s = AppState::default();
        // Default (conventional): wheel up scrolls up into older text (+1).
        let m = mouse_event(MouseEventKind::ScrollUp, 90, 10, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None),
            Action::TranscriptScroll(1)
        ));
        // Inverted: wheel up scrolls the other way.
        s.config.mouse_wheel_invert = true;
        let m2 = mouse_event(MouseEventKind::ScrollUp, 90, 10, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, m2, map_rect(), story_rect(), &[], &None),
            Action::TranscriptScroll(-1)
        ));
    }

    // ── Mouse-wheel modal precedence tests ─────────────────────────────────────
    //
    // When a scrollable overlay is open, the wheel must drive THAT surface's
    // vertical nav (one item per tick) ahead of the underlying map/story, reusing
    // each modal's existing Up/Down action. A wheel position over the MAP area is
    // used so the same events also exercise precedence over map pan/zoom.

    fn wheel_up() -> crossterm::event::MouseEvent {
        // Position (10, 10) is inside map_rect (0,0,80,40).
        mouse_event(crossterm::event::MouseEventKind::ScrollUp, 10, 10, KeyModifiers::NONE)
    }
    fn wheel_down() -> crossterm::event::MouseEvent {
        mouse_event(crossterm::event::MouseEventKind::ScrollDown, 10, 10, KeyModifiers::NONE)
    }

    #[test]
    fn wheel_drives_saves_selection() {
        use crate::state::SavesState;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState { entries: Vec::new(), scroll: Default::default() });
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::SavesNav(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::SavesNav(1)
        ));
    }

    #[test]
    fn wheel_drives_replay_step() {
        use crate::state::ReplayState;
        let mut s = AppState::default();
        s.overlays.replay = Some(ReplayState::new(0));
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::ReplayStep(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::ReplayStep(1)
        ));
    }

    #[test]
    fn wheel_drives_file_browser_selection() {
        use crate::state::{FbMode, FileBrowserState};
        let mut s = AppState::default();
        s.overlays.file_browser = Some(FileBrowserState::build(
            std::env::temp_dir(),
            FbMode::PickFile));
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::FbNav(-1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::FbNav(1)
        ));
    }

    #[test]
    fn wheel_drives_verb_menu_selection() {
        use crate::state::{VerbMenuPane, VerbMenuState};
        let mut s = AppState::default();
        s.overlays.verb_menu = Some(VerbMenuState {
            pane: VerbMenuPane::Verbs,
            verb_scroll: Default::default(),
            noun_scroll: Default::default(),
            prep_scroll: Default::default(),
            nouns: Vec::new(),
            story_focused: false,
        });
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::VerbMenuNav(VerbMenuNavKind::Up)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::VerbMenuNav(VerbMenuNavKind::Down)
        ));
    }

    #[test]
    fn wheel_modal_precedence_beats_open_dialog_chrome() {
        // Saves open WITH a dialog present: the wheel must still drive the saves
        // list (not be swallowed by the dialog chrome block).
        use crate::state::SavesState;
        use crate::render::dialog::DialogRects;
        use ratatui::layout::Rect;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState { entries: Vec::new(), scroll: Default::default() });
        let dialog = Some(DialogRects {
            area: Rect::new(10, 5, 40, 15),
            content: Rect::new(11, 7, 38, 10),
            close: Some(Rect::new(48, 5, 1, 1)),
            buttons: Vec::new(),
            field: None,
        });
        // Wheel over the map area, with the dialog open.
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &dialog),
            Action::SavesNav(-1)
        ));
    }

    #[test]
    fn wheel_invert_swaps_modal_nav_direction() {
        // Representative modal (saves): with mouse_wheel_invert set, ScrollUp maps
        // to the DOWN action and ScrollDown to the UP action.
        use crate::state::SavesState;
        let mut s = AppState::default();
        s.overlays.saves = Some(SavesState { entries: Vec::new(), scroll: Default::default() });
        s.config.mouse_wheel_invert = true;
        assert!(matches!(
            mouse_to_action(&s, wheel_up(), map_rect(), story_rect(), &[], &None),
            Action::SavesNav(1)
        ));
        assert!(matches!(
            mouse_to_action(&s, wheel_down(), map_rect(), story_rect(), &[], &None),
            Action::SavesNav(-1)
        ));
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

    // ── Command history Up/Down (feature D) ────────────────────────────────────

    #[test]
    fn plain_up_down_recall_history_in_game_focus() {
        let s = AppState::default(); // Game focus
        assert!(matches!(key_to_action(&s, key(KeyCode::Up)), Action::HistoryPrev));
        assert!(matches!(key_to_action(&s, key(KeyCode::Down)), Action::HistoryNext));
    }

    #[test]
    fn shift_up_down_still_pan_in_game_focus() {
        let s = AppState::default(); // Game focus
        assert!(matches!(key_to_action(&s, shift(KeyCode::Up)), Action::Pan(0, -1)));
        assert!(matches!(key_to_action(&s, shift(KeyCode::Down)), Action::Pan(0, 1)));
    }

    #[test]
    fn history_actions_apply_through_apply_action() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        s.command_history = vec!["look".into(), "inventory".into()];
        s.input = "dr".into();
        apply_action(Action::HistoryPrev, &mut s, &mut m);
        assert_eq!(s.input.value, "inventory");
        apply_action(Action::HistoryNext, &mut s, &mut m);
        assert_eq!(s.input.value, "dr"); // draft restored
    }

    // ── PageUp/PageDown transcript paging (feature C) ──────────────────────────

    #[test]
    fn page_scroll_toward_older_advances_by_page_clamped() {
        // viewport 20 rows → page = 19. From 0 toward older (dir > 0): 0 + 19 = 19.
        assert_eq!(page_scroll(0, 1, 20, 100), 19);
        // Next page: 19 + 19 = 38.
        assert_eq!(page_scroll(19, 1, 20, 100), 38);
        // Clamped to max_scroll.
        assert_eq!(page_scroll(90, 1, 20, 100), 100);
        assert_eq!(page_scroll(100, 1, 20, 100), 100);
    }

    #[test]
    fn page_scroll_toward_newer_recedes_by_page_to_zero() {
        // dir < 0 moves toward newer (smaller offset), saturating at 0.
        assert_eq!(page_scroll(38, -1, 20, 100), 19);
        assert_eq!(page_scroll(19, -1, 20, 100), 0);
        assert_eq!(page_scroll(5, -1, 20, 100), 0);
    }

    #[test]
    fn page_scroll_tiny_viewport_steps_at_least_one() {
        // viewport of 0 or 1 → page floors at 1 line so paging still progresses.
        assert_eq!(page_scroll(0, 1, 1, 100), 1);
        assert_eq!(page_scroll(0, 1, 0, 100), 1);
    }

    #[test]
    fn wheel_delta_maps_and_inverts_once() {
        use crossterm::event::MouseEventKind::*;
        assert_eq!(wheel_delta(ScrollUp, false), Some(-1));
        assert_eq!(wheel_delta(ScrollDown, false), Some(1));
        assert_eq!(wheel_delta(ScrollUp, true), Some(1));
        assert_eq!(wheel_delta(ScrollDown, true), Some(-1));
        assert_eq!(wheel_delta(Moved, false), None);
    }

    #[test]
    fn page_up_down_do_not_zoom() {
        // Regression guard: PageUp/PageDown must not produce zoom actions.
        let s = AppState::default();
        let up = key_to_action(&s, key(KeyCode::PageUp));
        let dn = key_to_action(&s, key(KeyCode::PageDown));
        assert!(!matches!(up, Action::ZoomIn | Action::ZoomOut));
        assert!(!matches!(dn, Action::ZoomIn | Action::ZoomOut));
        assert!(matches!(up, Action::TranscriptScrollPage(1)));
        assert!(matches!(dn, Action::TranscriptScrollPage(-1)));
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
    fn left_down_in_story_starts_selection_and_activates_game() {
        use crossterm::event::MouseEventKind;
        let mut s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m = mouse_event(MouseEventKind::Down(MouseButton::Left), 85, 5, KeyModifiers::NONE);
        let action = mouse_to_action(&s, m, map_rect(), story_rect(), &[], &None);
        assert!(
            matches!(action, Action::StartSelection(85, 5)),
            "left-down in story pane should start a selection, got {:?}", action
        );
        // Applying it activates the game pane and sets the selection anchor.
        // Publish geometry so screen cells map to absolute wrapped-row Points.
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: story_rect(), first_abs_row: 0, total_rows: 100,
        }));
        apply_action(action, &mut s, &mut Mapper::default());
        assert_eq!(s.focus, Focus::Game);
        // col 85 → col 5 within the story band (x=80); row 5 → abs row 5.
        assert_eq!(s.selection.map(|sel| sel.anchor), Some(crate::clipboard::Point { row: 5, col: 5 }));
    }

    #[test]
    fn left_drag_then_up_extends_and_ends_selection() {
        use crossterm::event::MouseEventKind;
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: story_rect(), first_abs_row: 0, total_rows: 100,
        }));
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());

        let drag = mouse_event(MouseEventKind::Drag(MouseButton::Left), 90, 7, KeyModifiers::NONE);
        let a = mouse_to_action(&s, drag, map_rect(), story_rect(), &[], &None);
        assert!(matches!(a, Action::ExtendSelection(90, 7)));
        apply_action(a, &mut s, &mut Mapper::default());
        // col 90 → col 10; row 7 (interior) → abs row 7.
        assert_eq!(s.selection.map(|sel| sel.head), Some(crate::clipboard::Point { row: 7, col: 10 }));

        let up = mouse_event(MouseEventKind::Up(MouseButton::Left), 90, 7, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, up, map_rect(), story_rect(), &[], &None),
            Action::EndSelection
        ));
    }

    #[test]
    fn screen_to_point_maps_row_and_col() {
        let g = crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(0, 0, 20, 10), first_abs_row: 5, total_rows: 100,
        };
        assert_eq!(screen_to_point(g, 3, 2), Some(crate::clipboard::Point { row: 7, col: 3 }));
        // row past the bottom clamps to first_abs_row + height - 1.
        assert_eq!(screen_to_point(g, 3, 99), Some(crate::clipboard::Point { row: 14, col: 3 }));
        // col past the right clamps to width - 1.
        assert_eq!(screen_to_point(g, 99, 2), Some(crate::clipboard::Point { row: 7, col: 19 }));
    }

    #[test]
    fn screen_to_point_clamps_to_total_rows() {
        // total_rows smaller than first_abs_row + dy: clamp to total_rows - 1.
        let g = crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(0, 0, 20, 10), first_abs_row: 5, total_rows: 8,
        };
        assert_eq!(screen_to_point(g, 0, 9), Some(crate::clipboard::Point { row: 7, col: 0 }));
    }

    #[test]
    fn start_selection_sets_anchor_from_geom() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 40), first_abs_row: 10, total_rows: 100,
        }));
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        let sel = s.selection.expect("selection set");
        let expected = crate::clipboard::Point { row: 15, col: 5 };
        assert_eq!(sel.anchor, expected);
        assert_eq!(sel.head, expected);
    }

    #[test]
    fn extend_selection_at_bottom_edge_autoscrolls_and_grows_head() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10; // mid-range (max = 100 - 20 = 80)
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        // Bottom edge row: area.bottom() - 1 = 19 → maps to abs row 30 + 19 = 49;
        // autoscroll then grows the head one more row → 50.
        apply_action(Action::ExtendSelection(90, 19), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, 1);
        assert_eq!(s.transcript_scroll, 9, "bottom edge scrolls toward newer (scroll -1)");
        assert_eq!(s.selection.unwrap().head.row, 50, "head grows downward past the edge row");
    }

    #[test]
    fn extend_selection_at_top_edge_autoscrolls_and_grows_head() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10;
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        // Top edge row: area.y = 0 → maps to abs row 30; autoscroll grows the head
        // one row upward → 29.
        apply_action(Action::ExtendSelection(90, 0), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, -1);
        assert_eq!(s.transcript_scroll, 11, "top edge scrolls toward older (scroll +1)");
        assert_eq!(s.selection.unwrap().head.row, 29, "head grows upward past the edge row");
    }

    #[test]
    fn extend_selection_interior_sets_edge_zero_no_scroll() {
        let mut s = AppState::default();
        s.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
            area: ratatui::layout::Rect::new(80, 0, 40, 20), first_abs_row: 30, total_rows: 100,
        }));
        s.transcript_scroll = 10;
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());
        apply_action(Action::ExtendSelection(90, 10), &mut s, &mut Mapper::default());
        assert_eq!(s.selection_edge, 0);
        assert_eq!(s.transcript_scroll, 10, "interior drag does not scroll");
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
        // The wheel keeps the FINE step (SQ-0350): three notches to a visible change, so a fast
        // ctrl+scroll cannot skip past Compact. The keyboard's `+`/`-` move a whole step instead.
        assert!(matches!(action, Action::ZoomInFine), "scroll up + Ctrl -> ZoomInFine");
    }

    #[test]
    fn scroll_in_story_produces_transcript_scroll() {
        use crossterm::event::MouseEventKind;
        let s = AppState::default();
        // col 85 is inside story_rect (x=80..120).
        let m_up = mouse_event(MouseEventKind::ScrollUp, 85, 5, KeyModifiers::NONE);
        let action_up = mouse_to_action(&s, m_up, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action_up, Action::TranscriptScroll(1)), "scroll up in story -> TranscriptScroll(1) (older)");

        let m_dn = mouse_event(MouseEventKind::ScrollDown, 85, 5, KeyModifiers::NONE);
        let action_dn = mouse_to_action(&s, m_dn, map_rect(), story_rect(), &[], &None);
        assert!(matches!(action_dn, Action::TranscriptScroll(-1)), "scroll down in story -> TranscriptScroll(-1) (newer)");
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
        // Grab-and-drag: drag 11 columns right → char_pan.0 = +11, scroll unchanged.
        apply_action(Action::DragPanTo(21, 10), &mut s, &mut m); // dx=11
        assert_eq!(s.char_pan.0, 11, "11-col drag right should set char_pan.0 to +11");
        assert_eq!(s.scroll, (0, 0), "scroll should not change during drag");

        // Drag 1 more column right: char_pan.0 = +12, scroll still unchanged.
        apply_action(Action::DragPanTo(22, 10), &mut s, &mut m); // dx=1
        assert_eq!(s.char_pan.0, 12, "additional 1-col drag should set char_pan.0 to +12");
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
        assert_eq!(s.char_pan.0, 5, "char_pan.0 should be +5 after 5-col drag right (grab)");
    }

    #[test]
    fn drag_pan_grab_and_drag_direction() {
        // Grab-and-drag: dragging LEFT moves content left → char_pan.0 negative.
        use crate::state::Zoom;

        let mut s = AppState::default();
        s.zoom = Zoom::Compact;
        let mut m = Mapper::default();

        apply_action(Action::BeginDragPan(20, 0), &mut s, &mut m);
        // Drag left by 12 columns: dx = -12, char_pan.0 += dx = -12.
        apply_action(Action::DragPanTo(8, 0), &mut s, &mut m);
        assert_eq!(s.char_pan.0, -12, "dragging left moves content left (grab): char_pan.0 = -12");
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

    // ── Leaf 1: ToggleMap ─────────────────────────────────────────────────────

    #[test]
    fn apply_action_toggle_map() {
        use crate::state::{AppState, Layout};
        let mut s = AppState::default(); // empty game_dir → no sidecar write
        let mut m = Mapper::default();
        assert!(matches!(s.layout, Layout::Split));
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::TranscriptFull));
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::Split));
    }

    #[test]
    fn toggle_map_persists_show_map_to_game_dir() {
        use crate::state::{AppState, Layout};
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let game_dir = std::env::temp_dir()
            .join(format!("bm-togglemap-{}-{}.save", std::process::id(), n));
        std::fs::create_dir_all(&game_dir).unwrap();

        let mut s = AppState::default();
        s.game_dir = game_dir.clone();
        let mut m = Mapper::default();

        // Hide the map → show_map = false persisted.
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::TranscriptFull));
        assert_eq!(crate::styles::read_per_game_show_map(&game_dir), Some(false));

        // Show it again → show_map = true persisted.
        apply_action(Action::ToggleMap, &mut s, &mut m);
        assert!(matches!(s.layout, Layout::Split));
        assert_eq!(crate::styles::read_per_game_show_map(&game_dir), Some(true));
        let _ = std::fs::remove_dir_all(&game_dir);
    }

    // ── Leaf 2: ResetGame opens the dialog ───────────────────────────────────

    #[test]
    fn reset_game_action_opens_reset_dialog() {
        use crate::state::AppState;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.overlays.reset_dialog, "dialog must start closed");
        apply_action(Action::ResetGame, &mut s, &mut m);
        assert!(s.overlays.reset_dialog, "ResetGame must set reset_dialog = true");
        assert!(!s.overlays.reset_clear_map, "checkbox must start unchecked");
        assert!(s.overlays.text_entry.is_none(), "no text-entry dialog should be opened");
    }

    // ── reset-game is now leader-only; F5 has no default binding ──────────────
    // reset-game was demoted out of the always-active default keymap (SQ-0202):
    // it's reached only through the Ctrl+K leader panel now. This test pins that
    // F5 no longer resolves directly, and (still) that Action::ResetGame — however
    // it's triggered — opens the confirmation dialog rather than instant-wiping.
    #[test]
    fn f5_key_no_longer_bound_and_reset_game_still_opens_dialog() {
        use crate::state::AppState;
        let s = AppState::default();
        // (a) F5 has no default binding (reset-game is leader-only now).
        assert!(
            matches!(key_to_command(&s, key(KeyCode::F(5))), KeyResolve::None),
            "F5 must not resolve to a command by default"
        );
        // (b) The from_key Reset branch opens the dialog via Action::ResetGame.
        let mut s2 = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ResetGame, &mut s2, &mut m);
        assert!(s2.overlays.reset_dialog, "reset-game must open the confirmation dialog, not instant-wipe");
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
        let mut session = GameSession::new(story_bytes.clone(), true, false, None).expect("GameSession::new");
        let mut mapper = Mapper::default();
        let mut state = crate::state::AppState::default();

        let start_loc = current_location(&session.machine);
        let start_room_number = start_loc.as_ref().map(|s| s.number);
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
                pictures: Vec::new(),
                transcript_elems: Vec::new(),
            };
            apply_turn(&mut mapper, "", &seed_result);
            state.select_room(Some(snap_number as mapper::graph::RoomId));
        }
        let banner = session.take_transcript();
        state.push_transcript(&banner);

        // Simulate some game turns to advance state.
        let r1 = session.submit("look");
        state.push_transcript_runs(&r1.transcript, crate::state::TranscriptKind::Story, &r1.transcript_runs);
        state.turns = 5;

        // Rebuild session from story_bytes (what the reset flow does on confirm).
        let mut new_session = GameSession::new(story_bytes.clone(), true, false, None).expect("GameSession::new for reset");
        let new_start_loc = current_location(&new_session.machine);
        let new_room_number = new_start_loc.as_ref().map(|s| s.number);

        // Reset state fields exactly as the reset flow does.
        state.turns = 0;
        state.input.clear();
        state.suggestions.clear();
        state.suggestion_idx = 0;
        state.transcript.clear();
        state.clear_anchor = None;
        state.transcript_kinds.clear();
        state.transcript_runs.clear();
        state.transcript_scroll = 0;
        let new_banner = new_session.take_transcript();
        state.push_transcript(&new_banner);
        if let Some(snap) = new_start_loc {
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
                pictures: Vec::new(),
                transcript_elems: Vec::new(),
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
        state.overlays.verb_menu = Some(crate::state::VerbMenuState {
            pane: crate::state::VerbMenuPane::Verbs,
            verb_scroll: Default::default(),
            noun_scroll: Default::default(),
            prep_scroll: Default::default(),
            nouns,
            // Helper opens with a pane focused so legacy nav/pick tests keep
            // exercising the dock; ring/focus tests flip `story_focused` as needed.
            story_focused: false,
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
        assert_eq!(s.input.value, "look ");

        // Pick again → "look look ".
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input.value, "look look ");
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
        s.overlays.verb_menu.as_mut().unwrap().verb_scroll.selected = unlock_idx;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unlock ");

        // 2. Switch to Nouns pane and pick "door" (noun_idx=0).
        s.overlays.verb_menu.as_mut().unwrap().pane = crate::state::VerbMenuPane::Nouns;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unlock door ");

        // 3. Switch to Preps pane and pick "with".
        s.overlays.verb_menu.as_mut().unwrap().pane = crate::state::VerbMenuPane::Preps;
        s.overlays.verb_menu.as_mut().unwrap().prep_scroll.selected = with_idx;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unlock door with ");

        // 4. Switch back to Nouns and pick "key" (noun_idx=1).
        s.overlays.verb_menu.as_mut().unwrap().pane = crate::state::VerbMenuPane::Nouns;
        s.overlays.verb_menu.as_mut().unwrap().noun_scroll.selected = 1;
        apply_action(Action::VerbMenuPick, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unlock door with key ");
    }

    #[test]
    fn verb_menu_close_leaves_input_intact() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.input.set("unlock door ", true);
        open_verb_menu_with_nouns(&mut s, vec![]);
        apply_action(Action::VerbMenuClose, &mut s, &mut mapper);
        assert_eq!(s.input.value, "unlock door ", "input must be preserved");
    }

    #[test]
    fn verb_menu_close_arms_dock_toward_closed_without_nulling_content() {
        // Drawer pattern: VerbMenuClose must NOT immediately clear verb_menu —
        // content persists so the panel visibly slides out. It arms verb_dock
        // toward closed instead; settle_verb_dock (called once the slide
        // finishes) is what actually clears verb_menu.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        apply_action(Action::VerbMenuClose, &mut s, &mut mapper);
        assert!(s.overlays.verb_menu.is_some(), "verb_menu content should persist during slide-out");
        assert!(!s.verb_dock.open, "verb_dock should be armed toward closed");
    }

    #[test]
    fn open_verb_menu_arms_dock_toward_open() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        assert!(s.verb_dock.open, "verb_dock should be armed toward open");
    }

    #[test]
    fn settle_verb_dock_clears_content_once_slide_out_settles() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        assert!(s.overlays.verb_menu.is_some());

        // Mid-slide-out: still armed/active, content must persist.
        apply_action(Action::VerbMenuClose, &mut s, &mut mapper);
        s.settle_verb_dock();
        assert!(s.overlays.verb_menu.is_some(), "content should persist while the slide is (potentially) active");

        // Force-settle the tween (as if the animation had completed), then
        // settle_verb_dock should clear the content.
        s.verb_dock.toggle_to(false, true);
        s.settle_verb_dock();
        assert!(s.overlays.verb_menu.is_none(), "content should be cleared once the slide-out has settled");
    }

    // ── SQ-0237: interactive pane resize mode ─────────────────────────────────

    #[test]
    fn resize_panes_enters_mode_targeting_story_map() {
        // Default state is Layout::Split with no verb menu/inventory, so
        // StoryMap is the only (and first) visible target.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(s.resize_mode);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::StoryMap);
    }

    #[test]
    fn resize_nav_right_grows_split_ratio_and_clamps_at_80() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.split_ratio, 50);
        apply_action(Action::ResizeNav(ResizeNavKind::Right), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.split_ratio, 53);
        assert_eq!(s.config.split_ratio, 53, "config must mirror pane_sizes after nav");
        for _ in 0..20 {
            apply_action(Action::ResizeNav(ResizeNavKind::Right), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.split_ratio, 80, "clamped at 80");
        assert_eq!(s.config.split_ratio, 80);
    }

    #[test]
    fn resize_nav_left_shrinks_split_ratio_to_floor_20() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        for _ in 0..20 {
            apply_action(Action::ResizeNav(ResizeNavKind::Left), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.split_ratio, 20, "clamped at floor 20");
        assert_eq!(s.config.split_ratio, 20);
    }

    #[test]
    fn resize_nav_up_on_inv_dock_target_raises_inv_dock_pct_clamped_at_80() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.show_inventory = true;
        s.resize_mode = true;
        s.resize_target = crate::state::ResizeTarget::InvDock;
        assert_eq!(s.pane_sizes.inv_dock_pct, 33);
        apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.inv_dock_pct, 36);
        assert_eq!(s.config.inv_dock_pct, 36);
        for _ in 0..20 {
            apply_action(Action::ResizeNav(ResizeNavKind::Up), &mut s, &mut mapper);
        }
        assert_eq!(s.pane_sizes.inv_dock_pct, 80, "clamped at 80");
    }

    #[test]
    fn resize_reset_restores_defaults_and_mirrors_config() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        apply_action(Action::ResizeNav(ResizeNavKind::Right), &mut s, &mut mapper);
        assert_ne!(s.pane_sizes.split_ratio, 50);

        apply_action(Action::ResizeReset, &mut s, &mut mapper);
        assert_eq!(s.pane_sizes.split_ratio, 50);
        assert_eq!(s.pane_sizes.verb_dock_pct, 32);
        assert_eq!(s.pane_sizes.inv_dock_pct, 33);
        assert_eq!(s.config.split_ratio, 50);
        assert_eq!(s.config.verb_dock_pct, 32);
        assert_eq!(s.config.inv_dock_pct, 33);
    }

    #[test]
    fn resize_reset_sets_pending_config_write() {
        // Regression for the reset-pane-size slash command never persisting:
        // the run loop's KeyResolve::Command dispatch path never reached the
        // old resize_persist-guarded write, so it needs this flag to signal
        // the pending write from apply_action instead. See flush_pending_config_write
        // in main.rs, called from both dispatch paths.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        assert!(!s.pending_config_write);
        apply_action(Action::ResizeReset, &mut s, &mut mapper);
        assert!(s.pending_config_write);
    }

    #[test]
    fn resize_exit_clears_resize_mode() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(s.resize_mode);
        apply_action(Action::ResizeExit, &mut s, &mut mapper);
        assert!(!s.resize_mode);
    }

    #[test]
    fn resize_exit_sets_pending_config_write() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(!s.pending_config_write);
        apply_action(Action::ResizeExit, &mut s, &mut mapper);
        assert!(s.pending_config_write);
    }

    #[test]
    fn resize_panes_is_noop_when_nothing_visible() {
        // TranscriptFull with no verb menu/inventory → no visible targets.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        s.layout = crate::state::Layout::TranscriptFull;
        apply_action(Action::ResizePanes, &mut s, &mut mapper);
        assert!(!s.resize_mode, "no visible pane → resize mode does not open");
    }

    #[test]
    fn resize_targets_visible_matches_spec_order() {
        let mut s = AppState::default();
        assert_eq!(s.resize_targets_visible(), vec![crate::state::ResizeTarget::StoryMap]);

        s.show_inventory = true;
        assert_eq!(
            s.resize_targets_visible(),
            vec![crate::state::ResizeTarget::StoryMap, crate::state::ResizeTarget::InvDock]
        );

        s.layout = crate::state::Layout::TranscriptFull;
        s.show_inventory = false;
        assert!(s.resize_targets_visible().is_empty());
    }

    #[test]
    fn resize_targets_visible_includes_verb_dock_when_menu_open() {
        // SQ-0238: resize mode now preempts the verb-menu key intercept, so the
        // two coexist and the verb dock becomes a resize target while the menu
        // is open (appended after StoryMap/InvDock in the Tab-cycle order).
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        assert_eq!(
            s.resize_targets_visible(),
            vec![crate::state::ResizeTarget::StoryMap, crate::state::ResizeTarget::VerbDock]
        );

        s.show_inventory = true;
        assert_eq!(
            s.resize_targets_visible(),
            vec![
                crate::state::ResizeTarget::StoryMap,
                crate::state::ResizeTarget::InvDock,
                crate::state::ResizeTarget::VerbDock,
            ]
        );
    }

    #[test]
    fn cycle_resize_target_wraps_and_skips_non_visible() {
        let mut s = AppState::default();
        s.show_inventory = true;
        s.resize_target = crate::state::ResizeTarget::StoryMap;

        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::InvDock, "wraps forward");
        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::StoryMap, "wraps forward again");

        s.cycle_resize_target(false);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::InvDock, "wraps backward");

        // Current target not visible → snaps to the first visible one.
        s.show_inventory = false;
        s.resize_target = crate::state::ResizeTarget::InvDock;
        s.cycle_resize_target(true);
        assert_eq!(s.resize_target, crate::state::ResizeTarget::StoryMap);
    }

    #[test]
    fn verb_menu_nav_tab_and_arrows_switch_pane() {
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().pane, VerbMenuPane::Verbs);

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
    fn open_verb_menu_focuses_story() {
        // (a) The dock opens with the story input live.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        assert!(s.overlays.verb_menu.as_ref().unwrap().story_focused, "dock opens story-focused");
    }

    #[test]
    fn verb_menu_tab_ring_includes_story() {
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string()]);
        // Start on the story.
        s.overlays.verb_menu.as_mut().unwrap().story_focused = true;

        // (b) Tab from Story focuses Verbs.
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::NextPane), &mut s, &mut mapper);
        {
            let vm = s.overlays.verb_menu.as_ref().unwrap();
            assert!(!vm.story_focused && vm.pane == VerbMenuPane::Verbs, "Story → Verbs");
        }

        // (c) Verbs → Nouns → Preps → Story (three more NextPane).
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::NextPane), &mut s, &mut mapper);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().pane, VerbMenuPane::Nouns);
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::NextPane), &mut s, &mut mapper);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().pane, VerbMenuPane::Preps);
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::NextPane), &mut s, &mut mapper);
        assert!(s.overlays.verb_menu.as_ref().unwrap().story_focused, "Preps → Story (ring wraps)");
    }

    #[test]
    fn verb_menu_intercept_respects_story_focus() {
        // (d) When the story is focused, letters/Enter fall through (None); when
        // a pane is focused, Enter picks.
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);

        s.overlays.verb_menu.as_mut().unwrap().story_focused = true;
        assert!(verb_menu_intercept(key(KeyCode::Char('x')), &s).is_none(), "letter falls through");
        assert!(verb_menu_intercept(key(KeyCode::Enter), &s).is_none(), "Enter falls through to game");

        s.overlays.verb_menu.as_mut().unwrap().story_focused = false;
        assert!(
            matches!(verb_menu_intercept(key(KeyCode::Enter), &s), Some(Action::VerbMenuPick)),
            "Enter picks when a pane is focused"
        );
    }

    #[test]
    fn verb_menu_click_token_inserts_and_keeps_focus() {
        // (e) Clicking a token appends it + a space and leaves story_focused as-is.
        use crate::render::verbmenu::VERB_MENU_VERBS;
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        s.overlays.verb_menu.as_mut().unwrap().story_focused = true;

        apply_action(Action::VerbMenuClickToken(VerbMenuPane::Verbs, 2), &mut s, &mut mapper);
        assert_eq!(s.input.value, format!("{} ", VERB_MENU_VERBS[2]));
        assert!(s.overlays.verb_menu.as_ref().unwrap().story_focused, "click leaves story focus unchanged");
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 2, "click moves highlight");
    }

    #[test]
    fn verb_menu_focus_pane_action_sets_pane() {
        // (f) Clicking a header focuses that pane.
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string()]);
        s.overlays.verb_menu.as_mut().unwrap().story_focused = true;

        apply_action(Action::VerbMenuFocusPane(VerbMenuPane::Nouns), &mut s, &mut mapper);
        let vm = s.overlays.verb_menu.as_ref().unwrap();
        assert!(!vm.story_focused && vm.pane == VerbMenuPane::Nouns, "header click focuses Nouns");
    }

    #[test]
    fn verb_menu_nav_up_down_moves_index() {
        use crate::state::VerbMenuPane;
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string(), "mailbox".to_string()]);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 0);

        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Down), &mut s, &mut mapper);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 1);

        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 0);

        // Up at 0 stays at 0 (saturating).
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Up), &mut s, &mut mapper);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 0);

        // Switch to Nouns, move down.
        s.overlays.verb_menu.as_mut().unwrap().pane = VerbMenuPane::Nouns;
        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Down), &mut s, &mut mapper);
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().noun_scroll.selected, 1);
    }

    #[test]
    fn verb_menu_wheel_scrolls_while_story_focused() {
        // Regression (SQ-0255): the dock opens story-focused, and the mouse wheel
        // produces VerbMenuNav(Up/Down) regardless of focus. List movement must
        // therefore scroll the active pane even while the story input is focused;
        // gating it behind !story_focused leaves wheel-scroll dead until Tab.
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        assert!(s.overlays.verb_menu.as_ref().unwrap().story_focused, "dock opens story-focused");
        assert_eq!(s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 0);

        apply_action(Action::VerbMenuNav(VerbMenuNavKind::Down), &mut s, &mut mapper);
        assert_eq!(
            s.overlays.verb_menu.as_ref().unwrap().verb_scroll.selected, 1,
            "wheel/Down must scroll the active pane even while story-focused"
        );
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
        // "Story always live": with the dock open, an ordinary letter is not
        // swallowed — it falls through to the game input line instead of closing.
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        let a = key_to_action(&s, key(KeyCode::Char('q')));
        assert!(matches!(a, Action::InputChar('q')), "q types into the live story input, not close");
    }

    #[test]
    fn open_verb_menu_key_m_is_unbound_by_default() {
        // open-verb-menu is leader-only now (SQ-0202); 'm' has no default binding.
        use crate::keymap::{KeyMap, KeySpec};
        use crossterm::event::KeyCode;
        let km = KeyMap::default();
        let spec = KeySpec { code: KeyCode::Char('m'), ctrl: false, shift: false, alt: false };
        let cmd = km.lookup(&spec, crate::keymap::Context::Global);
        assert_eq!(cmd, None, "m should not be bound to open-verb-menu by default");
    }

    #[test]
    fn open_verb_menu_in_view_dialog_group() {
        use crate::keymap::HotkeyLayout;
        let layout = HotkeyLayout::default();
        let view_group = layout.groups.iter().find(|(title, _)| title == "View");
        assert!(view_group.is_some(), "View group should exist");
        let (_, cmds) = view_group.unwrap();
        assert!(cmds.iter().any(|c| c.1 == "open-verb-menu"), "open-verb-menu should be in View group");
    }

    #[test]
    fn open_verb_menu_action_opens_modal() {
        let mut s = AppState::default();
        let mut mapper = Mapper::default();
        assert!(s.overlays.verb_menu.is_none());
        apply_action(Action::OpenVerbMenu, &mut s, &mut mapper);
        assert!(s.overlays.verb_menu.is_some(), "verb_menu should be Some after OpenVerbMenu");
        assert!(matches!(s.overlays.verb_menu.as_ref().unwrap().pane, crate::state::VerbMenuPane::Verbs));
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
        let nouns = &s.overlays.verb_menu.as_ref().unwrap().nouns;
        let mailbox_count = nouns.iter().filter(|n| n.as_str() == "mailbox").count();
        assert_eq!(mailbox_count, 1, "mailbox should appear exactly once in nouns (dedup)");
    }

    // ── File-browser sub-mode key tests ───────────────────────────────────────

    /// Build a state with saves open (for testing e/i dispatch).
    fn state_with_saves_for_fb_tests() -> AppState {
        let mut s = AppState::default();
        s.overlays.saves = Some(crate::state::SavesState { entries: Vec::new(), scroll: Default::default() });
        s
    }

    /// Build a state with the file browser open.
    fn state_with_filebrowser(mode: crate::state::FbMode) -> AppState {
        use crate::state::FileBrowserState;
        let mut s = AppState::default();
        let tmp = std::env::temp_dir();
        s.overlays.file_browser = Some(FileBrowserState::build(tmp, mode));
        s
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
        let s = state_with_filebrowser(crate::state::FbMode::PickFile);
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
    fn fb_close_action_clears_file_browser() {
        let mut s = state_with_filebrowser(crate::state::FbMode::PickFile);
        assert!(s.overlays.file_browser.is_some());
        apply_action(Action::FbClose, &mut s, &mut Mapper::default());
        assert!(s.overlays.file_browser.is_none(), "FbClose should clear file_browser");
    }

    #[test]
    fn fb_nav_wraps_around() {
        let mut s = state_with_filebrowser(crate::state::FbMode::PickFile);
        // We need at least one entry — the tmp dir should have ".." if not root.
        if let Some(fb) = &s.overlays.file_browser {
            if fb.entries.is_empty() {
                return; // nothing to navigate
            }
        }
        // Move up from 0 should wrap to last entry.
        apply_action(Action::FbNav(-1), &mut s, &mut Mapper::default());
        if let Some(fb) = &s.overlays.file_browser {
            assert_eq!(fb.scroll.selected, fb.entries.len() - 1, "nav -1 from 0 should wrap to last");
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
        // Drag 5 columns right, 3 rows down (grab: content follows cursor).
        apply_action(Action::DragPanTo(15, 13), &mut s, &mut m);
        assert_eq!(
            s.char_pan,
            (5, 3),
            "drag right+down by (5,3) should set char_pan to (5,3)"
        );
        // Continue dragging 2 columns left.
        apply_action(Action::DragPanTo(13, 13), &mut s, &mut m);
        assert_eq!(
            s.char_pan,
            (3, 3),
            "additional drag left by 2 should update char_pan to (3,3)"
        );
    }

    /// Ending the drag clears state.drag but leaves char_pan intact.
    #[test]
    fn end_drag_pan_leaves_char_pan() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::BeginDragPan(5, 5), &mut s, &mut m);
        apply_action(Action::DragPanTo(8, 5), &mut s, &mut m);
        assert_eq!(s.char_pan, (3, 0));
        apply_action(Action::EndDragPan, &mut s, &mut m);
        assert!(s.drag.is_none(), "EndDragPan should clear drag state");
        assert_eq!(s.char_pan, (3, 0), "EndDragPan must not reset char_pan");
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
            field: None,
        };

        // State with config_screen open (so dialog routing knows which modal).
        let mut state = AppState::default();
        let working = crate::input::clone_config(&state.config);
        state.overlays.config_screen = Some(ConfigScreenState { working, scroll: Default::default() });

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
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
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
            field: None,
        };

        let mut state = AppState::default();
        state.overlays.saves = Some(SavesState {
            entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.babelmap"),
                name: "a".into(),
                turns: 0,
                saved_at: String::new(),
                location: None, score: None, is_default: false,
            }],
            scroll: Default::default(),
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
            field: None,
        };

        let mut state = AppState::default();
        let tmp = std::env::temp_dir();
        state.overlays.file_browser = Some(crate::state::FileBrowserState::build(
            tmp,
            crate::state::FbMode::PickFile));

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
    fn verb_menu_dock_does_not_swallow_map_clicks() {
        // The verb menu is now a persistent left dock (no [X]/[Done] chrome, no
        // DialogRects), not a centered modal — a click in the map pane must
        // route normally instead of being swallowed as "centered modal" chrome.
        use ratatui::layout::Rect;
        use crate::state::VerbMenuState;

        let mut state = AppState::default();
        state.overlays.verb_menu = Some(VerbMenuState {
            pane: crate::state::VerbMenuPane::Verbs,
            verb_scroll: Default::default(),
            noun_scroll: Default::default(),
            prep_scroll: Default::default(),
            nouns: vec![],
            story_focused: false,
        });

        let map   = Rect::new(30, 0, 50, 24);
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = None;

        let a = mouse_to_action(&state, mouse_left_click(40, 5), map, story, room_rects, &dialog);
        assert!(!matches!(a, Action::None), "click in map should route normally with the verb dock open, got {:?}", a);
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
            field: None,
        };

        let mut state = AppState::default();
        state.overlays.hotkey_dialog = true;

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
        use crate::state::SavesState;
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
            field: None,
        };

        // 2. Saves: ESC → SavesClose, [X] → SavesClose
        {
            let mut s = AppState::default();
            s.overlays.saves = Some(SavesState { entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.babelmap"), name: "a".into(), turns: 0,
                saved_at: String::new(), location: None, score: None, is_default: false,
            }], scroll: Default::default() });
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
            s.overlays.file_browser = Some(crate::state::FileBrowserState::build(
                std::env::temp_dir(), crate::state::FbMode::PickFile));
            let esc_action = key_to_action(&s, key(KeyCode::Esc));
            assert!(matches!(esc_action, Action::FbClose),
                "file browser ESC should produce FbClose, got {:?}", esc_action);
            let dialog = Some(make_rects());
            let x_action = mouse_to_action(&s, mouse_left_click(99, 0), map, story, room_rects, &dialog);
            assert!(matches!(x_action, Action::FbClose),
                "file browser [X] click should produce FbClose, got {:?}", x_action);
        }

        // 4. Verb menu is now a left dock, not a centered modal with [X]/[Done]
        // chrome, so it's out of scope for this ESC == [X]-click sweep. Its
        // ESC → VerbMenuClose mapping is covered by `verb_menu_esc_closes`.

        // 5. Config screen: ESC → ConfigCancel, [X] → ConfigCancel
        {
            let mut s = AppState::default();
            let working = clone_config(&s.config);
            s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
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
            s.overlays.hotkey_dialog = true;
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
        use crate::state::{SavesState, VerbMenuState};
        use crate::persist_files::SaveInfo;
        use std::path::PathBuf;

        // Saves: q → not SavesClose
        {
            let mut s = AppState::default();
            s.overlays.saves = Some(SavesState { entries: vec![SaveInfo {
                path: PathBuf::from("/tmp/a.babelmap"), name: "a".into(), turns: 0,
                saved_at: String::new(), location: None, score: None, is_default: false,
            }], scroll: Default::default() });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::SavesClose),
                "q must not close the saves modal");
        }

        // File browser: q → not FbClose
        {
            let mut s = AppState::default();
            s.overlays.file_browser = Some(crate::state::FileBrowserState::build(
                std::env::temp_dir(), crate::state::FbMode::PickFile));
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::FbClose),
                "q must not close the file browser");
        }

        // Verb menu: q → not VerbMenuClose
        {
            let mut s = AppState::default();
            s.overlays.verb_menu = Some(VerbMenuState {
                pane: crate::state::VerbMenuPane::Verbs,
                verb_scroll: Default::default(), noun_scroll: Default::default(), prep_scroll: Default::default(), nouns: vec![], story_focused: false,
            });
            let a = key_to_action(&s, key(KeyCode::Char('q')));
            assert!(!matches!(a, Action::VerbMenuClose),
                "q must not close the verb menu");
        }

        // Config screen: q → not ConfigCancel
        {
            let mut s = AppState::default();
            let working = clone_config(&s.config);
            s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
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

    /// Regression: a centered modal (config screen) stacked on top of an open
    /// corner overlay (room_panel) must swallow all outside-dialog clicks. Without
    /// the fix, is_corner_overlay was true even when a centered modal was open, so
    /// the outside click fell through to ShowRoomInfo / ActivatePane.
    #[test]
    fn centered_modal_swallows_outside_clicks_even_with_room_panel_open() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::{ConfigScreenState, RoomPanel, RoomPanelMode};
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

        // Now open BOTH room_panel (corner overlay) AND config screen (centered modal).
        let mut state = AppState::default();
        state.zoom = Zoom::Compact;
        state.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        let working = clone_config(&state.config);
        state.overlays.config_screen = Some(ConfigScreenState { working, scroll: Default::default() });

        // The dialog rects represent the config screen's centered dialog (not covering (0,0)).
        let dialog = Some(DialogRects {
            area:    Rect::new(5, 3, 70, 24),
            content: Rect::new(6, 5, 68, 19),
            close:   Some(Rect::new(73, 3, 1, 1)),
            buttons: vec![(ButtonId::Done, Rect::new(65, 26, 8, 1))],
            field: None,
        });

        // Click OUTSIDE the config screen dialog (at (0,0), which is on the room).
        // Must be swallowed — NOT ShowRoomInfo or ActivatePane.
        let a = mouse_to_action(&state, mouse_left_click(0, 0), map_r, story_r, &live_room_rects, &dialog);
        assert!(
            matches!(a, Action::None),
            "outside-config-screen click with room_panel also open must be swallowed (None), got {:?}", a
        );
    }

    // ── SQ-0349: center-map target ───────────────────────────────────────────

    /// Two placed rooms; `current` is #1 at (2,2), #2 sits at (7,7).
    fn recenter_fixture() -> (AppState, Mapper) {
        let mut m = Mapper::default();
        m.graph.upsert_room(1, "Here".into());
        m.graph.upsert_room(2, "There".into());
        m.graph.set_pos(1, (2, 2));
        m.graph.set_pos(2, (7, 7));
        m.graph.set_current(1);
        (AppState::default(), m)
    }

    /// SQ-0361: merging showed the layer the PLAYER was standing in, not the one the rooms went
    /// to — so a merge from a nested layer looked like it had dumped everything on the top layer,
    /// when the rooms had gone to the parent and were merely off-screen.
    #[test]
    fn merging_a_layer_shows_where_the_rooms_went_not_where_the_player_is() {
        let mut m = Mapper::default();
        m.observe(1, "Hall", None);
        m.observe(2, "Cellar", Some(Direction::Down));
        let mut s = AppState::default();
        apply_action(Action::PeelLayer(None), &mut s, &mut m); // Cellar -> L1
        m.observe(3, "Vault", Some(Direction::E)); // joins L1
        m.observe(2, "Cellar", Some(Direction::W)); // walk back, so the Vault has a way out to cut
        // Peel the Vault by standing IN it and cutting the way back out: a peel takes the selected
        // room's own side (SQ-0364).
        s.select_room(Some(3));
        apply_action(Action::PeelLayer(Some(Direction::W)), &mut s, &mut m); // Vault -> L2
        let vault_layer = m.graph.layer_of(3);
        assert_eq!(m.graph.layers()[&vault_layer].parent, Some(1), "L2 was peeled out of L1");

        // The player walks back up: they are on the TOP layer, nowhere near the Vault.
        m.observe(1, "Hall", Some(Direction::Up));
        assert_eq!(m.graph.layer_of(1), mapper::layer::MAIN_LAYER);

        s.set_viewed_layer(Some(vault_layer));
        apply_action(Action::MergeLayer, &mut s, &mut m);

        assert_eq!(m.graph.layer_of(3), 1, "the Vault merges into its parent, the Cellar");
        assert_eq!(
            s.active_layer(&m.graph),
            1,
            "and the map shows the Cellar, where the rooms went — not the player's own top layer"
        );
    }

    // ── SQ-0360: peel at a named seam, and say why a peel refused ────────────

    /// A layer with no portal seam in it (Zork's Cellar: 35 rooms of solid compass maze) could not
    /// be divided at all, and the refusal was SILENT — the command simply did nothing.
    #[test]
    fn peel_layer_with_a_direction_cuts_a_layer_that_refuses_the_plain_peel() {
        let mut m = Mapper::default();
        m.observe(1, "Round Room", None);
        m.observe(2, "Loud Room", Some(Direction::E));
        m.observe(3, "Damp Cave", Some(Direction::E));
        let mut s = AppState::default();
        s.select_room(Some(1));

        // Plain peel: one connected region, so it refuses — and now explains itself.
        apply_action(Action::PeelLayer(None), &mut s, &mut m);
        assert_eq!(m.graph.layers().len(), 1, "nothing peeled");
        let msg = s.notifications.latest_text().expect("a refusal must not be silent").to_string();
        assert!(msg.contains("one connected region"), "says why: {msg:?}");
        assert!(msg.contains("peel-layer <direction>"), "and points at the way forward: {msg:?}");

        // Naming the seam cuts there — and posts no complaint.
        let before = s.notifications.history().len();
        apply_action(Action::PeelLayer(Some(Direction::E)), &mut s, &mut m);
        assert_eq!(s.notifications.history().len(), before, "no complaint when it works");
        let new = s.viewed_layer.expect("the peeled layer is now in view");
        // #1 is selected, so #1's own side leaves — the same side plain `peel-layer` would take
        // (SQ-0364). The far side is what stays.
        assert_eq!(m.graph.rooms_in_layer(new), vec![1], "the selected room's side leaves");
        assert_eq!(m.graph.rooms_in_layer(0), vec![2, 3], "the far side stays put");
    }

    #[test]
    fn peel_layer_explains_a_passage_that_is_not_a_seam() {
        // A→B directly and A→C→B as well: cutting A-B separates nothing.
        let mut m = Mapper::default();
        m.observe(1, "A", None);
        m.observe(2, "B", Some(Direction::E));
        m.graph.add_edge(1, Direction::N, 3);
        m.graph.upsert_room(3, "C".into());
        m.graph.add_edge(3, Direction::E, 2);
        let mut s = AppState::default();
        s.select_room(Some(1));

        apply_action(Action::PeelLayer(Some(Direction::E)), &mut s, &mut m);
        let msg = s.notifications.latest_text().expect("refusal must speak").to_string();
        assert!(msg.contains("not a boundary"), "{msg:?}");
        assert_eq!(m.graph.layers().len(), 1, "nothing peeled");

        // And a direction with no passage at all is a different complaint.
        apply_action(Action::PeelLayer(Some(Direction::W)), &mut s, &mut m);
        let msg = s.notifications.latest_text().expect("refusal must speak").to_string();
        assert!(msg.contains("no W passage"), "{msg:?}");
    }

    #[test]
    fn center_map_uses_the_selected_room() {
        let (mut s, mut m) = recenter_fixture();
        s.select_room(Some(2));
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((7, 7), 80, 24);
        assert_eq!(s.scroll, want.scroll, "centred on the SELECTED room, not the current one");
    }

    #[test]
    fn center_map_measures_the_real_map_pane_not_a_guess() {
        // SQ-0349: `apply_recenter` assumed 80×24, because `apply_action` cannot reach the run
        // loop's pane rects. `recenter_on` divides the pane by the zoom step to place the view,
        // so on any other pane size the target landed off-centre — which is what made pressing
        // `c` look like it centred on something other than the selected room.
        let (mut s, mut m) = recenter_fixture();
        s.select_room(Some(2));
        s.map_pane_size.set(Some((140, 48))); // what the renderer measured this frame
        apply_action(Action::Recenter, &mut s, &mut m);

        let mut want = AppState::default();
        want.recenter_on((7, 7), 140, 48);
        assert_eq!(s.scroll, want.scroll, "centred against the pane that was actually drawn");

        let mut guessed = AppState::default();
        guessed.recenter_on((7, 7), 80, 24);
        assert_ne!(s.scroll, guessed.scroll, "and not against the old 80×24 guess");
    }

    #[test]
    fn center_map_falls_back_to_eighty_by_twentyfour_before_the_first_frame() {
        // No frame has been drawn, so there is no pane to measure. The old constant is still the
        // only answer available — but it is now a fallback, not the assumption.
        let (mut s, mut m) = recenter_fixture();
        s.select_room(Some(2));
        assert!(s.map_pane_size.get().is_none(), "renderer has not run");
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((7, 7), 80, 24);
        assert_eq!(s.scroll, want.scroll);
    }

    #[test]
    fn center_map_falls_back_to_the_current_room_not_the_origin() {
        // SQ-0349: with nothing selected it recentred on (0,0) — an arbitrary corner of the map
        // that need not hold a room at all. The player's own location is the only sensible answer.
        let (mut s, mut m) = recenter_fixture();
        assert!(s.selected_room.is_none(), "nothing selected");
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((2, 2), 80, 24);
        assert_eq!(s.scroll, want.scroll, "centred on the CURRENT room");

        let mut origin = AppState::default();
        origin.recenter_on((0, 0), 80, 24);
        assert_ne!(s.scroll, origin.scroll, "and not on the origin");
    }

    #[test]
    fn center_map_falls_through_an_unplaced_selection_to_the_current_room() {
        // A selected room with no position cannot be centred on. Dropping to the origin would
        // strand the view; the current room is still a real answer.
        let (mut s, mut m) = recenter_fixture();
        m.graph.upsert_room(9, "Unplaced".into()); // never given a pos
        s.select_room(Some(9));
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((2, 2), 80, 24);
        assert_eq!(s.scroll, want.scroll, "fell through to the current room");
    }

    #[test]
    fn center_map_with_nothing_placed_at_all_uses_the_origin() {
        // Last resort, unchanged: no selection, no current room, nothing to aim at.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::Recenter, &mut s, &mut m);
        let mut want = AppState::default();
        want.recenter_on((0, 0), 80, 24);
        assert_eq!(s.scroll, want.scroll);
    }

    // ── SQ-0354: caret editing on the command line ───────────────────────────

    #[test]
    fn arrows_move_the_caret_and_typing_lands_at_it() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "north".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert_eq!(s.input.cursor, 5, "typing leaves the caret at the end");

        apply_action(Action::CursorLeft, &mut s, &mut m);
        apply_action(Action::CursorLeft, &mut s, &mut m);
        assert_eq!(s.input.cursor, 3);
        apply_action(Action::InputChar('X'), &mut s, &mut m);
        assert_eq!(s.input.value, "norXth", "a typed char lands AT the caret, not at the end");

        apply_action(Action::CursorHome, &mut s, &mut m);
        assert_eq!(s.input.cursor, 0);
        apply_action(Action::CursorEnd, &mut s, &mut m);
        assert_eq!(s.input.cursor, 6);
        // Clamped at both ends.
        apply_action(Action::CursorRight, &mut s, &mut m);
        assert_eq!(s.input.cursor, 6, "Right at the end does not run off");
        apply_action(Action::CursorHome, &mut s, &mut m);
        apply_action(Action::CursorLeft, &mut s, &mut m);
        assert_eq!(s.input.cursor, 0, "Left at the start does not run off");
    }

    #[test]
    fn backspace_and_delete_cut_opposite_sides_of_the_caret() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "abc".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        apply_action(Action::CursorLeft, &mut s, &mut m); // between b and c
        apply_action(Action::Backspace, &mut s, &mut m);
        assert_eq!(s.input.value, "ac", "Backspace cuts the char BEFORE the caret");
        apply_action(Action::DeleteChar, &mut s, &mut m);
        assert_eq!(s.input.value, "a", "Delete cuts the char AT the caret");
    }

    #[test]
    fn right_at_the_end_accepts_the_suggestion() {
        // SQ-0354: at the end of the line there is nothing to move onto, so Right would be a dead
        // key. Taking the showing suggestion is the only useful reading (the fish/zsh gesture).
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "/toggle-roo".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert!(!s.suggestions.is_empty(), "a suggestion is showing: {:?}", s.suggestions);
        let want = format!("/{}", s.suggestions[0]);
        apply_action(Action::CursorRight, &mut s, &mut m);
        assert_eq!(s.input.value, want, "Right at the end accepted the suggestion");
        assert!(s.suggestion_active, "and marks it applied, exactly as Tab does");
    }

    #[test]
    fn right_mid_line_moves_the_caret_even_with_a_suggestion_showing() {
        // The accept gesture must not eat plain caret movement: it fires ONLY at the end.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "/toggle-roo".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert!(!s.suggestions.is_empty());
        let before = s.input.value.clone();
        apply_action(Action::CursorHome, &mut s, &mut m);
        apply_action(Action::CursorRight, &mut s, &mut m);
        assert_eq!(s.input.value, before, "mid-line Right leaves the text alone");
        assert_eq!(s.input.cursor, 1, "it just moves the caret");
    }

    #[test]
    fn a_click_on_the_input_line_places_the_caret() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "north".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        // The renderer records where the text landed; without a frame there is nothing to click.
        assert!(s.input_click_index(10, 5).is_none(), "no origin captured yet -> no mapping");
        s.input_text_origin.set(Some((10, 5)));

        assert_eq!(s.input_click_index(12, 5), Some(2), "click on the 3rd char");
        assert_eq!(s.input_click_index(12, 4), None, "a different row is not the input line");
        assert_eq!(s.input_click_index(9, 5), None, "left of the text is not the input line");
        assert_eq!(s.input_click_index(99, 5), Some(5), "past the end clamps to the end");

        apply_action(Action::CursorToClick(12, 5), &mut s, &mut m);
        assert_eq!(s.input.cursor, 2, "the click moved the caret");
    }

    #[test]
    fn multi_byte_input_survives_caret_editing() {
        // The byte arithmetic this replaced would panic outright here: String::truncate rejects a
        // non-char boundary, and every one of these chars is multi-byte.
        let mut s = AppState::default();
        let mut m = Mapper::default();
        for c in "héllo".chars() {
            apply_action(Action::InputChar(c), &mut s, &mut m);
        }
        assert_eq!(s.input.char_len(), 5);
        apply_action(Action::CursorHome, &mut s, &mut m);
        apply_action(Action::CursorRight, &mut s, &mut m);
        apply_action(Action::InputChar('X'), &mut s, &mut m);
        assert_eq!(s.input.value, "hXéllo", "insert lands on a char boundary");
        apply_action(Action::DeleteChar, &mut s, &mut m);
        assert_eq!(s.input.value, "hXllo", "delete cuts a whole char, not a byte");
    }

    // ── slash_suggestions tests ───────────────────────────────────────────────

    #[test]
    fn slash_suggestions_filter_by_prefix() {
        let names = vec!["panh".to_string(),"panv".to_string(),"zoom".to_string(),"open-config".to_string()];
        let s = slash_suggestions("pa", &names, 6);
        assert!(s.contains(&"panh".to_string()) && s.contains(&"panv".to_string()));
        assert!(!s.contains(&"zoom".to_string()));
    }

    #[test]
    fn slash_suggestions_match_any_part_of_the_name() {
        // SQ-0353. Command names are compound (`toggle-room-numbers`), and the part you remember is
        // usually the NOUN, not the verb it happens to start with. Prefix-only matching meant the
        // word you could actually recall found nothing at all.
        let names = crate::slash::slash_names();
        let s = slash_suggestions("room", &names, 10);
        for want in ["toggle-room-numbers", "select-room", "nudge-room", "rename-room"] {
            assert!(s.contains(&want.to_string()), "'room' must offer {want}: {s:?}");
        }
        // And it must not turn into a free-for-all: a name without the token stays out.
        assert!(!s.iter().any(|n| !n.contains("room")), "every hit contains the token: {s:?}");
    }

    #[test]
    fn slash_suggestions_rank_prefix_then_word_start_then_middle() {
        // Substring matching is only useful if the obvious answer still comes first. Typing the
        // start of a name must not bury it under incidental mid-word hits elsewhere.
        let names: Vec<String> = ["map-x", "zoom-map", "unmappable", "map-y"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let s = slash_suggestions("map", &names, 10);
        assert_eq!(
            s,
            vec!["map-x", "map-y", "zoom-map", "unmappable"],
            "prefix first, then a name-part start, then mid-word; alphabetical within each rank",
        );
    }

    #[test]
    fn slash_suggestions_never_offer_what_is_already_typed() {
        // Pre-existing contract, kept: a completion identical to the token is noise.
        let names = vec!["map".to_string(), "zoom-map".to_string()];
        let s = slash_suggestions("map", &names, 6);
        assert!(!s.contains(&"map".to_string()), "exact token is not a suggestion: {s:?}");
        assert!(s.contains(&"zoom-map".to_string()), "but other substring hits still show: {s:?}");
    }

    /// Regression: CONFIG_ROW_COUNT must equal CONFIG_ROWS.len() so every config row is
    /// keyboard-reachable.  If a row is added to CONFIG_ROWS without updating this constant,
    /// Down from the penultimate row wraps to row 0 and the last row becomes unreachable.
    #[test]
    fn config_row_count_matches_config_rows_len() {
        assert_eq!(
            CONFIG_ROW_COUNT,
            crate::render::config_screen::CONFIG_ROWS.len(),
            "CONFIG_ROW_COUNT must equal CONFIG_ROWS.len(); update CONFIG_ROW_COUNT when adding/removing rows"
        );
    }

    #[test]
    fn config_cycle_interpreter_number_reaches_default() {
        let mut c = crate::config::Config::default();
        c.interpreter_number = None;
        config_cycle(&mut c, 20, 1); // default → 1
        assert_eq!(c.interpreter_number, Some(1));
        config_cycle(&mut c, 20, -1); // 1 → default
        assert_eq!(c.interpreter_number, None);
        config_cycle(&mut c, 20, -1); // default clamps, stays default
        assert_eq!(c.interpreter_number, None);
        for _ in 0..20 {
            config_cycle(&mut c, 20, 1); // climbs then clamps at 10
        }
        assert_eq!(c.interpreter_number, Some(10));
    }

    #[test]
    fn cycle_focus_wraps_both_ways() {
        assert_eq!(cycle_focus(0, 3, 1), 1);
        assert_eq!(cycle_focus(2, 3, 1), 0); // wrap forward
        assert_eq!(cycle_focus(0, 3, -1), 2); // wrap backward
        assert_eq!(cycle_focus(5, 0, 1), 0); // empty
    }

    // ── Task 3: config_screen Tab focus + Enter-activates-focused ─────────────

    #[test]
    fn config_screen_tab_then_enter_fires_cancel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        s.overlays.dialog_focus = cycle_focus(0, 2, 1); // focus Cancel (index 1)
        let a = config_screen_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.overlays.dialog_focus,
        );
        assert!(matches!(a, Action::ConfigCancel),
            "Enter with focus=1 (Cancel) should fire ConfigCancel, got {:?}", a);
    }

    #[test]
    fn config_screen_enter_at_default_focus_fires_save() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        s.overlays.dialog_focus = 0; // focus Save (default)
        let a = config_screen_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.overlays.dialog_focus,
        );
        assert!(matches!(a, Action::ConfigSave),
            "Enter with focus=0 (Save) should fire ConfigSave, got {:?}", a);
    }

    #[test]
    fn config_screen_space_still_toggles_row() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.overlays.config_screen = Some(crate::state::ConfigScreenState { working, scroll: Default::default() });
        // Space must toggle the selected row regardless of focus.
        for focus in [0, 1] {
            let a = config_screen_key_to_action(
                KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
                focus,
            );
            assert!(matches!(a, Action::ConfigToggle),
                "Space with focus={focus} should fire ConfigToggle, got {:?}", a);
        }
    }

    #[test]
    fn saves_tab_cycles_done_button_focus() {
        // The saves dialog has a ring of length 1 (Done only). Tab cycles 0 → 0 (stays).
        // Enter still loads the selected save (existing behavior).
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        s.overlays.saves = Some(crate::state::SavesState { entries: Vec::new(), scroll: Default::default() });
        s.overlays.dialog_focus = 0;
        // Tab with ring len 1 stays at 0.
        let after_tab = cycle_focus(s.overlays.dialog_focus, 1, 1);
        assert_eq!(after_tab, 0, "Tab on ring-len-1 should stay at 0");
        // Enter still produces SavesLoad (not SavesClose) regardless of focus.
        let a = saves_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.overlays.dialog_focus,
        );
        assert!(matches!(a, Action::SavesLoad),
            "Enter in saves should still fire SavesLoad (not affected by focus), got {:?}", a);
    }

    #[test]
    fn open_config_resets_dialog_focus() {
        let mut s = AppState::default();
        s.overlays.dialog_focus = 5; // non-zero
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::OpenConfig, &mut s, &mut m);
        assert_eq!(s.overlays.dialog_focus, 0, "OpenConfig must reset dialog_focus to 0");
    }

    #[test]
    fn open_saves_resets_dialog_focus_in_apply() {
        let mut s = AppState::default();
        s.overlays.dialog_focus = 5; // non-zero
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::OpenSaves, &mut s, &mut m);
        assert_eq!(s.overlays.dialog_focus, 0, "OpenSaves must reset dialog_focus to 0");
    }

    // ── Task 5: read-only / single-button panels ──────────────────────────────

    #[test]
    fn room_panel_enter_closes() {
        use crate::state::{RoomPanel, RoomPanelMode};
        let mut s = AppState::default();
        s.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(
            matches!(a, Action::CloseRoomPanel),
            "Enter must close the room panel (got {:?})",
            a
        );
    }

    #[test]
    fn hotkey_dialog_enter_closes() {
        let mut s = AppState::default();
        s.overlays.hotkey_dialog = true;
        let a = key_to_action(&s, key(KeyCode::Enter));
        assert!(
            matches!(a, Action::CloseHotkeyDialog),
            "Enter must close the hotkey dialog (got {:?})",
            a
        );
    }

    // ── Task 6: navigation panels — regression guard ──────────────────────────

    #[test]
    fn verb_menu_tab_still_navigates_panes() {
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        let a = verb_menu_intercept(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &s);
        assert!(matches!(a, Some(Action::VerbMenuNav(VerbMenuNavKind::NextPane))));
    }

    // ── SQ-0237: resize-mode key routing ──────────────────────────────────────

    #[test]
    fn resize_mode_key_to_action_maps_tab_and_backtab() {
        assert!(matches!(
            resize_mode_key_to_action(key(KeyCode::Tab)),
            Action::ResizeNav(ResizeNavKind::NextTarget)
        ));
        assert!(matches!(
            resize_mode_key_to_action(key(KeyCode::BackTab)),
            Action::ResizeNav(ResizeNavKind::PrevTarget)
        ));
    }

    #[test]
    fn resize_mode_key_to_action_maps_arrows() {
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Left)), Action::ResizeNav(ResizeNavKind::Left)));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Right)), Action::ResizeNav(ResizeNavKind::Right)));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Up)), Action::ResizeNav(ResizeNavKind::Up)));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Down)), Action::ResizeNav(ResizeNavKind::Down)));
    }

    #[test]
    fn resize_mode_key_to_action_maps_reset_and_exit() {
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Char('0'))), Action::ResizeReset));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Esc)), Action::ResizeExit));
        assert!(matches!(resize_mode_key_to_action(key(KeyCode::Enter)), Action::ResizeExit));
    }

    #[test]
    fn resize_mode_intercepts_tab_before_focus_toggle() {
        let mut s = AppState::default();
        s.resize_mode = true;
        let a = key_to_action(&s, key(KeyCode::Tab));
        assert!(matches!(a, Action::ResizeNav(ResizeNavKind::NextTarget)));
    }

    // ── SQ-0238: verb dock ⇄ resize mode coexistence ──────────────────────────

    /// Build a KeyEvent equal to the current hotkey-dialog leader prefix,
    /// regardless of what it is bound to (default ctrl+k, but another lane may
    /// move it) — the prefix must always pass through the verb-menu intercept.
    fn prefix_key(s: &AppState) -> KeyEvent {
        let p = s.hotkeys.prefix;
        let mut m = KeyModifiers::NONE;
        if p.ctrl { m |= KeyModifiers::CONTROL; }
        if p.shift { m |= KeyModifiers::SHIFT; }
        if p.alt { m |= KeyModifiers::ALT; }
        KeyEvent::new(p.code, m)
    }

    #[test]
    fn resize_mode_preempts_verb_menu_intercept() {
        // (a) With the verb menu open AND resize mode on, arrows/Tab resolve to
        // ResizeNav (resize owns them), not the verb-menu navigation actions.
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string()]);
        s.resize_mode = true;

        assert!(matches!(
            key_to_action(&s, key(KeyCode::Down)),
            Action::ResizeNav(ResizeNavKind::Down)
        ), "Down resizes, not VerbMenuNav");
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Tab)),
            Action::ResizeNav(ResizeNavKind::NextTarget)
        ), "Tab cycles resize target, not verb-menu pane");
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Esc)),
            Action::ResizeExit
        ), "Esc exits resize mode (back to the still-open menu), not VerbMenuClose");
    }

    #[test]
    fn verb_menu_intercept_passes_through_leader_prefix() {
        // (b) With the verb menu open, the leader prefix is NOT swallowed by the
        // verb-menu intercept, so it can open the leader palette (→ 'z' → resize).
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec![]);
        // Both focus states: the prefix must fall through either way.
        s.overlays.verb_menu.as_mut().unwrap().story_focused = true;
        assert!(verb_menu_intercept(prefix_key(&s), &s).is_none(), "prefix falls through (story focus)");
        s.overlays.verb_menu.as_mut().unwrap().story_focused = false;
        assert!(verb_menu_intercept(prefix_key(&s), &s).is_none(), "prefix falls through (pane focus)");

        // End-to-end: the prefix resolves to OpenHotkeyDialog while the menu is open.
        assert!(matches!(
            key_to_command(&s, prefix_key(&s)),
            KeyResolve::Action(Action::OpenHotkeyDialog)
        ), "prefix opens the hotkey dialog with the verb menu open");
    }

    #[test]
    fn verb_dock_is_a_resize_target_only_while_menu_open() {
        // The verb dock joins the resize Tab-cycle exactly when the menu is open.
        let mut s = AppState::default();
        assert!(!s.resize_targets_visible().contains(&crate::state::ResizeTarget::VerbDock));
        open_verb_menu_with_nouns(&mut s, vec![]);
        assert!(s.resize_targets_visible().contains(&crate::state::ResizeTarget::VerbDock));
    }

    #[test]
    fn verb_menu_nav_still_resolves_when_resize_mode_off() {
        // (c) With resize mode OFF, verb-menu navigation resolves normally.
        let mut s = AppState::default();
        open_verb_menu_with_nouns(&mut s, vec!["door".to_string()]);
        assert!(!s.resize_mode);
        assert!(matches!(
            key_to_action(&s, key(KeyCode::Tab)),
            Action::VerbMenuNav(VerbMenuNavKind::NextPane)
        ), "Tab cycles verb-menu pane when not resizing");
    }

    #[test]
    fn roominfo_ok_button_click_closes_panel() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::{RoomPanel, RoomPanelMode};

        let rects = DialogRects {
            area:    Rect::new(0, 0, 40, 15),
            content: Rect::new(1, 1, 38, 12),
            close:   Some(Rect::new(38, 0, 1, 1)),
            buttons: vec![(ButtonId::Ok, Rect::new(30, 14, 6, 1))],
            field: None,
        };

        let mut state = AppState::default();
        state.room_panel = Some(RoomPanel { id: 1, mode: RoomPanelMode::Info });

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // OK button click → CloseRoomPanel
        let a = mouse_to_action(&state, mouse_left_click(32, 14), map, story, room_rects, &dialog);
        assert!(
            matches!(a, Action::CloseRoomPanel),
            "room-info [OK] click should produce CloseRoomPanel, got {:?}", a
        );
    }

    #[test]
    fn tidy_ok_button_click_exits_anim() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::TidyAnim;
        use crate::state::TidyFrame;

        let rects = DialogRects {
            area:    Rect::new(0, 0, 40, 15),
            content: Rect::new(1, 1, 38, 12),
            close:   Some(Rect::new(38, 0, 1, 1)),
            buttons: vec![(ButtonId::Ok, Rect::new(30, 14, 6, 1))],
            field: None,
        };

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "test".to_string(),
            graph: mapper::graph::MapGraph::new(),
            description: String::new(),
            stats: mapper::layout::TidyStats::default(),
            stage_start: false,
            manifest: None,
        }], mapper::layer::MAIN_LAYER));

        let map   = Rect::default();
        let story = Rect::default();
        let room_rects: &[(mapper::graph::RoomId, Rect)] = &[];
        let dialog = Some(rects);

        // OK button click → AnimExit
        let a = mouse_to_action(&state, mouse_left_click(32, 14), map, story, room_rects, &dialog);
        assert!(
            matches!(a, Action::AnimExit),
            "tidy [OK] click should produce AnimExit, got {:?}", a
        );
    }

}
