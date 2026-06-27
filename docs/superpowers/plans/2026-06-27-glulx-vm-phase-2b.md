# Glulx VM Phase 2b Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the Glulx opcode set on top of Phase 2a so real Inform 7 Glulx programs run: compressed-string decoding + full `streamstr`, the memory-array ops, `mzero`/`mcopy`, the `malloc`/`mfree` heap, the search opcodes, and `gestalt`/string-table miscellany.

**Spec:** `docs/superpowers/specs/2026-06-27-glulx-vm-phase-2b-design.md`

## Existing interfaces (Phase 2a — extend these)

- `crates/gvm/src/exec.rs`: `Machine`, `StepResult`, `enum Dest { Discard, Push, Mem(u32), Local(u32) }`, `read_operands(n_load, n_store) -> R<(Vec<u32>, Vec<Dest>)>`, `store(dest, v) -> R<()>`, `step_once`, and the central `fn execute(&mut self, opcode: u32) -> R<()>` `match opcode { … }` — **new opcodes are new arms here.** `R<T>` is the crate's fault-or-value result; faults carry a `String` diagnostic and Quit (never panic).
- `crates/gvm/src/memory.rs`: `Memory` with `read8/16/32`, `write8/16/32`/`store_mem`, `mem_size`/`set_mem_size`, `ramstart()`/`extstart()`/`endmem()`, `stack_size()`, `decode_table()`.
- `crates/gvm/src/io.rs`: `Output`/`BufferOutput`. `crates/gvm/src/asm.rs`: `#[cfg(test)]` image/program builder — extend it for the new tests (decode tables, arrays, etc.).
- `crates/gvm/GLULX_NOTES.md`: the transcribed spec tables — **extend it** with the string-node types, heap algorithm, search options, and gestalt selectors before implementing each (re-fetch from the Glulx spec at `https://www.eblong.com/zarf/glulx/glulx-spec.txt` as needed; the spec wins over this prose).

## Global Constraints

- `gvm` stays zero-dependency. 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green per task.
- No panics on malformed input/faults — diagnostic + Quit. Opcode numbers / node types / gestalt selectors from the authoritative Glulx spec.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push. Do not edit `TODO.md`.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

---

## Task 1: Compressed-string decoding + full `streamstr`

**Files:** `crates/gvm/src/exec.rs` (a `print_string(addr)` path + the decode-table walker), `crates/gvm/src/asm.rs` (test builder), `GLULX_NOTES.md`.

