# Hotkey Dialog (leader-key command palette) — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — supersedes the F1 help screen (L24).
**Revises:** the L8 keymap interaction model. Keeps the L8 `KeyMap`/`Command`/`KeySpec` data; changes how commands are *reached* and *displayed*.

## Goal

Replace the always-on flat keymap + F1 help overlay with a **leader-key model**: a configurable prefix key opens a **sticky hotkey dialog** that lists every command (grouped/ordered per config) with its key. Pressing a command's key runs it and the dialog stays open; the prefix again or `q` closes it. A configurable subset of commands ("direct") still work without the prefix. The dialog replaces the F1 help screen.

## Decided behavior

- **Prefix key:** configurable, **modified** (so it works in both Game and Map focus without clashing with typed text). Default **`Ctrl+K`**.
- **Direct commands:** work as normal keybindings without the prefix. Default direct set: `save_game`, `restore_game`, `quit`, and navigation (`pan_left/right/up/down`, `zoom_in`, `zoom_out`, `select_next`, `select_prev`, `recenter`).
- **Dialog-only commands:** everything else — only fire while the dialog is open.
- **Dialog is sticky:** running an in-place command keeps it open; **prefix again or `q`** closes it. `q` is reserved while the dialog is open, so no command may use `q` as its dialog key.
- **Commands that open another sub-mode** (gallery, saves, a rename/notes/relabel prompt) **close the dialog** as they take over (no nested modals).
- **F1 help screen removed:** `render/help.rs`, `Command::ToggleHelp`, `Action::ToggleHelp`, `AppState.show_help`, and the `F1`/`?` help bindings all go away.
- **Bottom hint bar** simplifies to the prefix + direct keys, e.g. `Ctrl+K: commands · Ctrl+S: save · Ctrl+R: restore · Ctrl+Q: quit`.

## Config — new `[hotkeys]` section

Keys still live in `[keymap]` (L8, unchanged). `[hotkeys]` decides direct-vs-dialog and dialog layout only.

```toml
[hotkeys]
prefix = "ctrl+k"
direct = ["save_game","restore_game","quit",
          "pan_left","pan_right","pan_up","pan_down",
          "zoom_in","zoom_out","select_next","select_prev","recenter"]

[[hotkeys.group]]
title = "Layout"
commands = ["retidy","animate_tidy","cycle_layout"]
[[hotkeys.group]]
title = "Layers"
commands = ["peel_layer","merge_layer","cycle_layer_next","cycle_layer_prev","rename_layer"]
[[hotkeys.group]]
title = "Edit"
commands = ["rename_room","edit_notes","delete_selected_connection","relabel_selected_edge"]
[[hotkeys.group]]
title = "Files"
commands = ["open_saves","export_svg","export_dot","export_dump"]
[[hotkeys.group]]
title = "View"
commands = ["toggle_alignment","toggle_portal_labels","toggle_inspector","open_gallery"]
```

A built-in default `[hotkeys]` (the values above) is used when config is absent. Command names are the existing snake_case `Command::name()` values; unknown names in config are dropped with a warning (like `[keymap]`).

## Config struct (extends Track B `Config`)

```rust
#[derive(Debug, Default, Deserialize)]
pub struct HotkeysConfig {
    #[serde(default)] pub prefix: Option<String>,         // KeySpec string; None → default
    #[serde(default)] pub direct: Option<Vec<String>>,    // command names; None → default set
    #[serde(default)] pub group: Vec<HotkeyGroupConfig>,  // [[hotkeys.group]]
}
#[derive(Debug, Deserialize)]
pub struct HotkeyGroupConfig { pub title: String, pub commands: Vec<String> }
```
`Config` gains `#[serde(default)] pub hotkeys: HotkeysConfig`.

## Architecture (evolves `keymap.rs`, no new foundation)

1. **`keymap.rs` — `HotkeyLayout`:**
   ```rust
   pub struct HotkeyLayout {
       pub prefix: KeySpec,
       pub direct: std::collections::HashSet<Command>,
       pub groups: Vec<(String, Vec<Command>)>,
   }
   impl HotkeyLayout {
       pub fn default() -> HotkeyLayout;                 // the built-in layout above
       pub fn resolve(cfg: &crate::config::HotkeysConfig) -> (HotkeyLayout, Vec<String>);
       pub fn is_direct(&self, cmd: Command) -> bool;
   }
   ```
