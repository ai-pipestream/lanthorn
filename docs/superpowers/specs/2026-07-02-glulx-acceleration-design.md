# Glulx Acceleration (accelfunc interception) — Design

**Date:** 2026-07-02
**Status:** Approved (design); pending spec review → plan.
**Crate(s):** `gvm` (engine), `gvm-cli`, `app`.

## Goal

Make gvm honor the `accelfunc`/`accelparam` assignments it already stores: when a
running game calls a function that has been assigned a well-known accelerated-function
number, execute a native Rust equivalent instead of interpreting that function's
bytecode. This collapses the millions of Inform 7 veneer property/class-lookup calls
that dominate world-model initialization, so large story files (e.g.
`CounterfeitMonkey-11.gblorb`) reach the first prompt in a fraction of the current time.

Acceleration is the spec-designed lever for exactly this cost; it is what makes
Quixe/Glulxe/git "instant". gvm currently stores the tables but reports the
`Acceleration`(9)/`AccelFunc`(10) gestalt selectors as `0` and never intercepts.

## Background: what already exists

Phase 2c (`docs/superpowers/specs/2026-06-27-glulx-vm-phase-2c-design.md`) implemented
the **storage** half:

- `Machine.accel_funcs: HashMap<u32, u32>` — VM function address → accel number.
- `Machine.accel_params: HashMap<u32, u32>` — param index → value.
- The `accelfunc` (0x180) / `accelparam` (0x181) opcodes populate these
  (`exec.rs:537–550`), `0` cancels a function assignment.
- Both tables are interpreter configuration and are intentionally **not** touched by
  save/restore (verified by `state_roundtrips_with_accel_assignments`).
- `Machine::accel_func_for(addr)` / `accel_param(index)` read them back.
- `gestalt(9)` and `gestalt(10)` currently return `0` (`exec.rs:1208–1209`).

This design implements the **interception** half. No change to the storage opcodes.

## Non-goals

- Float ops, and any other deferred Glulx gestalt (unchanged).
- A gameplay-facing config key, F-key toggle, or settings row for acceleration.
  Acceleration is invisible when correct; the only control is a debug escape hatch.
- Re-entrant accelerated functions. The 13 standard functions are leaf computations
  over the memory image and never call back into VM bytecode; the design relies on
  this (no accelerated function pushes a frame).

## The accelerated functions

The Glulx acceleration spec defines 13 well-known functions, all pure computations
over the game's memory image plus the `accel_params` table:

| Num | Name | Effect |
|-----|------|--------|
| 1 | `Z__Region` | Classify an address: 3 = routine, 2 = string, 1 = object, 0 = none |
| 2 | `CP__Tab` (v1) | Find a property-table entry for (object, property id) |
| 3 | `RA__Pr` (v1) | Read property **address** for (object, id); 0 if absent |
| 4 | `RL__Pr` (v1) | Read property **length** for (object, id); 0 if absent |
| 5 | `OC__Cl` (v1) | Object-in-class test |
| 6 | `RV__Pr` (v1) | Read property **value** (dereferenced); veneer error path if absent |
| 7 | `OP__Pr` (v1) | Object-**provides**-property test |
| 8 | `CP__Tab` (v2) | v2 of 2 |
| 9 | `RA__Pr` (v2) | v2 of 3 |
| 10 | `RL__Pr` (v2) | v2 of 4 |
| 11 | `OC__Cl` (v2) | v2 of 5 |
| 12 | `RV__Pr` (v2) | v2 of 6 |
| 13 | `OP__Pr` (v2) | v2 of 7 |

Function 1 (`Z__Region`) is unchanged across versions. Functions **2–7** are the
original ("version 1") set; **8–13** are the revised ("version 2") set that corrects
class/`num_attr_bytes` handling. Modern Inform 7 (6G60+) assigns **1 + 8–13**; older
compilers assign **1–7**.

**Scope decision: implement all 13.** The v1/v2 pairs differ only in the class-lookup
and `num_attr_bytes` handling, so the incremental cost over "modern only" is small,
and it guarantees any Inform-compiled Glulx game accelerates regardless of compiler
vintage. The exact per-function algorithms are transcribed verbatim into the
implementation plan from the Glulx spec (the spec wins over any prose here).

### Parameters (`accel_params`)

The functions read these indices from the `accel_params` table (populated by the game
via `accelparam`):

| Index | Name | Meaning |
|-------|------|---------|
| 0 | `classes_table` | Address of the class-objects table |
| 1 | `indiv_prop_start` | First individual-property id |
| 2 | `class_metaclass` | Address of the `Class` object |
| 3 | `object_metaclass` | Address of the `Object` object |
| 4 | `routine_metaclass` | Address of the `Routine` object |
| 5 | `string_metaclass` | Address of the `String` object |
| 6 | `self` | Address of the `self` global |
| 7 | `num_attr_bytes` | Number of attribute bytes per object |
| 8 | `cpv__start` | Start of the class-property-values table |

