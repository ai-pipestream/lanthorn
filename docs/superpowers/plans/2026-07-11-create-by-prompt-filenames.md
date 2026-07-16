# Glk `create_by_prompt` filenames (Option 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route Glk `glk_fileref_create_by_prompt` through a host filename prompt (write) / picker (read) instead of a fixed per-usage name, so prompted files get unique, player-chosen names — while their bytes stay in the VFS and persist per-story via the existing SQ-0278 sidecar.

**Architecture:** Mirror the existing `@save`/`@restore` suspend/resume path. `create_by_prompt` becomes a VM suspension (`StepResult::NeedFilename`); a `supply_filename(Option<String>)` resume binds the fileref (or NULL on cancel). The app surfaces the request through the `Engine` trait and opens a name-entry prompt (write modes) or a VFS-file picker (read mode), reusing the in-game save/restore run-loop plumbing. Bytes remain in `Model::files` and auto-persist unchanged.

**Tech Stack:** Rust workspace — `gvm` (Glulx VM, zero external deps), `gvm-cli` (headless host), `app` (ratatui TUI).

## Global Constraints

- **Branch off `main`**, branch name `create-by-prompt-filenames`. Subagent-driven, review between tasks.
- Commit trailers on **every** commit:
  - `Quest: SQ-0279`
  - `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`
  - `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- **`gvm` stays zero-external-dep** (std only). The VM never touches the filesystem — bytes flow to the host, never to disk from inside `gvm`.
- **Staging hygiene:** the tree has pre-existing untracked files (`docs/mapping-*.md`, `docs/superpowers/plans/2026-07-*.md`, `tests/`, `ui.txt`, `stories/`). Stage ONLY the edited source files by path — never `git add -A`. This plan file itself stays untracked (matches the other plan files).
- **Filemode values** (Glk, raw literals as used in `glk.rs`): Write `0x01`, Read `0x02`, ReadWrite `0x03`, WriteAppend `0x05`. "Read mode" for the picker-vs-prompt decision = `fmode == 0x02` exactly.
- **Styleable UI:** any rendered dialog must use existing themed `ColorScheme` fields (e.g. `state.colors.dialog`, `state.colors.scrollbar`) — no hard-coded colours. Reusing existing selectors is fine; this plan adds no new selectors.
- **Shared-chrome dialog conventions:** Esc cancels, Enter activates/submits, Up/Down move a selection.

---

## Task 1 — gvm: `create_by_prompt` suspend + `supply_filename` + `file_names`

**Files:**
- Modify: `crates/gvm/src/exec.rs` (StepResult, PendingFileref, op_glk, 0x0062 arm, step(), supply_filename, file_names delegate)
- Modify: `crates/gvm/src/glk.rs` (`Model::file_names`)
- Modify: `crates/gvm/tests/accel_story_equivalence.rs`, `crates/gvm/tests/kerkerkruip_boots.rs` (exhaustive-match arms)
- Test: inline in `crates/gvm/src/exec.rs` (`#[cfg(test)]`) and `crates/gvm/src/glk.rs`

**Interfaces:**
- Produces:
  - `StepResult::NeedFilename { usage: u32, fmode: u32 }` (new StepResult variant; keeps `#[derive(Copy)]` — both fields are `u32`).
  - `Machine::supply_filename(&mut self, name: Option<String>)` — resume: `Some(name)` binds a fileref (via `Model::fileref_create`, which sanitizes) and stores its id into the suspended `@glk`'s S1; `None` stores `0` (NULL fileref).
  - `Machine::file_names(&self) -> Vec<String>` — user-visible VFS filenames for a read picker.
  - `Model::file_names(&self) -> Vec<String>`.

- [ ] **Step 1: Write the failing tests**

In `crates/gvm/src/glk.rs`, in the existing `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn file_names_lists_user_files_hiding_internal_keys() {
    let mut m = Model::new();
    m.files.insert("story-notes".to_string(), vec![1, 2, 3]);
    m.files.insert("__temp_0__".to_string(), vec![4]);
    m.files.insert("__prompt_2__".to_string(), vec![5]);
    let names = m.file_names();
    assert_eq!(names, vec!["story-notes".to_string()], "temp/legacy-prompt keys are hidden");
}
```

In `crates/gvm/src/exec.rs`, in the `#[cfg(test)] mod tests` (near the other `glk_fileref_*` tests, which use `glk_call`/`machine`/`asm`), add:

```rust
#[test]
fn glk_fileref_create_by_prompt_suspends_then_supply_binds_name() {
    use asm::Op::{C8, Mem16};
    // create_by_prompt(usage=0x00, fmode=0x01 Write, rock=0) -> store fileref id @ 0x0100.
    let mut body = glk_call(0x62, &[C8(0x00), C8(0x01), C8(0x00)], Mem16(0x0100));
    body.extend(asm::ins(0x120, &[])); // quit (runs only after the prompt resumes)
    let start = asm::func(0xC1, &[], &body);
    let built = asm::assemble(&[start], 0, 0x200);
    let mut m = machine(built);
    // Drive to the suspension.
    let mut sr = m.step();
    while sr == StepResult::Continue {
        sr = m.step();
    }
    assert_eq!(sr, StepResult::NeedFilename { usage: 0x00, fmode: 0x01 });
    // Re-reported while pending (idempotent, like @save/@restore).
    assert_eq!(m.step(), StepResult::NeedFilename { usage: 0x00, fmode: 0x01 });
    // Player names it; execution resumes and runs to quit.
    m.supply_filename(Some("mydata".to_string()));
    let mut sr = m.step();
    while sr == StepResult::Continue {
        sr = m.step();
    }
    assert_eq!(sr, StepResult::Quit);
    assert_ne!(m.mem.read32(0x100).unwrap(), 0, "supply_filename bound a live (non-NULL) fileref");
}

#[test]
fn glk_fileref_create_by_prompt_cancel_stores_null() {
    use asm::Op::{C8, Mem16};
    let mut body = glk_call(0x62, &[C8(0x00), C8(0x01), C8(0x00)], Mem16(0x0100));
    body.extend(asm::ins(0x120, &[]));
    let start = asm::func(0xC1, &[], &body);
    let built = asm::assemble(&[start], 0, 0x200);
    let mut m = machine(built);
    let mut sr = m.step();
    while sr == StepResult::Continue {
        sr = m.step();
    }
    assert_eq!(sr, StepResult::NeedFilename { usage: 0x00, fmode: 0x01 });
    m.supply_filename(None); // player cancelled
    let mut sr = m.step();
    while sr == StepResult::Continue {
        sr = m.step();
    }
    assert_eq!(sr, StepResult::Quit);
    assert_eq!(m.mem.read32(0x100).unwrap(), 0, "cancel -> NULL fileref (0)");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p gvm file_names_lists_user_files_hiding_internal_keys glk_fileref_create_by_prompt`
Expected: FAIL to compile — `NeedFilename`, `supply_filename`, `file_names` do not exist yet.

- [ ] **Step 3: Implement the gvm changes**

**3a. `crates/gvm/src/glk.rs`** — add `Model::file_names` near `fileref_create`/`fileref_create_by_prompt` (~line 1030):

```rust
/// The user-visible filenames in the VFS: every written file except the internal
/// temp (`__temp_`) and legacy prompt (`__prompt_`) keys. BTreeMap order (sorted).
/// Feeds the host's `create_by_prompt` read picker.
pub fn file_names(&self) -> Vec<String> {
    self.files
        .keys()
        .filter(|k| !k.starts_with("__temp_") && !k.starts_with("__prompt_"))
        .cloned()
        .collect()
}
```

**3b. `crates/gvm/src/exec.rs` — `StepResult`** (add a variant after `RestoreRequest`, ~line 47):

```rust
    /// The game executed `glk_fileref_create_by_prompt`: the host should prompt for
    /// a filename (write modes) or let the player pick an existing VFS file (read
    /// mode), then call [`Machine::supply_filename`]. `usage` is the Glk fileusage,
    /// `fmode` the Glk filemode.
    NeedFilename {
        /// The Glk fileusage (Data / SavedGame / Transcript / InputRecord + flags).
        usage: u32,
        /// The Glk filemode (Read `0x02`, Write `0x01`, ReadWrite `0x03`, WriteAppend `0x05`).
        fmode: u32,
    },
```

**3c. `crates/gvm/src/exec.rs` — `PendingFileref` struct** (add next to `PendingSaveLoad`, ~line 98):

```rust
/// A suspended `glk_fileref_create_by_prompt` awaiting the host's chosen filename.
/// `dest` (the `@glk` store operand) is filled by `op_glk` right after the arm sets
/// this; `supply_filename` binds the fileref and stores its id there.
struct PendingFileref {
    dest: Dest,
    usage: u32,
    fmode: u32,
    rock: u32,
}
```

**3d. `crates/gvm/src/exec.rs` — field on `Machine`** (next to `pending_saveload`, ~line 152):

```rust
    /// A suspended `glk_fileref_create_by_prompt` (see [`StepResult::NeedFilename`]).
    /// Set by the `0x0062` @glk arm; consumed by [`Machine::supply_filename`].
    pending_fileref: Option<PendingFileref>,
```

Initialize `pending_fileref: None,` in the constructor, next to `pending_saveload: None,` (~line 305).

**3e. `crates/gvm/src/exec.rs` — the `0x0062` @glk arm** (currently `0x0062 => self.glk.fileref_create_by_prompt(a(0), a(1), a(2)),`, ~line 2973). Replace with:

```rust
            0x0062 => {
                // glk_fileref_create_by_prompt(usage, fmode, rock): no synchronous
                // name. Suspend so the host can prompt (write) or pick (read);
                // supply_filename() resumes with the choice. op_glk fills in `dest`
                // (this @glk's S1) and returns the suspend instead of storing.
                self.pending_fileref =
                    Some(PendingFileref { dest: Dest::Discard, usage: a(0), fmode: a(1), rock: a(2) });
                0 // placeholder; not stored (op_glk suspends instead)
            }
```

**3f. `crates/gvm/src/exec.rs` — `op_glk`** (~line 2481). After the `glk_dispatch` line, before the `store`:

```rust
        let result = self.glk_dispatch(selector, &args)?;
        // glk_fileref_create_by_prompt suspends: the real fileref id is produced
        // later by supply_filename, which stores it into this @glk's S1. Capture the
        // destination and do not store the placeholder result now.
        if let Some(pf) = self.pending_fileref.as_mut() {
            pf.dest = s[0];
            return Ok(());
        }
        self.store(s[0], result)
```

**3g. `crates/gvm/src/exec.rs` — `fileref_prompt_result` + wire into `step()`.** Add the helper next to `saveload_result` (~line 3333):

```rust
    /// The [`StepResult`] for a suspended `glk_fileref_create_by_prompt`, if any.
    /// The host resolves it via [`Machine::supply_filename`].
    fn fileref_prompt_result(&self) -> Option<StepResult> {
        self.pending_fileref
            .as_ref()
            .map(|p| StepResult::NeedFilename { usage: p.usage, fmode: p.fmode })
    }
```

In `step()` (~line 3647), add a re-report at the top, after the `saveload_result` check:

```rust
        // Still suspended on a prior create_by_prompt: re-report until supplied.
        if let Some(sr) = self.fileref_prompt_result() {
            return sr;
        }
```

and extend the post-`step_once` chain:

```rust
            Ok(()) => self
                .suspend_result()
                .or_else(|| self.saveload_result())
                .or_else(|| self.fileref_prompt_result())
                .unwrap_or(StepResult::Continue),
```

**3h. `crates/gvm/src/exec.rs` — resume method + accessor.** Add next to `complete_restore_failure` (~line 3390):

```rust
    /// Complete a suspended `glk_fileref_create_by_prompt`. `Some(name)` binds a
    /// fileref to that (sanitized) name and stores its id into the `@glk`'s S1;
    /// `None` (the player cancelled) stores 0 (the Glk NULL fileref). No-op if none
    /// pending.
    pub fn supply_filename(&mut self, name: Option<String>) {
        if let Some(p) = self.pending_fileref.take() {
            let id = match name {
                Some(n) => self.glk.fileref_create(p.usage, n, p.rock),
                None => 0,
            };
            let _ = self.store(p.dest, id);
        }
    }

    /// The user-visible VFS filenames (for a host `create_by_prompt` read picker).
    pub fn file_names(&self) -> Vec<String> {
        self.glk.file_names()
    }
```

- [ ] **Step 4: Fix the two gvm test-file exhaustive matches**

`crates/gvm/tests/accel_story_equivalence.rs` (~line 72) and `crates/gvm/tests/kerkerkruip_boots.rs` (~line 86) each have a `match machine.step()` whose `StepResult::SaveRequest | StepResult::RestoreRequest => { ... }` arm handles unexpected suspensions. Read each arm and add `StepResult::NeedFilename { .. }` to that same arm (a `create_by_prompt` during these headless equivalence/boot drives is treated the same way as an unexpected save/restore). Do not otherwise change the tests.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p gvm`
Expected: PASS — the two new tests pass, `file_names` test passes, and the whole `gvm` suite (incl. the two integration test files) still passes.

- [ ] **Step 6: Commit**

```bash
git add crates/gvm/src/exec.rs crates/gvm/src/glk.rs crates/gvm/tests/accel_story_equivalence.rs crates/gvm/tests/kerkerkruip_boots.rs
git commit -F <msg-file>   # use -F, not -m (trailers + backticks); include the three trailers
```
Message subject: `feat(gvm): suspend create_by_prompt for a host-chosen filename (SQ-0279)`

---

## Task 2 — gvm-cli: service `NeedFilename` in the drive loop

**Files:**
- Modify: `crates/gvm-cli/src/main.rs` (`drive()` StepResult match, ~line 139)

**Interfaces:**
- Consumes: `StepResult::NeedFilename`, `Machine::supply_filename` (Task 1).

- [ ] **Step 1: Add the match arm**

In `drive()` (the `match machine.step()`), add a `NeedFilename` arm alongside `NeedLine`/`NeedChar`. Flush pending game output first (mirror the `before_input(machine)` call the input arms use), print a prompt to stderr, read one line from stdin, and supply it (blank = cancel):

```rust
            StepResult::NeedFilename { .. } => {
                before_input(machine);
                eprint!("Filename (blank to cancel): ");
                let _ = std::io::stderr().flush();
                let (line, _) = read_line();
                let name = line.trim_end_matches(['\n', '\r']);
                machine.supply_filename(if name.is_empty() { None } else { Some(name.to_string()) });
            }
```

If `std::io::Write` (for `flush`) is not already in scope in this file, add the import. Match the exact signature of the existing `read_line()` helper (the `NeedLine` arm shows it returns `(line, terminator)`); adapt if it differs.

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p gvm-cli`
Expected: builds cleanly (the exhaustive `StepResult` match is now complete).

Run: `cargo test -p gvm-cli`
Expected: existing tests (if any) still pass.

- [ ] **Step 3: Commit**

```bash
git add crates/gvm-cli/src/main.rs
git commit -F <msg-file>
```
Message subject: `feat(gvm-cli): prompt for a filename on create_by_prompt (SQ-0279)`

---

## Task 3 — app: Engine plumbing (`resume_filename`, `file_names`, `pending_filename`) + Glulx wiring

