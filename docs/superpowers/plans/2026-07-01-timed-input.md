# Z-Machine Timed Input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Honor the Z-machine `read`/`read_char` `time`+`routine` operands (timed / interrupt input) in the `zvm` engine and both hosts (`zvm-cli`, `app`), so real-time games such as Border Zone play correctly.

**Architecture:** The engine owns the re-entrant interrupt (it runs the game routine to completion via the existing call stack and reports whether to abort), staying zero-dependency — it never reads a clock. Each host provides the wall clock: it polls input with a timeout and, on each timeout, calls the engine's interrupt method, then either aborts the read or re-renders and keeps waiting.

**Tech Stack:** Rust. `zvm` (zero-dep engine), `zvm-cli` (crossterm), `app` (ratatui + crossterm TUI).

## Global Constraints

- `zvm` stays **zero-dependency**: no wall-clock reads, no new crates. The clock lives in the hosts. (Verify: `crates/zvm/Cargo.toml` has no `[dependencies]` additions.)
- Timed input default **ON** in both hosts. App: `honor_timed_input` config (default `true`) + slash command + settings row. CLI: `--no-timed-input` flag (timed on unless passed).
- `time` is in **tenths of a second**; wall-clock millis = `time * 100`.
- Untimed reads (`time == 0` or `routine == 0`) must behave **exactly as today** (regression-guarded).
- Cross-platform (Windows/Linux/macOS): use `crossterm` only; no platform-specific I/O.
- All work on a feature branch, not `main`. Completed TODO items move to `COMPLETED.md` via `scripts/todo-done` (see `.githooks/README.md`).

**Design mechanism refinement (vs. spec §3.2):** the spec proposed extending `StepResult::NeedLine`/`NeedChar` with `time`/`routine` fields. This plan instead exposes them via an accessor `Machine::pending_timeout()`, which is behavior-identical but avoids changing every `NeedLine`/`NeedChar` match arm across the engine, tests, and both hosts. All other spec sections are implemented as written.

---

### Task 1: Engine — parse `time`/`routine` and expose `pending_timeout()`

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (`PendingInput` struct ~line 58; `read` arm ~843; `read_char` arm ~851; add accessor method)
- Test: `crates/zvm/src/cpu/exec.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `Machine::pending_timeout(&self) -> Option<(u16, u16)>` — `Some((time_tenths, packed_routine))` while a *timed* read/read_char is pending (both operands nonzero); `None` otherwise. Consumed by Tasks 4, 6.
- Produces: `PendingInput` gains `interrupt_time: u16`, `interrupt_routine: u16` (Copy-able), consumed by Tasks 2, 3.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/zvm/src/cpu/exec.rs`. Reuse the existing `emit_read` test helper (defined ~line 3369) and `sample_story`.

```rust
#[test]
fn pending_timeout_exposes_time_and_routine() {
    // v5 read with time=10 (1.0s) and routine at packed addr 0x0040.
    // read operands: text_buf, parse_buf, time, routine.
    let mut buf = sample_story(5);
    // VAR read (0x04) with four operands: 2 large (text,parse) + 2 large (time,routine).
    // type byte 0x00 = all-large; but time/routine here use small consts, so use a
    // hand-assembled instruction: opcode 0xE4 (VAR read), operand-types byte.
    // text_buf=0x0200, parse_buf=0x0220, time=0x000A, routine=0x0040.
    // types: large,large,large,large -> 0b00_00_00_00 = 0x00.
    buf[0x10] = 0xE4; buf[0x11] = 0x00;
    buf[0x12] = 0x02; buf[0x13] = 0x00; // text_buf 0x0200
    buf[0x14] = 0x02; buf[0x15] = 0x20; // parse_buf 0x0220
    buf[0x16] = 0x00; buf[0x17] = 0x0A; // time = 10
    buf[0x18] = 0x00; buf[0x19] = 0x40; // routine = 0x40
    buf[0x1A] = 0x10;                   // store var (v5 read stores terminator)
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    let r = m.step();
    assert!(matches!(r, StepResult::NeedLine { .. }), "read suspends: {r:?}");
    assert_eq!(m.pending_timeout(), Some((10, 0x40)), "time+routine exposed");
}

#[test]
fn pending_timeout_none_when_untimed() {
    // Existing untimed v5 read (no time/routine) -> None.
    let mut buf = sample_story(5);
    let n = emit_read(&mut buf, 0x10, 0x0200, 0x0220, 5, Some(0x10));
    buf[0x10 + n] = 0xBA; // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    let _ = m.step();
    assert_eq!(m.pending_timeout(), None, "untimed read exposes no timeout");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm pending_timeout -- --nocapture`
