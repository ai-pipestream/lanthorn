# Sectioned Debug Trace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A multi-section debug trace (`screen` / `map` / `hostio`) toggleable via a `--trace <list>` CLI flag and a `/trace <list>` command, written to one tagged `~/.babelmap/trace.log`.

**Architecture:** Buffer-drain. The zero-dep VM crates (`zvm`, `gvm`) accumulate display-instruction lines into a `screen_trace: Vec<String>` gated by a `trace_screen: bool`, drained each turn by the app through a new `Engine::take_screen_trace()`. The `map` section reuses the *already-existing* `render_traced` stage labels + the app's `render_steps` worker buffer. The app owns section state (`Config.trace: TraceSections`), a `trace` module of best-effort file functions, and the `hostio` emit sites. Full design: `docs/superpowers/specs/2026-07-18-sectioned-trace-design.md`.

**Tech Stack:** Rust workspace — crates `zvm`, `gvm`, `mapper` (VM/map, no new deps), `app` (TUI, clap).

## Global Constraints

- `zvm` and `gvm` stay **zero-dependency**; `mapper` gains **no new deps**; trace is `std`-only everywhere.
- **Determinism:** no wall-clock timestamps, no RNG. Trace output reproducible for a given run+input.
- **Cross-platform:** paths via `PathBuf`/`Path::join`; no shell assumptions.
- Real `diagnostics` keep flowing to the transcript as `Warning` lines — the trace is a **separate** channel (`screen_trace`), never coupled to `diagnostics`.
- Section tag width is **8** columns: `[screen] `, `[map]    `, `[hostio] ` (bracket + name padded so the following text aligns).
- Never `git add -A`/`-u`; stage explicitly by path. Commit trailers on every commit:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- The reusable `gvm` decoders + hook already exist verbatim in `scratchpad/glk-trace-prototype.patch` (the backed-out prototype). Task 4 extracts them.

## File Structure

- `crates/app/src/trace.rs` — **new**: `Section`, `TraceSections`, `parse`, `truncate`, `write`.
- `crates/app/src/engine.rs` — Engine trait: add `set_trace_screen` / `take_screen_trace` defaults.
- `crates/zvm/src/cpu/exec.rs` — `Machine`: `trace_screen` + `screen_trace`; hook screen opcodes; decoders.
- `crates/app/src/session.rs` — `GameSession` Engine overrides.
- `crates/gvm/src/exec.rs` — `Machine`: `trace_screen` + `screen_trace`; glk decoders + dispatch hook (from patch).
- `crates/app/src/glulx_session.rs` — `GlulxSession` Engine overrides.
- `crates/app/src/config.rs` — `Cli.trace` + `Config.trace` + `resolve`.
- `crates/app/src/slash.rs` + `slash_dispatch.rs` — `/trace` command.
- `crates/app/src/startup.rs` + `turn.rs` — TraceLog lifecycle + per-turn screen drain.
- `crates/app/src/state.rs` — map-section routing from `render_steps`.
- `crates/app/src/{turn.rs,slash_dispatch.rs,main.rs}` — `hostio` emit sites.

---

### Task 1: `trace` module — sections, parsing, file I/O

**Files:**
- Create: `crates/app/src/trace.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod trace;` — match the existing `pub mod` ordering)
- Test: inline `#[cfg(test)] mod tests` in `trace.rs`

**Interfaces:**
- Produces:
  - `pub enum Section { Screen, Map, HostIo }` with `pub fn tag(self) -> &'static str` → `"screen"|"map"|"hostio"`.
  - `#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)] pub struct TraceSections { pub screen: bool, pub map: bool, pub hostio: bool }`
    - `pub fn any(self) -> bool`
    - `pub fn active_list(self) -> String` → `"screen,map"` or `"off"`.
    - `pub fn parse(s: &str) -> (TraceSections, Vec<String>)` — returns the set + unknown tokens.
  - `pub fn truncate(user_dir: &std::path::Path)` — start a fresh `trace.log`.
  - `pub fn write(user_dir: &std::path::Path, section: Section, lines: &[String])` — best-effort tagged append.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_comma_list_all_none_and_unknowns() {
        let (s, unknown) = TraceSections::parse("screen,map");
        assert!(s.screen && s.map && !s.hostio);
        assert!(unknown.is_empty());

        let (s, _) = TraceSections::parse("all");
        assert!(s.screen && s.map && s.hostio);

        let (s, _) = TraceSections::parse("none");
        assert!(!s.any());

        let (s, unknown) = TraceSections::parse(" screen , bogus ");
        assert!(s.screen && !s.map);
        assert_eq!(unknown, vec!["bogus".to_string()]);

        assert_eq!(TraceSections::default().active_list(), "off");
        assert_eq!(
            TraceSections { screen: true, map: true, hostio: false }.active_list(),
            "screen,map"
        );
    }

    #[test]
    fn write_tags_and_aligns_truncate_resets() {
        let dir = std::env::temp_dir().join(format!("babelmap-trace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("trace.log");
        let _ = std::fs::remove_file(&log);

        write(&dir, Section::Screen, &["@split_window(1)".to_string()]);
        write(&dir, Section::Map, &["detect chains".to_string()]);
        let body = std::fs::read_to_string(&log).unwrap();
        assert!(body.contains("[screen] @split_window(1)"));
        assert!(body.contains("[map]    detect chains"), "map tag padded to width 8: {body:?}");

        truncate(&dir);
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "", "truncate starts fresh");
        std::fs::remove_dir_all(&dir).ok();
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test -p app --lib trace::tests`
Expected: FAIL (module/functions not defined).

- [ ] **Step 3: Implement `trace.rs`**

```rust
//! Multi-section debug trace (SQ-0403 follow-up). Best-effort, std-only.
//! Sections are toggled via `--trace <list>` and `/trace <list>`; output goes
//! to `<user_dir>/trace.log`, one `[section] message` line per event.

use std::io::Write as _;
use std::path::Path;

/// A trace section — the origin/kind of a traced event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    /// Story→interpreter display instructions (Glk calls / Z-machine screen opcodes).
    Screen,
    /// Automapper pipeline stages.
    Map,
    /// Host-side save/restore, VFS file I/O, input/events.
    HostIo,
}

impl Section {
    pub fn tag(self) -> &'static str {
        match self {
            Section::Screen => "screen",
            Section::Map => "map",
            Section::HostIo => "hostio",
        }
    }
}

