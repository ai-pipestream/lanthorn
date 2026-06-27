//! Slash-command parser: `parse`, curated table, fallback to `keymap::Command`,
//! `slash_names`, and `help_text`.
//!
//! `parse` receives the input AFTER the leading prefix character has been
//! stripped. It does not know what the prefix was.

use crate::input::Action;
use crate::keymap::Context;

// ── SlashOutcome ──────────────────────────────────────────────────────────────

/// The result of parsing a slash-command body.
#[derive(Debug, Clone, PartialEq)]
pub enum SlashOutcome {
    /// Dispatch an action (pan, zoom, center, tidy, layer, …).
    Action(Action),
    /// Show an informational message on the status line (no effect).
    Message(String),
    /// Show an error message on the status line.
    Error(String),
    /// Print `/help` lines to the transcript as Meta entries.
    Help,
    /// Save the game; optionally to a named slot.
    Save(Option<String>),
    /// Load a save; optionally a named slot.
    Load(Option<String>),
    /// Reset the app; `map: true` also clears the automapper state.
    Reset { map: bool },
    /// Quit the application.
    Quit,
    /// Search the transcript; `None` repeats the last search.
    Search(Option<String>),
    /// Filter the transcript by category.
    Filter(TranscriptFilterArg),
    /// Export the visible transcript; `None` uses the default path.
    Export(Option<String>),
    /// Open the Hints panel (caller-handled, like Save/Load). Task D wires the real behavior.
    OpenHints,
    /// Show per-command detail for `help <name>`.
    HelpCommand(String),
}

// ── TranscriptFilterArg ───────────────────────────────────────────────────────

/// Argument for the `/filter` command. `main.rs` maps this to `state::TranscriptFilter`.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptFilterArg {
    Both,
    Story,
    Meta,
}

// ── Category ──────────────────────────────────────────────────────────────────

/// User-facing grouping for `/help` and the hotkey dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Game,
    Map,
    View,
    Transcript,
    Style,
    Export,
    Animation,
    Help,
}

impl Category {
    pub const ORDER: [Category; 8] = [
        Category::Game,
        Category::Map,
        Category::View,
        Category::Transcript,
        Category::Style,
        Category::Export,
        Category::Animation,
        Category::Help,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Game => "Game",
            Category::Map => "Map",
            Category::View => "View",
            Category::Transcript => "Transcript",
            Category::Style => "Style",
            Category::Export => "Export",
            Category::Animation => "Animation",
            Category::Help => "Help",
        }
    }
}

// ── CommandSpec registry ──────────────────────────────────────────────────────

pub struct CommandSpec {
    pub name: &'static str,
    pub category: Category,
    pub context: Context,
    pub usage: &'static str,
    pub description: &'static str,
    pub dispatch: fn(&[&str]) -> SlashOutcome,
}

fn err(s: impl Into<String>) -> SlashOutcome { SlashOutcome::Error(s.into()) }

