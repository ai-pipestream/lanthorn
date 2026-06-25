//! Slash-command parser: `parse`, curated table, fallback to `keymap::Command`,
//! `slash_names`, and `help_text`.
//!
//! `parse` receives the input AFTER the leading prefix character has been
//! stripped. It does not know what the prefix was.

use crate::input::Action;
use crate::keymap::{Command, ALL_COMMANDS};

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
}

// ── TranscriptFilterArg ───────────────────────────────────────────────────────

/// Argument for the `/filter` command. `main.rs` maps this to `state::TranscriptFilter`.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptFilterArg {
    Both,
    Story,
    Meta,
}

// ── Curated table ─────────────────────────────────────────────────────────────

/// One entry in the curated table: a command name (or alias) and a builder
/// that converts the remaining argument tokens into a `SlashOutcome`.
struct CuratedEntry {
    name: &'static str,
    help: &'static str,
    build: fn(&[&str]) -> SlashOutcome,
}

/// The full curated table. Entries are matched in order; first match wins.
/// Curated entries WIN over the kebab `Command::from_name` fallback.
static CURATED: &[CuratedEntry] = &[
    CuratedEntry {
        name: "panh",
        help: "panh <n>  — pan the map horizontally by n cells",
        build: |args| {
            match args.first().and_then(|s| s.parse::<i32>().ok()) {
                Some(n) => SlashOutcome::Action(Action::Pan(n, 0)),
                None => SlashOutcome::Error("panh requires an integer argument (e.g. panh -3)".into()),
            }
        },
    },
    CuratedEntry {
        name: "panv",
        help: "panv <n>  — pan the map vertically by n cells",
        build: |args| {
            match args.first().and_then(|s| s.parse::<i32>().ok()) {
                Some(n) => SlashOutcome::Action(Action::Pan(0, n)),
                None => SlashOutcome::Error("panv requires an integer argument (e.g. panv -3)".into()),
            }
        },
    },
    CuratedEntry {
        name: "zoom",
        help: "zoom in|out|reset|<n>  — zoom the map; <n> steps in (positive) or out (negative)",
        build: |args| {
            match args.first().copied() {
                Some("in") => SlashOutcome::Action(Action::ZoomIn),
                Some("out") => SlashOutcome::Action(Action::ZoomOut),
                Some("reset") => SlashOutcome::Action(Action::ZoomReset),
                Some(s) => {
                    // Interpret a signed integer as ZoomIn (positive) or ZoomOut (negative).
                    // There is no single "zoom to level N" Action, so we dispatch ZoomIn/ZoomOut
                    // for the first step; repeated steps would require the caller to loop.
                    // For /zoom 0 we treat it as a reset.
                    match s.parse::<i32>() {
                        Ok(0) => SlashOutcome::Action(Action::ZoomReset),
                        Ok(n) if n > 0 => SlashOutcome::Action(Action::ZoomIn),
                        Ok(_) => SlashOutcome::Action(Action::ZoomOut),
                        Err(_) => SlashOutcome::Error(
                            format!("zoom: expected in|out|reset|<integer>, got '{s}'")
                        ),
                    }
                }
                None => SlashOutcome::Error("zoom requires an argument: in|out|reset|<n>".into()),
            }
        },
    },
    CuratedEntry {
        name: "center",
        help: "center  — re-center the map on the selected room",
        build: |_args| SlashOutcome::Action(Action::Recenter),
    },
    CuratedEntry {
        name: "tidy",
        help: "tidy  — re-tidy the map layout",
        build: |_args| SlashOutcome::Action(Action::Retidy),
    },
    CuratedEntry {
        name: "layer",
        help: "layer next|prev|<n>  — cycle to next/prev layer, or jump by n steps",
        build: |args| {
            match args.first().copied() {
                Some("next") => SlashOutcome::Action(Action::CycleLayer(1)),
                Some("prev") => SlashOutcome::Action(Action::CycleLayer(-1)),
                Some(s) => {
                    // There is no "jump to absolute layer N" Action; CycleLayer(delta) is
                    // the only available primitive. We interpret the integer as a delta
                    // (positive = forward, negative = backward).
                    match s.parse::<i32>() {
                        Ok(n) => SlashOutcome::Action(Action::CycleLayer(n)),
                        Err(_) => SlashOutcome::Error(
                            format!("layer: expected next|prev|<integer delta>, got '{s}'")
                        ),
                    }
                }
                None => SlashOutcome::Error("layer requires an argument: next|prev|<n>".into()),
            }
        },
    },
    CuratedEntry {
        name: "save",
        help: "save [name]  — save the game, optionally to a named slot",
        build: |args| SlashOutcome::Save(args.first().map(|s| s.to_string())),
    },
    CuratedEntry {
        name: "load",
        help: "load [name]  — load a save, optionally a named slot",
        build: |args| SlashOutcome::Load(args.first().map(|s| s.to_string())),
    },
    CuratedEntry {
        name: "reset",
        help: "reset [map]  — reset the game; 'reset map' also clears the automapper",
        build: |args| {
            let map = args.first().copied() == Some("map");
            SlashOutcome::Reset { map }
        },
    },
    CuratedEntry {
        name: "quit",
        help: "quit  — exit the application",
        build: |_args| SlashOutcome::Quit,
    },
    CuratedEntry {
        name: "help",
        help: "help  — show this help text",
        build: |_args| SlashOutcome::Help,
    },
    CuratedEntry {
        name: "search",
        help: "search [query]  — search transcript (case-insensitive); no query repeats last search",
        build: |args| {
            if args.is_empty() {
                SlashOutcome::Search(None)
            } else {
                SlashOutcome::Search(Some(args.join(" ")))
            }
        },
    },
    CuratedEntry {
        name: "filter",
        help: "filter story|meta|both  — filter transcript by category",
        build: |args| match args.first().copied() {
            Some("story") => SlashOutcome::Filter(TranscriptFilterArg::Story),
            Some("meta")  => SlashOutcome::Filter(TranscriptFilterArg::Meta),
            Some("both")  => SlashOutcome::Filter(TranscriptFilterArg::Both),
            _ => SlashOutcome::Error("filter: use story | meta | both".into()),
        },
    },
    CuratedEntry {
        name: "export",
        help: "export [file]  — export visible transcript to a file (default: ~/.babelmap/exports/)",
        build: |args| SlashOutcome::Export(args.first().map(|s| s.to_string())),
    },
    CuratedEntry {
        name: "hint",
        help: "hint  — open the Hints panel (companion Invisiclues / hint-file mini-terminal)",
        build: |_args| SlashOutcome::OpenHints,
    },
    CuratedEntry {
        name: "hints",
        help: "hints  — alias for hint",
        build: |_args| SlashOutcome::OpenHints,
    },
    // ── Aliases ───────────────────────────────────────────────────────────────
    CuratedEntry {
        name: "q",
        help: "q  — alias for quit",
        build: |_args| SlashOutcome::Quit,
    },
    CuratedEntry {
        name: "h",
        help: "h  — alias for help",
        build: |_args| SlashOutcome::Help,
    },
    CuratedEntry {
        name: "recenter",
        help: "recenter  — alias for center",
        build: |_args| SlashOutcome::Action(Action::Recenter),
    },
    CuratedEntry {
        name: "retidy",
        help: "retidy  — alias for tidy",
        build: |_args| SlashOutcome::Action(Action::Retidy),
    },
    CuratedEntry {
        name: "pan",
        help: "pan <dx> <dy>  — pan the map by dx cols and dy rows",
        build: |args| {
            let dx = args.first().and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
            let dy = args.get(1).and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
            SlashOutcome::Action(Action::Pan(dx, dy))
        },
    },
];

