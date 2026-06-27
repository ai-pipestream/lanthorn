# Glulx VM — Phase 2c: Save/Restore Core, Undo, Protect, Accel, Random — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/gvm` (extends Phases 2a/2b)
**Depends on:** Phase 2b merged.

## Goal

Finish the headless-testable parts of the Glulx VM: the **save/restore
serialization core**, the **undo** opcodes, **protect**, the **acceleration**
opcodes (parameter storage; interception optional), and the **PRNG**. After 2c
the VM is functionally complete for non-interactive execution; the only
remaining VM-adjacent work is the file/stream wiring of `@save`/`@restore` and
the interactive surface, both of which belong to **sub-project 3 (Glk I/O)**.

Opcode numbers, the save-file format, gestalt selectors, and the accel scheme
come from the **authoritative Glulx specification** (extend `GLULX_NOTES.md`).

## Scope boundary (important)

- **In 2c:** the serialization *core* (`save_state`/`restore_state` as `Machine`
  methods over Glulx-Quetzal bytes), `saveundo`/`restoreundo` (in-memory, fully
  working), `protect`, `accelfunc`/`accelparam` (storage + optional
  interception), `random`/`setrandom`, `verify` (real checksum), and the gestalt
  updates for these capabilities.
- **Deferred to sub-project 3 (Glk):** the `@save`/`@restore` *opcodes*
  themselves write/read a **Glk stream**; without Glk streams there is no real
  destination, so the opcodes are wired then. The serialization core built here
  is exactly what they will call.
- **Deferred to sub-project 3:** the full **glulxercise** compliance run (needs
  Glk windows + input). At 2c, glulxercise can't be driven end to end; the
  per-opcode unit tests remain the conformance evidence.

## Design

### 1. Save/restore serialization core

`Machine::save_state() -> Vec<u8>` and `restore_state(&[u8]) -> Result<(),
GError>` serialize/deserialize the VM's mutable state in the **Glulx Quetzal**
format (IFF `FORM IFZS`: an `IFhd`-style identity chunk, `CMem` compressed (or
`UMem` uncompressed) RAM image `[RAMSTART, ENDMEM)`, a `Stks` stack chunk, and a
`MAll` heap-state chunk). Requirements:
- `Memory` retains the **original loaded RAM image** (for `CMem` XOR/RLE
  compression and for the restore reset); add an accessor if needed.
- Serialized state = RAM `[RAMSTART, mem_size)`, the stack bytes `[0, sp)` + `sp`
  + `fp`, `pc`, `iosys_mode`/`rock`, `cur_stringtbl`, `heap_start` +
  `heap_blocks`, and the protect range (§3).
- `restore_state` resets RAM to the original image, applies the saved diff,
  rebuilds the stack/heap/registers, recomputes the frame cache, and leaves
  `pc` at the saved continuation. The **protected** range (§3) is preserved
  across restore, not overwritten.
- Round-trip tested directly (no Glk needed): mutate state → `save_state` →
  mutate more → `restore_state` → assert the original state is back.

### 2. Undo (`saveundo` / `restoreundo`)

In-memory undo using the same core: `saveundo` pushes a state snapshot onto a
bounded undo stack (a small cap, e.g. matching zvm's default), storing the value
at the call stub's destination as success; `restoreundo` pops and restores,
writing `-1` per the spec to the *restored* destination, or failing (storing 0)
when the stack is empty. Fully working and tested in 2c (no streams involved).

### 3. `protect`

`protect(addr, len)` marks a RAM range that must be preserved across `restore`,
`restoreundo`, and `restart` (its bytes are not overwritten by the restored
image). Store the `(addr, len)` on `Machine`; honor it in `restore_state` and
`restoreundo`. `protect(_, 0)` clears it. Tested: a protected byte survives a
restore that would otherwise change it.

### 4. Acceleration (`accelfunc` / `accelparam`)

`accelfunc(L1, L2)` assigns accelerated-function number `L1` to the function at
address `L2` (0 = unassign); `accelparam(L1, L2)` sets accel parameter `L1` to
`L2`. Acceleration is purely a **speed optimization** — Inform 7 games run
correctly without it. 2c **stores** the assignments/params on `Machine` (so the
opcodes succeed and state round-trips) and may optionally intercept the
well-known accelerated functions (1–13) for speed; if not intercepting, the
real veneer functions run normally. Tested: the opcodes accept and store
assignments without error; (optional) an intercepted function returns the same
value as the veneer.

### 5. PRNG (`random` / `setrandom`)

A small deterministic PRNG on `Machine` (e.g. xorshift). `random(L1)` returns a
value in `[0, L1)` for `L1 > 0`, in `(L1, 0]` for `L1 < 0`, and any 32-bit value
for `L1 == 0` (per spec). `setrandom(seed)` seeds it; `setrandom(0)` reseeds
from a fixed deterministic default (true entropy seeding needs a dependency or
`std::time` and is deferred — note it). Tested: a known seed yields a known,
reproducible sequence and the range bounds hold.

### 6. `verify` + gestalt

Implement `verify` as a real image checksum (return 0 on success). Update
`gestalt` to report the now-supported capabilities (e.g. `Undo`, `MemCopy`,
`MAlloc`, `MAllocHeap` as appropriate) truthfully; `accelfunc`/`accelparam`
support per whether interception is implemented.

## Testing

Hand-assembled programs + direct method calls (extending `asm.rs`):
- `save_state`/`restore_state` round-trip restores RAM/stack/heap/registers;
  a protected byte survives.
- `saveundo` then mutate then `restoreundo` restores prior state; empty-stack
  `restoreundo` fails (stores 0); the dest-write convention (`-1` on restore).
- `protect` preserves its range across restore/undo; `protect(_,0)` clears.
- `accelfunc`/`accelparam` accept assignments; state round-trips with them.
- `random` range bounds for positive/negative/zero args; `setrandom(seed)` →
  reproducible sequence.
- `verify` returns success on an intact image; `gestalt` reports the right caps.

## Out of scope

- `@save`/`@restore` opcode file/stream wiring → sub-project 3 (Glk streams).
- Glk windows/streams/input and the glulxercise capstone → sub-project 3.
- Floating-point opcodes → later (gestalt reports unsupported).
- Real-entropy PRNG seeding (`setrandom(0)`) → later (deterministic default now).

## Global constraints

- `gvm` stays zero-dependency (std only). No new crates.
- Save format / opcode numbers / gestalt selectors from the authoritative Glulx
  spec (extend `GLULX_NOTES.md`).
- No panics on malformed save data or faults — diagnostic + Quit / `GError`.
- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace`
  green per task.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