/// Which trace sections are active. Runtime state, set from `--trace`/`/trace`.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct TraceSections {
    pub screen: bool,
    pub map: bool,
    pub hostio: bool,
}

impl TraceSections {
    pub fn any(self) -> bool {
        self.screen || self.map || self.hostio
    }

    /// The active sections as a comma list (`"screen,map"`), or `"off"`.
    pub fn active_list(self) -> String {
        let mut v = Vec::new();
        if self.screen { v.push("screen"); }
        if self.map { v.push("map"); }
        if self.hostio { v.push("hostio"); }
        if v.is_empty() { "off".to_string() } else { v.join(",") }
    }

    /// Parse a comma-separated section list. `all`/`none` are keywords. Returns
    /// the resulting set plus any unrecognised tokens (for the caller to report).
    pub fn parse(s: &str) -> (TraceSections, Vec<String>) {
        let mut out = TraceSections::default();
        let mut unknown = Vec::new();
        for tok in s.split(',') {
            match tok.trim() {
                "" => {}
                "all" => out = TraceSections { screen: true, map: true, hostio: true },
                "none" | "off" => out = TraceSections::default(),
                "screen" => out.screen = true,
                "map" => out.map = true,
                "hostio" => out.hostio = true,
                other => unknown.push(other.to_string()),
            }
        }
        (out, unknown)
    }
}

fn log_path(user_dir: &Path) -> std::path::PathBuf {
    user_dir.join("trace.log")
}

/// Start a fresh trace log (best-effort). Used at boot so each run stands alone.
pub fn truncate(user_dir: &Path) {
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(log_path(user_dir));
}

/// Append `lines` tagged with `section` (best-effort; a failed open is skipped).
pub fn write(user_dir: &Path, section: Section, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(user_dir))
    else {
        return;
    };
    // `[screen] ` = 9 chars; pad the tag so message columns align.
    let tag = format!("[{}]", section.tag());
    for line in lines {
        let _ = writeln!(f, "{tag:<8} {line}");
    }
}
```

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo test -p app --lib trace::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/trace.rs crates/app/src/lib.rs
git commit -m "feat(app): trace module — sections, parsing, tagged log I/O

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: `Engine` trait screen-trace seam

**Files:**
- Modify: `crates/app/src/engine.rs` (trait `Engine`, ~`engine.rs:412`; add defaults next to `window_dump` at ~`engine.rs:469`)
- Test: inline test in `engine.rs` using an existing minimal test double, or assert the default via `NotZmachineEngine` (`engine_helpers.rs:216`).

**Interfaces:**
- Produces (on `trait Engine`):
  - `fn set_trace_screen(&mut self, _on: bool) {}` — default no-op.
  - `fn take_screen_trace(&mut self) -> Vec<String> { Vec::new() }` — default empty.

- [ ] **Step 1: Write the failing test**

Add to `engine.rs` tests (or wherever `NotZmachineEngine` is constructed):

```rust
#[test]
fn engine_default_screen_trace_is_empty_and_toggle_is_noop() {
    let mut e = crate::engine_helpers::tests_support_not_zmachine_engine(); // or however the double is built
    e.set_trace_screen(true); // default no-op must not panic
    assert!(e.take_screen_trace().is_empty());
}
```
(If no ready constructor exists, instead add the assertion inside an existing `NotZmachineEngine` test; the point is only that the defaults compile and return empty.)

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p app --lib engine`
Expected: FAIL (methods not defined) — or a compile error, which also counts.

- [ ] **Step 3: Add the trait defaults**

In `crates/app/src/engine.rs`, next to `fn window_dump(&self) -> Vec<String>` (~469):

```rust
    /// Enable/disable the `screen` trace on this engine's VM (default: no-op for
    /// engines without a Glk/screen model, e.g. Scott). (trace feature)
    fn set_trace_screen(&mut self, _on: bool) {}

    /// Drain any accumulated `screen`-trace lines (display instructions the story
    /// issued this turn). Default empty; zvm/gvm sessions override. (trace feature)
    fn take_screen_trace(&mut self) -> Vec<String> {
        Vec::new()
    }
```

- [ ] **Step 4: Run test + full app build, verify pass**