Expected: FAIL — `pending_timeout` method does not exist (compile error).

- [ ] **Step 3: Implement**

In `crates/zvm/src/cpu/exec.rs`, extend `PendingInput` (add `#[derive(Clone, Copy)]` if not present):

```rust
#[derive(Clone, Copy)]
struct PendingInput {
    store_var: Option<u8>,
    text_buf: u32,
    parse_buf: u32,
    /// Timed-input interval in tenths of a second (0 = untimed).
    interrupt_time: u16,
    /// Packed address of the interrupt routine (0 = none).
    interrupt_routine: u16,
}
```

Update the `read` arm (0x04) to parse the extra operands and store them:

```rust
0x04 => {
    let text_buf = ops.first().copied().unwrap_or(0) as u32;
    let parse_buf = ops.get(1).copied().unwrap_or(0) as u32;
    let interrupt_time = ops.get(2).copied().unwrap_or(0);
    let interrupt_routine = ops.get(3).copied().unwrap_or(0);
    self.pending_input = Some(PendingInput {
        store_var: store, text_buf, parse_buf, interrupt_time, interrupt_routine,
    });
    StepResult::NeedLine { text_buf, parse_buf }
}
```

Update the `read_char` arm (0x16) — operands are `[1, time, routine]`:

```rust
0x16 => {
    let interrupt_time = ops.get(1).copied().unwrap_or(0);
    let interrupt_routine = ops.get(2).copied().unwrap_or(0);
    self.pending_input = Some(PendingInput {
        store_var: store, text_buf: 0, parse_buf: 0, interrupt_time, interrupt_routine,
    });
    StepResult::NeedChar
}
```

Find every other `PendingInput { ... }` construction (the save/restore-resumed read paths, if any) and add `interrupt_time: 0, interrupt_routine: 0` so it compiles. Then add the accessor method inside `impl Machine` (near `is_terminator`):

