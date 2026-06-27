# zvm-cli DOS Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `zvm-cli` a basic DOS-equivalent interpreter — render the Z-machine upper window (v4+) and v3 status line, ring the built-in bleeps, accept single-key `read_char`, style the lower window, and persist v5 aux tables across sessions.

**Architecture:** Almost entirely a `zvm-cli` frontend addition (new `screen.rs` + `aux.rs` modules, changes to `main.rs`). The only engine change is one additive, backward-compatible default method on the `zvm::io::Output` trait. A pinned ANSI top region is used when stdout is a TTY; a deduped inline plain-text block when piped (so the headless harness stays clean); `--no-status` restores byte-identical legacy output; `--no-aux` disables aux persistence.

**Tech Stack:** Rust, std only (zero new dependencies). ANSI escape sequences are plain bytes; TTY detection via `std::io::IsTerminal`; terminal size and raw single-key input via `stty` shelled out with `std::process::Command`.

**Specs:**
- `docs/superpowers/specs/2026-06-27-zvm-cli-screen-model-design.md`
- `docs/superpowers/specs/2026-06-27-zvm-cli-aux-persistence-design.md`

## Global Constraints

- 0 warnings (`cargo build`, `cargo doc --no-deps`) and full `cargo test --workspace` green after every task.
- `zvm-cli` stays zero-dependency (std only). No new crates anywhere.
- Exactly one engine change: the `Output::print_styled` default method (Task 1). It MUST leave every existing sink (`BufferOutput`, the app's `CaptureSink`) byte-for-byte unchanged — verified by the existing suite staying green plus a new default-delegation test.
- Game-visible header dims stay fixed 80×24 (today's `init_caps` default); the real terminal row count is used ONLY for the cosmetic TTY scroll region.
- Default piped output equals today's lower-window stream plus the deduped inline status/upper block; with `--no-status` it is byte-for-byte identical to today. `--no-aux` writes no files.
- Commit-only on local `main`; one commit per task (TDD). No push.
- Commit trailers on every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Do not edit `TODO.md`.

## Reference: engine types the frontend reads (no changes to these)

- `machine.mem.version() -> u8` (v3 = `< 4`).
- `machine.screen: ScreenState` with `upper: UpperWindow`, `upper_window_rows: u16`, `current_window: u8`, `cursor_row/cursor_col: u16`, `text_style: u8`, `show_status_requested: bool`.
- `UpperWindow { cols: u16, rows: u16, .. }`, `cell(row, col) -> Cell` (1-based), `Cell { ch: char, style: u8 }`.
- `machine.status_line() -> zvm::screen::StatusLine { location: String, right: StatusRight }`, `StatusRight = ScoreTurns { score: i16, turns: u16 } | Time { hours: u8, minutes: u8 }`.
- `machine.pending_beeps: Vec<zvm::cpu::exec::Beep>` (`Beep::High`, `Beep::Low`); drain after each step.
- Style bits (ZMSD §8.7.1): `1` reverse, `2` bold, `4` italic, `8` fixed-pitch.

---

## File Structure

- `crates/zvm/src/io.rs` — add `Output::print_styled` default method (Task 1).
- `crates/zvm/src/cpu/exec.rs` — one call-site change in `print_text` (Task 1).
- `crates/zvm-cli/src/screen.rs` — NEW. Pure formatting/SGR/term helpers (Task 2) + the stateful `ScreenView` (Task 3).
- `crates/zvm-cli/src/aux.rs` — NEW. Aux codec + path + preload/flush helpers (Task 5).
- `crates/zvm-cli/src/main.rs` — `StdoutOutput` becomes style-aware (Task 4); arg parsing + loop integration + raw single-key + aux wiring (Task 6); `mod screen; mod aux;`.

---

## Task 1: Engine seam — `Output::print_styled`

**Files:**
- Modify: `crates/zvm/src/io.rs` (trait `Output`)
- Modify: `crates/zvm/src/cpu/exec.rs` (`print_text` lower-window branch)
- Test: `crates/zvm/src/io.rs` (`#[cfg(test)] mod`)

**Interfaces:**
- Produces: `Output::print_styled(&mut self, s: &str, style: u8)` with a default body delegating to `print`. The lower-window text path now carries `screen.text_style` to the sink.

- [ ] **Step 1: Write the failing test** (in `crates/zvm/src/io.rs` tests)

```rust
#[cfg(test)]
mod print_styled_tests {
    use super::*;

    #[test]
    fn default_print_styled_delegates_to_print() {
        let mut a = BufferOutput::new();
        let mut b = BufferOutput::new();
        a.print("hello");
        b.print_styled("hello", 0x02); // style ignored by default impl
        assert_eq!(a.buf, b.buf, "default print_styled must equal print");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p zvm default_print_styled_delegates_to_print`
Expected: FAIL — `no method named print_styled`.

- [ ] **Step 3: Add the trait method (default impl)** in `crates/zvm/src/io.rs`

```rust
pub trait Output: Any {
    fn print(&mut self, s: &str);
    /// Print `s` carrying the current Z-machine text-style bitmask
    /// (ZMSD §8.7.1: 1=reverse, 2=bold, 4=italic, 8=fixed-pitch). The default
    /// ignores the style and delegates to `print`, so existing sinks are
    /// unaffected until they override this.
    fn print_styled(&mut self, s: &str, _style: u8) {
        self.print(s);
    }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

- [ ] **Step 4: Route lower-window text through it** in `crates/zvm/src/cpu/exec.rs` `print_text`

Change the final lower-window branch (currently `if self.streams.stream1 { self.out.print(s); }`) to:

```rust
        // Stream 3 is inactive; streams 1/2/4 apply.
        if self.streams.stream1 {
            self.out.print_styled(s, self.screen.text_style);
        }
```

(Leave the upper-window grid branch and the stream-3 early return exactly as they are.)

- [ ] **Step 5: Run tests** — new test plus the whole engine suite (proves no behavior change)

Run: `cargo test -p zvm`
Expected: PASS, including all pre-existing tests (the default delegation means output is unchanged).

- [ ] **Step 6: Commit**

```bash
git add crates/zvm/src/io.rs crates/zvm/src/cpu/exec.rs
git commit  # feat(zvm): additive Output::print_styled seam (default delegates to print)
```

---

## Task 2: zvm-cli pure render helpers (`screen.rs`)

**Files:**
- Create: `crates/zvm-cli/src/screen.rs`
- Modify: `crates/zvm-cli/src/main.rs` (add `mod screen;`)
- Test: `crates/zvm-cli/src/screen.rs` (`#[cfg(test)] mod`)

**Interfaces:**
- Produces (all pure, no I/O):
  - `pub const DEFAULT_COLS: u16 = 80;` `pub const DEFAULT_ROWS: u16 = 24;`
  - `pub fn sgr_set(style: u8) -> String` — SGR set-codes for the style bits (no reset, no codes for fixed-pitch); empty for `0`.
  - `pub fn style_wrap(s: &str, style: u8, is_tty: bool) -> String` — lower-window wrap.
  - `pub fn bleep_bytes(count: usize, is_tty: bool) -> String`.
  - `pub fn wants_raw_char(stdin_is_tty: bool) -> bool`.
  - `pub fn parse_stty_size(out: &str) -> Option<(u16, u16)>` (rows, cols).
  - `pub fn term_rows(stty_out: Option<&str>, env_lines: Option<&str>) -> u16`.
  - `pub fn status_text(st: &zvm::screen::StatusLine, cols: u16) -> String` (plain, padded to `cols`).
  - `pub fn upper_row_text(upper: &zvm::screen::UpperWindow, row: u16) -> String` (plain, right-trimmed).
  - `pub fn upper_row_ansi(upper: &zvm::screen::UpperWindow, row: u16) -> String` (per-cell SGR runs).
  - `pub fn enter_region(top_rows: u16, term_rows: u16) -> String` / `pub fn leave_region() -> String`.

- [ ] **Step 1: Add `mod screen;`** to `crates/zvm-cli/src/main.rs` (top, after the `use` block):

```rust
mod screen;
mod aux; // added in Task 5; declare now so the module tree is stable
```

Create `crates/zvm-cli/src/aux.rs` as an empty placeholder for now:

```rust
// Aux ("global state") persistence — implemented in Task 5.
```

- [ ] **Step 2: Write the failing tests** in `crates/zvm-cli/src/screen.rs`

```rust
use zvm::screen::{StatusLine, StatusRight, UpperWindow};

// ... (the pub fns above) ...

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight, UpperWindow};

    #[test]
    fn sgr_set_maps_bits() {
        assert_eq!(sgr_set(0), "");
        assert_eq!(sgr_set(1), "\x1b[7m");          // reverse
        assert_eq!(sgr_set(2), "\x1b[1m");          // bold
        assert_eq!(sgr_set(4), "\x1b[3m");          // italic
        assert_eq!(sgr_set(8), "");                 // fixed-pitch ignored
        assert_eq!(sgr_set(1 | 2), "\x1b[7m\x1b[1m");
    }

    #[test]
    fn style_wrap_only_when_tty_and_styled() {
        assert_eq!(style_wrap("hi", 0, true), "hi");
        assert_eq!(style_wrap("hi", 2, false), "hi");
        assert_eq!(style_wrap("hi", 2, true), "\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn bleep_bytes_tty_gated() {
        assert_eq!(bleep_bytes(3, true), "\x07\x07\x07");
        assert_eq!(bleep_bytes(3, false), "");
        assert_eq!(bleep_bytes(0, true), "");
    }

    #[test]
    fn parse_and_resolve_term_rows() {
        assert_eq!(parse_stty_size("24 80\n"), Some((24, 80)));
        assert_eq!(parse_stty_size("garbage"), None);
        assert_eq!(term_rows(Some("40 100"), None), 40);   // stty wins
        assert_eq!(term_rows(None, Some("50")), 50);       // env fallback
        assert_eq!(term_rows(None, None), DEFAULT_ROWS);   // default
        assert_eq!(term_rows(Some("bad"), Some("x")), DEFAULT_ROWS);
    }

    #[test]
    fn status_text_pads_and_right_aligns() {
        let st = StatusLine { location: "West of House".into(), right: StatusRight::ScoreTurns { score: 0, turns: 1 } };
        let row = status_text(&st, 40);
        assert_eq!(row.chars().count(), 40, "padded to width");
        assert!(row.starts_with(" West of House"), "location left: {row:?}");
        assert!(row.trim_end().ends_with("Moves: 1"), "right field: {row:?}");
    }

    #[test]
    fn status_text_truncates_long_location() {
        let st = StatusLine { location: "x".repeat(100), right: StatusRight::Time { hours: 9, minutes: 5 } };
        let row = status_text(&st, 20);
        assert_eq!(row.chars().count(), 20);
        assert!(row.contains("09:05"));
    }

    #[test]
    fn upper_row_text_and_ansi() {
        let mut u = UpperWindow::default();
        u.resize(1, 5);
        u.put(1, 1, 'H', 0);
        u.put(1, 2, 'i', 2); // bold
        let text = upper_row_text(&u, 1);
        assert_eq!(text, "Hi"); // trailing blanks trimmed
        let ansi = upper_row_ansi(&u, 1);
        assert!(ansi.contains("\x1b[1m") && ansi.ends_with("\x1b[0m"), "ansi: {ansi:?}");
    }

    #[test]
    fn region_strings() {
        assert_eq!(leave_region(), "\x1b[r");
        assert!(enter_region(1, 24).starts_with("\x1b[2;24r"));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p zvm-cli`
Expected: FAIL to compile (functions undefined).

- [ ] **Step 4: Implement the helpers** in `crates/zvm-cli/src/screen.rs`

```rust
//! Basic DOS-style screen model for zvm-cli: pure formatting/SGR/terminal
//! helpers (this module) plus the stateful `ScreenView` (Task 3).

use zvm::screen::{StatusLine, StatusRight, UpperWindow};

pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// SGR set-codes for a Z-machine text-style bitmask (no leading/trailing reset).
/// 1=reverse, 2=bold, 4=italic; 8 (fixed-pitch) has no terminal equivalent here.
pub fn sgr_set(style: u8) -> String {
    let mut s = String::new();
    if style & 0x01 != 0 { s.push_str("\x1b[7m"); }
    if style & 0x02 != 0 { s.push_str("\x1b[1m"); }
    if style & 0x04 != 0 { s.push_str("\x1b[3m"); }
    s
}

/// Wrap lower-window text in SGR when on a TTY and a style is set; else plain.
pub fn style_wrap(s: &str, style: u8, is_tty: bool) -> String {
    if !is_tty || style == 0 {
        return s.to_string();
    }
    format!("{}{}\x1b[0m", sgr_set(style), s)
}

/// Terminal BEL per bleep, TTY-gated.
pub fn bleep_bytes(count: usize, is_tty: bool) -> String {
    if is_tty { "\x07".repeat(count) } else { String::new() }
}

/// Raw single-key `read_char` only makes sense on a TTY stdin.
pub fn wants_raw_char(stdin_is_tty: bool) -> bool {
    stdin_is_tty
}

/// Parse `stty size` output ("rows cols").
pub fn parse_stty_size(out: &str) -> Option<(u16, u16)> {
    let mut it = out.split_whitespace();
    let rows = it.next()?.parse().ok()?;
    let cols = it.next()?.parse().ok()?;
    Some((rows, cols))
}

/// Resolve the terminal row count: stty size, then env LINES, then default.
pub fn term_rows(stty_out: Option<&str>, env_lines: Option<&str>) -> u16 {
    if let Some((rows, _)) = stty_out.and_then(parse_stty_size) {
        if rows > 0 { return rows; }
    }
    if let Some(n) = env_lines.and_then(|s| s.trim().parse::<u16>().ok()) {
        if n > 0 { return n; }
    }
    DEFAULT_ROWS
}

fn right_field(right: &StatusRight) -> String {
    match right {
        StatusRight::ScoreTurns { score, turns } => format!("Score: {score}  Moves: {turns}"),
        StatusRight::Time { hours, minutes } => format!("Time: {hours:02}:{minutes:02}"),
    }
}

/// Plain v3 status row: " <location> ... <right> ", padded to exactly `cols`.
pub fn status_text(st: &StatusLine, cols: u16) -> String {
    let cols = cols as usize;
    if cols < 2 {
        return " ".repeat(cols);
    }
    let inner = cols - 2; // one border space each side
    let right = right_field(&st.right);
    let right_w = right.chars().count().min(inner);
    let right: String = right.chars().take(right_w).collect();
    let left_max = inner - right_w;
    let left: String = st.location.chars().take(left_max).collect();
    let fill = inner - left.chars().count() - right_w;
    format!(" {}{}{} ", left, " ".repeat(fill), right)
}

/// Plain text of one upper-window row, trailing blanks trimmed.
pub fn upper_row_text(upper: &UpperWindow, row: u16) -> String {
    let mut s = String::new();
    for c in 1..=upper.cols {
        s.push(upper.cell(row, c).ch);
    }
    s.trim_end().to_string()
}

/// One upper-window row with per-cell SGR runs (for the pinned TTY region).
pub fn upper_row_ansi(upper: &UpperWindow, row: u16) -> String {
    let mut out = String::new();
    let mut cur = 0u8;
    for c in 1..=upper.cols {
        let cell = upper.cell(row, c);
        if cell.style != cur {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_set(cell.style));
            cur = cell.style;
        }
        out.push(cell.ch);
    }
    if cur != 0 {
        out.push_str("\x1b[0m");
    }
    out
}

/// Set the scroll region below the pinned rows and park the cursor at the
/// bottom of the lower region.
pub fn enter_region(top_rows: u16, term_rows: u16) -> String {
    format!("\x1b[{};{}r\x1b[{};1H", top_rows + 1, term_rows, term_rows)
}

/// Reset the scroll region to the full screen.
pub fn leave_region() -> String {
    "\x1b[r".to_string()
}
```

- [ ] **Step 5: Run tests** — `cargo test -p zvm-cli` PASS; `cargo build` 0 warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/zvm-cli/src/screen.rs crates/zvm-cli/src/aux.rs crates/zvm-cli/src/main.rs
git commit  # feat(zvm-cli): pure screen-model render/format helpers
```

---

## Task 3: `ScreenView` — stateful top-region rendering

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (add `ScreenView` + tests)

**Interfaces:**
- Consumes: the Task 2 helpers; `&zvm::cpu::exec::Machine`.
- Produces:
  - `pub struct ScreenView { is_tty, no_status, term_rows, active_rows, last_block }`
  - `pub fn new(is_tty: bool, no_status: bool, term_rows: u16) -> ScreenView`
  - `pub fn frame(&mut self, machine: &Machine) -> String` — returns the bytes to write before an input prompt (ANSI region update when TTY, or a deduped inline block when piped; empty when nothing changed or `no_status`).
  - `pub fn leave(&mut self) -> String` — bytes to restore the terminal at quit.
  - `fn top_rows(machine) -> u16` (associated helper; testable indirectly).

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod view_tests {
    use super::*;
    use zvm::cpu::exec::Machine;
    use zvm::memory::Memory;

    // Build a minimal v3 machine with a split-free status line, and a v5
    // machine with an upper window, using the existing test helpers in zvm if
    // available; otherwise construct via Memory::new on a tiny story stub.
    // (The implementer may reuse zvm's own test story builders.)

    #[test]
    fn no_status_suppresses_everything() {
        let m = tiny_v3_machine();
        let mut v = ScreenView::new(false, /*no_status=*/true, 24);
        assert_eq!(v.frame(&m), "");
    }

    #[test]
    fn piped_v3_emits_inline_block_once_then_dedupes() {
        let m = tiny_v3_machine(); // version < 4 -> status row active
        let mut v = ScreenView::new(/*is_tty=*/false, false, 24);
        let first = v.frame(&m);
        assert!(first.contains(&status_text(&m.status_line(), DEFAULT_COLS).trim_end().to_string())
            || !first.is_empty(), "first frame emits the status block");
        let second = v.frame(&m); // unchanged
        assert_eq!(second, "", "unchanged region dedupes to empty");
    }

    #[test]
    fn tty_enters_region_then_resets_on_leave() {
        let m = tiny_v3_machine();
        let mut v = ScreenView::new(/*is_tty=*/true, false, 24);
        let f = v.frame(&m);
        assert!(f.contains("\x1b[2;24r"), "sets scroll region: {f:?}");
        assert!(v.leave().contains("\x1b[r"), "leave resets region");
    }
}
```

(If constructing `Machine` in tests is awkward, the implementer may add a small `#[cfg(test)]` story-stub helper mirroring zvm's existing test builders, or move the dedupe/region assertions to operate on a hand-built `ScreenView` plus a hand-built `StatusLine`/`UpperWindow` by factoring `frame` to delegate to a pure `render(top_rows, status, upper, version)` core that the tests call directly. Prefer the pure-core factoring.)

- [ ] **Step 2: Run to verify failure** — `cargo test -p zvm-cli` FAILs to compile.

- [ ] **Step 3: Implement `ScreenView`**

```rust
use zvm::cpu::exec::Machine;

pub struct ScreenView {
    is_tty: bool,
    no_status: bool,
    term_rows: u16,
    active_rows: u16,        // current scroll-region top height (TTY)
    last_block: Option<String>, // last inline block emitted (non-TTY dedupe)
}

impl ScreenView {
    pub fn new(is_tty: bool, no_status: bool, term_rows: u16) -> Self {
        ScreenView { is_tty, no_status, term_rows, active_rows: 0, last_block: None }
    }

    /// Number of pinned top rows for the current machine state.
    fn top_rows(machine: &Machine) -> u16 {
        if machine.mem.version() < 4 {
            1 // v1-v3: a status line is always shown
        } else {
            machine.screen.upper_window_rows
        }
    }

    /// Build the plain-text rows of the top region (status row for v3, the
    /// upper grid for v4+). Empty vec when there is no region.
    fn rows_plain(machine: &Machine, top: u16) -> Vec<String> {
        if top == 0 { return Vec::new(); }
        if machine.mem.version() < 4 {
            vec![status_text(&machine.status_line(), DEFAULT_COLS)]
        } else {
            (1..=top).map(|r| upper_row_text(&machine.screen.upper, r)).collect()
        }
    }

    fn rows_ansi(machine: &Machine, top: u16) -> Vec<String> {
        if top == 0 { return Vec::new(); }
        if machine.mem.version() < 4 {
            vec![format!("\x1b[7m{}\x1b[0m", status_text(&machine.status_line(), DEFAULT_COLS))]
        } else {
            (1..=top).map(|r| upper_row_ansi(&machine.screen.upper, r)).collect()
        }
    }

    /// Bytes to emit just before an input prompt.
    pub fn frame(&mut self, machine: &Machine) -> String {
        if self.no_status {
            return String::new();
        }
        let top = Self::top_rows(machine);
        if self.is_tty {
            let mut out = String::new();
            if top != self.active_rows {
                out.push_str(&if top == 0 { leave_region() } else { enter_region(top, self.term_rows) });
                self.active_rows = top;
            }
            if top > 0 {
                out.push_str("\x1b7"); // DECSC save cursor
                for (i, row) in Self::rows_ansi(machine, top).into_iter().enumerate() {
                    out.push_str(&format!("\x1b[{};1H\x1b[2K", i as u16 + 1)); // row, clear
                    out.push_str(&row);
                }
                out.push_str("\x1b8"); // DECRC restore cursor
            }
            out
        } else {
            if top == 0 {
                return String::new();
            }
            let block = {
                let mut rows = Self::rows_plain(machine, top);
                while rows.last().map(|r| r.trim().is_empty()).unwrap_or(false) {
                    rows.pop();
                }
                if rows.is_empty() { String::new() } else { format!("{}\n", rows.join("\n")) }
            };
            if block.is_empty() || self.last_block.as_deref() == Some(block.as_str()) {
                return String::new();
            }
            self.last_block = Some(block.clone());
            block
        }
    }

    /// Restore the terminal at quit.
    pub fn leave(&mut self) -> String {
        if self.is_tty && self.active_rows > 0 {
            self.active_rows = 0;
            format!("{}\x1b[{};1H", leave_region(), self.term_rows)
        } else {
            String::new()
        }
    }
}
```

- [ ] **Step 4: Run tests** — adjust the test helpers to the pure-core factoring if needed; `cargo test -p zvm-cli` PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm-cli/src/screen.rs
git commit  # feat(zvm-cli): ScreenView pinned-region + inline-block rendering
```

---

## Task 4: Style-aware `StdoutOutput`

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (`StdoutOutput`)
- Test: `crates/zvm-cli/src/main.rs` tests

**Interfaces:**
- Consumes: `screen::style_wrap`.
- Produces: `StdoutOutput { is_tty: bool }` (constructor `StdoutOutput::new(is_tty)`); `print_styled` emits TTY-gated SGR.

- [ ] **Step 1: Failing test**

```rust
#[cfg(test)]
mod stdout_tests {
    use super::*;
    use zvm::io::Output;

    #[test]
    fn print_styled_wraps_only_on_tty() {
        // We can't easily capture stdout; assert via the pure helper that the
        // sink delegates to. The sink's print_styled must call
        // screen::style_wrap(s, style, self.is_tty) then write the result.
        assert_eq!(crate::screen::style_wrap("hi", 2, true), "\x1b[1mhi\x1b[0m");
        assert_eq!(crate::screen::style_wrap("hi", 2, false), "hi");
    }
}
```

(The sink itself writes to the real stdout, so its behavior is exercised by the integration smoke in Task 6; the unit test pins the wrapping helper the sink must use.)

- [ ] **Step 2: Run** — `cargo test -p zvm-cli` PASS for the helper; proceed.

- [ ] **Step 3: Make `StdoutOutput` style-aware**

```rust
struct StdoutOutput {
    is_tty: bool,
}

impl StdoutOutput {
    fn new(is_tty: bool) -> Self {
        StdoutOutput { is_tty }
    }
}

impl Output for StdoutOutput {
    fn print(&mut self, s: &str) {
        print!("{}", s);
        let _ = io::stdout().flush();
    }
    fn print_styled(&mut self, s: &str, style: u8) {
        let out = crate::screen::style_wrap(s, style, self.is_tty);
        print!("{}", out);
        let _ = io::stdout().flush();
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
```

Update `build_machine` to take and pass the TTY flag:

```rust
fn build_machine(story: Vec<u8>, stdout_is_tty: bool) -> Result<Machine, String> {
    // ...unchanged loading...
    let mut machine = Machine::with_output(mem, Box::new(StdoutOutput::new(stdout_is_tty)));
    machine.init_caps();
    Ok(machine)
}
```

- [ ] **Step 4: Run tests + build** — `cargo test -p zvm-cli`, `cargo build` 0 warnings. (Callers of `build_machine` updated in Task 6; for now update the two existing call sites in `main` to pass `false` temporarily, or land Task 6 in the same pass — the implementer may fold Tasks 4 and 6 if the borrow makes a clean split awkward, keeping the commit messages separate.)

- [ ] **Step 5: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit  # feat(zvm-cli): style-aware StdoutOutput emits TTY-gated SGR
```

---

## Task 5: Aux persistence module (`aux.rs`)

**Files:**
- Modify: `crates/zvm-cli/src/aux.rs` (replace the placeholder)
- Test: `crates/zvm-cli/src/aux.rs` tests

**Interfaces:**
- Produces:
  - `pub fn aux_path(story: &std::path::Path) -> std::path::PathBuf` — `<dir>/<stem>.aux`.
  - `pub fn encode_aux(map: &BTreeMap<String, Vec<u8>>) -> Vec<u8>`
  - `pub fn decode_aux(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, AuxError>`
  - `pub enum AuxError { BadMagic, BadVersion, Truncated }`

- [ ] **Step 1: Failing tests**

```rust
use std::collections::BTreeMap;
use std::path::Path;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_uses_stem_and_aux_ext() {
        assert_eq!(aux_path(Path::new("/g/story.z5")), Path::new("/g/story.aux"));
        assert_eq!(aux_path(Path::new("story")), Path::new("story.aux"));
    }

    #[test]
    fn codec_round_trips() {
        let mut m = BTreeMap::new();
        m.insert("FORM".to_string(), vec![1, 2, 3]);
        m.insert("memo".to_string(), Vec::new());
        m.insert("ünïcode".to_string(), vec![9]);
        let bytes = encode_aux(&m);
        assert_eq!(decode_aux(&bytes).unwrap(), m);
    }

    #[test]
    fn decode_rejects_bad_input() {
        assert!(matches!(decode_aux(b"XXXX"), Err(AuxError::BadMagic)));
        let mut good = encode_aux(&BTreeMap::new());
        good[4] = 99; // version byte
        assert!(matches!(decode_aux(&good), Err(AuxError::BadVersion)));
        assert!(matches!(decode_aux(b"ZAUX\x01\x00\x00\x00\x05"), Err(AuxError::Truncated)));
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p zvm-cli` FAIL.

- [ ] **Step 3: Implement the codec + path**

```rust
//! Aux ("global state") persistence for zvm-cli: a per-story `<stem>.aux` file
//! holding the v5 save/restore-table map (`Machine::aux_data`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"ZAUX";
const VERSION: u8 = 1;

#[derive(Debug, PartialEq)]
pub enum AuxError { BadMagic, BadVersion, Truncated }

pub fn aux_path(story: &Path) -> PathBuf {
    story.with_extension("aux")
}

pub fn encode_aux(map: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    for (name, data) in map {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u32).to_le_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

fn take<'a>(b: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], AuxError> {
    let end = pos.checked_add(n).ok_or(AuxError::Truncated)?;
    let s = b.get(*pos..end).ok_or(AuxError::Truncated)?;
    *pos = end;
    Ok(s)
}

fn take_u32(b: &[u8], pos: &mut usize) -> Result<usize, AuxError> {
    let s = take(b, pos, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

pub fn decode_aux(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, AuxError> {
    let mut pos = 0;
    if take(bytes, &mut pos, 4)? != MAGIC { return Err(AuxError::BadMagic); }
    if take(bytes, &mut pos, 1)?[0] != VERSION { return Err(AuxError::BadVersion); }
    let count = take_u32(bytes, &mut pos)?;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let nlen = take_u32(bytes, &mut pos)?;
        let name = String::from_utf8(take(bytes, &mut pos, nlen)?.to_vec()).map_err(|_| AuxError::Truncated)?;
        let dlen = take_u32(bytes, &mut pos)?;
        let data = take(bytes, &mut pos, dlen)?.to_vec();
        map.insert(name, data);
    }
    Ok(map)
}
```

- [ ] **Step 4: Run tests** — `cargo test -p zvm-cli` PASS; build 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm-cli/src/aux.rs
git commit  # feat(zvm-cli): aux-table persistence codec + path
```

---

## Task 6: main.rs integration (args, TTY, loop, raw key, aux wiring)

**Files:**
- Modify: `crates/zvm-cli/src/main.rs`
- Test: `crates/zvm-cli/src/main.rs` tests (arg parsing)

**Interfaces:**
- Consumes: `screen::{ScreenView, term_rows, wants_raw_char, bleep_bytes}`, `aux::{aux_path, encode_aux, decode_aux}`, `std::io::IsTerminal`.

- [ ] **Step 1: Failing test — arg parsing**

```rust
#[cfg(test)]
mod arg_tests {
    use super::*;

    #[test]
    fn parses_flags_and_story() {
        let a = parse_args(&["zvm-cli".into(), "--no-status".into(), "game.z5".into()]);
        assert_eq!(a.story.as_deref(), Some("game.z5"));
        assert!(a.no_status && !a.no_aux);

        let b = parse_args(&["zvm-cli".into(), "--no-aux".into(), "g".into()]);
        assert!(b.no_aux && !b.no_status);

        let c = parse_args(&["zvm-cli".into(), "g".into()]);
        assert!(!c.no_status && !c.no_aux);
    }
}
```

- [ ] **Step 2: Run to verify failure** — FAIL (no `parse_args`).

- [ ] **Step 3: Add arg parsing**

```rust
struct Args { story: Option<String>, no_status: bool, no_aux: bool }

fn parse_args(argv: &[String]) -> Args {
    let mut a = Args { story: None, no_status: false, no_aux: false };
    for arg in &argv[1..] {
        match arg.as_str() {
            "--no-status" | "--lower-only" => a.no_status = true,
            "--no-aux" => a.no_aux = true,
            s if !s.starts_with("--") && a.story.is_none() => a.story = Some(s.to_string()),
            _ => {}
        }
    }
    a
}
```

- [ ] **Step 4: Add terminal-size + raw-key + aux side-effect helpers** (I/O; not unit-tested, exercised by manual runs)

```rust
use std::io::IsTerminal;
use std::process::Command;

fn detect_term_rows() -> u16 {
    let stty = Command::new("stty").arg("size").output().ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    let env_lines = std::env::var("LINES").ok();
    screen::term_rows(stty.as_deref(), env_lines.as_deref())
}

/// Read one keypress in raw mode via stty (TTY only); fall back to a line byte.
fn read_char_input(stdin_is_tty: bool) -> u8 {
    use std::io::Read;
    if !screen::wants_raw_char(stdin_is_tty) {
        return read_byte_stdin();
    }
    let saved = Command::new("stty").arg("-g").output().ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    let _ = Command::new("stty").args(["-icanon", "-echo", "min", "1", "time", "0"]).status();
    let mut buf = [0u8; 1];
    let n = io::stdin().read(&mut buf).unwrap_or(0);
    if let Some(s) = saved {
        let _ = Command::new("stty").arg(s.trim()).status();
    }
    if n == 0 { b'\n' } else { buf[0] }
}

/// Load <stem>.aux into the machine's aux_data (preload); warn on decode error.
fn aux_preload(machine: &mut Machine, story_path: &std::path::Path, no_aux: bool) {
    if no_aux { return; }
    let path = aux::aux_path(story_path);
    if let Ok(bytes) = std::fs::read(&path) {
        match aux::decode_aux(&bytes) {
            Ok(map) => { machine.aux_data = map; machine.aux_dirty = false; }
            Err(e) => eprintln!("zvm: warning: ignoring corrupt {}: {:?}", path.display(), e),
        }
    }
}

/// Flush aux_data to <stem>.aux when dirty; clear the flag regardless.
fn aux_flush(machine: &mut Machine, story_path: &std::path::Path, no_aux: bool) {
    if no_aux || !machine.aux_dirty { return; }
    let path = aux::aux_path(story_path);
    if let Err(e) = std::fs::write(&path, aux::encode_aux(&machine.aux_data)) {
        eprintln!("zvm: warning: aux save to {} failed: {}", path.display(), e);
    }
    machine.aux_dirty = false;
}
```

- [ ] **Step 5: Rewrite `main` to wire it together**

Key changes (keep the existing Save/Restore/Restart handlers, but route them through the new flags + screen leave):

```rust
fn main() {
    let argv: Vec<String> = env::args().collect();
    let args = parse_args(&argv);
    let Some(story_arg) = args.story.clone() else {
        eprintln!("Usage: {} [--no-status] [--no-aux] <story-file>", argv[0]);
        process::exit(1);
    };
    let story_path = std::path::PathBuf::from(&story_arg);

    let story_bytes = match fs::read(&story_path) { /* unchanged error handling */ };
    let original_bytes = story_bytes.clone();

    let stdout_is_tty = io::stdout().is_terminal();
    let stdin_is_tty = io::stdin().is_terminal();

    let mut machine = match build_machine(story_bytes, stdout_is_tty) { /* unchanged */ };
    aux_preload(&mut machine, &story_path, args.no_aux);

    let mut view = screen::ScreenView::new(stdout_is_tty, args.no_status, detect_term_rows());

    loop {
        let step = machine.step();
        for d in machine.diagnostics.drain(..) { eprintln!("zvm: warning: {d}"); }
        // Bleeps: drain and ring (TTY only).
        let beeps = machine.pending_beeps.len();
        machine.pending_beeps.clear();
        if beeps > 0 { print!("{}", screen::bleep_bytes(beeps, stdout_is_tty)); let _ = io::stdout().flush(); }
        // v3 show_status redraw request.
        if machine.screen.show_status_requested {
            print!("{}", view.frame(&machine));
            let _ = io::stdout().flush();
            machine.screen.show_status_requested = false;
        }
        aux_flush(&mut machine, &story_path, args.no_aux);

        match step {
            StepResult::Continue => {}
            StepResult::Quit => { print!("{}", view.leave()); let _ = io::stdout().flush(); break; }
            StepResult::Restart => {
                machine = match build_machine(original_bytes.clone(), stdout_is_tty) { /* unchanged */ };
                aux_preload(&mut machine, &story_path, args.no_aux);
            }
            StepResult::NeedLine { .. } => {
                print!("{}", view.frame(&machine)); let _ = io::stdout().flush();
                let line = read_line_stdin();
                machine.supply_line(line.trim_end());
            }
            StepResult::NeedChar => {
                print!("{}", view.frame(&machine)); let _ = io::stdout().flush();
                let ch = read_char_input(stdin_is_tty);
                machine.supply_char(ch);
            }
            StepResult::SaveRequest => { /* unchanged file-based Quetzal */ }
            StepResult::RestoreRequest => { /* unchanged */ }
        }
    }
}
```

- [ ] **Step 6: Run tests + manual smoke**

Run: `cargo test --workspace` (all green), `cargo build` + `cargo doc --no-deps` (0 warnings).
Manual (not automated): pipe a v3 story (`echo "look" | cargo run -p zvm-cli -- game.z3`) and confirm the inline status block appears once; add `--no-status` and confirm byte-identical legacy output.

- [ ] **Step 7: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit  # feat(zvm-cli): DOS screen model + raw key + aux persistence wired into the run loop
```

---

## Self-review checklist (run before final review)

- Engine: only `Output::print_styled` (default) + one `print_text` call site changed; full `zvm` suite green (proves no behavior change for the app/BufferOutput).
- `--no-status` yields byte-identical legacy stdout (no ANSI, no inline block); `--no-aux` writes no files.
- Non-TTY never emits ANSI/`\x07`; the inline block dedupes.
- Game-visible dims remain 80×24 (no code sets header dims from the terminal).
- Aux file only written when a game saves a table; corrupt/missing aux is non-fatal.
- 0 warnings (`cargo build`, `cargo doc --no-deps`); `cargo test --workspace` green.
