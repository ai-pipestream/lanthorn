# Configurable Keymap + Help Screen — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — awaiting spec review
**TODO items:** "Configurable key-map, with update hints at bottom of screen" and "Help screen" (folded together — both are keymap-driven).

## Goal

Make the named map/global commands rebindable via config, keep a bottom hint bar that reflects the current bindings, and add a full-screen help overlay listing every binding. Defaults reproduce today's key handling exactly. Text entry, prompts, and Ctrl+Q-quit stay hardwired.

## Scope (decided)

**Named commands only.** The ~35 arg-free / fixed-arg commands are rebindable. NOT rebindable: Game-focus text entry (printable chars → `InputChar`, `Enter` → `SubmitCommand`, `Backspace`, `Esc`), the prompt sub-mode keys, and `Ctrl+Q`/`Ctrl+C` → `Quit`. Arg-carrying actions become fixed-direction commands.

## Architecture

App-side `KeyMap`, resolved from config into `AppState`; `key_to_action` consults it. Mirrors the `SymbolSet` pattern (see `2026-06-23-configurable-symbols-design.md`). The layered dispatch ORDER in `key_to_action` is preserved; only the two named-command layers (global Ctrl block, Map-focus letter match) and the Anim sub-mode become KeyMap lookups.

```
config.toml [keymap] ─▶ KeyMap::resolve(&Config) ─▶ AppState.keymap ─▶ key_to_action lookups + hint bar + help screen read it
```

## Components

### 1. `crates/app/src/keymap.rs` (new)

- `enum Context { Global, Map, Anim }`.
- `enum Command` — the rebindable commands, each convertible to an `Action`:
  - Global: `Quit` (Note: `Ctrl+Q`/`Ctrl+C` remain a hardwired quit that always wins; `Quit` here is the *rebindable* quit binding), `SaveGame`, `RestoreGame`, `ExportSvg`, `ExportDot`, `ExportDump`, `ToggleFocus`, `ToggleHelp`.
  - Map: `CycleLayout`, `Retidy`, `AnimateTidy`, `ZoomIn`, `ZoomOut`, `Recenter`, `SelectNext`, `SelectPrev`, `RenameRoom`, `RenameLayer`, `EditNotes`, `DeleteSelectedConnection`, `RelabelSelectedEdge`, `ToggleAlignment`, `TogglePortalLabels`, `ToggleInspector`, `PeelLayer`, `MergeLayer`, `PanLeft`, `PanRight`, `PanUp`, `PanDown`, `NudgeLeft`, `NudgeRight`, `NudgeUp`, `NudgeDown`, `CycleLayerNext`, `CycleLayerPrev`.
  - Anim: `AnimStepFwd`, `AnimStepBack`, `AnimTogglePlay`, `AnimExit` (plus pan/zoom reuse the Map pan/zoom commands in the Anim context).
- `fn Command::to_action(self) -> crate::input::Action` — e.g. `PanLeft => Action::Pan(-1, 0)`, `NudgeUp => Action::NudgeSelected(0, -1)`, `CycleLayerNext => Action::CycleLayer(1)`, `AnimStepFwd => Action::AnimStep(1)`, `RenameRoom => Action::RenameRoom`, etc.
- `struct KeySpec { code: crossterm::event::KeyCode, ctrl: bool, shift: bool, alt: bool }` with `FromStr`: parse `"ctrl+"`, `"shift+"`, `"alt+"` prefixes (any order) then a key token — a single char (`s`, `+`, `?`), or a named key (`left`, `right`, `up`, `down`, `tab`, `space`, `esc`, `enter`, `f1`..`f12`). A `Display`/`label()` for the hint bar (e.g. `Ctrl+S`, `Shift+←`, `h`).
- `struct KeyMap { bindings: Vec<(KeySpec, Command, Context)> }`. Multiple keyspecs MAY map to one command (defaults keep both `hjkl` and `Shift+arrows` for pan). Lookups: `fn lookup(&self, spec: &KeySpec, ctx: Context) -> Option<Command>` (Map lookups also fall through to Global). `fn primary_key(&self, cmd: Command) -> Option<&KeySpec>` for hints/help.
- `KeyMap::default()` — every current binding, transcribed from today's `key_to_action`/`prompt`/anim dispatch. (Binding inventory below.)
- `KeyMap::resolve(cfg: &KeymapConfig) -> KeyMap` — start from default; apply overrides; conflict handling (below).

### 2. Config integration (extends Track B's `Config`)

```toml
[keymap]
toggle_focus = "tab"
save_game    = "ctrl+s"
zoom_in      = "+"
pan_left     = "h"
toggle_help  = "f1"
```

- `#[derive(Deserialize, Default)] struct KeymapConfig { #[serde(default)] overrides: BTreeMap<String, String> }` (`command_name → keyspec`); add `#[serde(default)] keymap: KeymapConfig` to `Config`. Command names are snake_case of the `Command` variant.
- An override value may be a comma-separated list to bind several keys to one command (e.g. `pan_left = "h, shift+left"`). A command absent from `[keymap]` keeps its default binding(s).

### 3. `key_to_action` refactor (`crates/app/src/input.rs`)

