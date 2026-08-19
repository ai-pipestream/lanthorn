# Crash Stack-Trace Diagnostic — Design Spec

**Status:** Draft for review · 2026-07-02
**Goal:** When a VM fault halts the machine (out-of-bounds memory access, bad
opcode, stack under/overflow), emit a **call-frame stack trace** — each frame's
function address, return PC, locals, and working-stack operands, plus the
faulting PC and decoded opcode — instead of today's process-aborting panic (zvm)
or single-line diagnostic (gvm). Surfaces on `zvm-cli` stderr, `gvm-cli` stderr,
and inline in the lanthorn TUI transcript.

**Motivation:** The Brain Guzzlers heap fault during the Glulx survey took an
investigation to localize; a frame trace would have made it a glance. More
fundamentally, zvm currently **panics the host process** on bad game bytecode
(unchecked memory reads, `.expect` on an empty frame stack) — a latent bug: a
buggy or hostile story file should not nuke the TUI.

**Scope:** `zvm` + `gvm` (fault path + trace builder, both stay zero-dependency),
`zvm-cli` + `gvm-cli` (stderr formatting), `app` (inline transcript formatting +
style selector). No change to correct-execution behavior.

---

## 1. Global constraints

- **VM crates stay zero-dependency.** `zvm` and `gvm` build the trace as plain
  data (structs + `String`); no new dependency, no formatting logic in the VM.
- **Cross-platform** (Windows/Linux/macOS): plain text + existing TUI render; no
  platform-specific calls.
- **No behavior change on correct execution.** The fault path is only reachable
  on a genuine hard error; a normal `quit` / outer return is unaffected.
- **Styleable UI:** the TUI crash lines are themeable via a `style.toml`
  selector (per the styleable-UI-elements rule), not hard-coded.

---

## 2. The trace shape

Each VM crate defines this shape **independently** (no shared crate — that would
break zero-dep layering); the two definitions are structurally identical so the
host formatters can be written once per surface against each crate's type.

```rust
pub struct StackTrace {
    pub fault: String,           // "memory fault: read16 @0x004a1c"
    pub fault_pc: u32,           // start-PC of the faulting instruction
    pub fault_op: String,        // decoded mnemonic, e.g. "loadw" / "aload"
    pub width: u8,               // hex render width: 2 = u16 (zvm), 4 = u32 (gvm)
    pub frames: Vec<TraceFrame>, // innermost (faulting) frame first
}

pub struct TraceFrame {
    pub func_addr: u32,          // routine entry address (0 if unknown)
    pub return_pc: u32,          // PC to resume in the caller
    pub locals: Vec<i64>,        // widened: u16 (zvm) / u32 (gvm) → i64
    pub operands: Vec<i64>,      // this frame's working eval-stack values
}
```

`locals`/`operands` widen to `i64` so one host formatter handles both VMs; the
formatter renders them as hex at the VM's natural width (4 hex digits zvm,
8 hex digits gvm), tracked by a `width` byte on `StackTrace`
(`2` = u16, `4` = u32).

### 2.1 Faulting-PC capture

`fault_pc` must be the **start** of the faulting instruction, but by the time a
fault fires mid-instruction the VM's live PC has already advanced past the
opcode/operand bytes. Each VM therefore stashes the instruction-start PC into a
field (`instr_start_pc`) at the top of its decode/dispatch, before any operand
read. The fault builder reads that field, not the live PC.

`fault_op` is the decoded mnemonic of the instruction at `instr_start_pc`,
obtained from each VM's existing opcode-name path (zvm `decode`, gvm dispatch
name table). If the fault occurred before the opcode could be identified (e.g.
the PC itself is out of range), `fault_op` is `"<unknown>"`.

---

## 3. Fault trigger

### 3.1 gvm — extend the existing fault path

gvm already funnels recoverable faults through `fault(msg)` (exec.rs ~258:
push `msg` to `diagnostics`, set `halted = true`) and returns `StepResult::Quit`.
Its memory reads already bounds-check (`read8/16/32 -> Result`, faulting with
`memory fault: readN @addr`).

Changes:
- Add `fault_trace: Option<StackTrace>` to `Machine`.
- At the `fault()` call site, **before** setting `halted`, build the trace from
  the live call stack (still intact — nothing unwinds it) and store it in
  `fault_trace`. This distinguishes a **fatal fault** (has a `fault_trace`) from
  a benign deferred-feature diagnostic (pushes a `diagnostics` line but does not
  fault).
