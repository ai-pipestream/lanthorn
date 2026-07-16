# gvm-cli raw-mode gated echo (fix double-echo on self-echoing games) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Stop `gvm-cli` double-echoing input on a TTY for self-echoing Glulx games, by reading input in raw mode with a manual, style-aware echo gated on the game's Glk echo-line flag.

**Architecture:** Today `gvm-cli` reads line input in cooked mode (`read_line_stdin`), so the terminal always echoes keystrokes; gvm never echoes into its window and treats `glk_set_echo_line_event` as a no-op. A game that disables Glk echo and prints the command itself then shows it twice on a TTY. Fix: (1) gvm tracks the per-window echo-line flag and exposes a query + the window's Input-style colour; (2) `gvm-cli` reads in raw mode and echoes manually — but only when the window's echo-line flag is on — mirroring the existing `zvm-cli::read_line_raw`.

**Tech Stack:** Rust. gvm library stays zero-dep. gvm-cli already uses crossterm (`terminal::enable_raw_mode`, `event::read`).

## Global Constraints

- `gvm` **library** crate stays ZERO external dependencies (only `std`). gvm-cli may use its existing `crossterm` dep.
- Cross-platform: input handling must use `crossterm` (no `stty`/termios), per the repo's cross-platform requirement.
- Do NOT bump `GLK_SNAPSHOT_VERSION` (currently 4). The new `echo_line` flag is transient presentation state and is NOT serialized — `deserialize` resets it to its default `true`, exactly as it already does for `terminators`.
- Echo semantics on a scrolling terminal: the typed echo is GATED on the window's echo-line flag (echo-on → gvm-cli echoes; echo-off → gvm-cli does not, the game will). This intentionally differs from a redrawable-window library (which shows typing then erases post-Enter) because a scrolling terminal cannot retract already-typed text.
- Staging hygiene: the tree has pre-existing untracked files (`docs/mapping-*.md`, `docs/superpowers/plans/2026-07-*.md`, `tests/`, `ui.txt`). Stage ONLY the edited source files by path — never `git add -A`.
- Commit trailers on every commit:
  ```
  Quest: SQ-0275
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

## File Structure

- `crates/gvm/src/glk.rs` — add `Window::echo_line`, a setter, a query, and a window Input-colour resolver on `Model`.
- `crates/gvm/src/exec.rs` — wire the `0x0150` opcode to the setter; expose `Machine::window_line_echo` + `Machine::window_input_colour`.
- `crates/gvm-cli/src/glk_term.rs` — expose a `pub fn sgr_input(colour, honor)` wrapper over the private `sgr_open`.
- `crates/gvm-cli/src/main.rs` — add `read_line_raw`; thread `honor` + an echo argument through `drive()`; wire real + test call sites.

---

### Task 1: gvm — per-window echo-line flag, query, and Input-colour resolver

**Files:**
- Modify: `crates/gvm/src/glk.rs`
- Modify: `crates/gvm/src/exec.rs`
- Test: inline `#[cfg(test)]` in `crates/gvm/src/exec.rs` (mirror the existing `glk_set_echo_and_terminators_accepted_silently` test's `glk_call` harness).

**Interfaces:**
- Produces (consumed by Task 2):
  - `Machine::window_line_echo(&self, win: u32) -> bool` — the window's echo-line flag (default `true`; unknown window → `true`).
  - `Machine::window_input_colour(&self, win: u32) -> gvm::glk::StyleColour` — the resolved colour for `GlkStyle::Input` in that window's wintype.

- [ ] **Step 1: Add the field + Model methods (glk.rs)**

In `struct Window` (near the `terminators: Vec<u32>` field, ~line 710) add:
```rust
    /// Whether completed line input is echoed (`glk_set_echo_line_event`).
    /// Default true (Glk spec §4.2). Hosts that echo typed input consult this;
    /// a game that turns it off takes responsibility for echoing itself.
    echo_line: bool,
```
In the window-creation constructor (~line 944, the `self.windows.push(Some(Window { ... }))` with `terminators: Vec::new(),`) add `echo_line: true,`.
In the `deserialize` constructor (~line 2131-2133, the `Window { ... terminators: Vec::new(), ... }`) add `echo_line: true,` (NOT read from the blob — reset to default, exactly like `terminators`). Do NOT change `serialize` and do NOT change `GLK_SNAPSHOT_VERSION`.

