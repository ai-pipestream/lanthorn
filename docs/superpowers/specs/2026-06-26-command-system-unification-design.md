# Command-System Unification — Design

**Date:** 2026-06-26
**Status:** Approved design (pending spec review) → implementation plan next
**Scope:** Unify slash commands and key bindings into one command registry. Standardize every command to a verb-noun name, group them into categories, and make key bindings into stored `(context, key) → command-string` entries that run through the same parser as typed slash input. Supersedes the four "Slash Commands" TODO items.

## Goal

Make the slash command the single source of truth for every action, and make a key binding nothing more than a stored command-string (with optional arguments) that runs through the same parser. A user can bind any key to any command with any arguments, and `/help` lists every command — grouped by category — with no second naming scheme to keep in sync.

## Background (current two-universe state)

Today there are two parallel command systems:

- **`Command` enum (`keymap.rs`)** — ~55 named, bindable variants with `name()` (snake_case), `label()`, and `context()` (Global/Map/Anim). Key bindings resolve `key → Command → Action`. Cannot carry arguments (e.g. there are separate `ZoomIn`/`ZoomOut`/`ZoomReset` variants, `PanLeft/Right/Up/Down`, `CycleLayerNext/Prev`, `NudgeLeft/Right/Up/Down`).
- **Curated table (`slash.rs`)** — 16 entries plus aliases, each a `build: fn(&[&str]) -> SlashOutcome` closure. The slash parser checks curated first, then falls back to `Command::from_name`. Curated entries carry argument parsing and richer-than-`Action` outcomes (`Save`/`Load`/`Reset`/`Quit`/`Help`/`Search`/`Filter`/`Export`/`OpenHints`).

Consequences: names are inconsistent (`center`, `tidy`, `zoom`, `panh` vs `open-config`, `reset-game`); the same concept exists twice (`save`/`SaveGame`, `center`/`Recenter`, `zoom`/`ZoomIn+ZoomOut+ZoomReset`); `/help` must merge two sources; and a key cannot be bound to a command with a chosen argument.

## Design overview

One **command registry** holds every command's metadata and dispatch. The parser looks a command up by name and runs its dispatch closure with the remaining tokens. Key bindings become data — `(context, key) → command-string` — resolved on keypress by feeding the string to the **same parser**. The `Command` enum is retired as the naming/binding layer; execution still flows through the existing `Action` enum (via `SlashOutcome::Action`) and the existing `SlashOutcome` variants.

Directional/variant commands collapse into **parametric** commands; the default keymap supplies the arguments per key. This collapses ~55 enum variants to 48 verb-noun commands (the directional variants become key bindings, not separate commands).

Delivered in three waves, each leaving the app green and usable:

- **Wave 1** — Build the registry: verb-noun names, categories, per-command usage + description, the parametric collapse, and grouped `/help` with `help <command>` detail and hanging-indent wrap. Slash typing uses the registry. Key bindings still work via the existing `Command`→key path during this wave (the registry and the enum coexist until Wave 2 removes the enum).
- **Wave 2** — Key bindings become `(context, key) → command-string`; a built-in default keymap; the `keymap.use_defaults` clean-slate switch; keypresses dispatch through the registry parser. Retire the `Command` enum.
- **Wave 3** — Re-point the hint bar and the hotkey (leader-key) dialog onto the registry; ship the updated default `config.toml` and docs.

## Naming standard

Every command name is `verb-noun` (kebab-case), with exactly two conventional one-word exceptions: **`quit`** and **`help`**. Names are unique across the registry. The parametric collapse means directional behavior is expressed by arguments, not by separate command names.

### Full command table

Each row is `name` `usage` — `description` (category). Commands marked **(args)** parse arguments; the rest take none.

**Game**
- `save-game [name]` — save the game, optionally to a named slot. (args)
- `load-game [name]` — load a save, optionally a named slot. (args)
- `reset-game [map]` — restart the game; `reset-game map` also clears the map. (args)
- `quit` — exit babelmap.
- `open-hints` — open the hints panel.
- `open-history` — open the rewind/replay history.
- `open-verb-menu` — open the verb/item palette.
- `open-saves` — open the saves manager.

