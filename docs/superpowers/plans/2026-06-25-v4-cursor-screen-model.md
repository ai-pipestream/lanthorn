# v4+ Cursor-Addressed Screen Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the Z-machine upper window (a cursor-addressed character grid) and drive real-time `read_char` input, so v4+ status lines display and forms like Bureaucracy's licence application are fillable in place.

**Architecture:** (1) The VM gains an upper-window character grid in `ScreenState`; `print_text` routes into it when `current_window == 1`, positioned by `cursor_row/col`; `split_window`/`erase_window`/`erase_line` size/clear it. (2) The app renders that grid as a fixed region atop the scrolling transcript in the story pane, themed, with the viewport auto-following the cursor. (3) The session surfaces the pending input kind and a `submit_char`; the event loop forwards single keystrokes during `read_char`, hiding the bottom prompt, with Ctrl-K reserved.

**Tech Stack:** Rust; crate `zvm` (VM) + crate `app` (ratatui TUI). Tests are in-file `#[cfg(test)] mod tests`; `cargo test -p zvm` / `-p app`.

## Global Constraints (verbatim from the spec)

- Fixed, configurable virtual screen: `virtual_screen_cols` default **80**, `virtual_screen_rows` default **24**; stable for the session; resize only moves the viewport.
- The upper window renders as the **top, fixed, non-scrolling** region of the story pane; the transcript scrolls below it.
- Viewport clips and **auto-follows the game's cursor** when the pane is smaller than the virtual size; status hint to widen.
- `read_char` forwards single keystrokes; the bottom prompt is hidden during char mode; **Ctrl-K stays reserved** as the escape-hatch.
- Theming via `style.toml`: `upper_window` (text fg/bg), `upper_window_border` (color), `virtual_window_border` (BorderStyle, default **single**); per-cell reverse/bold from the game's text_style still apply.
- Out of scope: lower-window cursor addressing, timed input, v6 graphics, real font-pixel metrics.
- After every task: `cargo test -p <crate>` green; `cargo build -p <crate>` 0 warnings.
- Commit trailers (NO backticks in the body — zsh):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` /
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

## Key existing anchors (from investigation)

- `print_text` (`crates/zvm/src/cpu/exec.rs:1067`, `pub fn print_text(&mut self, s: &str)`): routes stream3 → `streams.write_stream3`; else stream1 → `self.out.print(s)`. Window-1 branch goes here.
- `StepResult::NeedLine { text_buf, parse_buf }` / `NeedChar` (`exec.rs:34/36`); resume via `supply_line(&str)` (`:1111`) / `supply_char(ch: u8)` (`:1181`).
- `pub screen: ScreenState` on `Machine` (`exec.rs:69`); `ScreenState` (`screen.rs:43`) fields `upper_window_rows`, `current_window`, `cursor_row`, `cursor_col`, `text_style` (1-based cursor).
- VAR arms: `split_window` 0x0A (`exec.rs:788`), `set_window` 0x0B (`:794`), `erase_window` 0x0D (`:800`), `set_cursor` 0x0F (`:810`), `erase_line` 0x0E (`:932`, no-op).
- `GameSession` (`crates/app/src/session.rs`): `pub machine: Machine` (`:67`), `new` (`:80`), `submit(&str) -> TurnResult` (`:98`), `take_transcript` (`:92`).
- `render_transcript(machine: &Machine, state: &AppState, area: Rect, buf: &mut Buffer)` (`crates/app/src/render/transcript.rs:361`); input line via `render_input_content`/`format_input_line` (`:479`/`:288`).
- Story-pane render calls: `main.rs:260` and `:308`. Char input buffering: `input.rs:962` `game_key_to_action`, submit at `main.rs:1434` from `state.input`.
- Config field pattern: `crates/app/src/config.rs` (e.g. SearchConfig `:88`, `#[serde(default="...")]` + module-level default fn). Style selectors: `crates/app/src/colors.rs` + `style.rs` SELECTOR_FIELDS/apply/export.

---

### Task 1: Upper-window grid data structure (VM)

**Files:** Modify `crates/zvm/src/screen.rs` (add `UpperWindow` + integrate into `ScreenState`). Test: same file.

