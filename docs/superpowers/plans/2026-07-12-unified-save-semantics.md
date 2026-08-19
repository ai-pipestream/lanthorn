# Unified Save Semantics (SQ-0283) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Glulx in-game `@save`/`@restore` produce a **spec-conformant standard Glulx-Quetzal** save (like the Z-machine's `.qzl`), fix the "Save failed." bug in Counterfeit Monkey, and decouple saves from the Glk VFS — while leaving the `.lanthorn` Save State path untouched.

**Architecture:** Add a bare, call-stub-based Glulx save (`save_quetzal`/`restore_quetzal`) beside the existing full snapshot (`save_state`/`restore_state`). `@save` pushes a Glulx call stub for its `S1` operand so PC/FP/SP self-describe (spec §1.8.2); `restore_quetzal` restores VM memory/stack/heap and pops the stub to resume, leaving all live Glk/iosys/stringtbl/protect state intact (spec §1.8.5). A tiny write-count shim satisfies the game's stream check under host-intercepted delivery. SavedGame Glk streams become null conduits, removing saves from the VFS.

**Tech Stack:** Rust workspace — `gvm` (Glulx VM + Glk model, zero-dep), `gvm-cli`, `zvm-cli`, `app` (ratatui TUI).

**Design spec:** `docs/superpowers/specs/2026-07-11-unified-save-semantics-design.md` — read it for rationale and spec citations. This plan is the executable decomposition.

## Global Constraints

- `gvm` and `zvm` crates stay **zero external deps**. CLI/app crates may use their existing deps only.
- **Do NOT change `save_state()` / `restore_state()` bytes or behavior** — that is the `.lanthorn` Save State path. A regression test guards this.
- Spec conformance targets (Glulx Spec, Plotkin): §1.3.2 call stub `(DestType, DestAddr, PC, FramePtr)`; §1.8 chunk set `IFhd + (CMem|UMem) + Stks + MAll`; §1.8.2 `@save` pushes a stub, restore pops it and stores the result (`0` success / `1` fail now, `-1` on a restored resume); §1.8.5 iosys mode/rock, string-decoding table, protect range, Glk state are **NOT saved** and must be left live on `restore_quetzal`.
- Commit trailers on every commit:
  ```
  Quest: SQ-0283
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Branch `sq-0283-unified-saves` (already created off `main`). Stage only the files each task edits — never `git add -A` (the tree has pre-existing untracked files).
- Verify with real `cargo build`/`cargo test` (do not trust editor diagnostics). Each task ends green.

---

## Task 1: `save_quetzal()` + `@save` call stub + `complete_save` stub-pop

**Files:**
- Modify: `crates/gvm/src/exec.rs` — `save_state` neighborhood (~1439), the `@save` handler `0x0123` (~724), `complete_save` (~3403), `return_value` (~1133, factor a helper out).
- Test: `crates/gvm/src/exec.rs` `#[cfg(test)] mod tests`.

**Interfaces:**
- Produces: `pub fn save_quetzal(&self) -> Vec<u8>`; a private `fn pop_save_stub_and_store(&mut self, v: u32) -> R<()>` (the stub-pop tail of `return_value`, WITHOUT the `sp = fp` frame teardown); reworked `@save` and `complete_save`.
- Consumes: existing `IFhd`/`compress_ram`(`CMem`)/`Stks`/`MAll` chunk builders in `save_state`; `Dest::to_stub()`; `push32`.

**Design notes:** `save_quetzal` = `save_state` MINUS the `GReg` and `Glk ` chunks. `@save` currently stores `-1` into `S1` and stashes `dest`; replace that with pushing a call stub `(dtype,daddr,pc,fp)` for `S1` (PC = `self.pc`, already past the opcode). `complete_save` currently does `store(dest, 0|1)`; replace with popping that stub and storing the run result via `pop_save_stub_and_store`. The `PendingSaveLoad` no longer needs `dest` for the save case (the stub carries it) — keep the field for the restore case.

- [ ] **Step 1: Write failing test — `save_quetzal` omits `GReg`/`Glk `.**
  Mirror the existing `strip_chunk`/Glk-chunk test (~exec.rs:5906). Build a small machine, run to a point, call `save_quetzal()`, assert the byte stream contains `IFhd`,`CMem`,`Stks` and does NOT contain `GReg` or `Glk `; assert `save_state()` on the same machine DOES contain them.
