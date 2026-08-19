# Z-machine Debug Inspector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a read-only `/debug` inspector for Z-machine games — disassembly plus live VM state (call stack, locals, globals, object tree, dictionary, memory) in a full-screen TUI panel.

**Architecture:** Fill in the reserved `Debugger` capability seam on the `Engine` trait. `GameSession` (zvm) implements it, returning **pre-formatted `Vec<String>` per pane** (mirroring `Engine::window_dump`), so the app render code stays engine-neutral and Glulx can slot in later. The panel keeps a rendered snapshot, refreshed in the run loop where the engine is in scope; drawing just paints strings. The VM step loop is untouched.

**Tech Stack:** Rust, ratatui, crossterm. zvm (zero-dep Z-machine VM), app crate (TUI).

**Spec:** `docs/superpowers/specs/2026-07-18-story-debug-inspector-design.md`

## Global Constraints

Every task's requirements implicitly include these:

- **zvm stays zero-dependency.** The new disassembler uses only in-crate types and `std`; no new `[dependencies]`.
- **Engine-neutral seam.** All Z-machine specifics live behind the `Debugger` impl; the app render/panel code sees only `Vec<String>` / `u32`. No `zvm::` calls from the panel or render code.
- **Inspect-only, no VM-loop changes.** Do not touch `run_until_input` (`session.rs:576`) or `Machine::step`. No stepper, no breakpoints in v1.
- **Verify external constants.** The opcode→mnemonic table is an external constant: every entry must be cross-checked against the Z-Machine Standards Document §14 (authoritative) AND the crate's own `execute()` dispatch (`exec.rs`), and backed by a fixture-decode test. Do not trust the table in this plan blindly.
- **Styleable.** Every new UI element is themeable via `style.toml` selectors; no hard-coded colors in the render path — pull from `ColorScheme`.
- **Cross-platform.** App-side only; no platform-specific code.
- **Staging.** Stage commits explicitly by path (`git add <path> …`). Never `git add -A`/`-u` (untracked non-plan files exist in the tree).

---

### Task 1: zvm public disassembler (mnemonic table + formatter)

**Files:**
- Create: `crates/zvm/src/cpu/disasm.rs`
- Modify: `crates/zvm/src/cpu/mod.rs` (add `pub mod disasm;`)
- Test: inline `#[cfg(test)]` in `disasm.rs`

**Interfaces:**
- Consumes: `zvm::cpu::decode::{decode, Instr, Operand, OperandCount, Branch}` (`decode.rs:15-74,358`), `zvm::memory::Memory` (`memory.rs`).
- Produces:
  - `pub fn mnemonic(count: &OperandCount, opcode: u8, version: u8) -> &'static str`
  - `pub fn format_instr(instr: &Instr, version: u8) -> String`
  - `pub fn disassemble(mem: &Memory, start: u32, version: u8, lines: usize) -> Vec<String>`
  - `pub fn next_instr(mem: &Memory, addr: u32, version: u8) -> u32`

**Reference for formatting style** — match the hex idiom already used in `crates/zvm/src/cpu/trace.rs::to_lines` (values `0x{:0Nx}`, addresses `0x{:06x}`). Operands render as: `Large(n)`→`#{:04x}` (constant), `Small(n)`→`#{:02x}`, `Var(0)`→`sp`, `Var(1..=15)`→`local{n-1}` (or `L{n-1}`), `Var(16..=255)`→`g{n-16:02x}` (global). Store → ` -> <var>`. Branch → ` ?<label>` where offset 0=`rfalse`, 1=`rtrue`, else `0x{addr:06x}` computed as `next_pc + offset - 2`; prefix `~` when `on_true == false`.

- [ ] **Step 1: Write the failing test for `mnemonic` coverage**

