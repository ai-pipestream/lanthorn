# Configurable Interpreter Number Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Default the advertised interpreter number to Frotz's rule (1 = DEC-20 for v1–5, 6 = IBM PC for v6) so colour-capable games like BeyondZork use colour, with an override via app config and a zvm-cli `-I`/`--interpreter` flag.

**Architecture:** The engine gains an `Option<u8>` interpreter-number field applied at `init_caps`; when `None` it falls back to a version-based default. Both hosts set it before `init_caps`, exactly as they already do for `honor_game_colours`.

**Tech Stack:** Rust workspace; `zvm` (zero-dep VM), `app` (ratatui), `zvm-cli` (crossterm). Design: `docs/superpowers/specs/2026-06-30-interpreter-number-design.md`.

## Global Constraints

- `zvm` stays zero-dependency.
- Cross-platform (Windows/Linux/macOS).
- 0 compiler warnings AND full workspace test suite green at the end of every task (`cargo test --workspace`; `cargo build --workspace --tests 2>&1 | grep -c warning` prints 0).
- Stage only the files each task names; never `git add -A`. Scratch stays under `.superpowers/`.
- `gvm` (Glulx) is untouched.
- The CP437 passthrough gate (`crates/zvm/src/cpu/exec.rs:1582`, `read_byte(0x1E) == 6`) stays unchanged.
- Commit trailers on every commit (no backticks in the body):
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```

---

## Task 1: Engine — configurable interpreter number with Frotz default

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` (`Machine` struct ~125; `with_output` ~171; `init_caps` ~187; new setter ~197; test at ~3972)
- Modify: `crates/zvm/src/screen.rs` (`init_header_caps` signature ~295 + `0x1E` write ~345; new helper; 10 test callers; assertion at ~545; doc comments ~289/~345)
- Test: both files' test modules

**Interfaces:**
- Produces:
  - `Machine.interpreter_number: Option<u8>` (public field, default `None`).
  - `Machine::set_interpreter_number(&mut self, n: Option<u8>)`.
  - `zvm::screen::default_interpreter_number(version: u8) -> u8`.
  - `zvm::screen::init_header_caps(mem: &mut Memory, honor_game_colours: bool, interpreter_number: Option<u8>)` (added third parameter).

- [ ] **Step 1: Write the failing tests**

Add to the `screen.rs` test module:

```rust
    #[test]
    fn default_interpreter_number_follows_frotz_rule() {
        // Frotz: DEC-20 (1) for non-v6, IBM PC (6) for v6.
        assert_eq!(default_interpreter_number(3), 1);
        assert_eq!(default_interpreter_number(5), 1);
        assert_eq!(default_interpreter_number(8), 1);
        assert_eq!(default_interpreter_number(6), 6);
    }

    #[test]
    fn init_header_caps_default_interpreter_is_dec20_for_v5() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, None);
        assert_eq!(mem.read_byte(0x1E), 1, "v5 default interpreter = DEC-20 (1)");
    }

    #[test]
    fn init_header_caps_interpreter_override_wins() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem, false, Some(6));
        assert_eq!(mem.read_byte(0x1E), 6, "override forces IBM PC (6)");
    }
```

Add to the `exec.rs` test module:

```rust
    #[test]
    fn set_interpreter_number_overrides_at_init_caps() {
        let mut buf = sample_story(5);
        buf[0x80] = 0xBA;                 // quit at 0x80
        buf[0x06] = 0x00; buf[0x07] = 0x80; // initial_pc = 0x0080
        let mem = Memory::new(buf).unwrap();
        let mut m = Machine::new(mem);
        m.set_interpreter_number(Some(4)); // Amiga
        m.init_caps();
        assert_eq!(m.mem.read_byte(0x1E), 4, "override advertised");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p zvm default_interpreter_number_follows_frotz_rule 2>&1 | tail -15`
Expected: FAIL — `default_interpreter_number` not defined and `init_header_caps` arity mismatch.

- [ ] **Step 3: Add the field, setter, and default helper**