**Interfaces — Produces:**
- `pub struct Cell { pub ch: char, pub style: u8 }` (Default: `{ ch: ' ', style: 0 }`).
- On `ScreenState`: `pub upper: UpperWindow` where `pub struct UpperWindow { pub cols: u16, pub rows: u16, pub cells: Vec<Cell> }` with methods:
  - `pub fn resize(&mut self, rows: u16, cols: u16)` — set dims, fill `cells` with `rows*cols` default Cells.
  - `pub fn clear(&mut self)` — reset all cells to default.
  - `pub fn cell(&self, row: u16, col: u16) -> Cell` — 1-based lookup, default if out of range.
  - `pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8)` — 1-based write, clamped (no-op if out of range).

- [ ] **Step 1: Failing test**

```rust
#[test]
fn upper_window_resize_put_and_cell() {
    let mut w = UpperWindow::default();
    w.resize(2, 4);
    assert_eq!(w.rows, 2);
    assert_eq!(w.cols, 4);
    assert_eq!(w.cell(1, 1).ch, ' ');
    w.put(2, 3, 'X', 0b0001);
    assert_eq!(w.cell(2, 3).ch, 'X');
    assert_eq!(w.cell(2, 3).style, 0b0001);
    w.put(9, 9, 'Z', 0); // out of range -> ignored, no panic
    w.clear();
    assert_eq!(w.cell(2, 3).ch, ' ');
}
```

- [ ] **Step 2: Run, expect failure** — `cargo test -p zvm upper_window_resize_put_and_cell 2>&1 | tail -8` → FAIL (no `UpperWindow`).

- [ ] **Step 3: Implement**

```rust
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub style: u8,
}
impl Default for Cell {
    fn default() -> Self { Cell { ch: ' ', style: 0 } }
}

#[derive(Debug, Default)]
pub struct UpperWindow {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
}
impl UpperWindow {
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.rows = rows;
        self.cols = cols;
        self.cells = vec![Cell::default(); rows as usize * cols as usize];
    }
    pub fn clear(&mut self) {
        for c in &mut self.cells { *c = Cell::default(); }
    }
    fn idx(&self, row: u16, col: u16) -> Option<usize> {
        if row == 0 || col == 0 || row > self.rows || col > self.cols { return None; }
        Some(((row - 1) as usize) * self.cols as usize + (col - 1) as usize)
    }
    pub fn cell(&self, row: u16, col: u16) -> Cell {
        self.idx(row, col).and_then(|i| self.cells.get(i).copied()).unwrap_or_default()
    }
    pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8) {
        if let Some(i) = self.idx(row, col) {
            if let Some(c) = self.cells.get_mut(i) { *c = Cell { ch, style }; }
        }
    }
}
```

Add `pub upper: UpperWindow,` to `ScreenState` (it derives `Default`, and `UpperWindow: Default`, so no constructor change needed).

- [ ] **Step 4: Run, expect pass.** `cargo test -p zvm upper_window_resize_put_and_cell 2>&1 | tail -6`.
- [ ] **Step 5: Commit** — `git commit -m "feat(zvm): add upper-window character grid to ScreenState"`.

---

### Task 2: Route output + cursor ops into the grid (VM)

**Files:** Modify `crates/zvm/src/cpu/exec.rs` — `print_text` (`:1067`), `split_window` 0x0A (`:788`), `erase_window` 0x0D (`:800`). Test: same file.

**Interfaces — Consumes:** Task 1's `UpperWindow` (`self.screen.upper`). The grid width comes from header byte 0x21 (`self.mem.read_byte(0x21)`), set by the host.

