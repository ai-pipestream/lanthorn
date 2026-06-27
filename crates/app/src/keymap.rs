//! Configurable keymap — `Command`, `KeySpec`, `Context`, and `KeyMap`.
//!
//! `Command` enumerates every rebindable action. `KeySpec` is a parsed
//! keystroke (key code + modifier flags). `Context` partitions bindings into
//! Global, Map, and Anim layers. `KeyMap` holds the full binding table and
//! exposes lookup / resolve / primary-key queries.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ── Context ────────────────────────────────────────────────────────────────────

/// Which dispatch layer a binding belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Context {
    /// Reached in any focus when no prompt or anim sub-mode is active.
    Global,
    /// Map-focus bindings (also fall through to Global on miss).
    Map,
    /// Tidy-animation sub-mode (does NOT fall through).
    Anim,
}

// ── Command ────────────────────────────────────────────────────────────────────

/// Every rebindable named command.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Command {
    // ── Global ────────────────────────────────────────────────────────────────
    /// Rebindable quit binding (Ctrl+Q/C remain hardwired separately).
    Quit,
    SaveGame,
    RestoreGame,
    ExportSvg,
    ExportDot,
    ExportDump,
    CycleLayout,
    Retidy,
    ReloadStyle,
    GameStyle,
    ToggleWatch,
    AnimateTidy,
    ToggleAlignment,
    TogglePortalLabels,
    /// Toggle between Game and Map focus.
    ToggleFocus,
    // ── Nudge (Global, Ctrl+Arrows) ───────────────────────────────────────────
    NudgeLeft,
    NudgeRight,
    NudgeUp,
    NudgeDown,

    // ── Map ───────────────────────────────────────────────────────────────────
    OpenGallery,
    ZoomIn,
    ZoomOut,
    /// Reset zoom to the default level and clear the char-pan offset.
    ZoomReset,
    Recenter,
    SelectNext,
    SelectPrev,
    RenameRoom,
    RenameLayer,
    EditNotes,
    DeleteSelectedConnection,
    RelabelSelectedEdge,
    ToggleInspector,
    PeelLayer,
    MergeLayer,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    CycleLayerNext,
    CycleLayerPrev,

    // ── Anim ──────────────────────────────────────────────────────────────────
    AnimStepFwd,
    AnimStepBack,
    AnimTogglePlay,
    AnimExit,
    // Anim pan/zoom reuse the Map variants above.

    // ── Saves manager ─────────────────────────────────────────────────────────
    /// Open the saves-manager modal (Ctrl+O by default).
    OpenSaves,

    // ── Inventory ─────────────────────────────────────────────────────────────
    /// Toggle the inventory strip at the bottom of the story pane (default: v).
    ToggleInventory,

    // ── Layout ────────────────────────────────────────────────────────────────
    /// Cycle the UI layout in reverse (Split → MapFull → TranscriptFull → Split).
    CycleLayoutReverse,

    // ── Game ──────────────────────────────────────────────────────────────────
    /// Reset the game to its opening state (keeps the accumulated map).
    ResetGame,

    // ── Verb menu ─────────────────────────────────────────────────────────────
    /// Open the verb/item token-palette modal (default: m).
    OpenVerbMenu,

    // ── Style editor ──────────────────────────────────────────────────────────
    /// Open the live style editor full-screen mode (default: F3).
    OpenStyleEditor,

    // ── Config screen ─────────────────────────────────────────────────────────
    /// Open the in-app config screen modal (default: F2).
    OpenConfig,

    // ── Room display ──────────────────────────────────────────────────────────
    /// Toggle room-number (#id) visibility in Boxes-zoom room boxes.
    ToggleRoomNumbers,
    /// Toggle the room-detection-method indicator in the map corner.
    ToggleLocMethod,
    /// Toggle the status/score bar (top row of the story pane).
    ToggleStatusBar,

    // ── Hints ─────────────────────────────────────────────────────────────────
    /// Open the Hints panel (companion Invisiclues / hint-file mini-terminal).
    OpenHints,

    // ── History ───────────────────────────────────────────────────────────────
    /// Open the rewind/replay history modal.
    OpenHistory,
}

impl Command {
    /// Convert this command into its `Action` equivalent.
    pub fn to_action(self) -> crate::input::Action {
        use crate::input::Action;
        match self {
            Command::Quit => Action::Quit,
            Command::SaveGame => Action::SaveGame,
            Command::RestoreGame => Action::RestoreGame,
            Command::ExportSvg => Action::ExportSvg,
            Command::ExportDot => Action::ExportDot,
            Command::ExportDump => Action::ExportDump,
            Command::CycleLayout => Action::CycleLayout,
            Command::Retidy => Action::Retidy,
            Command::ReloadStyle => Action::ReloadStyle,
            Command::GameStyle => Action::GameStyle,
            Command::ToggleWatch => Action::ToggleWatch,
            Command::AnimateTidy => Action::AnimateTidy,
            Command::ToggleAlignment => Action::ToggleAlignment,
            Command::TogglePortalLabels => Action::TogglePortalLabels,
            Command::ToggleFocus => Action::ToggleFocus,
            Command::NudgeLeft => Action::NudgeSelected(-1, 0),
            Command::NudgeRight => Action::NudgeSelected(1, 0),
            Command::NudgeUp => Action::NudgeSelected(0, -1),
            Command::NudgeDown => Action::NudgeSelected(0, 1),
            Command::OpenGallery => Action::OpenGallery,
            Command::ZoomIn => Action::ZoomIn,
            Command::ZoomOut => Action::ZoomOut,
            Command::ZoomReset => Action::ZoomReset,
            Command::Recenter => Action::Recenter,
            Command::SelectNext => Action::SelectNext,
            Command::SelectPrev => Action::SelectPrev,
            Command::RenameRoom => Action::RenameRoom,
            Command::RenameLayer => Action::RenameLayer,
            Command::EditNotes => Action::EditNotes,
            Command::DeleteSelectedConnection => Action::DeleteSelectedConnection,
            Command::RelabelSelectedEdge => Action::RelabelSelectedEdge,
            Command::ToggleInspector => Action::ToggleInspector,
            Command::PeelLayer => Action::PeelLayer,
            Command::MergeLayer => Action::MergeLayer,
            Command::PanLeft => Action::Pan(-1, 0),
            Command::PanRight => Action::Pan(1, 0),
            Command::PanUp => Action::Pan(0, -1),
            Command::PanDown => Action::Pan(0, 1),
            Command::CycleLayerNext => Action::CycleLayer(1),
            Command::CycleLayerPrev => Action::CycleLayer(-1),
            Command::AnimStepFwd => Action::AnimStep(1),
            Command::AnimStepBack => Action::AnimStep(-1),
            Command::AnimTogglePlay => Action::AnimTogglePlay,
            Command::AnimExit => Action::AnimExit,
            Command::OpenSaves => Action::OpenSaves,
            Command::ToggleInventory => Action::ToggleInventory,
            Command::CycleLayoutReverse => Action::CycleLayoutReverse,
            Command::ResetGame => Action::ResetGame,
            Command::OpenVerbMenu => Action::OpenVerbMenu,
            Command::OpenStyleEditor => Action::OpenStyleEditor,
            Command::OpenConfig => Action::OpenConfig,
            Command::ToggleRoomNumbers => Action::ToggleRoomNumbers,
            Command::ToggleLocMethod => Action::ToggleLocMethod,
            Command::ToggleStatusBar => Action::ToggleStatusBar,
            Command::OpenHints => Action::OpenHints,
            Command::OpenHistory => Action::OpenHistory,
        }
    }

