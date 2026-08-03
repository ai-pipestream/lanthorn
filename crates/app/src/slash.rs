//! Slash-command parser and the command registry.
//!
//! `COMMANDS` is the **single source of truth** for every command in the app.
//! Both typed slash input and key presses are dispatched the same way:
//! `parse_in_context(body, prefix, ctx)` looks the command up in `COMMANDS`
//! and runs its `dispatch` closure, producing a [`SlashOutcome`]; the run
//! loop's `dispatch_slash_outcome` (main.rs) then applies it. Key bindings are
//! just stored command-strings (see `crate::keymap`) fed through this same
//! `parse_in_context`. There is no separate command enum — execution flows
//! through [`crate::input::Action`] via `SlashOutcome::Action`.
//!
//! To add a command: add ONE [`CommandSpec`] to `COMMANDS`. That alone gives
//! it slash parsing, `/help` grouping + `help <command>` detail, and Tab
//! autocomplete — there is no second place to register it, so a command can
//! never be missing from `/help`. (Bump the count in the registry
//! well-formedness test when you do.) Names are verb-noun kebab-case; `quit`
//! and `help` are the only one-word exceptions. Directional behavior is
//! expressed by arguments bound to keys, not by separate commands (e.g.
//! `pan-map <dx> <dy>`), so prefer a parametric command over per-direction
//! variants.
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
    /// Reset the app; `map: true` also clears the automapper state; `data: true`
    /// also deletes the game's auto persistent data (VFS cache + aux + auto save).
    Reset { map: bool, data: bool },
    /// Quit the application.
    Quit,
    /// Exit the current story and return to the story picker (only meaningful
    /// when babelmap was launched from a directory). Handled in
    /// `slash_dispatch`: it mirrors `Quit`'s save-prompt path but resolves the
    /// loop to the library instead of exiting. (SQ-0435)
    QuitToLibrary,
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
    /// Print the resolved color scheme to the transcript. `actual` = render each
    /// selector line in its own style instead of the plain meta color.
    PrintColors { actual: bool },
    /// Diagnostic: dump the live Glk window layout (sizes, borders, per-window
    /// colours) to the transcript as Meta lines. Handled in `slash_dispatch`.
    DumpWindows,
    /// Toggle the Z-machine debug inspector tiled pane. Handled in `slash_dispatch`
    /// (needs AppState + the engine's debugger capability).
    ToggleDebug,
    /// Replay the notification history into the transcript as Meta lines, in case
    /// a toast was missed. Handled in `slash_dispatch`. (SQ-0176)
    DumpNotifications,
    /// Diagnostic: list Blorb `Snd` resources (`None`) or play resource `n`
    /// (`Some(n)`). Handled in `main.rs::dispatch_slash_outcome` because it
    /// needs `AppState` (audio backend + sound blorb).
    PlaySound(Option<u32>),
    /// Load a standalone map file (path argument) into the current session.
    LoadMap(String),
    /// Force this game's `honor_game_colours` on/off (`Some`) or clear the
    /// per-game override (`None` = `auto`, fall back to garglk.ini/global).
    /// Persisted per-game; handled in `slash_dispatch`.
    SetGameColours(Option<bool>),
    /// Set this game's `borderless_windows` override — the payload is the
    /// borderless value: `Some(true)` = abut (no borders), `Some(false)` = keep
    /// Glk borders, `None` = clear the override (default: bordered). Persisted
    /// per-game; applied live to a running Glulx session. (SQ-0341)
    SetGameBorderless(Option<bool>),
    /// Toggle debug-trace sections: `None` shows current state; `Some(list)` sets
    /// the active set (comma list of screen,map,hostio / all / none). (trace feature)
    Trace(Option<String>),
    /// Switch the v6 render mode live for this session (`Some`) or cycle to the
    /// next mode (`None` = bare). Session-only — the config screen persists.
    /// Handled in `slash_dispatch` (mutates `state.config.v6_render`).
    SetV6Render(Option<crate::config::V6RenderMode>),
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