Add to `disasm.rs` (create the file with just this test + `use` first):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::decode::OperandCount;

    #[test]
    fn mnemonics_cover_each_class() {
        assert_eq!(mnemonic(&OperandCount::Two, 0x14, 5), "add");
        assert_eq!(mnemonic(&OperandCount::Two, 0x0F, 5), "loadw");
        assert_eq!(mnemonic(&OperandCount::One, 0x00, 5), "jz");
        assert_eq!(mnemonic(&OperandCount::One, 0x0B, 5), "ret");
        assert_eq!(mnemonic(&OperandCount::Zero, 0x00, 5), "rtrue");
        assert_eq!(mnemonic(&OperandCount::Zero, 0x02, 5), "print");
        assert_eq!(mnemonic(&OperandCount::Var, 0x00, 5), "call_vs");
        assert_eq!(mnemonic(&OperandCount::Var, 0x04, 5), "aread");
        assert_eq!(mnemonic(&OperandCount::Var, 0x04, 3), "sread");
        assert_eq!(mnemonic(&OperandCount::One, 0x0F, 5), "call_1n");
        assert_eq!(mnemonic(&OperandCount::One, 0x0F, 4), "not");
        assert_eq!(mnemonic(&OperandCount::Ext, 0x09, 5), "save_undo");
        // Unknown falls back to a hex label, never panics.
        assert!(mnemonic(&OperandCount::Two, 0x7F, 5).starts_with("op:"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm disasm::tests::mnemonics_cover_each_class`
Expected: FAIL (compile error — `mnemonic` not defined).

- [ ] **Step 3: Implement the mnemonic table**

Write the module body (above the test). **The table below is a starting point copied from the Z-Machine Standards Document §14; the implementer MUST verify every arm against ZMSD §14 and the crate's `execute()` dispatch in `exec.rs` before moving on** (see Step 8's fixture test as the oracle). Version-dependent names are handled where the standard differs.

```rust
//! Public Z-machine disassembler: decoded `Instr` → human-readable text.
//! Zero-dependency. Verified against ZMSD §14 + this crate's `execute()` dispatch.

use crate::cpu::decode::{decode, Branch, Instr, Operand, OperandCount};
use crate::memory::Memory;

/// Mnemonic for a decoded instruction, version-aware. Hex fallback (never panics)
/// for opcodes with no assigned name in this version.
pub fn mnemonic(count: &OperandCount, opcode: u8, version: u8) -> &'static str {
    match count {
        OperandCount::Two => match opcode {
            0x01 => "je", 0x02 => "jl", 0x03 => "jg", 0x04 => "dec_chk",
            0x05 => "inc_chk", 0x06 => "jin", 0x07 => "test", 0x08 => "or",
            0x09 => "and", 0x0A => "test_attr", 0x0B => "set_attr",
            0x0C => "clear_attr", 0x0D => "store", 0x0E => "insert_obj",
            0x0F => "loadw", 0x10 => "loadb", 0x11 => "get_prop",
            0x12 => "get_prop_addr", 0x13 => "get_next_prop", 0x14 => "add",
            0x15 => "sub", 0x16 => "mul", 0x17 => "div", 0x18 => "mod",
            0x19 => "call_2s", 0x1A => "call_2n", 0x1B => "set_colour",
            0x1C => "throw",
            _ => "op:2op",
        },
        OperandCount::One => match opcode {
            0x00 => "jz", 0x01 => "get_sibling", 0x02 => "get_child",
            0x03 => "get_parent", 0x04 => "get_prop_len", 0x05 => "inc",
            0x06 => "dec", 0x07 => "print_addr", 0x08 => "call_1s",
            0x09 => "remove_obj", 0x0A => "print_obj", 0x0B => "ret",
            0x0C => "jump", 0x0D => "print_paddr", 0x0E => "load",
            0x0F => if version >= 5 { "call_1n" } else { "not" },
            _ => "op:1op",
        },
        OperandCount::Zero => match opcode {
            0x00 => "rtrue", 0x01 => "rfalse", 0x02 => "print",
            0x03 => "print_ret", 0x04 => "nop", 0x05 => "save",
            0x06 => "restore", 0x07 => "restart", 0x08 => "ret_popped",
            0x09 => if version >= 5 { "catch" } else { "pop" },
            0x0A => "quit", 0x0B => "new_line", 0x0C => "show_status",
            0x0D => "verify", 0x0E => "extended", 0x0F => "piracy",
            _ => "op:0op",
        },
        OperandCount::Var => match opcode {
            0x00 => "call_vs", 0x01 => "storew", 0x02 => "storeb",
            0x03 => "put_prop", 0x04 => if version >= 5 { "aread" } else { "sread" },
            0x05 => "print_char", 0x06 => "print_num", 0x07 => "random",
            0x08 => "push", 0x09 => "pull", 0x0A => "split_window",
            0x0B => "set_window", 0x0C => "call_vs2", 0x0D => "erase_window",
            0x0E => "erase_line", 0x0F => "set_cursor", 0x10 => "get_cursor",
            0x11 => "set_text_style", 0x12 => "buffer_mode", 0x13 => "output_stream",
            0x14 => "input_stream", 0x15 => "sound_effect", 0x16 => "read_char",
            0x17 => "scan_table", 0x18 => "not", 0x19 => "call_vn",
            0x1A => "call_vn2", 0x1B => "tokenise", 0x1C => "encode_text",
            0x1D => "copy_table", 0x1E => "print_table", 0x1F => "check_arg_count",
            _ => "op:var",
        },
        OperandCount::Ext => match opcode {
            0x00 => "save", 0x01 => "restore", 0x02 => "log_shift",
            0x03 => "art_shift", 0x04 => "set_font", 0x05 => "draw_picture",
            0x06 => "picture_data", 0x07 => "erase_picture", 0x08 => "set_margins",
            0x09 => "save_undo", 0x0A => "restore_undo", 0x0B => "print_unicode",
            0x0C => "check_unicode", 0x0D => "set_true_colour", 0x10 => "move_window",
            0x11 => "window_size", 0x12 => "window_style", 0x13 => "get_wind_prop",
            0x14 => "scroll_window", 0x15 => "pop_stack", 0x16 => "read_mouse",
            0x17 => "mouse_window", 0x18 => "push_stack", 0x19 => "put_wind_prop",
            0x1A => "print_form", 0x1B => "make_menu", 0x1C => "picture_table",
            0x1D => "buffer_screen",
            _ => "op:ext",
        },
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p zvm disasm::tests::mnemonics_cover_each_class`
Expected: PASS.

- [ ] **Step 5: Write the failing test for `format_instr`**

```rust
#[test]
fn formats_operands_store_and_branch() {
    use crate::cpu::decode::{Form, Instr};
    // add local1, #05 -> sp   (2OP:0x14, store to var 0)
    let instr = Instr {
        opcode: 0x14, form: Form::Long, operand_count: OperandCount::Two,
        operands: vec![Operand::Var(1), Operand::Small(5)],
        store: Some(0), branch: None, text: None, next_pc: 0x1000,
    };
    let s = format_instr(&instr, 5);
    assert!(s.starts_with("add "), "got {s:?}");
    assert!(s.contains("local0"), "got {s:?}");
    assert!(s.contains("#05"), "got {s:?}");
    assert!(s.contains("-> sp"), "got {s:?}");
}
```

- [ ] **Step 6: Implement `format_instr`, `disassemble`, `next_instr`**

```rust
fn fmt_operand(op: &Operand) -> String {
    match op {
        Operand::Large(n) => format!("#{:04x}", n),
        Operand::Small(n) => format!("#{:02x}", n),
        Operand::Var(0) => "sp".to_string(),
        Operand::Var(n @ 1..=15) => format!("local{}", n - 1),
        Operand::Var(n) => format!("g{:02x}", n - 16),
    }
}

fn fmt_var(v: u8) -> String {
    match v {
        0 => "sp".to_string(),
        1..=15 => format!("local{}", v - 1),
        n => format!("g{:02x}", n - 16),
    }
}

fn fmt_branch(b: &Branch, next_pc: u32) -> String {
    let neg = if b.on_true { "" } else { "~" };
    let target = match b.offset {
        0 => "rfalse".to_string(),
        1 => "rtrue".to_string(),
        off => format!("0x{:06x}", (next_pc as i64 + off as i64 - 2) as u32),
    };
    format!(" ?{}{}", neg, target)
}

/// Format one decoded instruction as "mnemonic op, op -> store ?branch [\"text\"]".
pub fn format_instr(instr: &Instr, version: u8) -> String {
    let mut s = mnemonic(&instr.operand_count, instr.opcode, version).to_string();
    if !instr.operands.is_empty() {
        let ops: Vec<String> = instr.operands.iter().map(fmt_operand).collect();
        s.push(' ');
        s.push_str(&ops.join(", "));
    }
    if let Some(v) = instr.store {
        s.push_str(&format!(" -> {}", fmt_var(v)));
    }
    if let Some(b) = &instr.branch {
        s.push_str(&fmt_branch(b, instr.next_pc));
    }
    if let Some((text, _)) = &instr.text {
        s.push_str(&format!(" {:?}", text));
    }
    s
}

/// Disassemble `lines` instructions starting at `start`, each prefixed with its
/// address. Stops early at the end of memory (never reads out of range).
pub fn disassemble(mem: &Memory, start: u32, version: u8, lines: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines);
    let mut pc = start.min(mem.len() as u32);
    for _ in 0..lines {
        if pc >= mem.len() as u32 {
            break;
        }
        let instr = decode(mem, pc, version);
        out.push(format!("{:06x}  {}", pc, format_instr(&instr, version)));
        // Guard against a decoder that fails to advance (malformed bytes).
        pc = if instr.next_pc > pc { instr.next_pc } else { pc + 1 };
    }
    out
}

/// Address of the instruction following the one at `addr` (clamped to memory).
pub fn next_instr(mem: &Memory, addr: u32, version: u8) -> u32 {
    if addr >= mem.len() as u32 {
        return mem.len() as u32;
    }
    let n = decode(mem, addr, version).next_pc;
    n.min(mem.len() as u32).max(addr + 1)
}
```

Add `pub mod disasm;` to `crates/zvm/src/cpu/mod.rs`.

- [ ] **Step 7: Run the format + module tests**

Run: `cargo test -p zvm disasm`
Expected: PASS (both tests).

- [ ] **Step 8: Add the fixture-decode oracle test (verification backstop)**

Use the crate's fixtures module (`zvm::fixtures`) or a minimal in-test story to decode a known routine and assert the disassembly is sensible (non-empty, addresses monotonically increase, at least one real mnemonic — not all `op:` fallbacks). This is the guard that the table in Step 3 was verified, not trusted:

```rust
#[test]
fn disassembles_a_real_routine_without_all_fallbacks() {
    let mem = crate::fixtures::minimal_story(); // adjust to the actual fixtures API
    let start = mem.read_word(0x06) as u32; // initial PC from header (v1-5)
    let lines = disassemble(&mem, start, mem.version(), 8);
    assert!(!lines.is_empty());
    assert!(lines.iter().any(|l| !l.contains("op:")), "all fallbacks: {lines:?}");
}
```

If `fixtures` has no ready story, load a bundled test blorb path used elsewhere in zvm tests (grep zvm tests for how they build a `Memory`). Adjust the header-PC read to the crate's helper if one exists.

- [ ] **Step 9: Run full zvm suite + zero-dep check**

Run: `cargo test -p zvm && cargo tree -p zvm --edges normal`
Expected: all zvm tests PASS; `cargo tree` shows **no** external dependencies.

- [ ] **Step 10: Commit**

```bash
git add crates/zvm/src/cpu/disasm.rs crates/zvm/src/cpu/mod.rs
git commit -m "feat(zvm): public Z-machine disassembler (mnemonic table + format_instr)"
```

---

### Task 2: Widen the `Debugger` trait (inspect-only, engine-neutral)

**Files:**
- Modify: `crates/app/src/engine.rs` (`Debugger` trait ~line 344; `Engine::debugger` default ~line 537)
- Test: inline `#[cfg(test)]` in `engine.rs`

**Interfaces:**
- Produces: the read-only `Debugger` trait (below); `Engine::debugger(&self) -> Option<&dyn Debugger>` default `None`.
- Note: this changes `debugger` from `&mut self -> Option<&mut dyn Debugger>` to `&self -> Option<&dyn Debugger>` (mirrors `introspect`). Recon confirmed no impls and no callers of the old signature, so this is safe.

- [ ] **Step 1: Write the failing test**

Add to `engine.rs` tests:

```rust
#[cfg(test)]
mod debugger_trait_tests {
    use super::*;

    struct Dummy;
    impl Debugger for Dummy {
        fn pc(&self) -> u32 { 0x4a2f }
        fn disassemble(&self, _a: u32, _n: usize) -> Vec<String> { vec!["4a2f  add".into()] }
        fn next_instr(&self, a: u32) -> u32 { a + 4 }
        fn stack_lines(&self) -> Vec<String> { vec!["#0 main".into()] }
        fn locals_lines(&self) -> Vec<String> { vec!["(none)".into()] }
        fn globals_lines(&self) -> Vec<String> { vec!["g00=0000".into()] }
        fn object_tree_lines(&self) -> Vec<String> { vec!["[1] thing".into()] }
        fn dictionary_lines(&self) -> Vec<String> { vec!["word".into()] }
        fn memory_hex(&self, _a: u32, _r: usize) -> Vec<String> { vec!["000000  00".into()] }
        fn memory_len(&self) -> u32 { 0x10000 }
    }

    #[test]
    fn debugger_object_is_usable() {
        let d = Dummy;
        let dyn_d: &dyn Debugger = &d;
        assert_eq!(dyn_d.pc(), 0x4a2f);
        assert_eq!(dyn_d.next_instr(0x4a2f), 0x4a33);
        assert!(!dyn_d.disassemble(0, 4).is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib debugger_trait_tests`
Expected: FAIL (the new trait methods don't exist).

- [ ] **Step 3: Replace the `Debugger` trait**

Replace the stub at `engine.rs:344-349` with:

```rust
/// Read-only debug inspection of a running engine. All methods return
/// pre-formatted lines so the app render code stays engine-neutral (mirrors
/// `Engine::window_dump`). Z-machine implements this; other engines return
/// `None` from `Engine::debugger` for now. (Inspect-only; a stepper is a
/// future increment that will add `&mut` control methods.)
pub trait Debugger {
    /// Instruction pointer the VM is parked at (for "jump to PC").
    fn pc(&self) -> u32;
    /// Disassemble `lines` instructions starting at `addr`, one string per line.
    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String>;
    /// Address of the instruction after the one at `addr` (clamped to memory);
    /// lets the panel advance the disassembly view by whole instructions.
    fn next_instr(&self, addr: u32) -> u32;
    /// Call stack, one or more lines per frame, innermost last.
    fn stack_lines(&self) -> Vec<String>;
    /// Locals of the innermost frame.
    fn locals_lines(&self) -> Vec<String>;
    /// Global variables, formatted.
    fn globals_lines(&self) -> Vec<String>;
    /// The object tree, indented.
    fn object_tree_lines(&self) -> Vec<String>;
    /// Dictionary words.
    fn dictionary_lines(&self) -> Vec<String>;
    /// Hex+ASCII dump: `rows` rows of 16 bytes from `addr`.
    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String>;
    /// Total addressable memory length (so the panel can clamp scroll).
    fn memory_len(&self) -> u32;
}
```

Change the `Engine::debugger` default at `engine.rs:537-539` to:

```rust
    /// Debug-inspection capability, when the engine has one.
    fn debugger(&self) -> Option<&dyn Debugger> {
        None
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib debugger_trait_tests`
Expected: PASS.

- [ ] **Step 5: Confirm the whole app crate still builds**

Run: `cargo build -p app`
Expected: builds clean (no other code referenced the old `Debugger`/`debugger` signature).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/engine.rs
git commit -m "feat(app): widen Debugger trait to a read-only inspection seam"
```

---

### Task 3: Implement `Debugger` for `GameSession` (zvm)

**Files:**
- Modify: `crates/app/src/session.rs` (add `impl Debugger for GameSession`; add `debugger()` override inside `impl Engine for GameSession` near the `introspect()` override at ~line 996)
- Test: inline `#[cfg(test)]` in `session.rs`

**Interfaces:**
- Consumes: `zvm::cpu::disasm::{disassemble, next_instr}` (Task 1), `Debugger` trait (Task 2), `self.machine` (`GameSession.machine: Machine`, `session.rs:257`), `zvm::object_tree_view`, `zvm::dictionary::load`, `zvm::objects::get_parent`, `zvm::memory::Memory` accessors.
- Produces: `GameSession: Debugger`; `Engine::debugger()` returns `Some(self)` for zvm.

Model the impl on the existing `impl Introspect for GameSession` (`session.rs:1001-1028`).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod debugger_impl_tests {
    use super::*;
    use crate::engine::Engine;

    fn zvm_session() -> GameSession {
        // Reuse whatever helper the existing session tests use to build a
        // GameSession from a fixture story. Grep this file for how other
        // #[test] fns construct a GameSession and copy that.
        crate::session::tests::fixture_session() // adjust to the real helper
    }

    #[test]
    fn zvm_exposes_a_debugger() {
        let s = zvm_session();
        let d = s.debugger().expect("zvm has a debugger");
        assert_eq!(d.pc(), s.machine.state.pc);
        assert_eq!(d.globals_lines().len(), 240);
        assert!(!d.dictionary_lines().is_empty());
        assert!(!d.object_tree_lines().is_empty());
        assert_eq!(d.memory_len(), s.machine.mem.len() as u32);
        let hex = d.memory_hex(0, 2);
        assert_eq!(hex.len(), 2);
        assert!(hex[0].starts_with("000000"));
    }
}
```

If there is no shared fixture helper, build the session the same way an existing `session.rs` test does (grep for `GameSession {` or a `new`/`load` constructor in the test module).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib debugger_impl_tests`
Expected: FAIL (`debugger()` returns `None`; trait not implemented).

- [ ] **Step 3: Implement `impl Debugger for GameSession`**

Add near the `impl Introspect for GameSession` block (`session.rs:~1028`):

```rust
impl Debugger for GameSession {
    fn pc(&self) -> u32 {
        self.machine.state.pc
    }

    fn disassemble(&self, addr: u32, lines: usize) -> Vec<String> {
        let version = self.machine.mem.version();
        zvm::cpu::disasm::disassemble(&self.machine.mem, addr, version, lines)
    }

    fn next_instr(&self, addr: u32) -> u32 {
        let version = self.machine.mem.version();
        zvm::cpu::disasm::next_instr(&self.machine.mem, addr, version)
    }

    fn stack_lines(&self) -> Vec<String> {
        let st = &self.machine.state;
        if st.frames.is_empty() {
            return vec!["(no frames)".to_string()];
        }
        let mut out = Vec::with_capacity(st.frames.len());
        for (i, f) in st.frames.iter().enumerate() {
            let locals: Vec<String> = f.locals.iter().map(|w| format!("{:04x}", w)).collect();
            out.push(format!(
                "#{i}  fn@{:06x}  ret={:06x}  args={}  locals=[{}]",
                f.func_addr, f.return_pc, f.arg_count, locals.join(",")
            ));
        }
        out
    }

    fn locals_lines(&self) -> Vec<String> {
        match self.machine.state.frames.last() {
            None => vec!["(no frame)".to_string()],
            Some(f) if f.locals.is_empty() => vec!["(none)".to_string()],
            Some(f) => f.locals.iter().enumerate()
                .map(|(i, w)| format!("local{i} = {:04x}  ({})", w, w))
                .collect(),
        }
    }

    fn globals_lines(&self) -> Vec<String> {
        (0u8..240).map(|n| format!("g{:02x} = {:04x}", n, self.machine.global(n))).collect()
    }

    fn object_tree_lines(&self) -> Vec<String> {
        // Indent each object by its depth in the parent chain.
        let mem = &self.machine.mem;
        let snaps = zvm::object_tree_view(&self.machine);
        snaps.iter().map(|s| {
            let mut depth = 0usize;
            let mut p = s.parent;
            while p != 0 && depth < 32 {
                depth += 1;
                p = zvm::objects::get_parent(mem, p);
            }
            format!("{}[{}] {}", "  ".repeat(depth), s.number, s.name)
        }).collect()
    }

    fn dictionary_lines(&self) -> Vec<String> {
        zvm::dictionary::load(&self.machine.mem).words(&self.machine.mem)
    }

    fn memory_hex(&self, addr: u32, rows: usize) -> Vec<String> {
        let bytes = self.machine.mem.raw_bytes();
        let len = bytes.len() as u32;
        let mut out = Vec::with_capacity(rows);
        let mut a = addr.min(len);
        for _ in 0..rows {
            if a >= len { break; }
            let end = (a + 16).min(len);
            let row = &bytes[a as usize..end as usize];
            let hex: String = row.iter().map(|b| format!("{:02x} ", b)).collect();
            let ascii: String = row.iter()
                .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
                .collect();
            out.push(format!("{:06x}  {:<48}{}", a, hex, ascii));
            a = end;
        }
        out
    }

    fn memory_len(&self) -> u32 {
        self.machine.mem.len() as u32
    }
}
```

Add the `debugger()` override inside `impl Engine for GameSession` (right after the `introspect()` override at `session.rs:996-998`):

```rust
    fn debugger(&self) -> Option<&dyn Debugger> {
        Some(self)
    }
```

Ensure `Debugger` is imported (add to the existing `use crate::engine::{...}` line in `session.rs`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib debugger_impl_tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/session.rs
git commit -m "feat(app): implement Debugger for the zvm GameSession"
```

---

### Task 4: `DebugPanelState` + navigation logic (pure, unit-tested)

**Files:**
- Create: `crates/app/src/debug_panel.rs`
- Modify: `crates/app/src/lib.rs` or `main.rs` module list (add `pub mod debug_panel;` / `mod debug_panel;` wherever sibling modules like `inventory`/`slash` are declared)
- Modify: `crates/app/src/state.rs` (add `debug_panel: Option<DebugPanelState>` field to `OverlayState` at ~line 1283)
- Test: inline `#[cfg(test)]` in `debug_panel.rs`

**Interfaces:**
- Consumes: `crate::engine::Debugger` (Task 2).
- Produces:
  - `pub struct DebugPanelState`, `pub enum DebugPane`, `pub enum DebugView`, `pub enum DebugKey { Consumed, Ignored, Close }`
  - `DebugPanelState::new(pc: u32) -> Self`
  - `DebugPanelState::refresh(&mut self, dbg: &dyn Debugger)`
  - `DebugPanelState::handle_key(&mut self, code: crossterm::event::KeyCode, dbg: &dyn Debugger) -> DebugKey`
  - `pub const DISASM_WINDOW: usize` / `MEM_WINDOW` (over-compute counts for the address-windowed panes)

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;

    // Minimal mock: 4-byte fixed instructions, 0x10000 bytes of memory.
    struct MockDbg;
    impl crate::engine::Debugger for MockDbg {
        fn pc(&self) -> u32 { 0x1000 }
        fn disassemble(&self, addr: u32, n: usize) -> Vec<String> {
            (0..n).map(|i| format!("{:06x}  add", addr + i as u32 * 4)).collect()
        }
        fn next_instr(&self, a: u32) -> u32 { a + 4 }
        fn stack_lines(&self) -> Vec<String> { vec!["#0 main".into()] }
        fn locals_lines(&self) -> Vec<String> { vec!["(none)".into()] }
        fn globals_lines(&self) -> Vec<String> { (0..240).map(|i| format!("g{i:02x}")).collect() }
        fn object_tree_lines(&self) -> Vec<String> { vec!["[1] thing".into()] }
        fn dictionary_lines(&self) -> Vec<String> { vec!["word".into()] }
        fn memory_hex(&self, a: u32, r: usize) -> Vec<String> {
            (0..r).map(|i| format!("{:06x}", a + i as u32 * 16)).collect()
        }
        fn memory_len(&self) -> u32 { 0x10000 }
    }

    #[test]
    fn tab_cycles_focus_with_view_rollover_and_shift_tab_reverses() {
        let mut p = DebugPanelState::new(0x1000);
        assert_eq!(p.focus, DebugPane::Disasm);
        assert_eq!(p.focus.view(), DebugView::Execution);
        p.handle_key(KeyCode::Tab, &MockDbg); // -> Locals
        p.handle_key(KeyCode::Tab, &MockDbg); // -> Stack
        p.handle_key(KeyCode::Tab, &MockDbg); // -> Globals (rolls into WorldState)
        assert_eq!(p.focus, DebugPane::Globals);
        assert_eq!(p.focus.view(), DebugView::WorldState);
        p.handle_key(KeyCode::BackTab, &MockDbg); // back to Stack
        assert_eq!(p.focus, DebugPane::Stack);
    }

    #[test]
    fn disasm_scroll_advances_by_instruction_and_up_pops_history() {
        let mut p = DebugPanelState::new(0x1000);
        // focus is Disasm by default
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1008);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1004);
        p.handle_key(KeyCode::Up, &MockDbg);
        assert_eq!(p.disasm_addr, 0x1000);
        p.handle_key(KeyCode::Up, &MockDbg); // history empty -> no-op
        assert_eq!(p.disasm_addr, 0x1000);
    }

    #[test]
    fn goto_pc_resets_disasm_and_esc_closes() {
        let mut p = DebugPanelState::new(0x1000);
        p.handle_key(KeyCode::Down, &MockDbg);
        assert_eq!(p.handle_key(KeyCode::Char('g'), &MockDbg), DebugKey::Consumed);
        assert_eq!(p.disasm_addr, 0x1000);
        assert!(p.disasm_history.is_empty());
        assert_eq!(p.handle_key(KeyCode::Esc, &MockDbg), DebugKey::Close);
    }

    #[test]
    fn memory_scroll_clamps_at_memory_len() {
        let mut p = DebugPanelState::new(0x1000);
        p.focus = DebugPane::Memory;
        p.mem_addr = 0x10000 - 16;
        p.handle_key(KeyCode::Down, &MockDbg);
        assert!(p.mem_addr < 0x10000);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app --lib debug_panel`
Expected: FAIL (module/types not defined).

- [ ] **Step 3: Implement `debug_panel.rs`**

```rust
//! Full-screen Z-machine debug inspector — panel state + navigation logic.
//! Pure over the `Debugger` trait (engine-neutral); the render code paints the
//! snapshot this holds. No `zvm::` calls here.

use crate::engine::Debugger;
use crossterm::event::KeyCode;

/// How many instructions / memory rows to pre-render for the address-windowed
/// panes (draw clips to the pane height; over-computing avoids threading height
/// into refresh).
pub const DISASM_WINDOW: usize = 256;
pub const MEM_WINDOW: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugView { Execution, WorldState }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugPane { Disasm, Locals, Stack, Globals, Objects, Dict, Memory }

impl DebugPane {
    /// Cycle order across both views (Tab walks this; rollover is implicit).
    const ORDER: [DebugPane; 7] = [
        DebugPane::Disasm, DebugPane::Locals, DebugPane::Stack,   // Execution
        DebugPane::Globals, DebugPane::Objects, DebugPane::Dict, DebugPane::Memory, // WorldState
    ];
    pub fn view(self) -> DebugView {
        match self {
            DebugPane::Disasm | DebugPane::Locals | DebugPane::Stack => DebugView::Execution,
            _ => DebugView::WorldState,
        }
    }
    fn cycle(self, dir: i32) -> DebugPane {
        let idx = Self::ORDER.iter().position(|&p| p == self).unwrap() as i32;
        let n = Self::ORDER.len() as i32;
        Self::ORDER[(idx + dir).rem_euclid(n) as usize]
    }
}

/// The formatted lines the render code paints, refreshed from the Debugger.
#[derive(Debug, Default, Clone)]
pub struct DebugSnapshot {
    pub disasm: Vec<String>,
    pub locals: Vec<String>,
    pub stack: Vec<String>,
    pub globals: Vec<String>,
    pub objects: Vec<String>,
    pub dict: Vec<String>,
    pub memory: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DebugPanelState {
    pub focus: DebugPane,
    pub disasm_addr: u32,
    pub disasm_history: Vec<u32>,
    pub mem_addr: u32,
    /// Scroll offset for the list panes (locals/stack/globals/objects/dict).
    pub list_scroll: usize,
    /// Focused-pane height captured by the last draw (for paging). 1 until drawn.
    pub viewport: usize,
    pub snapshot: DebugSnapshot,
}

/// Result of a keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugKey { Consumed, Ignored, Close }

impl DebugPanelState {
    pub fn new(pc: u32) -> Self {
        DebugPanelState {
            focus: DebugPane::Disasm,
            disasm_addr: pc,
            disasm_history: Vec::new(),
            mem_addr: 0,
            list_scroll: 0,
            viewport: 1,
            snapshot: DebugSnapshot::default(),
        }
    }

    /// Recompute the snapshot for the current cursor positions.
    pub fn refresh(&mut self, dbg: &dyn Debugger) {
        self.snapshot.disasm = dbg.disassemble(self.disasm_addr, DISASM_WINDOW);
        self.snapshot.locals = dbg.locals_lines();
        self.snapshot.stack = dbg.stack_lines();
        self.snapshot.globals = dbg.globals_lines();
        self.snapshot.objects = dbg.object_tree_lines();
        self.snapshot.dict = dbg.dictionary_lines();
        self.snapshot.memory = dbg.memory_hex(self.mem_addr, MEM_WINDOW);
    }

    fn page(&self) -> usize { self.viewport.max(1) }

    pub fn handle_key(&mut self, code: KeyCode, dbg: &dyn Debugger) -> DebugKey {
        match code {
            KeyCode::Esc => return DebugKey::Close,
            KeyCode::Tab => { self.focus = self.focus.cycle(1); self.list_scroll = 0; }
            KeyCode::BackTab => { self.focus = self.focus.cycle(-1); self.list_scroll = 0; }
            KeyCode::Char('g') => {
                self.disasm_history.clear();
                self.disasm_addr = dbg.pc();
            }
            KeyCode::Down | KeyCode::Up | KeyCode::PageDown | KeyCode::PageUp
            | KeyCode::Home | KeyCode::End => {
                let step = matches!(code, KeyCode::PageDown | KeyCode::PageUp)
                    .then(|| self.page()).unwrap_or(1);
                let down = matches!(code, KeyCode::Down | KeyCode::PageDown | KeyCode::End);
                match self.focus {
                    DebugPane::Disasm => self.scroll_disasm(down, step, dbg),
                    DebugPane::Memory => self.scroll_memory(down, step, dbg),
                    _ => self.scroll_list(code),
                }
            }
            _ => return DebugKey::Ignored,
        }
        self.refresh(dbg);
        DebugKey::Consumed
    }

    fn scroll_disasm(&mut self, down: bool, step: usize, dbg: &dyn Debugger) {
        for _ in 0..step {
            if down {
                let next = dbg.next_instr(self.disasm_addr);
                if next > self.disasm_addr {
                    self.disasm_history.push(self.disasm_addr);
                    self.disasm_addr = next;
                }
            } else if let Some(prev) = self.disasm_history.pop() {
                self.disasm_addr = prev;
            }
        }
    }

    fn scroll_memory(&mut self, down: bool, step: usize, dbg: &dyn Debugger) {
        let delta = (16 * step) as u32;
        if down {
            let max = dbg.memory_len().saturating_sub(16);
            self.mem_addr = (self.mem_addr + delta).min(max);
        } else {
            self.mem_addr = self.mem_addr.saturating_sub(delta);
        }
    }

    fn scroll_list(&mut self, code: KeyCode) {
        let len = self.focused_list_len();
        let vp = self.page();
        let max = len.saturating_sub(1);
        self.list_scroll = match code {
            KeyCode::Down => (self.list_scroll + 1).min(max),
            KeyCode::Up => self.list_scroll.saturating_sub(1),
            KeyCode::PageDown => (self.list_scroll + vp).min(max),
            KeyCode::PageUp => self.list_scroll.saturating_sub(vp),
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => self.list_scroll,
        };
    }

    fn focused_list_len(&self) -> usize {
        match self.focus {
            DebugPane::Locals => self.snapshot.locals.len(),
            DebugPane::Stack => self.snapshot.stack.len(),
            DebugPane::Globals => self.snapshot.globals.len(),
            DebugPane::Objects => self.snapshot.objects.len(),
            DebugPane::Dict => self.snapshot.dict.len(),
            _ => 0,
        }
    }
}
```

Add the module declaration alongside the other app modules (grep `main.rs`/`lib.rs` for `mod slash;` and add `mod debug_panel;` there; if the app exposes a `lib.rs` with `pub mod`, match that).

Add the field to `OverlayState` (`state.rs:~1283`, after `style_editor`):

```rust
    /// Active debug-inspector panel. `None` = closed.
    pub debug_panel: Option<crate::debug_panel::DebugPanelState>,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app --lib debug_panel`
Expected: PASS (all four tests).

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/debug_panel.rs crates/app/src/state.rs crates/app/src/main.rs
git commit -m "feat(app): DebugPanelState + navigation logic (inspect-only)"
```

(Adjust the staged module-declaration file if it's `lib.rs` rather than `main.rs`.)

---

### Task 5: Theme selectors for the debug panel

**Files:**
- Modify: `crates/app/src/colors.rs` (`ColorScheme` struct ~line 220; its `Default`/constructor)
- Modify: `crates/app/src/style.rs` (read match ~line 286; write/patch match ~line 436; selector registry array ~line 189-256)
- Test: inline `#[cfg(test)]` in `style.rs`

**Interfaces:**
- Produces: `ColorScheme.debug_pane`, `.debug_pane_focused`, `.debug_title` (all `ratatui::style::Style`), reachable via selectors `"debug_pane"`, `"debug_pane:focused"`, `"debug_title"`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn debug_selectors_resolve_and_patch() {
    let cs = crate::colors::ColorScheme::default();
    // read direction: selector maps to a field (no panic, returns a Style)
    let _ = style_for_selector(&cs, "debug_pane");
    let _ = style_for_selector(&cs, "debug_pane:focused");
    let _ = style_for_selector(&cs, "debug_title");
    // registry lists them (so the style editor shows them)
    assert!(ALL_SELECTORS.contains(&"debug_pane"));   // adjust to the real registry name
    assert!(ALL_SELECTORS.contains(&"debug_title"));
}
```

(Adjust `ALL_SELECTORS` to the actual registry identifier near `style.rs:189-256`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib debug_selectors_resolve_and_patch`
Expected: FAIL.

- [ ] **Step 3: Add the fields + register the selectors**

In `colors.rs`, add to `ColorScheme` (near `dialog`/`dialog_title` fields):

```rust
    /// Debug-inspector pane body (unfocused).
    pub debug_pane: Style,
    /// Debug-inspector pane body/border when focused.
    pub debug_pane_focused: Style,
    /// Debug-inspector pane title.
    pub debug_title: Style,
```

Initialize them in the `ColorScheme` `Default` (or wherever `dialog`/`dialog_title` are seeded) — sensible defaults reusing existing theme colors:

```rust
            debug_pane: Style::default(),
            debug_pane_focused: Style::default().add_modifier(Modifier::BOLD),
            debug_title: Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
```

In `style.rs` **read** match (`style_for_selector`, ~line 286):

```rust
        "debug_pane"          => cs.debug_pane,
        "debug_pane:focused"  => cs.debug_pane_focused,
        "debug_title"         => cs.debug_title,
```

In `style.rs` **write/patch** match (~line 436):

```rust
            "debug_pane"          => cs.debug_pane = cs.debug_pane.patch(style),
            "debug_pane:focused"  => cs.debug_pane_focused = cs.debug_pane_focused.patch(style),
            "debug_title"         => cs.debug_title = cs.debug_title.patch(style),
```

In the **selector registry** array (~line 189-256) add the three names:

```rust
    "debug_pane",
    "debug_pane:focused",
    "debug_title",
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib debug_selectors_resolve_and_patch`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/style.rs
git commit -m "feat(app): themeable selectors for the debug inspector panes"
```

---

### Task 6: Render the debug panel

**Files:**
- Create: `crates/app/src/render/debug_panel.rs`
- Modify: `crates/app/src/render/mod.rs` (add `pub mod debug_panel;` and re-export `draw_debug_panel` if siblings are re-exported)
- Modify: `crates/app/src/overlays.rs` (add the draw call after the style-editor branch ~line 150)
- Test: inline `#[cfg(test)]` in `render/debug_panel.rs`

**Interfaces:**
- Consumes: `crate::debug_panel::{DebugPanelState, DebugPane, DebugView}` (Task 4), `ColorScheme.debug_*` (Task 5), render primitives `draw_str_clipped` (`render/mod.rs:134`), `draw_pane_frame` (`render/paneframe.rs:65`), `draw_top_inset` (`render/paneframe.rs:307`).
- Produces: `pub fn draw_debug_panel(state: &AppState, area: Rect, buf: &mut Buffer)`.

Follow the `style_editor` two-column split idiom (`render/style_editor.rs:99-108`) using explicit `Rect::new` arithmetic. The panel writes the focused-pane height back into `state.overlays.debug_panel.viewport` for paging (mirroring how `config_screen` writes the viewport back).

- [ ] **Step 1: Write the failing render smoke test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn buf_text(buf: &Buffer) -> String {
        buf.content.iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn draws_execution_view_panes() {
        let mut state = crate::state::AppState::default(); // adjust to the real ctor
        let mut panel = crate::debug_panel::DebugPanelState::new(0x1000);
        panel.snapshot.disasm = vec!["1000  add".into()];
        panel.snapshot.locals = vec!["local0 = 0001".into()];
        panel.snapshot.stack = vec!["#0 main".into()];
        state.overlays.debug_panel = Some(panel);

        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        draw_debug_panel(&state, area, &mut buf);

        let text = buf_text(&buf);
        assert!(text.contains("add"));
        assert!(text.contains("main"));
    }
}
```

If `AppState::default()` isn't available, construct it the way other `render/*` tests do (grep `render/` tests for an `AppState` builder).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib render::debug_panel`
Expected: FAIL.

- [ ] **Step 3: Implement `draw_debug_panel`**

```rust
//! Full-screen debug-inspector renderer. Paints the DebugPanelState snapshot.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::debug_panel::{DebugPane, DebugPanelState, DebugView};
use crate::render::draw_str_clipped;
use crate::render::paneframe::{draw_pane_frame, draw_top_inset, InsetSegment};
use crate::state::AppState;

/// One titled pane: border in the focused/unfocused style, snapshot lines inside.
fn draw_pane(
    buf: &mut Buffer, area: Rect, title: &str, lines: &[String], scroll: usize,
    focused: bool, state: &AppState,
) -> u16 {
    if area.width < 2 || area.height < 2 { return 0; }
    let border = if focused { state.colors.debug_pane_focused } else { state.colors.debug_pane };
    let pane = draw_pane_frame(buf, area, state.colors.dialog_box_style, &state.colors.dialog_glyphs, border);
    draw_top_inset(buf, pane.top_inset, &[InsetSegment { text: title, active: focused }],
        state.colors.debug_title, state.colors.debug_title);
    let inner = pane.inner; // adjust to the actual PaneFrame inner-rect field name
    let body = state.colors.debug_pane;
    for (row, line) in lines.iter().skip(scroll).take(inner.height as usize).enumerate() {
        draw_str_clipped(buf, inner.x, inner.y + row as u16, line, body, inner);
    }
    inner.height
}

pub fn draw_debug_panel(state: &AppState, area: Rect, buf: &mut Buffer) {
    let Some(panel) = &state.overlays.debug_panel else { return };
    let view = panel.focus.view();

    // Left column full height; right column split into two stacked panes.
    let left_w = area.width / 2;
    let right_x = area.x + left_w;
    let right_w = area.width - left_w;
    let top_h = area.height / 2;
    let left = Rect::new(area.x, area.y, left_w, area.height);
    let r_top = Rect::new(right_x, area.y, right_w, top_h);
    let r_bot = Rect::new(right_x, area.y + top_h, right_w, area.height - top_h);

    let s = &panel.snapshot;
    let f = panel.focus;
    let ls = panel.list_scroll;
    // Which pane is focused decides where list_scroll applies; address panes
    // (disasm/memory) scroll via their addr, so pass 0 for their offset.
    let mut focused_h = 0u16;
    match view {
        DebugView::Execution => {
            let h = draw_pane(buf, left, " Disassembly ", &s.disasm, 0, f == DebugPane::Disasm, state);
            if f == DebugPane::Disasm { focused_h = h; }
            let h = draw_pane(buf, r_top, " Locals ", &s.locals, if f == DebugPane::Locals { ls } else { 0 }, f == DebugPane::Locals, state);
            if f == DebugPane::Locals { focused_h = h; }
            let h = draw_pane(buf, r_bot, " Stack ", &s.stack, if f == DebugPane::Stack { ls } else { 0 }, f == DebugPane::Stack, state);
            if f == DebugPane::Stack { focused_h = h; }
        }
        DebugView::WorldState => {
            let h = draw_pane(buf, left, " Globals ", &s.globals, if f == DebugPane::Globals { ls } else { 0 }, f == DebugPane::Globals, state);
            if f == DebugPane::Globals { focused_h = h; }
            // Right-top shows Objects; when Dict/Memory focused it takes the top slot.
            let (top_title, top_lines, top_pane) = match f {
                DebugPane::Dict => (" Dictionary ", &s.dict, DebugPane::Dict),
                DebugPane::Memory => (" Memory ", &s.memory, DebugPane::Memory),
                _ => (" Objects ", &s.objects, DebugPane::Objects),
            };
            let off = if f == top_pane && top_pane != DebugPane::Memory { ls } else { 0 };
            let h = draw_pane(buf, r_top, top_title, top_lines, off, f == top_pane, state);
            if f == top_pane { focused_h = h; }
            // Right-bottom shows the other of Objects/Dictionary for context.
            let bot = if top_pane == DebugPane::Objects { (" Dictionary ", &s.dict) } else { (" Objects ", &s.objects) };
            draw_pane(buf, r_bot, bot.0, bot.1, 0, false, state);
        }
    }

    // Write the focused-pane height back for paging (interior-mutability-free:
    // this needs &mut; see Task 7 — the run loop sets viewport before draw, or
    // draw is given &mut. If AppState is &, capture into a Cell/ënoop here and
    // let Task 7 set panel.viewport from the pane rects instead).
    let _ = focused_h;
}
```

Note on `panel.viewport`: `draw_debug_panel` takes `&AppState`, so it cannot write back directly. Task 7 sets `viewport` from the known geometry (`area.height/2 - 2` for the stacked right panes, `area.height - 2` for the left) right after computing `area` in the run loop, before `handle_key`. Keep `draw_pane`'s returned height for a future refinement but don't rely on it for paging in v1.

Register the module in `render/mod.rs` (`pub mod debug_panel;`) and add the draw call in `overlays.rs` after the style-editor branch (~line 150):

```rust
    // ── Debug inspector — full-screen, drawn last ──
    if state.overlays.debug_panel.is_some() {
        crate::render::debug_panel::draw_debug_panel(state, dialog_area, buf);
    }
```

**Field-name caveats:** the exact `PaneFrame` inner-rect field (`inner` vs `content`) and the `InsetSegment` import path come from `render/paneframe.rs` — confirm against that file (recon showed `pub top_inset: Rect` at `paneframe.rs:44`; find the inner-region field beside it). Adjust `dialog_glyphs`/`dialog_box_style` field names against `colors.rs` if they differ.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p app --lib render::debug_panel`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/debug_panel.rs crates/app/src/render/mod.rs crates/app/src/overlays.rs
git commit -m "feat(app): render the debug inspector panel"
```

---

### Task 7: Wire `/debug` command, open/close, and per-turn refresh

**Files:**
- Modify: `crates/app/src/slash.rs` (`SlashOutcome` enum ~line 31; `COMMANDS` array ~line 158, near the `dump-windows`/`trace` entries ~line 435)
- Modify: `crates/app/src/slash_dispatch.rs` (`dispatch_slash_outcome` match ~line 45; model on the `DumpWindows` arm ~line 80)
- Modify: `crates/app/src/main.rs` (run-loop key intercept near the config-screen Tab intercept ~line 1518; post-turn refresh)
- Test: inline `#[cfg(test)]` in `slash.rs` and `slash_dispatch.rs`

**Interfaces:**
- Consumes: `Engine::debugger()` (Task 3), `DebugPanelState::{new, refresh, handle_key}` + `DebugKey` (Task 4), `state.overlays.debug_panel` (Task 4).
- Produces: `SlashOutcome::OpenDebug`; a `"debug"` `CommandSpec`; the dispatch arm; the run-loop key intercept + refresh.

- [ ] **Step 1: Write the failing parse test**

In `slash.rs` tests:

```rust
#[test]
fn debug_command_parses_to_open_debug() {
    assert_eq!(parse("debug"), SlashOutcome::OpenDebug); // adjust to the real parse entry point
}
```

(Match the existing parse-test style in `slash.rs`; `find_command`/`parse_in_context` is at `slash.rs:452-458`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app --lib debug_command_parses_to_open_debug`
Expected: FAIL.

- [ ] **Step 3: Add the `SlashOutcome` variant + `CommandSpec`**

In `slash.rs`, add to `SlashOutcome` (~line 31):

```rust
    /// Open the Z-machine debug inspector. Handled in `slash_dispatch` (needs
    /// AppState + the engine's debugger capability).
    OpenDebug,
```

Add to `COMMANDS` near `dump-windows` (~line 435):

```rust
    CommandSpec { name: "debug", category: Category::Help, context: Context::Global,
        usage: "debug", description: "open the Z-machine debug inspector (disassembly + live VM state)",
        dispatch: |_| SlashOutcome::OpenDebug },
```

- [ ] **Step 4: Run parse test to verify it passes**

Run: `cargo test -p app --lib debug_command_parses_to_open_debug`
Expected: PASS.

- [ ] **Step 5: Write the failing dispatch test**

In `slash_dispatch.rs` tests, model on any existing dispatch test. Assert: on an engine whose `debugger()` is `Some`, `OpenDebug` sets `state.overlays.debug_panel` to `Some`; on an engine whose `debugger()` is `None`, it stays `None` and a Meta transcript line is pushed.

```rust
#[test]
fn open_debug_opens_on_zvm_and_reports_on_others() {
    // Build a zvm GameSession + AppState the way sibling dispatch tests do.
    // (grep slash_dispatch.rs tests for the harness.)
    let mut state = /* AppState */;
    let mut session = /* GameSession (zvm) as Box<dyn Engine> or &mut dyn Engine */;
    dispatch_open_debug(&mut state, session.as_mut()); // or call dispatch_slash_outcome
    assert!(state.overlays.debug_panel.is_some());
}
```

(If wiring a full `dispatch_slash_outcome` call is heavy in a unit test, factor the arm body into a small `fn open_debug(state, session)` in `slash_dispatch.rs` and test that directly.)

- [ ] **Step 6: Add the dispatch arm**

In `dispatch_slash_outcome` (`slash_dispatch.rs`), add alongside `DumpWindows`:

```rust
        SlashOutcome::OpenDebug => {
            if let Some(dbg) = session.debugger() {
                let mut panel = crate::debug_panel::DebugPanelState::new(dbg.pc());
                panel.refresh(dbg);
                state.overlays.debug_panel = Some(panel);
            } else {
                state.push_transcript_internal(
                    "debugger not available for this engine", TranscriptKind::Meta);
            }
        }
```

- [ ] **Step 7: Run dispatch test**

Run: `cargo test -p app --lib open_debug`
Expected: PASS.

- [ ] **Step 8: Wire the run-loop key intercept + refresh**

In `main.rs`, add a debug-panel intercept near the config-screen Tab intercept (~line 1518), BEFORE the normal `key_to_command`/`apply_action` path. `session` is the engine binding in the loop (confirm the exact name — recon saw `session.submit(...)` at `main.rs:2022`):

```rust
        // ── Debug inspector: intercept keys while open ──
        if state.overlays.debug_panel.is_some() {
            if let Event::Key(k) = &event {
                if k.kind == KeyEventKind::Press {
                    // Set the focused-pane viewport from geometry for paging.
                    if let Some(p) = &mut state.overlays.debug_panel {
                        p.viewport = (dialog_area.height.saturating_sub(2) / 2).max(1) as usize;
                    }
                    let outcome = if let Some(dbg) = session.debugger() {
                        state.overlays.debug_panel.as_mut().map(|p| p.handle_key(k.code, dbg))
                    } else { Some(crate::debug_panel::DebugKey::Close) };
                    if let Some(dk) = outcome {
                        if dk == crate::debug_panel::DebugKey::Close {
                            state.overlays.debug_panel = None;
                        }
                        continue; // swallow the key; do not fall through to game/map
                    }
                }
            }
        }
```

Note: `session.debugger()` borrows `session` immutably and `state.overlays.debug_panel` mutably — disjoint borrows, fine. If the borrow checker objects to the `as_mut()` + `dbg` overlap, split into: take the panel out with `.take()`, call `handle_key`, then put it back unless `Close`.

Add the post-turn refresh so the open panel reflects new VM state — after each turn is applied (grep for where `apply_turn`/`finish_command_turn` results land in the loop, and after `apply_game_driven_result`):

```rust
        if let Some(p) = &mut state.overlays.debug_panel {
            if let Some(dbg) = session.debugger() {
                p.refresh(dbg);
            }
        }
```

- [ ] **Step 9: Build + full app suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean; all tests PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/slash.rs crates/app/src/slash_dispatch.rs crates/app/src/main.rs
git commit -m "feat(app): wire /debug command, open/close, and per-turn refresh"
```

---

### Task 8: Final integration check + docs

**Files:**
- Modify: `README.md` only if the debug inspector warrants a mention (per project policy README covers *major* features — a developer/curiosity inspector likely does NOT; skip unless it clearly qualifies).
- No new code.

- [ ] **Step 1: Clippy + full workspace tests**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: no clippy warnings; all tests pass across zvm + app.

- [ ] **Step 2: Zero-dep re-confirm**

Run: `cargo tree -p zvm --edges normal`
Expected: no external dependencies (the disassembler added none).

- [ ] **Step 3: Manual TTY smoke (record for the user to run)**

This is the only non-headless step — leave it for the user (no TTY in CI):
- `lanthorn <story>.z5`, play a turn, type `/debug`. The panel opens on the Execution view: Disassembly (left), Locals + Stack (right).
- `Tab`/`Shift-Tab` cycles focus and rolls into the World-state view (Globals + Objects/Dictionary/Memory). Arrows/PgUp/PgDn scroll the focused pane; in Disassembly, `↓` advances by instruction and `↑` walks back; `g` jumps to PC. `Esc` closes.
- On a Glulx (`.gblorb`) game, `/debug` prints "debugger not available for this engine" and opens no panel.
- Confirm the panes pick up `style.toml` overrides for `debug_pane` / `debug_pane:focused` / `debug_title`.

- [ ] **Step 4: Set the quest to confirm**

The feature is merged but has an unexercised-in-CI TTY surface, so it takes the `confirm` status (per the project's verification policy). Use the on-PATH CLI:

```bash
side-quest status SQ-0169 confirm
```

---

## Self-Review notes (for the executor)

- **Spec coverage:** disassembly (T1/T3/T6), stack+locals (T3/T6), globals (T3/T6), object tree (T3/T6), dictionary (T3/T6), memory hex (T3/T6), `/debug` command + wrong-engine path (T7), themeable selectors (T5), navigable disassembly + address-history backward scroll (T4). Stepper/Glulx explicitly deferred (spec non-goals) — no task.
- **The opcode table is the one place trusting this plan verbatim is unsafe** — Task 1 Step 3's table MUST be verified against ZMSD §14 and `exec.rs::execute()`; Step 8's fixture test is the oracle.
- **Field-name/harness caveats are flagged inline** (PaneFrame inner-rect field, `session` binding name, AppState/GameSession test constructors, the parse entry point). Resolve each against the cited file:line rather than guessing.
- **Type consistency:** `DebugKey` (T4) is consumed in T7; `DebugPanelState`/`DebugPane`/`DebugView` (T4) in T6; `SlashOutcome::OpenDebug` (T7) matches the dispatch arm (T7); `Debugger` methods (T2) match the `GameSession` impl (T3) and the `MockDbg`/`Dummy` test doubles (T4/T2).