pub static COMMANDS: &[CommandSpec] = &[
    // ── Game ──────────────────────────────────────────────────────────────
    CommandSpec { name: "save-game", category: Category::Game, context: Context::Global,
        usage: "save-game [name]", description: "save the game, optionally to a named slot",
        dispatch: |a| SlashOutcome::Save(a.first().map(|s| s.to_string())) },
    CommandSpec { name: "load-game", category: Category::Game, context: Context::Global,
        usage: "load-game [name]", description: "load a save, optionally a named slot",
        dispatch: |a| SlashOutcome::Load(a.first().map(|s| s.to_string())) },
    CommandSpec { name: "reset-game", category: Category::Game, context: Context::Global,
        usage: "reset-game [map]", description: "restart the game; 'reset-game map' also clears the map",
        dispatch: |a| SlashOutcome::Reset { map: a.first().copied() == Some("map") } },
    CommandSpec { name: "quit", category: Category::Game, context: Context::Global,
        usage: "quit", description: "exit babelmap",
        dispatch: |_| SlashOutcome::Quit },
    CommandSpec { name: "open-hints", category: Category::Game, context: Context::Global,
        usage: "open-hints", description: "open the hints panel",
        dispatch: |_| SlashOutcome::OpenHints },
    CommandSpec { name: "open-history", category: Category::Game, context: Context::Global,
        usage: "open-history", description: "open the rewind/replay history",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenHistory) },
    CommandSpec { name: "open-verb-menu", category: Category::Game, context: Context::Global,
        usage: "open-verb-menu", description: "open the verb/item palette",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenVerbMenu) },
    CommandSpec { name: "open-saves", category: Category::Game, context: Context::Global,
        usage: "open-saves", description: "open the saves manager",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenSaves) },

    // ── Map ───────────────────────────────────────────────────────────────
    CommandSpec { name: "pan-map", category: Category::Map, context: Context::Map,
        usage: "pan-map <dx> <dy>", description: "pan the map by dx columns and dy rows",
        dispatch: |a| {
            let dx = a.first().and_then(|s| s.parse::<i32>().ok());
            let dy = a.get(1).and_then(|s| s.parse::<i32>().ok());
            match (dx, dy) {
                (Some(x), Some(y)) => SlashOutcome::Action(crate::input::Action::Pan(x, y)),
                _ => err("pan-map requires two integers (e.g. pan-map -3 0)"),
            }
        } },
    CommandSpec { name: "zoom-map", category: Category::Map, context: Context::Map,
        usage: "zoom-map in|out|reset|<n>", description: "zoom the map in/out, reset, or step by signed n",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("in") => SlashOutcome::Action(Action::ZoomIn),
                Some("out") => SlashOutcome::Action(Action::ZoomOut),
                Some("reset") => SlashOutcome::Action(Action::ZoomReset),
                Some(s) => match s.parse::<i32>() {
                    Ok(0) => SlashOutcome::Action(Action::ZoomReset),
                    Ok(n) if n > 0 => SlashOutcome::Action(Action::ZoomIn),
                    Ok(_) => SlashOutcome::Action(Action::ZoomOut),
                    Err(_) => err(format!("zoom-map: expected in|out|reset|<integer>, got '{s}'")),
                },
                None => err("zoom-map requires an argument: in|out|reset|<n>"),
            }
        } },
    CommandSpec { name: "center-map", category: Category::Map, context: Context::Map,
        usage: "center-map", description: "re-center the map on the selected room",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::Recenter) },
    CommandSpec { name: "tidy-map", category: Category::Map, context: Context::Map,
        usage: "tidy-map", description: "re-run the layout tidy",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::Retidy) },
    CommandSpec { name: "cycle-layer", category: Category::Map, context: Context::Map,
        usage: "cycle-layer next|prev|<n>", description: "switch map layer; n is a signed delta",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("next") => SlashOutcome::Action(Action::CycleLayer(1)),
                Some("prev") => SlashOutcome::Action(Action::CycleLayer(-1)),
                Some(s) => match s.parse::<i32>() {
                    Ok(n) => SlashOutcome::Action(Action::CycleLayer(n)),
                    Err(_) => err(format!("cycle-layer: expected next|prev|<integer delta>, got '{s}'")),
                },
                None => err("cycle-layer requires an argument: next|prev|<n>"),
            }
        } },
    CommandSpec { name: "select-room", category: Category::Map, context: Context::Map,
        usage: "select-room next|prev", description: "move the room selection",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("next") => SlashOutcome::Action(Action::SelectNext),
                Some("prev") => SlashOutcome::Action(Action::SelectPrev),
                _ => err("select-room requires an argument: next|prev"),
            }
        } },
    CommandSpec { name: "nudge-room", category: Category::Map, context: Context::Map,
        usage: "nudge-room <dx> <dy>", description: "nudge the selected room by dx, dy cells",
        dispatch: |a| {
            let dx = a.first().and_then(|s| s.parse::<i32>().ok());
            let dy = a.get(1).and_then(|s| s.parse::<i32>().ok());
            match (dx, dy) {
                (Some(x), Some(y)) => SlashOutcome::Action(crate::input::Action::NudgeSelected(x, y)),
                _ => err("nudge-room requires two integers (e.g. nudge-room -1 0)"),
            }
        } },
    CommandSpec { name: "rename-room", category: Category::Map, context: Context::Map,
        usage: "rename-room", description: "rename the selected room",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::RenameRoom) },
    CommandSpec { name: "rename-layer", category: Category::Map, context: Context::Map,
        usage: "rename-layer", description: "rename the current layer",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::RenameLayer) },
    CommandSpec { name: "edit-notes", category: Category::Map, context: Context::Map,
        usage: "edit-notes", description: "edit the selected room's notes",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::EditNotes) },
    CommandSpec { name: "delete-connection", category: Category::Map, context: Context::Map,
        usage: "delete-connection", description: "delete the selected connection",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::DeleteSelectedConnection) },
    CommandSpec { name: "relabel-edge", category: Category::Map, context: Context::Map,
        usage: "relabel-edge", description: "relabel the selected edge",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::RelabelSelectedEdge) },
    CommandSpec { name: "peel-layer", category: Category::Map, context: Context::Map,
        usage: "peel-layer", description: "peel the selected layer into its own view",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::PeelLayer) },
    CommandSpec { name: "merge-layer", category: Category::Map, context: Context::Map,
        usage: "merge-layer", description: "merge the selected layer down",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::MergeLayer) },
    CommandSpec { name: "toggle-inspector", category: Category::Map, context: Context::Map,
        usage: "toggle-inspector", description: "toggle the room-inspector overlay",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleInspector) },
    CommandSpec { name: "toggle-room-numbers", category: Category::Map, context: Context::Global,
        usage: "toggle-room-numbers", description: "toggle room-number labels",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleRoomNumbers) },
    CommandSpec { name: "toggle-loc-method", category: Category::Map, context: Context::Global,
        usage: "toggle-loc-method", description: "toggle the room-detection-method indicator",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleLocMethod) },
    CommandSpec { name: "toggle-alignment", category: Category::Map, context: Context::Global,
        usage: "toggle-alignment", description: "toggle alignment guides",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleAlignment) },
    CommandSpec { name: "toggle-portal-labels", category: Category::Map, context: Context::Global,
        usage: "toggle-portal-labels", description: "toggle portal labels",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::TogglePortalLabels) },
    CommandSpec { name: "open-gallery", category: Category::Map, context: Context::Map,
        usage: "open-gallery", description: "open the symbol gallery",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenGallery) },

    // ── View ──────────────────────────────────────────────────────────────
    CommandSpec { name: "cycle-layout", category: Category::View, context: Context::Global,
        usage: "cycle-layout [reverse]", description: "cycle the pane layout; 'reverse' cycles backward",
        dispatch: |a| {
            use crate::input::Action;
            if a.first().copied() == Some("reverse") {
                SlashOutcome::Action(Action::CycleLayoutReverse)
            } else {
                SlashOutcome::Action(Action::CycleLayout)
            }
        } },
    CommandSpec { name: "toggle-focus", category: Category::View, context: Context::Global,
        usage: "toggle-focus", description: "switch focus between panes",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleFocus) },
    CommandSpec { name: "toggle-inventory", category: Category::View, context: Context::Global,
        usage: "toggle-inventory", description: "toggle the inventory strip",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleInventory) },
    CommandSpec { name: "toggle-status-bar", category: Category::View, context: Context::Global,
        usage: "toggle-status-bar", description: "toggle the status/score bar",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleStatusBar) },

    // ── Transcript ────────────────────────────────────────────────────────
    CommandSpec { name: "search-transcript", category: Category::Transcript, context: Context::Global,
        usage: "search-transcript [query]", description: "search the transcript; no query repeats the last search",
        dispatch: |a| if a.is_empty() { SlashOutcome::Search(None) } else { SlashOutcome::Search(Some(a.join(" "))) } },
    CommandSpec { name: "filter-transcript", category: Category::Transcript, context: Context::Global,
        usage: "filter-transcript story|meta|both", description: "filter the transcript by category",
        dispatch: |a| match a.first().copied() {
            Some("story") => SlashOutcome::Filter(TranscriptFilterArg::Story),
            Some("meta")  => SlashOutcome::Filter(TranscriptFilterArg::Meta),
            Some("both")  => SlashOutcome::Filter(TranscriptFilterArg::Both),
            _ => err("filter-transcript: use story | meta | both"),
        } },
    CommandSpec { name: "export-transcript", category: Category::Transcript, context: Context::Global,
        usage: "export-transcript [file]", description: "export the visible transcript; default path when omitted",
        dispatch: |a| SlashOutcome::Export(a.first().map(|s| s.to_string())) },

    // ── Style ─────────────────────────────────────────────────────────────
    CommandSpec { name: "open-style-editor", category: Category::Style, context: Context::Global,
        usage: "open-style-editor", description: "open the live style editor",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenStyleEditor) },
    CommandSpec { name: "open-config", category: Category::Style, context: Context::Global,
        usage: "open-config", description: "open the settings screen",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenConfig) },
    CommandSpec { name: "reload-style", category: Category::Style, context: Context::Global,
        usage: "reload-style", description: "reload style.toml from disk",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ReloadStyle) },
    CommandSpec { name: "create-game-style", category: Category::Style, context: Context::Global,
        usage: "create-game-style", description: "scaffold a per-game style file",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::GameStyle) },
    CommandSpec { name: "toggle-watch", category: Category::Style, context: Context::Global,
        usage: "toggle-watch", description: "toggle live style-file watching",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleWatch) },

    // ── Export ────────────────────────────────────────────────────────────
    CommandSpec { name: "export-svg", category: Category::Export, context: Context::Global,
        usage: "export-svg", description: "export the map as SVG",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ExportSvg) },
    CommandSpec { name: "export-dot", category: Category::Export, context: Context::Global,
        usage: "export-dot", description: "export the map as Graphviz DOT",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ExportDot) },
    CommandSpec { name: "export-dump", category: Category::Export, context: Context::Global,
        usage: "export-dump", description: "dump the map structure",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ExportDump) },

    // ── Animation ─────────────────────────────────────────────────────────
    CommandSpec { name: "animate-tidy", category: Category::Animation, context: Context::Global,
        usage: "animate-tidy", description: "animate a tidy pass",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::AnimateTidy) },
    CommandSpec { name: "anim-step", category: Category::Animation, context: Context::Anim,
        usage: "anim-step forward|back", description: "step the animation one frame",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                Some("forward") => SlashOutcome::Action(Action::AnimStep(1)),
                Some("back") => SlashOutcome::Action(Action::AnimStep(-1)),
                _ => err("anim-step requires an argument: forward|back"),
            }
        } },
    CommandSpec { name: "anim-play", category: Category::Animation, context: Context::Anim,
        usage: "anim-play", description: "toggle animation play/pause",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::AnimTogglePlay) },
    CommandSpec { name: "anim-exit", category: Category::Animation, context: Context::Anim,
        usage: "anim-exit", description: "exit the animation view",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::AnimExit) },

    // ── Help ──────────────────────────────────────────────────────────────
    CommandSpec { name: "help", category: Category::Help, context: Context::Global,
        usage: "help [command]", description: "list all commands by category; with a name, show one command's detail",
        dispatch: |_| SlashOutcome::Help },
];

