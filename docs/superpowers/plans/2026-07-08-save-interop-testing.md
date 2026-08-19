# Save-Format Interoperability Testing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automated tests proving lanthorn's standard save files interoperate with reference interpreters (`dfrotz` for Z-machine, `glulxe` for Glulx) in both READ and WRITE directions, with the READ direction CI-enforced via checked-in golden saves.

**Architecture:** Cross-load equivalence oracle — within one interpreter, `play(prefix).probe()` must equal `restore(save_at_P).probe()`. READ tests run lanthorn against checked-in reference-produced golden saves (no binary at test time). WRITE tests (`#[ignore]`) drive the reference binary to restore lanthorn's saves. A one-time setup task (Task 1) empirically resolves the reference-tool scripting details, picks the deterministic prefix/probe, and generates the committed golden fixtures.

**Tech Stack:** Rust integration tests (`crates/{zvm,gvm}/tests/`) using `std::process::Command` for reference binaries (test-only; library crates stay zero-dep). Reference tools: `dfrotz`, `glulxe`+`cheapglk` (homebrew). Fixture: public-domain `glulxercise.ulx`.

> **Scope update (2026-07-08, during execution):** Glulx is **deferred to SQ-0229**
> after a spike showed it needs a disproportionate toolchain (curses-only `glulxe`;
> `glulxercise` has only a failure-counter as observable state → vacuous oracle; a sound
> counter `.ulx` needs Inform 6 + library). **This effort implements the Z-machine half
> only: Task 1 (Z-machine parts), Task 2, Task 3, Task 6.** Tasks 4 and 5 (Glulx) are
> NOT implemented here — they move to SQ-0229. In Task 1, do only the `dfrotz`/`minizork`
> steps (Steps 1, 3, 4, 7-Z, 8); skip the `glulxe`/`glulxercise` steps (2, 5, 6).

## Global Constraints

- `zvm` and `gvm` **library** crates stay ZERO-dependency. Test code may use `std` (incl. `std::process::Command`); do NOT add dev-dependencies without calling it out.
- Binary-gated tests are `#[ignore]` with a loud reason string (e.g. `"needs dfrotz on PATH; run with -- --ignored"`) — **never a silent skip that passes vacuously**.
- The cross-load oracle compares two outputs from the **same** interpreter (never lanthorn-vs-reference transcripts directly).
- The prefix must mutate **more than one state class** (PC/room AND object/flag state); the probe must reveal all of them, so restore dropping any class fails.
- Checked-in fixtures: `glulxercise.ulx` is **public domain** (verified). Every golden save gets a sibling `PROVENANCE.md` (story name+serial, interp+version, exact prefix).
- Commit trailer on every commit: `Quest: SQ-0158`, then `Co-Authored-By` / `Claude-Session`.

## File Structure

- `crates/zvm/tests/save_interop.rs` — Z-machine READ (CI) + WRITE (`#[ignore]`) tests + shared driver helpers.
- `crates/gvm/tests/save_interop.rs` — Glulx READ (CI) + WRITE (`#[ignore]`) tests.
- `crates/zvm/tests/fixtures/interop/` — `minizork-at-P.qzl` + `PROVENANCE.md`.
- `crates/gvm/tests/fixtures/glulxercise.ulx` — new public-domain story.
- `crates/gvm/tests/fixtures/interop/` — `glulxercise-at-P.glksave` (or counter fixture) + `PROVENANCE.md`.
- `scripts/gen-interop-goldens.sh` — regenerates goldens + runs the live suite.
- `docs/features/saves.md` — one-line note + how to run the live suite.

---

### Task 1: Setup spike — resolve tools, choose prefix/probe, generate golden fixtures

**This task is an empirical spike, not TDD.** Its deliverables are: committed golden fixtures + `glulxercise.ulx`, a `PROVENANCE.md` per golden, and a written findings note (`crates/../tests/fixtures/interop/PROVENANCE.md` + the task report) recording the exact commands and tool versions the later tasks depend on. Do NOT write the Rust tests here.