Run: `cargo test -p app --lib engine && cargo build -p app`
Expected: PASS; `ScottSession`, test doubles compile via defaults.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/engine.rs
git commit -m "feat(app): Engine::set_trace_screen/take_screen_trace seam

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: zvm Z-machine screen-opcode trace + GameSession wiring

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — `Machine` struct (`exec.rs:120`, fields to ~182), init literal (`exec.rs:212-236`, `diagnostics: Vec::new()` at 228); opcode arms in `exec_var` (`exec.rs:853`), `exec_2op` (`exec.rs:381`), `exec_ext` (`exec.rs:1319`); new decoder helpers.
- Modify: `crates/app/src/session.rs` — `impl Engine for GameSession` (`session.rs:866`).
- Test: inline in `exec.rs`; one in `session.rs`.

**Interfaces:**
- Consumes: `Engine::set_trace_screen`/`take_screen_trace` (Task 2); `ZColour`, `decode_set_colour` (`exec.rs:2237`), `decode_true_colour` (`exec.rs:2248`) from `crate::screen`.
- Produces on `zvm::cpu::exec::Machine`: `pub trace_screen: bool`, `pub screen_trace: Vec<String>`.

**Traced opcodes** (arm → line):
| Opcode | Arm | Line |
|---|---|---|
| split_window | exec_var `0x0A` @1043 | `@split_window(<rows>)` |
| set_window | exec_var `0x0B` @1053 | `@set_window(<lower|upper>)` |
| erase_window | exec_var `0x0D` @1063 | `@erase_window(<lower|upper|all|all(unsplit)>)` |
| set_cursor | exec_var `0x0F` @1086 | `@set_cursor(row=<r>, col=<c>)` |
| set_text_style | exec_var `0x11` @1098 | `@set_text_style(<roman|reverse\|bold\|…>)` |
| buffer_mode | exec_var `0x12` @1108 | `@buffer_mode(<on|off>)` |
| set_colour | exec_2op `0x1B` @568 | `@set_colour(fg=<c>, bg=<c>)` |
| set_font | exec_ext `0x04` @1416 | `@set_font(<normal|graphics|fixed|query|N>)` |
| set_true_colour | exec_ext `0x0D` @1462 | `@set_true_colour(fg=<c>, bg=<c>)` |

**erase_line (VAR:238 / 0x0E):** has NO dedicated arm — it is recognized/handled elsewhere (tests `exec.rs:5214`/`5225` assert it clears-to-end-of-row in the upper window and warns nowhere). **Locate the site that handles 0x0E and add a guarded trace push `@erase_line(<value>)` there** — do NOT add a shadowing `0x0E` arm in `exec_var` that would bypass the existing behavior. If the handling is a generic fall-through with no clear seam, emit the trace at the VAR-dispatch entry for `opcode & 0x1F == 0x0E` before the fall-through, and add a code comment noting why. This is the one investigation point in this task.

- [ ] **Step 1: Write the failing test** (in `exec.rs` tests)

```rust
#[test]
fn screen_trace_records_decoded_display_opcodes_when_enabled() {
    // A tiny routine: set_colour(fg=std5, bg=std2), set_text_style(reverse|bold),
    // split_window(1). Assemble via the crate's existing test helpers.
    let mut m = /* build a Machine over a program issuing those three opcodes,
                   using the same asm/test harness other exec.rs tests use */;
    m.trace_screen = true;
    /* run the program to quit */;
    assert!(m.screen_trace.iter().any(|l| l.starts_with("@set_colour(")), "{:?}", m.screen_trace);
    assert!(m.screen_trace.iter().any(|l| l.contains("@set_text_style(") && (l.contains("reverse") )));
    assert!(m.screen_trace.iter().any(|l| l == "@split_window(1)"));

    // Disabled → nothing accumulates.
    let mut m2 = /* same program */;
    m2.trace_screen = false;
    /* run */;
    assert!(m2.screen_trace.is_empty());
}
```
(Use the same program-assembly pattern as neighbouring `exec.rs` screen tests such as those at `exec.rs:5214`/`5225`. Keep the routine minimal.)

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p zvm --lib screen_trace_records_decoded`
Expected: FAIL (fields/behavior absent).

- [ ] **Step 3: Add fields + decoders + hooks**

Struct (`exec.rs`, in the field block ~120-182):
```rust
    /// When true, screen-control opcodes push a decoded line into `screen_trace`
    /// (the `screen` debug section). Separate from `diagnostics`. (trace feature)
    pub trace_screen: bool,
    /// Accumulated `screen`-trace lines since the host last drained them.
    pub screen_trace: Vec<String>,
```
Init literal (near `diagnostics: Vec::new()` @228):
```rust
            trace_screen: false,
            screen_trace: Vec::new(),
