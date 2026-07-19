# Routine-Discovery Disassembly Cache — Implementation Plan (SQ-0418)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the heuristic backward-disassembly (`prev_instr` voting) with an exact, cached `address → unit` map built by routine discovery, so scrolling up in the debug disassembler shows real instruction boundaries and honest data markers instead of invented `op:2op`-style garbage.

**Architecture:** A new **pure, zero-dependency** zvm module (`crates/zvm/src/cpu/disasm_cache.rs`) owns the discovery algorithm, an ordered `Vec<Unit>` tiling the code region, binary-search navigation, and per-unit formatting (delegating instruction rendering to the existing `format_instr`/`format_instr_basic`/`format_instr_raw`). `GameSession` (app) owns one `DisasmCache` behind interior mutability, builds it lazily on first disasm access, routes `disassemble`/`disassemble_basic`/`disassemble_raw`/`next_instr`/`prev_instr` through it, and folds runtime-confirmed PCs into it each turn. The `Debugger` trait surface is **unchanged**.

**Tech Stack:** Rust. zvm crate stays zero-dependency (std only). Existing pieces reused verbatim: `zvm::cpu::decode::decode`, `zvm::cpu::disasm::{format_instr, format_instr_basic, format_instr_raw, Unpack, class_tag}`, `zvm::header` fields.

## Global Constraints

- **zvm stays zero-dependency** (std only). `cargo tree -p zvm --edges normal` must show no external deps. No new crate deps anywhere without explicit approval.
- **`Debugger` trait signatures do NOT change** — `disassemble(addr,lines)`, `disassemble_basic`, `disassemble_raw`, `next_instr(addr)`, `prev_instr(addr)` keep their exact signatures; only their bodies change.
- **Verify external constants against the crate, not memory.** Packing (routine packed→byte: v3 ×2; v4/5 ×4; v7 ×4 + 8·routines_offset; v8 ×8) and header offsets (0x04 high_mem_base, 0x06 initial_pc, 0x0E static_mem_base, 0x28/0x2A v7 offsets) must be taken from the existing `Unpack`/`header` code — reuse `Unpack::from_mem` and its routine-unpack method rather than re-deriving. Do not hard-code packing from recollection.
- **Format output for a real instruction must byte-for-byte match today's output** for the same `(addr, mode)` — Full via `format_instr`, Basic via `format_instr_basic`, Raw via `format_instr_raw`, each prefixed exactly as the current `disassemble*` functions do (`{:06x}  ` for Full/Basic, `{:06x}: ` for Raw). GameSession's `annotate_refs` still runs on top of Full output only.
- **Every commit** carries a `Quest: SQ-0418` trailer (the session auto-link hook is unreliable). Stage files explicitly by path; never `git add -A`/`-u`.
- **Never leak a debug-read memory fault into the VM** — every cache method that reads memory is called from a `&self` Debugger path that must end with `self.machine.mem.take_mem_fault()` (keep the existing drains).
- **Fixture-backed tests skip gracefully when the fixture is absent** (`minizork.z3` / `zork1` may not be present in CI) — guard with an early return + a printed skip note, matching the existing fixture-test pattern in the crate.

---

## File Structure

- **Create** `crates/zvm/src/cpu/disasm_cache.rs` — the pure cache: `Unit`, `CacheFmt`, `DisasmCache`, discovery, nav, formatting, confirmation. All unit + discovery tests live here.
- **Modify** `crates/zvm/src/cpu/mod.rs` — `pub mod disasm_cache;`.
- **Modify** `crates/zvm/src/cpu/disasm.rs` — make `decode` reachable to the new module (already `pub(crate)`/`pub`? verify) and expose `class_tag` to the sibling module if needed (`pub(crate)`).
- **Modify** `crates/app/src/session.rs` — `GameSession` holds `disasm_cache: RefCell<Option<DisasmCache>>`; route the five Debugger methods through it; add a `confirm_disasm` fold called per turn.
- **Modify** `crates/app/src/engine.rs` — no trait change; the `Dummy` double is unaffected (it already returns canned strings).
- **Test** all zvm-side logic in `disasm_cache.rs`; app-side wiring tested in `session.rs` (fixture-backed) and via the existing `debug_panel.rs` MockDbg nav tests (unchanged behavior).

