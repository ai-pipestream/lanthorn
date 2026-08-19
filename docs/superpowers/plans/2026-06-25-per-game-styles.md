# Per-Game Style Overrides — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each game layer a per-game style (`user_dir/styles/<ifid>.toml`) over the global `style.toml`, merged through the existing `reload_style`, with a `/game-style` scaffold command.

**Architecture:** A new `styles` module owns the per-game path + scaffold. `reload_style` gains a per-game merge keyed by a new `AppState.ifid` (set at session creation), reusing the existing `merge(global, per_game)`. `/game-style` creates the file with a title header. The style watcher also watches the `styles/` dir.

**Tech Stack:** Rust, `toml` / `merge` (existing StyleDoc machinery), `notify` (existing watcher).

## Global Constraints

- Commit trailers on every commit (body, no backticks anywhere in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Zero compiler warnings; remove any symbol your change orphans.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- Per-game lives at `user_dir/styles/<ifid>.toml`, merged OVER the global via the
  existing `merge(global, per_game)` — overrides everything `style.toml` owns
  (`[colors]` per-key, `[symbols]`, `[[transcript.rule]]`, `[statusbar]`; the last
  two replace-if-present). No carve-outs.
- A per-game file read/parse error keeps the current look (one Warning line),
  matching the global error model.
- `/game-style` scaffolds the file with a title/IFID header if absent; if present,
  it reports the path and does NOT overwrite. It does not itself re-apply styling.
- Run `cargo test -p app` after every task: 0 failures, 0 warnings.

---

### Task 1: styles module — per-game path + scaffold

**Files:**
- Create: `crates/app/src/styles.rs`
- Modify: `crates/app/src/lib.rs` (`pub mod styles;`)

**Interfaces:**
- Produces: `pub fn per_game_style_path(user_dir: &Path, ifid: &str) -> PathBuf` = `user_dir/styles/<ifid>.toml`; `pub fn scaffold_per_game_style(user_dir: &Path, ifid: &str, title: &str) -> std::io::Result<(PathBuf, bool)>` (returns path + whether newly created; never overwrites).

- [ ] **Step 1: Write the failing test**

Create `crates/app/src/styles.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lanthorn-pgstyle-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn path_and_scaffold_behaviour() {
        let dir = tmp("scaffold");
        let ifid = "ZCODE-1-ABCDEF-0001";
        let p = per_game_style_path(&dir, ifid);
        assert_eq!(p, dir.join("styles").join(format!("{ifid}.toml")));

        // First scaffold creates the file (+ styles/ dir) with a header naming the title.
        let (path, created) = scaffold_per_game_style(&dir, ifid, "Zork I").unwrap();
        assert!(created);
        assert_eq!(path, p);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Zork I"), "header names the title");
        assert!(text.contains(ifid), "header names the IFID");
        assert!(text.contains("[colors]"), "seeds an editable [colors] section");

        // Second scaffold does NOT overwrite and reports created=false.
        std::fs::write(&path, "[colors]\n\"room\" = { fg = \"red\" }\n").unwrap();
        let (path2, created2) = scaffold_per_game_style(&dir, ifid, "Zork I").unwrap();
        assert!(!created2);
        assert_eq!(path2, p);
        assert!(std::fs::read_to_string(&path2).unwrap().contains("\"room\""), "existing content preserved");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app path_and_scaffold_behaviour`
Expected: compile error (module/fns missing).

- [ ] **Step 3: Implement the module**

At the top of `crates/app/src/styles.rs` (above the tests), add:

```rust
//! Per-game style overrides: a style.toml keyed by IFID, layered over the global
//! style.toml. See docs/superpowers/specs/2026-06-25-per-game-styles-design.md.

use std::path::{Path, PathBuf};

/// The per-game style file path: `user_dir/styles/<ifid>.toml`.
pub fn per_game_style_path(user_dir: &Path, ifid: &str) -> PathBuf {
    user_dir.join("styles").join(format!("{ifid}.toml"))
}

/// Create the per-game style file (and the `styles/` dir) if it does not exist,
/// seeded with a title/IFID header. Returns `(path, created)`; never overwrites an
/// existing file (`created = false`).
pub fn scaffold_per_game_style(user_dir: &Path, ifid: &str, title: &str) -> std::io::Result<(PathBuf, bool)> {
    let path = per_game_style_path(user_dir, ifid);
    if path.exists() {
        return Ok((path, false));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = format!(
        "# Per-game style override for: {title}\n\
         # IFID: {ifid}\n\
         # Layers on the global style.toml. See style.example.toml for the full schema.\n\
         # Anything style.toml supports works here (colors, symbols, transcript rules,\n\
         # statusbar) and overrides the global value for this game only.\n\
         \n\
         [colors]\n\
         # \"room:current\" = {{ fg = \"yellow\" }}\n"
    );
    std::fs::write(&path, body)?;
    Ok((path, true))
}
```

In `crates/app/src/lib.rs`, add `pub mod styles;` next to the other `pub mod` lines.

- [ ] **Step 4: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/styles.rs crates/app/src/lib.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): per-game style path + scaffold (styles module)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: reload_style merges the per-game layer (AppState.ifid)

