# Glulx VM Phase 2a Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A zero-dep `gvm` crate (Glulx VM foundation) + a `gvm-cli` headless runner that can load a Glulx image and execute a non-interactive program (arithmetic/branch/stack/calls + text output) to completion.

**Architecture:** Mirror `zvm`: `Memory` over the image, a `Machine` with `with_output(...)` and `step() -> StepResult`, a pluggable `Output` trait. Validate with hand-assembled Glulx programs.

**Spec:** `docs/superpowers/specs/2026-06-27-glulx-vm-phase-2a-design.md`

## AUTHORITATIVE SOURCE — read this first

The exact header layout, **opcode numbers**, operand addressing-mode nibbles,
branch convention, and the function/call-frame format MUST come from the
**Glulx specification** (Andrew Plotkin), not from prose. Before writing code,
**fetch the Glulx spec** (WebFetch — search "Glulx specification" / the
eblong.com Glulx spec) and extract, into a short notes file
(`crates/gvm/GLULX_NOTES.md`, committed in Task 1) you then implement against:
- the 36-byte header field offsets + memory map (RAMSTART/EXTSTART/ENDMEM rules);
- the operand addressing-mode nibble table (load + store semantics);
- the variable-length opcode-number encoding (1/2/4-byte);
- the function type bytes (`0xC0`/`0xC1`), locals-format list, call-frame layout,
  and the call-stub/destination format;
