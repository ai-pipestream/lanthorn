# Live Style Reload — Design

**Date:** 2026-06-25
**Status:** Draft, pending user review
**Depends on:** #82 (per-side borders) — touches the same `style.rs`/`colors.rs`/`config.rs`; implement after #82 merges.

## Goal

Apply style changes to a running babelmap without a restart: a manual `/reload`
command and an optional file-watcher that re-reads `style.toml` on save. Make
`style.toml` the single source of styling by removing the style-override sections
from `config.toml`.

## Background (current state)

- Rendering reads `state.colors` (`ColorScheme`) and `state.symbols` (`SymbolSet`)
  fresh every frame, so "apply a style" = rebuild those two values and assign them;
  the next `terminal.draw` reflects the change. No restart is needed for anything
  visual.
- Styling is resolved at startup (`main.rs` ~102 and ~673) as
  `load_style(config.style) → merge(base, config_overrides) → resolve → state.colors
  = …; state.symbols = …`, where `config_overrides` come from `config.toml`'s
  `[colors]`/`[symbols]` sections via `style_from_config`. The config-screen Save
  (`input.rs` ~1989) re-runs the same pipeline.
- `style.toml` is the base; `config.toml` sections currently override it (two-layer
  merge). `Config` carries `colors: StyleColors` and `symbols: StyleSymbols` plus the
  `style` pointer.
- `parse_style_toml` / `resolve` already return non-fatal `warnings`. `load_style`
  falls back to the built-in default theme on a read/parse error.
- Slash commands live in `slash.rs` (curated table + every `Command` by kebab name);
  app actions route through `apply_action`. The run loop already polls a background
  job each iteration (the tidy worker) via `is_finished()`.

## Design

### 1. `style.toml` as the single source (hard removal)

- `Config` drops the `colors: StyleColors` and `symbols: StyleSymbols` fields; the
  `style` pointer stays. `style_from_config` and the `merge(base, over)` step are
  removed from both resolve sites — styling is `load_style(config.style) →
  resolve(base)`.
- **One-time notice:** at startup, if the raw `config.toml` text contains a top-level
  `[colors]` or `[symbols]` table, push one **Warning** transcript line:
  `config.toml [colors]/[symbols] are no longer used — move them into style.toml`.
  (Detected by scanning the parsed config document for those tables; the fields are
  otherwise ignored by deserialization.)

### 2. `reload_style` — the shared pipeline

A single function rebuilds the live style from disk:

```
reload_style(state, user_dir) -> ReloadOutcome
  pointer = state.config.style            // path | "default" | None
  read + parse_style_toml the resolved style.toml from disk
    Ok(doc)  → let (cs, set, warnings) = resolve(doc, user_dir)
               state.colors = cs; state.symbols = set
               → Reloaded { warnings }
    Err(msg) → → Failed { msg }           // current colors/symbols untouched
```

