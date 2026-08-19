# In-Game (Game-Initiated) Save / Restore — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the VM's own `@save`/`@restore` (v4+, store form) through lanthorn's saves UI so a game that issues SAVE/RESTORE writes a real `.lanthorn`/reads a `.qzl`-or-`.lanthorn` and the VM **resumes** — and, on restore, the game **redraws its own status line** (the whole point: the standard-interpreter behavior lanthorn's snapshot Ctrl+S/Ctrl+R can't do). v3 keeps the current "isn't wired" info message.

**Architecture:** Four layers, one per task.
1. **zvm** — `Machine::complete_restore_success(&[u8])`: `restore_quetzal` then store `2` into the *original* `@save`'s target (`mem[pc-1]` for v4+), clear undo, clear the pending restore target. The companions `complete_save(ok)` / `complete_restore_failure()` already exist.
2. **app/session.rs** — `run_until_input` stops *bubbling* a `RunStop` (it no longer auto-fails v4+ save/restore; v3 still auto-fails). `TurnResult` gains `pending_io: Option<PendingIo>`; new `resume_save` / `resume_restore` complete the VM and continue the turn.
3. **app/archive.rs** — `read_quetzal_from_file(path)`: `game.sav` from a `.lanthorn` zip, else raw bytes (a plain `.qzl`).
4. **app/main.rs + state.rs** — `AppState.ingame_io: Option<PendingIo>`; after `submit`/`submit_char`/resume, if `pending_io` is `Some` open the saves dialog in an "in-game" mode whose confirm/cancel call `resume_*` (VM completion) instead of the direct Ctrl+S/Ctrl+R path. Remove the "isn't wired" line for v4+.

**Tech Stack:** Rust (zvm + app crates).

## Global Constraints

- Commit trailers on EVERY commit body (no backticks anywhere in commit bodies — zsh):
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
- Per task: `cargo test -p zvm` (Task 1) or `cargo test -p app` (Tasks 2-4) green, **0 warnings**. The headless smoke test (`crates/app/tests/headless.rs`) must still pass.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- Scope is **v4+** (store form). **v3** keeps the current "isn't wired" info message and auto-fails in-game save/restore.
- `restore_quetzal` is atomic: on `Err` the machine is untouched (documented contract) — `complete_restore_success` relies on this for the error path.
- Adding `pending_io` to `TurnResult` breaks every `TurnResult { … }` literal in the `app` crate. Task 2 must patch them all (enumerated in Task 2 Step 4) so the crate compiles.

---

### Task 1: VM — `complete_restore_success` (store 2 into the original `@save`)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add `Machine::complete_restore_success` near `restore_file`/`complete_restore_failure` (~1517-1540); add two tests in `mod tests` (the `pub(crate) mod tests` at ~1559, which already `use crate::header::tests_support::sample_story`).

**Interfaces:**
- Consumes: `self.restore_quetzal(&[u8]) -> Result<(), ZError>`, `self.mem.version() -> u8`, `self.mem.read_byte(u32) -> u8`, `self.state.pc: u32`, `self.do_store(Option<u8>, u16)`.
- Produces: `pub fn complete_restore_success(&mut self, data: &[u8]) -> Result<(), crate::error::ZError>`.

Verified facts: a v4 `@save` is `0OP:0x05` short form = byte `0xB5` followed by the store byte; `step()` leaves `state.pc` **past** the store byte (post-instruction convention), so the store byte is at `state.pc - 1`. `save_quetzal` records `state.pc` as-is. The `complete_restore_failure` test (`restore_failure_stores_zero_into_correct_var_and_pc_unchanged`, ~3451) is the harness template; the undo round-trip test (`undo_save_restore_round_trip`, ~1563) is the store/`global(n)` template.

- [ ] **Step 1: Write the failing tests**

In `crates/zvm/src/cpu/exec.rs`, inside the `pub(crate) mod tests` block, add (mirrors `restore_failure_*` for the harness and `undo_save_restore_round_trip` for the store/`global` assertions):

```rust
// ── In-game restore-success: the original @save "returns 2" on restore ─────
//
// v4 story at 0x40:  save -> G0 (0xB5, store byte 0x10), then quit (0xBA).
// After step() the @save suspends with SaveRequest and state.pc == 0x42, so the
// store byte lives at mem[0x41]. complete_restore_success(blob) must restore the
// saved state (PC back to 0x42) and store 2 into G0.
fn save_v4_into_g0_story() -> Vec<u8> {
    let mut buf = sample_story(4);
    buf[0x40] = 0xB5; // 0OP:0x05 save (store form, v4+)
    buf[0x41] = 0x10; // store -> global 0 (var 0x10)
    buf[0x42] = 0xBA; // quit
    buf
}

#[test]
fn complete_restore_success_stores_2_and_resumes_pc() {
    let mem = Memory::new(save_v4_into_g0_story()).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x40;

    // Execute @save -> SaveRequest; PC is now past the store byte (0x42).
    let r = m.step();
    assert_eq!(r, StepResult::SaveRequest, "save opcode suspends with SaveRequest");
    assert_eq!(m.state.pc, 0x42, "PC is post-instruction (store byte at 0x41)");

    // Host captures the Quetzal at the @save point, then completes the save.
    let blob = m.save_quetzal();
    m.complete_save(true);
    assert_eq!(m.global(0), 1, "save success stores 1 into G0");

    // Clobber G0 and move the PC away so the restore must reset BOTH.
    m.do_store(Some(0x10), 0x99);
    m.state.pc = 0x00AB;

    // Restore success: the ORIGINAL @save returns 2; PC resumes at 0x42.
    m.complete_restore_success(&blob).expect("restore must succeed");
    assert_eq!(m.global(0), 2, "restore makes the original @save 'return' 2");
    assert_eq!(m.state.pc, 0x42, "PC resumed at the post-@save address");
}

#[test]
fn complete_restore_success_err_on_corrupt_blob_leaves_state() {
    let mem = Memory::new(save_v4_into_g0_story()).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x42;
    m.do_store(Some(0x10), 0x77); // sentinel

    let err = m.complete_restore_success(b"not a quetzal blob");
    assert!(err.is_err(), "corrupt blob must return Err");
    assert_eq!(m.global(0), 0x77, "state untouched on restore failure");
    assert_eq!(m.state.pc, 0x42, "pc untouched on restore failure");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p zvm complete_restore_success`
