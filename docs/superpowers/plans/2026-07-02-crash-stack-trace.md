# Crash Stack-Trace Diagnostic Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a VM fault halts the machine (out-of-bounds memory access, stack under/overflow), emit a call-frame stack trace instead of panicking (zvm) or a bare one-line diagnostic (gvm), surfaced on zvm-cli stderr, gvm-cli stderr, and inline in the lanthorn TUI transcript.

**Architecture:** Each zero-dep VM crate gains a `StackTrace`/`TraceFrame` value type plus a `to_lines() -> Vec<String>` formatter (pure string building — zero-dep). zvm gains a graceful fault path (a fault latch on `Memory` + `State`, a new `StepResult::Fault`, and a trace built from `state.frames`); gvm captures a trace at its existing `Err(String)` → `Quit` fault point by walking the Glulx frame-pointer chain. Hosts consume `to_lines()`: the two CLIs print to stderr and exit non-zero; the app stores the pre-formatted lines on `TurnResult.fault` and renders them with a new themeable `transcript:crash` style selector.

**Tech Stack:** Rust workspace (`zvm`, `gvm`, `zvm-cli`, `gvm-cli`, `app`). No new dependencies.

## Global Constraints

- **VM crates (`zvm`, `gvm`) stay zero-dependency.** `StackTrace`/`TraceFrame` are plain structs; `to_lines()` builds `String`s. No formatting-to-terminal, no ratatui, no serde in the trace types.
- **Cross-platform** (Windows/Linux/macOS): plain text + existing render only.
- **No behavior change on correct execution.** The fault path is reachable only on a genuine hard error; normal `quit` / outer return is unaffected. A clean quit must yield **no** fault trace.
- **Styleable UI:** the TUI crash lines are themeable via a `style.toml` selector (`transcript:crash`), never hard-coded.
- **`func_addr` asymmetry (from the spec):** zvm frames carry a real routine entry address; gvm frames report `func_addr = 0` (Glulx does not store per-frame entry addresses; the return-PC chain localizes the call path). `TraceFrame.func_addr` documents `0 = unknown`.
- **Trace value widths:** zvm `width = 2` (u16 locals/operands), gvm `width = 4` (u32). `locals`/`operands` are widened to `i64` in the shared shape; `to_lines()` renders hex at `width` bytes (`2*width` hex digits).

---

## Task 1: zvm `StackTrace` type + `to_lines()` formatter

**Files:**
- Create: `crates/zvm/src/cpu/trace.rs`
- Modify: `crates/zvm/src/cpu/mod.rs` (add `pub mod trace;`)
- Test: in `crates/zvm/src/cpu/trace.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `zvm::cpu::trace::StackTrace { fault: String, fault_pc: u32, fault_op: String, width: u8, frames: Vec<TraceFrame> }` and `TraceFrame { func_addr: u32, return_pc: u32, locals: Vec<i64>, operands: Vec<i64> }`, both `#[derive(Debug, Clone, PartialEq)]`; and `impl StackTrace { pub fn to_lines(&self) -> Vec<String> }`.

- [ ] **Step 1: Write the failing test**

Add to a new `crates/zvm/src/cpu/trace.rs`:

```rust
//! Structured crash stack trace (zero-dep value type + text formatter).

/// A single call frame captured at a fault. Innermost (faulting) frame first
/// in `StackTrace::frames`.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceFrame {
    /// Routine entry address (0 = unknown; gvm always reports 0).
    pub func_addr: u32,
    /// PC to resume in the caller.
    pub return_pc: u32,
    /// Local variables, widened to i64.
    pub locals: Vec<i64>,
    /// This frame's working eval-stack values, widened to i64.
    pub operands: Vec<i64>,
}

/// A crash stack trace: the fault site plus the live call stack.
#[derive(Debug, Clone, PartialEq)]
pub struct StackTrace {
    /// Human-readable fault, e.g. "memory fault: read16 @0x004a1c".
    pub fault: String,
    /// Start-PC of the faulting instruction.
    pub fault_pc: u32,
    /// Decoded mnemonic of the faulting instruction, e.g. "loadw".
    pub fault_op: String,
    /// Hex render width in bytes: 2 = u16 (zvm), 4 = u32 (gvm).
    pub width: u8,
    /// Call frames, innermost (faulting) first.
    pub frames: Vec<TraceFrame>,
}

impl StackTrace {
    /// Canonical multi-line text form, shared by every host surface. One string
    /// per line, no trailing newlines.
    pub fn to_lines(&self) -> Vec<String> {
        let digits = (self.width as usize) * 2;
        let hexw = |v: i64| format!("0x{:0width$x}", v as u64 & mask(self.width), width = digits);
        let list = |xs: &[i64]| {
            xs.iter().map(|&v| hexw(v)).collect::<Vec<_>>().join(",")
        };
        let mut out = vec![
            "*** VM FAULT ***".to_string(),
            self.fault.clone(),
            format!("PC=0x{:06x}  op={}", self.fault_pc, self.fault_op),
        ];
        for (i, f) in self.frames.iter().enumerate() {
            let mut line = format!(
                "  #{i}  fn@0x{:06x}  ret=0x{:06x}  locals=[{}]",
                f.func_addr,
                f.return_pc,
                list(&f.locals),
            );
            if !f.operands.is_empty() {
                line.push_str(&format!("  stack=[{}]", list(&f.operands)));
            }
            out.push(line);
        }
        out
    }
}

fn mask(width: u8) -> u64 {
    match width {
        2 => 0xFFFF,
        4 => 0xFFFF_FFFF,
        _ => u64::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_lines_formats_header_pc_and_frames() {
        let t = StackTrace {
            fault: "memory fault: read16 @0x004a1c".to_string(),
            fault_pc: 0x004a1c,
            fault_op: "loadw".to_string(),
            width: 2,
            frames: vec![
                TraceFrame { func_addr: 0x4980, return_pc: 0x4a20, locals: vec![1, 0, 0xffff], operands: vec![0x2a] },
                TraceFrame { func_addr: 0x1200, return_pc: 0x32f0, locals: vec![], operands: vec![] },
            ],
        };
        let lines = t.to_lines();
        assert_eq!(lines[0], "*** VM FAULT ***");
        assert_eq!(lines[1], "memory fault: read16 @0x004a1c");
        assert_eq!(lines[2], "PC=0x004a1c  op=loadw");
        // width=2 → 4 hex digits per value; operands present → stack=[..]
        assert_eq!(lines[3], "  #0  fn@0x004980  ret=0x004a20  locals=[0x0001,0x0000,0xffff]  stack=[0x002a]");
        // empty locals + empty operands → no stack=[]
        assert_eq!(lines[4], "  #1  fn@0x001200  ret=0x0032f0  locals=[]");
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/zvm/src/cpu/mod.rs` add the line (keep alphabetical with the existing `decode`/`exec`/`state`):

