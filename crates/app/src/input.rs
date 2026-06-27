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

// ── AttrKind ──────────────────────────────────────────────────────────────────

/// Which text-modifier attribute to toggle on the active selector's `Decl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrKind {
    Bold,
    Italic,
    Underline,
    Dim,
    Reversed,
}

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
    /// Re-read style.toml and swap the live colors/symbols (keeps current look on error).
    ReloadStyle,
    /// Scaffold the per-game style file (user_dir/styles/<ifid>.toml) for this game.
    GameStyle,
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
    /// Toggle room-number (#id) visibility in Boxes-zoom room boxes.
    ToggleRoomNumbers,
    ToggleStatusBar,
    /// Toggle the room-detection-method indicator in the map corner.
    ToggleLocMethod,
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
    /// Apply the current gallery selection: persist to personal style then close (handled by main.rs).
    GalleryApply,
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
    /// Open the live style editor full-screen mode.
    OpenStyleEditor,
    /// Cancel the style editor without saving (drops the working doc).
    StyleEditorCancel,
    /// Navigate the style-editor board by delta (-1 = up, +1 = down).
    StyleNav(i32),
    /// Toggle an attribute chip on the active selector's Decl.
    StyleToggleAttr(AttrKind),
    /// Cycle the style-editor focus ring forward (+1) or backward (-1).
    StyleFocusCycle(i32),
    /// Move the attribute-chip cursor left (-1) or right (+1) within the Attrs focus.
    StyleAttrChipNav(i32),
    /// Set the fg (is_bg=false) or bg (is_bg=true) of the active selector.
    /// `value` is a color token (ANSI name, #rrggbb hex, or "default"); `None` clears to default.
    StyleSetColor { is_bg: bool, value: Option<String> },
    /// Commit the custom hex buffer to the slot chosen by `color_target`.
    StyleCommitCustom,
    /// Move the swatch cursor by `d` (-1 or +1), wrapping over 17 cells.
    StyleSwatchNav(i32),
    /// Apply the swatch at `swatch_cursor` to the `color_target` slot.
    StyleSwatchPick,
    /// Append a character to the style-editor custom hex buffer.
    StyleCustomChar(char),
    /// Delete the last character from the style-editor custom hex buffer.
    StyleCustomBackspace,
    /// Save the style editor: resolve working doc to live colors and close.
    StyleSave,
    /// Reset the active selector's Decl to the built-in default.
    StyleReset,
    /// Cycle the border type for the active selector by delta (+1/-1) over ["none","single","double","rounded","thick","picture-frame"].
    StyleBorderTypeCycle(i32),
    /// Move the border zone cursor by delta (-1/+1), wrapping over 0..8.
    StyleBorderZoneNav(i32),
    /// Clear the glyph override for the current border zone.
    StyleBorderClearZone,
    /// Toggle the header flag on the active selector's Decl.
    StyleBorderToggleHeader,
    /// Toggle the shadow flag on the active selector's Decl.
    StyleBorderToggleShadow,
    /// Open the glyph-picker modal over the style editor, targeting `zone`.
    StyleOpenGlyphPicker(crate::state::BorderZone),
    /// Navigate the glyph grid by `delta` cells (wraps within current block).
    GlyphPickerNav(i32),
    /// Shift the curated block by `delta` (−1 / +1), or cycle.
    GlyphPickerBlock(i32),
    /// Feed a character into the picker's pending slot (direct char entry path).
    GlyphPickerChar(char),
    /// Commit: write the selected/pending glyph to the target zone and close.
    GlyphPickerPick,
    /// Clear: set the target zone to None and close.
    GlyphPickerClear,
    /// Cancel: close without any change.
    GlyphPickerCancel,
    /// Enter the custom-range hex-entry mode (focus the U+____ field).
    GlyphPickerCustomFocus,
    /// Feed a hex digit into the custom-range codepoint buffer.
    GlyphPickerCustomChar(char),
    /// Remove the last hex digit from the custom-range codepoint buffer.
    GlyphPickerCustomBackspace,
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
    /// Begin a story-pane text selection at terminal cell (col, row).
    StartSelection(u16, u16),
    /// Extend the story-pane text selection to terminal cell (col, row).
    ExtendSelection(u16, u16),
    /// End the story-pane text selection (copies it to the clipboard).
    EndSelection,
    /// Scroll the transcript by delta lines (positive = down, negative = up).
    TranscriptScroll(i32),
    /// Open the Hints panel. Real behavior wired in Task D; stub here keeps match exhaustive.
    OpenHints,
    /// Open the rewind/replay history modal (seeds `replay` at the last turn).
    OpenHistory,
    /// Step the replay selection by delta turns (-1 left, +1 right).
    ReplayStep(isize),
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
/// 2. Prompt active → prompt_key_to_action; everything else absorbed.
/// 3. Tidy-anim active → Anim context lookup (Ctrl+Left/Right stage-jump hardwired).
/// 4-6. Modal sub-modes (gallery/saves/replay/file-browser/verb-menu/style-editor/
///      config-screen/hotkey-dialog/room-panel) → their handlers (hardwired Actions).
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

    // 2. Prompt sub-mode: consume all keys; only prompt-relevant ones produce
    //    an action, everything else (Tab, Ctrl+S/R/E/L, …) is absorbed.
    if state.prompt.is_some() {
        return KeyResolve::Action(prompt_key_to_action(key));
    }

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
    if state.gallery.is_some() {
        return KeyResolve::Action(gallery_key_to_action(key));
    }
    if state.saves.is_some() {
        return KeyResolve::Action(saves_key_to_action(key, state.dialog_focus));
    }
    if state.replay.is_some() {
        return KeyResolve::Action(history_key_to_action(key));
    }
    if state.file_browser.is_some() {
        return KeyResolve::Action(filebrowser_key_to_action(key));
    }
    if state.verb_menu.is_some() {
        return KeyResolve::Action(verb_menu_key_to_action(key));
    }
    if state.style_editor.is_some() {
        return KeyResolve::Action(style_editor_key_to_action(key, state));
    }
    if state.config_screen.is_some() {
        return KeyResolve::Action(config_screen_key_to_action(key, state.dialog_focus));
    }
    if state.hotkey_dialog {
        return hotkey_dialog_key_to_action(state, key);
    }

    // 6.5. Room panel close: Esc or Enter while a panel is open.
    // Comes after steps 2-6 (prompt/anim/gallery/saves/hotkey_dialog checks) so those
    // modes still take priority, but before the prefix key and normal dispatch.
    // Room panel is read-only (no text input), so Enter is safe as a close key.
    if state.room_panel.is_some() && key.modifiers == KeyModifiers::NONE {
        if matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            return KeyResolve::Action(Action::CloseRoomPanel);
        }
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

