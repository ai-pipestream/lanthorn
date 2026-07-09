# Standard-Compliant In-Game `@save`/`@restore` (incl. v3) — Design

**Date:** 2026-07-08
**Status:** Approved for planning
**Quest:** SQ-0163
**Supersedes:** the v3-deferral decision in
`docs/superpowers/specs/2026-06-25-in-game-save-restore-design.md` (§ "v3 deferral").

## Goal

Make the game-driven `@save`/`@restore` opcode path fully standard-compliant
across **all** Z-machine versions — v3 (branch form), v4 (store form), and v5+
(`EXT` store form) — so that in-game saves round-trip with other standard
interpreters (Frotz, Bocfel, Lectrote). This closes the deferred v3 case as a
natural consequence of adopting the standard Quetzal program-counter convention.

This is one piece of the broader project goal: a fully compliant Z-machine and
Glulx implementation that supports the standard save format.

## Background: the two gaps

Two problems block standard in-game save/restore today. They share one root cause.

**1. Non-standard saved-PC convention (the root cause).** babelmap stores
`state.pc` *past the whole save instruction* and, on restore, reads the store
byte at `pc - 1` (`crates/zvm/src/quetzal.rs:13-18`,
`crates/zvm/src/cpu/exec.rs:1994-2003`). This is internally consistent — babelmap
can restore its own `@save`s — but it is **not** what Quetzal §5.8 specifies:

- §5.8.1 (v3, branch form): "The saved PC points to the one or two bytes which
  describe this branch."
- §5.8.2 (v4+, store form): "The saved PC points to the single byte describing
  where to store the result."

i.e. the standard PC points **at** the result descriptor and restore reads it
*forward*. Because babelmap's PC is offset by the descriptor length, its in-game
`@save` files do not interoperate with standard interpreters, and vice-versa.

**2. v3 in-game save/restore is auto-failed at the app layer.**
`run_until_input` (`crates/app/src/session.rs:506-519`) short-circuits
`version() <= 3` to `complete_save(false)` / `complete_restore_failure()` plus an
info message, instead of bubbling `SavePending`/`RestorePending` like v4+.

These are the same root cause: the `pc - 1` back-up works only because a v4+
store descriptor is always exactly 1 byte. A v3 branch descriptor is **1 or 2
bytes**, so it cannot be located by backing up from a post-instruction PC —
which is precisely why v3 was deferred. Fixing the PC convention removes the
ambiguity and makes v3 fall out for free.

## Non-goal / explicitly unchanged: "Save State" (save-anywhere)

babelmap's emulator-style host snapshot — Ctrl+S/Ctrl+R, the `.babelmap`
archive, auto-save/auto-resume, `/save`+`/load`, history/undo, and Glulx saves —
routes through the engine-neutral `Engine::save_state()` / `restore_state()`
(`crates/app/src/session.rs:809-823`) → `save_quetzal` / `restore_file`. This
path saves at a resumable instruction boundary and *resumes* on restore (it does
not complete a store/branch). It is **not** an `@save` opcode and is **not**
changed by this work.

The design keeps that path **byte-identical**: the host snapshot never sets
`pending_save`, so `save_quetzal` continues to serialize `state.pc` unchanged for
it. Only the in-game `@save`/`@restore` opcode path adopts the new convention.
(Terminology rename `save-game` → "Save State"/"Restore State" is tracked
separately as SQ-0227; save-anywhere cross-interpreter portability is out of
scope for SQ-0163.)

### Screen / window state is not part of Quetzal (and is unchanged here)

Per the standard, the Quetzal file carries **no** screen/window state (window
splits, cursor, styles, colors) — only dynamic memory, the call stack, and the
PC. On a bare-`.qzl` restore the game re-establishes its own screen (ZMSD §8).

babelmap preserves window configuration in a **separate** archive entry,
`screen.json` (`crates/app/src/archive.rs:294-300`), written by
`save_archive_meta` / `save_named` for **both** the Save State path *and* the
in-game `@save` path (in-game `@save` writes a `.babelmap` archive via
`save_named`, passing `machine.screen`, `crates/app/src/persist_files.rs:98,120`).
Bare Quetzal export (`save_game`, `persist_files.rs:203`) is the interop path and
carries no `screen.json`, by design.

This design changes **only** the Quetzal blob's PC convention; it does not touch
`screen.json` or any screen handling, so window-config save/restore is
unaffected. **Separate follow-up (not SQ-0163):** confirm the in-game `@restore`
host flow re-applies the archive's `screen.json` (the comment at `archive.rs:102`
names only "Ctrl+R / auto-load"); if a once-split game's upper window is not
redrawn after an in-game `@restore`, that is a host-side screen-reapply gap,
independent of save-format compliance.

## Design

### 1. `crates/zvm/src/cpu/decode.rs` — expose branch length

The branch decoder (~`decode.rs:403-420`) already distinguishes the 1-byte
(`b0 & 0x40`) from the 2-byte form. Surface that length so callers can locate and
skip a branch descriptor:

- Add `len: u8` (1 or 2) to `struct Branch` (`decode.rs:46`), set at decode time.
- Add a reusable `pub fn decode_branch_at(mem: &Memory, addr: u32) -> Branch`
  (factored from the inline decode) that reads the descriptor at an arbitrary
  address. Restore uses it to read the *original* `@save`'s branch from the
  restored image.