**Files:**
- Modify: `crates/app/src/session.rs` (define `FilenameReq` near `PendingIo`, ~line 40)
- Modify: `crates/app/src/engine.rs` (import `FilenameReq`; add three `Engine` methods with defaults)
- Modify: `crates/app/src/glulx_session.rs` (`DriveStop::Filename`, drive/drive_settled/drive_turn arms, `pending_filename` field, `resume_filename`, `file_names`, `pending_filename()` overrides)
- Test: inline in `crates/app/src/glulx_session.rs`

**Interfaces:**
- Consumes: `StepResult::NeedFilename`, `Machine::supply_filename`, `Machine::file_names` (Task 1).
- Produces:
  - `session::FilenameReq { usage: u32, fmode: u32 }` (`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`).
  - `Engine::pending_filename(&self) -> Option<FilenameReq>` (default `None`).
  - `Engine::file_names(&self) -> Vec<String>` (default `Vec::new()`).
  - `Engine::resume_filename(&mut self, name: Option<String>) -> TurnResult` (default `unreachable!` — only Glulx produces filename requests).

- [ ] **Step 1: Write the failing test**

In `crates/app/src/glulx_session.rs` `#[cfg(test)] mod tests` (near `ingame_save_restore_round_trips_through_the_engine`, which shows `enc`/`open_buffer_prelude`/`line_prompt`/`image_for`/`E::*`), add:

```rust
#[test]
fn ingame_create_by_prompt_bubbles_filename_request_and_resume_continues() {
    use E::*;
    const FREF_RES: u32 = 0x410;
    let mut body = open_buffer_prelude();
    body.extend(line_prompt()); // turn 1 prompt
    // glk_fileref_create_by_prompt(usage=0, fmode=1 Write, rock=0) -> mem[FREF_RES].
    // @glk pops args with arg[0] topmost, so push rock, then fmode, then usage.
    body.extend(enc(0x40, &[Imm(0), Push])); // rock
    body.extend(enc(0x40, &[Imm(1), Push])); // fmode = Write
    body.extend(enc(0x40, &[Imm(0), Push])); // usage (topmost = arg[0])
    body.extend(enc(0x130, &[Imm(0x62), Imm(3), MemLoad(FREF_RES)]));
    body.extend(line_prompt()); // resume point after supply_filename
    body.extend(enc(0x120, &[])); // quit
    let mut sess = GlulxSession::new(image_for(body, 1), 80, 24, true, false, false, (1, 1), None).expect("new");
    assert_eq!(sess.pending_input(), InputKind::Line, "opens at the turn-1 prompt");

    // The command drives into create_by_prompt, which bubbles a filename request.
    let r1 = sess.submit("script");
    assert_eq!(sess.pending_filename(), Some(FilenameReq { usage: 0, fmode: 1 }));
    assert!(r1.pending_io.is_none(), "a filename request is not a save/restore");
    assert!(!r1.quit);

    // The host supplies a name; execution resumes to the next prompt (proving the
    // @glk stored a value and did not fault or wedge).
    let r2 = sess.resume_filename(Some("transcript".to_string()));
    assert_eq!(sess.pending_filename(), None, "request cleared after supply");
    assert!(!r2.quit);
    assert_eq!(sess.pending_input(), InputKind::Line, "resumes at the turn-2 prompt");
}
```

Import `FilenameReq` in the test module (`use crate::session::FilenameReq;` or via the existing `super::*` / session import — match how `PendingIo` is imported in this file: it comes from `use crate::session::{... PendingIo ...}` at the top, so add `FilenameReq` to that list).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app ingame_create_by_prompt_bubbles_filename_request_and_resume_continues`
Expected: FAIL to compile — `FilenameReq`, `pending_filename`, `resume_filename` don't exist.

- [ ] **Step 3: Implement**

**3a. `crates/app/src/session.rs`** — define next to `PendingIo` (~line 40):

```rust
/// A game-initiated Glk `create_by_prompt` awaiting a host-supplied filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilenameReq {
    /// Glk fileusage.
    pub usage: u32,
    /// Glk filemode (Read `0x02`, Write `0x01`, ReadWrite `0x03`, WriteAppend `0x05`).
    pub fmode: u32,
}
```

**3b. `crates/app/src/engine.rs`** — extend the import (`use crate::session::{FilenameReq, InputKind, TurnResult};`) and add three methods to the `Engine` trait (in the `turn cycle` area, next to `resume_restore`):

```rust
    /// The pending `create_by_prompt` filename request, if the VM suspended on one
    /// this turn. Default `None` (only the Glulx engine issues these).
    fn pending_filename(&self) -> Option<FilenameReq> {
        None
    }
    /// The user-visible VFS filenames, for a `create_by_prompt` read picker.
    /// Default empty (engines without a Glk VFS).
    fn file_names(&self) -> Vec<String> {
        Vec::new()
    }
    /// Resume after the host chose a filename (or cancelled with `None`) for a
    /// `create_by_prompt`. Only valid for engines that produce filename requests;
    /// the default panics because the run loop only calls this when
    /// [`Engine::pending_filename`] returned `Some`.
    fn resume_filename(&mut self, _name: Option<String>) -> TurnResult {
        unreachable!("resume_filename is only valid for engines that issue filename requests (Glulx)")
    }