**Behavior:** When `current_window == 1`, `print_text` writes each char into `screen.upper` at `(cursor_row, cursor_col)` using `screen.text_style`, advancing `cursor_col` (wrap to next row at `cols`+1; `\n` → next row, col 1), clamped to the grid; it does NOT call `self.out.print`. `split_window N` resizes the grid to `N` rows × (header cols) and resets the cursor to (1,1). `erase_window`: arg 1 or -1 clears the grid (and -1 also sets rows 0 / resizes to 0).

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn print_to_upper_window_lands_in_grid_not_stream() {
    let mut m = build_test_machine(&[]);
    m.mem.write_byte(0x21, 10); // screen width = 10 cols
    m.exec_var(0x0A, &[2], None, None);     // split_window 2
    m.exec_var(0x0B, &[1], None, None);     // set_window 1 (upper)
    m.screen.cursor_row = 1; m.screen.cursor_col = 1;
    m.print_text("Hi");
    assert_eq!(m.screen.upper.cell(1, 1).ch, 'H');
    assert_eq!(m.screen.upper.cell(1, 2).ch, 'i');
    assert_eq!(m.screen.cursor_col, 3, "cursor advanced past the text");
    // Nothing went to the lower-window output sink:
    assert_eq!(m.buffer_output().expect("sink").buf, "");
}

#[test]
fn lower_window_still_streams() {
    let mut m = build_test_machine(&[]);
    m.screen.current_window = 0;
    m.print_text("ok");
    assert_eq!(m.buffer_output().expect("sink").buf, "ok");
}

#[test]
fn split_window_sizes_grid_from_header_cols() {
    let mut m = build_test_machine(&[]);
    m.mem.write_byte(0x21, 12);
    m.exec_var(0x0A, &[3], None, None);
    assert_eq!(m.screen.upper.rows, 3);
    assert_eq!(m.screen.upper.cols, 12);
}
```

- [ ] **Step 2: Run, expect failure** — `cargo test -p zvm upper_window_lands 2>&1 | tail -10` (and the other two) → FAIL.

- [ ] **Step 3: Implement**

In `split_window` (0x0A) arm, after setting `upper_window_rows`:

```rust
0x0A => {
    let rows = ops.first().copied().unwrap_or(0);
    self.screen.upper_window_rows = rows;
    let cols = self.mem.read_byte(0x21) as u16;
    self.screen.upper.resize(rows, cols.max(1));
    self.screen.cursor_row = 1;
    self.screen.cursor_col = 1;
    StepResult::Continue
}
```

In `erase_window` (0x0D), extend so arg `1` or `-1` clears the grid:

```rust
0x0D => {
    let win = ops.first().copied().unwrap_or(0) as i16;
    if win == -1 {
        self.screen.upper_window_rows = 0;
        self.screen.upper.resize(0, self.screen.upper.cols);
    } else if win == 1 {
        self.screen.upper.clear();
    }
    StepResult::Continue
}
```

In `print_text` (`:1067`), before the stream-1 branch, add the window-1 routing:

```rust
pub fn print_text(&mut self, s: &str) {
    if self.streams.stream3_active() {
        self.streams.write_stream3(s);
        return;
    }
    if self.screen.current_window == 1 {
        let style = self.screen.text_style;
        let cols = self.screen.upper.cols.max(1);
        for ch in s.chars() {
            if ch == '\n' {
                self.screen.cursor_row += 1;
                self.screen.cursor_col = 1;
                continue;
            }
            let (r, c) = (self.screen.cursor_row, self.screen.cursor_col);
            self.screen.upper.put(r, c, ch, style);
            if self.screen.cursor_col >= cols {
                self.screen.cursor_row += 1;
                self.screen.cursor_col = 1;
            } else {
                self.screen.cursor_col += 1;
            }
        }
        return;
    }
    if self.streams.stream1 {
        self.out.print(s);
    }
}
```

(Match the exact existing body of `print_text` for the stream3/stream1 conditions — the above mirrors the investigated routing; preserve any nuance found in the real function.)

- [ ] **Step 4: Run, expect pass** (all three tests).
- [ ] **Step 5: Commit** — `git commit -m "feat(zvm): route window-1 output into the upper-window grid; size it on split_window"`.

---

### Task 3: Complete erase_line + get_cursor consistency (VM)

**Files:** Modify `crates/zvm/src/cpu/exec.rs` — `erase_line` 0x0E (`:932`). Test: same file.

**Interfaces — Consumes:** Task 1/2 grid. ZMSD: `erase_line 1` clears from the cursor to the end of the current line in the upper window.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn erase_line_clears_to_end_of_row_in_upper() {
    let mut m = build_test_machine(&[]);
    m.mem.write_byte(0x21, 5);
    m.exec_var(0x0A, &[1], None, None); // split 1 row, 5 cols
    m.screen.current_window = 1;
    m.screen.cursor_row = 1; m.screen.cursor_col = 1;
    m.print_text("ABCDE");
    // move cursor back to col 3 and erase to end of line
    m.screen.cursor_col = 3;
    m.exec_var(0x0E, &[1], None, None);
    assert_eq!(m.screen.upper.cell(1, 2).ch, 'B', "before cursor untouched");
    assert_eq!(m.screen.upper.cell(1, 3).ch, ' ', "from cursor cleared");
    assert_eq!(m.screen.upper.cell(1, 5).ch, ' ', "to end of line cleared");
}
```

