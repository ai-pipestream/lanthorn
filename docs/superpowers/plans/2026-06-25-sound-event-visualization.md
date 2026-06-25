# Sound & Diagnostic Event Visualization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface VM `sound_effect` beeps as a one-shot themeable pulse of the story-pane border (high #1 / low #2), and route unimplemented-opcode warnings to the meta transcript instead of stderr.

**Architecture:** The VM records events into two drainable queues on `Machine` (`pending_beeps`, `diagnostics`). The session drains them into `TurnResult` (`beep`, `diagnostics`). The app routes diagnostics to the meta transcript and a beep to a timed story-border color override, reusing the per-frame pulse pattern that `tidy_job` uses for the map border. The zvm-cli drains diagnostics to stderr to preserve current behavior.

**Tech Stack:** Rust workspace (crates `zvm`, `zvm-cli`, `app`); ratatui 0.29 / crossterm 0.28.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-06-25-sound-event-visualization-design.md`.
- ZMSD §9.4 sound numbering: **#1 = high-pitched bleep, #2 = low-pitched bleep**; numbers ≥ 3 are sampled sounds (unsupported — record a diagnostic).
- The engine NEVER prints; consumers decide presentation. Replace the `eprintln!` opcode warning with a `diagnostics` queue entry, keeping warn-once-per-opcode dedup (`warned_var_opcodes`).
- Default beep colors: high = `Color::Rgb(255, 180, 40)` (amber); low = `Color::Rgb(60, 140, 220)` (cyan-blue). Themeable via `sound_beep_high` / `sound_beep_low` selectors. These are signal colors — use the same fixed defaults in BOTH `ColorScheme` constructors (do not derive from the palette).
- Pulse: one-shot fade, `SOUND_PULSE_MS = 500`. Story border only (never the map border — that is the tidy pulse's; they must not contend).
- Commit message trailers (zsh — no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Run suites with `cargo test -p <crate> <filter>`. Keep `cargo build -p app` warning-free.

---

### Task 1: VM — `sound_effect` opcode, event queues, diagnostics rerouting

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — add `Beep` enum, `pending_beeps`/`diagnostics` fields + inits, the `0x15` arm, reroute the fallthrough, repoint one test.

**Interfaces:**
- Produces:
  - `pub enum Beep { High, Low }` (with `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`) at module scope in `crates/zvm/src/cpu/exec.rs`.
  - `pub pending_beeps: Vec<Beep>` and `pub diagnostics: Vec<String>` fields on `Machine`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/zvm/src/cpu/exec.rs` (near `unimplemented_var_opcode_is_warned_once`):

```rust
#[test]
fn sound_effect_records_high_and_low_beeps() {
    let mut m = build_test_machine(&[]);
    // number 1 = high bleep
    m.exec_var(0x15, &[1], None, None);
    assert_eq!(m.pending_beeps, vec![Beep::High]);
    // number 2 = low bleep
    m.exec_var(0x15, &[2], None, None);
    assert_eq!(m.pending_beeps, vec![Beep::High, Beep::Low]);
    // no diagnostics for plain bleeps
    assert!(m.diagnostics.is_empty(), "bleeps must not record diagnostics");
}

#[test]
fn sound_effect_sampled_sound_records_diagnostic_no_beep() {
    let mut m = build_test_machine(&[]);
    m.exec_var(0x15, &[3], None, None); // sampled sound -> needs Blorb
    assert!(m.pending_beeps.is_empty(), "sampled sound is not a bleep");
    assert_eq!(m.diagnostics.len(), 1, "sampled sound records one diagnostic");
    assert!(m.diagnostics[0].contains("sampled sound"));
}

#[test]
fn sound_effect_zero_records_nothing() {
    let mut m = build_test_machine(&[]);
    m.exec_var(0x15, &[0], None, None);
    assert!(m.pending_beeps.is_empty());
    assert!(m.diagnostics.is_empty());
}

#[test]
fn unimplemented_var_opcode_records_diagnostic_not_stderr() {
    let mut m = build_test_machine(&[]);
    // 0x14 has no arm in exec_var -> hits the unimplemented fallthrough.
    assert!(m.diagnostics.is_empty());
    m.exec_var(0x14, &[], None, None);
    assert_eq!(m.diagnostics.len(), 1, "fallthrough records one diagnostic line");
    assert!(m.diagnostics[0].contains("0x14"), "diagnostic names the opcode");
    m.exec_var(0x14, &[], None, None); // second call must not duplicate
    assert_eq!(m.diagnostics.len(), 1, "warn-once: no duplicate diagnostic");
}
```