In `crates/zvm/src/cpu/exec.rs`, add the field after `pub honor_game_colours: bool,` (~125):

```rust
    /// Interpreter number to advertise in header byte 0x1E. `None` = auto (Frotz's
    /// rule: 6 for v6, else 1). `Some(n)` overrides. Applied at `init_caps`.
    pub interpreter_number: Option<u8>,
```

Initialize it in `with_output` after `honor_game_colours: false,` (~171):

```rust
            interpreter_number: None,
```

Change `init_caps` (~187) to thread the field:

```rust
        init_header_caps(&mut self.mem, self.honor_game_colours, self.interpreter_number);
```

Add the setter after `set_honor_game_colours` (~197):

```rust
    /// Set the interpreter number to advertise (header 0x1E). `None` restores the
    /// auto default (Frotz's rule). Takes effect at the next `init_caps`.
    pub fn set_interpreter_number(&mut self, n: Option<u8>) {
        self.interpreter_number = n;
    }
```

- [ ] **Step 4: Add the default helper and thread it through `init_header_caps`**

In `crates/zvm/src/screen.rs`, add near `init_header_caps`:

```rust
/// Default interpreter number (header 0x1E) per Frotz's rule (ux_init.c): IBM PC
/// (6) for v6 story files, DECSystem-20 (1) otherwise. v6 is rejected at load,
/// so in practice every loaded game defaults to 1.
pub fn default_interpreter_number(version: u8) -> u8 {
    if version == 6 { 6 } else { 1 }
}
```

Change the `init_header_caps` signature to add the parameter:

```rust
pub fn init_header_caps(mem: &mut Memory, honor_game_colours: bool, interpreter_number: Option<u8>) {
```

Replace the `0x1E` write (currently `mem.write_byte(0x1E, 6);` with its comment at ~345) with:

```rust
    // Interpreter number (0x1E): explicit override, else Frotz's default
    // (6 for v6, else 1 = DEC-20). `version` was read at the top of this fn.
    let interp = interpreter_number.unwrap_or_else(|| default_interpreter_number(version));
    mem.write_byte(0x1E, interp);
```

Update the doc comment at `screen.rs:289` (the `/// - 0x1E: interpreter number — 6 (IBM PC)...` line) to:

```rust
///   - 0x1E: interpreter number — override, else Frotz's default (6 for v6, else 1).
```

- [ ] **Step 5: Update every `init_header_caps` caller to pass the new argument**

`crates/zvm/src/cpu/exec.rs:187` is already handled in Step 3 (passes `self.interpreter_number`).

In `crates/zvm/src/screen.rs`, the 10 test callers at lines ~526, ~536, ~554, ~564, ~597, ~606, ~617, ~631, ~640, ~642 each get a third argument `None` (they test other header bits; the interpreter default is irrelevant to them). Example — the call currently `init_header_caps(&mut mem, false);` becomes:

```rust
        init_header_caps(&mut mem, false, None);
```

and the single `true` caller (~642) becomes `init_header_caps(&mut mem, true, None);`.

- [ ] **Step 6: Update the two assertions that expected interpreter 6**

The default is now 1, so two existing tests must expect 1.

`crates/zvm/src/screen.rs:545` (in `header_caps_v5_clears_unsupported_bits`):

```rust
        assert_eq!(mem.read_byte(0x1E), 1, "interpreter number defaults to DEC-20 (1)");
```

`crates/zvm/src/cpu/exec.rs:3972` (in `machine_init_caps_sets_header_bits`):

```rust
        assert_eq!(m.mem.read_byte(0x1E), 1, "interpreter number defaults to DEC-20 (1)");
```

- [ ] **Step 7: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS — all tests green, including the four new ones. (The CP437 tests at `exec.rs:5050`/`5068`/`5076` write `0x1E` directly and are unaffected.)
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 8: Commit**