**Map**
- `pan-map <dx> <dy>` — pan the map by dx columns and dy rows. (args)
- `zoom-map in|out|reset|<n>` — zoom the map in/out, reset to default, or step by signed n. (args)
- `center-map` — re-center the map on the selected room.
- `tidy-map` — re-run the layout tidy.
- `cycle-layer next|prev|<n>` — switch map layer; n is a signed delta. (args)
- `select-room next|prev` — move the room selection. (args)
- `nudge-room <dx> <dy>` — nudge the selected room by dx, dy cells. (args)
- `rename-room` — rename the selected room.
- `rename-layer` — rename the current layer.
- `edit-notes` — edit the selected room's notes.
- `delete-connection` — delete the selected connection.
- `relabel-edge` — relabel the selected edge.
- `peel-layer` — peel the selected layer into its own view.
- `merge-layer` — merge the selected layer down.
- `toggle-inspector` — toggle the room-inspector overlay.
- `toggle-room-numbers` — toggle room-number labels.
- `toggle-loc-method` — toggle the room-detection-method indicator.
- `toggle-alignment` — toggle alignment guides.
- `toggle-portal-labels` — toggle portal labels.
- `open-gallery` — open the symbol gallery.

**View**
- `cycle-layout [reverse]` — cycle the pane layout (Split / Map / Transcript); `reverse` cycles backward. (args)
- `toggle-focus` — switch focus between panes.
- `toggle-inventory` — toggle the inventory strip.
- `toggle-status-bar` — toggle the status/score bar.

**Transcript**
- `search-transcript [query]` — search the transcript; no query repeats the last search. (args)
- `filter-transcript story|meta|both` — filter the transcript by category. (args)
- `export-transcript [file]` — export the visible transcript; default path when omitted. (args)

**Style**
- `open-style-editor` — open the live style editor.
- `open-config` — open the settings screen.
- `reload-style` — reload style.toml from disk.
- `create-game-style` — scaffold a per-game style file.
- `toggle-watch` — toggle live style-file watching.

**Export**
- `export-svg` — export the map as SVG.
- `export-dot` — export the map as Graphviz DOT.
- `export-dump` — dump the map structure.

**Animation**
- `animate-tidy` — animate a tidy pass.
- `anim-step forward|back` — step the animation one frame. (args)
- `anim-play` — toggle animation play/pause.
- `anim-exit` — exit the animation view.

**Help**
- `help [command]` — list all commands by category; with a command name, show that command's usage and description. (args)

## Components

### 1. Command registry (Wave 1)

A single static table is the source of truth. Each entry carries: canonical `name`, `category`, `context`, `usage` string, one-line `description`, and a `dispatch` closure converting argument tokens to a `SlashOutcome`. No-argument commands return `SlashOutcome::Action(...)`; argument or special-outcome commands return the appropriate variant (`Save`/`Load`/`Reset`/`Quit`/`Help`/`Search`/`Filter`/`Export`/and `Action` for `pan-map`/`zoom-map`/`cycle-layer`/etc.).

- `Category` is a new enum (`Game, Map, View, Transcript, Style, Export, Animation, Help`), independent of `Context`.
- `Context` (`Global, Map, Anim`) is carried over from the current `Command::context()`.
- The registry replaces both the curated table and (in Wave 2) the `Command` enum's naming role. Argument parsing that used to live in curated builders lives in the registry dispatch closures.

### 2. Parser & dispatch (Wave 1)

`parse(body, prefix) -> SlashOutcome`: tokenize the body, look up token 0 as a registry name, run its `dispatch(args)`. Unknown name → `Error("unknown command: <prefix><name> — try <prefix>help")`. Empty body → `Error`. `search-transcript` preserves internal whitespace in its query (current behavior). `help` accepts an optional command-name argument.

Context gating: when a command is invoked **by typing** outside its `context` (e.g. `anim-step forward` while not in the animation modal), dispatch returns a friendly `Error` ("anim-step is only available during animation playback") rather than firing. Commands with `Context::Global` are always valid; map commands are valid when a map is present (already the effective behavior).

### 3. Keymap as data (Wave 2)

A key binding is `(Context, KeyChord) -> command-string`. A built-in `DEFAULT_KEYMAP` table ships in code (the bindings that exist today, expressed as command-strings — e.g. `(Map, "+") -> "zoom-map in"`, `(Map, "left") -> "pan-map -1 0"`). User config overrides/extends per context:

```toml
[keymap]
use_defaults = true        # false = ignore DEFAULT_KEYMAP entirely (clean slate)

[keymap.global]
"ctrl+s" = "save-game"
"tab"    = "toggle-focus"

[keymap.map]
"left" = "pan-map -1 0"
"+"    = "zoom-map in"
"c"    = "center-map"

[keymap.anim]
"l" = "anim-step forward"
"h" = "anim-step back"
```

