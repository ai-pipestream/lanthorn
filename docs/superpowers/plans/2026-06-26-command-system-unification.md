# Command-System Unification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the slash command the single source of truth for every action, and make a key binding a stored `(context, key) → command-string` entry that runs through the same parser as typed input.

**Architecture:** A static command registry (`CommandSpec` table) replaces both the curated slash table and (by Wave 2) the `Command` enum's naming role. `parse()` looks a command up by name and runs its dispatch closure. Key bindings become command-strings dispatched through the same parser. Execution still flows through the existing `Action` enum (via `SlashOutcome::Action`) and `SlashOutcome` variants. Delivered in three waves, each green and usable.

**Tech Stack:** Rust 2021 workspace; crate `app` (binary `lanthorn`); ratatui 0.29; modules `crate::slash`, `crate::keymap`, `crate::input`, `crate::config`, `crate::render::transcript`, `crate::main`.

## Global Constraints

- Every command name is `verb-noun` kebab-case. The only one-word exceptions are `quit` and `help`. Names are unique across the registry.
- The full, authoritative command list (name, usage, description, category) is the table in `docs/superpowers/specs/2026-06-26-command-system-unification-design.md` under "Full command table". Implement **every** row.
- The 8 categories, in `/help` display order: `Game, Map, View, Transcript, Style, Export, Animation, Help`.
- No migration layer for old names (clean break). The only config robustness is: an unknown command name in a `[keymap.*]` entry is skipped with a one-line warning, and config load continues.
- `keymap.use_defaults` (default `true`); `false` loads no built-in bindings. Hardwired `Ctrl+Q`/`Ctrl+C` quit and `Esc`-closes-modal survive regardless.
- 0 compiler warnings and a green `cargo test -p app` after every task. Commit-only on local `main`; do not push or merge.
- Commit trailer on every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
  No backticks in commit message bodies.
- Surgical changes only; match existing style; do not edit `TODO.md` during the waves.

---

# Wave 1 — Command registry, names, categories, /help

Goal: a registry drives slash typing and `/help`. The `Command` enum and key bindings are untouched and still work (registry and enum coexist).

### Task 1: `Category` enum

**Files:**
- Modify: `crates/app/src/slash.rs` (top, after the `SlashOutcome`/`TranscriptFilterArg` enums)

**Interfaces:**
- Produces: `pub enum Category { Game, Map, View, Transcript, Style, Export, Animation, Help }`; `impl Category { pub fn title(self) -> &'static str; pub const ORDER: [Category; 8] }`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/slash.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn category_order_and_titles() {
    assert_eq!(Category::ORDER.len(), 8);
    assert_eq!(Category::ORDER[0], Category::Game);
    assert_eq!(Category::ORDER[7], Category::Help);
    assert_eq!(Category::Game.title(), "Game");
    assert_eq!(Category::Animation.title(), "Animation");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app category_order_and_titles`
Expected: FAIL (`Category` not found).

- [ ] **Step 3: Write minimal implementation**

```rust
/// User-facing grouping for `/help` and the hotkey dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Game, Map, View, Transcript, Style, Export, Animation, Help,
}

