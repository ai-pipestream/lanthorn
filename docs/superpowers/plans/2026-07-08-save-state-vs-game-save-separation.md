# Separate Save State (.babelmap) from Game Save (.qzl) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give babelmap's two save mechanisms different file formats — `.babelmap` = emulator Save State (resume-convention), `.qzl` = the game's `@save` (bare standard Quetzal, descriptor-convention) — so restore dispatches on extension, the SQ-0163 host-restore regression is fixed by construction, and game saves become portable.

**Architecture:** Extension = kind. Restore keys off the loaded file's extension: `.babelmap` → resume (`restore_file`); `.qzl` → complete the descriptor (`complete_restore_success` semantics). In-game `@save` writes a bare `.qzl` (VM state only). Save State keeps writing `.babelmap`. Plus the `save-game`→`save-state` rename.

**Tech Stack:** Rust — `crates/app` (ratatui TUI: `main.rs`, `session.rs`, `glulx_session.rs`, `engine.rs`, `persist_files.rs`, `slash.rs`, `keymap.rs`, `render/`), `crates/zvm` (`complete_restore_success` already exists).

## Global Constraints

- `zvm`/`gvm` library crates stay ZERO-dependency (test code may use `std`).
- Restore convention, exact: `.qzl` → complete descriptor (v3 branch true / v4+ store 2, advance PC); `.babelmap` → resume at saved PC. Foreign `.qzl` import completes the descriptor.
- In-game `@save` writes a bare `.qzl` captured while `pending_save` is set (so `save_pc()` = descriptor PC). It is Z-machine only (Glulx has no `@save`).
- No backward-compat/migration (user deletes old descriptor-PC `.babelmap` slots). Legacy resume-convention `.babelmap`s must still resume correctly.
- Commit trailer on every commit: `Quest: SQ-0227`, then `Co-Authored-By` / `Claude-Session`.
- Keep the diff surgical; match existing style.

## File Structure / responsibilities

- `crates/app/src/engine.rs` — add `restore_game_save(&mut self, bytes) -> Result<(), EngineError>` to the `Engine` trait.
- `crates/app/src/session.rs` — zvm impl of `restore_game_save` (→ `complete_restore_success`).
- `crates/app/src/glulx_session.rs` — Glulx impl (error: Glulx has no game-save format).
- `crates/app/src/persist_files.rs` — `restore_game` completes the descriptor; a `.qzl` game-save writer path (reuse `save_game`).
- `crates/app/src/main.rs` — in-game `@save` writes `.qzl`; load/restore handlers dispatch on extension; rename hint/label strings.
- `crates/app/src/slash.rs`, `keymap.rs`, `render/saves.rs`, `render/quit_dialog.rs` — rename.
- `docs/features/saves.md`, `README.md` — correct + rename.

---

### Task 1: Descriptor-completing restore for game saves + foreign import

**Files:**
- Modify: `crates/app/src/engine.rs` (`Engine` trait, near `restore_state` ~387)
- Modify: `crates/app/src/session.rs` (zvm impl ~783-793)
- Modify: `crates/app/src/glulx_session.rs` (~367)
- Modify: `crates/app/src/persist_files.rs` (`restore_game` ~207-213)
- Test: `crates/app/src/persist_files.rs` tests module

**Interfaces:**
- Produces: `Engine::restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError>` — restores a bare standard Quetzal *game* save by completing the descriptor. zvm → `machine.complete_restore_success(bytes)`; Glulx → `Err` (no game-save format).
- Consumes: `zvm::cpu::exec::Machine::complete_restore_success(&[u8]) -> Result<(), ZError>` (exists).

- [ ] **Step 1: Write the failing test**

In `persist_files.rs` tests, assert that restoring a descriptor-PC game save via `restore_game` completes the descriptor (resumes past the `@save`, game sees "restored"):