- the branch-offset convention (offset 0 → return 0, 1 → return 1, else relative);
- the exact opcode numbers for every opcode in the 2a subset (listed by NAME
  below — look each up in the spec's opcode table);
- `setiosys`/`getiosys` modes and the `@glk` dispatch (only the put-char/buffer
  selectors are needed in 2a).

Where this plan's prose and the spec disagree, **the spec wins.**

## Global Constraints

- `gvm` is zero-dependency (std only); `gvm-cli` depends only on `gvm` (+ `blorb` in Task 7).
- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace` green after every task.
- No panics on malformed images or runtime faults — record a `diagnostics` line and Quit.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Do not edit `TODO.md`.

## Test scaffolding (used by every task)

Build a `#[cfg(test)]` Glulx-image **assembler** in `gvm` (e.g. `asm.rs`) that
writes a valid header + a function (type byte + locals format) + a slice of
hand-encoded instructions, so tests can construct tiny programs. Grow it as
opcodes/modes are added. Keep it accurate to the spec's encoding.

---

## Task 1: `gvm` crate scaffold + spec notes + `Memory`/header loader

**Files:** Create `crates/gvm/Cargo.toml`, `crates/gvm/src/lib.rs`, `crates/gvm/src/io.rs`, `crates/gvm/src/memory.rs`, `crates/gvm/src/error.rs`, `crates/gvm/GLULX_NOTES.md`; modify root `Cargo.toml` (workspace members).

**Interfaces:** `GError`, `Output`/`BufferOutput`, `Memory::new(Vec<u8>) -> Result<Memory, GError>`, `read8/16/32`, `write8/16/32`, `mem_size`/`set_mem_size`, header field accessors (`ramstart`, `extstart`, `endmem`, `stack_size`, `start_func`, `decode_table`).

- [ ] **Step 1: Fetch the Glulx spec** and write `crates/gvm/GLULX_NOTES.md` with the header layout + memory map (and the other tables, for later tasks).
- [ ] **Step 2: Scaffold** the crate (zero-dep Cargo.toml; add to workspace members) and `io.rs` (the `Output` trait + `BufferOutput`, identical shape to `zvm::io`).
- [ ] **Step 3: Failing tests** in `memory.rs`: a hand-built valid header loads and exposes the right fields; bad magic → `GError::BadMagic`; unknown major version → `GError::UnsupportedVersion`; `[EXTSTART,ENDMEM)` reads back zero; `set_mem_size` grows (new space zeroed) and shrinks, clamped to the original `ENDMEM` floor and 256-aligned; out-of-range `read32` is a checked fault (no panic).
- [ ] **Step 4: Implement** `Memory` (big-endian accessors over a `Vec<u8>` sized to `ENDMEM`; ROM writes below `RAMSTART` record a diagnostic/no-op) and the header parse.
- [ ] **Step 5: Run + commit** — `feat(gvm): crate scaffold, Output trait, Memory + Glulx header loader`.

---

## Task 2: Stack, call frames, calling convention

**Files:** Modify `crates/gvm/src/exec.rs` (new), `lib.rs`.

**Interfaces:** `Machine { mem, stack, sp, fp, pc, … }`, `Machine::with_output(Memory, Box<dyn Output>) -> Machine` (pushes the initial frame for the start function), internal `call_function(addr, args, dest)` / `return_value(v)`.

- [ ] **Step 1: Failing tests** (using the asm helper): calling a `0xC1` function copies args into locals (extra locals zeroed, extra args dropped); the return value is written to the call stub's destination (discard / stack / memory / local); a `0xC0` function receives args + count on its stack; returning from the outermost frame yields `StepResult::Quit`.
- [ ] **Step 2–3: Implement** the byte-addressed stack, the documented call-frame layout (frame len, locals-format, locals, operand-stack region), `call_function` (build frame per the function type), and `return_value` (pop frame, write per the saved stub, restore pc/fp; outer → Quit). `with_output` builds the start frame and points `pc` into the start function.
- [ ] **Step 4: Run + commit** — `feat(gvm): stack, call frames, and the Glulx calling convention`.

---

## Task 3: Instruction decode + operand addressing modes

**Files:** Modify `crates/gvm/src/exec.rs`.

**Interfaces:** internal `decode_opcode() -> u32`, `read_operands(n_load, n_store) -> (Vec<u32> loads, Vec<Dest> stores)`, `store(Dest, u32)`.

- [ ] **Step 1: Failing tests:** the 1/2/4-byte opcode-number encoding decodes correctly; each operand mode resolves for LOAD (constant 0/8/16/32 sign-extended, contents-of-address 8/16/32, stack pop, local 8/16/32, RAM-relative 8/16/32) and STORE (discard, push, memory, local) — drive each via a tiny program that copies a value through the mode and outputs/returns it.
- [ ] **Step 2–3: Implement** decode + the mode nibble table (transcribed from `GLULX_NOTES.md`).
- [ ] **Step 4: Run + commit** — `feat(gvm): opcode + operand-mode decoding`.

---

## Task 4: Arithmetic/bit + branch opcodes

**Files:** Modify `crates/gvm/src/exec.rs`.

- [ ] **Step 1: Failing tests** per opcode (look up exact numbers in the spec table): `add/sub/mul/div/mod/neg` over positive/negative/overflow/truncation cases; div/mod by zero faults (diagnostic + Quit); `bitand/bitor/bitxor/bitnot`; `shiftl/sshiftr/ushiftr` (incl. shift ≥ 32). Branches `jump/jz/jnz/jeq/jne/jlt/jge/jgt/jle/jltu/jgeu/jgtu/jleu` taken/not-taken, plus the offset-`0`→return-0 / `1`→return-1 convention and the `pc + offset - 2` math.
- [ ] **Step 2–3: Implement** (32-bit two's-complement; signed vs unsigned compares).
- [ ] **Step 4: Run + commit** — `feat(gvm): arithmetic, bitwise, and branch opcodes`.

---

## Task 5: Stack ops, copy, memsize, call variants

**Files:** Modify `crates/gvm/src/exec.rs`.

- [ ] **Step 1: Failing tests:** `copy/copys/copyb` (truncated copies); `stkcount/stkpeek/stkswap/stkroll/stkcopy` over a known stack; `getmemsize/setmemsize`; `call` (args from stack + count), `callf/callfi/callfii/callfiii`, `return`, `tailcall` — exact opcode numbers from the spec.
- [ ] **Step 2–3: Implement.**
- [ ] **Step 4: Run + commit** — `feat(gvm): stack ops, copy, memsize, and call variants`.

---

## Task 6: Output (iosys + stream ops + minimal @glk) + run loop

**Files:** Modify `crates/gvm/src/exec.rs`.

**Interfaces:** `StepResult { Continue, Quit }`, `Machine::step()`, `Machine::run()`, `machine.diagnostics: Vec<String>`.

- [ ] **Step 1: Failing tests:** `setiosys(2,0)` then `streamnum`/`streamchar` (and `@glk glk_put_char`/`glk_put_buffer`) emit the expected text into a `BufferOutput`; iosys `0` discards; `nop`; `quit` → `StepResult::Quit`; an unknown/illegal opcode and a memory fault record a `diagnostics` line and Quit (no panic). A small end-to-end program (arith + branch + call + output + quit) yields the right transcript via `run()`.
- [ ] **Step 2–3: Implement** `get/setiosys`, `streamchar`/`streamnum`/`streamunichar` (route to `Output` under the glk iosys; `streamstr` of a plain `0xE0` C-string prints raw bytes, compressed `0xE1` deferred to 2b with a diagnostic), and a minimal `@glk` handling the put-char/buffer family (+ returning 0 for the couple of setup selectors a trivial program calls). The `step()`/`run()` loop and fault→diagnostic+Quit.
- [ ] **Step 4: Run + commit** — `feat(gvm): iosys/stream output, minimal @glk, run loop`.

---

## Task 7: `gvm-cli` headless runner

**Files:** Create `crates/gvm-cli/Cargo.toml`, `crates/gvm-cli/src/main.rs`; modify root `Cargo.toml`.

- [ ] **Step 1:** Scaffold `gvm-cli` (depends on `gvm` + `blorb`). Read the file: if `blorb::Blorb::is_blorb`, extract the `GLUL` executable (error clearly if the Blorb's exec is `ZCode`); else treat the bytes as a raw `.ulx`. `Memory::new`, `Machine::with_output(StdoutOutput)`, `run()`; stream output to stdout, `diagnostics` to stderr.
- [ ] **Step 2:** A test/smoke: a hand-assembled program file runs to completion and prints the expected output. (`glulxercise` is NOT wired here — it lands as the 2c capstone per the spec.)
- [ ] **Step 3: Run + commit** — `feat(gvm-cli): headless Glulx runner (.ulx and .gblorb)`.

---

## Self-review checklist (run before final review)

- `gvm` is zero-dep (`cargo tree -p gvm` shows only std); `GLULX_NOTES.md` matches the implemented header/modes/opcodes.
- Every opcode/mode/header value was taken from the fetched Glulx spec, not the plan's prose.
- No panics on malformed images, bad opcodes, faults, div-by-zero — all record a diagnostic and Quit.
- An end-to-end hand-assembled program (arith + branch + call + output + quit) runs correctly via `gvm` and `gvm-cli`.
- 0 warnings; `cargo test --workspace` green.