A param not set by the game reads as `0` (the spec's default), via the existing
`accel_param` accessor returning `None` → treated as `0`.

## Interception points

The result-delivery differs by call form, so there are **two** hook sites.
`op_call` / `op_callf` / `op_callfi` / `op_callfii` / `op_callfiii` all funnel through
`call_function`; `op_tailcall` does **not** — it enters via `build_frame_and_enter`
directly. Both must be hooked or tailcalled accelerated functions silently run
unaccelerated.

### 1. `call_function` (exec.rs:901)

Before pushing the stub / building the frame:

```
if self.acceleration {
    if let Some(num) = self.accel_func_for(func_addr) {
        if let Some(result) = accel::accelerated(self, num, args) {
            return self.deliver_accel_result(result, dest); // store per dest; no frame
        }
    }
}
// ... existing stub push + build_frame_and_enter
```

`deliver_accel_result` stores `result` according to `dest` (Discard / Mem / Local /
Push) — the same delivery `return_value` performs, minus the frame teardown, since no
frame was ever entered.

### 2. `op_tailcall` (exec.rs:1835)

A tailcall reuses the current frame's call stub, so tailcalling an accelerated
(immediately-returning) function is equivalent to returning its result from the
current function:

```
if self.acceleration {
    if let Some(num) = self.accel_func_for(func) {
        if let Some(result) = accel::accelerated(self, num, &args) {
            return self.return_value(result); // deliver to caller's stub, pop current frame
        }
    }
}
// ... existing build_frame_and_enter
```

`accel::accelerated` returns `None` for an unassigned or not-yet-implemented number,
so control falls through to the unchanged interpreted path.

## Gestalt

- `gestalt(9 /* Acceleration */, _)` → `1`.
- `gestalt(10 /* AccelFunc */, n)` → `1` iff `n ∈ 1..=13`, else `0`.

Updates the two lines at `exec.rs:1208–1209` and the `GLULX_NOTES.md` gestalt table.

## Escape hatch

- `Machine` gains `acceleration: bool` (default **true**) and
  `set_acceleration(&mut self, bool)`.
- `gvm-cli` gains a `--no-accel` flag that calls `set_acceleration(false)` before run.
- `lanthorn` (app) gains the same `--no-accel` flag, wired to the gvm session's
  machine.
- No config key, F-key toggle, or settings row: acceleration is not a gameplay
  preference. The flag exists solely to debug a game that misbehaves under
  acceleration (which the differential tests are designed to prevent shipping).

## Verification

Three layers, in increasing scope:

### 1. Spec-algorithm unit tests (`accel.rs`)

For each of the 13 functions, build a small synthetic object/class table in memory
in-test, set the relevant `accel_params`, and assert the native function returns
hand-computed expected values — including the spec's edge cases (absent property → 0,
invalid address → `Z__Region` 0, class vs. object vs. routine vs. string, individual
vs. common properties).

### 2. Differential equivalence (`exec.rs` test harness) — **best effort**

A helper runs the **real veneer frame** for a given `func_addr`/args — assemble a
minimal Glulx image whose function at `func_addr` is a faithful transcription of the
veneer routine, `build_frame_and_enter` + run to return — and asserts its result
equals `accel::accelerated(...)` for the same inputs.

This is the most involved scaffolding (hand-assembling a faithful veneer routine), so
it is **best effort**: implement it for the functions where a minimal transcription is
tractable (e.g. `Z__Region`, a simple property lookup), and where it proves too costly
to transcribe faithfully, rely on layer 3 (full-story on/off equivalence) as the
anti-divergence guarantee instead. Do **not** block the feature on achieving
differential coverage of all 13 functions.

### 3. Full-story equivalence — **primary anti-divergence guarantee**

`CounterfeitMonkey-11.gblorb` (and one more Glulx title) run twice — acceleration on
vs. off — must produce:

- identical transcript up to the first input prompt, and
- the same detected starting room, and
- the accelerated run executes dramatically fewer opcodes (assert a large reduction,
  not an exact number).

If any story asset is too large/slow for the normal test tier, gate that specific
check behind `#[ignore]` with a note, keeping the fast unit + differential tests in
the default run.

## Perf baseline (gated first task)

Before implementing interception, a measurement task (systematic-debugging Phase 1)
establishes evidence:

- Opcode count and wall-clock to first prompt for `CounterfeitMonkey-11.gblorb`
  (release build), acceleration off — the baseline.
- Confirmation that the accel-candidate veneer functions actually dominate the
  ~23.7M init opcodes (a per-function or per-address opcode tally). If they do **not**
  dominate, stop and reconvene — acceleration would be the wrong lever and the
  re-scoped TODO should point elsewhere.

This both de-risks the approach and yields the number the final story-equivalence test
proves the win against. It writes its findings into the plan's record; it changes no
production code.

## File structure

- **Create** `crates/gvm/src/accel.rs`
  - `pub(crate) fn accelerated(m: &Machine, num: u32, args: &[u32]) -> Option<u32>` —
    dispatch on `num`; `None` for unimplemented/unassigned.
  - The 13 functions as private helpers over `&Machine` memory + `accel_param`.
  - The spec-algorithm unit tests.
- **Modify** `crates/gvm/src/exec.rs`
  - `acceleration: bool` field (+ constructor init `true`) and `set_acceleration`.
  - `deliver_accel_result` helper.
  - Hooks in `call_function` and `op_tailcall`.
  - Gestalt 9/10.
  - The differential-equivalence test harness.
  - `mod accel;` declaration.
- **Modify** `crates/gvm-cli/src/main.rs` — `--no-accel` flag.
- **Modify** `crates/app/src/…` — `--no-accel` flag wired to the gvm session machine.
- **Update** `crates/gvm/GLULX_NOTES.md` §17 + gestalt table (interception now done).
- **Update** `README.md` — brief user-facing line (large Glulx games start faster).
- **Update** `TODO.md` → `COMPLETED.md` for the re-scoped interpreter-throughput item.

## Global constraints

- `gvm` stays **zero-dependency** (VM crate rule). `gvm-cli`/`app` may use their
  existing arg-parsing.
- All three platforms (Windows/Linux/macOS) — no OS-specific code introduced.
- Acceleration must be **behaviorally transparent**: with the differential + story
  equivalence tests green, an accelerated run is indistinguishable from an
  interpreted run except in speed.
- Follow the project ship ritual: green gate on touched crates, `scripts/todo-done`,
  README update, `Completes:` trailer, Co-Authored-By / Claude-Session trailers.