**Files:**
- Create: `crates/zvm/tests/fixtures/interop/minizork-at-P.qzl`, `.../interop/PROVENANCE.md`
- Create: `crates/gvm/tests/fixtures/glulxercise.ulx`, `crates/gvm/tests/fixtures/interop/glulxercise-at-P.glksave` (or the counter-fixture fallback), `.../interop/PROVENANCE.md`
- Create: `scripts/gen-interop-goldens.sh`

**Produces (the interface later tasks consume — record ALL of these in the report + PROVENANCE):**
- Z-machine: the story (`minizork.z3`), the exact deterministic `PREFIX_Z` command list, the `PROBE_Z` command(s), and the golden `.qzl` path.
- Glulx: the chosen fixture (`glulxercise.ulx` or a fallback `counter.ulx`), the exact `PREFIX_G`/`PROBE_G`, and the golden `.glksave` path.
- The exact reference-interpreter invocations that work headlessly (the `dfrotz` and `glulxe` command lines, incl. how a save is loaded and how commands are piped).

- [ ] **Step 1: Confirm/instal reference tools**

```bash
command -v dfrotz || brew install frotz
brew list glulxe >/dev/null 2>&1 || brew install glulxe
brew list cheapglk >/dev/null 2>&1 || brew install cheapglk
dfrotz -h 2>&1 | head -1        # expect: FROTZ ... Dumb interface
command -v glulxe && glulxe 2>&1 | head -3   # note whether it needs a story arg / which Glk frontend
```
Record tool versions. Determine how `glulxe` is driven headlessly: whether the homebrew `glulxe` links a dumb/cheapglk frontend (scriptable via stdin) or a curses frontend (not scriptable). If curses-only, note it — the Glulx **WRITE** direction will degrade to a manual procedure (Task 5), but Glulx **READ** (Task 4) is unaffected.

- [ ] **Step 2: Fetch the public-domain `glulxercise.ulx` fixture**

```bash
mkdir -p crates/gvm/tests/fixtures/interop
curl -fsSL -o crates/gvm/tests/fixtures/glulxercise.ulx \
  https://raw.githubusercontent.com/erkyrath/glk-dev/master/unittests/glulxercise.ulx
# Verify it's a Glulx image (magic bytes "Glul") and record size:
head -c4 crates/gvm/tests/fixtures/glulxercise.ulx | xxd | grep -qi "476c 756c" && echo "Glulx magic OK"
ls -l crates/gvm/tests/fixtures/glulxercise.ulx
```
If the raw URL 404s, fetch the `.ulx` from the `erkyrath/glk-dev` repo `unittests/` directory by another means, or compile `glulxercise.inf` with `inform -G`. Confirm it loads in `gvm` (a quick `cargo run -p gvm-cli -- crates/gvm/tests/fixtures/glulxercise.ulx` reaching a prompt) and in `glulxe`.

- [ ] **Step 3: Choose the Z-machine prefix/probe and verify determinism**

Using `minizork.z3`, pick a `PREFIX_Z` that mutates room AND object state and a `PROBE_Z` that reveals both, e.g. `PREFIX_Z = ["open mailbox", "take leaflet", "north"]`, `PROBE_Z = ["look", "inventory"]` (reveals room = North of House AND that the leaflet is carried). Drive `dfrotz` twice with the same input and confirm byte-identical output (no clock/RNG in the probe):
```bash
printf 'open mailbox\ntake leaflet\nnorth\nlook\ninventory\nquit\ny\n' | dfrotz -w 80 crates/zvm/tests/fixtures/minizork.z3 > /tmp/z1.txt
printf 'open mailbox\ntake leaflet\nnorth\nlook\ninventory\nquit\ny\n' | dfrotz -w 80 crates/zvm/tests/fixtures/minizork.z3 > /tmp/z2.txt
diff /tmp/z1.txt /tmp/z2.txt && echo "deterministic"
```
Adjust the prefix/probe until deterministic AND the probe visibly reflects the mutated state. Record the final `PREFIX_Z`/`PROBE_Z`.

- [ ] **Step 4: Generate the golden Z-machine save with `dfrotz`**