- The host reads `machine.fault_trace` after `step()` returns `Quit`; `Some`
  means crash, `None` means clean quit.

gvm's call stack is Glulx's unified in-memory stack. The trace builder walks the
frame-pointer chain from the current FP: for each frame read the frame length,
the locals-format list, and the locals values; the working operands are the
values above that frame's locals section, bounded by the start of the next
(inner) frame — or, for the innermost frame, by the current stack pointer. This
reuses the same frame layout gvm's `build_frame_and_enter` / return logic already
encodes.

### 3.2 zvm — introduce a graceful fault path (Option A)

zvm has **no** fault mechanism today: hard errors panic. This section adds one.

- Add `StepResult::Fault` (the host stops the run loop, like `Quit`, but knows a
  trace is available) and `fault_trace: Option<StackTrace>` on `Machine`.
- Add a private `fn fault(&mut self, msg: String) -> StepResult` that builds the
  trace from `state.frames` + `state.eval_stack` (see §3.3), stores it, and
  returns `StepResult::Fault`.
- **Bounds-check the memory read paths.** `memory.rs` `read_byte`/`read_word`
  currently index the backing slice directly (`self.bytes[addr]`), panicking on
  OOB. Add checked variants (`try_read_byte`/`try_read_word -> Option`) and route
  the CPU's reads through a helper on `Machine` that converts `None` into
  `self.fault(format!("memory fault: readN @{addr:#010x}"))`. The unchecked
  methods remain for callers that have already validated the address (header
  parse, dictionary), so hot-path reads are unaffected.
- **Convert the hot-path panics** that a story file can trigger:
  - stack underflow — `state.rs` `read_var`/`write_var`/return with no current
    frame (`.expect("no current frame")`, `.expect("return with no active frame")`)
    → fault `"stack underflow"`.
  - invalid routine header (`local_count > 15`) and OOB routine address in
    `call_routine`: these already coerce to "return 0" (a deliberate leniency for
    buggy callers); **leave that behavior** — it is not a fault. Only genuinely
    unrecoverable states fault.
  - unknown opcode: today decode falls through to a `diagnostics` warn-once and
    continues. **Leave that** — an unimplemented opcode is a soft diagnostic, not
    a crash. (A truly undecodable byte stream manifests as an OOB read, which
    faults.)

This keeps zvm's existing leniency where it is deliberate and only faults where
today it would panic.

### 3.3 zvm trace builder

`state.frames` is a `Vec<Frame>` where each `Frame` has `return_pc`,
`locals: Vec<u16>`, `eval_base`, `store_var`, `arg_count`. The builder walks
frames **from last to first** (innermost first):

- `func_addr`: the routine entry address. `Frame` does not currently store it;
  add a `func_addr: u32` field to `Frame`, set in `call_routine` (the value is
  already in scope there as `routine_addr`). The base/interrupt pseudo-frames
  (return_pc 0) set `func_addr = 0`.
- `return_pc`: `frame.return_pc`.
- `locals`: `frame.locals` widened to `i64`.
- `operands`: `eval_stack[frame.eval_base .. next_frame.eval_base]` (or
  `..eval_stack.len()` for the innermost), widened to `i64`.

---

## 4. Host surfaces

All three format the same `StackTrace`; **no formatting logic lives in the VM
crates.**

### 4.1 CLIs (`zvm-cli`, `gvm-cli`)

On a fatal fault (step returned `Fault` for zvm / `Quit` with `Some(fault_trace)`
for gvm), print to **stderr** and exit non-zero:

```
*** VM FAULT: memory fault: read16 @0x004a1c ***
  PC=0x004a1c  op=loadw
  #0  fn@0x004980  ret=0x004a20  locals=[0x01,0x00,0xffff]  stack=[0x2a]
  #1  fn@0x003b10  ret=0x004112  locals=[0x2a,0x0b]
  #2  fn@0x001200  ret=0x0032f0  locals=[]
```

- Header line: `fault`.
- `PC=` / `op=`: `fault_pc` (hex) and `fault_op`.
- One `#N` line per frame, innermost first: `fn@`=func_addr, `ret=`=return_pc,
  `locals=[…]`, and `stack=[…]` (omitted when empty).
- All numbers hex, zero-padded to the trace `width`.

