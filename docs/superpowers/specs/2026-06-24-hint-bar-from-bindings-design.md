# Hint Bar From Live Bindings — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO item:** #11 — "Map-focus hint bar advertises dead keys (gallery/layout/inspect show but do nothing after the leader-key redesign). Derive the hint bar from the ACTUAL current bindings for the focused context so it can never show a key that isn't directly bound. Also remove the tidy hint."
**Depends on:** the keymap/leader-key system (merged). No `mapper`/`zvm` changes. Touches `crates/app/src/main.rs` (the `hint_line`/`hint_line_game` builders + their call sites) and `crates/app/src/keymap.rs` only if a small lookup helper is needed.

## Goal

The bottom hint bar must never advertise a key that does nothing. Today it renders hardcoded curated lists (`MAP_HINTS`/`GAME_HINTS`/`ANIM_HINTS`) and shows each command's `primary_key` WITHOUT checking whether the command is actually directly available in the focused context — so commands that became `Ctrl+K`-dialog commands after the leader-key redesign (gallery/layout/inspect/tidy) still show a key that only works inside the dialog. Fix it with a **hybrid** builder: a curated priority list of commands, but inclusion + key + label all validated against the **live keymap**, with a hard "this key really triggers this command here" check.

## Design (hybrid, validated against live bindings)

### Curated lists become priority-ordered command lists
`GAME_HINTS`/`MAP_HINTS`/`ANIM_HINTS` change from `&[(Command, &str)]` (command + label override) to `&[Command]` — they define only the ORDER and which commands are "important enough" to appear. The **label** comes from `Command::label()` (the keymap's per-command short label) and the **key** from the live keymap. **Remove `Command::Retidy` (the tidy hint)** from the lists.

### One builder, three gates
```rust
fn hint_bar(keymap: &KeyMap, layout: &HotkeyLayout, ctx: Context, priority: &[Command], width: usize) -> String
```
For each `cmd` in `priority`, include an entry ONLY if all three hold:
1. **Direct:** `layout.is_direct(cmd)` — the command is directly available, not routed through the `Ctrl+K` dialog.
2. **Bound:** `keymap.primary_key(cmd)` returns a `KeySpec` (call it `k`).
3. **Active here (the definitive no-dead-key check):** `keymap.lookup(&k, ctx) == Some(cmd)` — pressing `k` in `ctx` actually resolves back to `cmd` (catches keys that are unbound-in-this-context or shadowed by another binding). If a small context-aware primary-key helper is cleaner, add `keymap.primary_key_in_context(cmd, ctx)` and use it for step 2; step 3 is still the guarantee.

Each surviving entry renders `"{k.label()}: {cmd.label()}"`; entries join with `" | "`.

### Width handling — truncate with `…`
The builder takes the available `width` (the hint row's width). If the joined string exceeds it, truncate to fit and append `…` (char-count aware, like the existing `truncate_line`). Everything dropped is still reachable via the `Ctrl+K` dialog, so `…` is an honest "more in the dialog" signal.

### Contexts + call sites
- `Focus::Game` → `hint_bar(.., Context::Game-or-Global, GAME_HINTS, w)`. (Game-focus hints fall through Game→Global like input routing; use the context the game-focus dispatch actually uses.)
- `Focus::Map` → `hint_bar(.., Context::Map, MAP_HINTS, w)`.
- `tidy_anim` active → `hint_bar(.., Context::Anim, ANIM_HINTS, w)`.
- The existing prompt-mode / gallery static instruction strings (not keymap-derived) are unchanged. Thread the `HotkeyLayout` (already on `AppState` from the `[hotkeys]` config) and the available width into the builder at each call site.

## Testing
- **No-dead-keys invariant (the core test):** for each context's hint list, build the bar, parse out each shown `KEY`, and assert `keymap.lookup(parsed_key, ctx) == Some(expected_cmd)` for every entry — i.e. nothing shown is dead. Equivalently, assert the builder NEVER emits a command for which `is_direct` is false (drive it with a command that's dialog-only and confirm it's absent).
- **Tidy removed:** the map/game bars do not contain "retidy"/"tidy".
- **Rebinding reflected:** rebind a shown command's key; the bar shows the new key (extends the existing `hint_line_reflects_rebinding` test).
- **is_direct gating:** make a curated command non-direct (route it via the dialog in a test `HotkeyLayout`); assert it drops out of the bar while a still-direct command remains.
- **Truncation:** with a narrow width, the bar fits the width and ends with `…`; with ample width, no `…` and all valid entries present (port the existing `hint_line_map_contains_zoom_with_plus_key` style assertions).
- **Labels from keymap:** an entry's label equals `Command::label()` (e.g. ZoomIn → "zoom in").

## Out of scope / non-goals
- The static prompt/gallery instruction lines (they're not key hints).
- Smart/contextual state-aware hints (the "what can I do right now" engine) — possible later; this spec is purely the no-dead-key binding-derived bar.
- Re-ordering or re-labeling commands beyond removing tidy (the curated order stays; labels come from `Command::label()`).

## Risks & limitations (accepted)
- **Directional labels lost:** today `ToggleFocus` shows a context-specific override ("story" in map focus, "map" in game focus); using `Command::label()` it becomes a single neutral label ("focus") in both. Accepted (consistent + simpler); if the directional cue is missed, a tiny per-context label override can be re-added later without changing the gating.
- **`Command::label()` lengths:** some labels are longer than the old terse overrides, so fewer fit before `…`. The priority order ensures the most useful hints survive; the rest live in the `Ctrl+K` dialog.

## Sources
- Current hint bar: `crates/app/src/main.rs` (`hint_line`, `hint_line_game`, `MAP_HINTS`/`GAME_HINTS`/`ANIM_HINTS`, the call sites ~370–400).
- Bindings: `crates/app/src/keymap.rs` (`KeyMap::primary_key`/`lookup`, `Command::label`, `HotkeyLayout::is_direct`).