- It resolves the `style` pointer the way `load_style` does — a file path, the
  built-in `"default"`, or `None` → `user_dir/style.toml` (or the built-in default
  when that file is absent). The built-in/`"default"`/missing-file cases parse the
  embedded `DEFAULT_STYLE_TOML` (always valid) and succeed. Only a **real file that
  fails to read or parse** yields `Failed` — and it uses `parse_style_toml`
  **directly** (not `load_style`'s default-fallback), so a syntax error in the user's
  `style.toml` leaves the current look in place rather than wiping it to the default
  theme.
- The caller surfaces the result: on `Reloaded`, set a `style reloaded` status and
  push each resolve `warning` as a Warning line; on `Failed`, push the error as a
  Warning line and set a `reload failed — keeping current style` status.
- Scope: re-resolves everything `style.toml` owns — colors, symbols, borders,
  transcript rules, statusbar. Keymap (config) and `virtual_screen_cols/rows` /
  `user_dir` are **not** touched (the latter remain restart-only).

### 3. Manual reload (`/reload`)

- New `Command::ReloadStyle`, exposed as the `/reload` slash command (and bindable
  via the keymap like any command). `apply_action` handles it by calling
  `reload_style` and applying the outcome (status + warnings) per §2.

### 4. File-watch (optional)

- New dependency: the `notify` crate (cross-platform filesystem watcher).
- Config `watch_style: bool`, **default `false`**.
- When active, a watcher monitors the resolved `style.toml` path and feeds change
  events into an `mpsc` channel. The run loop, each iteration, drains the channel;
  on an event it records a "dirty since" instant. When `dirty` and the debounce
  window (**200 ms**) has elapsed, it calls `reload_style` and clears `dirty`
  (coalescing the burst of events a single save emits).
- The watcher handle + receiver + debounce instant are run-loop locals in `main.rs`
  (filesystem I/O, like the terminal), not `AppState`.
- **Runtime toggle:** `Command::ToggleWatch` / `/watch on|off` starts or stops the
  watcher in place (no restart). Toggling on watches the current `style.toml`; off
  drops the watcher handle.
- If `style = "default"` (built-in, no file) or the pointed path does not exist,
  there is nothing to watch — enabling watch sets a status noting no file is being
  watched.

## Architecture / components

- `crates/app/src/config.rs`: remove `colors`/`symbols` from `Config` (and their
  defaults/serde); add `watch_style: bool` (default `false`). A small
  `config_has_style_sections(raw: &str) -> bool` (or reuse the parsed document) for
  the §1 notice.
- `crates/app/src/style.rs`: remove `style_from_config`. (`merge`, `parse_style_toml`,
  `resolve`, `load_style` stay — `merge` is still used for the no-op base path / tests.)
- `crates/app/src/reload.rs` (new): `pub enum ReloadOutcome { Reloaded { warnings },
  Failed { msg } }` and `pub fn reload_style(state: &mut AppState, user_dir: &Path) ->
  ReloadOutcome` — the pure-ish core (filesystem read + parse + resolve + swap),
  unit-testable by pointing `state.config.style` at a temp file.
- `crates/app/src/keymap.rs` / command enum: add `ReloadStyle` and `ToggleWatch`
  commands (kebab `reload` / `toggle-watch`).
- `crates/app/src/slash.rs`: map `/reload` → `ReloadStyle`; `/watch [on|off]` →
  `ToggleWatch` (argument selects state; bare `/watch` toggles).
- `crates/app/src/apply_action.rs` (or `main.rs` action handling): handle
  `ReloadStyle` (call `reload_style`, apply outcome) and `ToggleWatch` (start/stop the
  watcher via a run-loop hook).
- `crates/app/src/main.rs`: startup §1 notice; remove the config-override merge at both
  resolve sites; if `config.watch_style`, start the watcher; run-loop drain + debounce
  + `reload_style`; wire `ToggleWatch` to start/stop.
- `crates/app/Cargo.toml`: add `notify`.

## Error handling

- `style.toml` parse error on reload → keep current look, one Warning line, status
  `reload failed`.
- Resolve warnings (unknown selector, invalid regex, unknown align) → applied + each
  shown as a Warning line (consistent with startup behavior, now visible).
- Watch enabled but no watchable file → status notice, no watcher.
- `notify` watcher error (e.g. path removed) → drop the watcher, Warning line; manual
  `/reload` still works.
- Removed `config.toml` style sections present → one-time §1 Warning, then ignored.

## Testing

- `reload_style` success: write a temp `style.toml`, point `state.config.style` at it,
  set a sentinel `state.colors`, call `reload_style` → colors/symbols match the file's
  resolution; outcome `Reloaded` with the expected warnings.
- `reload_style` parse error: temp file with broken TOML → outcome `Failed`,
  `state.colors`/`state.symbols` unchanged (current look preserved).
- Resolve warnings surface: a file with an unknown selector → `Reloaded { warnings }`
  non-empty.
- Config: `Config` no longer has `colors`/`symbols`; `watch_style` defaults `false`;
  `config_has_style_sections` detects `[colors]`/`[symbols]` in raw config text.
- Slash: `/reload` → `Command::ReloadStyle`; `/watch on` / `/watch off` / `/watch` →
  `ToggleWatch` with the right intent.
- Debounce (pure): a helper `due(dirty_since: Option<Instant>, now, window) -> bool`
  returns false within the window, true after — unit-tested with fixed instants.
- Removal regression: startup resolve no longer merges config overrides — a
  `config.toml` with a `[colors]` section does not change the resolved `ColorScheme`
  (only `style.toml` does).

## Out of scope (deferred)

- Hot-reloading the **keymap** or non-style config (screen size, user_dir) — those
  stay restart-only.
- Watching multiple files / `@import`-style includes in `style.toml`.
- A migration that folds existing `config.toml` style sections into `style.toml`
  (hard removal chosen; only a one-time notice).
- Per-change animation/transition of the style swap (it applies on the next frame).