    /// Snake_case config key for this command.
    pub fn name(self) -> &'static str {
        match self {
            Command::Quit => "quit",
            Command::SaveGame => "save_game",
            Command::RestoreGame => "restore_game",
            Command::ExportSvg => "export_svg",
            Command::ExportDot => "export_dot",
            Command::ExportDump => "export_dump",
            Command::CycleLayout => "cycle_layout",
            Command::Retidy => "retidy",
            Command::ReloadStyle => "reload_style",
            Command::GameStyle => "game_style",
            Command::ToggleWatch => "toggle_watch",
            Command::AnimateTidy => "animate_tidy",
            Command::ToggleAlignment => "toggle_alignment",
            Command::TogglePortalLabels => "toggle_portal_labels",
            Command::ToggleFocus => "toggle_focus",
            Command::NudgeLeft => "nudge_left",
            Command::NudgeRight => "nudge_right",
            Command::NudgeUp => "nudge_up",
            Command::NudgeDown => "nudge_down",
            Command::OpenGallery => "open_gallery",
            Command::ZoomIn => "zoom_in",
            Command::ZoomOut => "zoom_out",
            Command::ZoomReset => "zoom_reset",
            Command::Recenter => "recenter",
            Command::SelectNext => "select_next",
            Command::SelectPrev => "select_prev",
            Command::RenameRoom => "rename_room",
            Command::RenameLayer => "rename_layer",
            Command::EditNotes => "edit_notes",
            Command::DeleteSelectedConnection => "delete_selected_connection",
            Command::RelabelSelectedEdge => "relabel_selected_edge",
            Command::ToggleInspector => "toggle_inspector",
            Command::PeelLayer => "peel_layer",
            Command::MergeLayer => "merge_layer",
            Command::PanLeft => "pan_left",
            Command::PanRight => "pan_right",
            Command::PanUp => "pan_up",
            Command::PanDown => "pan_down",
            Command::CycleLayerNext => "cycle_layer_next",
            Command::CycleLayerPrev => "cycle_layer_prev",
            Command::AnimStepFwd => "anim_step_fwd",
            Command::AnimStepBack => "anim_step_back",
            Command::AnimTogglePlay => "anim_toggle_play",
            Command::AnimExit => "anim_exit",
            Command::OpenSaves => "open_saves",
            Command::ToggleInventory => "toggle_inventory",
            Command::CycleLayoutReverse => "cycle_layout_reverse",
            Command::ResetGame => "reset_game",
            Command::OpenVerbMenu => "open_verb_menu",
            Command::OpenStyleEditor => "open_style_editor",
            Command::OpenConfig => "open_config",
            Command::ToggleRoomNumbers => "toggle_room_numbers",
            Command::ToggleLocMethod => "toggle_loc_method",
            Command::ToggleStatusBar => "toggle_status_bar",
            Command::OpenHints => "open_hints",
            Command::OpenHistory => "open_history",
        }
    }

    /// Short human-readable label for the hint bar and help overlay.
    pub fn label(self) -> &'static str {
        match self {
            Command::Quit => "quit",
            Command::SaveGame => "save game",
            Command::RestoreGame => "restore",
            Command::ExportSvg => "export SVG",
            Command::ExportDot => "export DOT",
            Command::ExportDump => "dump map",
            Command::CycleLayout => "layout",
            Command::Retidy => "retidy",
            Command::ReloadStyle => "reload style",
            Command::GameStyle => "game style",
            Command::ToggleWatch => "watch style",
            Command::AnimateTidy => "animate tidy",
            Command::ToggleAlignment => "alignment",
            Command::TogglePortalLabels => "portals",
            Command::ToggleFocus => "focus",
            Command::NudgeLeft => "nudge left",
            Command::NudgeRight => "nudge right",
            Command::NudgeUp => "nudge up",
            Command::NudgeDown => "nudge down",
            Command::OpenGallery => "gallery",
            Command::ZoomIn => "zoom in",
            Command::ZoomOut => "zoom out",
            Command::ZoomReset => "zoom reset",
            Command::Recenter => "center",
            Command::SelectNext => "next room",
            Command::SelectPrev => "prev room",
            Command::RenameRoom => "rename room",
            Command::RenameLayer => "rename layer",
            Command::EditNotes => "edit notes",
            Command::DeleteSelectedConnection => "delete conn",
            Command::RelabelSelectedEdge => "relabel edge",
            Command::ToggleInspector => "inspect",
            Command::PeelLayer => "peel layer",
            Command::MergeLayer => "merge layer",
            Command::PanLeft => "pan left",
            Command::PanRight => "pan right",
            Command::PanUp => "pan up",
            Command::PanDown => "pan down",
            Command::CycleLayerNext => "layer next",
            Command::CycleLayerPrev => "layer prev",
            Command::AnimStepFwd => "step fwd",
            Command::AnimStepBack => "step back",
            Command::AnimTogglePlay => "play/pause",
            Command::AnimExit => "exit anim",
            Command::OpenSaves => "saves",
            Command::ToggleInventory => "inventory",
            Command::CycleLayoutReverse => "layout back",
            Command::ResetGame => "reset game",
            Command::OpenVerbMenu => "verb menu",
            Command::OpenStyleEditor => "style editor",
            Command::OpenConfig => "settings",
            Command::ToggleRoomNumbers => "room numbers",
            Command::ToggleLocMethod => "location method",
            Command::ToggleStatusBar => "status bar",
            Command::OpenHints => "hints",
            Command::OpenHistory => "history",
        }
    }

    /// The primary context this command belongs to (used by resolve).
    pub fn context(self) -> Context {
        match self {
            Command::Quit
            | Command::SaveGame
            | Command::RestoreGame
            | Command::ExportSvg
            | Command::ExportDot
            | Command::ExportDump
            | Command::CycleLayout
            | Command::Retidy
            | Command::ReloadStyle
            | Command::GameStyle
            | Command::ToggleWatch
            | Command::AnimateTidy
            | Command::ToggleAlignment
            | Command::TogglePortalLabels
            | Command::ToggleFocus
            | Command::NudgeLeft
            | Command::NudgeRight
            | Command::NudgeUp
            | Command::NudgeDown => Context::Global,

            Command::OpenGallery
            | Command::ZoomIn
            | Command::ZoomOut
            | Command::ZoomReset
            | Command::Recenter
            | Command::SelectNext
            | Command::SelectPrev
            | Command::RenameRoom
            | Command::RenameLayer
            | Command::EditNotes
            | Command::DeleteSelectedConnection
            | Command::RelabelSelectedEdge
            | Command::ToggleInspector
            | Command::PeelLayer
            | Command::MergeLayer
            | Command::PanLeft
            | Command::PanRight
            | Command::PanUp
            | Command::PanDown
            | Command::CycleLayerNext
            | Command::CycleLayerPrev => Context::Map,

            Command::AnimStepFwd
            | Command::AnimStepBack
            | Command::AnimTogglePlay
            | Command::AnimExit => Context::Anim,

            Command::OpenSaves => Context::Global,
            Command::ToggleInventory => Context::Global,
            Command::CycleLayoutReverse => Context::Global,
            Command::ResetGame => Context::Global,
            Command::OpenVerbMenu => Context::Global,
            Command::OpenStyleEditor => Context::Global,
            Command::OpenConfig => Context::Global,
            Command::ToggleRoomNumbers => Context::Global,
            Command::ToggleLocMethod => Context::Global,
            Command::ToggleStatusBar => Context::Global,
            Command::OpenHints => Context::Global,
            Command::OpenHistory => Context::Global,
        }
    }

    /// Return the `Command` whose `name()` matches `s`, or `None`.
    pub fn from_name(s: &str) -> Option<Command> {
        ALL_COMMANDS.iter().copied().find(|c| c.name() == s)
    }
}

