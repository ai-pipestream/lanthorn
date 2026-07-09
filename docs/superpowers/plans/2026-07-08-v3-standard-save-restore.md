# Standard-Compliant In-Game `@save`/`@restore` (incl. v3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the game-driven `@save`/`@restore` opcode path fully standard-compliant across v3 (branch), v4 (store), and v5+ (`EXT` store) by adopting the Quetzal §5.8 saved-PC convention — the saved PC points at the result descriptor and restore reads it forward — which closes the deferred v3 case as a byproduct.

**Architecture:** The saved PC recorded for an in-game `@save` becomes the address of the instruction's result descriptor (v3: the branch byte(s); v4+: the store byte). On restore, `complete_restore_success` reads that descriptor forward and completes it as "restore succeeded" (v3: branch true; v4+: store 2). The emulator-style host "Save State" path (Ctrl+S / archive / auto-resume) never sets `pending_save`, so it keeps serializing `state.pc` unchanged and stays byte-identical.

**Tech Stack:** Rust workspace. `zvm` crate (zero-dep Z-machine VM: `decode.rs`, `exec.rs`, `quetzal.rs`), `app` crate (ratatui TUI: `session.rs`).

## Global Constraints

- **`zvm` stays zero-dependency.** No new crates in `crates/zvm`. (Cross-platform: VM crates are zero-dep.)
- **Quetzal §5.8 convention, exact:** v3 saved PC points at the 1-or-2-byte branch descriptor; v4+ saved PC points at the single store byte. Restore reads the descriptor *forward*.
- **Z-machine save result values, exact:** save success stores `1`; a restore makes the original `@save` yield `2`; failure stores `0` (v4+) or falls through (v3).
- **Save State (host snapshot) path is byte-identical** — only the in-game `@save`/`@restore` opcode path changes. Any code path that does not set `pending_save` (host Ctrl+S, `save_undo`) must serialize `state.pc` exactly as before.
- **No backward compatibility / migration** — pre-change in-game save files are discarded (the user deletes them). Do not add dual-path or version-sniffing restore logic.
- **Commit trailer:** every commit ends with `Quest: SQ-0163`, then the standard `Co-Authored-By` / `Claude-Session` trailers.
- Do not touch `restore_file` / `restore_quetzal` (the host restore path) or `complete_restore_failure`.

---

### Task 1: Branch length + `decode_branch_at` helper

**Files:**
- Modify: `crates/zvm/src/cpu/decode.rs` (`struct Branch` ~46-51; inline branch decode ~399-420)
- Test: `crates/zvm/src/cpu/decode.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub struct Branch { pub on_true: bool, pub offset: i16, pub len: u8 }` (adds `len`); and `pub fn decode_branch_at(mem: &crate::memory::Memory, addr: u32) -> Branch` returning the branch with `len` = 1 or 2. Consumed by Task 2 (`exec.rs` restore + save-descriptor math).

- [ ] **Step 1: Write the failing test**

Add to the decode `tests` module:

```rust
#[test]
fn decode_branch_at_reports_form_and_length() {
    let mut m = Memory::new(crate::header::tests_support::sample_story(3)).unwrap();
    // Single-byte form: on_true, short-form bit, offset 5.
    m.write_byte(0x40, 0x80 | 0x40 | 5);
    let b1 = decode_branch_at(&m, 0x40);
    assert!(b1.on_true);
    assert_eq!(b1.offset, 5);
    assert_eq!(b1.len, 1, "short-form branch is 1 byte");

    // Two-byte form: on_false (bit7=0), long-form (bit6=0), 14-bit offset = -1.
    m.write_byte(0x50, 0x3F); // high6 = 0x3F, sign bit (0x2000) set
    m.write_byte(0x51, 0xFF); // low8 = 0xFF -> raw 0x3FFF -> -1
    let b2 = decode_branch_at(&m, 0x50);
    assert!(!b2.on_true);
    assert_eq!(b2.offset, -1);
    assert_eq!(b2.len, 2, "long-form branch is 2 bytes");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm decode_branch_at_reports_form_and_length`
Expected: FAIL — `decode_branch_at` not found (and `len` field missing).

- [ ] **Step 3: Add `len` to `Branch` and the `decode_branch_at` helper**