- [ ] **Step 2: Run it — expect fail** (`cargo test -p gvm save_quetzal_omits`). Method missing → compile error / fail.
- [ ] **Step 3: Implement `save_quetzal`.** Copy `save_state`'s body, drop the `GReg` and `Glk ` `push_chunk` lines. Keep `FORM IFZS` framing identical.
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Write failing test — `@save` pushes a stub; `complete_save(true)` resumes with `S1==0`.**
  Assemble `@save 0, -> mem[0x100]` then a trailing sentinel `copy 0x7F -> mem[0x104]` then `quit` (mirror `save_suspends_and_failure_stores_one` at ~7457, but assert success path). Run to `SaveRequest`; assert the stack grew by 16 bytes (a stub was pushed); call `complete_save(true)`; assert `mem[0x100] == 0` and that stepping resumes and reaches the sentinel (`mem[0x104] == 0x7F`).
- [ ] **Step 6: Run — expect fail.**
- [ ] **Step 7: Implement the `@save` rework + `pop_save_stub_and_store` + `complete_save` rework.**
  - Factor `return_value`'s tail (from after `self.sp = self.fp;` through the `match dtype` store) into `fn pop_save_stub_and_store(&mut self, v: u32) -> R<()>`; have `return_value` call `self.sp = self.fp; self.pop_save_stub_and_store(v)`.
  - `@save` (`0x0123`): read `(l, s) = read_operands(1,1)` (as today; `s[0]` is already a `Dest` — the old code passed it straight to `self.store`); `let (dt, da) = s[0].to_stub();` push `dt, da, self.pc, self.fp as u32` (same order as `call_function`); set `pending_saveload = Some(PendingSaveLoad { restore: false, .. })`. Keep `l[0]` (the stream) for Task 3's shim.
  - `complete_save(ok)`: `let _ = self.pop_save_stub_and_store(if ok {0} else {1});` (drop the old `store(dest,…)`); keep the `pending_saveload.take()` guard.
- [ ] **Step 8: Run the new tests + full gvm suite** (`cargo test -p gvm`). Expect pass, including the existing save/restore tests (adjust `save_suspends_and_failure_stores_one` if it asserted the old `-1`-into-mem behavior; the failure path now stores `1` via the stub — update its expectation to match the stub-based store).
- [ ] **Step 9: Commit** — `feat(gvm): standard bare Glulx save via call stub (save_quetzal) (SQ-0283)` (+ trailers). Stage only `crates/gvm/src/exec.rs`.

---

## Task 2: `restore_quetzal()` + `complete_restore_quetzal()` (live-state preserving)

**Files:**
- Modify: `crates/gvm/src/exec.rs` — `restore_state` (~1528) refactor + new `restore_quetzal`; `complete_restore_success` (~3425) neighbor.
- Test: same file's tests.