Then REPOINT the existing `unimplemented_var_opcode_is_warned_once` test: it uses `0x15`, now implemented. Change both `m.exec_var(0x15, ...)` calls to `0x14` and update its comment from `0x15 sound_effect` to `0x14 is an undefined/unimplemented VAR opcode`. (Keep its `warned_var_opcodes` assertions — the dedup set is unchanged.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p zvm sound_effect 2>&1 | tail -20`
Expected: compile error (`Beep`, `pending_beeps`, `diagnostics` undefined).

- [ ] **Step 3: Add the `Beep` enum and the two `Machine` fields**

Add the enum just above `pub enum StepResult {` (around line 26) in `crates/zvm/src/cpu/exec.rs`:

```rust
/// A built-in Z-machine bleep recorded by `sound_effect` (ZMSD §9.4):
/// sound #1 is the high-pitched bleep, #2 the low-pitched bleep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beep {
    High,
    Low,
}
```

Add the fields to the `Machine` struct, right after the `warned_var_opcodes` field (around line 86):

```rust
    /// Bleeps recorded by `sound_effect` since the host last drained them.
    pub pending_beeps: Vec<Beep>,
    /// Host-facing diagnostic lines (e.g. unimplemented opcodes, sampled sounds)
    /// recorded since the host last drained them. The engine never prints.
    pub diagnostics: Vec<String>,
```

Initialise them in `Machine::new` (the struct literal around line 125, after `warned_var_opcodes: ...`):

```rust
            pending_beeps: Vec::new(),
            diagnostics: Vec::new(),
```

- [ ] **Step 4: Implement the `0x15` arm and reroute the fallthrough**

In `exec_var` (around line 954), add a new arm immediately before the `_ =>` fallthrough:

```rust
            // 0x15 sound_effect — number effect volume routine (ZMSD §9.4).
            // We render bleeps visually (host shows a border pulse); sampled
            // sounds (number >= 3) need a Blorb resource we do not yet load.
            // effect/volume/routine are accepted but unused until audio lands.
            0x15 => {
                let number = ops.first().copied().unwrap_or(0);
                match number {
                    1 => self.pending_beeps.push(Beep::High),
                    2 => self.pending_beeps.push(Beep::Low),
                    0 => {} // not a defined bleep
                    n => {
                        if self.warned_var_opcodes.insert(0x15) {
                            self.diagnostics.push(format!(
                                "sampled sound #{n} not supported (needs Blorb)"
                            ));
                        }
                    }
                }
                StepResult::Continue
            }
```

Replace the fallthrough body (around line 955) — change the `eprintln!` to a `diagnostics` push:

```rust
            // Unknown / unimplemented VAR opcode: record once, then ignore.
            _ => {
                if self.warned_var_opcodes.insert(opcode) {
                    self.diagnostics.push(format!(
                        "unimplemented VAR opcode 0x{opcode:02X} (ignored)"
                    ));
                }
                StepResult::Continue
            }
```

Note: the sampled-sound branch reuses the `warned_var_opcodes` set keyed on `0x15` so repeated sampled-sound calls do not spam the queue (matching the one-diagnostic assertion in Step 1).

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p zvm sound_effect 2>&1 | tail -20` → all pass.
Run: `cargo test -p zvm unimplemented_var_opcode 2>&1 | tail -20` → both pass.
Run: `cargo test -p zvm 2>&1 | tail -5` → full zvm suite green.

- [ ] **Step 6: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs
git commit -m "feat(zvm): implement sound_effect; record beeps + diagnostics instead of stderr

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: zvm-cli — drain diagnostics to stderr

**Files:**
- Modify: `crates/zvm-cli/src/main.rs:116-118` — drain `machine.diagnostics` each loop iteration.

**Interfaces:**
- Consumes: `Machine::diagnostics` (Task 1).

- [ ] **Step 1: Implement the drain in the host loop**

In `crates/zvm-cli/src/main.rs`, change the loop head from:

```rust
    loop {
        match machine.step() {
            StepResult::Continue => {}
```

to:

```rust
    loop {
        let step = machine.step();
        for d in machine.diagnostics.drain(..) {
            eprintln!("zvm: warning: {d}");
        }
        match step {
            StepResult::Continue => {}
```

(The closing `}` of the loop body and all other arms are unchanged — only the `match machine.step()` head becomes `match step`.)

- [ ] **Step 2: Build to verify it compiles**

Run: `cargo build -p zvm-cli 2>&1 | tail -5`
Expected: builds, no warnings.

- [ ] **Step 3: Smoke-test that warnings still appear**

Run (a v3 story with no sound is fine — this just confirms the binary runs and the drain path compiles end-to-end):

```bash
printf 'quit\ny\n' | cargo run -q -p zvm-cli -- stories/*.z3 2>&1 | tail -5 || true
```

Expected: the game runs and exits (no panic). Any `zvm: warning:` lines now come from the drain, not the old `eprintln!`.

- [ ] **Step 4: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit -m "feat(zvm-cli): drain VM diagnostics to stderr each step

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: session — surface beep + diagnostics in `TurnResult`

**Files:**
- Modify: `crates/app/src/session.rs` — add `beep`/`diagnostics` to `TurnResult`; drain in `submit` and `submit_char`.

**Interfaces:**
- Consumes: `zvm::cpu::exec::Beep`, `Machine::pending_beeps`, `Machine::diagnostics` (Task 1).
- Produces: `TurnResult { ..., beep: Option<Beep>, diagnostics: Vec<String> }`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/session.rs`. Use the same story-loading helper the existing `submit_char_returns_turn_result_and_advances` test uses (read that test first to mirror its setup, e.g. `GameSession::new(<story bytes>)`). The new test only needs to assert the new fields exist and default empty on a turn that emits no sound:

```rust
#[test]
fn turn_result_has_empty_sound_fields_by_default() {
    // Reuse whatever story-bytes helper the sibling submit tests use.
    let mut sess = make_test_session();           // <- match the existing helper name
    let r = sess.submit("look");
    assert!(r.beep.is_none(), "no beep when the game emits no sound");
    assert!(r.diagnostics.is_empty(), "no diagnostics on a clean turn");
    // VM queues are drained after the turn.
    assert!(sess.machine.pending_beeps.is_empty());
    assert!(sess.machine.diagnostics.is_empty());
}
```

If the existing tests construct the session inline rather than via a helper, copy that exact construction into this test instead of `make_test_session()`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app turn_result_has_empty_sound_fields 2>&1 | tail -20`
Expected: compile error (`beep`/`diagnostics` not fields of `TurnResult`).

- [ ] **Step 3: Add the fields and drain logic**

In `crates/app/src/session.rs`, add an import near the top (where `Machine` is imported):

```rust
use zvm::cpu::exec::Beep;
```

Extend `TurnResult` (around line 67):

```rust
pub struct TurnResult {
    pub transcript: String,
    pub location: Option<ObjectSnapshot>,
    pub quit: bool,
    /// Optional informational note to surface to the player (e.g. when the
    /// game's own save/restore is auto-failed, hint them toward Ctrl+S/Ctrl+R).
    pub info: Option<String>,
    /// The latest bleep emitted this turn (last wins), if any.
    pub beep: Option<Beep>,
    /// Host-facing diagnostic lines emitted this turn (drained from the VM).
    pub diagnostics: Vec<String>,
}
```

In BOTH `submit` (line 132) and `submit_char` (line 153), replace the trailing `TurnResult { transcript, location, quit, info }` with a version that drains the VM queues. Insert, just before the `TurnResult { ... }` return in each method:

```rust
        let diagnostics = std::mem::take(&mut self.machine.diagnostics);
        let beep = self.machine.pending_beeps.last().copied();
        self.machine.pending_beeps.clear();

        TurnResult { transcript, location, quit, info, beep, diagnostics }
```

(Note: `new()` deliberately does NOT drain — any intro-emitted diagnostics persist in the VM queue and surface on the first `submit`/`submit_char`. This avoids adding a separate intro accessor; the warn-once dedup prevents repeats.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p app turn_result_has_empty_sound_fields 2>&1 | tail -20` → pass.
Run: `cargo test -p app session 2>&1 | tail -10` → session tests green.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/session.rs
git commit -m "feat(app): surface beep + diagnostics from the VM in TurnResult

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: theming — `sound_beep_high` / `sound_beep_low` selectors

**Files:**
- Modify: `crates/app/src/colors.rs` — add two `Style` fields + defaults in both constructors.
- Modify: `crates/app/src/style.rs` — add to `SELECTOR_FIELDS`, `apply_color_decls`, and `write_style_full` export.

**Interfaces:**
- Produces: `ColorScheme { ..., sound_beep_high: Style, sound_beep_low: Style }`, defaulting to amber / cyan-blue fg.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/colors.rs`:

```rust
#[test]
fn sound_beep_defaults_are_amber_and_cyan_blue() {
    let cs = ColorScheme::terminal_default();
    assert_eq!(cs.sound_beep_high.fg, Some(Color::Rgb(255, 180, 40)));
    assert_eq!(cs.sound_beep_low.fg, Some(Color::Rgb(60, 140, 220)));
}
```

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/style.rs`:

```rust
#[test]
fn sound_beep_selectors_parse_and_apply() {
    let doc = parse_style_toml(
        "[colors]\n\"sound_beep_high\" = { fg = \"red\" }\n\"sound_beep_low\" = { fg = \"blue\" }\n"
    ).unwrap();
    let (cs, _set, warnings) = resolve(&doc, std::path::Path::new("."));
    assert!(warnings.is_empty(), "known selectors must not warn: {warnings:?}");
    assert_eq!(cs.sound_beep_high.fg, Some(ratatui::style::Color::Red));
    assert_eq!(cs.sound_beep_low.fg, Some(ratatui::style::Color::Blue));
}
```

(If `resolve` is not in scope in the style test module, mirror the call form used by the sibling `map_border` parse test — e.g. `crate::style::resolve` / the test module's existing helper.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app sound_beep 2>&1 | tail -20`
Expected: compile error (`sound_beep_high` not a field).

- [ ] **Step 3: Add the `ColorScheme` fields and defaults**

In `crates/app/src/colors.rs`, add the fields at the end of the struct (after `virtual_window_border: BorderStyle,`, line 204):

```rust
    /// Border pulse color for the high-pitched bleep (sound_effect #1).
    pub sound_beep_high: Style,
    /// Border pulse color for the low-pitched bleep (sound_effect #2).
    pub sound_beep_low: Style,
```

In `terminal_default()` (after `virtual_window_border: BorderStyle::Single,`, line 256):

```rust
            sound_beep_high: Style::new().fg(Color::Rgb(255, 180, 40)),
            sound_beep_low: Style::new().fg(Color::Rgb(60, 140, 220)),
```

In the `from_ghostty` constructor's struct literal (after `virtual_window_border: BorderStyle::Single,`, line 382) — same fixed signal colors (do NOT derive from the palette):

```rust
            sound_beep_high: Style::new().fg(Color::Rgb(255, 180, 40)),
            sound_beep_low: Style::new().fg(Color::Rgb(60, 140, 220)),
```

- [ ] **Step 4: Wire the selectors in style.rs**

Add to `SELECTOR_FIELDS` (after `"upper_window_border",`, line 142):

```rust
    "sound_beep_high",
    "sound_beep_low",
```

Add to the `apply_color_decls` match (after the `"upper_window_border" => ...` arm — find it near line 200; add alongside the other plain selectors):

```rust
            "sound_beep_high"    => cs.sound_beep_high = cs.sound_beep_high.patch(style),
            "sound_beep_low"     => cs.sound_beep_low = cs.sound_beep_low.patch(style),
```

Add to `write_style_full` export (after the `upper_window_border` block, line 794):

```rust
    doc.colors.selectors.insert("sound_beep_high".to_string(), style_to_decl(&cs.sound_beep_high));
    doc.colors.selectors.insert("sound_beep_low".to_string(),  style_to_decl(&cs.sound_beep_low));
```

(Defaults live in the `ColorScheme` constructors, consistent with other plain color selectors like `suggestion`; no `DEFAULT_STYLE_TOML` entry is needed.)

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p app sound_beep 2>&1 | tail -20` → both pass.
Run: `cargo test -p app style 2>&1 | tail -10` → style suite green (incl. `write_style_full_is_self_contained`).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/style.rs
git commit -m "feat(app): themeable sound_beep_high / sound_beep_low border colors

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: render helper — `sound_pulse_color` (one-shot fade)

**Files:**
- Modify: `crates/app/src/render/map.rs` — add `SOUND_PULSE_MS` + `sound_pulse_color` beside `pulse_border_color` (around line 68).

**Interfaces:**
- Produces:
  - `pub const SOUND_PULSE_MS: u64 = 500;`
  - `pub fn sound_pulse_color(beep: Color, normal: Color, elapsed: std::time::Duration) -> Option<Color>`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/render/map.rs` (near the `pulse_border_color` tests, ~line 4230):

```rust
#[test]
fn sound_pulse_full_color_at_start() {
    let beep = Color::Rgb(255, 180, 40);
    let normal = Color::Rgb(0, 0, 0);
    let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(0));
    assert_eq!(c, Some(Color::Rgb(255, 180, 40)), "elapsed 0 => full beep color");
}

#[test]
fn sound_pulse_fades_toward_normal_partway() {
    let beep = Color::Rgb(200, 0, 0);
    let normal = Color::Rgb(0, 0, 0);
    // Halfway through the window: roughly the midpoint between beep and normal.
    let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(SOUND_PULSE_MS / 2));
    match c {
        Some(Color::Rgb(r, _, _)) => assert!((90..=110).contains(&r), "expected ~100, got {r}"),
        other => panic!("expected an Rgb mid-fade color, got {other:?}"),
    }
}

#[test]
fn sound_pulse_expires_after_window() {
    let beep = Color::Rgb(255, 180, 40);
    let normal = Color::Rgb(0, 0, 0);
    let c = sound_pulse_color(beep, normal, std::time::Duration::from_millis(SOUND_PULSE_MS));
    assert_eq!(c, None, "at/after the window the pulse is over");
}

#[test]
fn sound_pulse_non_rgb_normal_fades_toward_dim_beep() {
    // When the border color is a named/terminal color (no RGB), fade toward a
    // dimmed copy of the beep color instead (spec fallback).
    let beep = Color::Rgb(200, 200, 200);
    let c = sound_pulse_color(beep, Color::Reset, std::time::Duration::from_millis(SOUND_PULSE_MS - 1));
    match c {
        Some(Color::Rgb(r, _, _)) => assert!(r < 200, "must fade below full beep, got {r}"),
        other => panic!("expected an Rgb color, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app sound_pulse 2>&1 | tail -20`
Expected: compile error (`sound_pulse_color` / `SOUND_PULSE_MS` undefined).

- [ ] **Step 3: Implement the helper**

In `crates/app/src/render/map.rs`, immediately after `pulse_border_color` (line 68) add:

```rust
/// Duration of the one-shot story-border flash for a `sound_effect` bleep.
pub const SOUND_PULSE_MS: u64 = 500;

/// Extract RGB channels from a `Color`, or `None` for non-RGB colors
/// (named/indexed/Reset have no fixed RGB to interpolate toward).
fn rgb_of(c: Color) -> Option<(u8, u8, u8)> {
    if let Color::Rgb(r, g, b) = c {
        Some((r, g, b))
    } else {
        None
    }
}

/// One-shot fade for a sound bleep: full `beep` color at `elapsed == 0`, lerping
/// toward `normal` as `elapsed` approaches `SOUND_PULSE_MS`. Returns `None` once
/// the window has elapsed (the caller then clears the pulse and the border
/// renders normally). When `normal` is not an RGB color (e.g. a terminal/named
/// border color), fade toward a dimmed copy of the beep color instead.
pub fn sound_pulse_color(
    beep: Color,
    normal: Color,
    elapsed: std::time::Duration,
) -> Option<Color> {
    let ms = elapsed.as_millis() as u64;
    if ms >= SOUND_PULSE_MS {
        return None;
    }
    let (br, bg, bb) = rgb_of(beep).unwrap_or((255, 180, 40));
    let (nr, ng, nb) = rgb_of(normal).unwrap_or((br / 4, bg / 4, bb / 4));
    let f = ms as f64 / SOUND_PULSE_MS as f64; // 0.0 -> 1.0 across the window
    let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    Some(Color::Rgb(lerp(br, nr), lerp(bg, ng), lerp(bb, nb)))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app sound_pulse 2>&1 | tail -20` → all four pass.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/render/map.rs
git commit -m "feat(app): sound_pulse_color one-shot fade helper for the story border

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: app run loop — set the pulse, push diagnostics, override the story border

**Files:**
- Modify: `crates/app/src/state.rs` — add `SoundPulse` struct + `sound_pulse` field (+ default).
- Modify: `crates/app/src/main.rs` — push diagnostics as `Meta`, set `sound_pulse` on a beep (both turn paths), expire + apply the story-border override, widen the poll condition.

**Interfaces:**
- Consumes: `TurnResult.beep` / `TurnResult.diagnostics` (Task 3); `ColorScheme.sound_beep_high/low` (Task 4); `sound_pulse_color` / `SOUND_PULSE_MS` (Task 5); `zvm::cpu::exec::Beep`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/app/src/state.rs`:

```rust
#[test]
fn sound_pulse_defaults_none_and_holds_kind() {
    use zvm::cpu::exec::Beep;
    let mut s = AppState::default();
    assert!(s.sound_pulse.is_none(), "no pulse by default");
    s.sound_pulse = Some(SoundPulse { kind: Beep::High, started: std::time::Instant::now() });
    assert!(matches!(s.sound_pulse.as_ref().map(|p| p.kind), Some(Beep::High)));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app sound_pulse_defaults_none 2>&1 | tail -20`
Expected: compile error (`SoundPulse` / `sound_pulse` undefined).

- [ ] **Step 3: Add `SoundPulse` state**

In `crates/app/src/state.rs`, add the struct near `TidyJob` (around line 243):

```rust
/// An in-flight one-shot story-border flash triggered by a `sound_effect` bleep.
pub struct SoundPulse {
    pub kind: zvm::cpu::exec::Beep,
    pub started: std::time::Instant,
}
```

Add the field to `AppState` (near `tidy_job`, line 541):

```rust
    pub sound_pulse: Option<SoundPulse>,
```

Initialise it in the `AppState` default (near `tidy_job: None,`, line ~712):

```rust
            sound_pulse: None,
```

- [ ] **Step 4: Add a routing helper in main.rs and call it on both turn paths**

In `crates/app/src/main.rs`, update the imports:
- Extend the `render::map` import (line 35) to include the new symbols:
  `use app::render::map::{pulse_border_color, render_map_layered, room_screen_rects, sound_pulse_color, SOUND_PULSE_MS};`
- Extend the `state` import (line 49) to include `SoundPulse`.

Add a free function near the other run-loop helpers (e.g. just above `fn key_to_zscii`):

```rust
/// Route a turn's sound/diagnostic events: diagnostics become meta transcript
/// lines; the latest beep arms a one-shot story-border pulse.
fn apply_turn_events(state: &mut AppState, result: &TurnResult) {
    for line in &result.diagnostics {
        state.push_transcript_kind(line, app::state::TranscriptKind::Meta);
    }
    if let Some(kind) = result.beep {
        state.sound_pulse = Some(SoundPulse { kind, started: std::time::Instant::now() });
    }
}
```

Call it on the **char path** — in the char-mode gate, right after `state.push_transcript(&result.transcript);` (line 1201):

```rust
                            apply_turn_events(&mut state, &result);
```

Call it on the **line path** — right after `state.push_transcript(&result.transcript);` (line 1503):

```rust
                apply_turn_events(&mut state, &result);
```

(Note: `result.info` handling stays exactly as-is in both paths; `apply_turn_events` only adds the diagnostics + beep routing.)

- [ ] **Step 5: Expire + apply the story-border override in the render section**

In `crates/app/src/main.rs`, find the `map_border_override` computation (line 254). Immediately AFTER it, add the pulse expiry + the story-border color resolution:

```rust
        // Expire a finished sound pulse so the story border returns to normal.
        if let Some(p) = &state.sound_pulse {
            if p.started.elapsed().as_millis() as u64 >= SOUND_PULSE_MS {
                state.sound_pulse = None;
            }
        }
        // Resolve the story-border color: a live sound pulse overrides the fg.
        let story_border_style = {
            let base = state.colors.story_border;
            match &state.sound_pulse {
                Some(p) => {
                    let beep_color = match p.kind {
                        zvm::cpu::exec::Beep::High => state
                            .colors
                            .sound_beep_high
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(255, 180, 40)),
                        zvm::cpu::exec::Beep::Low => state
                            .colors
                            .sound_beep_low
                            .fg
                            .unwrap_or(ratatui::style::Color::Rgb(60, 140, 220)),
                    };
                    let normal = base.fg.unwrap_or(ratatui::style::Color::Reset);
                    match sound_pulse_color(beep_color, normal, p.started.elapsed()) {
                        Some(c) => base.fg(c),
                        None => base,
                    }
                }
                None => base,
            }
        };
```

Then, at the two story-pane `draw_pane_frame` call sites, pass `story_border_style` instead of `state.colors.story_border`:
- `Layout::TranscriptFull` (line 260):
  `let story_frame = draw_pane_frame(buf, main_area, state.colors.story_border_style, story_border_style);`
- `Layout::Split` (line 311):
  `let story_frame = draw_pane_frame(buf, chunks[0], state.colors.story_border_style, story_border_style);`

(The `MapFull` layout draws no story pane, so it needs no change.)

- [ ] **Step 6: Widen the poll condition so the fade animates**

In `crates/app/src/main.rs`, find the poll-interval line (line 819):

```rust
        let poll_ms = if state.tidy_job.is_some() { TIDY_POLL_MS } else { 50 };
```

Change it so an active sound pulse also polls fast (driving the ~500ms fade):

```rust
        let poll_ms = if state.tidy_job.is_some() || state.sound_pulse.is_some() { TIDY_POLL_MS } else { 50 };
```

- [ ] **Step 7: Run the tests + build to verify**

Run: `cargo test -p app sound_pulse_defaults_none 2>&1 | tail -20` → pass.
Run: `cargo build -p app 2>&1 | tail -5` → builds, zero warnings.
Run: `cargo test -p app 2>&1 | tail -5` → full app suite green.

- [ ] **Step 8: Commit**

```bash
git add crates/app/src/state.rs crates/app/src/main.rs
git commit -m "feat(app): pulse the story border on a beep, route diagnostics to meta transcript

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: README — document the sound visualization

**Files:**
- Modify: `README.md` — note the beep border pulse + meta-tagged opcode warnings.

- [ ] **Step 1: Update the Interpreter feature bullet**

In `README.md`, under "### Interpreter (the Z-machine)", add a bullet after the upper-window screen-model bullet (line ~52):

```markdown
- **Sound effects** — the `sound_effect` opcode's two built-in bleeps (high #1 /
  low #2) flash the story-pane border in distinct, themeable colors
  (`sound_beep_high` / `sound_beep_low`); a brief one-shot fade. (Sampled sounds
  need Blorb audio, still on the roadmap.) Unimplemented-opcode warnings surface
  in the transcript as meta lines (hidden by `/filter story`) rather than on
  stderr.
```

- [ ] **Step 2: Verify the doc reads correctly**

Run: `grep -n "sound_beep_high\|Sound effects" README.md`
Expected: the new bullet appears once.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: README — sound_effect bleep border pulse + meta opcode warnings

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Self-Review

**Spec coverage:**
- VM `sound_effect` #1→High / #2→Low, sampled→diagnostic, 0→nothing → Task 1. ✓
- Diagnostics moved off `eprintln!` to a queue, warn-once preserved → Task 1; CLI drains to stderr → Task 2. ✓
- `TurnResult.beep` (last wins) + `diagnostics`, VM queues cleared → Task 3. ✓
- Diagnostics → meta transcript → Task 6. ✓
- Beep → `sound_pulse` → story-border override; expiry clears it → Task 6. ✓
- One-shot fade, `SOUND_PULSE_MS = 500`, non-RGB-normal fallback → Task 5. ✓
- Themeable `sound_beep_high`/`sound_beep_low`, amber/cyan-blue defaults, both constructors, selector apply+export → Task 4. ✓
- Story border only; no map-border contention → Task 6 (only the two story sites changed; `MapFull` untouched). ✓
- Forward note to Blorb audio → already in the spec + TODO; README mentions roadmap → Task 7. ✓
- Repoint the now-stale `unimplemented_var_opcode_is_warned_once` test → Task 1. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases". The one non-literal is the session test's `make_test_session()` helper name, with an explicit instruction to match the sibling tests' existing construction — necessary because the helper name is not yet known without reading that file.

**Type consistency:** `Beep` (zvm) is used identically across session, state, and main. `sound_pulse_color(Color, Color, Duration) -> Option<Color>` matches all call/test sites. `SoundPulse { kind: Beep, started: Instant }` consistent in state + main. `pending_beeps`/`diagnostics` field names consistent across Tasks 1–3.
