# Glulx Acceleration (accelfunc interception) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make gvm honor the `accelfunc`/`accelparam` tables it already stores by intercepting the 13 well-known accelerated functions with native Rust, collapsing Inform 7 init cost so large Glulx stories reach the first prompt far faster.

**Architecture:** A new `impl Machine` split in `crates/gvm/src/accel.rs` implements the 13 functions as native computations over the memory image + `accel_params`. `exec.rs` checks for an installed, implemented accel number at the two frame-entry choke points (`call_function`, `op_tailcall`) and delivers the native result without building a VM frame. On by default; a `--no-accel` flag disables it in gvm-cli and the app.

**Tech Stack:** Rust, zero-dependency `gvm` crate. Design spec: `docs/superpowers/specs/2026-07-02-glulx-acceleration-design.md`. Algorithm reference: `docs/superpowers/plans/2026-07-02-glulx-acceleration-algorithms.md` (**authoritative — the function bodies come from here + the cited accel.c/spec; do not guess**).

## Global Constraints

- `gvm` stays **zero-dependency**. `gvm-cli` / `app` use their existing arg parsing (manual `env::args` scan / `clap`).
- Cross-platform (Windows/Linux/macOS) — no OS-specific code.
- Native accelerated functions must be **behaviorally transparent**: with the story on-vs-off equivalence test green, an accelerated run is indistinguishable from an interpreted run except in speed. Accelerated functions never call a VM function and never push a frame.
- Faithfulness: transcribe each function from the algorithm reference; do **not** substitute `7` for an unset `num_attr_bytes`; the sole V1/V2 divergence is the CP__Tab offset.
- Ship ritual: green gate on touched crates (`cargo build --tests` + `cargo test`), `scripts/todo-done`, README update, `Completes:` trailer (auto), Co-Authored-By / Claude-Session trailers on every commit.
- gvm memory API: `self.m8(a)?` / `m16` / `m32` (= Mem1/2/4, return `R<u32>`); `self.mem.ramstart()`, `self.mem.endmem()`; `self.accel_param(i) -> Option<u32>`; `self.accel_func_for(addr) -> Option<u32>`; `self.store(dest, v) -> R<()>`; `self.return_value(v) -> R<()>`; `Dest` is `pub(crate)`.

---

## Task 0: Perf baseline (gated evidence)

**Files:**
- Create: `docs/superpowers/plans/2026-07-02-accel-baseline.md` (findings record; not code)

**Interfaces:**
- Produces: the baseline number (opcodes + wall-clock to first prompt, accel off) that Task 8 proves the win against, and a go/no-go on whether accel candidates dominate.

- [ ] **Step 1: Measure baseline.** With the current tree (acceleration not yet implemented), run `stories/CounterfeitMonkey-11.gblorb` in gvm-cli **release** to the first input prompt. Capture: total opcodes executed to first prompt, wall-clock. If gvm has no opcode counter, add a temporary local counter (do NOT commit it) or use an existing diagnostic.

- [ ] **Step 2: Confirm the lever.** Produce a per-function (or per-address) tally of where those opcodes are spent, and confirm the accel-candidate veneer functions (property/class lookups — the ones a game assigns accel numbers to via `accelfunc`) dominate. Record which `func_addr`s carry accel assignments (`accel_func_for`) and roughly what share of init opcodes they represent.

- [ ] **Step 3: Go / no-go.** Write findings to `docs/superpowers/plans/2026-07-02-accel-baseline.md`. **If accel candidates do NOT dominate, STOP** — acceleration is the wrong lever; report to the human and do not proceed to Task 1. If they dominate (expected), record the baseline and proceed.

- [ ] **Step 4: Commit** the findings doc.

```bash
git add docs/superpowers/plans/2026-07-02-accel-baseline.md
git commit -m "docs(perf): Glulx accel baseline — CounterfeitMonkey init profile"
```

---

## Task 1: Machine acceleration flag, setter, and gestalt

**Files:**
- Modify: `crates/gvm/src/exec.rs` (Machine struct + initializer(s); gestalt selectors 9/10 at ~1208; update the two accel-unsupported tests at ~4170 and ~4550)

**Interfaces:**
- Produces: `Machine.acceleration: bool` (default `true`); `pub fn set_acceleration(&mut self, on: bool)`; gestalt 9→1, 10→`1` iff function number ∈ 1..=13. Free fn `crate::accel::accel_impl_supported` is defined in Task 2 — for this task, inline the range check `(1..=13).contains(&arg)` in gestalt 10 and replace it with the free fn in Task 2.