---

## Phase 1 — RD-only cache, navigation, data units, mode-aware formatting

Delivers a correct cache for everything reachable by constant calls; data regions render as bytes in all three modes; nav is exact over the tiled units.

### Task 1: Unit model + region bounds + empty-cache skeleton

**Files:**
- Create: `crates/zvm/src/cpu/disasm_cache.rs`
- Modify: `crates/zvm/src/cpu/mod.rs` (add `pub mod disasm_cache;`)
- Test: in `disasm_cache.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, PartialEq, Eq, Debug)]
  pub enum Unit {
      Instr { addr: u32, next: u32 },
      RoutineHeader { addr: u32, nlocals: u8, first_instr: u32 },
      Data { addr: u32, len: u32 },
  }
  impl Unit { pub fn addr(&self) -> u32; pub fn end(&self) -> u32; } // end = next / first_instr end / addr+len

  #[derive(Clone, Copy, PartialEq, Eq, Debug)]
  pub enum CacheFmt { Full, Basic, Raw }

  pub struct DisasmCache {
      units: Vec<Unit>,            // sorted by addr(), tiling [region_start, region_end) with no gaps
      routines: std::collections::BTreeSet<u32>, // routine ENTRY (header) addresses
      region_start: u32,
      region_end: u32,
      version: u8,
      unpack: crate::cpu::disasm::Unpack,
  }
  pub fn code_region(mem: &Memory) -> (u32, u32); // (high_mem_base, mem.len()), clamped; region_start = min(high_mem_base, initial_pc)
  ```

- [ ] **Step 1: Write failing test** — `code_region` on a fixture header returns `(high_mem_base_or_less, mem.len())` with `start <= initial_pc` and `start < end`. `Unit::end()` returns `next` for Instr, `first_instr` for RoutineHeader (header occupies `[addr, first_instr)`), `addr+len` for Data.
- [ ] **Step 2: Run test, verify it fails** (`cargo test -p zvm disasm_cache`).
- [ ] **Step 3: Implement** `Unit`, `CacheFmt`, the `DisasmCache` struct (fields only, no discovery yet), `code_region`, and `Unit::{addr,end}`. Read header fields via `mem` accessors already used in `Unpack::from_mem` (grep for how `Unpack` reads 0x04/0x06 — reuse the same accessors; do not add new header parsing).
- [ ] **Step 4: Run test, verify pass.**
- [ ] **Step 5: Commit** (`git add crates/zvm/src/cpu/disasm_cache.rs crates/zvm/src/cpu/mod.rs`; message `feat(zvm): disasm cache — unit model + region bounds (SQ-0418)` + `Quest: SQ-0418`).

### Task 2: Recursive-descent routine discovery

**Files:**
- Modify: `crates/zvm/src/cpu/disasm_cache.rs`
- Test: same file

**Interfaces:**
- Produces (private): `fn discover_rd(mem: &Memory, version: u8, unpack: &Unpack, region: (u32,u32)) -> BTreeSet<u32>` — returns routine ENTRY (header) addresses. Also a helper `fn routine_first_instr(mem, entry, version) -> u32` (entry + 1 locals byte + (v<=4 ? nlocals*2 : 0)).