```rust
#[test]
fn restore_game_completes_descriptor_of_a_gamesave_qzl() {
    // Build a v4 machine that @saves G0; capture the game-save .qzl (pending_save set
    // => descriptor PC), then restore_game() must complete it: G0==2, pc past the save.
    use zvm::cpu::exec::{Machine, StepResult};
    use zvm::memory::Memory;
    let mut buf = zvm::header::tests_support::sample_story(4);
    buf[0x40] = 0xB5; buf[0x41] = 0x10; buf[0x42] = 0xBA; // save->G0 ; quit
    let mut m = Machine::new(Memory::new(buf).unwrap());
    m.state.pc = 0x40;
    assert_eq!(m.step(), StepResult::SaveRequest);
    let blob = m.save_quetzal();               // descriptor PC (0x41), pending_save set
    m.complete_save(true);
    // Persist the game save and restore it via the game-save path.
    let tmp = std::env::temp_dir().join(format!("bm-gs-{}.qzl", std::process::id()));
    std::fs::write(&tmp, &blob).unwrap();
    m.do_store(Some(0x10), 0x99); m.state.pc = 0x00AB;
    super::restore_game(&tmp, &mut m).expect("restore game save");
    assert_eq!(m.global(0), 2, "game-save restore completes the @save descriptor (store 2)");
    assert_eq!(m.state.pc, 0x42, "resumes at the post-@save address");
    let _ = std::fs::remove_file(&tmp);
}
```

- [ ] **Step 2: Run it, confirm it fails**

Run: `cargo test -p app restore_game_completes_descriptor_of_a_gamesave_qzl`
Expected: FAIL — current `restore_game` calls `restore_quetzal` (resume), leaving pc=0x41 / G0≠2.

- [ ] **Step 3: Implement**

`persist_files.rs` `restore_game` — complete the descriptor:

```rust
pub fn restore_game(path: &Path, machine: &mut zvm::cpu::exec::Machine) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    machine.complete_restore_success(&bytes).map_err(|e| match e {
        zvm::error::ZError::SaveMismatch => "save is for a different story".to_string(),
        other => format!("restore failed: {:?}", other),
    })
}
```

`engine.rs` — add to the `Engine` trait (beside `restore_state`):

```rust
    /// Restore a bare standard Quetzal *game* save (`.qzl`) by completing the save
    /// instruction's descriptor (v3 branch true / v4+ store 2). Z-machine only.
    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError>;
```

`session.rs` (zvm impl):

```rust
    fn restore_game_save(&mut self, bytes: &[u8]) -> Result<(), EngineError> {
        self.machine.complete_restore_success(bytes)
            .map_err(|e| EngineError::BadSave(format!("{e:?}")))
    }
```

`glulx_session.rs` (Glulx has no game-save format):

```rust
    fn restore_game_save(&mut self, _bytes: &[u8]) -> Result<(), EngineError> {
        Err(EngineError::BadSave("Glulx has no game-save (.qzl) format".into()))
    }
```

- [ ] **Step 4: Run to green**

Run: `cargo test -p app` — new test passes; existing save/restore tests stay green.

- [ ] **Step 5: Commit** (`feat(app): descriptor-completing restore for game saves + foreign import (SQ-0227)`, trailers).

---

### Task 2: In-game `@save` writes a bare `.qzl`

**Files:**
- Modify: `crates/app/src/main.rs` (`PromptKind::SaveAs` submit handler ~4250-4290)
- Modify: `crates/app/src/persist_files.rs` (a `.qzl` game-save write helper, if a path helper is wanted; else reuse `save_game`)
- Test: an app-level test (or a focused `persist_files` test) that an in-game save writes `<ifid>-<slug>.qzl`, not `.babelmap`.

**Interfaces:**
- Consumes: `persist_files::save_game(path, &machine)` (writes `machine.save_quetzal()`); `zvm_session_opt(&*session)` to reach `&z.machine`; the existing `slugify`/saves-dir conventions.
- Produces: in-game `@save` → `<dir>/<ifid>-<slug>.qzl` (descriptor PC, since `pending_save` is set at this point). Host save-as (`ingame == false`) is unchanged (`.babelmap` via `save_named`).

- [ ] **Step 1: Write the failing test**

Add a `persist_files` helper `save_game_named(dir, ifid, name, machine) -> io::Result<PathBuf>` that writes `<dir>/<ifid>-<slug>.qzl` and returns the path, and test it writes a `.qzl` whose bytes equal `machine.save_quetzal()`:

```rust
#[test]
fn save_game_named_writes_bare_qzl() {
    let m = /* a Machine at an @save SaveRequest (pending_save set) — reuse the Task 1 setup */;
    let dir = std::env::temp_dir();
    let path = super::save_game_named(&dir, "IFIDX", "slot one", &m).unwrap();
    assert!(path.to_string_lossy().ends_with("IFIDX-slot-one.qzl"));
    assert_eq!(std::fs::read(&path).unwrap(), m.save_quetzal(), "bare Quetzal bytes");
    let _ = std::fs::remove_file(&path);
}
```
(Build the `Machine` exactly as in Task 1 Step 1 up to the `SaveRequest`; do not call `complete_save` so `pending_save` stays set → descriptor PC.)