Drive `dfrotz` through `PREFIX_Z`, then issue the game's `save` verb writing to the golden path. `dfrotz` prompts for a filename; feed it on stdin. Example:
```bash
mkdir -p crates/zvm/tests/fixtures/interop
printf 'open mailbox\ntake leaflet\nnorth\nsave\ncrates/zvm/tests/fixtures/interop/minizork-at-P.qzl\nquit\ny\n' \
  | dfrotz crates/zvm/tests/fixtures/minizork.z3
ls -l crates/zvm/tests/fixtures/interop/minizork-at-P.qzl   # exists, FORM IFZS
```
Confirm the file begins with `FORM....IFZS`. Write `crates/zvm/tests/fixtures/interop/PROVENANCE.md` (story minizork.z3 + serial, `dfrotz` version, `PREFIX_Z`).

- [ ] **Step 5: Determine Glulx prefix/probe on `glulxercise` (or trigger the fallback)**

Drive `glulxercise.ulx` (in `gvm-cli` and in `glulxe`) and find a command sequence whose output, after a save+restore, reveals distinct pre-save state (so the oracle is non-vacuous). `glulxercise` is a test harness — inspect its command menu. If NO sequence yields restore-observable state, **fall back**: author `crates/gvm/tests/fixtures/counter.inf` (a ~15-line Inform 6 game: a global counter, an `increment` verb, and a `count` verb that prints it), compile with `inform -G counter.inf`, and use `counter.ulx` as the fixture instead. Record which fixture was chosen and the final `PREFIX_G`/`PROBE_G`. Verify determinism as in Step 3.

- [ ] **Step 6: Generate the golden Glulx save with `glulxe`**

Drive `glulxe` through `PREFIX_G`, save to the golden path (scripting the Glk file prompt via stdin under the cheapglk/dumb frontend). If `glulxe` cannot be scripted headlessly (Step 1 finding), generate the golden save by another available Glulx interpreter, OR document that the golden `.glksave` is produced by `gvm` itself for the READ test's *self-consistency* baseline while the cross-interp READ is validated in the live Task 5 — **but prefer a genuinely foreign save**; note the exact provenance. Write `crates/gvm/tests/fixtures/interop/PROVENANCE.md`.

- [ ] **Step 7: Write `scripts/gen-interop-goldens.sh`**

A documented script that reproduces Steps 4 and 6 (installs tools if missing, drives the terps, regenerates both goldens) and then runs `cargo test -p zvm -p gvm -- --ignored` (the live tests). It is developer-run, not CI.

- [ ] **Step 8: Commit fixtures + script + findings**

```bash
git add crates/zvm/tests/fixtures/interop crates/gvm/tests/fixtures/glulxercise.ulx crates/gvm/tests/fixtures/interop scripts/gen-interop-goldens.sh
git commit -m "$(cat <<'EOF'
test(interop): golden save fixtures + generator for save-format interop (SQ-0158)

Public-domain glulxercise.ulx (or counter.ulx fallback) + dfrotz/glulxe golden
saves at a fixed point, with PROVENANCE. One-time generator in scripts/.

Quest: SQ-0158
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
)"
```

**Report:** record `PREFIX_Z/PROBE_Z`, `PREFIX_G/PROBE_G`, chosen Glulx fixture, tool versions, the working `dfrotz`/`glulxe` invocations (verbatim), and any fallback taken. Later tasks read this.

---

### Task 2: Z-machine READ test (CI-enforced)

**Files:**
- Create: `crates/zvm/tests/save_interop.rs`
- Uses: `crates/zvm/tests/fixtures/minizork.z3`, `.../interop/minizork-at-P.qzl`

**Interfaces:**
- Consumes from Task 1: `PREFIX_Z`, `PROBE_Z`, the golden `.qzl` path.
- Consumes existing zvm API: `Machine::new(Memory::new(bytes))`, `machine.step()`, the input-supply methods, `machine.restore_file(&bytes)`, and the transcript sink used by existing zvm integration tests. (Model the headless driver on the existing zvm test harness — inspect `crates/zvm/tests/*.rs` for the established pattern of feeding line input and collecting output; reuse it rather than inventing one.)

- [ ] **Step 1: Write the failing test**

A helper `fn drive(story, prefix: &[&str], probe: &[&str]) -> String` that builds a `Machine`, feeds each prefix then probe command, and returns the concatenated probe-phase transcript. Then:

```rust
#[test]
fn zmachine_reads_reference_save() {
    let story = include_bytes!("fixtures/minizork.z3");
    let golden = include_bytes!("fixtures/interop/minizork-at-P.qzl");

    // Baseline: play the prefix, then probe.
    let played = drive(story, &PREFIX_Z, &PROBE_Z);

    // Cross-load: fresh machine, restore the dfrotz-made save, then the SAME probe.
    let restored = drive_from_save(story, golden, &PROBE_Z);

    assert_eq!(restored, played,
        "restoring dfrotz's save must reproduce the state reached by playing the prefix");
    assert!(restored.contains(/* a distinctive token proving non-empty state, e.g. "leaflet" */),
        "probe output must reveal the mutated state (guards against a vacuous match)");
}
```
Use the exact `PREFIX_Z`/`PROBE_Z`/distinctive token from Task 1's report.

- [ ] **Step 2: Run it and confirm it fails**

Run: `cargo test -p zvm --test save_interop zmachine_reads_reference_save`
Expected first failure: the helpers (`drive`/`drive_from_save`) don't exist yet, or the assertion fails — implement until green.

- [ ] **Step 3: Implement the driver helpers**

Implement `drive` and `drive_from_save` against the zvm headless test API (per the existing test harness pattern). `drive_from_save` builds the `Machine`, calls `restore_file(golden)`, then feeds `probe`.

- [ ] **Step 4: Run to green**

Run: `cargo test -p zvm --test save_interop zmachine_reads_reference_save`
Expected: PASS. Then sanity-check the guard: temporarily flip one byte of the in-memory golden copy and confirm the test goes RED (revert after).

- [ ] **Step 5: Commit** (message `test(interop): zvm reads dfrotz-produced Quetzal save (SQ-0158)`, standard trailers).

---

### Task 3: Z-machine WRITE test (live, `#[ignore]`-gated on `dfrotz`)

**Files:** Modify `crates/zvm/tests/save_interop.rs`.

**Interfaces:** Consumes Task 1's `PREFIX_Z`/`PROBE_Z` and the working `dfrotz` invocation (incl. `-L <save>` load and stdin piping). Consumes zvm save API: produce lanthorn's save bytes at P (drive `PREFIX_Z`, then `machine.save_quetzal()`).

- [ ] **Step 1: Write the `#[ignore]` test**

```rust
#[test]
#[ignore = "needs dfrotz on PATH; run with: cargo test -p zvm --test save_interop -- --ignored"]
fn zmachine_save_read_by_dfrotz() {
    if which("dfrotz").is_none() { panic!("dfrotz not found — this --ignored test requires it"); }
    // lanthorn writes its save at P to a temp file.
    let bab_save = lanthorn_save_at_p();                 // drive PREFIX_Z, save_quetzal(), write temp
    // Oracle: dfrotz restoring lanthorn's save == dfrotz having played the prefix.
    let dfrotz_played   = dfrotz_run(STORY_PATH, /*load*/ None,      &PREFIX_Z, &PROBE_Z);
    let dfrotz_restored = dfrotz_run(STORY_PATH, /*load*/ Some(&bab_save), &[],  &PROBE_Z);
    assert_eq!(normalize(&dfrotz_restored), normalize(&dfrotz_played),
        "dfrotz restoring lanthorn's save must match dfrotz playing the prefix");
}
```
`dfrotz_run` shells out via `std::process::Command` using the verbatim invocation Task 1 verified (`-L` for the load case; piping prefix+probe+`quit`+`y` on stdin). `normalize` trims trailing whitespace / the quit banner if needed (define minimally, only what Task 1 showed differs). `#[ignore]` reason is loud.

- [ ] **Step 2: Run it (opt-in) and confirm it passes**

Run: `cargo test -p zvm --test save_interop -- --ignored zmachine_save_read_by_dfrotz`
Expected: PASS locally (dfrotz present). If it fails, the divergence localizes a real interop bug — investigate before proceeding (do not weaken the assertion). Confirm the non-ignored suite still ignores it: `cargo test -p zvm --test save_interop` shows it as ignored.

- [ ] **Step 3: Commit** (message `test(interop): dfrotz reads lanthorn Quetzal save (SQ-0158)`, standard trailers).

---

### Task 4: Glulx READ test (CI-enforced)

