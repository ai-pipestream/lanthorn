# Z-Machine Colour Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Honor game-driven `set_colour` / `set_true_colour` in the Z-machine and render the colours in zvm-cli and the app, routed through the user's 16-colour scheme palette, gated by a `honor_game_colours` toggle (default ON).

**Architecture:** zvm gains a zero-dep `ZColour` type and tracks current fg/bg in `ScreenState`; the upper-window `Cell` and the lower-window output seam carry colour. A host config flag is threaded into the VM (advertises the Flags1 colour bit) and into both renderers (resolve `ZColour` → ANSI SGR in the CLI, → ratatui `Color` via the scheme palette in the app). Reverse video swaps fg/bg at render time.

**Tech Stack:** Rust workspace — zvm (zero-dep VM), zvm-cli (crossterm), app (ratatui).

## Global Constraints

- **zvm and gvm crates stay ZERO-DEPENDENCY.** `ZColour` derives only std traits (no serde). The app serializes colour via a packed `u32`.
- **Cross-platform:** Windows/Linux/macOS. CLI colour is ANSI SGR (already used for text styles).
- **Scope is Z-machine only.** gvm-cli and Glulx/Glk colour are sub-project 2 — do NOT touch gvm in this plan.
- **`honor_game_colours` default is `true` for all clients** (zvm-cli and app).
- **Colour semantics are per-channel replace with sentinels (NOT cumulative).** `set_colour`: 0=keep, 1=default, 2–12=palette/grey. `set_true_colour`: -2=keep, -1=default, 0..=0x7FFF=15-bit RGB.
- **Reverse video (style bit 0x01) swaps fg/bg exactly once at render time.** Cells/runs store logical (un-swapped) colour.
- **Grey RGB values:** light grey (10) = `#B0B0B0`, medium grey (11) = `#808080`, dark grey (12) = `#505050`.
- **Quality gate per task:** TDD; `cargo build --workspace --tests` with **0 warnings**; full `cargo test --workspace` green.
- **Commits:** local `main` only, never push. No backticks in commit bodies. End every commit body with the two trailers:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

---

## File Structure

- `crates/zvm/src/screen.rs` — new `ZColour` enum; `ScreenState.current_fg/current_bg`; `Cell.fg/bg`; `UpperWindow::put` colour params; `init_header_caps` colour-bit gating + new `advertise_colour` helper.
- `crates/zvm/src/io.rs` — new `TextAttrs` struct; new `Output::print_attr` default method.
- `crates/zvm/src/cpu/exec.rs` — `set_colour` (2OP:0x1B) + `set_true_colour` (EXT:0x0D) honor sentinels; `print_text` records colour into upper cells and calls `print_attr` for the lower window; `Machine.honor_game_colours` field + `set_honor_game_colours`.
- `crates/zvm-cli/src/screen.rs` — `style_wrap` emits colour SGR.
- `crates/zvm-cli/src/main.rs` — sink `print_attr` override; `--no-game-colours` flag; default-on plumbing into the VM.
- `crates/app/src/render/mod.rs` — `resolve_zcolour(ZColour, &ColorScheme) -> Color` + colour helpers.
- `crates/app/src/render/upper_window.rs` — render per-cell fg/bg, reverse-swap.
- `crates/app/src/render/transcript.rs` + `crates/app/src/state.rs` — `StyleRun` carries packed fg/bg; transcript render resolves colour.
- `crates/app/src/session.rs` — `CaptureSink` runs carry colour; `take_styled`/`clamp_runs`/`TurnResult`; `set_honor_game_colours` hook.
- `crates/app/src/config.rs` — `honor_game_colours: bool` (default true).

---

## Task 1: `ZColour` type + screen-state colour fields

**Files:**
- Modify: `crates/zvm/src/screen.rs` (Cell ~39, ScreenState ~93/119, UpperWindow::put ~78)

**Interfaces:**
- Produces:
  - `pub enum ZColour { Default, Standard(u8), True(u16) }` — `Default`-deriving = `ZColour::Default`. `Standard(2..=9)` palette, `Standard(10..=12)` greys, `True(15-bit)`.
  - `Cell { ch: char, style: u8, fg: ZColour, bg: ZColour }`.
  - `ScreenState.current_fg: ZColour`, `ScreenState.current_bg: ZColour`.
  - `UpperWindow::put(&mut self, row, col, ch, style, fg, bg)`.

- [ ] **Step 1: Write the failing test** — append to the `#[cfg(test)] mod tests` in `screen.rs`:

```rust
#[test]
fn zcolour_defaults_and_cell_carries_colour() {
    assert_eq!(ZColour::default(), ZColour::Default);
    let c = Cell::default();
    assert_eq!(c.fg, ZColour::Default);
    assert_eq!(c.bg, ZColour::Default);

    let mut w = UpperWindow::default();
    w.resize(1, 4);
    w.put(1, 1, 'X', 0x01, ZColour::Standard(3), ZColour::Standard(6));
    let cell = w.cell(1, 1);
    assert_eq!(cell.ch, 'X');
    assert_eq!(cell.style, 0x01);
    assert_eq!(cell.fg, ZColour::Standard(3));
    assert_eq!(cell.bg, ZColour::Standard(6));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p zvm zcolour_defaults_and_cell_carries_colour 2>&1 | tail -20`
Expected: compile error — `ZColour` not found / `put` arity mismatch.

- [ ] **Step 3: Implement.** Add the enum above `Cell` (around line 36):

```rust
/// A Z-machine colour channel value (logical, pre-reverse-swap).
///
/// Transient display state — NOT serialised into Quetzal saves (like
/// `current_font`). The host resolves `Default` to the terminal/scheme
/// default, `Standard(2..=9)` to the scheme palette, `Standard(10..=12)` to
/// fixed grey RGB, and `True` to an exact 15-bit RGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZColour {
    Default,
    Standard(u8),
    True(u16),
}
impl Default for ZColour {
    fn default() -> Self {
        ZColour::Default
    }
}
```