/// All commands in a single slice, for iteration.
pub const ALL_COMMANDS: &[Command] = &[
    Command::Quit,
    Command::SaveGame,
    Command::RestoreGame,
    Command::ExportSvg,
    Command::ExportDot,
    Command::ExportDump,
    Command::CycleLayout,
    Command::Retidy,
    Command::ReloadStyle,
    Command::GameStyle,
    Command::ToggleWatch,
    Command::AnimateTidy,
    Command::ToggleAlignment,
    Command::TogglePortalLabels,
    Command::ToggleFocus,
    Command::NudgeLeft,
    Command::NudgeRight,
    Command::NudgeUp,
    Command::NudgeDown,
    Command::OpenGallery,
    Command::ZoomIn,
    Command::ZoomOut,
    Command::ZoomReset,
    Command::Recenter,
    Command::SelectNext,
    Command::SelectPrev,
    Command::RenameRoom,
    Command::RenameLayer,
    Command::EditNotes,
    Command::DeleteSelectedConnection,
    Command::RelabelSelectedEdge,
    Command::ToggleInspector,
    Command::PeelLayer,
    Command::MergeLayer,
    Command::PanLeft,
    Command::PanRight,
    Command::PanUp,
    Command::PanDown,
    Command::CycleLayerNext,
    Command::CycleLayerPrev,
    Command::AnimStepFwd,
    Command::AnimStepBack,
    Command::AnimTogglePlay,
    Command::AnimExit,
    Command::OpenSaves,
    Command::ToggleInventory,
    Command::CycleLayoutReverse,
    Command::ResetGame,
    Command::OpenVerbMenu,
    Command::OpenStyleEditor,
    Command::OpenConfig,
    Command::ToggleRoomNumbers,
    Command::ToggleLocMethod,
    Command::ToggleStatusBar,
    Command::OpenHints,
    Command::OpenHistory,
];

// ── KeySpec ────────────────────────────────────────────────────────────────────

/// A parsed keystroke: key code plus modifier flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeySpec {
    pub code: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeySpec {
    /// Normalize a live crossterm `KeyEvent` into a `KeySpec` for lookups.
    pub fn from_key_event(k: KeyEvent) -> KeySpec {
        KeySpec {
            code: k.code,
            ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
            shift: k.modifiers.contains(KeyModifiers::SHIFT),
            alt: k.modifiers.contains(KeyModifiers::ALT),
        }
    }

    /// Human-readable label for the hint bar / help screen.
    /// Examples: "Ctrl+S", "Shift+←", "h", "F1", "Space", "Shift+Tab".
    pub fn label(&self) -> String {
        // BackTab is always "Shift+Tab" regardless of modifier flags.
        if self.code == KeyCode::BackTab {
            let mut s = String::new();
            if self.ctrl { s.push_str("Ctrl+"); }
            if self.alt { s.push_str("Alt+"); }
            s.push_str("Shift+Tab");
            return s;
        }
        let mut s = String::new();
        if self.ctrl { s.push_str("Ctrl+"); }
        if self.alt { s.push_str("Alt+"); }
        if self.shift { s.push_str("Shift+"); }
        let key_str = match self.code {
            KeyCode::Left => "←".to_string(),
            KeyCode::Right => "→".to_string(),
            KeyCode::Up => "↑".to_string(),
            KeyCode::Down => "↓".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => unreachable!("handled above"),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PgUp".to_string(),
            KeyCode::PageDown => "PgDn".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::Char(c) => c.to_uppercase().to_string(),
            _ => format!("{:?}", self.code),
        };
        s.push_str(&key_str);
        s
    }
}