- [ ] **Step 2: Run it, confirm it fails** (`save_game_named` undefined).

- [ ] **Step 3: Implement `save_game_named`** in `persist_files.rs` (mirror `save_named`'s slugify + path build, but write `.qzl` via `save_game`):

```rust
pub fn save_game_named(dir: &Path, ifid: &str, name: &str, machine: &zvm::cpu::exec::Machine) -> io::Result<std::path::PathBuf> {
    let slug = slugify(name);
    if slug.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "save name is empty after sanitization"));
    }
    let path = dir.join(format!("{}-{}.qzl", ifid, slug));
    save_game(&path, machine)?;
    Ok(path)
}
```

- [ ] **Step 4: Route in-game `@save` to it**

In `main.rs` `PromptKind::SaveAs` submit handler, branch on the existing `ingame` flag: when `ingame` (game `@save`), write the game save; otherwise keep `save_named` (host Save State). Replace the single `match save_named(...)` with:

```rust
            let result = if ingame {
                // Game @save -> bare standard .qzl (VM state only, descriptor PC).
                let machine = &zvm_session_opt(&*session).expect("in-game save is Z-machine only").machine;
                save_game_named(dir, ifid, &buf, machine).map(|_| ())
            } else {
                // Host "Save State" named slot -> rich .babelmap archive.
                save_named(dir, ifid, &buf, mapper, &session.save_state(), zvm_session_opt(&*session).map(|z| &z.machine.screen), session.aux_data(), state.turns, &state.transcript, &state.transcript_kinds, &state.transcript_runs)
            };
            match result {
                Ok(()) => { /* unchanged: push_transcript, refresh saves list, ingame flag-hop */ }
                Err(e) => { /* unchanged */ }
            }
```
Keep the existing `Ok`/`Err` bodies (transcript, saves-list refresh, `ingame_resume_save` flag-hop) verbatim.

- [ ] **Step 5: Run to green** — `cargo test -p app`. Manually confirm no compile refs to `save_named` for the in-game branch remain.

- [ ] **Step 6: Commit** (`feat(app): in-game @save writes a bare standard .qzl game save (SQ-0227)`, trailers).

---

### Task 3: Restore dispatch on file extension (+ the regression test)

**Files:**
- Modify: `crates/app/src/main.rs` — the load/restore handlers: saves-manager Load (~3506-3617), `/load-game` (~3920-3996), replay/launch/auto paths (~3628, 4969, 1738), and the in-game restore branch (~3516-3547). Import (~3448) already fixed in Task 1.
- Test: `crates/app/src/main.rs` tests (or a focused integration) for extension dispatch + the regression.

**Interfaces:**
- Consumes: `Path::extension()`; `restore_game`/`restore_game_save` (Task 1, complete descriptor for `.qzl`); `restore_state`/`load_archive` (resume for `.babelmap`).
- Rule (apply at every load site): if the selected file is `.qzl` → **complete the descriptor** (`session.restore_game_save(&bytes)` or `restore_game` for the bare-machine path); if `.babelmap` → **resume** (`load_archive` → `session.restore_state(&ac.engine_save())`, as today).

- [ ] **Step 1: Write the failing regression test**

Drive a v4 (or v3) fixture story to an in-game `@save`, write the `.qzl` (Task 2 path), then restore it via the **host** load path and assert it resumes correctly (this is the SQ-0163 regression, currently red) — and that a `.babelmap` still resumes. Model on the existing app save/restore tests; assert on the resulting `session`/machine PC and a "restored" store value. (If a full app-loop test is heavy, write a focused test that calls the same extension-dispatch helper directly.)

- [ ] **Step 2: Run it, confirm it fails** (host restore of the `.qzl` currently resumes at the descriptor / uses `restore_file`).

- [ ] **Step 3: Implement extension dispatch**

Factor a small helper (e.g. in `main.rs` or a `persist_files` helper) `fn is_game_save(path: &Path) -> bool { path.extension().is_some_and(|e| e == "qzl") }`. At each load site, branch: `.qzl` → complete-descriptor restore; else → the existing archive-resume path. For the **in-game restore branch** (`state.ingame_io == Some(Restore)`, ~3516-3547) that currently always calls `resume_restore(Some(&bytes))`: keep `resume_restore` for `.qzl` (it already completes the descriptor via `complete_restore_success`), but for a picked `.babelmap` use the resume path. For the **host** saves-manager Load / `/load-game` / launch / auto paths that currently call `restore_state`: add the `.qzl` → `restore_game_save` branch.

- [ ] **Step 4: Run to green** — regression test passes; `.babelmap` resume unchanged; `cargo test -p app` green.

- [ ] **Step 5: Commit** (`fix(app): dispatch restore on save file extension; fixes SQ-0163 host-restore regression (SQ-0227)`, trailers).

---

### Task 4: Rename `save-game`/`load-game` → `save-state`/`restore-state`

**Files:**
- Modify: `crates/app/src/slash.rs` (~141-146), `keymap.rs` (~212-213, test ~835), `main.rs` (`GAME_HINTS` ~153-158; hint-label assertion ~5414; prompt/label strings ~661-665, 684-693, 801-848), `render/saves.rs` (~43, 47, 138), `render/quit_dialog.rs` (~46, 51, 69).
- Modify docs: `docs/features/saves.md` (~13-17, fix the inaccurate "separate format" note), `README.md` (~63-64).

**Interfaces:** Consumes: the command registry + keymap binding format. Produces: command names `save-state` / `restore-state` bound to Ctrl+S / Ctrl+R.

- [ ] **Step 1: Update the failing tests first** — change `keymap.rs:835` (`"ctrl+s" -> "save-state"`) and `main.rs:5414` (`"Ctrl+S: save state"`) to the new labels; run, watch them fail.

- [ ] **Step 2: Rename the command specs** in `slash.rs`: `name: "save-state"` (usage/description reworded to "save state / emulator snapshot"), `name: "restore-state"`. Keep the `SlashOutcome` variants; only the user-facing names change.

- [ ] **Step 3: Rename bindings + hints** — `keymap.rs:212-213` bind Ctrl+S→`"save-state"`, Ctrl+R→`"restore-state"`; `GAME_HINTS` entries; the dialog/prompt label strings (`"Save name"`→context-appropriate; saves-manager footer/title as specified — keep the manager title "Saves").

- [ ] **Step 4: Fix docs** — `docs/features/saves.md:13-17`: replace the inaccurate note with the correct model (`.babelmap` = Save State; `.qzl` = the game's standard save; different files). `README.md` bullet reworded.

- [ ] **Step 5: Run to green** — `cargo test -p app` green (renamed-label tests pass).

- [ ] **Step 6: Commit** (`refactor(app): rename save-game/load-game -> save-state/restore-state (SQ-0227)`, trailers).

---

### Task 5: Integration round trip + final verification

**Files:** Test in `crates/app` (session-level); final workspace check.

- [ ] **Step 1: Write the round-trip test** — drive a v3 story (`minizork.z3`) or a v4 fixture through the game's `save`→`.qzl`→`restore` of that `.qzl`, asserting probe-based state equivalence (per SQ-0158's oracle: `play(prefix).probe()` == `restore(qzl).probe()`). Confirms the game-save format works end to end.

- [ ] **Step 2: Run to green** — `cargo test -p app <name>`.

- [ ] **Step 3: Final verification:**
```bash
cargo test -p zvm -p app        # all green
cargo build --workspace         # no warnings (no orphaned save_named refs / unused imports)
grep -rn "save-game\|load-game" crates/app/src   # only intentional (none, or historical comments)
```

- [ ] **Step 4: Commit** (`test(app): in-game @save/@restore .qzl round trip (SQ-0227)`, trailers).

---

## Self-review checklist (before execution)

- Each task ends green independently (Task 1 restore path, Task 2 write path, Task 3 dispatch+regression, Task 4 rename, Task 5 integration).
- The regression test (Task 3) is red before Task 3 and green after — it is the acceptance proof.
- No descriptor-PC data can reach a `.babelmap` (in-game `@save` writes `.qzl` only; Save State sites write `.babelmap` with `pending_save == None`).
- Glulx: `restore_game_save` errors (never invoked — no `.qzl` for Glulx); Glulx Save State (`.babelmap`) unchanged.
- `zvm`/`gvm` gain no dependencies.
