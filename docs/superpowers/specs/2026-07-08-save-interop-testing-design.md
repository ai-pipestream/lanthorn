# Save-Format Interoperability Testing (Z-machine + Glulx) — Design

**Date:** 2026-07-08
**Status:** Approved for planning
**Quest:** SQ-0158
**Related:** SQ-0163 (made the Z-machine in-game `@save` PC convention standard — this verifies it).

> **Scope update (2026-07-08, during execution):** A Task-1 spike found Glulx interop
> needs a disproportionate toolchain investment — homebrew `glulxe` is curses-only (not
> headless-scriptable), `glulxercise`'s only restore-observable state is a failure counter
> (making the cross-load oracle vacuous on a correct interpreter), and a sound counter
> `.ulx` fixture needs Inform 6 + its library. **Glulx is therefore deferred to SQ-0229.**
> SQ-0158 now covers the **Z-machine** direction only (`dfrotz`, fully automated). The Glulx
> design below is retained as the starting point for SQ-0229.

## Goal

Prove that babelmap's save files interoperate with other standard interpreters, in
**both** directions and for **both** engines:

- **READ** — babelmap correctly restores saves produced by a reference interpreter.
- **WRITE** — a reference interpreter correctly restores saves produced by babelmap.

The primary durable artifact is automated tests. The one CI-enforced guarantee per
engine is the READ direction (via checked-in golden saves, no external binary at
test time); the WRITE direction is an `#[ignore]`-gated live test that runs a
reference interpreter, plus a documented local runner.

## Background

babelmap emits two standard formats, both `FORM IFZS` (Quetzal):
- Z-machine: bare Quetzal `.qzl` via `zvm::quetzal::save_quetzal` (host `save_game`,
  `persist_files.rs:203`); restore via `restore_file`/`restore_quetzal`.
- Glulx: "Glulx-Quetzal" via `gvm` `save_state` (`gvm/src/exec.rs:1373`; `CMem` +
  a `Glk ` snapshot chunk, GLULX_NOTES §20); restore via `gvm` `restore`.

Reference interpreters:
- **Z-machine: `dfrotz`** (installed; homebrew). Loads a save at startup with
  `dfrotz -L <save> <story>` — non-interactive, ideal for the WRITE direction.
- **Glulx: `glulxe` + `cheapglk`** (installable; homebrew `glulxe`, `cheapglk`).

