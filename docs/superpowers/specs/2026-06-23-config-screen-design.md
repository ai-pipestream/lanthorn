# Configuration Screen — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued. Touches `config.rs`, `state.rs`, `main.rs`, `input.rs`, `keymap.rs` + a new render module.
**TODO item:** "Configuration screen for all options, command-line options (command-line takes precedent over config file)" (L23).

## Goal

An in-app modal to view and edit the **scalar/enum/preset** config options, with an explicit **Save** (writes `~/.babelmap/config.toml`, format-preserving, and applies to the running session) and **Cancel** (discard). CLI-overridden values are shown read-only/marked (command-line already wins this session via `Config::resolve`). Complex list/map options (keymap bindings, symbol per-glyph overrides, color elements, hotkey groups) are edited via their dedicated tools (hotkey dialog, gallery, config file) and are out of scope here.

## Settings shown (v1)

| Row | Type | Values |
|-----|------|--------|
| `user_dir` | path | text-edit (reuse the prompt sub-mode) |
| `use_default_map` | bool | toggle |
| `auto_load` | bool | toggle |
| `auto_save` | bool | toggle |
| `record_history` | bool | toggle |
| `background_tidy` | enum | off / every_room / on_overlap / debounced |
| `colors.scheme` | choice | (none) / mono / high-contrast / tomorrow-night / (custom path via text-edit) |
| `symbols.box_style` | enum | from `BoxStyle::preset_names()` (rounded/thick/double/ascii/borderless) |
| `symbols.arrow_set` | enum | from `Arrows::preset_names()` |
| `symbols.portal_icons` | enum | from `PortalGlyphs::preset_names()` |
| `symbols.path_style` | enum | from `PathGlyphs::preset_names()` |

Enum value sets come from the existing `*::preset_names()` (the gallery already uses them) and the `BackgroundTidy` variants.

## Architecture — runtime Config in AppState

To apply changes live, the resolved runtime `Config` moves into `AppState`:
- `AppState.config: Config` — set at startup from `Config::resolve(&cli)`; the event loop reads `state.config.auto_save` / `auto_load` / `background_tidy` / `use_default_map` / `record_history` (currently read from a local `cfg` — repoint these reads to `state.config`). `map_dir(&state.config.user_dir)` likewise.
- `AppState.config_screen: Option<ConfigScreenState { working: Config, selected: usize, editing: Option<EditKind> }>` — `working` is a clone edited in the modal; `None` = closed.

## UX

A modal opened by `Command::OpenConfig` (keymap, default key + hotkey dialog group). A sub-mode in `key_to_action` (like saves/gallery):
- `↑/↓` move the selected row; the selected row shows its value highlighted; CLI-overridden rows are marked `(cli)` and skipped for editing.
- **bool:** `Space`/`Enter` toggles `working.<field>`.
- **enum/choice:** `←/→` cycles the value among its set.
- **path/custom:** `Enter` opens a text-edit prompt (reuse `PromptKind`), writing into `working`.
- **`s` = Save:** apply `working` → `state.config`; re-resolve `state.symbols = SymbolSet::resolve(&working.symbols)` and `state.colors = ColorScheme::resolve(&working.colors, &working.user_dir).0`; `config::write_config(&working.user_dir, &working)`; close.
- **`Esc`/`q` = Cancel:** drop `working`, close (no changes).

## Persistence — `config::write_config`

`pub fn write_config(dir: &Path, cfg: &Config) -> std::io::Result<()>` — load `dir/config.toml` with `toml_edit` (or new doc), set the scalar/enum/scheme/preset keys from `cfg` (`use_default_map`, `auto_load`, `auto_save`, `record_history`, `background_tidy`, `user_dir`, `[colors].scheme`, `[symbols]` four presets), PRESERVING all other keys/comments (keymap, overrides, elements, groups untouched). Extends the existing `write_symbols` (gallery) pattern to the full scalar set.

## Components

- **`crates/app/src/render/config_screen.rs` (new)** — `draw_config_screen(state, area, buf)`: the settings list with name/value columns, the selected row, `(cli)` markers, and a footer (`↑↓ move · ←→/Space change · s save · Esc cancel`). Opaque background (`Style::reset().bg(...)`).
- **`state.rs`** — `config: Config`, `config_screen: Option<ConfigScreenState>`, `EditKind`.
- **`input.rs`** — `Action::OpenConfig`, `ConfigNav`, `ConfigToggle`, `ConfigCycle(i32)`, `ConfigEdit`, `ConfigSave`, `ConfigCancel`; the sub-mode router; `apply_action` mutates `working` / commits on Save.
- **`keymap.rs`** — `Command::OpenConfig` (+ default key; hotkey dialog group).
- **`config.rs`** — `write_config`.
- **`main.rs`** — store `state.config` at startup; repoint the loop's `cfg.*` reads to `state.config.*`; render the modal.

## CLI precedence

`Config::resolve` already applies defaults < file < CLI. The screen edits FILE values; a value overridden by a CLI flag this session (e.g. `--user-dir`) is shown read-only with a `(cli)` marker — editing it writes the file (takes effect next launch without the flag) but does not change the live session. v1 only `user_dir` can be CLI-overridden (the `Cli` struct), so only it can be marked.

## Testing

- `write_config` round-trip: write scalars+presets into a temp `config.toml` containing an unrelated `[keymap]` + comments; re-read; assert the scalar/preset keys are set AND `[keymap]`/comments survive.
- Config-screen edits: `ConfigToggle` flips a bool in `working`; `ConfigCycle` advances an enum within its value set; Save copies `working`→`state.config`, re-resolves `state.symbols`/`state.colors`, and calls `write_config`; Cancel leaves `state.config` unchanged.
- Loop reads: `state.config.auto_save` etc. drive the same behavior the old `cfg` did (a small smoke check).
- Render test (TestBackend): the modal lists a known setting + value; opaque background.
- The toggle key → `Action::OpenConfig`.

## Out of scope / non-goals

- Editing keymap bindings, symbol per-glyph overrides, color `elements`, or hotkey groups inline (use the hotkey dialog / gallery / config file).
- Validating arbitrary paths exist.
- A separate "reset to defaults" action (could be added later).
- `mapper`/`zvm` changes.

## Risks & limitations (accepted)

- **Config-in-AppState refactor:** moving the runtime `Config` into `AppState` repoints a handful of loop reads; mechanical and covered by existing behavior tests.
- **Live-apply scope:** symbols/colors re-resolve immediately; the loop-scalars apply from the next loop iteration. No restart needed for any v1 setting.
- **`background_tidy` mid-session change** takes effect from the next turn — expected.