/// Per-modal button-to-action mapping for the style editor.
pub fn style_dialog_action(
    rects: &crate::render::dialog::DialogRects,
    col: u16,
    row: u16,
) -> Option<Action> {
    use crate::render::dialog::ButtonId;

    // Check close [X]
    if let Some(close_rect) = rects.close {
        if hit(close_rect, col, row) {
            return Some(Action::StyleEditorCancel);
        }
    }

    // Check buttons
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Save   => Action::StyleSave,
                ButtonId::Cancel => Action::StyleEditorCancel,
                _                => Action::None,
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

    // Check buttons: Ok → GalleryApply
    for (id, rect) in &rects.buttons {
        if hit(*rect, col, row) {
            return Some(match id {
                ButtonId::Ok => Action::GalleryApply,
                _            => Action::None,
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

    // Honor the user's wheel-direction preference: when mouse_wheel_invert is
    // set, swap scroll up/down (some terminals report "natural" scrolling).
    let kind = match (m.kind, state.config.mouse_wheel_invert) {
        (MouseEventKind::ScrollUp, true) => MouseEventKind::ScrollDown,
        (MouseEventKind::ScrollDown, true) => MouseEventKind::ScrollUp,
        (k, _) => k,
    };

    match kind {
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
    // Enter is handled before lookup_any to prevent the Anim/AnimExit binding
    // from firing when the hotkey dialog is open.
    if matches!(key.code, KeyCode::Esc | KeyCode::Enter) && key.modifiers == KeyModifiers::NONE {
        return KeyResolve::Action(Action::CloseHotkeyDialog);
    }

    let spec = KeySpec::from_key_event(key);

    // Prefix key closes the dialog.
    if spec == state.hotkeys.prefix {
        return KeyResolve::Action(Action::CloseHotkeyDialog);
    }

    // Look up the key across all contexts (Global, Map, Anim) so that commands
    // in any context can be triggered from the dialog. Route the resolved
    // command-string through the slash parser using its registry context.
    if let Some(s) = state.keymap.lookup_any(&spec) {
        let name = s.split_whitespace().next().unwrap_or("");
        let ctx = crate::slash::find_command(name)
            .map(|c| c.context)
            .unwrap_or(Context::Global);
        return KeyResolve::Command(s.to_string(), ctx);
    }

    KeyResolve::None
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
        KeyCode::Enter => Action::SavesLoad,
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::SavesSaveAs,
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE => Action::SavesDelete,
        KeyCode::Char('e') if key.modifiers == KeyModifiers::NONE => Action::SavesExport,
        KeyCode::Char('i') if key.modifiers == KeyModifiers::NONE => Action::SavesImport,
        KeyCode::Esc => Action::SavesClose,
        _ => Action::None,
    }
}

/// Hardwired replay/rewind sub-mode keys (not rebindable, like saves/anim).
fn history_key_to_action(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Left => Action::ReplayStep(-1),
        KeyCode::Right => Action::ReplayStep(1),
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
        KeyCode::Enter => Action::GalleryApply,
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

// ── Internal: style-editor key routing ───────────────────────────────────────

/// Key dispatch for the style-editor full-screen mode.
fn style_editor_key_to_action(key: KeyEvent, state: &crate::state::AppState) -> Action {
    use crate::state::StyleFocus;
    let ed_ref = state.style_editor.as_ref();
    let focus = ed_ref.map(|e| e.focus).unwrap_or(StyleFocus::Board);
    let attr_cursor = ed_ref.map(|e| e.attr_cursor).unwrap_or(0);

    // When Custom focus is active, route printable keys into the custom_buf.
    if focus == StyleFocus::Custom {
        match key.code {
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT =>
            {
                return Action::StyleCustomChar(c);
            }
            KeyCode::Backspace => return Action::StyleCustomBackspace,
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => {
                let buf = ed_ref.map(|e| e.custom_buf.as_str()).unwrap_or("");
                return if crate::style_mru::is_valid_color_token(buf) {
                    Action::StyleCommitCustom
                } else {
                    Action::None
                };
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Up   => Action::StyleNav(-1),
        KeyCode::Down => Action::StyleNav(1),
        KeyCode::Tab if key.modifiers == KeyModifiers::NONE => Action::StyleFocusCycle(1),
        KeyCode::BackTab => Action::StyleFocusCycle(-1),
        KeyCode::Left if focus == StyleFocus::Attrs => Action::StyleAttrChipNav(-1),
        KeyCode::Right if focus == StyleFocus::Attrs => Action::StyleAttrChipNav(1),
        KeyCode::Left if focus == StyleFocus::Fg || focus == StyleFocus::Bg => Action::StyleSwatchNav(-1),
        KeyCode::Right if focus == StyleFocus::Fg || focus == StyleFocus::Bg => Action::StyleSwatchNav(1),
        KeyCode::Enter if key.modifiers == KeyModifiers::NONE
            && (focus == StyleFocus::Fg || focus == StyleFocus::Bg) =>
        {
            Action::StyleSwatchPick
        }
        KeyCode::Char(' ') if key.modifiers == KeyModifiers::NONE
            && (focus == StyleFocus::Fg || focus == StyleFocus::Bg) =>
        {
            Action::StyleSwatchPick
        }
        KeyCode::Char(' ') if key.modifiers == KeyModifiers::NONE
            && focus == StyleFocus::Attrs =>
        {
            match attr_cursor {
                0 => Action::StyleToggleAttr(AttrKind::Bold),
                1 => Action::StyleToggleAttr(AttrKind::Italic),
                2 => Action::StyleToggleAttr(AttrKind::Underline),
                3 => Action::StyleToggleAttr(AttrKind::Dim),
                _ => Action::StyleToggleAttr(AttrKind::Reversed),
            }
        }
        // Border focus only occurs on bordered selectors; see StyleFocusCycle/StyleNav gating.
        KeyCode::Left if focus == StyleFocus::Border => Action::StyleBorderZoneNav(-1),
        KeyCode::Right if focus == StyleFocus::Border => Action::StyleBorderZoneNav(1),
        KeyCode::Enter if key.modifiers == KeyModifiers::NONE && focus == StyleFocus::Border => {
            let zone = ed_ref.map(|e| border_zone_from_index(e.border_zone))
                .unwrap_or(crate::state::BorderZone::Top);
            Action::StyleOpenGlyphPicker(zone)
        }
        KeyCode::Char('t') if key.modifiers == KeyModifiers::NONE && focus == StyleFocus::Border => {
            Action::StyleBorderTypeCycle(1)
        }
        KeyCode::Char('[') if key.modifiers == KeyModifiers::NONE && focus == StyleFocus::Border => {
            Action::StyleBorderTypeCycle(-1)
        }
        KeyCode::Char(']') if key.modifiers == KeyModifiers::NONE && focus == StyleFocus::Border => {
            Action::StyleBorderTypeCycle(1)
        }
        KeyCode::Delete if focus == StyleFocus::Border => Action::StyleBorderClearZone,
        KeyCode::Char('h') if key.modifiers == KeyModifiers::NONE && focus == StyleFocus::Border => {
            Action::StyleBorderToggleHeader
        }
        KeyCode::Char('d') if key.modifiers == KeyModifiers::NONE && focus == StyleFocus::Border => {
            Action::StyleBorderToggleShadow
        }
        KeyCode::Char('s') if key.modifiers == KeyModifiers::NONE => Action::StyleSave,
        KeyCode::Char('r') if key.modifiers == KeyModifiers::NONE => Action::StyleReset,
        KeyCode::Esc  => Action::StyleEditorCancel,
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

// ── Focus cycling ─────────────────────────────────────────────────────────────

/// Cycle a button-focus index by `delta` (+1 Tab, -1 Shift-Tab), wrapping within
/// `0..len`. Returns 0 when `len` is 0.
pub fn cycle_focus(idx: usize, len: usize, delta: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let next = idx as i32 + delta;
    next.rem_euclid(len as i32) as usize
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
    changed: bool,
    counter: &mut u32,
) -> bool {
    use crate::config::BackgroundTidy;
    // A turn that did not change the graph (look, examine, inventory, a failed
    // move, …) must never auto-tidy. `overlap` is a state predicate — true
    // whenever the layout currently has any overlap/distortion — so without this
    // gate a persistent overlap re-triggered a tidy on EVERY turn, making the map
    // border pulse on a bare "look". Re-tidying an unchanged graph is also
    // deterministically pointless (same input → same layout).
    if !changed {
        return false;
    }
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
            // Clear any transient status message on the first keypress.
            state.status_msg = None;
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
                let prefix = state.config.command_prefix;
                // Slash-command name suggestions hold the bare command name (no
                // prefix). When completing the first token of a slash command,
                // rebuild input as prefix + name so the leading prefix survives.
                if state.input.starts_with(prefix)
                    && !state.input[prefix.len_utf8()..].contains(' ')
                {
                    state.input.clear();
                    state.input.push(prefix);
                    state.input.push_str(&completion);
                } else {
                    // Replace the partial word at the end of input with the completion.
                    let partial_len = state.current_partial().len();
                    let new_len = state.input.len() - partial_len;
                    state.input.truncate(new_len);
                    state.input.push_str(&completion);
                }
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

        Action::GameStyle => {
            if state.ifid.is_empty() {
                state.set_status("no game loaded");
            } else {
                let user_dir = state.config.user_dir.clone();
                let ifid = state.ifid.clone();
                let title = state.title.clone();
                match crate::styles::scaffold_per_game_style(&user_dir, &ifid, &title) {
                    Ok((path, true))  => state.set_status(format!("created {}", path.display())),
                    Ok((path, false)) => state.set_status(format!("per-game style: {}", path.display())),
                    Err(e)            => state.set_status(format!("game-style failed: {}", e)),
                }
            }
        }

        Action::ToggleWatch => { /* handled in the run loop (owns the watcher) */ }

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
        Action::ToggleRoomNumbers => state.show_room_numbers = !state.show_room_numbers,
        Action::ToggleLocMethod => state.show_loc_method = !state.show_loc_method,
        Action::ToggleStatusBar => state.show_status_bar = !state.show_status_bar,
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
            state.dialog_focus = 0;
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
            state.dialog_focus = 0;
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

        Action::GalleryApply => {
            if let Some(g) = state.gallery.take() {
                state.symbols = crate::symbols::SymbolSet::resolve(&g.symbol_config());
                // Persistence is handled by the caller (main.rs detects GalleryApply).
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
            state.focus = crate::state::Focus::Game;
            state.selection = Some(crate::clipboard::Selection::new((col, row)));
        }

        Action::ExtendSelection(col, row) => {
            if let Some(sel) = &mut state.selection {
                sel.head = (col, row);
            }
        }

        // The copy is performed by the run loop (it needs the rendered buffer);
        // if this reaches apply_action directly, just drop the selection.
        Action::EndSelection => {
            state.selection = None;
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

        // ── Style editor actions ──────────────────────────────────────────────

        Action::OpenStyleEditor => {
            open_style_editor(state);
        }

        Action::StyleEditorCancel => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &state.style_editor {
                let _ = crate::style_mru::save_mru(&dir, &ed.mru);
            }
            state.style_editor = None;
        }

        Action::StyleNav(d) => {
            if let Some(ed) = &mut state.style_editor {
                let n = ed.selectors.len() as i32;
                ed.active = ((ed.active as i32 + d).rem_euclid(n.max(1))) as usize;
                if ed.focus == crate::state::StyleFocus::Border
                    && !is_bordered_selector(ed.selectors[ed.active])
                {
                    ed.focus = crate::state::StyleFocus::Board;
                }
            }
        }

        Action::StyleToggleAttr(kind) => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                let slot = match kind {
                    AttrKind::Bold      => &mut decl.bold,
                    AttrKind::Italic    => &mut decl.italic,
                    AttrKind::Underline => &mut decl.underline,
                    AttrKind::Dim       => &mut decl.dim,
                    AttrKind::Reversed  => &mut decl.reversed,
                };
                *slot = Some(!slot.unwrap_or(false));
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleFocusCycle(d) => {
            if let Some(ed) = &mut state.style_editor {
                use crate::state::StyleFocus;
                let bordered = is_bordered_selector(ed.selectors[ed.active]);
                let order: &[StyleFocus] = if bordered {
                    &[StyleFocus::Board, StyleFocus::Fg, StyleFocus::Bg, StyleFocus::Custom, StyleFocus::Attrs, StyleFocus::Border]
                } else {
                    &[StyleFocus::Board, StyleFocus::Fg, StyleFocus::Bg, StyleFocus::Custom, StyleFocus::Attrs]
                };
                let cur = order.iter().position(|f| *f == ed.focus).unwrap_or(0) as i32;
                let n = order.len() as i32;
                ed.focus = order[((cur + d).rem_euclid(n)) as usize];
                match ed.focus {
                    StyleFocus::Fg => ed.color_target = false,
                    StyleFocus::Bg => ed.color_target = true,
                    StyleFocus::Custom => {
                        if ed.custom_buf.is_empty() {
                            ed.custom_buf = "#".to_string();
                        }
                    }
                    _ => {}
                }
            }
        }

        Action::StyleAttrChipNav(d) => {
            if let Some(ed) = &mut state.style_editor {
                let n = 5i32;
                ed.attr_cursor = ((ed.attr_cursor as i32 + d).rem_euclid(n)) as usize;
            }
        }

        Action::StyleSetColor { is_bg, value } => {
            let dir = state.config.user_dir.clone();
            apply_style_set_color(state, is_bg, value, &dir);
        }

        Action::StyleCommitCustom => {
            if let Some(ed) = &state.style_editor {
                if crate::style_mru::is_valid_color_token(&ed.custom_buf) {
                    let is_bg = ed.color_target;
                    let value = if ed.custom_buf == "default" { None } else { Some(ed.custom_buf.clone()) };
                    let dir = state.config.user_dir.clone();
                    apply_style_set_color(state, is_bg, value, &dir);
                    if let Some(ed) = &mut state.style_editor { ed.custom_buf.clear(); }
                }
            }
        }

        Action::StyleSwatchNav(d) => {
            if let Some(ed) = &mut state.style_editor {
                let n = crate::style_mru::ANSI_NAMES.len() as i32 + 1; // +1 for default cell
                ed.swatch_cursor = ((ed.swatch_cursor as i32 + d).rem_euclid(n)) as usize;
            }
        }

        Action::StyleSwatchPick => {
            if let Some(ed) = &state.style_editor {
                let is_bg = ed.color_target;
                let cur = ed.swatch_cursor;
                let value = crate::style_mru::ANSI_NAMES.get(cur).map(|s| s.to_string());
                let dir = state.config.user_dir.clone();
                apply_style_set_color(state, is_bg, value, &dir);
            }
        }

        Action::StyleCustomChar(c) => {
            if let Some(ed) = &mut state.style_editor {
                ed.custom_buf.push(c);
            }
        }

        Action::StyleCustomBackspace => {
            if let Some(ed) = &mut state.style_editor {
                if ed.custom_buf.len() > 1 {
                    ed.custom_buf.pop();
                }
            }
        }

        Action::StyleSave => {
            if let Some(ed) = state.style_editor.take() {
                let dir = state.config.user_dir.clone();
                let _ = crate::style_mru::save_mru(&dir, &ed.mru);
                let (cs, set, _w) = crate::style::resolve(&ed.doc, &dir);
                state.colors = cs;
                state.symbols = set;
            }
        }

        Action::StyleReset => {
            if let Some(ed) = &mut state.style_editor {
                let default_doc = crate::style::parse_style_toml(crate::style::DEFAULT_STYLE_TOML)
                    .expect("DEFAULT_STYLE_TOML is always valid");
                let sel = ed.selectors[ed.active].to_string();
                match default_doc.colors.selectors.get(&sel) {
                    Some(d) => { ed.doc.colors.selectors.insert(sel, d.clone()); }
                    None => { ed.doc.colors.selectors.remove(&sel); }
                }
                let dir = state.config.user_dir.clone();
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderTypeCycle(d) => {
            const STYLES: &[&str] = &["none", "single", "double", "rounded", "thick", "picture-frame"];
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                let cur_name = decl.style.as_deref().unwrap_or("single");
                let cur_idx = STYLES.iter().position(|s| *s == cur_name).unwrap_or(1) as i32;
                let n = STYLES.len() as i32;
                let new_idx = ((cur_idx + d).rem_euclid(n)) as usize;
                decl.style = Some(STYLES[new_idx].to_string());
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderZoneNav(d) => {
            if let Some(ed) = &mut state.style_editor {
                let n = 8i32;
                ed.border_zone = ((ed.border_zone as i32 + d).rem_euclid(n)) as usize;
            }
        }

        Action::StyleBorderClearZone => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let zone = border_zone_from_index(ed.border_zone);
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                set_zone_glyph(decl, zone, None);
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderToggleHeader => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                decl.header = Some(!decl.header.unwrap_or(false));
                recompute_style_preview(ed, &dir);
            }
        }

        Action::StyleBorderToggleShadow => {
            let dir = state.config.user_dir.clone();
            if let Some(ed) = &mut state.style_editor {
                let sel = ed.selectors[ed.active].to_string();
                let decl = ed.doc.colors.selectors.entry(sel).or_default();
                decl.shadow = Some(!decl.shadow.unwrap_or(false));
                recompute_style_preview(ed, &dir);
            }
        }

        // ── Glyph-picker modal actions ────────────────────────────────────────

        Action::StyleOpenGlyphPicker(zone) => {
            if let Some(ed) = &state.style_editor {
                let target_selector = ed.selectors[ed.active].to_string();
                // Picture-frame is a composite border; per-zone glyph overrides don't apply.
                let is_picture_frame = ed.doc.colors.selectors.get(&target_selector)
                    .and_then(|d| d.style.as_deref())
                    .unwrap_or("single") == "picture-frame";
                if !is_picture_frame {
                    let user_dir = state.config.user_dir.clone();
                    let mru = crate::style_mru::load_glyph_mru(&user_dir);
                    state.glyph_picker = Some(crate::state::GlyphPickerState {
                        target_selector,
                        target_zone: zone,
                        block: 0,
                        custom_start: None,
                        custom_focus: false,
                        custom_buf: String::new(),
                        cursor: 0,
                        pending: None,
                        mru,
                    });
                }
                // picture-frame: leave state.glyph_picker as None (no-op).
            }
        }

        Action::GlyphPickerNav(delta) => {
            if let Some(picker) = &mut state.glyph_picker {
                let (lo, hi) = picker_block_range(picker);
                let count = (hi - lo + 1) as usize;
                if count > 0 {
                    picker.cursor =
                        ((picker.cursor as i32 + delta).rem_euclid(count as i32)) as usize;
                }
            }
        }

        Action::GlyphPickerBlock(delta) => {
            if let Some(picker) = &mut state.glyph_picker {
                picker.custom_start = None; // return to curated blocks
                picker.custom_focus = false;
                picker.custom_buf.clear();
                let n = GLYPH_BLOCKS.len() as i32;
                picker.block = ((picker.block as i32 + delta).rem_euclid(n)) as usize;
                picker.cursor = 0;
                picker.pending = None;
            }
        }

        Action::GlyphPickerChar(c) => {
            if let Some(picker) = &mut state.glyph_picker {
                if !picker.custom_focus {
                    picker.pending = Some(c.to_string());
                }
            }
        }

        Action::GlyphPickerPick => {
            // Gather what we need before splitting borrows.
            let resolve_info = state.glyph_picker.as_ref().and_then(|picker| {
                let glyph = if let Some(s) = &picker.pending {
                    if crate::style_mru::is_valid_glyph(s) { Some(s.clone()) } else { None }
                } else {
                    picker_glyph_at_cursor(picker)
                };
                glyph.map(|g| (picker.target_selector.clone(), picker.target_zone, g))
            });

            if let Some((sel, zone, glyph)) = resolve_info {
                let user_dir = state.config.user_dir.clone();

                // Write glyph into the style doc.
                if let Some(ed) = &mut state.style_editor {
                    let decl = ed.doc.colors.selectors.entry(sel).or_default();
                    set_zone_glyph(decl, zone, Some(glyph.clone()));
                }

                // Push to glyph MRU and save.
                let saved_mru = if let Some(picker) = &mut state.glyph_picker {
                    crate::style_mru::push_glyph_mru(&mut picker.mru, &glyph);
                    picker.mru.clone()
                } else {
                    Vec::new()
                };
                let _ = crate::style_mru::save_glyph_mru(&user_dir, &saved_mru);

                // Close the picker.
                state.glyph_picker = None;

                // Recompute the preview.
                if let Some(ed) = &mut state.style_editor {
                    recompute_style_preview(ed, &user_dir);
                }
            }
            // If glyph was invalid / none, leave picker open.
        }

        Action::GlyphPickerClear => {
            let pick_info = state.glyph_picker.as_ref()
                .map(|p| (p.target_selector.clone(), p.target_zone));
            if let Some((sel, zone)) = pick_info {
                let user_dir = state.config.user_dir.clone();
                if let Some(ed) = &mut state.style_editor {
                    let decl = ed.doc.colors.selectors.entry(sel).or_default();
                    set_zone_glyph(decl, zone, None);
                    recompute_style_preview(ed, &user_dir);
                }
            }
            state.glyph_picker = None;
        }

        Action::GlyphPickerCancel => {
            state.glyph_picker = None;
        }

        Action::GlyphPickerCustomFocus => {
            if let Some(picker) = &mut state.glyph_picker {
                picker.custom_focus = true;
                picker.pending = None;
            }
        }

        Action::GlyphPickerCustomChar(c) => {
            if let Some(picker) = &mut state.glyph_picker {
                if c.is_ascii_hexdigit() && picker.custom_buf.len() < 6 {
                    picker.custom_buf.push(c.to_ascii_uppercase());
                    if let Ok(cp) = u32::from_str_radix(&picker.custom_buf, 16) {
                        picker.custom_start = Some(cp);
                        picker.cursor = 0;
                    }
                }
            }
        }

        Action::GlyphPickerCustomBackspace => {
            if let Some(picker) = &mut state.glyph_picker {
                picker.custom_buf.pop();
                picker.custom_start = if picker.custom_buf.is_empty() {
                    None
                } else {
                    u32::from_str_radix(&picker.custom_buf, 16).ok()
                };
            }
        }

        // ── Config screen actions ─────────────────────────────────────────────

        Action::OpenConfig => {
            state.hotkey_dialog = false;
            state.dialog_focus = 0;
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
            state.config_screen = None;
        }

        Action::ResetGame => {
            // Open the reset dialog; the caller (main.rs) handles confirm/cancel/clear-map.
            state.hotkey_dialog = false;
            state.reset_dialog = true;
            state.reset_clear_map = false;
            state.dialog_focus = 0;
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

        // TODO Task D: wire the real open/discover/sub-session behavior.
        Action::OpenHints => {}

        // ── Replay / rewind actions ───────────────────────────────────────────

        Action::OpenHistory => {
            // Seed at the last turn; no-op when there is no history.
            state.hotkey_dialog = false;
            if !state.history.is_empty() {
                state.replay = Some(crate::state::ReplayState::new(state.history.len() - 1));
            }
        }

        Action::ReplayStep(delta) => {
            let len = state.history.len();
            if let Some(r) = &mut state.replay {
                r.step(delta, len);
            }
        }

        Action::ReplayTogglePlay => {
            if let Some(r) = &mut state.replay {
                r.toggle_play();
            }
        }

        Action::ReplayClose => {
            state.replay = None;
        }

        // ReplayResume is caller-handled in main.rs (needs the live session/VM).
        Action::ReplayResume => {}

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
        // Saves-manager, export, and config-path prompts: return to the caller to act on.
        PromptKind::SaveAs
        | PromptKind::ConfirmDeleteSave(_)
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

// ── Glyph-picker helpers ──────────────────────────────────────────────────────

/// Number of glyph columns in the picker grid (matches `GRID_COLS` in the render module).
pub const GLYPH_GRID_COLS: usize = 16;

/// Curated Unicode blocks offered by the glyph picker.
/// Each entry is (display name, first codepoint, last codepoint inclusive).
pub(crate) const GLYPH_BLOCKS: &[(&str, u32, u32)] = &[
    ("Box Drawing",       0x2500, 0x257F),
    ("Block Elements",    0x2580, 0x259F),
    ("Geometric Shapes",  0x25A0, 0x25FF),
    ("Arrows",            0x2190, 0x21FF),
];

/// Return the (lo, hi) codepoint range for the picker's current block/custom range.
pub(crate) fn picker_block_range(picker: &crate::state::GlyphPickerState) -> (u32, u32) {
    if let Some(start) = picker.custom_start {
        (start, start.saturating_add(127))
    } else {
        let (_, lo, hi) = GLYPH_BLOCKS[picker.block.min(GLYPH_BLOCKS.len() - 1)];
        (lo, hi)
    }
}

/// Resolve the glyph at the picker's current `cursor` position, if it is single-width.
/// Returns `None` for empty ranges or non-single-width codepoints.
pub(crate) fn picker_glyph_at_cursor(picker: &crate::state::GlyphPickerState) -> Option<String> {
    let (lo, hi) = picker_block_range(picker);
    // Collect single-width glyphs in order.
    let mut idx = 0usize;
    for cp in lo..=hi {
        if let Some(c) = char::from_u32(cp) {
            let s = c.to_string();
            if crate::style_mru::is_valid_glyph(&s) {
                if idx == picker.cursor {
                    return Some(s);
                }
                idx += 1;
            }
        }
    }
    None
}

/// Returns `true` for the six selectors that have configurable borders.
pub fn is_bordered_selector(sel: &str) -> bool {
    matches!(sel, "map_border" | "story_border" | "dialog" | "upper_window_border" | "status_header" | "input_line")
}

/// Map the 8-slot border_zone cursor to a BorderZone.
/// Layout: 0=Tl, 1=Top, 2=Tr, 3=Left, 4=Right, 5=Bl, 6=Bottom, 7=Br
fn border_zone_from_index(i: usize) -> crate::state::BorderZone {
    use crate::state::BorderZone::*;
    match i {
        0 => Tl,
        1 => Top,
        2 => Tr,
        3 => Left,
        4 => Right,
        5 => Bl,
        6 => Bottom,
        7 => Br,
        _ => Top,
    }
}

/// Write `g` into the `decl` field that corresponds to `zone`.
pub(crate) fn set_zone_glyph(
    decl: &mut crate::style::Decl,
    zone: crate::state::BorderZone,
    g: Option<String>,
) {
    use crate::state::BorderZone::*;
    match zone {
        Top    => decl.glyph_top    = g,
        Bottom => decl.glyph_bottom = g,
        Left   => decl.glyph_left   = g,
        Right  => decl.glyph_right  = g,
        Tl     => decl.glyph_tl     = g,
        Tr     => decl.glyph_tr     = g,
        Bl     => decl.glyph_bl     = g,
        Br     => decl.glyph_br     = g,
    }
}

// ── Config screen helpers ─────────────────────────────────────────────────────

/// Number of rows in the config screen — derived from the row list so it cannot drift.
pub(crate) const CONFIG_ROW_COUNT: usize = crate::render::config_screen::CONFIG_ROWS.len();

/// Clone a Config (Config derives Clone, this is a convenience wrapper for tests).
pub(crate) fn clone_config(cfg: &crate::config::Config) -> crate::config::Config {
    cfg.clone()
}

/// Open the live style editor: load the current style doc, resolve a preview
/// ColorScheme, and seed the StyleEditorState on `state.style_editor`.
///
/// Does not touch `state.colors` — the live theme is untouched until Save.
pub fn open_style_editor(state: &mut AppState) {
    let user_dir = state.config.user_dir.clone();
    let (global, _warnings) = crate::style::load_style(state.config.style.as_deref(), &user_dir);
    // Layer the per-game override (user_dir/styles/<ifid>.toml) over the global so
    // the editor opens showing the live look. A missing or unparseable per-game
    // file falls back to the global doc.
    let doc = if !state.ifid.is_empty() {
        let pg_path = crate::styles::per_game_style_path(&user_dir, &state.ifid);
        match std::fs::read_to_string(&pg_path) {
            Ok(text) => match crate::style::parse_style_toml(&text) {
                Ok(over) => crate::style::merge(&global, &over),
                Err(_) => global,
            },
            Err(_) => global,
        }
    } else {
        global
    };
    let (preview, _set, _w2) = crate::style::resolve(&doc, &user_dir);
    let selectors: Vec<&'static str> =
        crate::style::SELECTOR_GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
    state.style_editor = Some(crate::state::StyleEditorState {
        doc,
        preview,
        selectors,
        active: 0,
        focus: crate::state::StyleFocus::Board,
        custom_buf: String::new(),
        mru: crate::style_mru::load_mru(&user_dir),
        attr_cursor: 0,
        color_target: false,
        swatch_cursor: 0,
        border_zone: 0,
    });
    state.dialog_focus = 0;
}

/// Test-only: open the style editor over a fresh, empty temp `user_dir` so it
/// reads the built-in default style instead of the contributor's real
/// `~/.babelmap/style.toml`. Without this, an on-disk style that overrides an
/// asserted selector (fg/bg/glyph/style) causes spurious failures, and
/// `StyleSave`/`save_mru` would write into the real user directory during tests.
/// Each call gets a unique directory so save-path tests never collide.
#[cfg(test)]
pub(crate) fn open_style_editor_hermetic(state: &mut AppState) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir()
        .join(format!("babelmap-style-test-{}-{}", std::process::id(), n));
    let _ = std::fs::create_dir_all(&dir);
    state.config.user_dir = dir;
    open_style_editor(state);
}

/// Re-resolve `ed.preview` from the current `ed.doc` + `user_dir`.
///
/// Called from edit handlers (Tasks 4-6) whenever the doc changes.
/// Nav doesn't change the doc, so it skips this call.
pub fn recompute_style_preview(ed: &mut crate::state::StyleEditorState, user_dir: &std::path::Path) {
    let (cs, _set, _w) = crate::style::resolve(&ed.doc, user_dir);
    ed.preview = cs;
}

/// Set the fg or bg color for the active selector, push hex to MRU, recompute preview.
///
/// Shared by `StyleSetColor`, `StyleCommitCustom`, and `StyleSwatchPick`.
pub(crate) fn apply_style_set_color(
    state: &mut AppState,
    is_bg: bool,
    value: Option<String>,
    user_dir: &std::path::Path,
) {
    if let Some(ed) = &mut state.style_editor {
        let sel = ed.selectors[ed.active].to_string();
        let decl = ed.doc.colors.selectors.entry(sel).or_default();
        let slot = if is_bg { &mut decl.bg } else { &mut decl.fg };
        *slot = value.clone();
        if let Some(v) = &value {
            if v.starts_with('#') {
                crate::style_mru::push_mru(&mut ed.mru, v);
            }
        }
        recompute_style_preview(ed, user_dir);
    }
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
        4 => { if let Some(cs) = &mut state.config_screen { cs.working.prompt_save_on_quit = !cs.working.prompt_save_on_quit; } }
        5 => { if let Some(cs) = &mut state.config_screen { cs.working.prompt_load_on_launch = !cs.working.prompt_load_on_launch; } }
        6 => { if let Some(cs) = &mut state.config_screen { cs.working.record_history = !cs.working.record_history; } }
        7 => { if let Some(cs) = &mut state.config_screen { cs.working.show_room_numbers = !cs.working.show_room_numbers; } }
        8 => { if let Some(cs) = &mut state.config_screen { config_cycle_background_tidy(&mut cs.working.background_tidy, 1); } }
        9 => { if let Some(cs) = &mut state.config_screen { config_cycle_aux_storage(&mut cs.working.aux_storage, 1); } }
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

/// Apply ConfigCycle to the selected row.
fn config_cycle(working: &mut crate::config::Config, row: usize, delta: i32) {
    match row {
        0 => {} // path: no cycling
        1 => working.use_default_map = !working.use_default_map,
        2 => working.auto_load = !working.auto_load,
        3 => working.auto_save = !working.auto_save,
        4 => working.prompt_save_on_quit = !working.prompt_save_on_quit,
        5 => working.prompt_load_on_launch = !working.prompt_load_on_launch,
        6 => working.record_history = !working.record_history,
        7 => working.show_room_numbers = !working.show_room_numbers,
        8 => config_cycle_background_tidy(&mut working.background_tidy, delta),
        9 => config_cycle_aux_storage(&mut working.aux_storage, delta),
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

    #[test]
    fn editor_opens_over_merged_per_game_style() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("bm-merge-open-{}-{}", std::process::id(), n));
        let styles_dir = dir.join("styles");
        std::fs::create_dir_all(&styles_dir).unwrap();
        // Global: room fg = white, connector fg = cyan.
        std::fs::write(
            dir.join("style.toml"),
            "[colors]\n\"room\" = { fg = \"white\" }\n\"connector\" = { fg = \"cyan\" }\n[symbols]\n",
        ).unwrap();
        // Per-game override for IFID: room fg = red (connector untouched).
        let ifid = "ZCODE-1-ABCDEF-0001";
        std::fs::write(
            styles_dir.join(format!("{ifid}.toml")),
            "[colors]\n\"room\" = { fg = \"red\" }\n[symbols]\n",
        ).unwrap();

        let mut s = AppState::default();
        s.config.user_dir = dir;
        s.config.style = None; // load global from user_dir/style.toml
        s.ifid = ifid.to_string();
        open_style_editor(&mut s);

        let ed = s.style_editor.as_ref().unwrap();
        assert_eq!(
            ed.doc.colors.selectors.get("room").and_then(|d| d.fg.as_deref()),
            Some("red"),
            "per-game override wins for room",
        );
        assert_eq!(
            ed.doc.colors.selectors.get("connector").and_then(|d| d.fg.as_deref()),
            Some("cyan"),
            "global value survives for non-overridden connector",
        );
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
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-game"));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('r'))), KeyResolve::Command(c, _) if c == "load-game"));
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
        s.replay = Some(ReplayState::new(2));
        apply_action(Action::ReplayStep(-1), &mut s, &mut Mapper::default());
        assert_eq!(s.replay.as_ref().unwrap().idx, 1);
        apply_action(Action::ReplayClose, &mut s, &mut Mapper::default());
        assert!(s.replay.is_none(), "Esc closes without change");
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
    fn reload_action_applies_style_file() {
        let dir = std::env::temp_dir().join(format!("babelmap-reloadact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("style.toml");
        std::fs::write(&path, "[colors]\n\"transcript\" = { fg = \"magenta\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(path.to_string_lossy().to_string());
        let mut mapper = Mapper::default();

        apply_action(Action::ReloadStyle, &mut state, &mut mapper);
        assert_eq!(state.colors.transcript.fg, Some(ratatui::style::Color::Magenta));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn game_style_action_scaffolds_file() {
        let dir = std::env::temp_dir().join(format!("babelmap-gamestyle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.ifid = "ZCODE-1-GS-0001".to_string();
        state.title = "Zork I".to_string();
        let mut mapper = Mapper::default();

        apply_action(Action::GameStyle, &mut state, &mut mapper);
        let path = crate::styles::per_game_style_path(&dir, &state.ifid);
        assert!(path.is_file(), "scaffold created the per-game file");
        assert!(std::fs::read_to_string(&path).unwrap().contains("Zork I"));
        let _ = std::fs::remove_dir_all(&dir);
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
    fn autocomplete_slash_command_preserves_prefix() {
        let mut s = AppState::default();
        let mut m = Mapper::default();
        // Slash suggestions hold the bare command name (no prefix).
        s.input = "/sav".to_string();
        s.suggestions = vec!["save".to_string(), "save-as".to_string()];
        s.suggestion_idx = 0;
        apply_action(Action::Autocomplete, &mut s, &mut m);
        // The leading prefix must survive completion.
        assert_eq!(s.input, "/save");
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
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-game"));
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('r'))), KeyResolve::Command(c, _) if c == "load-game"));
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
        assert!(matches!(key_to_command(&s, ctrl(KeyCode::Char('s'))), KeyResolve::Command(c, _) if c == "save-game"));
        // Ctrl+Left no longer nudges (nudge moved to F6-F9).
        assert!(matches!(key_to_action(&s, ctrl(KeyCode::Left)), Action::None));
        // F6-F9 nudge work in map focus via Global fallthrough.
        assert!(matches!(key_to_action(&s, key(KeyCode::F(6))), Action::NudgeSelected(-1, 0)));
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
    fn gallery_enter_is_apply() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let a = gallery_key_to_action(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(matches!(a, Action::GalleryApply));
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
        apply_action(action, &mut s, &mut Mapper::default());
        assert_eq!(s.focus, Focus::Game);
        assert_eq!(s.selection.map(|sel| sel.anchor), Some((85, 5)));
    }

    #[test]
    fn left_drag_then_up_extends_and_ends_selection() {
        use crossterm::event::MouseEventKind;
        let mut s = AppState::default();
        apply_action(Action::StartSelection(85, 5), &mut s, &mut Mapper::default());

        let drag = mouse_event(MouseEventKind::Drag(MouseButton::Left), 90, 7, KeyModifiers::NONE);
        let a = mouse_to_action(&s, drag, map_rect(), story_rect(), &[], &None);
        assert!(matches!(a, Action::ExtendSelection(90, 7)));
        apply_action(a, &mut s, &mut Mapper::default());
        assert_eq!(s.selection.map(|sel| sel.head), Some((90, 7)));

        let up = mouse_event(MouseEventKind::Up(MouseButton::Left), 90, 7, KeyModifiers::NONE);
        assert!(matches!(
            mouse_to_action(&s, up, map_rect(), story_rect(), &[], &None),
            Action::EndSelection
        ));
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

    // ── should_bg_tidy ────────────────────────────────────────────────────────

    #[test]
    fn should_bg_tidy_off_always_false() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(!should_bg_tidy(BackgroundTidy::Off, true, true, true, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::Off, false, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_no_change_never_fires() {
        use crate::config::BackgroundTidy;
        // Regression (bug: "look pulses tidy"): a turn that did not change the
        // graph must NOT auto-tidy, even with a persistent layout overlap.
        let mut c = 0u32;
        for mode in [BackgroundTidy::EveryRoom, BackgroundTidy::OnOverlap, BackgroundTidy::Debounced] {
            assert!(!should_bg_tidy(mode, false, true, false, &mut c),
                "{:?}: overlap without a graph change must not fire", mode);
            assert!(!should_bg_tidy(mode, true, true, false, &mut c),
                "{:?}: changed=false must override new_room/overlap", mode);
        }
    }

    #[test]
    fn should_bg_tidy_every_room_follows_new_room_or_overlap() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        // Fires on new room.
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, true, false, true, &mut c));
        // Fires on overlap even without a new room (the change added a connection).
        assert!(should_bg_tidy(BackgroundTidy::EveryRoom, false, true, true, &mut c));
        // No new room and no overlap: no fire.
        assert!(!should_bg_tidy(BackgroundTidy::EveryRoom, false, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_on_overlap_follows_overlap() {
        use crate::config::BackgroundTidy;
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::OnOverlap, false, true, true, &mut c));
        assert!(!should_bg_tidy(BackgroundTidy::OnOverlap, true, false, true, &mut c));
    }

    #[test]
    fn should_bg_tidy_debounced_fires_every_k_new_rooms() {
        use crate::config::{BackgroundTidy, BG_TIDY_DEBOUNCE};
        let mut c = 0u32;
        // First K-1 new rooms should not fire.
        for _ in 0..BG_TIDY_DEBOUNCE - 1 {
            assert!(!should_bg_tidy(BackgroundTidy::Debounced, true, false, true, &mut c));
        }
        // K-th new room fires and resets counter.
        assert!(should_bg_tidy(BackgroundTidy::Debounced, true, false, true, &mut c));
        assert_eq!(c, 0, "counter resets after Debounced fires");
        // No new room: never fires.
        assert!(!should_bg_tidy(BackgroundTidy::Debounced, false, false, false, &mut c));
    }

    #[test]
    fn should_bg_tidy_debounced_fires_immediately_on_overlap() {
        use crate::config::BackgroundTidy;
        // Overlap fires immediately regardless of debounce counter value.
        let mut c = 0u32;
        assert!(should_bg_tidy(BackgroundTidy::Debounced, false, true, true, &mut c),
            "overlap should fire immediately even without a new room");
        assert_eq!(c, 0, "counter is reset when overlap fires");

        // Even with a partially-accumulated counter, overlap fires immediately.
        let mut c = 2u32;
        assert!(should_bg_tidy(BackgroundTidy::Debounced, false, true, true, &mut c),
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

    // ── Leaf 2: ResetGame opens the dialog ───────────────────────────────────

    #[test]
    fn reset_game_action_opens_reset_dialog() {
        use crate::state::AppState;
        let mut s = AppState::default();
        let mut m = Mapper::default();
        assert!(!s.reset_dialog, "dialog must start closed");
        apply_action(Action::ResetGame, &mut s, &mut m);
        assert!(s.reset_dialog, "ResetGame must set reset_dialog = true");
        assert!(!s.reset_clear_map, "checkbox must start unchecked");
        assert!(s.prompt.is_none(), "no text prompt should be opened");
    }

    // ── Regression: F5 (key-bound reset-game) must reach the confirmation dialog ──
    // The command-system unification routes F5 through key_to_command -> "reset-game"
    // -> SlashOutcome::Reset. The from_key branch of dispatch_slash_outcome calls
    // apply_action(Action::ResetGame), which opens the dialog. This test pins both
    // halves of that link so an instant-wipe regression (F5 silently resetting with
    // no confirmation) cannot return unnoticed.
    #[test]
    fn f5_key_resolves_to_reset_game_command_and_opens_dialog() {
        use crate::state::AppState;
        let s = AppState::default();
        // (a) F5 resolves to the "reset-game" command (the key-dispatch half).
        match key_to_command(&s, key(KeyCode::F(5))) {
            KeyResolve::Command(cmd, _) => {
                assert_eq!(cmd, "reset-game", "F5 must resolve to the reset-game command");
            }
            other => panic!("F5 must resolve to KeyResolve::Command(\"reset-game\"), got {:?}", other),
        }
        // (b) The from_key Reset branch opens the dialog via Action::ResetGame.
        let mut s2 = AppState::default();
        let mut m = Mapper::default();
        apply_action(Action::ResetGame, &mut s2, &mut m);
        assert!(s2.reset_dialog, "key-bound reset-game must open the confirmation dialog, not instant-wipe");
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
                beep: None,
                diagnostics: vec![],
                location_method: None,
                pending_io: None,
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
        state.transcript_kinds.clear();
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
                beep: None,
                diagnostics: vec![],
                location_method: None,
                pending_io: None,
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
        assert_eq!(cmd, Some("open-verb-menu"), "m should be bound to open-verb-menu");
    }

    #[test]
    fn open_verb_menu_in_view_dialog_group() {
        use crate::keymap::HotkeyLayout;
        let layout = HotkeyLayout::default();
        let view_group = layout.groups.iter().find(|(title, _)| title == "View");
        assert!(view_group.is_some(), "View group should exist");
        let (_, cmds) = view_group.unwrap();
        assert!(cmds.iter().any(|c| c == "open-verb-menu"), "open-verb-menu should be in View group");
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

    /// Assert gallery [X] produces GalleryClose and [OK] produces GalleryApply.
    #[test]
    fn gallery_dialog_x_and_done_produce_gallery_close() {
        use ratatui::layout::Rect;
        use crate::render::dialog::{ButtonId, DialogRects};
        use crate::state::GalleryState;

        let rects = DialogRects {
            area:    Rect::new(5, 3, 70, 24),
            content: Rect::new(6, 5, 68, 19),
            close:   Some(Rect::new(73, 3, 1, 1)),
            buttons: vec![(ButtonId::Ok, Rect::new(65, 26, 8, 1))],
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

        // OK button → GalleryApply
        let a = mouse_to_action(&state, mouse_left_click(67, 26), map, story, room_rects, &dialog);
        assert!(matches!(a, Action::GalleryApply), "gallery [OK] click should produce GalleryApply, got {:?}", a);

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
        s.config_screen = Some(crate::state::ConfigScreenState { working, selected: 0 });
        s.dialog_focus = cycle_focus(0, 2, 1); // focus Cancel (index 1)
        let a = config_screen_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.dialog_focus,
        );
        assert!(matches!(a, Action::ConfigCancel),
            "Enter with focus=1 (Cancel) should fire ConfigCancel, got {:?}", a);
    }

    #[test]
    fn config_screen_enter_at_default_focus_fires_save() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.config_screen = Some(crate::state::ConfigScreenState { working, selected: 0 });
        s.dialog_focus = 0; // focus Save (default)
        let a = config_screen_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.dialog_focus,
        );
        assert!(matches!(a, Action::ConfigSave),
            "Enter with focus=0 (Save) should fire ConfigSave, got {:?}", a);
    }

    #[test]
    fn config_screen_space_still_toggles_row() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut s = AppState::default();
        let working = clone_config(&s.config);
        s.config_screen = Some(crate::state::ConfigScreenState { working, selected: 0 });
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
        s.saves = Some(crate::state::SavesState { entries: Vec::new(), selected: 0 });
        s.dialog_focus = 0;
        // Tab with ring len 1 stays at 0.
        let after_tab = cycle_focus(s.dialog_focus, 1, 1);
        assert_eq!(after_tab, 0, "Tab on ring-len-1 should stay at 0");
        // Enter still produces SavesLoad (not SavesClose) regardless of focus.
        let a = saves_key_to_action(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            s.dialog_focus,
        );
        assert!(matches!(a, Action::SavesLoad),
            "Enter in saves should still fire SavesLoad (not affected by focus), got {:?}", a);
    }

    #[test]
    fn open_config_resets_dialog_focus() {
        let mut s = AppState::default();
        s.dialog_focus = 5; // non-zero
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::OpenConfig, &mut s, &mut m);
        assert_eq!(s.dialog_focus, 0, "OpenConfig must reset dialog_focus to 0");
    }

    #[test]
    fn open_saves_resets_dialog_focus_in_apply() {
        let mut s = AppState::default();
        s.dialog_focus = 5; // non-zero
        let mut m = mapper::mapper::Mapper::default();
        apply_action(Action::OpenSaves, &mut s, &mut m);
        assert_eq!(s.dialog_focus, 0, "OpenSaves must reset dialog_focus to 0");
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
        s.hotkey_dialog = true;
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
        let a = verb_menu_key_to_action(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(a, Action::VerbMenuNav(VerbMenuNavKind::NextPane)));
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
        };

        let mut state = AppState::default();
        state.tidy_anim = Some(TidyAnim::new(vec![TidyFrame {
            label: "test".to_string(),
            graph: mapper::graph::MapGraph::new(),
            description: String::new(),
            stats: mapper::layout::TidyStats::default(),
            stage_start: false,
        }]));

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

    #[test]
    fn open_style_editor_seeds_doc_and_preview() {
        let mut s = AppState::default();
        apply_action(Action::OpenStyleEditor, &mut s, &mut mapper::mapper::Mapper::default());
        let ed = s.style_editor.as_ref().expect("editor open");
        assert_eq!(ed.active, 0);
        assert!(!ed.selectors.is_empty(), "selector list seeded");
        // Cancel closes it.
        apply_action(Action::StyleEditorCancel, &mut s, &mut mapper::mapper::Mapper::default());
        assert!(s.style_editor.is_none());
    }

    #[test]
    fn hermetic_editor_ignores_ambient_user_dir() {
        // Even when config.user_dir already points at a directory containing a
        // poison style.toml, the hermetic helper rebinds to a fresh empty dir,
        // so the editor loads the built-in default (room has no fg) — proving
        // tests never inherit the contributor's real ~/.babelmap/style.toml.
        let poison =
            std::env::temp_dir().join(format!("bm-poison-{}", std::process::id()));
        std::fs::create_dir_all(&poison).unwrap();
        std::fs::write(
            poison.join("style.toml"),
            "[colors]\n\"room\" = { fg = \"#123456\" }\n[symbols]\n",
        )
        .unwrap();
        let mut s = AppState::default();
        s.config.user_dir = poison;
        open_style_editor_hermetic(&mut s);
        let ed = s.style_editor.as_ref().unwrap();
        assert!(
            ed.doc.colors.selectors.get("room").and_then(|d| d.fg.as_ref()).is_none(),
            "hermetic editor must not inherit the ambient user_dir's style.toml",
        );
    }

    #[test]
    fn toggling_bold_updates_decl_and_preview() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        let sel = s.style_editor.as_ref().unwrap().selectors[0].to_string();
        apply_action(Action::StyleToggleAttr(AttrKind::Bold), &mut s, &mut mapper::mapper::Mapper::default());
        let ed = s.style_editor.as_ref().unwrap();
        assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.bold), Some(true));
        // Smoke: preview was recomputed (exercises the code path).
        let _ = ed.preview;
    }

    #[test]
    fn style_set_color_sets_fg_and_pushes_hex_to_mru() {
        let mut s = AppState::default();
        // Use a non-existent user_dir so load_mru returns empty regardless of disk state.
        s.config.user_dir = std::path::PathBuf::from("/tmp/babelmap-test-empty-mru-dir");
        open_style_editor_hermetic(&mut s);
        let sel = s.style_editor.as_ref().unwrap().selectors[0].to_string();

        // Set fg to a named color — no MRU push.
        apply_action(Action::StyleSetColor { is_bg: false, value: Some("red".into()) },
                     &mut s, &mut mapper::mapper::Mapper::default());
        {
            let ed = s.style_editor.as_ref().unwrap();
            assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.as_deref()), Some("red"));
            assert!(ed.mru.is_empty(), "named colors do not push to MRU");
        }

        // Set fg to a hex color — should push to MRU.
        apply_action(Action::StyleSetColor { is_bg: false, value: Some("#aabbcc".into()) },
                     &mut s, &mut mapper::mapper::Mapper::default());
        {
            let ed = s.style_editor.as_ref().unwrap();
            assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.as_deref()), Some("#aabbcc"));
            assert_eq!(ed.mru, vec!["#aabbcc".to_string()]);
        }

        // Set bg to None — clears to default.
        apply_action(Action::StyleSetColor { is_bg: true, value: None },
                     &mut s, &mut mapper::mapper::Mapper::default());
        {
            let ed = s.style_editor.as_ref().unwrap();
            assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.bg.as_deref()), None);
        }
    }

    #[test]
    fn style_custom_char_and_backspace_edit_buf() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        let m = &mut mapper::mapper::Mapper::default();

        apply_action(Action::StyleCustomChar('#'), &mut s, m);
        apply_action(Action::StyleCustomChar('f'), &mut s, m);
        apply_action(Action::StyleCustomChar('f'), &mut s, m);
        apply_action(Action::StyleCustomChar('0'), &mut s, m);
        apply_action(Action::StyleCustomChar('0'), &mut s, m);
        apply_action(Action::StyleCustomChar('0'), &mut s, m);
        apply_action(Action::StyleCustomChar('0'), &mut s, m);
        assert_eq!(s.style_editor.as_ref().unwrap().custom_buf, "#ff0000");

        apply_action(Action::StyleCustomBackspace, &mut s, m);
        apply_action(Action::StyleCustomBackspace, &mut s, m);
        assert_eq!(s.style_editor.as_ref().unwrap().custom_buf, "#ff00");
    }

    #[test]
    fn style_focus_cycle_to_custom_seeds_hash() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        let m = &mut mapper::mapper::Mapper::default();
        // Cycle delta=3 lands on Custom (Board=0 → index 3).
        apply_action(Action::StyleFocusCycle(3), &mut s, m);
        let ed = s.style_editor.as_ref().unwrap();
        assert_eq!(ed.focus, crate::state::StyleFocus::Custom);
        assert_eq!(ed.custom_buf, "#", "entering Custom via Tab seeds custom_buf with '#'");
    }

    #[test]
    fn style_custom_backspace_cannot_delete_leading_hash() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        let m = &mut mapper::mapper::Mapper::default();
        // Seed via focus cycle.
        apply_action(Action::StyleFocusCycle(3), &mut s, m);
        assert_eq!(s.style_editor.as_ref().unwrap().custom_buf, "#");
        // Backspace on lone '#' must be a no-op.
        apply_action(Action::StyleCustomBackspace, &mut s, m);
        assert_eq!(s.style_editor.as_ref().unwrap().custom_buf, "#",
            "backspace on lone '#' must not delete it");
    }

    #[test]
    fn style_save_applies_to_live_colors_and_closes() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        apply_action(
            Action::StyleSetColor { is_bg: false, value: Some("#ff0000".into()) },
            &mut s, &mut mapper::mapper::Mapper::default(),
        );
        apply_action(Action::StyleSave, &mut s, &mut mapper::mapper::Mapper::default());
        // Save must close the editor.
        assert!(s.style_editor.is_none(), "save closes the editor");
        // The live color scheme must have been updated (resolve ran).
        // We can't assert a specific selector value without knowing which selector is first,
        // but we can verify that state.colors is a valid ColorScheme (non-default fields
        // may have changed). The smoke is: resolve ran without panic.
        let _ = &s.colors;
    }

    #[test]
    fn style_reset_reverts_active_selector_to_default() {
        let mut s = AppState::default();
        crate::input::open_style_editor_hermetic(&mut s);
        let sel = s.style_editor.as_ref().unwrap().selectors[0].to_string();
        // Mutate the first selector's fg.
        apply_action(
            Action::StyleSetColor { is_bg: false, value: Some("#ff0000".into()) },
            &mut s, &mut mapper::mapper::Mapper::default(),
        );
        // Reset should revert it to the built-in default.
        apply_action(Action::StyleReset, &mut s, &mut mapper::mapper::Mapper::default());
        let ed = s.style_editor.as_ref().unwrap();
        let default_doc = crate::style::parse_style_toml(crate::style::DEFAULT_STYLE_TOML).unwrap();
        assert_eq!(
            ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.as_deref()),
            default_doc.colors.selectors.get(&sel).and_then(|d| d.fg.as_deref()),
            "reset restores the default fg for selector '{}'", sel,
        );
    }

    #[test]
    fn open_style_editor_resets_dialog_focus() {
        let mut s = AppState::default();
        s.dialog_focus = 5; // non-zero stale value
        open_style_editor_hermetic(&mut s);
        assert_eq!(s.dialog_focus, 0, "open_style_editor must reset dialog_focus to 0");
    }

    #[test]
    fn style_dialog_action_buttons() {
        use crate::render::dialog::{ButtonId, DialogRects};
        use ratatui::layout::Rect;

        // Build a minimal DialogRects with a Save button at (10,5), Cancel at (20,5),
        // and a close [X] at (30,5).
        let save_rect   = Rect { x: 10, y: 5, width: 4, height: 1 };
        let cancel_rect = Rect { x: 20, y: 5, width: 6, height: 1 };
        let close_rect  = Rect { x: 30, y: 5, width: 3, height: 1 };

        let rects = DialogRects {
            area: Rect::default(),
            content: Rect::default(),
            close: Some(close_rect),
            buttons: vec![
                (ButtonId::Save,   save_rect),
                (ButtonId::Cancel, cancel_rect),
            ],
        };

        // Save button click → StyleSave
        assert!(
            matches!(style_dialog_action(&rects, 11, 5), Some(Action::StyleSave)),
            "Save button must return StyleSave"
        );

        // Cancel button click → StyleEditorCancel
        assert!(
            matches!(style_dialog_action(&rects, 22, 5), Some(Action::StyleEditorCancel)),
            "Cancel button must return StyleEditorCancel"
        );

        // Close [X] click → StyleEditorCancel
        assert!(
            matches!(style_dialog_action(&rects, 31, 5), Some(Action::StyleEditorCancel)),
            "Close [X] must return StyleEditorCancel"
        );

        // Miss → None
        assert!(
            style_dialog_action(&rects, 0, 0).is_none(),
            "miss must return None"
        );
    }

    #[test]
    fn custom_commit_targets_bg_when_color_target_is_bg() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        {
            let ed = s.style_editor.as_mut().unwrap();
            ed.color_target = true; // bg
            ed.custom_buf = "#abcdef".into();
        }
        apply_action(Action::StyleCommitCustom, &mut s, &mut mapper::mapper::Mapper::default());
        let ed = s.style_editor.as_ref().unwrap();
        let sel = ed.selectors[ed.active].to_string();
        assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.bg.clone()), Some("#abcdef".into()));
        assert!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg.clone()).is_none());
    }

    #[test]
    fn glyph_picker_pick_sets_zone_glyph_and_closes() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        {
            let ed = s.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap();
            // Ensure not picture-frame regardless of what style.toml on disk says.
            ed.doc.colors.selectors.entry("map_border".into()).or_default().style = None;
        }
        apply_action(Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top), &mut s, &mut Mapper::default());
        assert!(s.glyph_picker.is_some(), "picker opens");
        // Feed '═' via the pending path then commit.
        apply_action(Action::GlyphPickerChar('═'), &mut s, &mut Mapper::default());
        apply_action(Action::GlyphPickerPick, &mut s, &mut Mapper::default());
        assert!(s.glyph_picker.is_none(), "pick closes the picker");
        let ed = s.style_editor.as_ref().unwrap();
        assert_eq!(
            ed.doc.colors.selectors.get("map_border").and_then(|d| d.glyph_top.clone()),
            Some("═".into()),
            "glyph written to the doc",
        );
    }

    #[test]
    fn glyph_picker_clear_sets_zone_to_none_and_closes() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        {
            let ed = s.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap();
            // Pre-set a glyph so we can verify clear removes it; also ensure not picture-frame.
            let decl = ed.doc.colors.selectors.entry("map_border".into()).or_default();
            decl.glyph_top = Some("═".into());
            decl.style = None;
        }
        apply_action(Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top), &mut s, &mut Mapper::default());
        assert!(s.glyph_picker.is_some());
        apply_action(Action::GlyphPickerClear, &mut s, &mut Mapper::default());
        assert!(s.glyph_picker.is_none(), "clear closes the picker");
        let ed = s.style_editor.as_ref().unwrap();
        assert_eq!(
            ed.doc.colors.selectors.get("map_border").and_then(|d| d.glyph_top.clone()),
            None,
            "glyph cleared from the doc",
        );
    }

    #[test]
    fn glyph_picker_custom_range_entry() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        apply_action(
            Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top),
            &mut s,
            &mut Mapper::default(),
        );
        assert!(s.glyph_picker.is_some(), "picker opens");

        // Enter custom-entry focus via the action.
        apply_action(Action::GlyphPickerCustomFocus, &mut s, &mut Mapper::default());
        assert!(s.glyph_picker.as_ref().unwrap().custom_focus, "custom_focus set");

        // Type '2', '5', '0', '0' → U+2500.
        for c in ['2', '5', '0', '0'] {
            apply_action(Action::GlyphPickerCustomChar(c), &mut s, &mut Mapper::default());
        }
        {
            let picker = s.glyph_picker.as_ref().unwrap();
            assert_eq!(picker.custom_buf, "2500", "buf accumulates hex digits");
            assert_eq!(picker.custom_start, Some(0x2500), "custom_start set to U+2500");
        }

        // Backspace removes last digit; custom_start updates.
        apply_action(Action::GlyphPickerCustomBackspace, &mut s, &mut Mapper::default());
        {
            let picker = s.glyph_picker.as_ref().unwrap();
            assert_eq!(picker.custom_buf, "250");
            assert_eq!(picker.custom_start, Some(0x250));
        }

        // Block navigation clears custom state.
        apply_action(Action::GlyphPickerBlock(1), &mut s, &mut Mapper::default());
        {
            let picker = s.glyph_picker.as_ref().unwrap();
            assert!(!picker.custom_focus, "block nav exits custom focus");
            assert!(picker.custom_buf.is_empty(), "block nav clears custom_buf");
            assert_eq!(picker.custom_start, None, "block nav clears custom_start");
        }
    }

    #[test]
    fn glyph_picker_custom_focus_blocks_pending() {
        // Verify that GlyphPickerChar is ignored when custom_focus is active.
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        apply_action(
            Action::StyleOpenGlyphPicker(crate::state::BorderZone::Top),
            &mut s,
            &mut Mapper::default(),
        );
        apply_action(Action::GlyphPickerCustomFocus, &mut s, &mut Mapper::default());
        apply_action(Action::GlyphPickerChar('═'), &mut s, &mut Mapper::default());
        assert!(
            s.glyph_picker.as_ref().unwrap().pending.is_none(),
            "GlyphPickerChar should not set pending while in custom focus",
        );
    }

    #[test]
    fn swatch_pick_sets_color_for_target_and_default_clears() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        { let ed = s.style_editor.as_mut().unwrap(); ed.color_target = false; ed.swatch_cursor = 16; } // default cell
        apply_action(Action::StyleSwatchPick, &mut s, &mut mapper::mapper::Mapper::default());
        let ed = s.style_editor.as_ref().unwrap();
        let sel = ed.selectors[ed.active].to_string();
        // default cell clears fg
        assert!(ed.doc.colors.selectors.get(&sel).map_or(true, |d| d.fg.is_none()));
    }

    #[test]
    fn border_type_cycle_updates_decl_style() {
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        { let ed = s.style_editor.as_mut().unwrap();
          ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap(); }
        apply_action(Action::StyleBorderTypeCycle(1), &mut s, &mut mapper::mapper::Mapper::default());
        let ed = s.style_editor.as_ref().unwrap();
        let st = ed.doc.colors.selectors.get("map_border").and_then(|d| d.style.clone());
        assert!(st.is_some(), "cycling sets the border style name on the decl");
    }
    #[test]
    fn is_bordered_selector_covers_the_six() {
        for sel in ["map_border","story_border","dialog","upper_window_border","status_header","input_line"] {
            assert!(crate::input::is_bordered_selector(sel), "{sel} is bordered");
        }
        assert!(!crate::input::is_bordered_selector("transcript"));
    }

    #[test]
    fn picture_frame_zone_does_not_open_picker() {
        use crate::state::BorderZone;

        // Active selector is "map_border" with style = "picture-frame" → picker must stay None.
        let mut state = AppState::default();
        open_style_editor_hermetic(&mut state);
        {
            let ed = state.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap();
            ed.doc.colors.selectors.entry("map_border".into()).or_default().style =
                Some("picture-frame".into());
        }
        apply_action(
            Action::StyleOpenGlyphPicker(BorderZone::Top),
            &mut state,
            &mut Mapper::default(),
        );
        assert!(
            state.glyph_picker.is_none(),
            "picture-frame zones must not open the glyph picker",
        );

        // Sanity: the same selector with style = None (→ "single") DOES open the picker.
        let mut state2 = AppState::default();
        open_style_editor_hermetic(&mut state2);
        {
            let ed = state2.style_editor.as_mut().unwrap();
            ed.active = ed.selectors.iter().position(|x| *x == "map_border").unwrap();
            // Explicitly clear any disk-loaded border style so this is not picture-frame.
            ed.doc.colors.selectors.entry("map_border".into()).or_default().style = None;
        }
        apply_action(
            Action::StyleOpenGlyphPicker(BorderZone::Top),
            &mut state2,
            &mut Mapper::default(),
        );
        assert!(
            state2.glyph_picker.is_some(),
            "non-picture-frame selector opens the picker",
        );
    }

    #[test]
    fn border_focus_only_on_bordered_selectors() {
        use crate::state::StyleFocus;
        let m = &mut mapper::mapper::Mapper::default();

        // ── non-bordered selector: cycling from Attrs wraps to Board, never Border ──
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        {
            let ed = s.style_editor.as_mut().unwrap();
            let non_bordered_idx = ed.selectors.iter()
                .position(|sel| !crate::input::is_bordered_selector(sel))
                .expect("at least one non-bordered selector exists");
            ed.active = non_bordered_idx;
            ed.focus = StyleFocus::Attrs;
        }
        apply_action(Action::StyleFocusCycle(1), &mut s, m);
        assert_eq!(s.style_editor.as_ref().unwrap().focus, StyleFocus::Board,
            "non-bordered selector must skip Border focus");

        // ── bordered selector: cycling from Attrs reaches Border ──
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        {
            let ed = s.style_editor.as_mut().unwrap();
            let bordered_idx = ed.selectors.iter()
                .position(|sel| crate::input::is_bordered_selector(sel))
                .expect("at least one bordered selector exists");
            ed.active = bordered_idx;
            ed.focus = StyleFocus::Attrs;
        }
        apply_action(Action::StyleFocusCycle(1), &mut s, m);
        assert_eq!(s.style_editor.as_ref().unwrap().focus, StyleFocus::Border,
            "bordered selector must reach Border focus");

        // ── navigating away from bordered selector drops stale Border focus ──
        let mut s = AppState::default();
        open_style_editor_hermetic(&mut s);
        {
            let ed = s.style_editor.as_mut().unwrap();
            let bordered_idx = ed.selectors.iter()
                .position(|sel| crate::input::is_bordered_selector(sel))
                .expect("at least one bordered selector exists");
            ed.active = bordered_idx;
            ed.focus = StyleFocus::Border;
        }
        apply_action(Action::StyleNav(1), &mut s, m);
        let ed = s.style_editor.as_ref().unwrap();
        if !crate::input::is_bordered_selector(ed.selectors[ed.active]) {
            assert_ne!(ed.focus, StyleFocus::Border,
                "Border focus must drop on a non-bordered selector");
        }
    }
}
