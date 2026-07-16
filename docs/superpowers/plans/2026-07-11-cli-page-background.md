# CLI page-background colour via OSC 11 (gvm-cli + zvm-cli) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Make both CLIs paint the game's page background across the whole terminal window (not just coloured text runs), using OSC 11 to set the terminal's default background — honor-gated and reliably reset on every exit path.

**Architecture:** Per-run text background already renders via SGR in both CLIs. The gap is the *page* background (margins, blank lines, area below content). OSC 11 (`ESC ] 11 ; #rrggbb BEL`) sets the terminal's own default background for the entire window; text with its own bg still overrides locally. Each turn we read the game's current page bg (gvm: the Normal buffer style's bg; zvm: `screen.current_bg`); when it changes we emit OSC 11; on exit we emit OSC 111 to restore the user's terminal.

**Tech Stack:** Rust. No gvm/zvm library changes needed — both already expose the source colour. CLIs use existing crossterm.

## Global Constraints

- `gvm`/`zvm` **library** crates: NO changes required and none should be made — the page-bg source is already public (`Machine::style_colour` / `Machine.screen.current_bg`).
- Gate ALL OSC 11 emission on the existing honor flag (`--no-game-colours` off → never emit): gvm-cli `honor` (already a `drive()` param), zvm-cli `machine.honor_game_colours`.
- OSC 11 changes the user's real terminal background, so it MUST be reset (OSC 111) on EVERY interactive exit path — normal quit, fault/exit(70), and the Ctrl-C/Ctrl-D `process::exit` inside each CLI's `read_line_raw`. A missed path leaves the user's terminal recoloured.
- OSC format (match the app's OSC 52 precedent `crates/app/src/clipboard.rs:104`, BEL-terminated): set = `"\x1b]11;#{:02x}{:02x}{:02x}\x07"`, reset = `"\x1b]111\x07"`.
- Non-TTY (piped) runs must not emit OSC (no terminal to colour). gvm-cli: the drive loop only emits when the real backend is a TTY — gate on the same `stdout_is_tty` the backend uses; zvm-cli: gate on `is_tty`.
- Staging hygiene: stage ONLY edited source files by path; never `git add -A` (tree has pre-existing untracked files).
- Commit trailers on every commit:
  ```
  Quest: SQ-0280
  Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

## Shared component design (both tasks implement their own copy — separate crates, no shared lib)

Each CLI gets three small helpers in its `screen`/`glk_term` module, with unit tests:
```rust
/// OSC 11: set the terminal's default background to `#rrggbb`.
pub fn osc_set_bg((r, g, b): (u8, u8, u8)) -> String {
    format!("\x1b]11;#{r:02x}{g:02x}{b:02x}\x07")
}
/// OSC 111: reset the terminal's default background to the user's default.
pub fn osc_reset_bg() -> &'static str { "\x1b]111\x07" }
/// The escape to emit for a page-bg transition from `prev` to `cur` (both are
/// already honor-resolved: `None` = no game bg / default). `None` return = no change.
pub fn page_bg_escape(cur: Option<(u8, u8, u8)>, prev: Option<(u8, u8, u8)>) -> Option<String> {
    if cur == prev { return None; }
    Some(match cur {
        Some(rgb) => osc_set_bg(rgb),
        None => osc_reset_bg().to_string(),
    })
}
```
Emission pattern in the loop (track `let mut last_page_bg: Option<(u8,u8,u8)> = None;`):
```rust
let cur = if honor && is_tty { page_bg_rgb(machine) } else { None };
if let Some(esc) = page_bg_escape(cur, last_page_bg) {
    print!("{esc}"); let _ = io::stdout().flush(); last_page_bg = cur;
}
```
Exit reset: emit `osc_reset_bg()` once on each exit path when a bg may have been set. On the Ctrl-C hard-exit path (which doesn't know `last_page_bg`), emit it unconditionally (harmless when nothing was set — it just re-asserts the default).

---

### Task 1: gvm-cli page background

**Files:**
- Modify: `crates/gvm-cli/src/glk_term.rs` (OSC helpers + `page_bg_escape` + tests)
- Modify: `crates/gvm-cli/src/main.rs` (emit in `drive` loop; reset on exits)

**Interfaces:**
- Page-bg source: `machine.style_colour(gvm::WinType::TextBuffer, gvm::glk::GlkStyle::Normal).bg` → `Option<u32>` (0xRRGGBB). Convert with the existing `rgb24` (glk_term.rs:75, `fn rgb24(v: u32) -> (u8,u8,u8)`) — make a small `page_bg_rgb(machine) -> Option<(u8,u8,u8)>` helper, or inline `.map(rgb24)`. Confirm the exact `WinType`/`GlkStyle` import paths used elsewhere in gvm-cli (main.rs already `use gvm::{...}`; the app uses `gvm::glk::GlkStyle::Normal` and `WinType::TextBuffer`).

- [ ] **Step 1: Add OSC helpers + tests (glk_term.rs)**

Add `osc_set_bg`, `osc_reset_bg`, `page_bg_escape` (verbatim from the shared design). Add tests in glk_term.rs's `#[cfg(test)]`:
```rust
#[test]
fn osc_set_bg_formats_hex() {
    assert_eq!(super::osc_set_bg((0x12, 0x34, 0x56)), "\x1b]11;#123456\x07");
    assert_eq!(super::osc_set_bg((0, 0, 0)), "\x1b]11;#000000\x07");
}
#[test]
fn page_bg_escape_emits_only_on_change() {
    assert_eq!(super::page_bg_escape(None, None), None);
    assert_eq!(super::page_bg_escape(Some((1,2,3)), None), Some("\x1b]11;#010203\x07".into()));
    assert_eq!(super::page_bg_escape(Some((1,2,3)), Some((1,2,3))), None);
    assert_eq!(super::page_bg_escape(Some((9,9,9)), Some((1,2,3))), Some("\x1b]11;#090909\x07".into()));
    assert_eq!(super::page_bg_escape(None, Some((1,2,3))), Some("\x1b]111\x07".into()));
}
```
(Match how the file's existing tests reference module items — `super::` vs a `use`.)

- [ ] **Step 2: Run tests red→green** — `cargo test -p gvm-cli osc_ page_bg` (implement to green).

- [ ] **Step 3: Emit in the drive loop (main.rs)**

`drive()` already has `machine: &mut Machine` and `honor: bool`. Add a `stdout_is_tty: bool` parameter to `drive()` (the page bg must not be emitted on piped stdout). Inside `drive`, before the loop: `let mut last_page_bg: Option<(u8,u8,u8)> = None;`. At the TOP of the loop body (after the existing vfs-flush block, before `machine.step()`), insert:
```rust
        // Page background: reflect the game's Normal-style bg onto the terminal's
        // default background (OSC 11), honor-gated and TTY-only. Reset on exit.
        let cur_bg = if honor && stdout_is_tty {
            machine.style_colour(gvm::WinType::TextBuffer, gvm::glk::GlkStyle::Normal).bg.map(rgb_from_u32)
        } else { None };
        if let Some(esc) = glk_term::page_bg_escape(cur_bg, last_page_bg) {
            print!("{esc}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            last_page_bg = cur_bg;
        }
```
where `rgb_from_u32` reuses glk_term's `rgb24` (expose it: either `pub fn rgb24` or add `pub fn page_bg_rgb`). Use the file's actual flush idiom. When `drive` breaks on `StepResult::Quit`, before/after the break emit the reset if a bg was set — OR (simpler) do the reset in `main` after `drive` returns (Step 4). Choose Step 4 (single teardown point); do NOT double-reset.

- [ ] **Step 4: Reset on exit paths (main.rs)**

- After `drive(...)` returns (main.rs ~365, alongside `machine.flush()` / `disable_raw_mode()`): `if honor && stdout_is_tty { print!("{}", glk_term::osc_reset_bg()); let _ = std::io::Write::flush(&mut std::io::stdout()); }`. This covers normal quit and the post-drive fault/exit(70) path.
- In `read_line_raw`'s Ctrl-C/Ctrl-D branch (main.rs ~110-118), before `std::process::exit(0)` and after the existing SGR reset / `disable_raw_mode`, add `print!("{}", glk_term::osc_reset_bg());` + flush. Unconditional here (this path can't see `honor`/`last_page_bg`; an OSC 111 when nothing was set is harmless).

- [ ] **Step 5: Wire `stdout_is_tty` into the real `drive()` call + tests**

Real call site (main.rs ~336-360): pass the existing `stdout_is_tty` (the map noted the backend already knows TTY-ness via `stdout_is_tty`/`both_tty` in `main()`; use whichever boolean `main` already computes for stdout — confirm its name). Update the four test `drive(...)` call sites to pass `false` for `stdout_is_tty` (tests are non-TTY; no OSC expected).

- [ ] **Step 6: Verify + commit**

```
cargo test -p gvm-cli
cargo build -p gvm-cli   # warning-free
```
The pure helpers are unit-tested; the terminal effect + exit-reset is manual-smoke only (do NOT add a vacuous TTY test). Commit ONLY `crates/gvm-cli/src/glk_term.rs crates/gvm-cli/src/main.rs`. Subject: `feat(gvm-cli): paint the page background via OSC 11 (SQ-0280)`.

---

### Task 2: zvm-cli page background

**Files:**
- Modify: `crates/zvm-cli/src/screen.rs` (OSC helpers + `page_bg_escape` + a `ZColour`→RGB helper + tests)
- Modify: `crates/zvm-cli/src/main.rs` (emit at the input-poll sites; reset on exits)

**Interfaces:**
- Page-bg source: `machine.screen.current_bg: zvm::screen::ZColour` (public field; `crates/zvm/src/screen.rs:172`). honor: `machine.honor_game_colours` (public field).
- `ZColour` variants (`crates/zvm/src/screen.rs:46-54`): `Default | Standard(u8) | True(u16) | True24(u32)`. RGB helpers in `zvm::screen`: `rgb15_to_888(u16)`, `grey_rgb(u8)`. The full variant→colour resolution today lives only in `push_colour_sgr` (`crates/zvm-cli/src/screen.rs:11-29`, builds SGR params) — MIRROR its match to produce a bare RGB triple.

- [ ] **Step 1: Add a `ZColour`→RGB helper (screen.rs)**

Add `pub fn zcolour_rgb(c: zvm::screen::ZColour) -> Option<(u8,u8,u8)>` returning `None` for `Default` and the resolved 24-bit triple otherwise, EXACTLY mirroring `push_colour_sgr`'s per-variant resolution (`Standard(n)` → its palette entry as that fn does; `True(v)` → `rgb15_to_888(v)`; `True24(v)` → `((v>>16)&0xFF, (v>>8)&0xFF, v&0xFF)`). Read `push_colour_sgr` and reuse the same sources — do not invent a palette.

- [ ] **Step 2: Add OSC helpers + tests (screen.rs)**

Add `osc_set_bg`, `osc_reset_bg`, `page_bg_escape` (verbatim from the shared design). Tests:
```rust
#[test]
fn osc_and_transition() {
    assert_eq!(super::osc_set_bg((0x12,0x34,0x56)), "\x1b]11;#123456\x07");
    assert_eq!(super::page_bg_escape(None, None), None);
    assert_eq!(super::page_bg_escape(Some((1,2,3)), None), Some("\x1b]11;#010203\x07".into()));
    assert_eq!(super::page_bg_escape(Some((1,2,3)), Some((1,2,3))), None);
    assert_eq!(super::page_bg_escape(None, Some((1,2,3))), Some("\x1b]111\x07".into()));
}
#[test]
fn zcolour_rgb_default_is_none_true24_unpacks() {
    assert_eq!(super::zcolour_rgb(zvm::screen::ZColour::Default), None);
    assert_eq!(super::zcolour_rgb(zvm::screen::ZColour::True24(0x123456)), Some((0x12,0x34,0x56)));
}
```

- [ ] **Step 3: Run tests red→green** — `cargo test -p zvm-cli osc zcolour page_bg`.

- [ ] **Step 4: Emit at the input-poll sites (main.rs)**

In `main`, before the play loop: `let mut last_page_bg: Option<(u8,u8,u8)> = None;`. At the NeedLine and NeedChar poll sites (main.rs ~891-893 and ~925-927, right before calling `read_line_raw`/`read_char_input`, alongside the existing `view.frame` print), insert the emission block:
```rust
        let cur_bg = if is_tty && machine.honor_game_colours {
            crate::screen::zcolour_rgb(machine.screen.current_bg)
        } else { None };
        if let Some(esc) = crate::screen::page_bg_escape(cur_bg, last_page_bg) {
            print!("{esc}"); let _ = io::stdout().flush(); last_page_bg = cur_bg;
        }
```
(Use the loop's actual `is_tty` variable name — confirm from main.rs; the map noted `stdin_is_tty`/`stdout_is_tty`/`both_tty` exist. Page bg is a stdout concern → gate on the stdout TTY boolean.)

- [ ] **Step 5: Reset on exit paths (main.rs)**

Emit `crate::screen::osc_reset_bg()` (+ flush) at:
- `StepResult::Quit` (main.rs ~846-852), after `view.leave()` / `disable_raw_mode`, before `break` — gate `if is_tty && machine.honor_game_colours`.
- `StepResult::Fault` (main.rs ~854-864), before `std::process::exit(70)` (~863) — same gate.
- `read_line_raw`'s Ctrl-C/Ctrl-D branch (main.rs ~580-590), before `std::process::exit(0)` (~589), after `leave_region()` — unconditional (path can't see honor/last).

- [ ] **Step 6: Verify + commit**

```
cargo test -p zvm-cli
cargo build -p zvm-cli   # warning-free
```
Commit ONLY `crates/zvm-cli/src/screen.rs crates/zvm-cli/src/main.rs`. Subject: `feat(zvm-cli): paint the page background via OSC 11 (SQ-0280)`.

---

## Manual smoke (both CLIs; TTY effect the tests can't cover)

1. A game that sets a page/background colour (Glulx: a game using `stylehint_BackColor` on Normal; Z-machine: a game that sets background via `@set_colour`/true-colour) fills the WHOLE terminal window with that colour, including margins and the area below the text — not just behind glyphs.
2. `--no-game-colours`: no page background applied (terminal default unchanged).
3. Quit normally → terminal background returns to your default. Ctrl-C and Ctrl-D mid-input → also restored (no stuck recoloured terminal). Trigger a fault if feasible → restored.
4. Piped input (`printf 'look\nquit\n' | <cli> <story>`): no OSC leakage into captured output.
5. A game that changes background mid-play updates the window background at the next turn.

## Self-review checklist

- gvm/zvm libraries unchanged.
- OSC emission honor-gated AND TTY-gated; reset on ALL exit paths (quit, fault, Ctrl-C/Ctrl-D).
- `page_bg_escape` emits only on change (no per-turn OSC spam).
- Pure helpers unit-tested; terminal effect left to the documented smoke (no vacuous TTY test).
- Only the edited source files staged per commit.