impl std::str::FromStr for KeySpec {
    type Err = String;

    /// Parse a key spec string like "ctrl+s", "shift+left", "+", "f1", "space".
    ///
    /// Modifiers (ctrl, shift, alt) may appear in any order before the key
    /// token, separated by '+'. A lone '+' character parses as Char('+').
    fn from_str(s: &str) -> Result<KeySpec, String> {
        let lower = s.trim().to_lowercase();

        // Special case: a lone "+" (would otherwise produce empty tokens when split).
        if lower == "+" {
            return Ok(KeySpec { code: KeyCode::Char('+'), ctrl: false, shift: false, alt: false });
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        let mut key_token: Option<String> = None;

        let parts: Vec<&str> = lower.split('+').collect();
        let n = parts.len();

        // Walk tokens: modifier keywords consume early slots; the last
        // non-empty, non-modifier token is the key. Handle "+" embedded in
        // the split: a trailing empty part after '+' means '+' was the last char.
        let mut i = 0;
        while i < n {
            let p = parts[i].trim();
            match p {
                "ctrl" | "control" => { ctrl = true; i += 1; }
                "shift" => { shift = true; i += 1; }
                "alt" => { alt = true; i += 1; }
                "" => {
                    // An empty segment from split means a literal '+' was there.
                    // E.g. "shift++" splits as ["shift", "", ""] — the key is '+'.
                    // We treat the first empty token after modifiers as the '+' key.
                    key_token = Some("+".to_string());
                    i += 1;
                }
                other => {
                    key_token = Some(other.to_string());
                    i += 1;
                }
            }
        }

        let tok = key_token.ok_or_else(|| format!("empty key spec: '{s}'"))?;

        let code = match tok.as_str() {
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "tab" => {
                // "shift+tab" parses as shift=true + token "tab"; map to BackTab.
                if shift {
                    shift = false; // BackTab encodes the shift itself
                    KeyCode::BackTab
                } else {
                    KeyCode::Tab
                }
            }
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "esc" | "escape" => KeyCode::Esc,
            "enter" | "return" => KeyCode::Enter,
            "backspace" => KeyCode::Backspace,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" | "pgdown" => KeyCode::PageDown,
            "f1" => KeyCode::F(1),
            "f2" => KeyCode::F(2),
            "f3" => KeyCode::F(3),
            "f4" => KeyCode::F(4),
            "f5" => KeyCode::F(5),
            "f6" => KeyCode::F(6),
            "f7" => KeyCode::F(7),
            "f8" => KeyCode::F(8),
            "f9" => KeyCode::F(9),
            "f10" => KeyCode::F(10),
            "f11" => KeyCode::F(11),
            "f12" => KeyCode::F(12),
            s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
            other => return Err(format!("unknown key token: '{other}'")),
        };

        Ok(KeySpec { code, ctrl, shift, alt })
    }
}

// ── KeyMap ─────────────────────────────────────────────────────────────────────

/// The full binding table. Each entry is `(KeySpec, Command, Context)`.
/// Multiple specs may map to the same command (multi-bind defaults).
#[derive(Debug)]
pub struct KeyMap {
    pub bindings: Vec<(KeySpec, Command, Context)>,
}

impl KeyMap {
    /// Build the default keymap from today's `key_to_action` dispatch.
    ///
    /// This is the single source of truth for back-compat. Every binding here
    /// must match `input.rs` exactly.
    pub fn default() -> KeyMap {
        use KeyCode::*;

        // Shorthand constructors.
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        let plain = |code| g(code, false, false);
        let ctrl = |code| g(code, true, false);
        let shift = |code| g(code, false, true);

        let mut b: Vec<(KeySpec, Command, Context)> = Vec::new();

        macro_rules! bind {
            ($spec:expr, $cmd:expr, $ctx:expr) => {
                b.push(($spec, $cmd, $ctx));
            };
        }

        // ── Global ────────────────────────────────────────────────────────────
        // Tab → ToggleFocus (the Tab KEY itself stays hardwired in key_to_action;
        // this entry lets the keymap advertise it for hints/help).
        bind!(plain(Tab), Command::ToggleFocus, Context::Global);

        bind!(ctrl(Char('s')), Command::SaveGame, Context::Global);
        bind!(ctrl(Char('r')), Command::RestoreGame, Context::Global);
        bind!(ctrl(Char('e')), Command::ExportSvg, Context::Global);
        bind!(ctrl(Char('g')), Command::ExportDot, Context::Global);
        bind!(ctrl(Char('d')), Command::ExportDump, Context::Global);
        bind!(ctrl(Char('l')), Command::CycleLayout, Context::Global);
        bind!(ctrl(Char('t')), Command::Retidy, Context::Global);
        bind!(ctrl(Char('y')), Command::AnimateTidy, Context::Global);
        bind!(ctrl(Char('a')), Command::ToggleAlignment, Context::Global);
        bind!(ctrl(Char('p')), Command::TogglePortalLabels, Context::Global);
        // Ctrl+O → open saves manager (free key; not used by any other command).
        bind!(ctrl(Char('o')), Command::OpenSaves, Context::Global);
        // v → toggle inventory strip (free key; not used in any context).
        bind!(plain(Char('v')), Command::ToggleInventory, Context::Global);
        // m → open verb/item token-palette modal (free key; not used by any other command).
        bind!(plain(Char('m')), Command::OpenVerbMenu, Context::Global);
        // F2 → open config screen.
        bind!(plain(F(2)), Command::OpenConfig, Context::Global);
        // F3 → open style editor (F3 is unbound; F2/F4–F9 are taken).
        bind!(plain(F(3)), Command::OpenStyleEditor, Context::Global);
        // Shift+Tab (BackTab) → cycle layout in reverse (inverse of Ctrl+L forward cycle).
        // BackTab is delivered by crossterm as KeyCode::BackTab, typically with no SHIFT modifier.
        bind!(KeySpec { code: BackTab, ctrl: false, shift: false, alt: false }, Command::CycleLayoutReverse, Context::Global);
        // F5 → reset game (free key; opens a confirmation prompt before acting).
        bind!(plain(F(5)), Command::ResetGame, Context::Global);
        // F4 → open rewind/replay history modal (free function key).
        bind!(plain(F(4)), Command::OpenHistory, Context::Global);

        // F6-F9 → Nudge (plain function keys; ctrl+arrow removed so all direct
        // bindings remain modifier-free).
        bind!(plain(F(6)), Command::NudgeLeft, Context::Global);
        bind!(plain(F(7)), Command::NudgeRight, Context::Global);
        bind!(plain(F(8)), Command::NudgeUp, Context::Global);
        bind!(plain(F(9)), Command::NudgeDown, Context::Global);

        // ── Map ───────────────────────────────────────────────────────────────
        // Pan: plain arrows + hjkl (two sets; shift-arrows removed so all
        // direct bindings remain modifier-free).
        bind!(plain(Left), Command::PanLeft, Context::Map);
        bind!(plain(Right), Command::PanRight, Context::Map);
        bind!(plain(Up), Command::PanUp, Context::Map);
        bind!(plain(Down), Command::PanDown, Context::Map);

        bind!(plain(Char('h')), Command::PanLeft, Context::Map);
        bind!(plain(Char('l')), Command::PanRight, Context::Map);
        bind!(plain(Char('k')), Command::PanUp, Context::Map);
        bind!(plain(Char('j')), Command::PanDown, Context::Map);

        // Zoom: + / = (shift(+) alias removed; plain alternatives cover it).
        bind!(plain(Char('+')), Command::ZoomIn, Context::Map);
        bind!(plain(Char('=')), Command::ZoomIn, Context::Map);
        bind!(plain(Char('-')), Command::ZoomOut, Context::Map);
        // '0' (zero) resets zoom to default (Boxes) and clears char-pan offset.
        bind!(plain(Char('0')), Command::ZoomReset, Context::Map);

        // Map commands
        bind!(plain(Char('c')), Command::Recenter, Context::Map);
        bind!(plain(Char('n')), Command::SelectNext, Context::Map);
        bind!(plain(Char('p')), Command::SelectPrev, Context::Map);
        bind!(shift(Char('N')), Command::RenameLayer, Context::Map);
        bind!(shift(Char('P')), Command::PeelLayer, Context::Map);
        bind!(shift(Char('M')), Command::MergeLayer, Context::Map);
        bind!(shift(Char('R')), Command::Retidy, Context::Map);
        bind!(plain(Char(']')), Command::CycleLayerNext, Context::Map);
        bind!(plain(Char('[')), Command::CycleLayerPrev, Context::Map);
        bind!(plain(Char('r')), Command::RenameRoom, Context::Map);
        bind!(plain(Char('o')), Command::EditNotes, Context::Map);
        bind!(plain(Char('d')), Command::DeleteSelectedConnection, Context::Map);
        bind!(plain(Char('e')), Command::RelabelSelectedEdge, Context::Map);
        bind!(plain(Char('i')), Command::ToggleInspector, Context::Map);
        bind!(plain(Char('g')), Command::OpenGallery, Context::Map);
        // Esc → ToggleFocus (in map context)
        bind!(plain(Esc), Command::ToggleFocus, Context::Map);

        // ── Anim ──────────────────────────────────────────────────────────────
        // Pan in anim: hjkl only (plain arrows are bound to step; shift-arrows
        // removed so all direct bindings remain modifier-free).
        bind!(plain(Char('h')), Command::PanLeft, Context::Anim);
        bind!(plain(Char('l')), Command::PanRight, Context::Anim);
        bind!(plain(Char('k')), Command::PanUp, Context::Anim);
        bind!(plain(Char('j')), Command::PanDown, Context::Anim);

        // Zoom in anim
        bind!(plain(Char('+')), Command::ZoomIn, Context::Anim);
        bind!(plain(Char('=')), Command::ZoomIn, Context::Anim);
        bind!(plain(Char('-')), Command::ZoomOut, Context::Anim);

        // Step / play / exit
        bind!(plain(Left), Command::AnimStepBack, Context::Anim);
        bind!(plain(Right), Command::AnimStepFwd, Context::Anim);
        bind!(plain(Char(' ')), Command::AnimTogglePlay, Context::Anim);
        bind!(plain(Esc), Command::AnimExit, Context::Anim);
        bind!(plain(Enter), Command::AnimExit, Context::Anim);

        KeyMap { bindings: b }
    }