**Algorithm (RD to fixpoint):**
- Seed = `initial_pc` (byte address of the first instruction — NOT a routine header; in v3–8 header byte 0x06 is a byte address). Track a worklist of **first-instruction addresses** to decode-forward, and a set of discovered **routine entries**.
- Decoding forward from a first-instruction address: `let instr = decode(mem, pc, version)`; on any `call*` opcode whose routine operand is a **constant** (`Operand::Large`/`Operand::Small`, value ≠ 0), unpack via `unpack.unpack_routine(packed)` (verify the method name in `disasm.rs`) to a routine entry; if in-region and new, add to routines set and enqueue its `routine_first_instr` for decoding. Stop decoding a run at a `ret`/`rtrue`/`rfalse`/`ret_popped`/`jump`(unconditional)/`quit`/`print_ret` terminator, or when `next_pc` leaves the region, or a cap of N instructions.
- `call*` opcodes to recognize (verify against `mnemonic`/decode): 1OP `call_1s`(0x08)/`call_1n`(0x0F); 2OP `call_2s`(0x19)/`call_2n`(0x1A); VAR `call_vs`(0x00)/`call_vn`(0x19)/`call_vs2`(0x0C)/`call_vn2`(0x1A). The routine operand is operand index 0 in every case.

- [ ] **Step 1: Write failing test** (fixture `minizork.z3`, skip if absent): `discover_rd` returns a set that (a) is non-empty, (b) every entry is in-region and its locals byte ≤ 15, (c) contains the routine reached by the first constant `call` from `initial_pc` (compute that target independently in the test and assert membership).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** `discover_rd` + `routine_first_instr`. Reuse `decode` and `unpack.unpack_routine`. Cap per-routine decode at e.g. 4096 instructions (safety). Drain no fault here (pure `&Memory`; the app wrapper drains).
- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Commit** (`feat(zvm): disasm cache — recursive-descent routine discovery (SQ-0418)` + trailer).

### Task 3: Tile units from routines (RD-only build)

**Files:** Modify + test `crates/zvm/src/cpu/disasm_cache.rs`

**Interfaces:**
- Produces: `impl DisasmCache { pub fn build(mem: &Memory) -> DisasmCache; }` (RD-only for now; linear scan added in Phase 2). Internally: sort routine entries; for each routine emit a `RoutineHeader` unit then `Instr` units by linear decode from `first_instr` up to the next routine entry (or region_end); fill gaps (before first routine, between routines, after last) with `Data` units. **Units tile `[region_start, region_end)` with no gaps and are sorted by `addr()`.**

- [ ] **Step 1: Write failing test** (fixture; skip if absent): after `build`, assert (a) `units` is non-empty and strictly sorted by `addr()`, (b) units tile with no gap/overlap: `units[i].end() == units[i+1].addr()` for all i, `units[0].addr() == region_start`, `units.last().end() == region_end`, (c) at least one `RoutineHeader` and at least one `Instr` unit exist, (d) the `initial_pc` address falls inside an `Instr` unit (it is real code).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** `build`. A routine's instruction extent = linear decode from `first_instr` to `min(next_routine_entry, region_end)`; if a decode's `next_pc` would cross the boundary, truncate the last instruction's `next` at the boundary (boundary wins). Emit intervening `Data` units for any bytes between `region_start`/routine-end and the next routine entry.
- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Commit** (`feat(zvm): disasm cache — tile units from discovered routines (SQ-0418)` + trailer).

### Task 4: Binary-search navigation

**Files:** Modify + test `crates/zvm/src/cpu/disasm_cache.rs`

**Interfaces:**
- Produces:
  ```rust
  impl DisasmCache {
      fn unit_index_at(&self, addr: u32) -> usize; // index of unit containing addr; clamps to [0, len-1]
      pub fn next_addr(&self, addr: u32) -> u32;    // addr() of unit strictly after the one containing addr (clamps to region_end/last)
      pub fn prev_addr(&self, addr: u32) -> u32;    // addr() of unit strictly before the one containing addr (clamps to region_start)
  }
  ```

- [ ] **Step 1: Write failing test** (fixture; skip if absent): pick three consecutive units A,B,C. `next_addr(A.addr()) == B.addr()`; `next_addr(mid_of_A) == B.addr()`; `prev_addr(C.addr()) == B.addr()`; `prev_addr(B.addr()) == A.addr()`; `next_addr` of the last unit's addr clamps (== last addr or region_end sentinel — pick and assert one); `prev_addr(region_start)` clamps to `region_start`. Also **monotonic-nav across a routine boundary never invents an instruction**: from an `Instr` unit at a routine's last instruction, `next_addr` lands on the next `RoutineHeader`/`Data`, never a mid-data `Instr`.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** via `partition_point`/binary search on `units` by `addr()`.
- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Commit** (`feat(zvm): disasm cache — binary-search navigation (SQ-0418)` + trailer).