In `crates/zvm/src/cpu/decode.rs`, extend the struct (keep the existing doc comments on the other fields):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    /// Branch when condition is true (bit 7 of first branch byte).
    pub on_true: bool,
    /// Signed offset. 0 = return false, 1 = return true, else PC-relative.
    pub offset: i16,
    /// Number of bytes the branch descriptor occupies (1 for short form, 2 for long).
    pub len: u8,
}
```

Add the free function (near the other decode helpers):

```rust
/// Decode a branch descriptor at `addr`. The Z-machine short form (bit 6 set) is
/// one byte with a 6-bit unsigned offset; the long form is two bytes with a
/// 14-bit signed offset. `Branch::len` reports how many bytes were consumed.
pub fn decode_branch_at(mem: &crate::memory::Memory, addr: u32) -> Branch {
    let b0 = mem.read_byte(addr);
    let on_true = (b0 & 0x80) != 0;
    if (b0 & 0x40) != 0 {
        Branch { on_true, offset: (b0 & 0x3F) as i16, len: 1 }
    } else {
        let b1 = mem.read_byte(addr + 1);
        let raw = (((b0 & 0x3F) as u16) << 8) | (b1 as u16);
        let offset = if raw & 0x2000 != 0 { (raw | 0xC000) as i16 } else { raw as i16 };
        Branch { on_true, offset, len: 2 }
    }
}
```

- [ ] **Step 4: Refactor the inline branch decode to use the helper**

Replace the inline branch block (`decode.rs` ~399-420) with:

```rust
    // Branch bytes (read after store)
    let branch = if branches {
        let br = decode_branch_at(mem, cursor);
        cursor += br.len as u32;
        Some(br)
    } else {
        None
    };