```rust
/// While a *timed* `read`/`read_char` is pending, return `(time_tenths,
/// packed_routine)`. `None` for an untimed read or when no read is pending.
/// The clock lives in the host: it polls input for `time_tenths * 100` ms and
/// calls `run_timed_interrupt` on each timeout.
pub fn pending_timeout(&self) -> Option<(u16, u16)> {
    let p = self.pending_input?;
    if p.interrupt_time != 0 && p.interrupt_routine != 0 {
        Some((p.interrupt_time, p.interrupt_routine))
    } else {
        None
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zvm`
Expected: PASS — new tests green; all existing tests still pass (untimed regression).

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): parse read/read_char time+routine, expose pending_timeout"
```

---

### Task 2: Engine — `run_timed_interrupt` (re-entrant routine execution)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (add `TimedInterrupt` type + `run_timed_interrupt`; uses `call_routine` from `state.rs:118` and `StepResult`)
- Test: same file, `tests` module

**Interfaces:**
- Consumes: `PendingInput.interrupt_routine` (Task 1); `crate::cpu::state::call_routine`.
- Produces: `pub struct TimedInterrupt { pub aborted: bool }` and `Machine::run_timed_interrupt(&mut self) -> TimedInterrupt`. Consumed by Tasks 4, 6.

- [ ] **Step 1: Write the failing test**

Two helper stories: an interrupt routine that returns 1 (abort) and one that returns 0 after a side effect (store 7 into global 5). A routine at address `R` with 0 locals: header byte `0x00`, then body. `rtrue` = 0OP `0xB0`, `rfalse` = `0xB1`. To store a global then return false: `store` is 2OP; simplest side-effect is `inc` (1OP:0x05) on a global then `rfalse`.

```rust
// Build a v5 story whose read at 0x10 uses time=5, routine=ROUT, and whose
// routine at ROUT increments global 0x10 then returns false (0).
fn timed_read_story(routine_body: &[u8]) -> (Vec<u8>, u32) {
    let mut buf = sample_story(5);
    let rout: u32 = 0x0300;
    // read: text=0x0200 parse=0x0220 time=5 routine=packed(ROUT).
    // v5 packed routine addr = byte addr / 4 (sample_story uses routine_offset 0).
    let packed = (rout / 4) as u16;
    buf[0x10]=0xE4; buf[0x11]=0x00;
    buf[0x12]=0x02; buf[0x13]=0x00;
    buf[0x14]=0x02; buf[0x15]=0x20;
    buf[0x16]=0x00; buf[0x17]=0x05;
    buf[0x18]=(packed>>8) as u8; buf[0x19]=(packed&0xff) as u8;
    buf[0x1A]=0x10;
    // routine header: 0 locals.
    buf[rout as usize] = 0x00;
    for (i, b) in routine_body.iter().enumerate() {
        buf[rout as usize + 1 + i] = *b;
    }
    (buf, rout)
}

#[test]
fn run_timed_interrupt_abort_when_routine_true() {
    // routine body: rtrue (0xB0).
    let (buf, _) = timed_read_story(&[0xB0]);
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    assert!(matches!(m.step(), StepResult::NeedLine { .. }));
    let depth_before = m.state.frames.len();
    let out = m.run_timed_interrupt();
    assert!(out.aborted, "routine returned true -> abort");
    assert_eq!(m.state.frames.len(), depth_before, "frame stack restored");
    assert!(m.pending_timeout().is_some(), "read still pending on abort=engine (host decides)");
}

#[test]
fn run_timed_interrupt_continue_and_side_effect() {
    // routine body: inc G0x10 (1OP:0x05 with variable operand 0x10), then rfalse.
    // 1OP short form, variable operand: opcode byte 0x95 (0b10_01_0101), operand 0x10.
    let (buf, _) = timed_read_story(&[0x95, 0x10, 0xB1]);
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    assert!(matches!(m.step(), StepResult::NeedLine { .. }));
    let g_before = m.global(0);
    let out = m.run_timed_interrupt();
    assert!(!out.aborted, "routine returned false -> continue");
    assert_eq!(m.global(0), g_before.wrapping_add(1), "routine side effect applied");
    assert!(m.pending_timeout().is_some(), "read still pending after continue");
}
```

*(Note: `global(0)` reads global variable 0x10, which is variable number 16; confirm the existing `global()` helper indexes globals from 0 — adjust the operand/index so the test's `inc` target and the asserted global match. If `m.global(n)` takes the 0-based global index, use variable number `0x10 + n`.)*

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm run_timed_interrupt`
Expected: FAIL — `run_timed_interrupt` / `TimedInterrupt` undefined.

- [ ] **Step 3: Implement**

Add near the input methods in `crates/zvm/src/cpu/exec.rs`:

```rust
/// Outcome of running a timed-input interrupt routine.
pub struct TimedInterrupt {
    /// The routine returned nonzero: the host should abort the pending read.
    pub aborted: bool,
}

impl Machine {
    /// Run the pending read's interrupt routine to completion and report whether
    /// input should abort. Called by the host once per elapsed timer interval.
    /// The routine's output flows to the normal sink; `pending_input` is left
    /// intact so an un-aborted read resumes. If the routine attempts nested
    /// input/save/restart (unsupported per ZMSD), the interrupt is abandoned and
    /// reported as non-aborting, with engine state restored.
    pub fn run_timed_interrupt(&mut self) -> TimedInterrupt {
        let saved = match self.pending_input {
            Some(p) if p.interrupt_routine != 0 => p, // PendingInput: Copy
            _ => return TimedInterrupt { aborted: false },
        };
        let base_frames = self.state.frames.len();
        let base_stack = self.state.eval_stack.len();
        // Push the routine, storing its return value onto the eval stack (var 0).
        crate::cpu::state::call_routine(
            &mut self.state, &mut self.mem, saved.interrupt_routine, &[], Some(0),
        );
        if self.state.frames.len() == base_frames {
            // packed 0 / bad addr: call_routine pushed 0 to the stack already.
            let ret = self.state.eval_stack.pop().unwrap_or(0);
            return TimedInterrupt { aborted: ret != 0 };
        }
        loop {
            match self.step() {
                StepResult::Continue => {
                    if self.state.frames.len() <= base_frames { break; }
                }
                // Nested input/save/restart/quit inside the routine: unsupported.
                // Unwind and restore, including pending_input (a nested read
                // opcode may have overwritten it).
                _ => {
                    self.state.frames.truncate(base_frames);
                    self.state.eval_stack.truncate(base_stack);
                    self.pending_input = Some(saved);
                    return TimedInterrupt { aborted: false };
                }
            }
        }
        let ret = self.state.eval_stack.pop().unwrap_or(0);
        // Guard: a well-behaved routine leaves the stack where we started.
        self.state.eval_stack.truncate(base_stack);
        TimedInterrupt { aborted: ret != 0 }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zvm run_timed_interrupt`
Expected: PASS. Then `cargo test -p zvm` — all green.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): run_timed_interrupt runs the interrupt routine to completion"
```

---

### Task 3: Engine — `abort_timed_input`

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (add method; reuses the buffer-writing logic in `supply_line` ~1635 and store logic)
- Test: same file, `tests` module

**Interfaces:**
- Consumes: `pending_input` (Task 1), the existing `supply_line`/`supply_char` buffer helpers.
- Produces: `Machine::abort_timed_input(&mut self, typed: &str)`. Consumed by Tasks 4, 6.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn abort_timed_input_read_char_stores_zero() {
    // v5 read_char at 0x10 -> NeedChar; abort stores 0 in the store var (G0).
    let mut buf = sample_story(5);
    buf[0x10]=0xF6; buf[0x11]=0x01; buf[0x12]=0x10; // read_char 1 -> G0x10
    buf[0x13]=0xBA;
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    assert_eq!(m.step(), StepResult::NeedChar);
    m.abort_timed_input("");
    assert_eq!(m.global(0), 0, "aborted read_char stores 0");
    assert!(m.pending_timeout().is_none(), "pending cleared after abort");
}

#[test]
fn abort_timed_input_read_writes_partial_and_terminator_zero() {
    // v5 read at 0x10 with time/routine; abort writes the partial buffer and
    // stores terminator 0.
    let (buf, _) = timed_read_story(&[0xB0]); // routine irrelevant here
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    assert!(matches!(m.step(), StepResult::NeedLine { .. }));
    m.abort_timed_input("no");
    // v5 text buffer: byte0=max, byte1=count, text from byte2.
    assert_eq!(m.mem.read_byte(0x0201), 2, "count = len('no')");
    assert_eq!(m.mem.read_byte(0x0202), b'n');
    assert_eq!(m.mem.read_byte(0x0203), b'o');
    assert_eq!(m.global(0), 0, "terminator stored is 0");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm abort_timed_input`
Expected: FAIL — method undefined.

- [ ] **Step 3: Implement**

Add to `impl Machine`. Reuse the same buffer-writing that `supply_line` performs; the simplest correct implementation delegates to `supply_line` for the line case (which already writes text + count and stores the terminator for v5+) and to `supply_char` for the char case:

```rust
/// Complete a pending timed read as *interrupted* (the interrupt routine
/// returned true / the game timed out): `read_char` stores 0; `read` writes the
/// partial `typed` line and stores terminator 0 (v5+). Clears `pending_input`.
pub fn abort_timed_input(&mut self, typed: &str) {
    match self.pending_input {
        Some(p) if p.text_buf == 0 => {
            // read_char: deliver ZSCII 0.
            self.supply_char(0);
        }
        Some(_) => {
            // read (line): partial buffer, terminator 0.
            self.supply_line(typed, 0);
        }
        None => {}
    }
}
```

*(Confirm `supply_char`/`supply_line` clear `pending_input` — they do, as the normal input path relies on it. If `supply_line`'s `trim`/lower-casing differs from desired, keep it: aborted input is normally discarded by the game, and matching the normal path is the least-surprising behavior.)*

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p zvm abort_timed_input` then `cargo test -p zvm`
Expected: PASS, all green.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): abort_timed_input completes a timed read as interrupted"
```

---

### Task 4: zvm-cli — poll-with-timeout + `--no-timed-input`

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (`read_char_input` ~324; `read_line_raw` ~394; `NeedLine`/`NeedChar` run-loop arms ~647/676; CLI args struct + `--no-timed-input`)

**Interfaces:**
- Consumes: `machine.pending_timeout()` (Task 1), `machine.run_timed_interrupt()` (Task 2), `machine.abort_timed_input()` (Task 3).
- Produces: (host behavior only)

- [ ] **Step 1: Add the CLI flag**

In the `clap`-derived args struct in `crates/zvm-cli/src/main.rs`, add:

```rust
/// Ignore game timers (timed read/read_char behave as untimed).
#[arg(long)]
no_timed_input: bool,
```

Thread a `bool timed = !args.no_timed_input;` into the run loop scope (next to `paging`, `honor`).

- [ ] **Step 2: Compute the timeout at the input arms**

In the `NeedChar` arm (~676) and `NeedLine` arm (~647), compute the per-read timeout before calling the read helper:

```rust
let timeout = if timed { machine.pending_timeout() } else { None };
```

- [ ] **Step 3: Timed `read_char`**

Change `read_char_input` to accept the timeout and the machine, poll with it, and run the interrupt on expiry. New signature and body:

```rust
/// Returns `(zscii, resize, aborted)`. When `timeout` is `Some((time,_))` the
/// read polls for `time*100` ms; on each timeout it runs the interrupt routine,
/// returning `aborted=true` if the routine aborts (caller stores 0).
fn read_char_input(
    is_tty: bool,
    machine: &mut Machine,
    timeout: Option<(u16, u16)>,
) -> (u8, Option<(u16, u16)>, bool) {
    if !is_tty {
        return (read_byte_stdin(), None, false);
    }
    let _ = terminal::enable_raw_mode();
    let mut last_resize = None;
    let result = loop {
        // Timed: poll with the interval; untimed: block.
        let got = match timeout {
            Some((t, _)) => event::poll(std::time::Duration::from_millis(t as u64 * 100))
                .unwrap_or(false),
            None => { let _ = event::read().map(|e| pending_event(e, &mut last_resize)); true },
        };
        if timeout.is_some() && !got {
            // Interval elapsed with no key: run the interrupt.
            let _ = terminal::disable_raw_mode();
            let out = machine.run_timed_interrupt();
            let _ = terminal::enable_raw_mode();
            if out.aborted { break (0u8, last_resize, true); }
            continue; // routine may have printed; caller re-renders next loop
        }
        match event::read() {
            Ok(Event::Key(KeyEvent { code, .. })) => break (decode_keycode(code), last_resize, false),
            Ok(Event::Resize(c, r)) => last_resize = Some((c, r)),
            _ => {}
        }
    };
    let _ = terminal::disable_raw_mode();
    result
}
```

*(Implementer note: the untimed branch must keep today's exact blocking semantics — if a `pending_event` helper does not already exist, keep the existing untimed loop body verbatim and only add the timed branch. Do not regress untimed `read_char`.)*

The `NeedChar` arm becomes:

```rust
let (ch, resize, aborted) = read_char_input(stdin_is_tty, &mut machine, timeout);
if let Some((nc, nr)) = resize { apply_resize(nr, nc, /* … */); }
if aborted {
    machine.abort_timed_input("");
} else {
    machine.supply_char(ch);
}
// re-render happens at the next loop's frame(); reset lines/current_col as today.
```

- [ ] **Step 4: Timed `read_line_raw`**

`read_line_raw` already owns `buf` and `&machine`. Add the timeout param and replace the blocking `event::read()` with a poll when timed. On timeout: `machine.run_timed_interrupt()`; if aborted, return with an `aborted` flag; else redraw the frame and continue (buffer preserved). Signature becomes:

```rust
fn read_line_raw(
    is_tty: bool,
    echo: zvm::io::TextAttrs,
    machine: &mut Machine,
    timeout: Option<(u16, u16)>,
) -> (String, u8, Option<(u16, u16)>, bool) // (line, terminator, resize, aborted)
```

Inside the loop, wrap the event read:

```rust
if let Some((t, _)) = timeout {
    if !event::poll(std::time::Duration::from_millis(t as u64 * 100)).unwrap_or(false) {
        let _ = terminal::disable_raw_mode();
        let out = machine.run_timed_interrupt();
        let _ = terminal::enable_raw_mode();
        if out.aborted { break_aborted = true; break; }
        // Re-render: the routine may have printed to the screen model.
        print!("{}", /* caller cannot frame here; see note */);
        continue;
    }
}
// existing: match event::read() { … terminator handling … }
```

*(Because `read_line_raw` does not hold the `ScreenView`, re-rendering after the routine prints is done by the caller: return a third state `NeedsRedraw` is overkill — instead, keep the loop but have the interrupt output flush through the sink; the game routine's text prints directly via `machine.out`. For the upper-window/frame refresh, the `NeedLine` arm already prints `view.frame(&machine)` on each entry; to refresh mid-line, pass a redraw closure `&mut dyn FnMut()` that prints `view.frame`. Implement the closure param.)*

Update the `NeedLine` arm to pass `timeout` and a redraw closure, and on `aborted` call `machine.abort_timed_input(&line)` instead of `supply_line`.

- [ ] **Step 5: Build + manual smoke**

Run: `cargo build -p zvm-cli` — expect clean build.
Run (manual, TTY): `./target/debug/zvm-cli stories/borderzone-r9-s871008.z5` — reach a timed scene; confirm the tick fires (routine text appears) and a keypress still submits. `--no-timed-input` blocks as before. (Automated PTY smoke is in Task 7's validation.)

- [ ] **Step 6: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit -m "feat(zvm-cli): poll input with the game timer; --no-timed-input"
```

---

### Task 5: app — `honor_timed_input` config + slash toggle + settings row

**Files:**
- Modify: `crates/app/src/config.rs` (mirror `honor_game_colours`: default fn ~183, serde field ~376, `Config::default` ~412, file-merge ~474, `write_config` ~531, test ~964)
- Modify: `crates/app/src/render/config_screen.rs` (row list ~23, value display ~171)
- Modify: `crates/app/src/input.rs` (config-screen toggle handlers ~3494 and ~3529)
- Modify: `crates/app/src/slash.rs` (add `toggle-timed-input` to the `COMMANDS` registry)

**Interfaces:**
- Produces: `config.honor_timed_input: bool` (default true), a `toggle-timed-input` command. Consumed by Tasks 6, 7.

- [ ] **Step 1: Config field + tests (mirror honor_game_colours)**

In `crates/app/src/config.rs`, add exactly parallel to every `honor_game_colours` site:

```rust
fn default_honor_timed_input() -> bool { true }
```
```rust
    #[serde(default = "default_honor_timed_input")]
    pub honor_timed_input: bool,
```
```rust
    honor_timed_input: default_honor_timed_input(), // in Config::default
```
```rust
    cfg.honor_timed_input = from_file.honor_timed_input; // in the file-merge fn
```
```rust
    doc["honor_timed_input"] = toml_edit::value(cfg.honor_timed_input); // in write_config
```

Add a test mirroring `honor_game_colours_defaults_true`:

```rust
#[test]
fn honor_timed_input_defaults_true() {
    let c = Config::default();
    assert!(c.honor_timed_input);
    let s = write_config_string(&c); // use the same helper the colour test uses
    let back: Config = toml::from_str(&s).unwrap();
    assert!(back.honor_timed_input);
    let off: Config = toml::from_str("honor_timed_input = false\n").unwrap();
    assert!(!off.honor_timed_input);
}
```

Run: `cargo test -p app honor_timed_input` — expect PASS.

- [ ] **Step 2: Settings screen row**

In `crates/app/src/render/config_screen.rs`, add to the row table (~line 23, after the colours row):

```rust
    ("honor_timed_input",    ConfigRowKind::Bool),
```

And in the value formatter (~171):

```rust
        11 => bool_str(cfg.honor_timed_input),
```

*(Use the correct next index; the colour row is index 10. Renumber subsequent arms if the table is index-matched — check the `match` for off-by-one.)*

In `crates/app/src/input.rs`, add the toggle at the two config-screen handler sites (mirror the `10 => … honor_game_colours` arms at ~3494 and ~3529):

```rust
        11 => { if let Some(cs) = &mut state.config_screen { cs.working.honor_timed_input = !cs.working.honor_timed_input; } }
```
```rust
        11 => working.honor_timed_input = !working.honor_timed_input,
```

- [ ] **Step 3: Slash command**

In `crates/app/src/slash.rs`, register `toggle-timed-input` in the `COMMANDS` registry (follow an existing boolean-toggle command such as the status-bar/room-number toggles). Its handler flips `state.config.honor_timed_input` and emits a quiet status-line confirmation (`Timed input: on/off`). Add it to the appropriate category (Game).

- [ ] **Step 4: Build + tests**

Run: `cargo build -p app && cargo test -p app config`
Expected: clean build, config tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/config.rs crates/app/src/render/config_screen.rs crates/app/src/input.rs crates/app/src/slash.rs
git commit -m "feat(app): honor_timed_input config, settings row, and slash toggle"
```

---

### Task 6: app — session interrupt/abort methods + surface the timeout

**Files:**
- Modify: `crates/app/src/session.rs` (add methods delegating to the engine; expose the pending timeout)

**Interfaces:**
- Consumes: `machine.pending_timeout()`, `machine.run_timed_interrupt()`, `machine.abort_timed_input()`.
- Produces on `GameSession`:
  - `pub fn pending_timeout(&self) -> Option<(u16, u16)>`
  - `pub fn run_timed_interrupt(&mut self) -> TurnResult` (drains routine output into a `TurnResult` like `submit`, `aborted` recorded)
  - `pub fn abort_timed_input(&mut self, typed: &str) -> TurnResult`
  Consumed by Task 7.

- [ ] **Step 1: Write the failing test**

Reuse `read_char_story_v5` (session.rs ~897) but assemble a timed variant, or add a helper. Minimal test: a timed `read_char` story whose routine returns true; `pending_timeout()` is `Some`, `run_timed_interrupt()` reports aborted, and the resulting `TurnResult` advances to the next input.

```rust
#[test]
fn session_surfaces_timeout_and_runs_interrupt() {
    // timed_read_char_story_v5(): read_char 1 time=5 routine=R; R returns rtrue.
    let bytes = timed_read_char_story_v5();
    let mut s = GameSession::new(bytes, true, None).unwrap();
    assert!(matches!(s.pending_input(), InputKind::Char));
    assert_eq!(s.pending_timeout(), Some((5, /* packed R */ EXPECTED_R)));
    let tr = s.run_timed_interrupt();
    assert!(tr.timed_out, "routine aborted the read");
}
```

*(Add a `timed_out: bool` field to `TurnResult`, defaulting false, set true by `abort` completion. Or assert on the follow-up state; pick one and keep it consistent with Task 7's needs — Task 7 needs to know the read was aborted so it re-renders and continues.)*

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app session_surfaces_timeout`
Expected: FAIL — methods undefined.

- [ ] **Step 3: Implement**

In `crates/app/src/session.rs`, add to `impl GameSession`:

```rust
/// While a timed read/read_char is pending, `(time_tenths, packed_routine)`.
pub fn pending_timeout(&self) -> Option<(u16, u16)> {
    self.machine.pending_timeout()
}

/// Run the pending read's interrupt routine once; returns a TurnResult carrying
/// any text the routine printed and whether input was aborted (`timed_out`).
pub fn run_timed_interrupt(&mut self) -> TurnResult {
    let out = self.machine.run_timed_interrupt();
    if out.aborted {
        self.abort_timed_input("")
    } else {
        // Routine printed but input continues: build a TurnResult from drained
        // output, keeping `pending` unchanged, `quit=false`, `timed_out=false`.
        self.collect_turn(false, false)
    }
}

/// Complete the pending read as timed-out with the partial `typed` line.
pub fn abort_timed_input(&mut self, typed: &str) -> TurnResult {
    self.machine.abort_timed_input(typed);
    // The game resumes after the read; step to the next input and collect output,
    // exactly like submit() does after supply_line.
    self.advance_after_input(true)
}
```

*(Implementer: factor the existing `submit`/`submit_char` post-input drain-and-step logic into a shared helper (`collect_turn` / `advance_after_input`) and reuse it here; do not duplicate. `submit` is at session.rs:201, `submit_char` at 209 — both already do "supply → step to next input → build TurnResult". Extract that tail.)*

Add `timed_out: bool` to `TurnResult` (default false in all existing constructors).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p app session`
Expected: PASS, all green.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/session.rs
git commit -m "feat(app): session run_timed_interrupt/abort + pending_timeout"
```

---

### Task 7: app — interleave the timer with the run loop + Border Zone validation

**Files:**
- Modify: `crates/app/src/main.rs` (the run loop with the `event::poll(Duration::from_millis(16))` tick ~903; the input-await / render path; `char_mode` update ~1467)
- Modify: `crates/app/src/state.rs` (add `input_deadline: Option<std::time::Instant>`)

**Interfaces:**
- Consumes: `session.pending_timeout()`, `session.run_timed_interrupt()`, `config.honor_timed_input` (Tasks 5, 6).

- [ ] **Step 1: Add the deadline state**

In `crates/app/src/state.rs`, add to the app state struct:

```rust
/// When a timed read is active and honored, the wall-clock instant the next
/// interrupt tick is due. `None` when no timer is armed.
pub input_deadline: Option<std::time::Instant>,
```
Initialize `None` in the state constructor(s).

- [ ] **Step 2: Arm the deadline when awaiting timed input**

Where the loop determines it is waiting for input (near the `char_mode` update at main.rs:1467), compute/refresh the deadline each iteration:

```rust
state.input_deadline = if state.config.honor_timed_input {
    session.pending_timeout().map(|(t, _)| {
        std::time::Instant::now() + std::time::Duration::from_millis(t as u64 * 100)
    })
} else {
    None
};
```
Only set this when actually awaiting game input (not while a dialog/overlay is open). Guard with the existing "is the game waiting for input" condition.

- [ ] **Step 3: Bound the poll timeout by the deadline**

At the poll site (main.rs:903, currently `event::poll(Duration::from_millis(16))`), clamp the wait so the loop wakes at the deadline:

```rust
let poll_ms = match state.input_deadline {
    Some(dl) => {
        let now = std::time::Instant::now();
        let remaining = dl.saturating_duration_since(now).as_millis() as u64;
        remaining.min(16).max(1) // keep the 16ms UI cadence as the ceiling
    }
    None => 16,
};
let has_event = crossterm::event::poll(std::time::Duration::from_millis(poll_ms)).unwrap_or(false);
```

- [ ] **Step 4: Fire the interrupt on deadline**

After the poll, before handling events, check the deadline:

```rust
if !has_event {
    if let Some(dl) = state.input_deadline {
        if std::time::Instant::now() >= dl {
            let tr = session.run_timed_interrupt();
            apply_turn_result(&mut state, tr); // same path submit() results use
            // Re-arm or clear the deadline: if still awaiting input, Step 2 resets
            // it next iteration; if the read aborted, pending_timeout() is None.
            state.dirty = true; // force a re-render (routine may have printed)
        }
    }
}
```

*(Use whatever function the loop already calls to apply a `TurnResult` from `submit` — find the call at main.rs:2458's result handling and reuse it. Do not duplicate transcript/append logic.)*

- [ ] **Step 5: Build + workspace tests**

Run: `cargo build -p app && cargo test --workspace`
Expected: clean build; full suite green.

- [ ] **Step 6: Manual Border Zone validation (both hosts)**

- zvm-cli: `./target/debug/zvm-cli stories/borderzone-r9-s871008.z5` — reach a real-time scene; confirm the clock ticks (routine text updates), a typed command still works, and inaction triggers the timed outcome. `--no-timed-input` makes it wait indefinitely.
- app: `cargo run -p app -- stories/borderzone-r9-s871008.z5` — same, plus toggle `honor_timed_input` off via the slash command / F2 settings and confirm timers stop firing.

Record the outcome in the commit message.

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/main.rs crates/app/src/state.rs
git commit -m "feat(app): interleave game timer with the run loop (timed input)"
```

---

## Self-Review

**Spec coverage:** §3 engine (parse + interrupt + abort) → Tasks 1-3. §4 zvm-cli (poll + flag) → Task 4. §5 app (config + slash + settings + interleave) → Tasks 5-7. §6 data flow / §7 abort semantics → exercised by Tasks 2-3 tests + Task 7 validation. §8 testing → per-task tests + Task 7 Border Zone. All spec sections have a task.

**Deviation (flagged):** `pending_timeout()` accessor replaces the spec's `StepResult` field extension (§3.2) — behavior-identical, less invasive. Documented in Global Constraints.

**Open implementer confirmations (called out inline, not placeholders):** (a) `global(n)` indexing base in Task 2's test; (b) that `supply_line`/`supply_char` clear `pending_input` (Task 3); (c) the exact shared post-input drain helper to extract in Task 6; (d) the config-screen row index and the `TurnResult`-apply function name in Tasks 5/7. Each names the exact existing site to check.