Extend `Cell` (line 39) and its `Default` (line 43):

```rust
#[derive(Debug, Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}
impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', style: 0, fg: ZColour::Default, bg: ZColour::Default }
    }
}
```

Change `UpperWindow::put` (line 78):

```rust
pub fn put(&mut self, row: u16, col: u16, ch: char, style: u8, fg: ZColour, bg: ZColour) {
    if let Some(i) = self.idx(row, col) {
        if let Some(c) = self.cells.get_mut(i) {
            *c = Cell { ch, style, fg, bg };
        }
    }
}
```

Add to `ScreenState` (after `current_font`, line 116):

```rust
    /// Current logical foreground/background colour (ZMSD §8.3). Transient
    /// display state — NOT serialised into Quetzal saves.
    pub current_fg: ZColour,
    pub current_bg: ZColour,
```

And to `ScreenState::Default` (after `current_font: 1,`, line 133):

```rust
            current_fg: ZColour::Default,
            current_bg: ZColour::Default,
```

Fix the two existing `put` call sites in `exec.rs` (the erase-line at ~1075 and the print loop at ~1494) by passing the current colours — these are updated fully in Task 4, but to keep the build green now, pass `self.screen.current_fg, self.screen.current_bg` at both sites:

- `exec.rs:1075`: `self.screen.upper.put(row, c, ' ', style, self.screen.current_fg, self.screen.current_bg);`
- `exec.rs:1494`: `self.screen.upper.put(r, c, out_ch, style, self.screen.current_fg, self.screen.current_bg);`

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p zvm 2>&1 | tail -15`
Expected: all pass, including the new test.

- [ ] **Step 5: Commit**

```bash
git add crates/zvm/src/screen.rs crates/zvm/src/cpu/exec.rs
git commit -F - <<'EOF'
feat(zvm): add ZColour type and per-cell / screen colour state

EOF
# (append the standard trailers to the commit body)
```

---

## Task 2: `set_colour` (2OP:0x1B) honors sentinels

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs:418-420`
- Test: same file's `tests` module

**Interfaces:**
- Consumes: `ScreenState.current_fg/current_bg`, `ZColour` (Task 1).
- Produces: `set_colour(fg, bg)` updates `current_fg`/`current_bg`: 0=keep, 1=`Default`, 2..=12=`Standard(n)`, else keep.

- [ ] **Step 1: Write the failing test** — add to the `tests` module:

```rust
#[test]
fn set_colour_honors_sentinels() {
    // Helper: run "set_colour fg,bg" (2OP:0x1B long form, two smalls) at 0x10.
    fn run_set_colour(fg: u8, bg: u8) -> (ZColour, ZColour) {
        let mut buf = sample_story(5);
        // 2OP long form, both operands Small: opcode byte 0x1B, fg, bg.
        buf[0x10] = 0x1B;
        buf[0x11] = fg;
        buf[0x12] = bg;
        buf[0x13] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m.step(); // set_colour
        (m.screen.current_fg, m.screen.current_bg)
    }

    // start both non-default, then 0 must KEEP each channel
    let (fg, bg) = run_set_colour(3, 6);
    assert_eq!(fg, ZColour::Standard(3));
    assert_eq!(bg, ZColour::Standard(6));

    // 1 = default
    assert_eq!(run_set_colour(1, 1), (ZColour::Default, ZColour::Default));

    // greys accepted
    assert_eq!(run_set_colour(10, 12), (ZColour::Standard(10), ZColour::Standard(12)));
}

#[test]
fn set_colour_zero_keeps_channel() {
    // set fg=3,bg=6 then set fg=0,bg=4: fg keeps 3, bg becomes 4.
    let mut buf = sample_story(5);
    buf[0x10] = 0x1B; buf[0x11] = 3; buf[0x12] = 6;      // set_colour 3,6
    buf[0x13] = 0x1B; buf[0x14] = 0; buf[0x15] = 4;      // set_colour 0,4
    buf[0x16] = 0xBA;                                     // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    m.step(); m.step();
    assert_eq!(m.screen.current_fg, ZColour::Standard(3), "fg=0 kept prior fg");
    assert_eq!(m.screen.current_bg, ZColour::Standard(4), "bg updated to 4");
}
```

