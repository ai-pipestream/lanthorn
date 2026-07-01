# Z-Machine Timed Input — Design

**Date:** 2026-07-01
**Scope:** Support the Z-machine `read` / `read_char` `time` + `routine`
operands (timed / interrupt input) in the engine (`zvm`) and both hosts
(`zvm-cli`, `app`). The engine currently parses but ignores these operands.
**Validation target:** Border Zone (`stories/borderzone-r9-s871008.z5`), the
canonical real-time timed-input game, plays its timed scenes in both hosts.

## 1. Background

Z-machine v4+ `read` (VAR:0x04) and `read_char` (VAR:0x16) accept two optional
operands: `time` (in tenths of a second) and `routine` (a packed routine
address). While waiting for input, the interpreter calls `routine` every `time`
tenths of a second. If the routine returns true (nonzero), input is aborted;
if false, input continues. The routine may print (e.g. a ticking clock or a
status update). ZMSD §10.7 (timed input), §15 (`read_char`).

Today (`crates/zvm/src/cpu/exec.rs:842`) the operands are read but discarded,
and both hosts block on input with no timer.

**Constraint:** `zvm` is zero-dependency and never reads a wall clock. The
clock lives in the hosts; the engine only *runs the routine on demand* when the
host says the interval elapsed.

## 2. Architecture (chosen: engine owns the interrupt)

The one hard part — running the interrupt routine as re-entrant Z-code while a
`read` is suspended — stays inside the engine, where the call stack lives. The
host contributes only the clock and the input poll.

- **Engine** exposes the interval to the host and offers a single method that
  runs the routine to completion and reports whether input should abort.
- **Hosts** poll input with a timeout; on each timeout they call that method,
  then either abort the read or re-render and keep waiting.

Rejected alternatives: (B) a generic `call_routine_sync` on the host — leaks
abort/buffer semantics into both hosts and invites divergence; (C) an
engine-internal poll loop with injected clock/input closures — inverts the
clean `step()`/`supply_*` control flow and couples the engine to I/O + time.

## 3. Engine changes (`crates/zvm/src/cpu/exec.rs`, zero-dep)

### 3.1 Operand parsing
- `read` (0x04): `time = ops[2]`, `routine = ops[3]` (absent ⇒ 0).
- `read_char` (0x16): `time = ops[1]`, `routine = ops[2]` (`ops[0]` must be 1).
- Store on `PendingInput`: new fields `interrupt_time: u16` (tenths of a
  second), `interrupt_routine: u16` (packed address).
- `time == 0 || routine == 0` ⇒ **untimed** (host blocks as today).

### 3.2 `StepResult` carries the interval
```
NeedLine { text_buf: u32, parse_buf: u32, time: u16, routine: u16 }
NeedChar { time: u16, routine: u16 }
```
Existing match arms in both hosts and the engine tests update to the new shape.