2a handles `0xE0` (Latin-1 C-string) and `0xE2` (Unicode C-string) and stubs `0xE1`. Implement `0xE1` compressed strings: bit-walk the decoding table (root at `decode_table()`, overridable by `setstringtbl`). Implement the full node-type set from the spec: branch (`0x00`), string-terminator (`0x01`), single char (`0x02`) / Unicode char (`0x04`), C-string (`0x03`) / Unicode C-string (`0x05`), and the indirect nodes (`0x08`/`0x09`) and indirect-with-args (`0x0A`/`0x0B`) — the indirect nodes reference another string or **call a function** whose output is the substring (used by Inform 7's printing veneer). Output respects iosys (glk → `Output`, null → discard).

- [ ] **Step 1:** Extend `GLULX_NOTES.md` with the string types + decode-table node table.
- [ ] **Step 2: Failing tests** (extend `asm.rs` to emit a decode table + strings): an `0xE1` compressed string with a known table decodes to the expected text; an indirect node (`0x08`) referencing an `0xE0` string prints it; `setstringtbl`/`getstringtbl` switch tables; `0xE0`/`0xE2` still work (no regression).
- [ ] **Step 3: Implement** the walker + `streamstr` arm + `setstringtbl`/`getstringtbl` opcodes (numbers from the spec).
- [ ] **Step 4: Run + commit** — `feat(gvm): compressed-string decoding + full streamstr`.

---

## Task 2: Memory-array opcodes + `mzero`/`mcopy`

**Files:** `crates/gvm/src/exec.rs`, `asm.rs`.

`aload`/`astore` (32-bit at `L1+4*L2`), `aloads`/`astores` (16-bit at `L1+2*L2`), `aloadb`/`astoreb` (byte at `L1+L2`), `aloadbit`/`astorebit` (bit `L2`, signed, from byte addr `L1`); `mzero(count, addr)`, `mcopy(count, from, to)` (overlap-safe direction). All bounds-checked via `Memory`.

- [ ] **Step 1: Failing tests:** each array op round-trips at a computed address (incl. negative bit index and the 16/8-bit variants); out-of-range faults; `mcopy` with overlapping ranges copies correctly; `mzero` clears.
- [ ] **Step 2: Implement** (opcode numbers from the spec).
- [ ] **Step 3: Run + commit** — `feat(gvm): memory-array opcodes + mzero/mcopy`.

---

## Task 3: Heap (`malloc` / `mfree`)

**Files:** `crates/gvm/src/exec.rs` (heap state on `Machine`), `asm.rs`, `GLULX_NOTES.md`.

The heap lives above the initial `ENDMEM`. On first `malloc`, the heap activates and memory grows as needed; maintain a free-list per the spec's algorithm (`malloc` returns an address or 0; `mfree` releases + coalesces). While the heap is active, `setmemsize` is illegal (fault/diagnostic). Track the heap start + block list on `Machine` (it joins the save image in 2c).

- [ ] **Step 1:** Note the heap algorithm in `GLULX_NOTES.md`.
- [ ] **Step 2: Failing tests:** `malloc` returns distinct, non-overlapping, in-range blocks; `mfree` then `malloc` reuses freed space; `setmemsize` while the heap is active faults; `malloc` that can't fit returns 0.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run + commit** — `feat(gvm): malloc/mfree heap`.

---

## Task 4: Search opcodes

**Files:** `crates/gvm/src/exec.rs`, `asm.rs`.

`linearsearch`, `binarysearch`, `linkedsearch` with the spec operands (key, key-size, start, struct-size, num-structs/key-offset, options bitfield: key-indirect, zero-key-terminates, return-index). Return the matching struct address / index / 0 / -1 per options.

- [ ] **Step 1: Failing tests:** each variant finds a present key and reports absence; the return-index option; key-indirect; zero-key-terminates; `binarysearch` on a sorted table; `linkedsearch` over a linked list.
- [ ] **Step 2: Implement.**
- [ ] **Step 3: Run + commit** — `feat(gvm): linear/binary/linked search opcodes`.

---

## Task 5: `gestalt` + miscellany

**Files:** `crates/gvm/src/exec.rs`, `asm.rs`, `GLULX_NOTES.md`.

`gestalt(selector, arg)` — return the values target games query: GlulxVersion (3.1.x), TerpVersion, ResizeMem, Undo, IOSystem (glk + null supported; filter not), Unicode, MemCopy, MAlloc, MAllocHeap, and others as the spec lists (report `1`/capability for what 2a/2b implement, `0` for not-yet — save/undo/accel report per 2c later). Plus `verify` (checksum; may return 0 = success) and any remaining stream/iosys completeness.

- [ ] **Step 1:** List the gestalt selectors + values in `GLULX_NOTES.md`.
- [ ] **Step 2: Failing tests:** `gestalt` returns the expected version + capability values for the implemented selectors and `0` for the unimplemented; `verify` returns success.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run + commit** — `feat(gvm): gestalt + verify + stream completeness`.

---

## Self-review checklist (run before final review)

- `gvm` still zero-dep; `GLULX_NOTES.md` matches the implemented string-nodes/heap/search/gestalt.
- Every opcode/selector/node value taken from the fetched Glulx spec, not this prose.
- No panics on malformed strings/tables, bad addresses, heap exhaustion, or search edge cases — all diagnostic + Quit.
- A program that prints a compressed string, does array + heap + search work, then quits, runs correctly via `gvm-cli`.
- 0 warnings; `cargo test --workspace` green.