(If `sample_story`/`Memory`/`Machine`/`ZColour` are not already imported in the test module, add the imports to match the existing tests — they use `sample_story(5)` and `Machine::new` already.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm set_colour 2>&1 | tail -20`
Expected: FAIL — current behavior leaves both `Default`.

- [ ] **Step 3: Implement.** Add a colour-decoding helper near the other private helpers in `exec.rs`, then replace the `0x1B` arm:

```rust
/// Decode a `set_colour` operand into a colour-channel update.
/// Returns `None` for 0 ("leave unchanged"); `Some(ZColour)` otherwise.
fn decode_set_colour(v: u16) -> Option<crate::screen::ZColour> {
    use crate::screen::ZColour;
    match v {
        0 => None,                     // keep current channel
        1 => Some(ZColour::Default),   // default
        2..=12 => Some(ZColour::Standard(v as u8)), // palette + v6 greys
        _ => None,                     // -1 (pixel) / unknown → keep
    }
}
```

Replace `exec.rs:418-420`:

```rust
            // 2OP:0x1B set_colour (v5+). Per-channel replace with sentinels
            // (ZMSD §8.3): 0 = keep, 1 = default, 2..=12 = palette + v6 greys.
            0x1B => {
                if let Some(c) = decode_set_colour(a) {
                    self.screen.current_fg = c;
                }
                if let Some(c) = decode_set_colour(b) {
                    self.screen.current_bg = c;
                }
                StepResult::Continue
            }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm set_colour 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Commit** (`feat(zvm): honor set_colour with per-channel sentinels`).

---

## Task 3: `set_true_colour` (EXT:0x0D) honors sentinels + fix mislabeled 0x05

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — EXT dispatch: correct the `0x05` comment (it is `draw_picture`, not set_true_colour) and add a real `0x0D` arm before the catch-all (`_ =>`) at ~1252.
- Test: same file.

**Interfaces:**
- Produces: `set_true_colour(fg, bg)` (operands signed): -2=keep, -1=`Default`, 0..=0x7FFF=`True(v)`, other negatives=keep.

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn set_true_colour_honors_sentinels() {
    // EXT:0x0D, two operands. Encode via emit_var_instr against the EXT path
    // the existing tests use (see set_colour_and_true_colour_are_graceful_noops
    // for the EXT encoding helper). Drive fg=0x7FFF (white), bg=-1 (default).
    fn run_true(fg: i16, bg: i16) -> (ZColour, ZColour) {
        let mut buf = sample_story(5);
        let instr = {
            let mut v = vec![];
            emit_ext_instr(&mut v, 0x0D, &[fg as u16, bg as u16]);
            v
        };
        buf[0x10..0x10 + instr.len()].copy_from_slice(&instr);
        buf[0x10 + instr.len()] = 0xBA; // quit
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.state.pc = 0x10;
        m.step();
        (m.screen.current_fg, m.screen.current_bg)
    }

    assert_eq!(run_true(0x7FFF, -1), (ZColour::True(0x7FFF), ZColour::Default));

    // -2 keeps. Pre-set fg=3, then true_colour(-2,-1): fg stays Standard(3).
    let mut buf = sample_story(5);
    buf[0x10] = 0x1B; buf[0x11] = 3; buf[0x12] = 6;   // set_colour 3,6
    let mut pos = 0x13;
    let instr = { let mut v = vec![]; emit_ext_instr(&mut v, 0x0D, &[(-2i16) as u16, (-1i16) as u16]); v };
    buf[pos..pos + instr.len()].copy_from_slice(&instr); pos += instr.len();
    buf[pos] = 0xBA;
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    m.step(); m.step();
    assert_eq!(m.screen.current_fg, ZColour::Standard(3), "-2 kept fg");
    assert_eq!(m.screen.current_bg, ZColour::Default, "-1 set bg default");
}
```

If no `emit_ext_instr` helper exists in the test module, add one mirroring `emit_var_instr` but emitting the EXT prefix `0xBE` then opcode then a VAR operand-types byte (all operands as `Large`/`0b00` = word). Reuse the encoding already used by `set_colour_and_true_colour_are_graceful_noops` (exec.rs:4473) — copy its EXT encoding into a named helper so both tests share it.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm set_true_colour_honors_sentinels 2>&1 | tail -20`
Expected: FAIL — 0x0D currently falls through the catch-all (both stay Default).

- [ ] **Step 3: Implement.** Add a decoder helper:

```rust
/// Decode a `set_true_colour` operand (signed). Returns `None` for "keep".
fn decode_true_colour(v: u16) -> Option<crate::screen::ZColour> {
    use crate::screen::ZColour;
    match v as i16 {
        -2 => None,                        // keep current channel
        -1 => Some(ZColour::Default),      // default
        n if n >= 0 => Some(ZColour::True((n as u16) & 0x7FFF)),
        _ => None,                         // -3 transparent / other → keep
    }
}
```

Correct the misleading comment at `exec.rs:1224` (0x05 is `draw_picture`, a v6 picture op — not set_true_colour):

```rust
            // EXT:0x05 draw_picture (v6) — graphics unsupported; accept and ignore.
            0x05 => StepResult::Continue,
```

Add the real arm just before `// Other EXT opcodes: no-op` / `_ =>` (line 1252):

```rust
            // EXT:0x0D set_true_colour (v5+). Same channel model as set_colour
            // but signed sentinels: -2 = keep, -1 = default, else 15-bit RGB.
            0x0D => {
                if let Some(c) = decode_true_colour(ops.first().copied().unwrap_or(0)) {
                    self.screen.current_fg = c;
                }
                if let Some(c) = decode_true_colour(ops.get(1).copied().unwrap_or(0)) {
                    self.screen.current_bg = c;
                }
                StepResult::Continue
            }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p zvm set_true_colour 2>&1 | tail -15`
Expected: PASS (and the pre-existing `set_colour_and_true_colour_are_graceful_noops` still passes — it exercises 0x05, which remains a no-op).

- [ ] **Step 5: Commit** (`feat(zvm): honor set_true_colour (EXT:0x0D); fix mislabeled 0x05`).

---

## Task 4: Upper-window colour capture

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — `print_text` window-1 path (line 1483-1502) already passes `current_fg/bg` after Task 1; add a focused test proving capture. The erase-line at 1075 also already passes them.
- Test: same file.

**Interfaces:**
- Consumes: `set_colour` (Task 2), `UpperWindow::put` colour (Task 1).
- Produces: upper-window cells written while a colour is active carry that colour.

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn upper_window_cells_capture_active_colour() {
    // split_window 1; set_window 1; set_colour 3,6; print "H".
    let mut buf = sample_story(5);
    let mut pos = 0x10;
    let mut emit = |code: u8, args: &[u8], pos: &mut usize| {
        let mut v = vec![]; emit_var_instr(&mut v, code, args);
        buf[*pos..*pos + v.len()].copy_from_slice(&v); *pos += v.len();
    };
    emit(0x0A, &[1], &mut pos); // split_window 1
    emit(0x0B, &[1], &mut pos); // set_window 1
    // set_colour 3,6 (2OP long form)
    buf[pos] = 0x1B; buf[pos+1] = 3; buf[pos+2] = 6; pos += 3;
    // print "H" via print_char 72 (VAR:0x05)
    emit(0x05, &[72], &mut pos);
    buf[pos] = 0xBA; // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    let cell = m.screen.upper.cell(1, 1);
    assert_eq!(cell.ch, 'H');
    assert_eq!(cell.fg, ZColour::Standard(3));
    assert_eq!(cell.bg, ZColour::Standard(6));
}
```

- [ ] **Step 2: Run to verify it fails or passes**

Run: `cargo test -p zvm upper_window_cells_capture_active_colour 2>&1 | tail -20`
Expected: PASS already if Task 1 wired the `put` call sites correctly. If it FAILS (cells show `Default`), the print loop is not passing `current_fg/current_bg` — fix `exec.rs:1494` to pass them.

- [ ] **Step 3: Implement (only if the test failed).** Ensure `exec.rs:1494` reads:

```rust
                self.screen.upper.put(r, c, out_ch, style, self.screen.current_fg, self.screen.current_bg);
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p zvm upper_window 2>&1 | tail -15` → PASS.

- [ ] **Step 5: Commit** (`test(zvm): upper-window cells capture active colour`). If no code changed (test passed at step 2), still commit the test.

---

## Task 5: Output seam — `TextAttrs` + `print_attr`; lower-window uses it

**Files:**
- Modify: `crates/zvm/src/io.rs` (trait + new struct)
- Modify: `crates/zvm/src/cpu/exec.rs:1505-1515` (lower-window print path)
- Test: `crates/zvm/src/io.rs` tests

**Interfaces:**
- Produces:
  - `pub struct TextAttrs { pub style: u8, pub fg: ZColour, pub bg: ZColour }` (`Default`, `Clone`, `Copy`).
  - `Output::print_attr(&mut self, s: &str, attrs: TextAttrs)` — default delegates to `print_styled(s, attrs.style)`.
- Consumes: `ZColour` (Task 1).

- [ ] **Step 1: Write the failing test** (in `io.rs`):

```rust
    #[test]
    fn default_print_attr_delegates_to_print_styled() {
        use crate::screen::ZColour;
        let mut a = BufferOutput::new();
        let mut b = BufferOutput::new();
        a.print_styled("hi", 0x02);
        b.print_attr("hi", TextAttrs { style: 0x02, fg: ZColour::Standard(3), bg: ZColour::Default });
        assert_eq!(a.buf, b.buf, "default print_attr falls back to print_styled");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm print_attr 2>&1 | tail -15`
Expected: FAIL — `TextAttrs` / `print_attr` not defined.

- [ ] **Step 3: Implement.** In `io.rs`, add the struct and the trait method:

```rust
use crate::screen::ZColour;

/// Text attributes for one styled run (logical colour, pre-reverse-swap).
#[derive(Debug, Clone, Copy, Default)]
pub struct TextAttrs {
    pub style: u8,
    pub fg: ZColour,
    pub bg: ZColour,
}
```

Add to `trait Output` (after `print_styled`):

```rust
    /// Print `s` carrying full text attributes (style bitmask + logical
    /// colour). The default delegates to `print_styled`, so sinks that do not
    /// render colour are unaffected.
    fn print_attr(&mut self, s: &str, attrs: TextAttrs) {
        self.print_styled(s, attrs.style);
    }
```

In `exec.rs`, change the lower-window print (1505-1515) to call `print_attr`:

```rust
        if self.streams.stream1 {
            let attrs = crate::io::TextAttrs {
                style: self.screen.text_style,
                fg: self.screen.current_fg,
                bg: self.screen.current_bg,
            };
            if font3 {
                let translated: String = s.chars().map(|ch| {
                    let code = ch as u32;
                    if (32..=126).contains(&code) { font3_translate(ch) } else { ch }
                }).collect();
                self.out.print_attr(&translated, attrs);
            } else {
                self.out.print_attr(s, attrs);
            }
        }
```

- [ ] **Step 4: Run to verify pass** — `cargo test -p zvm 2>&1 | tail -15` → all pass (BufferOutput inherits the default; existing `print_styled` sinks still work).

- [ ] **Step 5: Commit** (`feat(zvm): add TextAttrs + Output::print_attr colour seam`).

---

## Task 6: Header colour-bit gating + `honor_game_colours` on Machine

**Files:**
- Modify: `crates/zvm/src/screen.rs` — `init_header_caps` signature + new `advertise_colour`; v4+ branch stops touching bit 0.
- Modify: `crates/zvm/src/cpu/exec.rs` — `Machine.honor_game_colours` field, `set_honor_game_colours`, update the `init_header_caps` call at line 183.
- Modify: all other `init_header_caps(&mut mem)` call sites (screen.rs tests ~455–560) to pass `false`.
- Test: `screen.rs`.

**Interfaces:**
- Produces:
  - `pub fn advertise_colour(mem: &mut Memory, on: bool)` — sets/clears Flags1 bit 0 for v4+ (no-op v3).
  - `pub fn init_header_caps(mem: &mut Memory, honor_game_colours: bool)`.
  - `Machine::set_honor_game_colours(&mut self, on: bool)` — sets the field and re-advertises.
  - `Machine.honor_game_colours: bool` (default `false`).

- [ ] **Step 1: Write the failing test** (in `screen.rs` tests):

```rust
#[test]
fn colour_bit_tracks_honor_flag() {
    let mut mem = sample_mem(5); // v5 header (use the helper the other caps tests use)
    init_header_caps(&mut mem, false);
    assert_eq!(mem.read_byte(0x01) & 1, 0, "colour bit clear when honor=false");
    init_header_caps(&mut mem, true);
    assert_eq!(mem.read_byte(0x01) & 1, 1, "colour bit set when honor=true");
    advertise_colour(&mut mem, false);
    assert_eq!(mem.read_byte(0x01) & 1, 0, "advertise_colour clears it again");
}
```

(Match whatever memory-builder the existing `init_header_caps` tests use — reuse it; do not invent `sample_mem` if a different name exists.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p zvm colour_bit_tracks_honor_flag 2>&1 | tail -20`
Expected: FAIL — `init_header_caps` arity / `advertise_colour` missing.

- [ ] **Step 3: Implement.** In `screen.rs`, remove `(1 << 0)` from the v4+ clear-mask (line 267) so it reads:

```rust
        f1 & !((1 << 1) | (1 << 5) | (1 << 7))  // clear unsupported (colour handled separately)
          | (1 << 2) | (1 << 3) | (1 << 4)  // bold, italic, fixed-space font available
```

Change the signature (line 238) to `pub fn init_header_caps(mem: &mut Memory, honor_game_colours: bool)` and add, just before the closing brace (after `write_screen_dims`, line 304):

```rust
    advertise_colour(mem, honor_game_colours);
```

Add the helper after `init_header_caps`:

```rust
/// Set or clear the Flags1 "colour available" bit (bit 0). No-op for v3, which
/// has no colour capability bit. Re-applied on every header init and whenever
/// the host toggles `honor_game_colours`.
pub fn advertise_colour(mem: &mut Memory, on: bool) {
    if mem.version() < 4 {
        return;
    }
    let f1 = mem.read_byte(0x01);
    let f1 = if on { f1 | 1 } else { f1 & !1 };
    mem.write_byte(0x01, f1);
}
```

Update all other `init_header_caps(&mut mem)` calls in `screen.rs` tests to `init_header_caps(&mut mem, false)`.

In `exec.rs`: add the field to `Machine` (near the other screen-related fields), default it `false` in `Machine::new`/`with_output`/`with_glk` (whichever constructors build the struct literal), and update line 183:

```rust
        init_header_caps(&mut self.mem, self.honor_game_colours);
```

Add the setter on `impl Machine`:

```rust
    /// Enable/disable honoring game-driven colour. Advertises (or clears) the
    /// Flags1 colour bit immediately so a not-yet-run game sees the capability.
    pub fn set_honor_game_colours(&mut self, on: bool) {
        self.honor_game_colours = on;
        crate::screen::advertise_colour(&mut self.mem, on);
    }
```

Note: `init_caps` (the host-facing wrapper that calls `init_header_caps`, used at session.rs:152) must forward `self.honor_game_colours` — find it in `exec.rs` and confirm it calls the updated `init_header_caps`. The default `false` keeps every existing caller/test unchanged.

- [ ] **Step 4: Run to verify pass** — `cargo build --workspace --tests 2>&1 | grep -c warning` → `0`; `cargo test -p zvm 2>&1 | tail -15` → all pass.

- [ ] **Step 5: Commit** (`feat(zvm): gate Flags1 colour bit on honor_game_colours`).

---

## Task 7: zvm-cli `style_wrap` emits colour SGR

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (`style_wrap`, line 26; tests ~347)

**Interfaces:**
- Produces: `style_wrap(s: &str, attrs: TextAttrs, is_tty: bool) -> String` — emits SGR including fg/bg; piped output stays plain. (Replaces the `style: u8` parameter.)
- Consumes: `zvm::io::TextAttrs`, `zvm::screen::ZColour`.

- [ ] **Step 1: Write the failing test** (in `screen.rs`):

```rust
#[test]
fn style_wrap_emits_colour_sgr() {
    use zvm::io::TextAttrs;
    use zvm::screen::ZColour;
    // standard fg=red(3)->31, bg=blue(6)->44
    let a = TextAttrs { style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) };
    assert_eq!(style_wrap("x", a, true), "\x1b[31;44mx\x1b[0m");
    // default channels emit 39/49
    let d = TextAttrs { style: 0, fg: ZColour::Default, bg: ZColour::Default };
    assert_eq!(style_wrap("x", d, true), "x"); // no attrs → no wrap
    // true colour fg
    let t = TextAttrs { style: 0, fg: ZColour::True(0x7FFF), bg: ZColour::Default };
    assert_eq!(style_wrap("x", t, true), "\x1b[38;2;255;255;255mx\x1b[0m");
    // grey 11 -> 808080
    let g = TextAttrs { style: 0, fg: ZColour::Standard(11), bg: ZColour::Default };
    assert_eq!(style_wrap("x", g, true), "\x1b[38;2;128;128;128mx\x1b[0m");
    // non-tty stays plain
    assert_eq!(style_wrap("x", a, false), "x");
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p zvm-cli style_wrap_emits_colour_sgr 2>&1 | tail -20` → FAIL (signature/behavior).

- [ ] **Step 3: Implement.** Rewrite `style_wrap` and add colour helpers in `zvm-cli/src/screen.rs`:

```rust
use zvm::io::TextAttrs;
use zvm::screen::ZColour;

/// Expand a 15-bit RGB (0bbbbbgggggrrrrr) to 8-bit (r, g, b).
fn rgb15_to_888(v: u16) -> (u8, u8, u8) {
    let exp = |c: u16| -> u8 { ((c << 3) | (c >> 2)) as u8 };
    (exp(v & 0x1F), exp((v >> 5) & 0x1F), exp((v >> 10) & 0x1F))
}

fn grey_rgb(n: u8) -> (u8, u8, u8) {
    match n {
        10 => (0xB0, 0xB0, 0xB0),
        11 => (0x80, 0x80, 0x80),
        _ => (0x50, 0x50, 0x50), // 12
    }
}

/// Push SGR parameters for one colour channel. `fg` selects 3x vs 4x codes.
fn push_colour_sgr(params: &mut Vec<String>, c: ZColour, fg: bool) {
    let (base_std, base_true) = if fg { (30, 38) } else { (40, 48) };
    match c {
        ZColour::Default => {} // 39/49 are implied by the trailing reset; omit
        ZColour::Standard(n @ 2..=9) => params.push((base_std + (n as u16 - 2)).to_string()),
        ZColour::Standard(n) => {
            let (r, g, b) = grey_rgb(n);
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
        ZColour::True(v) => {
            let (r, g, b) = rgb15_to_888(v);
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
    }
}

pub fn style_wrap(s: &str, attrs: TextAttrs, is_tty: bool) -> String {
    if !is_tty {
        return s.to_string();
    }
    let mut params: Vec<String> = Vec::new();
    // text styles (existing behavior): 1=reverse->7, 2=bold->1, 4=italic->3
    if attrs.style & 0x01 != 0 { params.push("7".into()); }
    if attrs.style & 0x02 != 0 { params.push("1".into()); }
    if attrs.style & 0x04 != 0 { params.push("3".into()); }
    push_colour_sgr(&mut params, attrs.fg, true);
    push_colour_sgr(&mut params, attrs.bg, false);
    if params.is_empty() {
        return s.to_string();
    }
    format!("\x1b[{}m{}\x1b[0m", params.join(";"), s)
}
```

(Confirm the existing text-style SGR mapping matches the prior `style_wrap` — adjust the style codes to whatever the current implementation used so existing style-only tests still pass. Reverse is emitted as `7`; do NOT also pre-swap fg/bg.)

- [ ] **Step 4: Run to verify pass** — `cargo test -p zvm-cli 2>&1 | tail -15` → pass (update any existing style-only `style_wrap` tests to the new `TextAttrs` signature).

- [ ] **Step 5: Commit** (`feat(zvm-cli): render game colour as ANSI SGR in style_wrap`).

---

## Task 8: zvm-cli sink `print_attr` + default-on flag + `--no-game-colours`

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` — the `Output` impl (`print_styled` at ~125) gains `print_attr`; arg parsing adds `--no-game-colours`; after constructing the Machine, call `machine.set_honor_game_colours(enabled)`.

**Interfaces:**
- Consumes: `style_wrap(.., TextAttrs, ..)` (Task 7), `Machine::set_honor_game_colours` (Task 6).

- [ ] **Step 1: Write the failing test** — add a unit test for the flag parse (match the existing arg-parse test style in `main.rs`; if args are parsed inline in `main`, extract a small `fn parse_game_colours(args: &[String]) -> bool` returning `true` unless `--no-game-colours` present, and test that):

```rust
#[test]
fn game_colours_default_on_unless_disabled() {
    assert!(parse_game_colours(&[]));
    assert!(parse_game_colours(&["story.z5".into()]));
    assert!(!parse_game_colours(&["--no-game-colours".into(), "story.z5".into()]));
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p zvm-cli game_colours_default_on 2>&1 | tail -15` → FAIL (function missing).

- [ ] **Step 3: Implement.**
- Add `fn parse_game_colours(args: &[String]) -> bool { !args.iter().any(|a| a == "--no-game-colours") }` and filter that flag out of the positional story-path parsing.
- Add the `print_attr` override to the sink (next to `print_styled` at ~125):

```rust
    fn print_attr(&mut self, s: &str, attrs: zvm::io::TextAttrs) {
        let out = crate::screen::style_wrap(s, attrs, self.is_tty);
        // ... same write path print_styled uses (paging, flush) ...
        self.write_out(&out); // mirror whatever print_styled does
    }
```

(Keep `print_styled` working — or have it delegate to `print_attr` with `TextAttrs { style, ..Default::default() }` to avoid two code paths.)
- After building the Machine in `main`, before the run loop: `machine.set_honor_game_colours(parse_game_colours(&args));`

- [ ] **Step 4: Run to verify pass** — `cargo test -p zvm-cli 2>&1 | tail -15` → pass.

- [ ] **Step 5: Commit** (`feat(zvm-cli): honor game colour by default with --no-game-colours opt-out`).

---

## Task 9: app `ZColour` → ratatui `Color` resolver

**Files:**
- Modify: `crates/app/src/render/mod.rs` (near `apply_text_style`, line 34)
- Test: `render/mod.rs` tests

**Interfaces:**
- Produces: `pub(crate) fn resolve_zcolour(c: ZColour, scheme: &ColorScheme) -> Color`.
- Consumes: `zvm::screen::ZColour`, `ColorScheme.palette` (colors.rs:110).

- [ ] **Step 1: Write the failing test** (in `render/mod.rs`):

```rust
    #[test]
    fn resolve_zcolour_maps_palette_grey_true_default() {
        use zvm::screen::ZColour;
        let mut scheme = ColorScheme::default();
        scheme.palette[1] = Color::Rgb(10, 20, 30); // "red" slot
        assert_eq!(resolve_zcolour(ZColour::Standard(3), &scheme), Color::Rgb(10, 20, 30));
        assert_eq!(resolve_zcolour(ZColour::Default, &scheme), Color::Reset);
        assert_eq!(resolve_zcolour(ZColour::Standard(11), &scheme), Color::Rgb(0x80, 0x80, 0x80));
        assert_eq!(resolve_zcolour(ZColour::True(0x7FFF), &scheme), Color::Rgb(255, 255, 255));
    }
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p app resolve_zcolour 2>&1 | tail -20` → FAIL.

- [ ] **Step 3: Implement** in `render/mod.rs`:

```rust
use zvm::screen::ZColour;

fn rgb15_to_888(v: u16) -> (u8, u8, u8) {
    let exp = |c: u16| -> u8 { ((c << 3) | (c >> 2)) as u8 };
    (exp(v & 0x1F), exp((v >> 5) & 0x1F), exp((v >> 10) & 0x1F))
}

pub(crate) fn resolve_zcolour(c: ZColour, scheme: &ColorScheme) -> Color {
    match c {
        ZColour::Default => Color::Reset,
        ZColour::Standard(n @ 2..=9) => scheme.palette[(n - 2) as usize],
        ZColour::Standard(10) => Color::Rgb(0xB0, 0xB0, 0xB0),
        ZColour::Standard(11) => Color::Rgb(0x80, 0x80, 0x80),
        ZColour::Standard(_) => Color::Rgb(0x50, 0x50, 0x50), // 12 (and any stray)
        ZColour::True(v) => { let (r, g, b) = rgb15_to_888(v); Color::Rgb(r, g, b) }
    }
}
```

(Confirm the `Color`/`ColorScheme` imports exist in `mod.rs`; add `use crate::colors::ColorScheme;` if needed.)

- [ ] **Step 4: Run to verify pass** — `cargo test -p app resolve_zcolour 2>&1 | tail -15` → PASS.

- [ ] **Step 5: Commit** (`feat(app): ZColour to ratatui Color resolver via scheme palette`).

---

## Task 10: app lower-window (transcript) colour

**Files:**
- Modify: `crates/app/src/session.rs` — `CaptureSink.runs` element type, `take_styled`, `clamp_runs`, `TurnResult.transcript_runs`, `print_attr` override.
- Modify: `crates/app/src/state.rs` — `StyleRun` packed fg/bg; `push_transcript_runs` param + body.
- Modify: `crates/app/src/render/transcript.rs` — resolve run colour when rendering (gating added in Task 12).

**Interfaces:**
- Run element becomes `(usize, u8, ZColour, ZColour)` end-to-end; `StyleRun` gains `fg: u32, bg: u32` (packed, `#[serde(default)]`).
- Produces: `pack_zcolour(ZColour) -> u32` / `unpack_zcolour(u32) -> ZColour` in `state.rs`.

- [ ] **Step 1: Write the failing test** (in `state.rs`):

```rust
    #[test]
    fn pack_roundtrip_and_run_carries_colour() {
        use zvm::screen::ZColour;
        for c in [ZColour::Default, ZColour::Standard(3), ZColour::Standard(12), ZColour::True(0x1234)] {
            assert_eq!(unpack_zcolour(pack_zcolour(c)), c);
        }
        let mut s = AppState::new_for_test(); // use the existing test ctor the other tests use
        s.push_transcript_runs("ab", TranscriptKind::Story,
            &[(2, 0x02, ZColour::Standard(3), ZColour::Default)]);
        let run = /* fetch the last line's first StyleRun */ s.last_style_run_for_test();
        assert_eq!(unpack_zcolour(run.fg), ZColour::Standard(3));
    }
```

(Adapt the state-fetch to however the existing `push_transcript_runs` tests inspect lines — mirror `push_transcript_runs_*` tests at state.rs:1728+.)

- [ ] **Step 2: Run to verify it fails** — `cargo test -p app pack_roundtrip 2>&1 | tail -20` → FAIL.

- [ ] **Step 3: Implement.**
- In `state.rs`, add packing + extend `StyleRun`:

```rust
pub fn pack_zcolour(c: zvm::screen::ZColour) -> u32 {
    use zvm::screen::ZColour;
    match c {
        ZColour::Default => 0,
        ZColour::Standard(n) => (1 << 24) | n as u32,
        ZColour::True(v) => (2 << 24) | v as u32,
    }
}
pub fn unpack_zcolour(p: u32) -> zvm::screen::ZColour {
    use zvm::screen::ZColour;
    match p >> 24 {
        1 => ZColour::Standard((p & 0xFF) as u8),
        2 => ZColour::True((p & 0xFFFF) as u16),
        _ => ZColour::Default,
    }
}
```

```rust
pub struct StyleRun {
    pub start: usize,
    pub end: usize,
    pub bits: u8,
    #[serde(default)]
    pub fg: u32,
    #[serde(default)]
    pub bg: u32,
}
```

- Change `push_transcript_runs` signature to `chunks: &[(usize, u8, zvm::screen::ZColour, zvm::screen::ZColour)]`; when building each `StyleRun`, set `fg: pack_zcolour(fg), bg: pack_zcolour(bg)`. Update the empty-text guard and the existing call-site tests (state.rs:1728-1761) to the new 4-tuple (plain text → `ZColour::Default` for both).
- In `session.rs`: change `CaptureSink.runs` to `Vec<(usize, u8, ZColour, ZColour)>`; `print` pushes `(.., 0, ZColour::Default, ZColour::Default)`; add `print_attr` pushing `(chars, attrs.style, attrs.fg, attrs.bg)` (and keep `print_styled` pushing default colour, or delegate). Update `take_styled`, `clamp_runs`, and `TurnResult.transcript_runs` to the 4-tuple. Update the turn-drain code that feeds `push_transcript_runs`.

- [ ] **Step 4: Run to verify pass** — `cargo build --workspace --tests 2>&1 | grep -c warning` → `0`; `cargo test -p app 2>&1 | tail -15` → pass.

- [ ] **Step 5: Commit** (`feat(app): thread game colour through the transcript run model`).

---

## Task 11: app upper-window colour render + reverse-swap

**Files:**
- Modify: `crates/app/src/render/upper_window.rs` (draw_grid — applies `apply_text_style`; add colour resolve + reverse-swap)

**Interfaces:**
- Consumes: `resolve_zcolour` (Task 9), `Cell.fg/bg` (Task 1).

- [ ] **Step 1: Write the failing test** (in `upper_window.rs` tests):

```rust
    #[test]
    fn upper_cell_colour_resolves_and_reverse_swaps() {
        use zvm::screen::{ZColour, Cell};
        let mut scheme = ColorScheme::default();
        scheme.palette[1] = Color::Rgb(200, 0, 0);   // red
        scheme.palette[4] = Color::Rgb(0, 0, 200);   // blue
        // no reverse: fg=red, bg=blue
        let s = cell_style(Cell { ch: 'x', style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) }, &scheme);
        assert_eq!(s.fg, Some(Color::Rgb(200, 0, 0)));
        assert_eq!(s.bg, Some(Color::Rgb(0, 0, 200)));
        // reverse: swap fg/bg
        let r = cell_style(Cell { ch: 'x', style: 0x01, fg: ZColour::Standard(3), bg: ZColour::Standard(6) }, &scheme);
        assert_eq!(r.fg, Some(Color::Rgb(0, 0, 200)));
        assert_eq!(r.bg, Some(Color::Rgb(200, 0, 0)));
    }
```

(Introduce a small pure helper `cell_style(cell, scheme) -> Style` so the swap logic is unit-testable; `draw_grid` calls it per cell. Adapt assertions to ratatui's `Style` field access.)

- [ ] **Step 2: Run to verify it fails** — `cargo test -p app upper_cell_colour 2>&1 | tail -20` → FAIL.

- [ ] **Step 3: Implement.** Add to `upper_window.rs`:

```rust
fn cell_style(cell: zvm::screen::Cell, scheme: &ColorScheme) -> Style {
    let mut fg = crate::render::resolve_zcolour(cell.fg, scheme);
    let mut bg = crate::render::resolve_zcolour(cell.bg, scheme);
    if cell.style & 0x01 != 0 {
        std::mem::swap(&mut fg, &mut bg); // reverse video swaps colour
    }
    // bold/italic via the existing helper (pass the non-reverse style bits so
    // we don't ALSO apply the REVERSED modifier — the swap already did it)
    let base = Style::default().fg(fg).bg(bg);
    crate::render::apply_text_style(base, cell.style & !0x01)
}
```

Replace the per-cell style construction in `draw_grid` with `cell_style(cell, scheme)`. **Pick one reverse mechanism:** the swap above — so remove any `apply_text_style` REVERSED handling for these cells (pass `style & !0x01`). Verify the existing upper-window cursor reverse-toggle still XORs correctly on top (it toggles REVERSED on the cursor cell; with colour, that path should also swap — keep its behavior consistent by toggling bit 0x01 into `cell.style` before calling `cell_style`).

- [ ] **Step 4: Run to verify pass** — `cargo test -p app upper 2>&1 | tail -15` → pass; manually confirm no double-swap.

- [ ] **Step 5: Commit** (`feat(app): render upper-window game colour with reverse-swap`).

---

## Task 12: app config toggle + session hook + render gating + F2 toggle

**Files:**
- Modify: `crates/app/src/config.rs` (struct ~284, Default ~399) — `honor_game_colours: bool` default true.
- Modify: `crates/app/src/session.rs:151-152` — call `machine.set_honor_game_colours(cfg.honor_game_colours)`.
- Modify: `crates/app/src/render/upper_window.rs` + `render/transcript.rs` — when the flag is OFF, skip colour resolution (fall back to `Color::Reset` fg/bg = today's look).
- Modify: the F2 settings modal to surface the toggle (follow the existing scalar-toggle pattern).

**Interfaces:**
- Consumes: `Machine::set_honor_game_colours` (Task 6), `resolve_zcolour` (Task 9).

- [ ] **Step 1: Write the failing test** (in `config.rs`):

```rust
#[test]
fn honor_game_colours_defaults_true() {
    let c = Config::default();
    assert!(c.honor_game_colours);
    // round-trips through TOML
    let toml = toml::to_string(&c).unwrap();
    let back: Config = toml::from_str(&toml).unwrap();
    assert!(back.honor_game_colours);
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p app honor_game_colours_defaults_true 2>&1 | tail -15` → FAIL (field missing).

- [ ] **Step 3: Implement.**
- Add `fn default_honor_game_colours() -> bool { true }` and the field:

```rust
    #[serde(default = "default_honor_game_colours")]
    pub honor_game_colours: bool,
```

with `honor_game_colours: default_honor_game_colours(),` in `Config::default()`.
- In `session.rs` after `Machine::with_output`, call `machine.set_honor_game_colours(...)`. `GameSession::new` takes `story: Vec<u8>` today — thread the flag in (add a parameter or read it from a `Config` the session already holds; match how other config values reach the session). Default callers pass `cfg.honor_game_colours`.
- Gate rendering: in `cell_style` (Task 11) and the transcript run resolver (Task 10), when the flag is off, return `Color::Reset` for both channels (skip `resolve_zcolour`). Pass the flag into the render functions via the `ColorScheme`/render context already threaded there (or a bool param).
- Add the F2 settings row (scalar bool toggle) per the existing settings-modal pattern; on save it writes `honor_game_colours` via the format-preserving config writer.

- [ ] **Step 4: Run to verify pass** — `cargo build --workspace --tests 2>&1 | grep -c warning` → `0`; `cargo test --workspace 2>&1 | grep "test result:" | grep -v "0 failed"` → empty (all green).

- [ ] **Step 5: Commit** (`feat(app): honor_game_colours config + F2 toggle, default on`).

---

## Self-Review

**1. Spec coverage:**
- set_colour sentinels → Task 2. set_true_colour sentinels (+ EXT:0x0D fix) → Task 3. ✓
- ZColour model + Cell/ScreenState fields → Task 1. ✓
- Output seam (TextAttrs/print_attr) → Task 5. ✓
- Upper-window capture → Task 4; render + reverse-swap → Task 11. ✓
- Lower-window/transcript colour → Task 10. ✓
- Palette mapping (2–9 → scheme.palette; 10–12 grey RGB; true RGB) → resolver Task 9 + CLI Task 7. ✓
- Header colour-bit gating consistent with rendering → Task 6 (advertise) + Task 12 (render gate). ✓
- Config `honor_game_colours` default true, CLI flag, F2 toggle → Tasks 8, 12. ✓
- Reverse swaps fg/bg once → Task 11 (app), Task 7 (CLI emits SGR 7, no pre-swap). ✓
- Zero-dep zvm (ZColour no serde; app packs u32) → Global Constraints + Task 10. ✓
- Greys in scope as fixed RGB → Tasks 7, 9. ✓

**2. Placeholder scan:** No TBD/“handle edge cases”; each code step shows code. A few steps say “match the existing helper/pattern” (test ctors, EXT encoding, settings-modal row) — these point at concrete existing code the implementer must mirror, not invented behavior.

**3. Type consistency:** `ZColour { Default, Standard(u8), True(u16) }` used identically across tasks. `TextAttrs { style, fg, bg }` consistent (Task 5 → 7, 8, 10). `resolve_zcolour(ZColour, &ColorScheme) -> Color` consistent (Task 9 → 11, 12). Run tuple `(usize, u8, ZColour, ZColour)` consistent (Task 10). `pack_zcolour`/`unpack_zcolour` consistent (Task 10). `init_header_caps(mem, bool)` + `advertise_colour` + `set_honor_game_colours` consistent (Task 6 → 8, 12).

**Known follow-ups (out of scope here):** gvm-cli + Glulx/Glk colour (sub-project 2); v6 `set_colour -1` and transparent backgrounds.
