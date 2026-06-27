# Glulx VM — Phase 2a: Foundation + Headless Runner — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**New crates:** `crates/gvm` (zero-dependency), `crates/gvm-cli`
**Roadmap:** Glulx support sub-project 2 (the VM), phase 2a of 2a→2b→2c. See
`2026-06-27-blorb-parser-design.md` (sub-project 1).

## Context

A from-scratch Glulx interpreter, structured like `zvm`: a `Memory` over the
loaded image, a `Machine` with `with_output(...)` + `step() -> StepResult`, and a
pluggable `Output` trait. Phase 2a is the load-bearing core: load the image, run
functions, decode/dispatch a starter opcode set, and produce text output — enough
to execute a non-interactive Glulx program end to end and validate against
hand-assembled programs (and later glulxercise). Interactive input and full Glk
are sub-project 3; 2a only needs stream output and a minimal `@glk` for
`put_char`/`put_buffer`.

This phase deliberately mirrors `zvm`'s shapes so the app can later host both
engines uniformly and so the patterns are familiar.

## Glulx image (header) — what we load

The image begins with a 36-byte header (all 32-bit big-endian):
- `0x00` magic `b"Glul"`; `0x04` version; `0x08` RAMSTART; `0x0C` EXTSTART;
  `0x10` ENDMEM; `0x14` stack size; `0x18` start function address;
  `0x1C` decoding-table address (string table; used in 2b); `0x20` checksum.
- Memory map: `[0, RAMSTART)` is ROM (read-only at runtime), `[RAMSTART, EXTSTART)`
  is initialized RAM from the image, `[EXTSTART, ENDMEM)` is zero-initialized RAM.
  `ENDMEM` can grow/shrink at runtime via `setmemsize` (down to the original
  ENDMEM minimum). Version supported: 3.x (the common Inform 7 output); reject
  unknown major versions cleanly.

## Design

### Crate `gvm` (`crates/gvm`), zero-dep

Modules mirror `zvm`: `memory.rs`, `io.rs`, `header.rs`, `exec.rs`, `lib.rs`.

#### `memory.rs` — `Memory`

- `Memory::new(image: Vec<u8>) -> Result<Memory, GError>`: validate magic/version,
  read the header, allocate the full `ENDMEM` span (image bytes for
  `[0,EXTSTART)`, zeros for `[EXTSTART,ENDMEM)`), store header fields.