```

**3c. `crates/app/src/glulx_session.rs`:**

- Add `FilenameReq` to the `use crate::session::{...}` import at the top.
- Add a field to `GlulxSession` (next to `pending_io`, ~line 76):
  ```rust
      /// A game-initiated create_by_prompt awaiting a host filename, bubbled to the
      /// run loop. Set when a turn's drive stops on one; cleared by resume_filename.
      pending_filename: Option<FilenameReq>,
  ```
  and initialize `pending_filename: None,` in `GlulxSession::new` (next to `pending_io: None,`, ~line 193).
- Add a `DriveStop` variant (~line 118):
  ```rust
      /// The game executed `create_by_prompt`: the host must supply a filename, then
      /// [`GlulxSession::resume_filename`].
      Filename { usage: u32, fmode: u32 },
  ```
- In `drive()` (~line 145), add the arm:
  ```rust
              StepResult::NeedFilename { usage, fmode } => return DriveStop::Filename { usage, fmode },
  ```
- In `drive_settled()` (~line 156), add an arm that auto-cancels (startup/resize/sound have no UI to prompt — mirror the `Save`/`Restore` auto-fail):
  ```rust
              DriveStop::Filename { .. } => machine.supply_filename(None),
  ```
- In `drive_turn()` (~line 240), add:
  ```rust
              DriveStop::Filename { usage, fmode } => {
                  self.pending_filename = Some(FilenameReq { usage, fmode })
              }
  ```
  Also set `self.pending_filename = None;` inside the existing `DriveStop::Input` and `DriveStop::Quit` arms (alongside their existing `self.pending_io = None;`) so a resolved turn clears any prior request.
- In the `impl Engine for GlulxSession` block, override the three methods (next to `resume_save`/`resume_restore`, ~line 574):
  ```rust
      fn pending_filename(&self) -> Option<FilenameReq> {
          self.pending_filename
      }

      fn file_names(&self) -> Vec<String> {
          self.machine.file_names()
      }

      fn resume_filename(&mut self, name: Option<String>) -> TurnResult {
          // `Some` = the player chose/entered a name; `None` = cancelled (NULL fileref).
          self.machine.supply_filename(name);
          self.pending_filename = None;
          self.drive_turn();
          self.finish_turn()
      }
  ```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p app ingame_create_by_prompt_bubbles_filename_request_and_resume_continues`
Expected: PASS.

Run: `cargo test -p app` and `cargo build -p app`
Expected: the app compiles (the Glulx `StepResult` match is exhaustive again) and the suite passes. NOTE: the run loop does not yet OPEN a modal for `pending_filename` — that is Task 4/5. A real game calling `create_by_prompt` would suspend unhandled until then; this is fine mid-branch (the branch merges whole).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/engine.rs crates/app/src/glulx_session.rs
git commit -F <msg-file>
```
Message subject: `feat(app): surface create_by_prompt filename requests through Engine (SQ-0279)`

---

## Task 4 — app: write-mode name prompt + run-loop wiring + resolver

**Files:**
- Modify: `crates/app/src/state.rs` (`PromptKind::CreateFile`; `AppState` fields; a pure `filename_modal_for` decision helper + its enum)
- Modify: `crates/app/src/input.rs` (route `CreateFile` submit like `SaveAs`)
- Modify: `crates/app/src/main.rs` (open modal on `pending_filename`; resolver → `resume_filename`)
- Test: inline in `crates/app/src/state.rs` (the pure decision helper)

**Interfaces:**
- Consumes: `Engine::pending_filename`, `Engine::file_names`, `Engine::resume_filename` (Task 3); `FilenameReq`.
- Produces: `state::filename_modal_for(req, names_len) -> FilenameModal` (pure; drives which modal the run loop opens); `AppState.pending_filename: Option<FilenameReq>`; `AppState.filename_submitted: Option<Option<String>>` (flag-hop: outer = "a decision is ready", inner = name-or-cancel).

**Design note:** mirror the in-game save/restore flow. `finish_turn`/`submit` already return; the run loop, after applying a turn, checks `session.pending_filename()`. `filename_modal_for` decides: read mode with ≥1 file → picker (Task 5); read mode with 0 files → auto-cancel (`resume_filename(None)`); any write mode → name prompt. This task implements everything EXCEPT the picker (Task 5), which is why the helper returns a `Picker` variant the run loop routes to Task 5's modal.

- [ ] **Step 1: Write the failing test (pure decision helper)**

In `crates/app/src/state.rs` `#[cfg(test)] mod tests`, add:

```rust
#[test]
fn filename_modal_for_picks_prompt_picker_or_autocancel() {
    use super::{filename_modal_for, FilenameModal};
    use crate::session::FilenameReq;
    // Write / WriteAppend / ReadWrite -> name prompt (regardless of existing files).
    assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x01 }, 3), FilenameModal::NamePrompt);
    assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x05 }, 0), FilenameModal::NamePrompt);
    assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x03 }, 0), FilenameModal::NamePrompt);
    // Read with existing files -> picker.
    assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x02 }, 2), FilenameModal::Picker);
    // Read with no files -> nothing to pick, auto-cancel.
    assert_eq!(filename_modal_for(FilenameReq { usage: 0, fmode: 0x02 }, 0), FilenameModal::AutoCancel);
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app filename_modal_for_picks_prompt_picker_or_autocancel`
Expected: FAIL — `filename_modal_for`/`FilenameModal` don't exist.