**Interfaces:**
- Produces: `pub fn restore_quetzal(&mut self, blob: &[u8]) -> Result<(), GError>`; `pub fn complete_restore_quetzal(&mut self, blob: &[u8]) -> bool`; `pub fn is_saveload_pending(&self) -> bool` (`self.pending_saveload.is_some()` — consumed by Task 6's host-snapshot guard); a private shared `fn restore_vm_core(&mut self, chunks…) -> Result<(), GError>` used by both `restore_state` and `restore_quetzal`.
- Consumes: `pop_save_stub_and_store` (Task 1); existing `CMem`/`UMem`/`Stks`/`MAll` parsing in `restore_state`.

**Design notes:** Factor the shared parse+apply of `IFhd`(verify identity)/`CMem`|`UMem`(reset image + diff, honoring the live protect range)/`Stks`(load stack)/`MAll`(heap) into `restore_vm_core`. `restore_state` = `restore_vm_core` + apply `GReg` (sp/fp/pc/iosys/stringtbl/protect) + replace Glk from `Glk ` chunk. `restore_quetzal` = `restore_vm_core` + **pop the call stub storing `-1`** (recovers pc/fp; sp from Stks length) + **leave `self.glk`, iosys, stringtbl, protect untouched** (§1.8.5). Also confirm `CMem`/`UMem` both read (write `CMem` only).

- [ ] **Step 1: Failing test — full `@save`→`restore_quetzal` round-trip preserving live Glk/iosys.**
  Build a machine; open a Glk window + a data file-stream (put a byte in the VFS) and set a non-default `iosys` via `setiosys`; run to an `@save`; capture `save_quetzal()`; mutate RAM + change iosys + write more VFS; then `restore_quetzal(&blob)`. Assert: RAM/stack restored; the `@save`'s `S1 == -1` after resume; **`self.glk` still has the window/stream/VFS byte from AFTER the save-time mutation is NOT reverted** (i.e. live Glk preserved, not reset to save-time); **iosys is the current (post-restore) value, not the saved one**. This asserts the §1.8.5 exclusion — the distinguishing behavior from `restore_state`.
- [ ] **Step 2: Run — expect fail.**
- [ ] **Step 3: Implement `restore_vm_core` refactor + `restore_quetzal` + `complete_restore_quetzal`.**
  - Extract shared logic from `restore_state`; `restore_state` keeps applying `GReg` + `Glk ` after the core.
  - `restore_quetzal`: call core; then `self.pop_save_stub_and_store((-1i32) as u32)?`; do NOT touch glk/iosys/stringtbl/protect. Ensure `UMem` is accepted (core handles both).
  - `complete_restore_quetzal(blob)`: mirror `complete_restore_success` but call `restore_quetzal`; on `Ok` clear `pending_saveload` + `undo_stack`, return true; on `Err` push diagnostic, return false.
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Failing tests — `UMem` read + protect-range honored.**
  (a) Hand-build a `FORM IFZS` with a `UMem` (uncompressed) chunk and assert `restore_quetzal` applies it. (b) Set a `@protect` range, `restore_quetzal` a blob whose diff would overwrite it, assert those bytes keep pre-restore values (§1.8.5).
- [ ] **Step 6: Run — expect fail; Step 7: implement any gaps; Step 8: run — pass.**
- [ ] **Step 9: Add `is_saveload_pending()`** (`pub fn is_saveload_pending(&self) -> bool { self.pending_saveload.is_some() }`) with a one-line test (pending after `@save`/`@restore` suspends, false otherwise). This is consumed by Task 6's host-snapshot guard (see the carry-forward below).
- [ ] **Step 10: Regression — `save_state`/`restore_state` unchanged.** Run the existing Save-State round-trip test (`save_restore_roundtrips_ram_stack_heap_registers` ~5752) — must still pass byte-for-byte after the refactor. Run `cargo test -p gvm`.
- [ ] **Step 10: Commit** — `feat(gvm): live-state-preserving restore_quetzal for in-game @restore (SQ-0283)`. Stage only `crates/gvm/src/exec.rs`.

---

## Task 3: Glk stream write-count shim

**Files:**
- Modify: `crates/gvm/src/glk.rs` (add `note_stream_write`); `crates/gvm/src/exec.rs` (`@save` credits `L1`).
- Test: both files' tests.

**Interfaces:**
- Produces: `pub fn note_stream_write(&mut self, id: u32, n: u32)` on `Model`.
- Consumes: `save_quetzal` (Task 1), `stream_mut`.

- [ ] **Step 1: Failing test (glk.rs) — `note_stream_write` bumps `write_count`, stores no bytes.** Open a file stream over a named VFS file; `note_stream_write(sid, 42)`; assert `stream_close(sid)` returns `write==42` and the VFS file length is unchanged (still empty).
- [ ] **Step 2: Run — fail. Step 3: implement** `note_stream_write` (`if let Some(st)=self.stream_mut(id){ st.write_count = st.write_count.saturating_add(n); }`). **Step 4: run — pass.**
- [ ] **Step 5: Failing test (exec.rs) — `@save` credits its `L1` stream.** Build a machine with an open file stream `sid`; run an `@save sid, S1`; assert (before `complete_save`) `glk.stream(sid).write_count == save_quetzal().len()`.
- [ ] **Step 6: Run — fail. Step 7:** in the `@save` handler, after pushing the stub: `let n = self.save_quetzal().len() as u32; self.glk.note_stream_write(l[0], n);` (l[0] = the stream operand). **Step 8: run — pass; run `cargo test -p gvm`.**
- [ ] **Step 9: Commit** — `fix(gvm): credit the @save Glk stream write count so libraries see success (SQ-0283)`. Stage `crates/gvm/src/glk.rs` + `crates/gvm/src/exec.rs`.

---

## Task 4: SavedGame Glk streams → `StreamKind::Null` conduits (VFS decouple)

**Files:**
- Modify: `crates/gvm/src/glk.rs` — `StreamKind` enum (~621), `stream_open_file` (~1522), `stream_close` (~1556), `file_stream_write`/`file_stream_read_char`/`stream_position` (Null arms).
- Test: glk.rs tests.

**Interfaces:**
- Produces: `StreamKind::Null` variant; SavedGame-usage branch in `stream_open_file`.
- Consumes: `fileref_name` (returns usage); `note_stream_write` (Task 3, works on Null via `stream_mut`).

**Design notes:** `fileusage_SavedGame == 0x01`; test `usage & 0x0f == 0x01`. A `Null` stream: no `self.files` entry, no `file_streams` side-table entry, opens successfully for ALL modes (crucially Read succeeds with no prior save so the game reaches `@restore` and the host decides), discards writes, reads EOF. Free it in `stream_close` like a memory stream.

- [ ] **Step 1: Failing test — SavedGame open creates no VFS entry and Read succeeds with no prior save.** `fileref_create(0x01, "s", 0)` → `stream_open_file(fref, FM_WRITE)` returns non-zero, `self.files` has no `"s"` key; then `stream_open_file(fref, FM_READ)` (no file exists) returns non-zero (a conduit). Writes via `file_stream_write` discard (VFS still empty); `file_stream_read_char` returns `None` (EOF).
- [ ] **Step 2: Run — fail. Step 3: implement.**
  - Add `Null` to `StreamKind` (keep it `Copy`).
  - In `stream_open_file`, right after `fileref_name`: `if usage & 0x0f == 0x01 { let sid = self.alloc_stream(StreamKind::Null, rock); return sid; }` (no `file_streams` insert, no `self.files` touch).
  - `stream_close`: free `Null` like `Memory` (`streams[id-1]=None`), no `file_streams` removal.
  - `file_stream_write`/`file_stream_read_char`/`stream_position`: `Null` → discard / `None` / `Some(0)` (guard by checking the stream kind or the absence of a `file_streams` entry).
- [ ] **Step 4: Run — pass.**
- [ ] **Step 5: Failing test — a Glulx `@save` leaves no SavedGame slot in the VFS nor in a Save State.** Drive a tiny program: create_by_prompt(SavedGame) is host-resolved → open stream Write → `@save`. Assert `vfs_bytes()` contains no save slot and `save_state()`'s `Glk ` chunk has no SavedGame file. (If a full drive is heavy, assert the narrower property: after opening a SavedGame conduit and writing, `self.files` is empty.)
- [ ] **Step 6–8: run/implement/run.** **Step 9: `cargo test -p gvm`.**
- [ ] **Step 10: Commit** — `feat(gvm): SavedGame Glk streams are null conduits, decoupling saves from the VFS (SQ-0283)`. Stage `crates/gvm/src/glk.rs`.

---

## Task 5: `gvm-cli` host wiring — prompted filename + standard save

**Files:**
- Modify: `crates/gvm-cli/src/main.rs` — `drive` `SaveRequest`/`RestoreRequest` arms (~301–319), the `save_path` setup (~380), the drive signature/callers if needed.
- Test: the headless drive tests in the same file (~819).

**Interfaces:**
- Consumes: `machine.save_quetzal()` (Task 1), `machine.complete_restore_quetzal()` (Task 2), `machine.complete_save()`.

**Design notes:** Mirror `zvm-cli`: on `SaveRequest`, prompt "Save to file: ", write `save_quetzal()` to the typed path, `complete_save(ok)`. On `RestoreRequest`, prompt "Restore from file: ", read, `complete_restore_quetzal(&bytes)` (else `complete_restore_failure`). Remove the fixed `<story>.glksave` slot. Keep the `.glkvfs` sidecar. Reuse the existing raw-mode `read_line_raw`/filename-prompt path already used for non-SavedGame `NeedFilename`.

- [ ] **Step 1: Update the headless drive test** (`drive_persists_and_reloads_the_vfs_sidecar` neighborhood) OR add a new test: drive an assembled program that does create_by_prompt(SavedGame)+open+`@save`+`@restore`, feeding a fixed filename through the prompt reader, and assert the written file is a valid `save_quetzal` (round-trips via `restore_quetzal`). Expect fail initially.
- [ ] **Step 2: Run — fail. Step 3: implement** the `SaveRequest`/`RestoreRequest` rewrite + drop `save_path`. Wire the filename prompt to the existing `read_line`/`before_input` params.
- [ ] **Step 4: Run — pass. Step 5: `cargo test -p gvm-cli` + `cargo build -p gvm-cli`.**
- [ ] **Step 6: Commit** — `feat(gvm-cli): prompt for a filename and write a standard .qzl on in-game save (SQ-0283)`. Stage `crates/gvm-cli/src/main.rs`.

---

## Task 6: `app` host wiring — Glulx in-game save uses `save_quetzal`

**Files:**
- Modify: `crates/app/src/persist_files.rs` (`save_game_named_bytes` ~236), `crates/app/src/glulx_session.rs` (expose `save_quetzal`/`complete_restore_quetzal`), `crates/app/src/session.rs` (Engine plumbing if needed), `crates/app/src/main.rs` (the Glulx `.qzl` restore branch of `Action::SavesLoad` / `resume_restore`).
- Test: `crates/app/src/glulx_session.rs` tests (mirror `glulx_state_round_trips_through_lanthorn_archive`).

**Interfaces:**
- Consumes: `Machine::save_quetzal`, `Machine::complete_restore_quetzal` (Tasks 1–2).
- Produces: an engine-level accessor for the Glulx bare save bytes + restore.

**Design notes:** Only the Glulx path changes: swap `session.save_state().bytes` → the new `save_quetzal()` bytes in `save_game_named_bytes`; route the Glulx in-game `.qzl` restore through `complete_restore_quetzal` instead of `complete_restore_success`. Z-machine paths and the `.lanthorn` Save State path are untouched. The `<ifid>-<slug>.qzl` naming stays (SQ-0284 changes naming later).

**CARRY-FORWARD FIX (from Task 1 review, Important):** With the new stack-based `@save` stub, a host `save_state()` taken *during* an `@save` suspension captures the un-popped stub, and `restore_state` never pops it → corrupted stack on resume. Interactive Ctrl+S is already overlay-guarded, but **exit auto-save** (`crates/app/src/main.rs` ~4047–4060, `session.save_state()` called unconditionally when `config.auto_save`) is NOT. Guard every host-snapshot trigger against a pending in-game save: use the new `Engine`/`Machine::is_saveload_pending()` (Task 2) and, when it's true, **skip the exit auto-save** (the in-game save the user was making is the relevant persistence anyway) — do not capture a snapshot mid-suspension. Add a regression test.

- [ ] **Step 1: Failing test — Glulx in-game save/restore round-trips via `save_quetzal`.** In `glulx_session.rs` tests: build a Glulx session, run to a prompt, take a Glulx in-game save (bare bytes), mutate, restore, assert VM state restored and the live Glk window survives. Expect fail (until wired).
- [ ] **Step 2: Run — fail. Step 3: implement** the accessor(s) + `save_game_named_bytes` byte-source swap + the restore-branch method swap.
- [ ] **Step 4: Run — pass.**
- [ ] **Step 4b: Carry-forward guard (failing test first).** Add a test that exit auto-save is skipped when a save/restore is pending (`is_saveload_pending()` true) — e.g. simulate a session at an `@save` suspension and assert the exit-save path does not call `save_state()`. Then implement the guard at the exit auto-save site (`main.rs` ~4047) and any other unconditional host `save_state()` trigger. Run — pass.
- [ ] **Step 5: `cargo test -p app` (full) + `cargo build -p app`.** Confirm no Save State / `.lanthorn` regressions.
- [ ] **Step 6: Commit** — `feat(app): Glulx in-game @save writes a standard .qzl, restore preserves live state (SQ-0283)`. Stage the edited app files by path. (Guard fix may be a second commit `fix(app): skip host snapshot while an in-game @save is pending (SQ-0283)`.)

---

## Task 7: `zvm-cli` in-game restore consistency fix

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` — `RestoreRequest` arm (~1015–1031).
- Test: same file (or a headless drive test if present).

**Design notes:** Replace the direct `machine.restore_quetzal(&data)` on the in-game `@restore` with `machine.complete_restore_success(&data)` so the `@save` descriptor is advanced forward like the app does (matches the descriptor-PC convention). On error keep `complete_restore_failure()`.

- [ ] **Step 1: Failing test** (or assertion) that an in-game restore resumes past the `@save` (descriptor advanced), not at it. **Step 2: run — fail. Step 3: swap to `complete_restore_success`. Step 4: run — pass. Step 5: `cargo test -p zvm-cli` + build.**
- [ ] **Step 6: Commit** — `fix(zvm-cli): complete in-game @restore via the descriptor path (SQ-0283)`. Stage `crates/zvm-cli/src/main.rs`.

---

## Task 8: Documentation

**Files:**
- Modify: `docs/persistence.md`, `docs/features/saves.md`, `crates/gvm/GLULX_NOTES.md` (§14).

**Design notes:** Reflect the spec's Documentation section. No README change.

- [ ] **Step 1:** `docs/persistence.md` — Terminology (~L18): Glulx `@save` produces a bare **standard Glulx-Quetzal** save, not the host snapshot. Layer 1 (~L31–33): Glulx writes a standard in-game save (VM only, call-stub resume, no VFS embed); interop verified internally, cross-interpreter goldens tracked under SQ-0229. Table (~L92–96): Glulx `@save`/`@restore` rows → bare `.qzl`. `create_by_prompt` section (~L102–112): SavedGame no longer resolves into a VFS slot (host conduit); VFS now holds only transcripts/recordings/data.
- [ ] **Step 2:** `docs/features/saves.md` — extend "Standard in-game save/restore" to note Glulx now writes a real standard in-game save (VM state only, distinct from `.lanthorn`), with the SQ-0229 caveat (cross-interpreter interop not yet golden-tested).
- [ ] **Step 3:** `crates/gvm/GLULX_NOTES.md` §14 — document the new `save_quetzal`/`restore_quetzal` standard path (call-stub resume; `IFhd/CMem/Stks/MAll`; §1.8.5 live-state exclusions) beside the existing `GReg`-based `save_state`, and why the two differ.
- [ ] **Step 4: Commit** — `docs(persistence): Glulx in-game saves are standard Glulx-Quetzal (SQ-0283)`. Stage the three doc files.

---

## Verification (end to end)

```bash
cargo build --workspace --tests
cargo test -p gvm          # save_quetzal/restore_quetzal round-trips, conduit, shim, live-state preservation
cargo test -p gvm-cli
cargo test -p zvm-cli
cargo test -p app          # no Save State / .lanthorn regressions
```

**Manual smoke (user — not headlessly runnable):**
- Counterfeit Monkey in `gvm-cli`: type `save` → **"Ok."** (not "Save failed."), then `restore` restores.
- Counterfeit Monkey in the app: same, through the SaveAs prompt + saves manager.
- A Glulx game with `SCRIPT ON` (open transcript window): in-game `@restore` must NOT wipe the live transcript window/VFS (validates §1.8.5 end-to-end).
- Z-machine `@save`/`@restore` and both engines' Ctrl+S Save State (`.lanthorn`) unchanged.

## Notes / Out of scope

- Storage relocation (story-filename key, per-game dir, `--data-dir`) is **SQ-0284** — do not touch save *paths/naming* here beyond what the format change requires.
- Cross-interpreter Glulx save interop goldens (Glulxe/Git) are **SQ-0229**.
- After all tasks + the manual smoke, set SQ-0283 to `confirm` (save/persistence + unexercised VM path → user-verified), not `done`.
