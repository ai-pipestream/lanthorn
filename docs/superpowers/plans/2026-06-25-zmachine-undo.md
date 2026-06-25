# Z-Machine UNDO — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `save_undo` (EXT:0x09) / `restore_undo` (EXT:0x0A) as a bounded, multi-level, in-memory undo reusing the tested Quetzal save/restore, with a configurable depth.

**Architecture:** A bounded `undo_stack` of `(Quetzal blob, save_undo store target)` lives on the `Machine`. The opcode arms call small `do_save_undo` / `do_restore_undo` methods (testable without instruction encoding) that use `save_quetzal`/`restore_quetzal` and `do_store` inline (no host suspension). The app sets the cap from a new `undo_levels` config.

**Tech Stack:** Rust (zvm + app crates).

## Global Constraints

- Commit trailers on every commit (body, no backticks anywhere in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Zero compiler warnings; remove any symbol your change orphans.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- Undo is **in-memory and inline** — it must NOT return `SaveRequest`/`RestoreRequest`; it `do_store`s immediately (like the current `-1` stubs).
- Store values: `save_undo` → `1` (success) / `-1` i.e. `0xFFFF` (when `undo_cap == 0`, disabled); `restore_undo` → `2` into the **original `save_undo`'s** store target on success, `0` into its own target when the stack is empty.
- `save_quetzal` / `restore_quetzal` snapshot/replace dynamic memory + frames + eval stack + PC. The snapshot PC is the post-`save_undo` address (the standard `step()` advance), so restoring resumes there.
- Undo history is session-only (never written into `.babelmap` saves).
- Depth is `config.undo_levels` (default 16; `0` disables). Run `cargo test -p app` and `cargo test -p zvm` after the tasks that touch each crate: 0 failures, 0 warnings.

---

### Task 1: VM undo store + opcodes

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (`UndoSnapshot` type; `Machine` fields + `Machine::new`; `do_save_undo`/`do_restore_undo`; the `0x09`/`0x0A` arms ~1060-1069; tests)

**Interfaces:**
- Consumes: `self.save_quetzal() -> Vec<u8>`, `self.restore_quetzal(&[u8]) -> Result<(), ZError>`, `self.do_store(Option<u8>, u16)`, `self.global(n)`.
- Produces: `pub struct UndoSnapshot { pub blob: Vec<u8>, pub store: Option<u8> }`; `Machine.undo_stack: Vec<UndoSnapshot>`; `Machine.undo_cap: usize`; `pub(crate) fn do_save_undo(&mut self, store: Option<u8>)`; `pub(crate) fn do_restore_undo(&mut self, store: Option<u8>)`.

- [ ] **Step 1: Write the failing tests**

In `crates/zvm/src/cpu/exec.rs`, inside `mod tests`, add (these use the in-memory methods directly — no instruction encoding):

```rust
#[test]
fn undo_save_restore_round_trip() {
    let mem = Memory::new(sample_story(5)).unwrap();
    let mut m = Machine::new(mem);
    m.undo_cap = 4;
    m.state.pc = 0x0040;
    m.do_store(Some(0x11), 1); // G1 = 1 (pre-save value)

    // save_undo storing to G0: snapshot taken, G0 := 1, one stack entry.
    m.do_save_undo(Some(0x10));
    assert_eq!(m.global(0), 1, "save_undo stores 1");
    assert_eq!(m.undo_stack.len(), 1);

    // Mutate G1, then restore_undo storing to G2.
    m.do_store(Some(0x11), 0x99);
    m.do_restore_undo(Some(0x12));
    assert_eq!(m.global(1), 1, "G1 reverted to the snapshot value");
    assert_eq!(m.global(0), 2, "the original save_undo 'returns' 2");
    assert_eq!(m.state.pc, 0x0040, "PC resumed at the post-save_undo address");
    assert!(m.undo_stack.is_empty(), "snapshot consumed");
}

#[test]
fn undo_empty_and_disabled_and_cap() {
    let mem = Memory::new(sample_story(5)).unwrap();
    let mut m = Machine::new(mem);

    // Empty stack: restore_undo stores 0 into its own target, no state change.
    m.undo_cap = 4;
    m.do_restore_undo(Some(0x10));
    assert_eq!(m.global(0), 0);

    // Disabled (cap 0): save_undo stores -1 (0xFFFF) and pushes nothing.
    m.undo_cap = 0;
    m.do_save_undo(Some(0x11));
    assert_eq!(m.global(1), 0xFFFF, "cap 0 => -1 (unsupported)");
    assert!(m.undo_stack.is_empty());

    // Cap drop: with cap 2, three saves keep the newest two.
    m.undo_cap = 2;
    m.do_save_undo(Some(0x10));
    m.do_save_undo(Some(0x10));
    m.do_save_undo(Some(0x10));
    assert_eq!(m.undo_stack.len(), 2, "oldest dropped past the cap");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p zvm undo_save_restore_round_trip undo_empty_and_disabled_and_cap`
Expected: compile error (type/fields/methods missing).

- [ ] **Step 3: Add the `UndoSnapshot` type**

In `crates/zvm/src/cpu/exec.rs`, near the `Machine` struct, add:

```rust
/// One in-memory undo snapshot: the Quetzal state blob plus the `save_undo`
/// instruction's store target, so `restore_undo` can write 2 back into it.
#[derive(Debug, Clone)]
pub struct UndoSnapshot {
    pub blob: Vec<u8>,
    pub store: Option<u8>,
}
```

- [ ] **Step 4: Add the `Machine` fields + defaults**

In the `Machine` struct, after `pub original_dynamic: Vec<u8>,`, add:

```rust
    /// In-memory undo snapshots (newest last). Session-only; not saved.
    pub undo_stack: Vec<UndoSnapshot>,
    /// Max retained undo snapshots; 0 disables undo. Default 16; the app sets it
    /// from `config.undo_levels`.
    pub undo_cap: usize,
```

In `Machine::new`, in the struct initializer, add:

```rust
            undo_stack: Vec::new(),
            undo_cap: 16,
```

- [ ] **Step 5: Add the undo methods**

In `crates/zvm/src/cpu/exec.rs`, in `impl Machine` (near `do_store`), add:

```rust
/// `save_undo` (EXT:0x09): push an in-memory snapshot and store the result.
/// Stores -1 (0xFFFF) when undo is disabled (`undo_cap == 0`).
pub(crate) fn do_save_undo(&mut self, store: Option<u8>) {
    if self.undo_cap == 0 {
        self.do_store(store, 0xFFFF);
        return;
    }
    let blob = self.save_quetzal();
    self.undo_stack.push(UndoSnapshot { blob, store });
    if self.undo_stack.len() > self.undo_cap {
        self.undo_stack.remove(0); // drop oldest
    }
    self.do_store(store, 1);
}

/// `restore_undo` (EXT:0x0A): restore the newest snapshot and resume, storing 2
/// into the original `save_undo`'s target. Stores 0 (into this instruction's
/// target) when the stack is empty or a restore fails.
pub(crate) fn do_restore_undo(&mut self, store: Option<u8>) {
    match self.undo_stack.pop() {
        Some(snap) => match self.restore_quetzal(&snap.blob) {
            Ok(()) => self.do_store(snap.store, 2),
            Err(_) => self.do_store(store, 0),
        },
        None => self.do_store(store, 0),
    }
}
```

- [ ] **Step 6: Wire the opcode arms**

In `crates/zvm/src/cpu/exec.rs`, replace the `0x09` / `0x0A` stub arms (~1060-1069):

```rust
            // EXT:0x09 save_undo — in-memory undo snapshot.
            0x09 => {
                self.do_save_undo(store);
                StepResult::Continue
            }
            // EXT:0x0A restore_undo — restore the newest in-memory undo snapshot.
            0x0A => {
                self.do_restore_undo(store);
                StepResult::Continue
            }
```

- [ ] **Step 7: Run the tests + full zvm suite**

Run: `cargo test -p zvm`
Expected: PASS, 0 warnings.

- [ ] **Step 8: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(zvm): in-memory multi-level undo (save_undo / restore_undo)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: undo_levels config

**Files:**
- Modify: `crates/app/src/config.rs` (`Config.undo_levels` + default + file-merge; tests)

**Interfaces:**
- Produces: `Config.undo_levels: usize` (default 16; `0` disables undo).

- [ ] **Step 1: Write the failing test**

In `crates/app/src/config.rs`, inside `mod tests`, add:

```rust
#[test]
fn undo_levels_defaults_to_16() {
    assert_eq!(Config::default().undo_levels, 16);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app undo_levels_defaults_to_16`
Expected: compile error (field missing).

- [ ] **Step 3: Add the field + default helper**

In `crates/app/src/config.rs`, in the `Config` struct (near `watch_style`), add:

```rust
    /// Undo depth: max retained in-memory undo snapshots (default 16; 0 disables).
    #[serde(default = "default_undo_levels")]
    pub undo_levels: usize,
```

Add the default helper near the other `default_*` fns:

```rust
fn default_undo_levels() -> usize { 16 }
```

In `impl Default for Config`, add (near `watch_style: false,`):

```rust
            undo_levels: default_undo_levels(),
```

In the test-literal `Config { … }` (the one that sets `watch_style: false,`), add:

```rust
            undo_levels: 16,
```

- [ ] **Step 4: Carry it in the file-merge**

In `config::resolve`, in the `if let Ok(text) = …` from-file merge block (near `cfg.watch_style = from_file.watch_style;`), add:

```rust
            cfg.undo_levels = from_file.undo_levels;
```

- [ ] **Step 5: Run the test + full suite**

Run: `cargo test -p app`
Expected: PASS, 0 warnings.

- [ ] **Step 6: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/config.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): undo_levels config (default 16, 0 disables)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: Apply the cap to the VM at session creation

