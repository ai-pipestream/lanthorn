# Z-Machine Input-Key Handling — Design

**Date:** 2026-06-30
**Status:** Approved (design)
**Scope source:** `docs/superpowers/audits/2026-06-30-zmachine-feature-gaps.md` — the input-key cluster (audit §D/§F ADD-soon items) that survived the colour/erase waves.

## Goal

Complete Z-machine input-key handling across the engine and both front-ends:

1. Line input terminates on the game's declared terminating keys (header 0x2E table), not just Enter, and the actual terminating key's ZSCII code is stored (v5+).
2. The function-key ZSCII range 129–144 is decoded in both hosts (up from 129–136 / F1–F4).
3. Delete (ZSCII 8) and ESC (ZSCII 27) input codes are verified end-to-end for `read_char`.

## Non-Goals

- Timed / interrupt input (`read` / `read_char` time+routine operands) — deferred (audit DEFER).
- Keypad ZSCII 145–154 — terminals report keypad digits as ordinary `Char`, so these codes are unreachable in a TUI host. Documented as unreachable; not implemented (no dead code).
- v6 mouse input (`read_mouse`) — out of scope.
- `gvm` (Glulx) — has its own separate `supply_line`; untouched.

## Current State (main @ `29c3a9a`)

- `zvm` `supply_line(&mut self, input: &str)` hardcodes the stored terminator to `13` (`crates/zvm/src/cpu/exec.rs:1621`, TODO at `:1669`).
- `zvm` `is_terminator(u16)` correctly reads the header 0x2E terminating-characters table (`exec.rs:1431`), including the `255` = any-function-key wildcard.
- `zvm` `supply_char(u8)` stores the raw ZSCII byte — correct, unchanged.
- `zvm-cli` `decode_keycode` (`crates/zvm-cli/src/main.rs:258`) maps: Backspace/Delete→8, Esc→27, Up/Down/Left/Right→129–132, F1–F4→133–136. Used by the `read_char` (NeedChar) path.
- `zvm-cli` `read_line_raw` (the NeedLine raw-mode editor) handles printable chars, Backspace, Enter only — no function-key termination; calls `supply_line` with no terminator.
- `app` `key_input_to_zscii` (`crates/app/src/session.rs:501`) maps Enter→13, Backspace→8, Escape→27, Up/Down/Left/Right→129–132, `Func(n)`→132+n (already covers F1–F12), ASCII `Char`. Used by the `read_char` (submit_key) path.
- `app` line input routes printable/Enter/Backspace to the prompt buffer; Enter submits via `supply_line(command)`. Up/Down do command-history recall. No function-key termination.

## Design

### Component 1 — Engine API: thread the terminator (`zvm`)

Change the signature:

```rust
// crates/zvm/src/cpu/exec.rs
pub fn supply_line(&mut self, input: &str, terminator: u8)
```

Behavior:
- v5+: store `terminator` (as `u16`) into the read's store variable instead of the hardcoded `13`.
- v1–4: no store variable exists for `read`; `terminator` is ignored. Callers pass `13`.
- The `debug_assert!(self.is_terminator(term))` guard and the `TODO function-key terminator threading` comment at `exec.rs:1664–1670` are removed.

Rationale for signature change over an additive `supply_line_terminated`: only ~9 call sites total (2 production, ~7 tests); one method keeps the terminator explicit at every call and removes the footgun of a method that silently lies about the terminator.

Call-site updates:
- `crates/app/src/session.rs:196` — pass the terminator the app computed (see Component 3); Enter path passes `13`.
- `crates/zvm-cli/src/main.rs:629` — pass the terminator `read_line_raw` returns (see Component 3); Enter path passes `13`.
- `crates/zvm/tests/regression.rs:26` and the ~6 `exec.rs` unit-test callers — pass `13`.

`is_terminator` is unchanged and becomes the shared oracle both hosts consult.

### Component 2 — Function-key range 129–144

- **zvm-cli** `decode_keycode`: extend the `F(1..=4) → 133..=136` arms to `F(1..=12) → 132 + n`. Guard `n <= 12`.
- **app**: `key_input_to_zscii` already computes `Func(n) → 132 + n`. Verify `crates/app/src/input.rs` produces `KeyInput::Func(n)` for crossterm `KeyCode::F(5..=12)` (extend the mapping if it caps at 4).
- Keypad 145–154: add a one-line doc comment at each decode site noting these are unreachable in terminal hosts; do not implement.

