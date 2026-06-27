# Glulx VM — Phase 2b: Strings, Memory Arrays, Heap, Search — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/gvm` (extends Phase 2a)
**Depends on:** Phase 2a (`2026-06-27-glulx-vm-phase-2a-design.md`) merged — 2b
builds on its `Memory`, `Machine`, decode/operand layer, the test asm helper,
and `GLULX_NOTES.md`.

## Goal

Round out the Glulx opcode set so real Inform 7 Glulx games can execute: full
**string decoding** (the decoding table + `streamstr` for compressed strings),
the **memory-array** opcodes, the **heap** (`malloc`/`mfree`), and the **search**
opcodes — plus the remaining stream/gestalt miscellany. After 2b the VM can run
substantial non-interactive programs; save/restore + acceleration are 2c, and
interactive Glk I/O is sub-project 3.

As in 2a, all opcode numbers / encodings / node-type tables come from the
**authoritative Glulx specification** (extend `crates/gvm/GLULX_NOTES.md`), not
from this prose.

## Design

### 1. String decoding + `streamstr`

A string address holds a 1-byte type: `0xE0` unencoded Latin-1 C-string,
`0xE2` unencoded Unicode (32-bit) C-string, `0xE1` compressed. `streamstr`
prints the string to the current iosys.

- `0xE0`/`0xE2`: walk bytes/words until the zero terminator, emitting chars.
- `0xE1` (compressed): bit-walk the **decoding table** (its address is the
  header `decode_table`, overridable via `setstringtbl`). The table is a tree of
  nodes; node types include: branch (`0x00`, left/right child addrs), string
  terminator (`0x01`), single char (`0x02`) / Unicode char (`0x04`), C-string
  (`0x03`) / Unicode C-string (`0x05`), and the indirect-reference nodes
  (`0x08`/`0x09` and the with-args `0x0A`/`0x0B`) that reference another string
  or **call a function** to produce a substring. Implement the full node set
  (the indirect/function nodes are used by Inform 7's printing veneer).
- Output respects iosys (glk → `Output`; null → discard; "filter" iosys, if a
  game selects it, calls the filter function per char — implement or record a
  diagnostic if unused by targets).

### 2. Memory-array opcodes

`aload`/`astore` (32-bit at `L1 + 4*L2`), `aloads`/`astores` (16-bit at
`L1 + 2*L2`), `aloadb`/`astoreb` (byte at `L1 + L2`), `aloadbit`/`astorebit`
(bit `L2` from byte address `L1`, signed bit index). All bounds-checked.

### 3. `mzero` / `mcopy`

`mzero(count, addr)` zeroes a span; `mcopy(count, from, to)` copies (handling
overlap per the spec — copy direction chosen so overlapping moves are correct).
RAM-only; bounds-checked.

### 4. Heap (`malloc` / `mfree`)

The heap occupies memory above the initial `ENDMEM`. On the first `malloc`, the
heap activates and the VM grows memory as needed; maintain a free-list of blocks
(per the spec's heap algorithm: allocation returns an address or 0 on failure;
`mfree` releases and coalesces). While the heap is active, `setmemsize` is
illegal (fault/diagnostic). The heap extent/free-list become part of the save
image in 2c. Tests cover alloc/free/realloc-via-free + the active-heap
`setmemsize` rule.

### 5. Search opcodes

`linearsearch`, `binarysearch`, `linkedsearch` with the documented operands
(key, key-size, start, struct-size, num-structs/key-offset, options bitfield:
key-indirect, zero-key-terminates, return-index). Each returns the matching
struct address (or index, or 0/-1) per options. Bounds/edge cases tested.

### 6. Miscellany

`gestalt` (report VM version + selector capabilities — return the values the
target games query, e.g. GlulxVersion, Unicode, MemCopy, MAlloc, etc.),
`getstringtbl`/`setstringtbl`, and any stream/iosys completeness not covered in
2a. `verify` may be stubbed to return success (full checksum verify is fine to
add here or 2c).

## Testing

Hand-assembled programs (extending the 2a asm helper):
- Decode a hand-built compressed `0xE1` string against a known decoding table →
  expected output; an `0xE0` and `0xE2` C-string; an indirect node that
  references another string; (if feasible) a function-call node producing a
  substring.
- Array ops: `aload`/`astore` and the 16/8/bit variants round-trip at computed
  addresses; out-of-range faults.
- `mzero`/`mcopy` including an overlapping copy.
- Heap: `malloc` returns distinct non-overlapping blocks; `mfree` + re-`malloc`
  reuses freed space; `setmemsize` while the heap is active faults.
- Search: `linearsearch`/`binarysearch`/`linkedsearch` find a present key and
  report absence; the return-index and zero-key-terminates options.
- `gestalt` returns the expected version/capability values.

## Out of scope (this phase)

- `save`/`restore`/`saveundo`/`restoreundo`/`protect`, `accelfunc`/`accelparam`,
  `random`/`setrandom` → **2c**.
- Floating point → later.
- Full Glk (windows, input, styles) → sub-project 3.
- The full `glulxercise` compliance run → 2c capstone (per the 2a spec).

## Global constraints

- `gvm` stays zero-dependency (std only). No new crates.
- Opcode numbers / string-node types / gestalt selectors transcribed from the
  authoritative Glulx spec (extend `GLULX_NOTES.md`).
- No panics on malformed input or faults — diagnostic + Quit.
- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace`
  green per task.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
