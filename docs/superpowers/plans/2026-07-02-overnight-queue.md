# Overnight Autonomous Queue — 2026-07-02

**Context:** User approved 7 TODO items for unsupervised overnight execution. Confirmed
twice via AskUserQuestion. Base = `main` @ `4adf7f9` (current tip, clean except an
uncommitted user wording edit in TODO.md item MAP#1 and the untracked
`docs/mapping-rules-current.md` — both unrelated, leave them).

## Execution model
- **Parallel background worktree agents**, one per lane (`isolation: "worktree"`,
  `run_in_background: true`). Worktree isolation removes all hot-file collision risk.
- **Do NOT merge to main.** Each lane ships fully green on its own branch for the
  user's morning review.
- **Per-lane ship ritual** (project convention):
  1. Verify base: `git merge-base --is-ancestor 4adf7f9 HEAD` sanity; confirm prior
     work survived (grep for a known symbol). Stale-base hazard is real.
  2. Implement conservatively. If blocked or the only fix is risky, STOP and leave a
     clear report + branch in a safe (green) state rather than force a bad change.
  3. Green gate: `cargo build --tests` + `cargo test` for the touched crate(s) must pass;
     do not break existing tests.
  4. `scripts/todo-done "<unique substring of the TODO line>"` (moves it to COMPLETED.md,
     random `TODO-xxxxxx` id).
  5. Update `README.md` for any user-facing feature (required by project rule).
  6. Commit: `git add TODO.md COMPLETED.md README.md <code>` then `git commit`.
     Hooks live at `.githooks/` (core.hooksPath) — `prepare-commit-msg` auto-adds the
     `Completes: TODO-xxxxxx` trailer, `commit-msg` validates it. End the message with the
     Co-Authored-By / Claude-Session trailers.
- **Review as they land:** when an agent notifies completion, review the diff (spawn a
  reviewer or inspect), dispatch a fix agent for real issues, re-verify green.

## Lanes (6 agents; models noted)

- **Lane A — zvm (sonnet):** TODO Engine#4. Add a regression test for
  `run_timed_interrupt`'s no-frame branch in `crates/zvm/src/exec.rs`: a timed read whose
  `interrupt_routine` unpacks to an out-of-bounds address (or `local_count > 15`) so
  `call_routine` returns without pushing a frame; assert the return value is popped and
  `aborted` reported without corrupting the eval stack. Test-only; changes no logic.

- **Lane B — zvm-cli (sonnet):** TODO Engine#3. `read_byte_stdin()` in
  `crates/zvm-cli/src/main.rs` busy-spins at EOF (empty `read_line` → returns `\n`
  forever). Fix: detect true EOF (0 bytes read) and exit/return a sentinel. Add a
  regression test. Bounded to one function.

- **Lane C — gvm + gvm-cli (opus):** TODO Engine#49. Honor Glk LINE-INPUT TERMINATORS.
  Currently `glk_set_terminators_line_event` (0x0151) is a no-op (`crates/gvm/src/exec.rs`
  ~2484) and `gestalt_LineTerminators` (17/18) unsupported (~2802); line-event struct
  already carries the terminator-key field (`glk.rs:228`). Implement: record per-window
  requested terminators, answer `gestalt_LineTerminators`, deliver the terminator keycode
  in the line event's second value on terminator-ended input; wire gvm-cli to forward
  those keys. (App wiring optional/secondary.) Test via a glulxercise-style path.

- **Lane D — blorb (opus):** TODO Engine#2. Large `.glorb` load is 5-10s (Lectrote is
  instant). Benchmark `stories/CounterfeitMonkey-11.gblorb` (11 MB), profile the load path
  in `crates/blorb/src/lib.rs` (618 lines), fix the bottleneck (likely excessive copying /
  O(n²) scan). MUST preserve parsing correctness — keep existing blorb tests green; add a
  perf-sanity or regression test if feasible.

- **Lane E — app (opus), TWO tasks sequentially (both touch `app/src/style.rs`):**
  1. TODO line 32 — per-game style default-freeze bug. `write_style_full`/`style_to_decl`
     omit a default (None) color as a missing key, and `merge_decl` is field-level
     (`over.fg.or(base.fg)`), so a per-game field explicitly reset to "default" re-inherits
     the GLOBAL non-default on reload. Fix: emit an explicit `default` sentinel in
     `write_style_full` so the per-game override wins at merge. NOTE: changes the global
     on-disk style format — add round-trip + format-preservation tests. (Fallback if the
     format change proves too invasive overnight: implement conservatively and document, but
     prefer the real fix with tests.)
  2. TODO Engine#1 — clean up 19 `cargo doc -p app --no-deps` broken-intra-doc-link
     warnings. Three kinds, each with the exact fix in the TODO text: (a) ~13 false
     positives (TOML section names `[keymap]` etc. in config.rs:9,16,51,108,159,436 +
     symbols.rs:4; UI labels `[X]`/`[Done]` in input.rs:803, gallery.rs:19,
     hints_panel.rs:30, verbmenu.rs:64) → escape as code spans; (b) ~4 links to private
     items (colors.rs:9, map.rs:432, style.rs:730, style.rs:1144) → harmless, leave or
     fix; (c) ~4 mis-pathed type links (style.rs:115,128 `config::SymbolConfig`;
     style.rs:727,1144 `SymbolSet`) → fix the paths. Verify: `cargo doc -p app --no-deps`
     warning count drops (ideally 0); `cargo build`/`cargo test` stay clean.
  Do the style-freeze fix FIRST (logic), then doc-link (comments) to minimize churn.

- **Lane F — verify/reconcile (sonnet):** TODO MAP#3 (BeyondZork/v4+ automapping). The
  player-object detection fix ALREADY landed in `4adf7f9` (`find_player_object`,
  `player_candidates`, `nearest_matching_ancestor`, `PlayerParent`, `status_line_room_name`
  in `crates/zvm/src/location.rs`). Task is VERIFY + reconcile, not implement: run
  `detect_location` across all `stories/*.z3/.z4/.z5` incl. `beyondzork-r57-s871221.z5`;
  confirm BeyondZork now yields a room (no longer None) AND no v3 regression. If confirmed,
  `scripts/todo-done` the MAP#3 line (and note any residual gap, e.g. LostPig Glulx
  upper-window, which stays deferred). If NOT confirmed, leave a precise report; do not
  hack location.rs blind. Touches TODO.md + possibly a small test in zvm — coordinate the
  zvm test file with Lane A only at merge (separate worktrees, so fine).

## Verified facts
- Story files present: `stories/beyondzork-r57-s871221.z5`,
  `stories/CounterfeitMonkey-11.gblorb`.
- Hooks: `.githooks/{prepare-commit-msg,commit-msg}` active via core.hooksPath.
- `.worktrees/` is git-ignored (worktree home).
- Disk: 2.4 TB free — 6 worktrees fine.

## Status
Plan persisted pre-compaction. Next turn: launch the 6 lane agents as background
worktree tasks, then review each as it reports.