2. **`AppState`:** add `pub hotkeys: HotkeyLayout` and `pub hotkey_dialog: bool` (default false). REMOVE `pub show_help`.
3. **`key_to_action` dispatch order (`input.rs`)** — extend the existing layered scheme:
   1. `Ctrl+Q`/`Ctrl+C` → Quit (hardwired, unchanged).
   2. prompt sub-mode (unchanged).
   3. tidy-anim sub-mode (unchanged).
   4. gallery sub-mode (unchanged).
   5. saves sub-mode (unchanged).
   6. **NEW: hotkey dialog active** (`state.hotkey_dialog`) → `hotkey_dialog_key_to_action`: if key == `hotkeys.prefix` or `Char('q')` → `Action::CloseHotkeyDialog`; else `KeySpec::from_key_event` looked up across ALL contexts in the `KeyMap` → if a command is found, `command.to_action()` (the dialog stays open unless the command opens a sub-mode); else `Action::None`.
   7. **NEW: key == `hotkeys.prefix`** (in either focus) → `Action::OpenHotkeyDialog`.
   8. Global ctrl lookup → returns its command's action ONLY if `hotkeys.is_direct(cmd)`, else `Action::None`.
   9. `Tab` special-case (unchanged).
   10. Focus dispatch: Game text path unchanged; Map-context KeyMap lookup → action ONLY if the command `is_direct`, else `Action::None`. (Game-focus F1 fallthrough is removed with the help screen.)
4. **Actions:** `OpenHotkeyDialog` (sets `hotkey_dialog = true`), `CloseHotkeyDialog` (false). `apply_action`: when a command runs from the dialog and it activates another sub-mode (sets `prompt`/`gallery`/`saves`), also clear `hotkey_dialog`.
5. **`render/hotkeys.rs`** (new; `render/help.rs` deleted) — `draw_hotkey_dialog(state, area, buf)`: a centered overlay listing `hotkeys.groups`, each `title` then its commands as `<key>  <label>` (key via `keymap.primary_key(cmd).label()`), plus a footer `prefix / q: close`. Rendered from `draw_frame` when `state.hotkey_dialog`.
6. **Hint bar (`main.rs`):** replace the curated per-context list with `prefix-label: commands` + the direct save/restore/quit keys. (The L5/L8 hint plumbing stays; only the content shrinks.)

## Removals (F1 help screen)

- Delete `crates/app/src/render/help.rs` and its `render/mod.rs` registration.
- Remove `Command::ToggleHelp`, `Action::ToggleHelp`, `AppState.show_help`, the `F1`/`?` default bindings in `KeyMap::default()`, the `draw_help` call in `draw_frame`, and the Game-focus F1 fallthrough in `key_to_action`.
- Update/remove the help-screen tests; the equivalence sample test must drop `ToggleHelp`.

## Defaults & back-compat

`HotkeyLayout::default()` reproduces the config above; with no `[hotkeys]` config the app uses it. Existing `[keymap]` configs keep working (key bindings are independent). The hardwired `Ctrl+Q` quit always works regardless of the keymap.

## Testing

- `HotkeyLayout::default`: `is_direct` true for the navigation/save/restore/quit set, false for e.g. `retidy`/`open_gallery`; groups in the specified order.
- `resolve`: a `[hotkeys]` with a custom `direct` list and a reordered group resolves correctly; unknown command name → warning + dropped.
- Dispatch tests (`input.rs`): with the dialog CLOSED, a dialog-only command's key (e.g. `t`→retidy in Map) returns `Action::None`; a direct command's key still works; the prefix returns `OpenHotkeyDialog`. With the dialog OPEN, the same `t` returns `Retidy`, the prefix and `q` return `CloseHotkeyDialog`, and an in-place command leaves `hotkey_dialog` true.
- `apply_action`: running `open_gallery` from the dialog opens the gallery AND clears `hotkey_dialog`.
- Render test (TestBackend): `draw_hotkey_dialog` shows a group title and a known `<key> label` row; nothing drawn when `hotkey_dialog` is false.
- All other pre-existing `input.rs` tests still pass (save/restore/navigation unchanged); the removed help tests are deleted, not weakened.

## Out of scope / non-goals

- Multi-key chord sequences (the prefix opens a mode, it is not a 2-key chord per command).
- Per-focus different layouts (one layout for both focuses).
- Mouse interaction with the dialog.
- `mapper` changes (none).

## Risks & limitations (accepted)

- **`q` reserved in the dialog** — a command cannot use a bare `q` as its key while expecting it to fire from the dialog. Documented; the validator warns if a dialog command's primary key is `q`.
- **Discoverability of direct keys:** direct commands work silently without the prefix; the hint bar advertises the prefix + the essentials, and every command (direct included) is listed in the dialog.
- **Behavioral change:** map-focus letter commands (retidy `Shift+R`, peel `Shift+P`, etc.) no longer fire directly by default — they move behind the prefix. This is the intended redesign; the `direct` config list lets a user promote any of them back.