```rust
pub mod trace;
```

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test -p zvm cpu::trace -- --nocapture`
Expected: PASS (`to_lines_formats_header_pc_and_frames`).

- [ ] **Step 4: Commit**

```bash
git add crates/zvm/src/cpu/trace.rs crates/zvm/src/cpu/mod.rs
git commit -m "feat(zvm): StackTrace value type + to_lines formatter"
```

---

## Task 2: zvm fault latches (stop panicking)

Make out-of-bounds memory access and stack underflow **latch a fault and return a benign default** instead of panicking. This task does not yet surface anything — it only removes the panics and records the fault, verified in isolation.

**Files:**
- Modify: `crates/zvm/src/memory.rs` (add a fault latch + bounds checks in `read_byte`/`read_word`/`write_byte`/`write_word`)
- Modify: `crates/zvm/src/cpu/state.rs` (add `State.fault`; convert the three `.expect` sites)
- Test: `crates/zvm/src/memory.rs` and `crates/zvm/src/cpu/state.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `Memory::take_mem_fault(&self) -> Option<(bool, u8, u32)>` (`(is_write, size_bytes, addr)`); `State.fault: Option<String>` (public field, default `None`).
- Consumes: nothing.

- [ ] **Step 1: Write the failing memory test**

Add to `crates/zvm/src/memory.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn oob_read_latches_fault_instead_of_panicking() {
    let m = Memory::new(sample_story(3)).unwrap();
    let end = m.len() as u32;
    // Previously panicked (unchecked slice index); must now return 0 + latch.
    let v = m.read_word(end + 100);
    assert_eq!(v, 0, "OOB read returns benign 0");
    assert_eq!(m.take_mem_fault(), Some((false, 2, end + 100)));
    // Latch is drained by take.
    assert_eq!(m.take_mem_fault(), None);
}

#[test]
fn oob_write_latches_fault() {
    let mut m = Memory::new(sample_story(3)).unwrap();
    let end = m.len() as u32;
    m.write_byte(end + 5, 0xAB);
    assert_eq!(m.take_mem_fault(), Some((true, 1, end + 5)));
}

#[test]
fn first_fault_wins() {
    let m = Memory::new(sample_story(3)).unwrap();
    let end = m.len() as u32;
    let _ = m.read_byte(end + 1);
    let _ = m.read_byte(end + 2);
    assert_eq!(m.take_mem_fault(), Some((false, 1, end + 1)), "keep first fault addr");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm memory::tests::oob_read_latches_fault -- --nocapture`
Expected: FAIL — `take_mem_fault` does not exist (compile error), and today the read panics.

- [ ] **Step 3: Implement the memory fault latch**

In `crates/zvm/src/memory.rs`, add a field to `struct Memory` (near `bytes`):

```rust
    /// Latched out-of-bounds access from the current instruction: (is_write,
    /// size_bytes, addr). Interior-mutable so read paths (&self) can record it.
    /// Drained by `take_mem_fault`; the CPU checks it after each instruction.
    mem_fault: std::cell::Cell<Option<(bool, u8, u32)>>,
```

Initialize it in every `Memory` constructor (search for `Memory {` struct-literal sites) with `mem_fault: std::cell::Cell::new(None),`.

Add a private helper + public drain:

```rust
    fn latch_fault(&self, is_write: bool, size: u8, addr: u32) {
        if self.mem_fault.get().is_none() {
            self.mem_fault.set(Some((is_write, size, addr)));
        }
    }

    /// Take and clear a latched OOB access recorded since the last drain.
    pub fn take_mem_fault(&self) -> Option<(bool, u8, u32)> {
        self.mem_fault.take()
    }
```

Rewrite the four accessors to bounds-check (replace the existing bodies):

```rust
    pub fn read_byte(&self, addr: u32) -> u8 {
        match self.bytes.get(addr as usize) {
            Some(&b) => b,
            None => { self.latch_fault(false, 1, addr); 0 }
        }
    }

    pub fn read_word(&self, addr: u32) -> u16 {
        let i = addr as usize;
        match (self.bytes.get(i), self.bytes.get(i + 1)) {
            (Some(&hi), Some(&lo)) => ((hi as u16) << 8) | lo as u16,
            _ => { self.latch_fault(false, 2, addr); 0 }
        }
    }

    pub fn write_byte(&mut self, addr: u32, v: u8) {
        debug_assert!(
            addr < self.header.static_mem_base as u32,
            "write to read-only memory at {:#06x} (static_mem_base = {:#06x})",
            addr, self.header.static_mem_base
        );
        match self.bytes.get_mut(addr as usize) {
            Some(slot) => *slot = v,
            None => self.latch_fault(true, 1, addr),
        }
    }

    pub fn write_word(&mut self, addr: u32, v: u16) {
        debug_assert!(
            addr < self.header.static_mem_base as u32,
            "write to read-only memory at {:#06x} (static_mem_base = {:#06x})",
            addr, self.header.static_mem_base
        );
        let i = addr as usize;
        if i + 1 < self.bytes.len() {
            self.bytes[i] = (v >> 8) as u8;
            self.bytes[i + 1] = (v & 0xFF) as u8;
        } else {
            self.latch_fault(true, 2, addr);
        }
    }
```

- [ ] **Step 4: Run the memory tests to verify they pass**

Run: `cargo test -p zvm memory::tests -- --nocapture`
Expected: PASS (the three new tests + all existing memory tests).

- [ ] **Step 5: Write the failing state test**