Preserve the exact layer ORDER:
1. `Ctrl+Q`/`Ctrl+C` → `Quit` (hardwired, always).
2. Prompt sub-mode active → `prompt_key_to_action` (hardwired).
3. Tidy-anim sub-mode active → KeyMap lookup in `Context::Anim` (pan/zoom/step/play/exit); unmatched → `Action::None`.
4. Game focus: text entry stays hardwired (printable → `InputChar`, `Enter` → `SubmitCommand`, `Backspace`, `Tab` → autocomplete-or-`ToggleFocus` per the existing L14 rule, `Esc`); THEN `Context::Global` KeyMap lookup for the rest (Ctrl+S etc.).
5. Map focus: `Context::Map` (falling through to `Global`) KeyMap lookup → `command.to_action()`; unmatched → `Action::None`.

`key_to_action` takes `&AppState` (already does) and reads `state.keymap`.

### 4. Hint bar (`crates/app/src/main.rs`)

Replace the hardcoded `help_text` strings in `draw_frame` with a **curated per-context shortlist**: a fixed `&[(Command, &str label)]` per context (Map / Game / Anim). Render each as `<key>: <label>`, the `<key>` from `keymap.primary_key(cmd).label()`, so rebinding updates the hint. Append a fixed `F1: help` hint.

### 5. Conflict handling

In `resolve`, applying overrides: if a keyspec would bind a key already bound to a DIFFERENT command in the same context, reject that one override and keep the default; record a short message the app shows once on the status line (e.g. "keymap: 'x' already bound; kept default"). Defaults are conflict-free by construction.

### 6. Help screen (`crates/app/src/render/help.rs`, new)

- `Command::ToggleHelp` + `Action::ToggleHelp` flips `AppState.show_help: bool` (default false).
- When `show_help`, `draw_frame` renders a centered full-pane overlay listing ALL bindings grouped by `Context` (Global, Map, Tidy-anim), each row `<key(s)>  <command label>`, keys read live from the `KeyMap`. `Esc` or the toggle key closes it.
- Default binding: `F1` (works in both Game and Map focus, no text-input collision), plus `?` in the Map context. Both rebindable.

## Defaults & back-compat

`KeyMap::default()` reproduces every current binding; with no `[keymap]` config the app behaves identically. The large existing `input.rs` test suite is the primary back-compat guard, supplemented by equivalence tests asserting `key_to_action` yields the same `Action` for a representative key sample.

### Binding inventory (from today's dispatch — the defaults)

- Global: `Tab`→ToggleFocus, `Ctrl+S`→SaveGame, `Ctrl+R`→RestoreGame, `Ctrl+E`→ExportSvg, `Ctrl+L`→(export dot — confirm), `Ctrl+D`→ExportDump (confirm exact ctrl letters against `input.rs` during implementation).
- Map: `c`→Recenter, `n`→SelectNext, `p`→SelectPrev, `+`/`=`→ZoomIn, `-`→ZoomOut, `h/j/k/l` and `Shift+←↓↑→`→Pan*, `Ctrl+←↓↑→`→Nudge*, `[`/`]`→CycleLayerPrev/Next, `P`→PeelLayer, `M`→MergeLayer, `N`→RenameLayer, `r`→RenameRoom, `o`/`d`/`e`→(edit family — confirm), `i`→ToggleInspector, plus the layout/tidy/alignment/portal toggles (confirm exact keys from `main.rs` help banner + `input.rs`).
- Anim: `←`/`→`→AnimStepBack/Fwd, `Space`→AnimTogglePlay, `Esc`/`Enter`→AnimExit, `h/j/k/l`+`Shift+arrows`→Pan, `+`/`-`→Zoom.

(The implementer transcribes the EXACT current bindings from `input.rs` and the `main.rs` help strings — these are the source of truth — and a test pins `KeyMap::default()` to them.)

## Testing

- `KeySpec` parse/label round-trips: `"ctrl+s"`, `"shift+left"`, `"+"`, `"tab"`, `"f1"`, comma lists.
- `KeyMap::default()` maps a representative key sample to the expected commands; `key_to_action` equivalence before/after the refactor for that sample (covering each sub-mode).
- `resolve` applies an override; a conflicting override is rejected and the default kept.
- `Command::to_action` mapping for the directional/anim commands.
- Hint bar uses the rebound key (rebind `zoom_in` to `z`, assert the Map hint shows `z: zoom`).
- Help overlay: lists a known binding; toggles open/closed; reflects a rebinding.

## Out of scope / non-goals

- Rebinding text-entry / prompt / `Ctrl+Q`-quit.
- Per-OS or chord (multi-key) sequences.
- `mapper` changes (none — input is app-side).
- Mouse bindings (separate TODO).

## Risks & limitations (accepted)

- **Most invasive refactor so far.** `input.rs` is ~1200 lines and the dispatch order is delicate; the existing test suite + equivalence tests are the guard. The refactor is mechanical (literal arms → table lookups) per layer.
- **Key-name coverage.** `KeySpec` parsing supports the keys the app actually uses; an unknown key token in config is rejected (default kept) with a status message.