Story fixtures:
- Z-machine: `crates/zvm/tests/fixtures/minizork.z3` (checked in; v3).
- Glulx: no story is checked in (the `stories/` `.gblorb`s are gitignored, per the
  `#[ignore]` precedent in `crates/gvm/tests/accel_story_equivalence.rs`). We add
  **`glulxercise.ulx`** — Andrew Plotkin's Glulx interpreter unit-test story,
  **public domain** (verified from the `glulxercise.inf` header: "This unit test
  suite, and its functions, are in the public domain"; Release 13, serial 241202) —
  as a checked-in fixture so Glulx also gets a CI-enforced READ test.

## Design

### The cross-load equivalence oracle

Comparing babelmap's transcript to a reference interpreter's transcript byte-for-byte
is fragile (different formatting, prompts, status lines). Instead, **every assertion
compares two outputs from the *same* interpreter**, varying only the state source:

> Within interpreter *I*: **playing a fixed command prefix to point P**, then a probe,
> must produce the **same** output as **restoring a save taken at P**, then the same
> probe.

- **READ** (*I* = babelmap): `babelmap.play(prefix).probe()` ==
  `babelmap.restore(reference_golden_save).probe()`. If equal, babelmap reconstructs
  the reference save's state correctly. Needs only the checked-in golden save +
  story — **CI-enforced, no binary**.
- **WRITE** (*I* = reference terp): `terp.play(prefix).probe()` ==
  `terp.restore(babelmap_save).probe()`. If equal, the reference terp reconstructs
  babelmap's save correctly. Needs the reference binary — **`#[ignore]`-gated**.

**Requirements on prefix/probe (per story):** the prefix must mutate observable state
of more than one kind (position/PC *and* object/flag state), and the probe must reveal
that mutated state (e.g. a `look` + `inventory` pair after moving rooms and taking an
item), so the test fails if restore drops any state class — not just the PC. `dfrotz`
and babelmap (and `glulxe` and babelmap) must reach the *same* state from the same
prefix; both stories' relevant openings are deterministic (no RNG at the chosen P).

### Fixtures (checked in)

- `crates/zvm/tests/fixtures/minizork.z3` (exists).
- `crates/zvm/tests/fixtures/interop/minizork-at-P.qzl` — golden Z-machine save made
  once by `dfrotz` at point P (below).
- `crates/gvm/tests/fixtures/glulxercise.ulx` — new, public domain.
- `crates/gvm/tests/fixtures/interop/glulxercise-at-P.glksave` — golden Glulx save
  made once by `glulxe` at point P.

Each golden save is accompanied by a `PROVENANCE.md` in the `interop/` dir recording
the exact story (name/serial), the reference interpreter + version, and the exact
prefix commands used to reach P — so the golden can be regenerated deterministically.

### Tests

- `crates/zvm/tests/save_interop.rs`
  - `zmachine_reads_reference_save` (CI): fresh `Machine` from `minizork.z3`; assert
    `play(prefix).probe()` == `restore(golden .qzl).probe()`.
  - `zmachine_save_read_by_dfrotz` (`#[ignore]`, gated on `dfrotz` present): babelmap
    writes a save at P to a temp file; assert
    `dfrotz.play(prefix).probe()` == `dfrotz.restore(babelmap_save).probe()`, driving
    `dfrotz` via `-L <save>` and stdin, comparing its dumb-mode output.
- `crates/gvm/tests/save_interop.rs`
  - `glulx_reads_reference_save` (CI): fresh `gvm` machine from `glulxercise.ulx`;
    assert `play(prefix).probe()` == `restore(golden .glksave).probe()`.
  - `glulx_save_read_by_glulxe` (`#[ignore]`, gated on `glulxe` present): same shape,
    driving `glulxe` (cheapglk dumb frontend) headlessly.

Tests use `std::process::Command` for the reference binaries (test-only; the `zvm`/`gvm`
library crates remain zero-dependency — dev/test process invocation does not add crate
deps). Binary-gated tests are `#[ignore]` with a loud reason string
(`"needs dfrotz on PATH; run with -- --ignored"`), never a silent skip.

### One-time local setup (generates goldens + validates the live path)

A helper script `scripts/gen-interop-goldens.sh` (documented, run by a developer, not
in CI):
1. Installs/uses `dfrotz`, `glulxe`, `cheapglk`.
2. Drives each reference terp through the exact prefix to P and saves, producing the
   two golden files, which are then committed under `.../interop/`.
3. Runs the `#[ignore]` live tests (`cargo test -- --ignored`) to confirm the WRITE
   direction passes locally.

### `glulxe` scriptability + `glulxercise` suitability — validated up front

Two risks are resolved during the one-time setup, before the plan commits to them:
1. **`glulxe` frontend:** the homebrew `glulxe` may link a curses (`glkterm`) frontend
   that cannot be scripted headlessly. Mitigation: build/use a `cheapglk` (dumb)
   frontend for `glulxe`; if unavailable, the Glulx **WRITE** (live) direction degrades
   to a documented manual procedure. The Glulx **READ** (golden) test is unaffected —
   it needs no binary.
2. **`glulxercise` observable state:** it is a test harness, not a room game. Confirm a
   command sequence exists whose post-save/restore output reveals distinct pre-save
   state (so the oracle is not vacuous). If no such sequence exists, fall back to a
   tiny authored public-domain counter `.ulx` (compiled once from Inform 6) as the
   Glulx fixture instead of `glulxercise.ulx`.

## Components / files

- `crates/zvm/tests/save_interop.rs` — new (Z-machine READ + WRITE tests).
- `crates/gvm/tests/save_interop.rs` — new (Glulx READ + WRITE tests).
- `crates/gvm/tests/fixtures/glulxercise.ulx` — new public-domain fixture.
- `crates/{zvm,gvm}/tests/fixtures/interop/*.{qzl,glksave}` + `PROVENANCE.md` — golden saves.
- `scripts/gen-interop-goldens.sh` — one-time golden generator + live-test runner.
- `docs/features/saves.md` — a short note that saves are interoperability-tested; how
  to run the full (live) suite locally.

## Testing (of this work itself)

- The two READ tests must FAIL if the save format regresses — verify by mutating a byte
  of a golden save (or temporarily breaking the PC convention) and confirming red.
- The `#[ignore]` live tests must actually pass when run with `-- --ignored` locally
  (evidenced in the plan's execution, not just written).
- `cargo test -p zvm` and `cargo test -p gvm` (non-ignored) stay green in CI.

## Out of scope

- Testing against additional interpreters (Bocfel, Lectrote, Parchment/Quixe web) —
  `dfrotz`/`glulxe` are sufficient reference oracles; more can be added later.
- Aux-data (`Glk `/`aux`) and screen-state interop beyond what a normal restore covers.
- Making babelmap's emulator-style "Save State" archive portable (it is intentionally a
  babelmap container; only the standard `@save` path is interop-tested here).
- Automatically installing reference binaries in CI (the live tests are local/opt-in).

## Open risks (tracked, resolved in the plan's setup task)

- `glulxe` headless scriptability (see mitigation above).
- `glulxercise` observable-state suitability (fallback: authored counter `.ulx`).
- `dfrotz` dumb-mode output determinism across its own play-vs-restore paths (the
  oracle compares dfrotz-to-dfrotz, so formatting cancels; confirm no timestamp/RNG in
  the chosen probe output).