**Files:**
- Modify: `crates/app/src/state.rs` (`AppState.ifid` field + default)
- Modify: `crates/app/src/reload.rs` (per-game merge in `reload_style`; tests)

**Interfaces:**
- Consumes: `styles::per_game_style_path` (Task 1), `style::{parse_style_toml, merge}`.
- Produces: `AppState.ifid: String` (default `""`); `reload_style` merges `user_dir/styles/<ifid>.toml` over the global when `ifid` is non-empty and the file exists.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/reload.rs`, inside `mod tests`, add:

```rust
#[test]
fn reload_merges_per_game_over_global() {
    use ratatui::style::Color;
    let dir = temp_dir("pergame");
    // global: transcript white; per-game overrides transcript to green.
    let global = dir.join("style.toml");
    std::fs::write(&global, "[colors]\n\"transcript\" = { fg = \"white\" }\n").unwrap();
    let ifid = "ZCODE-1-PG-0001";
    let pg_dir = dir.join("styles");
    std::fs::create_dir_all(&pg_dir).unwrap();
    std::fs::write(pg_dir.join(format!("{ifid}.toml")), "[colors]\n\"transcript\" = { fg = \"green\" }\n").unwrap();

    let mut state = AppState::default();
    state.config.user_dir = dir.clone();
    state.config.style = Some(global.to_string_lossy().to_string());
    state.ifid = ifid.to_string();

    let outcome = reload_style(&mut state);
    assert!(matches!(outcome, ReloadOutcome::Reloaded { .. }));
    assert_eq!(state.colors.transcript.fg, Some(Color::Green), "per-game overrides global");

    // With no per-game file, the global value stands.
    state.ifid = "ZCODE-1-NONE-9999".to_string();
    reload_style(&mut state);
    assert_eq!(state.colors.transcript.fg, Some(Color::White), "global-only when no per-game file");
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app reload_merges_per_game_over_global`
Expected: compile error (`state.ifid` missing) / assertion failure.

- [ ] **Step 3: Add the `ifid` field**

In `crates/app/src/state.rs`, in the `AppState` struct (near `pub title: String,`), add:

```rust
    /// The current story's IFID (set at session creation). Keys the per-game
    /// style override (`user_dir/styles/<ifid>.toml`). Empty until set.
    pub ifid: String,
```

In `impl Default for AppState`, add (near `title: String::new(),`):

```rust
            ifid: String::new(),
```

- [ ] **Step 4: Merge the per-game layer in `reload_style`**

In `crates/app/src/reload.rs`, in `reload_style`, after the global `doc` is built and before `resolve`, replace the `let (cs, set, warnings) = …` line's lead-in so the per-game file is merged. Insert just before the `resolve` call:

```rust
    // Layer the per-game override (user_dir/styles/<ifid>.toml) over the global.
    let doc = if !state.ifid.is_empty() {
        let pg_path = crate::styles::per_game_style_path(&user_dir, &state.ifid);
        if pg_path.is_file() {
            match std::fs::read_to_string(&pg_path) {
                Ok(text) => match crate::style::parse_style_toml(&text) {
                    Ok(over) => crate::style::merge(&doc, &over),
                    Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", pg_path.display(), e) },
                },
                Err(e) => return ReloadOutcome::Failed { msg: format!("{}: {}", pg_path.display(), e) },
            }
        } else {
            doc
        }
    } else {
        doc
    };
```

(`doc` is rebound; ensure this sits after the `let doc = match resolved_style_path(...)` block and before `let (cs, set, warnings) = crate::style::resolve(&doc, &user_dir);`. `doc` must be `let mut`? No — rebinding with `let doc = …` shadows; keep the original `let doc` and add this `let doc = …`.)

- [ ] **Step 5: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings. (Existing `reload_applies_style_file_and_keeps_current_on_error` stays green: `state.ifid` defaults empty, so the per-game branch is skipped.)

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/state.rs crates/app/src/reload.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): reload_style merges per-game style over global (AppState.ifid)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: `/game-style` command

**Files:**
- Modify: `crates/app/src/keymap.rs` (`Command::GameStyle` + registries)
- Modify: `crates/app/src/input.rs` (`Action::GameStyle` + `apply_action` handler; tests)
- Modify: `crates/app/src/slash.rs` (`/game-style` alias)

**Interfaces:**
- Consumes: `styles::scaffold_per_game_style` (Task 1), `AppState.{ifid, title, config}`.
- Produces: `Command::GameStyle` / `Action::GameStyle`; `apply_action` scaffolds + sets status.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/input.rs`, inside `mod tests`, add:

```rust
#[test]
fn game_style_action_scaffolds_file() {
    let dir = std::env::temp_dir().join(format!("lanthorn-gamestyle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = AppState::default();
    state.config.user_dir = dir.clone();
    state.ifid = "ZCODE-1-GS-0001".to_string();
    state.title = "Zork I".to_string();
    let mut mapper = Mapper::default();

    apply_action(Action::GameStyle, &mut state, &mut mapper);
    let path = crate::styles::per_game_style_path(&dir, &state.ifid);
    assert!(path.is_file(), "scaffold created the per-game file");
    assert!(std::fs::read_to_string(&path).unwrap().contains("Zork I"));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app game_style_action_scaffolds_file`
Expected: compile error (`Action::GameStyle` missing).

- [ ] **Step 3: Wire the command**

In `crates/app/src/keymap.rs`, add `GameStyle` to the `Command` enum and mirror `Retidy`'s entries everywhere `Retidy` appears (the enum, `name()`/`from_name()`, `to_action()`, and `label()`/`context()` if those are exhaustive matches — they are, per the live-reload work). `to_action`: `Command::GameStyle => Action::GameStyle`. `label`: `"game style"`. `context`: `Global`. Kebab name: `game-style`.

In `crates/app/src/input.rs`, add `GameStyle` to the `Action` enum.

- [ ] **Step 4: Handle it in `apply_action`**

In `crates/app/src/input.rs`, in `apply_action`, add an arm:

```rust
        Action::GameStyle => {
            if state.ifid.is_empty() {
                state.set_status("no game loaded");
            } else {
                let user_dir = state.config.user_dir.clone();
                let ifid = state.ifid.clone();
                let title = state.title.clone();
                match crate::styles::scaffold_per_game_style(&user_dir, &ifid, &title) {
                    Ok((path, true))  => state.set_status(format!("created {}", path.display())),
                    Ok((path, false)) => state.set_status(format!("per-game style: {}", path.display())),
                    Err(e)            => state.set_status(format!("game-style failed: {}", e)),
                }
            }
        }
```

- [ ] **Step 5: Add the `/game-style` alias**

In `crates/app/src/slash.rs`, before the generic `Command::from_name` fallback, add:

```rust
    if t0 == "game-style" {
        return SlashOutcome::Action(crate::keymap::Command::GameStyle.to_action());
    }
```

(The generic kebab path also resolves `game-style` via `from_name`; the explicit alias is harmless and keeps the command discoverable.)

- [ ] **Step 6: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/keymap.rs crates/app/src/input.rs crates/app/src/slash.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): /game-style command scaffolds the per-game style file

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: Set ifid at startup; watch the styles dir; docs