### Task 5: Mode-aware windowed formatting

**Files:** Modify + test `crates/zvm/src/cpu/disasm_cache.rs`; possibly widen `class_tag` visibility in `disasm.rs`.

**Interfaces:**
- Produces:
  ```rust
  impl DisasmCache {
      pub fn disassemble(&self, mem: &Memory, addr: u32, lines: usize, fmt: CacheFmt) -> Vec<String>;
  }
  ```
  Returns up to `lines` formatted rows starting at the unit at/after `addr`. Per unit:
  - `Instr { addr, .. }` → decode + format per `fmt`: Full `format!("{:06x}  {}", addr, format_instr(&instr, &self.unpack))`; Basic `format!("{:06x}  {}", addr, format_instr_basic(&instr, self.version))`; Raw same shape as `format_instr_raw` (bytes + `{:06x}: `).
  - `RoutineHeader { addr, nlocals, .. }` → `format!("{:06x}  ; routine, {} locals", addr, nlocals)` (all modes).
  - `Data { addr, len }` → one or more raw byte rows, 16 bytes/row, `format!("{:06x}  .byte {hex bytes}")`, each row counting as one line toward `lines` (a long Data run yields multiple rows; stop at `lines`).

- [ ] **Step 1: Write failing test** (fixture; skip if absent): `disassemble(mem, initial_pc, 4, CacheFmt::Full)` first row equals today's `zvm::cpu::disasm::disassemble(mem, initial_pc, version, 1)[0]` (byte-identical for the real instruction). A Data-region start yields a `.byte ` row, never an instruction. Raw/Basic first rows match their respective legacy formatter for the same real instruction. Requesting more `lines` than remaining units stops cleanly at `region_end`.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement.** For Raw byte extraction reuse the exact logic in `disassemble_raw` (bytes capped at 12, `truncated` flag). Make `class_tag`/`format_instr_raw` reachable (they're already `pub` in `disasm.rs` per current code — verify; widen to `pub(crate)` only if needed).
- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Commit** (`feat(zvm): disasm cache — mode-aware windowed formatting (SQ-0418)` + trailer).

### Task 6: Wire GameSession through the cache (behind unchanged trait)

**Files:**
- Modify: `crates/app/src/session.rs`
- Test: `session.rs` (fixture-backed) + confirm existing `debug_panel.rs` MockDbg nav tests still pass unchanged.

**Interfaces:**
- `GameSession` gains `disasm_cache: std::cell::RefCell<Option<zvm::cpu::disasm_cache::DisasmCache>>` (init `None` in the constructor). A private `fn with_cache<R>(&self, f: impl FnOnce(&DisasmCache) -> R) -> R` builds lazily (`DisasmCache::build(&self.machine.mem)`) and memoizes, then calls `f`.
- Route the five methods:
  - `disassemble(addr,n)` → `with_cache(|c| c.disassemble(mem, addr, n, CacheFmt::Full))`, then existing `annotate_refs` loop, then `take_mem_fault()`.
  - `disassemble_basic` → `CacheFmt::Basic` (no annotations), `take_mem_fault()`.
  - `disassemble_raw` → `CacheFmt::Raw`, `take_mem_fault()`.
  - `next_instr(addr)` → `with_cache(|c| c.next_addr(addr))`, `take_mem_fault()`.
  - `prev_instr(addr)` → `with_cache(|c| c.prev_addr(addr))`, `take_mem_fault()`.