pub fn find_command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

// ── parse ─────────────────────────────────────────────────────────────────────

/// Parse a slash-command body (the text AFTER the leading prefix, e.g. `/`).
///
/// `prefix` is the configured command prefix character, used only in user-facing
/// error/help display strings. Routing and matching logic is unaffected.
///
/// Routes entirely through the `COMMANDS` registry. Special case:
/// `search-transcript` passes the raw remainder of the line preserving internal
/// whitespace. All other commands receive split tokens.
pub fn parse(body: &str, prefix: char) -> SlashOutcome {
    parse_in_context(body, prefix, Context::Global)
}

pub fn parse_in_context(body: &str, prefix: char, ctx: Context) -> SlashOutcome {
    let Some(t0) = body.split_whitespace().next() else {
        return SlashOutcome::Error(format!("type {prefix}help for commands"));
    };

    // help: bare `help` → Help; `help <name>` → HelpCommand.
    if t0 == "help" {
        let rest = body.split_whitespace().nth(1);
        return match rest {
            Some(name) => SlashOutcome::HelpCommand(name.to_string()),
            None => SlashOutcome::Help,
        };
    }

    // search-transcript: preserve internal whitespace in the query.
    if t0 == "search-transcript" {
        let remainder = body[t0.len()..].trim_start_matches(' ').trim_end();
        return if remainder.is_empty() { SlashOutcome::Search(None) }
               else { SlashOutcome::Search(Some(remainder.to_string())) };
    }

    let Some(spec) = find_command(t0) else {
        return SlashOutcome::Error(format!("unknown command: {prefix}{t0} — try {prefix}help"));
    };

    if spec.context == Context::Anim && ctx != Context::Anim {
        return SlashOutcome::Error(format!("{} is only available during animation playback", spec.name));
    }

    let tokens: Vec<&str> = body.split_whitespace().collect();
    (spec.dispatch)(&tokens[1..])
}