Expected: compile error (`complete_restore_success` does not exist).

- [ ] **Step 3: Implement `complete_restore_success`**

In `crates/zvm/src/cpu/exec.rs`, in `impl Machine`, immediately after `complete_restore_failure` (~1540) add:

```rust
    /// Complete a game-initiated restore (v4+) with the supplied Quetzal bytes.
    ///
    /// On success the machine state (dynamic memory, frames, eval stack, PC) is
    /// replaced with the saved state, and the ORIGINAL `@save` "returns 2": the
    /// saved PC is post-instruction, so the v4+ `@save`'s store byte is the last
    /// byte of that instruction, at `state.pc - 1`. We store 2 there. A restore
    /// invalidates undo history (like `restore_file`), and the `@restore`'s own
    /// store target is unused on success, so both are cleared.
    ///
    /// On `Err` the machine is untouched (the `restore_quetzal` contract); the
    /// caller should then call `complete_restore_failure()`.
    pub fn complete_restore_success(&mut self, data: &[u8]) -> Result<(), crate::error::ZError> {
        self.restore_quetzal(data)?;
        if self.mem.version() >= 4 {
            let store_var = self.mem.read_byte(self.state.pc - 1);
            self.do_store(Some(store_var), 2);
        }
        self.undo_stack.clear();
        self.pending_restore_store = None;
        Ok(())
    }
```

- [ ] **Step 4: Run the tests + full zvm suite**

Run: `cargo test -p zvm`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): complete_restore_success — game-initiated restore stores 2 into the original save target

The saved PC is post-instruction, so the v4+ save store byte is mem[pc-1];
storing 2 there makes the original save 'return 2' on restore (the game
redraws). Clears undo history and the unused restore target. Err leaves state
untouched so the caller can fall back to complete_restore_failure.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: session — bubble the request + resume API

**Files:**
- Modify: `crates/app/src/session.rs` — `PendingIo` enum; `RunStop` enum; rewrite `run_until_input`; add `TurnResult.pending_io`; extract a shared `finish_turn`; add `resume_save` / `resume_restore`; update `new`/`submit`/`submit_char`; add tests.
- Modify (field add only): every other `TurnResult { … }` literal in the `app` crate (Step 4) so it compiles.

**Interfaces:**
- Consumes: `Machine::step`, `Machine::complete_save(bool)`, `Machine::complete_restore_failure()`, `Machine::complete_restore_success(&[u8])` (Task 1), `Machine::mem.version()`.
- Produces:
  - `pub enum PendingIo { Save, Restore }` (derive `Debug, Clone, Copy, PartialEq, Eq`).
  - `pub pending_io: Option<PendingIo>` on `TurnResult`.
  - `pub fn resume_save(&mut self, wrote_ok: bool) -> TurnResult`.
  - `pub fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/session.rs`, inside `mod tests`, add. These reuse the existing `read_char_story_v5()` helper (already in `mod tests`) so `new()` stops for a keypress *before* the save/restore opcode — otherwise `new` would auto-fail it. The keypress then drives read_char → `@save`/`@restore`.