/// The command registry — the single source of truth for every command.
/// Add a new command by adding one entry here (see the module docs); nothing
/// else needs to change for it to appear in `/help`, Tab autocomplete, and the
/// parser. Keep entries grouped by `Category` in the display order of
/// `Category::ORDER`.
pub static COMMANDS: &[CommandSpec] = &[
    // ── Game ──────────────────────────────────────────────────────────────
    CommandSpec { name: "save-state", category: Category::Game, context: Context::Global,
        usage: "save-state [name]", description: "save an emulator Save State, optionally to a named slot",
        dispatch: |a| SlashOutcome::Save(a.first().map(|s| s.to_string())) },
    CommandSpec { name: "restore-state", category: Category::Game, context: Context::Global,
        usage: "restore-state [name]", description: "restore an emulator Save State — bare opens the saves dialog to pick one; a name restores that slot directly",
        dispatch: |a| match a.first() {
            Some(name) => SlashOutcome::Load(Some(name.to_string())),
            None => SlashOutcome::Action(crate::input::Action::OpenSaves),
        } },
    CommandSpec { name: "reset-game", category: Category::Game, context: Context::Global,
        usage: "reset-game [map] [data]", description: "restart the game — bare opens the options dialog; 'map' also clears the map, 'data' deletes the game's saved progress/cache so it starts fresh",
        dispatch: |a| SlashOutcome::Reset { map: a.contains(&"map"), data: a.contains(&"data") } },
    CommandSpec { name: "quit", category: Category::Game, context: Context::Global,
        usage: "quit", description: "exit babelmap",
        dispatch: |_| SlashOutcome::Quit },
    CommandSpec { name: "quit-to-library", category: Category::Game, context: Context::Global,
        usage: "quit-to-library", description: "exit the current story and return to the story library",
        dispatch: |_| SlashOutcome::QuitToLibrary },
    CommandSpec { name: "open-hints", category: Category::Game, context: Context::Global,
        usage: "open-hints", description: "open the hints panel",
        dispatch: |_| SlashOutcome::OpenHints },
    CommandSpec { name: "open-history", category: Category::Game, context: Context::Global,
        usage: "open-history", description: "open the rewind/replay history",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenHistory) },
    CommandSpec { name: "open-verb-menu", category: Category::Game, context: Context::Global,
        usage: "open-verb-menu", description: "open the verb/item palette",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenVerbMenu) },
    CommandSpec { name: "toggle-timed-input", category: Category::Game, context: Context::Global,
        usage: "toggle-timed-input", description: "toggle honoring the game's timed-input timers",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleTimedInput) },
    CommandSpec { name: "toggle-sound", category: Category::Game, context: Context::Global,
        usage: "toggle-sound", description: "toggle audio playback (bleeps + sampled sounds)",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleSound) },
    CommandSpec { name: "volume", category: Category::Game, context: Context::Global,
        usage: "volume <0-100>", description: "set the master audio volume (0-100)",
        dispatch: |a| match a.first().and_then(|s| s.parse::<u8>().ok()) {
            Some(v) if v <= 100 => SlashOutcome::Action(crate::input::Action::SetVolume(v)),
            _ => err("volume requires an integer 0-100 (e.g. volume 60)"),
        } },
    CommandSpec { name: "play-sound", category: Category::Game, context: Context::Global,
        usage: "play-sound [n]", description: "diagnostic: list Snd resources, or play resource n",
        dispatch: |a| match a.first() {
            None => SlashOutcome::PlaySound(None),
            Some(s) => match s.parse::<u32>() {
                Ok(n) => SlashOutcome::PlaySound(Some(n)),
                Err(_) => err(format!("play-sound: expected a resource number, got '{s}'")),
            },
        } },

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
                    // Keep the magnitude (SQ-0355): collapsing every n to a single step made the
                    // usage's "step by signed n" a lie — `zoom-map 5` moved exactly as far as
                    // `zoom-map in`.
                    Ok(n) => SlashOutcome::Action(Action::ZoomBy(n)),
                    Err(_) => err(format!("zoom-map: expected in|out|reset|<integer>, got '{s}'")),
                },
                None => err("zoom-map requires an argument: in|out|reset|<n>"),
            }
        } },
    CommandSpec { name: "center-map", category: Category::Map, context: Context::Map,
        usage: "center-map", description: "re-center the map on the selected room, or the current one",
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
        usage: "peel-layer [direction]", description: "peel a region into its own layer; a direction cuts at that passage",
        dispatch: |a| {
            use crate::input::Action;
            match a.first().copied() {
                None => SlashOutcome::Action(Action::PeelLayer(None)),
                Some(s) => match mapper::direction::parse_direction(s) {
                    Some(d) => SlashOutcome::Action(Action::PeelLayer(Some(d))),
                    None => err(format!("peel-layer: '{s}' is not a direction")),
                },
            }
        } },
    CommandSpec { name: "merge-layer", category: Category::Map, context: Context::Map,
        usage: "merge-layer", description: "merge the selected layer down",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::MergeLayer) },
    CommandSpec { name: "toggle-inspector", category: Category::Map, context: Context::Map,
        usage: "toggle-inspector", description: "toggle the room-inspector overlay",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleInspector) },
    CommandSpec { name: "load-map", category: Category::Map, context: Context::Global,
        usage: "load-map <path>", description: "load a standalone map file into the current session",
        dispatch: |a| match a.first() {
            Some(p) => SlashOutcome::LoadMap(p.to_string()),
            None => SlashOutcome::Error("load-map: a file path is required".into()),
        } },
    CommandSpec { name: "toggle-room-numbers", category: Category::Map, context: Context::Global,
        usage: "toggle-room-numbers", description: "toggle room-number labels",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleRoomNumbers) },
    CommandSpec { name: "toggle-untried-exits", category: Category::Map, context: Context::Global,
        usage: "toggle-untried-exits", description: "mark compass directions you haven't tried yet with ?",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleUntriedExits) },
        CommandSpec { name: "toggle-alignment", category: Category::Map, context: Context::Global,
        usage: "toggle-alignment", description: "toggle alignment guides",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleAlignment) },
    CommandSpec { name: "toggle-portal-labels", category: Category::Map, context: Context::Global,
        usage: "toggle-portal-labels", description: "toggle portal labels",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::TogglePortalLabels) },
    // ── View ──────────────────────────────────────────────────────────────
    CommandSpec { name: "toggle-map", category: Category::View, context: Context::Global,
        usage: "toggle-map", description: "show or hide the map panel",
        dispatch: |_a| SlashOutcome::Action(crate::input::Action::ToggleMap) },
    CommandSpec { name: "toggle-focus", category: Category::View, context: Context::Global,
        usage: "toggle-focus", description: "switch focus between panes",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleFocus) },
    CommandSpec { name: "toggle-inventory", category: Category::View, context: Context::Global,
        usage: "toggle-inventory", description: "toggle the inventory strip",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleInventory) },
    CommandSpec { name: "toggle-status-bar", category: Category::View, context: Context::Global,
        usage: "toggle-status-bar", description: "toggle the status/score bar",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleStatusBar) },
    CommandSpec { name: "resize-panes", category: Category::View, context: Context::Global,
        usage: "resize-panes", description: "enter interactive pane-resize mode",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ResizePanes) },
    CommandSpec { name: "reset-pane-size", category: Category::View, context: Context::Global,
        usage: "reset-pane-size", description: "reset all pane sizes to their defaults",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ResizeReset) },

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
    CommandSpec { name: "open-config", category: Category::Style, context: Context::Global,
        usage: "open-config", description: "open the settings screen",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::OpenConfig) },
    CommandSpec { name: "reload-style", category: Category::Style, context: Context::Global,
        usage: "reload-style", description: "reload style.toml from disk",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ReloadStyle) },
    CommandSpec { name: "toggle-watch", category: Category::Style, context: Context::Global,
        usage: "toggle-watch", description: "toggle live style-file watching",
        dispatch: |_| SlashOutcome::Action(crate::input::Action::ToggleWatch) },
    CommandSpec { name: "print-colors", category: Category::Style, context: Context::Global,
        usage: "print-colors [color]", description: "print the current color scheme (color = actual colors)",
        dispatch: |a| SlashOutcome::PrintColors { actual: a.first() == Some(&"color") } },
    CommandSpec { name: "set-game-colours", category: Category::Style, context: Context::Global,
        usage: "set-game-colours on|off|auto", description: "force this game's own colours on/off (auto follows garglk.ini/global); persisted per-game",
        dispatch: |a| match a.first().copied() {
            Some("on")   => SlashOutcome::SetGameColours(Some(true)),
            Some("off")  => SlashOutcome::SetGameColours(Some(false)),
            Some("auto") => SlashOutcome::SetGameColours(None),
            _ => err("set-game-colours requires an argument: on | off | auto"),
        } },
    CommandSpec { name: "set-v6-render", category: Category::Style, context: Context::Global,
        usage: "set-v6-render [hybrid|raster|frameless]", description: "switch the v6 render mode live — bare cycles to the next mode; session-only (the settings screen persists)",
        dispatch: |a| {
            use crate::config::V6RenderMode;
            match a.first().copied() {
                None => SlashOutcome::SetV6Render(None),
                Some("hybrid")    => SlashOutcome::SetV6Render(Some(V6RenderMode::Hybrid)),
                Some("raster")    => SlashOutcome::SetV6Render(Some(V6RenderMode::Raster)),
                Some("frameless") => SlashOutcome::SetV6Render(Some(V6RenderMode::Frameless)),
                Some(s) => err(format!("set-v6-render: unknown mode '{s}' (hybrid | raster | frameless, or bare to cycle)")),
            }
        } },
    CommandSpec { name: "set-game-borders", category: Category::Style, context: Context::Global,
        usage: "set-game-borders on|off|auto", description: "show this game's Glk window borders (on), or render borderless/abutting (off); auto = default (on); persisted per-game",
        // on = borders shown (default) → borderless=false; off = borderless/abut
        // → borderless=true; auto = clear the override.
        dispatch: |a| match a.first().copied() {
            Some("on")   => SlashOutcome::SetGameBorderless(Some(false)),
            Some("off")  => SlashOutcome::SetGameBorderless(Some(true)),
            Some("auto") => SlashOutcome::SetGameBorderless(None),
            _ => err("set-game-borders requires an argument: on | off | auto"),
        } },

    // ── Export ────────────────────────────────────────────────────────────
    CommandSpec { name: "export-svg", category: Category::Export, context: Context::Global,
        usage: "export-svg [file]", description: "export the map as SVG; default path when omitted",
        dispatch: |a| SlashOutcome::Action(crate::input::Action::ExportSvg(a.first().map(|s| s.to_string()))) },
    CommandSpec { name: "export-dot", category: Category::Export, context: Context::Global,
        usage: "export-dot [file]", description: "export the map as Graphviz DOT; default path when omitted",
        dispatch: |a| SlashOutcome::Action(crate::input::Action::ExportDot(a.first().map(|s| s.to_string()))) },
    CommandSpec { name: "export-map", category: Category::Export, context: Context::Global,
        usage: "export-map [file]", description: "dump the map structure; default path when omitted",
        dispatch: |a| SlashOutcome::Action(crate::input::Action::ExportMap(a.first().map(|s| s.to_string()))) },

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
    CommandSpec { name: "dump-windows", category: Category::Help, context: Context::Global,
        usage: "dump-windows", description: "dump the live Glk window layout (sizes, borders, colours)",
        dispatch: |_| SlashOutcome::DumpWindows },
    CommandSpec { name: "debug", category: Category::Help, context: Context::Global,
        usage: "debug", description: "toggle the Z-machine debug inspector pane",
        dispatch: |_| SlashOutcome::ToggleDebug },
    CommandSpec {
        name: "trace", category: Category::Help, context: Context::Global,
        usage: "trace [sections|all|none]",
        description: "toggle debug-trace sections (screen, map, hostio, v6) written to trace.log; no arg shows current state",
        dispatch: |a| SlashOutcome::Trace(a.first().map(|s| s.to_string())),
    },
    CommandSpec { name: "dump-notifications", category: Category::Help, context: Context::Global,
        usage: "dump-notifications", description: "print the notification history to the transcript, in case a toast was missed",
        dispatch: |_| SlashOutcome::DumpNotifications },
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

    /// SQ-0360: `peel-layer` takes an optional direction naming the seam to cut at.
    #[test]
    fn peel_layer_takes_an_optional_direction() {
        use crate::input::Action;
        use mapper::direction::Direction;
        assert!(matches!(parse("peel-layer", '/'), SlashOutcome::Action(Action::PeelLayer(None))));
        assert!(matches!(
            parse("peel-layer east", '/'),
            SlashOutcome::Action(Action::PeelLayer(Some(Direction::E)))
        ));
        // Whatever `parse_direction` accepts for a game command works here too — one vocabulary.
        assert!(matches!(
            parse("peel-layer nw", '/'),
            SlashOutcome::Action(Action::PeelLayer(Some(Direction::NW)))
        ));
        assert!(matches!(parse("peel-layer sideways", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn parse_registry_and_errors() {
        use crate::input::Action;
        assert!(matches!(parse("pan-map -1 0", '/'), SlashOutcome::Action(Action::Pan(-1, 0))));
        assert!(matches!(parse("pan-map 0 2", '/'), SlashOutcome::Action(Action::Pan(0, 2))));
        assert!(matches!(parse("zoom-map reset", '/'), SlashOutcome::Action(Action::ZoomReset)));
        // SQ-0355: the usage promises "step by signed n", so the magnitude must survive parsing.
        // It used to collapse to a single ZoomIn/ZoomOut, making `zoom-map 5` a synonym for
        // `zoom-map in`.
        assert!(matches!(parse("zoom-map 2", '/'), SlashOutcome::Action(Action::ZoomBy(2))));
        assert!(matches!(parse("zoom-map -3", '/'), SlashOutcome::Action(Action::ZoomBy(-3))));
        assert!(matches!(parse("zoom-map 1", '/'), SlashOutcome::Action(Action::ZoomBy(1))));
        // 0 still means "reset", and a non-integer is still an error.
        assert!(matches!(parse("zoom-map 0", '/'), SlashOutcome::Action(Action::ZoomReset)));
        assert!(matches!(parse("zoom-map wat", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("cycle-layer next", '/'), SlashOutcome::Action(Action::CycleLayer(1))));
        assert!(matches!(parse("save-state foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("save-state", '/'), SlashOutcome::Save(None)));
        // restore-state: a name restores that slot directly; bare opens the saves dialog.
        assert!(matches!(parse("restore-state foo", '/'), SlashOutcome::Load(Some(_))));
        assert!(matches!(parse("restore-state", '/'), SlashOutcome::Action(Action::OpenSaves)));
        assert!(matches!(parse("reset-game map", '/'), SlashOutcome::Reset { map: true, data: false }));
        assert!(matches!(parse("reset-game", '/'), SlashOutcome::Reset { map: false, data: false }));
        assert!(matches!(parse("reset-game data", '/'), SlashOutcome::Reset { map: false, data: true }));
        assert!(matches!(parse("reset-game map data", '/'), SlashOutcome::Reset { map: true, data: true }));
        assert!(matches!(parse("reset-game data map", '/'), SlashOutcome::Reset { map: true, data: true }));
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
            lines.iter().any(|l| l.contains("/save-state")),
            "save-state should appear in /help"
        );
        assert!(
            lines.iter().any(|l| l.contains("/zoom-map")),
            "zoom-map should appear in /help"
        );
    }

    #[test]
    fn quit_to_library_parses_to_quit_to_library() {
        assert!(matches!(parse("quit-to-library", '/'), SlashOutcome::QuitToLibrary));
        assert_eq!(find_command("quit-to-library").expect("quit-to-library").category, Category::Game);
    }

    #[test]
    fn slash_hint_parses_to_open_hints() {
        assert!(matches!(crate::slash::parse("open-hints", '/'), crate::slash::SlashOutcome::OpenHints));
        // old short names no longer resolve (clean break):
        assert!(matches!(crate::slash::parse("hint", '/'), crate::slash::SlashOutcome::Error(_)));
        assert!(matches!(crate::slash::parse("hints", '/'), crate::slash::SlashOutcome::Error(_)));
    }

    #[test]
    fn load_map_parses_path_argument() {
        assert!(matches!(parse("load-map ~/Downloads/map.json", '/'),
            SlashOutcome::LoadMap(p) if p == "~/Downloads/map.json"));
    }

    #[test]
    fn load_map_without_path_is_an_error() {
        assert!(matches!(parse("load-map", '/'), SlashOutcome::Error(_)));
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
        assert!(matches!(parse("save-state foo", '/'), SlashOutcome::Save(Some(_))));
        assert!(matches!(parse("reset-game map", '/'), SlashOutcome::Reset { map: true, data: false }));
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
            if c.name == "quit" || c.name == "help" || c.name == "volume" || c.name == "trace" || c.name == "debug" { continue; }
            assert!(c.name.contains('-'), "non-verb-noun command name: {}", c.name);
        }
        // Spot-check representative commands exist with the right category.
        let by = |n: &str| COMMANDS.iter().find(|c| c.name == n).expect(n);
        assert_eq!(by("save-state").category, Category::Game);
        assert_eq!(by("zoom-map").category, Category::Map);
        assert_eq!(by("anim-step").context, Context::Anim);
        // Total count matches the spec table (Game 12, Map 20, View 6,
        // Transcript 3, Style 7, Export 3, Animation 4, Help 3). `open-saves`
        // was removed — `restore-state` (bare) opens the saves dialog instead.
        // `debug` (SQ-0169) opens the Z-machine debug inspector. `open-gallery`
        // and `open-style-editor` were removed (SQ-0309): the interactive
        // gallery/style-editor UIs are gone; `reload-style` remains.
        // `quit-to-library` (SQ-0435) exits to the story picker.
        // `set-v6-render` switches/cycles the v6 render mode live.
        // `nudge-room` was removed with Manual layout mode (SQ-0600).
        assert_eq!(COMMANDS.len(), 59, "registry must match the spec's Full command table");
    }

    #[test]
    fn sound_commands_present() {
        let by = |n: &str| COMMANDS.iter().find(|c| c.name == n).expect(n);
        assert_eq!(by("toggle-sound").category, Category::Game);
        assert_eq!(by("volume").category, Category::Game);
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
    fn print_colors_command_parses_flag() {
        assert!(find_command("print-colors").is_some());
        assert!(matches!(parse("print-colors", '/'), SlashOutcome::PrintColors { actual: false }));
        assert!(matches!(parse("print-colors color", '/'), SlashOutcome::PrintColors { actual: true }));
    }

    #[test]
    fn dump_windows_command_parses() {
        assert!(find_command("dump-windows").is_some());
        assert!(matches!(parse("dump-windows", '/'), SlashOutcome::DumpWindows));
    }

    #[test]
    fn debug_command_parses_to_toggle_debug() {
        assert!(find_command("debug").is_some());
        assert!(matches!(parse("debug", '/'), SlashOutcome::ToggleDebug));
    }

    #[test]
    fn trace_command_parses_set_and_show() {
        assert!(find_command("trace").is_some());
        let set = parse("trace screen,map", '/');
        assert!(matches!(set, SlashOutcome::Trace(Some(ref s)) if s == "screen,map"));
        let show = parse("trace", '/');
        assert!(matches!(show, SlashOutcome::Trace(None)));
    }

    #[test]
    fn play_sound_command_parses_optional_number() {
        assert!(matches!(parse("play-sound", '/'), SlashOutcome::PlaySound(None)));
        assert!(matches!(parse("play-sound 3", '/'), SlashOutcome::PlaySound(Some(3))));
        assert!(matches!(parse("play-sound nope", '/'), SlashOutcome::Error(_)));
    }

    #[test]
    fn play_sound_command_present() {
        assert_eq!(find_command("play-sound").expect("play-sound").category, Category::Game);
    }

    #[test]
    fn help_for_command_round_trip() {
        // The run loop calls help_for_command on a HelpCommand(name); verify the
        // function exists with the expected signature and returns non-empty lines.
        assert!(!help_for_command('/', "save-state").is_empty());
    }

    #[test]
    fn set_game_colours_parses_on_off_auto() {
        assert!(matches!(parse("set-game-colours on", '/'), SlashOutcome::SetGameColours(Some(true))));
        assert!(matches!(parse("set-game-colours off", '/'), SlashOutcome::SetGameColours(Some(false))));
        assert!(matches!(parse("set-game-colours auto", '/'), SlashOutcome::SetGameColours(None)));
        assert!(matches!(parse("set-game-colours", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("set-game-colours maybe", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-game-colours").expect("set-game-colours").category, Category::Style);
    }

    #[test]
    fn set_game_borders_parses_on_off_auto() {
        // on = show borders (borderless=false); off = borderless (true); auto = clear.
        assert!(matches!(parse("set-game-borders on", '/'), SlashOutcome::SetGameBorderless(Some(false))));
        assert!(matches!(parse("set-game-borders off", '/'), SlashOutcome::SetGameBorderless(Some(true))));
        assert!(matches!(parse("set-game-borders auto", '/'), SlashOutcome::SetGameBorderless(None)));
        assert!(matches!(parse("set-game-borders", '/'), SlashOutcome::Error(_)));
        assert!(matches!(parse("set-game-borders maybe", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-game-borders").expect("set-game-borders").category, Category::Style);
    }

    #[test]
    fn set_v6_render_parses_modes_and_bare_cycles() {
        use crate::config::V6RenderMode;
        assert!(matches!(parse("set-v6-render hybrid", '/'), SlashOutcome::SetV6Render(Some(V6RenderMode::Hybrid))));
        assert!(matches!(parse("set-v6-render raster", '/'), SlashOutcome::SetV6Render(Some(V6RenderMode::Raster))));
        assert!(matches!(parse("set-v6-render frameless", '/'), SlashOutcome::SetV6Render(Some(V6RenderMode::Frameless))));
        assert!(matches!(parse("set-v6-render", '/'), SlashOutcome::SetV6Render(None)));
        assert!(matches!(parse("set-v6-render sepia", '/'), SlashOutcome::Error(_)));
        assert_eq!(find_command("set-v6-render").expect("set-v6-render").category, Category::Style);
    }

    #[test]
    fn create_game_style_command_removed() {
        assert!(find_command("create-game-style").is_none(),
            "create-game-style is replaced by the Save Game Style button");
    }
}