// ── parse ─────────────────────────────────────────────────────────────────────

/// Parse a slash-command body (the text AFTER the leading prefix, e.g. `/`).
///
/// `prefix` is the configured command prefix character, used only in user-facing
/// error/help display strings. Routing and matching logic is unaffected.
///
/// Empty body → `Error`.
/// Token0 matched in the curated table → run its builder with remaining tokens.
/// Token0 matched as kebab-case `Command` name → `Action(command.to_action())`.
/// Otherwise → `Error("unknown command: <prefix><t0> — try <prefix>help")`.
///
/// Special case: `search` passes the raw remainder of the line (after the
/// command word and its single following space) as the query, preserving any
/// internal whitespace. Other commands still receive split tokens.
pub fn parse(body: &str, prefix: char) -> SlashOutcome {
    let tokens: Vec<&str> = body.split_whitespace().collect();

    let Some(t0) = tokens.first().copied() else {
        return SlashOutcome::Error(format!("type {prefix}help for commands"));
    };

    // Special-case `search`: preserve internal whitespace in the query by
    // taking the raw remainder after the command word rather than re-joining
    // split tokens.
    if t0 == "search" {
        let remainder = body[t0.len()..].trim_start_matches(' ');
        let trimmed = remainder.trim_end();
        if trimmed.is_empty() {
            return SlashOutcome::Search(None);
        }
        return SlashOutcome::Search(Some(trimmed.to_string()));
    }

    let args = &tokens[1..];

    // Curated table first.
    if let Some(entry) = CURATED.iter().find(|e| e.name == t0) {
        return (entry.build)(args);
    }

    if t0 == "reload" {
        return SlashOutcome::Action(crate::keymap::Command::ReloadStyle.to_action());
    }

    if t0 == "watch" {
        return SlashOutcome::Action(crate::keymap::Command::ToggleWatch.to_action());
    }

    if t0 == "game-style" {
        return SlashOutcome::Action(crate::keymap::Command::GameStyle.to_action());
    }

    // Fallback: kebab-name → Command::from_name (snake_case; convert hyphens to underscores).
    let snake = t0.replace('-', "_");
    if let Some(cmd) = Command::from_name(&snake) {
        return SlashOutcome::Action(cmd.to_action());
    }

    SlashOutcome::Error(format!("unknown command: {prefix}{t0} — try {prefix}help"))
}