- [ ] **Step 2: Run, expect failure** (current arm is a no-op).

- [ ] **Step 3: Implement** — replace the `0x0E => StepResult::Continue,` skeleton:

```rust
0x0E => {
    let value = ops.first().copied().unwrap_or(0);
    if value == 1 {
        let (row, start) = (self.screen.cursor_row, self.screen.cursor_col);
        let cols = self.screen.upper.cols;
        let style = self.screen.text_style;
        let mut c = start;
        while c <= cols {
            self.screen.upper.put(row, c, ' ', style);
            c += 1;
        }
    }
    StepResult::Continue
}
```

- [ ] **Step 4: Run, expect pass.**
- [ ] **Step 5: Commit** — `git commit -m "feat(zvm): implement erase_line against the upper-window grid"`.

---

### Task 4: Virtual-screen config + host applies it (app)

**Files:** Modify `crates/app/src/config.rs` (add fields + defaults); `crates/app/src/main.rs` (apply on load). Test: config.rs.

**Interfaces — Produces:** `Config.virtual_screen_cols: u16` (default 80), `virtual_screen_rows: u16` (default 24).

- [ ] **Step 1: Failing tests** (config.rs tests module):

```rust
#[test]
fn virtual_screen_defaults_80x24() {
    let cfg = Config::default();
    assert_eq!(cfg.virtual_screen_cols, 80);
    assert_eq!(cfg.virtual_screen_rows, 24);
}
#[test]
fn virtual_screen_parses_from_toml() {
    let cfg: Config = toml::from_str("virtual_screen_cols = 64\nvirtual_screen_rows = 20").unwrap();
    assert_eq!(cfg.virtual_screen_cols, 64);
    assert_eq!(cfg.virtual_screen_rows, 20);
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement** — add to the `Config` struct (with the `#[serde(default="...")]` pattern used by existing fields) and module-level defaults:

```rust
#[serde(default = "default_virtual_screen_cols")]
pub virtual_screen_cols: u16,
#[serde(default = "default_virtual_screen_rows")]
pub virtual_screen_rows: u16,
```
```rust
fn default_virtual_screen_cols() -> u16 { 80 }
fn default_virtual_screen_rows() -> u16 { 24 }
```
Add the two fields to the `impl Default for Config` block (= 80 / 24). If the config has a CONFIG_ROWS guard or a TOML export writer (mirror `auto_save`), add the two keys there too.

- [ ] **Step 4: Apply on story load** — in `main.rs`, after `session` is constructed and `init_caps` has run (search for where the archive/map is set up, near `let mut mapper = ...`), call:

```rust
zvm::screen::write_screen_dims(&mut session.machine.mem, cfg.virtual_screen_rows as u8, cfg.virtual_screen_cols as u8);
```
(`write_screen_dims` is `pub` in `crates/zvm/src/screen.rs`. Confirm the exact module path; it may be re-exported. If `mem` is not public on `Machine`, add `pub fn set_screen_dims(&mut self, rows: u8, cols: u8)` on `Machine` that calls `crate::screen::write_screen_dims(&mut self.mem, rows, cols)` and call that instead.)

- [ ] **Step 5: Run config tests + build** — `cargo test -p app virtual_screen 2>&1 | tail -8`; `cargo build -p app 2>&1 | grep -c warning`.
- [ ] **Step 6: Commit** — `git commit -m "feat(app): configurable virtual screen size, applied to the VM on load"`.

---

### Task 5: Upper-window theming (style selectors)