- [ ] **Step 1: Write the failing tests.** Update the existing accel-unsupported assertions and add a setter test. In `exec.rs` tests, change the two assertions that currently expect accel unsupported (search `gestalt(9, 0), 0` and `gestalt(10, 0), 0`) to the new truth, and add:

```rust
#[test]
fn gestalt_reports_acceleration_supported() {
    let m = Machine::new(Memory::new(minimal_image()).unwrap());
    assert_eq!(m.gestalt(9, 0), 1);   // Acceleration: interception implemented
    assert_eq!(m.gestalt(10, 0), 0);  // AccelFunc 0 is "cancel", not a function
    assert_eq!(m.gestalt(10, 1), 1);  // Z__Region implemented
    assert_eq!(m.gestalt(10, 13), 1); // last implemented
    assert_eq!(m.gestalt(10, 14), 0); // beyond the set
}

#[test]
fn acceleration_defaults_on_and_toggles() {
    let mut m = Machine::new(Memory::new(minimal_image()).unwrap());
    assert!(m.acceleration);
    m.set_acceleration(false);
    assert!(!m.acceleration);
}
```

(Use whatever minimal-image helper the existing accel tests use; match their construction.)

- [ ] **Step 2: Run to verify they fail.** `cargo test -p gvm acceleration 2>&1 | tail` — expect failures (field/setter absent, gestalt still 0).

- [ ] **Step 3: Implement.** Add the field to the `Machine` struct (near `accel_funcs`), init `acceleration: true` in the initializer(s) (`Machine::new` / `Machine::with_glk` share the struct literal near the other field inits), add the setter, and update gestalt:

```rust
// struct Machine { ... near accel_params ... }
/// Whether accelerated-function interception is active (default true).
pub(crate) acceleration: bool,
```
```rust
// in the Machine initializer, alongside accel_funcs / accel_params:
acceleration: true,
```
```rust
/// Enable/disable accelerated-function interception (debug escape hatch).
pub fn set_acceleration(&mut self, on: bool) {
    self.acceleration = on;
}
```
```rust
// gestalt() match, replacing lines 1208-1209:
9 => 1,                                        // Acceleration: interception implemented
10 => u32::from((1..=13).contains(&arg)),      // AccelFunc: implemented function numbers
```

- [ ] **Step 4: Run to verify pass.** `cargo test -p gvm 2>&1 | tail` — the new tests pass; no regressions.

- [ ] **Step 5: Commit.**

```bash
git add crates/gvm/src/exec.rs
git commit -m "feat(gvm): acceleration flag + setter; gestalt reports accel supported"
```

---

## Task 2: `accel.rs` scaffold — params, Z__Region, dispatch

**Files:**
- Create: `crates/gvm/src/accel.rs`
- Modify: `crates/gvm/src/lib.rs` or `exec.rs` (add `mod accel;` / `pub(crate) mod accel;` so the `impl Machine` split compiles)

**Interfaces:**
- Consumes: `Machine.m8/m16/m32`, `mem.ramstart/endmem`, `accel_param`.
- Produces: `impl Machine { pub(crate) fn accel_dispatch(&self, num: u32, args: &[u32]) -> R<u32> }` (implemented for `1`; `2..=13` return a placeholder `Ok(0)` for now — filled in Task 3); free fn `pub(crate) fn accel_impl_supported(num: u32) -> bool`; private `arg`, `param`, `z_region`. Later tasks call `accel_dispatch` from the interception hooks.

- [ ] **Step 1: Write the failing test** (in `accel.rs` `#[cfg(test)]`). Build a tiny image and assert `z_region` classification. Use the crate's existing test-image helpers (mirror `exec.rs` accel tests). Test the four regions:

```rust
#[test]
fn z_region_classifies_addresses() {
    // Build an image with: a byte < ramstart (region 0 for addr<36 or non-object),
    // an object header byte 0x70..0x7F at addr >= ramstart (region 1),
    // a routine byte 0xC0 (region 2), a string byte 0xE0 (region 3).
    let m = accel_test_machine();       // helper: see Step 3 note
    assert_eq!(m.accel_dispatch(1, &[10]).unwrap(), 0);          // addr < 36
    assert_eq!(m.accel_dispatch(1, &[OBJ_ADDR]).unwrap(), 1);    // object
    assert_eq!(m.accel_dispatch(1, &[ROUTINE_ADDR]).unwrap(), 2);// routine
    assert_eq!(m.accel_dispatch(1, &[STRING_ADDR]).unwrap(), 3); // string
    assert_eq!(m.accel_dispatch(1, &[]).unwrap(), 0);            // no arg -> addr 0
}
```

