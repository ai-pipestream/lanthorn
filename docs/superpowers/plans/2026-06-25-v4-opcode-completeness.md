# v4+ Opcode-Completeness Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the missing core v4+ VAR opcodes (`scan_table`, `copy_table`, `print_table`, `get_cursor`) and a recognized `erase_line` skeleton, and replace the silent VAR fallthrough with a once-per-opcode warning — so v4+ games (e.g. Bureaucracy) stop misbehaving on unimplemented opcodes.

**Architecture:** All work is in `crates/zvm/src/cpu/exec.rs` inside `exec_var` (signature `fn exec_var(&mut self, opcode: u8, ops: &[u16], store: Option<u8>, branch: Option<Branch>) -> StepResult`, line ~626). Each opcode becomes an explicit match arm before the catch-all at line ~853. The catch-all changes from a silent `_ => StepResult::Continue` to one that records + logs the opcode once.

**Tech Stack:** Rust. Tests are in-file `#[cfg(test)] mod tests` (so they can call the private `exec_var` directly), run with `cargo test -p zvm`.

## Global Constraints

- Implement each opcode per the Z-Machine Standards Document (ZMSD §15).
- Use existing helpers: `self.do_store(store: Option<u8>, val: u16)`, `self.do_branch(branch: Option<Branch>, cond: bool)`, `self.print_text(&str)`, `zscii_to_char(zscii: u16) -> char`, `self.mem.read_byte(addr: u32) -> u8`, `self.mem.read_word(addr: u32) -> u16`, `self.mem.write_byte(addr: u32, v: u8)`, `self.mem.write_word(addr: u32, v: u16)`.
- Store variable numbering (for `do_store`): 0 = stack, 1–15 locals, 16+ = global N (so global 0 is var 16); read globals in tests via `m.global(n)`.
- Tests build a machine with `build_test_machine(&[])` (an empty program is fine; we call `exec_var` directly) and write any needed data into memory first. The dynamic-memory region used by `sample_story(5)` around 0x0200+ is safe scratch space for test tables.
- `cargo test -p zvm` green and `cargo build -p zvm` 0 warnings after every task.
- Commit trailers (NO backticks in the commit body — zsh):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

---

### Task 1: scan_table (VAR:0x17) — store + branch

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add a `0x17 =>` arm in `exec_var`.
- Test: same file, `mod tests`.

**Interfaces:**
- Consumes: `do_store`, `do_branch`, `mem.read_word`, `mem.read_byte`.
- ZMSD: `scan_table x table len form` — search `len` entries starting at `table` for value `x`. `form` (default 0x82): bit 7 set → word (2-byte) entries; low 7 bits → bytes to step per entry. Store the address of the first match (0 if none); branch if a match was found.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn scan_table_word_finds_and_stores_address() {
    let mut m = build_test_machine(&[]);
    // Word table at 0x0200: [0x1111, 0x2222, 0x3333]
    m.mem.write_word(0x0200, 0x1111);
    m.mem.write_word(0x0202, 0x2222);
    m.mem.write_word(0x0204, 0x3333);
    // scan_table 0x2222, table=0x0200, len=3, form=0x82 (word, step 2) -> G0
    m.exec_var(0x17, &[0x2222, 0x0200, 3, 0x82], Some(16), None);
    assert_eq!(m.global(0), 0x0202, "address of the matching word entry");
}

#[test]
fn scan_table_not_found_stores_zero() {
    let mut m = build_test_machine(&[]);
    m.mem.write_word(0x0200, 0x1111);
    m.exec_var(0x17, &[0x9999, 0x0200, 1, 0x82], Some(16), None);
    assert_eq!(m.global(0), 0, "no match -> store 0");
}

