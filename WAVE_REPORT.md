# Wave Report — Live Style Reload

Branch: `wave-live-reload` (worktree `.worktrees/live-reload`). No push, no merge.
Commit range: `4b318d1..e5739f9`.

All tasks end green with zero compiler warnings (`cargo build -p app --tests` → 0 warnings).
Final suite: **637 lib + 39 (app integration) + 3 (headless) = 679 tests, 0 failures**.

---

## Task 1 — style.toml is the single styling source
- Commit: `4b318d1` (resolved and committed by the coordinator; not redone here).
- Per coordinator: removed `Config.colors`/`Config.symbols` and `style::style_from_config`;
  both resolve sites + the config-screen save now resolve `style.toml` directly; added
  `watch_style` (default false), `config_has_style_sections`, a `pub config_path(cli)` helper,
  and the startup one-time notice. The config-screen styling editor (colors.scheme + 4 symbol
  rows, their handlers, `ConfigPathField::ColorsScheme`, the orphaned cycle helpers, and the
  unused `Choice` `ConfigRowKind`) was removed.
- Build clean (0 warnings); suite green (634 lib + 39 + 3 at that commit).
- Note: this was the blocker I originally reported (the config-screen styling editor was outside
  the plan's stated blast radius). The coordinator chose "remove the editor" and implemented it.

## Task 2 — reload_style core
- Commit: `b75b147`.
- Test (red→green): `reload::tests::reload_applies_style_file_and_keeps_current_on_error`.
- Result line: `test result: ok. 635 passed; 0 failed; ...` (lib) + 39 + 3.
- Deviation: created `crates/app/src/reload.rs` and registered `pub mod reload;` in `lib.rs` at
  Step 1 (not Step 3) so the failing test actually compiles-and-fails meaningfully (the plan's
  Step 1/Step 3 split would otherwise leave the module unregistered). Implementation is the
  plan's literal code. `colors::expand_path` is `pub(crate)` — fine, `reload.rs` is in-crate.

## Task 3 — /reload command
- Commit: `bd291b6`.
- Test (red→green): `input::tests::reload_action_applies_style_file`.
- Result line: `test result: ok. 636 passed; 0 failed; ...` (lib) + 39 + 3.
- Wiring: `Command::ReloadStyle` mirrored across keymap (`enum`, `to_action`, `name`→`reload_style`,
  `ALL_COMMANDS`), `Action::ReloadStyle` + `apply_action` arm (plan's literal code), `/reload`
  alias in `slash.rs` before the `from_name` fallback.
- Deviation: keymap's `label()` and `context()` are exhaustive `match`es, so adding the variant
  forced arms there too (compiler-driven for exhaustiveness): `label` → `"reload style"`,
  `context` → `Context::Global`. The plan named only enum/name/from_name/to_action.

## Task 4 — File-watch + /watch toggle
- Commit: `0bccad6`.
- Test (red→green): `watch::tests::due_only_after_window` (pure debounce).
- Result line: `test result: ok. 637 passed; 0 failed; ...` (lib) + 39 + 3.
- `notify` version: `notify = "6"` → resolved to **6.1.1**. `Cargo.lock` updated and staged with
  `Cargo.toml`.
- Command wiring: `Command::ToggleWatch` mirrored across keymap (same set as Task 3, plus the
  exhaustive `label`→`"watch style"` / `context`→`Global` arms); `Action::ToggleWatch` with the
  no-op `apply_action` arm; `/watch` alias in `slash.rs`.
- Run-loop wiring adaptations (main.rs, `fn main`) — deviations from the plan's literal snippets:
  - Imports: the plan's `use std::time::{Duration, Instant};` was NOT added. `Duration` is already
    module-imported (line 2) and `Instant` is used fully-qualified elsewhere; I reused `Duration`
    and wrote `std::time::Instant` inline to avoid a redundant-import warning. `TranscriptKind` was
    already in scope (used by existing slash help handling), so the plan's `app::state::TranscriptKind`
    path was written as the in-scope `TranscriptKind`.
  - Watcher init: placed immediately before `loop {` (vars `style_watcher`, `watch_dirty`), per plan.
  - Drain + debounce-reload: placed at the TOP of the loop body (before the existing background-tidy
    block and the `draw_frame` call), NOT at the bottom. Rationale: the loop has many early
    `continue`s (incl. the idle `!event_ready` path that fires every ~50 ms poll tick); a
    bottom-of-loop placement would be skipped on those iterations and the debounced reload would
    never fire while idle. Top placement runs every iteration and applies before the same frame's
    draw.
  - ToggleWatch intercept: the plan's single top-level `if matches!(action, Action::ToggleWatch)`
    intercept does not cover the `/watch` route, because `/watch` arrives as a top-level
    `Action::SubmitCommand` and the `ToggleWatch` action is produced *inside* `slash::parse` and
    dispatched via `SlashOutcome::Action(a) => apply_action(a, ...)`. I therefore handle it in BOTH
    places via a shared free fn `toggle_style_watch(&mut state, &mut style_watcher)`:
    (1) a top-level intercept (covers a direct keybinding / hotkey-dialog route) with `continue`,
    and (2) inside the `SlashOutcome::Action(a)` arm (`if matches!(a, ToggleWatch) { toggle } else
    { apply_action }`). The helper mirrors the plan's start/stop + status-message logic.
- Step 9 manual verification: **DEFERRED** — file-watch cannot be driven headlessly (no graphical
  terminal in this environment). The pure pieces (`due`, `reload_style`, command parsing) are
  unit-tested; the `notify` integration matches the documented wiring and builds clean. Manual
  check of live recolor-on-save / `/reload` / broken-edit-keeps-look was not run.

## Task 5 — Docs
- Commit: `e5739f9`.
- Verify: `style::tests::style_example_toml_parses_and_resolves_clean` → ok (comments don't change
  resolution). Full suite 637 + 39 + 3, 0 warnings.
- Added the live-reload/`watch_style` comment block to `style.example.toml` (after the header) and
  the `/reload` + `watch_style` pointer to the README "Shareable style files" bullet, per the plan's
  literal text. Did not touch the adjacent (now slightly stale) "config.toml sections override
  per-key" line — out of scope for this task and surgical-changes discipline.

---

### Deviation summary
- Task 2: module registered in `lib.rs` at Step 1 (so the red test compiles) rather than Step 3.
- Task 3 & 4: keymap `label()`/`context()` arms added for `ReloadStyle`/`ToggleWatch` (exhaustive
  matches; compiler-required, beyond the enum/name/from_name/to_action the plan listed).
- Task 4: no new `use std::time::...` import (reused module `Duration`, fully-qualified `Instant`);
  used in-scope `TranscriptKind`; watcher drain/debounce placed at loop TOP (early-`continue` safety);
  ToggleWatch intercepted in two sites via a shared `toggle_style_watch` helper because `/watch`
  routes through `SubmitCommand`→`slash::parse`, not a top-level `Action::ToggleWatch`.
- Task 4: `notify "6"` → 6.1.1; `Cargo.lock` staged.
- Task 4 Step 9 manual verification deferred (headless).