Add these `Model` methods (mirror `set_line_terminators`/`is_line_terminator` at ~1903/1915, using the same `win_mut`/`win` accessors those use):
```rust
    /// Set the window's line-echo flag (`glk_set_echo_line_event`). No-op for an
    /// invalid window id.
    pub fn set_window_echo_line(&mut self, win: u32, on: bool) {
        if let Some(w) = self.win_mut(win) {
            w.echo_line = on;
        }
    }

    /// The window's line-echo flag (default true; unknown window → true).
    pub fn window_echo_line(&self, win: u32) -> bool {
        self.win(win).map(|w| w.echo_line).unwrap_or(true)
    }

    /// The resolved colour for `GlkStyle::Input` in this window (its wintype's
    /// Input style hint); falls back to a text-buffer Input colour for an unknown
    /// window. Used by scrolling-terminal hosts to colour the input echo.
    pub fn window_input_colour(&self, win: u32) -> StyleColour {
        let wt = self.win(win).map(|w| w.wintype).unwrap_or(WinType::TextBuffer);
        self.style_colour(wt, GlkStyle::Input)
    }
```
(If the accessor used by `set_line_terminators` is not literally named `win_mut`, use whatever that method uses — match the existing code.)

- [ ] **Step 2: Wire the opcode + Machine queries (exec.rs)**