- [ ] **Step 2: Run to verify it fails.** `cargo test -p gvm z_region 2>&1 | tail` — fails (module/fn absent).

- [ ] **Step 3: Implement the scaffold.** Add `pub(crate) mod accel;` to the crate root, then write `accel.rs`:

```rust
//! Native implementations of the 13 well-known Glulx accelerated functions
//! (`@accelfunc`). See docs/superpowers/plans/2026-07-02-glulx-acceleration-algorithms.md
//! (authoritative) and the Glulx spec §2.17 / Glulxe accel.c.

use crate::exec::{Machine, R};

/// True iff `num` names an accelerated function this VM implements (1..=13).
pub(crate) fn accel_impl_supported(num: u32) -> bool {
    (1..=13).contains(&num)
}

/// The V1 (funcs 2-7) vs V2 (funcs 8-13) family; differ only in the CP__Tab offset.
#[derive(Clone, Copy, PartialEq)]
enum Variant { V1, V2 }

impl Machine {
    /// Run accelerated function `num` (assumed 1..=13) with `args`, returning its
    /// value. Never builds a frame; a memory fault propagates as an interpreter error.
    pub(crate) fn accel_dispatch(&self, num: u32, args: &[u32]) -> R<u32> {
        match num {
            1 => self.accel_z_region(args),
            2 => self.accel_cp_tab(args, Variant::V1),
            3 => self.accel_ra_pr(args, Variant::V1),
            4 => self.accel_rl_pr(args, Variant::V1),
            5 => self.accel_oc_cl(args, Variant::V1),
            6 => self.accel_rv_pr(args, Variant::V1),
            7 => self.accel_op_pr(args, Variant::V1),
            8 => self.accel_cp_tab(args, Variant::V2),
            9 => self.accel_ra_pr(args, Variant::V2),
            10 => self.accel_rl_pr(args, Variant::V2),
            11 => self.accel_oc_cl(args, Variant::V2),
            12 => self.accel_rv_pr(args, Variant::V2),
            13 => self.accel_op_pr(args, Variant::V2),
            _ => Ok(0),
        }
    }

    #[inline]
    fn accel_arg(args: &[u32], i: usize) -> u32 { args.get(i).copied().unwrap_or(0) }
    #[inline]
    fn accel_param_or0(&self, i: u32) -> u32 { self.accel_param(i).unwrap_or(0) }

    /// Function 1 — Z__Region.
    fn accel_z_region(&self, args: &[u32]) -> R<u32> {
        let addr = Self::accel_arg(args, 0);
        if addr < 36 || addr >= self.mem.endmem() { return Ok(0); }
        let tb = self.m8(addr)?;
        Ok(if tb >= 0xE0 { 3 }
           else if tb >= 0xC0 { 2 }
           else if (0x70..=0x7F).contains(&tb) && addr >= self.mem.ramstart() { 1 }
           else { 0 })
    }

    // Task 3 adds: accel_cp_tab, accel_ra_pr, accel_rl_pr, accel_oc_cl,
    // accel_rv_pr, accel_op_pr, and the obj_in_class / get_prop / binsearch helpers.
    // For now the 2..=13 arms above call methods that Task 3 defines; to keep this
    // task compiling on its own, temporarily stub them (see Step 3b).
}
```

- [ ] **Step 3b: Keep Task 2 compiling standalone.** Since Task 3 adds `accel_cp_tab` etc., either (a) land Task 2 with the dispatch arms `2..=13 => Ok(0)` and the per-function methods added in Task 3 (recommended — smaller diff), or (b) include empty `fn accel_cp_tab(...) -> R<u32> { Ok(0) }` stubs now. Use (a): replace arms `2..=13` with a single `2..=13 => Ok(0), // filled in Task 3` line and drop the per-function calls until Task 3. Adjust the match accordingly so it compiles.

- [ ] **Step 4: Run to verify pass.** `cargo test -p gvm z_region 2>&1 | tail` — passes. `cargo build -p gvm --tests` clean.

- [ ] **Step 5: Wire gestalt to the free fn.** Replace the inline `(1..=13).contains(&arg)` from Task 1 with `crate::accel::accel_impl_supported(arg)`; run `cargo test -p gvm gestalt 2>&1 | tail`.

- [ ] **Step 6: Commit.**

```bash
git add crates/gvm/src/accel.rs crates/gvm/src/lib.rs crates/gvm/src/exec.rs
git commit -m "feat(gvm): accel module scaffold — params, Z__Region, dispatch"
```

---

