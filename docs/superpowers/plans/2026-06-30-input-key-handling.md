# Z-Machine Input-Key Handling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Line input terminates on the game's declared terminating keys (storing the actual key), the full function-key ZSCII range 129–144 is decoded, and delete/ESC (8/27) are verified end-to-end — across the `zvm` engine, `zvm-cli`, and `app`.

**Architecture:** The engine gains an explicit terminator argument on `supply_line`; both hosts consult the engine's existing `is_terminator` oracle to decide when a special key ends line input and pass that key's ZSCII code back. The app exposes the capability through two `Engine`-trait methods (Glulx-safe defaults) and reuses its existing `SubmitCommand` turn-application path via a `pending_terminator` flag.

**Tech Stack:** Rust workspace; `zvm` (zero-dependency VM), `zvm-cli` (crossterm), `app` (ratatui TUI). Design: `docs/superpowers/specs/2026-06-30-input-key-handling-design.md`.

## Global Constraints

- `zvm` stays zero-dependency (no new crates in `crates/zvm`).
- Cross-platform (Windows/Linux/macOS); `zvm-cli`/`app` may use crossterm (already dependencies).
- 0 compiler warnings and full workspace test suite green at the end of every task.
- Stage only the files each task names; never `git add -A`. Scratch stays under `.superpowers/`.
- `gvm` (Glulx) has its own separate `supply_line`; it is out of scope and must not change behavior.
- Function keys map to ZSCII 129–144 (arrows 129–132, F1–F12 133–144). Keypad 145–154 is unreachable in a terminal host (keypad digits arrive as ordinary `Char`); do not implement it — document it at each decode site.
- Commit trailers on every commit:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
  No backticks in commit message bodies.

---

## File Structure

- `crates/zvm/src/cpu/exec.rs` — `supply_line` gains a `terminator: u8` param and stores it (v5+); new tests. (Task 1)
- `crates/zvm/tests/regression.rs` — one `supply_line` call-site update. (Task 1)
- `crates/zvm-cli/src/main.rs` — `decode_keycode` F5–F12 (Task 2); `line_terminator` helper + `read_line_raw` termination + NeedLine wiring (Task 3).
- `crates/app/src/engine.rs` — two new `Engine`-trait methods with defaults. (Task 4)
- `crates/app/src/session.rs` — `GameSession` overrides for the two methods + tests. (Task 4)
- `crates/app/src/state.rs` — `pending_terminator` field. (Task 5)
- `crates/app/src/main.rs` — `line_terminator_action` gate + `SubmitCommand` handler terminator handling. (Task 5)

---

## Task 1: Engine — thread the terminator through `supply_line`

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (`supply_line` at ~1621; test module callers)
- Modify: `crates/zvm/tests/regression.rs:26`
- Test: `crates/zvm/src/cpu/exec.rs` (test module)

**Interfaces:**
- Produces: `pub fn supply_line(&mut self, input: &str, terminator: u8)` — stores `terminator` (as `u16`) into the read's store variable for v5+; ignores it for v1–4 (no store variable). Existing `pub fn is_terminator(&self, ch: u16) -> bool` and `pub fn supply_char(&mut self, ch: u8)` are unchanged.

- [ ] **Step 1: Update the two new failing tests**

Add to the `exec.rs` test module (near the existing `read_v5_stores_terminator` test):