Replace the `0x0150` arm (currently `0x0150 => 0, // glk_set_echo_line_event: best-effort no-op`) with:
```rust
            0x0150 => {
                // glk_set_echo_line_event(win, val): record the window's echo flag
                // so scrolling-terminal hosts can avoid double-echoing a game that
                // echoes its own input line.
                self.glk.set_window_echo_line(a(0), a(1) != 0);
                0
            }
```
Add public queries on `Machine` (near other Glk-facing accessors; `StyleColour` and `WinType`/`GlkStyle` are already in scope via the `glk` module — re-export or fully-qualify as the surrounding code does):
```rust
    /// The line-echo flag of window `win` (`glk_set_echo_line_event`; default true).
    pub fn window_line_echo(&self, win: u32) -> bool {
        self.glk.window_echo_line(win)
    }

    /// The resolved `GlkStyle::Input` colour for window `win` (for host input echo).
    pub fn window_input_colour(&self, win: u32) -> crate::glk::StyleColour {
        self.glk.window_input_colour(win)
    }
```
(Confirm `StyleColour` is `pub` in `glk.rs`; it is already used across the backend trait. If `Machine`'s existing methods reference these types by a shorter path, match that.)

- [ ] **Step 3: Write the failing test (exec.rs `#[cfg(test)]`)**

Mirror `glk_set_echo_and_terminators_accepted_silently`'s `glk_call` construction:
```rust
    #[test]
    fn glk_set_echo_line_event_toggles_the_window_flag() {
        use asm::Op::{C8, Zero};
        // A fresh window defaults to echo-on.
        let mut on = glk_call(0x150, &[C8(1), C8(0)], Zero); // set_echo_line_event(win=1, 0)
        on.extend(asm::ins(0x120, &[]));                     // quit
        let m = run_program(on);
        assert!(!m.window_line_echo(1), "echo turned off for window 1");

        // A window that never set the flag reports the default (true).
        let base = run_program(asm::ins(0x120, &[]));
        assert!(base.window_line_echo(1), "default echo is on");
    }
```
(Use whatever window-1 opening the `run_program` harness already provides; the existing echo/terminators test calls `glk_call(0x150, ...)` on window `1` without opening a window explicitly, so window 1 must be the harness's default root — match that. If `run_program` does NOT open a root window, prepend the same window-open the sibling test uses. Verify against the actual harness before finalizing.)

- [ ] **Step 4: Run to red, then green**

```
cargo test -p gvm glk_set_echo_line_event_toggles_the_window_flag
```
Red first (method/field missing), then green after Steps 1-2. Also confirm the existing `glk_set_echo_and_terminators_accepted_silently` still passes (it must remain diagnostic-free).

- [ ] **Step 5: Full gvm suite + commit**

```
cargo test -p gvm
```
Expected: all green (baseline was 31 passing + the new one; snapshot round-trip tests unaffected since `echo_line` isn't serialized). Then:
```
git add crates/gvm/src/glk.rs crates/gvm/src/exec.rs
git commit -F <msgfile>
```
Subject: `feat(gvm): track glk_set_echo_line_event per window + expose input colour (SQ-0275)`

---

### Task 2: gvm-cli — raw-mode gated, coloured input echo

**Files:**
- Modify: `crates/gvm-cli/src/glk_term.rs` (expose `sgr_input`)
- Modify: `crates/gvm-cli/src/main.rs` (`read_line_raw`, `drive` signature, call sites)

**Interfaces:**
- Consumes from Task 1: `Machine::window_line_echo(win)`, `Machine::window_input_colour(win)`.
- Consumes existing gvm-cli: `read_line_stdin()`, crossterm `terminal`/`event`/`Event`/`KeyEvent`/`KeyCode`/`KeyModifiers` (already imported for `read_char_input`).

- [ ] **Step 1: Expose the input SGR (glk_term.rs)**

The colour helpers `sgr_open`/`sgr_set` are private. Add (right after `sgr_open`):
```rust
/// Opening SGR for the Input style + resolved colour, for a host echoing typed
/// input (mirrors zvm-cli drawing its echo in the game's input style/colour).
pub fn sgr_input(colour: StyleColour, honor: bool) -> String {
    sgr_open(GlkStyle::Input, colour, honor)
}
```
(`StyleColour` and `GlkStyle` are already imported in this file — confirm; if not, add the `use`.)

- [ ] **Step 2: Add `read_line_raw` (main.rs)**

Model on `zvm-cli::read_line_raw` but simplified (no timeout/sound/view/frame). Place it next to `read_line_stdin`:
```rust
/// Read a line of input in RAW mode, echoing typed characters manually. `echo`
/// is `Some(sgr)` to echo (the SGR prefix colours it; empty string = plain echo)
/// or `None` to read WITHOUT echoing — used when the game has disabled Glk line
/// echo (`glk_set_echo_line_event(win, 0)`), so a self-echoing game is not shown
/// twice on a scrolling terminal. Falls back to cooked line input on non-TTY
/// stdin (piped input has no terminal echo, so it is already correct).
/// The terminator is always 0 (normal Enter), matching the prior cooked path.
fn read_line_raw(is_tty: bool, echo: Option<&str>) -> (String, u32) {
    if !is_tty {
        return read_line_stdin(); // (String, 0)
    }
    let echoing = echo.is_some();
    let sgr = echo.unwrap_or("");
    let _ = terminal::enable_raw_mode();
    let mut buf = String::new();
    if echoing && !sgr.is_empty() {
        print!("{sgr}");
        let _ = io::Write::flush(&mut io::stdout());
    }
    loop {
        match event::read() {
            Ok(Event::Key(KeyEvent { code, modifiers, .. })) => match code {
                KeyCode::Enter => break,
                // Raw mode swallows signals; exit cleanly on Ctrl-C / Ctrl-D.
                KeyCode::Char('c') | KeyCode::Char('d')
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if echoing && !sgr.is_empty() { print!("\x1b[0m"); }
                    print!("\r\n");
                    let _ = io::Write::flush(&mut io::stdout());
                    let _ = terminal::disable_raw_mode();
                    std::process::exit(0);
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    if echoing {
                        print!("{c}");
                        let _ = io::Write::flush(&mut io::stdout());
                    }
                }
                KeyCode::Backspace => {
                    if buf.pop().is_some() && echoing {
                        print!("\x08 \x08");
                        let _ = io::Write::flush(&mut io::stdout());
                    }
                }
                _ => {} // other special keys consumed (no on-screen garbage)
            },
            Ok(Event::Resize(..)) => {} // caught by next before_input size poll
            _ => {}
        }
    }
    if echoing && !sgr.is_empty() { print!("\x1b[0m"); }
    let _ = terminal::disable_raw_mode();
    print!("\r\n"); // raw mode does not translate Enter to CRLF
    let _ = io::Write::flush(&mut io::stdout());
    (buf, 0)
}
```
(Match the file's existing flush idiom — it uses `io::stdout().flush()` with `use std::io::Write` or `io::Write::flush`. Inspect the top of `main.rs` and use the same form.)

- [ ] **Step 3: Thread echo through `drive()` (main.rs)**

Add a `honor: bool` parameter to `drive(...)` (place it before the closures). Change the `read_line` closure param type from `impl FnMut() -> (String, u32)` to `impl FnMut(Option<&str>) -> (String, u32)`.

In the `NeedLine { win }` arm, replace:
```rust
            StepResult::NeedLine { .. } => {
                before_input(machine);
                let (line, terminator) = read_line();
                machine.supply_line_terminated(line.trim_end_matches(['\n', '\r']), terminator);
            }
```
with:
```rust
            StepResult::NeedLine { win } => {
                before_input(machine);
                // Echo typed input ourselves (raw mode), UNLESS the game disabled
                // Glk line echo — then it echoes its own command and we must not,
                // or a scrolling terminal shows it twice (SQ-0275).
                let echo: Option<String> = if machine.window_line_echo(win) {
                    Some(if honor {
                        glk_term::sgr_input(machine.window_input_colour(win), true)
                    } else {
                        String::new()
                    })
                } else {
                    None
                };
                let (line, terminator) = read_line(echo.as_deref());
                machine.supply_line_terminated(line.trim_end_matches(['\n', '\r']), terminator);
            }
```
In the `NeedFilename` arm, the filename prompt must stay visible, so echo it plainly: change `let (line, _) = read_line();` to `let (line, _) = read_line(Some(""));`.

(Reference `glk_term` by the path the file already uses — check the existing `use`/module reference for the terminal backend, e.g. `crate::glk_term::` or `glk_term::`.)

- [ ] **Step 4: Wire the real call site + tests (main.rs)**

Real driver (~line 259) `drive(&mut machine, &save_path, &vfs_path, before_input, read_line_stdin, move || read_char_input(stdin_is_tty))`: pass `honor` and swap the cooked reader for the raw one:
```rust
    drive(
        &mut machine,
        &save_path,
        &vfs_path,
        honor,
        before_input,          // (existing before_input closure)
        move |echo| read_line_raw(stdin_is_tty, echo),
        move || read_char_input(stdin_is_tty),
    );
```
(Keep the existing `before_input`/save/vfs arguments exactly as they are; only add `honor` and replace the read_line argument. `honor` is already computed in `main()`.)

Update the four test call sites (~554, 578, 681, 694): insert `true` for `honor` and give the read_line closures the new `Option<&str>` arg (ignored):
- `|| (String::new(), 0)` → `|_echo| (String::new(), 0)`
- `move || (lines.next().unwrap_or_default(), 0)` → `move |_echo| (lines.next().unwrap_or_default(), 0)`

- [ ] **Step 5: Build, test, and reason about coverage**

```
cargo test -p gvm-cli
cargo build -p gvm-cli
```
Expected: green (the existing drive tests exercise the piped/non-echo path through the new closure signature). The raw-TTY editor itself (real-key echo, backspace, colour) is NOT headlessly testable — it is verified by the manual smoke below. Do NOT add a vacuous test that pretends to cover it.

- [ ] **Step 6: Commit**

```
git add crates/gvm-cli/src/glk_term.rs crates/gvm-cli/src/main.rs
git commit -F <msgfile>
```
Subject: `fix(gvm-cli): raw-mode gated echo so self-echoing games don't double (SQ-0275)`

---

## Manual smoke (records the TTY behaviour the tests can't)

On a real terminal (`cargo run -p gvm-cli -- <story.gblorb>`):
1. A standard Glk game (library echo on): typed commands appear **once**, coloured in the game's input style; game responses render normally.
2. `--no-game-colours`: typed input still echoes once, uncoloured.
3. A self-echoing game that disables Glk echo (e.g. Counterfeit Monkey, if it does): the command appears **once** (only the game's own echo), not twice.
4. Piped input (`printf 'look\nquit\n' | cargo run -p gvm-cli -- <story>`): unchanged, no crash.
5. Backspace during entry erases one char on screen; Ctrl-C/Ctrl-D exits cleanly (terminal restored, no stuck raw mode).

## Self-review checklist

- No snapshot-version bump; `echo_line` not serialized; `deserialize` sets it `true` like `terminators`. Snapshot round-trip tests stay green.
- gvm library gains no external deps.
- The double-echo fix is gated on the *game's* flag, not unconditional — standard games keep their single echo.
- Filename prompt still echoes (passes `Some("")`).
- Only the four edited source files are staged per commit; no `git add -A`.