- [ ] **Step 1: Write failing test** (fixture `minizork.z3`, skip if absent): through `GameSession as Debugger`, `disassemble(pc, 1)[0]` still starts with the PC's 6-hex address and matches the Full format for the instruction at PC; `prev_instr(next_instr(pc)) == pc` for the PC (a real boundary); scrolling `prev_instr` repeatedly from PC eventually reaches `region_start` and **never yields an `op:` mnemonic** in the rendered window above real code (assert no line in a backward window contains `op:2op`/`op:1op`).
- [ ] **Step 2: Run, verify fail** (before wiring; e.g. the no-garbage assertion fails today).
- [ ] **Step 3: Implement** the `RefCell` field, `with_cache`, and route all five methods. Keep every `take_mem_fault()`.
- [ ] **Step 4: Run, verify pass.** Also run the full `debug_panel.rs` suite — MockDbg nav tests use the mock (not GameSession), so they must be unaffected.
- [ ] **Step 5: Commit** (`feat(app): route GameSession disassembly through the routine-discovery cache (SQ-0418)` + trailer).

---

## Phase 2 — Linear-scan augmentation (indirectly-called routines)

### Task 7: Validated linear-scan header discovery

**Files:** Modify + test `crates/zvm/src/cpu/disasm_cache.rs`

**Interfaces:**
- Produces (private): `fn discover_linear(mem, version, region, known: &BTreeSet<u32>) -> BTreeSet<u32>` — scans `region` for candidate routine headers not already in `known`. A candidate at `p` is accepted iff: (a) locals byte `≤ 15`; (b) decoding from `routine_first_instr(p)` proceeds cleanly (every opcode assigned — no `op:` fallback mnemonic; `next_pc` strictly advances; no read past region) up to the next known boundary or a clean terminator; (c) it does not overlap a higher-confidence RD routine (RD wins). `build` unions RD + linear results before tiling.

- [ ] **Step 1: Write failing test** (fixture; skip if absent): `discover_linear` finds ≥1 routine that pure `discover_rd` misses (assert `linear_only` non-empty for minizork — an indirectly-called routine), and **rejects a non-routine**: pick a byte address known to be data (e.g. inside the dictionary) whose locals byte happens to be ≤15, assert it is NOT accepted because its decode doesn't validate cleanly.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement.** "Assigned opcode" check: `mnemonic(...)` does not start with `"op:"` (that's the existing unknown-opcode sentinel). Tune the clean-decode window to reach the next known boundary. Err toward rejection (classify ambiguous bytes as Data).
- [ ] **Step 4: Run, verify pass.** Re-run the full Phase-1 tiling/nav tests — they must still hold with the larger routine set.
- [ ] **Step 5: Commit** (`feat(zvm): disasm cache — validated linear-scan routine discovery (SQ-0418)` + trailer).

---

## Phase 3 — Runtime confirmation (self-healing)

### Task 8: Confirm-PC / confirm-routine repair on the cache

**Files:** Modify + test `crates/zvm/src/cpu/disasm_cache.rs`

**Interfaces:**
- Produces:
  ```rust
  impl DisasmCache {
      /// A confirmed instruction start. If `pc` disagrees with the cache
      /// (lands in a Data unit or mid-instruction), re-anchor at `pc` and
      /// re-decode forward, splitting/replacing affected units locally. Returns
      /// true if the cache changed.
      pub fn confirm_pc(&mut self, mem: &Memory, pc: u32) -> bool;
      /// A confirmed routine ENTRY (strongest signal). Promotes `entry` to a
      /// routine header and re-aligns that routine forward. Returns true if changed.
      pub fn confirm_routine(&mut self, mem: &Memory, entry: u32) -> bool;
  }
  ```
  Repair is **local** (bounded re-decode of the affected span to the next unit boundary), not a full rebuild. A confirmed PC already at an `Instr` boundary is a no-op (returns false).

