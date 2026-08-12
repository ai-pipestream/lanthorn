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
cargo nextest run -p blorb -p zvm -p gvm -p scott -p app 2>&1 | grep -acE "^error(\[|:)| [1-9][0-9]* failed"
```

This must **print 0**. Note: grep exits 1 when it finds zero matches — that exit code IS the pass, so never chain this with `&&` or treat a nonzero exit as failure. (`-a` because a panicking test can emit a NUL byte, after which grep treats the stream as binary and reports nothing.)

Cross-check completeness against nextest's own summary rather than by counting lines: `Starting N tests across M binaries` at the top must be followed by `Summary [...] N tests run: N passed`, with the **same N**. A binary that dies mid-run fails the run outright instead of disappearing. That still catches a binary that *died*, not one that was never *enumerated* — a newly-created `crates/app/tests/*.rs` can be missing from the first run after you add it, and every count is self-consistent because cargo never built it. When you add a test file, confirm its name appears in the run (`cargo nextest list | grep <name>`) or just re-run the gate.

`cargo test` still works and is fine for a single binary (above), but it runs test binaries **one at a time**, parallelising only within each. Measured on this workspace at 12 cores: 542s for `cargo test` against 99s for `cargo nextest run`, same 4176 tests — half the binaries carry three tests or fewer while one carries 2343, so global scheduling is worth ~5.5x. Install with `cargo install cargo-nextest --locked`, or the prebuilt binary from <https://get.nexte.st>.

Two consequences of nextest's model worth knowing: it runs **each test in its own process**, so a test that depends on state left behind by another test in the same binary will fail under it (that is a defect, not an incompatibility); and it does not run doctests, which costs us nothing because every crate sets `doctest = false` — if you ever add a real doctest, remove that setting and run `cargo test --doc` alongside.

**Clippy gate**: `cargo clippy --workspace --all-targets -- -D warnings` must be clean. It costs ~149s the first time after a test build (separate fingerprints, so it shares nothing) and ~0.3s when already warm — cheap to re-run, so re-run it rather than assuming.

## Hard rules

- **`zvm`, `gvm`, and `scott` take ZERO external dependencies.** All parsing, text codecs, and Quetzal/save handling are hand-rolled. CLI crates and `app` may add deps (crossterm, ratatui, etc.).
- **Stage files explicitly by path.** Never `git add -A` / `git add .` — the working tree routinely carries untracked scratch files and gitignored fixtures that must not be committed. Delete any `scratch_*.rs` test files before committing.
- **No GitHub PRs.** Workflow is: work on main for routine changes (a feature branch + local merge for major work), then `git push origin HEAD:main`.
- **Commit trailers**: a git hook requires a quest trailer on every commit — `Quest: SQ-xxxx` (work in progress), `Completes: SQ-xxxx` (closes it), `Confirm: SQ-xxxx` (done but awaiting user verification), or `Quest: none`. Quests are tracked with the side-quest MCP tools / `side-quest` CLI, not files.
- **Verify spec constants against authoritative sources** (Z-Machine Standards Document, Glk/Glulx specs), never from memory — unit tests that share the implementation's wrong assumption pass anyway. VM/protocol features need a real-game smoke test.

## Test fixtures

`stories/` is **gitignored** (commercial game files). Real-game integration tests must skip vacuously when their fixture is absent (see `any_v6_story_present()` in `crates/app/tests/zmsd_screen_compliance.rs` for the CI-safe pattern). Freely redistributable fixtures live in `unit_tests/`. Git worktrees lack `stories/` — symlink it from the main checkout when smoke tests matter there.

**A disk image is a different release, not the same story on other media.** `stories/journey.z6` is release 83 / serial 890706; `Journey - The Quest Begins.adf` is release **30** / serial 890322, and the two differ in behaviour (r83 narrates through window 0, r30 through window 2 — which was the whole of SQ-0755). `InterpreterProfile::resolve` reads the medium, so "the Amiga build" means a different build of the game, not merely a different profile. Name the exact fixture and release in any finding, and when a defect is reported on a disk image, reproduce it on that image — a clean result off the bare story file proves nothing about it (SQ-0760). The release every medium in `stories/` carries is pinned in `crates/app/tests/real_media_releases.rs` and tabulated in `docs/features/interpreter.md`; drive the floppy there before claiming a suite covers "the Amiga profile".

## Architecture

Full detail in `docs/architecture.md`; docs under `docs/features/` track the code (README tracks the released build). Big picture:

- **`crates/zvm` / `gvm` / `scott`** — pure, headless VM cores (Z-machine, Glulx, Scott Adams). No I/O policy; they expose sessions the app drives. `zvm-cli` / `gvm-cli` / `scott-cli` are minimal terminal front-ends useful for debugging an engine without the TUI.
- **`crates/app`** — the babelmap TUI. Talks to every engine through the engine-neutral `Engine` trait (`src/engine.rs`); `session.rs` (Z-machine), `glulx_session.rs`, and `scott_session.rs` adapt each VM into it. Glk exists only inside the Glulx adapter — it never leaks into shared app types.
- **`crates/mapper`** — the automap graph (rooms, exits, layout). Direction: map work moves off the main thread; only the story interpreter should run there.
- **`crates/blorb`**, **`crates/audio`** — resource-file parsing and sound playback.
- **Render pipeline** — `crates/app/src/render/`, entry `screen.rs`. Graphical v6 has two modes: **hybrid** (terminal cells for text, kitty graphics for art — the default; test this mode first) and **raster** (full-frame image). v6 geometry bugs are usually cell-quantization issues (art scaled by pixel, text placed by cell — watch for ceil-vs-round mismatches on shared boundaries).
  - **In hybrid, never rasterise what the game printed as a character.** That is what hybrid is *for*: text as text, art as art. A strip whose pixels the game's own paint runs fully explain must be drawn with glyphs. Rasterising a character costs alignment (a resampled edge meeting a font glyph on a shared boundary is exactly the ceil-vs-round trap above), costs crispness, and costs bandwidth — Journey ships four side rules as 8x900 and 16x900 RGBA bitmaps, ~192 KB per frame, to draw 200 `│`s, and the *same rule* is drawn as glyphs seven rows lower where it happens to cross the menu strip. Classify a strip by what is in it, not by where it sits: reserve raster for pixels the runs cannot account for, which is genuine artwork (Zork Zero's and Arthur's side columns) and nothing else. SQ-0750.
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
- **Editor diagnostics that arrive while an agent is working are snapshots of an unfinished edit, not findings.** A half-written file genuinely has unbalanced parens, and a new call site genuinely outruns its `pub` export by a few seconds — both resolve themselves. `cargo check --all-targets` and the gate are the only authority; never act on a diagnostic without reproducing it there first. Multi-file lanes (render-path work especially) are quieter in a worktree, where the checkout the editor watches never sees the churn — symlink `stories/` into it or every real-game smoke skips vacuously into a false green.
- **Three render-testing layers; escalate only when the cheaper one can't explain the symptom.** Cell-buffer harnesses (`crates/app/tests/v6_*.rs`) assert on babelmap's INTERNAL model — always the first stop, but blind to a defect that's correct in the model and wrong on the user's screen. The emitted-stream harness (`crates/app/tests/pty_stream/`, SQ-0762; ad hoc via `cargo run -p app --example pty_capture`) runs the real binary under a pty and keeps every byte it emits — the pty must answer the terminal queries convincingly as kitty, or the capture silently measures the half-block backend and every number in it is worthless. Reach for it when the model looks right and the screen doesn't; it's the only layer that tells an image PLACEMENT apart from a background PAINTED into cells, indistinguishable on screen, different bugs. The placement oracle (`pty_stream/oracle.rs`, SQ-0764; dev-dep `qwertty-term-vt`) resolves those same bytes the way a real terminal does instead of through our hand-rolled decoder — reach for it when the stream also looks right and the screen is still wrong (placement lifetime, z-order, overlap, stale placements, unicode-placeholder continuation). It is a faithful **port** of Ghostty's core, not Ghostty itself — see `docs/architecture.md` for its caveats (an id-encoding mismatch between the two decoders, the SQ-0772 image-coverage gap, and the libghostty-vt ground-truth escalation that exists but isn't built).