- [ ] **Step 3: Implement the helper**

In `crates/app/src/state.rs` (module scope, near other small helpers):

```rust
/// Which modal the run loop opens for a `create_by_prompt` filename request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilenameModal {
    /// Read mode with existing files: let the player pick one.
    Picker,
    /// Write / WriteAppend / ReadWrite: prompt for a new name.
    NamePrompt,
    /// Read mode with no existing files: nothing to pick — cancel immediately.
    AutoCancel,
}

/// Decide the modal for a filename request. Read mode (`fmode == 0x02`) picks from
/// existing VFS files (or auto-cancels when there are none); every other mode
/// prompts for a name.
pub fn filename_modal_for(req: crate::session::FilenameReq, existing_files: usize) -> FilenameModal {
    if req.fmode == 0x02 {
        if existing_files == 0 {
            FilenameModal::AutoCancel
        } else {
            FilenameModal::Picker
        }
    } else {
        FilenameModal::NamePrompt
    }
}
```

- [ ] **Step 4: Add the prompt kind + state fields**

In `crates/app/src/state.rs`:
- Add to `PromptKind` (~line 705):
  ```rust
      /// Enter a filename for a game `create_by_prompt` (write modes). The pending
      /// request lives on `AppState.pending_filename`.
      CreateFile,
  ```
- Add to `AppState` (near `ingame_io`/`saves_prompt_submitted`, ~line 1139):
  ```rust
      /// A game `create_by_prompt` awaiting a host filename (the modal is open).
      pub pending_filename: Option<crate::session::FilenameReq>,
      /// Flag-hop: the chosen filename (`Some(name)`) or cancel (`None`) submitted
      /// from the CreateFile prompt / file picker, drained by the run loop to call
      /// `resume_filename`. Outer `Some` = a decision is ready.
      pub filename_submitted: Option<Option<String>>,
  ```
  Initialize both to `None` in `AppState`'s constructor/`Default` (match how `ingame_io: None` / `saves_prompt_submitted: None` are initialized).

- [ ] **Step 5: Route the `CreateFile` submit (input.rs)**

In `crates/app/src/input.rs`, in `finish_prompt` (~line 3610) where `SaveAs | ConfirmDeleteSave | ConfigEditPath` return the prompt to the caller, add `PromptKind::CreateFile` to that same return group so its submitted text reaches the run loop:

```rust
        PromptKind::SaveAs
        | PromptKind::CreateFile
        | PromptKind::ConfirmDeleteSave(_)
        | PromptKind::ConfigEditPath { .. } => {
            return Some(prompt);
        }
```

Then, wherever the returned prompt is stored (the code that sets `state.saves_prompt_submitted` from a returned `SaveAs`), add a branch: if the returned prompt is `CreateFile`, set `state.filename_submitted = Some(Some(buffer))` instead. (Find the site that currently does `state.saves_prompt_submitted = Some((kind, buf))`; add a `match`/`if` so `CreateFile` routes to `filename_submitted`.) Preserve the existing `SaveAs`/`ConfirmDeleteSave`/`ConfigEditPath` behavior unchanged.

- [ ] **Step 6: Open the modal + resolver (main.rs)**

**6a. Open on a pending request.** Find where a turn result is applied and `result.pending_io` opens `open_ingame_saves` (~line 4685). Right after that block, add handling keyed on the session's filename request. Because the picker is Task 5, this task wires NamePrompt + AutoCancel and leaves a clearly-marked hook for Picker:

```rust
    if state.pending_filename.is_none() {
        if let Some(req) = session.pending_filename() {
            match app::state::filename_modal_for(req, session.file_names().len()) {
                app::state::FilenameModal::NamePrompt => {
                    state.pending_filename = Some(req);
                    state.prompt = Some(app::state::Prompt {
                        kind: app::state::PromptKind::CreateFile,
                        buffer: String::new(),
                    });
                }
                app::state::FilenameModal::AutoCancel => {
                    // Read with no files to pick: cancel immediately (NULL fileref).
                    state.filename_submitted = Some(None);
                }
                app::state::FilenameModal::Picker => {
                    // Task 5 opens the file picker here.
                    state.pending_filename = Some(req);
                    open_filename_picker(&mut state, &*session); // added in Task 5
                }
            }
        }
    }
```

(Match the actual local variable names for `session`/`state` at that site — they may be `&mut *session` etc. If `open_filename_picker` does not yet exist, Task 5 adds it; to keep THIS task compiling, temporarily inline the Picker arm as `state.filename_submitted = Some(None);` with a `// TODO(Task 5): picker` comment, and Task 5 replaces it.)

**6b. Resolver.** Add a resolver mirroring `resolve_ingame_dialog` (~line 5024). It drains `filename_submitted`, calls `resume_filename`, and runs the standard post-turn bookkeeping via `finish_resumed_turn`:

```rust
/// Resume a suspended `create_by_prompt` once the player has entered a name in the
/// CreateFile prompt / picked a file / cancelled (flag-hopped via
/// `state.filename_submitted`). Mirrors `resolve_ingame_dialog`.
fn resolve_filename_request(
    session: &mut dyn Engine,
    mapper: &mut Mapper,
    state: &mut AppState,
    save_dir: &std::path::Path,
    ifid: &str,
    map_area: Rect,
) -> bool {
    if let Some(choice) = state.filename_submitted.take() {
        state.pending_filename = None;
        let result = session.resume_filename(choice);
        return finish_resumed_turn(result, mapper, state, session, save_dir, ifid, map_area);
    }
    // Cancel: the modal closed without a submit (Esc). If a request is still pending
    // and no prompt/picker is open, treat it as a cancel.
    if state.pending_filename.is_some()
        && state.prompt.as_ref().map_or(true, |p| p.kind != app::state::PromptKind::CreateFile)
        && !filename_picker_open(state) // Task 5 predicate; before Task 5 this is `true`
    {
        state.pending_filename = None;
        let result = session.resume_filename(None);
        state.push_notice("[create_by_prompt cancelled]");
        return finish_resumed_turn(result, mapper, state, session, save_dir, ifid, map_area);
    }
    false
}
```

Call `resolve_filename_request(...)` in the run loop at the same two sites `resolve_ingame_dialog` is dispatched (~line 3518 and ~line 3953), right after the `resolve_ingame_dialog` call. (Before Task 5, define `filename_picker_open` as a stub returning `false`, or omit that clause and add it in Task 5.)

- [ ] **Step 7: Verify**

Run: `cargo test -p app` and `cargo build -p app`
Expected: builds and all tests (incl. `filename_modal_for_*`) pass.

**Manual smoke** (write path): run a Glulx game that scripts a transcript — `cargo run -p app -- <a Glulx story>`, then `SCRIPT ON` (or an equivalent that calls `create_by_prompt` for write). A name-entry prompt appears; type a name, Enter; the game continues. Type `SCRIPT ON` again with a different name → no collision (two distinct VFS files). Esc at the prompt → the game sees a cancelled (NULL) fileref and continues.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/input.rs crates/app/src/main.rs
git commit -F <msg-file>
```
Message subject: `feat(app): name create_by_prompt files via a host prompt (SQ-0279)`

---

## Task 5 — app: read-mode file picker

**Files:**
- Modify: `crates/app/src/state.rs` (`FilePickerState`)
- Create: `crates/app/src/render/file_picker.rs` (draw the picker with existing dialog chrome)
- Modify: `crates/app/src/render/mod.rs` (register the module) and the render dispatch that draws overlays
- Modify: `crates/app/src/input.rs` (picker key handling: Up/Down/Enter/Esc)
- Modify: `crates/app/src/main.rs` (`open_filename_picker`, `filename_picker_open`; replace the Task 4 Picker/stub hooks)
- Test: inline in `crates/app/src/state.rs` (`FilePickerState` navigation)

**Interfaces:**
- Consumes: `Engine::file_names`; `AppState.pending_filename`, `AppState.filename_submitted` (Task 4); `state::ListScroll` (existing).
- Produces: `state::FilePickerState { names: Vec<String>, scroll: ListScroll }` with `selected()`, `move_up()`, `move_down()`; `AppState.file_picker: Option<FilePickerState>`; `main::open_filename_picker`, `main::filename_picker_open`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/state.rs` tests:

```rust
#[test]
fn file_picker_navigation_clamps_and_selects() {
    use super::FilePickerState;
    let mut p = FilePickerState::new(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    assert_eq!(p.selected(), Some("a"));
    p.move_up(); // clamps at top
    assert_eq!(p.selected(), Some("a"));
    p.move_down();
    p.move_down();
    assert_eq!(p.selected(), Some("c"));
    p.move_down(); // clamps at bottom
    assert_eq!(p.selected(), Some("c"));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app file_picker_navigation_clamps_and_selects`
Expected: FAIL — `FilePickerState` doesn't exist.

- [ ] **Step 3: Implement `FilePickerState`**

In `crates/app/src/state.rs`:

```rust
/// A minimal list picker over existing VFS filenames, for a read-mode
/// `create_by_prompt`. Rendered with the shared dialog chrome.
#[derive(Debug, Clone)]
pub struct FilePickerState {
    pub names: Vec<String>,
    pub scroll: crate::list_scroll::ListScroll,
}

impl FilePickerState {
    pub fn new(names: Vec<String>) -> Self {
        FilePickerState { names, scroll: Default::default() }
    }
    pub fn selected(&self) -> Option<&str> {
        self.names.get(self.scroll.selected).map(|s| s.as_str())
    }
    pub fn move_up(&mut self) {
        self.scroll.selected = self.scroll.selected.saturating_sub(1);
    }
    pub fn move_down(&mut self) {
        if self.scroll.selected + 1 < self.names.len() {
            self.scroll.selected += 1;
        }
    }
}
```

(Match the actual field/method names of `ListScroll` — the saves manager uses `scroll.selected()` / `target_offset()`. If `ListScroll`'s selection accessor differs from a public `selected` field, adapt `selected()`/`move_*` to its API; keep the test's observable behavior.)