    /// Look up a key in the given context.
    ///
    /// - `Context::Map` also searches `Context::Global` on miss (fall-through).
    /// - `Context::Global` and `Context::Anim` do not fall through.
    pub fn lookup(&self, spec: &KeySpec, ctx: Context) -> Option<Command> {
        // Exact context match first.
        for (s, cmd, c) in &self.bindings {
            if c == &ctx && s == spec {
                return Some(*cmd);
            }
        }
        // Map falls through to Global.
        if ctx == Context::Map {
            for (s, cmd, c) in &self.bindings {
                if c == &Context::Global && s == spec {
                    return Some(*cmd);
                }
            }
        }
        None
    }

    /// Return the first (primary) `KeySpec` bound to `cmd`.
    ///
    /// Prefers the binding in the command's own context; falls back to any context.
    pub fn primary_key(&self, cmd: Command) -> Option<KeySpec> {
        let preferred_ctx = cmd.context();
        // Try the command's own context first.
        if let Some((s, _, _)) = self.bindings.iter().find(|(_, c, cx)| *c == cmd && *cx == preferred_ctx) {
            return Some(*s);
        }
        // Fall back to any context.
        self.bindings.iter()
            .find(|(_, c, _)| *c == cmd)
            .map(|(s, _, _)| *s)
    }

    /// Look up a key across ALL contexts (Global → Map → Anim) and return the
    /// first match. Used by the hotkey dialog so that commands in any context
    /// can be triggered from the dialog.
    pub fn lookup_any(&self, spec: &KeySpec) -> Option<Command> {
        for ctx in [Context::Global, Context::Map, Context::Anim] {
            // Use exact context match only (no Map→Global fallthrough here,
            // since we already iterate Global first).
            for (s, cmd, c) in &self.bindings {
                if c == &ctx && s == spec {
                    return Some(*cmd);
                }
            }
        }
        None
    }