On keypress: resolve `(active_context, key) → string`, then `parse(string, prefix)` → `SlashOutcome`, dispatched by the run loop exactly as typed input is. Keypresses for non-command input (text editing in the input line, scrolling, mouse) are unchanged and do not route through the registry.

`keymap.use_defaults = false` loads an empty default set, so the user's `[keymap.*]` sections are the entire keymap. The hardwired **Ctrl+Q / Ctrl+C quit** and **Esc-closes-modal** remain regardless, as anti-lockout safeties.

The leader-key prefix model is preserved: direct vs prefixed is a property of which binding table a key lives in (the existing `is_direct` rule), now expressed over command-strings.

The `Command` enum is removed in this wave; `Action` (execution) and `SlashOutcome` (dispatch) remain.

### 4. /help rendering (Wave 1)

`help` with no argument prints, to the transcript as Meta lines, the commands **grouped by category** in a fixed category order, each as `  <prefix><usage>  — <description>`. `help <command>` prints just that command's usage and description (error if unknown).

Hanging-indent wrap: `/help` lines that exceed the transcript width wrap with their continuation rows indented to align under the description text, instead of starting at column 0. This is implemented as a wrap variant that takes a hanging-indent column and is applied to the Meta lines `/help` emits. (Today `wrap_line` is indentation-unaware; the `0bd2501` spec for honoring gutter indent on wrapped meta lines is the related groundwork.)

### 5. Hint bar + hotkey dialog (Wave 3)

- **Hint bar** iterates the active context's key bindings and shows `key → the bound command's short label/description` (label derived from the registry, replacing `Command::label()`). Same filtering rules as today (direct bindings that resolve in the current context).
- **Hotkey dialog** (leader-key) iterates the registry grouped by **category**, showing each command's bound key (if any), name, and description.

Both stop referencing the retired `Command` enum.

## Error handling

- Unknown command name (typed) → `Error("unknown command: …— try …help")`; never panics.
- Bad/missing arguments → the command's dispatch returns a specific `Error` (e.g. `"pan-map requires two integers (e.g. pan-map -3 0)"`), matching current curated-builder behavior.
- Command typed outside its context → friendly `Error` naming the context, no side effect.
- **Unknown command name in a `[keymap.*]` config entry** → the binding is skipped and a one-line warning is surfaced (transcript Warning); the rest of the config loads. This is ordinary robustness, not a migration layer.
- `keymap.use_defaults = false` with no user bindings → a keymap with only the hardwired quit/Esc safeties; valid, not an error.

## Testing strategy

- **Registry invariants:** names are unique; every entry has a non-empty `usage` and `description` and a `category`; a verb-noun lint passes for all names except the `quit`/`help` whitelist.
- **Parser:** `parse()` routes every registry name to the right `SlashOutcome`; argument commands parse representative inputs (`pan-map -3 0`, `zoom-map in`, `cycle-layer next`, `save-game slot`, `filter-transcript meta`, `select-room prev`, `anim-step back`); unknown name and empty body → `Error`; `search-transcript` preserves internal whitespace; `help foo` for unknown → `Error`.
- **Context gating:** an Anim-context command typed in Global context returns an `Error`; a Global command is accepted in any context.
- **Key-string dispatch parity (Wave 2):** binding a key to a command-string and pressing the key produces the same `SlashOutcome` as typing that command; `use_defaults = false` yields only the hardwired safeties plus user bindings; an unknown command name in a `[keymap.*]` entry is skipped with a warning and does not abort config load.
- **/help (Wave 1):** grouped output lists every registry command under its category in order; `help zoom-map` prints that command's usage + description; long `/help` lines wrap with hanging indent aligned under the description.
- **Hint bar + dialog (Wave 3):** the hint bar shows the active-context bindings with registry-derived labels; the hotkey dialog lists commands grouped by category.

## Out of scope

- A graphical key-binding editor (config is edited as TOML).
- Per-command argument autocomplete beyond the existing command-name Tab completion.
- New commands beyond renaming/parametrizing the existing set.
- Any name-migration layer for old config/command names (clean break; the only config robustness is skip-and-warn for unknown names).
- Mouse-action rebinding (mouse handling is unchanged).

## Open questions

None blocking. The exact category ordering in `/help`, the precise default key→command-string bindings, and the hanging-indent column may be refined during planning.