**Files:** Modify `crates/app/src/colors.rs` (style fields + defaults), `crates/app/src/style.rs` (SELECTOR_FIELDS + apply + export), and the `style.toml` schema doc. Test: colors.rs / style.rs.

**Interfaces — Produces:** `ColorScheme.upper_window: Style`, `upper_window_border: Style`; `ColorScheme.virtual_window_border: BorderStyle` (default `BorderStyle::Single`).

- [ ] **Step 1: Failing test** (style.rs tests):

```rust
#[test]
fn upper_window_selectors_parse_and_default() {
    // default border is single
    let (cs, _, _) = resolve(&parse_style_toml(DEFAULT_STYLE_TOML).unwrap(), std::path::Path::new("."));
    assert_eq!(cs.virtual_window_border, crate::render::paneframe::BorderStyle::Single);
    // selector applies
    let doc = parse_style_toml("[upper_window]\nfg = \"cyan\"\n").unwrap();
    let (cs2, _, _) = resolve(&doc, std::path::Path::new("."));
    // (assert the fg parsed; match how other selector tests assert a Style fg)
    let _ = cs2.upper_window;
}
```

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement** — add the two `Style` fields + the `virtual_window_border: BorderStyle` to `ColorScheme` (both hardcoded and scheme-based default blocks in colors.rs), defaulting `upper_window`/`upper_window_border` to sensible values (e.g. inherit fg, border = map_border color) and `virtual_window_border = BorderStyle::Single`. Add `"upper_window"` and `"upper_window_border"` to `SELECTOR_FIELDS` and the `apply`/`export` match arms in style.rs, mirroring `map_layer_tab`. Add a `virtual_window_border` border-style key (mirror how `map_border_style` is parsed from the style file).

- [ ] **Step 4: Run, expect pass + build clean.**
- [ ] **Step 5: Commit** — `git commit -m "feat(app): themeable upper window (upper_window/_border + virtual_window_border)"`.

---

### Task 6: Render the upper-window grid (app)

**Files:** Create `crates/app/src/render/upper_window.rs`; modify `crates/app/src/render/mod.rs` (export); modify `crates/app/src/main.rs` story-pane render (`:260`, `:308`). Test: upper_window.rs.

**Interfaces — Consumes:** `session.machine.screen.upper` (Task 1/2), the theming (Task 5). Produces: `pub fn draw_upper_window(machine: &Machine, state: &AppState, area: Rect, buf: &mut Buffer) -> u16` — draws the grid (with themed border + bg + per-cell reverse/bold) into the top of `area`, viewport auto-following `screen.cursor_row/col`, and returns the number of story-pane rows it consumed (0 when `upper_window_rows == 0`).

- [ ] **Step 1: Failing test**

```rust
#[test]
fn draws_grid_cells_and_consumes_rows() {
    use ratatui::{buffer::Buffer, layout::Rect};
    // Build a session/machine with a 2x5 upper window holding "HI" on row 1.
    // (Use the app test helpers; if none expose a Machine, construct via
    //  app::session::GameSession::new(minimal story) or a screen-only fixture.)
    // Assert: draw_upper_window returns >= 2; the cells 'H','I' appear in the buffer's top rows.
}
```

(Write this against whatever machine/screen fixture the app tests already use; if the app has no direct screen fixture, add a tiny helper that builds a `ScreenState` with a populated `UpperWindow` and render from that — keep the renderer taking the grid, not the whole machine, if that is easier to test. Prefer `draw_upper_window(upper: &UpperWindow, cursor: (u16,u16), style…, area, buf)` if it improves testability.)

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement** the renderer: draw the themed border (via `draw_pane_frame` with `virtual_window_border`) when not `None`, fill the bg with `upper_window`, then for each visible grid cell draw its `ch` with a Style derived from `upper_window` patched by the cell's text_style bits (bit1 bold, bit4 reverse). Compute the viewport offset so the cursor row/col stays visible (auto-follow): if `cursor_row` > visible rows, scroll; same for cols. Return rows consumed.

- [ ] **Step 4: Wire into the story pane** — at `main.rs:260` and `:308`, before `render_transcript`, split `story_frame.content`: `let used = draw_upper_window(&session.machine, state, story_frame.content, buf); let transcript_area = Rect::new(content.x, content.y + used, content.width, content.height.saturating_sub(used));` then `render_transcript(&session.machine, state, transcript_area, buf);`.

