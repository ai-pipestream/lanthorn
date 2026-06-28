# Glulx Save/Restore — Phase B (app: route archives through Engine) Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route babelmap's `.babelmap` archive save/restore (and restart) through the engine-neutral `Engine::save_state`/`restore_state`, so Glulx games save/restore/auto-load. Z-machine `game.sav` stays byte-identical; standard `.qzl`/`.sav` interchange stays Z-machine-only.

**Spec:** `docs/superpowers/specs/2026-06-27-glulx-save-restore-design.md` (Phase B).

## Existing interfaces

- `crates/app/src/archive.rs`: `save_archive(path, mapper, machine: &Machine)`, `save_archive_meta(path, mapper, machine: &Machine, meta, transcript, kinds, runs, history, command_history)`, `save_named(...)`, `load_archive(path) -> ArchiveContents` (`ArchiveContents { save: Vec<u8> (Quetzal), screen: Option<ScreenState>, transcript, kinds, runs, … }`), entries `ENTRY_SAVE`/`ENTRY_SCREEN`/`ENTRY_ENGINE`.
- `crates/app/src/engine.rs`: `Engine::save_state() -> EngineSave`, `restore_state(&EngineSave) -> Result`; `EngineSave { engine, format_version, bytes }`.
- `crates/app/src/main.rs`: helpers `zvm_session_opt(&dyn Engine) -> Option<&GameSession>` (for the zvm-only `screen.json`), `engine_supports_save(&dyn Engine) -> bool` (the guards). Guard sites ~lines 1579/1620 (Save&Quit), 2421/2457 (Save/Load), 2614/2715/2787 (Restore), 3017/3052 (SaveAs/Restore), 3134 (reset_game/restart).
- `crates/app/src/hints.rs`: `load_story(path) -> LoadedStory { ZCode(Vec<u8>), Glulx(Vec<u8>) }` — the session factory used at construction; reuse it for restart.

## Global Constraints

- Z-machine save/restore/restart **byte-for-byte unchanged** (zvm `EngineSave.bytes` == today's Quetzal; `screen.json` still written/restored for zvm). Old `.babelmap` archives load unchanged.
- 0 warnings (`cargo build`, `cargo doc -p app --no-deps`) + full `cargo test --workspace` green per task.
- Commit-only on the phase's worktree branch; one commit per task (TDD). No push. Do not edit `TODO.md`. App crate only.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`

---

## Task 1: Engine-agnostic archive save/load

**Files:** `crates/app/src/archive.rs`.

- [ ] **Step 1: Failing tests:**
  - A round-trip with an `EngineSave { "zmachine", …, <quetzal bytes> }` writes `game.sav` == those bytes + `engine.txt` == `"zmachine"`, and a zvm caller also writes `screen.json`; `load_archive` returns the `EngineSave` (tag from `engine.txt`, bytes from `game.sav`) + the `screen`.
  - A `"glulx"`-tagged save round-trips with **no** `screen.json`.
  - **Back-compat:** an archive written by the OLD code (raw Quetzal `game.sav`, no `engine.txt`, `screen.json` present) loads as `EngineSave { "zmachine", <bytes> }` + the screen.
- [ ] **Step 2:** Change the save fns to take the **game-save bytes + engine tag** (an `&EngineSave`) plus an `Option<&ScreenState>` (the zvm-only screen) instead of `machine: &Machine`. Write `EngineSave.bytes` → `ENTRY_SAVE`, the tag → `ENTRY_ENGINE`, and `screen.json` only when the screen is `Some`. `load_archive`'s `ArchiveContents` exposes the `EngineSave` (or `{ save_bytes, engine_tag }`) + `screen: Option<ScreenState>` (absent for `"glulx"`); default `engine_tag` to `"zmachine"` when `engine.txt` is absent.
- [ ] **Step 3:** Run + commit — `refactor(app): engine-agnostic archive save/load (EngineSave + tag; screen.json zvm-only)`.

---

## Task 2: Route the archive handlers through `Engine`; restart via the factory

**Files:** `crates/app/src/main.rs`.

- [ ] **Step 1:** For each `.babelmap`-**archive**-based handler — Save & Quit, `SlashOutcome::Save` (named + archive), `SlashOutcome::Load`, the saves-manager archive load, restore-from-archive, replay-resume, `SaveAs`, launch "Resume", and the per-turn / on-exit autosave — **replace** the `engine_supports_save` guard + the `zvm_session(...).machine` use with: `let es = session.save_state();` then the archive write (passing `&es` + `zvm_session_opt(&*session).map(|z| &z.machine.screen)` for the zvm-only screen); and on restore, `session.restore_state(&es)` (mapping `EngineError::EngineMismatch` to a graceful status), plus `if let Some(z) = zvm_session_opt_mut(...) { apply screen.json }`. These now work for **both** engines.
- [ ] **Step 2: Restart** (`reset_game`): rebuild the engine via the factory — `load_story(path)` → `GameSession::new`/`GlulxSession::new` boxed as `Box<dyn Engine>` — instead of `*zvm_session_mut(session) = new_session`. Remove its guard. (Reuse the construction logic from session creation in `main`.)
- [ ] **Step 3: Keep** the `engine_supports_save` guard ONLY on the standard `.qzl`/`.sav` **import/export** paths (export-save-name, file-browser import, and any `Action::SaveGame`/`RestoreGame` standard-save arms) — these stay Z-machine-only.
- [ ] **Step 4: Tests / smoke:** a `GlulxSession`-backed engine round-trips through the archive save+restore path (state preserved, no panic, no "not supported" message on those paths); restart rebuilds a working engine for both a Z-code and a Glulx story; the standard `.qzl` import/export still shows the guard for Glulx; Z-machine save/restore/restart unchanged (existing tests green).
- [ ] **Step 5:** Run + commit — `feat(app): Glulx games save/restore/restart via the .babelmap archive (Engine-routed)`.

---

## Self-review checklist (run before final review)

- A Glulx game saves and restores (quick/named/archive/auto-load) and restarts; no "not supported" on those paths.
- Z-machine `game.sav` is byte-identical; `screen.json` still written/restored for zvm; old archives load.
- The foreign-engine restore guard still fires (a Glulx save into a Z-machine game → graceful message), via `Engine::restore_state`'s `EngineMismatch`.
- Standard `.qzl`/`.sav` import/export still guards Glulx (parked).
- 0 warnings; full `cargo test --workspace` green.