```
Decoders (free fns near `decode_set_colour` ~`exec.rs:2237`):
```rust
fn zscreen_window_name(v: u16) -> String {
    match v as i16 {
        0 => "lower".into(), 1 => "upper".into(),
        -1 => "all(unsplit)".into(), -2 => "all".into(),
        other => format!("win{other}"),
    }
}
fn zscreen_style_name(bits: u16) -> String {
    if bits == 0 { return "roman".into(); }
    let mut p = Vec::new();
    if bits & 1 != 0 { p.push("reverse"); }
    if bits & 2 != 0 { p.push("bold"); }
    if bits & 4 != 0 { p.push("italic"); }
    if bits & 8 != 0 { p.push("fixed"); }
    if p.is_empty() { format!("0x{bits:x}") } else { p.join("|") }
}
fn zscreen_font_name(v: u16) -> String {
    match v { 0 => "query".into(), 1 => "normal".into(), 3 => "graphics".into(), 4 => "fixed".into(), n => format!("font{n}") }
}
fn zscreen_colour_name(c: crate::screen::ZColour) -> String {
    use crate::screen::ZColour::*;
    match c {
        Default => "default".into(),
        Standard(n) => format!("std{n}"),
        True(v) => format!("true(0x{v:04x})"),
        True24(rgb) => format!("#{:06X}", rgb & 0x00FF_FFFF),
    }
}
```
Hooks — one guarded push per arm, e.g.:
```rust
// exec_var 0x0A (split_window):
if self.trace_screen { self.screen_trace.push(format!("@split_window({})", ops[0])); }
// 0x0B set_window:
if self.trace_screen { self.screen_trace.push(format!("@set_window({})", zscreen_window_name(ops[0]))); }
// 0x0D erase_window (ops[0] is i16-valued):
if self.trace_screen { self.screen_trace.push(format!("@erase_window({})", zscreen_window_name(ops[0]))); }
// 0x0F set_cursor:
if self.trace_screen { self.screen_trace.push(format!("@set_cursor(row={}, col={})", ops[0], ops[1])); }
// 0x11 set_text_style:
if self.trace_screen { self.screen_trace.push(format!("@set_text_style({})", zscreen_style_name(ops[0]))); }
// 0x12 buffer_mode:
if self.trace_screen { self.screen_trace.push(format!("@buffer_mode({})", if ops[0] != 0 { "on" } else { "off" })); }
```
```rust
// exec_2op 0x1B set_colour (a=fg, b=bg):
if self.trace_screen {
    let fg = decode_set_colour(a).map(zscreen_colour_name).unwrap_or_else(|| a.to_string());
    let bg = decode_set_colour(b).map(zscreen_colour_name).unwrap_or_else(|| b.to_string());
    self.screen_trace.push(format!("@set_colour(fg={fg}, bg={bg})"));
}
```
```rust
// exec_ext 0x04 set_font:
if self.trace_screen { self.screen_trace.push(format!("@set_font({})", zscreen_font_name(ops[0]))); }
// exec_ext 0x0D set_true_colour (ops[0]=fg, ops[1]=bg):
if self.trace_screen {
    let fg = decode_true_colour(ops[0]).map(zscreen_colour_name).unwrap_or_else(|| ops[0].to_string());
    let bg = decode_true_colour(ops[1]).map(zscreen_colour_name).unwrap_or_else(|| ops[1].to_string());
    self.screen_trace.push(format!("@set_true_colour(fg={fg}, bg={bg})"));
}
```
Place each push at the START of its arm (before the arm mutates/consumes `ops`/`a`/`b`). Then handle `erase_line` per the note above.

- [ ] **Step 4: Run test + build, verify pass**

Run: `cargo test -p zvm --lib screen_trace && cargo build -p zvm`
Expected: PASS.

- [ ] **Step 5: Wire `GameSession` (app)**

In `crates/app/src/session.rs`, inside `impl Engine for GameSession` (`session.rs:866`):
```rust
    fn set_trace_screen(&mut self, on: bool) {
        self.machine.trace_screen = on;
    }
    fn take_screen_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.machine.screen_trace)
    }
```

- [ ] **Step 6: Test GameSession drain**

Add to `session.rs` tests (mirror an existing `GameSession` submit test):
```rust
#[test]
fn game_session_take_screen_trace_drains_when_enabled() {
    let mut s = /* build a GameSession over a story that on its first turn sets a colour */;
    s.set_trace_screen(true);
    let _ = s.submit("look");
    let lines = s.take_screen_trace();
    assert!(lines.iter().any(|l| l.starts_with("@")), "{lines:?}");
    assert!(s.take_screen_trace().is_empty(), "second drain is empty");
}
```
(If a convenient story fixture that issues screen opcodes on turn one isn't handy, assert the plumbing instead: set trace on, push a line directly onto `s.machine.screen_trace`, and verify `take_screen_trace` drains it and a second call is empty.)

- [ ] **Step 7: Run + commit**

Run: `cargo test -p zvm -p app --lib screen_trace game_session_take_screen_trace`
```bash
git add crates/zvm/src/cpu/exec.rs crates/app/src/session.rs
git commit -m "feat(zvm,app): trace Z-machine screen-control opcodes (screen section)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: gvm Glulx Glk trace + GlulxSession wiring (from prototype patch)

**Files:**
- Modify: `crates/gvm/src/exec.rs` — `Machine` struct + init; glk decoders (7 fns) + `is_glk_text_io`; dispatch hook in `glk_dispatch`.
- Modify: `crates/app/src/glulx_session.rs` — `impl Engine for GlulxSession` (`glulx_session.rs:752`).
- Reference: `scratchpad/glk-trace-prototype.patch` (contains the exact code).

**Interfaces:**
- Produces on `gvm::exec::Machine`: `pub trace_screen: bool`, `pub screen_trace: Vec<String>`.
- The prototype's decoders (`glk_selector_name`, `glk_wintype_name`, `glk_style_name`, `glk_hint_name`, `glk_color_hex`, `glk_trace_args`, `is_glk_text_io`) — copy **verbatim** from the patch.