Add to `crates/zvm/src/cpu/state.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn return_with_no_frame_latches_underflow_not_panic() {
    let mut m = Memory::new(sample_story(3)).unwrap();
    let mut st = State::new(0x0400);
    // No frames pushed → previously `.expect("return with no active frame")`.
    return_value(&mut st, &mut m, 5);
    assert_eq!(st.fault.as_deref(), Some("stack underflow"));
}

#[test]
fn read_local_with_no_frame_latches_underflow() {
    let mut m = Memory::new(sample_story(3)).unwrap();
    let mut st = State::new(0x0400);
    let v = read_var(&mut st, &m, 0x01); // local 1, no frame
    assert_eq!(v, 0);
    assert_eq!(st.fault.as_deref(), Some("stack underflow"));
}
```

(Use whatever `Memory`/`sample_story` import the existing `state.rs` tests use; add `use super::*;` / `use crate::memory::Memory;` / `use crate::fixtures::sample_story;` as the sibling tests do.)

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test -p zvm cpu::state::tests::return_with_no_frame -- --nocapture`
Expected: FAIL — `st.fault` field does not exist; today the call panics.

- [ ] **Step 7: Implement the state fault field + convert the `.expect` sites**

In `crates/zvm/src/cpu/state.rs`, add to `struct State`:

```rust
    /// Latched stack-underflow fault from the current instruction. Drained by
    /// the CPU after each step. `None` in normal operation.
    pub fault: Option<String>,
```

Initialize in `State::new`: add `fault: None,`.

Convert the three panics:

`read_var` (locals branch, was `.expect("no current frame")`):
```rust
        0x01..=0x0F => {
            let idx = (var - 1) as usize;
            let Some(frame) = state.frames.last() else {
                state.fault = Some("stack underflow".to_string());
                return 0;
            };
            frame.locals.get(idx).copied().unwrap_or(0)
        }