    /// Iterate all `(KeySpec, Command)` pairs that belong to `ctx`
    /// (for the help screen's per-context listing).
    pub fn for_context(&self, ctx: Context) -> impl Iterator<Item = (&KeySpec, &Command)> {
        self.bindings.iter()
            .filter(move |(_, _, c)| *c == ctx)
            .map(|(s, cmd, _)| (s, cmd))
    }

    /// Build a keymap from config overrides.
    ///
    /// Returns the resolved `KeyMap` and a list of warning strings for
    /// overrides that were rejected (unknown name, parse error, conflict).
    ///
    /// TODO(Task 8): consume context sections (use_defaults, global, map, anim).
    pub fn resolve(_cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String>) {
        (KeyMap::default(), Vec::new())
    }
}

// ── HotkeyLayout ──────────────────────────────────────────────────────────────

/// Default snake_case names for the direct (always-available) command set.
const DEFAULT_DIRECT_NAMES: &[&str] = &[
    "quit",
    "save_game",
    "restore_game",
    "pan_left",
    "pan_right",
    "pan_up",
    "pan_down",
    "zoom_in",
    "zoom_out",
    "zoom_reset",
    "select_next",
    "select_prev",
    "recenter",
    "toggle_focus",
    "cycle_layout_reverse",
    "nudge_left",
    "nudge_right",
    "nudge_up",
    "nudge_down",
];

/// Default groups for the hotkey dialog (title, command snake_case names).
const DEFAULT_GROUPS: &[(&str, &[&str])] = &[
    ("Layout", &["retidy", "animate_tidy", "cycle_layout"]),
    ("Layers", &["peel_layer", "merge_layer", "cycle_layer_next", "cycle_layer_prev", "rename_layer"]),
    ("Edit", &["rename_room", "edit_notes", "delete_selected_connection", "relabel_selected_edge"]),
    ("Files", &["open_saves", "open_history", "reset_game", "export_svg", "export_dot", "export_dump"]),
    ("View", &["toggle_alignment", "toggle_portal_labels", "toggle_inspector", "open_gallery", "toggle_inventory", "open_verb_menu", "open_config"]),
];

/// Runtime layout for the hotkey dialog.
///
/// Controls which key opens the dialog (`prefix`), which commands are always
/// reachable without the dialog (`direct`), and how commands are grouped inside
/// the dialog (`groups`).
#[derive(Debug)]
pub struct HotkeyLayout {
    /// The key that opens (and closes) the dialog.
    pub prefix: KeySpec,
    /// Commands that are always available without opening the dialog.
    pub direct: std::collections::HashSet<Command>,
    /// Groups of commands shown in the dialog: (group title, commands).
    pub groups: Vec<(String, Vec<Command>)>,
}

impl HotkeyLayout {
    /// Build the built-in default layout.
    pub fn default() -> HotkeyLayout {
        let prefix: KeySpec = "ctrl+k".parse().expect("ctrl+k must parse");

        let direct = DEFAULT_DIRECT_NAMES
            .iter()
            .filter_map(|name| Command::from_name(name))
            .collect();

        let groups = DEFAULT_GROUPS
            .iter()
            .map(|(title, names)| {
                let cmds = names
                    .iter()
                    .filter_map(|name| Command::from_name(name))
                    .collect();
                (title.to_string(), cmds)
            })
            .collect();

        HotkeyLayout { prefix, direct, groups }
    }

    /// Resolve a `HotkeyLayout` from config, producing warnings for unknown command names.
    ///
    /// Fields that are `None` in the config use the built-in defaults.
    pub fn resolve(cfg: &crate::config::HotkeysConfig) -> (HotkeyLayout, Vec<String>) {
        let mut layout = HotkeyLayout::default();
        let mut warnings: Vec<String> = Vec::new();

        // Override prefix if specified.
        if let Some(prefix_str) = &cfg.prefix {
            match prefix_str.parse::<KeySpec>() {
                Ok(spec) => layout.prefix = spec,
                Err(e) => warnings.push(format!("hotkeys: prefix '{}': {e}; using default", prefix_str)),
            }
        }

        // Override direct set if specified.
        if let Some(direct_names) = &cfg.direct {
            let mut direct_set = std::collections::HashSet::new();
            for name in direct_names {
                match Command::from_name(name) {
                    Some(cmd) => { direct_set.insert(cmd); }
                    None => warnings.push(format!("hotkeys: direct: unknown command '{name}'; skipped")),
                }
            }
            layout.direct = direct_set;
        }

        // Override groups if any are specified.
        if !cfg.group.is_empty() {
            let mut groups = Vec::new();
            for g in &cfg.group {
                let mut cmds = Vec::new();
                for name in &g.commands {
                    match Command::from_name(name) {
                        Some(cmd) => cmds.push(cmd),
                        None => warnings.push(format!("hotkeys: group '{}': unknown command '{name}'; dropped", g.title)),
                    }
                }
                groups.push((g.title.clone(), cmds));
            }
            layout.groups = groups;
        }

        (layout, warnings)
    }