- Big-endian accessors: `read32/read16/read8(addr)`, `write32/16/8(addr, v)`
  (writes below RAMSTART are a fault in real Glulx; for 2a treat ROM writes as a
  guarded error/no-op and record a diagnostic — games don't write ROM).
- `mem_size()` / `set_mem_size(new)` (ENDMEM growth, zero-filled, clamped to the
  original ENDMEM floor; 256-byte aligned per spec).
- Bounds-checked; out-of-range access returns/records a fault rather than panics.

#### `io.rs` — `Output`

Same shape as `zvm::io::Output`:
```rust
pub trait Output: std::any::Any {
    fn print(&mut self, s: &str);
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
pub struct BufferOutput { pub buf: String } // accumulating test sink
```

#### `exec.rs` — `Machine`, the stack, decode, dispatch

**Stack.** Glulx uses a single byte-addressed stack (size = header stack size).
Model it as `Vec<u8>` with `sp`, plus a frame pointer `fp`. Helpers:
`push32/pop32`, and frame-relative local access. A **call frame** layout (per
spec): frame length, locals-format list, then the locals, then the operand
stack region. Implement the documented frame format so `stkcount`/`stkpeek` and
locals addressing work.

**Calling convention.**
- A function starts with a type byte: `0xC0` (args passed on the stack) or
  `0xC1` (args stored into locals), followed by the **locals format** (pairs of
  `(LocalType, count)` terminated by `(0,0)`; types 1/2/4 byte).
- `call_function(addr, args)`: build a new frame (locals zeroed; for `0xC1`,
  copy args into locals; for `0xC0`, push args + count onto the new frame's
  stack), set `pc` to after the locals format, record the **call stub** (dest
  type/addr, pc, fp) for return.
- `return_value(v)`: pop the frame, write `v` per the saved call stub's
  destination (discard / memory / stack / local), restore pc/fp. Returning from
  the outermost (start) frame → `StepResult::Quit`.

**Decode.** `pc` walks instructions:
- Opcode number: 1 byte if `< 0x80`; 2 bytes if top bits `10`; 4 bytes if top
  bits `11` (per spec's variable-length opcode encoding).
- Operand addressing modes: one nibble per operand (two per byte, low nibble
  first), from a packed mode-byte run sized to the operand count. Modes: `0`
  constant 0; `1/2/3` constant 8/16/32-bit (sign-extended); `5/6/7` contents of
  8/16/32-bit address; `8` stack; `9/A/B` call-frame local at 8/16/32-bit offset;
  `D/E/F` contents of 8/16/32-bit RAM address (RAMSTART-relative). Load operands
  read a value; store operands hold a destination (mode `0` = discard, `8` =
  push, `5-7`/`D-F` = memory, `9-B` = local).
- A small `read_operands(load_count, store_count)` helper returns resolved load
  values and store destinations; `store(dest, value)` writes per mode.

**Dispatch (Phase 2a opcode subset).** Opcode numbers per the Glulx spec:
- Arithmetic/bit: `add 0x10`, `sub 0x11`, `mul 0x12`, `div 0x13`, `mod 0x14`,
  `neg 0x15`, `bitand 0x18`, `bitor 0x19`, `bitxor 0x1A`, `bitnot 0x1B`,
  `shiftl 0x1C`, `sshiftr 0x1D`, `ushiftr 0x1E` (32-bit two's-complement; div/mod
  truncate toward zero; div-by-zero → fault/diagnostic).
- Branch: `jump 0x20`, `jz 0x22`, `jnz 0x23`, `jeq 0x24`, `jne 0x25`, `jlt 0x26`,
  `jge 0x27`, `jgt 0x28`, `jle 0x29`, `jltu 0x2A`, `jgeu 0x2B`, `jgtu 0x2C`,
  `jleu 0x2D`. Branch target convention: operand `0` → return 0, `1` → return 1,
  else `pc = pc_after_operands + offset - 2`.
- Move/stack: `copy 0x40`, `copys 0x41`, `copyb 0x42` (truncated copies);
  `stkcount 0x70`, `stkpeek 0x71`, `stkswap 0x72`, `stkroll 0x73`, `stkcopy 0x74`,
  and push/pull via stack-mode operands.
- Functions: `call 0x30` (args from stack, count operand), `return 0x31`,
  `tailcall 0x34`, `callf 0x160`, `callfi 0x161`, `callfii 0x162`,
  `callfiii 0x163`.
- Memory size: `getmemsize 0x102`, `setmemsize 0x103`.
- Control: `nop 0x00`, `quit 0x120`, `glk 0x130` (minimal: see below),
  `getiosys 0x148`, `setiosys 0x149`.
- Output: `streamchar`, `streamnum`, `streamstr`, `streamunichar` — emit a
  character / signed-decimal number / string / Unicode char to the current iosys.
  Their exact opcode numbers come from the Glulx spec's table (see the note
  below); `streamstr` of compressed strings is deferred to 2b.

> Implementer note: opcode numbers above are a guide; the **authoritative source
> is the Glulx specification's opcode table** (the plan will cite exact values).
> Where this prose and the spec disagree, the spec wins — transcribe carefully.

**Output / I/O system.** `setiosys(mode, rock)` / `getiosys`. For 2a:
- iosys `0` (null) → discard.
- iosys `2` (glk): `streamchar`/`streamnum` and the `@glk` selectors
  `glk_put_char (0x0080)`, `glk_put_buffer (0x0084)`, `glk_put_char_uni`/
  `glk_put_buffer_uni` route their text to `self.out.print(...)`. A real Glk
  window/stream model is sub-project 3; here `@glk` implements just the
  put-char/put-buffer family (and returns 0 for the few setup selectors a
  minimal program calls, e.g. `glk_window_open`), enough to print.
- `streamnum` prints the signed-decimal of its operand.
- `streamstr` (compressed string decode) is **2b** — for 2a, a `streamstr` on a
  C-string (`0xE0`) type may print raw Latin-1 bytes until the terminator (simple
  case) and defer compressed (`0xE1`) strings to 2b.

**Run loop.** `step()` decodes+executes one instruction, returning
`StepResult::Continue` normally and `StepResult::Quit` on `quit`/outer return.
No input states in 2a (input is Glk → sub-project 3). `run()` convenience loops
until Quit. Faults (bad memory access, div-by-zero, bad opcode) record a
diagnostic (`machine.diagnostics: Vec<String>`) and Quit, rather than panicking.

```rust
pub enum StepResult { Continue, Quit }
pub struct Machine { /* mem, stack, pc, fp, iosys, out, diagnostics, … */ }
impl Machine {
    pub fn with_output(mem: Memory, out: Box<dyn Output>) -> Machine; // calls the start function
    pub fn step(&mut self) -> StepResult;
    pub fn run(&mut self);
}
```

### Crate `gvm-cli` (`crates/gvm-cli`)

A minimal headless runner mirroring zvm-cli's skeleton: read a file
(`.ulx` raw Glulx; `.gblorb` once the Blorb crate is merged — extract `GLUL`),
`Memory::new`, `Machine::with_output(StdoutOutput)`, `run()`. No screen model
yet. Prints stream output to stdout; prints diagnostics to stderr.

## Testing

`gvm` unit tests with **hand-assembled** Glulx images (a small `asm` test helper
that writes a header + a function + instructions as bytes):
- Header/load: valid image parses; bad magic/version → `GError`; RAM zero-fill;
  `set_mem_size` grows/shrinks and clamps to the floor.
- Arithmetic/bit: `add`/`sub`/`mul`/`div`/`mod`/`neg`, the shifts, bitops over
  known operands incl. negative/overflow/truncation; div-by-zero faults.
- Branch: each `jXX` taken/not-taken; the `0`/`1` return-convention; offset math.
- Operand modes: constants (0/8/16/32 sign-extended), memory contents (8/16/32),
  stack, locals, RAM-relative — for both load and store.
- Stack ops: `stkcount`/`stkpeek`/`stkswap`/`stkroll`/`stkcopy` on a known stack.
- Calls: `call`/`callf*`/`return`/`tailcall` with `0xC0` and `0xC1` functions,
  locals init, arg passing, and destination write-back; outer return → Quit.
- Output: a program that `setiosys(2,0)` then `streamnum`/`streamchar` (or
  `@glk glk_put_char`) produces the expected string in a `BufferOutput`.
- A small end-to-end program (arith + branch + call + output + quit) yields the
  right transcript via `gvm`'s `run()`.

(If a reference glulxercise/`.ulx` binary can be vendored as a test fixture,
add a smoke that runs it and checks for its known "pass" output — optional.)

## Out of scope (this phase)

- Compressed-string decoding + the string table + full `streamstr` → **2b**.
- `aload*`/`astore*`/bit array ops, `mzero`/`mcopy`, heap `malloc`/`mfree`,
  search opcodes → **2b**.
- `save`/`restore`/undo/`protect`/`verify`, `accelfunc`/`accelparam`,
  `random` → **2c**.
- Floating point → later.
- Full Glk (windows, input, styles) and the TUI mapping → **sub-project 3**.
- Automapping / app integration → **sub-project 4**.

## Global constraints

- `gvm` is zero-dependency (std only), mirroring `zvm`. `gvm-cli` depends only on
  `gvm` (and later `blorb`).
- Opcode numbers and encodings transcribed from the **Glulx specification's
  authoritative tables**, not from this prose — the plan cites exact values.
- No panics on malformed images or faults — record a diagnostic and Quit.
- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace`
  green per task.
- Commit-only on local `main` (or the phase's worktree branch); one commit per
  task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