#[test]
fn scan_table_byte_form_compares_low_byte() {
    let mut m = build_test_machine(&[]);
    m.mem.write_byte(0x0200, 0x05);
    m.mem.write_byte(0x0201, 0x07);
    // form=0x01 -> byte entries, step 1
    m.exec_var(0x17, &[0x0007, 0x0200, 2, 0x01], Some(16), None);
    assert_eq!(m.global(0), 0x0201, "byte form matches low byte at the second entry");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm scan_table 2>&1 | tail -15`
Expected: FAIL — current `_ => Continue` stores nothing, so `m.global(0)` is 0 in the "finds" test.

- [ ] **Step 3: Implement the arm**

Add inside `exec_var`, before the `_ => StepResult::Continue` catch-all:

```rust
// VAR:0x17 scan_table — search a table for x; store match address (0 if none), branch if found.
0x17 => {
    let x = ops.first().copied().unwrap_or(0);
    let table = ops.get(1).copied().unwrap_or(0) as u32;
    let len = ops.get(2).copied().unwrap_or(0);
    let form = ops.get(3).copied().unwrap_or(0x82);
    let is_word = form & 0x80 != 0;
    let step = ((form & 0x7F) as u32).max(1);
    let mut found: u16 = 0;
    for i in 0..len as u32 {
        let addr = table + i * step;
        let val = if is_word { self.mem.read_word(addr) } else { self.mem.read_byte(addr) as u16 };
        let target = if is_word { x } else { x & 0xFF };
        if val == target {
            found = addr as u16;
            break;
        }
    }
    self.do_store(store, found);
    self.do_branch(branch, found != 0);
    StepResult::Continue
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm scan_table 2>&1 | tail -8`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): implement scan_table (VAR:0x17)"
```

---

### Task 2: copy_table (VAR:0x1D)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add `0x1D =>` in `exec_var`.
- Test: same file.

**Interfaces:**
- Consumes: `mem.read_byte`, `mem.write_byte`.
- ZMSD: `copy_table first second size`. If `second == 0`: zero `|size|` bytes at `first`. Else if `size < 0`: copy `|size|` bytes forward (overlap allowed/deliberate). Else (`size > 0`): copy `size` bytes without corruption even when the regions overlap (snapshot-then-write).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn copy_table_copies_forward() {
    let mut m = build_test_machine(&[]);
    for i in 0..4u32 { m.mem.write_byte(0x0200 + i, (i + 1) as u8); } // 1,2,3,4
    m.exec_var(0x1D, &[0x0200, 0x0300, 4], None, None);
    for i in 0..4u32 { assert_eq!(m.mem.read_byte(0x0300 + i), (i + 1) as u8); }
}

#[test]
fn copy_table_zeroes_when_second_is_zero() {
    let mut m = build_test_machine(&[]);
    for i in 0..3u32 { m.mem.write_byte(0x0200 + i, 0xFF); }
    m.exec_var(0x1D, &[0x0200, 0, 3], None, None);
    for i in 0..3u32 { assert_eq!(m.mem.read_byte(0x0200 + i), 0); }
}

#[test]
fn copy_table_positive_size_overlap_is_noncorrupting() {
    let mut m = build_test_machine(&[]);
    for i in 0..4u32 { m.mem.write_byte(0x0200 + i, (i + 1) as u8); } // 1,2,3,4
    // Overlapping forward copy by 1 (dest > src). Positive size must NOT corrupt:
    // result at 0x0201..=0x0204 should be the ORIGINAL 1,2,3,4.
    m.exec_var(0x1D, &[0x0200, 0x0201, 4], None, None);
    assert_eq!(m.mem.read_byte(0x0201), 1);
    assert_eq!(m.mem.read_byte(0x0202), 2);
    assert_eq!(m.mem.read_byte(0x0203), 3);
    assert_eq!(m.mem.read_byte(0x0204), 4);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm copy_table 2>&1 | tail -15`
Expected: FAIL — nothing is copied/zeroed today.

- [ ] **Step 3: Implement the arm**

```rust
// VAR:0x1D copy_table — copy/zero a memory region (ZMSD §15).
0x1D => {
    let first = ops.first().copied().unwrap_or(0) as u32;
    let second = ops.get(1).copied().unwrap_or(0) as u32;
    let size = ops.get(2).copied().unwrap_or(0) as i16;
    if second == 0 {
        for i in 0..size.unsigned_abs() as u32 {
            self.mem.write_byte(first + i, 0);
        }
    } else if size < 0 {
        // forced forward copy; overlap corruption is intentional
        let n = size.unsigned_abs() as u32;
        for i in 0..n {
            let b = self.mem.read_byte(first + i);
            self.mem.write_byte(second + i, b);
        }
    } else {
        // positive: copy avoiding corruption — snapshot the source first
        let n = size as u32;
        let src: Vec<u8> = (0..n).map(|i| self.mem.read_byte(first + i)).collect();
        for (i, &b) in src.iter().enumerate() {
            self.mem.write_byte(second + i as u32, b);
        }
    }
    StepResult::Continue
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm copy_table 2>&1 | tail -8`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): implement copy_table (VAR:0x1D)"
```

---

### Task 3: get_cursor (VAR:0x10)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add `0x10 =>` in `exec_var`.
- Test: same file.

**Interfaces:**
- Consumes: `mem.write_word`, `self.screen.cursor_row`, `self.screen.cursor_col`.
- ZMSD: `get_cursor array` — write the current cursor row into word 0 of `array` and the column into word 1.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn get_cursor_writes_row_and_col() {
    let mut m = build_test_machine(&[]);
    m.screen.cursor_row = 3;
    m.screen.cursor_col = 7;
    m.exec_var(0x10, &[0x0200], None, None); // array at 0x0200
    assert_eq!(m.mem.read_word(0x0200), 3, "word 0 = row");
    assert_eq!(m.mem.read_word(0x0202), 7, "word 1 = col");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm get_cursor_writes 2>&1 | tail -12`
Expected: FAIL — array words stay 0.

- [ ] **Step 3: Implement the arm**

```rust
// VAR:0x10 get_cursor — write (row, col) of the upper-window cursor into a 2-word array.
0x10 => {
    let array = ops.first().copied().unwrap_or(0) as u32;
    self.mem.write_word(array, self.screen.cursor_row);
    self.mem.write_word(array + 2, self.screen.cursor_col);
    StepResult::Continue
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm get_cursor_writes 2>&1 | tail -6`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): implement get_cursor (VAR:0x10)"
```

---

### Task 4: print_table (VAR:0x1E)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add `0x1E =>` in `exec_var`.
- Test: same file.

**Interfaces:**
- Consumes: `self.print_text`, `zscii_to_char`, `self.mem.read_byte`, `self.screen.cursor_row`, `self.screen.cursor_col`.
- ZMSD: `print_table zscii-text width [height=1] [skip=0]` — print a rectangle starting at the current cursor: `width` ZSCII chars per row, `height` rows, advancing the source by `width + skip` bytes per row; each row begins at the starting column, one row down.

> **Forward-compatibility note:** correct column positioning needs the upper-window grid, which lands in the cursor-screen-model pass. Here we advance `self.screen.cursor_row` per row (so behavior is correct once `print_text` routes through the grid) and print the row characters via `print_text`. Pre-grid, the characters stream to the transcript; exact rectangular alignment completes with the grid. The test asserts the emitted characters, not pixel position.

- [ ] **Step 1: Write the failing test**

(Match the BufferOutput accessor used by existing print tests — find an existing `print_char`/`print_num` test in this file and reuse its pattern for reading captured output. The assertion below uses that captured text.)

```rust
#[test]
fn print_table_emits_each_row_chars() {
    let mut m = build_test_machine(&[]);
    // 2x2 region of ASCII at 0x0200: "AB" / "CD"
    m.mem.write_byte(0x0200, b'A');
    m.mem.write_byte(0x0201, b'B');
    m.mem.write_byte(0x0202, b'C');
    m.mem.write_byte(0x0203, b'D');
    m.exec_var(0x1E, &[0x0200, 2, 2, 0], None, None); // width 2, height 2, skip 0
    let out = captured_output(&m); // helper mirroring existing print tests
    assert!(out.contains('A') && out.contains('B') && out.contains('C') && out.contains('D'),
        "all rectangle characters are printed");
}
```

If existing print tests read output differently (e.g. `m.buffer_output().unwrap().<accessor>()`), define `captured_output` in the test module to match that accessor instead of inventing a new one.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm print_table_emits 2>&1 | tail -12`
Expected: FAIL — nothing printed.

- [ ] **Step 3: Implement the arm**

```rust
// VAR:0x1E print_table — print a rectangle of ZSCII text from the current cursor (ZMSD §15).
0x1E => {
    let mut addr = ops.first().copied().unwrap_or(0) as u32;
    let width = ops.get(1).copied().unwrap_or(0);
    let height = ops.get(2).copied().unwrap_or(1).max(1);
    let skip = ops.get(3).copied().unwrap_or(0) as u32;
    let start_col = self.screen.cursor_col;
    let start_row = self.screen.cursor_row;
    for row in 0..height {
        // Position each row at the starting column, one line down (correct once the grid exists).
        self.screen.cursor_row = start_row + row;
        self.screen.cursor_col = start_col;
        for _ in 0..width {
            let ch = zscii_to_char(self.mem.read_byte(addr) as u16);
            let mut buf = [0u8; 4];
            self.print_text(ch.encode_utf8(&mut buf));
            addr += 1;
        }
        addr += skip;
    }
    StepResult::Continue
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm print_table_emits 2>&1 | tail -6`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): implement print_table (VAR:0x1E)"
```

---

### Task 5: erase_line (VAR:0x0E) — recognized skeleton

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add `0x0E =>` in `exec_var`.
- Test: same file.

**Interfaces:**
- ZMSD: `erase_line value` — when `value == 1`, erase from the cursor to the end of the current line. This is an upper-window grid operation; the grid does not exist until the cursor-screen-model pass. This task adds a **recognized no-op arm** so the opcode is consumed (and does NOT trigger the Task 6 warning); the real erase completes in the screen-model pass.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn erase_line_is_recognized_noop_without_warning() {
    let mut m = build_test_machine(&[]);
    let r = m.exec_var(0x0E, &[1], None, None);
    assert!(matches!(r, StepResult::Continue));
    // It must be an explicit arm, not the unknown-opcode fallthrough (Task 6),
    // so it is NOT recorded as a warned opcode.
    assert!(!m.warned_var_opcodes.contains(&0x0E),
        "erase_line is a recognized arm, not an unimplemented fallthrough");
}
```

> This test depends on the `warned_var_opcodes` set added in Task 6. If executing Task 5 before Task 6, write the arm now and add this assertion's second half when Task 6 lands; the `matches!(r, Continue)` half can run immediately.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm erase_line_is_recognized 2>&1 | tail -12`
Expected: FAIL to compile/assert until the arm (and Task 6 field) exist.

- [ ] **Step 3: Implement the arm**

```rust
// VAR:0x0E erase_line — erase from cursor to end of line in the upper window.
// Recognized here; the actual grid erase is implemented with the upper-window
// grid in the cursor-screen-model pass. No-op until then.
0x0E => StepResult::Continue,
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm erase_line_is_recognized 2>&1 | tail -6`
Expected: PASS (after Task 6 provides `warned_var_opcodes`).

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): recognize erase_line (VAR:0x0E) as a no-op skeleton"
```

---

### Task 6: Warn on unimplemented VAR opcodes

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add a `warned_var_opcodes` field to `Machine` (+ initialize it in the constructor) and change the `exec_var` catch-all.
- Test: same file.

**Interfaces:**
- Produces: `pub(crate) warned_var_opcodes: std::collections::HashSet<u8>` on `Machine`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn unimplemented_var_opcode_is_warned_once() {
    let mut m = build_test_machine(&[]);
    // 0x15 sound_effect is intentionally unimplemented -> hits the fallthrough.
    assert!(m.warned_var_opcodes.is_empty());
    m.exec_var(0x15, &[], None, None);
    assert!(m.warned_var_opcodes.contains(&0x15), "fallthrough records the opcode");
    m.exec_var(0x15, &[], None, None); // second call must not duplicate
    assert_eq!(m.warned_var_opcodes.len(), 1, "warned at most once per opcode");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p zvm unimplemented_var_opcode 2>&1 | tail -12`
Expected: FAIL — no `warned_var_opcodes` field.

- [ ] **Step 3: Add the field + initialize it**

In the `Machine` struct definition add:

```rust
/// VAR opcodes that have hit the unimplemented fallthrough (warned once each).
pub(crate) warned_var_opcodes: std::collections::HashSet<u8>,
```

In every `Machine` constructor (`new`, and any `with_*` that builds a `Machine` literal) initialize `warned_var_opcodes: std::collections::HashSet::new(),`. (Find them: `grep -n "Machine {" crates/zvm/src/cpu/exec.rs`.)

- [ ] **Step 4: Change the catch-all**

Replace the `exec_var` catch-all:

```rust
// Unknown / unimplemented VAR opcode: warn once, then ignore.
_ => {
    if self.warned_var_opcodes.insert(opcode) {
        eprintln!("zvm: warning: unimplemented VAR opcode 0x{opcode:02X} (ignored)");
    }
    StepResult::Continue
}
```

- [ ] **Step 5: Run to verify pass + full suite**

Run: `cargo test -p zvm unimplemented_var_opcode 2>&1 | tail -6` then `cargo test -p zvm 2>&1 | grep "test result"` and `cargo build -p zvm 2>&1 | grep -c warning`
Expected: PASS; all green; `0`.

- [ ] **Step 6: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): warn once on unimplemented VAR opcodes instead of silent no-op"
```

---

### Task 7: Verify against Bureaucracy + zvm-cli

**Files:** none (verification only).

- [ ] **Step 1: Headless smoke test**

Run the built `zvm-cli` on a v4+ game and confirm no "unimplemented VAR opcode" warnings appear for the newly-implemented opcodes (0x17/0x1D/0x1E/0x10/0x0E):

```bash
cargo build -q -p zvm-cli
printf "x\nlook\n" | ./target/debug/zvm-cli stories/bureaucr.z4 2>&1 | head -30
```

Expected: the licence-application intro prints; any remaining warnings are for still-unimplemented opcodes (sound_effect, tokenise, etc.), NOT the five implemented here. Note: full form RENDERING still awaits the cursor-screen-model pass — this task only confirms the opcodes execute without falling through.

- [ ] **Step 2: Record findings** in the ledger (which opcodes Bureaucracy actually exercised; any surprising warnings). No commit.
