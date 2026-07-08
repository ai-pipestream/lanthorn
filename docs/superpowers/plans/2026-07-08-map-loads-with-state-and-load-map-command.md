# Map Loads With State + `/load-map` — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A map loads only as part of the `.babelmap` archive's state — never standalone, never decoupled from that state. Standalone map files enter only via a new `/load-map <path>` command.

**Architecture:** Gate the startup archive-map adoption on `cfg.auto_load` (same gate as the game-state restore); make the launch-resume dialog adopt the archive map on accept; delete all standalone-map auto-load (and the `use_default_map` option); add a `/load-map` slash command mirroring the existing arg-bearing commands.

**Tech Stack:** Rust workspace; `app` crate (ratatui TUI). Quest SQ-0226.

## Global Constraints

- A map NEVER auto-loads from a standalone file. Only the `.babelmap` archive (gated by `auto_load`) or explicit `/load-map` bring a map in.
- Explicit save/restore/`/load` paths (Ctrl+R `main.rs:3314`, saves-list `3542`/`3576`, `/load` `3967`) already adopt the embedded map and must stay unchanged.
- Command naming follows the registry's verb-noun kebab convention (`export-transcript`, `zoom-map`), so the command is `load-map`.
- Run `cargo test -p app` per task; it must stay green (~1055+ lib tests).

---

### Task 1: Couple startup map to state; remove standalone-map infrastructure

**Files:**
- Modify: `crates/app/src/main.rs` (startup block `1736-1786`; exit fallback `3820`; `apply_launch_resume` `4959`; the two launch-resume accept sites `2531-2533`, `2554-2556`; `map_dir` `116`; imports `25`,`27`; `map_dir` test `6002`)
- Modify: `crates/app/src/ifid.rs` (`map_path` `3` + test `18`)
- Modify: `crates/app/src/persist_files.rs` (`save_map` `189` + its tests)

**Interfaces:**
- Consumes: `load_archive`, `Mapper::default()`, `cfg.auto_load`, `cfg.prompt_load_on_launch`.
- Keeps: `load_map` (`persist_files.rs:196`) — used by Task 3. Only `save_map`/`map_path`/`map_dir` are removed.

- [ ] **Step 1: Gate the startup archive-map load on `auto_load` and delete the standalone branches**

Replace the whole `let mut mapper = if arc_file.exists() { … } else { … };` block (`main.rs:1736-1786`) with:

```rust
    let mut mapper = if arc_file.exists() {
        match load_archive(&arc_file) {
            Ok(ac) => {
                // Restore the machine from the saved game state only when auto_load is enabled.
                if cfg.auto_load {
                    match session.restore_state(&ac.engine_save()) {
                        Ok(()) => {
                            if let Some(scr) = ac.screen.clone() {
                                if let Some(zs) = zvm_session_opt_mut(&mut *session) {
                                    zs.machine.screen = scr;
                                }
                            }
                            startup_transcript = Some((ac.transcript, ac.transcript_kinds, ac.transcript_runs));
                            startup_history = ac.history;
                        }
                        Err(e) => {
                            eprintln!("babelmap: warning: could not restore game from archive: {}; starting fresh", restore_error_msg(e));
                        }
                    }
                } else if cfg.prompt_load_on_launch && !ac.save.is_empty() {
                    pending_resume_stash = Some((ac.engine_save(), ac.transcript, ac.transcript_kinds, ac.screen));
                }
                if cfg.aux_storage != app::config::AuxStorage::Global {
                    session.set_aux_data(ac.aux.clone());
                }
                startup_command_history = ac.command_history;
                // The map is part of the game's state: it loads only when the state is
                // auto-resumed here. When auto_load is off it either rides the launch-resume
                // dialog (adopted on accept, see apply_launch_resume) or stays blank.
                if cfg.auto_load { ac.mapper } else { Mapper::default() }
            }
            Err(e) => {
                eprintln!("babelmap: warning: could not load archive {}: {}", arc_file.display(), e);
                Mapper::default()
            }
        }
    } else {
        Mapper::default()
    };
```

Also delete the now-stale doc comment lines `main.rs:1726-1727` ("Migration: … / use_default_map = true: …"). Remove `let dir = map_dir(&cfg.user_dir);` (`1720`) and `let map_file = map_path(&dir, &ifid);` (`1723`) — they are now unused.

- [ ] **Step 2: Make the launch-resume dialog adopt the archive map on accept**

`apply_launch_resume` (`main.rs:4959`) gains an `arc_file: &std::path::Path` parameter and, after restoring game state, adopts the archive's map so a dialog-resume brings the map with it:

```rust
    // The resumed game's map is part of its archive state — load it alongside.
    if let Ok(ac) = load_archive(arc_file) {
        *mapper = ac.mapper;
    }
```

Update both accept call sites (`main.rs:2533` and `2556`) to pass `&arc_file`:
`apply_launch_resume(&save, lines, kinds, screen, &mut *session, &mut mapper, &mut state, &last_panes, &arc_file);`
(match the existing argument order; append `arc_file` last, consistent with the signature change.)

- [ ] **Step 3: Remove the orphaned exit-save standalone-map fallback**

In the exit block (`main.rs:3813-3823`), delete the `Err(e)` fallback's `save_map` call so the arm just logs:

```rust
            Err(e) => {
                eprintln!("babelmap: warning: could not save to {}: {}", arc_file.display(), e);
            }
```

- [ ] **Step 4: Remove now-unused standalone-map helpers + fix imports**

Compile (`cargo build -p app`) and remove everything the compiler flags as unused from the standalone-map removal:
- `map_dir` fn (`main.rs:116`) and its test (`main.rs:6002`).
- `map_path` (`ifid.rs:3`) and its test (`ifid.rs:18`).
- `save_map` (`persist_files.rs:189`) and any `save_map`-specific tests (`persist_files.rs:236` etc. — keep tests that also exercise `load_map`).
- Drop `save_map` and `map_path` from the `use` imports at `main.rs:25`/`27`; **keep `load_map`** (Task 3 uses it).

- [ ] **Step 5: Verify**

Run: `cargo test -p app` and `cargo build -p app`
Expected: PASS, no unused-item warnings. Manually confirm the two behaviors (no automated startup harness exists): with a `.babelmap` present, `auto_load=true` resumes with its map; `auto_load=false` starts blank; accepting the launch-resume dialog brings the map.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/main.rs crates/app/src/ifid.rs crates/app/src/persist_files.rs
git commit -m "feat(app): map loads only with .babelmap state; drop standalone map auto-load (SQ-0226)

Quest: SQ-0226"
```

---

### Task 2: Remove the `use_default_map` config option

**Files:**
- Modify: `crates/app/src/config.rs` (field `349-353`; default `471`; resolve `543`; write_config `603`; tests `810-826`, `687`, `918`, `956`)
- Modify: `crates/app/src/render/config_screen.rs` (row list `14`; value renderer `157`)
- Modify: `crates/app/src/input.rs` (toggle `3619`; cycle `3656`)

**Interfaces:** Consumes nothing new. After Task 1 there are no remaining readers of `cfg.use_default_map` outside config plumbing, so it can be deleted cleanly.

- [ ] **Step 1: Delete the field and its plumbing**

Remove, in `config.rs`: the `use_default_map` struct field + its doc comment + `#[serde(default)]` (`349-353`); the `Default` line (`471`); the `resolve()` merge line (`543`); the `write_config()` line (`603`); the three dedicated tests `use_default_map_is_false_by_default`, `use_default_map_parses_from_toml`, `use_default_map_omitted_stays_false` (`810-826`); the `use_default_map: true,` literal in the `write_config` round-trip test struct (`918`) and its assertion (`956`); and drop `use_default_map = true\n` from the `config_has_style_sections` fixture string (`687`) (leave the rest of that string intact).

- [ ] **Step 2: Delete the config-screen row and renumber**

In `config_screen.rs`: remove `("use_default_map", ConfigRowKind::Bool),` (row index 1, `14`) from `CONFIG_ROWS`. In `config_row_value` (`154-179`) remove the `1 => bool_str(cfg.use_default_map),` arm and **decrement every subsequent numeric arm by 1** (2→1, 3→2, … down the list). In `input.rs`, do the same renumbering in BOTH `config_toggle_or_edit` (remove `1 => …use_default_map…` at `3619`, decrement the rest) and `config_cycle` (remove `1 => …` at `3656`, decrement the rest). The row order after removal is: `user_dir`(0), `auto_load`(1), `auto_save`(2), `prompt_save_on_quit`(3), `prompt_load_on_launch`(4), `record_history`(5), `show_room_numbers`(6), `background_tidy`(7), `aux_storage`(8), `honor_game_colours`(9), `honor_timed_input`(10), `enable_sound`(11), `volume`(12).

- [ ] **Step 3: Verify**

Run: `cargo test -p app config`
Expected: PASS. Add/confirm a test that a config TOML still containing `use_default_map = true` parses fine (serde ignores the unknown key): 