```

`write_var` (locals branch, was `.expect("no current frame")`):
```rust
        0x01..=0x0F => {
            let idx = (var - 1) as usize;
            let Some(frame) = state.frames.last_mut() else {
                state.fault = Some("stack underflow".to_string());
                return;
            };
            // ...existing body that writes frame.locals[idx]...
```

`return_value` (was `.expect("return with no active frame")`):
```rust
    let Some(frame) = state.frames.pop() else {
        state.fault = Some("stack underflow".to_string());
        return;
    };
```

- [ ] **Step 8: Run the state tests to verify they pass**

Run: `cargo test -p zvm cpu::state::tests -- --nocapture`
Expected: PASS (new + existing).

- [ ] **Step 9: Full crate build/test (no regressions, no panics)**

Run: `cargo test -p zvm`
Expected: PASS (all).

- [ ] **Step 10: Commit**

```bash
git add crates/zvm/src/memory.rs crates/zvm/src/cpu/state.rs
git commit -m "feat(zvm): latch OOB memory + stack-underflow faults instead of panicking"
```

---

## Task 3: zvm fault wiring — `StepResult::Fault` + trace builder

Turn the latched faults (Task 2) into a `StepResult::Fault` carrying a `StackTrace` (Task 1), captured with the correct instruction-start PC and opcode name.

**Files:**
- Modify: `crates/zvm/src/cpu/state.rs` (add `func_addr` to `Frame`; set it in `call_routine`)
- Modify: `crates/zvm/src/cpu/exec.rs` (`StepResult::Fault`, `Machine.fault_trace`, `instr_start_pc` capture in `step()`, `Machine::build_trace`/`fault`, drain latches, `opcode_name`)
- Test: `crates/zvm/src/cpu/exec.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `StackTrace`/`TraceFrame` (Task 1); `Memory::take_mem_fault`, `State.fault` (Task 2).
- Produces: `StepResult::Fault` variant; `Machine.fault_trace: Option<crate::cpu::trace::StackTrace>` (public); `Machine::take_fault_trace(&mut self) -> Option<StackTrace>`.

- [ ] **Step 1: Write the failing test**

Add to `crates/zvm/src/cpu/exec.rs` `#[cfg(test)] mod tests`. Mirror the existing exec tests' machine-construction helper (they build a `Machine` from a `sample_story`/hand-built body — reuse that helper; below assumes a `machine_for(body)` style helper exists, else construct as the sibling tests do).

```rust
#[test]
fn loadw_out_of_bounds_faults_with_trace() {
    // loadw (2OP:0x0F) array=0xFFFE index=0x7FFF → address far past memory end.
    // Build: put a large base into a local via the routine, then loadw.
    // Simplest: call a routine that executes `loadw 0xFFFF 0xFFFF -> sp`.
    let mut m = build_machine_running_loadw_oob(); // see helper note below
    let start_pc = m.state.pc;
    let r = m.step();
    assert_eq!(r, StepResult::Fault);
    let t = m.take_fault_trace().expect("fault trace present");
    assert!(t.fault.starts_with("memory fault: read16 @"), "fault: {}", t.fault);
    assert_eq!(t.fault_op, "loadw");
    assert_eq!(t.fault_pc, start_pc, "fault_pc is the instruction start, not next_pc");
    assert_eq!(t.width, 2);
    assert!(!t.frames.is_empty());
}

#[test]
fn clean_quit_produces_no_fault_trace() {
    let mut m = build_machine_running_quit(); // a routine whose first op is `quit`
    let r = m.step();
    assert_eq!(r, StepResult::Quit);
    assert!(m.take_fault_trace().is_none());
}
```

**Helper note (do this in the test module):** the two `build_machine_*` helpers assemble a minimal story whose initial PC points at the target opcode. Follow the pattern the existing exec tests use to hand-assemble instruction bytes (search `mod tests` in exec.rs for how they encode opcodes and set `state.pc`). For `loadw` OOB, encode a 2OP loadw with two Large operands `0xFFFF, 0xFFFF` and a store-to-stack; place the machine's `state.pc` at that opcode with at least one frame pushed (call a wrapper routine, or push a synthetic frame as the state.rs tests do so `frames` is non-empty). For `quit`, encode 0OP:0x0A.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm cpu::exec::tests::loadw_out_of_bounds_faults -- --nocapture`
Expected: FAIL — `StepResult::Fault` and `take_fault_trace` do not exist.

- [ ] **Step 3: Add `func_addr` to `Frame` and set it**

In `crates/zvm/src/cpu/state.rs`, add to `struct Frame`:

```rust
    /// Routine entry address of this frame (0 for base/interrupt pseudo-frames).
    pub func_addr: u32,
```

In `call_routine`, set it in the `Frame { .. }` literal (the `routine_addr` is already in scope):

```rust
    state.frames.push(Frame {
        return_pc,
        locals,
        eval_base,
        store_var,
        arg_count: args.len().min(255) as u8,
        func_addr: routine_addr,
    });
```

Fix any other `Frame { .. }` literals the compiler flags (e.g. synthetic frames in `state.rs` tests and `quetzal.rs` restore) by adding `func_addr: 0,` (or `func_addr: routine_addr` where an entry address is known). Search: `Frame {`.

- [ ] **Step 4: Add the `Fault` variant + `fault_trace` field**

In `crates/zvm/src/cpu/exec.rs`, add to `enum StepResult`:

```rust
    /// A runtime fault halted the machine. The host reads `take_fault_trace()`.
    Fault,
```

Add to `struct Machine` (near `diagnostics`):

```rust
    /// Set when `step()` returns `Fault`; the host drains it for display.
    pub fault_trace: Option<crate::cpu::trace::StackTrace>,
```

Initialize `fault_trace: None,` in the `Machine` constructor(s).

Add the drain accessor + trace builder as `Machine` methods:

```rust
    /// Take and clear the stack trace captured at the last fault.
    pub fn take_fault_trace(&mut self) -> Option<crate::cpu::trace::StackTrace> {
        self.fault_trace.take()
    }

    fn build_trace(&self, fault: String, fault_pc: u32, fault_op: String)
        -> crate::cpu::trace::StackTrace
    {
        use crate::cpu::trace::{StackTrace, TraceFrame};
        let st = &self.state;
        let n = st.frames.len();
        let mut frames = Vec::with_capacity(n);
        // Innermost (last) frame first.
        for i in (0..n).rev() {
            let f = &st.frames[i];
            let upper = st.frames.get(i + 1).map(|nf| nf.eval_base).unwrap_or(st.eval_stack.len());
            let operands = st.eval_stack[f.eval_base..upper]
                .iter().map(|&w| w as i64).collect();
            frames.push(TraceFrame {
                func_addr: f.func_addr,
                return_pc: f.return_pc,
                locals: f.locals.iter().map(|&w| w as i64).collect(),
                operands,
            });
        }
        StackTrace { fault, fault_pc, fault_op, width: 2, frames }
    }
```

- [ ] **Step 5: Add `opcode_name` and capture `instr_start_pc` in `step()`; drain latches**

Add a free function in `exec.rs` (module scope):

```rust
use crate::cpu::decode::OperandCount;

/// Best-effort mnemonic for a decoded instruction; hex fallback when unknown.
/// Covers the memory/stack opcodes most likely to fault, plus common ones.
fn opcode_name(count: OperandCount, opcode: u8) -> String {
    let name = match (count, opcode) {
        (OperandCount::Two, 0x0F) => "loadw",
        (OperandCount::Two, 0x10) => "loadb",
        (OperandCount::Two, 0x01) => "je",
        (OperandCount::One, 0x0F) => "call_1n",
        (OperandCount::Var, 0x01) => "storew",
        (OperandCount::Var, 0x02) => "storeb",
        (OperandCount::Var, 0x00) => "call",
        (OperandCount::Var, 0x06) => "print_num",
        _ => return format!("op:{:?}/0x{:02x}", count, opcode),
    };
    name.to_string()
}
```

(Add `#[derive(Debug)]` to `OperandCount` in `decode.rs` if not already derived, so the `{:?}` fallback compiles.)

Rewrite `step()` to capture the start PC and drain the latches after `execute`:

```rust
    pub fn step(&mut self) -> StepResult {
        let version = self.mem.version();
        let instr_start_pc = self.state.pc;
        let instr = decode(&self.mem, self.state.pc, version);
        let op_name = opcode_name(instr.operand_count, instr.opcode);

        // CRITICAL: advance PC before executing so call/branch targets are correct.
        self.state.pc = instr.next_pc;

        let result = self.execute(instr);

        // A latched OOB access or stack underflow overrides the normal result.
        if let Some((is_write, size, addr)) = self.mem.take_mem_fault() {
            let kind = if is_write { "write".to_string() } else { format!("read{}", size as u32 * 8) };
            let msg = format!("memory fault: {kind} @{addr:#010x}");
            self.fault_trace = Some(self.build_trace(msg, instr_start_pc, op_name));
            return StepResult::Fault;
        }
        if let Some(msg) = self.state.fault.take() {
            self.fault_trace = Some(self.build_trace(msg, instr_start_pc, op_name));
            return StepResult::Fault;
        }
        result
    }
```

Note: `execute` takes `instr` by value; compute `op_name` (and `instr_start_pc`) before the `execute(instr)` move, as shown.

- [ ] **Step 6: Run the exec tests to verify they pass**

Run: `cargo test -p zvm cpu::exec::tests::loadw_out_of_bounds_faults cpu::exec::tests::clean_quit_produces_no_fault_trace -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Full crate test**

Run: `cargo test -p zvm`
Expected: PASS (all). If a `match step` elsewhere in `zvm` is now non-exhaustive (unlikely — zvm has no host loop), add a `StepResult::Fault => {}` arm.

- [ ] **Step 8: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs crates/zvm/src/cpu/state.rs crates/zvm/src/cpu/decode.rs
git commit -m "feat(zvm): StepResult::Fault with stack trace built from live frames"
```

---

## Task 4: gvm `StackTrace` type + fault-trace capture

gvm mirrors Task 1's type and captures a trace at its existing `Err(String)` → `Quit` fault point by walking the Glulx frame-pointer chain (read-only). `func_addr` is always 0.

**Files:**
- Create: `crates/gvm/src/trace.rs`
- Modify: `crates/gvm/src/lib.rs` (add `pub mod trace;` + re-export)
- Modify: `crates/gvm/src/exec.rs` (`Machine.instr_start_pc`, `Machine.fault_trace`, capture at the `Err` handler, frame-chain walk, `opcode_name`)
- Test: `crates/gvm/src/trace.rs` and `crates/gvm/src/exec.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `gvm::trace::StackTrace`/`TraceFrame` (identical shape + `to_lines()` to zvm's, `width` defaults to 4 at construction); `Machine.fault_trace: Option<StackTrace>` (public) + `Machine::take_fault_trace`.

- [ ] **Step 1: Create the gvm trace type (copy of Task 1, width-agnostic)**

Create `crates/gvm/src/trace.rs` with the **exact same** `TraceFrame`, `StackTrace`, `to_lines()`, and `mask()` code as Task 1 Step 1 (minus the zvm test), plus this gvm-focused test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_lines_renders_u32_width() {
        let t = StackTrace {
            fault: "memory fault: read32 @0x00040000".to_string(),
            fault_pc: 0x1abc,
            fault_op: "aload".to_string(),
            width: 4,
            frames: vec![TraceFrame { func_addr: 0, return_pc: 0x00001234, locals: vec![0xdead_beef], operands: vec![] }],
        };
        let lines = t.to_lines();
        assert_eq!(lines[0], "*** VM FAULT ***");
        assert_eq!(lines[2], "PC=0x001abc  op=aload");
        // width=4 → 8 hex digits; func_addr 0 renders as fn@0x000000
        assert_eq!(lines[3], "  #0  fn@0x000000  ret=0x001234  locals=[0xdeadbeef]");
    }
}
```

In `crates/gvm/src/lib.rs` add (after the other `pub mod` lines):

```rust
pub mod trace;
```

and to the re-export block:

```rust
pub use trace::{StackTrace, TraceFrame};
```

- [ ] **Step 2: Run the trace test**

Run: `cargo test -p gvm trace -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Write the failing exec fault-capture test**

Add to `crates/gvm/src/exec.rs` `#[cfg(test)] mod tests` (reuse the existing `machine_with_body`/function-assembly helpers — search the test module):

```rust
#[test]
fn oob_load_captures_fault_trace() {
    // A function whose body does `aload` (or copy) from a wildly OOB address.
    // Reuse the existing asm helpers to build a start function that faults.
    let mut m = build_machine_faulting_load(); // helper: start fn reads OOB memory
    // Drive until it halts.
    let mut steps = 0;
    loop {
        match m.step() {
            StepResult::Continue => { steps += 1; assert!(steps < 1000, "runaway"); }
            StepResult::Quit => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    let t = m.take_fault_trace().expect("fault trace present");
    assert!(t.fault.starts_with("memory fault: "), "fault: {}", t.fault);
    assert_eq!(t.width, 4);
    assert!(!t.frames.is_empty());
    assert_eq!(t.frames[0].func_addr, 0, "gvm func_addr is always 0 (unknown)");
}

#[test]
fn clean_quit_has_no_fault_trace() {
    let mut m = build_machine_immediate_quit(); // start fn: `quit`
    loop { match m.step() { StepResult::Quit => break, StepResult::Continue => {}, o => panic!("{o:?}") } }
    assert!(m.take_fault_trace().is_none());
}
```

**Helper note:** build the faulting function with the existing `asm::func(...)` + opcode-encoding helpers used elsewhere in the gvm test module. The faulting op should read main memory out of range so `m8/m16/m32` returns `Err` (e.g. a `copy`/`aload` with a constant address `0x7FFF_FFFF`). Keep at least the start frame on the stack (it always exists).

- [ ] **Step 4: Run to verify it fails**

Run: `cargo test -p gvm exec::tests::oob_load_captures_fault_trace -- --nocapture`
Expected: FAIL — `take_fault_trace` does not exist.

- [ ] **Step 5: Add fields + capture at the fault point**

In `crates/gvm/src/exec.rs`, add to `struct Machine`:

```rust
    /// PC at the start of the instruction currently executing (captured before
    /// operand reads); used as the fault site if this instruction faults.
    instr_start_pc: u32,
    /// Stack trace captured when a fault converted to Quit. Host drains it.
    pub fault_trace: Option<crate::trace::StackTrace>,
```

Initialize both in the `Machine` constructor: `instr_start_pc: 0,` and `fault_trace: None,`.

Capture the start PC at the top of `step_once` (line ~415):

```rust
    pub(crate) fn step_once(&mut self) -> R<()> {
        self.insn_count += 1;
        self.instr_start_pc = self.pc;
        let opcode = self.decode_opcode()?;
        self.execute(opcode)
    }
```

At the `Err(msg)` handler inside `step()` (~exec.rs:2916), build the trace before halting:

```rust
                Err(msg) => {
                    self.fault_trace = Some(self.build_trace(msg.clone()));
                    self.diagnostics.push(msg);
                    self.halted = true;
                    StepResult::Quit
                }
```

Add the drain accessor:

```rust
    pub fn take_fault_trace(&mut self) -> Option<crate::trace::StackTrace> {
        self.fault_trace.take()
    }
```

- [ ] **Step 6: Implement `build_trace` (frame-chain walk) + `opcode_name`**

Add `Machine` methods in `exec.rs`. The walk uses `st_r32`, `fp`, `sp`, and the frame header layout (FrameLen@f, LocalsPos@f+4, format pairs@f+8; stub below a frame at `[f-16, f)` with `caller_fp = st_r32(f-4)`, `ret_pc = st_r32(f-8)`; value/operand region of a frame runs `[f + frame_len, child_f - 16)` or `[f + frame_len, sp)` for the innermost):

```rust
    fn build_trace(&self, fault: String) -> crate::trace::StackTrace {
        use crate::trace::{StackTrace, TraceFrame};
        let fault_op = self.opcode_name_at(self.instr_start_pc);
        let mut frames = Vec::new();
        let mut f = self.fp;                 // innermost frame offset
        let mut inner_bottom = self.sp;      // top of innermost value region
        loop {
            let frame_len = self.st_r32(f as u32) as usize;
            let localspos = self.st_r32(f as u32 + 4) as usize;
            // Walk the locals-format list at f+8 to read each local value.
            let locals = self.read_frame_locals(f, localspos);
            // Value/operand region: above this frame's frame_len, up to inner_bottom.
            let val_lo = f + frame_len;
            let operands = self.read_stack_words(val_lo, inner_bottom);
            let (caller_fp, _ret_pc, this_ret_pc) = if f == 0 {
                (0usize, 0u32, 0u32) // start frame: no stub beneath it
            } else {
                let caller_fp = self.st_r32(f as u32 - 4) as usize;
                let ret_pc = self.st_r32(f as u32 - 8);
                (caller_fp, ret_pc, ret_pc)
            };
            frames.push(TraceFrame {
                func_addr: 0, // Glulx does not store per-frame entry addresses
                return_pc: this_ret_pc,
                locals,
                operands,
            });
            if f == 0 { break; }
            inner_bottom = f.saturating_sub(16); // stub sits at [f-16, f)
            f = caller_fp;
            if frames.len() > 256 { break; } // guard against a corrupt chain
        }
        StackTrace { fault, fault_pc: self.instr_start_pc, fault_op, width: 4, frames }
    }

    /// Read a frame's local values by walking its (type,count) format list.
    fn read_frame_locals(&self, f: usize, localspos: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let mut fmt = f + 8;
        let mut off = f + localspos;
        loop {
            let ty = self.stack_byte(fmt);
            let count = self.stack_byte(fmt + 1);
            if ty == 0 && count == 0 { break; }
            let size = ty as usize; // Glulx local sizes: 1, 2, or 4 bytes
            for _ in 0..count {
                off = align_up(off, size);
                let v = match size {
                    1 => self.stack_byte(off) as i64,
                    2 => (((self.stack_byte(off) as u32) << 8) | self.stack_byte(off + 1) as u32) as i64,
                    _ => self.st_r32(off as u32) as i64,
                };
                out.push(v);
                off += size.max(1);
            }
            fmt += 2;
            if out.len() > 256 { break; }
        }
        out
    }

    fn read_stack_words(&self, lo: usize, hi: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let mut a = lo;
        while a + 4 <= hi {
            out.push(self.st_r32(a as u32) as i64);
            a += 4;
        }
        out
    }

    fn stack_byte(&self, off: usize) -> u8 {
        self.stack.get(off).copied().unwrap_or(0)
    }

    fn opcode_name_at(&self, pc: u32) -> String {
        // Best-effort: decode just the opcode number at pc for a name.
        match self.mem.read8(pc) {
            Some(b) => opcode_name(b as u32),
            None => "<unknown>".to_string(),
        }
    }
```

Add module-scope helpers (reuse gvm's existing `align_up` if one exists — search; the frame builder at exec.rs:846 already uses `align_up`, so it exists — do **not** redefine it):

```rust
/// Best-effort Glulx opcode mnemonic; hex fallback when unknown.
fn opcode_name(opcode: u32) -> String {
    let name = match opcode {
        0x30 => "call",
        0x31 => "return",
        0x40 => "copy",
        0x48 => "aload",
        0x4C => "astore",
        0x70 => "streamchar",
        0x160..=0x163 => "callf",
        _ => return format!("0x{opcode:x}"),
    };
    name.to_string()
}
```

Note: the single-byte `opcode_name_at` read is a simplification (Glulx opcodes are variable-length up to 4 bytes). It is acceptable for a best-effort `fault_op`; the fault message + `fault_pc` carry the authoritative location. Do not expand it.

- [ ] **Step 7: Run the exec tests**

Run: `cargo test -p gvm exec::tests::oob_load_captures_fault_trace exec::tests::clean_quit_has_no_fault_trace -- --nocapture`
Expected: PASS.

- [ ] **Step 8: Full crate test**

Run: `cargo test -p gvm`
Expected: PASS (all).

- [ ] **Step 9: Commit**

```bash
git add crates/gvm/src/trace.rs crates/gvm/src/lib.rs crates/gvm/src/exec.rs
git commit -m "feat(gvm): StackTrace + fault-trace capture via frame-chain walk"
```

---

## Task 5: zvm-cli — print trace to stderr, exit non-zero

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (add a `StepResult::Fault` arm to the run loop)

**Interfaces:**
- Consumes: `StepResult::Fault`, `machine.take_fault_trace()` (Task 3).

- [ ] **Step 1: Add the Fault arm**

In `crates/zvm-cli/src/main.rs`, in the `match step` (~846–970), add an arm alongside `Quit`:

```rust
            StepResult::Fault => {
                print!("{}", view.leave());
                let _ = io::stdout().flush();
                let _ = terminal::disable_raw_mode();
                if let Some(trace) = machine.take_fault_trace() {
                    for line in trace.to_lines() {
                        eprintln!("{line}");
                    }
                }
                std::process::exit(70); // EX_SOFTWARE: internal software error
            }
```

(70 = `sysexits.h` `EX_SOFTWARE`; any non-zero is acceptable — keep it distinct from the setup `exit(1)` paths so a crash is greppable.)

- [ ] **Step 2: Build to verify it compiles + the match is exhaustive**

Run: `cargo build -p zvm-cli`
Expected: PASS (no non-exhaustive-match error).

- [ ] **Step 3: Manual smoke (optional, no automated CLI harness exists)**

There is no story file that reliably faults in-repo; verification is covered by the zvm unit tests (Task 3). Confirm the binary still runs a normal game:

Run: `cargo run -p zvm-cli -- <any local .z5>` (Ctrl-C to exit) — sanity only.

- [ ] **Step 4: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit -m "feat(zvm-cli): print crash stack trace to stderr and exit non-zero"
```

---

## Task 6: gvm-cli — print trace to stderr, exit non-zero

gvm faults surface as `Quit` with `machine.fault_trace` set (Task 4). Check after the drive loop.

**Files:**
- Modify: `crates/gvm-cli/src/main.rs`

**Interfaces:**
- Consumes: `machine.take_fault_trace()` (Task 4).

- [ ] **Step 1: Emit the trace after the loop and exit non-zero**

In `crates/gvm-cli/src/main.rs`, after `drive(...)` returns and after `machine.flush()` + `disable_raw_mode()` (~228–234), before/around the diagnostics print, add:

```rust
    if let Some(trace) = machine.take_fault_trace() {
        for line in trace.to_lines() {
            eprintln!("{line}");
        }
        // Still surface any other diagnostics, then exit non-zero.
        for d in &machine.diagnostics {
            eprintln!("gvm: {d}");
        }
        std::process::exit(70);
    }
```

Leave the existing `for d in &machine.diagnostics { eprintln!("gvm: {d}"); }` for the non-fault path (it runs when `fault_trace` is `None`).

- [ ] **Step 2: Build**

Run: `cargo build -p gvm-cli`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/gvm-cli/src/main.rs
git commit -m "feat(gvm-cli): print crash stack trace to stderr and exit non-zero"
```

---

## Task 7: app — `transcript:crash` style selector

Add a themeable style for crash lines (applied by Task 8). No new `TranscriptKind` variant — crash lines reuse the `Warning` kind with an explicit per-line style override.

**Files:**
- Modify: `crates/app/src/colors.rs` (`ColorScheme.transcript_crash` + defaults in both ctors)
- Modify: `crates/app/src/style.rs` (`SELECTOR_FIELDS`, `SELECTOR_GROUPS`, `style_for_selector`, `apply_color_decls`)
- Test: `crates/app/src/style.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `ColorScheme.transcript_crash: ratatui::style::Style`; selector string `"transcript:crash"`.

- [ ] **Step 1: Write the failing selector test**

Add to `crates/app/src/style.rs` `#[cfg(test)] mod tests`:

```rust
#[test]
fn crash_selector_maps_to_transcript_crash_field() {
    let mut cs = ColorScheme::default();
    // style_for_selector must resolve the new selector.
    let _ = style_for_selector(&cs, "transcript:crash");
    // apply_color_decls must patch the field.
    apply_color_decls(&mut cs, "[transcript:crash]\nfg = \"red\"\n").unwrap();
    assert_eq!(style_for_selector(&cs, "transcript:crash").fg, Some(ratatui::style::Color::Red));
}
```

(Match the exact call signatures of `style_for_selector` / `apply_color_decls` used by the sibling tests — adapt the decl-string form to however `apply_color_decls_patches_correct_fields` feeds decls.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app style::tests::crash_selector_maps -- --nocapture`
Expected: FAIL — selector unknown / field missing.

- [ ] **Step 3: Add the `ColorScheme` field + defaults**

In `crates/app/src/colors.rs`, add near `transcript_warning` (line ~325):

```rust
    /// VM crash / fault trace lines in the transcript.
    pub transcript_crash: Style,
```

In the constants ctor (near line ~420, beside `transcript_warning: Style::new().fg(Color::Yellow)`):

```rust
        transcript_crash: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
```

In the theme-derived ctor (near line ~598, beside the `transcript_warning: ... scheme.palette[3]` default):

```rust
        transcript_crash: Style::new().fg(scheme.palette[1]).add_modifier(Modifier::BOLD),
```

(Use whatever palette index the existing "error/red" role uses; `[1]` is illustrative — match the neighbouring alert/error default. Ensure `Modifier` is already imported in this file; it is used by other fields.)

- [ ] **Step 4: Register the selector in `style.rs`**

1. In `SELECTOR_FIELDS` (~163–208), add after `"transcript:warning"`:
   ```rust
       "transcript:crash",
   ```
2. In `SELECTOR_GROUPS` (~218), add `"transcript:crash"` to the Transcript group's slice (the one containing `"transcript:warning"`).
3. In `style_for_selector` (~254–304), add an arm:
   ```rust
       "transcript:crash" => cs.transcript_crash,
   ```
4. In `apply_color_decls` (~390), add an arm:
   ```rust
       "transcript:crash" => cs.transcript_crash = cs.transcript_crash.patch(style),
   ```

- [ ] **Step 5: Run the selector tests (incl. completeness)**

Run: `cargo test -p app style::`
Expected: PASS — the new test plus `selector_groups_cover_all_selector_fields` and `style_for_selector_reads_the_right_field` (which will now include `transcript:crash`).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/style.rs
git commit -m "feat(app): add themeable transcript:crash style selector"
```

---

## Task 8: app — surface the fault trace in the transcript

Thread the pre-formatted trace lines from both sessions onto `TurnResult.fault`, and render them (crash-styled) plus a `(game halted)` line.

**Files:**
- Modify: `crates/app/src/session.rs` (`TurnResult.fault` field; handle `StepResult::Fault` in `run_until_input`; populate in `drain_turn`)
- Modify: `crates/app/src/glulx_session.rs` (populate `TurnResult.fault` from `machine.take_fault_trace()` in `finish_turn`)
- Modify: `crates/app/src/main.rs` (`apply_turn_events` renders the fault lines)
- Test: `crates/app/src/render/transcript.rs` (render test) + a session-level test in `session.rs`

**Interfaces:**
- Consumes: zvm `StepResult::Fault` + `take_fault_trace()` (Task 3); gvm `take_fault_trace()` (Task 4); `to_lines()` (Tasks 1/4); `ColorScheme.transcript_crash` (Task 7).
- Produces: `TurnResult.fault: Option<Vec<String>>` (the same field name in both sessions' `TurnResult`, or the shared `TurnResult` if they share one — verify: both are `app::session::TurnResult`).

- [ ] **Step 1: Add the `fault` field to `TurnResult`**

In `crates/app/src/session.rs`, add to `struct TurnResult` (near `diagnostics`, ~135):

```rust
    /// Pre-formatted crash stack-trace lines when the VM faulted this turn.
    pub fault: Option<Vec<String>>,
```

Set `fault: None` in every `TurnResult { .. }` construction the compiler flags (search `TurnResult {` across `session.rs` and `glulx_session.rs`).

- [ ] **Step 2: Handle `StepResult::Fault` in the zvm run loop + populate**

In `crates/app/src/session.rs` `run_until_input` (~419–449), add an arm mirroring `Quit`:

```rust
            StepResult::Fault => return (RunStop::Quit, /* same trailing fields as the Quit arm */),
```

(Copy the exact tuple the `Quit` arm returns.) Then in `drain_turn` (~336, where `diagnostics` is taken) add:

```rust
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
```

and set `fault` on the `TurnResult` it builds (~348).

- [ ] **Step 3: Populate `fault` in the Glulx session**

In `crates/app/src/glulx_session.rs` `finish_turn` (~155, where `diagnostics` is taken):

```rust
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
```

and set it on the `TurnResult` (~164).

- [ ] **Step 4: Write the failing render test**

Add to `crates/app/src/render/transcript.rs` `#[cfg(test)] mod tests`, mirroring `render_kinds_draw_their_own_styles_and_gutters`:

```rust
#[test]
fn crash_lines_render_with_crash_style() {
    let mut state = AppState::for_test(); // use the same constructor the sibling tests use
    let crash_style = state.colors.transcript_crash;
    state.push_transcript_styled("*** VM FAULT ***", TranscriptKind::Warning, crash_style);
    // render into a Buffer as the sibling test does, then assert a text cell
    // carries crash_style.fg.
    // (Reuse the exact Buffer/area/render call from render_kinds_draw_their_own_styles_and_gutters.)
    let buf = render_transcript_to_test_buffer(&state); // adapt to the real helper
    let fg = /* fg color of a cell on the crash row, per the sibling test's cell-probe */;
    assert_eq!(fg, crash_style.fg.unwrap());
}
```

Adapt the buffer-probe mechanics to match `render_kinds_draw_their_own_styles_and_gutters` exactly (same `Rect`, same render entry point, same cell-access pattern).

- [ ] **Step 5: Run to verify it fails**

Run: `cargo test -p app render::transcript::tests::crash_lines_render_with_crash_style -- --nocapture`
Expected: FAIL (assertion or, if `push_transcript_styled` already works, it may pass — in that case the test still guards the wiring; proceed).

- [ ] **Step 6: Render the fault lines in `apply_turn_events`**

In `crates/app/src/main.rs` `apply_turn_events` (~4758), after the diagnostics loop, add:

```rust
    if let Some(lines) = &result.fault {
        for line in lines {
            state.push_transcript_styled(line, app::state::TranscriptKind::Warning, state.colors.transcript_crash);
        }
        state.push_transcript_styled("(game halted)", app::state::TranscriptKind::Warning, state.colors.transcript_crash);
    }
```

- [ ] **Step 7: Write a session-level test (zvm fault → TurnResult.fault)**

Add to `crates/app/src/session.rs` `#[cfg(test)] mod tests`: build a `Session` over a story that faults on its first turn (reuse the zvm test's faulting-body approach, or a minimal hand-built story), drive one turn, and assert `turn_result.fault` is `Some(lines)` where `lines[0] == "*** VM FAULT ***"`. If constructing a faulting `Session` in-app is impractical, assert instead that a `TurnResult` carrying `fault: Some(vec![...])` flows unchanged through `drain_turn`'s assembly (a narrower unit check). Prefer the end-to-end version if the test harness supports loading a byte buffer.

- [ ] **Step 8: Run the app test suite**

Run: `cargo test -p app`
Expected: PASS (all, including the new render + session tests). Fix any non-exhaustive `match` on `StepResult` the compiler flags in `session.rs` by handling `Fault`.

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/session.rs crates/app/src/glulx_session.rs crates/app/src/main.rs crates/app/src/render/transcript.rs
git commit -m "feat(app): surface VM crash stack trace inline in the transcript"
```

---

## Task 9: workspace verification + docs

**Files:**
- Modify: `TODO.md` (close the crash-diagnostic item via `scripts/todo-done`)
- Modify: `README.md` (only if this counts as a major feature — see note)

- [ ] **Step 1: Full workspace build + test**

Run: `cargo test --workspace`
Expected: PASS (all crates).

- [ ] **Step 2: Clippy (match project norm)**

Run: `cargo clippy --workspace --all-targets`
Expected: no new warnings from the changed files.

- [ ] **Step 3: Close the TODO item**

Run: `scripts/todo-done "Crash stack-trace diagnostic"`
(Then commit the COMPLETED.md move as the git hooks require; the `prepare-commit-msg` hook adds the `Completes:` trailer.)

- [ ] **Step 4: README**

Per the README-major-features-only rule, a developer crash-diagnostic is a borderline major feature. Add a single bullet under the appropriate section **only if** the reviewer agrees it's major; otherwise skip. Do not add per-VM detail.

- [ ] **Step 5: Commit**

```bash
git add TODO.md COMPLETED.md README.md
git commit -m "docs: close crash stack-trace TODO"
```

---

## Self-Review notes (for the executor)

- **Spec coverage:** §2 trace shape → Tasks 1/4. §2.1 fault_pc capture → Tasks 3/4 (`instr_start_pc`). §3.1 gvm trigger → Task 4. §3.2/§3.3 zvm graceful fault + builder → Tasks 2/3. §4.1 CLIs → Tasks 5/6. §4.2 app inline + `crash` selector → Tasks 7/8. §6 tests → each task's tests. §7 non-goals (no symbolication, gvm func_addr=0) → honored.
- **Deviation from spec wording:** the spec §3.2 described "checked variants routed through a Machine helper"; this plan uses a **fault latch on `Memory`/`State`** instead — same behavior, far less call-site churn, still zero-dep. The spec §4 said "no formatting in the VM crates"; this plan puts the pure `to_lines()` string builder in the VM crates so all three host surfaces share one formatter (DRY) — the VMs still contain no terminal/styling logic. Both refinements were agreed during design; flag them to the reviewer.
- **Type consistency:** `StackTrace`/`TraceFrame` fields identical in zvm (`cpu::trace`) and gvm (`trace`); `to_lines()` identical; `width` 2 (zvm) / 4 (gvm); `TurnResult.fault: Option<Vec<String>>`; `take_fault_trace()` on both machines.
- **Reviewer watch-items:** (a) every `Frame {`/`TurnResult {`/`Memory {` literal gets the new field; (b) the zvm `step()` computes `op_name`/`instr_start_pc` before `execute(instr)` moves `instr`; (c) gvm's frame-walk value-region bounds (`[f+frame_len, child_f-16)`); (d) no new `TranscriptKind` variant (crash reuses `Warning` + explicit style).