Add the field to `AppState`: `pub file_picker: Option<FilePickerState>,` (init `None`).

- [ ] **Step 4: Render (render/file_picker.rs)**

Create `crates/app/src/render/file_picker.rs` with a `pub fn draw_file_picker(f, area, picker: &FilePickerState, colors: &ColorScheme)` that draws a centered dialog titled "Pick a file" listing `picker.names` with the selected row highlighted, using ONLY existing themed styles (`colors.dialog`, `colors.scrollbar`, and the same selected-row treatment `render/saves.rs` uses). Mirror `render/saves.rs`'s layout/scrollbar. Register `mod file_picker;` in `crates/app/src/render/mod.rs`, and call `draw_file_picker` from the overlay-drawing dispatch when `state.file_picker.is_some()` (same place the saves manager / prompt overlays are drawn).

- [ ] **Step 5: Input handling (input.rs)**

Where overlay key handling lives, add: when `state.file_picker` is `Some`, handle Up (`move_up`), Down (`move_down`), Enter (`state.filename_submitted = Some(Some(selected.to_string()))`, then close: `state.file_picker = None`), Esc (`state.file_picker = None` — leaves `pending_filename` set so the resolver treats it as a cancel). Honor Shift-Tab/Tab only if the surrounding convention requires; a list picker needs only Up/Down/Enter/Esc.

- [ ] **Step 6: Wire main.rs hooks**

Add:
```rust
fn open_filename_picker(state: &mut AppState, session: &dyn Engine) {
    state.file_picker = Some(app::state::FilePickerState::new(session.file_names()));
}
fn filename_picker_open(state: &AppState) -> bool {
    state.file_picker.is_some()
}
```
Replace the Task 4 Picker-arm stub (`// TODO(Task 5)`) with the real `open_filename_picker(...)` call, and replace the `filename_picker_open` stub in `resolve_filename_request` with this real predicate.

- [ ] **Step 7: Verify**

Run: `cargo test -p app` and `cargo build -p app`
Expected: builds; `file_picker_navigation_clamps_and_selects` and the whole suite pass.

**Manual smoke** (read path): with a game that reads a prior file via `create_by_prompt` (e.g. command replay, or a game that reads back a data file it wrote): the picker lists existing VFS files; Up/Down + Enter picks one and the read succeeds; Esc cancels (game sees NULL fileref). With no existing files, the read auto-cancels with no empty picker shown.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/render/file_picker.rs crates/app/src/render/mod.rs crates/app/src/input.rs crates/app/src/main.rs
git commit -F <msg-file>
```
Message subject: `feat(app): pick an existing file for read-mode create_by_prompt (SQ-0279)`

---

## Task 6 — docs + to-verify

**Files:**
- Modify: `docs/persistence.md`

- [ ] **Step 1: Update the limitations section**

In `docs/persistence.md`, the "Known limitations (Glk file VFS)" section currently says `create_by_prompt uses a fixed per-usage name … (tracked as SQ-0279)`. Replace that bullet with a short paragraph (in the appropriate section) describing the new behavior: prompted files now get a player-chosen name via a host prompt (write / append / read-write) or a picker over existing VFS files (read), the bytes live in the VFS and persist per-story via the SQ-0278 sidecar exactly like any other Glk file, and note the remaining simplification that the read picker is **not** filtered by usage class (all user files are listed) since the GVFS codec does not record per-file usage. Keep the "text-mode newline translation is omitted" bullet.

- [ ] **Step 2: Verify**

Run: `grep -n "SQ-0279\|create_by_prompt" docs/persistence.md`
Expected: the limitation-as-bug wording is gone; the new behavior + remaining simplification are documented.

- [ ] **Step 3: Commit**

```bash
git add docs/persistence.md
git commit -F <msg-file>
```
Message subject: `docs(persistence): create_by_prompt now names VFS files (SQ-0279)`

**After merge (controller, not a task):** check off the SQ-0279 smoke items in the external memory `to-verify.md` list for manual confirmation (write-path name prompt; read-path picker; cross-session persistence of a named prompted file). This file is OUTSIDE the repo — never stage it.

---

## Self-Review notes (for the controller)

- **Spec coverage:** suspend/resume (T1), CLI (T2), engine plumbing (T3), write prompt (T4), read picker (T5), docs (T6). Every Option-1 requirement maps to a task.
- **Type consistency:** `StepResult::NeedFilename { usage, fmode }`, `PendingFileref { dest, usage, fmode, rock }`, `FilenameReq { usage, fmode }`, `FilenameModal { Picker, NamePrompt, AutoCancel }`, `FilePickerState { names, scroll }`, `supply_filename(Option<String>)`, `resume_filename(Option<String>) -> TurnResult`, `file_names() -> Vec<String>`, `filename_modal_for(FilenameReq, usize) -> FilenameModal`. Names are used identically across tasks.
- **Deferred (was Option 2, out of scope):** usage-tagged browsable rows in the saves manager; per-file usage in the GVFS codec; usage-filtered read picker.
- **Known cross-task compile ordering:** the app first fully compiles at end of Task 3; Tasks 4/5 keep it compiling via the stubbed Picker hook that Task 5 replaces. The branch merges whole.