```bash
git add crates/zvm/src/cpu/exec.rs crates/zvm/src/screen.rs
git commit -F - <<'EOF'
feat(zvm): configurable interpreter number, default per Frotz

Adds Machine.interpreter_number (Option, None = auto) and
set_interpreter_number. init_header_caps now writes 0x1E from the override
or the version default (6 for v6, else 1 = DEC-20), matching Frotz. This
lets BeyondZork use colour instead of monochrome. CP437 gate unchanged.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 2: App — config field + session wiring

**Files:**
- Modify: `crates/app/src/config.rs` (field + default fn + `Default` impl + load-merge + toml write; test)
- Modify: `crates/app/src/session.rs` (`GameSession::new` parameter + wiring)
- Modify: `crates/app/src/main.rs`, `crates/app/src/history.rs`, `crates/app/src/input.rs` (`GameSession::new` call sites)
- Test: `crates/app/src/config.rs`, `crates/app/src/session.rs`

**Interfaces:**
- Consumes: `Machine::set_interpreter_number(Option<u8>)` from Task 1.
- Produces:
  - `Config.interpreter_number: Option<u8>` (default `None`).
  - `GameSession::new(story: Vec<u8>, honor_game_colours: bool, interpreter_number: Option<u8>) -> Result<GameSession, ZError>`.

- [ ] **Step 1: Write the failing tests**

Add to the `config.rs` test module (mirror `honor_game_colours_defaults_true`, which uses `toml::from_str` directly):

```rust
    #[test]
    fn interpreter_number_defaults_none_and_parses_override() {
        // Default and absent key → None (auto).
        assert_eq!(Config::default().interpreter_number, None);
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back.interpreter_number, None, "absent key keeps None");
        // Explicit override parses.
        let over: Config = toml::from_str("interpreter_number = 6\n").unwrap();
        assert_eq!(over.interpreter_number, Some(6), "explicit override parses");
    }
```

Add to the `session.rs` test module (mirror `pending_input_is_char_after_new_on_read_char_story`):

```rust
    #[test]
    fn new_applies_interpreter_override() {
        // read_char_story_v5 is a v5 story; default would be 1, override to 4.
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, Some(4)).expect("GameSession::new");
        assert_eq!(session.interpreter_number_for_test(), 4, "override advertised");
    }

    #[test]
    fn new_default_interpreter_is_dec20() {
        let story = read_char_story_v5();
        let session = GameSession::new(story, true, None).expect("GameSession::new");
        assert_eq!(session.interpreter_number_for_test(), 1, "v5 default = DEC-20 (1)");
    }