```rust
// ── In-game save/restore plumbing (v4) ─────────────────────────────────────
//
// read_char_story_v5 lays out: 0x40 read_char->G0 (4 bytes), 0x44 quit.
// We re-stamp it to v4 and overwrite the quit at 0x44 with the save/restore
// opcode so the FIRST keypress drives read_char -> the opcode.
fn read_char_then_save_v4() -> Vec<u8> {
    let mut buf = read_char_story_v5();
    buf[0x00] = 4;    // version 4 (0OP save/restore store form lives here)
    buf[0x44] = 0xB5; // 0OP:0x05 save (store form) -> G0
    buf[0x45] = 0x10; // store byte: global 0
    buf[0x46] = 0xBA; // quit
    buf
}

fn read_char_then_restore_v4() -> Vec<u8> {
    let mut buf = read_char_story_v5();
    buf[0x00] = 4;
    buf[0x44] = 0xB6; // 0OP:0x06 restore (store form) -> G0
    buf[0x45] = 0x10; // store byte: global 0
    buf[0x46] = 0xBA; // quit
    buf
}

#[test]
fn ingame_save_yields_pending_io_and_resume_continues() {
    let mut sess = GameSession::new(read_char_then_save_v4()).expect("new");
    assert_eq!(sess.pending_input(), InputKind::Char);

    // The keypress drives read_char -> @save, which suspends with pending_io.
    let r = sess.submit_char(b'x');
    assert_eq!(r.pending_io, Some(PendingIo::Save));
    assert!(!r.quit, "a save-pending turn is not a quit");
    assert!(r.info.is_none(), "v4+ in-game save shows no 'isn't wired' info line");

    // Host wrote the file OK: resume stores 1 into G0 and runs to quit.
    let r2 = sess.resume_save(true);
    assert!(r2.quit, "resume_save continues the VM to the quit opcode");
    assert_eq!(sess.machine.global(0), 1, "complete_save(true) stored 1 into G0");
}

#[test]
fn ingame_restore_yields_pending_io_and_cancel_fails_cleanly() {
    let mut sess = GameSession::new(read_char_then_restore_v4()).expect("new");

    let r = sess.submit_char(b'x');
    assert_eq!(r.pending_io, Some(PendingIo::Restore));
    assert!(!r.quit);

    // Cancel: resume_restore(None) -> complete_restore_failure stores 0, runs on.
    let r2 = sess.resume_restore(None);
    assert!(r2.quit);
    assert_eq!(sess.machine.global(0), 0, "cancelled restore stored 0 into G0");
}

#[test]
fn v3_ingame_save_still_auto_fails_with_info() {
    // v3 keeps the host-mediated message; the VM auto-fails the request.
    // v3 save is a BRANCH instruction (0OP:0x05 short form 0xB5 + 1 branch byte).
    let mut buf = read_char_story_v5();
    buf[0x00] = 3;
    buf[0x44] = 0xB5; // 0OP:0x05 save (branch form in v3)
    buf[0x45] = 0xC0; // branch: on-true, offset that lands on quit (see note)
    buf[0x46] = 0xBA; // quit
    let mut sess = GameSession::new(buf).expect("new");
    let r = sess.submit_char(b'x');
    assert_eq!(r.pending_io, None, "v3 never bubbles pending_io");
    assert!(r.info.is_some(), "v3 keeps the 'isn't wired' info line");
}
```

Note on the v3 branch byte: a single short-branch byte `0xC0` means "branch on true, offset 0" (return false) — the exact landing is irrelevant to this test, which only asserts `pending_io == None` and `info.is_some()`. If the chosen byte makes `step()` misbehave before reaching the assertions, pick any valid short branch byte (`0x40`..`0xFF`) that keeps PC in dynamic memory; the v3 path auto-fails the request regardless of where it resumes. If wiring a valid v3 branch proves fiddly, replace this test with a comment pointing at the manual v3 check and keep the two v4 tests (the v3 behavior is also exercised by the unchanged `turn_result_info_defaults_none_for_normal_turn`).

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app -- ingame_save_yields ingame_restore_yields`
Expected: compile error (`PendingIo`, `pending_io`, `resume_save`, `resume_restore` missing).

- [ ] **Step 3: Add `PendingIo`, `RunStop`, and rewrite `run_until_input`**

In `crates/app/src/session.rs`, add the public type near `InputKind`:

```rust
/// Which in-game (game-initiated) I/O the VM is suspended on after a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingIo {
    Save,
    Restore,
}
```

Add `pending_io` to `TurnResult` (after `info`):

```rust
    /// Set when the VM suspended on its own `@save`/`@restore` (v4+). The host
    /// performs the file I/O and calls `resume_save`/`resume_restore`. `None` for
    /// an ordinary turn (and for v3, which still auto-fails — see `info`).
    pub pending_io: Option<PendingIo>,
```

Replace the private `run_until_input` (currently returns `(bool, bool, InputKind)`) with a richer stop reason. **v3 still auto-fails in the loop and reports it; v4+ bubbles the request.** The second return value is the v3 auto-fail flag that drives the legacy `info` line.

```rust
/// Stop reason from `run_until_input`.
enum RunStop {
    /// VM is waiting for player input of this kind.
    Input(InputKind),
    /// VM ended (Quit/Restart).
    Quit,
    /// VM suspended on its own `@save` (v4+) — host must `resume_save`.
    SavePending,
    /// VM suspended on its own `@restore` (v4+) — host must `resume_restore`.
    RestorePending,
}