// ── slash_names ───────────────────────────────────────────────────────────────

/// All known slash-command names (for Tab autocomplete).
///
/// Returns the registry command names.
pub fn slash_names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}

// ── help_text / help_for_command ──────────────────────────────────────────────

/// Lines to display when the user types the help command.
///
/// `prefix` is the configured command prefix character used in all display strings.
/// Commands are grouped by category in `Category::ORDER` order, sorted by name
/// within each group.
pub fn help_text(prefix: char) -> Vec<String> {
    let mut lines = vec![
        format!("Slash commands (type {prefix}<command> [args]):"),
        String::new(),
    ];
    for cat in Category::ORDER {
        let mut group: Vec<&CommandSpec> = COMMANDS.iter().filter(|c| c.category == cat).collect();
        if group.is_empty() { continue; }
        group.sort_by_key(|c| c.name);
        lines.push(format!("{}:", cat.title()));
        for c in group {
            lines.push(format!("  {prefix}{}  — {}", c.usage, c.description));
        }
        lines.push(String::new());
    }
    lines
}

/// Lines to display for a single command's detail.
///
/// Returns the command's usage and description, or an unknown-command message.
pub fn help_for_command(prefix: char, name: &str) -> Vec<String> {
    match find_command(name) {
        Some(c) => vec![format!("  {prefix}{}  — {}", c.usage, c.description)],
        None => vec![format!("unknown command: {prefix}{name} — try {prefix}help")],
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_registry_and_errors() {
        use crate::input::Action;
        assert!(matches!(parse("pan-map -1 0", '/'), SlashOutcome::Action(Action::Pan(-1, 0))));
        assert!(matches!(parse("pan-map 0 2", '/'), SlashOutcome::Action(Action::Pan(0, 2))));
        assert!(matches!(parse("zoom-map reset", '/'), SlashOutcome::Action(Action::ZoomReset)));
        assert!(matches!(parse("save-game foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("save-game", '/'), SlashOutcome::Save(None)));
        assert!(matches!(parse("reset-game map", '/'), SlashOutcome::Reset { map: true }));
        assert!(matches!(parse("reset-game", '/'), SlashOutcome::Reset { map: false }));
        assert!(matches!(parse("quit", '/'), SlashOutcome::Quit));
        assert!(matches!(parse("help", '/'), SlashOutcome::Help));
        // in registry:
        assert!(matches!(parse("open-config", '/'), SlashOutcome::Action(_)));
        // errors:
        assert!(matches!(parse("panh", '/'), SlashOutcome::Error(_)));   // no longer in registry
        assert!(matches!(parse("nope", '/'), SlashOutcome::Error(_)));   // unknown
        assert!(matches!(parse("", '/'), SlashOutcome::Error(_)));       // bare prefix
        // old short names now error (clean break):
        assert!(matches!(parse("save", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("pan", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("zoom", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn slash_names_returns_registry() {
        let n = slash_names();
        assert!(n.iter().any(|s| s == "pan-map")); // registry name
        assert!(n.iter().any(|s| s == "open-config")); // registry name
        assert!(!n.iter().any(|s| s == "panh")); // old curated name, not in registry
    }

    #[test]
    fn help_text_uses_prefix() {
        let lines = help_text('/');
        assert!(lines[0].contains('/'));
        let lines_semi = help_text(';');
        assert!(lines_semi[0].contains(';'));
        assert!(!lines_semi[0].contains('/'));
    }

    #[test]
    fn help_text_lists_registry_commands() {
        // Every registry command's usage must appear in /help.
        let lines = help_text('/');
        assert!(
            lines.iter().any(|l| l.contains("/open-config")),
            "open-config should appear in /help"
        );
        assert!(
            lines.iter().any(|l| l.contains("/save-game")),
            "save-game should appear in /help"
        );
        assert!(
            lines.iter().any(|l| l.contains("/zoom-map")),
            "zoom-map should appear in /help"
        );
    }

    #[test]
    fn slash_hint_parses_to_open_hints() {
        assert!(matches!(crate::slash::parse("open-hints", '/'), crate::slash::SlashOutcome::OpenHints));
        // old short names no longer resolve (clean break):
        assert!(matches!(crate::slash::parse("hint", '/'), crate::slash::SlashOutcome::Error(_)));
        assert!(matches!(crate::slash::parse("hints", '/'), crate::slash::SlashOutcome::Error(_)));
    }

    #[test]
    fn slash_style_opens_editor() {
        use crate::input::Action;
        assert!(matches!(parse("open-style-editor", '/'), SlashOutcome::Action(Action::OpenStyleEditor)));
        // old short name no longer resolves (clean break):
        assert!(matches!(parse("style", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn parse_search_filter_export() {
        assert!(matches!(parse("search-transcript twisty maze", '/'), SlashOutcome::Search(Some(q)) if q == "twisty maze"));
        assert!(matches!(parse("search-transcript a  b", '/'), SlashOutcome::Search(Some(q)) if q == "a  b"));
        assert!(matches!(parse("search-transcript", '/'), SlashOutcome::Search(None)));
        assert!(matches!(parse("filter-transcript meta", '/'), SlashOutcome::Filter(TranscriptFilterArg::Meta)));
        assert!(matches!(parse("filter-transcript both", '/'), SlashOutcome::Filter(TranscriptFilterArg::Both)));
        assert!(matches!(parse("filter-transcript nope", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("export-transcript", '/'), SlashOutcome::Export(None)));
        assert!(matches!(parse("export-transcript out.txt", '/'), SlashOutcome::Export(Some(f)) if f == "out.txt"));
        // old short names now error (clean break):
        assert!(matches!(parse("search", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("filter", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("export", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn parse_routes_registry_and_gates_anim() {
        use crate::input::Action;
        use crate::keymap::Context;
        assert!(matches!(parse("pan-map -1 0", '/'), SlashOutcome::Action(Action::Pan(-1, 0))));
        assert!(matches!(parse("zoom-map in", '/'), SlashOutcome::Action(Action::ZoomIn)));
        assert!(matches!(parse("select-room next", '/'), SlashOutcome::Action(Action::SelectNext)));
        assert!(matches!(parse("save-game foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("reset-game map", '/'), SlashOutcome::Reset { map: true }));
        assert!(matches!(parse("quit", '/'), SlashOutcome::Quit));
        // Old short names no longer resolve (clean break).
        assert!(matches!(parse("center", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("panh", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("nope", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("", '/'), SlashOutcome::Error(_)));
        // Context gating: anim-step outside Anim errors; inside Anim it fires.
        assert!(matches!(parse_in_context("anim-step forward", '/', Context::Global), SlashOutcome::Error(_)));
        assert!(matches!(parse_in_context("anim-step forward", '/', Context::Anim), SlashOutcome::Action(Action::AnimStep(1))));
        // search-transcript preserves internal whitespace.
        assert!(matches!(parse("search-transcript a  b", '/'), SlashOutcome::Search(Some(q)) if q == "a  b"));
    }

    #[test]
    fn category_order_and_titles() {
        assert_eq!(Category::ORDER.len(), 8);
        assert_eq!(Category::ORDER[0], Category::Game);
        assert_eq!(Category::ORDER[7], Category::Help);
        assert_eq!(Category::Game.title(), "Game");
        assert_eq!(Category::Animation.title(), "Animation");
    }

    #[test]
    fn registry_is_complete_and_well_formed() {
        use std::collections::HashSet;
        // Names unique.
        let mut seen = HashSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate command name: {}", c.name);
            assert!(!c.usage.is_empty(), "{} has empty usage", c.name);
            assert!(!c.description.is_empty(), "{} has empty description", c.name);
        }
        // Verb-noun lint: every name contains '-' except the whitelist.
        for c in COMMANDS {
            if c.name == "quit" || c.name == "help" { continue; }
            assert!(c.name.contains('-'), "non-verb-noun command name: {}", c.name);
        }
        // Spot-check representative commands exist with the right category.
        let by = |n: &str| COMMANDS.iter().find(|c| c.name == n).expect(n);
        assert_eq!(by("save-game").category, Category::Game);
        assert_eq!(by("zoom-map").category, Category::Map);
        assert_eq!(by("create-game-style").category, Category::Style);
        assert_eq!(by("anim-step").context, Context::Anim);
        // Total count matches the spec table (48 commands: Game 8, Map 20, View 4,
        // Transcript 3, Style 5, Export 3, Animation 4, Help 1).
        assert_eq!(COMMANDS.len(), 48, "registry must match the spec's Full command table");
    }

    #[test]
    fn help_text_grouped_and_per_command() {
        let lines = help_text('/');
        // Category headers appear in order.
        let game_at = lines.iter().position(|l| l.contains("Game")).unwrap();
        let map_at = lines.iter().position(|l| l.contains("Map")).unwrap();
        assert!(game_at < map_at, "Game group precedes Map group");
        // Every command's usage shows up.
        assert!(lines.iter().any(|l| l.contains("/zoom-map")));
        assert!(lines.iter().any(|l| l.contains("/create-game-style")));

        // Per-command detail.
        let one = help_for_command('/', "zoom-map");
        assert!(one.iter().any(|l| l.contains("zoom-map in|out|reset")));
        assert!(one.iter().any(|l| l.contains("zoom the map")));
        let bad = help_for_command('/', "nope");
        assert!(bad.iter().any(|l| l.contains("unknown command")));

        // `help <command>` parses to HelpCommand; bare help to Help.
        assert!(matches!(parse("help", '/'), SlashOutcome::Help));
        assert!(matches!(parse("help zoom-map", '/'), SlashOutcome::HelpCommand(n) if n == "zoom-map"));
    }

    #[test]
    fn help_for_command_round_trip() {
        // The run loop calls help_for_command on a HelpCommand(name); verify the
        // function exists with the expected signature and returns non-empty lines.
        assert!(!help_for_command('/', "save-game").is_empty());
    }
}