**Files:**
- Modify: `crates/app/src/main.rs` (set `undo_cap` after every `GameSession::new`)

**Interfaces:**
- Consumes: `Machine.undo_cap` (Task 1), `Config.undo_levels` (Task 2).

The VM defaults `undo_cap` to 16; this overrides it from config at every session
creation so a user setting (incl. `0` to disable, or a larger depth) takes effect,
including after game reset / restore (which rebuild the session).

- [ ] **Step 1: Set the cap at the primary session creation**

In `crates/app/src/main.rs`, the primary session is created at ~640
(`let mut session = match GameSession::new(...)`). After the session exists and
`cfg`/`state.config` is available (the config override block is noted at ~657),
add:

```rust
    session.machine.undo_cap = cfg.undo_levels;
```

(Place it alongside the other post-creation config overrides near line 657, using
`cfg` — the resolved `Config` in scope there.)

- [ ] **Step 2: Set the cap on session re-creation paths**

`GameSession::new` is also called when the game is reset/restored (main.rs ~2182,
~2462, ~2495 — `*session = new_session;` / `match GameSession::new(...)`). After
each rebuild that replaces the live session, set the cap from `state.config`:

```rust
    session.machine.undo_cap = state.config.undo_levels;
```

(Use the variable that names the rebuilt session at each site — `session` after
`*session = new_session;`, or the freshly-bound session. `state.config` is the
resolved config in the run loop. If a site is headless/test-only with no
`state.config` in scope, use the default by leaving the VM's 16 — but the three
interactive reset/restore sites all have `state` available.)