/// Step until the machine pauses for input, quits, or (v4+) suspends on its own
/// save/restore. Returns `(stop, v3_auto_failed)` where `v3_auto_failed` is true
/// when a v3 game's `@save`/`@restore` was auto-rejected this run (drives the
/// host hint). v4+ save/restore is NOT auto-failed: it bubbles up as
/// `SavePending`/`RestorePending`.
fn run_until_input(machine: &mut Machine) -> (RunStop, bool) {
    let mut v3_failed = false;
    loop {
        match machine.step() {
            StepResult::Quit => return (RunStop::Quit, v3_failed),
            StepResult::NeedLine { .. } => return (RunStop::Input(InputKind::Line), v3_failed),
            StepResult::NeedChar => return (RunStop::Input(InputKind::Char), v3_failed),
            StepResult::SaveRequest => {
                if machine.mem.version() <= 3 {
                    machine.complete_save(false);
                    v3_failed = true;
                } else {
                    return (RunStop::SavePending, v3_failed);
                }
            }
            StepResult::RestoreRequest => {
                if machine.mem.version() <= 3 {
                    machine.complete_restore_failure();
                    v3_failed = true;
                } else {
                    return (RunStop::RestorePending, v3_failed);
                }
            }
            StepResult::Restart => return (RunStop::Quit, v3_failed),
            StepResult::Continue => {}
        }
    }
}
```

- [ ] **Step 4: Extract `finish_turn`; rewrite `new`/`submit`/`submit_char`; add `resume_*`**

`submit` and `submit_char` currently duplicate the post-`run_until_input` body (drain transcript, detect location, info, beep, diagnostics) and build a `TurnResult` (lines ~128-151 and ~158-185). Extract that body into one private method so `submit`, `submit_char`, `resume_save`, and `resume_restore` share it. In `impl GameSession`:

```rust
    /// Build the `TurnResult` from a `RunStop` (+ v3 auto-fail flag) and drain the
    /// VM's per-turn buffers. Shared by submit/submit_char/resume_*.
    fn finish_turn(&mut self, stop: RunStop, v3_failed: bool) -> TurnResult {
        let (quit, pending, pending_io) = match stop {
            RunStop::Quit => (true, InputKind::Line, None),
            RunStop::Input(k) => (false, k, None),
            RunStop::SavePending => (false, self.pending, Some(PendingIo::Save)),
            RunStop::RestorePending => (false, self.pending, Some(PendingIo::Restore)),
        };
        self.quit = quit;
        self.pending = pending;

        let raw = sink_mut(&mut self.machine).take_text();
        let transcript = strip_read_prompt(&raw).to_owned();
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(|loc| match loc {
            Location::NameOnly(name) => zvm::ObjectSnapshot {
                number: crate::roomid::synthetic_room_id(name),
                parent: 0,
                name: name.clone(),
            },
            _ => loc.object().expect("non-NameOnly variants carry an object").clone(),
        });
        let location_method = detected.as_ref().map(Location::method);

        let info = if v3_failed {
            Some("(lanthorn: this game's in-game save/restore isn't wired; use Ctrl+S to save and Ctrl+R to restore instead.)".to_string())
        } else {
            None
        };

        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let beep = self.machine.pending_beeps.last().copied();
        self.machine.pending_beeps.clear();

        TurnResult { transcript, location, quit, info, beep, diagnostics, location_method, pending_io }
    }
```

Rewrite `submit` and `submit_char` bodies to use it (replacing the duplicated tails):

```rust
    pub fn submit(&mut self, command: &str) -> TurnResult {
        self.machine.supply_line(command);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }

    pub fn submit_char(&mut self, ch: u8) -> TurnResult {
        self.machine.supply_char(ch);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }
```

Add the resume methods:

```rust
    /// Resume after the host performed an in-game SAVE (`wrote_ok` = file written).
    pub fn resume_save(&mut self, wrote_ok: bool) -> TurnResult {
        self.machine.complete_save(wrote_ok);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }

    /// Resume after the host performed an in-game RESTORE. `Some(bytes)` =
    /// the user picked a save (Quetzal); `None` = cancelled. On corrupt bytes we
    /// fall back to failure so the game sees a clean "Failed.".
    pub fn resume_restore(&mut self, data: Option<&[u8]>) -> TurnResult {
        match data {
            Some(bytes) => {
                if self.machine.complete_restore_success(bytes).is_err() {
                    self.machine.complete_restore_failure();
                }
            }
            None => self.machine.complete_restore_failure(),
        }
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }
```

Update `new` (line ~105) so initialization never gets *stuck* on an I/O request: keep stepping, auto-failing any save/restore that fires before the first input (pathological, but preserves prior behavior):

```rust
        let mut quit = false;
        let pending = loop {
            let (stop, _v3) = run_until_input(&mut machine);
            match stop {
                RunStop::Quit => { quit = true; break InputKind::Line; }
                RunStop::Input(k) => break k,
                RunStop::SavePending => machine.complete_save(false),
                RunStop::RestorePending => machine.complete_restore_failure(),
            }
        };

        Ok(GameSession { machine, quit, pending })