**Files:**
- Modify: `crates/app/src/main.rs` (set `state.ifid`; watch `styles/` dir)
- Modify: `crates/app/src/watch.rs` (watch an extra dir)
- Modify: `README.md` (per-game mention)

**Interfaces:**
- Consumes: `AppState.ifid`, `styles::per_game_style_path`, the existing watcher.

- [ ] **Step 1: Set `state.ifid` at session creation**

In `crates/app/src/main.rs`, after `state.config = cfg;` (or wherever `state` and the computed `ifid` are both in scope, near the title resolution ~748), add:

```rust
    state.ifid = ifid.clone();
```

(`ifid` is computed at `main.rs:666` via `compute_ifid`. Set it on `state` so `reload_style` merges the per-game layer. Also set it on any session re-creation path — game reset/restore — if those rebuild `state`; if they reuse the same `state`, `ifid` is unchanged and correct.)

- [ ] **Step 2: Watch the styles dir too**

In `crates/app/src/watch.rs`, add a method to watch an additional directory on the same watcher:

```rust
impl StyleWatcher {
    /// Also watch `dir` (non-recursively) on this watcher; ignored on error.
    pub fn also_watch(&mut self, dir: &std::path::Path) {
        let _ = self._watcher.watch(dir, RecursiveMode::NonRecursive);
    }
}
```

