# Live Style Reload — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply `style.toml` changes to a running lanthorn without restart — a `/reload` command and an opt-in file-watcher — and make `style.toml` the single styling source by removing the override sections from `config.toml`.

**Architecture:** A `reload_style(&mut AppState)` core re-reads `style.toml` from disk, resolves it, and swaps `state.colors`/`state.symbols`, keeping the current look on a parse error. `/reload` routes through `apply_action`. A `notify`-based watcher (gated by `watch_style`, default off) feeds the run loop, which debounces and calls the same core. `config.toml` no longer carries `[colors]`/`[symbols]`.

**Tech Stack:** Rust, `notify` 6 (filesystem watcher), `toml`.

## Global Constraints

- Commit trailers on every commit (body, no backticks anywhere in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Zero compiler warnings; remove any symbol your change orphans.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- `style.toml` is the **single** styling source — `config.toml` `[colors]`/`[symbols]`
  sections are removed (a one-time Warning notice if an old config still has them).
- Reload re-resolves only what `style.toml` owns (colors, symbols, borders,
  transcript rules, statusbar). Keymap, `virtual_screen_cols/rows`, and `user_dir`
  are NOT reloaded (restart-only).
- A `style.toml` read/parse error on reload keeps the CURRENT look (no fallback to
  the default theme) and shows one Warning transcript line.
- `watch_style` defaults `false`. Manual `/reload` always works regardless.
- Run `cargo test -p app` after every task: 0 failures, 0 warnings.

---

### Task 1: Remove style overrides from config.toml; add watch_style

**Files:**
- Modify: `crates/app/src/config.rs` (`Config` struct ~205; defaults; add `watch_style`; a detector)
- Modify: `crates/app/src/style.rs` (remove `style_from_config`)
- Modify: `crates/app/src/main.rs` (resolve site ~102; the main startup resolve)
- Modify: `crates/app/src/input.rs` (config-screen save ~1989)

**Interfaces:**
- Produces: `Config.watch_style: bool` (default `false`); `pub fn config_has_style_sections(raw: &str) -> bool`. Removes `Config.colors`/`Config.symbols` and `style::style_from_config`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/config.rs`, inside `mod tests` (add one if absent), add:

```rust
#[test]
fn watch_style_defaults_false_and_detector_works() {
    let c = Config::default();
    assert!(!c.watch_style);
    assert!(config_has_style_sections("[colors]\n\"room\" = { fg = \"red\" }\n"));
    assert!(config_has_style_sections("[symbols]\nbox_style = \"thick\"\n"));
    assert!(!config_has_style_sections("user_dir = \"/x\"\nstyle = \"s.toml\"\n"));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app watch_style_defaults_false_and_detector_works`
Expected: compile error (`watch_style` / `config_has_style_sections` missing).

- [ ] **Step 3: Remove the override fields, add `watch_style`**

In `crates/app/src/config.rs`, in the `Config` struct, delete:

```rust
    pub colors: crate::style::StyleColors,
    pub symbols: crate::style::StyleSymbols,
```

and add (near the `style` pointer):

```rust
    /// Watch style.toml and live-reload on change (default false).
    #[serde(default)]
    pub watch_style: bool,
```

Update `Config`'s `Default` impl: remove the `colors`/`symbols` initializers and add `watch_style: false,`. If `clone_config` (input.rs) or any other `Config { … }` literal sets `colors`/`symbols`, remove those lines there too (compiler-driven).

- [ ] **Step 4: Add the detector**

In `crates/app/src/config.rs`, add:

```rust
/// True if a raw config.toml still contains a top-level `[colors]` or `[symbols]`
/// table (those style sections moved to style.toml and are now ignored).
pub fn config_has_style_sections(raw: &str) -> bool {
    match raw.parse::<toml::Value>() {
        Ok(toml::Value::Table(t)) => t.contains_key("colors") || t.contains_key("symbols"),
        _ => false,
    }
}
```

- [ ] **Step 5: Remove `style_from_config` and the config-override merges**

In `crates/app/src/style.rs`, delete the `style_from_config` function.

In `crates/app/src/main.rs` (resolve site ~102), replace:

```rust
    let (base, _w1) = app::style::load_style(state.config.style.as_deref(), user_dir);
    let over = app::style::style_from_config(&state.config.colors, &state.config.symbols);
    let (cs, set, _w2) = app::style::resolve(&app::style::merge(&base, &over), user_dir);
    state.colors = cs;
    state.symbols = set;
```

with:

```rust
    let (base, _w1) = app::style::load_style(state.config.style.as_deref(), user_dir);
    let (cs, set, _w2) = app::style::resolve(&base, user_dir);
    state.colors = cs;
    state.symbols = set;
```

Apply the same removal at the **main startup** resolve site (search `main.rs` for the other `style_from_config` call and remove the `over`/`merge` there too — there are two resolve paths).

In `crates/app/src/input.rs` (~1989), replace:

```rust
                let (base, _w1) =
                    crate::style::load_style(cs.working.style.as_deref(), &cs.working.user_dir);
                let over = crate::style::style_from_config(&cs.working.colors, &cs.working.symbols);
                let (colors, set, _w2) =
                    crate::style::resolve(&crate::style::merge(&base, &over), &cs.working.user_dir);
                state.colors = colors;
                state.symbols = set;
```

with:

```rust
                let (base, _w1) =
                    crate::style::load_style(cs.working.style.as_deref(), &cs.working.user_dir);
                let (colors, set, _w2) = crate::style::resolve(&base, &cs.working.user_dir);
                state.colors = colors;
                state.symbols = set;
```

- [ ] **Step 6: One-time notice at startup**

In `crates/app/src/main.rs`, where the config file is read at startup (it produces the `Config`), after the banner is pushed, add a check that pushes a Warning line when the raw config has style sections. Find where the config file path is known (`config.user_dir`-relative `config.toml`, or the `--config` path) and add:

```rust
    if let Ok(raw_cfg) = std::fs::read_to_string(&config_path) {
        if app::config::config_has_style_sections(&raw_cfg) {
            state.push_transcript_kind(
                "config.toml [colors]/[symbols] are no longer used — move them into style.toml",
                app::state::TranscriptKind::Warning,
            );
        }
    }
```

(Use the actual config path variable in scope; if the path is not retained, read it from `--config` / `user_dir.join("config.toml")` as the loader does. If no config file exists, the read fails and nothing is pushed — correct.)

- [ ] **Step 7: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (Removing `Config.colors`/`symbols` will surface every literal/field use via the compiler — fix each to drop those fields.)

- [ ] **Step 8: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/config.rs crates/app/src/style.rs crates/app/src/main.rs crates/app/src/input.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): style.toml is the single source; drop config style overrides

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: reload_style core

**Files:**
- Create: `crates/app/src/reload.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod reload;`)

**Interfaces:**
- Consumes: `style::{load_style, parse_style_toml, resolve}`, `colors::expand_path`, `AppState.{config, colors, symbols}`.
- Produces: `pub enum ReloadOutcome { Reloaded { warnings: Vec<String> }, Failed { msg: String } }`; `pub fn reload_style(state: &mut AppState) -> ReloadOutcome`.

- [ ] **Step 1: Write the failing test**

Create `crates/app/src/reload.rs` with only this test module to start (implementation added next step):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lanthorn-reload-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reload_applies_style_file_and_keeps_current_on_error() {
        let dir = temp_dir("ok");
        let path = dir.join("style.toml");
        std::fs::write(&path, "[colors]\n\"transcript\" = { fg = \"green\" }\n").unwrap();

        let mut state = AppState::default();
        state.config.user_dir = dir.clone();
        state.config.style = Some(path.to_string_lossy().to_string());

        let outcome = reload_style(&mut state);
        assert!(matches!(outcome, ReloadOutcome::Reloaded { .. }));
        assert_eq!(state.colors.transcript.fg, Some(ratatui::style::Color::Green));

        // Now break the file: reload keeps the current (green) look and reports Failed.
        std::fs::write(&path, "this is not valid = = toml [[[").unwrap();
        let outcome2 = reload_style(&mut state);
        assert!(matches!(outcome2, ReloadOutcome::Failed { .. }));
        assert_eq!(state.colors.transcript.fg, Some(ratatui::style::Color::Green), "current look preserved on error");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app reload_applies_style_file_and_keeps_current_on_error`
Expected: compile error (module/fn missing).

- [ ] **Step 3: Implement `reload_style`**

At the top of `crates/app/src/reload.rs` (above the test module), add:

```rust
//! Live style reload: re-resolve style.toml from disk and swap the live
//! ColorScheme / SymbolSet, keeping the current look on a parse error.

use std::path::Path;

use crate::state::AppState;

/// Result of a reload attempt.
pub enum ReloadOutcome {
    /// Applied; carries any non-fatal resolve warnings.
    Reloaded { warnings: Vec<String> },
    /// Not applied (read/parse error); the current look is untouched.
    Failed { msg: String },
}

/// Resolve the `style` pointer to its on-disk path, if it names a real file.
/// Returns `None` for the built-in `"default"` or when `None` resolves to a
/// missing `user_dir/style.toml` (those parse the embedded default — no file).
pub fn resolved_style_path(style: Option<&str>, user_dir: &Path) -> Option<std::path::PathBuf> {
    match style {
        Some("default") => None,
        Some(p) => Some(crate::colors::expand_path(p, user_dir)),
        None => {
            let cand = user_dir.join("style.toml");
            if cand.is_file() { Some(cand) } else { None }
        }
    }
}

/// Re-read and apply `style.toml`. On a real-file read/parse error, the current
/// `state.colors`/`state.symbols` are left in place.
pub fn reload_style(state: &mut AppState) -> ReloadOutcome {
    let user_dir = state.config.user_dir.clone();
    let pointer = state.config.style.clone();

    // Build the StyleDoc: a real file parses directly (error → Failed); the
    // default/missing cases use the embedded default via load_style.
    let doc = match resolved_style_path(pointer.as_deref(), &user_dir) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => match crate::style::parse_style_toml(&text) {
                Ok(doc) => doc,
                Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", path.display(), e) },
            },
            Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", path.display(), e) },
        },
        None => {
            let (doc, _w) = crate::style::load_style(pointer.as_deref(), &user_dir);
            doc
        }
    };

    let (cs, set, warnings) = crate::style::resolve(&doc, &user_dir);
    state.colors = cs;
    state.symbols = set;
    ReloadOutcome::Reloaded { warnings }
}
```

In `crates/app/src/lib.rs`, add `pub mod reload;` next to the other `pub mod` lines.

- [ ] **Step 4: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/reload.rs crates/app/src/lib.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): reload_style core (re-resolve style.toml, keep look on error)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: `/reload` command

**Files:**
- Modify: `crates/app/src/keymap.rs` (`Command::ReloadStyle` + its name/from_name/to_action entries)
- Modify: `crates/app/src/input.rs` (`Action::ReloadStyle` + `apply_action` arm; tests)

**Interfaces:**
- Consumes: `reload::{reload_style, ReloadOutcome}`.
- Produces: `Command::ReloadStyle`, `Action::ReloadStyle`; `apply_action` applies the reload + surfaces warnings/status.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/input.rs`, inside `mod tests`, add:

```rust
#[test]
fn reload_action_applies_style_file() {
    let dir = std::env::temp_dir().join(format!("lanthorn-reloadact-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("style.toml");
    std::fs::write(&path, "[colors]\n\"transcript\" = { fg = \"magenta\" }\n").unwrap();

    let mut state = AppState::default();
    state.config.user_dir = dir.clone();
    state.config.style = Some(path.to_string_lossy().to_string());
    let mut mapper = Mapper::default();

    apply_action(Action::ReloadStyle, &mut state, &mut mapper);
    assert_eq!(state.colors.transcript.fg, Some(ratatui::style::Color::Magenta));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app reload_action_applies_style_file`
Expected: compile error (`Action::ReloadStyle` missing).

- [ ] **Step 3: Wire the command**

In `crates/app/src/keymap.rs`, add `ReloadStyle` to the `Command` enum, and mirror `Retidy`'s entries wherever they appear (the enum, the `name()`/`from_name()` registry, and `to_action()`). For `to_action`, add:

```rust
            Command::ReloadStyle => Action::ReloadStyle,
```

The kebab name is `reload-style`, so the `name()`/`from_name()` mapping uses snake `reload_style`. (The `/reload` alias is added in Task 3 Step 5.)

In `crates/app/src/input.rs`, add `ReloadStyle` to the `Action` enum.

- [ ] **Step 4: Handle it in `apply_action`**

In `crates/app/src/input.rs`, in `apply_action`, add an arm:

```rust
        Action::ReloadStyle => {
            match crate::reload::reload_style(state) {
                crate::reload::ReloadOutcome::Reloaded { warnings } => {
                    for w in &warnings {
                        state.push_transcript_kind(w, crate::state::TranscriptKind::Warning);
                    }
                    state.set_status("style reloaded");
                }
                crate::reload::ReloadOutcome::Failed { msg } => {
                    state.push_transcript_kind(
                        &format!("style reload failed: {}", msg),
                        crate::state::TranscriptKind::Warning,
                    );
                    state.set_status("reload failed — keeping current style");
                }
            }
        }
```

- [ ] **Step 5: Add the `/reload` alias**

In `crates/app/src/slash.rs`, in `parse`, before the generic `Command::from_name` fallback, add an explicit alias so `/reload` maps to the command:

```rust
    if t0 == "reload" {
        return SlashOutcome::Action(crate::keymap::Command::ReloadStyle.to_action());
    }
```

- [ ] **Step 6: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/keymap.rs crates/app/src/input.rs crates/app/src/slash.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): /reload command (live re-resolve style.toml)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: File-watch + `/watch` toggle

**Files:**
- Modify: `crates/app/Cargo.toml` (add `notify`)
- Create: `crates/app/src/watch.rs` (watcher handle + debounce helper)
- Modify: `crates/app/src/keymap.rs` / `input.rs` (`ToggleWatch` command/action)
- Modify: `crates/app/src/slash.rs` (`/watch [on|off]`)
- Modify: `crates/app/src/main.rs` (run-loop integration)

**Interfaces:**
- Produces: `pub struct StyleWatcher` (owns a `notify` watcher + an `mpsc::Receiver`); `pub fn start(path) -> Option<StyleWatcher>`; `pub fn due(dirty_since: Option<Instant>, now: Instant, window: Duration) -> bool`. `Command::ToggleWatch` / `Action::ToggleWatch` (run-loop-handled).

- [ ] **Step 1: Add the dependency**

In `crates/app/Cargo.toml` under `[dependencies]`, add:

```toml
notify = "6"
```

- [ ] **Step 2: Write the failing test (pure debounce)**

Create `crates/app/src/watch.rs` with this test (impl added next):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn due_only_after_window() {
        let now = Instant::now();
        let win = Duration::from_millis(200);
        assert!(!due(None, now, win), "never due when not dirty");
        assert!(!due(Some(now), now, win), "not due immediately");
        assert!(!due(Some(now), now + Duration::from_millis(100), win), "not due within window");
        assert!(due(Some(now), now + Duration::from_millis(250), win), "due after window");
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p app due_only_after_window`
Expected: compile error (module/fn missing).

- [ ] **Step 4: Implement the watcher + debounce**

At the top of `crates/app/src/watch.rs`, add:

```rust
//! Optional filesystem watcher for live style reload. Watches the directory
//! containing style.toml and signals the run loop, which debounces and reloads.

use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

/// A live style watcher: keeps the `notify` watcher alive and exposes its events.
pub struct StyleWatcher {
    _watcher: RecommendedWatcher,
    pub rx: Receiver<notify::Result<notify::Event>>,
}

/// Start watching the directory that contains `file` (non-recursive), so the file
/// being created/edited/replaced all surface. Returns `None` if the path has no
/// parent or the watcher cannot be created.
pub fn start(file: &Path) -> Option<StyleWatcher> {
    let dir = file.parent()?;
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res| { let _ = tx.send(res); }).ok()?;
    watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;
    Some(StyleWatcher { _watcher: watcher, rx })
}

/// True when a pending change has settled: dirty and at least `window` elapsed.
pub fn due(dirty_since: Option<Instant>, now: Instant, window: Duration) -> bool {
    match dirty_since {
        Some(t) => now.duration_since(t) >= window,
        None => false,
    }
}
```

In `crates/app/src/lib.rs`, add `pub mod watch;`.

- [ ] **Step 5: Wire `ToggleWatch` command/action**

In `keymap.rs`, add `Command::ToggleWatch` (mirror `Retidy`'s registry entries; kebab `toggle-watch`); `to_action` → `Action::ToggleWatch`. In `input.rs`, add `Action::ToggleWatch` to the `Action` enum. Do NOT handle it in `apply_action` (it needs the run-loop watcher) — instead add a no-op arm there with a comment, or let the run loop intercept it (Step 7). To keep `apply_action` exhaustive without behavior, add:

```rust
        Action::ToggleWatch => { /* handled in the run loop (owns the watcher) */ }
```

In `slash.rs`, add a `/watch [on|off]` alias before the generic fallback:

```rust
    if t0 == "watch" {
        return SlashOutcome::Action(crate::keymap::Command::ToggleWatch.to_action());
    }
```

(Bare `/watch` toggles; `on`/`off` arguments are honored by the run-loop handler in Step 7 by reading `state` — for v1, `/watch` toggles the current state; treat any argument as a toggle. Keep it simple.)

- [ ] **Step 6: Run the suite (everything but the run-loop wiring)**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Run-loop integration (main.rs)**

In `crates/app/src/main.rs`'s run loop, add watcher state and polling. This is filesystem I/O wiring, verified by build + manual run (no unit test — the pure `due`/`reload_style` pieces are already tested).

a) Before the loop, initialize the watcher when `config.watch_style` is set:

```rust
    use std::time::{Duration, Instant};
    let mut style_watcher: Option<app::watch::StyleWatcher> = None;
    let mut watch_dirty: Option<Instant> = None;
    if state.config.watch_style {
        if let Some(p) = app::reload::resolved_style_path(state.config.style.as_deref(), &state.config.user_dir) {
            style_watcher = app::watch::start(&p);
        }
    }
```

b) Inside the loop, after the event/draw handling each iteration, drain events and debounce-reload:

```rust
        if let Some(w) = &style_watcher {
            let mut saw = false;
            while w.rx.try_recv().is_ok() { saw = true; }
            if saw { watch_dirty = Some(Instant::now()); }
        }
        if app::watch::due(watch_dirty, Instant::now(), Duration::from_millis(200)) {
            watch_dirty = None;
            match app::reload::reload_style(&mut state) {
                app::reload::ReloadOutcome::Reloaded { warnings } => {
                    for wn in &warnings { state.push_transcript_kind(wn, app::state::TranscriptKind::Warning); }
                    state.set_status("style reloaded (watch)");
                }
                app::reload::ReloadOutcome::Failed { msg } => {
                    state.push_transcript_kind(&format!("style reload failed: {}", msg), app::state::TranscriptKind::Warning);
                }
            }
        }
```

c) Handle `Action::ToggleWatch` where the run loop dispatches actions (search for where `apply_action` is called / where `GalleryApply` is intercepted), starting/stopping the watcher:

```rust
            if matches!(action, Action::ToggleWatch) {
                if style_watcher.is_some() {
                    style_watcher = None;
                    state.set_status("style watch off");
                } else if let Some(p) = app::reload::resolved_style_path(state.config.style.as_deref(), &state.config.user_dir) {
                    style_watcher = app::watch::start(&p);
                    state.set_status(if style_watcher.is_some() { "style watch on" } else { "style watch: no file to watch" });
                } else {
                    state.set_status("style watch: no file to watch");
                }
                continue;
            }
```

Place this intercept alongside the existing per-action handling in the run loop (mirror how a run-loop-only action is handled), before/instead of forwarding `ToggleWatch` to `apply_action`.

- [ ] **Step 8: Build + run the suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean, 0 warnings; suite PASS.

- [ ] **Step 9: Manual verification (record in report)**

Run `cargo run -p app -- crates/zvm/tests/fixtures/minizork.z3` with `watch_style = true` in the config (or run `/watch` then edit). Edit `style.toml` (e.g. change `transcript` fg), save; confirm the transcript recolors within ~200 ms without restart, and that `/reload` does the same on demand. A broken edit shows a Warning line and keeps the current look. If no graphical terminal is available, state manual verification is deferred and the code matches the documented `notify` integration.

- [ ] **Step 10: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/Cargo.toml crates/app/src/watch.rs crates/app/src/lib.rs crates/app/src/keymap.rs crates/app/src/input.rs crates/app/src/slash.rs crates/app/src/main.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): opt-in style.toml file-watch + /watch toggle

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: Document in style.example.toml + README

**Files:**
- Modify: `style.example.toml` (note the reload + watch_style)
- Modify: `README.md` (one-line mention under Customization)

- [ ] **Step 1: Add a comment to style.example.toml**

At the top of `style.example.toml`, after the existing header comment, add:

```toml
# Edit this file and run /reload in-app to apply changes live, or set
# watch_style = true in config.toml to auto-reload on save. A syntax error
# keeps your current look and shows a warning.
```

- [ ] **Step 2: README pointer**

In `README.md`, in the Customization section's "Shareable style files" bullet, append:

```
  Changes apply live: `/reload` re-reads `style.toml`, and `watch_style = true`
  in `config.toml` auto-reloads on save (`/watch` toggles it at runtime).
```

- [ ] **Step 3: Verify**

Run: `cargo test -p app style_example_toml_parses_and_resolves_clean`
Expected: PASS (comments don't change resolution).

- [ ] **Step 4: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add style.example.toml README.md
git -C /Volumes/Videos/Source/lanthorn commit -m "docs: document /reload + watch_style live reload

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Dependency order: 1 (config removal) → 2 (reload core) → 3 (/reload) → 4 (watch) → 5 (docs). Each ends green (`cargo test -p app`, 0 warnings) before committing.
- Task 1 is compiler-driven: removing `Config.colors`/`symbols` surfaces every use (literals in `clone_config`, the two resolve sites, the config-screen save). Fix each by dropping those fields/lines. `merge` stays in `style.rs` (still used elsewhere/tests) — only `style_from_config` is removed.
- Task 4 Step 7 is run-loop I/O wiring; the testable parts (`due`, `reload_style`, command parsing) are unit-tested in earlier steps. If the run-loop action dispatch differs in shape, mirror how an existing run-loop-only action (e.g. a Resize/GalleryApply intercept) is handled — intercept `ToggleWatch` before `apply_action`.
- `README.md` is committed; `TODO.md` is gitignored — never stage it.