**Extraction rules (the patch used `glk_trace`/`diagnostics`; this task uses `trace_screen`/`screen_trace`):**
1. Add fields `trace_screen: bool` + `screen_trace: Vec<String>` to `Machine` (NOT `glk_trace`), init both.
2. Copy the 7 decoder fns + `is_glk_text_io` verbatim from the patch (they are pure).
3. Dispatch hook at the top of `glk_dispatch` (`exec.rs:~3209` in the patch), but push to **`self.screen_trace`** (not `diagnostics`) and gate on **`self.trace_screen`**:
   ```rust
   if self.trace_screen && !is_glk_text_io(selector) {
       self.screen_trace.push(format!("{}({})", glk_selector_name(selector), glk_trace_args(selector, args)));
   }
   ```
   (Note: drop the `[glk] ` prefix the prototype used — the app adds the `[screen]` tag on write.)
4. Copy the two decoder tests (`glk_trace_names_structural_selectors_and_skips_text_io`, `glk_trace_args_decodes_colour_and_style_calls`) verbatim from the patch.
5. Do **not** re-apply the patch's `graphics_window_clear` hunk or its test — already committed in `de46e4f4`.

- [ ] **Step 1: Add a populate test (in `exec.rs` tests)**

```rust
#[test]
fn screen_trace_records_glk_calls_when_enabled_and_skips_text_io() {
    let mut m = super::tests::machine_with_glk(&[]);
    m.trace_screen = true;
    m.glk_dispatch(0x0023, &[0, 0, 3]).ok();   // glk_window_open (structural → traced)
    m.glk_dispatch(0x0080, &[b'x' as u32]).ok(); // glk_put_char (text I/O → skipped)
    assert!(m.screen_trace.iter().any(|l| l.starts_with("glk_window_open(")), "{:?}", m.screen_trace);
    assert!(!m.screen_trace.iter().any(|l| l.starts_with("glk_put_char")), "text I/O skipped");

    let mut off = super::tests::machine_with_glk(&[]);
    off.trace_screen = false;
    off.glk_dispatch(0x0023, &[0, 0, 3]).ok();
    assert!(off.screen_trace.is_empty());
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p gvm --lib screen_trace_records_glk_calls`
Expected: FAIL.

- [ ] **Step 3: Apply the extraction (fields, decoders, hook, decoder tests)** per the rules above.

- [ ] **Step 4: Run gvm tests, verify pass**

Run: `cargo test -p gvm --lib screen_trace glk_trace_names glk_trace_args`
Expected: PASS.

- [ ] **Step 5: Wire `GlulxSession`**

In `crates/app/src/glulx_session.rs`, `impl Engine for GlulxSession` (~752):
```rust
    fn set_trace_screen(&mut self, on: bool) {
        self.machine.trace_screen = on;
    }
    fn take_screen_trace(&mut self) -> Vec<String> {
        std::mem::take(&mut self.machine.screen_trace)
    }
```

- [ ] **Step 6: Run + commit**

Run: `cargo test -p gvm -p app --lib`
```bash
git add crates/gvm/src/exec.rs crates/app/src/glulx_session.rs
git commit -m "feat(gvm,app): trace Glulx Glk/garglk calls (screen section)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: `--trace` CLI flag + Config wiring

**Files:**
- Modify: `crates/app/src/config.rs` — `Cli` (add field before the `}` at `config.rs:200`); `Config` (skip-field cluster `config.rs:484-495`); `Config::default()` (`config.rs:533-535`); `resolve` (mapping block `config.rs:618-620`); five test `Cli { .. }` literals at `config.rs:820,836,853,1180,1197`.

**Interfaces:**
- Consumes: `crate::trace::TraceSections` (Task 1).
- Produces: `Cli.trace: Option<String>`; `Config.trace: TraceSections`.

- [ ] **Step 1: Write the failing test** (in `config.rs` tests)

```rust
#[test]
fn resolve_parses_trace_flag() {
    let mut cli = /* one of the existing test Cli literals */;
    cli.trace = Some("screen,map".to_string());
    let cfg = resolve(&cli);
    assert!(cfg.trace.screen && cfg.trace.map && !cfg.trace.hostio);
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p app --lib resolve_parses_trace_flag`
Expected: FAIL.

- [ ] **Step 3: Add fields + resolve mapping**

`Cli` (before `config.rs:200` `}`):
```rust
    /// Debug trace sections to enable from boot: comma list of screen,map,hostio
    /// (or `all`/`none`). Output goes to <user_dir>/trace.log. (trace feature)
    #[arg(long, value_name = "LIST")]
    pub trace: Option<String>,
```
`Config` (in the `#[serde(skip)]` cluster ~494):
```rust
    /// Active debug-trace sections. Runtime-only (from --trace / /trace); not persisted.
    #[serde(skip)]
    pub trace: crate::trace::TraceSections,
```
`Config::default()` (~533): `trace: crate::trace::TraceSections::default(),`
`resolve` (after `cfg.images = ...` at ~620):
```rust
    if let Some(list) = &cli.trace {
        let (sections, unknown) = crate::trace::TraceSections::parse(list);
        cfg.trace = sections;
        for u in unknown {
            eprintln!("warning: unknown --trace section '{u}' (valid: screen, map, hostio, all, none)");
        }
    }
```
Add `trace: None,` to each of the five test `Cli` literals (config.rs:820,836,853,1180,1197).