    /// Check whether a command is in the direct (always-available) set.
    pub fn is_direct(&self, cmd: Command) -> bool {
        self.direct.contains(&cmd)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Action;

    // Task 1: Command::to_action and name()
    #[test]
    fn command_to_action_maps_directionals_and_names() {
        assert!(matches!(Command::PanLeft.to_action(), Action::Pan(-1, 0)));
        assert!(matches!(Command::NudgeUp.to_action(), Action::NudgeSelected(0, -1)));
        assert!(matches!(Command::CycleLayerNext.to_action(), Action::CycleLayer(1)));
        assert!(matches!(Command::AnimStepBack.to_action(), Action::AnimStep(-1)));
        assert!(matches!(Command::SaveGame.to_action(), Action::SaveGame));
        assert_eq!(Command::ToggleFocus.name(), "toggle_focus");
    }

    // Task 2: KeySpec parsing and labels
    #[test]
    fn keyspec_parse_and_label_roundtrip() {
        let s: KeySpec = "ctrl+s".parse().unwrap();
        assert_eq!((s.ctrl, s.code), (true, KeyCode::Char('s')));
        assert_eq!("shift+left".parse::<KeySpec>().unwrap().code, KeyCode::Left);
        assert_eq!("+".parse::<KeySpec>().unwrap().code, KeyCode::Char('+'));
        assert_eq!("f1".parse::<KeySpec>().unwrap().code, KeyCode::F(1));
        assert_eq!("space".parse::<KeySpec>().unwrap().code, KeyCode::Char(' '));
        assert!("nope".parse::<KeySpec>().is_err());
        assert_eq!("ctrl+s".parse::<KeySpec>().unwrap().label(), "Ctrl+S");
    }

    // Task 3a: default keymap matches today's bindings
    #[test]
    fn default_keymap_matches_todays_bindings() {
        let km = KeyMap::default();
        let g = |code, ctrl, shift| KeySpec { code, ctrl, shift, alt: false };
        use KeyCode::*;
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Global), Some(Command::SaveGame));
        assert_eq!(km.lookup(&g(Char('n'), false, false), Context::Map), Some(Command::SelectNext));
        assert_eq!(km.lookup(&g(Char('h'), false, false), Context::Map), Some(Command::PanLeft));
        // Map falls through to Global:
        assert_eq!(km.lookup(&g(Char('s'), true, false), Context::Map), Some(Command::SaveGame));
        // shift-arrow pan aliases were removed; plain arrow still works:
        assert_eq!(km.lookup(&g(Left, false, false), Context::Map), Some(Command::PanLeft));
        // shift-arrow is no longer bound in map context:
        assert_eq!(km.lookup(&g(Left, false, true), Context::Map), None);
    }

    // ── HotkeyLayout tests ────────────────────────────────────────────────────

    #[test]
    fn hotkey_layout_default_direct_and_indirect() {
        let layout = HotkeyLayout::default();
        // Direct commands
        assert!(layout.is_direct(Command::Recenter), "Recenter should be direct");
        assert!(layout.is_direct(Command::Quit), "Quit should be direct");
        assert!(layout.is_direct(Command::ToggleFocus), "ToggleFocus should be direct");
        // Non-direct (dialog-only) commands
        assert!(!layout.is_direct(Command::Retidy), "Retidy should NOT be direct");
        assert!(!layout.is_direct(Command::OpenGallery), "OpenGallery should NOT be direct");
        // Groups
        assert_eq!(layout.groups.len(), 5, "default layout should have 5 groups");
        assert_eq!(layout.groups[0].0, "Layout", "first group title should be Layout");
    }

    #[test]
    fn hotkey_layout_resolve_custom_direct_and_unknown_name() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: Some(vec!["save_game".into(), "quit".into(), "not_a_command".into()]),
            group: vec![HotkeyGroupConfig { title: "T".into(), commands: vec!["retidy".into()] }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        // Specified direct commands are direct
        assert!(layout.is_direct(Command::SaveGame), "SaveGame should be direct");
        assert!(layout.is_direct(Command::Quit), "Quit should be direct");
        // Recenter is NOT in custom direct list
        assert!(!layout.is_direct(Command::Recenter), "Recenter should NOT be direct with custom list");
        // Unknown command produces a warning
        assert!(!warnings.is_empty(), "unknown command in direct should produce warning");
        assert!(warnings.iter().any(|w| w.contains("not_a_command")), "warning should mention not_a_command");
    }

    #[test]
    fn hotkey_layout_resolve_unknown_group_command_dropped() {
        use crate::config::{HotkeysConfig, HotkeyGroupConfig};
        let cfg = HotkeysConfig {
            prefix: None,
            direct: None,
            group: vec![HotkeyGroupConfig {
                title: "MyGroup".into(),
                commands: vec!["retidy".into(), "totally_fake_cmd".into()],
            }],
        };
        let (layout, warnings) = HotkeyLayout::resolve(&cfg);
        assert_eq!(layout.groups.len(), 1);
        assert_eq!(layout.groups[0].1.len(), 1, "unknown command should be dropped from group");
        assert_eq!(layout.groups[0].1[0], Command::Retidy);
        assert!(!warnings.is_empty(), "unknown group command should produce warning");
        assert!(warnings.iter().any(|w| w.contains("totally_fake_cmd")));
    }

    #[test]
    fn backtab_keyevent_maps_to_cycle_layout_reverse() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        use crate::input::{key_to_action, Action};
        use crate::state::AppState;

        let mut state = AppState::default();
        // BackTab is typically delivered with no modifiers.
        let backtab = KeyEvent {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        // In Game focus: BackTab falls through game_key_to_action → Global lookup → CycleLayoutReverse.
        state.focus = crate::state::Focus::Game;
        let action = key_to_action(&state, backtab);
        assert!(
            matches!(action, Action::CycleLayoutReverse),
            "BackTab in Game focus should produce CycleLayoutReverse, got {:?}",
            action
        );

        // In Map focus: Map context lookup falls through to Global then checks is_direct.
        // CycleLayoutReverse is in the direct set so it fires without the dialog.
        state.focus = crate::state::Focus::Map;
        let action_map = key_to_action(&state, backtab);
        assert!(
            matches!(action_map, Action::CycleLayoutReverse),
            "BackTab in Map focus should produce CycleLayoutReverse, got {:?}",
            action_map
        );
    }

    #[test]
    fn backtab_keyspec_label_is_shift_tab() {
        let spec = KeySpec { code: KeyCode::BackTab, ctrl: false, shift: false, alt: false };
        assert_eq!(spec.label(), "Shift+Tab");
    }

    #[test]
    fn shift_tab_parses_to_backtab() {
        let spec: KeySpec = "shift+tab".parse().unwrap();
        assert_eq!(spec.code, KeyCode::BackTab);
        // shift flag should be false (BackTab encodes the shift itself)
        assert!(!spec.shift);
    }

    #[test]
    fn backtab_token_parses_to_backtab() {
        let spec: KeySpec = "backtab".parse().unwrap();
        assert_eq!(spec.code, KeyCode::BackTab);
    }

    #[test]
    fn cycle_layout_reverse_command_wiring() {
        assert_eq!(Command::CycleLayoutReverse.name(), "cycle_layout_reverse");
        assert_eq!(Command::CycleLayoutReverse.label(), "layout back");
        assert_eq!(Command::CycleLayoutReverse.context(), Context::Global);
        assert!(matches!(Command::CycleLayoutReverse.to_action(), Action::CycleLayoutReverse));
    }

