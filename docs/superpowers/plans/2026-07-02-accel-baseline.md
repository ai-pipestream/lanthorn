# Glulx Acceleration — Perf Baseline (Task 0)

**Story:** `stories/CounterfeitMonkey-11.gblorb` (11 MB Glulx image, extracted from its `GLUL` Blorb chunk).
**Target:** first prompt = first `StepResult::NeedLine` (per the plan's definition).
**Build:** `gvm` release profile (`cargo test -p gvm --release`, single-threaded, one run — no averaging).
**Instrument:** `Machine.insn_count: u64` (permanent, committed in `aca3627`), incremented once per opcode dispatched in `step_once`. All other instrumentation described below (per-function tally, wall-clock print, a one-line gestalt probe) was temporary and has been reverted — the working tree is byte-identical to the `insn_count`-only commit.

## Step 1: Baseline (current tree, accel off)

| Metric | Value |
|---|---|
| Opcodes to first prompt | **23,747,546** |
| Wall-clock (release) | **3.138 s** |

This matches the plan's cited ballpark of "~23.7M opcodes of Inform 7 initialization."

## Step 2: Confirming the lever

### A surprising wrinkle: `accel_funcs` was empty at first prompt

The naive tally-by-`accel_func_for` check came back **all `None`** — `Machine.accel_funcs` was empty the entire run. Root cause: gvm's `gestalt` opcode currently hardcodes selector 9 (`Acceleration`) to `0` (line ~1217 in `exec.rs`, pre-existing, "not yet implemented"). CounterfeitMonkey's compiled Inform 6 veneer checks `@gestalt 9 0` before calling `@accelfunc` at all, and skips installing the table when the interpreter reports no support. This is a defensive compiler optimization, not a spec requirement (`@accelfunc`, opcode `0x180`, is itself unconditionally dispatched by gvm and would record the assignment regardless of gestalt) — but it means **the current tree can never observe which functions the game intends to accelerate**, because the game never tells it.

This does not block the go/no-go: it only means identity of the accel-candidate functions had to be established by a temporary diagnostic probe rather than read directly off `accel_func_for` in the true baseline run.

### Per-function tally (true baseline, accel off, unmodified tree)

Opcodes bucketed by the entry address of the function whose frame was executing at each dispatched opcode (frame-pointer → entry-address map, populated at every `build_frame_and_enter`). Top entries:

| Rank | Func addr | Opcodes | Share of total |
|---|---|---|---|
| 1 | `0x0026468d` | 7,795,996 | 32.83% |
| 2 | `0x00263e1a` | 4,016,162 | 16.92% |
| 3 | `0x00264742` | 3,679,144 | 15.50% |
| 4 | `0x00263f8b` | 3,161,717 | 13.32% |
| 5 | `0x00113895` | 1,159,404 | 4.88% *(not an accel candidate — see below)* |
| 6 | `0x00263e9e` | 1,115,216 | 4.70% |
| 7 | `0x00263b98` | 769,802 | 3.24% |
| 8 | `0x00263f28` | 547,118 | 2.30% |
| 9 | `0x001efafa` | 362,280 | 1.53% |
| 10 | `0x002572ad` | 191,212 | 0.81% |
| … | (long tail: ~20 distinct addrs around `0x001c60xx`–`0x001c64xx`, each ≈12,793 opcodes — a per-object init loop, unrelated to acceleration) | | |

Six of the top eight addresses (everything except rank 5, `0x00113895`) are the classic Inform 6 Glulx veneer functions. **Their combined share of the true baseline is 21,085,155 / 23,747,546 = 88.79%.**

### Confirming identity + accel-number assignment (temporary probe, reverted)

To positively identify those six addresses as the accelerable veneer functions (rather than inferring by opcode-count alone), I temporarily flipped the hardcoded `gestalt(9, _)` return from `0` to `1` (a one-line, reverted-after edit — not part of any committed change) and reran. With gestalt reporting acceleration support, the game's veneer *did* call `@accelfunc`, installing exactly **7** assignments — funcnums 1–7, the full "V1" variant set from the algorithm reference:

| Func addr | accelfunc # | Name (algorithm reference) | Opcodes (probe run) |
|---|---|---|---|
| `0x0026468d` | 1 | `Z__Region` | 7,801,773 |
| `0x00263e1a` | 3 | `RA__Pr` | 4,019,341 |
| `0x00264742` | 2 | `CP__Tab` | 3,681,456 |
| `0x00263f8b` | 5 | `OC__Cl` | 3,161,731 |
| `0x00263e9e` | 4 | `RL__Pr` | 1,115,216 |
| `0x00263b98` | 6 | `RV__Pr` | 770,358 |
| `0x00263f28` | 7 | `OP__Pr` | 548,090 |

Probe run totals: 23,780,550 opcodes (vs. 23,747,546 in the true baseline — a ~0.14% difference, expected: acceleration-support detection changes a handful of downstream branches). Accel-candidate share in the probe run: 21,097,965 / 23,780,550 = **88.72%**, matching the address-identity computation from the true baseline (88.79%) within noise. The addresses, ranks, and opcode counts line up 1:1 between the two runs, so this is the same code both times — the probe only adds the `@accelfunc` bookkeeping calls (7 opcodes, noise) and confirms names/numbers, it does not change what code the game runs.

Note funcnums 8–13 (the "V2" variant, different `CP__Tab` offset) are **not** installed by this story — CounterfeitMonkey uses the V1 attribute-byte layout only. Function 5 (`0x00113895`, 4.88%) is genuinely not an accel candidate — some other init-time hotspot (not investigated further; out of scope for the gate).

## Step 3: Go / No-Go

**GO.** Accel-candidate veneer functions account for **~88.8% of opcodes executed to the first prompt** in CounterfeitMonkey-11's Inform 7 initialization (21.09M of 23.75M opcodes, true baseline; corroborated at 88.72% by the independent probe run). This is a clear, dominant majority — accelerating even the top 4 functions (`Z__Region`, `RA__Pr`, `CP__Tab`, `OC__Cl`, together 78.6%) would collapse the bulk of init cost. Proceed to Task 1.

**Caveat for implementers:** the current tree's `gestalt(9, _)` hardcodes "unsupported," which suppresses the game's own `@accelfunc` calls. Task 1 (gestalt reporting) is a prerequisite not just for correctness but for the *game to even attempt* using acceleration — until it lands, `accel_funcs` will stay empty on any real story regardless of whether native interception is implemented.

## Methodology notes / what was reverted

- Permanent (committed in `aca3627`): `Machine.insn_count: u64` field + `self.insn_count += 1;` at the top of `step_once`.
- Temporary (added, measured, then `git checkout`-reverted — confirmed zero diff against `aca3627`):
  - `profile_fp_entry: HashMap<usize, u32>` (frame pointer → function entry address), populated in `build_frame_and_enter`.
  - An `#[ignore]`d test `profile_baseline_counterfeit_monkey` in `exec.rs`'s test module: loads the gblorb (manual top-level IFF `GLUL`-chunk extraction, no new dependency), runs `step()` to first `NeedLine` (auto-supplying a space char on any incidental `NeedChar`), tallies opcodes per function, prints the report.
  - A one-line flip of `gestalt(9, _)` from `0` to `1`, used only for the identity-confirmation probe described above.
- Single run each (no statistical averaging) — wall-clock numbers are indicative, not rigorous benchmarks; the opcode counts are exact and deterministic (same PRNG seed, same input).