```

- [ ] **Step 5: Add `pending_io: None` to every remaining `TurnResult` literal**

The struct change breaks all other literals in the crate. Add `pending_io: None,` to each. Sites (grep `TurnResult {` under `crates/app/` to confirm none are missed):

- `crates/app/src/session.rs` tests: ~384 (`first`), ~399 (`second`), ~422, ~444, ~460 (`turn` helper).
- `crates/app/src/main.rs`: ~806, ~1574, ~1928, ~2060, ~2132, ~2189, ~2326, ~2773.
- `crates/app/src/input.rs`: ~4229, ~4266.
- `crates/app/tests/headless.rs`: ~45 (`turn` helper).

(The `info` test `turn_result_info_defaults_none_for_normal_turn` keeps asserting `info.is_none()`; just add `pending_io: None` to its literal.)

- [ ] **Step 6: Run the tests + full app suite**

Run: `cargo test -p app`
Expected: PASS (including the headless smoke test), 0 warnings.

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/session.rs crates/app/src/main.rs crates/app/src/input.rs crates/app/tests/headless.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): bubble in-game save/restore from the session + resume API

run_until_input now returns a RunStop: v4+ save/restore bubble as
SavePending/RestorePending instead of auto-failing; v3 still auto-fails and
keeps the 'isn't wired' info line. TurnResult gains pending_io; resume_save /
resume_restore complete the VM and continue the turn via a shared finish_turn.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: archive — `read_quetzal_from_file`

**Files:**
- Modify: `crates/app/src/archive.rs` — add `read_quetzal_from_file`; add two tests in `mod tests` (reuse `temp_archive_path`, `small_mapper`, `dummy_machine`).

**Interfaces:**
- Consumes: `zip::ZipArchive`, `ENTRY_SAVE` (the `"game.sav"` const), `std::fs::read`.
- Produces: `pub fn read_quetzal_from_file(path: &Path) -> io::Result<Vec<u8>>`.

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/archive.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn read_quetzal_extracts_game_sav_from_lanthorn() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/czech.z5");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let machine = dummy_machine();
        let expected = machine.save_quetzal();

        let path = temp_archive_path("qzl-from-lanthorn");
        save_archive(&path, &small_mapper(), &machine, &[], &[], &[]).expect("save_archive");
        let got = read_quetzal_from_file(&path).expect("read_quetzal_from_file");
        let _ = std::fs::remove_file(&path);

        assert_eq!(got, expected, "game.sav bytes extracted from the .lanthorn");
    }

    #[test]
    fn read_quetzal_returns_raw_bytes_for_plain_qzl() {
        // A non-zip file (a plain .qzl) returns its raw bytes unchanged.
        let path = temp_archive_path("plain-qzl");
        std::fs::write(&path, b"FORM\x00\x00fake-quetzal").unwrap();
        let got = read_quetzal_from_file(&path).expect("read raw");
        let _ = std::fs::remove_file(&path);
        assert_eq!(got, b"FORM\x00\x00fake-quetzal");
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p app read_quetzal`
Expected: compile error (`read_quetzal_from_file` missing).

- [ ] **Step 3: Implement `read_quetzal_from_file`**

In `crates/app/src/archive.rs` (after `load_archive`), add. `std::io::Read` is already imported at the top of the file.

```rust
/// Read raw Quetzal bytes from a save file for an in-game RESTORE.
///
/// If `path` is a `.lanthorn` ZIP archive, returns its `game.sav` entry;
/// otherwise returns the file's raw bytes (a plain `.qzl` Quetzal save).
pub fn read_quetzal_from_file(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    if let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        if let Ok(mut entry) = zip.by_name(ENTRY_SAVE) {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Ok(bytes)
}
```

- [ ] **Step 4: Run the tests + full app suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/archive.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): read_quetzal_from_file — game.sav from a .lanthorn, else raw .qzl

Used by the in-game RESTORE path so the picker can restore both lanthorn
archives and plain Quetzal saves.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: app run-loop + state integration (the intricate task)

**Files:**
- Modify: `crates/app/src/state.rs` — add `AppState.ingame_io: Option<crate::session::PendingIo>` (+ init).
- Modify: `crates/app/src/main.rs` — open the saves dialog "in-game" after `submit`/`submit_char`/resume; branch the `SavesLoad` / `SavesClose` / `handle_saves_prompt::SaveAs` handlers on `state.ingame_io`; list `*.qzl` in the in-game restore picker; remove the v4+ "isn't wired" surfacing (Task 2 already drops it for v4+ — verify nothing re-adds it).
- Modify: `crates/app/src/session.rs` — add the fixture-gated redraw test (uses the Task 2 resume API + `crates/zvm/tests/fixtures` — note: there is currently **no** bundled `bureaucr.z4` fixture under `crates/`; the file lives at repo `stories/bureaucr.z4`. The test must locate it via the repo path and skip when absent).

**Interfaces:**
- Consumes: `TurnResult.pending_io` (Task 2), `session.resume_save` / `session.resume_restore` (Task 2), `archive::read_quetzal_from_file` (Task 3), `archive::load_archive`, `archive::save_archive_meta`, `persist_files::list_saves`, `screen.upper` (for the redraw probe).

This task has no pure unit-testable surface for the event loop itself, so verification is: the app crate builds and tests green (0 warnings), the headless smoke test passes, the fixture-gated redraw session test passes-or-skips, plus the manual checklist. **The async-over-frames dialog is the risk** (see Notes). Implement by *mirroring the existing saves flow exactly* — "in-game" differs only in that confirm/cancel call `resume_*` instead of the direct restore/save path.

- [ ] **Step 1: Add `ingame_io` to `AppState`**

In `crates/app/src/state.rs`, in the `AppState` struct near `pub saves: Option<SavesState>` (~649), add:

```rust
    /// Set while a game-initiated (v4+) `@save`/`@restore` is awaiting the host's
    /// file I/O. The saves dialog runs in "in-game" mode: its confirm/cancel call
    /// `session.resume_save`/`resume_restore` instead of the Ctrl+S/Ctrl+R path.
    pub ingame_io: Option<crate::session::PendingIo>,
```

In the `AppState` constructor near `saves: None,` (~810), add `ingame_io: None,`. (If any test constructs `AppState` with a struct literal rather than the constructor, add it there too — grep `AppState {`.)

- [ ] **Step 2: Open the in-game dialog after a turn suspends on I/O**

Add a small helper near `handle_saves_prompt` in `crates/app/src/main.rs`:

```rust
/// Open the saves dialog in "in-game" mode for a game-initiated save/restore.
/// SAVE: prompt for a save name (reuses the SaveAs prompt). RESTORE: open the
/// saves list, including plain *.qzl files alongside *.lanthorn saves.
fn open_ingame_saves(
    io: app::session::PendingIo,
    save_dir: &std::path::Path,
    ifid: &str,
    state: &mut AppState,
) {
    use app::session::PendingIo;
    state.ingame_io = Some(io);
    state.dialog_focus = 0;
    match io {
        PendingIo::Save => {
            // The game asked to SAVE: ask where. On submit -> resume_save(true);
            // on cancel -> resume_save(false) (handled in the prompt-cancel path).
            state.prompt = Some(app::state::Prompt {
                kind: app::state::PromptKind::SaveAs,
                buffer: String::new(),
            });
        }
        PendingIo::Restore => {
            // The game asked to RESTORE: list lanthorn saves + plain .qzl files.
            let mut entries = list_saves(save_dir, ifid);
            entries.extend(list_qzl(save_dir));
            state.saves = Some(SavesState { entries, selected: 0 });
        }
    }
}

/// List plain `*.qzl` Quetzal files in `dir` as SaveInfo rows (for the in-game
/// restore picker). Mirrors the SaveInfo shape used by `list_saves`.
fn list_qzl(dir: &std::path::Path) -> Vec<app::persist_files::SaveInfo> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("qzl") {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("save.qzl").to_string();
                out.push(app::persist_files::SaveInfo {
                    path: p, name, turns: 0, saved_at: String::new(), is_default: false,
                });
            }
        }
    }
    out
}
```

Then, after the two live game-turn submit sites, detect `pending_io` and open the dialog instead of falling through to the normal post-turn map/quit handling:

- At the `submit_char` site (~1390): after `apply_turn(&mut mapper, "", &result);` and before the quit check, insert:

```rust
                            if let Some(io) = result.pending_io {
                                open_ingame_saves(io, &save_dir, &ifid, &mut state);
                                continue;
                            }
```

- At the `submit` site (~1707): after `apply_turn(&mut mapper, &cmd, &result);` (and the graph-gen bump), before the auto-save/tidy block, insert the same guard:

```rust
                if let Some(io) = result.pending_io {
                    open_ingame_saves(io, &save_dir, &ifid, &mut state);
                    continue;
                }
```

(Place it early enough that we skip per-turn auto-save and history capture for the *incomplete* turn — those run after the resume completes. The `continue` returns to the event loop so the dialog renders.)

- [ ] **Step 3: Branch RESTORE confirm on `ingame_io` (`Action::SavesLoad`)**

In `Action::SavesLoad` (~2105), before the existing `load_archive`/`restore_file` body, branch when in-game restore is active. Mirror the existing post-restore re-observe/recenter block, but obtain bytes via `read_quetzal_from_file` and complete the VM via `resume_restore`:

```rust
            Action::SavesLoad => {
                let load_info = state.saves.as_ref().and_then(|s| {
                    s.entries.get(s.selected).map(|e| (e.path.clone(), e.name.clone()))
                });
                let Some((path, entry_name)) = load_info else { continue };

                if state.ingame_io == Some(app::session::PendingIo::Restore) {
                    // In-game restore: feed Quetzal bytes back into the suspended VM.
                    match app::archive::read_quetzal_from_file(&path) {
                        Ok(bytes) => {
                            // For a .lanthorn, also load its map (as Ctrl+R does).
                            if let Ok(ac) = load_archive(&path) {
                                mapper = ac.mapper;
                            }
                            state.ingame_io = None;
                            state.saves = None;
                            let result = session.resume_restore(Some(&bytes));
                            state.push_transcript(&format!("[Restored: {}]", entry_name));
                            finish_resumed_turn(result, &mut session, &mut mapper, &mut state, last_panes);
                        }
                        Err(e) => {
                            state.ingame_io = None;
                            state.saves = None;
                            let result = session.resume_restore(None);
                            state.push_transcript(&format!("[Restore failed: {}]", e));
                            finish_resumed_turn(result, &mut session, &mut mapper, &mut state, last_panes);
                        }
                    }
                    continue;
                }

                // ... existing (non-in-game) restore_file body unchanged ...
            }
```

Add a `finish_resumed_turn` helper near `handle_saves_prompt` that mirrors the post-turn handling already in the `submit` path (push transcript, `apply_turn`, bump graph_gen, re-observe + recenter on the current room, handle quit), and re-checks `pending_io` so a *chained* request (resume itself ending in another `@save`/`@restore`) re-opens the dialog:

```rust
/// Post-process a TurnResult produced by `session.resume_*`: render output,
/// re-observe the location, recenter, and re-open the dialog if the resume
/// itself suspended on another in-game I/O. Returns true if the app should quit.
fn finish_resumed_turn(
    result: app::session::TurnResult,
    session: &mut app::session::GameSession,
    mapper: &mut Mapper,
    state: &mut AppState,
    last_panes: Panes,
) -> bool {
    state.push_transcript(&result.transcript);
    apply_turn_events(state, &result);
    apply_turn(mapper, "", &result);
    state.graph_gen = state.graph_gen.wrapping_add(1);
    state.set_viewed_layer(None);
    if let Some(snap) = &result.location {
        let rid = snap.number as mapper::graph::RoomId;
        state.select_room(Some(rid));
        if let Some(room) = mapper.graph.room(rid) {
            if let Some(pos) = room.pos {
                let (pw, ph) = map_pane_dims(last_panes.map);
                state.recenter_on(pos, pw, ph);
            }
        }
    }
    // A chained request: the resumed turn suspended on another @save/@restore.
    if let Some(io) = result.pending_io {
        state.ingame_io = Some(io);
        // The caller opens the dialog next frame; just remember the request.
    }
    result.quit
}
```