## Task 3: `accel.rs` — the six property/class functions (V1 + V2)

**Files:**
- Modify: `crates/gvm/src/accel.rs`

**Interfaces:**
- Consumes: the scaffold from Task 2.
- Produces: full `accel_dispatch` coverage of 2..=13 via `accel_cp_tab`, `accel_ra_pr`, `accel_rl_pr`, `accel_oc_cl`, `accel_rv_pr`, `accel_op_pr`, and helpers `obj_in_class`, `get_prop`, `binsearch_prop`, `accel_error`.

**Reference:** implement each body from `2026-07-02-glulx-acceleration-algorithms.md` (§"Shared helpers" + §"The 13 functions"). The mutual recursion `get_prop ↔ accel_oc_cl` is expected (all `&self` reads — no borrow conflict). Every read is `?`-propagated.

- [ ] **Step 1: Write the failing tests** — a synthetic object table exercising both variants and the key edge cases. Build a small image in-test containing: one object with a small property table (one common property with a known value, one absent), a class object, and the `classes_table`; set params 0–8 (with `num_attr_bytes = 7`). Assert:

```rust
#[test]
fn ra_rl_rv_on_synthetic_object() {
    let m = accel_world();  // helper builds the object table + sets params
    // present property P on OBJ: address non-zero, length as laid out, value dereferenced
    assert_eq!(m.accel_dispatch(3, &[OBJ, P]).unwrap(), PROP_ADDR);        // RA__Pr v1
    assert_eq!(m.accel_dispatch(9, &[OBJ, P]).unwrap(), PROP_ADDR);        // RA__Pr v2 (same, nab=7)
    assert_eq!(m.accel_dispatch(4, &[OBJ, P]).unwrap(), PROP_LEN_BYTES);   // RL__Pr
    assert_eq!(m.accel_dispatch(6, &[OBJ, P]).unwrap(), PROP_VALUE);       // RV__Pr
    // absent property Q -> RA/RL 0; RV falls back to cpv__start default for common props
    assert_eq!(m.accel_dispatch(3, &[OBJ, Q_ABSENT]).unwrap(), 0);
    assert_eq!(m.accel_dispatch(6, &[OBJ, Q_ABSENT_COMMON]).unwrap(), CPV_DEFAULT);
}

#[test]
fn oc_cl_and_op_pr_classify() {
    let m = accel_world();
    assert_eq!(m.accel_dispatch(5, &[OBJ, THE_CLASS]).unwrap(), 1);   // OC__Cl: obj is of class
    assert_eq!(m.accel_dispatch(5, &[OBJ, OTHER_CLASS]).unwrap(), 0);
    assert_eq!(m.accel_dispatch(7, &[OBJ, P]).unwrap(), 1);          // OP__Pr: provides P
    assert_eq!(m.accel_dispatch(7, &[OBJ, Q_ABSENT]).unwrap(), 0);
    // Z__Region routing inside OP__Pr: routine provides only `call` (indiv+5)
    assert_eq!(m.accel_dispatch(7, &[ROUTINE_ADDR, INDIV+5]).unwrap(), 1);
}

#[test]
fn cp_tab_v1_v2_agree_at_nab7_and_diverge_otherwise() {
    // With num_attr_bytes = 7 the V1 (obj+16) and V2 (obj+4*(3+7/4)=obj+16) offsets match.
    let m = accel_world();
    assert_eq!(m.accel_dispatch(2, &[OBJ, P]).unwrap(),
               m.accel_dispatch(8, &[OBJ, P]).unwrap());
    // With num_attr_bytes != 7, only V2 lands on the real table. Build a second world
    // whose object places its prop-table pointer at obj+4*(3+ nab/4); assert V2 finds
    // the property and V1 (obj+16) does not.
    let m2 = accel_world_nab(9);
    assert!(m2.accel_dispatch(8, &[OBJ, P]).unwrap() != 0);
    assert_eq!(m2.accel_dispatch(2, &[OBJ, P]).unwrap(), 0);
}
```

(Compute the expected addresses/values from your in-test layout; keep the object table minimal but real. The `accel_world*` helpers are test-local.)

- [ ] **Step 2: Run to verify they fail.** `cargo test -p gvm accel 2>&1 | tail` — fail (methods absent / `Ok(0)` stubs).

- [ ] **Step 3: Implement** all helpers and the six functions per the algorithm reference. Replace the Task-2 `2..=13 => Ok(0)` arms with the real dispatch to these methods:

```rust
fn accel_error(&self, _msg: &str) {
    // accel.c writes msg to the current Glk stream; correct games never reach these
    // programming-error paths, so we record a diagnostic instead (see algorithms.md).
    // Note: takes &self; if diagnostics needs &mut, route via a Cell/skip — keep it
    // side-effect-light. Simplest: drop the message (documented). Prefer a no-op that
    // returns, matching "no output under filter iosys".
}

fn obj_in_class(&self, obj: u32) -> R<bool> {
    let nab = self.accel_param_or0(7);
    Ok(self.m32(obj.wrapping_add(13).wrapping_add(nab))? == self.accel_param_or0(2))
}

/// Binary search the property table: `num` 10-byte records from `start`, 2-byte
/// big-endian key at record offset 0. Returns the matching record address or 0.
fn binsearch_prop(&self, key: u32, start: u32, num: u32) -> R<u32> {
    let key = key & 0xFFFF;
    let (mut lo, mut hi) = (0u32, num);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let rec = start.wrapping_add(mid.wrapping_mul(10));
        let have = self.m16(rec)?;
        match have.cmp(&key) {
            std::cmp::Ordering::Equal => return Ok(rec),
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
        }
    }
    Ok(0)
}

fn accel_cp_tab(&self, args: &[u32], variant: Variant) -> R<u32> {
    let obj = Self::accel_arg(args, 0);
    let id = Self::accel_arg(args, 1);
    if self.accel_z_region(&[obj])? != 1 {
        self.accel_error("[** Programming error: tried to find the \".\" of (something) **]");
        return Ok(0);
    }
    let otab = match variant {
        Variant::V1 => self.m32(obj.wrapping_add(16))?,
        Variant::V2 => {
            let nab = self.accel_param_or0(7);
            self.m32(obj.wrapping_add(4u32.wrapping_mul(3 + nab / 4)))?
        }
    };
    if otab == 0 { return Ok(0); }
    let max = self.m32(otab)?;
    self.binsearch_prop(id, otab + 4, max)
}

fn get_prop(&self, mut obj: u32, mut id: u32, variant: Variant) -> R<u32> {
    let mut cla = 0u32;
    if id & 0xFFFF_0000 != 0 {
        cla = self.m32(self.accel_param_or0(0).wrapping_add((id & 0xFFFF).wrapping_mul(4)))?;
        if self.accel_oc_cl(&[obj, cla], variant)? == 0 { return Ok(0); }
        id >>= 16;
        obj = cla;
    }
    let prop = self.accel_cp_tab(&[obj, id], variant)?;
    if prop == 0 { return Ok(0); }
    if self.obj_in_class(obj)? && cla == 0 {
        let ips = self.accel_param_or0(1);
        if id < ips || id >= ips + 8 { return Ok(0); }
    }
    if self.m32(self.accel_param_or0(6))? != obj && self.m8(prop + 9)? & 1 != 0 {
        return Ok(0);
    }
    Ok(prop)
}

fn accel_ra_pr(&self, args: &[u32], variant: Variant) -> R<u32> {
    let prop = self.get_prop(Self::accel_arg(args, 0), Self::accel_arg(args, 1), variant)?;
    if prop == 0 { Ok(0) } else { self.m32(prop + 4) }
}

fn accel_rl_pr(&self, args: &[u32], variant: Variant) -> R<u32> {
    let prop = self.get_prop(Self::accel_arg(args, 0), Self::accel_arg(args, 1), variant)?;
    if prop == 0 { Ok(0) } else { Ok(4 * self.m16(prop + 2)?) }
}

fn accel_oc_cl(&self, args: &[u32], variant: Variant) -> R<u32> {
    // Transcribe the full branch ladder from algorithms.md §5 (5/11). Uses params
    // 2/3/4/5, obj_in_class, and — in the general case — get_prop(obj, 2, variant),
    // reading Mem4(prop+4) as inlist and Mem2(prop+2) as inlistlen, scanning for cla.
    // ... (implement exactly as the reference; return 0/1) ...
}

fn accel_rv_pr(&self, args: &[u32], variant: Variant) -> R<u32> {
    let id = Self::accel_arg(args, 1);
    let addr = self.accel_ra_pr(args, variant)?;
    if addr == 0 {
        if id > 0 && id < self.accel_param_or0(1) {
            return self.m32(self.accel_param_or0(8).wrapping_add(4 * id));
        }
        self.accel_error("[** Programming error: tried to read (something) **]");
        return Ok(0);
    }
    self.m32(addr)
}

fn accel_op_pr(&self, args: &[u32], variant: Variant) -> R<u32> {
    // Transcribe from algorithms.md §5 (7/13): Z__Region routing for string/routine,
    // then the indiv_prop_start..+8 obj_in_class shortcut, else RA__Pr != 0.
    // ... (implement exactly as the reference; return 0/1) ...
}
```

