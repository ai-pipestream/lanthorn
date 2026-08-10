# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

babelmap is a playable interactive-fiction interpreter for the terminal with live automapping. One TUI app (`babelmap <story-file>`) drives three engines: Z-machine (v3–v8 including graphical v6), Glulx, and Scott Adams. Cross-platform: macOS, Linux, Windows.

## Commands

```sh
cargo build --workspace                 # build everything
cargo run -p app -- stories/foo.z5      # run the TUI (binary name: babelmap)
cargo test -p app --test v6_arthur_status            # one integration-test binary
cargo test -p app --test v6_arthur_status -- <name>  # one test within it
```

**Full test gate** (run before any commit):

```sh
cargo test -p zvm -p gvm -p scott -p app 2>&1 | grep -cE "^error(\[|:)| [1-9][0-9]* failed"
```

This must **print 0**. Note: grep exits 1 when it finds zero matches — that exit code IS the pass, so never chain this with `&&` or treat a nonzero exit as failure. Also cross-check completeness: the number of `test result:` lines must equal the number of test binaries launched (compare `grep -c "^     Running\|^   Doc-tests"` against `grep -c "^test result:"`) — a binary that dies mid-run otherwise disappears silently. That check catches a binary that *died*, not one that was never *enumerated*: a newly-created `crates/app/tests/*.rs` can be missing from the first run after you add it, and the counts still match because cargo never launched it. When you add a test file, also confirm its name appears in the `Running` lines (or just re-run the gate).

**Clippy gate**: `cargo clippy --workspace --all-targets -- -D warnings` must be clean.

## Hard rules

- **`zvm`, `gvm`, and `scott` take ZERO external dependencies.** All parsing, text codecs, and Quetzal/save handling are hand-rolled. CLI crates and `app` may add deps (crossterm, ratatui, etc.).
- **Stage files explicitly by path.** Never `git add -A` / `git add .` — the working tree routinely carries untracked scratch files and gitignored fixtures that must not be committed. Delete any `scratch_*.rs` test files before committing.
- **No GitHub PRs.** Workflow is: work on main for routine changes (a feature branch + local merge for major work), then `git push origin HEAD:main`.
- **Commit trailers**: a git hook requires a quest trailer on every commit — `Quest: SQ-xxxx` (work in progress), `Completes: SQ-xxxx` (closes it), `Confirm: SQ-xxxx` (done but awaiting user verification), or `Quest: none`. Quests are tracked with the side-quest MCP tools / `side-quest` CLI, not files.
- **Verify spec constants against authoritative sources** (Z-Machine Standards Document, Glk/Glulx specs), never from memory — unit tests that share the implementation's wrong assumption pass anyway. VM/protocol features need a real-game smoke test.

## Test fixtures

`stories/` is **gitignored** (commercial game files). Real-game integration tests must skip vacuously when their fixture is absent (see `any_v6_story_present()` in `crates/app/tests/zmsd_screen_compliance.rs` for the CI-safe pattern). Freely redistributable fixtures live in `unit_tests/`. Git worktrees lack `stories/` — symlink it from the main checkout when smoke tests matter there.

**A disk image is a different release, not the same story on other media.** `stories/journey.z6` is release 83 / serial 890706; `Journey - The Quest Begins.adf` is release **30** / serial 890322, and the two differ in behaviour (r83 narrates through window 0, r30 through window 2 — which was the whole of SQ-0755). `InterpreterProfile::resolve` reads the medium, so "the Amiga build" means a different build of the game, not merely a different profile. Name the exact fixture and release in any finding, and when a defect is reported on a disk image, reproduce it on that image — a clean result off the bare story file proves nothing about it (SQ-0760).

## Architecture

Full detail in `docs/architecture.md`; docs under `docs/features/` track the code (README tracks the released build). Big picture:

- **`crates/zvm` / `gvm` / `scott`** — pure, headless VM cores (Z-machine, Glulx, Scott Adams). No I/O policy; they expose sessions the app drives. `zvm-cli` / `gvm-cli` / `scott-cli` are minimal terminal front-ends useful for debugging an engine without the TUI.
- **`crates/app`** — the babelmap TUI. Talks to every engine through the engine-neutral `Engine` trait (`src/engine.rs`); `session.rs` (Z-machine), `glulx_session.rs`, and `scott_session.rs` adapt each VM into it. Glk exists only inside the Glulx adapter — it never leaks into shared app types.
- **`crates/mapper`** — the automap graph (rooms, exits, layout). Direction: map work moves off the main thread; only the story interpreter should run there.
- **`crates/blorb`**, **`crates/audio`** — resource-file parsing and sound playback.
- **Render pipeline** — `crates/app/src/render/`, entry `screen.rs`. Graphical v6 has two modes: **hybrid** (terminal cells for text, kitty graphics for art — the default; test this mode first) and **raster** (full-frame image). v6 geometry bugs are usually cell-quantization issues (art scaled by pixel, text placed by cell — watch for ceil-vs-round mismatches on shared boundaries).
- **Slash commands** — one registry, `slash::COMMANDS` (`src/slash.rs`), verb-noun names, keys bind to command strings. Add new commands there; there is no Command enum.
- **Config & styles** — `~/.babelmap/config.toml` and `style.toml` are seeded as fully-commented templates (`src/config_template.rs`; uncommented section headers, `# key = default` lines). `write_config` writes only non-default values but always updates keys already present in the file. Per-game overrides are a bare-lines sidecar `<game_dir>/config.toml` (at most a few keys; absent key = inherit global) — never template it. Every new UI element must be styleable via a `style.toml` selector (ColorScheme field + `style.rs` selector + render apply); never hard-code styles.
- **Persistence** — two save families with distinct names: "Save State/Restore State" = engine-neutral host snapshots (save-anywhere, archive, auto-resume); `@save`/`@restore` = the game's own in-game Quetzal path. They must behave uniformly across engines. Pre-release: formats may break old files freely; no back-compat shims.
  - **Persist the recipe, not the result.** Nothing goes into the archive without either its regeneration inputs or a one-line comment saying why the derived artifact is authoritative. Quetzal saves no screen state by design — the standard assumes the *story* repaints after a restore, and a host Save State swaps memory under a game that never learns it happened, so everything the screen needs is ours to carry. Snapshotting an output (canvas PNGs) instead of its inputs (display list + palette) restores something that looks right and cannot be recomputed when the inputs change (SQ-0587/0588).
  - **The archive is backend- and terminal-neutral.** No cell coordinates, font metrics, or picker state in a save — v6 geometry is zvm native pixels, so a save moves between kitty/halfblocks/sixel and between terminal sizes. A restore reconciles the saved screen with the *current* pane (`reconcile_restored_screen_size`), because a restore into a different size is a resize the game never saw.

## Testing conventions

- Colour/render test areas pin **both** `honor_game_colours` modes (true is the shipped default and primary baseline); single-mode suites have masked regressions before.
- Falsify fixes: temporarily revert the fix and confirm the new test fails with the originally reported symptom before trusting it.
- **Restore tests must perturb before asserting.** Restore bugs surface one action *after* the restore, when the game next repaints, changes palette, splits, or resizes — asserting the frame immediately after a restore is when everything still looks correct. Restore, then make a move, then assert (`v6_restore_palette_replay.rs` is the pattern). Cover restoring into a *different* terminal size and a different graphics backend; both are common in the field and neither is visible to a same-session round-trip.
- Headless render harnesses live in the app integration tests (see `crates/app/tests/v6_*.rs` for the pattern: drive a real story, render to a buffer, assert on cells/geometry).