### 4.2 App (TUI) — inline in transcript

The fault halts the session. The host formats the same trace as **styled lines
appended to the transcript** (the chosen presentation — no new modal), reusing
the existing transcript diagnostic-line path:

```
*** VM FAULT ***
memory fault: read16 @0x004a1c
PC=0x004a1c  op=loadw
  #0  fn@0x004980  ret=0x004a20  locals=[0x01,0x00,0xffff]  stack=[0x2a]
  #1  fn@0x003b10  ret=0x004112  locals=[0x2a,0x0b]
(game halted)
```

- Lines carry a new `crash` style selector (added to `ColorScheme` +
  `style.rs` selector list + applied at render), defaulting to an alert/error
  style. No new config toggle — a crash always shows.
- After the trace, `(game halted)`; the session enters its normal halted state
  (no further input accepted), exactly as a `quit` does today.
- Long traces flow into scrollback like any transcript text.

---

## 5. Data flow (one fault)

```
zvm: step() dispatch sets instr_start_pc = pc
  loadw reads memory @0x004a1c  → try_read_word returns None
  → self.fault("memory fault: read16 @0x004a1c")
      builds StackTrace from state.frames + eval_stack, fault_pc=instr_start_pc,
      fault_op="loadw"; stores in fault_trace; returns StepResult::Fault
host (app): step() == Fault → read machine.fault_trace
  → format lines into transcript with the `crash` style → halt session
host (cli): step() == Fault → format lines to stderr → exit(1)
```

gvm is identical except the trigger is the existing `fault()` and the halt
signal is `Quit` + `Some(fault_trace)`.

---

## 6. Testing

**VM unit tests (per crate, zero-dep, hand-built bytecode via the existing
`machine_with_body` / sample-story helpers):**

- **zvm memory fault:** a `loadw` from an address past memory end → `step()`
  returns `Fault`; `fault_trace` present; `fault_op == "loadw"`; `fault_pc`
  equals the instruction's start address; **and the read no longer panics**
  (the regression: same input used to abort).
- **zvm stack underflow:** force a return / var-read with an empty frame stack →
  faults with `"stack underflow"` instead of panicking.
- **gvm memory fault:** an `aload` past memory end → `Quit` with `fault_trace`
  present, correct `fault_op`/`fault_pc`.
- **Nested frames:** A calls B calls C, C faults → assert 3 frames, innermost
  first, each with the expected `func_addr`, `return_pc`, and `locals`; assert a
  non-empty `operands` on a frame with working values.
- **Clean quit is not a fault:** a normal `quit` → `fault_trace` is `None`
  (guards against mislabeling ordinary termination as a crash).

**Host formatting tests (no VM needed — feed a synthetic `StackTrace`):**

- CLI formatter → exact expected stderr block (both widths: a u16 trace and a
  u32 trace).
- App formatter → the expected transcript lines, and that they carry the `crash`
  style.

**No story files required** for the core; faults are synthesized from hand-built
functions, mirroring the existing VM test helpers.

---

## 7. Non-goals / deferred

- **Symbolication (function/local names).** IF story files are generally stripped
  of symbols; Inform debug files exist but parsing them is out of scope. Frames
  show raw addresses only.
- **Per-frame source locations.** Same reason — no line info without debug files.
- **Recovering execution after a fault.** A fault halts the machine; there is no
  resume. (Undo/restore remain available to the player as normal, from before the
  fault.)
- **Frames + full operand history / instruction backtrace.** Only the live call
  stack at the fault instant is captured, not an execution log.
- **A TUI crash modal / copy-to-clipboard.** The chosen presentation is inline
  transcript text; a dedicated overlay was considered and dropped (YAGNI — the
  scrollback already holds the text).
- **v6 / graphics faults** and other engine areas not yet implemented.

---

## 8. Success criteria

1. A story that reads out-of-bounds memory in zvm **halts with a trace instead
   of aborting the process**; the same class of fault in gvm shows a trace
   instead of a bare one-line diagnostic.
2. The trace names the faulting PC + opcode and lists every live call frame,
   innermost first, with per-frame locals and working operands.
3. All three surfaces (zvm-cli stderr, gvm-cli stderr, app transcript) render the
   trace; the app lines are themeable via the `crash` selector.
4. Correct execution and normal `quit` are unaffected (no false crash traces).
5. `zvm` and `gvm` remain zero-dependency.