### 3.3 `run_timed_interrupt`
```
pub struct TimedInterrupt { pub aborted: bool }
pub fn run_timed_interrupt(&mut self) -> TimedInterrupt
```
- Precondition: a `pending_input` with a nonzero `interrupt_routine`.
- Record `base = self.state.frames.len()`. Push the routine via the existing
  `call_routine(interrupt_routine, &[], …)`. Step the machine until
  `frames.len() == base`, capturing the routine's return value; `aborted =
  ret != 0`.
- Routine output flows to the normal `out` sink (a ticking clock prints).
- `pending_input` is left intact so the suspended read resumes on `false`.
- **Guard:** if stepping the routine yields a nested `NeedLine` / `NeedChar` /
  `SaveRequest` / `RestoreRequest`, abandon the interrupt and return
  `{ aborted: false }` (interrupt routines must not do I/O — ZMSD; documented
  limitation, no game in the library relies on it).

### 3.4 `abort_timed_input`
```
pub fn abort_timed_input(&mut self, typed: &str)
```
Completes the suspended read as timed-out, then clears `pending_input`:
- `read_char` (no `text_buf`): store `0` into the store var.
- `read`: write `typed` (the partial line entered so far) + its count into the
  text buffer as `supply_line` would, and store terminator `0` (v5+); v3 `read`
  has no store var (null-terminate the partial buffer). Frotz-compatible.

## 4. zvm-cli host (`crates/zvm-cli/src/main.rs`)

- CLI flag **`--no-timed-input`** (default: timed **ON**). When set, the host
  passes `time` as 0 so reads block exactly as before.
- `read_char_input` and `read_line_raw`: replace the blocking `event::read()`
  with `event::poll(Duration::from_millis(time as u64 * 100))` + `read()` when
  `time > 0`. On a real event → today's key handling (`read_line_raw` already
  owns `buf` and takes `&machine`, so a partial line survives ticks). On
  timeout → `machine.run_timed_interrupt()`; if `aborted` →
  `machine.abort_timed_input(buf)` and signal the run loop; else re-render
  (`print!("{}", view.frame(&machine))`) and re-poll (reset the interval).
- The `NeedLine` / `NeedChar` run-loop arms pass `time`/`routine` into the read
  helpers and, on abort, skip the normal `supply_*` (the engine already
  completed the read).

## 5. app host

- Config **`honor_timed_input: bool`** (default **true**) in `config.rs`
  (file-merge + `write_config`); a **slash command** `toggle-timed-input` in the
  `slash::COMMANDS` registry; a settings-screen (F2) row — mirroring
  `honor_game_colours`.
- `session.rs` surfaces the pending timeout (`time`/`routine`) alongside
  `RunStop::Input`, and exposes `run_timed_interrupt` / `abort_timed_input`.
- `main.rs`'s run loop already polls events with a periodic tick (the tidy
  pulse). When a timed input is active **and** `honor_timed_input`, set the poll
  deadline to the timer interval; on expiry → `run_timed_interrupt`; `aborted`
  → complete-as-interrupted; else mark the frame dirty (re-render — the routine
  may have printed to the upper window / transcript) and reset the deadline.
  The partial line lives in the app's input state, so it is preserved across
  ticks. `honor_timed_input == false` ⇒ untimed.

## 6. Data flow (Border Zone)

`read text parse 10 R` (1.0 s): host polls 1 s → no key → `run_timed_interrupt`
runs `R` (prints "The border guard steps closer.", decrements a counter,
returns 0) → host re-renders, waits another 1 s → eventually `R` returns 1 →
host `abort_timed_input` (terminator 0) → the game takes its "caught" branch. A
key typed before the timeout submits a normal command (terminator 13 or a
function-key terminator from the existing terminating-chars support).

## 7. Abort semantics (ZMSD §10.7, §15)

- `read_char` interrupted → store `0`.
- `read` interrupted → store terminator `0` (v5+) + the partial typed buffer.
- Routine return: `0` = keep waiting; nonzero = abort.
- `routine == 0` or `time == 0` → never arm the timer (untimed).
- Nested input inside a routine → unsupported; the interrupt bails to
  `{ aborted: false }` (documented limitation).

## 8. Testing / validation

**Engine unit tests (no TTY):**
- `read_char` with a routine that returns 1 → `run_timed_interrupt().aborted`;
  after `abort_timed_input("")` the store var holds `0`.
- Routine returns 0 with a visible side effect (increments a global) →
  `aborted == false`, the global changed, `frames.len()` restored (no stack
  corruption), `pending_input` still present.
- Timed `read` partial-buffer abort → text buffer holds the partial line, v5
  store var holds terminator `0`.
- Untimed path (`time == 0`) unchanged — regression guard.
- Nested-input guard → `run_timed_interrupt` returns `aborted == false` and does
  not corrupt state.

**Host validation:**
- zvm-cli: PTY smoke on Border Zone — drive to a timed scene, confirm the tick
  fires (routine output appears), a keypress submits normally, and the timeout
  aborts. `--no-timed-input` reproduces the pre-feature blocking behavior.
- app: manual Border Zone playthrough; `honor_timed_input` toggle (slash +
  settings) flips between timed and untimed.

## 9. File / unit map

| File | Change |
|------|--------|
| `crates/zvm/src/cpu/exec.rs` | operand parse; `StepResult` fields; `run_timed_interrupt`; `abort_timed_input`; `PendingInput` fields |
| `crates/zvm-cli/src/main.rs` | poll-with-timeout in the read helpers; `--no-timed-input` flag; abort wiring |
| `crates/app/src/session.rs` | surface timeout; expose interrupt/abort |
| `crates/app/src/main.rs` | interleave the timer with the render/input loop |
| `crates/app/src/config.rs` | `honor_timed_input` flag (default true) |
| `crates/app/src/slash.rs` (+ settings screen) | `toggle-timed-input` command + F2 row |

## 10. Out of scope

- Nested input inside an interrupt routine (bail-to-continue; documented).
- Sound-finished callbacks (`sound_effect` routine operand) — separate Blorb
  audio work; this design covers only `read`/`read_char` timers.
- Sub-100 ms interval precision (poll granularity is the interval itself).