```rust
#[test]
fn stale_use_default_map_key_is_ignored() {
    let cfg: crate::config::Config = toml::from_str("use_default_map = true").unwrap();
    let _ = cfg; // unknown key ignored, no panic
}
```

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/config.rs crates/app/src/render/config_screen.rs crates/app/src/input.rs
git commit -m "refactor(config): remove obsolete use_default_map option (SQ-0226)

Quest: SQ-0226"
```

---

### Task 3: Add the `/load-map <path>` command

**Files:**
- Modify: `crates/app/src/slash.rs` (`SlashOutcome` enum `31-66`; a new `CommandSpec` near the Map-category commands; parse tests)
- Modify: `crates/app/src/main.rs` (`dispatch_slash_outcome` `3841-…`: a new match arm)
- Test: `crates/app/src/slash.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::colors::expand_path(s, base_dir)` (`colors.rs:746`, `pub(crate)`); `load_map` (`persist_files.rs:196`, `Option<Mapper>`); `SlashOutcome`, `CommandSpec`, `Category::Map`, `Context::Global`.
- Produces: `SlashOutcome::LoadMap(String)`.

- [ ] **Step 1: Write the failing parse tests**

Add to `slash.rs` tests:

```rust
#[test]
fn load_map_parses_path_argument() {
    assert!(matches!(parse("load-map ~/Downloads/map.json", '/'),
        SlashOutcome::LoadMap(p) if p == "~/Downloads/map.json"));
}

#[test]
fn load_map_without_path_is_an_error() {
    assert!(matches!(parse("load-map", '/'), SlashOutcome::Error(_)));
}
```

- [ ] **Step 2: Run to confirm they fail**

Run: `cargo test -p app load_map_parses_path_argument load_map_without_path`
Expected: FAIL — no `LoadMap` variant / command.

- [ ] **Step 3: Add the `SlashOutcome` variant + command spec**

In `slash.rs`, add to `SlashOutcome`:

```rust
    /// Load a standalone map file (path argument) into the current session.
    LoadMap(String),
```

Add a `CommandSpec` in the Map-category group:

```rust
    CommandSpec { name: "load-map", category: Category::Map, context: Context::Global,
        usage: "load-map <path>", description: "load a standalone map file into the current session",
        dispatch: |a| match a.first() {
            Some(p) => SlashOutcome::LoadMap(p.to_string()),
            None => SlashOutcome::Error("load-map: a file path is required".into()),
        } },
```

- [ ] **Step 4: Handle the outcome (load, replace, refresh)**

In `dispatch_slash_outcome` (`main.rs`), add a match arm mirroring the trimmed `/load` refresh (map-only, no game state):

```rust
        SlashOutcome::LoadMap(path) => {
            let full = crate::colors::expand_path(&path, &std::env::current_dir().unwrap_or_default());
            match load_map(&full) {
                Some(m) => {
                    *mapper = m;
                    state.set_viewed_layer(None);
                    if let Some(rid) = mapper.graph.current() {
                        state.select_room(Some(rid));
                        if let Some(pos) = mapper.graph.room(rid).and_then(|r| r.pos) {
                            let (pw, ph) = map_pane_dims(map_rect);
                            state.recenter_on(pos, pw, ph);
                        }
                    }
                    state.set_status(format!("loaded map: {}", full.display()));
                }
                None => state.set_status(format!("load-map failed: {}", full.display())),
            }
        }
```

(Confirm `mapper.graph.current()` returns the current `RoomId`; if the accessor differs, use the graph's current-room accessor. `expand_path` handles `~`; relative paths resolve against cwd.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p app`
Expected: PASS (parse tests + full suite). Also confirm `cargo check -p app` is clean.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/slash.rs crates/app/src/main.rs
git commit -m "feat(app): /load-map <path> loads a standalone map on demand (SQ-0226)

Quest: SQ-0226"
```

---

## Self-Review notes

- **Spec coverage:** startup gate on `auto_load` (T1), launch-resume adopts map (T1), all standalone auto-load + orphaned helpers removed (T1), `use_default_map` deleted (T2), `/load-map` command with `~` expansion + replace + error handling (T3), explicit paths untouched (no task changes them). ✔
- **Type consistency:** `SlashOutcome::LoadMap(String)` defined T3 Step 3, produced by the command spec, consumed by the `main.rs` arm; `expand_path`/`load_map` signatures used as-published. ✔
- **Verify-during-execution flags:** T1's startup/launch-resume changes have no unit harness — verified by build + manual smoke (called out in T1 Step 5). T3's load-apply arm runs in `main.rs`; its parse layer is unit-tested, the apply layer is manual + `cargo check`. The `mapper.graph.current()` accessor name must be confirmed against the real API during T3.