```

Then grep for any other `Branch {` literal that now needs `len`:
Run: `grep -rn "Branch {" crates/zvm/src` — the only construction site should be `decode_branch_at`. Fix any other literal by giving it the correct `len`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p zvm`
Expected: PASS — the new test passes and all existing decode tests stay green (they assert `on_true`/`offset`, unaffected by the added `len`).

- [ ] **Step 6: Commit**

```bash
git add crates/zvm/src/cpu/decode.rs
git commit -m "$(cat <<'EOF'
feat(zvm): Branch.len + decode_branch_at helper (SQ-0163)

Quest: SQ-0163
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 2: Standard PC convention in the VM (save + restore, atomically)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (`PendingSave` ~177-179; save handlers 0OP `0x05` ~789-805 and EXT `0x00` ~1264-1287; add `save_pc`; `complete_restore_success` ~1994-2003; its doc comment ~1983-1993)
- Modify: `crates/zvm/src/quetzal.rs` (`encode_ifhd` PC write ~115; save-PC doc comment ~13-18; add a `saved_pc_of` test helper)
- Test: `crates/zvm/src/cpu/exec.rs` (`tests` module ~2172) — updates the existing v4 test's stale comment and adds v3 + v5 round trips
- Test: `crates/zvm/src/quetzal.rs` (`tests` module)

**Interfaces:**
- Consumes: `crate::cpu::decode::decode_branch_at`, `Branch::len` (Task 1).
- Produces: `Machine::save_pc(&self) -> u32` (`pub(crate)`); `PendingSave { result_dest: SaveDest, descriptor_pc: u32 }`; `#[cfg(test)] pub(crate) fn crate::quetzal::saved_pc_of(&[u8]) -> u32`. The in-game `@save` now records the descriptor PC; `complete_restore_success` completes it forward.

**Why atomic:** changing only the save side would leave the saved PC pointing at the descriptor while `complete_restore_success` still reads `pc - 1`, breaking the existing v4 round-trip test. Both halves ship together so the suite stays green.

- [ ] **Step 1: Write the failing tests**

First add the `saved_pc_of` helper to `crates/zvm/src/quetzal.rs` (module body, not inside `mod tests`), so tests can read a blob's IFhd PC:

```rust
/// Test helper: extract the IFhd program counter from a Quetzal blob.
#[cfg(test)]
pub(crate) fn saved_pc_of(data: &[u8]) -> u32 {
    let chunks = parse_iff(data).expect("valid IFF");
    let ifhd = find_chunk(&chunks, b"IFhd").expect("IFhd present");
    decode_ifhd_pc(ifhd)
}
```

Then, in `crates/zvm/src/cpu/exec.rs` `tests` module, add fixtures + tests:

```rust
// v3: @save is a BRANCH instruction. 0x40 save (0xB5) + 1 branch byte (0x41).
// Branch: on-true, short form, offset 5. next_pc after the branch byte is 0x42,
// so a taken branch lands at 0x42 + 5 - 2 = 0x45.
fn save_v3_branch_story() -> Vec<u8> {
    let mut buf = sample_story(3);
    buf[0x40] = 0xB5;          // 0OP:0x05 save (branch form in v3)
    buf[0x41] = 0x80 | 0x40 | 5; // branch: on-true, short form, offset 5
    buf[0x45] = 0xBA;          // quit at the branch-taken target
    buf
}

#[test]
fn v3_branch_save_restore_round_trip() {
    let mem = Memory::new(save_v3_branch_story()).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x40;

    let r = m.step();
    assert_eq!(r, StepResult::SaveRequest, "v3 save suspends with SaveRequest");
    assert_eq!(m.state.pc, 0x42, "PC post-instruction: opcode 0x40 + 1 branch byte");

    // Standard convention: saved (IFhd) PC points AT the branch descriptor (0x41).
    let blob = m.save_quetzal();
    assert_eq!(crate::quetzal::saved_pc_of(&blob), 0x41, "v3 saved PC = branch byte address");

    // Immediate save success takes the branch -> 0x45.
    m.complete_save(true);
    assert_eq!(m.state.pc, 0x45, "save success branches to 0x45");

    // Move PC away; restore must make the original @save 'succeed' (branch taken).
    m.state.pc = 0x00AB;
    m.complete_restore_success(&blob).expect("v3 restore must succeed");
    assert_eq!(m.state.pc, 0x45, "restore resumes as if the v3 @save branched");
}

// v5: @save is EXT:0x00 (0xBE 0x00), VAR types byte (0xFF = 0 operands), store byte.
fn save_v5_ext_into_g0_story() -> Vec<u8> {
    let mut buf = sample_story(5);
    buf[0x40] = 0xBE; // EXT prefix
    buf[0x41] = 0x00; // EXT:0x00 save
    buf[0x42] = 0xFF; // VAR types: all 4 operands omitted
    buf[0x43] = 0x10; // store byte -> global 0
    buf[0x44] = 0xBA; // quit
    buf
}

#[test]
fn v5_ext_save_restore_round_trip() {
    let mem = Memory::new(save_v5_ext_into_g0_story()).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x40;

    let r = m.step();
    assert_eq!(r, StepResult::SaveRequest, "v5 EXT save suspends with SaveRequest");
    assert_eq!(m.state.pc, 0x44, "PC post-instruction (store byte at 0x43)");

    let blob = m.save_quetzal();
    assert_eq!(crate::quetzal::saved_pc_of(&blob), 0x43, "v5 saved PC = store byte address");

    m.complete_save(true);
    assert_eq!(m.global(0), 1, "save success stores 1");

    m.do_store(Some(0x10), 0x99);
    m.state.pc = 0x00AB;
    m.complete_restore_success(&blob).expect("v5 restore must succeed");
    assert_eq!(m.global(0), 2, "restore makes the original @save 'return' 2");
    assert_eq!(m.state.pc, 0x44, "PC resumes post-@save");
}

#[test]
fn save_state_host_path_keeps_state_pc() {
    // No @save opcode fired (pending_save is None): save_quetzal must serialize
    // state.pc verbatim — the host "Save State" convention is unchanged.
    let mem = Memory::new(sample_story(5)).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x0123;
    let blob = m.save_quetzal();
    assert_eq!(crate::quetzal::saved_pc_of(&blob), 0x0123, "host save keeps state.pc");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p zvm v3_branch_save_restore_round_trip v5_ext_save_restore_round_trip save_state_host_path_keeps_state_pc`
Expected: FAIL — `saved_pc_of` gives the post-instruction PC (0x42 / 0x44), and v3 `complete_restore_success` does nothing (no v3 body), so the PC/branch assertions fail.

- [ ] **Step 3: Capture the descriptor PC at save time**

In `crates/zvm/src/cpu/exec.rs`, extend `PendingSave`:

```rust
/// Context captured when the `save` opcode fires, needed by `complete_save`.
struct PendingSave {
    result_dest: SaveDest,
    /// Address of the instruction's result descriptor (Quetzal §5.8): the store
    /// byte (v4+) or the first branch byte (v3). Written into the save file's PC.
    descriptor_pc: u32,
}
```

Update the 0OP `0x05` save handler (`exec.rs` ~789-805) so it records both the destination and the descriptor address:

```rust
            0x05 => {
                let (dest, descriptor_pc) = if self.mem.version() <= 3 {
                    match branch {
                        Some(b) => {
                            let dpc = self.state.pc - b.len as u32;
                            (SaveDest::Branch(b), dpc)
                        }
                        None => (SaveDest::Store(0), self.state.pc.saturating_sub(1)),
                    }
                } else {
                    match store {
                        Some(sv) => (SaveDest::Store(sv), self.state.pc.saturating_sub(1)),
                        None => (SaveDest::Store(0), self.state.pc.saturating_sub(1)),
                    }
                };
                self.pending_save = Some(PendingSave { result_dest: dest, descriptor_pc });
                StepResult::SaveRequest
            }
```

Update the EXT `0x00` save handler's suspend branch (`exec.rs` ~1279-1286):

```rust
                } else {
                    let dest = match store {
                        Some(sv) => SaveDest::Store(sv),
                        None => SaveDest::Store(0),
                    };
                    self.pending_save = Some(PendingSave {
                        result_dest: dest,
                        descriptor_pc: self.state.pc.saturating_sub(1),
                    });
                    StepResult::SaveRequest
                }
```

Add the `save_pc` accessor (near `save_quetzal`, ~1918):

```rust
    /// The program counter to record in a save file. For an in-game `@save`
    /// (pending_save set) this is the result descriptor's address, per Quetzal
    /// §5.8; otherwise (host Save State, undo snapshots) it is the current pc.
    pub(crate) fn save_pc(&self) -> u32 {
        self.pending_save.as_ref().map(|p| p.descriptor_pc).unwrap_or(self.state.pc)
    }
```

- [ ] **Step 4: Serialize the descriptor PC**

In `crates/zvm/src/quetzal.rs`, `encode_ifhd` (~115), change the PC source:

```rust
    // PC at save time (3 bytes, big-endian). For an in-game @save this is the
    // result-descriptor address (Quetzal §5.8); for a host Save State it is
    // state.pc. See Machine::save_pc.
    let pc = machine.save_pc();
```

Rewrite the module-header "Save PC semantics" comment (`quetzal.rs` ~13-18) to describe the two conventions:

```rust
// Save PC semantics: the IFhd PC comes from Machine::save_pc(). For an in-game
// @save/@restore opcode (pending_save set) it is the result-descriptor address —
// the store byte (v4+) or first branch byte (v3) — per Quetzal §5.8; restore
// reads that descriptor forward (see complete_restore_success). For a host
// "Save State" snapshot (no pending save) it is state.pc, and restore simply
// resumes there (see restore_file).
```

- [ ] **Step 5: Read the descriptor forward on restore**

Replace `complete_restore_success` (`exec.rs` ~1994-2003) and its doc comment:

```rust
    /// Complete a game-initiated restore with the supplied Quetzal bytes.
    ///
    /// On success the machine state (dynamic memory, frames, eval stack, PC) is
    /// replaced with the saved state, whose PC points at the original `@save`'s
    /// result descriptor (Quetzal §5.8). We complete that descriptor forward as
    /// "restore succeeded": v3 takes the `@save` branch as true; v4+ stores 2
    /// into the `@save`'s store variable. A restore invalidates undo history, and
    /// the `@restore`'s own store target is unused on success — both are cleared.
    ///
    /// On `Err` the machine is untouched (the `restore_quetzal` contract); the
    /// caller should then call `complete_restore_failure()`.
    pub fn complete_restore_success(&mut self, data: &[u8]) -> Result<(), crate::error::ZError> {
        self.restore_quetzal(data)?;
        if self.mem.version() <= 3 {
            // v3 @save is a branch instruction; resume as if it branched on success.
            let br = crate::cpu::decode::decode_branch_at(&self.mem, self.state.pc);
            self.state.pc += br.len as u32; // advance to next_pc (do_branch uses pc + off - 2)
            self.do_branch(Some(br), true);
        } else {
            // v4+ @save stores its result; the game is being restored, so store 2.
            let store_var = self.mem.read_byte(self.state.pc);
            self.do_store(Some(store_var), 2);
            self.state.pc += 1; // advance past the store byte
        }
        self.undo_stack.clear();
        self.pending_restore_store = None;
        Ok(())
    }
```

- [ ] **Step 6: Fix the existing v4 test's stale comment**

The existing `complete_restore_success_stores_2_and_resumes_pc` test (`exec.rs` ~2192) still passes (final `global(0) == 2`, `pc == 0x42`), but its comment describes the old post-instruction PC. Update the comment block above `save_v4_into_g0_story` (~2177-2182) to reflect the new convention:

```rust
    // ── In-game restore-success: the original @save "returns 2" on restore ─────
    //
    // v4 story at 0x40:  save -> G0 (0xB5, store byte 0x10 at 0x41), then quit.
    // After step() the @save suspends with SaveRequest; the saved (IFhd) PC points
    // at the store byte 0x41 (Quetzal §5.8). complete_restore_success restores the
    // state, reads the store byte forward, stores 2 into G0, and resumes at 0x42.
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p zvm`
Expected: PASS — new v3/v5/host tests pass; the existing v4 round-trip test and all `quetzal.rs` round-trip tests stay green (host path unchanged).

- [ ] **Step 8: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs crates/zvm/src/quetzal.rs
git commit -m "$(cat <<'EOF'
feat(zvm): standard Quetzal PC convention for in-game @save/@restore (SQ-0163)

Saved PC points at the result descriptor (v3 branch bytes / v4+ store byte);
restore reads it forward (v3 branch true / v4+ store 2). Host Save State path
(no pending_save) still serializes state.pc unchanged.

Quest: SQ-0163
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 3: Lift the v3 auto-fail in the app session

**Files:**
- Modify: `crates/app/src/session.rs` (`run_until_input` ~498-529 + its doc ~493-497; `submit` ~349-350; `finish_turn` ~355-365; `advance_after_input` ~371-376; `drain_turn` ~391-432; `collect_turn` ~382-384)
- Test: `crates/app/src/session.rs` (`tests` module — replace `v3_ingame_save_still_auto_fails_with_info` ~1657-1670)

**Interfaces:**
- Consumes: the VM's now-working v3 save/restore (Task 2).
- Produces: `run_until_input(machine: &mut Machine) -> RunStop` (drops the `bool`); v3 `@save`/`@restore` now bubble `PendingIo::Save`/`Restore` to the host.

**Note on the `info` field:** `TurnResult.info` is a general "note to display" consumed in `main.rs` (3161/4524/5139). The v3 hint is its *only* current producer. Removing the hint leaves the field always-`None` — keep the field and its `main.rs` consumers; only remove the v3-specific computation and `v3_failed` plumbing.

- [ ] **Step 1: Replace the v3 auto-fail test with a bubbling test**

In `crates/app/src/session.rs` `tests`, replace `v3_ingame_save_still_auto_fails_with_info` (~1657-1670) with:

```rust
    #[test]
    fn v3_ingame_save_and_restore_bubble_pending_io() {
        // v3 @save/@restore are BRANCH instructions (0OP:0x05/0x06 = 0xB5/0xB6 +
        // 1 branch byte). After the standard-PC fix they bubble pending_io like v4+.
        let mut save_buf = read_char_story_v5();
        save_buf[0x00] = 3;              // version 3 (branch form)
        save_buf[0x44] = 0xB5;           // 0OP:0x05 save (branch form)
        save_buf[0x45] = 0x80 | 0x40 | 2; // branch on-true, short form, offset 2 -> quit at 0x46
        save_buf[0x46] = 0xBA;           // quit
        let mut sess = GameSession::new(save_buf, true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Save), "v3 in-game save now bubbles pending_io");
        assert!(r.info.is_none(), "no 'isn't wired' info line for v3 anymore");
        let r2 = sess.resume_save(true);
        assert!(r2.quit, "resume_save completes the branch and runs to quit");

        let mut restore_buf = read_char_story_v5();
        restore_buf[0x00] = 3;
        restore_buf[0x44] = 0xB6;           // 0OP:0x06 restore (branch form)
        restore_buf[0x45] = 0x80 | 0x40 | 2; // branch byte (unused on cancel)
        restore_buf[0x46] = 0xBA;           // quit
        let mut sess = GameSession::new(restore_buf, true, false, None).expect("new");
        let r = sess.submit_char(b'x');
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "v3 in-game restore now bubbles pending_io");
        let r2 = sess.resume_restore(None);
        assert!(r2.quit, "cancelled v3 restore falls through to quit");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app v3_ingame_save_and_restore_bubble_pending_io`
Expected: FAIL — v3 still auto-fails, so `pending_io` is `None`.

- [ ] **Step 3: Remove the v3 short-circuit from `run_until_input`**

Replace `run_until_input` and its doc comment (`session.rs` ~493-529) with:

```rust
/// Step until the machine pauses for input, quits, or suspends on its own
/// `@save`/`@restore`. In-game save/restore bubbles up as `SavePending`/
/// `RestorePending` for the host to service (all versions, v3 included).
fn run_until_input(machine: &mut Machine) -> RunStop {
    loop {
        match machine.step() {
            StepResult::Quit => return RunStop::Quit,
            StepResult::Fault => return RunStop::Quit,
            StepResult::NeedLine { .. } => return RunStop::Input(InputKind::Line),
            StepResult::NeedChar => return RunStop::Input(InputKind::Char),
            StepResult::SaveRequest => return RunStop::SavePending,
            StepResult::RestoreRequest => return RunStop::RestorePending,
            StepResult::Restart => return RunStop::Quit, // not supported headless; treat as quit
            StepResult::Continue => {}
        }
    }
}
```

- [ ] **Step 4: Drop the `v3_failed` plumbing from the callers**

`submit` (~349-350):

```rust
        let stop = run_until_input(&mut self.machine);
        self.finish_turn(stop)
```

`advance_after_input` (~371-376):

```rust
    fn advance_after_input(&mut self, timed_out: bool) -> TurnResult {
        let stop = run_until_input(&mut self.machine);
        let mut result = self.finish_turn(stop);
        result.timed_out = timed_out;
        result
    }
```

`finish_turn` (~355-365) — drop the `v3_failed` param:

```rust
    /// Build the `TurnResult` from a `RunStop` and drain the VM's per-turn
    /// buffers. Shared by submit/submit_char/resume_*.
    fn finish_turn(&mut self, stop: RunStop) -> TurnResult {
        let (quit, pending, pending_io) = match stop {
            RunStop::Quit => (true, InputKind::Line, None),
            RunStop::Input(k) => (false, k, None),
            RunStop::SavePending => (false, self.pending, Some(PendingIo::Save)),
            RunStop::RestorePending => (false, self.pending, Some(PendingIo::Restore)),
        };
        self.quit = quit;
        self.pending = pending;
        self.drain_turn(quit, pending_io, false)
    }
```

`collect_turn` (~382-384):

```rust
    fn collect_turn(&mut self) -> TurnResult {
        self.drain_turn(self.quit, None, false)
    }
```

- [ ] **Step 5: Drop `v3_failed` from `drain_turn` and set `info: None`**

`drain_turn` (~391-432) — remove the `v3_failed` param and the `info` computation; set `info: None` directly:

```rust
    fn drain_turn(
        &mut self,
        quit: bool,
        pending_io: Option<PendingIo>,
        timed_out: bool,
    ) -> TurnResult {
        let (raw, raw_runs) = sink_mut(&mut self.machine).take_styled();
        let transcript = strip_read_prompt(&raw).to_owned();
        let transcript_runs = clamp_runs(raw_runs, transcript.chars().count());
        let detected = detect_location(&self.machine);
        let location = detected.as_ref().map(location_to_snapshot);
        let location_method = detected.as_ref().map(Location::method);

        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let fault = self.machine.take_fault_trace().map(|t| t.to_lines());
        let sounds = std::mem::take(&mut self.machine.pending_sounds);
        let erase_lower = std::mem::take(&mut self.machine.screen.erase_lower_requested);

        TurnResult {
            transcript,
            transcript_runs,
            location,
            quit,
            erase_lower,
            info: None,
            sounds,
            glulx_sound_ops: Vec::new(),
            diagnostics,
            fault,
            location_method,
            pending_io,
            timed_out,
            transcript_elems: Vec::new(),
        }
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p app`
Expected: PASS — the new v3 bubbling test passes; `turn_result_info_defaults_none_for_normal_turn` and the v4 `ingame_save_yields_pending_io_and_resume_continues` (`r.info.is_none()`) stay green. No remaining references to `v3_failed`.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/session.rs
git commit -m "$(cat <<'EOF'
feat(app): bubble v3 in-game @save/@restore to the host (SQ-0163)

Removes the v3 auto-fail short-circuit and its info-hint plumbing now that
the VM handles v3 branch-form save/restore. v3 yields PendingIo::Save/Restore
like v4+.

Quest: SQ-0163
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

### Task 4: Real-game v3 smoke test (`minizork.z3`)

**Files:**
- Test: `crates/app/src/session.rs` (`tests` module — add one fixture-gated test, following the `bureaucracy_ingame_restore_redraws_status_grid` pattern ~1703)

**Interfaces:**
- Consumes: the full v3 path (Tasks 2-3) end to end via the session resume API.

- [ ] **Step 1: Confirm the fixture path**

Run: `ls crates/zvm/tests/fixtures/minizork.z3 stories/minizork.z3 2>/dev/null`
Use whichever path exists. If **neither** exists, run `ls stories/*.z3 | head` and pick any real v3 story, or place `minizork.z3` under `crates/zvm/tests/fixtures/`. The test must actually run — a permanently-skipping test is not acceptable coverage. Record the chosen path for Step 2.

- [ ] **Step 2: Write the smoke test**

Add to `crates/app/src/session.rs` `tests` (use the confirmed path from Step 1 in the `.join(...)`):

```rust
    // Real v3 game: an in-game @save then @restore must round-trip through the
    // standard branch-form path. Oracle: replaying the same command after a
    // restore reproduces the pre-restore transcript exactly.
    #[test]
    fn minizork_v3_ingame_save_restore_round_trips() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zvm/tests/fixtures/minizork.z3"); // adjust to the Step 1 path
        if !fixture.exists() {
            panic!("minizork.z3 fixture missing at {} — this smoke test must run", fixture.display());
        }
        let story = std::fs::read(&fixture).expect("read minizork.z3");
        let mut sess = GameSession::new(story, true, false, None).expect("new minizork.z3");

        // Reach a stable prompt, then @save via the game's save verb.
        let mut blob: Option<Vec<u8>> = None;
        for cmd in ["open mailbox", "save"] {
            let r = sess.submit(cmd);
            if r.pending_io == Some(PendingIo::Save) {
                blob = Some(sess.machine.save_quetzal());
                let _ = sess.resume_save(true); // host "wrote" the file; @save returns success
                break;
            }
            assert!(!r.quit, "unexpected quit before reaching @save");
        }
        let blob = blob.expect("minizork reached @save via 'save'");

        // Probe command on the post-save branch.
        let t1 = sess.submit("north").transcript;

        // Restore via the game's @restore, supplying the captured blob.
        let r = sess.submit("restore");
        assert_eq!(r.pending_io, Some(PendingIo::Restore), "'restore' reaches @restore");
        sess.resume_restore(Some(&blob));

        // Same probe after restore must reproduce the same transcript.
        let t2 = sess.submit("north").transcript;
        assert_eq!(t2, t1, "post-restore continuation matches the pre-restore continuation");
    }
```

- [ ] **Step 3: Run the test to verify behavior**

Run: `cargo test -p app minizork_v3_ingame_save_restore_round_trips -- --nocapture`
Expected: PASS. If the command sequence does not reach `@save` (game-specific prompts), adjust the `cmd` list until `submit("save")` yields `PendingIo::Save`, and confirm the probe command produces non-empty, deterministic transcript on both branches. Do not weaken the assertion to make it pass.

- [ ] **Step 4: Commit**

```bash
git add crates/app/src/session.rs
git commit -m "$(cat <<'EOF'
test(app): v3 minizork in-game @save/@restore round-trip smoke test (SQ-0163)

Quest: SQ-0163
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

---

## Final verification

- [ ] `cargo test -p zvm` — all green (decode, quetzal, exec save/restore incl. new v3/v5 round trips).
- [ ] `cargo test -p app` — all green (session v3 bubbling + minizork smoke).
- [ ] `cargo build --workspace` — no warnings about unused `v3_failed`/`info` plumbing.
- [ ] Grep sanity: `grep -rn "v3_failed\|isn't wired" crates/app/src` returns nothing.