(`_watcher` must be reachable — it is a field of `StyleWatcher`; rename its access if needed so `also_watch` can call `.watch`. If the field is named with a leading underscore only to silence unused warnings, it is still usable as `self._watcher`.)

In `crates/app/src/main.rs`, where the watcher is started (~610-614, and in `toggle_style_watch`), after a successful `start`, also watch the styles dir so a per-game file created mid-session is seen:

```rust
        if let Some(w) = watcher.as_mut() {
            w.also_watch(&state.config.user_dir.join("styles"));
        }
```

(Create the `styles/` dir watch even if it does not exist yet is not possible — `watch` errors on a missing dir, which `also_watch` ignores. The `/game-style` scaffold creates `styles/`; to catch the very first creation, the watcher already watches `user_dir` (style.toml's parent), so the `styles/` dir creation event surfaces, and a subsequent toggle/`/reload` picks up the file. Document this minor latency in the README note.)

- [ ] **Step 3: Build + run the suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean, 0 warnings; suite PASS. (No new unit test for the watcher wiring — the pure pieces are tested; this is filesystem I/O.)

- [ ] **Step 4: README mention**

In `README.md`, in the Customization section (near the style-files bullet), add a sentence:

```
  Per-game looks: run `/game-style` to scaffold `~/.lanthorn/styles/<ifid>.toml`,
  edit it, and `/reload` — it layers over `style.toml` for that game only
  (including its own statusbar / transcript rules).
```

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/main.rs crates/app/src/watch.rs README.md
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): set ifid at startup, watch styles dir, document per-game styles

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Dependency order: 1 (styles module) → 2 (reload merge + ifid) → 3 (command) → 4 (wiring + docs). Each ends green (`cargo test -p app`, 0 warnings) before committing.
- Task 3 command wiring: grep `Retidy` in `keymap.rs` to find every registry it appears in (enum, `name`/`from_name`, `to_action`, `label`, `context`, and any `ALL_COMMANDS` list) and mirror it for `GameStyle`.
- Task 4 watcher: if `StyleWatcher`'s notify field is private/underscored, expose just enough (the `also_watch` method) — do not restructure the watcher.
- `README.md` is committed; `TODO.md` is gitignored — never stage it.