```

To read the advertised byte in a test, add a small test-only accessor on `GameSession` in `session.rs`:

```rust
    #[cfg(test)]
    fn interpreter_number_for_test(&self) -> u8 {
        self.machine.mem.read_byte(0x1E)
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app interpreter_number 2>&1 | tail -20`
Expected: FAIL — `Config.interpreter_number` field missing / `GameSession::new` arity mismatch.

- [ ] **Step 3: Add the config field**

In `crates/app/src/config.rs`, mirror `honor_game_colours`:

Default fn (near `default_honor_game_colours` at ~183) — `Option<u8>` defaults to `None`, so use serde's built-in default (no custom fn needed). In the `Config` struct (near ~376), add:

```rust
    /// Interpreter number to advertise (header 0x1E). `None` = auto (Frotz's rule:
    /// 1 for v1-5, 6 for v6). Set to override, e.g. 6 for BeyondZork's IBM PC
    /// character-graphics instead of colour.
    #[serde(default)]
    pub interpreter_number: Option<u8>,
```

In the `Default` impl (near ~407, alongside `honor_game_colours: default_honor_game_colours(),`):

```rust
            interpreter_number: None,
```

In the load-merge block (near ~468, alongside the `honor_game_colours` merge):

```rust
            cfg.interpreter_number = from_file.interpreter_number;
```

In the toml writer (near ~524, alongside the `honor_game_colours` write) — only write when set (toml has no null):

```rust
            if let Some(n) = cfg.interpreter_number {
                doc["interpreter_number"] = toml_edit::value(n as i64);
            }
```

If the `config.rs:820` test literal constructs a `Config` with explicit fields, add `interpreter_number: None,` there too.

- [ ] **Step 4: Add the `GameSession::new` parameter and wiring**

In `crates/app/src/session.rs`, change the signature and add the setter call before `init_caps` (mirroring `set_honor_game_colours`):

```rust
    pub fn new(story: Vec<u8>, honor_game_colours: bool, interpreter_number: Option<u8>) -> Result<GameSession, ZError> {
        let mem = Memory::new(story)?;
        let sink = Box::new(CaptureSink::new());
        let mut machine = Machine::with_output(mem, sink);
        machine.set_honor_game_colours(honor_game_colours);
        machine.set_interpreter_number(interpreter_number);
        machine.init_caps();
        // ... rest unchanged ...
```

- [ ] **Step 5: Update every `GameSession::new` call site**

Add the third argument. Production call sites pass the config value; test call sites pass `None`.

Production (pass `cfg.interpreter_number` / `state.config.interpreter_number` — match the receiver used for `honor_game_colours` on the same line):
- `crates/app/src/main.rs:1096` — `GameSession::new(bytes, cfg.honor_game_colours, cfg.interpreter_number)`
- `crates/app/src/main.rs:3362` — `GameSession::new(bytes, state.config.honor_game_colours, state.config.interpreter_number)`
- `crates/app/src/main.rs:3979` — `GameSession::new(bytes, state.config.honor_game_colours, state.config.interpreter_number)`
- `crates/app/src/main.rs:4014` — `GameSession::new(bytes, state.config.honor_game_colours, state.config.interpreter_number)`

Tests (pass `None`):
- `crates/app/src/main.rs:4307` — `GameSession::new(bytes.clone(), true, None)`
- `crates/app/src/history.rs:189` — `GameSession::new(story, true, None)`
- `crates/app/src/input.rs:5799` — `GameSession::new(story_bytes.clone(), true, None)`
- `crates/app/src/input.rs:5831` — `GameSession::new(story_bytes.clone(), true, None)`

(If a workspace grep `grep -rn "GameSession::new" crates/app` reveals any call site not listed here, update it the same way: production → the config value, tests → `None`.)

- [ ] **Step 6: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/config.rs crates/app/src/session.rs crates/app/src/main.rs crates/app/src/history.rs crates/app/src/input.rs
git commit -F - <<'EOF'
feat(app): interpreter_number config, threaded into the session

Adds an interpreter_number config field (None = auto per Frotz) plumbed
into GameSession::new before init_caps, so users can override the
advertised interpreter (e.g. 6 for BeyondZork's IBM PC graphics).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Task 3: zvm-cli — `-I` / `--interpreter` override flag

**Files:**
- Modify: `crates/zvm-cli/src/main.rs` (arg parsing; `build_machine`; setup + restart wiring)
- Test: `crates/zvm-cli/src/main.rs` (test module)

**Interfaces:**
- Consumes: `Machine::set_interpreter_number(Option<u8>)` from Task 1.
- Produces: `parse_interpreter(args: &[String]) -> Option<u8>`; `build_machine(..., interpreter_number: Option<u8>)`.

- [ ] **Step 1: Write the failing tests**

Add to the `zvm-cli` test module (mirror the `parse_game_colours` tests):

```rust
    #[test]
    fn parse_interpreter_reads_flag() {
        assert_eq!(parse_interpreter(&["-I".into(), "4".into(), "story.z5".into()]), Some(4));
        assert_eq!(parse_interpreter(&["--interpreter".into(), "3".into(), "story.z5".into()]), Some(3));
    }

    #[test]
    fn parse_interpreter_absent_is_none() {
        assert_eq!(parse_interpreter(&["story.z5".into()]), None);
    }

    #[test]
    fn parse_interpreter_bad_value_is_none() {
        assert_eq!(parse_interpreter(&["-I".into(), "notanumber".into(), "story.z5".into()]), None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p zvm-cli parse_interpreter 2>&1 | tail -15`
Expected: FAIL — `parse_interpreter` not defined.

- [ ] **Step 3: Add `parse_interpreter`**

In `crates/zvm-cli/src/main.rs`, add near `parse_game_colours` (~237):

```rust
/// Read the interpreter-number override from `-I N` / `--interpreter N`.
/// Returns None when absent or when N is not a valid u8 (lenient — falls back
/// to the engine's Frotz default).
fn parse_interpreter(args: &[String]) -> Option<u8> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-I" || a == "--interpreter" {
            return it.next().and_then(|v| v.parse::<u8>().ok());
        }
    }
    None
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p zvm-cli parse_interpreter 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 5: Filter the flag + value out of the positional args**

Find where positional args (the story path) are extracted — the same place `--no-game-colours` is filtered (search for `--no-game-colours` in the arg-collection code near `parse_args`/`argv`). Extend that filter so `-I` / `--interpreter` and the token immediately following them are dropped from positionals, so the story path is still found. Concretely, when iterating argv to collect positionals, skip a token equal to `-I`/`--interpreter` and also skip the next token.

- [ ] **Step 6: Thread the override into `build_machine` and call sites**

Change `build_machine` (~185) to accept the override and apply it before `init_caps`:

```rust
fn build_machine(
    story: Vec<u8>,
    stdout_is_tty: bool,
    paging: bool,
    page_height: u16,
    term_cols: u16,
    honor_game_colours: bool,
    interpreter_number: Option<u8>,
) -> Result<Machine, String> {
    // ... unchanged construction ...
    machine.set_interpreter_number(interpreter_number);
    machine.init_caps();
    Ok(machine)
}
```

In `main`, compute `let interpreter = parse_interpreter(&argv);` next to `let honor = parse_game_colours(&argv);` (~521) and pass `interpreter` to `build_machine`. If the CLI re-applies caps on restart (search for `set_honor_game_colours` at ~603 and `init_caps` on the restart path), also call `machine.set_interpreter_number(interpreter)` there, before the corresponding `init_caps`, so restart keeps the override.

- [ ] **Step 7: Run the full workspace to verify green + 0 warnings**

Run: `cargo test --workspace 2>&1 | tail -20`
Expected: PASS.
Run: `cargo build --workspace --tests 2>&1 | grep -c warning`
Expected: `0`

- [ ] **Step 8: Commit**

```bash
git add crates/zvm-cli/src/main.rs
git commit -F - <<'EOF'
feat(zvm-cli): -I / --interpreter override flag

Adds parse_interpreter and threads the override into build_machine before
init_caps (and on restart), matching dfrotz's -I. Absent = engine default
(Frotz's rule). The flag and its value are filtered from the story path.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
EOF
```

---

## Manual verification (after all tasks)

- `cargo run -p zvm-cli -- stories/beyondzork-r57-s871221.z5` → BeyondZork's menu selection uses colour (not reverse video); the map box still renders.
- `cargo run -p zvm-cli -- -I 6 stories/beyondzork-r57-s871221.z5` → reverts to IBM PC monochrome + CP437 box-drawing.
- App: set `interpreter_number = 6` in config.toml and confirm the same override applies.

---

## Self-Review Notes

- **Spec coverage:** Component 1 (engine) → Task 1. Component 2 (app config + session) → Task 2. Component 3 (zvm-cli flag) → Task 3. Frotz default rule (`default_interpreter_number`) and CP437-gate-unchanged both covered in Task 1.
- **Type consistency:** `interpreter_number: Option<u8>` throughout; `init_header_caps(mem, bool, Option<u8>)`; `GameSession::new(Vec<u8>, bool, Option<u8>)`; `build_machine(..., Option<u8>)`; `parse_interpreter(&[String]) -> Option<u8>`; `default_interpreter_number(u8) -> u8`.
- **Test-expectation updates:** the two existing assertions expecting interpreter 6 (`screen.rs:545`, `exec.rs:3972`) are updated to 1 in Task 1 Step 6 — a deliberate behavior change, not a regression. CP437 tests write `0x1E` directly and are unaffected.