```rust
    #[test]
    fn supply_line_v5_stores_function_key_terminator() {
        // v5 read terminated by a cursor key (ZSCII 129) stores 129, not 13.
        let (mut buf, ..) = build_input_story(5);
        let text_buf: u16 = 0x0250; buf[text_buf as usize] = 20;
        let parse_buf: u16 = 0x0260; buf[parse_buf as usize] = 8;
        let n = emit_read(&mut buf, 0x0010, text_buf, parse_buf, 5, Some(0x11));
        buf[0x0010 + n] = 0xBA;

        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x0010;
        m.step();
        m.supply_line("look", 129);

        let g1 = m.mem.read_word(m.mem.global_vars() as u32 + 1 * 2);
        assert_eq!(g1, 129, "v5 read stores the function-key terminator in G1");
    }

    #[test]
    fn supply_char_stores_delete_and_escape() {
        // ZSCII 8 (delete/backspace) and 27 (ESC) reach the read_char store var.
        for z in [8u8, 27u8] {
            let mut buf = sample_story(5);
            buf[0x0010] = 0xF6; // VAR read_char
            buf[0x0011] = 0x7F; // small const, omit, omit, omit
            buf[0x0012] = 1;    // device = keyboard
            buf[0x0013] = 0x10; // store → G0
            buf[0x0014] = 0xBA; // quit
            let mem = Memory::new(buf).unwrap();
            let mut m = Machine::new(mem);
            m.state.pc = 0x0010;
            assert_eq!(m.step(), StepResult::NeedChar);
            m.supply_char(z);
            assert_eq!(m.global(0), z as u16, "supply_char({z}) stored in G0");
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test -p zvm supply_line_v5_stores_function_key_terminator 2>&1 | tail -20`
Expected: FAIL — `supply_line` takes 1 argument but 2 were supplied (and the whole crate's test build fails on the new 2-arg call).

- [ ] **Step 3: Change the `supply_line` signature and terminator store**

In `crates/zvm/src/cpu/exec.rs`, change the signature:

```rust
    pub fn supply_line(&mut self, input: &str, terminator: u8) {
```

Update the doc line that currently reads "For v5+: stores the terminating character (13 = Enter) into the store variable." to:

```rust
    /// For v5+: stores `terminator` (the ZSCII code of the key that ended the
    /// line — 13 for Enter, or a function-key code the host matched against the
    /// terminating-characters table) into the store variable. v1–4 have no store
    /// variable for `read`, so `terminator` is ignored there.
```

Replace the v5+ store block (the `if version >= 5 { let term: u16 = 13; debug_assert!(...); self.do_store(...); }` and its preceding comment + `TODO function-key terminator threading` lines) with:

```rust
        // v5+: store the terminating character the host supplied. `is_terminator`
        // is the host's oracle for which keys may end line input (ZMSD §10.7); by
        // the time we get here the host has already applied it.
        if version >= 5 {
            self.do_store(pending.store_var, terminator as u16);
        }
```

- [ ] **Step 4: Update all in-crate `supply_line` call sites to the new signature**

Every call passes `13` (behavior-preserving — Enter was the old hardcoded terminator). In the `exec.rs` test module, the calls are (by their string argument): `"north"` (two calls), `"open mailbox"`, `"hello"`, and `"NORTH"` — append `, 13` to each:

```rust
        m.supply_line("north", 13);
        // ...
        m.supply_line("open mailbox", 13);
        // ...
        m.supply_line("hello", 13);
        // ...
        m.supply_line("NORTH", 13);
```

In `crates/zvm/tests/regression.rs:26`:

```rust
                machine.supply_line("", 13);
```

(The two production hosts — `crates/app/src/session.rs:196` and `crates/zvm-cli/src/main.rs:629` — are updated to `, 13` in this same step so the workspace builds; Tasks 3 and 5 replace those `13`s with real terminators.)

`crates/app/src/session.rs:196`:

```rust
        self.machine.supply_line(command, 13);
```

`crates/zvm-cli/src/main.rs:629`:

```rust
                machine.supply_line(line.trim_end(), 13);
```

- [ ] **Step 5: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -25`
Expected: PASS — all tests green, including the two new ones.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 6: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs crates/zvm/tests/regression.rs crates/app/src/session.rs crates/zvm-cli/src/main.rs
git commit -F - <<'EOF'
feat(zvm): thread line-input terminator through supply_line

supply_line now takes an explicit terminator and stores it (v5+) instead
of hardcoding 13, so hosts can report a function key that ended the line.
Also verifies supply_char delivers ZSCII 8 (delete) and 27 (ESC). All
call sites pass 13 for now; hosts compute real terminators in later tasks.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 2: zvm-cli — decode function keys F5–F12

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (`decode_keycode` at 258–274; its test near 780)
- Test: `crates/zvm-cli/src/main.rs` (test module)

**Interfaces:**
- Consumes: nothing from Task 1.
- Produces: `decode_keycode(KeyCode::F(n))` returns `132 + n` for `n` in `1..=12` (F1→133 … F12→144). Delete/Backspace→8, Esc→27, arrows→129–132 unchanged.

- [ ] **Step 1: Extend the failing test**

In the `decode_keycode` test (currently asserting F1–F4 = 133–136), add F5–F12:

```rust
        assert_eq!(decode_keycode(KeyCode::F(5)), 137);
        assert_eq!(decode_keycode(KeyCode::F(12)), 144);
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p zvm-cli decode 2>&1 | tail -20`
Expected: FAIL — `decode_keycode(KeyCode::F(5))` returns `b'\n'` (10, the catch-all), not 137.

- [ ] **Step 3: Replace the F1–F4 arms with a range arm**

In `crates/zvm-cli/src/main.rs`, replace:

```rust
        KeyCode::F(1) => 133,
        KeyCode::F(2) => 134,
        KeyCode::F(3) => 135,
        KeyCode::F(4) => 136,
```

with:

```rust
        // Function keys F1–F12 → ZSCII 133–144 (ZMSD §3.8). Keypad digits
        // (ZSCII 145–154) are unreachable: terminals report them as ordinary
        // Char events, indistinguishable from the number row.
        KeyCode::F(n) if (1..=12).contains(&n) => 132 + n,
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p zvm-cli decode 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit -F - <<'EOF'
feat(zvm-cli): decode function keys F5-F12 to ZSCII 137-144

Extends decode_keycode from F1-F4 to the full F1-F12 range (ZMSD 3.8).
Keypad 145-154 stays unimplemented (unreachable in a terminal host).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 3: zvm-cli — terminate line input on function keys

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (`read_line_raw` at 366–421; NeedLine handler at 624–629; new `line_terminator` helper)
- Test: `crates/zvm-cli/src/main.rs` (test module)

**Interfaces:**
- Consumes: `supply_line(input, terminator)` from Task 1; `decode_keycode` from Task 2; `Machine::is_terminator`.
- Produces: `read_line_raw(is_tty: bool, echo: TextAttrs, is_term: impl Fn(u16) -> bool) -> (String, u8, Option<(u16, u16)>)` — the middle `u8` is the ZSCII terminator (13 for Enter). New free fn `line_terminator(code: KeyCode, is_term: impl Fn(u16) -> bool) -> Option<u8>`.

- [ ] **Step 1: Write the failing tests for the `line_terminator` helper**

Add to the `zvm-cli` test module:

```rust
    #[test]
    fn line_terminator_enter_always_ends_line() {
        use crossterm::event::KeyCode;
        assert_eq!(line_terminator(KeyCode::Enter, |_| false), Some(13));
    }

    #[test]
    fn line_terminator_editing_keys_never_end_line() {
        use crossterm::event::KeyCode;
        assert_eq!(line_terminator(KeyCode::Char('x'), |_| true), None);
        assert_eq!(line_terminator(KeyCode::Backspace, |_| true), None);
    }

    #[test]
    fn line_terminator_function_key_only_when_listed() {
        use crossterm::event::KeyCode;
        // Up arrow → ZSCII 129: ends the line only when the table lists it.
        assert_eq!(line_terminator(KeyCode::Up, |z| z == 129), Some(129));
        assert_eq!(line_terminator(KeyCode::Up, |_| false), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p zvm-cli line_terminator 2>&1 | tail -20`
Expected: FAIL — `line_terminator` is not defined.

- [ ] **Step 3: Add the `line_terminator` helper**

Add near `decode_keycode` in `crates/zvm-cli/src/main.rs`:

```rust
/// Decide whether a key ends line input in raw mode, returning the ZSCII code to
/// store as the terminator. Enter always ends the line (13). A function/special
/// key ends it only if the game lists its ZSCII code in the terminating-
/// characters table (`is_term`, backed by `Machine::is_terminator`; ZMSD §10.7).
/// Editing keys (printable Char, Backspace) return None — the caller handles them.
fn line_terminator(code: KeyCode, is_term: impl Fn(u16) -> bool) -> Option<u8> {
    match code {
        KeyCode::Enter => Some(13),
        KeyCode::Char(_) | KeyCode::Backspace => None,
        other => {
            let z = decode_keycode(other) as u16;
            if z != 13 && is_term(z) {
                Some(z as u8)
            } else {
                None
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p zvm-cli line_terminator 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Rewrite `read_line_raw` to consult `line_terminator` and return the terminator**

Change the signature and the non-TTY fallback:

```rust
fn read_line_raw(
    is_tty: bool,
    echo: zvm::io::TextAttrs,
    is_term: impl Fn(u16) -> bool,
) -> (String, u8, Option<(u16, u16)>) {
    if !is_tty {
        return (read_line_stdin(), 13, None);
    }
```

Add a terminator accumulator before the loop (next to `let mut buf = String::new();`):

```rust
    let mut terminator: u8 = 13;
```

Replace the `KeyCode::Enter => break,` arm and the final `// Arrows, function keys, etc. are consumed` catch-all arm with a single catch-all that handles both Enter and terminating function keys (keep the existing Ctrl-C/D, `KeyCode::Char(c)`, and `KeyCode::Backspace` arms exactly as they are, in that order, before this one):

```rust
                other => {
                    if let Some(t) = line_terminator(other, &is_term) {
                        terminator = t;
                        break;
                    }
                    // Non-terminating special key: consume (no on-screen garbage).
                }
```

Change the final return from `(buf, last_resize)` to:

```rust
    (buf, terminator, last_resize)
```

- [ ] **Step 6: Wire the NeedLine handler to pass the terminator**

In `crates/zvm-cli/src/main.rs`, replace the NeedLine call (currently `let (line, resize) = read_line_raw(stdin_is_tty, echo);` … `machine.supply_line(line.trim_end(), 13);`) with:

```rust
                let (line, terminator, resize) =
                    read_line_raw(stdin_is_tty, echo, |z| machine.is_terminator(z));
                if let Some((new_cols, new_rows)) = resize {
                    apply_resize(new_rows, new_cols, &mut term_rows, &mut term_cols,
                                 &mut page_height, &mut machine, &mut view);
                }
                machine.supply_line(line.trim_end(), terminator);
```

(The closure borrows `machine` immutably and is dropped before the `supply_line` mutable borrow; `echo` was already computed into an owned value above.)

- [ ] **Step 7: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 8: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit -F - <<'EOF'
feat(zvm-cli): end line input on the game's terminating keys

The raw-mode line editor now consults is_terminator: a function key the
game lists in its 0x2E table ends the line and is stored as the terminator
(e.g. Beyond Zork's cursor-key hint menu). Enter still terminates with 13.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 4: app — session terminated-submit API

**Files:**
- Modify: `crates/app/src/engine.rs` (`Engine` trait — add two defaulted methods)
- Modify: `crates/app/src/session.rs` (`GameSession` inherent methods + `impl Engine for GameSession` overrides; tests)
- Test: `crates/app/src/session.rs` (test module)

**Interfaces:**
- Consumes: `supply_line(input, terminator)` and `is_terminator` from Task 1; existing private `GameSession::key_input_to_zscii`.
- Produces:
  - `Engine::submit_line_terminated(&mut self, command: &str, terminator: u8) -> TurnResult` (default: `self.submit(command)`).
  - `Engine::line_terminator_for(&self, key: KeyInput) -> Option<u8>` (default: `None`).
  - `GameSession` overrides both; `line_terminator_for` returns `Some(zscii)` when `key` is a non-editing key whose ZSCII satisfies `is_terminator`.

- [ ] **Step 1: Write the failing tests**

First add a line-read story helper and tests to the `session.rs` test module (model the header on the existing `read_char_story_v5`):

```rust
    /// Build a minimal v5 story that suspends on a `read` (aread) whose store var
    /// is G0, with a terminating-characters table at 0x0200 listing 255 (any
    /// function key ends line input). GameSession::new stops at NeedLine.
    fn read_line_story_v5_any_terminator() -> Vec<u8> {
        let mut buf = read_char_story_v5(); // reuse header field setup
        // Terminating-characters table pointer (header 0x2E) → 0x0200 = [255, 0].
        buf[0x2E] = 0x02; buf[0x2F] = 0x00;
        buf[0x0200] = 255; buf[0x0201] = 0;
        // Text/parse buffers.
        buf[0x0250] = 20; // text buffer max chars
        buf[0x0260] = 8;  // parse buffer max tokens
        // Program at 0x0040: aread text_buf parse_buf -> G0 ; quit.
        buf[0x0040] = 0xE4; // VAR aread (read)
        buf[0x0041] = 0x0F; // types: large(00), large(00), omit(11), omit(11)
        buf[0x0042] = 0x02; buf[0x0043] = 0x50; // text_buf = 0x0250
        buf[0x0044] = 0x02; buf[0x0045] = 0x60; // parse_buf = 0x0260
        buf[0x0046] = 0x10; // store → G0
        buf[0x0047] = 0xBA; // quit
        buf
    }

    #[test]
    fn line_terminator_for_function_key_in_table() {
        let session = GameSession::new(read_line_story_v5_any_terminator(), true).unwrap();
        assert_eq!(session.pending_input(), InputKind::Line);
        // Table lists 255 (any function key) → Up (ZSCII 129) terminates.
        assert_eq!(session.line_terminator_for(KeyInput::Up), Some(129));
        // Editing keys never terminate a line.
        assert_eq!(session.line_terminator_for(KeyInput::Enter), None);
        assert_eq!(session.line_terminator_for(KeyInput::Backspace), None);
        assert_eq!(session.line_terminator_for(KeyInput::Char('a')), None);
    }

    #[test]
    fn submit_line_terminated_advances_the_turn() {
        let mut session = GameSession::new(read_line_story_v5_any_terminator(), true).unwrap();
        // read → quit, so a terminated submit drives the machine to Quit.
        let result = session.submit_line_terminated("look", 129);
        assert!(result.quit, "submit_line_terminated should advance read → quit");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app line_terminator_for 2>&1 | tail -20`
Expected: FAIL — `line_terminator_for` / `submit_line_terminated` not found on `GameSession`.

- [ ] **Step 3: Add the two defaulted methods to the `Engine` trait**

In `crates/app/src/engine.rs`, inside the `Engine` trait (next to `submit` / `submit_key` / `pending_input`), add:

```rust
    /// Submit a line terminated by ZSCII `terminator` (a v5+ terminating key,
    /// per the game's 0x2E table). Default ignores the terminator — engines
    /// without the concept behave exactly like `submit`.
    fn submit_line_terminated(&mut self, command: &str, _terminator: u8) -> TurnResult {
        self.submit(command)
    }

    /// If `key` ends line input for the current read (the game's terminating-
    /// characters table), return its ZSCII code; otherwise None. Default: an
    /// engine with no terminating-keys concept never terminates on a key.
    fn line_terminator_for(&self, _key: KeyInput) -> Option<u8> {
        None
    }
```

(Glulx's `GlulxSession` and the test-only mock `Engine` impls inherit the defaults — no changes needed there.)

- [ ] **Step 4: Add the `GameSession` inherent methods and trait overrides**

In `crates/app/src/session.rs`, in the inherent `impl GameSession` block (next to `submit` / `submit_char`), add:

```rust
    /// Supply a player command terminated by ZSCII `terminator` (a v5+ function
    /// key that ends line input — e.g. a cursor key in a menu read), step until
    /// the next input request or Quit, and return the turn result.
    pub fn submit_line_terminated(&mut self, command: &str, terminator: u8) -> TurnResult {
        self.machine.supply_line(command, terminator);
        let (stop, v3) = run_until_input(&mut self.machine);
        self.finish_turn(stop, v3)
    }

    /// If `key` is a non-editing key whose ZSCII code the game lists as a line
    /// terminator (header 0x2E table; ZMSD §10.7), return that code. Enter,
    /// Backspace, and printable characters are editing keys and never terminate.
    pub fn line_terminator_for(&self, key: KeyInput) -> Option<u8> {
        let z = GameSession::key_input_to_zscii(key)?;
        match key {
            KeyInput::Enter | KeyInput::Backspace | KeyInput::Char(_) => None,
            _ if self.machine.is_terminator(z as u16) => Some(z),
            _ => None,
        }
    }
```

In `impl Engine for GameSession`, add overrides that delegate to the inherent methods (matching the existing `submit`/`submit_char` delegation pattern):

```rust
    fn submit_line_terminated(&mut self, command: &str, terminator: u8) -> TurnResult {
        self.submit_line_terminated(command, terminator)
    }

    fn line_terminator_for(&self, key: KeyInput) -> Option<u8> {
        self.line_terminator_for(key)
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app line_terminator_for 2>&1 | tail -20`
Expected: PASS (both new tests).

- [ ] **Step 6: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/engine.rs crates/app/src/session.rs
git commit -F - <<'EOF'
feat(app): session API for terminating-key line submits

Adds Engine::line_terminator_for and Engine::submit_line_terminated
(Glulx-safe defaults) with GameSession overrides, so the host can detect a
function key that ends a v5+ line read and submit the buffer terminated by
its ZSCII code.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 5: app — line-terminator gate in the run loop

**Files:**
- Modify: `crates/app/src/state.rs` (add `pending_terminator` field + init)
- Modify: `crates/app/src/main.rs` (`line_terminator_action` free fn; gate before action routing at ~2179; `SubmitCommand` handler at ~2405)
- Test: `crates/app/src/main.rs` (test module — pure-fn test of the gate decision is covered by Task 4's `line_terminator_for`; here we test the empty-buffer submit rule)

**Interfaces:**
- Consumes: `Engine::line_terminator_for` and `Engine::submit_line_terminated` from Task 4; `app::engine::key_event_to_input`; `AppState::{char_mode, focus, any_overlay_open, input, take_input}`.
- Produces: `AppState.pending_terminator: Option<u8>`. Behavior: when a v5+ line read is pending and a terminating function key is pressed, the current input buffer is submitted terminated by that key — including an empty buffer (menu keypress).

- [ ] **Step 1: Add the `pending_terminator` field**

In `crates/app/src/state.rs`, add to the `AppState` struct (near `input`):

```rust
    /// Set by the run loop when a v5+ terminating key (game's 0x2E table) ends
    /// line input; consumed by the SubmitCommand handler to submit the current
    /// buffer terminated by that key. `None` for ordinary Enter submits.
    pub pending_terminator: Option<u8>,
```

Initialize it to `None` wherever `AppState` is constructed (the `Default` derive or the explicit constructor). If `AppState` derives `Default`, `Option<u8>` defaults to `None` automatically — no init change needed; verify by building.

- [ ] **Step 2: Write the failing test for the empty-buffer submit rule**

The gate itself needs a live TTY, but the SubmitCommand empty-buffer rule is unit-testable. Add to the `main.rs` test module a test of the guard predicate. First, extract the guard into a tiny pure helper next to `line_terminator_action` (Step 4 adds the fn; the test references it):

```rust
    #[test]
    fn terminated_submit_allows_empty_buffer() {
        // A terminating keypress with an empty input line is a valid submit
        // (menu navigation); a plain empty Enter is not.
        assert!(should_submit_line("", Some(129)));
        assert!(!should_submit_line("", None));
        assert!(should_submit_line("look", None));
    }
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p app terminated_submit_allows_empty_buffer 2>&1 | tail -20`
Expected: FAIL — `should_submit_line` not defined.

- [ ] **Step 4: Add `should_submit_line` and `line_terminator_action`**

In `crates/app/src/main.rs`, add two free functions (near the other run-loop helpers):

```rust
/// Whether a line submit should proceed: a non-empty buffer always submits; an
/// empty buffer submits only when a terminating key drove it (menu keypress).
fn should_submit_line(cmd: &str, terminator: Option<u8>) -> bool {
    !cmd.is_empty() || terminator.is_some()
}

/// If a v5+ line read is pending and `event` is a function key the game lists as
/// a line terminator, record the terminator on `state` and return the
/// `SubmitCommand` action that submits the current buffer. Returns None
/// otherwise, so ordinary keys fall through to normal routing.
fn line_terminator_action(
    state: &mut AppState,
    session: &dyn app::engine::Engine,
    event: &Event,
) -> Option<Action> {
    use crossterm::event::{KeyEventKind, KeyModifiers};
    if state.char_mode || state.focus != Focus::Game || state.any_overlay_open() {
        return None;
    }
    let Event::Key(k) = event else { return None };
    if k.kind != KeyEventKind::Press
        || k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let term = app::engine::key_event_to_input(*k)
        .and_then(|ki| session.line_terminator_for(ki))?;
    state.pending_terminator = Some(term);
    Some(Action::SubmitCommand(state.input.clone()))
}
```

(Import `Focus` and `Action` as they are already imported in `main.rs`.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p app terminated_submit_allows_empty_buffer 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Insert the gate before action routing**

In `crates/app/src/main.rs`, change the action routing (currently `let action = match event { … };` at ~2180) to consult the gate first:

```rust
        // Line-input terminator: a v5+ game may list function keys in its
        // terminating-characters table (ZMSD §10.7); such a key submits the
        // current buffer terminated by its ZSCII code (e.g. Beyond Zork's
        // cursor-key hint menu). Reuses the SubmitCommand path via
        // state.pending_terminator so all turn bookkeeping stays in one place.
        let action = if let Some(a) = line_terminator_action(&mut state, &*session, &event) {
            a
        } else {
            match event {
                // ... existing arms unchanged ...
            }
        };
```

(Wrap the existing `match event { … }` verbatim as the `else` branch. Its inner `continue 'event_loop` / `break` still compile in value position, exactly as before.)

- [ ] **Step 7: Handle the terminator in the `SubmitCommand` handler**

In the `Action::SubmitCommand(cmd) =>` arm (the normal game path, after the `state.prompt.is_some()` block), change the input-taking and submit lines. Replace:

```rust
                let cmd = state.take_input();
                if cmd.is_empty() {
                    continue;
                }

                // Record into the shell-style command history ...
                state.record_command(&cmd);
```

with:

```rust
                let cmd = state.take_input();
                let terminator = state.pending_terminator.take();
                if !should_submit_line(&cmd, terminator) {
                    continue;
                }

                // Record into the shell-style command history (skip empty
                // terminating-key menu submits).
                if !cmd.is_empty() {
                    state.record_command(&cmd);
                }
```

Guard the slash interception so a terminating-key submit is never treated as a slash command (replace `if is_slash(&cmd, state.config.command_prefix) {` with):

```rust
                if terminator.is_none() && is_slash(&cmd, state.config.command_prefix) {
```

Replace the submit call `let result = session.submit(&cmd);` with:

```rust
                let result = match terminator {
                    Some(t) => session.submit_line_terminated(&cmd, t),
                    None => session.submit(&cmd),
                };
```

Guard the `> cmd` echo so empty menu submits don't print a blank prompt line (replace `state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);` with):

```rust
                if !cmd.is_empty() {
                    state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
                }
```

- [ ] **Step 8: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 9: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/main.rs
git commit -F - <<'EOF'
feat(app): submit line input on terminating function keys

When a v5+ line read is pending and the player presses a key the game
lists in its terminating-characters table, the run loop submits the
current buffer terminated by that key (empty buffer included, for menu
navigation) via the existing SubmitCommand path. Beyond Zork's cursor-key
hint menu now works in the app, matching zvm-cli.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Manual verification (after all tasks)

Not automatable in the unit suite (needs a real TTY + game). Run once by hand and note the result:

- `cargo run -p zvm-cli -- <BeyondZork.z5>` → open the hint menu; cursor keys navigate (line read terminated by 129/130); no arrow-key garbage.
- `cargo run -p app -- <BeyondZork.z5>` → same hint menu; cursor keys navigate the menu instead of recalling command history.

If a game file is unavailable, state that the manual check was deferred.

---

## Self-Review Notes

- **Spec coverage:** Component 1 (terminator API) → Task 1. Component 2 (function-key range) → Task 2 (zvm-cli); app already covers F1–F12 via `key_event_to_input`/`key_input_to_zscii`, confirmed, so no app task needed. Component 3 (line termination, full parity) → Task 3 (zvm-cli) + Tasks 4–5 (app). Component 4 (ZSCII 8/27 verification) → Task 1 (engine store test) plus the pre-existing host mapping tests (`decode_keycode` 8/27 in zvm-cli; `key_input_to_zscii` 8/27 in app), noted here so the reviewer confirms they exist rather than re-adding them.
- **Type consistency:** `supply_line(&str, u8)`, `read_line_raw(..) -> (String, u8, Option<(u16,u16)>)`, `line_terminator(KeyCode, impl Fn(u16)->bool) -> Option<u8>`, `Engine::{submit_line_terminated(&mut,&str,u8)->TurnResult, line_terminator_for(&self,KeyInput)->Option<u8>}`, `AppState.pending_terminator: Option<u8>` are used consistently across tasks.
- **Keypad 145–154:** intentionally not implemented (unreachable in terminal hosts); documented at the decode site (Task 2).
