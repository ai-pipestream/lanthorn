# Glulx VM Phase 2c Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the headless-testable Glulx VM: save/restore serialization core, undo opcodes, protect, acceleration storage, PRNG, real verify + gestalt updates. (`@save`/`@restore` stream wiring and glulxercise are sub-project 3.)

**Spec:** `docs/superpowers/specs/2026-06-27-glulx-vm-phase-2c-design.md`

## Existing interfaces (extend these)

- `crates/gvm/src/exec.rs`: `Machine` (fields `mem, stack, sp, fp, pc, iosys_mode, iosys_rock, cur_stringtbl, heap_start, heap_blocks, diagnostics, halted`, frame cache), `execute(opcode)` match (new arms here), `read_operands`/`store`/`Dest`, `call_function`, the frame cache rebuild. Add new `Machine` fields for the undo stack, protect range, accel maps, and PRNG state.
- `crates/gvm/src/memory.rs`: `Memory` (read/write, `mem_size`/`set_mem_size`, `ramstart`/`extstart`/`endmem`). May need an accessor for the **original loaded RAM image** (for CMem compression + restore reset) — add one.
- `crates/gvm/src/asm.rs` (`#[cfg(test)]`): extend for the new tests.
- `crates/gvm/GLULX_NOTES.md`: extend with the save-file format (Glulx spec §"save format"), accel scheme, and gestalt selectors — re-fetch from `https://www.eblong.com/zarf/glulx/glulx-spec.txt` (the spec wins over this prose).

## Global Constraints

- `gvm` stays zero-dependency. 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green per task.
- No panics on malformed save data / faults — diagnostic + Quit / `GError`. Save format + opcode numbers + gestalt selectors from the authoritative Glulx spec.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push. Do not edit `TODO.md`.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

---

## Task 1: Save/restore serialization core

**Files:** `crates/gvm/src/exec.rs`, `crates/gvm/src/memory.rs`, `asm.rs`, `GLULX_NOTES.md`.

**Interfaces:** `Machine::save_state(&self) -> Vec<u8>`, `Machine::restore_state(&mut self, &[u8]) -> Result<(), GError>`.

- [ ] **Step 1:** Note the Glulx Quetzal save format (FORM IFZS: identity, `CMem`/`UMem` RAM, `Stks` stack, `MAll` heap) in `GLULX_NOTES.md`; ensure `Memory` retains the original RAM image (add an accessor).
- [ ] **Step 2: Failing tests:** mutate RAM + stack + heap + registers, `save_state`, mutate further, `restore_state`, assert the saved state is restored exactly; a corrupt/truncated save → `GError` (no panic); `CMem` round-trips against the original image.
- [ ] **Step 3: Implement** `save_state`/`restore_state` (RAM `[RAMSTART, mem_size)`, stack `[0,sp)` + `sp`/`fp`/`pc`, iosys, stringtbl, heap, protect range; restore resets RAM to original then applies the diff and rebuilds the frame cache).
- [ ] **Step 4: Run + commit** — `feat(gvm): Glulx save/restore serialization core`.

---

## Task 2: Undo opcodes (`saveundo` / `restoreundo`)

**Files:** `crates/gvm/src/exec.rs`, `asm.rs`.

- [ ] **Step 1: Failing tests:** `saveundo` then mutate then `restoreundo` restores prior state and writes `-1` to the restored destination; `saveundo` stores `0` (success) at its destination; `restoreundo` on an empty stack stores `1` (failure) per the spec and leaves state unchanged; the undo stack is bounded (oldest dropped past the cap).
- [ ] **Step 2: Implement** a bounded in-memory undo stack of `save_state()` snapshots; `saveundo`/`restoreundo` opcodes (numbers from the spec) using the Task 1 core; honor the spec's destination-write convention exactly.
- [ ] **Step 3: Run + commit** — `feat(gvm): saveundo/restoreundo`.

---

## Task 3: `protect`

**Files:** `crates/gvm/src/exec.rs`, `asm.rs`.

- [ ] **Step 1: Failing tests:** set `protect(addr, len)`, change those bytes, `restore_state`/`restoreundo` — the protected bytes keep their *current* values (not the restored image's); `protect(_, 0)` clears protection; protection survives in the saved state.
- [ ] **Step 2: Implement** the `protect` opcode + a `(addr, len)` field on `Machine`, honored in `restore_state`/`restoreundo`.
- [ ] **Step 3: Run + commit** — `feat(gvm): protect (preserve a RAM range across restore)`.

---

## Task 4: Accel + random + verify + gestalt

**Files:** `crates/gvm/src/exec.rs`, `asm.rs`, `GLULX_NOTES.md`.

- [ ] **Step 1: Failing tests:** `accelfunc`/`accelparam` accept and store assignments without error (state round-trips with them); `random(L1)` honors range bounds for positive/negative/zero `L1`; `setrandom(seed)` yields a reproducible sequence; `verify` returns `0` (success) on an intact image; `gestalt` reports the now-supported capabilities truthfully (Undo, MemCopy, MAlloc, …) and `0` for still-unsupported (float, accel-if-not-intercepted).
- [ ] **Step 2: Implement** `accelfunc`/`accelparam` (store maps on `Machine`; acceleration interception optional — if omitted, the real veneer runs and gestalt reports accel unsupported), the xorshift PRNG (`random`/`setrandom`; `setrandom(0)` → fixed deterministic reseed, true entropy deferred), real `verify` checksum, and the gestalt updates.
- [ ] **Step 3: Run + commit** — `feat(gvm): accel storage, PRNG, verify, gestalt updates`.

---

## Self-review checklist (run before final review)

- `gvm` still zero-dep; `GLULX_NOTES.md` matches the save format + gestalt selectors.
- Save/restore round-trips exactly (RAM/stack/heap/registers); corrupt saves error without panicking; protect preserved across restore/undo.
- Undo dest-write convention matches the spec; bounded stack.
- PRNG range bounds hold; deterministic for a known seed.
- `@save`/`@restore` opcodes and glulxercise are NOT in scope (sub-project 3) — not added here.
- 0 warnings; `cargo test --workspace` green.