### 2. `crates/zvm/src/cpu/exec.rs` — capture the descriptor address; read it forward on restore

**Capture at save time.** `PendingSave` (`exec.rs:177`) gains
`descriptor_pc: u32`. When a save opcode fires, `state.pc` is post-instruction, so
the descriptor is the last bytes of the instruction:

- v3 `0OP:0x05` (branch): `descriptor_pc = state.pc - branch.len`.
- v4 `0OP:0x05` (store) and v5+ `EXT:0x00` (store): `descriptor_pc = state.pc - 1`.

Set this in all three save handlers (`exec.rs:789-805` and `exec.rs:1264-1287`).

**Write the descriptor PC only for the opcode path.** Add
`fn save_pc(&self) -> u32` returning
`self.pending_save.as_ref().map(|p| p.descriptor_pc).unwrap_or(self.state.pc)`.
In `crates/zvm/src/quetzal.rs`, `encode_ifhd` writes `machine.save_pc()` instead
of `machine.state.pc` (`quetzal.rs:115`). When no `@save` is pending (host Save
State, undo snapshots), this is exactly `state.pc` — unchanged.

**`complete_save` — unchanged** (`exec.rs:1928-1937`). It already applies the
opcode's own store/branch (store 1 / branch true) with `pc` at post-instruction,
which is correct.

**`complete_restore_success` — read the descriptor forward** (`exec.rs:1994`).
Replace the `pc - 1` store read with a version-dispatched forward completion,
after `restore_quetzal` has set `state.pc` to the file's descriptor PC:

- v3 (branch): `let br = decode_branch_at(&self.mem, self.state.pc);
  self.state.pc += br.len as u32; self.do_branch(Some(br), true);`
  (advancing PC to next_pc first is required because `do_branch` computes
  `pc + offset - 2` relative to next_pc, `exec.rs:1479`.)
- v4+ (store): `let sv = self.mem.read_byte(self.state.pc);
  self.do_store(Some(sv), 2); self.state.pc += 1;`

Then clear undo + `pending_restore_store` as today.

**`complete_restore_failure` — unchanged** (`exec.rs:1970`). v3 falls through at
the post-instruction PC; v4+ stores 0 into the captured `pending_restore_store`.

### 3. `crates/app/src/session.rs` — lift the v3 auto-fail

Remove the two `version() <= 3` short-circuits in `run_until_input`
(`session.rs:506-519`) so v3 returns `RunStop::SavePending` / `RestorePending`
exactly like v4+. Remove the now-dead `v3_failed` flag and its info-line
plumbing (`session.rs:353-405`, `498-524`), and the corresponding `info` message.

## Components / files

- `crates/zvm/src/cpu/decode.rs` — `Branch.len`; `decode_branch_at`.
- `crates/zvm/src/cpu/exec.rs` — `PendingSave.descriptor_pc`; set it in the three
  save handlers; `save_pc()`; forward-read `complete_restore_success`.
- `crates/zvm/src/quetzal.rs` — `encode_ifhd` writes `save_pc()`; update the
  save-PC doc comment (`quetzal.rs:13-18`).
- `crates/app/src/session.rs` — remove the v3 auto-fail short-circuit + `v3_failed`
  plumbing; replace the auto-fail test.

## Testing

**zvm unit (`exec.rs` / `quetzal.rs`):**
- v3 `@save` → `@restore` round trip (branch form): a synthetic v3 story whose
  `@save` branches on success; after a save+restore the branch is taken and
  execution resumes correctly.
- v4 and v5+ (`EXT`) in-game `@save`/`@restore` round trips **stay green** under
  the new convention — guards the changed path against regression.
- IFhd PC assertion: after an in-game `@save`, the serialized IFhd PC equals the
  descriptor address (store byte / first branch byte), not the post-instruction
  PC. A host Save State (`pending_save` absent) still serializes `state.pc`.

**app session (`session.rs`):**
- Replace `v3_ingame_save_still_auto_fails_with_info` with a test asserting a v3
  save/restore opcode now yields `PendingIo::Save` / `PendingIo::Restore`
  (no auto-fail, no info line).

**Real-game smoke (standing requirement — a VM feature needs a real game):**
- Drive `crates/zvm/tests/fixtures/minizork.z3` (v3) headless through a scripted
  `@save`/`@restore` round trip; assert the post-restore continuation transcript
  matches the un-saved continuation.

**Interop oracle (serves SQ-0158):**
- Unit check that the IFhd PC matches the standard offset. A full cross-interpreter
  check (restore a babelmap v3/v4/v5 `@save` in Frotz/Bocfel and vice-versa) is
  noted as an SQ-0158 follow-up, not required to land SQ-0163.

**Suite:** `cargo test -p zvm` and `cargo test -p app` stay green.

## Out of scope

- The "Save State"/"Restore State" user-facing rename (SQ-0227).
- Save-anywhere (host snapshot) cross-interpreter portability (SQ-0158 territory).
- Glulx save-format compliance (separate; Glulx has no `@save` opcode).
- Backward compatibility with pre-change `@save` files — old in-game saves are
  discarded (user will delete them); no migration path is provided.