- [ ] **Step 1: Write failing test** (synthetic or fixture): (a) seed a cache, force a span to be `Data` (e.g. build then manually mark a known-code routine's region as data via a test-only helper, or pick a routine the linear scan is configured to miss), feed `confirm_pc` a real instruction start inside it, assert the unit at that address becomes `Instr` with `addr == pc` and tiling still holds. (b) `confirm_pc` on an address already at an `Instr` boundary returns false and leaves units unchanged. (c) `confirm_routine(entry)` produces a `RoutineHeader` unit at `entry` and an `Instr` unit at its `first_instr`, tiling preserved.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** local repair: find the unit containing the confirmed address; re-decode forward from the confirmed anchor to the end of that unit (or next confirmed/known boundary); replace the overlapped units with `[Data before?][Instr…]` preserving the no-gap tiling invariant. Keep an assertion (debug) that tiling holds after repair.
- [ ] **Step 4: Run, verify pass.**
- [ ] **Step 5: Commit** (`feat(zvm): disasm cache — runtime confirmation / self-healing repair (SQ-0418)` + trailer).

### Task 9: Fold confirmed PCs into the cache each turn (GameSession)

**Files:**
- Modify: `crates/app/src/session.rs`
- Test: `session.rs` (fixture-backed)

**Interfaces:**
- `GameSession` gains `fn confirm_disasm(&self)` (or folds into the existing per-turn refresh path): if the cache is built, for each `func_addr` in `self.machine.state.frames` call `confirm_routine`; for the parked PC (`self.machine.state.pc`) and each addr in `self.machine.exec_pcs` call `confirm_pc`. Called once per turn where the panel already refreshes (find the existing per-turn hook — grep where `exec_pcs` is consumed / where the debug snapshot refreshes). If the cache is `None` (never opened), do nothing (cheap).
- Accumulate confirmed PCs so repair is idempotent (a `confirm_*` on an already-correct boundary is a no-op).

- [ ] **Step 1: Write failing test** (fixture; skip if absent): drive a few turns through `GameSession`, call the confirm fold, then assert every executed PC from `exec_pcs` sits exactly at an `Instr` unit boundary in the cache (`next_instr(prev_instr(pc)) == pc`), and every `func_addr` is a `RoutineHeader` entry. Assert the fold is a no-op (no change / stable) on a second identical call.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** `confirm_disasm` and call it from the per-turn refresh. Only borrow the cache mutably here; ensure no double-borrow with `with_cache` (this path takes `&mut` via `borrow_mut`, the read path takes `&` — they are not concurrent).
- [ ] **Step 4: Run, verify pass.** Full `cargo test -p zvm -p app`.
- [ ] **Step 5: Commit** (`feat(app): fold runtime-confirmed PCs into the disasm cache each turn (SQ-0418)` + trailer).

---

## Verify (every task) & final gate

- `cargo build -p zvm -p app` clean.
- `cargo test -p zvm -p app` FULL green (report counts).
- `cargo clippy -p zvm -p app --all-targets -- -D warnings` clean.
- `cargo tree -p zvm --edges normal` — zero external deps.
- **Manual smoke (user, TTY):** open `/debug` on zork1.z5, scroll the disassembly UP across the parked PC into the region above the current routine — confirm real instructions / `; routine` markers / `.byte` data rows, and NO `op:2op`-style invented instructions. Play a few turns and confirm regions you executed stay clean. (This is the correctness payoff; mark SQ-0418 `confirm` until smoked.)

## Self-Review notes

- **Spec coverage:** Approach §Discovery(1–5)→Tasks 2,3,7; §Cache/Unit model→Task 1,3; §Navigation→Task 4; §disassemble windowing→Task 5; §Data/string handling→Task 3,5 (strings-as-data is the v1 non-goal, honored); §Runtime confirmation→Tasks 8,9; §Debugger trait impact "None"→Task 6 (signatures unchanged); §gvm generalization→satisfied by putting pure logic in a standalone zvm module.
- **Non-goals honored:** no string decoding (Data), no cache invalidation (built once + local repair), no gvm impl.
- **Type consistency:** `Unit`, `CacheFmt`, `DisasmCache`, `build(mem)`, `disassemble(mem,addr,lines,fmt)`, `next_addr`/`prev_addr`, `confirm_pc`/`confirm_routine` names are used identically across all tasks.
- **Open risk:** the exact `Unpack` routine-unpack method name and header accessor names must be confirmed from `disasm.rs`/`header.rs` at Task 1–2 (the plan says "verify the method name") — a fast grep, not a design change.