**Files:** Create `crates/gvm/tests/save_interop.rs`. Uses `crates/gvm/tests/fixtures/<glulx fixture>.ulx`, `.../interop/<golden>.glksave`.

**Interfaces:** Consumes Task 1's Glulx fixture choice, `PREFIX_G`, `PROBE_G`, golden path. Consumes existing gvm API: how to build a machine from a `.ulx`, step it, feed line input, restore a save (`gvm` `restore`), and collect output — model on `crates/gvm/tests/*.rs` (e.g. `accel_story_equivalence.rs`) for the established headless-drive pattern.

- [ ] **Step 1: Write the failing test** — same shape as Task 2 (`glulx_reads_reference_save`): baseline `drive(fixture, PREFIX_G, PROBE_G)` vs `drive_from_save(fixture, golden, PROBE_G)`, `assert_eq!`, plus a distinctive-token guard from Task 1.

- [ ] **Step 2: Run and confirm it fails**, then

- [ ] **Step 3: Implement the gvm driver helpers** against the gvm headless test API (reuse the accel-test pattern).

- [ ] **Step 4: Run to green**; verify the byte-flip guard goes RED.
Run: `cargo test -p gvm --test save_interop glulx_reads_reference_save` → PASS.

- [ ] **Step 5: Commit** (message `test(interop): gvm reads reference-produced Glulx save (SQ-0158)`, standard trailers).

---

### Task 5: Glulx WRITE test (live, `#[ignore]`-gated on `glulxe`) — or documented manual fallback

**Files:** Modify `crates/gvm/tests/save_interop.rs` and/or `docs/features/saves.md`.

**Interfaces:** Consumes Task 1's finding on whether `glulxe` is headless-scriptable and its verbatim invocation.

- [ ] **Step 1 (if `glulxe` is scriptable):** Write `#[ignore]`-gated `glulx_save_read_by_glulxe`, same oracle as Task 3 (`glulxe.restore(lanthorn_save).probe()` == `glulxe.play(prefix).probe()`), shelling out via `Command`. Loud `#[ignore]` reason. Run with `-- --ignored`, confirm PASS; do not weaken on divergence (investigate the interop bug).

- [ ] **Step 1 (if `glulxe` is NOT headless-scriptable, per Task 1):** Instead of a test, add a **documented manual procedure** to `docs/features/saves.md` (or a `docs/interop-manual.md`): the exact steps to restore a lanthorn `.glksave` in `glulxe`/Lectrote/Quixe and what to verify. `log`/note this limitation loudly; do not ship a skipping test that pretends to cover it.

- [ ] **Step 2: Commit** (message `test(interop): glulxe reads lanthorn Glulx save (SQ-0158)` or `docs(interop): manual Glulx write-direction procedure (SQ-0158)`).

---

### Task 6: Docs note + final verification

**Files:** Modify `docs/features/saves.md`.

- [ ] **Step 1:** Add a one-line note to `docs/features/saves.md` that standard saves are interoperability-tested (both engines, both directions), with a pointer to `scripts/gen-interop-goldens.sh` / `cargo test -- --ignored` for the live suite.

- [ ] **Step 2: Final verification:**
```bash
cargo test -p zvm -p gvm --test save_interop            # CI (READ) tests green; live tests show as ignored
cargo test -p zvm -p gvm --test save_interop -- --ignored  # live tests PASS locally
cargo test -p zvm -p gvm                                # whole suites green
```
Confirm: the two READ tests run in CI (not ignored); the live tests are ignored-by-default and pass with `--ignored`; the byte-flip guard was demonstrated for each READ test.

- [ ] **Step 3: Commit** (message `docs(saves): note save-format interop testing (SQ-0158)`, standard trailers).

---

## Self-review checklist (run before execution)

- Task 1 is a spike; its report is the interface for Tasks 2-5 (exact prefix/probe/invocations). Confirm each later task references Task 1's recorded values, not invented ones.
- No READ test can pass vacuously: each has a distinctive-token guard AND a demonstrated byte-flip-goes-red check.
- No live test silently skips: each is `#[ignore]` with a loud reason and `panic!`s if its binary is somehow absent when run with `--ignored`.
- `zvm`/`gvm` library crates gain no dependencies (test-only `std::process::Command`).