### Component 3 — Line-input termination (full parity)

Both hosts, when an `InputKind::Line` read is pending, end the line on a terminating key and pass its ZSCII to `supply_line`.

**zvm-cli** (`read_line_raw`):
- On a non-editing special key (arrow/function/etc.), compute its ZSCII via `decode_keycode`.
- If `machine.is_terminator(zscii)` → stop editing and return `(buffer, zscii)`.
- Enter → return `(buffer, 13)` (unchanged terminator, new return shape).
- Return type extends from the current line string to `(String, u8 /* terminator */, Option<(u16,u16)> /* resize */)` (fold the terminator into the existing tuple).
- The NeedLine handler passes the returned terminator to `supply_line`.

**app**:
- Expose `GameSession::is_terminator(zscii: u8) -> bool` delegating to the engine, so the input router can classify keys against the *current* pending read.
- When `InputKind::Line` is pending and a special key arrives whose ZSCII (via `key_input_to_zscii`) satisfies `is_terminator`, submit the current input buffer with that terminator **instead of** the key's normal action (e.g. Up/Down history recall). Enter continues to submit with `13`.
- The submit path carries a terminator byte down to `supply_line`. `GameSession::submit` (the Enter path) passes `13`; a new terminated-submit path passes the function-key ZSCII.

This makes BeyondZork's cursor-key-driven hint menu (a v5+ `read` with a terminating-characters table) work in both hosts.

### Component 4 — ZSCII 8/27 verification

Both hosts already map delete→8 and ESC→27 for `read_char`. This component adds end-to-end tests asserting the store variable receives 8 and 27. Expected to pass with no behavior change; a failure reveals a real host→ZSCII gap to fix.

## Data Flow

```
key event ──► host decode (decode_keycode / key_input_to_zscii) ──► ZSCII byte
                                                                      │
   read_char (NeedChar): ─────────────────────────────► supply_char(zscii)
                                                                      │
   read line (NeedLine): edit buffer; on terminating key ──► supply_line(text, terminator)
                          (host consults machine.is_terminator)
```

## Error Handling

- Unknown/unmapped keys: hosts return `None`/skip (existing behavior) — the read stays pending.
- Non-terminating special keys during line editing: ignored for termination; retain existing editing/nav behavior (e.g. app Up/Down history recall when the key is not a terminator).
- v3/v4 `read`: terminator argument ignored by the engine; hosts pass `13`.

## Testing

**zvm** (`crates/zvm/src/cpu/exec.rs` tests):
- `supply_line(text, 13)` on v3/v4 writes text + null/count correctly, ignores terminator (no store var).
- `supply_line(text, term)` on v5 stores `term` in the read's store variable.
- `supply_line(text, 129)` on v5 with a terminating table containing 255 stores 129.
- Existing `supply_line` tests updated to the new signature (pass `13`).

**zvm-cli** (`crates/zvm-cli/src/main.rs` tests):
- `decode_keycode(F(5))..(F(12))` → 137..144.
- `read_line_raw` returns the terminating key's ZSCII when a terminator key ends the line (unit-testable at the decode boundary; the raw editor loop is exercised by the existing PTY harness for integration).

**app** (`crates/app/src/session.rs` tests):
- `key_input_to_zscii(Func(5..12))` → 137..144.
- `is_terminator` delegate returns the engine's answer.
- Pending `InputKind::Line` + a terminator key → submit-with-terminator (buffer + terminator reach `supply_line`); a non-terminator special key does not submit.
- `read_char` delivers ZSCII 8 (delete) and 27 (ESC) to the store variable.

## Files Touched

- `crates/zvm/src/cpu/exec.rs` — `supply_line` signature + terminator store; tests.
- `crates/zvm/tests/regression.rs` — call-site update.
- `crates/zvm-cli/src/main.rs` — `decode_keycode` F5–F12; `read_line_raw` termination + return shape; NeedLine handler.
- `crates/app/src/session.rs` — `is_terminator` delegate; terminated-submit path; `supply_line` call-site; tests.
- `crates/app/src/input.rs` — `Func(5..12)` mapping (if capped); line-input terminator routing.

## Constraints

- `zvm` stays zero-dependency.
- Cross-platform (Windows/Linux/macOS); zvm-cli/app may use crossterm (already dependencies).
- 0 warnings + full workspace test suite green per task.