// ── slash_names ───────────────────────────────────────────────────────────────

/// All known slash-command names (for Tab autocomplete).
///
/// Returns the union of curated names and `ALL_COMMANDS` kebab names
/// (underscores converted to hyphens).
pub fn slash_names() -> Vec<String> {
    let mut names: Vec<String> = CURATED.iter().map(|e| e.name.to_string()).collect();

    for cmd in ALL_COMMANDS {
        let kebab = cmd.name().replace('_', "-");
        if !names.contains(&kebab) {
            names.push(kebab);
        }
    }

    names
}

// ── help_text ─────────────────────────────────────────────────────────────────

/// Lines to display when the user types the help command.
///
/// `prefix` is the configured command prefix character used in all display strings.
pub fn help_text(prefix: char) -> Vec<String> {
    let mut lines = vec![
        format!("Slash commands (type {prefix}<command> [args]):"),
        String::new(),
    ];
    for entry in CURATED {
        lines.push(format!("  {prefix}{}", entry.help));
    }
    lines.push(String::new());
    lines.push(
        format!("Any keymap command is also available by its kebab name (e.g. {prefix}open-config).")
    );
    lines
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_curated_and_fallback_and_errors() {
        use crate::input::Action;
        assert!(matches!(parse("panh -1", '/'), SlashOutcome::Action(Action::Pan(-1, 0))));
        assert!(matches!(parse("panv 2", '/'), SlashOutcome::Action(Action::Pan(0, 2))));
        assert!(matches!(parse("zoom reset", '/'), SlashOutcome::Action(Action::ZoomReset)));
        assert!(matches!(parse("save foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("save", '/'), SlashOutcome::Save(None)));
        assert!(matches!(parse("reset map", '/'), SlashOutcome::Reset { map: true }));
        assert!(matches!(parse("reset", '/'), SlashOutcome::Reset { map: false }));
        assert!(matches!(parse("quit", '/'), SlashOutcome::Quit));
        assert!(matches!(parse("help", '/'), SlashOutcome::Help));
        // fallback by kebab name:
        assert!(matches!(parse("open-config", '/'), SlashOutcome::Action(_)));
        // errors:
        assert!(matches!(parse("panh", '/'), SlashOutcome::Error(_)));   // missing arg
        assert!(matches!(parse("nope", '/'), SlashOutcome::Error(_)));   // unknown
        assert!(matches!(parse("", '/'), SlashOutcome::Error(_)));       // bare prefix
    }

    #[test]
    fn slash_names_includes_curated_and_fallback() {
        let n = slash_names();
        assert!(n.iter().any(|s| s == "panh"));
        assert!(n.iter().any(|s| s == "open-config")); // a kebab Command name
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
    fn slash_hint_parses_to_open_hints() {
        assert!(matches!(crate::slash::parse("hint", '/'), crate::slash::SlashOutcome::OpenHints));
        assert!(matches!(crate::slash::parse("hints", '/'), crate::slash::SlashOutcome::OpenHints));
    }

    #[test]
    fn parse_search_filter_export() {
        assert!(matches!(parse("search twisty maze", '/'), SlashOutcome::Search(Some(q)) if q == "twisty maze"));
        assert!(matches!(parse("search a  b", '/'), SlashOutcome::Search(Some(q)) if q == "a  b"));
        assert!(matches!(parse("search", '/'), SlashOutcome::Search(None)));
        assert!(matches!(parse("filter meta", '/'), SlashOutcome::Filter(TranscriptFilterArg::Meta)));
        assert!(matches!(parse("filter both", '/'), SlashOutcome::Filter(TranscriptFilterArg::Both)));
        assert!(matches!(parse("filter nope", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("export", '/'), SlashOutcome::Export(None)));
        assert!(matches!(parse("export out.txt", '/'), SlashOutcome::Export(Some(f)) if f == "out.txt"));
    }
}