- [ ] **Step 4: Run test + full config tests, verify pass**

Run: `cargo test -p app --lib config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/config.rs
git commit -m "feat(app): --trace CLI flag → Config.trace sections

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: `/trace` command

**Files:**
- Modify: `crates/app/src/slash.rs` — `SlashOutcome` (`slash.rs:32-84`, add variant); `COMMANDS` (registry array, add entry; pattern: `save-state` at `slash.rs:157`); the `COMMANDS.len()` assertion (`slash.rs:735`, `58` → `59`).
- Modify: `crates/app/src/slash_dispatch.rs` — new arm in `dispatch_slash_outcome` (`slash_dispatch.rs:46`).

**Interfaces:**
- Consumes: `Config.trace` (Task 5), `Engine::set_trace_screen` (Task 2), `crate::trace::TraceSections`.
- Produces: `SlashOutcome::Trace(Option<String>)` — `None` = show state; `Some(list)` = set.

- [ ] **Step 1: Failing test** (registry — `slash.rs` tests)

```rust
#[test]
fn trace_command_parses_set_and_show() {
    let set = crate::slash::parse("trace screen,map"); // match the crate's parse entrypoint
    assert!(matches!(set, /* Ok/variant wrapping */ SlashOutcome::Trace(Some(ref s)) if s == "screen,map"));
    let show = crate::slash::parse("trace");
    assert!(matches!(show, SlashOutcome::Trace(None)));
}
```
(Use whatever `slash` parse function the other command tests use; adjust the wrapper matching accordingly.)

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p app --lib trace_command_parses`
Expected: FAIL.

- [ ] **Step 3: Add the variant + registry entry**

`SlashOutcome` (end of enum ~`slash.rs:83`):
```rust
    /// Toggle debug-trace sections: `None` shows current state; `Some(list)` sets
    /// the active set (comma list of screen,map,hostio / all / none). (trace feature)
    Trace(Option<String>),
```
`COMMANDS` entry (place near `dump-windows`, category `Category::Help`, `Context::Global`):
```rust
    CommandSpec {
        name: "trace", category: Category::Help, context: Context::Global,
        usage: "trace [sections|all|none]",
        description: "toggle debug-trace sections (screen, map, hostio) written to trace.log; no arg shows current state",
        dispatch: |a| SlashOutcome::Trace(a.first().map(|s| s.to_string())),
    },
```
Bump the `COMMANDS.len()` assertion at `slash.rs:735` from `58` to `59`.

- [ ] **Step 4: Add the dispatch arm** (`slash_dispatch.rs`, in the `match outcome`)