impl Category {
    pub const ORDER: [Category; 8] = [
        Category::Game, Category::Map, Category::View, Category::Transcript,
        Category::Style, Category::Export, Category::Animation, Category::Help,
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app category_order_and_titles`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/slash.rs
git commit -m "feat(app): Category enum for command grouping"
```

### Task 2: `CommandSpec` registry with every command

**Files:**
- Modify: `crates/app/src/slash.rs` (replace the `CuratedEntry`/`CURATED` machinery)
- Reference (read-only, for dispatch targets): `crates/app/src/input.rs` (`enum Action`), `crates/app/src/keymap.rs` (`Command::to_action` for the exact `Action` each command maps to)

**Interfaces:**
- Consumes: `Category` (Task 1); `Action` (`crate::input::Action`); `SlashOutcome`, `TranscriptFilterArg` (existing).
- Produces:
  - `pub struct CommandSpec { pub name: &'static str, pub category: Category, pub context: Context, pub usage: &'static str, pub description: &'static str, pub dispatch: fn(&[&str]) -> SlashOutcome }`
  - `pub static COMMANDS: &[CommandSpec]` — one entry per row of the spec's Full command table.
  - `pub fn find_command(name: &str) -> Option<&'static CommandSpec>`
- Note: import `use crate::keymap::Context;`. Context per command matches the spec: Animation-category commands are `Context::Anim`; map-pane commands are `Context::Map`; everything else `Context::Global`.

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app registry_is_complete_and_well_formed`
Expected: FAIL (`CommandSpec`/`COMMANDS` not found).

- [ ] **Step 3: Write minimal implementation**

Remove `struct CuratedEntry` and `static CURATED`. Add the registry. Each dispatch closure mirrors the current curated builder or the matching `Command::to_action()`. Reproduce the **exact** argument semantics from the existing curated builders (`pan-map`, `zoom-map`, `cycle-layer`, `save-game`, `load-game`, `reset-game`, `filter-transcript`, `export-transcript`). The no-argument commands return `SlashOutcome::Action(<the Action from Command::to_action>)`.

```rust
use crate::keymap::Context;

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
        usage: "quit", description: "exit lanthorn",
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
```

Note: `help`'s dispatch ignores args here; Task 4 adds the `help <command>` handling in `parse()`/`help_text`. Confirm each `Action` variant name against `crate::input::Action` and `Command::to_action()` while implementing (e.g. `Action::NudgeSelected`, `Action::CycleLayer`, `Action::Pan`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app registry_is_complete_and_well_formed`
Expected: PASS. If `COMMANDS.len()` differs from 48, reconcile against the spec table — do not change the assertion.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/slash.rs
git commit -m "feat(app): unified CommandSpec registry (all 40 verb-noun commands)"
```

### Task 3: `parse()` over the registry + context gating

**Files:**
- Modify: `crates/app/src/slash.rs` (`parse`, `slash_names`)

**Interfaces:**
- Consumes: `COMMANDS`, `find_command` (Task 2).
- Produces: `parse(body: &str, prefix: char) -> SlashOutcome` routes by registry; new `parse_in_context(body: &str, prefix: char, ctx: Context) -> SlashOutcome` gates by context; `slash_names() -> Vec<String>` returns the registry names.
- Context gating rule: a command whose `context` is `Context::Anim` invoked while `ctx != Context::Anim` returns `Error("<name> is only available during animation playback")`. `Context::Global` commands are valid in any `ctx`. `Context::Map` commands are valid in any `ctx` (they no-op harmlessly when no map interaction applies, matching today). Only Anim is gated.

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app parse_routes_registry_and_gates_anim`
Expected: FAIL (`parse_in_context` missing; old curated routing).

- [ ] **Step 3: Write minimal implementation**

Replace `parse` and add `parse_in_context`. Delete the `reload`/`watch`/`game-style` inline special-cases and the `Command::from_name` fallback (those names now live in the registry as `reload-style`/`toggle-watch`/`create-game-style`).

```rust
pub fn parse(body: &str, prefix: char) -> SlashOutcome {
    parse_in_context(body, prefix, Context::Global)
}

pub fn parse_in_context(body: &str, prefix: char, ctx: Context) -> SlashOutcome {
    let Some(t0) = body.split_whitespace().next() else {
        return SlashOutcome::Error(format!("type {prefix}help for commands"));
    };

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

pub fn slash_names() -> Vec<String> {
    COMMANDS.iter().map(|c| c.name.to_string()).collect()
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p app parse_routes_registry_and_gates_anim slash_names`
Expected: PASS. Then run the whole module: `cargo test -p app slash`. Fix any now-stale tests in `slash.rs` (old curated assertions like `parse("save", …)`) by updating them to the new names — the names changed by design.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/slash.rs
git commit -m "feat(app): parse() routes the registry + Anim context gating"
```

### Task 4: Grouped `help_text` + `help <command>` detail

**Files:**
- Modify: `crates/app/src/slash.rs` (`help_text`; add `help_for_command`; route `help <command>` in `parse_in_context`)

**Interfaces:**
- Consumes: `COMMANDS`, `Category` (Tasks 1-2).
- Produces:
  - `help_text(prefix: char) -> Vec<String>` — grouped by `Category::ORDER`.
  - `help_for_command(prefix: char, name: &str) -> Vec<String>` — one command's usage + description, or an unknown-command line.
  - `parse_in_context` returns `SlashOutcome::HelpCommand(String)` when `help` has an argument (add this variant to `SlashOutcome`), else `SlashOutcome::Help`.

- [ ] **Step 1: Write the failing test**

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app help_text_grouped_and_per_command`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

Add `HelpCommand(String)` to `SlashOutcome`. In `parse_in_context`, before the generic dispatch, special-case `help` with an argument:

```rust
if t0 == "help" {
    let rest = body.split_whitespace().nth(1);
    return match rest {
        Some(name) => SlashOutcome::HelpCommand(name.to_string()),
        None => SlashOutcome::Help,
    };
}
```

Implement the help renderers:

```rust
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

pub fn help_for_command(prefix: char, name: &str) -> Vec<String> {
    match find_command(name) {
        Some(c) => vec![format!("  {prefix}{}  — {}", c.usage, c.description)],
        None => vec![format!("unknown command: {prefix}{name} — try {prefix}help")],
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app help_text_grouped_and_per_command`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/slash.rs
git commit -m "feat(app): grouped /help by category + help <command> detail"
```

### Task 5: Wire the new help outcomes into the run loop

**Files:**
- Modify: `crates/app/src/main.rs` (the `slash::parse` match around line 2144 — add `HelpCommand`; keep `Help`)

**Interfaces:**
- Consumes: `SlashOutcome::HelpCommand` (Task 4), `slash::help_for_command`, `slash::help_text`.

- [ ] **Step 1: Write the failing test**

This is a small integration edit; cover it with a unit assertion in `slash.rs` already done (Task 4) plus a compile-level guarantee. Add to `slash.rs` tests:

```rust
#[test]
fn help_for_command_round_trip() {
    // The run loop calls help_for_command on a HelpCommand(name); verify the
    // function exists with the expected signature and returns non-empty lines.
    assert!(!help_for_command('/', "save-game").is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails (or passes if Task 4 covered it)**

Run: `cargo test -p app help_for_command_round_trip`
Expected: PASS (Task 4 provides the function). If the run-loop arm is missing, `cargo build -p app` fails on the non-exhaustive match — that is the real signal for this task.

- [ ] **Step 3: Write minimal implementation**

In `main.rs`, add an arm next to `SlashOutcome::Help`:

```rust
SlashOutcome::HelpCommand(name) => {
    for line in slash::help_for_command(state.config.command_prefix, &name) {
        state.push_transcript_kind(&line, TranscriptKind::Meta);
    }
}
```

- [ ] **Step 4: Verify build + tests**

Run: `cargo build -p app && cargo test -p app`
Expected: builds with 0 warnings; all tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/main.rs crates/app/src/slash.rs
git commit -m "feat(app): run loop handles help <command> outcome"
```

### Task 6: Hanging-indent wrap for `/help` (and Meta) lines

**Files:**
- Modify: `crates/app/src/render/transcript.rs` (`wrap_line` → add `wrap_line_hanging`; apply to Meta/Warning rows in `wrap_lines_kinded`)
- Test: same file's `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: existing `wrap_line`, `META_GUTTER`.
- Produces: `pub(crate) fn wrap_line_hanging(line: &str, width: u16, indent: u16) -> Vec<String>` — continuation rows are prefixed with `indent` spaces; first row is not. Hanging indent is derived from the line's own leading-space count (so a `/help` entry indented by 2 wraps its continuations aligned under its text). `wrap_lines_kinded` uses `wrap_line_hanging` for `Meta`/`Warning` lines with `indent = leading_spaces(line)`, and plain `wrap_line` for `Story`/`Input`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn hanging_indent_wraps_continuations() {
    // A 2-space-indented line longer than width wraps with continuations
    // indented 2 spaces.
    let line = "  abcd efgh ijkl mnop";
    let rows = wrap_line_hanging(line, 10, 2);
    assert!(rows.len() >= 2);
    assert!(rows[0].starts_with("  abcd"), "first row keeps original indent");
    for cont in &rows[1..] {
        assert!(cont.starts_with("  "), "continuation '{cont}' is indented 2");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app hanging_indent_wraps_continuations`
Expected: FAIL (`wrap_line_hanging` missing).

- [ ] **Step 3: Write minimal implementation**

```rust
/// Like `wrap_line`, but every continuation row after the first is prefixed
/// with `indent` spaces so wrapped text hangs under the first row's content.
pub(crate) fn wrap_line_hanging(line: &str, width: u16, indent: u16) -> Vec<String> {
    let indent = (indent as usize).min(width.saturating_sub(1) as usize);
    if width == 0 || (line.chars().count() as u16) <= width {
        return wrap_line(line, width);
    }
    // Wrap the body at the reduced width, then re-prefix continuations.
    let first = wrap_line(line, width);
    let mut out: Vec<String> = Vec::new();
    for (i, row) in first.into_iter().enumerate() {
        if i == 0 {
            out.push(row);
        } else {
            // Re-wrap continuation content within (width - indent) to keep the
            // hang stable, prefixing the indent.
            let pad = " ".repeat(indent);
            for sub in wrap_line(&row, width.saturating_sub(indent as u16)) {
                out.push(format!("{pad}{sub}"));
            }
        }
    }
    out
}

/// Count leading ASCII spaces in `s`.
pub(crate) fn leading_spaces(s: &str) -> u16 {
    s.chars().take_while(|c| *c == ' ').count() as u16
}
```

In `wrap_lines_kinded`, replace the `wrap_line(line, w)` call for the Meta/Warning branch:

```rust
let rows = match kind {
    TranscriptKind::Meta | TranscriptKind::Warning =>
        wrap_line_hanging(line, w, leading_spaces(line).max(2)),
    TranscriptKind::Story | TranscriptKind::Input => wrap_line(line, w),
};
rows.into_iter().map(move |row| (row, kind, style))
```

(Adjust the closure body to bind `rows` then map; keep the `flat_map` shape.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p app hanging_indent_wraps_continuations && cargo test -p app transcript`
Expected: PASS; existing wrap/scroll tests still green.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/transcript.rs
git commit -m "feat(app): hanging-indent wrap for /help and meta lines"
```

---

# Wave 2 — Keys as command-strings; retire the Command enum

Goal: key bindings become `(context, key) → command-string`, dispatched through `parse_in_context`. The `Command` enum is removed.

### Task 7: `KeymapConfig` clean-slate flag + context sections

**Files:**
- Modify: `crates/app/src/config.rs` (`KeymapConfig` — add `use_defaults: bool` default `true`, and per-context override maps)

**Interfaces:**
- Produces: `KeymapConfig { pub use_defaults: bool, pub global: BTreeMap<String,String>, pub map: BTreeMap<String,String>, pub anim: BTreeMap<String,String> }` where each map is `key-spec → command-string`. Keep the old flat `overrides` field deserialization removed (clean break — the format changes from command→key to key→command-string).
- TOML shape:
  ```toml
  [keymap]
  use_defaults = true
  [keymap.global]
  "ctrl+s" = "save-game"
  [keymap.map]
  "left" = "pan-map -1 0"
  [keymap.anim]
  "l" = "anim-step forward"
  ```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn keymap_config_parses_context_sections() {
    let toml = r#"
[keymap]
use_defaults = false
[keymap.global]
"ctrl+s" = "save-game"
[keymap.map]
"left" = "pan-map -1 0"
"#;
    let cfg: Config = toml::from_str(toml).unwrap();
    assert!(!cfg.keymap.use_defaults);
    assert_eq!(cfg.keymap.global.get("ctrl+s").map(String::as_str), Some("save-game"));
    assert_eq!(cfg.keymap.map.get("left").map(String::as_str), Some("pan-map -1 0"));
    // Default keeps use_defaults true.
    assert!(Config::default().keymap.use_defaults);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app keymap_config_parses_context_sections`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

Rewrite `KeymapConfig`:

```rust
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub use_defaults: bool,
    pub global: std::collections::BTreeMap<String, String>,
    pub map: std::collections::BTreeMap<String, String>,
    pub anim: std::collections::BTreeMap<String, String>,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self {
            use_defaults: true,
            global: Default::default(),
            map: Default::default(),
            anim: Default::default(),
        }
    }
}
```

Update the old `flat_keymap_toml_parses_into_overrides` test and any `KeymapConfig::default()` usage that referenced `overrides`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p app keymap_config_parses_context_sections config`
Expected: PASS (after fixing the stale `overrides` test).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/config.rs
git commit -m "feat(app): keymap config — context sections + use_defaults clean-slate flag"
```

### Task 8: `KeyMap` of command-strings + `resolve` from config

**Files:**
- Modify: `crates/app/src/keymap.rs` (`KeyMap` now stores command-strings; rewrite `default()`, `lookup`, `lookup_any`, `for_context`, `resolve`; drop `primary_key`'s dependence on `Command`)

**Interfaces:**
- Produces:
  - `pub struct KeyMap { pub bindings: Vec<(KeySpec, String, Context)> }` (command-string instead of `Command`).
  - `pub fn default() -> KeyMap` — the built-in bindings as command-strings (port every current `bind!` line to its command-string, e.g. `Command::ZoomIn` at `+` → `"zoom-map in"`, `Command::PanLeft` at `Left` → `"pan-map -1 0"`, `Command::Recenter` at `c` → `"center-map"`, etc.). Use the spec's parametric names.
  - `pub fn lookup(&self, spec: &KeySpec, ctx: Context) -> Option<&str>` (returns the command-string; Map→Global fallthrough preserved).
  - `pub fn lookup_any(&self, spec: &KeySpec) -> Option<&str>`.
  - `pub fn primary_key(&self, command_name: &str) -> Option<KeySpec>` (matches by string prefix == the command name's first token).
  - `pub fn resolve(cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String>)` — start from `default()` unless `!cfg.use_defaults` (then empty); apply each context section; **skip-and-warn** on an unknown command name (first token not in `slash::find_command`) or an unparseable key.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn keymap_default_and_resolve_command_strings() {
    use crate::config::KeymapConfig;
    let km = KeyMap::default();
    let plus: KeySpec = "+".parse().unwrap();
    assert_eq!(km.lookup(&plus, Context::Map), Some("zoom-map in"));
    let left: KeySpec = "left".parse().unwrap();
    assert_eq!(km.lookup(&left, Context::Map), Some("pan-map -1 0"));

    // use_defaults=false → empty base; only the user binding exists.
    let mut cfg = KeymapConfig { use_defaults: false, ..Default::default() };
    cfg.global.insert("ctrl+s".into(), "save-game".into());
    let (km2, warns) = KeyMap::resolve(&cfg);
    let cs: KeySpec = "ctrl+s".parse().unwrap();
    assert_eq!(km2.lookup(&cs, Context::Global), Some("save-game"));
    assert!(km2.lookup(&plus, Context::Map).is_none(), "no defaults loaded");
    assert!(warns.is_empty());

    // Unknown command name → skip + warn.
    let mut cfg3 = KeymapConfig::default();
    cfg3.global.insert("ctrl+z".into(), "frobnicate".into());
    let (_km3, warns3) = KeyMap::resolve(&cfg3);
    assert!(warns3.iter().any(|w| w.contains("frobnicate")));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app keymap_default_and_resolve_command_strings`
Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

Change the `bindings` element type to `(KeySpec, String, Context)`. Port `default()`: every `bind!(spec, Command::X, ctx)` becomes `b.push((spec, "<command-string>".to_string(), ctx))`. Map each old `Command` to its command-string:

- `ZoomIn` → `"zoom-map in"`, `ZoomOut` → `"zoom-map out"`, `ZoomReset` → `"zoom-map reset"`
- `PanLeft` → `"pan-map -1 0"`, `PanRight` → `"pan-map 1 0"`, `PanUp` → `"pan-map 0 -1"`, `PanDown` → `"pan-map 0 1"`
- `Recenter` → `"center-map"`, `Retidy` → `"tidy-map"`
- `SelectNext` → `"select-room next"`, `SelectPrev` → `"select-room prev"`
- `CycleLayerNext` → `"cycle-layer next"`, `CycleLayerPrev` → `"cycle-layer prev"`
- `NudgeLeft` → `"nudge-room -1 0"`, `NudgeRight` → `"nudge-room 1 0"`, `NudgeUp` → `"nudge-room 0 -1"`, `NudgeDown` → `"nudge-room 0 1"`
- `CycleLayout` → `"cycle-layout"`, `CycleLayoutReverse` → `"cycle-layout reverse"`
- `AnimStepFwd` → `"anim-step forward"`, `AnimStepBack` → `"anim-step back"`, `AnimTogglePlay` → `"anim-play"`, `AnimExit` → `"anim-exit"`
- `SaveGame` → `"save-game"`, `RestoreGame` → `"load-game"`, `ResetGame` → `"reset-game"`, `Quit` → `"quit"`
- All remaining toggles/opens → their registry name verbatim (`open-config`, `open-gallery`, `toggle-inventory`, `rename-room`, `edit-notes`, `delete-connection`, `relabel-edge`, `toggle-inspector`, `peel-layer`, `merge-layer`, `rename-layer`, `toggle-alignment`, `toggle-portal-labels`, `open-saves`, `open-style-editor`, `open-history`, `open-verb-menu`, `export-svg`, `export-dot`, `export-dump`, `animate-tidy`, `toggle-focus`).

Rewrite `lookup`/`lookup_any`/`for_context` to return `&str`. Rewrite `resolve`:

```rust
pub fn resolve(cfg: &crate::config::KeymapConfig) -> (KeyMap, Vec<String>) {
    let mut km = if cfg.use_defaults { KeyMap::default() } else { KeyMap { bindings: Vec::new() } };
    let mut warnings = Vec::new();
    for (ctx, section) in [
        (Context::Global, &cfg.global),
        (Context::Map, &cfg.map),
        (Context::Anim, &cfg.anim),
    ] {
        for (key, command) in section {
            let spec = match key.parse::<KeySpec>() {
                Ok(s) => s,
                Err(e) => { warnings.push(format!("keymap: cannot parse key '{key}': {e}; skipped")); continue; }
            };
            let cmd_name = command.split_whitespace().next().unwrap_or("");
            if crate::slash::find_command(cmd_name).is_none() {
                warnings.push(format!("keymap: unknown command '{command}'; skipped"));
                continue;
            }
            km.bindings.retain(|(s, _, c)| !(*s == spec && *c == ctx));
            km.bindings.push((spec, command.clone(), ctx));
        }
    }
    (km, warnings)
}
```

`primary_key(command_name)` matches `bindings` whose first token equals `command_name`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p app keymap`
Expected: PASS. Update any keymap tests that asserted `Command` values.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/keymap.rs
git commit -m "feat(app): KeyMap stores command-strings; resolve from context sections"
```

### Task 9: Dispatch keypresses through the parser

**Files:**
- Modify: `crates/app/src/input.rs` (`key_to_action` — replace the four `cmd.to_action()` sites; the Anim sub-mode lookup)
- Modify: `crates/app/src/main.rs` (the `Event::Key` arm calling `key_to_action` — route the resulting command-string through `slash::parse_in_context` and the existing `SlashOutcome` handler)

**Interfaces:**
- Consumes: `KeyMap::lookup` (`&str`, Task 8), `slash::parse_in_context` (Task 3), the existing `SlashOutcome` run-loop handler (Task 5).
- Decision: `key_to_action` currently returns `Action`. Introduce `pub fn key_to_command(state: &AppState, key: KeyEvent) -> KeyResolve` where `pub enum KeyResolve { Action(Action), Command(String, Context), None }`. Hardwired keys (Ctrl+Q/C quit, Tab autocomplete/focus, text entry, modal sub-modes) keep returning `KeyResolve::Action(...)`. The four keymap-lookup sites return `KeyResolve::Command(command_string, ctx)`. The run loop: `Action(a) => apply_action(a,…)`; `Command(s, ctx) => dispatch via slash::parse_in_context(&s, prefix, ctx)` reusing the exact `SlashOutcome` match from the typed-input path (factor that match into a helper `fn dispatch_slash_outcome(outcome, &mut state, &mut mapper, …)` so typed input and keys share it).

- [ ] **Step 1: Write the failing test**

In `input.rs` tests:

```rust
#[test]
fn key_resolves_to_command_string() {
    let state = AppState::default_for_test(); // existing test helper; if absent, build minimal state as other input tests do
    // '+' in Map focus resolves to the zoom-map command string.
    let plus = KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE);
    // Drive through Map focus.
    let mut st = state;
    st.focus = Focus::Map;
    match key_to_command(&st, plus) {
        KeyResolve::Command(s, _ctx) => assert_eq!(s, "zoom-map in"),
        other => panic!("expected Command, got {other:?}"),
    }
}
```

(If `AppState` has no test constructor, mirror the setup used by the existing `key_to_action_equivalence_sample` test in this file.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app key_resolves_to_command_string`
Expected: FAIL (`key_to_command`/`KeyResolve` missing).

- [ ] **Step 3: Write minimal implementation**

Add `KeyResolve`; implement `key_to_command` by copying `key_to_action`'s structure but returning `KeyResolve::Command(cmd_string.to_string(), ctx)` at the four lookup sites (Anim sub-mode, ctrl-Global, Game-Global fallthrough, Map). Preserve the `is_direct` filtering — see Task 11 note: until the hotkey dialog moves (Wave 3), keep an `is_direct(command_name: &str)` shim on `HotkeyLayout` so this task compiles. Keep `key_to_action` temporarily as a thin wrapper that maps `KeyResolve::Command` via `slash::parse_in_context(...).into_action_or_none()` ONLY for cases that yield `SlashOutcome::Action` — but the cleaner path is to switch `main.rs` to consume `KeyResolve` directly (preferred). In `main.rs`, replace `key_to_action(&state, k)` with `key_to_command(&state, k)` and handle:

```rust
match key_to_command(&state, k) {
    KeyResolve::Action(a) => { /* existing apply_action / ToggleWatch special-case */ }
    KeyResolve::Command(s, ctx) => {
        let outcome = slash::parse_in_context(&s, state.config.command_prefix, ctx);
        dispatch_slash_outcome(outcome, &mut state, &mut mapper, /* … */);
    }
    KeyResolve::None => {}
}
```

Factor the existing typed-input `match slash::parse(...) { … }` body (Task 5) into `fn dispatch_slash_outcome(...)` and call it from both the typed path and the key path.

- [ ] **Step 4: Run tests**

Run: `cargo test -p app && cargo build -p app`
Expected: builds 0 warnings; full suite green. Manually sanity-run `lanthorn ./stories/minizork.z3`: arrows pan, `+`/`-` zoom, `c` centers, Ctrl+S saves, `g` opens gallery.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/input.rs crates/app/src/main.rs
git commit -m "feat(app): keypresses dispatch through the slash parser (KeyResolve)"
```

### Task 10: Retire the `Command` enum

**Files:**
- Modify: `crates/app/src/keymap.rs` (delete `enum Command` and its `impl`: `to_action`, `name`, `label`, `context`, `from_name`, `ALL_COMMANDS`)
- Modify: any remaining references (compiler will list them)

**Interfaces:**
- Consumes: nothing new. This task removes dead code once Tasks 8-9 no longer reference `Command`.
- Note: `Context` and `KeySpec` stay in `keymap.rs`. `Command::context()` logic for the registry already lives on `CommandSpec.context`. The hotkey dialog (`HotkeyLayout`, `is_direct`, `DEFAULT_DIRECT_NAMES`, `DEFAULT_GROUPS`) still references command **names as strings** — leave those; Wave 3 Task 11 re-points them onto the registry.

- [ ] **Step 1: Write the failing test (compile gate)**

No new unit test; the gate is a clean build with the enum removed. First confirm no functional references remain:

Run: `rg -n "keymap::Command|Command::|ALL_COMMANDS" crates/app/src`
Expected before edit: only `HotkeyLayout`/dialog string-name uses and dead `impl` blocks.

- [ ] **Step 2: Delete the enum + impl**

Remove `enum Command`, its `impl Command { … }`, and `pub const ALL_COMMANDS`. Convert any lingering `Command::from_name(x).map(...)` call needed by the dialog to `crate::slash::find_command(x)`.

- [ ] **Step 3: Build**

Run: `cargo build -p app 2>&1 | rg -n "error|warning" | head`
Expected: resolve each error by re-pointing to the registry/command-strings. End at 0 errors, 0 warnings.

- [ ] **Step 4: Run tests**

Run: `cargo test -p app`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/keymap.rs
git commit -m "refactor(app): retire the Command enum (registry is the source of truth)"
```

---

# Wave 3 — Hint bar, hotkey dialog, default config, docs

### Task 11: Re-point hint bar + hotkey dialog onto the registry

**Files:**
- Modify: `crates/app/src/main.rs` (`hint_bar` — derive labels from the registry/command-strings instead of `Command::label`)
- Modify: `crates/app/src/keymap.rs` (`HotkeyLayout`: `DEFAULT_DIRECT_NAMES`, `DEFAULT_GROUPS`, `is_direct` now keyed by registry command names; update the defaults to the new verb-noun names)
- Modify: `crates/app/src/input.rs` (`hotkey_dialog_key_to_action` and any dialog listing that referenced `Command`)
- Modify: `crates/app/src/render/` (whichever module renders the hotkey dialog list — locate with `rg -n "hotkey" crates/app/src/render`)

**Interfaces:**
- Consumes: `slash::COMMANDS`, `slash::find_command`, `slash::Category`, `KeyMap::lookup`/`primary_key`.
- `hint_bar`: for each candidate command name in `*_HINTS`, look up its primary key and short label. Replace `Command::label()` with the registry `description` (or a short form) — define a short label helper if the descriptions are too long for the bar (e.g. `fn short_label(spec) -> &str` returning the command name).
- `HotkeyLayout`: `DEFAULT_DIRECT_NAMES` and `DEFAULT_GROUPS` use the new names (e.g. `save_game` → `save-game`, `zoom_in` → `zoom-map`, `recenter` → `center-map`, `restore_game` → `load-game`, `select_next` → `select-room`). `is_direct(name: &str)`.
- The hotkey dialog list iterates `slash::COMMANDS` grouped by `Category` (replacing the `DEFAULT_GROUPS` snake-case lists if you choose category grouping; keep `DEFAULT_GROUPS` only if you prefer the curated grouping — pick one and make the dialog read from it consistently).

- [ ] **Step 1: Write the failing test**

```rust
// in keymap.rs tests
#[test]
fn hotkey_defaults_use_registry_names() {
    for name in DEFAULT_DIRECT_NAMES {
        assert!(crate::slash::find_command(name).is_some(), "direct name not in registry: {name}");
    }
    for (_title, names) in DEFAULT_GROUPS {
        for n in *names {
            assert!(crate::slash::find_command(n).is_some(), "group name not in registry: {n}");
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app hotkey_defaults_use_registry_names`
Expected: FAIL (old snake_case names like `save_game` not in the registry).

- [ ] **Step 3: Write minimal implementation**

Update `DEFAULT_DIRECT_NAMES`/`DEFAULT_GROUPS` to registry names; re-point `hint_bar` and the dialog renderer/handler to the registry. Locate the hint constants:

Run: `rg -n "GAME_HINTS|MAP_HINTS|ANIM_HINTS" crates/app/src/main.rs` — update those name lists to registry names too.

- [ ] **Step 4: Run tests + manual check**

Run: `cargo test -p app && cargo build -p app`
Expected: green, 0 warnings. Manual: open the hotkey dialog (Ctrl+K), confirm commands list with keys; confirm the bottom hint bar shows correct keys in Game/Map/Anim.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/main.rs crates/app/src/keymap.rs crates/app/src/input.rs crates/app/src/render
git commit -m "feat(app): hint bar + hotkey dialog read the command registry"
```

### Task 12: Updated default `config.toml` + docs

**Files:**
- Modify: the shipped sample/default config (locate with `rg -ln "command_prefix|\\[keymap\\]" --glob '*.toml' .` and `rg -ln "keymap" README.md docs`)
- Modify: `README.md` (slash-command / keymap section), and `docs/` keymap notes if present

**Interfaces:**
- Documents the new `[keymap]` format (`use_defaults` + `[keymap.global|map|anim]` `key = "command args"`), the verb-noun command names, `/help` and `help <command>`.

- [ ] **Step 1: Write the failing test**

Docs/config have no unit test; the gate is a round-trip parse of the shipped example. Add to `config.rs` tests:

```rust
#[test]
fn shipped_keymap_example_parses() {
    let toml = r#"
[keymap]
use_defaults = true
[keymap.map]
"+" = "zoom-map in"
"c" = "center-map"
"#;
    let cfg: Config = toml::from_str(toml).unwrap();
    let (km, warns) = crate::keymap::KeyMap::resolve(&cfg.keymap);
    assert!(warns.is_empty());
    let c: crate::keymap::KeySpec = "c".parse().unwrap();
    assert_eq!(km.lookup(&c, crate::keymap::Context::Map), Some("center-map"));
}
```

- [ ] **Step 2: Run test to verify it fails (or passes)**

Run: `cargo test -p app shipped_keymap_example_parses`
Expected: PASS if Tasks 7-8 are in; this test guards the documented example stays valid.

- [ ] **Step 3: Update docs + sample config**

Edit `README.md` and any sample config to the new format and names. Keep examples copy-pasteable.

- [ ] **Step 4: Run full suite**

Run: `cargo test -p app && cargo build -p app`
Expected: green, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add README.md docs crates/app/src/config.rs
git commit -m "docs(app): document the unified command/keymap system + new config format"
```

---

## Self-Review

**Spec coverage:**
- Registry + verb-noun names + categories → Tasks 1-2. ✓
- parse() over registry + context gating → Task 3. ✓
- Grouped /help + help <command> + hanging-indent wrap → Tasks 4-6. ✓
- Keys as (context,key)→command-string + use_defaults clean-slate → Tasks 7-9. ✓
- Retire Command enum → Task 10. ✓
- Hint bar + hotkey dialog re-point → Task 11. ✓
- Default config + docs → Task 12. ✓
- No migration layer; skip-and-warn unknown names → Task 8 `resolve`. ✓
- Hardwired Ctrl+Q/C + Esc survive → preserved in `key_to_command` (Task 9, hardwired path unchanged). ✓

**Placeholder scan:** No "TBD"/"add error handling"; each code step shows code. Two tasks (9, 11) reference "locate with rg" for files whose exact path the implementer must confirm in the live tree — these are discovery steps with the exact ripgrep command given, not placeholders.

**Type consistency:** `CommandSpec.dispatch: fn(&[&str]) -> SlashOutcome` used consistently; `KeyMap::lookup -> Option<&str>` consumed by `key_to_command` (Task 9) and `resolve` produces `(KeySpec, String, Context)`; `find_command` signature stable across Tasks 2/3/8/11; `SlashOutcome::HelpCommand(String)` defined in Task 4 and consumed in Task 5.

**Notes for the executor:**
- Tasks 1-6 (Wave 1) leave key bindings on the live `Command` path — the app stays fully usable. Do not delete `Command` until Task 10.
- The `Action` enum is never removed; it remains the execution unit.
- After Task 9, run the app manually before Task 10 to confirm key dispatch parity (the riskiest change).