- [ ] **Step 3: Build + run the suite**

Run: `cargo build -p app && cargo test -p app`
Expected: builds clean, 0 warnings; suite PASS. (No new unit test — this is
one-line wiring; the undo behavior is covered by the zvm tests in Task 1. Manual
check: with `undo_levels = 0` in config, a game's UNDO reports it is unavailable;
with the default, UNDO works.)

- [ ] **Step 4: Document `undo_levels` in the README**

In `README.md`, in the Configuration section, add a line:

```
- `undo_levels` (default 16) — how many in-memory undo states the Z-machine
  keeps for the game's own UNDO command (0 disables undo).
```

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/main.rs README.md
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): apply undo_levels to the VM at session creation + docs

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Dependency order: 1 (VM undo) → 2 (config field) → 3 (wiring). Task 1 is `zvm`
  (`cargo test -p zvm`); Tasks 2-3 are `app` (`cargo test -p app`). Each ends green
  with 0 warnings before committing.
- Task 1's tests call `do_save_undo`/`do_restore_undo` directly with `do_store` to
  set/read globals (G0 = var `0x10`, G1 = `0x11`, …) — no instruction encoding
  needed. `sample_story(5)` / `Machine::new` / `m.global(n)` are the existing test
  helpers.
- Task 3: grep `GameSession::new` in `main.rs` to find every session-creation site;
  set `undo_cap` after each interactive one. Reset/restore rebuild the session, so
  they must re-apply the cap or it reverts to the VM default 16.
- `README.md` is committed; `TODO.md` is gitignored — never stage it.