- [ ] **Step 5: Run tests + build; commit** — `git commit -m "feat(app): render the upper-window grid atop the transcript with viewport auto-follow"`.

---

### Task 7: Session surfaces input kind + submit_char (app)

**Files:** Modify `crates/app/src/session.rs`. Test: session.rs.

**Interfaces — Produces:**
- `pub enum InputKind { Line, Char }`.
- `GameSession::pending_input(&self) -> InputKind` — derived from the last `StepResult` the session paused on (track it in a field set by `new`/`submit`/`submit_char`).
- `pub fn submit_char(&mut self, ch: u8) -> TurnResult` — calls `self.machine.supply_char(ch)`, runs `run_until_input`, returns `TurnResult` like `submit`.

- [ ] **Step 1: Failing test** — build a session on a tiny story that issues `read_char`; assert `pending_input()` is `Char`; `submit_char(b'x')` advances; for a normal line read, `pending_input()` is `Line`. (If no such fixture exists, assert the simpler invariant: after `new` on the standard test story, `pending_input()` is `Line`, and `submit_char` exists and returns a `TurnResult`.)

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement** — add a `pending: InputKind` field; set it wherever the session pauses on `NeedLine` (→ Line) vs `NeedChar` (→ Char) inside `run_until_input`. Add `submit_char` mirroring `submit` but calling `supply_char`. Expose `pending_input()`.

- [ ] **Step 4: Run, expect pass; commit** — `git commit -m "feat(app): session exposes pending input kind + submit_char"`.

---

### Task 8: Char-input mode in the event loop (app)

**Files:** Modify `crates/app/src/main.rs` (event loop) and `crates/app/src/render/transcript.rs` (`render_input_content` `:479` — hide prompt during char mode).

**Interfaces — Consumes:** Task 7 `pending_input()`/`submit_char`.

- [ ] **Step 1: Failing test** — a unit test asserting the routing decision: a helper `fn is_char_mode(session: &GameSession) -> bool { matches!(session.pending_input(), InputKind::Char) }` and that, given char mode, a `KeyCode::Char('y')` event maps to a "send char" path rather than buffering into `state.input`. (Test the helper + the branch predicate; full event-loop wiring is integration.)

- [ ] **Step 2: Run, expect failure.**

- [ ] **Step 3: Implement** — in the event loop, before the normal key→action routing, add: if `session.pending_input() == InputKind::Char` AND the key is not the reserved hotkey prefix (Ctrl-K) and not Esc-for-overlays, forward the keystroke: map the `KeyCode` to a ZSCII byte (Enter→13, Char(c)→c as u8, Backspace→8) and call `session.submit_char(byte)`, then `state.append_transcript(session.take_transcript())` (match how `submit`'s result is consumed at `main.rs:1434`), and `continue`. Keep Ctrl-K opening the hotkey dialog. In `render_input_content`, guard the prompt draw with `if !char_mode` (thread a `char_mode: bool` into the render or read it from the machine's pending state).

- [ ] **Step 4: Run tests + build; commit** — `git commit -m "feat(app): forward read_char keystrokes in char-input mode; hide the bottom prompt; keep Ctrl-K"`.

---

### Task 9: README + style schema + Bureaucracy verification

**Files:** `README.md`, `style.toml` schema doc; verification only otherwise.

- [ ] **Step 1: README** — add a "v4+ screen model" note under Interpreter/Playing-aids: upper-window status lines + cursor-addressed forms render; configurable `virtual_screen_cols/rows`; themeable via `upper_window*`. Update the style-selector list with the three new selectors.
- [ ] **Step 2: Commit docs** — `git commit -m "docs: README + style schema for the v4+ upper-window screen model"`.
- [ ] **Step 3: Verify Bureaucracy** — run the TUI build manually (note: interactive, so describe the check rather than automating): launch `babelmap stories/bureaucr.z4` in a wide (>=80 col) terminal, press a key past the intro, confirm the licence form renders in the upper region and fields accept typed characters in place. Record findings in the ledger. No commit.