> Fill the two elided bodies (`accel_oc_cl`, `accel_op_pr`) verbatim from the algorithm reference — they are spelled out there line-by-line. Do not paraphrase.

- [ ] **Step 4: Run to verify pass.** `cargo test -p gvm accel 2>&1 | tail` — all Task-3 tests pass; `cargo test -p gvm` shows no regressions.

- [ ] **Step 5: Commit.**

```bash
git add crates/gvm/src/accel.rs
git commit -m "feat(gvm): native CP__Tab/RA/RL/OC/RV/OP accelerated functions (V1+V2)"
```

---

## Task 4: Interception at `call_function` and `op_tailcall`

**Files:**
- Modify: `crates/gvm/src/exec.rs` (`call_function` ~901; `op_tailcall` ~1835)

**Interfaces:**
- Consumes: `accel_dispatch`, `accel_impl_supported`, `self.store`, `self.return_value`.
- Produces: accelerated calls bypass frame construction.

- [ ] **Step 1: Write the failing tests.** Prove the native path is taken for both call forms. Assemble an image with a function at `FADDR` whose *bytecode* would return a sentinel `0xBAD`, assign it accel number 1 (Z__Region) via `@accelfunc`, then call it with an argument that Z__Region maps to a known non-`0xBAD` value; assert the accelerated result, and assert `--no-accel` (setter off) yields the bytecode result:

```rust
#[test]
fn call_uses_accelerated_function_when_installed() {
    let mut m = accel_installed_machine();  // FADDR assigned accel #1; bytecode returns 0xBAD
    m.call_function(FADDR, &[ROUTINE_ADDR], Dest::Push).unwrap();
    // run to the call's completion if needed; then check pushed value
    assert_eq!(top_of_stack(&m), 2);        // Z__Region(routine) == 2, not 0xBAD
}

#[test]
fn tailcall_uses_accelerated_function() {
    // f1 tailcalls FADDR(accel #1); result must reach f1's caller stub.
    let mut m = accel_tailcall_machine();
    // ... step to the tailcall; assert the delivered value is the Z__Region result ...
}

#[test]
fn no_accel_runs_the_bytecode() {
    let mut m = accel_installed_machine();
    m.set_acceleration(false);
    m.call_function(FADDR, &[ROUTINE_ADDR], Dest::Push).unwrap();
    assert_eq!(top_of_stack(&m), 0xBAD);    // interpreted path
}
```