```rust
        SlashOutcome::Trace(arg) => {
            match arg {
                None => {
                    state.set_status(format!("[trace: {}]", state.config.trace.active_list()));
                }
                Some(list) => {
                    let (sections, unknown) = crate::trace::TraceSections::parse(&list);
                    state.config.trace = sections;
                    session.set_trace_screen(sections.screen);
                    if sections.any() && !unknown.is_empty() {
                        state.set_status(format!("[trace: {} — ignored: {}]", sections.active_list(), unknown.join(",")));
                    } else if !unknown.is_empty() {
                        state.set_status(format!("[trace: unknown section(s): {}]", unknown.join(",")));
                    } else {
                        state.set_status(format!("[trace: {}]", sections.active_list()));
                    }
                }
            }
        }
```
(`map`/`hostio` need no engine call — they're read from `state.config.trace` at their emit/routing sites in Tasks 7-9.)

- [ ] **Step 5: Run tests, verify pass**

Run: `cargo test -p app --lib slash`
Expected: PASS (including the `COMMANDS.len()` test at 59).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/slash.rs crates/app/src/slash_dispatch.rs
git commit -m "feat(app): /trace command — set/show trace sections at runtime

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: TraceLog lifecycle + screen drain (boot + per-turn)

**Files:**
- Modify: `crates/app/src/startup.rs` — after `game_dir`/session are known (session creation `startup.rs:191-253`); after the seed turn apply (`startup.rs:470`).
- Modify: `crates/app/src/turn.rs` — `finish_command_turn` (`turn.rs:33`), draining after `apply_turn_events` (`turn.rs:83`).
- Test: a focused helper test in `turn.rs`.

**Interfaces:**
- Consumes: `crate::trace::{truncate, write, Section}`, `Config.trace`, `Engine::{set_trace_screen, take_screen_trace}`, `state.config.user_dir`.
- Produces: `pub(crate) fn flush_screen_trace(user_dir: &Path, session: &mut dyn Engine, on: bool)` in `turn.rs` (reused at boot + per turn).

- [ ] **Step 1: Failing test** (`turn.rs` tests)

```rust
#[test]
fn flush_screen_trace_writes_when_on_and_drains() {
    let dir = std::env::temp_dir().join(format!("bm-flush-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut eng = /* a test Engine whose take_screen_trace returns one line once */;
    flush_screen_trace(&dir, &mut eng, true);
    let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
    assert!(body.contains("[screen] "), "{body:?}");
    // off → no write, and buffer still drained (no growth)
    flush_screen_trace(&dir, &mut eng, false);
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run, verify fail** — `cargo test -p app --lib flush_screen_trace` → FAIL.

- [ ] **Step 3: Implement the helper** (`turn.rs`)

```rust
/// Drain the engine's `screen` trace and, when `on`, append it to trace.log.
/// Always drains (so the buffer never grows while the section is off between a
/// runtime toggle). (trace feature)
pub(crate) fn flush_screen_trace(user_dir: &std::path::Path, session: &mut dyn crate::engine::Engine, on: bool) {
    let lines = session.take_screen_trace();
    if on {
        crate::trace::write(user_dir, crate::trace::Section::Screen, &lines);
    }
}
```

- [ ] **Step 4: Wire boot** (`startup.rs`)

After the session is created and `state.config` is available (following the session `match` ~`startup.rs:253`, before or at the banner apply):
```rust
    if state.config.trace.any() {
        crate::trace::truncate(&state.config.user_dir);
    }
    session.set_trace_screen(state.config.trace.screen);
```
After the seed turn apply (`apply_turn(&mut mapper, "", &seed_result)` at `startup.rs:470`):
```rust
    crate::turn::flush_screen_trace(&state.config.user_dir, &mut *session, state.config.trace.screen);
    if state.config.trace.any() {
        let ptr = format!("[trace → {}: {}]",
            state.config.user_dir.join("trace.log").display(),
            state.config.trace.active_list());
        state.push_transcript_internal(&ptr, app::state::TranscriptKind::Meta);
    }
```
(Confirm the exact `session` binding name/scope at that point; it is the boxed `Box<dyn Engine>` from `BootResult`.)

- [ ] **Step 5: Wire per-turn** (`turn.rs::finish_command_turn`, after `apply_turn_events` at `turn.rs:83`)

```rust
    flush_screen_trace(&state.config.user_dir, session, state.config.trace.screen);
```
(Match `finish_command_turn`'s actual `state`/`session` parameter names.)

- [ ] **Step 6: Run + build + commit**

Run: `cargo test -p app --lib flush_screen_trace && cargo build -p app`
```bash
git add crates/app/src/startup.rs crates/app/src/turn.rs
git commit -m "feat(app): screen-trace flush at boot and each turn + log lifecycle

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 8: `map` section routing from `render_steps`

**Files:**
- Modify: `crates/app/src/state.rs` — `poll_render_job` (`state.rs:2280`), which installs a completed render; `render_steps` (`state.rs:1392`), `render_steps_snapshot` (`state.rs:2328`).
- Test: focused test on the routing helper.

**Interfaces:**
- Consumes: `crate::trace::{write, Section}`, `Config.trace.map`, `render_steps_snapshot()`.
- The render worker already pushes stage labels (`detect chains`, `place rooms`, `route edges`, `route lanes`) into `render_steps` (`state.rs:2271`). No mapper change required for v1.

- [ ] **Step 1: Failing test** (`state.rs` tests)

```rust
#[test]
fn map_trace_routes_render_steps_to_log_only_when_on() {
    let dir = std::env::temp_dir().join(format!("bm-maptrace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let steps = vec!["detect chains".to_string(), "route lanes".to_string()];
    // helper under test:
    write_map_trace(&dir, &steps, /* on = */ true);
    let body = std::fs::read_to_string(dir.join("trace.log")).unwrap_or_default();
    assert!(body.contains("[map]    detect chains"), "{body:?}");

    let dir2 = std::env::temp_dir().join(format!("bm-maptrace-off-{}", std::process::id()));
    std::fs::create_dir_all(&dir2).unwrap();
    write_map_trace(&dir2, &steps, false);
    assert!(!dir2.join("trace.log").exists() || std::fs::read_to_string(dir2.join("trace.log")).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok(); std::fs::remove_dir_all(&dir2).ok();
}
```

- [ ] **Step 2: Run, verify fail** — `cargo test -p app --lib map_trace_routes` → FAIL.

- [ ] **Step 3: Implement the routing helper + call site**

Helper (in `state.rs` or `turn.rs`, wherever `poll_render_job` can reach it):
```rust
/// Append this render pass's pipeline stage labels to trace.log when `on`. (trace feature)
pub(crate) fn write_map_trace(user_dir: &std::path::Path, steps: &[String], on: bool) {
    if on {
        crate::trace::write(user_dir, crate::trace::Section::Map, steps);
    }
}
```
Call site in `poll_render_job` (`state.rs:2280`), at the point a job is confirmed installed (gen still matches — NOT on the stale-discard path):
```rust
    // (inside poll_render_job, right after the completed RenderMap is installed)
    if self.config.trace.map {
        let steps = self.render_steps_snapshot();
        crate::state::write_map_trace(&self.config.user_dir, &steps, true);
    }
```
(Adjust `self.config`/`self.render_steps_snapshot()` to the real field/method receiver in `state.rs`. Only route on successful install so stale-discarded passes don't emit.)

- [ ] **Step 4: Run + build + commit**

Run: `cargo test -p app --lib map_trace_routes && cargo build -p app`
```bash
git add crates/app/src/state.rs
git commit -m "feat(app): route render-pipeline stage labels to trace.log (map section)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 9: `hostio` emit sites

**Files:**
- Modify: `crates/app/src/turn.rs` — VFS write (`persist_vfs_after_turn`, `turn.rs:358-367`).
- Modify: `crates/app/src/startup.rs` — VFS boot read (`startup.rs:143`).
- Modify: `crates/app/src/slash_dispatch.rs` — save (`SlashOutcome::Save` `slash_dispatch.rs:124`) + restore (`SlashOutcome::Load` `slash_dispatch.rs:177`).
- Modify: `crates/app/src/main.rs` — line submit (`main.rs:1992`) + key submit (`main.rs:1544`).
- Test: focused formatter test.

**Interfaces:**
- Consumes: `crate::trace::{write, Section}`, `Config.trace.hostio`, `state.config.user_dir`.
- Produces: `pub(crate) fn hostio(user_dir, on, line: String)` convenience in `trace.rs`:
  ```rust
  pub fn hostio(user_dir: &std::path::Path, on: bool, line: String) {
      if on { write(user_dir, Section::HostIo, &[line]); }
  }
  ```

- [ ] **Step 1: Failing test** (`trace.rs` tests)

```rust
#[test]
fn hostio_writes_only_when_on() {
    let dir = std::env::temp_dir().join(format!("bm-hostio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    hostio(&dir, false, "save_state(auto, 1024 bytes)".to_string());
    assert!(!dir.join("trace.log").exists() || std::fs::read_to_string(dir.join("trace.log")).unwrap().is_empty());
    hostio(&dir, true, "save_state(auto, 1024 bytes)".to_string());
    assert!(std::fs::read_to_string(dir.join("trace.log")).unwrap().contains("[hostio] save_state(auto, 1024 bytes)"));
    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 2: Run, verify fail** — `cargo test -p app --lib hostio_writes_only_when_on` → FAIL.

- [ ] **Step 3: Add the `hostio` helper to `trace.rs`** (code above).

- [ ] **Step 4: Add emit calls at each site** (all gated on `…config.trace.hostio`):

- VFS write (`turn.rs::persist_vfs_after_turn`, after the sidecar write ~`turn.rs:358-367`):
  ```rust
  crate::trace::hostio(&state.config.user_dir, state.config.trace.hostio,
      format!("vfs_write({} bytes)", bytes.len()));
  ```
- VFS boot read (`startup.rs:143`, after `read_vfs`):
  ```rust
  crate::trace::hostio(&cfg.user_dir, cfg.trace.hostio,
      format!("vfs_read({} bytes)", vfs_sidecar.len()));
  ```
  (use the actual bound variable names for the config + the read bytes).
- Save (`slash_dispatch.rs`, `SlashOutcome::Save` ~124, after the snapshot is produced):
  ```rust
  crate::trace::hostio(&state.config.user_dir, state.config.trace.hostio,
      format!("save_state({} bytes)", snapshot_len));
  ```
- Restore (`slash_dispatch.rs`, `SlashOutcome::Load` ~177, after `restore_from_file`):
  ```rust
  crate::trace::hostio(&state.config.user_dir, state.config.trace.hostio,
      format!("restore_state({})", path.display()));
  ```
- Line submit (`main.rs:1992`, before/after `session.submit(&cmd)`):
  ```rust
  crate::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("input_line({cmd:?})"));
  ```
- Key submit (`main.rs:1544`, where a key is delivered):
  ```rust
  crate::trace::hostio(&state.config.user_dir, state.config.trace.hostio, format!("input_key({ki:?})"));
  ```
  (Use the real key/config bindings in scope; keep each call one line.)

- [ ] **Step 5: Run + build + commit**

Run: `cargo test -p app --lib trace && cargo build -p app`
```bash
git add crates/app/src/trace.rs crates/app/src/turn.rs crates/app/src/startup.rs crates/app/src/slash_dispatch.rs crates/app/src/main.rs
git commit -m "feat(app): hostio trace at save/restore, VFS, and input sites

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Final verification (after all tasks)

- [ ] `cargo test -p zvm -p gvm -p app` — all green.
- [ ] `cargo clippy -p zvm -p gvm -p app -p gvm-cli` — clean.
- [ ] Manual smoke (user): `babelmap <zstory>.z5 --trace screen,map`, play a turn, confirm `~/.babelmap/trace.log` shows tagged `[screen] @…` (Z-machine) and `[map] …` lines; then `babelmap cm.gblorb --trace screen` shows `[screen] glk_…` lines; `/trace hostio` then a save shows `[hostio] save_state(...)`; `/trace none` stops output; `/trace` shows current state.
- [ ] Update `README.md` if the trace flag warrants a mention (per project policy: major features only — a debug flag likely does not; note in side-quest instead).

## Self-Review notes

- **Spec coverage:** screen (Task 3 zvm + Task 4 gvm), map (Task 8), hostio (Task 9), CLI+runtime same-grammar control (Tasks 5-6), one tagged log (Task 1), buffer-drain via Engine seam (Task 2), boot capture (Task 7). All covered.
- **Ordering limitation** (turn-granular, subsystem order) is inherent to the drain points in Task 7/8 — no task promises global ordering.
- **erase_line** is the one under-specified opcode (Task 3 note) — implementer must locate its handling site.
- **Type consistency:** `trace_screen: bool` + `screen_trace: Vec<String>` identical in both `zvm` and `gvm`; `Engine::{set_trace_screen, take_screen_trace}` signatures identical across Tasks 2/3/4; `TraceSections`/`Section`/`trace::{write,truncate,hostio}` defined in Task 1 and consumed unchanged thereafter.