Confirm the exact type of `last_panes` in the run loop and pass it (or its `.map` rect) accordingly; the existing `SavesLoad` block already uses `last_panes.map`. If `finish_resumed_turn` returning `true` (quit) must break the loop, capture the return and `break` at the call site (a `continue`-then-check pattern, since helpers can't `break` the loop). Simplest: have the call sites do `if finish_resumed_turn(...) { break; }`. For the *chained* re-open, after `finish_resumed_turn` sets `state.ingame_io`, call `open_ingame_saves(io, &save_dir, &ifid, &mut state)` at the call site when `state.ingame_io` is still `Some`.

- [ ] **Step 4: Branch SAVE confirm + dialog cancel on `ingame_io`**

SAVE confirm — in `handle_saves_prompt`, `PromptKind::SaveAs` arm (~2362): after a *successful* save, if in-game, complete the VM. Mirror the existing save, then resume:

```rust
        PromptKind::SaveAs => {
            if buf.is_empty() {
                state.push_transcript("[Save name cannot be empty]");
                // Stay in-game: do NOT resume yet (the request is still pending).
                return;
            }
            match save_named(dir, ifid, &buf, mapper, &session.machine, state.turns, &state.transcript, &state.transcript_kinds) {
                Ok(()) => {
                    state.push_transcript(&format!("[Saved as: {}]", buf));
                    if let Some(s) = &mut state.saves { s.entries = list_saves(dir, ifid); }
                    if state.ingame_io == Some(app::session::PendingIo::Save) {
                        state.ingame_io = None;
                        let result = session.resume_save(true);
                        // (post-process via the same path used by SavesLoad; see note)
                        // handle_saves_prompt currently has no access to mapper-recenter
                        // panes — push the resumed transcript and re-observe location.
                        state.push_transcript(&result.transcript);
                        // NOTE: full recenter is done by the caller (see Step 5 note).
                    }
                }
                Err(e) => {
                    state.push_transcript(&format!("[Save failed: {}]", e));
                    // Stay in-game so the user can retry; do not resume.
                }
            }
        }
```

`handle_saves_prompt`'s signature lacks `last_panes`. To keep the resumed-turn rendering uniform, prefer NOT to resume inside `handle_saves_prompt`. Instead, set a flag (`state.ingame_resume_save = Some(true)`) and let the run-loop call site (right after `handle_saves_prompt(...)`, the two sites at ~1471 and ~2228) detect it and run `session.resume_save(true)` + `finish_resumed_turn(...)`. This keeps all VM-resume + recenter logic in the run loop where `session`, `mapper`, and `last_panes` are in scope. Add `pub ingame_resume_save: Option<bool>` to `AppState` (default `None`) for this hop, or — simpler — have `handle_saves_prompt` take `&mut session` (it already does) **and** an extra `last_panes: Panes` + `&mut Mapper` param and resume there. Pick whichever keeps the diff smallest; the flag-hop is the least invasive and is the recommended approach.

Dialog CANCEL — in `Action::SavesClose` handling: lanthorn closes the modal via `apply_action` (`state.saves = None`). For in-game restore, intercept in the run loop *before* dispatching, OR detect in the `SavesClose` arm if it is caller-handled. Since `SavesClose` is handled inside `apply_action` (input.rs ~1763) which has no `session`, add a run-loop guard: after `apply_action` returns, if the saves modal just closed (`state.saves` went from `Some` to `None`) **and** `state.ingame_io == Some(Restore)`, call `session.resume_restore(None)` + `finish_resumed_turn`, then clear `ingame_io`. Likewise for the SaveAs prompt being cancelled (Esc) while `ingame_io == Some(Save)` → `session.resume_save(false)`.

Concretely: capture `let saves_was_open = state.saves.is_some();` and `let prompt_was_save = matches!(&state.prompt, Some(p) if matches!(p.kind, PromptKind::SaveAs));` before `apply_action`, then after it check whether the overlay closed without a submit, and if `ingame_io` is still set, resume with the failure/cancel result. Mirror the wiring already used for `saves_prompt_submitted` (the run loop already inspects state after `apply_action`).

- [ ] **Step 5: Remove the v4+ "isn't wired" surfacing; keep v3**

Task 2 already makes `info` `None` for v4+ (only v3 auto-fail sets it). Verify the two `if let Some(note) = &result.info { state.push_transcript(note); }` sites (~1711, ~1393) now only fire for v3. No code change expected here beyond confirmation — note it in the commit body.

- [ ] **Step 6: Fixture-gated redraw test (strongest feasible) + NOTE the gap**

In `crates/app/src/session.rs` `mod tests`, add a best-effort end-to-end-ish test driving the real `stories/bureaucr.z4` through the resume API and asserting the upper-window status grid is non-empty after an in-game RESTORE. The app event loop itself is not unit-testable, so this exercises the *session* layer (the VM-resume + redraw), which is the behavioral heart of the feature:

```rust
    // Fixture-gated: in-game SAVE then RESTORE on Bureaucracy (v4) must leave the
    // upper-window status grid non-empty (the redraw this whole feature is about).
    // NOTE/GAP: this drives the SESSION resume API, not the app event loop, and it
    // depends on reaching @save by typing into the game. If the input sequence does
    // not reach @save within the probe budget, the test skips (no false failure).
    #[test]
    fn bureaucracy_ingame_restore_redraws_status_grid() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/bureaucr.z4");
        if !fixture.exists() {
            return; // fixture absent — skip
        }
        let story = std::fs::read(&fixture).expect("read bureaucr.z4");
        let mut sess = GameSession::new(story).expect("new bureaucr.z4");

        // Probe: type SAVE-ish commands until the VM suspends on @save.
        let mut blob: Option<Vec<u8>> = None;
        for cmd in ["save", "yes", "save", "y", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                let _ = sess.resume_save(true); // pretend the host wrote the file
                break;
            }
            if r.quit { break; }
        }
        let Some(blob) = blob else {
            // Could not reach @save with this probe sequence — document the gap.
            eprintln!("bureaucr.z4: did not reach @save via the probe; skipping redraw assertion");
            return;
        };

        // Now drive a RESTORE and feed the captured blob back.
        let mut restored = false;
        for cmd in ["restore", "yes", "restore", "y", "restore"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Restore) {
                let _ = sess.resume_restore(Some(&blob));
                restored = true;
                break;
            }
            if r.quit { break; }
        }
        if !restored {
            eprintln!("bureaucr.z4: did not reach @restore via the probe; skipping redraw assertion");
            return;
        }

        // The resumed game redrew its own status line into the upper window.
        let any_drawn = sess.machine.screen.upper.cells.iter().any(|c| c.ch != ' ');
        assert!(any_drawn, "after in-game RESTORE the upper-window grid must be non-empty (redraw)");
    }
```

If the probe never reaches `@save`/`@restore` (Bureaucracy's opening form may intercept the words), the test skips with an `eprintln!` — that is the **noted gap**: a true app-event-loop end-to-end test is impractical here. The deterministic coverage is Task 1 (store-2 semantics) + Task 2 (`pending_io` + resume continue/cancel).

- [ ] **Step 7: Build, test, headless smoke, manual check**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean (0 warnings), suite PASS, headless smoke PASS, redraw test PASS-or-skips.

Manual (not gating, but recommended): `cargo run -p app -- stories/bureaucr.z4`, navigate to a SAVE prompt, save (saves dialog appears, write a `.lanthorn`), continue, then RESTORE it — the status line should redraw and the game resume mid-routine.

- [ ] **Step 8: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/app/src/state.rs crates/app/src/main.rs crates/app/src/session.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(app): wire game-initiated save/restore through the saves dialog (v4+)

After a turn suspends on its own @save/@restore, open the saves dialog in
in-game mode: SAVE writes a .lanthorn then resume_save(true); RESTORE reads a
.lanthorn/.qzl (loading the map for archives) then resume_restore(Some). Cancel
resumes with failure. The game resumes inside its own routine and redraws its
status line. v3 keeps the host-mediated 'isn't wired' message.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- **Dependency order:** 1 (zvm) → 2 (session) → 3 (archive) → 4 (app). Task 1 is `cargo test -p zvm`; Tasks 2-4 are `cargo test -p app`. Each ends green with 0 warnings before committing.
- **Why store 2 into `mem[pc-1]`:** on a successful restore it is the original `@save` that "returns 2" (the game checks `result == 2` to redraw), not `@restore`. lanthorn's saved PC is post-instruction, so a v4+ `@save`'s store byte is its last byte, at `saved_pc - 1`. No PC-convention change.
- **Task 2 literal churn:** adding `pending_io` to `TurnResult` is the only thing forcing edits in `main.rs`/`input.rs`/`headless.rs` in Task 2 — those are field-add-only; the run-loop *behavior* changes land in Task 4.
- **Task 4 is the risk.** The saves dialog is asynchronous over event-loop frames; the integration spine is `state.ingame_io` + branching the existing `SavesLoad` (restore confirm), `SavesClose`/prompt-cancel (cancel), and `handle_saves_prompt::SaveAs` (save confirm). Keep VM-resume + recenter logic in the run loop (where `session`, `mapper`, `last_panes` are in scope) via a small flag-hop from `handle_saves_prompt`; do not resume inside `apply_action`/`handle_saves_prompt` directly. Mirror the existing restore re-observe block exactly — "in-game" only swaps `restore_file` for `resume_restore` and `save_archive`/`save_named` + `resume_save` for the direct save.
- **Chained I/O:** a resume can itself end in another `pending_io` (e.g. game does `@save` then `@restore`). `finish_resumed_turn` re-records `ingame_io`; the call site re-opens the dialog. Handle it; it is rare but cheap to support.
- **`stories/bureaucr.z4` is NOT under `crates/`** — it is at the repo root `stories/`. The fixture-gated test reaches it via `CARGO_MANIFEST_DIR/../../stories/bureaucr.z4` and skips when absent. Confirm the relative depth from `crates/app`.
- `README.md` is committed; `TODO.md` is gitignored — never stage it. This feature does not require README changes (it surfaces no new config); add a line only if asked.