(Adapt to the crate's existing call-test harness — see the `call_function` tests near exec.rs:3500+ and `tailcall_reuses_caller_stub` at ~3342 for image/step helpers.)

- [ ] **Step 2: Run to verify they fail.** `cargo test -p gvm accel_ 2>&1 | tail` (or the test names) — fail (no interception yet).

- [ ] **Step 3: Implement the two hooks.**

In `call_function`, before `let (dtype, daddr) = dest.to_stub();`:
```rust
pub(crate) fn call_function(&mut self, func_addr: u32, args: &[u32], dest: Dest) -> R<()> {
    if self.acceleration {
        if let Some(num) = self.accel_func_for(func_addr) {
            if crate::accel::accel_impl_supported(num) {
                let result = self.accel_dispatch(num, args)?;
                return self.store(dest, result);
            }
        }
    }
    let (dtype, daddr) = dest.to_stub();
    // ... unchanged ...
}
```

In `op_tailcall`, **after** popping args but **before** `self.sp = self.fp;` (so `return_value` finds the stub intact):
```rust
    for _ in 0..argc {
        args.push(self.pop32()?);
    }
    if self.acceleration {
        if let Some(num) = self.accel_func_for(func) {
            if crate::accel::accel_impl_supported(num) {
                let result = self.accel_dispatch(num, &args)?;
                return self.return_value(result);
            }
        }
    }
    self.sp = self.fp;
    self.build_frame_and_enter(func, &args)
```

- [ ] **Step 4: Run to verify pass.** `cargo test -p gvm 2>&1 | tail` — new tests pass; full gvm suite green.

- [ ] **Step 5: Commit.**

```bash
git add crates/gvm/src/exec.rs
git commit -m "feat(gvm): intercept accelerated functions at call_function + tailcall"
```

---

## Task 5: Differential harness (best effort)

**Files:**
- Modify: `crates/gvm/src/exec.rs` or `crates/gvm/tests/accel_differential.rs` (test-only)

**Interfaces:**
- Consumes: `accel_dispatch`, the call/step harness.
- Produces: for the tractable functions, a test asserting native == interpreted for the same `func_addr`/args.

- [ ] **Step 1: Implement a differential test for Z__Region** (and one property function if a faithful veneer transcription is tractable). Assemble an image whose function at `FADDR` is a hand-written Glulx transcription of Z__Region; run it interpreted (`call_function` with `set_acceleration(false)`) and compare to `accel_dispatch(1, args)` for a spread of inputs (below 36, ≥ endmem, object/routine/string bytes). They must be equal.

```rust
#[test]
fn differential_z_region_matches_interpreter() {
    for &addr in &[0, 35, 36, OBJ_ADDR, ROUTINE_ADDR, STRING_ADDR, endmem_probe()] {
        let native = { let m = accel_world(); m.accel_dispatch(1, &[addr]).unwrap() };
        let interp = run_interpreted_z_region(addr);   // set_acceleration(false)
        assert_eq!(native, interp, "Z__Region diverged at {addr:#x}");
    }
}
```

- [ ] **Step 2:** If a faithful transcription of a property function proves too costly, **stop here** — record in the test module a comment noting differential coverage is Z__Region-only and the full-story on/off equivalence (Task 8) is the primary guarantee. Do not block the feature.

- [ ] **Step 3: Run + commit.** `cargo test -p gvm differential 2>&1 | tail`.

```bash
git add crates/gvm/
git commit -m "test(gvm): best-effort differential accel-vs-interpreter equivalence"
```

---

## Task 6: gvm-cli `--no-accel`

**Files:**
- Modify: `crates/gvm-cli/src/main.rs` (~150-160, the arg scan; usage string)

**Interfaces:**
- Consumes: `Machine::set_acceleration`.
- Produces: `--no-accel` disables interception; default on.

- [ ] **Step 1: Implement** mirroring `--no-game-colours`. After the machine is constructed (before the run loop):
```rust
let accel = !argv.iter().any(|a| a == "--no-accel");
// ... after building the Machine `m`:
m.set_acceleration(accel);
```
Update the usage string to include `[--no-accel]`.

- [ ] **Step 2: Verify build + a manual smoke.** `cargo build -p gvm-cli`; run a small story with and without `--no-accel` and confirm both reach the prompt. (No new unit test required — flag parsing mirrors the existing colour flag; keep it minimal.)

- [ ] **Step 3: Commit.**

```bash
git add crates/gvm-cli/src/main.rs
git commit -m "feat(gvm-cli): --no-accel flag (acceleration on by default)"
```

---

## Task 7: app `--no-accel`

**Files:**
- Modify: `crates/app/src/config.rs` (the clap `Cli` struct, ~164)
- Modify: `crates/app/src/glulx_session.rs` (`GlulxSession::new` signature, ~97)
- Modify: `crates/app/src/main.rs` (the two `GlulxSession::new` call sites ~1548 and ~3887; the test-only ctor at ~4945 gets the new arg)

**Interfaces:**
- Consumes: `Machine::set_acceleration`.
- Produces: `Cli.no_accel`; `GlulxSession::new(image, cols, rows, acceleration)` applies the flag **before** its internal `drive()` runs init.

- [ ] **Step 1: Add the clap flag** to `Cli` in config.rs:
```rust
/// Disable Glulx accelerated-function interception (debug; default: enabled)
#[arg(long)]
pub no_accel: bool,
```

- [ ] **Step 2: Thread it into `GlulxSession::new`** so acceleration is set on the machine **before** `drive()`:
```rust
pub fn new(image: Vec<u8>, cols: u32, rows: u32, acceleration: bool) -> Result<GlulxSession, GError> {
    let mem = Memory::new(image)?;
    let backend = Box::new(AppGlk::new(cols, rows));
    let mut machine = Machine::with_glk(mem, backend);
    machine.set_acceleration(acceleration);      // <-- before drive() runs init
    let (pending, quit) = drive(&mut machine);
    // ... unchanged ...
}
```

- [ ] **Step 3: Update the call sites.** In main.rs pass `!cli.no_accel` (thread the parsed `Cli`/config value to where the story loads). At ~1548 and ~3887 add the argument; at the test ctor ~4945 pass `true`. Grep `GlulxSession::new(` to catch every site.

- [ ] **Step 4: Verify.** `cargo build -p app`; `cargo test -p app 2>&1 | tail` green. If a config→session wiring value is needed, mirror how `honor_game_colours` reaches `GameSession::new`.

- [ ] **Step 5: Commit.**

```bash
git add crates/app/src/config.rs crates/app/src/glulx_session.rs crates/app/src/main.rs
git commit -m "feat(app): --no-accel flag threaded into GlulxSession before init"
```

---

## Task 8: Full-story on/off equivalence + perf assertion

**Files:**
- Create: `crates/gvm/tests/accel_story_equivalence.rs`

**Interfaces:**
- Consumes: gvm-cli/gvm library run-to-first-prompt path; `set_acceleration`.
- Produces: the primary anti-divergence guarantee + the proven speed win.

- [ ] **Step 1: Implement the equivalence test.** Load `stories/CounterfeitMonkey-11.gblorb` (and one more Glulx title present under `stories/`), run to the first input prompt twice — `set_acceleration(true)` and `set_acceleration(false)` — and assert:
  - identical output transcript to the first prompt, and
  - identical detected starting room / screen state, and
  - the accelerated run executes **substantially fewer** opcodes (assert a large reduction vs the Task-0 baseline, e.g. `accel_opcodes * 3 < interp_opcodes` — pick a margin the baseline clearly supports; do not hard-code an exact count).

```rust
#[test]
fn counterfeit_monkey_accel_matches_interpreted_and_is_faster() {
    let (out_on, ops_on) = run_to_first_prompt(CM_PATH, true);
    let (out_off, ops_off) = run_to_first_prompt(CM_PATH, false);
    assert_eq!(out_on, out_off, "accel changed the transcript");
    assert!(ops_on * 3 < ops_off, "accel not materially faster: {ops_on} vs {ops_off}");
}
```

- [ ] **Step 2: Gate heavy assets if needed.** If CounterfeitMonkey is too slow for the default test tier, mark that specific test `#[ignore]` with a note (run in CI/manually); keep a smaller Glulx title in the default run if one exists. Record the decision in the test file header. **`log`/note any such cap — do not silently skip.**

- [ ] **Step 3: Run + commit.** `cargo test -p gvm --test accel_story_equivalence 2>&1 | tail` (or with `-- --ignored`).

```bash
git add crates/gvm/tests/accel_story_equivalence.rs
git commit -m "test(gvm): accel on/off story equivalence + speed assertion"
```

---

## Task 9: Docs + TODO close

**Files:**
- Modify: `crates/gvm/GLULX_NOTES.md` (§17 rewrite; gestalt table lines for 9/10)
- Modify: `README.md` (one user-facing line)
- Modify: `TODO.md` → `COMPLETED.md` (the re-scoped interpreter-throughput item), via `scripts/todo-done`

**Interfaces:** none (docs).

- [ ] **Step 1: Rewrite GLULX_NOTES §17** to state interception is now implemented (13 functions, two hook sites, on-by-default with `--no-accel`), and update the gestalt table so 9→1 and 10→"1 for function numbers 1–13".

- [ ] **Step 2: README line** — under the Glulx/features section, note that large Glulx games (e.g. CounterfeitMonkey) start substantially faster via accelerated-function interception, disable with `--no-accel`.

- [ ] **Step 3: Close the TODO.**
```bash
scripts/todo-done "Speed up large Glulx game startup"
git add TODO.md COMPLETED.md crates/gvm/GLULX_NOTES.md README.md
git commit -m "docs(gvm): document acceleration interception; close throughput TODO"
```
(The `commit-msg` hook auto-adds the `Completes:` trailer.)

- [ ] **Step 4: Final green gate.** `cargo test` (workspace) green; `cargo build --tests` clean.

---

## Self-Review notes

- **Spec coverage:** Task 0 = perf-baseline gate; Tasks 1–4 = engine (field/gestalt, module, functions, interception); Task 5 = differential (best effort); Tasks 6–7 = escape hatch (cli + app); Task 8 = story equivalence + speed; Task 9 = docs + TODO. Maps to every section of the design spec.
- **Type consistency:** `accel_dispatch(&self, u32, &[u32]) -> R<u32>` and `accel_impl_supported(u32) -> bool` are used identically in gestalt (Task 1/2) and both hooks (Task 4). `Variant` is private to accel.rs. Delivery reuses `store`/`return_value`.
- **Ambiguity resolved:** V1/V2 = CP__Tab offset only; `accel_error` = diagnostic/no-op (documented); story assets may be `#[ignore]`-gated with a logged note.
- **Open risk carried into execution:** the `accel_error` `&self` vs `&mut self` diagnostics detail (Task 3 Step 3) — simplest faithful choice is a no-op that returns, since correct games never hit it and equivalence tests use correct games.
