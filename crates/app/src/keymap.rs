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
    AnimateTidy,
    ToggleAlignment,
    TogglePortalLabels,
    /// Toggle between Game and Map focus.
    ToggleFocus,
    /// Open or close the full-screen help overlay.
    ToggleHelp,
    // ── Nudge (Global, Ctrl+Arrows) ───────────────────────────────────────────
    NudgeLeft,
    NudgeRight,
    NudgeUp,
    NudgeDown,

    // ── Map ───────────────────────────────────────────────────────────────────
    OpenGallery,
    ZoomIn,
    ZoomOut,
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
            Command::AnimateTidy => Action::AnimateTidy,
            Command::ToggleAlignment => Action::ToggleAlignment,
            Command::TogglePortalLabels => Action::TogglePortalLabels,
            Command::ToggleFocus => Action::ToggleFocus,
            Command::ToggleHelp => Action::ToggleHelp,
            Command::NudgeLeft => Action::NudgeSelected(-1, 0),
            Command::NudgeRight => Action::NudgeSelected(1, 0),
            Command::NudgeUp => Action::NudgeSelected(0, -1),
            Command::NudgeDown => Action::NudgeSelected(0, 1),
            Command::OpenGallery => Action::OpenGallery,
            Command::ZoomIn => Action::ZoomIn,
            Command::ZoomOut => Action::ZoomOut,
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
            Command::AnimateTidy => "animate_tidy",
            Command::ToggleAlignment => "toggle_alignment",
            Command::TogglePortalLabels => "toggle_portal_labels",
            Command::ToggleFocus => "toggle_focus",
            Command::ToggleHelp => "toggle_help",
            Command::NudgeLeft => "nudge_left",
            Command::NudgeRight => "nudge_right",
            Command::NudgeUp => "nudge_up",
            Command::NudgeDown => "nudge_down",
            Command::OpenGallery => "open_gallery",
            Command::ZoomIn => "zoom_in",
            Command::ZoomOut => "zoom_out",
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
            Command::AnimateTidy => "animate tidy",
            Command::ToggleAlignment => "alignment",
            Command::TogglePortalLabels => "portals",
            Command::ToggleFocus => "focus",
            Command::ToggleHelp => "help",
            Command::NudgeLeft => "nudge left",
            Command::NudgeRight => "nudge right",
            Command::NudgeUp => "nudge up",
            Command::NudgeDown => "nudge down",
            Command::OpenGallery => "gallery",
            Command::ZoomIn => "zoom in",
            Command::ZoomOut => "zoom out",
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
            | Command::AnimateTidy
            | Command::ToggleAlignment
            | Command::TogglePortalLabels
            | Command::ToggleFocus
            | Command::ToggleHelp
            | Command::NudgeLeft
            | Command::NudgeRight
            | Command::NudgeUp
            | Command::NudgeDown => Context::Global,

            Command::OpenGallery
            | Command::ZoomIn
            | Command::ZoomOut
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
    Command::AnimateTidy,
    Command::ToggleAlignment,
    Command::TogglePortalLabels,
    Command::ToggleFocus,
    Command::ToggleHelp,
    Command::NudgeLeft,
    Command::NudgeRight,
    Command::NudgeUp,
    Command::NudgeDown,
    Command::OpenGallery,
    Command::ZoomIn,
    Command::ZoomOut,
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
    /// Examples: "Ctrl+S", "Shift+←", "h", "F1", "Space".
    pub fn label(&self) -> String {
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
            "tab" => KeyCode::Tab,
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

        // Ctrl+Arrows → Nudge
        bind!(ctrl(Left), Command::NudgeLeft, Context::Global);
        bind!(ctrl(Right), Command::NudgeRight, Context::Global);
        bind!(ctrl(Up), Command::NudgeUp, Context::Global);
        bind!(ctrl(Down), Command::NudgeDown, Context::Global);

        // Help toggle: F1 global, ? in map
        bind!(plain(F(1)), Command::ToggleHelp, Context::Global);

        // ── Map ───────────────────────────────────────────────────────────────
        // Pan: plain arrows + Shift+arrows + hjkl (all three sets)
        bind!(plain(Left), Command::PanLeft, Context::Map);
        bind!(plain(Right), Command::PanRight, Context::Map);
        bind!(plain(Up), Command::PanUp, Context::Map);
        bind!(plain(Down), Command::PanDown, Context::Map);

        bind!(shift(Left), Command::PanLeft, Context::Map);
        bind!(shift(Right), Command::PanRight, Context::Map);
        bind!(shift(Up), Command::PanUp, Context::Map);
        bind!(shift(Down), Command::PanDown, Context::Map);

        bind!(plain(Char('h')), Command::PanLeft, Context::Map);
        bind!(plain(Char('l')), Command::PanRight, Context::Map);
        bind!(plain(Char('k')), Command::PanUp, Context::Map);
        bind!(plain(Char('j')), Command::PanDown, Context::Map);

        // Zoom: + / = / Shift++ (three specs for ZoomIn)
        bind!(plain(Char('+')), Command::ZoomIn, Context::Map);
        bind!(plain(Char('=')), Command::ZoomIn, Context::Map);
        bind!(shift(Char('+')), Command::ZoomIn, Context::Map);
        bind!(plain(Char('-')), Command::ZoomOut, Context::Map);

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
        // ? → ToggleHelp (in map context)
        bind!(plain(Char('?')), Command::ToggleHelp, Context::Map);

        // ── Anim ──────────────────────────────────────────────────────────────
        // Pan in anim: Shift+arrows + hjkl (plain arrows go to step)
        bind!(shift(Left), Command::PanLeft, Context::Anim);
        bind!(shift(Right), Command::PanRight, Context::Anim);
        bind!(shift(Up), Command::PanUp, Context::Anim);
        bind!(shift(Down), Command::PanDown, Context::Anim);
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
    pub fn resolve(cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String>) {
        let mut km = KeyMap::default();
        let mut warnings: Vec<String> = Vec::new();

        for (name, value) in &cfg.overrides {
            // Resolve command name.
            let cmd = match Command::from_name(name) {
                Some(c) => c,
                None => {
                    warnings.push(format!("keymap: unknown command '{name}'; skipped"));
                    continue;
                }
            };

            // Parse comma-separated list of KeySpecs.
            let specs: Vec<KeySpec> = {
                let mut ok = Vec::new();
                for token in value.split(',') {
                    let token = token.trim();
                    match token.parse::<KeySpec>() {
                        Ok(s) => ok.push(s),
                        Err(e) => {
                            warnings.push(format!(
                                "keymap: '{name}': cannot parse '{token}': {e}; skipped"
                            ));
                        }
                    }
                }
                ok
            };

            if specs.is_empty() {
                continue;
            }

            let ctx = cmd.context();

            // Remove the command's existing default bindings in this context.
            km.bindings.retain(|(_, c, cx)| !(*c == cmd && *cx == ctx));

            // Add new specs, rejecting any that conflict with another command.
            for spec in specs {
                let conflict = km.bindings.iter().any(|(s, c, cx)| {
                    s == &spec && *c != cmd && (*cx == ctx || (ctx == Context::Map && *cx == Context::Global))
                });
                if conflict {
                    warnings.push(format!(
                        "keymap: '{name}': '{}' already bound to another command; kept default",
                        spec.label()
                    ));
                } else {
                    km.bindings.push((spec, cmd, ctx));
                }
            }
        }

        (km, warnings)
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
        // multi-binding default preserved:
        assert_eq!(km.lookup(&g(Left, false, true), Context::Map), Some(Command::PanLeft));
    }

    // Task 3b: resolve applies overrides and rejects conflicts
    #[test]
    fn resolve_applies_override_and_rejects_conflict() {
        let mut cfg = crate::config::KeymapConfig::default();
        cfg.overrides.insert("zoom_in".into(), "z".into());
        let (km, warns) = KeyMap::resolve(&cfg);
        use KeyCode::*;
        assert_eq!(
            km.lookup(&KeySpec { code: Char('z'), ctrl: false, shift: false, alt: false }, Context::Map),
            Some(Command::ZoomIn)
        );
        assert!(warns.is_empty());
        // binding to an already-used key in the same context is rejected:
        let mut cfg2 = crate::config::KeymapConfig::default();
        cfg2.overrides.insert("zoom_in".into(), "n".into()); // 'n' is SelectNext in Map
        let (km2, warns2) = KeyMap::resolve(&cfg2);
        assert_eq!(
            km2.lookup(&KeySpec { code: Char('n'), ctrl: false, shift: false, alt: false }, Context::Map),
            Some(Command::SelectNext)
        );
        assert!(!warns2.is_empty());
    }
}