    #[test]
    fn reset_game_command_wiring() {
        assert_eq!(Command::ResetGame.name(), "reset_game");
        assert_eq!(Command::ResetGame.label(), "reset game");
        assert_eq!(Command::ResetGame.context(), Context::Global);
        assert!(matches!(Command::ResetGame.to_action(), Action::ResetGame));
        // F5 is the default key
        let km = KeyMap::default();
        let f5 = KeySpec { code: KeyCode::F(5), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&f5, Context::Global), Some(Command::ResetGame));
    }

    #[test]
    fn open_history_command_wiring() {
        assert_eq!(Command::OpenHistory.name(), "open_history");
        assert_eq!(Command::OpenHistory.label(), "history");
        assert_eq!(Command::OpenHistory.context(), Context::Global);
        assert!(matches!(Command::OpenHistory.to_action(), Action::OpenHistory));
        // F4 is the default key.
        let km = KeyMap::default();
        let f4 = KeySpec { code: KeyCode::F(4), ctrl: false, shift: false, alt: false };
        assert_eq!(km.lookup(&f4, Context::Global), Some(Command::OpenHistory));
        // It appears in the Files hotkey group.
        let layout = HotkeyLayout::default();
        let files = layout.groups.iter().find(|(t, _)| t == "Files").expect("Files group");
        assert!(files.1.contains(&Command::OpenHistory), "OpenHistory in Files group");
    }

    #[test]
    fn reset_game_in_files_dialog_group() {
        let layout = HotkeyLayout::default();
        let files_group = layout.groups.iter().find(|(title, _)| title == "Files");
        assert!(files_group.is_some(), "Files group should exist");
        let (_, cmds) = files_group.unwrap();
        assert!(cmds.contains(&Command::ResetGame), "ResetGame should be in Files group");
    }

    #[test]
    fn toggle_inventory_key_is_v_and_routes_to_action() {
        let km = KeyMap::default();
        let spec = KeySpec { code: KeyCode::Char('v'), ctrl: false, shift: false, alt: false };
        let cmd = km.lookup(&spec, Context::Global);
        assert_eq!(cmd, Some(Command::ToggleInventory), "v should be bound to ToggleInventory");
        assert!(matches!(Command::ToggleInventory.to_action(), Action::ToggleInventory));
    }

    #[test]
    fn toggle_inventory_in_view_group() {
        let layout = HotkeyLayout::default();
        let view_group = layout.groups.iter().find(|(title, _)| title == "View");
        assert!(view_group.is_some(), "View group should exist");
        let (_, cmds) = view_group.unwrap();
        assert!(cmds.contains(&Command::ToggleInventory), "ToggleInventory should be in View group");
    }

    #[test]
    fn apply_action_toggle_inventory_flips_bool() {
        use mapper::mapper::Mapper;
        use crate::input::apply_action;
        use crate::state::AppState;
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        assert!(!state.show_inventory);
        apply_action(Action::ToggleInventory, &mut state, &mut mapper);
        assert!(state.show_inventory);
        apply_action(Action::ToggleInventory, &mut state, &mut mapper);
        assert!(!state.show_inventory);
    }

    // ── Item 2: ZoomReset command wiring ─────────────────────────────────────

    #[test]
    fn zoom_reset_command_wiring() {
        assert_eq!(Command::ZoomReset.name(), "zoom_reset");
        assert_eq!(Command::ZoomReset.label(), "zoom reset");
        assert_eq!(Command::ZoomReset.context(), Context::Map);
        assert!(matches!(Command::ZoomReset.to_action(), Action::ZoomReset));
    }

    #[test]
    fn zoom_reset_bound_to_zero_key() {
        let km = KeyMap::default();
        let zero = KeySpec { code: KeyCode::Char('0'), ctrl: false, shift: false, alt: false };
        assert_eq!(
            km.lookup(&zero, Context::Map),
            Some(Command::ZoomReset),
            "'0' key must be bound to ZoomReset in Map context"
        );
    }

    #[test]
    fn zoom_reset_is_in_direct_set() {
        let layout = HotkeyLayout::default();
        assert!(
            layout.is_direct(Command::ZoomReset),
            "ZoomReset must be in the direct set (accessible without the hotkey dialog)"
        );
    }

    #[test]
    fn zoom_reset_action_resets_level() {
        use mapper::mapper::Mapper;
        use crate::input::apply_action;
        use crate::state::{AppState, Zoom};
        let mut state = AppState::default();
        let mut mapper = Mapper::default();
        // Zoom all the way out first.
        for _ in 0..8 {
            apply_action(Action::ZoomOut, &mut state, &mut mapper);
        }
        assert!(matches!(state.zoom, Zoom::Overview));
        // Reset
        apply_action(Action::ZoomReset, &mut state, &mut mapper);
        assert_eq!(state.zoom_level, 7, "ZoomReset must restore zoom_level to 7");
        assert!(matches!(state.zoom, Zoom::Boxes), "ZoomReset must restore Zoom::Boxes");
    }

    /// Every default binding for a DIRECT command (excluding save_game and
    /// restore_game, which intentionally use Ctrl) must have ctrl=false and
    /// shift=false. This invariant ensures that direct commands are reachable
    /// with plain (unmodified) keystrokes.
    #[test]
    fn direct_default_bindings_have_no_modifiers() {
        let km = KeyMap::default();
        let layout = HotkeyLayout::default();

        // Commands excluded from this invariant by design.
        let excluded = [Command::SaveGame, Command::RestoreGame];

        let mut violations: Vec<String> = Vec::new();
        for (spec, cmd, _ctx) in &km.bindings {
            if excluded.contains(cmd) {
                continue;
            }
            if !layout.is_direct(*cmd) {
                continue;
            }
            if spec.ctrl || spec.shift {
                violations.push(format!(
                    "{} ({}): ctrl={} shift={}",
                    cmd.name(),
                    spec.label(),
                    spec.ctrl,
                    spec.shift,
                ));
            }
        }

        assert!(
            violations.is_empty(),
            "direct bindings with modifier keys found (should be plain):\n  {}",
            violations.join("\n  ")
        );
    }
}
